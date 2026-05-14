use crate::core::models::AppSettings;
use aes::Aes128;
use anyhow::{anyhow, bail, Context, Result};
#[cfg(windows)]
use base64::Engine;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, REFERER, USER_AGENT};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

const BASE_URL: &str = "https://www.hollysys.net";
const BASE_HOST: &str = "www.hollysys.net";
const COOKIE_SALT: &[u8] = b"saltysalt";
const COOKIE_IV: &[u8; 16] = b"                ";
const COOKIE_ITERATIONS: u32 = 1003;
const AGGREGATION_CATEGORIES: [(&str, &str); 2] = [
    ("18a032b3695468f23f38a0f40d5a3602", "项目关闭工作流"),
    (
        "18a032b4e48b3ad71bf4c08405487452",
        "项目关闭工作流(工软分包项目)",
    ),
];
const IDENTITY_PATHS: [&str; 6] = [
    "/",
    "/index.jsp",
    "/ekp/index.jsp",
    "/sys/portal/page.jsp",
    "/sys/portal/sys_portal_page/sysPortalPage.do",
    "/sys/portal/sys_portal_page/sysPortalPage.do?method=index",
];
static PROFILE_NAME_RE: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Clone)]
struct CookieRow {
    host_key: String,
    name: String,
    encrypted_value: Vec<u8>,
}

#[derive(Debug, Clone)]
struct TodoItem {
    category_name: String,
    todo_fd_id: String,
    subject: String,
    detail_path: String,
}

