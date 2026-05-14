use crate::core::compare::compare_project_data;
use crate::core::config::{app_state_db_path, default_settings, workspace_state_db_path};
use crate::core::discovery::{discover_project_files, discover_projects, project_dir_names};
use crate::core::models::{
    AppSettings, DocxData, PdfData, ProjectCompareLog, ProjectCompareLogRow, ProjectErrorDetail,
    ProjectExtraction, ProjectFiles, WebData, WorkflowProgress, WorkflowSummary,
};
use crate::db::app_state::{timestamp, AppStateStore, SuccessRecord};
use crate::readers::docx::read_docx;
use crate::readers::excel::read_close_sheet;
use crate::readers::pdf::read_pdf;
use crate::readers::web_txt::read_web_txt;
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

const MAX_COMPARE_WORKERS: usize = 4;

pub fn run_compare_with_progress<F>(file_root: &Path, mut progress: F) -> Result<WorkflowSummary>
where
    F: FnMut(WorkflowProgress),
{
    let projects = discover_projects(file_root)?;
    let store = AppStateStore::new(workspace_state_db_path(file_root));
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

    let results =
        compare_projects_parallel(file_root, &projects, &settings, worker_count, |done, result| {
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
        })?;

    let mut success_codes = Vec::new();
    let mut success_records = Vec::new();
    let mut error_details = Vec::new();
    for result in results {
        if let Some(record) = result.success_record {
            success_codes.push(record.project_code.clone());
            success_records.push(record);
        }
        error_details.extend(result.error_details);
    }

    store.append_result_logs(&success_codes, &error_details, &success_records)?;
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
    summary(file_root, "compare")
}

#[derive(Debug)]
struct ProjectCompareResult {
    order: usize,
    project_name: String,
    success_record: Option<SuccessRecord>,
    error_details: Vec<ProjectErrorDetail>,
    log: ProjectCompareLog,
}

#[derive(Debug, Clone)]
struct CompareLogFileNames {
    xlsx: String,
    docx: String,
    pdf: String,
    web: String,
}

