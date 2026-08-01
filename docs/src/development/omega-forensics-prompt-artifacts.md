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
that digest. Omega tests the baseline artifact against a digest produced by the
TypeScript contract implementation.

## Editor and diff

The workbench's **Prompt artifacts** lane shows the active ref and digest, exposes
save-as cloning, and lists immutable candidates. Structured draft updates produce
semantic diff entries rather than a text-only patch:

- sections: role, threat model, vulnerability classes, invariants, and evidence;
- examples and model-parameter refs;
- finding and hypothesis schema refs;
- requested tool-policy refs; and
- dependency, uncertainty, PoC, severity, context, and budget policies.

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

Candidates are evaluated in isolated [run matrices](omega-forensics-run-matrices.md)
before any promotion decision.

## Verification

```sh
cargo test -p omega_forensics
cargo test -p agent_ui forensics_workbench --lib
./script/clippy -p omega_forensics -p agent_ui --all-features
```
