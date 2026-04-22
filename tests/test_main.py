import main


def test_main_runs_gui_entrypoint(monkeypatch):
    called = {}

    def fake_run_gui() -> int:
        called["ran"] = True
        return 0

    monkeypatch.setattr(main, "run_gui", fake_run_gui)

    exit_code = main.main()

    assert exit_code == 0
    assert called == {"ran": True}
