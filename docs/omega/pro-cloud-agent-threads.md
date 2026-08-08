# Omega cloud agent threads with Pro

Date: 2026-08-08

Status: design exploration; no feature commitment

Omega source surveyed: `OpenAgentsInc/omega@aa3f47c2ae`

Pro source surveyed: `OpenAgentsInc/pro@e9282c6` and deployed platform records
dated 2026-08-06

Final Pro seam audit reviewed:
`OpenAgentsInc/pro@cbfd96e:docs/convex/omega-cloud-threads.md`, auditing Pro at
`c6ad928`

Convex source surveyed: `get-convex/convex-backend@7caed949c` after updating
local `main` on 2026-08-08

Deployed Pro Convex pin in the latest platform record:
`get-convex/convex-backend@38abb4627`

## Purpose

This note assesses what a Pro account could add to Omega by using the
OpenAgents cloud as a shared state and execution plane. It describes options
and seams that exist in the current code. It does not claim that hosted Omega
threads, multi-user computers, billing, or the Convex ideas below are shipped.

The source basis is the prior Pro cloud-host audit, the relevant Omega and Pro
code and engineering docs, and the agent and self-hosting documentation in the
current Convex backend checkout. Production statements below come from the
latest committed receipts. They were not re-observed during this survey.

## Assessment

The architecture is farther along than the Omega product surface suggests.
Pro already has most of a cloud thread control plane:

- a self-hosted Convex deployment with tenant-scoped reactive queries;
- durable thread shells and bounded semantic transcripts;
- command envelopes with idempotency, conflict detection, and five separate
  acknowledgements;
- browser and mobile command brokers for send, approval, input, and interrupt;
- a leased effect outbox with retry and dead-letter receipts;
- an owner-only GCE machine that runs Omega's headless `eval-cli`;
- a production-tested Rust Convex subscription in Omega;
- a trace path that can archive and derive activity from Omega threads.

One missing link prevents these pieces from forming a hosted Omega thread:
`workShells:sendMessage` records a queued user message, but no runtime consumes
that message. `workShells:interrupt` records a stop request, but no thread
runtime observes it. The existing Omega machine worker only consumes factory
tasks attached to issues.

The first useful hosted feature therefore does not require a new cloud stack.
It requires a thread-turn execution adapter between Pro's existing command
plane and its existing Omega worker.

The final seam audit confirms the authority split in this note and narrows the
first release. My recommendation is:

1. keep Pro/Convex authoritative for shared semantic thread state, commands,
   receipts, attention, and durable execution intent;
2. keep Omega or an Omega-derived supervisor authoritative for coding-agent
   process execution, workspaces, tools, and run artifacts;
3. regularize Omega's existing Convex spike and network disclosures, then ship
   a read-only cloud-thread projection before adding desktop writes;
4. add cloud thread turns to the leased machine-effect lane and prove the
   runtime from Pro web or mobile;
5. add Omega's typed command client and durable local outbox before presenting
   a shared thread as writable in the desktop app;
6. target native Omega agent threads first; external ACP agents need a separate
   Omega-owned portable transcript before they can offer the same guarantees;
7. express cloud placement through the existing `Reach::Shared` policy seam,
   selected before the first message, while keeping local placement the
   default;
8. add true conversation resume and one private computer per account after the
   owner-only turn canary works;
9. use Convex Agent concepts selectively, without replacing Pro's current
   thread contract;
10. keep a Convex backend fork outside the near-term plan.

## What exists now

### Pro account and state plane

Pro is an authenticated operator workspace backed by a self-hosted Convex
deployment at `convex.openagents.com`. The production record describes a
Cloud Run singleton with always-allocated CPU, private Cloud SQL PostgreSQL,
and GCS for modules, files, search indexes, imports, and exports.

Pro derives the Convex tenant from the authenticated OpenAgents user. Native
clients obtain a five-minute read-only JWT from
`GET /api/mobile/controller/token`. Product writes go through
`POST /api/mobile/controller/command`, where Pro derives the actor and
workspace, evaluates the capability, mints server authority, and invokes the
Convex command mutation. A native client cannot choose a tenant or mint write
authority.

The relevant state model is already first-class for threads:

| Plane     | Existing primitive                    | What it provides                                                                   |
| --------- | ------------------------------------- | ---------------------------------------------------------------------------------- |
| Summary   | `workShells`                          | Status, attention, visibility, generation, branch and pull request summaries       |
| Detail    | `workDetails`                         | Bounded append records for messages, work, approvals, input, interrupts, and plans |
| Rendering | `workShells:listTranscript`           | Portable `openagents.work_transcript.v1` semantic rows                             |
| Commands  | `commandReceipts` and `commandEvents` | Replay safety, conflicts, results, and five acknowledgements                       |
| Effects   | `effectOutbox` and effect receipts    | Leases, retries, generation fencing, idempotency, and dead letter                  |
| Evidence  | `agentTraces` and `agentTraceRecords` | Resumable, content-addressed, secret-scanned trace ingest                          |

