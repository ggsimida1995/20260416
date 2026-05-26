use crate::core::models::DocxData;
use crate::core::normalizers::{
    normalize_date, normalize_phone, normalize_project_code, normalize_text,
};
use anyhow::{anyhow, Context, Result};
use encoding_rs::GB18030;
use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use zip::ZipArchive;

pub fn read_docx(path: &Path) -> Result<DocxData> {
    let text = if path
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .eq_ignore_ascii_case("doc")
    {
        read_doc_text(path)?
    } else {
        read_docx_text(path)?
    };
    Ok(parse_docx_text(&text))
}

fn read_docx_text(path: &Path) -> Result<String> {
    let file =
        std::fs::File::open(path).with_context(|| format!("无法打开 docx: {}", path.display()))?;
    let mut archive = ZipArchive::new(file)?;
    let mut document = String::new();
    archive
        .by_name("word/document.xml")?
        .read_to_string(&mut document)?;

    let mut reader = Reader::from_str(&document);
    reader.config_mut().trim_text(true);
    let mut parts = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(event)) => {
                let text = event.decode()?.to_string();
                let normalized = normalize_text(&text);
                if !normalized.is_empty() {
                    parts.push(normalized);
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(error.into()),
            _ => {}
        }
    }
    Ok(parts.join(" "))
}

fn read_doc_text(path: &Path) -> Result<String> {
    for reader in [
        read_doc_text_plain as fn(&Path) -> Option<String>,
        read_doc_text_from_binary_strings,
        read_doc_text_with_textutil,
        read_doc_text_with_antiword,
        read_doc_text_with_soffice,
    ] {
        if let Some(text) = reader(path) {
            if !text.is_empty() {
                return Ok(text);
            }
        }
    }
    Err(anyhow!(
        "无法读取 .doc 文件，请安装 Microsoft Word/LibreOffice 或转换为 .docx: {}",
        path.display()
    ))
}

fn read_doc_text_plain(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if let Ok(text) = String::from_utf8(bytes) {
        let normalized = normalize_text(&strip_rtf_markup(&text));
        if looks_like_doc_text(&normalized) {
            return Some(normalized);
        }
    }
    None
}

fn read_doc_text_from_binary_strings(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    for text in [
        extract_utf16le_text_runs(&bytes),
        extract_gb18030_text_runs(&bytes),
    ] {
        let normalized = normalize_text(&strip_rtf_markup(&text));
        if looks_like_doc_text(&normalized) {
            return Some(normalized);
        }
    }
    None
}

