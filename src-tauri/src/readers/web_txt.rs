use crate::core::models::WebData;
use crate::core::normalizers::{normalize_project_code, normalize_text};
use anyhow::{Context, Result};
use std::path::Path;

pub fn read_web_txt(path: &Path) -> Result<WebData> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取网页详情文件: {}", path.display()))?;
    Ok(parse_web_txt(&content))
}

fn parse_web_txt(content: &str) -> WebData {
    WebData {
        project_code: normalize_project_code(&extract_value(content, "项目编号")),
        project_name: normalize_text(&extract_value(content, "项目名称")),
    }
}

fn extract_value(content: &str, label: &str) -> String {
    for line in content.lines() {
        let normalized = normalize_text(line);
        if let Some(value) = normalized.strip_prefix(&format!("{label}:")) {
            return normalize_text(value);
        }
        if let Some(value) = normalized.strip_prefix(&format!("{label}：")) {
            return normalize_text(value);
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::parse_web_txt;

    #[test]
    fn parses_downloaded_web_txt() {
        let data = parse_web_txt(
            "项目编号: BHE-25060338-L1\n项目名称: 某某项目名称\n来源分类: 待办\n详情页: https://example.com\n",
        );
        assert_eq!(data.project_code, "BHE-25060338-L1");
        assert_eq!(data.project_name, "某某项目名称");
    }
}
