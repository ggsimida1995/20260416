from __future__ import annotations

from pathlib import Path

import src.gui as gui_module
from src.config_store import AISettings, AppSettings
from src.gui import WebviewApi, build_app_html, run_gui_app
from src.hollysys_batch_download import SessionInspectionResult
from src.models import BatchWorkflowResult, WebPhaseResult, WorkflowResult

FIXED_LOG_TIME = "09:30:00"


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

    assert "Hollysys 批处理" in html
    assert "运行日志" in html
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
    api = WebviewApi(
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

    api = WebviewApi(
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
    api = WebviewApi(
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

    api = WebviewApi(
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


def test_webview_api_save_settings_can_defer_persist(monkeypatch, tmp_path: Path):
    saved = {}
    started = []

    class FakeThread:
        def __init__(self, *, target=None, args=(), daemon=None):
            self.target = target
            self.args = args
            self.daemon = daemon

        def start(self):
            started.append((self.target, self.args, self.daemon))

    api = WebviewApi(
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
    monkeypatch.setattr(gui_module.threading, "Thread", FakeThread)

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

    assert saved == {}
    assert state["busy"]["active"] is True
    assert state["busy"]["kind"] == "settings"
    assert api.settings.username == "old-user"
    assert state["settings"]["lastFileRoot"] == str(tmp_path / "new-root")
    assert state["settings"]["requestTimeoutSeconds"] == 45
    assert len(started) == 1
    assert started[0][0] == api._run_save_settings_sync

    api._run_save_settings_sync()

    assert saved["username"] == "old-user"
    assert saved["password"] == "old-pass"
    assert saved["last_file_root"] == str(tmp_path / "new-root")
    assert saved["ai_settings"].enabled is True
    assert saved["ai_settings"].ai_model == "vision-model"
    assert with_time("[设置] 已更新") in api.logs


def test_webview_api_handle_start_stop_requests_stop_when_running(tmp_path: Path):
    api = WebviewApi(
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

    api = WebviewApi(
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
    api = WebviewApi(
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

    api = WebviewApi(
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

    api = WebviewApi(
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
    api = WebviewApi(
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
    api = WebviewApi(
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

    api = WebviewApi(
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
            success_workbook_path=tmp_path / "success" / "2026年关闭满意度回访表0331.xlsx",
            error_report_paths=[tmp_path / "error" / "项目A.txt"],
        )

    api = WebviewApi(
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
    assert api.output_summary["errorReportPaths"] == [str(tmp_path / "error" / "项目A.txt")]
    assert api.output_summary["logPath"] == str(tmp_path / "error" / "logs" / "workflow.txt")


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
            compare_success_workbook_path=tmp_path / "success" / "2026年关闭满意度回访表0331.xlsx",
            compare_error_report_paths=[tmp_path / "error" / "项目B.txt"],
        )

    api = WebviewApi(
        settings_loader=lambda path: AppSettings(last_file_root=str(tmp_path)),
        session_inspector=lambda **kwargs: ready_session_result(),
        batch_runner=fake_run_batch_workflow,
        log_time_provider=lambda: FIXED_LOG_TIME,
    )

    api._run_action_sync("batch")

    assert api.status_text == "批处理完成"
    assert with_time("[运行] 批处理完成: 下载=1 | 追加成功=1 | 重复跳过=0 | 失败=0 | 清理成功=1") in api.logs
    assert api.output_summary["successWorkbookPath"] == str(tmp_path / "success" / "2026年关闭满意度回访表0331.xlsx")
    assert api.output_summary["errorReportPaths"] == [str(tmp_path / "error" / "项目B.txt")]


def test_run_gui_app_creates_window_and_starts_webview(monkeypatch):
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
