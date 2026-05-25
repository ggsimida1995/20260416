use crate::core::cancel::CancelFlag;
use crate::core::compare::compare_project_data;
use crate::core::config::{
    app_state_db_path, default_settings, ensure_workspace_layout, success_projects_root,
    workspace_file_root, workspace_state_db_path,
};
use crate::core::discovery::{discover_project_files, discover_projects, project_dir_names};
use crate::core::models::{
    AppSettings, DocxData, PdfData, PdfRecognitionContext, ProjectCompareLog, ProjectCompareLogRow,
    ProjectErrorDetail, ProjectExtraction, ProjectFiles, WebData, WorkflowProgress,
    WorkflowSummary,
};
use crate::core::normalizers::{normalize_date_value, normalize_phone, normalize_text};
use crate::db::app_state::{timestamp, AppStateStore, SuccessRecord};
use crate::readers::docx::read_docx;
use crate::readers::excel::read_close_sheet;
use crate::readers::pdf::{pdf_file_fingerprint, read_pdf};
use crate::readers::web_txt::read_web_txt;
use anyhow::Result;
use chrono::Local;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

const MAX_COMPARE_WORKERS: usize = 4;

pub fn run_compare_with_progress<F>(
    workspace_root: &Path,
    cancel: &CancelFlag,
    mut progress: F,
) -> Result<WorkflowSummary>
where
    F: FnMut(WorkflowProgress),
{
    ensure_workspace_layout(workspace_root)?;
    let file_root = workspace_file_root(workspace_root);
    let projects = discover_projects(&file_root)?;
    let state_db_path = workspace_state_db_path(workspace_root);
    let store = AppStateStore::new(state_db_path.clone());
    let settings = AppStateStore::new(app_state_db_path())
        .load_settings()?
        .unwrap_or_else(default_settings);
    store.append_runtime_log(&format!("[运行] 项目目录: {}", file_root.display()))?;
    store.append_runtime_log(&format!("[运行] 发现项目数量: {}", projects.len()))?;

    let total = projects.len();

    progress(compare_progress(
        "running",
        0,
        total,
        "",
        format!("开始本地比对，共 {total} 个项目"),
    ));

    let worker_count = compare_worker_count(total);
    if total > 1 {
        progress(compare_progress(
            "running",
            0,
            total,
            "",
            format!("本地比对并发处理: {worker_count} 个线程"),
        ));
    }

    let results = compare_projects_until_first_error(
        &file_root,
        &projects,
        &settings,
        &state_db_path,
        workspace_root,
        worker_count,
        cancel,
        |done, result| {
            let message = if result.error_details.is_empty() {
                format!("已处理 {}", result.project_name)
            } else {
                format!(
                    "已处理 {}，发现 {} 个问题",
                    result.project_name,
                    result.error_details.len()
                )
            };
            let log_line = serde_json::to_string(&result.log).unwrap_or_default();
            let _ = store.append_runtime_log(&format!("[项目比对] {log_line}"));
            progress(compare_progress_with_log(
                "running",
                done,
                total,
                &result.project_name,
                message,
                Some(result.log.clone()),
            ));
        },
    )?;

    let mut success_codes = Vec::new();
    let mut error_details = Vec::new();
    for result in results {
        if let Some(record) = result.success_record {
            success_codes.push(record.project_code.clone());
        }
        error_details.extend(result.error_details);
    }

    progress(compare_progress(
        "done",
        total,
        total,
        "",
        format!(
            "本地比对完成: 成功={} | 失败={}",
            success_codes.len(),
            error_details.len()
        ),
    ));
    summary(workspace_root, "compare")
}

