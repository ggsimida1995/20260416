from pathlib import Path

from src.models import ProjectErrorDetail
from src.writers.result_log_writer import (
    clear_result_log,
    error_log_path,
    read_latest_log_items,
    success_log_path,
    write_result_logs,
)


def test_write_result_logs_appends_success_and_error_logs(tmp_path: Path):
    success_path, error_path = write_result_logs(
        file_root=tmp_path,
        success_project_codes=["BHE-25030367/01"],
        error_details=[
            ProjectErrorDetail(
                project_code="BHE-25030368/01",
                field_name="用户联系人",
                message="xlsx 与 pdf 不一致",
                values={"xlsx": "张三", "pdf": "李四"},
            )
        ],
    )

    assert success_path == tmp_path / "result_logs" / "success.log"
    assert error_path == tmp_path / "result_logs" / "error.log"
    assert "BHE-25030367/01" in success_path.read_text(encoding="utf-8")
    assert "BHE-25030368/01 | 用户联系人 | xlsx 与 pdf 不一致 | xlsx=张三, pdf=李四" in error_path.read_text(encoding="utf-8")


def test_read_latest_log_items_returns_latest_twenty(tmp_path: Path):
    path = success_log_path(tmp_path)
    path.parent.mkdir(parents=True)
    path.write_text("\n".join(f"line-{index:02d}" for index in range(25)) + "\n", encoding="utf-8")

    assert read_latest_log_items(path) == [f"line-{index:02d}" for index in range(5, 25)]


def test_clear_result_log_empties_log_file(tmp_path: Path):
    path = error_log_path(tmp_path)
    path.parent.mkdir(parents=True)
    path.write_text("old\n", encoding="utf-8")

    clear_result_log(path)

    assert path.read_text(encoding="utf-8") == ""
