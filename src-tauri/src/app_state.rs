use std::{
    collections::HashMap,
    env,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::Utc;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tauri::{AppHandle, Manager};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    agent,
    audit::audit_event,
    models::{
        ActionKind, ActionRequest, AgentCapabilitySetting, AgentMemoryMode, AgentProfile,
        ApprovalGrant, ApprovalMode, CommandSession, DashboardState, DelegationMode, FileRule,
        McpToolRule, PendingApproval, PolicyDecision, PolicyVerdict, SessionPolicy,
        ProtectionStatus, SupervisorTask, TaskGuardrails, TerminalControl, TerminalOutputEvent, Worker,
        WorkerOutputEvent, WorkerStatus,
    },
    policy::evaluate_request,
    security,
};

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
}

struct TerminalRuntime {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    _child: Box<dyn portable_pty::Child + Send>,
}

struct WorkerRuntime {
    stdin: Arc<Mutex<ChildStdin>>,
    child: Child,
    _os_enforcement: security::WorkerOsEnforcement,
}

#[derive(Debug, Clone)]
struct PendingWorkerCommandExecution {
    request_id: String,
    worker_id: String,
    command: String,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    result_path: PathBuf,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerCommandResultFile {
    request_id: String,
    exit_code: i32,
}

#[derive(Debug, Clone)]
struct WorkerEnvelopeCommand {
    command: String,
    cwd: Option<String>,
}

enum ParsedWorkerEnvelope {
    Command(WorkerEnvelopeCommand),
    Rejected(String),
}

pub struct ProxyTerminalState {
    pub policy: SessionPolicy,
    pub protections: Vec<ProtectionStatus>,
    pub audit: Vec<crate::models::AuditEvent>,
    pub pending_approvals: Vec<PendingApproval>,
    pub grants: Vec<ApprovalGrant>,
    pub sessions: Vec<CommandSession>,
    pub profiles: Vec<AgentProfile>,
    pub workers: Vec<Worker>,
    pub tasks: Vec<SupervisorTask>,
    terminal_runtimes: HashMap<String, TerminalRuntime>,
    worker_runtimes: HashMap<String, WorkerRuntime>,
    worker_terminal_sessions: HashMap<String, String>,
}

impl ProxyTerminalState {
    pub fn new() -> Self {
        let cwd = default_policy_root();

        let policy = SessionPolicy {
            name: "Default local policy".into(),
            roots: vec![cwd.clone()],
            file_rules: vec![FileRule {
                root: cwd.clone(),
                access: crate::models::AccessLevel::Manage,
            }],
            allow_commands: vec![
                "dir".into(),
                "cd".into(),
                "echo".into(),
                "pwd".into(),
                "get-childitem".into(),
                "get-location".into(),
                "get-content".into(),
                "select-string".into(),
                "test-path".into(),
                "type".into(),
                "node".into(),
                "npm".into(),
                "openclaw".into(),
                "powershell".into(),
                "y".into(),
                "n".into(),
                "yes".into(),
                "no".into(),
            ],
            allow_apps: vec!["code".into()],
            allow_domains: vec!["localhost".into(), "openclaw.ai".into()],
            mcp: vec![McpToolRule {
                server: "local://filesystem".into(),
                tools: vec!["read".into(), "list".into()],
            }],
            elevated_commands: vec!["type".into()],
            audit_redactions: vec!["OPENAI_API_KEY".into()],
            default_memory_mode: AgentMemoryMode::Ephemeral,
            delegation_mode: DelegationMode::Deny,
            delegation_max_depth: 0,
        };

        let profiles = default_agent_profiles();

        let mut initial_worker = agent::create_worker(
            "Default agent".into(),
            "openclaw".into(),
            cwd,
            None,
            Vec::new(),
            policy.default_memory_mode.clone(),
        );
        apply_profile_to_worker(&mut initial_worker, "profile-strict-broker", &profiles);

        let mut state = Self {
            policy,
            profiles,
            protections: security::detect_host_protections(),
            audit: Vec::new(),
            pending_approvals: Vec::new(),
            grants: Vec::new(),
            sessions: Vec::new(),
            workers: vec![initial_worker],
            tasks: Vec::new(),
            terminal_runtimes: HashMap::new(),
            worker_runtimes: HashMap::new(),
            worker_terminal_sessions: HashMap::new(),
        };

        state.audit.push(audit_event(
            "session",
            "system",
            Some("ready".into()),
            "O.R.C. Terminal booted with default local policy.",
            None,
            None,
        ));

        state
    }

    pub fn bootstrap(&mut self, app: &AppHandle) -> Result<DashboardState, AppError> {
        self.harden_openclaw_config(app, None, "app bootstrap");

        if self.sessions.is_empty() {
            self.create_command_session(app, Some("Session 1".into()))?;
        }

        Ok(self.snapshot())
    }

    pub fn snapshot(&self) -> DashboardState {
        DashboardState {
            policy: self.policy.clone(),
            profiles: self.profiles.clone(),
            protections: self.protections.clone(),
            audit: self.audit.iter().rev().cloned().collect(),
            pending_approvals: self.pending_approvals.clone(),
            sessions: self.sessions.clone(),
            workers: self.workers.clone(),
            tasks: self.tasks.clone(),
        }
    }

    pub fn create_command_session(
        &mut self,
        app: &AppHandle,
        title: Option<String>,
    ) -> Result<DashboardState, AppError> {
        let cwd = self
            .policy
            .roots
            .first()
            .cloned()
            .unwrap_or_else(|| "C:\\".into());
        let session_id = Uuid::new_v4().to_string();
        let (runtime, session) =
            spawn_terminal_runtime(app.clone(), session_id.clone(), title, cwd)?;

        self.audit.push(audit_event(
            "session",
            "human_terminal",
            Some("created".into()),
            format!("Created PTY command session `{}`.", session.title),
            None,
            None,
        ));
        self.terminal_runtimes.insert(session_id, runtime);
        self.sessions.push(session);
        Ok(self.snapshot())
    }

    pub fn send_terminal_input(
        &mut self,
        app: &AppHandle,
        session_id: &str,
        input: String,
    ) -> Result<DashboardState, AppError> {
        let request = ActionRequest {
            id: Uuid::new_v4().to_string(),
            kind: ActionKind::Command,
            target: input.clone(),
            command: Some(input.clone()),
            cwd: self
                .sessions
                .iter()
                .find(|session| session.id == session_id)
                .map(|session| session.cwd.clone()),
            args: None,
            rationale: Some("Interactive PTY input".into()),
            worker_id: None,
            session_id: Some(session_id.to_string()),
        };

        let decision = evaluate_request(&self.policy, &request, &self.grants);
        self.audit.push(audit_event(
            "policy",
            "human_terminal",
            Some(policy_outcome_value(&decision).into()),
            describe_policy_outcome(&request.target, &decision),
            Some(request.id.clone()),
            None,
        ));

        match decision.verdict {
            PolicyVerdict::Allow => {
                self.write_to_terminal(session_id, &input)?;
                self.audit.push(audit_event(
                    "command",
                    "human_terminal",
                    Some("sent".into()),
                    format!("Sent PTY input `{input}`."),
                    Some(request.id),
                    None,
                ));
            }
            PolicyVerdict::Prompt => {
                self.pending_approvals
                    .push(PendingApproval { request, decision });
                emit_terminal_output(
                    app,
                    session_id,
                    "\r\n[proxy] input blocked pending approval\r\n",
                );
            }
            PolicyVerdict::Deny => {
                emit_terminal_output(app, session_id, "\r\n[proxy] input denied\r\n");
                self.audit.push(audit_event(
                    "command",
                    "human_terminal",
                    Some("denied".into()),
                    "PTY input denied.",
                    None,
                    None,
                ));
            }
        }

        Ok(self.snapshot())
    }

