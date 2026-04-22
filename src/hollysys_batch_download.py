from __future__ import annotations

import argparse
import base64
import ctypes
import hashlib
import html as html_lib
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any
from urllib.parse import parse_qsl, urlencode, urljoin, urlsplit, urlunsplit

import requests
from Crypto.Cipher import AES
from lxml import html as lxml_html

from src.config import DEBUG_ROOT, FILE_ROOT


def _default_chrome_user_data_dir() -> Path:
    if sys.platform == "win32":
        local_app_data = os.environ.get("LOCALAPPDATA")
        if local_app_data:
            return Path(local_app_data) / "Google" / "Chrome" / "User Data"
        return Path.home() / "AppData" / "Local" / "Google" / "Chrome" / "User Data"
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "Google" / "Chrome"
    return Path.home() / ".config" / "google-chrome"


def _default_cookie_db() -> Path:
    user_data_dir = _default_chrome_user_data_dir()
    if sys.platform == "win32":
        return user_data_dir / "Default" / "Network" / "Cookies"
    return user_data_dir / "Default" / "Cookies"


BASE_URL = "https://www.hollysys.net"
DEFAULT_COOKIE_DB = _default_cookie_db()
DEFAULT_OUTPUT_ROOT = FILE_ROOT
DEFAULT_SUMMARY_PATH = DEBUG_ROOT / "hollysys" / "latest_batch_summary.json"
CHROME_SAFE_STORAGE_SERVICE = "Chrome Safe Storage"
CHROME_COOKIE_SALT = b"saltysalt"
CHROME_COOKIE_IV = b" " * 16
CHROME_COOKIE_ITERATIONS = 1003
CHROME_COOKIE_KEY_LENGTH = 16
PROJECT_CODE_PATTERN = re.compile(r"项目号[:：]\s*([A-Z]+-\d+(?:/[A-Z0-9]+)?)", re.IGNORECASE)
ATTACHMENT_CALL_TEMPLATE = (
    r'attachmentObject_{flag_id}\.addDoc\('
    r'"((?:\\.|[^"\\])*)",'
    r'"([0-9a-f]+)",'
    r'(?:true|false),'
    r'"((?:\\.|[^"\\])*)",'
    r'"((?:\\.|[^"\\])*)",'
    r'"((?:\\.|[^"\\])*)",'
    r'"((?:\\.|[^"\\])*)"\s*\);'
)


@dataclass(frozen=True)
class AggregationCategory:
    aggregation_id: str
    name: str


@dataclass(frozen=True)
class Attachment:
    name: str
    fd_id: str
    mime_type: str
    size: str
    file_key: str

    @property
    def download_url(self) -> str:
        return (
            f"{BASE_URL}/sys/attachment/sys_att_main/sysAttMain.do"
            f"?method=download&fdId={self.fd_id}"
        )


@dataclass(frozen=True)
class TodoItem:
    category: AggregationCategory
    todo_fd_id: str
    subject: str
    detail_path: str
    project_code_hint: str

    @property
    def detail_url(self) -> str:
        absolute = urljoin(f"{BASE_URL}/", self.detail_path.lstrip("/"))
        return _merge_query_parameter(absolute, "LLType", "PC")

    @property
    def notify_view_url(self) -> str:
        return (
            f"{BASE_URL}/sys/notify/sys_notify_todo/sysNotifyTodo.do"
            f"?method=view&fdId={self.todo_fd_id}"
        )


@dataclass(frozen=True)
class DetailRecord:
    item: TodoItem
    project_code: str
    project_name: str
    attachments: tuple[Attachment, ...]


@dataclass(frozen=True)
class SessionInspectionResult:
    status: str
    detail: str
    cookie_db_exists: bool
    cookie_db_path: str
    hollysys_cookie_count: int
    cookie_names: tuple[str, ...]
    safe_storage_available: bool
    authenticated: bool
    http_status: int
    final_url: str


