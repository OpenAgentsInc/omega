# eval-cli

Headless Rust binary for running Omega Agent in evaluation and benchmark
environments. It is designed for containerized harnesses such as
[Harbor](https://harborframework.com/) and Pier, where the repository is already
checked out and model API keys are provided via environment variables.

`eval-cli` uses the same `NativeAgent` + `AcpThread` pipeline as the production
Zed editor: a full agentic loop with tool calls, subagents, and retries, without
a GUI.

This directory also contains `zed_eval/`, the Python `zed-eval` package used to
build this binary, launch remote benchmark runs on Modal/Harbor/Pier, and fetch
results. For normal benchmark orchestration, start with
[`zed_eval/README.md`](zed_eval/README.md).

## Building

### Native, for local testing on the same OS

```sh
cargo build --release -p eval_cli
```

The dependency graph is intentionally headless: it excludes `agent_ui`,
`editor`, `terminal_view`, `theme`, and the LiveKit/WebRTC stack, so no C++
compiler, ALSA, or GLib development packages are required. On Debian/Ubuntu the
system dependencies are:

```sh
apt-get install -y cmake build-essential libssl-dev pkg-config
```

(`build-essential` provides the C toolchain for `-sys` crates and tree-sitter
grammars, `cmake` is needed by `aws-lc-sys`, and `libssl-dev`/`pkg-config` by
`native-tls`.)

Tree-sitter grammars are compiled in by default through the `load-grammars`
feature. If evals do not need syntax-aware language support, build with
`--no-default-features` to skip compiling all grammars:

```sh
cargo build --release -p eval_cli --no-default-features
```

### Linux x86_64, for Harbor/Pier sandboxes

Harbor and Pier containers run Linux x86_64. From the repository root, use the
Docker-based build script:

```sh
crates/eval_cli/script/build-linux
```

This produces `target/eval-cli`, an x86_64 Linux ELF binary. You can also
specify a custom output path:

```sh
crates/eval_cli/script/build-linux --output ~/bin/eval-cli-linux
```

## Standalone usage

```sh
eval-cli \
  --workdir /testbed \
  --model anthropic/claude-sonnet-4-6 \
  --profile wide \
  --instruction "Fix the bug described in..." \
  --timeout 600 \
  --output-dir /logs/agent
```

`eval-cli` reads provider API keys from environment variables such as
`ANTHROPIC_API_KEY` and `OPENAI_API_KEY`. It writes `result.json`, `thread.md`,
and `thread.json` to the output directory.

`--profile basic` selects the closed Omega five-tool profile.
`--profile market` enables project context servers with only the skill tool from
the built-in set. Use it to test an agent and its MCP tools without a GUI.
`--profile wide` selects the inherited write profile and is the default for
backward-compatible benchmark runs. `result.json` records the selected profile.

To test Omega Agent against the local OpenAgents API and this repository's
market server, start the API on port 8080, then run:

```sh
cargo run -p eval_cli -- \
  --workdir . \
  --model openagents/omega-agent \
  --profile market \
  --openagents-development-api \
  --instruction "Use market_network_status and summarize the live demo network." \
  --timeout 120 \
  --output-dir /tmp/omega-market-agent
```

The command writes the full thread and a tool-call count to the output
directory. Swap execution still needs user approval.

### Exit codes

| Code | Meaning                                   |
| ---- | ----------------------------------------- |
| 0    | Agent finished                            |
| 1    | Error, such as model/auth/runtime failure |
| 2    | Timeout                                   |
| 3    | Interrupted by SIGTERM or SIGINT          |

## Running benchmarks

Most benchmark runs should use the Python `zed-eval` CLI instead of invoking
`eval-cli` directly. From the repository root:

```sh
crates/eval_cli/script/install-zed-eval
zed-eval doctor --create-volume
zed-eval run rf --from local --n-tasks 2
```

For one-off source runs without installing the tool globally, use
`crates/eval_cli/script/zed-eval <args>`.

See [`zed_eval/README.md`](zed_eval/README.md) for supported benchmarks, remote
builds, Modal setup, reporting, rejudging, baselines, and Harbor/Pier installed
agent usage.
