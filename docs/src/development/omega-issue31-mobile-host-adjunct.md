# Omega issue 31 mobile host adjunct

- Date: 2026-07-24
- Packet: `OMEGA-MOB-31-00`
- Issue: [#44](https://github.com/OpenAgentsInc/omega/issues/44)
- Connected record: [#45](https://github.com/OpenAgentsInc/omega/issues/45)
- Parent: [#31](https://github.com/OpenAgentsInc/omega/issues/31)

## Result {#result}

Omega exposes a bounded, public-safe decoder for
`openagents.omega.issue31.host.v1`. OpenAgents Mobile joins this adjunct with
the existing signed Nostr records in its local `Issue31WorkroomReadModel`.
Neither the adjunct nor the local aggregate is durable authority.

The decoder lives in `workroom_receipts` because that crate already owns the
pure public-reference and redaction boundary used by the workroom receipt
inspector. It has no GPUI, network, process, or durable-state dependency.
Runtime production of the snapshot stays with Omega's operation owners and
`omega_effectd`; the later Full Auto mobile packet connects that runtime.

## Direct device bridge {#direct-device-bridge}

Omega provides the authenticated
`openagents.omega.device_bridge.v1` WebSocket transport for a paired mobile
device. The socket implementation lives in the GPUI-free
`omega_device_bridge` crate. `omega_effectd` owns its lifecycle and adapts the
durable Issue 31 grant controller to the transport.

The bridge binds only a literal loopback address or a Tailscale address in
`100.64.0.0/10` or `fd7a:115c:a1e0::/48`. It does not resolve hostnames or
accept a wildcard or public bind. The client proves possession of its Nostr
device key with a signed kind `27272` event. The event binds the host key,
protocol, grant reference, resume cursor, and a fresh nonce.

After admission, the host sends a snapshot followed by generation-fenced,
monotonically sequenced deltas. A cursor from another generation, a cursor
ahead of the host, or a cursor older than the bounded delta journal receives a
fresh snapshot. A contiguous cursor receives only the missing deltas.

The direct bridge reads the same `Issue31HostController` grant fold and
device-wide revocation scan used by relay admission. Expiry closes a live
session with `grant_expired`. Revocation closes it with `grant_revoked`, and
the same grant is refused on reconnect.

Version 1 is read-only. The only accepted client frames after `hello` are
`heartbeat` and `bye`. Any command or unknown frame receives a typed
`protocol_error` close. Both directions enforce the 64 KiB frame limit.

Discovery v3 adds one to four signed direct endpoints to the existing host
announcement. Each endpoint is a structured MagicDNS name, nonzero port, and
exact `openagents.omega.device_bridge.v1` protocol identifier. It is not an
arbitrary URL: schemes, credentials, paths, uppercase labels, wildcard binds,
and public IP substitutions are refused. The host publishes v3 through the
same owner-identity signer and durable relay outbox used by discovery v2.
Persisted v1 and v2 records remain decodable.

A discovery is usable only from `issuedAt` through (but not including)
`expiresAt`. A higher generation supersedes the prior endpoint set. A
same-generation renewal must keep the endpoint set identical, advance
`issuedAt`, and extend `expiresAt`; changing a port or hostname at the same
generation is a conflict.

The zero-base sidebar exposes **Pair phone** after the direct bridge runtime is
ready. Its QR code contains the MagicDNS endpoint, host public key, bridge
protocol, generation, expiry, and a random one-time secret. The secret is
stored only as a digest, expires after five minutes, and is removed on the first
admission attempt. The phone still signs the bridge hello with its own Nostr
device key, including the secret digest in that proof. A successful scan drives
the ordinary Issue 31 request/challenge/response/scoped-grant engine. It never
exports an owner or Sarah private key and the secret never becomes a reusable
grant.

## Coverage boundary {#coverage-boundary}

The adjunct has exactly four host-owned projections:

| Projection            | Host-owned fact                                              |
| --------------------- | ------------------------------------------------------------ |
| `connection_identity` | Host, pairing, binding, grant, and revocation references     |
| `full_auto_runs`      | Full Auto record and permitted control references            |
| `provider_accounts`   | Provider roster and host-owned connection-handoff references |
| `evidence_chain`      | Bounded evidence, outcome, decision, and receipt references  |

The other issue 31 capabilities remain Nostr-primary. Owner-private Sarah,
memory, read state, reminders, attention targets, community membership,
community work, and experience are not copied into this schema. The mobile
read model joins their signed records with these four projections by stable
reference.

## Nostr host transport {#nostr-host-transport}

Omega's Issue 31 host transport is Nostr-primary. On Sarah bootstrap and room
refresh, the production host publishes a signed parameterized-replaceable
kind `31990` discovery event, queries every configured relay for NIP-59 gift
wraps addressed to the owner key, and locally validates every signature, kind,
author, tag, private sender/recipient binding, schema, and grant before use.
Relay filters are only an optimization. A relay `OK` frame confirms transport;
only a signed `command_result` is terminal command truth.

Pairing and command records use a kind `14` rumor, kind `13` NIP-44 seal, and
kind `1059` NIP-59 gift wrap. The host enforces the complete request,
challenge, response, and scoped-grant chain. Renewal preserves the grant
reference and advances exactly one generation. Revocation is terminal for that
grant reference, including after process restart and replay of old relay
events. Host-admin renewal and revocation are exposed only through the bounded
`sarah_renew_device_grant` and `sarah_revoke_device_grant` methods; there is no
generic remote-authority method.

The host-signed discovery content binds both `hostPublicKeyHex` and the distinct
`sarahPublicKeyHex`. Every host-authored scoped grant, renewal, and revocation
repeats that Sarah key, and the grant fold rejects any lifecycle event that
changes it. Mobile admits Sarah-authored private records only while the active
grant binding matches the current unexpired host-signed discovery. The host key
remains pairing and structured-command authority; the Sarah binding does not
turn Sarah into host/controller authority.

Discovery v2 additionally binds the exact `sarah.<24 hex>` conversation and
advertises both command v1 and command v2. Omega publishes v2 by default while
continuing to decode persisted v1 discovery and commands. A persisted v1 host
state is migrated by binding the configured conversation and replacing its
announcement with a newly signed v2 record; grant lineage and revocation
history are preserved.

Command v2 carries typed, bounded arguments for Sarah send and interrupt,
NIP-RS read-frontier advance, and NIP-ER reminder create/change/complete/cancel.
Its `accepted` result means only that Omega admitted and handled the command;
target completion remains a separate source record. Send and interrupt use the
real Sarah publication path. Read-state writes merge the grow-only max register
before publishing an owner-authored, NIP-44-encrypted kind `30078` record.
Reminder lifecycle writes publish owner-authored, NIP-44-encrypted replaceable
kind `30300` records. Relay acknowledgement is never rewritten as target
completion.

For every active `observe_issue31` grant, Omega projects each admitted owner or
Sarah source record into an `openagents.omega.issue31.owner_projection.v1`
record encrypted to that device. The projection binds the source event ID,
exact author, kind, creation time, role, grant, and generation. Omega decrypts
NIP-AE, NIP-RS, NIP-ER, turn, and authority records only inside identity
custody, then re-encrypts the bounded projection to the device; neither owner
nor Sarah secret keys are exportable. Projection replay state is bounded and
durable, so restart and relay replay cannot duplicate or rebind a source.

Host subscriptions and local admission include NIP-29 group chat kind `9` and
the NIP-LBR request/result/feedback lifecycle kinds `5934`, `6934`, and `7000`.
They are never admitted merely because a relay returned them: signatures and
bounded event shape must verify. NIP-29 records must carry exactly one locally
configured `h` group tag; LBR records must be authored by a locally configured
requester or provider key. Owner, Sarah, device, and relay status do not imply
community authority. Device keys are admitted only as the signer of
schema-validated private pairing and command intents; pre-admission never lets
a device author generic Sarah or community records. Sarah-owned conversation
records retain their conversation binding. NIP-AE, NIP-RS, and NIP-ER instead
use their native author, address, owner, and time-tag contracts, without an
invented `conversation` tag.

Production configuration is read from:

- `OPENAGENTS_OMEGA_NOSTR_RELAYS`: one to eight comma-separated, credential-free
  `ws://` or `wss://` endpoints. Paths are allowed; credentials, URL query
  strings, and fragments are refused, matching the mobile discovery policy.
- `OPENAGENTS_OMEGA_SARAH_PUBLIC_KEY_HEX`: Sarah's lowercase 64-hex Nostr public
  key.
- `OPENAGENTS_OMEGA_NOSTR_DEVICE_PUBLIC_KEYS`: required comma-separated
  allowlist of one to 32 lowercase 64-hex mobile-device Nostr public keys.
  Missing, empty, duplicated, uppercase, or malformed values fail closed before
  Omega exposes the production Sarah transport. Omega challenges pairing
  requests only from this local allowlist, including when the request asks for
  every scope. Bootstrap exposes only uppercase 16-character SHA-256 device
  fingerprints for owner verification; it never exposes or logs the raw
  configuration value.
- `OPENAGENTS_OMEGA_NOSTR_DEVICE_SCOPES`: required comma-separated owner-approved
  scope subset applied to the admitted device keys. Accepted values are
  `observe_issue31`, `send_message`, `interrupt_turn`, `control_full_auto`,
  `request_provider_handoff`, and `act_in_community`. A grant contains only the
  intersection of this policy and the device request; an empty intersection
  receives no challenge. Adding a device key therefore never implicitly grants
  every privileged scope.
- `OPENAGENTS_OMEGA_NOSTR_COMMUNITY_GROUP_IDS`: optional comma-separated local
  admission policy for NIP-29 `h` group identifiers. With no configured group,
  no NIP-29 group record is subscribed to or admitted.
- `OPENAGENTS_OMEGA_NOSTR_COMMUNITY_PUBLIC_KEYS`: optional comma-separated local
  admission policy for lowercase 64-hex NIP-LBR requester/provider public keys.
  With no configured key, no LBR lifecycle record is subscribed to or admitted.
- `OPENAGENTS_OMEGA_SARAH_CONVERSATION_DIGEST`: optional stable conversation
  digest; by default Omega derives 24 characters from the owner public key.
- `OPENAGENTS_OMEGA_DEVICE_BRIDGE_MAGIC_DNS`: optional lowercase MagicDNS name
  advertised in signed discovery v3. It must be configured together with the
  port and bind address below.
- `OPENAGENTS_OMEGA_DEVICE_BRIDGE_PORT`: nonzero TCP port advertised in
  discovery and bound by the direct WebSocket bridge.
- `OPENAGENTS_OMEGA_DEVICE_BRIDGE_BIND_ADDRESS`: literal loopback or Tailscale
  address on the local machine. This value is never advertised; it exists to
  keep listener binding separate from MagicDNS naming. Wildcard, LAN, public,
  and hostname binds fail closed.

The Omega identity must already be ready in channel-specific secure custody.
Missing environment or custody is re-evaluated on the next Sarah host request,
so correcting configuration does not require restarting Omega. Discovery is
refreshed before its 24-hour record approaches expiry. A same-generation
replacement is a renewal only when every binding field is identical, its
`issuedAt` is strictly newer, and its `expiresAt` advances. A binding change at
the same generation remains a conflict. Omega replaces a partially
acknowledged pending announcement atomically and never advances stored expiry
metadata without storing the exact newly signed event.

The operator can issue Sarah's public NIP-OA relay attestation without reading
or exporting the Omega owner secret:

```sh
cargo run -p omega_identity --bin omega-identity -- \
  --channel rc attest-agent \
  --agent-public-key-hex <sarah-public-key-hex>
```

The command reads channel custody through `IdentityService` and returns only a
public `auth` tag plus public identity fields. It refuses self-attestation,
invalid agent keys, control characters, oversized conditions, non-ready
custody, and identity mismatch. The returned tag is suitable for the Sarah
service's public `SARAH_NOSTR_OWNER_AUTH_TAG_JSON` configuration; the owner key
never enters an environment variable, shell substitution, file, or process
output.

The device allowlist is runtime-local admission policy, not Nostr identity or
durable grant history. Omega rebinds it after loading host state. Changing it
controls which new pairing requests receive a challenge without rewriting or
silently invalidating existing grants; owners must use explicit grant
revocation when an already-paired device loses authority.

The Sarah Workroom exposes an owner-facing **Paired devices** section backed by
the bounded `sarah_device_grants` projection. It shows grant reference, hashed
device fingerprint, generation, status, expiry, and scopes without exposing a
raw device key. Active rows provide explicit 24-hour renewal and revocation
buttons wired to the generation-fenced host methods. Relay records cannot
invoke these owner-admin methods.

The exact signed outbox, grant history, terminal command results, normal Sarah
send/interrupt results, run/turn state, message sequence, control-scan cursor,
and idempotency fingerprints are atomically stored under the channel data root at
`openagents/issue31-nostr-host-state.json`. The directory and file are owner
only (`0700` and `0600` on Unix). The bounded document contains public records,
encrypted gift-wrap bytes, and public-safe references; it never contains an
`nsec`, raw private key, bearer token, or provider credential. A malformed,
oversized, symlinked, identity-mismatched, or relay-configuration-mismatched
state file fails closed. Recovery is to preserve the file while restoring the
matching identity/configuration; deliberate re-pairing uses a new grant
reference after owner revocation.

Outbox state, the controller transition, and normal Sarah send/interrupt
idempotency result are committed before the first network publish. One healthy
relay acknowledgement permits host processing to continue, while the exact
signed event and the per-relay acknowledgement set remain durable until every
configured relay has acknowledged it. Lagging relays are retried without
regenerating or resigning the event. Room state reports the actual healthy
relay set and a partial relay is degraded/gapped, not falsely connected.

The effectd process generation is a restart fence and may advance while Omega
is running. It is synchronized only from the already-fenced host request and
cannot move backwards. It is deliberately separate from the stable Nostr host
discovery generation and from each scoped grant generation, so an effectd
restart neither invalidates persisted pairing state nor changes discovery
identity. Transcript and activity pagination likewise keep independent
cursors and limits; neither stream advances the other stream's resume point.
The independent Issue 31 control scan also persists its cursor. Each refresh
drains at most eight 64-record pages; if more remain, Omega advances the cursor
only through records already processed and reports a possible gap instead of
silently abandoning the tail. WebSocket reads use one absolute deadline and a
64-control-frame budget, so ping/pong traffic cannot extend an operation
forever.

One expired, unknown-lineage, or otherwise invalid signed pairing/command
record cannot poison the subscription. Omega writes a bounded quarantine entry
from event ID to a typed public-safe reason reference, commits it before moving
on, continues with later records, and advances the control cursor only through
the scanned page. Transport, signature, custody, and durable-write failures
still fail the whole synchronization.

Legacy v1 Full Auto and community action intents do not yet dispatch into a
generation-fenced real controller. Omega returns typed `unavailable` with
`reason.omega.controller_not_bound` and leaves local state untouched.
Command-v2 Sarah and owner-state actions use their real Nostr paths, but
`accepted` is deliberately non-terminal. Neither a relay acknowledgement nor a
host-side display mutation is reported as completed/stopped execution.

## Provider connection handoffs {#provider-connection-handoffs}

`action.omega.provider_handoff` does dispatch (omega#91). A device holding the
`request_provider_handoff` scope sends a command-v1 intent whose `argumentsRef`
is `arguments.omega.provider_handoff.<provider>`; that prefix is the action's
entire input surface, so there is no wire field through which a device could
state a time, an account, a lane, or an outcome. The host answers by opening a
record in `Issue31ProviderHandoffLedger` and returns `completed` with the new
`handoffRef` as its `outcomeRef` — the record the device then watches. That
`completed` describes the command, not the handoff: the handoff at that moment
is `requested`.

Each record has one lifecycle, and every field on it is a host measurement:

| Field           | How the host measures it                                       |
| --------------- | -------------------------------------------------------------- |
| `requestedAtMs` | One reading of `now`, taken when the host admitted the command |
| `accountRef`    | Chosen from the host's own provider roster, never supplied     |
| `state`         | Advanced by at most one roster observation per host pump pass  |
| `reasonClass`   | Why a non-successful handoff ended that way                    |
| `outcomeRef`    | Present exactly when the handoff is terminal                   |

Terminal outcomes are `outcome.omega.handoff_connected` (a ready account for
that provider, bound to its lane), `outcome.omega.handoff_refused` with
`reason.omega.handoff_account_revoked`, `outcome.omega.handoff_failed` with
`reason.omega.handoff_account_lane_conflict` or
`reason.omega.handoff_account_withdrawn`,
`outcome.omega.handoff_interrupted` with
`reason.omega.handoff_host_restarted`, and
`outcome.omega.handoff_expired` with `reason.omega.handoff_deadline_passed`
after fifteen minutes.

Binding and completing are separate passes on purpose, so a phone observes the
handoff appear, bind, and settle rather than seeing it jump. The roster is read
through the same `parse_provider_accounts` the desktop provider roster renders,
so the account a handoff chose and the lane it serves are the ones Omega shows;
`handoff.accountRef` → `account.laneRef` is the account-to-lane relation
omega#42 asked for, and the contract refuses a handoff naming an account the
snapshot does not carry.

The ledger is committed with the rest of the durable Issue 31 host state. On
load, every handoff still in flight is settled as
`failed`/`reason.omega.handoff_host_restarted`: the isolated provider home and
the login it drove died with the previous process, so the honest terminal
answer is that the handoff was interrupted. That direction can only
under-claim; a restart never reports a connection the host did not make. A row
persisted before `requestedAtMs` existed decodes, is reported unavailable, and
is refused — never stamped at load time, which would give one field two
provenances nothing on the wire distinguishes.

A request the host never admitted — no scope, or an `argumentsRef` that names
no provider — leaves **no record at all**. That is what makes a failed handoff
distinguishable from one that never started: one is a row carrying a
host-owned reason and outcome, the other is an empty list.

A handoff record carries the fact of a connection and never the connection
secret. Nothing in this path reads, writes, or names a provider credential, an
isolated provider home, or a filesystem path, and every projected row is routed
back through `workroom_receipts::decode_issue31_provider_handoff` — the exact
function the whole-document decoder uses — so the host cannot write a handoff
the phone would refuse.

Pairing and command intake uses the same durability order. Omega applies an
inbound record to a cloned controller, signs and inserts any terminal response
into the exact outbox, and atomically commits both before treating the inbound
event as processed. Signing, enqueue, or durable-write failure therefore leaves
the event retryable after restart. Distinct owner sends also bind their
idempotency reference into the rumor tags and durable outbox key, so identical
text submitted as two commands cannot collapse into one Nostr event.

Room snapshots expose an independently paginated `nostrRecords` page for
confirmed NIP-AE, NIP-RS, NIP-ER, NIP-29, and NIP-LBR references. The workroom
renders only bounded event ID, kind, record family, author fingerprint,
timestamp, cursor, source, and gap state. Encrypted or private record content is
never copied into this projection, and an unavailable/gapped source is not
rendered as an empty success.

This split keeps all eleven issue 31 coverage rows visible without creating an
aggregate event or a REST mirror. An unavailable host projection does not hide
an already-confirmed Nostr record.

## Contract states and bounds {#contract-states-and-bounds}

Each projection contains:

- one `omega_host` source reference and observation time
- `current`, `stale`, or `unknown` freshness
- `complete`, `partial`, `missing`, or `unavailable` gap state
- an owner, member, verifier, or observer role and grant state
- at most 16 record references and 16 permitted action references
- one idle, pending, refused, or terminal command state

Millisecond timestamps cannot exceed `8,640,000,000,000,000`, the shared
JavaScript `Date` bound used by the TypeScript decoder.

The decoder requires all four projections once. It rejects duplicate
capabilities and references. `missing` and `unavailable` projections must use
unknown freshness, expose no records or actions, and remain idle. A pending
command requires an active role and a currently permitted action.

All references use the existing public-reference validator and are limited to
256 characters. Unknown fields fail closed. Exact paths, credentials, prompts,
private payloads, and unbounded output cannot enter the decoded projection.
Decoder errors report a reason class and never echo rejected input.

## Shared fixtures {#shared-fixtures}

Rust owns the canonical fixture bytes under
`crates/workroom_receipts/fixtures`. TypeScript mirrors the same four files:

- `openagents.omega.issue31.host.v1.canonical.json`
- `openagents.omega.issue31.host.v1.negative-private-field.json`
- `openagents.omega.issue31.host.v1.negative-unsafe-ref.json`
- `openagents.omega.issue31.host.v1.negative-invalid-state.json`

`openagents.omega.issue31.fullauto.v1.host-produced-handoffs.json` is shared the
same way and is not written by hand: it is exactly what
`Issue31ProviderHandoffLedger` emits when driven through all six lifecycle
states, asserted on the Rust side and digest-pinned on the TypeScript side.

The canonical fixture covers idle, pending, refused, and terminal states. The
negative fixtures prove unknown private fields, unsafe references, and an
incoherent unavailable projection fail closed.

The production Nostr record contract is frozen independently under
`crates/omega_effectd/fixtures` and mirrored byte-for-byte by OpenAgents
Mobile:

| Fixture                                                       | SHA-256                                                            |
| ------------------------------------------------------------- | ------------------------------------------------------------------ |
| `openagents.omega.issue31.host_discovery.v1.canonical.json`   | `f68dab0de93f533a97b59ebac2db7ea28e66a8aef913e108bfb5c1fa74618f6b` |
| `openagents.omega.issue31.pairing.v1.canonical.json`          | `82b36c29d74b3f07ef5d50941532eb038d4f2839d640019b136e013b7dc426fa` |
| `openagents.omega.issue31.command.v1.canonical-intent.json`   | `c990ba250393d271bc4040d2f11fefea909c3bb733e0a33895512c28151c56c2` |
| `openagents.omega.issue31.command.v1.canonical-result.json`   | `89581c5090117eecf766884dd762db99f301b6669a123ca034ff634f6b883ee8` |
| `openagents.omega.issue31.host_discovery.v2.canonical.json`   | `a5604d4c792a5ed556f023e150f01b371c5cf702b95b72786e0c7a9adbbdcb1c` |
| `openagents.omega.issue31.command.v2.canonical-intent.json`   | `7bb7b23680be10756184668ae7722c09c634a1941b086f66d0425da4e8371bbe` |
| `openagents.omega.issue31.command.v2.canonical-result.json`   | `51bca57e14c3d45518c342c2d1f848972281de848f809c34566ed183c7e4e387` |
| `openagents.omega.issue31.owner_projection.v1.canonical.json` | `2a8bec5fa23f27d20db35f3d76bd59817672431328f191bc4302dfa37e7f804d` |

The pairing fixture includes the host-authored Sarah identity binding. A hash
change is therefore a cross-repository contract change, not formatting churn.

## Direct mirror projection {#direct-mirror-projection}

Once a scoped device grant is admitted, the direct WebSocket bridge sends the
current Omega projection as one snapshot and then ordered deltas. The snapshot
contains:

- up to 64 active or recently loaded Agent threads, including title, turn
  state, and the typed executor and model disclosure shown by the desktop;
- up to 64 Full Auto runs with queued, running, paused, completed, failed, or
  cancelled state and up to 32 authority receipt references;
- engine availability, generation, lane readiness, and observation time.

Thread transcripts contain at most 64 user or assistant entries. Each entry is
an 8 KiB UTF-8-safe preview and must pass the same public-text gate as the Sync
projection. System notes, tool-call records, elicitation payloads,
completed-plan internals, compaction records, raw provider payloads,
credentials, and exact private paths are excluded at the desktop projection
boundary. The bridge validates the same count and size limits again before it
advances a cursor, and the encoded snapshot or delta must fit the 64 KiB frame
limit.

Full Auto rows come from `latest_issue31_live_reading`, the same engine
observation used by the Sync projection. The device bridge does not maintain a
second run registry, and GPUI owns neither run authority nor receipt truth.
Thread deltas are produced from the live Agent thread entities; the existing
correlation journal remains only the run-to-thread binding used across a
restart.

A client may resume from a generation and sequence cursor while retained
deltas are contiguous. A gap or generation mismatch returns a canonical
snapshot. When the engine generation changes, Omega replaces the projection,
resets sequence to zero, and makes every old-generation cursor resnapshot.

## Verification {#verification}

```sh
cargo test -p workroom_receipts --lib
cargo test -p omega_identity --lib
cargo test -p omega_effectd --lib
cargo test -p full_auto_ui --lib
cargo test -p agent_ui -p workroom_ui --lib
./script/clippy -p omega_identity
./script/clippy -p omega_effectd
./script/clippy -p agent_ui
./script/clippy -p workroom_ui
./script/clippy -p workroom_receipts
```

Against the deployed relay (omega#91):

```sh
OMEGA_LIVE_RELAY_URL=wss://relay.openagents.com \
  cargo test -p full_auto_ui --lib \
  a_provider_handoff_appears_binds_and_settles_on_a_live_relay -- --ignored --nocapture
OMEGA_LIVE_RELAY_URL=wss://relay.openagents.com \
  cargo test -p full_auto_ui --lib \
  a_refused_handoff_is_distinct_from_one_that_never_started_on_a_live_relay \
  -- --ignored --nocapture
```

## Falsifiers {#falsifiers}

- A GPUI entity or mobile view becomes the record owner.
- The host adjunct duplicates a Sarah or community Nostr record.
- A mobile REST route mirrors the Nostr record.
- A fixture is presented as a connected host source.
- A secret, private payload, exact local path, or raw tool output decodes.
- A provider connection handoff states a time, an account, a lane, or an
  outcome the device supplied.
- A handoff in flight at shutdown is lost, or is reported as connected.
- A handoff row missing its measured request time is shown with one filled in.
- A request the host never admitted produces a handoff record.
