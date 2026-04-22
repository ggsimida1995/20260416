from pathlib import Path

from src.config_store import AISettings, load_settings, save_settings


def test_load_settings_returns_defaults_when_file_missing(tmp_path: Path):
    settings = load_settings(tmp_path / "settings.json")

    assert settings.username == ""
    assert settings.password == ""
    assert settings.last_file_root == ""
    assert settings.ai.enabled is False
    assert settings.ai.ai_base_url == "https://ark.cn-beijing.volces.com/api/v3"
    assert settings.ai.ai_model == "doubao-seed-2-0-lite-260215"
    assert settings.ai.ocr_base_url == ""
    assert settings.ai.request_timeout_seconds == 30
    assert settings.ai.image_max_kb == 100


def test_save_settings_persists_username_password_and_last_path(tmp_path: Path):
    path = tmp_path / "settings.json"

    save_settings(path, username="user1", password="pass1", last_file_root="D:/data")
    loaded = load_settings(path)

    assert loaded.username == "user1"
    assert loaded.password == "pass1"
    assert loaded.last_file_root == "D:/data"


def test_save_settings_persists_full_configuration(tmp_path: Path):
    path = tmp_path / "settings.json"

    save_settings(
        path,
        username="user1",
        password="pass1",
        last_file_root="D:/data",
        ai_settings=AISettings(
            enabled=True,
            ai_base_url="https://example.com/ai",
            ai_api_key="secret-ai-key",
            ai_model="vision-model",
            ocr_base_url="https://example.com/ocr",
            ocr_api_key="secret-ocr-key",
            request_timeout_seconds=45,
            image_max_kb=88,
        ),
    )
    loaded = load_settings(path)

    assert loaded.username == "user1"
    assert loaded.password == "pass1"
    assert loaded.last_file_root == "D:/data"
    assert loaded.ai.enabled is True
    assert loaded.ai.ai_base_url == "https://example.com/ai"
    assert loaded.ai.ai_api_key == "secret-ai-key"
    assert loaded.ai.ai_model == "vision-model"
    assert loaded.ai.ocr_base_url == "https://example.com/ocr"
    assert loaded.ai.ocr_api_key == "secret-ocr-key"
    assert loaded.ai.request_timeout_seconds == 45
    assert loaded.ai.image_max_kb == 88


def test_save_settings_preserves_existing_ai_configuration(tmp_path: Path):
    path = tmp_path / "settings.json"
    save_settings(
        path,
        username="user1",
        password="pass1",
        last_file_root="D:/data",
        ai_settings=AISettings(
            enabled=True,
            ai_base_url="https://example.com/ai",
            ai_api_key="secret-ai-key",
            ai_model="vision-model",
            ocr_base_url="https://example.com/ocr",
            ocr_api_key="secret-ocr-key",
            request_timeout_seconds=45,
            image_max_kb=88,
        ),
    )

    save_settings(path, username="user2", password="pass2", last_file_root="/tmp/data")
    loaded = load_settings(path)

    assert loaded.username == "user2"
    assert loaded.password == "pass2"
    assert loaded.last_file_root == "/tmp/data"
    assert loaded.ai.enabled is True
    assert loaded.ai.ai_base_url == "https://example.com/ai"
    assert loaded.ai.ai_api_key == "secret-ai-key"
    assert loaded.ai.ai_model == "vision-model"
    assert loaded.ai.ocr_base_url == "https://example.com/ocr"
    assert loaded.ai.ocr_api_key == "secret-ocr-key"


def test_load_settings_migrates_legacy_ocr_provider_configuration(tmp_path: Path):
    path = tmp_path / "settings.json"
    path.write_text(
        """
{
  "username": "user1",
  "password": "pass1",
  "last_file_root": "D:/data",
  "ai": {
    "enabled": true,
    "provider": "ocr_http",
    "base_url": "https://example.com/ocr",
    "api_key": "legacy-ocr-key",
    "model": "",
    "request_timeout_seconds": 18,
    "image_max_kb": 96
  }
}
        """.strip(),
        encoding="utf-8",
    )

    loaded = load_settings(path)

    assert loaded.ai.enabled is True
    assert loaded.ai.ai_base_url == "https://ark.cn-beijing.volces.com/api/v3"
    assert loaded.ai.ai_api_key == ""
    assert loaded.ai.ai_model == "doubao-seed-2-0-lite-260215"
    assert loaded.ai.ocr_base_url == "https://example.com/ocr"
    assert loaded.ai.ocr_api_key == "legacy-ocr-key"
