# The Native Omega Agent in Omega

Omega Agent is the admitted first-party orchestrator identity. Its first
executor is the inherited native Rust runtime described here; the rename does
not make the native loop the router, a service identity, or a new run
authority. The sealed default surface presents it as **Omega Agent** in the
three-mode new-conversation front door.

This page explains how to expose the current implementation, what it does, and
where it fits in Omega's architecture.

## Open It in Omega Today {#open-it}

Run {#action agent::NewThread} and choose **Omega Agent** after its row reports
Ready. The row becomes Ready only after the prepared connection has created a
session, and selection reuses that exact conversation entity. No editor-panel
switch or View-menu path is required on the default surface.

The transitional editor compatibility surface was removed with the mode
split (omega#161): Omega has one launch surface, and the upstream
settings-file and panel controls are not advertised by its application menu.

If a project has `"disable_ai": true` in its project settings, that setting
also disables the Agent Panel and removes its actions.

## Configure the API Environment {#configure-api-environment}

Omega Agent exposes one logical model identifier, `omega-agent`. The client
does not select an inference model or reasoning level. The OpenAgents API owns
provider credentials, model selection, and routing.

Open **Settings** and enable **Use Development API** to use
`ws://127.0.0.1:8080/v1/responses` for response streams. Other development API
requests use `http://127.0.0.1:8080/v1`. Disable it to use
`https://api.openagents.com/v1`. Omega signs both endpoints with the active
Nostr identity.

An External Agent such as Codex ACP is a different mode in the same
new-conversation front door. It does not supply a model to Omega Agent. The
external agent owns its own runtime, authentication, model selection, tools,
and sessions.

## What Omega Agent Is {#what-it-is}

Omega Agent is the in-process, project-aware coding-agent engine inherited from
Zed. It combines:

- the `omega-agent` Responses API adapter
- a native conversation `Thread`
- project and worktree context
- instructions and skills
- built-in editor, language, filesystem, and terminal tools
- MCP tools
- tool profiles and permission policy
- local thread persistence
- Agent Panel, Threads Sidebar, diff review, and checkpoint projections

The user-facing identity is fixed by `OMEGA_AGENT_ID` as `Omega Agent`. The
symbol was renamed along with the value it holds, so an upstream rebase that
restores the old identity has no familiar name to restore it under. The
implementation remains `NativeAgent`; `NativeAgentServer` constructs it inside
the Omega process and wraps it in `NativeAgentConnection`.

The connection implements the same `AgentConnection` interface used by the
Agent Panel for ACP-backed agents. That shared interface does not make Omega Agent
an external ACP process. No agent executable is launched and no ACP transport
is crossed for a native Omega Agent thread.

## How a Turn Works {#turn-flow}

A native turn follows this path:

1. The Agent Panel opens or restores a native session.
2. `NativeAgent` binds the session to an Omega `Project`.
3. It builds `ProjectContext` from visible worktrees, project rules,
   instructions, and available skills.
4. The selected profile determines which built-in and MCP tools the model can
   see.
5. `Thread` builds a Responses API request from the system prompt, project
   context, conversation history, and current message.
6. The OpenAgents API streams text, reasoning, and tool calls. Server-side
   routing stays outside the client request.
7. Tool calls pass through Omega's permission policy and execute against the
   project, editor, language servers, filesystem, terminal, or MCP server.
8. Tool results are appended to the thread and sent back to the model.
9. The loop continues until the model stops, the user cancels, or an error
   terminates the turn.
10. Thread events are projected into the Agent Panel and saved by the local
    thread store.

Long conversations can be compacted. A compacted thread replaces older model
context with a summary while preserving the visible local history.

## Native Capabilities {#capabilities}

The native runtime has tighter editor integration than an arbitrary external
agent. Depending on the selected profile, model support, and current project,
it can:

- read, search, create, edit, move, and delete project files
- inspect symbols, definitions, references, and diagnostics
- request language-server code actions and renames
- run terminal commands and project tasks
- fetch URLs and use configured web tools
- read project instructions such as `AGENTS.md`
- discover and invoke installed skills
- call tools from configured MCP servers
- spawn child native agents
- create sibling panel threads with an available agent and model
- stream file edits and terminal output into the Agent Panel
- present changes in Omega's native review UI
- restore checkpoints made before agent edits

Profiles control tool availability. Permissions control whether an available
tool call is allowed, denied, or confirmed. These are separate controls.

## Omega's Permission Delta {#permissions}

Omega intentionally differs from upstream Zed. Its global tool-permission
default is `allow`, not `confirm`, because unattended Full Auto work cannot
make progress while waiting at a prompt.

Owner-directed Codex and Claude ACP sessions also run in their respective
full-access modes. Omega reapplies that mode when it creates or restores a
session. Changing another agent-server preference, such as the default model,
does not remove the full-access mode. As a result, these coding agents can read
and write outside the selected project, use the network, and run commands
without an approval prompt.

This does not mean every possible command is safe:

- hardcoded terminal rules still reject a small set of catastrophic commands
- `always_deny` and `always_confirm` patterns can narrow the default
- a tool omitted by the active profile is unavailable
- an external agent can apply additional permission rules of its own

Omega must describe the effective policy honestly. It must not copy upstream
documentation that promises a confirmation prompt for every consequential
tool call.

Omega also must not claim that a native thread is sandboxed merely because the
inherited agent has sandbox-related types or settings. The current Omega
product direction removed Bubblewrap and other gates that blocked unattended
work. Any future containment must report the effective runtime and filesystem
boundary for the specific run.

## What It Is Not {#what-it-is-not}

Omega Agent is not:

- a language model or provider
- Codex, Claude, Hermes, or another external agent
- the ACP protocol, even though its UI connection uses the shared thread
  abstraction
- Full Auto
- `omega-effectd`
- Agent Computer
- Sarah
- an OpenAgents identity
- a durable workroom, run, policy, receipt, or claim authority

Those distinctions matter because the Agent Panel can display several of these
systems without owning their durable state.

## Runtime Boundaries in Omega {#runtime-boundaries}

| Path               | Runtime owner                                             | Configuration owner                                                           | Durable authority                                  | Omega UI role                               |
| ------------------ | --------------------------------------------------------- | ----------------------------------------------------------------------------- | -------------------------------------------------- | ------------------------------------------- |
| Omega Agent        | Omega's local tool loop and the OpenAgents Responses API  | OpenAgents model routing; Omega profile, skill, instruction, and MCP settings | Local native thread history only                   | Execute and review coding turns             |
| External ACP agent | The external agent process or endpoint                    | The external agent                                                            | The external agent, where supported                | Project an ACP session                      |
| Terminal Thread    | The selected CLI or TUI                                   | The CLI or TUI                                                                | The CLI or its service                             | Host a terminal-backed session              |
| Full Auto          | Released OpenAgents engine under `omega-effectd`          | OpenAgents Full Auto contracts and provider bindings                          | OpenAgents work, run, outcome, and receipt records | Launch, observe, review, and intervene      |
| Agent Computer     | OpenAgents cloud capacity reached through `omega-effectd` | OpenAgents placement and harness environment                                  | OpenAgents cloud-session and receipt records       | Start and observe a bounded cloud turn      |
| Sarah workroom     | Sarah services and the admitted Nostr record              | Sarah and workroom contracts                                                  | Signed conversation, decision, and receipt records | Project the owner-private or community room |

Zed remains authoritative for editor, project, buffer, language, terminal, and
worktree state. OpenAgents remains authoritative for work, agent, policy,
receipt, and run state. GitHub remains repository and claim authority until a
separate admitted cutover.

## How Omega Should Use It {#omega-use}

Omega should retain the native runtime as its local agent-to-code loop. This is
the implementation substrate for roadmap packet `OMEGA-OA-06`:

- connect a work item to a project and worktree
- give an agent native buffer, selection, diagnostic, Git, and language context
- execute local tool calls
- show the exact edits and command output
- let the operator review, keep, reject, or restore changes

Omega should also retain the shared `AgentConnection` and `AcpThread`
projection layer. It lets one Agent Panel host native agents, external agents,
and terminal-backed work without pretending that they share a runtime or
authority model.

The native thread store should remain local conversation and UI continuity. It
must not become an Omega-only cloud thread store or the canonical record for a
workroom, Full Auto run, Agent Computer session, decision, or receipt. A native
thread attached to durable OpenAgents work should store references to those
records and project their terminal outcomes.

Skills, instructions, and recalled context should guide the native coding
loop. They are input to the model, not authorization. A signed event also does
not grant authority by itself. Commands that affect durable OpenAgents state
must still pass the typed OpenAgents command, grant, and receipt boundaries.

## How Omega Should Not Use It {#omega-non-use}

Omega should not:

- wrap every external agent inside `NativeAgent`
- copy an external agent's home, credentials, memory, or provider state into
  Omega
- make a native `Thread` the source of truth for Full Auto
- call Firecracker, placement, or cloud coding-session APIs directly from the
  native agent or GPUI
- treat Nostr relay acceptance as permission to run a local tool
- infer capabilities from an agent or provider name
- claim hosted Zed AI support
- present the inherited `telemetry_id` as an OpenAgents service identity
- expose the server's provider model as a client selection

Full Auto should continue through `omega-effectd` and its typed protocol.
Agent Computer should remain one execution environment behind that service.
Sarah and community workrooms should attach bounded work to workers without
granting the worker Sarah's authority. The native agent can be one such worker;
it does not absorb the system that assigned the work.

## Product Work Still Needed {#product-gaps}

The implementation exists, but its Omega product contract is unfinished.

1. Add endpoint health and environment status to the Settings surface.
2. Complete the admitted router and typed-disclosure packets. The identity
   rename does not by itself implement routing, receipts, or a public claim.
3. Keep remaining Zed-specific copy out of the native-agent journey.
4. Show the exact runtime, API environment, profile, and effective permission
   policy in each thread. Treat provider routing as server metadata.
5. Keep hosted Zed plans, trials, account state, feedback upload, and service
   assumptions out of Omega.
6. Define typed links from a local agent thread to OpenAgents work, runs,
   decisions, and receipts without duplicating their storage.
7. Test API failure, restart, cancellation, dirty worktree, wrong project,
   revoked grant, and external-agent handoff paths.
8. Bind any release claim to an installed Omega candidate. A source test or
   fixture pass does not prove the packaged journey.

**Omega Agent** in the reachable UI selects the native Omega tool loop backed
by the OpenAgents Responses API. The local thread remains the authority for
local transcript and editor state.

## Implementation Map {#implementation-map}

| Responsibility                                             | Current source                                      |
| ---------------------------------------------------------- | --------------------------------------------------- |
| Native runtime, project context, skills, and UI connection | `crates/agent/src/agent.rs`                         |
| Model/tool loop and thread events                          | `crates/agent/src/thread.rs`                        |
| Local native thread storage                                | `crates/agent/src/thread_store.rs`                  |
| In-process server construction                             | `crates/agent/src/native_agent_server.rs`           |
| Tool implementations                                       | `crates/agent/src/tools/`                           |
| Tool permission evaluation                                 | `crates/agent/src/tool_permissions.rs`              |
| Agent defaults and Omega permission delta                  | `assets/settings/default.json`                      |
| OpenAgents Responses API adapter                           | `crates/language_models/src/provider/openagents.rs` |
| Agent Panel and new-thread selection                       | `crates/agent_ui/src/agent_panel.rs`                |
| Native/external agent registration                         | `crates/agent_ui/src/agent_ui.rs`                   |
| Durable Omega product boundaries                           | `PRODUCT.md` and `OMEGA_DELTAS.md`                  |

The product guidance on this page is derived from the complete
`openagents/docs/omega/` planning corpus, especially its permanent design laws,
native agent-to-code packet, Full Auto contract, Agent Computer contract,
identity-first onboarding plan, Sarah workroom contracts, mobile adaptation
audit, brand audit, and Bubblewrap removal audit.
