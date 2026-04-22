from __future__ import annotations

import json
from datetime import datetime
from pathlib import Path


def load_processed_projects(path: Path) -> dict[str, dict[str, object]]:
    if not path.exists():
        return {}

    data = json.loads(path.read_text(encoding="utf-8"))
    projects = data.get("processed_projects", {})
    if not isinstance(projects, dict):
        return {}
    return {str(project_code): dict(project_data) for project_code, project_data in projects.items()}


def mark_project_processed(path: Path, project_code: str, processed_at: str | None = None) -> None:
    projects = load_processed_projects(path)
    projects[str(project_code)] = {
        "processed_at": processed_at or datetime.now().astimezone().isoformat(timespec="seconds"),
        "success_workbook_written": True,
        "directory_deleted": True,
    }

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps({"processed_projects": projects}, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )


def processed_project_codes(path: Path) -> set[str]:
    return set(load_processed_projects(path))
