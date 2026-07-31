# Omega Nostr authentication assurance

- Status: AUTH-09 source assurance implemented; installed verdicts remain evidence-bound
- Packet: `OMEGA-AUTH-09`
- Machine-readable matrix: `nostr-authentication-assurance-matrix.json`

This document separates what the Rust test suite proves from what must still be
observed in an installed candidate. A passing source test is not an installed
macOS, Windows, Linux, web, Android, or iOS result. A row marked
`owner-assisted-pending` has no verdict until the named host journey is run
against the exact packaged candidate and its evidence is retained. A row
marked `not-admitted` is not a claim that the host works.

## Identity and authority model

Omega displays and stores four principals separately:

| Principal | Public projection | Authority |
| --- | --- | --- |
| Person | Account reference and Nostr public key | Human-controlled account; does not identify one device, agent, or hosted user |
| Device | Device fingerprint, platform, capabilities, expiry, and revocation | One independently revocable device grant; never the person's root key |
| Agent | Agent reference, separate Nostr public key, and owner attestation | Only the exact methods, event kinds, room or tenant resources, generation, and time window in its grant |
| Hosted user | OpenAgents user reference and verified binding | Hosted session only; cannot substitute for relay auth, group admission, or action authorization |

The account dashboard may render those public records and bounded receipts. It
must never render or serialize a private key, disposable signer secret, bearer,
recovery password, decrypted payload, or private prompt. Agent signing does not
fall back to the person signer. A person-signing capability would require a
separate, exact, admitted capability and is not part of AUTH-09.

## Agent grant assurance

Each owner-attested agent grant names:

- the owner account reference and person public key;
- the distinct agent reference and agent public key;
- exact signer methods and event kinds;
- exact room or tenant resource references;
- account generation, issue time, expiry, and grant reference;
- owner-attestation reference; and
- active or revoked state and last successful use.

Every request additionally binds its request id, signer, subsystem, purpose,
destination, origin, content digest, capability, gesture state, and expiry.
Wrong account, generation, method, kind, resource, origin, digest, request id,
signer response, or time window fails closed. NIP-42 relay authentication,
NIP-29 group membership, OpenAgents hosted linking, device enrollment, and an
owner action authorization are independent evidence domains.

NIP-AA agent relay authentication is admitted only through the pinned agent
relay profile and is displayed separately from ordinary NIP-42. It binds the
owner-attested agent grant and relay connection; it is not virtual membership
and cannot be spent as the person's NIP-42 receipt.

## Secret tripwires

The installed canary collector is
`script/collect-omega-installed-tripwires`. The caller must give the candidate
a disposable canary during the journey and pass the already-open canary file
descriptor to the collector. The collector does not invent its own needle and
does not record its value. It live-fires the scanner, then inspects logs,
telemetry, diagnostics, crash reports, the general clipboard, and the
accessibility tree. An unreadable required surface is `blocked`, never a pass.

Run a distinct canary and receipt after each applicable installed journey:
migration, storage, recovery, NIP-42, NIP-46, account switching, logout,
community invite, hydration, device pairing, and authority separation. The
matrix does not currently claim that this repetition is orchestrated
automatically. The existing collector is automated; placing the canary into
the exact UI/protocol journey and associating the receipt with that journey is
owner-assisted.

Each matrix row sets `tripwire_required` and names the applicable disposable
canary class: private-key-shaped, disposable signer secret,
hosted-token-shaped, private prompt, decrypted payload, or invite capability.
These are fresh test values, never a person's real key, token, prompt, or
payload. Run the collector once per named class; a row without every named
canary receipt has no installed verdict.

The source suite additionally rejects secret-shaped fields and values in public
authentication, agent, device, invite, hosted, and receipt projections. Source
tripwires cannot observe a packaged clipboard, accessibility tree, OS crash
report, filesystem ACL, deep-link handler, or external signer.

## Installed-candidate matrix

The adjacent JSON file is the normative row inventory. Every row names its
source checks, installed surfaces, expected refusal cases, and host status.
The current meanings are:

- `source-automated`: a hermetic Rust or script test runs in the repository;
- `installed-automated`: a packaged executable was exercised without a person
  and produced candidate-bound evidence;
- `owner-assisted-pending`: the exact packaged journey still needs a person,
  external signer, relay, second device, or host inspection;
- `blocked`: the required host or observation interface was unavailable; and
- `not-admitted`: Omega does not ship that journey on that host.

AUTH-09 adds no blanket `installed-automated` claim. The release record may
promote one host row only after it stores candidate-bound evidence for every
surface named by that row. Source tests and fixtures cannot promote it.

### Host truth in this wave

- macOS desktop: source coverage exists. Packaged filesystem modes, clipboard,
  deep links, external signer behavior, crash surfaces, and accessibility
  projection remain owner-assisted until run on the signed candidate.
- Windows and Linux desktop: source behavior is shared, but installed ACL,
  permissions, deep links, and package integration remain pending on actual
  packages. Unix `0600`/`0700` tests do not prove Windows ACLs.
- Web: NIP-07 and NIP-46 are admitted adapter boundaries in Rust source, not a
  packaged Omega web host. Installed web rows are `not-admitted` in this
  matrix, and Omega stores no local root key there.
- Android: only an explicitly reported and admitted NIP-55 host is allowed.
  There is no packaged Android Omega host in this matrix, so installed rows are
  `not-admitted`.
- iOS: NIP-46 is the honest admitted route in this wave. No NIP-55 or native
  signer parity is claimed, and no packaged iOS Omega host is admitted by this
  matrix.

## Storage checks

All Omega-local person, device, agent, pairing, grant, recovery metadata, and
session records in this wave use ordinary local files. Secret-bearing files
are intentionally unencrypted at rest and use atomic replacement plus
owner-only `0700` directories and `0600` files on Unix. This boundary is the
operating-system user and application-data directory.

Agent records use
`identity/agents/records/<account-ref-sha256>/<agent-pubkey>.json`; incomplete
attestations use
`identity/agents/pending/<request-ref-sha256>.json`. Both are ordinary
unencrypted files under the same boundary.

AUTH-09 enables no macOS Keychain, Secure Enclave, Windows credential vault,
Linux secret service, Android keystore, encrypted application vault, native
enclave, or hardware-backed credential store. External NIP-07, NIP-46, and
NIP-55 signers keep their own custody; Omega does not reclassify that external
custody as an Omega native vault.

## Release evidence procedure

For each applicable row and host:

1. Bind the evidence to the exact installed executable digest and release
   record.
2. Start from the row's clean or migrated profile precondition.
3. Exercise the real packaged storage, signer, clipboard, deep-link, relay, and
   filesystem interfaces named in the row.
4. Inject a fresh disposable canary through the exercised path, run the
   installed tripwire collector, and retain its redacted receipt.
5. Record every refusal outcome named in the row. A timeout, unavailable
   external signer, or unreadable surface is not rounded up to success.
6. Keep logs, telemetry, crash records, public receipts, screenshots, and test
   artifacts free of raw keys, tokens, private prompts, and decrypted payloads.

The release gate must show pending or blocked rows honestly until those steps
produce candidate-bound evidence.
