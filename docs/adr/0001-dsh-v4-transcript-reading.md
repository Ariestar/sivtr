# Dsh v4 transcript reading

Status: accepted, 2026-09-30.

Sivtr reads human conversation history through the existing Dsh provider. It does
not restore a runnable Harness session or reconstruct model input.

- Support on-disk generations 0 and 4 explicitly; check filename/header agreement.
  Keep predecessor artifacts immutable and select the newest on-disk generation
  per session directory, preferring zstd within a generation. An invalid or
  unsupported successor suppresses predecessors rather than exposing stale history.
- Reuse JSONL parsing, concatenated-zstd decoding, content extraction, and block
  conversion. Native v4 tool messages have the same `toolCallId` and `content`
  fields consumed by the existing tool-result block conversion.
- In v4, include append-origin human prompts, assistant messages, and tool results.
  Exclude injected context and model-only replacement copies. Preserve v0 behavior.
- Validate the transcript fields consumed by the adapter. Skip known context/trace
  events; refuse unknown required events and skip unknown `ignorable: true` events.
  Keep the v4 event vocabulary aligned with the official catalog when extending it.
- Native v4 rejects the retired `source.kind: "plugin"` wrapper, including user
  context. Accept nonempty producer-owned context kinds and omit them from dialogue
  and fallback titles. Preserve the legacy plugin-context behavior only in v0.
- Use a new Dsh listing-cache namespace because unchanged directory stamps cannot
  reveal files omitted by the old discovery predicate. No shared cache change is needed.

Limits: the existing block model represents textual tool output, not Harness
lifecycle state, native tool-error flags, developer tool schemas, or binary media.
No archive schema, external interface, or dependency change is required. Reverting
the adapter changes restores legacy reading without modifying source logs.

References:

- [V4 persistence contract](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/persistence-changes/2026-09-16-session-format-v4.zh.md)
- [Append-origin transcript semantics](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/core/session/src/surface.ts)
- [Known event vocabulary](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/core/session/src/known-event-types.ts)
- [Native v4 source admission](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/session/session-format-v3-to-v4/src/message-sources.ts)
