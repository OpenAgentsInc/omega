# Sarah managed Realtime voice contract

Sarah voice is an alpha-cohort feature reached through the visible Sarah
admission surface inside Agent Panel. The hidden Sarah workroom remains the
runtime owner during the transition, but it is not a public navigation
destination. It is independent of collaboration calls. It does not join a
LiveKit room, publish a LiveKit track, or change collaboration mute or deafen
state.

Opening Sarah from the new-conversation row, Thread menu, or toolbar `+` menu
only opens admission. The composer microphone control follows the same route
when voice is idle, unavailable, or needs retry; it never starts or retries
audio directly. Before **Start voice** is enabled, the service-backed projection
must disclose `admissionCohortRef`, the effective rate in msat per million
tokens, credit hold, remaining credit, maximum duration, transcript policy,
bounded capabilities, and confirmation class. The admitted cohort is
`alpha_v1` or the explicit owner staging cohort;
`cohort_inactive` remains a refusal regardless of account credit. Missing rate
fields are an unavailable projection, not zero and not a desktop estimate. A
null remaining balance for `sarah_voice_cohort:staging_owner_v1` renders as a non-metered owner
entitlement; it is not coerced to zero. A settled projection names the final
charge, optional remaining credit, receipt reference when returned, and
transcript recovery result.

While voice is active, Agent Panel shows the newest 100 attributed transcript
rows. Any command that requires confirmation appears there with its exact
request reference, confirmation copy, optional detail, and **Allow once** and
**Decline** controls. A selection replacement also shows the exact target path,
document version, 1-based selection range, selected text, and replacement text.
A voice-created Omega Agent thread appears as a receipt
with its thread reference, foreground or background presentation, and current
status. These artifacts remain visible with settlement; the hidden workroom
owns the runtime state but is not required to complete the visible journey.

## Trust and authentication

Omega first verifies and reuses the normal OpenAgents bearer session in the
operating system credential provider. It requests admission with:

```text
POST https://openagents.com/api/omega/sarah/voice/admission
Authorization: Bearer <short-lived OpenAgents session>
x-openagents-omega-device-ref: <stable Omega device reference>
Content-Type: application/json
```

The admission request uses schema `openagents.sarah.voice.admission.v1`, client
profile `omega_editor`, and no `auth` member. It neither creates a voice
session nor reserves credit. If the bearer is missing or rejected, Omega
automatically uses its existing local Nostr identity. This fallback does not
create, adopt, replace, show, or export an identity. Custody must already be
`Ready`. `Absent`, `Unadopted`, locked, lost, incomplete, and conflict states
produce distinct actionable errors in the Sarah panel.

Omega first sends:

```text
POST https://openagents.com/api/omega/sarah/voice/auth/challenge
Content-Type: application/json

{"schema":"openagents.sarah.voice.auth-challenge.v1","deviceRef":"<device>","pubkey":"<lowercase 64-character hex pubkey>"}
```

The successful `201` response is `Cache-Control: no-store`:

```json
{
  "schema": "openagents.sarah.voice.auth-challenge.v1",
  "challenge": "<base64url nonce>",
  "expiresAtMs": 0,
  "ownerRef": "<canonical OpenAgents user ID>"
}
```

The challenge expires after at most 120 seconds. Omega validates its bounds and
copies `ownerRef`; it never guesses the account mapping. The server binds the
one-use challenge to the lowercase pubkey, device reference, owner reference,
and expiry.

Omega serializes this final request exactly once:

```json
{
  "schema": "openagents.sarah.voice.admission.v1",
  "identity": {
    "ownerRef": "<ownerRef from challenge>",
    "deviceRef": "<same device>",
    "threadRef": "sarah-owner-private",
    "sessionRef": "<fresh client reference>",
    "generation": 1
  },
  "disclosureRef": "omega.voice.disclosure.v1",
  "clientProfile": "omega_editor",
  "auth": {
    "method": "nostr_nip98",
    "challenge": "<server nonce>"
  }
}
```

It signs the exact UTF-8 bytes as a NIP-98 event and sends those same bytes:

```text
POST https://openagents.com/api/omega/sarah/voice/admission
Authorization: Nostr <base64 JSON event>
x-openagents-omega-device-ref: <same device>
Content-Type: application/json
```

The event has kind `27235`, empty content, a current integer `created_at`, and
exactly one each of:

- `u`: the full admission URL.
- `method`: `POST`.
- `payload`: lowercase SHA-256 of the exact request bytes.

Omega uses NIP-98, not NIP-42. The OS credential boundary performs signing.
The private key and any permanent OpenAI key never enter the request, UI, logs,
or Sarah protocol.

The successful response contains the exact server-authoritative admission
terms, a random one-use `admissionRef`, and `admissionExpiresAtMs` no more than
120 seconds in the future. Omega rejects malformed or overlong references and
expired or implausibly long admissions. A Nostr-authenticated response also
contains:

```json
{"auth":{"method":"nostr_nip98","accessToken":"oa_omega_...","expiresIn":900}}
```

Omega stores that bearer through the existing OpenAgents credential provider.
The Nostr-issued credential record does not store `ownerRef` or another mapped
account identifier. Normal bearer verification resolves the owner in memory
when a later voice request needs it. Existing legacy bearer credentials keep
their compatible owner binding. Disconnect and session verification retain
their existing behavior.

Only after the admitted user chooses **Start voice** does Omega make the normal
bearer-authenticated session request. It sends the reviewed `admissionRef` with
the same identity. The server consumes that reference in the credit-reservation
transaction, so it cannot be replayed and a changed cohort, price, balance,
hold, duration, profile, capability boundary, or identity fails closed. An
expired or rejected admission returns the user to admission review rather than
silently creating a session. This separate request returns the one-use
WebSocket ticket.

## Voice session and WebSocket

The session response uses schema `openagents.sarah.voice.v1` and contains
`sessionRef`, `gatewayUrl`, a one-use `ticket`, ticket/session expiries, credit
reservation, maximum duration, model, fixed input/output audio formats, and a
full echo of the consumed admission reference, expiry, cohort, credit mode,
rate, exact pre-hold spendable balance, and capability boundary. Omega compares
that echo with every term shown at review and refuses the ticket if any field
differs. It also requires `gpt-realtime-2.1`, 24 kHz mono PCM16, a same-origin
`wss` URL, and a ticket that has not expired.

Omega opens `gatewayUrl` with:

```text
x-openagents-sarah-voice-session: <sessionRef>
x-openagents-sarah-voice-ticket: <ticket>
```

No bearer, Nostr proof, or OpenAI credential is sent on the WebSocket. The
first control frame is sequence zero:

```json
{
  "schema": "openagents.sarah.voice.v1",
  "identity": {
    "ownerRef": "...",
    "deviceRef": "...",
    "threadRef": "...",
    "sessionRef": "...",
    "generation": 1
  },
  "sequence": 0,
  "_tag": "session_hello",
  "disclosureRef": "omega.voice.disclosure.v1"
}
```

Control frames carry the exact identity and contiguous sequence numbers. Omega
rejects identity changes, sequence gaps, oversized fields, invalid tool
digests, and unknown tagged variants.

Audio uses the `OAA1` media envelope. Each binary frame contains:

1. Four ASCII bytes `OAA1`.
2. A four-byte big-endian JSON-header length.
3. A bounded `openagents.audio.v1` JSON header.
4. Raw little-endian signed 16-bit mono PCM at 24 kHz.

The header binds identity, independent audio sequence, direction, format,
payload length, and lowercase SHA-256. Omega validates all fields and the digest
before playback. Microphone frames use the same envelope before transmission.

Mute stops new microphone content. Interrupt discards queued playback and sends
an `interrupt` control. End sends a `close` control, closes the socket, stops
capture, and releases playback. Dropping the panel performs the same cleanup.

## Editor command bridge

The gateway and Omega both decode a closed command enum. Unknown variants,
unknown fields, absolute or parent-traversing paths, oversized ranges, and
oversized text are rejected. There is no shell, URL, terminal, Git, generic
action, arbitrary agent, or arbitrary model dispatch.