#[derive(Debug)]
struct ProjectCompareResult {
    order: usize,
    project_name: String,
    success_record: Option<SuccessRecord>,
    error_details: Vec<ProjectErrorDetail>,
    log: ProjectCompareLog,
    abort_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct CompareLogFileNames {
    xlsx: String,
    docx: String,
    pdf: String,
    web: String,
}

fn compare_projects_until_first_error<F>(
    _file_root: &Path,
    projects: &[PathBuf],
    settings: &AppSettings,
    state_db_path: &Path,
    workspace_root: &Path,
    worker_count: usize,
    cancel: &CancelFlag,
    mut progress: F,
) -> Result<Vec<ProjectCompareResult>>
where
    F: FnMut(usize, &ProjectCompareResult),
{
    if projects.is_empty() {
        return Ok(Vec::new());
    }

    let worker_count = worker_count.max(1).min(projects.len());

    // Shared, read-only inputs: use Arc to avoid per-job clone() of AppSettings and paths.
    let settings = Arc::new(settings.clone());
    let state_db_path: Arc<Path> = Arc::from(state_db_path.to_path_buf().into_boxed_path());
    let workspace_root: Arc<Path> = Arc::from(workspace_root.to_path_buf().into_boxed_path());
    let projects: Arc<[PathBuf]> = Arc::from(projects.to_vec().into_boxed_slice());

    let next_index = Arc::new(AtomicUsize::new(0));
    let abort = Arc::new(AtomicBool::new(false));

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(worker_count)
        .thread_name(|i| format!("compare-worker-{i}"))
        .build()
        .map_err(|err| anyhow::anyhow!("failed to build rayon pool: {err}"))?;

    let (result_tx, result_rx) = mpsc::channel::<Result<ProjectCompareResult>>();

    for _ in 0..worker_count {
        let projects = Arc::clone(&projects);
        let settings = Arc::clone(&settings);
        let state_db_path = Arc::clone(&state_db_path);
        let workspace_root = Arc::clone(&workspace_root);
        let next_index = Arc::clone(&next_index);
        let abort = Arc::clone(&abort);
        let cancel = cancel.clone();
        let result_tx = result_tx.clone();
        pool.spawn(move || loop {
            if abort.load(Ordering::Relaxed) || cancel.is_cancelled() {
                break;
            }
            let index = next_index.fetch_add(1, Ordering::Relaxed);
            if index >= projects.len() {
                break;
            }
            let project_dir = projects[index].clone();
            let outcome = compare_one_project(
                &settings,
                &state_db_path,
                &workspace_root,
                index,
                project_dir,
            );
            if result_tx.send(outcome).is_err() {
                break;
            }
        });
    }
    drop(result_tx);

    let mut results = Vec::with_capacity(projects.len());
    let mut abort_reason: Option<String> = None;
    while let Ok(item) = result_rx.recv() {
        let result = item?;
        if let Some(reason) = result.abort_reason.clone() {
            abort_reason.get_or_insert(reason);
            abort.store(true, Ordering::Relaxed);
        }
        results.push(result);
        if let Some(latest) = results.last() {
            progress(results.len(), latest);
        }
    }

    results.sort_by_key(|item| item.order);
    if cancel.is_cancelled() {
        return Err(anyhow::anyhow!("已取消"));
    }
    if let Some(reason) = abort_reason {
        return Err(anyhow::anyhow!("{reason}"));
    }
    Ok(results)
}

fn compare_worker_count(total: usize) -> usize {
    total.clamp(1, MAX_COMPARE_WORKERS)
}

fn compare_one_project(
    settings: &AppSettings,
    state_db_path: &Path,
    workspace_root: &Path,
    order: usize,
    project_dir: PathBuf,
) -> Result<ProjectCompareResult> {
    let files = discover_project_files(&project_dir)?;
    Ok(compare_project_files(
        settings,
        state_db_path,
        workspace_root,
        order,
        files,
    ))
}

fn read_pdf_with_cache(
    path: &Path,
    settings: &AppSettings,
    detect_stamp: bool,
    context: &PdfRecognitionContext,
    store: &AppStateStore,
) -> Result<PdfData> {
    let fingerprint = pdf_cache_fingerprint(path, settings, detect_stamp, context)?;
    if let Some(data) = store.load_pdf_recognition_cache(path, &fingerprint)? {
        return Ok(data);
    }
    let data = read_pdf(path, settings, detect_stamp, context)?;
    store.save_pdf_recognition_cache(path, &fingerprint, &data)?;
    Ok(data)
}

fn move_success_project_dir(workspace_root: &Path, project_dir: &Path) -> Result<()> {
    let success_root = success_projects_root(workspace_root);
    fs::create_dir_all(&success_root)?;
    let name = project_dir
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("无法识别项目目录名称: {}", project_dir.display()))?;
    let mut target = success_root.join(name);
    if target.exists() {
        let timestamp = Local::now().format("%Y%m%d%H%M%S").to_string();
        let name = name.to_string_lossy();
        target = success_root.join(format!("{name}_{timestamp}"));
    }
    fs::rename(project_dir, &target).or_else(|_| move_dir_by_copy(project_dir, &target))?;
    Ok(())
}