The five acknowledgements are worth retaining as a product distinction:

1. admission says whether the command was accepted;
2. effect says whether external execution was requested or completed;
3. turn says whether agent work is queued, running, or terminal;
4. quiescence says whether the aggregate has stopped producing work;
5. verification says whether independent evidence supports the outcome.

A generic chat schema usually collapses these states. Pro already avoids that
mistake.

### Omega native client

Omega includes the `omega_convex` crate and initializes it after the hosted
OpenAgents session. The `openagents: open convex inbox` action opens a GPUI
window that:

- gets the existing native session from Omega's credential provider;
- fetches a controller token from Pro;
- runs the official Convex Rust client on a dedicated Tokio worker;
- refreshes authentication after reconnect;
- subscribes to `workShells:attentionInbox` with a hard limit of 100;
- rejects rows that do not match the full web/mobile projection contract.

Omega pins `convex = "=0.10.4"` to the version paired with the deployed
backend. The production receipt covers authentication, subscription,
reconnect, token refresh, and observing a post-reconnect mutation.

This is a compatibility spike and a small inbox window. Omega does not yet
subscribe to the selected shell's semantic transcript, submit controller
commands, persist a Pro command outbox, or present cloud threads in the normal
Agent Panel.

The final seam audit identifies several release constraints around this spike:

- `omega_convex` is not registered in `OMEGA_DELTAS` and currently opens a
  standalone, query-specific window. It should either become a registered
  product integration or remain gated as development-only code.
- The OpenAgents and Convex hosts used by this flow are absent from Omega's
  endpoint allowlist. Shipping cloud synchronization requires deliberate
  entries with purpose, owner, and review policy.
- Native Omega agent threads persist their full transcript in the agent store,
  while the sidebar metadata lives in a separate metadata store. External ACP
  threads depend on the provider's `session/load` and do not give Omega an
  equivalent durable transcript.
- `acp_thread` provides a useful in-memory projection across native and
  external agents, but some entity references, including diff and image IDs,
  do not survive process restart and are not a portable cloud record.

These constraints make the native Omega agent the right first scope. External
Claude Code, Codex, and other ACP sessions should join the cloud contract only
after Omega owns a registered, portable transcript for them.

### Existing placement and publication seams

Omega already has an audience policy for each thread:
`Reach::{ThisComputer, Shared}`. The reach is bound when a thread is created,
cannot be rebound later, and is checked through `may_publish()`. Cloud
placement should become the first transport behind `Reach::Shared`. It should
not become another `ExecutorClass`; Omega remains the executor while reach
decides whether the thread may cross the local-machine boundary.

The existing device and host bridge projections also establish the disclosure
pattern for optional local-thread mirroring. They publish bounded state while
omitting reasoning, tool arguments, tool output, and diffs, and they apply a
safety gate and truncation marker. A future local mirror should reuse that
policy shape rather than copying the local transcript by default.

All Work already models cloud control as generation-fenced commands such as
`StartAgentSession`, `RecordActivity`, and `ControlSession`, using references
instead of embedding sensitive payloads. The cloud-thread command vocabulary
should follow that boundary.

### Omega execution in the cloud

Pro's owner-only machine worker already runs Omega on a private GCE host. The
current path is:

```text
factory dispatch
  -> Convex factory task and machine effect
  -> Pro HTTPS claim endpoint
  -> generation-fenced worker lease
  -> openagents-omega-run
  -> /opt/openagents/omega/eval-cli
  -> bounded result and transcript
  -> Pro delivery and verification
```

The worker polls every two seconds while a run is active, kills the process
group when cancellation or a stale lease is observed, and returns bounded
artifacts. The model credential is fetched from Secret Manager at run time and
is not persisted into the workspace or run evidence.

The machine has a durable workspace and run directories. Each `eval-cli`
invocation starts a new conversation, so this proves persistent filesystem
state and fresh Omega turns. It does not prove conversational process state or
thread resume.

### The missing thread runtime

The gap can be stated precisely:

| Need                                               | State                          |
| -------------------------------------------------- | ------------------------------ |
| Reactive thread list and transcript                | Exists                         |
| Native authentication and token refresh            | Exists                         |
| Brokered message, decision, and interrupt commands | Exists                         |
| Durable machine effect queue                       | Exists                         |
| Cloud machine running Omega                        | Exists for issue factory tasks |
| Omega trace archival                               | Exists                         |
| Message command enqueues a thread-turn effect      | Missing                        |
| Worker claims and runs a thread turn               | Missing                        |
| Running turn appends coalesced semantic progress   | Missing                        |
| Interrupt reaches the exact thread process         | Missing                        |
| Desktop command client and local outbox            | Missing                        |
| Conversation resume across turns                   | Missing                        |
| Per-account computer and device routing            | Missing                        |

The current `sendMessage` acknowledgement reports `effect: not_requested` and
`turn: queued`. That is accurate today. A hosted implementation should change
the effect acknowledgement only when the machine effect is committed in the
same Convex transaction.

## Product options

### Option A: Pro as shared visibility only

Omega continues to run every agent locally. Pro holds attention, task,
artifact, trace, and account projections. The desktop can inspect Pro work and
publish selected local outcomes, but Pro does not run general Omega turns.

Benefits:

- smallest security and operational surface;
- useful account value through cross-device inbox, traces, tasks, and receipts;
- no cloud workspace or model-cost commitment.

Limits:

- closing the laptop stops normal Omega work;
- browser and mobile can observe work but cannot continue a local process;
- thread history remains split between local Omega storage and Pro
  projections.

This is a valid baseline and should remain a supported local-first mode.

### Option B: owner-only cloud threads on the existing machine

Give `aggregateType: "thread"` the execution treatment already used for issue
factory tasks. A queued message creates a machine effect, the existing worker
claims it, and Omega runs a bounded turn in the persistent owner workspace.

Benefits:

- shortest path to "send a prompt, close Omega, and return to the result";
- reuses current auth, receipts, leases, worker, model broker, and trace path;
- proves the product loop before multi-tenant infrastructure is built.

Limits:

- one owner, one machine, one active run through the current `flock` policy;
- fresh `eval-cli` conversation for every message until resume is added;
- the shared workspace creates cross-thread interference unless workspaces are
  partitioned.

This is the recommended first canary.

### Option C: hybrid local and cloud execution per thread or turn

Omega exposes an execution target such as `This Mac` or `OpenAgents Cloud`.
A thread has a default target, and an allowed turn may override it. Pro remains
the command and receipt authority for cloud turns; local turns stay in Omega's
existing runtime.

Benefits:

- preserves Omega's local-first identity;
- lets users move long or asynchronous work to Pro;
- supports local credentials and hardware where cloud custody is unwanted;
- gives the account a concrete benefit without forcing all threads into the
  cloud.

Costs:

- moving a live conversation between runtimes requires a portable context and
  workspace handoff contract;
- local and cloud tool availability may differ;
- the UI must disclose target, funding, data retention, and capability changes
  before admission.

The first version should choose the target when a thread is created and avoid
mid-turn migration. Per-turn switching can follow after the context contract
is proven.

### Option D: lightweight agents inside Convex actions

The `@convex-dev/agent` component can run LLM calls from Convex actions and
persist threads, messages, tool calls, stream deltas, approvals, usage, files,
and retrieval metadata. This is attractive for bounded assistants, triage,
summarization, routing, memory extraction, and scheduled account work.

It is a poor host for Omega's coding runtime. Omega needs a filesystem, Git,
process trees, native tools, potentially long turns, and execution isolation.
Convex Node actions are bounded function invocations. The documented default
Node action timeout is ten minutes, even though the self-hosted source exposes
timeout and concurrency knobs.

Use this option for server-native assistants that emit commands or projections
into Pro. Do not treat a Convex action as a substitute for an Omega computer.

### Option E: one managed Omega computer per Pro account

Move from the owner-only GCE host to the managed-sandbox lifecycle already
described in Pro. Each account receives a stable private computer identity,
persistent disk, generation-fenced supervisor, budgets, funding mode, and
explicit stop, resume, and delete semantics.

Benefits:

- durable workspaces and agent threads across devices;
- resource and credential isolation per account;
- a clean place for repository clones, caches, tools, and user-selected
  providers;
- a foundation for scheduled and delegated work.

Costs:

- lifecycle, quota, idle shutdown, deletion, egress, metering, and support;
- product policy for provider credentials versus OpenAgents-funded inference;
- stronger artifact and workspace retention obligations.

This is the likely product architecture after the owner canary, not a
prerequisite for proving cloud thread turns.

### Explicit non-goal: a Convex backend fork

Omega and Pro should not plan to fork Convex in the near future. The current
product gap is an application/runtime gap: Pro does not dispatch admitted
thread messages into its existing machine-effect lane, and Omega does not yet
have a productized read/write cloud-thread client. Neither problem requires a
database fork.

