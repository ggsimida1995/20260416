use crate::core::cancel::CancelFlag;
use crate::core::config::app_state_db_path;
use crate::core::models::AppSettings;
use crate::db::app_state::AppStateStore;
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const BASE_URL: &str = "https://www.hollysys.net";
const BASE_HOST: &str = "www.hollysys.net";
const HTTP_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";
const TODO_PAGE_SIZE: usize = 100;
const TODO_MAX_PAGES: usize = 200;
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

#[derive(Debug, Clone)]
struct TodoItem {
    category_name: String,
    todo_fd_id: String,
    subject: String,
    detail_path: String,
}

#[derive(Debug, Clone)]
struct TodoListPage {
    items: Vec<TodoItem>,
    total_count: Option<usize>,
    total_pages: Option<usize>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub state: String,
    pub message: String,
    pub browser_name: String,
    pub account: String,
    pub display_name: String,
    pub checked_at: String,
}

pub fn run_download(
    file_root: &Path,
    skip_project_codes: &HashSet<String>,
    settings: &AppSettings,
    cancel: &CancelFlag,
) -> Result<DownloadSummary> {
    std::fs::create_dir_all(file_root)?;
    let timeout = settings.request_timeout_seconds.max(1) as u64;
    let client = build_authenticated_client_from_stored_cookies(timeout)?;
    let mut summary = DownloadSummary {
        processed_count: 0,
        saved_project_dirs: Vec::new(),
        skipped_projects: Vec::new(),
        errors: Vec::new(),
    };

    for (category_id, category_name) in AGGREGATION_CATEGORIES {
        if cancel.is_cancelled() {
            summary.errors.push("已取消".to_string());
            return Ok(summary);
        }
        let items = fetch_todo_items(&client, category_id, category_name)?;
        for item in items {
            if cancel.is_cancelled() {
                summary.errors.push("已取消".to_string());
                return Ok(summary);
            }
            let record = match fetch_detail_record(&client, item.clone()) {
                Ok(record) => record,
                Err(error) => {
                    summary
                        .errors
                        .push(format!("{} | {}", item.detail_url(), error));
                    return Ok(summary);
                }
            };
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
                Err(error) => {
                    summary
                        .errors
                        .push(format!("{} | {}", record.project_code, error));
                    return Ok(summary);
                }
            }
        }
    }
    Ok(summary)
}

pub fn check_session_status(settings: &AppSettings) -> SessionStatus {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut status = SessionStatus {
        state: "checking".to_string(),
        message: "正在检测会话".to_string(),
        browser_name: "内置 WebView".to_string(),
        account: String::new(),
        display_name: String::new(),
        checked_at: now,
    };

    #[cfg(target_os = "windows")]
    if let Err(error) = import_external_login_cookies() {
        eprintln!("[session] import external login cookies skipped: {error:#}");
    }

    let timeout = settings.request_timeout_seconds.max(1) as u64;
    let client = match build_authenticated_client_from_stored_cookies(timeout) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[session] build client failed: {error:#}");
            status.state = "missing".to_string();
            status.message = format!("未登录: {error}");
            return status;
        }
    };

    match verify_session(&client) {
        Ok(identity) => {
            status.state = "ok".to_string();
            status.message = "会话可用".to_string();
            status.account = if identity.account.is_empty() {
                settings.account.clone()
            } else {
                identity.account
            };
            status.display_name = identity.display_name;
        }
        Err(error) => {
            eprintln!("[session] verify failed: {error:#}");
            status.state = "missing".to_string();
            status.message = format!("会话验证失败: {error}");
        }
    }
    status
}

#[cfg(target_os = "windows")]
fn import_external_login_cookies() -> Result<()> {
    let cookies = crate::core::browser_login::import_external_login_cookies()?;
    if cookies.is_empty() {
        return Ok(());
    }
    AppStateStore::new(app_state_db_path()).save_cookies(&cookies)?;
    Ok(())
}

