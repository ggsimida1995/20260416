from __future__ import annotations

from typing import Callable

from src.config import STAMP_REQUIRED_AMOUNT
from src.models import CompareFailure, CompareResult, DocxData, PdfData
from src.normalizers import (
    names_match_by_loose_pinyin,
    normalize_amount,
    normalize_compact_text,
    normalize_date,
    normalize_phone,
    normalize_project_code,
    normalize_text,
)


def compare_project_data(
    xlsx_fields: dict[str, object],
    docx_data: DocxData,
    pdf_data: PdfData,
    log_callback: Callable[[str], None] | None = None,
) -> CompareResult:
    failures: list[CompareFailure] = []

    _compare_text_field(
        failures,
        field_name="项目编号",
        xlsx_value=xlsx_fields.get("项目编号"),
        other_value=docx_data.project_code,
        normalizer=normalize_project_code,
        sources=("xlsx", "docx"),
        log_callback=log_callback,
    )
    _compare_text_field(
        failures,
        field_name="项目全称",
        xlsx_value=xlsx_fields.get("项目全称"),
        other_value=docx_data.project_name,
        normalizer=normalize_compact_text,
        sources=("xlsx", "docx"),
        log_callback=log_callback,
    )
    _compare_text_candidates_field(
        failures,
        field_name="用户联系人",
        xlsx_value=xlsx_fields.get("用户联系人"),
        other_values=_docx_contact_names(docx_data),
        normalizer=normalize_text,
        sources=("xlsx", "docx"),
        log_callback=log_callback,
    )
    _compare_text_field(
        failures,
        field_name="用户联系人",
        xlsx_value=xlsx_fields.get("用户联系人"),
        other_value=pdf_data.signer_name,
        normalizer=normalize_text,
        matcher=names_match_by_loose_pinyin,
        sources=("xlsx", "pdf"),
        log_callback=log_callback,
    )
    _compare_text_candidates_field(
        failures,
        field_name="用户联系方式",
        xlsx_value=xlsx_fields.get("用户联系方式"),
        other_values=_docx_contact_phones(docx_data),
        normalizer=normalize_phone,
        sources=("xlsx", "docx"),
        log_callback=log_callback,
    )
    _compare_text_field(
        failures,
        field_name="用户联系方式",
        xlsx_value=xlsx_fields.get("用户联系方式"),
        other_value=pdf_data.signer_phone,
        normalizer=normalize_phone,
        sources=("xlsx", "pdf"),
        log_callback=log_callback,
    )

    _compare_acceptance_window(failures, docx_data, pdf_data, log_callback=log_callback)
    _compare_stamp_rule(failures, xlsx_fields, pdf_data, log_callback=log_callback)

    return CompareResult(passed=not failures, failures=failures)


def _compare_text_field(
    failures: list[CompareFailure],
    field_name: str,
    xlsx_value: object,
    other_value: object,
    normalizer,
    sources: tuple[str, str],
    matcher=None,
    log_callback: Callable[[str], None] | None = None,
) -> None:
    left = normalizer(xlsx_value)
    if not left:
        if log_callback is not None:
            log_callback(f"[字段比对] {field_name}({sources[0]}/{sources[1]}) | 跳过")
        return

    right = normalizer(other_value)
    matched = left == right or (matcher is not None and matcher(xlsx_value, other_value))
    if log_callback is not None:
        status = "一致" if matched else "不一致"
        log_callback(f"[字段比对] {field_name}({sources[0]}/{sources[1]}) | {status}")
    if matched:
        return

    failures.append(
        CompareFailure(
            field_name=field_name,
            message=f"{sources[0]} 与 {sources[1]} 不一致",
            values={sources[0]: xlsx_value, sources[1]: other_value},
        )
    )