AGGREGATION_CATEGORIES = (
    AggregationCategory("18a032b3695468f23f38a0f40d5a3602", "项目关闭工作流"),
    AggregationCategory("18a032b4e48b3ad71bf4c08405487452", "项目关闭工作流(工软分包项目)"),
)


def build_authenticated_session(cookie_db: Path | None = None) -> requests.Session:
    cookie_db = _resolve_cookie_db(cookie_db)
    if not cookie_db.exists():
        raise FileNotFoundError(f"未找到 Chrome Cookie 数据库: {cookie_db}")

    temp_dir = Path(tempfile.mkdtemp(prefix="hollysys-cookie-"))
    temp_cookie_db = temp_dir / "Cookies"
    shutil.copy2(cookie_db, temp_cookie_db)

    session = requests.Session()
    try:
        rows = _read_hollysys_cookie_rows(temp_cookie_db)
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)

    key = _read_chrome_cookie_encryption_key(cookie_db, rows)
    for host_key, name, encrypted_value in rows:
        value = _decrypt_chrome_cookie(host_key, encrypted_value, key)
        session.cookies.set(name, value, domain=host_key.lstrip("."), path="/")

    session.headers.update(
        {
            "User-Agent": (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
                "AppleWebKit/537.36 (KHTML, like Gecko) "
                "Chrome/135.0.0.0 Safari/537.36"
            )
        }
    )
    return session


def inspect_local_hollysys_session(
    cookie_db: Path | None = None,
    *,
    timeout_seconds: float = 10.0,
) -> SessionInspectionResult:
    if cookie_db is None:
        return _inspect_candidate_cookie_dbs(timeout_seconds=timeout_seconds)

    cookie_db = _resolve_cookie_db(cookie_db)
    cookie_db_path = str(cookie_db)
    if not cookie_db.exists():
        return SessionInspectionResult(
            status="missing",
            detail=f"未找到 Chrome Cookie 数据库: {cookie_db}",
            cookie_db_exists=False,
            cookie_db_path=cookie_db_path,
            hollysys_cookie_count=0,
            cookie_names=(),
            safe_storage_available=False,
            authenticated=False,
            http_status=0,
            final_url="",
        )

    try:
        rows = _read_hollysys_cookie_rows(cookie_db)
    except Exception as exc:
        return SessionInspectionResult(
            status="error",
            detail=f"读取 Hollysys Cookie 失败: {exc}",
            cookie_db_exists=True,
            cookie_db_path=cookie_db_path,
            hollysys_cookie_count=0,
            cookie_names=(),
            safe_storage_available=False,
            authenticated=False,
            http_status=0,
            final_url="",
        )

    cookie_names = tuple(sorted({str(name) for _, name, _ in rows}))
    if not rows:
        return SessionInspectionResult(
            status="missing",
            detail="已找到 Chrome，但未发现 Hollysys 相关 Cookie。",
            cookie_db_exists=True,
            cookie_db_path=cookie_db_path,
            hollysys_cookie_count=0,
            cookie_names=(),
            safe_storage_available=False,
            authenticated=False,
            http_status=0,
            final_url="",
        )

    try:
        session = build_authenticated_session(cookie_db)
    except Exception as exc:
        return SessionInspectionResult(
            status="error",
            detail=f"Cookie 解密失败: {exc}",
            cookie_db_exists=True,
            cookie_db_path=cookie_db_path,
            hollysys_cookie_count=len(rows),
            cookie_names=cookie_names,
            safe_storage_available=False,
            authenticated=False,
            http_status=0,
            final_url="",
        )

    try:
        response = session.get(f"{BASE_URL}/sys/aggregation/", timeout=timeout_seconds)
    except Exception as exc:
        return SessionInspectionResult(
            status="error",
            detail=f"会话探测请求失败: {exc}",
            cookie_db_exists=True,
            cookie_db_path=cookie_db_path,
            hollysys_cookie_count=len(rows),
            cookie_names=cookie_names,
            safe_storage_available=True,
            authenticated=False,
            http_status=0,
            final_url="",
        )

    response_text = response.text[:8000]
    final_url = response.url
    if response.status_code == 200 and "/sys/aggregation/" in final_url and "待办事宜" in response_text:
        return SessionInspectionResult(
            status="ready",
            detail="已拿到可用 Hollysys 会话，可直接访问待办事宜。",
            cookie_db_exists=True,
            cookie_db_path=cookie_db_path,
            hollysys_cookie_count=len(rows),
            cookie_names=cookie_names,
            safe_storage_available=True,
            authenticated=True,
            http_status=response.status_code,
            final_url=final_url,
        )

    if "login" in final_url.lower() or "扫码" in response_text or "登录" in response_text:
        return SessionInspectionResult(
            status="expired",
            detail="检测到 Hollysys Cookie，但当前会话已失效或需要重新登录。",
            cookie_db_exists=True,
            cookie_db_path=cookie_db_path,
            hollysys_cookie_count=len(rows),
            cookie_names=cookie_names,
            safe_storage_available=True,
            authenticated=False,
            http_status=response.status_code,
            final_url=final_url,
        )

    return SessionInspectionResult(
        status="error",
        detail="已读取到 Cookie，但无法确认当前 Hollysys 会话是否可用。",
        cookie_db_exists=True,
        cookie_db_path=cookie_db_path,
        hollysys_cookie_count=len(rows),
        cookie_names=cookie_names,
        safe_storage_available=True,
        authenticated=False,
        http_status=response.status_code,
        final_url=final_url,
    )


