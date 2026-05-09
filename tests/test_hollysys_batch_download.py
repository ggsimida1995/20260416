from __future__ import annotations

import base64
import json
from pathlib import Path

import src.hollysys_batch_download as hollysys_module
from lxml import html as lxml_html

from src.hollysys_batch_download import (
    AggregationCategory,
    Attachment,
    DetailRecord,
    TodoItem,
    extract_detail_field,
    extract_section_attachments,
    inspect_local_hollysys_session,
    save_record,
    select_target_attachments,
)


def test_extract_detail_field_reads_value_cell_text():
    doc = lxml_html.fromstring(
        """
        <table>
          <tr>
            <td><label>项目编号</label></td>
            <td>
              <div><xformflag flagtype="xform_text">BHE-25030367/01</xformflag></div>
            </td>
          </tr>
          <tr>
            <td><label>项目名称</label></td>
            <td>
              <div><xformflag flagtype="xform_text">示例项目名称</xformflag></div>
            </td>
          </tr>
        </table>
        """
    )

    assert extract_detail_field(doc, "项目编号") == "BHE-25030367/01"
    assert extract_detail_field(doc, "项目名称") == "示例项目名称"


def test_extract_section_attachments_reads_add_doc_calls_from_matching_section():
    doc = lxml_html.fromstring(
        """
        <table>
          <tr>
            <td><label>关闭依据附件</label></td>
            <td>
              <xformflag flagid="fd_attach" flagtype="xform_relation_attachment" _xform_type="attachment"></xformflag>
              <script>
                var attachmentObject_fd_attach = new Swf_AttachmentObject("fd_attach","","","true","byte","view");
                attachmentObject_fd_attach.addDoc("项目关闭移交登记表.xlsx","19dab24e3cad13592028fdb45709025d",true,"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet","1","k1","0");
                attachmentObject_fd_attach.addDoc("项目竣工总结报告.docx","19dab24e4137a64d1a251e64233bd779",true,"application/vnd.openxmlformats-officedocument.wordprocessingml.document","2","k2","0");
                attachmentObject_fd_attach.addDoc("验收报告.pdf","19dae72e1e319ef83201b4a4cccb4a47",true,"application/pdf","3","k3","0");
              </script>
            </td>
          </tr>
        </table>
        """
    )

    attachments = extract_section_attachments(doc, "关闭依据附件")

    assert [attachment.name for attachment in attachments] == [
        "项目关闭移交登记表.xlsx",
        "项目竣工总结报告.docx",
        "验收报告.pdf",
    ]


