from __future__ import annotations

from datetime import datetime
import json
import os
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
    if sys.platform == "darwin":
        subprocess.run(["open", resolved], check=False)
        return
    subprocess.run(["xdg-open", resolved], check=False)


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

        self._lock = threading.RLock()
        self.window: Any | None = None
        self.settings = self._settings_loader(self.settings_path)
        self.desktop_runtime_check = _build_desktop_runtime_check()
        self.runtime_directory_check = _build_directory_check("运行目录", APP_RUNTIME_ROOT)
        self.file_root_check = _build_directory_check("资料目录", self._current_file_root())
        self.session_result: SessionInspectionResult | None = None
        self.logs: list[str] = []
        self.status_text = "待执行"
        self.status_tone = "idle"
        self.active_task_name = ""
        self.stop_requested = False
        self.startup_snapshot_logged = False

    def attach_window(self, window: Any) -> None:
        self.window = window

    def bootstrap(self) -> dict[str, Any]:
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
        self._inspect_session(log_result=True, push=False)
        return self._build_state()

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

    def save_settings(self, payload: dict[str, Any]) -> dict[str, Any]:
        with self._lock:
            self.settings = AppSettings(
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
            self.file_root_check = _build_directory_check("资料目录", self._current_file_root())
            self._persist_settings()
            self._append_log("[设置] 已更新", push=False)
            return self._build_state()

    def _launch_action(self, action: str) -> dict[str, Any]:
        with self._lock:
            if self._is_running():
                return self._build_state()

        if action == "batch" and self.settings.ai.enabled and not is_remote_recognition_configured(self.settings.ai):
            with self._lock:
                self._append_log("[运行] 未配置 AI 或 OCR，请先在设置中完成至少一种识别配置", push=False)
                self._set_status("需配置 AI/OCR", "warning")
                return self._build_state()

        if action in {"batch", "download"}:
            result = self._inspect_session(log_result=False, push=False)
            if result.status != "ready":
                with self._lock:
                    self._append_log(f"[会话] {result.detail}", push=False)
                    self._set_status(f"{'批处理' if action == 'batch' else '下载'}前需先登录", "warning")
                    return self._build_state()

        with self._lock:
            self.stop_requested = False
            self.active_task_name = action
            self._persist_settings()
            if action == "batch":
                self._append_log("[运行] 开始执行批处理", push=False)
                self._set_status("批处理中", "running")
            elif action == "download":
                self._append_log("[运行] 开始下载 Hollysys 待办资料", push=False)
                self._set_status("下载中", "running")
            else:
                self._append_log("[运行] 开始执行文件比对", push=False)
                self._set_status("比对中", "running")
            state = self._build_state()

        worker = threading.Thread(target=self._run_action_sync, args=(action,), daemon=True)
        worker.start()
        return state

    def _run_action_sync(self, action: str) -> None:
        try:
            file_root = self._current_file_root()
            if action == "batch":
                result = self._batch_runner(
                    file_root=file_root,
                    username="",
                    password="",
                    log_callback=self._background_log,
                    ai_settings=self.settings.ai,
                )
                self._background_log(
                    "[运行] 批处理完成: "
                    f"下载={result.web_processed_count} | 追加成功={result.compare_appended_count} | "
                    f"重复跳过={result.compare_duplicate_count} | 失败={result.compare_failed_count} | 清理成功={result.cleaned_count}"
                )
                with self._lock:
                    self._set_status("批处理完成", "success")
            elif action == "download":
                result = self._download_runner(
                    file_root=file_root,
                    username="",
                    password="",
                    log_callback=self._background_log,
                    processed_projects_path=self.processed_projects_path,
                )
                self._background_log(
                    f"[运行] 下载完成: {len(result.processed_projects)} | 跳过: {len(result.skipped_projects)}"
                )
                with self._lock:
                    self._set_status("下载完成", "success")
            else:
                result = self._compare_runner(
                    file_root=file_root,
                    username="",
                    password="",
                    log_callback=self._background_log,
                    ai_settings=self.settings.ai,
                )
                self._background_log(
                    f"[运行] 本地比对完成: 追加成功={result.appended_count} | 重复跳过={result.duplicate_count} | 失败={result.failed_count}"
                )
                with self._lock:
                    self._set_status("比对完成", "success")
        except Exception as exc:
            self._background_log(f"[异常] {exc}")
            with self._lock:
                if action == "batch":
                    self._set_status("批处理失败", "error")
                elif action == "download":
                    self._set_status("下载失败", "error")
                else:
                    self._set_status("比对失败", "error")
        finally:
            with self._lock:
                self.active_task_name = ""
            self._push_state()

    def _background_log(self, message: str) -> None:
        with self._lock:
            self._append_log(message, push=False)
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
        try:
            result = self._session_inspector(timeout_seconds=self.session_timeout_seconds)
        except Exception as exc:
            result = SessionInspectionResult(
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
    api = WebviewApi()
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
