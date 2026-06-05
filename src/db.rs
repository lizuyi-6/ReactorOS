use std::{
    env,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::Duration,
};

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{Context, Result};
use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
    Engine as _,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow},
    Row,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::{control::SafeCommand, optimizer::Recommendation, state::SensorSnapshot};

const READ_CONNECTIONS: usize = 2;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const DB_ENCRYPTION_KEY_ENV: &str = "XINGSHU_DB_ENCRYPTION_KEY";
const ENCRYPTED_JSON_PREFIX: &str = "xingshu:v1:aes256gcm:";
const DB_ENCRYPTION_AAD: &[u8] = b"xingshu:integration_tasks:json:v1";
const AUDIT_CHAIN_CHECK_LIMIT: usize = 10_000;
const SCHEMA_SQL: &str = r#"
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS batches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    process_id INTEGER,
    name TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    target_temperature_c REAL NOT NULL,
    target_stirrer_rpm REAL NOT NULL,
    heating_minutes REAL NOT NULL,
    stirring_minutes REAL NOT NULL,
    FOREIGN KEY(process_id) REFERENCES processes(id)
);

CREATE TABLE IF NOT EXISTS processes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'draft',
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    applied_at TEXT
);

CREATE TABLE IF NOT EXISTS process_steps (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    process_id INTEGER NOT NULL,
    step_index INTEGER NOT NULL,
    name TEXT NOT NULL,
    target_temperature_c REAL NOT NULL,
    ramp_rate_c_min REAL NOT NULL,
    duration_minutes REAL NOT NULL,
    target_stirrer_rpm REAL NOT NULL,
    target_shake_speed_cpm REAL NOT NULL,
    target_pressure_mpa REAL NOT NULL,
    cooling_mode TEXT NOT NULL DEFAULT '自然',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(process_id, step_index),
    FOREIGN KEY(process_id) REFERENCES processes(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS sensor_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER,
    temperature_c REAL NOT NULL,
    pressure_mpa REAL NOT NULL DEFAULT 0,
    stirrer_rpm REAL NOT NULL,
    shake_speed_cpm REAL NOT NULL DEFAULT 0,
    tilt_state INTEGER NOT NULL DEFAULT 0,
    tilt_angle_deg REAL NOT NULL DEFAULT 0,
    flow_rate_l_min REAL NOT NULL DEFAULT 0,
    product_concentration_percent REAL NOT NULL DEFAULT 0,
    ph REAL NOT NULL DEFAULT 7,
    captured_at TEXT NOT NULL,
    FOREIGN KEY(batch_id) REFERENCES batches(id)
);

CREATE TABLE IF NOT EXISTS control_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER,
    event_type TEXT NOT NULL,
    target_temperature_c REAL,
    target_stirrer_rpm REAL,
    target_shake_speed_cpm REAL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL,
    previous_hash TEXT,
    event_hash TEXT,
    FOREIGN KEY(batch_id) REFERENCES batches(id)
);

CREATE TABLE IF NOT EXISTS product_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER NOT NULL UNIQUE,
    yield_percent REAL NOT NULL,
    product_ratio REAL NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    FOREIGN KEY(batch_id) REFERENCES batches(id)
);

