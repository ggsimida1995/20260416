from __future__ import annotations

import os
import shutil
import inspect
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from src.compare import compare_project_data
from src.config import PROCESSED_PROJECTS_PATH, SUCCESS_WORKBOOK_PATH
from src.config_store import AISettings
from src.discovery import discover_project_files, discover_projects
from src.models import BatchWorkflowResult, CompareFailure, DocxData, PdfData, ProjectFiles, WebPhaseResult, WorkflowResult
from src.process_state import mark_project_processed, processed_project_codes
from src.readers.docx_reader import read_docx
from src.hollysys_batch_download import run_batch as run_hollysys_download_batch
from src.readers.pdf_reader import read_pdf
from src.readers.signature_ai import (
    is_ai_recognition_configured,
    is_ocr_recognition_configured,
    is_remote_recognition_configured,
)
from src.readers.xlsx_reader import read_close_sheet
from src.runtime_logging import RuntimeLogger, build_log_path
from src.writers.error_writer import write_error_report
from src.writers.success_writer import append_success_row, build_success_row

XLSX_LOG_FIELDS = ("项目编号", "项目全称", "用户联系人", "用户联系方式", "合同额（万元）")


@dataclass
class PreparedProjectResult:
    project_name: str
    project_code: str
    status: str
    log_lines: list[str] = field(default_factory=list)
    failures: list[CompareFailure] = field(default_factory=list)
    success_row: dict[str, object] = field(default_factory=dict)


def run_workflow(
    file_root: Path,
    username: str,
    password: str,
    project_dirs: list[Path] | None = None,
    log_callback: Callable[[str], None] | None = None,
    ai_settings: AISettings | None = None,
) -> WorkflowResult:
    return run_compare_workflow(
        file_root=file_root,
        username=username,
        password=password,
        project_dirs=project_dirs,
        log_callback=log_callback,
        ai_settings=ai_settings,
    )


def run_compare_workflow(
    file_root: Path,
    username: str,
    password: str,
    project_dirs: list[Path] | None = None,
    log_callback: Callable[[str], None] | None = None,
    ai_settings: AISettings | None = None,
) -> WorkflowResult:
    del username, password

    result = WorkflowResult()
    success_workbook_path = file_root / "success" / SUCCESS_WORKBOOK_PATH.name
    error_root = file_root / "error"
    result.success_workbook_path = success_workbook_path
    target_project_dirs = project_dirs if project_dirs is not None else discover_projects(file_root)

    with RuntimeLogger(build_log_path(file_root), callback=log_callback) as logger:
        result.log_path = logger.log_path
        logger.log(f"[运行] 资料目录: {file_root}")
        if logger.log_path is not None:
            logger.log(f"[运行] 日志文件: {logger.log_path}")
        _log_pdf_recognition_status(logger, ai_settings)
        logger.log(f"[运行] 发现项目数量: {len(target_project_dirs)}")

        if not target_project_dirs:
            logger.log("[运行] 未发现可处理项目目录")
        else:
            logger.log(f"[运行] 并行线程数: {_resolve_worker_count(len(target_project_dirs))}")

        prepared_results = _prepare_project_results(target_project_dirs, ai_settings=ai_settings)

        for prepared in prepared_results:
            logger.log("")
            _apply_prepared_result(
                logger=logger,
                prepared=prepared,
                result=result,
                success_workbook_path=success_workbook_path,
                error_root=error_root,
            )

        logger.section("[汇总]")
        logger.log(
            f"[汇总] 追加成功={result.appended_count} | 重复跳过={result.duplicate_count} | 失败={result.failed_count}"
        )

    return result