    pub fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), AppError> {
        if let Some(runtime) = self.terminal_runtimes.get_mut(session_id) {
            runtime
                .master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|error| AppError::Message(format!("Failed to resize PTY: {error}")))?;
        }

        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.cols = cols;
            session.rows = rows;
        }

        Ok(())
    }

    pub fn send_terminal_control(
        &mut self,
        app: &AppHandle,
        session_id: &str,
        control: TerminalControl,
    ) -> Result<DashboardState, AppError> {
        let (bytes, label, message) = match control {
            TerminalControl::CtrlC => (vec![0x03], "Ctrl+C", "\r\n[proxy] sent Ctrl+C\r\n"),
            TerminalControl::CtrlD => (vec![0x04], "Ctrl+D", "\r\n[proxy] sent Ctrl+D\r\n"),
            TerminalControl::ClearLine => (
                vec![0x15],
                "Clear line",
                "\r\n[proxy] sent line-clear control\r\n",
            ),
            TerminalControl::Space => (vec![0x20], "Space", "\r\n[proxy] sent Space\r\n"),
            TerminalControl::ArrowUp => (b"\x1b[A".to_vec(), "Arrow Up", "\r\n[proxy] sent Arrow Up\r\n"),
            TerminalControl::ArrowDown => (
                b"\x1b[B".to_vec(),
                "Arrow Down",
                "\r\n[proxy] sent Arrow Down\r\n",
            ),
            TerminalControl::PageUp => (b"\x1b[5~".to_vec(), "Page Up", "\r\n[proxy] sent Page Up\r\n"),
            TerminalControl::PageDown => (
                b"\x1b[6~".to_vec(),
                "Page Down",
                "\r\n[proxy] sent Page Down\r\n",
            ),
            TerminalControl::Enter => (vec![0x0d], "Enter", "\r\n[proxy] sent Enter\r\n"),
        };

        self.write_raw_to_terminal(session_id, &bytes)?;
        emit_terminal_output(app, session_id, message);
        self.audit.push(audit_event(
            "command",
            "human_terminal",
            Some("control".into()),
            format!("Sent terminal control `{label}`."),
            None,
            None,
        ));

        Ok(self.snapshot())
    }

    pub fn restart_terminal_session(
        &mut self,
        app: &AppHandle,
        session_id: &str,
    ) -> Result<DashboardState, AppError> {
        let existing = self
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
            .ok_or_else(|| AppError::Message("Terminal session was not found.".into()))?;

        if let Some(mut runtime) = self.terminal_runtimes.remove(session_id) {
            let _ = runtime._child.kill();
        }

        let (runtime, mut session) = spawn_terminal_runtime(
            app.clone(),
            session_id.to_string(),
            Some(existing.title.clone()),
            existing.cwd.clone(),
        )?;
        runtime
            .master
            .resize(PtySize {
                rows: existing.rows,
                cols: existing.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| AppError::Message(format!("Failed to resize restarted PTY: {error}")))?;
        session.cols = existing.cols;
        session.rows = existing.rows;

        self.terminal_runtimes.insert(session_id.to_string(), runtime);
        if let Some(entry) = self.sessions.iter_mut().find(|item| item.id == session_id) {
            *entry = session;
        }

        self.audit.push(audit_event(
            "session",
            "human_terminal",
            Some("restarted".into()),
            format!("Restarted PTY command session `{}`.", existing.title),
            None,
            None,
        ));

        Ok(self.snapshot())
    }

    pub fn approve_request(
        &mut self,
        app: &AppHandle,
        request_id: &str,
        mode: ApprovalMode,
    ) -> Result<DashboardState, AppError> {
        let index = self
            .pending_approvals
            .iter()
            .position(|pending| pending.request.id == request_id)
            .ok_or_else(|| AppError::Message("Approval request was not found.".into()))?;

        let pending = self.pending_approvals.remove(index);
        let promoted_to_policy =
            self.apply_approval_scope_delta_if_persistent(&pending.decision, &mode);
        self.clear_obsolete_approvals_for_request(&pending.request);
        self.clear_obsolete_approvals_for_policy();
        self.grants.push(ApprovalGrant {
            request_id: pending.request.id.clone(),
            mode: mode.clone(),
            created_at: Utc::now(),
        });

        if let Some(session_id) = &pending.request.session_id {
            if let Some(command) = &pending.request.command {
                if let Some(worker_id) = &pending.request.worker_id {
                    self.execute_worker_command_in_terminal(
                        app,
                        worker_id,
                        session_id,
                        &pending.request.id,
                        command,
                    )?;
                } else {
                    self.write_to_terminal(session_id, command)?;
                }
                emit_terminal_output(app, session_id, "\r\n[proxy] approval granted\r\n");
            }
        }

        self.audit.push(audit_event(
            "approval",
            approval_source(&pending.request),
            Some("approved".into()),
            format!(
                "Approved `{}` with mode {:?}{}.",
                pending.request.target,
                mode,
                if promoted_to_policy {
                    " and promoted it into editable policy"
                } else {
                    ""
                }
            ),
            Some(pending.request.id),
            pending.request.worker_id.clone(),
        ));

        Ok(self.snapshot())
    }

    pub fn deny_request(
        &mut self,
        app: &AppHandle,
        request_id: &str,
    ) -> Result<DashboardState, AppError> {
        self.deny_request_internal(app, request_id, false)
    }

    pub fn deny_request_and_stop(
        &mut self,
        app: &AppHandle,
        request_id: &str,
    ) -> Result<DashboardState, AppError> {
        self.deny_request_internal(app, request_id, true)
    }

    fn deny_request_internal(
        &mut self,
        app: &AppHandle,
        request_id: &str,
        stop_worker: bool,
    ) -> Result<DashboardState, AppError> {
        let index = self
            .pending_approvals
            .iter()
            .position(|pending| pending.request.id == request_id)
            .ok_or_else(|| AppError::Message("Approval request was not found.".into()))?;

        let pending = self.pending_approvals.remove(index);
        self.clear_obsolete_approvals_for_request(&pending.request);
        if let Some(session_id) = &pending.request.session_id {
            emit_terminal_output(app, session_id, "\r\n[proxy] approval denied\r\n");
        }
        if let Some(worker_id) = &pending.request.worker_id {
            if let Some(command) = &pending.request.command {
                let _ = self.send_worker_command_result(
                    worker_id,
                    build_worker_command_result_envelope(
                        &pending.request.id,
                        command,
                        "denied",
                        None,
                        Vec::new(),
                        Vec::new(),
                        Some("Human supervisor denied the request."),
                    ),
                );
            }
        }
        self.audit.push(audit_event(
            "approval",
            approval_source(&pending.request),
            Some(if stop_worker { "denied_and_stopped" } else { "denied" }.into()),
            format!(
                "{} `{}`.",
                if stop_worker { "Denied and stopped" } else { "Denied" },
                pending.request.target
            ),
            Some(pending.request.id.clone()),
            pending.request.worker_id.clone(),
        ));

        if stop_worker {
            if let Some(worker_id) = pending.request.worker_id.clone() {
                self.stop_worker(&worker_id)?;
                if let Some(worker) = self.workers.iter_mut().find(|worker| worker.id == worker_id) {
                    worker.status = WorkerStatus::Failed;
                    emit_worker_output(
                        app,
                        &worker_id,
                        "[proxy] supervisor denied a blocked request and stopped the agent".into(),
                    );
                }
                self.audit.push(audit_event(
                    "worker",
                    "supervisor",
                    Some("stopped".into()),
                    "Stopped agent after denied approval request.",
                    Some(pending.request.id),
                    Some(worker_id.clone()),
                ));
                self.sync_worker_task_state(&worker_id, &WorkerStatus::Failed);
            }
        }
        Ok(self.snapshot())
    }

    pub fn export_audit_log(&self) -> Result<String, AppError> {
        serde_json::to_string_pretty(&self.audit)
            .map_err(|error| AppError::Message(format!("Failed to serialize audit log: {error}")))
    }

    fn apply_approval_scope_delta_if_persistent(
        &mut self,
        decision: &PolicyDecision,
        mode: &ApprovalMode,
    ) -> bool {
        if matches!(mode, ApprovalMode::Persistent) {
            self.apply_scope_delta(decision.clone());
            true
        } else {
            false
        }
    }

    pub fn update_policy(&mut self, policy: SessionPolicy) -> DashboardState {
        self.policy = policy;
        let removed = self.clear_obsolete_approvals_for_policy();
        self.audit.push(audit_event(
            "policy",
            "supervisor",
            Some("updated".into()),
            "Updated session policy.",
            None,
            None,
        ));
        if removed > 0 {
            self.audit.push(audit_event(
                "approval",
                "system",
                Some("cleared".into()),
                format!("Cleared {removed} obsolete approval request(s) after policy update."),
                None,
                None,
            ));
        }
        self.snapshot()
    }

    pub fn create_worker(
        &mut self,
        adapter: String,
        name: String,
        executable_path: Option<String>,
        args: Vec<String>,
        memory_mode: AgentMemoryMode,
        profile_id: Option<String>,
    ) -> DashboardState {
        let root = self
            .policy
            .roots
            .first()
            .cloned()
            .unwrap_or_else(|| "C:\\".into());
        let mut worker = agent::create_worker(
            name,
            adapter.clone(),
            root,
            executable_path,
            args,
            memory_mode.clone(),
        );
        if let Some(profile_id) = profile_id.clone() {
            apply_profile_to_worker(&mut worker, &profile_id, &self.profiles);
        }
        self.audit.push(audit_event(
            "worker",
            "supervisor",
            Some("created".into()),
            format!(
                "Created {adapter} worker `{}` with {} memory{}.",
                worker.id,
                format_memory_mode(&worker.memory_mode),
                worker
                    .profile_id
                    .as_ref()
                    .and_then(|id| self.profiles.iter().find(|profile| &profile.id == id))
                    .map(|profile| format!(" using profile `{}`", profile.name))
                    .unwrap_or_default()
            ),
            None,
            Some(worker.id.clone()),
        ));
        self.workers.push(worker);
        self.snapshot()
    }

    pub fn update_worker(
        &mut self,
        worker_id: &str,
        name: String,
        executable_path: Option<String>,
        args: Vec<String>,
        memory_mode: AgentMemoryMode,
        profile_id: Option<String>,
    ) -> Result<DashboardState, AppError> {
        let was_running = self.worker_runtimes.contains_key(worker_id);
        if was_running {
            self.stop_worker(worker_id)?;
        }

        let profiles = self.profiles.clone();
        let worker = self
            .workers
            .iter_mut()
            .find(|worker| worker.id == worker_id)
            .ok_or_else(|| AppError::Message("Worker was not found.".into()))?;

        worker.name = name.clone();
        worker.executable_path = executable_path;
        worker.args = args;
        worker.memory_mode = memory_mode.clone();

        match profile_id {
            Some(profile_id) => apply_profile_to_worker(worker, &profile_id, &profiles),
            None => worker.profile_id = None,
        }

        worker.name = name.clone();
        worker.memory_mode = memory_mode;
        if was_running {
            worker.status = WorkerStatus::Idle;
        }

        self.audit.push(audit_event(
            "worker",
            "supervisor",
            Some("updated".into()),
            format!(
                "Updated worker `{}`{}.",
                worker.name,
                if was_running {
                    " and stopped the active process so the new config can be relaunched"
                } else {
                    ""
                }
            ),
            None,
            Some(worker.id.clone()),
        ));

        Ok(self.snapshot())
    }

    pub fn delete_worker(&mut self, worker_id: &str) -> Result<DashboardState, AppError> {
        self.stop_worker(worker_id)?;

        let worker_index = self
            .workers
            .iter()
            .position(|worker| worker.id == worker_id)
            .ok_or_else(|| AppError::Message("Worker was not found.".into()))?;

        let worker = self.workers.remove(worker_index);
        self.tasks.retain(|task| task.assigned_worker_id.as_deref() != Some(worker_id));
        self.pending_approvals
            .retain(|approval| approval.request.worker_id.as_deref() != Some(worker_id));

        self.audit.push(audit_event(
            "worker",
            "supervisor",
            Some("deleted".into()),
            format!("Deleted worker `{}`.", worker.name),
            None,
            Some(worker.id.clone()),
        ));

        Ok(self.snapshot())
    }

    pub fn save_agent_profile(
        &mut self,
        name: String,
        allow_commands: Vec<String>,
        allow_domains: Vec<String>,
        memory_mode: AgentMemoryMode,
        delegation_mode: DelegationMode,
        delegation_max_depth: u8,
        default_guardrails: TaskGuardrails,
    ) -> DashboardState {
        let profile = AgentProfile {
            id: Uuid::new_v4().to_string(),
            name: name.clone(),
            built_in: false,
            allow_commands,
            allow_domains,
            memory_mode: memory_mode.clone(),
            delegation_mode,
            delegation_max_depth,
            default_guardrails,
        };
        self.audit.push(audit_event(
            "profile",
            "supervisor",
            Some("saved".into()),
            format!("Saved agent profile `{name}`."),
            None,
            None,
        ));
        self.profiles.push(profile);
        self.snapshot()
    }

    pub fn apply_agent_profile(
        &mut self,
        worker_id: &str,
        profile_id: &str,
    ) -> Result<DashboardState, AppError> {
        let profiles = self.profiles.clone();
        let profile_name = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.name.clone())
            .ok_or_else(|| AppError::Message("Agent profile was not found.".into()))?;

        let worker = self
            .workers
            .iter_mut()
            .find(|worker| worker.id == worker_id)
            .ok_or_else(|| AppError::Message("Worker was not found.".into()))?;

        apply_profile_to_worker(worker, profile_id, &profiles);
        self.audit.push(audit_event(
            "profile",
            "supervisor",
            Some("applied".into()),
            format!("Applied profile `{profile_name}` to agent `{}`.", worker.name),
            None,
            Some(worker.id.clone()),
        ));
        Ok(self.snapshot())
    }

    pub fn assign_task(
        &mut self,
        app: &AppHandle,
        worker_id: &str,
        title: String,
        summary: String,
        guardrails: TaskGuardrails,
    ) -> Result<DashboardState, AppError> {
        let worker = self
            .workers
            .iter_mut()
            .find(|worker| worker.id == worker_id)
            .ok_or_else(|| AppError::Message("Worker was not found.".into()))?;

        let task = agent::assign_task(worker, title, summary, guardrails.clone());
        if let Some(runtime) = self.worker_runtimes.get(&worker.id) {
            let envelope = format!(
                "TASK {}\nTITLE_B64: {}\nSUMMARY_B64: {}\nALLOW_SHELL: {}\nALLOW_NETWORK: {}\nALLOW_WRITES: {}\n\n",
                task.id,
                BASE64.encode(task.title.as_bytes()),
                BASE64.encode(task.summary.as_bytes()),
                task.guardrails.allow_shell,
                task.guardrails.allow_network,
                task.guardrails.allow_writes
            );
            runtime
                .stdin
                .lock()
                .map_err(|_| AppError::Message("Worker stdin lock failed.".into()))?
                .write_all(envelope.as_bytes())
                .map_err(|error| {
                    AppError::Message(format!("Failed to send worker task: {error}"))
                })?;
        } else {
            emit_worker_output(
                app,
                &worker.id,
                "[proxy] agent is not running yet; task queued for human review".into(),
            );
        }

        self.audit.push(audit_event(
            "task",
            "supervisor",
            Some("assigned".into()),
            format!(
                "Assigned task `{}` to agent `{}` with guardrails shell={}, network={}, writes={}.",
                task.title,
                worker.name,
                task.guardrails.allow_shell,
                task.guardrails.allow_network,
                task.guardrails.allow_writes
            ),
            Some(task.id.clone()),
            Some(worker.id.clone()),
        ));
        self.tasks.push(task);
        Ok(self.snapshot())
    }

    pub fn delete_task(&mut self, task_id: &str) -> Result<DashboardState, AppError> {
        let task_index = self
            .tasks
            .iter()
            .position(|task| task.id == task_id)
            .ok_or_else(|| AppError::Message("Task was not found.".into()))?;

        let task = self.tasks.remove(task_index);
        if let Some(worker_id) = &task.assigned_worker_id {
            if let Some(worker) = self.workers.iter_mut().find(|worker| worker.id == *worker_id) {
                if worker.current_task.as_deref() == Some(task_id) {
                    worker.current_task = None;
                    if worker.status == WorkerStatus::Running {
                        worker.status = WorkerStatus::Idle;
                    }
                }
            }
        }

        self.audit.push(audit_event(
            "task",
            "supervisor",
            Some("deleted".into()),
            format!("Deleted task `{}`.", task.title),
            Some(task.id.clone()),
            task.assigned_worker_id.clone(),
        ));

        Ok(self.snapshot())
    }

    fn handle_worker_envelope_command(
        &mut self,
        app: &AppHandle,
        worker_id: &str,
        envelope: WorkerEnvelopeCommand,
    ) -> Result<(), AppError> {
        let worker = self
            .workers
            .iter()
            .find(|worker| worker.id == worker_id)
            .cloned()
            .ok_or_else(|| AppError::Message("Worker was not found.".into()))?;
        let worker_name = worker.name.clone();
        let worker_scope_root = worker.scope_roots.first().cloned();
        let worker_id_owned = worker.id.clone();

        let session_id = self.ensure_worker_terminal_session(app, worker_id)?;
        let cwd = envelope
            .cwd
            .clone()
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|session| session.id == session_id)
                    .map(|session| session.cwd.clone())
            })
            .or(worker_scope_root);

        let request = ActionRequest {
            id: Uuid::new_v4().to_string(),
            kind: ActionKind::Command,
            target: envelope.command.clone(),
            command: Some(envelope.command.clone()),
            cwd,
            args: None,
            rationale: Some("Worker command envelope".into()),
            worker_id: Some(worker_id.to_string()),
            session_id: Some(session_id.clone()),
        };

        if let Some(reason) = self.guardrail_block_reason(worker_id, &envelope.command) {
            emit_terminal_output(app, &session_id, "\r\n[proxy] worker input denied by task guardrail\r\n");
            emit_worker_output(
                app,
                worker_id,
                format!("[proxy] worker command `{}` denied by task guardrail: {reason}", envelope.command),
            );
            self.audit.push(audit_event(
                "policy",
                "agent_envelope",
                Some("denied".into()),
                format!("worker {worker_name}: {} -> denied by task guardrail. {reason}", request.target),
                Some(request.id.clone()),
                Some(worker_id_owned.clone()),
            ));
            let _ = self.send_worker_command_result(
                worker_id,
                build_worker_command_result_envelope(
                    &request.id,
                    &envelope.command,
                    "denied",
                    None,
                    Vec::new(),
                    Vec::new(),
                    Some(&reason),
                ),
            );
            return Ok(());
        }

        let effective_policy = effective_policy_for_worker(&self.policy, &worker, &self.profiles);
        let decision = evaluate_request(&effective_policy, &request, &self.grants);
        self.audit.push(audit_event(
            "policy",
            "agent_envelope",
            Some(policy_outcome_value(&decision).into()),
            describe_policy_outcome(&format!("worker {worker_name}: {}", request.target), &decision),
            Some(request.id.clone()),
            Some(worker_id_owned.clone()),
        ));

        match decision.verdict {
            PolicyVerdict::Allow => {
                self.execute_worker_command_in_terminal(
                    app,
                    worker_id,
                    &session_id,
                    &request.id,
                    &envelope.command,
                )?;
                if let Some(worker) = self.workers.iter_mut().find(|worker| worker.id == worker_id) {
                    worker.compatibility = crate::models::AgentCompatibility::BrokerCompatible;
                }
                emit_worker_output(
                    app,
                    worker_id,
                    format!("[proxy] allowed worker command `{}`", envelope.command),
                );
                self.audit.push(audit_event(
                    "command",
                    "agent_envelope",
                    Some("sent".into()),
                    format!("Sent worker PTY input `{}`.", envelope.command),
                    Some(request.id),
                    Some(worker_id_owned.clone()),
                ));
            }
            PolicyVerdict::Prompt => {
                self.pending_approvals
                    .push(PendingApproval { request, decision });
                emit_terminal_output(
                    app,
                    &session_id,
                    "\r\n[proxy] worker input blocked pending approval\r\n",
                );
                emit_worker_output(
                    app,
                    worker_id,
                    format!("[proxy] worker command `{}` blocked pending approval", envelope.command),
                );
            }
            PolicyVerdict::Deny => {
                let request_id = request.id.clone();
                emit_terminal_output(app, &session_id, "\r\n[proxy] worker input denied\r\n");
                emit_worker_output(
                    app,
                    worker_id,
                    format!("[proxy] worker command `{}` denied", envelope.command),
                );
                self.audit.push(audit_event(
                    "command",
                    "agent_envelope",
                    Some("denied".into()),
                    format!("Worker PTY input denied: `{}`.", envelope.command),
                    Some(request_id.clone()),
                    Some(worker_id_owned),
                ));
                let _ = self.send_worker_command_result(
                    worker_id,
                    build_worker_command_result_envelope(
                        &request_id,
                        &envelope.command,
                        "denied",
                        None,
                        Vec::new(),
                        Vec::new(),
                        Some("Command denied by policy."),
                    ),
                );
            }
        }

        Ok(())
    }

    pub fn set_worker_status(
        &mut self,
        app: &AppHandle,
        worker_id: &str,
        status: WorkerStatus,
    ) -> Result<DashboardState, AppError> {
        match status {
            WorkerStatus::Running => {
                if let Err(error) = self.start_worker(app, worker_id) {
                    self.handle_worker_error(app, worker_id, error.to_string());
                    return Err(error);
                }
            }
            WorkerStatus::Paused | WorkerStatus::Completed | WorkerStatus::Failed => {
                self.stop_worker(worker_id)?
            }
            WorkerStatus::Idle => {}
        }

        let worker = self
            .workers
            .iter_mut()
            .find(|worker| worker.id == worker_id)
            .ok_or_else(|| AppError::Message("Worker was not found.".into()))?;
        worker.status = status.clone();

        self.audit.push(audit_event(
            "worker",
            "supervisor",
            Some(format!("{status:?}").to_ascii_lowercase()),
            format!("Worker `{}` moved to {:?}.", worker.name, status),
            None,
            Some(worker.id.clone()),
        ));
        self.sync_worker_task_state(worker_id, &status);
        if matches!(status, WorkerStatus::Completed | WorkerStatus::Failed) {
            self.cleanup_worker_memory_if_needed(worker_id, "agent")?;
        }
        Ok(self.snapshot())
    }

    fn harden_openclaw_config(
        &mut self,
        app: &AppHandle,
        worker_id: Option<&str>,
        trigger: &str,
    ) {
        match enforce_openclaw_secure_auth_setting() {
            Ok(true) => {
                let message = format!(
                    "[proxy] hardened OpenClaw config after {trigger}: set gateway.controlUi.allowInsecureAuth=false"
                );
                if let Some(worker_id) = worker_id {
                    emit_worker_output(app, worker_id, message.clone());
                }
                self.audit.push(audit_event(
                    "security",
                    "supervisor",
                    Some("hardened".into()),
                    message,
                    None,
                    worker_id.map(|id| id.to_string()),
                ));
            }
            Ok(false) => {}
            Err(error) => {
                let message = format!("OpenClaw config hardening failed after {trigger}: {error}");
                if let Some(worker_id) = worker_id {
                    emit_worker_output(app, worker_id, format!("[proxy] {message}"));
                }
                self.audit.push(audit_event(
                    "security",
                    "supervisor",
                    Some("warning".into()),
                    message,
                    None,
                    worker_id.map(|id| id.to_string()),
                ));
            }
        }
    }

    fn start_worker(&mut self, app: &AppHandle, worker_id: &str) -> Result<(), AppError> {
        if self.worker_runtimes.contains_key(worker_id) {
            return Ok(());
        }

        let worker_index = self
            .workers
            .iter()
            .position(|worker| worker.id == worker_id)
            .ok_or_else(|| AppError::Message("Worker was not found.".into()))?;

        let worker_adapter = self.workers[worker_index].adapter.clone();
        let worker_runtime_id = self.workers[worker_index].id.clone();
        let executable = self.workers[worker_index]
            .executable_path
            .clone()
            .ok_or_else(|| AppError::Message("Worker has no executable path configured.".into()))?;
        let executable_path = PathBuf::from(&executable);
        if !executable_path.exists() {
            return Err(AppError::Message(format!(
                "Agent executable was not found at `{}`.",
                executable
            )));
        }
        if !executable_path.is_file() {
            return Err(AppError::Message(format!(
                "Agent executable path `{}` is not a file.",
                executable
            )));
        }
        if worker_adapter.eq_ignore_ascii_case("openclaw") {
            self.harden_openclaw_config(app, Some(&worker_runtime_id), "worker launch");
        }
        let worker = &mut self.workers[worker_index];
        let sandbox_dir = ensure_worker_sandbox_dir(worker)?;

        let mut command = Command::new(&executable);
        command
            .args(&worker.args)
            .current_dir(&sandbox_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        apply_worker_sandbox_env(&mut command, worker, &sandbox_dir);

        let mut child = command.spawn().map_err(|error| {
            AppError::Message(format!("Failed to start worker process: {error}"))
        })?;
        let os_enforcement = security::apply_worker_os_enforcement(&child)
            .map_err(|error| AppError::Message(format!("Failed to apply OS enforcement: {error}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Message("Failed to capture worker stdin.".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Message("Failed to capture worker stdout.".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Message("Failed to capture worker stderr.".into()))?;

        if worker_adapter.eq_ignore_ascii_case("openclaw") {
            schedule_openclaw_config_hardening(app.clone(), worker.id.clone());
        }

        spawn_worker_reader(app.clone(), worker.id.clone(), stdout, "stdout");
        spawn_worker_reader(app.clone(), worker.id.clone(), stderr, "stderr");

        self.worker_runtimes.insert(
            worker.id.clone(),
            WorkerRuntime {
                stdin: Arc::new(Mutex::new(stdin)),
                child,
                _os_enforcement: os_enforcement,
            },
        );

        emit_worker_output(
            app,
            &worker.id,
            format!(
                "[proxy] started sandboxed {} agent at {} in {}",
                worker.adapter,
                executable,
                sandbox_dir.display()
            ),
        );
        emit_worker_output(
            app,
            &worker.id,
            "[proxy] active protections: sandboxed launch, stripped environment, job object, one-process limit".into(),
        );
        Ok(())
    }

    fn stop_worker(&mut self, worker_id: &str) -> Result<(), AppError> {
        if let Some(mut runtime) = self.worker_runtimes.remove(worker_id) {
            runtime
                .child
                .kill()
                .map_err(|error| AppError::Message(format!("Failed to stop worker: {error}")))?;
        }

        Ok(())
    }

    fn cleanup_worker_memory_if_needed(
        &mut self,
        worker_id: &str,
        scope: &str,
    ) -> Result<(), AppError> {
        let Some(worker) = self.workers.iter().find(|worker| worker.id == worker_id) else {
            return Ok(());
        };

        let should_cleanup = matches!(
            (&worker.memory_mode, scope),
            (AgentMemoryMode::Ephemeral, _)
                | (AgentMemoryMode::TaskScoped, "task")
                | (AgentMemoryMode::TaskScoped, "agent")
        );

        if !should_cleanup {
            return Ok(());
        }

        let sandbox_dir = ensure_worker_sandbox_dir(worker)?;
        if sandbox_dir.exists() {
            fs::remove_dir_all(&sandbox_dir).map_err(|error| {
                AppError::Message(format!(
                    "Failed to clear agent memory sandbox `{}`: {error}",
                    sandbox_dir.display()
                ))
            })?;
            self.audit.push(audit_event(
                "memory",
                "system",
                Some("cleared".into()),
                format!(
                    "Cleared {} memory for agent `{}` at `{}`.",
                    format_memory_mode(&worker.memory_mode),
                    worker.name,
                    sandbox_dir.display()
                ),
                None,
                Some(worker.id.clone()),
            ));
        }

        Ok(())
    }

    fn ensure_worker_terminal_session(
        &mut self,
        app: &AppHandle,
        worker_id: &str,
    ) -> Result<String, AppError> {
        if let Some(session_id) = self.worker_terminal_sessions.get(worker_id) {
            return Ok(session_id.clone());
        }

        let worker_name = self
            .workers
            .iter()
            .find(|worker| worker.id == worker_id)
            .map(|worker| worker.name.clone())
            .unwrap_or_else(|| "Worker".into());

        let cwd = self
            .workers
            .iter()
            .find(|worker| worker.id == worker_id)
            .and_then(|worker| worker.scope_roots.first().cloned())
            .or_else(|| self.policy.roots.first().cloned())
            .unwrap_or_else(|| "C:\\".into());

        let session_id = Uuid::new_v4().to_string();
        let title = format!("{worker_name} Shell");
        let (runtime, session) =
            spawn_terminal_runtime(app.clone(), session_id.clone(), Some(title), cwd)?;

        self.audit.push(audit_event(
            "session",
            "agent_envelope",
            Some("created".into()),
            format!("Created worker PTY session `{}`.", session.title),
            None,
            Some(worker_id.to_string()),
        ));
        self.terminal_runtimes.insert(session_id.clone(), runtime);
        self.sessions.push(session);
        self.worker_terminal_sessions
            .insert(worker_id.to_string(), session_id.clone());
        Ok(session_id)
    }

    fn execute_worker_command_in_terminal(
        &mut self,
        app: &AppHandle,
        worker_id: &str,
        session_id: &str,
        request_id: &str,
        command: &str,
    ) -> Result<(), AppError> {
        let sandbox_dir = self
            .workers
            .iter()
            .find(|worker| worker.id == worker_id)
            .and_then(|worker| worker.scope_roots.first().cloned())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(default_policy_root()))
            .join(".orc-agent-sandboxes")
            .join(worker_id);

        fs::create_dir_all(&sandbox_dir).map_err(|error| {
            AppError::Message(format!(
                "Failed to prepare worker command sandbox `{}`: {error}",
                sandbox_dir.display()
            ))
        })?;

        let execution = PendingWorkerCommandExecution {
            request_id: request_id.to_string(),
            worker_id: worker_id.to_string(),
            command: command.to_string(),
            stdout_path: sandbox_dir.join(format!("command-{request_id}-stdout.txt")),
            stderr_path: sandbox_dir.join(format!("command-{request_id}-stderr.txt")),
            result_path: sandbox_dir.join(format!("command-{request_id}-result.json")),
        };

        let wrapped = build_worker_command_wrapper(&execution);
        self.write_to_terminal(session_id, &wrapped)?;
        spawn_worker_command_result_watcher(app.clone(), execution);
        Ok(())
    }

    fn write_to_terminal(&mut self, session_id: &str, input: &str) -> Result<(), AppError> {
        self.write_raw_to_terminal(session_id, format!("{input}\r").as_bytes())
    }

    fn write_raw_to_terminal(&mut self, session_id: &str, bytes: &[u8]) -> Result<(), AppError> {
        let runtime = self
            .terminal_runtimes
            .get_mut(session_id)
            .ok_or_else(|| AppError::Message("Terminal session runtime was not found.".into()))?;

        let mut writer = runtime
            .writer
            .lock()
            .map_err(|_| AppError::Message("PTY writer lock failed.".into()))?;
        writer
            .write_all(bytes)
            .map_err(|error| AppError::Message(format!("Failed to write to PTY: {error}")))?;
        writer
            .flush()
            .map_err(|error| AppError::Message(format!("Failed to flush PTY input: {error}")))?;
        Ok(())
    }

    fn guardrail_block_reason(&self, worker_id: &str, command: &str) -> Option<String> {
        let task_id = self
            .workers
            .iter()
            .find(|worker| worker.id == worker_id)
            .and_then(|worker| worker.current_task.clone())?;

        let task = self.tasks.iter().find(|task| task.id == task_id)?;

        if !task.guardrails.allow_shell {
            return Some(format!(
                "Task `{}` does not allow shell execution.",
                task.title
            ));
        }

        if !task.guardrails.allow_network && command_requests_network(command) {
            return Some(format!(
                "Task `{}` blocks network commands.",
                task.title
            ));
        }

        if !task.guardrails.allow_writes && command_may_write(command) {
            return Some(format!(
                "Task `{}` blocks write-capable commands.",
                task.title
            ));
        }

        None
    }

    fn send_worker_command_result(
        &mut self,
        worker_id: &str,
        envelope: String,
    ) -> Result<(), AppError> {
        let runtime = self
            .worker_runtimes
            .get(worker_id)
            .ok_or_else(|| AppError::Message("Agent runtime was not found.".into()))?;

        runtime
            .stdin
            .lock()
            .map_err(|_| AppError::Message("Agent stdin lock failed.".into()))?
            .write_all(envelope.as_bytes())
            .map_err(|error| AppError::Message(format!("Failed to send command result to agent: {error}")))?;

        Ok(())
    }

    fn complete_worker_command_execution(
        &mut self,
        execution: PendingWorkerCommandExecution,
    ) -> Result<(), AppError> {
        let result_text = fs::read_to_string(&execution.result_path).map_err(|error| {
            AppError::Message(format!(
                "Failed to read worker command result `{}`: {error}",
                execution.result_path.display()
            ))
        })?;
        let result_text = result_text.trim_start_matches('\u{feff}');
        let result: WorkerCommandResultFile = serde_json::from_str(result_text).map_err(|error| {
            AppError::Message(format!(
                "Failed to parse worker command result `{}`: {error}",
                execution.result_path.display()
            ))
        })?;

        if result.request_id != execution.request_id {
            return Err(AppError::Message(format!(
                "Worker command result id mismatch: expected `{}`, got `{}`.",
                execution.request_id, result.request_id
            )));
        }

        let stdout = fs::read(&execution.stdout_path).unwrap_or_default();
        let stderr = fs::read(&execution.stderr_path).unwrap_or_default();
        let envelope = build_worker_command_result_envelope(
            &execution.request_id,
            &execution.command,
            "completed",
            Some(result.exit_code),
            stdout,
            stderr,
            None,
        );

        self.send_worker_command_result(&execution.worker_id, envelope)?;
        self.audit.push(audit_event(
            "command",
            "system",
            Some("returned".into()),
            format!(
                "Returned PTY result for worker command `{}` with exit code {}.",
                execution.command, result.exit_code
            ),
            Some(execution.request_id.clone()),
            Some(execution.worker_id.clone()),
        ));

        let _ = fs::remove_file(&execution.result_path);
        let _ = fs::remove_file(&execution.stdout_path);
        let _ = fs::remove_file(&execution.stderr_path);

        Ok(())
    }

    fn apply_scope_delta(&mut self, decision: PolicyDecision) {
        if let Some(scope_delta) = decision.scope_delta {
            for root in scope_delta.add_roots {
                if !self.policy.roots.contains(&root) {
                    self.policy.roots.push(root);
                }
            }

            for domain in scope_delta.add_domains {
                if !self.policy.allow_domains.contains(&domain) {
                    self.policy.allow_domains.push(domain);
                }
            }

            for command in scope_delta.add_commands {
                if !self
                    .policy
                    .allow_commands
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(&command))
                {
                    self.policy.allow_commands.push(command);
                }
            }
        }
    }

    fn clear_obsolete_approvals_for_request(
        &mut self,
        request: &ActionRequest,
    ) -> usize {
        let before = self.pending_approvals.len();
        self.pending_approvals.retain(|pending| {
            pending.request.id == request.id
                || pending.request.kind != request.kind
                || pending.request.target != request.target
                || pending.request.session_id != request.session_id
                || pending.request.worker_id != request.worker_id
        });
        before.saturating_sub(self.pending_approvals.len())
    }

    fn clear_obsolete_approvals_for_policy(&mut self) -> usize {
        let before = self.pending_approvals.len();
        self.pending_approvals.retain(|pending| {
            matches!(
                evaluate_request(&self.policy, &pending.request, &self.grants).verdict,
                PolicyVerdict::Prompt
            )
        });
        before.saturating_sub(self.pending_approvals.len())
    }

    fn handle_worker_error(
        &mut self,
        app: &AppHandle,
        worker_id: &str,
        message: String,
    ) {
        if let Some(worker) = self.workers.iter_mut().find(|worker| worker.id == worker_id) {
            worker.status = WorkerStatus::Failed;
            emit_worker_output(app, worker_id, format!("[proxy] {message}"));
            self.audit.push(audit_event(
                "worker",
                "system",
                Some("failed".into()),
                format!("Worker `{}` failed: {message}", worker.name),
                None,
                Some(worker.id.clone()),
            ));
            self.sync_worker_task_state(worker_id, &WorkerStatus::Failed);
            let _ = self.cleanup_worker_memory_if_needed(worker_id, "agent");
        }
    }

    fn handle_worker_protocol_violation(
        &mut self,
        app: &AppHandle,
        worker_id: &str,
        message: String,
    ) {
        let _ = self.stop_worker(worker_id);
        self.handle_worker_error(
            app,
            worker_id,
            format!("terminated agent for broker violation: {message}"),
        );
    }

    fn sync_worker_task_state(&mut self, worker_id: &str, status: &WorkerStatus) {
        let current_task_id = self
            .workers
            .iter()
            .find(|worker| worker.id == worker_id)
            .and_then(|worker| worker.current_task.clone());

        if let Some(task_id) = current_task_id {
            if let Some(task) = self.tasks.iter_mut().find(|task| task.id == task_id) {
                task.status = status.clone();
            }
        }

        if matches!(status, WorkerStatus::Completed | WorkerStatus::Failed) {
            if let Some(worker) = self.workers.iter_mut().find(|worker| worker.id == worker_id) {
                worker.current_task = None;
            }
        }
    }

    fn handle_worker_status_line(
        &mut self,
        worker_id: &str,
        line_value: &str,
    ) {
        let normalized = line_value.to_ascii_lowercase();

        if normalized.contains("adapter-status: run ") && normalized.contains(" completed with status ok")
        {
            if let Some(worker) = self.workers.iter_mut().find(|worker| worker.id == worker_id) {
                worker.status = WorkerStatus::Idle;
            }
            self.sync_worker_task_state(worker_id, &WorkerStatus::Completed);
            let _ = self.cleanup_worker_memory_if_needed(worker_id, "task");
            self.audit.push(audit_event(
                "task",
                "agent_runtime",
                Some("completed".into()),
                format!("Worker `{}` completed its current task.", worker_id),
                None,
                Some(worker_id.to_string()),
            ));
        }
    }
}

fn default_policy_root() -> String {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("C:\\"));

    let resolved = if cwd
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("src-tauri"))
    {
        cwd.parent()
            .map(PathBuf::from)
            .unwrap_or(cwd)
    } else {
        cwd
    };

    resolved.to_string_lossy().to_string()
}

