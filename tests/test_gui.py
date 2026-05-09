from __future__ import annotations

from pathlib import Path

import src.gui as gui_module
from src.config_store import AISettings, AppSettings
from src.gui import WebviewApi, build_app_html, run_gui_app
from src.hollysys_batch_download import SessionInspectionResult
from src.models import BatchWorkflowResult, WebPhaseResult, WorkflowResult

FIXED_LOG_TIME = "09:30:00"


def build_api(tmp_path: Path, **kwargs) -> WebviewApi:
    kwargs.setdefault("settings_path", tmp_path / "config" / "settings.json")
    return WebviewApi(**kwargs)


def ready_session_result() -> SessionInspectionResult:
    return SessionInspectionResult(
        status="ready",
        detail="已拿到可用 Hollysys 会话，可直接访问待办事宜。",
        cookie_db_exists=True,
        cookie_db_path="/Users/test/Library/Application Support/Google/Chrome/Default/Cookies",
        hollysys_cookie_count=3,
        cookie_names=("JSESSIONID", "HLSID", "sid"),
        safe_storage_available=True,
        authenticated=True,
        http_status=200,
        final_url="https://www.hollysys.net/sys/aggregation/",
    )


def missing_session_result() -> SessionInspectionResult:
    return SessionInspectionResult(
        status="missing",
        detail="已找到 Chrome，但未发现 Hollysys 相关 Cookie。",
        cookie_db_exists=True,
        cookie_db_path="/Users/test/Library/Application Support/Google/Chrome/Default/Cookies",
        hollysys_cookie_count=0,
        cookie_names=(),
        safe_storage_available=False,
        authenticated=False,
        http_status=0,
        final_url="",
    )


class FakeWindow:
    def __init__(self, dialog_result=None) -> None:
        self.dialog_result = dialog_result
        self.dialog_calls: list[tuple[int, str]] = []
        self.js_calls: list[str] = []

    def create_file_dialog(self, dialog_type: int, directory: str = "", **kwargs):
        self.dialog_calls.append((dialog_type, directory))
        return self.dialog_result

    def evaluate_js(self, script: str):
        self.js_calls.append(script)
        return None


def with_time(message: str) -> str:
    return f"{FIXED_LOG_TIME} {message}"


def test_build_app_html_contains_core_sections():
    html = build_app_html()

    assert "项目资料比对助手" in html
    assert "运行日志" in html
    assert "themeToggleButton" in html
    assert "project-file-compare-theme" in html
    assert "pywebviewready" in html
    assert "/*__APP_CSS__*/" not in html
    assert "/*__APP_JS__*/" not in html


def test_build_app_html_prefers_frozen_bundle_assets(monkeypatch, tmp_path: Path):
    bundle_index = tmp_path / "src" / "webui" / "index.html"
    bundle_index.parent.mkdir(parents=True, exist_ok=True)
    bundle_index.write_text("<html>bundled ui</html>", encoding="utf-8")
    monkeypatch.setattr(gui_module.sys, "_MEIPASS", str(tmp_path), raising=False)

    html = build_app_html()

    assert html == "<html>bundled ui</html>"


def test_build_app_html_inlines_bundle_assets(monkeypatch, tmp_path: Path):
    bundle_dir = tmp_path / "src" / "webui"
    bundle_dir.mkdir(parents=True, exist_ok=True)
    (bundle_dir / "index.html").write_text(
        "<style>/*__APP_CSS__*/</style><script>/*__APP_JS__*/</script>",
        encoding="utf-8",
    )
    (bundle_dir / "app.css").write_text("body { color: red; }", encoding="utf-8")
    (bundle_dir / "app.js").write_text("window.inlineTest = true;", encoding="utf-8")
    monkeypatch.setattr(gui_module.sys, "_MEIPASS", str(tmp_path), raising=False)

    html = build_app_html()

    assert "body { color: red; }" in html
    assert "window.inlineTest = true;" in html


