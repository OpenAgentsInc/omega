# QA Process

Omega's QA process is the operational contract for **drawn-implies-working**
controls. It exists so a visible button, menu row, or modal never ships as an
enabled no-op, and so new surfaces cannot arrive without being registered for
crawl coverage.

This page covers cadence, ownership, severity, the control-crawl gate
(item 17 of the owner review ledger), and the same-commit registration law.
Feedback intake for alpha testers lives in
[Alpha Feedback](./alpha-feedback.md); this page is for people shipping
Omega.

## Standing product laws the crawl enforces

These are product laws, not style preferences. The crawl gate and copy lint
exist to keep them from rotting:

1. **Drawn implies working.** A visible control has an admitted action, a
   loaded dependency, and a visible result. Enabled-looking no-ops are
   defects.
2. **No exposition in the UI.** Controls are labeled, not narrated. One-word
   tooltips are acceptable; multi-sentence tooltips and status essays fail
   the copy lint unless an allowlist entry names why.
3. **Statuses are colors/icons, never words.** Lifecycle is a colored
   dot/icon; a one-word tooltip is the maximum copy.
4. **Escape closes every modal.** Settings, pair-phone, and every future
   modal must dismiss on Escape. The crawl asserts this per surface that
   opens a modal.
5. **No screen between `+` and a blinking cursor.** New-thread friction is
   out of scope for the crawl itself but is still a product law.
6. **Version copy is short.** Footer version rules are out of scope for the
   crawl; see the release-gate `version-truth` row.

## Control-crawl gate {#control-crawl-gate}

### What it does

The hermetic control-crawl harness
(`crates/omega_control_crawl`, OMEGA-DELTA-0187):

- Enumerates interactive controls per registered scene (synthetic proving
  scene today; hermetic GPUI scenes as coverage expands).
- Activates each control with **pointer and keyboard**.
- **Fails** when activation produces zero observable consequence, unless a
  registered exemption names a reason.
- Activates **menu entries individually** so a display-only menu row cannot
  hide behind a parent that only looks interactive.
- Asserts **Escape dismissal** for every modal the crawl opens.
- Loads a **checked-in crawl registry**. A surface the product photographs
  or ships without a registry entry fails the delta check.
- Runs a **copy lint** over multi-sentence tooltips/status strings against
  `docs/omega/control-crawl-copy-allowlist.json`.

### Mutation proof

A deliberate no-op control must fail the crawl. The crate carries a
mutation-proof test that injects such a control and asserts the crawl
report is non-empty. If that test ever passes while the no-op remains
injected, the gate is broken and must not be weakened to green.

### Where it runs

| Lane                               | Command / hook                                  | Coverage today                                                            |
| ---------------------------------- | ----------------------------------------------- | ------------------------------------------------------------------------- |
| `cargo test`                       | `cargo test -p omega_control_crawl`             | Synthetic proving scene, copy lint, registry load                         |
| Installed release gate             | `script/omega-release-gate` row `control-crawl` | Runs the same cargo package as an automated row                           |
| Full visual / semantic scene crawl | Pending expansion                               | Register each new hermetic scene in the crawl registry in the same commit |

The release-gate row is automated: it must not be marked
`owner-assisted-pending`. Expansion of scene coverage is recorded as
`pending-expansion` entries in the registry, not as silent omissions.

## Crawl registry {#crawl-registry}

Path: `docs/omega/control-crawl-registry.json`

Schema: `openagents.omega.control-crawl-registry.v1`

Every interactive surface that people can reach on the shipped product, and
every hermetic scene that photographs one, must appear in this file. Status
values:

| Status              | Meaning                                                                                                                                                          |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `complete`          | Crawl coverage is implemented and enforced in `cargo test` (and the release-gate row).                                                                           |
| `pending-expansion` | Registered so the delta check knows the surface exists; crawl implementation still open. Filing the expansion is required before the surface can claim complete. |
| `exempt`            | Explicitly out of crawl with a non-empty `exemption_reason`.                                                                                                     |

### Same-commit registration law {#same-commit-registration}

**Register in the same commit that introduces the surface.** Adding a menu
entry, modal, work-surface control, or hermetic scene without a registry
row is a process defect equal to shipping an untested control.

