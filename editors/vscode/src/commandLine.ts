import * as path from "node:path";

type ShellKind = "cmd" | "posix" | "powershell";

function shellNamesFromPath(shellPathOrKind: string): string[] {
  const raw = shellPathOrKind.toLowerCase();
  const posixName = path.posix.basename(shellPathOrKind).toLowerCase();
  const winName = path.win32.basename(shellPathOrKind).toLowerCase();
  return Array.from(new Set([raw, posixName, winName]));
}

function shellKindFromPath(shellPathOrKind: string): ShellKind {
  const normalized = shellPathOrKind.toLowerCase();
  if (
    normalized === "cmd" ||
    normalized === "posix" ||
    normalized === "powershell"
  ) {
    return normalized;
  }

  const shellNames = shellNamesFromPath(shellPathOrKind);
  if (shellNames.some((name) => name.includes("powershell") || name.includes("pwsh"))) {
    return "powershell";
  }
  if (shellNames.some((name) => name === "cmd.exe" || name === "cmd")) {
    return "cmd";
  }

  return "posix";
}

export function resolveArgs(
  configured: string[],
  cwd: string,
): string[] {
  const resolveCwdValue = (value: string): string => {
    if (value === ".") {
      return cwd;
    }
    if (value.startsWith("./")) {
      return path.join(cwd, value.slice(2));
    }
    return value;
  };

  const expandWorkspaceFolder = (value: string): string =>
    value.split("${workspaceFolder}").join(cwd);

  return configured.map((arg, index, args) => {
    if (args[index - 1] === "--cwd") {
      return resolveCwdValue(expandWorkspaceFolder(arg));
    }
    if (arg.startsWith("--cwd=")) {
      const value = arg.slice("--cwd=".length);
      return `--cwd=${resolveCwdValue(expandWorkspaceFolder(value))}`;
    }
    return expandWorkspaceFolder(arg);
  });
}

export function buildCommandLine(
  command: string,
  args: string[],
  shellPath: string,
): string {
  const shellKind = shellKindFromPath(shellPath);
  return [command, ...args]
    .map((value) => quoteShellToken(value, shellKind))
    .join(" ");
}

export function buildTerminalCommandLine(
  commandLine: string,
  closeOnSuccess: boolean,
  shellPath: string,
): string {
  if (!closeOnSuccess) {
    return commandLine;
  }

  const shellKind = shellKindFromPath(shellPath);
  if (shellKind === "powershell") {
    return `${commandLine}; if ($LASTEXITCODE -eq 0) { exit }`;
  }
  if (shellKind === "cmd") {
    return `${commandLine} & if not errorlevel 1 exit`;
  }

  if (shellNamesFromPath(shellPath).some((name) => name.includes("fish"))) {
    return `${commandLine}; test $status -eq 0; and exit`;
  }

  return `${commandLine}; code=$?; if [ "$code" -eq 0 ]; then exit 0; fi`;
}

export function quoteShellToken(value: string, shellPathOrKind = ""): string {
  const shellKind = shellKindFromPath(shellPathOrKind);
  if (/^[A-Za-z0-9_./:@+=,-]+$/.test(value)) {
    return value;
  }

  if (shellKind === "powershell") {
    return `'${value.replace(/'/g, "''")}'`;
  }
  if (shellKind === "cmd") {
    return `"${value.replace(/"/g, "\"\"")}"`;
  }

  return `'${value.replace(/'/g, `'\\''`)}'`;
}
