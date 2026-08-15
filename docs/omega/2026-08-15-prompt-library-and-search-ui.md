# Prompt library and prompt search UI

- Status: feature request for owner and team review
- Date: 2026-08-15

## 1. Purpose

This note requests a product capability: a native Prompt Library and Search UI
in Omega so users can save, organize, search, reopen, insert, and run reusable
prompts over time. The request is deliberately implementation-agnostic: it
states what the user should be able to do and what a strong first version
should include, and does not prescribe how prompt records are stored, how a
search index is built, how schemas are defined, or how the UI is architected.
Decisions rest with the owners; every consequential claim below carries an
[E], [D], or [P] tag.

## 2. Claim categories

- `[E]` Existing system behavior
- `[D]` Existing documented direction
- `[P]` Proposed future direction

[Taxonomy rule 4](taxonomy.md), "Say what is true now, not what is planned,"
is why every consequential claim is tagged. This note is a proposal, so
proposed direction is always labeled as such and verified absences are marked.

## 3. The gap

Omega persists a prompt library at the storage layer and keeps unsent drafts
per thread, but there is no user-facing library, organization, or search
surface, and no way to run a saved prompt on demand.

- A `prompt_store` crate exists with an LMDB-backed prompts library, user
  prompt records, and a built-in commit-message prompt ([E];
  crates/prompt_store). It exposes metadata (id, title, default, saved_at) but
  no search, filtering, folders, tags, or category model ([E]).
- Unsent draft text is persisted per thread so a draft survives switching
  agents or closing the thread ([E]; crates/agent_ui/src/draft_prompt_store.rs).
  Drafts are session-bound; they are not a reusable library a user searches
  across later ([E]).
- Skills are on-demand instruction artifacts loaded from local directories
  with progressive disclosure ([E]; taxonomy.md, crates/agent_skills), which
  serve agent behavior rather than user-authored, reusable prompts.

The missing product shape is the layer on top: a user-facing library where
prompts are saved, organized, found again, and run as recurring work.

## 4. Requested capability

[P] Omega should let a user:

- save prompts for reuse
- organize prompts by title, folder, tag, or category
- distinguish personal prompts from team or shared prompts
- search saved prompts quickly
- reopen past prompts later
- insert a prompt into a live session
- run a prompt directly inside Omega
- build reusable workflows over time instead of losing them across chats and
  notes

This is not only about storing text. It is about making Omega a real
orchestration environment for recurring work: the same request, brief, review,
or analysis can be run again instead of being retyped or re-assembled each
time.

## 5. Organize and distinguish

[P] The library should let a user keep prompts findable as the collection
grows:

- a title and an optional folder, tag, or category on each saved prompt
- a personal library that belongs to the user
- a distinct team or shared space for prompts the group reuses, with the
  boundary between them visible so a shared prompt is not mistaken for a
  personal one

Organization is a finding problem: with a small number of prompts, a list is
enough; the value of folders, tags, and search grows with the collection.

## 6. Search

[P] Users should be able to search saved prompts quickly by title, content,
folder, tag, or category, and see results as they type. Search should cover
both personal and shared prompts, with the scope made clear in the results so
the user knows which collection each hit comes from.

## 7. Reopen, insert, and run

[P] A saved prompt should be usable in three ways, all from the library:

- reopen it to read, edit, and re-save it
- insert it into the composer of a live session as a starting point or an
  edit target
- run it directly, launching a conversation whose first message is the prompt
  (or the prompt with user-fillable fields) without leaving the library

Run-as-recurring-work is the step that turns a saved prompt from a stored text
into an orchestration primitive.

## 8. Why this matters

[P] A prompt library would support the workflows Omega already serves:

- founder and operator workflows
- research workflows
- admin workflows
- onboarding
- content production
- reusable internal SOP-style workflows

Reusable prompts support repeatable workflows, and repeatable workflows
improve continuity, delegation, onboarding, and process maturity: the same
handoff, brief, review, or analysis runs the same way every time instead of
depending on memory or a recreated draft. Over time, the saved prompts that
prove themselves become natural source material for documentation and
onboarding assets, so the library compounds rather than only accumulates.

## 9. What a strong first version should enable

[P] A strong first version should let a user, from one surface:

- save the current composer prompt as a reusable prompt with a title
- browse saved prompts by title and see the personal vs shared scope
- search saved prompts by title or content and see results quickly
- open a saved prompt, edit it, and save the change
- insert a saved prompt into a live session composer
- run a saved prompt directly, starting a conversation with it
- distinguish personal prompts from shared prompts at a glance

Editing, deleting, and renaming a saved prompt should be plain and reversible
enough that the library stays trusted as the place to keep recurring work.

## 10. Non-goals / out of scope

This request does not propose:

- any prompt storage schema, search-index architecture, or UI framework
- any specific folder, tag, or category data model
- any change to how drafts are persisted per thread ([E])
- any change to skills, rules, or the agent-prompt pipeline ([E])
- any team-sharing or sync implementation; the personal-vs-shared boundary is
  requested as a product concept, with the mechanics left to the team

It is a capability request, not a build plan.

## 11. Privacy and sharing notes

[P] Product-level considerations for the team to weigh, not decisions made
here: personal prompts should follow Omega's local data posture and not
appear in shared collections by accident; shared prompts should be clearly
scoped so a prompt authored by one person is not silently presented as
group-owned; and prompts can contain sensitive working text, so the boundary
between personal and shared should be explicit in both storage intent and
display.

## 12. Open design questions

1. Is the personal-vs-shared boundary a first-version feature or a later one?
   [P]
2. Should a saved prompt support fields or variables (for example, a template
   with fill-in parts) in the first version, or be plain text only? [P]
3. Should search be instant in-memory, or is a separate index justified at
   the expected library size? [P]
4. Where does the library surface: a panel, a command palette action, or the
   composer itself? [P]
5. Should running a saved prompt always start a new conversation, or should
   it also be able to continue an existing one? [P]

## 13. Related documents and evidence

- [taxonomy.md](taxonomy.md): glossary and naming rules; rule 4 is the claim
  discipline this note follows
- [../../crates/prompt_store/src/prompt_store.rs](../../crates/prompt_store/src/prompt_store.rs):
  the existing LMDB prompt library, user and built-in prompt records,
  metadata without search or organization [E]
- [../../crates/agent_ui/src/draft_prompt_store.rs](../../crates/agent_ui/src/draft_prompt_store.rs):
  per-thread unsent-draft persistence, distinct from a reusable library [E]
- [PRODUCT.md](../../PRODUCT.md): Omega's product shape, users, and design
  principles
