use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    None,
    Read,
    Write,
    Manage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Command,
    File,
    Network,
    App,
    Mcp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    OneTime,
    Session,
    Persistent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict {
    Allow,
    Deny,
    Prompt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Idle,
    Running,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRule {
    pub root: String,
    pub access: AccessLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolRule {
    pub server: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPolicy {
    pub name: String,
    pub roots: Vec<String>,
    pub file_rules: Vec<FileRule>,
    pub allow_commands: Vec<String>,
    pub allow_apps: Vec<String>,
    pub allow_domains: Vec<String>,
    pub mcp: Vec<McpToolRule>,
    pub elevated_commands: Vec<String>,
    pub audit_redactions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScopeDelta {
    pub add_roots: Vec<String>,
    pub add_domains: Vec<String>,
    pub add_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequest {
    pub id: String,
    pub kind: ActionKind,
    pub target: String,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub args: Option<Vec<String>>,
    pub rationale: Option<String>,
    pub worker_id: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyDecision {
    pub verdict: PolicyVerdict,
    pub reason: String,
    pub requires_approval: bool,
    pub scope_delta: Option<ScopeDelta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalGrant {
    pub request_id: String,
    pub mode: ApprovalMode,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub category: String,
    pub message: String,
    pub request_id: Option<String>,
    pub worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub request: ActionRequest,
    pub decision: PolicyDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSession {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub shell: String,
    pub last_exit_code: Option<i32>,
    pub lines: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worker {
    pub id: String,
    pub name: String,
    pub adapter: String,
    pub status: WorkerStatus,
    pub scope_roots: Vec<String>,
    pub current_task: Option<String>,
    pub executable_path: Option<String>,
    pub args: Vec<String>,
    pub output_lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorTask {
    pub id: String,
    pub title: String,
    pub assigned_worker_id: Option<String>,
    pub status: WorkerStatus,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardState {
    pub policy: SessionPolicy,
    pub audit: Vec<AuditEvent>,
    pub pending_approvals: Vec<PendingApproval>,
    pub sessions: Vec<CommandSession>,
    pub workers: Vec<Worker>,
    pub tasks: Vec<SupervisorTask>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputEvent {
    pub session_id: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExitEvent {
    pub session_id: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerOutputEvent {
    pub worker_id: String,
    pub line: String,
}
