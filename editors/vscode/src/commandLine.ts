import * as path from "node:path";

export function resolveArgs(
  configured: string[],
  cwd: string,
): string[] {
  return configured.map((arg, index, args) => {
    if ((arg === "." || arg === "${workspaceFolder}") && args[index - 1] === "--cwd") {
      return cwd;
    }
    return arg;
  });
}

export function buildCommandLine(
  command: string,
  args: string[],
  shellPath?: string,
): string {
  return [command, ...args].map((arg) => quoteShellToken(arg, shellPath)).join(" ");
}

export function buildTerminalCommandLine(
  commandLine: string,
  closeOnSuccess: boolean,
  shellPath: string,
): string {
  if (!closeOnSuccess) {
    return commandLine;
  }

  const shellName = detectShellName(shellPath);
  if (shellName.includes("powershell") || shellName.includes("pwsh")) {
    return `${commandLine}; if ($LASTEXITCODE -eq 0) { exit }`;
  }
  if (shellName === "cmd.exe" || shellName === "cmd") {
    return `${commandLine} & if not errorlevel 1 exit`;
  }
  if (shellName.includes("fish")) {
    return `${commandLine}; test $status -eq 0; and exit`;
  }

  return `${commandLine}; code=$?; if [ "$code" -eq 0 ]; then exit 0; fi`;
}

export function quoteShellToken(value: string, shellPath?: string): string {
  if (/^[A-Za-z0-9_./:@%+=,-]+$/.test(value)) {
    return value;
  }

  const shellName = shellPath ? detectShellName(shellPath) : "";
  if (shellName.includes("powershell") || shellName.includes("pwsh")) {
    return `'${value.replace(/'/g, "''")}'`;
  }
  if (shellName === "cmd.exe" || shellName === "cmd") {
    return `"${value.replace(/"/g, "\"\"")}"`;
  }

  return `'${value.replace(/'/g, `'\\''`)}'`;
}

function detectShellName(shellPath: string): string {
  return shellPath
    .split(/[\\/]/)
    .at(-1)
    ?.toLowerCase()
    ?? path.basename(shellPath).toLowerCase();
}
