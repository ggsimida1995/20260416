use crate::core::models::ProjectExtraction;
use crate::core::normalizers::normalize_date_value;
use anyhow::{Context, Result};
use calamine::{open_workbook_auto, Data, DataType, Reader};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const FIELD_NAME_HEADER: &str = "字段名称";
const CONTENT_HEADER: &str = "内容";

pub fn read_close_sheet(path: &Path) -> Result<ProjectExtraction> {
    let mut workbook =
        open_workbook_auto(path).with_context(|| format!("无法读取 Excel: {}", path.display()))?;
    let sheet_names = workbook.sheet_names().to_vec();
    for sheet_name in sheet_names {
        let Ok(range) = workbook.worksheet_range(&sheet_name) else {
            continue;
        };
        if !sheet_has_field_header(&range) {
            continue;
        }
        return Ok(ProjectExtraction {
            raw_fields: read_field_rows(&range),
        });
    }
    Ok(ProjectExtraction::default())
}

fn sheet_has_field_header(range: &calamine::Range<Data>) -> bool {
    field_header_position(range).is_some()
        || known_field_names()
            .iter()
            .any(|field_name| find_cell(range, field_name).is_some())
}

fn read_field_rows(range: &calamine::Range<Data>) -> BTreeMap<String, Value> {
    let mut fields = if let Some((header_row, field_col, value_col)) = field_header_position(range)
    {
        read_field_rows_by_header(range, header_row, field_col, value_col)
    } else {
        BTreeMap::new()
    };
    for field_name in known_field_names() {
        if fields
            .get(*field_name)
            .map(|value| !value.is_null())
            .unwrap_or(false)
        {
            continue;
        }
        if let Some(value) = find_value_by_label(range, field_name) {
            fields.insert(field_name.to_string(), value);
        }
    }
    fields
}

fn read_field_rows_by_header(
    range: &calamine::Range<Data>,
    header_row: usize,
    field_col: usize,
    value_col: usize,
) -> BTreeMap<String, Value> {
    let mut fields = BTreeMap::new();
    for row_index in (header_row + 1)..range.height() {
        let field_name = clean_cell(range.get((row_index, field_col)));
        if should_skip_field_name(&field_name) {
            continue;
        }
        let value = data_to_json_for_field(&field_name, range.get((row_index, value_col)));
        if !value.is_null() {
            fields.insert(field_name, value);
        }
    }
    fields
}

fn field_header_position(range: &calamine::Range<Data>) -> Option<(usize, usize, usize)> {
    let max_rows = range.height().min(20);
    for row_index in 0..max_rows {
        for col_index in 0..range.width() {
            let current = clean_cell(range.get((row_index, col_index)));
            if current != FIELD_NAME_HEADER {
                continue;
            }
            for value_col in (col_index + 1)..range.width().min(col_index + 5) {
                if clean_cell(range.get((row_index, value_col))) == CONTENT_HEADER {
                    return Some((row_index, col_index, value_col));
                }
            }
        }
    }
    None
}

fn known_field_names() -> &'static [&'static str] {
    &[
        "项目编号",
        "项目全称",
        "产品线",
        "项目类型",
        "老项目编号",
        "软件版本",
        "合同额（万元）",
        "核实方式",
        "核实人",
        "项目部",
        "项目经理",
        "移交人",
        "接收日期",
        "核实日期",
        "完成日期",
        "用户联系人",
        "用户职务",
        "用户联系方式",
        "验收报告",
        "CRM有无信息",
        "CRM有无信息关联主项目号",
        "验收日期",
    ]
}

fn should_skip_field_name(field_name: &str) -> bool {
    field_name.is_empty()
        || field_name == "项目关闭登记表"
        || field_name == FIELD_NAME_HEADER
        || field_name == CONTENT_HEADER
        || field_name == "备注"
        || field_name.starts_with("注：")
}

fn find_value_by_label(range: &calamine::Range<Data>, field_name: &str) -> Option<Value> {
    let (row, col) = find_cell(range, field_name)?;
    for next_col in (col + 1)..range.width().min(col + 6) {
        let value = data_to_json_for_field(field_name, range.get((row, next_col)));
        if is_meaningful_value(&value) {
            return Some(value);
        }
    }
    for next_row in (row + 1)..range.height().min(row + 4) {
        let value = data_to_json_for_field(field_name, range.get((next_row, col)));
        if is_meaningful_value(&value) {
            return Some(value);
        }
    }
    None
}

