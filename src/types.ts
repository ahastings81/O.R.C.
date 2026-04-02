export type AccessLevel = "none" | "read" | "write" | "manage";
export type ActionKind = "command" | "file" | "network" | "app" | "mcp";
export type ApprovalMode = "one_time" | "session" | "persistent";
export type PolicyVerdict = "allow" | "deny" | "prompt";
export type WorkerStatus = "idle" | "running" | "paused" | "completed" | "failed";

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
  message: string;
  requestId?: string;
  workerId?: string;
}

export interface PendingApproval {
  request: ActionRequest;
  decision: PolicyDecision;
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
  status: WorkerStatus;
  scopeRoots: string[];
  currentTask?: string;
  executablePath?: string | null;
  args: string[];
  outputLines: string[];
}

export interface SupervisorTask {
  id: string;
  title: string;
  assignedWorkerId?: string;
  status: WorkerStatus;
  summary: string;
}

export interface DashboardState {
  policy: SessionPolicy;
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
