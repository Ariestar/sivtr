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

test("buildCommandLine preserves a cwd with single quotes", () => {
  assert.equal(
    buildCommandLine("sivtr", [
      "hotkey-pick-codex",
      "--cwd",
      "/tmp/it's project",
    ]),
    "sivtr hotkey-pick-codex --cwd '/tmp/it'\\''s project'",
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
});
