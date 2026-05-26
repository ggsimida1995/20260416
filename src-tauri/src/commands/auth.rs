use crate::core::config::app_state_db_path;
use crate::db::app_state::{AppStateStore, CookieEntry};
use tauri::webview::{Cookie, PageLoadEvent};
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const LOGIN_WINDOW_LABEL: &str = "login";
const PREVIEW_WINDOW_LABEL: &str = "session-preview";
const HOLLYSYS_URL: &str = "https://www.hollysys.net/";
const HOLLYSYS_HOST: &str = "www.hollysys.net";
const HOLLYSYS_TODO_PREVIEW_URL: &str = "https://www.hollysys.net/sys/notify/sys_notify_todo/sysNotifyMainIndex.do?method=list&from=aggregation&dataType=todo&fdType=13&aggregationId=18a032b3695468f23f38a0f40d5a3602&pageno=1&rowsize=100&pageNo=1&rowSize=100&pageSize=100";
const AUTH_COOKIES_UPDATED_EVENT: &str = "auth://cookies-updated";
const AUTH_LOGIN_PAGE_DETECTED_EVENT: &str = "auth://login-page-detected";
const AUTH_LOGIN_WINDOW_CLOSED_EVENT: &str = "auth://login-window-closed";

#[cfg(target_os = "windows")]
const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";
#[cfg(target_os = "macos")]
const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";

#[tauri::command]
pub async fn open_login_window(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        crate::core::browser_login::open_external_login().map_err(to_string)?;
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
            existing.show().map_err(to_string)?;
            existing.set_focus().map_err(to_string)?;
            return Ok(());
        }

        let settings = crate::commands::state::load_settings().map_err(|e| e.to_string())?;
        let autofill_script = if !settings.account.is_empty() || !settings.password.is_empty() {
            Some(build_autofill_script(&settings.account, &settings.password))
        } else {
            None
        };

        let login_url: Url = HOLLYSYS_URL.parse().map_err(to_string)?;
        let app_handle = app.clone();
        let login_window =
            WebviewWindowBuilder::new(&app, LOGIN_WINDOW_LABEL, WebviewUrl::External(login_url))
                .title("登录账号")
                .inner_size(1024.0, 720.0)
                .min_inner_size(800.0, 600.0)
                .resizable(true)
                .closable(true)
                .focused(true)
                .center()
                .user_agent(LOGIN_USER_AGENT)
                .on_page_load(move |window, payload| {
                    if payload.event() != PageLoadEvent::Finished {
                        return;
                    }
                    if let Some(script) = autofill_script.as_ref() {
                        let _ = window.eval(script);
                    }
                    if is_login_url(payload.url().as_str()) {
                        let _ = app_handle.emit(AUTH_LOGIN_PAGE_DETECTED_EVENT, ());
                        return;
                    }
                    if let Err(error) = capture_cookies(&window) {
                        eprintln!("[auth] capture cookies failed: {error}");
                        return;
                    }
                    let _ = app_handle.emit(AUTH_COOKIES_UPDATED_EVENT, ());
                })
                .build()
                .map_err(to_string)?;
        let login_window_for_close = login_window.clone();
        let app_handle_for_close = app.clone();
        login_window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = app_handle_for_close.emit(AUTH_LOGIN_WINDOW_CLOSED_EVENT, ());
                let _ = login_window_for_close.destroy();
            }
        });

        Ok(())
    }
}

