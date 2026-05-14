use crate::core::models::{AppSettings, ProjectErrorDetail};
use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessRecord {
    pub project_code: String,
    pub project_name: String,
    pub row_data: BTreeMap<String, Value>,
}

pub struct AppStateStore {
    path: PathBuf,
}

impl AppStateStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load_settings(&self) -> Result<Option<AppSettings>> {
        let connection = self.connect()?;
        let mut statement =
            connection.prepare("SELECT value FROM app_settings WHERE key = 'settings'")?;
        let mut rows = statement.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let value: String = row.get(0)?;
        Ok(serde_json::from_str(&value).ok())
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        let connection = self.connect()?;
        connection.execute(
            "INSERT INTO app_settings(key, value, updated_at)
             VALUES ('settings', ?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![serde_json::to_string(settings)?, timestamp()],
        )?;
        Ok(())
    }

    pub fn append_runtime_log(&self, message: &str) -> Result<()> {
        let connection = self.connect()?;
        connection.execute(
            "INSERT INTO runtime_logs(bucket, message, created_at) VALUES ('default', ?1, ?2)",
            params![message, timestamp()],
        )?;
        Ok(())
    }

    pub fn latest_runtime_logs(&self, limit: i64) -> Result<Vec<String>> {
        let connection = self.connect()?;
        let mut statement =
            connection.prepare("SELECT message FROM runtime_logs ORDER BY id DESC LIMIT ?1")?;
        let rows = statement.query_map(params![limit], |row| row.get::<_, String>(0))?;
        let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        items.reverse();
        Ok(items)
    }

    pub fn clear_runtime_logs(&self) -> Result<()> {
        let connection = self.connect()?;
        connection.execute("DELETE FROM runtime_logs", [])?;
        Ok(())
    }

    pub fn append_result_logs(
        &self,
        success_project_codes: &[String],
        error_details: &[ProjectErrorDetail],
        success_records: &[SuccessRecord],
    ) -> Result<()> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let now = timestamp();

        for code in success_project_codes {
            tx.execute(
                "INSERT INTO result_logs(kind, rendered, project_code, field_name, message, values_json, created_at)
                 VALUES ('success', ?1, ?2, '', '', '{}', ?3)",
                params![format!("{now} | {code}"), code, now],
            )?;
        }

        for detail in error_details {
            tx.execute(
                "INSERT INTO result_logs(kind, rendered, project_code, field_name, message, values_json, created_at)
                 VALUES ('error', ?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    format_error_line(&now, detail),
                    detail.project_code,
                    detail.field_name,
                    detail.message,
                    serde_json::to_string(&detail.values)?,
                    now
                ],
            )?;
        }

        for record in success_records {
            tx.execute(
                "INSERT INTO success_records(project_code, project_name, row_json, status, created_at, updated_at, exported_at)
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5, NULL)
                 ON CONFLICT(project_code) DO UPDATE SET
                   project_name = excluded.project_name,
                   row_json = excluded.row_json,
                   status = 'pending',
                   updated_at = excluded.updated_at,
                   exported_at = NULL",
                params![
                    record.project_code,
                    record.project_name,
                    serde_json::to_string(&record.row_data)?,
                    now,
                    now
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn latest_result_logs(&self, kind: &str, limit: i64) -> Result<Vec<String>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT rendered FROM result_logs WHERE kind = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![kind, limit], |row| row.get::<_, String>(0))?;
        let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        items.reverse();
        Ok(items)
    }

    pub fn count_result_logs(&self, kind: &str) -> Result<usize> {
        let connection = self.connect()?;
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM result_logs WHERE kind = ?1",
            params![kind],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn pending_success_records(&self) -> Result<Vec<SuccessRecord>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT project_code, project_name, row_json
             FROM success_records
             WHERE status = 'pending'
             ORDER BY id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            let row_json: String = row.get(2)?;
            let row_data = serde_json::from_str(&row_json).unwrap_or_default();
            Ok(SuccessRecord {
                project_code: row.get(0)?,
                project_name: row.get(1)?,
                row_data,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn count_pending_success_records(&self) -> Result<usize> {
        let connection = self.connect()?;
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM success_records WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn mark_success_records_exported(&self, project_codes: &[String]) -> Result<()> {
        if project_codes.is_empty() {
            return Ok(());
        }
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let now = timestamp();
        for code in project_codes {
            tx.execute(
                "UPDATE success_records
                 SET status = 'exported', updated_at = ?1, exported_at = ?2
                 WHERE project_code = ?3",
                params![now, now, code],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn connect(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建 SQLite 目录: {}", parent.display()))?;
        }
        let connection = Connection::open(&self.path)
            .with_context(|| format!("无法打开 SQLite: {}", self.path.display()))?;
        connection.busy_timeout(Duration::from_secs(30))?;
        connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS app_settings (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS runtime_logs (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              bucket TEXT NOT NULL,
              message TEXT NOT NULL,
              created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS result_logs (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              kind TEXT NOT NULL,
              rendered TEXT NOT NULL,
              project_code TEXT NOT NULL,
              field_name TEXT NOT NULL,
              message TEXT NOT NULL,
              values_json TEXT NOT NULL,
              created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS success_records (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              project_code TEXT NOT NULL UNIQUE,
              project_name TEXT NOT NULL,
              row_json TEXT NOT NULL,
              status TEXT NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              exported_at TEXT
            );
            ",
        )?;
        Ok(connection)
    }
}

fn format_error_line(timestamp: &str, detail: &ProjectErrorDetail) -> String {
    let values = if detail.values.is_empty() {
        String::new()
    } else {
        let rendered = detail
            .values
            .iter()
            .map(|(key, value)| format!("{key}={}", value_to_string(value)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(" | {rendered}")
    };
    format!(
        "{} | {} | {} | {}{}",
        timestamp, detail.project_code, detail.field_name, detail.message, values
    )
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub fn timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
