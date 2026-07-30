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

Every release channel uses files below its channel-specific application data
root for runtime credentials. The Nostr signer secret is
`identity/identity.secret`; provider and hosted credentials are
`credentials/credentials.json`. Writes are atomic and use owner-only directory
and file modes on Unix. The files are not encrypted at rest. See
[Runtime credential storage](../../omega/runtime-credential-storage.md) and
the [Nostr authentication contract](../../omega/nostr-authentication-contract.md).

## Native identity custody

`omega_identity::IdentityService` is the Rust boundary for creating,
importing, opening, signing with, inspecting, and resetting an Omega identity.
The migrated root account stores its 32-byte Nostr secret in the release
channel's owner-only `identity/identity.secret` file. Added local accounts use
the same file format in deterministic per-account directories under
`identity/accounts/`. `KeyringLocator` is a stable version-one
logical locator name, not a macOS Keychain implementation. On a clean profile,
the startup coordinator creates this identity silently in the background; it
does not block the front door on an identity ceremony.

This implementation is file-backed only. AUTH-01 does not enable or probe the
macOS Keychain, Secure Enclave, Windows credential vault, Linux secret service,
Android keystore, or another native key vault. Native secure-storage work is
deferred beyond this wave.

Creation and import are explicit transactions. Omega serializes each mutation
with a process-global mutex and a channel-scoped cross-process operating-system
lock, writes the credential, reads it through a fresh credential-provider
entry backed by the local identity file, derives its public key, and compares
that key with the expected identity.
Only after that comparison succeeds does it atomically write the public
manifest and completion record. A mismatch, missing read-back, malformed public
record, inaccessible store, or lost credential leaves custody non-ready and denies
signing.

The public API accepts zeroizing import and recovery-password wrappers but has
no secret-return operation. Signing accepts only a validated
`AdmittedSigningRequest` bound to the active identity and returns the signed
public Nostr event.

Create and import use `identity.transaction.json` as a public-safe journal keyed
by the action receipt. A repeated action resumes the exact transaction; it
never invokes the generator when the journaled local-file value already exists.
The journal records only operation metadata and, after verified local-file
write and read-back, the expected public identity. A returned failure rolls
back to `absent` only after credential deletion is read back. If rollback
cannot be proved, the
journal remains and replacement creation stays blocked as `incomplete`.
`inspect_details` exposes the pending operation, original receipt, and optional
expected public identity without exposing import-candidate or secret material.
`resume_incomplete_create` consumes the private journal directly. It can
generate only when the journal has not established an expected identity; a
known identity whose credential is missing remains blocked instead of rotating.

Detailed inspection also classifies conflicts without treating every conflict
as an owner-choice screen. An ambiguous local identity-file result has no
readable candidate key. A manifest/custody mismatch exposes both derived
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
ambiguous local identity-file result cannot use this path because Omega cannot
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

A lost identity is the narrow exception to the relaunch boundary. In that state
the public identity is known but the active store has already proven that no
signing secret is available. The onboarding **Reset identity** action therefore
completes and acknowledges cleanup in the current process, returning directly
to local identity creation. This is especially relevant when a public manifest
outlives a deleted, unreadable, or otherwise unavailable secret file.

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
  > secret-store-locked
  > relaunch-required
  > identity-conflict
  > identity-lost
  > incomplete transaction
  > identity absent
  > ready