#[derive(Debug, Clone)]
struct DetailRecord {
    item: TodoItem,
    project_code: String,
    project_name: String,
    attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Attachment {
    name: String,
    fd_id: String,
    mime_type: String,
    size: String,
    file_key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSummary {
    pub processed_count: usize,
    pub saved_project_dirs: Vec<String>,
    pub skipped_projects: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BrowserConfig {
    kind: BrowserKind,
    user_data_dir: PathBuf,
    profile: String,
    safe_storage_service: String,
    request_timeout_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserKind {
    Chrome,
    Edge,
    Chromium,
    Custom,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub state: String,
    pub message: String,
    pub browser_name: String,
    pub user_data_dir: String,
    pub profile: String,
    pub cookie_db: String,
    pub account: String,
    pub display_name: String,
    pub checked_at: String,
}

pub fn run_download(
    file_root: &Path,
    skip_project_codes: &HashSet<String>,
    settings: &AppSettings,
) -> Result<DownloadSummary> {
    std::fs::create_dir_all(file_root)?;
    let browser = BrowserConfig::from_settings(settings);
    let client = build_authenticated_client(&browser)?;
    let mut summary = DownloadSummary {
        processed_count: 0,
        saved_project_dirs: Vec::new(),
        skipped_projects: Vec::new(),
        errors: Vec::new(),
    };

    for (category_id, category_name) in AGGREGATION_CATEGORIES {
        let items = fetch_todo_items(&client, category_id, category_name)?;
        for item in items {
            match fetch_detail_record(&client, item.clone()) {
                Ok(record) => {
                    let normalized_code = normalize_project_code(&record.project_code);
                    if skip_project_codes.contains(&normalized_code) {
                        summary.skipped_projects.push(normalized_code);
                        continue;
                    }
                    match save_record(&client, &record, file_root) {
                        Ok(path) => {
                            summary.processed_count += 1;
                            summary
                                .saved_project_dirs
                                .push(path.to_string_lossy().to_string());
                        }
                        Err(error) => summary
                            .errors
                            .push(format!("{} | {}", record.project_code, error)),
                    }
                }
                Err(error) => summary
                    .errors
                    .push(format!("{} | {}", item.detail_url(), error)),
            }
        }
    }
    Ok(summary)
}

pub fn check_session_status(settings: &AppSettings) -> SessionStatus {
    let browser = BrowserConfig::from_settings(settings);
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut status = SessionStatus {
        state: "checking".to_string(),
        message: "正在检测会话".to_string(),
        browser_name: browser.display_name().to_string(),
        user_data_dir: browser.user_data_dir.to_string_lossy().to_string(),
        profile: browser.profile.clone(),
        cookie_db: String::new(),
        account: String::new(),
        display_name: String::new(),
        checked_at: now,
    };

    let cookie_db = match resolve_cookie_db(&browser) {
        Ok(path) => path,
        Err(_) => {
            mark_not_logged_in(&mut status);
            return status;
        }
    };
    status.cookie_db = cookie_db.to_string_lossy().to_string();

    let client = match build_authenticated_client_with_cookie_db(&browser, &cookie_db) {
        Ok(client) => client,
        Err(_) => {
            mark_not_logged_in(&mut status);
            return status;
        }
    };

    match verify_session(&client) {
        Ok(identity) => {
            status.state = "ok".to_string();
            status.message = "会话可用".to_string();
            status.account = identity.account;
            status.display_name = identity.display_name;
        }
        Err(_) => {
            mark_not_logged_in(&mut status);
        }
    }
    status
}

fn mark_not_logged_in(status: &mut SessionStatus) {
    status.state = "missing".to_string();
    status.message = "未登录".to_string();
    status.account.clear();
    status.display_name.clear();
}

pub fn unchecked_session_status(settings: &AppSettings) -> SessionStatus {
    let browser = BrowserConfig::from_settings(settings);
    SessionStatus {
        state: "unknown".to_string(),
        message: "未检测浏览器会话".to_string(),
        browser_name: browser.display_name().to_string(),
        user_data_dir: browser.user_data_dir.to_string_lossy().to_string(),
        profile: browser.profile.clone(),
        cookie_db: String::new(),
        account: String::new(),
        display_name: String::new(),
        checked_at: String::new(),
    }
}

fn build_authenticated_client(browser: &BrowserConfig) -> Result<Client> {
    let cookie_db = resolve_cookie_db(browser)?;
    build_authenticated_client_with_cookie_db(browser, &cookie_db)
}

fn build_authenticated_client_with_cookie_db(
    browser: &BrowserConfig,
    cookie_db: &Path,
) -> Result<Client> {
    let rows = read_hollysys_cookie_rows(&cookie_db)?;
    if rows.is_empty() {
        bail!(
            "已找到 {} Cookie 数据库，但未发现 Hollysys Cookie: {}",
            browser.display_name(),
            cookie_db.display()
        );
    }
    let key = read_browser_cookie_key(browser)?;
    let mut cookie_pairs = Vec::new();
    let mut seen_cookie_names = HashSet::new();
    let mut applicable_count = 0usize;
    for row in rows {
        if !cookie_host_matches(&row.host_key, BASE_HOST) {
            continue;
        }
        applicable_count += 1;
        if !seen_cookie_names.insert(row.name.clone()) {
            continue;
        }
        match decrypt_browser_cookie(&row.host_key, &row.encrypted_value, &key, browser) {
            Ok(value) if !value.is_empty() => cookie_pairs.push(format!("{}={}", row.name, value)),
            Ok(_) => {}
            Err(error) => {
                if is_required_cookie(&row.name) {
                    return Err(error)
                        .with_context(|| format!("关键 Cookie 解密失败: {}", row.name));
                }
            }
        }
    }
    if applicable_count == 0 {
        bail!(
            "已找到 {} Cookie 数据库，但未发现适用于 {} 的 Hollysys Cookie: {}",
            browser.display_name(),
            BASE_HOST,
            cookie_db.display()
        );
    }
    if cookie_pairs.is_empty() {
        bail!(
            "Hollysys Cookie 全部解密失败，请确认 {} 已登录并允许访问钥匙串",
            browser.display_name()
        );
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36"),
    );
    headers.insert(COOKIE, HeaderValue::from_str(&cookie_pairs.join("; "))?);
    Ok(Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(browser.request_timeout_seconds))
        .build()?)
}

fn cookie_host_matches(cookie_host: &str, request_host: &str) -> bool {
    if let Some(domain) = cookie_host.strip_prefix('.') {
        request_host == domain || request_host.ends_with(&format!(".{domain}"))
    } else {
        cookie_host == request_host
    }
}

fn verify_session(client: &Client) -> Result<SessionIdentity> {
    let mut last_error = None;
    for attempt in 0..2 {
        for (category_id, _) in AGGREGATION_CATEGORIES {
            match verify_session_once(client, category_id) {
                Ok(identity) => return Ok(identity),
                Err(error) => last_error = Some(error),
            }
        }
        if attempt == 0 {
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("会话检测失败")))
}

fn verify_session_once(client: &Client, category_id: &str) -> Result<SessionIdentity> {
    let response = client
        .get(build_list_url(category_id))
        .send()?
        .error_for_status()?
        .text()?;
    if looks_like_login_page(&response) {
        bail!("Hollysys 会话已失效，请先在浏览器中重新登录");
    }
    let payload: Value = match serde_json::from_str(response.trim_start()) {
        Ok(payload) => payload,
        Err(error) => {
            if let Some(identity) = fetch_identity_from_pages(client) {
                return Ok(identity);
            }
            return Err(error).context("会话检测接口返回不是 JSON，已读取到 Cookie，但接口响应异常");
        }
    };
    if json_contains_expired_signal(&payload) {
        bail!("Hollysys 会话已失效，请先在浏览器中重新登录");
    }
    let mut identity = extract_identity_from_value(&payload).unwrap_or_default();
    if identity.account.is_empty() || identity.display_name.is_empty() {
        identity.merge(fetch_identity_from_pages(client));
    }
    Ok(identity)
}

#[derive(Debug, Clone, Default)]
struct SessionIdentity {
    account: String,
    display_name: String,
}

impl SessionIdentity {
    fn merge(&mut self, other: Option<SessionIdentity>) {
        let Some(other) = other else {
            return;
        };
        if self.account.is_empty() {
            self.account = other.account;
        }
        if self.display_name.is_empty() {
            self.display_name = other.display_name;
        }
    }
}

impl BrowserConfig {
    pub fn from_settings(settings: &AppSettings) -> Self {
        let kind = BrowserKind::from_setting(&settings.browser_kind);
        let configured_dir = settings.browser_user_data_dir.trim();
        let user_data_dir = if configured_dir.is_empty() {
            default_browser_user_data_dir(kind)
        } else {
            PathBuf::from(configured_dir)
        };
        let profile = if settings.browser_profile.trim().is_empty() {
            "auto".to_string()
        } else {
            settings.browser_profile.trim().to_string()
        };
        let safe_storage_service =
            resolve_safe_storage_service(kind, &settings.browser_safe_storage_service);
        Self {
            kind,
            user_data_dir,
            profile,
            safe_storage_service,
            request_timeout_seconds: settings.request_timeout_seconds.max(1) as u64,
        }
    }

    fn display_name(&self) -> &'static str {
        self.kind.display_name()
    }

    fn safe_storage_service(&self) -> &str {
        &self.safe_storage_service
    }

    #[cfg(windows)]
    fn local_state_path(&self) -> PathBuf {
        self.user_data_dir.join("Local State")
    }
}

impl BrowserKind {
    fn from_setting(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "edge" | "microsoft_edge" | "microsoft-edge" => Self::Edge,
            "chromium" => Self::Chromium,
            "custom" | "custom_chromium" | "custom-chromium" => Self::Custom,
            _ => Self::Chrome,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Chromium => "Chromium",
            Self::Custom => "自定义 Chromium",
        }
    }
}

fn default_browser_user_data_dir(kind: BrowserKind) -> PathBuf {
    if cfg!(windows) {
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("AppData")
                    .join("Local")
            });
        return match kind {
            BrowserKind::Chrome => base.join("Google").join("Chrome").join("User Data"),
            BrowserKind::Edge => base.join("Microsoft").join("Edge").join("User Data"),
            BrowserKind::Chromium => base.join("Chromium").join("User Data"),
            BrowserKind::Custom => base.join("Chromium").join("User Data"),
        };
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    match kind {
        BrowserKind::Chrome => home.join("Library/Application Support/Google/Chrome"),
        BrowserKind::Edge => home.join("Library/Application Support/Microsoft Edge"),
        BrowserKind::Chromium => home.join("Library/Application Support/Chromium"),
        BrowserKind::Custom => home.join("Library/Application Support/Chromium"),
    }
}

