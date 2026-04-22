from __future__ import annotations

from contextlib import AbstractContextManager
from datetime import datetime
from pathlib import Path
from typing import Callable


LogCallback = Callable[[str], None]


class RuntimeLogger(AbstractContextManager["RuntimeLogger"]):
    def __init__(self, log_path: Path | None = None, callback: LogCallback | None = None) -> None:
        self._callback = callback
        self._log_path = log_path
        self._handle = None

        if self._log_path is not None:
            self._log_path.parent.mkdir(parents=True, exist_ok=True)
            self._handle = self._log_path.open("w", encoding="utf-8")

    @property
    def log_path(self) -> Path | None:
        return self._log_path

    def log(self, message: str) -> None:
        if self._callback is not None:
            self._callback(message)
        if self._handle is not None:
            self._handle.write(f"{message}\n")
            self._handle.flush()

    def section(self, title: str) -> None:
        self.log("")
        self.log(title)

    def close(self) -> None:
        if self._handle is not None:
            self._handle.close()
            self._handle = None

    def __exit__(self, exc_type, exc_value, traceback) -> None:
        self.close()


def build_log_path(file_root: Path) -> Path:
    log_dir = file_root / "error" / "logs"
    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    return log_dir / f"workflow-{timestamp}.txt"
