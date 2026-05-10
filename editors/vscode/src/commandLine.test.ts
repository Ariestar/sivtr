import test from "node:test";
import assert from "node:assert/strict";

import {
  buildCommandLine,
  buildTerminalCommandLine,
  quoteShellToken,
  resolveArgs,
} from "./commandLine";

test("quoteShellToken escapes single quotes for POSIX shells", () => {
  assert.equal(
    quoteShellToken("/tmp/it's project"),
    "'/tmp/it'\\''s project'",
  );
});

test("quoteShellToken escapes single quotes for PowerShell", () => {
  assert.equal(
    quoteShellToken("C:\\Users\\O'Connor\\project", "powershell"),
    "'C:\\Users\\O''Connor\\project'",
  );
});

test("quoteShellToken uses double quotes for cmd.exe", () => {
  assert.equal(
    quoteShellToken("C:\\Program Files\\sivtr", "cmd"),
    "\"C:\\Program Files\\sivtr\"",
  );
});

test("buildCommandLine preserves a cwd with single quotes", () => {
  assert.equal(
    buildCommandLine("sivtr", [
      "hotkey-pick-codex",
      "--cwd",
      "/tmp/it's project",
    ], "/usr/bin/bash"),
    "sivtr hotkey-pick-codex --cwd '/tmp/it'\\''s project'",
  );
});

test("buildCommandLine uses shell-aware quoting for Windows shells", () => {
  assert.equal(
    buildCommandLine("sivtr", [
      "hotkey-pick-codex",
      "--cwd",
      "C:\\Users\\O'Connor\\workspace",
    ], "C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
    "sivtr hotkey-pick-codex --cwd 'C:\\Users\\O''Connor\\workspace'",
  );
});

test("resolveArgs expands workspaceFolder placeholder only after --cwd", () => {
  assert.deepEqual(
    resolveArgs(["hotkey-pick-codex", "--cwd", "${workspaceFolder}", "--session", "2"], "/tmp/repo"),
    ["hotkey-pick-codex", "--cwd", "/tmp/repo", "--session", "2"],
  );
});

test("buildTerminalCommandLine appends shell-specific close logic", () => {
  assert.equal(
    buildTerminalCommandLine("sivtr hotkey-pick-codex", true, "/usr/bin/bash"),
    'sivtr hotkey-pick-codex; code=$?; if [ "$code" -eq 0 ]; then exit 0; fi',
  );
  assert.equal(
    buildTerminalCommandLine("sivtr hotkey-pick-codex", true, "C:\\Windows\\System32\\cmd.exe"),
    "sivtr hotkey-pick-codex & if not errorlevel 1 exit",
  );
  assert.equal(
    buildTerminalCommandLine("sivtr hotkey-pick-codex", true, "C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
    "sivtr hotkey-pick-codex; if ($LASTEXITCODE -eq 0) { exit }",
  );
});
