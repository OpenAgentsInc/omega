# Sarah managed Realtime voice contract

Sarah voice is an owner-private feature of Omega's existing Sarah workroom. It is
independent of collaboration calls: it does not join a LiveKit room, publish a
LiveKit track, or change collaboration mute/deafen state.

## Trust and authentication

Omega resolves its existing OpenAgents native session from the operating
system's credential provider and verifies it with OpenAgents before connecting.
The verified OpenAgents access token is sent only in the `Authorization: Bearer
…` header of a same-origin secure WebSocket request:

```text
wss://openagents.com/api/sarah/realtime
X-OpenAgents-Sarah-Protocol: 1
```

Tokens are never placed in the URL, transcript, logs, or protocol messages.
Omega does not request, store, or transmit an OpenAI API key. The gateway owns
the OpenAI Realtime connection and credit metering. The gateway must reject
unverified OpenAgents sessions and enforce `gpt-realtime-2.1` server-side.

## Session and audio protocol

The first client text frame is:

```json
{
  "type": "session.start",
  "protocolVersion": 1,
  "clientSessionId": "uuid",
  "model": "gpt-realtime-2.1",
  "inputAudio": {
    "encoding": "pcm_s16le",
    "sampleRate": 24000,
    "channels": 1
  },
  "outputAudio": {
    "encoding": "pcm_s16le",
    "sampleRate": 24000,
    "channels": 1
  },
  "editorBridge": {
    "protocolVersion": 1,
    "commands": [
      "read_context",
      "navigate",
      "insert",
      "replace_selection",
      "action",
      "start_agent_thread"
    ],
    "approvedActions": ["undo", "redo", "save_active_file"],
    "confirmationRequiredFor": ["destructive", "external_effect"]
  }
}
```

After `session.start`, client binary frames are microphone PCM and server binary
frames are speaker PCM. Each binary frame contains raw little-endian signed
16-bit mono samples at 24 kHz with no container header. Omega currently emits
20 ms microphone frames. Omega rejects gateway frames larger than 256 KiB and
bounds transcript, identifier, error, context, and edit fields before retaining
or executing them.

The gateway sends these text frames:

```json
{"type":"session.ready","sessionId":"session.ref"}
{"type":"session.state","state":"listening"}
{"type":"session.state","state":"user_speaking"}
{"type":"session.state","state":"sarah_speaking"}
{"type":"transcript.delta","itemId":"item.ref","participant":"sarah","delta":"Hello"}
{"type":"transcript.completed","itemId":"item.ref","participant":"sarah","text":"Hello."}
{"type":"session.ended","reason":"credits_exhausted"}
```

`participant` is `user` or `sarah`. Transcript deltas for the same `itemId`
are append-only until the completed frame replaces them with the final text.

To interrupt Sarah's current spoken response, Omega sends:

```json
{"type":"response.cancel"}
```

Omega also immediately discards queued local playback. To end the session,
Omega sends `{"type":"session.close"}`, closes the WebSocket, stops microphone
capture, and releases the output device. Dropping the Sarah panel performs the
same cleanup. A disconnected session is not silently presented as live; the UI
moves to reconnect-required and starts a new authenticated session only when
the user retries.

Errors use an actionable, public-safe text frame:

```json
{
  "type": "error",
  "message": "Voice credits are exhausted.",
  "retryable": false,
  "action": "Add credits in OpenAgents account settings."
}
```

The gateway must not put secrets or raw upstream errors in `message` or
`action`.

## Editor command bridge

The gateway can send:

```json
{
  "type": "command.request",
  "requestId": "command.ref",
  "command": {"name": "read_context", "maxChars": 8192}
}
```

Omega deserializes `command` into a closed enum. Unknown names, unknown fields,
oversized text, and invalid context limits are rejected without execution.
There is no shell command, URL opener, generic action name, or arbitrary action
or agent dispatch in this protocol.

Allowed commands:

| Command | Payload | Behavior | Confirmation |
| --- | --- | --- | --- |
| `read_context` | `maxChars?: 1..16384` | Returns the active file's relative path, title, zero-based cursor, selected text, and bounded nearby text | No |
| `navigate` | `line`, `column` | Moves the active editor cursor to a clipped zero-based position | No |
| `insert` | `text` up to 64 KiB | Collapses any selection and inserts at the cursor | No |
| `replace_selection` | `text` up to 64 KiB | Replaces the current selection | Yes, destructive |
| `action` | `action` | Runs one approved action | Depends on action |
| `start_agent_thread` | `message`, `presentation` | Creates an Omega Agent thread and submits the message through the normal Agent composer/send path | Yes, external effect |

Approved actions are `undo`, `redo`, and `save_active_file`. Undo and redo
require destructive confirmation. Saving requires external-effect
confirmation. Each confirmation is one-shot and visible in the Sarah panel;
the gateway cannot pre-approve it.

`start_agent_thread` accepts a non-empty message of at most 16 KiB and exactly
one of two presentation values:

- `foreground`: create and select the thread, reveal the normal Omega Agent
  panel, and focus it.
- `background`: create and submit a retained thread without changing the active
  Agent thread, showing or switching panels, or moving focus.

The command does not accept an agent ID, model, tool list, action, worktree, or
other dispatch target. Omega always uses its native Omega Agent route and the
existing Agent authorization. Before either presentation mode runs, the Sarah
panel shows the complete bounded message and requires **Allow once**. After
creation, the Sarah panel retains the thread ID, submission status, presentation
mode, and an explicit **Open Agent thread** button.

Example foreground request:

```json
{
  "type": "command.request",
  "requestId": "command.agent.ref",
  "command": {
    "name": "start_agent_thread",
    "message": "Investigate the failing test and propose a fix.",
    "presentation": "foreground"
  }
}
```

A completed result identifies the thread without exposing internal credentials:

```json
{
  "type": "command.result",
  "requestId": "command.agent.ref",
  "status": "completed",
  "output": {
    "threadId": "00000000-0000-0000-0000-000000000000",
    "presentation": "foreground",
    "status": "submitted"
  }
}
```

Omega replies:

```json
{
  "type": "command.result",
  "requestId": "command.ref",
  "status": "completed",
  "output": {}
}
```

`status` is `completed`, `rejected`, or `failed`. Rejected and failed results
can contain a public-safe `message`. Context output is included only for a
completed `read_context`.

## Manual verification

1. Sign in to OpenAgents through Omega's existing account flow.
2. In Settings → Collaboration, select and test an input and output device.
3. Open Agent Panel → New Thread menu → Sarah.
4. In Sarah's owner-private room, choose **Start voice**. Confirm the lifecycle
   reaches “Sarah is listening” and a managed-session reference appears.
5. Speak and verify the user transcript appears. Let Sarah respond and verify
   audio playback and Sarah's transcript.
6. Toggle **Mute**, speak, and verify no new microphone audio is accepted.
   Unmute and verify input resumes.
7. While Sarah is speaking, choose **Interrupt speech** and verify playback
   stops immediately.
8. Ask Sarah to navigate and insert text. Verify those allowlisted operations
   run without a prompt. Ask Sarah to replace selected text, undo, redo, or save
   and verify the one-shot Allow/Decline card appears before execution.
9. Ask Sarah to start an Agent thread in `background` mode. Verify the full
   message is visible before approval, decline once and confirm no thread is
   created, then approve and confirm the Sarah panel remains visible and focused.
   Verify the created-thread card reports `submitted`, then use **Open Agent
   thread** and confirm the message appears in the normal Agent UI.
10. Repeat with `foreground` and verify approval creates, selects, and reveals
    the new Agent thread. Add `agent` or `model` to a gateway fixture and verify
    the request is rejected rather than dispatched.
11. Have a test gateway send an unknown command such as `run_shell`; verify the
   editor is unchanged and the gateway receives a rejected command result.
12. Choose **End voice** and verify the mic indicator and playback stop. Disable
    networking during another session, verify reconnect-required appears, then
    restore networking and use **Retry**.
13. Deny Omega microphone permission (or select a missing device) and verify the
    panel offers actionable permission/device guidance and a link to
    Collaboration settings.
