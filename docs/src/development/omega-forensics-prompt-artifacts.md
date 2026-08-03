# Omega Forensics prompt artifacts

Omega treats a forensic prompt as an immutable, structured experiment input. The
active artifact can be inspected or cloned, but never edited in place. Saving a
draft creates a new candidate with a parent reference; activating or reverting a
candidate only changes the active pointer, so prior candidates and every run's
prompt binding remain available.

The artifact uses `openagents.forensic_prompt_artifact.v1`. Its canonical digest
matches the OpenAgents contract: recursively sorted JSON object keys and SHA-256
over prompt IR, parent lineage, examples, parameters, dataset revision, and
compatibility refs. Artifact identity and creation time are intentionally outside
that digest. The baseline now includes the canonical discovery workflow from
OpenAgents commit `5b69795b87`: candidate enumeration, severity ordering,
prior-work search, root-cause identity, falsifier construction, uncertainty
disposition, one finding per root cause, continuation after a duplicate,
style/hardening exclusion, and conservative severity. Omega tests the baseline
artifact against a digest produced by the TypeScript contract implementation.

## Visible task compiler

Every visible repository and catalog Forensics task is compiled through
`omega.compiled_forensic_task.v1`. The compiler accepts the canonical Prompt IR
and one immutable entropy prompt snapshot as bounded domain direction. Its output
binds the exact prompt ref and digest, source and revision, coverage disposition,
focal unit, tranche, model route and parameter digest, available and unavailable
tools, typed schemas, budget policy, domain-text digest, and compiled-task
digest.

The selected focal unit does not hide the rest of the admitted source. The task
states that neighboring source remains readable as context. Source completeness
starts as `incomplete` so task creation stays immediate. The separate
[mechanical source inspector](omega-forensics-source-inspection.md) can then
advance the live workbench projection when recursive dependencies and required
source are mechanically accounted for.
The typed finding, typed hypothesis, and prior-work search tools are named as
unavailable until their separate runtime issues land. Text cannot advertise an
unavailable capability.

The compiler rejects pending or denied coverage, a missing discovery workflow,
overlapping available/unavailable tool sets, and domain bytes that do not match
their immutable digest. The same input produces byte-identical task text and a
SHA-256 digest. User domain text can ask for network, writes, more budget, or
public reporting, but those words remain analytic input and do not change the
typed effective configuration.

## Editor and diff

The workbench's **Prompt artifacts** lane shows the active ref and digest, exposes
save-as cloning, and lists immutable candidates. Structured draft updates produce
semantic diff entries rather than a text-only patch:

- sections: role, threat model, vulnerability classes, invariants, and evidence;
- examples and model-parameter refs;
- finding and hypothesis schema refs;
- requested tool-policy refs; and
- dependency, uncertainty, PoC, severity, context, and budget policies.
- the complete discovery workflow.

Clients edit the structured `ForensicPromptIr` draft through the workbench state
API. Saving recalculates and validates the canonical digest before the candidate
is admitted to the candidate set.

## Authority and launch safety

Prompt prose is analytic input, not authority. It cannot change the repository or
commit, worker placement, network, shell, checkout mode, numeric budget,
reporting, disclosure, or promotion rules. Those remain typed preflight and host
runtime inputs. Before launch, Omega can reject a candidate unless its exact ref,
finding and hypothesis schemas, compatibility refs, and requested tools match the
admitted scan profile and runtime tool surface.

Only typed finding submission creates a finding. Longer prompt or response prose
does not. Typed hypotheses remain unverified leads. The launch event carries the
active canonical prompt digest, and the review projection displays that same
digest so a result can always be attributed to its exact prompt candidate.

For the current visible-task bridge, submission tools are explicitly unavailable,
so transcript Markdown remains diagnostic only. The later typed-tool issue must
admit those tools before a task can create finding or hypothesis state.

Candidates are evaluated in isolated [run matrices](omega-forensics-run-matrices.md)
before any promotion decision.

## Verification

```sh
cargo test -p omega_forensics
cargo test -p agent_ui forensics_workbench --lib
./script/clippy -p omega_forensics -p agent_ui --all-features
```
