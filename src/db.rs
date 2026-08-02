use std::{
    env, fmt,
    io::Read,
    path::{Path, PathBuf},
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
use anyhow::{anyhow, Context, Result};
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
    sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
    },
    Row,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    control::SafeCommand,
    optimizer::Recommendation,
    state::{validate_sensor_snapshot, SensorRange, SensorSnapshot},
};

const READ_CONNECTIONS: usize = 2;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(15);
const SQLITE_MMAP_SIZE_BYTES: i64 = 64 * 1024 * 1024;
pub const DB_ENCRYPTION_KEY_ENV: &str = "XINGSHU_DB_ENCRYPTION_KEY";
pub const ENCRYPTED_JSON_PREFIX: &str = "xingshu:v1:aes256gcm:";
const DB_ENCRYPTION_AAD: &[u8] = b"xingshu:integration_tasks:json:v1";
const AUDIT_CHAIN_CHECK_LIMIT: usize = 10_000;
const TARGET_TEMPERATURE_C_RANGE: SensorRange = SensorRange {
    field: "target_temperature_c",
    min: 0.0,
    max: 500.0,
};
const TARGET_STIRRER_RPM_RANGE: SensorRange = SensorRange {
    field: "target_stirrer_rpm",
    min: 0.0,
    max: 2000.0,
};
const BATCH_HEATING_MINUTES_RANGE: SensorRange = SensorRange {
    field: "heating_minutes",
    min: 0.0,
    max: 1440.0,
};
const BATCH_STIRRING_MINUTES_RANGE: SensorRange = SensorRange {
    field: "stirring_minutes",
    min: 0.0,
    max: 1440.0,
};
const PROCESS_RAMP_RATE_C_MIN_RANGE: SensorRange = SensorRange {
    field: "ramp_rate_c_min",
    min: -20.0,
    max: 20.0,
};
const PROCESS_DURATION_MINUTES_RANGE: SensorRange = SensorRange {
    field: "duration_minutes",
    min: 1.0,
    max: 1440.0,
};
const PROCESS_SHAKE_SPEED_CPM_RANGE: SensorRange = SensorRange {
    field: "target_shake_speed_cpm",
    min: 0.0,
    max: 60.0,
};
const PROCESS_PRESSURE_MPA_RANGE: SensorRange = SensorRange {
    field: "target_pressure_mpa",
    min: 0.0,
    max: 10.0,
};
const PRODUCT_RESULT_YIELD_PERCENT_RANGE: SensorRange = SensorRange {
    field: "yield_percent",
    min: 0.0,
    max: 100.0,
};
const PRODUCT_RESULT_RATIO_RANGE: SensorRange = SensorRange {
    field: "product_ratio",
    min: 0.0,
    max: 1.0,
};
const RECOMMENDATION_EXPECTED_SCORE_RANGE: SensorRange = SensorRange {
    field: "expected_score",
    min: 0.0,
    max: 100.0,
};
const INTEGRATION_TASK_SOURCE_MAX_CHARS: usize = 40;
const INTEGRATION_TASK_EXTERNAL_ID_MAX_CHARS: usize = 120;
const INTEGRATION_TASK_ACTIONS: &[&str] = &["set_targets", "start_process", "stop_process"];
const INTEGRATION_TASK_STATUSES: &[&str] =
    &["received", "executing", "executed", "failed", "rejected"];
const INTEGRATION_TASK_TERMINAL_STATUSES: &[&str] = &["executed", "failed", "rejected"];
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
CREATE UNIQUE INDEX IF NOT EXISTS idx_integration_tasks_unique_active_external_task_id
    ON integration_tasks(source, external_task_id)
    WHERE external_task_id IS NOT NULL
      AND status IN ('received', 'executing', 'executed');
"#;
const COLUMN_MIGRATIONS: [(&str, &str, &str); 19] = [
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
    ("integration_tasks", "external_task_id", "TEXT"),
    (
        "integration_tasks",
        "source",
        "TEXT NOT NULL DEFAULT 'legacy'",
    ),
    (
        "integration_tasks",
        "action",
        "TEXT NOT NULL DEFAULT 'set_targets'",
    ),
    (
        "integration_tasks",
        "status",
        "TEXT NOT NULL DEFAULT 'failed'",
    ),
    (
        "integration_tasks",
        "request_json",
        "TEXT NOT NULL DEFAULT '{}'",
    ),
    (
        "integration_tasks",
        "response_json",
        r#"TEXT NOT NULL DEFAULT '{"status":"failed","error":"legacy task migrated without original response"}'"#,
    ),
    (
        "integration_tasks",
        "created_at",
        "TEXT NOT NULL DEFAULT '1970-01-01T00:00:00+00:00'",
    ),
    (
        "integration_tasks",
        "updated_at",
        "TEXT NOT NULL DEFAULT '1970-01-01T00:00:00+00:00'",
    ),
];

#[derive(Clone)]
pub struct Db {
    inner: Arc<DbInner>,
}

#[derive(Clone)]
pub struct DbEncryption {
    cipher: Aes256Gcm,
    key_source: &'static str,
}

