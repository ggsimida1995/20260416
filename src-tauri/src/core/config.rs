use crate::core::models::AppSettings;
use std::path::{Path, PathBuf};

pub const SUCCESS_WORKBOOK_NAME: &str = "2026年关闭满意度回访表0331.xlsx";
pub const SUCCESS_SHEET_NAME: &str = "登记表";
const APP_STATE_DB_NAME: &str = "app_state.sqlite3";
const WORKSPACE_STATE_DB_NAME: &str = "project_compare_state.sqlite3";
const FILE_DIR_NAME: &str = "file";
const SQL_DIR_NAME: &str = "sql";
const SUCCESS_PROJECTS_DIR_NAME: &str = "success_projects";

pub fn runtime_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("ProjectFileCompare")
}

pub fn default_workspace_root() -> PathBuf {
    runtime_root()
}

pub fn workspace_file_root(workspace_root: &Path) -> PathBuf {
    workspace_root.join(FILE_DIR_NAME)
}

pub fn workspace_sql_root(workspace_root: &Path) -> PathBuf {
    workspace_root.join(SQL_DIR_NAME)
}

pub fn ensure_workspace_layout(workspace_root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(workspace_file_root(workspace_root))?;
    std::fs::create_dir_all(workspace_sql_root(workspace_root))?;
    std::fs::create_dir_all(success_projects_root(workspace_root))?;
    std::fs::create_dir_all(export_dir(workspace_root))?;
    Ok(())
}

pub fn app_state_db_path() -> PathBuf {
    migrated_sqlite_path(
        runtime_root().join(SQL_DIR_NAME).join(APP_STATE_DB_NAME),
        &[runtime_root().join(APP_STATE_DB_NAME)],
    )
}

pub fn workspace_state_db_path(workspace_root: &Path) -> PathBuf {
    migrated_sqlite_path(
        workspace_sql_root(workspace_root).join(WORKSPACE_STATE_DB_NAME),
        &[
            workspace_root.join(WORKSPACE_STATE_DB_NAME),
            workspace_file_root(workspace_root).join(WORKSPACE_STATE_DB_NAME),
        ],
    )
}

pub fn success_workbook_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join("success").join(SUCCESS_WORKBOOK_NAME)
}

pub fn success_projects_root(workspace_root: &Path) -> PathBuf {
    workspace_root.join(SUCCESS_PROJECTS_DIR_NAME)
}

pub fn export_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("export")
}

pub fn default_settings() -> AppSettings {
    AppSettings {
        last_file_root: default_workspace_root().to_string_lossy().to_string(),
        ai_enabled: true,
        ai_base_url: String::new(),
        ai_api_key: String::new(),
        ai_model: String::new(),
        ocr_base_url: String::new(),
        ocr_api_key: String::new(),
        request_timeout_seconds: 30,
        image_max_kb: 100,
        theme_mode: "light".to_string(),
    }
}

fn migrated_sqlite_path(target: PathBuf, legacy_candidates: &[PathBuf]) -> PathBuf {
    if target.exists() {
        return target;
    }
    let Some(legacy_path) = legacy_candidates.iter().find(|path| path.exists()) else {
        return target;
    };
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    copy_sqlite_family(legacy_path, &target);
    target
}

fn copy_sqlite_family(source: &Path, target: &Path) {
    let _ = std::fs::copy(source, target);
    for suffix in ["-wal", "-shm"] {
        let source_sidecar = PathBuf::from(format!("{}{}", source.to_string_lossy(), suffix));
        if source_sidecar.exists() {
            let target_sidecar = PathBuf::from(format!("{}{}", target.to_string_lossy(), suffix));
            let _ = std::fs::copy(source_sidecar, target_sidecar);
        }
    }
}