def test_detect_webview2_version_hides_windows_console(monkeypatch):
    class FakeStartupInfo:
        def __init__(self) -> None:
            self.dwFlags = 0
            self.wShowWindow = None

    calls = []

    def fake_run(*args, **kwargs):
        calls.append(kwargs)

        class Result:
            returncode = 1
            stdout = ""

        return Result()

    monkeypatch.setattr(gui_module.sys, "platform", "win32")
    monkeypatch.setattr(gui_module.subprocess, "STARTUPINFO", FakeStartupInfo, raising=False)
    monkeypatch.setattr(gui_module.subprocess, "STARTF_USESHOWWINDOW", 1, raising=False)
    monkeypatch.setattr(gui_module.subprocess, "SW_HIDE", 0, raising=False)
    monkeypatch.setattr(gui_module.subprocess, "CREATE_NO_WINDOW", 0x08000000, raising=False)
    monkeypatch.setattr(gui_module.subprocess, "run", fake_run)

    version = gui_module._detect_webview2_version()

    assert version == ""
    assert calls
    assert calls[0]["creationflags"] == 0x08000000
    assert isinstance(calls[0]["startupinfo"], FakeStartupInfo)
    assert calls[0]["startupinfo"].dwFlags & 1
    assert calls[0]["startupinfo"].wShowWindow == 0


def test_webview_api_bootstrap_returns_ready_state(monkeypatch, tmp_path: Path):
    startup_checks = [
        {"label": "桌面内核", "status": "已就绪", "tone": "success", "detail": "macOS | pywebview 6.2.1"},
        {"label": "运行目录", "status": "可写", "tone": "success", "detail": str(tmp_path)},
    ]
    monkeypatch.setattr(gui_module, "collect_startup_checks", lambda **kwargs: startup_checks)
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    api.desktop_runtime_check = startup_checks[0]
    api.runtime_directory_check = startup_checks[1]
    api.file_root_check = {"label": "资料目录", "status": "可写", "tone": "success", "detail": str(tmp_path / "files")}

    state = api.bootstrap()

    assert state["summary"]["directory"] == str(tmp_path)
    assert state["status"]["text"] == "待执行"
    assert state["session"]["badge"]["text"] == "会话可用"
    assert state["session"]["browser"] == "Chrome 本机会话可读 | Hollysys 已认证"
    assert state["startupChecks"] == startup_checks
    assert state["logs"] == [
        with_time("[启动] 自动检测当前环境和会话"),
        with_time("[环境] 桌面内核: 已就绪 | macOS | pywebview 6.2.1"),
        with_time(f"[自检] 运行目录: 可写 | {tmp_path}"),
        with_time(f"[自检] 资料目录: 可写 | {tmp_path / 'files'}"),
        with_time("[会话] 已拿到可用 Hollysys 会话，可直接访问待办事宜。"),
    ]

    state = api.bootstrap()

    assert state["logs"].count(with_time("[启动] 自动检测当前环境和会话")) == 1


def test_webview_api_bootstrap_can_defer_startup_checks(monkeypatch, tmp_path: Path):
    started = []

    class FakeThread:
        def __init__(self, *, target=None, args=(), daemon=None):
            self.target = target
            self.args = args
            self.daemon = daemon

        def start(self):
            started.append((self.target, self.args, self.daemon))

    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: (_ for _ in ()).throw(AssertionError("startup should not block bootstrap")),
        log_time_provider=lambda: FIXED_LOG_TIME,
        startup_in_background=True,
    )
    monkeypatch.setattr(gui_module.threading, "Thread", FakeThread)

    state = api.bootstrap()

    assert state["status"]["text"] == "启动检测中"
    assert state["status"]["tone"] == "running"
    assert state["startupLoading"] is True
    assert state["busy"]["active"] is True
    assert state["busy"]["kind"] == "startup"
    assert state["busy"]["title"] == "正在后台检测环境和会话"
    assert state["logs"] == [with_time("[启动] 页面已打开，后台检测环境和会话")]
    assert len(started) == 1
    assert started[0][0] == api._run_startup_probe


