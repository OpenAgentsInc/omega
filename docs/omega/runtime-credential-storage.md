# Runtime credential storage

Omega keeps ordinary application runtime credentials out of the macOS
Keychain. Hyperliquid agent-wallet private keys are the single narrow
exception: Omega generates those trading-only keys locally and stores them in
the platform credential store. They are never written to the ordinary
credential file.

The shared credentials provider stores provider API keys, OAuth sessions,
OpenAgents native-session access and refresh tokens, and similar byte credentials in
`credentials/credentials.json` below the channel-specific application data
directory. Records remain release-channel namespaced. Writes use an atomic
replacement, the directory is mode `0700`, and the file is mode `0600` on
Unix.

Hyperliquid testnet and mainnet agent wallets have separate, versioned,
release-channel-namespaced credential keys and separate fixed agent names.
Loading a record validates both the named network and agent name. The selected
wallet is passed to the supervised Nautilus process in one typed bootstrap
record over its inherited stdin; it is not placed in the parent environment,
command line, configuration, or a plaintext disk file. Omega surfaces the
owner, agent address, network, `extraAgents` approval, and `validUntil`, but
never the private key. Expired, revoked, unknown-mode, or network-mismatched
approval halts startup and emits a typed credential wakeup. Hyperliquid
mainnet connections remain refused before any request or sidecar effect until
the separate graduation gate passes.

This exception grants trading authority only: **Omega can trade on this
account; Omega cannot withdraw.** The person's Hyperliquid master wallet is
never held by Omega. The existing `global` and `local_credentials` providers
continue to use the owner-only file above; they do not read, migrate, or prompt
for Keychain entries.

The Nostr signing secret is stored separately as
`identity/identity.secret` below the channel-specific application data
directory. It uses the same atomic-write and owner-only permission rules.
AUTH-03 preserves that path for the migrated root account. Additional local
accounts use the same file-only format at
`identity/accounts/<account-directory>/identity.secret`; the directory name is
deterministically derived from the opaque account reference and is not a
secret.

The identity directory also contains public-safe authentication metadata:

- `identity/identity.account.json` records the exact public identity, account
  generation, and candidate, activating, or active state.
- `identity/identity.action-intent.json` temporarily records one typed durable
  action while activation is in progress. It contains references, a
  destination, a payload digest, and an expiry, never the payload or secret.
- `identity/identity.recovery-protection.json` records that an encrypted
  recovery artifact for the exact identity was written and verified. It
  contains no recovery password, ciphertext, or secret.

These metadata records do not make the secret file encrypted and do not grant
relay, group, hosted-account, or action authority.

The multi-account registry is `identity/accounts/index.json`. It contains only
public identity, lifecycle, signer summary, recovery state, optional local
profile, storage locator, and the active account reference with its monotonic
generation. `identity/accounts/switch.transaction.json`, add transaction
journals, and per-account purge journals make interrupted mutations resumable
without placing secret material in the registry.

Drafts and community/audience room state are stored under public-key-derived
namespaces. Forget this device asks those owning stores to delete and verify
their own account partitions. Unsupported decrypted-cache, wallet, relay,
signer-session, and device-grant purge targets remain explicit retryable
partial failures; the purge journal does not claim success while any target is
unverified.

The public authentication glossary and target schemas are frozen in
[Omega Nostr authentication contract](nostr-authentication-contract.md).

Exo is always launched with `EXO_SECRET_BACKEND=file` and an explicit master
key path. Configuring `apple-keychain` is rejected.

The version-one identity evidence schema still calls its stable logical
identity locator a `KeyringLocator`. That serialized compatibility name does
not invoke or describe the macOS Keychain; runtime secret storage is the local
file above. Omega does not read or migrate old Keychain entries because doing
so could trigger the prompt this change removes.

These files are intentionally not encrypted at rest. This removes unstable
binary-identity prompts from development and release builds, but it makes the
security boundary the user's operating-system account and application data
directory. Omega must never log or render their contents. Unix ownership and
mode are set to owner-only values when directories and files are written;
equivalent Windows ACL assurance and validation of permissions on every read
remain explicit platform-verification requirements.

AUTH-01 through AUTH-04 deliberately keep this file-backed design. AUTH-02
writes the NIP-49 `ncryptsec` artifact to the directory explicitly selected by
the person and keeps passwords only in zeroizing memory for the operation. It
does not introduce an encrypted application vault or an account-reset
password. AUTH-03 does not move either the migrated or partitioned
`identity.secret` files into native storage.

Remote signer custody in AUTH-04 stores NIP-46 pairing state below
`identity/nip46/<capability-ref>/`. Public-safe pairing and capability JSON sit
beside owner-only `client.secret` and short-lived `pairing.secret` files. The
pairing secret is deleted after acknowledgement. Rejection, revocation, and
**Disconnect signer** delete and read back the disposable client secret. The
person's root `nsec` remains in the external signer and never enters Omega.

These packets do not enable or probe the Secure Enclave, Windows credential
vault, Linux secret service, Android keystore, or another native key-vault
integration. The platform credential-store exception described above is
limited to Hyperliquid agent-wallet custody.

AUTH-05 keeps hosted access and refresh tokens in the same unencrypted,
owner-only `credentials/credentials.json` file. A schema-versioned record
tracks issued and expiry times so verification and rotation cannot silently
reuse an expired bearer. The public hosted-session projection contains only a
public account binding, lifecycle state, timestamps, and public-safe failure
category. It never contains either token. Disconnect does not report success
until remote revocation and local credential deletion have reached their
defined terminal state; storage and revocation failures remain visible and
retryable.

