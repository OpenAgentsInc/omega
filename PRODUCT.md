# Product

## Register

product

## Users

Developers and human-agent teams who need to understand, delegate, review, approve, and ship work from one native workspace. They may arrive with existing projects and authenticated external agents, and should not have to surrender those configurations to begin working in Omega.

## Product Purpose

Omega is the primary OpenAgents desktop client: a native, durable workroom connecting people, agents, conversations, code, reviews, decisions, approvals, and execution. Success means identity and authority are explicit, work remains attached to durable evidence, and existing agents can join without losing their own configuration or authentication boundaries.

## Product Shape

Omega has one normal, flag-free launch surface: a conversation, composer, navigation sidebar, tester channels, and workbench rail. A person chooses one of three conversation modes when creating a conversation:

- **Direct Agent** runs Codex, Claude Code, Grok Build, or another configured ACP agent directly.
- **Omega Agent** routes work among eligible executors and discloses the route it chose.
- **Sarah** is the voice conversation mode and states its eligibility, price, limits, and authority before a session starts.

The chosen mode, concrete executor or router selection, project, and readiness are visible before the first send. Ready means that the named target has connected and created an actual session; executable or configuration detection alone is not enough. Once a conversation starts, its mode and owner do not change underneath its transcript, and every surface names the executor that actually does the work.

There is no second launch surface and no launch mode vocabulary: the full-editor mode split was removed (omega#161), and the flag-free launch is the application. Vim remains part of the product in the composer and the focused editing surface.

## Brand Personality

Sovereign, capable, and direct. Omega should feel like a trustworthy native tool for consequential work: calm about complexity, precise about authority, and concise about what the user must do.

## Anti-references

- A generic chat panel bolted onto an editor.
- A hosted-agent upsell disguised as local setup.
- Credential import flows that copy or blur ownership of another agent's login.
- Decorative onboarding ceremony that delays the first real project or agent thread.
- Zed or legacy OpenAgents/Pylon product identity presented as Omega's authority.

## Design Principles

- Establish portable identity before asking the user to configure the workroom.
- Preserve familiar editing controls, Vim, and the selected Zed theme structure inside Omega's focused editing surfaces.
- Keep authority boundaries explicit: external agents own their runtime, authentication, billing, and configuration.
- Prefer real setup actions over tutorials; onboarding should create durable state users immediately keep using.
- Fail closed and explain the smallest next action without exposing secrets or encouraging repeated permission prompts.
- No exposition in the UI anywhere: controls are labeled, not narrated; multi-sentence tooltips and status sentences that explain internal mechanics are defects (`OMEGA-DELTA-0189`).
- Statuses are colors or icons, never words; a one-word tooltip is the maximum copy (`OMEGA-DELTA-0189`).
- Production navigation is capability-derived. A destination, row, tab, badge, or action appears only when its source-backed route, primary interaction, loading/empty/error/offline states, keyboard behavior, accessibility semantics, and required acceptance evidence are implemented for that build. Unimplemented and fixture-only destinations are absent from normal, candidate, and release builds. They can appear only behind an explicit development/mock gate and must identify their state as non-production.
- Escape closes every modal and auxiliary window, including Settings (`OMEGA-DELTA-0189`).

## Accessibility & Inclusion

First-run and release-critical surfaces must be keyboard operable, expose accurate accessibility labels and state, preserve readable contrast across the supported light and dark themes, and avoid motion that is required to understand or complete setup. Installed-candidate proof includes inspection of the accessibility tree and visible product labels.