CREATE TABLE IF NOT EXISTS ai_recommendations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    based_on_batch_count INTEGER NOT NULL,
    target_temperature_c REAL NOT NULL,
    target_stirrer_rpm REAL NOT NULL,
    heating_minutes REAL NOT NULL,
    stirring_minutes REAL NOT NULL,
    expected_score REAL NOT NULL,
    rationale TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS demo_alarms (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    alarm_type TEXT NOT NULL,
    sensor TEXT NOT NULL,
    level TEXT NOT NULL,
    message TEXT NOT NULL,
    current_value REAL,
    limit_value REAL,
    suggestion TEXT NOT NULL DEFAULT '',
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS integration_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_task_id TEXT,
    source TEXT NOT NULL,
    action TEXT NOT NULL,
    status TEXT NOT NULL,
    request_json TEXT NOT NULL,
    response_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;
const INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_sensor_samples_captured_id
    ON sensor_samples(captured_at, id);
CREATE INDEX IF NOT EXISTS idx_sensor_samples_batch_id_id
    ON sensor_samples(batch_id, id);
CREATE INDEX IF NOT EXISTS idx_control_events_batch_id_id
    ON control_events(batch_id, id);
CREATE INDEX IF NOT EXISTS idx_control_events_hashed_id
    ON control_events(id) WHERE event_hash IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_process_steps_process_index
    ON process_steps(process_id, step_index);
CREATE INDEX IF NOT EXISTS idx_product_results_batch_id
    ON product_results(batch_id);
CREATE INDEX IF NOT EXISTS idx_integration_tasks_source_id
    ON integration_tasks(source, id);
CREATE INDEX IF NOT EXISTS idx_integration_tasks_external_task_id
    ON integration_tasks(source, external_task_id);
"#;
const COLUMN_MIGRATIONS: [(&str, &str, &str); 11] = [
    ("sensor_samples", "pressure_mpa", "REAL NOT NULL DEFAULT 0"),
    (
        "sensor_samples",
        "shake_speed_cpm",
        "REAL NOT NULL DEFAULT 0",
    ),
    (
        "sensor_samples",
        "tilt_angle_deg",
        "REAL NOT NULL DEFAULT 0",
    ),
    ("sensor_samples", "tilt_state", "INTEGER NOT NULL DEFAULT 0"),
    (
        "sensor_samples",
        "flow_rate_l_min",
        "REAL NOT NULL DEFAULT 0",
    ),
    (
        "sensor_samples",
        "product_concentration_percent",
        "REAL NOT NULL DEFAULT 0",
    ),
    ("sensor_samples", "ph", "REAL NOT NULL DEFAULT 7"),
    ("control_events", "target_shake_speed_cpm", "REAL"),
    ("control_events", "previous_hash", "TEXT"),
    ("control_events", "event_hash", "TEXT"),
    ("batches", "process_id", "INTEGER REFERENCES processes(id)"),
];

#[derive(Clone)]
pub struct Db {
    inner: Arc<DbInner>,
}

#[derive(Clone)]
struct DbEncryption {
    cipher: Aes256Gcm,
    key_source: &'static str,
}

struct DbInner {
    write: Mutex<Connection>,
    reads: Vec<Mutex<Connection>>,
    next_read: AtomicUsize,
    encryption: Option<DbEncryption>,
    sqlx_pool: Option<sqlx::SqlitePool>,
    audit_write_lock: AsyncMutex<()>,
    process_write_lock: AsyncMutex<()>,
}

enum DbConnectionGuard<'a> {
    Write(MutexGuard<'a, Connection>),
    Read(MutexGuard<'a, Connection>),
}

impl std::ops::Deref for DbConnectionGuard<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Write(conn) => conn,
            Self::Read(conn) => conn,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Batch {
    pub id: i64,
    pub process_id: Option<i64>,
    pub name: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub target_temperature_c: f64,
    pub target_stirrer_rpm: f64,
    pub heating_minutes: f64,
    pub stirring_minutes: f64,
}

#[derive(Debug, Clone)]
pub struct ProductResult {
    pub batch_id: i64,
    pub yield_percent: f64,
    pub product_ratio: f64,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchOutcome {
    pub batch_id: i64,
    pub target_temperature_c: f64,
    pub target_stirrer_rpm: f64,
    pub heating_minutes: f64,
    pub stirring_minutes: f64,
    pub yield_percent: f64,
    pub product_ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorSampleRecord {
    pub batch_id: Option<i64>,
    #[serde(flatten)]
    pub sample: SensorSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlEvent {
    pub id: i64,
    pub batch_id: Option<i64>,
    pub event_type: String,
    pub target_temperature_c: Option<f64>,
    pub target_stirrer_rpm: Option<f64>,
    pub target_shake_speed_cpm: Option<f64>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub previous_hash: Option<String>,
    pub event_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditChainStatus {
    pub total_hashed_events: usize,
    pub checked_events: usize,
    pub chained_events: usize,
    pub broken_events: usize,
    pub window_valid: bool,
    pub valid: bool,
    pub last_event_hash: Option<String>,
    pub checked_from_event_id: Option<i64>,
    pub checked_to_event_id: Option<i64>,
    pub verification_limit: usize,
    pub verification_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoAlarm {
    pub id: i64,
    pub alarm_type: String,
    pub sensor: String,
    pub level: String,
    pub message: String,
    pub current_value: Option<f64>,
    pub limit_value: Option<f64>,
    pub suggestion: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationTask {
    pub id: i64,
    pub external_task_id: Option<String>,
    pub source: String,
    pub action: String,
    pub status: String,
    pub request: Value,
    pub response: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbEncryptionStatus {
    pub enabled: bool,
    pub algorithm: &'static str,
    pub key_source: Option<&'static str>,
    pub encrypted_fields: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessDefinition {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub version: i64,
    pub step_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub applied_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessStep {
    pub id: i64,
    pub process_id: i64,
    pub step_index: i64,
    pub name: String,
    pub target_temperature_c: f64,
    pub ramp_rate_c_min: f64,
    pub duration_minutes: f64,
    pub target_stirrer_rpm: f64,
    pub target_shake_speed_cpm: f64,
    pub target_pressure_mpa: f64,
    pub cooling_mode: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessDetail {
    pub process: ProcessDefinition,
    pub steps: Vec<ProcessStep>,
}

#[derive(Debug, Clone)]
pub struct NewProcessStep {
    pub name: String,
    pub target_temperature_c: f64,
    pub ramp_rate_c_min: f64,
    pub duration_minutes: f64,
    pub target_stirrer_rpm: f64,
    pub target_shake_speed_cpm: f64,
    pub target_pressure_mpa: f64,
    pub cooling_mode: String,
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_optional_encryption(path, db_encryption_from_env()?)
    }

    pub fn open_with_encryption_key(path: impl AsRef<Path>, key: [u8; 32]) -> Result<Self> {
        Self::open_with_optional_encryption(path, Some(DbEncryption::from_key(key, "test")))
    }

    fn open_with_optional_encryption(
        path: impl AsRef<Path>,
        encryption: Option<DbEncryption>,
    ) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory {}", parent.display())
            })?;
        }
        let write = open_configured_connection(path.as_ref())
            .with_context(|| format!("failed to open database {}", path.as_ref().display()))?;
        let mut reads = Vec::with_capacity(READ_CONNECTIONS);
        for _ in 0..READ_CONNECTIONS {
            reads.push(Mutex::new(
                open_configured_connection(path.as_ref()).with_context(|| {
                    format!("failed to open database reader {}", path.as_ref().display())
                })?,
            ));
        }
        let sqlx_pool = open_sqlx_pool(path.as_ref());
        let db = Self::from_connections(write, reads, encryption, sqlx_pool);
        db.migrate()?;
        Ok(db)
    }

    pub fn open_memory() -> Result<Self> {
        let db = Self::from_connections(
            configure_connection(Connection::open_in_memory()?)?,
            vec![],
            db_encryption_from_env()?,
            None,
        );
        db.migrate()?;
        Ok(db)
    }

    pub fn open_memory_with_encryption_key(key: [u8; 32]) -> Result<Self> {
        let db = Self::from_connections(
            configure_connection(Connection::open_in_memory()?)?,
            vec![],
            Some(DbEncryption::from_key(key, "test")),
            None,
        );
        db.migrate()?;
        Ok(db)
    }

    pub fn encryption_status(&self) -> DbEncryptionStatus {
        DbEncryptionStatus {
            enabled: self.inner.encryption.is_some(),
            algorithm: "AES-256-GCM",
            key_source: self
                .inner
                .encryption
                .as_ref()
                .map(|encryption| encryption.key_source),
            encrypted_fields: vec![
                "integration_tasks.request_json",
                "integration_tasks.response_json",
            ],
        }
    }

    fn from_connections(
        write: Connection,
        reads: Vec<Mutex<Connection>>,
        encryption: Option<DbEncryption>,
        sqlx_pool: Option<sqlx::SqlitePool>,
    ) -> Self {
        Self {
            inner: Arc::new(DbInner {
                write: Mutex::new(write),
                reads,
                next_read: AtomicUsize::new(0),
                encryption,
                sqlx_pool,
                audit_write_lock: AsyncMutex::new(()),
                process_write_lock: AsyncMutex::new(()),
            }),
        }
    }

    pub fn migrate(&self) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute_batch(SCHEMA_SQL)?;
        create_indexes(&conn)?;
        let has_legacy_pressure_kpa = column_exists(&conn, "sensor_samples", "pressure_kpa")?;
        for migration in COLUMN_MIGRATIONS {
            add_column_if_missing(&conn, migration.0, migration.1, migration.2)?;
        }
        if has_legacy_pressure_kpa {
            conn.execute(
                "UPDATE sensor_samples SET pressure_mpa = pressure_kpa / 1000.0 WHERE pressure_mpa = 0 AND pressure_kpa > 0",
                [],
            )?;
        }
        Ok(())
    }

    pub fn demo_seed_exists(&self) -> Result<bool> {
        let conn = self.read_conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM control_events WHERE event_type = 'demo_seed_applied'",
            [],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn create_batch(
        &self,
        name: &str,
        target_temperature_c: f64,
        target_stirrer_rpm: f64,
        heating_minutes: f64,
        stirring_minutes: f64,
    ) -> Result<Batch> {
        self.create_batch_for_process(
            None,
            name,
            target_temperature_c,
            target_stirrer_rpm,
            heating_minutes,
            stirring_minutes,
        )
    }

    pub fn create_batch_for_process(
        &self,
        process_id: Option<i64>,
        name: &str,
        target_temperature_c: f64,
        target_stirrer_rpm: f64,
        heating_minutes: f64,
        stirring_minutes: f64,
    ) -> Result<Batch> {
        let now = Utc::now();
        let conn = self.write_conn()?;
        conn.execute(
            r#"
            INSERT INTO batches
                (process_id, name, started_at, target_temperature_c, target_stirrer_rpm, heating_minutes, stirring_minutes)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                process_id,
                name,
                now.to_rfc3339(),
                target_temperature_c,
                target_stirrer_rpm,
                heating_minutes,
                stirring_minutes
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(Batch {
            id,
            process_id,
            name: name.to_string(),
            started_at: now,
            finished_at: None,
            target_temperature_c,
            target_stirrer_rpm,
            heating_minutes,
            stirring_minutes,
        })
    }

    pub async fn create_batch_for_process_sqlx(
        &self,
        process_id: Option<i64>,
        name: &str,
        target_temperature_c: f64,
        target_stirrer_rpm: f64,
        heating_minutes: f64,
        stirring_minutes: f64,
    ) -> Result<Batch> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.create_batch_for_process(
                process_id,
                name,
                target_temperature_c,
                target_stirrer_rpm,
                heating_minutes,
                stirring_minutes,
            );
        };
        let now = Utc::now();
        let result = sqlx::query(
            r#"
            INSERT INTO batches
                (process_id, name, started_at, target_temperature_c, target_stirrer_rpm, heating_minutes, stirring_minutes)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(process_id)
        .bind(name)
        .bind(now.to_rfc3339())
        .bind(target_temperature_c)
        .bind(target_stirrer_rpm)
        .bind(heating_minutes)
        .bind(stirring_minutes)
        .execute(pool)
        .await
        .context("failed to create batch with SQLx")?;
        Ok(Batch {
            id: result.last_insert_rowid(),
            process_id,
            name: name.to_string(),
            started_at: now,
            finished_at: None,
            target_temperature_c,
            target_stirrer_rpm,
            heating_minutes,
            stirring_minutes,
        })
    }

    pub fn create_process(&self, name: &str, description: &str) -> Result<ProcessDefinition> {
        let now = Utc::now();
        let conn = self.write_conn()?;
        conn.execute(
            r#"
            INSERT INTO processes (name, description, status, version, created_at, updated_at)
            VALUES (?1, ?2, 'draft', 1, ?3, ?3)
            "#,
            params![name, description, now.to_rfc3339()],
        )?;
        let id = conn.last_insert_rowid();
        Ok(ProcessDefinition {
            id,
            name: name.to_string(),
            description: description.to_string(),
            status: "draft".to_string(),
            version: 1,
            step_count: 0,
            created_at: now,
            updated_at: now,
            applied_at: None,
        })
    }

    pub async fn create_process_sqlx(
        &self,
        name: &str,
        description: &str,
    ) -> Result<ProcessDefinition> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.create_process(name, description);
        };
        let now = Utc::now();
        let result = sqlx::query(
            r#"
            INSERT INTO processes (name, description, status, version, created_at, updated_at)
            VALUES (?, ?, 'draft', 1, ?, ?)
            "#,
        )
        .bind(name)
        .bind(description)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await
        .context("failed to create process with SQLx")?;
        Ok(ProcessDefinition {
            id: result.last_insert_rowid(),
            name: name.to_string(),
            description: description.to_string(),
            status: "draft".to_string(),
            version: 1,
            step_count: 0,
            created_at: now,
            updated_at: now,
            applied_at: None,
        })
    }

    pub fn update_process(
        &self,
        process_id: i64,
        name: &str,
        description: &str,
        status: &str,
    ) -> Result<Option<ProcessDefinition>> {
        let now = Utc::now();
        let conn = self.write_conn()?;
        let changed = conn.execute(
            r#"
            UPDATE processes
            SET name = ?1, description = ?2, status = ?3, version = version + 1, updated_at = ?4
            WHERE id = ?5
            "#,
            params![name, description, status, now.to_rfc3339(), process_id],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        process_summary_by_id(&conn, process_id).map_err(Into::into)
    }

    pub async fn update_process_sqlx(
        &self,
        process_id: i64,
        name: &str,
        description: &str,
        status: &str,
    ) -> Result<Option<ProcessDefinition>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.update_process(process_id, name, description, status);
        };
        let _process_guard = self.inner.process_write_lock.lock().await;
        let now = Utc::now();
        let mut tx = pool
            .begin()
            .await
            .context("failed to begin process update transaction with SQLx")?;
        let result = sqlx::query(
            r#"
            UPDATE processes
            SET name = ?, description = ?, status = ?, version = version + 1, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(name)
        .bind(description)
        .bind(status)
        .bind(now.to_rfc3339())
        .bind(process_id)
        .execute(&mut *tx)
        .await
        .context("failed to update process with SQLx")?;
        if result.rows_affected() == 0 {
            tx.commit()
                .await
                .context("failed to commit empty process update transaction with SQLx")?;
            return Ok(None);
        }
        let process = self
            .process_summary_by_id_sqlx_tx(&mut tx, process_id)
            .await?;
        tx.commit()
            .await
            .context("failed to commit process update transaction with SQLx")?;
        Ok(process)
    }

    pub fn list_processes(&self) -> Result<Vec<ProcessDefinition>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT p.id, p.name, p.description, p.status, p.version,
                   COUNT(s.id) AS step_count, p.created_at, p.updated_at, p.applied_at
            FROM processes p
            LEFT JOIN process_steps s ON s.process_id = p.id
            GROUP BY p.id
            ORDER BY p.updated_at DESC, p.id DESC
            "#,
        )?;
        let rows = stmt.query_map([], process_definition_from_row)?;

        let mut processes = Vec::new();
        for row in rows {
            processes.push(row?);
        }
        Ok(processes)
    }

    pub async fn list_processes_sqlx(&self) -> Result<Vec<ProcessDefinition>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.list_processes();
        };
        let rows = sqlx::query(
            r#"
            SELECT p.id, p.name, p.description, p.status, p.version,
                   COUNT(s.id) AS step_count, p.created_at, p.updated_at, p.applied_at
            FROM processes p
            LEFT JOIN process_steps s ON s.process_id = p.id
            GROUP BY p.id
            ORDER BY p.updated_at DESC, p.id DESC
            "#,
        )
        .fetch_all(pool)
        .await
        .context("failed to list processes with SQLx")?;
        rows.into_iter()
            .map(process_definition_from_sqlx_row)
            .collect()
    }

    pub fn process_detail(&self, process_id: i64) -> Result<Option<ProcessDetail>> {
        let conn = self.read_conn()?;
        let Some(process) = process_summary_by_id(&conn, process_id)? else {
            return Ok(None);
        };
        let steps = process_steps_for_conn(&conn, process_id)?;
        Ok(Some(ProcessDetail { process, steps }))
    }

    pub async fn process_detail_sqlx(&self, process_id: i64) -> Result<Option<ProcessDetail>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.process_detail(process_id);
        };
        let Some(process) = self
            .process_summary_by_id_sqlx_pool(pool, process_id)
            .await?
        else {
            return Ok(None);
        };
        let steps = process_steps_for_pool_sqlx(pool, process_id).await?;
        Ok(Some(ProcessDetail { process, steps }))
    }

    pub fn add_process_step(
        &self,
        process_id: i64,
        step: &NewProcessStep,
    ) -> Result<Option<ProcessStep>> {
        let conn = self.write_conn()?;
        if process_summary_by_id(&conn, process_id)?.is_none() {
            return Ok(None);
        }
        let next_index: i64 = conn.query_row(
            "SELECT COALESCE(MAX(step_index), 0) + 1 FROM process_steps WHERE process_id = ?1",
            [process_id],
            |row| row.get(0),
        )?;
        let now = Utc::now();
        conn.execute(
            r#"
            INSERT INTO process_steps
                (process_id, step_index, name, target_temperature_c, ramp_rate_c_min,
                 duration_minutes, target_stirrer_rpm, target_shake_speed_cpm,
                 target_pressure_mpa, cooling_mode, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
            "#,
            params![
                process_id,
                next_index,
                step.name,
                step.target_temperature_c,
                step.ramp_rate_c_min,
                step.duration_minutes,
                step.target_stirrer_rpm,
                step.target_shake_speed_cpm,
                step.target_pressure_mpa,
                step.cooling_mode,
                now.to_rfc3339()
            ],
        )?;
        touch_process(&conn, process_id)?;
        process_step_by_id(&conn, conn.last_insert_rowid()).map_err(Into::into)
    }

    pub async fn add_process_step_sqlx(
        &self,
        process_id: i64,
        step: &NewProcessStep,
    ) -> Result<Option<ProcessStep>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.add_process_step(process_id, step);
        };
        let _process_guard = self.inner.process_write_lock.lock().await;
        let mut tx = pool
            .begin()
            .await
            .context("failed to begin process step insert transaction with SQLx")?;
        if self
            .process_summary_by_id_sqlx_tx(&mut tx, process_id)
            .await?
            .is_none()
        {
            tx.commit()
                .await
                .context("failed to commit empty process step insert transaction with SQLx")?;
            return Ok(None);
        }
        let next_index: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(step_index), 0) + 1 FROM process_steps WHERE process_id = ?",
        )
        .bind(process_id)
        .fetch_one(&mut *tx)
        .await
        .context("failed to allocate process step index with SQLx")?;
        let now = Utc::now();
        let result = sqlx::query(
            r#"
            INSERT INTO process_steps
                (process_id, step_index, name, target_temperature_c, ramp_rate_c_min,
                 duration_minutes, target_stirrer_rpm, target_shake_speed_cpm,
                 target_pressure_mpa, cooling_mode, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(process_id)
        .bind(next_index)
        .bind(&step.name)
        .bind(step.target_temperature_c)
        .bind(step.ramp_rate_c_min)
        .bind(step.duration_minutes)
        .bind(step.target_stirrer_rpm)
        .bind(step.target_shake_speed_cpm)
        .bind(step.target_pressure_mpa)
        .bind(&step.cooling_mode)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await
        .context("failed to insert process step with SQLx")?;
        touch_process_sqlx(&mut tx, process_id).await?;
        let step = process_step_by_id_sqlx_tx(&mut tx, result.last_insert_rowid()).await?;
        tx.commit()
            .await
            .context("failed to commit process step insert transaction with SQLx")?;
        Ok(step)
    }

    pub fn update_process_step(
        &self,
        process_id: i64,
        step_id: i64,
        step: &NewProcessStep,
    ) -> Result<Option<ProcessStep>> {
        let now = Utc::now();
        let conn = self.write_conn()?;
        let changed = conn.execute(
            r#"
            UPDATE process_steps
            SET name = ?1,
                target_temperature_c = ?2,
                ramp_rate_c_min = ?3,
                duration_minutes = ?4,
                target_stirrer_rpm = ?5,
                target_shake_speed_cpm = ?6,
                target_pressure_mpa = ?7,
                cooling_mode = ?8,
                updated_at = ?9
            WHERE id = ?10 AND process_id = ?11
            "#,
            params![
                step.name,
                step.target_temperature_c,
                step.ramp_rate_c_min,
                step.duration_minutes,
                step.target_stirrer_rpm,
                step.target_shake_speed_cpm,
                step.target_pressure_mpa,
                step.cooling_mode,
                now.to_rfc3339(),
                step_id,
                process_id
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        touch_process(&conn, process_id)?;
        process_step_by_id(&conn, step_id).map_err(Into::into)
    }

    pub async fn update_process_step_sqlx(
        &self,
        process_id: i64,
        step_id: i64,
        step: &NewProcessStep,
    ) -> Result<Option<ProcessStep>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.update_process_step(process_id, step_id, step);
        };
        let _process_guard = self.inner.process_write_lock.lock().await;
        let now = Utc::now();
        let mut tx = pool
            .begin()
            .await
            .context("failed to begin process step update transaction with SQLx")?;
        let result = sqlx::query(
            r#"
            UPDATE process_steps
            SET name = ?,
                target_temperature_c = ?,
                ramp_rate_c_min = ?,
                duration_minutes = ?,
                target_stirrer_rpm = ?,
                target_shake_speed_cpm = ?,
                target_pressure_mpa = ?,
                cooling_mode = ?,
                updated_at = ?
            WHERE id = ? AND process_id = ?
            "#,
        )
        .bind(&step.name)
        .bind(step.target_temperature_c)
        .bind(step.ramp_rate_c_min)
        .bind(step.duration_minutes)
        .bind(step.target_stirrer_rpm)
        .bind(step.target_shake_speed_cpm)
        .bind(step.target_pressure_mpa)
        .bind(&step.cooling_mode)
        .bind(now.to_rfc3339())
        .bind(step_id)
        .bind(process_id)
        .execute(&mut *tx)
        .await
        .context("failed to update process step with SQLx")?;
        if result.rows_affected() == 0 {
            tx.commit()
                .await
                .context("failed to commit empty process step update transaction with SQLx")?;
            return Ok(None);
        }
        touch_process_sqlx(&mut tx, process_id).await?;
        let step = process_step_by_id_sqlx_tx(&mut tx, step_id).await?;
        tx.commit()
            .await
            .context("failed to commit process step update transaction with SQLx")?;
        Ok(step)
    }

    pub fn mark_process_applied(&self, process_id: i64) -> Result<Option<ProcessDefinition>> {
        let now = Utc::now();
        let conn = self.write_conn()?;
        let changed = conn.execute(
            r#"
            UPDATE processes
            SET status = 'applied', applied_at = ?1, updated_at = ?1
            WHERE id = ?2
            "#,
            params![now.to_rfc3339(), process_id],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        process_summary_by_id(&conn, process_id).map_err(Into::into)
    }

    pub async fn mark_process_applied_sqlx(
        &self,
        process_id: i64,
    ) -> Result<Option<ProcessDefinition>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.mark_process_applied(process_id);
        };
        let _process_guard = self.inner.process_write_lock.lock().await;
        let now = Utc::now();
        let mut tx = pool
            .begin()
            .await
            .context("failed to begin process applied transaction with SQLx")?;
        let result = sqlx::query(
            r#"
            UPDATE processes
            SET status = 'applied', applied_at = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(process_id)
        .execute(&mut *tx)
        .await
        .context("failed to mark process applied with SQLx")?;
        if result.rows_affected() == 0 {
            tx.commit()
                .await
                .context("failed to commit empty process applied transaction with SQLx")?;
            return Ok(None);
        }
        let process = self
            .process_summary_by_id_sqlx_tx(&mut tx, process_id)
            .await?;
        tx.commit()
            .await
            .context("failed to commit process applied transaction with SQLx")?;
        Ok(process)
    }

    pub fn finish_batch(&self, batch_id: i64) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute(
            "UPDATE batches SET finished_at = ?1 WHERE id = ?2 AND finished_at IS NULL",
            params![Utc::now().to_rfc3339(), batch_id],
        )?;
        Ok(())
    }

    pub async fn finish_batch_sqlx(&self, batch_id: i64) -> Result<()> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.finish_batch(batch_id);
        };
        sqlx::query("UPDATE batches SET finished_at = ? WHERE id = ? AND finished_at IS NULL")
            .bind(Utc::now().to_rfc3339())
            .bind(batch_id)
            .execute(pool)
            .await
            .context("failed to finish batch with SQLx")?;
        Ok(())
    }

    pub fn insert_sample(&self, batch_id: Option<i64>, sample: &SensorSnapshot) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute(
            r#"
            INSERT INTO sensor_samples
                (batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                 shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                batch_id,
                sample.temperature_c,
                sample.pressure_mpa,
                sample.stirrer_rpm,
                sample.shake_speed_cpm,
                sample.tilt_state,
                sample.tilt_angle_deg,
                sample.flow_rate_l_min,
                sample.product_concentration_percent,
                sample.ph,
                sample.captured_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub async fn insert_sample_sqlx(
        &self,
        batch_id: Option<i64>,
        sample: &SensorSnapshot,
    ) -> Result<()> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.insert_sample(batch_id, sample);
        };
        sqlx::query(
            r#"
            INSERT INTO sensor_samples
                (batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                 shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(batch_id)
        .bind(sample.temperature_c)
        .bind(sample.pressure_mpa)
        .bind(sample.stirrer_rpm)
        .bind(sample.shake_speed_cpm)
        .bind(i64::from(sample.tilt_state))
        .bind(sample.tilt_angle_deg)
        .bind(sample.flow_rate_l_min)
        .bind(sample.product_concentration_percent)
        .bind(sample.ph)
        .bind(sample.captured_at.to_rfc3339())
        .execute(pool)
        .await
        .context("failed to insert sensor sample with SQLx")?;
        Ok(())
    }

    pub fn recent_samples(&self, limit: usize) -> Result<Vec<SensorSnapshot>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM (
                SELECT id, temperature_c, pressure_mpa, stirrer_rpm,
                       shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
                FROM sensor_samples
                ORDER BY id DESC
                LIMIT ?1
            )
            ORDER BY id ASC
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let captured_at: String = row.get(10)?;
            Ok(SensorSnapshot {
                temperature_c: row.get(1)?,
                pressure_mpa: row.get(2)?,
                stirrer_rpm: row.get(3)?,
                shake_speed_cpm: row.get(4)?,
                tilt_state: row.get(5)?,
                tilt_angle_deg: row.get(6)?,
                flow_rate_l_min: row.get(7)?,
                product_concentration_percent: row.get(8)?,
                ph: row.get(9)?,
                captured_at: parse_dt(&captured_at)?,
            })
        })?;

        let mut samples = Vec::new();
        for row in rows {
            samples.push(row?);
        }
        Ok(samples)
    }

    pub fn recent_sample_records(&self, limit: usize) -> Result<Vec<SensorSampleRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM (
                SELECT id, batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                       shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
                FROM sensor_samples
                ORDER BY id DESC
                LIMIT ?1
            )
            ORDER BY id ASC
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let captured_at: String = row.get(11)?;
            Ok(SensorSampleRecord {
                batch_id: row.get(1)?,
                sample: SensorSnapshot {
                    temperature_c: row.get(2)?,
                    pressure_mpa: row.get(3)?,
                    stirrer_rpm: row.get(4)?,
                    shake_speed_cpm: row.get(5)?,
                    tilt_state: row.get(6)?,
                    tilt_angle_deg: row.get(7)?,
                    flow_rate_l_min: row.get(8)?,
                    product_concentration_percent: row.get(9)?,
                    ph: row.get(10)?,
                    captured_at: parse_dt(&captured_at)?,
                },
            })
        })?;

        let mut samples = Vec::new();
        for row in rows {
            samples.push(row?);
        }
        Ok(samples)
    }

    pub async fn recent_sample_records_sqlx(
        &self,
        limit: usize,
    ) -> Result<Vec<SensorSampleRecord>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.recent_sample_records(limit);
        };
        let rows = sqlx::query(
            r#"
            SELECT id, batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM (
                SELECT id, batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                       shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
                FROM sensor_samples
                ORDER BY id DESC
                LIMIT ?
            )
            ORDER BY id ASC
            "#,
        )
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .context("failed to list recent sensor samples with SQLx")?;
        rows.into_iter()
            .map(sensor_sample_record_from_sqlx_row)
            .collect()
    }

    pub fn samples_between(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SensorSampleRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM sensor_samples
            WHERE captured_at >= ?1 AND captured_at <= ?2
            ORDER BY captured_at ASC, id ASC
            LIMIT ?3 OFFSET ?4
            "#,
        )?;
        let rows = stmt.query_map(
            params![
                start_time.to_rfc3339(),
                end_time.to_rfc3339(),
                limit as i64,
                offset as i64
            ],
            |row| {
                let captured_at: String = row.get(10)?;
                Ok(SensorSampleRecord {
                    batch_id: row.get(0)?,
                    sample: SensorSnapshot {
                        temperature_c: row.get(1)?,
                        pressure_mpa: row.get(2)?,
                        stirrer_rpm: row.get(3)?,
                        shake_speed_cpm: row.get(4)?,
                        tilt_state: row.get(5)?,
                        tilt_angle_deg: row.get(6)?,
                        flow_rate_l_min: row.get(7)?,
                        product_concentration_percent: row.get(8)?,
                        ph: row.get(9)?,
                        captured_at: parse_dt(&captured_at)?,
                    },
                })
            },
        )?;

        let mut samples = Vec::new();
        for row in rows {
            samples.push(row?);
        }
        Ok(samples)
    }

    pub async fn samples_between_sqlx(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SensorSampleRecord>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.samples_between(start_time, end_time, limit, offset);
        };
        let rows = sqlx::query(
            r#"
            SELECT batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM sensor_samples
            WHERE captured_at >= ? AND captured_at <= ?
            ORDER BY captured_at ASC, id ASC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(start_time.to_rfc3339())
        .bind(end_time.to_rfc3339())
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(pool)
        .await
        .context("failed to list sensor history with SQLx")?;
        rows.into_iter()
            .map(sensor_sample_record_from_sqlx_row)
            .collect()
    }

    pub fn insert_control_event(
        &self,
        batch_id: Option<i64>,
        event_type: &str,
        command: Option<&SafeCommand>,
        reason: &str,
    ) -> Result<()> {
        let conn = self.write_conn()?;
        let created_at = Utc::now().to_rfc3339();
        let previous_hash: Option<String> = conn
            .query_row(
                r#"
                SELECT event_hash
                FROM control_events
                WHERE event_hash IS NOT NULL
                ORDER BY id DESC
                LIMIT 1
                "#,
                [],
                |row| row.get(0),
            )
            .optional()?;
        let target_temperature_c = command.map(|cmd| cmd.target_temperature_c);
        let target_stirrer_rpm = command.map(|cmd| cmd.target_stirrer_rpm);
        let target_shake_speed_cpm = command.map(|cmd| cmd.target_shake_speed_cpm);
        let event_hash = control_event_hash(
            previous_hash.as_deref(),
            batch_id,
            event_type,
            target_temperature_c,
            target_stirrer_rpm,
            target_shake_speed_cpm,
            reason,
            &created_at,
        )?;
        conn.execute(
            r#"
            INSERT INTO control_events
                (batch_id, event_type, target_temperature_c, target_stirrer_rpm, target_shake_speed_cpm,
                 reason, created_at, previous_hash, event_hash)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                batch_id,
                event_type,
                target_temperature_c,
                target_stirrer_rpm,
                target_shake_speed_cpm,
                reason,
                created_at,
                previous_hash,
                event_hash
            ],
        )?;
        Ok(())
    }

    pub async fn insert_control_event_sqlx(
        &self,
        batch_id: Option<i64>,
        event_type: &str,
        command: Option<&SafeCommand>,
        reason: &str,
    ) -> Result<()> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.insert_control_event(batch_id, event_type, command, reason);
        };
        let _audit_guard = self.inner.audit_write_lock.lock().await;
        let created_at = Utc::now().to_rfc3339();
        let mut tx = pool
            .begin()
            .await
            .context("failed to begin audit insert transaction with SQLx")?;
        let previous_hash: Option<String> = sqlx::query_scalar(
            r#"
            SELECT event_hash
            FROM control_events
            WHERE event_hash IS NOT NULL
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&mut *tx)
        .await
        .context("failed to read previous audit hash with SQLx")?;
        let target_temperature_c = command.map(|cmd| cmd.target_temperature_c);
        let target_stirrer_rpm = command.map(|cmd| cmd.target_stirrer_rpm);
        let target_shake_speed_cpm = command.map(|cmd| cmd.target_shake_speed_cpm);
        let event_hash = control_event_hash(
            previous_hash.as_deref(),
            batch_id,
            event_type,
            target_temperature_c,
            target_stirrer_rpm,
            target_shake_speed_cpm,
            reason,
            &created_at,
        )?;
        sqlx::query(
            r#"
            INSERT INTO control_events
                (batch_id, event_type, target_temperature_c, target_stirrer_rpm, target_shake_speed_cpm,
                 reason, created_at, previous_hash, event_hash)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(batch_id)
        .bind(event_type)
        .bind(target_temperature_c)
        .bind(target_stirrer_rpm)
        .bind(target_shake_speed_cpm)
        .bind(reason)
        .bind(created_at)
        .bind(previous_hash)
        .bind(event_hash)
        .execute(&mut *tx)
        .await
        .context("failed to insert audit event with SQLx")?;
        tx.commit()
            .await
            .context("failed to commit audit insert transaction with SQLx")?;
        Ok(())
    }

    pub fn insert_demo_alarm(
        &self,
        alarm_type: &str,
        sensor: &str,
        level: &str,
        message: &str,
        current_value: Option<f64>,
        limit_value: Option<f64>,
        suggestion: &str,
    ) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute(
            r#"
            INSERT INTO demo_alarms
                (alarm_type, sensor, level, message, current_value, limit_value, suggestion, active, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)
            "#,
            params![
                alarm_type,
                sensor,
                level,
                message,
                current_value,
                limit_value,
                suggestion,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn recent_demo_alarms(&self, limit: usize) -> Result<Vec<DemoAlarm>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, alarm_type, sensor, level, message, current_value, limit_value,
                   suggestion, active, created_at
            FROM (
                SELECT id, alarm_type, sensor, level, message, current_value, limit_value,
                       suggestion, active, created_at
                FROM demo_alarms
                ORDER BY id DESC
                LIMIT ?1
            )
            ORDER BY id ASC
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let created_at: String = row.get(9)?;
            Ok(DemoAlarm {
                id: row.get(0)?,
                alarm_type: row.get(1)?,
                sensor: row.get(2)?,
                level: row.get(3)?,
                message: row.get(4)?,
                current_value: row.get(5)?,
                limit_value: row.get(6)?,
                suggestion: row.get(7)?,
                active: row.get::<_, i64>(8)? != 0,
                created_at: parse_dt(&created_at)?,
            })
        })?;

        let mut alarms = Vec::new();
        for row in rows {
            alarms.push(row?);
        }
        Ok(alarms)
    }

    pub fn create_integration_task(
        &self,
        source: &str,
        external_task_id: Option<&str>,
        action: &str,
        request: &Value,
    ) -> Result<IntegrationTask> {
        let conn = self.write_conn()?;
        let now = Utc::now().to_rfc3339();
        let request_json = self.serialize_sensitive_json(request)?;
        let response_json = self.serialize_sensitive_json(&Value::Null)?;
        conn.execute(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES (?1, ?2, ?3, 'received', ?4, ?5, ?6, ?6)
            "#,
            params![external_task_id, source, action, request_json, response_json, now],
        )?;
        let id = conn.last_insert_rowid();
        self.integration_task_by_id_conn(&conn, id)?
            .ok_or_else(|| anyhow::anyhow!("integration task was not readable after insert"))
    }

    pub async fn create_integration_task_sqlx(
        &self,
        source: &str,
        external_task_id: Option<&str>,
        action: &str,
        request: &Value,
    ) -> Result<IntegrationTask> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.create_integration_task(source, external_task_id, action, request);
        };
        let now = Utc::now().to_rfc3339();
        let request_json = self.serialize_sensitive_json(request)?;
        let response_json = self.serialize_sensitive_json(&Value::Null)?;
        let result = sqlx::query(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES (?, ?, ?, 'received', ?, ?, ?, ?)
            "#,
        )
        .bind(external_task_id)
        .bind(source)
        .bind(action)
        .bind(request_json)
        .bind(response_json)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .context("failed to create integration task with SQLx")?;
        let id = result.last_insert_rowid();
        self.integration_task_sqlx(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("integration task was not readable after SQLx insert"))
    }

    pub fn update_integration_task(
        &self,
        id: i64,
        status: &str,
        response: &Value,
    ) -> Result<Option<IntegrationTask>> {
        let conn = self.write_conn()?;
        let response_json = self.serialize_sensitive_json(response)?;
        conn.execute(
            r#"
            UPDATE integration_tasks
            SET status = ?1, response_json = ?2, updated_at = ?3
            WHERE id = ?4
            "#,
            params![status, response_json, Utc::now().to_rfc3339(), id],
        )?;
        Ok(self.integration_task_by_id_conn(&conn, id)?)
    }

    pub async fn update_integration_task_sqlx(
        &self,
        id: i64,
        status: &str,
        response: &Value,
    ) -> Result<Option<IntegrationTask>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.update_integration_task(id, status, response);
        };
        let response_json = self.serialize_sensitive_json(response)?;
        sqlx::query(
            r#"
            UPDATE integration_tasks
            SET status = ?, response_json = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(response_json)
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await
        .context("failed to update integration task with SQLx")?;
        self.integration_task_sqlx(id).await
    }

    pub fn integration_task(&self, id: i64) -> Result<Option<IntegrationTask>> {
        let conn = self.read_conn()?;
        Ok(self.integration_task_by_id_conn(&conn, id)?)
    }

    pub async fn integration_task_sqlx(&self, id: i64) -> Result<Option<IntegrationTask>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.integration_task(id);
        };
        let row = sqlx::query(
            r#"
            SELECT id, external_task_id, source, action, status, request_json, response_json,
                   created_at, updated_at
            FROM integration_tasks
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("failed to load integration task with SQLx")?;
        row.map(|row| self.integration_task_from_sqlx_row(row))
            .transpose()
    }

    pub fn integration_tasks(
        &self,
        source: Option<&str>,
        limit: usize,
    ) -> Result<Vec<IntegrationTask>> {
        let conn = self.read_conn()?;
        let mut tasks = Vec::new();
        if let Some(source) = source {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, external_task_id, source, action, status, request_json, response_json,
                       created_at, updated_at
                FROM integration_tasks
                WHERE source = ?1
                ORDER BY id DESC
                LIMIT ?2
                "#,
            )?;
            let rows = stmt.query_map(params![source, limit as i64], |row| {
                self.integration_task_from_row(row)
            })?;
            for row in rows {
                tasks.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, external_task_id, source, action, status, request_json, response_json,
                       created_at, updated_at
                FROM integration_tasks
                ORDER BY id DESC
                LIMIT ?1
                "#,
            )?;
            let rows = stmt.query_map([limit as i64], |row| self.integration_task_from_row(row))?;
            for row in rows {
                tasks.push(row?);
            }
        }
        Ok(tasks)
    }

    pub async fn integration_tasks_sqlx(
        &self,
        source: Option<&str>,
        limit: usize,
    ) -> Result<Vec<IntegrationTask>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.integration_tasks(source, limit);
        };
        let rows = if let Some(source) = source {
            sqlx::query(
                r#"
                SELECT id, external_task_id, source, action, status, request_json, response_json,
                       created_at, updated_at
                FROM integration_tasks
                WHERE source = ?
                ORDER BY id DESC
                LIMIT ?
                "#,
            )
            .bind(source)
            .bind(limit as i64)
            .fetch_all(pool)
            .await
            .context("failed to list filtered integration tasks with SQLx")?
        } else {
            sqlx::query(
                r#"
                SELECT id, external_task_id, source, action, status, request_json, response_json,
                       created_at, updated_at
                FROM integration_tasks
                ORDER BY id DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(pool)
            .await
            .context("failed to list integration tasks with SQLx")?
        };
        rows.into_iter()
            .map(|row| self.integration_task_from_sqlx_row(row))
            .collect()
    }

    pub fn insert_product_result(&self, result: &ProductResult) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute(
            r#"
            INSERT INTO product_results (batch_id, yield_percent, product_ratio, notes, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(batch_id) DO UPDATE SET
                yield_percent = excluded.yield_percent,
                product_ratio = excluded.product_ratio,
                notes = excluded.notes,
                created_at = excluded.created_at
            "#,
            params![
                result.batch_id,
                result.yield_percent,
                result.product_ratio,
                result.notes,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub async fn insert_product_result_sqlx(&self, result: &ProductResult) -> Result<()> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.insert_product_result(result);
        };
        sqlx::query(
            r#"
            INSERT INTO product_results (batch_id, yield_percent, product_ratio, notes, created_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(batch_id) DO UPDATE SET
                yield_percent = excluded.yield_percent,
                product_ratio = excluded.product_ratio,
                notes = excluded.notes,
                created_at = excluded.created_at
            "#,
        )
        .bind(result.batch_id)
        .bind(result.yield_percent)
        .bind(result.product_ratio)
        .bind(&result.notes)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .context("failed to insert product result with SQLx")?;
        Ok(())
    }

    pub fn insert_recommendation(&self, recommendation: &Recommendation) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute(
            r#"
            INSERT INTO ai_recommendations
                (based_on_batch_count, target_temperature_c, target_stirrer_rpm,
                 heating_minutes, stirring_minutes, expected_score, rationale, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                recommendation.based_on_batch_count,
                recommendation.target_temperature_c,
                recommendation.target_stirrer_rpm,
                recommendation.heating_minutes,
                recommendation.stirring_minutes,
                recommendation.expected_score,
                recommendation.rationale,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub async fn insert_recommendation_sqlx(&self, recommendation: &Recommendation) -> Result<()> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.insert_recommendation(recommendation);
        };
        sqlx::query(
            r#"
            INSERT INTO ai_recommendations
                (based_on_batch_count, target_temperature_c, target_stirrer_rpm,
                 heating_minutes, stirring_minutes, expected_score, rationale, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(recommendation.based_on_batch_count)
        .bind(recommendation.target_temperature_c)
        .bind(recommendation.target_stirrer_rpm)
        .bind(recommendation.heating_minutes)
        .bind(recommendation.stirring_minutes)
        .bind(recommendation.expected_score)
        .bind(&recommendation.rationale)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .context("failed to insert AI recommendation with SQLx")?;
        Ok(())
    }

    pub fn latest_recommendation(&self) -> Result<Option<Recommendation>> {
        let conn = self.read_conn()?;
        conn.query_row(
            r#"
            SELECT based_on_batch_count, target_temperature_c, target_stirrer_rpm,
                   heating_minutes, stirring_minutes, expected_score, rationale
            FROM ai_recommendations
            ORDER BY id DESC
            LIMIT 1
            "#,
            [],
            |row| {
                Ok(Recommendation {
                    based_on_batch_count: row.get(0)?,
                    target_temperature_c: row.get(1)?,
                    target_stirrer_rpm: row.get(2)?,
                    heating_minutes: row.get(3)?,
                    stirring_minutes: row.get(4)?,
                    expected_score: row.get(5)?,
                    rationale: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub async fn latest_recommendation_sqlx(&self) -> Result<Option<Recommendation>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.latest_recommendation();
        };
        let row = sqlx::query(
            r#"
            SELECT based_on_batch_count, target_temperature_c, target_stirrer_rpm,
                   heating_minutes, stirring_minutes, expected_score, rationale
            FROM ai_recommendations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await
        .context("failed to load latest AI recommendation with SQLx")?;
        row.map(recommendation_from_sqlx_row).transpose()
    }

    pub fn batch_outcomes(&self) -> Result<Vec<BatchOutcome>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT b.id, b.target_temperature_c, b.target_stirrer_rpm,
                   b.heating_minutes, b.stirring_minutes,
                   p.yield_percent, p.product_ratio
            FROM batches b
            JOIN product_results p ON p.batch_id = b.id
            ORDER BY b.id ASC
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(BatchOutcome {
                batch_id: row.get(0)?,
                target_temperature_c: row.get(1)?,
                target_stirrer_rpm: row.get(2)?,
                heating_minutes: row.get(3)?,
                stirring_minutes: row.get(4)?,
                yield_percent: row.get(5)?,
                product_ratio: row.get(6)?,
            })
        })?;

        let mut outcomes = Vec::new();
        for row in rows {
            outcomes.push(row?);
        }
        Ok(outcomes)
    }

    pub async fn batch_outcomes_sqlx(&self) -> Result<Vec<BatchOutcome>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.batch_outcomes();
        };
        let rows = sqlx::query(
            r#"
            SELECT b.id, b.target_temperature_c, b.target_stirrer_rpm,
                   b.heating_minutes, b.stirring_minutes,
                   p.yield_percent, p.product_ratio
            FROM batches b
            JOIN product_results p ON p.batch_id = b.id
            ORDER BY b.id ASC
            "#,
        )
        .fetch_all(pool)
        .await
        .context("failed to list batch outcomes with SQLx")?;
        rows.into_iter().map(batch_outcome_from_sqlx_row).collect()
    }

    pub fn recent_batch_outcomes(&self, limit: usize) -> Result<Vec<BatchOutcome>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT b.id, b.target_temperature_c, b.target_stirrer_rpm,
                   b.heating_minutes, b.stirring_minutes,
                   p.yield_percent, p.product_ratio
            FROM (
                SELECT id, target_temperature_c, target_stirrer_rpm,
                       heating_minutes, stirring_minutes
                FROM batches
                WHERE id IN (
                    SELECT b.id
                    FROM batches b
                    JOIN product_results p ON p.batch_id = b.id
                    ORDER BY b.id DESC
                    LIMIT ?1
                )
            ) b
            JOIN product_results p ON p.batch_id = b.id
            ORDER BY b.id ASC
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(BatchOutcome {
                batch_id: row.get(0)?,
                target_temperature_c: row.get(1)?,
                target_stirrer_rpm: row.get(2)?,
                heating_minutes: row.get(3)?,
                stirring_minutes: row.get(4)?,
                yield_percent: row.get(5)?,
                product_ratio: row.get(6)?,
            })
        })?;

        let mut outcomes = Vec::new();
        for row in rows {
            outcomes.push(row?);
        }
        Ok(outcomes)
    }

    pub async fn recent_batch_outcomes_sqlx(&self, limit: usize) -> Result<Vec<BatchOutcome>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.recent_batch_outcomes(limit);
        };
        let rows = sqlx::query(
            r#"
            SELECT b.id, b.target_temperature_c, b.target_stirrer_rpm,
                   b.heating_minutes, b.stirring_minutes,
                   p.yield_percent, p.product_ratio
            FROM (
                SELECT id, target_temperature_c, target_stirrer_rpm,
                       heating_minutes, stirring_minutes
                FROM batches
                WHERE id IN (
                    SELECT b.id
                    FROM batches b
                    JOIN product_results p ON p.batch_id = b.id
                    ORDER BY b.id DESC
                    LIMIT ?
                )
            ) b
            JOIN product_results p ON p.batch_id = b.id
            ORDER BY b.id ASC
            "#,
        )
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .context("failed to list recent batch outcomes with SQLx")?;
        rows.into_iter().map(batch_outcome_from_sqlx_row).collect()
    }

    pub fn recent_batches(&self, limit: usize) -> Result<Vec<Batch>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM (
                SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                       target_stirrer_rpm, heating_minutes, stirring_minutes
                FROM batches
                ORDER BY id DESC
                LIMIT ?1
            )
            ORDER BY id ASC
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let started_at: String = row.get(3)?;
            let finished_at: Option<String> = row.get(4)?;
            Ok(Batch {
                id: row.get(0)?,
                process_id: row.get(1)?,
                name: row.get(2)?,
                started_at: parse_dt(&started_at)?,
                finished_at: match finished_at {
                    Some(value) => Some(parse_dt(&value)?),
                    None => None,
                },
                target_temperature_c: row.get(5)?,
                target_stirrer_rpm: row.get(6)?,
                heating_minutes: row.get(7)?,
                stirring_minutes: row.get(8)?,
            })
        })?;

        let mut batches = Vec::new();
        for row in rows {
            batches.push(row?);
        }
        Ok(batches)
    }

    pub async fn recent_batches_sqlx(&self, limit: usize) -> Result<Vec<Batch>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.recent_batches(limit);
        };
        let rows = sqlx::query(
            r#"
            SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM (
                SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                       target_stirrer_rpm, heating_minutes, stirring_minutes
                FROM batches
                ORDER BY id DESC
                LIMIT ?
            )
            ORDER BY id ASC
            "#,
        )
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .context("failed to list recent batches with SQLx")?;
        rows.into_iter().map(batch_from_sqlx_row).collect()
    }

    pub fn batch_by_id(&self, batch_id: i64) -> Result<Option<Batch>> {
        let conn = self.read_conn()?;
        conn.query_row(
            r#"
            SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM batches
            WHERE id = ?1
            "#,
            [batch_id],
            |row| {
                let started_at: String = row.get(3)?;
                let finished_at: Option<String> = row.get(4)?;
                Ok(Batch {
                    id: row.get(0)?,
                    process_id: row.get(1)?,
                    name: row.get(2)?,
                    started_at: parse_dt(&started_at)?,
                    finished_at: match finished_at {
                        Some(value) => Some(parse_dt(&value)?),
                        None => None,
                    },
                    target_temperature_c: row.get(5)?,
                    target_stirrer_rpm: row.get(6)?,
                    heating_minutes: row.get(7)?,
                    stirring_minutes: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn batch_outcome_by_id(&self, batch_id: i64) -> Result<Option<BatchOutcome>> {
        let conn = self.read_conn()?;
        conn.query_row(
            r#"
            SELECT b.id, b.target_temperature_c, b.target_stirrer_rpm,
                   b.heating_minutes, b.stirring_minutes,
                   p.yield_percent, p.product_ratio
            FROM batches b
            JOIN product_results p ON p.batch_id = b.id
            WHERE b.id = ?1
            "#,
            [batch_id],
            |row| {
                Ok(BatchOutcome {
                    batch_id: row.get(0)?,
                    target_temperature_c: row.get(1)?,
                    target_stirrer_rpm: row.get(2)?,
                    heating_minutes: row.get(3)?,
                    stirring_minutes: row.get(4)?,
                    yield_percent: row.get(5)?,
                    product_ratio: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn sample_records_for_batch(
        &self,
        batch_id: i64,
        limit: usize,
    ) -> Result<Vec<SensorSampleRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM (
                SELECT id, batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                       shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
                FROM sensor_samples
                WHERE batch_id = ?1
                ORDER BY id DESC
                LIMIT ?2
            )
            ORDER BY id ASC
            "#,
        )?;
        let rows = stmt.query_map(params![batch_id, limit as i64], |row| {
            let captured_at: String = row.get(11)?;
            Ok(SensorSampleRecord {
                batch_id: row.get(1)?,
                sample: SensorSnapshot {
                    temperature_c: row.get(2)?,
                    pressure_mpa: row.get(3)?,
                    stirrer_rpm: row.get(4)?,
                    shake_speed_cpm: row.get(5)?,
                    tilt_state: row.get(6)?,
                    tilt_angle_deg: row.get(7)?,
                    flow_rate_l_min: row.get(8)?,
                    product_concentration_percent: row.get(9)?,
                    ph: row.get(10)?,
                    captured_at: parse_dt(&captured_at)?,
                },
            })
        })?;

        let mut samples = Vec::new();
        for row in rows {
            samples.push(row?);
        }
        Ok(samples)
    }

    pub async fn sample_records_for_batch_sqlx(
        &self,
        batch_id: i64,
        limit: usize,
    ) -> Result<Vec<SensorSampleRecord>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.sample_records_for_batch(batch_id, limit);
        };
        let rows = sqlx::query(
            r#"
            SELECT id, batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM (
                SELECT id, batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                       shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
                FROM sensor_samples
                WHERE batch_id = ?
                ORDER BY id DESC
                LIMIT ?
            )
            ORDER BY id ASC
            "#,
        )
        .bind(batch_id)
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .context("failed to list batch sensor samples with SQLx")?;
        rows.into_iter()
            .map(sensor_sample_record_from_sqlx_row)
            .collect()
    }

    pub fn recent_control_events(&self, limit: usize) -> Result<Vec<ControlEvent>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                   target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
            FROM (
                SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                       target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                FROM control_events
                ORDER BY id DESC
                LIMIT ?1
            )
            ORDER BY id ASC
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], control_event_from_row)?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    pub fn audit_events(
        &self,
        limit: usize,
        offset: usize,
        event_type: Option<&str>,
    ) -> Result<Vec<ControlEvent>> {
        let conn = self.read_conn()?;
        let (sql, params): (&str, Vec<Box<dyn rusqlite::ToSql>>) =
            if let Some(event_type) = event_type {
                (
                    r#"
                    SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                           target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                    FROM control_events
                    WHERE event_type = ?1
                    ORDER BY id DESC
                    LIMIT ?2 OFFSET ?3
                    "#,
                    vec![
                        Box::new(event_type.to_string()),
                        Box::new(limit as i64),
                        Box::new(offset as i64),
                    ],
                )
            } else {
                (
                    r#"
                    SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                           target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                    FROM control_events
                    ORDER BY id DESC
                    LIMIT ?1 OFFSET ?2
                    "#,
                    vec![Box::new(limit as i64), Box::new(offset as i64)],
                )
            };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(params.iter().map(|value| value.as_ref())),
            control_event_from_row,
        )?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    pub fn audit_event_count(&self, event_type: Option<&str>) -> Result<usize> {
        let conn = self.read_conn()?;
        if let Some(event_type) = event_type {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM control_events WHERE event_type = ?1",
                [event_type],
                |row| row.get(0),
            )?;
            Ok(count as usize)
        } else {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM control_events", [], |row| row.get(0))?;
            Ok(count as usize)
        }
    }

    pub async fn audit_event_count_sqlx(&self, event_type: Option<&str>) -> Result<usize> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.audit_event_count(event_type);
        };
        let count: i64 = if let Some(event_type) = event_type {
            sqlx::query_scalar("SELECT COUNT(*) FROM control_events WHERE event_type = ?1")
                .bind(event_type)
                .fetch_one(pool)
                .await
                .context("failed to count filtered control events with SQLx")?
        } else {
            sqlx::query_scalar("SELECT COUNT(*) FROM control_events")
                .fetch_one(pool)
                .await
                .context("failed to count control events with SQLx")?
        };
        Ok(count as usize)
    }

    pub async fn audit_events_sqlx(
        &self,
        limit: usize,
        offset: usize,
        event_type: Option<&str>,
    ) -> Result<Vec<ControlEvent>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.audit_events(limit, offset, event_type);
        };
        let rows = if let Some(event_type) = event_type {
            sqlx::query(
                r#"
                SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                       target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                FROM control_events
                WHERE event_type = ?
                ORDER BY id DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(event_type)
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(pool)
            .await
            .context("failed to list filtered control events with SQLx")?
        } else {
            sqlx::query(
                r#"
                SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                       target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                FROM control_events
                ORDER BY id DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(pool)
            .await
            .context("failed to list control events with SQLx")?
        };

        rows.into_iter().map(control_event_from_sqlx_row).collect()
    }

    pub fn audit_chain_status(&self) -> Result<AuditChainStatus> {
        let conn = self.read_conn()?;
        let total_hashed_events: usize = conn.query_row(
            "SELECT COUNT(*) FROM control_events WHERE event_hash IS NOT NULL",
            [],
            |row| row.get::<_, i64>(0),
        )? as usize;
        let verification_truncated = total_hashed_events > AUDIT_CHAIN_CHECK_LIMIT;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                   target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
            FROM (
                SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                       target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                FROM control_events
                WHERE event_hash IS NOT NULL
                ORDER BY id DESC
                LIMIT ?1
            )
            ORDER BY id ASC
            "#,
        )?;
        let rows = stmt.query_map([AUDIT_CHAIN_CHECK_LIMIT as i64], control_event_from_row)?;
        let events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        audit_chain_status_from_events(
            events,
            total_hashed_events,
            AUDIT_CHAIN_CHECK_LIMIT,
            verification_truncated,
        )
    }

    pub async fn audit_chain_status_sqlx(&self) -> Result<AuditChainStatus> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.audit_chain_status();
        };
        let total_hashed_events: usize = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM control_events WHERE event_hash IS NOT NULL",
        )
        .fetch_one(pool)
        .await
        .context("failed to count hashed control events with SQLx")?
            as usize;
        let verification_truncated = total_hashed_events > AUDIT_CHAIN_CHECK_LIMIT;
        let rows = sqlx::query(
            r#"
            SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                   target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
            FROM (
                SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                       target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                FROM control_events
                WHERE event_hash IS NOT NULL
                ORDER BY id DESC
                LIMIT ?
            )
            ORDER BY id ASC
            "#,
        )
        .bind(AUDIT_CHAIN_CHECK_LIMIT as i64)
        .fetch_all(pool)
        .await
        .context("failed to load audit chain window with SQLx")?;
        let events = rows
            .into_iter()
            .map(control_event_from_sqlx_row)
            .collect::<Result<Vec<_>>>()?;
        audit_chain_status_from_events(
            events,
            total_hashed_events,
            AUDIT_CHAIN_CHECK_LIMIT,
            verification_truncated,
        )
    }

    pub fn full_audit_chain_status_for_diagnostics(&self) -> Result<AuditChainStatus> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                   target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
            FROM control_events
            WHERE event_hash IS NOT NULL
            ORDER BY id ASC
            "#,
        )?;
        let rows = stmt.query_map([], control_event_from_row)?;
        let mut checked_events = 0usize;
        let mut broken_events = 0usize;
        let mut previous: Option<String> = None;
        let mut last_event_hash = None;
        let mut checked_from_event_id = None;
        let mut checked_to_event_id = None;
        for row in rows {
            let event = row?;
            checked_from_event_id.get_or_insert(event.id);
            checked_to_event_id = Some(event.id);
            checked_events += 1;
            let expected = control_event_hash(
                previous.as_deref(),
                event.batch_id,
                &event.event_type,
                event.target_temperature_c,
                event.target_stirrer_rpm,
                event.target_shake_speed_cpm,
                &event.reason,
                &event.created_at.to_rfc3339(),
            )?;
            if event.previous_hash != previous || event.event_hash.as_deref() != Some(&expected) {
                broken_events += 1;
            }
            previous = event.event_hash.clone();
            last_event_hash = event.event_hash;
        }
        let window_valid = broken_events == 0;
        Ok(AuditChainStatus {
            total_hashed_events: checked_events,
            checked_events,
            chained_events: checked_events.saturating_sub(broken_events),
            broken_events,
            window_valid,
            valid: window_valid,
            last_event_hash,
            checked_from_event_id,
            checked_to_event_id,
            verification_limit: checked_events,
            verification_truncated: false,
        })
    }

    pub fn control_events_for_batch(
        &self,
        batch_id: i64,
        limit: usize,
    ) -> Result<Vec<ControlEvent>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                   target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
            FROM (
                SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                       target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                FROM control_events
                WHERE batch_id = ?1
                ORDER BY id DESC
                LIMIT ?2
            )
            ORDER BY id ASC
            "#,
        )?;
        let rows = stmt.query_map(params![batch_id, limit as i64], control_event_from_row)?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    pub fn clear_runtime_data_for_tests(&self) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute_batch(
            r#"
            DELETE FROM ai_recommendations;
            DELETE FROM product_results;
            DELETE FROM control_events;
            DELETE FROM demo_alarms;
            DELETE FROM integration_tasks;
            DELETE FROM sensor_samples;
            DELETE FROM batches;
            DELETE FROM process_steps;
            DELETE FROM processes;
            "#,
        )?;
        Ok(())
    }

    #[doc(hidden)]
    pub fn index_names_for_diagnostics(&self) -> Result<Vec<String>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT name
            FROM sqlite_master
            WHERE type = 'index' AND name NOT LIKE 'sqlite_%'
            ORDER BY name ASC
            "#,
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut indexes = Vec::new();
        for row in rows {
            indexes.push(row?);
        }
        Ok(indexes)
    }

    fn write_conn(&self) -> Result<DbConnectionGuard<'_>> {
        self.inner
            .write
            .lock()
            .map(DbConnectionGuard::Write)
            .map_err(|_| anyhow::anyhow!("database write lock poisoned"))
    }

    fn read_conn(&self) -> Result<DbConnectionGuard<'_>> {
        if self.inner.reads.is_empty() {
            return self.write_conn();
        }
        let index = self.inner.next_read.fetch_add(1, Ordering::Relaxed) % self.inner.reads.len();
        self.inner.reads[index]
            .lock()
            .map(DbConnectionGuard::Read)
            .map_err(|_| anyhow::anyhow!("database read lock poisoned"))
    }

    #[cfg(debug_assertions)]
    pub fn break_control_events_for_tests(&self) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute("DROP TABLE control_events", [])?;
        Ok(())
    }

    fn serialize_sensitive_json(&self, value: &Value) -> Result<String> {
        let plaintext = serde_json::to_string(value)?;
        match &self.inner.encryption {
            Some(encryption) => encryption.encrypt_json(&plaintext),
            None => Ok(plaintext),
        }
    }

    fn parse_sensitive_json(&self, value: &str) -> rusqlite::Result<Value> {
        let plaintext = match &self.inner.encryption {
            Some(encryption) => encryption.decrypt_json_if_needed(value)?,
            None if value.starts_with(ENCRYPTED_JSON_PREFIX) => {
                return Err(rusqlite_conversion_error(anyhow::anyhow!(
                    "{DB_ENCRYPTION_KEY_ENV} is required to read encrypted integration task payloads"
                )));
            }
            None => value.to_string(),
        };
        parse_json_value(&plaintext)
    }

    fn parse_sensitive_json_anyhow(&self, value: &str) -> Result<Value> {
        let plaintext = match &self.inner.encryption {
            Some(encryption) => encryption.decrypt_json_if_needed_anyhow(value)?,
            None if value.starts_with(ENCRYPTED_JSON_PREFIX) => {
                anyhow::bail!(
                    "{DB_ENCRYPTION_KEY_ENV} is required to read encrypted integration task payloads"
                );
            }
            None => value.to_string(),
        };
        serde_json::from_str(&plaintext)
            .with_context(|| "failed to parse integration task JSON payload")
    }

    fn integration_task_by_id_conn(
        &self,
        conn: &Connection,
        id: i64,
    ) -> rusqlite::Result<Option<IntegrationTask>> {
        conn.query_row(
            r#"
            SELECT id, external_task_id, source, action, status, request_json, response_json,
                   created_at, updated_at
            FROM integration_tasks
            WHERE id = ?1
            "#,
            [id],
            |row| self.integration_task_from_row(row),
        )
        .optional()
    }

    fn integration_task_from_row(
        &self,
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<IntegrationTask> {
        let request_json: String = row.get(5)?;
        let response_json: String = row.get(6)?;
        let created_at: String = row.get(7)?;
        let updated_at: String = row.get(8)?;
        Ok(IntegrationTask {
            id: row.get(0)?,
            external_task_id: row.get(1)?,
            source: row.get(2)?,
            action: row.get(3)?,
            status: row.get(4)?,
            request: self.parse_sensitive_json(&request_json)?,
            response: self.parse_sensitive_json(&response_json)?,
            created_at: parse_dt(&created_at)?,
            updated_at: parse_dt(&updated_at)?,
        })
    }

    fn integration_task_from_sqlx_row(&self, row: SqliteRow) -> Result<IntegrationTask> {
        let request_json: String = row.try_get("request_json")?;
        let response_json: String = row.try_get("response_json")?;
        let created_at: String = row.try_get("created_at")?;
        let updated_at: String = row.try_get("updated_at")?;
        Ok(IntegrationTask {
            id: row.try_get("id")?,
            external_task_id: row.try_get("external_task_id")?,
            source: row.try_get("source")?,
            action: row.try_get("action")?,
            status: row.try_get("status")?,
            request: self.parse_sensitive_json_anyhow(&request_json)?,
            response: self.parse_sensitive_json_anyhow(&response_json)?,
            created_at: parse_dt_anyhow(&created_at)?,
            updated_at: parse_dt_anyhow(&updated_at)?,
        })
    }

    async fn process_summary_by_id_sqlx_pool(
        &self,
        pool: &sqlx::SqlitePool,
        process_id: i64,
    ) -> Result<Option<ProcessDefinition>> {
        process_summary_by_id_sqlx_executor(pool, process_id).await
    }

    async fn process_summary_by_id_sqlx_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        process_id: i64,
    ) -> Result<Option<ProcessDefinition>> {
        process_summary_by_id_sqlx_executor(&mut **tx, process_id).await
    }
}

