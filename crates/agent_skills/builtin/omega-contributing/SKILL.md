---
name: omega-contributing
description: How a change to the Omega editor itself is made and sent, for a contributor working in the Omega development workspace. Use when changing anything under the omega repository — a crate, a default, a check, or a document — and before opening a pull request or pushing to main.
---

# Contributing to Omega

This is the path as it is **today**. If a step here does not match what the
repository actually does, this file is wrong and fixing it is part of the
change you are making.

## Where the code is

**GitHub is authoritative.** The repository is `OpenAgentsInc/omega`. A change
has landed when it is on `main` there, and not before.

The OpenAgents Forge also hosts this repository, at `tenant.openagents/omega`,
and it is where the community workspace's conversation lives. It is **not** the
authority for code today. Do not treat a push to the Forge as landing a change,
and do not write a step that only works after a migration that has not happened.

When that changes, this file is where the change lands, so that every
contributor's agent learns the new path at once instead of finding out
separately.

## Get a checkout

```sh
git clone https://github.com/OpenAgentsInc/omega.git
cd omega
```

Work from `main`. If your checkout is dirty with work that is not yours, do not
stash, reset, or check out over it — make a worktree from a clean
`origin/main` instead:

```sh
git fetch origin main
git worktree add --detach ../omega-work origin/main
```

Several lanes land at once. Fetch and rebase before you push, every time.

## Make the change

Read the repository's own rules first — `.rules` at the repository root, which
`AGENTS.md` and `CLAUDE.md` are symlinks to. It is short and it is binding.

The parts that catch people:

- Propagate errors with `?`. Do not call `unwrap()` and do not discard a
  fallible result with `let _ =`.
- Do not write comments that summarise the code. Write the comment that says
  *why*, when the why is not obvious.
- New crates declare their library root explicitly in `Cargo.toml`:
  `[lib] path = "src/<crate_name>.rs"`. Never a `mod.rs`.
- Use full words in names.

If your change alters how Omega behaves differently from upstream Zed, it is a
**delta**, and there is a second skill for that: `omega-delta-discipline`. Read
it before you write the code, not after.

## Prove it

Run the checks for what you touched, and read the output rather than the exit
code you expected:

```sh
cargo test -p <crate>
./script/clippy -p <crate>
cargo fmt -p <crate>
```

`./script/clippy` is the repository's wrapper — it adds `--release
--all-targets --all-features -- --deny warnings` and runs the workspace when
you do not pass `-p`. Use it instead of a bare `cargo clippy`.

**`cargo check` is not `cargo test`.** A compiling change is not a working one,
and a test you have never watched fail is not evidence. Break the thing your
test is about, watch the test fail and read what it says, then put it back.

## Send it

Write the commit subject as an imperative sentence naming the behaviour that
changed — `Give a tool result a height ceiling the reader can lift` — not a
summary of the diff and not a conventional-commit prefix. Cite the issue as
`(omega#NN)` when the change closes one.

If you have write access to `main`:

```sh
git fetch origin main
git rebase origin/main
cargo test -p <crate>          # again, after the rebase
git push origin HEAD:main
```

Otherwise open a pull request:

```sh
gh pr create --repo OpenAgentsInc/omega
```

A pull request needs an imperative, correctly capitalised title with no
conventional-commit prefix and no trailing punctuation, and a `Release Notes:`
section as the last section of the body, with one bullet:

```
Release Notes:

- N/A
```

Use `- Added …`, `- Fixed …`, or `- Improved …` for a user-facing change, and
`- N/A` for anything that is not.

## What not to do

Stated plainly, because each of these has cost somebody a day:

1. **Do not push work to a branch and call it done.** A change is done when it
   is on `main` and the checks are green there. A branch is evidence of work in
   progress.
2. **Do not move another contributor's uncommitted work.** No `git stash`, no
   `git reset --hard`, no `git checkout` over a dirty tree you did not dirty.
   Use a fresh worktree and say what you found.
3. **Do not edit `.rules` as part of a feature change.** If you found a trap
   worth recording, propose the text in your pull request description under a
   `Suggested .rules additions` heading and let a reviewer decide.
4. **Do not delete a failing test to reach green.** Find out what it covered.
   An inherited upstream test that contradicts a live delta is updated to the
   Omega value with the delta ID named beside it — never removed.
5. **Do not weaken a check so a change passes.** Changing the assertion to
   match the new behaviour is a policy change and needs the record that goes
   with one.
6. **Do not claim a rendered result you have not seen.** A screenshot, a test
   output, or a recorded run — not a description of what the code should do.
7. **Do not put a secret, a key, an `nsec`, or a host credential in the tree,
   in a commit message, or in an issue comment.** Not even in a test fixture.
