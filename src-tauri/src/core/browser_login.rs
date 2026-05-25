#[cfg(target_os = "windows")]
use crate::core::config::runtime_root;
#[cfg(target_os = "windows")]
use crate::db::app_state::CookieEntry;
#[cfg(target_os = "windows")]
use anyhow::{anyhow, bail, Context, Result};
#[cfg(target_os = "windows")]
use serde_json::{json, Value};
#[cfg(target_os = "windows")]
use std::path::PathBuf;

#[cfg(target_os = "windows")]
const HOLLYSYS_URL: &str = "https://www.hollysys.net/";

#[cfg(target_os = "windows")]
const DEVTOOLS_PORT: u16 = 41995;

#[cfg(target_os = "windows")]
pub fn open_external_login() -> Result<()> {
    let browser = find_browser_executable()
        .ok_or_else(|| anyhow!("未找到 Edge 或 Chrome，无法打开外部登录窗口"))?;
    let profile_dir = external_login_profile_dir();
    std::fs::create_dir_all(&profile_dir)
        .with_context(|| format!("无法创建登录浏览器目录: {}", profile_dir.display()))?;

    std::process::Command::new(browser)
        .arg(format!("--remote-debugging-port={DEVTOOLS_PORT}"))
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--new-window")
        .arg(HOLLYSYS_URL)
        .spawn()
        .context("无法启动 Edge/Chrome 登录窗口")?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn import_external_login_cookies() -> Result<Vec<CookieEntry>> {
    let tabs = fetch_devtools_tabs()?;
    let Some(websocket_url) = tabs
        .iter()
        .filter_map(|tab| tab.get("webSocketDebuggerUrl").and_then(Value::as_str))
        .next()
    else {
        bail!("未找到登录浏览器调试页面，请先点击“登录系统”并完成浏览器登录");
    };

    let mut socket = tungstenite::connect(websocket_url)
        .context("无法连接登录浏览器，请保持刚打开的浏览器窗口不要关闭")?
        .0;
    socket
        .send(tungstenite::Message::Text(
            json!({"id": 1, "method": "Network.getAllCookies"}).to_string().into(),
        ))
        .context("无法读取登录浏览器 Cookie")?;

    loop {
        let message = socket.read().context("读取登录浏览器 Cookie 失败")?;
        let Ok(text) = message.to_text() else {
            continue;
        };
        let payload: Value = serde_json::from_str(text).context("浏览器 Cookie 响应不是 JSON")?;
        if payload.get("id").and_then(Value::as_i64) != Some(1) {
            continue;
        }
        if let Some(error) = payload.get("error") {
            bail!("登录浏览器拒绝读取 Cookie: {error}");
        }
        let cookies = payload
            .get("result")
            .and_then(|result| result.get("cookies"))
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("登录浏览器未返回 Cookie"))?;
        let entries = cookies
            .iter()
            .filter_map(cookie_value_to_entry)
            .filter(|cookie| cookie_domain_matches(&cookie.domain, "www.hollysys.net"))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            bail!("浏览器里还没有 Hollysys 登录 Cookie，请先在打开的浏览器窗口完成登录");
        }
        return Ok(entries);
    }
}

#[cfg(target_os = "windows")]
fn fetch_devtools_tabs() -> Result<Vec<Value>> {
    let url = format!("http://127.0.0.1:{DEVTOOLS_PORT}/json");
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?
        .get(url)
        .send()
        .context("登录浏览器尚未启动或调试端口未就绪")?
        .error_for_status()?
        .json::<Vec<Value>>()?;
    Ok(response)
}

#[cfg(target_os = "windows")]
fn cookie_value_to_entry(value: &Value) -> Option<CookieEntry> {
    let name = value.get("name")?.as_str()?.to_string();
    let cookie_value = value.get("value")?.as_str()?.to_string();
    if name.is_empty() || cookie_value.is_empty() {
        return None;
    }
    let domain = value.get("domain")?.as_str()?.to_string();
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("/")
        .to_string();
    Some(CookieEntry {
        name,
        value: cookie_value,
        domain,
        path,
    })
}

#[cfg(any(target_os = "windows", test))]
fn cookie_domain_matches(cookie_domain: &str, request_host: &str) -> bool {
    let stripped = cookie_domain.trim_start_matches('.');
    if stripped.is_empty() {
        return false;
    }
    request_host == stripped || request_host.ends_with(&format!(".{stripped}"))
}

#[cfg(target_os = "windows")]
fn external_login_profile_dir() -> PathBuf {
    runtime_root().join("external-login-browser")
}

#[cfg(target_os = "windows")]
fn find_browser_executable() -> Option<PathBuf> {
    browser_candidates().into_iter().find(|path| path.exists())
}

#[cfg(target_os = "windows")]
fn browser_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for var in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            let base = PathBuf::from(base);
            candidates.push(base.join("Microsoft/Edge/Application/msedge.exe"));
            candidates.push(base.join("Google/Chrome/Application/chrome.exe"));
        }
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::cookie_domain_matches;

    #[test]
    fn matches_hollysys_cookie_domains() {
        assert!(cookie_domain_matches("www.hollysys.net", "www.hollysys.net"));
        assert!(cookie_domain_matches(".hollysys.net", "www.hollysys.net"));
        assert!(!cookie_domain_matches("sso.example.com", "www.hollysys.net"));
    }
}
