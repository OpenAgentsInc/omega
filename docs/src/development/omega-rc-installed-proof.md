# Omega RC installed proof

This is the owned proof protocol for identity issue
[#8](https://github.com/OpenAgentsInc/omega/issues/8) / OMEGA-OID-09 and brand
issue [#16](https://github.com/OpenAgentsInc/omega/issues/16) /
OMEGA-BRAND-06, and Full Auto issue
[#26](https://github.com/OpenAgentsInc/omega/issues/26) / OMEGA-FA-07.

Unit tests and source scans are **not** sufficient. The exact signed candidate
must be installed beside Zed and exercised.

## Prerequisites

1. A release record from `script/bundle-omega-rc` for
   `Omega-v0.2.0-rc2-macos-arm64.dmg`.
2. Matching package digest in that record.
3. OpenAgents signing / notarization evidence when distribution requires it.
4. A clean user profile that already has Zed installed for the side-by-side
   check.

## Command

After `script/bundle-omega-rc` produces the signed and notarized DMG, generate
the admitted identity-candidate record, install that exact candidate, collect
the candidate-bound manual evidence described below, then:

```sh
script/prove-omega-rc-install \
  --release-record target/omega-rc/omega-v0.2.0-rc2-macos-arm64.release.json \
  --artifact target/omega-rc/Omega-v0.2.0-rc2-macos-arm64.dmg \
  --app /Applications/Omega.app \
  --identity-evidence target/omega-identity-evidence/candidate-evidence.json \
  --identity-matrix target/omega-identity-proof/matrix-evidence.json \
  --installed-tripwires target/omega-identity-proof/installed-secret-tripwires.json \
  --evidence-root target/omega-identity-proof \
  --full-auto-evidence target/omega-full-auto-evidence/candidate-evidence.json \
  --manual-evidence target/omega-rc-proof/manual-evidence.json \
  --network-evidence target/omega-rc-proof/network-evidence.json \
  --lifecycle-evidence target/omega-rc-proof/lifecycle/manifest.json
```

Harness validation without an installed candidate:

```sh
script/prove-omega-rc-install --harness-check
```

Bind the exact package and release record to the frozen identity contract,
public vector, reviewed Buzz source, and native package versions:

```sh
script/generate-omega-identity-candidate-evidence \
  --release-record target/omega-rc/omega-v0.2.0-rc2-macos-arm64.release.json \
  --artifact target/omega-rc/Omega-v0.2.0-rc2-macos-arm64.dmg \
  --cargo-lock-snapshot target/omega-rc/Cargo.lock.omega-v0.2.0-rc2 \
  --identity-matrix target/omega-identity-proof/matrix-evidence.json \
  --installed-tripwires target/omega-identity-proof/installed-secret-tripwires.json \
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
refusal, unavailable or corrupt keychain data, corrupt recovery artifacts,
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
digest mismatch, and substitution of another receipt. Other gates retain
nonempty candidate-bound results where a file receipt is not applicable. One
generic evidence key cannot pass a multi-requirement gate.

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

The admitted Omega Full Auto AssuranceSpec is revision 5. Its design-admission
receipt does not prove an installed candidate. Collect a separate packet with
schema `openagents.omega.full-auto-observations.v1`, then bind it to the exact
identity candidate, Omega commit, DMG, release record, embedded
`omega-effectd`, admitted AssuranceSpec proposal/document/receipt, and
ProductSpec:

```sh
script/generate-omega-full-auto-candidate-evidence \
  --release-record target/omega-rc/omega-v0.2.0-rc2-macos-arm64.release.json \
  --artifact target/omega-rc/Omega-v0.2.0-rc2-macos-arm64.dmg \
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
  --release-record "$PWD/target/omega-rc/omega-v0.2.0-rc2-macos-arm64.release.json" \
  --artifact "$PWD/target/omega-rc/Omega-v0.2.0-rc2-macos-arm64.dmg" \
  --candidate-evidence "$PWD/target/omega-identity-evidence/candidate-evidence.json" \
  --output "$PWD/target/omega-identity-proof/matrix-evidence.json"
```

The runner accepts only `/Applications/Omega.app` and the packaged driver
whose digest is jointly bound by the release record and candidate evidence. It
also requires the installed driver to pass strict code-signature verification.
Its temporary roots are internally created with the proof-only prefix, and the
driver itself fixes the Keychain locator to
`com.openagents.omega.identity-proof.v1` / `disposable-proof-only`.

The matrix covers creation and read-back, process restart, signing,
same-receipt double creation, distinct-receipt rejection, concurrent creation
and process start, forged and stale requests, reset and relaunch, and every
exposed crash checkpoint. It also records explicit deterministic, no-Keychain
simulations for conflict/lost/locked custody, unsafe public stores,
unavailable/corrupt secure storage, malformed or unadmitted signing requests,
conflicting recovery selection, late completion, and signer crash. Simulation
receipts are labeled as such and do not claim live Keychain execution. It
resets the disposable entry between live cases and
fails if final cleanup cannot be proved. Evidence contains candidate and
component digests, case states, and hashes of public driver outcomes; it does
not retain identities, secrets, command output, temporary paths, or error
details from the driver.

Run the deterministic harness checks without touching Keychain:

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
must bind the same DMG and release record. Validate the lifecycle helper with:

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

## Pass rule

All acceptance criteria in issue #16 must be true for the exact candidate.
If any step is blocked (missing DMG, missing secrets, missing owner sign-off),
the proof record must mark that step `blocked` and the GitHub issue must stay
open.
