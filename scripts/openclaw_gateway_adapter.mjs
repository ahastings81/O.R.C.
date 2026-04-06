#!/usr/bin/env node

import {
  createHash,
  createPrivateKey,
  generateKeyPairSync,
  randomUUID,
  sign as signBytes
} from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import readline from "node:readline";

function parseCliArgs(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith("--")) {
      continue;
    }

    const key = token.slice(2);
    const next = argv[index + 1];
    if (!next || next.startsWith("--")) {
      options[key] = "true";
      continue;
    }

    options[key] = next;
    index += 1;
  }

  return options;
}

const cliOptions = parseCliArgs(process.argv.slice(2));
const sandboxRoot = process.env.ORC_TERMINAL_SANDBOX_ROOT || process.cwd();
const stateFile =
  cliOptions["device-identity-file"] ||
  process.env.OPENCLAW_DEVICE_IDENTITY_FILE ||
  join(sandboxRoot, "openclaw-device-identity.json");
const gatewayUrl =
  cliOptions["gateway-url"] || process.env.OPENCLAW_GATEWAY_URL || "ws://127.0.0.1:18789";
const targetAgentId = cliOptions["agent-id"] || process.env.OPENCLAW_AGENT_ID || "main";
const thinkingLevel = cliOptions.thinking || process.env.OPENCLAW_THINKING || "low";
const sessionKeyPrefix =
  cliOptions["session-key-prefix"] || process.env.OPENCLAW_SESSION_KEY_PREFIX || "orc-terminal";
const waitTimeoutMs = Number.parseInt(
  cliOptions["wait-timeout-ms"] || process.env.OPENCLAW_WAIT_TIMEOUT_MS || "30000",
  10
);
const maxPromptChars = Number.parseInt(
  cliOptions["max-prompt-chars"] || process.env.OPENCLAW_MAX_PROMPT_CHARS || "12000",
  10
);

if (typeof WebSocket === "undefined") {
  console.error("adapter-error: Node runtime does not provide a global WebSocket implementation.");
  process.exit(1);
}

function base64UrlEncode(buffer) {
  return Buffer.from(buffer)
    .toString("base64")
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

function base64UrlDecode(value) {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padding = normalized.length % 4 === 0 ? "" : "=".repeat(4 - (normalized.length % 4));
  return Buffer.from(normalized + padding, "base64");
}

function decodeBase64(value) {
  if (!value) {
    return "";
  }
  return Buffer.from(value, "base64").toString("utf8");
}

function ensureParentDir(path) {
  mkdirSync(dirname(path), { recursive: true });
}

function readTrimmedFile(path) {
  return readFileSync(path, "utf8").trim();
}

function resolveGatewayToken() {
  const inlineToken = cliOptions["gateway-token"] || process.env.OPENCLAW_GATEWAY_TOKEN;
  if (inlineToken) {
    return inlineToken;
  }

  const tokenFile =
    cliOptions["gateway-token-file"] || process.env.OPENCLAW_GATEWAY_TOKEN_FILE || "";
  if (tokenFile) {
    return readTrimmedFile(tokenFile);
  }

  throw new Error(
    "OpenClaw gateway token missing. Use --gateway-token, --gateway-token-file, or OPENCLAW_GATEWAY_TOKEN."
  );
}

function generateDeviceIdentity() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const jwk = publicKey.export({ format: "jwk" });
  const publicKeyRaw = base64UrlDecode(jwk.x);
  const deviceId = createHash("sha256").update(publicKeyRaw).digest("hex");
  const privateKeyPem = privateKey.export({ format: "pem", type: "pkcs8" }).toString();

  return {
    device: {
      id: deviceId,
      publicKey: base64UrlEncode(publicKeyRaw),
      privateKey: privateKeyPem
    },
    deviceToken: ""
  };
}

function saveAdapterState(state) {
  ensureParentDir(stateFile);
  writeFileSync(stateFile, JSON.stringify(state, null, 2));
}

