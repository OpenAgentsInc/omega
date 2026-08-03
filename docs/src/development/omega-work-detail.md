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
Review, Preview, Log, Metric, Guide, Artifact, Receipt, Case, Lifecycle,
Evidence, Models, and Publication Block kinds. A Block keeps the Work and
source references that supplied it. Domain Blocks can also contain bounded,
typed facts with explicit observed, provisional, unavailable, missing,
blocked, failed, accepted, or rejected state. Selecting a Block does not grant
permission to change the Work. See [Omega Security
Work](./omega-security-work.md) for the first domain projection.

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

Omega-native Thread Work also has a separate participation journal. The owner
can assign the Work to the local human principal, delegate it to the Thread's
verified Direct Agent, revoke that grant, and record an owner disposition. A
grant names its agent, issuer, generation, allowed Thread capabilities, host
when known, privacy policy, and evidence requirement. It grants no tools,
budget, deadline, release authority, settlement authority, or public-claim
authority by default. Assignment and delegation remain separate facts. Omega
rejects delegation without a human assignee, a second active delegate, a stale
generation, or a grant issued by a different principal.

The participation journal is local authority for Omega-native Thread Work
only. It cannot add authority to Forensics or Effect-backed Work. The Work
Index projects the journal's current assignee and active delegate without
changing Thread ownership. A revoked grant stays in bounded history and cannot
become active again.

Offline, rejected, stale-generation, and revision-conflict outcomes remain
visible and do not change canonical Work. Replaying the same idempotency key
returns the recorded outcome. Reusing the key for a different request is an
error.

Omega stores bounded view state, Intent outcomes, and the separate
participation journal under `work-detail-v1/`. The directory uses mode `0700`,
and journal files use mode `0600` on Unix. The view journal does not copy the
source authority or canonical Work summary. The participation journal is
identity-bound to its Work and source and is validated again before projection.

## Interface and navigation {#interface-and-navigation}

From Inbox or My Work, press Enter, Space, or I, or select **Details**, to open
the detail surface. Source opening is an explicit action from detail. In
detail:

- press E to edit when the source supplies title authority;
- press Enter to submit or Escape to cancel the edit;
- press I to switch between Work and the same-identity Issue projection;
- press Left/Right or H/L to switch Blocks;
- press O to open the source;
- press C or `/` to open the command menu.

For writable Omega-native Thread Work, the Inspector also provides explicit
**Assign to me**, **Delegate**, **Revoke delegate**, **Accept**, and **Needs
changes** actions. A legacy ambiguous Thread owner does not become a delegation
candidate.

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
fences, secure journal round trips, bounded delegation generations, revocation,
owner disposition, 10,000-row history bounds, and deterministic replay. The
GPUI test uses a real Thread metadata source and verifies keyboard inspection,
the command menu, Issue identity, title admission, source revision, Work Index
reconciliation, and source navigation. It does not control an installed
application UI.

## OAW-005 installed receipt {#oaw-005-installed-receipt}

The installed development build was produced from source commit
`afbac88b38186c1685e4107e5b2458712aeac700`, whose parent is
`607f76401ba09f7cce215255c22c23a7a72f0a19`. The release-fast build embeds the
same source commit in its `omega` executable.

- Application: `/Applications/Omega Dev.app`
- Bundle identifier: `com.openagents.omega.dev`
- Bundle version: `20260803.022357`
- Architecture: arm64
- Installed and bundled `omega` SHA-256:
  `e5fe01a17bd1efcd9bade101a0c089fcfc3204d2bc7b9c7476f2e2e8333b835e`
- DMG SHA-256:
  `a3dc6cff64bd296bbf296544a4d8867b88b420f090dcbefe59b17585e762ef39`
- CLI receipt: `Omega 0.2.0 – /Applications/Omega Dev.app`
- Signature receipt: ad hoc arm64 signature; `codesign --verify --deep
--strict` passed before and after installation.
- Recoverable previous development build:
  `/private/tmp/Omega-Dev-before-oaw005-20260802.app`

The exact source passed 8 Work-detail model tests, 9 Work Index tests, 21
workbench-state tests, the focused release GPUI Work/Issue admission test, the
Forensics source-route regression, all 315 delta checks, the pinned mdBook
build, documentation formatting, and release all-target lint for the affected
packages. The GPUI journey creates a real durable Thread metadata row, admits a
revision-bound title Intent at that source, receives the canonical Event,
advances the same-identity Issue projection, refreshes the Work Index, and
opens the exact Thread source.

Installed-artifact verification used hashes, bundle metadata, CLI output,
embedded source provenance, and code-sign checks only. It did not launch or
control the installed UI. `/Applications/Omega.app` was not changed; its
`omega` executable remained
`0475b4f52bd0c79b53a9b4dfafd83a9ed081b7ee8858ba48966ead53ae5a5f73`.