fn resolve_safe_storage_service(kind: BrowserKind, configured: &str) -> String {
    let trimmed = configured.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    match kind {
        BrowserKind::Chrome => "Chrome Safe Storage",
        BrowserKind::Edge => "Microsoft Edge Safe Storage",
        BrowserKind::Chromium | BrowserKind::Custom => "Chromium Safe Storage",
    }
    .to_string()
}

fn is_auto_profile(profile: &str) -> bool {
    let value = profile.trim();
    value.is_empty() || value.eq_ignore_ascii_case("auto") || value == "自动"
}

fn push_unique_path(items: &mut Vec<PathBuf>, item: PathBuf) {
    if !items.contains(&item) {
        items.push(item);
    }
}

fn fetch_todo_items(
    client: &Client,
    category_id: &str,
    category_name: &str,
) -> Result<Vec<TodoItem>> {
    let response = client
        .get(build_list_url(category_id))
        .send()?
        .error_for_status()?
        .text()?;
    if looks_like_login_page(&response) {
        bail!("Hollysys 会话已失效，请先在浏览器中重新登录");
    }
    let payload: Value = serde_json::from_str(response.trim_start())
        .with_context(|| format!("待办列表返回不是 JSON，分类: {category_name}"))?;
    let mut items = Vec::new();
    let Some(rows) = payload.get("datas").and_then(Value::as_array) else {
        return Ok(items);
    };
    for raw_row in rows {
        let row = list_row_to_map(raw_row);
        let detail_path = row
            .get("tr_href")
            .or_else(|| row.get("_tr_href"))
            .cloned()
            .unwrap_or_default();
        if detail_path.is_empty() {
            continue;
        }
        let subject = strip_html(
            row.get("todo.subject4View")
                .map(String::as_str)
                .unwrap_or(""),
        );
        items.push(TodoItem {
            category_name: category_name.to_string(),
            todo_fd_id: row.get("fdId").cloned().unwrap_or_default(),
            subject,
            detail_path,
        });
    }
    Ok(items)
}

fn fetch_detail_record(client: &Client, item: TodoItem) -> Result<DetailRecord> {
    let html = client
        .get(item.detail_url())
        .send()?
        .error_for_status()?
        .text()?;
    if looks_like_login_page(&html) {
        bail!("Hollysys 会话已失效，请先在浏览器中重新登录");
    }
    let project_code = extract_detail_field(&html, "项目编号")
        .or_else(|| extract_project_code(&item.subject))
        .ok_or_else(|| anyhow!("详情页未解析到项目编号"))?;
    let project_name =
        extract_detail_field(&html, "项目名称").ok_or_else(|| anyhow!("详情页未解析到项目名称"))?;
    let attachments = extract_section_attachments(&html, "关闭依据附件");
    Ok(DetailRecord {
        item,
        project_code,
        project_name,
        attachments,
    })
}

fn save_record(client: &Client, record: &DetailRecord, output_root: &Path) -> Result<PathBuf> {
    let project_dir = output_root.join(normalize_project_code(&record.project_code));
    std::fs::create_dir_all(&project_dir)?;
    for attachment in select_target_attachments(&record.attachments) {
        let destination = project_dir.join(sanitize_filename(&attachment.name));
        download_attachment(client, attachment, &destination, &record.item.detail_url())?;
    }
    let info_path = project_dir.join(format!(
        "{}.txt",
        project_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ));
    std::fs::write(
        info_path,
        format!(
            "项目编号: {}\n项目名称: {}\n来源分类: {}\n详情页: {}\n待办页: {}\n",
            record.project_code,
            record.project_name,
            record.item.category_name,
            record.item.detail_url(),
            record.item.notify_view_url()
        ),
    )?;
    Ok(project_dir)
}