function loadAdapterState() {
  try {
    if (existsSync(stateFile)) {
      return JSON.parse(readFileSync(stateFile, "utf8"));
    }
  } catch (error) {
    console.error(`adapter-warning: failed to read device identity state: ${String(error)}`);
  }

  const state = generateDeviceIdentity();
  saveAdapterState(state);
  console.log("adapter-status: generated persistent OpenClaw device identity");
  return state;
}

function buildSignedDevice(deviceIdentity, nonce, signedAt, tokenForSignature) {
  const payload = [
    "v2",
    deviceIdentity.id,
    "orc-terminal-adapter",
    "adapter",
    "operator",
    "operator.read,operator.write",
    String(signedAt),
    tokenForSignature,
    nonce
  ].join("|");

  const privateKey = createPrivateKey(deviceIdentity.privateKey);
  const signature = signBytes(null, Buffer.from(payload, "utf8"), privateKey);

  return {
    id: deviceIdentity.id,
    publicKey: deviceIdentity.publicKey,
    signature: base64UrlEncode(signature),
    signedAt,
    nonce
  };
}

function waitForOpen(websocket) {
  return new Promise((resolve, reject) => {
    websocket.addEventListener("open", () => resolve(), { once: true });
    websocket.addEventListener("error", (event) => reject(event.error || new Error("websocket open failed")), {
      once: true
    });
  });
}

function nextFrame(websocket, timeoutMs = 30000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for gateway frame after ${timeoutMs}ms`));
    }, timeoutMs);

    const cleanup = () => {
      clearTimeout(timer);
      websocket.removeEventListener("message", onMessage);
      websocket.removeEventListener("close", onClose);
      websocket.removeEventListener("error", onError);
    };

    const onMessage = (event) => {
      cleanup();
      try {
        resolve(JSON.parse(event.data.toString()));
      } catch (error) {
        reject(error);
      }
    };

    const onClose = () => {
      cleanup();
      reject(new Error("gateway websocket closed"));
    };

    const onError = (event) => {
      cleanup();
      reject(event.error || new Error("gateway websocket error"));
    };

    websocket.addEventListener("message", onMessage);
    websocket.addEventListener("close", onClose);
    websocket.addEventListener("error", onError);
  });
}

async function connectGateway() {
  const state = loadAdapterState();
  const gatewayToken = resolveGatewayToken();
  const websocket = new WebSocket(gatewayUrl);
  await waitForOpen(websocket);

  const challenge = await nextFrame(websocket, 10000);
  if (challenge?.type !== "event" || challenge?.event !== "connect.challenge") {
    throw new Error("expected connect.challenge from OpenClaw Gateway");
  }

  const nonce = challenge.payload?.nonce;
  const signedAt = challenge.payload?.ts;
  const tokenForSignature = state.deviceToken || gatewayToken;
  const signedDevice = buildSignedDevice(state.device, nonce, signedAt, tokenForSignature);
  const connectId = randomUUID();

  websocket.send(
    JSON.stringify({
      type: "req",
      id: connectId,
      method: "connect",
      params: {
        minProtocol: 3,
        maxProtocol: 3,
        client: {
          id: "orc-terminal-adapter",
          version: "0.1.0",
          platform: process.platform,
          mode: "operator"
        },
        role: "operator",
        scopes: ["operator.read", "operator.write"],
        caps: [],
        commands: [],
        permissions: {},
        auth: { token: gatewayToken },
        locale: "en-US",
        userAgent: "orc-terminal-openclaw-adapter/0.1.0",
        device: signedDevice
      }
    })
  );

  const response = await nextFrame(websocket, 10000);
  if (response?.type !== "res" || response?.id !== connectId || response?.ok !== true) {
    throw new Error(response?.error?.message || "OpenClaw connect failed");
  }

  const newDeviceToken = response.payload?.auth?.deviceToken;
  if (newDeviceToken && newDeviceToken !== state.deviceToken) {
    state.deviceToken = newDeviceToken;
    saveAdapterState(state);
    console.log("adapter-status: persisted OpenClaw device token");
  }

  return { websocket, state };
}

function taskToMessage(task) {
  const lines = [];
  if (task.title) {
    lines.push(`Task: ${task.title}`);
  }
  if (task.summary) {
    lines.push(`Summary: ${task.summary}`);
  }
  if (task.id) {
    lines.push(`O.R.C. task id: ${task.id}`);
  }
  lines.push("Reply clearly for the human operator when no shell command is needed.");
  lines.push("If shell execution is necessary, emit a single broker line and stop.");
  return lines.join("\n");
}

function taskToExtraSystemPrompt(task) {
  return [
    "You are running inside O.R.C. Terminal as an untrusted broker-only external agent.",
    "You do not have direct shell, PowerShell, cmd, Python, or terminal authority.",
    "If you need shell execution, emit exactly one line that begins with `PROXY_CMD ` followed by the raw command.",
    "If you need to specify cwd, emit exactly one `PROXY_JSON {\"kind\":\"command\",\"command\":\"...\",\"cwd\":\"...\"}` line.",
    "Do not wrap the broker line in backticks, markdown, or extra commentary.",
    "Do not attempt to approve blocked commands. Human approval is required.",
    "After emitting a broker line, stop. Wait for the next broker result instead of inventing command output.",
    task.title ? `Current task title: ${task.title}` : "",
    task.summary ? `Current task summary: ${task.summary}` : ""
  ]
    .filter(Boolean)
    .join("\n");
}

