import base64
from pathlib import Path

from PIL import Image

from src.config_store import AISettings
from src.readers import signature_ai
from src.readers.signature_ai import (
    build_chat_completions_url,
    compress_image_for_remote_service,
    extract_signature_fields_with_remote_service,
)


def test_build_chat_completions_url_appends_endpoint_once():
    assert build_chat_completions_url("https://example.com/api/v3") == "https://example.com/api/v3/chat/completions"
    assert (
        build_chat_completions_url("https://example.com/api/v3/chat/completions")
        == "https://example.com/api/v3/chat/completions"
    )


def test_compress_image_for_remote_service_keeps_image_under_limit(tmp_path: Path):
    image_path = tmp_path / "source.png"
    Image.new("RGB", (2200, 1800), color="white").save(image_path)

    compressed = compress_image_for_remote_service(image_path, max_kb=100)

    assert len(compressed) <= 100 * 1024


def test_legacy_ai_only_entrypoints_are_removed():
    assert hasattr(signature_ai, "extract_signature_fields_with_ai") is False
    assert hasattr(signature_ai, "compress_image_for_ai") is False


def test_extract_signature_fields_with_remote_service_posts_image_and_parses_chat_response(monkeypatch, tmp_path: Path):
    image_path = tmp_path / "signature.png"
    Image.new("RGB", (1400, 800), color="white").save(image_path)
    captured = {}

    class FakeResponse:
        def raise_for_status(self) -> None:
            return None

        def json(self):
            return {
                "choices": [
                    {
                        "message": {
                            "content": '{"signer_name":"黄汉明","signer_phone":"14714691425","sign_date":"2026-04-10"}'
                        }
                    }
                ]
            }

    def fake_post(url, headers=None, json=None, timeout=None):
        captured["url"] = url
        captured["headers"] = headers
        captured["json"] = json
        captured["timeout"] = timeout
        return FakeResponse()

    monkeypatch.setattr("src.readers.signature_ai.requests.post", fake_post)

    parsed = extract_signature_fields_with_remote_service(
        image_path,
        AISettings(
            enabled=True,
            ai_base_url="https://example.com/api/v3",
            ai_api_key="secret-key",
            ai_model="vision-model",
            request_timeout_seconds=18,
            image_max_kb=100,
        ),
    )

    assert captured["url"] == "https://example.com/api/v3/chat/completions"
    assert captured["headers"]["Authorization"] == "Bearer secret-key"
    assert captured["timeout"] == 18
    image_url = captured["json"]["messages"][1]["content"][0]["image_url"]["url"]
    assert image_url.startswith("data:image/jpeg;base64,")
    decoded = base64.b64decode(image_url.split(",", 1)[1])
    assert len(decoded) <= 100 * 1024
    assert parsed.signer_name == "黄汉明"
    assert parsed.signer_phone == "14714691425"
    assert parsed.sign_date.isoformat() == "2026-04-10"


def test_extract_signature_fields_with_remote_service_returns_empty_when_chat_response_is_not_json(monkeypatch, tmp_path: Path):
    image_path = tmp_path / "signature.png"
    Image.new("RGB", (800, 400), color="white").save(image_path)

    class FakeResponse:
        def raise_for_status(self) -> None:
            return None

        def json(self):
            return {
                "choices": [
                    {
                        "message": {
                            "content": "无法识别"
                        }
                    }
                ]
            }

    monkeypatch.setattr("src.readers.signature_ai.requests.post", lambda *args, **kwargs: FakeResponse())

    parsed = extract_signature_fields_with_remote_service(
        image_path,
        AISettings(
            enabled=True,
            ai_base_url="https://example.com/api/v3",
            ai_api_key="secret-key",
            ai_model="vision-model",
        ),
    )

    assert parsed.signer_name == ""
    assert parsed.signer_phone == ""
    assert parsed.sign_date is None


