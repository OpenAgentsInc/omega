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

The public API accepts zeroizing import and recovery-password wrappers but has
no secret-return operation. Signing accepts only a validated
`AdmittedSigningRequest` bound to the active identity and returns the signed
public Nostr event.

Create and import use `identity.transaction.json` as a public-safe journal keyed
by the action receipt. A repeated action resumes the exact transaction; it
never invokes the generator when the journaled secure value already exists.
The journal records only operation metadata and, after verified secure storage,
the expected public identity. A returned failure rolls back to `absent` only
after credential deletion is read back. If rollback cannot be proved, the
journal remains and replacement creation stays blocked as `incomplete`.
`inspect_details` exposes the pending operation, original receipt, and optional
expected public identity without exposing import-candidate or secret material.
`resume_incomplete_create` consumes the private journal directly. It can
generate only when the journal has not established an expected identity; a
known identity whose credential is missing remains blocked instead of rotating.

Detailed inspection also classifies conflicts without treating every conflict
as an owner-choice screen. An ambiguous operating-system credential result has
no readable candidate key. A manifest/custody mismatch exposes both derived
public identities. A pending-transaction mismatch exposes only the conflicting
public identities from the journal and custody records. Secrets and selected
paths do not enter the inspection result.

A manifest/custody mismatch can be repaired only through
`resolve_conflict`. The caller must prepare and explicitly select recovery
material for one of the public identities reported by the current conflict.
Omega journals that selected public identity, candidate reference, owner-action
receipt, and the inspected conflict identities before changing custody. It
deletes a differing stored key only when its freshly derived public identity is
one of those journaled conflict identities, then verifies deletion before
committing the selected key. A crash or failed delete leaves the journal for an
exact retry. Normal `adopt` remains unable to replace conflict custody, and an
ambiguous operating-system credential cannot use this path because Omega cannot
inspect the competing keys.

Reset is marker-first and restart-safe. The initial authorized request writes
`identity.reset.json` and returns `relaunch-required` without deleting custody.
Startup or an explicit resume then verifies the expected identity, deletes and
fresh-reads custody, removes completion, manifest, and transaction records, and
marks reset complete last. Failure keeps a `reset-failed` marker that blocks
signing and can be retried. A relaunch acknowledgement clears the final marker
only after all identity state is proven absent. The application boot router
must call `inspect_for_process_start` exactly once per process. Under one custody
mutation lock, that inspection acknowledges a reset only when its marker was
already complete at method entry, proving that completion happened in a prior
process. A pending or failed marker may be resumed during the call, but its
result remains `relaunch-required` or `reset-failed` and the marker is preserved
for a later process to acknowledge. This process-level inspection must not be
called from individual onboarding view constructors.

Ready-state onboarding UI exposes Protect, not Reset. Use the operator CLI for
an authorized wipe of channel custody:

```sh
cargo run -p omega_identity --bin omega-identity -- --channel rc status
cargo run -p omega_identity --bin omega-identity -- --channel rc wipe --yes
```

`status`, `reset --yes`, `resume`, `acknowledge`, and `wipe --yes` cover the
typed marker-first path. Pass `--data-root` only when targeting a non-standard
channel data directory.
### Encrypted recovery artifacts

Version 1 recovery artifacts are standard NIP-49 `ncryptsec` tokens produced by
`nostr 0.44.4` with its `nip49` feature. The format uses scrypt and
XChaCha20-Poly1305; Omega exports with work factor `log_n = 16` and imports only
`16..=18` so an untrusted artifact cannot request an unbounded KDF allocation.
The file contains one encrypted token and a newline—no manifest, `npub`, path,
or Omega metadata.

Discovery receives an explicitly selected path and reads metadata only. An
authorized preparation call performs the bounded read and decryption, derives
the public identity for preview and conflict resolution, and returns an opaque
prepared value. Adoption consumes that value through the same journaled custody
transaction as advanced masked import. Prepared values cannot be adopted
directly: the caller must name a candidate and receive an opaque
`SelectedRecovery`. Distinct public identities therefore require an explicit
owner selection; duplicate artifacts for the same public key collapse to one
candidate.

Exports write and sync a protected same-directory temporary file, then publish
it atomically with a no-replace hard link. Unix exports use mode `0600`; Unix
imports require a current-user, single-link, regular non-symlink file with no
group or world permissions. Artifacts are limited to 4 KiB and paths are
redacted from debug output. Windows opens selected inputs without following
reparse points and uses the same no-replace publication, but parity with Unix
ownership and ACL verification requires a future Windows security-descriptor
review. Expensive NIP-49 KDF work is serialized once per process and must run
through GPUI's background executor.