fn download_attachment(
    client: &Client,
    attachment: &Attachment,
    destination: &Path,
    referer: &str,
) -> Result<()> {
    let content = client
        .get(attachment.download_url())
        .header(REFERER, referer)
        .send()?
        .error_for_status()?
        .bytes()?;
    if looks_like_html(&content) {
        bail!(
            "附件下载返回 HTML，疑似会话失效: {}",
            attachment.download_url()
        );
    }
    write_bytes_atomically(destination, &content)
}

fn write_bytes_atomically(destination: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_path = destination.with_file_name(format!(
        ".{}.{}.part",
        destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        std::process::id()
    ));
    std::fs::write(&temp_path, content)?;
    if destination.exists() {
        std::fs::remove_file(destination)?;
    }
    std::fs::rename(&temp_path, destination).or_else(|error| {
        let _ = std::fs::remove_file(&temp_path);
        Err(error)
    })?;
    Ok(())
}

fn select_target_attachments(attachments: &[Attachment]) -> Vec<&Attachment> {
    let remaining = attachments
        .iter()
        .filter(|item| !is_message_attachment(item))
        .collect::<Vec<_>>();
    let mut selected = Vec::new();
    pick_best(
        &remaining,
        &mut selected,
        &["项目关闭移交登记表"],
        &[".xlsx", ".xls"],
    );
    pick_best(
        &remaining,
        &mut selected,
        &["项目竣工总结报告"],
        &[".docx", ".doc"],
    );
    pick_best(
        &remaining,
        &mut selected,
        &["验收报告", "开箱验收单", "开箱验收"],
        &[".pdf", ".jpg", ".jpeg", ".png"],
    );
    let mut seen = selected
        .iter()
        .map(|item| item.fd_id.clone())
        .collect::<HashSet<_>>();
    for attachment in remaining {
        if selected.len() >= 3 {
            break;
        }
        if seen.insert(attachment.fd_id.clone()) {
            selected.push(attachment);
        }
    }
    selected.truncate(3);
    selected
}

fn pick_best<'a>(
    attachments: &[&'a Attachment],
    selected: &mut Vec<&'a Attachment>,
    keywords: &[&str],
    suffixes: &[&str],
) {
    if let Some(found) = attachments
        .iter()
        .copied()
        .find(|item| attachment_matches(item, keywords, suffixes))
    {
        selected.push(found);
    }
}

fn attachment_matches(attachment: &Attachment, keywords: &[&str], suffixes: &[&str]) -> bool {
    let name = attachment.name.to_lowercase();
    keywords
        .iter()
        .any(|keyword| name.contains(&keyword.to_lowercase()))
        && suffixes.iter().any(|suffix| name.ends_with(suffix))
}

fn is_message_attachment(attachment: &Attachment) -> bool {
    let lower = attachment.name.to_lowercase();
    lower.ends_with(".eml") || lower.ends_with(".msg") || attachment.mime_type == "message/rfc822"
}

fn extract_detail_field(html: &str, label: &str) -> Option<String> {
    let label_pos = html
        .find(&format!("<label>{label}</label>"))
        .or_else(|| html.find(&format!("<label>{label}</label>\r")))
        .or_else(|| html.find(label))?;
    let tail = &html[label_pos..];
    let td_start = tail.find("</td>")?;
    let value_tail = &tail[td_start + 5..];
    let td_end = value_tail.find("</td>").unwrap_or(value_tail.len());
    let cell = &value_tail[..td_end];
    let text = strip_html(cell);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn extract_section_attachments(html: &str, section_label: &str) -> Vec<Attachment> {
    let Some(label_pos) = html.find(section_label) else {
        return Vec::new();
    };
    let section = &html[label_pos..html.len().min(label_pos + 30_000)];
    let Ok(pattern) = Regex::new(
        r#"addDoc\("((?:\\.|[^"\\])*)","([0-9a-f]+)",(?:true|false),"((?:\\.|[^"\\])*)","((?:\\.|[^"\\])*)","((?:\\.|[^"\\])*)","((?:\\.|[^"\\])*)"\s*\);"#,
    ) else {
        return Vec::new();
    };
    let mut attachments = Vec::new();
    let mut seen = HashSet::new();
    for captures in pattern.captures_iter(section) {
        let fd_id = captures
            .get(2)
            .map(|item| item.as_str())
            .unwrap_or("")
            .to_string();
        if !seen.insert(fd_id.clone()) {
            continue;
        }
        attachments.push(Attachment {
            name: decode_js_string(captures.get(1).map(|item| item.as_str()).unwrap_or("")),
            fd_id,
            mime_type: decode_js_string(captures.get(3).map(|item| item.as_str()).unwrap_or("")),
            size: decode_js_string(captures.get(4).map(|item| item.as_str()).unwrap_or("")),
            file_key: decode_js_string(captures.get(5).map(|item| item.as_str()).unwrap_or("")),
        });
    }
    attachments
}

fn list_row_to_map(raw_row: &Value) -> BTreeMap<String, String> {
    let mut row = BTreeMap::new();
    let Some(entries) = raw_row.as_array() else {
        return row;
    };
    for entry in entries {
        let Some(column) = entry.get("col").and_then(Value::as_str) else {
            continue;
        };
        let value = entry
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        row.insert(column.to_string(), value);
    }
    row
}

fn extract_identity_from_value(value: &Value) -> Option<SessionIdentity> {
    let mut identity = SessionIdentity::default();
    find_identity_fields(value, &mut identity);
    if identity.account.is_empty() && identity.display_name.is_empty() {
        None
    } else {
        Some(identity)
    }
}

fn find_identity_fields(value: &Value, identity: &mut SessionIdentity) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                if let Some(text) = item
                    .as_str()
                    .map(clean_whitespace)
                    .filter(|text| !text.is_empty())
                {
                    let lower_key = key.to_lowercase();
                    if identity.account.is_empty()
                        && (lower_key.contains("login")
                            || lower_key.contains("account")
                            || lower_key.contains("userno")
                            || lower_key.contains("username")
                            || lower_key.contains("user_name")
                            || key.contains("账号"))
                    {
                        identity.account = text.clone();
                    }
                    if identity.display_name.is_empty()
                        && (lower_key.contains("display")
                            || lower_key.contains("fullname")
                            || lower_key.contains("fdname")
                            || lower_key == "name"
                            || key.contains("姓名")
                            || key.contains("名称"))
                    {
                        identity.display_name = text.clone();
                    }
                }
                if identity.account.is_empty() || identity.display_name.is_empty() {
                    find_identity_fields(item, identity);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if identity.account.is_empty() || identity.display_name.is_empty() {
                    find_identity_fields(item, identity);
                }
            }
        }
        _ => {}
    }
}

