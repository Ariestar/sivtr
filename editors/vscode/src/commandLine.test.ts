import test from "node:test";
import assert from "node:assert/strict";
import * as path from "node:path";

import {
  buildCommandLine,
  buildTerminalCommandLine,
  quoteShellToken,
  resolveArgs,
} from "./commandLine";

test("quoteShellToken escapes single quotes for POSIX shells", () => {
  assert.equal(
    quoteShellToken("/tmp/it's project", "posix"),
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
    '"C:\\Program Files\\sivtr"',
  );
});

test("buildCommandLine preserves agent picker args with POSIX quoting", () => {
  assert.equal(
    buildCommandLine(
      "sivtr",
      ["hotkey-pick-agent", "--cwd", "/tmp/it's project", "--provider", "all"],
      "/usr/bin/bash",
    ),
    "sivtr hotkey-pick-agent --cwd '/tmp/it'\\''s project' --provider all",
  );
});

test("buildCommandLine uses shell-aware quoting for Windows shells", () => {
  assert.equal(
    buildCommandLine(
      "sivtr",
      ["hotkey-pick-agent", "--cwd", "C:\\Users\\O'Connor\\workspace"],
      "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
    ),
    "sivtr hotkey-pick-agent --cwd 'C:\\Users\\O''Connor\\workspace'",
  );
});

test("resolveArgs expands workspaceFolder placeholder in all arguments", () => {
  assert.deepEqual(
    resolveArgs(
      [
        "hotkey-pick-agent",
        "--cwd",
        "${workspaceFolder}",
        "--output",
        "${workspaceFolder}/logs",
      ],
      "/tmp/repo",
    ),
    ["hotkey-pick-agent", "--cwd", "/tmp/repo", "--output", "/tmp/repo/logs"],
  );
});

test("resolveArgs supports --cwd=<value> and relative cwd values", () => {
  assert.deepEqual(
    resolveArgs(["hotkey-pick-agent", "--cwd=.", "--provider", "all"], "/tmp/repo"),
    ["hotkey-pick-agent", "--cwd=/tmp/repo", "--provider", "all"],
  );
  assert.deepEqual(
    resolveArgs(["hotkey-pick-agent", "--cwd", "./nested"], "/tmp/repo"),
    ["hotkey-pick-agent", "--cwd", path.join("/tmp/repo", "nested")],
  );
});

test("buildTerminalCommandLine appends shell-specific close logic", () => {
  assert.equal(
    buildTerminalCommandLine("sivtr hotkey-pick-agent", true, "/usr/bin/bash"),
    'sivtr hotkey-pick-agent; code=$?; if [ "$code" -eq 0 ]; then exit 0; fi',
  );
  assert.equal(
    buildTerminalCommandLine(
      "sivtr hotkey-pick-agent",
      true,
      "C:\\Windows\\System32\\cmd.exe",
    ),
    "sivtr hotkey-pick-agent & if not errorlevel 1 exit",
  );
  assert.equal(
    buildTerminalCommandLine(
      "sivtr hotkey-pick-agent",
      true,
      "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
    ),
    "sivtr hotkey-pick-agent; if ($LASTEXITCODE -eq 0) { exit }",
  );
});