def _compare_text_candidates_field(
    failures: list[CompareFailure],
    field_name: str,
    xlsx_value: object,
    other_values: list[object],
    normalizer,
    sources: tuple[str, str],
    log_callback: Callable[[str], None] | None = None,
) -> None:
    left = normalizer(xlsx_value)
    if not left:
        if log_callback is not None:
            log_callback(f"[字段比对] {field_name}({sources[0]}/{sources[1]}) | 跳过")
        return

    normalized_candidates = [normalized for item in other_values if (normalized := normalizer(item))]
    display_candidates = _format_values_for_log(other_values)
    matched = left in normalized_candidates
    if log_callback is not None:
        status = "一致" if matched else "不一致"
        log_callback(f"[字段比对] {field_name}({sources[0]}/{sources[1]}) | {status}")

    if matched:
        return

    failures.append(
        CompareFailure(
            field_name=field_name,
            message=f"{sources[0]} 与 {sources[1]} 不一致",
            values={sources[0]: xlsx_value, sources[1]: display_candidates},
        )
    )


def _compare_acceptance_window(
    failures: list[CompareFailure],
    docx_data: DocxData,
    pdf_data: PdfData,
    log_callback: Callable[[str], None] | None = None,
) -> None:
    if docx_data.has_invalid_acceptance_range:
        if log_callback is not None:
            log_callback("[字段比对] 验收时间区间 | 不一致")
        failures.append(
            CompareFailure(
                field_name="竣工验收时间区间",
                message="开始时间晚于完成时间",
                values={
                    "开始时间": docx_data.acceptance_start,
                    "完成时间": docx_data.acceptance_end,
                },
            )
        )
        return

    start = normalize_date(docx_data.acceptance_start)
    end = normalize_date(docx_data.acceptance_end)
    sign_date = normalize_date(pdf_data.sign_date)
    if start is None or end is None or sign_date is None:
        if log_callback is not None:
            log_callback("[字段比对] 验收时间区间 | 无法校验")
        failures.append(
            CompareFailure(
                field_name="验收时间",
                message="无法完成区间校验",
                values={"开始时间": start, "完成时间": end, "签字时间": sign_date},
            )
        )
        return

    if not (start <= sign_date <= end):
        if log_callback is not None:
            log_callback("[字段比对] 验收时间区间 | 不一致")
        failures.append(
            CompareFailure(
                field_name="验收时间",
                message="pdf 签字时间不在 docx 验收区间内",
                values={"开始时间": start, "完成时间": end, "签字时间": sign_date},
            )
        )
        return

    if log_callback is not None:
        log_callback("[字段比对] 验收时间区间 | 一致")


def _compare_stamp_rule(
    failures: list[CompareFailure],
    xlsx_fields: dict[str, object],
    pdf_data: PdfData,
    log_callback: Callable[[str], None] | None = None,
) -> None:
    amount = normalize_amount(xlsx_fields.get("合同额（万元）"))
    if amount is None or amount <= STAMP_REQUIRED_AMOUNT:
        if log_callback is not None:
            log_callback("[字段比对] 盖章检查 | 跳过")
        return

    if pdf_data.has_red_stamp:
        if log_callback is not None:
            log_callback("[字段比对] 盖章检查 | 一致")
        return

    if log_callback is not None:
        log_callback("[字段比对] 盖章检查 | 不一致")
    failures.append(
        CompareFailure(
            field_name="盖章检查",
            message="合同额超过 50 万但 pdf 未识别到红章",
            values={"合同额（万元）": amount, "pdf_has_red_stamp": pdf_data.has_red_stamp},
        )
    )


def _docx_contact_names(docx_data: DocxData) -> list[str]:
    if docx_data.contact_names:
        return docx_data.contact_names
    if docx_data.contact_name:
        return [docx_data.contact_name]
    return []


def _docx_contact_phones(docx_data: DocxData) -> list[str]:
    if docx_data.contact_phones:
        return docx_data.contact_phones
    if docx_data.contact_phone:
        return [docx_data.contact_phone]
    return []


def _format_values_for_log(values: list[object]) -> str:
    rendered = [str(value) for value in values if value not in (None, "")]
    if not rendered:
        return "<空>"
    return " | ".join(rendered)
