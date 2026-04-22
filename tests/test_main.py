import main


def test_main_runs_gui_entrypoint(monkeypatch):
    called = {}

    def fake_run_gui() -> int:
        called["ran"] = True
        return 0

    def fake_freeze_support() -> None:
        called["freeze"] = True

    monkeypatch.setattr(main, "run_gui", fake_run_gui)
    monkeypatch.setattr(main.multiprocessing, "freeze_support", fake_freeze_support)

    exit_code = main.main()

    assert exit_code == 0
    assert called == {"freeze": True, "ran": True}