After an encrypted export has been published successfully, Omega atomically
writes `identity.recovery-protection.json`. This public-safe record is bound to
the public identity and contains the encrypted artifact's SHA-256 digest and
byte length, but not its path or password. Detailed inspection reports
`not-applicable`, `needed`, or `protected` from that durable record. Committing
a different identity removes a mismatched record, and verified reset removes
the record before the reset marker becomes complete. A malformed record is
removed and reported as `needed`; recovery metadata is advisory and cannot
block otherwise verified ready custody. Genuine storage I/O failures still
propagate instead of being mistaken for missing protection.

Omega wraps password input in zeroizing storage before validating its length,
and zeroizes its import and long-lived secret wrappers. The pinned
NIP-49 implementation creates internal normalized-password, KDF, and decrypted
temporary buffers that do not all implement `Zeroize`; replacing or patching
that upstream implementation is required if the assurance target expands to
provable erasure of every library-internal copy.

Custody resolution follows:

```text
reset-failed
  > keychain-locked
  > relaunch-required
  > identity-conflict
  > identity-lost
  > incomplete transaction
  > identity absent
  > ready
```

System credential and cross-process lock operations are synchronous. GPUI
callers must run them on the background executor and propagate
`CustodyError` to the onboarding UI instead of blocking the foreground thread.

## Onboarding integration

The Omega onboarding identity section renders `IdentityInspection`, not fixture
flags. Its durable view contains only public custody state, public identity,
pending operation/receipt facts, typed conflict details, and recovery-protection
status. Create, inspect, import, recovery KDF, export, and reset operations run
on GPUI's background executor. The view holds the foreground waiter task and
accepts a completion only when both its monotonic generation and operation
phase still match.

Recovery passwords and advanced Nostr imports use a dedicated `SecureInput`
instead of the editor-backed form field. It preallocates a bounded
`Zeroizing<String>`, always paints bullets, has no undo history or reveal
control, consumes copy/cut shortcuts, and transfers ownership before starting
background work. Cancel, deactivation, workspace deactivation, item removal,
success, and error clear the secure fields and discard opaque prepared recovery
candidates. When the platform requests surrounding text for IME, the input
handler returns only a length-aligned bullet mask rather than the secret.

The onboarding item cannot be split, and its serializer persists only the page
record. It never serializes a pending secret, password, recovery candidate,
selected path, or identity task. Theme selection and registry-agent setup are
the existing implementations and follow the identity section in the same
screen.

Optional display name and avatar metadata use the identity-bound
`omega.identity-profile.v1.<identity-ref>` local record. The avatar is copied
into Omega's local data directory and represented by an opaque
`local-avatar:` token. This presentation record has no signing, relay, event
kind, or publication fields and is never a Nostr kind 0 profile.

### First-launch routing

Every initial editor surface waits on one process-global identity startup
coordinator. The coordinator owns one shared background call to
`inspect_for_process_start`; nested restore, open-request, CLI, file, remote,
new-window, and new-file paths reuse that result and cannot acknowledge a reset
marker twice in one process. The operating-system single-instance decision
happens before this coordinator is installed.

Ready custody releases callers immediately. Every other custody state, and an
inspection error, opens at most one identity onboarding view and keeps each
owned launch intent suspended in its original task. The intent, including a CLI
response sink and wait flag, resumes through its existing dispatch path only
after the user finishes onboarding. Closing the view does not release callers;
it permits a later request to reopen the singleton view.

Finish rechecks the live section's durable Ready inspection. First Run writes
the identity-bound `omega_identity_onboarding_completion_v1` local completion
record, removes the dedicated bootstrap window, and then releases all waiters.
A bootstrap-window open failure is a shared terminal error, so current and
future intents fail instead of hanging or bypassing identity. Merely opening
onboarding writes no completion flag. The completion record is presentation
history rather than identity authority: every new Omega process still performs
the custody/public-manifest startup inspection.

