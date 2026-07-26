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
- **An inherited test that contradicts a live delta loses.** A rebase can bring
  back an upstream test asserting the pre-Omega value, which makes the tree
  self-contradictory: two tests, one value, neither able to pass. The delta is
  the policy record, so update the inherited assertion to the Omega value and
  name the delta ID in a comment beside it. Do not delete the test to reach
  green — establish what else it covered and keep that coverage under a fixture
  that does not depend on the shipped default.
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
- **Inherited test reconciled, 2026-07-25 (omega#73).**
  `agent_settings::test_default_json_tool_permissions_parse` reads the shipped
  `default.json` and still asserted the upstream `Confirm`, so the tree
  contained two tests asserting opposite things about one value and neither
  could pass. That test arrived with an upstream rebase and was never
  reconciled with this delta. It now asserts `Allow` and cites this ID, and the
  `"confirm"` parse coverage it used to provide incidentally was moved to an
  explicit fixture in `test_tool_permissions_explicit_global_default` rather
  than deleted — the parser must still understand `"confirm"`, because that is
  how an operator draws the line back.

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
- **The default model is `google/gemini-3.6-flash`.** An earlier revision of
  this entry recorded `ollama/llama3.1` and said the agent needed a local
  Ollama to work out of the box. That stopped being true when the owner set the
  Google default, and this text is corrected rather than left to mislead.
  The agent now needs a Google API key, not a local model server.
- **Why the isolation test alone was not enough.** The service-isolation test
  asserts only that the default provider is `google`
  (`crates/app_identity/src/service_isolation.rs`), because what it protects is
  that the default never points at a *Zed* service. That is the right scope for
  that test, but it leaves the model string unpinned: a rebase could change
  `gemini-3.6-flash` to any other Google model and every check would stay
  green.
- **Enforced by:** `the_agent_ships_enabled`, `the_default_model_is_pinned`, and
  `the_new_thread_chord_is_window_global`, which asserts the chord is bound
  window-globally to `agent::NewThread` in all three default keymaps and that
  every narrower binding of it is one of the deliberately admitted surfaces —
  the toolchain and recent-projects pickers, and (on Linux and Windows, where
  the chord is `ctrl-shift-a`) a terminal's select-all. omega#76 asked for the
  shadowed lower-priority bindings to be resolved deliberately; this is that
  resolution written down, so a *new* shadow fails rather than quietly making
  the chord focus-dependent again.

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
- **Enforced by:** `aiur_is_a_single_dark_theme`,
  `default_themes_exist_in_shipped_assets`, and
  `the_shipped_theme_defaults_are_the_omega_themes`.
- **Amended 2026-07-25 (omega#73): the check was reading the fallback, not the
  value that ships.** `default_themes_exist_in_shipped_assets` reads
  `DEFAULT_LIGHT_THEME` and `DEFAULT_DARK_THEME`, which `theme_settings`
  consults only when *no* settings layer names a theme.
  `assets/settings/default.json` is the base settings layer and always names
  one, so `"light": "Ayu Light"` / `"dark": "Aiur"` there are what actually
  select the theme — and they were unchecked. A rebase restoring `"One Light"`
  / `"One Dark"` in that file would have shipped One Dark with every check
  green, because One Dark is still a shipped theme and both constants would
  still have said Aiur. The new check asserts the shipped values name shipped
  themes *and* agree with the constants, so a half-revert of either mechanism
  fails.
- **The same values live in a second shipped file, and nothing read that one
  either.** `assets/settings/initial_user_settings.json` is the template copied
  into a *new user's own* settings file on first start, and it also names
  `"Ayu Light"` / `"Aiur"` (`3493676d71`). A revert there is the more durable
  of the two: it lands in the user layer, which overrides the base layer, so
  correcting `default.json` afterwards would not undo it. Both files are now
  read by the same check.
- **Inherited test reconciled, 2026-07-25 (omega#73).**
  `workspace::test_toggle_theme_mode_persists_and_updates_active_theme` seeds a
  static theme, toggles the appearance mode, and asserted the static-to-dynamic
  migration produced `{"light": "One Light", "dark": "One Dark"}` — the
  pre-Omega defaults. It was **red on `main`**, and had been since this delta
  landed. It now reads `DEFAULT_LIGHT_THEME` and `DEFAULT_DARK_THEME` rather
  than naming themes, because what that test owns is the migration behaviour
  and what this delta owns is the values.
  **It also got stronger, not just green.** Upstream seeds `"One Light"` and
  expects `"One Light"` in the light slot, so upstream cannot distinguish *"the
  slots were filled from the defaults"* from *"the seeded static theme was
  carried into a slot"*. Under Omega's defaults those are different strings, so
  the distinction is real; the seed is now a named constant with an `assert_ne!`
  against both defaults, so a future default change cannot quietly collapse the
  test back into that ambiguity.

### OMEGA-DELTA-0017 — No competitor's name in the packaged `Info.plist`

- **Upstream Zed:** `crates/zed/resources/info/Permissions.plist` and
  `DocumentTypes.plist` name Zed in thirteen strings — twelve
  `NS*UsageDescription` values and `CFBundleTypeName`.
- **Omega, before this:** the fork inherited all thirteen verbatim and shipped
  them signed and notarized in `0.2.0-rc10`. `cargo-bundle` merges every file
  in that directory into `Contents/Info.plist`
  (`osx_info_plist_exts = ["resources/info/*"]`).
- **Omega now:** all thirteen name Omega, and both the source tree and the
  packaged bundle are gated.
- **Why:** macOS renders an `NS*UsageDescription` inside *its own* permission
  dialog. The first time Omega asked for the microphone, the operating system
  told the owner that an application in **Zed** wanted it — a competitor's
  product name, system-modal, presented under our Developer ID signature.
  `CFBundleTypeName` shows in Finder's Get Info and Open With menu. See
  omega#83.
- **Why nothing caught it:** `script/bundle-omega-rc` scanned the packaged app
  for exactly three *identity* literals (`BUZZ_PRIVATE_KEY`, `identity.key`,
  `get_nsec`). There was no brand gate at all, so no packaging step had ever
  read `Info.plist`. Every prior brand check on omega#16 compared strings
  inside the compiled executable, and these strings are not in the executable.
  The rc5 and rc6 evidence tables were true and the product still shipped it.
- **The gate reads values, not a list of keys**, and walks the whole fragment
  directory rather than a list of known files, so a brand-new key with a Zed
  string in a file nobody has heard of fails the same way.
- **Enforced by:** `no_info_plist_value_names_a_competitor` and
  `the_plist_fragment_parser_reaches_real_values` in `crates/omega_deltas/`
  (source tree), `script/verify-omega-brand --app` called from
  `script/bundle-omega-rc` (packaged bundle), and
  `the_packaging_path_runs_the_brand_gate`, which fails if the bundle script
  stops calling it. Policy is shared in `script/omega-brand-gate.json`.

### OMEGA-DELTA-0018 — No competitor's mark in the shipped icon set

- **Upstream Zed:** ships `zed_assistant.svg`, `zed_agent.svg`,
  `zed_agent_two.svg`, `zed_predict*.svg` and `zed_src_*.svg`, several of which
  draw the Zed **Z**, behind `IconName::Zed*` variants.
- **Omega, before this:** `IconName::ZedAssistant` and `IconName::ZedAgent`
  rendered a Zed logo mark on three status-bar buttons of the running
  `0.2.0-rc10`, and the same mark appeared in the conversation view, the
  sidebar, the model selector and the edit-prediction button.
- **Omega now:** those variants are `IconName::OmegaAgent`, `OmegaAgentTwo`,
  `OmegaAssistant` and `OmegaPredict*`, drawn with the Ω letterform taken
  verbatim from `assets/images/omega_logo.svg` — the same artwork as the app
  icon — keeping each icon's upstream affordance (the assistant sparkles, the
  `2`, the prediction chevrons and arrows, the error cross, the disabled
  slash). `zed_src_custom.svg` and `zed_src_extension.svg` carry no Z and were
  renamed to `src_custom.svg` and `src_extension.svg`.
- **`ai_zed.svg` stays**, and is recorded as a third-party allowance in
  `script/omega-brand-gate.json`. It labels the Zed **base-keymap preset**
  (beside VS Code, JetBrains and Sublime Text) and Zed's own model provider,
  exactly as `ai_anthropic.svg` labels Anthropic. Naming somebody else's
  product is not Omega presenting itself as that product.
- **Why nothing caught it:** a logo carries no text. No scan of the source tree
  and no scan of the compiled executable can see one, and every brand check on
  omega#16 through rc6 was a string comparison. See omega#84.
- **Why the gate is built the way it is.** Two designs were available: pin the
  digests of the shipped icons, or forbid competitor-named icon identifiers and
  assets. **Only the name rule would have caught these three**, because digest
  pinning catches a *change* from whatever was pinned, and what would have been
  pinned in rc10 is the Zed artwork itself. So the name rule is the gate, over
  a complete inventory: `assets/icons/` and the `IconName` enum are a
  bijection, enforced by the icons crate's own `test_all_icons_exist` and
  `test_no_dangling_icons`, and both halves are checked, because renaming only
  the file leaves the next rebase an identifier to restore the artwork under.
  The digest pin is kept **as well**, for the one hole the name rule cannot
  see: Zed artwork placed inside a correctly named Omega file.
- **Enforced by:** `no_shipped_icon_carries_a_competitor_name` and
  `the_omega_marks_are_the_reviewed_artwork` in `crates/omega_deltas/` (source
  tree), and `script/verify-omega-brand --app` from `script/bundle-omega-rc`,
  which checks the packaged executable's embedded rust-embed asset paths *and*
  asserts the reviewed artwork bytes are the bytes that were built.
- **What these gates do not cover.** Neither gate can recognise a competitor's
  drawing under a name nobody has flagged; the digest pin only says the shipped
  bytes are the reviewed bytes, and only for the icons listed in
  `reviewed_marks`. Nothing here inspects rendered pixels. `.icns` and `.png`
  app artwork is pinned separately, by the icon-family manifest in the release
  record, not by this gate. The packaged half runs against the macOS bundle
  only, so the Linux `.desktop`, Flatpak, Snap and Windows resources under
  `crates/zed/resources/` are unchecked. And no name is forbidden unless it is
  written down in `script/omega-brand-gate.json`. Rendered review of a
  candidate is still an owner step, not a mechanical one.

### OMEGA-DELTA-0019 — A window with nothing to restore opens on the agent

- **Upstream Zed:** `restore_or_create_workspace` in `crates/zed/src/main.rs`
  answers a window with no restorable session by calling
  `Editor::new_file(...)`, so the first thing a new user meets is an empty
  untitled buffer. The only exception is `restore_on_startup: "launchpad"`,
  which opens no content at all.
- **Omega:** the same two call sites call
  `agent_ui::AgentPanel::open_front_door(window, cx)`, which focuses the agent
  panel and activates a new thread. The launchpad behaviour is untouched —
  overriding it would be Omega ignoring a setting the user set, which is a
  different bug from the one this fixes.
- **Why:** owner UX direction on omega#76 — *"`cmd-shift-a` opens the main New
  Agent Thread screen, and the app defaults to showing that screen — welcome as
  new agent chat, standard chat input, typing immediately."* Omega is an agent
  product that inherited a text editor's front door. A blank buffer asks the
  user to already know what they came to do.
- **Enforced by:** `crates/omega_deltas/src/omega_deltas.rs`,
  `a_fresh_window_opens_on_the_agent`, and the typed rule it mirrors in
  `crates/omega_front_door`, `launch_surface`.
- **What this delta does not yet deliver.** Landing on the agent panel is not
  the same as landing on a focused composer. `AgentPanel::activate_new_thread`
  returns early when no project is open (`has_open_project`, one of seventeen
  such guards), and the no-restorable-session path is by definition the
  no-project case, so a genuinely fresh install lands on the agent panel's
  "Open Project / Clone Repository" state rather than on a composer. Making a
  thread bind to a project lazily, on its first workspace-touching action, is
  the remaining half of omega#76 and is not claimed here. A window that
  restores a project reaches the composer.

### OMEGA-DELTA-0020 — Full Auto is a surface of the chat panel, not a panel of its own

- **Omega, before this:** `FullAutoPanel` was a dock panel in its own right,
  registered in `initialize_panels` in `crates/zed/src/zed.rs`, with
  `DockPosition::Right`, a 520px default width, its own Ω dock button
  tooltipped "Full Auto", `activation_priority` 8, and its own
  `full_auto_ui::init` registering `full_auto_panel::ToggleFocus` and
  `full_auto_panel::OpenLauncher` against it. The agent panel's new-thread menu
  held a "Full Auto" item that dispatched the user out of chat and into that
  panel.
- **Omega now:** `agent_ui::AgentPanel` owns a retained `FullAutoPanel` entity
  and renders it as one of its own surfaces. The dock registration is gone,
  and both `full_auto_panel::` actions are answered by the agent panel, so a
  keymap or command-palette invocation that worked before still works. The
  views themselves did not change: `crates/full_auto_ui` still renders every
  control it rendered, under a new parent.
- **Why:** owner direction, 2026-07-25 — *"I don't actually want a Full Auto
  panel, it should be folded into whatever the chat UI for Omega is - you can
  decide how to handle this."* Full Auto is one of the three admitted executor
  classes, so a user starting a lane from chat is the router doing its job.
  Two destinations for agent work made the user choose a destination before
  choosing a task.
- **Why not a composer mode flag.** That is the obvious way to fold a surface
  into a composer and it is the wrong one. A flag is a boolean the send path
  reads, so anything able to set it can start a run: a slash command, a
  restored draft, a model-authored composer insertion. Owner gate 8 says only
  an explicit human action may start Full Auto authority. The fold therefore
  keeps a dedicated entry and a dedicated Start button — two human gestures —
  and moves only where the entry lives. `full_auto_is_not_a_composer_mode_flag`
  in `crates/full_auto_ui` states the surviving half of that law and now
  actually checks it.
- **What the fold costs.** Every *control* survives, which
  `every_full_auto_affordance_is_mapped` in `crates/omega_front_door` proves
  against the source rather than against its author's memory. Two capabilities
  that were not controls do not survive, and are recorded in `FOLD_COSTS`:
  independent dock placement, and reading a run's full detail beside a chat
  thread at the same time. Active runs still list on the monitor rail, so
  noticing a run is preserved; reading one in full alongside a thread is not.
- **The `Panel` implementation is deliberately kept** on `FullAutoPanel`, so a
  re-dock is a registration line rather than a rewrite.
- **Enforced by:** `crates/omega_deltas/src/omega_deltas.rs`,
  `full_auto_is_folded_into_the_chat_panel` and
  `only_a_click_listener_starts_a_full_auto_run`.

### OMEGA-DELTA-0021 — A thread names the executor that did its work

- **Upstream Zed:** a thread shows the agent's display name and icon in the
  panel chrome, and nothing else. That is enough when the panel is Zed's own
  agent talking to a model the user picked in the same window.
- **Omega, before this:** Omega presents one chat surface over three executor
  classes — the native agent loop, external ACP agents such as `codex-acp`, and
  `omega-effectd` engine lanes. All three rendered identically. Output produced
  by another company's coding agent, on another company's model, dispatched by
  a Full Auto run the user was not watching, appeared in an Omega window and
  read as Omega's own work. Nothing in the surface said otherwise.
- **Omega now:** every thread carries an executor line above its entries,
  naming the runtime class, the executing agent, the provider and model where
  the executor reports them, and the engine run where there is one. The line is
  rendered from `omega_front_door::ExecutorDisclosure` on every draw.
- **Why:** honest attribution before any routing intelligence. Omega Agent is a
  router that owns routing, disclosure and receipts and owns no execution
  (omega#74, admitted by the owner 2026-07-25). A router that does not disclose
  what it routed to is indistinguishable from a first-party agent that did the
  work itself, and the difference is a claim about authorship.
- **Why it is a record and not a string, which is the binding part.** The owner
  accepted, in the same admission, that the first-party agent does **not** sign
  with its own principal and projects onto the owner's record — *on the
  condition that disclosure is stored as a typed record that a label renders,
  never as a label string.* That condition is the only reason the choice stays
  cheap to reverse. A record of parts can be handed to a signer later; a stored
  line `"engine_lane · codex-acp · run.77"` cannot, because recovering the
  parts means parsing prose, so switching to a signing principal would mean
  rewriting every thread record instead of adding a signer. A label-string
  disclosure would silently convert a reversible owner decision into an
  irreversible one. `executor_disclosure_is_a_typed_record_not_a_label_string`
  asserts the field set exactly, rather than scanning for suspicious names: a
  `line`, `text` or `summary` field would be a rendered label under a name no
  denylist anticipated.
- **Nothing new is persisted, and that is the design rather than an omission.**
  omega#77's falsifier names a new GPUI-owned durable store as a failure. Every
  part of the record already has a durable home — the agent id in
  `sidebar_threads`, the provider and model in `DbThread.model` restored by
  `Thread::from_db`, the run reference in `full-auto-host-correlation.json`,
  reloaded at startup — so the record is a projection over them. A projection
  cannot disagree with the thread it describes; a cached copy can, and a
  disclosure that disagrees with its executor is worse than none.
- **The classification is a checked downcast, not a name match.** `agent_id()`
  is a display identifier that `OMEGA-DELTA-0020`'s neighbour omega#75 is
  renaming, and that any extension can set to anything. Deciding *what ran*
  from it would make the disclosure a string comparison on a label. The
  fallback for an unrecognised connection is `ExternalAcp`, never `NativeLoop`:
  guessing wrong towards "not ours" costs precision, guessing wrong towards
  "ours" is exactly the dishonest first-party claim this delta exists to stop.
- **`provider` and `model` became optional here.** `AcpConnection` does not
  implement `AgentConnection::model_selector`, so a `codex-acp` thread has no
  model to report. With required strings the only options were to fabricate a
  model or to fail `is_coherent()` on every external thread; the line now says
  "model not disclosed" instead. An *empty* identifier stays incoherent, so the
  distinction between "not disclosed" and "built from a missing value" is
  preserved rather than flattened.
- **The binding is an extension trait, not a fork.** `AcpThread` is upstream
  and unchanged: `ThreadExecutorDisclosure` is implemented for it from
  `crates/agent_ui/src/omega_executor_disclosure.rs`. A rebase that reshapes
  the shared thread type breaks the `impl` and fails the build, rather than
  silently dropping the disclosure.
- **What this does not cover.** The executor line is on the thread surface. A
  thread listed in the sidebar, exported, or shared shows its agent name and no
  executor line. The model is disclosed only where the executor reports it, so
  most external ACP threads disclose a class and an agent rather than a model.
  And no check here inspects rendered pixels: the line is asserted to be
  constructed and drawn, not to be legible.
- **Enforced by:** `crates/omega_deltas/src/omega_deltas.rs`,
  `executor_disclosure_is_a_typed_record_not_a_label_string`,
  `the_thread_surface_renders_the_executor_line_from_the_record`, and
  `the_disclosure_is_an_extension_trait_and_not_a_fork_of_the_shared_thread`;
  plus `a_restarted_process_still_discloses_the_lane_that_owns_a_thread` in
  `crates/agent_ui`, which empties the process-local lane index exactly as a
  process exit does and rebuilds the disclosure from the journal on disk.

### OMEGA-DELTA-0022 — No competitor's identity in any shipped asset or any command-palette label

- **Upstream Zed:** ships `assets/images/zed_logo.svg` and
  `assets/images/zed_x_copilot.svg` behind `VectorName::ZedLogo` and
  `VectorName::ZedXCopilot`, declares every application-level action in the
  `zed` namespace, and titles the Copilot device-code flow
  "Use GitHub Copilot in Zed".
- **Omega, before this:** all three shipped in `0.2.0-rc11`, signed and
  notarized, **after** OMEGA-DELTA-0017 and OMEGA-DELTA-0018 had been added and
  reported green.
  1. The Zed logo **rendered in a release build**.
     `workspace: open component preview` was in the release command palette
     with no dev gate — the product gates `dev::ToggleInspector` and
     `dev::ResetOnboarding`, but not this — and drawing the `Vector` preview put
     the Zed **Z** on screen. The compatibility allow-list recorded
     `VectorName::Zed*` as `source_only`, meaning nothing rendered it. That was
     false.
  2. **"Use GitHub Copilot in Zed"** appeared as a floating-window title bar, as
     the modal `Headline`, and beside the Zed × Copilot lockup — one user-facing
     surface, three presentations — and was in **no** allow-list entry, while
     `Welcome to Zed` and `About Zed` were listed `blocked`. Same class, missed.
  3. The **`zed:` command namespace was user-facing**: `zed: about`,
     `zed: quit` and `zed: get merch` were visible in the palette while the
     allow-list entry claimed these "are not user-facing product copy". The
     targets were already correct (`MERCH_URL` → openagents.com, the About
     window titled `About Omega`); the label and the classification were the
     defect.
- **Omega now:**
  1. Both images are deleted, `VectorName::ZedLogo` and `VectorName::ZedXCopilot`
     are gone, the `Vector` preview draws `OmegaLogo`, the Copilot modal draws
     `IconName::Copilot`, and `workspace: open component preview` is registered
     only under `debug_assertions` — hidden from the palette and refused in a
     release build, the way `dev::ToggleInspector` already was. Removing the
     artwork and gating the surface are both done, because either alone leaves
     the other half of the failure standing.
  2. Every presentation of the Copilot surface names Omega. The GitHub Copilot
     **integration is retained**; the *"in Zed"* framing is not. The claim is
     recorded `blocked` in the compatibility allow-list, and blocked claims are
     now read back against the whole tree.
  3. Sixty-six actions moved from the `zed` namespace to `omega`, plus
     `cli::RegisterZedScheme` → `RegisterAppScheme`, `feedback::EmailZed` →
     `EmailOpenAgents`, `zed_predict_onboarding::OpenZedPredictOnboarding` →
     `omega_predict_onboarding::OpenOmegaPredictOnboarding`,
     `zed::OpenZedRepo` → `omega::OpenRepository` (it opens the *Omega*
     repository) and `zed::OpenZedUrl` → `omega::OpenAppUrl`. Every retired name
     survives as a `deprecated_aliases` entry so an existing user keymap still
     resolves, and the shipped keymaps dispatch the new names. The
     `zed-keybind-context` grammar directory, embedded in the binary and
     surfaced as the language name "Zed Keybind Context", is now
     `keybind-context` / "Keybind Context".
- **Why the previous gate did not catch any of it.** OMEGA-DELTA-0018
  inventories `assets/icons/*.svg` and the `IconName` enum. `assets/images/*.svg`
  and `VectorName` are outside it entirely — and that is exactly where the
  surviving artwork lived. Nothing had ever read an action declaration, and
  nothing read the allow-list's own `blocked` entries back against the tree.
  **A gate scoped to one directory reports green about that directory and says
  nothing about the product.** This was the third time Zed branding survived a
  gate that truthfully reported clean: rc5/rc6 scanned the packaged app for
  three identity literals, rc10 added the packaged `Info.plist` and the icon
  set, and rc11 still shipped all three of the above.
- **How this inventory is complete rather than enumerated.** Each of the four
  inventories is *derived from the thing that decides what ships*, and each
  carries an anti-vacuity guard that fails if the derivation stops working:
  - **Every embedded file.** The assets tree plus every directory any
    `#[folder = "…"]` in the repository points at, resolved against the crate
    root the way `rust-embed` resolves it. A rust-embed folder added tomorrow is
    inside the gate the day it is added. Guards: a floor on the file count, and
    a failure if no embed declaration is found at all.
  - **Every enum that names an embedded asset.** Discovered by finding
    `format!("<dir>/{…}")` for a directory that is in the inventory above, then
    reading that file's enums. `IconName` and `VectorName` are both found this
    way; the policy no longer names a file. Guard: `required_discoveries` fails
    if either stops being found.
  - **Every command-palette label.** Every `actions!(namespace, [...])` block
    and every `#[action(namespace = …)]` derive in the tree — the only two ways
    to declare a gpui action, so this is the complete set of
    `namespace: action name` labels the palette can display. Guards: a floor of
    1000 declarations and `required_actions`.
  - **Every blocked public claim.** Read out of the compatibility allow-list and
    searched for across `crates/` and `assets/`, with four named corpus files
    exempt (the allow-list and the tests that assert the strings' absence), each
    asserted to exist so the exemption list cannot quietly grow.
- **References deliberately kept.** `ai_zed.svg` labels Zed's base-keymap preset
  (beside VS Code, JetBrains and Sublime Text) and Zed's hosted model provider,
  exactly as `ai_anthropic.svg` labels Anthropic — recorded as a third-party
  allowance in both the icon and embedded-asset sections. The `zed://` URL
  scheme still resolves so existing deep links open. `ZED_*` environment
  variables, `.zed` project folders and the `zed`/`zed_actions` crate names stay
  as fork seams. Every `zed::` action name survives as a deprecated alias.
  We are removing Zed **as our identity**, not erasing that Zed exists.
- **Enforced by:** `no_embedded_asset_carries_a_competitor_name`,
  `no_asset_name_enum_carries_a_competitor_name`,
  `no_command_palette_label_names_a_competitor`,
  `the_retired_action_namespace_still_resolves`,
  `blocked_public_copy_appears_nowhere_in_the_tree` and
  `the_component_preview_is_gated_to_dev_builds` in `crates/omega_deltas/`
  (source tree), `no_vector_name_carries_a_competitor_name` in `crates/ui/`, and
  `script/verify-omega-brand --app` from `script/bundle-omega-rc`, which scans
  the packaged executable's embedded asset paths for the forbidden token and
  asserts the current `omega::` action labels were actually built. It rejects
  the installed `0.2.0-rc11` on `images/zed_logo.svg`, `images/zed_x_copilot.svg`
  and three missing `omega::` labels. Policy is shared in
  `script/omega-brand-gate.json`.
- **What these gates still do not cover.** No gate recognises a competitor's
  drawing under a name nobody flagged — the digest pins only say the shipped
  bytes are the reviewed bytes, and only for the files in `reviewed_marks`.
  **Nothing inspects rendered pixels.** Arbitrary user-facing prose is not under
  a complete inventory: the string half enforces the allow-list's `blocked`
  claims, which is a written-down list, so a *new* sentence naming Zed as the
  product fails only once somebody adds it — that is exactly how
  "Use GitHub Copilot in Zed" survived, and widening it further needs the 168
  remaining brand-bearing prose literals classified, which this lane did not do.
  Action *doc comments*, which the palette does not show but the keymap editor
  does, are unchecked. The packaged half checks action labels by **presence of
  the current ones**, not by exhaustive absence: a stripped binary's string
  table has no separators and no type information, so `zed::About` appears as
  `zed::AboutOpens` and a module path like
  `zed_edit_prediction_delegate::ZedEditPredictionDelegate` is indistinguishable
  from an action name. The absence rule is enforced on the source, where a
  declaration can be read. The packaged half runs against the macOS bundle only, so
  Linux `.desktop`, Flatpak, Snap and Windows resources are unchecked. And no
  name is forbidden unless it is written in `script/omega-brand-gate.json`.
  Rendered review of a candidate is still an owner step, not a mechanical one.
- **Falsified.** Each defect was reintroduced and the gate watched to fail, then
  restored: the artwork back in `assets/images/`, `VectorName::ZedLogo` back on
  the enum, `Use GitHub Copilot in Zed` back in the modal, `actions!(zed, …)`
  back in `zed_actions`, and the dev gate removed from the component preview.
  The widened gate also rejects the installed `0.2.0-rc11` itself, on
  `images/zed_logo.svg`, `images/zed_x_copilot.svg` and three missing `omega::`
  action labels — the same way the previous gate was proven by rejecting rc10.

### OMEGA-DELTA-0023 — The application bundle is stapled, not only the disk image

- **Upstream Zed:** notarizes and staples the release archive.
- **Omega, before this:** `script/bundle-omega-rc` submitted and stapled the
  **DMG only**. `stapler validate /Applications/Omega.app` on the installed
  product reported no ticket, so Gatekeeper acceptance of the installed
  application could rest on an online lookup with Apple.
- **Why it matters:** omega#16 requires **offline first start**. A DMG ticket
  covers the disk image the owner throws away; it does not travel with the
  application that ends up in `/Applications`. Without a ticket stapled to the
  `.app`, first launch on a machine with no network is not provably accepted,
  and the offline-start scope item cannot be closed honestly.
- **Omega now:** the signed `Omega.app` is zipped, submitted to `notarytool`,
  and stapled **before** the disk image is built, so the DMG is assembled from
  the already-stapled application. The DMG is then signed, submitted and
  stapled as before. Both are validated with `stapler validate` afterwards, and
  the release record carries `notarization.app_stapled` alongside
  `notarization.stapled` so the two cannot be conflated.
- **Enforced by:** `the_packaging_path_staples_the_application` in
  `crates/omega_deltas/`, which fails if the bundle script stops stapling or
  stops validating the `.app`.

### OMEGA-DELTA-0024 — Omega Agent is the first-party agent identity

- **Upstream Zed:** the native `AgentConnection`, selector label, component
  preview, thread placeholder, and evaluation client identify the runtime as
  `Zed Agent`.
- **Omega now:** those reachable surfaces identify the admitted first-party
  orchestrator as **Omega Agent** and use the reviewed `OmegaAgent` icon. The
  identity symbol is `OMEGA_AGENT_ID`, renamed along with the value it holds:
  the icon rename already paid for the lesson that renaming only the string
  leaves the next upstream rebase an obvious name to restore the old identity
  under.
- **Boundary:** this delta renames the identity projected by the inherited
  native executor. It does not turn `NativeAgent` into the router, make the
  inherited `telemetry_id` an OpenAgents service identity, change run
  authority, or claim that the later routing and receipt packets exist.
- **Telemetry:** `NativeAgentConnection::telemetry_id` still reports the
  inherited `"zed"` key, because it keys an analytics series and rewriting it
  would break the series without renaming anything a user sees. The rule is
  that it stays out of the identity path, and it is now asserted rather than
  assumed.
- **Docs:** reachable agent documentation names Omega Agent, and the page
  moved from `docs/src/ai/zed-agent.md` to `docs/src/ai/omega-agent.md`. The
  file *name* is checked as well as the contents: falsifying this delta found
  that restoring the old name passed every check, because a renamed file reads
  as clean either way.
- **Kept on purpose:** `assets/icons/ai_zed.svg` still labels Zed's
  base-keymap preset and Zed's hosted model provider, `zed_urls.rs` still
  links Zed's own docs, and `gpui_macros` still records that upstream
  generated a file with Zed's agent. Removing Zed as *our* identity is not
  erasing Zed as a thing that exists; the allowances in
  `script/omega-brand-gate.json` name each one with a reason.
- **Enforced by:** `the_first_party_agent_identity_is_omega_agent`,
  `no_phrasing_presents_zed_as_the_first_party_agent`, and
  `the_inherited_telemetry_id_is_not_the_product_identity` in
  `crates/omega_deltas`, plus the existing high-risk public-branding scan in
  `crates/app_identity`.
- **Not covered:** all three checks read the source tree. None of them opens a
  packaged application, so nothing here proves the shipped `.app` is clean —
  that is the packaged brand gate's job (OMEGA-DELTA-0017/0018), and it scans
  `brand.words`, not this phrase family. The phrase list is a fixed set of
  substrings: a new way of writing the same claim passes until somebody adds
  it, exactly as `0.2.0-rc10`'s three-literal scan passed. The scan reaches
  only the roots and file extensions named in the policy, which is how a
  `.py` file in the eval harness kept saying "the Zed agent's" through a
  first pass of this delta. Files outside `scan_roots` — `.github/`,
  `crates/zed/resources/` — are unread here. And no check looks at a rendered
  pixel, so a label that is correct in source and truncated, mis-cased, or
  absent on screen still passes.

### OMEGA-DELTA-0025 — A wrapped harness runs only bytes Omega measured, and only if the pins admit them

- **Upstream Zed:** `LocalRegistryArchiveAgent` resolves whichever version the
  ACP registry document currently advertises, downloads it into a versioned
  cache directory, and hands back the command. The registry's `sha256` is
  checked at download time when the document supplies one, and then nothing is
  recorded: no digest, no receipt, and no way for the owner to say *not that
  version*. The extracted tree is not re-read on later launches at all, so a
  file replaced after the download runs unnoticed. A registry agent runs with
  the tool permissions of the thread that started it.
- **Omega now:** two gates around the same path, both in
  `crates/project/src/agent_server_store.rs`.
  - Before anything is fetched, `authorize_version_fetch` reads the pin ledger
    and refuses a version the owner froze out. This is a prefilter, and
    `the_prefilter_never_admits_what_the_gate_refuses` proves it can only
    refuse what the gate below would also refuse.
  - After the tree is on disk and before the command is returned,
    `authorize_installed_harness` hashes **every regular file in the installed
    tree**, folds them into one digest bound to their paths, and refuses unless
    the owner's pins admit that digest. It runs on every launch, not only on
    install, so bytes replaced after an install receipt was written are caught.
  Both write a receipt, permitted or refused.
- **Why a digest and not a version.** A pin on a version string is satisfied by
  a release re-tagged in place, which is precisely the substitution omega#81's
  falsifier describes. `a_retagged_release_does_not_satisfy_a_pin` holds that
  shut. The version is still compared, because a refusal has to name something
  an owner recognises — but it is not what authorises the run.
- **Why the receipt is a measurement.** `MeasuredDigest` has exactly two
  constructors, `measure(bytes)` and `measure_tree` over already-measured
  digests. There is no `From<String>`, no `FromStr`, and no `Deserialize`: a
  value that arrived as text is not a measurement this host made, and giving it
  the same type would erase the only distinction that matters. The `Applied`
  receipt input takes a `MeasuredDigest` rather than a string, so a receipt
  claiming an update was applied structurally cannot be written by a caller
  that never hashed the bytes. `observedAtMs` is stamped from one `now_ms()`
  reading taken where the action happened; nothing else that reaches the
  receipt writer carries a time, so there is no path by which a registry
  document or a settings file can supply one.
- **No backfill.** A log record from a schema this build does not know decodes
  to `ProvenanceUnavailable`, which has no digest field, and
  `verify_installation` refuses it. An installation with no receipt at all —
  every harness installed before this delta — is `Unattested` and stays that
  way until a maintenance action measures it. The schema is read *before* the
  strict decoder runs, because `deny_unknown_fields` would otherwise classify a
  genuinely newer record as garbage, and a silent skip is the backfill this
  contract refuses.
- **Fails closed.** An unreadable pin ledger is not an unpinned machine:
  `PinState::Unreadable` refuses every action, so truncating one file does not
  unfreeze every harness. A tree that cannot be measured is refused whether or
  not a pin exists, because a machine with no pin is not a machine that
  consented to running unread bytes. Symlinks inside the tree are recorded by
  path but not followed, so a link planted in the install directory cannot
  attest bytes from elsewhere on the machine.
- **Visible.** A refusal is an `anyhow` error carrying
  `MaintenanceRefusal::reason()`, a sentence that names the pinned version, the
  version that was refused, and what to do. `update_affordance` returns
  `Disabled { reason }` with no way to build a disabled state without one —
  `0.2.0-rc11` bound `appendSystemNote` to `() => {}` on the framed provider
  path, so a refused handoff said nothing in the thread and a different model
  spent the owner's budget with no trace. A blocked update nobody can see is
  the same defect.
- **Enforced by:** `a_measured_digest_cannot_be_built_from_a_string`,
  `the_external_harness_launch_path_is_gated_on_a_measurement`, and
  `the_enforcement_path_writes_receipts_only_from_decisions` in
  `crates/omega_deltas`; 46 contract tests in `crates/omega_harness`; and 8
  filesystem tests in `crates/project/tests/integration/harness_maintenance.rs`
  that drive the real enforcement functions against a `FakeFs`.
- **Falsified.** Each behaviour was reverted and the check watched to fail, then
  restored: the tree measurement replaced with a single-file measurement (the
  sidecar test goes red), `PinState::Unreadable` folded into `Unpinned` (the
  corrupt-ledger tests go red), the digest comparison dropped so only versions
  are compared (`a_retagged_release_does_not_satisfy_a_pin` goes red), the gate
  moved below the command it gates (the delta check goes red), and
  `impl From<String> for MeasuredDigest` added (the delta check goes red).
- **What this does not cover.** `LocalRegistryNpxAgent` is **not** gated: an
  npx-distributed harness resolves and caches inside the node runtime's own
  cache, which Omega does not own a directory for, so there is no tree to
  measure and no gate to hold. `LocalCustomAgent` — a command the user wrote in
  settings — is not gated either, and should not be: the user named the binary.
  Neither the ledger nor the receipt log is signed, so an attacker who can
  write to `paths::external_agents_dir()` can rewrite both; this raises the bar
  from *no record* to *a record that has to be forged*, and no further. There
  is **no settings UI** for taking or removing a pin yet — the ledger is a file,
  and `MaintenanceAffordance` is the typed state a front-door control will
  render, not a rendered control. Nothing here verifies a publisher signature:
  the digest says the bytes did not change since Omega measured them, not that
  they are the bytes the publisher built.

### OMEGA-DELTA-0026 — The shipped defaults reach no Zed service

- **Upstream Zed:** `server_url` is `https://zed.dev`, `auto_update` is `true`,
  `edit_predictions.provider` is `"zed"`, and `auto_install_extensions`
  installs the `html` extension from Zed's extension registry on first start.
- **Omega:** `server_url` is `https://services.openagents.invalid`,
  `auto_update` is `false`, `edit_predictions.provider` is `"none"`, and
  `auto_install_extensions` is `{}`.
- **Why:** all four landed in one commit, `9e585569cb` ("Isolate Omega from Zed
  production services"), and they are one decision rather than four: the
  settings-layer half of the isolation that `OMEGA_ALLOW_ZED_SERVICES` gates in
  code. Omega has no update feed, no hosted edit-prediction service, and no
  extension registry of its own, so each of these defaults otherwise points a
  running Omega at a competitor's production host — with no account, no
  consent, and in `auto_update`'s case at the one host that can replace the
  binary.
- **The telemetry values from that same commit are `OMEGA-DELTA-0004`**, and
  were registered when the delta programme started two days later. These four
  were not, and have shipped unregistered since — including through the
  `0.2.0-rc10` and `0.2.0-rc11` brand reviews, which read icons, plists,
  actions and binaries, and never read a settings *value*.
- **`disable_ai` is not in this set.** The isolation commit also set it `true`;
  `87703b753a` set it back to `false` when registry ACP agents were enabled, so
  it matches upstream today and is not a divergence. The comment above it still
  differs, which is prose, not policy.
- **Enforced by:**
  `default_settings_enable_registry_acp_without_enabling_zed_production` in
  `crates/app_identity/src/service_isolation.rs`, which has asserted all four
  since the isolation commit and is the primary check; plus
  `the_service_isolation_defaults_are_still_the_omega_values` and
  `the_service_isolation_test_still_asserts_the_registered_defaults` in
  `crates/omega_deltas/`.
- **Why there is a second check, when the rule is to cite rather than
  duplicate.** Citing alone fails two ways here, and both are the failure this
  file exists for.
  1. The cited assertions can be deleted, and deleting an assertion turns a
     test green. `auto_update` and `auto_install_extensions` read as off-topic
     inside a test named for Zed *service* isolation and are exactly what a
     tidy-up removes. `the_service_isolation_test_still_asserts_the_registered_defaults`
     pins them, and pins the delta ID beside them.
  2. `cargo test -p omega_deltas` is the command this registry tells a reader
     to run. A delta whose only value assertion lives in another crate's test
     is green under that command while the value is reverted — a mechanism
     reporting green about less than the reader assumed, which is precisely the
     shape of every miss recorded above.
- **Not covered:** these are *defaults*. A user settings file, a project
  settings file, or an environment variable still overrides every one of them,
  and `ZED_SERVER_URL` overrides `server_url` without touching any file. This
  delta says what Omega ships, not what a running Omega cannot be told to do.

### OMEGA-DELTA-0027 — Codex ACP is configured out of the box

- **Upstream Zed:** `agent_servers` is `{}`. An external ACP agent is something
  the user adds.
- **Omega:** `agent_servers` declares `codex-acp` with `"type": "registry"`, so
  it resolves from the ACP registry at `cdn.agentclientprotocol.com` — an
  `approved` host in `crates/app_identity/fixtures/endpoint_allowlist.json`.
- **Why:** `bc87aec95c` routed Full Auto through Codex ACP, which makes this
  not an optional extra. It is one of the three executor classes
  `OMEGA-DELTA-0021` exists to disclose, and a Full Auto lane dispatched at an
  agent that is not configured does not fall back — it fails. Shipping it
  configured is the difference between Full Auto working on first launch and
  Full Auto reporting a missing agent.
- **Stated plainly: this is a default that reaches a third-party network
  service.** It is not covered by `OMEGA-DELTA-0026` and is not meant to be —
  that posture is about *Zed's* production services, not about Omega making no
  requests at all. `codex-acp` is an npx-published registry agent, so the first
  turn on it reaches `cdn.agentclientprotocol.com`, `registry.npmjs.org` and
  `nodejs.org` — all three `approved` in the same allow-list. A genuinely
  offline first start therefore does not get Codex. The offline-start
  requirement on omega#16 is about launching, not about every executor being
  reachable.
- **Enforced by:** `codex_acp_is_configured_by_default` in
  `crates/omega_deltas/`, and the `agent_servers` assertion in
  `default_settings_enable_registry_acp_without_enabling_zed_production`, which
  `the_service_isolation_test_still_asserts_the_registered_defaults` keeps in
  place.

### OMEGA-DELTA-0028 — The default icon theme is Omega's own

- **Upstream Zed:** the built-in icon theme is `Zed (Default)`, in both
  `DEFAULT_ICON_THEME_NAME` and the shipped `icon_theme` setting.
- **Omega:** both are `Omega (Default)`.
- **Why:** the owner opened `~/.config/omega-rc/settings.json` and it
  introduced the product as Zed (`3493676d71`). A settings file the owner opens
  is a product surface, and omega#16 forbids presenting Zed as the product
  there. The name is also what the icon-theme selector displays.
- **Why the check is about agreement rather than about the string.** The two
  values are coupled: `configured_icon_theme` looks the settings value up in
  the registry, and the registry's only built-in icon theme is registered under
  `DEFAULT_ICON_THEME_NAME`. A rebase that reverts one and not the other breaks
  nothing visible — `crates/theme_settings/src/theme_settings.rs` logs the
  lookup failure and falls back — so the product keeps working while shipping a
  competitor's name in the file the owner reads. Pinning the literal in a third
  place would catch a revert of *both* and miss the half-revert entirely. The
  check therefore asserts the two agree, and then asserts neither names a
  competitor, using the same `script/omega-brand-gate.json` word list as the
  packaged gate.
- **`base_keymap: "Zed"` deliberately stays**, and is why the check reads one
  key instead of scanning the settings file for brand words. That value names
  Zed's keybinding scheme, offered beside VS Code, JetBrains and Sublime,
  exactly as `ai_zed.svg` labels Zed's own model provider. Renaming it would
  misdescribe what the setting selects.
- **Not covered, and deliberately not registered:** the *comments* in
  `assets/settings/default.json`. Four of them were reworded from Zed to Omega
  alongside the value changes above (`3493676d71`, `9e585569cb`), and roughly
  sixty-five other lines in that same file still describe inherited behaviour
  by naming Zed. Registering the four would assert a policy — "the shipped
  settings comments say Omega" — that is not true of the file and that no check
  could enforce without classifying the other sixty-five, which is the
  prose-classification work `OMEGA-DELTA-0022` explicitly records as not done.
  They are drive-by edits, so they are flagged here rather than blessed with an
  ID. This delta covers the icon theme's *name*, which is a value the product
  renders into a selector and writes into the owner's settings file.
- **Enforced by:** `the_default_icon_theme_is_omegas` in
  `crates/omega_deltas/`.

### OMEGA-DELTA-0029 — Omega Agent routes deterministically, fails closed, and records why

- **Upstream Zed:** a thread is bound to whichever agent connection created it,
  for its whole life. There is one executor per thread and no decision to make,
  so there is nothing to explain and nothing to record.
- **Omega, before this:** Omega presents one chat surface over three executor
  classes, and which one a thread got was decided implicitly by which panel
  entry the user happened to open. `OMEGA-DELTA-0021` made a thread *name* its
  executor; nothing yet chose it on purpose, and nothing wrote down why.
- **Omega now:** `omega_front_door::router` is the routing law — a pure function
  from typed inputs (a user pin, the engine's last framed `get_capacity` answer,
  and which executors are connected) to a typed `RouteDecision`.
  `agent_ui::omega_router::OmegaAgentConnection` is the dispatch half: it
  implements `AgentConnection` and hands every method to the executor the
  decision names.
- **Pins are honoured.** An explicitly pinned executor that can serve is always
  used, whatever else is ready. A pin outranks an idle engine lane, because the
  engine being free is not a reason to move a turn a person placed.
- **Engine-down fails closed to the native loop, and says so.** Every way the
  engine can be unavailable — not running, timed out, an answer this build
  cannot read, at its active-run limit, no ready lane, a named lane that is
  busy, no executor connected for lanes — lands on the native loop with a typed
  fallback reason, keeps the pin it could not honour, and renders a line that
  says a pin was not honoured. A fallback the user cannot see is the same defect
  class as a handoff with no system note, which shipped in `0.2.0-rc11` because
  `appendSystemNote` was bound to `() => {}` on the framed path.
- **The decision is recorded.** Every decision is written to
  `agent-route-journal.json` under the Omega data directory in a canonical form
  that round-trips back into a typed decision, keyed by session, rewritten
  atomically. It carries **no clock**: a timestamp would make two identical
  decisions look different and would put a non-deterministic value beside a
  decision path whose whole point is that it is reproducible. A record that
  reads cleanly but describes an impossible decision — a fallback that claims an
  engine lane, an honoured pin naming a different class from the one that ran —
  is rejected rather than believed.
- **Determinism, and why it is not just a unit test.** The routing law reads
  nothing but its argument, walks only ordered slices, and resolves a choice
  among equally ready lanes by a total order on the lane reference rather than
  by the order the engine listed them — the engine's array order is not a stable
  input, so "the first available lane" would route the same thread two ways on
  two runs with nothing wrong with either.
  `the_routing_law_has_no_clock_no_randomness_and_no_hash_order` reads both
  source files for clocks, randomness, hash-map iteration, and the environment,
  because a behavioural test can only show that the inputs it happened to try
  agreed twice.
- **An unpinned thread never reaches an engine lane, on purpose.** Owner gate 8
  says no model-initiated path may start Full Auto authority, wherever that
  action lives. An engine lane *is* Full Auto authority, so a router that
  preferred a ready lane for an unpinned thread would be exactly that
  forbidden start, through a door nobody had flagged — which is how
  `full_auto_enable` survived until it was removed today. v1 therefore routes an
  unpinned thread to the native loop, always, and engine lanes are reachable
  only through a pin a person sets on a visible control. Model-advisory routing
  is out of scope for v1 by the packet's own terms; this is the shape that keeps
  it out.
- **The router owns no execution, and that is read off the source.**
  `the_router_owns_no_execution_and_starts_no_run` scans the dispatch file for
  execution vocabulary and for run-control verbs;
  `the_router_delegates_every_agent_connection_method` parses the
  `impl AgentConnection` block and fails if any method stops handing its work to
  an executor. A method that quietly grew a turn loop would still compile, still
  pass every behavioural test that did not call it, and still read as a router
  from its own module docs.
- **`omega-effectd` stays the sole run authority.** The engine's capacity answer
  is read, never written back, never cached as run state. The router projects;
  it does not own. A later engine answer does not rewrite a decision already
  recorded, so a turn does not move executors mid-thread because capacity
  changed between turns.
- **A routed thread carries the executor's connection, not the router's.**
  `OMEGA-DELTA-0021` classifies a thread by downcasting its connection, so a
  thread carrying the router would disclose the router as its executor — the
  exact first-party attribution claim omega#77 exists to stop.
- **Disclosure grew a part, not a caption.** `ExecutorDisclosure` gained
  `route: Option<RouteReason>`, a closed typed set, added to
  `EXECUTOR_DISCLOSURE_FIELDS`. `None` means the thread was not routed by the
  router, which is different from claiming a reason nobody recorded.
- **Enforced by:** `the_routing_law_has_no_clock_no_randomness_and_no_hash_order`,
  `the_router_owns_no_execution_and_starts_no_run`,
  `the_router_delegates_every_agent_connection_method`, and
  `the_route_decision_is_a_record_that_round_trips` in `crates/omega_deltas`;
  the routing-law suite in `crates/omega_front_door/src/router.rs`; and the
  dispatch and journal suite in `crates/agent_ui/src/omega_router.rs`.
- **Not covered.** The wiring this entry once listed as missing landed in
  `OMEGA-DELTA-0035`: the native agent entry now resolves to an
  `OmegaAgentConnection`, the panel polls `get_capacity` into
  `observe_capacity`, and a pin menu sits on every thread's disclosure line. Read
  `OMEGA-DELTA-0035` for what that wiring does and does not reach. Separately,
  the router can decide an engine-lane route it
  cannot itself dispatch, because engine lanes are started by a person on the
  Full Auto surface and driven by the host bridge rather than through
  `AgentConnection::prompt`; that gap is the named `engine_lane_not_connected`
  fallback rather than a substitution. And no check here looks at a rendered
  pixel, so a route line that is correct in source and truncated or absent on
  screen still passes.

### OMEGA-DELTA-0030 — A linked run shows its receipt chain in the thread

- **Upstream Zed:** an agent thread shows the turns the agent produced. There
  is no engine, no run authority, and nothing to link to, so there is nothing
  upstream to revert to here — only something to quietly leave out.
- **Omega, before this:** `OMEGA-DELTA-0021` gave every thread an executor
  line, and a thread executed by an `omega-effectd` lane names its run in it.
  Naming a run is not showing one. The reader was handed a reference and left
  to go somewhere else to learn whether the work was host-verified, refused, or
  merely claimed — and on the Full Auto surface, where the chain did render, a
  broken chain rendered as one sentence: "Evidence chain unavailable or
  cross-links did not verify." Four distinct situations, one word.
- **Omega now:** a thread an engine lane executed renders, above its entries,
  the run's reference, the agent the run delegated to, the run's lifecycle, and
  its receipt chain — the nine omega#43 hops in normative order when the chain
  is complete, and `chain: unavailable` with a named reason when it is not. The
  rows use the same label/value grammar as the receipt inspector, so a chain in
  a thread reads like a chain in the receipt pane rather than like a second
  format.
- **A refusal is shown, never hidden and never rounded up.** The four
  record-level reasons are not interchangeable. `hop_missing` says the host
  never produced the step; `hop_mismatched` says two records tell two stories
  about one run; `hop_private` says the host produced it and this surface may
  not carry it; `self_reported` says the run vouched for itself. Collapsing
  them is how a *contradicted* chain gets read as merely *incomplete* — the
  exact confusion omega#47's reason vocabulary was written to end. And a
  broken chain still draws rows: a surface that renders nothing when it
  cannot verify has told the reader nothing, and silence reads as "no run".
  One malformed record must not blank the surface.
- **The engine stays the sole run authority, and staleness is the proof.**
  omega#80's falsifier is "a run's source of truth ends up in a panel entity".
  The thread stores the engine's *records* and the instant it read them, never
  a projected link, and re-derives the link on every draw. Past
  `THREAD_RUN_LINK_MAX_AGE_MS` — five missed reads of a three-second poll — the
  link renders `host_unavailable` instead of the last chain it saw. A cached
  conclusion would have quietly outlived its source; a cached record with an
  expiry cannot. A state the contract does not model is not translated into the
  nearest one either: a run reporting `acknowledged` reports no state at all,
  because a relay's acknowledgement is a statement about a message, not about
  work.
- **A terminal state is not a receipt.** `ThreadRunLink::is_receipted` requires
  a complete chain *and* an allowing authority decision. A run whose engine
  state is `succeeded` and whose chain is unavailable renders both facts and is
  not claimed as receipted; a chain that resolves to `allowed: false` renders
  complete and is still not receipted. Together those are the second half of
  omega#80's falsifier.
- **The chain comes from the omega#47 producer, not from a second
  implementation.** `workroom_receipts::project_issue31_evidence_pair` is the
  single-pair entry point to the same code the phone's projection is built
  from, and it routes its output back through the adjunct's own decoder — so a
  hop this surface may not carry is refused rather than shown, and the desktop
  and the phone cannot hold two opinions about one run. Writing a second chain
  reader would have passed every other check in this repository and still
  drifted.
- **Dispatch became a typed command.** The start request used to be a `json!`
  blob built inline in the render file: its fields were whatever that
  expression happened to contain, and the only proof it carried no evidence was
  that nobody had added any. `FullAutoDispatch` is now the record, its field set
  is asserted exactly, and it has no field for an `evidence` block, a
  `decisionRef`, or an `authorityReceiptRef`. omega#47 watched a live engine
  ignore all three, forged, in a real start request; this makes the same claim
  one layer earlier, where the forgery cannot be written at all.
- **Owner gate 8 is enforced by the argument type.** `FullAutoDispatch` has one
  constructor and it takes an `omega_front_door::LaunchOrigin`, every variant of
  which is a control a person operates. There is no `LaunchOrigin::ToolCall`, so
  a model-authored path cannot produce a dispatch, and
  `no_model_callable_crate_can_dispatch_full_auto` checks that no crate outside
  the Full Auto surface can even name the command.
- **The refusal that replaced a string test.** "No worktree" used to be decided
  by testing whether a formatted reference ended in `"missing"`, which refused a
  real project whose name ended that way and accepted an unsafe one. It is a
  typed `DispatchRefusal` now, and the honest case is covered by a test.
- **What this does not cover.** The thread's run link is read-only. Pause,
  resume, stop and retry stay on the Full Auto surface, where each control is
  bound to the run generation the host minted it for; a second set of buttons
  reading a projection would be a second place that believes it can command a
  run. The `NewThreadMenuItem` origin is not distinguished in practice, because
  that menu entry dispatches the same `OpenLauncher` action as the command
  palette, so both record `open_launcher_action`. Only threads the host bridge
  correlated to a run carry a run reference at all, so a run a person watches
  only on the Full Auto surface renders its chain there and not in a thread.
  And no check here reads a rendered pixel: the rows are asserted to be built
  and drawn, not to be legible.
- **Enforced by:** `a_full_auto_dispatch_carries_no_evidence`,
  `no_model_callable_crate_can_dispatch_full_auto`,
  `a_thread_renders_the_receipt_chain_of_its_linked_run`, and
  `the_thread_run_link_is_a_projection_and_not_a_second_authority` in
  `crates/omega_deltas`; plus the falsification suites in
  `crates/full_auto_ui` (`thread_run_link`, `dispatch`) and
  `crates/workroom_receipts` (`the_single_pair_entry_point_names_each_refusal`).
### OMEGA-DELTA-0031 — No user-facing sentence presents a competitor as the product

- **Upstream Zed:** names itself throughout its own copy, which is correct for
  Zed and is inherited wholesale by a fork.
- **Omega, before this:** the signed, notarized **`0.2.0-rc13`** still told the
  user, in its own voice:
  - `Click 'Connect' below to start using Ollama in Zed` — and the identical
    llama.cpp line. Provider onboarding, one click from the model picker.
  - `Checking for Zed Updates…` / `Downloading Zed Update…` /
    `Installing Zed Update…` in the **title bar**.
  - `<title>Authorization Successful — Zed</title>` and a `Zed` brand line on
    the **OAuth callback page rendered in the user's browser**.
  - `You are the Zed coding agent running inside the Zed editor.` — the
    **system prompt**, i.e. the identity handed to the model on every turn.
  - `# ====== Auto-added by Zed: =======`, written into the user's
    `.git/info/exclude`.
  - `Open with Zed` in the Windows Explorer context menu, `Error: Running Zed
    as root…`, `Zed managed Node.js`, `Request blocked by the Zed sandbox
    network policy.`, `Zed: v{version}` in the system specs a user pastes into
    a bug report, and ~forty settings-schema descriptions the settings editor
    renders as tooltips (`Settings related to calls in Zed`,
    `Configuration of voice calls in Zed.`, …).
  - Every brand check on omega#16 passed while all of that shipped.
- **Why the previous gate did not catch it.** OMEGA-DELTA-0022 closed assets,
  asset-name enums and command-palette labels with derived inventories, and
  **named this class as the one it could not close**: its string rule enforces
  the compatibility allow-list's `blocked` claims, which is a written-down
  denylist. A *new* sentence fails only once somebody adds it — which is
  exactly how `Use GitHub Copilot in Zed` survived — and 168 brand-bearing
  prose literals were left unclassified.
- **Omega now:** the default is inverted. Every brand-bearing prose literal in
  a **derived** inventory must be **classified** in
  `script/omega-brand-gate.json`; an unclassified one fails. A new sentence is
  unclassified the moment it is written, so it fails on the commit that adds it
  rather than on the release that ships it. Two hundred and seventy lines of
  copy across 94 files were rewritten to Omega, and 56 literals are recorded as
  deliberate references to Zed with a class and a reason (4 of them exist only
  in the package).
- **The rule, for product claim versus third-party reference.** Substitute our
  own name. If the sentence stays **true** with `Omega` in place of the brand,
  the brand was standing where our product's name belongs and it is a product
  claim — rewrite it. `start using Ollama in Zed` → `start using Ollama in
  Omega` is true, so it was a claim about us. If the substitution makes the
  sentence **false**, it states a fact about somebody else's product, service,
  documentation, or authorship — keep it. `similar to Zed's default
  keybindings` does not become true of Omega's keybindings, so the preset
  really is describing Zed. The five classes in the policy are the reasons a
  substitution would have been false: `zed_product`, `zed_service`,
  `zed_authorship`, `fork_seam`, `omega_contrast`.
- **How this inventory is complete rather than enumerated.** Five streams, each
  derived from a mechanism that exists in the tree:
  - **Every Rust string literal** under `crates/`, including raw and
    multi-line ones, outside `#[cfg(test)]` items and test files. A literal is
    compiled in; nothing can prove one is never rendered, and assuming
    otherwise is what left the provider copy standing through four release
    candidates. The lexer matters: a regex over single lines misses exactly the
    literals carrying the longest copy — the OAuth page, the run-as-root
    warning and four provider error toasts are all multi-line.
  - **Every settings-schema description**: doc comments in a file that really
    derives `JsonSchema`, which is how `schemars` turns them into the schema's
    `description` values and how the settings editor renders them as tooltips.
    Doc lines are stripped before the derive is looked for, so a derive written
    inside a rustdoc *example* does not drag a framework crate's internal
    documentation into the inventory.
  - **Every action description**: doc comments inside an `actions!(…)` body or
    above an `#[action(…)]` derive — the text the keymap editor shows.
    OMEGA-DELTA-0022 recorded these as unchecked.
  - **Every `--help` line**: doc comments in a file deriving
    `Parser`/`Args`/`Subcommand` or using `#[command(…)]`.
  - **Every shipped asset line**, over the embedded-asset inventory
    OMEGA-DELTA-0022 already derives — keymap and default-settings comments,
    the agent prompt templates, themes.
  A literal enters the inventory when it carries a `brand.words` /
  `brand.substrings` hit **and** is prose-shaped: three tokens or more, at
  least two plain alphabetic words. That is the one judgement in the
  derivation, and it is deliberately loose — `Zed Plex Sans` is three words and
  is *in* the inventory, classified, rather than quietly filtered away.
- **Anti-vacuity.** The floors are on what the scanners **read**, not on what
  they find, because a clean tree finds almost nothing and a broken parser
  finds nothing, and those two must not look alike: 1500 Rust sources, 100 000
  string literals, 6000 schema doc lines, 1200 action doc lines, 300 `--help`
  doc lines, 400 embedded files. Separately, every classified entry is asserted
  to still be **present**, so the registry cannot become a graveyard and a
  scanner that stops reading a stream fails on that stream's entries.
- **The two halves are checked against each other.** `crates/omega_deltas` and
  `script/verify-omega-brand` implement the inventory independently in Rust and
  Python from the one policy file. Dumped and diffed on this tree they produce
  **byte-identical** inventories — 90 literals, same kind, file, line and text —
  which is the drift guard a shared policy file only half provides. The two
  lexers disagree on six string literals out of 135 000 on an edge case in char
  literals; none carries a brand, and the counts are only used as floors.
- **The packaged half reads values, not source.** Assets and literals survive
  in the executable, so the shipped binary is scanned directly: it does not
  honour `#[cfg(test)]`, does not care which crate a string came from, and sees
  generated files — the licence attribution page is gitignored and exists only
  in the package, which is where `Copyright 2022 - 2024 Zed Industries, Inc.`
  comes from. A stripped string table has no separators, so the scan is
  anchored on each brand occurrence and reports one only when the brand is
  written as a word in running text **and** no classified sentence spans that
  position. Classified entries are compared as the compiler leaves them: a `\`
  continuation joins two lines with no separator and a `\n` splits the run, so
  the fragments are matched, not the source spelling.
- **Two live defects fell out of the inventory rather than out of a search.**
  `f10` on Linux and Windows was bound to
  `["app_menu::OpenApplicationMenu", "Zed"]`, naming a menu that has been
  `PRODUCT_NAME` — "Omega" — since the rename: the key did nothing. And
  renaming the `.git/info/exclude` marker without reading the old one back
  would have stranded an inherited-marker block in the user's repository, still
  excluding files from git with nothing left that knew how to remove it, so
  `GitExcludeOverride` now cleans up both.
- **References deliberately kept**, each with a class and a reason in the
  policy: Zed's hosted service, account, plans, billing and servers
  (`Zed Pro Plan`, `Business Plan - Zed models enabled`, `Signs in to Zed
  account.`, `Zed's Edit Predictions`, the whole Zed Cloud provider); Zed's own
  documentation (`zed.dev/docs/...` in remote-development, keymap,
  Linux and Windows troubleshooting copy); Zed's authorship of the inherited
  One/Ayu/Gruvbox themes and the Windows Performance Recorder profile; the
  `Zed Plex Sans` / `Zed Plex Mono` family names recorded inside the shipped
  font files; the retired `.git/info/exclude` marker, read back for cleanup;
  and Omega-authored copy that names Zed in order to say how Omega differs from
  it. `Enable Fast Mode for Zed?` was the one genuinely ambiguous case — it
  read as the application — and became `Enable Fast Mode for Zed AI?`, naming
  the service the toggle and its billing actually belong to. We are removing
  Zed **as our identity**, not erasing that Zed exists.
- **Enforced by:** `no_unclassified_prose_names_a_competitor` and
  `the_prose_lexer_reads_multi_line_and_raw_literals` in
  `crates/omega_deltas/`, and `check_prose_inventory` /
  `check_packaged_prose` in `script/verify-omega-brand`, which
  `script/bundle-omega-rc` already runs against the built bundle. The packaged
  half rejects the installed `0.2.0-rc13` on **200** distinct windows, every
  one of which is prose this delta rewrote.
- **Falsified.** A new sentence naming Zed as the product was added to a
  provider page and to `assets/settings/default.json`; both halves failed as
  unclassified and both recovered when it was removed. `Click 'Connect' below
  to start using Ollama in Zed` was restored verbatim and both halves failed
  again. A classified entry was deleted from the policy and the surviving
  literal failed; the entry was pointed at a sentence that is not in the tree
  and the staleness assertion failed. Each floor was raised above the observed
  count in turn and the corresponding guard fired. And the packaged half was
  run against the signed, notarized `0.2.0-rc13` in `/Applications`, which it
  rejects, and against a stub bundle whose binary holds no strings, where its
  own vacuity guard fired. Rebasing onto seven newly landed
  `assets/settings/default.json` comments naming Zed failed the gate on all
  seven until each was classified — the mechanism working on somebody else's
  concurrent change rather than on a planted one.
- **What this still does not cover.** The scan reads `crates/` and the embedded
  assets; `docs/`, `.github/`, `script/` and `crates/zed/resources/` are
  outside it, so a Zed sentence in the docs site or a workflow file passes.
  Prose-shape is a heuristic: a one- or two-word label like `name = "zed"` in
  the CLI's clap `#[command]` is not prose and is not seen — that identifier
  still spells the old name and is a known residual. Nothing reads a rendered
  pixel, so a sentence that is correct in source and truncated on screen still
  passes. The packaged half runs against the macOS bundle only, so Linux
  `.desktop`, Flatpak, Snap and Windows resources are unchecked. `#[cfg(test)]`
  exclusion is a source-side convenience whose only real backstop is the
  packaged scan. And no name is forbidden unless `brand.words` or
  `brand.substrings` says so: this delta is about *how completely* a forbidden
  name is looked for, not about which names are forbidden.

### OMEGA-DELTA-0032 — A send during a running turn has one declared answer per executor

- **Upstream behaviour.** `ThreadView::send` checks whether a turn is running
  and, if so, queues the message. `MessageQueue` already tells a steer from an
  enqueue, and `sync_queue_flag_to_native_thread` sets
  `end_turn_at_next_boundary` on `agent::Thread` when the front entry wants to
  steer. That function no-ops for anything that is not a native thread, and
  `dispatch_queued_entry` then cancels the running turn unconditionally before
  sending.
- **Why Omega diverges.** Those two facts together mean one button does three
  different things. On the native loop a steer ends the turn at a message
  boundary. On an external ACP peer and on an engine lane the same gesture
  cancels the running turn and starts a new one — a behaviour nobody declared,
  nobody negotiated, and the user cannot distinguish from steering. Omega runs
  three executor classes on purpose (`OMEGA-DELTA-0029`), so "two work and one
  silently drops" is not a rough edge here, it is the concurrency hole omega#79
  names at design time.
- **The law.** `omega_front_door::disposition` is a total const function from
  (command, executor class, declared steer capability) to a declared outcome.
  There is no fallthrough and no variant meaning "whatever the executor does".
  A steer the class cannot honour is a typed refusal that **carries its
  fallback**, so a refusal cannot be constructed without saying what happened to
  the message.
  - The native loop steers at its message boundary. Omega owns both sides of
    that loop, so it does not negotiate.
  - An external ACP peer is asked. The Agent Client Protocol has no capability
    for mid-turn delivery, so every peer today answers "unknown" — and unknown
    is refused, not assumed. Silence is not a capability, and the cost of
    guessing wrong is the user's running turn.
  - An engine lane refuses whatever the engine can do. An engine lane *is* Full
    Auto authority, and `OMEGA-DELTA-0030` keeps a run's controls bound to the
    generation the run surface minted them for. A composer that could interrupt
    a run would be a second surface that believes it can command one.
- **Durable admission.** The queue is written down before the composer
  acknowledges it. `openagents.omega.agent_send_queue.v1` is keyed by thread and
  item, ordered by an admission sequence rather than by map order, rewritten
  atomically, and refuses a document it did not write. A terminal item is never
  reopened: `promoted`, `cancelled` and `failed` are final, because a restart
  that could move an item back to `queued` would promote it twice. Promotion
  needs *proven* quiescence — after a reconnect Omega never saw the prior turn
  stop, and promoting there is how a queued message races the turn it was meant
  to follow.
- **The disposition is derived, never stored.** The same rule
  `ExecutorDisclosure` holds to. A stored disposition could disagree with the
  law that produced it, and then the record would be the lie. The journal holds
  the parts; the phrase is a function of them.
- **What this does not cover.** The queue's live half still lives on the view,
  so the durable record is authority for *what was admitted* and not yet for the
  editor state a restart would need to rebuild the composer rows. Nothing here
  reads a rendered pixel. And an external peer's capability is a single call
  site returning "unknown" — when ACP gains a mid-turn capability, that call
  site is what changes, and the law above already has the variant for it.
- **Enforced by:** `the_send_during_turn_law_answers_for_every_executor_class`,
  `the_queue_law_and_its_journal_read_nothing_but_their_inputs`,
  `the_composer_decides_a_mid_turn_send_through_the_law`, and
  `the_send_queue_is_a_durable_record_and_not_renderer_memory` in
  `crates/omega_deltas`; plus the suites in
  `crates/omega_front_door/src/send_during_turn.rs` and
  `crates/agent_ui/src/omega_send_queue.rs`.

### OMEGA-DELTA-0033 — A pin is a control a person can press, and a refusal is a sentence they can read

**Upstream:** an external agent's row in Settings shows a name, a source icon
and a Remove button. There is no version pinning, no provenance, and nothing to
say what bytes a wrapped harness will run with the thread's tool permissions.

**Omega before this delta:** `OMEGA-DELTA-0025` (omega#81) built the whole
decision layer and rendered none of it. `MaintenanceAffordance::Disabled`
structurally could not exist without a sentence, and nothing put that sentence
on a screen. A refusal reached the owner only as agent-launch error text. There
was **no writer for the pin ledger in production code at all** —
`HarnessPinLedger::set_pin` and `remove_pin` were called only by tests, so a
"pin" was a JSON file the owner had to hand-edit. Two of the four maintenance
actions omega#81 named did not exist. And `LocalRegistryNpxAgent` consulted
nothing: pinning an npx harness did **nothing whatsoever**, which matters more
than it sounds, because the harness omega#81's acceptance sentence names —
`codex-acp` — is npx-distributed in the live ACP registry.

**Omega now:**

- **One control, never two.** `PinControl` offers exactly one of `Take`,
  `Remove`, or `Unavailable { reason }` per harness. Re-pointing a freeze at
  whatever is installed now is deliberately two actions, because one click that
  silently moved a pin would undo the freeze in the exact case it exists for.
  `Unavailable` carries its reason by construction, for the same reason
  `Disabled` does: omega 0.2.0-rc11 shipped a refusal nobody could see.
- **The row cannot disagree with the gate.** `harness_front_door_state` routes
  every answer through the same `decide_maintenance` / `admits_version` /
  `admits_package_manager_launch` the launch path enforces, and
  `the_rendered_launch_state_equals_what_the_gate_would_decide` asserts equality
  across the whole space of pin states and measurements. The settings page calls
  none of those functions itself — it matches on the result and writes no
  sentence of its own.
- **A pin is taken at bytes, through the gate.** `pin_installed_harness` obtains
  its digest from `authorize_installed_harness`, so taking a pin runs the real
  gate, writes a real receipt, and cannot freeze a tree the gate would refuse.
  An unreadable ledger refuses both controls rather than being rewritten from
  the subset this build could parse — that is not removal, it is deletion of
  everything unreadable.
- **The npx path consults the pin.** There is no tree to hash, so the measured
  gate cannot run; `admits_package_manager_launch` narrows the question to the
  one the ledger can answer. A pinned harness refuses with a sentence naming the
  resolver, and the refusal is recorded. **The honest limit, stated rather than
  hidden:** this raises no bar on an *unpinned* npx harness, which still launches
  unattested — the front door says so on the row.
- **Resolving the channel and re-probing are their own actions.** Channel
  resolution happens when nothing is about to launch, so a frozen harness is no
  longer *offered* a version the next launch would refuse — and the resolution
  that decided so is recorded, because an update that never starts leaves no
  other trace. `ReprobeCapability` is what the owner's control does with nothing
  about to run, kept distinct from `Verify` so the log can say whether a
  measurement was taken because something was about to execute or because a
  person asked.
- **Proven live.**
  `live_a_real_registry_install_produces_a_receipt_and_a_pin_blocks_the_next_version`
  fetches the live ACP registry, downloads a real release through the same
  downloader the launch path uses, gates the extracted tree, reads the receipt
  off a real disk, pins it, watches the pin block a later version, and then adds
  one file to the installed tree and watches the harness stop running. It is
  `#[ignore]`d: a suite that silently depends on a third party's release assets
  goes red for reasons that are not about this repository.
- **What this does not do.** Neither the ledger nor the receipt log is signed;
  anyone who can write to `paths::external_agents_dir()` can rewrite both. The
  digest says the bytes did not change since Omega measured them, not that they
  are the bytes the publisher built. And an owner-named custom binary has no
  maintenance state at all, by design — Omega did not choose it and does not
  update it.
- **Enforced by:** `the_front_door_page_renders_decisions_it_did_not_make`,
  `a_withheld_control_carries_a_sentence_all_the_way_to_the_widget`,
  `the_pin_ledger_has_a_writer_the_owner_can_reach`,
  `the_package_manager_launch_path_is_gated_on_the_pin`,
  `resolving_a_channel_is_a_recorded_action_that_gates_the_offer`, and
  `the_front_door_measures_the_tree_the_launch_path_gates` in
  `crates/omega_deltas`; plus the suites in
  `crates/omega_harness/src/front_door.rs` and
  `crates/project/tests/integration/harness_maintenance.rs`.

### OMEGA-DELTA-0034 — The front door works with no project open

- **Upstream Zed:** the agent panel requires an open project. `agent_ui: Require
  an open project for agent panel` (#56577, "a bit brute force, but it works")
  put a `has_open_project` early return in front of every entry to the panel, so
  a window with no worktree shows "Open a Project" and nothing else. That is
  coherent for an editor whose agent is an accessory to a project.
- **Omega, before this:** `OMEGA-DELTA-0019` made a window with nothing to
  restore open on the agent instead of on an untitled buffer, and
  `OMEGA-DELTA-0020` folded Full Auto into that panel. Both landed. The exit
  omega#76 actually asked for — *fresh launch lands on the surface **and typing
  starts a thread*** — did not: a window with nothing to restore is by
  definition a window with no project, so the front door opened onto
  "Open Project / Clone Repository" and there was no composer to type into. The
  landing half shipped and the typing half did not, and the two were reported
  together.
- **Omega now:** the guards on the front door's own path are gone.
  `activate_new_thread`, `activate_draft`, `new_thread`,
  `ensure_native_agent_connection` and `toggle_new_thread_menu` no longer refuse
  a projectless window, the toolbar's create controls are live, and a thread's
  title is editable. Typing on a fresh install starts a real thread.
- **What the guards protected, checked rather than assumed.** The one that could
  plausibly have been load-bearing was `ensure_native_agent_connection`, and it
  is not: `NativeAgentServer::connect` takes the project as `_project` and never
  reads it, and `NativeAgent::new_session` builds a thread from the project
  entity without requiring a visible worktree. What the guard bought was not
  spinning up a connection for a window upstream had decided would never show
  the agent — a resource choice resting on a premise Omega does not share.
- **The workspace-touching guards stay.** A terminal genuinely needs a working
  directory, so `should_create_terminal_for_new_entry` keeps its check and a
  projectless `new_thread` falls through to a thread rather than a terminal.
  Loading a thread from the clipboard, resuming a persisted draft, refreshing
  skills, initialising from a source workspace, and starting an external ACP
  agent all still require a project. Removing *those* would not be
  project-optional threads; it would be threads that fail later and less
  legibly.
- **`cmd-?` gave the agent panel back to macOS.** `agent::ToggleFocus` was bound
  to `cmd-?`, which is macOS's reserved Help chord — the Help menu's search
  field. Omega cannot win that keystroke, so the binding was a keybinding that
  looked present and did not work. It is `ctrl-cmd-a` now: free, not
  system-reserved, and on the same letter as `cmd-shift-a`. Linux (`ctrl-?`) and
  Windows (`ctrl-shift-/`) are untouched, because neither is reserved there and
  changing them would be churn with no defect behind it.
- **The rendered proof, and why the runner needed standing up.**
  `zed_visual_test_runner` was already in the fork — macOS headless Metal
  capture, no Screen Recording permission, no window to get frontmost — and it
  had no committed baselines and no invocation, so no Omega packet had ever
  used it. `script/omega-visual-proof` is the invocation, `OMEGA_VISUAL_ONLY=1`
  runs Omega's suite alone, and the `omega_*` baselines are re-included in
  `.gitignore` and committed. A baseline nobody committed cannot fail. The
  suite asserts the rendered line's text through the same method the render
  calls and asserts the record is coherent *before* each capture, so a picture
  can never be the only thing standing behind a claim.
- **Enforced by:** `the_front_door_does_not_require_an_open_project` and
  `required_keymap_bindings_resolve` in `crates/omega_deltas`;
  `test_empty_workspace_opens_the_front_door` in `crates/agent_ui`; the rendered
  proofs `omega_front_door_no_project` and `omega_front_door_typing` in
  `crates/zed/src/visual_test_runner.rs`.
- **What this does not cover.** A **first-ever** launch lands on the Onboarding
  tab, not on the agent — observed on a packaged build with a fresh
  `--user-data-dir`, where the front door is behind onboarding and the agent
  dock is closed until it is finished. Every launch after that lands on the
  agent. That ordering was left open here as a question for the owner; it is
  now decided and recorded as `OMEGA-DELTA-0040`, which tests the handoff from
  onboarding to this front door. A projectless thread is also not yet *bound* to a
  project on its first workspace-touching action, which is the rest of omega#76's
  project-optional deliverable. Today it simply has no worktree: file mentions
  find nothing and a tool that needs a path fails the way it would in any
  worktree-less window. The `cmd-shift-a` bindings named in omega#76 are
  resolved rather than changed, and the resolution is now checked by
  `the_new_thread_chord_is_window_global` under `OMEGA-DELTA-0013`: on macOS
  there is no editor-context collision to fix — upstream's
  `editor::SelectToBeginningOfLine` is on `ctrl-shift-a`, a different chord —
  and the only narrower `cmd-shift-a` bindings are inside modal pickers
  (`ToolchainSelector`, `RecentProjects`) that fire while the user has opened
  them on purpose. On Linux and Windows the chord is `ctrl-shift-a` and a
  terminal's select-all shadows it there, which is admitted for the same
  reason and enumerated rather than left to be discovered.

### OMEGA-DELTA-0035 — The router is wired, and a pin is a gesture

- **Upstream Zed:** there is one agent connection per thread and no routing, so
  there is nothing to wire and nothing to pin.
- **Omega, before this:** `OMEGA-DELTA-0029` built the routing law and the
  dispatch seam and wired neither. Nothing in the shipped app constructed an
  `OmegaAgentConnection`, `observe_capacity` had no caller, and no surface could
  set a pin — so the route journal stayed empty and every thread disclosed
  `route: None`. The exit properties held in the router and not in the product.
- **Omega now:** `Agent::NativeAgent.server(..)` returns an
  `omega_router::OmegaRouterServer`, whose `connect` builds an
  `OmegaAgentConnection` over the native connection and publishes it. Every new
  first-party session is routed on purpose and its decision is written to the
  route journal before the turn exists. The agent panel polls the engine's
  framed `get_capacity` answer on the same three-second cadence the Full Auto
  roster uses and hands it to `observe_capacity`, so an engine-lane pin is
  decided against what `omega-effectd` is actually doing rather than against a
  default of "not running".
- **A pin is a gesture, enforced by the argument type.** `pin_session`,
  `unpin_session` and `pin_next_session` each require an
  `omega_front_door::PinGesture`, every variant of which is a control a person
  operates. There is no `PinGesture::ToolCall`, no `SlashCommand`, no
  `RestoredDraft` and no `ComposerMode`, and `pin_gestures_are_all_human_gestures`
  fails if one appears. This is the same construction owner gate 8 already uses
  for `LaunchOrigin`, applied at the pin because a pin is the only door to an
  engine lane and an engine lane *is* Full Auto authority. omega#76 rejected a
  composer mode flag for Full Auto because a boolean the send path reads can be
  set by a slash command, a restored draft, or a model-authored insertion; a pin
  stored as a mode would be the same construct wearing a different name.
- **Engine lanes are still reachable only through a pin.** The unpinned default
  is the native loop, always. Nothing here auto-prefers a ready lane, and
  `an_unpinned_thread_never_reaches_an_engine_lane` in
  `crates/omega_front_door/src/router.rs` fails if the law changes.
- **A human pin re-decides; capacity moving does not.** Setting or clearing a
  pin re-runs the decision for that session and records it, so an unhonourable
  pin appears on the thread's own line as a fallback with its typed reason
  instead of silently doing nothing. A later engine answer never re-decides a
  recorded session, so a turn does not move executors mid-thread because
  capacity changed.
- **The thread carries the executor, not the router.**
  `OmegaAgentConnection::new_session` delegates and returns what the executor
  built, so `OMEGA-DELTA-0021`'s disclosure keeps classifying the executor. The
  two places that asked "is this the native agent?" by downcasting — the
  shared-project refusal and the native thread-store hand-off — go through
  `is_native_agent_server` now, because a wrapped native agent reading as
  external is a silently wrong `false` rather than a compile error.
- **Enforced by:** `the_router_is_wired_into_the_native_agent_entry`,
  `only_a_named_human_gesture_can_pin_an_executor` and
  `nothing_asks_for_the_native_agent_with_a_bare_downcast` in
  `crates/omega_deltas`; `pin_gestures_are_all_human_gestures` in
  `crates/omega_front_door`; the dispatch and journal suite in
  `crates/agent_ui/src/omega_router.rs`; and the rendered proofs
  `omega_executor_disclosure_*` and `omega_route_pin_not_honoured` in
  `crates/zed/src/visual_test_runner.rs`.
- **What this does not cover.** No external ACP agent or engine-lane executor is
  registered on the router in this build, so a pin to either fails closed to the
  native loop with `external_acp_unavailable` or `engine_lane_not_connected` —
  which is the honest answer and is what the rendered proof shows. The honoured
  pin the rendered proof shows is therefore a pin to the native loop, which is
  a real honoured pin (`routed: pinned`, not `routed: unpinned`) but not an
  honoured *engine-lane* pin; that path is exercised only in tests. Turns still reach
  the executor through the thread's own connection rather than through
  `OmegaAgentConnection::prompt`, because the thread carries the executor; the
  router decides per session, not per turn. And the pin menu is rendered on the
  thread's disclosure line, so a surface with no thread on it has no pin
  control.
### OMEGA-DELTA-0036 — `--uninstall` removes Omega, and no part of anybody else

- **Upstream Zed:** ships `script/uninstall.sh` to remove a Zed installed by
  `install.sh`. Correct for Zed, and inherited wholesale by a fork.
- **Omega, before this:** `crates/cli/src/main.rs` embedded that file with
  `include_bytes!` and ran it, advertised in `--help` as **"Uninstall Omega from
  user system"**. It contained zero occurrences of `Omega` or `omega`. With
  `ZED_CHANNEL` unset it removed `/Applications/Zed.app`, the whole
  `~/Library/Application Support/Zed` tree, `~/Library/Logs/Zed`, the caches,
  HTTP storage, preferences plist and saved application state under
  `dev.zed.Zed`, and `~/.zed_server`; it asked whether to keep *"your Zed
  preferences"*; and it printed **`Zed has been uninstalled`**. It removed no
  Omega path at all — not the bundle, not `~/Library/Application Support/Omega
  RC`, not `~/Library/Logs/omega-rc`. A user who ran the advertised "Uninstall
  Omega" kept Omega and lost their other editor. This shipped in the signed,
  notarized `0.2.0-rc13` and `0.2.0-rc14` (omega#88).
- **Why nothing caught it.** The paths lived in a hand-written table in a shell
  script, disconnected from the code that creates those directories, so nothing
  in the tree could observe that the two had never agreed. And
  `script/verify-omega-brand` opened `Contents/MacOS/omega` at three places and
  `Contents/MacOS/cli` at none, although `script/bundle-omega-rc` copies and
  signs `cli` into the bundle — the file the defect was in had never been opened
  by any check. That half is `OMEGA-DELTA-0038`.
- **Omega now:** there is no path table. `crates/cli/src/uninstall.rs` builds an
  `UninstallRoots` — one field per place Omega writes — where every field is
  read from the `paths::` function that writes it: `data_dir`, `config_dir`,
  `logs_dir`, `temp_dir`, `state_dir`, the CLI symlink under
  `paths::BINARY_NAME`, and the bundle-identifier paths from
  `release_channel::RELEASE_CHANNEL.app_id()`. The app bundle comes from the
  bundle this CLI was launched out of, not from a literal. `plan` destructures
  the struct **exhaustively**, so a root added to it and left out of the plan
  does not compile. The script receives the plan in `OMEGA_UNINSTALL_PATHS`, one
  absolute path per line, and removes exactly those; unset or empty exits
  non-zero rather than falling back to a default, because every default this
  file has ever had belonged to somebody else's product. `/`, `/Applications`,
  `$HOME` and a relative path are refused before anything is removed.
- **Settings are asked for, never taken.** `config_dir` is held out of the
  automatic list and prompted for by name, because it is the user's
  `settings.json` and keymap and the one root somebody may want to keep.
- **Enforced by:** `the_uninstall_path_removes_omega_and_names_no_competitor` in
  `crates/omega_deltas/` (the script names no other product; it still reads its
  plan from the caller; `from_installed_paths` makes at least one `paths::` call
  per root; `plan` still destructures exhaustively; the script refuses an empty
  plan), and by three tests in `crates/cli/src/uninstall.rs` — including
  `the_script_removes_omega_and_leaves_the_other_editor_untouched`, which runs
  the shipped script against a fabricated home holding **both** an Omega
  installation and another product's, and reads both halves back afterwards.
- **Falsified.** Adding a competitor's directory to the script failed the name
  assertion. Removing `paths::logs_dir()` from the plan failed both the plan
  test and the end-to-end test, which reads the roots rather than the plan
  precisely so a forgotten root cannot pass. Adding a hard-coded removal outside
  the plan failed the "left untouched" assertion byte-for-byte. Each
  falsification was probed against a pristine copy of the file before its test
  ran, so an edit that did not apply is a hard error rather than a green run.
- **Paid for once, in the worst way.** The first falsification of this delta
  restored the `0.2.0-rc14` script over the rewritten one and ran the suite. The
  refusal test did not override `HOME` at the time, the restored script ignores
  the plan entirely, and it destroyed the real machine's other editor and its
  application-support tree — the exact damage this entry describes. Every test
  in that module now runs with a `HOME` of its own, whatever the script under it
  happens to be that minute.

### OMEGA-DELTA-0037 — Omega identifies itself to third parties as Omega

- **Upstream Zed:** sends `HTTP-Referer: https://zed.dev` and
  `X-Title: Zed Editor` on every OpenRouter request. Correct for Zed.
- **Omega, before this:** sent the same two headers, unchanged, on both the
  streaming and the non-streaming call. OpenRouter **displays the `X-Title`
  value to the account holder in their own dashboard**, so this is not a wire
  contract that happens to carry a name — it is Omega telling a third party, and
  the user, that it is a different product. Every request through `0.2.0-rc14`
  did it, and `strings` on the shipped binary found exactly one occurrence,
  classified nowhere (omega#89).
- **Omega now:** both call sites send `app_identity::PRODUCT_NAME` and
  `app_identity::PRODUCT_REPOSITORY_URL`. The values come from the identity
  constants rather than from literals, so a rebase cannot restore the old value
  on one path and leave the other correct.
- **Enforced by:** `outbound_attribution_names_omega` in `crates/omega_deltas/`,
  which asserts both request paths set the header, that neither literal is back,
  and that the count of `PRODUCT_NAME` uses equals the count of `X-Title`
  headers. The pair is also recorded as `blocked` in
  `crates/app_identity/fixtures/compatibility_allowlist.json`.
- **Falsified.** Restoring `"Zed Editor"` on one of the two call sites failed
  the test; restoring it on both failed it; the packaged scan reports it out of
  the built binary independently.

### OMEGA-DELTA-0038 — The packaged gate opens every executable that ships, and reads help as clap renders it

- **Upstream Zed:** has no equivalent; this is entirely about Omega's own gate.
- **Omega, before this:** `script/verify-omega-brand --app` opened
  `Contents/MacOS/omega` and nothing else. `script/bundle-omega-rc` copies and
  signs **three** binaries into `Contents/MacOS` — `omega`, `cli` and
  `omega-identity-proof`. Two of the three had never been opened by any check,
  which is why a destructive uninstaller (`OMEGA-DELTA-0036`) shipped in two
  published prereleases with the gate reporting green about the bundle.
  Separately, every prose stream in `OMEGA-DELTA-0031` reads **source**: a doc
  line, a literal, a schema comment. clap does not print source. It joins
  several doc lines into one sentence, resolves `cfg_attr` for the platform it
  was built for, prints the flag *name* beside the description, and lays the
  whole thing out at run time. Nothing had ever read that output, so
  `--zed <ZED>`, `Run zed in the foreground`, `Run zed in dev-server mode`,
  `Instructs zed to run as a dev server` and a `--user-data-dir` line naming a
  different product's data directory as Omega's own all shipped under a green
  gate (omega#89).
- **Omega now, three derived inventories where there were three lists:**
  - **Every executable in the bundle.** `bundle_executables` walks the app and
    keeps every file whose first four bytes are Mach-O magic. The icon-name
    rule, the embedded-asset rule, the prose scan and the first-party-agent scan
    all run over that set; only the two *presence* checks — the reviewed artwork
    and the required action labels — stay on the main binary, because only it
    embeds them. A helper added to the bundle tomorrow is inside the gate the
    day it is added.
  - **Rendered output.** Every executable is run with `--help` and `--version`,
    subcommands are enumerated **from the help text itself** rather than listed,
    and each is run in turn. Every line of the result is read as prose. The
    floor is on invocations *read*, not on findings.
  - **`first_party_agent.phrases`, applied to something.** `grep -c
    first_party_agent script/verify-omega-brand` returned **0**: the key existed,
    a Rust test read it against the source tree, and no gate had ever applied it
    to a package. A reviewer had to run it by hand every time, which is the same
    as not having it. It now runs over every executable, with the identity
    string itself as the anti-vacuity guard.
- **A classified sentence is matched as the compiler leaves it.** Splitting
  already handled `\` continuations, `\n` and non-ASCII; a `{}` format
  placeholder is not in the binary either, because rustc splits a format string
  at every placeholder. Classified entries are now split there too — conservative
  in the same direction, since shorter fragments cover less, never more.
- **Enforced by:**
  `the_packaged_gate_opens_every_shipped_executable_and_reads_rendered_help` in
  `crates/omega_deltas/`, which derives the shipped binary set by parsing what
  `script/bundle-omega-rc` writes into `Contents/MacOS`, asserts the gate's floor
  covers it, asserts the gate names a binary path in exactly one place, and
  asserts each new check is both defined and called.
- **Falsified against a real candidate.** The rewritten gate was run against the
  installed, signed, notarized `0.2.0-rc15` in `/Applications`, which the
  previous gate passed. It rejects it on 42 findings, including the uninstall
  script's text inside `Contents/MacOS/cli` — a file the old gate never opened —
  and on `cli --help` and `omega --help` printing three of the sentences above.
  The rebuilt `cli` from this tree passes the same scan. Lowering
  `minimum_executables` below what the bundler ships fails the delta test;
  deleting either new check's call fails it.

### OMEGA-DELTA-0039 — The installed-proof harness observes what it records

- **Upstream Zed:** has no equivalent; this is Omega's own installed-candidate
  harness.
- **Omega, before this:** three checks in it could not fail, and a check that
  cannot fail is worse than no check, because it produces a clean evidence table
  (omega#90).
  - **The secret tripwire.** `deliver_needle_through_protected_fd` made a pipe,
    wrote a fresh `secrets.token_hex(32)` into it, and closed **both ends in the
    same function**. No process was spawned; `--app` appeared only in the
    docstring. The needle had never been seen by anything but the script, which
    then searched the disk for it. `status: "pass"` was guaranteed by
    construction, whatever the product did.
  - **Four of six surfaces watched directories Omega never writes.** `logs`,
    `telemetry`, `crashes` and `clipboard` resolved under
    `~/Library/Application Support/Omega RC/…`, while `paths::logs_dir()` is
    `~/Library/Logs/omega-rc` and `paths::crashes_dir()` is
    `~/Library/Logs/DiagnosticReports`. The live 191 KB log was never opened.
    They recorded `absent`, and `absent` did not fail the receipt.
  - **`light-theme` and `dark-theme` wrote `content_legible: True` as a Python
    literal**, with zero `ocr_lines()` calls and zero `differing_pixels()` calls,
    never comparing the light capture against the dark one. A frozen, blank or
    appearance-ignoring window passed both. This is the same shape as the
    `defaults read` defect `5ce7f9855f` already corrected for `high-contrast`
    and `reduced-motion`: a fact about the *host* filed as a fact about the
    *product*.
- **Omega now:**
  - The needle is **read from the caller** through `--needle-fd` — the canary the
    candidate was actually given — and a run without one refuses rather than
    inventing a secret nothing has seen. Before any surface is trusted, the same
    scanner is pointed at a needle planted in a private `0600` file; no match
    ends the run with no receipt.
  - Surfaces resolve from `crates/paths/src/paths.rs`: `logs_dir()`, the
    telemetry log beside it, `database_dir()` plus `hang_traces_dir()`, and
    `crashes_dir()`. Clipboard and accessibility are read through their own
    interfaces — NSPasteboard (corroborated by `clipboard info`) and the
    `AXUIElement` tree of the running candidate — because neither was ever a file
    in a data directory. A surface that **cannot** be observed records `blocked`
    and the validator refuses the receipt, so "nothing was found there" and
    "nobody looked" no longer read the same.
  - The host appearance is only the **precondition**. Legibility is OCR'd off
    each capture against a stated line count and confidence, and the light and
    dark captures must differ by a stated pixel threshold. Every failure routes
    to `blocked`.
  - `script/bundle-omega-rc` derived `"dirty": False` from nothing; it now reads
    `git status --porcelain`, so a provenance field that reads like an
    observation is one.
- **Enforced by:** `the_installed_proof_harness_observes_what_it_records` in
  `crates/omega_deltas/`, plus each collector's own `--self-test`.
- **Falsified.** Breaking the needle comparison ended the run at exit 2 with no
  receipt, where the old code would have written `"status": "pass"`. Repointing
  the surfaces back at the data root blocked the receipt and the untouched
  validator refused it; planting the needle in the real `~/Library/Logs/omega-rc`
  produced `status: fail` over 193,984 bytes, which the old table could not have
  seen. Four separate plants against the appearance block — an unconditional
  pass, a dropped pixel threshold, a removed pixel diff and fabricated OCR lines
  — each failed the self-test. Every falsification was probed against a pristine
  copy before its test ran.

### OMEGA-DELTA-0040 — A first-ever launch lands on identity onboarding, and finishing it opens the front door

- **Upstream Zed:** first-run onboarding is a page you may open, skip, or close;
  nothing in the startup path waits for it, and the window opens regardless.
- **Omega, before this:** the ordering existed and nothing named it. Identity
  onboarding gates startup (`await_identity_ready`), `OMEGA-DELTA-0019` and
  `OMEGA-DELTA-0034` open the front door on a window with nothing to restore,
  and no test connected the two. A packaged first-ever launch therefore lands on
  Onboarding rather than the agent, which reads as `OMEGA-DELTA-0034` failing
  its own exit unless somebody decides on purpose that it does not.
- **The owner's decision, recorded rather than assumed.** Omega is
  identity-first by design: omega#9's packet exists to put identity before
  editor setup, and an agent thread before an identity would invert it. "Fresh
  launch lands on the surface" is satisfied by the front door being the surface
  a *usable* session opens on. A genuinely first-ever launch legitimately lands
  on onboarding first, and every launch after it lands on the agent. The owner
  may veto this; it is written down so a veto has something to point at.
- **The decision is only sound while the handoff is real,** so the handoff is
  the thing that is tested. "Onboarding first" and "onboarding instead" are the
  same picture on a first launch and completely different products on the
  second. Three links, each of which fails silently on its own:
  `restore_or_create_workspace` **waits** on `await_identity_ready` before it
  opens anything; the first-run branch of `on_finish` **closes its own window
  and releases** that wait; and `release_identity_waiters` **completes the
  channel** the startup path is parked on. Remove the middle link and setup
  completes into a launchpad that never becomes an agent — a failure no check on
  the front door alone can see.
- **Enforced by:** `first_run_onboarding_hands_the_startup_off_to_the_front_door`
  in `crates/omega_deltas`, alongside `a_fresh_window_opens_on_the_agent`
  (`OMEGA-DELTA-0019`) and `the_front_door_does_not_require_an_open_project`
  (`OMEGA-DELTA-0034`).
- **What this does not cover.** **Abandoning** onboarding is a dead end in that
  launch. The first-run onboarding page is an ordinary closeable item; closing
  it calls `Item::on_removed` → `onboarding_closed`, which clears the
  "onboarding is open" flag and **does not** complete the startup channel. So a
  user who closes the tab instead of finishing setup keeps the launchpad window
  the onboarding page was hosted in, with the agent dock closed and no window
  ever created behind it. Identity is still not ready, so relaunching shows
  onboarding again and nothing is lost but the session — and refusing to
  continue without an identity is the identity-first posture working, not
  failing. Making the page unclosable, or making a close mean "quit", is an
  onboarding-surface decision (omega#9) and not this delta's to take. This delta
  also does not cover *what* onboarding asks for, or the editor-setup journey,
  which reaches the welcome page rather than the front door and is unchanged.

### OMEGA-DELTA-0041 — Omega Agent is attachable over ACP, on a loopback socket that is off by default and read-only

- **Upstream Zed:** an ACP client only. `crates/agent_servers` reaches *out* to
  external agents and every `ConnectionTo<…>` in the tree is a
  `ConnectionTo<Agent>`. Nothing in upstream implements the agent role, so no
  external host can attach to the editor and use the agent it is configured
  with.
- **Omega:** Omega Agent — the router, not a raw provider — can be served over
  ACP to an external host on `127.0.0.1`. An attaching host initialises,
  creates a session, prompts it, and is answered with the same disclosed
  routing an in-app thread carries. omega#82's recursive composability, in the
  direction upstream does not have.
- **Off by default, and that is asserted rather than intended.**
  `OMEGA_ACP_SERVER` must be exactly `1`. `true`, `yes`, `on`, `01`, ` 1` and an
  unset variable are each off, with a typed `OffReason` rather than a bool, and
  `the_served_acp_surface_is_off_unless_the_flag_is_exact` fails on a
  truthy-tolerant read such as `to_lowercase()` or `parse::<bool>()`. A listener
  that is on by default is a different product.
- **Loopback by construction, not by configuration.** `LoopbackHost::new`
  refuses anything but `127.0.0.1` and `::1`, and refuses a *name* rather than
  resolving it — resolution is the step at which `localhost` can be made to mean
  something routable. `LoopbackAcpServer::bind` takes that type, so reaching a
  routable interface needs a new type rather than a new setting.
- **Read-only, inside the authority partition rather than beside it.** This is
  an **unauthenticated** model-driven surface — `authMethods` is empty, so it
  carries no bearer at all and is structurally weaker than the Desktop MCP
  surface. Owner gate 8 (*no model-initiated path can start Full Auto
  authority; only an explicit human action can, wherever that action lives*)
  therefore reaches it directly, and three model-callable Full Auto starts were
  removed from OpenAgents Desktop the same day. So the surface does not sit
  beside the partition: `SERVED_SURFACE` is the four methods a host can reach,
  every one of them observation, and `UNEXPOSED_AUTHORITY` classifies every
  authority-bearing control with the typed refusal a host attempting it gets —
  checked in **both directions** against `FULL_AUTO_AFFORDANCES` and
  `PinGesture::all()`, so adding a Full Auto control fails this crate's tests
  until somebody says what a served host is told. Anything not listed is refused
  by the absence of a dispatch entry, not by a branch someone has to remember.
- **A served session can never reach an engine lane.** A pin is the only door to
  one, setting a pin requires a `PinGesture`, and no variant of that enum is
  reachable over a socket. `a_served_session_can_never_reach_an_engine_lane`
  proves the consequence against every engine state the router can be shown —
  including an engine with idle lanes and an executor registered — so "the
  engine happened to be down" is not what is doing the work.
- **Ingress is a record beside the disclosure, not a fourth executor class.**
  `ExecutorClass` answers *who ran the work*; serving over ACP changes *who
  asked*. Reusing `ExternalAcp` would make a served session claim an external
  agent did work Omega did, and make one token mean opposite things depending on
  which side of the socket the reader stands on. `OMEGA-AGENT-AC-04` therefore
  stands unrevised at three classes and `SessionOrigin` carries the ingress.
  Both records cross the wire with **exactly** their declared fields —
  `EXECUTOR_DISCLOSURE_FIELDS` and `SESSION_ORIGIN_FIELDS`, asserted exactly
  rather than against a denylist, because a denylist that fails on `label`
  passes for `line`, `text`, `summary`, and `caption`.
- **GPUI never opens the socket.** `crates/omega_acp_server` declares no
  dependency on GPUI, `workspace`, `project`, `agent_ui`, or `ui`, and the only
  production caller of `start_if_enabled` is `crates/omega_effectd` — the
  supervisor layer. Both are checked by scanning the tree rather than asserted
  in prose.
- **Enforced by:** `the_served_acp_surface_is_off_unless_the_flag_is_exact`,
  `only_the_supervisor_opens_the_served_acp_socket`,
  `nothing_over_the_served_acp_surface_can_take_a_pin`,
  `the_served_surface_presents_the_first_party_agent_id`, and
  `the_supervisor_starts_the_served_surface_before_it_resolves_the_engine` in
  `crates/omega_deltas`; the fourteen checks in `crates/omega_acp_server`,
  including one that drives the real loopback socket with the **upstream** ACP
  SDK client and reads the disclosure back off it; and
  `the_disclosure_record_holds_no_rendered_label` plus
  `the_origin_record_holds_no_rendered_label` in `crates/omega_front_door`.
- **A served prompt ends its turn, and says what it did not do.** ACP has a
  `refusal` stop reason and it is the wrong one: it means *the prompt and
  everything after it will not be included in the next prompt*, and stock Zed
  1.12.0 implements that literally — it dropped the turn and showed a refusal
  banner with **no disclosure at all**. That was watched happening before this
  shape was chosen. The turn genuinely ends, so it says `end_turn`, and what did
  not happen is in the message the operator reads and in the typed record beside
  it.
- **What this does not cover.** The listener runs in the Omega process under the
  supervisor's control, **not** inside the packaged `@openagentsinc/omega-effectd`
  daemon; that daemon lives in the openagents repository and this packet is
  scoped to omega. What is enforced here is the property omega#82's falsifier
  names — the bind lives in a crate the UI layer cannot reach, and the start
  lives in the supervisor. The surface also does not *execute*: a served turn is
  answered by disclosing where it would route, and dispatching a served turn to
  a real executor is a later decision with its own authority question, not an
  omission. The v1 fleet-backed server (openagents #9179) stays deferred.