```

Secret-file and cross-process lock operations are synchronous. GPUI
callers must run them on the background executor and propagate
`CustodyError` to the onboarding UI instead of blocking the foreground thread.

## Onboarding integration

The Omega onboarding identity section renders real `IdentityInspection` and
`IdentityAccountRecord` values. It has no fixture environment-variable runtime
switch. Its durable view contains only public custody state, account activation
state, public identity and fingerprint, pending operation/receipt facts, typed
conflict details, and recovery-protection status. Create, inspect, import,
recovery KDF, export, reset, and account inspection operations run on GPUI's
background executor. The view holds the foreground waiter task and accepts a
completion only when both its monotonic generation and operation phase still
match.

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

### Startup provisioning and explicit setup

Every initial editor surface shares one process-global identity startup task.
The task calls `provision_for_process_start` in the background; nested restore,
open-request, CLI, file, remote, new-window, and new-file paths reuse its
result. A clean profile creates a local identity, an unadopted ready secret is
adopted, and a ready identity is reused without rotating its key. No result
blocks the editor front door and no onboarding `Finish` action releases a
startup waiter.

Startup also creates or migrates the public-safe
`identity/identity.account.json` record. A fresh generated key is
`CandidateLocal`. A ready identity with no account record is
`CandidateExisting`; migration preserves the exact key and signed history.
The account control shows the candidate state and short public fingerprint
instead of calling ready custody an active account.

The first public post, community join, device grant, hosted-account link, or
agent attestation passes through the durable identity-action gate. A candidate
causes one exact intent to be atomically held in
`identity/identity.action-intent.json` and moves to `Activating`. The intent
binds account generation, identity, destination, authorization, payload digest,
and expiry. Activation completion and intent consumption are separate:
consumption revalidates every binding and succeeds once. Cancellation restores
the original candidate state, deletes the held intent, and resumes nothing.
The title bar opens the account dashboard. A candidate or recovery-needed row
offers **Complete setup**, which opens the existing activation and recovery
ceremony; gated action notices continue to lead to that ceremony.

### Account dashboard and multiple local identities

AUTH-03 stores the durable registry at `identity/accounts/index.json`. The
registry preserves the legacy root account, added local candidates, their
public identities and fingerprints, optional local profiles, signer summaries,
recovery and retirement states, and the active account reference plus a
monotonic generation. New identities are created once through a journaled
staging-to-partition rename and remain `CandidateLocal` with NIP-49 protection
needed until explicit setup completes.

The GPUI account dashboard is the always-reachable desktop home for add local,
complete setup, switch, lock or unlock, sign out, forget this device, and the
future retirement entry point. Switch verifies the destination custody before
a crash-resumable registry commit. The selected account generation advances on
selection and lifecycle changes, and signing revalidates the account reference,
identity, lifecycle, and generation immediately before using the partitioned
`identity.secret`. A stale token or locked, signed-out, or forgotten selection
is refused.

Lock leaves the active selection in place but makes its signer unavailable.
Unlock revalidates custody. Sign out clears the active selection without
deleting the account. Forget this device is a journaled local purge and states
that relay or peer events and an external NIP-49 file remain. Retirement is a
separate signed-policy operation and stays unavailable until that policy
ships.

Draft prompts and audience/community room state are partitioned by public
identity. Their owning stores delete and verify those partitions during
forget-device. Decrypted cache, wallet state, relay state, signer sessions, and
device grants do not yet have owner purge hooks; they remain visible,
retryable partial failures rather than being treated as deleted.

### NIP-46 remote signer enrollment

Remote signer enrollment in AUTH-04 adds a Rust NIP-46 path to the same desktop
account dashboard.
**Use a signer on another device** opens it directly. A person may paste a
`bunker://` URI or review a bounded permission profile before Omega creates a
temporary `nostrconnect://` link. The generated link remains transient and is
offered only through explicit open and copy controls while Omega listens on the
declared rendezvous relay.

The first approval covers login proof and exact allowed `sign_event` kinds; it
does not include encryption or bulk decrypt. After the signer reports the
person's public key, the dashboard shows that account key and the signer-device
key for a second approval. Activation then requires a verified signed
challenge under the reported account key. Remote accounts use the same
generation-fenced selection registry as local accounts, while **Disconnect
signer** remains distinct from sign out.

Omega persists only the disposable NIP-46 client capability and public signer
metadata below `identity/nip46/<capability-ref>/`. Its `client.secret` and
short-lived `pairing.secret` are atomic owner-only local files on Unix. The
person's root `nsec` stays in the signer. AUTH-04 does not enable the macOS
Keychain, Secure Enclave, Windows credential vault, Linux secret service,
Android keystore, encrypted application vault, or any other native key enclave.

### Relay and hosted authentication state

AUTH-05 adds relay and hosted status to the same account dashboard without
collapsing their authority into signer readiness. The UI uses five exact
domains: **Signer ready**, **Relay authenticated**, **Group admitted**,
**Hosted linked**, and **Action authorized**. A positive state in one row
cannot satisfy another row.

NIP-42 receipts are per account public key, normalized relay URL, and connection
generation. The account dashboard filters them to the selected identity. The
public projection exposes disconnected, challenge pending, authenticated,
refused, and stale outcomes with a digest-derived challenge reference and
public-safe refusal category. The raw challenge never enters the projection.
Wrong relay, challenge, account, timestamp, signature, acknowledgement,
connection generation, and reused proof are refused.

