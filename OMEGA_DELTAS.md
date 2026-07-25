# Omega deltas from Zed

Omega is a fork of Zed. Every place Omega deliberately behaves differently from
upstream is recorded here, and every entry is mechanically checked.

The reason this file exists: a fork accumulates silent divergence. Someone
changes a default, a rebase quietly reverts it, and nobody notices until an
owner sees the upstream behaviour again in a release candidate. A comment in
the code is not enough, because a merge can drop the comment and the value
together.

## Rules

- **Every delta has an ID** (`OMEGA-DELTA-NNNN`), and the ID appears in the code
  it governs, so a reader who finds the code finds the reason.
- **Every delta has a programmatic check** in `crates/omega_deltas/`, so a
  rebase that reverts it fails the test suite instead of shipping.
- **Every delta has its own test.** A delta whose check cannot fail is not a
  check. Each test asserts the Omega value *and* names the upstream value it
  replaces, so the diff stays legible.
- **A delta is a policy record, not a changelog.** Record why the owner wanted
  it, not what the commit did.
- **Removing a delta is a policy change**, and needs the same care as adding
  one: delete the entry, the check, and the test together.

Run the checks with:

```sh
cargo test -p omega_deltas
```

## Registry

### OMEGA-DELTA-0001 — No Restricted Mode, no trust prompt

- **Upstream Zed:** `session.trust_all_worktrees` defaults to `false`. Opening
  an unrecognized project shows an "Unrecognized Project" modal offering
  "Stay in Restricted Mode" or "Trust and Continue". Restricted Mode blocks
  project settings, language servers, and MCP server integrations.
- **Omega:** `session.trust_all_worktrees` defaults to `true`. Nothing is
  restricted, so the modal never auto-shows.
- **Why:** owner direction, 2026-07-25, on seeing the prompt in
  `0.2.0-rc3`: *"I HATE THAT. I NEVER WANT TO SEE THAT AGAIN. NO RESTRICTED
  SHIT."* Omega is an owner-operated editor opened on the owner's own
  repositories. The prompt interrupts every new project, and declining it
  silently disables language servers and project settings — which reads as the
  editor being broken rather than as a security posture.
- **Also:** the modal body told the user to *"Review .zed/settings.json"*, an
  unclassified Zed identifier on a visible product surface, which omega#16
  forbids independently of the owner's preference.
- **Enforced by:** `crates/omega_deltas/src/omega_deltas.rs`,
  `trust_all_worktrees_defaults_to_true`.
- **Correction, 2026-07-25 (adversarial review).** An earlier version of this
  entry claimed the `ToggleWorktreeSecurity` action still opens the modal on
  demand. **That was false.** `can_trust`
  (`crates/project/src/trusted_worktrees.rs:469`) returns early when
  `trust_all_worktrees` is set, before populating the `restricted` map, so
  `has_restricted_worktrees` is permanently false and
  `show_worktree_trust_security_modal`
  (`crates/workspace/src/workspace.rs:8541`) returns without opening anything.
  The action is a silent no-op. This delta therefore *does* remove the ability
  to inspect trust — invisibly, rather than by deleting the code.
- **The Zed identifier is still in the tree.** The modal body's
  *"Review .zed/settings.json"* (`crates/workspace/src/security_modal.rs:196`)
  was made unreachable, not removed. Tracked separately; unreachable code that
  a rebase can revive is not a fix.
- **Remote peers cannot reintroduce it.** `handle_restrict_worktrees`
  (`crates/project/src/project.rs`) used to call `restrict()` unconditionally,
  so a remote server running upstream Zed could push Restricted Mode onto this
  machine regardless of the local default. It now returns early when
  `trust_all_worktrees` is set.

### OMEGA-DELTA-0002 — Agents do not ask before every tool action

- **Upstream Zed:** `agent.tool_permissions.default` is `"confirm"`. Every agent
  tool action prompts before running.
- **Omega:** defaults to `"allow"`.
- **Why:** Omega's purpose is unattended agent work. With nobody watching, a
  confirmation prompt is not a safeguard — it is a hang, and the run stalls
  until a human returns. Owner direction, 2026-07-25: Omega is *"YOLO mode
  throughout, aka 'do what the user tells you'"*.
- **The line is still drawable.** `always_confirm` and `always_deny` patterns
  keep working and are the supported way to gate a specific operation, such as
  `git reset --hard`, force pushes, or reads of `.env` and key material. Omega
  ships none by default because the owner asked for none. Anyone who wants one
  adds a pattern rather than reverting this default.
- **Known tradeoff, stated plainly:** an agent can now run a destructive command
  without asking. That is the requested behaviour, not an oversight.
- **Enforced by:** `agent_tool_permissions_default_to_allow`.

### OMEGA-DELTA-0003 — Quitting is never confirmed

- **Upstream Zed:** ships a confirm-on-quit path.
- **Omega:** `confirm_quit` defaults to `false`.
- **Why:** quitting is deliberate and recoverable; unsaved buffers are handled
  by `restore_unsaved_buffers`, not by a modal.
- **Note:** this value already matched. The delta exists to lock it, so a rebase
  cannot quietly reintroduce the prompt.
- **Enforced by:** `quitting_is_never_confirmed`.

### OMEGA-DELTA-0004 — Telemetry stays off

- **Upstream Zed:** ships telemetry defaults that may change between releases.
- **Omega:** `telemetry.diagnostics` and `telemetry.metrics` both default to
  `false`.
- **Why:** privacy posture, and Omega has no telemetry endpoint of its own. A
  posture that depends on nobody changing an upstream default is not a posture.
- **Note:** these values already matched, and are locked for the same reason as
  `OMEGA-DELTA-0003`.
- **Enforced by:** `telemetry_stays_off`.