def test_select_target_attachments_skips_email_and_keeps_three_target_files():
    attachments = [
        Attachment("回复_审批邮件.eml", "fd_mail", "message/rfc822", "10", "mail"),
        Attachment("A项目关闭移交登记表.xlsx", "fd_xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "11", "xlsx"),
        Attachment("A项目竣工总结报告.docx", "fd_docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", "12", "docx"),
        Attachment("A项目验收报告.pdf", "fd_pdf", "application/pdf", "13", "pdf"),
        Attachment("其他说明.txt", "fd_txt", "text/plain", "14", "txt"),
    ]

    selected = select_target_attachments(attachments)

    assert [attachment.fd_id for attachment in selected] == ["fd_xlsx", "fd_docx", "fd_pdf"]


def test_write_bytes_atomically_replaces_destination(tmp_path: Path):
    destination = tmp_path / "downloaded.xlsx"
    destination.write_bytes(b"old")

    hollysys_module._write_bytes_atomically(destination, b"new")

    assert destination.read_bytes() == b"new"
    assert list(tmp_path.glob("*.part")) == []


def test_write_bytes_atomically_removes_temp_file_on_failure(monkeypatch, tmp_path: Path):
    destination = tmp_path / "downloaded.xlsx"
    destination.write_bytes(b"old")

    def fail_replace(self, target):
        raise OSError("replace failed")

    monkeypatch.setattr(Path, "replace", fail_replace)

    try:
        hollysys_module._write_bytes_atomically(destination, b"new")
    except OSError:
        pass

    assert destination.read_bytes() == b"old"
    assert list(tmp_path.glob("*.part")) == []


def test_save_record_writes_project_under_project_directory(monkeypatch, tmp_path: Path):
    category = AggregationCategory("cat-1", "项目关闭工作流")
    item = TodoItem(
        category=category,
        todo_fd_id="todo-1",
        subject="待办",
        detail_path="/sys/detail.do?fdId=detail-1",
        project_code_hint="BHE-25030367/01",
    )
    record = DetailRecord(
        item=item,
        project_code="BHE-25030367/01",
        project_name="示例项目",
        attachments=(
            Attachment("项目关闭移交登记表.xlsx", "fd-xlsx", "application/vnd.ms-excel", "10", "xlsx"),
        ),
    )

    monkeypatch.setattr(
        hollysys_module,
        "download_attachment",
        lambda session, attachment, destination, referer: destination.write_bytes(b"demo"),
    )

    saved = save_record(object(), record, tmp_path / "file")

    project_dir = tmp_path / "file" / "project" / "BHE-25030367-01"
    assert saved["project_dir"] == str(project_dir)
    assert (project_dir / "项目关闭移交登记表.xlsx").exists()
    assert (project_dir / "BHE-25030367-01.txt").exists()
    assert not (tmp_path / "file" / "BHE-25030367-01").exists()


def test_save_record_overwrites_existing_downloaded_files(monkeypatch, tmp_path: Path):
    category = AggregationCategory("cat-1", "项目关闭工作流")
    item = TodoItem(
        category=category,
        todo_fd_id="todo-1",
        subject="待办",
        detail_path="/sys/detail.do?fdId=detail-1",
        project_code_hint="BHE-25030367/01",
    )
    record = DetailRecord(
        item=item,
        project_code="BHE-25030367/01",
        project_name="示例项目",
        attachments=(
            Attachment("项目关闭移交登记表.xlsx", "fd-xlsx", "application/vnd.ms-excel", "10", "xlsx"),
        ),
    )
    target = tmp_path / "file" / "project" / "BHE-25030367-01" / "项目关闭移交登记表.xlsx"
    target.parent.mkdir(parents=True)
    target.write_bytes(b"old")

    monkeypatch.setattr(
        hollysys_module,
        "download_attachment",
        lambda session, attachment, destination, referer: hollysys_module._write_bytes_atomically(destination, b"new"),
    )

    save_record(object(), record, tmp_path / "file")

    assert target.read_bytes() == b"new"


def test_inspect_local_hollysys_session_reports_missing_cookie_db(tmp_path: Path):
    result = inspect_local_hollysys_session(tmp_path / "Cookies")

    assert result.status == "missing"
    assert result.cookie_db_exists is False
    assert result.hollysys_cookie_count == 0


def test_inspect_local_hollysys_session_reports_missing_hollysys_cookies(monkeypatch, tmp_path: Path):
    cookie_db = tmp_path / "Cookies"
    cookie_db.write_bytes(b"sqlite")
    monkeypatch.setattr(hollysys_module, "_read_hollysys_cookie_rows", lambda path: [])

    result = inspect_local_hollysys_session(cookie_db)

    assert result.status == "missing"
    assert result.cookie_db_exists is True
    assert result.hollysys_cookie_count == 0


def test_inspect_local_hollysys_session_reports_cookie_build_failure(monkeypatch, tmp_path: Path):
    cookie_db = tmp_path / "Cookies"
    cookie_db.write_bytes(b"sqlite")
    monkeypatch.setattr(
        hollysys_module,
        "_read_hollysys_cookie_rows",
        lambda path: [(".hollysys.net", "JSESSIONID", b"v10-demo-cookie")],
    )
    monkeypatch.setattr(
        hollysys_module,
        "build_authenticated_session",
        lambda path: (_ for _ in ()).throw(RuntimeError("bad keychain")),
    )

    result = inspect_local_hollysys_session(cookie_db)

    assert result.status == "error"
    assert "Cookie 解密失败" in result.detail
    assert result.hollysys_cookie_count == 1


def test_inspect_local_hollysys_session_reports_ready(monkeypatch, tmp_path: Path):
    cookie_db = tmp_path / "Cookies"
    cookie_db.write_bytes(b"sqlite")
    monkeypatch.setattr(
        hollysys_module,
        "_read_hollysys_cookie_rows",
        lambda path: [
            (".hollysys.net", "JSESSIONID", b"v10-demo-cookie"),
            (".hollysys.net", "sid", b"v10-demo-cookie-2"),
        ],
    )

    class FakeResponse:
        status_code = 200
        url = "https://www.hollysys.net/sys/aggregation/"
        text = "<html><title>待办事宜</title><body>待办事宜</body></html>"

    class FakeSession:
        def get(self, url: str, timeout: float):
            assert url == "https://www.hollysys.net/sys/aggregation/"
            assert timeout == 10.0
            return FakeResponse()

    monkeypatch.setattr(hollysys_module, "build_authenticated_session", lambda path: FakeSession())

    result = inspect_local_hollysys_session(cookie_db)

    assert result.status == "ready"
    assert result.authenticated is True
    assert result.http_status == 200
    assert result.final_url == "https://www.hollysys.net/sys/aggregation/"
    assert result.cookie_names == ("JSESSIONID", "sid")


def test_candidate_cookie_dbs_include_windows_default_and_profiles(monkeypatch, tmp_path: Path):
    user_data_dir = tmp_path / "LocalAppData" / "Google" / "Chrome" / "User Data"
    (user_data_dir / "Default").mkdir(parents=True)
    (user_data_dir / "Profile 2").mkdir(parents=True)
    monkeypatch.setattr(hollysys_module.sys, "platform", "win32")
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "LocalAppData"))

    candidates = hollysys_module._candidate_cookie_dbs()

    assert candidates[:4] == [
        user_data_dir / "Default" / "Network" / "Cookies",
        user_data_dir / "Default" / "Cookies",
        user_data_dir / "Profile 2" / "Network" / "Cookies",
        user_data_dir / "Profile 2" / "Cookies",
    ]