#[tauri::command]
pub async fn open_session_preview_window(app: AppHandle) -> Result<(), String> {
    let preview_url: Url = HOLLYSYS_TODO_PREVIEW_URL.parse().map_err(to_string)?;
    if let Some(existing) = app.get_webview_window(PREVIEW_WINDOW_LABEL) {
        apply_stored_cookies(&existing)?;
        existing.navigate(preview_url).map_err(to_string)?;
        existing.show().map_err(to_string)?;
        existing.set_focus().map_err(to_string)?;
        return Ok(());
    }

    let blank_url: Url = "about:blank".parse().map_err(to_string)?;
    let app_handle = app.clone();
    let preview_window =
        WebviewWindowBuilder::new(&app, PREVIEW_WINDOW_LABEL, WebviewUrl::External(blank_url))
            .title("已登录网站预览")
            .inner_size(1280.0, 820.0)
            .min_inner_size(960.0, 640.0)
            .resizable(true)
            .closable(true)
            .focused(true)
            .center()
            .user_agent(LOGIN_USER_AGENT)
            .on_page_load(move |window, payload| {
                if payload.event() != PageLoadEvent::Finished {
                    return;
                }
                if is_login_url(payload.url().as_str()) {
                    let _ = app_handle.emit(AUTH_LOGIN_PAGE_DETECTED_EVENT, ());
                    return;
                }
                if let Err(error) = capture_cookies(&window) {
                    eprintln!("[auth] preview capture cookies failed: {error}");
                    return;
                }
                let _ = app_handle.emit(AUTH_COOKIES_UPDATED_EVENT, ());
            })
            .build()
            .map_err(to_string)?;

    if let Err(error) = apply_stored_cookies(&preview_window) {
        let _ = preview_window.destroy();
        return Err(error);
    }
    if let Err(error) = preview_window.navigate(preview_url).map_err(to_string) {
        let _ = preview_window.destroy();
        return Err(error);
    }

    let preview_window_for_close = preview_window.clone();
    preview_window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = preview_window_for_close.destroy();
        }
    });

    Ok(())
}

fn build_autofill_script(account: &str, password: &str) -> String {
    let user_json = serde_json::to_string(account).unwrap_or_else(|_| "\"\"".into());
    let pass_json = serde_json::to_string(password).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(function(){{
  function first(selectors) {{
    for (var i = 0; i < selectors.length; i++) {{
      var el = document.querySelector(selectors[i]);
      if (el) return el;
    }}
    return null;
  }}
  function setValue(el, value) {{
    if (!el) return;
    el.focus();
    var setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value');
    if (setter && setter.set) setter.set.call(el, value);
    else el.value = value;
    ['keydown','keypress','input','keyup','change','blur'].forEach(function(type) {{
      el.dispatchEvent(new Event(type, {{ bubbles: true, cancelable: true }}));
    }});
  }}
  var u = first(['#username_show', '#username', 'input[name="username"]', 'input[name="j_username"]', 'input[name="loginName"]', 'input[type="text"]']);
  var p = first(['#password_show', '#password', 'input[name="password"]', 'input[name="j_password"]', 'input[type="password"]']);
  if ({user}.length) setValue(u, {user});
  if ({pass}.length) setValue(p, {pass});
}})();"#,
        user = user_json,
        pass = pass_json
    )
}

#[tauri::command]
pub async fn close_login_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        window.destroy().map_err(to_string)?;
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

fn apply_stored_cookies(window: &tauri::WebviewWindow) -> Result<(), String> {
    let cookies = AppStateStore::new(app_state_db_path())
        .load_cookies()
        .map_err(to_string)?;
    if cookies.is_empty() {
        return Err("还没有可预览的登录 Cookie，请先登录并刷新会话".to_string());
    }

    let mut applied = 0usize;
    for entry in cookies {
        if entry.name.is_empty()
            || entry.value.is_empty()
            || !cookie_domain_matches(&entry.domain, HOLLYSYS_HOST)
        {
            continue;
        }
        let path = if entry.path.is_empty() {
            "/".to_string()
        } else {
            entry.path
        };
        let cookie = Cookie::build((entry.name, entry.value))
            .domain(entry.domain)
            .path(path)
            .secure(true)
            .build();
        window.set_cookie(cookie).map_err(to_string)?;
        applied += 1;
    }

    if applied == 0 {
        return Err("已保存的 Cookie 不匹配 Hollysys，请重新登录后再预览".to_string());
    }
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
                path: c
                    .path()
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "/".to_string()),
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

fn is_login_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("/login")
        || lower.contains("login.jsp")
        || lower.contains("login.do")
        || lower.contains("login_error")
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::is_login_url;

    #[test]
    fn detects_hollysys_login_urls() {
        assert!(is_login_url("https://www.hollysys.net/login.jsp"));
        assert!(is_login_url("https://www.hollysys.net/j_acegi_security_check?login_error=1"));
        assert!(is_login_url("https://sso.hollysys.net/login"));
        assert!(!is_login_url("https://www.hollysys.net/sys/notify/sys_notify_todo/sysNotifyMainIndex.do"));
    }
}
