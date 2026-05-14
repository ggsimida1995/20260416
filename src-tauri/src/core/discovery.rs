use crate::core::models::ProjectFiles;
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
    !name.starts_with('.') && !matches!(name.as_str(), "cache" | "success" | "export" | "debug")
}

pub fn discover_project_files(project_dir: &Path) -> Result<ProjectFiles> {
    let mut files = ProjectFiles {
        project_name: project_dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
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
        let lower = name.to_lowercase();

        if files.xlsx_path.is_none()
            && (name.contains("项目关闭移交登记表") || name.contains("关闭移交登记表"))
            && (lower.ends_with(".xlsx") || lower.ends_with(".xls"))
        {
            files.xlsx_path = Some(path.clone());
        }
        if files.docx_path.is_none()
            && name.contains("项目竣工总结报告")
            && (lower.ends_with(".docx") || lower.ends_with(".doc"))
        {
            files.docx_path = Some(path.clone());
        }
        if files.pdf_path.is_none()
            && (name.contains("PA竣工验收报告")
                || name.contains("竣工验收报告")
                || name.contains("验收报告"))
            && lower.ends_with(".pdf")
        {
            files.pdf_path = Some(path);
            continue;
        }
        if files.web_txt_path.is_none()
            && lower.ends_with(".txt")
            && name.starts_with(&files.project_name)
        {
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
        files.missing_files.push("pdf".to_string());
    }
    Ok(files)
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