fn ensure_worker_sandbox_dir(worker: &Worker) -> Result<PathBuf, AppError> {
    let base_root = worker
        .scope_roots
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default_policy_root()));
    let sandbox_dir = base_root
        .join(".orc-agent-sandboxes")
        .join(&worker.id);

    fs::create_dir_all(&sandbox_dir).map_err(|error| {
        AppError::Message(format!(
            "Failed to create sandbox directory `{}`: {error}",
            sandbox_dir.display()
        ))
    })?;

    Ok(sandbox_dir)
}

fn apply_worker_sandbox_env(command: &mut Command, worker: &Worker, sandbox_dir: &PathBuf) {
    for key in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Ok(value) = env::var(key) {
            command.env(key, value);
        }
    }

    command
        .env("PATH", sandbox_dir.as_os_str())
        .env("PYTHONIOENCODING", "utf-8")
        .env("ORC_TERMINAL_ENVELOPE_STDOUT", "1")
        .env("ORC_TERMINAL_COMMAND_PREFIX", "PROXY_CMD")
        .env("ORC_TERMINAL_JSON_PREFIX", "PROXY_JSON")
        .env("ORC_TERMINAL_SANDBOX_ROOT", sandbox_dir.as_os_str())
        .env("ORC_TERMINAL_MEMORY_MODE", format_memory_mode(&worker.memory_mode))
        .env("ORC_TERMINAL_DELEGATION_MODE", format_capability_setting(&worker.capability_profile.delegation))
        .env(
            "ORC_TERMINAL_POLICY_ROOT",
            worker
                .scope_roots
                .first()
                .map(|root| root.as_str())
                .unwrap_or_default(),
        );
}

