from __future__ import annotations

from pathlib import Path

from src.config import REQUIRED_FILE_KEYWORDS, RESERVED_DIRS
from src.models import ProjectFiles


def discover_projects(file_root: Path) -> list[Path]:
    if not file_root.exists():
        return []

    return sorted(
        [
            path
            for path in file_root.iterdir()
            if path.is_dir() and path.name not in RESERVED_DIRS
        ],
        key=lambda item: item.name,
    )


def discover_project_files(project_dir: Path) -> ProjectFiles:
    project_files = ProjectFiles(
        project_name=project_dir.name,
        project_dir=project_dir,
    )

    for path in sorted(project_dir.iterdir(), key=lambda item: item.name):
        if not path.is_file():
            continue

        name = path.name
        if name.endswith(".txt") and name == f"{project_dir.name}.txt":
            project_files.txt_path = path
        if project_files.xlsx_path is None and REQUIRED_FILE_KEYWORDS["xlsx"] in name and name.endswith(".xlsx"):
            project_files.xlsx_path = path
        if project_files.docx_path is None and REQUIRED_FILE_KEYWORDS["docx"] in name and name.endswith(".docx"):
            project_files.docx_path = path
        if project_files.pdf_path is None and REQUIRED_FILE_KEYWORDS["pdf"] in name and name.endswith(".pdf"):
            project_files.pdf_path = path

    missing_files = []
    if project_files.xlsx_path is None:
        missing_files.append("xlsx")
    if project_files.docx_path is None:
        missing_files.append("docx")
    if project_files.pdf_path is None:
        missing_files.append("pdf")

    project_files.missing_files = missing_files
    return project_files