Keep the deployed backend on a reviewed upstream pin, use Pro functions and
components for state behavior, and use adjacent workers for filesystem and
process execution. If a backend limitation appears, record it and test the
application-level or upstream path; do not place a fork on the cloud-thread
roadmap.

## Recommended hosted thread architecture

### Authority split

```text
Omega / browser / mobile
  -> read-only Convex subscriptions
  -> Pro HTTPS command broker
       -> capability decision
       -> command envelope
       -> Convex transaction
            -> semantic detail
            -> shell projection
            -> command receipt
            -> machine effect
  -> Omega machine supervisor
       -> workspace and process
       -> coalesced progress callbacks
       -> terminal result and trace
  -> Convex terminal projection and receipts
  -> every subscribed client updates
```

Convex owns shared truth about intent and outcome. The supervisor owns the
process and filesystem while a lease is valid. GitHub credentials remain in a
server broker where possible. Model credentials use a short-lived lease or a
platform inference broker rather than a long-lived secret in the guest.

### Canonical thread identity

Add a stable cloud thread reference independent of any one device, process, or
Omega SQLite row. A runtime binding can name:

- execution target: local device or cloud computer;
- account and computer reference;
- workspace reference;
- active lease generation;
- current turn reference;
- optional resume handle and runtime version;
- model, profile, funding mode, and budget policy revisions.

In Omega, the thread's placement is the existing audience reach. A cloud
thread is created with `Reach::Shared` before its first message and remains
shared for its lifetime. A local thread remains `Reach::ThisComputer` unless a
future, separately designed migration flow creates a new shared thread. Reach
is a publication and placement decision, not an executor selection.

Do not put this execution state into `workShells`. The shell should stay a
bounded presentation projection. A dedicated turn or runtime table can carry
lease-sensitive fields, while `workDetails` carries semantic transcript rows.

Omega's local `threads.db` remains authoritative for local-only conversations.
For a cloud thread, the local database may retain a remote reference and cache,
but it should not become a competing shared-state authority.

### Message admission and effect creation

For a cloud-bound thread, `sendMessage` should atomically:

1. validate the command, expected generation, message, context, target, and
   current turn policy;
2. insert the queued user detail;
3. create a durable turn record with a stable turn ID;
4. enqueue a generation-fenced `thread.turn` machine effect;
5. update the shell projection;
6. write the event and command receipt.

The effect payload should contain references and bounded policy, not secrets or
a complete transcript. The worker can fetch the admitted context through a
server broker after it proves the current lease.

The idempotency chain should bind:

```text
command ID -> message detail ID -> turn ID -> effect ID -> run ID
```

Replaying a command returns the same chain. A new payload under an old command
ID conflicts.

### First continuity model

The first cloud-thread canary can reconstruct bounded conversation context
from the semantic transcript and start a fresh `eval-cli` process for each
turn. This provides conversational usefulness without claiming process
continuity. The persistent workspace carries files between turns.

True resume should follow as a separate runtime feature for the native Omega
agent. A resident supervisor can hold or restore its state from an explicit
resume artifact. The resume contract must bind the thread, workspace, runtime
version, model, profile, and generation. A stale or incompatible resume handle
should fall back to disclosed context reconstruction or fail with a visible
receipt.

External ACP agents are a separate continuity project. Their provider session
IDs and `session/load` behavior are not a portable Omega transcript, and the
current in-memory projection contains entity references that do not survive a
restart. Cloud sync for those agents needs an Omega-owned transcript store, a
registered data-format change, and an explicit adapter for provider resume.
The first cloud-thread release should not imply that support.

Workspace isolation is needed before more than one thread uses the machine.
Reasonable early choices are one worktree per cloud thread or one repository
checkout plus isolated worktrees per active task. A single shared working tree
will produce branch and file conflicts.

### Progress streaming

Convex Agent's delta streaming provides a useful pattern: chunk output at a
semantic boundary, throttle writes, single-flight persistence, and let clients
subscribe through normal queries. Pro should apply the pattern to its existing
`workDetails` contract.

Do not append one Convex row per token. Coalesce progress by line, tool event,
or a bounded time window. Keep transient deltas separate from finalized
semantic rows so cleanup does not rewrite the durable transcript. Enforce:

- a maximum update rate per turn;
- maximum resident delta bytes and rows;
- monotonically ordered chunks;
- a terminal compaction into final semantic records;
- disclosed clipping or omission;
- reconnect from the last durable sequence.

The current Cloud Run singleton makes write amplification a capacity concern.
Measure mutation rate and subscription fan-out before tuning the throttle.

### Interrupt and steering