The OpenAgents link uses an exact URL-, method-, payload-, key-, and
freshness-bound NIP-98 proof. The dashboard separately exposes verifying,
linked, expired, rotating, disconnected, revoked, owner-scope refused, service
unavailable, storage failed, and revocation failed. The public binding names
only the Omega public identity and OpenAgents user reference; it does not
contain an access or refresh token and does not imply relay authentication,
NIP-29 group admission, or action authorization.

Hosted access and refresh tokens stay in the channel-scoped, unencrypted
`credentials/credentials.json` file. Atomic Unix writes enforce owner-only
directory mode `0700` and file mode `0600`. This first wave enables no macOS
Keychain, Secure Enclave, Windows credential vault, Linux secret service,
Android keystore, encrypted application vault, native enclave, or
hardware-backed credential store.

### Optional profile and bounded hydration

AUTH-06 adds an optional kind `0` editor to the account detail hierarchy.
**Skip** records only that the optional step was skipped and signs or publishes
nothing. **Save locally** writes an account-partitioned draft. **Publish
profile** signs one exact kind `0` event through the selected signer and does
not report success until the account, public key, generation, event, and relay
acknowledgement have been revalidated.

Imported, recovered, switched, and remote-signer identities run a bounded
hydration plan after selection. The foreground gate has an overall deadline
and each source has a smaller deadline. The dashboard exposes complete,
partial, offline, failed, or skipped-fresh overall state and shows source-level
fresh, cached, locked, disabled, timeout, stale, offline, or failed outcomes.
The UI opens from cache or defaults when the gate expires, then retryable
sources recover in the background. A generation fence prevents late results
from a previous selection from crossing accounts.

External-signer bulk decrypt uses a distinct durable consent control:
unknown, allowed, or declined. Decline leaves content locked and suppresses
repeated requests until the person changes the setting. A signer capability
without bulk decrypt routes to signer reconnection rather than an implicit
permission expansion. Plaintext-cache policy is shown separately and applies
only to the selected account partition.

Profile drafts, hydration receipts, bulk-decrypt consent, ciphertext,
plaintext, and signer cache metadata are ordinary unencrypted local files in
separate `identity/hydration/accounts/<public-key>/` partitions. Atomic Unix
writes enforce directory mode `0700` and file mode `0600`; verified account
purge owns their removal. AUTH-06
enables no macOS Keychain, Secure Enclave, Windows credential vault, Linux
secret service, Android keystore, encrypted application vault, native enclave,
or hardware-backed secret store.

### Activation and recovery ceremony

The desktop identity section inspects one combined activation projection:
account state, exact held intent, and recovery-protection state. Candidate
accounts expose **Set up identity**. An intercepted durable action exposes
both setup and **Cancel action**; cancellation submits the exact
`HeldIdentityAction`, clears the durable intent, notifies its process-local
owner, and never signs, publishes, or resumes anything.

Setup first explains the boundary: the public key is durable authorship, the
local secret authorizes signatures, Omega cannot reset that secret like a
password, and a valid signature does not establish identity, truth,
membership, or permission. The screen then presents three product paths:

- **Keep this identity** is implemented for the local file signer. If recovery
  is not protected it requires creation and verification of an encrypted
  NIP-49 file. If the exact identity already has a verified artifact, setup
  completes without exporting another copy.
- **Use an existing identity** does not overwrite a healthy local candidate.
  The screen explains that safe replacement waits for multi-account switching;
  the existing recovery importer remains available for missing or damaged
  custody.
- **Use a signer on another device** is visibly unavailable until remote
  signer support lands. Selecting it neither activates the account nor moves
  the local secret.

Password and confirmation use `SecureInput`, and the selected directory
receives `omega-identity-recovery.ncryptsec`. The password protects that
artifact only; it is not stored and is not an account-reset password. Export,
read-back verification, activation, and action resumption are ordered. For an
intercepted action, the identity section completes durable activation and then
signals `IdentityActivationEvents` with the exact intent. Only its one-shot
owner may revalidate, consume, and resume the original payload. A mismatched,
expired, or ownerless intent cannot resume.

The callback owner is intentionally process-local because the held payload may
contain content that must not be written into identity metadata. After restart,
the durable intent is shown as orphaned. Setup is disabled; the person cancels
it and retries from the initiating surface. Proactive setup uses the typed
`AccountSetup` intent and may be consumed by the identity section because
there is no external payload to replay.

