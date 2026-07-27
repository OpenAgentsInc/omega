---
name: omega-delta-discipline
description: The Omega repository's own contract — how a deliberate divergence from upstream is recorded and mechanically checked, what counts as green, and when work is finished. Use when changing a default, a check, or any behaviour that differs from upstream, and when deciding whether a change is done.
---

# Omega's delta discipline

Omega maintains deliberate differences from its upstream. This is the rule
that keeps those differences from rotting, and it is the one a new contributor
most often misses.

## Why it exists

A fork accumulates silent divergence. Somebody changes a default, a rebase
quietly reverts it, and nobody notices until an owner sees upstream behaviour
again in a release candidate. A code comment does not survive that, because a
merge can drop the comment and the value together.

So every deliberate difference from upstream is a **delta**, and every delta is
three things at once:

1. An entry in `OMEGA_DELTAS.md`, with a heading `### OMEGA-DELTA-NNNN — …`.
2. An ID that appears in the code it governs, so a reader who finds the code
   finds the reason.
3. A test in `crates/omega_deltas/` that fails if the Omega value reverts to
   the upstream one, and an entry in that crate's `ENFORCED_DELTAS` list.

The registry and the checks are held in sync in both directions: an ID in
`ENFORCED_DELTAS` with no heading fails, and a heading with no ID fails too.

```sh
cargo test -p omega_deltas
```

## Writing one

- **A delta is a policy record, not a changelog.** Record why the owner wanted
  it, not what the commit did.
- **Name the upstream value you are replacing**, in the entry and in the test,
  so the diff stays legible to whoever does the next rebase.
- **Take an unused ID.** IDs are never reused, and two lanes allocating numbers
  at the same time has already made four entries uncitable once.
- **A delta whose check cannot fail is not a check.** Break the Omega value,
  watch the test fail, read what it says, put it back. If it stayed green, the
  test is decoration.
- **Removing a delta is a policy change.** Delete the entry, the check, and the
  test together, and say why.

## What green means here

Green is not "it compiled".

- `cargo check` proves the types line up. It proves nothing about behaviour.
- `cargo test -p <crate>` for what you touched, plus
  `cargo test -p omega_deltas` if you went near a default, a shipped setting,
  or anything with an `OMEGA-DELTA` comment on it.
- `./script/clippy -p <crate>` — the repository's wrapper, which denies
  warnings.
- A test you have never watched fail is not evidence that anything works.

When a check fails, read the message it prints. The checks in this repository
are written to explain the defect they caught, not to say `assertion failed`.

## When it is finished

A change is finished when it is **on `main` in `OpenAgentsInc/omega` with the
checks green there**. Not when it compiles, not when it is committed locally,
and not when it is pushed to a branch. Branch work is evidence of work in
progress.

Rebase onto `origin/main` and re-run the tests before you push — several lanes
land at once, and a change that was green an hour ago against a different base
has not been tested against this one.

If you cannot finish, land the coherent part and say plainly what is real, what
is stubbed, and what is still owed. An honest partial result is worth more than
a complete-sounding one.
