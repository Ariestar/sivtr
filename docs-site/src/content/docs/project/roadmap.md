---
title: Roadmap
description: Directional product roadmap for sivtr and the broader personal AI workspace.
---

This roadmap explains the direction of `sivtr`, not a release contract. It is intentionally outcome-based: the project should stay useful as a small terminal tool while growing into infrastructure for long-lived AI work.

## Product direction

`sivtr` starts from a practical problem: terminal output, command history, and AI-agent conversations are valuable, but they are usually scattered across scrollback, JSONL transcripts, copied snippets, and editor tabs.

The long-term direction is to make this material reusable:

- capture important work at the moment it happens;
- normalize terminal output and AI conversation records into stable blocks;
- make those blocks searchable, selectable, exportable, and explainable;
- use the accumulated record as a foundation for a personal AI-era profile.

## Current focus: stronger CLI

The near-term priority is to make the command-line surface more complete, predictable, and scriptable.

Expected work includes:

- improve command naming, help text, and option consistency across `copy`, `history`, `codex`, `hotkey`, and workspace picker flows;
- make selectors and filters easier to compose in shell scripts;
- strengthen history import, export, and search behavior for larger local archives;
- keep config files explicit, portable, and safe to share;
- preserve compatibility with ordinary terminal workflows instead of requiring a custom shell or terminal emulator.

The goal is that `sivtr` can be trusted as a daily command-line utility before it becomes a larger personal data layer.

## Agent support

AI sessions are becoming a first-class capture source. `sivtr` already treats Codex and Claude-style transcripts as structured conversation records. The next step is to make provider support more systematic.

Planned work includes:

- support more agent providers behind a common session-provider interface;
- keep provider-specific parsing isolated from shared selection, search, and export logic;
- make session discovery robust across local, mirrored, and shared transcript directories;
- expose provider selection consistently in CLI commands, hotkeys, and the TUI workspace;
- avoid locking the data model to one vendor's transcript format.

The product principle is simple: agent transcripts should feel like a normal `sivtr` source, not a special-case feature.

## TUI improvements

The TUI should remain fast and keyboard-first, but it needs to scale from single-output browsing to multi-source workspace navigation.

Planned work includes:

- refine the workspace picker for many sessions, many providers, and long conversations;
- improve search scope, result navigation, and visual feedback;
- make selection behavior consistent between terminal output, command blocks, and AI dialogue blocks;
- improve rendering of markdown, tool calls, and structured agent content;
- keep the interface dense and predictable rather than decorative.

The TUI is not meant to replace an editor. Its job is to make finding and copying the right piece of work faster than opening raw logs.

## Later: sivtr-me

After the CLI and workspace foundations are stable, the larger direction is `sivtr-me`: a personal profile generated from a user's accumulated work records.

The idea is to turn terminal sessions, AI conversations, project history, and selected artifacts into an AI-era personal card. Unlike a static resume, `sivtr-me` should be evidence-backed and continuously updated from real work.

Possible capabilities include:

- summarize a person's projects, tools, domains, and working style from their actual records;
- surface representative conversations, decisions, code changes, debugging traces, and shipped outcomes;
- build a public or private profile that can answer "what has this person actually worked on?";
- support selective disclosure so sensitive records stay local while high-signal summaries can be shared;
- preserve provenance from every displayed claim back to underlying sessions or artifacts.

The guiding constraint is trust. A personal AI profile is only useful if users can see where each claim came from and decide what should be visible.

## Non-goals

The roadmap does not imply that `sivtr` will become:

- a terminal emulator;
- a hosted transcript storage service by default;
- a vendor-specific wrapper for one AI assistant;
- a replacement for source control, issue trackers, or note-taking tools.

`sivtr` should stay small at the edge and structured at the core.

## Roadmap principles

- **Capture first.** Important work should be recorded when it happens, not reconstructed later from memory.
- **Local by default.** Personal transcripts and terminal history should remain under user control unless explicitly exported.
- **Provider-neutral.** Agent support should be implemented through replaceable providers and stable shared abstractions.
- **Composable CLI.** Every interactive feature should have a scriptable path where practical.
- **Provenance matters.** Summaries, profiles, and exports should be traceable back to source sessions and command output.
- **Editor-friendly.** `sivtr` should hand off to existing editors and workflows instead of trying to own the whole developer environment.
