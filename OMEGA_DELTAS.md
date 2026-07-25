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
- **An ID names exactly one entry, and IDs are never reused.**
  `OMEGA-DELTA-0012` and `OMEGA-DELTA-0013` were originally landed as second
  uses of `0010` and `0011` by two lanes allocating numbers at the same time,
  which made four entries uncitable. They were renumbered on 2026-07-25.
  `delta_ids_are_unique` now fails on a repeat, in both the registry headings
  and `ENFORCED_DELTAS`, so the collision cannot recur silently.

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
  the profile-selector and language-server Restricted Mode affordances, the
  default Restricted Mode key bindings, and the component-gallery sample that
  carried *"Review .zed/settings.json"*.
- **Why:** `OMEGA-DELTA-0001` stopped the modal appearing but left it compiled
  in, which an adversarial review flagged and a binary scan confirmed — the
  `.zed/settings.json` identifier was still shipping in `0.2.0-rc4`.
  Unreachable code that a rebase can revive is not a removal.
- **Enforced by:** `removed_surfaces_stay_removed`,
  `no_zed_product_copy_survives_anywhere`, and
  `restricted_mode_ui_and_shortcuts_are_absent`.

### OMEGA-DELTA-0010 — The title-bar identity entry stays local

- **Upstream Zed:** the title-bar *Sign In* button starts the hosted-account
  browser flow.
- **Omega:** the title bar presents *Omega Identity* and opens the local
  identity-first onboarding journey.
- **Why:** Omega's inherited service endpoint is intentionally non-routable.
  A visible control must not send the owner to a browser error page, and local
  Omega identity is the product's actual account boundary.
- **Enforced by:** `title_bar_identity_entry_opens_onboarding`.

### OMEGA-DELTA-0011 — AI onboarding configures providers, not a hosted plan

- **Upstream Zed:** agent and edit-prediction onboarding inspect the Zed account
  plan, offer hosted sign-in, and promote hosted AI tiers.
- **Omega:** agent onboarding lists directly configured providers and opens
  Agent Settings for provider credentials. Edit-prediction onboarding only
  configures its retained GitHub Copilot provider.
- **Why:** provider setup is a local Omega capability. Zed account status,
  trials, hosted-plan callbacks, and hosted sign-in are not.
- **Enforced by:** `ai_onboarding_is_provider_only`.

### OMEGA-DELTA-0012 — Zed collab is retired

- **Renumbered 2026-07-25.** This entry landed as a second `OMEGA-DELTA-0010`.
  Cite `0012`.
- **Upstream Zed:** ships a collaboration product — a collab panel with a
  nested channel tree, contacts, calls, channel notes, and a `collab` server
  crate behind it.
- **Omega:** `crates/collab_ui` and `crates/collab` are deleted, along with the
  panel registration, the `Collab Panel` menu item, the `ToggleFocus` action,
  and the `open_channel_notes` CLI path.
- **Why:** owner direction, 2026-07-25. Collab is not an Omega product surface.
  Omega never ran the collab server, and nothing else in the workspace depended
  on it. The Buzz parity ledger states the replacement direction explicitly:
  workrooms are "native GPUI panes over the Nostr workroom log" and people and
  agents are "signed Nostr identity, profile, membership, and presence events".
  Carrying Zed's channel and contact model forward would fight that.
- **Harvested first.** The owner asked for retirement *unless* something was
  worth keeping for Buzz parity, and the GPUI panel shapes were: the nested
  channel tree with expand/collapse and drag reparenting, per-row context
  menus, unread and mention indicators, the fuzzy roster picker, and the
  membership modal. Those map onto the Workrooms and People-and-agents parity
  rows and are recorded in
  `docs/buzz/2026-07-25-collab-ui-harvest-before-retirement.md` in the
  openagents repository, with recovery instructions. The code is gone; the map
  is not.
- **This also removes** the three collab confirmation prompts on omega#54's
  list. They die with the feature rather than separately, which is what that
  entry asked for.
- **Enforced by:** `removed_surfaces_stay_removed`.

### OMEGA-DELTA-0013 — The agent is on by default

- **Renumbered 2026-07-25.** This entry landed as a second `OMEGA-DELTA-0011`.
  Cite `0013`. The comment at `assets/settings/default.json:1063` still reads
  *"See OMEGA-DELTA-0011"* and now points at the AI-onboarding entry instead of
  this one. That file is owned by another lane this session, so the correction
  is deferred rather than made here; it is a stale citation, not a behaviour
  change.
- **Upstream Zed:** ships the agent enabled.
- **Omega, before this:** `agent.enabled` and `agent.button` were both `false`
  in `assets/settings/default.json`, turned off by commit `9e585569cb`
  ("Isolate Omega from Zed production services").