function clampText(value, limit = maxPromptChars) {
  if (value.length <= limit) {
    return value;
  }
  const remainder = value.length - limit;
  return `${value.slice(0, limit)}\n...[truncated ${remainder} chars]`;
}

function commandResultToMessage(result) {
  const command = decodeBase64(result.commandB64);
  const stdout = decodeBase64(result.stdoutB64);
  const stderr = decodeBase64(result.stderrB64);
  const combined = decodeBase64(result.combinedB64);
  const reason = decodeBase64(result.reasonB64);
  const lines = [
    "Broker result from O.R.C. Terminal:",
    `Request id: ${result.requestId}`,
    `Status: ${result.status}`,
    `Command: ${command || "(missing)"}`,
    `Stream mode: ${result.streamMode || "unknown"}`
  ];

  if (typeof result.exitCode === "number") {
    lines.push(`Exit code: ${result.exitCode}`);
  }

  if (reason) {
    lines.push(`Reason: ${reason}`);
  }

  if (stdout) {
    lines.push("STDOUT:");
    lines.push(clampText(stdout));
  }

  if (stderr) {
    lines.push("STDERR:");
    lines.push(clampText(stderr));
  }

  if (!stdout && !stderr && combined) {
    lines.push("OUTPUT:");
    lines.push(clampText(combined));
  }

  lines.push("If you need another shell command, emit exactly one PROXY_CMD/PROXY_JSON line and stop.");
  lines.push("Otherwise, answer the human operator directly.");

  return lines.join("\n");
}

function safeString(value) {
  return typeof value === "string" ? value : JSON.stringify(value);
}

function createAssistantLineWriter() {
  let buffer = "";
  let requestedBroker = false;

  function handleLine(line) {
    if (!line) {
      process.stdout.write("\n");
      return;
    }

    if (line.startsWith("PROXY_CMD ") || line.startsWith("PROXY_JSON ")) {
      requestedBroker = true;
    }
    process.stdout.write(`${line}\n`);
  }

  return {
    append(text) {
      if (!text) {
        return;
      }

      buffer += text;
      const parts = buffer.split(/\r?\n/);
      buffer = parts.pop() ?? "";
      for (const line of parts) {
        handleLine(line);
      }
    },
    flush() {
      if (buffer) {
        handleLine(buffer);
        buffer = "";
      }
    },
    requestedBroker() {
      return requestedBroker;
    }
  };
}

function printAgentEvent(payload, lineWriter) {
  if (!payload || typeof payload !== "object") {
    return;
  }

  if (payload.stream === "assistant") {
    const text =
      payload.data?.delta ??
      payload.data?.text ??
      payload.data?.content ??
      payload.data?.message ??
      "";
    lineWriter.append(String(text));
    return;
  }

  if (payload.stream === "tool") {
    lineWriter.flush();
    console.log(`adapter-tool: ${safeString(payload.data)}`);
    return;
  }

  if (payload.stream === "lifecycle") {
    const phase = payload.data?.phase || "unknown";
    lineWriter.flush();
    console.log(`adapter-lifecycle: ${phase}`);
  }
}

