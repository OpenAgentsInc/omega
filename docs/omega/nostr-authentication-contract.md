# Omega Nostr authentication contract

- Status: frozen target contract; runtime behavior is unchanged
- Packet: `OMEGA-AUTH-00`
- Source baseline: Omega `0136fca2d11900ddc7982665482ed8cd035391c7`
- Product plan:
  [Omega Nostr authentication and onboarding](https://github.com/OpenAgentsInc/openagents/blob/7010561549ebb46a37257292a9100f990a4a3356/docs/omega/2026-07-30-omega-nostr-authentication-and-onboarding.md)

This document records what Omega ships before the authentication work begins
and freezes the public-safe Rust contract that the later packets must converge
on. A type existing in `omega_identity::authentication` does not mean the
corresponding product flow ships yet.

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

Ready local custody allows `IdentityService` to sign an admitted Nostr event.
It does not prove any network or application authority. Omega separately:

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
The desktop also has an advanced raw-`nsec` backup surface and a value-triggered
backup nudge. Neither currently activates an account in the sense defined
below.

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

`SignerProjection` names the signer kind, availability, recovery state, and
declared capabilities. Local-native, NIP-46, NIP-07, NIP-55, device-grant, and
agent-grant signers are distinct. A ready signer still conveys no relay,
group, hosted, or action authority.

## Future signing authorization

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

## Migration boundary

No runtime decision consumes these projections in AUTH-00. Background
provisioning, storage, signing, NIP-42, NIP-98, account binding, recovery, and
backup behavior remain unchanged. `OMEGA-AUTH-01` starts the migration by
persisting candidate versus active account state without rotating an existing
key.