- **Omega now:** both `true`.
- **Why the original change went too far.** The isolation sweep was right about
  the hosted-model path — `language_models_cloud` is still gated behind
  `OMEGA_ALLOW_ZED_SERVICES`, and it stays gated. But it disabled the *agent*,
  which is not a Zed service. The runtime (`crates/agent`), the thread
  abstraction (`crates/acp_thread`), the tools, and eighteen local and direct
  providers are Omega's own and reach no Zed host.
- **What the owner saw.** With `enabled: false`, `agent_ui` also removes the
  `agent`, `agents`, and `assistant` namespaces from the command palette
  (`crates/agent_ui/src/agent_ui.rs:836-840`), and the panel reports
  `enabled() == false` (`agent_panel.rs:5035`). So the feature was not merely
  off — it was unreachable, and the Settings UI exposes only `agent.button`,
  never `agent.enabled`. There was no in-product way to turn it on.
- **Nothing structural was removed**, which is why this is two booleans: the
  panel is constructed, the actions are registered, and roughly thirty
  `agent::*` keybindings already existed and were simply unreachable.
- **`cmd-shift-a` / `ctrl-shift-a`** now opens a new agent thread globally.
  `agent::NewThread` previously existed only in panel-scoped contexts bound to
  `cmd-n`, so it could not start a thread unless the panel already had focus.
- **Known limitation, not hidden:** `agent.default_model` is still
  `ollama/llama3.1`, so out of the box the agent needs Ollama running locally.
  Changing that default is a separate decision, and the service-isolation test
  asserts the current value (`service_isolation.rs:113`).
- **Enforced by:** `the_agent_ships_enabled`.

### OMEGA-DELTA-0014 — A protected recovery offers replacement, not protection

- **Upstream Zed:** has no equivalent. The onboarding Identity section is an
  Omega surface, so what this entry locks is a divergence from Omega's own
  earlier behaviour. It is registered here because a refactor of that branch
  reverts it as silently as a rebase would.
- **Omega, before this:** the `CustodyState::Ready` branch of
  `crates/onboarding/src/identity_section.rs` switched the description and the
  colour on `RecoveryProtectionState`, then emitted
  `actions: vec![IdentityAction::Protect]` unconditionally. A protected
  identity therefore read *"Recovery protected"* in green with a **Protect
  recovery** button directly beneath it.
- **Omega now:** the branch selects the action from the same state it already
  used for the copy — `Protect` when protection is needed, `ReplaceRecovery`
  ("Replace recovery file") when it is not.
- **Why:** owner report, omega#68, 2026-07-25. A control whose label denies the
  status line above it reads as *"this did not work"*, which for a custody
  surface is a confidence defect rather than a cosmetic one.
- **Rotation was kept deliberately.** Hiding the control when protected was the
  simpler option and was not taken. Replacing a recovery file is a real
  journey, so the control stays reachable and is relabelled — option 2 on the
  issue. Dropping it silently is exactly what that issue's falsifier forbids.
- **What the check catches:** a `Ready` branch that emits a constant action
  again. It asserts the `ReplaceRecovery` variant and its label still exist,
  that the branch is state-conditional rather than a literal
  `actions: vec![IdentityAction::Protect],`, and that the crate-local
  regression test keeps its `assert_ne!` on the two states — so deleting the
  behavioural test is caught here too.
- **Enforced by:** `crates/omega_deltas/src/omega_deltas.rs`,
  `protected_recovery_offers_a_different_action`.

### OMEGA-DELTA-0015 — `cmd-shift-s` opens the Sarah workroom

- **Upstream Zed:** binds the chord to `workspace::SaveAs` in all three default
  keymaps, and has no workroom.
- **Omega:** all three default keymaps bind `workroom::OpenPanel` —
  `cmd-shift-s` on macOS, `ctrl-shift-s` on Linux and Windows — in the
  `Workspace` context section, once each.
