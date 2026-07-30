# Omega Nostr authentication contract

- Status: AUTH-00 contract frozen; AUTH-01 through AUTH-07 implemented
- Packets: `OMEGA-AUTH-00`, `OMEGA-AUTH-01`, `OMEGA-AUTH-02`, `OMEGA-AUTH-03`, `OMEGA-AUTH-04`, `OMEGA-AUTH-05`, `OMEGA-AUTH-06`, `OMEGA-AUTH-07`
- Source baseline: Omega `0136fca2d11900ddc7982665482ed8cd035391c7`
- Product plan:
  [Omega Nostr authentication and onboarding](https://github.com/OpenAgentsInc/openagents/blob/7010561549ebb46a37257292a9100f990a4a3356/docs/omega/2026-07-30-omega-nostr-authentication-and-onboarding.md)

This document records the authentication contract and the runtime portions that
have converged on it. A type existing in `omega_identity::authentication` does
not by itself mean every later product flow ships.

## Current shipping behavior

Omega opens the front door while one shared startup task silently creates or
adopts a local Nostr identity. It does not show a first-run identity ceremony.
The native signer stores its 32-byte secret in
`identity/identity.secret` below the release-channel data root. Provider API
keys, OAuth material, and OpenAgents access or refresh tokens are stored
separately in `credentials/credentials.json`. Both stores use atomic
replacement and owner-only directory and file modes (`0700` and `0600`) on
Unix. They are not encrypted at rest. `KeyringLocator` and `keyring_*` fields
remain version-one compatibility names; they do not mean Omega accesses the
macOS Keychain.

Ready local custody is now distinct from an active account. Startup writes a
public-safe `identity/identity.account.json` record. A newly generated identity
is `CandidateLocal`; an already-ready identity without an account record is
migrated as `CandidateExisting` without changing its key. The account control
shows that state and the short public fingerprint. `Activating` means one
typed durable action is held; `Active` means the local activation step
completed. None of these states proves network or application authority.

AUTH-03 adds a durable multi-account registry at
`identity/accounts/index.json`. The existing root identity migrates without
moving or rotating its `identity/identity.secret` file. Added local identities
are created once in deterministic per-account directories below
`identity/accounts/`, each with its own `identity.secret` and public-safe
custody documents. The account dashboard exposes the public fingerprint,
optional local profile, signer state, recovery state, last successful signer
use, and separate add, setup, switch, lock or unlock, sign-out, forget, and
retirement entry points.

Adding a local identity produces a `CandidateLocal` with NIP-49 protection
needed. **Complete setup** opens the existing recovery-gated activation
ceremony; the dashboard does not silently promote the candidate. Retirement
remains unavailable until a signed retirement policy is implemented. AUTH-04
adds remote signer enrollment through NIP-46 without receiving the person's
root `nsec`.

Omega gates the five durable identity-bearing actions before signing or
mutation. Generic admitted signing remains available for bounded protocol work
such as relay authentication; custody readiness by itself does not authorize a
durable public action. Omega separately:

- answers a relay's exact NIP-42 challenge and tracks success inside that relay
  connection;
- creates an exact URL-, method-, body-, key-, and freshness-bound NIP-98 event
  for `POST /api/omega/auth/session`;
- verifies the returned hosted session before storing its bearer;
- records an Omega public-key to OpenAgents user binding independently of the
  hosted credential;
- performs NIP-29 group admission through the community subsystem.

Recovery uses NIP-49 `ncryptsec` artifacts, bounded password work, public-key
preview, opaque prepared candidates, journaled mutation, and custody read-back.
AUTH-02 makes a verified NIP-49 artifact the normal local-candidate activation
path. Raw `nsec` remains an explicitly advanced recovery path. The activation
screen routes **Use a signer on another device** into the account dashboard's
NIP-46 setup. Replacing a healthy local candidate is still an account-management
decision rather than an in-place key overwrite.

The recovery password encrypts the exported file. It is not an Omega login or
password-reset mechanism. A previously verified recovery artifact satisfies
the activation requirement without forcing another export.

## Identities

The contract keeps four principals distinct:

| Principal | Meaning | Must not imply |
| --- | --- | --- |
| Person account | The human-controlled Nostr identity and its public key | A particular installation, agent, or hosted user |
| Device identity | A key belonging to one installed client and owner account | Possession of the person's root secret |
| Agent identity | A separate Nostr key with an owner attestation | Permission to sign as the person |
| Hosted user | An OpenAgents service-side user reference linked by proof | Local signer possession or relay/group authority |

Their Rust records are `PublicPersonIdentity`, `PublicDeviceIdentity`,
`PublicAgentIdentity`, and `PublicHostedUserIdentity`. Their reference
newtypes prevent accidental substitution in typed code. `PrincipalSet`
additionally rejects equal serialized identifiers across principal roles.

## Five independent authority states

These phrases are not synonyms:

| State | Question answered | Evidence |
| --- | --- | --- |
| Local signer ready | Can this signer perform its declared cryptographic operation now? | A signer reference |
| Relay authenticated | Did one relay connection accept the exact NIP-42 response? | Relay URL, auth event, and time |
| Group admitted | Did the named group authority admit this account? | Group and authority references |
| Hosted account linked | Did OpenAgents verify the public-safe account binding? | Hosted user and proof references |
| Action authorized | May this exact bounded action proceed? | Capability, authorization, and validity window |

`AccountAuthenticationProjection::validate` requires matching typed evidence
for every positive state and rejects duplicate or cross-domain evidence.
`action_is_authorized` is true only for an `Authorized` action state backed by
`AuthenticationEvidence::Action`; signer, relay, group, and hosted evidence
cannot substitute.

## Account and signer state

`AccountLifecycleState` freezes local and imported candidates, activation,
active use, switching, locking, sign-out, forget, repair, and conflict as
separate states. AUTH-03 implements a monotonic generation for active-account
selection.
Switch, lock, unlock, sign out, and forget compare the expected generation
before mutation. Signing resolves the currently selected account partition and
revalidates the account reference, public identity, lifecycle, and generation;
stale selection tokens and locked, signed-out, or forgotten accounts are
refused.

AUTH-01 implements the first four native states as
`IdentityActivationState::{CandidateLocal, CandidateExisting, Activating,
Active}`. The durable account record binds the account reference, generation,
exact public identity, candidate origin, activation reference, and update
time. A missing account record beside ready custody means only that an older
profile needs migration; it never authorizes deletion or key rotation.

`SignerProjection` names the signer kind, availability, recovery state, and
declared capabilities. Local-native, NIP-46, NIP-07, NIP-55, device-grant, and
agent-grant signers are distinct. A ready signer still conveys no relay,
group, hosted, or action authority.

## Signing and durable-action authorization

`SigningAuthorizationContext` is the target envelope for a future admitted
signing request. It binds:

- authorization proof, account, account generation, and signer;
- calling subsystem and signing purpose;
- event kind, resource or room, and network origin;
- the exact content digest and capability;
- user-gesture status; and
- issue and expiry times.

AUTH-00 deliberately does not replace the shipping
`AdmittedSigningRequest`. Later packets must validate the target envelope
against live state immediately before signing and refuse stale or mismatched
work.

AUTH-01 adds a narrower gate for the first durable identity-bearing action.
`DurableIdentityActionDescriptor` covers public posts, community joins, device
grants, hosted-account links, and agent attestations. It binds an intent,
destination, authorization proof, payload digest, and expiry. For a candidate,
Omega atomically persists the corresponding `HeldIdentityAction` in
`identity/identity.action-intent.json` and moves the account to `Activating`.

Completing activation does not itself replay work. The initiating surface owns
the exact action payload in a process-local one-shot callback while the durable
file stores only its bounded digest and references. After completion, that
owner must consume the same held action exactly once. Consumption rechecks the account reference,
account generation, identity, action kind, destination, authorization proof,
payload digest, and expiry. Cancellation restores the original candidate
state, clears the held action, and resumes nothing. A product surface must
provide explicit complete and cancel controls and retain enough typed state to
resume the original action; redirecting to the account control alone does not
satisfy this contract.

If Omega restarts, the durable intent remains inspectable but its secret or
content-bearing payload is deliberately not reconstructed from disk. The
activation screen therefore refuses completion for that orphan, explains why,
and offers exact cancellation. The person then starts the originating action
again. The ownerless `AccountSetup` intent used for proactive setup is the sole
case the identity screen may consume itself.

## Public-safety rules and fixtures

All public structs deny unknown fields. Contract validation scans public JSON
recursively and rejects secret-, private-key-, `nsec`-, password-, mnemonic-,
seed-, token-, ciphertext-, decrypted-content-, and private-prompt-shaped
fields, along with secret-bearing `nsec1`, `ncryptsec1`, `bunker://`, and
Bearer values.

Canonical and negative vectors live in
`crates/omega_identity/fixtures/omega_authentication_contract_v1.*.json` and
`omega_signing_authorization_context_v1.canonical.json`. They prove schema
round-trip, principal separation, authority non-substitution, unknown-field
refusal, secret tripwires, account-generation binding, and content-digest
binding.

## Migration and custody boundary

AUTH-01 and AUTH-02 change account admission and recovery decisions, not secret custody. The person key remains
the raw 32-byte value in the owner-only, channel-namespaced
`identity/identity.secret` file. This wave does not enable the macOS Keychain,
Secure Enclave, Windows credential vault, Linux secret service, Android
keystore, or any other native secure-enclave or key-vault integration. The
public account and action-intent files contain no secret material and use the
same atomic local-file writer.

Fresh profiles still open without a modal. Existing ready identities keep the
exact public key and signed history. Lost, conflict, incomplete, locked,
reset-failed, and relaunch-required custody states remain distinct and route
to the existing repair surface rather than being rounded into a candidate or
active account.

AUTH-02 adds no password vault. NIP-49 output is written only to the folder the
person chooses, while its public-safe verification record remains under the
identity directory. No recovery password or decrypted secret is persisted.

## Account switching and local removal

AUTH-03 preserves the legacy root account and added local candidates instead of
overwriting either one. The registry records the active account reference and
generation, signer summary, recovery state, optional local profile, lifecycle,
and storage locator. A crash-resumable switch transaction verifies the target
partition before committing a new active selection and generation.

Drafts, audience state, and community room state use namespaces derived from
the public identity. Forget this device starts a durable per-account purge
journal. Registry-owned custody files are deleted and read back by the identity
service; Drafts and combined RoomState are marked deleted only after their
owning stores return verified success. Decrypted cache, wallet state, relay
state, signer sessions, and device grants do not yet expose owner purge hooks,
so they remain named, retryable partial failures. Omega does not turn those
pending integrations into a successful purge.

Forget this device is local lifecycle, not Nostr event retraction. Its
confirmation states that events held by relays or peers remain and that an
external NIP-49 recovery file is not deleted. Identity retirement is a
different future signed-policy operation and is unavailable in this wave.

## NIP-46 remote signers

AUTH-04 supports both NIP-46 pairing directions. A person may paste a bounded
`bunker://` connection from an existing signer, or ask Omega to create a
`nostrconnect://` pairing link. Omega generates a disposable client key and
secret for the latter. The secret-bearing URI is transient UI state and is
available only through explicit open and copy controls while the listener is
running. It is never included in logs, errors, action payloads, or public
account records.

Before network exchange, the desktop UI shows the expected signer when known,
the requested methods, exact event kinds, exact relay set, seven-day lifetime,
and dependence on the remote signer for recovery. The first profile is limited
to login proof plus `sign_event` for kinds `9`, `1111`, `1984`, `9021`, `10009`,
`22242`, and `27235`. Kinds `9021` and `10009` are the exact NIP-29 join-request
and relay-qualified group-list permissions; Omega does not request an unbounded
`sign_event` wildcard. NIP-44 encrypt/decrypt and bulk decrypt use separate
consent profiles; they are not silently folded into connection consent.

Pairing is generation-fenced and correlation-bound. Omega accepts only NIP-46
events addressed to the disposable client key from a declared relay, verifies
the event signature, author, request id, kind, tags, ciphertext, and response
shape, and rejects duplicate or stale responses. A signer acknowledgement is
not activation. Omega first asks for the person's public key, then presents
both the reported Nostr account and signer-device public key for a second
explicit approval. Only after that approval does Omega request and verify an
exact one-time signed login challenge under the reported account key and
register the account.

Offline, silence, timeout, explicit rejection, revocation, and protocol
verification failures remain distinct. Rejection and revocation are terminal
and never trigger another approval prompt. Sign out only clears account
selection. **Disconnect signer** is the separate remote lifecycle action that
revokes the client capability, verifies deletion of its disposable client key,
and marks the signer unavailable.

Remote signer state is stored below `identity/nip46/<capability-ref>/`.
`pairing.json`, `capability.json`, and public account metadata are public-safe;
`client.secret` and the short-lived `pairing.secret` are atomic owner-only
local files on Unix. The pairing secret is deleted after acknowledgement, and
the disposable client secret is deleted on rejection, revocation, or
disconnect. This wave does not use the macOS Keychain, Secure Enclave, Windows
credential vault, Linux secret service, Android keystore, encrypted
application vault, or any other native enclave integration.

## Relay and hosted authentication

AUTH-05 makes relay authentication and hosted account linking observable
without treating either as general authority.

NIP-42 state is connection-scoped and keyed by the account public key,
normalized relay URL, and monotonic connection generation. Each public-safe
receipt reports one of
disconnected, challenge pending, authenticated, refused, or stale. A receipt
may include a digest-derived challenge reference, the accepted authentication
event id, a bounded refusal category, and the observation time. It never
contains the relay's raw challenge. A challenge and its response may be used
only once, and a late acknowledgement from a previous connection generation
cannot authenticate the current connection. The event must bind the exact
relay, exact challenge, selected account, current connection, signature, and
fresh timestamp.

The desktop account dashboard filters receipts to the selected account public
key and presents each relay independently. **Relay
authenticated** means only that this WebSocket connection accepted its exact
NIP-42 proof. It does not mean **Signer ready**, **Group admitted**, **Hosted
linked**, or **Action authorized**.

The OpenAgents hosted flow uses one exact NIP-98 proof for the HTTPS URL,
method, payload digest, signer, and freshness window. Proof identifiers are
single-use for the account. The returned session is verified before it is
stored or exposed as linked. The public session projection distinguishes
verification, expiry, rotation, disconnect, revocation, owner-scope refusal,
service unavailability, credential-storage failure, and revocation failure.
Failures are never rounded up to a linked session.

The public Omega-public-key to OpenAgents-user binding is stored as public-safe
evidence independently from the bearer credential. That binding proves only
which hosted user the service associated with the Omega account. It cannot
admit a NIP-29 group, authenticate a relay, or authorize an arbitrary action.

Hosted access and refresh tokens remain in the owner-only, unencrypted
`credentials/credentials.json` file below the channel data root. Unix writes
enforce directory mode `0700` and file mode `0600`. This first wave explicitly
enables no macOS Keychain, Secure Enclave, Windows credential vault, Linux
secret service, Android keystore, encrypted application vault, native enclave,
or hardware-backed secret store.

## Profile and bounded hydration

AUTH-06 makes a Nostr kind `0` profile optional. The account dashboard offers
three exact outcomes: **Skip**, **Save locally**, and **Publish profile**.
Skip writes only the local skipped state and performs no signing, relay
connection, or publication. Saving locally updates the account-partitioned
draft without claiming a public profile. Publishing builds one exact kind `0`
event, routes it through the selected signer, revalidates the account and
generation after any external approval, and records the acknowledged result.

Imported, recovered, switched, and remote-signer accounts start a bounded
hydration plan. The plan covers the kind `0` profile, relay preferences,
selected NIP-29 group list, recent membership and room metadata, bounded recent
room pages, hosted account and device state when linked, and adapter-specific
state only when that adapter is enabled. Every source has its own deadline and
the whole gate has an overall deadline. A fresh unpublished candidate records
`skipped_fresh` and opens immediately.

The receipt reports complete, partial, offline, or failed overall outcomes and
one explicit source result for fresh, cached, disabled, locked, offline,
timeout, stale, success, or failure. Cached/default state may open the desktop
after the foreground deadline. Retryable sources continue recovering in the
background under the same selected-account and generation fence. Late work
from a previous account selection cannot update the new account.

Bulk decryption by a remote or external signer is a separate durable consent:
unknown, allowed, or declined. It is never inferred from login, kind `0`
publication, ordinary event signing, or one successful decrypt. A decline is
persisted and suppresses repeated prompts; content remains locked behind an
explicit later consent change. If the NIP-46 capability lacks the bulk-decrypt
method, the UI requires signer reconnection rather than requesting it
silently.

Persistent plaintext caching has a separate, disclosed per-account policy.
Plaintext, ciphertext, profile drafts, hydration receipts, and signer cache
metadata use separate partitions under
`identity/hydration/accounts/<public-key>/` and participate in verified local
purge. They are ordinary unencrypted local files protected by the user's OS
account and owner-only permissions on Unix. AUTH-06 enables no macOS Keychain,
Secure Enclave, Windows credential vault, Linux secret service, Android
keystore, encrypted application vault, native enclave, or hardware-backed
credential store.

## Community entry interoperability

AUTH-07 resolves community entry without changing the selected account's Nostr
public key. It recognizes five explicit destination profiles:

- relay-qualified standards-first NIP-29 groups;
- NIP-29 relay servers whose signed kind `10009` list is updated separately;
- the pinned Buzz compatibility profile;
- admitted Armada Concord v1 or v2 profiles; and
- Omega/OpenAgents service invites.

This first implementation executes the standards-first NIP-29 profile. Buzz,
Armada Concord, and OpenAgents authority adapters are preview-only and their
commit controls remain disabled until their typed verification and mutation
paths are available.

The preview names the profile, authoritative relay or service, room identifier,
visibility, terms requirement, requested signing operations, recovery model,
and portability to independent NIP-29, Buzz, Armada, web, and mobile clients.
These labels are claims about the exact resolved profile. A Buzz extension is
not presented as complete NIP-29 support, and Concord is not presented as
encrypted NIP-29. An OpenAgents service can grant only the OpenAgents result it
actually returns; NIP-42 authentication, relay membership, a Buzz claim, or a
Concord membership never implies OpenAgents membership, command, moderation,
payment, or release authority.

Malformed, stale, banned, terms-required, and unsupported-profile outcomes are
separate refusal states. Unsupported input remains opaque evidence and cannot
be joined, translated, or projected into a guessed common room model. A
terms-required preview must be accepted explicitly before the transaction can
advance.

Omega writes a public-safe join transaction before its first network mutation.
Relay addition, NIP-42 authentication, invite claim, NIP-29 join, and an
OpenAgents grant are independent step results. Completed steps are not repeated
after restart. A partial transaction remains visible with its transaction
reference, completed and failed steps, and a **Resume join** action; it is not
rounded up to **Group admitted** and it does not silently roll back evidence of
a remote mutation.

Raw invite codes, capability query values, and URL fragments are never rendered
in a preview, error, public transaction projection, log, telemetry event, or
serialized public record. Public projections retain only profile and authority
labels, normalized public destination facts, an input digest and byte length,
and step receipts. The private transaction payload below
`identity/invites/accounts/<public-key>/` may retain bounded exact step-request
bytes required for restart-safe NIP-29, Buzz, or Concord work until completion,
revocation, expiry, or cancellation. It is account-partitioned, never enters a
public projection, and is deleted with verified read-back.

The public projections describe ordinary unencrypted private transaction files.
Unix directories use mode `0700` and files use mode `0600`. AUTH-07 enables no
macOS Keychain, Secure Enclave, Windows credential vault, Linux secret service,
Android keystore, encrypted application vault, native enclave, or
hardware-backed credential store.