def test_webview_api_choose_file_root_uses_window_dialog(tmp_path: Path):
    chosen = str(tmp_path / "chosen")
    window = FakeWindow(dialog_result=[chosen])
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    api.attach_window(window)

    result = api.choose_file_root()

    assert result["selected"] == chosen
    assert window.dialog_calls == [(gui_module.webview.FileDialog.FOLDER, str(tmp_path))]


def test_webview_api_refresh_session_can_defer_probe(monkeypatch, tmp_path: Path):
    started = []

    class FakeThread:
        def __init__(self, *, target=None, args=(), daemon=None):
            self.target = target
            self.args = args
            self.daemon = daemon

        def start(self):
            started.append((self.target, self.args, self.daemon))

    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: (_ for _ in ()).throw(AssertionError("refresh should not block api call")),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    monkeypatch.setattr(gui_module.threading, "Thread", FakeThread)

    state = api.refresh_session()

    assert state["status"]["text"] == "刷新会话中"
    assert state["busy"]["active"] is True
    assert state["busy"]["kind"] == "session"
    assert state["logs"] == [with_time("[会话] 开始刷新 Hollysys 会话")]
    assert len(started) == 1
    assert started[0][0] == api._run_refresh_session_sync


def test_webview_api_save_settings_persists_before_returning(tmp_path: Path):
    saved = {}

    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(username="old-user", password="old-pass", last_file_root=str(tmp_path)),
        settings_saver=lambda path, username, password, last_file_root, ai_settings=None: saved.update(
            {
                "path": path,
                "username": username,
                "password": password,
                "last_file_root": last_file_root,
                "ai_settings": ai_settings,
            }
        ),
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    state = api.save_settings(
        {
            "lastFileRoot": str(tmp_path / "new-root"),
            "aiEnabled": True,
            "aiBaseUrl": "https://example.com/ai",
            "aiApiKey": "secret-ai-key",
            "aiModel": "vision-model",
            "ocrBaseUrl": "https://example.com/ocr",
            "ocrApiKey": "secret-ocr-key",
            "requestTimeoutSeconds": "45",
            "imageMaxKb": "96",
        }
    )

    assert api.settings.username == "old-user"
    assert state["settings"]["lastFileRoot"] == str(tmp_path / "new-root")
    assert state["settings"]["requestTimeoutSeconds"] == 45
    assert state["busy"]["active"] is False
    assert saved["username"] == "old-user"
    assert saved["password"] == "old-pass"
    assert saved["last_file_root"] == str(tmp_path / "new-root")
    assert saved["ai_settings"].enabled is True
    assert saved["ai_settings"].ai_model == "vision-model"
    assert with_time("[设置] 已更新") in api.logs


