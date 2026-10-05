#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod updater;

use std::sync::Arc;

use kiyi_core::workspace::Workspace;
use tauri::Manager;

fn main() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
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
            commands::connect,
            commands::disconnect,
            commands::load_schema,
            commands::run_query,
            commands::cancel_query,
            commands::table_details,
            commands::browse_table,
            commands::count_rows,
            commands::plan_row_changes,
            commands::plan_table,
            commands::plan_table_action,
            commands::execute_script,
            updater::check_update,
            updater::download_update,
            updater::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Kıyı");
}
