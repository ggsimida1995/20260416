from __future__ import annotations

import multiprocessing

def run_gui() -> int:
    from src.gui import run_gui_app

    return run_gui_app()


def main() -> int:
    multiprocessing.freeze_support()
    return run_gui()


if __name__ == "__main__":
    raise SystemExit(main())
