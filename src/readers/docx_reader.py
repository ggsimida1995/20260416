from __future__ import annotations

import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from xml.etree import ElementTree as ET
from zipfile import ZipFile

from src.models import DocxData
from src.normalizers import normalize_date, normalize_phone, normalize_text


DATE_PATTERN = re.compile(
    r"(?:\d\s*){4}年\s*(?:\d\s*){1,2}月\s*(?:\d\s*){1,2}日|\d{4}[-/]\d{1,2}[-/]\d{1,2}"
)


def read_docx_text(path: Path) -> str:
    if path.suffix.lower() == ".doc":
        return read_doc_text(path)

    with ZipFile(path) as archive:
        xml = archive.read("word/document.xml")
    root = ET.fromstring(xml)
    namespace = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
    parts = [
        normalize_text(node.text)
        for node in root.findall(".//w:t", namespace)
        if normalize_text(node.text)
    ]
    return " ".join(parts)


def read_doc_text(path: Path) -> str:
    for reader in (_read_doc_text_as_plain, _read_doc_text_with_textutil, _read_doc_text_with_antiword, _read_doc_text_with_soffice):
        text = reader(path)
        if text:
            return text
    raise RuntimeError(f"无法读取 .doc 文件，请安装 Microsoft Word/LibreOffice 或转换为 .docx: {path}")


def parse_docx_text(text: str) -> DocxData:
    cleaned = normalize_text(text)
    contact_names = _extract_values(cleaned, "用户姓名", ["用户姓名", "用户职务", "联系电话", "电子邮件"])
    contact_phones = _extract_phone_values(
        cleaned,
        "联系电话",
        ["联系电话", "电子邮件", "项目经理", "所属部门", "竣工验收", "用户姓名"],
    )
    data = DocxData(
        project_code=_extract_value(cleaned, "项目编号", ["报告日期", "项目全称"]),
        project_name=_extract_value(cleaned, "项目全称", ["项目类型", "项目关注", "用户姓名"]),
        contact_name=contact_names[0] if contact_names else "",
        contact_phone=contact_phones[0] if contact_phones else "",
        contact_names=contact_names,
        contact_phones=contact_phones,
    )

    acceptance_start, acceptance_end = _extract_acceptance_range(cleaned)
    data.acceptance_start = acceptance_start
    data.acceptance_end = acceptance_end
    if acceptance_start and acceptance_end and acceptance_start > acceptance_end:
        data.has_invalid_acceptance_range = True
    return data


def read_docx(path: Path) -> DocxData:
    return parse_docx_text(read_docx_text(path))


def _read_doc_text_as_plain(path: Path) -> str:
    data = path.read_bytes()
    for encoding in ("utf-8", "gb18030", "utf-16"):
        try:
            text = data.decode(encoding)
        except UnicodeDecodeError:
            continue
        cleaned = _strip_rtf_markup(text) if text.lstrip().startswith("{\\rtf") else text
        normalized = normalize_text(cleaned)
        if _looks_like_doc_text(normalized):
            return normalized
    return ""


def _read_doc_text_with_textutil(path: Path) -> str:
    if shutil.which("textutil") is None:
        return ""
    try:
        result = subprocess.run(
            ["textutil", "-convert", "txt", "-stdout", str(path)],
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except Exception:
        return ""
    if result.returncode != 0:
        return ""
    return normalize_text(result.stdout)


def _read_doc_text_with_antiword(path: Path) -> str:
    if shutil.which("antiword") is None:
        return ""
    try:
        result = subprocess.run(
            ["antiword", str(path)],
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except Exception:
        return ""
    if result.returncode != 0:
        return ""
    return normalize_text(result.stdout)


def _read_doc_text_with_soffice(path: Path) -> str:
    soffice = shutil.which("soffice") or shutil.which("libreoffice")
    if soffice is None:
        return ""
    with tempfile.TemporaryDirectory(prefix="project-file-compare-doc-") as temp_dir:
        try:
            result = subprocess.run(
                [soffice, "--headless", "--convert-to", "txt:Text", "--outdir", temp_dir, str(path)],
                check=False,
                capture_output=True,
                text=True,
                timeout=60,
            )
        except Exception:
            return ""
        if result.returncode != 0:
            return ""
        output_path = Path(temp_dir) / f"{path.stem}.txt"
        if not output_path.exists():
            return ""
        return normalize_text(output_path.read_text(encoding="utf-8", errors="ignore"))


def _strip_rtf_markup(text: str) -> str:
    text = re.sub(r"\\'[0-9a-fA-F]{2}", " ", text)
    text = re.sub(r"\\[a-zA-Z]+\d* ?", " ", text)
    return re.sub(r"[{}]", " ", text)


def _looks_like_doc_text(text: str) -> bool:
    return any(label in text for label in ("项目编号", "项目全称", "用户姓名", "联系电话", "竣工验收"))


def _extract_value(text: str, label: str, stop_labels: list[str]) -> str:
    values = _extract_values(text, label, stop_labels)
    if not values:
        return ""
    return values[0]


def _extract_values(text: str, label: str, stop_labels: list[str]) -> list[str]:
    values: list[str] = []
    search_from = 0
    while True:
        start = text.find(label, search_from)
        if start == -1:
            return _dedupe(values)

        value_start = start + len(label)
        tail = text[value_start:]
        stop_positions = [tail.find(stop) for stop in stop_labels if tail.find(stop) != -1]
        if stop_positions:
            tail = tail[: min(stop_positions)]

        value = normalize_text(tail).lstrip("：:")
        if value:
            values.append(value)

        search_from = value_start


def _extract_phone_values(text: str, label: str, stop_labels: list[str]) -> list[str]:
    numbers: list[str] = []
    for value in _extract_values(text, label, stop_labels):
        digits = normalize_phone(value)
        match = re.search(r"1\d{10}", digits)
        if match:
            numbers.append(match.group(0))
    return _dedupe(numbers)


def _dedupe(values: list[str]) -> list[str]:
    seen: set[str] = set()
    unique_values: list[str] = []
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        unique_values.append(value)
    return unique_values


def _extract_acceptance_range(text: str):
    if "竣工验收" not in text:
        return None, None

    start = text.index("竣工验收") + len("竣工验收")
    tail = text[start : start + 120]
    matches = DATE_PATTERN.findall(tail)
    if len(matches) < 2:
        return None, None

    return normalize_date(matches[0]), normalize_date(matches[1])