fn move_dir_by_copy(source: &Path, target: &Path) -> Result<()> {
    copy_dir_all(source, target)?;
    fs::remove_dir_all(source)?;
    Ok(())
}

fn copy_dir_all(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_all(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn pdf_cache_fingerprint(
    path: &Path,
    settings: &AppSettings,
    detect_stamp: bool,
    context: &PdfRecognitionContext,
) -> Result<String> {
    let file = pdf_file_fingerprint(path)?;
    let context = serde_json::json!({
        "file": file,
        "detect_stamp": detect_stamp,
        "ai_enabled": settings.ai_enabled,
        "ai_base_url": settings.ai_base_url,
        "ai_model": settings.ai_model,
        "ocr_base_url": settings.ocr_base_url,
        "image_max_kb": settings.image_max_kb,
        "candidate_names": context.candidate_names,
        "candidate_phones": context.candidate_phones,
        "excel_acceptance_date": context.excel_acceptance_date.map(|date| date.to_string()),
        "acceptance_start": context.acceptance_start.map(|date| date.to_string()),
        "acceptance_end": context.acceptance_end.map(|date| date.to_string()),
    });
    Ok(context.to_string())
}

fn compare_project_files(
    settings: &AppSettings,
    state_db_path: &Path,
    workspace_root: &Path,
    order: usize,
    files: ProjectFiles,
) -> ProjectCompareResult {
    let mut project_code = files.project_name.clone();
    let mut error_details = Vec::new();
    let file_names = compare_log_file_names(&files);

    let extraction = if let Some(xlsx_path) = files.xlsx_path.as_ref() {
        match read_close_sheet(xlsx_path) {
            Ok(value) => Some(value),
            Err(error) => {
                error_details.push(read_error(&project_code, "xlsx读取", error));
                None
            }
        }
    } else {
        error_details.push(read_error(
            &project_code,
            "文件识别",
            anyhow::anyhow!("缺少 Excel 文件"),
        ));
        None
    };

    if let Some(extraction) = extraction.as_ref() {
        if let Some(code) = value_to_string(extraction.raw_fields.get("项目编号")) {
            project_code = code;
        }
    }

    let docx_data = if let Some(docx_path) = files.docx_path.as_ref() {
        match read_docx(docx_path) {
            Ok(value) => {
                if project_code == files.project_name && !value.project_code.trim().is_empty() {
                    project_code = value.project_code.trim().to_string();
                }
                Some(value)
            }
            Err(error) => {
                error_details.push(read_error(&project_code, "doc读取", error));
                None
            }
        }
    } else {
        error_details.push(read_error(
            &project_code,
            "文件识别",
            anyhow::anyhow!("缺少 Word 文件"),
        ));
        None
    };

    let web_data = if let Some(web_txt_path) = files.web_txt_path.as_ref() {
        match read_web_txt(web_txt_path) {
            Ok(value) => {
                if project_code == files.project_name && !value.project_code.trim().is_empty() {
                    project_code = value.project_code.trim().to_string();
                }
                Some(value)
            }
            Err(error) => {
                error_details.push(read_error(&project_code, "网页详情读取", error));
                None
            }
        }
    } else {
        error_details.push(read_error(
            &project_code,
            "文件识别",
            anyhow::anyhow!("缺少 网页详情文件"),
        ));
        None
    };

    let Some(extraction) = extraction.as_ref() else {
        return project_result_with_partial_log(
            order,
            files.project_name,
            project_code,
            &file_names,
            None,
            web_data.as_ref(),
            docx_data.as_ref(),
            None,
            error_details,
            if files.xlsx_path.is_some() {
                "Excel 读取失败"
            } else {
                "缺少 Excel 文件"
            },
        );
    };
    let Some(docx_data) = docx_data.as_ref() else {
        return project_result_with_partial_log(
            order,
            files.project_name,
            project_code,
            &file_names,
            Some(extraction),
            web_data.as_ref(),
            None,
            None,
            error_details,
            if files.docx_path.is_some() {
                "Word 读取失败"
            } else {
                "缺少 Word 文件"
            },
        );
    };

    let Some(pdf_path) = files.pdf_path.as_ref() else {
        error_details.push(read_error(
            &project_code,
            "文件识别",
            anyhow::anyhow!("缺少竣工验收报告文件"),
        ));
        return project_result_with_partial_log(
            order,
            files.project_name,
            project_code,
            &file_names,
            Some(extraction),
            web_data.as_ref(),
            Some(docx_data),
            None,
            error_details,
            "缺少竣工验收报告文件",
        );
    };

    let detect_stamp = should_detect_stamp(&extraction.raw_fields);
    let recognition_context = build_pdf_recognition_context(extraction, docx_data);
    let cache_store = AppStateStore::new(state_db_path.to_path_buf());
    let pdf_data = match read_pdf_with_cache(
        pdf_path,
        settings,
        detect_stamp,
        &recognition_context,
        &cache_store,
    ) {
        Ok(value) => value,
        Err(error) => {
            error_details.push(read_error(&project_code, "验收报告读取", error));
            return project_result_with_partial_log(
                order,
                files.project_name,
                project_code,
                &file_names,
                Some(extraction),
                web_data.as_ref(),
                Some(docx_data),
                None,
                error_details,
                "验收报告读取失败",
            );
        }
    };
    let compare = compare_project_data(
        &extraction.raw_fields,
        web_data.as_ref(),
        docx_data,
        &pdf_data,
    );
    if compare.passed {
        let row_data = build_success_row(&extraction.raw_fields);
        let success_record = SuccessRecord {
            project_code: project_code.clone(),
            project_name: files.project_name.clone(),
            row_data,
        };
        if let Err(error) = AppStateStore::new(state_db_path.to_path_buf())
            .append_success_record(&success_record)
            .and_then(|_| move_success_project_dir(workspace_root, &files.project_dir))
        {
            error_details.push(read_error(&project_code, "成功项目归档", error));
            let summary = format!("比对成功但归档失败，{} 个问题", error_details.len());
            let log = build_compare_log(
                &files.project_name,
                &project_code,
                Some(extraction),
                web_data.as_ref(),
                Some(docx_data),
                Some(&pdf_data),
                &file_names,
                false,
                &summary,
                &[],
            );
            return project_result(
                order,
                files.project_name,
                project_code,
                None,
                error_details,
                log,
            );
        }
        let log = build_compare_log(
            &files.project_name,
            &project_code,
            Some(extraction),
            web_data.as_ref(),
            Some(docx_data),
            Some(&pdf_data),
            &file_names,
            true,
            "比对成功",
            &[],
        );
        return project_result(
            order,
            files.project_name.clone(),
            project_code.clone(),
            Some(success_record),
            error_details,
            log,
        );
    }

    let compare_failures = compare.failures;
    error_details.extend(compare_failures.iter().map(|failure| ProjectErrorDetail {
        project_code: project_code.clone(),
        field_name: failure.field_name.clone(),
        message: failure.message.clone(),
        values: failure.values.clone(),
    }));
    let summary = format!("比对失败，{} 个问题", error_details.len());
    let log = build_compare_log(
        &files.project_name,
        &project_code,
        Some(extraction),
        web_data.as_ref(),
        Some(docx_data),
        Some(&pdf_data),
        &file_names,
        false,
        &summary,
        &compare_failures,
    );
    project_result(
        order,
        files.project_name,
        project_code,
        None,
        error_details,
        log,
    )
}

fn project_result(
    order: usize,
    project_name: String,
    _project_code: String,
    success_record: Option<SuccessRecord>,
    error_details: Vec<ProjectErrorDetail>,
    log: ProjectCompareLog,
) -> ProjectCompareResult {
    ProjectCompareResult {
        order,
        project_name,
        success_record,
        abort_reason: fatal_abort_reason(&error_details),
        error_details,
        log,
    }
}

fn fatal_abort_reason(error_details: &[ProjectErrorDetail]) -> Option<String> {
    error_details
        .iter()
        .find(|detail| is_fatal_process_error(detail))
        .map(|detail| {
            format!(
                "{} | {} | {}，已停止后续任务",
                detail.project_code, detail.field_name, detail.message
            )
        })
}

fn is_fatal_process_error(detail: &ProjectErrorDetail) -> bool {
    if detail.field_name == "文件识别" && detail.message.starts_with("缺少") {
        return false;
    }
    matches!(
        detail.field_name.as_str(),
        "文件识别" | "xlsx读取" | "doc读取" | "网页详情读取" | "验收报告读取"
    )
}

fn project_result_with_partial_log(
    order: usize,
    project_name: String,
    project_code: String,
    file_names: &CompareLogFileNames,
    extraction: Option<&ProjectExtraction>,
    web: Option<&WebData>,
    docx: Option<&DocxData>,
    pdf: Option<&PdfData>,
    error_details: Vec<ProjectErrorDetail>,
    summary: &str,
) -> ProjectCompareResult {
    let log = build_compare_log(
        &project_name,
        &project_code,
        extraction,
        web,
        docx,
        pdf,
        file_names,
        false,
        summary,
        &[],
    );
    project_result(order, project_name, project_code, None, error_details, log)
}

fn compare_progress(
    status: &str,
    current: usize,
    total: usize,
    project_name: &str,
    message: String,
) -> WorkflowProgress {
    compare_progress_with_log(status, current, total, project_name, message, None)
}

fn compare_progress_with_log(
    status: &str,
    current: usize,
    total: usize,
    project_name: &str,
    message: String,
    project_log: Option<ProjectCompareLog>,
) -> WorkflowProgress {
    WorkflowProgress {
        task_id: "compare".to_string(),
        stage: "本地比对".to_string(),
        status: status.to_string(),
        current,
        total,
        percent: percent(current, total),
        message,
        project_name: project_name.to_string(),
        project_log,
    }
}

fn build_compare_log(
    project_name: &str,
    project_code: &str,
    extraction: Option<&ProjectExtraction>,
    web: Option<&WebData>,
    docx: Option<&DocxData>,
    pdf: Option<&PdfData>,
    file_names: &CompareLogFileNames,
    passed: bool,
    summary: &str,
    compare_failures: &[crate::core::models::CompareFailure],
) -> ProjectCompareLog {
    ProjectCompareLog {
        project_name: project_name.to_string(),
        project_code: project_code.to_string(),
        passed,
        summary: summary.to_string(),
        finished_at: timestamp(),
        rows: vec![
            xlsx_log_row(&file_names.xlsx, project_code, project_name, extraction),
            docx_log_row(&file_names.docx, docx),
            pdf_log_row(&file_names.pdf, pdf),
            web_log_row(&file_names.web, web),
            result_log_row(passed, extraction, docx, pdf, compare_failures),
        ],
    }
}

fn xlsx_log_row(
    file_name: &str,
    project_code: &str,
    project_name: &str,
    extraction: Option<&ProjectExtraction>,
) -> ProjectCompareLogRow {
    let fields = extraction.map(|item| &item.raw_fields);
    ProjectCompareLogRow {
        file_name: file_name.to_string(),
        project_code: xlsx_field(fields, &["项目编号"])
            .unwrap_or_else(|| display_value(project_code)),
        project_name: xlsx_field(fields, &["项目全称"])
            .unwrap_or_else(|| display_value(project_name)),
        contact_name: xlsx_field(fields, &["用户联系人", "用户姓名", "联系人"])
            .unwrap_or_else(unrecognized),
        contact_phone: xlsx_field(fields, &["用户联系方式", "联系电话", "联系方式"])
            .unwrap_or_else(unrecognized),
        acceptance_time: xlsx_field(fields, &["验收日期", "完成日期", "核实日期"])
            .unwrap_or_else(unrecognized),
        start_time: dash(),
        amount: xlsx_field(fields, &["合同额（万元）"]).unwrap_or_else(unrecognized),
        has_red_stamp: dash(),
    }
}

fn docx_log_row(file_name: &str, docx: Option<&DocxData>) -> ProjectCompareLogRow {
    ProjectCompareLogRow {
        file_name: file_name.to_string(),
        project_code: docx
            .map(|item| display_value(&item.project_code))
            .unwrap_or_else(unrecognized),
        project_name: docx
            .map(|item| display_value(&item.project_name))
            .unwrap_or_else(unrecognized),
        contact_name: docx
            .map(|item| display_joined(&item.contact_names))
            .unwrap_or_else(unrecognized),
        contact_phone: docx
            .map(|item| display_joined(&item.contact_phones))
            .unwrap_or_else(unrecognized),
        acceptance_time: docx
            .and_then(|item| item.acceptance_end)
            .map(|date| date.to_string())
            .unwrap_or_else(unrecognized),
        start_time: docx
            .and_then(|item| item.acceptance_start)
            .map(|date| date.to_string())
            .unwrap_or_else(unrecognized),
        amount: dash(),
        has_red_stamp: dash(),
    }
}

fn pdf_log_row(file_name: &str, pdf: Option<&PdfData>) -> ProjectCompareLogRow {
    ProjectCompareLogRow {
        file_name: file_name.to_string(),
        project_code: dash(),
        project_name: dash(),
        contact_name: pdf
            .map(|item| display_value(&item.signer_name))
            .unwrap_or_else(unrecognized),
        contact_phone: pdf
            .map(|item| display_value(&item.signer_phone))
            .unwrap_or_else(unrecognized),
        acceptance_time: pdf
            .and_then(|item| item.sign_date)
            .map(|date| date.to_string())
            .unwrap_or_else(unrecognized),
        start_time: dash(),
        amount: dash(),
        has_red_stamp: pdf
            .map(|item| red_stamp_text(item.has_red_stamp))
            .unwrap_or_else(unrecognized),
    }
}

fn web_log_row(file_name: &str, web: Option<&WebData>) -> ProjectCompareLogRow {
    ProjectCompareLogRow {
        file_name: file_name.to_string(),
        project_code: web
            .map(|item| display_value(&item.project_code))
            .unwrap_or_else(unrecognized),
        project_name: web
            .map(|item| display_value(&item.project_name))
            .unwrap_or_else(unrecognized),
        contact_name: dash(),
        contact_phone: dash(),
        acceptance_time: dash(),
        start_time: dash(),
        amount: dash(),
        has_red_stamp: dash(),
    }
}

fn result_log_row(
    passed: bool,
    extraction: Option<&ProjectExtraction>,
    docx: Option<&DocxData>,
    pdf: Option<&PdfData>,
    compare_failures: &[crate::core::models::CompareFailure],
) -> ProjectCompareLogRow {
    let amount_over_threshold = extraction
        .and_then(|item| {
            crate::core::normalizers::normalize_amount(item.raw_fields.get("合同额（万元）"))
        })
        .map(|value| value > 50.0)
        .unwrap_or(false);
    ProjectCompareLogRow {
        file_name: "比对结果".to_string(),
        project_code: compare_mark(passed, compare_failures, &["项目编号"]),
        project_name: compare_mark(passed, compare_failures, &["项目全称"]),
        contact_name: compare_mark(passed, compare_failures, &["用户姓名"]),
        contact_phone: compare_mark(passed, compare_failures, &["联系电话"]),
        start_time: range_compare_mark(passed, compare_failures, docx),
        acceptance_time: compare_mark(passed, compare_failures, &["验收时间"]),
        amount: threshold_mark(amount_over_threshold),
        has_red_stamp: stamp_compare_mark(amount_over_threshold, pdf, compare_failures),
    }
}

fn compare_mark(
    passed: bool,
    failures: &[crate::core::models::CompareFailure],
    field_names: &[&str],
) -> String {
    if passed {
        return "✅".to_string();
    }
    if failures.is_empty() {
        return dash();
    }
    if failures
        .iter()
        .any(|failure| field_names.contains(&failure.field_name.as_str()))
    {
        "❌".to_string()
    } else {
        "✅".to_string()
    }
}

fn range_compare_mark(
    passed: bool,
    failures: &[crate::core::models::CompareFailure],
    docx: Option<&DocxData>,
) -> String {
    if passed {
        return "✅".to_string();
    }
    if failures.is_empty() {
        return dash();
    }
    if failures
        .iter()
        .any(|failure| failure.field_name == "竣工验收时间区间" || failure.field_name == "验收时间")
    {
        "❌".to_string()
    } else {
        docx.map(|item| {
            if item.acceptance_start.is_some() && item.acceptance_end.is_some() {
                "✅".to_string()
            } else {
                dash()
            }
        })
        .unwrap_or_else(dash)
    }
}

fn threshold_mark(over_threshold: bool) -> String {
    if !over_threshold {
        return dash();
    }
    "✅".to_string()
}

fn stamp_compare_mark(
    amount_over_threshold: bool,
    pdf: Option<&PdfData>,
    failures: &[crate::core::models::CompareFailure],
) -> String {
    if !amount_over_threshold {
        return dash();
    }
    if failures
        .iter()
        .any(|failure| failure.field_name == "盖章检查")
    {
        return "❌".to_string();
    }
    if pdf.is_some() {
        "✅".to_string()
    } else {
        dash()
    }
}

fn compare_log_file_names(files: &ProjectFiles) -> CompareLogFileNames {
    let _ = files;
    CompareLogFileNames {
        xlsx: "关闭移交登记表".to_string(),
        docx: "竣工总结报告".to_string(),
        pdf: "竣工验收报告".to_string(),
        web: "网页".to_string(),
    }
}

fn xlsx_field(fields: Option<&BTreeMap<String, Value>>, names: &[&str]) -> Option<String> {
    let fields = fields?;
    for name in names {
        if let Some(value) = value_to_string(fields.get(*name)) {
            let display = display_value(&value);
            if display != "未识别" {
                return Some(display);
            }
        }
    }
    None
}

fn display_joined(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| display_value(value))
        .filter(|value| value != "未识别")
        .collect::<Vec<_>>();
    if items.is_empty() {
        unrecognized()
    } else {
        items.join(" | ")
    }
}

fn display_value(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        unrecognized()
    } else {
        value.to_string()
    }
}

