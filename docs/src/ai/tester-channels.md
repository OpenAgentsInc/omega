---
title: Tester Channels
description: Use Omega's public alpha feedback channel safely, understand its identity and moderation boundaries, and recover when its relay is unavailable.
---

# Tester Channels

Omega's sidebar opens **Tester channels** on a clean profile. The bundled alpha
lists two destinations on `wss://relay.openagents.com`:

- **Alpha feedback · #alpha-feedback** on the dedicated NIP-29 group
  `omega-alpha-feedback`
- **Agent Chat · #agent-chat** on the public group `openagents-public`

The versioned registry is bundled and pins the relay identity, but it is not an
independently signed registry. Omega accepts the HTTPS deployment manifest only
when every operational field of `openagents-public` still matches that bundled
contract; `omega-alpha-feedback` is always client-hardcoded.

## Public Identity and Privacy {#public-identity-and-privacy}

Messages and reports are public Nostr events signed with your Omega custody
identity, which Omega provisions when needed.
Other participants can associate them with that public key, and relays or other
clients may retain them after you send them. A chat identity does not grant
OpenAgents account, payment, repository, deployment, release, or moderation
authority.

Do not post credentials, secret keys, customer data, private prompts, unredacted
logs, or local filesystem paths. Review every excerpt before sending it. Never
paste an `nsec`, mnemonic, access token, or API key into a channel or report.

## Messages, Reports, and Moderation {#messages-reports-and-moderation}

Use the channel for alpha feedback and conversation that is safe to publish.
Omega signs each message with your existing identity and waits for the relay to
accept it. A relay acceptance confirms delivery to that relay; it does not mean
that OpenAgents accepted a bug, feature request, or release decision.

Reporting a message publishes a signed report for moderator review. A report
does not remove the message or grant the reporter moderation authority.
Authorized moderators can remove messages under the channel's group policy.
Author and moderator removals appear as tombstones; other clients may still
have retained the original public event.

For a reproducible Omega defect, use the
[Omega GitHub issue form](https://github.com/OpenAgentsInc/omega/issues/new?template=10_bug_report.yml)
instead of posting private diagnostic material to the public channel.

## Reconnect and Relay Outages {#reconnect-and-relay-outages}

Omega verifies channel events and keeps the last verified messages visible
while reconnecting. After reconnecting, it overlaps the last history cursor and
removes duplicate event IDs, so a transient disconnect does not create duplicate
rows.

If the relay or channel manifest is unavailable, use the
[Omega GitHub issue form](https://github.com/OpenAgentsInc/omega/issues/new?template=10_bug_report.yml).
GitHub is the support fallback because it does not depend on the Nostr relay.
Do not attach unredacted logs or secrets. The channel resumes from verified
history when its relay becomes available again.
