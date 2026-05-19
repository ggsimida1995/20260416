use crate::core::models::PdfData;
use crate::core::normalizers::{
    normalize_date, normalize_phone, normalize_project_code, normalize_text,
};
use regex::Regex;

pub fn parse_signature_text(text: &str) -> PdfData {
    let cleaned = normalize_text(text);
    let sign_date = extract_signature_date(&cleaned);

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
        signer_name_confidence: None,
        signer_phone_confidence: None,
        sign_date_confidence: None,
    }
}

fn extract_signature_date(text: &str) -> Option<chrono::NaiveDate> {
    let direct = Regex::new(
        r"(?:\d\s*){4}年\s*(?:\d\s*){1,2}月\s*(?:\d\s*){1,2}日|\d{4}[-/]\d{1,2}[-/]\d{1,2}",
    )
    .ok()
    .and_then(|re| re.find(text).and_then(|item| normalize_date(item.as_str())));
    if direct.is_some() {
        return direct;
    }

    let reversed =
        Regex::new(r"(?P<day>\d{1,2})\s*日\s*(?P<year>\d{4})\s*年\s*(?P<month>\d{1,2})\s*月")
            .ok()?;
    let captures = reversed.captures(text)?;
    let year = captures.name("year")?.as_str().parse().ok()?;
    let month = captures.name("month")?.as_str().parse().ok()?;
    let day = captures.name("day")?.as_str().parse().ok()?;
    chrono::NaiveDate::from_ymd_opt(year, month, day)
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

#[cfg(test)]
mod tests {
    use super::parse_signature_text;
    use chrono::NaiveDate;

    #[test]
    fn parses_reversed_ocr_date() {
        let data = parse_signature_text("工有 y 张 签字/盖章 山 花 11 电话： 16日 2026 年 4月");

        assert_eq!(data.sign_date, NaiveDate::from_ymd_opt(2026, 4, 16));
    }
}