fn process_definition_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProcessDefinition> {
    let created_at: String = row.get(6)?;
    let updated_at: String = row.get(7)?;
    let applied_at: Option<String> = row.get(8)?;
    Ok(ProcessDefinition {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        status: row.get(3)?,
        version: row.get(4)?,
        step_count: row.get(5)?,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
        applied_at: match applied_at {
            Some(value) => Some(parse_dt(&value)?),
            None => None,
        },
    })
}

fn process_definition_from_sqlx_row(row: SqliteRow) -> Result<ProcessDefinition> {
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    let applied_at: Option<String> = row.try_get("applied_at")?;
    Ok(ProcessDefinition {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        status: row.try_get("status")?,
        version: row.try_get("version")?,
        step_count: row.try_get("step_count")?,
        created_at: parse_dt_anyhow(&created_at)?,
        updated_at: parse_dt_anyhow(&updated_at)?,
        applied_at: applied_at.as_deref().map(parse_dt_anyhow).transpose()?,
    })
}

async fn process_summary_by_id_sqlx_executor<'e, E>(
    executor: E,
    process_id: i64,
) -> Result<Option<ProcessDefinition>>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let row = sqlx::query(
        r#"
        SELECT p.id, p.name, p.description, p.status, p.version,
               COUNT(s.id) AS step_count, p.created_at, p.updated_at, p.applied_at
        FROM processes p
        LEFT JOIN process_steps s ON s.process_id = p.id
        WHERE p.id = ?
        GROUP BY p.id
        "#,
    )
    .bind(process_id)
    .fetch_optional(executor)
    .await
    .context("failed to read process summary with SQLx")?;
    row.map(process_definition_from_sqlx_row).transpose()
}

