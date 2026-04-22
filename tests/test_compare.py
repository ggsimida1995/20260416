from datetime import date

from src.compare import compare_project_data
from src.models import CompareFailure, DocxData, PdfData
from src.writers.error_writer import write_error_report


def test_compare_passes_when_pdf_sign_date_is_inside_docx_range():
    result = compare_project_data(
        xlsx_fields={
            "项目编号": "BHE-25030367/01",
            "项目全称": "示例项目",
            "用户联系人": "黄汉民",
            "用户联系方式": "14714691425",
            "合同额（万元）": 16.3,
        },
        docx_data=DocxData(
            project_code="BHE-25030367/01",
            project_name="示例项目",
            contact_name="黄汉民",
            contact_phone="14714691425",
            acceptance_start=date(2026, 4, 9),
            acceptance_end=date(2026, 4, 10),
        ),
        pdf_data=PdfData(
            signer_name="黄汉民",
            signer_phone="14714691425",
            sign_date=date(2026, 4, 10),
            has_red_stamp=False,
        ),
    )

    assert result.passed is True


def test_compare_ignores_spacing_differences_in_project_name():
    result = compare_project_data(
        xlsx_fields={
            "项目全称": "乳源东阳光氟有限公司75吨锅炉配套烟气装置改造DCS工程项目合同",
        },
        docx_data=DocxData(
            project_name="乳源东阳光氟有限公司 75 吨锅炉配套烟气装置改造 DCS 工程项目合同",
            acceptance_start=date(2026, 4, 9),
            acceptance_end=date(2026, 4, 10),
        ),
        pdf_data=PdfData(sign_date=date(2026, 4, 10)),
    )

    assert result.passed is True


def test_compare_passes_when_any_docx_contact_group_matches():
    result = compare_project_data(
        xlsx_fields={
            "用户联系人": "黄汉民",
            "用户联系方式": "14714691425",
        },
        docx_data=DocxData(
            contact_name="张三",
            contact_phone="13800000000",
            contact_names=["张三", "黄汉民"],
            contact_phones=["13800000000", "14714691425"],
            acceptance_start=date(2026, 4, 9),
            acceptance_end=date(2026, 4, 10),
        ),
        pdf_data=PdfData(
            signer_name="黄汉民",
            signer_phone="14714691425",
            sign_date=date(2026, 4, 10),
        ),
    )

    assert result.passed is True


def test_compare_passes_when_pdf_name_matches_by_loose_pinyin():
    result = compare_project_data(
        xlsx_fields={
            "用户联系人": "陈汉民",
            "用户联系方式": "14714691425",
            "合同额（万元）": 16.3,
        },
        docx_data=DocxData(
            contact_name="陈汉民",
            contact_phone="14714691425",
            acceptance_start=date(2026, 4, 9),
            acceptance_end=date(2026, 4, 10),
        ),
        pdf_data=PdfData(
            signer_name="陈汉明",
            signer_phone="14714691425",
            sign_date=date(2026, 4, 10),
        ),
    )

    assert result.passed is True


def test_compare_still_fails_when_pdf_name_pronunciation_is_different():
    result = compare_project_data(
        xlsx_fields={
            "用户联系人": "陈汉民",
            "用户联系方式": "14714691425",
            "合同额（万元）": 16.3,
        },
        docx_data=DocxData(
            contact_name="陈汉民",
            contact_phone="14714691425",
            acceptance_start=date(2026, 4, 9),
            acceptance_end=date(2026, 4, 10),
        ),
        pdf_data=PdfData(
            signer_name="陈汉强",
            signer_phone="14714691425",
            sign_date=date(2026, 4, 10),
        ),
    )

    assert result.passed is False
    assert any(item.field_name == "用户联系人" and item.values.get("pdf") == "陈汉强" for item in result.failures)


def test_compare_still_requires_exact_match_between_xlsx_and_docx_names():
    result = compare_project_data(
        xlsx_fields={
            "用户联系人": "陈汉民",
            "用户联系方式": "14714691425",
            "合同额（万元）": 16.3,
        },
        docx_data=DocxData(
            contact_name="陈汉明",
            contact_phone="14714691425",
            acceptance_start=date(2026, 4, 9),
            acceptance_end=date(2026, 4, 10),
        ),
        pdf_data=PdfData(
            signer_name="陈汉明",
            signer_phone="14714691425",
            sign_date=date(2026, 4, 10),
        ),
    )

    assert result.passed is False
    assert any(item.field_name == "用户联系人" and item.values.get("docx") == "陈汉明" for item in result.failures)


def test_compare_fails_when_docx_acceptance_range_is_reversed():
    result = compare_project_data(
        xlsx_fields={"合同额（万元）": 16.3},
        docx_data=DocxData(
            acceptance_start=date(2026, 4, 10),
            acceptance_end=date(2026, 4, 9),
            has_invalid_acceptance_range=True,
        ),
        pdf_data=PdfData(sign_date=date(2026, 4, 10), has_red_stamp=False),
    )

    assert result.passed is False
    assert any(item.field_name == "竣工验收时间区间" for item in result.failures)


def test_compare_fails_when_amount_over_threshold_without_stamp():
    result = compare_project_data(
        xlsx_fields={"合同额（万元）": 88},
        docx_data=DocxData(),
        pdf_data=PdfData(has_red_stamp=False),
    )

    assert result.passed is False
    assert result.failures[-1].field_name == "盖章检查"


def test_compare_logs_only_result_without_field_values():
    messages: list[str] = []

    compare_project_data(
        xlsx_fields={
            "用户联系人": "黄汉民",
            "用户联系方式": "14714691425",
            "合同额（万元）": 16.3,
        },
        docx_data=DocxData(
            contact_names=["黄汉民"],
            contact_phones=["14714691425"],
            acceptance_start=date(2026, 4, 9),
            acceptance_end=date(2026, 4, 10),
        ),
        pdf_data=PdfData(
            signer_name="黄汉民",
            signer_phone="14714691425",
            sign_date=date(2026, 4, 10),
            has_red_stamp=False,
        ),
        log_callback=messages.append,
    )

    joined = "\n".join(messages)

    assert "[字段比对] 用户联系人(xlsx/docx) | 一致" in joined
    assert "[字段比对] 用户联系方式(xlsx/pdf) | 一致" in joined
    assert "[字段比对] 验收时间区间 | 一致" in joined
    assert "[字段比对] 盖章检查 | 跳过" in joined
    assert "xlsx=" not in joined
    assert "pdf=" not in joined


def test_write_error_report_lists_all_failures(tmp_path):
    output_path = write_error_report(
        tmp_path,
        "BHE-25030367-01",
        [
            CompareFailure(
                field_name="用户联系人",
                message="姓名不一致",
                values={"xlsx": "黄汉民", "pdf": "王某某"},
            )
        ],
    )

    assert output_path.exists()
    content = output_path.read_text(encoding="utf-8")
    assert "BHE-25030367-01" in content
    assert "用户联系人" in content
