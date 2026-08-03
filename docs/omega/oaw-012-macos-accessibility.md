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

Source admission is only the first slice of omega#217. Close the issue only
after one installed v0.2.0 candidate supplies the complete automated
observation set and an independent VoiceOver pass covers navigation, focus,
selection, actions, status announcements, live updates, and virtualized Work
content. Record any unlabeled or unreachable control as a product defect. Do
not convert it to a waiver or a generic placeholder name.