def test_webview_api_saved_settings_survive_new_instance(tmp_path: Path):
    settings_path = tmp_path / "config" / "settings.json"
    api = build_api(tmp_path,
        settings_path=settings_path,
        settings_loader=gui_module.load_settings,
        settings_saver=gui_module.save_settings,
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    api.save_settings(
        {
            "lastFileRoot": str(tmp_path / "data"),
            "aiEnabled": True,
            "aiBaseUrl": "https://example.com/ai",
            "aiApiKey": "secret-ai-key",
            "aiModel": "vision-model",
            "ocrBaseUrl": "https://example.com/ocr",
            "ocrApiKey": "secret-ocr-key",
            "requestTimeoutSeconds": "45",
            "imageMaxKb": "96",
        }
    )

    restarted = build_api(tmp_path,
        settings_path=settings_path,
        settings_loader=gui_module.load_settings,
        settings_saver=gui_module.save_settings,
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    assert restarted.settings.last_file_root == str(tmp_path / "data")
    assert restarted.settings.ai.enabled is True
    assert restarted.settings.ai.ai_base_url == "https://example.com/ai"
    assert restarted.settings.ai.ai_api_key == "secret-ai-key"
    assert restarted.settings.ai.ai_model == "vision-model"
    assert restarted.settings.ai.ocr_base_url == "https://example.com/ocr"
    assert restarted.settings.ai.ocr_api_key == "secret-ocr-key"
    assert restarted.settings.ai.request_timeout_seconds == 45
    assert restarted.settings.ai.image_max_kb == 96


def test_webview_api_loads_latest_fixed_result_logs_on_startup(tmp_path: Path):
    file_root = tmp_path / "file"
    result_dir = file_root / "result_logs"
    result_dir.mkdir(parents=True)
    settings_path = tmp_path / "config" / "settings.json"
    gui_module.save_settings(
        settings_path,
        username="",
        password="",
        last_file_root=str(file_root),
    )
    (result_dir / "success.log").write_text("\n".join(f"success-{index:02d}" for index in range(25)) + "\n", encoding="utf-8")
    (result_dir / "error.log").write_text("\n".join(f"error-{index:02d}" for index in range(25)) + "\n", encoding="utf-8")

    api = build_api(tmp_path,
        settings_path=settings_path,
        settings_loader=gui_module.load_settings,
        settings_saver=gui_module.save_settings,
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    outputs = api._build_outputs_state()

    assert outputs["successLogPath"] == str(result_dir / "success.log")
    assert outputs["errorLogPath"] == str(result_dir / "error.log")
    assert outputs["successProjectCodes"] == [f"success-{index:02d}" for index in range(5, 25)]
    assert outputs["errorProjectCodes"] == [f"error-{index:02d}" for index in range(5, 25)]


def test_webview_api_outputs_current_project_count(tmp_path: Path):
    file_root = tmp_path / "file"
    (file_root / "project" / "BHE-25030367-01").mkdir(parents=True)
    (file_root / "project" / "BHE-25030368-01").mkdir(parents=True)
    (file_root / "project" / "readme.txt").write_text("skip", encoding="utf-8")
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(file_root)),
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    outputs = api._build_outputs_state()

    assert outputs["projectRoot"] == str(file_root / "project")
    assert outputs["projectCount"] == 2
    assert outputs["downloadedProjectNames"] == ["BHE-25030367-01", "BHE-25030368-01"]


def test_webview_api_outputs_default_success_workbook_path(tmp_path: Path):
    file_root = tmp_path / "file"
    success_path = file_root / "success" / "2026年关闭满意度回访表0331.xlsx"
    success_path.parent.mkdir(parents=True)
    success_path.write_bytes(b"demo")
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(file_root)),
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    outputs = api._build_outputs_state()

    assert outputs["successWorkbookPath"] == str(success_path)
    assert outputs["successWorkbookExists"] is True


def test_webview_api_open_path_creates_empty_fixed_log_before_opening(tmp_path: Path):
    opened: list[Path] = []
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path / "file")),
        session_inspector=lambda **kwargs: ready_session_result(),
        open_path=opened.append,
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    log_path = tmp_path / "file" / "result_logs" / "success.log"

    assert api.open_path(str(log_path)) is True

    assert log_path.exists()
    assert opened == [log_path]


def test_webview_api_can_clear_fixed_result_logs(tmp_path: Path):
    file_root = tmp_path / "file"
    result_dir = file_root / "result_logs"
    result_dir.mkdir(parents=True)
    success_path = result_dir / "success.log"
    error_path = result_dir / "error.log"
    success_path.write_text("success-01\n", encoding="utf-8")
    error_path.write_text("error-01\n", encoding="utf-8")
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(file_root)),
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    success_state = api.clear_success_log()
    error_state = api.clear_error_log()

    assert success_path.read_text(encoding="utf-8") == ""
    assert error_path.read_text(encoding="utf-8") == ""
    assert success_state["outputs"]["successProjectCodes"] == []
    assert error_state["outputs"]["errorProjectCodes"] == []
    assert success_state["outputs"]["successCount"] == 0
    assert error_state["outputs"]["failedCount"] == 0


