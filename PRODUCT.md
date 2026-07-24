# Product

## Register

product

## Users

Developers and human-agent teams who need to understand, delegate, review, approve, and ship work from one native workspace. They may arrive with existing editors, projects, and authenticated external agents, and should not have to surrender those configurations to begin working in Omega.

## Product Purpose

Omega is the primary OpenAgents desktop client: an IDE and durable workroom connecting people, agents, conversations, code, reviews, decisions, approvals, and execution. Nostr is a native signed-coordination substrate—not merely an onboarding identity or wallet feature—so identities, delegations, approvals, project activity, and run evidence can remain portable and independently verifiable. Success means identity and authority are explicit, work remains attached to durable evidence, and existing agents can join without losing their own configuration or authentication boundaries.

## Brand Personality

Sovereign, capable, and direct. Omega should feel like a trustworthy native tool for consequential work: calm about complexity, precise about authority, and concise about what the user must do.

## Anti-references

- A generic chat panel bolted onto an editor.
- A hosted-agent upsell disguised as local setup.
- Credential import flows that copy or blur ownership of another agent's login.
- Decorative onboarding ceremony that delays the first real project or agent thread.
- Zed or legacy OpenAgents/Pylon product identity presented as Omega's authority.

## Design Principles

- Establish portable identity before asking the user to configure the editor.
- Treat Buzz's custody patterns as a safety baseline, then integrate Nostr more deeply across people, agents, projects, signed actions, approvals, and durable receipts.
- Preserve familiar editor controls and the selected Zed theme structure while Omega-specific surfaces mature.
- Keep authority boundaries explicit: external agents own their runtime, authentication, billing, and configuration.
- Prefer real setup actions over tutorials; onboarding should create durable state users immediately keep using.
- Fail closed and explain the smallest next action without exposing secrets or encouraging repeated permission prompts.

## Accessibility & Inclusion

First-run and release-critical surfaces must be keyboard operable, expose accurate accessibility labels and state, preserve readable contrast across the supported light and dark themes, and avoid motion that is required to understand or complete setup. Installed-candidate proof includes inspection of the accessibility tree and visible product labels.