fn control_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ControlEvent> {
    let created_at: String = row.get(7)?;
    Ok(ControlEvent {
        id: row.get(0)?,
        batch_id: row.get(1)?,
        event_type: row.get(2)?,
        target_temperature_c: row.get(3)?,
        target_stirrer_rpm: row.get(4)?,
        target_shake_speed_cpm: row.get(5)?,
        reason: row.get(6)?,
        created_at: parse_dt(&created_at)?,
        previous_hash: row.get(8)?,
        event_hash: row.get(9)?,
    })
}

fn control_event_from_sqlx_row(row: SqliteRow) -> Result<ControlEvent> {
    let created_at: String = row.try_get("created_at")?;
    Ok(ControlEvent {
        id: row.try_get("id")?,
        batch_id: row.try_get("batch_id")?,
        event_type: row.try_get("event_type")?,
        target_temperature_c: row.try_get("target_temperature_c")?,
        target_stirrer_rpm: row.try_get("target_stirrer_rpm")?,
        target_shake_speed_cpm: row.try_get("target_shake_speed_cpm")?,
        reason: row.try_get("reason")?,
        created_at: parse_dt_anyhow(&created_at)?,
        previous_hash: row.try_get("previous_hash")?,
        event_hash: row.try_get("event_hash")?,
    })
}

