use crate::core::models::{AppSettings, PdfData};
use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessRecord {
    pub project_code: String,
    pub project_name: String,
    pub row_data: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieEntry {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
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

    pub fn append_success_record(&self, record: &SuccessRecord) -> Result<()> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let now = timestamp();
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
        tx.commit()?;
        Ok(())
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

    pub fn load_pdf_recognition_cache(
        &self,
        file_path: &Path,
        fingerprint: &str,
    ) -> Result<Option<PdfData>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT data_json FROM pdf_recognition_cache WHERE file_path = ?1 AND fingerprint = ?2",
        )?;
        let mut rows = statement.query(params![file_path.to_string_lossy(), fingerprint])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let data_json: String = row.get(0)?;
        Ok(serde_json::from_str(&data_json).ok())
    }

    pub fn save_cookies(&self, cookies: &[CookieEntry]) -> Result<()> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let now = timestamp();
        for cookie in cookies {
            tx.execute(
                "INSERT INTO hollysys_cookies(name, domain, path, value, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(name, domain, path) DO UPDATE SET
                   value = excluded.value,
                   updated_at = excluded.updated_at",
                params![cookie.name, cookie.domain, cookie.path, cookie.value, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_cookies(&self) -> Result<Vec<CookieEntry>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT name, value, domain, path FROM hollysys_cookies ORDER BY name ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(CookieEntry {
                name: row.get(0)?,
                value: row.get(1)?,
                domain: row.get(2)?,
                path: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn clear_cookies(&self) -> Result<()> {
        let connection = self.connect()?;
        connection.execute("DELETE FROM hollysys_cookies", [])?;
        Ok(())
    }

    pub fn save_pdf_recognition_cache(
        &self,
        file_path: &Path,
        fingerprint: &str,
        data: &PdfData,
    ) -> Result<()> {
        let connection = self.connect()?;
        let now = timestamp();
        connection.execute(
            "INSERT INTO pdf_recognition_cache(file_path, fingerprint, data_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(file_path) DO UPDATE SET
               fingerprint = excluded.fingerprint,
               data_json = excluded.data_json,
               updated_at = excluded.updated_at",
            params![
                file_path.to_string_lossy(),
                fingerprint,
                serde_json::to_string(data)?,
                now
            ],
        )?;
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
            CREATE TABLE IF NOT EXISTS pdf_recognition_cache (
              file_path TEXT PRIMARY KEY,
              fingerprint TEXT NOT NULL,
              data_json TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS hollysys_cookies (
              name TEXT NOT NULL,
              domain TEXT NOT NULL,
              path TEXT NOT NULL,
              value TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              PRIMARY KEY (name, domain, path)
            );
            ",
        )?;
        Ok(connection)
    }
}

pub fn timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
