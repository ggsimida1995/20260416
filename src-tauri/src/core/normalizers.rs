use chrono::NaiveDate;
use pinyin::ToPinyin;
use regex::Regex;
use serde_json::Value;

pub fn normalize_text_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => normalize_text(text),
        Some(Value::Number(number)) => normalize_text(&number.to_string()),
        Some(Value::Bool(value)) => value.to_string(),
        Some(other) => normalize_text(&other.to_string()),
        None => String::new(),
    }
}

pub fn normalize_text(text: &str) -> String {
    let mut result = String::new();
    let mut last_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_space {
                result.push(' ');
                last_space = true;
            }
        } else {
            result.push(ch);
            last_space = false;
        }
    }
    result.trim().to_string()
}

pub fn normalize_phone(text: &str) -> String {
    text.chars().filter(|ch| ch.is_ascii_digit()).collect()
}

pub fn normalize_project_code(text: &str) -> String {
    normalize_text(text)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

pub fn normalize_compact_text(text: &str) -> String {
    normalize_text(text)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

pub fn normalize_date_value(value: Option<&Value>) -> Option<NaiveDate> {
    value.and_then(|item| normalize_date(&normalize_text_value(Some(item))))
}

pub fn normalize_date(text: &str) -> Option<NaiveDate> {
    let text = normalize_text(text);
    let compact_text = text.replace(' ', "");
    for value in [&text, &compact_text] {
        for fmt in [
            "%Y-%m-%d",
            "%Y/%m/%d",
            "%Y.%m.%d",
            "%Y-%-m-%-d",
            "%Y/%-m/%-d",
            "%Y.%-m.%-d",
        ] {
            if let Ok(date) = NaiveDate::parse_from_str(value, fmt) {
                return Some(date);
            }
        }
    }

    for fmt in ["%Y年%m月%d日", "%Y年%-m月%-d日"] {
        if let Ok(date) = NaiveDate::parse_from_str(&compact_text, fmt) {
            return Some(date);
        }
    }

    if compact_text.len() == 8 && compact_text.chars().all(|ch| ch.is_ascii_digit()) {
        let year = compact_text[0..4].parse().ok()?;
        let month = compact_text[4..6].parse().ok()?;
        let day = compact_text[6..8].parse().ok()?;
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            return Some(date);
        }
    }

    for fmt in [
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%Y.%m.%d",
        "%Y-%-m-%-d",
        "%Y/%-m/%-d",
        "%Y.%-m.%-d",
    ] {
        if let Ok(date) = NaiveDate::parse_from_str(&text, fmt) {
            return Some(date);
        }
    }

    let compact =
        Regex::new(r"(?P<year>\d{4})\s*年\s*(?P<month>\d{1,2})\s*月\s*(?P<day>\d{1,2})\s*日")
            .ok()?;
    let captures = compact.captures(&text)?;
    let year = captures.name("year")?.as_str().parse().ok()?;
    let month = captures.name("month")?.as_str().parse().ok()?;
    let day = captures.name("day")?.as_str().parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

pub fn normalize_amount(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => {
            let cleaned = normalize_text(text)
                .replace(',', "")
                .replace('，', "")
                .replace("万元", "")
                .replace('万', "");
            cleaned.trim().parse().ok()
        }
        _ => None,
    }
}

pub fn names_match_by_loose_pinyin(left: &str, right: &str) -> bool {
    let left = normalize_compact_text(left);
    let right = normalize_compact_text(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    if !contains_chinese(&left) || !contains_chinese(&right) {
        return false;
    }
    normalize_name_pinyin(&left) == normalize_name_pinyin(&right)
}

fn contains_chinese(value: &str) -> bool {
    value
        .chars()
        .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
}

fn normalize_name_pinyin(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            ch.to_pinyin()
                .map(|py| normalize_pinyin_syllable(py.plain()))
                .unwrap_or_else(|| ch.to_string())
        })
        .collect::<String>()
}

fn normalize_pinyin_syllable(value: &str) -> String {
    let syllable = normalize_text(value)
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_lowercase())
        .collect::<String>();
    for suffix in ["ang", "eng", "ing", "ong"] {
        if syllable.ends_with(suffix) {
            return syllable[..syllable.len() - 1].to_string();
        }
    }
    syllable
}
