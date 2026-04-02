import { FormEvent, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  approveRequest,
  assignTask,
  bootstrap,
  createCommandSession,
  createWorker,
  denyRequest,
  exportAudit,
  sendTerminalInput,
  setWorkerStatus,
  updatePolicy
} from "./api";
import { TerminalView } from "./TerminalView";
import type {
  ApprovalMode,
  DashboardState,
  SessionPolicy,
  TerminalExitEvent,
  TerminalOutputEvent,
  WorkerOutputEvent,
  WorkerStatus
} from "./types";

const EMPTY_COMMAND = "";

export function App() {
  const [state, setState] = useState<DashboardState | null>(null);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [command, setCommand] = useState(EMPTY_COMMAND);
  const [taskTitle, setTaskTitle] = useState("Inspect workspace");
  const [taskSummary, setTaskSummary] = useState("Review scoped files before touching any command.");
  const [workerName, setWorkerName] = useState("Worker 1");
  const [workerPath, setWorkerPath] = useState("");
  const [workerArgs, setWorkerArgs] = useState("");
  const [selectedWorker, setSelectedWorker] = useState<string>("");
  const [exportedAudit, setExportedAudit] = useState<string>("");

  useEffect(() => {
    bootstrap().then((next) => {
      setState((current) => mergeDashboardState(current, next));
      setActiveSessionId(next.sessions[0]?.id ?? null);
      setSelectedWorker(next.workers[0]?.id ?? "");
    });
  }, []);

  useEffect(() => {
    const stopTerminalOutput = listen<TerminalOutputEvent>("terminal-output", (event) => {
      setState((current) => {
        if (!current) {
          return current;
        }

        return {
          ...current,
          sessions: current.sessions.map((session) =>
            session.id === event.payload.sessionId
              ? {
                  ...session,
                  lines: [...session.lines, event.payload.data].slice(-400)
                }
              : session
          )
        };
      });
    });

    const stopTerminalExit = listen<TerminalExitEvent>("terminal-exit", (event) => {
      setState((current) => {
        if (!current) {
          return current;
        }

        return {
          ...current,
          sessions: current.sessions.map((session) =>
            session.id === event.payload.sessionId
              ? {
                  ...session,
                  lastExitCode: event.payload.exitCode,
                  lines: [
                    ...session.lines,
                    `\r\n[proxy] terminal exited with code ${event.payload.exitCode ?? -1}\r\n`
                  ]
                }
              : session
          )
        };
      });
    });

    const stop = listen<WorkerOutputEvent>("worker-output", (event) => {
      setState((current) => {
        if (!current) {
          return current;
        }

        return {
          ...current,
          workers: current.workers.map((worker) =>
            worker.id === event.payload.workerId
              ? {
                  ...worker,
                  outputLines: [...worker.outputLines, event.payload.line].slice(-120)
                }
              : worker
          )
        };
      });
    });

    return () => {
      void stopTerminalOutput.then((unlisten) => unlisten());
      void stopTerminalExit.then((unlisten) => unlisten());
      void stop.then((unlisten) => unlisten());
    };
  }, []);

  const activeSession = useMemo(() => {
    if (!state || !activeSessionId) {
      return null;
    }

    return state.sessions.find((session) => session.id === activeSessionId) ?? null;
  }, [state, activeSessionId]);

  if (!state) {
    return <div className="loading">Booting proxy terminal...</div>;
  }

  const dashboard = state;

  async function refresh<T>(fn: Promise<T>) {
    const next = (await fn) as DashboardState;
    setState((current) => mergeDashboardState(current, next));
    if (!activeSessionId && next.sessions[0]) {
      setActiveSessionId(next.sessions[0].id);
    }
    if (!selectedWorker && next.workers[0]) {
      setSelectedWorker(next.workers[0].id);
    }
  }

  async function submitCommand(event: FormEvent) {
    event.preventDefault();
    if (!activeSessionId || !command.trim()) {
      return;
    }

    await refresh(sendTerminalInput(activeSessionId, command));
    setCommand(EMPTY_COMMAND);
  }

  async function addWorker(adapter: "openclaw" | "nemoclaw") {
    const args = workerArgs
      .split(" ")
      .map((value) => value.trim())
      .filter(Boolean);

    await refresh(createWorker(adapter, workerName, workerPath || undefined, args));
    setWorkerName(`${adapter === "openclaw" ? "OpenClaw" : "NemoClaw"} worker`);
  }

  async function createTask() {
    if (!selectedWorker) {
      return;
    }

    await refresh(assignTask(selectedWorker, taskTitle, taskSummary));
  }

  async function updateWorkerState(workerId: string, status: WorkerStatus) {
    await refresh(setWorkerStatus(workerId, status));
  }

  async function handleAuditExport() {
    const content = await exportAudit();
    setExportedAudit(content);
  }

  async function mutatePolicy(mutator: (current: SessionPolicy) => SessionPolicy) {
    const nextPolicy = mutator(dashboard.policy);
    await refresh(updatePolicy(nextPolicy));
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <section className="panel">
          <div className="panel-header">
            <h2>Proxy Terminal</h2>
            <button onClick={() => refresh(createCommandSession(`Session ${dashboard.sessions.length + 1}`))}>
              New Session
            </button>
          </div>
          <p className="muted">
            PTY-backed terminal sessions, deny-by-default policy checks, and supervised external agent adapters.
          </p>
        </section>

        <section className="panel">
          <div className="panel-header">
            <h3>Policy</h3>
            <span className="badge deny">Deny by default</span>
          </div>
          <div className="chips">
            {dashboard.policy.roots.map((root) => (
              <span className="chip" key={root}>
                {root}
              </span>
            ))}
          </div>
          <div className="list-block">
            <strong>Allowed commands</strong>
            {dashboard.policy.allowCommands.map((item) => (
              <span className="line" key={item}>
                {item}
              </span>
            ))}
          </div>
          <div className="list-block">
            <strong>Network allowlist</strong>
            {dashboard.policy.allowDomains.map((item) => (
              <span className="line" key={item}>
                {item}
              </span>
            ))}
          </div>
          <div className="panel-actions">
            <button
              onClick={() =>
                mutatePolicy((current) => ({
                  ...current,
                  allowDomains: Array.from(new Set([...current.allowDomains, "api.openai.com"]))
                }))
              }
            >
              Allow `api.openai.com`
            </button>
            <button
              onClick={() =>
                mutatePolicy((current) => ({
                  ...current,
                  allowCommands: Array.from(new Set([...current.allowCommands, "git"]))
                }))
              }
            >
              Allow `git`
            </button>
          </div>
        </section>

        <section className="panel">
          <div className="panel-header">
            <h3>Approvals</h3>
            <span className="badge neutral">{dashboard.pendingApprovals.length}</span>
          </div>
          {dashboard.pendingApprovals.length === 0 ? (
            <p className="muted">No requests waiting for review.</p>
          ) : (
            dashboard.pendingApprovals.map((approval) => (
              <div className="approval-card" key={approval.request.id}>
                <strong>{approval.request.kind.toUpperCase()}</strong>
                <p>{approval.request.target}</p>
                <span className="muted">{approval.decision.reason}</span>
                <div className="approval-actions">
                  {(["one_time", "session", "persistent"] as ApprovalMode[]).map((mode) => (
                    <button key={mode} onClick={() => refresh(approveRequest(approval.request.id, mode))}>
                      {mode.replace("_", " ")}
                    </button>
                  ))}
                  <button className="danger" onClick={() => refresh(denyRequest(approval.request.id))}>
                    Deny
                  </button>
                </div>
              </div>
            ))
          )}
        </section>
      </aside>

      <main className="main-pane">
        <section className="panel terminal-panel">
          <div className="panel-header">
            <div>
              <h3>Terminal</h3>
              <p className="muted">Shell runs inside a PTY; each line you send is policy-checked before entering it.</p>
            </div>
            <div className="tab-strip">
              {dashboard.sessions.map((session) => (
                <button
                  key={session.id}
                  className={session.id === activeSessionId ? "tab active" : "tab"}
                  onClick={() => setActiveSessionId(session.id)}
                >
                  {session.title}
                </button>
              ))}
            </div>
          </div>

          <TerminalView session={activeSession} />

          <form className="command-form" onSubmit={submitCommand}>
            <label htmlFor="command-input">$</label>
            <input
              id="command-input"
              value={command}
              onChange={(event) => setCommand(event.target.value)}
              placeholder="Send a command through the PTY, for example `dir` or `git status`."
            />
            <button type="submit">Send</button>
          </form>
        </section>

        <section className="grid">
          <section className="panel">
            <div className="panel-header">
              <h3>Supervisor</h3>
              <span className="badge neutral">{dashboard.workers.length} workers</span>
            </div>
            <div className="worker-creation">
              <input value={workerName} onChange={(event) => setWorkerName(event.target.value)} placeholder="Worker name" />
              <input
                value={workerPath}
                onChange={(event) => setWorkerPath(event.target.value)}
                placeholder="Path to openclaw or nemoclaw executable"
              />
              <input
                value={workerArgs}
                onChange={(event) => setWorkerArgs(event.target.value)}
                placeholder="Optional args, space separated"
              />
              <div className="inline-actions">
                <button onClick={() => addWorker("openclaw")}>Add OpenClaw</button>
                <button onClick={() => addWorker("nemoclaw")}>Add NemoClaw</button>
              </div>
            </div>

            <div className="worker-list">
              {dashboard.workers.map((worker) => (
                <div className="worker-card" key={worker.id}>
                  <div className="panel-header">
                    <strong>{worker.name}</strong>
                    <span className={`badge ${worker.status}`}>{worker.status}</span>
                  </div>
                  <span className="muted">{worker.adapter}</span>
                  <span className="line">{worker.executablePath || "No executable configured yet"}</span>
                  <div className="chips">
                    {worker.scopeRoots.map((root) => (
                      <span className="chip" key={root}>
                        {root}
                      </span>
                    ))}
                  </div>
                  <div className="inline-actions">
                    <button onClick={() => updateWorkerState(worker.id, "running")}>Run</button>
                    <button onClick={() => updateWorkerState(worker.id, "paused")}>Pause</button>
                    <button onClick={() => updateWorkerState(worker.id, "completed")}>Stop</button>
                  </div>
                  <div className="worker-output">
                    {worker.outputLines.length === 0 ? (
                      <span className="muted">No adapter output yet.</span>
                    ) : (
                      worker.outputLines.slice(-14).map((line, index) => (
                        <span className="line" key={`${worker.id}-${index}`}>
                          {line}
                        </span>
                      ))
                    )}
                  </div>
                </div>
              ))}
            </div>
          </section>

          <section className="panel">
            <div className="panel-header">
              <h3>Tasks</h3>
              <span className="badge neutral">{dashboard.tasks.length}</span>
            </div>
            <select value={selectedWorker} onChange={(event) => setSelectedWorker(event.target.value)}>
              {dashboard.workers.map((worker) => (
                <option key={worker.id} value={worker.id}>
                  {worker.name}
                </option>
              ))}
            </select>
            <input value={taskTitle} onChange={(event) => setTaskTitle(event.target.value)} />
            <textarea value={taskSummary} onChange={(event) => setTaskSummary(event.target.value)} rows={4} />
            <button onClick={createTask}>Assign task</button>
            <div className="task-list">
              {dashboard.tasks.map((task) => (
                <div className="task-card" key={task.id}>
                  <strong>{task.title}</strong>
                  <span className="muted">{task.summary}</span>
                  <span className="line">Worker: {task.assignedWorkerId ?? "unassigned"}</span>
                  <span className={`badge ${task.status}`}>{task.status}</span>
                </div>
              ))}
            </div>
          </section>

          <section className="panel">
            <div className="panel-header">
              <h3>Audit</h3>
              <button onClick={handleAuditExport}>Export</button>
            </div>
            <div className="audit-list">
              {dashboard.audit.slice(0, 10).map((event) => (
                <div className="audit-item" key={event.id}>
                  <strong>{event.category}</strong>
                  <span>{event.message}</span>
                  <span className="muted">{event.timestamp}</span>
                </div>
              ))}
            </div>
            {exportedAudit ? <textarea className="export-box" readOnly value={exportedAudit} rows={8} /> : null}
          </section>
        </section>
      </main>
    </div>
  );
}

function mergeDashboardState(current: DashboardState | null, next: DashboardState): DashboardState {
  if (!current) {
    return next;
  }

  return {
    ...next,
    sessions: next.sessions.map((session) => {
      const existing = current.sessions.find((item) => item.id === session.id);
      return existing ? { ...session, lines: existing.lines } : session;
    }),
    workers: next.workers.map((worker) => {
      const existing = current.workers.find((item) => item.id === worker.id);
      return existing ? { ...worker, outputLines: existing.outputLines } : worker;
    })
  };
}
