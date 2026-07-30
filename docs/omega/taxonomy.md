# Omega taxonomy and glossary

What each word means here, and which layer it belongs to.

This exists because the words collide. In one day, work was lost to two
different projects called Exo, a directory and an audience both called a
workspace, and a wire token shown to a person as though it were a name. Each
confusion cost an hour or more. A term is in this document when getting it
wrong has already cost something, or clearly will.

Read the layer diagram first. Most collisions are two layers using one word.

## The layers

```text
  person
    │
    ▼
  thread ─────────────── what a person reads and types into
    │
    ▼
  Omega Agent ────────── the router: routing, disclosure, receipts.
    │                    Owns no execution.
    ▼
  executor class ─────── native loop │ external ACP │ engine lane
    │                    Exactly three. A closed enum.
    ▼
  executor ───────────── the thing that runs the turn:
                         Omega's own loop, or Codex, Claude, Exo,
                         or an omega-effectd lane
```

Beside that stack, and often confused with it:

```text
  project ────── a directory on disk the thread can read
  workspace ──── a window (Zed's type)
  skill ──────── instructions an agent loads on demand
  delta ──────── a recorded divergence from upstream Zed
```

## Terms

### Exo — always say which one

Two unrelated projects. This has already cost a day.

**Exo (ours)** — `OpenAgentsInc/exo`, at `~/work/exo`. A Rust agent harness
with an ACP surface. It runs turns: prompts, tools, an event log. This is what
Omega attaches to as an executor, and what the disclosure line means by
`exo/basic`. Built from scratch — 85 commits, first commit "Initial commit" —
and named after, but not forked from, the project below.

**exo labs** — `exo-explore/exo`. A Python and MLX system that turns several
Macs into one model endpoint over an OpenAI-compatible HTTP API. It supplies
*capacity*, not an agent. Omega does not integrate it today.

They share only a name. When writing, say "our Exo" or "exo labs", never bare
"exo" in a sentence where both could apply. Teardowns exist for each and are
named accordingly:

- `docs/teardowns/2026-07-25-exoharness-exo-teardown.md` — ours
- `docs/teardowns/2026-07-25-exo-teardown.md` — exo labs

**exoharness** — in the upstream project's own vocabulary, the *trusted
substrate*: the durable event log, artifacts, sandboxes, secrets. It stops
before the model call. The **executor** owns the turn loop. Together they are
the **harness**. Note that this use of "executor" is that project's, and is not
the same as Omega's executor class below.

### Executor, executor class, and lane

**Executor class** — one of exactly three, fixed by ProductSpec
`OMEGA-AGENT-AC-04` and expressed as a closed Rust enum in
`crates/omega_front_door`:

| Class | Wire token | What runs the turn |
| --- | --- | --- |
| Native loop | `native_loop` | Omega's own agent loop in `crates/agent` |
| External ACP | `external_acp` | An agent reached over ACP: Codex, Claude Code, our Exo |
| Engine lane | `engine_lane` | An `omega-effectd` lane. Full Auto, Agent Computer |

A fourth class needs a spec revision. That is why it is an enum and not a
string.

**Wire tokens are not names.** `native_loop`, `external_acp` and `engine_lane`
are persisted and compared. They are never shown to a person on their own — the
reader sees the agent id and the model. This rule was written in the code from
the start and broken anyway: the disclosure line led with `native_loop` until
the owner read it and said he could not tell what it meant.

**Executor** — the concrete thing that runs a turn. Codex is an executor; so is
our Exo; so is Omega's native loop.

**Lane** — a configured route to an executor, with its own readiness. "The Exo
lane" is a lane file naming a binary, a checkout, an `.exo` root, an agent and
a conversation. "Engine lane" is a lane in `omega-effectd`. Avoid "lane" bare
where "executor" is meant.

**Omega Agent** — the router. It owns routing, disclosure and receipts, and
owns **no execution**. A thread the router creates carries the *executor's*
connection, not the router's, so a thread never discloses the router as its
executor.

