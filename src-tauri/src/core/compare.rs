use crate::core::models::{CompareFailure, CompareResult, DocxData, PdfData, WebData};
use crate::core::normalizers::{
    names_match_by_loose_pinyin, normalize_amount, normalize_compact_text, normalize_date_value,
    normalize_phone, normalize_project_code, normalize_text_value,
};
use chrono::NaiveDate;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn compare_project_data(
    xlsx_fields: &BTreeMap<String, Value>,
    web_data: Option<&WebData>,
    docx_data: &DocxData,
    pdf_data: &PdfData,
) -> CompareResult {
    let mut failures = Vec::new();

    compare_text_field(
        &mut failures,
        "项目编号",
        xlsx_fields.get("项目编号"),
        &docx_data.project_code,
        normalize_project_code,
        ("xlsx", "docx"),
        None,
    );
    compare_text_field(
        &mut failures,
        "项目编号",
        xlsx_fields.get("项目编号"),
        &pdf_data.project_code,
        normalize_project_code,
        ("xlsx", "pdf"),
        None,
    );
    if let Some(web_data) = web_data {
        compare_text_field(
            &mut failures,
            "项目编号",
            xlsx_fields.get("项目编号"),
            &web_data.project_code,
            normalize_project_code,
            ("xlsx", "web"),
            None,
        );
    }
    compare_text_field(
        &mut failures,
        "项目全称",
        xlsx_fields.get("项目全称"),
        &docx_data.project_name,
        normalize_compact_text,
        ("xlsx", "docx"),
        None,
    );
    if let Some(web_data) = web_data {
        compare_text_field(
            &mut failures,
            "项目全称",
            xlsx_fields.get("项目全称"),
            &web_data.project_name,
            normalize_compact_text,
            ("xlsx", "web"),
            None,
        );
    }
    compare_candidates_field(
        &mut failures,
        "用户姓名",
        xlsx_fields.get("用户联系人"),
        &docx_data.contact_names,
        |text| crate::core::normalizers::normalize_text(text),
        ("xlsx", "docx"),
    );
    compare_text_field(
        &mut failures,
        "用户姓名",
        xlsx_fields.get("用户联系人"),
        &pdf_data.signer_name,
        |text| crate::core::normalizers::normalize_text(text),
        ("xlsx", "pdf"),
        Some(names_match_by_loose_pinyin),
    );
    compare_candidates_field(
        &mut failures,
        "联系电话",
        xlsx_fields.get("用户联系方式"),
        &docx_data.contact_phones,
        normalize_phone,
        ("xlsx", "docx"),
    );
    compare_text_field(
        &mut failures,
        "联系电话",
        xlsx_fields.get("用户联系方式"),
        &pdf_data.signer_phone,
        normalize_phone,
        ("xlsx", "pdf"),
        None,
    );
    compare_acceptance_time(&mut failures, xlsx_fields, docx_data, pdf_data);
    compare_acceptance_range(&mut failures, docx_data, pdf_data);
    compare_stamp_rule(&mut failures, xlsx_fields, pdf_data);

    CompareResult {
        passed: failures.is_empty(),
        failures,
    }
}

fn compare_text_field(
    failures: &mut Vec<CompareFailure>,
    field_name: &str,
    xlsx_value: Option<&Value>,
    other_value: &str,
    normalizer: fn(&str) -> String,
    sources: (&str, &str),
    matcher: Option<fn(&str, &str) -> bool>,
) {
    let left_raw = normalize_text_value(xlsx_value);
    let left = normalizer(&left_raw);
    if left.is_empty() {
        return;
    }
    let right = normalizer(other_value);
    let matched = left == right
        || matcher
            .map(|item| item(&left_raw, other_value))
            .unwrap_or(false);
    if matched {
        return;
    }
    failures.push(CompareFailure {
        field_name: field_name.to_string(),
        message: format!("{} 与 {} 不一致", sources.0, sources.1),
        values: BTreeMap::from([
            (
                sources.0.to_string(),
                xlsx_value.cloned().unwrap_or(Value::Null),
            ),
            (
                sources.1.to_string(),
                Value::String(other_value.to_string()),
            ),
        ]),
    });
}

fn compare_candidates_field(
    failures: &mut Vec<CompareFailure>,
    field_name: &str,
    xlsx_value: Option<&Value>,
    other_values: &[String],
    normalizer: fn(&str) -> String,
    sources: (&str, &str),
) {
    let left = normalizer(&normalize_text_value(xlsx_value));
    if left.is_empty() {
        return;
    }
    let candidates = other_values
        .iter()
        .map(|item| normalizer(item))
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if candidates.iter().any(|item| item == &left) {
        return;
    }
    failures.push(CompareFailure {
        field_name: field_name.to_string(),
        message: format!("{} 与 {} 不一致", sources.0, sources.1),
        values: BTreeMap::from([
            (
                sources.0.to_string(),
                xlsx_value.cloned().unwrap_or(Value::Null),
            ),
            (
                sources.1.to_string(),
                Value::String(if other_values.is_empty() {
                    "<空>".to_string()
                } else {
                    other_values.join(" | ")
                }),
            ),
        ]),
    });
}