fn batch_from_sqlx_row(row: SqliteRow) -> Result<Batch> {
    let started_at: String = row.try_get("started_at")?;
    let finished_at: Option<String> = row.try_get("finished_at")?;
    Ok(Batch {
        id: row.try_get("id")?,
        process_id: row.try_get("process_id")?,
        name: row.try_get("name")?,
        started_at: parse_dt_anyhow(&started_at)?,
        finished_at: finished_at.as_deref().map(parse_dt_anyhow).transpose()?,
        target_temperature_c: row.try_get("target_temperature_c")?,
        target_stirrer_rpm: row.try_get("target_stirrer_rpm")?,
        heating_minutes: row.try_get("heating_minutes")?,
        stirring_minutes: row.try_get("stirring_minutes")?,
    })
}

fn batch_outcome_from_sqlx_row(row: SqliteRow) -> Result<BatchOutcome> {
    Ok(BatchOutcome {
        batch_id: row.try_get("id")?,
        target_temperature_c: row.try_get("target_temperature_c")?,
        target_stirrer_rpm: row.try_get("target_stirrer_rpm")?,
        heating_minutes: row.try_get("heating_minutes")?,
        stirring_minutes: row.try_get("stirring_minutes")?,
        yield_percent: row.try_get("yield_percent")?,
        product_ratio: row.try_get("product_ratio")?,
    })
}

