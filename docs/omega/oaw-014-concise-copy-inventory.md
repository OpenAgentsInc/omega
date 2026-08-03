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

`scan_presentation_copy` extracts user-facing string literals through the
`COPY_MARKERS` table, which names every call this repository writes copy into
and the argument that carries it. The first version of this scan read three
markers — `.aria_label(`, `Tooltip::text(`, and `.child(` — and an installed
review found that this left the dominant visible-text constructor entirely
unscanned: `Label::new("…")` accounts for 113 of the literal strings in
Omega-owned UI against 47 in `forensics_workbench.rs` alone, and none of them
were ever read. The gate was green because it was looking at the wrong slot.

The table now covers `.aria_label(` (accessible name); `Tooltip::text(` and
`Tooltip::with_meta(` (tooltip); `.child(`, `Button::new(` argument 1,
`.action(`, `.entry(`, `.header(`, `.title(`, `.description(`, and
`.placeholder(` (visible label); and `Label::new(` (visible text). A marker
declares which argument is user-facing, so `Button::new("id", "Label")` yields
the label and never the element id. A trailing `#[cfg(test)]` module is skipped
because test fixtures are not shipped chrome; an item-level `#[cfg(test)]` gate
does not stop the scan. Wrapped literals decode through Rust's `\`-newline line
continuation, so an offense and an allowlist entry compare equal.

`lint_presentation_copy` refuses any string that narrates more than one
sentence, or that exceeds its slot's limit: 80 characters for an accessible
name, 48 for a tooltip or visible label, and 96 for general visible text. The
accessible name is more generous than a visible label on purpose — conciseness
must never truncate assistive meaning. `Label` gets the loosest ceiling because
the same constructor carries button-sized labels, section subtitles, and
one-line empty-state records; the defect this contract exists to catch there is
narration, not width. With the current tree repaired, no single-sentence literal
is near that ceiling, so the sentence rule is the binding constraint and the
character limit is a backstop. The one-word rule is the status channel's, not
every tooltip's, and `lint_status_words` plus the cue's own tests enforce it
there.

A sentence boundary requires whitespace after the terminator. Without that,
`AGENTS.md`, `v0.2.0`, and `openagents.com` would each be read as two sentences
and real menu labels would be pushed onto the allowlist.

Three tests keep the extension honest.
`label_narration_is_invisible_to_the_previous_marker_set` is the falsifier: it
takes the exact narration the installed review photographed in the primary
Forensics layout, proves the previous three-marker set neither sees nor refuses
it, and proves the current table refuses it as visible-text narration. It fails
in both mutation directions — with `Label::new` removed from `COPY_MARKERS`, and
with `Label::new` added to `LEGACY_COPY_MARKERS`.
`every_marker_extracts_its_user_facing_argument` synthesises a call for every
marker in the table, so a marker cannot be added to widen the contract's claimed
coverage without actually extracting anything.
`leading_identifier_arguments_are_not_read_as_copy` pins the id/label split.

`omega_owned_presentation_copy_is_concise` runs the whole scan over the real
repository and must find nothing.

Three boundaries stay explicit. The scan sees literals, so a `format!` string, a
source-owned domain record, or a runtime-composed accessible name is out of its
reach and is governed by its own surface's tests. The scan sees calls, so copy
routed through a helper the table does not name is invisible until the marker is
added. And a lint can prove a string is short; it cannot prove the short string
is still true.

## Allowlist

Deleting a string is still preferred over allowlisting it. The allowlist admits
one reason: a **consequence disclosure** — copy a person must read in full
before an irreversible or privacy-bearing action, where shortening it would
remove a stated consequence rather than remove narration. The five current
entries are the raw identity-key export warnings, the voice-principal authority
boundary, and the public-channel privacy notice. Each entry names its exact text
and its reason. None of them is Forensics or Work chrome.

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
- Lifecycle-scene names such as `Awaiting profile` were previously recorded here
  as fixture selector labels only. That was wrong: the installed review found
  `Awaiting profile` rendered twice as a right-aligned textual status badge in
  the primary Lifecycle layout, on the `Preflight and lifecycle` header and on
  the selected-stage header. Both badges now use `omega_status_cue`, with the
  exact scene name as the cue's accessible context — the cue reads as
  `Awaiting profile: Blocked` rather than losing which lifecycle state is meant.
  The labelled `State` record still carries the exact scene name as visible
  text, and it is now shown for every lifecycle selection rather than only the
  summary, so removing the badge removes narration and no fact.
- Forensics narration repaired by the visible-text extension: the model-matrix
  comparison note, the publication-authority and publication-readiness notes,
  the Coldcard case-overview note, the missing-rung warning, the repository-scan
  note, and the campaign note. Each exact fact that a shortened line no longer
  states was moved into a labelled record — `Promotion`, `Authorization`,
  `Execution`, and `Missing source` — beside the records that were already
  there.
- Work detail Block cards use the shared cue for source availability instead of
  the `Source-backed`/`Unavailable` word pair, and the narrated two-sentence
  authority note is now the concise `View only — grants no authority`. The exact
  source reference and the Block's facts remain visible domain records.
- The Forensics preflight empty state still says which admitted managed profile
  is missing. That is a precise limitation, not chrome, and the lint leaves
  single-fragment limitation text alone.

## Remaining installed gate

The automated rule now reads the constructor that carries most Omega copy, and
passes over the current inventory. What it cannot do is confirm that the
retained meaning is still legible.

The installed review of build `16b6dc4b5b` found that none of the status
information in the Forensics surface reaches the rendered accessibility tree:
the badges, the `State`/`Blocker`/`Next action`/`Authority` rows and their
values, and the per-row catalog states are all absent from it. That is the
incomplete-tree defect tracked as omega#217, and it means the migrated cues are
proved at construction and not against a rendered tree. The `#219` criterion
"status chips/icons retain exact accessible names" is therefore **blocked on
#217**, not met and not refuted here.

Localization expansion is not runnable — Omega ships no localization mechanism,
so the character limits remain calibrated on English only. Reduced motion is not
meaningfully observable — nothing animates in the states reachable from the
current preflight. Omega does not follow macOS appearance; the light theme
applies through `theme.mode`, and the installed review reported secondary and
muted text as noticeably lower contrast there, as an observation rather than a
measured ratio.

omega#219 remains open for that installed visual and accessibility review.