fn describe_policy_outcome(target: &str, decision: &PolicyDecision) -> String {
    match decision.verdict {
        PolicyVerdict::Allow => format!("{target} -> allowed. {}", decision.reason),
        PolicyVerdict::Prompt => {
            format!("{target} -> blocked pending approval. {}", decision.reason)
        }
        PolicyVerdict::Deny => format!("{target} -> denied. {}", decision.reason),
    }
}

fn policy_outcome_value(decision: &PolicyDecision) -> &'static str {
    match decision.verdict {
        PolicyVerdict::Allow => "allowed",
        PolicyVerdict::Prompt => "blocked_pending_approval",
        PolicyVerdict::Deny => "denied",
    }
}

fn approval_source(request: &ActionRequest) -> &'static str {
    if request.worker_id.is_some() {
        "agent_envelope"
    } else {
        "human_terminal"
    }
}

fn default_agent_profiles() -> Vec<AgentProfile> {
    vec![
        AgentProfile {
            id: "profile-strict-broker".into(),
            name: "Strict Broker-Only".into(),
            built_in: true,
            allow_commands: vec![
                "dir".into(),
                "pwd".into(),
                "get-childitem".into(),
                "get-location".into(),
                "get-content".into(),
                "select-string".into(),
                "test-path".into(),
                "type".into(),
                "node".into(),
                "npm".into(),
                "openclaw".into(),
                "powershell".into(),
                "y".into(),
                "n".into(),
                "yes".into(),
                "no".into(),
            ],
            allow_domains: vec!["localhost".into(), "openclaw.ai".into()],
            memory_mode: AgentMemoryMode::Ephemeral,
            delegation_mode: DelegationMode::Deny,
            delegation_max_depth: 0,
            default_guardrails: TaskGuardrails {
                allow_shell: true,
                allow_network: false,
                allow_writes: false,
            },
        },
        AgentProfile {
            id: "profile-safe-coder".into(),
            name: "Safe Coder".into(),
            built_in: true,
            allow_commands: vec![
                "dir".into(),
                "pwd".into(),
                "get-childitem".into(),
                "get-location".into(),
                "type".into(),
                "y".into(),
                "n".into(),
                "yes".into(),
                "no".into(),
                "git".into(),
                "npm".into(),
                "cargo".into(),
            ],
            allow_domains: vec!["localhost".into(), "openclaw.ai".into(), "api.openai.com".into()],
            memory_mode: AgentMemoryMode::TaskScoped,
            delegation_mode: DelegationMode::Prompt,
            delegation_max_depth: 1,
            default_guardrails: TaskGuardrails {
                allow_shell: true,
                allow_network: false,
                allow_writes: true,
            },
        },
        AgentProfile {
            id: "profile-research-agent".into(),
            name: "Research Agent".into(),
            built_in: true,
            allow_commands: vec![
                "dir".into(),
                "pwd".into(),
                "get-childitem".into(),
                "get-location".into(),
                "type".into(),
                "y".into(),
                "n".into(),
                "yes".into(),
                "no".into(),
                "curl".into(),
                "wget".into(),
                "iwr".into(),
                "irm".into(),
            ],
            allow_domains: vec!["localhost".into(), "openclaw.ai".into()],
            memory_mode: AgentMemoryMode::AgentScoped,
            delegation_mode: DelegationMode::Deny,
            delegation_max_depth: 0,
            default_guardrails: TaskGuardrails {
                allow_shell: true,
                allow_network: true,
                allow_writes: false,
            },
        },
    ]
}

