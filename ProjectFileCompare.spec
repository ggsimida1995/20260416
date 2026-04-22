# -*- mode: python ; coding: utf-8 -*-

import sys
from pathlib import Path


project_root = Path(SPECPATH)
webui_dir = project_root / "src" / "webui"

a = Analysis(
    [str(project_root / "main.py")],
    pathex=[str(project_root)],
    binaries=[],
    datas=[(str(webui_dir), "src/webui")],
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=["PySide6", "PyQt5", "PyQt6", "qtpy"],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="ProjectFileCompare",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)

if sys.platform == "darwin":
    app = BUNDLE(
        exe,
        name="ProjectFileCompare.app",
        icon=None,
        bundle_identifier=None,
    )
