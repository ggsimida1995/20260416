from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from pathlib import Path

DEFAULT_AI_BASE_URL = "https://ark.cn-beijing.volces.com/api/v3"
DEFAULT_AI_MODEL = "doubao-seed-2-0-lite-260215"


@dataclass
class AISettings:
    enabled: bool = False
    ai_base_url: str = DEFAULT_AI_BASE_URL
    ai_api_key: str = ""
    ai_model: str = DEFAULT_AI_MODEL
    ocr_base_url: str = ""
    ocr_api_key: str = ""
    request_timeout_seconds: int = 30
    image_max_kb: int = 100


@dataclass
class AppSettings:
    username: str = ""
    password: str = ""
    last_file_root: str = ""
    ai: AISettings = field(default_factory=AISettings)


def load_settings(path: Path) -> AppSettings:
    if not path.exists():
        return AppSettings()

    data = json.loads(path.read_text(encoding="utf-8"))
    ai_data = data.get("ai", {})
    merged_ai = _load_ai_settings(ai_data)
    return AppSettings(
        username=str(data.get("username", "")),
        password=str(data.get("password", "")),
        last_file_root=str(data.get("last_file_root", "")),
        ai=merged_ai,
    )


def save_settings(
    path: Path,
    username: str,
    password: str,
    last_file_root: str,
    ai_settings: AISettings | None = None,
) -> None:
    current = load_settings(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    settings = AppSettings(
        username=username,
        password=password,
        last_file_root=last_file_root,
        ai=ai_settings or current.ai,
    )
    path.write_text(
        json.dumps(asdict(settings), ensure_ascii=False, indent=2),
        encoding="utf-8",
    )


def _load_ai_settings(ai_data: dict[str, object]) -> AISettings:
    enabled = bool(ai_data.get("enabled", False))
    request_timeout_seconds = int(ai_data.get("request_timeout_seconds", AISettings.request_timeout_seconds))
    image_max_kb = int(ai_data.get("image_max_kb", AISettings.image_max_kb))

    if any(key in ai_data for key in ("ai_base_url", "ai_api_key", "ai_model", "ocr_base_url", "ocr_api_key")):
        return AISettings(
            enabled=enabled,
            ai_base_url=str(ai_data.get("ai_base_url", DEFAULT_AI_BASE_URL)),
            ai_api_key=str(ai_data.get("ai_api_key", "")),
            ai_model=str(ai_data.get("ai_model", DEFAULT_AI_MODEL)),
            ocr_base_url=str(ai_data.get("ocr_base_url", "")),
            ocr_api_key=str(ai_data.get("ocr_api_key", "")),
            request_timeout_seconds=request_timeout_seconds,
            image_max_kb=image_max_kb,
        )

    provider = str(ai_data.get("provider", "chat_completions"))
    legacy_base_url = str(ai_data.get("base_url", ""))
    legacy_api_key = str(ai_data.get("api_key", ""))
    legacy_model = str(ai_data.get("model", ""))

    if provider == "ocr_http":
        return AISettings(
            enabled=enabled,
            ai_base_url=DEFAULT_AI_BASE_URL,
            ai_api_key="",
            ai_model=DEFAULT_AI_MODEL,
            ocr_base_url=legacy_base_url,
            ocr_api_key=legacy_api_key,
            request_timeout_seconds=request_timeout_seconds,
            image_max_kb=image_max_kb,
        )

    return AISettings(
        enabled=enabled,
        ai_base_url=legacy_base_url or DEFAULT_AI_BASE_URL,
        ai_api_key=legacy_api_key,
        ai_model=legacy_model or DEFAULT_AI_MODEL,
        ocr_base_url="",
        ocr_api_key="",
        request_timeout_seconds=request_timeout_seconds,
        image_max_kb=image_max_kb,
    )