### Thread, conversation, session, turn

**Thread** — what a person reads and types into. The unit a title belongs to.

**Session** — the executor's handle on that thread. A session id is minted by
the executor, not by Omega. Reopening a thread means loading or resuming its
session, which is why a thread's executor must be known before it can be
reopened.

**Conversation** — our Exo's word for its own durable thread. Not
interchangeable with Omega's thread: a conversation may hold millions of events
while the prompt is a compacted slice of it.

**Turn** — one exchange: a person's message, the executor's work, and its reply.
Steering and queueing negotiate against turn boundaries.

**The log is not the prompt.** A durable record stays complete; the prompt is a
view of it. Compaction changes the view, never the record.

### Project, workspace, workroom

The worst collision in the codebase. Three meanings, and a fourth arriving.

**Project** — a directory on disk that a thread can read. `grep`, `read_file`
and `find_path` operate on a project's worktrees. Zero base opens the working
directory as its project (`OMEGA-DELTA-0054`); before that it opened none, and
every file tool silently returned nothing.

**Workspace (Zed's)** — the window-level type, `workspace::Workspace`. Panels,
docks, the status bar. Inherited from upstream and not worth renaming.

**Audience** — *which people share this history*. Local by default; a
Forge-backed community audience after its invitation is joined. The audience
is recorded on the thread when it opens and does not follow later selector
changes. Community messages are NIP-22 records bound to the Forge repository's
NIP-34 coordinate, signed by the sender's Omega identity, persisted before
network I/O, and sent through a bounded NIP-42-authenticated relay outbox.

**Workroom** — a room with channels, membership, and signed records, in the
Buzz-parity sense. Nostr-primary and native, not a Buzz deployment.

Say **project** for a directory, **window** for the Zed type, and **audience**
for the sharing boundary. “Community workspace” is acceptable product copy for
a Forge-backed audience, but it is not a directory or a Zed workspace.

### Skill

Instructions an agent loads on demand, as a `SKILL.md` with `name` and
`description` frontmatter. Three sources, and precedence runs upward:

| Source | Where | Precedence |
| --- | --- | --- |
| Built-in | compiled into the binary | 0 |
| Global | `~/.agents/skills/` | 1 |
| Project-local | `{project}/.agents/skills/` | 2 |

Higher shadows lower, so shipping a default never takes away someone's ability
to replace it.

**Progressive disclosure**: name and description are always in context, the
body is fetched when the skill is used. A skill is cheap to have and cheap to
ignore.

### Delta

A recorded, deliberate divergence from upstream Zed, numbered
`OMEGA-DELTA-NNNN`, registered in `OMEGA_DELTAS.md`, and enforced by a
mechanical check in `crates/omega_deltas`.

A delta is not a changelog entry. It is a claim with a test attached, and the
test must be **watched failing** before it is trusted: break the behaviour,
confirm the check fails naming the right thing, restore, confirm green.

**Probe that the edit applied first.** A check that "passes" because the
mutation silently did not land is testing nothing. This has happened
repeatedly — including a check that passed against a *commented-out* mutation,
found only by running the falsification.

### Agent, subagent, harness

**Agent** — the behaviour: prompts, tools, policy. Above the harness.

**Subagent** — a thread spawned by another thread, one level deep
(`MAX_SUBAGENT_DEPTH = 1`). It inherits the parent's project and model, returns
only its final message, and its transcript is readable through
`read_subagent_transcript` — scoped so a thread can only read subagents it
spawned.

**Harness** — the substrate plus the turn loop. Our Exo's term; do not use it
loosely for "an agent".

### Forge

**OpenAgents Forge** — an invite-only Nostr and ngit forge at
`openagents.com/forge`. NIP-34 events on the owned relay are the collaboration
fabric.

**GitHub is still authoritative.** The Forge epic describes demoting GitHub to
a read-only mirror as its *target*. That has not happened. Development still
happens on GitHub, and converting it is separate, unscoped work to be figured
out through the workrooms rather than before them. Repeating the target as
though it were the present is a mistake this document exists to stop.

### Zero base

The legacy implementation name for Omega's one normal, flag-free launch
surface. It is not a one-agent product contract and it is not a second product:
the surface contains the conversation, composer, navigation sidebar, tester
channels, and workbench rail. New conversations begin in one of three modes:
**Direct Agent**, **Omega Agent**, or **Sarah**.

The conversation mode is chosen at creation. A direct conversation belongs to
the selected ACP agent, an Omega Agent conversation belongs to the router and
discloses the concrete executor it selects, and a Sarah conversation belongs to
the voice session. An existing transcript never changes executors underneath
its entries. Every title, composer label, status, and disclosure names the
executor actually doing the work.

The new-conversation boundary always shows the three modes in that order. Each
row has exactly one readiness state: **Ready**, **Setup required**,
**Temporarily unavailable**, or **Not supported in this build**. Ready requires
both a connection and a created session, represented by a
receipt bound to the exact target and session. Registration, binary discovery,
or path detection is not readiness. The receipt is volatile; ownership is
restored from the agent identity already persisted with the thread rather than
a second mode column.

New metadata writes preserve the exact conversation owner under the v1 owner
contract. Older rows remain explicitly legacy: a historical native-null row is
recognized as legacy Omega, while a non-null agent id may name either an owner
or a routed executor and is therefore refused as ambiguous. No timestamp, installed-agent match, or route journal upgrades a legacy row by inference.

Sealed, structurally: the window starts with no centre pane group, tab bar,
title bar, or status bar. It is not the editor hidden behind a zoomed panel.
A plain click on a transcript file link is the narrow exception: it reveals the
ordinary editable centre pane beside the thread for that file. Command-click
keeps the compact read-only peek, and closing the last editor tab restores the
agent-only surface.

There is no runtime switch to a second application surface. The legacy
`--full-editor` compatibility path may remain during the alpha transition, but
it is not part of the normal launch and is scheduled for post-alpha removal.
Vim remains supported in the composer and the focused editing surface.

A **path argument names the project, not the mode**. `omega <path>` opens zero
base with `<path>` as the folder the thread reads, searches and runs in; a file
argument names the folder that holds it. The folder is named in the panel
header, because an agent whose directory is invisible cannot be checked. That
header value is always clickable: it chooses a folder when none is attached and
changes the directory after one has been chosen. During the alpha transition,
`--diff`, `--dev-container`, and `--demo-workroom` remain legacy editor-only
arguments and require the compatibility `--full-editor` path — see
`OMEGA-DELTA-0116`. None of those flags changes what a normal, flag-free Omega
launch exposes.

### Words for evidence

**Falsify** — break the thing the check protects and watch the check fail. The
only way a check earns trust.

**False green** — a check that is honest about what it does and misleading about
what it means. Nine were found in one day, every one by accident. Examples: a
test command whose `&&` skipped a whole suite on failure, a brand scan pointed
at the wrong directory, a review tool that assessed an artifact nobody named.

**Not verified by pixels** — a claim about a rendered surface that no one has
looked at. A live process is not a window; an exit code is not a rendered
result; zero panics means nothing about what is on screen.

**Harness-reported** — a number an external system stated about itself. Usable
as a record, never as accounting truth.

## Naming rules

1. **Say which Exo.** "Our Exo" or "exo labs". Never bare, where both fit.
2. **Never show a wire token to a person.** Tokens are for machines. People get
   the agent id and the model.
3. **Project is a directory.** Not an audience, not a window.
4. **Say what is true now, not what is planned.** GitHub is authoritative today.
   A target stated as a fact is how a brief becomes wrong.
5. **Name the layer when a word spans two.** "Executor class", not "executor",
   when the enum is meant.
6. **A number is not evidence unless someone watched it fail.**
