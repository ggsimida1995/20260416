from __future__ import annotations

import re

from src.models import PdfData
from src.normalizers import normalize_date, normalize_phone, normalize_text


DATE_PATTERN = re.compile(
    r"(?:\d\s*){4}年\s*(?:\d\s*){1,2}月\s*(?:\d\s*){1,2}日|\d{4}[-/]\d{1,2}[-/]\d{1,2}"
)


def parse_signature_text(text: str) -> PdfData:
    cleaned = normalize_text(text)
    sign_date = None
    date_match = DATE_PATTERN.search(cleaned)
    if date_match:
        sign_date = normalize_date(date_match.group(0))

    return PdfData(
        signer_name=_extract_first_available(
            cleaned,
            [
                ("签字人姓名", ["签字人姓名", "签字/盖章", "联系电话", "电话", "签字时间", "日期"]),
                ("签字/盖章", ["签字/盖章", "联系电话", "电话", "签字时间", "日期"]),
            ],
        ),
        signer_phone=_extract_phone_after_labels(cleaned, ["联系电话", "电话"]),
        sign_date=sign_date,
    )


def _extract_value(text: str, label: str, stop_labels: list[str]) -> str:
    if label not in text:
        return ""
    start = text.index(label) + len(label)
    tail = text[start:]

    stop_positions = [tail.find(stop) for stop in stop_labels if tail.find(stop) != -1]
    if stop_positions:
        tail = tail[: min(stop_positions)]

    return normalize_text(tail).lstrip("：:")


def _extract_first_available(text: str, rules: list[tuple[str, list[str]]]) -> str:
    for label, stop_labels in rules:
        value = _extract_value(text, label, stop_labels)
        if value:
            return value
    return ""


def _extract_phone_after_labels(text: str, labels: list[str]) -> str:
    for label in labels:
        if label not in text:
            continue
        start = text.index(label) + len(label)
        tail = text[start : start + 40]
        digits = normalize_phone(tail)
        match = re.search(r"1\d{10}", digits)
        if match:
            return match.group(0)
    return ""
