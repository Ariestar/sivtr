import { execFile } from "node:child_process";
import * as vscode from "vscode";

let sivtrTerminal: vscode.Terminal | undefined;
let sivtrTerminalCwd: string | undefined;

const INSTALL_WITH_CARGO = "Install with cargo";
const OPEN_INSTALL_DOCS = "Open install docs";
const CONFIGURE_PATH = "Configure path";
const INSTALL_DOCS_URL = "https://github.com/Ariestar/sivtr#installation";

export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.commands.registerCommand("sivtr.pickCodex", pickCodex),
    vscode.window.onDidCloseTerminal((terminal) => {
      if (terminal === sivtrTerminal) {
        sivtrTerminal = undefined;
        sivtrTerminalCwd = undefined;
      }
    }),
  );
}

export function deactivate(): void {
  sivtrTerminal = undefined;
  sivtrTerminalCwd = undefined;
}

async function pickCodex(): Promise<void> {
  const workspaceFolder = resolveWorkspaceFolder();
  if (!workspaceFolder) {
    void vscode.window.showErrorMessage("sivtr: open a workspace folder first.");
    return;
  }

  const config = vscode.workspace.getConfiguration("sivtr");
  const command = config.get<string>("command", "sivtr").trim();
  const args = config.get<string[]>("args", ["copy", "codex", "--pick"]);
  const terminalName = config.get<string>("terminalName", "sivtr");
  const reuseTerminal = config.get<boolean>("reuseTerminal", true);

  if (!command) {
    void vscode.window.showErrorMessage("sivtr: setting `sivtr.command` is empty.");
    return;
  }

  if (!(await isSivtrAvailable(command, workspaceFolder.uri.fsPath))) {
    await promptInstallSivtr(terminalName, workspaceFolder.uri.fsPath, reuseTerminal);
    return;
  }

  const terminal = getTerminal(terminalName, workspaceFolder.uri.fsPath, reuseTerminal);
  terminal.show();
  terminal.sendText(buildCommandLine(command, args), true);
}

function isSivtrAvailable(command: string, cwd: string): Promise<boolean> {
  return new Promise((resolve) => {
    execFile(command, ["--version"], { cwd, timeout: 5000, windowsHide: true }, (error) => {
      resolve(!error);
    });
  });
}

async function promptInstallSivtr(
  terminalName: string,
  cwd: string,
  reuseTerminal: boolean,
): Promise<void> {
  const choice = await vscode.window.showErrorMessage(
    "sivtr CLI is required. Install it or configure `sivtr.command`.",
    INSTALL_WITH_CARGO,
    OPEN_INSTALL_DOCS,
    CONFIGURE_PATH,
  );

  if (choice === INSTALL_WITH_CARGO) {
    const terminal = getTerminal(terminalName, cwd, reuseTerminal);
    terminal.show();
    terminal.sendText("cargo install sivtr", true);
  } else if (choice === OPEN_INSTALL_DOCS) {
    await vscode.env.openExternal(vscode.Uri.parse(INSTALL_DOCS_URL));
  } else if (choice === CONFIGURE_PATH) {
    await vscode.commands.executeCommand(
      "workbench.action.openSettings",
      "sivtr.command",
    );
  }
}

function resolveWorkspaceFolder(): vscode.WorkspaceFolder | undefined {
  const activeEditor = vscode.window.activeTextEditor;
  if (activeEditor) {
    const folder = vscode.workspace.getWorkspaceFolder(activeEditor.document.uri);
    if (folder) {
      return folder;
    }
  }

  return vscode.workspace.workspaceFolders?.[0];
}

function getTerminal(name: string, cwd: string, reuse: boolean): vscode.Terminal {
  if (
    reuse &&
    sivtrTerminal &&
    sivtrTerminal.exitStatus === undefined &&
    sivtrTerminalCwd === cwd
  ) {
    return sivtrTerminal;
  }

  sivtrTerminal = vscode.window.createTerminal({
    name,
    cwd,
  });
  sivtrTerminalCwd = cwd;
  return sivtrTerminal;
}

function buildCommandLine(command: string, args: string[]): string {
  return [command, ...args].map(quoteShellToken).join(" ");
}

function quoteShellToken(value: string): string {
  if (/^[A-Za-z0-9_./:@%+=,-]+$/.test(value)) {
    return value;
  }

  return `'${value.replace(/'/g, "''")}'`;
}
