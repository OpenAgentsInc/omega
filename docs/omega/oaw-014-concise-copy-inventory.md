# OAW-014 concise copy inventory

Omega distinguishes presentation chrome from domain records. Chrome tells a
person what a control is or shows a compact state. Domain records preserve the
exact blocker, limitation, evidence, authority, next action, and receipt facts
needed to understand consequential Work.

## Enforced presentation contract

`omega_status_cue` is the shared Omega-owned status primitive. It carries the
same state on four independent channels:

- its own icon shape, one per status, so a person who cannot separate the
  theme's success, warning, and error hues still reads seven distinct states;
- a semantic color;
- an admitted one-word tooltip under `Role::Status`; and
- an accessible name containing the exact context and status.

The admitted words are Ready, Running, Complete, Blocked, Warning, Failed, and
Offline. `omega_control_crawl::lint_status_words` rejects multi-word status
prose and unregistered labels, and the cue's own test proves every word it can
emit is admitted by that lint. `color_is_never_the_only_cue` proves the seven
shapes are distinct.

## Enforcement, not disposition

This inventory used to be a written disposition that nothing checked. It is now
mechanical.

`omega_control_crawl::omega_owned_ui_sources` discovers the Omega-owned UI
sources by location — every `crates/omega_*/src/**/*.rs`, plus the `omega_*`,
shell, Forensics, Effective Principal, and Organization-scope files in
`agent_ui` — so a new Omega destination enters the inventory the moment its file
exists rather than when someone remembers to list it. The shell file
`crates/agent_ui/src/agent_panel.rs` is included because it renders the Omega
sidebar, navigation rows, and footer. Only the lint's own source is excluded,
because it quotes the call markers it searches for and renders no chrome.

`scan_presentation_copy` extracts user-facing string literals from three slots:
`.aria_label(...)` accessible names, `Tooltip::text(...)`, and `.child("...")`
visible labels. A trailing `#[cfg(test)]` module is skipped because test
fixtures are not shipped chrome; an item-level `#[cfg(test)]` gate does not stop
the scan. `lint_presentation_copy` then refuses any string that narrates more
than one sentence, or that exceeds its slot's limit: 80 characters for an
accessible name and 48 for a tooltip or visible label. The accessible name has
the most generous limit on purpose — conciseness must never truncate assistive
meaning. The one-word rule is the status channel's, not every tooltip's, and
`lint_status_words` plus the cue's own tests enforce it there.

`omega_owned_presentation_copy_is_concise` runs the whole scan over the real
repository and must find nothing. The checked-in allowlist remains empty;
deleting a string is preferred over allowlisting it.

Two boundaries stay explicit. The scan sees literals, so a `format!` string, a
source-owned domain record, or a runtime-composed accessible name is out of its
reach and is governed by its own surface's tests. And a lint can prove a string
is short; it cannot prove the short string is still true.

## Current inventory disposition

- Thread lifecycle already uses the icon/color/one-word-tooltip contract.
- Effective Principal uses icon, color, concise visible identity/scope/signer
  facts, and one exact accessibility label. These are distinct authority facts,
  not a decorative lifecycle chip.
- Work Index and Work detail keep source-owned titles, descriptions, blockers,
  evidence, receipts, and lifecycle facts. Those records are not tooltips and
  must not be deleted by a presentation lint.
- The fixture Work list, board, table, timeline, roadmap, and Issue detail use
  `omega_status_cue` for completion, blocker, and readiness presentation.
  Blocker counts and source lifecycle values remain visible domain facts.
- Fixture provenance uses the shared cue and a compact source/gap/issue
  summary. Exact projection loss and authority limits remain in detail and
  inspector records.
- Signed Workroom delivery uses the shared cue for outbox state. Exact relay
  counts and accepted, rejected, or unreachable attempt records remain visible
  transport facts. Raw relay error detail remains in the ledger but is not
  rendered into the accessibility tree.
- Empty execution and signed-history chrome now state the minimum facts:
  assignment/session absence, mock authority, or signed transport scope.
  Exact command, claim, lease, evidence, verification, receipt, release, and
  owner-authority limits remain available in Work detail.
- Forensics publication and promotion headers now use `omega_status_cue`.
  The prominent `PRIVATE · PUBLICATION BLOCKED` sentence is removed.
- Forensics detail continues to show why publication authority is unavailable.
  That is the domain record that makes the blocked icon precise, not product
  exposition to hide.
- Lifecycle-scene names such as `Awaiting profile` are fixture selector labels,
  not the rendered status channel. They remain in development/mock navigation
  and are absent with the fixture gate off.
- Work detail Block cards use the shared cue for source availability instead of
  the `Source-backed`/`Unavailable` word pair, and the narrated two-sentence
  authority note is now the concise `View only — grants no authority`. The exact
  source reference and the Block's facts remain visible domain records.
- The Forensics preflight empty state still says which admitted managed profile
  is missing. That is a precise limitation, not chrome, and the lint leaves
  single-fragment limitation text alone.

## Remaining installed gate

The automated rule now runs over the current inventory and passes. What it
cannot do is confirm that the retained meaning is still legible: no installed
light/dark, reduced-motion, localization-expansion, or assistive-technology pass
has been run against these surfaces, and no accessibility-tree assertion exists
for the migrated cues beyond their construction. omega#219 remains open for that
installed visual and accessibility review.
