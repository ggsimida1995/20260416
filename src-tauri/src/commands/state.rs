use crate::core::config::{
    app_state_db_path, default_settings, ensure_workspace_layout, workspace_file_root,
    workspace_state_db_path,
};
use crate::core::discovery::project_dir_names;
use crate::core::download::{check_session_status, unchecked_session_status, SessionStatus};
use crate::core::models::{AppSettings, WorkflowSummary};
use crate::core::secret_store;
use crate::core::workflow::summary;
use crate::db::app_state::AppStateStore;
use serde::Serialize;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub window_title: String,
    pub settings: AppSettings,
    pub session: SessionStatus,
    pub logs: Vec<String>,
    pub outputs: WorkflowSummary,
}

#[tauri::command]
pub fn bootstrap() -> Result<AppState, String> {
    build_state().map_err(to_string)
}

#[tauri::command]
pub fn save_settings(payload: AppSettings) -> Result<AppState, String> {
    let mut settings = normalize_settings(payload);
    secret_store::save_password(&settings.password);
    settings.password.clear();
    AppStateStore::new(app_state_db_path())
        .save_settings(&settings)
        .map_err(to_string)?;
    build_state().map_err(to_string)
}

#[tauri::command]
pub fn open_file_root() -> Result<bool, String> {
    let settings = load_settings().map_err(to_string)?;
    open::that(settings.last_file_root).map_err(to_string)?;
    Ok(true)
}

#[tauri::command]
pub fn open_path(path: String) -> Result<bool, String> {
    if path.is_empty() {
        return Ok(false);
    }
    open::that(path).map_err(to_string)?;
    Ok(true)
}

#[tauri::command]
pub async fn choose_file_root(app: AppHandle) -> Result<Option<String>, String> {
    let selected = app
        .dialog()
        .file()
        .blocking_pick_folder()
        .map(|path| path.to_string());
    Ok(selected)
}

#[tauri::command]
pub async fn check_session() -> Result<SessionStatus, String> {
    let settings = load_settings().map_err(to_string)?;
    let status = tauri::async_runtime::spawn_blocking(move || check_session_status(&settings))
        .await
        .map_err(to_string)?;
    Ok(status)
}

pub fn build_state() -> anyhow::Result<AppState> {
    let settings = load_settings()?;
    let workspace_root = PathBuf::from(&settings.last_file_root);
    ensure_workspace_layout(&workspace_root)?;
    let file_root = workspace_file_root(&workspace_root);
    let runtime_store = AppStateStore::new(workspace_state_db_path(&workspace_root));
    let logs = runtime_store.latest_runtime_logs(500).unwrap_or_default();
    let outputs = summary(&workspace_root, "startup").unwrap_or_else(|_| WorkflowSummary {
        mode: String::new(),
        updated_at: String::new(),
        pending_success_count: 0,
        failed_count: 0,
        project_count: project_dir_names(&file_root).len(),
        downloaded_project_names: project_dir_names(&file_root),
    });
    Ok(AppState {
        window_title: "项目资料比对助手".to_string(),
        session: unchecked_session_status(&settings),
        settings,
        logs,
        outputs,
    })
}

pub fn load_settings() -> anyhow::Result<AppSettings> {
    let store = AppStateStore::new(app_state_db_path());
    let raw_settings = store.load_settings()?.unwrap_or_else(default_settings);
    let mut settings = normalize_settings(raw_settings.clone());
    let legacy_password = std::mem::take(&mut settings.password);
    if !legacy_password.is_empty() {
        secret_store::save_password(&legacy_password);
    }
    store.save_settings(&settings)?;
    settings.password = secret_store::load_password().unwrap_or_default();
    Ok(settings)
}

fn normalize_settings(mut settings: AppSettings) -> AppSettings {
    settings.last_file_root = normalize_file_root_path(&settings.last_file_root);
    if settings.last_file_root.is_empty() {
        settings.last_file_root = default_settings().last_file_root;
    }
    settings.theme_mode = normalize_theme_mode(&settings.theme_mode);
    if settings.request_timeout_seconds < 1 {
        settings.request_timeout_seconds = 30;
    }
    if settings.image_max_kb < 20 {
        settings.image_max_kb = 100;
    }
    settings.account = settings.account.trim().to_string();
    settings
}

fn normalize_theme_mode(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "system" | "auto" | "跟随系统" => "system".to_string(),
        "dark" | "night" | "夜间" => "dark".to_string(),
        _ => "light".to_string(),
    }
}

fn normalize_file_root_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    PathBuf::from(trimmed).to_string_lossy().to_string()
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
