import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "xterm-addon-fit";
import { Terminal } from "xterm";
import { resizeTerminal } from "./api";
import type { CommandSession, TerminalOutputEvent } from "./types";

interface TerminalViewProps {
  session: CommandSession | null;
}

export function TerminalView({ session }: TerminalViewProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const lastSessionRef = useRef<string | null>(null);
  const sessionRef = useRef<CommandSession | null>(null);

  sessionRef.current = session;

  useEffect(() => {
    if (!hostRef.current || terminalRef.current) {
      return;
    }

    const terminal = new Terminal({
      convertEol: true,
      cursorBlink: true,
      fontFamily: "IBM Plex Mono, Consolas, monospace",
      fontSize: 14,
      theme: {
        background: "#081018",
        foreground: "#edf2ff",
        cursor: "#66d9c1",
        black: "#081018",
        green: "#7fe395",
        brightGreen: "#7fe395",
        yellow: "#ffcc66",
        brightYellow: "#ffcc66",
        red: "#ff6d7a",
        brightRed: "#ff6d7a",
        blue: "#8cc8ff",
        brightBlue: "#8cc8ff"
      }
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(hostRef.current);
    fitAddon.fit();

    terminalRef.current = terminal;
    fitRef.current = fitAddon;

    const resizeObserver = new ResizeObserver(() => {
      if (!terminalRef.current || !fitRef.current || !sessionRef.current) {
        return;
      }

      fitRef.current.fit();
      void resizeTerminal(sessionRef.current.id, terminalRef.current.cols, terminalRef.current.rows);
    });
    resizeObserver.observe(hostRef.current);

    return () => {
      resizeObserver.disconnect();
      terminal.dispose();
      terminalRef.current = null;
      fitRef.current = null;
    };
  }, [session]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !session) {
      return;
    }

    if (lastSessionRef.current !== session.id) {
      terminal.reset();
      session.lines.forEach((line) => terminal.write(line));
      lastSessionRef.current = session.id;
      if (fitRef.current) {
        fitRef.current.fit();
      }
      void resizeTerminal(session.id, terminal.cols, terminal.rows);
    }
  }, [session]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) {
      return;
    }

    let disposed = false;

    const stopOutput = listen<TerminalOutputEvent>("terminal-output", (event) => {
      if (!disposed && session && event.payload.sessionId === session.id) {
        terminal.write(event.payload.data);
      }
    });

    return () => {
      disposed = true;
      void stopOutput.then((unlisten) => unlisten());
    };
  }, [session]);

  return <div className="terminal-host" ref={hostRef} />;
}