fn compare_acceptance_time(
    failures: &mut Vec<CompareFailure>,
    xlsx_fields: &BTreeMap<String, Value>,
    docx_data: &DocxData,
    pdf_data: &PdfData,
) {
    let excel_date = xlsx_date(xlsx_fields, &["验收日期", "完成日期", "核实日期"]);
    let docx_date = docx_data.acceptance_end;
    let pdf_date = pdf_data.sign_date;

    let Some(excel_date) = excel_date else {
        failures.push(CompareFailure {
            field_name: "验收时间".to_string(),
            message: "Excel 验收日期未识别".to_string(),
            values: acceptance_values(None, docx_date, pdf_date, docx_data.acceptance_start),
        });
        return;
    };
    let Some(docx_date) = docx_date else {
        failures.push(CompareFailure {
            field_name: "验收时间".to_string(),
            message: "Word 完成时间未识别".to_string(),
            values: acceptance_values(Some(excel_date), None, pdf_date, docx_data.acceptance_start),
        });
        return;
    };
    let Some(pdf_date) = pdf_date else {
        failures.push(CompareFailure {
            field_name: "验收时间".to_string(),
            message: "PDF 手写日期未识别".to_string(),
            values: acceptance_values(Some(excel_date), Some(docx_date), None, docx_data.acceptance_start),
        });
        return;
    };

    if excel_date == docx_date && docx_date == pdf_date {
        return;
    }

    failures.push(CompareFailure {
        field_name: "验收时间".to_string(),
        message: "Excel、Word、PDF 验收时间不一致".to_string(),
        values: acceptance_values(
            Some(excel_date),
            Some(docx_date),
            Some(pdf_date),
            docx_data.acceptance_start,
        ),
    });
}

fn compare_acceptance_range(
    failures: &mut Vec<CompareFailure>,
    docx_data: &DocxData,
    pdf_data: &PdfData,
) {
    if docx_data.has_invalid_acceptance_range {
        failures.push(CompareFailure {
            field_name: "竣工验收时间区间".to_string(),
            message: "开始时间晚于完成时间".to_string(),
            values: acceptance_values(
                None,
                docx_data.acceptance_end,
                pdf_data.sign_date,
                docx_data.acceptance_start,
            ),
        });
        return;
    }

    let Some(start) = docx_data.acceptance_start else {
        return;
    };
    let Some(end) = docx_data.acceptance_end else {
        return;
    };
    if start > end {
        failures.push(CompareFailure {
            field_name: "竣工验收时间区间".to_string(),
            message: "开始时间晚于完成时间".to_string(),
            values: acceptance_values(None, Some(end), pdf_data.sign_date, Some(start)),
        });
    }
}

fn compare_stamp_rule(
    failures: &mut Vec<CompareFailure>,
    xlsx_fields: &BTreeMap<String, Value>,
    pdf_data: &PdfData,
) {
    let amount = normalize_amount(xlsx_fields.get("合同额（万元）"));
    if amount.map(|value| value <= 50.0).unwrap_or(true) || pdf_data.has_red_stamp {
        return;
    }
    failures.push(CompareFailure {
        field_name: "盖章检查".to_string(),
        message: "合同额超过 50 万但 pdf 未识别到红章".to_string(),
        values: BTreeMap::from([
            (
                "合同额（万元）".to_string(),
                xlsx_fields
                    .get("合同额（万元）")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "pdf_has_red_stamp".to_string(),
                Value::Bool(pdf_data.has_red_stamp),
            ),
        ]),
    });
}

fn acceptance_values(
    excel_date: Option<NaiveDate>,
    docx_end: Option<NaiveDate>,
    pdf_date: Option<NaiveDate>,
    docx_start: Option<NaiveDate>,
) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("excel".to_string(), date_value(excel_date)),
        ("docx".to_string(), date_value(docx_end)),
        ("pdf".to_string(), date_value(pdf_date)),
        ("开始时间".to_string(), date_value(docx_start)),
    ])
}

fn xlsx_date(fields: &BTreeMap<String, Value>, names: &[&str]) -> Option<NaiveDate> {
    for name in names {
        if let Some(date) = normalize_date_value(fields.get(*name)) {
            return Some(date);
        }
    }
    None
}

fn date_value(value: Option<NaiveDate>) -> Value {
    value
        .map(|date| Value::String(date.to_string()))
        .unwrap_or(Value::Null)
}

#[allow(dead_code)]
fn _normalize_date_from_value(value: Option<&Value>) -> Option<NaiveDate> {
    normalize_date_value(value)
}