struct DbInner {
    write: Mutex<Connection>,
    reads: Vec<Mutex<Connection>>,
    next_read: AtomicUsize,
    encryption: Option<DbEncryption>,
    sqlx_pool: Option<sqlx::SqlitePool>,
    sqlx_write_lock: AsyncMutex<()>,
    audit_write_lock: AsyncMutex<()>,
    process_write_lock: AsyncMutex<()>,
    #[cfg(debug_assertions)]
    fail_control_events_after: AtomicUsize,
    #[cfg(debug_assertions)]
    after_control_event_success: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Path the main database file was opened from, captured by the
    /// `open_*` constructors so backup / restore can report it without
    /// re-deriving it from the runtime state.
    db_path: Option<PathBuf>,
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

impl std::ops::DerefMut for DbConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
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
        let path_ref = path.as_ref();
        if let Some(parent) = path_ref.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory {}", parent.display())
            })?;
        }
        let write = open_configured_connection(path_ref)
            .with_context(|| format!("failed to open database {}", path_ref.display()))?;
        let mut reads = Vec::with_capacity(READ_CONNECTIONS);
        for _ in 0..READ_CONNECTIONS {
            reads.push(Mutex::new(
                open_configured_connection(path_ref).with_context(|| {
                    format!("failed to open database reader {}", path_ref.display())
                })?,
            ));
        }
        let sqlx_pool = open_sqlx_pool(path_ref);
        let db = Self::from_connections_with_path(
            write,
            reads,
            encryption,
            sqlx_pool,
            Some(path_ref.to_path_buf()),
        );
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

    fn from_connections_with_path(
        write: Connection,
        reads: Vec<Mutex<Connection>>,
        encryption: Option<DbEncryption>,
        sqlx_pool: Option<sqlx::SqlitePool>,
        db_path: Option<PathBuf>,
    ) -> Self {
        Self {
            inner: Arc::new(DbInner {
                write: Mutex::new(write),
                reads,
                next_read: AtomicUsize::new(0),
                encryption,
                sqlx_pool,
                sqlx_write_lock: AsyncMutex::new(()),
                audit_write_lock: AsyncMutex::new(()),
                process_write_lock: AsyncMutex::new(()),
                #[cfg(debug_assertions)]
                fail_control_events_after: AtomicUsize::new(usize::MAX),
                #[cfg(debug_assertions)]
                after_control_event_success: Mutex::new(None),
                db_path,
            }),
        }
    }

    fn from_connections(
        write: Connection,
        reads: Vec<Mutex<Connection>>,
        encryption: Option<DbEncryption>,
        sqlx_pool: Option<sqlx::SqlitePool>,
    ) -> Self {
        Self::from_connections_with_path(write, reads, encryption, sqlx_pool, None)
    }

    pub fn migrate(&self) -> Result<()> {
        let mut conn = self.write_conn()?;
        conn.execute_batch(SCHEMA_SQL)
            .context("failed to create base SQLite schema before migrations")?;
        let tx = conn
            .transaction()
            .context("failed to begin SQLite schema migration transaction")?;
        let has_legacy_pressure_kpa = column_exists(&tx, "sensor_samples", "pressure_kpa")?;
        for migration in COLUMN_MIGRATIONS {
            add_column_if_missing(&tx, migration.0, migration.1, migration.2).with_context(
                || {
                    format!(
                        "failed to migrate SQLite column {}.{}",
                        migration.0, migration.1
                    )
                },
            )?;
        }
        if has_legacy_pressure_kpa {
            tx.execute(
                "UPDATE sensor_samples SET pressure_mpa = pressure_kpa / 1000.0 WHERE pressure_mpa = 0 AND pressure_kpa > 0",
                [],
            )?;
        }
        prepare_integration_task_unique_index(&tx)?;
        create_indexes(&tx).context("failed to create SQLite indexes after column migrations")?;
        tx.commit()
            .context("failed to commit SQLite schema migration transaction")?;
        Ok(())
    }

    /// Take an online SQLite backup of the main database file. The
    /// implementation uses the `VACUUM INTO '<path>'` statement which
    /// SQLite documents as a safe online backup command — it rewrites
    /// the entire database into the destination file in a single
    /// transaction, and writers continue to operate against the source
    /// until the moment of the swap. Unlike `fs::copy`, the resulting
    /// file is a fully compacted SQLite image, not whatever happened to
    /// be on disk mid-transaction.
    pub fn backup_to(&self, destination: &Path) -> Result<BackupReport> {
        if let Some(parent) = destination.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create backup parent dir {}", parent.display())
                })?;
            }
        }
        // VACUUM INTO is a one-shot statement; quote the path with the
        // single-quote doubling convention so an operator-managed
        // directory containing an apostrophe does not break the SQL.
        let path_str = destination.to_string_lossy().replace('\'', "''");
        let sql = format!("VACUUM INTO '{path_str}'");
        let started = std::time::Instant::now();
        let conn = self.write_conn()?;
        conn.execute_batch(&sql)
            .with_context(|| format!("VACUUM INTO failed for {}", destination.display()))?;
        let size = std::fs::metadata(destination).map(|m| m.len()).unwrap_or(0);
        Ok(BackupReport {
            source: self.path_display(),
            destination: destination.display().to_string(),
            copied_pages: -1,
            size_bytes: size,
            duration_ms: started.elapsed().as_millis(),
            sha256: sha256_hex(destination)?,
        })
    }

    /// Restore a SQLite file from a backup image. This is intended for
    /// disaster recovery after the daemon has been stopped: it validates
    /// the source image, preserves the existing main DB file, removes
    /// stale WAL/SHM/JOURNAL sidecars, copies the backup into place, and
    /// opens the restored DB to verify the schema and integrity check.
    pub fn restore_file(
        source: &Path,
        destination: &Path,
        overwrite: bool,
    ) -> Result<RestoreReport> {
        if !source.is_file() {
            return Err(anyhow!(
                "restore source {} does not exist",
                source.display()
            ));
        }
        if destination.exists() && !overwrite {
            return Err(anyhow!(
                "refusing to restore over an existing database without overwrite = true: {}",
                destination.display()
            ));
        }
        let mut magic = [0u8; 16];
        std::fs::File::open(source)
            .with_context(|| format!("failed to open {}", source.display()))?
            .read_exact(&mut magic)
            .with_context(|| format!("failed to read SQLite magic from {}", source.display()))?;
        if magic != *b"SQLite format 3\0" {
            return Err(anyhow!(
                "restore source {} is not a SQLite file (magic header missing)",
                source.display()
            ));
        }

        if let Some(parent) = destination.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create restore target dir {}", parent.display())
                })?;
            }
        }

        let restore_tmp = restore_tmp_path(destination);
        remove_restore_tmp_if_present(&restore_tmp)?;
        let restore_result = (|| {
            std::fs::copy(source, &restore_tmp).with_context(|| {
                format!(
                    "failed to copy {} -> temporary restore file {}",
                    source.display(),
                    restore_tmp.display()
                )
            })?;
            sync_file(&restore_tmp).with_context(|| {
                format!(
                    "failed to sync temporary restore file {}",
                    restore_tmp.display()
                )
            })?;
            let (integrity, tables) = validate_restored_db_file(&restore_tmp)?;
            Ok((integrity, tables))
        })();
        let (integrity, tables) = match restore_result {
            Ok(result) => result,
            Err(err) => {
                let _ = std::fs::remove_file(&restore_tmp);
                return Err(err);
            }
        };

        let publish_result = (|| {
            let mut preserved_existing = None;
            if destination.exists() {
                let backup_existing =
                    unique_path(path_with_file_suffix(destination, "pre-restore"))?;
                copy_evidence_file_atomic(destination, &backup_existing, "existing db")?;
                preserved_existing = Some(backup_existing.display().to_string());
            }

            let mut removed_sidecars = Vec::new();
            let mut preserved_sidecars = Vec::new();
            for suffix in ["-wal", "-shm", "-journal"] {
                let sidecar = path_with_raw_suffix(destination, suffix);
                if sidecar.exists() {
                    let preserved_sidecar = unique_path(path_with_raw_suffix(
                        destination,
                        &format!("{suffix}.pre-restore"),
                    ))?;
                    copy_evidence_file_atomic(&sidecar, &preserved_sidecar, "SQLite sidecar")?;
                    std::fs::remove_file(&sidecar).with_context(|| {
                        format!(
                            "failed to remove stale SQLite sidecar {}",
                            sidecar.display()
                        )
                    })?;
                    preserved_sidecars.push(preserved_sidecar.display().to_string());
                    removed_sidecars.push(sidecar.display().to_string());
                }
            }

            std::fs::rename(&restore_tmp, destination).with_context(|| {
                format!("failed to publish restored db {}", destination.display())
            })?;
            sync_parent_dir(destination).with_context(|| {
                format!(
                    "restored db {} was published but directory sync failed; target_may_have_changed=true",
                    destination.display()
                )
            })?;
            Ok((preserved_existing, removed_sidecars, preserved_sidecars))
        })();
        let (preserved_existing, removed_sidecars, preserved_sidecars) = match publish_result {
            Ok(result) => result,
            Err(err) => {
                let _ = std::fs::remove_file(&restore_tmp);
                return Err(err);
            }
        };
        Ok(RestoreReport {
            source: source.display().to_string(),
            destination: destination.display().to_string(),
            preserved_existing,
            removed_sidecars,
            preserved_sidecars,
            integrity_check: integrity,
            size_bytes: std::fs::metadata(destination).map(|m| m.len()).unwrap_or(0),
            sha256: sha256_hex(destination)?,
            tables,
        })
    }

    /// Restore into this Db's recorded path. Prefer `restore_file` from
    /// CLI/operations code so no live SQLite connection is open while
    /// replacing the target file.
    pub fn restore_from(&self, source: &Path, overwrite: bool) -> Result<RestoreReport> {
        Self::restore_file(source, self.path_ref(), overwrite)
    }

    /// Returns the on-disk path of the main database file as a String
    /// for inclusion in error messages and reports.
    fn path_display(&self) -> String {
        self.path_ref().display().to_string()
    }

    fn path_ref(&self) -> &Path {
        // The daemon always opens the main file at a known absolute or
        // project-relative path. The Db inner store keeps it; for now
        // we surface the inner path directly via a small accessor on
        // DbInner; if it is not yet recorded we fall back to a
        // placeholder so reports do not crash.
        self.inner
            .db_path
            .as_deref()
            .map(Path::new)
            .unwrap_or_else(|| Path::new("<unknown>"))
    }

    /// Apply the same schema migration to the SQLx pool so SQLx-only callers
    /// (audit inserts, process writes, batch lifecycle) can rely on the schema
    /// being present even when the rusqlite write connection was bypassed.
    /// Idempotent: every statement uses IF NOT EXISTS / IF EXISTS guards.
    pub async fn migrate_sqlx(&self) -> Result<()> {
        let Some(pool) = self.inner.sqlx_pool.as_ref() else {
            return Ok(());
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        let statements: Vec<&str> = SCHEMA_SQL
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        for statement in statements {
            sqlx::query(statement)
                .execute(pool)
                .await
                .with_context(|| format!("sqlx schema migration step failed: {statement:.80}"))?;
        }
        for migration in COLUMN_MIGRATIONS {
            let sql = format!(
                "ALTER TABLE {} ADD COLUMN {} {}",
                migration.0, migration.1, migration.2
            );
            // Use a swallow-and-continue for ALTER TABLE ADD COLUMN: sqlite
            // raises a duplicate-column error when the column already exists,
            // which is the expected steady state.
            if let Err(err) = sqlx::query(&sql).execute(pool).await {
                let message = err.to_string();
                if !message.contains("duplicate column") && !message.contains("already exists") {
                    return Err(anyhow::Error::from(err)
                        .context(format!("sqlx column migration step failed: {sql}")));
                }
            }
        }
        for statement in INDEX_SQL
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement)
                .execute(pool)
                .await
                .with_context(|| format!("sqlx index migration step failed: {statement:.80}"))?;
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
        ensure_valid_batch_targets_for_insert(
            target_temperature_c,
            target_stirrer_rpm,
            heating_minutes,
            stirring_minutes,
        )?;
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
        ensure_valid_batch_targets_for_insert(
            target_temperature_c,
            target_stirrer_rpm,
            heating_minutes,
            stirring_minutes,
        )?;
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

    pub fn create_process_with_audit(
        &self,
        name: &str,
        description: &str,
        event_type: &str,
        reason: &str,
    ) -> Result<ProcessDefinition> {
        let now = Utc::now();
        let created_at = now.to_rfc3339();
        let mut conn = self.write_conn()?;
        let tx = conn
            .transaction()
            .context("failed to begin process create transaction")?;
        tx.execute(
            r#"
            INSERT INTO processes (name, description, status, version, created_at, updated_at)
            VALUES (?1, ?2, 'draft', 1, ?3, ?3)
            "#,
            params![name, description, created_at],
        )
        .context("failed to create process")?;
        let process_id = tx.last_insert_rowid();
        self.consume_control_event_failure_for_tests()?;
        insert_control_event_in_rusqlite_tx(&tx, None, event_type, None, reason, &created_at)?;
        let process = ProcessDefinition {
            id: process_id,
            name: name.to_string(),
            description: description.to_string(),
            status: "draft".to_string(),
            version: 1,
            step_count: 0,
            created_at: now,
            updated_at: now,
            applied_at: None,
        };
        tx.commit()
            .context("failed to commit process create transaction")?;
        self.run_after_control_event_success_for_tests();
        Ok(process)
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

    pub async fn create_process_with_audit_sqlx(
        &self,
        name: &str,
        description: &str,
        event_type: &str,
        reason: &str,
    ) -> Result<ProcessDefinition> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.create_process_with_audit(name, description, event_type, reason);
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        let _audit_guard = self.inner.audit_write_lock.lock().await;
        let _process_guard = self.inner.process_write_lock.lock().await;
        let now = Utc::now();
        let created_at = now.to_rfc3339();
        let mut tx = pool
            .begin()
            .await
            .context("failed to begin process create transaction with SQLx")?;
        let result = sqlx::query(
            r#"
            INSERT INTO processes (name, description, status, version, created_at, updated_at)
            VALUES (?, ?, 'draft', 1, ?, ?)
            "#,
        )
        .bind(name)
        .bind(description)
        .bind(&created_at)
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .context("failed to create process with SQLx")?;
        self.consume_control_event_failure_for_tests()?;
        insert_control_event_in_sqlx_tx(&mut tx, None, event_type, None, reason, &created_at)
            .await?;
        let process = ProcessDefinition {
            id: result.last_insert_rowid(),
            name: name.to_string(),
            description: description.to_string(),
            status: "draft".to_string(),
            version: 1,
            step_count: 0,
            created_at: now,
            updated_at: now,
            applied_at: None,
        };
        tx.commit()
            .await
            .context("failed to commit process create transaction with SQLx")?;
        self.run_after_control_event_success_for_tests();
        Ok(process)
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

    pub fn update_process_with_audit(
        &self,
        process_id: i64,
        name: &str,
        description: &str,
        status: &str,
        event_type: &str,
        reason: &str,
    ) -> Result<Option<ProcessDefinition>> {
        let now = Utc::now();
        let updated_at = now.to_rfc3339();
        let mut conn = self.write_conn()?;
        let tx = conn
            .transaction()
            .context("failed to begin process update transaction")?;
        let changed = tx
            .execute(
                r#"
                UPDATE processes
                SET name = ?1, description = ?2, status = ?3, version = version + 1, updated_at = ?4
                WHERE id = ?5
                "#,
                params![name, description, status, updated_at, process_id],
            )
            .context("failed to update process")?;
        if changed == 0 {
            tx.commit()
                .context("failed to commit empty process update transaction")?;
            return Ok(None);
        }
        self.consume_control_event_failure_for_tests()?;
        insert_control_event_in_rusqlite_tx(&tx, None, event_type, None, reason, &updated_at)?;
        let process = process_summary_by_id(&tx, process_id).map_err(anyhow::Error::from)?;
        tx.commit()
            .context("failed to commit process update transaction")?;
        self.run_after_control_event_success_for_tests();
        Ok(process)
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

    pub async fn update_process_with_audit_sqlx(
        &self,
        process_id: i64,
        name: &str,
        description: &str,
        status: &str,
        event_type: &str,
        reason: &str,
    ) -> Result<Option<ProcessDefinition>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.update_process_with_audit(
                process_id,
                name,
                description,
                status,
                event_type,
                reason,
            );
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        let _audit_guard = self.inner.audit_write_lock.lock().await;
        let _process_guard = self.inner.process_write_lock.lock().await;
        let now = Utc::now();
        let updated_at = now.to_rfc3339();
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
        .bind(&updated_at)
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
        self.consume_control_event_failure_for_tests()?;
        insert_control_event_in_sqlx_tx(&mut tx, None, event_type, None, reason, &updated_at)
            .await?;
        let process = self
            .process_summary_by_id_sqlx_tx(&mut tx, process_id)
            .await?;
        tx.commit()
            .await
            .context("failed to commit process update transaction with SQLx")?;
        self.run_after_control_event_success_for_tests();
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
        ensure_valid_process_step_for_insert(step)?;
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

    pub fn add_process_step_with_audit(
        &self,
        process_id: i64,
        step: &NewProcessStep,
        event_type: &str,
        reason: &str,
    ) -> Result<Option<ProcessStep>> {
        let mut conn = self.write_conn()?;
        let tx = conn
            .transaction()
            .context("failed to begin process step insert transaction")?;
        if process_summary_by_id(&tx, process_id)?.is_none() {
            tx.commit()
                .context("failed to commit empty process step insert transaction")?;
            return Ok(None);
        }
        ensure_valid_process_step_for_insert(step)?;
        let next_index: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(step_index), 0) + 1 FROM process_steps WHERE process_id = ?1",
                [process_id],
                |row| row.get(0),
            )
            .context("failed to allocate process step index")?;
        let now = Utc::now();
        let created_at = now.to_rfc3339();
        tx.execute(
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
                created_at
            ],
        )
        .context("failed to insert process step")?;
        let step_id = tx.last_insert_rowid();
        touch_process(&tx, process_id).context("failed to touch process")?;
        self.consume_control_event_failure_for_tests()?;
        insert_control_event_in_rusqlite_tx(&tx, None, event_type, None, reason, &created_at)?;
        let step = process_step_by_id(&tx, step_id).map_err(anyhow::Error::from)?;
        tx.commit()
            .context("failed to commit process step insert transaction")?;
        self.run_after_control_event_success_for_tests();
        Ok(step)
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
        ensure_valid_process_step_for_insert(step)?;
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

    pub async fn add_process_step_with_audit_sqlx(
        &self,
        process_id: i64,
        step: &NewProcessStep,
        event_type: &str,
        reason: &str,
    ) -> Result<Option<ProcessStep>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.add_process_step_with_audit(process_id, step, event_type, reason);
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        let _audit_guard = self.inner.audit_write_lock.lock().await;
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
        ensure_valid_process_step_for_insert(step)?;
        let next_index: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(step_index), 0) + 1 FROM process_steps WHERE process_id = ?",
        )
        .bind(process_id)
        .fetch_one(&mut *tx)
        .await
        .context("failed to allocate process step index with SQLx")?;
        let now = Utc::now();
        let created_at = now.to_rfc3339();
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
        .bind(&created_at)
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .context("failed to insert process step with SQLx")?;
        touch_process_sqlx(&mut tx, process_id).await?;
        self.consume_control_event_failure_for_tests()?;
        insert_control_event_in_sqlx_tx(&mut tx, None, event_type, None, reason, &created_at)
            .await?;
        let step = process_step_by_id_sqlx_tx(&mut tx, result.last_insert_rowid()).await?;
        tx.commit()
            .await
            .context("failed to commit process step insert transaction with SQLx")?;
        self.run_after_control_event_success_for_tests();
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
        let Some(existing_step) = process_step_by_id(&conn, step_id)? else {
            return Ok(None);
        };
        if existing_step.process_id != process_id {
            return Ok(None);
        }
        ensure_valid_process_step_for_insert(step)?;
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

    pub fn update_process_step_with_audit(
        &self,
        process_id: i64,
        step_id: i64,
        step: &NewProcessStep,
        event_type: &str,
        reason: &str,
    ) -> Result<Option<ProcessStep>> {
        let now = Utc::now();
        let updated_at = now.to_rfc3339();
        let mut conn = self.write_conn()?;
        let tx = conn
            .transaction()
            .context("failed to begin process step update transaction")?;
        let Some(existing_step) = process_step_by_id(&tx, step_id)? else {
            tx.commit()
                .context("failed to commit empty process step update transaction")?;
            return Ok(None);
        };
        if existing_step.process_id != process_id {
            tx.commit()
                .context("failed to commit empty process step update transaction")?;
            return Ok(None);
        }
        ensure_valid_process_step_for_insert(step)?;
        let changed = tx
            .execute(
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
                    updated_at,
                    step_id,
                    process_id
                ],
            )
            .context("failed to update process step")?;
        if changed == 0 {
            tx.commit()
                .context("failed to commit empty process step update transaction")?;
            return Ok(None);
        }
        touch_process(&tx, process_id).context("failed to touch process")?;
        self.consume_control_event_failure_for_tests()?;
        insert_control_event_in_rusqlite_tx(&tx, None, event_type, None, reason, &updated_at)?;
        let step = process_step_by_id(&tx, step_id).map_err(anyhow::Error::from)?;
        tx.commit()
            .context("failed to commit process step update transaction")?;
        self.run_after_control_event_success_for_tests();
        Ok(step)
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
        let Some(existing_step) = process_step_by_id_sqlx_tx(&mut tx, step_id).await? else {
            tx.commit()
                .await
                .context("failed to commit empty process step update transaction with SQLx")?;
            return Ok(None);
        };
        if existing_step.process_id != process_id {
            tx.commit()
                .await
                .context("failed to commit empty process step update transaction with SQLx")?;
            return Ok(None);
        }
        ensure_valid_process_step_for_insert(step)?;
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

    pub async fn update_process_step_with_audit_sqlx(
        &self,
        process_id: i64,
        step_id: i64,
        step: &NewProcessStep,
        event_type: &str,
        reason: &str,
    ) -> Result<Option<ProcessStep>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self
                .update_process_step_with_audit(process_id, step_id, step, event_type, reason);
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        let _audit_guard = self.inner.audit_write_lock.lock().await;
        let _process_guard = self.inner.process_write_lock.lock().await;
        let now = Utc::now();
        let updated_at = now.to_rfc3339();
        let mut tx = pool
            .begin()
            .await
            .context("failed to begin process step update transaction with SQLx")?;
        let Some(existing_step) = process_step_by_id_sqlx_tx(&mut tx, step_id).await? else {
            tx.commit()
                .await
                .context("failed to commit empty process step update transaction with SQLx")?;
            return Ok(None);
        };
        if existing_step.process_id != process_id {
            tx.commit()
                .await
                .context("failed to commit empty process step update transaction with SQLx")?;
            return Ok(None);
        }
        ensure_valid_process_step_for_insert(step)?;
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
        .bind(&updated_at)
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
        self.consume_control_event_failure_for_tests()?;
        insert_control_event_in_sqlx_tx(&mut tx, None, event_type, None, reason, &updated_at)
            .await?;
        let step = process_step_by_id_sqlx_tx(&mut tx, step_id).await?;
        tx.commit()
            .await
            .context("failed to commit process step update transaction with SQLx")?;
        self.run_after_control_event_success_for_tests();
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
        ensure_valid_sensor_sample_for_insert(sample)?;
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
        ensure_valid_sensor_sample_for_insert(sample)?;
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.insert_sample(batch_id, sample);
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
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
        let rows = stmt.query_map([limit as i64], |row| sensor_snapshot_from_row(row, 1, 10))?;

        let mut samples = Vec::new();
        for row in rows {
            match row {
                Ok(sample) => samples.push(sample),
                Err(err) => {
                    if let Some(reason) = invalid_sensor_sample_reason_from_rusqlite(&err) {
                        warn_invalid_sensor_sample_row("recent_samples", reason);
                        continue;
                    }
                    return Err(err.into());
                }
            }
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
            sensor_sample_record_from_row(row, 1, 2, 11)
        })?;

        let mut samples = Vec::new();
        for row in rows {
            match row {
                Ok(sample) => samples.push(sample),
                Err(err) => {
                    if let Some(reason) = invalid_sensor_sample_reason_from_rusqlite(&err) {
                        warn_invalid_sensor_sample_row("recent_sample_records", reason);
                        continue;
                    }
                    return Err(err.into());
                }
            }
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
        collect_valid_sensor_sample_records_from_sqlx_rows(rows, "recent_sample_records_sqlx")
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
            |row| sensor_sample_record_from_row(row, 0, 1, 10),
        )?;

        let mut samples = Vec::new();
        for row in rows {
            match row {
                Ok(sample) => samples.push(sample),
                Err(err) => {
                    if let Some(reason) = invalid_sensor_sample_reason_from_rusqlite(&err) {
                        warn_invalid_sensor_sample_row("samples_between", reason);
                        continue;
                    }
                    return Err(err.into());
                }
            }
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
        collect_valid_sensor_sample_records_from_sqlx_rows(rows, "samples_between_sqlx")
    }

    pub fn insert_control_event(
        &self,
        batch_id: Option<i64>,
        event_type: &str,
        command: Option<&SafeCommand>,
        reason: &str,
    ) -> Result<()> {
        self.consume_control_event_failure_for_tests()?;
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
        ensure_valid_control_event_targets_for_insert(
            target_temperature_c,
            target_stirrer_rpm,
            target_shake_speed_cpm,
        )?;
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
        self.run_after_control_event_success_for_tests();
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
        self.consume_control_event_failure_for_tests()?;
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
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
        ensure_valid_control_event_targets_for_insert(
            target_temperature_c,
            target_stirrer_rpm,
            target_shake_speed_cpm,
        )?;
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
        self.run_after_control_event_success_for_tests();
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

    pub async fn recent_demo_alarms_sqlx(&self, limit: usize) -> Result<Vec<DemoAlarm>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.recent_demo_alarms(limit);
        };
        let rows = sqlx::query(
            r#"
            SELECT id, alarm_type, sensor, level, message, current_value, limit_value,
                   suggestion, active, created_at
            FROM (
                SELECT id, alarm_type, sensor, level, message, current_value, limit_value,
                       suggestion, active, created_at
                FROM demo_alarms
                ORDER BY id DESC
                LIMIT ?
            )
            ORDER BY id ASC
            "#,
        )
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .context("failed to list demo alarms with SQLx")?;
        rows.into_iter().map(demo_alarm_from_sqlx_row).collect()
    }

    pub fn create_integration_task(
        &self,
        source: &str,
        external_task_id: Option<&str>,
        action: &str,
        request: &Value,
    ) -> Result<IntegrationTask> {
        ensure_valid_integration_task_create_for_insert(source, external_task_id, action, request)?;
        let conn = self.write_conn()?;
        if let Some(external_task_id) = external_task_id {
            if let Some(task) =
                self.integration_task_by_external_id_conn(&conn, source, external_task_id)?
            {
                return Ok(task);
            }
        }
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
        ensure_valid_integration_task_create_for_insert(source, external_task_id, action, request)?;
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.create_integration_task(source, external_task_id, action, request);
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        if let Some(external_task_id) = external_task_id {
            if let Some(task) = self
                .integration_task_by_external_id_sqlx(source, external_task_id)
                .await?
            {
                return Ok(task);
            }
        }
        let now = Utc::now().to_rfc3339();
        let request_json = self.serialize_sensitive_json(request)?;
        let response_json = self.serialize_sensitive_json(&Value::Null)?;
        let insert_result = sqlx::query(
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
        .await;
        let result = match insert_result {
            Ok(result) => result,
            Err(err) if is_sqlite_unique_constraint_error(&err) => {
                if let Some(external_task_id) = external_task_id {
                    if let Some(task) = self
                        .integration_task_by_external_id_sqlx(source, external_task_id)
                        .await?
                    {
                        return Ok(task);
                    }
                }
                return Err(anyhow::Error::from(err)
                    .context("integration task insert hit unique constraint but existing task was not readable"));
            }
            Err(err) => {
                return Err(
                    anyhow::Error::from(err).context("failed to create integration task with SQLx")
                );
            }
        };
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
        ensure_valid_integration_task_update_for_insert(status, response)?;
        let conn = self.write_conn()?;
        if self.integration_task_by_id_conn(&conn, id)?.is_none() {
            return Ok(None);
        }
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
        ensure_valid_integration_task_update_for_insert(status, response)?;
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.update_integration_task(id, status, response);
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        if self.integration_task_sqlx(id).await?.is_none() {
            return Ok(None);
        }
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

    pub fn mark_integration_task_executing(&self, id: i64) -> Result<Option<IntegrationTask>> {
        let conn = self.write_conn()?;
        let Some(existing) = self.integration_task_by_id_conn(&conn, id)? else {
            return Ok(None);
        };
        if existing.status != "received" {
            return Ok(Some(existing));
        }
        let response_json = self.serialize_sensitive_json(&json!({
            "status": "executing",
            "message": "integration task action started; awaiting final receipt"
        }))?;
        conn.execute(
            r#"
            UPDATE integration_tasks
            SET status = 'executing', response_json = ?1, updated_at = ?2
            WHERE id = ?3 AND status = 'received'
            "#,
            params![response_json, Utc::now().to_rfc3339(), id],
        )?;
        Ok(self.integration_task_by_id_conn(&conn, id)?)
    }

    pub async fn mark_integration_task_executing_sqlx(
        &self,
        id: i64,
    ) -> Result<Option<IntegrationTask>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.mark_integration_task_executing(id);
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        let Some(existing) = self.integration_task_sqlx(id).await? else {
            return Ok(None);
        };
        if existing.status != "received" {
            return Ok(Some(existing));
        }
        let response_json = self.serialize_sensitive_json(&json!({
            "status": "executing",
            "message": "integration task action started; awaiting final receipt"
        }))?;
        sqlx::query(
            r#"
            UPDATE integration_tasks
            SET status = 'executing', response_json = ?, updated_at = ?
            WHERE id = ? AND status = 'received'
            "#,
        )
        .bind(response_json)
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await
        .context("failed to mark integration task executing with SQLx")?;
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
                match row {
                    Ok(task) => tasks.push(task),
                    Err(err) => {
                        if let Some(reason) = invalid_integration_task_reason_from_rusqlite(&err) {
                            warn_invalid_integration_task_row("integration_tasks", reason);
                            continue;
                        }
                        return Err(err.into());
                    }
                }
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
                match row {
                    Ok(task) => tasks.push(task),
                    Err(err) => {
                        if let Some(reason) = invalid_integration_task_reason_from_rusqlite(&err) {
                            warn_invalid_integration_task_row("integration_tasks", reason);
                            continue;
                        }
                        return Err(err.into());
                    }
                }
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
        self.collect_valid_integration_tasks_from_sqlx_rows(rows, "integration_tasks_sqlx")
    }

    pub fn insert_product_result(&self, result: &ProductResult) -> Result<()> {
        ensure_valid_product_result_for_insert(result)?;
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

    pub fn insert_product_result_with_audit(
        &self,
        result: &ProductResult,
        event_type: &str,
        reason: &str,
    ) -> Result<()> {
        ensure_valid_product_result_for_insert(result)?;
        self.consume_control_event_failure_for_tests()?;
        let mut conn = self.write_conn()?;
        let tx = conn
            .transaction()
            .context("failed to begin product result transaction")?;
        let created_at = Utc::now().to_rfc3339();
        tx.execute(
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
                created_at
            ],
        )
        .context("failed to insert product result")?;
        insert_control_event_in_rusqlite_tx(
            &tx,
            Some(result.batch_id),
            event_type,
            None,
            reason,
            &created_at,
        )?;
        tx.commit()
            .context("failed to commit product result transaction")?;
        self.run_after_control_event_success_for_tests();
        Ok(())
    }

    pub async fn insert_product_result_sqlx(&self, result: &ProductResult) -> Result<()> {
        ensure_valid_product_result_for_insert(result)?;
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

    pub async fn insert_product_result_with_audit_sqlx(
        &self,
        result: &ProductResult,
        event_type: &str,
        reason: &str,
    ) -> Result<()> {
        ensure_valid_product_result_for_insert(result)?;
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.insert_product_result_with_audit(result, event_type, reason);
        };
        self.consume_control_event_failure_for_tests()?;
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        let _audit_guard = self.inner.audit_write_lock.lock().await;
        let created_at = Utc::now().to_rfc3339();
        let mut tx = pool
            .begin()
            .await
            .context("failed to begin product result transaction with SQLx")?;
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
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .context("failed to insert product result with SQLx")?;
        insert_control_event_in_sqlx_tx(
            &mut tx,
            Some(result.batch_id),
            event_type,
            None,
            reason,
            &created_at,
        )
        .await?;
        tx.commit()
            .await
            .context("failed to commit product result transaction with SQLx")?;
        self.run_after_control_event_success_for_tests();
        Ok(())
    }

    pub fn insert_recommendation(&self, recommendation: &Recommendation) -> Result<()> {
        ensure_valid_recommendation_for_insert(recommendation)?;
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
        ensure_valid_recommendation_for_insert(recommendation)?;
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

    pub fn insert_recommendation_with_audit(
        &self,
        recommendation: &Recommendation,
        event_type: &str,
        reason: &str,
    ) -> Result<()> {
        ensure_valid_recommendation_for_insert(recommendation)?;
        let mut conn = self.write_conn()?;
        let tx = conn
            .transaction()
            .context("failed to begin AI recommendation transaction")?;
        let created_at = Utc::now().to_rfc3339();
        tx.execute(
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
                created_at
            ],
        )
        .context("failed to insert AI recommendation")?;
        self.consume_control_event_failure_for_tests()?;
        let command = safe_command_from_recommendation(recommendation, reason);
        insert_control_event_in_rusqlite_tx(
            &tx,
            None,
            event_type,
            Some(&command),
            reason,
            &created_at,
        )?;
        tx.commit()
            .context("failed to commit AI recommendation transaction")?;
        self.run_after_control_event_success_for_tests();
        Ok(())
    }

    pub async fn insert_recommendation_with_audit_sqlx(
        &self,
        recommendation: &Recommendation,
        event_type: &str,
        reason: &str,
    ) -> Result<()> {
        ensure_valid_recommendation_for_insert(recommendation)?;
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.insert_recommendation_with_audit(recommendation, event_type, reason);
        };
        let _write_guard = self.inner.sqlx_write_lock.lock().await;
        let _audit_guard = self.inner.audit_write_lock.lock().await;
        let created_at = Utc::now().to_rfc3339();
        let mut tx = pool
            .begin()
            .await
            .context("failed to begin AI recommendation transaction with SQLx")?;
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
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .context("failed to insert AI recommendation with SQLx")?;
        self.consume_control_event_failure_for_tests()?;
        let command = safe_command_from_recommendation(recommendation, reason);
        insert_control_event_in_sqlx_tx(
            &mut tx,
            None,
            event_type,
            Some(&command),
            reason,
            &created_at,
        )
        .await?;
        tx.commit()
            .await
            .context("failed to commit AI recommendation transaction with SQLx")?;
        self.run_after_control_event_success_for_tests();
        Ok(())
    }

    pub fn latest_recommendation(&self) -> Result<Option<Recommendation>> {
        let conn = self.read_conn()?;
        match conn
            .query_row(
                r#"
            SELECT based_on_batch_count, target_temperature_c, target_stirrer_rpm,
                   heating_minutes, stirring_minutes, expected_score, rationale
            FROM ai_recommendations
            ORDER BY id DESC
            LIMIT 1
            "#,
                [],
                recommendation_from_row,
            )
            .optional()
        {
            Ok(recommendation) => Ok(recommendation),
            Err(err) => {
                if let Some(reason) = invalid_recommendation_reason_from_rusqlite(&err) {
                    warn_invalid_recommendation_row("latest_recommendation", reason);
                    return Ok(None);
                }
                Err(err.into())
            }
        }
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
        match row.map(recommendation_from_sqlx_row).transpose() {
            Ok(recommendation) => Ok(recommendation),
            Err(err) => {
                if let Some(reason) = invalid_recommendation_reason_from_anyhow(&err) {
                    warn_invalid_recommendation_row("latest_recommendation_sqlx", reason);
                    return Ok(None);
                }
                Err(err)
            }
        }
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
            WHERE b.finished_at IS NOT NULL
            ORDER BY b.id ASC
            "#,
        )?;
        let rows = stmt.query_map([], batch_outcome_from_row)?;
        collect_valid_batch_outcomes_from_rusqlite_rows(rows, "batch_outcomes")
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
            WHERE b.finished_at IS NOT NULL
            ORDER BY b.id ASC
            "#,
        )
        .fetch_all(pool)
        .await
        .context("failed to list batch outcomes with SQLx")?;
        collect_valid_batch_outcomes_from_sqlx_rows(rows, "batch_outcomes_sqlx")
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
                    WHERE b.finished_at IS NOT NULL
                    ORDER BY b.id DESC
                    LIMIT ?1
                )
            ) b
            JOIN product_results p ON p.batch_id = b.id
            ORDER BY b.id ASC
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], batch_outcome_from_row)?;
        collect_valid_batch_outcomes_from_rusqlite_rows(rows, "recent_batch_outcomes")
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
                    WHERE b.finished_at IS NOT NULL
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
        collect_valid_batch_outcomes_from_sqlx_rows(rows, "recent_batch_outcomes_sqlx")
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
        let rows = stmt.query_map([limit as i64], batch_from_row)?;
        collect_valid_batches_from_rusqlite_rows(rows, "recent_batches")
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
        collect_valid_batches_from_sqlx_rows(rows, "recent_batches_sqlx")
    }

    pub fn latest_unfinished_batch(&self) -> Result<Option<Batch>> {
        let conn = self.read_conn()?;
        conn.query_row(
            r#"
            SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM batches
            WHERE finished_at IS NULL
            ORDER BY id DESC
            LIMIT 1
            "#,
            [],
            batch_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub async fn latest_unfinished_batch_sqlx(&self) -> Result<Option<Batch>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.latest_unfinished_batch();
        };
        let row = sqlx::query(
            r#"
            SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM batches
            WHERE finished_at IS NULL
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await
        .context("failed to read latest unfinished batch with SQLx")?;
        row.map(batch_from_sqlx_row).transpose()
    }

    pub fn unfinished_batches(&self, limit: usize) -> Result<Vec<Batch>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM batches
            WHERE finished_at IS NULL
            ORDER BY id DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], batch_from_row)?;

        let mut batches = Vec::new();
        for row in rows {
            batches.push(row?);
        }
        Ok(batches)
    }

    pub async fn unfinished_batches_sqlx(&self, limit: usize) -> Result<Vec<Batch>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.unfinished_batches(limit);
        };
        let rows = sqlx::query(
            r#"
            SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM batches
            WHERE finished_at IS NULL
            ORDER BY id DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .context("failed to list unfinished batches with SQLx")?;
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
            batch_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub async fn batch_by_id_sqlx(&self, batch_id: i64) -> Result<Option<Batch>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.batch_by_id(batch_id);
        };
        let row = sqlx::query(
            r#"
            SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM batches
            WHERE id = ?
            "#,
        )
        .bind(batch_id)
        .fetch_optional(pool)
        .await
        .context("failed to read batch by id with SQLx")?;
        row.map(batch_from_sqlx_row).transpose()
    }

    pub fn batch_outcome_by_id(&self, batch_id: i64) -> Result<Option<BatchOutcome>> {
        let conn = self.read_conn()?;
        match conn
            .query_row(
                r#"
            SELECT b.id, b.target_temperature_c, b.target_stirrer_rpm,
                   b.heating_minutes, b.stirring_minutes,
                   p.yield_percent, p.product_ratio
            FROM batches b
            JOIN product_results p ON p.batch_id = b.id
            WHERE b.id = ?1
              AND b.finished_at IS NOT NULL
            "#,
                [batch_id],
                batch_outcome_from_row,
            )
            .optional()
        {
            Ok(outcome) => Ok(outcome),
            Err(err) => {
                if let Some(reason) = invalid_batch_outcome_reason_from_rusqlite(&err) {
                    warn_invalid_batch_outcome_row("batch_outcome_by_id", reason);
                    return Ok(None);
                }
                Err(err.into())
            }
        }
    }

    pub async fn batch_outcome_by_id_sqlx(&self, batch_id: i64) -> Result<Option<BatchOutcome>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.batch_outcome_by_id(batch_id);
        };
        let row = sqlx::query(
            r#"
            SELECT b.id, b.target_temperature_c, b.target_stirrer_rpm,
                   b.heating_minutes, b.stirring_minutes,
                   p.yield_percent, p.product_ratio
            FROM batches b
            JOIN product_results p ON p.batch_id = b.id
            WHERE b.id = ?
              AND b.finished_at IS NOT NULL
            "#,
        )
        .bind(batch_id)
        .fetch_optional(pool)
        .await
        .context("failed to read batch outcome by id with SQLx")?;
        match row.map(batch_outcome_from_sqlx_row).transpose() {
            Ok(outcome) => Ok(outcome),
            Err(err) => {
                if let Some(reason) = invalid_batch_outcome_reason_from_anyhow(&err) {
                    warn_invalid_batch_outcome_row("batch_outcome_by_id_sqlx", reason);
                    return Ok(None);
                }
                Err(err)
            }
        }
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
            sensor_sample_record_from_row(row, 1, 2, 11)
        })?;

        let mut samples = Vec::new();
        for row in rows {
            match row {
                Ok(sample) => samples.push(sample),
                Err(err) => {
                    if let Some(reason) = invalid_sensor_sample_reason_from_rusqlite(&err) {
                        warn_invalid_sensor_sample_row("sample_records_for_batch", reason);
                        continue;
                    }
                    return Err(err.into());
                }
            }
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
        collect_valid_sensor_sample_records_from_sqlx_rows(rows, "sample_records_for_batch_sqlx")
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
        collect_valid_control_events_from_rusqlite_rows(rows, "recent_control_events")
    }

    pub async fn recent_control_events_sqlx(&self, limit: usize) -> Result<Vec<ControlEvent>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.recent_control_events(limit);
        };
        let rows = sqlx::query(
            r#"
            SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                   target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
            FROM (
                SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                       target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                FROM control_events
                ORDER BY id DESC
                LIMIT ?
            )
            ORDER BY id ASC
            "#,
        )
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .context("failed to list recent control events with SQLx")?;
        collect_valid_control_events_from_sqlx_rows(rows, "recent_control_events_sqlx")
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
        collect_valid_control_events_from_rusqlite_rows(rows, "audit_events")
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

        collect_valid_control_events_from_sqlx_rows(rows, "audit_events_sqlx")
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
        collect_valid_control_events_from_rusqlite_rows(rows, "control_events_for_batch")
    }

    pub async fn control_events_for_batch_sqlx(
        &self,
        batch_id: i64,
        limit: usize,
    ) -> Result<Vec<ControlEvent>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            return self.control_events_for_batch(batch_id, limit);
        };
        let rows = sqlx::query(
            r#"
            SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                   target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
            FROM (
                SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm,
                       target_shake_speed_cpm, reason, created_at, previous_hash, event_hash
                FROM control_events
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
        .context("failed to list batch control events with SQLx")?;
        collect_valid_control_events_from_sqlx_rows(rows, "control_events_for_batch_sqlx")
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
    pub fn product_result_notes_for_tests(&self, batch_id: i64) -> Result<Option<String>> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT notes FROM product_results WHERE batch_id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    #[cfg(debug_assertions)]
    pub fn corrupt_process_step_for_tests(
        &self,
        step_id: i64,
        duration_minutes: Option<f64>,
        target_stirrer_rpm: Option<f64>,
        target_shake_speed_cpm: Option<f64>,
    ) -> Result<()> {
        let conn = self.write_conn()?;
        if let Some(value) = duration_minutes {
            conn.execute(
                "UPDATE process_steps SET duration_minutes = ?1 WHERE id = ?2",
                params![value, step_id],
            )?;
        }
        if let Some(value) = target_stirrer_rpm {
            conn.execute(
                "UPDATE process_steps SET target_stirrer_rpm = ?1 WHERE id = ?2",
                params![value, step_id],
            )?;
        }
        if let Some(value) = target_shake_speed_cpm {
            conn.execute(
                "UPDATE process_steps SET target_shake_speed_cpm = ?1 WHERE id = ?2",
                params![value, step_id],
            )?;
        }
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn break_control_events_for_tests(&self) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute("DROP TABLE control_events", [])?;
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn repair_control_events_for_tests(&self) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn break_integration_tasks_for_tests(&self) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute("DROP TABLE integration_tasks", [])?;
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn break_samples_for_tests(&self) -> Result<()> {
        let conn = self.write_conn()?;
        conn.execute("DROP TABLE sensor_samples", [])?;
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn fail_control_events_after_successes_for_tests(&self, successes: usize) {
        self.inner
            .fail_control_events_after
            .store(successes, Ordering::SeqCst);
    }

    #[cfg(debug_assertions)]
    pub fn after_control_event_success_for_tests(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        *self
            .inner
            .after_control_event_success
            .lock()
            .expect("after control event hook lock poisoned") = Some(callback);
    }

    #[cfg(debug_assertions)]
    fn run_after_control_event_success_for_tests(&self) {
        if let Some(callback) = self
            .inner
            .after_control_event_success
            .lock()
            .expect("after control event hook lock poisoned")
            .take()
        {
            callback();
        }
    }

    #[cfg(debug_assertions)]
    fn consume_control_event_failure_for_tests(&self) -> Result<()> {
        let remaining = self.inner.fail_control_events_after.load(Ordering::SeqCst);
        if remaining == usize::MAX {
            return Ok(());
        }
        if remaining == 0 {
            self.inner
                .fail_control_events_after
                .store(usize::MAX, Ordering::SeqCst);
            return Err(anyhow!("injected control event write failure for tests"));
        }
        self.inner
            .fail_control_events_after
            .fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }

    #[cfg(not(debug_assertions))]
    fn run_after_control_event_success_for_tests(&self) {}

    #[cfg(not(debug_assertions))]
    fn consume_control_event_failure_for_tests(&self) -> Result<()> {
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

    fn integration_task_by_external_id_conn(
        &self,
        conn: &Connection,
        source: &str,
        external_task_id: &str,
    ) -> rusqlite::Result<Option<IntegrationTask>> {
        conn.query_row(
            r#"
            SELECT id, external_task_id, source, action, status, request_json, response_json,
                   created_at, updated_at
            FROM integration_tasks
            WHERE source = ?1 AND external_task_id = ?2
            ORDER BY id ASC
            LIMIT 1
            "#,
            params![source, external_task_id],
            |row| self.integration_task_from_row(row),
        )
        .optional()
    }

    async fn integration_task_by_external_id_sqlx(
        &self,
        source: &str,
        external_task_id: &str,
    ) -> Result<Option<IntegrationTask>> {
        let Some(pool) = &self.inner.sqlx_pool else {
            let conn = self.read_conn()?;
            return self
                .integration_task_by_external_id_conn(&conn, source, external_task_id)
                .map_err(Into::into);
        };
        let row = sqlx::query(
            r#"
            SELECT id, external_task_id, source, action, status, request_json, response_json,
                   created_at, updated_at
            FROM integration_tasks
            WHERE source = ? AND external_task_id = ?
            ORDER BY id ASC
            LIMIT 1
            "#,
        )
        .bind(source)
        .bind(external_task_id)
        .fetch_optional(pool)
        .await
        .context("failed to load integration task by external id with SQLx")?;
        row.map(|row| self.integration_task_from_sqlx_row(row))
            .transpose()
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
        .and_then(|task| {
            validate_integration_task_record(task)
                .map_err(invalid_integration_task_conversion_error)
        })
    }

    fn integration_task_from_sqlx_row(&self, row: SqliteRow) -> Result<IntegrationTask> {
        let request_json: String = row.try_get("request_json")?;
        let response_json: String = row.try_get("response_json")?;
        let created_at: String = row.try_get("created_at")?;
        let updated_at: String = row.try_get("updated_at")?;
        let task = IntegrationTask {
            id: row.try_get("id")?,
            external_task_id: row.try_get("external_task_id")?,
            source: row.try_get("source")?,
            action: row.try_get("action")?,
            status: row.try_get("status")?,
            request: self.parse_sensitive_json_anyhow(&request_json)?,
            response: self.parse_sensitive_json_anyhow(&response_json)?,
            created_at: parse_dt_anyhow(&created_at)?,
            updated_at: parse_dt_anyhow(&updated_at)?,
        };
        validate_integration_task_record(task)
            .map_err(|reason| anyhow!(InvalidIntegrationTaskRow { reason }))
    }

    fn collect_valid_integration_tasks_from_sqlx_rows(
        &self,
        rows: Vec<SqliteRow>,
        source: &str,
    ) -> Result<Vec<IntegrationTask>> {
        let mut tasks = Vec::new();
        for row in rows {
            match self.integration_task_from_sqlx_row(row) {
                Ok(task) => tasks.push(task),
                Err(err) if invalid_integration_task_reason_from_anyhow(&err).is_some() => {
                    let reason = invalid_integration_task_reason_from_anyhow(&err).unwrap();
                    warn_invalid_integration_task_row(source, reason);
                }
                Err(err) => return Err(err),
            }
        }
        Ok(tasks)
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

fn is_sqlite_unique_constraint_error(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db_err| db_err.code())
        .is_some_and(|code| code == "2067" || code == "1555")
        || err.to_string().contains("UNIQUE constraint failed")
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
    let event = ControlEvent {
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
    };
    validate_control_event_record(event)
        .map_err(|reason| invalid_control_event_conversion_error(reason))
}

fn control_event_from_sqlx_row(row: SqliteRow) -> Result<ControlEvent> {
    let created_at: String = row.try_get("created_at")?;
    let event = ControlEvent {
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
    };
    validate_control_event_record(event)
        .map_err(|reason| anyhow!(InvalidControlEventRow { reason }))
}

fn demo_alarm_from_sqlx_row(row: SqliteRow) -> Result<DemoAlarm> {
    let created_at: String = row.try_get("created_at")?;
    let active: i64 = row.try_get("active")?;
    Ok(DemoAlarm {
        id: row.try_get("id")?,
        alarm_type: row.try_get("alarm_type")?,
        sensor: row.try_get("sensor")?,
        level: row.try_get("level")?,
        message: row.try_get("message")?,
        current_value: row.try_get("current_value")?,
        limit_value: row.try_get("limit_value")?,
        suggestion: row.try_get("suggestion")?,
        active: active != 0,
        created_at: parse_dt_anyhow(&created_at)?,
    })
}

fn batch_from_sqlx_row(row: SqliteRow) -> Result<Batch> {
    let started_at: String = row.try_get("started_at")?;
    let finished_at: Option<String> = row.try_get("finished_at")?;
    let batch = Batch {
        id: row.try_get("id")?,
        process_id: row.try_get("process_id")?,
        name: row.try_get("name")?,
        started_at: parse_dt_anyhow(&started_at)?,
        finished_at: finished_at.as_deref().map(parse_dt_anyhow).transpose()?,
        target_temperature_c: row.try_get("target_temperature_c")?,
        target_stirrer_rpm: row.try_get("target_stirrer_rpm")?,
        heating_minutes: row.try_get("heating_minutes")?,
        stirring_minutes: row.try_get("stirring_minutes")?,
    };
    validate_batch_record(batch).map_err(|reason| anyhow!(InvalidBatchRow { reason }))
}

fn batch_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Batch> {
    let started_at: String = row.get(3)?;
    let finished_at: Option<String> = row.get(4)?;
    let batch = Batch {
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
    };
    validate_batch_record(batch).map_err(|reason| invalid_batch_conversion_error(reason))
}

fn batch_outcome_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BatchOutcome> {
    let outcome = BatchOutcome {
        batch_id: row.get(0)?,
        target_temperature_c: row.get(1)?,
        target_stirrer_rpm: row.get(2)?,
        heating_minutes: row.get(3)?,
        stirring_minutes: row.get(4)?,
        yield_percent: row.get(5)?,
        product_ratio: row.get(6)?,
    };
    validate_batch_outcome_record(outcome)
        .map_err(|reason| invalid_batch_outcome_conversion_error(reason))
}

fn batch_outcome_from_sqlx_row(row: SqliteRow) -> Result<BatchOutcome> {
    let outcome = BatchOutcome {
        batch_id: row.try_get("id")?,
        target_temperature_c: row.try_get("target_temperature_c")?,
        target_stirrer_rpm: row.try_get("target_stirrer_rpm")?,
        heating_minutes: row.try_get("heating_minutes")?,
        stirring_minutes: row.try_get("stirring_minutes")?,
        yield_percent: row.try_get("yield_percent")?,
        product_ratio: row.try_get("product_ratio")?,
    };
    validate_batch_outcome_record(outcome)
        .map_err(|reason| anyhow!(InvalidBatchOutcomeRow { reason }))
}

fn recommendation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Recommendation> {
    let recommendation = Recommendation {
        based_on_batch_count: row.get(0)?,
        target_temperature_c: row.get(1)?,
        target_stirrer_rpm: row.get(2)?,
        heating_minutes: row.get(3)?,
        stirring_minutes: row.get(4)?,
        expected_score: row.get(5)?,
        rationale: row.get(6)?,
    };
    validate_recommendation_record(recommendation)
        .map_err(|reason| invalid_recommendation_conversion_error(reason))
}

fn recommendation_from_sqlx_row(row: SqliteRow) -> Result<Recommendation> {
    let recommendation = Recommendation {
        based_on_batch_count: row.try_get("based_on_batch_count")?,
        target_temperature_c: row.try_get("target_temperature_c")?,
        target_stirrer_rpm: row.try_get("target_stirrer_rpm")?,
        heating_minutes: row.try_get("heating_minutes")?,
        stirring_minutes: row.try_get("stirring_minutes")?,
        expected_score: row.try_get("expected_score")?,
        rationale: row.try_get("rationale")?,
    };
    validate_recommendation_record(recommendation)
        .map_err(|reason| anyhow!(InvalidRecommendationRow { reason }))
}

#[derive(Debug)]
struct InvalidRecommendationRow {
    reason: String,
}

impl fmt::Display for InvalidRecommendationRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid AI recommendation in database: {}", self.reason)
    }
}

impl std::error::Error for InvalidRecommendationRow {}

fn validate_recommendation_record(
    recommendation: Recommendation,
) -> std::result::Result<Recommendation, String> {
    if recommendation.based_on_batch_count < 0 {
        return Err(format!(
            "based_on_batch_count must be >= 0, got {}",
            recommendation.based_on_batch_count
        ));
    }
    TARGET_TEMPERATURE_C_RANGE.validate(recommendation.target_temperature_c)?;
    TARGET_STIRRER_RPM_RANGE.validate(recommendation.target_stirrer_rpm)?;
    BATCH_HEATING_MINUTES_RANGE.validate(recommendation.heating_minutes)?;
    BATCH_STIRRING_MINUTES_RANGE.validate(recommendation.stirring_minutes)?;
    RECOMMENDATION_EXPECTED_SCORE_RANGE.validate(recommendation.expected_score)?;
    Ok(recommendation)
}

fn invalid_recommendation_conversion_error(reason: String) -> rusqlite::Error {
    rusqlite_conversion_error(InvalidRecommendationRow { reason })
}

fn invalid_recommendation_reason_from_rusqlite(err: &rusqlite::Error) -> Option<&str> {
    match err {
        rusqlite::Error::FromSqlConversionFailure(_, _, source) => source
            .downcast_ref::<InvalidRecommendationRow>()
            .map(|err| err.reason.as_str()),
        _ => None,
    }
}

fn invalid_recommendation_reason_from_anyhow(err: &anyhow::Error) -> Option<&str> {
    err.downcast_ref::<InvalidRecommendationRow>()
        .map(|err| err.reason.as_str())
}

fn warn_invalid_recommendation_row(source: &str, reason: &str) {
    tracing::warn!("ignoring invalid AI recommendation row from {source}: {reason}");
}

#[derive(Debug)]
struct InvalidBatchRow {
    reason: String,
}

impl fmt::Display for InvalidBatchRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid batch in database: {}", self.reason)
    }
}

impl std::error::Error for InvalidBatchRow {}

fn validate_batch_record(batch: Batch) -> std::result::Result<Batch, String> {
    TARGET_TEMPERATURE_C_RANGE.validate(batch.target_temperature_c)?;
    TARGET_STIRRER_RPM_RANGE.validate(batch.target_stirrer_rpm)?;
    BATCH_HEATING_MINUTES_RANGE.validate(batch.heating_minutes)?;
    BATCH_STIRRING_MINUTES_RANGE.validate(batch.stirring_minutes)?;
    Ok(batch)
}

fn collect_valid_batches_from_rusqlite_rows<F>(
    rows: rusqlite::MappedRows<'_, F>,
    source: &str,
) -> Result<Vec<Batch>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Batch>,
{
    let mut batches = Vec::new();
    for row in rows {
        match row {
            Ok(batch) => batches.push(batch),
            Err(err) => {
                if let Some(reason) = invalid_batch_reason_from_rusqlite(&err) {
                    warn_invalid_batch_row(source, reason);
                    continue;
                }
                return Err(err.into());
            }
        }
    }
    Ok(batches)
}

fn collect_valid_batches_from_sqlx_rows(rows: Vec<SqliteRow>, source: &str) -> Result<Vec<Batch>> {
    let mut batches = Vec::new();
    for row in rows {
        match batch_from_sqlx_row(row) {
            Ok(batch) => batches.push(batch),
            Err(err) if invalid_batch_reason_from_anyhow(&err).is_some() => {
                let reason = invalid_batch_reason_from_anyhow(&err).unwrap();
                warn_invalid_batch_row(source, reason);
            }
            Err(err) => return Err(err),
        }
    }
    Ok(batches)
}

fn invalid_batch_conversion_error(reason: String) -> rusqlite::Error {
    rusqlite_conversion_error(InvalidBatchRow { reason })
}

fn invalid_batch_reason_from_rusqlite(err: &rusqlite::Error) -> Option<&str> {
    match err {
        rusqlite::Error::FromSqlConversionFailure(_, _, source) => source
            .downcast_ref::<InvalidBatchRow>()
            .map(|err| err.reason.as_str()),
        _ => None,
    }
}

fn invalid_batch_reason_from_anyhow(err: &anyhow::Error) -> Option<&str> {
    err.downcast_ref::<InvalidBatchRow>()
        .map(|err| err.reason.as_str())
}

fn warn_invalid_batch_row(source: &str, reason: &str) {
    tracing::warn!("skipping invalid batch row from {source}: {reason}");
}

#[derive(Debug)]
struct InvalidControlEventRow {
    reason: String,
}

impl fmt::Display for InvalidControlEventRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid control event in database: {}", self.reason)
    }
}

