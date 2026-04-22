from __future__ import annotations

from pathlib import Path

from src.config import APP_NAME, resolve_runtime_root


def test_resolve_runtime_root_uses_current_workspace_when_not_frozen():
    runtime_root = resolve_runtime_root(frozen=False, platform="win32", env={})

    assert runtime_root == Path(".")


def test_resolve_runtime_root_uses_local_appdata_for_frozen_windows(tmp_path: Path):
    runtime_root = resolve_runtime_root(
        frozen=True,
        platform="win32",
        env={"LOCALAPPDATA": str(tmp_path / "LocalAppData")},
    )

    assert runtime_root == tmp_path / "LocalAppData" / APP_NAME


def test_resolve_runtime_root_uses_xdg_data_home_for_frozen_linux(tmp_path: Path):
    runtime_root = resolve_runtime_root(
        frozen=True,
        platform="linux",
        env={"XDG_DATA_HOME": str(tmp_path / "xdg-data")},
    )

    assert runtime_root == tmp_path / "xdg-data" / APP_NAME
