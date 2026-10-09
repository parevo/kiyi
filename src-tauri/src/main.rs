#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod logging;
mod updater;

use std::sync::Arc;

use kiyi_core::workspace::Workspace;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let log = logging::init(&app.path().app_log_dir()?, &app.package_info().version.to_string());
            app.manage(logging::LogFile(log));
            let config_dir = app.path().app_config_dir()?;
            app.manage(Arc::new(Workspace::new(&config_dir)?));
            app.manage(updater::PendingUpdate::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_drivers,
            commands::list_connections,
            commands::parse_connection_url,
            commands::save_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::forget_host_key,
            commands::connect,
            commands::disconnect,
            commands::load_schema,
            commands::run_query,
            commands::cancel_query,
            commands::table_details,
            commands::browse_table,
            commands::count_rows,
            commands::plan_row_changes,
            commands::plan_replace,
            commands::summarize,
            commands::plan_table,
            commands::plan_table_action,
            commands::execute_script,
            commands::ai_status,
            commands::ai_presets,
            commands::ai_settings,
            commands::save_ai_provider,
            commands::delete_ai_provider,
            commands::set_active_ai,
            commands::ai_models,
            commands::discover_local,
            commands::ssh_config_hosts,
            commands::check_sql,
            commands::explain,
            commands::schema_graph,
            commands::backup,
            commands::backup_tools,
            commands::compare,
            commands::restore,
            commands::log_ui_error,
            commands::diagnostics,
            commands::export_table,
            commands::export_query,
            commands::csv_preview,
            commands::import_csv,
            commands::ai_filters,
            commands::ai_ask,
            commands::ai_write_sql,
            commands::ai_fix_sql,
            commands::ai_explain_sql,
            updater::check_update,
            updater::download_update,
            updater::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Kiyi");
}
