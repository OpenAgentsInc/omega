# Omega RC installed proof

This is the owned proof protocol for identity issue
[#8](https://github.com/OpenAgentsInc/omega/issues/8) / OMEGA-OID-09 and brand
issue [#16](https://github.com/OpenAgentsInc/omega/issues/16) /
OMEGA-BRAND-06.

Unit tests and source scans are **not** sufficient. The exact signed candidate
must be installed beside Zed and exercised.

## Prerequisites

1. A release record from `script/bundle-omega-rc` for
   `Omega-v0.2.0-rc1-macos-arm64.dmg`.
2. Matching package digest in that record.
3. OpenAgents signing / notarization evidence when distribution requires it.
4. A clean user profile that already has Zed installed for the side-by-side
   check.

## Command

After `script/bundle-omega-rc` produces `target/omega-rc/release-record.json` and
the signed DMG, install the candidate, then:

```sh
script/prove-omega-rc-install \
  --release-record target/omega-rc/release-record.json \
  --app /Applications/Omega.app
```

Harness validation without an installed candidate:

```sh
script/prove-omega-rc-install --harness-check
```

Bind the exact package and release record to the frozen identity contract,
public vector, reviewed Buzz source, and native package versions:

```sh
script/generate-omega-identity-candidate-evidence \
  --release-record target/omega-rc/omega-v0.2.0-rc1-macos-arm64.release.json \
  --artifact target/omega-rc/Omega-v0.2.0-rc1-macos-arm64.dmg \
  --cargo-lock-snapshot target/omega-rc/Cargo.lock.omega-v0.2.0-rc1
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
`required_evidence` strings in the pending record, and every value must be a
nonempty candidate-bound reference or recorded result. One generic evidence
reference cannot pass a multi-requirement gate.

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
   `crates/app_identity/fixtures/endpoint_allowlist.json`.
9. Inspect data, cache, log, browser, update, and credential roots for Omega
   vs Zed separation.
10. Confirm disabled service / update behavior is honest.
11. Remove Omega and confirm Zed data did not change.
12. Record owner observation and independent verification in the proof record.

## Pass rule

All acceptance criteria in issue #16 must be true for the exact candidate.
If any step is blocked (missing DMG, missing secrets, missing owner sign-off),
the proof record must mark that step `blocked` and the GitHub issue must stay
open.
