from src.runtime_logging import RuntimeLogger, build_log_path


def test_build_log_path_uses_hourly_log_file(tmp_path):
    path = build_log_path(tmp_path)

    assert path.parent == tmp_path / "error" / "logs"
    assert path.name.startswith("workflow-")
    assert path.suffix == ".log"
    assert len(path.stem) == len("workflow-20260509-12")


def test_runtime_logger_appends_existing_hourly_log(tmp_path):
    path = tmp_path / "error" / "logs" / "workflow-20260509-12.log"
    with RuntimeLogger(path) as logger:
        logger.log("first")
    with RuntimeLogger(path) as logger:
        logger.log("second")

    assert path.read_text(encoding="utf-8") == "first\nsecond\n"