def test_extract_signature_fields_with_remote_service_routes_to_ocr_http_and_parses_text_response(
    monkeypatch, tmp_path: Path
):
    image_path = tmp_path / "signature.png"
    Image.new("RGB", (1200, 700), color="white").save(image_path)
    captured = {}

    class FakeResponse:
        def raise_for_status(self) -> None:
            return None

        def json(self):
            return {"text": "签字/盖章：黄汉明 电话：14714691425 2026年4月10日"}

    def fake_post(url, headers=None, json=None, timeout=None):
        captured["url"] = url
        captured["headers"] = headers
        captured["json"] = json
        captured["timeout"] = timeout
        return FakeResponse()

    monkeypatch.setattr("src.readers.signature_ai.requests.post", fake_post)

    parsed = extract_signature_fields_with_remote_service(
        image_path,
        AISettings(
            enabled=True,
            ocr_base_url="https://example.com/ocr",
            ocr_api_key="secret-key",
            request_timeout_seconds=18,
            image_max_kb=100,
        ),
    )

    assert captured["url"] == "https://example.com/ocr"
    assert captured["headers"]["Authorization"] == "Bearer secret-key"
    assert captured["timeout"] == 18
    assert captured["json"]["image_base64"]
    assert captured["json"]["image_mime_type"] == "image/jpeg"
    assert parsed.signer_name == "黄汉明"
    assert parsed.signer_phone == "14714691425"
    assert parsed.sign_date.isoformat() == "2026-04-10"


def test_extract_signature_fields_with_remote_service_falls_back_to_ocr_when_ai_is_not_configured(
    monkeypatch, tmp_path: Path
):
    image_path = tmp_path / "signature.png"
    Image.new("RGB", (1200, 700), color="white").save(image_path)
    captured = {}

    class FakeResponse:
        def raise_for_status(self) -> None:
            return None

        def json(self):
            return {"text": "签字/盖章：黄汉明 电话：14714691425 2026年4月10日"}

    def fake_post(url, headers=None, json=None, timeout=None):
        captured["url"] = url
        captured["headers"] = headers
        captured["json"] = json
        captured["timeout"] = timeout
        return FakeResponse()

    monkeypatch.setattr("src.readers.signature_ai.requests.post", fake_post)

    parsed = extract_signature_fields_with_remote_service(
        image_path,
        AISettings(
            enabled=True,
            ocr_base_url="https://example.com/ocr",
            ocr_api_key="secret-key",
            request_timeout_seconds=18,
            image_max_kb=100,
        ),
    )

    assert captured["url"] == "https://example.com/ocr"
    assert "model" not in captured["json"]
    assert parsed.signer_name == "黄汉明"


def test_extract_signature_fields_with_remote_service_falls_back_to_ocr_when_ai_returns_empty(
    monkeypatch, tmp_path: Path
):
    image_path = tmp_path / "signature.png"
    Image.new("RGB", (1200, 700), color="white").save(image_path)
    captured_urls: list[str] = []

    class FakeResponse:
        def __init__(self, payload):
            self._payload = payload

        def raise_for_status(self) -> None:
            return None

        def json(self):
            return self._payload

    def fake_post(url, headers=None, json=None, timeout=None):
        captured_urls.append(url)
        if url.endswith("/chat/completions"):
            return FakeResponse({"choices": [{"message": {"content": "{}"}}]})
        return FakeResponse({"text": "签字/盖章：黄汉明 电话：14714691425 2026年4月10日"})

    monkeypatch.setattr("src.readers.signature_ai.requests.post", fake_post)

    parsed = extract_signature_fields_with_remote_service(
        image_path,
        AISettings(
            enabled=True,
            ai_base_url="https://example.com/api/v3",
            ai_api_key="secret-ai-key",
            ai_model="vision-model",
            ocr_base_url="https://example.com/ocr",
            ocr_api_key="secret-ocr-key",
        ),
    )

    assert captured_urls == [
        "https://example.com/api/v3/chat/completions",
        "https://example.com/ocr",
    ]
    assert parsed.signer_name == "黄汉明"