fn unrecognized() -> String {
    "未识别".to_string()
}

fn dash() -> String {
    "—".to_string()
}

fn red_stamp_text(value: bool) -> String {
    if value {
        "有".to_string()
    } else {
        "无".to_string()
    }
}

fn percent(current: usize, total: usize) -> u8 {
    if total == 0 {
        return 100;
    }
    ((current.min(total) * 100) / total).min(100) as u8
}

pub fn summary(workspace_root: &Path, mode: &str) -> Result<WorkflowSummary> {
    let file_root = workspace_file_root(workspace_root);
    let store = AppStateStore::new(workspace_state_db_path(workspace_root));
    Ok(WorkflowSummary {
        mode: mode.to_string(),
        updated_at: timestamp(),
        pending_success_count: store.count_pending_success_records()?,
        failed_count: count_failed_project_logs(&store)?,
        project_count: project_dir_names(&file_root).len(),
        downloaded_project_names: project_dir_names(&file_root),
    })
}

fn count_failed_project_logs(store: &AppStateStore) -> Result<usize> {
    Ok(store
        .latest_runtime_logs(10_000)?
        .into_iter()
        .filter(|line| line.contains("[项目比对]") && line.contains("\"passed\":false"))
        .count())
}

fn read_error(project_code: &str, field_name: &str, error: anyhow::Error) -> ProjectErrorDetail {
    ProjectErrorDetail {
        project_code: project_code.to_string(),
        field_name: field_name.to_string(),
        message: error.to_string(),
        values: BTreeMap::new(),
    }
}

