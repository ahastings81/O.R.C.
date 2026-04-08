export type AccessLevel = "none" | "read" | "write" | "manage";
export type ActionKind = "command" | "file" | "network" | "app" | "mcp";
export type ApprovalMode = "one_time" | "session" | "persistent";
export type TerminalControl =
  | "ctrl_c"
  | "ctrl_d"
  | "clear_line"
  | "space"
  | "arrow_up"
  | "arrow_down"
  | "page_up"
  | "page_down"
  | "enter";
export type PolicyVerdict = "allow" | "deny" | "prompt";
export type WorkerStatus = "idle" | "running" | "paused" | "completed" | "failed";
export type ProtectionState = "active" | "available" | "optional" | "unsupported" | "degraded";
export type AgentTrustLevel = "untrusted";
export type AgentRuntimeMode = "broker_only";
export type AgentCompatibility = "unknown" | "broker_compatible";
export type AgentCapabilitySetting =
  | "brokered"
  | "scoped"
  | "prompted"
  | "human_only"
  | "denied"
  | "isolated";
export type AgentMemoryMode = "ephemeral" | "task_scoped" | "agent_scoped" | "persistent";
export type DelegationMode = "deny" | "prompt" | "allow";

export interface FileRule {
  root: string;
  access: AccessLevel;
}

export interface MpcToolRule {
  server: string;
  tools: string[];
}

export interface SessionPolicy {
  name: string;
  roots: string[];
  fileRules: FileRule[];
  allowCommands: string[];
  allowApps: string[];
  allowDomains: string[];
  mcp: MpcToolRule[];
  elevatedCommands: string[];
  auditRedactions: string[];
  defaultMemoryMode: AgentMemoryMode;
  delegationMode: DelegationMode;
  delegationMaxDepth: number;
}

export interface ScopeDelta {
  addRoots?: string[];
  addDomains?: string[];
  addCommands?: string[];
}

export interface ActionRequest {
  id: string;
  kind: ActionKind;
  target: string;
  command?: string;
  cwd?: string;
  args?: string[];
  rationale?: string;
  workerId?: string;
  sessionId?: string;
}

export interface PolicyDecision {
  verdict: PolicyVerdict;
  reason: string;
  requiresApproval: boolean;
  scopeDelta?: ScopeDelta;
}

export interface ApprovalGrant {
  requestId: string;
  mode: ApprovalMode;
  createdAt: string;
}

export interface AuditEvent {
  id: string;
  timestamp: string;
  category: string;
  source: string;
  outcome?: string;
  message: string;
  requestId?: string;
  workerId?: string;
}

export interface PendingApproval {
  request: ActionRequest;
  decision: PolicyDecision;
}

export interface AgentProfile {
  id: string;
  name: string;
  builtIn: boolean;
  allowCommands: string[];
  allowDomains: string[];
  memoryMode: AgentMemoryMode;
  delegationMode: DelegationMode;
  delegationMaxDepth: number;
  defaultGuardrails: TaskGuardrails;
}

export interface CommandSession {
  id: string;
  title: string;
  cwd: string;
  shell: string;
  lastExitCode?: number | null;
  lines: string[];
  cols: number;
  rows: number;
}

export interface Worker {
  id: string;
  name: string;
  adapter: "openclaw" | "nemoclaw";
  trustLevel: AgentTrustLevel;
  runtimeMode: AgentRuntimeMode;
  compatibility: AgentCompatibility;
  capabilityProfile: AgentCapabilityProfile;
  memoryMode: AgentMemoryMode;
  profileId?: string | null;
  status: WorkerStatus;
  scopeRoots: string[];
  currentTask?: string;
  executablePath?: string | null;
  args: string[];
  outputLines: string[];
}

export interface AgentCapabilityProfile {
  execution: AgentCapabilitySetting;
  filesystem: AgentCapabilitySetting;
  network: AgentCapabilitySetting;
  memory: AgentCapabilitySetting;
  delegation: AgentCapabilitySetting;
  controlPlane: AgentCapabilitySetting;
}

export interface ProtectionStatus {
  id: string;
  label: string;
  state: ProtectionState;
  detail: string;
}

export interface SupervisorTask {
  id: string;
  title: string;
  assignedWorkerId?: string;
  status: WorkerStatus;
  summary: string;
  guardrails: TaskGuardrails;
}

export interface TaskGuardrails {
  allowShell: boolean;
  allowNetwork: boolean;
  allowWrites: boolean;
}

export interface DashboardState {
  policy: SessionPolicy;
  profiles: AgentProfile[];
  protections: ProtectionStatus[];
  audit: AuditEvent[];
  pendingApprovals: PendingApproval[];
  sessions: CommandSession[];
  workers: Worker[];
  tasks: SupervisorTask[];
}

export interface TerminalOutputEvent {
  sessionId: string;
  data: string;
}

export interface TerminalExitEvent {
  sessionId: string;
  exitCode: number | null;
}

export interface WorkerOutputEvent {
  workerId: string;
  line: string;
}