fn effective_policy_for_worker(
    base: &SessionPolicy,
    worker: &Worker,
    profiles: &[AgentProfile],
) -> SessionPolicy {
    let mut policy = base.clone();
    if let Some(profile_id) = &worker.profile_id {
        if let Some(profile) = profiles.iter().find(|profile| &profile.id == profile_id) {
            policy.allow_commands = profile.allow_commands.clone();
            policy.allow_domains = profile.allow_domains.clone();
            policy.default_memory_mode = profile.memory_mode.clone();
            policy.delegation_mode = profile.delegation_mode.clone();
            policy.delegation_max_depth = profile.delegation_max_depth;
        }
    }
    policy
}

fn apply_profile_to_worker(worker: &mut Worker, profile_id: &str, profiles: &[AgentProfile]) {
    if let Some(profile) = profiles.iter().find(|profile| profile.id == profile_id) {
        worker.profile_id = Some(profile.id.clone());
        worker.memory_mode = profile.memory_mode.clone();
        worker.capability_profile.memory = match profile.memory_mode {
            AgentMemoryMode::Ephemeral | AgentMemoryMode::TaskScoped => AgentCapabilitySetting::Isolated,
            AgentMemoryMode::AgentScoped => AgentCapabilitySetting::Scoped,
            AgentMemoryMode::Persistent => AgentCapabilitySetting::Prompted,
        };
        worker.capability_profile.delegation = match profile.delegation_mode {
            DelegationMode::Deny => AgentCapabilitySetting::Denied,
            DelegationMode::Prompt => AgentCapabilitySetting::HumanOnly,
            DelegationMode::Allow => AgentCapabilitySetting::Prompted,
        };
        worker.capability_profile.network = if profile.allow_domains.is_empty() {
            AgentCapabilitySetting::Denied
        } else {
            AgentCapabilitySetting::Prompted
        };
    }
}

