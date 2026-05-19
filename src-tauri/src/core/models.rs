use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn default_timeout() -> i64 {
    30
}

fn default_image_max_kb() -> i64 {
    100
}

fn default_browser_kind() -> String {
    "chrome".to_string()
}

fn default_browser_profile() -> String {
    "auto".to_string()
}

fn default_browser_safe_storage_service() -> String {
    "Chrome Safe Storage".to_string()
}

fn default_theme_mode() -> String {
    "light".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default, rename = "lastFileRoot")]
    pub last_file_root: String,
    #[serde(default, rename = "aiEnabled")]
    pub ai_enabled: bool,
    #[serde(default, rename = "aiBaseUrl")]
    pub ai_base_url: String,
    #[serde(default, rename = "aiApiKey")]
    pub ai_api_key: String,
    #[serde(default, rename = "aiModel")]
    pub ai_model: String,
    #[serde(default, rename = "ocrBaseUrl")]
    pub ocr_base_url: String,
    #[serde(default, rename = "ocrApiKey")]
    pub ocr_api_key: String,
    #[serde(default = "default_timeout", rename = "requestTimeoutSeconds")]
    pub request_timeout_seconds: i64,
    #[serde(default = "default_image_max_kb", rename = "imageMaxKb")]
    pub image_max_kb: i64,
    #[serde(default = "default_browser_kind", rename = "browserKind")]
    pub browser_kind: String,
    #[serde(default, rename = "browserUserDataDir")]
    pub browser_user_data_dir: String,
    #[serde(default = "default_browser_profile", rename = "browserProfile")]
    pub browser_profile: String,
    #[serde(
        default = "default_browser_safe_storage_service",
        rename = "browserSafeStorageService"
    )]
    pub browser_safe_storage_service: String,
    #[serde(default = "default_theme_mode", rename = "themeMode")]
    pub theme_mode: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectFiles {
    pub project_name: String,
    pub project_dir: PathBuf,
    pub xlsx_path: Option<PathBuf>,
    pub docx_path: Option<PathBuf>,
    pub pdf_path: Option<PathBuf>,
    pub web_txt_path: Option<PathBuf>,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectExtraction {
    pub raw_fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocxData {
    pub project_code: String,
    pub project_name: String,
    pub contact_names: Vec<String>,
    pub contact_phones: Vec<String>,
    pub acceptance_start: Option<NaiveDate>,
    pub acceptance_end: Option<NaiveDate>,
    pub has_invalid_acceptance_range: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebData {
    pub project_code: String,
    pub project_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PdfData {
    pub project_code: String,
    pub signer_name: String,
    pub signer_phone: String,
    pub sign_date: Option<NaiveDate>,
    pub has_red_stamp: bool,
    #[serde(default)]
    pub signer_name_confidence: Option<f64>,
    #[serde(default)]
    pub signer_phone_confidence: Option<f64>,
    #[serde(default)]
    pub sign_date_confidence: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct PdfRecognitionContext {
    pub candidate_names: Vec<String>,
    pub candidate_phones: Vec<String>,
    pub excel_acceptance_date: Option<NaiveDate>,
    pub acceptance_start: Option<NaiveDate>,
    pub acceptance_end: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareFailure {
    pub field_name: String,
    pub message: String,
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct CompareResult {
    pub passed: bool,
    pub failures: Vec<CompareFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectErrorDetail {
    pub project_code: String,
    pub field_name: String,
    pub message: String,
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub mode: String,
    pub updated_at: String,
    pub pending_success_count: usize,
    pub failed_count: usize,
    pub project_count: usize,
    pub downloaded_project_names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCompareLog {
    pub project_name: String,
    pub project_code: String,
    pub passed: bool,
    pub summary: String,
    pub finished_at: String,
    pub rows: Vec<ProjectCompareLogRow>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCompareLogRow {
    pub file_name: String,
    pub project_code: String,
    pub project_name: String,
    pub contact_name: String,
    pub contact_phone: String,
    pub acceptance_time: String,
    pub start_time: String,
    #[serde(default)]
    pub amount: String,
    #[serde(default)]
    pub has_red_stamp: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowProgress {
    pub task_id: String,
    pub stage: String,
    pub status: String,
    pub current: usize,
    pub total: usize,
    pub percent: u8,
    pub message: String,
    pub project_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_log: Option<ProjectCompareLog>,
}
