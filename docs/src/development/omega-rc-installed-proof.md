# Omega RC installed proof

This is the owned proof protocol for issue
[#16](https://github.com/OpenAgentsInc/omega/issues/16) / OMEGA-BRAND-06.

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