fn read_doc_text_with_textutil(path: &Path) -> Option<String> {
    let output = Command::new("textutil")
        .args(["-convert", "txt", "-stdout"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(normalize_text(&String::from_utf8_lossy(&output.stdout)))
}

fn read_doc_text_with_antiword(path: &Path) -> Option<String> {
    let output = Command::new("antiword").arg(path).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(normalize_text(&String::from_utf8_lossy(&output.stdout)))
}

fn read_doc_text_with_soffice(path: &Path) -> Option<String> {
    let temp_dir = std::env::temp_dir().join(format!(
        "project-file-compare-doc-{}",
        chrono::Local::now().timestamp_nanos_opt()?
    ));
    std::fs::create_dir_all(&temp_dir).ok()?;
    let output = Command::new("soffice")
        .args(["--headless", "--convert-to", "txt:Text", "--outdir"])
        .arg(&temp_dir)
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return None;
    }
    let txt_path = temp_dir.join(format!("{}.txt", path.file_stem()?.to_string_lossy()));
    let text = std::fs::read_to_string(txt_path)
        .ok()
        .map(|item| normalize_text(&item));
    let _ = std::fs::remove_dir_all(&temp_dir);
    text
}

fn parse_docx_text(text: &str) -> DocxData {
    let cleaned = normalize_text(text);
    let contact_names = extract_values(
        &cleaned,
        "用户姓名",
        &["用户姓名", "用户职务", "联系电话", "电子邮件"],
    );
    let contact_phones = extract_values(
        &cleaned,
        "联系电话",
        &[
            "联系电话",
            "电子邮件",
            "项目经理",
            "所属部门",
            "竣工验收",
            "用户姓名",
        ],
    )
    .into_iter()
    .filter_map(|value| {
        let digits = normalize_phone(&value);
        Regex::new(r"1\d{10}")
            .ok()
            .and_then(|re| re.find(&digits).map(|item| item.as_str().to_string()))
    })
    .collect::<Vec<_>>();

    let (acceptance_start, acceptance_end) = extract_acceptance_range(&cleaned);
    DocxData {
        project_code: normalize_project_code(&extract_value(
            &cleaned,
            "项目编号",
            &["报告日期", "项目全称"],
        )),
        project_name: extract_value(&cleaned, "项目全称", &["项目类型", "项目关注", "用户姓名"]),
        contact_names,
        contact_phones,
        acceptance_start,
        acceptance_end,
        has_invalid_acceptance_range: acceptance_start
            .zip(acceptance_end)
            .map(|(start, end)| start > end)
            .unwrap_or(false),
    }
}

fn extract_value(text: &str, label: &str, stop_labels: &[&str]) -> String {
    extract_values(text, label, stop_labels)
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn extract_values(text: &str, label: &str, stop_labels: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    let mut search_from = 0;
    while let Some(relative_start) = text[search_from..].find(label) {
        let value_start = search_from + relative_start + label.len();
        let mut tail = &text[value_start..];
        if let Some(stop) = stop_labels
            .iter()
            .filter_map(|label| tail.find(label))
            .min()
        {
            tail = &tail[..stop];
        }
        let value = normalize_text(tail)
            .trim_start_matches(['：', ':'])
            .to_string();
        if !value.is_empty() && !values.contains(&value) {
            values.push(value);
        }
        search_from = value_start;
    }
    values
}

fn extract_acceptance_range(text: &str) -> (Option<chrono::NaiveDate>, Option<chrono::NaiveDate>) {
    let explicit_start = extract_date_after_any_label(
        text,
        &[
            "竣工验收(开始时间)",
            "竣工验收（开始时间）",
            "竣工验收开始时间",
        ],
    );
    let explicit_end = extract_date_after_any_label(
        text,
        &[
            "竣工验收(完成时间)",
            "竣工验收（完成时间）",
            "竣工验收完成时间",
        ],
    );
    if explicit_start.is_some() || explicit_end.is_some() {
        return (explicit_start, explicit_end);
    }

    let Some(start) = text.find("竣工验收") else {
        return (None, None);
    };
    let tail = text[start + "竣工验收".len()..]
        .chars()
        .take(120)
        .collect::<String>();
    let Some(pattern) = date_pattern() else {
        return (None, None);
    };
    let matches = pattern
        .find_iter(&tail)
        .map(|item| item.as_str().to_string())
        .collect::<Vec<_>>();
    if matches.len() < 2 {
        return (None, None);
    }
    (normalize_date(&matches[0]), normalize_date(&matches[1]))
}

fn extract_date_after_any_label(text: &str, labels: &[&str]) -> Option<chrono::NaiveDate> {
    for label in labels {
        if let Some(date) = extract_date_after_label(text, label) {
            return Some(date);
        }
    }

    let compact_text = text.replace(' ', "");
    for label in labels {
        let compact_label = label.replace(' ', "");
        if let Some(date) = extract_date_after_label(&compact_text, &compact_label) {
            return Some(date);
        }
    }
    None
}

fn extract_date_after_label(text: &str, label: &str) -> Option<chrono::NaiveDate> {
    let label_start = text.find(label)?;
    let tail = text[label_start + label.len()..]
        .chars()
        .take(80)
        .collect::<String>();
    date_pattern()?
        .find(&tail)
        .and_then(|item| normalize_date(item.as_str()))
}

fn date_pattern() -> Option<Regex> {
    Regex::new(
        r"(?:\d\s*){4}年\s*(?:\d\s*){1,2}月\s*(?:\d\s*){1,2}日|(?:\d\s*){4}\s*[-/.]\s*(?:\d\s*){1,2}\s*[-/.]\s*(?:\d\s*){1,2}",
    )
    .ok()
}

fn strip_rtf_markup(text: &str) -> String {
    let text = Regex::new(r"\\'[0-9a-fA-F]{2}")
        .map(|re| re.replace_all(text, " ").to_string())
        .unwrap_or_else(|_| text.to_string());
    let text = Regex::new(r"\\[a-zA-Z]+\d* ?")
        .map(|re| re.replace_all(&text, " ").to_string())
        .unwrap_or(text);
    text.replace(['{', '}'], " ")
}

fn looks_like_doc_text(text: &str) -> bool {
    ["项目编号", "项目全称", "用户姓名", "联系电话", "竣工验收"]
        .iter()
        .any(|label| text.contains(label))
}

fn extract_utf16le_text_runs(bytes: &[u8]) -> String {
    let mut best = String::new();
    for offset in [0usize, 1] {
        let mut chars = Vec::new();
        let mut index = offset;
        while index + 1 < bytes.len() {
            let value = u16::from_le_bytes([bytes[index], bytes[index + 1]]);
            let ch = char::from_u32(value as u32).unwrap_or('\0');
            chars.push(ch);
            index += 2;
        }
        let text = collect_text_runs(chars.into_iter());
        let score = score_doc_text(&text);
        let best_score = score_doc_text(&best);
        if score > best_score || score == best_score && text.len() > best.len() {
            best = text;
        }
    }
    best
}

fn extract_gb18030_text_runs(bytes: &[u8]) -> String {
    let (text, _, _) = GB18030.decode(bytes);
    collect_text_runs(text.chars())
}

fn collect_text_runs(chars: impl Iterator<Item = char>) -> String {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in chars {
        if is_doc_text_char(ch) {
            current.push(if ch.is_whitespace() { ' ' } else { ch });
        } else {
            push_doc_text_run(&mut parts, &mut current);
        }
    }
    push_doc_text_run(&mut parts, &mut current);
    parts.join(" ")
}

fn push_doc_text_run(parts: &mut Vec<String>, current: &mut String) {
    let run = normalize_text(current);
    current.clear();
    let useful_chars = run
        .chars()
        .filter(|ch| ch.is_alphanumeric() || is_cjk(*ch))
        .count();
    if useful_chars >= 2 {
        parts.push(run);
    }
}

fn is_doc_text_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || ch.is_ascii_punctuation()
        || ch.is_whitespace()
        || is_cjk(ch)
        || matches!(ch, '，' | '。' | '：' | '；' | '（' | '）' | '、' | '—')
}

fn is_cjk(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
}

fn score_doc_text(text: &str) -> usize {
    ["项目编号", "项目全称", "用户姓名", "联系电话", "竣工验收"]
        .iter()
        .filter(|label| text.contains(**label))
        .count()
}

#[cfg(test)]
mod tests {
    use super::{extract_utf16le_text_runs, parse_docx_text};
    use chrono::NaiveDate;

    #[test]
    fn parses_required_summary_fields() {
        let data = parse_docx_text(
            "项目编号： PHE-25080042/B1 报告日期：2026年05月06日 \
             一、项目信息 项目全称 阜阳市淮河能源谢桥发电厂智慧控制系统工程项目 项目类型 \
             用户姓名 张凡 用户职务 项目经理 联系电话 15720040243 电子邮件 \
             三、项目总结 任务名称 开始时间 完成时间 参与人 \
             竣工验收 202 6 年0 4 月 19 日 202 6 年0 4 月 22 日 张凡",
        );

        assert_eq!(data.project_code, "PHE-25080042/B1");
        assert_eq!(
            data.project_name,
            "阜阳市淮河能源谢桥发电厂智慧控制系统工程项目"
        );
        assert_eq!(data.contact_names, vec!["张凡"]);
        assert_eq!(data.contact_phones, vec!["15720040243"]);
        assert_eq!(data.acceptance_start, NaiveDate::from_ymd_opt(2026, 4, 19));
        assert_eq!(data.acceptance_end, NaiveDate::from_ymd_opt(2026, 4, 22));
    }

    #[test]
    fn parses_explicit_acceptance_labels() {
        let data = parse_docx_text(
            "项目编号 BHE-25060213/01 项目全称 绵竹四川龙佰钛业DCS项目合同 项目类型 \
             用户姓名 李学明 用户职务 项目经理 联系电话 13866533365 电子邮件 \
             竣工验收(完成时间) 2026.04.10 竣工验收(开始时间) 2026.03.10",
        );

        assert_eq!(data.acceptance_start, NaiveDate::from_ymd_opt(2026, 3, 10));
        assert_eq!(data.acceptance_end, NaiveDate::from_ymd_opt(2026, 4, 10));
    }

    #[test]
    fn parses_split_hyphenated_acceptance_dates() {
        let data = parse_docx_text(
            "任务名称 开始时间 完成时间 参与人 预算工时 实际工时 \
             竣工验收 20 26 - 4 - 1 6 20 26 - 4 - 16 赵洋",
        );

        assert_eq!(data.acceptance_start, NaiveDate::from_ymd_opt(2026, 4, 16));
        assert_eq!(data.acceptance_end, NaiveDate::from_ymd_opt(2026, 4, 16));
    }

    #[test]
    fn extracts_utf16le_text_runs_from_binary_doc_fallback() {
        let text = "项目编号：BHE-24060165/01 项目全称 测试项目 用户姓名 张三 联系电话 13800000000 竣工验收 2026年01月02日 2026年01月05日";
        let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0, 1, 2, 3];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&[4, 5, 6, 7]);

        let extracted = extract_utf16le_text_runs(&bytes);
        assert!(extracted.contains("项目编号"));
        assert!(extracted.contains("BHE-24060165/01"));
        assert!(extracted.contains("联系电话"));
    }
}
