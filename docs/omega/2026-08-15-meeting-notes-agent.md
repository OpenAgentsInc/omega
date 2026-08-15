# Meeting Notes Agent and In-Person / Hybrid Meeting Copilot

- Status: feature request for owner and team review
- Date: 2026-08-15

## 1. Purpose

This note requests a product capability: a first-party Meeting Notes Agent
and In-Person / Hybrid Meeting Copilot in Omega that can attend a
conversation, capture it responsibly, organize it into useful work, and
participate when invited. The capability covers supported virtual meeting
applications, a room around a shared device, and hybrid sessions that include
both in-person and remote participants. It is comparable in outcome to
products such as Fireflies and Otter for virtual calls, while going further:
in-person and hybrid sessions are equal first-class contexts, and the agent
has explicit, host-controlled roles for live interaction. The request is
deliberately implementation-agnostic: it states what the user should be able
to do and what a strong first version should produce, and does not prescribe
SDK choices, browser automation, media architecture, recording design, audio
stacks, transcription providers, or storage design. Decisions rest with the
owners; every consequential claim below carries an [E], [D], or [P] tag.

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
meeting, captures an external call or an in-room discussion, or produces
meeting notes ([E]).

What is missing is a distinct capability: an agent that can support the
conversations a person actually has — virtual calls, a room around a shared
device, and hybrid sessions — capture them responsibly, turn them into
structured, reusable work, and participate when the meeting host invites it.

## 4. Requested capability

[P] The Meeting Notes Agent and In-Person / Hybrid Meeting Copilot should,
over time, be able to:

- join supported virtual meetings automatically or on command
- start an in-person room session from an available device
- capture meeting content from supported platforms and from in-room
  conversation
- generate transcript-backed notes
- produce summaries, decisions, action items, owners, follow-ups, assumptions,
  open questions, and research items
- save meeting outputs into Omega for later reuse
- make meeting knowledge searchable and operationally useful
- participate in live conversation only according to a host-controlled
  participation mode

This is not just meeting transcription or meeting summarization. It is
meeting intelligence and workflow capture: the value is the structured,
reusable record produced from the meeting, plus a controlled in-room and
in-call presence that makes the meeting itself more productive.

## 5. Meeting contexts

[P] The capability should support the three contexts where conversations
happen:

- supported virtual meeting applications
- an in-person room around a shared device
- hybrid sessions that include both in-person and remote participants

Virtual meeting support may be phased across environments (section 6).
In-person room mode (section 7) and hybrid capture (section 8) are product
contexts of equal standing, not follow-ons to virtual support.

## 6. Platform phasing

[P] The team should be free to roll out virtual support in stages across
meeting environments rather than all at once. Candidate environments, each
phased separately, include:

- Zoom
- Google Meet
- Jitsi
- other supported meeting platforms over time

This note deliberately does not prescribe how any platform integration is
built. It only states the outcome each supported environment should produce:
when a meeting is captured, the same class of notes, summaries, decisions, and
action items should come out of it, with platform-specific availability made
clear to the user before a meeting is joined.

## 7. In-person and room mode

[P] Omega should support an in-person meeting mode where a user can start an
Omega-connected session from an available device and use Omega as a visible,
consent-based room participant. The requested user outcomes are:

- capture in-person discussion
- generate a live or post-meeting transcript
- create structured notes
- identify decisions, action items, owners, follow-ups, assumptions, open
  questions, and research items
- preserve the meeting as a searchable internal record
- allow meeting outputs to become tasks, briefs, prompts, follow-up
  materials, or other Omega workflows

The mode is a room participant with a visible presence, not a hidden
listener: people in the room should know Omega is there and what it is doing.

## 8. Hybrid meetings

[P] Omega should support the product concept of hybrid meeting capture,
creating a unified record of the full conversation:

- room discussion
- remote participant discussion
- decisions and action items across the whole session

The user outcome is one coherent meeting record even though the conversation
happened across two contexts. This note deliberately does not prescribe how
the technical capture works; it states the unified outcome the product
should create.

## 9. Meeting outcomes

[P] For a captured meeting, the agent should produce, at a strong level of
quality:

- a transcript-backed summary of what was discussed
- the decisions made, with who decided and when where the meeting supports it
- action items and follow-ups, with the owner and due date where stated
- assumptions, open questions, and research items surfaced from the
  conversation
- a saved record the user can reopen, search, and quote later

The notes should be traceable to the meeting content they come from, and
where a detail cannot be confirmed from the meeting, the notes should say so
rather than invent it. Speaker attribution is an aid to reading the record,
not a guarantee: where the meeting does not support reliable attribution,
notes should be honest about that.

## 10. Meeting knowledge in Omega

[P] Saved meeting outputs should behave like the other reusable knowledge
objects in Omega: discoverable, searchable, and usable in later work. A
meeting record should be reusable as context in future threads, briefs, and
workflows, so meeting knowledge compounds instead of living in a separate
transcript file the user never returns to. Action items and follow-ups from
meetings should be findable when the user is planning the next session or
building follow-up work, and meeting outputs should be convertible into
tasks, briefs, prompts, follow-up materials, or other Omega workflows.

## 11. Host-controlled participation modes

[P] Omega should participate in live conversation only according to an
explicit participation mode selected or controlled by the meeting host.
Requested modes:

- **Silent recorder** — Omega captures and structures the conversation
  without speaking
- **Facilitator on request** — Omega responds only when directly addressed
- **Research copilot** — Omega retrieves approved internal or public
  information when asked
