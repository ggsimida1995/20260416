from pathlib import Path

from src.discovery import discover_project_files, discover_projects


def test_discover_projects_skips_reserved_dirs(tmp_path: Path):
    (tmp_path / "file" / "other").mkdir(parents=True)
    (tmp_path / "file" / "success").mkdir(parents=True)
    (tmp_path / "file" / "error").mkdir(parents=True)
    (tmp_path / "file" / "BHE-25030367-01").mkdir(parents=True)

    projects = discover_projects(tmp_path / "file")

    assert [project.name for project in projects] == ["BHE-25030367-01"]


def test_discover_project_files_marks_missing_required_files(tmp_path: Path):
    project_dir = tmp_path / "file" / "BHE-25030367-01"
    project_dir.mkdir(parents=True)
    (project_dir / "only.xlsx").touch()

    discovered = discover_project_files(project_dir)

    assert discovered.missing_files == ["xlsx", "docx", "pdf"]


def test_discover_project_files_matches_required_keywords(tmp_path: Path):
    project_dir = tmp_path / "file" / "BHE-25030367-01"
    project_dir.mkdir(parents=True)
    xlsx_path = project_dir / "A项目关闭移交登记表.xlsx"
    docx_path = project_dir / "A项目竣工总结报告.docx"
    pdf_path = project_dir / "APA竣工验收报告.pdf"
    xlsx_path.touch()
    docx_path.touch()
    pdf_path.touch()

    discovered = discover_project_files(project_dir)

    assert discovered.xlsx_path == xlsx_path
    assert discovered.docx_path == docx_path
    assert discovered.pdf_path == pdf_path
    assert discovered.missing_files == []