fn should_detect_stamp(fields: &BTreeMap<String, Value>) -> bool {
    crate::core::normalizers::normalize_amount(fields.get("合同额（万元）"))
        .map(|value| value > 50.0)
        .unwrap_or(false)
}

fn build_pdf_recognition_context(
    extraction: &ProjectExtraction,
    docx: &DocxData,
) -> PdfRecognitionContext {
    let mut context = PdfRecognitionContext {
        excel_acceptance_date: xlsx_date(
            &extraction.raw_fields,
            &["验收日期", "完成日期", "核实日期"],
        ),
        acceptance_start: docx.acceptance_start,
        acceptance_end: docx.acceptance_end,
        ..PdfRecognitionContext::default()
    };

    for key in ["用户联系人", "用户姓名", "联系人"] {
        if let Some(value) = value_to_string(extraction.raw_fields.get(key)) {
            push_text_candidate(&mut context.candidate_names, &value);
        }
    }
    for value in &docx.contact_names {
        push_text_candidate(&mut context.candidate_names, value);
    }

    for key in ["用户联系方式", "联系电话", "联系方式"] {
        if let Some(value) = value_to_string(extraction.raw_fields.get(key)) {
            push_phone_candidate(&mut context.candidate_phones, &value);
        }
    }
    for value in &docx.contact_phones {
        push_phone_candidate(&mut context.candidate_phones, value);
    }

    context
}

