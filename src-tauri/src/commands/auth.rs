use crate::core::config::app_state_db_path;
use crate::db::app_state::{AppStateStore, CookieEntry};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const LOGIN_WINDOW_LABEL: &str = "login";
const HOLLYSYS_URL: &str = "https://www.hollysys.net/";
const HOLLYSYS_HOST: &str = "www.hollysys.net";

#[cfg(target_os = "windows")]
const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";
#[cfg(target_os = "macos")]
const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";

#[tauri::command]
pub async fn open_login_window(app: AppHandle) -> Result<(), String> {
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
    let login_window = WebviewWindowBuilder::new(
        &app,
        LOGIN_WINDOW_LABEL,
        WebviewUrl::External(login_url),
    )
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
        if let Err(error) = capture_cookies(&window) {
            eprintln!("[auth] capture cookies failed: {error}");
            return;
        }
        let _ = app_handle.emit("auth://cookies-updated", ());
    })
    .build()
    .map_err(to_string)?;
    let login_window_for_close = login_window.clone();
    login_window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = login_window_for_close.destroy();
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