fn fetch_identity_from_pages(client: &Client) -> Option<SessionIdentity> {
    let mut merged = SessionIdentity::default();
    for path in IDENTITY_PATHS {
        let url = format!("{BASE_URL}{path}");
        let Ok(response) = client
            .get(url)
            .send()
            .and_then(|item| item.error_for_status())
        else {
            continue;
        };
        let Ok(text) = response.text() else {
            continue;
        };
        if looks_like_login_page(&text) {
            continue;
        }
        if let Some(identity) = extract_identity_from_text(&text) {
            merged.merge(Some(identity));
            if !merged.display_name.is_empty() {
                return Some(merged);
            }
        }
    }
    if merged.account.is_empty() && merged.display_name.is_empty() {
        None
    } else {
        Some(merged)
    }
}

fn extract_identity_from_text(text: &str) -> Option<SessionIdentity> {
    let mut identity = SessionIdentity::default();
    identity.display_name = extract_lui_user_name(text).unwrap_or_default();
    if let Some(account) = capture_identity_text(
        text,
        &[
            r#"(?i)(?:fdLoginName|loginName|login_name|account|userNo|user_no)\s*[:=]\s*["']?([^"',<>\s]+)"#,
            r#"(?:账号|工号)\s*[:：]\s*([^<\s，,;；]+)"#,
        ],
    ) {
        identity.account = account;
    }
    if identity.display_name.is_empty() {
        if let Some(display_name) = capture_identity_text(
            text,
            &[
                r#"(?i)(?:displayName|display_name|fullName|full_name|fdName|userDisplayName)\s*[:=]\s*["']?([^"',<>]{2,40})"#,
                r#"(?:姓名|当前用户|登录人)\s*[:：]\s*([^<\s，,;；]{2,20})"#,
            ],
        ) {
            identity.display_name = display_name;
        }
    }
    if identity.display_name.is_empty() {
        identity.display_name = extract_welcome_name(text).unwrap_or_default();
    }
    if identity.account.is_empty() && identity.display_name.is_empty() {
        None
    } else {
        Some(identity)
    }
}

fn extract_lui_user_name(text: &str) -> Option<String> {
    let info_html = Regex::new(
        r#"(?is)<div[^>]*class=["'][^"']*\blui_user_img\b[^"']*["'][^>]*>.*?<span[^>]*class=["'][^"']*\binfo\b[^"']*["'][^>]*>(.*?)</span>"#,
    )
    .ok()?
    .captures(text)
    .and_then(|captures| captures.get(1).map(|item| item.as_str().to_string()))?;
    let name = strip_html(&info_html);
    if name.is_empty() || looks_like_placeholder_identity(&name) {
        None
    } else {
        Some(name)
    }
}

fn capture_identity_text(text: &str, patterns: &[&str]) -> Option<String> {
    for pattern in patterns {
        let Ok(regex) = Regex::new(pattern) else {
            continue;
        };
        let Some(captures) = regex.captures(text) else {
            continue;
        };
        let Some(value) = captures
            .get(1)
            .map(|item| clean_whitespace(&html_unescape(item.as_str())))
        else {
            continue;
        };
        let cleaned = value
            .trim_matches(['"', '\'', ',', ';', '，', '；'])
            .trim()
            .to_string();
        if !cleaned.is_empty() && !looks_like_placeholder_identity(&cleaned) {
            return Some(cleaned);
        }
    }
    None
}

fn extract_welcome_name(text: &str) -> Option<String> {
    let plain = strip_html(text);
    capture_identity_text(
        &plain,
        &[
            r#"欢迎\s*([^,，\s]{2,20})"#,
            r#"([^,，\s]{2,20})\s*，?\s*欢迎"#,
        ],
    )
}

