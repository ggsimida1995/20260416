from __future__ import annotations

from datetime import datetime
import json
import multiprocessing
import os
import queue
import subprocess
import sys
import threading
from importlib import metadata
from pathlib import Path
from typing import Any, Callable

import webview

from src.config import APP_RUNTIME_ROOT, FILE_ROOT, PROCESSED_PROJECTS_PATH, SETTINGS_PATH
from src.config_store import AISettings, AppSettings, load_settings, save_settings
from src.hollysys_batch_download import SessionInspectionResult, inspect_local_hollysys_session
from src.models import BatchWorkflowResult, WebPhaseResult, WorkflowResult
from src.readers.signature_ai import is_remote_recognition_configured
from src.workflow import run_batch_workflow, run_compare_workflow, run_download_workflow

WINDOW_TITLE = "Project File Compare"
WINDOW_WIDTH = 1080
WINDOW_HEIGHT = 760
WINDOW_MIN_SIZE = (WINDOW_WIDTH, WINDOW_HEIGHT)
DEFAULT_SESSION_TIMEOUT_SECONDS = 4.0
LOG_LIMIT = 500
WEBUI_DIR = Path(__file__).with_name("webui")
WEBVIEW2_CLIENT_ID = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"


def _platform_label() -> str:
    return {
        "win32": "Windows",
        "darwin": "macOS",
        "linux": "Linux",
    }.get(sys.platform, sys.platform)


def _pywebview_version() -> str:
    try:
        return metadata.version("pywebview")
    except metadata.PackageNotFoundError:
        return ""


def _hidden_subprocess_kwargs() -> dict[str, Any]:
    if sys.platform != "win32":
        return {}

    startupinfo = subprocess.STARTUPINFO()
    startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startupinfo.wShowWindow = getattr(subprocess, "SW_HIDE", 0)
    return {
        "startupinfo": startupinfo,
        "creationflags": getattr(subprocess, "CREATE_NO_WINDOW", 0),
    }


def _detect_webview2_version() -> str:
    if sys.platform != "win32":
        return ""

    registry_keys = (
        fr"HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_ID}",
        fr"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_ID}",
        fr"HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_ID}",
    )
    for registry_key in registry_keys:
        try:
            result = subprocess.run(
                ["reg", "query", registry_key, "/v", "pv"],
                check=False,
                capture_output=True,
                text=True,
                **_hidden_subprocess_kwargs(),
            )
        except OSError:
            return ""
        if result.returncode != 0:
            continue
        for line in result.stdout.splitlines():
            if "REG_SZ" not in line:
                continue
            return line.split("REG_SZ", 1)[1].strip()
    return ""


def _build_desktop_runtime_check() -> dict[str, str]:
    version = _pywebview_version()
    runtime_name = "pywebview"
    if version:
        runtime_name = f"{runtime_name} {version}"

    if sys.platform == "win32":
        webview2_version = _detect_webview2_version()
        if webview2_version:
            return {
                "label": "桌面内核",
                "status": "已就绪",
                "tone": "success",
                "detail": f"{_platform_label()} | {runtime_name} | WebView2 {webview2_version}",
            }
        return {
            "label": "桌面内核",
            "status": "需确认",
            "tone": "warning",
            "detail": f"{_platform_label()} | {runtime_name} | 未发现 WebView2 注册项",
        }

    return {
        "label": "桌面内核",
        "status": "已就绪",
        "tone": "success",
        "detail": f"{_platform_label()} | {runtime_name}",
    }


def _build_pending_check(label: str, detail: str) -> dict[str, str]:
    return {
        "label": label,
        "status": "检测中",
        "tone": "running",
        "detail": detail,
    }


def _build_directory_check(label: str, path: Path) -> dict[str, str]:
    target = path.expanduser()
    existed_before = target.exists()
    try:
        target.mkdir(parents=True, exist_ok=True)
        probe_path = target / ".write_test"
        probe_path.write_text("ok", encoding="utf-8")
        probe_path.unlink(missing_ok=True)
        return {
            "label": label,
            "status": "可写" if existed_before else "已创建",
            "tone": "success",
            "detail": str(target.resolve()),
        }
    except Exception as exc:
        return {
            "label": label,
            "status": "不可写",
            "tone": "error",
            "detail": f"{target} | {exc}",
        }


