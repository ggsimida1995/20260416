use crate::core::models::ProjectFiles;
use crate::core::normalizers::normalize_project_code;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn discover_projects(file_root: &Path) -> Result<Vec<PathBuf>> {
    if !file_root.exists() {
        return Ok(Vec::new());
    }
    let mut projects = std::fs::read_dir(file_root)?
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| path.is_dir() && is_project_dir_candidate(path))
        .collect::<Vec<_>>();
    projects.sort_by_key(|path| path.file_name().map(|name| name.to_os_string()));
    Ok(projects)
}

fn is_project_dir_candidate(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|item| item.to_string_lossy().to_string())
        .unwrap_or_default();
    !name.starts_with('.')
        && !matches!(
            name.as_str(),
            "cache"
                | "success"
                | "success_projects"
                | "export"
                | "debug"
                | "file"
                | "sql"
                | "project"
        )
}

pub fn discover_project_files(project_dir: &Path) -> Result<ProjectFiles> {
    let mut files = ProjectFiles {
        project_name: project_dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
        project_dir: project_dir.to_path_buf(),
        ..ProjectFiles::default()
    };

    let mut entries = std::fs::read_dir(project_dir)?
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| path.is_file() && !is_workspace_state_file(path))
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();

        if files.xlsx_path.is_none() && is_close_handover_workbook(&name) {
            files.xlsx_path = Some(path.clone());
        }
        if files.docx_path.is_none() && is_completion_summary_document(&name) {
            files.docx_path = Some(path.clone());
        }
        if files.pdf_path.is_none() && is_completion_acceptance_report(&name) {
            files.pdf_path = Some(path);
            continue;
        }
        if files.web_txt_path.is_none() && is_project_web_txt(&path, &files.project_name) {
            files.web_txt_path = Some(path);
        }
    }

    if files.xlsx_path.is_none() {
        files.missing_files.push("xlsx/xls".to_string());
    }
    if files.docx_path.is_none() {
        files.missing_files.push("docx/doc".to_string());
    }
    if files.pdf_path.is_none() {
        files.missing_files.push("pdf/jpg/jpeg/png".to_string());
    }
    Ok(files)
}

fn is_close_handover_workbook(name: &str) -> bool {
    name.contains("关闭移交登记表") && has_supported_extension(name, &["xls", "xlsx"])
}

fn is_completion_summary_document(name: &str) -> bool {
    name.contains("竣工总结报告") && has_supported_extension(name, &["doc", "docx"])
}

fn is_completion_acceptance_report(name: &str) -> bool {
    name.contains("竣工验收报告") && has_supported_extension(name, &["pdf", "jpg", "jpeg", "png"])
}

fn has_supported_extension(name: &str, extensions: &[&str]) -> bool {
    let lower = name.to_lowercase();
    extensions
        .iter()
        .any(|extension| lower.ends_with(&format!(".{extension}")))
}

fn is_project_web_txt(path: &Path, project_name: &str) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !name.to_lowercase().ends_with(".txt") {
        return false;
    }
    if name.starts_with(project_name) {
        return true;
    }
    let expected = normalize_project_code(project_name).replace('_', "-");
    let by_name = normalize_project_code(name.trim_end_matches(".txt")).replace('_', "-");
    if !expected.is_empty() && by_name == expected {
        return true;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Some(code) = extract_txt_project_code(&content) else {
        return false;
    };
    normalize_project_code(&code).replace('_', "-") == expected
}

fn extract_txt_project_code(content: &str) -> Option<String> {
    for line in content.lines() {
        let normalized = line.trim();
        for label in ["项目编号:", "项目编号："] {
            if let Some(value) = normalized.strip_prefix(label) {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn is_workspace_state_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "project_compare_state.sqlite3")
}

pub fn project_dir_names(file_root: &Path) -> Vec<String> {
    discover_projects(file_root)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{discover_project_files, discover_projects};
    use std::fs;

    #[test]
    fn discovers_files_by_business_keywords_not_extension_only() {
        let dir = std::env::temp_dir().join(format!(
            "project-file-discovery-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        for name in [
            "无关数据.xlsx",
            "客户A关闭移交登记表.xls",
            "其他报告.docx",
            "客户A竣工总结报告.doc",
            "扫描件.png",
            "客户A竣工验收报告.png",
        ] {
            fs::write(dir.join(name), "").unwrap();
        }

        let files = discover_project_files(&dir).unwrap();
        assert_eq!(
            files
                .xlsx_path
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "客户A关闭移交登记表.xls"
        );
        assert_eq!(
            files
                .docx_path
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "客户A竣工总结报告.doc"
        );
        assert_eq!(
            files
                .pdf_path
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "客户A竣工验收报告.png"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_workspace_container_directories() {
        let dir = std::env::temp_dir().join(format!(
            "project-container-ignore-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        for name in ["file", "sql", "project", "BHE-25090004-01"] {
            fs::create_dir_all(dir.join(name)).unwrap();
        }

        let names = discover_projects(&dir)
            .unwrap()
            .into_iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["BHE-25090004-01"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovers_web_txt_by_normalized_project_code() {
        let dir = std::env::temp_dir().join(format!(
            "project-web-txt-discovery-test-{}",
            std::process::id()
        ));
        let project = dir.join("PHE-25080042_B1");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("PHE-25080042-B1.txt"),
            "项目编号: PHE-25080042-B1\n项目名称: 测试项目\n",
        )
        .unwrap();

        let files = discover_project_files(&project).unwrap();

        assert_eq!(
            files
                .web_txt_path
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "PHE-25080042-B1.txt"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
