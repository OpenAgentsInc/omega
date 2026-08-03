# Omega RC release procedure

This document is the owned OpenAgents procedure for producing and publishing
`v0.2.0-rc31`. It replaces inherited Zed release automation for this candidate.

`v0.2.0-rc1` was withdrawn after publication and remains an immutable historical
candidate. All RC tags and assets are immutable; this build, its proof, and its
publication use the rc31 names below.

## Goal

Produce a signed macOS arm64 candidate:

- Artifact: `Omega-v0.2.0-rc31-macos-arm64.dmg`
- Volume name: `Omega RC`
- Bundle identifier: `com.openagents.omega.rc`
- Team ID: `HQWSG26L43`
- Signing identity: `Developer ID Application: OpenAgents, Inc. (HQWSG26L43)`

Publish only to `OpenAgentsInc/omega` as a **prerelease**. Never mark the
candidate as GitHub `latest`.

## Owner secrets

Keep these off the repository and out of commit messages:

| Secret | Purpose |
| --- | --- |
| OpenAgents Developer ID certificate / keychain access | `codesign` |
| `APPLE_NOTARIZATION_KEY` | App Store Connect API key as inline PEM contents or a readable PEM file path (for example, `ASC_API_PRIVATE_KEY_PATH`) |
| `APPLE_NOTARIZATION_KEY_ID` | Key id |
| `APPLE_NOTARIZATION_ISSUER_ID` | Issuer id |
| GitHub release token with `contents: write` on `OpenAgentsInc/omega` | Prerelease upload |

Notarization is optional for local contract validation. A release record must
not claim notarization or stapling unless those steps actually completed.

## Local command

From a clean checkout on the release commit:

```sh
script/bundle-omega-rc --dry-run
```

Dry-run checks:

- clean git tree
- OpenAgents signing identity availability
- release-record JSON schema and required digests

Full package attempt:

```sh
script/bundle-omega-rc
```

When local signing secrets expose the App Store Connect key as
`ASC_API_PRIVATE_KEY_PATH`, pass that path directly without reading or printing
the key:

```sh
APPLE_NOTARIZATION_KEY="$ASC_API_PRIVATE_KEY_PATH" script/bundle-omega-rc
```

Validate both supported key input forms without contacting Apple:

```sh
script/bundle-omega-rc --self-test-notarization-key
```

The command temporarily sets the preview channel and package version `0.2.0`,
restores those files on exit, refuses a dirty tree, fails clearly when the
OpenAgents signing identity is missing, does not attach Zed commercial terms,
copies `LICENSE-GPL` / `LICENSE-APACHE` into the disk image, and writes:

```text
target/omega-rc/Omega-v0.2.0-rc31-macos-arm64.dmg
target/omega-rc/omega-v0.2.0-rc31-macos-arm64.release.json
```

The release record keeps `source.dirty` false only when the sole worktree
change made during packaging is the bundler's exact, unstaged
`crates/omega/RELEASE_CHANNEL` rewrite. That deterministic transformation is
listed in `source.build_transformations` with its committed and generated
digests, and the original porcelain entry remains in
`source.worktree_entries`. Any different channel contents, staged change,
unrelated tracked change, or unexpected untracked file aborts release-record
generation. Exercise those refusal cases without building an application:

```sh
python3 script/omega_rc_source_state.py --self-test
```

## Release record digests

The JSON release record binds at least:

- source commit
- upstream commit when available
- `Cargo.lock` SHA-256
- icon-family manifest and pinned icon digests
- package SHA-256 when the DMG exists
- signing identity and team id
- notarization attempted / stapled / status
- `publication.prerelease = true` and `publication.latest = false`
- pinned `omega-effectd` release, archive, manifest, source, wrapper, bundle,
  source Node, and signed packaged Node digests

Validate schema after any hand edit:

```sh
script/bundle-omega-rc --dry-run
cargo test -p app_identity release_record -- --nocapture
```

## Publication checklist

1. Confirm the release record `source.commit` matches the intended git SHA.
2. Confirm `digests.package_sha256` matches `shasum -a 256` of the DMG.
3. Confirm notarization status honestly (`not_attempted`, `submit_failed`, or
   `submitted_and_stapled`).
4. Create a GitHub **prerelease** tagged `v0.2.0-rc31` on
   `OpenAgentsInc/omega`.
5. Upload the DMG and release-record JSON.
6. Do **not** set the release as latest.
7. Do **not** publish through Zed release workflows or Zed artifact names.

## Owner blockers

The release record derives these from the work that actually completed. A
dry-run retains the Developer ID signing blocker, and an unstapled candidate
retains the notarization blocker. A signed, stapled candidate must not carry
either stale blocker.

Human confirmation of the GitHub prerelease upload always remains external to
the bundler and therefore remains in `owner_blockers` until publication is
proved separately.

## Legal packaging

macOS and Windows packaging must not present Zed commercial Terms of Service.
Keep GPL, Apache, and generated third-party notices. Windows Inno Setup uses
`script/terms/open-source-notices.rtf`.
