use crate::commands::state::{build_state, load_settings};
use crate::core::cancel::CancelFlag;
use crate::core::config::{ensure_workspace_layout, workspace_file_root, workspace_state_db_path};
use crate::core::download::{check_session_status, run_download, DownloadSummary};
use crate::core::models::WorkflowProgress;
use crate::core::workflow::run_compare_with_progress;
use crate::db::app_state::AppStateStore;
use crate::writers::success_excel::{
    export_error_records, export_pending_success_rows, SuccessExportResult,
};
use std::any::Any;
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
pub async fn run_compare_only(
    file_root: String,
    app: AppHandle,
) -> Result<crate::commands::state::AppState, String> {
    let cancel = app.state::<CancelFlag>().inner().clone();
    cancel.reset();
    run_blocking_command(move || run_compare_only_inner(file_root, app, cancel)).await
}

#[tauri::command]
pub async fn run_batch(
    file_root: String,
    app: AppHandle,
) -> Result<crate::commands::state::AppState, String> {
    let cancel = app.state::<CancelFlag>().inner().clone();
    cancel.reset();
    run_blocking_command(move || run_batch_inner(file_root, app, cancel)).await
}

#[tauri::command]
pub async fn run_download_only(
    file_root: String,
    app: AppHandle,
) -> Result<crate::commands::state::AppState, String> {
    let cancel = app.state::<CancelFlag>().inner().clone();
    cancel.reset();
    run_blocking_command(move || run_download_only_inner(file_root, cancel)).await
}

#[tauri::command]
pub fn cancel_workflow(cancel: State<'_, CancelFlag>) -> Result<(), String> {
    cancel.cancel();
    Ok(())
}

fn run_compare_only_inner(
    file_root: String,
    app: AppHandle,
    cancel: CancelFlag,
) -> Result<crate::commands::state::AppState, String> {
    let workspace_root = PathBuf::from(file_root);
    run_compare_with_progress(&workspace_root, &cancel, |progress| {
        emit_progress(&app, progress)
    })
    .map_err(to_string)?;
    build_state().map_err(to_string)
}

fn run_batch_inner(
    file_root: String,
    app: AppHandle,
    cancel: CancelFlag,
) -> Result<crate::commands::state::AppState, String> {
    let workspace_root = PathBuf::from(file_root);
    ensure_workspace_layout(&workspace_root).map_err(to_string)?;
    let store = AppStateStore::new(workspace_state_db_path(&workspace_root));
    store
        .append_runtime_log("[网页阶段] 开始")
        .map_err(to_string)?;
    let summary = run_download_guarded(&workspace_root, &store, &cancel)?;
    append_download_summary_logs(&store, &summary).map_err(to_string)?;
    if cancel.is_cancelled() {
        let _ = store.append_runtime_log("[本地比对阶段] 已取消");
        return Err("已取消".to_string());
    }
    store
        .append_runtime_log("[本地比对阶段] 开始")
        .map_err(to_string)?;
    if let Err(error) = run_compare_with_progress(&workspace_root, &cancel, |progress| {
        emit_progress(&app, progress)
    }) {
        let message = error.to_string();
        let _ = store.append_runtime_log(&format!("[本地比对阶段] 失败: {message}"));
        return Err(message);
    }
    build_state().map_err(to_string)
}

fn emit_progress(app: &AppHandle, progress: WorkflowProgress) {
    let _ = app.emit("workflow-progress", progress);
}

fn run_download_only_inner(
    file_root: String,
    cancel: CancelFlag,
) -> Result<crate::commands::state::AppState, String> {
    let workspace_root = PathBuf::from(file_root);
    ensure_workspace_layout(&workspace_root).map_err(to_string)?;
    let store = AppStateStore::new(workspace_state_db_path(&workspace_root));
    store
        .append_runtime_log("[网页阶段] 开始")
        .map_err(to_string)?;
    let summary = run_download_guarded(&workspace_root, &store, &cancel)?;
    append_download_summary_logs(&store, &summary).map_err(to_string)?;
    build_state().map_err(to_string)
}