def test_webview_api_migrates_legacy_project_settings(monkeypatch, tmp_path: Path):
    legacy_path = tmp_path / "legacy" / "config" / "settings.json"
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(
        """
{
  "username": "",
  "password": "",
  "last_file_root": "/old/file",
  "ai": {
    "enabled": true,
    "ai_base_url": "https://example.com/ai",
    "ai_api_key": "secret",
    "ai_model": "vision-model",
    "ocr_base_url": "",
    "ocr_api_key": "",
    "request_timeout_seconds": 45,
    "image_max_kb": 96
  }
}
        """.strip(),
        encoding="utf-8",
    )
    settings_path = tmp_path / "new" / "config" / "settings.json"
    monkeypatch.setattr(gui_module, "LEGACY_SETTINGS_PATH", legacy_path)

    api = WebviewApi(
        settings_path=settings_path,
        settings_loader=gui_module.load_settings,
        settings_saver=gui_module.save_settings,
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    assert settings_path.exists()
    assert api.settings.last_file_root == "/old/file"
    assert api.settings.ai.enabled is True
    assert api.settings.ai.ai_model == "vision-model"


def test_webview_api_repairs_pytest_polluted_settings_from_legacy(monkeypatch, tmp_path: Path):
    legacy_file_root = tmp_path / "real" / "file"
    legacy_file_root.mkdir(parents=True)
    legacy_path = tmp_path / "legacy" / "config" / "settings.json"
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(
        f"""
{{
  "username": "",
  "password": "",
  "last_file_root": "{legacy_file_root}",
  "ai": {{
    "enabled": true,
    "ai_base_url": "https://example.com/ai",
    "ai_api_key": "legacy-secret",
    "ai_model": "vision-model",
    "ocr_base_url": "",
    "ocr_api_key": "",
    "request_timeout_seconds": 45,
    "image_max_kb": 96
  }}
}}
        """.strip(),
        encoding="utf-8",
    )
    settings_path = tmp_path / "new" / "config" / "settings.json"
    settings_path.parent.mkdir(parents=True)
    settings_path.write_text(
        """
{
  "username": "",
  "password": "",
  "last_file_root": "/private/var/folders/test/pytest-of-fxy/pytest-65/stale",
  "ai": {
    "enabled": false,
    "ai_base_url": "https://ark.cn-beijing.volces.com/api/v3",
    "ai_api_key": "",
    "ai_model": "doubao-seed-2-0-lite-260215",
    "ocr_base_url": "",
    "ocr_api_key": "",
    "request_timeout_seconds": 30,
    "image_max_kb": 100
  }
}
        """.strip(),
        encoding="utf-8",
    )
    monkeypatch.setattr(gui_module, "LEGACY_SETTINGS_PATH", legacy_path)
    monkeypatch.setattr(gui_module, "SETTINGS_PATH", settings_path)

    api = WebviewApi(
        settings_path=settings_path,
        settings_loader=gui_module.load_settings,
        settings_saver=gui_module.save_settings,
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    assert api.settings.last_file_root == str(legacy_file_root)
    assert api.settings.ai.enabled is True
    assert api.settings.ai.ai_api_key == "legacy-secret"


def test_webview_api_handle_start_stop_requests_stop_when_running(tmp_path: Path):
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    api.active_task_name = "batch"

    state = api.handle_start_stop()

    assert state["status"]["text"] == "等待当前步骤结束"
    assert state["status"]["tone"] == "warning"
    assert with_time("[运行] 当前任务不支持立即停止，请等待当前步骤结束") in state["logs"]


def test_webview_api_run_download_only_can_defer_preflight(monkeypatch, tmp_path: Path):
    started = []

    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: (_ for _ in ()).throw(AssertionError("download preflight should not block api call")),
        download_runner=lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("download runner should not be called")),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    monkeypatch.setattr(api, "_start_action_worker", lambda action: started.append(action))

    state = api.run_download_only()

    assert state["status"]["text"] == "下载准备中"
    assert state["running"] is True
    assert state["busy"]["active"] is True
    assert state["busy"]["kind"] == "download"
    assert with_time("[运行] 已接收下载任务，正在后台准备") in state["logs"]
    assert len(started) == 1
    assert started[0] == "download"


def test_webview_api_start_batch_can_defer_preflight(monkeypatch, tmp_path: Path):
    started = []

    monkeypatch.setattr(gui_module, "is_remote_recognition_configured", lambda settings: False)
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path), ai=AISettings(enabled=True)),
        session_inspector=lambda **kwargs: (_ for _ in ()).throw(AssertionError("batch preflight should not block api call")),
        batch_runner=lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("batch runner should not be called")),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    monkeypatch.setattr(api, "_start_action_worker", lambda action: started.append(action))

    state = api.start_batch()

    assert state["status"]["text"] == "批处理准备中"
    assert state["running"] is True
    assert state["busy"]["active"] is True
    assert state["busy"]["kind"] == "batch"
    assert with_time("[运行] 已接收批处理任务，正在后台准备") in state["logs"]
    assert len(started) == 1
    assert started[0] == "batch"


