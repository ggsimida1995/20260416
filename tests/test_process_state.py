from pathlib import Path

from src.process_state import load_processed_projects, mark_project_processed, processed_project_codes


def test_load_processed_projects_returns_empty_mapping_when_file_missing(tmp_path: Path):
    assert load_processed_projects(tmp_path / "processed_projects.json") == {}


def test_mark_project_processed_persists_successful_project(tmp_path: Path):
    path = tmp_path / "processed_projects.json"

    mark_project_processed(path, project_code="BHE-25030367-01", processed_at="2026-04-17T16:20:00+08:00")

    data = load_processed_projects(path)
    assert "BHE-25030367-01" in data
    assert data["BHE-25030367-01"]["processed_at"] == "2026-04-17T16:20:00+08:00"
    assert data["BHE-25030367-01"]["success_workbook_written"] is True
    assert data["BHE-25030367-01"]["directory_deleted"] is True


def test_processed_project_codes_returns_saved_codes(tmp_path: Path):
    path = tmp_path / "processed_projects.json"
    mark_project_processed(path, project_code="BHE-25030367-01", processed_at="2026-04-17T16:20:00+08:00")
    mark_project_processed(path, project_code="BHE-25030368-01", processed_at="2026-04-17T16:21:00+08:00")

    assert processed_project_codes(path) == {"BHE-25030367-01", "BHE-25030368-01"}
