export type LibraryItemKind = "command" | "domain";

export interface LibraryItem {
  kind: LibraryItemKind;
  value: string;
}

export interface CommandGroup {
  id: string;
  label: string;
  description: string;
  items: LibraryItem[];
}

function command(value: string): LibraryItem {
  return { kind: "command", value };
}

function domain(value: string): LibraryItem {
  return { kind: "domain", value };
}

export const COMMAND_LIBRARY: CommandGroup[] = [
  {
    id: "workspace-read",
    label: "Workspace Read",
    description: "Inspect files and current location without changing the workspace.",
    items: [command("cd"), command("dir"), command("pwd"), command("get-childitem"), command("get-location"), command("type")]
  },
  {
    id: "version-control",
    label: "Version Control",
    description: "Read and operate on git repositories.",
    items: [command("git")]
  },
  {
    id: "package-managers",
    label: "Package Managers",
    description: "Dependency managers and build tool entrypoints.",
    items: [command("npm"), command("pnpm"), command("yarn"), command("cargo")]
  },
  {
    id: "network-fetch",
    label: "Network Fetch",
    description: "Commands and domains commonly used to retrieve remote content.",
    items: [
      command("curl"),
      command("wget"),
      command("iwr"),
      command("irm"),
      command("ping"),
      command("nslookup"),
      domain("localhost")
    ]
  },
  {
    id: "scripting-runtimes",
    label: "Scripting Runtimes",
    description: "General runtimes with broad execution capability.",
    items: [command("python"), command("node"), command("powershell"), command("cmd")]
  },
  {
    id: "orc-tools",
    label: "O.R.C. Tools",
    description: "Commands commonly used for O.R.C. and claw workflows.",
    items: [command("openclaw"), command("nemoclaw"), command("orc"), command("orc-cli")]
  }
];
