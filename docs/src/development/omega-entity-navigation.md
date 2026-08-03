# Omega entity navigation

Omega uses a typed entity route graph for All Work navigation. The graph keeps
conversation identity separate from the work, records, and agent activity that
a conversation can open.

## Route contract

`omega_workbench_state::EntityRoute` defines stable routes for:

- Thread
- Work
- Issue projection
- Project
- Document
- Decision
- Agent Session
- Settings

A domain Block is nested in its owning Work route. A selected Block therefore
cannot inherit the identity of the active Thread by accident. Each route has a
stable key, kind, title, icon class, focus target, availability state, and
persisted history representation.

The versioned persistence schema is
`openagents.omega.entity-navigation.v1`. History is bounded to 128 entries and
supports back, forward, branching, restart restoration, and tab-close fallback.
Older panel state migrates its restorable active Thread into the new graph.
Invalid schema versions and invalid indexes are rejected before restoration.

## Admitted destinations

The production interface currently admits these destinations:

- restorable Thread routes;
- the Forensics Work route with its selected Forensics Block;
- Settings.

Forensics has its own Work tab and browser-history identity. Opening it does not
leave an unrelated Thread title selected. Closing its tab returns to the active
Thread. Selecting a Thread closes the work surface and returns focus to the
conversation transcript.

The remaining route variants are contract-first. They do not appear in
production navigation until their feature is implemented and admitted. If a
persisted or linked destination is unknown, stale, deleted, unauthorized, or
not implemented, Omega renders a typed unavailable surface. It does not render
the previous entity's content or reveal the unavailable entity reference.

## Interface terminology

The left sidebar uses **Repositories** for local editor and worktree context.
A repository selection changes the IDE context; it does not create a portfolio
Project.

The conversation list and tabs use **Threads**. A Thread is a durable
conversation. It is not an Agent Session or Run. **Projects** and **Agent
Sessions** remain reserved for their canonical All Work entity meanings.

## Verification

The shared-state tests verify every route's identity, icon, focus target,
Block ownership, bounded history, persistence validation, and legacy migration.
The GPUI regression test verifies the Repositories and Threads sections,
Forensics Work-tab selection, tab close, back and forward navigation, and the
typed unavailable surface without an active Thread marker.

Because installed UI automation was explicitly disabled for this delivery,
release evidence consists of deterministic GPUI interaction tests, the
release-fast bundle, package signature inspection, and artifact hashes. No
GUI-control tool is used.