impl std::error::Error for InvalidControlEventRow {}

fn validate_control_event_record(event: ControlEvent) -> std::result::Result<ControlEvent, String> {
    validate_control_event_targets(
        event.target_temperature_c,
        event.target_stirrer_rpm,
        event.target_shake_speed_cpm,
    )
    .map(|_| event)
}

fn validate_control_event_targets(
    target_temperature_c: Option<f64>,
    target_stirrer_rpm: Option<f64>,
    target_shake_speed_cpm: Option<f64>,
) -> std::result::Result<(), String> {
    if let Some(value) = target_temperature_c {
        TARGET_TEMPERATURE_C_RANGE.validate(value)?;
    }
    if let Some(value) = target_stirrer_rpm {
        TARGET_STIRRER_RPM_RANGE.validate(value)?;
    }
    if let Some(value) = target_shake_speed_cpm {
        PROCESS_SHAKE_SPEED_CPM_RANGE.validate(value)?;
    }
    Ok(())
}

fn collect_valid_control_events_from_rusqlite_rows<F>(
    rows: rusqlite::MappedRows<'_, F>,
    source: &str,
) -> Result<Vec<ControlEvent>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<ControlEvent>,
{
    let mut events = Vec::new();
    for row in rows {
        match row {
            Ok(event) => events.push(event),
            Err(err) => {
                if let Some(reason) = invalid_control_event_reason_from_rusqlite(&err) {
                    warn_invalid_control_event_row(source, reason);
                    continue;
                }
                return Err(err.into());
            }
        }
    }
    Ok(events)
}

