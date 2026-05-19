use crate::core::config::{
    export_dir, success_workbook_path, workspace_state_db_path, SUCCESS_SHEET_NAME,
};
use crate::db::app_state::AppStateStore;
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessExportResult {
    pub workbook_path: String,
    pub status: String,
    pub pending_count: usize,
    pub appended_count: usize,
    pub duplicate_count: usize,
}

pub fn export_pending_success_rows(file_root: &Path) -> Result<SuccessExportResult> {
    let workbook_path = success_workbook_path(file_root);
    let store = AppStateStore::new(workspace_state_db_path(file_root));
    let records = store.pending_success_records()?;
    if records.is_empty() {
        return Ok(result(&workbook_path, "empty", 0, 0, 0));
    }
    if !workbook_path.exists() {
        return Ok(result(
            &workbook_path,
            "missing_workbook",
            records.len(),
            0,
            0,
        ));
    }

    let mut workbook = umya_spreadsheet::reader::xlsx::read(&workbook_path)
        .with_context(|| format!("无法读取成功台账: {}", workbook_path.display()))?;
    let sheet = workbook
        .get_sheet_by_name_mut(SUCCESS_SHEET_NAME)
        .ok_or_else(|| anyhow!("成功台账缺少工作表: {SUCCESS_SHEET_NAME}"))?;

    let headers = read_headers(sheet);
    let project_code_column = headers.get("项目编码").copied();
    let mut existing_codes = HashSet::new();
    if let Some(column) = project_code_column {
        let max_row = sheet.get_highest_row();
        for row in 2..=max_row {
            let value = sheet.get_cell_value((column, row)).get_value().to_string();
            if !value.is_empty() {
                existing_codes.insert(value);
            }
        }
    }

    let mut appended = 0;
    let mut duplicate = 0;
    let mut exported_codes = Vec::new();
    for record in &records {
        if !record.project_code.is_empty() && existing_codes.contains(&record.project_code) {
            duplicate += 1;
            exported_codes.push(record.project_code.clone());
            continue;
        }
        let target_row = sheet.get_highest_row() + 1;
        write_row(sheet, target_row, &headers, &record.row_data);
        if !record.project_code.is_empty() {
            existing_codes.insert(record.project_code.clone());
        }
        appended += 1;
        exported_codes.push(record.project_code.clone());
    }

    umya_spreadsheet::writer::xlsx::write(&workbook, &workbook_path)
        .with_context(|| format!("无法保存成功台账: {}", workbook_path.display()))?;
    store.mark_success_records_exported(&exported_codes)?;
    Ok(result(
        &workbook_path,
        "exported",
        records.len(),
        appended,
        duplicate,
    ))
}

pub fn export_error_records(file_root: &Path) -> Result<PathBuf> {
    let store = AppStateStore::new(workspace_state_db_path(file_root));
    let lines = store
        .latest_runtime_logs(100_000)?
        .into_iter()
        .filter(|line| line.contains("[项目比对]") && line.contains("\"passed\":false"))
        .collect::<Vec<_>>();
    let dir = export_dir(file_root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!(
        "compare-error-current-{}.txt",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::write(
        &path,
        if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        },
    )?;
    Ok(path)
}

fn result(
    path: &Path,
    status: &str,
    pending: usize,
    appended: usize,
    duplicate: usize,
) -> SuccessExportResult {
    SuccessExportResult {
        workbook_path: path.to_string_lossy().to_string(),
        status: status.to_string(),
        pending_count: pending,
        appended_count: appended,
        duplicate_count: duplicate,
    }
}

fn read_headers(sheet: &umya_spreadsheet::Worksheet) -> BTreeMap<String, u32> {
    let mut headers = BTreeMap::new();
    for column in 1..=sheet.get_highest_column() {
        let value = sheet.get_cell_value((column, 1)).get_value().to_string();
        if !value.is_empty() {
            headers.insert(value, column);
        }
    }
    headers
}

fn write_row(
    sheet: &mut umya_spreadsheet::Worksheet,
    row: u32,
    headers: &BTreeMap<String, u32>,
    row_data: &BTreeMap<String, Value>,
) {
    for (header, value) in row_data {
        let Some(column) = headers.get(header) else {
            continue;
        };
        sheet
            .get_cell_mut((*column, row))
            .set_value(value_to_string(value));
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
