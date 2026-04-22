from __future__ import annotations

import base64
import json
import re
from io import BytesIO
from pathlib import Path

import requests
from PIL import Image, ImageOps

from src.config_store import AISettings
from src.models import PdfData
from src.normalizers import normalize_date, normalize_phone, normalize_text
from src.readers.signature_text_parser import parse_signature_text


AI_PROMPT = (
    "请从这张甲方签字区图片中提取签字人姓名、电话、日期。"
    "只返回一个JSON对象，不要输出额外说明。"
    '字段固定为 signer_name、signer_phone、sign_date。'
    "如果某个字段无法确认，请返回空字符串。"
    "日期统一输出为 YYYY-MM-DD。"
)
OCR_HTTP_PROMPT = "请识别图片中的甲方签字区文字，优先保留姓名、电话、日期。"
REMOTE_JSON_CONTENT_TYPE = "application/json"


def build_chat_completions_url(base_url: str) -> str:
    normalized = normalize_text(base_url).rstrip("/")
    if normalized.endswith("/chat/completions"):
        return normalized
    return f"{normalized}/chat/completions"


def build_ocr_http_url(base_url: str) -> str:
    return normalize_text(base_url).rstrip("/")


def is_ai_recognition_configured(settings: AISettings | None) -> bool:
    if not settings or not settings.enabled:
        return False

    return bool(
        normalize_text(settings.ai_base_url)
        and normalize_text(settings.ai_api_key)
        and normalize_text(settings.ai_model)
    )


def is_ocr_recognition_configured(settings: AISettings | None) -> bool:
    if not settings or not settings.enabled:
        return False

    return bool(normalize_text(settings.ocr_base_url) and normalize_text(settings.ocr_api_key))


def is_remote_recognition_configured(settings: AISettings | None) -> bool:
    return is_ai_recognition_configured(settings) or is_ocr_recognition_configured(settings)


def compress_image_for_remote_service(image_path: Path, max_kb: int) -> bytes:
    limit_bytes = max(1, max_kb) * 1024
    with Image.open(image_path) as image:
        prepared = ImageOps.exif_transpose(image)
        if prepared.mode not in ("RGB", "L"):
            prepared = prepared.convert("RGB")
        elif prepared.mode == "L":
            prepared = prepared.convert("RGB")

        best = _encode_jpeg(prepared, quality=85)
        if len(best) <= limit_bytes:
            return best

        for scale in (1.0, 0.9, 0.8, 0.7, 0.6, 0.5):
            candidate_image = prepared
            if scale != 1.0:
                resized_width = max(1, int(prepared.width * scale))
                resized_height = max(1, int(prepared.height * scale))
                candidate_image = prepared.resize((resized_width, resized_height), Image.Resampling.LANCZOS)
            for quality in (80, 70, 60, 50, 40, 32, 24):
                candidate = _encode_jpeg(candidate_image, quality=quality)
                if len(candidate) < len(best):
                    best = candidate
                if len(candidate) <= limit_bytes:
                    return candidate
        return best


def extract_signature_fields_with_remote_service(image_path: Path, settings: AISettings | None) -> PdfData:
    if not settings or not settings.enabled:
        return PdfData()

    first_error = None

    if is_ai_recognition_configured(settings):
        try:
            ai_parsed = _extract_signature_fields_with_chat_completions(image_path, settings)
        except requests.RequestException as exc:
            first_error = exc
        else:
            if ai_parsed.signer_name or ai_parsed.signer_phone or ai_parsed.sign_date:
                return ai_parsed

    if is_ocr_recognition_configured(settings):
        return _extract_signature_fields_with_ocr_http(image_path, settings)

    if first_error is not None:
        raise first_error

    return PdfData()


def _extract_signature_fields_with_chat_completions(image_path: Path, settings: AISettings) -> PdfData:
    payload = {
        "model": settings.ai_model,
        "messages": [
            {"role": "system", "content": "你是一个文档字段抽取助手。你只能返回JSON。"},
            {
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": _build_image_data_url(image_path, settings)}},
                    {"type": "text", "text": AI_PROMPT},
                ],
            },
        ],
    }
    response = _post_remote_json(
        build_chat_completions_url(settings.ai_base_url),
        settings.ai_api_key,
        payload,
        settings.request_timeout_seconds,
    )
    response.raise_for_status()
    return _parse_ai_pdf_data(response.json())


