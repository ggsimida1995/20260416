use crate::core::models::AppSettings;
use std::path::{Path, PathBuf};

pub const SUCCESS_WORKBOOK_NAME: &str = "2026年关闭满意度回访表0331.xlsx";
pub const SUCCESS_SHEET_NAME: &str = "登记表";

pub fn runtime_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("ProjectFileCompare")
}

pub fn default_file_root() -> PathBuf {
    runtime_root().join("file")
}

pub fn app_state_db_path() -> PathBuf {
    runtime_root().join("app_state.sqlite3")
}

pub fn workspace_state_db_path(file_root: &Path) -> PathBuf {
    file_root.join("project_compare_state.sqlite3")
}

pub fn success_workbook_path(file_root: &Path) -> PathBuf {
    file_root.join("success").join(SUCCESS_WORKBOOK_NAME)
}

pub fn export_dir(file_root: &Path) -> PathBuf {
    file_root.join("export")
}

pub fn default_settings() -> AppSettings {
    AppSettings {
        last_file_root: default_file_root().to_string_lossy().to_string(),
        ai_enabled: false,
        ai_base_url: String::new(),
        ai_api_key: String::new(),
        ai_model: String::new(),
        ocr_base_url: String::new(),
        ocr_api_key: String::new(),
        request_timeout_seconds: 30,
        image_max_kb: 100,
        browser_kind: "chrome".to_string(),
        browser_user_data_dir: String::new(),
        browser_profile: "auto".to_string(),
        browser_safe_storage_service: "Chrome Safe Storage".to_string(),
        theme_mode: "light".to_string(),
    }
}