fn collect_valid_control_events_from_sqlx_rows(
    rows: Vec<SqliteRow>,
    source: &str,
) -> Result<Vec<ControlEvent>> {
    let mut events = Vec::new();
    for row in rows {
        match control_event_from_sqlx_row(row) {
            Ok(event) => events.push(event),
            Err(err) if invalid_control_event_reason_from_anyhow(&err).is_some() => {
                let reason = invalid_control_event_reason_from_anyhow(&err).unwrap();
                warn_invalid_control_event_row(source, reason);
            }
            Err(err) => return Err(err),
        }
    }
    Ok(events)
}

fn invalid_control_event_conversion_error(reason: String) -> rusqlite::Error {
    rusqlite_conversion_error(InvalidControlEventRow { reason })
}

fn invalid_control_event_reason_from_rusqlite(err: &rusqlite::Error) -> Option<&str> {
    match err {
        rusqlite::Error::FromSqlConversionFailure(_, _, source) => source
            .downcast_ref::<InvalidControlEventRow>()
            .map(|err| err.reason.as_str()),
        _ => None,
    }
}

fn invalid_control_event_reason_from_anyhow(err: &anyhow::Error) -> Option<&str> {
    err.downcast_ref::<InvalidControlEventRow>()
        .map(|err| err.reason.as_str())
}