fn recommendation_from_sqlx_row(row: SqliteRow) -> Result<Recommendation> {
    let based_on_batch_count: i64 = row.try_get("based_on_batch_count")?;
    if based_on_batch_count < 0 {
        anyhow::bail!("invalid based_on_batch_count in database: {based_on_batch_count}");
    }
    Ok(Recommendation {
        based_on_batch_count,
        target_temperature_c: row.try_get("target_temperature_c")?,
        target_stirrer_rpm: row.try_get("target_stirrer_rpm")?,
        heating_minutes: row.try_get("heating_minutes")?,
        stirring_minutes: row.try_get("stirring_minutes")?,
        expected_score: row.try_get("expected_score")?,
        rationale: row.try_get("rationale")?,
    })
}

fn sensor_sample_record_from_sqlx_row(row: SqliteRow) -> Result<SensorSampleRecord> {
    let captured_at: String = row.try_get("captured_at")?;
    let tilt_state: i64 = row.try_get("tilt_state")?;
    Ok(SensorSampleRecord {
        batch_id: row.try_get("batch_id")?,
        sample: SensorSnapshot {
            temperature_c: row.try_get("temperature_c")?,
            pressure_mpa: row.try_get("pressure_mpa")?,
            stirrer_rpm: row.try_get("stirrer_rpm")?,
            shake_speed_cpm: row.try_get("shake_speed_cpm")?,
            tilt_state: u8::try_from(tilt_state)
                .with_context(|| format!("invalid tilt_state in database: {tilt_state}"))?,
            tilt_angle_deg: row.try_get("tilt_angle_deg")?,
            flow_rate_l_min: row.try_get("flow_rate_l_min")?,
            product_concentration_percent: row.try_get("product_concentration_percent")?,
            ph: row.try_get("ph")?,
            captured_at: parse_dt_anyhow(&captured_at)?,
        },
    })
}

