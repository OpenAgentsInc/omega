# OFR-024: Independent verdict before patch

Status: code-complete contract, Workbench transition, persistence, and regression coverage. The installed aggregate journey remains deferred to the final Forensics build.

## Outcome

Omega can move one eligible immutable finding into one independent-verification case. Discovery completion does not start verification automatically. The operator must use **Request independent verification**. The transition preserves the complete finding and produces a canonical verifier envelope compatible with the OpenAgents OFR-007 verifier boundary; Omega does not implement a second verifier.

Remediation, patch generation, and superseding PoCs remain unreachable until one immutable initial verdict is durably stored. A review decision, model agreement, applicable diff, completed process, or UI state cannot unlock remediation.

## Verifier envelope

`IndependentVerifierEnvelope` binds and digests:

- the complete immutable finding and finding digest;
- assumptions, causal chain, occurrence references, and root-cause reference;
- source bundle and coverage-manifest identities and digests;
- the original candidate PoC identity and content digest;
- discovery actor, prompt digest and lineage;
- provider, model, route, and configuration provenance;
- discovery tool surface and all evidence references;
- the distinct verifier actor and independently admitted capability references;
- immutable vulnerable and fixed revision digests.

The discovery and verifier identities must differ. Missing PoC, provenance, capability, evidence, or immutable target information refuses the request.

## Verdict and control evidence

The settlement boundary accepts `confirmed`, `dismissed`, or `inconclusive`. It atomically stores one verdict plus its evidence references. Replaying the identical settlement is idempotent. A different second settlement is rejected.

A confirmed settlement requires all of these independent receipts:

1. source validation;
2. dependency validation;
3. application of the original PoC as artifact evidence;
4. an executed vulnerable-revision control with an observed failure;
5. an executed fixed-revision control with an observed success.

Every receipt binds immutable revision, command, environment, output, worker result, outcome, and observation time. Source, artifact/applicability, executed, and independently verified tiers stay distinct. An applicable diff that passes on the vulnerable revision cannot confirm a finding.

## Persistence and failure truth

The case is a canonical, event-ordered projection. Restore validates the envelope digest, dense event sequence, original PoC identity, verdict cardinality, remediation lock, and complete case digest. Terminal failure states remain distinct: refused, worker unavailable, source unavailable, inconclusive, interrupted, failed control, stale source, recovery required, and cleanup failed. None promotes evidence.

Original and superseding PoCs remain separate content-addressed rows. A superseding PoC must name the prior PoC and can only be appended after verdict storage. Finding truth remains immutable if remediation is absent, rejected, or poor.

The retained regression creates 132 requested finding cases with zero verdicts and proves all 132 remain incomplete with remediation locked. It prevents the historical “132 findings, zero settled verdicts” seam from rendering as success.

## Workbench

Each finding card shows the verification state, retained receipt count, and remediation lock. Fixture-backed Work screens can request the independent case with a complete fixture envelope. Live findings refuse this fixture transition until their provider supplies the complete envelope. This makes the provisional UI honest while the final installed provider journey remains batched.

Verified does not imply contact approval, reported, owner accepted, remediated, published, or released. Existing publication gates remain separate and fail closed.

## Verification

```sh
cargo test -p omega_forensics
./script/clippy -p omega_forensics
cargo check -p agent
cargo check -p agent_ui
```

Focused tests cover identity refusal, verdict-before-patch ordering, original/superseding PoC lineage, applicability-without-controls refusal, vulnerable/fixed executed controls, idempotent settlement and restore, and the 132/zero-verdict regression.
