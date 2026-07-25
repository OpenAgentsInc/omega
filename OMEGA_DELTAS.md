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
- **Enforced by:** `the_agent_ships_enabled` and
  `the_default_model_is_pinned`.

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
