---
title: Roadmap
description: Directional product roadmap for sivtr and the broader agent memory workspace.
---

This roadmap is a working plan, not a release contract. It describes the direction of `sivtr` in outcome terms so the project can stay useful as a small terminal tool while growing into a unified agent memory workspace for humans and agents.

## Roadmap map

```text
Reliable CLI
  -> Multi-agent workspace
    -> High-quality retrieval
      -> Skills, MCP, and agent interfaces
        -> High-signal TUI
          -> Remote collaboration + privacy lifecycle
            -> Provenance, trust, and memory hygiene
              -> Semantic / multimodal memory
                -> A2A messenger + sivtr-me
```

| Track | Status | Target outcome |
| --- | --- | --- |
| CLI foundation | In progress | A daily command-line utility for capturing, searching, selecting, and exporting terminal and agent work. |
| Agent support | In progress | Provider-neutral parsing and browsing for AI-agent conversation records. |
| Retrieval quality | Next | Structured, precise, rankable search that makes the evidence paradigm trustworthy at scale. |
| Skills and playbooks | In progress | Reusable agent procedures that use `sivtr` as the unified memory entry point. |
| Agent interfaces | In progress | CLI, MCP, and later local API / SDK surfaces so other agents can treat `sivtr` as work-memory infrastructure. |
| TUI workspace | Planned | A dense keyboard-first interface for many sessions, many providers, and long conversations. |
| Source expansion | Planned | More capture surfaces beyond current shells and coding agents, without weakening the shared model. |
| Remote collaboration | Landed (core) | Permissioned, read-only access to teammate workspace memory via Share / Grant / Mount. |
| Privacy and lifecycle | Planned | Redaction, retention, expiry, and selective disclosure so sensitive work does not leak or rot in place. |
| Provenance and trust | Planned | Source-traced, versioned, trust-scored memory that can be verified and pruned. |
| Semantic and multimodal | Later | Optional vector / multimodal retrieval layered on top of structured evidence, not instead of it. |
| A2A messenger | Later | Client-only A2A handoff of selected WorkRef / WorkPart content to other agents. |
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

## Retrieval quality

Retrieval quality decides whether the evidence paradigm is actually usable. `sivtr` should make structured search excellent before adding semantic layers.

- [ ] Expand search beyond basic matching toward explicit scopes, literal / keyword / fuzzy methods, source filters, ranking, and context-rich machine-readable results.
- [ ] Keep progressive disclosure as the default: compact refs first, full content only when selected.
- [ ] Improve recency, status, provider, session, and part-kind ranking so useful evidence surfaces first.
- [ ] Make search results stable enough for scripts and agents: deterministic ranking options, rich JSON, and WorkRef-preserving output.
- [ ] Add evaluation fixtures and golden queries so retrieval changes can be measured, not only felt.
- [ ] Treat semantic / vector search as an optional method on top of this foundation, not a replacement for structured filters.

## Skills and playbooks

Skills make `sivtr` usable by agents as a shared memory entry point. They turn generic memory commands into reusable procedures such as "fix the latest terminal error," "continue from the last task," or "write a timeline of recent work."

- [x] Add an initial `skills/sivtr-memory/` package with command recipes, evidence discipline, workflows, and examples.
- [x] Document why skills are part of the product model rather than just optional prompt snippets.
- [ ] Define a stable packaging convention for community skills and local team playbooks.
- [ ] Build a skill registry so users can discover workflows such as terminal-failure debugging, timeline generation, PR handoff, recap, and onboarding.
- [ ] Add examples that show agents using refs and validation evidence from workspace memory.
- [ ] Keep skill procedures grounded in existing CLI commands; do not let community playbooks imply unavailable `sivtr` features.

## Agent interfaces

`sivtr` should become work-memory infrastructure that other agents can call directly. Interfaces should expose the same evidence model, not invent parallel APIs.

- [x] CLI surface for capture, search, show, filter, nav, zoom, copy, and remote memory.
- [x] Read-only MCP server and host install flow.
- [ ] Stabilize MCP tool contracts around WorkRef / WorkSet semantics and progressive disclosure.
- [ ] Add a local developer API for programmatic query and export without shelling out when needed.
- [ ] Publish a thin SDK or client library only after CLI / MCP contracts stabilize.
- [ ] Keep every interface evidence-first: return refs, provenance, and selectable parts rather than opaque blobs.
- [ ] Prefer local, opt-in services over always-on cloud endpoints.

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

## Source expansion

More platforms should widen capture, not fragment the model. New sources must map into the shared WorkRecord / WorkPart / WorkRef abstractions.

- [ ] Add more coding-agent and IDE transcript providers behind the shared provider interface.
- [ ] Support additional shells and terminal capture paths where the existing hook model is insufficient.
- [ ] Explore importers for web AI conversations and collaboration tools only when durable local exports or APIs exist.
- [ ] Prefer offline-first import and local indexes over scraping fragile remote UIs.
- [ ] Keep provider adapters thin; search, privacy, ranking, and export stay shared.
- [ ] Reject sources that cannot preserve provenance back to an original session or artifact.

## Remote collaboration

Remote collaboration extends the local memory model to permissioned teammate records. The goal is not to become a hosted transcript service; it is to let explicit collaborators mount relevant workspace memory so agents can coordinate across machines.