function requestFrame(id, method, params) {
  return {
    type: "req",
    id,
    method,
    params
  };
}

async function runAgentTurn(websocket, sessionKey, message, task, isFirstTurn) {
  const requestId = randomUUID();
  const waitId = randomUUID();
  const lineWriter = createAssistantLineWriter();

  websocket.send(
    JSON.stringify(
      requestFrame(requestId, "agent", {
        message,
        agentId: targetAgentId,
        sessionKey,
        thinking: thinkingLevel,
        deliver: false,
        bestEffortDeliver: false,
        extraSystemPrompt: taskToExtraSystemPrompt(task),
        idempotencyKey: randomUUID()
      })
    )
  );

  let runId = "";

  while (true) {
    const frame = await nextFrame(websocket, 120000);

    if (frame?.type === "res" && frame?.id === requestId) {
      if (!frame.ok) {
        throw new Error(frame?.error?.message || "agent request failed");
      }

      runId = frame.payload?.runId || "";
      lineWriter.flush();
      console.log(`adapter-status: agent accepted task run ${runId}`);
      websocket.send(
        JSON.stringify(
          requestFrame(waitId, "agent.wait", {
            runId,
            timeoutMs: waitTimeoutMs
          })
        )
      );
      continue;
    }

    if (frame?.type === "res" && frame?.id === waitId) {
      if (!frame.ok) {
        throw new Error(frame?.error?.message || "agent.wait failed");
      }

      const status = frame.payload?.status || "unknown";
      if (status === "timeout") {
        websocket.send(
          JSON.stringify(
            requestFrame(waitId, "agent.wait", {
              runId,
              timeoutMs: waitTimeoutMs
            })
          )
        );
        continue;
      }

      lineWriter.flush();
      const errorText = frame.payload?.error ? safeString(frame.payload.error) : "";
      console.log(`adapter-status: run ${runId} completed with status ${status}${errorText ? ` ${errorText}` : ""}`);
      return {
        requestedBroker: lineWriter.requestedBroker(),
        status,
        runId
      };
    }

    if (frame?.type === "event" && frame?.event === "agent") {
      if (!runId || frame.payload?.runId === runId) {
        printAgentEvent(frame.payload, lineWriter);
      }
    }
  }
}

function parseTaskEnvelope(lines) {
  const task = { type: "task", id: "", title: "", summary: "" };

  for (const rawLine of lines) {
    const line = rawLine.trimEnd();
    if (line.startsWith("TASK ")) {
      task.id = line.slice(5).trim();
      continue;
    }
    if (line.startsWith("TITLE:")) {
      task.title = line.slice(6).trim();
      continue;
    }
    if (line.startsWith("SUMMARY:")) {
      task.summary = line.slice(8).trim();
      continue;
    }
  }

  return task;
}

function parseCommandResultEnvelope(lines) {
  const result = {
    type: "command_result",
    requestId: lines[0].slice("COMMAND_RESULT ".length).trim(),
    status: "",
    commandB64: "",
    streamMode: "",
    exitCode: undefined,
    reasonB64: "",
    stdoutB64: "",
    stderrB64: "",
    combinedB64: ""
  };

  for (const rawLine of lines.slice(1)) {
    const line = rawLine.trimEnd();
    if (line.startsWith("STATUS:")) {
      result.status = line.slice(7).trim();
      continue;
    }
    if (line.startsWith("COMMAND_B64:")) {
      result.commandB64 = line.slice(12).trim();
      continue;
    }
    if (line.startsWith("STREAM_MODE:")) {
      result.streamMode = line.slice(12).trim();
      continue;
    }
    if (line.startsWith("EXIT_CODE:")) {
      const parsed = Number.parseInt(line.slice(10).trim(), 10);
      result.exitCode = Number.isNaN(parsed) ? undefined : parsed;
      continue;
    }
    if (line.startsWith("REASON_B64:")) {
      result.reasonB64 = line.slice(11).trim();
      continue;
    }
    if (line.startsWith("STDOUT_B64:")) {
      result.stdoutB64 = line.slice(11).trim();
      continue;
    }
    if (line.startsWith("STDERR_B64:")) {
      result.stderrB64 = line.slice(11).trim();
      continue;
    }
    if (line.startsWith("COMBINED_B64:")) {
      result.combinedB64 = line.slice(13).trim();
    }
  }

  return result;
}

