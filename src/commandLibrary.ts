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
    items: [
      command("cd"),
      command("dir"),
      command("pwd"),
      command("get-childitem"),
      command("get-location"),
      command("get-content"),
      command("select-string"),
      command("test-path"),
      command("type")
    ]
  },
  {
    id: "interactive-replies",
    label: "Interactive Replies",
    description: "Harmless confirmation-style replies commonly used by installers and setup wizards.",
    items: [command("y"), command("n"), command("yes"), command("no")]
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
      domain("localhost"),
      domain("openclaw.ai")
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
  },
  {
    id: "onboarding-windows",
    label: "Windows Onboarding",
    description: "Low-level commands commonly needed for OpenClaw and Windows onboarding flows.",
    items: [command("node"), command("npm"), command("openclaw"), command("powershell"), command("get-content"), command("select-string"), command("test-path")]
  }
];