def _build_session_check(result: SessionInspectionResult | None) -> dict[str, str]:
    if result is None:
        return {
            "label": "Hollysys 会话",
            "status": "未检测",
            "tone": "idle",
            "detail": "启动后自动检测 Chrome Hollysys 会话。",
        }

    status_text = "检测失败"
    tone = "error"
    if result.status == "ready":
        status_text = "会话可用"
        tone = "success"
    elif result.status == "expired":
        status_text = "需要重登"
        tone = "warning"
    elif result.status == "missing":
        status_text = "未检测到"
        tone = "warning"

    return {
        "label": "Hollysys 会话",
        "status": status_text,
        "tone": tone,
        "detail": result.detail,
    }


def collect_startup_checks(
    *,
    session_result: SessionInspectionResult | None,
    desktop_runtime_check: dict[str, str] | None = None,
    runtime_directory_check: dict[str, str] | None = None,
    file_root_check: dict[str, str] | None = None,
) -> list[dict[str, str]]:
    return [
        desktop_runtime_check or _build_desktop_runtime_check(),
        runtime_directory_check or _build_directory_check("运行目录", APP_RUNTIME_ROOT),
        file_root_check or _build_directory_check("资料目录", FILE_ROOT),
        _build_session_check(session_result),
    ]


def _resolve_webui_dir() -> Path:
    bundle_root = getattr(sys, "_MEIPASS", "")
    if bundle_root:
        bundled_dir = Path(bundle_root) / "src" / "webui"
        if bundled_dir.exists():
            return bundled_dir
    return WEBUI_DIR


def build_app_html() -> str:
    webui_dir = _resolve_webui_dir()
    html = (webui_dir / "index.html").read_text(encoding="utf-8")
    if "/*__APP_CSS__*/" in html:
        html = html.replace("/*__APP_CSS__*/", (webui_dir / "app.css").read_text(encoding="utf-8"))
    if "/*__APP_JS__*/" in html:
        html = html.replace("/*__APP_JS__*/", (webui_dir / "app.js").read_text(encoding="utf-8"))
    return html


def _open_local_path(path: Path) -> None:
    resolved = str(path.expanduser().resolve())
    if sys.platform == "win32":
        os.startfile(resolved)  # type: ignore[attr-defined]
        return
    command = ["open", resolved] if sys.platform == "darwin" else ["xdg-open", resolved]
    subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def _inspect_session_with(
    session_inspector: Callable[..., SessionInspectionResult],
    *,
    timeout_seconds: float,
) -> SessionInspectionResult:
    try:
        return session_inspector(timeout_seconds=timeout_seconds)
    except Exception as exc:
        return SessionInspectionResult(
            status="error",
            detail=f"会话检测失败: {exc}",
            cookie_db_exists=False,
            cookie_db_path="",
            hollysys_cookie_count=0,
            cookie_names=(),
            safe_storage_available=False,
            authenticated=False,
            http_status=0,
            final_url="",
        )


