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
- **Amended by OMEGA-DELTA-0186 (omega#162):** the debugger crates
  (`debugger_ui`, `debugger_tools`, `dap_adapters`) were deleted outright, so
  the terminate confirmation cannot return without the whole surface
  returning. The check now asserts `crates/debugger_ui` stays deleted; the
  original no-prompt policy binds any future deliberate revival.

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
- **The primary `cmd-n` / `ctrl-n` shortcut is global too.** It is the shortcut
  drawn by the Thread menu and must work while an auxiliary or unavailable
  route owns focus. A persisted Forensics route is restored on a deferred
  callback during startup; that callback now applies only while the restored
  route remains current, so a newer New Thread action cannot be overwritten by
  stale restoration. New Thread also performs the complete transcript-route
  transition, clearing Settings, Work detail, unavailable-route, and workbench
  surface state before focusing the composer.
- **The default is `openagents/omega-agent`** (owner direction 2026-08-08:
  Omega Agent is one cloud-native agent and the OpenAgents API owns its model
  selection and routing — see `OMEGA-DELTA-0201`). Earlier revisions of this
  entry recorded `ollama/llama3.1`, `google/gemini-3.6-flash`, and
  `openagents/gpt-5.6-luna`; each correction is recorded rather than left to
  mislead. The hosted default authenticates with the signed-in OpenAgents
  session, so a fresh install needs no local API key or model server.
- **Why the isolation test alone was not enough.** The service-isolation test
  asserts only that the default provider is `openagents`
  (`crates/app_identity/src/service_isolation.rs`), because what it protects is
  that the default never points at a *Zed* service. That is the right scope for
  that test, but it leaves the model string unpinned: a rebase could replace
  the logical `omega-agent` id with a provider model and every isolation check
  would stay green.
- **Enforced by:** `the_agent_ships_enabled`, `the_default_model_is_pinned`,
  `the_primary_new_thread_chord_reaches_the_workspace`,
  `the_new_thread_chord_is_window_global`, and
  `test_new_thread_supersedes_a_deferred_forensics_restore`. Together they
  assert both New Thread chords reach `agent::NewThread` from the window,
  reject a late restored Forensics route after newer user intent, and prove the
  resulting destination is the focused composer. The compatibility-chord test
  also asserts that every narrower binding of it is one of the deliberately
  admitted surfaces —
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

### OMEGA-DELTA-0015 — Sarah and Save As no longer borrow the same shortcut

- **Superseded contract:** omega#69 temporarily replaced the inherited Save As
  chord with `workroom::OpenPanel` and kept Save As discoverable through the
  File menu. The default surface later stopped rendering the workroom panel,
  so the replacement chord was refused, and the one-surface menu no longer has
  a File menu.
- **Omega now:** the three default keymaps bind neither
  `workroom::OpenPanel` nor `workspace::SaveAs`. The approved application menu
  intentionally omits Save As. In-place `workspace::Save` remains admitted for
  the sanctioned revealed editor. Sarah is visibly unavailable until her
  admitted voice entry has complete cohort, price, and authority truth; that
  work may assign a shortcut to `workroom::StartVoice` deliberately.
- **Why:** owner approval on omega#150, 2026-07-29. Retaining a refused Sarah
  shortcut or claiming a removed File-menu fallback would preserve two lies in
  the name of a superseded mitigation.
- **Enforced by:** `the_default_surface_has_one_honest_menu_contract` and the
  built-in keymap resolution checks in `crates/omega_deltas`.

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

- **Upstream Zed:** `crates/omega/resources/info/Permissions.plist` and
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
  `crates/omega/resources/` are unchecked. And no name is forbidden unless it is
  written down in `script/omega-brand-gate.json`. Rendered review of a
  candidate is still an owner step, not a mechanical one.

### OMEGA-DELTA-0019 — A window with nothing to restore opens on the agent

- **Upstream Zed:** `restore_or_create_workspace` in `crates/omega/src/main.rs`
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
  registered in `initialize_panels` in `crates/omega/src/zed.rs`, with
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
- **Where the line is drawn is not part of the rule; that it is drawn is.**
  omega#99. In zero base the line is not above the entries — it is the left
  half of the composer bar, beside the model dropdown and the executor pin.
  The owner asked for the input bar to become the centre of gravity and for
  everything else to go, and the disclosure went *with* the composer rather
  than away: it is still built from `ExecutorDisclosure` and still rendered by
  `Label::new(disclosure.label())` on every draw. Reading it beside the model
  a person is about to send to is, if anything, the better place for it. What
  changed is that the disclosure is now **conditional on the mode**, and a
  conditional obligation needs both branches checked — so
  `the_thread_surface_renders_the_executor_line_from_the_record` now pins the
  zero-base call site and the record inside it as well as the ordinary one.
  With only the original assertions, deleting zero base's bar would have left
  the check green while the mode whose whole purpose is to show one executor
  working stopped naming it.
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
     `IconName::Copilot`, and omega#162 subsequently deleted the
     `component_preview` crate and its `workspace::OpenComponentPreview`
     action outright. Removing the artwork and the surface are both done,
     because either alone leaves the other half of the failure standing.
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
  `removed_editor_crates_stay_removed` in `crates/omega_deltas/` (source tree),
  `no_vector_name_carries_a_competitor_name` in `crates/ui/`, and
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
  `crates/omega/resources/` — are unread here. And no check looks at a rendered
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
  `full_auto_enable` survived until it was removed today. The original v1 law
  routed an unpinned thread to the native loop; the 2026-07-29 three-mode
  direction supersedes that default so Omega Agent can choose eligible ordinary
  executors. The retained boundary is narrower and stronger: an engine lane is
  reachable only through explicit human authority. Model-advisory entry into
  Full Auto remains out of scope.
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
  `GitExcludeOverride` cleaned up both. Upstream deleted that struct on
  2026-07-26, because agent checkpointing no longer writes to
  `.git/info/exclude` at all, and the 2026-07-27 sync took the deletion: it has
  no callers left. Nothing writes either marker now, and nothing removes a
  block an older build left behind. Upstream carries the same gap for its own
  marker. A one-shot cleanup for an inherited block is a product decision, not
  a merge decision.
- **References deliberately kept**, each with a class and a reason in the
  policy: Zed's hosted service, account, plans, billing and servers
  (`Zed Pro Plan`, `Business Plan - Zed models enabled`, `Signs in to Zed
  account.`, `Zed's Edit Predictions`, the whole Zed Cloud provider); Zed's own
  documentation (`zed.dev/docs/...` in remote-development, keymap,
  Linux and Windows troubleshooting copy); Zed's authorship of the inherited
  One/Ayu/Gruvbox themes and the Windows Performance Recorder profile; the
  `Zed Plex Sans` / `Zed Plex Mono` family names recorded inside the shipped
  font files; and Omega-authored copy that names Zed in order to say how Omega
  differs from it. `Enable Fast Mode for Zed?` was the one genuinely ambiguous case — it
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
  assets; `docs/`, `.github/`, `script/` and `crates/omega/resources/` are
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

### OMEGA-DELTA-0034 — A working folder gates thread creation

- **Upstream Zed:** the agent panel requires an open project. `agent_ui: Require
  an open project for agent panel` (#56577) put a `has_open_project` early
  return in front of panel entry points.
- **Superseded Omega behavior:** Omega briefly allowed a projectless composer
  so a fresh window could accept a prompt immediately. The owner reversed that
  decision on 2026-08-03: a thread without a selected filesystem root has
  ambiguous tool and context authority, and a hidden draft before folder
  selection is not an honest front door.
- **Omega now:** without a selected working folder, the panel shows **Select a
  working folder to start a new thread** and one **Open Folder** action. It
  does not create or restore a draft, does not persist thread metadata, and
  does not show the inherited Clone Repository control. Command-N, direct-agent
  creation, terminal creation, startup restoration, and internal composition
  paths all fail closed in this state.
- **The first folder completes the transition.** When the Project Graph admits
  its first visible Project Root, the panel opens the normal new-thread
  composer and transfers keyboard focus to its input. Later folder additions
  do not replace an active thread or other meaningful destination state.
- **The sidebar names what the user selects.** The Omega shell labels the list
  **Working folders**, not Repositories. A working folder can contain a Git
  Repository, but the two are not the same model. Active thread-target rows and
  retained workspace rows use distinct element identities; selecting an
  inactive folder activates its retained Workspace instead of colliding with
  the active row's click handler.
- **The workspace-touching guards stay.** A terminal needs a working directory.
  Loading a thread from the clipboard, resuming a persisted draft, refreshing
  skills, initializing from a source Workspace, and starting an external ACP
  agent also require a selected folder.
- **`cmd-?` gave the agent panel back to macOS.** `agent::ToggleFocus` was bound
  to `cmd-?`, which is macOS's reserved Help chord — the Help menu's search
  field. Omega cannot win that keystroke, so the binding was a keybinding that
  looked present and did not work. It is `ctrl-cmd-a` now: free, not
  system-reserved, and on the same letter as `cmd-shift-a`. Linux (`ctrl-?`) and
  Windows (`ctrl-shift-/`) are untouched, because neither is reserved there and
  changing them would be churn with no defect behind it.
- **Enforced by:**
  `the_front_door_requires_a_working_folder_before_creating_threads` and
  `required_keymap_bindings_resolve` in `crates/omega_deltas`;
  `test_empty_workspace_requires_a_working_folder_before_creating_threads`,
  `test_new_thread_defaults_to_omega_and_syncs_the_selection`, and
  `omega_working_folder_list_switches_to_an_inactive_folder` in
  `crates/agent_ui`. The old projectless-composer visual baselines are retained
  only as historical evidence and are not acceptance evidence for this policy.
- **What this does not cover.** The `cmd-shift-a` bindings named in omega#76 are
  resolved rather than changed. Their platform-specific resolution remains
  checked by `the_new_thread_chord_is_window_global` under
  `OMEGA-DELTA-0013`.

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
- **Engine lanes are still reachable only through explicit human authority.**
  An unpinned Omega Agent route never auto-enters Full Auto authority. Direct
  Agent creation and ordinary external-ACP routing are not engine-lane
  authority, so this rule does not force those conversations onto the native
  loop. `an_unpinned_thread_never_reaches_an_engine_lane` in
  `crates/omega_front_door/src/router.rs` fails if that boundary changes.
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
  `crates/omega/src/visual_test_runner.rs`.
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

### OMEGA-DELTA-0040 — First launch provisions the Nostr identity in the background and opens the front door

- **Superseded twice, deliberately.** The original record put a first-ever
  launch on identity onboarding and released the startup wait from the
  first-run `on_finish`. A 2026-07-27 amendment removed the ceremony from the
  startup path but provisioned nothing, so a fresh profile sat at `Absent`
  forever and every identity-consuming surface (tester channels, device
  pairing, Sarah voice) failed for exactly the users being onboarded. This
  amendment (omega#164, owner direction 2026-07-29: *no onboarding flow; the
  Nostr identity work happens in the background*) closes that gap: the
  ceremony stays gone and the identity actually exists.
- **Upstream Zed:** first-run onboarding is a page you may open, skip, or
  close; nothing in the startup path waits for it, and the window opens
  regardless. Upstream has no identity to create, so it has nothing to
  provision either.
- **Omega:** startup awaits one shared background provisioning task before the
  front door opens. Custody's `provision_for_process_start` creates the
  keypair on `Absent` — a Nostr identity is the one identity type that permits
  full background creation: no email, no signup, no verification, a keypair in
  milliseconds with zero user input — adopts on `Unadopted` exactly as
  `OMEGA-DELTA-0110`'s path always did, resolves silently on `Ready`, and
  refuses every other state **by name**, because replacing a `Lost` or
  `Conflict` identity unattended is the omega#110 silent pick in the worst
  possible place. The process-start inspection also acknowledges a completed
  identity reset, which the dormant gate had silently stopped doing.
- **A refusal is logged, never a park.** `await_identity_ready` returns `Ok`
  either way and the thread opens; the surfaces that need the identity refuse
  with the same named state when touched. A launch that waits forever behind
  an unattended custody problem with no screen to repair it on is the dead end
  this delta family exists to delete.
- **The `onboarding::Finish` dead-end class is structurally impossible.** No
  completion channel exists for a UI action to forget to complete: the wait
  resolves when provisioning resolves, full stop. The first-run onboarding
  journey — `show_onboarding_view`, the first-run window mode, the
  release-waiters handoff — is removed rather than unrendered, and zero base's
  `onboarding::Finish` admission (omega#99) retired with the gate it existed
  to release (`OMEGA-DELTA-0051`).
- **The gate's purpose survives the ceremony.** Startup and zero base still
  call `await_identity_ready` before opening the front door, so no surface
  acts before custody has answered, and the seam for any future owner-decided
  startup journey is still that one call.
- **Enforced by:**
  `startup_provisions_identity_in_the_background_and_opens_the_front_door`
  in `crates/omega_deltas`, alongside `a_fresh_window_opens_on_the_agent`
  (`OMEGA-DELTA-0019`) and
  `the_front_door_requires_a_working_folder_before_creating_threads`
  (`OMEGA-DELTA-0034`); plus
  `process_start_provisioning_creates_adopts_and_is_idempotent` and
  `process_start_provisioning_refuses_every_state_it_cannot_answer` in
  `crates/omega_identity`, and
  `startup_provisioning_is_shared_by_concurrent_callers` and
  `a_provisioning_refusal_never_blocks_the_front_door` in
  `crates/onboarding`.
- **What this does not cover:** mode-scoped disclosure of the identity
  (channel handle defaults, Sarah truth at Sarah-selection, pairing at
  pairing), which each belong to their consuming surface; and the backup
  nudge, which is `OMEGA-DELTA-0183`.

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
  banner with **no disclosure at all**. The turn genuinely ends, so it says
  `end_turn`, and what did not happen is in the message the operator reads and
  in the typed record beside it. **Both shapes are photographed rendering**, in
  stock Zed 1.12.0 attached to this crate's socket, same build and same prompt,
  one value apart:
  `crates/omega_acp_server/evidence/2026-07-26-zed-1.12.0-served-turn-end_turn.png`
  keeps the turn and shows the executor and origin disclosure;
  `…-served-turn-refusal.png` shows what the rejected shape cost — the turn
  gone from the thread and a bare "Request Refused" banner guessing at a
  *content policy* violation that never happened. A test asserting
  `stopReason == "refusal"` was green across that entire difference, which is
  why this bullet is answered with pixels and not with a passing assertion.
- **What this does not cover.** The listener runs in the Omega process under the
  supervisor's control, **not** inside the packaged `@openagentsinc/omega-effectd`
  daemon; that daemon lives in the openagents repository and this packet is
  scoped to omega. What is enforced here is the property omega#82's falsifier
  names — the bind lives in a crate the UI layer cannot reach, and the start
  lives in the supervisor. The surface also does not *execute*: a served turn is
  answered by disclosing where it would route, and dispatching a served turn to
  a real executor is a later decision with its own authority question, not an
  omission. The v1 fleet-backed server (openagents #9179) stays deferred.
### OMEGA-DELTA-0042 — Omega drives Exo as a lane, and supplies the authority gate Exo lacks

- **Upstream Zed:** external agents are attached over ACP or configured as
  custom agent servers. There is no notion of an executor whose own agent can
  rewrite itself, and nothing in the fork's ancestry has ever had to bound one.
- **Which Exo, first.** This is the maintained `OpenAgentsInc/exo` fork of
  `exoharness/exo`, the recursive-self-improvement agent harness. Omega pins
  commit `cd7c0d29db869e953fb7261d8390ca93007d36a6` and tree
  `c61846e3f44daaf445930d1a499432ca9b069306`. The fork contains the ACP
  transport while the upstream change is under review. It is **not** exo labs'
  `exo-explore/exo` cluster-inference appliance, which shares a name and nothing
  else; omega#86 was closed for integrating the wrong one. The pin therefore
  carries the repository as a field a test reads, alongside the commit and the
  tree — both, for the same reason `omega-effectd` is pinned by release tag *and*
  asset digest, because a commit id alone is satisfied by a rewritten history in
  a clone nobody re-fetched. Exo declares itself unstable with the written house
  rule "do not write fallback code or handle backwards compatibility."
- **What Omega is integrating, stated plainly.** An unauthenticated local
  harness whose agent has an unrestricted networked shell and can rebuild
  itself. Exo has no approval prompt anywhere by design — its security model is
  sandbox isolation, and its threat model assumes you *want* the agent to modify
  itself. Omega adds the gate Exo does not have. The turn path runs these checks
  **before** it sends. Each check uses a live observation:
  - **where Exo is** — `EXO_EXOHARNESS_URL` is inherited from the environment,
    and setting it redirects the lane from the state root on disk to an HTTP
    server that has no authentication and full access to Exo's secrets. It is
    parsed through `LoopbackEndpoint`, whose only constructor refuses anything
    that is not this machine, and whose refusal separates `0.0.0.0` and `::`
    from loopback because those are the *plausible* mistakes;
  - **which Exo** — `git` in the checkout answers with a remote, a commit, and a
    tree, and the pin admits or refuses it;
  - **which bytes** — the binary is measured and compared against the owner's
    `omega_harness` pin ledger entry for `exo`, when the owner froze one;
  - **which capability** — Omega reads both `exo agent show` and
    `exo conversation show`. The read includes exact tool-module paths, module
    digests, and agent-level and conversation-level mounts. The normal path
    refuses a self-modifying turn. A person can use a dedicated warning dialog
    to authorize one exact draft. The one-use grant binds the source commit and
    tree, binary digest, agent, conversation, connection generation, objective,
    capability set, tool-module digests, mount paths, turn reference, and
    expiry. Drift, reuse, restart, cancellation, and a different draft refuse.
    Networking is reported and is not refused.
- **Prompts cross ACP, not command-line arguments.** Omega starts
  `exo --root <root> acp <agent> <conversation>`. ACP JSON-RPC carries the
  prompt on standard input. User text cannot become Exo argument syntax.
- **Omega never configures Exo.** The admitted command starts `exo acp` and
  reads capability records. `create`, `update`, `delete`, `mount`, `set`,
  `register`, `configure`, `serve`, and `repl` are unreachable. Exo owns its
  state and its durable turn log.
- **The executor class, decided rather than defaulted.** `ExecutorClass` is
  closed at three by `OMEGA-AGENT-AC-04`, and it answers *who ran the work*. An
  Exo thread reports `ExternalAcp`. Not `NativeLoop`, which is the first-party
  claim and would present an unrestricted shell's output as Omega's own. Not
  `EngineLane`, which *is* Full Auto authority — a record on it must carry a
  `run_ref`, and owner gate 8 admits only an explicit human action into it. Exo
  has no engine run and no receipt, so a lane reporting `EngineLane` would be a
  fourth model-reachable door into Full Auto authority opened by adding an
  executor, three of which were removed from OpenAgents Desktop the same day.
  `ExternalAcp` fits because the class is about the executor and not the wire: a
  separate process Omega does not own, carrying no run reference. ACP does not
  change the class. A fourth variant was considered and **not
  taken**; the argument is written in `crates/omega_exo_lane` so a later reader
  can disagree with something concrete.
- **The disclosure names Exo, its executor, and its model** —
  `external_acp · exo/basic · provider not disclosed/gpt-5-mini`, from a real
  turn. No field was added to `ExecutorDisclosure`: the record's field list is
  closed on purpose. `provider` is genuinely absent because Exo's LLM binding
  has no provider field at all, only an optional base URL, and saying "not
  disclosed" is better than deriving `openai` from an absence.
- **The stream is typed.** Exo maps each `ExecutionStreamEvent` to ACP. Omega
  receives live text chunks, tool calls, tool results, and a completion record.
  The completion metadata includes Exo's durable session, turn, and latest-event
  references. ACP cancellation uses cooperative executor cancellation. Exo
  appends a cancellation event and closes the durable turn before it reports
  `Cancelled`.
- **Turn-boundary behavior is explicit.** Exo declares `CannotSteer`. Omega
  queues a mid-turn prompt. It does not guess that Exo can steer, and it does not
  turn a steer request into an implicit cancellation.
- **Self-modification receipts are durable.** Omega writes the one-use authority
  decision and Exo's returned durable references to
  `exo-self-modification-receipts.jsonl`. The thread shows the outcome,
  generation, and latest event reference.
- **Enforced by:** `the_exo_lane_drives_the_harness_exo_and_not_the_cluster_one`,
  `the_exo_lane_puts_no_user_text_before_the_argument_terminator`,
  `the_exo_lane_exposes_no_endpoint_off_this_machine`,
  `the_exo_lane_opens_no_path_into_full_auto_authority`,
  `an_exo_turn_checks_the_pin_and_the_agent_before_it_sends`,
  `an_exo_turn_streams_cancels_and_requires_exact_one_use_authority`, and
  `the_exo_lane_is_reachable_from_omega_agent` in `crates/omega_deltas/`, the 40
  unit checks in `crates/omega_exo_lane/`, and `drives_a_real_exo` in
  `crates/agent_ui/`, which is `#[ignore]`d because it needs a real Exo.
- **Falsified against a running Exo,** every edit probed before its run. Pointing
  the checkout's remote at `exo-explore/exo` refused the turn by name. Enabling
  `tool_creation` on the live agent refused it. Mounting Exo's own source tree
  `rw` refused it. `EXO_EXOHARNESS_URL=http://100.64.7.9:4766` refused it before
  any Exo process started. Dropping the argument terminator, making the turn
  reader accept any stdout, reporting `EngineLane`, unwiring the lane from the
  router, and moving the capability read after the send each failed their tests.
  **One falsification found a real hole:** removing the disclosure's Exo arm
  left the live test green, because it asserted only the agent id — which the
  fallback also produces. The live test now asserts the model, which only that
  arm supplies, and the same edit fails it. A second falsification silently
  no-opped through a shell quoting error and was caught by its probe rather than
  by its result.
- **What this does not cover.** The grant does not give Exo Full Auto
  authority. Omega does not edit Exo configuration. Exo remains an external ACP
  executor and has no engine run reference.

### OMEGA-DELTA-0043 — `--uninstall` removes the installation, not one file inside it

- **Upstream Zed:** `script/uninstall.sh` removes `/Applications/Zed.app` — the
  bundle — along with the user-level directories. Correct for Zed.
- **Omega, before this:** `OMEGA-DELTA-0036` moved the path table out of the
  shell script and into `UninstallRoots`, built from the `paths::` functions
  that write those directories, and that fix is real: the plan derived by the
  signed `0.2.0-rc16` holds nine Omega paths and zero belonging to anybody else.
  But `crates/cli/src/main.rs` handed it `app.path()`, which on macOS is
  `Omega.app/Contents/MacOS/omega` — **one executable inside the bundle** — into
  a field whose doc comment read "the application bundle **or executable** this
  CLI belongs to". So a full uninstall run against the signed `0.2.0-rc16`
  reported success and left `/Applications/Omega.app` standing with **130.9 MB
  and five executables** in it: `cli`, which still carries `--uninstall`,
  `omega-identity-proof`, `omega-effectd` and its **bundled Node runtime**. Zed
  was safe and Omega was not removed; the user was left to drag a signed,
  non-functional bundle out of `/Applications` by hand (omega#92).
- **Why nothing caught it.** The end-to-end test planted the app root as a
  directory holding a single marker file, and asserted the root was gone.
  "Removed the bundle" and "removed the one file the plan named" are the same
  observation from a root like that. And `InstalledApp` had a single `path()`
  answering two different questions, shadowed on macOS by an inherent `path()`
  that returned the bundle — so the same call spelled two ways returned two
  different paths, and the uninstaller took the wrong one.
- **Omega now:** `InstalledApp` has two methods with two names.
  `path()` is the executable to run; `installation_root()` is the whole
  directory a removal has to remove, and on macOS it is the `.app`. The
  ambiguous inherent `path()` is gone, so there is one answer per question. The
  field is `UninstallRoots::app_root`, and `from_installed_paths` runs whatever
  it is given through `uninstall::installation_root`, which resolves any path
  under a `.app` to the outermost `.app` — a caller holding an executable, which
  every macOS caller does, cannot reintroduce the defect from outside. A loose
  development build with no bundle above it is returned unchanged, because
  inventing a root there would plan its parent directory for removal.
- **Enforced by:** `the_uninstall_plan_names_the_installation_root` in
  `crates/omega_deltas/` (the derivation exists, the constructor normalizes
  through it, the call site asks for the installation, and the trait still
  distinguishes the two), and by two tests in `crates/cli/src/uninstall.rs`:
  `the_plan_names_the_bundle_root_and_never_an_executable_inside_it`, which
  asserts both directions — the bundle is planned, and no file inside it stands
  in for it — and the end-to-end
  `the_script_removes_omega_and_leaves_the_other_editor_untouched`, whose
  fabricated bundle now holds `omega`, `cli`, `omega-identity-proof`, the
  `omega-effectd` bundle and its Node runtime, every one of which is read back
  after the real script has run. The bundle path in that test is named
  independently of the plan, so a plan that names the executable plants its
  files somewhere else and fails rather than moving with the defect.
- **Proved against the product, not only the tests.** The plan was derived by
  running the built `cli --uninstall` with `OMEGA_UNINSTALL_DRY_RUN=1` under an
  isolated `HOME` against a fabricated bundle, exactly as omega#92 recorded the
  defect. Before: `remove: …/Omega.app/Contents/MacOS/omega`. After:
  `remove: …/Omega.app`, with the other eight roots unchanged.
- **Falsified.** Reverting the call site to `app.path()` reproduced the
  executable in the product's own dry-run plan and failed the delta test.
  Making `installation_root` return its argument unchanged failed the plan test
  and left `cli` alive in the end-to-end test. Both edits were probed in the
  file and in the rebuilt binary before their tests ran.
- **Handled with the care this path has earned.** This is the code that
  destroyed the owner's real Zed installation on 2026-07-25, when a
  falsification restored the old script and one test did not override `HOME`.
  Every manual exercise here ran with `OMEGA_UNINSTALL_DRY_RUN=1` and an
  isolated `HOME`, against a fabricated bundle in a scratch directory. Nothing
  was ever run against `/Applications`.

### OMEGA-DELTA-0044 — A brand-bearing command form is user-facing text

- **Upstream Zed:** the CLI's interactive open-behavior prompt shows
  `zed --existing`, `zed --classic` and `zed <path>`. Correct for Zed.
- **Omega, before this:** those three literals shipped verbatim in the signed
  `cli` of `0.2.0-rc16`, inside a panel whose surrounding copy reads "Add to
  existing **Omega** window", "You can change this later in **Omega**
  settings" — naming our product twice and somebody else's binary three times.
  `zed --existing` and `zed --classic` are not Omega commands at all
  (omega#93).
- **They are product claims by the gate's own rule.** `OMEGA-DELTA-0031` is
  written around one test: substitute our own name, and if the sentence stays
  true the brand was standing where our product's name belongs. "Add to existing
  Omega window (`omega --existing`)" is true of Omega. All three are rewrites,
  not classifications.
- **Why the gate passed.** `is_prose` requires three tokens, so a two-token
  command form is invisible to it. That blind spot was *known* — the
  `0.2.0-rc16` release notes record "does not classify one- and two-word
  labels" — and this is it biting: the three literals were in neither
  `prose.classified` nor the compatibility allow-list, and
  `verify-omega-brand --app` exited 0.
- **Omega now:** the prompt builds every command form from
  `paths::BINARY_NAME`, so it cannot drift from the binary it describes. And
  the blind spot is **narrowed rather than papered over**: the inventory admits
  prose *or* a command form — a brand word standing in `argv[0]` followed by
  flags, placeholders or paths — in all three streams (Rust literals, doc lines,
  embedded assets), on both the Python and the Rust side. Adding the three
  strings to a denylist instead would have reproduced exactly the defect the
  inverted inventory was built to end.
- **The width was measured, not asserted.** Across the whole tree the
  command-form shape adds **3** items — the three defects, and nothing else.
  Admitting any two-token literal beginning with a brand word instead adds
  **11** (8 more: `Zed Pro`, `Zed Agent`, `Zed AI`, `Zed Repository`,
  `Zed Twitter`, `Zed Default`, and `Zed (Default)` twice), all genuine
  references to somebody else's product that would need classifying. Admitting a
  bare brand token adds **58**. A gate that cries wolf gets deleted, which is
  how a known blind spot stays open; the shape that costs nothing is the one
  that can be carried.
- **Enforced by:** `a_brand_bearing_command_form_is_user_facing_text` in
  `crates/omega_deltas/` (the three shipped literals are invisible to `is_prose`
  and visible to `is_command_form`; the eight two-word labels and a bare brand
  token are not command forms; and `script/verify-omega-brand` carries the same
  rule wired into the same three streams, so the source and packaged sides
  cannot drift), and `the_cli_prompt_names_our_own_binary`, which reads
  `prompt_open_behavior` and fails on any brand hit or on a command form not
  built from `paths::BINARY_NAME`.
- **Reachability, and what was actually observed.** `resolve_open_behavior` in
  `crates/omega/src/zed/open_listener.rs` sends `CliResponse::PromptOpenBehavior`
  when there are existing windows, the paths are not already in a workspace, at
  least one path is a directory, and the settings file has no
  `cli_default_open_behavior`. omega#93 recorded a *shipped-string* result,
  because the reporting lane could not get the installed CLI to render the panel
  with all four conditions held. This lane rendered it: the built `cli` was run
  under a pty against a stand-in for the editor that performs the real IPC
  handshake and sends `PromptOpenBehavior`, and the panel came up reading

  ```
  Configure default behavior for omega <path>
  You can change this later in Omega settings:
  > Add to existing Omega window (omega --existing)
    Open a new window (omega --classic)
  ```

  So the CLI half is live code, not dead code, and the copy it renders is ours.
  What that does **not** establish is that a running Omega reaches the four send
  conditions in practice — the stand-in supplied the response. That half is
  still unobserved, and the panel is still worth deleting if nobody can make a
  real Omega send it.
- **Falsified.** Restoring `zed --existing` failed both the prompt test and the
  unclassified-prose test. Restoring the three-token rule alone — leaving the
  literals fixed — failed the command-form test. Removing the rule from
  `script/verify-omega-brand` while keeping it in Rust failed the parity
  assertion. Each edit was probed in the file before its test ran.

### OMEGA-DELTA-0045 — A provider handoff is visible in the thread the owner reads

- **Upstream Zed:** `AgentThreadEntry` has six variants — a user message, an
  assistant message, a tool call, an elicitation, a completed plan, a context
  compaction. Every one of them is something a model or a user said. Correct for
  Zed, which has no supervising host writing into a thread.
- **Omega, before this:** Omega does have one. `omega-effectd` moves a Full Auto
  run from one provider lane to another and emits a note naming both lanes,
  addressed to the target thread. The host method it calls,
  `omega_host_bridge::append_system_note`, validated its parameters and then
  answered `unavailable("Agent threads do not expose an owner-visible
  system-note authority.")` — because there was no entry kind a non-model
  disclosure could be, so there was nowhere to put one.
- **Why that is a defect and not a limitation.** `0.2.0-rc11` shipped a handoff
  that changed **which model was spending the owner's budget** and left nothing
  in the transcript the owner reads. The refusal is an improvement on rc11's
  silent `() => {}` — it is typed, it is honest, and an operator reading the
  wire can see the disclosure was dropped — but the owner reading the thread
  cannot, and the owner reading the thread is the person the disclosure is for.
  An independent reviewer confirmed the refusal string is in the shipped bytes
  of `0.2.0-rc15` **and** `0.2.0-rc16`; it is in `rc17` too. No candidate to
  date discloses a cross-provider handoff in the thread.
- **Omega now:** a seventh variant, `AgentThreadEntry::SystemNote`, carrying an
  engine-supplied id and plain text; `AcpThread::push_system_note`, idempotent
  on that id; and `ThreadView::render_system_note`, which draws it as a
  captioned rule in the transcript — the same shape the thread already uses for
  "Subagent Output". `append_system_note` resolves the thread named by
  `threadRef` and appends.
- **Three properties the shape is chosen for.**
  - **Unconditional.** No collapse, no expansion toggle, no hover. The gate is
    owner *visibility*; anything the owner has to click to see is a disclosure
    the rc11 handoff would also have passed.
  - **Not Markdown.** The text is a `SharedString` the host wrote, drawn as a
    `Label`. Nothing a provider emits can style one of these lines or pass
    itself off as one.
  - **Idempotent on the engine's id, not last-write-wins.** The engine may retry
    after a response it never saw. A retry must not be able to rewrite a
    disclosure the owner has already read, and must not double it either. The
    idempotence is per live thread, which is the scope that matters: the note is
    an entry in the thread, and a thread that is gone has no owner reading it.
- **The cost, stated.** This is a variant added to `crates/acp_thread`, a shared
  upstream crate, so it is a real rebase surface — unlike `OMEGA-DELTA-0021`,
  which deliberately kept the executor record out of that crate behind an
  extension trait. An extension trait cannot add an enum variant, and an
  out-of-band side table keyed by entry index would not survive reordering, would
  not appear in the thread's Markdown export, and would put the disclosure
  somewhere other than where the transcript is read. The variant is the seam;
  the rebase cost is accepted deliberately.
- **Enforced by:** `a_host_authored_note_is_a_thread_entry_kind`,
  `the_host_appends_a_provider_handoff_note_rather_than_refusing_it`, and
  `the_thread_surface_draws_a_host_authored_note_unconditionally` in
  `crates/omega_deltas/`. The three halves are separable and each one alone is
  passable and useless: a variant nothing renders discloses nothing, a renderer
  nothing dispatches to discloses nothing, and a host method that returns
  `{"appended": true}` without writing anything is a refusal that lies rather
  than a refusal that is honest.
- **Scope.** This is the **source** half. `0.2.0-rc17` and every earlier
  candidate carry the refusal in their shipped bytes; the packaged half needs
  the next candidate.
### OMEGA-DELTA-0046 — Exo threads have a native conversation workspace

- **Before this change:** An Exo thread used the standard transcript and
  composer, but its Exo-specific interface was one compressed disclosure row.
  The row did not show the observed harness state, the current turn state, or
  the exact authority boundary. A user could not inspect the Exo lane as one
  coherent system.
- **Omega now:** An Exo thread has a dedicated workspace header and runtime
  inspector. The transcript and composer remain the standard Omega components.
  Text, tool calls, tool results, errors, completion, queued prompts, and
  cancellation therefore keep one rendering and control path.
- **The inspector is a projection, not a second source of truth.** It reads the
  exact observation that gates each turn. It shows the agent, conversation,
  executor, model, provider disclosure, ACP transport, turn state, source pin,
  binary digest, tool modules and digests, writable mounts, network state, and
  the last one-turn authority receipt. Each terminal turn also shows the
  durable Exo session, turn, and latest event references that Exo returned,
  whether or not that turn needed self-modification authority. If observation
  fails, the inspector reports that failure and does not invent runtime facts.
- **The controls use existing authority.** Refresh runs the read-only Exo
  observation. Stop uses the existing ACP cancellation path. The one-turn
  self-modification control appears only when the observed capability set needs
  it, and it uses the existing exact-draft confirmation and receipt path.
  Omega does not add an Exo configuration command, listener, proxy, or Full
  Auto route.
- **The layout is responsive and native.** A wide window puts the inspector
  beside the transcript. A narrow window puts a bounded inspector above it.
  The interface uses the active Omega theme tokens and standard controls. It
  does not add a separate dark theme or a web surface.
- **Enforced by:**
  `an_exo_thread_has_a_live_workspace_and_exact_runtime_inspector` in
  `crates/omega_deltas/`, the Exo inspection and turn-state tests in
  `crates/agent_ui/`, the real Exo acceptance path `drives_a_real_exo`, and
  `omega_exo_workspace_wide` plus `omega_exo_workspace_narrow` in the native
  Metal visual runner. The visual path starts the shipped ACP transport, sends
  one real turn, requires the reply and all three durable references, and then
  records both layouts.

### OMEGA-DELTA-0047 — Zero base is read from the process command line and from nowhere else

> **`OMEGA-DELTA-0052` changed which way the default points, and omega#161
> removed the mode entirely.** There is no reader left: the crate has no entry
> point, `is_active` is constant `true`, and no argument selects a surface.
> The property this delta protected — nothing but the person at the keyboard
> could change the window's shape — is now enforced by absence: the check
> fails if an entry point, a mode selector, or a settings/env/file reader
> grows back. Read the historical sentences below as the record of how the
> mode worked while it existed.

- **Before this change:** Omega had one surface. Every mode-like behaviour it
  had was a setting, and a setting is writable by a project settings file and by
  anything else that can write settings.
- **Omega now:** `omega` opens one window that shows one Exo thread and the
  controls that operate it. `omega --full-editor` is unchanged Omega.
- **The mode is read from the process command line, once, and from nowhere
  else.** `crates/omega_zero_base/` reads no settings store, no environment
  variable and no file. The only writer of its entry is the argument parser in
  `crates/omega/src/main.rs`. A rejected alternative is recorded for each: a
  setting, because a project settings file must not be able to hide
  authority-bearing surfaces (`OMEGA-DELTA-0020` records the same objection
  against a composer mode flag); a release channel, because that is a second
  product with its own update path and proof matrix; and a second binary,
  because `OMEGA-DELTA-0038` requires the packaged gate to open every executable
  that ships.
- **Nothing persists.** The mode is never written to disk, so ending the process
  is a complete repair. It used to say here that a visible control on the status
  bar leaves the mode inside the window. `OMEGA-DELTA-0052` removed that control
  and the whole leave path with it: a process that starts in zero base stays in
  zero base until it exits.
- **Enforced by:** `zero_base_is_entered_only_from_the_command_line` in
  `crates/omega_deltas/`, and the mode's own tests in
  `crates/omega_zero_base/`.

### OMEGA-DELTA-0048 — The transitional shell hides by filter and refusal until subtraction becomes the product

- **Amended by the 2026-07-29 single-experience direction.** “Deletes
  nothing” was a safe migration constraint, not the final product shape. Omega
  is converging on one application surface; the legacy editor and its bindings
  are scheduled for post-alpha removal in dependency-safe batches. Until those
  batches land, the filter, refusal gate, and intact keymaps remain
  load-bearing and this delta continues to check them rather than pretending
  the deletion has already happened.
- **Amended by omega#161.** The mode split is removed: the filter and refusal
  gate install unconditionally, `initialize_panels` has exactly one shape, and
  the refusal sentence names no flag because there is no editor to start. The
  refusal-and-filter mechanisms and the intact keymaps remain load-bearing
  until omega#162 deletes the gated crates; this delta keeps checking them.

- **Before this change:** Omega's two ways to make a surface go away were
  deletion (`OMEGA-DELTA-0009`, `OMEGA-DELTA-0012`) and a settings default.
  Deletion is not available here: the built-in keymap is loaded and unwrapped at
  startup, so a binding naming a missing action kills the process before any
  window opens while `cargo check --workspace` stays green. `0.2.0-rc6` died
  that way.
- **Omega now:** Zero base hides by two mechanisms and removes nothing. The
  editor-only panels, status-bar indicators, editor pane and tab bar are **not
  rendered**. Files, Git, and Terminal are the deliberate exception: the
  default surface draws those workbench controls, so production and the proof
  harness both call `agent_ui::initialize_workbench_panels` to fully load and
  register their three native panels before constructing `AgentPanel`.
  Everything outside the admitted set is **disabled** — the command palette is
  restricted to `omega_zero_base::ADMITTED_NAMESPACES` and
  `ADMITTED_ACTIONS`, and an action outside that set is refused at dispatch.
- **A refusal is a sentence, never a silent no-op.** The refusal names the
  action and says what Omega is instead. `OMEGA-DELTA-0052` cut the first of
  the two ways out it used to name (the control in the window), and omega#161
  cut the second (the editor flag): there is nothing left for the sentence to
  offer, so offering anything would be a lie.
- **The gate is the reason "not rendered" is safe.** A surface that is only
  visually absent is still one key press away, so the mode installs an action
  gate consulted before any listener runs, and the two halves are applied
  together to the Full Auto entry and the Full Auto start control.
- **No action and no key binding is deleted.** `assets/keymaps/default-macos.json`
  and its Linux and Windows siblings are untouched, and every namespace zero
  base hides is still bound in all three.
- **The shipped and proof front doors share the initializer.** This ordering is
  not an optimization: `AgentPanel::new` snapshots
  `workspace.panel::<ProjectPanel>()`, `GitPanel`, and `TerminalPanel`. Loading
  them concurrently with Agent Panel can permanently capture three `None`
  values. The helper returns only after all three registrations exist; the
  zero-base branch and `AgentWorkbenchFrontDoor::mount` both await it before
  constructing Agent Panel.
- **Enforced by:** `the_transitional_shell_hides_by_filter_and_refusal_until_subtraction_lands`
  in `crates/omega_deltas/`, which also requires the three keymap files to still
  bind the hidden namespaces, and `keymaps_name_no_deleted_action`, which stays
  green.

### OMEGA-DELTA-0049 — A zero-base turn still names its executor

- **Before this change:** the executor disclosure line, `OMEGA-DELTA-0021`, is
  the control that reaches the Exo lane — a thread routes to Exo exactly when a
  person pins `ExternalAcp` on it — and it is also the line that stops Omega
  presenting somebody else's output as its own. A mode whose whole purpose is
  subtraction is the most likely place for it to be subtracted by accident.
- **Omega now:** zero base draws the same executor disclosure line, built by the
  same `omega_executor_disclosure` binding from the same typed record.
  `conversation_view/thread_view.rs` does not know that zero base exists; there
  is no zero-base branch in the surface that draws the line, which is why there
  is no second code path for it to drift down.
- **The pin control that used to sit beside it is gone.** `OMEGA-DELTA-0055`
  removed it. The sentence above this bullet used to say a thread routes to Exo
  exactly when a person pins `ExternalAcp` on it, and while that was true the
  pin was the door as well as the statement — which is why this delta asserted
  the control was drawn. It is not the door any more: an unpinned thread runs on
  an attached external agent automatically. **The pin assertion was removed from
  this delta's check**, and it was removed because the policy changed rather
  than to make anything pass. What stays is the half that still holds: the line
  is drawn, and it has no cheaper zero-base-specific rendering path.
- **Photographed, not asserted.** `omega_zero_base_wide` and
  `omega_zero_base_narrow` are recorded by `run_omega_exo_visual_tests`, so the
  turn in the picture is a real streamed Exo turn over `exo acp` — text
  deltas, a `shell` tool call, its result, and the durable Exo session, turn
  and event references — and not a mock. Compared in-process through Metal at
  `MATCH_THRESHOLD` 0.99. Until omega#161 the same capture also recorded
  `omega_exo_workspace_wide`/`_narrow` on the `--full-editor` surface, with a
  one-way mode flip between the pairs and a per-scene surface assertion; the
  mode split's removal retired that pair, the `ExoSceneSurface` enum, and the
  flip machinery, and the check now refuses their return.
- **Every wait in the capture is bounded.** This is what had made these
  baselines unrecordable, and it is worth naming precisely because the first
  diagnosis was wrong. The capture waited with `run_until_parked`, which returns
  only when the scheduler has nothing left to run. That is right for a suite
  whose tasks are all simulated and wrong while a real `exo acp` child is
  attached: the ACP transport's read of the child's stdout becomes runnable
  again as soon as it is polled, so the call never returned. The runner spun on
  one core and hung — before the turn, before any screenshot — and a suite that
  hangs is worse than one that fails, because it says nothing. Each wait now
  spends a step budget and moves on.
- **The suite ends the process it started.** `run_omega_exo_visual_capture`
  ends the `exo acp` process by name rather than waiting for `AcpConnection`'s
  `Drop`, which runs only once every owner has let go — and the owners include
  GPUI entities whose teardown the runner can ask for and cannot observe.
  Sampled every 100ms across a full run with that call removed, a capture's
  child was still alive while the next capture's child was starting. It was not
  what hung the suite, and it is still not something a scene should depend on.
- **Enforced by:** `a_zero_base_turn_still_names_its_executor` in
  `crates/omega_deltas/`, and the two committed baselines under
  `crates/omega/test_fixtures/visual_tests/`.
- **One token in that check changed with `OMEGA-DELTA-0052`.** The check used to
  require the runner to call `omega_zero_base_ui::install_on_workspace`, which
  put the mode's status-bar control on the captured workspace. That control no
  longer exists. What the token protected — "the zero-base scene is built by the
  shipped surface code, not by a stand-in" — is now protected more directly: the
  capture refuses to photograph a scene whose declared surface does not match
  the mode's actual state, so an ordinary-surface baseline cannot be recorded
  with zero base on, or the reverse.
- **Scope, and why it was narrowed.** The check reads the bodies of
  `render_executor_disclosure` and `render_executor_pin`, not the whole of
  `thread_view.rs`. It was first written against the whole file, on the
  reasoning that a branch anywhere could reach the line. That was too strong.
  Zero base has to know the mode elsewhere in the same file: an empty
  transcript must claim the vertical space so the composer sits at the bottom
  rather than floating at the top, which is the layout the owner asked for
  directly. A rule that forbids the fix it exists to protect is the wrong rule,
  so the scope is now the two functions that draw the line. Layout elsewhere in
  the file may name the mode; the disclosure and its pin may not.
- **What this does not cover.** A cheaper rendering of the line placed in some
  other file is not caught here. The pairing with the baselines is what makes
  that visible: both scenes are photographed by the same capture that runs a
  real Exo turn, so a second mocked path would have to forge a turn to hide.

### OMEGA-DELTA-0050 — Zero base opens no authority path

- **Before this change:** Owner gate 8 closes the admitted launch origins at four
  in `origins_are_all_human_gestures` and the admitted pin gestures at two in
  `pin_gestures_are_all_human_gestures`. A mode that pre-pinned its thread to the
  Exo lane would need a third pin gesture, which is an edit to a closed list.
- **Omega now:** Zero base makes no such edit. It pins nothing. The viewer sets
  the pin with one visible click on the disclosure line, which also demonstrates
  the line doing its work.
- **No zero-base path reaches Full Auto.** The Full Auto entry in the agent
  panel's new-thread menu is not rendered, `open_full_auto` and
  `toggle_full_auto` refuse, the `full_auto_panel` namespace is outside the
  admitted set so its actions are refused at dispatch, and the "Start Full Auto"
  control is neither rendered nor able to start a run.
- **No change to the Exo lane.** Zero base writes no Exo configuration, opens no
  listener, proxies no `exo serve`, does not bypass the one-use Tier C grant, and
  changes none of the four preflight refusals in `OMEGA-DELTA-0042`.
- **`OMEGA-DELTA-0040` keeps its order.** A first-ever launch still lands on
  identity onboarding; the flag adds no bypass. Zero base's panel branch awaits
  the identity gate before it opens and zooms its panel, because a mode that
  merely covered onboarding with a zoomed panel would be a bypass of an identity
  gate wearing a layout's clothes.
- **Enforced by:** `zero_base_opens_no_authority_path` in
  `crates/omega_deltas/`, alongside the unchanged
  `origins_are_all_human_gestures` and `pin_gestures_are_all_human_gestures` in
  `crates/omega_front_door/`.

### OMEGA-DELTA-0051 — Zero base derives its setup, and can finish the one step it still asks for

- **Amended by omega#164 (owner direction 2026-07-29): the identity step is
  silent.** `OMEGA-DELTA-0040` now provisions the Nostr identity in the
  background, so no startup path opens the onboarding page in zero base and no
  UI action releases any startup wait. The `onboarding::Finish` admission this
  record added is therefore retired — the dead end it repaired can no longer
  exist, and an admitted action whose only job is gone is a door frame
  standing where the room was demolished. The identity-only page branch stays
  as defence in depth for any future admission that reaches the page. The
  amended check asserts the whole `onboarding` namespace is refused in zero
  base. Everything below is the historical record of the ceremony era.
- **Upstream Zed:** first-run onboarding is one page that asks for a theme, a
  base keymap, editor imports, vim mode, and telemetry, then hands off to the
  editor. Every question is a preference with a shipped default behind it.
- **Omega, before this:** `--zero-base` rendered that entire page. On a fresh
  `--user-data-dir` the mode advertised as "one Exo thread and nothing else"
  opened on a theme picker, a keymap picker offering eight other editors, an
  agent-install grid, an import-settings row, and two toggles. The owner named
  it directly: *"there should be no connect screen if you can auto detect i
  have codex and all that shit like the onboarding screen"*. The agent grid was
  the sharpest version of the complaint — it showed Codex with a green check,
  meaning detection had already happened, and asked about it anyway.
- **Omega now:** in zero base `render_basics_page` renders the identity section
  and nothing else. Theme, base keymap, agent install, import settings, vim
  mode, worktree auto-trust and telemetry are **not rendered**; each has a
  shipped default, and zero base takes it rather than asking. Nothing is
  deleted — without the flag the page is byte-identical to what shipped.
- **The identity step stays, and the delta boundary is why.** `OMEGA-DELTA-0040`
  binds two things: that a first-ever launch lands on identity onboarding, and
  that the first-run `on_finish` releases the startup wait. It says in as many
  words that it "does not cover *what* onboarding asks for". So the preference
  chrome is outside that delta and may go; the identity gate is inside it and
  stays. Rendering zero base with no identity step would be the same bypass the
  previous lane already caught itself committing with a zoomed panel — an
  identity gate skipped while wearing a layout's clothes.
- **The dead end this fixes, which is the part that was actually broken.**
  Zero base's action gate refused `onboarding::Finish`. `OMEGA-DELTA-0040`
  parks startup on `await_identity_ready`, and the *only* thing that releases
  it is the first-run branch of `on_finish`, reached from that action. So a
  fresh profile in zero base landed on identity onboarding, created an
  identity, pressed "Finish Setup" — and nothing happened, permanently, across
  restarts, because identity never became ready. Observed as the log line
  `onboarding::Finish is off in zero base`. `onboarding::Finish` is now
  admitted. That admits *completing* the identity gate, which is the opposite
  of bypassing it, and `SignIn`, `OpenAccount` and `ResetHints` stay refused.
- **Why this was not caught.** Zero base is entered only by a command-line flag,
  so no test that does not launch the binary with that flag can see any of it.
  `cargo check`, `cargo test -p omega_deltas` and `./script/clippy` were green
  across both this dead end and a startup panic in the same branch. The same
  shape as `keymaps_name_no_deleted_action`: a mode whose failures are invisible
  to a compiler needs a check that reads the mode's own admitted set.
- **Enforced by:** `zero_base_derives_setup_and_can_finish_identity_onboarding`
  in `crates/omega_deltas/`, which pins the identity-only branch in
  `crates/onboarding/src/basics_page.rs` and the `onboarding::Finish`
  admission, alongside the unchanged `OMEGA-DELTA-0040` checks.
- **What this does not cover.** Leaving zero base restores the full page for
  any later visit, and the editor-setup journey outside a first run is
  unchanged. This does not make onboarding unclosable, and closing it is still
  the dead end `OMEGA-DELTA-0040` already records.

### OMEGA-DELTA-0052 — The flag-free surface is the product and the legacy editor flag is transitional

- **Amended by the 2026-07-29 single-experience direction.** A normal,
  flag-free launch is the only advertised Omega experience. The
  `--full-editor` path may remain for the alpha transition, but it is not a
  second product and is scheduled for removal after the alpha gate.
- **Amended by omega#161: the transition is over.** `--full-editor`, `--diff`,
  `--dev-container`, and `--demo-workroom` are deleted from the argument
  parser. For one release a stale invocation gets a one-line startup error
  naming the removal; after that it is a plain unknown argument. `--zero-base`
  stays accepted-and-ignored. `is_active()` is constant `true`, the crate's
  entry point and `ENTERED` static are gone, and the refusal sentence names no
  flag because there is no editor to start. The check now enforces absence:
  a revived selector, entry point, or flag literal fails it.
- **The primary presentation is also flag-free.** A later scaffold left its
  stricter presentation behind `OMEGA_PRIMARY_INTERFACE_BUILD` or the hidden
  `--primary-interface` argument. That made a plain `cargo build` produce a
  binary that opened the transitional presentation even though the mode split
  was documented as removed. Startup now selects the primary presentation
  immediately after parsing arguments. The hidden argument remains accepted
  for old launchers but does not decide anything, and the delta check rejects
  any return of the build-time marker.

- **Upstream Zed:** starting the binary with no arguments opens the editor. There
  is no mode, so there is nothing to leave.
- **Omega, before this:** `omega` opened the editor and `omega --zero-base`
  opened one Exo thread. A control on the status bar read "Zero base" beside a
  button reading "Leave zero base", and pressing it put the editor back in the
  running window.
- **Omega during the transition:** `omega` opens the product surface.
  `omega --full-editor` opens the legacy editor compatibility path.
  A process that starts in zero base stays in zero base until it exits: the
  status-bar control, the `Leave` action, and the runtime unwind behind them are
  removed. The owner asked for exactly this: *"remove the 'zero base / leave zero
  base' buttons. they must be stuck in zero base with no way out if it was
  started in this mode. which must be the default starting now. booting the full
  editor must require a separate flag."*
- **The reader did not move.** `OMEGA-DELTA-0047` says the mode is read from the
  parsed process command line, once, and never from a settings key. That is
  unchanged. What changed is the direction of the default when nobody says
  anything, and a default decided in the argument parser is still a decision
  made from the command line.
- **`--zero-base` is accepted and does nothing.** It asks for what it already
  gets. Keeping it means commands, scripts and muscle memory that carry it keep
  working instead of failing on an unknown argument.
- **Editor-only options do not imply the editor.** A diff pair,
  `--dev-container`, and `--demo-workroom` each require `--full-editor`.
  Without it clap refuses the command and names the missing prerequisite.
  Zero base does not silently accept a surface it cannot draw, and those
  options do not silently choose a different mode.
- **Amended by `OMEGA-DELTA-0116`: a path argument used to imply the editor, and
  does not.** The reasoning was that `omega src/main.rs` opening a single chat
  thread with no way to reach that file would be a regression rather than a
  subtraction. The owner overruled it, and the reason he was right is in this
  entry's own sentence: *"booting the full editor must require a separate
  flag."* A positional path is not a flag. It was the one term on the list
  nobody types on purpose, and it made `omega <directory>` — the most ordinary
  command there is — the command-line twin of the way out this delta had just
  removed from inside the app. The mode decision now reads only
  `--full-editor`; `OMEGA-DELTA-0116` checks that path and editor-only arguments
  cannot become alternate selectors.
- **Absent, not unrendered — and this is the part that needed the check.** The
  cheap version of this change is one `when(false)` that hides the button, and
  it leaves `omega_zero_base::leave` on the crate, the `Leave` action in the
  registry, and the palette restriction still clearable at runtime. That reads
  as removed while remaining one dispatch away, which is the failure
  `OMEGA-DELTA-0048` names about every other hidden surface. So the way out is
  deleted: no `leave`, no `LEFT` static, no `LEAVE_LABEL`, no `BANNER_LABEL`, no
  status item, no `clear_restriction`, no `clear_action_gate`, and the
  `omega_zero_base` namespace is out of the admitted set because the one action
  it carried is gone.
- **Nothing else was deleted.** `OMEGA-DELTA-0048` still holds: no shipped keymap
  binding is removed, and `keymaps_name_no_deleted_action` stays green. `Leave`
  was Omega's own action with no shipped binding, which is why removing it does
  not reach the failure that killed `0.2.0-rc6`.
- **The refusal sentence changed with it.** It used to name two ways out — the
  control in the window, and starting Omega without the flag. It now names
  `--full-editor` only. A refusal that offered a button that does not exist would
  send a person looking for it.
- **The visual runner is unaffected by the new default, and says so.** It is a
  separate binary with its own `main`; it never parses `Args`, so nothing turns
  the mode on in that process except its own explicit call before the two
  zero-base scenes. The ordinary-surface baselines therefore still photograph
  something that happens — what a person sees with `--full-editor`. The runner no
  longer installs a status-bar control, and each capture now asserts that the
  mode's state matches the surface the scene declares, so a scene recorded in the
  wrong order fails instead of filing a subtracted window under an ordinary name.
- **Enforced by:** `the_flag_free_surface_has_no_runtime_switch_to_the_legacy_editor` in
  `crates/omega_deltas/`, the mode's own tests in `crates/omega_zero_base/`, and
  the transitional
  `the_transitional_shell_hides_by_filter_and_refusal_until_subtraction_lands`.
- **What this does not cover.** This says nothing about what zero base *shows*.
  The composer, the transcript and the executor line are `OMEGA-DELTA-0049` and
  `OMEGA-DELTA-0051`. It also does not make the mode persistent: it is still
  never written to disk, so ending the process is still a complete repair, and
  `omega --full-editor` remains available during the alpha transition.

### OMEGA-DELTA-0053 — The sealed layout becomes the one application layout

- **Amended by the 2026-07-29 single-experience direction.** The seal is a
  transition mechanism around a still-compiled editor, not a permanent launch
  mode. Post-alpha subtraction collapses the sealed render into the ordinary
  render. Until then, the structural seal remains checked so the advertised
  surface cannot reveal the legacy editor accidentally.
- **Amended by omega#161: the seal moved to process start.** The ordering this
  entry defends below — “the seal is later than the mode” — protected
  `OMEGA-DELTA-0040`'s centre-pane identity onboarding, and omega#164 deleted
  that page: identity is provisioned silently in the background and nothing
  renders in the centre before the thread, so there is no dead end left for a
  startup seal to create. `crates/omega/src/main.rs` now calls
  `omega_zero_base::seal()` once, before `app.run`, so the editor chrome is
  never drawn — not even for a frame — and `initialize_panels` no longer
  seals. `seal()` lost its mode guard with the mode, and `is_sealed()` reads
  the one static. Test processes and proof harnesses still start unsealed and
  opt in per scene. The structural render sites in `workspace` and
  `title_bar` are unchanged and still checked; they become the ordinary
  render's own shape when omega#162 deletes the legacy layout.

- **Upstream Zed:** the workspace is an editor. Panels sit in docks around a
  centre pane group, and a zoomed panel is drawn over that group rather than
  instead of it.
- **Omega, before this:** zero base used the zoom. `initialize_panels` opened the
  agent panel, called `set_zoomed(true, ...)` and focused it, and the comment in
  `crates/omega/src/zed.rs` said what that bought: "Zooming is what takes the
  editor pane and the tab bar off the screen". It was one control away from
  being false. The owner pressed the sidebar toggle and the zoom was released,
  and Zed's whole welcome surface appeared — "Welcome back to Omega / Your last
  IDE", with New File, Open Project, Clone Repository, Open Command Palette,
  Open Settings, Customize Keymaps, Explore Extensions and Open Agent Panel. His
  words were "wtf is that".
- **Why the action gate did not catch it.** The gate refuses *actions*. The
  control that did this is an ordinary click listener on the title bar that
  calls a workspace method, so nothing was dispatched and nothing was refused.
  A gate over actions cannot cover a surface that is merely covered.
- **Omega during the transition:** once zero base is **sealed**, the workspace renders no centre
  pane group, tab bar, inherited title-bar controls, or status bar. The titlebar
  view reduces to Zed's platform drag strip so the window remains movable and
  retains native double-click behavior without restoring any of the controls
  the action gate cannot see. `dismiss_zoomed_items_to_reveal`, the function the
  sidebar control reached, returns early in a sealed zero base — it used to
  close every dock that was not the one being revealed, which in this mode is
  the one panel the window has.
- **The seal is later than the mode, and that is load-bearing.**
  `OMEGA-DELTA-0040`'s identity onboarding is a centre-pane item. A window with
  no centre pane could never show it, so a mode that sealed at startup would
  leave a fresh profile with nowhere to answer the identity gate — the same
  shape of dead end `OMEGA-DELTA-0051` repaired, and worse, because it would be
  unreachable rather than unanswerable. So the ordinary workspace renders until
  the identity gate is answered and the thread is open, and
  `initialize_panels` seals exactly once, at that point.
- **Removing the button alone would have been dishonest.** `OMEGA-DELTA-0052`
  removed the way out. Doing that while the editor still sat one un-zoom away
  would have produced a mode that looks sealed and is not, which is worse than a
  leak a person can see and name.
- **The status bar goes with it, and that answers a second complaint.** The owner
  hovered the status bar's bottom-left icon, read "Close Left Dock ⌘B", pressed
  it, and nothing happened: `workspace::ToggleLeftDock` is outside the admitted
  set, so the gate refused it. A control that is drawn and denied is the same
  "looks one way, is another" failure as the zoom, pointing the other way — it
  looks available and is not. The general rule is that **if the gate refuses an
  action, its control must not be drawn**, and not rendering the status bar in a
  sealed zero base is how every control on it obeys that rule at once, including
  ones a later crate adds.
- **`--full-editor` was untouched while it existed.** The seal was gated on
  the mode being active, so a build started with the flag could not be sealed
  by a stray call. omega#161 removed the flag and the guard together: there is
  no unsealed shipped surface left to protect.
- **Enforced by:** `the_transitional_sealed_layout_starts_without_the_legacy_editor` in
  `crates/omega_deltas/`, which pins the seal's render sites, the early
  return in the reveal path, and the single startup seal call before
  `app.run` in `crates/omega/src/main.rs`.
- **What this does not cover.** This is about what the workspace draws. It says
  nothing about what the thread surface draws inside it — that is
  `OMEGA-DELTA-0049` and `OMEGA-DELTA-0051` — and it does not claim the admitted
  action set is correct, which is `OMEGA-DELTA-0048`'s job. It also cannot be
  checked by a compiler: the seal is reached only in a process that started in
  zero base and answered the identity gate, so `cargo check`, `cargo test` and
  clippy say nothing about whether the window looks right. Only opening it does.

### OMEGA-DELTA-0054 — Zero base opens the directory it was started in, or says it opened none

- **Upstream Zed:** a workspace with no paths has no worktrees, and that is
  correct for an editor. A person opens a folder when they want one.
- **Omega, before this:** zero base opened no project, and
  `crates/omega/src/zed.rs` said so in its own comment — "no project is opened, so
  there is no buffer for them to show". The missing buffer was not the problem.
  The workspace had no worktrees, so `grep`, `find_path`, `list_directory`,
  `read_file` and `terminal` all had nothing to operate on. The owner ran
  several searches, every one returned no matches, and the agent concluded that
  the workspace appeared to be empty. That is literally correct about the
  workspace and completely useless about his code. His words: *"grep is finding
  no files why???????? what the fuck is the working dir"*.
- **Omega now:** in zero base, a working directory somebody chose is opened as
  the project. `crates/omega_workdir/` decides what "somebody chose" means, and
  when the answer is no the composer asks for one in a single line, in the
  ordinary text colour, beside a control that opens the folder picker. It asks
  rather than warns, and the header control beside the thread title reads
  `Choose a folder` for the same reason: the owner installed a candidate,
  opened it, and met two yellow notices before he had done anything wrong. A
  warning for the state every new window starts in is how a person learns to
  read past warnings. There is no setup page: nothing is asked before the
  thread opens.
- **Why the test is plausibility and not project-ness.** Requiring a marker — a
  `.git`, a `Cargo.toml` — would refuse a plain folder of files, which is a
  legitimate thing to point an agent at and the case a person is most likely in.
  So the rule runs the other way: reject the directories a launcher hands over
  and accept the rest. What is rejected is the filesystem root (which is what
  Finder and the Dock give), the home directory itself (opening it means
  scanning everything a person owns to answer one question), a relative path, a
  path that is not a directory, and the bundle and system prefixes in
  `LAUNCHER_PREFIXES`. A directory under `$HOME` is accepted, because almost
  every real checkout is one.
- **Only in zero base — until omega#161 removed the other surface.** While
  `--full-editor` existed it opened an empty workspace exactly as upstream did,
  and this behaviour was guarded to the mode. The guard is gone with the flag:
  there is one surface, and it always opens the chosen working directory or
  says it opened none.
- **The composer line changed with it.** It used to read "No AI provider
  configured — leave zero base to add one", and the zero-base baselines caught
  it rendered directly beneath a turn that had just completed through
  `exo/basic`. It was true of *model providers* and false of what the person had
  just watched happen, and it offered a way out of a mode `OMEGA-DELTA-0052` has
  since removed. The line now asks whether anything on the machine can run a
  turn — `omega_agent_detect` reading `PATH` as well as the provider registry —
  and, when something can, whether the thread has a folder. Only one is ever
  shown, executor first, because a thread with no executor cannot use a folder.
- **The Open Folder control is admitted, deliberately.** `workspace::Open` is in
  the admitted set, because a control that is drawn and refused at dispatch is
  the same "looks one way, is another" failure as the hidden editor, pointing
  the other way. `workspace::OpenFiles` and the rest of that namespace stay
  refused: choosing what the thread can see is one thing, and opening the
  editor's file surfaces inside a mode that does not render them is another.
- **Why no test saw this.** The visual baselines photograph a workspace the
  runner builds itself, and the runner hands it a lane path. The scenes had a
  project while the shipped launch path did not. That is the same class of gap
  `OMEGA-DELTA-0049` already records about `install_on_workspace` — a proof that
  reaches a surface without reaching the path that builds it — and it has now
  cost something twice.
- **Enforced by:** `zero_base_opens_the_directory_it_was_started_in` in
  `crates/omega_deltas/`, and the seven unit tests in `crates/omega_workdir/`.
- **The check's spelling changed with `OMEGA-DELTA-0093`, and its property did
  not.** It used to require the literal text
  `if open_zero_base_project(&app_state, cx).await {`. The driven send has to
  run after *either* branch of the startup path rather than only after the
  fallback, which meant negating that condition, so the check now asserts the
  order directly: the project attempt comes first and the empty workspace is
  what happens when it fails. Recorded here rather than silently adjusted.
- **What this does not cover.** It does not decide which executor the turn runs
  on, which is `OMEGA-DELTA-0049`'s and the routing delta's business. It does
  not persist the choice — a folder chosen with the control lasts as long as the
  workspace does, and restoring a previous session already restores its paths
  ahead of this. And it cannot be checked by a compiler: the path runs only in a
  process that started in zero base with no restorable workspace.

### OMEGA-DELTA-0055 — Routing is decided, not selected

- **Upstream Zed:** the agent a thread talks to is chosen from a picker, and the
  picker names products a person recognises.
- **Omega, before this:** the composer row carried a control reading
  `pin: none ⌄`. Opening it offered "Pin this thread's executor" over
  `native_loop`, `external_acp`, `engine_lane` and "Unpin". Those are the
  router's wire tokens. `ExecutorClass::token`'s own documentation says the
  token is "never shown to a user on its own", and this control was doing
  exactly that. The owner: *"that UI selector makes no sense, i have no fucking
  clue what youre talking about so the user won't, remove that UI piece and
  handle it smartly in the background"*.
- **Omega now:** the wire-token pin control is removed. The 2026-07-29
  amendment to `OMEGA-DELTA-0150` restores human-readable mode and executor
  choice at conversation creation; it does not restore a picker exposing
  `native_loop`, `external_acp`, or `engine_lane` tokens.
- **Owner gate 8 is untouched, and the distinction is why this is admissible.**
  The gate forbids any model-initiated path from starting Full Auto authority.
  An engine lane *is* that authority: the engine-lane arm of `route` is
  unchanged and still requires a pin, and an unpinned thread still never reaches
  one. An external ACP agent is not that authority — `omega_exo_lane`'s module
  docs choose `ExternalAcp` for Exo precisely *because* it is neither the
  first-party claim nor Full Auto. It is also not model-initiated: nothing a
  turn can say attaches an external agent, because that connection is made at
  startup from what is installed on the machine. `PinGesture` still has exactly
  two variants and nothing new calls `pin_session`.
- **Old records remain readable.** `DetectedExternalAcp` remains in the durable
  reason vocabulary because older sessions may carry it. Creation-time choice
  changes policy, not the decoder for durable route history.
- **What the disclosure line says now.** It still names who ran the turn and on
  what model, because a person is entitled to know which runtime and model spent
  their budget — the owner objected to the *selector*, not to being told. The
  `routed: unpinned` fragment is the same jargon as the control and is gone with
  it.
- **Three existing checks asserted the old policy and were changed, not
  weakened.** `OMEGA-DELTA-0021`'s composer-bar check required the bar to carry
  the pin and now requires it not to; `OMEGA-DELTA-0035`'s wiring check required
  the pin control to be rendered and now requires the automatic arm to exist in
  its place; `OMEGA-DELTA-0049` dropped the pin from the list of things the
  thread surface must draw. Each is recorded in its own entry, with the reason.
  Two behaviour tests changed for the same reason:
  `an_unpinned_thread_never_reaches_an_engine_lane` and
  `a_served_session_can_never_reach_an_engine_lane` both used to assert
  `NativeLoop`, and both now assert the property they are named for — not an
  engine lane — with the native answer kept for the case where nothing is
  attached.
- **Enforced by:** `an_unpinned_thread_never_reaches_an_engine_lane` in
  `crates/omega_deltas/`, and
  `an_unpinned_thread_runs_on_the_external_agent_that_is_attached` in
  `crates/omega_front_door/`.
- **What this does not cover.** It does not attach anything. A thread reaches
  Exo automatically only when an Exo lane is connected, which today still needs
  the lane file `OMEGA-DELTA-0042` describes; deriving that lane from what
  detection found is omega#100's remaining deliverable and is not in this delta.
  It also says nothing about thread titles, which are generated by the
  summarization model rather than the thread's executor — a second model
  spending budget on a thread routed elsewhere. The owner was asked and accepted
  it directly: *"i am ok with gemini flash being used for thread titles to api
  key"*.
### OMEGA-DELTA-0060 — A session ID can read a live or persisted thread transcript

- **Upstream behaviour.** `spawn_agent` returns the subagent's final message and
  nothing else. `SubagentSessionInfo` is attached to the tool call for the UI,
  and the `From<SpawnAgentToolOutput>` conversion drops it with the comment
  "Don't show this to the model". The parent receives `{session_id, output}`.
  There is no tool that turns that `session_id` into the work behind it.
- **Why Omega diverges.** A parent delegates a task and gets one paragraph. If
  that paragraph is thin, or wrong, or contradicts something the parent knows,
  the parent cannot look at what the subagent did. It can only delegate again
  and hope for a better paragraph. The information exists — `Thread` keeps its
  messages, and `SubagentContext` records the thread that spawned it — but
  nothing exposes it. `read_subagent_transcript` is the compatibility-named
  reader, and the basic `read` dispatcher routes session addresses to it.
- **The law: an explicit Omega session ID is readable.** `read` accepts
  `thread:`, `session:`, `agent:`, and `delegate:` addresses. The environment
  first resolves the target from `NativeAgent.sessions`, which permits the
  calling thread itself plus open top-level, sibling, parent, and subagent
  threads. If the session is not open, it loads the `DbThread` asynchronously
  from `ThreadStore` and renders the same stored messages without opening the
  thread in the UI. Live external ACP transcripts remain process-local and are
  available only while the spawning environment retains their handle.
- **Live state wins over persisted state.** An open thread may have messages
  newer than its last save, so lookup must consult `NativeAgent.sessions`
  before `ThreadStore`. Persisted messages use the same flattening function as
  live `Thread` messages so detail modes, message indexes, tool blocks, and
  pagination do not drift between the two paths.
- **The law: a bound that fires must be visible.** The reason the subagent's
  work is not in the parent's context is that it costs too much to put there,
  so an unbounded read hands that cost straight back and can exhaust the
  parent's window in one call. Three bounds apply, and each announces itself.
  - A message window. `offset` and `limit`, defaulting to 20 messages and
    capped at 100. The header always states the range returned and the total,
    so paging is a decision made with the total in hand.
  - A per-block clip, 200 bytes in `outline` detail and 2000 in `full`, marked
    with the bytes shown and the bytes there were.
  - A whole-response cap of 24000 bytes. When it fires, the rendering stops and
    names the messages it dropped and the `offset` to resume from. Room for
    that marker is reserved out of the cap, so the cap can never be the reason
    the reader is not told about the cap.
  - Silence is the failure mode being designed against. A reader who cannot see
    the cut concludes the subagent never did the thing and re-delegates work
    that was already done.
- **`outline` is the default, and the description says so.** The description
  tells the model that the final message from `spawn_agent` is usually enough
  and that this is for checking delegated work, not a routine follow-up. A
  cheap default matters more than the sentence: `outline` lists tool calls,
  input, and result sizes, and it does not spend the parent's context on the
  subagent's thinking text.
- **What this does not cover.** Nothing here reads a rendered pixel; the tool
  produces text for the model and the panel, and no check confirms how a
  transcript looks on screen. External ACP agents do not have an Omega
  `DbThread`, so their transcripts cannot be restored after the retaining
  process exits. Image content becomes a marker rather than being carried. And
  the byte caps are counts of bytes, not tokens: they bound the cost, they do
  not measure it.
- **Enforced by:** `a_session_id_reads_a_live_or_persisted_thread`,
  `a_truncated_transcript_says_that_it_was_truncated`, and
  `the_transcript_tool_reaches_the_model` in `crates/omega_deltas`; plus the
  renderer suite in `crates/agent/src/tools/read_subagent_transcript_tool.rs`
  and live/self/persisted `read` integration tests in `crates/agent/src/agent.rs`.

### OMEGA-DELTA-0070 — A public Nostr chat skill is in every install

- **Upstream Zed:** one skill is compiled into the binary, `create-skill`.
  Every other skill is a file the user puts in `~/.agents/skills/` or in
  `{project}/.agents/skills/`. An agent that must speak on a public NIP-29
  relay therefore starts with no procedure, and each user writes their own.
- **Omega now:** `public-nostr-chat` is compiled in. Omega Agent has it with no
  skill directory, no project, and no network. The file is
  `crates/agent_skills/builtin/public-nostr-chat/SKILL.md`, and it is an exact
  copy of the skill the owner wrote in the OpenAgents repository on
  2026-07-25. The text is unchanged, including its frontmatter.
- **The copy is deliberate, and a shared path is not permitted.** Omega must
  build and run from its own tree. A path into a sibling repository is a build
  dependency on a checkout that a user does not have and an installed
  application cannot read.
- **The precedence is unchanged, and this is the point.** `SkillSource::BuiltIn`
  stays at precedence `0`, so a global or project-local skill with the same name
  continues to shadow it. A default that a person cannot replace is a removal of
  their control, not an addition of a capability.
- **The registration table is now the only list.** `builtin_skills` read the
  name `create-skill` directly while `BUILTIN_SKILL_ENTRIES` was a second list
  used only to serve bodies. With one skill the two lists could not disagree, so
  nothing showed that the loader did not read the table. The loader now reads
  the table. A skill added to the table alone used to get a body that no catalog
  entry pointed at.
- **A built-in that does not parse is recorded.** The loader dropped a parse
  failure silently. The content is compiled in, so a failure is a defect in the
  shipped binary, and a skill that is silently absent is the hardest form of it
  to find. The failure is logged with the name of the skill.
- **The skill keeps host names out of protocol code.** Relay URL, group
  identifier, accepted kinds, and limits are configuration that an operator or a
  public manifest supplies. Omega does not put an OpenAgents host name or group
  identifier into the client, and the same procedure operates a different
  NIP-29 relay and group.
- **Enforced by:** `a_public_nostr_chat_skill_ships_in_the_binary` in
  `crates/omega_deltas/`, and `public_nostr_chat_is_built_in` plus
  `every_builtin_entry_loads_through_the_loader` in `crates/agent_skills/`. The
  first reads the shipped file and the registration. The other two run the real
  loader and assert that the skill is in the catalog with source `BuiltIn`.
- **What this does not cover.** It adds a procedure, not a transport. Omega does
  not sign, connect, or publish anything because this skill is present. The
  signer, the relay, and the group stay operator-selected, and the skill refuses
  a shared key and an implicit machine key.

### OMEGA-DELTA-0080 — A tool result opens at a ceiling, and says what it hides

- **Upstream Zed:** a terminal tool call renders its result body at the natural
  height of the output. The only ceiling is
  `TerminalView::MAX_EMBEDDED_LINES`, 1,000 lines, above which the body becomes
  a scroll region of `h_72`. A forty-line result is therefore forty lines tall.
- **Omega:** the body opens at 16 lines. A control below it states the count of
  lines it hides, and a click lifts the ceiling. A second click puts it back.
- **Why:** owner direction, 2026-07-26, on a thread that ran the
  `public-nostr-chat` skill: *"those tool calls are not looking so good — the
  huge json blobs shouldnt be full height automatically like that, maybe make
  them smaller default but then expandable"*. The thread ran several `nak`
  commands, and each one returned raw Nostr events. One result was five events,
  about forty lines of hexadecimal identifiers and signatures. Several such
  bodies in one turn push the reasoning that produced them off the screen, and
  the one useful line — `publishing to relay.openagents.com... success.` — sits
  under a wall of signature text.
- **The ceiling is 16 lines, because the tree already treats 16 lines as a
  bounded body.** The scrollable fallback for a result over 1,000 lines is
  `h_72`: 18 rem, 288 px, which is about 16 lines at the agent panel's text
  size. A capped result is therefore the same size as a result that upstream
  already capped, and it costs about a fifth of a full-height window. A
  two-line result is under the ceiling, so nothing is cut and no control is
  drawn.
- **The last lines stay, not the first.** A terminal keeps its viewport at the
  bottom, so a body given fewer lines than it holds shows the most recent ones.
  This is also the correct end for a command: the first lines are the setup and
  the last lines are the result. `publishing... success.` was the last line of
  the body the owner was looking at.
- **The control names a count.** `tool_output_ceiling_label` returns
  `Show 24 more lines`, not `Show more`. A reader decides whether to spend the
  screen, and `Show more` gives that reader nothing to decide with. The count
  is the difference between the lines the body holds and the lines it shows, so
  it is exact rather than estimated.
- **The ceiling wins over the scrollable fallback.** `content_mode` checks for a
  ceiling before it compares against `MAX_EMBEDDED_LINES`. Without that order,
  the longest results are the ones that escape the ceiling.
- **The open state survives streaming.** The state is the ceiling itself, held
  on the `TerminalView` that `EntryViewState` builds once per terminal and
  reuses for every frame. There is no second record to drift from it, and a
  body a reader opened does not shut while the turn continues. It has the same
  lifetime as the terminal's own scroll position.
- **The command stays visible.** The ceiling applies to the result body. The
  card header, which carries the command, the working directory, the elapsed
  time, and the failure state, is untouched.
- **Focus does not lift the ceiling.** The upstream field beside it,
  `max_lines_when_unfocused`, is removed by a click into the terminal. A
  ceiling that a click for a text selection removes is not a ceiling the reader
  controls, so the new field is independent of focus.
- **The control sits outside the body it describes.** `OMEGA-DELTA-0060`'s rule
  is that a bound cannot be the reason the reader is not told about the bound.
  A control drawn inside the capped region is cut by the ceiling it announces,
  so the control is a sibling of the body, in the card.
- **This is the view, and omega#105 is the record. They are independent, and
  both are needed.** That issue makes a tool result a versioned artifact and
  puts a bounded preview plus a truncation marker in the event, so the model's
  context and a transcript reader stop carrying the whole blob. It does not
  replace this delta: a preview is sized for a context budget, and a budget of
  a few thousand bytes is still about forty lines of Nostr JSON, which is the
  height the owner objected to. A ceiling of 16 lines is smaller than any
  preview that issue would pick.
- **The seam between them is one function.** `tool_output_ceiling_label` forms
  the whole sentence, and nothing else does. After omega#105 the body may
  itself be a preview, so the line count this delta reads would be the
  preview's, and the label would state a total that is not the total — a
  reader who lifts the ceiling and reaches the last line would conclude they
  had the whole result. The repair is a third input to that one function, the
  amount the record withheld, not a second sentence elsewhere. The card already
  keeps the two levels apart: the header carries the existing notice for what
  the *agent* received, and this control carries what the *reader* can see.

- **Enforced by:** `a_tool_result_opens_at_a_ceiling_the_reader_can_lift` in
  `crates/omega_deltas/`, which pins the value, the application at
  construction, the order against the fallback, and the control. The arithmetic
  and the label run through their real functions in
  `embedded_ceiling_only_binds_on_a_long_result`
  (`crates/terminal_view/`) and `test_tool_output_ceiling_label`
  (`crates/agent_ui/`).
- **What this does not cover.** No check reads a rendered pixel, so the height
  in points, the position of the control, and the text that a person sees are
  unverified here. The ceiling counts terminal lines, not wrapped display rows,
  so a body of long lines shows fewer than 16 rows of content. It applies to a
  terminal tool result only: a tool that returns Markdown, a diff, or an image
  keeps the height it had. And the terminal is resized to the ceiling, so a
  command that reads the terminal height sees 16 rows while the ceiling holds.

### OMEGA-DELTA-0090 — A check is falsified in a forked episode, never in the working tree

- **Upstream behaviour.** Zed has no notion of an episode. Omega's delta
  discipline — watch a check fail before you trust it — is therefore performed
  by hand: edit a file, run the check, revert. On 2026-07-26 that loop ran about
  fifteen times across four lanes and cost real damage. One lane reverted with
  `git checkout --` and wiped uncommitted work in two files; it recovered, and
  changed its script to restore from a byte copy. One check "passed" while
  testing nothing, because the mutated string also appeared in a second code
  path, so deleting the intended arm left it green. One suite flaked on a model
  declining to call a tool, and there was no way to re-run from an identical
  start, so the answer was to run it again and hope.
- **Why Omega diverges.** Every one of those is an episode-reset problem, and
  the Exo lane (`OMEGA-DELTA-0042`) already puts a harness with the primitive
  inside reach. `conversation_fork` replays a conversation's whole event log at
  a chosen event into a *new* conversation, leaving the source untouched. A
  mutation applied in a fork is applied to a copy, so a failed revert cannot
  destroy anything — there is nothing to revert.
- **The law.** `crates/omega_exo_episode` is a leaf, in the shape
  `omega_exo_lane` established: no GPUI, no process, no filesystem, no clock, no
  network. It has five parts, and each one is a refusal rather than a
  convention.
  - **The working tree is unreachable, not merely untouched.** The crate names
    no path type, no `std::fs`, no `std::process`, and no `Command`. The reason
    a forked episode cannot wipe uncommitted work is not that it is careful; it
    is that nothing in it can reach a file.
  - **Query and fork families only.** `exo serve` is unauthenticated by design
    and answers all 52 request variants on one endpoint, `get_secret` and
    `delete_agent` included. `omega_exo_episode::family` partitions the whole
    protocol rather than allowlisting the calls this crate happens to make: an
    allowlist is silent about a 53rd variant nobody classified, and a partition
    fails on it. Two of the 52 are admitted beyond reading — `conversation_fork`
    and `start_sandbox` — and each is its own family, so nobody reads "queries
    only" and believes an episode leaves Exo's storage untouched. It does not:
    it adds one conversation, which is Exo's own record of the fork it was asked
    to make.
  - **The fork point is required.** Upstream's `up_to_inclusive` is optional and
    `None` means "the whole history". `EpisodeRequest::ForkAtEvent` takes an
    event id and cannot say `None`, because forking after the mutation puts the
    mutation in the sibling and a fork at "now" is that same mistake by
    omission.
  - **The reset is aimed at the fork, by type.** `RestoreSandbox` takes a
    `ForkedConversation`, whose only constructor reads a fork response. A
    restore cannot be pointed at the conversation the episode forked *from*.
  - **One writer per root.** `.exo` is single-writer storage with one in-process
    mutex; two processes on one root interleave rather than conflict, which
    would produce a fork of a history that never existed while every check still
    passed. `ExoRoots` refuses a second claim, is not `Clone`, and refuses any
    path spelling that could alias another.
- **Two forks are compared, not asserted.** They are not byte-identical and
  cannot be: `fork` re-mints every event id, sets the fork's own
  `conversation_id`, and recomputes `created_at`. Those three fields are the
  identity of the copy; everything else — `session_id`, `turn_id`, the whole
  payload — is preserved verbatim and is content. `EpisodeState` removes exactly
  those three and compares the rest, with a digest for "are these the same" and
  a diff for "where did they stop". The dangerous direction is growth: an
  exclusion set that grew would make more episodes compare equal, so the set is
  asserted to be those three and nothing else.
- **The probe comes before the check.** `verdict` is total over (probe, check
  outcome) and has three answers, not two. A mutation that applied against a
  check that passed is `CheckDidNotNotice` — the check tests something else. A
  mutation that never applied is `MutationDidNotApply` whatever the check said —
  a failure of the loop, not of the check, and a different file to go read.
  Folding those together is how a loop sends somebody to the wrong place.
- **The filesystem half does not compose yet, and the crate says so.** omega#103
  and `docs/teardowns/2026-07-25-exoharness-exo-teardown.md` §11.5 both state
  that fork plus `start_sandbox { snapshot_id }` is a complete episode reset
  needing no upstream change. The conversation half is exactly that. The
  filesystem half is not, at the pinned Exo: `fork` copies four prefixes —
  `bindings`, `secrets`, `artifacts`, `sandboxes` — and `snapshots` is not one
  of them, so a conversation-scoped snapshot taken before the fork does not
  exist inside it and `start_sandbox` fails loading the manifest. An
  agent-scoped snapshot is reachable and is one sandbox record shared by every
  conversation of the agent, so two siblings restoring it share a filesystem and
  are not two episodes. `admit_filesystem_reset` is total over (scope, shape)
  and issues its witness for one combination — agent scope, one episode at a
  time — naming the reason for each refusal. Exo's own documentation agrees:
  snapshots time-travel a sandbox "without forking the conversation itself" and
  are "not a conversation rewind". The fix is one more `copy_prefix` beside the
  four that are already there, additive and in upstream's own direction, and it
  is owner-gated like every other upstream contribution.
- **What this does not cover.** This is the law, not the client: nothing here
  opens a socket, so no episode has been run end to end against a live `exo
  serve` on this machine. The shapes and the fork finding are read from the
  pinned source — `crates/exoharness/src/{protocol,basic,types}.rs`,
  `http/server.rs`, and `docs/sandbox-snapshots.md`, which are byte-identical
  between upstream `baa07f67` where the teardown read them and the
  `OpenAgentsInc/exo` fork `cd7c0d29` the lane drives — and they are stated as
  values so a live run can contradict them loudly. The mutation itself stays
  where it already lives: a turn sent by `omega_exo_lane::ExoCommand::SendTurn`,
  the lane's one write. Discarding a fork is Exo's `delete_conversation`, a
  write family call this crate does not make, so a discarded fork leaves Exo's
  own record of it behind — Omega starts no sandbox, spawns no process, and
  makes no other `.exo` mutation. And self-modification stays out entirely:
  `guardian_action`, agent-authored tools, and the read-write source mount are
  refused by `omega_exo_lane::capability`, and nothing here relaxes,
  re-implements, or routes around that gate.
- **Enforced by:** `the_episode_crate_cannot_reach_the_working_tree`,
  `an_episode_sends_no_write_or_secret_request`,
  `the_episode_comparison_ignores_only_what_a_fork_rewrites`,
  `the_falsification_loop_forks_first_and_probes_before_it_checks`, and
  `the_episode_reset_records_that_a_fork_does_not_carry_snapshots` in
  `crates/omega_deltas`; plus the 42 unit tests in
  `crates/omega_exo_episode/src/`.

### OMEGA-DELTA-0091 — Omega reads Exo's durable log, and can name nothing else

- **Upstream Zed:** no Exo, no exoharness, no external agent whose durable
  record Zed reads. There is no upstream behaviour to revert to here; the
  divergence is that Omega talks to a second agent runtime at all, and this
  entry records the shape of that conversation rather than a changed default.
- **Omega before this change:** Omega attached to Exo over ACP
  (`OMEGA-DELTA-0042`, omega#87) and saw the live turn — text, tool calls, tool
  results as ACP framed them, one completion record. Beside that stream sat
  Exo's actual record, a durable replayable event log with versioned artifacts
  and sandbox snapshots, and Omega read none of it. On 2026-07-26 a
  `read_subagent_transcript` tool was built here from scratch
  (`OMEGA-DELTA-0060`) because a parent thread could see only a subagent's final
  message. That tool is right for Omega's own native subagents. For an
  Exo-backed thread it was a second, weaker record beside a complete one, on a
  socket Omega already talks to.
- **Omega now:** `crates/omega_exo_log` is a read-only client for `exo serve`.
  An Exo thread's events can be read after the turn ends, an artifact an event
  references resolves, and a non-loopback endpoint is refused with a reason.
- **The wrong call cannot be expressed, rather than being refused.** `exo serve`
  answers **52** request variants — counted off `Request::kind` at the pin
  `omega_exo_lane::EXO_PIN`, not quoted. `OMEGA-DELTA-0102` moved that
  enumeration into `omega_exo_lane::ExoRequestKind`, where it is written once;
  this crate's `admission::is_admitted_read` is one of the two decisions over
  it. `ExoQuery` is closed at eight of them:
  `get_agent`, `get_conversation`, `conversation_get_events`,
  `conversation_get_event`, and the four artifact list/read variants. There is
  no `from_kind(&str)`, no public wire-string constructor, and every reader
  takes an `ExoQuery`. A caller that wants `conversation_fork` has to add a
  variant to a file whose diff a reviewer reads. The check is a partition
  rather than a denylist, and it reads the two halves in different places for a
  reason found by falsifying it: the negative half scans every source file in
  the crate for the forty-four, and the positive half reads *only* the closed
  type's variant-to-kind map. An earlier version scanned the whole crate for the
  eight as well, which the crate's own published table satisfies on its own — so
  a variant could stop sending a read and the check would still pass, reading
  the list instead of the code. Whole string literals, not substrings: Exo's
  event tag `conversation_forked` contains its request kind `conversation_fork`,
  and `conversation_get_event` is a prefix of `conversation_get_events`.
- **Ten of the forty-four are reads, and they are refused anyway.** Exo's
  protocol has eighteen query variants; this client admits eight. The other ten
  — `list_agents`, `list_conversations`, `get_sandbox_process_events`,
  `wait_sandbox_process`, and the six binding list-and-get variants — read, and
  are refused. The issue scoped this client to *a conversation's own record*,
  and a list of every agent on the host is not that. A denylist of writes would
  have admitted all ten without anyone deciding to. The count is not a guess:
  `omega_exo_episode::family` (`OMEGA-DELTA-0090`, omega#103) transcribed the
  same 52 variants independently for a different purpose, and the two
  transcriptions agreed exactly. `OMEGA-DELTA-0102` has since merged the
  *enumeration* into `omega_exo_lane::ExoRequestKind` and left the two
  *decisions* separate, which is the shape both lanes asked for.
- **Loopback is checked twice, because `localhost` is a name.** The address is
  parsed by `LoopbackEndpoint`, so a client value cannot hold a remote host.
  Then the *resolved* socket address is checked again before the connection
  opens: a hosts file or resolver that points `localhost` somewhere else is
  enough to move it, and the parse cannot see that. Exo's CLI refuses a
  non-loopback `--bind`; this is the other half of the same law, on the
  destination.
- **No bearer token, ever.** Exo's HTTP client has `with_bearer_token` and Exo's
  server never reads an `Authorization` header — its own documentation says so.
  Sending one would leave a capture in which the endpoint looks protected. The
  check refuses the vocabulary in code.
- **The artifact read is what carries tool results, and its absence is
  visible.** Exo's event log names artifacts — `artifact_written` carries an id,
  a path, and a version — and never contains their bytes; Exo's own scheduler
  writes a run's whole stdout into an artifact and leaves a preview behind. So a
  history built from events alone keeps every name and loses every body, and
  `ExoBody::NotRead` says which artifact is missing rather than rendering a tool
  result with no body, which reads as a tool that returned nothing. The
  falsifier is run as a test: the same events rendered with and without the
  artifact set produce the same number of rows and different content.
- **Exo-reported usage is typed as unattested.** Exo never makes the model call
  through an attested path and its own cost design document calls the numbers
  "agent-reported telemetry, not an attested ledger". The type is
  `HarnessReportedUsage`, every rendering prints its provenance, and there is
  deliberately no `From` impl out of it — a conversion is how a harness number
  reaches a ledger without anybody deciding it should.
- **An event this build does not know is kept, not dropped.** Exo declares
  itself unstable and writes "do not write fallback code" into its own
  `AGENTS.md`, so it will add variants. An unknown variant becomes a row naming
  its tag. A decoder that failed the page would lose the forty rows around it,
  which is the opposite of reading the durable record.
- **Read-only, and it grants nothing else.** No write, no fork, no snapshot, no
  sandbox, no secret. Forking and snapshotting are omega#103 and are scoped
  there. Secrets stay Exo-owned; the env-var injection paths are Exo's to run.
- **Enforced by:** `the_exo_log_client_can_name_only_the_eight_read_variants`,
  `the_exo_log_client_reaches_exo_only_on_this_machine`,
  `exo_reported_usage_is_never_accounting_truth`, and
  `an_exo_history_without_its_artifacts_says_what_is_missing` in
  `crates/omega_deltas/`; plus the suite in `crates/omega_exo_log/`, which
  includes a real loopback round trip over a real socket with Exo's documented
  HTTP envelope.
- **What this does not cover.** No pixel. This crate is a leaf with no GPUI, for
  the same reason `omega_exo_lane` has none: a law that needs a window to check
  is a law nobody checks. It produces rows and a plain-text rendering, and the
  Exo workspace in `crates/agent_ui/src/omega_exo_connection.rs` is where those
  rows become a surface — one call this entry does not make. Nothing here has
  been run against a live `exo serve`: the wire format is taken from three
  witnesses in the pinned tree (`crates/exoharness/src/protocol.rs`,
  `typescript/harness/runner.ts`, and `docs/exoharness-http.md`) and exercised
  against a loopback server this repository writes, which is agreement with the
  source and not agreement with a running Exo. The transport is blocking
  `std::net`, so a caller runs it off the main thread; nothing here enforces
  that. And paging is the caller's business: the client returns one page and its
  cursor, and never decides how much of somebody's history to hold.
- **The 52 request types were written down twice, and are not any more.** This
  entry and `OMEGA-DELTA-0090` landed independent transcriptions of the same
  `Request::kind` at the same pin. They agreed exactly, which was the argument
  for trusting both and also the argument for keeping one: a 53rd variant
  upstream would have had to be noticed twice. `OMEGA-DELTA-0102` resolved it in
  the direction both lanes named — one enumeration in
  `omega_exo_lane::ExoRequestKind`, two decisions over it, kept separate because
  the episode law's `Query` family is eighteen variants and this client admits
  eight, so a merge would widen this one to include the fork.
### OMEGA-DELTA-0061 — A subagent can be a different executor from its parent, chosen per spawn

- **Upstream behaviour.** `Thread::new_subagent` copies the parent's model, so
  every subagent is the parent wearing the same face. `subagent_model` can
  override it, but it is one global setting for all subagents; it cannot say
  "this one is Codex and that one is Claude". `SpawnAgentToolInput` was
  `{label, message, session_id}` with no field for what should run the work.
- **Why Omega diverges.** Delegating to a copy of yourself is not delegating to
  a second opinion. Codex and Claude Code are not models — they are external ACP
  agents with their own logins, tools and loops — so a parent that can route one
  subagent to Codex and another to Claude gets work done by agents that are
  actually independent of it. `spawn_agent` now takes an optional `executor`, and
  a subagent named `codex-acp` runs a real ACP session against the Codex agent
  server through the same `CustomAgentServer` → `connect` → `new_session` path
  the panel uses, not a second path built beside it.
- **The law: honoured, or refused by name. Never substituted.** Resolution is
  `resolve_subagent_executor`, a pure function of what was asked for, what Omega
  knows, and what is installed. It has two outcomes — `Resolved` and `Refused` —
  and deliberately no third meaning "could not honour this, ran something else".
  - A request for `codex-acp` on a machine without Codex **fails, naming Codex**,
    and says it did not fall back. The alternative is a subagent that reports as
    Codex and is not, which is the same defect class as an undisclosed provider
    handoff: the parent believes an independent agent looked at the problem when
    the same agent looked at it twice.
  - **An unrecognised name is refused, not guessed.** The tempting reading of
    "or a model for the native loop" is that anything which is not an agent id is
    a model name. That makes the typo `codex-acpp` a silent inherit. Per-spawn
    native *model* selection is therefore not accepted at all here — it needs a
    validated model lookup, and adding it as a fallthrough would reintroduce
    exactly the substitution this delta forbids.
  - Matching is by **exact id** against a closed set. No prefix or case
    matching, so `codex` does not become `codex-acp` and one agent's name cannot
    capture another's request.
  - `executor` together with `session_id` is **refused**, not ignored. A resumed
    session runs on whatever created it, so honouring the request is impossible
    and accepting it silently would drop it — the same fallback through a
    different door.
  - Omitting the field still inherits the parent, unchanged. That is the
    compatibility promise, and it is pinned in both directions.
- **The law: presence, not configuration.** The installed set comes from
  `omega_agent_detect`, a `PATH` probe, and never from `AllAgentServersSettings`.
  Settings record what is *configured*, which is a different fact: a fresh
  `--user-data-dir` has no settings written whatever is on disk, so a
  settings-based check offers nothing on exactly the machine a new person is
  using — and offers a missing agent on one whose settings outlived the binary.
  Presence is decided once, before anything is created; the connect path does not
  re-derive it, because two sources for one question eventually disagree.
- **The law: every result names what produced it.** A mixed fan-out the parent
  cannot attribute is not finished. Each result carries an `executor` record in
  the JSON the model reads, and the record is asked of the **handle**, not of the
  request — a record taken from what was asked for would still read "Codex" on a
  subagent that ran as something else, reporting the intention rather than the
  fact. The *shape* of that record is `OMEGA-DELTA-0101`; the first cut of this
  delta reported a hand-written sentence instead, which is the thing
  `OMEGA-DELTA-0021` forbids.
- **What has now run, and what has not.** The live path is no longer
  hypothetical: `crates/agent/src/tests/external_acp_subagent.rs` drives
  `ExternalAcpSubagentHandle` against the real `codex-acp` and `claude-acp`
  adapters — session creation, the prompt, the stream, the stop reason, the
  final message and teardown — and runs a mixed fan-out of two Codex and one
  Claude concurrently. It is `#[gpui::test]` behind the `e2e` feature, because a
  child process on stdio needs no window:
  `cargo test -p agent --features e2e --lib external_acp_subagent`.
  What still has **not** been seen is any of it in a window: the agent panel's
  subagent card looks the session up in the native connection's thread map, an
  external subagent is not in it, and that is `crates/agent_ui`'s to fix.
  Transcripts remain unreadable through `read_subagent_transcript` — the session
  lives in the agent server's process, and OMEGA-DELTA-0060's lookup says so
  rather than implying a bad ID.
- **Enforced by:** `a_named_executor_is_honoured_or_refused_by_name`,
  `only_detected_agents_are_offered`, and
  `every_subagent_result_names_its_executor` in `crates/omega_deltas`; plus the
  suite in `crates/agent/src/tools/subagent_executor.rs`.

### OMEGA-DELTA-0101 — A subagent discloses its executor as a record, not a sentence

- **Upstream behaviour.** Upstream Zed has no executor disclosure at all: a
  subagent is the parent's own loop, so there is nothing to attribute.
- **Omega, before this.** `OMEGA-DELTA-0061` gave subagents executors and then
  disclosed them outside the surface every other executor in Omega discloses
  through. Each handle returned a hand-written sentence —
  `"Codex (codex-acp, external ACP agent)"` and
  `"Omega (native loop, inherited from parent)"` — and the tool put the string
  in the result. That met "the parent can read something" and failed the
  acceptance it was written against, which is that each subagent's
  **`ExecutorDisclosure`** names its own executor.
- **Why the shape is the point.** `OMEGA-DELTA-0021` fixed disclosure as a typed
  record that a label renders, never a stored rendering. That is the binding
  condition of the owner's 2026-07-25 identity decision — the first-party agent
  does not sign with its own principal *on the condition that* disclosure stays
  a record — and it is what keeps the decision cheap to reverse: a record of
  parts can be handed to a signer, a sentence cannot. Two hand-written strings
  also cannot be compared, cannot be checked for coherence, and one of them
  rendered the wire token `native loop` that omega#100 had already removed from
  what a person reads.
- **What a subagent discloses.**
  - An external ACP subagent: class `external_acp`, the agent id it connected
    with, and **no model in the parent tool report**. That report is created
    before the UI owns the live ACP session, so an invented model there would
    read as a disclosure while being a guess.
  - An inherited subagent: class `native_loop`, Omega's own agent id, and the
    **subagent's** provider and model — not the parent's. `subagent_model`
    overrides the inherited model for every subagent, so those are two facts and
    only the second one is what happened.
  - Neither carries a `run_ref`, because neither has run authority, and neither
    carries a `route`, because the router did not put it there — the parent named
    it in the tool call. Both records satisfy
    `ExecutorDisclosure::is_coherent`, which is the check a string could not be
    put to.
- **What reaches the parent.** `SubagentExecutorReport`, a projection of the
  record onto the wire: `class`, `agent_id`, `provider`, `model`, and no
  rendered line under any name. The parent reading it is a model, so it gets the
  parts rather than the sentence a window renders. A result written by the older
  build holds a string there; it reads as *not disclosed* rather than being
  parsed into an invented class, and it must not fail to load — the output is an
  untagged enum, so a field that fails takes the whole tool call with it.
- **What reaches the subagent card.** The card owns the external ACP thread, so
  it can read the live session's `model` config option when the adapter
  advertises one. The collapsed header renders `agent-id · model`, using the
  adapter's human-readable model name; an unrecognised selected value remains
  visible as its raw id. When an adapter does not disclose a model, the line
  says `model not disclosed` rather than guessing. This live UI projection does
  not retroactively invent a model for the parent tool report.
- **Enforced by:** `a_subagents_executor_is_disclosed_as_a_record` and
  `a_subagent_card_names_the_external_acp_model_when_the_adapter_discloses_it`,
  and `a_session_with_no_transcript_names_both_reasons` in
  `crates/omega_deltas`;
  plus `an_external_subagent_discloses_its_own_executor`,
  `an_external_subagent_never_discloses_as_the_native_loop`,
  `a_mixed_fan_out_is_attributable_record_by_record` and
  `an_inherited_subagent_discloses_its_own_model` in
  `crates/agent/src/tools/subagent_executor.rs`, and the result-shape suite in
  `crates/agent/src/tools/spawn_agent_tool.rs`. Against real binaries, the
  record is asked of the handle after a live turn in
  `crates/agent/src/tests/external_acp_subagent.rs`.
### OMEGA-DELTA-0092 — The Exo lane is derived from the install, not written by hand

- **Upstream Zed:** an external agent is a `agent_servers` entry someone
  configured. There is no notion of finding one on the machine and attaching to
  it without being asked to.
- **Omega, before this:** `OMEGA-DELTA-0055` removed the executor pin control
  and routed an unpinned thread to the external ACP agent that is attached. The
  routing worked. Nothing attached one. Reaching Exo still meant writing five
  fields — `binary`, `checkout`, `root`, `agent`, `conversation` — into
  `omega-exo-lane.json` by hand, so the automatic route had nothing to be
  automatic about, and 0055's own entry said so: *"deriving that lane from what
  detection found is omega#100's remaining deliverable and is not in this
  delta."* That is this delta.
- **Omega now:** `crates/omega_agent_detect/src/exo.rs` derives the five fields
  from what is on the machine, and `ExoLaneConfig::resolve` uses the derivation
  when there is no lane file. A person with Exo built gets a thread that reaches
  it without writing anything.
- **Exo is not found the way the other agents are, and that is not a
  workaround.** The rest of `omega_agent_detect` looks for executables on
  `PATH`, which is what `codex`, `claude`, `copilot` and `cursor-agent` are.
  Exo has no release artifact at all — its install path is `curl setup.sh |
  bash`, which clones and *builds from source* — so on this machine `exo` is not
  on `PATH` and never will be. It is a checkout with a binary under its own
  `target/`. `PATH` is therefore deliberately **not** searched for `exo`: the
  lane needs the checkout as well as the binary, and its own field
  documentation says why — the checkout is "the checkout the binary was built
  from, for the pin check". A binary found on `PATH` carries no evidence of
  which checkout built it, so pairing it with a checkout found elsewhere would
  fabricate exactly the correspondence the pin check assumes.
- **The layout trap, which this nearly shipped wrong.** Exo's `--root` is not
  where its records are. `crates/cli/src/main.rs` builds the harness with
  `root: cli.root.join("exoharness")` and opens the object store there, so an
  agent record is at `<root>/exoharness/agents/<id>/record.json`. A reader that
  looked at `<root>/agents` would find nothing on a machine that has agents and
  would report that Exo had never run here. `<root>` itself holds `adapters/`,
  `adapters.lock` and `exo-profile.md`, which belong to the CLI and not the
  harness — which is what the extra level is for. The check asserts the level.
- **What is refused rather than guessed.** Every step produces its field or
  produces `ExoLaneUnderivable`, which names the field and the exact path it
  looked at. No checkout, the *other* Exo, a checkout with nothing built, no
  state root, no agent, no conversation, and — separately — more than one agent
  or more than one conversation, listed rather than chosen between. Sending
  somebody's first message to whichever agent a directory listing happened to
  yield first is `OMEGA-DELTA-0042`'s "pointed at the wrong one" with a smaller
  radius, not a different failure. `OMEGA_EXO_CHECKOUT`, `OMEGA_EXO_ROOT`,
  `OMEGA_EXO_AGENT` and `OMEGA_EXO_CONVERSATION` each answer one of those
  refusals with one variable, which is what makes "fail naming what is missing"
  actionable rather than merely honest.
- **A directory that is the other Exo is reported, not searched past.** A person
  with exo labs' `exo-explore/exo` cloned at `~/work/exo` is in the omega#86
  situation, and telling them Omega looked in six places and found nothing would
  send them to install what they already have. A candidate that is simply absent
  is not remembered, because absence is not evidence of anything.
- **Only the upstream is checked here, and the split is deliberate.**
  `ExoPin::admits_upstream` is new, factored out of `admits` rather than
  duplicated, because a second spelling of "is this the same repository" is how
  the two answers eventually disagree on the question that already cost a day.
  The commit, the tree and the bytes stay where `OMEGA-DELTA-0042` put them:
  read immediately before every turn. Derivation answers *which install*; the
  pin answers *may it run*. A derivation that also enforced the commit would
  turn an actionable refusal on the disclosure line into a silent "no lane
  found" for anyone whose checkout had moved.
- **This reads files rather than asking Exo.** Asking means starting
  `exo agent list` before the window draws, and it needs `EXO_SECRET_BACKEND`
  set alongside `EXO_MASTER_KEY_PATH` or Exo dies with a decryption error that
  reads like a corrupt state root and is not. The cost of reading is a coupling
  to Exo's on-disk layout, and the pin is what makes that safe: `EXO_PIN` names
  an exact commit and tree, so the layout is pinned with everything else.
- **A lane file that exists wins, and it wins on existing rather than on
  parsing.** `ExoLaneConfig::load` returns `None` for a file that is
  half-written or carries the wrong schema. Falling through to derivation there
  would replace somebody's explicit, damaged configuration with a guess about a
  different `.exo` — `OMEGA-DELTA-0042` approached from the other side.
- **Derivation is admitted for the product's own lane and nowhere else.** The
  gate is that the path *is* `ExoLaneConfig::data_dir_path()`. `agent_ui`
  deliberately hands a stateless run a path inside the temporary directory that
  does not exist, so that a rendering harness never spawns somebody's Exo, and
  deriving whenever a file was absent would have undone that quietly. The gate
  is positive rather than a list of paths to exclude, so a harness invented
  tomorrow is excluded by default. A fresh `--user-data-dir` still derives,
  because `paths::data_dir()` follows it — which is the state a new person is
  in and the case the acceptance names.
- **The root is searched for, because Exo's root is wherever `--root` said.**
  This first looked only at `<checkout>/.exo`, found nothing, and the absence
  was reported as *"Exo has never been run on this machine"* — while two state
  roots with live agents sat elsewhere on the same disk, one of them the root
  the visual baselines run against. **That is the same error as the harness
  directory, one level further out:** a reader that looks in one place produces
  a confident false absence on a machine that has the thing. Exo's `--root`
  default is `.exo` relative to *the directory `exo` was run from*, which on a
  real machine is very often not the checkout. The candidates, in order, are
  `OMEGA_EXO_ROOT`, `<working directory>/.exo`, `<checkout>/.exo`, and the root
  named by the lane file at `OMEGA_EXO_LANE_FILE`. The refusal now names every
  one it tried, because a refusal that names a single path gets read as a
  statement about every path — which is precisely how the wrong summary was
  drawn from a message that was itself true.
- **The working-directory candidate has to be a directory somebody chose.**
  macOS hands a Finder or Dock launch a working directory of `/`, and a packaged
  Omega is started that way far more often than from a terminal. Read raw, the
  candidate above becomes `/.exo` on exactly the launch a new person makes —
  inert at best, and at worst a path nobody named offered to the search that
  decides which `.exo` somebody's first message lands in. `OMEGA-DELTA-0054`
  already answers "is this a directory a person chose" on this same startup
  path, to decide what a thread's `grep` and `read_file` can see, so
  `derive_lane_from_env` asks it rather than deciding again: two answers to that
  question would eventually disagree about one launch, with the thread opened on
  one directory and its Exo lane derived from another. The gate is on the
  candidate and not on the derivation — a Finder launch still reaches
  `<checkout>/.exo` and everything after it.
- **A root holding an agent beats one that merely exists.** An empty root is the
  same dead end as no root, so preferring an earlier empty candidate over a
  later working one would reintroduce the failure the search exists to remove.
  An explicitly named root is exempt: the caller said which one, and using a
  different one because theirs looked emptier would be Omega disagreeing with an
  instruction rather than answering it.
- **Only the root is taken from a lane file, and only with the schema.** Two
  fields; the other four belong to `ExoLaneConfig`, which owns the format. The
  binary still comes from the checkout that built it. A check asserts the two
  files spell the schema the same way, because a guard that silently stopped
  matching would make the search accept a file the product refuses.
- **Several conversations are ordered, not refused, and several agents still
  are.** Two agents are two capabilities — different tool modules, different
  mounts, a different model binding — so choosing between them is the "pointed
  at the wrong one" failure `OMEGA-DELTA-0042` exists for. Two conversations are
  two threads of the *same* agent, sharing its capability and its mounts; the
  worst case is a message landing in a thread the person was not looking at,
  which is visible the moment it happens. So the tie is broken by what the agent
  was last used for: `latest_event_id` is a UUIDv7 and therefore time-ordered by
  construction, so "most recent" is a comparison over a value Exo itself wrote,
  not a file mtime that a copy rewrites. Conversations that have never been used
  offer no evidence to order by, and are still refused. This is not a softening
  of "refuse rather than guess": the real `exo-lane` root holds three
  conversations on one agent, so refusing there was a dead end on the one
  machine this has to work on.
- **Enforced by:**
  `the_exo_lane_is_derived_from_the_install_and_only_for_the_product` in
  `crates/omega_deltas/`, the 30 unit checks in `crates/omega_agent_detect/`,
  and `a_lane_file_that_will_not_parse_is_not_replaced_by_a_derived_lane` plus
  `an_absent_lane_file_outside_the_data_directory_derives_nothing` in
  `crates/agent_ui/`.
- **What this does not cover.** It does not create anything in Exo.
  `OMEGA-DELTA-0042` makes `create`, `update` and `delete` unreachable and that
  is unchanged, so a state root with no agent is a refusal and not a lane Omega
  builds for itself. It does not check the commit, the tree or the bytes — see
  above. It does not search the disk for roots: the four candidates are places a
  root is named or is by convention, and a machine whose root is somewhere else
  entirely names it with `OMEGA_EXO_ROOT`. And it cannot be checked by a
  compiler: the resolution runs when the router is built in a launched process,
  which is the path `cargo check`, `cargo test` and clippy were all green across
  when it was broken before.
- **What it derived on this machine, and what it derives now.** Run from
  `scratchpad/exo-lane`, the working-directory candidate answered and the whole
  lane resolved — binary `~/work/exo/target/release/exo`, checkout `~/work/exo`,
  root `scratchpad/exo-lane/.exo`, agent `zerobase`, conversation `zb-proof` —
  which was field for field the lane file that had been written there by hand,
  including the same choice of conversation among the agent's three. **That root
  is gone.** `scratchpad/` no longer exists, and a search of `$HOME` finds no
  `.exo` and no `exoharness` store anywhere on this disk: the checkout is ours
  and the binary is built, and there is now no Exo state root on this machine at
  all. So `cargo run -p omega_agent_detect --example detect` reports the three
  agents on `PATH` and `exo lane: none`, naming both places it looked. That is
  the honest answer under this policy rather than a residual defect — and it is
  a fact about the machine, not about the search: no candidate order and no
  remembered root can derive a lane to a root that is not there.

### OMEGA-DELTA-0093 — A turn can be driven without a keyboard, over the send a typed message uses

- **Upstream Zed:** an agent turn starts because somebody typed into the
  composer and pressed Enter. There is no way to start one from the command
  line.
- **Omega, before this:** none either, and it cost real time. Proving that the
  transcript grows upward failed twice because the synthetic keystrokes landed
  in another application — a busy desktop steals focus, and a key sent to a
  window that no longer has it is a key sent somewhere else. Every visual claim
  about a turn depends on being able to send one *without* the window having
  focus, so every visual claim was blocked on a person sitting there.
- **Omega now:** `omega --omega-send "…"` opens the thread this process would
  have opened anyway and submits that text on it. `--omega-send-transcript`
  writes the thread's Markdown once the turn settles, `--omega-quit-after-send`
  ends the process with status `0` for a completed turn and non-zero otherwise,
  and `--omega-send-timeout-secs` bounds the wait. Launch, detect, send, render
  and check become one command a script can branch on.
- **It is not a second way to send, and that is the point.**
  `AgentPanel::omega_send_first_message` hands
  `AgentInitialContent::ContentBlock` with `auto_submit` to `external_thread` —
  the same call the Git panel's "review this branch diff" action already makes.
  The text lands in the composer through `MessageEditor::set_message` and the
  submit is `ThreadView::send`, the identical function the Enter key reaches, so
  mention resolution, the message queue and the send disposition all still
  happen. `AcpThread::send` is public and reachable from `crates/omega`, so the
  shortest way to make this flag "work" would have been to build a prompt and
  push it at the connection; the check refuses that by name, because a control
  surface that bypasses the production path proves nothing about the production
  path.
- **The wait is "generating, then idle", never just "idle".** A thread is idle
  for the moment between being built and the turn starting. A driver that
  waited only for idle would report a completed turn before the first token —
  a green unattended run that means nothing, which is the failure this whole
  deliverable exists to stop being possible. Both waits are checked.
- **Nothing sleeps for a guessed duration.** Connecting an agent and completing
  ACP initialization are real I/O whose length is a property of the machine.
  Every wait polls against one deadline and names what it was waiting for when
  it runs out, because "the driven turn timed out" and "timed out waiting for
  the agent panel" send a reader to different places.
- **A thread with no project is a refusal, not a success.**
  `external_thread` already returns early when the panel has no project, and
  silently doing nothing would have reported that a turn had started when none
  had. `omega_send_first_message` checks first and says so. That case is
  `OMEGA-DELTA-0054`'s: a thread whose `grep`, `read_file` and `terminal` have
  no worktree is the one that told the owner his code was an empty workspace.
- **The flag is read from the command line and from nowhere else.** The same
  shape as `OMEGA-DELTA-0047`'s zero base, and for the same reason:
  `restore_or_create_workspace` is four call sites deep and takes no `Args`, so
  a process global set once at startup keeps a command-line concern out of four
  signatures that have nothing to do with it. Nothing is written anywhere, so
  ending the process leaves nothing to repair.
- **The driver runs after every branch of the startup path.** The zero-base
  branch used to return early once it had opened a project, which would have
  made `--omega-send` work on an empty workspace and silently do nothing on the
  one case that matters.
- **Enforced by:** `a_turn_can_be_driven_over_the_send_a_typed_message_uses` in
  `crates/omega_deltas/`.
- **What this does not cover.** It does not capture a window — that is the
  visual runner's job, and it is a separate binary. It does not decide which
  executor the turn runs on; `OMEGA-DELTA-0055` and `OMEGA-DELTA-0092` do. And
  like every mode entered by a command-line flag, it cannot be reached by
  `cargo check`, `cargo test` or clippy: the check above reads the source, and
  only launching the binary with the flag proves the sequence runs.
### OMEGA-DELTA-0094 — A thread's audience is recorded on the thread, and Local needs nothing

- **Upstream behaviour.** Zed has no notion of who can read a thread. Every
  agent thread is local to the machine and to the person at it, so there is
  nothing to disclose and nothing to choose. The nearest upstream concept is
  `crates/channel`, which is collaboration over Zed's own collab server, and it
  is a different thing: a channel is a place with members, not a property of a
  thread.
- **Why Omega diverges.** omega#107 asks for more than one, and omega#108 puts a
  public Forge-backed one behind it. The moment there are two, "which one is
  this thread in" becomes a question a person has to be able to answer before
  they type, and the obvious implementation answers it wrongly. Reading the
  current selection at draw time is indistinguishable from reading a record
  while there is one audience, and is a disclosure defect as soon as there are
  two: selecting a community audience repaints every thread on screen as
  belonging to it, so a conversation held in private last week renders as one
  held in public. Nothing is published, and the person has no way to learn that
  — the only thing they can ask has told them the opposite.
- **The vocabulary, deliberately not "workspace".** The issue asks for
  "workspace identity", and the word is already spent three times: *project* is
  a directory (`OMEGA-DELTA-0054` gives zero base one), Zed's `Workspace` is a
  window, and `crates/workroom_receipts` calls a place a machine works a *room*.
  A fourth meaning is how a person reads the composer and thinks about their
  folder. The concept here is *who can read this*, so it is an **audience**:
  the one word that carries both halves the issue names — an audience and its
  history — and the only one nothing in this repository had taken.
- **The law.**
  - **Local is a constructor, not a configuration.** `Audience::local` takes no
    argument that renames it and none that gives it a reach. Its reach is
    `Reach::ThisComputer`, and `crates/omega_audience` depends on `serde` and
    nothing else, so "no account, no relay, no network" is a property of the
    dependency graph before it is a property of any code.
  - **Local is always present and always first.** `AudienceRoster::entries`
    yields it before anything else, `AudienceRoster::new` drops anything
    claiming its identity, `AudienceId::joined` refuses its key, and
    `AudienceRoster::is_empty` is `const false`. A profile that has joined
    nothing sees Local and one honest sentence, not an empty menu.
  - **The record is on the thread.** `AudienceBook::audience_of` takes a thread
    and no selection; there is no parameter it could read one from. A thread
    with no record is Local — never the selection — because the threads with no
    record are exactly the ones written before this existed, which are the ones
    somebody held in private.
  - **A thread keeps the audience it was started in.** `AudienceBook::bind`
    returns `RebindRefused` rather than overwriting, and
    `record_thread_opening` returns early on an existing record. Choosing an
    audience changes the next thread; the menu says so in as many words.
  - **The recording happens where a thread starts.** `ConversationView::new` is
    the only place that knows the difference between a new thread and a resumed
    one, because `resume_session_id` and `thread_id` are both `Option` there.
    The draw-time substitute — `AcpThread::is_draft_thread`, which is
    `entries().is_empty()` — is also true of a resumed thread whose entries have
    not loaded, so binding on it would hand a community audience to an old
    private conversation on a slow disk.
  - **It is visible without opening anything, and it is not a settings page.**
    The control is a button in zero base's composer row, ahead of the executor
    line, with the current audience on its face. The owner has rejected a modal
    setup screen repeatedly; this is beside the model and the executor, where he
    asked for controls to live.
  - **The refusal omega#108 needs is stated here, once.** `may_publish` takes a
    thread's recorded audience and returns `PublishRefused::ThreadIsLocal` for a
    local thread and `AudienceUnresolved` for one this profile cannot see. It
    takes no selection and no roster-plus-choice, so omega#108's "authorization
    before the effect" cannot be satisfied by asking a different question.
  - **An audience this build cannot resolve reads as unknown, not as local.**
    `ThreadAudience::Unresolved` renders "Unknown audience" with a warning icon
    and reports `is_private_to_this_computer() == false`. An unanswerable
    question is not a yes.
- **What this does not cover.** No check reads a rendered pixel: the position of
  the control in the row, the icons, the menu's wording and the tooltip text are
  unverified here, and omega#107's acceptance is four rendered windows. Nothing
  here publishes, joins, or transports anything — `Reach::Shared` is a value
  with no code behind it until omega#108. There is no membership, no identity
  binding, and no relay. The `OMEGA_AUDIENCE_PREVIEW` environment variable adds
  one fixture entry to the roster so the selector and the does-not-move rule can
  be looked at before omega#108 exists; it publishes nothing, its identity is
  `preview:` prefixed so it cannot be mistaken for a Forge coordinate, and it
  cannot become the default — a selection the roster cannot resolve falls back
  to Local at load.
- **Enforced by:** `local_needs_no_network_no_relay_and_no_account`,
  `the_composer_reads_the_audience_from_the_thread`,
  `the_composer_shows_the_audience_without_opening_a_menu`,
  `a_thread_keeps_the_audience_it_was_started_in`, and
  `the_audience_is_recorded_where_a_thread_starts` in `crates/omega_deltas`;
  plus the 17 unit tests in `crates/omega_audience/src/omega_audience.rs`.

### OMEGA-DELTA-0095 — The coding agent that is installed runs the turn

- **Upstream Zed:** an external ACP agent runs a thread only when a person
  picks it from the agent panel, and the picked agent then owns that thread.
  There is no first-party router above the executors, so there is nothing for a
  detected agent to be attached *to*.
- **Omega:** on a machine where `omega_agent_detect` finds Codex or Claude on
  `PATH`, that agent is connected as `OmegaAgentConnection`'s external ACP
  executor when the router is built. `OMEGA-DELTA-0055` already sends an
  unpinned thread to the attached external agent, so the first message a person
  types executes on Codex — no lane file, no pin, no flag — and
  `OMEGA-DELTA-0021`'s disclosure line names `codex-acp` rather than
  `native_loop`.
- **Why:** omega#106. Every part of this existed and one call did not.
  Detection landed on omega#100, the onboarding grid lists both agents,
  `agent_servers` hosts `codex-acp` and `claude-acp` over ACP, and the routing
  law already prefers an attached external agent. The only production site that
  registered an external executor could register the Exo harness lane and
  nothing else, so `codex-acp` and `claude-acp` appeared in the tree only as
  `stub("codex-acp")` inside tests. The visible symptom was a machine with
  Codex installed running every turn on the native loop against a
  model-provider key, and saying so in the disclosure line.
- **Presence decides, configuration does not.** The gate is an executable file
  on `PATH`, never `AllAgentServersSettings`. That map records what is
  *configured*, and Omega ships `codex-acp` in its own defaults
  (`OMEGA-DELTA-0027`), so it is non-empty on a machine that has never had
  Codex. Attaching from it would attach Codex everywhere and the failure would
  arrive as a thread that reports one executor and runs another — the defect
  class the disclosure line exists to prevent.
- **Codex first, then Claude, and the order cannot move.** The chosen agent is
  the first entry of `omega_agent_detect::CANDIDATES` that is both present and
  drivable. That is the candidate order, not `PATH` order, so the shell Omega
  was launched from cannot change which agent runs a turn. GitHub Copilot and
  Cursor are detected and are not drivable, because Omega hosts no ACP server
  for either; they are passed over by name in the log line.
- **`omega_agent_detect::preferred` is deliberately not used.** It answers the
  stricter question omega#100 asked — "is Codex here?" — and returns `None`
  when Codex is absent even though Claude is present, so that a missing Codex
  is never substituted for silently. omega#106 asks the wider question and
  answers it out loud instead: acceptance 3 is Codex absent, Claude present,
  the turn running on Claude, and the line saying so. The disclosure carries
  the agent id of the connection that ran, so `codex-acp` and `claude-acp` are
  distinguishable in the window without a second record that could drift.
- **An explicit Exo lane still wins.** The router tries
  `omega_exo_connection::connect_configured_lane` first and the detected agent
  only if that found nothing. A lane file is something the owner wrote; a
  binary on `PATH` is something that happens to be there.
- **A chosen agent that cannot start is an error naming it, never a
  fallback.** Once an agent has been chosen, the store going away, the ACP
  server never registering, and the ACP server failing to start all return an
  error carrying the agent's name and the file detection found. `Ok(None)` has
  exactly one meaning — nothing drivable is installed — and the check counts
  its occurrences to keep it that way. The no-agent case is the ordinary case
  for a new person and is unchanged: no external executor is registered, the
  native loop runs, and the composer shows its existing fallback reason.
- **`claude-acp` joins `codex-acp` in the shipped defaults.** Configured is not
  installed; this only means that when detection *does* find Claude there is an
  ACP server for it to be hosted by. Without the entry, acceptance 3 could not
  be reached on any machine.
- **The detected set is passed into the router, not read by it.** The router
  may not read the environment (`NON_DETERMINISTIC_ROUTING_TOKENS`), and a
  stateless run must attach nothing so a rendering harness never spawns
  somebody's real Codex — the same rule the Exo lane path beside it follows.
  Both are decisions of the factory in `crates/agent_ui/src/agent_ui.rs`.
- **The bridge is `CustomAgentServer`.** `AgentServerStore::get_external_agent`
  returns a `&mut dyn ExternalAgentServer`, which is a command source rather
  than something `connect` can take. `CustomAgentServer` is the `AgentServer`
  that resolves that borrow inside its own `connect`, and it is what the agent
  panel already goes through, so the attach path and the hand-picked path start
  the same process the same way.

- **Enforced by:** `the_installed_coding_agent_is_attached_as_the_thread_executor`
  in `crates/omega_deltas/`, which pins the connect sequence, the drivable ids,
  the absence of a settings-keyed gate, the single `Ok(None)`, the registration
  call in the router, the `ZED_STATELESS` guard in the factory, and both
  registry entries in the shipped defaults. The choice itself runs through its
  real function in `choose_executor`'s tests in `crates/agent_ui/`.
- **What this does not cover.** No check starts a process, so none of this
  proves that `codex-acp` launches, that it finds the Codex login, or that a
  turn streams. The acceptance is a rendered window. The bounded wait for the
  ACP server to register is 5 seconds, which is a race the check cannot
  observe: on a machine where the ACP registry never loads — no network and no
  cached registry — a detected agent spends that bound and then fails naming
  itself, and the agent panel reports the failure rather than opening on the
  native loop. That is the stated rule, and it is the one behaviour here worth
  watching in a window first.

#### Amendment, omega#106 close-out — the failure is kept, the sentence is not

The paragraph above ends by flagging the one open behaviour: a chosen agent
Omega cannot reach is a hard error, and the agent panel reports it instead of
opening on the native loop. That was reopened and decided. **The rule stands.**
What changed is what the failure says, and why the rule is now believed rather
than merely stated.

- **The case for degrading was real.** An unreachable *registry* is not an
  absent *agent*. `codex` is on `PATH`, its login works, and the only thing
  missing is Omega's own download. Refusing to open a thread over that reads as
  the app failing to open, and "the app must open" is the older promise.
- **The case against it is stronger, and it is not the principle.** It is
  `ConversationView::handle_agent_servers_updated`. A view in
  `ServerState::LoadError` re-drives its connect whenever the agent-server
  store changes, and the ACP registry finishing its load is exactly such a
  change. So a registry that is a few seconds late already costs a few seconds
  of a named error and then heals into Codex by itself. A degrade would connect
  *successfully* with the native loop, land in `Connected` with no thread
  error, and never be re-driven — leaving the session on the native loop for
  good after the agent became reachable. That is a thread running one executor
  while the reader believes another, arrived at from the opposite direction:
  the same defect the disclosure line exists to prevent.
- **The distinction cannot be drawn where it would have to be drawn.** At the
  instant the 5-second bound expires, a registry three seconds behind and a
  registry permanently gone are the same observation. Any policy fixed there is
  wrong in one direction or the other. An error defers the question to time,
  which is the only thing that can answer it, and the retry closes the loop.
- **The offline case buys less than it looks.** With a warm registry cache the
  adapter registers and the attach succeeds. With a cold one — a first launch
  that has never reached the network — the native loop has no configured
  provider either, so a degrade hands back a composer that fails one layer
  later with a worse sentence.
- **What was actually wrong was the attribution.** Detection proves the `codex`
  binary exists. What runs the turn is `codex-acp`, a *separate* npx adapter
  resolved from the ACP registry. The failure sentences opened with the
  reader's binary and its path — "Codex is installed at /usr/local/bin/codex,
  but …" — and then reported a failure, which reads as "your Codex is broken"
  and sends them to debug the one part that is working. That is the
  honest-attribution rule pointed the wrong way. The sentences now name the
  adapter, say the registry is what could not be reached, say the reader's
  install is fine, and say Omega retries.
- **The 5-second bound is short on purpose.** It is far under the registry's
  own 30-second fetch timeout. Waiting out the fetch would show a spinner where
  an explanation belongs; expiring early shows the explanation and lets the
  retry do the waiting.
- **The residual cost, stated rather than argued away.** `Agent::NativeAgent`
  *is* the router, so while a chosen agent stays unreachable no picker entry
  reaches the native loop, and a persistently unreachable adapter leaves the
  panel with no first-party path. The fix is an explicit "run on Omega's own
  loop" action on the error — a choice the reader makes, not a substitution
  made for them — and it belongs to the panel, not to the attach.

- **Enforced by:** `a_failed_attach_is_retried_when_the_adapter_registers` and
  `an_attach_failure_names_what_omega_could_not_reach` in
  `crates/omega_deltas/`. The first is the load-bearing one: the argument for
  failing lives in `conversation_view.rs`, which `omega_agent_attach` does not
  own, so deleting the retry would quietly falsify this amendment. Plus
  `the_drivable_ids_are_adapters_and_not_the_detected_binaries` in
  `crates/agent_ui/`, which pins the adapter/binary split the sentences depend
  on.

### OMEGA-DELTA-0100 — The composer stays at the bottom and the transcript grows up to it

- **Upstream Zed:** an empty thread gives the composer the whole panel. The
  message list collapses to nothing, the composer takes `flex_1().size_full()`,
  and the column between them is `justify_between()`.
- **Omega, before this:** the rule was already inverted in zero base, by omega#99
  and omega#100, and nothing asserted it. Zero base zooms the panel to the whole
  window, and at that size upstream's rule put the text field at the top of the
  screen, the model and pin dropdowns at the very bottom, and a field of dead
  black between them. The owner asked for the input to sit at the bottom, the
  way a chat surface puts it.
- **Omega now:** the composer hugs its content in both modes unless the reader
  explicitly expands it. The empty transcript above it takes the remaining
  space, so a thread grows upward from a composer that stays at the bottom. In
  a full-editor split the field stays attached to its controls instead of
  becoming a full-height left column.
- **Three facts produce it, and each is separately load-bearing.**
  `fills_container` is exactly `editor_expanded`, so an empty thread cannot
  absorb the panel in either mode. The empty-transcript branch takes the space
  in both modes, with
  `flex_1().size_full()` on a thread with no messages — without that, nothing
  claims the space and the composer floats back up; without the first, two
  elements both expand. And the conversation is drawn *before* the composer in
  the column, which is the whole of "the transcript grows upward" and is the one
  fact a reader assumes rather than checks.
- **Why it needed a check at all.** This is omega#100's fifth acceptance, and
  the only evidence for it was four PNG baselines compared at a 0.99 pixel
  threshold. A rebase restoring the upstream branch would be a large visual
  change and would fail that comparison — but only for somebody who regenerates
  the baselines, and those four are exactly the ones that need a live Exo state
  root to produce. There is no Exo state root on this machine (see
  `OMEGA-DELTA-0092`), so today they cannot be regenerated at all: the sole
  check on the layout rule was one that cannot currently be run. A source check
  does not replace the picture — it cannot see a layout — but it does make the
  three facts a rebase has to delete on purpose rather than by accident.
- **The empty-transcript assertion is scoped to its branch.**
  `flex_1().size_full()` also appears on the branch that *has* messages, so a
  whole-file `contains` stays green with the zero-base branch deleted — the
  branch that does the pushing. The check reads from the branch's own condition
  and was watched failing with that branch removed.
- **The order assertion reads the last `.child(conversation)`.** Three earlier
  spellings sit inside the Exo inspector's own match and are not the column the
  composer is in.
- **Enforced by:**
  `the_composer_stays_at_the_bottom_and_the_transcript_grows_up_to_it` in
  `crates/omega_deltas/`. All three assertions were watched failing against a
  falsified `thread_view.rs`, which was then restored byte for byte.
- **What this does not cover.** It is a source check and it proves no pixel. The
  acceptance still asks for a capture somebody opened, and the capture still has
  to come from the visual runner, which builds its workspace in-process and
  never takes the launch path.

### OMEGA-DELTA-0102 — Exo's protocol is enumerated once, and decided about twice

- **Upstream Zed:** no Exo, no exoharness, no second agent runtime whose request
  protocol Zed has an opinion about. There is no upstream behaviour to revert
  to; this entry records how Omega holds somebody else's protocol.
- **Omega before this change:** two lanes landed on 2026-07-26, hours apart,
  each carrying its own complete transcription of the same 52 request variants.
  `omega_exo_episode::family::EXO_REQUEST_FAMILIES` (`OMEGA-DELTA-0090`,
  omega#103) partitioned them into admitted and refused families for the episode
  reset. `omega_exo_log` plus this registry's `EXO_LOG_ADMITTED_KINDS` and
  `EXO_LOG_UNADMITTED_KINDS` (`OMEGA-DELTA-0091`, omega#104) split them into the
  eight reads that client may name and the forty-four it may not. Both
  transcriptions were read off `Request::kind` at the pin, both were correct,
  and they agreed exactly — the second lane diffed them and said so. That is the
  *good* case, and it was still one copy too many: the next variant upstream
  adds would have had to be noticed twice, by two people who each already
  believed their list was complete. Neither lane could move it while the other
  was in flight.
- **Omega now:** the list lives once, in `omega_exo_lane::protocol` as
  `ExoRequestKind` — 52 variants, upstream's declaration order, upstream's wire
  strings, and nothing else. Each crate above keeps **its own decision** as a
  total function over that type: `omega_exo_episode::family::family_of` returns a
  `RequestFamily`, and `omega_exo_log::admission::is_admitted_read` returns a
  bool. No consumer spells an Exo request kind as a string literal any more, and
  the check refuses one that starts to.
- **Two decisions, not one shared decision.** This is the part worth reading
  twice, because the obvious "fix" for a duplicated list is a single shared
  answer, and that would be wrong. The two admit different subsets on purpose:
  - `conversation_fork` and `start_sandbox` are **admitted** by the episode law
    — forking *is* the episode reset and the restore is its filesystem half —
    and **refused** by the log client, which omega#104 scoped to read-only in as
    many words.
  - Ten variants that read — `list_agents`, `list_conversations`,
    `get_sandbox_process_events`, `wait_sandbox_process`, and the six binding
    list-and-get variants — are **`Query`** in the episode partition, because
    they change nothing, and **refused** by the log client, because omega#104
    scoped it to *a conversation's own record* and a list of every agent on the
    host is not that.

  A merge would hand one of the two a capability nobody granted it, and the
  worse direction is obvious: a read-only client that can fork.
  `the_two_decisions_over_exos_protocol_are_not_merged` asserts the
  disagreement in both directions and that the reader stays the smaller
  authority of the two.
- **A 53rd variant cannot pass unclassified.** Every decision is a `match` with
  no wildcard arm, so upstream's next variant is a build failure on the person
  adding it, in each crate independently, rather than a runtime discovery by
  whoever sent it. A `_ =>` arm would make the same variant compile and be
  classified by nobody — safe, in that the default is refusal, and invisible,
  which is how one protocol stayed transcribed twice for a day. The wildcard is
  refused by name.
- **The enumeration cannot quietly shrink either.** `ExoRequestKind::ALL` is
  `[Self; EXO_REQUEST_KIND_COUNT]` and `ordinal` is a wildcard-free `match`
  checked against each entry's position, so an array that forgot a variant,
  listed one twice, or grew without the count growing fails
  `the_enumeration_lists_every_variant_exactly_once`.
- **The registry checks by calling, not by scraping.** `omega_deltas` takes the
  three Exo law crates as dev-dependencies — all leaves, no gpui, no process, no
  filesystem — so `both_decisions_are_total_over_the_one_enumeration` resolves
  every variant identifier it parses out of a source file through the compiled
  enum, and a parse that misread the file fails instead of passing. Comments are
  stripped before every scan: a commented-out match arm reads as a classified
  variant to `contains`, and that exact shape produced a false green in both
  prior lanes.
- **Enforced by:** `exos_request_protocol_is_transcribed_in_exactly_one_place`,
  `both_decisions_are_total_over_the_one_enumeration`, and
  `the_two_decisions_over_exos_protocol_are_not_merged` in
  `crates/omega_deltas/`, plus `the_enumeration_lists_every_variant_exactly_once`
  and `every_variant_has_its_own_wire_spelling` in `crates/omega_exo_lane/`.
- **What this does not cover.** Nothing here opens a socket. The enumeration is
  read from `crates/exoharness/src/protocol.rs` at `omega_exo_lane::EXO_PIN`; a
  running `exo serve` has never confirmed it. If upstream adds a variant, this
  fails on the *next transcription*, not on the day upstream adds it — no check
  here watches the pinned tree change.

### OMEGA-DELTA-0105 — What omega#107's acceptance can be proved without a window, is

- **Upstream behaviour.** Not a divergence from Zed in its own right. This is
  `OMEGA-DELTA-0094` closed out: that delta shipped the audience concept, the
  record on the thread, and the composer control, and it recorded that its four
  acceptance items were "four rendered windows" that nobody had looked at. This
  one separates the part of that acceptance which is a structural property from
  the part which genuinely needs an eye, proves the first, and bounds the
  second so it can be looked at once instead of per case.
- **Why Omega diverges.** A property that can be checked and is instead left to
  a screenshot is a property that will be true on the day of the screenshot.
  Three of `OMEGA-DELTA-0094`'s own recorded claims about the preview fixture
  had nothing holding them at all, and one of them was false.
- **The law.**
  - **Local is what a fresh profile *loads* into, not only what an unrecorded
    thread resolves to.** `OMEGA-DELTA-0094` pinned `AudienceBook::audience_of`'s
    fallback. The other fallback on that path is the selection in `loaded`,
    which hydrates from the key-value store; nothing held it, and pointing it
    at any other audience left all five of that delta's checks green while
    making a machine that has never chosen anything start its threads
    elsewhere. Both fallbacks, and the filter that discards a stored selection
    the roster cannot resolve, are now checked.
  - **`may_publish` refuses a fixture by identity, before it consults reach.**
    The preview entry is `Reach::Shared` on purpose — it exists to make the
    not-private case observable — and `may_publish` answered on reach, so it
    returned `Ok` for it. Nothing publishes today, so nothing left any machine;
    the first transport wired behind that gate would have been authorised, on
    any machine with `OMEGA_AUDIENCE_PREVIEW` set, to publish into an audience
    that does not exist. `PublishRefused::AudienceIsAFixture` is the refusal,
    and it is reached for a resolved fixture and for a record that outlived the
    variable alike.
  - **`preview:` is reserved in both directions, exactly as `local` is.**
    `AudienceId::preview` produces it and `AudienceId::joined` refuses it, so
    omega#108's Forge coordinates cannot land on the prefix and a fixture
    cannot be minted through the door a real membership arrives at. The claim
    that the fixture "cannot be mistaken for a Forge coordinate" was a naming
    convention, and a convention is a thing the code that has to respect it has
    never heard of.
  - **The fixture describes itself as a fixture.** It used to make a joined
    audience's sentence — "Shared with everyone in Preview audience. Needs a
    network." — which is false in both halves. A fixture that describes itself
    as a place is the one way this fixture can mislead somebody.
  - **What the fixture means is a function of a value, not of the
    environment.** `omega_audience::preview_audience` takes the variable's
    value; `agent_ui` reads the variable and decides nothing. Absent, empty and
    `0` produce nothing; `1` produces the default name; anything else is the
    name, so the long-name case can be looked at on purpose.
  - **The name on the control's face is bounded to `MAX_LABEL_CHARS`,
    counted in characters.** An audience name is not a value this repository
    chooses: omega#108's come from a Forge repository and the fixture's comes
    from an environment string. The button sits in a `flex_wrap` row whose only
    other text is the `OMEGA-DELTA-0021` executor disclosure, which is
    `.truncate()`d — so an unbounded name does not merely look wide, it takes
    room from the mandatory attribution of which executor ran the turn. The
    menu entry and the tooltip stay unbounded, because they have the room and a
    name nobody can read in full anywhere is a name nobody can check.
  - **The menu's three sentences are written once.** They are the least
    verified thing in this feature and the one part of it no check can reach:
    a person picks a different audience, the menu closes, and the button reads
    what it read before — correct, and also what a broken dropdown looks like.
    So they are constants in `omega_audience` beside the rules they describe,
    with the guess and its falsifier written out, and a literal reappearing in
    the control fails.
  - **A dependency cannot be declared where Local's allowlist cannot see it.**
    `local_needs_no_network_no_relay_and_no_account` reads `[dependencies]` and
    stops at the next `[`, so `[dependencies.tokio]`, `[build-dependencies]`
    and `[target.'cfg(unix)'.dependencies]` are invisible to it. The manifest
    is held to the two sections that check can read.
- **Enforced by:** `local_is_what_a_fresh_profile_loads_into`,
  `the_menus_sentences_are_written_once`,
  `the_audience_on_the_composer_is_bounded_and_the_row_can_wrap`,
  `the_preview_audience_is_a_fixture_and_not_a_place`, and
  `nothing_can_declare_a_dependency_the_local_allowlist_cannot_see` in
  `crates/omega_deltas`; plus six unit tests in
  `crates/omega_audience/src/omega_audience.rs`, each watched failing against
  the mutation of the rule it is about.
- **What this does not cover.** Still no pixel. Whether the two menu sentences
  read as a deliberate control rather than a broken one, whether the row fits a
  narrow dock with a 24-character audience name beside the disclosure and the
  model selector, and whether a fresh profile's button is legible where it sits
  are all rendered facts, and all three need somebody who has not read this
  file. The bound makes the second one answerable once rather than per name;
  it does not answer it.

### OMEGA-DELTA-0106 — A shared audience is a Forge repository, and every install already knows how to contribute

- **Upstream Zed:** a conversation with the agent is a local thing. There is no
  concept of a room a person is invited into, no signed record of what was said,
  and no shipped procedure for contributing to the editor itself — a
  contributor reads the repository's documentation if they find it, and their
  agent invents the rest. The only skill compiled into the binary upstream is
  `create-skill`.
- **Omega:** `crates/omega_community` says what a shared audience actually
  stands on. `OMEGA-DELTA-0094` gave a thread an audience and left the shared
  case abstract on purpose; this fills it in with an OpenAgents Forge
  repository, the people the Forge already admitted, and NIP-22 messages bound
  to the repository's NIP-34 coordinate. Two more skills are compiled into every
  install: `omega-contributing` and `omega-delta-discipline`.
- **No new membership system, deliberately.** `FORGE-04` (openagents#9246)
  already binds one npub per tenant to an OpenAgents account and grants
  `forge:admin`, `forge:member`, or `forge:viewer`. `ForgeMembership` is that
  authority's own response decoded. Omega cannot admit anybody; it can only read
  what the Forge decided and refuse to exceed it — a `forge:viewer` cannot post,
  because the Forge would not issue that person a push credential either. A role
  this build has not heard of grants nothing and locks nobody out.
- **The audience rule is carried, not restated.** `may_post` calls
  `omega_audience::may_publish` and then adds only the fact the audience crate
  cannot know. Two implementations of "a local thread may not publish" agree
  until the day one of them is edited, and the wrong one is the one nobody is
  looking at.
- **A revoked member is refused by the audience, before membership.** Losing a
  membership removes the room from the roster, so a thread recorded in it stops
  resolving and reads as `Unknown audience` rather than as private. That is the
  true sentence — "you cannot see this room" — where "sending failed" would be
  an invitation to retry.
- **Omega composes bytes and never signs them.** A signed record is *accepted*,
  never produced: the crate names no key type outside its tests, so a person's
  identity stays theirs and Omega has no key to lose. A signature over anything
  other than the exact authorized bytes is refused, which is the one place a
  signer could substitute something.
- **Omega ships the ability to join a Forge repository, not the address of
  one.** A `CommunityDescriptor` is what an invitation carries — tenant,
  repository, coordinate, relays, and the name a person reads. The crate names
  no host, which `COMMUNITY_FORBIDDEN_IN_PRODUCTION` holds for the reason
  `OMEGA-DELTA-0070` gives about the public-chat skill: a host name in the code
  makes the code work for exactly one deployment. Relays are a list from the
  first contract, because widening that later would be a stored-record
  migration for every profile that had joined anything.
- **The binding has no default.** An outbound event with its room tag removed
  fails rather than falling back to whichever room is selected. There is nothing
  to fall back to.
- **Pending work is visible and stops.** The outbox is a record rather than a
  set of in-flight futures, so it survives a restart; a relay refusal that will
  not change fails on the first answer; everything else stops at five attempts
  in a state a person is shown. Retrying forever is the failure omega#108 names,
  because a person watching "sending…" for an hour has been told nothing.
- **The skills are the migration seam, and they describe today.** GitHub is
  authoritative for accepting changes to this repository, but Omega must not
  contain GitHub Actions. Test and release gates remain repository-owned local
  scripts or external infrastructure. The Forge epic (openagents#9242)
  describes demoting GitHub as a *target*, and the owner directed that the
  conversion be figured out through these workrooms rather than before them.
  So `omega-contributing` describes the current contribution path, a check
  refuses the epic's target phrasing until the authority actually changes, and
  the same check refuses GitHub Actions, their configuration, and their source
  generator.
- **The precedence is unchanged, and this is the point.** `SkillSource::BuiltIn`
  stays at precedence `0`, so a contributor's own `omega-contributing` shadows
  the shipped one. A default somebody cannot replace is a removal of their
  control.
- **Enforced by:** `the_community_audience_carries_no_transport_and_no_key`,
  `the_room_carries_the_audience_rule_rather_than_restating_it`,
  `the_contribution_skills_ship_in_the_binary`, and
  `the_contribution_skills_describe_the_path_this_repository_uses` in
  `crates/omega_deltas/`, plus the crate's own checks in
  `crates/omega_community/`. The last of the four is the one that matters most:
  it asserts the skills against the tree — the files they cite must exist, the
  delta ID shape they teach must be the one `ENFORCED_DELTAS` uses — rather than
  against a copy of their own sentences.
- **What this does not cover.** It is the first rung of one row of the parity
  ledger: a room, membership, and messages. Threads, reactions, pins, presence,
  and read state are later packets. It ships no transport, no composer control,
  and no pane — nothing in this delta connects to a relay, and the audience
  selector, the invitation flow, and the conversation actions omega#108 asks for
  are still owed. Nothing becomes public by default; public read stays
  per-repository and behind the Forge epic's own promise gates.

### OMEGA-DELTA-0103 — A tool result is an artifact; its event is a preview that names what it withheld

- **Upstream Zed:** a terminal tool result goes onto the event whole. The only
  bound is `output_byte_limit` — 16 KiB, applied by `terminal_tool` — and it is
  silent in the text: the response carries a separate `truncated` boolean, and
  the body it cut says nothing about having been cut. Nothing keeps the full
  result anywhere, so what the limit removed is gone.
- **Omega:** a terminal result over `TOOL_RESULT_PREVIEW_BYTE_BUDGET` (4,000
  bytes) is recorded whole as a versioned artifact addressed
  `terminal:<id>@v<n>`, and the event carries a preview ending in a marker that
  names the bytes and lines withheld and the address to fetch the rest from. A
  result under the budget is untouched: no artifact, no marker, no ceremony.
- **Why:** omega#105, from the owner's 2026-07-26 screenshots. A thread ran
  several `nak` commands returning raw Nostr events, and about forty lines of
  hexadecimal identifiers and signatures went into the record. `OMEGA-DELTA-0080`
  hides that from a reader. It does not stop the model's own context, a
  transcript reader, or a receipt from carrying the blob, because a rendering
  ceiling is a property of one surface and this is a property of the record.
- **The marker is the whole point, and it is one sentence.** `truncation_marker`
  is the only place the withheld amount is put into words, and it states what is
  missing before it states where the rest is, so a reader who stops after the
  first clause has still been told the body is incomplete. Silently dropping the
  middle of a result is how a reader concludes something did not happen when it
  did — the failure class behind the false greens of 2026-07-26.
- **Room for the marker is reserved inside the budget**, following
  `OMEGA-DELTA-0060`: the bound can never be the reason the reader is not told
  about the bound. The reserve here is the marker *rendered at its widest* —
  every count at its total, which no real count can exceed — rather than
  `OMEGA-DELTA-0060`'s hand-written 320. A constant is a number that falls
  behind the sentence it protects; this one cannot. Squeeze the budget to zero
  and the marker survives alone.
- **A result cut twice reports both cuts through one marker.** The terminal's
  own `output_byte_limit` runs before the preview ever sees the text, so the
  totals are passed in rather than measured off the body. Measuring the body
  would describe the second cut and silently absorb the first.
- **The artifact is complete, never a preview.** An artifact that were itself
  bounded would leave the full result nowhere, and the fetch path would answer
  with the thing the reader already had.
- **Versions accumulate; addresses do not move.** A source alone would silently
  re-point at a later capture, which is how a reader ends up quoting output that
  no longer exists. Recording text identical to the current latest returns that
  version rather than appending, so a version number counts changes and not
  reads.
- **A running command has no artifact, and says so.** There is no complete
  result to version until the command exits, so the marker says the command is
  still running instead of naming an address that resolves to nothing.
- **The store lives on the terminal, which is what outlives the turn.** A
  finished `ToolCall` still owns its `acp_thread::Terminal` entity, so an
  artifact recorded during the turn is still addressable after it ends. There is
  no second index to drift from it.
- **This and `OMEGA-DELTA-0080` are independent, and both are needed.** 4,000
  bytes of unwrapped Nostr JSON is still about forty lines — the height the
  owner objected to — so a preview cannot replace a 16-line ceiling, and a
  ceiling cannot replace a bounded record. The check asserts the budget stays
  far above the ceiling's worth of text, so neither can be deleted in favour of
  the other.
- **The seam between them is one function, and it is now closed.**
  `tool_output_ceiling_label` takes `record_total_lines`. Without it the label
  counts only the lines that reached the surface, and a reader who lifts the
  ceiling and reaches the last line concludes they have the whole result. When
  the record holds no more than the body — every terminal-backed result today —
  the input is `None` and the sentence is byte-for-byte the one
  `OMEGA-DELTA-0080` shipped. When it holds more, the label names both
  remainders: `Show 24 more lines · 360 more withheld`. A body short enough that
  the ceiling never bound it still draws the control, disabled, because it has
  nothing to open and must not pretend otherwise.

- **Enforced by:** `a_tool_result_is_an_artifact_and_its_event_is_a_marked_preview`
  in `crates/omega_deltas/`, which pins the budget, every fact the marker must
  state, the single sentence, the reserve, the fetch path, the recording at
  exit, both preview branches of `current_output`, and the wiring of the
  record's total into the ceiling control. The arithmetic runs through its real
  functions in `crates/acp_thread/src/tool_result_artifact.rs`'s tests — including
  the falsifier, `a_truncated_preview_is_distinguishable_from_a_complete_one` —
  and the label in `test_tool_output_ceiling_label_names_what_the_record_withheld`
  (`crates/agent_ui/`).
- **What this does not cover.** The bound is applied at
  `acp_thread::Terminal::current_output`, which is the terminal tool's result
  path. A tool that returns a text content block rather than a terminal — every
  native tool in `crates/agent/` — is unbounded still; that half needs
  `crates/agent/src/thread.rs` to route its results through
  `preview_tool_result`, and a `read_tool_result_artifact` tool to spend the
  address the marker hands out. Until it does, the model can read a marker it
  has no way to act on. Nothing here reads a rendered pixel, so the disabled
  control and the label a person sees are unverified. The artifact lives in
  memory for the life of the thread; it does not survive a restart, and no check
  asserts that it does.
### OMEGA-DELTA-0113 — A joined Forge room reaches the composer's selector, and is operated from a line

- **Upstream Zed:** the composer has no audience. A conversation is local, there
  is no room to be invited into, and no way to say from inside the conversation
  that a person now belongs to one.
- **Omega before this change:** `OMEGA-DELTA-0094` gave a thread an audience and
  a control to read it on; `OMEGA-DELTA-0106` said what a shared audience stands
  on. Neither connected them. The only non-Local entry any selector could show
  was the `OMEGA_AUDIENCE_PREVIEW` fixture, and joining a room was not something
  a person could do at all — `omega#108` recorded it plainly: "it ships no
  transport, no composer control, and no pane."
- **Omega now:** an invitation is a line, accepting it is a line, and the room
  it admits you to is in the composer's audience selector for as long as you are
  in it. `omega_community::JoinedRooms` is the durable set,
  `agent_ui::omega_community_control` is the edge that stores it and reads a
  key, and `omega_audience_control` builds its roster from that seam rather than
  keeping a second list beside it.
- **The room is a line, not a pane.** The owner's requirement in omega#108 is
  that "joining, seeing who is present, and posting are conversation actions"
  rather than a separate administrative surface. So `omega_community_control`
  renders nothing and has no menu; its whole control surface is `run`, which
  takes a line somebody typed. The grammar lives in `omega_community::command`,
  where it is checked without a window.
- **The recogniser is a literal, deliberately.** `parse` answers `None` for
  anything not beginning with `/community`, and refuses an unknown verb by name
  rather than resolving it to the nearest one. A person writing "I should join
  the omega room" to their agent has described an intention, not issued an
  instruction, and a recogniser that could not tell those apart is one that
  sometimes publishes on a hunch. "Did you mean post" is a helpful sentence and
  a dangerous behaviour.
- **A verb that takes nothing refuses the rest of the line.** `\/community leave
  the room when you are done` reads like a sentence; ignoring the tail would
  have left the room.
- **An invitation carries the Forge's own answer, and grants nothing.**
  `FORGE-04` binds one npub per tenant and issues the roles; with no transport,
  the binding travels in the invitation instead of being fetched. It is a claim
  and not an authority: what it decides is which room appears in *this* person's
  selector, which is their decision about their own machine. The Forge still
  issues the credentials, and the relay still refuses an event from somebody it
  does not admit.
- **An unrecognised invitation field is refused; an unrecognised role is kept.**
  The opposite calls, on purpose. A role this build has not heard of grants
  nothing, so refusing it would lock somebody out of a room they are in. A field
  may be part of the room's address, and joining while discarding it is joining
  something other than what was sent.
- **A room a person cannot read never reaches the selector.** `JoinedRooms::join`
  asks the Forge's answer before it records anything, so a revoked binding is
  refused at the join. A selector entry that fails on the first send is worse
  than no entry: it is an invitation to type the message again.
- **Leaving keeps nothing, and says what that costs.** The room goes, and the
  threads recorded in it stop resolving — `Unknown audience` rather than
  private, because a conversation held in a room this profile has left was never
  private and Omega will not start saying it was.
- **"Who is here" answers what Omega has verified, and names its own basis.**
  The Forge answers about one binding at a time, so there is no roll to read.
  `RoomPresence` lists the keys that have signed a record in the room, and says
  in the same breath that this is not a member list. "3 people are here" would
  have been a confident wrong answer to a question this cannot answer.
- **Nothing reports a send that did not happen.** `\/community post` runs the
  whole authorization — `may_publish`, the room match, the Forge's roles — and
  composes the exact bytes, and then says that nothing in this build signs or
  reaches a relay. The message is not queued and it is not lost; there is
  nowhere for it to go. The refusals, though, are real today, and they are
  omega#108's own falsifiers.
- **The key is read at the edge and nowhere else.** `omega_community` still
  names no key type outside its tests, so a person's identity stays theirs;
  `omega_community_control` reads the public half from `omega_identity`, which
  is where a check can see it.
- **Enforced by:** `the_selector_offers_the_rooms_a_person_joined`,
  `the_room_is_operated_from_a_line_and_not_a_pane`, and
  `posting_is_authorized_before_a_single_byte_is_composed` in
  `crates/omega_deltas/`, plus the crate's own checks in
  `crates/omega_community/`. `OMEGA-DELTA-0106`'s dependency and
  key/host bans now cover every file in the crate rather than the three it
  shipped with, so a new module cannot be the one that quietly opens a socket.
- **What this does not cover.** There is still no transport and no signer wired,
  so nothing leaves the machine and no record arrives — which means acceptance 2
  of omega#108 (a message visible to the other member) is not reachable in this
  build and is honestly reported as such rather than simulated. `run` is not yet
  called from the composer's send path; the line a person types has an executor
  and not yet a caller.

### OMEGA-DELTA-0107 — Omega reads Exo's durable log from a server the owner runs, and starts none

- **Upstream Zed:** no Exo, no exoharness, no second agent runtime whose durable
  record to read. There is nothing upstream to revert to here; this entry
  records a decision and its consequence.
- **Omega before this change:** `OMEGA-DELTA-0091` landed
  `crates/omega_exo_log` — compiled, unit-tested, and with **nothing calling
  it.** Its only two dependents in the workspace were dev-dependencies of
  `crates/omega_deltas`, added by `OMEGA-DELTA-0102` so the registry could check
  a decision nobody was using. A law with no caller is a law about nothing.
- **Omega now:** `ExoHarnessConnection::read_durable_history` is the call.
  `ExoDriver::observe` already reads the ids Exo prints and now keeps them on
  the inspector, so the lane can name the agent and conversation it is running
  turns on — the lane's configuration holds *slugs*, and Exo addresses
  everything by `Uuid7`.
- **Route A, and the reason for it.** omega#104 named two routes and did not
  pick. This takes **A**: Omega reads `exo serve` when `EXO_EXOHARNESS_URL`
  names a loopback one, and never starts one.
  - Route B — Omega spawning the server — is new process authority, a port, and
    a lifetime to own. Worse, it puts a **second writer on one `.exo` root**,
    which is exactly what `omega_exo_episode::root::ExoRoots` exists to refuse
    and the interleaving that makes a fork a copy of a history that never
    existed. And Omega cannot know whether the owner already has one running.
  - `serve` is already in `EXO_REDIRECTING_FLAGS` and `ExoCommand` cannot
    express it. This delta adds the other half: the attach itself may not spell
    a way to start one, held against the connection's *string literals* rather
    than as substrings, because `observe` contains `serve`.
- **The consequence, made legible rather than hidden.** With the variable unset
  — the ordinary machine, and the safe one, because the CLI reads the state root
  on disk and no socket exists at all — the durable log is simply unavailable.
  That must read as **not configured**, never as *this thread has no history*.
  So `ExoDurableHistory` has no `Default`, an unavailability is a value carrying
  a sentence, and every one of the three sentences ends with
  `ExoHistoryUnavailable::NOT_AN_EMPTY_HISTORY`. A surface that showed the
  absence as an empty conversation would be telling the reader something false
  about their own thread.
- **Two passes, and the second is the one with the history in it.** Exo's event
  log *names* artifacts and never contains them, so the first render is the
  question — which bodies would change the rendering — and the second is the
  answer. `ExoReadClient::conversation_history` does both, in the crate that
  owns both halves, so the sequence is falsifiable against a scripted loopback
  server rather than only against a machine somebody has to have.
- **An artifact is a versioned record.** `ExoArtifactSet` is keyed by
  `(artifact_id, version)`. Keyed by id alone — as it landed in `b074ac3986`,
  and as a reviewer caught — inserting version 2 made every version-1 reference
  render version 2's bytes, and a set holding only version 2 made a version-1
  reference *look resolved*. The row read as complete and artifact-backed while
  showing a body from a later point in the conversation: the durable-replay
  claim failing in the one direction nobody would notice. The unresolved list
  now carries references rather than ids, so the second pass asks for the
  version the event named instead of for whatever is latest.
- **omega#103's half that Omega can own.** `conversation_fork` does not copy the
  `snapshots` prefix, and separately a sandbox that was never snapshotted has
  nothing to restore. **Exo reports both with the same sentence** — its own
  *"loading snapshot manifest for `<id>` (have you taken a snapshot?)"* — which
  sends a reader hunting a fork bug when nobody ever took a snapshot.
  `admit_filesystem_reset` now takes `SnapshotEvidence` and refuses without it
  as `ResetRefusal::NoSnapshotObserved`, separately worded, before a request
  exists. The upstream patch for the *other* case is written out on omega#103,
  is cross-repo and owner-gated, and is **not made here**.
- **Enforced by:** `the_exo_durable_log_is_read_by_the_lane_that_runs_the_turns`,
  `omega_reads_exos_server_and_never_starts_one`,
  `an_unreadable_exo_log_never_renders_as_a_thread_with_no_history`,
  `the_durable_read_is_two_passes_and_respects_artifact_versions`, and
  `a_reset_with_no_snapshot_is_refused_by_name_and_not_by_exos_confusion` in
  `crates/omega_deltas`; plus unit tests in `crates/omega_exo_log`
  (`each_versioned_reference_renders_its_own_version`,
  `an_unavailable_durable_history_names_its_cause`,
  `the_second_pass_carries_the_tool_results_the_first_only_named`,
  `an_artifact_read_that_fails_costs_one_body_and_no_rows`),
  `crates/omega_exo_episode` (`a_fork_with_no_snapshot_is_refused_for_having_none`),
  and `crates/agent_ui`
  (`an_observed_turn_carries_the_ids_the_durable_read_addresses_by`,
  `an_exo_that_printed_no_id_loses_its_history_and_not_its_lane`).
- **What this does not cover.** **Nobody has run this against a live
  `exo serve`.** The framing, the envelope, the request shapes and the ordering
  are exercised against a loopback HTTP server this repo writes; the answers are
  the tests'. omega#104's two live acceptance items — an Exo thread's events
  read after the turn ends, and an artifact an event references resolving — are
  agreement with the source at the pin, not with a running Exo, and they stay
  open on a machine with one. There is also **no call site in a view**: the
  method exists and is checked, and what draws it is a separate change in
  `thread_view.rs`.

### OMEGA-DELTA-0110 — A profile with no identity files adopts the identity already in custody, and says so

- **Amended by omega#164:** the startup gate is silent background provisioning
  (`OMEGA-DELTA-0040`), so an `Unadopted` profile is adopted at launch through
  `adopt_custodied` — the same one-way adoption this record demands, now
  unattended, matching the precedent `provision_unattended` set for the hosted
  lane (`OMEGA-DELTA-0159`). The invariants keep their teeth in the amended
  checks: `Unadopted` is never counted ready without an adoption transaction,
  never routed to `create` (whose empty-store fallback generates), and a
  planted transaction still resolves to `Incomplete` and refuses. The
  `onboarding_required` startup predicate this record cited is replaced by the
  exhaustive state mapping in `provision_for_process_start`. The identity
  section's unadopted screen and its disclosure sentence survive for explicit
  visits.
- **Upstream Zed:** first-run onboarding asks for nothing that outlives the
  profile directory. `--user-data-dir` is a complete reset of who you are to
  the app, because there is nothing about you outside it.
- **Amended by `OMEGA-AUTH-00`:** every channel stores the signing key in
  `identity/identity.secret` below that channel's application data root. The
  store uses atomic replacement and owner-only Unix permissions and is not
  encrypted at rest. `KeyringLocator` is a version-one logical compatibility
  name, not a macOS Keychain backend. The frozen current-state contract is
  `docs/omega/nostr-authentication-contract.md`.
- **What omega#110 reported is the conclusion, not the scope.** A brand-new
  `--user-data-dir` has no public identity files and a configured secret store
  that already holds an identity. Custody called that `Incomplete` — the state written for a
  transaction that was interrupted — so onboarding said *"Identity setup needs
  repair: a prior recovery transaction needs the same owner-authorized identity
  candidate"* and offered **Recover identity** as its only control. There was
  no prior transaction. `create` was refused for the profile not being
  `Absent`, `resume_incomplete_create` was refused for having nothing to
  resume, and recovery needed an encrypted artifact the owner may not have to
  hand. With `OMEGA-DELTA-0040` parking the front door behind identity
  onboarding, **the composer was unreachable on a fresh profile** — which is
  why every acceptance item phrased "a fresh `--user-data-dir` reaches a
  composer" was unsatisfiable, and why every "fresh profile" run that day was
  not fresh.
- **The state is now separated, and a transaction is what separates them.**
  `resolve_locked` resolves a data root with no manifest and a readable secret
  to `CustodyState::Unadopted` when no transaction is on disk, and keeps
  `Incomplete` when one is. Damage is still reported as damage: a planted
  transaction beside the same secret-store entry still resolves to `Incomplete`,
  still says "Identity setup needs repair", and adoption over it is refused as
  a conflict rather than overwriting it.
- **The screen says which, because the owner ruled that it must.** Adopting
  silently and adopting visibly are the same behaviour and different products.
  The unadopted screen shows the npub and fingerprint it is about to adopt and
  states *"Omega adopts that identity for this profile; it does not create a
  second one"*, under a control labelled **Use this identity** — not "Create
  identity", which would name an identity the owner does not get.
- **Adoption adopts.** The control routes to `IdentityService::adopt_custodied`,
  not `create`. `create` falls back to generating when the store turns out to
  be empty; reached from a screen that has already named an npub, that fallback
  would produce a different identity behind the sentence promising this one. An
  empty store is a refusal here (`CustodyDenied(Absent)`) with nothing
  generated, nothing written, and no transaction left on disk. `create` itself
  stays refused on an unadopted profile: replacing a custodied identity with a
  fresh one is a reset, and a reset is a separate owner-authorized decision.
- **`OMEGA-DELTA-0040` is unchanged and is checked to be.** `Unadopted` is not
  `Ready`, so the fresh profile still waits on onboarding; this fixes what
  onboarding *concludes*, not whether the wait happens. The gate predicate is
  now the named `onboarding_required`, and both this delta and a unit test hold
  it: a state that counted as ready would open a composer having silently taken
  an identity nobody was shown, which is omega#110 with the opposite sign.
- **Enforced by:**
  `a_profile_with_no_identity_files_adopts_the_custodied_identity_and_says_so`
  in `crates/omega_deltas`; plus unit tests in `crates/omega_identity`
  (`a_fresh_profile_beside_a_custodied_identity_is_adoptable_not_damaged`,
  `an_interrupted_transaction_beside_a_custodied_identity_is_still_a_repair`,
  `adoption_refuses_rather_than_generating_when_custody_turns_out_empty`) and
  `crates/onboarding`
  (`a_fresh_profile_is_offered_the_custodied_identity_and_told_it_is_adopted`,
  `a_genuinely_interrupted_transaction_still_reports_repair`,
  `a_machine_with_no_omega_identity_is_still_offered_creation`,
  `every_custody_state_but_ready_holds_the_startup_wait`).
- **What this does not cover.** **No window has been opened.** Every claim here
  is proved against fabricated state — a temporary data root and a fake secret
  store — so the test cannot disturb the owner's live signing identity. So "a
  fresh profile reaches a
  composer" is proved as far as the gate: custody resolves to `Ready` after
  adoption, and `Ready` is the predicate the startup wait releases on. The
  pixels between that release and a composer are `OMEGA-DELTA-0019` and
  `OMEGA-DELTA-0040`'s, already covered, and unverified in the same launch as
  this change. The genuinely-new-user case — a channel data root whose secret
  store has never held an Omega identity — is answered the same way: `Absent`
  still offers
  **Create identity**, asserted against a fabricated empty store rather than by
  deleting the owner's secret file. A profile choosing a *different* identity
  from the custodied one still goes through recovery or reset; there is no one-click
  "give this profile its own identity", and adding one would be a decision
  about what a profile is, not a bug fix.
### OMEGA-DELTA-0114 — The install a person waits on is bounded and named, and the way past it is theirs to take

- **Upstream Zed:** an external ACP agent is started only after a person picks
  it from the agent panel, so its npx resolve happens somewhere the reader
  already knows they asked for a download, and the panel they were using is
  still there while it runs. `LocalRegistryNpxAgent` reports no progress
  (`set_loading_status_tx` is the trait's empty default for it) and nothing
  bounds the resolve or the ACP `initialize` handshake, which upstream can
  afford because neither sits in front of a first composer.
- **Omega:** `OMEGA-DELTA-0095` moved that same resolve onto the path between a
  new person and their first message. This delta makes it bounded
  (`ADAPTER_START_TIMEOUT`), named, and *visibly alive* — the label counts
  seconds — and gives a reader whose adapter never arrives an explicit **Run on
  Omega's Own Loop** button rather than no way forward at all.
- **Why:** omega#106's close-out. The attach put an unbounded silent download
  between the composer and the first turn, where it had not been before. A
  first-run person waiting on a pulsing `Loading…` cannot tell an install from a
  hang, and had no way to end either.
- **What the wait actually is, which is not what it said.** Not a release
  archive: `npm exec --yes` resolving an npx package
  (`@agentclientprotocol/codex-acp`, `claude-agent-acp`), plus Zed's Node
  runtime if this machine has none. It **recurs** — `npm exec --yes` runs on
  every connect, not once — so it is not written as a first-launch sentence.
  Nothing bounded it: the resolve has no deadline, and
  `agent_servers::acp`'s `initialize` races only against the child process
  exiting, so an adapter that started and never answered held the panel open
  for as long as the machine stayed on.
- **The elapsed count is the mechanism, not decoration.** A bound alone leaves a
  wait nobody can read; a label alone leaves a wait nobody can outlast. A number
  that goes up is what separates a slow link from a wedged process, and a person
  can read it without knowing what npx is. `ADAPTER_START_TIMEOUT` is
  deliberately generous at three minutes for the same reason: a cold resolve is
  tens of megabytes, and a deadline tight enough to catch a hang quickly would
  turn a working first launch on a slow connection into a failure, which is the
  worse of the two errors.
- **The channel has exactly one holder, so it is taken and then forwarded.**
  `watch::Sender` is not `Clone` and closes on drop. The router hands its whole
  delegate to `NativeAgentServer::connect`, which binds it as `_delegate` and
  drops it — so on `origin/main` the panel's loading-status channel was **dead
  before the attach began**, and the attach then built its own delegate with
  `None` in that slot. `AgentServerDelegate::take_loading_status` moves it to
  the one part of the sequence that downloads anything. The adapter still gets a
  channel of its own and whatever it says wins, so the archive path's
  `Installing {version}…` survives and Omega only fills the silence the two npx
  adapters leave.
- **`Agent::NativeAgent` is the router, so a failed attach had no exit.**
  `OMEGA-DELTA-0095` recorded that cost when it kept the hard failure. There is
  no picker entry that reaches the native loop, so a persistently unreachable
  adapter left the panel as a callout and nothing else.
  `run_on_omegas_own_loop` is the payment, and **who calls it is the whole
  design**. From a button, after a reader has read what failed, it is a choice:
  the thread runs where they asked and the disclosure says `native_loop`
  truthfully. From a timeout handler or a retry limit it is precisely the silent
  substitution that delta refused, reached through the escape hatch instead of
  through the policy — so the call sites are counted across the tree rather than
  trusted.
- **The escape hatch must not become the policy.** The choice is per-process and
  is not persisted, it is offered only once an attach has actually failed
  (`unreachable_adapter` is typed state, not a read of the error prose), and a
  restart goes back to the adapter. A decision made about a network that was
  down a minute ago should not outlive the process that made it.
- **`OMEGA-DELTA-0095` is amended in two places, and neither is to make a check
  pass.** `DETECTED_EXECUTOR_CONNECT_STEPS` said
  `AgentServerDelegate::new(store, None, None)`, which spelled "this connect
  reports nothing"; that is no longer what it must do. And `Ok(None)` now has
  two admitted reasons rather than one — nothing drivable is installed, or a
  person chose Omega's own loop — while the *count* stays at one, because both
  funnel through the same return and the rule being counted is unchanged: no
  **failure** may reach it.
- **Attribution, finished.** `14c519b0ec` fixed the registration sentence; the
  store-gone sentence still opened with the reader's binary and its path. It now
  names the adapter alone. Every remaining mention of the binary in this path is
  there to exonerate it.
- **Enforced by:**
  `the_adapters_npm_start_is_bounded_and_says_how_long_it_has_taken` and
  `only_a_person_sends_a_thread_to_omegas_own_loop` in `crates/omega_deltas`,
  plus `a_person_who_chose_omegas_own_loop_gets_it`,
  `omegas_own_loop_is_not_chosen_by_default`,
  `the_starting_label_counts_and_names_the_adapter` and
  `the_adapter_start_is_bounded_well_past_a_slow_download` in `crates/agent_ui`.
  Each was watched failing first: the amended 0095 step against the new source,
  and the two new checks against a timeout that calls
  `run_on_omegas_own_loop`, a deleted `take_loading_status`, a router that drops
  the channel, a restored direct `.await` on the connect, a button offered with
  no failure behind it, and an attach that stops consulting the choice.
- **What this does not cover.** No check starts a process, so nothing here
  proves what a person sees. **The three windows this owes are listed on
  omega#106 and none of them exist yet**: the ticking label during a real npx
  resolve, the callout carrying the button, and a thread that ran on the native
  loop after the button was pressed disclosing `native_loop`. A fresh
  `--user-data-dir` cannot currently reach a composer at all (omega#110), so
  none of them can be produced from here. The bound itself is also untested
  against a real hang — the test asserts the constant's shape, not that a wedged
  npx resolve ends in a sentence.


### OMEGA-DELTA-0111 — Every tool result is bounded, and the address the marker prints can be spent

- **Upstream Zed / Omega before this:** `OMEGA-DELTA-0103` bounded the terminal
  path and left the rest of the agent unbounded. Every native tool that returns
  a text block — `read_file`, `grep`, `edit_file`, `fetch`, `diagnostics`, the
  MCP tools — put whatever it produced onto the record whole. And the marker
  `OMEGA-DELTA-0103` did emit named an artifact nothing could fetch: the address
  was honest and unspendable.
- **Omega:** the bound is applied once, in `Thread::run_tool`, on the way from a
  tool's output to `LanguageModelToolResult`. A result over
  `TOOL_RESULT_PREVIEW_BYTE_BUDGET` is recorded whole in the thread's
  `ToolResultArtifactRegistry` under `tool:<tool call id>` and the event carries
  `OMEGA-DELTA-0103`'s preview and marker. A new tool `read_tool_result_artifact`
  takes the address the marker prints and returns the complete text, windowed by
  line and bounded by bytes.
- **The bound is where every tool passes, not in each tool.** A per-tool bound
  is one a new tool is unbounded by *forgetting*, and the tool that forgets will
  be the one nobody tested. `AgentTool::bounds_own_result` defaults to `false`
  for the same reason: the unbounded result is the dangerous one, so a tool that
  does not answer gets bounded rather than exempted.
- **Three tools opt out, and each already carries its own visible marker.** The
  two terminal variants (`OMEGA-DELTA-0103` records and previews inside
  `acp_thread::Terminal`), `read_subagent_transcript` (`OMEGA-DELTA-0060` bounds
  it to `MAX_TRANSCRIPT_BYTES` and marks every bound that fires), and the fetch
  tool itself. Bounding an already-bounded body twice is worse than either
  alone: the second cut removes the first cut's marker and reports the preview's
  own size as the total.
- **One truncation sentence for the whole system.** The agent half calls
  `preview_tool_result` and never restates it — including inside
  `read_tool_result_artifact`, whose own byte backstop speaks the same sentence
  that sent the reader there. A second, differently worded sentence is how a
  reader learns to skip both. The check asserts these files reuse the law and do
  not contain the sentence.
- **The second, wronger sentence is deleted.** `terminal_tool::process_content`
  wrapped a truncated response in `Command output too long. The first {} bytes:`
  — a second truncation sentence on top of `OMEGA-DELTA-0103`'s accurate one,
  whose byte count was computed from the *formatted* string, fences and prefix
  included, so the number it printed was never the number of bytes shown.
  `output.truncated` is already carried by the marker inside the body.
- **An address that does not resolve says why.** `ArtifactLookup` has three
  answers, not two: found, wrong version (which names the versions that exist),
  and forgotten. A bare "not found" would collapse a caller's off-by-one into a
  fact about the thread, and it reads as *that result never existed* — the same
  false-absence class the marker was built against, moved from the marker to the
  fetch. A malformed address is answered separately again, as the typo it is.
- **What this did not cover, and where it went.** This delta declined to make
  artifacts survive a restart, on the reasoning that persisting them would put
  every complete tool result back on disk unbounded. **`OMEGA-DELTA-0121` found
  that premise false** — the complete result is already on disk, in
  `LanguageModelToolResult::output` — and took the copy that was already there
  instead. `AGENT_UNRESOLVED_ARTIFACT_REQUIRED_FACTS` still holds, narrowed to
  the terminal addresses it is still true of. Read 0121 for the argument.
- **Also not covered:** nothing reads a rendered pixel here either. The
  marker and the fetch are checked as text and as a round trip through a fake
  model, not as something a person saw in a real thread.
- **Enforced by:**
  `every_tool_result_is_bounded_and_the_marker_it_prints_can_be_spent` in
  `crates/omega_deltas`; plus unit tests in `crates/agent`
  (`tool_result_artifacts::tests`, `tools::read_tool_result_artifact_tool::tests`)
  and the end-to-end
  `test_large_native_tool_result_is_bounded_and_its_address_is_spendable`,
  `test_small_native_tool_result_is_untouched`, and
  `test_an_address_from_a_reopened_thread_says_why_it_no_longer_resolves`.
### OMEGA-DELTA-0121 — Every address a marker prints is one something can take

- **Omega before this.** `OMEGA-DELTA-0103` gave a bounded tool result a marker
  naming an address to fetch the rest from, and `OMEGA-DELTA-0111` built the
  fetch path. Three kinds of address were still being handed out that nothing
  could take, and an unspendable address is the failure the marker exists to
  prevent, arriving one layer down: the reader is told the rest is available,
  acts as though it is, and is answered as though the result never existed.

- **1. Every `terminal:` address.** `acp_thread::Terminal` recorded the complete
  result and printed `terminal:<id>@v<n>`. `Terminal::result_artifacts` had **no
  caller anywhere in the tree** — the store was write-only — and the fetch tool
  reads the thread's registry, which held only `tool:` sources. So every
  terminal marker was decoration, on the exact path the owner screenshotted, and
  the fetch tool's own documentation advertised `terminal:2@v3` as an address to
  copy. `ToolResultArtifactRegistry::adopt` takes the terminal's whole store —
  whole, because a re-run terminal's second capture is `@v2` and re-recording it
  into an empty store would answer `@v1` — and refuses any store in the `tool:`
  namespace, so the separation `OMEGA-DELTA-0111` relies on cannot be lost by
  adoption. It is handed over *before* `current_output` forms the preview, so
  the marker can never exist before the artifact it names.

- **2. Every `tool:` address after a reopen. The decision, argued.**
  `OMEGA-DELTA-0111` declined to persist artifacts because "persisting them puts
  every complete tool result back on disk for the life of the thread and grows
  without limit, which is the size property this exists to hold." **That premise
  is false, and checkably so.** `Thread::run_tool` puts the tool's *complete*
  output in `LanguageModelToolResult::output`; `AgentMessage::tool_results` is
  `Serialize`; `DbThread` holds those messages. The unbounded copy has been on
  disk since long before omega#105 was filed. `OMEGA-DELTA-0111` bounded what
  reaches the model, and never bounded what reaches the file — which is right,
  because `Thread::replay` rebuilds the tool call's rendering out of that same
  `raw_output` when the thread is reopened. The thread you reopen *shows you the
  whole result* while the fetch tool tells the model it is not recoverable.

  So the choice was never "no copy on disk" versus "one copy". It was one copy
  versus two. Two is strictly worse: the same bytes again, plus a `DbThread`
  migration, to answer a question the first copy already answers. And the
  middle options — persist only what is referenced, cap the total, evict the
  oldest — are all ways of managing a second copy, so they inherit the cost they
  were meant to avoid and add a policy to get wrong.

  This takes the first copy. `Thread::replay_tool_call` reads each saved
  `raw_output` back through its own tool (`AnyAgentTool::llm_output_from_raw`,
  the exact inverse of the one step `run` took to produce it) and re-runs the
  same pure `bound` over it. Same text, same order, same addresses. Nothing new
  is written and nothing new is kept: the registry becomes an index over bytes
  that were already there. It is idempotent on a thread that already holds its
  artifacts, because recording text identical to the latest version returns that
  version rather than appending one.

  A tool that cannot be reproduced exactly answers `None` rather than guessing.
  An MCP result's text parts are saved *concatenated*, so rebuilding from them
  would number the versions differently and resolve an address to something
  other than what it named — a wrong answer, which is worse than a refusal that
  says why.

- **3. Any result the model windowed itself.** `terminal`'s `head_lines` and
  `tail_lines` are model-facing, and the tool's description tells the model to
  prefer them over piping to `head`. They windowed the *whole* preview, and
  `OMEGA-DELTA-0103`'s marker is at the end — so a head window deleted it. The
  model was handed a body cut twice that said it was cut zero times, on the
  common path. The marker is now split off first (`split_truncation_marker`, in
  the law, beside the words it searches for, so no second copy of them exists to
  drift) and put back after. The window also *names what it dropped*: `head: 1,
  tail: 1` rendered the first line, a blank line, and the last, which reads as
  adjacency and is the silent middle-drop omega#105 names outright. Four
  existing expectations changed for that, and the head/tail overlap no longer
  prints a line twice as though the command had.

  The window's note is deliberately **not** the truncation sentence. The two
  cuts have different remedies — widen the window versus fetch the artifact —
  and telling a reader to spend an address for lines the artifact never withheld
  sends it after nothing.

- **The refusal now depends on which kind of address failed.** A `tool:` address
  that does not resolve was never this thread's or cannot be rebuilt; a
  `terminal:` address that does not resolve very likely *was* this thread's and
  stopped when the process did. One sentence for both would have to be vague
  enough to be true of both. Told the wrong one, a reader either re-runs a
  command it did not need to or gives up on a result one correctly-spelled
  address away.

- **The gap that remains, and it is the narrow one.** A terminal's complete
  output is the one result `DbThread` does not hold — the tool returns the
  preview, so the preview is what is saved. That is `OMEGA-DELTA-0103`'s size
  property, and persisting it *would* be the second copy `OMEGA-DELTA-0111`
  described, of the results most likely to be enormous. It stays unpersisted,
  and `AGENT_UNRESOLVED_ARTIFACT_REQUIRED_FACTS` is the standard it is held to:
  the refusal names the lifetime that caused it and never reads as a result that
  never existed. A check asserts nothing artifact-shaped appears in `DbThread`,
  so the second copy cannot arrive as a field without being re-argued.

- **Still not covered:** nothing here reads a rendered pixel. The disabled
  ceiling control from `OMEGA-DELTA-0103` and the marker text a person sees in a
  real thread remain unverified, unchanged from what `OMEGA-DELTA-0111` said.

- **Enforced by:**
  `every_address_a_marker_prints_resolves_and_no_window_can_erase_it` in
  `crates/omega_deltas`; plus `tool_result_artifacts::tests` in `crates/agent`
  (`a_terminals_own_store_becomes_addressable_here`,
  `an_empty_store_is_not_adopted`,
  `a_forgotten_tool_address_says_the_rebuild_could_not_reach_it`,
  `a_forgotten_terminal_address_still_names_the_lifetime_that_caused_it`),
  `tools::terminal_tool::tests::test_select_terminal_output_keeps_the_truncation_marker_it_was_handed`,
  `acp_thread::tool_result_artifact::tests::a_marker_can_be_found_again_by_the_words_it_was_written_with`,
  and the end-to-end `test_a_terminal_marker_address_is_spendable` and
  `test_an_address_still_resolves_after_the_thread_is_saved_and_reopened`.
### OMEGA-DELTA-0112 — An external subagent is spawned by the tool, and the panel can find it

- **Upstream behaviour.** Upstream Zed has no per-spawn executor: a subagent is
  the parent's own loop, so nothing spawns an agent server, nothing has a
  session on somebody else's connection, and the subagent card always finds its
  thread where it looks.
- **Omega, before this.** `OMEGA-DELTA-0061` gave `spawn_agent` an `executor`
  and `OMEGA-DELTA-0101` gave the result a real disclosure record. Both were
  checked, and **the tool path had never executed**. omega#102's live test
  opened its own session with its own three calls —
  `CustomAgentServer` → `connect` → `new_session` — which proves
  `ExternalAcpSubagentHandle` and cannot fail when the *caller* of those calls
  is wrong. It was wrong.
- **The defect a re-stated sequence could not see.**
  `NativeThreadEnvironment::create_external_acp_subagent` opened the session
  with `PathList::default()`. `session_directories_from_work_dirs` takes the
  first path as the session's `cwd` and refuses an empty list with *"Working
  directory cannot be empty"*, so **every external subagent failed at
  `new_session`, on every machine**, before any agent-specific behaviour was
  reached — and the failure surfaced as the generic *"Could not open a … session
  for this subagent"*, which reads as the agent's fault. The parent's own
  working directories are now used, falling back to the project's default list,
  which is what the panel already does when it opens a subagent session. A
  subagent asked to work on this project must be looking at this project.
- **Why the panel had nowhere to render.** The subagent card resolves its thread
  from the connection's session map. An external subagent is deliberately not in
  it: the handle holds only an `AcpThread`, there is no native `Thread` behind
  it, and `NativeAgent::sessions` never learns of it, because the session
  belongs to the agent server that runs its own loop with its own login and its
  own tools. `AcpThreadEvent::SubagentSpawned` carries an id and nothing else,
  and `load_session` on the native connection cannot produce that thread, so the
  card had a name it could not resolve.
- **One fact, recorded once.** `external_subagent_sessions` maps a session id to
  the `AcpThread` behind it, written where the session is opened and read by the
  panel. It is not a second session map: it decides nothing, owns nothing, and
  is consulted by nothing that runs a subagent. The entities are **weak** —
  the subagent's lifetime belongs to the tool call that spawned it, which drops
  the connection and therefore the child process when it ends, and a map that
  never forgot would silently extend that for the life of the process. The panel
  takes a strong reference through the `ThreadView` it builds, which is why the
  card still shows what the subagent did after the agent server is gone.
- **The card says who ran it.** A non-native executor is named on the card, read
  through `omega_executor_disclosure` — the same classification every other
  surface in `crates/agent_ui` uses, so a card and a thread cannot disagree
  about what ran. Nothing is shown for a subagent on Omega's own loop: naming
  Omega inside Omega is noise, and absence means the default, which is what it
  already meant. `OMEGA-DELTA-0021`'s law does not stop applying because the
  reader is looking at a card rather than at a thread.
- **Enforced by:**
  `an_external_subagent_is_reachable_by_the_tool_and_findable_by_id` and
  `the_panel_resolves_an_external_subagent_and_names_its_executor` in
  `crates/omega_deltas`; `an_external_subagent_is_resolved_by_id_and_names_its_own_executor`
  in `crates/agent_ui/src/conversation_view.rs`; and, against the real
  `codex-acp` and `claude-agent-acp` binaries,
  `one_turn_spawns_two_codex_and_one_claude_through_the_tool`,
  `an_external_subagent_is_findable_by_session_id_while_it_runs` and
  `reading_an_external_subagent_transcript_refuses_with_its_own_reason` in
  `crates/agent/src/tests/external_acp_subagent.rs`
  (`cargo test -p agent --features e2e --lib external_acp_subagent`).
- **What this does not cover.** **Nobody has seen the three cards.** The panel's
  lookup is proved headlessly — the id resolves, the thread is the right one,
  and it classifies as `external_acp` — and removing the lookup fails that check
  on exactly that. What is not proved is the drawing: the card's layout with an
  executor name in it, three of them in one turn, has not been rendered in a
  window. omega#109's acceptance 3 stays open for that reason and no other.


### OMEGA-DELTA-0116 — A path argument names the project, never the mode, and the folder it names is on screen

- **Upstream Zed:** `zed <path>` opens `<path>` in the editor. There is no mode
  to change, so the argument can only mean one thing.
- **Omega, before this:** `OMEGA-DELTA-0052` made zero base the default and
  removed the way out of it, then read a non-empty `paths_or_urls` as *"this
  names something to edit"*. So `omega --user-data-dir <profile>
  /Users/…/work/omega` booted the full editor — file tree, dock, status bar —
  while a bare `omega` stayed in zero base.
- **Omega now:** a path argument sets the **project**. `omega <path>` opens
  the surface with `<path>` as the folder the thread reads, searches and runs
  in. While the mode split existed, `--full-editor` was the one flag that
  opened the editor; omega#161 removed it, so there is no mode for a path to
  leave alone any more.
- **Why this was the same defect twice.** The owner's rule, given when zero base
  became the default, is quoted in `OMEGA-DELTA-0052`: *"they must be stuck in
  zero base with no way out if it was started in this mode. which must be the
  default starting now. booting the full editor must require a separate flag."*
  A positional path is not a flag. `OMEGA-DELTA-0052` removed the way out from
  inside the app and left its twin on the command line — and left it reachable
  **by accident**, because opening a project is the most ordinary thing a person
  types. The owner found it in about ten seconds.
- **The dedicated mode flag was the only selector, and now none exists.**
  While the split lasted, `--diff`, `--dev-container`, and `--demo-workroom`
  asked for surfaces the default did not draw, so each declared
  `--full-editor` as a clap prerequisite and omitting it was a visible
  command-line error. omega#161 deleted all four arguments; for one release a
  stale invocation gets a startup error naming the removal, and the check now
  fails if any argument grows back into a surface selector.
- **Development uses the same launch.** `script/zed-local` used to append the
  editor flag to every local Omega process, so "open in dev mode" meant
  "silently opt out of the product surface." It passes the caller's arguments
  to the first instance unchanged and passes no arguments to the others, and
  since omega#161 there is no editor flag for anyone to type.
- **A path that no longer changes the mode has to do something.** Otherwise the
  repair turns `omega <path>` from "opens the wrong product" into "does nothing
  visible", which is not obviously better. `resolve_zero_base_project_arguments`
  rewrites the parsed arguments into the folder they name, so everything
  downstream — the open listener, the workspace, the worktree, the `cwd` an
  external agent is spawned in — keeps its single existing meaning for a path.
- **A file argument becomes the folder that holds it.** Zero base has no pane to
  open a buffer into, so `omega src/main.rs` can only usefully mean "work in
  `src`". A single-file worktree would be the `OMEGA-DELTA-0054` failure with
  one file in it instead of none: `grep`, `find_path`, `list_directory` and
  `terminal` would all still have essentially nothing to operate on.
- **`OMEGA-DELTA-0054` keeps the one answer.** `project_root_named` is
  `plausible_project_root` with two more jobs — resolve a relative argument
  against the working directory, and climb from a file to its directory — and it
  lives in `omega_workdir` beside the rule it applies. An argument gets no
  exemption: `omega /` and `omega ~` are still refused. A path that names nothing
  is refused rather than climbed, because falling back to the parent of a typo
  opens a directory nobody named, which is the failure that module exists to
  prevent.
- **The second half is how the first half was found.** The owner asked the agent
  which directory it was in and got the build worktree. That answer was correct:
  an external agent is spawned in the project root, and the project root was
  whatever directory the launcher happened to be in. Nothing in the window said
  so — there was no way to notice before asking and no way to check the answer
  after. The panel header now names the folder in zero base, with the whole path
  in its tooltip.
- **It goes in the header, and specifically not in the composer's bottom-left.**
  The owner looked at the running app and had that corner emptied (`be27475c11`)
  and asked for it to stay empty. The header already carries what the thread
  *is* — its agent and its title — and where it runs belongs with those. A check
  fails if the working directory's spelling reaches the composer.
- **Only when there is a folder.** With none, the composer already says *"No
  folder open — file search and terminal have nothing to read"* and offers the
  control that fixes it. Repeating that in the header would be the same fact
  twice, which is the exact objection that emptied the bottom-left.
- **The glance is shortened from the front, not the back.** The two directories
  the owner confused were a checkout and a build worktree; the build worktree's
  head is `/private/tmp/claude-501/…`, which identifies nothing, and its tail is
  the only part that does. An end-truncated label would therefore have been most
  misleading in exactly the case that produced this delta, so
  `short_display_for_person` keeps the last three components and marks the cut
  with a leading `…`. `$HOME` becomes `~` for the same reason: a home prefix
  spends width the tail needs. The whole path is in the tooltip, unabbreviated —
  this is the glance, not the record.
- **Enforced by:** `a_path_argument_sets_the_project_and_never_the_mode` in
  `crates/omega_deltas`, plus
  `a_directory_argument_is_the_project_it_names`,
  `a_relative_argument_is_resolved_against_the_working_directory`,
  `a_file_argument_names_the_directory_that_holds_it`,
  `an_argument_that_names_nothing_is_refused_rather_than_climbed`,
  `an_argument_is_still_refused_when_it_is_not_a_project`,
  `a_working_directory_is_written_the_way_a_person_wrote_it` and
  `shortening_keeps_the_end_that_identifies_a_directory` in
  `crates/omega_workdir`. Each assertion in the delta check was watched failing
  against the source before the repair, and against the repair reverted one
  piece at a time: the restored `paths_or_urls` term, a deleted resolver, a
  resolver that decides for itself what a project root is, a
  `project_root_named` that no longer climbs from a file, a deleted header
  method, a header that spells the path itself, a label that is built and never
  rendered, and the spelling moved into the composer.
- **`OMEGA-DELTA-0052` is amended, and not to make a check pass.** Its list of
  implied terms lost `paths_or_urls` and its prose says why. Both halves it
  defended were re-asserted here while the split lasted; since omega#161 the
  stronger property holds — no argument asks for any second surface at all.
- **What this does not cover.** **No window has been opened.** No check in this
  repository starts the binary, so nothing here proves what a person sees:
  `omega <path>` staying in zero base, the header carrying the folder, or an
  agent answering with the directory that was typed. Those three are omega#111's
  acceptance and they stay open. The header's label is also unproved against a
  narrow panel — `truncate()` is asserted, a rendered width is not.

### OMEGA-DELTA-0119 — A file link in the transcript opens a reader, and a link that resolves to nothing says so

- **Upstream Zed:** a file link in an agent message opens the file in the centre
  pane. The editor is always drawn, so this always works.
- **Omega, before this:** the same handler ran and the same file opened. The
  owner clicked
  `crates/agent/src/templates/system_prompt.hbs` in a live build and *"nothing
  happened."*
- **The click was never a no-op, which is why it took reading to find.** The
  code-span resolver in `conversation_view.rs` recognises the path, resolves it
  against the project, and hands the markdown renderer a `file://` URI — that
  resolution is the reason the text is blue and underlined at all, and an
  unresolved code span is styled as plain inline code. So the handler existed,
  the path resolved, and `open_abs_path_at_point` opened the buffer, moved focus
  into it, and put it in the centre pane. `OMEGA-DELTA-0053` does not draw a
  centre pane once zero base is sealed. The file opened somewhere with no
  pixels, and it took the composer's focus with it.
- **An invisible success is indistinguishable from an unimplemented handler.** A
  person reports the second. Any repair that leaves a click with no visible
  consequence — including a repair that fails to find the file — reproduces the
  bug under a different cause, so the reader draws in every case, including
  failure.
- **Omega now:** in a sealed zero base, a transcript link to a local file opens
  `crates/agent_ui/src/omega_file_peek.rs` — a read-only sheet in the workspace's
  modal layer, carrying the file, its path, and the line the link named.
- **Read-only, and not as a simplification.** Zero base refuses `workspace::Save`
  at the action gate. An editable sheet would take typing and then have nowhere
  to put it, which is a larger version of the lie being repaired. The header says
  *Read only* so the surface is not mistaken for the editor returning.
- **`OMEGA-DELTA-0052` is not weakened.** No dock, no pane, no tab, and no way
  out. The modal layer is rendered by `MultiWorkspace` outside the seal — which
  is why the command palette still opens in zero base — and it is absolutely
  positioned with no height of its own. It therefore takes part in no layout and
  **cannot clip or push the composer**, which is the property the composer's
  wrapping exists to protect. Its height is bounded so it covers the transcript
  and not the composer. Dismissing it leaves the window zero base already had.
- **A link that resolves to nothing still opens the sheet.** It prints the path
  exactly as the agent wrote it, and every directory that was searched. The two
  failures a person actually hits — the agent invented the path, and the agent is
  running somewhere this window is not — are told apart only by reading that
  list, so the list is never elided. With no roots at all, it says the thread has
  no working directory and names the control that fixes it, which is the state
  `OMEGA-DELTA-0054` already put a folder picker beside the composer for.
- **The thread's own working directories are the first roots, ahead of the
  project's worktrees.** An external executor's session carries its own `cwd`,
  and `AcpThread::work_dirs` is where that reaches the panel. Resolving a
  relative path against the process working directory, or against a worktree the
  agent is not running in, is how a link ends up naming a file that is not
  there. Both root sets are searched, in that order, and both spellings are tried
  under each root: `crates/foo.rs`, and `omega/crates/foo.rs` where the agent has
  repeated the root's own folder name because that is what its `cwd` shows it.
  The literal join is tried first, so a real `omega/` subdirectory still wins.
- **Anchors are supported in the link and nowhere else.** `foo.rs:42`,
  `foo.rs:42:7`, `#L42`, `#L42:7` and `#L1-L150` all move the cursor, tolerating
  a missing or lower-case `L`. An anchor that will not parse is dropped and the
  file opens at the top, because refusing the link over an unreadable anchor
  would turn a cosmetic problem back into a dead click. Prose ranges such as
  *"(lines 1-150)"* written **beside** a link are not part of its target and are
  not read; only the link's own text is.
- **The reader is bound to the seal.** The reader takes the click only when
  `omega_zero_base::is_sealed()`, which is the exact moment the centre pane
  stops being drawn. Under omega#161 the shipped process seals at startup, so
  in production this is always; unsealed windows exist only in tests and proof
  harnesses, where the ordinary open path keeps its behaviour. Links that are
  not local files — `https`, threads, fetches, rules, directories — are
  declined and keep the handling they have.
- **Enforced by:** `a_transcript_file_link_opens_a_reader_in_zero_base` in
  `crates/omega_deltas`, plus the nine parser tests in
  `crates/agent_ui/src/omega_file_peek.rs`. Each assertion in the delta check was
  watched failing against the source before the repair and against the repair
  reverted one piece at a time: the reader deleted, the seal gate removed, the
  read-only call removed, a pane-opening call reintroduced into the reader, the
  unresolved arm removed, the reader called after `open_link` instead of before
  it, and the thread's work dirs replaced with `None`.
- **What this does not cover.** **No window has been opened.** No check here
  starts the binary, so nothing proves what a person sees: the sheet's rendered
  size against a narrow window, the composer staying uncovered, Escape reaching
  the sheet rather than the read-only editor inside it, or the cursor landing on
  the named line. Those stay open. The reader is also not offered in a full
  editor, so a regression there would show up as the editor opening — which is
  the correct behaviour, and therefore not a check.

### OMEGA-DELTA-0122 — A wait a person can type through

- **Upstream Zed:** the composer belongs to the thread view, and the thread view
  is built from a session. Until an agent connects there is nothing to type
  into, and a centred *"Loading…"* fills the pane. Connecting happens once, at
  startup, so the gap is a second nobody is looking at.
- **Omega, before this:** `OMEGA-DELTA-0115` put an executor selector in the
  composer and omega#112 made choosing one call `reset_onto_new_executor`, which
  tears the connection down and builds a new one. The gap stopped being a
  startup detail and became a place a person goes on purpose, several times an
  hour, immediately after deciding who they are about to talk to. The owner, on
  a live build: *"that loading thing is ok but you still dont show the input bar
  while its fucking loading. i want to be able to type while shit is loading.
  and move the loading indicator to inside the input bar like bottom left"*.
- **Omega now:** `ConversationView` draws its own composer for the whole of
  `Loading` — same box, same place, same type, focused, typable. The loading
  indicator is its status line, bottom-left. What is typed and not sent is
  moved into the real composer, caret included, the moment one exists. What is
  typed and *sent* is `OMEGA-DELTA-0170`'s: Enter always accepts, and the
  message queues as a visible pending turn that dispatches on connect.
- **The handover is the point, not the field.** A field whose contents are
  discarded when the thing it was waiting for arrives is *worse* than no field:
  a wait is a wait, but a lost sentence is a lost sentence, and the sentence
  lost here is the first one — the one that states the task, the longest one a
  person writes. `hand_loading_draft_over` runs where the thread view is built,
  restores the caret offset, and never clears the destination: if the real
  composer already carries a restored draft, the typed text goes after it rather
  than over it. Untidy, and correct — both texts are somebody's.
- **SUPERSEDED (2026-07-28): Send is live, and it auto-sends on connect.**
  This entry originally held that Send was disabled while connecting, that a
  `Chat` was refused with *"Not sent — still connecting. Press Enter again
  when this clears."*, and that nothing auto-sent on connect because a person
  who reached the wait by switching executor never watched the message go. The
  owner overruled both halves on a live build — he typed "hi" into a
  brand-new thread, pressed Enter, read the refusal, and said: *"refactor this
  'not sent' bullshit. never block user from hitting enter, if not connected
  just show a loading thing in the chat."* The replacement contract — Enter
  always accepts, the message becomes a visible pending turn naming the
  executor it will go to, and it dispatches automatically, in order, exactly
  once, on connect — is `OMEGA-DELTA-0170`. The visibility of the pending turn
  is what answers the switched-executor worry this entry used to answer with a
  second press.
- **A bare `Editor`, wearing the real composer's face.** Almost everything that
  makes `MessageEditor` the real composer — `@` mentions, `/` commands, skills,
  the queue — is a question asked of a session that does not exist yet. A
  mention resolved in the loading field would be a crease the field can hold and
  the handover cannot, so it would vanish silently at the one moment a person is
  not looking. The field takes plain text and its placeholder says so. It draws
  with `composer_editor_style`, now declared once and worn by both, so the two
  cannot drift into a reflow under the caret at handover.
- **The bottom-left is still empty once loaded.** `be27475c11` emptied it at the
  owner's request and `OMEGA-DELTA-0116` keeps it empty; the indicator is there
  *only* while connecting. `OMEGA-DELTA-0150` later removed external provider
  controls from the zero-base row. The row carries
  `flex_wrap` for the same reason the real one does: unwrapped, a narrow window
  pushes Send off the edge, and a control nobody can see is indistinguishable
  from one that was never built.
- **Neither gate is weakened.** `OMEGA-DELTA-0052` is untouched — this adds no
  dock, no status bar and no way out of zero base. `OMEGA-DELTA-0040` is
  untouched — the identity gate runs before any window opens, so a composer
  inside a window that does not exist yet is not reachable any earlier than it
  was. This delta is about the wait *after* the window, not the wait before it.
- **Enforced by:** `the_wait_for_an_executor_is_one_a_person_can_type_through`
  in `crates/omega_deltas`. Every assertion was watched failing against the
  source before the repair and against the repair reverted one piece at a time:
  a deleted `render_loading_composer`, a composer built and never drawn, a label
  where the editor should be, a deleted handover, a handover that is never
  called, a handover that restores text but not the caret, a handover that
  clears the destination first, a handover that sends, the indicator moved to
  the right of Send, an unwrapped row, and the shared style un-shared. The
  disabled-Send and refusal assertions were removed with the 2026-07-28
  supersession; their replacements live in `OMEGA-DELTA-0170`'s check.
- **What this does not cover.** **No window has been opened.** Nothing here
  proves what a person sees: that the loading composer is the same size and in
  the same place as the real one, that focus lands in it without a click, that
  the caret is visibly where it was left after the handover, or that the status
  sentence fits beside Send in a narrow panel. Those are omega#112's acceptance
  and they stay open. The loading field is also not persisted: text typed into
  it and then abandoned by closing the window before the connection lands is
  gone, because `draft_prompt` belongs to a thread and there is no thread.

### OMEGA-DELTA-0123 — An executor that cannot run here says so, and Omega still creates nothing of Exo's

- **Upstream Zed:** there is no executor selector. The agent panel picks between
  configured agent servers, and one that is not configured is simply not there.
- **Omega, before this:** `OMEGA-DELTA-0115` built the composer's selector on
  one rule — *a name appears only when it can run* — and shipped it with no
  second half. So a name that could not run was rendered as **nothing at all**.
  On this machine, which has an Exo checkout and a built `exo` binary, the menu
  read `Omega  Codex  Claude` and said nothing about the fourth name. The owner
  opened it, did not find Exo, and had to ask why it was missing.
- **Omega now:** `ready` is untouched, and `unavailable` is added beside it as
  its exact complement. Every one of the four names is either offered or
  explained; a disabled entry under a separator carries the short reason —
  `Exo — Exo has never been run here`, `Codex — not installed`. Nothing new
  became clickable.
- **The short list was right and incomplete.** `OMEGA-DELTA-0115`'s reason for
  the filter still holds in full: *"a selector offering a name that fails when
  it is clicked is worse than one that never offered it, because the person then
  has to work out whether they broke something."* The unnoticed cost is that the
  person has to work that out anyway, from a menu with no entry, with less to go
  on. Silence does not remove the question; it removes the answer. So both
  halves are kept, and they answer different questions — `ready` decides what
  may be **clicked**, `unavailable` decides what may be **read**.
- **The reason is in the label because a disabled entry has nowhere else.**
  `ContextMenu::select_index` registers a documentation aside only for an item
  that `is_selectable`, and `is_selectable` is `!disabled` for an entry — so an
  aside on a disabled entry never appears, and the `Info` icon the component
  draws beside one is an affordance for something that is not there. That is a
  fact about upstream code, which a rebase can change without touching anything
  of ours, so `a_disabled_menu_entry_still_cannot_be_selected` asserts it: if it
  stops being true, the long form becomes available and the label can go back to
  being one word.
- **The sentence is the type's, not the menu's.**
  `ExoLaneUnderivable::summary` gives each of the eight refusals its own line.
  That type's documentation already argued for exactly this — *"'Exo is not
  installed', 'that is the other Exo', 'Exo has never been run here' and 'Exo
  has four agents and Omega will not choose for you' are four different
  sentences and four different things for a person to do next"* — and then its
  only caller discarded the value and rendered the absence as nothing. The enum
  was built to be read and had no reader. A summary that could interpolate a
  path would grow one and become `Display` again, so it is `&'static str`.
- **Omega still creates nothing of Exo's, and that is the decision, not an
  omission.** The other way to make the name appear was to give it something to
  appear for: create the state root, the agent and the conversation the
  derivation looks for. Refused for three reasons. `.exo` is single-writer
  storage and Omega cannot prove no `exo serve` already holds the root it would
  write into. A root alone resolves nothing — `agent_slug` refuses an empty one
  — so creating a root means creating an agent, which means choosing a model
  binding and a provider secret, which is the owner's money and the owner's
  credentials. And `OMEGA-DELTA-0107` already settled the neighbouring question:
  Omega reads a durable log from a server the owner runs and starts none;
  writing into that server's storage is the same claim of authority through a
  different door. `omega_creates_no_exo_state_root_agent_or_conversation`
  enforces the absence, because "we did not add a write" stays true only while
  somebody is checking.
- **The other rejected option: offer Exo whenever the binary is present, and
  derive the lane lazily at connect.** That was measured before it was refused,
  and it fails twice. `exo acp` takes an existing agent *and* an existing
  conversation as arguments, so on a machine with a binary and an empty root
  there is nothing to attach to. And with a file-backed secret store the turn
  fails at `session/prompt` — not at connect — with `failed to decrypt secret
  payload`, because `initialize` and `session/new` succeed without ever touching
  a secret. A name offered on binary-presence would therefore fail *after* the
  first message was typed, which is the worst version of the failure
  `OMEGA-DELTA-0115` exists to prevent.
- **One answer to "is there a lane".** `exo_lane_resolves` is now the absence of
  an absence rather than a second cached read of the same two files. Two caches
  of one question is how a menu ends up offering Exo and explaining its absence
  in the same list.
- **Enforced by:** `a_name_that_cannot_run_is_still_named_and_still_not_offered`,
  `a_disabled_menu_entry_still_cannot_be_selected`,
  `there_is_one_answer_to_whether_an_exo_lane_resolves`,
  `every_exo_refusal_has_a_sentence_short_enough_to_read` and
  `omega_creates_no_exo_state_root_agent_or_conversation` in
  `crates/omega_deltas`, plus `every_name_is_either_ready_or_explained`,
  `omega_is_never_among_the_unavailable`,
  `an_agent_that_is_not_installed_says_so` and
  `exos_reason_is_carried_through_rather_than_reworded` in `crates/agent_ui`.
  Each was watched failing against a mutation of the thing it guards. Three of
  them were vacuous when first written and only mutation found it: one looked
  for `choice.name()`, which the *offered* loop renders too, and stayed green
  with the reason removed from the disabled label entirely; one looked for a
  refusal's variant name, which survives being folded into another arm; and one
  claimed to hold "not installed" and "installed and undrivable" apart when
  the second case cannot occur at all — `ready` offers a name only when it is
  detected **and** drivable, so for Codex and Claude that arm is unreachable.
  The arm is kept, because it is the truthful answer if `DRIVABLE_AGENT_IDS`
  ever stops naming one of the four, and the check now asserts that
  reachability rather than pretending to exercise it.
- **What this does not cover.** **No window has been opened**, so nothing here
  proves the rendered menu: that the separator and the disabled entries draw,
  that the label fits the popover's width, or that the owner now sees `Exo —
  Exo has never been run here` where there was blank space. **And this does not
  make Exo run on this machine.** No lane resolves here, and the composer now
  says so instead of saying nothing; what a person must do about it is
  `docs/exo/` in the `openagents` repository. The lane file's schema still
  carries no secret-store fields, so `AgentServerCommand { env: None }` hands
  the `exo acp` child whatever environment Omega was launched with — which for
  a Dock or Finder launch is neither `EXO_SECRET_BACKEND` nor
  `EXO_MASTER_KEY_PATH`. That gap is measured and written down and is not
  repaired here.

### OMEGA-DELTA-0117 — The adapters a person can switch to are already started, and one of them is never started at all

- **Upstream Zed:** an external ACP agent is started when a person picks it from
  the agent panel, once, and stays picked. There is no control that re-attaches a
  live conversation somewhere else, so there is no repeated start to preload
  against, and `LocalRegistryNpxAgent`'s `npm exec --yes` sits where the reader
  already knows they asked for something.
- **Omega, before this:** `OMEGA-DELTA-0115` gave the composer a control that
  switches between Omega, Exo, Codex and Claude, and switching works by
  rebuilding the connection — `reset_onto_new_executor` drops the cached entry so
  `connect` runs again. So every switch paid a full adapter start: `npm exec
  --yes` resolving an npx package, Node booting it, and the ACP `initialize`
  handshake, in front of somebody who had just clicked a menu item.
  `OMEGA-DELTA-0114` had already bounded and named that wait. Naming a wait does
  not shorten it.
- **Omega now:** once a connection has been established, the adapters for the
  *other* offerable executors are started in the background and held. A switch to
  one of them takes the started adapter instead of starting one.
- **Why:** the owner, switching executors in a running build: *"that acp shit is
  ideally preloaded in the background so user doesnt have to sit there waiting
  for that bullshit"*.
- **What is warm is a process, and the reason is measured rather than assumed.**
  The obvious thing to preload is the npx *package*, and it is what
  `OMEGA-DELTA-0114`'s prose points at — a cold resolve is tens of megabytes. It
  is also already warm on any machine that has connected once: npm keeps the
  resolved package under `<cache>/_npx`, that cache is Omega's own directory, and
  it survives launches. Measured on the owner's machine against his real caches
  and the exact command `LocalRegistryNpxAgent` builds, timed to the `initialize`
  reply — which is precisely where `CustomAgentServer::connect` returns, since
  `agent_servers::acp` does `initialize` and nothing else: a **cold** resolve of
  `codex-acp` costs **4.60s**, and every resolve after it costs **0.55–0.67s**
  (median 0.63s over five runs; `claude-acp` 0.55–0.60s, median 0.59s, after a
  cold 4.25s). Preloading the package therefore buys a first launch several
  seconds and buys the owner's actual complaint — the second switch, and the
  third — nothing. What remains in that 0.6s is npm's startup, Node's boot and
  the handshake: work, not bytes, and uncacheable. The only way not to pay it at
  switch time is to have paid it already, so what is held is the whole
  `Rc<dyn AgentConnection>`.
- **What it costs when nobody uses it, stated rather than elided.** A warmed
  adapter is two live processes, the `npm exec` wrapper and the Node child.
  Measured idle, immediately after `initialize`: **~100 MB RSS each, ~200 MB the
  pair**, per warmed executor. With both agents installed one is already
  attached, so the standing cost is one warm adapter, ~200 MB, for the one menu
  entry the person has not chosen; on Omega's own loop or an Exo lane both are
  warm and it is ~400 MB. Somebody who never opens the menu pays that for
  nothing. `WARM_LIFETIME` is what stops them paying it forever.
- **Exo is never warmed, and that is the whole of the interaction with
  `OMEGA-DELTA-0107`.** That delta took route A on omega#104: Omega reads an
  `exo serve` the owner already runs and starts none, because a second process
  pointed at one `.exo` root is the write interleaving that makes a fork a copy
  of a history that never existed. "Warm everything the selector offers" is a
  sentence with Exo in it and the convenience is real, which is exactly how a
  decision gets reversed by accident instead of by argument. `warmable` filters
  on having an ACP adapter of one's own, which excludes Exo and Omega for the
  two different right reasons — a lane that must not be started, and a loop
  compiled in with nothing to start — and the check holds the `serve` spellings
  against the module's literals the way 0107 does.
- **A preload cannot be seen unless it worked.** It carries no loading-status
  channel: `watch::Sender` has exactly one holder and `CustomAgentServer::
  connect` installs it into a slot on the store keyed by agent id, so a preload
  carrying one could take the channel a person's own connect is ticking on and
  leave them at the silent unbounded `Loading…` that `OMEGA-DELTA-0114` exists to
  have removed. Passing `None` means `connect` never touches that slot. A failed
  preload records nothing — not `record_unreachable`, which is what puts **Run on
  Omega's Own Loop** on screen and would be offering a person a way past a
  failure they never saw; and not `run_on_omegas_own_loop` itself, which only a
  person may call and whose sites 0114 already counts across the tree. The person
  who then picks that executor gets the ordinary attach: the ticking label,
  `ADAPTER_START_TIMEOUT`, and the same failure they would have had if none of
  this existed.
- **There is still exactly one place an adapter is started.** `start_adapter_
  silently` is a wrapper over the same `attach` a person's own connect uses, not
  a second path beside it, because a second start would be the *unbounded* one —
  reintroduced in the one place nobody is watching, precisely because nobody is
  waiting on it.
- **A warm connection expires and is checked, because nothing else will notice.**
  A held connection is the one connection whose death is observed by nobody:
  `AcpConnection`'s wait task fans `LoadError::Exited` out to that connection's
  *sessions*, and one in reserve has none. So `take_warm` refuses on two grounds
  — older than `WARM_LIFETIME`, or `agent_server_process_has_exited` — and both
  refusals **end the process by name** rather than dropping the last `Rc` and
  hoping, since `Drop` runs only when every owner is released and 200 MB of Node
  with no owner is the failure this is supposed to avoid. Handing over a stale
  handle is worse than being slow: a slow start is a bounded wait with a label,
  a dead handle is a failure that arrives after the person has typed, from
  somewhere the composer cannot explain.
- **A warm attempt still running is left alone.** Not awaited, because that would
  put a person behind a start with no channel and no tick; not cancelled, because
  dropping the task drops a `Child` that nothing else kills — `util::process::
  Child` has no `Drop` on Unix — and a cancelled preload would leak the very
  process it exists to save. The person's own attach runs beside it, and the warm
  one is taken by the next switch or expires.
- **When it starts is a decision, not a default.** The trigger is the router's
  `connect`, after `publish_active_router`. A connection exists because a panel
  asked for one, so a window is up; and the connect the person was actually
  waiting on has returned, so the preload is not racing it for the same npm, the
  same registry and the same CPU. `WARM_START_DELAY` adds two seconds on top,
  because a connect returning is not the window being finished with it — the
  session opens next and the transcript draws. The call sites are counted rather
  than trusted, for the same reason `run_on_omegas_own_loop`'s are: nothing in
  the function distinguishes a preload that follows a connection from one wired
  into startup, and the startup version spends a person's first paint on adapters
  they never asked for, which moves the wait instead of removing it.
- **Enforced by:** `a_preload_never_starts_an_exo_server`,
  `a_preload_starts_an_adapter_only_through_the_bounded_door`,
  `a_warm_connection_is_refused_rather_than_handed_over_stale` and
  `a_preload_begins_only_once_a_connection_exists` in `crates/omega_deltas`,
  plus `the_exo_lane_is_never_warmed`, `omegas_own_loop_is_never_warmed`,
  `the_two_adapters_are_what_gets_warmed`,
  `the_attached_executor_is_not_warmed_again`,
  `an_exo_lane_leaves_both_adapters_worth_warming`, `a_bare_machine_warms_nothing`
  and `nothing_is_warmed_that_the_selector_does_not_offer` in `crates/agent_ui`.
  Each was watched failing first, against fifteen mutations: a `warmable` that
  stopped filtering on the adapter id and so offered Exo and Omega, one that
  stopped excluding the attached executor, one that warmed names the selector
  never offered; a module that named `exo serve`, one that reached for the Exo
  lane's connection, one that started adapters outside `start_adapter_silently`,
  one that built its own `CustomAgentServer`, one that carried a loading-status
  channel, one that called `record_unreachable`, a `start_adapter_silently` given
  a channel; a `take_warm` with its expiry test disabled, its liveness test
  disabled, and one of its two reclaims deleted; and a trigger moved above
  `publish_active_router` or added to the attach.
- **Two of these checks were themselves falsified before they were kept, and both
  had passed.** Disabling the expiry test left the check green, because it looked
  for the token `WARM_LIFETIME` and the constant was still named by the log line
  *inside* the block that had just been switched off — a check for a mention is
  not a check for a rule, so it now holds the whole condition. Deleting one of
  the two `end(&connection)` reclaims also left it green, because `contains`
  was satisfied by the other one; it is now counted, once per refusal. This is
  the whole argument for watching a check fail rather than watching it pass.
- **What this does not cover.** **No window has been opened, and no check starts
  a process**, so nothing here proves what a person sees. The measurements above
  are of the adapter start in isolation, driven through the exact command
  `LocalRegistryNpxAgent` builds against the owner's own caches — not of a switch
  in a running Omega. What a switch pays *besides* the adapter is untouched and
  unmeasured here: the native connect, the ACP registry's own load, and
  `session/new`, which is a further 0.15–0.60s warm on this machine and cannot be
  preloaded at all, because a warm session would be a conversation nobody asked
  for. The expiry is asserted as a rule, not exercised against a connection that
  actually went stale; the liveness answer is `try_status` on a reaped child and
  therefore says nothing about an adapter that is alive and wedged.
- **Adapter processes can already outlive Omega, and this adds one more of
  them.** `util::process::Child` has no `Drop` on Unix and children are started
  in their own session, so an adapter is reclaimed by `AcpConnection::drop`,
  by `end_agent_server_process`, or not at all. That is the existing situation
  for the attached adapter; warming does not create the class, it adds a member
  to it. `sweep_expired` bounds the in-process half — an expired connection is
  ended at the next connect rather than waiting for somebody to ask for the
  executor nobody chose — but a warm adapter belonging to a closed window still
  lives until then, and one belonging to a crashed Omega is not reclaimed at
  all.

### OMEGA-DELTA-0118 — Zero base's threads sidebar is its own, and a thread reopens on the executor that made it

- **Upstream Zed:** thread history lives in the workspace sidebar, beside the
  projects. There is no mode in which the workspace is absent, so there is no
  second place for it to live.
- **Omega, before this:** the agent panel's `…` menu carried an entry reading
  "Toggle Threads Sidebar", bound `cmd-alt-j`, and in zero base pressing it did
  nothing at all. The owner, testing a live build: *"this 'Toggle Threads
  Sidebar' does nothing when i click on it but i want it. i want threads sidebar
  to see historical chats."*
- **What it actually was.** Not an unimplemented feature and not a zero-width
  panel. The entry named `multi_workspace::ToggleWorkspaceSidebar`, and
  `multi_workspace` is outside zero base's admitted set, so `App::set_action_gate`
  refused it **before any listener ran**. It was a control that is drawn and
  denied, which is the exact failure `OMEGA-DELTA-0053` names about the
  status-bar dock button — "if the gate refuses an action, its control must not
  be drawn" — reappearing on a surface that mode still renders.
- **Admitting the namespace would have been the wrong repair, and this is the
  part that decided the shape.** `multi_workspace`'s sidebar is the project
  switcher: projects, workspaces, thread import, a folder picker, `NewThread`,
  and windows it can move a project into. It hangs off `MultiWorkspace`, which
  is *above* the `Workspace` that `OMEGA-DELTA-0053` seals, so nothing about the
  seal contains it. One name added to `ADMITTED_NAMESPACES` would have put the
  editor's whole navigation back inside a mode whose premise is that it is
  absent — `OMEGA-DELTA-0052` weakened by one line in a constant, which is how
  that kind of thing always goes. So zero base gets a surface of its own,
  reachable through `agent::ToggleThreadsSidebar`, and the `agent` namespace was
  already admitted: **`ADMITTED_NAMESPACES` and `ADMITTED_ACTIONS` are byte-for-byte
  unchanged by this delta.**
- **The menu entry names the action that works in the window it is in.** In
  the legacy editor surface it named `multi_workspace::ToggleWorkspaceSidebar`,
  because there the project switcher was the right answer and not refused; the
  default surface names the panel's own. omega#161 removed the editor surface
  (and with it the workspace sidebar registration), so the panel's action is
  the one a person can reach.
- **Nothing was deleted, and `cmd-alt-j` still means what it meant.** The
  editor's binding is untouched in all three shipped keymaps. What was added is
  a narrower context, `AgentPanel && ZeroBase`, which is why the panel's key
  context now carries `ZeroBase` when the mode is on. `OMEGA-DELTA-0048`'s rule
  holds: no shipped binding is removed, and `keymaps_name_no_deleted_action`
  stays green.
- **Every refusal this gate has ever made was silent, and that is why "nothing
  happened" rather than "no".** `report_refusal` resolved its window with
  `active_window().downcast::<Workspace>()`. Every Omega window is opened with a
  `MultiWorkspace` root wrapping the workspace, so that downcast answered `None`
  on every machine, the `let else` returned, and **no zero-base refusal toast has
  ever been shown**. `OMEGA-DELTA-0053` records the owner pressing a denied
  status-bar control and reporting that nothing happened; that was this. The
  mode's entire safety argument is that hiding a surface is safe *because*
  something refuses it out loud, and the out-loud half was switched off by a type
  parameter. It now reads the workspace through the `MultiWorkspace` root, and
  falls back to a bare `Workspace` root so a test window still gets its toast.
- **What the sidebar shows.** Past conversations, newest first, each row a title,
  a compact age — `3s`, `4m`, `2h`, `5d` — and the executor that ran it when that
  is not Omega's own loop. The age is not decoration: omega#100 found in the
  `@`-mention list that threads are named by a summarisation model, so two
  conversations routinely carry the same title, and the owner's words there were
  "if the last two chats have same name i cant tell the difference". The same
  list, ordered by the same field, needs the same answer. Nothing is drawn for
  Omega's own loop, which is `OMEGA-DELTA-0021`'s convention everywhere else in
  this crate: naming Omega inside Omega is noise, and absence already means the
  default.
- **Drafts are not history.** A draft is a thread whose first message was never
  sent — no session id, no transcript. The person asked for historical chats, and
  a list whose top row is the empty composer he is looking at buries what he
  opened it to find. Archived threads are excluded for the reason archiving
  exists. The list is bounded at 200 rows, which bounds the drawing rather than
  the query: the store is already in memory.
- **A thread reopens on the executor that recorded it, never on the one
  selected.** A session id is not portable — it names a conversation inside the
  agent server that created it — so resuming a Codex session on Claude's
  connection reaches an adapter that has never heard of it and answers `no
  rollout found for thread id ...`. That sentence is about a rollout file; the
  question was "can I have my chat back". `load_agent_thread` is therefore handed
  `metadata.agent_id`, which means the ordinary cross-executor case simply works:
  a Codex thread opened while Claude is selected opens **on Codex**.
- **The one case that cannot work refuses in a sentence, before it reaches an
  adapter.** When the recorded executor cannot run on this machine at all — it
  is on `omega_executor_selector::unavailable_here`, or an unrecognised adapter
  id is not registered — the row is drawn muted and clicking it says so in the
  sidebar. The refusal is rendered where the person is already looking rather
  than as a toast, which is the same judgement the bullet above records about
  notifications in a sealed window.
- **Amended, omega#131: the row says it unclicked, and the sentence stops
  shouting.** The refusal above was the whole answer and it arrived too late.
  The owner opened a fresh install with no Codex, clicked several rows in a row,
  and got the same yellow paragraph each time: *"You're showing me histories I
  click on and it's a yellow warning."* Every one of those rows already held the
  reason — `rows` was handed `OMEGA-DELTA-0123`'s list and used it — and nothing
  on the row said so. A list whose dead rows are indistinguishable from its live
  ones is a list of dead ends found one click at a time, and on a fresh install
  most of the list is dead. So a row that will refuse now carries
  `Codex — not installed` where a row that opens carries `Codex`, and the
  refusal renders in `Color::Muted`.
- **The mark is the composer selector's treatment, not a second one.** That menu
  greys out a name it cannot offer and appends the reason after an em dash;
  `OMEGA-DELTA-0123` records why the reason is in the label there rather than in
  an aside. The row does the same thing with the same reason from the same list,
  so the two places a person meets this fact look like one window. It uses
  `SelectableExecutor::name` rather than `selector_name` because the row's own
  executor label and the refusal sentence already spell `Claude` — a row reading
  `Claude Code` above a sentence reading `Claude` would be a third answer
  invented by the shorter of the two. No icon: the menu draws none on a disabled
  entry, for the reason `OMEGA-DELTA-0123` gives about affordances with nothing
  behind them.
- **Both halves come out of one call.** `reopen_refusal` returns the mark and
  the sentence together, because they are one fact told at two lengths. Computed
  apart they could disagree, and a row marked live that refuses — or marked dead
  that opens — is worse than the unmarked row either was meant to repair.
- **The colour, and why it is not a warning.** A machine that does not have
  Codex is not a fault, and the person did not cause it; once the row states the
  case, the click produces the long form of something already read. `53fa7902`
  moved two first-run notices off `Color::Warning` for exactly this, and its
  sentence holds here: a warning colour for an ordinary state is how a person
  learns to read past warnings. The rows are not hidden. Hiding somebody's
  history to avoid marking it answers the question by deleting it.
- **The reason in that sentence is `OMEGA-DELTA-0123`'s, not a second one.**
  That delta had just made the composer's selector explain every name it cannot
  offer — `Codex — not installed`, `installed; Omega hosts no adapter for it`,
  the Exo lane's own refusal — and `unavailable` is where those live. The first
  version of this sidebar wrote its own remedies, and a row reading "Codex is
  not installed" two inches above a menu reading "installed; Omega hosts no
  adapter for it" would send somebody to install what they already have. So the
  list is passed in rather than recomputed, and what this adds is only the part
  the menu has no reason to know: the transcript is inside that executor, so no
  other one can produce it.
- **It cannot push the composer around, structurally.** The sidebar is an
  absolutely positioned child appended last to the panel's flex column, so it
  takes part in no other element's layout. `OMEGA-DELTA-0105` records that the
  composer row already has to wrap so a narrow dock does not clip Send; a sidebar
  that took width out of that row would have made the clip it protects against
  more likely, not less. Closing is the same action again, `cmd-alt-j`, or the
  close control in the sidebar's own header.
- **Enforced by:** `zero_bases_threads_sidebar_is_its_own_and_reopens_by_executor`
  in `crates/omega_deltas/`, and ten unit tests in
  `crates/agent_ui/src/omega_threads_sidebar.rs` covering the order, the age, the
  draft and archive exclusions, the executor naming, the bound, both refusals,
  and the mark each refusing row carries before it is clicked — including that
  the sidebar spells no reason of its own. Each assertion in the delta check was
  watched failing against the source with the corresponding edit reverted.
- **The amendment's test found the fixtures were testing the wrong thing.**
  `a_row_that_cannot_be_reopened_is_marked_before_it_is_clicked` asserts that a
  native row carries no mark, and it failed with
  `omega — not registered in this window`. Every fixture in the module spelled
  the native agent id `"omega"`, and `Agent::from` recognises
  `agent::OMEGA_AGENT_ID`, which is `"Omega Agent"` — so what those tests called
  a native thread was an unregistered foreign adapter that happened to answer
  `None` to the only question any of them asked it, including
  `omega_is_not_named_on_a_row_and_another_executor_is`, whose whole subject is
  that Omega is not named. The fixtures now read the constant.
- **What this does not cover.** **No window has been opened.** Nothing in this
  repository starts the binary, so the drawing is unproved: the overlay's width
  against a narrow dock, the rows against a long title, and the refusal line's
  wrapping have not been photographed. It also says nothing about keyboard
  navigation *inside* the list — the sidebar is click-driven and does not take
  focus, deliberately, because stealing focus from the composer to browse
  history is the trade the owner has not asked for. And it does not make the
  sidebar's open state persist: closing the process closes the sidebar.

### OMEGA-DELTA-0124 — A thought's own title heads its block, and is not left underneath

- **Upstream behaviour.** Upstream Zed heads every thinking block with the
  literal word *"Thinking"*, an icon, and a disclosure. The model's own title
  line renders inside the block, in bold, as the first thing under the header.

- **What the owner saw.** Testing a live build, with three thoughts in a row on
  screen: *"rather than showing 'Thinking' each time, where it says Thinking I
  want it showing the actual thought, not on a separate line, and using the same
  font color/size that 'Thinking' is now"*. A run of five thoughts reads as five
  identical labels with the real content indented beneath each — the header
  costs a row per thought and carries no information, and the one line that
  would have told you what the thought was is the line below it.

- **The obvious implementation is the defect, and it shipped.** Reading the
  title in `render_thinking_block` and putting it in the header is a one-line
  move, and it was made. Every thought then rendered its title **twice**: muted
  in the header, and still bold in the body underneath. The cause is that the
  header and the body draw from the *same* `Entity<Markdown>` — the one
  `ContentBlock::markdown()` hands out, which is shared with the search bar, the
  selection, the context menu and the copy path. There is nowhere in the
  renderer to hide the title from the body. It was reverted in `2b6e004d1c`.

- **Two ways out, and why this is the one taken.** The alternative was to teach
  `MarkdownElement` to skip the root blocks that are titles, keeping one entity.
  That is attractive — no second parse, no cache, and source offsets stay
  identical, so search highlights and selection need no thought at all. It was
  rejected on a case it cannot express: a title and its prose in **one
  paragraph** (`**Title**\nprose`, no blank line between) is one root block, and
  a renderer that skips blocks can only take all of it or none. Taking all of it
  deletes the prose; taking none renders the title twice, which is the defect.
  Removing the title *line* has no such case, because a line is the unit the
  model actually writes titles in.

- **So the body is a second markdown, derived.** `split_thought` takes a block's
  source and returns the titles it found and the source with those lines
  removed. `ThoughtView` holds the result and the source it was derived from,
  and lives in `EntryViewState` — **not** in the renderer, which holds `&self`
  on every frame and can neither build an entity nor memoise one. Deriving there
  would construct a `Markdown` per frame per visible thought, and a thought
  streams token by token. `EntryViewState::sync_entry` already runs once per
  arriving update, compares the source, and does nothing when it has not moved.

- **Emphasis marks a title, not position.** A block can hold more than one
  thought, and the owner saw it: two titles under one lightbulb, the second
  reading as a subheading of the first. His call — *"youll need to handle if
  theres 2 thoughts in a single 'thinking block' in which case u can just show
  that same lightbulb line twice its ok"* — so the header draws one muted row
  per title. The second thought is the second *emphasised* line, not the second
  line, so every line is asked rather than the first one.

- **What is deliberately not hoisted.**
  - A **streaming title has no closing marker** — `**Search` arrives with
    nothing after it and stays that way until the next token. It counts, because
    waiting for the close would leave the header blank for the whole time a
    thought is being written, which is the only time anybody is watching it.
  - A **closed `**` run must own its whole line.** `**Note** that the file is
    gone` is a bold lead-in inside prose; hoisting it would put half a sentence
    in the header and delete it from the paragraph it belongs to.
  - A **`#` inside fenced or indented code is a comment.** Fences are tracked,
    and a line four spaces in is an indented code block.
  - `#Title` and `#######` are not headings, by the same rule that stops a line
    of prose being taken out of the body.

- **A thought with no title keeps every word.** The header falls back to the
  first non-empty line, and — unlike a title — that line **stays in the body**.
  A title is a label for the paragraph under it and says nothing that paragraph
  does not; a first sentence *is* the thought, and lifting it out to avoid an
  echo would delete content. `titles` and `preview` are separate fields for
  exactly this reason: only the first kind is removed, and the check can tell
  them apart. When even that is empty — a block that has arrived as `**` and no
  words — the row falls back to `UNTITLED_THOUGHT_HEADING`, the word the header
  used to always say, so a row is never blank mid-stream.

- **Search follows what is drawn.** A highlight is an offset into a source, and
  the rendered body has lines removed from above it, so highlighting the block's
  own markdown while rendering the derived body would land on the wrong words.
  `collect_markdowns` takes the rendered body. The cost is that a title is no
  longer found by thread search; it is on screen in the header, which is not a
  markdown entity and cannot be highlighted.

- **Long titles truncate.** `min_w_0` on the row and `truncate` on the label,
  rather than wrapping to a second row or pushing the disclosure off the end.

- **Enforced by:** `a_thoughts_title_reaches_the_header_and_is_not_left_in_the_body`
  in `crates/omega_deltas`; and
  `a_title_never_appears_in_both_the_header_and_the_body`,
  `a_thought_always_has_a_heading`,
  `a_thinking_block_splits_into_its_titles_and_the_rest` and
  `a_hash_inside_code_is_a_comment_and_stays_where_it_is` in
  `crates/agent_ui/src/entry_view_state.rs`
  (`cargo test -p agent_ui --lib entry_view_state`).

  The first of those is the one that matters: it states the property as an
  *idempotence* — split a body that has already been split and there is no title
  left to find — so anything that puts a title in the header while leaving it in
  the body fails, whatever route it took. Its first version asked each body line
  on its own and went red on a `# comment` inside a fenced shell snippet, which
  is not a title and is in no header; running the whole splitter is what tells
  those apart, and a check that could not would have been satisfied by hoisting
  the comment out of the code.

- **What this does not cover.** **Nobody has seen it drawn.** The split is
  proved as text, and the renderer is proved to read the split rather than the
  chunk — which is the defect that shipped, and removing either line fails on
  exactly that. What has not been proved is the drawing: two lightbulb rows
  under one disclosure, a title truncating at the window's edge, and the row
  that shows while a title is still arriving have not been rendered in a window.

### OMEGA-DELTA-0125 — The thread header's `…` menu does what it says, or is not there

- **Owner, on a live build:** *"literally nothing in this top right menu does
  anything when i click on it. if its easy to reenable those things to actually
  work, do it, otherwise hide the menu."* Eight entries: Open Thread as Markdown,
  Add Server…, Install New Servers…, Skills, Open Project Rules (AGENTS.md),
  Profiles, Settings, Toggle Threads Sidebar.
- **One symptom, three causes, and one entry that was never broken.** Every entry
  was read on its own, because treating them as one bug is how six of them get
  "fixed" by a change that repairs two.

**Cause one: the refusal never reached a screen — and this delta does not own
it.** `report_refusal` in `crates/omega/src/omega_zero_base_ui.rs` asked the active
window for `downcast::<Workspace>()`. **A `Workspace` is never a window root.**
Every Omega window's root view is `MultiWorkspace`, so the `let ... else`
returned on every window that has ever existed, the toast was never built, and
each refusal reached `zed.log` and nowhere else. `OMEGA-DELTA-0048` makes zero
base safe by *refusing at dispatch* what it does not render, and the refusal's
whole visible half was a no-op: a refused menu entry, key press or palette
command was indistinguishable from a broken button.

This lane found it independently and repaired it, and so did the threads-sidebar
lane; theirs landed first as `OMEGA-DELTA-0118`, handles a `Workspace` root as a
fallback as well, and is enforced by
`zero_bases_threads_sidebar_is_its_own`. **This delta's version was discarded on
the rebase** rather than merged beside it. Two repairs of one line is how one of
them silently stops being the one that runs, and two checks over it is how one
ends up guarding a spelling nobody kept. It is recorded here because it is half
of *why the owner saw nothing*, and a reader of this entry who did not know that
would conclude the menu entries below were the whole story.

**Cause two: an invisible success.** `Open Thread as Markdown` and both AGENTS.md
entries dispatched nothing at all. They called a handler that opened an item in
the centre pane — `add_item_to_active_pane` for the thread, `open_abs_path` for
the rules files — and `OMEGA-DELTA-0053` draws no centre pane once zero base is
sealed. The buffer opened, took the composer's focus with it, and landed
somewhere with no pixels. This is `OMEGA-DELTA-0119` exactly, three entries it
did not reach.

`OMEGA-DELTA-0174` later generalized the editable half of this rule: every
default-surface action that intentionally creates or activates a Workspace
center item first calls the shared user-open reveal boundary. This delta's
reader remains intentional for thread Markdown and rules-file peeks; it is not
a substitute for making explicitly editable opens visible.

- **Omega now:** all three open the reader `OMEGA-DELTA-0119` built —
  `crates/agent_ui/src/omega_file_peek.rs`, a read-only sheet in the workspace's
  modal layer, which `MultiWorkspace` renders outside the seal. Two new entry
  points into the *same* sheet, `open_file` for a path this window already
  resolved and `open_text` for a thread rendered to markdown, which is on no
  disk. A second reader for the same job would be two surfaces to keep
  read-only, two to keep out of the composer's layout, and two to repair the
  next time one silently stops drawing.
- **`OMEGA-DELTA-0052` and `0053` are not weakened.** No dock, no pane, no tab,
  no way out. The sheet is absolutely positioned, takes part in no layout, and
  cannot clip or push the composer; its height is bounded so it covers the
  transcript and not the composer. Dismissing it leaves the window zero base
  already had.
- **The reader's failure state is reachable by an ordinary click.** The menu
  offers Open Project Rules only when AGENTS.md exists, and `open_project_rules`
  resolves the path again *without* that check, so a file deleted in between
  lands in "No file at …" rather than in silence. A small gap, and the one worth
  drawing for: a repair whose own failure mode is a dead click is the same bug
  wearing a different cause.

**Cause three: refused, and rightly.** `Add Server…` → `omega::OpenSettingsAt`.
`Install New Servers…` → `omega::Extensions`. `Skills` → `agent::ManageSkills`,
admitted, which then dispatches `omega::OpenSettingsAt`. `Settings` →
`agent::OpenSettings`, admitted, which then dispatches
`omega::OpenSettingsPage`. Zero base refuses the whole `omega` namespace.

- **Hidden, not admitted, and the second hop is why the question was close.**
  The settings surface opens its own OS window, so it does not need the centre
  pane and admitting it looked cheap. It was refused for two reasons. That
  window carries controls — *Open Current Settings File*, *Open Keymap* — that
  close themselves and open a buffer in **this** workspace, which is the sealed
  centre; admitting it would import the identical dead click one level down,
  behind a surface a person had to travel to. And `omega_zero_base`'s admitted
  set already records the decision in as many words: admitting the `omega`
  namespace "would admit the extensions and settings surfaces with it, and
  section 4 of the design note records that those reach nothing in Omega today".
- **Omega now:** those four are not built in a sealed zero base. The `Context`
  header moves to the AGENTS.md entries so nothing is left with a heading over
  it and nothing loses one.

**`Profiles` was never broken.** It is `agent::ManageProfiles`, which the gate
admits, and it opens a modal — and the modal layer is rendered by
`MultiWorkspace` outside the seal, which is the same reason the command palette
still works in the mode. It stays, and the check below fails if a later change
hides it.

**`Toggle Threads Sidebar` was cause three too, and its own lane fixed it.** It
dispatched `multi_workspace::ToggleWorkspaceSidebar`, refused, dead —
the same shape as the four above. `OMEGA-DELTA-0118` landed while this lane was
open and gave it `agent::ToggleThreadsSidebar` in zero base and a sidebar of its
own to toggle. This delta touched none of it and simply classifies the result:
the entry now reaches an admitted action, so the invariant below says it is
offered, and it is. That is the outcome the table is *for* — an entry whose
surface became reachable stops being hidden, and the check is what asks the
question rather than leaving it to whoever next reads the menu.

- **Enforced by:** `a_thread_menu_entry_lands_somewhere_a_person_can_see` and
  `a_menu_entry_that_opens_a_buffer_opens_the_reader_instead` in
  `crates/omega_deltas`. Cause one's check is `OMEGA-DELTA-0118`'s, not a second
  copy here.
- **The defect class, not the eight instances.** The first check holds one
  invariant: **an entry is offered in a sealed zero base exactly when the action
  gate admits what its click finally reaches.** It asks
  `omega_zero_base::admits_action` rather than keeping a second copy of the
  admitted set — `omega_zero_base` is a dev-dependency of the delta crate for
  that reason — and it fails in *both* directions, so an entry hidden for a
  reason that has since expired is caught as loudly as an entry shown while
  refused. "Finally reaches" is load-bearing: a check that read the action the
  menu names would have passed `Skills` and `Settings`, which are two of the four
  the owner was complaining about, so the second hop is asserted in the source of
  `manage_skills` and `open_configuration` too — if either stops landing in the
  `omega` namespace, the hiding is no longer justified and the test says so.
  A census of every `.action("…"` and `.entry("…"` label in the menu fails on an
  entry nobody classified, which is what stops the next one arriving unexamined.
- **Watched failing.** Each assertion was run against a mutation of the thing it
  guards: the guard reverted to `is_active`, the guard removed entirely, each of
  the four entries moved back outside it, `Profiles` moved inside it, the reader
  call moved after the pane opener in all three funnels, the seal check dropped
  from `open_file` and `open_text`, an unclassified entry added to the menu, an
  entry deleted from it, `omega::Extensions` added to the admitted set, and
  `manage_skills` stopped from dispatching into the `omega` namespace. Two more
  were watched failing before this delta's own copy of the refusal repair was
  discarded in favour of `OMEGA-DELTA-0118`'s: the downcast put back to
  `Workspace`, and the toast removed.
- **What this does not cover.** **No window has been opened.** Nothing here
  proves the rendered result: that the sheet draws over the transcript and not
  the composer for a thread the length of the owner's, that the refusal toast is
  legible above a zoomed panel, or that the shortened menu still reads as a menu.
  The `alt-cmd-L`, `alt-cmd-C` and `alt-cmd-P` **key bindings for the hidden
  entries still exist** — zero base deletes no keymap binding, by
  `OMEGA-DELTA-0052`'s reasoning — so those keys still refuse, and after this
  change they refuse *audibly*. That is the intended end state and not a
  leftover.
### OMEGA-DELTA-0126 — A lane names the key that opens its root, and an ordinary Exo turn is ordinary

- **Upstream Zed:** an agent server is launched from settings, and its
  environment is whatever those settings say. There is no Exo, no state root and
  no secret store, so none of this exists to get wrong.
- **Omega, before this:** `connect_configured_lane` built its child with
  `AgentServerCommand { env: None }`, and `env: None` does not mean "no
  environment" — it means **inherit whatever Omega was launched with**. Exo
  encrypts the provider credentials in its state root and learns which key opens
  them from `EXO_SECRET_BACKEND` and `EXO_MASTER_KEY_PATH`. So a file-backed
  root worked from a terminal that had exported them and failed from the Dock,
  which has neither — and it failed at `session/prompt`, **after** the person
  had typed and sent their first message, with `failed to decrypt secret
  payload`: a sentence that reads like a corrupt state root and is not.
- **Omega now:** the lane carries the store. `ExoSecretStore` is a sixth field
  on `ExoLaneConfig` and on `DerivedExoLane`, `openagents.omega.exo_lane.v1`
  gained two optional keys (`secret_backend`, `master_key_path`), and both spawn
  sites — the `exo acp` child and the five observation commands — are launched
  with `child_env()`. The same root now behaves the same way from a Dock launch
  and a shell launch, which it did not before.
- **The store cannot be read off the root, so the lane has to carry it.** Exo's
  secret file is the same AES-GCM envelope whichever backend holds the key:
  `{metadata, secret: {algorithm, nonce, ciphertext}}`, byte-identical for the
  keychain and for a file. There is no field to look at. That is why this is a
  schema change and not a smarter reader — the information genuinely is not
  there, and any reader that appeared to find it would be guessing.
- **A derived lane finds the store from the one trace the file backend leaves.**
  Exo's own `default_master_key_path` writes `$XDG_CONFIG_HOME/exo/master.key`,
  else `$HOME/.config/exo/master.key`, and nothing but the file backend ever
  writes it. `secret_store` takes a person's exported `EXO_SECRET_BACKEND`
  first, then `EXO_MASTER_KEY_PATH` alone, then that file if it is there, then
  nothing. **Nothing** — not `AppleKeychain` — because the two are the same
  behaviour today and stop being the same the moment Exo's default moves, and a
  lane that names a backend Exo did not pick is a lane that breaks on an Exo
  upgrade for a reason nobody can see.
- **A backend Exo does not have is refused at the lane file, not at the model
  call.** `ExoSecretStore::parse` is closed over Exo's two, so a typo yields no
  store and a logged warning rather than an argument Exo rejects seconds later
  inside somebody's first turn.
- **`ExoSecretStore` names a key and cannot read one.** The same boundary
  `ExoRoot` draws around the state root: there is no method here that opens a
  keychain or reads a key file, and `a_secret_store_is_a_name_and_never_a_key`
  fails if one appears. If it could read, a provider credential would be inside
  Omega's address space and "Omega never holds Exo's credentials" would stop
  being true.
- **This delta amends `OMEGA-DELTA-0042`: the turn-level self-modification gate
  was wrong and is gone.** That gate refused any turn whose agent reported a
  capability that *could* widen it — and `tool_creation: enabled` is Exo's
  default on every agent `exo agent create` makes. So the first thing the owner
  ever did with Exo — select it, type `hi` — produced a red **An Error
  Happened** banner reading *"this Exo turn can modify itself; use the dedicated
  confirmation control for this exact draft"*, and a `Refused` dot in the
  composer. Exo did not work at all.
- **Why it was wrong, stated so nobody reintroduces it.** A gate has to name the
  act it prevents. "The person typed a word" is not an act; it is the whole
  product. The dangerous acts, if any, are specific — Exo writing into its own
  checkout, editing its own agent record, writing through a read-write mount —
  and every one of them happens **inside Exo**, where Omega cannot see it and
  therefore cannot gate it at the point it happens. A gate that cannot reach the
  act it names does not become correct by moving upstream until it reaches
  something it *can* stop; it becomes a gate on the wrong thing. The refusal was
  not protecting anybody, because the capability it fired on was configuration
  Exo ships by default rather than anything the turn was about to do.
- **What replaced it: saying, not refusing.** The preflight still reads the
  agent, and the observed capabilities still reach the runtime inspector and the
  log — that was always the useful half. The turn runs. The one-turn
  confirmation control is kept and still mints a grant that rides on the receipt
  for a person who wants one, but no turn requires it, and a grant that no
  longer matches the machine now lets the turn run **without** authority and
  records that, rather than cancelling somebody's message.
- **A policy decision must never be an error banner.** `An Error Happened` is
  the surface for things that went wrong; a refusal that went exactly as
  designed rendered there reads as a bug, which is precisely how the owner read
  it. Nothing on the Exo turn path calls `bail!` for a policy reason any more —
  `no_exo_policy_decision_reaches_the_person_as_an_error` asserts it.
- **`OMEGA-DELTA-0107` is untouched.** Omega still starts no `exo serve`, and
  still creates no root, agent or conversation. That law is about Omega claiming
  authority over Exo's storage and processes; it never had anything to do with
  gating a person's message, and the gate was not protecting it.
- **Enforced by:** `an_exo_lane_names_the_key_that_opens_its_root`,
  `an_exo_child_is_launched_with_the_lane_and_not_with_omegas_environment`,
  `a_derived_lane_finds_the_file_backend_by_the_key_exo_writes`,
  `no_exo_policy_decision_reaches_the_person_as_an_error` and
  `an_observed_exo_capability_is_said_and_never_refused` in
  `crates/omega_deltas`, plus the `secret_store` module's own six tests in
  `crates/omega_exo_lane` and `secret_store`'s in `crates/omega_agent_detect`.
  Each assertion was watched failing against a mutation of the exact thing it
  guards.
- **Driven, not asserted.** `exo acp` was run against the owner's root at
  `~/work/exo/.exo` with the exact argv and the exact two variables
  `connect_configured_lane` now produces, from an environment stripped of every
  `EXO_*` and `OMEGA_EXO_*` variable — the Dock launch, reproduced. `hi`
  returned `stopReason: end_turn` with the reply streamed as
  `agent_message_chunk` deltas. The same invocation with the two variables
  removed returns `failed to decrypt secret payload`, which is the defect this
  delta closes, still reproducible on demand.
- **What this does not cover.** **No window has been opened.** Nothing here
  starts Omega, so the selector's rendering with Exo present, the disappearance
  of the red banner, and the composer's dot after a completed Exo turn are
  unproved against pixels. It also does not make the harness or the model
  choosable from Omega: the lane names one agent and one conversation, and
  changing either is still an edit to the lane file. And it adds secret-store
  fields to the Exo lane and to nothing else.

### OMEGA-DELTA-0127 — Exo says what it can be run with, and Omega already knew how to draw it

- **Upstream Zed:** an ACP agent may return `configOptions` from `session/new`,
  and `ConfigOptionsView` draws one control per advertised option into the
  composer's row. Codex's model and reasoning-effort knobs are exactly this and
  there is no Codex-specific UI anywhere.
- **Omega, before this:** the owner asked for a control for choosing what Exo
  runs — *"ideally theres an ACP control or otherwise a dropdown for selecting
  which to use for exo cuz id like to try some w gemini some w codex"*. Exo had
  no such control, and neither side of the wire was ready: `exo acp` answered
  `session/new` with a session id and nothing else, and
  `ExoHarnessConnection` — the facade `new_session` installs over the thread in
  place of the inner `AcpConnection` — answered the trait's default `None` for
  `session_config_options`, so options would have been parsed, held, and never
  asked for.
- **Omega now:** the facade passes both `session_config_options` and
  `session_modes` through to the `exo acp` connection underneath it. That is
  the whole of the Omega-side change. Nothing new is drawn, because
  `ConfigOptionsView` already draws it, in the composer's bottom-right row
  beside the executor selector, with the keybindings it binds by option
  category.
- **`exo acp` now advertises a model selector**, in `OpenAgentsInc/exo`. One
  `select` option, id `model`, category `model`: every registered LLM binding
  is an option, the current value is the conversation's model override if it
  has one and the agent record's model otherwise, and
  `session/set_config_option` writes a **conversation-level** override. That
  last word is the point — two conversations on one Exo agent can run on two
  models and neither changes what a third gets, which is what "some threads
  with Gemini" means. `apply_conversation_model_override` runs inside every
  send, so a change lands on the next turn with nothing restarted.
- **The unit of choice was a real question and this is the answer to half of
  it.** Exo binds a *harness* and a *model*, and they are not the same axis. The
  model is per-conversation, switchable while a thread is running, and needs no
  process rebuilt — so it is a dropdown, and it is this one. The harness is
  fixed when `exo acp` builds its executor, and the three interesting values —
  `codex`, `claude-code`, `cursor` — are TypeScript presets that resolve a
  module and can npm-install on first use. Switching one mid-session means
  rebuilding the harness and reopening the conversation underneath a live ACP
  connection, with an install possibly happening inside the switch. **That is
  not built, and no control claims it is**: the harness axis is still which Exo
  agent the lane names. A dropdown that listed harnesses and could not change
  one would be worse than no dropdown.
- **An empty root advertises nothing rather than an empty dropdown.** A root
  with no registered model binding returns no `configOptions` at all, because a
  select with zero options is a control that opens onto nothing and a client
  cannot tell that apart from a broken agent.
- **A `currentValue` is always in its own list.** A conversation whose override
  names a binding that has since been unregistered gets that name inserted as an
  option marked `not a registered binding`, rather than advertising a current
  value no option matches — which renders as a selector with nothing selected.
- **Nothing bespoke, on purpose.** A model selector Omega drew for Exo would be
  a second thing to keep in step with what Exo can actually do, and it would not
  be reachable by the keybindings `ConfigOptionsView` already binds by category.
  `exos_own_session_configuration_reaches_the_composer` asserts the Exo
  connection builds no configuration view of its own.
- **Enforced by:** `exos_own_session_configuration_reaches_the_composer` and
  `an_agents_config_options_are_read_off_the_threads_own_connection` in
  `crates/omega_deltas`. The second pins an upstream fact — the composer reads
  config options off the connection the *thread* holds, which for an Exo thread
  is the facade, which is precisely why the facade had to answer.
- **Driven, not asserted.** `exo acp` was run against the owner's root and
  `session/new` returned the option — `"id": "model"`, `"category": "model"`,
  `"currentValue": "gemini-flash"` — and `session/set_config_option` returned
  the refreshed list. Two bindings are registered on that root, so the dropdown
  has something to switch between.
- **What this does not cover.** **No window has been opened**, so the control's
  rendering beside the executor selector is unproved against pixels. It does not
  make the harness choosable, it does not persist a default across threads, and
  it says nothing about what happens if a binding is unregistered while a turn
  is in flight.
### OMEGA-DELTA-0128 — Markdown still arriving is drawn as what it is going to be, and as what it was the moment it stops

- **Upstream behaviour.** Upstream Zed parses the markdown source it has. This
  is not a bug in the parser — while a model streams, the source is a *prefix*,
  and a prefix of `**Searching**` is `**Searching`, whose asterisks a correct
  parser has no choice but to render literally. So the reader watches raw syntax
  appear and snap into styled text, once per construct, for the whole message.

- **What the owner saw.** A live build, a thinking block, and `**Searching`
  drawn with its asterisks showing in the header. His instruction: *"You're
  showing a bunch of open markdown tags before it gets to the final one, and you
  probably need to take a page out of the streamdown playbook."* The same class
  lands anywhere a token boundary falls mid-syntax: a half-written `**bold`, a
  `[link](htt`, an unclosed backtick, a table with only its header row so far.

- **The choice, because it is a real trade.** Three answers were available.
  *Render as plain text until closed* is the upstream behaviour and is what the
  owner is complaining about. *Hide until closed* never shows a wrong style, but
  text then appears late and in bursts, which reads as the model stalling.
  *Complete and render* — treat `**bold` as though the closing pair had arrived
  — reads best while streaming and can be briefly wrong if the next token turns
  the construct into something else. Streamdown (`packages/remend`, read at
  `~/work/projects/repos/streamdown`) takes the third, and so does this: it
  repairs the source before parsing and turns the repair off for the final,
  non-streaming render so nothing invented outlives the stream. The flicker the
  third answer risks is bounded by not repairing a delimiter whose meaning the
  next byte decides.

- **Two deliberate differences from streamdown.**

  **1. Every repair is a suffix.** Streamdown rewrites in the middle of the
  string: it deletes an unfinished image, strips a half-typed HTML tag, moves a
  `_` in front of a trailing newline. It can, because it hands a string to
  `react-markdown` and nothing downstream refers back to the source by offset.
  Here the rendered output carries byte ranges into the source for selection,
  copy-as-markdown, click-to-source, autoscroll and search highlights, and a
  middle edit shifts every offset after it — so all of them would be addressed
  to the wrong characters, silently and only while streaming, which is the
  hardest possible shape of bug to see. So the repair may only be appended.
  Every byte the model sent keeps the offset it was parsed at, and only invented
  bytes live past what was sent. `ParsedMarkdown::streaming_completion_len`
  records how many, and a selection is clamped to what was actually sent so the
  invented markers cannot reach a clipboard.

  **2. Nothing is ever deleted.** Streamdown drops an incomplete image and an
  incomplete HTML tag. The promise here is that every character the model sent
  is on screen, so where a construct cannot be completed it is left exactly as
  it came and renders as its own literal text — which is the honest thing for it
  to look like. An image with half a destination is left alone for that reason
  rather than completed: completing it would draw a broken image where the words
  the model sent used to be.

- **The repair is off unless a stream is running, and that is what keeps the
  promise.** A model that writes a literal `**` and never closes it must end up
  with `**` on screen. `Markdown::append_streamed` arms the repair and
  `Markdown::finish_streaming` ends it, so the last parse of every message is of
  the raw source. An end that turns out to be premature costs nothing — the next
  streamed append re-arms it — but an arm with no end would leave a literal
  marker hidden for the life of the thread, so both paths that arm it end it:
  the turn's own text and thoughts end at `flush_streaming_text`, and a tool
  call's body ends when the call reaches a terminal status.

- **What is completed.** Emphasis (`*`, `**`, `***`, `_`, `__`), strikethrough
  (`~~`), inline code of any backtick-run length, a link destination that has
  been opened with `](`, headings and lists by virtue of being ordinary blocks,
  and a table header row waiting for its delimiter. Emphasis is matched with
  CommonMark's left- and right-flanking rules rather than by counting
  delimiters, which is what keeps `2 * 3`, `snake_case_name`, a `*` bullet and a
  `***` thematic break from being treated as unfinished emphasis. A `~` opens
  nothing on its own, so `~/work` keeps its tilde. A paragraph about to be
  turned into a heading by a `-` that is on its way to being a list bullet gets
  a zero-width space, which is streamdown's trick for the same case.

- **What is not.** A bare `[` with no `](` yet is left alone: nothing says it is
  a link, and `[` reads perfectly well as itself, where streamdown guesses and
  substitutes a placeholder destination. Images are left alone, above. HTML
  blocks, indented code blocks and the inside of an unterminated fence are left
  alone entirely — all three are already showing their source on purpose. A
  table header row is only completed once it is newline-terminated, because
  firing on `| Name | Size |` while it is still being written builds a table
  whose column count changes under the reader, which is worse to watch than the
  pipes. A table whose delimiter row is being typed *is* completed, so the pipes
  do not come back for the several ticks that takes. Nothing is done about
  footnotes, link reference definitions, or math.

- **Enforced by:** `markdown_still_arriving_completes_its_markers_and_gives_them_back`
  in `crates/omega_deltas`, for the four things a rebase could silently revert;
  the `streaming` tests in `crates/markdown/src/streaming.rs`, which feed seven
  documents one byte at a time and assert the repair is a suffix and is itself
  complete at every prefix; `no_prefix_of_a_streamed_document_draws_a_marker_with_text_after_it`
  and `every_word_that_has_arrived_is_drawn_at_every_prefix` in
  `crates/markdown/src/markdown.rs`, which are the owner's complaint and the
  no-loss promise stated as oracles over the rendered text;
  `a_streamed_message_draws_its_markers_and_gives_them_back_at_the_end` in the
  same file, through the real entity and element; and
  `a_streaming_thought_completes_its_markers_until_the_stream_ends` and
  `a_streaming_tool_call_body_completes_its_markers_until_the_call_stops` in
  `crates/acp_thread`, at the seams the two streaming paths actually run at.

- **What this does not cover.** **Nobody has watched it stream in a window.**
  Every claim above is proved headlessly, including through the real
  `MarkdownElement`, and the byte-at-a-time sweeps are the reason to believe the
  boundary cases. What is not proved is a human watching a live model write a
  table and a fenced block back to back and reporting that nothing jumped.

### OMEGA-DELTA-0129 — Nothing stands between the keystroke and the model

- **Upstream Zed:** an agent server is launched and prompted. There is no
  preflight, because there was nothing upstream wanted to be true about the
  agent before letting somebody talk to it.
- **Omega, before this:** the owner selected Exo, saw a green **ready**, typed
  `who are you`, pressed send, and got a red **An Error Happened** reading *"the
  Exo checkout is not at the pinned commit"*. His words: *"You showed me a green
  dot that said ready and then you're giving me this bullshit."*
- **omega#118 removed that one. This removes the rest, and the ability to add
  another.** The pin was the third refusal to fire on an ordinary message —
  `OMEGA-DELTA-0126` had already removed the self-modification gate, which fired
  on Exo's own default `tool_creation: enabled`. Under the pin sat the endpoint
  check, the observation's own `Err`, and two receipt writes, each able to end a
  turn after it was typed. Removing them one at a time is how a night is spent
  discovering the next one.
- **So the rule, not the fix:** *by the time a person is typing, every question
  about whether this can run has already been answered. If it cannot run, it
  must not have said ready.*
  `nothing_can_refuse_an_exo_turn_between_the_keystroke_and_the_model` enforces
  it literally — no `bail!`, no `return Err(`, no `?;` anywhere from the start
  of the spawned turn to `acp.prompt`. Not a list of forbidden checks, because
  the failure was always the *next* check nobody had listed.
- **The endpoint check moved rather than went.** An off-loopback
  `EXO_EXOHARNESS_URL` would send the person's prompt to an unauthenticated
  server holding Exo's secrets, and that is worth refusing. It is not worth
  refusing *after they typed it*. It is asked once now, in
  `connect_configured_lane`, where a refusal means **Exo is never offered** — so
  the selector cannot show a name that would reject a message. Same boundary,
  answered while the answer is still free.
- **The observation informs; it does not decide.** `preflight` still runs and
  still fills the runtime inspector, because what it learns is worth showing. A
  failure now sets the inspector to unavailable, writes a log line, and the turn
  is sent anyway. `observed` became an `Option` so that "the turn can run
  without it" is a fact the compiler keeps rather than a promise a future edit
  can quietly break.
- **`ExoTurnPhase::Refused` is deleted, not merely unused.** An unused variant
  is an invitation, and this work accepted it twice. The way a refusal stays
  gone is that there is no state to put a person's message into.
- **A receipt that cannot be written no longer eats a turn.**
  `record_tier_c_receipt` logs where `persist_tier_c_receipt` returned. Omega's
  bookkeeping about a turn must not be able to destroy the turn.
- **What may still be an error:** a genuine runtime failure. Exo's process
  dying, an API key rejected, the ACP stream ending badly — those happen *after*
  the send and are reported, because they are things that went wrong rather than
  policy that went right.
- **Enforced by:**
  `nothing_can_refuse_an_exo_turn_between_the_keystroke_and_the_model`,
  `an_exo_turn_has_no_refused_state`,
  `a_failed_exo_observation_does_not_stop_the_turn` and
  `an_exo_receipt_that_cannot_be_written_does_not_stop_the_turn` in
  `crates/omega_deltas`, plus the amended
  `the_exo_lane_exposes_no_endpoint_off_this_machine`, which now asserts the
  endpoint is answered at connect rather than on the turn path.
- **Driven, not asserted.** `exo acp` was run against the owner's root at
  `~/work/exo/.exo` with the exact argv and environment the lane produces, from
  a shell stripped of every `EXO_*` variable. `who are you` returned
  `stopReason: end_turn` with the answer streamed back.
- **What this does not cover.** **No window has been opened**, so the absence of
  the banner is proved against the source and against a driven turn, not against
  pixels. And it does not audit the other executors' send paths for the same
  shape.

### OMEGA-DELTA-0130 — Zero base gets one persistent sidebar, and every section in it can fail alone

- **Upstream behaviour.** Upstream Zed's navigation lives in docks and a project
  panel. `OMEGA-DELTA-0052` and `OMEGA-DELTA-0053` removed both from zero base:
  no docks, no centre pane, no status bar. `OMEGA-DELTA-0118` then gave the mode
  a threads sidebar of its own, drawn as an **overlay** — absolutely positioned
  over the thread surface, opened with `cmd-alt-j`, closed again to see what was
  under it.

- **What the owner asked for.** *"i want a persistent sidebar, collapsible,
  kinda like the thread sidebar but also with some vertical collapsible menus. i
  want the last 10 chat threads as one thing, and codex etc ratelimits showing,
  and nostr nip 29 activity too etc — get me an initial version of that added
  now. default open on the zerobase chat page."* Then, on the third section:
  *"for initial nip29 shit i want it showing the most recent 5 messages from the
  default channel, the one we show at /agentchat in apps/openagents.com of
  openagents repo."*

- **One sidebar, not two.** The overlay is gone and this is what `cmd-alt-j`
  now toggles. The action, its namespace and the menu entry are unchanged —
  `agent::ToggleThreadsSidebar`, which was `OMEGA-DELTA-0118`'s entire
  load-bearing repair, because `agent` is the namespace zero base admits and
  `multi_workspace` is not. What changed is what appears: the threads are the
  first of the sidebar's sections, and pressing the binding collapses it to a
  rail rather than removing it. Keeping both surfaces would have been two lists
  of the same threads, which `OMEGA-DELTA-0118`'s own notes call one window
  giving two answers to one question.

- **The composer, which is what actually decides the layout.**
  `OMEGA-DELTA-0105` records that the composer's bottom row wraps so a narrow
  dock does not clip **Send**. `OMEGA-DELTA-0118` protected that by taking part
  in no layout at all. A *persistent* sidebar cannot make that promise, because
  a column that is always there always takes width from the column beside it —
  and an overlay that is always there is a permanent lid over the transcript and
  over the composer's left edge, which is worse. So the sidebar is a real column
  that **yields**: `omega_sidebar::layout` gives it its 280px only while the
  content column keeps `MIN_CONTENT_WIDTH` (600px), and draws a 30px rail
  otherwise. The stored preference is never overwritten by that, so widening the
  window restores the sidebar the person asked for. The composer is therefore
  neither covered nor narrowed past the floor, which is a stronger promise than
  "it happens not to overlap at today's widths". The composer's bottom-left is
  untouched and still empty.

- **No section may interrupt.** `SectionBody` has no error variant. A section
  that cannot load has a quiet muted line where its rows would be, in place, and
  the sections above and below it never find out. There is no toast, no banner,
  no modal and no refusal on any path in the sidebar — omega#119 records what
  happened the last time a zero-base surface toasted. A sidebar that cannot
  reach a relay still draws the owner's threads.

- **What each section actually is, stated rather than implied.**
  - *Recent threads* is **real**. It is `OMEGA-DELTA-0118`'s rows, unchanged:
    `omega_threads_sidebar::rows` still decides the order, the exclusions, the
    ages and the refusals, and this takes the first ten. A thread still reopens
    under the executor that recorded it, and a row whose executor cannot run
    here still refuses in the composer's own words rather than dispatching a
    load that fails in somebody else's error text.
  - *Tester channels* is **real**. A versioned registry bundled with Omega pins
    the exact **Alpha feedback · #alpha-feedback** destination, including its
    relay identity, group, accepted kinds, and limits, so a clean launch does
    not lose the support destination when its HTTPS refresh is unavailable.
    The published Agent Chat manifest may refresh that record only when every
    operational field still matches the bundle. Navigation draws the
    destination rather than messages. Selecting it opens the channel shell in
    the main view. `OMEGA-DELTA-0160` owns the channel-navigation interaction;
    `OMEGA-DELTA-0182` owns the bounded writer in the selected view.

- **The initial disclosure follows the task hierarchy.** On a profile with no
  stored sidebar state, *Recent threads* and *Tester channels* start expanded,
  making the release-candidate support destination unmistakable. Once a person
  changes either section, the stored choice remains authoritative across
  launches.

- **Registry refresh is still a read.** Startup first decodes the bundled
  registry, then performs one HTTPS manifest read and accepts it only as an
  exact compatible refresh. That refresh opens no relay socket and handles no
  signer or key. Relay subscription and the writer begin only after a person
  selects the destination; `OMEGA-DELTA-0182` records that separate boundary.

- **Adding a fourth section.** A variant on `omega_sidebar::SectionId` and its
  entry in `ALL`, its frozen `key` and its `title`, and one arm in the panel's
  `render_sidebar_section`. The delta check reads the enum and asserts every
  variant is named in that match, so a section added and forgotten fails the
  suite rather than drawing a heading over blank space.

- **Settings is persistent navigation, not editor escape.** Settings lives on
  the workbench activity rail (see `OMEGA-DELTA-0205`), so it remains reachable
  when the threads sidebar is collapsed and is not duplicated in the sidebar
  footer. Zero base admits only `OpenSettings`, `OpenSettingsAt`, and
  `OpenSettingsPage`; this makes provider-recovery buttons, Skills, Add Server,
  and Settings work without admitting Extensions or the rest of the
  editor-facing `omega` namespace. The Settings window is a separate visible
  window, so the sealed centre pane cannot hide it.

- **Enforced by:** `zero_bases_sidebar_is_persistent_sectioned_and_silent_when_it_fails`,
  `the_legacy_activity_preview_remains_read_only_and_the_tester_registry_is_pinned`, and
  `public_chat_navigation_is_channel_first_and_read_only` in
  `crates/omega_deltas`,
  for the wiring; `zero_bases_threads_sidebar_is_its_own_and_reopens_by_executor`
  in the same file, whose composer-clipping assertion moved from "draws
  absolutely" to "takes its width from `omega_sidebar::layout`" rather than
  being dropped; and the unit tests in `crates/agent_ui/src/omega_sidebar.rs`
  and `crates/agent_ui/src/omega_nostr_activity.rs`, which are where the
  behaviour lives — the sidebar yielding to a rail at 800px and not at 880px, a
  collapsed section surviving a round trip, a corrupt stored state opening
  rather than failing, a quiet group and an unreachable relay reading
  differently, the relay's non-standard three-element `EOSE` ending the read,
  and a real captured event verifying while the same event with one word changed
  is refused.

- **What this does not cover.** **Nobody has watched it draw in a window.**
  Everything above is proved headlessly. Three things are knowingly not done in
  this first version: the public-chat section shows truncated pubkeys rather
  than kind-0 display names, it reads once when the panel is built rather than
  holding a live subscription, and it reads no moderation state — a message
  deleted after the fetch stays drawn until the next one.

### OMEGA-DELTA-0131 — Conversation creation offers agent choice and every executor label stays truthful

**Superseded in part by the 2026-07-29 three-mode direction.** The clamp that
forced every new zero-base thread through `Agent::NativeAgent` repaired a lying
selector, but it was not a product-shape law. A person must be able to choose a
Direct Agent, Omega Agent, or Sarah when creating a conversation. The chosen
mode and concrete executor are shown before send, and an existing transcript
never changes executors underneath its entries.

The invariant this delta originally protected is retained without exception:
every title, composer label, pending state, and disclosure names the executor
that is doing the work, or clearly distinguishes “will be” from “is”. Restoring
choice must not restore the disconnected selections described below.

The old blank-thread selector and executor rebuild seam remained only on the
legacy `--full-editor` compatibility surface, which omega#161 removed; its
code path is dead behind the constant surface check until omega#162 deletes
it. The default three-mode front door creates or opens another conversation
and never invokes that retarget path.

The owner selected Exo, was shown **Exo** in the composer's executor selector,
typed `who are you`, and read back *"I'm Codex, your AI coding collaborator."*
His question was the right one: **"IS IT FUCKING EXO AND HOW DO I KNOW"**.

Every surface in that window was telling the truth except one. The thread was
titled *New Codex Thread*. The composer said *Message Codex*. The reply said
Codex. Only the selector said Exo, and the selector was the control he had just
used.

**There were two agent selections and they were not connected to each other.**

- `omega_executor_selector::SELECTED` — Omega's, set by this control, read by
  `OmegaRouterServer::connect` when it decides what goes in the router's one
  external-ACP slot.
- `AgentPanel::selected_agent` — Zed's, serialized per workspace and written
  again to a global *last-used agent*, deciding which `AgentServer` the
  conversation is built on in the first place.

The panel had been on `Agent::Codex` since some earlier session, so the
conversation held **Codex's own server**, and Omega's router — the only thing
that reads the executor selection — was never in the path. Choosing an executor
did everything it was supposed to: it debounced, it dropped the cached
connection, it rebuilt. It rebuilt *Codex*, three times in six seconds. The log
for that stretch is three `OMEGA-DELTA-0115: a person chose …` lines, each
followed a second later by an ACP connection and by nothing else — no
`OMEGA-DELTA-0095` attach, no Exo lane, because the router never ran.

The sidebar header is part of the same truthful labeling contract. It used to
reserve the macOS traffic-light safe area before drawing **Omega**, because at
the time nothing else stood between the header and the window controls. Then
`OMEGA-DELTA-0157` installed the platform titlebar, which spans the window
above this header and owns that safe area, so the reservation became a wide
blank strip in front of the label with no control in it. The header now draws
**Omega** flush left with the same padding as the rows under it. Expand lives
on the activity rail when the sidebar is collapsed (`OMEGA-DELTA-0205`).

**The immediate repair built every new zero-base thread on Omega's router.**
That historical clamp made the displayed choice and actual connection agree,
but the 2026-07-29 direction supersedes it with an explicit creation-time
choice. The accessor remains the single answer consumed by both the new-thread
action and its heading so those surfaces cannot drift apart.

**The clamp is on the accessor and not on the stored field, and that correction
cost a launch.** The first version of this fix clamped every *write*. That
rewrote a reopened thread's own agent on the way back in: `OMEGA-DELTA-0118`
restores the last thread under the executor that recorded it, so a Codex thread
came back as the router, the router had no route record for a session it had
never opened, and the owner's next launch said **Failed to Launch — no thread
found with ID**. A reopened thread keeps the agent it was recorded under. What is
pinned is what a *new* one starts on. The panel's `New … Thread` heading reads
the same accessor, because a heading naming a different agent from the thread
`+` would open is the same defect in a smaller place.

**The sidebar's header is one line with the toolbar beside it.** It took its
height from its own padding and drew in `border_variant`, so the rule sat lower
than the thread toolbar's and was fainter — two rules at two heights in two
weights, read as a seam across the top of the window. It now takes
`Tab::container_height` and `border`, which is what the toolbar takes.

**And the label distinguishes *is* from *will be*.** `OMEGA-DELTA-0120` had just
changed it to show the selection rather than the attachment, so that Shift-Tab
would visibly move — right about the control, wrong about the truth, and it is
what let "Exo" sit over a Codex thread. The label still moves on the keystroke,
and now reads `Exo…` while the choice and the attachment disagree, with a
tooltip that says the thread is still on whatever was attached before. There is
no state in which this control names an executor with nothing to separate the
promise from the fact.

That pending name is now scoped to the thread that actually started a switch.
The selection is process-global because it is also the standing choice for the
next connection. Reading it unconditionally let a choice from an earlier
conversation relabel a fresh thread, producing two executor names before the
person had done anything. The attached disclosure is the face at rest; the
standing choice replaces it only while this thread's debounce task exists.

A visibly blank new thread is always switchable. An adapter may transiently
report a running state before it has produced a user or assistant entry. There
is no transcript to preserve in that state, so treating it as an ongoing chat
turns a startup detail into a trap. Once the thread has an entry, only an idle
turn may switch.

- **The nearest miss.** Three separate causes of this one symptom have now been
  fixed by hand — a session id that belonged to another adapter, a connection
  cache keyed by something that did not change, and this. Each was found by
  reading a log after the owner hit it. So the assertion added here is not about
  any of those mechanisms: `choosing_an_executor_rebuilds_the_thread_once_the_presses_stop`
  chooses something other than what is attached, lets the settle window pass,
  and requires the thread to be a new one. It would have failed for cause two,
  and it fails today for a fourth nobody has thought of.

- **Enforced by:** `the_new_conversation_heading_names_the_agent_it_will_open` and
  `the_executor_label_separates_the_choice_from_the_connection` in
  `crates/omega_deltas`, and
  `choosing_an_executor_rebuilds_the_thread_once_the_presses_stop` in
  `crates/agent_ui/src/conversation_view.rs`.

- **What this does not cover.** The three-mode front door landed as
  `OMEGA-DELTA-0177` and was then re-homed into the composer executor dropdown
  by `OMEGA-DELTA-0184`. These checks keep the truthful-labeling and
  immutable-transcript laws live underneath whichever selection surface is
  current: the dropdown's face reads the active conversation's own owner, so
  the two-selections defect recorded here cannot be restated by it.

### OMEGA-DELTA-0132 — The Exo child is told where Exo's harness lives, and the Exo lane runs Exo's own harness

The owner, looking at an Exo thread whose answers were indistinguishable from
the model's: **"what can this Exo do that raw Gemini model cannot???? how to
test this?????"**

Nothing, and the code said so. His lane ran `--harness basic`, and
`build_tool_definitions` in Exo's `crates/executor/src/basic.rs` pushes exactly
one tool, `shell`. The conversation's own event log — 37 events under
`.exo/exoharness/agents/…/conversations/…/events` — held `messages`,
`turn_started`, `turn_ended`, and **no tool call at all**. Every turn had been
text. The two mentions of "shell" in that log are the model talking about a tool
it had never used.

Three separate things had to be true for that, and all three were.

- **`basic` is the thinnest of four harnesses.** `AgentHarnessKind` is `Basic`,
  `Rlm`, `TypeScript`, `Exo`. `Exo`'s tool runtime carries fifteen: `shell`,
  `create_adapter` and the adapter surface, `schedule_sandbox_task` and its
  companions, `snapshot_sandbox`, `rewind_sandbox`, `list_conversation_events`.
  The lane was on the one that carries one.
- **The sandbox provider could not run.** `apple_container` was configured and
  the `container` binary is not installed on this machine; the Docker daemon was
  down. So the one tool the lane did advertise had nowhere to execute.
- **`instructions` was empty.** The agent had no system prompt, so it answered
  "I am a large language model, trained by Google" — accurately. It did not know
  it was Exo.

**Exo's TypeScript harnesses only work from inside the Exo checkout.**
`TypeScriptHarness::exo_from_root` took its workspace root from
`std::env::current_dir()`, and the runner is loaded from
`typescript/harness/runner.ts` beneath it. That is fine for `exo repl` in a
terminal and wrong for every embedder: Omega spawns `exo acp` in the person's
project, so the harness failed with `typescript harness runner does not exist:
<their project>/typescript/harness/runner.ts` — a path naming a file that was
never going to be there, in a directory they have no reason to connect to Exo.

Exo now reads `EXO_WORKSPACE_ROOT` and falls back to the working directory, so
nothing that ran before changes. Omega names it from the lane's own `checkout`
field, which the lane already held for the pin check.

- **What the lane runs now.** `--harness exo`, with
  `examples/typescript/omega-harness.ts` — a module that registers the built-in
  tools (`shell`, `install_agent_tool`, `uninstall_agent_tool`), the adapter
  tools, and the skill tools, rather than a chosen subset. The runtime having a
  tool and the model being told about it are two different things, and the
  second is what this file decides. Thirteen tools are advertised where one was.

- **Driven, not asserted.** `uname -a` and `sw_vers` both executed and returned
  this machine's real output, artifact-backed — `result.json` and `stdout.txt`
  written as conversation artifacts, which is how large tool output stays out of
  the model's context and stays readable afterwards. That is the first tool call
  this lane has ever made.

- **The sandbox is `local_process`, and that is a real trade.** `shell` runs on
  the host, not in a container: the `uname` above names the owner's Mac. The
  alternative available today was a provider that cannot start, which is not
  isolation either — it is the same host execution with an error in front of it.
  One command switches it back (`exo agent update gemini --sandbox-provider
  docker`) once the daemon is up.

- **Enforced by:** `the_exo_child_is_told_where_exos_typescript_harness_lives`
  in `crates/omega_deltas`.

- **What this does not cover.** The harness and module are lane configuration,
  which lives in the owner's `.exo` and not in this repository, so this delta
  holds the wiring rather than the choice. A lane pointed back at `basic` is a
  supported configuration and this suite will not object — what it will not
  allow again is an `exo`-harness lane that cannot find its own runner.

### OMEGA-DELTA-0133 — A fresh Omega Agent has a closed native tool surface

ProductSpec revision 2 makes the reliable no-harness agent the default. A fresh
thread now starts on `basic`, whose model-visible surface contains the six
coding tools (`read`, `write`, `edit`, `bash`, `delegate`, and `resume_thread`)
and five built-in market tools (`market_network_status`, `market_swap_quote`,
`market_execute_swap`, `market_swap_status`, and `market_provision_cloud`).
OMEGA-DELTA-0241 adds the three first-party LN Markets tools to this closed
surface: `lnmarkets_account`, `lnmarkets_market_data`, and `lnmarkets_swap`.
Context-server tools are off, so an MCP installation cannot silently add
another tool. `search_web` is absent; every provider Omega ships refuses it.

The existing implementations and permission identifiers remain intact. The
basic profile aliases `ReadTool`, `WriteFileTool`, `EditFileTool`,
`TerminalTool`, `SpawnAgentTool`, and `ResumeThreadTool` only where
`Thread::enabled_tools` builds the model request. (`OMEGA-DELTA-0136` later
replaced the original file-only read alias with the scoped read dispatcher.)
Persisted permission rules therefore keep their meaning.
The former `write` surface remains available as the explicit `editor` profile,
including its broad built-in and context-server tool sets. No tool was deleted.

`OMEGA-DELTA-0013` already pins the fresh-install model (now
`openagents/gpt-5.6-luna`, owner direction 2026-07-30); this delta composes
that provider default with the closed native-tool profile instead of
duplicating the model check.

- **Enforced by:** `the_basic_profile_has_the_closed_native_tool_surface` in
  `crates/omega_deltas`.

### OMEGA-DELTA-0134 — Destructive Git commands inspect the dirty tree first

The terminal permission ladder still decides whether a command is generally
allowed, including built-in denials and user deny/confirm rules. After that
authorization succeeds—but before a process exists—Omega separately guards Git
commands that can discard local data: path checkout and restore, stash
drop/pop/clear, hard reset, and forced or directory clean.

The guard parses chained commands with the existing fail-closed shell parser
and resolves each command's repository and path scope, including `git -C`.
Unparseable protected commands and scopes outside the project are denied. For a
known scope, the repository backend runs a fresh status query rather than
trusting the asynchronously refreshed UI snapshot. A clean scope proceeds
without friction. A dirty scope prompts with the affected file names; an
unavailable status prompts with the affected repository scope instead.

Every applicable outcome is retained on the tool call as versioned
`openagents.omega.git-data-loss-guard.v1` metadata with an `allow`, `confirm`,
or `deny` decision. This receipt makes the guard auditable even after later
terminal output replaces the visible tool-call content.

- **Enforced by:** `protected_git_commands_cannot_silently_discard_dirty_work`
  in `crates/omega_deltas`, plus parser and classifier unit tests in
  `shell_command_parser` and `agent`.

### OMEGA-DELTA-0135 — The basic profile uses a measured slim prompt

The `basic` profile renders `basic_system_prompt.hbs`; every other profile keeps
the inherited broad system prompt. The slim template is ordered as identity,
communication, native-tool use, work safety, task execution, delegation, system
information, optional sandbox guidance, optional skills, and instruction
files. It omits the wide surface's Mermaid essay, grep/find and LSP workflows,
and editor-specific tool guidance.

The empty-context prompt is rendered in tests with and without sandboxing and
must remain at or below 8,192 bytes before skills and instruction-file bodies.
Changing that ceiling is a policy change. The prompt also binds the work-loss
law directly: the agent preserves user work, uses its own snapshots for undo,
and never treats Git checkout, restore, or stash as an undo mechanism.
Delegation remains optional; the prompt forbids calling it when no executor
exists and tells the agent to complete the work itself.

Project-context maintenance retains its equality gate, so refresh events that
do not change model-visible context keep rendering byte-identical prompts and
continue to hit the provider prompt cache.

- **Enforced by:** `the_basic_prompt_is_separate_short_and_measured` in
  `crates/omega_deltas` and the rendered-template tests in
  `crates/agent/src/templates.rs`.

### OMEGA-DELTA-0136 — Basic `read` spends every address the thread can hold

The basic profile no longer aliases `read_file` and then leaves artifact,
delegation, and skill addresses stranded behind hidden canonical tools. Its
`read` is a dispatcher over the existing readers: project files and images,
this thread's tool-result registry, thread transcripts resolved from live
sessions or `ThreadStore`, and the live skill catalog. Editor and Ask retain
the separate canonical tools; no reader was deleted.

File reads use 1-based `offset` and bounded `limit`, preserve line numbers,
large-file outlines, image content, and `ActionLog::buffer_read` mtime
recording. A partial window prints `Use offset=N to continue.` A directory is a
typed tool error that names the available route, `bash` with `ls`, rather than a
hidden `list_directory` tool.

Artifact lookup still holds only the calling thread's registry.
Transcript lookup asks `ThreadEnvironment::read_thread_transcript`; it reads
the current, parent, sibling, top-level, or persisted Omega session named by
`thread:`, `session:`, `agent:`, or `delegate:`. Skill locations are matched
exactly against the current catalog before the existing skill loader and
permission path reads the body. `delegate` prints `session:<session_id>` as a
transcript address, so the model spends an address it received rather than
inventing one.

- **Enforced by:** `the_basic_read_tool_spends_supported_addresses` in
  `crates/omega_deltas`, plus file, artifact, transcript, and skill-tool unit
  tests in `agent`.

### OMEGA-DELTA-0137 — Basic delegation names its executor and discloses the chain

The basic profile's model-facing `delegate` name continues to use the existing
subagent sessions, depth cap, transcript scope, cancellation, and panel
registry. Its compact input is `executor`, `task`, `label`, and optional
`session`; the stored canonical tool remains compatible with the former
`message` and `session_id` fields.

Executor choice is explicit. `native`, installed ACP agent IDs, `exo`, and
`engine:<lane>` are admitted spellings; `auto` and Khala are not. An unavailable
name returns `no_executor`, and an engine lane that cannot be reached through
the framed `omega-effectd` authority returns `engine_unavailable`. Neither path
runs a different executor. Provider capacity failures are classified as
`account_exhausted` or `account_rate_limited` instead of being flattened into a
generic execution error.

Successful results give the final message, the typed disclosure record reported
by the live handle, and a spendable `session:<id>` address. Exo is resolved from
the installed checkout and existing state root, then connected through the
ordinary ACP transport with no synthetic settings, copied credentials, or
second home. Before the turn, Exo's own agent and model records produce the
hosted identity. The result carries the typed chain Omega Agent → Exo → hosted
runtime/model, so a vendor-backed Exo answer cannot be attributed only to Exo.
The spawning parent retains each live external handle for follow-up turns and
projects its ACP entries through the bounded transcript reader. That map belongs
to the parent's environment and dies with it, so quoting another thread's
session ID cannot widen access and no process-global durable session store is
created.

- **Enforced by:** `delegate_names_never_substitutes_and_discloses_what_ran` in
  `crates/omega_deltas`, resolver and result unit tests in `agent`, and
  `CommandAgentServer` tests in `agent_servers`.

### OMEGA-DELTA-0138 — Slim-agent claims require one integrated proof record

The slim-agent proof protocol validates three distinct observations. The
out-of-box journey binds a candidate, source commit, empty external-executor
inventory, exact ten-tool request surface, direct
`google/gemini-3.6-flash` model, coding change, passing verification command,
completed turn, and content-addressed transcript. The harness journey binds an
exact installed executor to its successful disclosure and readable session
address, then requires the same target to return `no_executor` after removal.
The eval comparison requires the basic and wide profiles to use the same model,
source commit, and task IDs.

`eval-cli` now selects `basic` or `wide` explicitly and writes that choice to
`result.json`. The `zed-eval` launcher records and forwards the selection to
both Harbor and Pier agents. The fixed basic surface refuses
`ZED_EVAL_DISABLE_TOOLS`; a comparison cannot quietly make that profile easier
by removing one of its ten tools. A skipped basic-versus-wide run is a typed
gap in the proof output, not a passing comparison.

The integrated sweep requires OMEGA-DELTA-0133 through OMEGA-DELTA-0138 in
both the ledger and the mechanical registry. The proof output remains
`incomplete` when any installed journey is absent, invalid, from a different
source commit, or when the eval comparison is skipped. Fixture evidence is not
packaged evidence and cannot authorize a release or public reliability claim.

- **Enforced by:**
  `slim_agent_claims_are_bound_to_journeys_comparison_and_delta_sweep` in
  `crates/omega_deltas`, `script/prove-omega-slim-agent --harness-check`,
  `eval_cli` unit tests, and `zed_eval` launcher and harness tests.

### OMEGA-DELTA-0139 — Transcript file links choose editing or peeking

In sealed zero base, a plain click on a transcript file link resolves the file
against the thread's working directories, reveals the ordinary centre pane
beside the agent surface, and opens a normal editable tab at the requested
line. The standard save action is admitted individually; the rest of the
workspace action namespace remains refused. Repeated clicks reuse the ordinary
workspace tab behavior, and closing the final editor tab restores the
agent-only surface.

A secondary click—Command-click on macOS and Control-click elsewhere—preserves
the existing compact read-only modal. Missing files still produce that modal's
visible diagnostic rather than a dead click. Non-file and web links continue
to fall through to the ordinary transcript link handler.

- **Enforced by:** `transcript_file_links_choose_editing_or_peeking` in
  `crates/omega_deltas`, plus the transcript-link parsing tests in `agent_ui`.

### OMEGA-DELTA-0140 — The thread header owns a persistent folder picker

Zero base always renders the thread's working-folder value in the header. With
an attached folder it shows the bounded path; without one it shows
`No folder attached`. The value uses a standard subtle button with a disclosure
chevron, so its affordance is visible without competing with the thread title.

Clicking either state opens the ordinary folder picker in the current window.
The initial composer warning retains its prominent `Open Folder` action, but it
is no longer the only route: once that warning disappears, the header remains
available to correct or change the directory.

- **Enforced by:** `the_thread_folder_is_a_persistent_picker_control` in
  `crates/omega_deltas`.

### OMEGA-DELTA-0141 — Runtime identity custody is local and owner-only

Every Omega channel stores its Nostr identity secret in an atomic,
owner-readable file at `identity/identity.secret` below the channel data root.
The same 32-byte secret validation and custody reset path applies to
development, Nightly, RC, stable, and the operator CLI. On Unix, the directory
uses mode `0700` and the file uses mode `0600`. The file is not encrypted at
rest; the operating-system user account and data-directory permissions are the
current security boundary.

- **Enforced by:** `identity_custody_is_always_private_local_storage` in
  `crates/omega_deltas`, plus file-store round-trip and permission tests in
  `crates/omega_identity`.

### OMEGA-DELTA-0142 — Provider credential errors lead to their solution

A missing or rejected provider credential is not a transient request failure.
Omega stops retrying it, presents the provider's credential guidance, and adds
a provider-named action that opens **Settings → AI → LLM Providers**, where the
credential can be entered or replaced. The action sits in the error callout
that reported the problem rather than requiring the user to discover the
settings hierarchy independently.

Thread errors render as contained cards in the same centered, maximum-width
column as conversation messages and the composer. They use the message
surface's full border and corner treatment rather than drawing an edge-to-edge
status strip across the window. Error cards use the semantic error color for
their full outline, so their state does not read as an ordinary focused field.

Google AI reports a missing key through the typed, non-retryable
`NoApiKey` completion error. Missing and invalid credentials from every native
provider use the same actionable error surface.

- **Enforced by:** `provider_credential_errors_open_llm_provider_settings` in
  `crates/omega_deltas`.

### OMEGA-DELTA-0143 — Lost identity reset returns to first-run setup

When an identity's public manifest remains but its signing secret is absent,
there is no secret for the normal marker-first, relaunch-required reset to
protect. The explicit **Reset identity** action completes and acknowledges that
cleanup immediately, removes the stale public identity records, and returns
onboarding to the create-or-import state in the same process.

Ready identities retain the restart-safe reset boundary. This exception applies
only to the already-lost state and cannot delete an available signing key.

- **Enforced by:** `a_lost_identity_can_reset_without_another_relaunch` in
  `crates/omega_deltas`, plus
  `resetting_a_lost_identity_returns_to_first_run_without_a_relaunch` in
  `crates/omega_identity`.

### OMEGA-DELTA-0144 — Exo is opt-in for each launch

Exo is outside an ordinary Omega process. A default launch does not inspect an
Exo lane file or installation, attempt an Exo connection, offer Exo in the
executor selector, or show an Exo rate-limit row.

The person launching Omega may opt in for that process with `--enable-exo`.
The choice is a command-line fact read before application initialization; it
is not persisted and cannot be enabled by settings, a thread, or a tool.

- **Enforced by:** `exo_is_opt_in_for_each_launch` in `crates/omega_deltas`,
  plus executor-selector attachment-plan tests in `agent_ui`.

### OMEGA-DELTA-0145 — Settings starts with Omega-owned provider credentials

**Settings** opens a focused Omega surface containing only provider API-key
configuration. It reuses the providers' existing secure credential storage and
status controls, so Google AI and the other native providers can be configured
without exposing unrelated inherited editor preferences.

The previous full settings editor remains available as **Legacy Settings**.
The application menu and command palette name that distinction explicitly.
Provider-credential recovery actions and the persistent zero-base Settings
button open the focused surface, while Legacy Settings remains an admitted,
separate action.

- **Enforced by:** `focused_settings_separates_provider_keys_from_legacy_settings`
  in `crates/omega_deltas`.

### OMEGA-DELTA-0146 — Direct-agent inventory and background warming remain separate

**Superseded in part by the 2026-07-29 three-mode direction.** Restricting
ordinary selection to Omega was a temporary public boundary, not the product
contract. Ready Codex, Claude Code, Grok Build, and configured ACP agents return
as Direct Agent choices at conversation creation. Exo remains subject to its
own readiness and launch contract.

The separation this delta established remains: the public creation-time list
is derived from honest readiness, while detection and background warming use
the complete runtime inventory. Hiding or disabling a row must never turn off
the infrastructure Omega Agent needs for routing, and warming an adapter must
never by itself authorize a direct conversation.

- **Enforced by:** `direct_agent_inventory_stays_separate_from_background_warming`
  in `crates/omega_deltas`, plus selector policy tests in `agent_ui`.

### OMEGA-DELTA-0147 — Sealed zero base keeps the native window drag strip

Sealed zero base renders Zed's `PlatformTitleBar` as an empty platform strip.
It keeps the standard traffic-light spacing, window drag gesture, and native
titlebar double-click behavior. The project, branch, collaboration, account,
sidebar, and application-menu controls remain absent.

The workspace owns titlebar dragging through the same
`Window::start_window_move` path as ordinary Zed windows. Restoring the drag
region cannot restore the editor or create an action-gate bypass.

- **Enforced by:** `sealed_zero_base_keeps_only_zeds_platform_drag_strip` in
  `crates/omega_deltas`, plus
  `the_transitional_sealed_layout_starts_without_the_legacy_editor`.

### OMEGA-DELTA-0148 — The sidebar contains no empty rate-limits section

Zero base's left sidebar contains **Recent threads** and **Channels**. It
does not contain a rate-limits heading, executor rows saying “not reported,” or
an explanation of why quota data is unavailable. That surface offered neither
data nor an action, so it consumed persistent navigation space without helping
the person using the application.

Older stored sidebar state may still contain the retired `rate-limits` collapse
key. The forgiving state decoder continues to accept and preserve unknown keys,
so removing the section cannot corrupt a person's other sidebar preferences.

- **Enforced by:** `zero_bases_sidebar_is_persistent_sectioned_and_silent_when_it_fails`
  in `crates/omega_deltas`, plus sidebar state tests in `agent_ui`.

### OMEGA-DELTA-0149 — Executor choice belongs to conversation creation, not the turn composer

**Superseded in part by the 2026-07-29 three-mode direction.** Executor
selection returns at the conversation front door. It does not return as a
provider/model control that can silently retarget a transcript between turns.
The mode, executor, and readiness are visible before the first send;
after that point the composer reports what the conversation owns.
`OMEGA-DELTA-0184` re-homed the front door itself into the composer bar as
the executor dropdown: the selection control now lives in the turn composer,
but it still only ever creates or re-homes a conversation — over a bound
transcript it starts a new thread and never retargets the one under it.

Codex, Claude Code, Grok Build, configured ACP agents, Omega's routing
infrastructure, and Sarah remain available to the creation surface. Their
provider-specific model, mode, reasoning, and fast-mode controls do not leak
into the shared turn composer.

- **Enforced by:** `executor_choice_belongs_to_creation_not_the_turn_composer`
  and `direct_agent_inventory_stays_separate_from_background_warming` in
  `crates/omega_deltas`.

### OMEGA-DELTA-0150 — A conversation keeps the mode and executor chosen at creation

**Superseded by the 2026-07-29 three-mode direction.** A new conversation may
belong to a directly selected ACP agent, to Omega Agent and its disclosed
router, or to Sarah voice. Detection and warming are readiness observations,
not consent; the explicit creation choice is the authority to create the
conversation on that mode.

Once created, the conversation retains its owner and concrete executor across
turns and relaunch. A keyboard cycle, provider control, readiness change, or
router observation cannot retarget an existing transcript. Durable historical
route reasons remain readable even when the current creation policy no longer
emits them.

- **Enforced by:** `an_unpinned_thread_never_reaches_an_engine_lane`,
  `conversation_creation_does_not_retarget_an_existing_transcript`, and
  `executor_choice_belongs_to_creation_not_the_turn_composer` in
  `crates/omega_deltas`, plus router behavior tests in `omega_front_door` and
  `agent_ui`.

### OMEGA-DELTA-0151 — Omega uses hosted compute without a Google key

Zero base sends Google-backed Omega requests through OpenAgents hosted compute
by default. The desktop reuses its verified OpenAgents account session, obtains
a short-lived quota grant, and sends the inference request to the OpenAgents
broker. The hosted Google credential remains server-side and is never returned
to or stored by the desktop.

A locally configured Google API key remains an optional fallback when hosted
authentication is not available. It is not required to start or use Omega, and
zero base must not surface the inherited `NoApiKey` failure for this default
path.

- **Enforced by:** `zero_base_uses_hosted_google_without_a_local_key` in
  `crates/omega_deltas`, plus the OpenAgents Gemini broker route tests.

### OMEGA-DELTA-0152 — Omega names itself and its delegate executors

The first-party zero-base prompt identifies the product as **Omega**. Its
hosted model provider is an implementation detail and must not become the
assistant's name, so the prompt neither presents the backing model as system
identity nor permits the assistant to identify as Gemini or Google.

The same process-level detector used to resolve a `delegate` call supplies the
prompt's installed executor catalog. Codex, Claude Code, and Grok remain absent
from public executor controls, but Omega can see their stable IDs and names
before deciding whether to delegate. Detection, prompt disclosure, and runtime
resolution therefore cannot silently become three different inventories.

- **Enforced by:** `omega_names_itself_and_the_executors_it_can_delegate_to`
  in `crates/omega_deltas`, plus basic system-prompt rendering tests in
  `agent`.

### OMEGA-DELTA-0153 — OpenAgents authentication stays in the background

Connecting Omega to its owner-scoped OpenAgents services no longer opens a web
browser or starts a loopback callback server. Omega signs a fresh NIP-98 event
with its built-in Nostr identity and sends the proof directly to the exact
OpenAgents session endpoint. The shared signer binds the HTTPS URL, POST method,
exact payload hash, and current timestamp. The ordinary hosted-session request
signs an empty payload; managed Sarah admission signs the exact serialized
admission JSON, then uses the returned short-lived bearer for the separately
requested voice session. The server resolves the configured owner and consumes
each proof once.

The resulting access token is short-lived and remains in Omega's isolated
credential store. Existing OAuth credentials remain readable for migration,
but new sessions do not carry a refresh token. NIP-42 remains in use where it
belongs: authenticating Omega to Nostr relays. It is not treated as an HTTP
session protocol.

- **Enforced by:** `openagents_authentication_never_opens_a_browser` in
  `crates/omega_deltas`, plus the NIP-98 signer and server proof-exchange tests.

### OMEGA-DELTA-0154 — The mobile bridge is direct, authenticated, and read-only

Omega's mobile mirror uses a GPUI-free WebSocket transport that binds only a
literal loopback or Tailscale address. A device proves possession of its Nostr
key and presents the same durable scoped grant used by the relay lane. The
bridge sends snapshots and ordered deltas, and deliberately has no command
frame.

- **Enforced by:** `device_bridge_preserves_its_authority_partition` in
  `crates/omega_deltas`, plus transport tests in `omega_device_bridge`.

### OMEGA-DELTA-0155 — Mobile discovery and QR pairing remain engine-owned

Omega advertises direct mobile endpoints only in the identity-signed Issue 31
discovery V3 record. Endpoints are structured MagicDNS name, port, and exact
bridge protocol values; arbitrary URLs are not accepted. Records expire, and
only a newer generation or an identical-endpoint renewal supersedes an active
record.

The desktop's **Pair phone** surface renders an engine-issued QR bootstrap. Its
secret is short-lived and consumed on the first admission attempt. The phone
still proves its own Nostr device key, and successful admission mints the
ordinary request/challenge/response/scoped-grant lineage. GPUI neither creates
grants nor receives a private identity key.

- **Enforced by:** `mobile_discovery_and_qr_pairing_keep_engine_authority` in
  `crates/omega_deltas`, plus discovery, QR, and one-use grant tests in
  `omega_effectd` and `omega_device_bridge`.

### OMEGA-DELTA-0156 — The mobile mirror projects live Omega state

The authenticated direct bridge starts with active and recent Agent threads,
typed executor and model disclosure, bounded user/assistant transcript
previews, Full Auto run lifecycle and authority receipt references, and engine
generation and lane readiness. Subsequent thread, transcript, run, and health
changes are ordered deltas from that same projection.

The run feed reads the existing Issue 31 Full Auto observation used by the Sync
lane. GPUI does not create a second run registry. Raw provider payloads,
credentials, exact paths, model reasoning, and unbounded transcript or artifact
data do not enter the mirror. A generation change resets the cursor and requires
a fresh snapshot.

A tool call contributes one bounded line: its label and its state, through the
same public-text bound every other mirrored string passes. Arguments, diffs,
file contents, command output, and raw results stay on the desktop. The owner
directed this on 2026-07-27, because excluding tool calls entirely left a phone
showing a question and then a silence while the desktop delegated the work and
answered. A delegation to another harness is a tool call, so hiding the class
hid the thing the mirror most needed to show.

Reduction happens before the safety check, not after. The check used to read the
raw Markdown export, which still carried the model's reasoning inside
`<thinking>` tags, and one disallowed line in a thought rejected the whole
message. That is why a streaming answer stayed invisible until its reasoning
ended and then arrived in one piece.

- **Enforced by:** `mobile_mirror_projects_live_state_without_new_authority` in
  `crates/omega_deltas`, plus projection journey, redaction, bound, resume, and
  restart tests in `omega_device_bridge`, `full_auto_ui`, and `agent_ui`.

### OMEGA-DELTA-0157 — The titlebar view is installed by the shipped binary

`title_bar::init` is called from Omega's own startup sequence in
`crates/omega/src/main.rs`. It used to be called from `collab_ui::init`, and
"Retire Zed collab" deleted that crate with the call inside it. Nothing else
called it, so `Workspace::titlebar_item` was `None` in every window of every
mode: no `PlatformTitleBar`, no `WindowControlArea::Drag` region, no
`start_window_move` listener, and a window a person could not move at all.
`OMEGA-DELTA-0147` describes what that view renders once sealed; this delta is
what puts the view in the window.

Zero base hides the two panel-layout actions from the command palette rather
than showing them. `CommandPaletteFilter::is_hidden` answers `false` for a
shown action type before it reads the admitted set, so the ordinary
`show_action_types` call would list `workspace::UseClassicLayout` and
`workspace::UseAgenticLayout` in a palette that admits neither.

- **Enforced by:** `omega_installs_the_titlebar_view_it_renders` in
  `crates/omega_deltas`, plus
  `sealed_zero_base_keeps_a_full_width_platform_drag_strip`.

### OMEGA-DELTA-0158 — A session never opens in a directory that is gone

- **Upstream Zed:** `session_directories_from_work_dirs` takes the first of a
  thread's recorded working directories and sends it as the session's `cwd`,
  whatever it names.
- **Omega:** a recorded directory that is no longer there is dropped instead of
  sent. If that leaves nothing, the session opens in the project's visible
  worktree roots — the value the thread header is already showing
  (`OMEGA-DELTA-0140`). If there are none of those either, it refuses in a
  sentence naming the directory that went and the control that fixes it.
- **Why:** owner report against `0.2.0-rc21`, on a fresh install. He set the
  thread's folder to `~/work/openagents`, watched the header say so, sent a
  message, and got `Failed to Launch — Invalid params: cwd does not exist on
  the machine running the agent: /private/tmp/omega-rc-final-project.JpARAa`
  with the path repeated as a JSON blob. The temporary directory was left over
  from an earlier release-candidate smoke run.
- **The two values had drifted apart.** The header renders
  `project.visible_worktrees(cx).next()`, live. The agent's `cwd` comes from
  `AcpThread::work_dirs`, restored from `ThreadMetadataStore::folder_paths` or
  the serialized panel state and preferred over the project's own list at
  `conversation_view.rs`'s `work_dirs.unwrap_or_else(..)`. Choosing a folder
  from the header calls `MultiWorkspace::open_project`, which does not write the
  thread's record, so the window could show one folder while the agent was
  started in another that no longer existed.
- **Why the fallback is the worktree roots and not `default_path_list`.** That
  helper stands the home directory in when a project has no folder open, while
  the header says `Choose a folder`. Substituting `$HOME` would start an agent
  in a directory nothing on screen ever named — the failure this delta is
  about, with the error removed rather than the cause.
- **A remote project is not probed.** It starts its agent through its remote
  client, where `cwd` names a directory on the other host, so nothing here can
  answer for it and every directory is taken on trust. The guarantee is
  therefore local-project-only, and says so in the code.
- **An empty recorded list still refuses.** That is a caller that never chose a
  directory, which `OMEGA-DELTA-0112` found and fixed at the caller.
  Substituting for it here would hide the next one.
- **What this does not do.** The thread's recorded working directories are left
  as they are, so a substituted session re-substitutes on the next send rather
  than repairing the record. Reconciling the record with the header is a
  separate change to who owns that value.
- **Enforced by:** `a_session_never_opens_in_a_directory_that_is_gone` in
  `crates/omega_deltas`.

### OMEGA-DELTA-0159 — Hosted sign-in respects identity consent and names its refusal

A hosted Gemini request is the only way a zero-base install can send a message,
and the only credential it can present is a NIP-98 proof signed by this
channel's Omega identity. Automatic hosted sign-in must not create an identity
or adopt a protected identity for a new profile. It inspects custody and signs
only when the state is already `Ready`. An `Absent` or `Unadopted` state returns
an actionable blocker that directs the owner to the existing identity setup.

Sarah's challenge response can carry a server-side account mapping that Omega
does not know in advance. Omega copies that `ownerRef` into the exact signed
voice-admission body but does not persist it in the Nostr-issued bearer
credential. Normal bearer verification resolves the owner in memory. Existing
legacy credentials retain their stored owner binding for compatibility.

Phone pairing is different because the owner explicitly starts that operation.
It can use the narrow unattended provisioning transaction after that visible
action. A state the profile cannot sign for still refuses by name.

Provisioning is deliberately narrow. `Absent` creates, `Unadopted` adopts the
identity this data root's own secret file already holds, and every other state
is a refusal that names itself. `Lost`, `Conflict`, `Incomplete`, and the reset
states each mean an identity exists that this profile cannot sign for, and
replacing one with a fresh key unattended is the silent pick omega#110 forbids.

Every step logs. Before this delta the whole path — provisioning, signing, the
mint request, verification, and the grant — swallowed each error into a phase
enum and wrote nothing, so a refused installation produced no log line at all
and the only evidence was a UI sentence that named none of the causes.

That sentence is retired. `OpenAgents sign-in was not completed. Send the
message again…` was wrong in both halves for the failure people actually hit: a
`401` from the session endpoint is terminal, and retrying can never turn it into
a session. The message now carries the specific blocker — the custody state, or
the HTTP status — and offers retry advice only when a retry could work.

- **Enforced by:** `hosted_sign_in_respects_identity_consent_and_names_every_refusal`
  in `crates/omega_deltas`, plus unattended-provisioning tests in
  `omega_identity`, blocker tests in `omega_effectd`, and failure-message tests
  in `language_models`.

### OMEGA-DELTA-0160 — Public chat navigation contains channels, not messages

The zero-base sidebar shows stable public-channel destinations. It does not use
individual relay events as navigation rows. The first production destination
is `#agent-chat`. Selecting it opens a channel shell in the main view and keeps
the thread or terminal behind that shell in memory.

The generic controller consumes the versioned
`openagents.public_channel_registry.v1` contract. The current OpenAgents web API
still publishes one Agent Chat manifest, not a registry endpoint. Omega adapts
that manifest to one production descriptor. An exact copy of the two-channel
OpenAgents fixture is test data only. It proves that a second channel needs no
schema or layout branch, including when two channels use the same group ID on
different relays.

Each channel owns its relay-qualified coordinate, lifecycle, cursor, verified
event IDs, cached state, and unread count. The subscription policy is
`SelectedOnly`: selection marks the prior observed snapshot as cached and
clears unread for the new selection. It does not describe an initial,
unobserved snapshot as cached. A snapshot for a different relay or group is
refused. New events can change a snapshot or unread count, but they cannot
change the destination count or order.

Each destination is a keyboard-focusable button with a stable channel-based
element ID. Its accessible label includes the visible channel label, lifecycle,
cached state, and unread count. Enter or Space selects a focused destination.
Selection moves both panel focus contracts into the channel surface. The
existing 880-pixel layout floor remains authoritative, so the channel list
yields with the rest of the sidebar in a narrow window.

This slice opens no relay socket. It adds no live timeline, media loader,
composer, signer, identity, join, or moderation control. The selected shell
shows the authoritative relay, group, and lifecycle while the dependent live
timeline work remains separate.

- **Enforced by:** `public_chat_navigation_is_channel_first_and_read_only` in
  `crates/omega_deltas`, plus registry, controller, coordinate, lifecycle,
  unread, fixture, accessibility-label, and narrow-layout tests in `agent_ui`.

### OMEGA-DELTA-0161 — A delegated sub-agent never asks for permission

- **Upstream Zed:** an external ACP agent opens in whatever mode it defaults to.
  Claude Code's ACP adapter defaults to `default` ("Manual"), which prompts
  before dangerous operations, and Codex to its own gated mode. A session's mode
  is otherwise only chosen by a person, from settings or the mode selector.
- **Omega:** a sub-agent opened by the `delegate` tool is put into the agent's
  unattended mode — `bypassPermissions` for `claude-acp`, `agent-full-access`
  for `codex-acp` — before the parent can send it anything, and the delegation is
  refused if that did not happen.
- **Why:** owner report against `0.2.0-rc21`: a delegated `claude-acp` sub-agent
  stopped on `Always Allow / Allow / Reject` for a `Read` outside the thread's
  folder, and macOS said `omega • Waiting for tool confirmation`. Owner
  direction: *"IT'S ASKING ME FOR PERMISSIONS. I SAID ALWAYS MAKE IT BYPASS
  PERMISSIONS MODE."* A delegated sub-agent's prompt has nobody to answer it.
  Its caller is a model, the prompt renders on a subagent card, and the run stops
  there until a person notices — which is the whole failure `OMEGA-DELTA-0002`
  removed for Omega's own tools and which arrived again through a different door.
- **The mode was already being asked for, and was being dropped.**
  `CustomAgentServer::default_mode` returned the unattended mode and `connect`
  carried it to `AcpConnection`. But `AcpConnection::stdio` stores that value and
  then immediately calls `AcpConnectionDefaults::observe_settings`, whose first
  act is `refresh_from_settings` — which *overwrites* the mode with whatever
  `agent_servers.<id>.default_mode` says, and with `None` when it says nothing.
  The owner's live `~/.config/omega-rc/settings.json` holds
  `"claude-acp": { "type": "registry" }` and no mode, so the value was gone
  before the first session existed and `new_session` sent no `session/set_mode`
  at all. Upstream never saw this because upstream's only source for that value
  *is* settings, so re-reading settings returns the same answer; a hardcoded
  mode is the one kind that cannot survive the refresh.
- **So it is applied where it can be proven, not where it looks tidiest.**
  Both panel threads and delegated sessions now wait for the adapter to
  acknowledge the full-access mode before the session can receive a prompt.
  `session/set_mode` and the newer mode `session/set_config_option` path are
  awaited. A rejected mode fails the session open instead of silently exposing
  a thread in the adapter's interactive default.
- **The mode has two homes, and the agent picks.** ACP lets `session/new` answer
  with `modes` *or* with `configOptions`, and `config_state`
  (`agent_servers::acp`) keeps only whichever arrived — so for an agent of the
  second kind `AgentConnection::session_modes` is `None` and there is no
  `session/set_mode` surface to use. `claude-agent-acp@0.62.0` is of the second
  kind: it sends a `Select` carrying the `Mode` category whose values are the
  mode ids. The first cut of this delta knew only `session/set_mode`, so its
  live run refused *every* Claude delegation with "exposes no session modes" —
  correct as a refusal, useless as a product. Both surfaces are now read, and
  the selector is found by its declared category rather than by its id, because
  an agent is not obliged to name it `mode`.
- **The agent's own answer is what counts.** `set_mode` is sent even when
  `current_mode` already reports the wanted mode, because that field is written
  optimistically before the request is answered — reading it would accept the
  agent's starting mode as proof of a request that was never made. If the agent
  does not offer the mode, the delegation is **refused by name**, listing what
  was offered instead. `claude-code-acp` withholds `bypassPermissions` when it
  runs as root, so this is a real state and not a defensive branch. A delegation
  that will stall is worse than one that refuses up front: the refusal is visible
  immediately, the stall is visible only to whoever looks at the card.
- **Scope is every owner coding-agent session.** Codex and Claude panel threads
  use the same full-access modes as delegated sessions. This is an Omega product
  default and is not inherited from the user's Codex config. An agent with no
  entry in `full_access_mode_for_agent` keeps its own permission policy.
- **The adapter's dual configuration surface is not optional.**
  `codex-acp@1.1.9` returns both legacy `modes` and modern `configOptions`.
  Omega's `config_state` deliberately keeps `configOptions` when both exist, so
  applying full access only through `session/set_mode` skipped the setting
  completely. The connection now finds the config option by its declared
  `Mode` category, selects `agent-full-access`, and waits for the response.
- **An approval request is treated as an adapter-policy defect, not UI.** If
  Codex or Claude still sends `session/request_permission` while it is bound to
  Omega's full-access profile, Omega selects the strongest offered allow option
  immediately. No confirmation card is added to the thread. Other ACP agents
  retain normal interactive permission behavior.
- **Known tradeoff, stated plainly:** an owner coding-agent session can read and
  write outside the thread's folder and run commands without asking. That is the
  requested behaviour. `OMEGA-DELTA-0002`'s `always_confirm` / `always_deny`
  patterns still gate Omega's native tools; they do not narrow an external ACP
  agent that is intentionally running in its own full-access mode.
- **What has run.** `a_delegated_claude_subagent_reads_outside_its_folder_without_asking`
  drives the real `claude-acp` adapter through the real
  `create_external_acp_subagent`, asks it to read a file the session's `cwd`
  does not contain — the owner's exact scenario — and requires the turn to
  finish. In `default` mode the adapter sends `session/request_permission` for
  that read and the turn cannot complete, so the test fails by not finishing.
  It is `#[gpui::test]` behind the `e2e` feature:
  `cargo test -p agent --features e2e --lib external_acp_subagent`.
  What has **not** been seen is the subagent card in a window; that is
  `crates/agent_ui`, and `OMEGA-DELTA-0101` already names it unproven.
- **Enforced by:** `a_delegated_subagent_is_admitted_only_in_its_unattended_mode`
  in `crates/omega_deltas`, plus
  `routed_subagents_are_unattended_without_locking_other_cards` and
  `owner_coding_threads_acknowledge_full_access_and_hide_approvals`; and
  `a_delegated_claude_subagent_is_put_in_bypass_permissions`,
  `a_delegated_subagent_mode_is_set_through_config_options_when_that_is_the_surface`,
  `an_unlisted_delegate_executor_is_left_on_its_own_mode`,
  `a_delegation_is_refused_when_the_mode_is_not_on_offer` and the live
  `a_delegated_claude_subagent_reads_outside_its_folder_without_asking` in
  `crates/agent`.

### OMEGA-DELTA-0162 — A delegated subagent names the thread that spawned it, and its card opens both ways

- **Upstream Zed:** a subagent is the parent's own loop, so there is no second
  session, no second connection, and nothing that can fail to record a parent.
  There is no upstream behaviour to revert to; the divergence is that Omega
  delegates a turn to another agent's server at all.
- **Omega, before this.** `OMEGA-DELTA-0112` made an external subagent
  *findable* — the panel could resolve the session id in
  `SubagentSpawned` and draw a card. What it could not do was say whose
  subagent it was. `AcpConnection::new_session` builds the thread with
  `parent_session_id: None`, because it runs on the external agent's own
  connection, which has never heard of the thread that asked for it. So every
  Codex, Claude and Exo delegation arrived claiming to be a root thread, and
  the panel believed it. That one missing fact produced four separate defects,
  and the owner hit three of them in one sitting:
  - `render_subagent_titlebar` returns `None` for a thread with no parent, so
    the full-screen subagent view had **no minimize control at all**. The card's
    always-visible strip offered `Maximize` and nothing else, and the header
    chevron that would have closed the card in place was drawn only on hover.
    Expanding was a **one-way door**: *"if I click the button to expand it, I
    see no way to shrink it back down."*
  - `workspace::GoBack` reads the same field, so the keyboard way back was dead
    for the same reason.
  - `menu::Cancel` cancels generation only for a thread with no parent — a
    guard written so Escape cannot kill work a parent's tool call is waiting
    on. A delegation that claimed no parent fell on the wrong side of it, so
    **Escape cancelled the delegation** in the one view where a person presses
    Escape to leave.
  - `ConversationView::handle_thread_event` decided "did the root thread stop?"
    by asking the stopping thread whether it named a parent. A delegation
    finishing therefore read as the root finishing, and the root's message
    queue was spent on it: a follow-up the person had typed was dispatched
    mid-turn and left the queue. *"I had a message queued out, but then it just
    was gone."* Silent loss of the person's own typing, with no undo and no
    notice.
- **Omega now.** The one place that knows the parent records it.
  `create_external_acp_subagent` and `create_exo_subagent` read the spawning
  thread's session id before opening the session and set it on the returned
  `AcpThread`, beside the `external_subagent_sessions` registration
  `OMEGA-DELTA-0112` already puts there. A parent that is already gone leaves
  the field alone rather than inventing one.
- **And the queue asks the conversation, not the thread.** Recording the parent
  fixes the queue defect, but it fixed it by making a thread's self-report
  true, and a queue that spends the person's typing must not depend on a thread
  being honest about itself. `handle_thread_event` now asks whether the
  stopping session **is** this conversation's root, which the conversation
  already knows from `root_session_id`. The failure mode inverts with it: an
  unknown root suppresses the auto-send and leaves the message queued and
  visible, where before an unrecorded parent dispatched it. A message that
  stays queued is a message the person can still see and still send.
- **The way back is drawn, not hovered.** The card's bottom strip is the only
  control on an open card that exists without the pointer on it, and it offered
  one direction. It now carries collapse beside full-screen, and the header
  chevron stops hiding on hover once the card is open — a control that appears
  only under the pointer is not a control a reader can find at the bottom of
  content they have just opened. Expanding a subagent now opens it beside the
  root thread in the right pane instead of replacing the root thread. Escape
  closes that pane rather than being swallowed, and the titlebar's minimize
  reads as the inverse of the card's open-in-right-pane control and names the
  key.
- **The card scrolls.** The preview was a 14rem window onto content that is
  routinely taller, with `track_scroll` recording an offset and no
  `overflow_y_scroll` to move it, under an `overflow_hidden` that clipped the
  rest: *"I can't scroll inside those subagent things."* Expanding was not only
  a one-way door, it was the only door. It scrolls now, and it **occludes**: the
  transcript is a `List` that registers its scroll handler after painting its
  items and never stops propagation, so without occlusion the same wheel delta
  moves both and the thread jumps out from under what is being read. The
  deliberate edge behaviour is that the gesture stops at the card's top and
  bottom rather than chaining outward. Following the tail is also now
  conditional on the reader being at the tail; it used to be unconditional on
  every render, which pulled a live delegation's output back down under anyone
  reading it.
- **Enforced by:** `a_delegated_subagent_records_the_thread_that_spawned_it` and
  `a_subagent_card_draws_a_way_back_and_a_way_to_read_it` in
  `crates/omega_deltas`;
  `a_subagent_turn_ending_cannot_spend_the_root_threads_queue` in
  `crates/agent_ui/src/conversation_view.rs`; and
  `a_subagent_card_closes_by_the_same_state_that_opened_it` in
  `crates/agent_ui/src/entry_view_state.rs`.
- **What this does not cover.** **Nobody has seen it move.** The controls, the
  key, the queue and the scroll are checked headlessly and in unit tests; none
  of it has been driven in a window. `OMEGA-DELTA-0112`'s acceptance 3 was left
  open for the same reason and stays open. Recording the parent also makes an
  external subagent's own view stop drawing a composer, which is what a native
  subagent's view has always done — a person can no longer type directly into a
  delegated thread from the panel, and that consequence has not been rendered
  either.

### OMEGA-DELTA-0163 — A selected public channel owns a live verified timeline

The `#agent-chat` destination now opens a read-only live Nostr timeline. The
selected channel owns the socket. Switching channels closes the old
subscriptions and starts or resumes the new channel. Each retained view keeps
its verified rows, cursor, lifecycle, unread calculation, event-detail state,
and media state separate from all other relay-qualified channel coordinates.
A generation and coordinate fence refuses a relay or media completion from a
channel that is no longer selected. The first snapshot from a resumed session
merges with the retained verified rows and last current time. It does not
replace them with an empty connecting snapshot.

The relay lane follows the OpenAgents
`openagents.agent_chat_parity_fixture.v1` contract at revision
`3d7c49d4fdc3215802707088242e709dbe902932`. It requests 50-event history
pages, waits for the required history and relay-state `EOSE` frames, accepts
the relay's three-element `EOSE`, and moves through connecting, replaying,
current, stale, and reconnecting states without removing valid rows. A
reconnect asks from one second before the newest retained event and removes
duplicates by event ID. This deliberately preserves the web contract's current
profile-influenced reconnect cursor until both clients revise that contract
together.

Only verified events for the selected `h` group enter the projection. It
renders kind 9 messages, kind 1337 code messages, kind 0 profile facts, kind 5
author tombstones, authorized kind 9005 moderator tombstones, kind 7 reaction
counts, and the newest verified relay-self kind 39005 pin state. The current
kind 39001 administrator set replaces older sets; it is not a historical
union. A malformed frame, rejected event, profile failure, or media failure
sets a bounded visible gap and leaves valid signed messages in place. When no
relay-self key is pinned, messages remain readable and group metadata is
explicitly untrusted. The trust warning and the lifecycle or gap warning are
separate. One warning cannot hide the other. Only a current channel with no
messages uses the quiet state. Connecting and replaying use history states.
Disconnected, reconnecting, and stale channels use distinct empty states.

The main view uses a virtual tail-follow list. A reader who scrolls away from
the tail is not pulled back; **Jump to latest** restores follow mode. **Load
older** issues one bounded page request and keeps same-second boundary events.
Content warnings hide content and media controls until the reader reveals the
message. Tombstones always win and never expose deleted content. Only projected
HTTP(S) links open. Nostr references stay inert pending a reviewed desktop
handler. **Inspect** opens bounded event facts instead of raw relay JSON.
Media facts include the signed URL, MIME type, digest, size, and current media
state.

Remote media never loads because an event arrived or because a row rendered.
The reader must select **Load media**. The loader sends no credentials or
referrer, revalidates redirects and public hosts, caps response bytes, checks
the signed MIME type, and calculates SHA-256 before decode or storage. A digest
mismatch never reaches an image or native viewer. Verified static raster images
render from memory. Verified audio and video use a second native-open action;
other verified files use a save action. The signed message stays visible for
all mismatch and unavailable states.

This slice originally added no composer, signer, NIP-42 response, publish
event, join, leave, identity change, or moderation control. The read-only
restriction is superseded by `OMEGA-DELTA-0182` for the one pinned alpha tester
destination; the relay reader, timeline verification, media gate, and
selected-channel isolation described here remain in force. The global HTTP
client can resolve a validated DNS name again after the media host check, so a
DNS rebinding time-of-check/time-of-use gap remains until Omega has a pin-aware
HTTP transport. AVIF is verified and offered to the native viewer because the
current image build does not decode it inline.

- **Enforced by:** `public_channel_timeline_is_live_verified_and_bounded` in
  `crates/omega_deltas`; the exact fixture projection, lifecycle, reconnect,
  pagination, invalid-input, AUTH-inert, and close tests in
  `omega_public_channel_relay` and `omega_public_channel_timeline`; a loopback
  WebSocket test that runs the production relay driver through both required
  `EOSE` frames, forces a disconnect, accepts the replacement connection, and
  proves retained duplicate-free repair; rendered GPUI tests for lifecycle,
  empty, gap, trust, pagination, content-warning, event-detail, and all five
  media states; the
  gated fetch, redirect, digest, MIME, image-bound, native-open, and failure
  tests in `omega_public_channel_media`; and channel isolation tests in
  `omega_public_channels` and `omega_public_channel_view`.
### OMEGA-DELTA-0164 — A symlink with no target is not an error

- **Upstream Zed:** `BackgroundScanner::scan_dir` logs
  `error reading target of symlink ...` at `ERROR` for every symlink it cannot
  canonicalize.
- **Omega:** the same case logs at `DEBUG`, and the message says what actually
  happened — the symlink has no target and is being skipped.
- **Why:** `pnpm` installs one symlink per platform for every optional native
  dependency, and all but one of them are broken by design on any given
  machine. Opening a folder holding a few `node_modules` trees produced
  thousands of `ERROR` lines in `~/Library/Logs/omega-rc/omega-rc.log` during
  the 2026-07-27 `0.2.0-rc21` incident, and 278 survived in the retained tail.
  A condition that is expected, unactionable, and produced in bulk is not an
  error: at `ERROR` it hides the faults an operator opens the log to find, and
  it charges formatting and I/O for the privilege.
- **Enforced by:** `dangling_symlinks_are_not_logged_as_errors` in
  `crates/omega_deltas`.

### OMEGA-DELTA-0165 — A directory its creator tagged as cache is not walked

- **Upstream Zed:** the worktree scanner descends into every directory that no
  `.gitignore`, and no `file_scan_exclusions` glob, excludes.
- **Omega:** a directory holding a `CACHEDIR.TAG` file with the standard
  signature is treated exactly as a gitignored directory is — its contents are
  not scanned, the directory stays visible in the project panel, and expanding
  it scans it. Controlled by `project.worktree.skip_tagged_cache_dirs`,
  default `true`.
- **Why:** on 2026-07-27 the owner opened `~/work` in `0.2.0-rc21`. Nested
  `.gitignore` files were being honoured correctly — 5,835,451 entries on disk
  reduced to roughly 856,000 scanned — but a large part of what remained was
  Cargo `target` directories placed *beside* their checkouts
  (`omega-worktrees/cwd-fix-target`, `.cargo-target-slim`, and others), so no
  repository's `.gitignore` covered them and no name-based exclusion would have
  matched them either. What does identify them is the tag Cargo itself writes.
  The [Cache Directory Tagging Specification](https://bford.info/cachedir/) is
  already honoured by `rsync`, `borg`, `restic` and `tar`; a directory whose own
  creator declared it regenerable is not worth a worktree entry each.
- **The signature is checked, not just the name.** A file merely called
  `CACHEDIR.TAG` cannot hide a directory the operator meant to open.
- **The tag is remembered, not re-derived.** A tagged directory is still
  watched, and a build writes into it constantly. Answering those filesystem
  events from `.gitignore` alone would let the first `cargo build` re-expand
  exactly what the tag said to skip, so the tagged paths are recorded on the
  snapshot and `ignore_stack_for_abs_path` consults them.
- **Enforced by:** `tagged_cache_dirs_are_skipped_by_default` in
  `crates/omega_deltas`, plus `test_tagged_cache_dirs_are_not_descended` and
  `test_a_write_inside_a_tagged_cache_dir_does_not_expand_it` in
  `crates/worktree`.

### OMEGA-DELTA-0166 — A folder too large to index says so

- **Upstream Zed:** a worktree scan is unbounded. It walks until it runs out of
  directories, whatever that costs.
- **Omega:** the scan stops at `project.worktree.max_scan_entries` entries
  (default 150,000), records the bound on the snapshot, logs it, and shows a
  notification naming the folder, the bound, and the setting that changes it.
  `0` restores unbounded scanning.
- **Why:** `~/work` cost the owner 852 MiB of resident memory 30 seconds after
  he selected it, 1,466 MiB within six minutes, and repeated hang-detector
  reports from `fs.rs`, `git/repository.rs` and `project/git_store.rs` — all of
  them downstream consumers walking the entry set the scan produced. No
  exclusion list can be complete, because the next machine has a differently
  shaped pile of files on it. A bound can be complete.
- **Bounded, not refused, and never silent.** The owner's instruction was
  explicit: *"I don't care if they are big"* and *do not* cap "without telling
  the person". So the folder opens, the parts that were indexed work normally,
  and the notification states what was left out and how to index all of it.
- **A bounded folder is still a working folder.** The bound stops the scan
  spreading on its own; it does not refuse a path someone asked for. Expanding
  a directory in the project panel, or opening a file under it, still scans it,
  and a worktree that later drops below the bound resumes scanning without a
  restart. The bound is re-checked when a queued directory is picked up rather
  than only when it was enqueued, so the work already in flight when the bound
  was crossed cannot overshoot it.
- **Enforced by:** `worktree_scans_are_bounded_by_default` in
  `crates/omega_deltas`, plus `test_scan_stops_at_max_entries_and_says_so` and
  `test_scan_below_max_entries_is_not_truncated` in `crates/worktree`.

### OMEGA-DELTA-0167 — Phone pairing has no required configuration

- **Upstream Zed:** has no phone pairing, so no configuration surface exists to
  diverge from. This delta records a divergence from Omega's own earlier shape.
- **Omega before:** `production_sarah_conversation` refused to build unless six
  environment variables were present — the relay list, Sarah's public key, the
  admitted-device allowlist, the device scope set, and the direct bridge's
  MagicDNS name, port, and bind address.
- **Omega now:** every one of those has a built-in default. The relay is
  `wss://relay.openagents.com`, Sarah's public key is the production bridge
  identity, the scope set is `observe_issue31` alone, the allowlist starts
  empty, and the direct bridge defaults to the live Tailscale MagicDNS name
  bound on the CGNAT IPv4 when Tailscale is up, or `localhost:4317` on
  `127.0.0.1` when it is not (so a same-machine simulator still pairs). Each
  environment variable survives only as a development override, and a blank
  value counts as unset.
- **Why:** an installed app is launched from Finder, which passes on no shell
  environment. A value the pairing runtime refuses to start without therefore
  cannot live in an environment variable, because it is guaranteed absent in
  exactly the build that ships. v0.2.0-rc22 shipped this way: pressing
  **Pair phone** answered `OPENAGENTS_OMEGA_NOSTR_RELAYS is not configured`,
  and there was no way for the owner to make it say anything else.
- **The relay variable also killed the lane that needs no relay.** The direct
  loopback bridge is constructed inside the same function, so a missing relay
  variable returned before any endpoint was resolved, and the endpoint then
  defaulted to none. QR pairing over loopback, which never contacts a relay,
  died of relay configuration. The endpoint is now always present.
- **The allowlist is state, not configuration.** It is what pairing produces.
  A fresh install legitimately admits no device, and
  `issue_direct_pairing_grant` admits the first one only after that device
  proves possession of its key. An empty allowlist is no longer applied to the
  host controller, which both keeps the one-to-32 policy check intact and stops
  a restart overwriting the admissions restored from durable state.
- **No default carries authority.** OMEGA-DELTA-0154 makes the mirror
  read-only, so `observe_issue31` is the whole of the default scope set. A
  default naming a command or steering scope would hand a phone authority the
  mirror does not have.
- **A half-set override is refused by name.** Advertising a MagicDNS name with
  no port, or a port with no name, still fails — but the message says which
  half is missing and what unsetting both restores, instead of `X is not
  configured`.
- **Enforced by:** `phone_pairing_needs_no_configuration` in
  `crates/omega_deltas`, plus
  `pairing_configuration_is_complete_with_an_empty_environment`,
  `a_fresh_install_admits_no_device_and_grants_only_observation`,
  `no_default_scope_carries_command_authority`,
  `the_environment_still_overrides_every_default`,
  `a_blank_override_falls_back_to_the_default`, and
  `half_an_endpoint_override_names_the_missing_half` in `agent_ui`.

### OMEGA-DELTA-0168 — An internal issue number never renders to a person

- **Upstream Zed:** not applicable; this records a divergence from Omega's own
  earlier wording.
- **Omega before:** the device-mirror feature grew under the internal codename
  "Issue 31", and the codename saturated its runtime prose: pairing errors
  ("Issue 31 pairing response has no challenge"), scope-policy refusals
  ("Issue 31 device scope policy is invalid"), durable-state refusals
  ("durable Issue 31 host state does not match this identity or relay
  configuration"), receipt-validator reasons ("issue 31 host adjunct …"), and
  host-bridge log lines ("Issue31 mobile command …"). Any of these could reach
  the **Pair phone** popover through
  `DevicePairingSurface::Unavailable(error.to_string())`, a refusal notice, or
  a log-derived surface — and one did, on the owner's machine.
- **Omega now:** every string literal that reads as prose names the product —
  device mirror, device pairing, phone pairing, desktop mirror — never the
  issue number. Internal Rust symbols (`issue31_*` functions, `Issue31…`
  types, module and file names) stay: they are invisible. Machine tokens stay
  in their exact wire form: the grant scope `observe_issue31` is validated
  byte-for-byte by the shipped mobile client (TestFlight build 126), and the
  `openagents.omega.issue31.*` schema ids, `action.issue31.*` /
  `idempotency.issue31.*` refs, `snapshot.omega.issue31.*` refs, the
  `omega-issue31-host` discovery `t` tag, and the `issue31-*.json` state
  filenames are contract or durable-state identifiers. None of them may be
  displayed raw; a surface that shows a scope shows a product description of
  it instead.
- **Why:** the owner, on a live build, from the **Pair phone** popover:
  *"FIX IT NOW AND NEVER EVER SHOW ISSUE 31."* An internal issue number on a
  consent screen or an error popover tells a person nothing and tells them we
  forgot they were there. Wire compatibility is the one reason a codename
  survives at all, and it survives only below the display boundary.
- **Enforced by:** `internal_issue_references_never_render` in
  `crates/omega_deltas`, which extracts every string literal in `crates/`
  (excluding the deltas crate itself, whose literals quote internal symbols by
  design) and fails on the prose forms `issue 31` and `issue31 `.

### OMEGA-DELTA-0169 — Stale device mirror state is archived, not a dead end

- **Upstream Zed:** has no phone pairing and no durable device mirror state.
- **Omega before:** `load_issue31_host_state` hard-refused when the persisted
  state file could not be used — a schema from another build, an identity or
  relay configuration that no longer matched the stored controller, a failed
  integrity bound, or unparseable JSON. After an identity reset the refusal
  was permanent: **Pair phone** answered
  `durable Issue 31 host state does not match this identity or relay
  configuration`, nothing said what to do, and pairing stayed bricked until
  someone deleted the file by hand. Deleting the file by hand and pressing the
  button again rendered the QR immediately, which proved a fresh re-derive is
  both safe and sufficient.
- **Omega now:** a state file that can never match again is set aside beside
  itself (`issue31-nostr-host-state.json.stale-<nonce>.bak`), one log line
  names exactly what mismatched, fresh state is derived, and pairing proceeds.
  The archive is a rename, not a delete, because the file carries the
  admitted-device grants, pending phone commands, command idempotency
  records, and unpublished outbox items someone may want to inspect. Every
  recoverable class re-derives: wrong schema version, identity or relay
  mismatch, failed integrity bounds, invalid quarantine or acknowledgement
  ledgers, a stale published discovery record, unparseable JSON, and an
  oversized regular file. One class still refuses: a path that is not a
  regular file (a symlink or directory), because renaming something another
  actor planted is not this host's call.
- **The honest cost, stated where it happens:** archiving drops previously
  admitted phone pairings on this host — and with them the command
  idempotency ledger — so paired devices pair again by scanning a new QR
  code. The log line says so. A bricked pairing flow is worse.
- **Enforced by:** `stale_device_mirror_state_archives_and_rederives` in
  `crates/omega_deltas`, and
  `stale_identity_durable_state_is_archived_and_rederived_instead_of_bricking_pairing`
  in `omega_effectd`, which writes state under one identity, loads it under
  another, and requires a fresh start with the stale file archived beside it.

### OMEGA-DELTA-0170 — Enter while connecting queues the message

- **Upstream Zed:** there is no composer while an agent connects, so there is
  no Enter to honour or refuse. The question only exists because
  `OMEGA-DELTA-0122` put a typable composer into the wait.
- **Omega, before this:** a `Chat` during `Loading` was refused. The text
  stayed in the composer and the status line read *"Not sent — still
  connecting. Press Enter again when this clears."* The refusal was 0122's
  deliberate answer to the switched-executor case, and it made every first
  message of every new thread cost a second keystroke whenever the executor
  was still warming. The owner, on a live build, having typed "hi" into a
  brand-new thread and pressed Enter: *"refactor this 'not sent' bullshit.
  never block user from hitting enter, if not connected just show a loading
  thing in the chat."*
- **Omega now:** Enter always accepts. `submit_while_connecting` moves the
  composer's text into `pending_connect_messages` on the `ConversationView`,
  the composer clears, and the message is drawn in the chat as a pending turn
  — the app's own `TodoProgress` spinner, plus a line naming the executor it
  will go to. Several Enters queue several turns, in order. The Send button
  does the same thing, because a dead button beside a live Enter would make
  two claims about one state.
- **On connect it dispatches itself, in order, exactly once.**
  `dispatch_pending_connect_messages` runs where the thread view is built. It
  `std::mem::take`s the pending list — taken before anything is enqueued, so
  no later state churn can dispatch a message twice — and feeds every message
  into the thread's ordinary `MessageQueue`: the first is fast-tracked exactly
  the way Enter on an empty composer fast-tracks a queued follow-up, and the
  rest auto-dispatch as turns stop. Ordering, exactly-once, and the subagent
  fence (`43219aacd1` — a delegated subagent finishing must not spend the root
  queue) are that queue's existing promises; a parallel send path would
  re-earn its bugs one at a time.
- **The visibility is what answers 0122's worry.** The old refusal existed
  because a person who reaches the wait by switching executor would have a
  message fired at a runtime nobody watched it go to. The pending turn is the
  watching: it sits in the chat naming the executor it is waiting for, and
  cancelling is deleting the text in front of you before the connection lands
  — the same control a queued follow-up gives.
- **A terminal failure never eats the message.** The pending list lives on the
  view, not on `ServerState::Loading`, so `handle_load_error` inherits it
  untouched. In `LoadError` the same turns render marked unsent — text intact,
  `Not sent — {executor} failed to connect` — with a **Retry Connection**
  button that calls `reset`, re-entering `Loading` with the list intact; a
  retry that connects dispatches the preserved messages. This repository just
  finished paying for silently dropped queued text; this delta does not
  reintroduce the class.
- **The draft handover is untouched.** Text a person typed but did not submit
  still crosses via `hand_loading_draft_over`, caret and all, and the handover
  still never sends: pressing Enter is what submits, and only submitted text
  rides the pending queue.
- **Enforced by:**
  `enter_while_connecting_queues_the_message_and_never_loses_it` in
  `crates/omega_deltas`, plus two GPUI regression tests in
  `crates/agent_ui/src/conversation_view.rs`:
  `a_message_sent_while_connecting_dispatches_once_on_connect` (submit two
  messages while `Loading`, connect, prove the first dispatches exactly once
  with the exact text, the second waits in the ordinary queue and dispatches
  when the turn stops) and
  `a_message_sent_while_connecting_survives_a_terminal_failure` (submit,
  terminally fail, prove the text is preserved and the retry dispatches it
  exactly once). Each was watched failing against the fix broken one piece at
  a time: the dispatch call removed (both GPUI tests failed on "connecting
  must dispatch" / "a retry that connects must dispatch"),
  `handle_load_error` made to clear the pending list (the survival test failed
  on "must preserve the submitted text"), and the delta check run against the
  pre-change source shape.
- **What this does not cover.** No window has been opened by the checks, so
  nothing here proves what a person sees: that the pending turn is where the
  transcript will be, that the spinner spins, or that the failed turn's retry
  is reachable in a narrow panel. The auth-required state renders the pending
  turns and a successful sign-in rebuilds the session through the same
  dispatch path, but no automated check walks that flow end to end.

### OMEGA-DELTA-0171 — Pairing defaults to the live tailnet, not localhost

- **Upstream Zed:** has no phone pairing.
- **Omega before:** `OMEGA-DELTA-0167` gave every pairing input a built-in
  default so Finder launches could pair without a shell environment. The
  direct bridge defaults were `localhost:4317` bound to `127.0.0.1` — correct
  for a same-machine simulator, fatal for a real phone. A phone that scanned
  the QR dialed `ws://localhost:4317` and saw its own loopback answer nothing.
- **Omega now:** when `tailscale status --json` reports a MagicDNS name and a
  CGNAT IPv4, those become the product default for the QR endpoint and the
  bind address. When Tailscale is down or the binary is missing, the loopback
  fallback remains. Environment overrides still win. Discovery runs once per
  process.
- **Why:** the owner, on a live phone after a successful decode: *"Could not
  connect to ws://localhost:4317"* and *"should that be a tailnet URL?"* Yes.
  A default a phone cannot reach is not a default.
- **Enforced by:** `a_live_tailnet_becomes_the_default_pairing_endpoint` and
  `live_tailnet_is_parsed_from_tailscale_status_json` in `agent_ui`, plus
  `phone_pairing_prefers_the_live_tailnet` in `crates/omega_deltas`.

### OMEGA-DELTA-0172 — One cloud-native Omega Agent in the zero-base input bar

- **Upstream Zed:** full model picker with every provider model.
- **Omega before:** zero base hid the model picker; the default stayed
  `google/gemini-3.6-flash` with no Pro path to hosted Kimi K3.
- **Omega originally:** a **Flash | Pro** dropdown. Flash (default) selected
  `google/gemini-3.6-flash`; Pro selected `openagents/kimi-k3` through the
  OpenAgents language-model provider, which streams OpenAI-compatible
  completions via the signed-in OpenAgents session to
  `POST /api/v1/chat/completions`.
- **Amended 2026-07-30 (owner direction: "CORE OMEGA AGENT MUST ALWAYS HIT
  OUR API AND WORK"):** the control is now **Luna | Flash | Pro**. Luna
  (default) selects `openagents/gpt-5.6-luna` on the hosted OpenAgents lane.
  Flash stays reachable as the Gemini backup. Pro stays `openagents/kimi-k3`.
  OpenAgents cloud accepts the user bearer for those exact hosted lanes
  (gemini-3.6-flash, kimi-k3, gpt-5.6-luna).
- **Amended 2026-07-31 by `OMEGA-DELTA-0202`:** Flash selected
  `openagents/gemini-3.6-flash`, not `google/gemini-3.6-flash`, so all three
  tiers used hosted lanes.
- **Amended 2026-08-08:** the input bar now presents one **Omega Agent** and no
  model or reasoning selector. The client sends the logical
  `openagents/omega-agent` id to the OpenAgents Responses API. The API owns the
  provider model, reasoning policy, routing, and fallback. This lets routing
  change without an IDE release and prevents the UI from promising a provider
  model before the server has made its decision. External agents can still
  expose their own model and configuration controls.
- **Enforced by:** `language_models` OpenAgents provider tests,
  `omega_agent_picker_has_no_client_model_controls` in `agent_ui`, and
  `zero_base_input_bar_offers_one_cloud_native_omega_agent` in
  `crates/omega_deltas`.

### OMEGA-DELTA-0173 — One flag-free surface creates three kinds of conversation

- **Omega's product contract:** a normal launch has one surface and no launch
  mode choice. From that surface, a person creates a **Direct Agent**,
  **Omega Agent**, or **Sarah** conversation. This is the reconciliation of the
  owner's 2026-07-29 single-experience and Episode 263 directions: one
  experience is not one agent.
- **Choice happens at creation.** Before the first send, the surface names the
  mode, concrete executor, project, and readiness. A conversation then keeps
  that ownership underneath its transcript. Direct Agent uses the selected ACP
  agent; Omega Agent routes among eligible executors and discloses its concrete
  choice; Sarah owns the voice session and states its authority and limits.
- **Truthful labeling survives the supersession.** Every title, composer label,
  pending state, status, and turn disclosure names the executor that actually
  does the work. A pending change distinguishes “will be” from “is”, and no
  readiness observation silently substitutes an executor.
- **The shell converged.** The legacy `--full-editor` compatibility path was
  removed by omega#161: there is one launch surface and no mode vocabulary.
  `OMEGA-DELTA-0048`, `0052`, and `0053` keep the hiding, refusal, and sealing
  mechanisms honest until omega#162 deletes the gated legacy crates.
- **Vim stays.** The `vim` crate and `assets/keymaps/vim.json` remain in the
  kept product closure. Modal editing belongs in the composer and focused
  editing surface; admitting its action set and re-homing its mode indicator
  are implementation work, not an open product decision.
- **Enforced by:** `one_surface_three_conversation_modes_is_the_product_contract`
  in `crates/omega_deltas`, plus the creation and restore behavior tests that
  land with the three-mode front door.

### OMEGA-DELTA-0174 — A user-triggered center open is visible before it succeeds

- **Upstream Zed:** editor opens assume the Workspace center is rendered.
- **Omega before:** sealed Zero Base could successfully open or activate a
  center item without revealing the center. The operation stole focus while
  Files, tool locations, artifact sources, skill and rule links, debug JSON,
  the ACP registry, and file-like URLs remained invisible.
- **Omega now:** `Workspace::reveal_zero_base_center_for_user_open` is the one
  semantic boundary for default-surface center opens. It reveals first and is
  a no-op outside sealed Zero Base. Low-level `open_path`, `open_abs_path`,
  `add_item`, and activation APIs remain unchanged so restores and background
  work do not acquire presentation side effects.
- **The exceptions stay explicit.** Read-only file and Markdown peeks keep the
  modal reader. `AgentDiff::review_in_active_editor` advances among files in an
  already-visible editor and does not reveal a new surface. HTTP, HTTPS, and
  other external URL schemes still go to the system without revealing the
  Workspace center.
- **Lifecycle is unchanged:** closing the final revealed center tab restores
  the agent-only surface under `OMEGA-DELTA-0139`.
- **Enforced by:** `default_surface_center_opens_reveal_before_opening` and the
  amended `transcript_file_links_choose_editing_or_peeking` checks in
  `crates/omega_deltas`, plus focused Workspace URL-routing tests.

### OMEGA-DELTA-0175 — Vim editing and its mode readout belong to the default surface

- **Upstream Zed:** Vim actions run in editors, `workspace::ToggleVimMode`
  enables them, and `vim::ModeIndicator` lives in the Workspace status bar.
- **Omega before:** the process-wide zero-base action gate refused the entire
  `vim` namespace. The rc refusal inventory contains eleven unique actions:
  `PushFindForward`, `InsertBefore`, `Down`, `InsertLineBelow`, `Substitute`,
  `PushDelete`, `InsertAfter`, `Number`, `Up`, `PreviousLineStart`, and
  `InsertLineAbove`. The default surface also removes the Workspace status
  bar, so enabling Vim would still leave its current mode invisible.
- **Omega now:** the exact non-Helix editor action set mechanically derived from
  `assets/keymaps/vim.json` is admitted. Helix-flavored actions and pane, tab,
  Project Panel, and Workspace management stay refused. The exact
  `workspace::ToggleVimMode` action is admitted because its control must work
  while Vim is off; `workspace::Save` remains admitted for a revealed editor.
- **One readout follows focus.** Workspace initialization constructs one
  `vim::ModeIndicator` before center editors or Agent Panel. Agent Panel passes
  that entity into every `ConversationView` and `ThreadView`, and loading plus
  the active connected composer render it at bottom left. Stored observer
  subscriptions and a weak callback owner keep its lifetime bounded. Its stable
  `vim.mode-indicator` debug selector and status label expose the same state to
  accessibility and UI automation.
- **Enforced by:** `vim_is_admitted_with_one_shared_default_surface_indicator`
  in `crates/omega_deltas`, the zero-base admission unit test, and focused GPUI
  coverage in `agent_ui` that exercises loading-to-connected entity reuse and
  focused Vim mode changes.

### OMEGA-DELTA-0176 — The application menu exposes only working default-surface paths

- **Upstream Zed:** the native menu bar advertises the full editor product,
  including File, Selection, Go, Run, Save As, onboarding, and extension paths
  that Omega's default surface does not currently make honest or useful.
- **Omega now:** the menu bar has exactly six top-level menus: **Omega**,
  **Edit**, **View**, **Thread**, **Window**, and **Help**. Their items are the
  approved default-surface contract. Find opens thread search; View exposes
  zoom, full screen, the Threads Sidebar, and the six Workbench surfaces;
  Thread creates a thread or chooses its folder; Help opens documentation and
  bundled licenses. macOS alone retains Services, Hide, Hide Others, and the
  automatic Window list, while every platform retains Minimize.
- **Every enabled row reaches a live path.** The precise `omega::About`,
  `omega::AcpRegistry`, `omega::OpenDocs`, and `omega::OpenLicenses` actions
  are admitted without admitting the wider namespace. Opening bundled licenses
  reveals the sealed center before adding its item. Workspace-level forwarding
  makes Threads Sidebar and Workbench menu actions work even when focus is in
  the center; a surface that cannot open reports its reason instead of silently
  doing nothing.
- **Unavailable creation modes say why.** Sarah remains visible but disabled
  as `Sarah Voice — Voice access is not available yet`. Direct-agent rows are
  disabled with a concrete zero-base, missing-folder, or shared-project reason.
  **Add More Agents** remains enabled and opens the ACP Registry. The
  direct-unavailable portion is superseded by OMEGA-DELTA-0178; shared projects
  remain the product-level refusal.
- **No borrowed or dead shortcut remains.** Save As is absent from the menu and
  all three default keymaps. Sarah's former `cmd-shift-s` / `ctrl-shift-s`
  binding is also absent; issue #154 owns a future deliberate voice shortcut.
  Menu shortcut actions are resolved against each shipped macOS, Linux, and
  Windows keymap.
- **Enforced by:** `the_default_surface_has_one_honest_menu_contract` and
  `honest_menu_shortcuts_resolve_on_every_platform` in `crates/omega_deltas`,
  `the_application_menu_is_the_approved_six_menu_contract` in `zed`, the exact
  zero-base admission unit test, and mounted `agent_ui` coverage that dispatches
  menu actions while focus is outside Agent Panel.

### OMEGA-DELTA-0177 — New conversations cross one typed three-mode boundary

**Superseded in part by OMEGA-DELTA-0184 (omega#165).** The full-screen
three-row chooser this delta shipped was the owner's top UX complaint —
anything between `+` and a blinking cursor is friction — and no longer exists.
`agent::NewThread` and the Thread menu now land directly in a focused composer
on the default executor, and the selection surface is the composer's executor
dropdown. Everything below that is typed law rather than screen description —
receipts, ownership immutability, honest readiness, draft retention, and the
startup recheck — survives unchanged behind that dropdown.

- **Omega before 0182:** `agent::NewThread` and the Thread menu opened one
  persistent Agent Panel front door with exactly three rows: **Direct Agent**,
  **Omega Agent**, and **Sarah**. Every row showed its exact agent or router
  selection, folder, and one of four closed readiness states before send.
- **Ready is proved, not inferred.** A row can report Ready only with a receipt
  bound to its exact target and a session created by that target. Executable,
  configuration, registry, and path detection do not count. The prepared Omega
  `ConversationView` is the entity selection reveals; preparation is never a
  disposable probe followed by a second session.
- **Ownership is immutable.** Direct targets carry a validated, non-empty ACP
  agent id and may not substitute Omega. Existing Agent serialization and
  thread metadata persist the conversation owner, while disclosure records the
  executor Omega routed to. No redundant mode column or volatile readiness
  receipt is persisted.
- **Unavailable modes remain truthful.** Direct Agent and Sarah are permanent
  rows but report **Not supported in this build** until their providers land.
  Disabled rows have no activation handler and cannot fall back to Omega. A
  generic new-thread action never repeats a previously selected terminal;
  `agent::NewTerminalThread` owns terminal creation.
- **Typed compatibility actions keep their request.** In sealed zero base,
  `agent::NewExternalAgentThread` preserves the exact requested ACP identity,
  selects the Direct Agent row, and shows its unsupported state. It never calls
  the legacy retarget clamp or activates the prepared Omega conversation. The
  legacy editor-surface draft-creation branch is dead code since omega#161 and
  is deleted with the gated crates in omega#162.
- **Loading text owns its conversation.** Draft retention covers the loading
  editor, accepted pre-connect messages, the connected message queue, draft
  prompt, and composer. Opening the front door cannot discard or retarget text
  during any connection handoff phase. A sessionless prepared conversation is
  persisted with its exact owner and work directories before its first send,
  then promoted in place when the physical session is created.
- **Startup does not overwrite restored state.** The asynchronous startup task
  rechecks the mounted panel before presenting the front door. A conversation,
  terminal, Full Auto surface, pending terminal restore, or typed draft that
  appeared while startup waited remains selected and usable.
- **Compatibility scope:** the legacy editor-surface executor selector and
  rebuild seam became unreachable when omega#161 removed `--full-editor`.
  Default front-door selection never calls either retarget path.
- **Legacy ownership is ambiguous.** New metadata writes persist the exact
  conversation owner. Older inactive rows may hold either an owner or an
  executor, and the current schema and restore path do not distinguish those
  meanings. This delta therefore makes no claim that existing restoration is
  a safe ownership proof; agent-scoped route journals are insufficient to make
  it one. #152 must add versioned owner semantics before Direct Agent becomes
  available.
- **Enforced by:** `the_three_mode_front_door_claims_one_exact_prepared_conversation`
  in `crates/omega_deltas`, the typed core tests in `omega_front_door`, and
  focused Agent Panel GPUI tests for row order, receipt/entity reuse, metadata
  ownership, and unavailable-mode refusal.

### OMEGA-DELTA-0178 — Direct Agent owns one exact ACP session

- **Supersedes the Direct Agent refusal in OMEGA-DELTA-0176 and
  OMEGA-DELTA-0177.** Installed Codex (`codex-acp`), Claude Code
  (`claude-acp`), registry Grok (`grok-build`), legacy custom Grok (`grok`),
  and generic ACP agents such as `opencode` are selectable in zero base. IDs
  remain exact; `grok` and `grok-build` are never aliases.
- **One target, view, and session.** A Direct selection replaces an unused
  off-screen preparation with `Agent::Custom { id }`. Loading, native auth and
  configuration, setup failure, retry, composer identity, and Ready receipt
  all belong to that same `ConversationView`. Activation claims it rather than
  creating a second session, and no Direct path substitutes Omega Agent.
- **The registry returns to the front door.** **Add More Agents** opens the
  visible ACP Registry. Its installed-agent `SelectAgent` action updates the
  active front-door target and immediately prepares that exact id, even though
  the registry temporarily owns the center surface.
- **Ownership is versioned.** `sidebar_threads.conversation_owner_version = 1`
  means `agent_id` is the immutable conversation owner. Pre-version native-null
  rows remain `LegacyOmega`; pre-version non-null rows are
  `LegacyAmbiguous` and visibly refuse restore rather than guessing. Existing
  rows preserve the `(agent_id, version)` pair atomically. Unknown future
  versions fail closed. Read-only cross-channel import detects older schemas
  and classifies their rows as legacy without migrating another channel's DB.
- **Identity follows the active owner.** Direct loading and connecting labels,
  toolbar text, transcript restore, and sidebar reopen derive from the active
  exact `ConversationView` or v1 metadata, never from the process-global Omega
  router selector or a last-used native clamp.
- **Refusal never launches a substitute.** Shared projects keep the requested
  Direct target visible but start neither that ACP agent nor Omega. Deleting an
  ambiguous legacy archive removes local metadata and worktree records without
  reconnecting an unproven agent for remote deletion.
- **Test coverage:** `direct_agents_restore_exact_owners_without_native_fallback`
  in `crates/omega_deltas`; metadata migration and cross-channel tests; focused
  Agent Panel tests for exact preparation and restore, registry selection,
  setup failure, owner switching, toolbar front-door precedence, and zero-base
  external actions; plus the archive remote-delete ownership helper test. The
  shared-project refusal remains an explicit fail-closed source policy pending
  installed lifecycle proof.

### OMEGA-DELTA-0179 — Omega routes the first request to one exact ready executor

- **Supersedes OMEGA-DELTA-0029's native fallback and OMEGA-DELTA-0150's
  unpinned-native law.** Omega Agent prepares a logical conversation and a
  typed executor inventory before it creates a physical executor session. The
  normalized first request is therefore an input to the routing law instead of
  text that arrives after routing has already happened.
- **The input and decision are records.** A versioned `RouteInputs` contains
  normalized task requirements, a stably ordered inventory of exact executor
  identities and readiness, and an optional exact new-conversation override.
  The pure route law returns one exact identity, class, reason, and any bounded
  pre-decision fallback or hard refusal. Discovery order, clocks, randomness,
  process globals, and model calls are outside that law.
- **The first automatic candidate set is deliberately narrow.** Omega may
  automatically choose its native loop or a ready ordinary ACP executor. An
  engine lane is Full Auto authority and still requires its separate explicit
  human gesture; reporting engine capacity does not admit it as an automatic
  chat candidate.
- **An override is a one-conversation gesture.** It wins over automatic
  routing, names an exact executor rather than only a class, and is consumed by
  the new conversation. It cannot retarget a transcript or leak to the next
  conversation. An unavailable override is a named refusal, not permission to
  run elsewhere.
- **Persistence precedes dispatch.** Omega durably writes the complete inputs
  and decision under a router-generated dispatch reference before calling an
  executor. After the executor mints a session id, the journal binds that id to
  the same receipt. A failed write blocks the send. Restore reads the recorded
  decision and never recomputes it from current readiness.
- **A selected executor is immutable.** If it disappears before or during a
  send, load, resume, retry, cancellation, or a later turn, the error names the
  exact executor. Missing or corrupt route state also fails visibly. Neither
  case silently substitutes the native loop or replays a request whose tools
  may already have produced effects.
- **Disclosure is part of the dispatch boundary.** Before the first request can
  reach an executor, the thread shows the exact selection, task-requirement
  summary, reason, override or automatic mode, and fallback or refusal. Zero
  base and the ordinary thread surface read the same typed durable receipt.
- **Enforced by:** deterministic routing and canonical-record tests in
  `omega_front_door`; journal restart, exact multi-ACP dispatch, and
  disappearance tests in `agent_ui`; source contracts in `omega_deltas`; and
  the installed positive and negative routing receipts owned by the strict
  installed gate.

### OMEGA-DELTA-0180 — Sarah voice starts behind one visible admission boundary

- **Supersedes Sarah's disabled state in OMEGA-DELTA-0176 and
  OMEGA-DELTA-0177.** The permanent Sarah row, the Thread menu, and the
  toolbar `+` menu now open one Sarah admission surface inside Agent Panel.
  They do not open the legacy Sarah dock panel and do not start the microphone.
- **Every named entry point actually opens the surface** (omega#168). A
  `ContextMenuEntry` with only `.action(...)` names the keybinding and the
  keyboard dispatch path; the pointer click runs the entry handler, whose
  default is a no-op — rc27 shipped the `+` menu's Sarah row that way and it
  clicked into nothing on every profile. The row must carry a live click
  handler that routes through the same panel admission path the action
  reaches, so click and keybinding cannot diverge.
- **Admission truth precedes audio.** A dependency-neutral workspace
  projection carries the effective rate in msat per million tokens, credit
  hold, remaining credit, maximum duration, cohort reference, transcript
  policy, exact bounded capabilities, and the confirmation class for each
  capability. A cohort refusal remains a refusal even when the account has
  credit. The staging-owner entitlement renders as not metered rather than a
  fabricated zero balance. Only a Ready projection renders **Start voice**;
  loading and unavailable states keep the microphone off.
- **Reviewed terms bind the reservation.** Ready admission carries a random,
  one-use reference with a maximum 120-second lifetime. Omega sends that
  reference only after **Start voice**, and accepts the resulting ticket only
  when the service echoes the same reference, expiry, profile, cohort, credit
  mode, rate, pre-hold balance, hold, duration, and capability boundary. An
  expired, replayed, missing, or changed admission fails closed and requires a
  fresh review.
- **The persona is deliberately bounded.** Public copy calls Sarah a voice
  editor and delegation assistant. The surface renders the server's exact
  exposed subset of context read, reveal range, replace selection, save
  document, and start agent thread. It also renders the hard-false direct shell,
  direct Git, payment, credential-access, and device-control authorities.
  Confirmed actions do not inherit authority from Sarah's identity.
- **Completion remains visible.** Active admission retains the exact terms and
  session reference, a bounded transcript, any pending command confirmation,
  and the receipt for a Sarah-created Omega Agent thread. **Allow once** and
  **Decline** act on the hidden runtime owner's exact pending request. Settled
  admission retains those artifacts and replaces estimates with the final
  charge, optional remaining credit, receipt reference, and transcript recovery
  result. Missing settlement fields remain unavailable rather than being
  inferred by the desktop.
- **Replacement consent binds one exact editor effect.** A replacement proposal
  retains its workspace reference, active path, document version, selection
  range, selected text, and replacement text. The visible confirmation shows
  the exact path, range, before text, and after text. Omega validates the binding
  both when **Allow once** is pressed and atomically with the edit; workspace,
  document, active-file, or selection drift refuses the command without editing.
- **Compatibility:** the hidden `SarahWorkroomPanel` remains the voice runtime
  owner during the alpha transition. `workroom::StartVoice` remains the runtime
  action, but only the Ready admission button dispatches it. Menu choices and
  idle, unavailable, and retryable composer controls use
  `agent::OpenSarahAdmission`, so no visible entry can bypass the contract.
  `workroom::OpenPanel` is not restored.
- **Enforced by:** `sarah_voice_admission_is_visible_bounded_and_fail_closed`
  in `crates/omega_deltas`; Agent Panel GPUI coverage for Ready and Settled;
  the application-menu contract test; and managed-session projection,
  settlement, reconnect, transcript-recovery, and revocation tests at their
  source boundaries.

### OMEGA-DELTA-0181 — Concurrent agent turns are supervised and isolated (amended 2026-07-31)

- **Every direct agent thread exposes one lifecycle.** The Threads Sidebar and
  active-thread header identify the executor and show the lifecycle as a
  colored icon only (Running, Waiting, Failed, Completed, or Cancelled — words
  live in the one-word tooltip and accessibility label, never as visible status
  text; amended by `OMEGA-DELTA-0189`). Waiting for a person takes precedence
  over a generic running state. Only terminal states survive a restart; an
  interrupted nonterminal record fails closed instead of pretending that work
  is still live.
- **Cancellation is thread-owned.** The active-thread control cancels only that
  thread. Switching threads does not stop background work, and relaunch keeps
  each thread's identity, terminal lifecycle, transcript, and durable queued
  messages independent.
- **Concurrent writers are isolated, and the explicit decision is recorded
  once.** Before a direct root turn begins, Omega claims every selected
  worktree for the lifetime of that turn. Local aliases are canonicalized,
  multi-root overlap is detected, and remote paths are scoped by remote
  identity. When a new thread's root is already held by a live thread, Omega
  provisions a linked git worktree for the new thread and runs it there —
  silently. The person's standing decision lives in `agent.thread_worktree`:
  `isolate` (default) provisions, `shared` declares that concurrent agents may
  write one checkout. When isolation is genuinely unavailable — a session whose
  working directory was fixed when it started, a project with no repository to
  branch from — the collision is disclosed on the thread, naming the occupying
  thread, its executor, and the overlapping path, and the turn proceeds.
- **Amended 2026-07-31 by `OMEGA-DELTA-0214`** (owner direction, on being shown
  the collision modal after pressing New Thread: *"i never want to see that
  shit. figure out better workflow."*): the safety claim of this delta is
  **isolation of concurrent writers**. The modal was only the chosen mechanism,
  and it was the weaker one — it disclosed the hazard and then let a person
  walk into it with one click, once per turn, forever. Auto-provisioning
  satisfies the same property more strongly, because it makes the collision
  impossible rather than merely visible. `Cancel` / `Run here anyway`, the
  `window.prompt` in `request_worktree_admission`, and the
  `WorktreeAdmission::Cancelled` outcome with its prompt-restore path are
  deleted. Nothing about claim scoping, canonicalization, remote identity, or
  turn-scoped claim lifetime changes.
- **The text queue is durable and fail-closed.** A text-only prompt is
  acknowledged as queued only after its text, executor class, steer capability,
  disposition inputs, ordering identity, and per-thread processing state are
  atomically persisted. Images, files, and other rich blocks retain their exact
  content in the live queue and dispatch normally; they are not represented as
  restart-safe because the journal has no rich-content schema. This limitation
  does not reject the message or surface as an error.
  Promotion happens only after worktree admission, terminal items cannot be
  reopened, failed edits remain visibly unsaved, and corrupt storage is never
  overwritten. Restored open queues start paused until a person explicitly
  resumes them; a new process's local Idle state is not provider quiescence.
  Pause, resume, send-now, cancellation, and restart recovery are isolated by
  thread.
- **Enforced by:** `concurrent_agent_supervision_is_visible_durable_and_guarded`
  in `crates/omega_deltas`; lifecycle, collision, metadata migration, and queue
  journal unit tests; `queued_image_is_kept_in_memory_and_dispatches_without_an_error`
  in the Conversation View GPUI suite; Agent Panel GPUI coverage; and the
  concurrent-agent visual workbench sequence.

### OMEGA-DELTA-0182 — Alpha feedback is a pinned, bounded public writer

- **Supersedes the read-only channel restrictions in OMEGA-DELTA-0130 and
  OMEGA-DELTA-0163.** The selected **Alpha feedback · #alpha-feedback** tester
  destination has a composer and report action. The legacy activity preview
  and relay subscription remain readers; signing and publication live in a
  separate sealed writer boundary.
- **Amended 2026-07-30 (rc28 review batch 3).** A clean launch lists **two**
  destinations from the bundled registry (`contentRevision`
  `omega.alpha-feedback.2`): **Alpha feedback · #alpha-feedback** on the
  dedicated NIP-29 group `omega-alpha-feedback`, and **Agent Chat ·
  #agent-chat** on `openagents-public`. Both pin the same canonical WSS relay
  and relay-self public key. The remote Agent Chat manifest may only clear the
  refresh path when every operational field of the `openagents-public` channel
  still matches; `omega-alpha-feedback` is never derived from remote bytes.
  Channel rows in the sidebar use the same `ListItem` styling as recent
  threads (no blue link text); lifecycle words such as "Live" are not drawn
  under the channel name.
- **A clean launch retains the exact bundled destinations.** Omega decodes a
  bundled, versioned registry before it attempts the published-manifest
  refresh. Drift or an unavailable HTTPS endpoint leaves the bundled
  destinations visible.
- **The identity service owns the secret.** The writer provisions and signs
  through `omega_identity::IdentityService`; agent UI receives only the signed
  event. It verifies the event and its exact `h` binding before publication.
  Kind 9 messages and kind 1984 reports are the only authored forms. Reports
  carry exact `h`, `e`, and `p` tags and no copied message content.
- **Retry never changes authorship.** One signed event is retained across the
  bounded publish attempt, including NIP-42 handling. A relay duplicate is an
  ambiguous delivery result, not fabricated success: Omega tells the person to
  check the timeline before retrying.
- **Public means public before typing.** Persistent copy says messages and
  reports are signed, may be retained, and must not contain secrets,
  credentials, private code, customer data, prompts, local paths, or
  unredacted logs. It also states that moderation cannot guarantee erasure.
  The report confirmation discloses the public event and author identifiers
  and does not promise automatic removal.
- **Relay failure has a different path out.** Verified cached rows remain
  visible while the selected channel offers both **Retry relay** and **Open
  support**. The support target is the product's HTTPS GitHub issue path, so it
  does not depend on the failed Nostr relay.
- **Enforced by:** `tester_channels_are_pinned_bounded_and_honest` in
  `crates/omega_deltas`; registry drift, signing, exact-event retry, reports,
  rendered privacy/composer, and outage-fallback tests in `agent_ui`; and the
  deterministic tester-channel visual scenes for the first-launch destination
  and relay-unavailable fallback.
- **Two-account acceptance is deterministic.** The release gate runs
  `script/omega-tester-channel-proof`, which creates two isolated Omega
  identity roots, signs and publishes a kind-9 message through the production
  writer boundary, delivers the exact event through two independent pinned
  relay state machines, signs a kind-1984 report from the second identity,
  admits a signed moderation event, then forces a relay disconnect and proves
  verified history remains available. The rendered outage test separately
  proves that stale state exposes **Retry relay** and **Open support**, not a
  spinner. No owner credential or live public-room write is required.

### OMEGA-DELTA-0183 — The identity backup nudge appears only after the identity has something to lose

- **Upstream Zed:** has no local signing identity and therefore no backup
  problem. There is nothing this diverges *from* except the obvious
  alternative designs, which is why they are named here.
- **The problem this answers.** omega#164 made identity creation silent
  (`OMEGA-DELTA-0040`), so a key nobody was shown silently accrues value —
  channel reputation, device grants, Sarah entitlements — and its loss becomes
  expensive before its existence was ever mentioned. The rejected answers: a
  backup step at first launch is the onboarding ceremony the owner deleted,
  and a modal at any time is a prompt. What ships is a quiet, dismissible
  sidebar row — "Back up your Omega identity key" — that blocks nothing.
- **Armed by value, never by time.** The nudge renders only when a durable
  record says value accrued. Three events write that record, each through the
  one custody seam, each fail-soft so the nudge input can never block the act
  that armed it: a signed public channel write — both the tester-channel
  writer (`omega_public_channel_publish`, `OMEGA-DELTA-0182`) and the legacy
  community control path record it — a freshly minted device pairing grant (the production bridge servers inject
  the recorder; the pairing state machine's constructor keeps `None`, so its
  tests never write into a real profile), and a live Sarah voice session (the
  `Ready` event). A fresh profile has no record, so a first launch can never
  show the nudge; an identity already protected by a recovery artifact has
  nothing to nudge about and is likewise quiet.
- **Dismissal is durable.** One click writes the dismissal beside the identity
  files and the nudge stays gone across restarts. A nudge that reappears every
  launch is a prompt wearing a nudge's clothes. The record is idempotent and
  keeps the first value kind, because "when did this key start mattering" is a
  fact about the first event, not the latest.
- **Quiet by construction.** Every read failure in `backup_nudge_status`
  resolves to "do not nudge". The sidebar polls the durable status on a slow
  cadence instead of holding channels into the three subsystems where value
  accrues, and the poll dies with the panel.
- **Amended 2026-07-30 (rc28 review batch 3 / omega#164 follow-up).** The
  nudge is no longer an enabled no-op. Clicking it opens a minimal honest
  backup surface: the bech32 `nsec` via
  `IdentityService::export_nsec_for_backup` (zeroized on drop; not a generic
  renderer `get_nsec` RPC), a Copy control, one short warning line, and
  Dismiss. Escape closes the surface (standing law 4), as it does the pair-
  phone surface beside it. The durable dismiss control on the row still
  persists and keeps the nudge gone across restarts.
- **Enforced by:** `the_backup_nudge_arms_on_value_and_stays_quiet` in
  `crates/omega_deltas`,
  `backup_nudge_arms_on_first_value_and_stays_dismissed` and
  `export_nsec_for_backup_returns_a_bech32_nsec_for_a_ready_identity` in
  `crates/omega_identity`, and
  `test_backup_nudge_click_opens_a_surface_and_escape_closes_pair_phone` in
  `agent_ui`.

### OMEGA-DELTA-0184 — The composer executor dropdown is the new-conversation front door

- **Supersedes the interstitial three-row screen from OMEGA-DELTA-0177.** The
  owner hit the full-screen "Start a new conversation" chooser in a
  `release-fast` build and called it horrible friction (omega#165): anything
  between `+` and a blinking cursor fails the product. The typed boundary
  survives; the screen does not.
- **New Thread opens a thread, not a screen.** `agent::NewThread`, the
  Thread menu, and the toolbar `+`'s Omega entry land directly in a normal
  conversation with the composer focused, on the default executor
  (**Omega Agent**). Startup's empty-window landing takes the same path, and
  the `OMEGA-DELTA-0177` startup recheck still refuses to cover restored
  state with a new blank composer.
- **The selection surface is a composer-bar dropdown.** It sits beside the
  Flash/Pro tier control (`OMEGA-DELTA-0172`) at the same visual weight, in
  both the loading composer and the zero-base bar. Its fixed order is Omega
  Agent, the named direct agents (Codex `codex-acp`, Claude Code
  `claude-acp`, Grok Build `grok-build`), every other installed ACP agent,
  Sarah (voice), then **Add More Agents…** into the ACP registry.
  `agent::ToggleComposerExecutorMenu` opens it from the keyboard.
- **Readiness stays typed and honest.** Every row carries `ModeReadiness` —
  Ready, Setup required, Temporarily unavailable, or Not supported in this
  build — and a row that cannot run here renders disabled with its reason,
  never hidden and never fake-enabled. Ready is still minted only through a
  `PreparationReceipt` bound to a created session or a connected router, and
  a Ready claim still travels through the receipt-validated activation from
  `OMEGA-DELTA-0177`.
- **One selection authority.** The dropdown's face reads the active
  conversation's own owner — the loading composer's agent key or the thread
  view's construction-time identity — never a second selection store, so the
  `OMEGA-DELTA-0131` two-selections lie cannot be restated. Choosing a row
  goes through one panel path, `compose_on_executor`, which replaces a blank
  draft or starts a new conversation.
- **The ownership law survives unchanged.** Selection is free until the first
  send; the first send binds the conversation (`OMEGA-DELTA-0178`); choosing
  a different executor over a bound transcript starts a new thread and never
  retargets the transcript underneath its entries (`OMEGA-DELTA-0150`).
  Selecting Sarah routes through `OMEGA-DELTA-0180`'s admission surface.
- **Enforced by:**
  `the_composer_executor_dropdown_is_the_new_conversation_front_door` in
  `crates/omega_deltas`, the rewritten `OMEGA-DELTA-0177` claim check, the
  Agent Panel GPUI tests for Command-N and for the folder-selection handoff.
  The projectless-composer baselines predate the working-folder gate and no
  longer prove the current landing policy.

### OMEGA-DELTA-0185 — The sealed baselines and the installed release gate are release evidence

- **Every scene that photographs the shipped sealed surface asserts the seal
  per scene.** The front-door pair, the Sarah admission pair, and the
  tester-channel pair each carry an explicit `omega_zero_base::is_sealed()`
  ensure at their own capture site in
  `crates/omega/src/visual_test_runner.rs`. Ordering after the seal call is not
  a guarantee: the Sarah pair used to rely on running after the front-door
  seal, which is exactly the arrangement that silently photographs an
  unsealed window when the call order changes. The six committed baselines
  under `crates/omega/test_fixtures/visual_tests/` must exist non-empty.
- **The Exo-lane `omega_zero_base_wide` / `omega_zero_base_narrow` baselines
  photograph the unsealed harness form** (the surface under a zoomed panel,
  `seal()` never called on that runner path — since omega#161 an arrangement
  only proof harnesses can produce) and require a live `exo acp` runtime to
  re-record. They are not evidence for the sealed surface; the six sealed
  scenes are.
- **The installed-candidate release gate is scripted, not heroic.**
  `script/omega-release-gate` runs the Episode 263 gap-analysis section 7
  matrix against the packaged candidate from `script/bundle-omega-rc`,
  staged from its DMG and launched from a clean `--user-data-dir` profile.
  Every row emits a preserved evidence record bound to the candidate's
  package digest. A row is `automated-pass` / `automated-fail` only when the
  harness observed it end-to-end; a row needing a human account, interactive
  provider auth, a second person, or a judgment call is
  `owner-assisted-pending` with the exact instruction; a host refusal is
  `blocked` with the reason. No row is ever fabricated.
- **The zero-refusal sweep is bound to the refusal sentence.** The harness
  scans the clean-profile log for the sentence
  `omega_zero_base::refusal` produces — since omega#161: "… is not part of
  Omega, which shows one agent thread and the controls that operate it." —
  and the flag-free journey must log zero of them. The delta test holds the
  harness fragments and the `crates/omega_zero_base` source in agreement so
  the sweep cannot rot into scanning for a sentence the product no longer
  says.
- **Why:** omega#158 — the alpha cut needs the sealed render photographed and
  the release gate runnable on every packaged candidate; the manual checklist
  does not survive contact with a real release cadence, and GitHub-hosted CI
  is currently locked (billing), so the local proof runner and this gate are
  the only render-evidence authorities.
- **Enforced by:**
  `the_sealed_baselines_and_the_installed_release_gate_hold` in
  `crates/omega_deltas`, the per-scene seal ensures in the visual test
  runner, and the generated gate report at `docs/omega/release-gate.md`.

### OMEGA-DELTA-0186 — The removed editor crates stay removed

- **Upstream Zed:** ships the full editor around every surface: debugger,
  task modals, repl, panels, pickers, previews, and the rest of the
  full-editor crate set.
- **Omega:** omega#162 deleted the full-editor-only crate set from the build
  graph, per the single-experience plan (sections 5–6) — the product is one
  agent thread and the controls that operate it, and `omega#161` already made
  that the only surface. The final batch removed the remaining selector,
  preview, onboarding, extension, feedback, journal, snippets, profiling,
  keymap-editor, and which-key crates. Each deleted crate is recorded in
  `REMOVED_EDITOR_CRATES`; its crate directory, workspace-member entry, and
  workspace-dependency entry must all stay gone. Namespaces whose declaring
  crate died move to `FORBIDDEN_KEYMAP_NAMESPACES` in the same commit that
  deletes the crate, because the built-in keymap is unwrapped at startup and a
  binding naming a deleted action is a process-killing panic that the build
  cannot catch (`0.2.0-rc6` shipped exactly that failure).
- **Measured result:** 46 editor-only crates are absent. The `omega` internal
  dependency closure is 197 crates, down from the issue's measured 245 and
  within its approximate `~191` target. The source inventory contains 1,646
  Rust files after deletion, so the brand gate's 1,500-file anti-vacuity floor
  remains measured and strict rather than being lowered speculatively.
- **Keep set:** `vim` (owner decision 2026-07-29), `editor`, `workspace`,
  `project`, `project_panel`, `git_ui`, `search`, `terminal`/`terminal_view`,
  `title_bar`, `command_palette`, `settings_ui`, `onboarding`, `markdown`,
  `buffer_search`, `notifications`, plus `file_finder`, `go_to_line`,
  `language_tools`, and `acp_tools`, which the owner explicitly kept in the
  preceding omega#162 batch. `activity_indicator` and `lsp_locations` remain
  because they provide the paired LSP/activity visibility named by the
  keep-if catalog.
- **Why:** the owner's single-experience direction: the editor around the
  thread is surface Omega does not sell, roughly 18% of the build graph, and
  every hidden-but-compiled surface is one key press or one rebase away from
  returning.
- **Also amends:** `OMEGA-DELTA-0007` (debugger deleted, not just unprompted),
  `OMEGA-DELTA-0048` (a namespace leaves `ZERO_BASE_HIDDEN_KEYMAP_NAMESPACES`
  in the commit that deletes its crate and forbids its bindings), and
  `keymaps_name_no_deleted_action` now scans every keymap asset, base keymaps
  included, not only the three defaults.
- **Enforced by:** `removed_editor_crates_stay_removed` and
  `keymaps_name_no_deleted_action` in `crates/omega_deltas`.

### OMEGA-DELTA-0187 — Drawn implies working: the control-crawl gate

- **Upstream Zed:** no product-wide gate that every visible control produces
  an observable consequence; menu rows can render armed while carrying no
  action; multi-sentence tooltips and status essays ship freely.
- **Omega:** owner review item 17 and the standing product laws in the review
  ledger. A hermetic control-crawl harness enumerates interactive controls per
  registered scene, activates each with pointer **and** keyboard, and **fails**
  on zero observable consequence unless a registered exemption names a reason.
  Menu entries are activated individually (the display-only
  `ContextMenuEntry.action` trap). Escape dismissal is asserted for every
  modal the crawl opens. A checked-in crawl registry
  (`docs/omega/control-crawl-registry.json`) is the inventory: a new surface
  without a same-commit registration fails the delta check. Multi-sentence
  tooltips/status strings fail a copy lint unless listed in
  `docs/omega/control-crawl-copy-allowlist.json`. Process cadence, ownership,
  severity, and the same-commit registration law live in
  `docs/src/qa-process.md`.
- **Coverage:** the synthetic proving scene in `crates/omega_control_crawl` is
  `complete`. `OMEGA-DELTA-0191` completes the source-backed activity-rail,
  new-thread-menu, thread-header-menu, and application-menu inventories.
  Sealed front-door, Sarah, tester-channel, settings, pair-phone, and
  composer-executor menu remain registered as `pending-expansion` so the
  delta check knows they exist; expanding each to a full semantic-tree crawl
  is follow-up work, not a silent omission.
- **Mutation proof:** `deliberate_noop_control_fails_the_crawl` injects an
  inert control and asserts the crawl fails. That test must never be inverted
  or deleted to green a broken gate.
- **Release evidence:** `script/omega-release-gate` runs
  `cargo test -p omega_control_crawl` as the automated `control-crawl` row.
  Full installed visual crawl of every GPUI scene remains expansion work; the
  automated row already refuses a broken protocol.
- **Why:** enabled-looking no-ops (backup-key notice, display-only menu
  entries, Escape-deaf modals) keep shipping past unit tests that never
  activate the control. The crawl makes "drawn implies working" a machine
  check rather than a hope.
- **Enforced by:** `the_control_crawl_gate_holds` in `crates/omega_deltas`,
  the tests in `crates/omega_control_crawl`, and the `control-crawl` row of
  `script/omega-release-gate`.

### OMEGA-DELTA-0188 — The agent-thread Outline sidebar is deleted

- **Upstream Zed:** has no agent-thread outline pane; the surface was an
  Omega addition (omega#135), so this delta records the removal of an Omega
  divergence rather than a divergence from upstream.
- **Omega:** the owner reviewed the rc28 candidate and directed 2026-07-30:
  "I don't want that at all, delete it from the codebase entirely." Deleted in
  the same change: `crates/agent_ui/src/thread_outline.rs` and every
  `AgentPanel` binding/navigation/artifact-activation hook to it, the
  `ThreadOutline` keymap context and all `omega_thread_outline::*` bindings in
  the three default keymaps, the `omega_thread_outline` action namespace, the
  workbench-state outline projection lane (`ThreadOutlineProjection`, its
  seven `ProjectionTransition` variants, and both outline errors — the lane
  had no production driver outside the pane), the conformance
  `ThreadOutlineState`, the harness outline fixture family, and the 14
  `omega_workbench_outline_*` visual scenes (none of which had committed
  baselines). This is not the buffer outline (`outline::`, `outline_panel::`),
  which stays.
- **Why:** owner direction from the live rc28 review (omega#160). The pane
  competed with the transcript for attention and the owner wants one reading
  surface, not a parallel index.
- **Also amends:** `OMEGA-DELTA-0174` (the `try_activate_outline_target`
  reveal site no longer exists and left its pinned reveal list).
- **Enforced by:** `removed_surfaces_stay_removed` (the deleted file) and
  `keymaps_name_no_deleted_action` (the forbidden
  `omega_thread_outline::` namespace) in `crates/omega_deltas`.

### OMEGA-DELTA-0189 — No exposition in the UI; statuses are icons; Escape closes modals

- **Origin:** owner review of rc28 (omega#160, 2026-07-30). Standing laws 2–4
  from the owner review ledger in `docs/omega/release-gate.md`.
- **No exposition anywhere.** The product never renders multi-sentence
  explanations of internal mechanics in tooltips, status lines, or empty
  states. Controls are labeled, not narrated. One-word tooltips are the
  maximum copy. Specifically removed:
  - The composer executor dropdown tooltip essay
    (`"The executor is free to change until the first message is sent."` /
    `"This conversation will run on …"`) — delete, no replacement.
  - Composer ready/status sentences such as
    `"Omega router ready · route selected when sent"` and
    `"Choosing an executor and creating its session…"`.
  - The routing-mode dropdown
    (`"Run this new conversation on"` → Automatic/Omega and its
    `NewConversationRouteOverride` state). Routing stays automatic
    (`OMEGA-DELTA-0179`); only the selector and its state die. Exact executor
    choice remains the composer executor dropdown (`OMEGA-DELTA-0184`).
  - The sidebar annotation `"Owner unverified — legacy thread"`. Legacy owner
    ambiguity stays internal (omega#152 versioned owner metadata); click still
    refuses so a session is not guessed.
- **Statuses are colors/icons, never words.** Sidebar and header lifecycle
  badges are a colored dot only. A one-word tooltip (`Running` / `Waiting` /
  `Failed` / `Completed` / `Cancelled`) is the maximum copy. Amends the
  presentation half of `OMEGA-DELTA-0181`.
- **Escape closes every modal/auxiliary window.** Settings
  (`crates/settings_ui`) handles `workspace::CloseWindow` (the keymap binding
  for Escape in the `SettingsWindow` context) by removing its own window. The
  embedded Settings route (the Omega shell's Settings page) re-dispatches
  `CloseEmbeddedSettings` instead — the same action its Back control sends —
  so Escape leaves the route without closing the shell's window.
- **Why:** the owner hit the Automatic/Omega routing control and the
  "route selected when sent" status and rejected both as unclear exposition.
  Status words next to every thread row are noise; colors carry the same
  information. A settings window that ignores Escape fails the modal law.
- **Enforced by:** `ui_carries_no_exposition_statuses_are_icons_and_escape_closes_settings`
  in `crates/omega_deltas`; unit tests on the composer menu constants, the
  threads sidebar (legacy note absent), and
  `settings_window_closes_on_close_window_action` in `crates/settings_ui`.


### OMEGA-DELTA-0190 — ZEDREMOVE: Omega paths, env precedence, and crate rename (omega#174)

- **Upstream Zed:** ships as the `zed` package under `crates/zed`, project-local
  settings in `.zed/`, env vars named `ZED_*`, and issue templates that say
  "Zed".
- **Omega:** visible product identity is Omega end-to-end.
  1. **Project-local settings path.** Canonical folder is `.omega/`
     (`paths::local_settings_folder_name`). Legacy `.zed/` remains readable
     (`legacy_local_settings_folder_name` and the dual path matchers in
     `project_settings` / worktree scan / agent tool permissions). Writes and
     UI surfaces use `.omega`. Omega **never deletes** a legacy `.zed` tree
     silently; prefer new when both exist, honor legacy when only legacy
     exists (copy-on-write style: the next save lands in `.omega`).
  2. **Env vars.** `OMEGA_*` takes precedence over inherited `ZED_*` for the
     dual-read helpers in `client` and `zed_env_vars`. Bundle and
     `script/omega-local` set both during the transition; `script/zed-local`
     remains a thin compatibility wrapper.
  3. **Issue templates.** Bug and crash forms ask for Omega version, Omega
     commands, and Omega logs — not "Zed version".
  4. **Crate rename.** Application crate `crates/zed` → `crates/omega`
     (package name `omega`); `crates/zed_actions` → `crates/omega_actions`
     (package name `omega_actions`). Action **namespaces** were already
     `omega::` with `zed::` deprecated aliases; keymaps keep resolving.
     Scripts, brand-gate fragment paths, and `omega_deltas` source-literal
     checks name `crates/omega/...`.
- **Why:** owner-directed ZEDREMOVE from the rc28 review (omega#160 items 16a/16b,
  issue omega#174). A settings path, package name, or template that still says
  Zed presents the wrong product.
- **Enforced by:** `paths` unit tests for the dual folder names;
  `omega_deltas` path literals and workspace membership for the renamed crates;
  brand-gate inventory that scans `crates/omega` rather than the deleted path;
  issue-template content reviewed in this change.
### OMEGA-DELTA-0191 — Refusals fail proofs and drawn controls have loaded actions

- **Upstream Zed:** action refusal is an ordinary dispatch outcome, and no
  product-wide proof binds rendered controls to the handlers loaded by the
  shipped startup path.
- **Omega:** the action gate remains installed only as a tripwire. Every
  product refusal advances a process counter. The visual-proof runner and
  `--omega-send` smoke driver fail when that counter advances, even if the
  underlying scene or turn otherwise succeeds. The seeded counter test proves
  one refusal is rejected. The visual runner also drops the stale legacy
  multi-workspace-sidebar scenes whose deleted crate prevented the proof
  executable from compiling.
- **Drawn implies working:** the activity rail, Agent Panel `+` menu, thread
  header menu, and application menu each have an exact source-backed control
  inventory. Every control on the one application surface must dispatch an
  admitted action, and the check pins the handler or dependency initialization
  that the shipped path loads. Adding a row without extending its inventory,
  admitting its action, and loading its handler fails the delta suite.
- **One product shape:** “Zero Base” remains only in legacy crate and check
  names. The taxonomy and crate documentation state that the flag-free surface
  is Omega itself, with no second application mode or escape hatch.
- **Enforced by:** `proof_processes_fail_on_any_logged_refusal`,
  `drawn_activity_rail_controls_are_admitted_and_loaded`,
  `drawn_new_thread_menu_controls_are_admitted_and_loaded`,
  `drawn_header_menu_controls_are_admitted_and_loaded`,
  `drawn_application_menu_controls_are_admitted_and_loaded`, and
  `zero_base_is_only_a_legacy_implementation_name` in
  `crates/omega_deltas`, plus
  `a_seeded_refusal_trips_the_proof_counter` in `crates/omega_zero_base`.

### OMEGA-DELTA-0192 — Background identities are candidates until explicit activation

- **Origin:** OMEGA-AUTH-01, omega#176. Amends the account-admission side of
  `OMEGA-DELTA-0040` without restoring a blocking first-run ceremony.
- **Silent startup stays.** One shared process startup task still creates a
  local Nostr identity from `Absent`, adopts the exact file-backed identity
  from `Unadopted`, or reuses `Ready`, then opens the front door. Named custody
  refusals are logged and remain repairable; they never park startup.
- **Custody-ready is not account-active.** Omega atomically stores
  `identity/identity.account.json`. A freshly generated key is
  `CandidateLocal`; every ready pre-AUTH-01 identity without this record
  migrates to `CandidateExisting` with the same public key. The account
  control shows the state and short public fingerprint. Lost, conflict,
  incomplete, locked, reset-failed, and relaunch-required remain distinct.
- **Durable actions activate explicitly.** Public posts, community joins,
  device grants, hosted-account links, and agent attestations pass a typed
  identity-action gate. A candidate atomically moves to `Activating` while one
  `identity/identity.action-intent.json` intent binds account, generation,
  identity, kind, destination, authorization, payload digest, and expiry.
  Completion never implies replay: the caller consumes the exact intent once
  after every binding is revalidated. Cancellation restores the candidate,
  removes the held intent, and resumes nothing. A second window or process
  cannot install a competing intent under the channel identity mutation lock.
- **File custody only.** The root Nostr secret remains the raw 32-byte
  `identity/identity.secret` file, atomically replaced and owner-only on Unix.
  This delta enables no macOS Keychain, Secure Enclave, Windows credential
  vault, Linux secret service, Android keystore, or native key-vault path.
  `KeyringLocator` remains a serialized compatibility name only.
- **Enforced by:** activation migration, cancellation, exact-once consumption,
  stale-binding, concurrent-intent, reset-cleanup, and active-signing tests in
  `omega_identity`; candidate/active and repair-priority presentation tests in
  `onboarding`; typed gate tests at the public-channel, community, pairing,
  hosted-link, and agent-attestation entry points; and the
  `OMEGA-DELTA-0192` source assertions in `crates/omega_deltas`.

### OMEGA-DELTA-0193 — Activation makes recovery and exact resumption explicit

- **Origin:** OMEGA-AUTH-02, omega#177. Extends the candidate and held-action
  model from `OMEGA-DELTA-0192`; it does not add another secret store.
- **The local path is real.** The desktop account control explains what the
  public key and signing secret mean, and that a signature does not itself
  prove identity, truth, membership, or permission. **Keep this identity**
  requires a verified encrypted NIP-49 recovery artifact before a candidate
  becomes Active. A verified artifact for that exact identity is reused rather
  than forcing another export. The password protects the file and is neither
  persisted nor presented as an account-reset password. Raw `nsec` import is
  an advanced recovery escape hatch using the zeroizing secure input.
- **Deferred paths are honest.** **Use an existing identity** does not
  overwrite a healthy local candidate; safe replacement waits for
  multi-account switching. **Use a signer on another device** originally
  refused activation without moving a secret; OMEGA-DELTA-0195 now routes it
  into bounded NIP-46 enrollment.
- **Completion is not replay.** The durable action file contains only typed
  references, a payload digest, and expiry. The initiating surface owns the
  actual payload through a process-local one-shot callback. After recovery and
  activation, only that exact owner may revalidate, consume, and resume the
  held intent. Exact cancellation restores the candidate and tells the owner
  to resume nothing. After restart, an orphaned durable intent is explained
  and may be cancelled, but Omega will not guess or recreate its payload.
  Proactive `AccountSetup`, which has no external payload, is the only
  ownerless intent the identity screen consumes itself.
- **File custody only.** The root secret remains the owner-only
  `identity/identity.secret` file. NIP-49 output goes to the directory the
  person selects; only public-safe recovery verification metadata is retained.
  This delta enables no macOS Keychain, Secure Enclave, Windows credential
  vault, Linux secret service, Android keystore, encrypted application vault,
  or native enclave path.
- **Enforced by:** recovery-gated activation and exact-intent tests in
  `omega_identity`; one-shot owner tests in `omega_actions`; candidate,
  activation, and cancellation presentation tests in `onboarding`; and the
  `OMEGA-DELTA-0193` source assertions in `crates/omega_deltas`.

### OMEGA-DELTA-0194 — Account selection is durable, partitioned, and explicit

- **Origin:** OMEGA-AUTH-03, omega#178. Extends the candidate and recovery
  model from `OMEGA-DELTA-0192` and `OMEGA-DELTA-0193` to multiple local
  identities.
- **One account home.** **Omega Identity** in the title bar opens a responsive
  account dashboard showing each public fingerprint, optional local profile,
  signer kind and availability, recovery state, and last successful signer
  use. Add local identity, complete setup, switch, lock or unlock, sign out,
  forget this device, and retire identity are distinct controls. A new local
  identity is a file-backed `CandidateLocal` with recovery needed; **Complete
  setup** returns to the existing NIP-49 activation ceremony. Retirement stays
  unavailable until its signed policy is implemented and is never presented as
  local deletion. Remote signers are added by OMEGA-DELTA-0195.
- **Selection is generation-fenced.** The durable registry is
  `identity/accounts/index.json`. It preserves the legacy root account and
  stores added accounts below deterministic per-account directories under
  `identity/accounts/`. Switching uses a crash-resumable transaction and
  monotonically advances the active generation. Signing validates the selected
  account reference, public identity, lifecycle, and generation immediately
  before use; stale selection tokens are refused.
- **Local lifecycle has exact effects.** Lock makes the active local signer
  unavailable until explicit unlock. Sign out clears the active selection
  without deleting custody. Forget this device starts a durable purge journal
  and never claims to retract relay or peer events or delete an external NIP-49
  file. Draft and room-state owners verify their own per-public-key deletion.
  Targets without an owning purge hook remain named partial failures and can be
  retried instead of being rounded up to success.
- **File custody only.** Every local account secret remains an `identity.secret`
  file: the migrated account may remain at `identity/identity.secret`, while
  added accounts use their per-account directory. This delta enables no macOS
  Keychain, Secure Enclave, Windows credential vault, Linux secret service,
  Android keystore, encrypted application vault, or native enclave path.
- **Enforced by:** registry migration, add, switching, generation, lifecycle,
  and purge tests in `omega_identity`; per-public-key draft and room-state
  tests in `agent_ui`; dashboard action and purge-result tests in `account_ui`;
  and the `OMEGA-DELTA-0194` source assertions in `crates/omega_deltas`.

### OMEGA-DELTA-0195 — Remote signers use bounded, file-backed NIP-46 capabilities

- **Origin:** OMEGA-AUTH-04, omega#179. Extends the account registry from
  OMEGA-DELTA-0194 without importing the person's root `nsec`.
- **Two explicit pairing directions.** Omega accepts a bounded `bunker://`
  connection or creates a transient `nostrconnect://` link backed by a
  disposable client key and exact rendezvous relay set. The generated link can
  be opened or copied while the verified listener is running; it does not
  enter logs, action payloads, errors, or public account records.
- **Consent precedes authority.** The desktop dashboard shows the expected
  signer when known, methods, exact event kinds, exact relays, expiry, and
  remote recovery dependency before the first approval. Its initial profile
  grants login proof and bounded event signing only. Encryption and bulk
  decrypt remain separate profiles. A verified acknowledgement leads to a
  second approval showing both the reported person account and signer-device
  key. Account activation additionally requires an exact signed login
  challenge under the reported account key.
- **Responses are fenced.** Pairing and runtime requests bind the selected
  account generation, capability, correlation id, author, relay, event kind,
  tags, signature, and content. Explicit rejection and revocation are terminal;
  offline, silence, and timeout remain visible outcomes. Sign out clears
  selection, while **Disconnect signer** separately revokes the capability and
  verifies disposable-key deletion.
- **File custody only.** Remote state lives below
  `identity/nip46/<capability-ref>/`. Public-safe `pairing.json` and
  `capability.json` sit beside atomic owner-only `client.secret` and
  short-lived `pairing.secret` files on Unix. The root `nsec` remains in the
  external signer. This delta enables no macOS Keychain, Secure Enclave,
  Windows credential vault, Linux secret service, Android keystore, encrypted
  application vault, or native enclave path.
- **Enforced by:** NIP-46 parsing, state-machine, correlation, signature,
  generation, permission, revocation, and file-mode tests in `omega_identity`;
  relay coordinator and remote signing tests in `omega_signer_broker`; account
  enrollment and lifecycle presentation tests in `account_ui`; and the
  `OMEGA-DELTA-0195` source assertions in `crates/omega_deltas`.

### OMEGA-DELTA-0196 — Relay and hosted authentication remain independent

- **Origin:** OMEGA-AUTH-05, omega#180. Makes the existing NIP-42 and NIP-98
  paths observable without promoting network or service evidence into general
  account authority.
- **Relay receipts are account- and connection-bound.** Every account public
  key and normalized relay URL has a public-safe receipt tied to a monotonic
  connection generation. Challenge
  pending, authenticated, refused, disconnected, and stale remain distinct.
  Exact relay, challenge, selected account, timestamp, signature,
  acknowledgement, connection generation, and one-use proof checks fail
  closed. Public state retains only a digest-derived challenge reference,
  accepted authentication event id, bounded refusal category, and observation
  time, never the raw challenge.
- **Hosted links have a complete lifecycle.** The exact NIP-98 request binds
  HTTPS URL, method, payload digest, signer, freshness, and a single-use proof.
  Verification, expiry, rotation, disconnect, revocation, owner-scope refusal,
  service unavailability, credential-storage failure, and revocation failure
  are independently visible. The public Omega-key to OpenAgents-user binding
  contains no bearer and cannot authenticate a relay, join a group, or
  authorize an arbitrary action.
- **The account home names authority precisely.** The dashboard keeps
  **Signer ready**, **Relay authenticated**, **Group admitted**, **Hosted
  linked**, and **Action authorized** as separate rows. Per-relay receipts and
  hosted-session controls do not imply one another.
- **File custody only.** Hosted access and refresh tokens remain in the
  unencrypted `credentials/credentials.json` file, written atomically with
  owner-only `0700` directory and `0600` file modes on Unix. This delta enables
  no macOS Keychain, Secure Enclave, Windows credential vault, Linux secret
  service, Android keystore, encrypted application vault, native enclave, or
  hardware-backed credential store.
- **Enforced by:** exact NIP-42 challenge, generation, acknowledgement, replay,
  and receipt tests; exact NIP-98 request and one-use proof tests; hosted
  verification, expiry, rotation, revocation, failure, and public-projection
  tests in `omega_effectd`; dashboard authority-language and lifecycle tests
  in `account_ui`; and the `OMEGA-DELTA-0196` source assertions in
  `crates/omega_deltas`.

### OMEGA-DELTA-0197 — Profile hydration is bounded, partitioned, and optional

- **Origin:** OMEGA-AUTH-06, omega#181. Recovers portable account state without
  turning startup or account switching into an unbounded network gate.
- **Kind 0 remains optional.** The account dashboard separates **Skip**,
  **Save locally**, and **Publish profile**. Skip signs and publishes nothing;
  local save writes only the selected account's draft; publish routes one exact
  kind `0` event through the selected signer and revalidates the public key,
  generation, event, and acknowledgement.
- **Hydration has explicit bounds.** Imported, recovered, switched, and
  remote-signer accounts hydrate profile, relay preferences, NIP-29 group list,
  membership and room metadata, bounded recent room pages, linked hosted
  state, and only enabled adapter state. Every source and the overall gate have
  deadlines. Complete, partial, offline, failed, and skipped-fresh outcomes
  remain distinct, as do fresh, cached, locked, disabled, timeout, stale,
  offline, and failed source results. Cache/default fallback opens the desktop
  while retryable recovery continues in the background under the same
  account-generation fence.
- **External decrypt consent is separate.** Bulk decrypt is unknown, allowed,
  or declined independently from login and ordinary signing. Decline is
  durable, leaves content locked, and suppresses prompt storms. A missing
  signer method requires explicit signer reconnection. Persistent plaintext
  cache policy is disclosed and controlled separately per account.
- **File custody only.** Profile drafts, hydration receipts, consent,
  ciphertext, plaintext, and signer cache metadata remain ordinary unencrypted
  account-partitioned files with atomic owner-only `0700` directory and `0600`
  file modes on Unix. This delta enables no macOS Keychain, Secure Enclave,
  Windows credential vault, Linux secret service, Android keystore, encrypted
  application vault, native enclave, or hardware-backed credential store.
- **Enforced by:** bounded-plan, fresh-skip, per-source deadline, generation,
  cache fallback, background retry, consent persistence, capability, plaintext
  policy, and purge tests in `omega_identity_sync`; kind `0` exact-signing and
  acknowledgement tests in the profile publisher; dashboard state and action
  tests in `account_ui`; and the `OMEGA-DELTA-0197` source assertions in
  `crates/omega_deltas`.

### OMEGA-DELTA-0198 — Community entry preserves protocol and authority boundaries

- **Origin:** OMEGA-AUTH-07, omega#182. Lets one selected Nostr account enter
  supported communities without translating their authorities into one
  misleading connected state.
- **Profiles remain explicit.** Standards-first NIP-29, NIP-29 relay lists, the
  pinned Buzz profile, admitted Armada Concord v1/v2 profiles, and
  Omega/OpenAgents service invites have typed previews. Every preview names its
  protocol, authority, room, visibility, requested signing operations, terms,
  recovery model, and client portability. Unsupported profile data remains
  opaque and unjoinable.
- **Joining is a durable sequence.** A public-safe transaction is written
  before network mutation. Relay addition, NIP-42 authentication, invite claim,
  NIP-29 join, and OpenAgents grant are independent results. Partial completion
  remains visible and resumable without duplicate claims. No relay or protocol
  result grants OpenAgents membership, command, moderation, payment, or release
  authority by inference.
- **Secret-bearing input stays private.** Raw invite codes, capability query
  values, and URL fragments never enter previews, errors, public transaction
  projections, logs, telemetry, or public records. A restart-safe NIP-29, Buzz,
  or Concord claim may retain exact bounded step-request bytes in an
  account-bound private transaction payload until completion, revocation,
  expiry, or cancellation, when deletion is verified.
- **File custody only.** Public join evidence and any private payload live in
  ordinary unencrypted files below
  `identity/invites/accounts/<public-key>/`, written atomically with Unix
  directory mode `0700` and file mode `0600`. This delta enables no macOS
  Keychain, Secure Enclave, Windows credential vault, Linux secret service,
  Android keystore, encrypted application vault, native enclave, or
  hardware-backed credential store.
- **Enforced by:** profile parsing, preview, redaction, stale, banned, terms,
  unsupported, partial-result, restart, and duplicate-claim tests in
  `omega_invites`; public controller projection tests in `agent_ui`; desktop
  authority and resume affordance assertions in `account_ui`; and the
  `OMEGA-DELTA-0198` source assertions in `crates/omega_deltas`.

### OMEGA-DELTA-0199 — Device enrollment grants a revocable device key, not the root key

- **Origin:** OMEGA-AUTH-08, omega#183. Adapts an ephemeral SAS exchange while
  keeping the person's root Nostr secret outside every device pairing payload.
- **Pairing is transcript-bound and two-screen.** The preview names the exact
  endpoint, approved target platform and capabilities, owner gesture
  reference, and expiry and discloses that no root key crosses the channel.
  The bridge formats the versioned core invitation as an
  `omega://device-enrollment/v1` deep link. Both peers bind their ephemeral
  keys, account generation, endpoint, approval, expiry, and one-use secret into
  the transcript, show the same short authentication string, and confirm it
  independently. Pending, expired, refused, redeemed, replayed,
  peer-substituted, and wrong-generation outcomes remain distinct.
- **Grants remain narrow and independently revocable.** A target creates its
  own permanent device key. Its grant names that key, exact capabilities,
  account generation, creation time, expiry, and last successful use. Revoking
  one device neither rotates the person identity nor revokes sibling devices;
  partial or failed revocation remains visible and retryable.
- **Platform claims remain exact.** Web exposes detected NIP-07 or NIP-46 only
  and stores no root key by default. Android exposes NIP-55 only after an
  admitted host implements it. iOS exposes NIP-46 in this wave and does not
  claim NIP-55 parity or an unaudited native bridge.
- **File custody only.** Pairing and grant records remain ordinary unencrypted,
  account-partitioned files below `identity/device-enrollment/`, with atomic
  owner-only Unix directory mode `0700` and file mode `0600`. A joining target
  writes its pending transcript and device key before returning a response,
  resumes the same record after restart, and replaces it with the permanent
  device credential only after receiving the redeemed grant. Public
  projections contain no introduction secret, ephemeral private key, device
  private key, root `nsec`, or bearer. This delta enables no macOS Keychain,
  Secure Enclave, Windows credential vault, Linux secret service, Android
  keystore, encrypted application vault, native enclave, or hardware-backed
  credential store.
- **Enforced by:** exchange expiry, SAS mismatch, replay, substitution,
  generation, redemption, grant scope, platform capability, last-use,
  independent revocation, verified-delete, redaction, and permission tests in
  the device enrollment core; preview, inventory, and lifecycle assertions in
  `account_ui`; and the `OMEGA-DELTA-0199` source assertions in
  `crates/omega_deltas`.

### OMEGA-DELTA-0200 — Agents are separate principals with bounded grants and evidence-bound assurance

- **Origin:** OMEGA-AUTH-09, omega#184. Extends person and device identity
  without lending either key or authority to an agent.
- **Identity stays separate.** Every admitted agent has its own Nostr key and
  public projection. Its owner attestation and grant name the owner account and
  person public key, agent reference and public key, exact methods, event kinds,
  room or tenant resources, generation, issue and expiry times, attestation
  reference, revocation, and last successful use. The agent cannot sign as the
  person by fallback and receives no raw person-key material.
- **Requests stay bounded.** Authorization rechecks owner, agent, grant,
  generation, method, kind, resource, request id, subsystem, purpose,
  destination, origin, content digest, capability, gesture, and expiry.
  NIP-AA agent relay authentication is available only under its exact admitted
  profile. NIP-AA, ordinary NIP-42 relay auth, NIP-29 membership, hosted
  linking, device enrollment, and exact owner action authorization cannot
  substitute for one another.
- **Installed claims stay honest.** A machine-readable matrix inventories
  migration, storage, recovery, NIP-42, NIP-46, account switching, logout,
  invites, hydration, pairing, and authority separation on desktop, web,
  Android, and iOS. Source-automated, installed-automated,
  owner-assisted-pending, blocked, and not-admitted are distinct. No host is
  promoted by a fixture or source test. The existing installed canary collector
  must run with a fresh journey canary against logs, telemetry, diagnostics,
  crashes, clipboard, and accessibility; an unreadable required surface blocks.
- **File custody only.** Agent records live below
  `identity/agents/records/<account-ref-sha256>/<agent-pubkey>.json`, with
  incomplete attestations below
  `identity/agents/pending/<request-ref-sha256>.json`, in ordinary unencrypted
  files
  written atomically with Unix directory mode `0700` and file mode `0600`.
  This delta enables no macOS Keychain, Secure Enclave, Windows credential
  vault, Linux secret service, Android keystore, encrypted application vault,
  native enclave, hardware-backed credential store, or other native key
  custody.
- **Enforced by:** bounded agent identity, attestation, authorization,
  revocation, redaction, storage, and non-substitution tests in
  `omega_agent_identity`; public principal and grant assertions in
  `account_ui`; the installed tripwire collector; the assurance document and
  host matrix; and the `OMEGA-DELTA-0200` source assertions in
  `crates/omega_deltas`.

### OMEGA-DELTA-0201 — Server-routed Omega Agent default

- **Origin:** owner directive 2026-07-30, after every Omega Agent turn on a
  fresh dev build died with *"Permission error with Google AI's API: Omega
  could not sign the hosted sign-in proof (the active account selection
  changed). Set GEMINI_API_KEY…"*. Verbatim intensity: *"NEVER LET ME SEE THIS
  BULLSHIT AGAIN, ALWAYS WORK — CORE OMEGA AGENT MUST ALWAYS HIT OUR API AND
  WORK."*
- **Amended 2026-08-08:** the core agent defaults to the logical
  `openagents/omega-agent` id and sends OpenAI Responses requests to the
  OpenAgents API. The client does not name the provider model, choose reasoning
  effort, or select fallback rungs.
- **One routing authority:** provider selection, routing, and fallback happen
  behind the OpenAgents API. If an Omega Agent request fails, the IDE reports
  that failure and never moves the turn onto a direct Google, OpenAI, or other
  client provider. This keeps account policy, observability, metadata, and
  fallback behavior on the server and lets them change without an IDE release.
- **Environment selection:** production uses `https://api.openagents.com/v1`.
  Development uses `http://127.0.0.1:8080/v1`, selected by the
  `language_models.openagents.use_development_api` setting. Both paths reuse
  the verified OpenAgents session token; neither asks for a local provider key.
- **Root cause, fixed alongside:** the multi-account registry migration left
  pre-existing identities `candidate_existing` with `recovery: needed`, and
  the signing gate (`validate_signing_selection`, `record_signer_use`)
  required `Active` + `Protected`, while the broker collapsed every registry
  refusal into "the active account selection changed". Candidate lifecycles
  are signable, recovery protection gates onboarding rather than proof
  signing, the broker names the real refusal
  (`SignerBrokerError::AccountNotSignable`), a genuine mid-proof generation
  bump re-resolves and retries once, and startup hydration resolves
  legacy-root custody instead of reporting `Absent`.
- **Enforced by:** `the_core_agent_defaults_to_the_server_routed_omega_agent`
  and the amended `the_default_model_is_pinned` /
  `zero_base_input_bar_offers_one_cloud_native_omega_agent` in
  `crates/omega_deltas`; OpenAgents request and endpoint tests in
  `language_models`; `omega_agent_does_not_fall_back_to_a_client_provider` in
  `agent`; `migrated_candidate_existing_account_without_recovery_signs` and
  `partition_identity_service_resolves_legacy_root_storage` in
  `omega_identity`.

### OMEGA-DELTA-0202 — One server authority for Omega Agent routing

- **Origin:** owner evidence 2026-07-31 showed why independent client model
  state drifts: the input bar said Luna while Kimi answered, and fallback
  state could change without the label changing.
- **Amended 2026-08-08:** Omega Agent no longer exposes provider models in the
  IDE. The client sends one logical `omega-agent` model to the OpenAgents
  Responses API. The API owns provider selection, reasoning, routing, and
  fallback.
- **Input-bar contract:** the composer says **Omega Agent** before and after a
  session exists. Its agent chooser remains available, but its model and
  reasoning collections are empty. External agents retain their own controls.
- **Failure contract:** an Omega Agent request never falls through to another
  client language-model provider. Such a fallback would bypass server account
  policy and split observability and routing decisions across two authorities.
- **Records remain exact:** executor disclosures and receipts may still carry
  exact provider/model metadata returned by the service. That machine-readable
  state does not become a client routing control.
- **Enforced by:** `omega_agent_routing_has_one_server_authority` in
  `crates/omega_deltas`; request-shape and endpoint tests in `language_models`;
  `omega_agent_picker_has_no_client_model_controls` in `agent_ui`; and
  `omega_agent_does_not_fall_back_to_a_client_provider` in `agent`.

### OMEGA-DELTA-0203 — An agent Omega offers to delegate to is an agent Omega can start

- **Origin:** owner evidence 2026-07-31. Delegating to the SCV subagent
  answered *"Could not start the SCV (`scv`) agent server for this
  subagent"* — on a machine where `scv` was installed, handshakes over ACP,
  and was found by detection.

**Two registries decided one thing.** The catalog a delegation is offered from
is `omega_agent_detect::CANDIDATES`, a `PATH` scan. The launcher is
`CustomAgentServer`, which resolves an id through the agent-server store, and
that store knows only what `assets/settings/default.json` and the ACP registry
declare. Nothing held the two together, so an id could be in one and absent
from the other, and the surface drew a delegation target that could not start.
`scv` was not the only one: `cursor` and `github-copilot-cli` were both
offered and both unstartable, and had been since they were added.

- **Upstream Zed:** has neither registry. Its agents are the ones settings and
  the registry name, and it never draws a delegation target from a `PATH` scan
  — so it has no way to disagree with itself and nothing to reconcile.
- **Omega:** each candidate declares how it is started, as
  `AgentLaunch::AgentServerStore` or `AgentLaunch::DetectedBinary`. There is no
  third case and no default, so a candidate added without a launch definition
  does not compile. `AgentServerStore` requires a `default.json` entry, which
  is checked mechanically; `DetectedBinary` is started as the file detection
  resolved, through `CommandAgentServer`.
- **Why the two kinds:** `cursor` and `github-copilot-cli` are in the ACP
  registry, so a `{"type": "registry"}` entry is the honest wiring for them.
  `scv` is first-party, in no registry, and ships inside the application — a
  settings entry naming a bare `scv` would resolve only from a shell `PATH`,
  which a packaged application does not have.

**The shipped binary was not shipped.** `crates/scv` is first-party and offered
by the product, and nothing built or installed its binary. The owner had to
`cargo build -p scv` and copy it to `~/.local/bin` by hand, which is why an
agent the product advertises was invisible on every other machine. The
packaging scripts now build it with the same profile and target as `omega` and
place it beside the application executable, and detection resolves that
directory before `PATH` so the binary this build shipped wins over a stale copy
someone left on `PATH`.

- **Known consequence, stated plainly:** an agent-server-store candidate is
  proven *registered*, not proven *installable*. The registry entry is what the
  store needs to attempt a launch; whether the ACP registry can then fetch that
  agent is the agent's own failure to report, and it now reports it instead of
  Omega refusing before trying.
- **Enforced by:** `every_delegable_agent_can_be_started` in
  `crates/omega_deltas`; `every_candidate_declares_how_it_is_launched`,
  `a_detected_agent_carries_its_launch_spec` and
  `a_binary_beside_the_executable_wins_over_one_on_the_path` in
  `omega_agent_detect`.

### OMEGA-DELTA-0204 — The composer offers the same controls before a session exists as after

- **Origin:** owner evidence 2026-07-31, two screenshots side by side. A brand
  new thread and a thread one message old. *"new chat threads dont show the
  dropdowns and voice button that existing threads do, like the input has fewer
  options on new thread, it should be the exact fucking same."*

There are two composers, not one. `ConversationView::render_loading_composer`
draws the box while the executor connects; `ThreadView::render_message_editor`
and `render_zero_base_executor_bar` draw it once a session exists. They are the
same box in the same place by construction — `OMEGA-DELTA-0122` built the first
one specifically so a person cannot see the composer change — and their control
rows had drifted into two different rows.

The pre-session bar carried the executor dropdown and Send, on the left. The
connected bar carried the executor dropdown, the Luna/Flash/Pro tier dropdown,
the microphone and Send, on the right, and the field carried an expand control.
So the composer a person meets **first** was the most reduced composer in the
app, and three controls appeared a moment later once a session existed.

**Nothing about an unconnected session made any of them unavailable.** That is
what makes this a defect rather than a limit:

- **Voice** is a Sarah session on the *workspace*, keyed by workspace entity
  id. It has no relationship to an ACP session at all. It was missing because
  `render_voice_controls` was a private method on `ThreadView`, so the code
  that draws it was out of reach — the control was not withheld, it was
  unreachable. It now lives in `crates/agent_ui/src/composer_voice.rs` beside
  the status it reads, and both composers call the same function.
- **The tier** is a settings default that the registry hands every thread when
  it is created. The connected control reaches the running session through its
  model selector, which also writes `agent.default_model`; the pre-session
  control has no session and no selector, so `select_before_session` writes
  that default itself. This mattered more than the missing pixel: a tier
  control that moved its face and not the model is exactly the disagreement
  `OMEGA-DELTA-0202` exists to forbid, one composer earlier. `RoutedFace::pending`
  is the documented face for a thread that has not routed anything yet, and it
  is the only place the standing choice is permitted to speak.
- **Expanding a text field** is local to the field. `OMEGA-DELTA-0100` already
  argued this exact case for the connected composer — *the first message in a
  thread is the one most likely to be long, because it is the one that states
  the task* — and the pre-session field is where that first message is actually
  typed. `ConversationView` keeps its own expanded bit rather than sharing
  `ThreadView`'s, because the two composers are different editors with
  different lifetimes; the state does not survive the handover any more than
  the caret position does.

**What stays absent.** The executor disclosure line, the routed model, the
turn's phase dot and the Exo inspector are all reports about work that has
happened. Before a session there is none, so they are not drawn and not
invented; the `OMEGA-DELTA-0189` connecting indicator occupies that space
instead. `OMEGA-DELTA-0175`'s Vim readout stays on the left in both bars.

- **What this does not claim.** The parity check is source text: it fails when
  a listed control disappears from either bar, and it does not notice a *new*
  control added to only one of them. `SHARED_COMPOSER_CONTROLS` is the list,
  and extending it is part of adding a composer control.

- **Enforced by:** `the_composer_offers_the_same_controls_before_and_after_the_session`
  in `crates/omega_deltas`; and
  `a_tier_chosen_before_the_session_exists_moves_the_model` in `agent_ui`.

### OMEGA-DELTA-0205 — Collapsed threads sidebar shares the activity rail

**When the threads sidebar is collapsed, it does not draw a second vertical
rail.** Expand/collapse sits at the top of the workbench activity rail, and
Settings sits at the bottom of that same rail. The expanded sidebar keeps its
header collapse control, sections, Pair phone, and version label; only the
duplicate Settings row leaves the sidebar footer.

**Why.** A collapsed 30-pixel rail beside the 40-pixel activity rail was two
left columns answering one question — "where are the navigation controls?" —
and wasted width without adding capability. The activity rail is already the
leftmost control strip; hosting the sidebar toggle and Settings there removes
the empty strip while keeping both controls drawn (so they never retreat into a
menu, which is the defect `OMEGA-DELTA-0118` repaired).

**Layout.** Collapsed sidebar width is zero. Dock clamping and the shared
allocator therefore reserve only the activity rail plus the transcript floor
when the sidebar is not expanded. The narrow dock-visible boundary is:

```text
40 activity rail + 240 dock + 600 transcript = 880
```

**Labels.** Expand keeps tooltip and accessible name `Expand Sidebar`; collapse
on the activity rail uses `Collapse Sidebar` (the expanded header still has its
own collapse control). Settings keeps tooltip `Settings` and accessible name
`Open Settings`, and still dispatches `omega::OpenSettings`.

- **Enforced by:** `collapsed_threads_sidebar_controls_live_on_the_activity_rail`
  in `crates/omega_deltas`; `a_collapsed_sidebar_takes_no_column_of_its_own` in
  `crates/agent_ui/src/omega_sidebar.rs`; and the workbench layout unit tests in
  `workbench_shell`.

### OMEGA-DELTA-0206 — The finish word is read without case as authority

**A completion's `finish_reason` is normalized before it is matched, so a
provider's capitalization cannot turn a completed turn into an unknown one.**
`STOP` and `stop` are the same answer. So are `MAX_TOKENS` and `length`, and
`SAFETY` and `content_filter`.

**Why.** Gemini's OpenAI-compatible surface writes the uppercase enum from
Gemini's own API — `STOP`, `MAX_TOKENS`, `SAFETY`, `RECITATION`,
`TOOL_CALLS` — and the hosted Gemini lane
(`openagents/gemini-3.6-flash`) reaches the same `open_ai` mapper every other
OpenAI-compatible provider does. That mapper matched three lower-case literals,
so every healthy Gemini turn took the unknown arm and logged
`Unexpected OpenAI stop_reason: "STOP"`. The turn was then reported as
`EndTurn` by accident rather than by reading, and a turn cut off at the token
limit was reported as a turn that finished.

**Where this is owned.** In the `open_ai` crate, not in the gateway. The
OpenAI-compatible wire format is what this crate parses, and Gemini's uppercase
enum is part of that format as Gemini serves it — as are the same words served
through OpenRouter and through a person's own configured Gemini base URL, which
never pass through our gateway at all. Normalizing at the gateway would mean
rewriting partner SSE bodies, which would break passthrough's defining
property and still leave the direct routes broken.

**What is now read rather than guessed.** `length` and `max_tokens` stop with
`MaxTokens`; `safety`, `recitation`, `blocklist`, `prohibited_content`, `spii`,
`content_filter` and `image_safety` stop with `Refusal`; `function_call` drains
tool calls exactly as `tool_calls` does. `length` had no arm at all before this,
so even upstream-spec OpenAI truncation was reported as a completed turn. A word
that is still unknown is logged as the provider actually wrote it, not as the
normalized copy, so it stays diagnosable.

- **Enforced by:** `a_provider_finish_word_is_read_without_case_as_authority` in
  `crates/omega_deltas`; and
  `gemini_upper_case_finish_reasons_are_not_unknown`,
  `an_upper_case_tool_call_finish_reason_still_drains_the_calls`,
  `an_unknown_finish_reason_still_ends_the_turn` in
  `crates/open_ai/src/completion.rs`.

### OMEGA-DELTA-0207 — The input bar names Omega Agent, not a provider model

- **Origin:** the former provider-model label could disagree before a thread
  resolved because it guessed from process-wide model-tier state.
- **Amended 2026-08-08:** the composer no longer predicts or displays the
  provider model. Both the pre-session and active-thread paths construct the
  same `ComposerModelPicker::omega_agent()` value and display **Omega Agent**.
- **Why:** provider routing is a server decision. A provider model shown before
  the response begins is either an implementation leak or a promise the IDE
  cannot keep. Exact routing metadata can arrive with response events for
  receipts, diagnostics, or a later routing-details UI.
- **Scope:** this applies to Omega Agent. External ACP agents can still name
  models and expose configuration controls they own.
- **Enforced by:**
  `the_input_bar_names_omega_agent_without_guessing_the_provider_model` in
  `crates/omega_deltas` and
  `omega_agent_picker_has_no_client_model_controls` in `agent_ui`.

### OMEGA-DELTA-0208 — One model, named once, by its own name

**A person's chrome names the model that is serving a thread exactly once, and
by the name the model has.** `openagents/kimi-k3` is a wire identifier. It stays
on the record, where receipts, copied system specs and machine readers want an
exact pair; it does not appear on the composer, the disclosure line, or the
thread toolbar.

**Why.** The composer drew two lines from one fact. The first was the record's
own `ExecutorDisclosure::label`, which names the model by its `provider/model`
pair. The second, added by `OMEGA-DELTA-0202`, was the model's own name. So a
thread on Kimi read `Omega Agent · openagents/kimi-k3` with `Kimi K3` directly
beneath it — one fact stated twice, once in a vocabulary this surface is not
allowed to teach. The owner: "remove the `openagents/gpt-5.6-luna` … its
duplicative with gpt 5.6 luna like the real name."

This is the same law that took the class token off this row in omega#100. The
tokens `native_loop`, `external_acp` and `engine_lane` were removed from the
line because a person is not here to learn Omega's routing vocabulary, and
`openagents/kimi-k3` is that vocabulary's other half. The standing
no-exposition law does not distinguish between them.

**What now holds.** `ExecutorDisclosure::label_with_model` owns the *shape* of
the line — who ran it, then the model, then the run, then a fallback when there
was one — and nothing else may build that line by hand.
`ExecutorDisclosure::label` calls it with the wire pair and is what a receipt
renders. `omega_routed_model::chrome_line` calls it with
`RoutedModel::human_name`, and is what every surface a person reads renders.
Because the model phrase is the only thing that varies, a surface choosing a
different word for the model cannot also drift into a different line.

**What did not change.** The run reference is still said. A fallback route a
person could not otherwise see is still said. A model nobody disclosed is still
*said* to be undisclosed rather than quietly dropped — `chrome_line` falls back
to the record's own phrase for exactly that case, so the surfaces still agree
even about their ignorance. `OMEGA-DELTA-0021`'s property is intact: every
thread surface still names its executor, still rendered from the typed record
and never from a stored or hand-built string.

**The second line is folded, not deleted.** `OMEGA-DELTA-0179` required a status
line to survive the loading composer's replacement by the physical session, and
`OMEGA-DELTA-0202` held that line to *at most the model name*. Because it was
already held to the model name and nothing else, folding it into the disclosure
line loses no fact — the same `Role::Status` region on the same two surfaces now
carries the whole line. `RoutedModel::status_line` is renamed `human_name`,
since it is a model's name rather than a line.

- **Enforced by:** `the_chrome_names_one_model_once_and_never_by_its_wire_id`
  in `crates/omega_deltas`; and
  `the_chrome_line_names_one_model_once_and_never_by_its_wire_id`,
  `folding_the_line_kept_every_other_part_of_it` in
  `crates/agent_ui/src/omega_routed_model.rs`.

### OMEGA-DELTA-0209 — An agent Omega offers to delegate to is one Omega sends a shape it accepts

- **Origin:** owner evidence 2026-07-31, omega#160, on dev build `d45a3b214c`
  — the build that carried `OMEGA-DELTA-0203`. Three test delegations to SCV,
  three failures: *"The SCV subagent failed: Invalid params:
  {"code":"invalid_params","message":"invalid tool request: expected value at
  line 1 column 1","path":""}"*. The delegated task was the prose *"Perform a
  read-only test delegation: report the project root path and list one or two
  top-level entries. Do not modify files."*

**Starting it was only half the law.** `OMEGA-DELTA-0203` closed the gap
between the catalog a delegation is offered from and the launcher that starts
it, and the owner's next three delegations still failed — further along, on the
first turn, with the agent running and the session open. The remaining gap was
the other end of the same seam: `spawn_agent` sends `task` to every ACP agent
uniformly, as prose, because every agent Omega had ever delegated to had a model
to read prose with. `scv` does not. It is a deterministic tool server whose
prompt must *be* a JSON tool request, `PromptToolRequest::parse` is
`serde_json::from_str` on the prompt text, and a sentence fails it at line 1
column 1 every time.

Neither side was wrong alone, which is why nothing caught it. `scv` parsed JSON
correctly and refused non-JSON correctly; Omega sent the parent's task
faithfully. The defect existed only between them, and only a test that crossed
the boundary could see it.

- **Upstream Zed:** delegates to model-backed agents only, so "the task is
  prose" is true of every target it has. It has no modelless agent to be wrong
  about.
- **Omega:** each candidate declares its prompt contract, as
  `AgentPromptContract::Prose` or `AgentPromptContract::Structured`. There is no
  third case and no default, so a candidate added without a contract does not
  compile — the same shape `AgentLaunch` takes, for the same reason. `scv` is
  `Structured`, and its request is `omega_agent_detect::SCV_REQUEST`.
- **The model is told.** `SpawnAgentTool::description` appends the structured
  targets and their exact requests, generated from the catalog rather than
  written into the doc comment, so an agent added to the catalog is described by
  the edit that adds it. A model handed the request writes the request; a model
  handed a sentence about JSON writes a sentence.
- **The shaping is not interpretation.** `shape_delegated_task` removes
  surrounding whitespace and one Markdown code fence — a model told to emit JSON
  very often emits JSON in a fence, and unwrapping a fence decides nothing — and
  then requires a JSON object. It never invents a request from prose. A `read`
  of a file nobody named would be an answer to a question nobody asked.
- **The refusal is honest and early.** A task a structured target cannot parse
  is refused before the process is launched, as
  `DelegateFailureClass::TaskNotInContract`, in one line that names the shape.
  Its own class because it asks for something different from an execution error:
  not a retry, a rewrite. A follow-up turn on an existing session is shaped from
  the live disclosure, whether or not it names an executor again.
- **SCV says it too.** A client that is not Omega still reaches `scv` directly,
  so its own refusal now names the request as well. `expected value at line 1
  column 1` states where a parser stopped and nothing a reader can act on.

**The tool described itself to nobody.** Checking that a *new* line of guidance
reached the model found that none of it had. `AgentTool::description` reads the
input type's JSON schema; `efdc784fa9` replaced `SpawnAgentToolInput`'s derived
`JsonSchema` with a hand-written one to pin the delegate contract, and a
hand-written `json_schema!` carries no doc comment. So `spawn_agent` had shipped
with an empty description ever since — every word about designing subtasks,
parallel delegation, choosing an executor and reading the result was written,
reviewed, and served to nothing. It is load-bearing here, because telling the
model a target's contract is the whole of the fix, so the guidance is now a
constant the description serves rather than a doc comment nothing reads.

- **Known consequence, stated plainly:** a structured agent still cannot be
  delegated a task in prose, and no error message can give it a model. What the
  delta guarantees is that Omega tells the delegating model the shape before it
  writes the task, and says the shape again if it writes the wrong one.
- **Enforced by:** `every_delegable_agent_accepts_the_shape_omega_sends` in
  `crates/omega_deltas`; `a_delegated_read_emits_the_content_as_a_completed_tool_call`,
  `a_delegated_read_returns_the_file_it_asked_for`,
  `the_shape_omega_advertises_is_the_shape_scv_documents` and
  `the_advertised_commands_are_the_catalogs_tools` in `crates/scv/tests/omega_delegation.rs`,
  against the real binary; `every_candidate_declares_the_shape_of_a_task`,
  `prose_is_refused_by_naming_the_shape` and
  `a_request_buried_in_prose_is_not_dug_out` in `omega_agent_detect`;
  `the_delegate_description_names_every_structured_contract`,
  `the_delegate_tool_is_described_to_the_model`,
  `an_unshapeable_task_is_not_an_execution_error` and
  `a_task_a_structured_target_cannot_parse_is_refused_by_naming_the_shape` in
  `agent`.

### OMEGA-DELTA-0210 — The hang detector owns the trace it writes

**A hang trace has the timings in it.** Omega's hang detector enables gpui's
task-timing trace itself, with a bounded per-thread window, and writes the
trace whenever a hang produced one.

**Why.** The owner's dev build logged `New foreground hang detected: Tasks(s)
that ran too long` and wrote
`~/Library/Application Support/Omega Dev/hang_traces/hang-2026-07-31_02-56-02.miniprof.json`.
That file — and every hang trace Omega had written — contained `"timings": []`
for all eleven threads. A file that exists, is named after the hang, is rotated
on a three-file cleanup, and cannot explain anything.

Two independent defects, stacked:

1. **Nothing enabled tracing.** `gpui::profiler::save_task_timing` pushes into
   the per-thread ring buffer only `if trace_enabled()`, and `PROFILER_ENABLED`
   is an `AtomicBool::new(false)` that only `set_trace_enabled` flips. Upstream,
   the one caller was `crates/miniprofiler_ui`. `OMEGA-DELTA-0186` deleted that
   crate with the rest of the legacy editor surface, which left
   `set_trace_enabled` with **no callers at all**. The deletion was right; the
   orphaned switch was not noticed.
2. **The writer refused to write when tracing was on.** `task_traces::save_any`
   read `if profiler::trace_enabled() { None } else { …write… }`. That guard is
   correct upstream, where a live miniprofiler session owns the buffer and a
   second consumer would fight the viewer. With the viewer gone it wrote the
   file *only* in the state where the buffer is guaranteed empty.

Each defect alone produces an empty trace. Together they made the emptiness
look intentional.

**Why the statistics still worked.** `TaskStatistics` is gathered
unconditionally, which is why the log line still named a frame — `110.114875ms
- crates/session/src/session.rs:75:16` an hour earlier in the same session —
while the trace file could not. The surface that appeared to work is what kept
the surface that did not from being noticed.

**What now holds.** `start_hang_detection` calls
`profiler::set_trace_enabled_with_capacity(true, 8192)`. The hang detector is
the only thing left in Omega that reads the buffer, so it is what turns the
buffer on; a buffer whose only reader does not enable it is a buffer that is
never read. `save_any` writes unconditionally. If a live trace consumer ever
returns, deferring is *its* business, not the writer's.

**The window is bounded, and the bound is the caller's.**
`set_trace_enabled` reserves `MAX_TASK_TIMINGS` — 16 MiB **per thread** — which
is sized for a viewer somebody is reading. A trace that is on for the life of
the process pays that on every thread forever, so
`set_trace_enabled_with_capacity` lets the hang detector ask for the recent
window it actually dumps: 8192 timings, a few hundred KiB a thread. The
eviction keeps the newest, because a hang is explained by what ran just before
it.

**This does not claim to have explained the 02:56 hang.** That trace is empty
and cannot be recovered, and the log frames went to a terminal rather than
`~/Library/Logs/omega-dev/omega-dev.log`, because the process was launched from
an interactive shell and `stdout_is_a_pty()` routes logging to stdout. What this
delta guarantees is that the next one is answerable. The open question it leaves
is recorded in omega#190.

- **Enforced by:** `the_hang_detector_enables_the_trace_it_writes` in
  `crates/omega_deltas`; and
  `an_enabled_trace_records_within_the_window_its_caller_paid_for` in
  `crates/gpui/src/profiler.rs`.

### OMEGA-DELTA-0211 — The composer's microphone never takes the conversation away

**The composer voice control never navigates.** It opens audio when the exact
admission terms are already loaded and admitted, loads them and opens audio
when they are not, and when voice is refused it draws one short dismissible
sentence anchored to the button that was clicked. The detailed admission
surface — cohort reference, refusal reason, rate, credit hold, remaining
credit, duration, transcript policy, bounded capabilities — is not deleted and
not weakened. It is reached from Sarah's chooser, the `+` menu, and the Thread
menu, by a person who went looking for it. Focused Settings does not proxy the
admission action across windows.

**Why.** The microphone sat in the composer, beside Send, on the surface where
a person writes. `OMEGA-DELTA-0180` wired every non-active phase of it —
`Idle`, `Unavailable`, `AccessRequired`, `Error`, `Reconnecting` — to
`agent::OpenSarahAdmission`, which sets `showing_sarah_admission` and replaces
the whole panel. So the ordinary gesture "I would rather say this than type it"
answered by discarding the view of the half-written message and presenting
`sarah_voice_cohort:alpha_v1`, `cohort_inactive`, and msat arithmetic. The
owner's words: *"When I click the voice button, I get this horrible screen. I
never want to see this shit, at least not navigated to from the input
composer."*

**The interstitial was not protecting anything either.** The admission page's
job is exact-terms disclosure before spend, and that job is done by the
projection, not by the page: `start_voice` refuses unless the workspace's
projection is `Ready`, and `has_same_reviewed_terms` re-refuses if OpenAgents
changes a term between review and connect. A composer that dispatches
`StartVoice` only on an already-`Ready` projection is inside the same gate the
page was inside. Nothing about the fail-closed contract required the person to
be moved.

**Drawn implies working, on both branches.** The click has to have a visible
consequence in every state, which is what made the naive fix wrong: a composer
that dispatched `StartVoice` with no prepared admission would publish
`preflight_required` into a projection the composer does not draw — a silent
no-op wearing a button. So `workroom::StartVoiceFromComposer` loads the terms
and opens audio when they arrive, and every refusal on that path — public demo,
community room, pending settlement, malformed terms, a cohort or credit blocker
— raises the same one line the composer draws for the states it can already
read.

**One line, not a page in miniature.** `COMPOSER_VOICE_NOTICE_COPY` is
mechanically held to a single short sentence with no cohort reference, no
refusal token, and no credit number. The notice is anchored to the microphone
rather than toasted into a window corner, which is `OMEGA-DELTA-0053` and
omega#119's existing law about where a refusal belongs, not a second one.

- **Enforced by:**
  `the_composer_microphone_never_navigates_away_from_the_conversation` in
  `crates/omega_deltas`; the amended
  `sarah_voice_admission_is_visible_bounded_and_fail_closed`, which now asserts
  the converse of what it used to; and
  `the_composer_microphone_never_navigates_to_the_admission_page` in
  `crates/agent_ui/src/agent_panel.rs`, which clicks the rendered control while
  voice is unavailable and asserts `omega.sarah.admission` is never drawn.

### OMEGA-DELTA-0212 — Sarah's admitted LiveKit room carries media, not authority

**The microphone stays off until OpenAgents admits an exact room.** Omega asks
for `livekit_room_v1` while consuming the terms the person reviewed. It rejects
unknown transports and malformed, non-OpenAgents, overprivileged, expired, or
incomplete LiveKit grants. Only after that generation-bound response exists
does the desktop open its selected audio devices.

**LiveKit is deliberately narrow.** The desktop joins with automatic
subscription disabled, publishes one microphone source, and subscribes only to
audio from the exact admitted Sarah participant. An unexpected participant
fails the session closed. Omega's existing capture, echo cancellation,
playback, mute, and interruption path remains the single audio owner; raw media
is not logged or persisted.

**The existing control plane remains authoritative.** Admission review, the
one-use session ticket, lifecycle, attributed transcripts, bounded command
proposals, interruption, usage, and settlement continue over the authenticated
OpenAgents WebSocket and API. LiveKit reconnect changes visible media state but
does not mint a second generation. `custom_wss_v1` remains an explicit
rollback transport and cannot be selected silently during an active session.

**Cleanup is part of the contract.** Every end and failure path signals the
media task, clears queued microphone frames, closes the room, terminates remote
audio stream tasks, closes the control socket, and releases capture and
playback.

- **Enforced by:**
  `sarah_livekit_media_is_generation_bound_narrow_and_beneath_control` in
  `crates/omega_deltas`; and the focused GPUI tests
  `livekit_subscription_accepts_only_sarah_audio` and
  `constructing_an_admitted_livekit_client_does_not_open_audio` in
  `crates/workroom_ui/src/voice.rs`.

### OMEGA-DELTA-0213 — Sarah LiveKit release claims require one immutable evidence binding

The installed-candidate gate always emits separate rows for private voice,
authenticated community rooms, UDP/TCP/TURN connectivity, concurrent-room
isolation, the failure/privacy matrix, and independent review. Those rows do
not inherit a verdict from source tests or from one successful headless room.

To promote a row, the gate requires a public-safe evidence manifest bound to
the exact Omega source and package digest, OpenAgents source revision, LiveKit
infrastructure/config/server-image revisions, Sarah worker source/image
revisions, admitted model revision, and price-catalog revision. Evidence paths
must exist in the repository, remain repository-relative, match their SHA-256,
and be marked public-safe. A manifest for another package is rejected; absent
evidence is blocked; incomplete observation is inconclusive. Owner-assisted
success is a distinct preserved status, not an automated pass.

- **Enforced by:**
  `sarah_livekit_release_rows_refuse_unbound_or_private_evidence` in
  `crates/omega_deltas`; `script/omega-release-gate --self-test`; and the
  checked-in evidence schema and valid fixture under
  `script/fixtures/omega-release-gate`.

### OMEGA-DELTA-0214 — A new thread never asks whose worktree this is

- **Upstream Zed:** no concurrency guard at all. Two agents in one checkout
  overwrite each other silently.
- **Omega before:** `OMEGA-DELTA-0181` added the guard and made it a modal.
  Pressing New Thread while another agent was running produced a blocking
  dialog titled **"Another agent is already using this worktree"**, four lines
  explaining that concurrent writes can conflict, and a choice between
  **Cancel** and **Run here anyway** — at the moment a person had just asked
  for a thread and wanted to type into it.
- **Omega now:** the collision is resolved instead of narrated. When a new
  thread's root is already held by a live, write-capable thread, Omega
  provisions a linked git worktree for it and runs there. No prompt, no
  interstitial, no name to invent.
- **Why:** the owner, shown the modal: *"i never want to see that shit. figure
  out better workflow."* The guard's intent was right and the UX was wrong.
  `OMEGA-DELTA-0001` is the same shape — the same owner, the same reaction to
  a dialog that made a person clear a warning before doing the thing they had
  already asked for. A guard that stops the person to re-ask a question they
  can answer once is not a safety mechanism, it is a tax on the safe path.
- **The safety property is strictly stronger, not relaxed.** The modal
  disclosed the hazard and then handed the person a button that walked into it.
  Isolation removes the hazard: after provisioning there is no shared tree to
  conflict over. `OMEGA-DELTA-0181`'s claim, canonicalization, overlap, and
  remote-identity rules are untouched and still run.
- **One setting, not one question per turn.** `agent.thread_worktree` is
  `isolate` (default) or `shared`. `shared` is the explicit decision
  `OMEGA-DELTA-0181` demanded, made once in settings: it provisions nothing and
  asks nothing, because the person has already said that concurrent agents may
  write one checkout.
- **Decided at thread creation, because later is too late.** `ConversationView`
  hands its working directories to `new_session`, and every external ACP agent
  reports `supports_live_work_dir_updates() == false`, so a Codex, Claude, or
  Grok session cannot be moved once it has started. The decision is therefore
  made synchronously in `AgentPanel::create_agent_thread_inner`, before the
  session exists. The `git worktree add` behind it is not synchronous: the
  thread opens immediately and the session load awaits
  `ConversationView::set_pending_work_dirs`, so the new-thread gesture never
  waits on git.
- **Occupancy is knowable without a claim.** `OMEGA-DELTA-0181`'s claim is
  turn-scoped and does not exist between turns, so `AgentSupervision` also
  records each thread's bound roots and answers `occupant_for` from those
  bindings plus the lifecycle it already keeps. Live means a held claim or a
  non-terminal lifecycle; a finished thread does not occupy anything, so
  ordinary sequential work provisions nothing. The comparison reuses
  `WorktreeScope::overlaps`, so there is one path-overlap rule, not two.
- **It reuses the worktree stack rather than growing a second one.**
  `git_ui::worktree_service::create_linked_worktrees` is the same
  name-generation, target-path, dedup, rollback, and provenance sequence as the
  worktree picker's, with `open_worktree_workspace` removed — auto-isolation
  must not open a workspace tab, because it is a quiet correction and not a
  navigation. Names come from the thread's title when it has one and from
  `generate_worktree_name` when it does not, which is almost always: a
  brand-new thread is still `DEFAULT_THREAD_TITLE`. A person is never asked to
  name a worktree Omega decided to create.
- **The guard stays, and never prompts.** `request_worktree_admission` has no
  `window.prompt` and `WorktreeAdmission` has no `Cancelled` variant. On a
  collision it isolates when the session can still be retargeted, and otherwise
  discloses on the thread — occupying thread, executor, and path, one line,
  dismissible, blocking nothing — then proceeds. `OMEGA-DELTA-0189`'s no-
  exposition law governs that line: the modal's paragraph about what concurrent
  writes can do to a checkout is not restated.
- **Deliberate isolation is a control, not a dialog.** The per-thread worktree
  picker gains **New worktree**, which provisions through the same path and
  retargets the active conversation.
- **Isolation does not move the conversation to another project.** The linked
  checkout becomes the thread's working folder, while its main-worktree path
  remains the project where the conversation was created. The threads sidebar
  groups by that main path, so switching away cannot make an isolated chat
  disappear. A one-time metadata repair resolves linked checkouts through
  Git's `commondir` and restores rows written before this invariant was
  enforced.
- **Known gap: no reaper.** A worktree provisioned for a draft the person then
  abandons — switching executor on an empty draft while another agent is live
  rebuilds the draft — stays on disk. So does an archived thread's.
  `docs/src/ai/parallel-agents.md` used to promise that archiving a thread
  saved and removed its worktree and that restoring restored it;
  `thread_worktree_archive.rs` implements `build_root_plan`,
  `persist_worktree_state`, `remove_root`, and `restore_worktree_via_git`, and
  none of them has ever been called outside tests, so that promise was unbacked
  before this change and the doc no longer makes it. Auto-provisioned worktrees
  are recorded as Omega-created and are therefore *eligible* for that
  reclamation, but wiring a destructive archive path is a separate change with
  its own owner review; see omega#155. Both leaks are bounded by the fact that
  nothing is provisioned unless a root is genuinely occupied by a live agent.
- **Enforced by:** `a_new_thread_isolates_instead_of_asking` in
  `crates/omega_deltas`; the amended
  `concurrent_agent_supervision_is_visible_durable_and_guarded`, which now
  asserts the absence of the prompt it used to assert the presence of;
  `a_bound_live_thread_occupies_its_root_without_holding_a_claim`,
  `a_terminal_thread_does_not_occupy_its_root`, and
  `removing_a_snapshot_releases_its_binding` in
  `crates/agent_ui/src/omega_agent_supervision.rs`; the slug tests in
  `crates/agent_ui/src/omega_thread_worktree.rs`; the isolated-worktree
  identity and migration tests in `crates/agent_ui/src/thread_metadata_store.rs`;
  `assert_no_worktree_collision_prompt` in the Agent Panel GPUI suite; and the
  `omega_concurrent_agents_worktree_no_dialog` visual scene.
### OMEGA-DELTA-0215 — Community Sarah never manufactures room authority

**The tester channel can draw the whole journey without pretending it is
live.** The compact surface names room voice, Sarah state, and the speaking
floor, and exposes join, leave, mute, summon, remove, Talk to Sarah, and
moderator stop. Disclosure is visible before Sarah may listen.

**Every enabled control comes from one exact server projection.** The
projection binds the community and channel, membership and E2EE revisions,
room epoch, Sarah identity and participant, dispatch and provider generation,
and a verified Nostr-user-to-LiveKit-participant roster. The floor holder must
be in that roster. Replays, cross-room mappings, expiry, removal, role loss,
revocation, and server stop clear authority and mute input.

**Community is not private voice.** Its fixed capability profile is
`community_member_v1`; no owner-private workspace or command authority exists.
The 30-second speaking floor remains a canonical OpenAgents lease and LiveKit
remains media only.

**The desktop consumes the production contract, but live proof remains a
release gate.** Omega joins with the authenticated authority response, pins the
admitted room and participant identities, publishes a default-muted microphone
only while the local verified floor lease is held, subscribes only to verified
Sarah audio, refreshes authority and roster state, and tears the transport down
on disconnect or revocation. Hermetic tests prove those boundaries; the
three-desktop installed-candidate journey is still required before release. An
unjoined or unavailable room says **Mic off**, never **Mic on**.

- **Enforced by:** `community_room_sarah_is_verified_bounded_and_fail_closed`
  in `crates/omega_deltas`; the model tests in
  `crates/agent_ui/src/omega_public_channel_sarah.rs`; and
  `community_sarah_controls_are_compact_and_fail_closed_until_verified` in
  `crates/agent_ui/src/omega_public_channel_view.rs`; and both tester-channel
  visual scenes, which require every authority-bearing room control to remain
  disabled while the contract is unavailable.

### OMEGA-DELTA-0216 — A broken visual launcher fails the ordinary preflight

The hermetic visual lane was silent after the application package changed from
`zed` to `omega`: its launcher still built the old package and exited before a
scene, diff, or receipt existed. Baseline drift then accumulated without a red
artifact.

The launcher now builds the `omega` package and its reviewed catalog is pinned
at 95 scenes. The ordinary `omega_deltas` preflight binds the launcher command,
the application package name, list-mode environment, and exact catalog count.
A later rename or unreviewed catalog change therefore fails before it can turn
the visual lane into silence again.

- **Enforced by:** `workbench_visual_launcher_tracks_the_application_package`
  in `crates/omega_deltas`; `script/omega-workbench-proof --list`; and
  `omega_workbench_harness::validate_scene_catalog`.

### OMEGA-DELTA-0217 — Sarah LiveKit release evidence is executable, not testimonial

**A pass now has a row-specific machine shape.** The six-row LiveKit manifest
still binds one exact Omega package, source revision, OpenAgents revision,
LiveKit infrastructure/config/server image, Sarah worker source/image, model,
and price catalog. In addition, every passing row must carry the observations
that make that row true. Three-desktop room evidence cannot pass without three
authenticated desktops, one verified Sarah identity, floor transfer, shared
audio, moderator stop, and non-floor/removed-member refusals. Failure evidence
cannot pass without all eight drills, eight privacy scopes, non-overlap, and
exact settlement.

**Every referenced receipt repeats the binding.** A repository-relative hash
is not enough: the referenced file must be bounded public-safe JSON, repeat
the exact manifest binding plus row/status, and exclude secret/media/private
material keys. `script/assemble-omega-sarah-livekit-evidence` computes the
candidate and receipt hashes from an installed DMG and release record; the
release gate recomputes them independently.

**Blocked remains honest.** The assembler accepts blocked and inconclusive
receipts without manufacturing pass facts. OpenAgents now owns the durable
community-room rendezvous and Omega consumes its authority and media contract,
but the committed rc28 report stays blocked until an installed candidate
produces the bound three-desktop and failure-drill receipts.

- **Enforced by:** `script/omega-release-gate --self-test`;
  `sarah_livekit_release_rows_require_bound_receipts_and_observed_facts` in
  `crates/omega_deltas`; and the candidate assembler's digest, binding, row,
  status, size, and public-safe checks.

### OMEGA-DELTA-0218 — A hosted start-up window is ridden out, not shown as a refusal

Every new Cloud Run revision of the hosted OpenAgents service takes traffic
before its Cloud SQL Auth Proxy sidecar has connected. For 10-70 seconds after
each deploy, `POST /api/omega/auth/session` answers 503 with a typed
`omega_nostr_auth_storage_*` code, and it flaps: the same identity gets 200,
then 503, then 200 within seconds. A single re-attempt a few seconds later
lands inside that window often enough that the owner saw a red banner for a
dependency that was about to answer.

**The transient class is named from the server's own code, not guessed from a
status.** `HostedSessionBlocker::ServiceStorageUnavailable` carries the status
and the bounded public error code. Any code under the
`omega_nostr_auth_storage` prefix admits it, and so does a bare 503, because
the owner's binary talks to whichever revision is live and an older build sends
only the first of the sibling codes.

**Only that class is re-attempted inside one sign-in.**
`is_transient_service_window` is deliberately narrower than `is_retryable`:
401/403, `RequestFramingRejected`, every identity and custody blocker, a
consumed proof, a challenge rate limit, and a local credential-storage failure
all return on the first answer, so no owner waits through a backoff for a
verdict that cannot change.

**The backoff is bounded on both axes.** Five attempts with delays of 1s, 2s,
4s, and 8s plus additive jitter, and a 30-second wall-clock budget measured
from the first attempt. The budget counts the failed requests themselves — a
cold instance burns about 30 seconds server-side before its 503 arrives — so
the worst case is the budget plus one in-flight request rather than five
serial timeouts. The schedule is an injectable value, so a test or a
deterministic harness runs it with no sleeping at all.

**A persistent outage is still honest.** When every attempt fails, the caller
gets the real blocker from the last attempt, the projection stays `retryable`,
and the owner sees the real message. Each re-attempt is logged with its number,
its wait, and the blocker line, which carries a status and a public error code
and never a token, key, or signature.

- **Enforced by:** `the_hosted_start_up_window_is_ridden_out_not_reported`
  in `crates/omega_deltas`;
  `the_hosted_backoff_doubles_and_stops_on_both_attempts_and_wall_clock`,
  `injected_schedules_never_sleep`, and
  `jitter_only_lengthens_a_delay_and_stays_bounded` in
  `crates/omega_effectd/src/openagents_session.rs`; and
  `the_hosted_storage_start_up_window_is_recognized_from_the_server_code` and
  `only_the_service_start_up_class_is_re_attempted_inside_one_sign_in` in
  `crates/omega_effectd/src/openagents_nostr_auth.rs`.

### OMEGA-DELTA-0219 — The gate report keeps the prose a person wrote in it

**The writer rebuilt the whole file.** `write_report` in
`script/omega-release-gate` composed the report from the receipt alone and
wrote it over the old one. Everything a person had typed into
`docs/omega/release-gate.md` — the nostr-authentication scope note, the
owner-evidence and Sarah LiveKit evidence instructions, the assembler command,
the whole cutover plan, the standing review laws `OMEGA-DELTA-0189` asserts
are still there, and the handoff protocol — was outside the receipt, so the
next regeneration deleted it. Nothing failed, nothing warned, and the loss was
visible only to whoever remembered writing it. A harness whose entire purpose
is that "nothing was found" and "nobody looked" must never read the same was
itself erasing observations.

**Authored prose has a source of its own.** Each authored region is a sibling
file named for the report and its section: `docs/omega/release-gate-overview.md`
above the candidate facts, `docs/omega/release-gate-evidence.md` between the
regeneration command and the row table, and
`docs/omega/release-gate-operator-notes.md` after it. `write_report` splices
each one in verbatim — headings, code fences, and table pipes survive
untouched — so the committed report stays exactly the generated facts plus its
authored sources, and the generated text now names those sources so the next
operator writes in the right place.

**A section that cannot be read refuses the report.** `load_authored_sections`
runs before the gate touches anything and raises on a missing, unreadable, or
blank source; `main` prints the path and exits 2 without rewriting the report.
Replacing silent loss with a silent empty section would have been the same
defect one step later.

- **Enforced by:** `authored_release_gate_prose_survives_regeneration` in
  `crates/omega_deltas`, which holds the committed report and its authored
  sources in agreement and runs the harness self-test; and
  `self_test_authored_report_sections` in `script/omega-release-gate`, which
  regenerates a scratch report twice, compares the authored bytes, and proves
  a missing and an empty section are both refused.

### OMEGA-DELTA-0220 — The selected media path is evidence, not a checkbox

**The packaged client records what WebRTC selected.** After private Sarah
publishes its microphone, after it subscribes to Sarah audio, and after a media
reconnect, Omega reads LiveKit's publisher and subscriber statistics. It joins
each transport's `selectedCandidatePairId` to the exact candidate pair and its
local and remote candidates, then records candidate type, protocol, relay
protocol, and packet counts. From those fields it classifies direct UDP,
non-relayed TCP fallback, and TURN over TLS. A different relay shape remains
`unclassified`; it is never promoted because a firewall mode was requested.

**The receipt is useful without becoming a network trace.** Each
`openagents.omega.sarah-livekit-transport-evidence.v1` JSONL row is written
under the active Omega profile's `voice` directory. Session, room, dispatch,
and provider-generation references are domain-separated SHA-256 digests. IP
addresses, ports, candidate URLs, TURN credentials, participant grants, media,
and transcripts are not serialized. Publisher and subscriber paths remain
separate so an operator cannot silently substitute one direction for both.

**A reconnect compares the generation it claims to preserve.** The
authenticated gateway's first `session_ready` now requires an opaque
`providerGenerationRef`. OpenAgents owns that reference: it must remain stable
across an allowed media reconnect, be distinct for concurrently active
generations, and never name a newly opened or revived provider generation.
`dispatchRef`, room epoch, and Omega's client generation are not substitutes.
Omega rejects a repeated readiness frame, overlapping reconnect notifications,
and a `Reconnected` notification without a preceding `Reconnecting`. The
release gate requires one connected and one reconnected observation whose
session, room, dispatch, and provider-generation digests all match.

**One Sarah track still owns playback.** A second simultaneous Sarah audio
track fails the media session instead of spawning another playback stream. An
unsubscribe for a track that does not own playback is stale and also fails.
This makes the no-duplicate-audio claim a runtime invariant rather than an
assumption about what LiveKit usually publishes.

**Booleans cannot pass the transport rows anymore.** A passing private or
connectivity row must include the packaged JSON objects in
`facts.transport_observations`. The gate recomputes every path classification
from its candidates and requires all three connectivity classes. Setting
`reconnect_same_generation`, `direct_udp_completed`, `tcp_fallback_completed`,
or `turn_tls_completed` without the measured objects is refused.

- **Enforced by:**
  `sarah_livekit_transport_evidence_is_measured_and_generation_bound` in
  `crates/omega_deltas`; the focused workroom tests
  `selected_ice_pairs_classify_direct_tcp_and_turn_tls_without_addresses`,
  `livekit_audio_owner_refuses_overlapping_and_stale_tracks`,
  `livekit_reconnect_fence_refuses_overlap_and_revival`,
  and `livekit_transport_receipt_hashes_generation_and_room_identity`; and
  `script/omega-release-gate --self-test`.
### OMEGA-DELTA-0221 — Community Sarah controls have one governed dispatch surface

The tester-channel room controls were working buttons but not durable product
actions. Join, Leave, Mute, Summon Sarah, Remove Sarah, Talk to Sarah, and
moderator Stop lived only inside `on_click` closures. A pointer could reach
them, but a keymap, accessibility driver, or deterministic packaged-client
operator could not name the same operation. That made the live three-desktop
journey depend on somebody clicking five controls in three windows.

**Every room operation is a registered action.** The seven operations live in
the `community_sarah` action namespace. The selected public channel installs
all seven handlers beside its `PublicChannelSarahRoom` key context. Buttons
dispatch the actions instead of mutating the model directly, so pointer,
keyboard, and direct action dispatch cannot acquire separate authority logic.

**Admission stays narrow.** Zero base admits the seven full action names, not
the namespace. A community-room control still passes through the process-wide
action gate while unrelated community or workspace capabilities remain
closed.

**The default keymaps can drive the complete row.** A context-scoped `Cmd-K`
chord on macOS and `Ctrl-K` chord on Linux/Windows binds J join, L leave, M
mute, S summon, R remove, T talk, and X moderator stop. The context exists only
on the selected tester-channel view, so those mnemonics do not replace editor
bindings elsewhere.

**Source readiness is not installed evidence.** rc29 predates these actions,
and the live three-desktop journey has not been observed on a later package.
The release row remains blocked until one exact packaged candidate preserves
the room, floor, audio, moderator, identity, and refusal facts.

- **Enforced by:**
  `community_sarah_actions_drive_pointer_keyboard_and_direct_dispatch` in
  `agent_ui`; `community_sarah_default_keybindings_resolve_at_startup` in
  `omega`; `community_room_controls_have_one_governed_dispatch_surface` in
  `omega_deltas`; and the exact-action assertions in `omega_zero_base`.

### OMEGA-DELTA-0222 — Packaged Sarah journeys share one sealed candidate run

The Sarah release rows previously checked structured row receipts but did not
bind the private, three-desktop, and forced-transport observations to the same
installed processes and isolated profiles. Three pass booleans and copied ICE
objects could therefore describe different candidates or an unconstrained
network path while retaining one manifest binding.

**One private run plan binds the package before launch.** The candidate runner
recomputes the DMG digest, checks the release record and complete Sarah
infrastructure binding, and verifies the exact installed Omega binary digest
and code signature. It creates three private homes and data roots and launches
the installed binary with three distinct `--user-data-dir` arguments. Capture
refuses a dead, replaced, or differently hashed process.

**Transport evidence comes from those profile roots.** The runner reads each
profile's bounded-field transport JSONL. A private pass needs connected and
reconnected rows with the same session, room, dispatch, and provider-generation
binding. Connectivity needs three distinct sessions whose selected ICE fields
exclusively classify as direct UDP, TCP fallback, and TURN/TLS under the three
declared constraints. Each cell also consumes the corresponding OpenAgents
acceptance receipt, recomputes its result digest, and checks its deployed
worker revision and forced-transport declaration. Unexpected transport fields
are private material and are rejected.

**The release gate reopens the capture.** Passing private, room, and
connectivity receipts carry a content-addressed `candidate_capture_ref`. The
gate verifies the collector revision, full binding, three authenticated
packaged profile references, row facts, and measured transport objects. A
hand-set pass value or a capture from another candidate cannot promote a row.

- **Enforced by:**
  `script/omega-sarah-livekit-candidate-run self-test`,
  `script/omega-release-gate --self-test`, and
  `sarah_livekit_candidate_run_binds_profiles_and_forced_transports` in
  `omega_deltas`.

### OMEGA-DELTA-0233 — The component library returns as a gated development surface

OMEGA-DELTA-0022 and OMEGA-DELTA-0186 deleted the inherited `component_preview`
crate and its `workspace::OpenComponentPreview` action after that ungated dev
surface shipped in a release command palette and rendered unreviewed artwork.
The removal left `crates/component`'s registry populated at startup with no
reader, and omega#247 needs an in-app gallery to review the market viz
primitives against the Bazaar Storybook.

This delta admits a successor with the original failure's two halves fixed
rather than reverted. New names throughout: the crate is
`crates/component_library` and the action is
`omega_workbench::OpenComponentLibrary` (an already-admitted namespace in
`omega_zero_base`); the removed names stay mechanically blocked. The surface is
doubly gated: at runtime it requires `debug_assertions` plus
`OMEGA_COMPONENT_LIBRARY=1` (otherwise the action is hidden from the palette
and no handler is registered), and at compile time the screen module is
omitted from release builds entirely, following the omega#220 rule that a
runtime gate alone does not keep a development payload out of a shipped
binary. On screen the surface labels itself a development surface, satisfying
PRODUCT.md's requirement that development-gated destinations identify their
state as non-production.

IDs 0223–0232 are referenced by in-flight checks that have not registered
registry entries; this entry takes 0233 to avoid colliding with them.

- **Enforced by:**
  `component_library_returns_only_as_a_gated_development_surface` in
  `omega_deltas`, and the gate unit tests in `crates/component_library`.

### OMEGA-DELTA-0234 — The palette advertises only the drawn surface

The zero-base gate admits whole namespaces (`agent`, `editor`,
`omega_workbench`), which is coarser than the sealed interface it protects.
An audit of the command palette found the mismatch runs in both directions.

**Outward: descoped surfaces stayed advertised.** The palette listed actions
whose targets this build does not draw: the deleted diagnostics, debugger, and
task crates (`editor::ToggleDiagnostics`, breakpoints, `SpawnNearestTask`),
center-pane splits and multibuffers the sealed shell never shows
(`editor::OpenExcerptsSplit`, `OpenSelectionsInMultibuffer`,
`OpenProposedChangesEditor`), center-pane duplicates of workbench surfaces
(`agent::Follow`, `agent::ChatWithFollow`, `agent::OpenAgentDiff`), and
`omega_workbench::SelectForensics`, whose menu row and keybinding the owner
withdrew on 2026-08-04 — the palette was its last advertised entry point.
These are now hidden from the palette by action type. `admits_action` is
unchanged, so menus, keymaps, and every existing delta contract keep passing;
`hide_action_types` composes with the restriction rather than replacing it.
`terminal::RerunTask` is removed from the admitted set outright: its handler
dispatches `task::Rerun`, which the gate already refuses, over task
infrastructure that was deleted. The duplicated
`workroom::PrepareVoiceAdmission` entry is collapsed to one.

**Inward: drawn controls were refused.** The Markets dock panel
(omega#244, loaded only behind `OMEGA_MARKET_PANEL=1`) had no reachable
entry point at all: the sealed interface renders no status-bar panel
buttons, so `market::ToggleFocus`/`market::Reconnect` are admitted as the
panel's palette entry point.

 The Workbench Search surface and the
thread search bar draw controls that dispatch `search::FocusSearch`,
`ToggleRegex`, `ToggleCaseSensitive`, `ToggleWholeWord`, `ToggleReplace`,
`ReplaceNext`, `ReplaceAll`, and `ToggleIncludeIgnored` — none of which were
admitted, so clicking them logged refusals in the real binary while passing
the visual lane (which installs no gate). The full drawn set is now admitted.

- **Enforced by:**
  `the_palette_matches_the_drawn_surface_in_both_directions` in
  `omega_deltas`, and `the_admitted_set_is_the_exo_surface_and_nothing_else`
  in `omega_zero_base`.

### OMEGA-DELTA-0241 — LN Markets credentials stay local to a direct v3 client

Omega connects to LN Markets without sending the account credential through
the OpenAgents API. The new Rust client signs the exact v3 method, path, query,
and compact JSON bytes that it sends. It rate-limits requests and retries only
read-only requests after connection-phase failures or transient HTTP statuses.
It never retries a 401 or a swap POST; a retry must not duplicate a mainnet
conversion whose response was lost.

The API Keys screen stores the key, secret, passphrase, and selected network in
Omega's private channel-scoped credential store. Save & Test calls the signed
`/v3/account` route before retaining a new credential. The credential types
redact their debug output and zero their owned strings when dropped.

Omega Agent can read the configured account, read public ticker and synthetic
USD prices, and request BTC/synthetic-USD swaps on signet or mainnet. Every
swap call names its network, and the tool refuses a request whose network does
not match the configured credential before it sends anything. A matching
mainnet request executes against LN Markets with real account funds.

The LN Markets capability ships as five plugin-shaped crates. The pure
`lnmarkets_client` crate has no dependency on an Omega crate. Data, strategy,
and UI code have their own crates, and the `lnmarkets` umbrella owns the
manifest, endpoint declarations, transport adapter, and one app registration
entry point. Omega's default-on `lnmarkets` feature enables the agent and
settings surfaces together. The feature-off build removes both surfaces and is
part of `script/omega-checks`.

The direct client covers the isolated-futures mutation surface. New market and
limit trades encode margin or quantity as an exclusive typed choice, leverage
is limited to 1 through 100, and trade IDs and positive amounts are validated
before a request is built. Close, cancel, cancel-all, add-margin, cash-in,
stoploss, and takeprofit operations name the account network and fail before
transport if it differs from the configured credential. Trade responses decode
the venue's four state booleans into one checked state enum. POST, PUT, and
DELETE signature shapes are pinned, and mutations are single-attempt.

The cross-margin mutation surface follows the same boundary. Market and limit
orders use positive integer USD quantities; limit prices use positive 0.5
ticks. Order cancellation, cancel-all, position close, leverage changes, and
wallet-to-margin transfers all require the caller to name the configured
network. The client validates order IDs, leverage, prices, quantities, and
transfer amounts before transport. Cross-margin POST and PUT signatures are
pinned, and every mutation is single-attempt.

The account mutation surface can create Lightning deposit invoices, pay
Lightning invoices, withdraw Bitcoin on-chain, create P2TR or P2WPKH deposit
addresses, and mark notifications read. Positive amounts and lowercase
64-character description hashes are validated before transport. Every call
names the credential network, and every POST or PUT is single-attempt.
Invoice and Bitcoin address values have redacted debug output while explicit
accessors preserve the values needed by account cards.

- **Enforced by:** signer, retry, request-body, and credential tests in
  `lnmarkets_client`; `ln_markets_uses_one_local_direct_fail_closed_client` in
  `omega_deltas`; the feature-off checks in `script/omega-checks`; and the
  endpoint declarations in `app_identity`.

### OMEGA-DELTA-0244 — Trading profit has one venue-neutral ledger

The platform owns a durable sats ledger independently of any venue plugin.
Every financial event has balanced postings and a strategy attribution.
Entries are sequenced, hash-chained, and protected from updates and deletes by
SQLite triggers. Reads and writes verify the complete chain and stop on a gap
or altered record.

Venue balance snapshots check the ledger without changing it. A mismatch
appends an attributed reconciliation alert with no postings, so observed venue
state cannot silently rewrite profit. The read API reports profit, fees,
funding, and worst drawdown per strategy and period. The default database lives
beside the thread store and survives application restarts.

- **Enforced by:** `trading_ledger` unit tests and
  `trading_ledger_is_venue_neutral_append_only_and_reconciled` in
  `omega_deltas`.

### OMEGA-DELTA-0245 — Trading authority is one approved mandate

The platform owns a typed, venue-neutral trading mandate. It names the network,
objective, venue-balance cap, position cap, leverage cap, daily loss stop,
allowed strategies, review cadence, and expiry. No mandate and an expired
mandate both require a flat-risk posture. Every proposed instruction is checked
against every limit regardless of whether it came from chat, a scheduled agent
turn, or strategy code.

Mandate history is append-only and durable beside the thread store. Creating or
widening authority requires an explicit settings prompt whose acceptance is
bound to the displayed candidate and base revision. Restriction and revocation
take effect immediately. The store exposes no unclassified mutation method,
and production code has one call site for the widening door: the settings UI.

- **Enforced by:** `trading_mandate` unit tests and
  `trading_mandate_has_one_ui_approved_widening_door` in `omega_deltas`.

### OMEGA-DELTA-0246 — Agent turns can start from bounded wakeups

Omega Agent accepts a versioned, venue-neutral wakeup envelope in addition to
user prompts. A wakeup names its thread, typed schedule or event source,
instruction, timestamp, and token reservation. The in-process scheduler and
future cloud durable-turn lane use the same serializable contract. Every
wakeup is written into the conversation as a labeled message before the model
runs, so the transcript records why the turn exists.

Each open native session owns one scheduler task. Settings keep wakeups off by
default and cap the review interval, polling frequency, turns per rolling hour,
tokens per turn, and tokens per rolling hour. The governor rejects a wakeup
while another turn is running and reserves the budget before starting the
turn, which makes a runaway loop impossible even if an event source repeats.
Background services publish typed funding, drawdown, liquidation-distance,
volatility, strategy-halt, deposit, or withdrawal events through the same
entry point.

- **Enforced by:** `agent_wakeup` unit tests and
  `native_agent_wakeups_are_labeled_typed_and_bounded` in `omega_deltas`.

### OMEGA-DELTA-0247 — Strategies execute deterministically within the mandate

Omega has one venue-neutral strategy engine. Strategy programs are pure,
typed functions from configuration, prior state, and a feature tick to next
state and order intents. The model remains outside this loop. A background
service processes start, adjust, tick, and halt commands in order and publishes
one typed lifecycle stream for the future agent tool card.

Before one single-attempt venue mutation, the engine validates the intent,
previews its resulting risk, requires a venue-side stop for leveraged risk
increases, derives recent order count and daily loss from the platform ledger,
and asks the active mandate to enforce venue balance, position, leverage, loss,
order-frequency, and liquidation-buffer limits. Any program, mandate, venue,
protection, or ledger failure halts the strategy and publishes a typed agent
wakeup. Every admitted order and each returned fill, fee, and funding event is
written to the venue-neutral ledger with strategy attribution.

- **Enforced by:** `strategy_engine` and `trading_mandate` unit tests and
  `strategy_execution_is_deterministic_bounded_and_single_attempt` in
  `omega_deltas`.

### OMEGA-DELTA-0248 — Collected evidence gates every live strategy configuration

A strategy cannot start or adopt a changed configuration until the platform
finds a passing backtest artifact for that exact strategy ID, strategy
version, network, and serialized parameter set. The artifact is append-only,
content-addressed, and records its collected-data range, policy, trade count,
expectancy after measured costs, maximum drawdown, and cost-measurement
provenance. The newest matching artifact controls the gate, including when it
records a failure after an earlier pass.

Backtests run the same model-free `StrategyProgram::on_tick` path used by the
live engine. LN Markets replay input comes from stored hourly candles and
funding settlements and invokes the existing feature derivation; either
missing history makes the replay fail closed. The cost model applies taker
fees, observed round-trip cost, and the recorded funding series. A bounded
history query lets later review turns and operator surfaces inspect the
reports without weakening the execution gate.

- **Enforced by:** `strategy_engine` and `lnmarkets_data` unit tests and
  `collected_backtests_gate_exact_live_strategy_parameters` in `omega_deltas`.

### OMEGA-DELTA-0249 — Target rebalancing is measured, ladder-aware, and signet-only

The first LN Markets strategy holds a configured share of account value in
synthetic USD. Its model-free program emits an order only when account drift
exceeds a configured threshold and the correction value exceeds the measured
round-trip cost plus a configured margin. Order value is capped by the
strategy limit and, when bucket data exists, by the configured share of the
relevant bid or ask depth.

Round-trip cost is configuration evidence with sample count, traded notional,
timestamp, and source. A ledger reader remeasures it from attributed swap
fills; no observed cost is embedded as a program constant. Each execution
uses the direct synthetic-USD swap route once, records the fill and measured
cost in the platform ledger, and exposes target, drift, last action, realized
cost, and hurdle through a versioned state update for the later streaming
card.

The program configuration and executor both refuse mainnet. The generic
backtest gate still binds the exact configuration before the engine starts,
so signet is not a bypass around collected evidence or the active mandate.

- **Enforced by:** `lnmarkets_trading` and `strategy_engine` unit tests and
  `target_rebalancing_is_measured_ladder_aware_and_signet_only` in
  `omega_deltas`.

### OMEGA-DELTA-0250 — Funding carry is cost-gated, protected, and attributed

The funding-carry strategy opens a delta-neutral short only when positive
funding over the configured holding horizon exceeds measured round-trip cost
plus margin. It sizes from funding magnitude within configured notional and
leverage limits, and uses separate entry and exit thresholds so noise near
zero does not churn the position. An exact-configuration backtest over
collected funding history must pass before the strategy can run.

The strategy can hold synthetic USD or an isolated short future. The direct
future is isolated because the LN Markets v3 cross-future surface does not
provide venue-side stop, add-margin, or cash-in operations. An isolated short
opens with its stop in the same single-attempt request. Private position data
then drives missing-stop repair, margin top-ups as liquidation distance
shrinks, profit sweeps, and sign-flip exits. Funding settlements are imported
idempotently into the double-entry ledger with strategy attribution.

The program, executor, and funding synchronizer refuse mainnet. The strategy
state exposes its funding signal, cost hurdle, instrument, protection, margin,
settled funding, and most recent action through a versioned projection.

- **Enforced by:** `lnmarkets_trading` and `strategy_engine` unit tests and
  `funding_carry_is_cost_gated_protected_and_attributed` in `omega_deltas`.

### OMEGA-DELTA-0251 — LN Markets agent controls are typed and card-backed

Omega Agent can read the local derived feature snapshot, inspect attributed
strategy profit, read the active trading mandate, and control the two bounded
LN Markets strategies. Every output has a versioned `omega.lnmarkets.*`
schema. The mandate tool is read-only; widening authority remains exclusive to
the settings approval flow. Strategy start and adjustment still require the
exact stored backtest and active mandate, and automated execution remains
restricted to signet.

Strategy commands receive an acknowledgement from the durable background
service. A refused command returns its reason without killing that service.
Start, adjust, halt, and status updates stream through one tool call so the
transcript updates one lifecycle card. Features, ledger profit and drawdown,
strategy lifecycle states, and mandate states each have component-library
coverage and render outside generic collapsed tool groups.

The four tools are part of the closed Basic profile alongside the account,
market-data, and direct-swap tools. Raw LN Markets tools remain available for
inspection and one-off actions.

- **Enforced by:** `strategy_engine`, `agent`, and `agent_ui` unit tests and
  `lnmarkets_agent_tools_are_versioned_bounded_and_visible` in `omega_deltas`.

### OMEGA-DELTA-0252 — Portfolio reviews stay local, bounded, and mandate-governed

Omega Agent can run scheduled or event-triggered reviews of the LN Markets
portfolio. Each review uses only local collector features, strategy lifecycle
state, ledger history, and the active mandate. The turn cannot fetch remote
market or account data, place a raw order, or execute a swap. It can issue at
most one bounded strategy command through the same backtest and mandate gates
used by a user-started turn.

One claimed thread owns the review schedule. Every review uses a 1,024-token
ceiling and the shared wakeup governor. Event wakeups remain pending until the
matching completed turn acknowledges them, so a failed turn can be retried
without losing its trigger.

- **Enforced by:** `lnmarkets`, `strategy_engine`, and `agent` unit tests and
  `lnmarkets_portfolio_reviews_are_local_bounded_and_mandate_governed` in
  `omega_deltas`.

### OMEGA-DELTA-0253 — The trading operator console projects local authority and state

The LN Markets plugin provides a right-dock operator panel. It projects
collector connection state, every subscribed topic's lag, backfill progress,
strategy state and limit headroom, attributed profit and costs, the active
mandate, pending wakeups, and recent review outcomes from local stores and
in-process services. The transcript remains the narrative agent surface.

The panel can narrow mandate limits by half or revoke the mandate immediately.
Neither action can create or widen authority, so neither uses the approval
door. Automatic refresh reads local state on a background executor. The panel
is loaded only with the LN Markets plugin feature.

- **Enforced by:** `lnmarkets_ui`, `lnmarkets`, and `lnmarkets_data` unit and
  GPUI paint tests and `lnmarkets_operator_panel_is_local_complete_and_tested`
  in `omega_deltas`.

### OMEGA-DELTA-0254 — Signet soak receipts prove bounded zero-nudge operation

The LN Markets acceptance recorder issues a passing receipt only when its
evidence names an approved signet mandate and one commit, covers a measured
window with no human messages, and shows every mandated strategy running.
Scheduled reviews must carry their typed transcript labels, reasoning-note
presence, strategy-card updates, and measured token use within both per-turn
and rolling-hour budgets.

The same receipt records deliberately injected mandate-limit refusals. Each
injection must halt its strategy and produce the matching typed strategy-halt
wakeup. Its ledger summary covers the exact window, and balance samples at
both boundaries and throughout the window must match the venue exactly.
Receipts reject unknown fields and invalid evidence, contain no credentials,
and use create-new storage so a prior acceptance record cannot be replaced.

- **Enforced by:** `lnmarkets` unit tests and
  `lnmarkets_signet_soak_receipts_fail_closed_over_complete_evidence` in
  `omega_deltas`.

### OMEGA-DELTA-0255 — Provider inventory hedging runs outside every custody boundary

The LN Markets provider hedger is a standalone program with no dependency on
Omega, Immortal, or the OpenAgents API. It reads an operator-owned Signet
credential from a mounted secret file and sends requests directly to LN
Markets. It refuses Mainnet before it creates a venue client. Its output
contains cycle state and ledger results without credential material.

Each cycle reads the configured provider inventory target, keeps one selected
cross-margin or synthetic-USD hedge, and sends at most one venue mutation.
Low liquidation distance causes a bounded margin top-up before an exposure
change. The direct client keeps every mutation single-attempt.

Fills, fees, funding settlements, and venue-balance residuals enter the shared
append-only sats ledger with provider-hedger attribution. A measured evaluation
passes only when hedged profit variance is below unhedged variance and funding
carry covers all recorded fees.

- **Enforced by:** `lnmarkets_hedger` unit tests and
  `lnmarkets_provider_hedger_is_standalone_signet_and_ledger_backed` in
  `omega_deltas`.

### OMEGA-DELTA-0256 — Threshold swing is bounded, feature-driven, and signet-only

The threshold-swing strategy uses the collected LN Markets index, volatility,
spread, account, and ladder features. It enters after a configured index move
measured in volatility units and exits after the symmetric move. Both entry
and exit must clear the measured round-trip cost plus margin. Position value,
spread, and liquidity utilization have explicit configuration caps.

Collected oracle-index history is part of deterministic replay. A passing
backtest must show positive expectancy after costs for the exact configuration
before the strategy can start. The program and executor refuse mainnet, and
the active mandate remains the authority for position size and execution.

The strategy is available through the same start, adjust, halt, lifecycle,
and operator surfaces as the other bounded strategies. Its executions carry
`threshold_swing` ledger attribution.

- **Enforced by:** `lnmarkets_data`, `lnmarkets_trading`, `lnmarkets`, and
  `agent` unit tests and
  `threshold_swing_is_feature_driven_bounded_and_runtime_integrated` in
  `omega_deltas`.
