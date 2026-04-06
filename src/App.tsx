import { FormEvent, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  applyAgentProfile,
  approveRequest,
  assignTask,
  bootstrap,
  createCommandSession,
  createWorker,
  denyRequest,
  denyRequestAndStop,
  exportAudit,
  saveAgentProfile,
  sendTerminalInput,
  setWorkerStatus,
  updatePolicy
} from "./api";
import { COMMAND_LIBRARY, type LibraryItem } from "./commandLibrary";
import { TerminalView } from "./TerminalView";
import type {
  AgentProfile,
  AgentMemoryMode,
  ApprovalMode,
  AuditEvent,
  DashboardState,
  PendingApproval,
  SessionPolicy,
  SupervisorTask,
  TaskGuardrails,
  TerminalExitEvent,
  TerminalOutputEvent,
  Worker,
  WorkerOutputEvent,
  WorkerStatus
} from "./types";

const EMPTY_COMMAND = "";

type ViewId =
  | "overview"
  | "agents"
  | "approvals"
  | "tasks"
  | "profiles"
  | "policy"
  | "audit"
  | "protections";

const VIEW_LABELS: Record<ViewId, string> = {
  overview: "Overview",
  agents: "Agents",
  approvals: "Approvals",
  tasks: "Tasks",
  profiles: "Profiles",
  policy: "Policy",
  audit: "Audit",
  protections: "Protections"
};