def _execute_action(
    action: str,
    *,
    file_root: Path,
    processed_projects_path: Path,
    ai_settings: AISettings,
    session_timeout_seconds: float,
    session_inspector: Callable[..., SessionInspectionResult],
    batch_runner: Callable[..., BatchWorkflowResult],
    download_runner: Callable[..., WebPhaseResult],
    compare_runner: Callable[..., WorkflowResult],
    log_callback: Callable[[str], None] | None,
) -> dict[str, Any]:
    session_result: SessionInspectionResult | None = None
    output_payload = {
        "mode": action,
        "updatedAt": datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "successWorkbookPath": "",
        "successProjectCodes": [],
        "errorReportPaths": [],
        "logPath": "",
        "successCount": 0,
        "duplicateCount": 0,
        "failedCount": 0,
    }

    try:
        if action == "batch" and ai_settings.enabled and not is_remote_recognition_configured(ai_settings):
            if log_callback is not None:
                log_callback("[运行] 未配置 AI 或 OCR，请先在设置中完成至少一种识别配置")
            return {
                "status_text": "需配置 AI/OCR",
                "status_tone": "warning",
                "session_result": session_result,
                "outputs": output_payload,
            }

        if action in {"batch", "download"}:
            session_result = _inspect_session_with(
                session_inspector,
                timeout_seconds=session_timeout_seconds,
            )
            if session_result.status != "ready":
                if log_callback is not None:
                    log_callback(f"[会话] {session_result.detail}")
                return {
                    "status_text": f"{'批处理' if action == 'batch' else '下载'}前需先登录",
                    "status_tone": "warning",
                    "session_result": session_result,
                    "outputs": output_payload,
                }

        if action == "batch":
            if log_callback is not None:
                log_callback("[运行] 开始执行批处理")
            result = batch_runner(
                file_root=file_root,
                username="",
                password="",
                log_callback=log_callback,
                ai_settings=ai_settings,
            )
            if log_callback is not None:
                log_callback(
                    "[运行] 批处理完成: "
                    f"下载={result.web_processed_count} | 追加成功={result.compare_appended_count} | "
                    f"重复跳过={result.compare_duplicate_count} | 失败={result.compare_failed_count} | 清理成功={result.cleaned_count}"
                )
            return {
                "status_text": "批处理完成",
                "status_tone": "success",
                "session_result": session_result,
                "outputs": {
                    **output_payload,
                    "successWorkbookPath": str(result.compare_success_workbook_path) if result.compare_success_workbook_path else "",
                    "successProjectCodes": list(result.compare_success_project_codes),
                    "errorReportPaths": [str(path) for path in result.compare_error_report_paths],
                    "logPath": str(result.log_path) if result.log_path else "",
                    "successCount": result.compare_appended_count,
                    "duplicateCount": result.compare_duplicate_count,
                    "failedCount": result.compare_failed_count,
                },
            }

        if action == "download":
            if log_callback is not None:
                log_callback("[运行] 开始下载 Hollysys 待办资料")
            result = download_runner(
                file_root=file_root,
                username="",
                password="",
                log_callback=log_callback,
                processed_projects_path=processed_projects_path,
            )
            if log_callback is not None:
                log_callback(f"[运行] 下载完成: {len(result.processed_projects)} | 跳过: {len(result.skipped_projects)}")
            return {
                "status_text": "下载完成",
                "status_tone": "success",
                "session_result": session_result,
                "outputs": output_payload,
            }

        if log_callback is not None:
            log_callback("[运行] 开始执行文件比对")
        result = compare_runner(
            file_root=file_root,
            username="",
            password="",
            log_callback=log_callback,
            ai_settings=ai_settings,
        )
        if log_callback is not None:
            log_callback(
                f"[运行] 本地比对完成: 追加成功={result.appended_count} | 重复跳过={result.duplicate_count} | 失败={result.failed_count}"
            )
        return {
            "status_text": "比对完成",
            "status_tone": "success",
            "session_result": session_result,
            "outputs": {
                **output_payload,
                "successWorkbookPath": str(result.success_workbook_path) if result.success_workbook_path else "",
                "successProjectCodes": list(result.success_project_codes),
                "errorReportPaths": [str(path) for path in result.error_report_paths],
                "logPath": str(result.log_path) if result.log_path else "",
                "successCount": result.appended_count,
                "duplicateCount": result.duplicate_count,
                "failedCount": result.failed_count,
            },
        }
    except Exception as exc:
        if log_callback is not None:
            log_callback(f"[异常] {exc}")
        return {
            "status_text": "批处理失败" if action == "batch" else "下载失败" if action == "download" else "比对失败",
            "status_tone": "error",
            "session_result": session_result,
            "outputs": output_payload,
        }


def run_action_worker_process(
    action: str,
    *,
    file_root: Path,
    processed_projects_path: Path,
    ai_settings: AISettings,
    session_timeout_seconds: float,
    worker_queue: Any,
) -> None:
    def queue_log(message: str) -> None:
        worker_queue.put({"event": "log", "message": message})

    result = _execute_action(
        action,
        file_root=file_root,
        processed_projects_path=processed_projects_path,
        ai_settings=ai_settings,
        session_timeout_seconds=session_timeout_seconds,
        session_inspector=inspect_local_hollysys_session,
        batch_runner=run_batch_workflow,
        download_runner=run_download_workflow,
        compare_runner=run_compare_workflow,
        log_callback=queue_log,
    )
    worker_queue.put({"event": "result", "payload": result})


