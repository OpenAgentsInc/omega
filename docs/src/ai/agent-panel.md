---
title: AI Coding Agent - Omega Agent Panel
description: Use Zed's AI coding agent to generate, refactor, and debug code with tool calling, checkpoints, and multi-model support.
---

# Agent Panel

The Agent Panel is where you interact with AI agents that can read, write, and run code in your project.
It's the core of Zed's AI code editing experience — use it for code generation, refactoring, debugging, documentation, and general questions.

Open it with {#action agent::NewThread} from [the Command Palette](../command-palette.md) or click the ✨ icon in the status bar.

## Getting Started {#getting-started}

If you're using the Agent Panel for the first time, configure either a model for the [Omega Agent](./omega-agent.md) or an [External Agent](./external-agents.md).

- Use [LLM Providers](./llm-providers.md) for Zed-hosted models, API access, subscriptions, gateways, and local models.
- Use [External Agents](./external-agents.md) for ACP-integrated agents.
- Use [AI Quick Start](./quick-start.md) if you are not sure which path to choose.

## Overview {#overview}

With an LLM provider or External Agent configured, type in the message editor and press `enter` to submit.
Expand the editor with {#kb agent::ExpandMessageEditor} if you need more room.

Responses stream in with indicators showing [which tools](./tools.md) the model is using.
The sections below cover what you can do from here.

The persistent sidebar also contains [Tester channels](./tester-channels.md).
On a clean profile, its public alpha feedback destination starts expanded so
you can find support and feedback without leaving the Agent Panel.

> Note that some Agent Panel features may not be available for every External Agent. Restoring threads from history, checkpoints, token usage display, and similar features depend on the agent integration.
> Their availability varies depending on the agent.

### Desktop Work Surfaces {#desktop-work-surfaces}

Omega desktop keeps the active thread transcript in the center and places a
vertical work-surface rail beside it. The rail contains Files, Search, Review,
Git, Terminal, and Plan.

Select an inactive item to open its dock. Select the active item again to
collapse the dock and return focus to the thread. Reopening a collapsed surface
retains its host instead of recreating the transcript or surface state.
Drag the dock's right edge to resize it, or double-click the edge to restore
the default width.

Use these default actions:

| Task                   | Action                                         |
| ---------------------- | ---------------------------------------------- |
| Focus the rail         | {#kb omega_workbench::FocusActivityRail}       |
| Open Files             | {#kb omega_workbench::SelectFiles}             |
| Open Search            | {#kb omega_workbench::SelectSearch}            |
| Open Review            | {#kb omega_workbench::SelectReview}            |
| Open Git               | {#kb omega_workbench::SelectGit}               |
| Open Terminal          | {#kb omega_workbench::SelectTerminal}          |
| Open Plan              | {#kb omega_workbench::SelectPlan}              |
| Collapse the open dock | {#kb omega_workbench::CollapseWorkSurfaceDock} |

While the rail is focused, use Up and Down to move between items, Home and End
to move to the first or last item, Enter or Space to activate the item, and
Escape to return focus to the thread.

Files, Search, Review, Git, and Terminal require an active workspace and
worktree. Plan requires an active agent thread. Unavailable items remain visible
and explain what is missing instead of opening another surface.

Files renders the same native Project Panel entity used by the workspace. It is
scoped to the active thread's selected worktree, so a multi-root workspace does
not mix files from another thread target into the dock. Native tree navigation,
file opening, reveal, rename, drag-and-drop, Git decorations, diagnostics, and
context menus continue to use the Project Panel implementation.

Before you open Files for the first time, the Project Panel remains unchanged
in its existing workspace dock. A successful Files activation moves that same
entity into the work-surface dock while retaining a nonvisual Workspace
registration for native commands such as reveal, rename, and File History. It
is never rendered in both docks. If activation fails, the workspace panel keeps
its previous owner and scope. Collapsing and reopening Files retains the
entity, its selection, and its expansion state. Scope changes preserve
selection, rename, marked-entry, clipboard, and undo state only when it belongs
to the compatible worktree. Expansion remains keyed by worktree. Transient drag
and context-menu state is cleared during the switch.

A scoped worktree root remains the tree's authority and is expanded when it is
rendered. Files shows a distinct empty state only when that worktree has no
visible entries below its root after the Project Panel's normal filters. A
missing root is a different state and never falls back to files from another
workspace root.

When the selected identity reports an error, Files replaces the tree with an
inline alert and keeps focus on that visible alert host. Loading, error,
offline, inconsistent, unbound, and missing-root states also disable the
rehomed panel's native actions, so a command cannot mutate a hidden or stale
worktree. A transient outage preserves compatible tree state for recovery;
changing to a different worktree filters the old selection and undo history.
Recovering the same identity restores the tree and its state. If the selected
root disappears or the repository connection goes offline, Omega closes Files
and moves actual keyboard focus back to the active thread.

Opening a file from Files uses the native Project Panel and Workspace path.
Preview open keeps focus in the tree; permanent open and split actions reveal
and focus the ordinary editor beside the still-mounted agent transcript.
Closing the final center item restores the agent-only surface. Project Panel
toggle and close-dock shortcuts operate on the embedded Files dock rather than
closing the outer Agent Panel.

The thread header shows the active project/repository, worktree, and Git branch
as separate controls. It also shows changed files, conflicts, and upstream
ahead/behind state when present. These values are the target used by Files,
Search, Review, Git, and Terminal actions, not a decorative copy of the
workspace title.

Use the repository or worktree control to choose a different valid target for
the active thread. The native picker disambiguates worktrees that share the
same branch name. Use the branch control to open the existing Git branch
picker. Changing a target updates the thread and its work surfaces together;
an in-flight result for the previous target is ignored.

Some External Agents fix their working directory when a session starts and do
not provide a safe live-retarget operation. For those sessions, the repository
and worktree controls remain visible but disabled, and their tooltip explains
that you must start a new thread in the other target. Omega does not present a
client-side path change as if the remote agent had accepted a new cwd.
Selection is also disabled while the session or repository identity is loading,
stale, offline, or reconnecting, and while any turn, permission request, or
elicitation is active. The rendered controls and their keyboard actions use the
same availability decision. Branch checkout uses that busy-session gate too,
and invalidates Git and Review results started before the checkout. While the
checkout is pending, Omega makes the composer read-only and disables every
repository/worktree/branch mutation so a prompt cannot observe a worktree
between branches.

Detached HEAD, an unborn repository, and a folder without Git are called out
explicitly. A no-Git folder can still use Files, Search, Terminal, and Plan.
Loading, stale, offline, reconnecting, missing-worktree, operation-error, and
inconsistent-session states appear beside the identity. An inconsistent state
means one agent session may have accepted a target change that another rejected
and Omega could not roll every session back. Repository-bound surfaces and
branch changes stop until you reselect a repository/worktree target or
reconnect the thread; reselecting forces every session onto that target before
restoring the controls.

Full names and paths remain available to assistive technology and in tooltips
when the visible controls compact in a narrow window. A missing worktree
retains its last-known label and keeps the repository picker available so you
can explicitly recover to another valid target; a failed recovery reports its
error without reviving the removed binding. Omega never silently chooses a
replacement. Plan remains available while offline because it is thread-local;
repository-bound surfaces still require the live projection.

On a narrow window, Omega collapses the threads sidebar first. If the dock,
rails, and transcript still do not fit, it collapses the work-surface dock and
returns focus to the transcript. Widening the window does not reopen the dock;
select the rail item again.

> **Note:** Files now mounts the native Project Panel. Search, Review, Git,
> Terminal, and Plan still use retained placeholder hosts while their native
> adapters are completed. Until those adapters land, use the existing native
> panels for the corresponding task. See
> [Omega desktop workbench shell](../development/omega-desktop-workbench-shell.md)
> for implementation scope.

### Creating New Threads {#new-thread}

With a project selected, {#action agent::NewThread} and the **Thread > New
Thread** menu item open a normal thread with the composer focused — there is no
interstitial mode screen. Without a selected project, Omega asks you to select
one and does not create a draft thread. The default executor is **Omega
Agent**, and the executor selection is a dropdown in the composer bar, beside
the Flash/Pro tier control. The
dropdown offers the executor rows in one fixed order: Omega Agent, the named
direct agents (Codex, Claude Code, Grok Build), every other installed ACP
agent, Sarah (voice), and **Add More Agents…**, which opens the ACP registry.
The toolbar `+` retains its compact creation menu; choosing **Omega Agent**
there reaches the same focused composer.
{#action agent::ToggleComposerExecutorMenu} opens the dropdown from the
keyboard, and the menu itself is arrow-key driven.

Every dropdown row carries one of four readiness states: **Ready**, **Setup
required**, **Temporarily unavailable**, or **Not supported in this build**. A
row that cannot run here is disabled with its reason — never hidden and never
fake-enabled. A Direct Agent is Ready only after that exact agent creates its
session. Omega Agent is Ready when its router has a live executor inventory;
its first submitted request then selects one exact executor and creates that
executor's session.

Selection is free until the first send: picking a different executor over a
blank conversation replaces the blank preparation with one bound to the exact
ACP id, and neither path probes with one session and creates another. The
first send binds the conversation to its executor. After that, picking a
different executor in the dropdown starts a new conversation — an existing
transcript's executor never changes underneath its entries, and the
transcript title, composer label, and disclosure keep naming the bound
executor.

If Omega needs authentication or setup fails, its conversation shows **Setup
required** with the reason and its existing authentication, error, and retry
controls. Direct setup and authentication errors reveal that same
external-agent view, with its native controls.

Choosing **Sarah**, **Thread > Sarah voice…**, or **Sarah voice…** in the
toolbar `+` menu opens the same admission surface inside Agent Panel. The
composer microphone stays in the conversation: when terms are ready it starts
voice, otherwise it loads them and shows a short dismissible refusal beside the
control. Detailed admission remains available from Sarah's chooser and menu
entries. Opening admission does not request
microphone access or a one-use voice ticket. A Ready surface first shows the
service-returned admission cohort, effective
rate, credit hold, remaining credit or explicit non-metered owner entitlement,
maximum duration, transcript policy,
bounded command list, and the confirmation rule for every command. An
unavailable surface shows the cohort and refusal reason when supplied; account
credit does not override `cohort_inactive`. Only an admitted **Start voice** or
the admitted composer gesture starts the managed session; neither can bypass
reviewed terms.

Sarah is a bounded voice editor and delegation assistant. The surface lists the
server's exact exposed subset of context read, reveal range, replace selection,
save document, and start agent thread, along with the confirmation rule for
each exposed capability. Direct shell, direct Git, payments, credential access,
and device control remain explicitly unavailable. Her identity grants none of
those authorities. Loading and unavailable admission states keep the
microphone off. For `livekit_room_v1`, Omega also keeps it off until OpenAgents
returns the exact generation-bound room, participant grant, Sarah participant,
dispatch, presence lease, and microphone-only permissions. LiveKit then carries
only microphone and Sarah audio; the authenticated OpenAgents WebSocket remains
authoritative for lifecycle, transcripts, commands, interruption, usage, and
settlement. After a session, the same surface shows the final charge,
remaining credit when supplied, settlement receipt reference, and transcript
recovery result rather than retaining an estimate. During an active session it
also shows the newest 100 attributed transcript rows, exact pending command
confirmation, and any created Omega Agent-thread receipt.
**Allow once** and **Decline** resolve the runtime owner's current request from
this visible surface, and the artifacts remain available after settlement.

Prepared conversations are saved before their first physical session, together
with the exact owner and working folders. If startup restores a conversation,
terminal, Full Auto surface, pending terminal, or typed draft while the panel is
opening, Omega keeps that restored state instead of covering it with a new
blank composer.

For a new Omega Agent conversation, **Automatic** derives normalized task
requirements from the first request and deterministically chooses between the
native loop and ready ordinary ACP agents. You can instead choose one exact
executor before sending; that one-conversation override is recorded and takes
priority. Engine lanes remain under the separate Full Auto gesture. The route
line appears before dispatch and includes the exact executor, reason, override,
and fallback. The full readiness inputs remain in the durable receipt. Once
selected, the executor is immutable for the conversation. If it becomes
unavailable, the send fails visibly by name
and is never replayed through another agent.

Use {#action agent::NewTerminalThread} for a terminal thread. The generic new
thread action no longer repeats the last terminal choice.

You can also start a new thread from the [Threads Sidebar](./parallel-agents.md#threads-sidebar), scoped to a specific project — see [Running Multiple Threads](./parallel-agents.md#running-multiple-threads).

### Managing Multiple Threads {#multiple-threads}

You can run multiple agent threads at once, each working independently with its own agent, context window, conversation history, durable message queue, and lifecycle. Open the Threads Sidebar with {#kb multi_workspace::ToggleWorkspaceSidebar} to see all your threads grouped by project. Each row shows its agent and whether it is Running, Waiting for you, Failed, Completed, or Cancelled. Click any thread to switch to it, or use the thread switcher ({#kb agents_sidebar::ToggleThreadSwitcher}) to cycle between recent threads without opening the sidebar. Cancelling the active thread does not cancel any other thread.

Threads you're no longer working on can be archived by hovering over them in the sidebar and clicking the archive icon, or selecting them and pressing {#kb agent::ArchiveSelectedThread}. The Thread History holds all your threads across all projects, sorted chronologically, and you can restore them at any time.

If two threads might edit the same files, Omega isolates them for you: a new thread whose folder is already held by a live agent gets its own linked Git worktree, with no dialog and nothing to answer. Use the worktree picker in the thread header to pick which worktree the agent runs in, or choose **New worktree** to move it into a fresh one on purpose. Set `agent.thread_worktree` to `"shared"` to run every thread in the checkout it was opened against instead. See [Worktree Isolation](./parallel-agents.md#worktree-isolation) for details.

For more details on the Threads Sidebar and managing multiple projects, see [Parallel Agents](./parallel-agents.md).

### Editing Messages {#editing-messages}

Any message that you send to the model is editable.
You can click on the card that contains your message and re-submit it with an adjusted prompt and/or new pieces of context.

### Queueing Messages

Messages sent while the agent is in the generating state get, by default, queued.

By default, queued messages get sent once the agent finishes generating. If you want a queued message to reach the Omega Agent sooner—interrupting it at its next step (usually between a tool call and a response) rather than waiting for it to finish—toggle "Steer" on that message. Steering is only available for the Omega Agent, since Zed can't detect turn boundaries for external agents.

You can edit or remove (an individual or all) queued messages. Queued text and per-thread processing state are written durably before Omega acknowledges the queue operation, and are restored independently for each thread after relaunch. Restored open queues start paused until you explicitly resume them; a fresh local Idle state is not treated as proof that the provider's pre-relaunch turn finished. If storage cannot be updated, Omega keeps the unsaved state visible and does not dispatch stale text.
You can also still interrupt the agent immediately if you want by either clicking on the stop button or by clicking the "Send Now" (double-enter) on a queued message.

### Checkpoints {#checkpoints}

Every time the model performs an edit, you should see a "Restore Checkpoint" button at the top of your message, allowing you to return your code base to the state it was in prior to that message.

The checkpoint button appears even if you interrupt the thread midway through an edit, as this is likely a moment when you've identified that the agent is not heading in the right direction and you want to revert back.

### Context Menu {#context-menu}

Right-click on any agent response in the thread view to access a context menu with the following actions:

- **Copy Selection**: Copies the currently selected text as Markdown (available when text is selected).
- **Copy This Agent Response**: Copies the full text of the agent response you right-clicked on.
- **Scroll to Top / Scroll to Bottom**: Scrolls to the beginning or end of the thread, depending on your current position.
- **Open Thread as Markdown**: Opens the entire thread as a Markdown file in a new tab.

### Navigating the Thread {#navigating-the-thread}

In long conversations, use the scroll arrow buttons at the bottom of the panel to jump to your most recent prompt or to the very beginning of the thread. You can also scroll the thread using arrow keys, Page Up/Down, Home/End, and Shift+Page Up/Down to jump between messages, when the thread pane is focused.

When focus is in the message editor, you can also use {#kb agent::ScrollOutputPageUp}, {#kb agent::ScrollOutputPageDown}, {#kb agent::ScrollOutputToTop}, {#kb agent::ScrollOutputToBottom}, {#kb agent::ScrollOutputLineUp}, and {#kb agent::ScrollOutputLineDown} to navigate the thread, or {#kb agent::ScrollOutputToPreviousMessage} and {#kb agent::ScrollOutputToNextMessage} to jump between your prompts.

### Thread titles {#thread-titles}

Thread titles are auto-generated based on the content of the conversation.
But you can also edit them manually by clicking the title and typing, or regenerate them by clicking the "Regenerate Thread Title" button in the ellipsis menu in the top right of the panel.

### Following the Agent {#following-the-agent}

Follow the agent as it reads and edits files by clicking the crosshair icon at the bottom left of the panel.
Your editor will jump to each file the agent touches.

You can also hold `cmd`/`ctrl` when submitting a message to automatically follow.

### Get Notified {#get-notified}

If you send a prompt to the Agent and then put Zed in the background, you can choose to be notified when its generation wraps up via:

- a visual desktop notification from your operating system
- a sound notification

These notifications can be used together or individually, and you can use the `agent.notify_when_agent_waiting` and `agent.play_sound_when_agent_done` settings keys to customize that, including turning both off entirely.

### Reviewing Changes {#reviewing-changes}

Once the agent has made changes to your project, the panel will surface which files, how many of them, and how many lines have been edited.

To see which files specifically have been edited, expand the accordion bar that shows up right above the message editor or click the `Review Changes` button ({#kb agent::OpenAgentDiff}), which opens a special multi-buffer tab with all changes.

You can accept or reject each individual change hunk, or the whole set of changes made by the agent.

Edit diffs can also appear inline in individual files with the same
keep/reject hunk controls as the multi-buffer review pane. This temporarily overrides the buffer's git diff while review is active. Enable it by setting `agent.single_file_review` to `true` in your settings.

## Terminal Threads {#terminal-threads}

The Agent Panel can host Terminal Threads alongside your agent threads. For opening, closing, notifications, terminal titles, and CLI/TUI-specific setup, see [Terminal Threads](./terminal-threads.md).

## Adding Context {#adding-context}

The agent can search your codebase to find relevant context, but providing it explicitly improves response quality and reduces latency.

Add context by typing `@` in the message editor.
You can mention files, directories, symbols, previous threads, skills, diagnostics, branch diffs, and URLs to fetch.

When you paste multi-line code selections copied from a buffer, Zed automatically formats them as @-mentions with the file context.
To paste content without this automatic formatting, use {#kb agent::PasteRaw} to paste raw text directly.

### Selection as Context

Additionally, you can also select text in a buffer or terminal and add it as context by using the {#kb agent::AddSelectionToThread} keybinding, running the {#action agent::AddSelectionToThread} action, or choosing the "Selection" item in the `+` menu in the message editor.

### Images as Context

It's also possible to attach images in your prompt for providers that support vision models.
OpenAI GPT-4o and later, Anthropic Claude 3 and later, Google Gemini 1.5 and 2.0, and Bedrock vision models (Claude 3+, Amazon Nova Pro and Lite, Meta Llama 3.2 Vision, Mistral Pixtral) all support image inputs.

To add an image, you can either search in your project's folder by @-mentioning it, or drag it from your file system directly into the Agent Panel message editor.
Copying an image and pasting it is also supported.

## Token Usage and Compaction {#token-usage}

Zed surfaces how many tokens you are consuming for your currently active thread near the profile selector in the panel's message editor.

Zed automatically compacts long Omega Agent threads as they approach the configured token threshold. Compaction summarizes earlier messages and replaces them in the model context with that summary, leaving more room for the next turn. The thread shows a **Context Compacted** entry that you can expand to inspect the summary. You can compact manually by typing `/compact` in the message editor.

If the selected model's context window is too small for automatic compaction (less than 80000 tokens), a banner appears above the message editor as you approach the token limit. Use **Start New Thread** from that banner, or choose **New From Summary** from the New Thread menu (the `+` button on the top right), to continue in a new thread seeded with a summary. You can also @-mention a past thread in a new one.

Configure automatic compaction with `agent.auto_compact`. See [Agent Settings](./agent-settings.md#automatic-compaction) for options.

## Changing Models {#changing-models}

After you've configured your LLM providers—either via [API access](./use-api-access.md) or through [Zed-hosted models](../account/zed-hosted-models.md)—you can switch between their models by clicking on the model selector on the message editor or by using the {#kb agent::ToggleModelSelector} keybinding.

> The same model can be offered via multiple providers - for example, Claude Sonnet 4.5 is available via Zed Pro, OpenRouter, Anthropic directly, and more.
> Make sure you've selected the correct model **_provider_** for the model you'd like to use, delineated by the logo to the left of the model in the model selector.

### Favoriting Models

You can mark specific models as favorites either through the model selector, by clicking on the star icon button that appears as you hover the model, or through your settings via the `agent.favorite_models` settings key.

Cycle through your favorites with {#kb agent::CycleFavoriteModels} without opening the model selector.

## Using Tools and Profiles {#using-tools}

The Agent Panel supports tool calling, which enables agentic editing. Zed includes [built-in tools](./tools.md) for searching your codebase, editing files, running terminal commands, and more.

Use [Agent Profiles](./agent-profiles.md) to choose which built-in tools and MCP tools are available in an Omega Agent thread. Use [Tool Permissions](./tool-permissions.md) to control whether permission-gated tool calls are allowed, denied, or confirmed.

You can add external tools with [MCP Servers](./mcp.md).

### Model Support {#model-support}

Tool calling needs to be individually supported by each model and model provider.
Therefore, despite the presence of built-in tools, some models may not have the ability to pick them up.
You should see a "No tools" label if you select a model that falls into this case.

All [Zed-hosted models](../account/zed-hosted-models.md) support tool calling out-of-the-box.

### MCP Servers {#mcp-servers}

Similarly to the built-in tools, some models may not support all tools included in a given MCP Server.
Zed's UI will inform you about this via a warning icon that appears close to the model selector.

## Errors and Debugging {#errors-and-debugging}

If you hit an error or unusual LLM behavior, open the thread as Markdown with {#action agent::OpenActiveThreadAsMarkdown} and attach it to your GitHub issue.

You can also open threads as Markdown by clicking on the file icon button, to the right of the thumbs down button, when focused on the panel's editor.

## Feedback {#feedback}

You can rate agent responses to help improve Zed's system prompt and tools.

> **Warning:** Rating an AI response sends the conversation thread to Zed. The
> conversation thread includes your messages, AI responses, and thread metadata.
> See [Feedback and Training Data](./ai-improvement.md) and
> [AI Privacy](./privacy-and-security.md) for more information.
> **_If you don't want data persisted on Zed's servers, don't rate_**.
> We will not collect data for improving the agent experience without you
> explicitly rating responses.

To help improve Zed's system prompt and tools, rate responses with the thumbs up/down controls at the end of each response.
In case of a thumbs down, a new text area will show up where you can add more specifics about what happened.

You can provide feedback on the thread at any point after the agent responds, and multiple times within the same thread.
