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

### OMEGA-DELTA-0005 — No hosted-plan or trial surfaces

- **Upstream Zed:** ships subscription plan definitions (Free / Pro / Business /
  VIP / Student), a trial-ended upsell that covers the agent panel and calls
  `block_mouse_except_scroll`, and a banner explaining that GitHub accounts
  under 30 days old cannot start a Pro trial.
- **Omega:** these files are deleted —
  `crates/ai_onboarding/src/plan_definitions.rs`,
  `crates/ai_onboarding/src/young_account_banner.rs`, and
  `crates/agent_ui/src/ui/end_trial_upsell.rs`.
- **Why:** Omega does not sell a hosted AI service, so these advertise a product
  that does not exist here, and they present Zed as the vendor, which omega#16
  forbids. The upsell also blocked operator input to show an advertisement.
- **Enforced by:** `removed_surfaces_stay_removed`.

### OMEGA-DELTA-0006 — Nothing nags from ambient state

- **Upstream Zed:** suggests installing an extension when a file's language is
  unrecognised, suggests reopening in a dev container based on repository
  contents, and asks to move the application into `/Applications` at startup —
  the last with a "Don't ask me again" button, which is an admission that it is
  a nag.
- **Omega:** all three are deleted, along with the subscriptions that drove them.
- **Why:** none responds to anything the operator did. They interrupt because
  the editor noticed something. An interruption has to be earned by a user
  action or by preventing irreversible loss.
- **Enforced by:** `removed_surfaces_stay_removed`.

### OMEGA-DELTA-0007 — Terminating a debug session does not ask

- **Upstream Zed:** prompts "This Debug Session is still running. Are you sure
  you want to terminate it?".
- **Omega:** terminates immediately.
- **Why:** terminating a debug session loses the session and nothing else. The
  operator asked for it.
- **Not covered:** the restart confirmation at
  `crates/workspace/src/workspace.rs` is left in place. It is gated on
  `confirm_quit`, which `OMEGA-DELTA-0003` already locks to `false`, so it is
  unreachable in Omega. Deleting it would mean surgery on the shutdown path for
  no behavioural change.
- **Enforced by:** `debug_terminate_never_prompts`.

### OMEGA-DELTA-0008 — No Zed subscription or hosted-plan copy

- **Upstream Zed:** `zed_ai_description` in
  `crates/language_models/src/provider/cloud.rs` renders a subscription pitch —
  Pro, Student, Business, VIP, and *"Subscribe for access to Zed's hosted
  models. Start with a 14 day free trial."*
- **Omega:** replaced with neutral copy about a configured provider.
- **Why:** it advertised a product Omega does not sell, named Zed as the vendor,
  and told the operator to buy something they cannot buy. omega#16 forbids
  presenting Zed as the product.
- **How it was found:** scanning the **installed rc4 binary**, not the source
  tree. The earlier source-level passes missed it because it lives in a
  different function from the plan definitions that were deleted. Binary
  verification is the only reason this is not still shipping.
- **Enforced by:** `no_zed_product_copy_survives_anywhere`.

### OMEGA-DELTA-0009 — The Restricted Mode UI is gone, not dormant

- **Upstream Zed:** a security modal, a title-bar "Restricted Mode" badge, a
  settings banner, and the trust-modal plumbing behind them.
- **Omega:** `crates/workspace/src/security_modal.rs` is deleted, along with
  `show_worktree_trust_security_modal`, the auto-show on project open, the
  `ToggleWorktreeSecurity` handler, the title-bar badge, the settings banner,
  and the component-gallery sample that carried
  *"Review .zed/settings.json"*.
- **Why:** `OMEGA-DELTA-0001` stopped the modal appearing but left it compiled
  in, which an adversarial review flagged and a binary scan confirmed — the
  `.zed/settings.json` identifier was still shipping in `0.2.0-rc4`.
  Unreachable code that a rebase can revive is not a removal.
- **Known remainder:** two dead render blocks still mention "Restricted Mode"
  (`crates/agent_ui/src/profile_selector.rs`,
  `crates/language_tools/src/lsp_button.rs`). They are gated on state that can
  no longer be true, and "Restricted Mode" is a feature name rather than a Zed
  product identifier, so they are not an omega#16 violation. Tracked on
  omega#64 rather than claimed as done.
- **Enforced by:** `removed_surfaces_stay_removed`, `no_zed_product_copy_survives_anywhere`.
