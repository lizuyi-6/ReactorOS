use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::{control::SafeCommand, optimizer::Recommendation, state::SensorSnapshot};

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
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
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory {}", parent.display())
            })?;
        }
        let conn = Connection::open(path.as_ref())
            .with_context(|| format!("failed to open database {}", path.as_ref().display()))?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_memory() -> Result<Self> {
        let db = Self {
            conn: Arc::new(Mutex::new(Connection::open_in_memory()?)),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn migrate(&self) -> Result<()> {
        let conn = self.lock()?;
        conn.execute_batch(
            r#"
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
            "#,
        )?;
        let has_legacy_pressure_kpa = column_exists(&conn, "sensor_samples", "pressure_kpa")?;
        add_column_if_missing(
            &conn,
            "sensor_samples",
            "pressure_mpa",
            "REAL NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            &conn,
            "sensor_samples",
            "shake_speed_cpm",
            "REAL NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            &conn,
            "sensor_samples",
            "tilt_angle_deg",
            "REAL NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            &conn,
            "sensor_samples",
            "tilt_state",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            &conn,
            "sensor_samples",
            "flow_rate_l_min",
            "REAL NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            &conn,
            "sensor_samples",
            "product_concentration_percent",
            "REAL NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(&conn, "sensor_samples", "ph", "REAL NOT NULL DEFAULT 7")?;
        add_column_if_missing(&conn, "control_events", "target_shake_speed_cpm", "REAL")?;
        add_column_if_missing(
            &conn,
            "batches",
            "process_id",
            "INTEGER REFERENCES processes(id)",
        )?;
        if has_legacy_pressure_kpa {
            conn.execute(
                "UPDATE sensor_samples SET pressure_mpa = pressure_kpa / 1000.0 WHERE pressure_mpa = 0 AND pressure_kpa > 0",
                [],
            )?;
        }
        Ok(())
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
        let conn = self.lock()?;
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

    pub fn create_process(&self, name: &str, description: &str) -> Result<ProcessDefinition> {
        let now = Utc::now();
        let conn = self.lock()?;
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

    pub fn update_process(
        &self,
        process_id: i64,
        name: &str,
        description: &str,
        status: &str,
    ) -> Result<Option<ProcessDefinition>> {
        let now = Utc::now();
        let conn = self.lock()?;
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

    pub fn list_processes(&self) -> Result<Vec<ProcessDefinition>> {
        let conn = self.lock()?;
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

    pub fn process_detail(&self, process_id: i64) -> Result<Option<ProcessDetail>> {
        let conn = self.lock()?;
        let Some(process) = process_summary_by_id(&conn, process_id)? else {
            return Ok(None);
        };
        let steps = process_steps_for_conn(&conn, process_id)?;
        Ok(Some(ProcessDetail { process, steps }))
    }

    pub fn add_process_step(
        &self,
        process_id: i64,
        step: &NewProcessStep,
    ) -> Result<Option<ProcessStep>> {
        let conn = self.lock()?;
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

    pub fn update_process_step(
        &self,
        process_id: i64,
        step_id: i64,
        step: &NewProcessStep,
    ) -> Result<Option<ProcessStep>> {
        let now = Utc::now();
        let conn = self.lock()?;
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

    pub fn mark_process_applied(&self, process_id: i64) -> Result<Option<ProcessDefinition>> {
        let now = Utc::now();
        let conn = self.lock()?;
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

    pub fn finish_batch(&self, batch_id: i64) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE batches SET finished_at = ?1 WHERE id = ?2 AND finished_at IS NULL",
            params![Utc::now().to_rfc3339(), batch_id],
        )?;
        Ok(())
    }

    pub fn insert_sample(&self, batch_id: Option<i64>, sample: &SensorSnapshot) -> Result<()> {
        let conn = self.lock()?;
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

    pub fn recent_samples(&self, limit: usize) -> Result<Vec<SensorSnapshot>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM sensor_samples
            ORDER BY id DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let captured_at: String = row.get(9)?;
            Ok(SensorSnapshot {
                temperature_c: row.get(0)?,
                pressure_mpa: row.get(1)?,
                stirrer_rpm: row.get(2)?,
                shake_speed_cpm: row.get(3)?,
                tilt_state: row.get(4)?,
                tilt_angle_deg: row.get(5)?,
                flow_rate_l_min: row.get(6)?,
                product_concentration_percent: row.get(7)?,
                ph: row.get(8)?,
                captured_at: parse_dt(&captured_at)?,
            })
        })?;

        let mut samples = Vec::new();
        for row in rows {
            samples.push(row?);
        }
        samples.reverse();
        Ok(samples)
    }

    pub fn recent_sample_records(&self, limit: usize) -> Result<Vec<SensorSampleRecord>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM sensor_samples
            ORDER BY id DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
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
        })?;

        let mut samples = Vec::new();
        for row in rows {
            samples.push(row?);
        }
        samples.reverse();
        Ok(samples)
    }

    pub fn samples_between(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SensorSnapshot>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT temperature_c, pressure_mpa, stirrer_rpm,
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
                let captured_at: String = row.get(9)?;
                Ok(SensorSnapshot {
                    temperature_c: row.get(0)?,
                    pressure_mpa: row.get(1)?,
                    stirrer_rpm: row.get(2)?,
                    shake_speed_cpm: row.get(3)?,
                    tilt_state: row.get(4)?,
                    tilt_angle_deg: row.get(5)?,
                    flow_rate_l_min: row.get(6)?,
                    product_concentration_percent: row.get(7)?,
                    ph: row.get(8)?,
                    captured_at: parse_dt(&captured_at)?,
                })
            },
        )?;

        let mut samples = Vec::new();
        for row in rows {
            samples.push(row?);
        }
        Ok(samples)
    }

    pub fn insert_control_event(
        &self,
        batch_id: Option<i64>,
        event_type: &str,
        command: Option<&SafeCommand>,
        reason: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            r#"
            INSERT INTO control_events
                (batch_id, event_type, target_temperature_c, target_stirrer_rpm, target_shake_speed_cpm, reason, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                batch_id,
                event_type,
                command.map(|cmd| cmd.target_temperature_c),
                command.map(|cmd| cmd.target_stirrer_rpm),
                command.map(|cmd| cmd.target_shake_speed_cpm),
                reason,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn insert_product_result(&self, result: &ProductResult) -> Result<()> {
        let conn = self.lock()?;
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

    pub fn insert_recommendation(&self, recommendation: &Recommendation) -> Result<()> {
        let conn = self.lock()?;
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

    pub fn latest_recommendation(&self) -> Result<Option<Recommendation>> {
        let conn = self.lock()?;
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

    pub fn batch_outcomes(&self) -> Result<Vec<BatchOutcome>> {
        let conn = self.lock()?;
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

    pub fn recent_batch_outcomes(&self, limit: usize) -> Result<Vec<BatchOutcome>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT b.id, b.target_temperature_c, b.target_stirrer_rpm,
                   b.heating_minutes, b.stirring_minutes,
                   p.yield_percent, p.product_ratio
            FROM batches b
            JOIN product_results p ON p.batch_id = b.id
            ORDER BY b.id DESC
            LIMIT ?1
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

    pub fn recent_batches(&self, limit: usize) -> Result<Vec<Batch>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, process_id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM batches
            ORDER BY id DESC
            LIMIT ?1
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

    pub fn batch_by_id(&self, batch_id: i64) -> Result<Option<Batch>> {
        let conn = self.lock()?;
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
        let conn = self.lock()?;
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
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT batch_id, temperature_c, pressure_mpa, stirrer_rpm,
                   shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min, product_concentration_percent, ph, captured_at
            FROM sensor_samples
            WHERE batch_id = ?1
            ORDER BY id DESC
            LIMIT ?2
            "#,
        )?;
        let rows = stmt.query_map(params![batch_id, limit as i64], |row| {
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
        })?;

        let mut samples = Vec::new();
        for row in rows {
            samples.push(row?);
        }
        samples.reverse();
        Ok(samples)
    }

    pub fn recent_control_events(&self, limit: usize) -> Result<Vec<ControlEvent>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm, target_shake_speed_cpm, reason, created_at
            FROM control_events
            ORDER BY id DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
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
            })
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    pub fn control_events_for_batch(
        &self,
        batch_id: i64,
        limit: usize,
    ) -> Result<Vec<ControlEvent>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, batch_id, event_type, target_temperature_c, target_stirrer_rpm, target_shake_speed_cpm, reason, created_at
            FROM control_events
            WHERE batch_id = ?1
            ORDER BY id DESC
            LIMIT ?2
            "#,
        )?;
        let rows = stmt.query_map(params![batch_id, limit as i64], |row| {
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
            })
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    pub fn clear_runtime_data_for_tests(&self) -> Result<()> {
        let conn = self.lock()?;
        conn.execute_batch(
            r#"
            DELETE FROM ai_recommendations;
            DELETE FROM product_results;
            DELETE FROM control_events;
            DELETE FROM sensor_samples;
            DELETE FROM batches;
            DELETE FROM process_steps;
            DELETE FROM processes;
            "#,
        )?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))
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

fn touch_process(conn: &Connection, process_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE processes SET updated_at = ?1 WHERE id = ?2",
        params![Utc::now().to_rfc3339(), process_id],
    )?;
    Ok(())
}

fn parse_dt(value: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })
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