def test_read_windows_chrome_master_key_reads_local_state(monkeypatch, tmp_path: Path):
    user_data_dir = tmp_path / "User Data"
    cookie_db = user_data_dir / "Default" / "Network" / "Cookies"
    cookie_db.parent.mkdir(parents=True, exist_ok=True)
    cookie_db.write_bytes(b"sqlite")
    local_state_path = user_data_dir / "Local State"
    local_state_path.write_text(
        json.dumps(
            {
                "os_crypt": {
                    "encrypted_key": base64.b64encode(b"DPAPIdemo-master-key").decode("ascii"),
                }
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setattr(hollysys_module.sys, "platform", "win32")
    monkeypatch.setattr(hollysys_module, "_crypt_unprotect_data", lambda value: b"plain:" + value)

    key = hollysys_module._read_windows_chrome_master_key(cookie_db)

    assert key == b"plain:demo-master-key"


def test_inspect_local_hollysys_session_reports_windows_app_bound_cookie_error(monkeypatch, tmp_path: Path):
    cookie_db = tmp_path / "Cookies"
    cookie_db.write_bytes(b"sqlite")
    monkeypatch.setattr(hollysys_module.sys, "platform", "win32")
    monkeypatch.setattr(
        hollysys_module,
        "_read_hollysys_cookie_rows",
        lambda path: [(".hollysys.net", "JSESSIONID", b"v20-demo-cookie")],
    )

    result = inspect_local_hollysys_session(cookie_db)

    assert result.status == "error"
    assert "App-Bound Encryption" in result.detail