def test_webview_api_run_compare_only_starts_worker_process(monkeypatch, tmp_path: Path):
    started = []

    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    monkeypatch.setattr(api, "_start_action_worker", lambda action: started.append(action))

    state = api.run_compare_only()

    assert state["status"]["text"] == "比对准备中"
    assert state["running"] is True
    assert state["busy"]["active"] is True
    assert state["busy"]["kind"] == "compare"
    assert with_time("[运行] 已接收本地比对任务，正在后台准备") in state["logs"]
    assert started == ["compare"]


def test_webview_api_start_action_worker_uses_process_and_monitor_thread(monkeypatch, tmp_path: Path):
    created = {}
    thread_started = []

    class FakeQueue:
        pass

    class FakeProcess:
        def __init__(self, *, target=None, kwargs=None, daemon=None):
            self.target = target
            self.kwargs = kwargs or {}
            self.daemon = daemon
            self.started = False

        def start(self):
            self.started = True

        def is_alive(self):
            return False

        def join(self, timeout=None):
            return None

        @property
        def exitcode(self):
            return 0

    class FakeContext:
        def Queue(self):
            created["queue"] = FakeQueue()
            return created["queue"]

        def Process(self, *, target=None, kwargs=None, daemon=None):
            process = FakeProcess(target=target, kwargs=kwargs, daemon=daemon)
            created["process"] = process
            return process

    class FakeThread:
        def __init__(self, *, target=None, args=(), daemon=None):
            self.target = target
            self.args = args
            self.daemon = daemon

        def start(self):
            thread_started.append((self.target, self.args, self.daemon))

    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: ready_session_result(),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    monkeypatch.setattr(gui_module.multiprocessing, "get_context", lambda method: FakeContext())
    monkeypatch.setattr(gui_module.threading, "Thread", FakeThread)

    api._start_action_worker("compare")

    assert created["process"].target == gui_module.run_action_worker_process
    assert created["process"].kwargs["action"] == "compare"
    assert created["process"].kwargs["file_root"] == tmp_path
    assert created["process"].kwargs["processed_projects_path"] == gui_module.PROCESSED_PROJECTS_PATH
    assert created["process"].kwargs["worker_queue"] is created["queue"]
    assert created["process"].daemon is True
    assert created["process"].started is True
    assert api.action_process is created["process"]
    assert len(thread_started) == 1
    assert thread_started[0][0] == api._monitor_action_worker


def test_webview_api_run_action_sync_download_sets_warning_when_session_missing(tmp_path: Path):
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: missing_session_result(),
        download_runner=lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("download runner should not be called")),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    api.active_task_name = "download"
    api._set_busy("download", "正在准备下载任务", "先检查会话，再开始抓取 Hollysys 附件")

    api._run_action_sync("download")

    assert api.status_text == "下载前需先登录"
    assert api.active_task_name == ""
    assert api.busy_operation == ""
    assert with_time("[会话] 已找到 Chrome，但未发现 Hollysys 相关 Cookie。") in api.logs


