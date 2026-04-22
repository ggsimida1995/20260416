from __future__ import annotations

from openpyxl import load_workbook

from src.models import ProjectExtraction


def _clean_field_name(value: object) -> str:
    if value is None:
        return ""
    return str(value).replace("\n", "").strip()


def read_close_sheet(path) -> ProjectExtraction:
    workbook = load_workbook(path, data_only=True)

    for sheet in workbook.worksheets:
        if _sheet_has_field_header(sheet):
            raw_fields = _read_field_rows(sheet)
            return ProjectExtraction(
                raw_fields=raw_fields,
                normalized_fields=dict(raw_fields),
            )

    return ProjectExtraction()


def _sheet_has_field_header(sheet) -> bool:
    for row in range(1, min(sheet.max_row, 10) + 1):
        left = _clean_field_name(sheet.cell(row, 1).value)
        right = _clean_field_name(sheet.cell(row, 2).value)
        if left == "字段名称" and right == "内容":
            return True
    return False


def _read_field_rows(sheet) -> dict[str, object]:
    raw_fields: dict[str, object] = {}
    for row in range(1, sheet.max_row + 1):
        field_name = _clean_field_name(sheet.cell(row, 1).value)
        if not field_name or field_name in {"项目关闭登记表", "字段名称"}:
            continue
        if field_name.startswith("注："):
            continue
        raw_fields[field_name] = sheet.cell(row, 2).value
    return raw_fields