def run_batch_workflow(
    file_root: Path,
    username: str,
    password: str,
    log_callback: Callable[[str], None] | None = None,
    web_phase_runner: Callable[..., WebPhaseResult] | None = None,
    compare_runner: Callable[..., WorkflowResult] | None = None,
    processed_projects_path: Path = PROCESSED_PROJECTS_PATH,
    ai_settings: AISettings | None = None,
) -> BatchWorkflowResult:
    result = BatchWorkflowResult()
    processed_codes = processed_project_codes(processed_projects_path)
    existing_project_dirs = discover_projects(file_root)
    pending_existing_project_dirs = [
        project_dir for project_dir in existing_project_dirs if project_dir.name not in processed_codes
    ]

    if log_callback is not None:
        log_callback(f"[已处理记录] 已处理项目数: {len(processed_codes)}")
        log_callback("[网页阶段] 开始")
        log_callback(f"[网页阶段] 项目数: {len(pending_existing_project_dirs)}")

    result.skipped_processed_count = len(existing_project_dirs) - len(pending_existing_project_dirs)

    web_runner = web_phase_runner or _run_web_phase
    compare_phase_runner = compare_runner or run_compare_workflow

    web_phase_result = web_runner(
        **_call_with_supported_kwargs(
            web_runner,
            file_root=file_root,
            username=username,
            password=password,
            project_dirs=pending_existing_project_dirs,
            log_callback=log_callback,
            processed_project_codes=processed_codes,
        )
    )
    result.web_processed_count = len(web_phase_result.processed_projects)
    if log_callback is not None:
        log_callback(f"[网页阶段] 完成: {result.web_processed_count}")
        log_callback("[本地比对阶段] 开始")

    all_project_dirs = discover_projects(file_root)
    pending_project_dirs = [project_dir for project_dir in all_project_dirs if project_dir.name not in processed_codes]
    compare_result = compare_phase_runner(
        **_call_with_supported_kwargs(
            compare_phase_runner,
            file_root=file_root,
            username=username,
            password=password,
            project_dirs=pending_project_dirs,
            log_callback=log_callback,
            ai_settings=ai_settings,
        ),
    )
    result.compare_appended_count = compare_result.appended_count
    result.compare_duplicate_count = compare_result.duplicate_count
    result.compare_failed_count = compare_result.failed_count
    result.log_path = compare_result.log_path
    result.compare_success_project_codes = list(compare_result.success_project_codes)
    result.compare_error_project_codes = list(compare_result.error_project_codes)
    result.compare_success_workbook_path = compare_result.success_workbook_path
    result.compare_error_report_paths = list(compare_result.error_report_paths)
    if log_callback is not None:
        log_callback(
            f"[本地比对阶段] 完成: 追加成功={result.compare_appended_count} | "
            f"重复跳过={result.compare_duplicate_count} | 失败={result.compare_failed_count}"
        )
        log_callback("[清理阶段] 开始")

    successful_dirs = {
        project_dir.name: project_dir
        for project_dir in pending_project_dirs
        if project_dir.name in compare_result.success_project_names
    }
    for project_code, project_dir in successful_dirs.items():
        try:
            if log_callback is not None:
                log_callback(f"[清理阶段] 删除项目目录: {project_dir}")
            _delete_project_dir(project_dir)
        except OSError:
            if log_callback is not None:
                log_callback(f"[清理阶段] 删除失败: {project_dir}")
            continue
        mark_project_processed(processed_projects_path, project_code=project_code)
        result.cleaned_count += 1
        if log_callback is not None:
            log_callback(f"[清理阶段] 已标记处理完成: {project_code}")

    if log_callback is not None:
        log_callback(f"[清理阶段] 完成: {result.cleaned_count}")

    return result


def run_download_workflow(
    file_root: Path,
    username: str,
    password: str,
    log_callback: Callable[[str], None] | None = None,
    processed_projects_path: Path = PROCESSED_PROJECTS_PATH,
) -> WebPhaseResult:
    del username, password

    processed_codes = processed_project_codes(processed_projects_path)
    return _run_web_phase(
        file_root=file_root,
        username="",
        password="",
        project_dirs=[],
        log_callback=log_callback,
        processed_project_codes=processed_codes,
    )


def _prepare_project_results(project_dirs: list[Path], ai_settings: AISettings | None = None) -> list[PreparedProjectResult]:
    if not project_dirs:
        return []

    worker_count = _resolve_worker_count(len(project_dirs))
    with ThreadPoolExecutor(max_workers=worker_count) as executor:
        futures = [
            executor.submit(
                _prepare_project_result,
                **_call_with_supported_kwargs(_prepare_project_result, project_dir=project_dir, ai_settings=ai_settings),
            )
            for project_dir in project_dirs
        ]
        return [future.result() for future in futures]


