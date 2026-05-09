from datetime import datetime
from pathlib import Path

from openpyxl import Workbook

from src.readers.xlsx_reader import read_close_sheet


def build_close_sheet_fixture(tmp_path: Path) -> Path:
    workbook = Workbook()
    sheet = workbook.active
    sheet.title = "Sheet2"
    sheet["A1"] = "项目关闭登记表"
    sheet["A2"] = "字段名称"
    sheet["B2"] = "内容"
    sheet["A5"] = "项目编号"
    sheet["B5"] = "BHE-25030367/01"
    sheet["A6"] = "项目全称"
    sheet["B6"] = "示例项目"
    sheet["A10"] = "合同额（万元）"
    sheet["B10"] = 16.3
    sheet["A12"] = "核实\n方式"
    sheet["B12"] = "电话"
    sheet["A20"] = "用户联系人"
    sheet["B20"] = "黄汉民"
    sheet["A49"] = "验收日期"
    sheet["B49"] = datetime(2026, 4, 10)
    path = tmp_path / "项目关闭移交登记表.xlsx"
    workbook.save(path)
    return path


def test_read_close_sheet_extracts_field_value_pairs(tmp_path: Path):
    workbook_path = build_close_sheet_fixture(tmp_path)

    result = read_close_sheet(workbook_path)

    assert result.raw_fields["项目编号"] == "BHE-25030367/01"
    assert result.raw_fields["用户联系人"] == "黄汉民"
    assert result.raw_fields["合同额（万元）"] == 16.3
    assert result.raw_fields["核实方式"] == "电话"


def test_read_close_sheet_supports_xls(monkeypatch, tmp_path: Path):
    class FakeSheet:
        nrows = 6
        ncols = 2

        def cell_value(self, row, column):
            values = {
                (1, 0): "字段名称",
                (1, 1): "内容",
                (3, 0): "项目编号",
                (3, 1): "BHE-25030367/01",
                (4, 0): "用户联系人",
                (4, 1): "黄汉民",
                (5, 0): "用户联系方式",
                (5, 1): 15239000050.0,
            }
            return values.get((row, column), "")

    class FakeWorkbook:
        def sheets(self):
            return [FakeSheet()]

    class FakeXlrd:
        @staticmethod
        def open_workbook(path):
            return FakeWorkbook()

    monkeypatch.setattr("src.readers.xlsx_reader.import_module", lambda name: FakeXlrd)

    result = read_close_sheet(tmp_path / "项目关闭移交登记表.xls")

    assert result.raw_fields["项目编号"] == "BHE-25030367/01"
    assert result.raw_fields["用户联系人"] == "黄汉民"
    assert result.raw_fields["用户联系方式"] == "15239000050"
