# Omega Nostr authentication contract

- Status: AUTH-00 contract frozen; AUTH-01 candidate model and AUTH-02 activation ceremony implemented
- Packets: `OMEGA-AUTH-00`, `OMEGA-AUTH-01`, `OMEGA-AUTH-02`
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
screen names existing-identity and remote-signer choices, but does not pretend
they work in this wave: replacing a healthy candidate waits for multi-account
switching, and remote signing waits for the signer packet.

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
separate states. `account_generation` is nonzero and will invalidate stale
network and signing work after account changes.

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