def _prepare_project_result(project_dir: Path, ai_settings: AISettings | None = None) -> PreparedProjectResult:
    project_files = discover_project_files(project_dir)
    project_code = _project_code_for_logs(project_files, None)
    log_lines = [f"[项目] {project_files.project_name}"]

    if project_files.missing_files:
        log_lines.append(f"[文件检查] 缺少必需文件: {', '.join(project_files.missing_files)}")
        return PreparedProjectResult(
            project_name=project_files.project_name,
            project_code=project_code,
            status="missing_files",
            log_lines=log_lines,
            failures=[
                CompareFailure(
                    field_name="文件识别",
                    message="缺少必需文件",
                    values={"missing_files": ",".join(project_files.missing_files)},
                )
            ],
        )

    xlsx_data = read_close_sheet(project_files.xlsx_path)
    project_code = _project_code_for_logs(project_files, xlsx_data.raw_fields.get("项目编号"))
    _log_key_values(log_lines.append, "[xlsx字段]", _xlsx_log_fields(xlsx_data.raw_fields))

    docx_data = read_docx(project_files.docx_path)
    _log_key_values(log_lines.append, "[docx字段]", _docx_log_fields(docx_data))

    pdf_data = read_pdf(project_files.pdf_path, ai_settings=ai_settings)
    _log_key_values(log_lines.append, "[pdf字段]", _pdf_log_fields(pdf_data))

    compare_result = compare_project_data(
        xlsx_data.raw_fields,
        docx_data,
        pdf_data,
        log_callback=log_lines.append,
    )

    if not compare_result.passed:
        log_lines.append(f"[比对结果] 失败，共 {len(compare_result.failures)} 项")
        for failure in compare_result.failures:
            log_lines.append(f"[比对失败] {failure.field_name} | {failure.message} | {_format_failure_values(failure.values)}")
        return PreparedProjectResult(
            project_name=project_files.project_name,
            project_code=project_code,
            status="compare_failed",
            log_lines=log_lines,
            failures=compare_result.failures,
        )

    log_lines.append("[比对结果] 通过")
    return PreparedProjectResult(
        project_name=project_files.project_name,
        project_code=project_code,
        status="compare_passed",
        log_lines=log_lines,
        success_row=build_success_row(xlsx_data.raw_fields),
    )


def _apply_prepared_result(
    *,
    logger,
    prepared: PreparedProjectResult,
    result: WorkflowResult,
    success_workbook_path: Path,
    error_root: Path,
) -> None:
    _log_project_start(logger, prepared.project_code)
    for line in prepared.log_lines:
        logger.log(line)

    if prepared.status in {"missing_files", "compare_failed"}:
        error_path = _write_prepared_error_report(logger, error_root, prepared.project_name, prepared.failures)
        result.error_project_codes.append(prepared.project_code)
        result.error_report_paths.append(error_path)
        _log_project_end(logger, prepared.project_code)
        result.failed_count += 1
        return

    append_result = append_success_row(success_workbook_path, prepared.success_row)
    if append_result.status == "duplicate":
        logger.log("[写入成功台账] 检测到重复项目编码，跳过追加")
        error_path = _write_prepared_error_report(
            logger,
            error_root,
            prepared.project_name,
            _build_duplicate_failures(prepared.success_row),
        )
        result.error_project_codes.append(prepared.project_code)
        result.error_report_paths.append(error_path)
        _log_project_end(logger, prepared.project_code)
        result.duplicate_count += 1
        return

    logger.log(
        f"[写入成功台账] 已追加到第 {append_result.appended_row_index} 行 | 文件={success_workbook_path}"
    )
    _log_project_end(logger, prepared.project_code)
    result.appended_count += 1
    result.success_project_names.append(prepared.project_name)
    result.success_project_codes.append(prepared.project_code)


def _resolve_worker_count(project_count: int) -> int:
    if project_count <= 1:
        return 1
    cpu_count = os.cpu_count() or 4
    return max(1, min(project_count, cpu_count, 4))


def _project_code_for_logs(project_files: ProjectFiles, project_code: object) -> str:
    if project_code not in (None, ""):
        return str(project_code)
    return project_files.project_name


def _log_project_start(logger: RuntimeLogger, project_code: str) -> None:
    logger.log(f"[{project_code}------------开始------{project_code}]")


def _log_project_end(logger: RuntimeLogger, project_code: str) -> None:
    logger.log(f"[{project_code}------------结束------{project_code}]")