Editor Onboarding is a separately replayable mode available from the Help menu,
the Welcome page, and the `omega::OpenEditorOnboarding` action (also
`omega::OpenOnboarding`). Prefer the command palette
(`cmd-shift-p` on macOS / `ctrl-shift-p` on Windows and Linux) over the file
picker (`cmd-p` / `ctrl-p`). In debug builds, `dev::ResetOnboarding` clears the
identity and editor completion records so reopen can be retested without wiping
the whole profile. It renders the
same Theme and Agent Setup implementations as First Run, with a compact
identity status that retains custody repair, conflict resolution, recovery, and
recovery-protection actions. Its Finish action also requires durable Ready
custody, but writes the independent
`omega_editor_onboarding_completion_v1` record and remains restorable in the
workspace. Neither completion record gates the other journey or suppresses
identity inspection and recovery. Completion records also do not block Help →
Editor Onboarding or Welcome → Return to Onboarding; those actions reopen the
Editor Setup view directly.

Agent Setup renders its four featured choices immediately from checked-in
display metadata: Claude, Codex, GitHub Copilot, and Cursor. The asynchronous
ACP registry remains the authority for install metadata; onboarding observes
that store and replaces fallback names and icons when registry data arrives.
An empty, slow, or temporarily unavailable registry therefore cannot leave the
first-run section blank, and the section does not import or copy agent
credentials.

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
records, transaction/reset journals, and recovery-protection records.

## Application icon family

Package icons are derived from the pinned OpenAgents Desktop icon under
`crates/zed/resources/icon_family/`. Regenerate with
`script/generate-omega-icon-family`.

Channel badges are deferred. Stable, RC, Nightly, and Dev currently reuse the
same unbadged OpenAgents icon until OpenAgents approves distinct channel
artwork.

Digest and dimension checks live in the `app_identity` icon-family tests and
`crates/zed/resources/icon_family/manifest.json`.

## Zed production service isolation

By default Omega does not contact Zed production hosts.

- `server_url` defaults to `https://services.openagents.invalid`
- Telemetry diagnostics/metrics and auto-update are off
- Local and registry ACP agents are on; Omega defaults `codex-acp` to the
  public ACP registry so the agent panel and Full Auto can use the existing
  Codex ACP authentication path
- Hosted Zed AI remains unavailable, edit predictions use provider `none`, and
  extension auto-install is off
- Account auth, extension registry fetches, remote-server downloads, and the
  Zed cloud model provider stay gated unless `OMEGA_ALLOW_ZED_SERVICES=1`
- The reviewed public ACP registry endpoint is independent of Zed production
  and is listed explicitly in the endpoint allowlist
- Reviewed hosts live in
  `crates/app_identity/fixtures/endpoint_allowlist.json`
- Capture journey helper: `script/omega-network-capture-journey`

The capture helper binds its root PID to the canonical installed Omega
executable, executable digest, strict Developer ID authority/team result, and
stable process-start identity at every sample. Its validator rehashes and
re-verifies the installed executable and rejects PID reuse or path drift before
it considers destination classification. Run the helper's `self-test` to prove
the path, PID, signing-team, and collector-digest tamper cases fail.

The endpoint allowlist is an installed-release proof contract, not an in-app
firewall. Runtime requests do not consult the JSON file. Candidate acceptance
must capture process network destinations and reject an unreviewed host or any
host in `normal_start_forbidden_hosts`.

### Native Codex ACP first install

The default `codex-acp` entry resolves through these authorities:

1. `AgentRegistryStore` fetches the ACP index and icons from
   `cdn.agentclientprotocol.com/registry/v1/`. The current registry entry pins
   `@agentclientprotocol/codex-acp@1.1.7`.
2. Omega uses a configured Node/npm pair or a compatible pair on `PATH`. If
   neither exists, `NodeRuntime` downloads its managed Node distribution,
   including npm, from `nodejs.org/dist/`.
3. `LocalRegistryNpxAgent` runs npm with a version ceiling of `1.1.7`. With
   default npm configuration, package metadata and tarballs for Codex ACP,
   `@openai/codex`, the current-platform Codex binary package, and transitive
   dependencies come from `registry.npmjs.org`.

The `omega-effectd` component contains only its private Node executable; it
does not contain npm and is not the Agent panel's `NodeRuntime`. A machine
without a configured or system Node/npm pair therefore needs both
`nodejs.org` and `registry.npmjs.org` for the first Npx ACP start. A user npm
registry, proxy, or custom Node path can replace those default destinations and
must be reviewed separately in installed proof. Package metadata for Codex ACP
1.1.7 currently declares no install scripts, and the resolved dependency graph
uses npm-registry tarballs rather than a secondary binary download host.