function parseEnvelope(lines) {
  if (lines.length === 0) {
    return null;
  }
  if (lines[0].startsWith("TASK ")) {
    return parseTaskEnvelope(lines);
  }
  if (lines[0].startsWith("COMMAND_RESULT ")) {
    return parseCommandResultEnvelope(lines);
  }
  return {
    type: "unknown",
    raw: lines.join("\n")
  };
}

const taskQueue = [];
const resultQueue = [];
const taskWaiters = [];
const resultWaiters = [];

function deliverEnvelope(envelope) {
  if (!envelope) {
    return;
  }

  if (envelope.type === "task") {
    const waiter = taskWaiters.shift();
    if (waiter) {
      waiter(envelope);
    } else {
      taskQueue.push(envelope);
    }
    return;
  }

  if (envelope.type === "command_result") {
    const waiter = resultWaiters.shift();
    if (waiter) {
      waiter(envelope);
    } else {
      resultQueue.push(envelope);
    }
    return;
  }

  console.log(`adapter-warning: ignored unknown stdin envelope ${safeString(envelope.raw)}`);
}

function takeNextTask() {
  if (taskQueue.length > 0) {
    return Promise.resolve(taskQueue.shift());
  }
  return new Promise((resolve) => {
    taskWaiters.push(resolve);
  });
}

function takeNextCommandResult() {
  if (resultQueue.length > 0) {
    return Promise.resolve(resultQueue.shift());
  }
  return new Promise((resolve) => {
    resultWaiters.push(resolve);
  });
}

async function runAgentTask(task) {
  const { websocket, state } = await connectGateway();
  const sessionKey = `${sessionKeyPrefix}:${targetAgentId}:${state.device.id.slice(0, 12)}:${task.id || randomUUID()}`;

  try {
    let message = taskToMessage(task);
    let turn = 0;

    while (true) {
      const turnResult = await runAgentTurn(websocket, sessionKey, message, task, turn === 0);
      if (!turnResult.requestedBroker) {
        break;
      }

      const commandResult = await takeNextCommandResult();
      console.log(`adapter-status: received broker result ${commandResult.requestId} (${commandResult.status})`);
      message = commandResultToMessage(commandResult);
      turn += 1;
    }
  } finally {
    websocket.close();
  }
}

let processing = false;
let inputClosed = false;

async function drainQueue() {
  if (processing) {
    return;
  }
  processing = true;

  while (taskQueue.length > 0 || !inputClosed) {
    if (taskQueue.length === 0) {
      const nextTask = await takeNextTask();
      taskQueue.unshift(nextTask);
    }

    const task = taskQueue.shift();
    if (!task) {
      continue;
    }

    try {
      await runAgentTask(task);
    } catch (error) {
      console.error(`adapter-error: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  processing = false;
}

const rl = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity
});

console.log("adapter-status: OpenClaw Gateway adapter ready");
console.log(`adapter-status: target agent ${targetAgentId} via ${gatewayUrl}`);
console.log("adapter-status: broker-only mode enabled; emit PROXY_CMD/PROXY_JSON through the model");

let envelopeLines = [];

rl.on("line", (line) => {
  if (!line.trim()) {
    const parsed = parseEnvelope(envelopeLines);
    if (parsed) {
      deliverEnvelope(parsed);
      void drainQueue();
    }
    envelopeLines = [];
    return;
  }

  envelopeLines.push(line);
});

rl.on("close", () => {
  inputClosed = true;
  if (envelopeLines.length > 0) {
    const parsed = parseEnvelope(envelopeLines);
    if (parsed) {
      deliverEnvelope(parsed);
    }
    envelopeLines = [];
  }
  void drainQueue().finally(() => process.exit(0));
});