pub fn unchecked_session_status(_settings: &AppSettings) -> SessionStatus {
    SessionStatus {
        state: "unknown".to_string(),
        message: "未检测登录状态".to_string(),
        browser_name: "内置 WebView".to_string(),
        account: String::new(),
        display_name: String::new(),
        checked_at: String::new(),
    }
}

pub fn build_authenticated_client_from_stored_cookies(timeout_seconds: u64) -> Result<Client> {
    let store = AppStateStore::new(app_state_db_path());
    let cookies = store.load_cookies()?;
    if cookies.is_empty() {
        bail!("尚未登录，请点击侧栏的「登录系统」按钮");
    }
    let mut cookie_pairs = Vec::new();
    let mut seen = HashSet::new();
    for cookie in cookies {
        if !cookie_host_matches(&cookie.domain, BASE_HOST) {
            continue;
        }
        if cookie.value.is_empty() {
            continue;
        }
        if !seen.insert(cookie.name.clone()) {
            continue;
        }
        cookie_pairs.push(format!("{}={}", cookie.name, cookie.value));
    }
    if cookie_pairs.is_empty() {
        bail!("已登录数据库为空或未匹配到适用 Cookie，请重新登录");
    }
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(HTTP_USER_AGENT));
    headers.insert(COOKIE, HeaderValue::from_str(&cookie_pairs.join("; "))?);
    Ok(Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(timeout_seconds))
        .build()?)
}

fn cookie_host_matches(cookie_host: &str, request_host: &str) -> bool {
    let domain = cookie_host.trim_start_matches('.');
    if domain.is_empty() {
        return false;
    }
    request_host == domain || request_host.ends_with(&format!(".{domain}"))
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
    let url = build_list_url(category_id, 1, TODO_PAGE_SIZE);
    let response = client.get(&url).send()?.error_for_status()?.text()?;
    if looks_like_login_page(&response) {
        bail!("Hollysys 会话已失效，请先在浏览器中重新登录");
    }
    let payload: Value = match serde_json::from_str(response.trim_start()) {
        Ok(payload) => payload,
        Err(error) => {
            if let Some(identity) = fetch_identity_from_pages(client) {
                return Ok(identity);
            }
            return Err(error)
                .context("会话检测接口返回不是 JSON，已读取到 Cookie，但接口响应异常");
        }
    };
    if json_contains_expired_signal(&payload) {
        bail!("Hollysys 会话已失效，请先在浏览器中重新登录");
    }
    let mut identity = extract_identity_from_value(&payload).unwrap_or_default();
    if identity.account.is_empty() || identity.display_name.is_empty() {
        identity.merge(fetch_identity_from_pages(client));
    }
    if !identity.is_confirmed() {
        bail!("已读取到 Cookie，但未确认当前登录账号，请重新登录");
    }
    Ok(identity)
}

#[derive(Debug, Clone, Default)]
struct SessionIdentity {
    account: String,
    display_name: String,
}

impl SessionIdentity {
    fn is_confirmed(&self) -> bool {
        !self.account.is_empty() || !self.display_name.is_empty()
    }

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

fn fetch_todo_items(
    client: &Client,
    category_id: &str,
    category_name: &str,
) -> Result<Vec<TodoItem>> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for page_no in 1..=TODO_MAX_PAGES {
        let page = fetch_todo_items_page(client, category_id, category_name, page_no)?;
        if page.items.is_empty() {
            break;
        }
        let total_count = page.total_count;
        let total_pages = page.total_pages;
        let before_count = items.len();
        for item in page.items {
            let key = todo_item_key(&item);
            if seen.insert(key) {
                items.push(item);
            }
        }
        if items.len() == before_count {
            break;
        }
        if todo_pagination_complete(page_no, items.len(), total_count, total_pages) {
            break;
        }
    }
    Ok(items)
}

