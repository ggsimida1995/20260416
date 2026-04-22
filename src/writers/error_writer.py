from __future__ import annotations

from datetime import datetime
from pathlib import Path

from src.models import CompareFailure


def write_error_report(
    error_root: Path,
    project_name: str,
    failures: list[CompareFailure],
) -> Path:
    error_root.mkdir(parents=True, exist_ok=True)
    output_path = error_root / f"{project_name}.txt"

    lines = [
        f"项目目录: {project_name}",
        "最终状态: 失败",
        f"处理时间: {datetime.now().isoformat(sep=' ', timespec='seconds')}",
        "失败项:",
    ]
    for failure in failures:
        values = ", ".join(f"{key}={value}" for key, value in failure.values.items())
        lines.append(f"- {failure.field_name}: {failure.message}" + (f" ({values})" if values else ""))

    output_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return output_path