`runtime.interrupt` remains live control and must never be queued offline. When
the turn has not been leased, interrupt can cancel the pending effect. When the
turn is running, the broker marks cancellation for the exact lease generation.
The worker's existing two-second status channel can terminate the process
group.

The receipt should distinguish:

- interrupt requested;
- process termination observed;
- turn terminalized;
- workspace and artifacts flushed;
- quiescence observed.

A later steering command can be a durable message admitted behind the active
turn, or a live runtime capability if Omega gains safe mid-turn steering. The
two behaviors should use different command classes.

### Attachments, artifacts, and Git

Convex file storage can hold bounded user attachments, generated images,
portable artifacts, and content-addressed trace material. Large repository
trees and active workspaces belong on the computer's disk or an artifact store,
not in Convex documents.

The semantic transcript should reference:

- uploaded attachment IDs and hashes;
- tool and terminal summaries;
- diffs and review artifacts;
- branch, commit, and pull request receipts;
- trace ranges for deeper inspection.

GitHub operations should continue through Pro's delegated server connection at
clone, push, and pull request boundaries where feasible. The worker should
receive a scoped capability or exact plan instead of a reusable GitHub token.

### Account, device, and funding policy

The current Convex tenant is the user ID. There is no general device registry
or per-device thread ownership. Hybrid execution needs an account-level device
and computer projection that can answer:

- which device or computer owns the active runtime;
- whether a target is connected and eligible;
- which capabilities and repositories it may use;
- its generation, software version, and last activity;
- whether the next turn is locally funded, user-credential funded, or paid by
  OpenAgents credits.

The missing device identity does not block the owner-only canary: every
signed-in client belongs to the same tenant and the cloud computer is fixed.
The product should name this as an owner-v1 constraint and avoid promising
per-device runtime ownership or routing until that projection exists.

The client must display execution target and funding before admission. Usage
and cost receipts should bind the account, thread, turn, model, provider,
funding mode, and effect.

## What Convex's agent features add

The current Convex documentation describes an application-level Agent
component. It is separate from Pro's schema and is not installed in Pro today.
Its features suggest several useful experiments.

### Useful without a backend fork

#### Asynchronous multi-client streams

The Agent component stores throttled stream deltas in the database so multiple
clients can follow a generation and reconnect without owning the original HTTP
stream. Pro should adopt the persistence and cleanup pattern for cloud turns.

#### Tool approval and human responses

The component persists approval requests, waits for all pending decisions, and
continues from the approval message. Pro already has stronger revision,
expiry, capability, and receipt rules. The component's UI message states can
inform adapters, but Pro's authority contract should remain the source.

The human-agent pattern also maps well to Pro: a person can contribute an
assistant response, answer a tool request, or take over a thread without
changing the transcript model.

#### Cross-thread memory and RAG

The component can combine recent messages with text and vector search, search
other threads for the same user, and mount a namespaced RAG component. Possible
Pro uses include:

- account memory with provenance and deletion controls;
- repository and documentation retrieval;
- retrieving relevant prior fixes and traces;
- thread summaries and compaction checkpoints;
- routing work to a specialized agent from similar past tasks.

Memory should be a derived, inspectable layer. Raw cross-thread retrieval
should not silently widen a thread's authority or data scope.

#### Durable workflows and work pools

Convex documents Workflow, Workpool, and Action Retrier components for durable
multi-step operations, concurrency limits, retry, delay, and recorded step
results. They could support lightweight orchestration such as:

- summarize a completed turn, then extract memory, then notify;
- fan out bounded reviews and combine their verdicts;
- wait for an approval before admitting a deployment effect;
- retry provider calls without retrying a completed Git or payment effect;
- schedule repository maintenance or account follow-up.

Pro's effect outbox remains necessary for machine, Git, payment, and other
external effects. A workflow component should orchestrate command and effect
references rather than create a second effect authority.

#### Files and multimodal context

The Agent component tracks file references from messages, supports images and
files, and can vacuum unreferenced files. This could improve Omega's portable
attachment contract and enable image or document questions from any device.
Large parsing jobs should stay on a worker when they exceed action memory or
time bounds.

#### Usage, quotas, and rate limits

The component exposes usage handlers and pairs with a rate limiter for
per-account and global message or token budgets. Pro can apply the pattern to
both inference and computer resources. Preflight estimates should reserve
capacity, while final provider usage settles the charge and prevents future
work if the account exceeds policy.

#### Scheduled and proactive account work

Convex scheduled functions survive restarts, and cron jobs provide recurring
entry points. They can create command intents for admitted programs such as:

- daily repository or issue summaries;
- stale pull request review;
- dependency or security scans;
- scheduled follow-up from Sarah;
- reminders when an approval, input request, or failed run remains unresolved.

Scheduling creates an intent. It does not bypass capability policy, budgets,
or approvals.

#### Debugging and evaluation

The Agent component supports a playground, context inspection, raw request and
response handlers, usage metadata, and OpenTelemetry. Pro's existing trace
store can receive normalized evidence while an operator-only tool provides
prompt, context, tool, and receipt inspection.

### Why the Agent component should not replace Pro threads

Pro's model covers local and external runtimes, offline client commands,
capability receipts, attention, machine effects, quiescence, and independent
verification. The Agent component centers on LLM messages generated from
Convex actions. Replacing `workShells` and `workDetails` would introduce a
migration and a second authority while losing Pro semantics.

Safer uses are:

1. borrow its streaming, context, file, usage, and workflow patterns;
2. mount it for a bounded server-native agent behind an adapter;
3. project any user-visible result into Pro's canonical semantic transcript;
4. run a compatibility spike against the pinned self-hosted deployment before
   depending on a component in production.

## Self-hosted Convex opportunities

### Stock backend and application-level work

The pulled backend source and docs already support the primitives needed for
most experiments:

- components;
- reactive queries and authenticated Rust clients;
- default and Node action runtimes;
- scheduled functions and cron jobs;
- file storage on S3-compatible storage;
- text and vector search;
- snapshot import and export;
- application concurrency and action-timeout environment knobs;
- log and telemetry integrations;
- a dashboard and system tables for scheduled work.

Ideas that do not need a fork include:

- Pro-native memory and retrieval indexes;
- coalesced stream-delta tables;
- scheduled account programs;
- per-account usage and rate limit tables;
- parent and child agent-thread relationships;
- human takeover and review queues;
- work pools around provider calls;
- content-addressed attachment and artifact references;
- disposable Convex deployments for agent development or test environments;
- snapshot-based restore drills and data-set fixtures.

Self-hosted source knobs can raise Node action timeouts or concurrency, but
that does not turn a function runtime into a secure coding sandbox. Increasing
limits also increases pressure on the same singleton that serves mutations and
subscriptions.

### Near-term backend policy

Keep self-hosted Convex close to upstream and make cloud-thread changes in Pro
application functions, components, effect workers, and surrounding
infrastructure. Preserve the exact backend/client compatibility pin, test
upgrades in a canary, and retain rollback receipts.

Useful experiments such as coalesced streams, tenant budgets, worker wakeups,
snapshot-based environments, and deployment restore drills can all begin
outside the database engine. A limitation discovered during those experiments
belongs in an upstream issue or a product backlog with measurements. It is not
a reason to add a backend fork to the short-term roadmap.

## Pro account experiences this could enable

The architecture can support more than remote chat:

- start a cloud thread in Omega and continue it from web or mobile;
- close the laptop while a bounded turn continues;
- receive an attention notification for approval, input, failure, or result;
- inspect a shared semantic transcript without downloading provider-native
  logs;
- approve a tool call on one device and see the same receipt on another;
- choose local or cloud execution when creating a thread;
- attach a repository, issue, order, file, image, or prior trace as typed
  context;
- let Sarah dispatch or supervise an admitted Omega task;
- schedule bounded maintenance, summaries, or reviews;
- run specialist child agents and fold their results into a parent thread;
- keep repository memory and prior-work retrieval scoped to an account or
  organization;
- track tokens, computer time, storage, and funded credits per turn;
- preserve branches, artifacts, commits, pull requests, and verification as
  linked receipts;
- stop an idle computer while retaining its disk, then resume it later;
- export or delete cloud thread data and derived memory;
- create isolated backend and workspace environments for agent-generated
  changes that include Convex functions.

Each item needs its own product and authority decision. The list describes
possibilities, not a committed Pro plan.

## Short-term roadmap

The shortest credible path is read-only Omega sync, a Pro-owned runtime proof,
and then durable Omega writes. The first shippable scope should be native Omega
agent threads on the owner cloud computer. Resident resume, external ACP
agents, and per-account computers follow after that loop works.

### Phase 0: regularize the release boundary and freeze contracts

1. Either register `omega_convex` in `OMEGA_DELTAS` as a deliberate product
   integration or gate the spike out of release builds.
2. Add the required OpenAgents and Convex hosts to Omega's endpoint allowlist
   with purpose, owner, and review metadata.
3. Freeze the cloud thread, transcript row, command, acknowledgement, turn,
   effect, progress, retention, and funding-disclosure contracts.
4. Register any required change to the closed `ExecutorDisclosure` surface as
   a deliberate delta, while keeping cloud placement out of `ExecutorClass`.