fn warn_invalid_control_event_row(source: &str, reason: &str) {
    tracing::warn!("skipping invalid control event row from {source}: {reason}");
}

#[derive(Debug)]
struct InvalidIntegrationTaskRow {
    reason: String,
}

impl fmt::Display for InvalidIntegrationTaskRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid integration task in database: {}", self.reason)
    }
}

impl std::error::Error for InvalidIntegrationTaskRow {}

fn validate_integration_task_record(
    task: IntegrationTask,
) -> std::result::Result<IntegrationTask, String> {
    validate_integration_task_source(&task.source)?;
    if let Some(external_task_id) = task.external_task_id.as_deref() {
        validate_integration_task_external_id(external_task_id)?;
    }
    validate_integration_task_action(&task.action)?;
    validate_integration_task_status(&task.status)?;
    validate_integration_task_request_payload(&task.request)?;
    validate_integration_task_response_payload(&task.status, &task.response)?;
    Ok(task)
}

fn validate_integration_task_source(source: &str) -> std::result::Result<(), String> {
    validate_integration_task_clean_text(
        "source",
        source,
        INTEGRATION_TASK_SOURCE_MAX_CHARS,
        false,
    )?;
    if !source
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.'))
    {
        return Err(
            "source must contain only lowercase ASCII letters, digits, '.', '_' or '-'".to_string(),
        );
    }
    Ok(())
}