NIP-42 receipts are public metadata rather than credentials. They contain the
account public key, normalized relay URL, connection generation, challenge reference, accepted
authentication event id, state, public-safe refusal category, and observation
time. They never persist the raw relay challenge or a signing secret, and they
cannot mint a hosted session.

AUTH-05 also enables no encrypted application vault, native enclave,
hardware-backed credential store, or native credential integration. File-backed
storage is the deliberate first-wave boundary.

AUTH-06 stores profile drafts, hydration receipts, bulk-decrypt consent, and
plaintext-cache policy in ordinary unencrypted files below
`identity/hydration/accounts/<public-key>/`. Public kind `0` drafts and receipts contain no signing
secret. The bulk-decrypt consent contains only unknown, allowed, or declined;
it never contains decrypted content or a signer capability. Declined consent
is durable so reconnects and background hydration cannot create a prompt
storm.

Plaintext cache policy is separate from the cache itself. When persistent
plaintext is disabled, background hydration may retain ciphertext and
public-safe metadata but must not write decrypted content. When enabled, every
plaintext entry is keyed to the exact account public key and is removed only
through the account purge owner with verified read-back. Ciphertext, plaintext,
profile drafts, hydration state, and signer caches remain separately named
purge targets.

These AUTH-06 records use the existing atomic owner-only file boundary:
directories mode `0700` and files mode `0600` on Unix. They do not enable the
macOS Keychain, Secure Enclave, Windows credential vault, Linux secret service,
Android keystore, encrypted application vault, native enclave, or another
native or hardware-backed key store.

AUTH-07 stores restart-safe community-entry transactions under
`identity/invites/accounts/<public-key>/`. Their public projections expose an
input digest and independent receipts for relay addition, NIP-42
authentication, invite claim, NIP-29 join, and any OpenAgents grant. Partial
failures remain durable and retryable. Unsupported profile material is
preserved as opaque evidence rather than interpreted.
Only standards-first NIP-29 is executable in this implementation. Buzz, Armada
Concord, and OpenAgents authority adapters remain preview-only and cannot create
false authority receipts.

Raw invite codes, secret query parameters, and URL fragments are not logged,
rendered, or included in public records or projections. Those surfaces expose
only a digest and byte length. A restart-safe NIP-29, Buzz, or Concord claim may
require exact bounded step-request bytes, so the private transaction payload may
store them in the same account partition. That payload is read only for the
exact transaction and is deleted with verified read-back after claim
completion, revocation, expiry, or cancellation.

The private transaction payload is an ordinary unencrypted file written
atomically with directory mode `0700` and file mode `0600` on Unix. This is not
a secret vault and AUTH-07 does not enable the macOS Keychain, Secure Enclave,
Windows credential vault, Linux secret service, Android keystore, encrypted
application vault, native enclave, hardware-backed credential store, or
another native custody integration.

AUTH-08 stores device enrollment below `identity/device-enrollment/`.
Host-side pairing state and device-grant inventory live in
`host/<owner-public-key>/enrollment.json`; target-side permanent device
credentials live in `local/<owner-public-key>/devices/<device-public-key>.json`.
Public projections contain the account and device public fingerprints,
platform, admitted capability names, lifecycle, creation and expiry times,
last successful use, and public-safe refusal or revocation results. They
contain no introduction secret, ephemeral private key, device private key,
root `nsec`, or bearer.

The one-use introduction secret and host ephemeral key remain in the host
record only while pairing is open. A joining target writes restart-safe
exchange and device-key material to
`local/<owner-public-key>/pending/<pairing-id>.json` before returning its
response. Successful redemption clears the host exchange secrets; persisting
the returned grant atomically writes the permanent target credential and
deletes the target pending record. Expiry pruning verifies host deletion and
removes the matching target pending record when both roles share a data root.
Revoking one grant does not delete the person identity or another device's
grant.

These records are ordinary unencrypted files written atomically with directory
mode `0700` and file mode `0600` on Unix. A permanent device key created and
owned by Omega is stored in those ordinary local files in this wave. NIP-07,
NIP-46, and NIP-55 adapters keep their separately managed keys outside Omega;
Omega does not copy them into its files. This first wave enables no macOS
Keychain, Secure Enclave, Windows credential vault, Linux secret service,
Android keystore, encrypted application vault, native enclave, hardware-backed
credential store, or other native key custody.

AUTH-09 stores separate agent identities and bounded owner grants below
`identity/agents/records/<account-ref-sha256>/<agent-pubkey>.json`, with
incomplete attestations below
`identity/agents/pending/<request-ref-sha256>.json`. The agent secret remains
in an ordinary local secret file owned by that agent record. Public inventory and
receipt records contain only agent and owner references, public keys, exact
methods, event kinds, room or tenant resources, generation, issue and expiry
times, attestation reference, revocation, and last successful use. They contain
no agent secret, person root `nsec`, device private key, bearer, private prompt,
or decrypted payload.

Agent files use the same atomic ordinary-file boundary: directories mode
`0700` and files mode `0600` on Unix. Installed Windows ACL behavior and
packaged host permissions remain explicit evidence requirements; Unix mode
tests do not prove them. The installed-candidate inventory and secret-tripwire
procedure are recorded in
[Omega Nostr authentication assurance](nostr-authentication-assurance.md).
AUTH-09 enables no macOS Keychain, Secure Enclave, Windows credential vault,
Linux secret service, Android keystore, encrypted application vault, native
enclave, hardware-backed credential store, or other native key custody.

Apple code signing and notarization may use a build-machine signing identity.
That packaging operation is outside the installed application's runtime and
does not give Omega access to the build machine's credential store.