5. Bind cloud placement to `Reach::Shared`, selected before the first message;
   retain `Reach::ThisComputer` as the default.
6. Declare native Omega agent threads as the v1 scope. Record external ACP
   transcript and resume support as a separate compatibility milestone.

Exit gate: Omega has no unregistered release behavior or undisclosed network
destination in this path, the contracts have fixtures, and a thread cannot be
published without the existing audience gate.

### Phase 1: read-only cloud threads in Omega

1. Generalize the Convex worker beyond `workShells:attentionInbox` with typed
   query and decoder boundaries.
2. Subscribe to the shell list and the selected shell's `listTranscript`
   projection.
3. Render cloud threads and their portable semantic rows in the normal Agent
   Panel, not a fixture-shaped standalone window.
4. Implement connecting, synchronizing, cached, live, expired-token, and error
   states while preserving the proven reconnect and token-refresh behavior.

Exit gate: a native cloud thread created through Pro is visible in Omega,
updates on Pro web or mobile appear after reconnect, and Omega still has no
write capability.

### Phase 2: prove the Pro thread runtime from web and mobile

1. Make an admitted cloud-thread message atomically create the user detail,
   durable turn, command receipt, and a generation-fenced `thread.turn` machine
   effect.
2. Add claim, running, coalesced progress, completion, failure, cancellation,
   and stale-generation paths to the existing effect worker.
3. Run `eval-cli` in an isolated worktree for the thread, reconstructing
   bounded semantic context for each turn and labeling that behavior.
4. Route interrupt to the exact turn and process group. Record terminal,
   quiescence, usage, artifact, and trace receipts.
5. Prove the complete path from Pro web and mobile before enabling Omega
   desktop writes.

Exit gate: a prompt admitted from Pro creates one turn/effect chain despite
replay or reconnect, continues after all clients close, streams bounded
progress, survives worker lease recovery, and can be interrupted without
accepting a stale completion.

### Phase 3: add durable Omega writes

1. Add a typed client for Pro's HTTPS command broker. Do not grant the native
   client direct Convex write authority.
2. Add a durable local outbox with the same command classes as web and mobile:
   durable message/input/approval commands and live-only runtime control.
3. Reconcile optimistic local state against admission, effect, turn,
   quiescence, and verification acknowledgements.
4. Surface offline, queued, synchronizing, replayed, conflicted, expired, and
   terminal states in the existing thread UI.

Exit gate: an offline message survives an Omega restart and is admitted at
most once after reconnect; stale approvals and inputs fail closed; live
interrupt is never queued for later delivery.

At this point Omega has the first cloud-synced agent loop: create or open a
native shared thread, observe it on every Pro client, admit work from Omega,
let the cloud computer continue after the desktop closes, and reconnect to the
same semantic transcript and receipts.

### Phase 4: productize placement and optional local mirroring

1. Add an explicit `This Computer` or `OpenAgents Cloud` choice at native
   thread creation, backed by `Reach::ThisComputer` or `Reach::Shared`.
2. Keep reach immutable after the first message. Treat migration as an export
   and new-thread flow until a portable handoff contract is proven.
3. Disclose execution target, model/provider, funding mode, retention,
   workspace, network, and capability changes before admission.
4. Add a per-profile default only after the explicit choice is understood;
   keep the default local.
5. If local-thread mirroring is offered, pass every publication through
   `may_publish()` and use the device-bridge redaction pattern: bounded state,
   no reasoning, tool arguments, tool output, or diffs, plus truncation and
   safety markers.

Exit gate: tests prove no local thread data crosses the machine boundary
without `Reach::Shared` and disclosure, and changing a preference cannot
silently rebind an existing thread.

### Phase 5: continuity and broader account infrastructure

1. Replace per-turn context reconstruction with a versioned native Omega
   resume artifact and resident supervisor.
2. Add restart, upgrade, stale-generation, workspace cleanup, and exact
   process-tree interruption tests.
3. Design an Omega-owned portable transcript and registered data migration for
   external ACP agents before promising their cloud sync or resume.
4. Move from the owner computer to per-account computers only after the owner
   canary establishes resource, credential, egress, budget, deletion, and
   support requirements.
5. Add higher-level Convex components for bounded memory, retrieval,
   summarization, schedules, and work pools behind the existing Pro authority
   contracts.

## Acceptance gates

A hosted thread should not leave owner-only canary status until tests and live
receipts prove:

- the Omega integration is registered in `OMEGA_DELTAS` and every contacted
  OpenAgents or Convex host has a deliberate endpoint-allowlist entry;
