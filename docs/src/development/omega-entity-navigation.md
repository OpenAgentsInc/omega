# Omega entity navigation

Omega uses a typed entity route graph for All Work navigation. The graph keeps
conversation identity separate from the work, records, and agent activity that
a conversation can open.

## Route contract

`omega_workbench_state::EntityRoute` defines stable routes for:

- Thread
- Work Index
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
- Inbox and My Work routes after two independent source adapters have
  qualified, nonempty rows;
- the Forensics Work route with its selected Forensics Block;
- source-backed Work detail routes projected by the admitted Work Index;
- Settings.

Inbox and My Work do not appear as disabled or empty promises before that
admission gate. They share one read-only index and keep independent route
identity, selection, history, and restart restoration. See
[Omega Work Index](./omega-work-index.md).

Forensics has its own Work tab and browser-history identity. Opening it does not
leave an unrelated Thread title selected. Closing its tab returns to the active
Thread. Selecting a Thread closes the work surface and returns focus to the
conversation transcript.

Issue projections, Projects, Documents, Decisions, and Agent Sessions remain
contract-first. They do not appear in production navigation until their
feature is implemented and admitted. If a persisted or linked destination is
unknown, stale, deleted, unauthorized, or not implemented, Omega renders a
typed unavailable surface. It does not render the previous entity's content
or reveal the unavailable entity reference.

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
conditional Inbox and My Work admission, both Work Index views, Forensics
Work-tab selection, tab close, back and forward navigation, and the typed
unavailable surface without an active Thread marker.

Because installed UI automation was explicitly disabled for this delivery,
release evidence consists of deterministic GPUI interaction tests, the
release-fast bundle, package signature inspection, and artifact hashes. No
GUI-control tool is used.

### Installed receipt

The 2026-08-02 OAW-003 receipt was built from source commit
`6fefd1cfd31ad7270741d197bff25918dc964860` with the `release-fast` profile.
The package was installed separately at `/Applications/Omega Dev.app`; the
existing production application was not changed. The prior development bundle
was preserved at `/private/tmp/Omega-Dev-before-oaw003-20260802.app`.

- Bundle identifier: `com.openagents.omega.dev`
- Bundle version: `20260803.000403`
- Installed CLI receipt: `Omega 0.2.0 – /Applications/Omega Dev.app`
- Code signature: `codesign --verify --deep --strict` passed; the bundle is a
  thin arm64 Mach-O with the expected ad hoc development signature.
- Packaged and installed `omega` SHA-256:
  `031e67d3c996baa8f178ac3da4081dde141f63c76e93f11bddc4c32aee6b4386`
- `Omega-arm64.dmg` SHA-256:
  `6f0d5b27e4da589b6c4aa04f154989b8826073c70809a443b69072c4c6f57379`

The matching packaged and installed executable hashes prove that the verified
development bundle is the bundle produced by this source revision.
Deterministic GPUI tests supply the route, pointer, keyboard,
unavailable-state, and restoration behavior proof without controlling the
installed application UI.