Do not:

- Land a surface and "register later."
- Delete a registry row to make a failing crawl green.
- Broaden an exemption to silence a real no-op.

Do:

- Add the registry row + crawl coverage (or `pending-expansion` with an
  issue link) in the same commit as the surface.
- Prefer fixing the control over exempting it.
- When expanding coverage from `pending-expansion` to `complete`, land the
  crawl implementation and the status flip together.

## Copy lint {#copy-lint}

Path of allowlist: `docs/omega/control-crawl-copy-allowlist.json`

Law 2 forbids multi-sentence tooltips and status strings. The lint treats a
string as multi-sentence when it contains more than one sentence terminator
(`.`, `!`, or `?`) that introduces further prose. One-word and short
phrase labels pass. Allowlist entries must name the exact string and a
reason; do not allowlist a whole file path without the string.

## Cadence {#cadence}

| When                                 | What                                                                                    |
| ------------------------------------ | --------------------------------------------------------------------------------------- |
| Every PR that touches interactive UI | Same-commit registry update; `cargo test -p omega_control_crawl` green                  |
| Every packaged RC                    | `script/omega-release-gate` including the `control-crawl` automated row                 |
| After landing a no-op fix            | Prefer a crawl assertion over a one-off unit test when the defect is "drawn but inert"  |
| Before claiming a surface complete   | Flip registry status from `pending-expansion` to `complete` with working crawl coverage |

## Ownership {#ownership}

| Concern                                         | Owner                                                      |
| ----------------------------------------------- | ---------------------------------------------------------- |
| Crawl protocol, registry schema, mutation proof | Maintainers shipping the gate (OMEGA-DELTA-0187)           |
| Scene coverage expansion for a surface          | The agent or person who lands or last touched that surface |
| Severity assignment for crawl-found defects     | Same triage as alpha feedback (below)                      |
| Release-gate green including `control-crawl`    | Release cutter for that candidate                          |

Agents claiming UI work must leave the registry and crawl green (or an
explicit `pending-expansion` row with an issue) before push.

## Severity ladder {#severity}

Crawl failures map onto the alpha severity ladder from
[Alpha Feedback](./alpha-feedback.md):

| Crawl finding                                                                                                     | Severity                                                                                 | Expectation                                                                       |
| ----------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| Enabled control with zero observable consequence on a core flow (send, new thread, open settings, Escape-dismiss) | `severity:s1`                                                                            | Fix before the next candidate; do not ship knowingly                              |
| Enabled no-op outside core flows, or menu entry that only looks armed                                             | `severity:s2`                                                                            | Fix or exempt with reason; track on the roadmap                                   |
| Copy-lint multi-sentence tooltip/status without allowlist                                                         | `severity:s3` unless it blocks understanding of a destructive action, then `severity:s2` | Batch polish unless it misleads                                                   |
| Missing registry row for a new surface                                                                            | Process defect treated as `severity:s2` until registered                                 | Same-commit law; block merge when the delta check fires                           |
| Mutation-proof test inverted or deleted                                                                           | `severity:s0` process integrity                                                          | Restore immediately; never land a green crawl that cannot fail a deliberate no-op |

## First-run policy {#first-run}

The first run of the crawl against main is allowed to find real defects.
For each finding:

1. **Fix** in the same land when the fix is small and local.
2. **File** an issue with severity when the fix is larger than the gate land.
3. **Never** weaken the crawl, delete the mutation proof, or add a
   blanket exemption to clear the board.

Pending hermetic scene expansion is recorded in the registry as
`pending-expansion`, not as a silent pass.

## Related machinery

- Delta: `OMEGA-DELTA-0187` in `OMEGA_DELTAS.md`, enforced by
  `crates/omega_deltas`
- Registry: `docs/omega/control-crawl-registry.json`
- Copy allowlist: `docs/omega/control-crawl-copy-allowlist.json`
- Crate: `crates/omega_control_crawl`
- Release gate: `script/omega-release-gate` row `control-crawl`
- Installed matrix report: `docs/omega/release-gate.md`
- Alpha intake: [Alpha Feedback](./alpha-feedback.md)