fn format_memory_mode(mode: &AgentMemoryMode) -> &'static str {
    match mode {
        AgentMemoryMode::Ephemeral => "ephemeral",
        AgentMemoryMode::TaskScoped => "task_scoped",
        AgentMemoryMode::AgentScoped => "agent_scoped",
        AgentMemoryMode::Persistent => "persistent",
    }
}

fn format_capability_setting(setting: &AgentCapabilitySetting) -> &'static str {
    match setting {
        AgentCapabilitySetting::Brokered => "brokered",
        AgentCapabilitySetting::Scoped => "scoped",
        AgentCapabilitySetting::Prompted => "prompted",
        AgentCapabilitySetting::HumanOnly => "human_only",
        AgentCapabilitySetting::Denied => "denied",
        AgentCapabilitySetting::Isolated => "isolated",
    }
}

fn command_verb(command: &str) -> String {
    command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn command_requests_network(command: &str) -> bool {
    matches!(
        command_verb(command).as_str(),
        "curl"
            | "wget"
            | "irm"
            | "iwr"
            | "invoke-webrequest"
            | "ping"
            | "nslookup"
            | "test-netconnection"
    )
}

fn command_may_write(command: &str) -> bool {
    if command.contains(">>") || command.contains('>') {
        return true;
    }

    let mut parts = command.split_whitespace();
    let verb = parts.next().unwrap_or_default().to_ascii_lowercase();
    let sub = parts.next().unwrap_or_default().to_ascii_lowercase();

    match verb.as_str() {
        "set-content" | "add-content" | "out-file" | "copy-item" | "move-item" | "remove-item"
        | "rename-item" | "new-item" | "mkdir" | "md" | "ni" | "touch" => true,
        "git" => !matches!(sub.as_str(), "" | "status" | "diff" | "log" | "show" | "branch"),
        "npm" => !matches!(sub.as_str(), "" | "view" | "list" | "prefix"),
        "cargo" => !matches!(sub.as_str(), "" | "check" | "test" | "tree" | "metadata"),
        _ => false,
    }
}

fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

fn build_worker_command_wrapper(execution: &PendingWorkerCommandExecution) -> String {
    let command_b64 = BASE64.encode(execution.command.as_bytes());
    let stdout_path = escape_powershell_single_quoted(&execution.stdout_path.to_string_lossy());
    let stderr_path = escape_powershell_single_quoted(&execution.stderr_path.to_string_lossy());
    let result_path = escape_powershell_single_quoted(&execution.result_path.to_string_lossy());
    let request_id = escape_powershell_single_quoted(&execution.request_id);

    format!(
        "& {{ \
$__orcCommand = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String('{command_b64}')); \
$__orcStdout = '{stdout_path}'; \
$__orcStderr = '{stderr_path}'; \
$__orcResult = '{result_path}'; \
Remove-Item -LiteralPath $__orcStdout, $__orcStderr, $__orcResult -ErrorAction SilentlyContinue; \
try {{ \
  Invoke-Expression $__orcCommand 1> $__orcStdout 2> $__orcStderr; \
  if ($null -ne $LASTEXITCODE) {{ $__orcExit = [int]$LASTEXITCODE }} elseif ($?) {{ $__orcExit = 0 }} else {{ $__orcExit = 1 }}; \
}} catch {{ \
  $_ | Out-String | Set-Content -LiteralPath $__orcStderr -Encoding utf8; \
  $__orcExit = if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {{ [int]$LASTEXITCODE }} else {{ 1 }}; \
}}; \
if (Test-Path -LiteralPath $__orcStdout) {{ Get-Content -LiteralPath $__orcStdout; }}; \
if (Test-Path -LiteralPath $__orcStderr) {{ Get-Content -LiteralPath $__orcStderr; }}; \
$__orcResultPayload = @{{ requestId = '{request_id}'; exitCode = $__orcExit }} | ConvertTo-Json -Compress; \
Set-Content -LiteralPath $__orcResult -Value $__orcResultPayload -Encoding utf8; \
}}"
    )
}

fn build_worker_command_result_envelope(
    request_id: &str,
    command: &str,
    status: &str,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    reason: Option<&str>,
) -> String {
    let combined = if stdout.is_empty() && stderr.is_empty() {
        Vec::new()
    } else if stderr.is_empty() {
        stdout.clone()
    } else if stdout.is_empty() {
        stderr.clone()
    } else {
        let mut merged = stdout.clone();
        if !merged.ends_with(b"\n") {
            merged.push(b'\n');
        }
        merged.extend_from_slice(&stderr);
        merged
    };

    let mut envelope = format!(
        "COMMAND_RESULT {request_id}\nSTATUS: {status}\nCOMMAND_B64: {}\nSTREAM_MODE: separated_capture\n",
        BASE64.encode(command.as_bytes())
    );

    if let Some(exit_code) = exit_code {
        envelope.push_str(&format!("EXIT_CODE: {exit_code}\n"));
    }

    if let Some(reason) = reason {
        envelope.push_str(&format!("REASON_B64: {}\n", BASE64.encode(reason.as_bytes())));
    }

    envelope.push_str(&format!(
        "STDOUT_B64: {}\nSTDERR_B64: {}\nCOMBINED_B64: {}\n\n",
        BASE64.encode(stdout),
        BASE64.encode(stderr),
        BASE64.encode(combined)
    ));
    envelope
}

fn spawn_worker_command_result_watcher(app: AppHandle, execution: PendingWorkerCommandExecution) {
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(300);
        while Instant::now() < deadline {
            if execution.result_path.exists() {
                let state = app.state::<Mutex<ProxyTerminalState>>();
                match state.lock() {
                    Ok(mut state) => {
                        if let Err(error) = state.complete_worker_command_execution(execution.clone()) {
                            state.handle_worker_error(
                                &app,
                                &execution.worker_id,
                                format!("failed to return command result: {error}"),
                            );
                        }
                    }
                    Err(_) => emit_worker_output(
                        &app,
                        &execution.worker_id,
                        "[proxy] failed to return command result: application state lock was poisoned".into(),
                    ),
                }
                return;
            }

            thread::sleep(Duration::from_millis(200));
        }

        let state = app.state::<Mutex<ProxyTerminalState>>();
        let lock_result = state.lock();
        if let Ok(mut state) = lock_result {
            let _ = state.send_worker_command_result(
                &execution.worker_id,
                build_worker_command_result_envelope(
                    &execution.request_id,
                    &execution.command,
                    "failed",
                    None,
                    Vec::new(),
                    Vec::new(),
                    Some("Timed out waiting for PTY command result."),
                ),
            );
            state.handle_worker_error(
                &app,
                &execution.worker_id,
                format!(
                    "timed out waiting for PTY command result for `{}`",
                    execution.command
                ),
            );
        }
    });
}