fn audit_chain_status_from_events(
    events: Vec<ControlEvent>,
    total_hashed_events: usize,
    verification_limit: usize,
    verification_truncated: bool,
) -> Result<AuditChainStatus> {
    let mut checked_events = 0usize;
    let mut broken_events = 0usize;
    let mut previous: Option<String> = None;
    let mut last_event_hash = None;
    let mut checked_from_event_id = None;
    let mut checked_to_event_id = None;
    for event in events {
        checked_from_event_id.get_or_insert(event.id);
        checked_to_event_id = Some(event.id);
        if checked_events == 0 && verification_truncated {
            previous = event.previous_hash.clone();
        }
        checked_events += 1;
        let expected = control_event_hash(
            previous.as_deref(),
            event.batch_id,
            &event.event_type,
            event.target_temperature_c,
            event.target_stirrer_rpm,
            event.target_shake_speed_cpm,
            &event.reason,
            &event.created_at.to_rfc3339(),
        )?;
        if event.previous_hash != previous || event.event_hash.as_deref() != Some(&expected) {
            broken_events += 1;
        }
        previous = event.event_hash.clone();
        last_event_hash = event.event_hash;
    }
    let window_valid = broken_events == 0;
    Ok(AuditChainStatus {
        total_hashed_events,
        checked_events,
        chained_events: checked_events.saturating_sub(broken_events),
        broken_events,
        window_valid,
        valid: window_valid && !verification_truncated,
        last_event_hash,
        checked_from_event_id,
        checked_to_event_id,
        verification_limit,
        verification_truncated,
    })
}

