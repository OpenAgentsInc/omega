# Omega RC installed proof

This is the owned proof protocol for identity issue
[#8](https://github.com/OpenAgentsInc/omega/issues/8) / OMEGA-OID-09 and brand
issue [#16](https://github.com/OpenAgentsInc/omega/issues/16) /
OMEGA-BRAND-06, and Full Auto issue
[#26](https://github.com/OpenAgentsInc/omega/issues/26) / OMEGA-FA-07.

The [Omega slim-agent proof](./omega-slim-agent-proof.md) is the companion
protocol for the basic-agent journeys, profile comparison, and delta sweep. It
uses the same candidate-binding and content-addressed evidence rules. Its
fixture checks do not replace this signed-candidate protocol.

Unit tests and source scans are **not** sufficient. The exact signed candidate
must be installed beside Zed and exercised.

## Prerequisites

1. A release record from `script/bundle-omega-rc` for
   `Omega-v0.2.0-rc3-macos-arm64.dmg`.
2. Matching package digest in that record.
3. OpenAgents signing / notarization evidence when distribution requires it.
4. A clean user profile that already has Zed installed for the side-by-side
   check.

## Command

After `script/bundle-omega-rc` produces the signed and notarized DMG, generate
the admitted identity-candidate record, install that exact candidate, collect
the candidate-bound manual evidence described below, then:

`APPLE_NOTARIZATION_KEY` accepts either inline PEM contents or a readable PEM
file path, so an owner secret exported as `ASC_API_PRIVATE_KEY_PATH` can be
passed directly. The bundle script does not print the input or resolved path.

```sh
script/prove-omega-rc-install \
  --release-record target/omega-rc/omega-v0.2.0-rc3-macos-arm64.release.json \
  --artifact target/omega-rc/Omega-v0.2.0-rc3-macos-arm64.dmg \
  --app /Applications/Omega.app \
  --identity-evidence target/omega-identity-evidence/candidate-evidence.json \
  --identity-matrix target/omega-identity-proof/matrix-evidence.json \
  --identity-recovery-evidence target/omega-identity-proof/recovery-evidence.json \
  --installed-tripwires target/omega-identity-proof/installed-secret-tripwires.json \
  --installed-identity-observations target/omega-identity-proof/installed-observations.json \
  --evidence-root target/omega-identity-proof \
  --full-auto-evidence target/omega-full-auto-evidence/candidate-evidence.json \
  --manual-evidence target/omega-rc-proof/manual-evidence.json \
  --network-evidence target/omega-rc-proof/network-evidence.json \
  --lifecycle-evidence target/omega-rc-proof/lifecycle/manifest.json
```

The Episode 263 release cut has a separate aggregate verdict. After collecting
the public-safe Sarah LiveKit manifest and owner-assisted observations, run:

```sh
script/omega-release-gate \
  --dmg target/omega-rc/Omega-v0.2.0-rc28-macos-arm64.dmg \
  --release-record target/omega-rc/omega-v0.2.0-rc28-macos-arm64.release.json \
  --sarah-livekit-evidence target/omega-release-gate/sarah-livekit.json \
  --owner-evidence target/omega-release-gate/owner-evidence.json \
  --receipt-copy docs/omega/release-gate/<date>-<candidate>.gate.json \
  --report docs/omega/release-gate.md \
  --require-green
```

The owner manifest is
`openagents.omega.release-gate-owner-evidence.v1`; its checked-in schema and
valid self-test fixture live under
`script/fixtures/omega-release-gate/`. Partial manifests are allowed while
collecting evidence, but they leave absent rows pending. Every included row
must name the observer and time, bind the exact package/source, and reference
repository-local public-safe evidence by SHA-256. The independent-review
observer cannot be the candidate producer. The distribution observation must
record claim-for-claim agreement among the Episode 263 transcript, website,
release notes, installed binary, and announcement copy. `--require-green`
turns any pending, blocked, inconclusive, or failed row into a nonzero release
decision; without it, exit zero means only that no automated check failed.

`--report` rewrites the whole file from the receipt, so operator prose does not
live in it. Write prose in the sibling authored sources instead —
`docs/omega/release-gate-overview.md` above the candidate facts,
`docs/omega/release-gate-evidence.md` between the regeneration command and the
row table, and `docs/omega/release-gate-operator-notes.md` after it — and the
gate splices each one back verbatim on every run. A missing, unreadable, or
blank source stops the run with exit 2 before the report is touched, so a
section is never emitted silently empty (OMEGA-DELTA-0219).

Harness validation without an installed candidate:

```sh
script/prove-omega-rc-install --harness-check
```

Bind the exact package and release record to the frozen identity contract,
public vector, reviewed Buzz source, and native package versions:

```sh
script/generate-omega-identity-candidate-evidence \
  --release-record target/omega-rc/omega-v0.2.0-rc3-macos-arm64.release.json \
  --artifact target/omega-rc/Omega-v0.2.0-rc3-macos-arm64.dmg \
  --cargo-lock-snapshot target/omega-rc/Cargo.lock.omega-v0.2.0-rc3 \
  --identity-matrix target/omega-identity-proof/matrix-evidence.json \
  --recovery-evidence target/omega-identity-proof/recovery-evidence.json \
  --installed-tripwires target/omega-identity-proof/installed-secret-tripwires.json \
  --installed-observations target/omega-identity-proof/installed-observations.json \
  --evidence-root target/omega-identity-proof \
  --attestations-dir target/omega-identity-proof/attestations
```

The command writes
`target/omega-identity-evidence/candidate-evidence.json`, then exits nonzero
while required gates are pending. It mounts the DMG read-only, requires exactly
one `Omega.app`, checks its product, bundle identifier, semantic version, and
nonempty macOS build version, and
compares the embedded `omega` and `cli` executable digests with the release
record. Both the DMG and mounted application must pass
`codesign --verify --deep --strict`; their Authority and TeamIdentifier must
match both the release record and the pinned
`Developer ID Application: OpenAgents, Inc. (HQWSG26L43)` /
`HQWSG26L43` identity. A release record naming any other signer or team is
rejected before package metadata is compared. The embedded `omega` executable
must contain the exact release source commit marker compiled as
`ZED_COMMIT_SHA`.

The harness freshly scans every regular, non-symlink file in the mounted
application for each forbidden identity-boundary literal and rejects any
match. It does not trust the release record's packaged-scan assertion by
itself. It also verifies that the explicit build `Cargo.lock` snapshot matches
the name and digest in the release record, then resolves the frozen identity
packages from that snapshot.

The candidate digest is SHA-256 over canonical JSON containing the Omega
commit, identity contract and vector versions and digests, reviewed Buzz
commit, exact native package versions and Cargo checksums, build lock snapshot,
release-record digest, package version and digest, packaged application facts,
and the identity verifier digest and passed source/package scan statuses.
The candidate binds both the release-time assertion and the fresh observed
mounted-app scan. Those automated scans are tripwires, not exhaustive proof of
installed behavior.

Each manual gate remains `pending` until a separate attestation is supplied.
The required inventory includes AssuranceSpec admission; custody, recovery,
forged-request, and stale-task scenarios (including symlink/weak-permission
refusal, unavailable, permission-denied, or corrupt secret-file data, corrupt recovery artifacts,
wrong recovery passwords, and signer crashes); installed scans of logs,
telemetry, clipboard, UI/accessibility tree, diagnostics, and crash output;
the manual journey; owner observation; independent verification;
accessibility; and the full install lifecycle.

To ingest attestations, create one JSON file named `<gate>.json` per gate and
rerun with `--attestations-dir DIR`. Each file must contain exactly:

```json
{
  "candidate_digest": "<64 lowercase hexadecimal characters>",
  "attestor": "Named observer",
  "observed_at": "2026-07-24T12:00:00Z",
  "evidence": {
    "named owner observation bound to candidate_digest": "candidate-bound evidence reference"
  }
}
```

The `evidence` object keys must exactly match that gate's
`required_evidence` strings in the pending record. Custody, forged-request,
stale-task, and installed-tripwire attestations use exact
`{"path":"relative/file.json","sha256":"..."}` references beneath
`--evidence-root`; the generator rejects traversal, symlinks, missing files,
digest mismatch, and substitution of another receipt. Manual-journey and
accessibility attestations use the same reference shape and must bind the
validated installed-observations receipt described below. Recovery attestations
must bind the validated recovery receipt described below. Other gates retain
nonempty candidate-bound results where a file receipt is not applicable. One
generic evidence key cannot pass a multi-requirement gate.

Validate recovery evidence independently before generation:

```sh
script/validate-omega-identity-recovery-evidence \
  --input target/omega-identity-proof/recovery-evidence.json \
  --candidate-digest "<candidate-sha256>" \
  --candidate-artifact-sha256 "<candidate-dmg-sha256>" \
  --candidate-binary-sha256 "<packaged-omega-binary-sha256>" \
  --evidence-root "$PWD/target/omega-identity-proof"
```

The `openagents.omega.identity-recovery-evidence.v1` receipt contains two
distinct content-addressed references. `identity_matrix` must name the exact
passed disposable matrix, including offline creation, encrypted recovery,
wrong-password and corrupt-artifact rejection, and recovery restart
continuity. `rollback_continuity` must name a separately collected
`openagents.omega.identity-update-downgrade-rollback.v2` installed observation.
That observation records equal before/after public fingerprints, an exact
candidate-to-older downgrade followed by update and rollback to the candidate,
the final installed candidate binary digest, exact supporting evidence
references, and its own canonical digest. Its evidence inventory is the
candidate-before observation; downgrade removal, install, and resulting
identity; and rollback removal, install, and resulting identity. The validator rejects stale
candidate binding, artifact or binary substitution, changed or secret-bearing
fingerprints, unsafe or repeated paths, digest mismatch, and either recovery
source being substituted for the other. It validates supplied observations;
it does not create or infer them.

Validate the installed manual and accessibility packet independently before
generation:

```sh
script/validate-omega-identity-observations \
  --input target/omega-identity-proof/installed-observations.json \
  --candidate-digest "<candidate-sha256>" \
  --evidence-root "$PWD/target/omega-identity-proof"
```

The packet uses schema
`openagents.omega.identity-installed-observations.v1` and contains exact
`manual_journey` and `accessibility` arrays. Every entry has only `check`,
`status`, `observed_at`, typed `facts`, and one or more content-addressed
`evidence_refs`. The manual inventory is identity-first first run, the
preserved Aiur/Ayu/Gruvbox Theme families with visible Agent Setup, and equal
before/after Zed digests. The accessibility inventory is forward and reverse
keyboard traversal with visible focus and keyboard activation; named
screen-reader output with labeled controls, announced identity status, and no
secret value; exact 360-pixel width without overflow; UI font size of at least
18 pixels without clipping; light and dark legibility; system increased
contrast; and system reduced motion with no motion required for completion.

The validator rejects missing or extra checks and facts, non-passed status,
timestamps without timezones, duplicate references, absolute or parent paths,
symbolic links in any referenced path, digest mismatch, stale candidate
binding, changed Zed state, and an invalid internal packet digest. It does not
capture or infer screen-reader, screenshot, setting, or visible results.
Omega's `dev: dump accessibility tree` action supplies structured
assistive-technology corroboration after the platform accessibility adapter is
active; an operator still records the visible and screen-reader observations.

Only a record with every required gate validly attested sets
`candidate_admitted: true`, reports `status: "admitted"`, and exits zero.
Missing, malformed, or candidate-mismatched attestations keep the record
pending and the command exits 3. The independent-verification attestor must
differ from the owner-observation attestor. Generation never creates
attestations.

Validate the deterministic generator without claiming candidate evidence:

```sh
script/generate-omega-identity-candidate-evidence --self-test
```

### Full Auto assurance evidence

Before assembling Full Auto observations, collect the durable receipts for
explicit installed run refs from the release-namespaced registries:

```sh
script/collect-omega-full-auto-installed-evidence \
  --release-record "$PWD/target/omega-rc/omega-v0.2.0-rc3-macos-arm64.release.json" \
  --identity-evidence "$PWD/target/omega-identity-evidence/candidate-evidence.json" \
  --app /Applications/Omega.app \
  --registry-root "$HOME/Library/Application Support/Omega RC/openagents/full-auto" \
  --run-ref run.full-auto.<explicit-ref> \
  --run-ref run.full-auto.<explicit-handoff-ref> \
  --output "$PWD/target/omega-full-auto-evidence/observed/installed-runs.json"
```

The collector accepts only the exact `/Applications/Omega.app` executable
whose digest, bundle identifier, version, release record, and identity
candidate agree. On macOS, the registry directory must be the private
`0700` directory derived from the release record's Omega namespace. The four
required registry files must be regular non-symlink `0600` files with the
expected run, report, provider-handoff, and native-binding schemas. Duplicate
JSON keys, duplicate registry refs, symbolic-link traversal, weak modes,
candidate substitution, registry digest changes, and inconsistent
run/report/handoff/native bindings are refused.

Output is a public-safe projection. It includes controlled lifecycle and
provider enums, counts, timestamps, pre-existing objective and done-condition
digests, hashed opaque thread/workspace/project/worktree refs, canonical
record and registry digests, and exact candidate bindings. It never emits
objectives, prompts, titles, reasons, outcome summaries, transcripts, raw
paths, credentials, tokens, or other free-form content. The output file is
written atomically with mode `0600`; `evidence_sha256` is the SHA-256 of its
canonical JSON before that field is added.

This receipt can support the applicable installed observations, but it does
not decide that an obligation or issue gate passed and does not create a
reviewer or release-authority decision. Test deterministic projection and
duplicate-key, tamper, redaction, symlink, and weak-mode refusal with:

```sh
script/collect-omega-full-auto-installed-evidence --self-test
```

The admitted Omega Full Auto AssuranceSpec is revision 5. Its design-admission
receipt does not prove an installed candidate. Collect a separate packet with
schema `openagents.omega.full-auto-observations.v1`, then bind it to the exact
identity candidate, Omega commit, DMG, release record, embedded
`omega-effectd`, admitted AssuranceSpec proposal/document/receipt, and
ProductSpec:

```sh
script/generate-omega-full-auto-candidate-evidence \
  --release-record target/omega-rc/omega-v0.2.0-rc3-macos-arm64.release.json \
  --artifact target/omega-rc/Omega-v0.2.0-rc3-macos-arm64.dmg \
  --identity-evidence target/omega-identity-evidence/candidate-evidence.json \
  --observations target/omega-full-auto-evidence/observations.json \
  --evidence-root "$PWD/target/omega-full-auto-evidence/observed" \
  --output target/omega-full-auto-evidence/candidate-evidence.json
```

The observation packet contains exactly the eight
`AO-OMEGA-FA-AC-01-01` through `AO-OMEGA-FA-AC-08-01` obligations and these
ten issue gates: incident replay, owner-real multi-turn, restart
reconciliation, control matrix, visible cross-provider handoff, offline and
Sync gap behavior, mobile typed outcomes, ordinary-chat separation,
redaction, and exact-candidate independent review. Each observation carries a
timestamp, evidence tier, and one or more evidence references. All eight
obligations and every issue gate, including incident replay, require
`installed_candidate`; source tests cannot satisfy them.

Every `evidence_refs` item is exactly
`{"path":"relative/file","sha256":"<64 lowercase hexadecimal>"}`. Paths
resolve beneath the explicit absolute `--evidence-root`. The generator and
installed prover both reject absolute or parent-traversing paths, symbolic
links in any path component, non-regular files, digest mismatches, and
duplicate references in one observation. Referenced JSON must contain the
same top-level `candidate_digest`; image, video, audio, and other non-JSON
files remain bound by their digest. The evidence root itself must be a real,
non-symlink directory.

The distinct verifier first writes a durable JSON decision under the evidence
root with schema `openagents.omega.full-auto-authority-decision.v1`, action
`verify_omega_full_auto_candidate`, actor role
`openagents.assurance_reviewer`, outcome `succeeded`, no predecessor, the four
candidate bindings, and structured evidence references. The owner writes a
separate later decision with action `release_omega_full_auto_candidate`, role
`openagents.owner`, and `predecessor_decision_ref` equal to the verifier's
decision ref. Decision refs use `authority.decision.*`.

`observations.json` includes structured `verification_decision` and
`release_decision` references. Its independent attestation repeats the
verification decision ref; its owner attestation repeats the release decision
ref. Both attestations repeat the candidate, artifact, release-record, and
Omega commit bindings and use structured evidence references. The owner actor
must equal the admitted identity-candidate owner, and the verifier must be a
different actor. Release must occur strictly after verification.

The generator rejects missing or unknown gates, duplicate JSON keys or
references, source-only incident or obligation evidence, stale or forged
files and bindings, role substitution, unordered decisions, and owner
self-review. It never creates observations, attestations, verification, or
release decisions. Test its refusal logic with:

```sh
script/generate-omega-full-auto-candidate-evidence --self-test
```

### Disposable installed identity proof matrix

After installing the exact signed candidate, exercise its packaged
`omega-identity-proof` driver without launching Omega:

```sh
script/run-omega-identity-proof-matrix \
  --release-record "$PWD/target/omega-rc/omega-v0.2.0-rc3-macos-arm64.release.json" \
  --artifact "$PWD/target/omega-rc/Omega-v0.2.0-rc3-macos-arm64.dmg" \
  --candidate-evidence "$PWD/target/omega-identity-evidence/candidate-evidence.json" \
  --output "$PWD/target/omega-identity-proof/matrix-evidence.json"
```

The runner accepts only `/Applications/Omega.app` and the packaged driver
whose digest is jointly bound by the release record and candidate evidence. It
also requires the installed driver to pass strict code-signature verification.
Its temporary roots are internally created with the proof-only prefix, and the
driver itself fixes the version-one logical custody locator to
`com.openagents.omega.identity-proof.v1` / `disposable-proof-only`.

The matrix covers creation and read-back, process restart, signing,
same-receipt double creation, distinct-receipt rejection, concurrent creation
and process start, forged and stale requests, reset and relaunch, and every
exposed crash checkpoint. It also records explicit deterministic, no-live-store
simulations for conflict/lost/locked custody, unsafe public stores,
unavailable/corrupt secure storage, malformed or unadmitted signing requests,
conflicting recovery selection, late completion, and signer crash. Simulation
receipts are labeled as such and do not claim live owner-store execution. It
resets the disposable entry between live cases and
fails if final cleanup cannot be proved.

The installed runner also creates a separate proof-only identity under a
deny-network sandbox, generates an ephemeral recovery password, and passes the
password to the signed driver only through an inherited file descriptor. The
driver writes a fixed encrypted artifact inside the internally generated proof
root. The runner exercises recovery protection, wrong-password rejection,
corrupt-artifact rejection, recovery adoption, and process-restart identity
continuity before proving final secret-file cleanup. The password, artifact path,
identity, and command output are not retained in the public receipt. Downgrade
and rollback continuity remains a separate real install-lifecycle observation;
the disposable recovery case does not stand in for that candidate journey.

Evidence contains candidate and
component digests, case states, and hashes of public driver outcomes; it does
not retain identities, secrets, command output, temporary paths, or error
details from the driver.

Run the deterministic harness checks without touching the live identity store:

```sh
script/run-omega-identity-proof-matrix --self-test
```

Before packaging, run the static supply-chain and secret-boundary gate:

```sh
script/verify-omega-identity
```

It verifies the exact `nostr`, `keyring`, and `atomic-write-file` locks and
checksums, the reviewed Buzz commit, the frozen public vector, public-manifest
field safety, renderer serialization boundaries, startup inspection without
minting, and the absence of the rejected Buzz secret APIs and fallback paths.
`script/bundle-omega-rc` runs this gate before building and rejects an assembled
application bundle containing the forbidden legacy secret literals. These
package gates complement, but do not replace, tripwire scans over the installed
candidate's logs, telemetry, clipboard, accessibility tree, diagnostics, and
crash output.

Incomplete proofs exit non-zero and write `target/omega-rc-proof/installed-proof.json`
with blockers. Do not close issue #16 until `status` is `complete` and the
manual UI checklist in that proof is signed off.

The harness still runs and records the artifact/release binding,
notarization/Gatekeeper assessment, and exact packaged-versus-installed bundle
comparison when identity admission or manual evidence is missing. Those
objective passes do not imply acceptance: the proof remains `incomplete`, and
the missing identity and human gates remain blocked or pending.

The objective component gates also validate the pinned `omega-effectd`
release manifest and packaged digests, verify the nested Node signature, and
launch both the mounted and installed component copies through framed
`initialize` and `health` requests. A present file without a running health
response is not a component pass. The manifest binds the pristine release Node
digest, while the release record separately binds the post-signing Node bytes;
proof also requires the signed runtime's exact minimal two-entitlement set.

### Installed-candidate bindings

The harness verifies the explicit DMG digest against the release record, checks
the notarization record and staple, mounts the DMG read-only, verifies the
package and application signatures, and compares every regular file and
symlink in the installed application with the application in the DMG. It also
checks the `omega` and `cli` executable digests against the release record.
Passing source scans, a similarly signed application, or a matching bundle
identifier cannot substitute for the exact installed candidate.

The identity evidence must use
`openagents.omega.identity-candidate-evidence.v1`, bind the same artifact,
release record, and source commit, and report `candidate_admitted: true` with
every gate passed. In particular, its manual journey, owner observation, and
independent verification gates must pass with distinct named attestors.

The manual evidence is a JSON object with schema
`openagents.omega.installed-brand-evidence.v1`, the exact
`candidate_digest`, and these exact `checks` keys:

- `clean_profile_side_by_side_zed`
- `offline_identity_first_start`
- `editor_open_edit_save_git_terminal`
- `restart_project_and_layout_restoration`
- `visible_brand_and_accessibility_labels`
- `icon_bundle_and_file_association_branding`
- `shell_installer_and_disk_image_branding`
- `legal_licenses_and_notices`
- `network_destinations_against_allowlist`
- `data_cache_log_browser_update_and_credential_isolation`
- `disabled_services_and_update_behavior`
- `uninstall_preserves_zed_data`

Each check is `{"status":"passed","evidence":"<candidate-bound receipt>"}`.
The object must also contain `owner_observation` and
`independent_verification` objects with `status`, distinct nonempty `attestor`
names, `observed_at`, and a candidate-bound `evidence` reference. The harness
does not generate or infer these attestations.

### Network capture evidence

`script/omega-network-capture-journey` keeps schema
`openagents.omega.network-capture.v1` and requires collector version 2 fields.
Before capture, it binds the root process to the canonical executable
`/Applications/Omega.app/Contents/MacOS/omega`, the installed executable
SHA-256, device and inode identity, strict deep `codesign` verification, leaf
authority `Developer ID Application: OpenAgents, Inc. (HQWSG26L43)`, and team
`HQWSG26L43`. Every sample records the root PID start identity and executable
binding. A reused PID, changed process start, different executable path, or
changed installed binary blocks the receipt.

The validator rehashes the current collector and installed executable, reruns
strict code-sign verification, requires one exact root process row, and checks
every recorded root identity sample. It does not trust the receipt's passed
status or `matches_installed_binary` field. Validate the candidate-bound
receipt immediately after capture:

```sh
script/omega-network-capture-journey validate \
  --candidate-digest "<candidate-sha256>" \
  --input "<network-capture.json>"
```

Old v1 receipts without the additive collector, installed-binary, code-sign,
and root-identity bindings must be recaptured. The self-test tampers the root
start identity, canonical path, signing team, and collector digest and requires
each forgery to fail:

```sh
script/omega-network-capture-journey self-test
```

### Lifecycle evidence

Lifecycle proof is a set of machine-readable receipts, not a free-form note.
Capture before, after-removal, and after-reinstall snapshots with
`script/omega-rc-lifecycle-proof snapshot`. Use its `remove-app` command with
literal confirmation `REMOVE-OMEGA-RC-APP` and a narrow recoverable holding
directory. Reinstall the exact release-record-bound DMG with its `reinstall`
command and literal confirmation `REINSTALL-OMEGA-RC-APP`.

Run `compare` for Zed before versus after removal, Zed before versus after
reinstall, and Omega before versus after reinstall. Create `manifest.json`
with schema `openagents.omega.installed-lifecycle-evidence.v1`, status
`passed`, the candidate/artifact/release-record digests, and exactly these
receipt references:

- `before_snapshot`
- `after_removal_snapshot`
- `after_reinstall_snapshot`
- `zed_after_removal_comparison`
- `zed_after_reinstall_comparison`
- `omega_after_reinstall_comparison`
- `app_removal`
- `app_reinstall`

Each reference is
`{"path":"relative/receipt.json","sha256":"<digest>"}`. The installed
prover reloads and hashes every non-symlink receipt, verifies each
comparison's snapshot hashes and empty change set, and requires both removal
and reinstall to state that no Zed target was touched. The reinstall receipt
must bind the same DMG and release record.

Lifecycle schema `openagents.omega.rc-lifecycle-proof.v1` now requires
collector version 2 on snapshots, comparisons, removal, reinstall, and cleanup
receipts. Each snapshot carries the exact Omega and Zed root-name inventory,
root count, and a canonical digest over every manifest summary. Validation
requires every expected root exactly once, verifies each path binding, status,
count, byte total, symlink count, and manifest digest shape, then recomputes the
inventory digest. A comparison binds both complete snapshot hashes, both scope
manifest-set digests, and the exact expected root names. It cannot pass because
the same root was omitted from both snapshots.

Validate a snapshot directly, or a comparison against both source snapshots:

```sh
script/omega-rc-lifecycle-proof validate \
  --candidate-digest "<candidate-sha256>" \
  --input "<snapshot.json>"

script/omega-rc-lifecycle-proof validate \
  --candidate-digest "<candidate-sha256>" \
  --input "<comparison.json>" \
  --before "<before-snapshot.json>" \
  --after "<after-snapshot.json>"
```

Receipts created before collector version 2 lack these additive bindings and
must be regenerated. The self-test rejects a missing expected root, a forged
manifest summary, a forged collector digest, and an incomplete comparison
inventory. It also rejects a `passed` comparison when both snapshots contain
the same blocked manifest:

```sh
script/omega-rc-lifecycle-proof self-test
```

Incomplete proofs exit non-zero and write
`target/omega-rc-proof/installed-proof.json` with structured pending or passed
gates and blockers. The v3 proof also records SHA-256 for every file input:
release record, DMG, identity evidence, Full Auto evidence, manual evidence,
network evidence, and lifecycle manifest. `status` can be `complete` only
when every automated and manual gate passes. Validate tamper and false-green
refusal behavior with:

```sh
script/prove-omega-rc-install --self-test
```

Do not close issue #16 until `status` is `complete`.

## Required journey

1. Verify the immutable candidate digest against the release record.
2. Install Omega RC beside Zed in a clean user profile.
3. Offline first start through the identity-first entry surface.
4. Open one local project, edit and save one file, open Git status, open a
   terminal.
5. Quit and relaunch; confirm project/layout restoration.
6. Inspect visible product text and accessibility labels for Zed product or
   publisher claims.
7. Inspect icons, bundle metadata, file associations, shell integration,
   installer/disk-image text, licenses, and notices.
8. Capture network destinations from first start through shutdown against
   `crates/app_identity/fixtures/endpoint_allowlist.json`. Exercise the first
   configured `codex-acp` start so the capture includes ACP registry, managed
   Node/npm fallback when applicable, and npm dependency installation. The
   allowlist is evidence for this review; it is not enforced by the process.
   The collector records the contemporaneous DNS answers used to classify an
   approved hostname when `lsof` exposes only its IP address. Loopback traffic
   is retained in the receipt but is not classified as an external host.
9. Inspect data, cache, log, browser, update, and credential roots for Omega
   vs Zed separation.
10. Confirm disabled service / update behavior is honest.
11. Remove Omega and confirm Zed data did not change.
12. Record owner observation and independent verification in the proof record.

## Installed secret-tripwire scan

Issue #8 requires the exact disposable test secret to be absent from installed
runtime surfaces. Use a disposable identity in an isolated macOS test user;
never use or extract the owner's real identity secret. Put the canary in a
mode-0600 file, pass it through a protected file descriptor, and capture the
clipboard and accessibility tree into private temporary files before scanning.

The scanner never prints the canary or a matching path. Its JSON contains only
per-surface state, counts, and evidence digests. A match exits 1; an unreadable
surface or invalid invocation exits 2.

```sh
OMEGA_PROOF_USER_ROOT="/Users/omega-rc-proof"
OMEGA_PRIVATE_PROOF_DIR="$(mktemp -d)"
chmod 700 "$OMEGA_PRIVATE_PROOF_DIR"

script/scan-omega-secret-tripwires \
  --candidate-digest "<exact candidate digest>" \
  --needle-fd 3 \
  --surface "logs=$OMEGA_PROOF_USER_ROOT/Library/Logs/omega-rc" \
  --surface "telemetry=$OMEGA_PRIVATE_PROOF_DIR/telemetry" \
  --surface "accessibility=$OMEGA_PRIVATE_PROOF_DIR/accessibility.json" \
  --surface "clipboard=$OMEGA_PRIVATE_PROOF_DIR/clipboard.bin" \
  --surface "diagnostics=$OMEGA_PRIVATE_PROOF_DIR/diagnostics" \
  --surface "crashes=$OMEGA_PROOF_USER_ROOT/Library/Logs/DiagnosticReports" \
  --output "$OMEGA_PRIVATE_PROOF_DIR/installed-secret-tripwires.json" \
  3<"$OMEGA_PRIVATE_PROOF_DIR/disposable-canary"
```

The candidate generator and installed prover directly reload this receipt,
require the exact six-surface inventory, and rehash it. An absent optional
crash or diagnostics surface is recorded as `absent`, not
silently treated as scanned. The independent reviewer must decide whether each
absence is expected for the exercised journey before admitting the attestation.

### Collecting the receipt in one command

`script/collect-omega-installed-tripwires` produces the same receipt without
the operator naming six paths. It resolves them from `crates/paths/src/paths.rs`
instead:

| surface       | where it is read                                                        |
| ------------- | ----------------------------------------------------------------------- |
| logs          | `paths::logs_dir()` — `~/Library/Logs/omega-rc`, less the telemetry log |
| telemetry     | `~/Library/Logs/omega-rc/telemetry.log`                                 |
| diagnostics   | `paths::database_dir()` and `paths::hang_traces_dir()`                  |
| crashes       | `paths::crashes_dir()` — `~/Library/Logs/DiagnosticReports`             |
| clipboard     | the general `NSPasteboard`, through PyObjC or `pbpaste`                 |
| accessibility | the running candidate's `AXUIElement` tree                              |

```sh
script/collect-omega-installed-tripwires \
  --app /Applications/Omega.app \
  --candidate-evidence target/omega-identity-evidence/candidate-evidence.json \
  --output "$OMEGA_PRIVATE_PROOF_DIR/installed-secret-tripwires.json" \
  --needle-fd 3 3<"$OMEGA_PRIVATE_PROOF_DIR/disposable-canary"
```

Three rules make the receipt worth reading.

`--needle-fd` is required. The canary is the one the candidate was actually
given during the journey, read from a descriptor the caller opened on the
mode-0600 file, and the collector will not generate one of its own. A secret no
other process has ever seen cannot have been leaked by anything, so a search
for it passes by construction — which is what an earlier version of this
command did, and what made its `pass` worth nothing.

The scan is proved able to fire before it is trusted. The needle is planted in
a private file and the same scanner is pointed at it; a scanner that does not
come back with that match ends the run at exit 2 with no receipt written,
rather than reporting that nothing was found.

A surface that could not be observed records `blocked` and the run exits 3.
That includes a logs or diagnostics root that does not exist — which means the
run looked somewhere the product does not write — a pasteboard that holds
flavours the read cannot see, and an accessibility tree that could not be read
because no candidate is running or because the terminal has not been granted
Accessibility in System Settings. `validate_installed_tripwires` refuses any
receipt carrying a `blocked` surface, because "nothing was found there" and
"nobody looked" must not read the same. The clipboard and accessibility
captures are written mode-0600 into a private temporary directory and deleted
when the run ends.

`script/collect-omega-installed-tripwires --self-test` checks the scanner on a
planted needle, checks that the live-fire control catches a scanner that never
matches, checks the canary length bounds, and checks that the surfaces still
resolve to `paths::logs_dir()` and `paths::crashes_dir()` rather than under the
application support root.

## Installed observations

`script/collect-omega-installed-observations` performs the eleven installed
observations. It performs them; it does not accept them as assertions. A check
it cannot perform on the host records `blocked` and the run exits nonzero,
because an observation nobody made is worse than a missing one. It looks like
evidence.

Capture the Zed data digest, then install and exercise the candidate, then
collect:

```sh
script/collect-omega-installed-observations \
  --app /Applications/Omega.app \
  --candidate-evidence target/omega-identity-evidence/candidate-evidence.json \
  --evidence-root target/omega-identity-proof \
  --zed-before "<digest captured before installing Omega>" \
  --output target/omega-identity-proof/installed-observations.json
```

The historical upstream parity probe is diagnostic only. It cannot waive an
absent Omega accessibility tree and is not part of candidate admission.

`light-theme` and `dark-theme` take their facts from the captures, not from the
host. The host appearance is their precondition, exactly as the increased-
contrast flag is the precondition of `high-contrast`: the window has to be
legible in each appearance — twelve lines recognised at confidence 0.5 off the
rendered capture — and the light and dark captures have to differ by at least
50,000 pixels. A frozen, blank, or appearance-ignoring window returns the same
image twice and blocks both checks. `content_legible` was a hardcoded literal
until omega#90, so a window that never repainted passed both.

`script/collect-omega-installed-observations --self-test` exercises that
decision offline, without driving the host or the candidate: it requires an
unreadable capture, a capture with too little legible text, an uncomparable
pair, and two captures of the same picture each to fail, and it asserts that
the appearance block still calls `ocr_lines`, `confident_lines`, and
`differing_pixels`, so the decision cannot quietly stop being wired to the
pixels.

Every keystroke the collector sends brings the candidate to the front first and
refuses if it is not there. macOS routes a synthesized key to whichever
application is frontmost, not to the process the script names, so an unchecked
keystroke lands in whatever window happens to be in front. In an earlier pass
this opened a Page Setup sheet in the operator's terminal, and that sheet was
very nearly recorded as the candidate responding to the keyboard.

### Historical waivers

Older alpha evidence can record `waived`. A waiver is not a pass. It is a
record that an observation _did not happen_. OAW-012 supersedes the alpha
accessibility exception for v0.2.0 candidates: `WAIVABLE_CHECKS` is now empty,
and every installed accessibility observation must be performed.

A `waived` entry carries no `facts`. It carries a `waiver` with the owner's
own words, the date of the direction, the basis, the issue, and the upstream
parity the direction is conditioned on. A `passed` entry carries facts and no
waiver. The two shapes are disjoint, so a waived entry cannot be relabelled
`passed` without also inventing the facts the observation would have produced,
and `validate_observation_group` refuses either shape in the other's place.

Three further rules keep a waiver visible:

- The report status is `passed_with_waivers`, never `passed`. A reader can see
  at a glance that something was not observed.
- The top-level `waivers` list and the waived entries must agree in both
  directions. A waiver cannot be dropped from the summary, and a summary
  cannot name a waiver that no entry records.
- `WAIVABLE_CHECKS` is empty. The validator refuses a waiver of every current
  installed check, including `screen-reader-output`.

`script/test-omega-installed-observation-waivers` asserts all of this, so the
waiver cannot quietly become a green claim later.

The 2026-07-25 screen-reader waiver remains historical evidence for candidates
created under that decision. It does not apply to a v0.2.0 candidate after
omega#208 admitted OAW-012. Upstream parity is diagnostic context only.

## Pass rule

All acceptance criteria in issue #16 must be true for the exact candidate.
If any step is blocked (missing DMG, missing secrets, missing owner sign-off),
the proof record must mark that step `blocked` and the GitHub issue must stay
open.

A candidate that fails a brand check fails this proof. Scanning the compiled
executable is not sufficient. `script/bundle-omega-rc` scans the packaged
application for three identity literals only, so no packaging gate reads
`Contents/Info.plist`, and no string scan of any kind can see a logo. Both
gaps produced real misses on `0.2.0-rc10`, which are recorded in
`docs/omega/2026-07-25-omega-rc10-installed-observation-proof.md` in the
openagents repository.
