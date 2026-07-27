# Omega community workspace

Omega can attach a thread to an invite-only OpenAgents Forge repository
audience. GitHub remains the authoritative development forge; this feature is
the signed conversation room around that repository.

The control surface is the Agent conversation:

- `/community join <invitation>` admits the Forge invitation and adds its
  audience to the composer.
- `/community status` shows joined rooms, delivery state, and recent verified
  messages.
- `/community who` shows identities observed through verified room records.
- `/community post <message>` posts to the current thread's audience.
- `/community leave` leaves the current thread's community audience.

Local remains the default. Selecting a community audience affects the next
thread opened; it does not retarget an existing thread.

## Message path

Authorization happens before any signing or network effect. Omega checks that
the thread has a known non-local audience, that it matches the Forge repository,
and that the invitation's current role admits writes. It then:

1. creates a kind `1111` NIP-22 message bound to the repository's NIP-34
   coordinate;
2. signs those exact bytes with the profile's Omega identity;
3. verifies the returned signature and repository binding;
4. persists the signed event in the outbox;
5. publishes through a bounded websocket transport, answering NIP-42 relay
   challenges with the same Omega identity.

Retries resend the original signed event. Duplicate relay acknowledgements are
idempotent. Retryable failures stop after five attempts and remain visible;
terminal and delivered records are also retained until explicitly handled.

Incoming relay records are accepted only after their signature and repository
binding verify. Omega deduplicates them by event id, bounds the local cache, and
persists the verified cache for restart and replay.

## Failure behavior

- A local thread is refused rather than redirected.
- A missing, malformed, or different repository binding is refused.
- A signer returning different bytes or an invalid signature is refused.
- An unavailable relay leaves the durable outbox visibly pending, then visibly
  stopped after bounded retries.
- Leaving a room does not relabel its old threads as private. Their recorded
  audience becomes unresolved.
- A profile with no invitations performs no community network work and sees
  Local alone.