The managed gateway commands map to Omega's bounded local bridge:

| Gateway command | Local behavior | Confirmation |
| --- | --- | --- |
| `context_read` | Reads bounded context only when the target path is the active editor | No |
| `reveal_range` | Reveals a bounded position only when the target path is active | No |
| `replace_selection` | Replaces the current selection in the exact active target | Yes |
| `save_document` | Saves the exact active target | Yes |
| `start_agent_thread` | Creates a native Omega Agent thread and submits a bounded message | Yes |

`context_read` returns `workspaceRef` and `documentVersion` with the active
path so a later selection replacement can name the editor state it observed.
Omega accepts `replace_selection` proposals only for
`workspace.omega.supervised`, the exact active workspace-relative path, and an
exact current document version. It captures the selected range and full
bounded selected text with the proposed replacement before showing **Allow
once**.

`open_path` is rejected by the local bridge until Omega has a separately
allowlisted file-opening implementation. The older local enum still prevents
arbitrary commands if a non-managed test gateway is used.

`start_agent_thread` accepts a non-empty message of at most 16 KiB and one
presentation:

- `foreground` creates, selects, reveals, and focuses the normal Agent thread.
- `background` creates and submits a retained thread without changing the
  active view, panel, or focus.

The command accepts no agent ID, model, tool list, worktree, or action name.
Omega always uses the native Agent route and existing authorization. The Sarah
panel shows the complete message and requires a visible one-shot confirmation.
It then reports the thread ID and status and offers **Open Agent thread**.

The gateway binds each proposal to a server digest. For a protected command,
Omega shows the proposal, sends the visible one-shot choice as `tool_decision`,
and waits. It performs the action only after the gateway returns the matching
`tool_execute`; it then returns the matching `tool_outcome`. The command,
proposal reference, digest, target, and expiry must remain unchanged across
both phases. Selection replacement additionally carries the locally captured
workspace, path, document version, range, selected text, and replacement text
through approval and `tool_execute`. Omega checks that binding when **Allow
once** is pressed and again inside the editor update that performs the edit. A
workspace, active-file, document-version, range, or selected-text change is a
refusal and does not edit the buffer. Transcript text cannot confirm a command.

## Manual verification

1. With an existing OpenAgents session, open Sarah and verify the bearer
   admission request discloses exact terms without creating a session or credit
   reservation. Choose **Start voice**, then verify the bearer session request,
   one-use WebSocket ticket, listening state, transcript, and playback.
2. Expire or remove only the OpenAgents session while leaving an already
   adopted Omega Nostr identity in `Ready`. Open Sarah and verify challenge,
   signed admission, secure bearer storage, and exact terms appear without a
   login prompt. Choose **Start voice** and verify the ordinary bearer session
   request and WebSocket connection.
3. Inspect the signed fixture: the `u`, `method`, and `payload` tags occur once;
   the payload digest matches the exact transmitted JSON; no NIP-42 event,
   private key, bearer URL parameter, or OpenAI key is present.
4. Replay the challenge or proof and verify the server returns `409`; retry and
   verify Omega obtains a fresh challenge and proof.
5. Test `Absent` and `Unadopted` custody. Verify Sarah explains setup or adoption
   and does not create or adopt an identity. Test locked/lost/conflict states
   and verify the state is actionable and contains no key material.
6. Select and test input/output devices. Verify mute, interrupt, end, network
   loss, retry, microphone denial, and device removal clean up capture and
   playback and show actionable state.
7. Exercise context, reveal, replace, and save against the active path. For a
   replacement, verify the confirmation shows the exact path, range, selected
   text, and replacement. Separately change the active file, edit the document,
   and move or alter the selection between proposal and execution; verify each
   stale effect is rejected without an edit.
8. Request a background Agent thread. Decline once and verify no thread exists;
   approve once and verify focus does not move, status is shown, and **Open
   Agent thread** reveals it later. Verify the thread is created only after the
   matching `tool_execute`, then repeat with foreground mode.
9. Add `agent`, `model`, or an unknown command such as `run_shell` to a fixture
   and verify decoding rejects it before dispatch.