fn validate_integration_task_external_id(
    external_task_id: &str,
) -> std::result::Result<(), String> {
    validate_integration_task_clean_text(
        "external_task_id",
        external_task_id,
        INTEGRATION_TASK_EXTERNAL_ID_MAX_CHARS,
        true,
    )
}

fn validate_integration_task_clean_text(
    field: &str,
    value: &str,
    max_chars: usize,
    allow_internal_space: bool,
) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if value.trim() != value {
        return Err(format!(
            "{field} must not have leading or trailing whitespace"
        ));
    }
    let chars = value.chars().count();
    if chars > max_chars {
        return Err(format!(
            "{field} must be at most {max_chars} characters, got {chars}"
        ));
    }
    for ch in value.chars() {
        if is_invisible_format_char(ch) {
            return Err(format!(
                "{field} must not contain invisible format characters"
            ));
        }
        if ch.is_control() {
            return Err(format!("{field} must not contain control characters"));
        }
        if ch.is_whitespace() && ch != ' ' {
            return Err(format!("{field} must not contain non-space whitespace"));
        }
        if ch == ' ' && !allow_internal_space {
            return Err(format!("{field} must not contain spaces"));
        }
    }
    Ok(())
}

fn is_invisible_format_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

fn validate_integration_task_action(action: &str) -> std::result::Result<(), String> {
    if INTEGRATION_TASK_ACTIONS.contains(&action) {
        return Ok(());
    }
    Err(format!(
        "action must be one of {}",
        INTEGRATION_TASK_ACTIONS.join(", ")
    ))
}

fn validate_integration_task_status(status: &str) -> std::result::Result<(), String> {
    if INTEGRATION_TASK_STATUSES.contains(&status) {
        return Ok(());
    }
    Err(format!(
        "status must be one of {}",
        INTEGRATION_TASK_STATUSES.join(", ")
    ))
}

fn validate_integration_task_terminal_status(status: &str) -> std::result::Result<(), String> {
    if INTEGRATION_TASK_TERMINAL_STATUSES.contains(&status) {
        return Ok(());
    }
    Err(format!(
        "status update must be one of {}",
        INTEGRATION_TASK_TERMINAL_STATUSES.join(", ")
    ))
}

fn validate_integration_task_request_payload(request: &Value) -> std::result::Result<(), String> {
    if request.is_object() {
        return Ok(());
    }
    Err("request JSON must be an object".to_string())
}

fn validate_integration_task_response_payload(
    status: &str,
    response: &Value,
) -> std::result::Result<(), String> {
    if status == "received" && response.is_null() {
        return Ok(());
    }
    if response.is_object() {
        return Ok(());
    }
    Err("response JSON must be an object once a task leaves received status".to_string())
}

fn invalid_integration_task_conversion_error(reason: String) -> rusqlite::Error {
    rusqlite_conversion_error(InvalidIntegrationTaskRow { reason })
}

fn invalid_integration_task_reason_from_rusqlite(err: &rusqlite::Error) -> Option<&str> {
    match err {
        rusqlite::Error::FromSqlConversionFailure(_, _, source) => source
            .downcast_ref::<InvalidIntegrationTaskRow>()
            .map(|err| err.reason.as_str()),
        _ => None,
    }
}

fn invalid_integration_task_reason_from_anyhow(err: &anyhow::Error) -> Option<&str> {
    err.downcast_ref::<InvalidIntegrationTaskRow>()
        .map(|err| err.reason.as_str())
}

fn warn_invalid_integration_task_row(source: &str, reason: &str) {
    tracing::warn!("skipping invalid integration task row from {source}: {reason}");
}

#[derive(Debug)]
struct InvalidProcessStepRow {
    reason: String,
}

impl fmt::Display for InvalidProcessStepRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid process step in database: {}", self.reason)
    }
}

impl std::error::Error for InvalidProcessStepRow {}

fn validate_process_step_record(step: ProcessStep) -> std::result::Result<ProcessStep, String> {
    let step_id = step.id;
    TARGET_TEMPERATURE_C_RANGE
        .validate(step.target_temperature_c)
        .map_err(|reason| format!("process step {step_id} {reason}"))?;
    PROCESS_RAMP_RATE_C_MIN_RANGE
        .validate(step.ramp_rate_c_min)
        .map_err(|reason| format!("process step {step_id} {reason}"))?;
    PROCESS_DURATION_MINUTES_RANGE
        .validate(step.duration_minutes)
        .map_err(|reason| format!("process step {step_id} {reason}"))?;
    TARGET_STIRRER_RPM_RANGE
        .validate(step.target_stirrer_rpm)
        .map_err(|reason| format!("process step {step_id} {reason}"))?;
    PROCESS_SHAKE_SPEED_CPM_RANGE
        .validate(step.target_shake_speed_cpm)
        .map_err(|reason| format!("process step {step_id} {reason}"))?;
    PROCESS_PRESSURE_MPA_RANGE
        .validate(step.target_pressure_mpa)
        .map_err(|reason| format!("process step {step_id} {reason}"))?;
    Ok(step)
}

fn invalid_process_step_conversion_error(reason: String) -> rusqlite::Error {
    rusqlite_conversion_error(InvalidProcessStepRow { reason })
}

#[derive(Debug)]
struct InvalidBatchOutcomeRow {
    reason: String,
}

impl fmt::Display for InvalidBatchOutcomeRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid batch outcome in database: {}", self.reason)
    }
}

impl std::error::Error for InvalidBatchOutcomeRow {}

fn validate_batch_outcome_record(
    outcome: BatchOutcome,
) -> std::result::Result<BatchOutcome, String> {
    TARGET_TEMPERATURE_C_RANGE.validate(outcome.target_temperature_c)?;
    TARGET_STIRRER_RPM_RANGE.validate(outcome.target_stirrer_rpm)?;
    BATCH_HEATING_MINUTES_RANGE.validate(outcome.heating_minutes)?;
    BATCH_STIRRING_MINUTES_RANGE.validate(outcome.stirring_minutes)?;
    PRODUCT_RESULT_YIELD_PERCENT_RANGE.validate(outcome.yield_percent)?;
    PRODUCT_RESULT_RATIO_RANGE.validate(outcome.product_ratio)?;
    Ok(outcome)
}

fn collect_valid_batch_outcomes_from_rusqlite_rows<F>(
    rows: rusqlite::MappedRows<'_, F>,
    source: &str,
) -> Result<Vec<BatchOutcome>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<BatchOutcome>,
{
    let mut outcomes = Vec::new();
    for row in rows {
        match row {
            Ok(outcome) => outcomes.push(outcome),
            Err(err) => {
                if let Some(reason) = invalid_batch_outcome_reason_from_rusqlite(&err) {
                    warn_invalid_batch_outcome_row(source, reason);
                    continue;
                }
                return Err(err.into());
            }
        }
    }
    Ok(outcomes)
}

fn collect_valid_batch_outcomes_from_sqlx_rows(
    rows: Vec<SqliteRow>,
    source: &str,
) -> Result<Vec<BatchOutcome>> {
    let mut outcomes = Vec::new();
    for row in rows {
        match batch_outcome_from_sqlx_row(row) {
            Ok(outcome) => outcomes.push(outcome),
            Err(err) if invalid_batch_outcome_reason_from_anyhow(&err).is_some() => {
                let reason = invalid_batch_outcome_reason_from_anyhow(&err).unwrap();
                warn_invalid_batch_outcome_row(source, reason);
            }
            Err(err) => return Err(err),
        }
    }
    Ok(outcomes)
}

