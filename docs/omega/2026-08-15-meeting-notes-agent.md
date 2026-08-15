# Meeting Notes Agent

- Status: feature request for owner and team review
- Date: 2026-08-15

## 1. Purpose

This note requests a product capability: a first-party Meeting Notes Agent in
Omega that can attend supported calls, capture meeting content, and turn it
into transcript-backed notes, summaries, decisions, action items, and
follow-ups, saved inside Omega for later reuse. The capability is comparable
in outcome to products such as Fireflies and Otter, while remaining
realistically phasable across meeting environments. The request is
deliberately implementation-agnostic: it states what the user should be able
to do and what a strong first version should produce, and does not prescribe
SDK choices, browser automation, media architecture, recording design, or
storage design. Decisions rest with the owners; every consequential claim
below carries an [E], [D], or [P] tag.

## 2. Claim categories

- `[E]` Existing system behavior
- `[D]` Existing documented direction
- `[P]` Proposed future direction

[Taxonomy rule 4](taxonomy.md), "Say what is true now, not what is planned,"
is why every consequential claim is tagged. This note is a proposal, so
proposed direction is always labeled as such and verified absences are marked.

## 3. The gap

Omega has no meeting-participation or meeting-notes capability today ([E]
verified absence). Omega has a realtime voice conversation mode, Sarah, which
joins an owner-private media room for a one-to-one voice session with a
disclosed transcript policy ([E]; docs/omega/sarah-realtime-voice.md), and
it has community room media support with a verified participant roster ([E];
crates/agent_ui/src/omega_public_channel_livekit.rs). Both are conversation
surfaces between the user and the agent; neither attends a third-party
meeting, captures an external call, or produces meeting notes ([E]).

What is missing is a distinct capability: an agent that can join or attend a
supported meeting environment as a notes participant, capture what was said,
and return structured, reusable meeting outputs to the user inside Omega.

## 4. Requested capability

[P] The Meeting Notes Agent should, over time, be able to:

- join supported meetings automatically or on command
- participate in or attend supported calls as a notes agent
- capture meeting content from supported platforms
- generate transcript-backed notes
- produce summaries, decisions, action items, and follow-ups
- save meeting outputs into Omega for later reuse
- make meeting knowledge searchable and operationally useful

This is not just meeting transcription. It is meeting intelligence and
workflow capture: the value is the structured, reusable record produced from
the meeting, not the raw transcript alone.

## 5. Platform phasing

[P] The team should be free to roll this out in stages across meeting
environments rather than all at once. Candidate environments, each phased
separately, include:

- Zoom
- Google Meet
- Jitsi
- other supported meeting platforms over time

This note deliberately does not prescribe how any platform integration is
built. It only states the outcome each supported environment should produce:
when a meeting is captured, the same class of notes, summaries, decisions, and
action items should come out of it, with platform-specific availability made
clear to the user before a meeting is joined.

## 6. Meeting outcomes

[P] For a captured meeting, the agent should produce, at a strong level of
quality:

- a transcript-backed summary of what was discussed
- the decisions made, with who decided and when where the meeting supports it
- action items and follow-ups, with the owner and due date where stated
- any commitments or open questions
- a saved record the user can reopen, search, and quote later

The notes should be traceable to the meeting content they come from, and
where a detail cannot be confirmed from the meeting, the notes should say so
rather than invent it.

## 7. Meeting knowledge in Omega

[P] Saved meeting outputs should behave like the other reusable knowledge
objects in Omega: discoverable, searchable, and usable in later work. A
meeting record should be reusable as context in future threads, briefs, and
workflows, so meeting knowledge compounds instead of living in a separate
transcript file the user never returns to. Action items and follow-ups from
meetings should be findable when the user is planning the next session or
building follow-up work.

## 8. Why this matters

[P] Meeting capture and notes would support the situations Omega users
already operate in:

- internal team meetings
- founder calls
- partner calls
- research interviews
- onboarding sessions
- product coordination
- operations follow-up

Meeting notes become much more valuable when they are transformed into
searchable internal records, action items, follow-up tasks, and reusable
context for future Omega workflows. A meeting that produces an actionable,
findable record in Omega is no longer an event the team must remember; it is
an asset the workflow can build on.

## 9. What a strong first version should enable

[P] A strong first version should let a user:

- start or schedule a notes agent session for a supported meeting
- receive a transcript-backed summary when the meeting ends
- see decisions and action items extracted from the meeting
- save the meeting record into Omega
- search past meeting records later
- reuse a meeting record as context in a later thread

The measure of the first version is user outcome, not mechanism: the user
walks out of a supported meeting and the useful, structured record is already
in Omega, with the raw transcript available underneath it.

## 10. Non-goals / out of scope

This request does not propose:

- any SDK, browser-automation, media-architecture, or storage design
- any specific meeting-platform integration mechanics
- any transcription engine or model choice
- any change to Sarah's existing voice-conversation surface or contract ([E])
- any change to community room media behavior ([E])

It is a capability request, not a build plan, and it explicitly allows the
team to phase platform support sensibly.

## 11. Privacy, consent, and recording notes

[P] Product-level considerations for the team to weigh, not decisions made
here: joining and recording meetings involves consent and privacy norms that
vary by environment and jurisdiction, so the user should control when the
agent joins and what is captured; sensitive or confidential meeting content
should follow Omega's local data posture and not be shared or synced without
the user's intent; and the agent's presence in a meeting should be visible to
other participants in a way that matches how the meeting environment treats
recording and bots. These are acknowledged as first-class product concerns,
not implementation details.

## 12. Open design questions

1. Which meeting environment should be the first supported platform, and how
   is phased support sequenced and communicated? [P]
2. Should a meeting record be one knowledge object (summary, decisions,
   action items, transcript) or several related records? [P]
3. How are action items and follow-ups turned into actionable work in Omega —
   as searchable records only, or as items the user can convert into tasks? [P]
4. Should the notes agent be able to speak or ask questions in a meeting, or
   is attendance-and-capture the first-version scope? [P]
5. How is recording consent handled per meeting environment in the first
   version? [P]

## 13. Related documents and evidence

- [taxonomy.md](taxonomy.md): glossary and naming rules; rule 4 is the claim
  discipline this note follows
- [sarah-realtime-voice.md](sarah-realtime-voice.md): the existing Sarah
  realtime voice contract; owner-private media room and transcript policy [E]
- [../../crates/agent_ui/src/omega_public_channel_livekit.rs](../../crates/agent_ui/src/omega_public_channel_livekit.rs):
  existing community room media support with verified participant roster [E]
- [PRODUCT.md](../../PRODUCT.md): Omega's product shape, users, and design
  principles