def fetch_todo_items(session: requests.Session, category: AggregationCategory) -> list[TodoItem]:
    response = session.get(_build_list_url(category.aggregation_id), timeout=30)
    response.raise_for_status()
    payload = json.loads(response.text.lstrip())

    items: list[TodoItem] = []
    for raw_row in payload.get("datas", []):
        row = _list_row_to_dict(raw_row)
        detail_path = str(row.get("tr_href") or row.get("_tr_href") or "")
        if not detail_path:
            continue
        subject = _strip_html(str(row.get("todo.subject4View", "")))
        items.append(
            TodoItem(
                category=category,
                todo_fd_id=str(row.get("fdId", "")),
                subject=subject,
                detail_path=detail_path,
                project_code_hint=_extract_project_code(subject),
            )
        )
    return items


def fetch_detail_record(session: requests.Session, item: TodoItem) -> DetailRecord:
    response = session.get(item.detail_url, timeout=30)
    response.raise_for_status()
    doc = lxml_html.fromstring(response.text)

    project_code = extract_detail_field(doc, "项目编号") or item.project_code_hint
    project_name = extract_detail_field(doc, "项目名称")
    attachments = tuple(extract_section_attachments(doc, "关闭依据附件"))

    if not project_code:
        raise ValueError(f"详情页未解析到项目编号: {item.detail_url}")
    if not project_name:
        raise ValueError(f"详情页未解析到项目名称: {item.detail_url}")

    return DetailRecord(
        item=item,
        project_code=project_code,
        project_name=project_name,
        attachments=attachments,
    )


def extract_detail_field(doc: lxml_html.HtmlElement, label: str) -> str:
    nodes = doc.xpath(f"//label[normalize-space()={json.dumps(label, ensure_ascii=False)}]")
    if not nodes:
        return ""

    value_cell = nodes[0].getparent().getnext()
    if value_cell is None:
        return ""

    for candidate in value_cell.xpath(".//xformflag[@flagtype='xform_text' or @_xform_type='text']"):
        value = _clean_whitespace(candidate.text_content())
        if value:
            return value

    for text_value in value_cell.xpath(".//text()"):
        value = _clean_whitespace(str(text_value))
        if value:
            return value
    return ""