fn control_event_hash(
    previous_hash: Option<&str>,
    batch_id: Option<i64>,
    event_type: &str,
    target_temperature_c: Option<f64>,
    target_stirrer_rpm: Option<f64>,
    target_shake_speed_cpm: Option<f64>,
    reason: &str,
    created_at: &str,
) -> Result<String> {
    let payload = json!({
        "previous_hash": previous_hash,
        "batch_id": batch_id,
        "event_type": event_type,
        "target_temperature_c": target_temperature_c,
        "target_stirrer_rpm": target_stirrer_rpm,
        "target_shake_speed_cpm": target_shake_speed_cpm,
        "reason": reason,
        "created_at": created_at
    });
    let bytes = serde_json::to_vec(&payload)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

fn process_summary_by_id(
    conn: &Connection,
    process_id: i64,
) -> rusqlite::Result<Option<ProcessDefinition>> {
    conn.query_row(
        r#"
        SELECT p.id, p.name, p.description, p.status, p.version,
               COUNT(s.id) AS step_count, p.created_at, p.updated_at, p.applied_at
        FROM processes p
        LEFT JOIN process_steps s ON s.process_id = p.id
        WHERE p.id = ?1
        GROUP BY p.id
        "#,
        [process_id],
        process_definition_from_row,
    )
    .optional()
}

fn process_steps_for_conn(
    conn: &Connection,
    process_id: i64,
) -> rusqlite::Result<Vec<ProcessStep>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, process_id, step_index, name, target_temperature_c, ramp_rate_c_min,
               duration_minutes, target_stirrer_rpm, target_shake_speed_cpm,
               target_pressure_mpa, cooling_mode, created_at, updated_at
        FROM process_steps
        WHERE process_id = ?1
        ORDER BY step_index ASC, id ASC
        "#,
    )?;
    let rows = stmt.query_map([process_id], process_step_from_row)?;
    let mut steps = Vec::new();
    for row in rows {
        steps.push(row?);
    }
    Ok(steps)
}

async fn process_steps_for_pool_sqlx(
    pool: &sqlx::SqlitePool,
    process_id: i64,
) -> Result<Vec<ProcessStep>> {
    let rows = sqlx::query(
        r#"
        SELECT id, process_id, step_index, name, target_temperature_c, ramp_rate_c_min,
               duration_minutes, target_stirrer_rpm, target_shake_speed_cpm,
               target_pressure_mpa, cooling_mode, created_at, updated_at
        FROM process_steps
        WHERE process_id = ?
        ORDER BY step_index ASC, id ASC
        "#,
    )
    .bind(process_id)
    .fetch_all(pool)
    .await
    .context("failed to read process steps with SQLx")?;
    rows.into_iter().map(process_step_from_sqlx_row).collect()
}

fn process_step_by_id(conn: &Connection, step_id: i64) -> rusqlite::Result<Option<ProcessStep>> {
    conn.query_row(
        r#"
        SELECT id, process_id, step_index, name, target_temperature_c, ramp_rate_c_min,
               duration_minutes, target_stirrer_rpm, target_shake_speed_cpm,
               target_pressure_mpa, cooling_mode, created_at, updated_at
        FROM process_steps
        WHERE id = ?1
        "#,
        [step_id],
        process_step_from_row,
    )
    .optional()
}

async fn process_step_by_id_sqlx_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    step_id: i64,
) -> Result<Option<ProcessStep>> {
    let row = sqlx::query(
        r#"
        SELECT id, process_id, step_index, name, target_temperature_c, ramp_rate_c_min,
               duration_minutes, target_stirrer_rpm, target_shake_speed_cpm,
               target_pressure_mpa, cooling_mode, created_at, updated_at
        FROM process_steps
        WHERE id = ?
        "#,
    )
    .bind(step_id)
    .fetch_optional(&mut **tx)
    .await
    .context("failed to read process step with SQLx")?;
    row.map(process_step_from_sqlx_row).transpose()
}

fn process_step_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProcessStep> {
    let created_at: String = row.get(11)?;
    let updated_at: String = row.get(12)?;
    Ok(ProcessStep {
        id: row.get(0)?,
        process_id: row.get(1)?,
        step_index: row.get(2)?,
        name: row.get(3)?,
        target_temperature_c: row.get(4)?,
        ramp_rate_c_min: row.get(5)?,
        duration_minutes: row.get(6)?,
        target_stirrer_rpm: row.get(7)?,
        target_shake_speed_cpm: row.get(8)?,
        target_pressure_mpa: row.get(9)?,
        cooling_mode: row.get(10)?,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn process_step_from_sqlx_row(row: SqliteRow) -> Result<ProcessStep> {
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    Ok(ProcessStep {
        id: row.try_get("id")?,
        process_id: row.try_get("process_id")?,
        step_index: row.try_get("step_index")?,
        name: row.try_get("name")?,
        target_temperature_c: row.try_get("target_temperature_c")?,
        ramp_rate_c_min: row.try_get("ramp_rate_c_min")?,
        duration_minutes: row.try_get("duration_minutes")?,
        target_stirrer_rpm: row.try_get("target_stirrer_rpm")?,
        target_shake_speed_cpm: row.try_get("target_shake_speed_cpm")?,
        target_pressure_mpa: row.try_get("target_pressure_mpa")?,
        cooling_mode: row.try_get("cooling_mode")?,
        created_at: parse_dt_anyhow(&created_at)?,
        updated_at: parse_dt_anyhow(&updated_at)?,
    })
}

fn touch_process(conn: &Connection, process_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE processes SET updated_at = ?1 WHERE id = ?2",
        params![Utc::now().to_rfc3339(), process_id],
    )?;
    Ok(())
}

async fn touch_process_sqlx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    process_id: i64,
) -> Result<()> {
    sqlx::query("UPDATE processes SET updated_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(process_id)
        .execute(&mut **tx)
        .await
        .context("failed to touch process with SQLx")?;
    Ok(())
}

fn open_configured_connection(path: &Path) -> Result<Connection> {
    configure_connection(Connection::open(path)?)
}

fn open_sqlx_pool(path: &Path) -> Option<sqlx::SqlitePool> {
    if tokio::runtime::Handle::try_current().is_err() {
        return None;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(SQLITE_BUSY_TIMEOUT);
    Some(
        SqlitePoolOptions::new()
            .max_connections((READ_CONNECTIONS + 1) as u32)
            .connect_lazy_with(options),
    )
}

fn configure_connection(conn: Connection) -> Result<Connection> {
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys=ON;
        PRAGMA journal_mode=WAL;
        "#,
    )?;
    Ok(conn)
}

fn create_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(INDEX_SQL)?;
    Ok(())
}

impl DbEncryption {
    fn from_key(key: [u8; 32], key_source: &'static str) -> Self {
        Self {
            cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key)),
            key_source,
        }
    }

    fn encrypt_json(&self, plaintext: &str) -> Result<String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: DB_ENCRYPTION_AAD,
                },
            )
            .map_err(|err| anyhow::anyhow!("failed to encrypt integration task payload: {err}"))?;
        Ok(format!(
            "{}{}:{}",
            ENCRYPTED_JSON_PREFIX,
            STANDARD_NO_PAD.encode(nonce),
            STANDARD_NO_PAD.encode(ciphertext)
        ))
    }

    fn decrypt_json_if_needed(&self, value: &str) -> rusqlite::Result<String> {
        self.decrypt_json_if_needed_anyhow(value)
            .map_err(rusqlite_conversion_error)
    }

    fn decrypt_json_if_needed_anyhow(&self, value: &str) -> Result<String> {
        let Some(envelope) = value.strip_prefix(ENCRYPTED_JSON_PREFIX) else {
            return Ok(value.to_string());
        };
        let Some((nonce, ciphertext)) = envelope.split_once(':') else {
            anyhow::bail!("encrypted integration task payload envelope is malformed");
        };
        let nonce = decode_base64_anyhow(nonce)?;
        let ciphertext = decode_base64_anyhow(ciphertext)?;
        if nonce.len() != 12 {
            anyhow::bail!("encrypted integration task payload nonce must be 12 bytes");
        }
        let plaintext = self
            .cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: DB_ENCRYPTION_AAD,
                },
            )
            .map_err(|err| anyhow::anyhow!("failed to decrypt integration task payload: {err}"))?;
        String::from_utf8(plaintext).map_err(|err| {
            anyhow::anyhow!("decrypted integration task payload was not utf-8: {err}")
        })
    }
}

fn db_encryption_from_env() -> Result<Option<DbEncryption>> {
    match env::var(DB_ENCRYPTION_KEY_ENV) {
        Ok(value) => {
            let key = parse_encryption_key(&value).with_context(|| {
                format!(
                    "{DB_ENCRYPTION_KEY_ENV} must be 32 bytes, 64 hex characters, or base64-encoded 32 bytes"
                )
            })?;
            Ok(Some(DbEncryption::from_key(key, DB_ENCRYPTION_KEY_ENV)))
        }
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(anyhow::anyhow!(
            "{DB_ENCRYPTION_KEY_ENV} must be valid unicode"
        )),
    }
}

fn parse_encryption_key(value: &str) -> Result<[u8; 32]> {
    let trimmed = value.trim();
    if trimmed.len() == 32 {
        return bytes_to_key(trimmed.as_bytes());
    }
    if trimmed.len() == 64
        && trimmed
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return bytes_to_key(&decode_hex(trimmed)?);
    }
    if let Ok(decoded) = STANDARD.decode(trimmed) {
        return bytes_to_key(&decoded);
    }
    if let Ok(decoded) = STANDARD_NO_PAD.decode(trimmed) {
        return bytes_to_key(&decoded);
    }
    anyhow::bail!("invalid key encoding")
}

fn bytes_to_key(bytes: &[u8]) -> Result<[u8; 32]> {
    if bytes.len() != 32 {
        anyhow::bail!("expected 32 bytes, got {}", bytes.len());
    }
    let mut key = [0_u8; 32];
    key.copy_from_slice(bytes);
    Ok(key)
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for index in (0..value.len()).step_by(2) {
        let byte = u8::from_str_radix(&value[index..index + 2], 16)?;
        bytes.push(byte);
    }
    Ok(bytes)
}

fn decode_base64_anyhow(value: &str) -> Result<Vec<u8>> {
    STANDARD_NO_PAD
        .decode(value)
        .or_else(|_| STANDARD.decode(value))
        .map_err(|err| {
            anyhow::anyhow!("encrypted integration task payload base64 decode failed: {err}")
        })
}

fn parse_json_value(value: &str) -> rusqlite::Result<Value> {
    serde_json::from_str(value).map_err(|err| rusqlite_conversion_error(err))
}

fn parse_dt(value: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })
}

fn parse_dt_anyhow(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .with_context(|| format!("invalid RFC3339 timestamp in database: {value}"))
}

fn rusqlite_conversion_error(
    err: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, err.into())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    if column_exists(conn, table, column)? {
        return Ok(());
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )?;
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}
