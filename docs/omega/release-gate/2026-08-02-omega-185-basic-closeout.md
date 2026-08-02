# Omega #185 Sarah LiveKit media closeout

- Issue: `OpenAgentsInc/omega#185`
- Acceptance date: 2026-08-02
- Tested source: `989adfac0b35bbc85817b64f9348944ed9195d95`
- Acceptance basis: owner-approved basic testing

## Accepted implementation

Omega's private Sarah voice path supports the server-selected
`livekit_room_v1` transport beneath the existing authenticated control and
settlement channel. The client validates the admitted endpoint, grant, room,
participant, dispatch, presence, session, and generation bindings before it
opens the microphone. It publishes microphone audio only, subscribes only to
the admitted Sarah participant, and rejects stale, unexpected, overlapping, or
replayed media state.

The existing Omega audio path remains the single capture, processing, playback,
mute, interruption, and device owner. Cleanup stops tracks, clears buffers,
disconnects the room, releases devices, and preserves exact settlement on the
authenticated control path. `custom_wss_v1` remains an explicit server-selected
rollback cohort; an active session never switches transport.

The installed client records public-safe selected ICE pair observations with
candidate type, transport protocol, relay protocol, packet counters, and hashed
session/generation bindings. It can distinguish direct UDP, TCP fallback, and
TURN/TLS without retaining candidate addresses, credentials, media, or
transcripts.

## Verification

Fresh focused verification passed on the tested source:

- `omega_effectd` Sarah admission/session/settlement contract: 11 tests;
- `workroom_ui::voice`: 23 tests, including microphone gating, Sarah-only
  subscription, single audio ownership, reconnect fencing, cleanup bindings,
  and ICE classification;
- Omega delta contracts 0212 and 0220: 2 tests.

The prior production headless receipt established a real LiveKit voice turn,
Sarah audio/transcription, interruption, exact provider usage, and terminal
settlement. Under the owner's 2026-08-02 direction, a new packaged multi-device
and forced-network campaign is not required to close #185.

## Release-gate boundary

This closeout does not change the candidate-bound status of the
`sarah-livekit-private` or `sarah-livekit-connectivity` rows. Candidate packaging,
the complete direct UDP/TCP/TURN campaign, and cross-platform device testing
remain release evidence owned by #187; they are not claimed as executed here.
