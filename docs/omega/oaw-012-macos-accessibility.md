# OAW-012 macOS accessibility contract

OAW-012 supersedes the alpha accessibility deferrals in omega#71 and
omega#171. Installed Omega must publish the semantic GPUI tree through macOS
Accessibility. This is a v0.2.0 release requirement, not an experimental mode.

## Runtime decision

`crates/omega/src/main.rs` constructs the application with
`Application::with_platform`. On macOS, GPUI installs its
`accesskit_macos::SubclassingAdapter` for each window. The former
`ZED_EXPERIMENTAL_A11Y=1` branch selected `Application::new_inaccessible` by
default and made every GPUI role, name, state, focus update, and action a
no-op. Omega no longer has that branch.

The internal tree remains useful for deterministic semantic tests and the
`dev: dump accessibility tree` diagnostic. It does not replace an installed
observation of the operating-system tree.

## Where an accessibility defect can be observed

There are three points, not two, and knowing which one a piece of evidence
comes from decides what a failure means.

1. **The element tree.** What `render` built. Only elements with both an
   `.id(...)` and a `.role(...)` reach the next point.
2. **The `accesskit::TreeUpdate` handed to the platform adapter.** GPUI builds
   one per frame in `Window::draw_roots` and sends it to
   `MacWindow::a11y_tree_update`. `A11yDebug` retains that exact update, so
   `Window::debug_a11y_tree_json`, the `DebugRenderSnapshot`
   `accessibility_tree_json()` used by tests, and the shipped
   `dev: dump accessibility tree` / `dev: copy accessibility tree` actions all
   serialize the same object. A test assertion here is an assertion about what
   the platform was given, not about an internal representation that the
   platform never sees.
3. **The macOS AX tree.** What AccessKit's adapter publishes after
   `accesskit_consumer`'s `common_filter` runs, and what VoiceOver and
   Computer Use read.

Between 2 and 3, `common_filter` excludes a node only when it is hidden, is a
graft, or has role `GenericContainer` or `TextRun`; a focused node is included
unconditionally. GPUI never marks a node hidden or grafted. So a node that is
present at point 2 and absent at point 3 is a narrow platform-adapter question,
and a node absent at point 3 with no assertion at point 2 is usually not a
platform question at all.

## Work-screen semantics

The v0.2.0 development Work surface publishes named List, Board-column, Table,
Timeline, Roadmap, dependency, label, signed-history, and delivery-attempt
collections. Every selectable Work row exposes the same selected state and a
stable name built only from its visible identifier, title, lifecycle, priority,
completion, and blocker count. Scene, Project, saved-View, and filter controls
describe whether they are current or will apply a new selection. Pointer and
keyboard activation still call the same existing selection handler.

Work detail and inspector regions have distinct group names. Signed delivery
semantics include actor, audience, transport state, relay target, outcome, and
time, but omit raw delivery-error detail, signature, payload digest, evidence
content, prompt, token, credential, and local path. This source contract limits
what can enter an accessible name; the installed leakage scan remains the
release authority.

Every semantic Work container and item has a stable GPUI element ID. GPUI uses
the complete ancestor-and-element ID path as the AccessKit node identity. A
role or accessible name on an element without an ID is not published. Stable
IDs also let selection, focus, and list membership survive projection refresh,
scene changes, and reordered rows without appearing as unrelated replacement
nodes to assistive technology.

The source regression mounts the real dogfood Work surface with accessibility
active and renders List, Board, Table, Timeline, Roadmap, and Work detail in
sequence. Each scene must publish its named region, and the complete tree must
exclude secret-key markers and local home paths. Pointer handlers continue to
register the matching AccessKit click action through GPUI, so assistive input
and pointer input reach the same typed callback.

## Candidate gate

`script/collect-omega-installed-observations` reads the exact installed
process by PID through macOS Accessibility. The `screen-reader-output` row can
pass only when all of these statements are true:

- the window publishes application-owned elements, not only standard macOS
  window controls;
- the Identity status and controls have usable names;
- no published label or value matches a private-key or secret shape.

An empty, unreadable, or unnamed tree blocks the candidate. Upstream Zed
parity is diagnostic context only. `WAIVABLE_CHECKS` is empty, so an alpha
waiver cannot be reused as v0.2.0 proof.

The visual OCR observations remain separate. They prove that required content
is visible and not clipped; an accessibility node alone cannot prove that.

## Remaining close evidence

### 2026-08-03 installed validation

An ad-hoc signed debug candidate at `/Applications/Omega.app` proved that the
default macOS adapter now publishes Omega-owned navigation, identity, Files,
Work-count, voice, model, and send controls. The prior observation that exposed
only the standard window controls now fails. Computer Use also activated the
sidebar toggle and New thread through macOS Accessibility while VoiceOver was
enabled; the visible route changed, Back became available, and the Work count
updated. VoiceOver was then returned to its original disabled state.

The same validation found one remaining installed-platform discrepancy. The
GPUI tree contains the composer as a named multiline text input with Focus and
SetValue actions, placeholder and value state, and synthetic text runs. The
macOS tree returned to Computer Use promotes the adjacent model, voice, and
send controls but omits that composer node. Three bounded candidate rebuilds
with the input node attached at different valid layout boundaries produced the
same result.

### 2026-08-03 root cause: there are two composers

The discrepancy was not a platform-adapter defect and not a layout boundary.
Omega draws **two** composers, and the evidence on each side of the gap was
looking at a different one.

- `ThreadView::render_message_editor` draws the thread composer. It carried
  the text-input node, and it is what the source regression mounted.
- `ConversationView::render_loading_composer` draws the pre-session composer,
  used while an executor connects (`ServerState::Loading`) **and** while
  Omega's router is ready with its first executor session deliberately
  deferred (`ConversationPreparation::RouterReady`). That is the state a
  launched window and a new thread are in, so it is the composer the installed
  observation was actually looking at. It drew a bare `EditorElement` with no
  id, no role, no name, no value, no text runs, and no Focus or SetValue
  action, while still drawing the executor menu, voice controls, and send
  button beside it.

That is the complete explanation for the reported shape of the failure: the
installed tree published the model, voice, and send controls with no field
between them, and moving the thread composer's node to three different valid
layout boundaries could not change it, because that element was never on
screen during the observation.

Both composers now build their accessible node through one
`accessible_composer_input` helper in `conversation_view.rs`, so the pair
cannot drift apart again, and
`pre_session_composer_publishes_its_text_input_node` asserts the pre-session
composer's node in the same `TreeUpdate` that GPUI hands the platform adapter.
That regression fails on the previous source and passes on the current source.

The installed close evidence is still outstanding. A candidate carrying this
fix must be built, and an installed VoiceOver journey must reach the composer,
before omega#217 can close. The prediction is that the node now appears,
because `common_filter` does not exclude `MultilineTextInput` and the same
adapter already publishes Omega's other roles; a prediction is not the
observation.

Source admission is only the first slice of omega#217. Close the issue only
after one installed v0.2.0 candidate supplies the complete automated
observation set and an independent VoiceOver pass covers navigation, focus,
selection, actions, status announcements, live updates, and virtualized Work
content. Record any unlabeled or unreachable control as a product defect. Do
not convert it to a waiver or a generic placeholder name.