These controls are shared by the full desktop onboarding item and its compact
replay presentation. Mobile and web clients can implement the same state
machine, education, NIP-49 format, exact-intent rules, and explicit
unavailability states, but this Rust/GPUI implementation does not assert that
those clients currently ship the ceremony.

Named refusal states remain visible through the account and identity repair
entry points: lost, conflict, incomplete, locked, reset-failed, and
relaunch-required are not rounded into Ready. They refuse signing until the
person completes the corresponding repair. The raw-`nsec` backup action is an
advanced escape hatch, while the value-triggered recovery nudge points to
encrypted NIP-49 protection.

Editor Onboarding is a separately replayable mode available from the Welcome
page and the `omega::OpenEditorOnboarding` action (also
`omega::OpenOnboarding`). The Welcome replay remains available on the
default surface. The two actions are aliases for the same editor-setup page;
they do not reinstate a blocking first-run mode. In debug builds,
`dev::ResetOnboarding` clears the identity and editor completion records
so reopen can be retested without wiping the whole profile. It renders the
same Theme and Agent Setup implementations as First Run, with a compact
identity status that retains custody repair, conflict resolution, recovery, and
recovery-protection actions. Its Finish action also requires durable Ready
custody, but writes the independent
`omega_editor_onboarding_completion_v1` record and remains restorable in the
workspace. Neither completion record gates the other journey or suppresses
identity inspection and recovery. Completion records also do not block Welcome
→ Return to Onboarding; that action reopens the Editor Setup view directly.

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
| `desktop/src-tauri/src/secret_store.rs`      | `2f1d93a3427bd2852001c81a0ad88afb6a614dec8eba78f23bcd56d630cd1ce8` | Read-back checks and serialized mutation               |
| `desktop/src-tauri/src/app_state_keyring.rs` | `f4f28872a57ea532dcd7d8f8f4a589b736d86b8bd25a81850c3e782defbd2aa7` | Service scoping, replaced by `app_identity` channels   |
| `desktop/src-tauri/src/app_state.rs`         | `1ee8b09732b1e39afcaf3450ff674880d5d17e434f8900429239a1f05ca33063` | Public recovery states and read-back ordering          |
| `desktop/src-tauri/src/commands/identity.rs` | `8ea33e58265b9a62a16dd163cdf37c06d9ad574418b1f02fd3ac295b8b2b290c` | Public identity results and admitted signing           |
| `desktop/src-tauri/src/reset.rs`             | `9a5b5ac615cf7501c74952e03eac774f937c1458ff163cb6c5a02365655aad64` | Restart-safe reset transaction pattern                 |

The contract uses exact reviewed native packages:

| Package             | Version  | Checksum                                                           | License           |
| ------------------- | -------- | ------------------------------------------------------------------ | ----------------- |
| `nostr`             | `0.44.4` | `98cf5d15d70d1f8f4059e5f79923ac15891eb691d2843d01191e0585fb064d70` | MIT               |
| `atomic-write-file` | `0.3.0`  | `84790c55b5704b0d35130bf16a4ce22a8e70eb0ea773522557524d9a4852663d` | BSD-3-Clause      |

The public contract vector is
`crates/omega_identity/fixtures/omega_nostr_identity_v1.json`, with SHA-256
`1e25670b072cdd500bdd65e1c0215b62068cfd26a98284e23a90c781fef0bba6`.
It freezes public-key, npub, public fingerprint, manifest, and admitted-signing
behavior without containing a private key.

Omega does not adopt Buzz's `BUZZ_PRIVATE_KEY` override, renderer-visible
`get_nsec` command, durable use of ephemeral recovery keys, or Spark/wallet
profile fields. Omega's background provisioning may generate a local identity,
but does so through the journaled native custody boundary.

## Application icon family

Package icons are derived from the pinned OpenAgents Desktop icon under
`crates/omega/resources/icon_family/`. Regenerate with
`script/generate-omega-icon-family`.

Channel badges are deferred. Stable, RC, Nightly, and Dev currently reuse the
same unbadged OpenAgents icon until OpenAgents approves distinct channel
artwork.

Digest and dimension checks live in the `app_identity` icon-family tests and
`crates/omega/resources/icon_family/manifest.json`.

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