fn invalid_batch_outcome_conversion_error(reason: String) -> rusqlite::Error {
    rusqlite_conversion_error(InvalidBatchOutcomeRow { reason })
}

fn invalid_batch_outcome_reason_from_rusqlite(err: &rusqlite::Error) -> Option<&str> {
    match err {
        rusqlite::Error::FromSqlConversionFailure(_, _, source) => source
            .downcast_ref::<InvalidBatchOutcomeRow>()
            .map(|err| err.reason.as_str()),
        _ => None,
    }
}

fn invalid_batch_outcome_reason_from_anyhow(err: &anyhow::Error) -> Option<&str> {
    err.downcast_ref::<InvalidBatchOutcomeRow>()
        .map(|err| err.reason.as_str())
}

fn warn_invalid_batch_outcome_row(source: &str, reason: &str) {
    tracing::warn!("skipping invalid batch outcome row from {source}: {reason}");
}

#[derive(Debug)]
struct InvalidSensorSampleRow {
    reason: String,
}

impl fmt::Display for InvalidSensorSampleRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid sensor sample in database: {}", self.reason)
    }
}

impl std::error::Error for InvalidSensorSampleRow {}

fn sensor_sample_record_from_row(
    row: &rusqlite::Row<'_>,
    batch_id_index: usize,
    sample_start_index: usize,
    captured_at_index: usize,
) -> rusqlite::Result<SensorSampleRecord> {
    Ok(SensorSampleRecord {
        batch_id: row.get(batch_id_index)?,
        sample: sensor_snapshot_from_row(row, sample_start_index, captured_at_index)?,
    })
}

fn sensor_snapshot_from_row(
    row: &rusqlite::Row<'_>,
    start_index: usize,
    captured_at_index: usize,
) -> rusqlite::Result<SensorSnapshot> {
    let captured_at: String = row.get(captured_at_index)?;
    let tilt_state = sensor_tilt_state_from_i64(row.get(start_index + 4)?)?;
    let sample = SensorSnapshot {
        temperature_c: row.get(start_index)?,
        pressure_mpa: row.get(start_index + 1)?,
        stirrer_rpm: row.get(start_index + 2)?,
        shake_speed_cpm: row.get(start_index + 3)?,
        tilt_state,
        tilt_angle_deg: row.get(start_index + 5)?,
        flow_rate_l_min: row.get(start_index + 6)?,
        product_concentration_percent: row.get(start_index + 7)?,
        ph: row.get(start_index + 8)?,
        captured_at: parse_dt(&captured_at)?,
    };
    validate_sensor_snapshot(&sample)
        .map(|_| sample)
        .map_err(|reason| invalid_sensor_sample_conversion_error(reason))
}

fn sensor_sample_record_from_sqlx_row(row: SqliteRow) -> Result<SensorSampleRecord> {
    let captured_at: String = row.try_get("captured_at")?;
    let tilt_state: i64 = row.try_get("tilt_state")?;
    let record = SensorSampleRecord {
        batch_id: row.try_get("batch_id")?,
        sample: SensorSnapshot {
            temperature_c: row.try_get("temperature_c")?,
            pressure_mpa: row.try_get("pressure_mpa")?,
            stirrer_rpm: row.try_get("stirrer_rpm")?,
            shake_speed_cpm: row.try_get("shake_speed_cpm")?,
            tilt_state: sensor_tilt_state_from_i64_anyhow(tilt_state)?,
            tilt_angle_deg: row.try_get("tilt_angle_deg")?,
            flow_rate_l_min: row.try_get("flow_rate_l_min")?,
            product_concentration_percent: row.try_get("product_concentration_percent")?,
            ph: row.try_get("ph")?,
            captured_at: parse_dt_anyhow(&captured_at)?,
        },
    };
    validate_sensor_sample_record(record)
}

fn validate_sensor_sample_record(record: SensorSampleRecord) -> Result<SensorSampleRecord> {
    validate_sensor_snapshot(&record.sample)
        .map(|_| record)
        .map_err(|reason| anyhow!(InvalidSensorSampleRow { reason }))
}

fn sensor_tilt_state_from_i64(value: i64) -> rusqlite::Result<u8> {
    sensor_tilt_state_from_i64_anyhow(value).map_err(|err| {
        if let Some(reason) = invalid_sensor_sample_reason_from_anyhow(&err) {
            return invalid_sensor_sample_conversion_error(reason.to_string());
        }
        rusqlite_conversion_error(err)
    })
}

fn sensor_tilt_state_from_i64_anyhow(value: i64) -> Result<u8> {
    u8::try_from(value).map_err(|_| {
        anyhow!(InvalidSensorSampleRow {
            reason: format!(
                "tilt_state must be 0 or 1 for the shake vessel binary tilt sensor, got {value}"
            ),
        })
    })
}

fn collect_valid_sensor_sample_records_from_sqlx_rows(
    rows: Vec<SqliteRow>,
    source: &str,
) -> Result<Vec<SensorSampleRecord>> {
    let mut records = Vec::new();
    for row in rows {
        match sensor_sample_record_from_sqlx_row(row) {
            Ok(record) => records.push(record),
            Err(err) if invalid_sensor_sample_reason_from_anyhow(&err).is_some() => {
                let reason = invalid_sensor_sample_reason_from_anyhow(&err).unwrap();
                warn_invalid_sensor_sample_row(source, reason);
            }
            Err(err) => return Err(err),
        }
    }
    Ok(records)
}

fn invalid_sensor_sample_conversion_error(reason: String) -> rusqlite::Error {
    rusqlite_conversion_error(InvalidSensorSampleRow { reason })
}

fn invalid_sensor_sample_reason_from_rusqlite(err: &rusqlite::Error) -> Option<&str> {
    match err {
        rusqlite::Error::FromSqlConversionFailure(_, _, source) => source
            .downcast_ref::<InvalidSensorSampleRow>()
            .map(|err| err.reason.as_str()),
        _ => None,
    }
}

fn invalid_sensor_sample_reason_from_anyhow(err: &anyhow::Error) -> Option<&str> {
    err.downcast_ref::<InvalidSensorSampleRow>()
        .map(|err| err.reason.as_str())
}

fn warn_invalid_sensor_sample_row(source: &str, reason: &str) {
    tracing::warn!("skipping invalid sensor sample row from {source}: {reason}");
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

fn safe_command_from_recommendation(recommendation: &Recommendation, reason: &str) -> SafeCommand {
    SafeCommand {
        target_temperature_c: recommendation.target_temperature_c,
        heat_time_s: recommendation.heating_minutes * 60.0,
        hold_time_s: recommendation.stirring_minutes * 60.0,
        cool_time_s: 0.0,
        target_stirrer_rpm: recommendation.target_stirrer_rpm,
        target_shake_speed_cpm: 0.0,
        target_pressure_mpa: 0.0,
        reason: reason.to_string(),
    }
}

fn insert_control_event_in_rusqlite_tx(
    tx: &rusqlite::Transaction<'_>,
    batch_id: Option<i64>,
    event_type: &str,
    command: Option<&SafeCommand>,
    reason: &str,
    created_at: &str,
) -> Result<()> {
    let previous_hash: Option<String> = tx
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
        .optional()
        .context("failed to read previous audit hash")?;
    let target_temperature_c = command.map(|cmd| cmd.target_temperature_c);
    let target_stirrer_rpm = command.map(|cmd| cmd.target_stirrer_rpm);
    let target_shake_speed_cpm = command.map(|cmd| cmd.target_shake_speed_cpm);
    ensure_valid_control_event_targets_for_insert(
        target_temperature_c,
        target_stirrer_rpm,
        target_shake_speed_cpm,
    )?;
    let event_hash = control_event_hash(
        previous_hash.as_deref(),
        batch_id,
        event_type,
        target_temperature_c,
        target_stirrer_rpm,
        target_shake_speed_cpm,
        reason,
        created_at,
    )?;
    tx.execute(
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
    )
    .context("failed to insert audit event")?;
    Ok(())
}

async fn insert_control_event_in_sqlx_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    batch_id: Option<i64>,
    event_type: &str,
    command: Option<&SafeCommand>,
    reason: &str,
    created_at: &str,
) -> Result<()> {
    let previous_hash: Option<String> = sqlx::query_scalar(
        r#"
        SELECT event_hash
        FROM control_events
        WHERE event_hash IS NOT NULL
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&mut **tx)
    .await
    .context("failed to read previous audit hash with SQLx")?;
    let target_temperature_c = command.map(|cmd| cmd.target_temperature_c);
    let target_stirrer_rpm = command.map(|cmd| cmd.target_stirrer_rpm);
    let target_shake_speed_cpm = command.map(|cmd| cmd.target_shake_speed_cpm);
    ensure_valid_control_event_targets_for_insert(
        target_temperature_c,
        target_stirrer_rpm,
        target_shake_speed_cpm,
    )?;
    let event_hash = control_event_hash(
        previous_hash.as_deref(),
        batch_id,
        event_type,
        target_temperature_c,
        target_stirrer_rpm,
        target_shake_speed_cpm,
        reason,
        created_at,
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
    .execute(&mut **tx)
    .await
    .context("failed to insert audit event with SQLx")?;
    Ok(())
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
    let step = ProcessStep {
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
    };
    validate_process_step_record(step)
        .map_err(|reason| invalid_process_step_conversion_error(reason))
}

fn process_step_from_sqlx_row(row: SqliteRow) -> Result<ProcessStep> {
    let created_at: String = row.try_get("created_at")?;
    let updated_at: String = row.try_get("updated_at")?;
    let step = ProcessStep {
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
    };
    validate_process_step_record(step).map_err(|reason| anyhow!(InvalidProcessStepRow { reason }))
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
    // Mirror configure_connection() so the SQLx read/write pool behaves
    // identically to the rusqlite connections (see that function for the
    // rationale behind each pragma on the RK3568/eMMC edge board).
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .pragma("wal_autocheckpoint", "400")
        .pragma("temp_store", "MEMORY")
        .pragma("cache_size", "-4096")
        .pragma("mmap_size", SQLITE_MMAP_SIZE_BYTES.to_string())
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
        -- synchronous=NORMAL is safe under WAL: the database stays consistent
        -- across application and OS crashes; only a power loss between the last
        -- WAL checkpoint and the crash can drop the very last committed
        -- transaction. On the RK3568/eMMC edge board this cuts fsync count
        -- dramatically (lower CPU, far less write amplification). If durability
        -- of the last audit event on sudden power loss is mandatory, raise this
        -- back to FULL and rely on the hardware brown-out/hold-up budget.
        PRAGMA synchronous=NORMAL;
        -- Bound the WAL so it cannot grow unbounded between checkpoints on a
        -- long-running unattended board (keeps memory-mapped WAL and disk small).
        PRAGMA wal_autocheckpoint=400;
        -- Keep scratch tables and sort/temp results in RAM instead of spilling
        -- to the eMMC, and cap the page cache to a modest ~4 MiB (negative value
        -- is KiB) so the daemon stays inside the PRD <30 MB memory envelope.
        PRAGMA temp_store=MEMORY;
        PRAGMA cache_size=-4096;
        "#,
    )?;
    // A bounded mmap window lets SQLite serve hot history/index pages without
    // copying them through the small heap page cache. mmap_size is a per-
    // connection setting and reserves address space on demand; it does not
    // eagerly allocate 64 MiB of RSS on the 2 GiB RK3568 target.
    conn.pragma_update(None, "mmap_size", SQLITE_MMAP_SIZE_BYTES)?;
    Ok(conn)
}

fn create_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(INDEX_SQL)?;
    Ok(())
}

