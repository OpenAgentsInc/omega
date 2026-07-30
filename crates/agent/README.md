# Omega Agent

`agent` is Omega's native, in-process coding-agent engine. It owns durable conversation state, language-model execution, tool dispatch, security enforcement, and the bridge to ACP sessions. It is a GPUI library (`src/agent.rs` is its library root), not a standalone service; `agent_ui` and the surrounding application provide presentation and host services.

## Overview and architecture

```text
agent_ui / ACP client
       │ user blocks, controls, streamed events
       ▼
NativeAgent ── session registry ── AcpThread
       │                               │
       ▼                               ▼
     Thread ─ request/context ─ LanguageModel stream
       │                               │
       ├─ built-in tools / MCP tools ◄─┘
       ├─ permissions and sandboxing
       ▼
ThreadStore ── ThreadsDatabase (SQLite)
```

`Thread` owns the durable agent semantics; `AcpThread` carries ACP protocol communication and the UI-facing terminal/session surface. `NativeAgent` coordinates both.

## Core components

### `NativeAgent`

`NativeAgent` maintains active sessions: an ACP session ID maps to a native `Thread`, its `AcpThread`, project ID, reference count, subscriptions, and pending save. It also owns per-project `ProjectContext`, discovered skills, a context-server registry, refresh channels/subscriptions, shared templates, and a cached authenticated language-model list.

Creating or reopening a session constructs/restores a `Thread`, creates its ACP peer, installs `NativeThreadEnvironment` and default tools, observes title/token/state changes, and persists changes. It flushes active threads on app quit. Project context refreshes asynchronously when project files, worktrees, trust, context servers, or skills change; identical contexts are retained to preserve prompt-cache prefixes. Global skills are scanned lazily on agent-panel use and watched under `~/.agents/skills`.

Model selections resolve through `LanguageModelRegistry`. A missing provider/model remains unresolved and can resolve after the provider appears. Profiles and subagent settings can select a model and configure thinking effort and speed. `SiblingThreadHost` is an application-provided extension point used by sibling-thread and agent/model-list tools.

### `Thread`

`Thread` owns the conversation, active turn, model/profile, token accounting, tool map, project/context-server state, title/summary tasks, compaction state, draft prompt, scroll position, and subagent links. Messages are user content (text, images, mentions), agent content (text, reasoning, tool uses and results), resume markers, or compaction markers.

An append-only `ThreadEventLog` records message appends/inserts/truncations, prompt-cache layouts, and large-result replacements. Its active sequence selects a history branch, allowing restoration at a point and forks without rewriting abandoned history. Subagents are `Thread`s with a parent and bounded depth; they inherit project context, templates, linked action logging, profile, and runtime settings unless explicitly overridden. Parent cancellation propagates to tracked child threads.

### `ThreadStore` and `ThreadsDatabase`

`ThreadStore` is the asynchronous, UI-oriented persistence index. It reloads metadata and supports loading, historical selection, forking, saving, and deleting threads; only top-level threads appear in its normal list. `ThreadsDatabase` owns the shared SQLite connection on a GPUI background executor. It uses in-memory storage for stateless/test configurations and otherwise stores data in Omega's data directory at `threads/threads.db`.

## Prompt and context assembly

`Thread::build_completion_request` constructs a language-model request from the selected model, active profile, current tool set, system prompt, and conversation. It supplies thread/prompt IDs, temperature, thinking/speed options, completion intent, tool schemas, and history; the final request message is cacheable.

`Templates` embeds `src/templates/*.hbs` at compile time and renders Handlebars in strict mode, so missing fields fail rather than silently omit instructions. `Thread::build_request_messages_until` reuses a saved system-prompt layout only when the exact ordered tool list matches. Otherwise it renders and records a new prompt plus tool order in the event log, preserving provider prompt-cache prefixes while accurately reflecting changed tools.

`SystemPromptTemplate` receives project context, visible tool names, detected external executors, selected model name, date, global personal `AGENTS.md`, and sandbox/platform state. The built-in `basic` profile renders `basic_system_prompt.hbs`; other profiles render `system_prompt.hbs`. `experimental_system_prompt.hbs` is embedded but not currently selected. Create-file, XML/fenced-edit, and diff-judge templates support focused workflows. Template sources are in `crates/agent/src/templates/`.

Project context selects applicable instruction/rule files from prompt-store candidates, including `.rules`, `.cursorrules`, `AGENTS.md`, `CLAUDE.md`, and related formats. Their trimmed content and worktree location enter the prompt; project instructions take precedence over global `AGENTS.md` when they conflict.

Skills come from built-ins, global skills, and trusted project-local `.agents/skills/*/SKILL.md` directories. Project skills override same-named global/built-in skills at the model/tool boundary. The prompt has a bounded catalog of eligible names, descriptions, and locations, rather than full bodies; the agent reads a relevant skill through its skill/read mechanism. Disabled-for-model-invocation skills are omitted, while loading, size, and catalog-budget problems become project skill issues. User mentions become structured sections for files, directories, symbols, selections, threads, URLs, diagnostics, diffs, conflicts, legacy rules, and attached skills.

## Tool subsystem and security

