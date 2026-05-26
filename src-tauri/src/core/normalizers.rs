use chrono::NaiveDate;
use pinyin::ToPinyin;
use regex::Regex;
use serde_json::Value;

pub fn normalize_text_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => normalize_text(text),
        Some(Value::Number(number)) => normalize_text(&number.to_string()),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Null) => String::new(),
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
        .filter_map(normalize_project_code_char)
        .collect::<String>()
        .to_uppercase()
}

fn normalize_project_code_char(ch: char) -> Option<char> {
    if ch.is_whitespace() || matches!(ch, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}') {
        return None;
    }
    Some(match ch {
        '／' | '⁄' | '∕' => '/',
        _ => ch,
    })
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
    let left_text = normalize_text(left);
    let right_text = normalize_text(right);
    if left_text.is_empty() || right_text.is_empty() {
        return false;
    }
    if left_text == right_text {
        return true;
    }

    let left = normalize_compact_text(&left_text);
    let right = normalize_compact_text(&right_text);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if !contains_chinese(&left) || !contains_chinese(&right) {
        return false;
    }
    if left == right {
        return true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    #[test]
    fn normalize_text_collapses_whitespace() {
        assert_eq!(normalize_text("  hello   world\n\t"), "hello world");
        assert_eq!(normalize_text("中文 \u{3000}空格"), "中文 空格");
        assert_eq!(normalize_text(""), "");
    }

    #[test]
    fn normalize_text_value_handles_variants() {
        assert_eq!(normalize_text_value(None), "");
        assert_eq!(normalize_text_value(Some(&json!("  abc  "))), "abc");
        assert_eq!(normalize_text_value(Some(&json!(42))), "42");
        assert_eq!(normalize_text_value(Some(&json!(true))), "true");
        assert_eq!(normalize_text_value(Some(&Value::Null)), "");
    }

    #[test]
    fn normalize_phone_keeps_only_ascii_digits() {
        assert_eq!(normalize_phone("(010) 6677-8899"), "01066778899");
        assert_eq!(normalize_phone("１３８-1234-5678"), "12345678");
        assert_eq!(normalize_phone(""), "");
    }

    #[test]
    fn normalize_project_code_uppercases_and_strips_whitespace() {
        assert_eq!(
            normalize_project_code("  bhe-25080117-01 "),
            "BHE-25080117-01"
        );
        assert_eq!(normalize_project_code("lhe 25090002 b1"), "LHE25090002B1");
        assert_eq!(
            normalize_project_code("BHE-25110001 / Z1"),
            "BHE-25110001/Z1"
        );
        assert_eq!(
            normalize_project_code("BHE-25110001\u{200B}/\u{FEFF}Z1"),
            "BHE-25110001/Z1"
        );
        assert_eq!(
            normalize_project_code("BHE-25110001 ／ Z1"),
            "BHE-25110001/Z1"
        );
    }

    #[test]
    fn normalize_compact_text_drops_whitespace_only() {
        assert_eq!(normalize_compact_text("张 三"), "张三");
        assert_eq!(normalize_compact_text(" 张 三 "), "张三");
        assert_eq!(normalize_compact_text("a b c"), "abc");
    }

    #[test]
    fn normalize_date_parses_common_formats() {
        let expected = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        assert_eq!(normalize_date("2026-03-18"), Some(expected));
        assert_eq!(normalize_date("2026/3/18"), Some(expected));
        assert_eq!(normalize_date("2026.03.18"), Some(expected));
        assert_eq!(normalize_date("2026年3月18日"), Some(expected));
        assert_eq!(normalize_date(" 2026 年 03 月 18 日 "), Some(expected));
        assert_eq!(normalize_date("20260318"), Some(expected));
    }

    #[test]
    fn normalize_date_rejects_garbage() {
        assert_eq!(normalize_date(""), None);
        assert_eq!(normalize_date("not a date"), None);
        assert_eq!(normalize_date("2026-13-40"), None);
    }

    #[test]
    fn normalize_date_value_handles_optional() {
        let expected = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        assert_eq!(normalize_date_value(Some(&json!("2026-3-18"))), Some(expected));
        assert_eq!(normalize_date_value(None), None);
        assert_eq!(normalize_date_value(Some(&Value::Null)), None);
    }

    #[test]
    fn normalize_amount_handles_strings_and_numbers() {
        assert_eq!(normalize_amount(Some(&json!(123.45))), Some(123.45));
        assert_eq!(normalize_amount(Some(&json!("1,234.5"))), Some(1234.5));
        assert_eq!(normalize_amount(Some(&json!("100万元"))), Some(100.0));
        assert_eq!(normalize_amount(Some(&json!("不是数字"))), None);
        assert_eq!(normalize_amount(None), None);
    }

    #[test]
    fn names_match_by_loose_pinyin_handles_homophones() {
        // identical
        assert!(names_match_by_loose_pinyin("张三", "张三"));
        // whitespace tolerance
        assert!(names_match_by_loose_pinyin(" 张 三 ", "张三"));
        // empty -> false
        assert!(!names_match_by_loose_pinyin("", "张三"));
        // different chars but matching loose pinyin: 王(wang) vs 汪(wang) both fold to "wan"
        assert!(names_match_by_loose_pinyin("王明", "汪明"));
        // unmistakably different
        assert!(!names_match_by_loose_pinyin("张三", "李四"));
        // no chinese -> only exact-string path
        assert!(!names_match_by_loose_pinyin("zhang san", "zhangsan"));
    }
}