def _write_prepared_error_report(
    logger,
    error_root: Path,
    project_name: str,
    failures: list[CompareFailure],
) -> Path:
    error_path = write_error_report(error_root, project_name, failures)
    logger.log(f"[写入错误报告] {error_path}")
    return error_path


def _build_duplicate_failures(success_row: dict[str, object]) -> list[CompareFailure]:
    return [
        CompareFailure(
            field_name="项目编码",
            message="成功台账已存在相同项目编码，跳过追加",
            values={"项目编码": success_row.get("项目编码")},
        )
    ]


def _log_key_values(log_callback: Callable[[str], None], prefix: str, values: dict[str, object]) -> None:
    if not values:
        log_callback(f"{prefix} 未识别到字段")
        return

    for key, value in values.items():
        log_callback(f"{prefix} {key} = {_format_value(value)}")


def _xlsx_log_fields(raw_fields: dict[str, object]) -> dict[str, object]:
    return {field_name: raw_fields.get(field_name) for field_name in XLSX_LOG_FIELDS if field_name in raw_fields}


def _docx_log_fields(docx_data: DocxData) -> dict[str, object]:
    return {
        "项目编号": docx_data.project_code,
        "项目全称": docx_data.project_name,
        "用户联系人": _join_values(docx_data.contact_names or [docx_data.contact_name]),
        "用户联系方式": _join_values(docx_data.contact_phones or [docx_data.contact_phone]),
        "开始时间": docx_data.acceptance_start,
        "完成时间": docx_data.acceptance_end,
    }


def _pdf_log_fields(pdf_data: PdfData) -> dict[str, object]:
    return {
        "签字人姓名": pdf_data.signer_name,
        "签字人电话": pdf_data.signer_phone,
        "签字时间": pdf_data.sign_date,
        "是否有红章": pdf_data.has_red_stamp,
    }


def _format_failure_values(values: dict[str, object]) -> str:
    if not values:
        return "-"
    return ", ".join(f"{key}={_format_value(value)}" for key, value in values.items())


def _format_value(value: object) -> str:
    if value in (None, ""):
        return "<空>"
    if isinstance(value, (list, tuple, set)):
        return _join_values(list(value))
    return str(value)


def _join_values(values: list[object]) -> str:
    rendered = [str(value) for value in values if value not in (None, "")]
    if not rendered:
        return "<空>"
    return " | ".join(rendered)


def _log_pdf_recognition_status(logger: RuntimeLogger, ai_settings: AISettings | None) -> None:
    if is_remote_recognition_configured(ai_settings):
        logger.log(
            "[PDF识别] 流程: 搜索“甲方：” -> 裁剪签字区 -> 在线识别"
            " | 优先=AI"
            f" | AI={'已配置' if is_ai_recognition_configured(ai_settings) else '未配置'}"
            f" | OCR={'已配置' if is_ocr_recognition_configured(ai_settings) else '未配置'}"
            f" | 图片上限={ai_settings.image_max_kb}KB"
        )
        return
    logger.log("[PDF识别] 流程: 搜索“甲方：” -> 裁剪签字区 -> 在线识别(未启用，仅保留截图与文本层解析)")


def _call_with_supported_kwargs(callable_obj, **kwargs):
    signature = inspect.signature(callable_obj)
    parameters = signature.parameters.values()
    if any(parameter.kind == inspect.Parameter.VAR_KEYWORD for parameter in parameters):
        return kwargs
    return {key: value for key, value in kwargs.items() if key in signature.parameters}


def _run_web_phase(
    *,
    file_root: Path,
    username: str,
    password: str,
    project_dirs: list[Path],
    log_callback: Callable[[str], None] | None = None,
    processed_project_codes: set[str] | None = None,
) -> WebPhaseResult:
    del username, password, project_dirs

    summary = run_hollysys_download_batch(
        output_root=file_root,
        skip_project_codes=processed_project_codes or set(),
        log_callback=log_callback,
    )
    processed_projects = [Path(project_dir).name for project_dir in summary.get("saved_project_dirs", [])]
    skipped_projects = [
        str(item.get("project_dir_name", ""))
        for item in summary.get("skipped_projects", [])
        if item.get("project_dir_name")
    ]
    return WebPhaseResult(
        processed_projects=processed_projects,
        skipped_projects=skipped_projects,
    )


def _delete_project_dir(project_dir: Path) -> None:
    shutil.rmtree(project_dir)
