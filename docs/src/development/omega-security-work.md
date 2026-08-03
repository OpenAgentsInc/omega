# Omega Security Work

Omega projects repository-bound security analysis as first-class Work. The
existing Forensics workbench remains the source authority. The shared Work
surface makes cases, runs, evidence, model comparisons, and publication gates
addressable without copying or weakening forensic meaning.

## Work identity and decomposition {#work-identity-and-decomposition}

Each Git repository candidate in the active Thread identity becomes a separate
Security case:

- `work:omega:forensics:repository:<repository-id>` is the case Work;
- `work:omega:forensics-run:<run-ref>` is a child run Work;
- current managed runs, entropy runs, campaign project runs, and matrix runs
  keep their exact source run references;
- case details name every child relation, and run details name their parent
  case relation.

The index no longer derives Security Work only from the selected repository.
Two cases can coexist without a singleton mutable case selection. Opening a
case or run goes to the normal Work and Issue detail surface. **Open source**
is an explicit action. For a different repository, that action selects the
exact Thread repository identity before it opens the source workbench.

## Typed Blocks {#typed-blocks}

Security Work uses five domain Blocks:

- **Case** preserves repository, commit, manifest, prompt, catalog, and source
  authority lineage;
- **Lifecycle** preserves preflight/run phase, generation-bound placement,
  event cursor, cancellation, cleanup, recovery, campaign, and project state;
- **Evidence** preserves findings, source citations, hypotheses, limitations,
  review decisions, model disagreement, and exact evidence and receipt refs;
- **Models** preserves model routes, qualified metrics, usage availability,
  token and cost exactness, matrix rows, and promotion state;
- **Publication** preserves privacy and publication authority separately from
  technical completion.

Each Block fact has a stable fact reference, a typed kind, an explicit state,
its value, and zero or more exact source references. The UI renders at most 96
facts in one selected Block and reports the omitted count. The underlying
detail accepts at most 1,024 facts across 64 Blocks.

Missing source, incomplete input, unavailable usage, provisional findings,
cleanup failure, and blocked publication stay explicit. A non-selected case
is still addressable, but it reports its source projection as unloaded until
the operator explicitly selects and refreshes that repository.

## Authority and fixture boundary {#authority-and-fixture-boundary}

Security Work is read-only. A shared Block does not grant source, review,
publication, release, or public-claim authority. Relay state, model votes,
process completion, and a clean worker do not promote a finding or authorize
publication.

Production shows a publication gate only when a source-owned projection is
attached. Otherwise it shows **private, publication blocked**. Bundled
publication scenes and Coldcard evidence remain synthetic development views.
They require the existing explicit test, test-support, or debug mock gate and
cannot appear in a production build.

## Differential boundary {#differential-boundary}

The adapter consumes the source-owned `ForensicsRunProjection`,
`ForensicsReviewProjection`, `EntropyRunProjection`,
`EntropyCampaignProjection`, `ForensicsMatrixProjection`, and
`ForensicPublicationGateProjection`. It does not replace those contracts.

Tests compare exact parent/child Work identity, run refs, evidence refs,
receipts, lifecycle state, and publication state after projection. Property
tests vary bounded unique child-run sets and require every ref to round-trip.
The fail-closed test also proves that a cleaned run does not change a blocked
publication gate.

## Verification {#verification}

Run:

```sh
cargo test -p omega_work_index
cargo test -p omega_work_detail
cargo test -p agent_ui forensics_work_projection --lib
cargo test -p agent_ui omega_entity_routes_render_and_navigate_without_thread_identity_leaks --lib
cargo test -p agent_ui bundled_publication_scenes_are_private_blocked_and_separate_authorities --lib
./script/clippy -p omega_forensics -p omega_work_index -p omega_work_detail -p agent_ui
```

These checks use source-backed contract projections and GPUI route state. They
do not control an installed application UI.

## Installed development receipt {#installed-development-receipt}

The development artifact was built from source commit
`3a06d5b9cad0a4d82aa1a3e9251ac367dac2ed0e`, whose parent was
`7a9154734fc5daa7af6b6b8d7815cce8f67a693c`, with
`OMEGA_PRIMARY_INTERFACE_BUILD=1 script/bundle-mac -f`.

The resulting artifact and installed development application have these
properties:

- application: `/Applications/Omega Dev.app`;
- bundle identifier: `com.openagents.omega.dev`;
- bundle version: `20260803.031121`;
- short version and CLI output: `Omega 0.2.0`;
- architecture: thin arm64 Mach-O;
- installed and bundled `omega` binary SHA-256:
  `17a6798a027b8bb6cb3e761f8b720e4e18d6533eee223c60f0fd81b1ca77de2c`;
- DMG SHA-256:
  `859ced2c5cd46413f112d16782e0709a2c6dd0df66613530c5081a891064449b`;
- the installed binary contains the exact source commit recorded above;
- `codesign --verify --deep --strict` accepted the installed application;
- the replaced development application remains recoverable at
  `/private/tmp/Omega-Dev-replaced-oaw006-20260803.app`, and the earlier
  pre-build copy remains at
  `/private/tmp/Omega-Dev-before-oaw006-20260802.app`;
- the production `/Applications/Omega.app` binary stayed unchanged at SHA-256
  `0475b4f52bd0c79b53a9b4dfafd83a9ed081b7ee8858ba48966ead53ae5a5f73`.

The source verification included 36 Forensics tests, 11 Work Index tests, 9
Work detail tests, 2 differential projection tests, focused GPUI route and
Work detail tests, the publication fixture-boundary test, 316 delta tests,
release all-target and all-feature lint, scoped Prettier, and the pinned mdBook
build. Installed verification used only hashes, metadata, CLI output, embedded
source identity, and signature checks. It did not launch or control the
installed application UI.
