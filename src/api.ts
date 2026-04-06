import { invoke } from "@tauri-apps/api/tauri";
import type {
  AgentMemoryMode,
  ApprovalMode,
  DashboardState,
  DelegationMode,
  SessionPolicy,
  TaskGuardrails
} from "./types";

export async function bootstrap(): Promise<DashboardState> {
  return invoke("bootstrap_state");
}

export async function createCommandSession(title?: string): Promise<DashboardState> {
  return invoke("create_command_session", { title });
}

export async function sendTerminalInput(sessionId: string, input: string): Promise<DashboardState> {
  return invoke("send_terminal_input", { sessionId, input });
}

export async function resizeTerminal(sessionId: string, cols: number, rows: number): Promise<void> {
  return invoke("resize_terminal", { sessionId, cols, rows });
}

export async function approveRequest(requestId: string, mode: ApprovalMode): Promise<DashboardState> {
  return invoke("approve_request", { requestId, mode });
}

export async function denyRequest(requestId: string): Promise<DashboardState> {
  return invoke("deny_request", { requestId });
}

export async function denyRequestAndStop(requestId: string): Promise<DashboardState> {
  return invoke("deny_request_and_stop", { requestId });
}

export async function exportAudit(): Promise<string> {
  return invoke("export_audit_log");
}

export async function updatePolicy(policy: SessionPolicy): Promise<DashboardState> {
  return invoke("update_policy", { policy });
}

export async function createWorker(
  adapter: "openclaw" | "nemoclaw",
  name: string,
  executablePath?: string,
  args?: string[],
  memoryMode?: AgentMemoryMode,
  profileId?: string
): Promise<DashboardState> {
  return invoke("create_worker", {
    adapter,
    name,
    executablePath,
    args,
    memoryMode,
    profileId
  });
}

export async function assignTask(
  workerId: string,
  title: string,
  summary: string,
  guardrails: TaskGuardrails
): Promise<DashboardState> {
  return invoke("assign_task", { workerId, title, summary, guardrails });
}

export async function saveAgentProfile(
  name: string,
  allowCommands: string[],
  allowDomains: string[],
  memoryMode: AgentMemoryMode,
  delegationMode: DelegationMode,
  delegationMaxDepth: number,
  defaultGuardrails: TaskGuardrails
): Promise<DashboardState> {
  return invoke("save_agent_profile", {
    name,
    allowCommands,
    allowDomains,
    memoryMode,
    delegationMode,
    delegationMaxDepth,
    defaultGuardrails
  });
}

export async function applyAgentProfile(workerId: string, profileId: string): Promise<DashboardState> {
  return invoke("apply_agent_profile", { workerId, profileId });
}

export async function setWorkerStatus(
  workerId: string,
  status: "idle" | "running" | "paused" | "completed" | "failed"
): Promise<DashboardState> {
  return invoke("set_worker_status", { workerId, status });
}