fn looks_like_placeholder_identity(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains("null")
        || lower.contains("undefined")
        || lower.contains("username")
        || lower.contains("display")
        || lower.contains("function")
        || lower.starts_with("${")
}

fn read_hollysys_cookie_rows(cookie_db: &Path) -> Result<Vec<CookieRow>> {
    let temp_db = copy_cookie_db_to_temp(cookie_db)?;
    let connection =
        Connection::open_with_flags(&temp_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("无法读取浏览器 Cookie 数据库: {}", cookie_db.display()))?;
    let mut statement = connection.prepare(
        "SELECT host_key, name, encrypted_value
         FROM cookies
         WHERE host_key LIKE ?1 AND name <> ''
         ORDER BY
           CASE
             WHEN host_key = 'www.hollysys.net' THEN 0
             WHEN host_key = '.hollysys.net' THEN 1
             ELSE 2
           END,
           host_key ASC,
           name ASC",
    )?;
    let rows = statement.query_map(["%hollysys.net%"], |row| {
        Ok(CookieRow {
            host_key: row.get(0)?,
            name: row.get(1)?,
            encrypted_value: row.get(2)?,
        })
    })?;
    let result = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    drop(connection);
    let _ = std::fs::remove_file(temp_db);
    Ok(result)
}

fn copy_cookie_db_to_temp(cookie_db: &Path) -> Result<PathBuf> {
    let temp_path = std::env::temp_dir().join(format!(
        "project-file-compare-browser-cookies-{}-{}.sqlite",
        std::process::id(),
        chrono::Local::now()
            .timestamp_nanos_opt()
            .unwrap_or_default()
    ));
    std::fs::copy(cookie_db, &temp_path)
        .with_context(|| format!("无法复制浏览器 Cookie 数据库: {}", cookie_db.display()))?;
    Ok(temp_path)
}

fn resolve_cookie_db(browser: &BrowserConfig) -> Result<PathBuf> {
    let candidates = candidate_cookie_dbs(browser);
    for candidate in &candidates {
        if !candidate.exists() {
            continue;
        }
        if read_hollysys_cookie_rows(candidate)
            .map(|rows| {
                rows.iter()
                    .any(|row| cookie_host_matches(&row.host_key, BASE_HOST))
            })
            .unwrap_or(false)
        {
            return Ok(candidate.clone());
        }
    }
    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| {
            anyhow!(
                "未找到 {} Cookie 数据库: {}",
                browser.display_name(),
                browser.user_data_dir.display()
            )
        })
}

fn candidate_cookie_dbs(browser: &BrowserConfig) -> Vec<PathBuf> {
    let user_data_dir = &browser.user_data_dir;
    let mut profiles = Vec::new();
    if is_auto_profile(&browser.profile) {
        push_unique_path(&mut profiles, user_data_dir.clone());
        push_unique_path(&mut profiles, user_data_dir.join("Default"));
    } else {
        push_unique_path(&mut profiles, user_data_dir.join(browser.profile.trim()));
        push_unique_path(&mut profiles, user_data_dir.clone());
    }
    if let Ok(entries) = std::fs::read_dir(&user_data_dir) {
        let mut profile_dirs = entries
            .filter_map(|entry| entry.ok().map(|item| item.path()))
            .filter(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .map(|name| profile_name_regex().is_match(&name.to_string_lossy()))
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        profile_dirs.sort();
        if is_auto_profile(&browser.profile) {
            for profile in profile_dirs {
                push_unique_path(&mut profiles, profile);
            }
        }
    }
    let relative_paths = if cfg!(windows) {
        vec![
            PathBuf::from("Network").join("Cookies"),
            PathBuf::from("Cookies"),
        ]
    } else {
        vec![PathBuf::from("Cookies")]
    };
    let mut candidates = Vec::new();
    for profile in profiles {
        for relative_path in &relative_paths {
            let candidate = profile.join(relative_path);
            push_unique_path(&mut candidates, candidate);
        }
    }
    candidates
}

fn profile_name_regex() -> &'static Regex {
    PROFILE_NAME_RE.get_or_init(|| Regex::new(r"^Profile \d+$").expect("valid profile regex"))
}

fn is_required_cookie(name: &str) -> bool {
    matches!(
        name,
        "JSESSIONID" | "SESSION" | "SESSIONID" | "LtpaToken" | "LtpaToken2"
    )
}

fn read_browser_cookie_key(browser: &BrowserConfig) -> Result<[u8; 16]> {
    if cfg!(windows) {
        return read_windows_browser_master_key(browser);
    }
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-w",
            "-s",
            browser.safe_storage_service(),
        ])
        .output()
        .with_context(|| format!("无法读取 macOS Keychain {}", browser.safe_storage_service()))?;
    if !output.status.success() {
        bail!(
            "读取 {} 失败，请确认已允许访问钥匙串",
            browser.safe_storage_service()
        );
    }
    let password = String::from_utf8_lossy(&output.stdout)
        .trim()
        .as_bytes()
        .to_vec();
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(&password, COOKIE_SALT, COOKIE_ITERATIONS, &mut key);
    Ok(key)
}

