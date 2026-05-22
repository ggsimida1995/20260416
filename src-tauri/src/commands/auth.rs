use crate::core::config::app_state_db_path;
use crate::db::app_state::{AppStateStore, CookieEntry};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder};

const LOGIN_WINDOW_LABEL: &str = "login";
const HOLLYSYS_URL: &str = "https://www.hollysys.net/";
const HOLLYSYS_HOST: &str = "www.hollysys.net";
const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";

#[tauri::command]
pub async fn open_login_window(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        existing.show().map_err(to_string)?;
        existing.set_focus().map_err(to_string)?;
        return Ok(());
    }

    let login_url: Url = HOLLYSYS_URL.parse().map_err(to_string)?;
    let app_handle = app.clone();
    WebviewWindowBuilder::new(
        &app,
        LOGIN_WINDOW_LABEL,
        WebviewUrl::External(login_url),
    )
    .title("登录账号")
    .inner_size(1024.0, 720.0)
    .min_inner_size(800.0, 600.0)
    .user_agent(LOGIN_USER_AGENT)
    .on_page_load(move |window, payload| {
        if payload.event() != PageLoadEvent::Finished {
            return;
        }
        if let Err(error) = capture_cookies(&window) {
            eprintln!("[auth] capture cookies failed: {error}");
            return;
        }
        let _ = app_handle.emit("auth://cookies-updated", ());
    })
    .build()
    .map_err(to_string)?;

    Ok(())
}

#[tauri::command]
pub async fn close_login_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        window.close().map_err(to_string)?;
    }
    Ok(())
}

#[tauri::command]
pub fn has_stored_cookies() -> Result<bool, String> {
    let cookies = AppStateStore::new(app_state_db_path())
        .load_cookies()
        .map_err(to_string)?;
    Ok(!cookies.is_empty())
}

#[tauri::command]
pub fn clear_login() -> Result<(), String> {
    AppStateStore::new(app_state_db_path())
        .clear_cookies()
        .map_err(to_string)?;
    Ok(())
}

fn capture_cookies(window: &tauri::WebviewWindow) -> Result<(), String> {
    let cookies = window.cookies().map_err(to_string)?;
    let total = cookies.len();
    let entries: Vec<CookieEntry> = cookies
        .iter()
        .filter_map(|c| {
            let name = c.name();
            if name.is_empty() {
                return None;
            }
            let domain = c.domain().unwrap_or("");
            if !cookie_domain_matches(domain, HOLLYSYS_HOST) {
                return None;
            }
            Some(CookieEntry {
                name: name.to_string(),
                value: c.value().to_string(),
                domain: domain.to_string(),
                path: c.path().map(|p| p.to_string()).unwrap_or_else(|| "/".to_string()),
            })
        })
        .collect();
    eprintln!(
        "[auth] capture_cookies: total={total} matched={} names={:?}",
        entries.len(),
        entries.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    if entries.is_empty() {
        return Ok(());
    }
    AppStateStore::new(app_state_db_path())
        .save_cookies(&entries)
        .map_err(to_string)?;
    Ok(())
}

fn cookie_domain_matches(cookie_domain: &str, request_host: &str) -> bool {
    let stripped = cookie_domain.trim_start_matches('.');
    if stripped.is_empty() {
        return false;
    }
    request_host == stripped || request_host.ends_with(&format!(".{stripped}"))
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
