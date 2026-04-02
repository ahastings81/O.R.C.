import { invoke } from "@tauri-apps/api/tauri";
import type { ApprovalMode, DashboardState, SessionPolicy } from "./types";

export async function bootstrap(): Promise<DashboardState> {
  return invoke("bootstrap_state");
}

export async function createCommandSession(title?: string): Promise<DashboardState> {
  return invoke("create_command_session", { title });
}

export async function sendTerminalInput(sessionId: string, input: string): Promise<DashboardState> {
  return invoke("send_terminal_input", { session_id: sessionId, input });
}

export async function resizeTerminal(sessionId: string, cols: number, rows: number): Promise<void> {
  return invoke("resize_terminal", { session_id: sessionId, cols, rows });
}

export async function approveRequest(requestId: string, mode: ApprovalMode): Promise<DashboardState> {
  return invoke("approve_request", { request_id: requestId, mode });
}

export async function denyRequest(requestId: string): Promise<DashboardState> {
  return invoke("deny_request", { request_id: requestId });
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
  args?: string[]
): Promise<DashboardState> {
  return invoke("create_worker", {
    adapter,
    name,
    executable_path: executablePath,
    args
  });
}

export async function assignTask(workerId: string, title: string, summary: string): Promise<DashboardState> {
  return invoke("assign_task", { worker_id: workerId, title, summary });
}

export async function setWorkerStatus(
  workerId: string,
  status: "idle" | "running" | "paused" | "completed" | "failed"
): Promise<DashboardState> {
  return invoke("set_worker_status", { worker_id: workerId, status });
}