fn spawn_terminal_runtime(
    app: AppHandle,
    session_id: String,
    title: Option<String>,
    cwd: String,
) -> Result<(TerminalRuntime, CommandSession), AppError> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| AppError::Message(format!("Failed to create PTY: {error}")))?;

    let mut cmd = CommandBuilder::new("powershell.exe");
    cmd.arg("-NoLogo");
    cmd.arg("-NoProfile");
    cmd.cwd(cwd.clone());

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|error| AppError::Message(format!("Failed to spawn PTY shell: {error}")))?;

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| AppError::Message(format!("Failed to clone PTY reader: {error}")))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| AppError::Message(format!("Failed to take PTY writer: {error}")))?;

    let output_session_id = session_id.clone();
    let app_for_output = app.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(size) => {
                    let data = String::from_utf8_lossy(&buf[..size]).to_string();
                    emit_terminal_output(&app_for_output, &output_session_id, &data);
                }
                Err(_) => break,
            }
        }
    });

    let session = CommandSession {
        id: session_id,
        title: title.unwrap_or_else(|| "Session".into()),
        cwd: cwd.clone(),
        shell: "powershell".into(),
        last_exit_code: None,
        lines: vec![
            format!("[proxy] PTY session booted in {cwd}\r\n"),
            "[proxy] commands sent here are evaluated before reaching the shell\r\n".into(),
        ],
        cols: 120,
        rows: 30,
    };

    Ok((
        TerminalRuntime {
            writer: Arc::new(Mutex::new(writer)),
            master: pair.master,
            _child: child,
        },
        session,
    ))
}

fn emit_terminal_output(app: &AppHandle, session_id: &str, data: &str) {
    let _ = app.emit_all(
        "terminal-output",
        TerminalOutputEvent {
            session_id: session_id.to_string(),
            data: data.to_string(),
        },
    );
}

fn emit_worker_output(app: &AppHandle, worker_id: &str, line: String) {
    let _ = app.emit_all(
        "worker-output",
        WorkerOutputEvent {
            worker_id: worker_id.to_string(),
            line,
        },
    );
}

fn openclaw_config_path() -> Option<PathBuf> {
    let home = env::var("USERPROFILE")
        .ok()
        .or_else(|| env::var("HOME").ok())?;
    Some(PathBuf::from(home).join(".openclaw").join("openclaw.json"))
}

fn enforce_openclaw_secure_auth_setting() -> Result<bool, AppError> {
    let Some(config_path) = openclaw_config_path() else {
        return Ok(false);
    };

    if !config_path.exists() {
        return Ok(false);
    }

    let config_text = fs::read_to_string(&config_path).map_err(|error| {
        AppError::Message(format!(
            "Failed to read OpenClaw config `{}`: {error}",
            config_path.display()
        ))
    })?;

    let mut config_value: serde_json::Value = serde_json::from_str(&config_text).map_err(|error| {
        AppError::Message(format!(
            "Failed to parse OpenClaw config `{}`: {error}",
            config_path.display()
        ))
    })?;

    let Some(root) = config_value.as_object_mut() else {
        return Err(AppError::Message(format!(
            "OpenClaw config `{}` is not a JSON object.",
            config_path.display()
        )));
    };

    let gateway_value = root
        .entry("gateway")
        .or_insert_with(|| serde_json::json!({}));
    if !gateway_value.is_object() {
        *gateway_value = serde_json::json!({});
    }
    let gateway = gateway_value.as_object_mut().expect("gateway object");

    let control_ui_value = gateway
        .entry("controlUi")
        .or_insert_with(|| serde_json::json!({}));
    if !control_ui_value.is_object() {
        *control_ui_value = serde_json::json!({});
    }
    let control_ui = control_ui_value
        .as_object_mut()
        .expect("controlUi object");

    let current_value = control_ui
        .get("allowInsecureAuth")
        .and_then(|value| value.as_bool());
    if current_value == Some(false) {
        return Ok(false);
    }

    control_ui.insert("allowInsecureAuth".into(), serde_json::Value::Bool(false));
    let updated = serde_json::to_string_pretty(&config_value).map_err(|error| {
        AppError::Message(format!(
            "Failed to serialize OpenClaw config `{}`: {error}",
            config_path.display()
        ))
    })?;
    fs::write(&config_path, format!("{updated}\n")).map_err(|error| {
        AppError::Message(format!(
            "Failed to write OpenClaw config `{}`: {error}",
            config_path.display()
        ))
    })?;
    Ok(true)
}

fn schedule_openclaw_config_hardening(app: AppHandle, worker_id: String) {
    thread::spawn(move || {
        for delay in [1_u64, 5, 15] {
            thread::sleep(Duration::from_secs(delay));
            match enforce_openclaw_secure_auth_setting() {
                Ok(true) => emit_worker_output(
                    &app,
                    &worker_id,
                    format!(
                        "[proxy] hardened OpenClaw config after startup check: set gateway.controlUi.allowInsecureAuth=false ({delay}s)"
                    ),
                ),
                Ok(false) => {}
                Err(error) => emit_worker_output(
                    &app,
                    &worker_id,
                    format!("[proxy] OpenClaw config hardening check failed: {error}"),
                ),
            }
        }
    });
}

fn parse_worker_envelope(line: &str) -> Option<ParsedWorkerEnvelope> {
    let trimmed = line.trim();
    if let Some(command) = trimmed.strip_prefix("PROXY_CMD ") {
        let command = command.trim();
        if command.is_empty() {
            return None;
        }
        return Some(ParsedWorkerEnvelope::Command(WorkerEnvelopeCommand {
            command: command.into(),
            cwd: None,
        }));
    }

    if let Some(json) = trimmed.strip_prefix("PROXY_JSON ") {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let kind = value.get("kind")?.as_str()?;
        if kind.eq_ignore_ascii_case("approval") || kind.eq_ignore_ascii_case("approve") {
            return Some(ParsedWorkerEnvelope::Rejected(
                "agents cannot approve blocked requests; human supervisor approval is required".into(),
            ));
        }
        if !kind.eq_ignore_ascii_case("command") {
            return Some(ParsedWorkerEnvelope::Rejected(format!(
                "unsupported worker envelope kind `{kind}`"
            )));
        }

        let command = value.get("command")?.as_str()?.trim();
        if command.is_empty() {
            return None;
        }

        let cwd = value
            .get("cwd")
            .and_then(|item| item.as_str())
            .map(|item| item.to_string());

        return Some(ParsedWorkerEnvelope::Command(WorkerEnvelopeCommand {
            command: command.into(),
            cwd,
        }));
    }

    None
}

