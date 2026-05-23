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
                name TEXT NOT NULL,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                target_temperature_c REAL NOT NULL,
                target_stirrer_rpm REAL NOT NULL,
                heating_minutes REAL NOT NULL,
                stirring_minutes REAL NOT NULL
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
        let now = Utc::now();
        let conn = self.lock()?;
        conn.execute(
            r#"
            INSERT INTO batches
                (name, started_at, target_temperature_c, target_stirrer_rpm, heating_minutes, stirring_minutes)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
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
            name: name.to_string(),
            started_at: now,
            finished_at: None,
            target_temperature_c,
            target_stirrer_rpm,
            heating_minutes,
            stirring_minutes,
        })
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
            SELECT id, name, started_at, finished_at, target_temperature_c,
                   target_stirrer_rpm, heating_minutes, stirring_minutes
            FROM batches
            ORDER BY id DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let started_at: String = row.get(2)?;
            let finished_at: Option<String> = row.get(3)?;
            Ok(Batch {
                id: row.get(0)?,
                name: row.get(1)?,
                started_at: parse_dt(&started_at)?,
                finished_at: match finished_at {
                    Some(value) => Some(parse_dt(&value)?),
                    None => None,
                },
                target_temperature_c: row.get(4)?,
                target_stirrer_rpm: row.get(5)?,
                heating_minutes: row.get(6)?,
                stirring_minutes: row.get(7)?,
            })
        })?;

        let mut batches = Vec::new();
        for row in rows {
            batches.push(row?);
        }
        Ok(batches)
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

    pub fn clear_runtime_data_for_tests(&self) -> Result<()> {
        let conn = self.lock()?;
        conn.execute_batch(
            r#"
            DELETE FROM ai_recommendations;
            DELETE FROM product_results;
            DELETE FROM control_events;
            DELETE FROM sensor_samples;
            DELETE FROM batches;
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