fn prepare_integration_task_unique_index(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "integration_tasks", "id")? {
        anyhow::bail!(
            "legacy integration_tasks schema is missing primary key column id; refusing a lossy automatic migration; preserve the database and run an explicit export/import migration"
        );
    }

    let existing_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_integration_tasks_unique_active_external_task_id'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let normalized = existing_sql.as_deref().map(|sql| {
        sql.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    });
    let expected_index = normalized.as_deref().is_some_and(|sql| {
        sql.contains("create unique index")
            && sql.contains("on integration_tasks(source, external_task_id)")
            && sql.contains("where external_task_id is not null")
            && sql.contains("status in ('received', 'executing', 'executed')")
    });

    let duplicate_count: i64 = conn.query_row(
        r#"
        SELECT COUNT(*)
        FROM integration_tasks AS duplicate
        WHERE duplicate.external_task_id IS NOT NULL
          AND duplicate.status IN ('received', 'executing', 'executed')
          AND EXISTS (
              SELECT 1
              FROM integration_tasks AS canonical
              WHERE canonical.source = duplicate.source
                AND canonical.external_task_id = duplicate.external_task_id
                AND canonical.status IN ('received', 'executing', 'executed')
                AND canonical.id < duplicate.id
          )
        "#,
        [],
        |row| row.get(0),
    )?;

    if !expected_index || duplicate_count > 0 {
        conn.execute(
            "DROP INDEX IF EXISTS idx_integration_tasks_unique_active_external_task_id",
            [],
        )?;
    }
    if duplicate_count > 0 {
        let repaired = conn.execute(
            r#"
            UPDATE integration_tasks AS duplicate
            SET external_task_id = NULL
            WHERE duplicate.external_task_id IS NOT NULL
              AND duplicate.status IN ('received', 'executing', 'executed')
              AND EXISTS (
                  SELECT 1
                  FROM integration_tasks AS canonical
                  WHERE canonical.source = duplicate.source
                    AND canonical.external_task_id = duplicate.external_task_id
                    AND canonical.status IN ('received', 'executing', 'executed')
                    AND canonical.id < duplicate.id
              )
            "#,
            [],
        )?;
        if repaired as i64 != duplicate_count {
            anyhow::bail!(
                "legacy integration task duplicate repair changed {repaired} rows but expected {duplicate_count}; rolling back migration"
            );
        }
        tracing::warn!(
            repaired,
            "legacy integration task duplicates preserved with later duplicate external_task_id values cleared before unique-index creation"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupReport {
    pub source: String,
    pub destination: String,
    pub copied_pages: i64,
    pub size_bytes: u64,
    pub duration_ms: u128,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreReport {
    pub source: String,
    pub destination: String,
    pub preserved_existing: Option<String>,
    pub removed_sidecars: Vec<String>,
    pub preserved_sidecars: Vec<String>,
    pub integrity_check: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub tables: Vec<String>,
}

fn path_with_file_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "reactor.sqlite3".to_string());
    name.push('.');
    name.push_str(suffix);
    path.with_file_name(name)
}

fn path_with_raw_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "reactor.sqlite3".to_string());
    name.push_str(suffix);
    path.with_file_name(name)
}

fn unique_path(base: PathBuf) -> Result<PathBuf> {
    if !base
        .try_exists()
        .with_context(|| format!("failed to inspect {}", base.display()))?
    {
        return Ok(base);
    }
    for index in 1..=10_000 {
        let candidate = PathBuf::from(format!("{}.{}", base.display(), index));
        if !candidate
            .try_exists()
            .with_context(|| format!("failed to inspect {}", candidate.display()))?
        {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "could not allocate a non-overwriting pre-restore path for {}",
        base.display()
    ))
}

fn copy_evidence_file_atomic(source: &Path, destination: &Path, label: &str) -> Result<()> {
    let tmp = evidence_tmp_path(destination);
    remove_restore_tmp_if_present(&tmp)?;
    let copy_result = (|| {
        std::fs::copy(source, &tmp).with_context(|| {
            format!(
                "failed to preserve {label} {} -> temporary evidence file {}",
                source.display(),
                tmp.display()
            )
        })?;
        sync_file(&tmp).with_context(|| {
            format!(
                "failed to sync temporary preserved {label} {}",
                tmp.display()
            )
        })?;
        let source_len = std::fs::metadata(source)
            .with_context(|| format!("failed to stat source evidence {}", source.display()))?
            .len();
        let tmp_len = std::fs::metadata(&tmp)
            .with_context(|| format!("failed to stat temporary evidence {}", tmp.display()))?
            .len();
        if source_len != tmp_len {
            return Err(anyhow!(
                "temporary preserved {label} {} has size {tmp_len}, expected {source_len}",
                tmp.display()
            ));
        }
        std::fs::rename(&tmp, destination).with_context(|| {
            format!(
                "failed to publish preserved {label} {}",
                destination.display()
            )
        })?;
        sync_parent_dir(destination)?;
        Ok(())
    })();
    if let Err(err) = copy_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

fn evidence_tmp_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "restore-evidence".to_string());
    name.push_str(&format!(".evidence.tmp.{}", std::process::id()));
    path.with_file_name(name)
}

fn restore_tmp_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "reactor.sqlite3".to_string());
    name.push_str(&format!(".restore.tmp.{}", std::process::id()));
    path.with_file_name(name)
}

fn remove_restore_tmp_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(anyhow!(
            "failed to remove stale temporary restore file {}: {err}",
            path.display()
        )),
    }
}

fn sync_file(path: &Path) -> Result<()> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to reopen {} for sync", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync {}", path.display()))
}

fn sync_parent_dir(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        std::fs::File::open(parent)
            .with_context(|| format!("failed to open directory {} for sync", parent.display()))?
            .sync_all()
            .with_context(|| format!("failed to sync directory {}", parent.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
    Ok(())
}

fn validate_restored_db_file(path: &Path) -> Result<(String, Vec<String>)> {
    let conn = Connection::open(path)
        .with_context(|| format!("restored db is unreadable: {}", path.display()))?;
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .context("failed to run SQLite integrity_check")?;
    if integrity != "ok" {
        return Err(anyhow!("restored db failed integrity_check: {integrity}"));
    }
    let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to list restored tables")?;
    Ok((integrity, tables))
}

fn sha256_hex(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open {} for sha256", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

impl DbEncryption {
    pub fn from_key(key: [u8; 32], key_source: &'static str) -> Self {
        Self {
            cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key)),
            key_source,
        }
    }

    pub fn encrypt_json(&self, plaintext: &str) -> Result<String> {
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

    pub fn decrypt_json_if_needed_anyhow(&self, value: &str) -> Result<String> {
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

pub fn parse_encryption_key(value: &str) -> Result<[u8; 32]> {
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

fn ensure_valid_sensor_sample_for_insert(sample: &SensorSnapshot) -> Result<()> {
    validate_sensor_snapshot(sample)
        .map_err(|reason| anyhow!("invalid sensor sample rejected before DB insert: {reason}"))
}

fn ensure_valid_batch_targets_for_insert(
    target_temperature_c: f64,
    target_stirrer_rpm: f64,
    heating_minutes: f64,
    stirring_minutes: f64,
) -> Result<()> {
    TARGET_TEMPERATURE_C_RANGE
        .validate(target_temperature_c)
        .map_err(|reason| anyhow!("invalid batch target rejected before DB insert: {reason}"))?;
    TARGET_STIRRER_RPM_RANGE
        .validate(target_stirrer_rpm)
        .map_err(|reason| anyhow!("invalid batch target rejected before DB insert: {reason}"))?;
    BATCH_HEATING_MINUTES_RANGE
        .validate(heating_minutes)
        .map_err(|reason| anyhow!("invalid batch target rejected before DB insert: {reason}"))?;
    BATCH_STIRRING_MINUTES_RANGE
        .validate(stirring_minutes)
        .map_err(|reason| anyhow!("invalid batch target rejected before DB insert: {reason}"))?;
    Ok(())
}

fn ensure_valid_process_step_for_insert(step: &NewProcessStep) -> Result<()> {
    TARGET_TEMPERATURE_C_RANGE
        .validate(step.target_temperature_c)
        .map_err(|reason| anyhow!("invalid process step rejected before DB insert: {reason}"))?;
    PROCESS_RAMP_RATE_C_MIN_RANGE
        .validate(step.ramp_rate_c_min)
        .map_err(|reason| anyhow!("invalid process step rejected before DB insert: {reason}"))?;
    PROCESS_DURATION_MINUTES_RANGE
        .validate(step.duration_minutes)
        .map_err(|reason| anyhow!("invalid process step rejected before DB insert: {reason}"))?;
    TARGET_STIRRER_RPM_RANGE
        .validate(step.target_stirrer_rpm)
        .map_err(|reason| anyhow!("invalid process step rejected before DB insert: {reason}"))?;
    PROCESS_SHAKE_SPEED_CPM_RANGE
        .validate(step.target_shake_speed_cpm)
        .map_err(|reason| anyhow!("invalid process step rejected before DB insert: {reason}"))?;
    PROCESS_PRESSURE_MPA_RANGE
        .validate(step.target_pressure_mpa)
        .map_err(|reason| anyhow!("invalid process step rejected before DB insert: {reason}"))?;
    Ok(())
}

fn ensure_valid_product_result_for_insert(result: &ProductResult) -> Result<()> {
    validate_finite_range_for_insert(
        "product result",
        "yield_percent",
        result.yield_percent,
        0.0,
        100.0,
    )?;
    validate_finite_range_for_insert(
        "product result",
        "product_ratio",
        result.product_ratio,
        0.0,
        1.0,
    )?;
    Ok(())
}

fn ensure_valid_recommendation_for_insert(recommendation: &Recommendation) -> Result<()> {
    validate_recommendation_record(recommendation.clone()).map_err(|reason| {
        anyhow!("invalid AI recommendation rejected before DB insert: {reason}")
    })?;
    Ok(())
}

fn ensure_valid_control_event_targets_for_insert(
    target_temperature_c: Option<f64>,
    target_stirrer_rpm: Option<f64>,
    target_shake_speed_cpm: Option<f64>,
) -> Result<()> {
    validate_control_event_targets(
        target_temperature_c,
        target_stirrer_rpm,
        target_shake_speed_cpm,
    )
    .map_err(|reason| anyhow!("invalid control event target rejected before DB insert: {reason}"))
}

fn ensure_valid_integration_task_create_for_insert(
    source: &str,
    external_task_id: Option<&str>,
    action: &str,
    request: &Value,
) -> Result<()> {
    validate_integration_task_source(source).map_err(|reason| {
        anyhow!("invalid integration task rejected before DB insert: {reason}")
    })?;
    if let Some(external_task_id) = external_task_id {
        validate_integration_task_external_id(external_task_id).map_err(|reason| {
            anyhow!("invalid integration task rejected before DB insert: {reason}")
        })?;
    }
    validate_integration_task_action(action).map_err(|reason| {
        anyhow!("invalid integration task rejected before DB insert: {reason}")
    })?;
    validate_integration_task_request_payload(request).map_err(|reason| {
        anyhow!("invalid integration task rejected before DB insert: {reason}")
    })?;
    Ok(())
}

fn ensure_valid_integration_task_update_for_insert(status: &str, response: &Value) -> Result<()> {
    validate_integration_task_terminal_status(status).map_err(|reason| {
        anyhow!("invalid integration task update rejected before DB insert: {reason}")
    })?;
    validate_integration_task_response_payload(status, response).map_err(|reason| {
        anyhow!("invalid integration task update rejected before DB insert: {reason}")
    })?;
    Ok(())
}

fn validate_finite_range_for_insert(
    subject: &str,
    field: &str,
    value: f64,
    min: f64,
    max: f64,
) -> Result<()> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        anyhow::bail!(
            "invalid {subject} rejected before DB insert: {field} must be between {min} and {max}"
        );
    }
    Ok(())
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

#[cfg(test)]
mod tuning_tests {
    use super::*;

    fn pragma_i64(conn: &Connection, name: &str) -> i64 {
        conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn rusqlite_connection_uses_rk3568_storage_tuning() {
        let dir = tempfile::tempdir().unwrap();
        let conn = configure_connection(Connection::open(dir.path().join("rusqlite.db")).unwrap())
            .unwrap();
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();

        assert_eq!(journal_mode, "wal");
        assert_eq!(pragma_i64(&conn, "synchronous"), 1);
        assert_eq!(pragma_i64(&conn, "wal_autocheckpoint"), 400);
        assert_eq!(pragma_i64(&conn, "temp_store"), 2);
        assert_eq!(pragma_i64(&conn, "cache_size"), -4096);
        assert_eq!(pragma_i64(&conn, "mmap_size"), SQLITE_MMAP_SIZE_BYTES);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sqlx_connection_uses_rk3568_storage_tuning() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_sqlx_pool(&dir.path().join("sqlx.db")).unwrap();

        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&pool)
            .await
            .unwrap();
        let wal_autocheckpoint: i64 = sqlx::query_scalar("PRAGMA wal_autocheckpoint")
            .fetch_one(&pool)
            .await
            .unwrap();
        let temp_store: i64 = sqlx::query_scalar("PRAGMA temp_store")
            .fetch_one(&pool)
            .await
            .unwrap();
        let cache_size: i64 = sqlx::query_scalar("PRAGMA cache_size")
            .fetch_one(&pool)
            .await
            .unwrap();
        let mmap_size: i64 = sqlx::query_scalar("PRAGMA mmap_size")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(journal_mode, "wal");
        assert_eq!(synchronous, 1);
        assert_eq!(wal_autocheckpoint, 400);
        assert_eq!(temp_store, 2);
        assert_eq!(cache_size, -4096);
        assert_eq!(mmap_size, SQLITE_MMAP_SIZE_BYTES);
        pool.close().await;
    }
}