`AgentTool` is the typed built-in tool contract: stable name, typed JSON-schema input, description, ACP display kind/title, optional input streaming/provider compatibility/restricted-workspace policy, and asynchronous execution. `AnyAgentTool` erases concrete types so `Thread` can register, schema-render, execute, and replay heterogeneous tools.

Default tools cover file/directory operations; read/write/edit; grep/path search; terminal execution; fetch/web search; diagnostics, definitions, references, rename, and code actions; skill access; subagent delegation/transcripts; sibling-thread operations; resume; and artifact reads. Context-server (MCP) tools are merged at request time, with duplicate names server-qualified where possible.

Registration does not imply model visibility. `enabled_tools` filters on the profile allowlist, selected provider, feature flags, restricted-workspace eligibility, and terminal sandbox state. The basic profile maps the exposed surface to `read`, `write`, `edit`, `bash`, `delegate`, and `resume_thread`. Tools may accept partial JSON while a call streams, then receive its final typed input. Their raw output is retained for replay.

Tool permissions return `Allow`, `Confirm`, or `Deny` with this precedence: non-bypassable hard-coded rules, `always_deny`, `always_confirm`, `always_allow`, tool-specific default, then global default. Terminal commands are parsed into command chains and each subcommand is checked. Unsafe/unsupported permission-protected syntax, including substitutions/interpolations, is rejected; automatic allows are disabled when parsing cannot establish safety. Destructive Git operations receive separate data-loss protection.

Sandboxing is terminal-specific. It is available only with the sandbox feature flag, a local project, and platform integration (macOS Seatbelt, Linux Bubblewrap, Windows through WSL/Bubblewrap). Persistent `allow_unsandboxed` disables the model-facing sandbox surface. Otherwise the sandbox allows worktree writes, protects Git metadata, and blocks network access unless settings or grants allow hosts. Per-thread grants can add paths, hosts, or an unsandboxed escape; per-command requests use the tool-call permission stream. `ThreadSandbox` distinguishes no sandbox from an unrestricted sandbox.

`Thread::run_tool` centrally bounds text results unless a tool declares that it already does so. Oversized output is retained in a thread-scoped, versioned `ToolResultArtifactRegistry`; the model receives a bounded preview and an address for `read_tool_result_artifact`. Artifacts are rebuilt from persisted raw tool output after reload when exact reconstruction is possible, rather than persisted a second time.

## Execution and lifecycle loop

`send` appends a user message and `resume` appends a resume marker. Both call `run_turn`, which flushes any earlier partial assistant message, cancels a prior turn, creates the ACP event channel, and captures the currently enabled tools.

`run_turn_internal` loops until natural completion, cancellation/steering, or failure:

1. It performs automatic compaction when needed (or an explicit manual compaction).
2. It refreshes model/profile/tools between rounds and builds a completion request, so user changes apply at tool-result boundaries.
3. It streams text, reasoning, token usage, and tool input into the pending assistant message and ACP events.
4. It executes completed tool calls, potentially while streaming continues. The completion stream is dropped before waiting for tools to release its rate-limit permit for work such as delegated subagents.
5. It incorporates results, flushes the assistant message, creates an initial title if necessary, and either ends or sends another request with `ToolResults` intent.

Completion failures use cancellable retry/backoff. A safety refusal can switch to a configured refusal-fallback model. Token usage is tracked per user request and cumulatively without double-counting streaming snapshots. `NativeThreadEnvironment` adapts a thread to application work directories, ACP terminal creation/release, native/external ACP subagents, and session registration.

## Data persistence and schema

SQLite uses two tables:

| Table | Contents |
| --- | --- |
| `threads` | ID, optional parent ID, folder paths/order, display summary/title, timestamps, payload type, and serialized thread payload. |
| `thread_events` | Append-only event rows keyed by `(thread_id, sequence)`, with an optional parent sequence and serialized event kind. |

The `threads.data` payload is versioned `DbThread` JSON, normally Zstandard-compressed; legacy JSON remains readable. It is the durable record of conversation state: messages include reasoning, tool uses, results, and raw outputs, while thread state includes model/profile configuration, token usage, summaries, drafts/scroll position, subagent context, and an event-log snapshot. Messages, turns, and tool calls are not split into normalized SQL tables.

Saving upserts a snapshot and inserts event rows with `INSERT OR IGNORE`, preserving existing append-only events. Loading deserializes the snapshot, overlays stored event rows, and prepares the thread to resume. Historical selection changes the active sequence; a fork creates a new session from an earlier event or message point. Deleting a parent recursively deletes durable subagent rows and associated sandboxed-terminal temporary directories.

## Integration with `agent_ui` and Omega

`agent` provides durable behavior; `agent_ui` provides the interface. The UI submits ACP content blocks and controls send/resume/compact/cancel, renders `ThreadEvent` streams, and uses `ThreadStore` metadata for thread lists and history/fork flows. `NativeAgent` mirrors titles, token usage, draft text, scroll position, tool replay, and lifecycle changes to `AcpThread`.

The wider application supplies terminal creation/release and, when desired, a `SiblingThreadHost` for independent sibling work. This keeps presentation and approval policy outside the execution engine while every native thread shares the same prompt construction, persistence, permissions, sandboxing, and tool semantics.
