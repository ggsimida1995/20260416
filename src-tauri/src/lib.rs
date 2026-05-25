mod commands;
mod core;
mod db;
mod readers;
mod writers;

pub fn run() {
    tauri::Builder::default()
        .manage(crate::core::cancel::CancelFlag::new())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            commands::state::bootstrap,
            commands::state::save_settings,
            commands::state::open_path,
            commands::state::open_file_root,
            commands::state::choose_file_root,
            commands::state::check_session,
            commands::auth::open_login_window,
            commands::auth::close_login_window,
            commands::auth::has_stored_cookies,
            commands::auth::clear_login,
            commands::workflow::run_compare_only,
            commands::workflow::run_batch,
            commands::workflow::run_download_only,
            commands::workflow::cancel_workflow,
            commands::workflow::export_success_results,
            commands::workflow::export_error_results,
            commands::workflow::clear_runtime_logs
        ])
        .run(tauri::generate_context!())
        .expect("failed to run tauri app");
}