- **Structured brainstorm copilot** — Omega helps recap, organize ideas,
  identify contradictions, track open questions, and surface decisions when
  invited
- **Active participant** — Omega may contribute under explicit facilitator
  control

The default mode should be **Silent recorder**. The meeting host should be
able to promote, mute, pause, or stop Omega during the session. Participation
is an opt-in, host-controlled capability, not a default autonomy.

## 12. Voice commands and interactions

[P] The product should enable natural voice interactions such as:

- "Omega, mark that as a decision."
- "Omega, capture that as an action item."
- "Omega, assign that action item to me."
- "Omega, add that to the parking lot."
- "Omega, summarize what we have agreed so far."
- "Omega, list the open questions."
- "Omega, research that claim."
- "Omega, compare this idea to our existing roadmap."
- "Omega, prepare a follow-up brief from this meeting."

These examples are product illustrations of the user outcomes to enable, not
a technical command specification. The exact command grammar, wake behavior,
and interaction surface are left to the team.

## 13. Meeting-time research

[P] Omega should function as a controlled research copilot during approved
sessions. When invited, Omega should be able to:

- search approved public sources
- access authorized internal project context
- retrieve prior meeting notes, decisions, and relevant prompts
- compare proposals with existing roadmap context
- provide concise answers
- clearly distinguish known information from uncertainty
- attach useful findings to the meeting record

Omega must not take external actions, send messages, create external
commitments, or make consequential decisions without explicit authorization.
Research participation is scoped to what the host and the per-meeting
knowledge controls allow (section 15).

## 14. Why this matters

[P] Meeting capture and notes would support the situations Omega users
already operate in:

- internal team meetings
- founder working sessions
- in-person brainstorming
- hybrid planning meetings
- partner calls
- research interviews
- onboarding sessions
- product planning
- operations coordination

Meeting notes become much more valuable when they are transformed into
searchable internal records, action items, follow-up tasks, and reusable
context for future Omega workflows. A meeting that produces an actionable,
findable record in Omega is no longer an event the team must remember; it is
an asset the workflow can build on. In-person and hybrid sessions are where a
large share of real working conversations happen, so capture there materially
extends the same value to the full range of a team's meetings.

## 15. Consent, privacy, and control

[P] This capability must be consent-first. At the product level, the request
is for:

- visible recording and listening status
- clear meeting-start disclosure that Omega is present
- host controls for the Silent, On-Request, Research, Structured Brainstorm,
  and Active Participant modes
- accessible mute, pause, and stop controls
- per-meeting controls for what internal and external knowledge sources Omega
  may access
- controls for meeting retention, export, access, and deletion
- participant identity and speaker-label correction where needed
- no autonomous external action without explicit authorization

These are clear product expectations, not a detailed privacy or compliance
design. The intent is that Omega's presence, capture, and participation are
always visible to and controllable by the people in the conversation, and
that meeting knowledge follows Omega's local data posture and the user's
control rather than being shared or synced without intent.

## 16. What a strong first version should enable

[P] A strong first version should let a user:

- start or schedule a notes agent session for a supported virtual meeting
- start an in-person room session from an available device
- receive a transcript-backed summary when the meeting ends
- see decisions, action items, owners, follow-ups, assumptions, and open
  questions extracted from the meeting
- use Omega in Silent recorder mode by default, with host controls to mute,
  pause, or stop
- have meeting-time research available when the host invites it
- save the meeting record into Omega
- search past meeting records later
- reuse a meeting record, or convert its outputs into tasks, briefs, prompts,
  or follow-up materials, in later work

The measure of the first version is user outcome, not mechanism: the user
walks out of a supported meeting — virtual, in-room, or hybrid — and the
useful, structured record is already in Omega, with the raw transcript
available underneath it and live participation available when invited.

## 17. Non-goals / out of scope

This request does not propose:

- any SDK, browser-automation, media-architecture, audio-stack,
  transcription-provider, or storage design
- any specific meeting-platform integration mechanics
- any transcription engine or model choice
- a fully autonomous meeting facilitator; participation is always
  host-controlled (section 11)
- guaranteed or perfect speaker attribution (section 9)
- any change to Sarah's existing voice-conversation surface or contract ([E])
- any change to community room media behavior ([E])

It is a capability request, not a build plan, and it explicitly allows the
team to phase virtual platform support sensibly while treating in-person and
hybrid contexts as first-class.

## 18. Open design questions

1. Which meeting environment should be the first supported platform, and how
   is phased support sequenced and communicated? [P]
2. Should a meeting record be one knowledge object (summary, decisions,
   action items, transcript) or several related records? [P]
3. How are action items and follow-ups turned into actionable work in Omega —
   as searchable records only, or as items the user can convert into tasks? [P]
4. How should in-person room capture and hybrid capture be made visible and
   consent-first across device types? [P]
5. How is recording consent handled per meeting environment and per room
   session in the first version? [P]
6. Should the research copilot be usable outside a live meeting (for
   example, on a saved meeting record), or is meeting-time-only the
   first-version scope? [P]

## 19. Related documents and evidence

- [taxonomy.md](taxonomy.md): glossary and naming rules; rule 4 is the claim
  discipline this note follows
- [sarah-realtime-voice.md](sarah-realtime-voice.md): the existing Sarah
  realtime voice contract; owner-private media room and transcript policy [E]
- [../../crates/agent_ui/src/omega_public_channel_livekit.rs](../../crates/agent_ui/src/omega_public_channel_livekit.rs):
  existing community room media support with verified participant roster [E]
- [PRODUCT.md](../../PRODUCT.md): Omega's product shape, users, and design
  principles