fn fetch_todo_items_page(
    client: &Client,
    category_id: &str,
    category_name: &str,
    page_no: usize,
) -> Result<TodoListPage> {
    let response = client
        .get(build_list_url(category_id, page_no, TODO_PAGE_SIZE))
        .send()?
        .error_for_status()?
        .text()?;
    if looks_like_login_page(&response) {
        bail!("Hollysys 会话已失效，请先在浏览器中重新登录");
    }
    let payload: Value = serde_json::from_str(response.trim_start())
        .with_context(|| format!("待办列表返回不是 JSON，分类: {category_name}"))?;
    Ok(parse_todo_list_page(&payload, category_name))
}

fn parse_todo_list_page(payload: &Value, category_name: &str) -> TodoListPage {
    let mut items = Vec::new();
    let Some(rows) = payload.get("datas").and_then(Value::as_array) else {
        return TodoListPage {
            items,
            total_count: todo_numeric_field(payload, &["total", "totalCount", "recordCount", "records"]),
            total_pages: todo_numeric_field(payload, &["totalPage", "totalPages", "pageCount"]),
        };
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
    TodoListPage {
        items,
        total_count: todo_numeric_field(payload, &["total", "totalCount", "recordCount", "records"]),
        total_pages: todo_numeric_field(payload, &["totalPage", "totalPages", "pageCount"]),
    }
}

fn todo_item_key(item: &TodoItem) -> String {
    if !item.todo_fd_id.is_empty() {
        return format!("id:{}", item.todo_fd_id);
    }
    format!("path:{}", item.detail_path)
}

fn todo_pagination_complete(
    page_no: usize,
    loaded_count: usize,
    total_count: Option<usize>,
    total_pages: Option<usize>,
) -> bool {
    if let Some(total_pages) = total_pages {
        if total_pages > 0 && page_no >= total_pages {
            return true;
        }
    }
    if let Some(total_count) = total_count {
        if total_count > 0 && loaded_count >= total_count {
            return true;
        }
    }
    false
}

fn todo_numeric_field(payload: &Value, keys: &[&str]) -> Option<usize> {
    for key in keys {
        if let Some(value) = payload.get(*key).and_then(value_to_usize) {
            return Some(value);
        }
    }
    None
}

fn value_to_usize(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .and_then(|item| usize::try_from(item).ok())
        .or_else(|| {
            value
                .as_str()
                .and_then(|text| text.trim().parse::<usize>().ok())
        })
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
        .and_then(|value| extract_project_code(&value).or(Some(value)))
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
        &["关闭移交登记表"],
        &[".xlsx", ".xls"],
    );
    pick_best(
        &remaining,
        &mut selected,
        &["竣工总结报告"],
        &[".docx", ".doc"],
    );
    pick_best(
        &remaining,
        &mut selected,
        &["竣工验收报告"],
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

fn build_list_url(category_id: &str, page_no: usize, page_size: usize) -> String {
    format!("{BASE_URL}/sys/notify/sys_notify_todo/sysNotifyMainIndex.do?method=list&from=aggregation&dataType=todo&fdType=13&aggregationId={category_id}&pageno={page_no}&rowsize={page_size}&pageNo={page_no}&rowSize={page_size}&pageSize={page_size}")
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
    sanitize_path_component(&clean_whitespace(project_code).replace('/', "-"))
}

fn sanitize_filename(filename: &str) -> String {
    let cleaned = sanitize_path_component(&clean_whitespace(filename));
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
    }
}

fn extract_project_code(value: &str) -> Option<String> {
    Regex::new(r"(?i)(?:项目号|项目编号)?[:：]?\s*([A-Z]+-\d+(?:/[A-Z0-9]+)?)")
        .ok()?
        .captures(value)
        .and_then(|captures| captures.get(1).map(|item| item.as_str().to_uppercase()))
}

fn strip_html(value: &str) -> String {
    let without_scripts = Regex::new(r"(?is)<(?:script|style)\b[^>]*>.*?</(?:script|style)>")
        .map(|re| re.replace_all(value, " ").to_string())
        .unwrap_or_else(|_| value.to_string());
    let stripped = Regex::new(r"<[^>]+>")
        .map(|re| re.replace_all(&without_scripts, " ").to_string())
        .unwrap_or(without_scripts);
    clean_whitespace(&html_unescape(&stripped))
}

fn sanitize_path_component(value: &str) -> String {
    let mut result = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        let replacement = ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*');
        let next = if replacement { '-' } else { ch };
        if next == '-' {
            if !last_dash {
                result.push(next);
            }
            last_dash = true;
        } else {
            result.push(next);
            last_dash = false;
        }
        if result.len() >= 120 {
            break;
        }
    }
    let cleaned = result.trim_matches(&[' ', '.', '-'][..]).to_string();
    if is_windows_reserved_name(&cleaned) {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

fn is_windows_reserved_name(value: &str) -> bool {
    let upper = value
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
            | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
            | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    )
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
        build_list_url, cookie_host_matches, extract_detail_field, extract_identity_from_text,
        extract_lui_user_name, extract_project_code, looks_like_login_page, normalize_project_code,
        parse_todo_list_page, sanitize_filename, todo_pagination_complete, SessionIdentity,
    };
    use serde_json::json;

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

    #[test]
    fn detail_field_ignores_embedded_script_text() {
        let html = r#"
            <table>
              <tr>
                <td><label>项目编号</label></td>
                <td>
                  BHE-25090233/01
                  <script>//此处添加js代码 alert("noise")</script>
                </td>
              </tr>
            </table>
        "#;

        assert_eq!(
            extract_detail_field(html, "项目编号").as_deref(),
            Some("BHE-25090233/01")
        );
    }

    #[test]
    fn project_code_extraction_stops_before_script_noise() {
        let text = r#"BHE-25090233/01 //此处添加js代码 function() { invalid path | noise }"#;

        assert_eq!(extract_project_code(text).as_deref(), Some("BHE-25090233/01"));
    }

    #[test]
    fn path_components_are_safe_for_windows() {
        assert_eq!(normalize_project_code("BHE-25090233/01|bad:name*"), "BHE-25090233-01-bad-name");
        assert_eq!(sanitize_filename(r#"竣工/验收:报告?.pdf"#), "竣工-验收-报告-.pdf");
        assert_eq!(sanitize_filename("CON"), "_CON");
    }

    #[test]
    fn todo_list_url_contains_pagination_parameters() {
        let url = build_list_url("category-id", 3, 100);

        assert!(url.contains("aggregationId=category-id"));
        assert!(url.contains("pageno=3"));
        assert!(url.contains("rowsize=100"));
        assert!(url.contains("pageNo=3"));
    }

    #[test]
    fn parses_todo_list_page_metadata_and_rows() {
        let payload = json!({
            "total": "2",
            "totalPage": 2,
            "datas": [
                [
                    { "col": "fdId", "value": "todo-1" },
                    { "col": "tr_href", "value": "/detail/1" },
                    { "col": "todo.subject4View", "value": "<span>项目号：BHE-TEST/01</span>" }
                ]
            ]
        });

        let page = parse_todo_list_page(&payload, "项目关闭工作流");

        assert_eq!(page.total_count, Some(2));
        assert_eq!(page.total_pages, Some(2));
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].todo_fd_id, "todo-1");
        assert_eq!(page.items[0].detail_path, "/detail/1");
        assert_eq!(page.items[0].subject, "项目号：BHE-TEST/01");
        assert!(!todo_pagination_complete(1, 1, page.total_count, page.total_pages));
        assert!(todo_pagination_complete(2, 2, page.total_count, page.total_pages));
    }

    #[test]
    fn empty_session_identity_is_not_confirmed() {
        let empty = SessionIdentity::default();
        let with_account = SessionIdentity {
            account: "user1".to_string(),
            display_name: String::new(),
        };
        let with_name = SessionIdentity {
            account: String::new(),
            display_name: "张三".to_string(),
        };

        assert!(!empty.is_confirmed());
        assert!(with_account.is_confirmed());
        assert!(with_name.is_confirmed());
    }
}