fn decrypt_browser_cookie(
    host_key: &str,
    encrypted_value: &[u8],
    key: &[u8; 16],
    browser: &BrowserConfig,
) -> Result<String> {
    if encrypted_value.is_empty() {
        return Ok(String::new());
    }
    if cfg!(windows) {
        return decrypt_windows_browser_cookie(encrypted_value, key, browser);
    }
    if !encrypted_value.starts_with(b"v10") && !encrypted_value.starts_with(b"v11") {
        return Ok(String::from_utf8_lossy(encrypted_value).to_string());
    }
    let payload = &encrypted_value[3..];
    let mut buffer = payload.to_vec();
    let decrypted = cbc::Decryptor::<Aes128>::new(key.into(), COOKIE_IV.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|_| anyhow!("{} Cookie 解密失败", browser.display_name()))?;
    let host_prefix = Sha256::digest(host_key.as_bytes());
    let value = if decrypted.starts_with(&host_prefix) {
        &decrypted[host_prefix.len()..]
    } else {
        decrypted
    };
    Ok(String::from_utf8_lossy(value).to_string())
}

#[cfg(not(windows))]
fn read_windows_browser_master_key(_browser: &BrowserConfig) -> Result<[u8; 16]> {
    bail!("Windows 浏览器 Cookie 解密仅支持 Windows")
}

#[cfg(windows)]
fn read_windows_browser_master_key(browser: &BrowserConfig) -> Result<[u8; 16]> {
    let local_state_path = browser.local_state_path();
    let payload = std::fs::read_to_string(&local_state_path).with_context(|| {
        format!(
            "未找到 {} Local State: {}",
            browser.display_name(),
            local_state_path.display()
        )
    })?;
    let value: Value = serde_json::from_str(&payload)?;
    let encrypted_key_b64 = value
        .get("os_crypt")
        .and_then(|item| item.get("encrypted_key"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow!(
                "{} Local State 未包含 os_crypt.encrypted_key: {}",
                browser.display_name(),
                local_state_path.display()
            )
        })?;
    let mut encrypted_key = base64::engine::general_purpose::STANDARD.decode(encrypted_key_b64)?;
    if encrypted_key.starts_with(b"DPAPI") {
        encrypted_key.drain(..5);
    }
    let key = crypt_unprotect_data(&encrypted_key)?;
    key.try_into()
        .map_err(|_| anyhow!("Windows {} 主密钥长度异常", browser.display_name()))
}

#[cfg(windows)]
fn crypt_unprotect_data(encrypted_value: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    use windows_sys::Win32::System::Memory::LocalFree;

    let mut input = CRYPT_INTEGER_BLOB {
        cbData: encrypted_value.len() as u32,
        pbData: encrypted_value.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &mut input,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        bail!("Windows DPAPI 解密浏览器 Cookie 失败");
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData as isize);
    }
    Ok(result)
}

#[cfg(not(windows))]
fn decrypt_windows_browser_cookie(
    _encrypted_value: &[u8],
    _key: &[u8; 16],
    _browser: &BrowserConfig,
) -> Result<String> {
    bail!("Windows 浏览器 Cookie 解密仅支持 Windows")
}

#[cfg(windows)]
fn decrypt_windows_browser_cookie(
    encrypted_value: &[u8],
    key: &[u8; 16],
    browser: &BrowserConfig,
) -> Result<String> {
    if encrypted_value.starts_with(b"v20") {
        bail!(
            "{} 已启用 App-Bound Encryption（v20），当前模式无法直接解密本机 Cookie",
            browser.display_name()
        );
    }
    if encrypted_value.starts_with(b"v10") || encrypted_value.starts_with(b"v11") {
        let payload = &encrypted_value[3..];
        if payload.len() < 12 + 16 {
            bail!("浏览器 Cookie 密文长度异常");
        }
        let nonce = &payload[..12];
        let ciphertext_and_tag = &payload[12..];
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::Aes128Gcm;
        let cipher = Aes128Gcm::new_from_slice(key)
            .map_err(|_| anyhow!("Windows {} 主密钥异常", browser.display_name()))?;
        let decrypted = cipher
            .decrypt(aes_gcm::Nonce::from_slice(nonce), ciphertext_and_tag)
            .map_err(|_| anyhow!("Windows {} Cookie 解密失败", browser.display_name()))?;
        return Ok(String::from_utf8_lossy(&decrypted).to_string());
    }
    Ok(String::from_utf8_lossy(&crypt_unprotect_data(encrypted_value)?).to_string())
}

fn build_list_url(category_id: &str) -> String {
    format!("{BASE_URL}/sys/notify/sys_notify_todo/sysNotifyMainIndex.do?method=list&from=aggregation&dataType=todo&fdType=13&aggregationId={category_id}")
}

impl TodoItem {
    fn detail_url(&self) -> String {
        let path = if self.detail_path.starts_with("http") {
            self.detail_path.clone()
        } else {
            format!("{BASE_URL}/{}", self.detail_path.trim_start_matches('/'))
        };
        if path.contains('?') {
            format!("{path}&LLType=PC")
        } else {
            format!("{path}?LLType=PC")
        }
    }

    fn notify_view_url(&self) -> String {
        format!(
            "{BASE_URL}/sys/notify/sys_notify_todo/sysNotifyTodo.do?method=view&fdId={}",
            self.todo_fd_id
        )
    }
}