def extract_section_attachments(
    doc: lxml_html.HtmlElement,
    section_label: str,
) -> list[Attachment]:
    nodes = doc.xpath(f"//label[normalize-space()={json.dumps(section_label, ensure_ascii=False)}]")
    if not nodes:
        return []

    section_cell = nodes[0].getparent().getnext()
    if section_cell is None:
        return []

    section_html = lxml_html.tostring(section_cell, encoding="unicode")
    flag_ids = section_cell.xpath(
        ".//xformflag[@flagtype='xform_relation_attachment' or @_xform_type='attachment']/@flagid"
    )

    attachments: list[Attachment] = []
    seen_fd_ids: set[str] = set()
    for flag_id in flag_ids:
        pattern = re.compile(
            ATTACHMENT_CALL_TEMPLATE.format(flag_id=re.escape(flag_id)),
            re.DOTALL,
        )
        for match in pattern.finditer(section_html):
            fd_id = match.group(2)
            if fd_id in seen_fd_ids:
                continue
            seen_fd_ids.add(fd_id)
            attachments.append(
                Attachment(
                    name=_decode_js_string(match.group(1)),
                    fd_id=fd_id,
                    mime_type=_decode_js_string(match.group(3)),
                    size=_decode_js_string(match.group(4)),
                    file_key=_decode_js_string(match.group(5)),
                )
            )
    return attachments


def select_target_attachments(attachments: list[Attachment] | tuple[Attachment, ...]) -> list[Attachment]:
    remaining = [attachment for attachment in attachments if not _is_message_attachment(attachment)]
    selected: list[Attachment] = []

    selected.extend(_pick_best_attachment(remaining, keywords=("项目关闭移交登记表",), suffixes=(".xlsx", ".xls")))
    selected.extend(_pick_best_attachment(remaining, keywords=("项目竣工总结报告",), suffixes=(".docx", ".doc")))
    selected.extend(
        _pick_best_attachment(
            remaining,
            keywords=("验收报告", "开箱验收单", "开箱验收"),
            suffixes=(".pdf", ".jpg", ".jpeg", ".png"),
        )
    )

    seen = {attachment.fd_id for attachment in selected}
    for attachment in remaining:
        if attachment.fd_id in seen:
            continue
        selected.append(attachment)
        seen.add(attachment.fd_id)
        if len(selected) >= 3:
            break

    return selected[:3]


