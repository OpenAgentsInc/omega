# OFR-023 typed Forensics tools and live ingestion

Issue: [Omega #230](https://github.com/OpenAgentsInc/omega/issues/230)
Parent: [Omega #208](https://github.com/OpenAgentsInc/omega/issues/208)
Canonical adapter delivery: OpenAgents #9293, landed before this Omega bridge

## Delivered boundary

Visible native Forensics tasks now receive six discovery tools:

- `query_prior_forensic_work`
- `get_forensic_work_by_ref`
- `submit_forensic_hypothesis`
- `submit_forensic_finding`
- `submit_forensic_limitation`
- `validate_candidate_diff_applicability`

`execute_independent_control` is a separate tool. Discovery threads never
register it. Omega can register it only on an independently identified verifier
thread, and the journal admits it only when the call actor matches the distinct
verifier identity.

The compiled task contains an Omega-created binding. Every call binds the run,
task, actor, role, audience, source bundle and digest, coverage generation,
prompt digest, model route, tool version, budget policy, and expected event
cursor. Prompt text and provider-reported availability cannot change that
binding or capability set.

## Canonical journal

`omega_forensics` owns one versioned, serializable journal for accepted and
rejected calls. It retains stable call and event identities, first observation
time, result or refusal reference, canonical call digest, ordered sequence, and
the admitted projection. A repeated identical delivery returns the original
event. A conflicting idempotency key appends a rejection without changing an
earlier accepted result.

Journal restore validates its schema, discovery/verifier split, exact tool
sets, binding, digest shapes, contiguous cursor, event identity, and accepted
versus rejected result shape. The workbench persists the journal with its
repository-scoped Forensics restore state. Reconnect resumes after an exact
cursor and replays completed ACP tool calls idempotently.

There is no assistant-message ingestion API. Markdown remains visible in the
task transcript but cannot mutate the journal.

## Source and evidence admission

For a finding, the live bridge reads cited files from the task's selected
repository root. Admission compares the call with those bytes and the pinned
revision. It rejects a missing file, absolute or traversal path, revision
mismatch, changed bytes, invalid line window, changed source-window digest, or
unsupported symbol. The existing finding contract also requires bounded source
citations, an ordered evidenced causal path, and honest executed-evidence
receipts.

Incomplete dependencies reject a finding. They remain expressible as a typed
hypothesis plus limitation, each with missing evidence and a required next
check. A candidate regression-test diff can enter only at
`artifact_observed`: `executed` must be false and `test_outcome` must be
`not_run`. Only the independent verifier tool can add an executed-control
receipt.

## Live workbench path

Forensics task creation registers the discovery tools before the native task
runs, installs the bound journal, and subscribes to completed ACP tool entries.
Provider argument shapes normalize from a direct structured call, an `input`
envelope, an `arguments` object or JSON string, or the native tool's single
`call` member. All shapes enter the same canonical admission function.

Each accepted or rejected event updates and persists the existing workbench
surface immediately. Findings, hypotheses, limitations, applicability facts,
independent controls, and public-safe rejection facts render from that journal.
The shared Work adapter projects the same journal into lifecycle and Evidence
blocks with prompt, model, source, task, run, budget, event, and refusal refs.
Agent UI does not keep a second findings store.

## Verification receipt

- `omega_forensics`: 62 tests passed. The suite covers fake prose, valid then
  malformed submission, exact source rejection fixtures, missing-dependency
  routing, applicability versus execution, discovery/verifier isolation,
  duplicate delivery, reconnect cursor, restore tampering, provider envelopes,
  and typed fallback conformance.
- `agent`: `cargo check -p agent` passed with all seven native tool definitions
  and role-specific registration methods.
- The full Agent UI check reached only pre-existing baseline failures in
  `omega_dogfood_surface.rs` and the unrelated private `ThreadId` field access
  in `agent_panel.rs`. Its complete diagnostics contain no OFR-023 error.
- The focused release clippy receipt is recorded in the issue close comment.

Per the owner instruction, this issue closes when code-complete. The installed
application proof remains part of the one aggregate build at the end of the
forensic issue sequence.
