use crate::core::models::PdfData;
use crate::core::normalizers::{normalize_date, normalize_phone, normalize_project_code, normalize_text};
use regex::Regex;

pub fn parse_signature_text(text: &str) -> PdfData {
    let cleaned = normalize_text(text);
    let sign_date = Regex::new(
        r"(?:\d\s*){4}年\s*(?:\d\s*){1,2}月\s*(?:\d\s*){1,2}日|\d{4}[-/]\d{1,2}[-/]\d{1,2}",
    )
    .ok()
    .and_then(|re| {
        re.find(&cleaned)
            .and_then(|item| normalize_date(item.as_str()))
    });

    PdfData {
        project_code: extract_project_code(&cleaned),
        signer_name: extract_first_available(
            &cleaned,
            &[
                (
                    "签字人姓名",
                    &[
                        "签字人姓名",
                        "签字/盖章",
                        "联系电话",
                        "电话",
                        "签字时间",
                        "日期",
                    ][..],
                ),
                (
                    "签字/盖章",
                    &["签字/盖章", "联系电话", "电话", "签字时间", "日期"][..],
                ),
            ],
        ),
        signer_phone: extract_phone_after_labels(&cleaned, &["联系电话", "电话"]),
        sign_date,
        has_red_stamp: false,
    }
}

fn extract_project_code(text: &str) -> String {
    let Some(pattern) = Regex::new(r"[A-Z]{2,4}-\d{6,8}(?:[-/][A-Z0-9]+)+").ok() else {
        return String::new();
    };
    pattern
        .find(&text.to_uppercase())
        .map(|item| normalize_project_code(item.as_str()))
        .unwrap_or_default()
}

fn extract_first_available(text: &str, rules: &[(&str, &[&str])]) -> String {
    for (label, stop_labels) in rules {
        let value = extract_value(text, label, stop_labels);
        if !value.is_empty() {
            return value;
        }
    }
    String::new()
}

fn extract_value(text: &str, label: &str, stop_labels: &[&str]) -> String {
    let Some(start) = text.find(label) else {
        return String::new();
    };
    let mut tail = &text[start + label.len()..];
    if let Some(stop) = stop_labels
        .iter()
        .filter_map(|label| tail.find(label))
        .min()
    {
        tail = &tail[..stop];
    }
    normalize_text(tail)
        .trim_start_matches(['：', ':'])
        .to_string()
}

fn extract_phone_after_labels(text: &str, labels: &[&str]) -> String {
    let Ok(pattern) = Regex::new(r"1\d{10}") else {
        return String::new();
    };
    for label in labels {
        let Some(start) = text.find(label) else {
            continue;
        };
        let tail = text[start + label.len()..]
            .chars()
            .take(40)
            .collect::<String>();
        let digits = normalize_phone(&tail);
        if let Some(found) = pattern.find(&digits) {
            return found.as_str().to_string();
        }
    }
    String::new()
}
