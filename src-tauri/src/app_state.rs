use std::{
    collections::HashMap,
    env,
    io::{Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};

use chrono::Utc;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tauri::{AppHandle, Manager};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    agent,
    audit::audit_event,
    models::{
        ActionKind, ActionRequest, ApprovalGrant, ApprovalMode, CommandSession, DashboardState,
        FileRule, McpToolRule, PendingApproval, PolicyDecision, PolicyVerdict, SessionPolicy,
        SupervisorTask, TerminalOutputEvent, Worker, WorkerOutputEvent, WorkerStatus,
    },
    policy::evaluate_request,
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
}

pub struct ProxyTerminalState {
    pub policy: SessionPolicy,
    pub audit: Vec<crate::models::AuditEvent>,
    pub pending_approvals: Vec<PendingApproval>,
    pub grants: Vec<ApprovalGrant>,
    pub sessions: Vec<CommandSession>,
    pub workers: Vec<Worker>,
    pub tasks: Vec<SupervisorTask>,
    terminal_runtimes: HashMap<String, TerminalRuntime>,
    worker_runtimes: HashMap<String, WorkerRuntime>,
}

impl ProxyTerminalState {
    pub fn new() -> Self {
        let cwd = env::current_dir()
            .ok()
            .map(|dir| dir.to_string_lossy().to_string())
            .unwrap_or_else(|| "C:\\".into());

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
                "type".into(),
            ],
            allow_apps: vec!["code".into()],
            allow_domains: vec!["localhost".into()],
            mcp: vec![McpToolRule {
                server: "local://filesystem".into(),
                tools: vec!["read".into(), "list".into()],
            }],
            elevated_commands: vec!["type".into()],
            audit_redactions: vec!["OPENAI_API_KEY".into()],
        };

        let initial_worker = agent::create_worker(
            "Supervisor worker".into(),
            "openclaw".into(),
            cwd,
            None,
            Vec::new(),
        );

        let mut state = Self {
            policy,
            audit: Vec::new(),
            pending_approvals: Vec::new(),
            grants: Vec::new(),
            sessions: Vec::new(),
            workers: vec![initial_worker],
            tasks: Vec::new(),
            terminal_runtimes: HashMap::new(),
            worker_runtimes: HashMap::new(),
        };

        state.audit.push(audit_event(
            "session",
            "Proxy Terminal booted with default local policy.",
            None,
            None,
        ));

        state
    }

    pub fn bootstrap(&mut self, app: &AppHandle) -> Result<DashboardState, AppError> {
        if self.sessions.is_empty() {
            self.create_command_session(app, Some("Session 1".into()))?;
        }

        Ok(self.snapshot())
    }

    pub fn snapshot(&self) -> DashboardState {
        DashboardState {
            policy: self.policy.clone(),
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
            format!("{} -> {:?}", request.target, decision.verdict),
            Some(request.id.clone()),
            None,
        ));

        match decision.verdict {
            PolicyVerdict::Allow => {
                self.write_to_terminal(session_id, &input)?;
                self.audit.push(audit_event(
                    "command",
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
                self.audit
                    .push(audit_event("command", "PTY input denied.", None, None));
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
        self.apply_scope_delta(pending.decision.clone());
        self.grants.push(ApprovalGrant {
            request_id: pending.request.id.clone(),
            mode: mode.clone(),
            created_at: Utc::now(),
        });

        if let Some(session_id) = &pending.request.session_id {
            if let Some(command) = &pending.request.command {
                self.write_to_terminal(session_id, command)?;
                emit_terminal_output(app, session_id, "\r\n[proxy] approval granted\r\n");
            }
        }

        self.audit.push(audit_event(
            "approval",
            format!(
                "Approved `{}` with mode {:?}.",
                pending.request.target, mode
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
        let index = self
            .pending_approvals
            .iter()
            .position(|pending| pending.request.id == request_id)
            .ok_or_else(|| AppError::Message("Approval request was not found.".into()))?;

        let pending = self.pending_approvals.remove(index);
        if let Some(session_id) = &pending.request.session_id {
            emit_terminal_output(app, session_id, "\r\n[proxy] approval denied\r\n");
        }
        self.audit.push(audit_event(
            "approval",
            format!("Denied `{}`.", pending.request.target),
            Some(pending.request.id),
            pending.request.worker_id,
        ));
        Ok(self.snapshot())
    }

    pub fn export_audit_log(&self) -> Result<String, AppError> {
        serde_json::to_string_pretty(&self.audit)
            .map_err(|error| AppError::Message(format!("Failed to serialize audit log: {error}")))
    }

    pub fn update_policy(&mut self, policy: SessionPolicy) -> DashboardState {
        self.policy = policy;
        self.audit
            .push(audit_event("policy", "Updated session policy.", None, None));
        self.snapshot()
    }

    pub fn create_worker(
        &mut self,
        adapter: String,
        name: String,
        executable_path: Option<String>,
        args: Vec<String>,
    ) -> DashboardState {
        let root = self
            .policy
            .roots
            .first()
            .cloned()
            .unwrap_or_else(|| "C:\\".into());
        let worker = agent::create_worker(name, adapter.clone(), root, executable_path, args);
        self.audit.push(audit_event(
            "worker",
            format!("Created {adapter} worker `{}`.", worker.id),
            None,
            Some(worker.id.clone()),
        ));
        self.workers.push(worker);
        self.snapshot()
    }

    pub fn assign_task(
        &mut self,
        app: &AppHandle,
        worker_id: &str,
        title: String,
        summary: String,
    ) -> Result<DashboardState, AppError> {
        let worker = self
            .workers
            .iter_mut()
            .find(|worker| worker.id == worker_id)
            .ok_or_else(|| AppError::Message("Worker was not found.".into()))?;

        let task = agent::assign_task(worker, title, summary);
        if let Some(runtime) = self.worker_runtimes.get(&worker.id) {
            let envelope = format!(
                "TASK {}\nTITLE: {}\nSUMMARY: {}\n\n",
                task.id, task.title, task.summary
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
                "[proxy] worker is not running yet; task queued in supervisor state".into(),
            );
        }

        self.audit.push(audit_event(
            "supervisor",
            format!("Assigned task `{}` to `{}`.", task.title, worker.name),
            Some(task.id.clone()),
            Some(worker.id.clone()),
        ));
        self.tasks.push(task);
        Ok(self.snapshot())
    }

    pub fn set_worker_status(
        &mut self,
        app: &AppHandle,
        worker_id: &str,
        status: WorkerStatus,
    ) -> Result<DashboardState, AppError> {
        match status {
            WorkerStatus::Running => self.start_worker(app, worker_id)?,
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
            format!("Worker `{}` moved to {:?}.", worker.name, status),
            None,
            Some(worker.id.clone()),
        ));
        Ok(self.snapshot())
    }

    fn start_worker(&mut self, app: &AppHandle, worker_id: &str) -> Result<(), AppError> {
        if self.worker_runtimes.contains_key(worker_id) {
            return Ok(());
        }

        let worker = self
            .workers
            .iter_mut()
            .find(|worker| worker.id == worker_id)
            .ok_or_else(|| AppError::Message("Worker was not found.".into()))?;

        let executable = worker
            .executable_path
            .clone()
            .ok_or_else(|| AppError::Message("Worker has no executable path configured.".into()))?;

        let mut child = Command::new(&executable)
            .args(&worker.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                AppError::Message(format!("Failed to start worker process: {error}"))
            })?;

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

        spawn_worker_reader(app.clone(), worker.id.clone(), stdout, "stdout");
        spawn_worker_reader(app.clone(), worker.id.clone(), stderr, "stderr");

        self.worker_runtimes.insert(
            worker.id.clone(),
            WorkerRuntime {
                stdin: Arc::new(Mutex::new(stdin)),
                child,
            },
        );

        emit_worker_output(
            app,
            &worker.id,
            format!(
                "[proxy] started {} adapter at {}",
                worker.adapter, executable
            ),
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

    fn write_to_terminal(&mut self, session_id: &str, input: &str) -> Result<(), AppError> {
        let runtime = self
            .terminal_runtimes
            .get_mut(session_id)
            .ok_or_else(|| AppError::Message("Terminal session runtime was not found.".into()))?;

        let mut writer = runtime
            .writer
            .lock()
            .map_err(|_| AppError::Message("PTY writer lock failed.".into()))?;
        writer
            .write_all(format!("{input}\r").as_bytes())
            .map_err(|error| AppError::Message(format!("Failed to write to PTY: {error}")))?;
        writer
            .flush()
            .map_err(|error| AppError::Message(format!("Failed to flush PTY input: {error}")))?;
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

fn spawn_worker_reader<T>(app: AppHandle, worker_id: String, mut reader: T, label: &'static str)
where
    T: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(size) => {
                    let chunk = String::from_utf8_lossy(&buf[..size]);
                    for line in chunk.lines() {
                        emit_worker_output(&app, &worker_id, format!("[{label}] {line}"));
                    }
                }
                Err(_) => break,
            }
        }
    });
}