fn xlsx_date(fields: &BTreeMap<String, Value>, names: &[&str]) -> Option<chrono::NaiveDate> {
    for name in names {
        if let Some(date) = normalize_date_value(fields.get(*name)) {
            return Some(date);
        }
    }
    None
}

fn push_text_candidate(candidates: &mut Vec<String>, value: &str) {
    let normalized = normalize_text(value);
    if normalized.is_empty()
        || candidates
            .iter()
            .any(|item| normalize_text(item) == normalized)
    {
        return;
    }
    candidates.push(normalized);
}

fn push_phone_candidate(candidates: &mut Vec<String>, value: &str) {
    let normalized = normalize_phone(value);
    if normalized.is_empty()
        || candidates
            .iter()
            .any(|item| normalize_phone(item) == normalized)
    {
        return;
    }
    candidates.push(normalized);
}

pub fn build_success_row(fields: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let mut row = BTreeMap::new();
    for (source, target) in success_field_mapping() {
        if let Some(value) = fields.get(source) {
            if !value.is_null()
                && value_to_string(Some(value))
                    .map(|item| !item.is_empty())
                    .unwrap_or(false)
            {
                row.insert(target.to_string(), value.clone());
            }
        }
    }
    row
}

fn success_field_mapping() -> Vec<(&'static str, &'static str)> {
    vec![
        ("项目编号", "项目编码"),
        ("项目全称", "项目全称"),
        ("产品线", "产品线"),
        ("项目类型", "项目类型"),
        ("老项目编号", "老项目号"),
        ("软件版本", "软件版本"),
        ("合同额（万元）", "合同额（万元）"),
        ("核实方式", "核实方式"),
        ("核实人", "核实人"),
        ("项目部", "项目部"),
        ("项目经理", "项目经理"),
        ("移交人", "移交人"),
        ("接收日期", "接收日期"),
        ("核实日期", "核实日期"),
        ("完成日期", "完成关闭"),
        ("用户联系人", "联系人"),
        ("用户职务", "职务"),
        ("用户联系方式", "联系方式"),
        ("验收报告", "验收报告"),
        ("CRM有无信息", "CRM有无信息"),
        ("CRM有无信息关联主项目号", "CRM有无信息关联主项目号"),
        ("验收日期", "验收日期"),
    ]
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::Bool(value)) => Some(value.to_string()),
        Some(Value::Null) | None => None,
        Some(other) => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{fatal_abort_reason, move_success_project_dir};
    use crate::core::models::ProjectErrorDetail;
    use std::collections::BTreeMap;
    use std::fs;

    fn error_detail(field_name: &str, message: &str) -> ProjectErrorDetail {
        ProjectErrorDetail {
            project_code: "BHE-TEST".to_string(),
            field_name: field_name.to_string(),
            message: message.to_string(),
            values: BTreeMap::new(),
        }
    }

    #[test]
    fn missing_required_files_do_not_abort_batch_compare() {
        let details = vec![error_detail("文件识别", "缺少竣工验收报告文件")];

        assert!(fatal_abort_reason(&details).is_none());
    }

    #[test]
    fn reader_or_service_errors_still_abort_batch_compare() {
        let details = vec![error_detail("验收报告读取", "AI 识别不可用")];

        assert!(fatal_abort_reason(&details).is_some());
    }

    #[test]
    fn moves_success_project_dir_out_of_file_root() {
        let root =
            std::env::temp_dir().join(format!("project-success-move-test-{}", std::process::id()));
        let project = root.join("file").join("BHE-TEST");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("marker.txt"), "ok").unwrap();

        move_success_project_dir(&root, &project).unwrap();

        assert!(!project.exists());
        assert!(root
            .join("success_projects")
            .join("BHE-TEST")
            .join("marker.txt")
            .exists());

        let _ = fs::remove_dir_all(&root);
    }
}