def test_webview_api_run_action_sync_batch_requires_remote_recognition(monkeypatch, tmp_path: Path):
    monkeypatch.setattr(gui_module, "is_remote_recognition_configured", lambda settings: False)
    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path), ai=AISettings(enabled=True)),
        session_inspector=lambda **kwargs: ready_session_result(),
        batch_runner=lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("batch runner should not be called")),
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    api.active_task_name = "batch"
    api._set_busy("batch", "正在准备批处理", "先检查会话和识别配置，再进入下载 / 比对 / 清理")

    api._run_action_sync("batch")

    assert api.status_text == "需配置 AI/OCR"
    assert api.active_task_name == ""
    assert api.busy_operation == ""
    assert with_time("[运行] 未配置 AI 或 OCR，请先在设置中完成至少一种识别配置") in api.logs


def test_webview_api_run_action_sync_download_updates_logs_and_status(tmp_path: Path):
    window = FakeWindow()

    def fake_run_download_workflow(file_root: Path, username: str, password: str, log_callback=None, processed_projects_path=None):
        assert file_root == tmp_path
        if log_callback is not None:
            log_callback("[网页阶段] 已下载项目: BHE-25030367-01 | 文件=3/3")
        return WebPhaseResult(processed_projects=["BHE-25030367-01"], skipped_projects=["BHE-25030366-01"])

    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: ready_session_result(),
        download_runner=fake_run_download_workflow,
        log_time_provider=lambda: FIXED_LOG_TIME,
    )
    api.attach_window(window)
    api.active_task_name = "download"

    api._run_action_sync("download")

    assert api.status_text == "下载完成"
    assert api.active_task_name == ""
    assert with_time("[网页阶段] 已下载项目: BHE-25030367-01 | 文件=3/3") in api.logs
    assert with_time("[运行] 下载完成: 1 | 跳过: 1") in api.logs
    assert api.output_summary["successProjectCodes"] == []
    assert api.output_summary["errorProjectCodes"] == []
    assert api.output_summary["resultLogPath"] == ""
    assert api.output_summary["successLogPath"] == ""
    assert api.output_summary["errorLogPath"] == ""
    assert api.output_summary["successCount"] == 0
    assert api.output_summary["duplicateCount"] == 0
    assert not (tmp_path / "result_logs").exists()
    assert window.js_calls


def test_webview_api_run_action_sync_compare_updates_logs_and_status(tmp_path: Path):
    def fake_run_compare_workflow(file_root: Path, username: str, password: str, log_callback=None, ai_settings=None):
        assert file_root == tmp_path
        if log_callback is not None:
            log_callback("[运行] 本地比对完成")
        return WorkflowResult(
            appended_count=2,
            duplicate_count=1,
            failed_count=0,
            log_path=tmp_path / "error" / "logs" / "workflow.txt",
            success_log_path=tmp_path / "result_logs" / "success.log",
            error_log_path=tmp_path / "result_logs" / "error.log",
            success_project_codes=["BHE-25030367/01", "BHE-25030368/01"],
            error_project_codes=["BHE-25030369/01"],
            success_workbook_path=tmp_path / "success" / "2026年关闭满意度回访表0331.xlsx",
            error_report_paths=[],
        )

    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: ready_session_result(),
        compare_runner=fake_run_compare_workflow,
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    api._run_action_sync("compare")

    assert api.status_text == "比对完成"
    assert with_time("[运行] 本地比对完成") in api.logs
    assert with_time("[运行] 本地比对完成: 追加成功=2 | 重复跳过=1 | 失败=0") in api.logs
    assert api.output_summary["successWorkbookPath"] == str(tmp_path / "success" / "2026年关闭满意度回访表0331.xlsx")
    assert api.output_summary["successProjectCodes"] == ["BHE-25030367/01", "BHE-25030368/01"]
    assert api.output_summary["errorProjectCodes"] == ["BHE-25030369/01"]
    assert api.output_summary["errorReportPaths"] == []
    assert api.output_summary["logPath"] == str(tmp_path / "error" / "logs" / "workflow.txt")
    assert api.output_summary["successLogPath"] == str(tmp_path / "result_logs" / "success.log")
    assert api.output_summary["errorLogPath"] == str(tmp_path / "result_logs" / "error.log")