class WebviewApi:
    def __init__(
        self,
        *,
        settings_path: Path = SETTINGS_PATH,
        processed_projects_path: Path = PROCESSED_PROJECTS_PATH,
        session_timeout_seconds: float = DEFAULT_SESSION_TIMEOUT_SECONDS,
        settings_loader: Callable[[Path], AppSettings] = load_settings,
        settings_saver: Callable[[Path, str, str, str, AISettings | None], None] = save_settings,
        session_inspector: Callable[..., SessionInspectionResult] = inspect_local_hollysys_session,
        batch_runner: Callable[..., BatchWorkflowResult] = run_batch_workflow,
        download_runner: Callable[..., WebPhaseResult] = run_download_workflow,
        compare_runner: Callable[..., WorkflowResult] = run_compare_workflow,
        open_path: Callable[[Path], None] = _open_local_path,
        webview_module=webview,
        log_time_provider: Callable[[], str] | None = None,
        startup_in_background: bool = False,
    ) -> None:
        self.settings_path = settings_path
        self.processed_projects_path = processed_projects_path
        self.session_timeout_seconds = session_timeout_seconds
        self._settings_loader = settings_loader
        self._settings_saver = settings_saver
        self._session_inspector = session_inspector
        self._batch_runner = batch_runner
        self._download_runner = download_runner
        self._compare_runner = compare_runner
        self._open_path = open_path
        self._webview = webview_module
        self._log_time_provider = log_time_provider or (lambda: datetime.now().strftime("%H:%M:%S"))
        self.startup_in_background = startup_in_background

        self._lock = threading.RLock()
        self.window: Any | None = None
        self.settings = self._settings_loader(self.settings_path)
        if self.startup_in_background:
            self.desktop_runtime_check = _build_pending_check("桌面内核", "页面打开后后台检测")
            self.runtime_directory_check = _build_pending_check("运行目录", "页面打开后后台检测")
            self.file_root_check = _build_pending_check("资料目录", str(self._current_file_root()))
        else:
            self.desktop_runtime_check = _build_desktop_runtime_check()
            self.runtime_directory_check = _build_directory_check("运行目录", APP_RUNTIME_ROOT)
            self.file_root_check = _build_directory_check("资料目录", self._current_file_root())
        self.session_result: SessionInspectionResult | None = None
        self.logs: list[str] = []
        self.status_text = "启动检测中" if self.startup_in_background else "待执行"
        self.status_tone = "running" if self.startup_in_background else "idle"
        self.active_task_name = ""
        self.stop_requested = False
        self.startup_snapshot_logged = False
        self.startup_probe_started = False
        self.startup_probe_running = False
        self.busy_operation = ""
        self.busy_title = ""
        self.busy_detail = ""
        self.action_process: Any | None = None
        self.output_summary = self._empty_output_summary()

    def attach_window(self, window: Any) -> None:
        self.window = window

    def bootstrap(self) -> dict[str, Any]:
        if self.startup_in_background:
            should_start = False
            with self._lock:
                if not self.startup_probe_started:
                    self.startup_probe_started = True
                    self.startup_probe_running = True
                    self._set_busy("startup", "正在后台检测环境和会话", "页面已打开，请稍候几秒")
                    self._append_log("[启动] 页面已打开，后台检测环境和会话", push=False)
                    should_start = True
                state = self._build_state()
            if should_start:
                worker = threading.Thread(target=self._run_startup_probe, daemon=True)
                worker.start()
            return state

        self._inspect_session(log_result=False, push=False)
        self._append_startup_snapshot_logs()
        return self._build_state()

    def handle_start_stop(self) -> dict[str, Any]:
        if self._is_running():
            return self.request_stop()
        return self.start_batch()

    def start_batch(self) -> dict[str, Any]:
        return self._launch_action("batch")

    def run_download_only(self) -> dict[str, Any]:
        return self._launch_action("download")

    def run_compare_only(self) -> dict[str, Any]:
        return self._launch_action("compare")

    def request_stop(self) -> dict[str, Any]:
        with self._lock:
            self.stop_requested = True
            self._append_log("[运行] 当前任务不支持立即停止，请等待当前步骤结束", push=False)
            self._set_status("等待当前步骤结束", "warning")
            return self._build_state()

    def refresh_session(self) -> dict[str, Any]:
        with self._lock:
            if self._is_ui_busy():
                return self._build_state()
            self._set_busy("session", "正在刷新 Hollysys 会话", "读取本机 Chrome Cookie 并验证当前登录状态")
            self._append_log("[会话] 开始刷新 Hollysys 会话", push=False)
            self._set_status("刷新会话中", "running")
            state = self._build_state()

        worker = threading.Thread(target=self._run_refresh_session_sync, daemon=True)
        worker.start()
        return state

    def clear_logs(self) -> dict[str, Any]:
        with self._lock:
            self.logs.clear()
            return self._build_state()

    def choose_file_root(self) -> dict[str, Any]:
        current_dir = str(self._current_file_root())
        selected: str = ""
        if self.window is not None:
            result = self.window.create_file_dialog(
                self._webview.FileDialog.FOLDER,
                directory=current_dir,
            )
            if result:
                selected = str(result[0])
        return {"selected": selected}

    def open_file_root(self) -> bool:
        self._open_path(self._current_file_root())
        return True

    def open_path(self, raw_path: str) -> bool:
        if not raw_path:
            return False
        self._open_path(Path(raw_path))
        return True

    def open_parent_path(self, raw_path: str) -> bool:
        if not raw_path:
            return False
        target = Path(raw_path)
        parent = target if target.is_dir() else target.parent
        self._open_path(parent)
        return True

    def save_settings(self, payload: dict[str, Any]) -> dict[str, Any]:
        with self._lock:
            if self._is_ui_busy():
                return self._build_state()
            self.settings = self._build_settings_from_payload(payload)
            self.output_summary = self._empty_output_summary()
            self._set_busy("settings", "正在保存设置", "检查资料目录并写入本地配置")
            return_state = self._build_state()

        worker = threading.Thread(target=self._run_save_settings_sync, daemon=True)
        worker.start()
        return return_state

    def _launch_action(self, action: str) -> dict[str, Any]:
        with self._lock:
            if self._is_ui_busy():
                return self._build_state()

        with self._lock:
            self.stop_requested = False
            self.active_task_name = action
            self._persist_settings()
            if action == "batch":
                self._set_busy("batch", "正在准备批处理", "先检查会话和识别配置，再进入下载 / 比对 / 清理")
                self._append_log("[运行] 已接收批处理任务，正在后台准备", push=False)
                self._set_status("批处理准备中", "running")
            elif action == "download":
                self._set_busy("download", "正在准备下载任务", "先检查会话，再开始抓取 Hollysys 附件")
                self._append_log("[运行] 已接收下载任务，正在后台准备", push=False)
                self._set_status("下载准备中", "running")
            else:
                self._set_busy("compare", "正在准备本地比对", "校验配置后开始读取本地文件")
                self._append_log("[运行] 已接收本地比对任务，正在后台准备", push=False)
                self._set_status("比对准备中", "running")
        self._start_action_worker(action)
        return self._build_state()

    def _start_action_worker(self, action: str) -> None:
        try:
            process_context = multiprocessing.get_context("spawn")
            worker_queue = process_context.Queue()
            process = process_context.Process(
                target=run_action_worker_process,
                kwargs={
                    "action": action,
                    "file_root": self._current_file_root(),
                    "processed_projects_path": self.processed_projects_path,
                    "ai_settings": self.settings.ai,
                    "session_timeout_seconds": self.session_timeout_seconds,
                    "worker_queue": worker_queue,
                },
                daemon=True,
            )
            process.start()
        except Exception as exc:
            with self._lock:
                self._append_log(f"[异常] 无法启动后台进程: {exc}", push=False)
                self._set_status("后台进程启动失败", "error")
                self.active_task_name = ""
                self._clear_busy(action)
            self._push_state()
            return

        with self._lock:
            self.action_process = process

        monitor = threading.Thread(
            target=self._monitor_action_worker,
            args=(action, worker_queue, process),
            daemon=True,
        )
        monitor.start()

    def _monitor_action_worker(self, action: str, worker_queue: Any, process: Any) -> None:
        payload: dict[str, Any] | None = None

        def handle_message(message: Any) -> dict[str, Any] | None:
            if not isinstance(message, dict):
                return None

            event = str(message.get("event", ""))
            if event == "log":
                log_message = message.get("message")
                if isinstance(log_message, str):
                    self._background_log(log_message)
                return None

            if event == "result":
                raw_payload = message.get("payload")
                if isinstance(raw_payload, dict):
                    return raw_payload
            return None

        try:
            while True:
                try:
                    message = worker_queue.get(timeout=0.2)
                except queue.Empty:
                    if not process.is_alive():
                        break
                    continue

                handled_payload = handle_message(message)
                if handled_payload is not None:
                    payload = handled_payload

                if payload is not None and not process.is_alive():
                    break
        finally:
            try:
                process.join(timeout=0.5)
            except Exception:
                pass

            while True:
                try:
                    message = worker_queue.get_nowait()
                except queue.Empty:
                    break
                handled_payload = handle_message(message)
                if handled_payload is not None:
                    payload = handled_payload

            try:
                worker_queue.close()
                worker_queue.join_thread()
            except Exception:
                pass

            with self._lock:
                if payload is not None:
                    session_result = payload.get("session_result")
                    if isinstance(session_result, SessionInspectionResult):
                        self.session_result = session_result
                    outputs = payload.get("outputs")
                    if isinstance(outputs, dict):
                        self.output_summary = self._sanitize_output_summary(outputs)
                    self._set_status(
                        str(payload.get("status_text", "任务完成")),
                        str(payload.get("status_tone", "success")),
                    )
                elif process.exitcode not in (0, None):
                    self._append_log(f"[异常] 后台进程异常退出: exitcode={process.exitcode}", push=False)
                    self._set_status("后台进程异常退出", "error")

                self.active_task_name = ""
                self.action_process = None
                self._clear_busy(action)
            self._push_state()

    def _run_action_sync(self, action: str) -> None:
        try:
            with self._lock:
                if action == "batch":
                    self._set_busy("batch", "批处理执行中", "正在下载 / 比对 / 清理项目资料")
                    self._set_status("批处理中", "running")
                elif action == "download":
                    self._set_busy("download", "下载任务执行中", "正在抓取 Hollysys 待办附件")
                    self._set_status("下载中", "running")
                else:
                    self._set_busy("compare", "本地比对执行中", "正在扫描目录并比对文件内容")
                    self._set_status("比对中", "running")
            self._push_state()
            result = _execute_action(
                action,
                file_root=self._current_file_root(),
                processed_projects_path=self.processed_projects_path,
                ai_settings=self.settings.ai,
                session_timeout_seconds=self.session_timeout_seconds,
                session_inspector=self._session_inspector,
                batch_runner=self._batch_runner,
                download_runner=self._download_runner,
                compare_runner=self._compare_runner,
                log_callback=self._background_log,
            )
            with self._lock:
                session_result = result.get("session_result")
                if isinstance(session_result, SessionInspectionResult):
                    self.session_result = session_result
                outputs = result.get("outputs")
                if isinstance(outputs, dict):
                    self.output_summary = self._sanitize_output_summary(outputs)
                self._set_status(
                    str(result.get("status_text", "任务完成")),
                    str(result.get("status_tone", "success")),
                )
        finally:
            with self._lock:
                self.active_task_name = ""
                self._clear_busy(action)
            self._push_state()

    def _background_log(self, message: str) -> None:
        with self._lock:
            self._append_log(message, push=False)
        self._push_state()

    def _run_refresh_session_sync(self) -> None:
        try:
            self._inspect_session(log_result=True, push=False)
        finally:
            with self._lock:
                self._clear_busy("session")
            self._push_state()

    def _run_save_settings_sync(self) -> None:
        try:
            file_root_check = _build_directory_check("资料目录", self._current_file_root())
            with self._lock:
                self.file_root_check = file_root_check
                self._persist_settings()
            self._background_log("[设置] 已更新")
        except Exception as exc:
            self._background_log(f"[异常] 保存设置失败: {exc}")
            with self._lock:
                self._set_status("设置保存失败", "error")
        finally:
            with self._lock:
                self._clear_busy("settings")
            self._push_state()

    def _run_startup_probe(self) -> None:
        try:
            desktop_runtime_check = _build_desktop_runtime_check()
            runtime_directory_check = _build_directory_check("运行目录", APP_RUNTIME_ROOT)
            file_root_check = _build_directory_check("资料目录", self._current_file_root())

            with self._lock:
                self.desktop_runtime_check = desktop_runtime_check
                self.runtime_directory_check = runtime_directory_check
                self.file_root_check = file_root_check

            self._inspect_session(log_result=False, push=False)
            self._append_startup_snapshot_logs()
        finally:
            with self._lock:
                self.startup_probe_running = False
                self._clear_busy("startup")
            self._push_state()

    def _append_startup_snapshot_logs(self) -> None:
        with self._lock:
            if self.startup_snapshot_logged:
                return

            self._append_log("[启动] 自动检测当前环境和会话", push=False)
            self._append_log(
                "[环境] "
                f"{self.desktop_runtime_check['label']}: "
                f"{self.desktop_runtime_check['status']} | "
                f"{self.desktop_runtime_check['detail']}",
                push=False,
            )
            self._append_log(
                "[自检] "
                f"{self.runtime_directory_check['label']}: "
                f"{self.runtime_directory_check['status']} | "
                f"{self.runtime_directory_check['detail']}",
                push=False,
            )
            self._append_log(
                "[自检] "
                f"{self.file_root_check['label']}: "
                f"{self.file_root_check['status']} | "
                f"{self.file_root_check['detail']}",
                push=False,
            )
            if self.session_result is not None:
                self._append_log(f"[会话] {self.session_result.detail}", push=False)
            self.startup_snapshot_logged = True

    def _inspect_session(self, *, log_result: bool, push: bool) -> SessionInspectionResult:
        result = _inspect_session_with(
            self._session_inspector,
            timeout_seconds=self.session_timeout_seconds,
        )

        with self._lock:
            self.session_result = result
            if result.status == "ready" and not self._is_running():
                self._set_status("待执行", "idle")
            elif result.status == "expired":
                self._set_status("需要重新登录", "warning")
            elif result.status == "missing":
                self._set_status("未检测到会话", "warning")
            elif result.status == "error":
                self._set_status("会话异常", "error")
            if log_result:
                self._append_log(f"[会话] {result.detail}", push=False)

        if push:
            self._push_state()
        return result

    def _append_log(self, message: str, *, push: bool) -> None:
        self.logs.append(f"{self._log_time_provider()} {message}")
        if len(self.logs) > LOG_LIMIT:
            self.logs = self.logs[-LOG_LIMIT:]
        if push:
            self._push_state()

    def _set_status(self, text: str, tone: str) -> None:
        self.status_text = text
        self.status_tone = tone

    def _set_busy(self, operation: str, title: str, detail: str) -> None:
        self.busy_operation = operation
        self.busy_title = title
        self.busy_detail = detail

    def _clear_busy(self, operation: str | None = None) -> None:
        if operation is not None and self.busy_operation != operation:
            return
        self.busy_operation = ""
        self.busy_title = ""
        self.busy_detail = ""

    def _persist_settings(self) -> None:
        self._settings_saver(
            self.settings_path,
            self.settings.username,
            self.settings.password,
            str(self._current_file_root()),
            self.settings.ai,
        )

    def _current_file_root(self) -> Path:
        return Path(self.settings.last_file_root or str(FILE_ROOT))

    def _is_running(self) -> bool:
        return self.active_task_name != ""

    def _is_ui_busy(self) -> bool:
        return self.busy_operation != ""

    def _empty_output_summary(self) -> dict[str, Any]:
        return {
            "mode": "",
            "updatedAt": "",
            "successWorkbookPath": "",
            "successProjectCodes": [],
            "errorReportPaths": [],
            "logPath": "",
            "successCount": 0,
            "duplicateCount": 0,
            "failedCount": 0,
        }

    def _sanitize_output_summary(self, payload: dict[str, Any]) -> dict[str, Any]:
        return {
            "mode": str(payload.get("mode", "") or ""),
            "updatedAt": str(payload.get("updatedAt", "") or ""),
            "successWorkbookPath": str(payload.get("successWorkbookPath", "") or ""),
            "successProjectCodes": [
                str(code)
                for code in payload.get("successProjectCodes", [])
                if isinstance(code, (str, Path)) and str(code)
            ],
            "errorReportPaths": [
                str(path)
                for path in payload.get("errorReportPaths", [])
                if isinstance(path, (str, Path)) and str(path)
            ],
            "logPath": str(payload.get("logPath", "") or ""),
            "successCount": int(payload.get("successCount", 0) or 0),
            "duplicateCount": int(payload.get("duplicateCount", 0) or 0),
            "failedCount": int(payload.get("failedCount", 0) or 0),
        }

    def _build_outputs_state(self) -> dict[str, Any]:
        file_root = self._current_file_root()
        success_dir = file_root / "success"
        error_dir = file_root / "error"
        log_dir = error_dir / "logs"
        summary = self.output_summary
        return {
            "mode": summary["mode"],
            "updatedAt": summary["updatedAt"],
            "successWorkbookPath": summary["successWorkbookPath"],
            "successProjectCodes": list(summary["successProjectCodes"]),
            "successDir": str(success_dir),
            "errorReportPaths": list(summary["errorReportPaths"]),
            "errorDir": str(error_dir),
            "logPath": summary["logPath"],
            "logDir": str(log_dir),
            "successCount": summary["successCount"],
            "duplicateCount": summary["duplicateCount"],
            "failedCount": summary["failedCount"],
        }

    def _build_settings_from_payload(self, payload: dict[str, Any]) -> AppSettings:
        return AppSettings(
            username=self.settings.username,
            password=self.settings.password,
            last_file_root=str(payload.get("lastFileRoot", "") or str(FILE_ROOT)),
            ai=AISettings(
                enabled=bool(payload.get("aiEnabled", False)),
                ai_base_url=str(payload.get("aiBaseUrl", "")),
                ai_api_key=str(payload.get("aiApiKey", "")),
                ai_model=str(payload.get("aiModel", "")),
                ocr_base_url=str(payload.get("ocrBaseUrl", "")),
                ocr_api_key=str(payload.get("ocrApiKey", "")),
                request_timeout_seconds=int(payload.get("requestTimeoutSeconds", 30) or 30),
                image_max_kb=int(payload.get("imageMaxKb", 100) or 100),
            ),
        )

    def _build_state(self) -> dict[str, Any]:
        with self._lock:
            return {
                "windowTitle": WINDOW_TITLE,
                "header": {
                    "title": "Hollysys 批处理",
                    "subtitle": "下载 / 比对 / 清理",
                },
                "status": {
                    "text": self.status_text,
                    "tone": self.status_tone,
                },
                "running": self._is_running(),
                "startupLoading": self.startup_probe_running,
                "busy": {
                    "active": self._is_ui_busy(),
                    "kind": self.busy_operation,
                    "title": self.busy_title,
                    "detail": self.busy_detail,
                },
                "summary": {
                    "mode": "Chrome 本机会话直连 + 本地比对",
                    "directory": str(self._current_file_root()),
                    "preflight": "启动自检 + 下载前复检会话",
                },
                "startupChecks": collect_startup_checks(
                    session_result=self.session_result,
                    desktop_runtime_check=self.desktop_runtime_check,
                    runtime_directory_check=self.runtime_directory_check,
                    file_root_check=self.file_root_check,
                ),
                "outputs": self._build_outputs_state(),
                "session": self._build_session_state(),
                "logs": list(self.logs),
                "settings": self._build_settings_state(),
            }

    def _build_settings_state(self) -> dict[str, Any]:
        return {
            "lastFileRoot": str(self._current_file_root()),
            "aiEnabled": self.settings.ai.enabled,
            "aiBaseUrl": self.settings.ai.ai_base_url,
            "aiApiKey": self.settings.ai.ai_api_key,
            "aiModel": self.settings.ai.ai_model,
            "ocrBaseUrl": self.settings.ai.ocr_base_url,
            "ocrApiKey": self.settings.ai.ocr_api_key,
            "requestTimeoutSeconds": self.settings.ai.request_timeout_seconds,
            "imageMaxKb": self.settings.ai.image_max_kb,
        }

    def _build_session_state(self) -> dict[str, Any]:
        result = self.session_result
        if result is None:
            return {
                "badge": {"text": "未检测", "tone": "idle"},
                "browser": "尚未检测本机 Hollysys 会话",
                "cookieDb": "-",
                "cookieInfo": "-",
                "probeUrl": "-",
                "detail": "点击“刷新会话”后显示详细结果。",
            }

        badge = {"text": "检测失败", "tone": "error"}
        if result.status == "ready":
            badge = {"text": "会话可用", "tone": "success"}
        elif result.status == "expired":
            badge = {"text": "需要重登", "tone": "warning"}
        elif result.status == "missing":
            badge = {"text": "未检测到", "tone": "warning"}

        browser = "未发现 Chrome 本机会话"
        if result.cookie_db_exists:
            browser = "Chrome 本机会话可读"
        if result.status == "ready":
            browser += " | Hollysys 已认证"
        elif result.hollysys_cookie_count > 0:
            browser += " | Hollysys 未认证"

        cookie_info = str(result.hollysys_cookie_count)
        if result.cookie_names:
            cookie_info = f"{cookie_info} | {', '.join(result.cookie_names)}"

        return {
            "badge": badge,
            "browser": browser,
            "cookieDb": result.cookie_db_path or "-",
            "cookieInfo": cookie_info,
            "probeUrl": result.final_url or "-",
            "detail": result.detail,
        }

    def _push_state(self) -> None:
        if self.window is None:
            return
        payload = json.dumps(self._build_state(), ensure_ascii=False)
        try:
            self.window.evaluate_js(f"window.appBridge && window.appBridge.sync({payload})")
        except Exception:
            return


def run_gui_app() -> int:
    api = WebviewApi(startup_in_background=True)
    window = webview.create_window(
        WINDOW_TITLE,
        html=build_app_html(),
        js_api=api,
        width=WINDOW_WIDTH,
        height=WINDOW_HEIGHT,
        resizable=False,
        min_size=WINDOW_MIN_SIZE,
        background_color="#F3F6FA",
        text_select=True,
    )
    if window is not None:
        api.attach_window(window)
    webview.start(debug=False)
    return 0