impl Attachment {
    fn download_url(&self) -> String {
        format!(
            "{BASE_URL}/sys/attachment/sys_att_main/sysAttMain.do?method=download&fdId={}",
            self.fd_id
        )
    }
}

fn normalize_project_code(project_code: &str) -> String {
    clean_whitespace(project_code).replace('/', "-")
}

fn sanitize_filename(filename: &str) -> String {
    let cleaned = Regex::new(r#"[\\/]+"#)
        .map(|re| re.replace_all(&clean_whitespace(filename), "-").to_string())
        .unwrap_or_else(|_| clean_whitespace(filename));
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
    }
}

fn extract_project_code(value: &str) -> Option<String> {
    Regex::new(r"项目号[:：]\s*([A-Z]+-\d+(?:/[A-Z0-9]+)?)")
        .ok()?
        .captures(value)
        .and_then(|captures| captures.get(1).map(|item| item.as_str().to_string()))
}

fn strip_html(value: &str) -> String {
    let stripped = Regex::new(r"<[^>]+>")
        .map(|re| re.replace_all(value, " ").to_string())
        .unwrap_or_else(|_| value.to_string());
    clean_whitespace(&html_unescape(&stripped))
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn clean_whitespace(value: &str) -> String {
    Regex::new(r"\s+")
        .map(|re| re.replace_all(value, " ").trim().to_string())
        .unwrap_or_else(|_| value.trim().to_string())
}

fn decode_js_string(value: &str) -> String {
    serde_json::from_str::<String>(&format!("\"{value}\"")).unwrap_or_else(|_| value.to_string())
}

fn looks_like_html(content: &[u8]) -> bool {
    let prefix = String::from_utf8_lossy(content)
        .trim_start()
        .chars()
        .take(128)
        .collect::<String>()
        .to_lowercase();
    prefix.starts_with("<!doctype html") || prefix.starts_with("<html")
}

fn looks_like_login_page(content: &str) -> bool {
    let text = content.to_lowercase();
    let title = Regex::new(r"(?is)<title[^>]*>(.*?)</title>")
        .ok()
        .and_then(|regex| regex.captures(content))
        .and_then(|captures| captures.get(1).map(|item| clean_whitespace(item.as_str())))
        .unwrap_or_default();
    let title_lower = title.to_lowercase();
    title.contains("登录")
        || title_lower.contains("login")
        || text.contains("location.href = '/login.jsp")
        || text.contains("location.href='/login.jsp")
        || text.contains("window.location.href = '/login.jsp")
        || text.contains("window.location.href='/login.jsp")
        || text.contains("<form") && text.contains("login.do")
        || text.contains("name=\"j_username\"")
        || text.contains("name=\"password\"")
}

fn json_contains_expired_signal(value: &Value) -> bool {
    match value {
        Value::String(text) => looks_like_login_page(text),
        Value::Array(items) => items.iter().any(json_contains_expired_signal),
        Value::Object(map) => map.iter().any(|(key, item)| {
            let lower_key = key.to_lowercase();
            (lower_key.contains("login") || lower_key.contains("session"))
                && item.as_str().map(looks_like_login_page).unwrap_or(false)
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cookie_host_matches, extract_identity_from_text, extract_lui_user_name, looks_like_login_page,
    };

    #[test]
    fn extracts_lui_user_name_from_home_header() {
        let html = r#"
            <div class="lui_user_img">
                <span class="lui_tlayout_avatar userimg">
                    <img alt="" src="/sys/person/image.jsp?personId=19ad7f5f621bf584ae720d940dfba5b3&amp;size=20">
                </span>
                <span class="info">
                    冯雨翔
                </span>
            </div>
        "#;

        assert_eq!(extract_lui_user_name(html).as_deref(), Some("冯雨翔"));
        let identity = extract_identity_from_text(html).expect("identity");
        assert_eq!(identity.display_name, "冯雨翔");
        assert!(identity.account.is_empty());
    }

    #[test]
    fn filters_host_only_cookies_like_browser() {
        assert!(cookie_host_matches("www.hollysys.net", "www.hollysys.net"));
        assert!(cookie_host_matches(".hollysys.net", "www.hollysys.net"));
        assert!(!cookie_host_matches("sso.hollysys.net", "www.hollysys.net"));
    }

    #[test]
    fn normal_home_script_text_is_not_login_page() {
        let html = r#"
            <html>
              <head><title>工作台（和利时）</title></head>
              <script>var SessionExpireTip="会话过期，请打开新页面重新登录后再提交本页面。";</script>
              <body><div class="lui_user_img"><span class="info">冯雨翔</span></div></body>
            </html>
        "#;

        assert!(!looks_like_login_page(html));
    }

    #[test]
    fn non_json_home_with_identity_is_usable_identity_source() {
        let html = r#"
            <html>
              <head><title>工作台（和利时）</title></head>
              <body>
                <div class="lui_user_img">
                  <span class="lui_tlayout_avatar userimg"></span>
                  <span class="info">冯雨翔</span>
                </div>
              </body>
            </html>
        "#;

        assert!(serde_json::from_str::<serde_json::Value>(html).is_err());
        assert!(!looks_like_login_page(html));
        let identity = extract_identity_from_text(html).expect("identity from html");
        assert_eq!(identity.display_name, "冯雨翔");
    }
}