- **Why:** owner direction, 2026-07-25 (omega#69). The action already existed
  and both focused the panel and marked the room read (OMEGA-SW-06), but no
  keymap named it, so the workroom was reachable only through the command
  palette. Opening the workroom must not depend on whether an editor, a
  terminal, or a panel happens to hold focus — a focus-dependent binding is
  that issue's stated falsifier.
- **`Workspace`, not a context-free section — deliberately, and not what the
  issue's text asked for.** omega#69 said "the top-level section with no
  context predicate". The binding landed in the `Workspace` section instead,
  which is the root context of the window tree: it matches from an editor, a
  terminal, or any panel, so the Exit holds. It is also where every other
  window-global Omega chord lives — `workspace::Save`, `workspace::NewWindow`,
  and the `agent::NewThread` binding from `OMEGA-DELTA-0013`. A truly
  context-free section is where the `menu::` bindings live and would have made
  this chord fire inside menus and pickers too. The check therefore accepts
  either no context or a context in `WINDOW_GLOBAL_KEYMAP_CONTEXTS`, and
  rejects anything narrower.
- **The Save As trade, stated plainly. A keystroke was taken, not shadowed.**
  This is what omega#69 asked to confirm before landing. The chord did not go
  to the workroom because it was free — it was **`workspace::SaveAs` in all
  three default keymaps** and was overwritten
  (`default-macos.json:708`, `default-linux.json:651`,
  `default-windows.json:645` at `7b347cb9a4^`). After this delta:
  - **macOS and Windows have no default Save As keystroke at all.**
  - **Linux keeps only `shift-save`**, the hardware `save` media key, which
    most keyboards do not have.
  - Emacs-keymap users are unaffected: `ctrl-x ctrl-w` still saves as, in both
    `macos/emacs.json` and `linux/emacs.json`.
  - No other base keymap — VS Code, JetBrains, Sublime, Atom, Cursor,
    TextMate — binds Save As at all, so none of them restores it.

  **The mitigation is the File menu**, not the palette: `Save As…` remains at
  `crates/zed/src/zed/app_menus.rs:129`, so the cost is discoverability rather
  than capability. The check asserts that menu item still exists, because if a
  later cleanup drops it, Save As becomes reachable only by knowing its command
  name — and this entry would then be recording a mitigation that no longer
  exists.
- **Two narrower bindings overlap the chord and keep precedence**, so they are
  not affected either way: `specific-overrides-macos.json` and
  `specific-overrides.json` bind it to `picker::ToggleMultiSelect` in context
  `Picker > Editor`, and `macos/textmate.json` binds `ctrl-shift-s` to
  `search::SelectPreviousMatch` in `BufferSearchBar` — a different chord from
  the macOS `cmd-shift-s`.
- **Why the check is stronger than asserting the string is present.** In
  `0.2.0-rc6` Omega hard-panicked before any window opened because 27 bindings
  named actions whose crates had been deleted: the built-in keymap is loaded
  and unwrapped at startup, and `cargo check --workspace` passes regardless
  because keymaps are runtime assets. So the check parses each keymap, requires
  exactly one binding of the chord — a second, narrower one would shadow the
  global one depending on focus — requires its context to be window-global, and
  resolves the action name back to a live `actions!` declaration in
  `crates/zed_actions/src/lib.rs`. Renaming or deleting the action fails here
  rather than at the owner's next launch.
- **Enforced by:** `required_keymap_bindings_resolve`.

### OMEGA-DELTA-0016 — Aiur is dark-only

- **Upstream Zed:** ships no Aiur, and its own defaults (`One Dark` /
  `One Light`) both name themes it ships.
- **Omega, before this:** `assets/themes/aiur/aiur.json` declared two themes in
  one family — `Aiur Dark` and `Aiur Light` — and `DEFAULT_LIGHT_THEME` pointed
  at `Aiur Light`.
- **Omega now:** the family declares exactly one theme, named `Aiur`,
  appearance `dark`. `DEFAULT_DARK_THEME` is `Aiur` in both
  `crates/theme/src/theme.rs` and `crates/settings_content/src/theme.rs`.
  `DEFAULT_LIGHT_THEME` is `Ayu Light`, which `assets/themes/ayu/ayu.json`
  ships.
- **Why:** owner direction, 2026-07-25 (omega#70): *"aiur light looks like shit
  and is not what i envisioned. its a dark mode theme only."* A family carrying
  a variant the owner disowns is worse than a family with no variant, because
  the appearance switch can select it without being asked.
- **The Aiur card stays selectable in Light appearance**, resolving to `Aiur`:
  `LIGHT_THEMES` and `DARK_THEMES` in `crates/onboarding/src/basics_page.rs`
  both list `Aiur` first. Choosing the Aiur family gives Aiur in either
  appearance rather than substituting a light theme the owner did not pick.
  Hiding the card in Light mode was the alternative and was not taken, because
  a family vanishing from a three-card selector reads as a bug.
- **The check answers the issue's falsifier directly.** That falsifier is Light
  appearance resolving to a missing theme because a default still names one
  that no longer exists. So the check does not merely assert the constant's
  text: it collects every theme name actually declared under `assets/themes/`
  and asserts both defaults are in that set, and that the two independent
  `DEFAULT_DARK_THEME` constants still agree. Deleting a variant without
  repointing a default fails here rather than at first light-mode launch.
- **Enforced by:** `aiur_is_a_single_dark_theme` and
  `default_themes_exist_in_shipped_assets`.