fn compare_projects_parallel<F>(
    _file_root: &Path,
    projects: &[PathBuf],
    settings: &AppSettings,
    worker_count: usize,
    mut progress: F,
) -> Result<Vec<ProjectCompareResult>>
where
    F: FnMut(usize, &ProjectCompareResult),
{
    if projects.is_empty() {
        return Ok(Vec::new());
    }

    let worker_count = worker_count.max(1).min(projects.len());
    let (job_tx, job_rx) = mpsc::channel::<(usize, PathBuf)>();
    let (result_tx, result_rx) = mpsc::channel::<Result<ProjectCompareResult>>();
    thread::scope(|scope| {
        let mut worker_txs = Vec::new();
        for _ in 0..worker_count {
            let (worker_tx, worker_rx) = mpsc::channel::<(usize, PathBuf)>();
            worker_txs.push(worker_tx);
            let result_tx = result_tx.clone();
            let settings = settings.clone();
            scope.spawn(move || {
                for (order, project_dir) in worker_rx {
                    let result = compare_one_project(&settings, order, project_dir);
                    if result_tx.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(result_tx);

        for (index, project_dir) in projects.iter().cloned().enumerate() {
            job_tx.send((index, project_dir))?;
        }
        drop(job_tx);

        for (index, job) in job_rx.into_iter().enumerate() {
            let worker_index = index % worker_txs.len();
            if worker_txs[worker_index].send(job).is_err() {
                break;
            }
        }
        drop(worker_txs);

        let mut done = 0usize;
        let mut results = Vec::with_capacity(projects.len());
        for result in result_rx {
            let result = result?;
            done += 1;
            progress(done, &result);
            results.push(result);
        }
        results.sort_by_key(|item| item.order);
        Ok(results)
    })
}

fn compare_one_project(
    settings: &AppSettings,
    order: usize,
    project_dir: PathBuf,
) -> Result<ProjectCompareResult> {
    let files = discover_project_files(&project_dir)?;
    Ok(compare_project_files(settings, order, files))
}

fn compare_project_files(
    settings: &AppSettings,
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
        error_details.push(read_error(&project_code, "文件识别", anyhow::anyhow!("缺少 Excel 文件")));
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
        error_details.push(read_error(&project_code, "文件识别", anyhow::anyhow!("缺少 Word 文件")));
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
        error_details.push(read_error(&project_code, "文件识别", anyhow::anyhow!("缺少 网页详情文件")));
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
            if files.xlsx_path.is_some() { "Excel 读取失败" } else { "缺少 Excel 文件" },
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
            if files.docx_path.is_some() { "Word 读取失败" } else { "缺少 Word 文件" },
        );
    };

    let Some(pdf_path) = files.pdf_path.as_ref() else {
        error_details.push(read_error(&project_code, "文件识别", anyhow::anyhow!("缺少 PDF 文件")));
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
                "缺少 PDF 文件",
            );
    };

    let detect_stamp = should_detect_stamp(&extraction.raw_fields);
    let pdf_data = match read_pdf(pdf_path, settings, detect_stamp) {
        Ok(value) => value,
        Err(error) => {
            error_details.push(read_error(&project_code, "pdf读取", error));
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
                "PDF 读取失败",
            );
        }
    };

    let compare = compare_project_data(&extraction.raw_fields, web_data.as_ref(), docx_data, &pdf_data);
    if compare.passed {
        let row_data = build_success_row(&extraction.raw_fields);
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
        );
        return project_result(
            order,
            files.project_name.clone(),
            project_code.clone(),
            Some(SuccessRecord {
                project_code,
                project_name: files.project_name,
                row_data,
            }),
            error_details,
            log,
        );
    }

    error_details.extend(
        compare
            .failures
            .into_iter()
            .map(|failure| ProjectErrorDetail {
                project_code: project_code.clone(),
                field_name: failure.field_name,
                message: failure.message,
                values: failure.values,
            }),
    );
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
    );
    project_result(order, files.project_name, project_code, None, error_details, log)
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
        error_details,
        log,
    }
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
    );
    project_result(order, project_name, project_code, None, error_details, log)
}

fn compare_worker_count(total: usize) -> usize {
    if total <= 1 {
        return 1;
    }
    let available = thread::available_parallelism()
        .map(|item| item.get())
        .unwrap_or(2);
    available.min(MAX_COMPARE_WORKERS).min(total).max(1)
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
            result_log_row(passed, summary, extraction, pdf),
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
        project_code: xlsx_field(fields, &["项目编号"]).unwrap_or_else(|| display_value(project_code)),
        project_name: xlsx_field(fields, &["项目全称"]).unwrap_or_else(|| display_value(project_name)),
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
    summary: &str,
    extraction: Option<&ProjectExtraction>,
    pdf: Option<&PdfData>,
) -> ProjectCompareLogRow {
    let fields = extraction.map(|item| &item.raw_fields);
    ProjectCompareLogRow {
        file_name: "比对结果".to_string(),
        project_code: if passed { "通过" } else { "失败" }.to_string(),
        project_name: display_value(summary),
        contact_name: dash(),
        contact_phone: dash(),
        acceptance_time: dash(),
        start_time: dash(),
        amount: xlsx_field(fields, &["合同额（万元）"]).unwrap_or_else(unrecognized),
        has_red_stamp: pdf
            .map(|item| red_stamp_text(item.has_red_stamp))
            .unwrap_or_else(unrecognized),
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

pub fn summary(file_root: &Path, mode: &str) -> Result<WorkflowSummary> {
    let store = AppStateStore::new(workspace_state_db_path(file_root));
    Ok(WorkflowSummary {
        mode: mode.to_string(),
        updated_at: timestamp(),
        success_project_codes: store.latest_result_logs("success", 20)?,
        error_project_codes: store.latest_result_logs("error", 20)?,
        success_count: store.count_result_logs("success")?,
        pending_success_count: store.count_pending_success_records()?,
        failed_count: store.count_result_logs("error")?,
        project_count: project_dir_names(file_root).len(),
        downloaded_project_names: project_dir_names(file_root),
    })
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