fn spawn_worker_reader<T>(app: AppHandle, worker_id: String, reader: T, label: &'static str)
where
    T: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let line_value = line.trim_end_matches(['\r', '\n']);
                    if line_value.is_empty() {
                        continue;
                    }

                    if label == "stdout" {
                        if let Some(envelope) = parse_worker_envelope(line_value) {
                            match envelope {
                                ParsedWorkerEnvelope::Command(command) => {
                                    let state = app.state::<Mutex<ProxyTerminalState>>();
                                    let lock_result = state.lock();
                                    match lock_result {
                                        Ok(mut state) => {
                                            if let Err(error) = state
                                                .handle_worker_envelope_command(&app, &worker_id, command)
                                            {
                                                emit_worker_output(
                                                    &app,
                                                    &worker_id,
                                                    format!("[proxy] worker envelope failed: {error}"),
                                                );
                                            }
                                        }
                                        Err(_) => emit_worker_output(
                                            &app,
                                            &worker_id,
                                            "[proxy] worker envelope failed: application state lock was poisoned".into(),
                                        ),
                                    }
                                }
                                ParsedWorkerEnvelope::Rejected(message) => {
                                    emit_worker_output(
                                        &app,
                                        &worker_id,
                                        format!("[proxy] rejected worker envelope: {message}"),
                                    );
                                    let state = app.state::<Mutex<ProxyTerminalState>>();
                                    let lock_result = state.lock();
                                    if let Ok(mut state) = lock_result {
                                        state.audit.push(audit_event(
                                            "approval",
                                            "agent_envelope",
                                            Some("rejected".into()),
                                            format!(
                                                "Worker `{}` attempted a forbidden approval action. {message}",
                                                worker_id
                                            ),
                                            None,
                                            Some(worker_id.clone()),
                                        ));
                                        state.handle_worker_protocol_violation(
                                            &app,
                                            &worker_id,
                                            message,
                                        );
                                    }
                                }
                            }
                            continue;
                        }
                    }

                    let state = app.state::<Mutex<ProxyTerminalState>>();
                    if let Ok(mut state) = state.lock() {
                        state.handle_worker_status_line(&worker_id, line_value);
                    }

                    emit_worker_output(&app, &worker_id, format!("[{label}] {line_value}"));
                }
                Err(_) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ScopeDelta;

    fn pending_command(target: &str, request_id: &str) -> PendingApproval {
        PendingApproval {
            request: ActionRequest {
                id: request_id.into(),
                kind: ActionKind::Command,
                target: target.into(),
                command: Some(target.into()),
                cwd: Some("C:\\workspace".into()),
                args: None,
                rationale: Some("test".into()),
                worker_id: None,
                session_id: Some("session-1".into()),
            },
            decision: PolicyDecision {
                verdict: PolicyVerdict::Prompt,
                reason: "needs approval".into(),
                requires_approval: true,
                scope_delta: Some(ScopeDelta {
                    add_commands: vec![target.into()],
                    ..ScopeDelta::default()
                }),
            },
        }
    }

    #[test]
    fn update_policy_clears_pending_approvals_that_are_now_allowed() {
        let mut state = ProxyTerminalState::new();
        state.pending_approvals.push(pending_command("git status", "req-1"));
        state.pending_approvals.push(pending_command("curl https://example.com", "req-2"));

        let mut policy = state.policy.clone();
        policy.allow_commands.push("git".into());
        state.update_policy(policy);

        assert_eq!(state.pending_approvals.len(), 1);
        assert_eq!(state.pending_approvals[0].request.id, "req-2");
    }

    #[test]
    fn clearing_obsolete_approvals_removes_duplicate_requests_for_same_command() {
        let mut state = ProxyTerminalState::new();
        let request = ActionRequest {
            id: "req-1".into(),
            kind: ActionKind::Command,
            target: "git status".into(),
            command: Some("git status".into()),
            cwd: Some("C:\\workspace".into()),
            args: None,
            rationale: Some("test".into()),
            worker_id: None,
            session_id: Some("session-1".into()),
        };

        state.pending_approvals.push(pending_command("git status", "req-2"));
        state.pending_approvals.push(pending_command("git status", "req-3"));
        state.pending_approvals.push(pending_command("dir", "req-4"));

        let removed = state.clear_obsolete_approvals_for_request(&request);

        assert_eq!(removed, 2);
        assert_eq!(state.pending_approvals.len(), 1);
        assert_eq!(state.pending_approvals[0].request.id, "req-4");
    }

    #[test]
    fn parses_simple_worker_command_envelope() {
        let envelope = parse_worker_envelope("PROXY_CMD git status").expect("envelope");
        match envelope {
            ParsedWorkerEnvelope::Command(envelope) => {
                assert_eq!(envelope.command, "git status");
                assert_eq!(envelope.cwd, None);
            }
            ParsedWorkerEnvelope::Rejected(message) => panic!("unexpected rejection: {message}"),
        }
    }

    #[test]
    fn parses_json_worker_command_envelope() {
        let envelope = parse_worker_envelope(
            r#"PROXY_JSON {"kind":"command","command":"dir","cwd":"C:\\workspace"}"#,
        )
        .expect("json envelope");

        match envelope {
            ParsedWorkerEnvelope::Command(envelope) => {
                assert_eq!(envelope.command, "dir");
                assert_eq!(envelope.cwd.as_deref(), Some("C:\\workspace"));
            }
            ParsedWorkerEnvelope::Rejected(message) => panic!("unexpected rejection: {message}"),
        }
    }

    #[test]
    fn rejects_worker_approval_envelope() {
        let envelope =
            parse_worker_envelope(r#"PROXY_JSON {"kind":"approval","requestId":"abc"}"#)
                .expect("approval envelope");

        match envelope {
            ParsedWorkerEnvelope::Rejected(message) => {
                assert!(message.contains("agents cannot approve"));
            }
            ParsedWorkerEnvelope::Command(_) => panic!("approval envelope must be rejected"),
        }
    }

    #[test]
    fn applies_minimal_worker_sandbox_environment() {
        let worker = Worker {
            id: "worker-1".into(),
            name: "Agent".into(),
            adapter: "openclaw".into(),
            trust_level: crate::models::AgentTrustLevel::Untrusted,
            runtime_mode: crate::models::AgentRuntimeMode::BrokerOnly,
            compatibility: crate::models::AgentCompatibility::Unknown,
            capability_profile: crate::models::AgentCapabilityProfile {
                execution: crate::models::AgentCapabilitySetting::Brokered,
                filesystem: crate::models::AgentCapabilitySetting::Scoped,
                network: crate::models::AgentCapabilitySetting::Prompted,
                memory: crate::models::AgentCapabilitySetting::Isolated,
                delegation: crate::models::AgentCapabilitySetting::HumanOnly,
                control_plane: crate::models::AgentCapabilitySetting::Denied,
            },
            memory_mode: crate::models::AgentMemoryMode::Ephemeral,
            profile_id: None,
            status: WorkerStatus::Idle,
            scope_roots: vec!["C:\\workspace".into()],
            current_task: None,
            executable_path: None,
            args: Vec::new(),
            output_lines: Vec::new(),
        };
        let sandbox_dir = PathBuf::from("C:\\workspace\\.orc-agent-sandboxes\\worker-1");
        let mut command = Command::new("cmd.exe");
        command.env("SHOULD_BE_REMOVED", "1");
        command.env_clear();

        apply_worker_sandbox_env(&mut command, &worker, &sandbox_dir);

        let envs: Vec<(String, String)> = command
            .get_envs()
            .filter_map(|(key, value)| {
                Some((
                    key.to_string_lossy().to_string(),
                    value?.to_string_lossy().to_string(),
                ))
            })
            .collect();

        assert!(envs.iter().any(|(key, value)| key == "PATH" && value.contains(".orc-agent-sandboxes")));
        assert!(envs.iter().any(|(key, value)| key == "ORC_TERMINAL_COMMAND_PREFIX" && value == "PROXY_CMD"));
        assert!(envs.iter().any(|(key, value)| key == "ORC_TERMINAL_POLICY_ROOT" && value == "C:\\workspace"));
        assert!(!envs.iter().any(|(key, _)| key == "SHOULD_BE_REMOVED"));
    }

    #[test]
    fn persistent_approval_promotes_scope_delta_but_one_time_does_not() {
        let mut state = ProxyTerminalState::new();
        let decision = PolicyDecision {
            verdict: PolicyVerdict::Prompt,
            reason: "needs approval".into(),
            requires_approval: true,
            scope_delta: Some(ScopeDelta {
                add_commands: vec!["git status".into()],
                ..ScopeDelta::default()
            }),
        };
        let promoted =
            state.apply_approval_scope_delta_if_persistent(&decision, &ApprovalMode::OneTime);

        assert!(!promoted);
        assert!(
            !state
                .policy
                .allow_commands
                .iter()
                .any(|command| command.eq_ignore_ascii_case("git status"))
        );

        let mut state = ProxyTerminalState::new();
        let decision = PolicyDecision {
            verdict: PolicyVerdict::Prompt,
            reason: "needs approval".into(),
            requires_approval: true,
            scope_delta: Some(ScopeDelta {
                add_commands: vec!["git status".into()],
                ..ScopeDelta::default()
            }),
        };
        let promoted =
            state.apply_approval_scope_delta_if_persistent(&decision, &ApprovalMode::Persistent);

        assert!(promoted);
        assert!(
            state
                .policy
                .allow_commands
                .iter()
                .any(|command| command.eq_ignore_ascii_case("git status"))
        );
    }

    #[test]
    fn completed_openclaw_run_marks_task_completed_and_worker_idle() {
        let mut state = ProxyTerminalState::new();
        let worker_id = state.workers[0].id.clone();
        let task = agent::assign_task(
            state
                .workers
                .iter_mut()
                .find(|worker| worker.id == worker_id)
                .expect("worker"),
            "Inspect workspace".into(),
            "Run git status and report the result.".into(),
            TaskGuardrails {
                allow_shell: true,
                allow_network: false,
                allow_writes: false,
            },
        );
        state.tasks.push(task.clone());

        state.handle_worker_status_line(&worker_id, "adapter-status: run 123 completed with status ok");

        let worker = state
            .workers
            .iter()
            .find(|worker| worker.id == worker_id)
            .expect("worker after completion");
        assert_eq!(worker.status, WorkerStatus::Idle);
        assert!(worker.current_task.is_none());

        let task = state
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .expect("task after completion");
        assert_eq!(task.status, WorkerStatus::Completed);
    }
}
