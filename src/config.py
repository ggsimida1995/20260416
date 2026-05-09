from __future__ import annotations

import os
import sys
from decimal import Decimal
from pathlib import Path
from typing import Mapping


PROJECTS_DIR_NAME = "project"
RESERVED_DIRS = {"other", "success", "error", "result_logs", PROJECTS_DIR_NAME}
APP_NAME = "ProjectFileCompare"


def resolve_runtime_root(
    *,
    frozen: bool | None = None,
    platform: str | None = None,
    env: Mapping[str, str] | None = None,
) -> Path:
    is_frozen = bool(getattr(sys, "frozen", False)) if frozen is None else frozen
    active_platform = sys.platform if platform is None else platform
    active_env = os.environ if env is None else env

    if active_platform == "win32":
        local_app_data = active_env.get("LOCALAPPDATA")
        if local_app_data:
            return Path(local_app_data) / APP_NAME
        return Path.home() / "AppData" / "Local" / APP_NAME

    if active_platform == "darwin":
        return Path.home() / "Library" / "Application Support" / APP_NAME

    xdg_data_home = active_env.get("XDG_DATA_HOME")
    if xdg_data_home:
        return Path(xdg_data_home) / APP_NAME
    return Path.home() / ".local" / "share" / APP_NAME


APP_RUNTIME_ROOT = resolve_runtime_root()
FILE_ROOT = APP_RUNTIME_ROOT / "file"
CONFIG_ROOT = APP_RUNTIME_ROOT / "config"
DEBUG_ROOT = APP_RUNTIME_ROOT / "debug"
SETTINGS_PATH = CONFIG_ROOT / "settings.json"
PROCESSED_PROJECTS_PATH = CONFIG_ROOT / "processed_projects.json"
SUCCESS_WORKBOOK_PATH = FILE_ROOT / "success" / "2026年关闭满意度回访表0331.xlsx"
FIELD_COMPARE_WORKBOOK_PATH = FILE_ROOT / "other" / "字段对比工作簿.xlsx"
SUCCESS_SHEET_NAME = "登记表"
STAMP_REQUIRED_AMOUNT = Decimal("50")

REQUIRED_FILE_KEYWORDS = {
    "xlsx": "项目关闭移交登记表",
    "docx": "项目竣工总结报告",
    "pdf": ("PA竣工验收报告", "竣工验收报告", "验收报告"),
}

SUCCESS_FIELD_MAPPING = {
    "项目编号": "项目编码",
    "项目全称": "项目全称",
    "产品线": "产品线",
    "项目类型": "项目类型",
    "老项目编号": "老项目号",
    "软件版本": "软件版本",
    "合同额（万元）": "合同额（万元）",
    "核实方式": "核实方式",
    "核实人": "核实人",
    "项目部": "项目部",
    "项目经理": "项目经理",
    "移交人": "移交人",
    "接收日期": "接收日期",
    "核实日期": "核实日期",
    "完成日期": "完成关闭",
    "用户联系人": "联系人",
    "用户职务": "职务",
    "用户联系方式": "联系方式",
    "产品评分(硬件）": "产品评分(硬件）",
    "产品评分(软件）": "产品评分(软件）",
    "产品满意度": "产品满意度",
    "意见建议": "意见建议",
    "服务评分（技术水平）": "服务评分（技术水平）",
    "服务评分（服务态度）": "服务评分（服务态度）",
    "服务满意度": "服务满意度",
    "项目整体评分": "项目整体评分",
    "项目整体满意度": "项目整体满意度",
    "如果有机会，您是否愿意将和利时的产品和服务，推荐给其他人员或企业？": "如果有机会，您是否愿意将和利时的产品和服务，推荐给其他人员或企业？",
    "推荐分数": "推荐分数",
    "市场确认人": "市场回访人",
    "提交市场确认日期": "市场回访（起始时间）",
    "市场回访（要求反馈时间）": "市场回访（要求反馈时间）",
    "市场回访(实际反馈时间)": "市场回访(实际反馈时间)",
    "销售经理评价得分（总分）": "销售经理评价得分（总分）",
    "问题记录": "问题记录",
    "现场问题备注": "现场问题备注",
    "现场情况": "特批现场情况",
    "备忘关闭项目预留工费数据": "遗留工作工时预算",
    "其他备注": "其他备注",
    "项目负责人回复": "项目负责人回复",
    "验收报告": "验收报告",
    "CRM有无信息": "CRM有无信息",
    "CRM有无信息关联主项目号": "CRM有无信息关联主项目号",
    "验收日期": "验收日期",
}


def project_root(file_root: Path) -> Path:
    return file_root / PROJECTS_DIR_NAME