def _extract_signature_fields_with_ocr_http(image_path: Path, settings: AISettings) -> PdfData:
    payload = {
        "image_base64": _build_image_base64(image_path, settings),
        "image_mime_type": "image/jpeg",
        "prompt": OCR_HTTP_PROMPT,
    }
    response = _post_remote_json(
        build_ocr_http_url(settings.ocr_base_url),
        settings.ocr_api_key,
        payload,
        settings.request_timeout_seconds,
    )
    response.raise_for_status()
    return _parse_ocr_http_pdf_data(response.json())


def _encode_jpeg(image: Image.Image, quality: int) -> bytes:
    buffer = BytesIO()
    image.save(buffer, format="JPEG", quality=quality, optimize=True)
    return buffer.getvalue()


def _build_image_base64(image_path: Path, settings: AISettings) -> str:
    image_bytes = compress_image_for_remote_service(image_path, max_kb=settings.image_max_kb)
    return base64.b64encode(image_bytes).decode("ascii")


def _build_image_data_url(image_path: Path, settings: AISettings) -> str:
    return "data:image/jpeg;base64," + _build_image_base64(image_path, settings)


def _build_remote_headers(api_key: str) -> dict[str, str]:
    return {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": REMOTE_JSON_CONTENT_TYPE,
    }


def _post_remote_json(url: str, api_key: str, payload: dict[str, object], timeout_seconds: int = 30):
    return requests.post(
        url,
        headers=_build_remote_headers(api_key),
        json=payload,
        timeout=timeout_seconds,
    )


def _parse_ai_pdf_data(payload: dict[str, object]) -> PdfData:
    text = _extract_ai_message_text(payload)
    if not text:
        return PdfData()

    return _parse_structured_or_empty(text)


def _parse_ocr_http_pdf_data(payload: dict[str, object]) -> PdfData:
    structured = _parse_json_payload_to_pdf_data(payload)
    if structured is not None:
        return structured

    text = _extract_ocr_http_text(payload)
    if not text:
        return PdfData()
    return parse_signature_text(text)


def _parse_structured_or_empty(text: str) -> PdfData:
    data = _extract_json_object(text)
    if data is None:
        return PdfData()
    return _pdf_data_from_mapping(data)


def _parse_json_payload_to_pdf_data(payload: dict[str, object]) -> PdfData | None:
    if any(key in payload for key in ("signer_name", "signer_phone", "sign_date")):
        return _pdf_data_from_mapping(payload)

    data = payload.get("data")
    if isinstance(data, dict) and any(key in data for key in ("signer_name", "signer_phone", "sign_date")):
        return _pdf_data_from_mapping(data)

    return None


def _pdf_data_from_mapping(data: dict[str, object]) -> PdfData:
    signer_name = normalize_text(data.get("signer_name"))
    signer_phone = normalize_phone(data.get("signer_phone"))
    sign_date = normalize_date(data.get("sign_date"))
    return PdfData(
        signer_name=signer_name,
        signer_phone=signer_phone,
        sign_date=sign_date,
    )


def _extract_ai_message_text(payload: dict[str, object]) -> str:
    choices = payload.get("choices")
    if not isinstance(choices, list) or not choices:
        return ""
    first_choice = choices[0]
    if not isinstance(first_choice, dict):
        return ""
    message = first_choice.get("message")
    if not isinstance(message, dict):
        return ""
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts: list[str] = []
        for item in content:
            if not isinstance(item, dict):
                continue
            text = item.get("text")
            if text:
                parts.append(str(text))
        return "\n".join(parts)
    return ""


def _extract_ocr_http_text(payload: dict[str, object]) -> str:
    direct = payload.get("text")
    if direct:
        return normalize_text(direct)

    result = payload.get("result")
    if isinstance(result, str):
        return normalize_text(result)
    if isinstance(result, dict):
        text = result.get("text")
        if text:
            return normalize_text(text)

    data = payload.get("data")
    if isinstance(data, dict):
        text = data.get("text")
        if text:
            return normalize_text(text)

    return ""


def _extract_json_object(text: str) -> dict[str, object] | None:
    normalized = normalize_text(text)
    if not normalized:
        return None

    try:
        payload = json.loads(normalized)
        if isinstance(payload, dict):
            return payload
    except json.JSONDecodeError:
        pass

    match = re.search(r"\{.*\}", text, re.DOTALL)
    if not match:
        return None
    try:
        payload = json.loads(match.group(0))
    except json.JSONDecodeError:
        return None
    if isinstance(payload, dict):
        return payload
    return None