def test_webview_api_run_action_sync_batch_updates_logs_and_status(tmp_path: Path):
    def fake_run_batch_workflow(file_root: Path, username: str, password: str, log_callback=None, ai_settings=None):
        assert file_root == tmp_path
        if log_callback is not None:
            log_callback("[网页阶段] 已下载项目: BHE-25030367-01 | 文件=3/3")
        return BatchWorkflowResult(
            web_processed_count=1,
            compare_appended_count=1,
            compare_duplicate_count=0,
            compare_failed_count=0,
            cleaned_count=1,
            log_path=tmp_path / "error" / "logs" / "workflow.txt",
            success_log_path=tmp_path / "result_logs" / "success.log",
            error_log_path=tmp_path / "result_logs" / "error.log",
            compare_success_project_codes=["BHE-25030367/01"],
            compare_error_project_codes=["BHE-25030370/01"],
            compare_success_workbook_path=tmp_path / "success" / "2026年关闭满意度回访表0331.xlsx",
            compare_error_report_paths=[],
        )

    api = build_api(tmp_path,
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: ready_session_result(),
        batch_runner=fake_run_batch_workflow,
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    api._run_action_sync("batch")

    assert api.status_text == "批处理完成"
    assert with_time("[运行] 批处理完成: 下载=1 | 追加成功=1 | 重复跳过=0 | 失败=0 | 清理成功=1") in api.logs
    assert api.output_summary["successWorkbookPath"] == str(tmp_path / "success" / "2026年关闭满意度回访表0331.xlsx")
    assert api.output_summary["successProjectCodes"] == ["BHE-25030367/01"]
    assert api.output_summary["errorProjectCodes"] == ["BHE-25030370/01"]
    assert api.output_summary["errorReportPaths"] == []
    assert api.output_summary["successLogPath"] == str(tmp_path / "result_logs" / "success.log")
    assert api.output_summary["errorLogPath"] == str(tmp_path / "result_logs" / "error.log")


def test_run_gui_app_creates_window_and_starts_webview(monkeypatch, tmp_path: Path):
    created = {}
    fake_window = object()

    def fake_create_window(title, **kwargs):
        created["title"] = title
        created["kwargs"] = kwargs
        return fake_window

    def fake_start(**kwargs):
        created["start_kwargs"] = kwargs

    monkeypatch.setattr(gui_module.webview, "create_window", fake_create_window)
    monkeypatch.setattr(gui_module.webview, "start", fake_start)
    monkeypatch.setattr(gui_module, "SETTINGS_PATH", tmp_path / "config" / "settings.json")
    monkeypatch.setattr(gui_module, "PROCESSED_PROJECTS_PATH", tmp_path / "config" / "processed_projects.json")

    exit_code = run_gui_app()

    assert exit_code == 0
    assert created["title"] == gui_module.WINDOW_TITLE
    assert created["kwargs"]["width"] == gui_module.WINDOW_WIDTH
    assert created["kwargs"]["height"] == gui_module.WINDOW_HEIGHT
    assert created["kwargs"]["resizable"] is False
    assert created["kwargs"]["min_size"] == gui_module.WINDOW_MIN_SIZE
    assert created["kwargs"]["html"] == build_app_html()
    assert isinstance(created["kwargs"]["js_api"], WebviewApi)
    assert created["kwargs"]["js_api"].window is fake_window
    assert created["start_kwargs"] == {"debug": False}