fn find_cell(range: &calamine::Range<Data>, target: &str) -> Option<(usize, usize)> {
    let target = normalize_field_label(target);
    for row_index in 0..range.height() {
        for col_index in 0..range.width() {
            let cell = normalize_field_label(&clean_cell(range.get((row_index, col_index))));
            if cell == target {
                return Some((row_index, col_index));
            }
        }
    }
    None
}

fn normalize_field_label(value: &str) -> String {
    value
        .replace(['\n', '\r', ' ', '　', ':', '：'], "")
        .trim()
        .to_string()
}

fn clean_cell(cell: Option<&Data>) -> String {
    data_to_string(cell).replace('\n', "").trim().to_string()
}

fn data_to_json(cell: Option<&Data>) -> Value {
    match cell {
        Some(Data::String(text)) => Value::String(text.clone()),
        Some(Data::Float(value)) if value.fract() == 0.0 => {
            Value::String((*value as i64).to_string())
        }
        Some(Data::Float(value)) => serde_json::Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Some(Data::Int(value)) => Value::String(value.to_string()),
        Some(Data::Bool(value)) => Value::Bool(*value),
        Some(Data::DateTime(value)) => Value::String(value.to_string()),
        Some(Data::DateTimeIso(value)) => Value::String(value.clone()),
        Some(Data::DurationIso(value)) => Value::String(value.clone()),
        Some(Data::Error(value)) => Value::String(value.to_string()),
        Some(Data::Empty) | None => Value::Null,
    }
}

fn data_to_json_for_field(field_name: &str, cell: Option<&Data>) -> Value {
    if is_date_field(field_name) {
        let raw_value = data_to_json(cell);
        if let Some(date) = normalize_date_value(Some(&raw_value)) {
            return Value::String(date.to_string());
        }
        if let Some(date) = excel_serial_date(cell) {
            return Value::String(date.to_string());
        }
    }
    data_to_json(cell)
}

fn excel_serial_date(cell: Option<&Data>) -> Option<chrono::NaiveDate> {
    let cell = cell?;
    match cell {
        Data::DateTime(_) | Data::DateTimeIso(_) => cell.as_date(),
        Data::Int(value) if (20_000..=80_000).contains(value) => cell.as_date(),
        Data::Float(value) if (20_000.0..=80_000.0).contains(value) => cell.as_date(),
        _ => None,
    }
}

fn is_date_field(field_name: &str) -> bool {
    field_name.contains("日期") || field_name.contains("时间")
}

fn is_meaningful_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => {
            let normalized = text.trim();
            !normalized.is_empty()
                && !known_field_names().iter().any(|field_name| {
                    normalize_field_label(field_name) == normalize_field_label(normalized)
                })
                && !reserved_values().contains(normalized)
        }
        _ => true,
    }
}

fn reserved_values() -> BTreeSet<&'static str> {
    [FIELD_NAME_HEADER, CONTENT_HEADER, "备注", "项目关闭登记表"]
        .into_iter()
        .collect()
}

fn data_to_string(cell: Option<&Data>) -> String {
    match data_to_json(cell) {
        Value::String(text) => text,
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{data_to_json_for_field, read_close_sheet};
    use calamine::Data;
    use serde_json::Value;
    use std::path::Path;

    #[test]
    fn reads_bhe_24080247_required_fields() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("file/project/BHE-24080247-01/BHE-24080247-01关闭移交登记表.xlsx");
        if !path.exists() {
            return;
        }

        let fields = read_close_sheet(&path).unwrap().raw_fields;
        assert_eq!(
            fields.get("用户联系人"),
            Some(&Value::String("倪叶星".to_string()))
        );
        assert_eq!(
            fields.get("用户联系方式"),
            Some(&Value::String("13016969600".to_string()))
        );
        assert_eq!(
            fields.get("验收日期"),
            Some(&Value::String("2026-05-27".to_string()))
        );
    }

    #[test]
    fn parses_compact_numeric_acceptance_date() {
        assert_eq!(
            data_to_json_for_field("验收日期", Some(&Data::Int(20260318))),
            Value::String("2026-03-18".to_string())
        );
        assert_eq!(
            data_to_json_for_field("验收日期", Some(&Data::Float(20260318.0))),
            Value::String("2026-03-18".to_string())
        );
    }

    #[test]
    fn reads_bhe_25110271_acceptance_date() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("file/project/BHE-25110271-01/BHE-2511027101 PHE-25120009B1 项目关闭移交登记表.xlsx");
        if !path.exists() {
            return;
        }

        let fields = read_close_sheet(&path).unwrap().raw_fields;
        assert_eq!(
            fields.get("验收日期"),
            Some(&Value::String("2026-03-18".to_string()))
        );
    }
}