Core model landed: **Device Daemon + Identity + Share + Grant + Mount** over encrypted iroh transport. Refs use `origin:body` (`desk:terminal/...`).

- [x] Device daemon with auto-start (`sivtr serve`).
- [x] Explicit workspace sharing (`sivtr share` / `share add` / `invite` / `grants` / `revoke`).
- [x] Workspace-local mounts (`sivtr remote add|list|remove|rename|test`).
- [x] Peer identity list/forget (`sivtr peer`).
- [x] Remote origins in WorkRef (`origin:body`) for search, show, filter, nav, zoom, copy.
- [x] Default secret redaction before data leaves the machine.
- [x] Local workspace origin labels (`sivtr ws list`).
- [ ] Identity CLI (`identity show|rotate|export`).
- [ ] Share audit log and per-share redact toggle CLI.
- [ ] Daemon autostart on login (`serve enable|disable`).
- [ ] Peer rename / verify / disconnect helpers.
- [ ] UDS or named-pipe control plane instead of localhost TCP.
- [ ] Protocol version negotiation for older servers.
- [ ] Richer selective disclosure (share specific sessions, not whole workspaces).

## Privacy and lifecycle

Permissioned sharing is not enough. Memory must also be safe to keep, safe to share, and safe to forget.

- [x] Default secret redaction on remote share paths.
- [ ] Expand redaction rules for tokens, keys, cookies, env dumps, and other high-risk patterns.
- [ ] Support private tags / exclude markers so sensitive spans never enter durable indexes.
- [ ] Add retention and expiry policies for local archives and shared mounts.
- [ ] Support forget / purge flows that remove records, indexes, and remote grants consistently.
- [ ] Make data-lifecycle actions auditable: what was shared, redacted, retained, or deleted.
- [ ] Keep privacy controls local-first and explicit; no silent cloud offload.

## Provenance and trust

Evidence is only useful if callers can verify where it came from and whether it is still current.

- [ ] Preserve source provenance on every record, part, summary, and export.
- [ ] Track memory versions so re-imported or re-parsed sessions do not silently overwrite history.
- [ ] Attach trust / freshness signals such as capture time, source reliability, and supersession.
- [ ] Support expiry and quarantine of stale or contradicted memory so old data does not pollute retrieval.
- [ ] Make search and profile surfaces show enough provenance for a human or agent to re-open the original evidence.
- [ ] Prefer citeable WorkRefs over free-floating regenerated summaries.

## Semantic and multimodal memory

Semantic and multimodal retrieval can raise the ceiling, but only after structured evidence search is strong.

- [ ] Optional local vector / embedding index as one search method alongside literal, keyword, and fuzzy.
- [ ] Keep hybrid retrieval: structured filters first, semantic ranking second.
- [ ] Avoid mandatory cloud embedding providers; local or user-chosen backends only.
- [ ] Index multimodal artifacts only when they can be addressed by stable refs and reopened later.
- [ ] Support locating historical images and other non-text artifacts without discarding text provenance.
- [ ] Never let embeddings become the sole source of truth; raw records remain authoritative.

## A2A messenger

A2A is a separate push-out feature: `sivtr` does not run an LLM. It selects structured evidence and posts it to another agent.

- [ ] Client-only A2A messenger that wraps selected WorkRef / WorkPart / WorkSet content as A2A Message / Artifact payloads.
- [ ] Target Agent Card endpoints over HTTP + JSON-RPC 2.0 without turning `sivtr` into an agent runtime.
- [ ] Keep A2A independent from remote collaboration: share/mount is pull-in; A2A is push-out.
- [ ] Gate the feature until WorkRef selection and privacy redaction are solid enough for outbound handoff.
- [ ] Prefer a minimal protocol subset or a maintained Rust client over a heavy agent framework dependency.

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
- a replacement for source control, issue trackers, or note-taking tools;
- an automatic long-term memory compressor that silently rewrites history;
- a full agent runtime or multi-agent orchestrator;
- a cloud-first RAG platform that requires remote embeddings to work.

`sivtr` should stay small at the edge and structured at the core.

## Principles

- **Capture first.** Important work should be recorded when it happens, not reconstructed later from memory.
- **Local by default.** Personal transcripts and terminal history should remain under user control unless explicitly shared or exported.
- **Provider-neutral.** Agent support should be implemented through replaceable providers and stable shared abstractions.
- **Evidence over paraphrase.** Prefer citeable raw records and WorkRefs over opaque regenerated summaries.
- **Structured search first.** Semantic and multimodal retrieval are additives, not substitutes for precise filters and refs.
- **Skills are interfaces.** A skill is how an agent learns to operate the shared memory layer; it should be precise, testable, and evidence-seeking.
- **Composable interfaces.** CLI, MCP, API, and SDK should expose the same model with scriptable paths where practical.
- **Provenance matters.** Summaries, profiles, and exports should be traceable back to source sessions and command output.
- **Privacy is a lifecycle.** Redaction, retention, expiry, and forget are product features, not afterthoughts.
- **Editor-friendly.** `sivtr` should hand off to existing editors and workflows instead of trying to own the whole developer environment.
