from __future__ import annotations

from datetime import datetime
from pathlib import Path

from src.models import ProjectErrorDetail


def success_log_path(file_root: Path) -> Path:
    return file_root / "result_logs" / "success.log"


def error_log_path(file_root: Path) -> Path:
    return file_root / "result_logs" / "error.log"


def write_result_logs(
    *,
    file_root: Path,
    success_project_codes: list[str],
    error_details: list[ProjectErrorDetail],
) -> tuple[Path, Path]:
    success_path = success_log_path(file_root)
    error_path = error_log_path(file_root)
    _append_lines(success_path, _format_success_lines(success_project_codes))
    _append_lines(error_path, _format_error_lines(error_details))
    return success_path, error_path


def read_latest_log_items(path: Path, *, limit: int = 20) -> list[str]:
    if not path.exists():
        return []
    lines = [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    return lines[-limit:]


def clear_result_log(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("", encoding="utf-8")


def ensure_result_log(path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.touch(exist_ok=True)
    return path


def _append_lines(path: Path, lines: list[str]) -> None:
    if not lines:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        for line in lines:
            handle.write(f"{line}\n")


def _format_success_lines(project_codes: list[str]) -> list[str]:
    timestamp = _timestamp()
    return [f"{timestamp} | {project_code}" for project_code in project_codes]


def _format_error_lines(details: list[ProjectErrorDetail]) -> list[str]:
    timestamp = _timestamp()
    return [f"{timestamp} | {detail.project_code} | {detail.field_name} | {detail.message}{_format_values(detail.values)}" for detail in details]


def _format_values(values: dict[str, object]) -> str:
    if not values:
        return ""
    rendered = ", ".join(f"{key}={value}" for key, value in values.items())
    return f" | {rendered}"


def _timestamp() -> str:
    return datetime.now().isoformat(sep=" ", timespec="seconds")
