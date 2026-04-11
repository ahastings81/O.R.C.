#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod app_state;
mod audit;
mod commands;
mod models;
mod policy;
mod security;

use std::sync::Mutex;

use app_state::ProxyTerminalState;

fn main() {
    tauri::Builder::default()
        .manage(Mutex::new(ProxyTerminalState::new()))
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap_state,
            commands::get_dashboard_state,
            commands::create_command_session,
            commands::send_terminal_input,
            commands::resize_terminal,
            commands::send_terminal_control,
            commands::restart_terminal_session,
            commands::approve_request,
            commands::deny_request,
            commands::deny_request_and_stop,
            commands::export_audit_log,
            commands::update_policy,
            commands::create_worker,
            commands::update_worker,
            commands::delete_worker,
            commands::save_agent_profile,
            commands::apply_agent_profile,
            commands::assign_task,
            commands::delete_task,
            commands::set_worker_status
        ])
        .run(tauri::generate_context!())
        .expect("failed to run proxy terminal");
}