export function App() {
  const [state, setState] = useState<DashboardState | null>(null);
  const [currentView, setCurrentView] = useState<ViewId>("overview");
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [selectedAgentId, setSelectedAgentId] = useState<string>("");
  const [selectedApprovalId, setSelectedApprovalId] = useState<string>("");
  const [selectedProtectionId, setSelectedProtectionId] = useState<string>("");
  const [selectedTaskId, setSelectedTaskId] = useState<string>("");
  const [selectedAuditId, setSelectedAuditId] = useState<string>("");
  const [command, setCommand] = useState(EMPTY_COMMAND);
  const [commandStatus, setCommandStatus] = useState<string>("");
  const [policyStatus, setPolicyStatus] = useState<string>("");
  const [workerStatusMessage, setWorkerStatusMessage] = useState<string>("");
  const [commandSearch, setCommandSearch] = useState("");
  const [showCommandLibrary, setShowCommandLibrary] = useState(true);
  const [taskTitle, setTaskTitle] = useState("Inspect workspace");
  const [taskSummary, setTaskSummary] = useState("Review scoped files before touching any command.");
  const [taskAllowShell, setTaskAllowShell] = useState(true);
  const [taskAllowNetwork, setTaskAllowNetwork] = useState(false);
  const [taskAllowWrites, setTaskAllowWrites] = useState(false);
  const [workerName, setWorkerName] = useState("Agent 1");
  const [workerPath, setWorkerPath] = useState("");
  const [workerArgs, setWorkerArgs] = useState("");
  const [workerMemoryMode, setWorkerMemoryMode] = useState<AgentMemoryMode>("ephemeral");
  const [selectedProfileId, setSelectedProfileId] = useState<string>("");
  const [selectedProfileDetailId, setSelectedProfileDetailId] = useState<string>("");
  const [profileName, setProfileName] = useState("Custom profile");
  const [selectedWorker, setSelectedWorker] = useState<string>("");
  const [exportedAudit, setExportedAudit] = useState<string>("");

  useEffect(() => {
    bootstrap().then((next) => {
      setState((current) => mergeDashboardState(current, next));
      setActiveSessionId(next.sessions[0]?.id ?? null);
      setSelectedWorker(next.workers[0]?.id ?? "");
      setSelectedAgentId(next.workers[0]?.id ?? "");
      setSelectedApprovalId(next.pendingApprovals[0]?.request.id ?? "");
      setSelectedProtectionId(next.protections[0]?.id ?? "");
      setSelectedTaskId(next.tasks[0]?.id ?? "");
      setSelectedAuditId(next.audit[0]?.id ?? "");
      setWorkerMemoryMode(next.policy.defaultMemoryMode);
      setSelectedProfileId(next.profiles[0]?.id ?? "");
      setSelectedProfileDetailId(next.profiles[0]?.id ?? "");
    });
  }, []);

  useEffect(() => {
    if (state) {
      setWorkerMemoryMode(state.policy.defaultMemoryMode);
      if (!selectedProfileId && state.profiles[0]) {
        setSelectedProfileId(state.profiles[0].id);
      }
      if (!selectedAgentId && state.workers[0]) {
        setSelectedAgentId(state.workers[0].id);
      }
      if (!selectedProfileDetailId && state.profiles[0]) {
        setSelectedProfileDetailId(state.profiles[0].id);
      }
      if (!selectedApprovalId && state.pendingApprovals[0]) {
        setSelectedApprovalId(state.pendingApprovals[0].request.id);
      }
      if (!selectedProtectionId && state.protections[0]) {
        setSelectedProtectionId(state.protections[0].id);
      }
      if (!selectedTaskId && state.tasks[0]) {
        setSelectedTaskId(state.tasks[0].id);
      }
      if (!selectedAuditId && state.audit[0]) {
        setSelectedAuditId(state.audit[0].id);
      }
    }
  }, [state, selectedProfileId, selectedProfileDetailId, selectedAgentId, selectedApprovalId, selectedProtectionId, selectedTaskId, selectedAuditId]);

  useEffect(() => {
    if (!state || !selectedWorker) {
      return;
    }

    const worker = state.workers.find((item) => item.id === selectedWorker);
    const profile = state.profiles.find((item) => item.id === worker?.profileId);
    if (profile) {
      setTaskAllowShell(profile.defaultGuardrails.allowShell);
      setTaskAllowNetwork(profile.defaultGuardrails.allowNetwork);
      setTaskAllowWrites(profile.defaultGuardrails.allowWrites);
    }
  }, [selectedWorker, state]);

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

  const selectedAgent = useMemo(
    () => state?.workers.find((worker) => worker.id === selectedAgentId) ?? null,
    [state, selectedAgentId]
  );

  const selectedApproval = useMemo(
    () => state?.pendingApprovals.find((approval) => approval.request.id === selectedApprovalId) ?? null,
    [state, selectedApprovalId]
  );

  const selectedProfileDetail = useMemo(
    () => state?.profiles.find((profile) => profile.id === selectedProfileDetailId) ?? null,
    [state, selectedProfileDetailId]
  );

  const selectedProtection = useMemo(
    () => state?.protections.find((protection) => protection.id === selectedProtectionId) ?? null,
    [state, selectedProtectionId]
  );

  const selectedTask = useMemo(
    () => state?.tasks.find((task) => task.id === selectedTaskId) ?? null,
    [state, selectedTaskId]
  );

  const selectedAuditEvent = useMemo(
    () => state?.audit.find((event) => event.id === selectedAuditId) ?? null,
    [state, selectedAuditId]
  );

  const workerNameById = useMemo(() => {
    if (!state) {
      return new Map<string, string>();
    }

    return new Map(state.workers.map((worker) => [worker.id, worker.name]));
  }, [state]);

  const filteredCommandGroups = useMemo(() => {
    const query = commandSearch.trim().toLowerCase();
    if (!query) {
      return COMMAND_LIBRARY;
    }

    return COMMAND_LIBRARY.filter(
      (group) =>
        group.label.toLowerCase().includes(query) ||
        group.description.toLowerCase().includes(query) ||
        group.items.some((item) => item.value.toLowerCase().includes(query))
    );
  }, [commandSearch]);

  if (!state) {
    return <div className="loading">Booting O.R.C. Terminal...</div>;
  }

  const dashboard = state;
  const approvalCount = dashboard.pendingApprovals.length;
  const runningCount = dashboard.workers.filter((worker) => worker.status === "running").length;
  const blockedTaskCount = dashboard.tasks.filter((task) => task.status === "paused" || task.status === "failed").length;
  const degradedProtectionCount = dashboard.protections.filter(
    (protection) => protection.state === "degraded" || protection.state === "unsupported"
  ).length;

  async function refresh<T>(fn: Promise<T>) {
    const next = (await fn) as DashboardState;
    setState((current) => mergeDashboardState(current, next));
    if (!activeSessionId && next.sessions[0]) {
      setActiveSessionId(next.sessions[0].id);
    }
    if (!selectedWorker && next.workers[0]) {
      setSelectedWorker(next.workers[0].id);
    }
    if (!selectedAgentId && next.workers[0]) {
      setSelectedAgentId(next.workers[0].id);
    }
    if (!selectedProfileDetailId && next.profiles[0]) {
      setSelectedProfileDetailId(next.profiles[0].id);
    }
    if (!selectedApprovalId && next.pendingApprovals[0]) {
      setSelectedApprovalId(next.pendingApprovals[0].request.id);
    }
    if (!selectedProtectionId && next.protections[0]) {
      setSelectedProtectionId(next.protections[0].id);
    }
    if (!selectedTaskId && next.tasks[0]) {
      setSelectedTaskId(next.tasks[0].id);
    }
    if (!selectedAuditId && next.audit[0]) {
      setSelectedAuditId(next.audit[0].id);
    }
  }

  async function submitCommand(event: FormEvent) {
    event.preventDefault();
    await handleCommandSend();
  }

  async function submitTerminalInput(sessionId: string, input: string) {
    if (!input.trim()) {
      return;
    }

    await refresh(sendTerminalInput(sessionId, input));
  }

  async function handleCommandSend() {
    if (!activeSessionId || !command.trim()) {
      setCommandStatus("Enter a command before sending.");
      return;
    }

    setCommandStatus(`Sending \`${command}\`...`);

    try {
      await submitTerminalInput(activeSessionId, command);
      setCommandStatus(`Sent \`${command}\`.`);
      setCommand(EMPTY_COMMAND);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCommandStatus(`Send failed: ${message}`);
    }
  }

  async function addWorker(adapter: "openclaw" | "nemoclaw") {
    const args = splitWorkerArgs(workerArgs);

    try {
      setWorkerStatusMessage(`Creating ${adapter} agent...`);
      await refresh(
        createWorker(adapter, workerName, workerPath || undefined, args, workerMemoryMode, selectedProfileId || undefined)
      );
      setWorkerStatusMessage(`Created ${adapter} agent.`);
      setWorkerName(`${adapter === "openclaw" ? "OpenClaw" : "NemoClaw"} agent`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setWorkerStatusMessage(`Agent creation failed: ${message}`);
    }
  }

  async function createTask() {
    if (!selectedWorker) {
      return;
    }

    const guardrails: TaskGuardrails = {
      allowShell: taskAllowShell,
      allowNetwork: taskAllowNetwork,
      allowWrites: taskAllowWrites
    };

    try {
      setWorkerStatusMessage(`Assigning task to agent...`);
      await refresh(assignTask(selectedWorker, taskTitle, taskSummary, guardrails));
      setWorkerStatusMessage(`Assigned task \`${taskTitle}\`.`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setWorkerStatusMessage(`Task assignment failed: ${message}`);
    }
  }

  async function updateWorkerState(workerId: string, status: WorkerStatus) {
    try {
      setWorkerStatusMessage(`Setting agent to \`${status}\`...`);
      await refresh(setWorkerStatus(workerId, status));
      setWorkerStatusMessage(`Agent moved to \`${status}\`.`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setWorkerStatusMessage(`Agent update failed: ${message}`);
    }
  }

  async function handleAuditExport() {
    const content = await exportAudit();
    setExportedAudit(content);
  }

  async function mutatePolicy(mutator: (current: SessionPolicy) => SessionPolicy) {
    const nextPolicy = mutator(dashboard.policy);
    try {
      setPolicyStatus("Saving policy...");
      await refresh(updatePolicy(nextPolicy));
      setPolicyStatus("Policy updated.");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setPolicyStatus(`Policy update failed: ${message}`);
    }
  }

  async function saveCurrentProfile() {
    if (!profileName.trim()) {
      setWorkerStatusMessage("Enter a profile name before saving.");
      return;
    }

    const defaultGuardrails: TaskGuardrails = {
      allowShell: taskAllowShell,
      allowNetwork: taskAllowNetwork,
      allowWrites: taskAllowWrites
    };

    try {
      setWorkerStatusMessage(`Saving profile \`${profileName}\`...`);
      await refresh(
        saveAgentProfile(
          profileName.trim(),
          dashboard.policy.allowCommands,
          dashboard.policy.allowDomains,
          dashboard.policy.defaultMemoryMode,
          dashboard.policy.delegationMode,
          dashboard.policy.delegationMaxDepth,
          defaultGuardrails
        )
      );
      setWorkerStatusMessage(`Saved profile \`${profileName}\`.`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setWorkerStatusMessage(`Profile save failed: ${message}`);
    }
  }

  async function applyProfileToWorker(workerId: string, profileId: string) {
    try {
      setWorkerStatusMessage("Applying profile to agent...");
      await refresh(applyAgentProfile(workerId, profileId));
      setWorkerStatusMessage("Applied profile to agent.");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setWorkerStatusMessage(`Profile apply failed: ${message}`);
    }
  }

  async function toggleLibraryGroup(items: LibraryItem[], enabled: boolean) {
    await mutatePolicy((current) => {
      const nextCommands = new Set(current.allowCommands);
      const nextDomains = new Set(current.allowDomains);

      for (const item of items) {
        if (item.kind === "command") {
          if (enabled) {
            nextCommands.add(item.value);
          } else {
            nextCommands.delete(item.value);
          }
        } else if (enabled) {
          nextDomains.add(item.value);
        } else {
          nextDomains.delete(item.value);
        }
      }

      return {
        ...current,
        allowCommands: Array.from(nextCommands),
        allowDomains: Array.from(nextDomains)
      };
    });
  }

  async function toggleLibraryItem(item: LibraryItem, enabled: boolean) {
    await toggleLibraryGroup([item], enabled);
  }

  return (
    <div className="app-shell app-shell-nav">
      <aside className="sidebar sidebar-nav">
        <section className="panel app-brand-panel">
          <div className="panel-header">
            <div>
              <h2>O.R.C. Terminal</h2>
              <p className="muted">Govern powerful agents without giving up the final say on execution, privacy, and risk.</p>
            </div>
            <button onClick={() => refresh(createCommandSession(`Session ${dashboard.sessions.length + 1}`))}>
              New Session
            </button>
          </div>
        </section>

        <section className="panel nav-panel">
          <div className="nav-list">
            {(["overview", "agents", "approvals", "tasks", "profiles", "policy", "audit", "protections"] as ViewId[]).map(
              (view) => (
                <button
                  key={view}
                  className={currentView === view ? "nav-link active" : "nav-link"}
                  onClick={() => setCurrentView(view)}
                >
                  <span>{VIEW_LABELS[view]}</span>
                  <span className="badge neutral">{navCount(view, dashboard)}</span>
                </button>
              )
            )}
          </div>
        </section>

        <section className="panel sidebar-summary-panel">
          <div className="summary-list compact">
            <div className="summary-card">
              <span className="muted">Running agents</span>
              <strong>{runningCount}</strong>
            </div>
            <div className="summary-card">
              <span className="muted">Pending approvals</span>
              <strong>{approvalCount}</strong>
            </div>
            <div className="summary-card">
              <span className="muted">Blocked tasks</span>
              <strong>{blockedTaskCount}</strong>
            </div>
            <div className="summary-card">
              <span className="muted">Protection issues</span>
              <strong>{degradedProtectionCount}</strong>
            </div>
          </div>
        </section>

      </aside>

      <main className="main-pane nav-main">
        <section className="panel page-header-panel">
          <h3>{VIEW_LABELS[currentView]}</h3>
          <p className="muted">{viewDescription(currentView)}</p>
        </section>

        {currentView === "overview" ? (
          <div className="view-stack">
            <section className="summary-grid">
              <div className="panel summary-panel">
                <span className="muted">Pending approvals</span>
                <strong>{approvalCount}</strong>
                <button onClick={() => setCurrentView("approvals")}>Review approvals</button>
              </div>
              <div className="panel summary-panel">
                <span className="muted">Running agents</span>
                <strong>{runningCount}</strong>
                <button onClick={() => setCurrentView("agents")}>Open agents</button>
              </div>
              <div className="panel summary-panel">
                <span className="muted">Tasks needing attention</span>
                <strong>{blockedTaskCount}</strong>
                <button onClick={() => setCurrentView("tasks")}>Open tasks</button>
              </div>
              <div className="panel summary-panel">
                <span className="muted">Protection issues</span>
                <strong>{degradedProtectionCount}</strong>
                <button onClick={() => setCurrentView("protections")}>Open protections</button>
              </div>
            </section>

            <section className="dual-pane-grid">
              <section className="panel">
                <div className="panel-header">
                  <h3>Attention now</h3>
                </div>
                <div className="overview-focus-list">
                  <button className="entity-row" onClick={() => setCurrentView("approvals")}>
                    <div>
                      <strong>Approvals waiting</strong>
                      <span className="muted">
                        {selectedApproval
                          ? `${selectedApproval.request.target} · ${approvalCapabilityLabel(selectedApproval.request.kind)}`
                          : "No requests waiting for review."}
                      </span>
                    </div>
                    <span className="badge deny">{approvalCount}</span>
                  </button>
                  <button className="entity-row" onClick={() => setCurrentView("tasks")}>
                    <div>
                      <strong>Tasks needing attention</strong>
                      <span className="muted">
                        {dashboard.tasks.find((task) => task.status === "paused" || task.status === "failed")?.title ??
                          "No blocked tasks right now."}
                      </span>
                    </div>
                    <span className="badge paused">{blockedTaskCount}</span>
                  </button>
                  <button className="entity-row" onClick={() => setCurrentView("protections")}>
                    <div>
                      <strong>Protection health</strong>
                      <span className="muted">
                        {selectedProtection
                          ? `${selectedProtection.label} · ${selectedProtection.state}`
                          : "All protection surfaces are currently available."}
                      </span>
                    </div>
                    <span className={degradedProtectionCount > 0 ? "badge degraded" : "badge active"}>
                      {degradedProtectionCount > 0 ? "review" : "healthy"}
                    </span>
                  </button>
                </div>
              </section>

              <section className="panel">
                <div className="panel-header">
                  <h3>Fleet snapshot</h3>
                  <button onClick={() => setCurrentView("agents")}>Open agents</button>
                </div>
                <div className="entity-list">
                  {dashboard.workers.slice(0, 4).map((worker) => (
                    <button
                      key={worker.id}
                      className="entity-row"
                      onClick={() => {
                        setSelectedAgentId(worker.id);
                        setCurrentView("agents");
                      }}
                    >
                      <div>
                        <strong>{worker.name}</strong>
                        <span className="muted">
                          {worker.adapter} · {profileNameById(dashboard.profiles, worker.profileId)}
                        </span>
                      </div>
                      <span className={`badge ${worker.status}`}>{worker.status}</span>
                    </button>
                  ))}
                </div>
              </section>
            </section>
          </div>
        ) : null}

        {currentView === "approvals" ? <section className="view-with-drawer">
          <section className="panel">
            <div className="panel-header">
              <h3>Approval inbox</h3>
              <span className="badge neutral">{dashboard.pendingApprovals.length}</span>
            </div>
            {dashboard.pendingApprovals.length === 0 ? (
              <p className="muted">No requests waiting for review.</p>
            ) : (
              <div className="entity-list">
                {dashboard.pendingApprovals.map((approval) => (
                  <button
                    key={approval.request.id}
                    className={selectedApprovalId === approval.request.id ? "entity-row active" : "entity-row"}
                    onClick={() => setSelectedApprovalId(approval.request.id)}
                  >
                    <div>
                      <strong>{approval.request.target}</strong>
                      <span className="muted">
                        {approvalCapabilityLabel(approval.request.kind)}
                        {approval.request.workerId
                          ? ` · ${workerNameById.get(approval.request.workerId) ?? approval.request.workerId}`
                          : " · human terminal"}
                      </span>
                    </div>
                    <span className="badge deny">{approval.request.kind}</span>
                  </button>
                ))}
              </div>
            )}
          </section>

          <aside className="panel detail-drawer">
            {selectedApproval ? (
              renderApprovalDrawer(
                selectedApproval,
                workerNameById,
                (requestId, mode) => refresh(approveRequest(requestId, mode)),
                (requestId) => refresh(denyRequest(requestId)),
                (requestId) => refresh(denyRequestAndStop(requestId))
              )
            ) : (
              <p className="muted">Select an approval request to review the capability, rationale, and available actions.</p>
            )}
          </aside>
        </section> : null}

        {currentView === "policy" ? <section className="dual-pane-grid">
          <section className="panel">
            <div className="panel-header">
              <h3>Command library</h3>
              <span className="badge deny">Deny by default</span>
            </div>
            <p className="muted">
              Choose what becomes broadly available to supervised agents and shell sessions. Commands and domains live in one searchable library.
            </p>
            <div className="list-block">
              <button type="button" className="section-toggle" onClick={() => setShowCommandLibrary((current) => !current)}>
                <strong>Search and select commands</strong>
                <span>{showCommandLibrary ? "Hide" : "Show"}</span>
              </button>
              {showCommandLibrary ? (
                <>
                  <input
                    value={commandSearch}
                    onChange={(event) => setCommandSearch(event.target.value)}
                    placeholder="Search commands, groups, or domains"
                  />
                  <div className="command-library">
                    {filteredCommandGroups.map((group) => {
                      const enabledCount = group.items.filter((item) => isLibraryItemEnabled(dashboard.policy, item)).length;
                      const allAllowed = enabledCount === group.items.length;
                      return (
                        <div className="command-group" key={group.id}>
                          <label className="toggle-row">
                            <input
                              type="checkbox"
                              checked={allAllowed}
                              onChange={(event) => void toggleLibraryGroup(group.items, event.target.checked)}
                            />
                            <span>
                              <strong>{group.label}</strong>
                              <span className="muted">{group.description}</span>
                            </span>
                          </label>
                          <div className="chips">
                            {group.items.map((item) => (
                              <label className="toggle-chip" key={`${item.kind}:${item.value}`}>
                                <input
                                  type="checkbox"
                                  checked={isLibraryItemEnabled(dashboard.policy, item)}
                                  onChange={(event) => void toggleLibraryItem(item, event.target.checked)}
                                />
                                <span className="chip">{item.kind === "domain" ? `${item.value} (domain)` : item.value}</span>
                              </label>
                            ))}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </>
              ) : null}
            </div>
          </section>

          <section className="view-stack">
            <section className="panel">
              <div className="panel-header">
                <h3>Global defaults</h3>
              </div>
              <div className="list-block">
                <strong>Memory</strong>
                <div className="policy-editor single-field">
                  <select
                    value={dashboard.policy.defaultMemoryMode}
                    onChange={(event) =>
                      mutatePolicy((current) => ({
                        ...current,
                        defaultMemoryMode: event.target.value as AgentMemoryMode
                      }))
                    }
                  >
                    <option value="ephemeral">Default: ephemeral</option>
                    <option value="task_scoped">Default: task scoped</option>
                    <option value="agent_scoped">Default: agent scoped</option>
                    <option value="persistent">Default: persistent</option>
                  </select>
                </div>
              </div>
              <div className="list-block">
                <strong>Delegation</strong>
                <div className="policy-editor">
                  <select
                    value={dashboard.policy.delegationMode}
                    onChange={(event) =>
                      mutatePolicy((current) => ({
                        ...current,
                        delegationMode: event.target.value as SessionPolicy["delegationMode"]
                      }))
                    }
                  >
                    <option value="deny">Deny</option>
                    <option value="prompt">Prompt</option>
                    <option value="allow">Allow</option>
                  </select>
                  <input
                    type="number"
                    min={0}
                    max={8}
                    value={dashboard.policy.delegationMaxDepth}
                    onChange={(event) =>
                      mutatePolicy((current) => ({
                        ...current,
                        delegationMaxDepth: Number.parseInt(event.target.value || "0", 10)
                      }))
                    }
                  />
                </div>
              </div>
              {policyStatus ? <p className="panel-status">{policyStatus}</p> : null}
            </section>

            <section className="panel">
              <div className="panel-header">
                <h3>Profile capture</h3>
              </div>
              <div className="form-grid">
                <input value={profileName} onChange={(event) => setProfileName(event.target.value)} placeholder="Profile name" />
                <button type="button" onClick={saveCurrentProfile}>
                  Save current controls as profile
                </button>
              </div>
              <div className="summary-list compact">
                <div className="summary-card compact-card">
                  <span className="muted">Allowed commands</span>
                  <strong>{dashboard.policy.allowCommands.length}</strong>
                </div>
                <div className="summary-card compact-card">
                  <span className="muted">Allowed domains</span>
                  <strong>{dashboard.policy.allowDomains.length}</strong>
                </div>
              </div>
            </section>
          </section>
        </section> : null}

        {currentView === "protections" ? <section className="view-with-drawer">
          <section className="panel">
            <div className="panel-header">
              <h3>Protection health</h3>
              <span className="badge neutral">{dashboard.protections.length}</span>
            </div>
            <div className="entity-list">
              {dashboard.protections.map((protection) => (
                <button
                  key={protection.id}
                  className={selectedProtectionId === protection.id ? "entity-row active" : "entity-row"}
                  onClick={() => setSelectedProtectionId(protection.id)}
                >
                  <div>
                    <strong>{protection.label}</strong>
                    <span className="muted">{protection.detail}</span>
                  </div>
                  <span className={`badge ${protection.state}`}>{protection.state}</span>
                </button>
              ))}
            </div>
          </section>

          <aside className="panel detail-drawer">
            {selectedProtection ? (
              renderProtectionDrawer(selectedProtection)
            ) : (
              <p className="muted">Select a protection surface to inspect its current state and what it means for the runtime.</p>
            )}
          </aside>
        </section> : null}

        {currentView === "overview" || currentView === "tasks" ? <section className="panel terminal-panel">
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

          <TerminalView session={activeSession} onSubmitInput={submitTerminalInput} />

          <form className="command-form" onSubmit={submitCommand}>
            <label htmlFor="command-input">$</label>
            <input
              id="command-input"
              value={command}
              onChange={(event) => setCommand(event.target.value)}
              placeholder="Send a command through the PTY, for example `dir` or `git status`."
            />
            <button type="button" onClick={handleCommandSend}>
              Send
            </button>
          </form>
          {commandStatus ? <p className="command-status">{commandStatus}</p> : null}
        </section> : null}

        <section className={currentView === "overview" ? "grid overview-grid" : "grid single-focus-grid"}>

          {currentView === "agents" ? <section className="view-with-drawer">
            <section className="panel">
              <div className="panel-header">
                <h3>Agent fleet</h3>
                <span className="badge neutral">{dashboard.workers.length}</span>
              </div>
              <div className="worker-creation">
                <input value={workerName} onChange={(event) => setWorkerName(event.target.value)} placeholder="Agent name" />
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
                <select value={workerMemoryMode} onChange={(event) => setWorkerMemoryMode(event.target.value as AgentMemoryMode)}>
                  <option value="ephemeral">Ephemeral memory</option>
                  <option value="task_scoped">Task-scoped memory</option>
                  <option value="agent_scoped">Agent-scoped memory</option>
                  <option value="persistent">Persistent memory</option>
                </select>
                <select value={selectedProfileId} onChange={(event) => setSelectedProfileId(event.target.value)}>
                  {dashboard.profiles.map((profile) => (
                    <option key={profile.id} value={profile.id}>
                      {profile.name}
                    </option>
                  ))}
                </select>
                <div className="inline-actions">
                  <button onClick={() => addWorker("openclaw")}>Add OpenClaw</button>
                  <button onClick={() => addWorker("nemoclaw")}>Add NemoClaw</button>
                </div>
              </div>
              <p className="muted">
                Pick an agent to inspect its capabilities, current profile, sandbox scope, and live output without expanding the entire fleet.
              </p>
              {workerStatusMessage ? <p className="panel-status">{workerStatusMessage}</p> : null}
              <div className="entity-list">
                {dashboard.workers.map((worker) => (
                  <button
                    key={worker.id}
                    className={selectedAgentId === worker.id ? "entity-row active" : "entity-row"}
                    onClick={() => setSelectedAgentId(worker.id)}
                  >
                    <div>
                      <strong>{worker.name}</strong>
                      <span className="muted">
                        {worker.adapter} · {profileNameById(dashboard.profiles, worker.profileId)}
                      </span>
                    </div>
                    <span className={`badge ${worker.status}`}>{worker.status}</span>
                  </button>
                ))}
              </div>
            </section>

            <aside className="panel detail-drawer">
              {selectedAgent ? renderAgentDrawer(selectedAgent, dashboard.profiles, updateWorkerState, applyProfileToWorker) : (
                <p className="muted">Select an agent to inspect its runtime, profile, and live output.</p>
              )}
            </aside>
          </section> : null}

          {currentView === "tasks" ? <section className="view-with-drawer">
            <section className="panel">
              <div className="panel-header">
                <h3>Task queue</h3>
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
              <div className="guardrail-list">
                <label className="toggle-row">
                  <input
                    type="checkbox"
                    checked={taskAllowShell}
                    onChange={(event) => setTaskAllowShell(event.target.checked)}
                  />
                  Allow shell execution
                </label>
                <label className="toggle-row">
                  <input
                    type="checkbox"
                    checked={taskAllowNetwork}
                    onChange={(event) => setTaskAllowNetwork(event.target.checked)}
                  />
                  Allow network access
                </label>
                <label className="toggle-row">
                  <input
                    type="checkbox"
                    checked={taskAllowWrites}
                    onChange={(event) => setTaskAllowWrites(event.target.checked)}
                  />
                  Allow write-capable commands
                </label>
              </div>
              <button onClick={createTask}>Assign task</button>
              <div className="entity-list">
                {dashboard.tasks.map((task) => (
                  <button
                    key={task.id}
                    className={selectedTaskId === task.id ? "entity-row active" : "entity-row"}
                    onClick={() => setSelectedTaskId(task.id)}
                  >
                    <div>
                      <strong>{task.title}</strong>
                      <span className="muted">{task.summary}</span>
                    </div>
                    <span className={`badge ${task.status}`}>{task.status}</span>
                  </button>
                ))}
              </div>
            </section>

            <aside className="panel detail-drawer">
              {selectedTask ? renderTaskDrawer(selectedTask, workerNameById) : (
                <p className="muted">Select a task to inspect its assignment, guardrails, and status.</p>
              )}
            </aside>
          </section> : null}

          {currentView === "overview" ? <section className="dual-pane-grid">
            <section className="panel">
              <div className="panel-header">
                <h3>Task snapshot</h3>
                <button onClick={() => setCurrentView("tasks")}>Open tasks</button>
              </div>
              <div className="entity-list">
                {dashboard.tasks.slice(0, 4).map((task) => (
                  <button
                    key={task.id}
                    className="entity-row"
                    onClick={() => {
                      setSelectedTaskId(task.id);
                      setCurrentView("tasks");
                    }}
                  >
                    <div>
                      <strong>{task.title}</strong>
                      <span className="muted">{task.summary}</span>
                    </div>
                    <span className={`badge ${task.status}`}>{task.status}</span>
                  </button>
                ))}
              </div>
            </section>

            <section className="panel">
              <div className="panel-header">
                <h3>Recent audit</h3>
                <button onClick={() => setCurrentView("audit")}>Open audit</button>
              </div>
              <div className="entity-list">
                {dashboard.audit.slice(0, 5).map((event) => (
                  <button
                    key={event.id}
                    className="entity-row"
                    onClick={() => {
                      setSelectedAuditId(event.id);
                      setCurrentView("audit");
                    }}
                  >
                    <div>
                      <strong>{event.category}</strong>
                      <span className="muted">{event.message}</span>
                    </div>
                    {event.outcome ? <span className="badge neutral">{event.outcome.replace(/_/g, " ")}</span> : null}
                  </button>
                ))}
              </div>
            </section>
          </section> : null}

          {currentView === "audit" ? <section className="view-with-drawer">
            <section className="panel">
              <div className="panel-header">
                <h3>Audit timeline</h3>
                <button onClick={handleAuditExport}>Export</button>
              </div>
              <div className="entity-list">
                {dashboard.audit.map((event) => (
                  <button
                    key={event.id}
                    className={selectedAuditId === event.id ? "entity-row active" : "entity-row"}
                    onClick={() => setSelectedAuditId(event.id)}
                  >
                    <div>
                      <strong>{event.category}</strong>
                      <span className="muted">{event.message}</span>
                    </div>
                    {event.outcome ? <span className="badge neutral">{event.outcome.replace(/_/g, " ")}</span> : null}
                  </button>
                ))}
              </div>
            </section>

            <aside className="panel detail-drawer">
              {selectedAuditEvent ? renderAuditDrawer(selectedAuditEvent) : (
                <p className="muted">Select an audit event to inspect its source, outcome, and identifiers.</p>
              )}
              {exportedAudit ? <textarea className="export-box" readOnly value={exportedAudit} rows={12} /> : null}
            </aside>
          </section> : null}

          {currentView === "profiles" ? <section className="view-with-drawer">
            <section className="panel">
              <div className="panel-header">
                <h3>Profile library</h3>
                <span className="badge neutral">{dashboard.profiles.length}</span>
              </div>
              <div className="form-grid">
                <input value={profileName} onChange={(event) => setProfileName(event.target.value)} placeholder="Profile name" />
                <button type="button" onClick={saveCurrentProfile}>
                  Save current controls as profile
                </button>
              </div>
              <p className="muted">
                Profiles capture the current command library choices, network domains, memory mode, delegation mode, and task defaults.
              </p>
              <div className="entity-list">
                {dashboard.profiles.map((profile) => (
                  <button
                    key={profile.id}
                    className={selectedProfileDetailId === profile.id ? "entity-row active" : "entity-row"}
                    onClick={() => setSelectedProfileDetailId(profile.id)}
                  >
                    <div>
                      <strong>{profile.name}</strong>
                      <span className="muted">
                        {profile.builtIn ? "built in" : "custom"} · {formatCapability(profile.memoryMode)} memory
                      </span>
                    </div>
                    <span className="badge neutral">{profile.allowCommands.length + profile.allowDomains.length}</span>
                  </button>
                ))}
              </div>
              {workerStatusMessage ? <p className="panel-status">{workerStatusMessage}</p> : null}
            </section>

            <aside className="panel detail-drawer">
              {selectedProfileDetail ? (
                renderProfileDrawer(selectedProfileDetail)
              ) : (
                <p className="muted">Select a profile to inspect its defaults, guardrails, and command/library footprint.</p>
              )}
            </aside>
          </section> : null}
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
      return existing
        ? {
            ...session,
            lines: existing.lines.length > session.lines.length ? existing.lines : session.lines
          }
        : session;
    }),
    workers: next.workers.map((worker) => {
      const existing = current.workers.find((item) => item.id === worker.id);
      return existing
        ? {
            ...worker,
            outputLines:
              existing.outputLines.length > worker.outputLines.length
                ? existing.outputLines
                : worker.outputLines
          }
        : worker;
    })
  };
}

function splitWorkerArgs(value: string): string[] {
  const matches = value.match(/"([^"]*)"|'([^']*)'|[^\s]+/g) ?? [];
  return matches
    .map((item) => {
      const quote = item[0];
      if ((quote === `"` || quote === "'") && item[item.length - 1] === quote) {
        return item.slice(1, -1);
      }
      return item;
    })
    .filter(Boolean);
}

function renderAgentDrawer(
  worker: Worker,
  profiles: AgentProfile[],
  updateWorkerState: (workerId: string, status: WorkerStatus) => Promise<void>,
  applyProfileToWorker: (workerId: string, profileId: string) => Promise<void>
) {
  return (
    <div className="detail-stack">
      <div className="panel-header">
        <div>
          <h3>{worker.name}</h3>
          <p className="muted">{worker.adapter}</p>
        </div>
        <span className={`badge ${worker.status}`}>{worker.status}</span>
      </div>
      <span className="line">{worker.executablePath || "No executable configured yet"}</span>
      <div className="chips">
        <span className="chip">trust: {worker.trustLevel.replace("_", " ")}</span>
        <span className="chip">mode: {worker.runtimeMode.replace("_", " ")}</span>
        <span className="chip">compat: {worker.compatibility.replace("_", " ")}</span>
        <span className="chip">memory: {formatCapability(worker.memoryMode)}</span>
        <span className="chip">profile: {profileNameById(profiles, worker.profileId)}</span>
      </div>
      <div className="capability-grid">
        <span className="chip">exec: {formatCapability(worker.capabilityProfile.execution)}</span>
        <span className="chip">files: {formatCapability(worker.capabilityProfile.filesystem)}</span>
        <span className="chip">network: {formatCapability(worker.capabilityProfile.network)}</span>
        <span className="chip">memory: {formatCapability(worker.capabilityProfile.memory)}</span>
        <span className="chip">delegation: {formatCapability(worker.capabilityProfile.delegation)}</span>
        <span className="chip">control: {formatCapability(worker.capabilityProfile.controlPlane)}</span>
      </div>
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
      <select value={worker.profileId ?? ""} onChange={(event) => void applyProfileToWorker(worker.id, event.target.value)}>
        {profiles.map((profile) => (
          <option key={profile.id} value={profile.id}>
            {profile.name}
          </option>
        ))}
      </select>
      <div className="worker-output">
        {worker.outputLines.length === 0 ? (
          <span className="muted">No agent output yet.</span>
        ) : (
          worker.outputLines.slice(-24).map((line, index) => (
            <span className="line" key={`${worker.id}-${index}`}>
              {line}
            </span>
          ))
        )}
      </div>
    </div>
  );
}

function renderApprovalDrawer(
  approval: PendingApproval,
  workerNameById: Map<string, string>,
  approve: (requestId: string, mode: ApprovalMode) => Promise<void>,
  deny: (requestId: string) => Promise<void>,
  denyAndStop: (requestId: string) => Promise<void>
) {
  return (
    <div className="detail-stack">
      <div className="panel-header">
        <div>
          <h3>{approval.request.target}</h3>
          <p className="muted">Capability: {approvalCapabilityLabel(approval.request.kind)}</p>
        </div>
        <span className="badge deny">{approval.request.kind}</span>
      </div>
      <span>{approval.decision.reason}</span>
      <span className="line">
        Source: {approval.request.workerId ? workerNameById.get(approval.request.workerId) ?? approval.request.workerId : "human terminal"}
      </span>
      {approval.request.cwd ? <span className="line">cwd: {approval.request.cwd}</span> : null}
      {approval.request.command ? <span className="line">command: {approval.request.command}</span> : null}
      <div className="approval-actions">
        {(["one_time", "session", "persistent"] as ApprovalMode[]).map((mode) => (
          <button key={mode} onClick={() => approve(approval.request.id, mode)}>
            {mode.replace("_", " ")}
          </button>
        ))}
        <button className="danger" onClick={() => deny(approval.request.id)}>
          Deny
        </button>
        {approval.request.workerId ? (
          <button className="danger" onClick={() => denyAndStop(approval.request.id)}>
            Deny + stop
          </button>
        ) : null}
      </div>
    </div>
  );
}

function renderTaskDrawer(task: SupervisorTask, workerNameById: Map<string, string>) {
  return (
    <div className="detail-stack">
      <div className="panel-header">
        <div>
          <h3>{task.title}</h3>
          <p className="muted">{task.summary}</p>
        </div>
        <span className={`badge ${task.status}`}>{task.status}</span>
      </div>
      <span className="line">
        Assigned agent: {task.assignedWorkerId ? workerNameById.get(task.assignedWorkerId) ?? task.assignedWorkerId : "unassigned"}
      </span>
      <div className="chips">
        <span className="chip">shell: {task.guardrails.allowShell ? "allowed" : "blocked"}</span>
        <span className="chip">network: {task.guardrails.allowNetwork ? "allowed" : "blocked"}</span>
        <span className="chip">writes: {task.guardrails.allowWrites ? "allowed" : "blocked"}</span>
      </div>
    </div>
  );
}

function renderAuditDrawer(event: AuditEvent) {
  return (
    <div className="detail-stack">
      <div className="panel-header">
        <div>
          <h3>{event.category}</h3>
          <p className="muted">Source: {event.source.replace(/_/g, " ")}</p>
        </div>
        {event.outcome ? <span className="badge neutral">{event.outcome.replace(/_/g, " ")}</span> : null}
      </div>
      <span>{event.message}</span>
      {event.workerId ? <span className="line">Worker: {event.workerId}</span> : null}
      {event.requestId ? <span className="line">Request: {event.requestId}</span> : null}
      <span className="muted">{event.timestamp}</span>
    </div>
  );
}

function renderProtectionDrawer(protection: DashboardState["protections"][number]) {
  return (
    <div className="detail-stack">
      <div className="panel-header">
        <div>
          <h3>{protection.label}</h3>
          <p className="muted">Runtime enforcement surface</p>
        </div>
        <span className={`badge ${protection.state}`}>{protection.state}</span>
      </div>
      <span>{protection.detail}</span>
      <p className="muted">
        This tells the supervisor whether the protection is fully active, merely available, optionally stronger with host support,
        or currently degraded.
      </p>
    </div>
  );
}

function renderProfileDrawer(profile: AgentProfile) {
  return (
    <div className="detail-stack">
      <div className="panel-header">
        <div>
          <h3>{profile.name}</h3>
          <p className="muted">{profile.builtIn ? "Built-in profile" : "Custom profile"}</p>
        </div>
        <span className="badge neutral">{profile.allowCommands.length + profile.allowDomains.length} rules</span>
      </div>
      <div className="chips">
        <span className="chip">memory: {formatCapability(profile.memoryMode)}</span>
        <span className="chip">delegation: {formatCapability(profile.delegationMode)}</span>
        <span className="chip">depth: {profile.delegationMaxDepth}</span>
      </div>
      <div className="chips">
        <span className="chip">shell: {profile.defaultGuardrails.allowShell ? "allowed" : "blocked"}</span>
        <span className="chip">network: {profile.defaultGuardrails.allowNetwork ? "allowed" : "blocked"}</span>
        <span className="chip">writes: {profile.defaultGuardrails.allowWrites ? "allowed" : "blocked"}</span>
      </div>
      <p className="muted">Allowed commands</p>
      <div className="chips">
        {profile.allowCommands.length === 0 ? <span className="chip">none</span> : profile.allowCommands.map((command) => (
          <span className="chip" key={command}>
            {command}
          </span>
        ))}
      </div>
      <p className="muted">Allowed domains</p>
      <div className="chips">
        {profile.allowDomains.length === 0 ? <span className="chip">none</span> : profile.allowDomains.map((domain) => (
          <span className="chip" key={domain}>
            {domain}
          </span>
        ))}
      </div>
    </div>
  );
}

function approvalCapabilityLabel(kind: string): string {
  switch (kind) {
    case "command":
      return "execution";
    case "file":
      return "filesystem";
    case "network":
      return "network";
    case "app":
      return "external app";
    case "mcp":
      return "connector";
    default:
      return kind;
  }
}

function formatCapability(value: string): string {
  return value.replace(/_/g, " ");
}

function isLibraryItemEnabled(policy: SessionPolicy, item: LibraryItem): boolean {
  return item.kind === "command"
    ? policy.allowCommands.includes(item.value)
    : policy.allowDomains.includes(item.value);
}

function profileNameById(profiles: AgentProfile[], profileId?: string | null): string {
  return profiles.find((profile) => profile.id === profileId)?.name ?? "none";
}

function navCount(view: ViewId, state: DashboardState): number {
  switch (view) {
    case "overview":
      return state.pendingApprovals.length + state.workers.filter((worker) => worker.status === "running").length;
    case "agents":
      return state.workers.length;
    case "approvals":
      return state.pendingApprovals.length;
    case "tasks":
      return state.tasks.length;
    case "profiles":
      return state.profiles.length;
    case "policy":
      return state.policy.allowCommands.length + state.policy.allowDomains.length;
    case "audit":
      return state.audit.length;
    case "protections":
      return state.protections.length;
    default:
      return 0;
  }
}

function viewDescription(view: ViewId): string {
  switch (view) {
    case "overview":
      return "See what needs attention right now without opening every low-level control.";
    case "agents":
      return "Create and supervise broker-only agents without crowding the rest of the dashboard.";
    case "approvals":
      return "Review blocked actions and decide what can proceed, for how long, and under whose authority.";
    case "tasks":
      return "Assign work with guardrails and monitor execution through the terminal.";
    case "profiles":
      return "Save reusable control envelopes so supervisors can launch the right kind of agent quickly.";
    case "policy":
      return "Manage the global command library and shared defaults that shape agent behavior.";
    case "audit":
      return "Inspect the event trail behind commands, approvals, policy changes, and agent activity.";
    case "protections":
      return "Check which runtime protections are active, degraded, or optionally available on this machine.";
    default:
      return "";
  }
}