#[tauri::command]
pub fn export_success_results(file_root: String) -> Result<SuccessExportResult, String> {
    export_pending_success_rows(&PathBuf::from(file_root)).map_err(to_string)
}

#[tauri::command]
pub fn export_error_results(file_root: String) -> Result<String, String> {
    export_error_records(&PathBuf::from(file_root))
        .map(|path| path.to_string_lossy().to_string())
        .map_err(to_string)
}

#[tauri::command]
pub fn clear_runtime_logs(file_root: String) -> Result<crate::commands::state::AppState, String> {
    let store = AppStateStore::new(workspace_state_db_path(&PathBuf::from(file_root)));
    store.clear_runtime_logs().map_err(to_string)?;
    build_state().map_err(to_string)
}

fn append_download_summary_logs(
    store: &AppStateStore,
    summary: &DownloadSummary,
) -> anyhow::Result<()> {
    store.append_runtime_log(&format!(
        "[网页阶段] 完成: 下载={} | 跳过={} | 错误={}",
        summary.processed_count,
        summary.skipped_projects.len(),
        summary.errors.len()
    ))?;
    for project_dir in &summary.saved_project_dirs {
        store.append_runtime_log(&format!("[网页阶段] 已下载: {project_dir}"))?;
    }
    for project_code in &summary.skipped_projects {
        store.append_runtime_log(&format!("[网页阶段] 已跳过: {project_code}"))?;
    }
    for error in &summary.errors {
        store.append_runtime_log(&format!("[网页阶段] 错误: {error}"))?;
    }
    if summary.processed_count == 0 && summary.skipped_projects.is_empty() && summary.errors.is_empty() {
        store.append_runtime_log("[网页阶段] 未发现待下载项目，请确认当前登录账号和待办列表")?;
    }
    Ok(())
}

fn run_download_guarded(
    workspace_root: &PathBuf,
    store: &AppStateStore,
    cancel: &CancelFlag,
) -> Result<DownloadSummary, String> {
    let settings = load_settings().map_err(to_string)?;
    store
        .append_runtime_log("[网页阶段] 初始化会话")
        .map_err(to_string)?;
    let session = check_session_status(&settings);
    if session.state != "ok" {
        store
            .append_runtime_log("[网页阶段] 未登录")
            .map_err(to_string)?;
        return Err("未登录".to_string());
    }
    let cancel_for_panic = cancel.clone();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let file_root = workspace_file_root(workspace_root);
        run_download(&file_root, &HashSet::new(), &settings, &cancel_for_panic)
    }));
    match result {
        Ok(Ok(summary)) => {
            if let Some(error) = summary.errors.first() {
                let message = format!("下载出现错误，已停止后续任务: {error}");
                store
                    .append_runtime_log(&format!("[网页阶段] 失败: {message}"))
                    .map_err(to_string)?;
                return Err(message);
            }
            Ok(summary)
        }
        Ok(Err(error)) => {
            let message = error.to_string();
            store
                .append_runtime_log(&format!("[网页阶段] 失败: {message}"))
                .map_err(to_string)?;
            Err(message)
        }
        Err(payload) => {
            let message = format!("下载阶段异常: {}", panic_payload_to_string(payload));
            store
                .append_runtime_log(&format!("[网页阶段] 失败: {message}"))
                .map_err(to_string)?;
            Err(message)
        }
    }
}

async fn run_blocking_command<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let result = tauri::async_runtime::spawn_blocking(move || catch_unwind(AssertUnwindSafe(task)))
        .await
        .map_err(to_string)?;
    match result {
        Ok(value) => value,
        Err(payload) => Err(format!(
            "后台任务异常: {}",
            panic_payload_to_string(payload)
        )),
    }
}

fn panic_payload_to_string(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "未知 panic".to_string()
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