def save_record(
    session: requests.Session,
    record: DetailRecord,
    output_root: Path,
) -> dict[str, Any]:
    project_dir = output_root / normalize_project_code(record.project_code)
    project_dir.mkdir(parents=True, exist_ok=True)

    selected_attachments = select_target_attachments(record.attachments)
    saved_files: list[str] = []
    for attachment in selected_attachments:
        destination = project_dir / sanitize_filename(attachment.name)
        download_attachment(session, attachment, destination, referer=record.item.detail_url)
        saved_files.append(destination.name)

    info_path = project_dir / f"{project_dir.name}.txt"
    info_path.write_text(
        "\n".join(
            [
                f"项目编号: {record.project_code}",
                f"项目名称: {record.project_name}",
                f"来源分类: {record.item.category.name}",
                f"详情页: {record.item.detail_url}",
                f"待办页: {record.item.notify_view_url}",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    return {
        "project_dir": str(project_dir),
        "project_code": record.project_code,
        "project_name": record.project_name,
        "saved_files": saved_files,
        "selected_attachment_count": len(selected_attachments),
        "all_attachment_count": len(record.attachments),
        "selected_attachments": [_attachment_to_dict(attachment) for attachment in selected_attachments],
        "all_attachments": [_attachment_to_dict(attachment) for attachment in record.attachments],
        "detail_url": record.item.detail_url,
        "notify_view_url": record.item.notify_view_url,
        "category": record.item.category.name,
    }


def download_attachment(
    session: requests.Session,
    attachment: Attachment,
    destination: Path,
    *,
    referer: str,
) -> None:
    response = session.get(
        attachment.download_url,
        headers={"Referer": referer},
        timeout=60,
    )
    response.raise_for_status()

    content = response.content
    if _looks_like_html(content):
        raise ValueError(f"附件下载返回了 HTML 页面，疑似会话失效: {attachment.download_url}")

    destination.write_bytes(content)


def run_batch(
    *,
    output_root: Path = DEFAULT_OUTPUT_ROOT,
    summary_path: Path = DEFAULT_SUMMARY_PATH,
    limit: int | None = None,
    skip_project_codes: set[str] | None = None,
    log_callback=None,
) -> dict[str, Any]:
    output_root.mkdir(parents=True, exist_ok=True)
    summary_path.parent.mkdir(parents=True, exist_ok=True)

    session = build_authenticated_session()
    skipped_codes = {normalize_project_code(code) for code in (skip_project_codes or set())}
    summary: dict[str, Any] = {
        "output_root": str(output_root.resolve()),
        "categories": [],
        "processed_count": 0,
        "saved_project_dirs": [],
        "skipped_projects": [],
        "errors": [],
    }

    processed = 0
    if log_callback is not None:
        log_callback("[网页阶段] 使用 Chrome 已登录会话直连 Hollysys")

    for category in AGGREGATION_CATEGORIES:
        items = fetch_todo_items(session, category)
        if log_callback is not None:
            log_callback(f"[网页阶段] 分类待办: {category.name} | {len(items)}")
        category_summary: dict[str, Any] = {
            "aggregation_id": category.aggregation_id,
            "name": category.name,
            "todo_count": len(items),
            "projects": [],
        }

        for item in items:
            if limit is not None and processed >= limit:
                break

            try:
                record = fetch_detail_record(session, item)
                normalized_code = normalize_project_code(record.project_code)
                if normalized_code in skipped_codes:
                    summary["skipped_projects"].append(
                        {
                            "project_code": record.project_code,
                            "project_dir_name": normalized_code,
                            "category": category.name,
                            "detail_url": record.item.detail_url,
                        }
                    )
                    if log_callback is not None:
                        log_callback(f"[网页阶段] 跳过已处理项目: {normalized_code}")
                    continue
                saved = save_record(session, record, output_root)
                category_summary["projects"].append(saved)
                summary["saved_project_dirs"].append(saved["project_dir"])
                processed += 1
                if log_callback is not None:
                    log_callback(
                        "[网页阶段] 已下载项目: "
                        f"{normalized_code} | 文件={saved['selected_attachment_count']}/{saved['all_attachment_count']}"
                    )
            except Exception as exc:  # pragma: no cover - exercised in live run
                summary["errors"].append(
                    {
                        "category": category.name,
                        "subject": item.subject,
                        "detail_url": item.detail_url,
                        "error": str(exc),
                    }
                )
                if log_callback is not None:
                    log_callback(f"[网页阶段] 失败: {item.detail_url} | {exc}")

        summary["categories"].append(category_summary)
        if limit is not None and processed >= limit:
            break

    summary["processed_count"] = processed
    summary_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    if log_callback is not None:
        log_callback(f"[网页阶段] 汇总文件: {summary_path}")
    return summary


def normalize_project_code(project_code: str) -> str:
    return _clean_whitespace(project_code).replace("/", "-")


def sanitize_filename(filename: str) -> str:
    cleaned = re.sub(r"[\\/]+", "-", _clean_whitespace(filename))
    return cleaned or "unnamed"


def _build_list_url(aggregation_id: str) -> str:
    return (
        f"{BASE_URL}/sys/notify/sys_notify_todo/sysNotifyMainIndex.do"
        f"?method=list&from=aggregation&dataType=todo&fdType=13&aggregationId={aggregation_id}"
    )


def _list_row_to_dict(raw_row: Any) -> dict[str, str]:
    row: dict[str, str] = {}
    if not isinstance(raw_row, list):
        return row
    for entry in raw_row:
        if not isinstance(entry, dict):
            continue
        column = str(entry.get("col", ""))
        if not column:
            continue
        row[column] = str(entry.get("value", ""))
    return row


def _attachment_to_dict(attachment: Attachment) -> dict[str, str]:
    payload = asdict(attachment)
    payload["download_url"] = attachment.download_url
    return payload


def _strip_html(value: str) -> str:
    stripped = re.sub(r"<[^>]+>", " ", value)
    return _clean_whitespace(html_lib.unescape(stripped))


def _extract_project_code(value: str) -> str:
    match = PROJECT_CODE_PATTERN.search(value)
    return match.group(1) if match is not None else ""


def _pick_best_attachment(
    attachments: list[Attachment],
    *,
    keywords: tuple[str, ...],
    suffixes: tuple[str, ...],
) -> list[Attachment]:
    for attachment in attachments:
        if _attachment_matches(attachment, keywords=keywords, suffixes=suffixes):
            return [attachment]
    return []


def _attachment_matches(
    attachment: Attachment,
    *,
    keywords: tuple[str, ...],
    suffixes: tuple[str, ...],
) -> bool:
    name = attachment.name.lower()
    return any(keyword.lower() in name for keyword in keywords) and name.endswith(suffixes)


def _is_message_attachment(attachment: Attachment) -> bool:
    suffix = Path(attachment.name).suffix.lower()
    return suffix in {".eml", ".msg"} or attachment.mime_type == "message/rfc822"


def _looks_like_html(content: bytes) -> bool:
    prefix = content.lstrip()[:128].lower()
    return prefix.startswith(b"<!doctype html") or prefix.startswith(b"<html")


def _clean_whitespace(value: str) -> str:
    return re.sub(r"\s+", " ", value or "").strip()


def _decode_js_string(value: str) -> str:
    return json.loads(f'"{value}"')


def _merge_query_parameter(url: str, key: str, value: str) -> str:
    parts = urlsplit(url)
    query = dict(parse_qsl(parts.query, keep_blank_values=True))
    query[key] = value
    return urlunsplit((parts.scheme, parts.netloc, parts.path, urlencode(query), parts.fragment))


def _read_hollysys_cookie_rows(cookie_db: Path) -> list[tuple[str, str, bytes]]:
    connection = sqlite3.connect(f"file:{cookie_db}?mode=ro", uri=True)
    try:
        rows = connection.execute(
            """
            SELECT host_key, name, encrypted_value
            FROM cookies
            WHERE host_key LIKE ? AND name <> ''
            ORDER BY host_key ASC, name ASC
            """,
            ("%hollysys.net%",),
        ).fetchall()
    finally:
        connection.close()

    normalized_rows: list[tuple[str, str, bytes]] = []
    for host_key, name, encrypted_value in rows:
        if isinstance(encrypted_value, memoryview):
            encrypted_value = encrypted_value.tobytes()
        elif encrypted_value is None:
            encrypted_value = b""
        normalized_rows.append((str(host_key), str(name), bytes(encrypted_value)))
    return normalized_rows


def _resolve_cookie_db(cookie_db: Path | None) -> Path:
    if cookie_db is not None:
        return cookie_db

    candidates = _candidate_cookie_dbs()
    existing = [candidate for candidate in candidates if candidate.exists()]
    for candidate in existing:
        try:
            if _read_hollysys_cookie_rows(candidate):
                return candidate
        except Exception:
            continue
    if existing:
        return existing[0]
    return candidates[0] if candidates else DEFAULT_COOKIE_DB


def _candidate_cookie_dbs() -> list[Path]:
    user_data_dir = _default_chrome_user_data_dir()
    profile_dirs: list[Path] = [user_data_dir / "Default"]
    if user_data_dir.exists():
        profile_dirs.extend(
            sorted(
                (
                    path
                    for path in user_data_dir.iterdir()
                    if path.is_dir() and re.fullmatch(r"Profile \d+", path.name)
                ),
                key=lambda path: path.name,
            )
        )

    relative_paths = ("Network/Cookies", "Cookies") if sys.platform == "win32" else ("Cookies",)
    candidates: list[Path] = []
    for profile_dir in profile_dirs:
        for relative_path in relative_paths:
            candidate = profile_dir / relative_path
            if candidate not in candidates:
                candidates.append(candidate)
    return candidates


def _inspect_candidate_cookie_dbs(*, timeout_seconds: float) -> SessionInspectionResult:
    candidates = _candidate_cookie_dbs()
    results: list[SessionInspectionResult] = []
    for candidate in candidates:
        result = inspect_local_hollysys_session(candidate, timeout_seconds=timeout_seconds)
        results.append(result)
        if result.status == "ready":
            return result
    if results:
        priority = {"ready": 4, "expired": 3, "error": 2, "missing": 1}
        return max(
            results,
            key=lambda result: (
                priority.get(result.status, 0),
                result.hollysys_cookie_count,
                int(result.cookie_db_exists),
            ),
        )
    return SessionInspectionResult(
        status="missing",
        detail=f"未找到 Chrome Cookie 数据库: {DEFAULT_COOKIE_DB}",
        cookie_db_exists=False,
        cookie_db_path=str(DEFAULT_COOKIE_DB),
        hollysys_cookie_count=0,
        cookie_names=(),
        safe_storage_available=False,
        authenticated=False,
        http_status=0,
        final_url="",
    )


def _read_chrome_cookie_encryption_key(
    cookie_db: Path,
    rows: list[tuple[str, str, bytes]],
) -> bytes | None:
    if not any(encrypted_value.startswith((b"v10", b"v11", b"v20")) for _, _, encrypted_value in rows):
        return None

    if sys.platform == "darwin":
        safe_storage_password = _read_chrome_safe_storage_password()
        return hashlib.pbkdf2_hmac(
            "sha1",
            safe_storage_password,
            CHROME_COOKIE_SALT,
            CHROME_COOKIE_ITERATIONS,
            dklen=CHROME_COOKIE_KEY_LENGTH,
        )

    if sys.platform == "win32":
        if any(encrypted_value.startswith(b"v20") for _, _, encrypted_value in rows):
            raise RuntimeError("Chrome 已启用 App-Bound Encryption（v20），当前模式无法直接解密本机 Cookie。")
        return _read_windows_chrome_master_key(cookie_db)

    raise RuntimeError(f"当前平台暂不支持直接读取 Chrome Cookie: {sys.platform}")


def _read_chrome_safe_storage_password() -> bytes:
    result = subprocess.run(
        ["security", "find-generic-password", "-w", "-s", CHROME_SAFE_STORAGE_SERVICE],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip().encode("utf-8")


def _read_windows_chrome_master_key(cookie_db: Path) -> bytes:
    local_state_path = _resolve_local_state_path(cookie_db)
    if not local_state_path.exists():
        raise FileNotFoundError(f"未找到 Chrome Local State: {local_state_path}")

    payload = json.loads(local_state_path.read_text(encoding="utf-8"))
    os_crypt = payload.get("os_crypt", {})
    encrypted_key_b64 = str(os_crypt.get("encrypted_key", "") or "")
    if not encrypted_key_b64:
        raise RuntimeError(f"Chrome Local State 未包含 os_crypt.encrypted_key: {local_state_path}")

    encrypted_key = base64.b64decode(encrypted_key_b64)
    if encrypted_key.startswith(b"DPAPI"):
        encrypted_key = encrypted_key[5:]
    return _crypt_unprotect_data(encrypted_key)


def _resolve_local_state_path(cookie_db: Path) -> Path:
    for parent in cookie_db.parents:
        candidate = parent / "Local State"
        if candidate.exists():
            return candidate
    return _default_chrome_user_data_dir() / "Local State"


def _crypt_unprotect_data(encrypted_value: bytes) -> bytes:
    if sys.platform != "win32":
        raise RuntimeError("CryptUnprotectData 仅支持 Windows")

    class DataBlob(ctypes.Structure):
        _fields_ = [
            ("cbData", ctypes.c_uint),
            ("pbData", ctypes.POINTER(ctypes.c_char)),
        ]

    if not encrypted_value:
        return b""

    buffer = ctypes.create_string_buffer(encrypted_value, len(encrypted_value))
    input_blob = DataBlob(
        cbData=len(encrypted_value),
        pbData=ctypes.cast(buffer, ctypes.POINTER(ctypes.c_char)),
    )
    output_blob = DataBlob()
    crypt32 = ctypes.windll.crypt32
    kernel32 = ctypes.windll.kernel32

    if not crypt32.CryptUnprotectData(
        ctypes.byref(input_blob),
        None,
        None,
        None,
        None,
        0,
        ctypes.byref(output_blob),
    ):
        raise ctypes.WinError()

    try:
        return ctypes.string_at(output_blob.pbData, output_blob.cbData)
    finally:
        if output_blob.pbData:
            kernel32.LocalFree(output_blob.pbData)


def _decrypt_chrome_cookie(host_key: str, encrypted_value: bytes, key: bytes | None) -> str:
    if not encrypted_value:
        return ""
    if sys.platform == "win32":
        return _decrypt_windows_chrome_cookie(encrypted_value, key)
    return _decrypt_legacy_chrome_cookie(host_key, encrypted_value, key)


def _decrypt_windows_chrome_cookie(encrypted_value: bytes, key: bytes | None) -> str:
    if encrypted_value.startswith(b"v20"):
        raise RuntimeError("Chrome 已启用 App-Bound Encryption（v20），当前模式无法直接解密本机 Cookie。")
    if encrypted_value.startswith((b"v10", b"v11")):
        if not key:
            raise RuntimeError("未找到 Windows Chrome 主密钥")
        payload = encrypted_value[3:]
        if len(payload) < 12 + 16:
            raise ValueError("Chrome Cookie 密文长度异常")
        nonce = payload[:12]
        ciphertext = payload[12:-16]
        tag = payload[-16:]
        decrypted = AES.new(key, AES.MODE_GCM, nonce=nonce).decrypt_and_verify(ciphertext, tag)
        return decrypted.decode("utf-8", "ignore")
    return _crypt_unprotect_data(encrypted_value).decode("utf-8", "ignore")


def _decrypt_legacy_chrome_cookie(host_key: str, encrypted_value: bytes, key: bytes | None) -> str:
    if not encrypted_value.startswith((b"v10", b"v11")):
        return encrypted_value.decode("utf-8", "ignore")
    if not key:
        raise RuntimeError("未找到 Chrome Cookie 解密密钥")
    payload = encrypted_value[3:]
    decrypted = AES.new(key, AES.MODE_CBC, CHROME_COOKIE_IV).decrypt(payload)
    decrypted = decrypted[: -decrypted[-1]]
    host_prefix = hashlib.sha256(host_key.encode("utf-8")).digest()
    if decrypted.startswith(host_prefix):
        decrypted = decrypted[len(host_prefix) :]
    return decrypted.decode("utf-8", "ignore")


def _build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="批量下载 Hollysys 项目关闭待办附件")
    parser.add_argument("--output-root", type=Path, default=DEFAULT_OUTPUT_ROOT)
    parser.add_argument("--summary-path", type=Path, default=DEFAULT_SUMMARY_PATH)
    parser.add_argument("--limit", type=int, default=None)
    return parser


def main() -> int:
    args = _build_arg_parser().parse_args()
    summary = run_batch(
        output_root=args.output_root,
        summary_path=args.summary_path,
        limit=args.limit,
    )
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