- the first supported thread kind is the native Omega agent, while unsupported
  external ACP threads are labeled and cannot enter the cloud placement flow;
- `Reach::ThisComputer` remains the default and `may_publish()` prevents local
  thread state from crossing the machine boundary;
- one admitted message produces one turn and one effect under replay;
- a worker crash and lease expiry cannot apply a stale completion;
- closing every client does not cancel admitted cloud work;
- reconnect refreshes the five-minute read token and resumes subscriptions;
- progress is ordered, bounded, coalesced, and recoverable from a durable
  cursor;
- interrupt kills the exact process tree and reaches quiescence;
- approvals and inputs fail closed when stale, expired, or for another turn;
- each thread has an isolated workspace and branch policy;
- model, provider, funding mode, budgets, and final usage appear in receipts;
- no long-lived model, GitHub, OpenAgents, or Convex credential reaches the
  workspace, logs, transcript, artifacts, or client cache;
- cross-account reads, commands, claims, artifacts, and resume handles are
  denied;
- a stopped computer retains the promised disk state;
- delete proves removal of compute, disk, grants, processes, and scratch data;
- final output is not labeled verified until an independent verifier accepts
  the exact artifact or commit.

## Product decisions still needed

1. Is cloud execution included with Pro, usage-billed, credit-funded, or
   available in several tiers?
2. When should the product offer an explicit migration from a local thread to
   a new shared thread, and which context is portable?
3. Is the first cloud workspace a repository worktree, a persistent computer
   home, or an ephemeral sandbox with exported artifacts?
4. Which models may run with user credentials and which use OpenAgents credits?
5. How long are semantic transcripts, raw traces, workspaces, files, and
   derived memories retained?
6. Which Pro capabilities are available to Sarah, browser clients, mobile,
   Omega, and third-party MCP clients?
7. Is tenant identity still one user, or does Pro need organization and shared
   workspace indirection before launch?
8. Which turns require approval before cloud dispatch, network use, Git push,
   deployment, or spending?

## Recommendation

Build the first cloud-synced agent loop in three proofs: read-only cloud threads
inside Omega, an owner-only Pro thread runtime exercised from web or mobile,
and durable Omega writes through the command broker. Use the current Pro
contracts and add a machine effect plus thread runtime adapter; no new state
plane is needed.

Limit v1 to native Omega agent threads. Bind cloud placement to
`Reach::Shared`, chosen before the first message, and keep local placement as
the default. Regularize the existing Convex spike and endpoint disclosures
before adding UI that can publish data. External ACP agents should wait for an
Omega-owned portable transcript and registered migration.

After the loop works, prioritize resident resume, per-thread workspace
isolation, and the one-computer-per-account lifecycle. Use Convex's agent
ecosystem for bounded orchestration, memory, files, usage, and scheduled work.
Keep the coding runtime on an isolated supervisor and keep a Convex backend
fork outside the near-term plan.

## Source map

Omega:

- `crates/acp_thread/src/thread_projection.rs`
- `crates/agent/src/db.rs`
- `crates/agent_ui/src/thread_metadata_store.rs`
- `crates/app_identity/fixtures/endpoint_allowlist.json`
- `crates/omega_audience/`
- `crates/omega_convex/`
- `crates/omega_device_bridge/`
- `crates/omega_effectd/src/openagents_session.rs`
- `crates/omega_host_bridge/`
- `crates/eval_cli/`

Pro:

- `docs/convex/omega-cloud-threads.md` at `OpenAgentsInc/pro@cbfd96e`
- `docs/convex/omega-rust-client.md`
- `docs/convex/command-envelope.md`
- `docs/convex/client-command-outbox.md`
- `docs/convex/effect-outbox.md`
- `docs/convex/work-transcript.md`
- `docs/convex/production-platform.md`
- `docs/sarah/persistent-computer-plan.md`
- `docs/factory/execution-lane.md`
- `docs/traces/omega-threads.md`
- `convex/workShells.ts`
- `convex/effectRuntime.ts`
- `convex/factoryTasks.ts`
- `ops/omega-computer/openagents-omega-worker`
- `ops/omega-computer/openagents-omega-run`

Convex backend at `7caed949c`:

- `npm-packages/docs/docs/agents/`
- `npm-packages/docs/docs/functions/actions.mdx`
- `npm-packages/docs/docs/functions/runtimes.mdx`
- `npm-packages/docs/docs/scheduling/`
- `npm-packages/docs/docs/self-hosting.mdx`
- `npm-packages/docs/docs/cli/background-agents.mdx`
- `self-hosted/README.md`
- `self-hosted/advanced/knobs.md`
- `crates/common/src/knobs.rs`
- `LICENSE.md`
