from __future__ import annotations

import re
from datetime import date, datetime
from decimal import Decimal

from pypinyin import Style, pinyin


CHINESE_TEXT_PATTERN = re.compile(r"[\u4e00-\u9fff]")


def normalize_text(value) -> str:
    if value is None:
        return ""
    return re.sub(r"\s+", " ", str(value)).strip()


def normalize_phone(value) -> str:
    return re.sub(r"\D+", "", normalize_text(value))


def normalize_project_code(value) -> str:
    return normalize_text(value).upper().replace(" ", "")


def normalize_compact_text(value) -> str:
    return normalize_text(value).replace(" ", "")


def normalize_date(value):
    if value in (None, ""):
        return None
    if isinstance(value, datetime):
        return value.date()
    if isinstance(value, date):
        return value

    text = normalize_text(value)
    for fmt in ("%Y-%m-%d", "%Y/%m/%d"):
        try:
            return datetime.strptime(text, fmt).date()
        except ValueError:
            continue

    compact = re.sub(r"(?<=\d)\s+(?=\d)", "", text)
    match = re.search(r"(\d{4})\s*年\s*(\d{1,2})\s*月\s*(\d{1,2})\s*日", compact)
    if match:
        return date(int(match.group(1)), int(match.group(2)), int(match.group(3)))
    return None


def normalize_amount(value):
    if value in (None, ""):
        return None
    if isinstance(value, Decimal):
        return value
    try:
        return Decimal(str(value).strip())
    except Exception:
        return None


def names_match_by_loose_pinyin(left_value, right_value) -> bool:
    left = normalize_compact_text(left_value)
    right = normalize_compact_text(right_value)
    if not left or not right:
        return False
    if left == right:
        return True
    if not (CHINESE_TEXT_PATTERN.search(left) and CHINESE_TEXT_PATTERN.search(right)):
        return False
    return _normalize_name_pinyin(left) == _normalize_name_pinyin(right)


def _normalize_name_pinyin(value: str) -> str:
    syllables = pinyin(value, style=Style.NORMAL, heteronym=False, errors="default", strict=False)
    return "".join(_normalize_pinyin_syllable(item[0] if item else "") for item in syllables)


def _normalize_pinyin_syllable(value: str) -> str:
    syllable = re.sub(r"[^a-z]", "", normalize_text(value).lower())
    for suffix in ("ang", "eng", "ing", "ong"):
        if syllable.endswith(suffix):
            return syllable[:-1]
    return syllable
