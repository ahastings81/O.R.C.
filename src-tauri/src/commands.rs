use std::sync::Mutex;

use tauri::{AppHandle, State};

use crate::{
    app_state::{AppError, ProxyTerminalState},
    models::{AgentMemoryMode, ApprovalMode, DashboardState, DelegationMode, SessionPolicy, TaskGuardrails, TerminalControl, WorkerStatus},
};

type ManagedState<'a> = State<'a, Mutex<ProxyTerminalState>>;

fn with_state<T>(
    state: ManagedState<'_>,
    handler: impl FnOnce(&mut ProxyTerminalState) -> Result<T, AppError>,
) -> Result<T, String> {
    let mut state = state
        .lock()
        .map_err(|_| "application state lock was poisoned".to_string())?;
    handler(&mut state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn bootstrap_state(app: AppHandle, state: ManagedState<'_>) -> Result<DashboardState, String> {
    with_state(state, |state| state.bootstrap(&app))
}

#[tauri::command]
pub fn create_command_session(
    app: AppHandle,
    state: ManagedState<'_>,
    title: Option<String>,
) -> Result<DashboardState, String> {
    with_state(state, |state| state.create_command_session(&app, title))
}

#[tauri::command]
pub fn send_terminal_input(
    app: AppHandle,
    state: ManagedState<'_>,
    session_id: String,
    input: String,
) -> Result<DashboardState, String> {
    with_state(state, |state| {
        state.send_terminal_input(&app, &session_id, input)
    })
}

#[tauri::command]
pub fn resize_terminal(
    state: ManagedState<'_>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    with_state(state, |state| {
        state.resize_terminal(&session_id, cols, rows)
    })
}

#[tauri::command]
pub fn send_terminal_control(
    app: AppHandle,
    state: ManagedState<'_>,
    session_id: String,
    control: TerminalControl,
) -> Result<DashboardState, String> {
    with_state(state, |state| {
        state.send_terminal_control(&app, &session_id, control)
    })
}

#[tauri::command]
pub fn restart_terminal_session(
    app: AppHandle,
    state: ManagedState<'_>,
    session_id: String,
) -> Result<DashboardState, String> {
    with_state(state, |state| {
        state.restart_terminal_session(&app, &session_id)
    })
}

#[tauri::command]
pub fn approve_request(
    app: AppHandle,
    state: ManagedState<'_>,
    request_id: String,
    mode: ApprovalMode,
) -> Result<DashboardState, String> {
    with_state(state, |state| {
        state.approve_request(&app, &request_id, mode)
    })
}

#[tauri::command]
pub fn deny_request(
    app: AppHandle,
    state: ManagedState<'_>,
    request_id: String,
) -> Result<DashboardState, String> {
    with_state(state, |state| state.deny_request(&app, &request_id))
}

#[tauri::command]
pub fn deny_request_and_stop(
    app: AppHandle,
    state: ManagedState<'_>,
    request_id: String,
) -> Result<DashboardState, String> {
    with_state(state, |state| state.deny_request_and_stop(&app, &request_id))
}

#[tauri::command]
pub fn export_audit_log(state: ManagedState<'_>) -> Result<String, String> {
    with_state(state, |state| state.export_audit_log())
}

#[tauri::command]
pub fn update_policy(
    state: ManagedState<'_>,
    policy: SessionPolicy,
) -> Result<DashboardState, String> {
    with_state(state, |state| Ok(state.update_policy(policy)))
}

#[tauri::command]
pub fn create_worker(
    state: ManagedState<'_>,
    adapter: String,
    name: String,
    executable_path: Option<String>,
    args: Option<Vec<String>>,
    memory_mode: Option<AgentMemoryMode>,
    profile_id: Option<String>,
) -> Result<DashboardState, String> {
    with_state(state, |state| {
        Ok(state.create_worker(
            adapter,
            name,
            executable_path,
            args.unwrap_or_default(),
            memory_mode.unwrap_or_else(|| state.policy.default_memory_mode.clone()),
            profile_id,
        ))
    })
}

#[tauri::command]
pub fn update_worker(
    state: ManagedState<'_>,
    worker_id: String,
    name: String,
    executable_path: Option<String>,
    args: Option<Vec<String>>,
    memory_mode: AgentMemoryMode,
    profile_id: Option<String>,
) -> Result<DashboardState, String> {
    with_state(state, |state| {
        state.update_worker(
            &worker_id,
            name,
            executable_path,
            args.unwrap_or_default(),
            memory_mode,
            profile_id,
        )
    })
}

#[tauri::command]
pub fn delete_worker(state: ManagedState<'_>, worker_id: String) -> Result<DashboardState, String> {
    with_state(state, |state| state.delete_worker(&worker_id))
}

#[tauri::command]
pub fn save_agent_profile(
    state: ManagedState<'_>,
    name: String,
    allow_commands: Vec<String>,
    allow_domains: Vec<String>,
    memory_mode: AgentMemoryMode,
    delegation_mode: DelegationMode,
    delegation_max_depth: u8,
    default_guardrails: TaskGuardrails,
) -> Result<DashboardState, String> {
    with_state(state, |state| {
        Ok(state.save_agent_profile(
            name,
            allow_commands,
            allow_domains,
            memory_mode,
            delegation_mode,
            delegation_max_depth,
            default_guardrails,
        ))
    })
}

#[tauri::command]
pub fn apply_agent_profile(
    state: ManagedState<'_>,
    worker_id: String,
    profile_id: String,
) -> Result<DashboardState, String> {
    with_state(state, |state| state.apply_agent_profile(&worker_id, &profile_id))
}

#[tauri::command]
pub fn assign_task(
    app: AppHandle,
    state: ManagedState<'_>,
    worker_id: String,
    title: String,
    summary: String,
    guardrails: TaskGuardrails,
) -> Result<DashboardState, String> {
    with_state(state, |state| {
        state.assign_task(&app, &worker_id, title, summary, guardrails)
    })
}

#[tauri::command]
pub fn delete_task(state: ManagedState<'_>, task_id: String) -> Result<DashboardState, String> {
    with_state(state, |state| state.delete_task(&task_id))
}

#[tauri::command]
pub fn set_worker_status(
    app: AppHandle,
    state: ManagedState<'_>,
    worker_id: String,
    status: WorkerStatus,
) -> Result<DashboardState, String> {
    with_state(state, |state| {
        state.set_worker_status(&app, &worker_id, status)
    })
}
