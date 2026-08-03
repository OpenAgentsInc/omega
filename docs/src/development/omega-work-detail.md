# Omega Work and Issue detail

Omega has one detail surface for Work and its Issue projection. The Issue view
is not a second record. It keeps the same Work reference, lifecycle state, and
source revision as Work.

## Identity and source authority {#identity-and-source-authority}

The surface consumes the generated All Work `WorkSnapshot` and `WorkSummary`
types. Omega does not define a second Work or Issue model. An Issue identifier
is a view identifier for the same Work. The view is omitted when the snapshot
does not contain an Issue projection.

The inspector names the owner, human assignee, bounded agent delegate,
delegation grant, generation, source authority, adapter, revision, freshness,
completeness, visibility, and exact source reference. It shows portfolio fields
only when the source supplies them. Missing participants, watchers,
subscribers, Nostr references, evidence, verification, disposition, release,
settlement, and public-claim facts remain explicit. The interface does not
infer them.

## Blocks and activity {#blocks-and-activity}

The central surface supports Conversation, Editor, Diff, Plan, Terminal,
Review, Preview, Log, Metric, Guide, Artifact, and Receipt Block kinds. A Block
keeps the Work and source references that supplied it. Selecting a Block does
not grant permission to change the Work.

Activity composes exact Thread, Session, Agent Session, Agent Activity, Run,
Intent, Event, Receipt, Evidence, Verification, Owner Disposition, and gap
references. Duplicate references are removed. The view renders at most 10,000
rows and reports omitted history.

## Intent and Event admission {#intent-and-event-admission}

An edit starts as a typed Work Intent. It names the Work, actor, source,
expected revision, target generation, idempotency key, timestamp, and operation.
Pending is not accepted. The displayed canonical Work does not change until a
matching source Event arrives with the next revision.

The first writable source is Omega's durable Thread metadata. It admits title
changes only. The source checks its current revision again, saves the title at
the Thread authority, and then emits a canonical Event. Work, Issue, and the
Work Index advance to that one revision. Forensics and Effect-backed Work stay
read-only unless their source later supplies an admitted mutation capability.

Offline, rejected, stale-generation, and revision-conflict outcomes remain
visible and do not change canonical Work. Replaying the same idempotency key
returns the recorded outcome. Reusing the key for a different request is an
error.

Omega stores bounded view state and Intent outcomes under
`work-detail-v1/`. The directory uses mode `0700`, and journal files use mode
`0600` on Unix. The journal does not copy the source authority or canonical
Work summary.

## Interface and navigation {#interface-and-navigation}

From Inbox or My Work, press Space or I, or select **Details**, to open the
detail surface. Enter still opens the source. In detail:

- press E to edit when the source supplies title authority;
- press Enter to submit or Escape to cancel the edit;
- press I to switch between Work and the same-identity Issue projection;
- press Left/Right or H/L to switch Blocks;
- press O to open the source;
- press C or `/` to open the command menu.

The same actions are available to pointer users. Focus returns to the detail
surface after edit submission or cancellation.

## Verification {#verification}

Run:

```sh
cargo test -p omega_work_detail
cargo test -p agent_ui omega_work_and_issue_detail_admit_thread_title_intents_as_canonical_events --lib
cargo test -p omega_deltas omega_work_detail_keeps_issue_identity_and_source_admission_explicit --lib
./script/clippy -p omega_work_detail -p omega_work_index -p agent_ui -p omega_deltas
```

The model suite checks same-identity projection, pending Intent behavior,
idempotent replay, rejected/offline/conflict/stale outcomes, Event and revision
fences, secure journal round trips, 10,000-row history bounds, and deterministic
replay. The GPUI test uses a real Thread metadata source and verifies keyboard
inspection, the command menu, Issue identity, title admission, source revision,
Work Index reconciliation, and source navigation. It does not control an
installed application UI.
