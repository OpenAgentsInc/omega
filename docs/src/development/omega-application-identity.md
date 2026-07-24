# Omega application identity

Omega uses application identities that do not overlap with an installed copy of
Zed. The release channel selects the data roots, credential namespace, bundle
identifier, and URL scheme.

| Channel        | Display/data name | Storage slug    | Bundle identifier              | Credential namespace                       | URL scheme      |
| -------------- | ----------------- | --------------- | ------------------------------ | ------------------------------------------ | --------------- |
| Development    | Omega Dev         | `omega-dev`     | `com.openagents.omega.dev`     | `com.openagents.omega.credentials.dev`     | `omega-dev`     |
| Nightly        | Omega Nightly     | `omega-nightly` | `com.openagents.omega.nightly` | `com.openagents.omega.credentials.nightly` | `omega-nightly` |
| RC (`preview`) | Omega RC          | `omega-rc`      | `com.openagents.omega.rc`      | `com.openagents.omega.credentials.rc`      | `omega-rc`      |
| Stable         | Omega             | `omega`         | `com.openagents.omega`         | `com.openagents.omega.credentials`         | `omega`         |

On macOS, the RC data root is
`~/Library/Application Support/Omega RC`; config, state, cache, and log roots use
the `omega-rc` slug. Linux and FreeBSD use the slug under their XDG roots.
Windows uses the display/data name under the platform data roots.

Run the development binary with:

```sh
cargo run --profile release-fast
```

The resulting application executable is named `omega`. Internal crate names,
`ZED_*` build variables, project `.zed` folders, remote-server folders, and
legacy `zed://` parsing remain compatibility surfaces. Omega does not register
itself as the handler for `zed://`.

Normal development credentials can still use the channel-local development
credentials file. Identity custody code must use the secure-only provider seam,
which always selects the system keychain and applies the channel credential
namespace.

## Review identity onboarding fixtures

Development builds accept `OMEGA_IDENTITY_FIXTURE` so every public onboarding
state can be reviewed without writing a key. Supported values are
`reset-failed`, `locked`, `relaunch-required`, `conflict`, `lost`,
`incomplete`, `absent`, `creating`, and `ready`.

For example:

```sh
OMEGA_IDENTITY_FIXTURE=ready cargo run --profile release-fast
```

These states are presentation fixtures and never write secure custody. The
masked import preview contains no private key material.

## Native identity custody

`omega_identity::IdentityService` is the secure native boundary for creating,
importing, opening, signing with, inspecting, and resetting an Omega identity.
It stores one 32-byte Nostr secret in the operating system credential provider
under the current release channel's `KeyringLocator`. There is no environment,
plaintext-file, app-data, or auto-generation fallback.

Creation and import are explicit transactions. Omega serializes each mutation
with a process-global mutex and a channel-scoped cross-process operating-system
lock, writes the credential, reads it through a fresh credential-provider
entry, derives its public key, and compares that key with the expected identity.
Only after that comparison succeeds does it atomically write the public
manifest and completion record. A mismatch, missing read-back, malformed public
record, locked provider, or lost credential leaves custody non-ready and denies
signing.

The public API accepts a zeroizing import wrapper but has no secret-return
operation. Signing accepts only a validated `AdmittedSigningRequest` bound to
the active identity and returns the signed public Nostr event. Reset requires
the expected identity, verifies that credential deletion is visible through a
fresh read, removes only the public identity documents, and requires a
relaunch.

System credential and cross-process lock operations are synchronous. GPUI
callers must run them on the background executor and propagate
`CustodyError` to the onboarding UI instead of blocking the foreground thread.

## Native identity provenance

Omega's native identity contract adapts selected patterns from
[OpenAgentsInc/buzz](https://github.com/OpenAgentsInc/buzz) Desktop v0.4.23 at
commit `acfbb1bb6af54cb29cb152496ff43b8285dcb8cf`. Buzz is licensed under
Apache-2.0. Omega changes the patterns to use its release-channel credential
namespaces and the Nostr-only `openagents.omega.nostr_only.v1` profile.

| Reviewed Buzz source                         | SHA-256                                                            | Adapted boundary                                       |
| -------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------ |
| `desktop/src-tauri/src/secret_store.rs`      | `2f1d93a3427bd2852001c81a0ad88afb6a614dec8eba78f23bcd56d630cd1ce8` | Keyring probing, read-back checks, serialized mutation |
| `desktop/src-tauri/src/app_state_keyring.rs` | `f4f28872a57ea532dcd7d8f8f4a589b736d86b8bd25a81850c3e782defbd2aa7` | Service scoping, replaced by `app_identity` channels   |
| `desktop/src-tauri/src/app_state.rs`         | `1ee8b09732b1e39afcaf3450ff674880d5d17e434f8900429239a1f05ca33063` | Public recovery states and read-back ordering          |
| `desktop/src-tauri/src/commands/identity.rs` | `8ea33e58265b9a62a16dd163cdf37c06d9ad574418b1f02fd3ac295b8b2b290c` | Public identity results and admitted signing           |
| `desktop/src-tauri/src/reset.rs`             | `9a5b5ac615cf7501c74952e03eac774f937c1458ff163cb6c5a02365655aad64` | Restart-safe reset transaction pattern                 |

The contract uses exact reviewed native packages:

| Package             | Version  | Checksum                                                           | License           |
| ------------------- | -------- | ------------------------------------------------------------------ | ----------------- |
| `nostr`             | `0.44.4` | `98cf5d15d70d1f8f4059e5f79923ac15891eb691d2843d01191e0585fb064d70` | MIT               |
| `keyring`           | `3.6.3`  | `eebcc3aff044e5944a8fbaf69eb277d11986064cba30c468730e8b9909fb551c` | MIT OR Apache-2.0 |
| `atomic-write-file` | `0.3.0`  | `84790c55b5704b0d35130bf16a4ce22a8e70eb0ea773522557524d9a4852663d` | BSD-3-Clause      |

The public contract vector is
`crates/omega_identity/fixtures/omega_nostr_identity_v1.json`, with SHA-256
`1e25670b072cdd500bdd65e1c0215b62068cfd26a98284e23a90c781fef0bba6`.
It freezes public-key, npub, public fingerprint, manifest, and admitted-signing
behavior without containing a private key.

Omega does not adopt Buzz's startup key generation, `BUZZ_PRIVATE_KEY`
override, plaintext `identity.key` fallback, renderer-visible `get_nsec`
command, durable use of ephemeral recovery keys, or Spark/wallet profile
fields. `atomic-write-file` is restricted to public manifests and completion
records.

## Application icon family

Package icons are derived from the pinned OpenAgents Desktop icon under
`crates/zed/resources/icon_family/`. Regenerate with
`script/generate-omega-icon-family`.

Channel badges are deferred. Stable, RC, Nightly, and Dev currently reuse the
same unbadged OpenAgents icon until OpenAgents approves distinct channel
artwork.

Digest and dimension checks live in the `app_identity` icon-family tests and
`crates/zed/resources/icon_family/manifest.json`.
