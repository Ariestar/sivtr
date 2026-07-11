---
title: Roadmap
description: Directional product roadmap for sivtr and the broader agent memory workspace.
---

This roadmap is a working plan, not a release contract. It describes the direction of `sivtr` in outcome terms so the project can stay useful as a small terminal tool while growing into a unified agent memory workspace for humans and agents.

## Roadmap map

```text
Reliable CLI
  -> Multi-agent workspace
    -> Skills and playbooks
      -> High-signal TUI
        -> Remote collaboration
          -> sivtr-me
```

| Track | Status | Target outcome |
| --- | --- | --- |
| CLI foundation | In progress | A daily command-line utility for capturing, searching, selecting, and exporting terminal and agent work. |
| Agent support | In progress | Provider-neutral parsing and browsing for AI-agent conversation records. |
| Skills and playbooks | In progress | Reusable agent procedures that use `sivtr` as the unified memory entry point. |
| TUI workspace | Planned | A dense keyboard-first interface for many sessions, many providers, and long conversations. |
| Remote collaboration | Landed (core) | Permissioned, read-only access to teammate workspace memory via Share / Grant / Mount. |
| `sivtr-me` | Later | An evidence-backed personal AI-era profile built from real work records. |

## CLI foundation

The near-term priority is to make the command-line surface complete, predictable, and scriptable. `sivtr` should be trustworthy as a daily utility before it becomes a broader personal data layer.

- [x] Capture command output from pipe mode.
- [x] Capture subprocess output with `sivtr run`.
- [x] Import shell session logs.
- [x] Copy recent command input, output, and command blocks by selector.
- [x] Search saved output history with SQLite.
- [x] Provide TOML configuration for core behavior.
- [ ] Tighten command naming and option consistency across `copy`, `history`, `codex`, `hotkey`, and workspace flows.
- [ ] Make selectors and filters easier to compose in shell scripts.
- [ ] Expand search beyond basic matching toward explicit scopes, literal/keyword/fuzzy/semantic methods, source filters, ranking, and context-rich machine-readable results.
- [ ] Strengthen import, export, and search behavior for larger local archives.
- [ ] Keep configuration explicit, portable, and safe to share.

## Agent support

Agent sessions are a first-class memory source. The product goal is for agent transcripts to behave like normal `sivtr` sources rather than special-case features.

- [x] Parse Codex session records.
- [x] Parse Claude-style session records.
- [x] Parse Hermes, OpenCode, and Pi session records.
- [x] Copy the latest user, assistant, tool, turn, or full session block.
- [x] Browse local and mirrored session directories through picker workflows.
- [ ] Support more agent providers behind the shared session-provider interface.
- [ ] Keep provider-specific parsing isolated from shared selection, search, and export logic.
- [ ] Make session discovery robust across local, mirrored, and shared transcript directories.
- [ ] Expose provider selection consistently in CLI commands, hotkeys, and the TUI workspace.
- [ ] Avoid binding the data model to one vendor's transcript format.

## Skills and playbooks

Skills make `sivtr` usable by agents as a shared memory entry point. They turn generic memory commands into reusable procedures such as "fix the latest terminal error," "continue from the last task," or "write a timeline of recent work."

- [x] Add an initial `skills/sivtr-memory/` package with command recipes, evidence discipline, workflows, and examples.
- [x] Document why skills are part of the product model rather than just optional prompt snippets.
- [ ] Define a stable packaging convention for community skills and local team playbooks.
- [ ] Build a skill registry so users can discover workflows such as terminal-failure debugging, timeline generation, PR handoff, recap, and onboarding.
- [ ] Add examples that show agents using refs and validation evidence from workspace memory.
- [ ] Keep skill procedures grounded in existing CLI commands; do not let community playbooks imply unavailable `sivtr` features.

## TUI workspace

The TUI should remain fast and keyboard-first, but it needs to scale from single-output browsing to multi-source workspace navigation.

- [x] Browse captured output in a Vim-style terminal UI.
- [x] Search within captured output.
- [x] Select character, line, and block ranges.
- [x] Pick sessions and dialogue blocks interactively.
- [ ] Refine the workspace picker for many sessions, providers, and long conversations.
- [ ] Improve search scope, result navigation, and visual feedback.
- [ ] Make selection behavior consistent across terminal output, command blocks, and AI dialogue blocks.
- [ ] Improve rendering for markdown, tool calls, and structured agent content.
- [ ] Keep the interface dense, predictable, and editor-friendly.

## Remote collaboration

Remote collaboration extends the local memory model to permissioned teammate records. The goal is not to become a hosted transcript service; it is to let explicit collaborators mount relevant workspace memory so agents can coordinate across machines.

Core model landed: **Device Daemon + Identity + Share + Grant + Mount** over encrypted iroh transport. Refs use `origin:body` (`desk:terminal/...`).

- [x] Device daemon with auto-start (`sivtr serve`).
- [x] Explicit workspace sharing (`sivtr share` / `share add` / `invite` / `grants` / `revoke`).
- [x] Workspace-local mounts (`sivtr remote add|list|remove|rename|test`).
- [x] Peer identity list/forget (`sivtr peer`).
- [x] Remote origins in WorkRef (`origin:body`) for search, show, filter, nav, zoom, copy.
- [x] Default secret redaction before data leaves the machine.
- [x] Local workspace origin labels (`sivtr wb list`).
- [ ] Identity CLI (`identity show|rotate|export`).
- [ ] Share audit log and per-share redact toggle CLI.
- [ ] Daemon autostart on login (`serve enable|disable`).
- [ ] Peer rename / verify / disconnect helpers.
- [ ] UDS or named-pipe control plane instead of localhost TCP.
- [ ] Protocol version negotiation for older servers.
- [ ] Richer selective disclosure (share specific sessions, not whole workspaces).

## sivtr-me

After the CLI and workspace foundations are stable, the larger direction is `sivtr-me`: a personal profile generated from accumulated work records. Unlike a static resume, it should be evidence-backed and continuously updated from real terminal sessions, AI conversations, project history, and selected artifacts.

- [ ] Define the local data model for long-lived personal work records.
- [ ] Summarize projects, tools, domains, and working style from real records.
- [ ] Surface representative conversations, decisions, code changes, debugging traces, and shipped outcomes.
- [ ] Build a public or private profile that can answer "what has this person actually worked on?"
- [ ] Support selective disclosure so sensitive records stay local while high-signal summaries can be shared.
- [ ] Preserve provenance from every displayed claim back to underlying sessions or artifacts.

## Non-goals

The roadmap does not imply that `sivtr` will become:

- a terminal emulator;
- a hosted transcript storage service by default;
- an unrestricted remote chat mirror without explicit permission;
- a vendor-specific wrapper for one AI assistant;
- a replacement for source control, issue trackers, or note-taking tools.

`sivtr` should stay small at the edge and structured at the core.

## Principles

- **Capture first.** Important work should be recorded when it happens, not reconstructed later from memory.
- **Local by default.** Personal transcripts and terminal history should remain under user control unless explicitly shared or exported.
- **Provider-neutral.** Agent support should be implemented through replaceable providers and stable shared abstractions.
- **Skills are interfaces.** A skill is how an agent learns to operate the shared memory layer; it should be precise, testable, and evidence-seeking.
- **Composable CLI.** Every interactive feature should have a scriptable path where practical.
- **Provenance matters.** Summaries, profiles, and exports should be traceable back to source sessions and command output.
- **Editor-friendly.** `sivtr` should hand off to existing editors and workflows instead of trying to own the whole developer environment.
