# Omega

**Your last IDE.**

Omega is the primary OpenAgents desktop client and integrated development
environment. It brings people, agents, conversations, code, reviews, decisions,
approvals, and execution into one native workspace.

Omega is in the bootstrap phase. This repository is a tracked fork of
[Zed](https://github.com/zed-industries/zed), and much of the current
application still has upstream branding and behavior. Omega is not ready for
general use yet.

## Product direction

Omega is an IDE and a workroom for human and agent teams.

- Work stays attached to durable projects, threads, files, runs, and evidence.
- Agents have explicit identities, permissions, and signed actions.
- Existing agents can join through ACP or another supported adapter without
  losing their configuration.
- Native workroom panes connect conversations and decisions to the editor,
  terminal, Git, reviews, tasks, and remote environments.
- Nostr provides an optional signed interoperability layer for identity,
  social context, discovery, and ecosystem-wide coordination.
- Local-first and self-hosted operation remain first-class deployment choices.

The goal is not to add a chat panel to an editor. The goal is one place where a
team can understand work, delegate it, review it, approve it, and ship it.

The accepted implementation plan lives in the
[OpenAgents monorepo](https://github.com/OpenAgentsInc/openagents/blob/main/docs/sol/2026-07-23-omega-zed-primary-surface-accepted-plan.md).

## Architecture

This repository owns the Omega client:

- the Rust and GPUI application
- editor, project, terminal, Git, language, task, and remote-development
  integration
- native workroom and social surfaces
- client state, native enforcement, process supervision, packaging, and updates
- the tracked upstream relationship and Omega-specific patches

The
[OpenAgents monorepo](https://github.com/OpenAgentsInc/openagents)
owns reusable TypeScript and Effect packages, services, schemas, generated
clients, and conformance fixtures. Omega consumes released, immutable artifacts
from that repository. Primary Omega client code does not live there.

Omega will use Rust for the native application core. A supervised Node 24 and
Effect service can host product semantics that are shared with other
OpenAgents clients. The two processes use one generated and versioned local
protocol.

## Build the current application

The repository pins its Rust toolchain in
[`rust-toolchain.toml`](./rust-toolchain.toml).

```sh
git clone https://github.com/OpenAgentsInc/omega.git
cd omega
cargo run --profile release-fast
```

Platform-specific prerequisites and inherited build details are available for
[macOS](./docs/src/development/macos.md),
[Linux](./docs/src/development/linux.md), and
[Windows](./docs/src/development/windows.md).

During the bootstrap phase, build products and application identifiers can
still use upstream names.

## Dev build

Run a dev build against **its own user-data directory**, never the default
profile. The default profile is shared with the installed Omega, so a dev build
pointed at it can write settings, databases, identity material, and logs that
the installed app then inherits. Isolate it and a bad dev run costs you a
directory you can delete.

Build and run in one step:

```sh
cargo run --profile release-fast -- --user-data-dir ~/.omega-dev
```

Or build once and launch the binary directly:

```sh
cargo build --profile release-fast
./target/release-fast/omega --user-data-dir ~/.omega-dev
```

To produce a signed `.app` and DMG instead, use the bundler:

```sh
script/bundle-mac -f
```

**Know which path you built.** The two commands do not write to the same
place, and mistaking one for the other means testing a stale binary:

| Command | Binary |
| --- | --- |
| `cargo build --profile release-fast` | `target/release-fast/omega` |
| `script/bundle-mac -f` | `target/aarch64-apple-darwin/release-fast/omega`, plus `Omega-arm64.dmg` |

`bundle-mac` passes an explicit `--target`, which is why its output lands under
the target-triple directory. Check the binary's timestamp before concluding a
change did or did not take effect.

No flag selects the entry surface. Omega opens on its agent surface; the full
editor is one reveal away rather than the thing that boots.

## Contribute

Read [`AGENTS.md`](./AGENTS.md) before agent-assisted work. The inherited
[`CONTRIBUTING.md`](./CONTRIBUTING.md) describes the current code-quality,
testing, and review conventions.

Use [Omega issues](https://github.com/OpenAgentsInc/omega/issues) for
Omega-specific changes. Send generally useful editor or GPUI fixes upstream
when they do not depend on the Omega product.

## Upstream and licenses

Omega is derived from Zed and keeps a tracked relationship with
[`zed-industries/zed`](https://github.com/zed-industries/zed). OpenAgents
maintains the Omega product and its fork-specific changes. Zed Industries does
not maintain or endorse Omega.

The source is licensed primarily under
[GPL-3.0-or-later](./LICENSE-GPL), with
[Apache-2.0](./LICENSE-APACHE) components where marked. Existing copyright,
attribution, source-delivery, and third-party license obligations remain in
effect.
