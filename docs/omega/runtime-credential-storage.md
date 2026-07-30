# Runtime credential storage

Omega does not access the macOS Keychain for application runtime credentials.

The shared credentials provider stores provider API keys, OAuth sessions,
OpenAgents native-session tokens, and similar byte credentials in
`credentials/credentials.json` below the channel-specific application data
directory. Records remain release-channel namespaced. Writes use an atomic
replacement, the directory is mode `0700`, and the file is mode `0600` on
Unix.

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

These packets do not enable or
probe the macOS Keychain, Secure Enclave, Windows credential vault, Linux
secret service, Android keystore, or another native key-vault integration.

Apple code signing and notarization may use a build-machine signing identity.
That packaging operation is outside the installed application's runtime and
does not give Omega access to the build machine's credential store.
