# SCV (Space Construction Vehicle) v0.1

Hyper-lightweight, standalone Rust [Agent Client Protocol](https://agentclientprotocol.com/)
agent. v0.1 is deliberately read-only: exactly one tool, `read`.

## Identity

| Field | Value |
| --- | --- |
| Name | `scv` |
| Title | Space Construction Vehicle |
| Version | `0.1.x` (crate version) |
| Transport | ACP over JSON-RPC 2.0, one JSON object per UTF-8 line on stdio |
| Tool surface | `read` only |

## Run

```sh
# Default read root: process current directory
cargo run -p scv

# Explicit roots (repeatable)
cargo run -p scv -- --read-root /path/to/project --read-root /path/to/docs
```

- **stdout**: protocol only (newline-delimited JSON-RPC).
- **stderr**: diagnostics via `tracing` / `tracing-subscriber` (`RUST_LOG` supported).

## `read` tool

### Input (`schemas/read.input.schema.json`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `path` | string | required | Absolute path of a regular UTF-8 file under a read root |
| `offset` | integer ≥ 1 | `1` | One-based start line |
| `limit` | integer 1..=2000 | `2000` | Max lines to return |

`additionalProperties: false`. Range is `offset .. offset+limit-1`, clipped at EOF.
Offset past EOF succeeds with an empty result.

### Output (`schemas/read.output.schema.json`)

| Field | Type | Notes |
| --- | --- | --- |
| `path` | string | Requested path |
| `content` | string | Numbered lines joined with `\n`, no trailing newline |
| `line_start` / `line_end` | int or null | Null when no lines returned |
| `truncated` | bool | True when stopped at the response-size bound |

Each content line: line number right-aligned, space-padded to
`max(6, decimal digits of line_end)`, then one tab, then the source line.

### Response size bound

Maximum `content` payload: **1_048_576 bytes** (`MAX_CONTENT_BYTES`).

- If the bound is hit after at least one complete display line: `truncated: true`.
- If the first display line alone exceeds the bound: tool error `response_too_large`.

### File safety

1. Path must be absolute.
2. Lexical path must lie under a configured read root.
3. After symlink resolution (`canonicalize`), the target must still lie under a root
   and must be a regular file.
4. Valid UTF-8 only (no lossy decode). Never mutates files.

### Tool error codes

| Code | Meaning |
| --- | --- |
| `invalid_params` | Schema / limit violation (`JSON-RPC -32602`) |
| `path_not_allowed` | Relative, outside root, or symlink escape |
| `not_found` | Missing file under a root |
| `not_regular_file` | Directory or other non-file |
| `invalid_text` | Not valid UTF-8 |
| `read_failed` | OS denied/interrupted read |
| `response_too_large` | No complete line fits the size bound |

Errors carry a concise public-safe message and the requested path; never file
content or OS error strings.

## ACP binding notes (schema grounding)

Pinned binding: `agent-client-protocol = "=2.0.0"` (workspace), schema types from
`agent_client_protocol::schema::v1`.

| Concern | SCV approach |
| --- | --- |
| Lifecycle | Standard `initialize`, `session/new`, `session/prompt`, `session/cancel` |
| Identity | `agentInfo.name = "scv"`, `version = 0.1.x`, title as above |
| Capabilities | Default `AgentCapabilities` (no load_session, no image/audio/embedded prompt, no MCP HTTP/SSE) |
| Tool list | **Deviation:** ACP v1 has no formal agent-advertised tool-descriptor list. SCV advertises `read` as the sole entry in `session/update` → `available_commands_update` after `session/new`. |
| Tool invocation | **Deviation:** ACP v1 has no client→agent tool-call RPC. Clients invoke `read` by sending `session/prompt` with a text block whose JSON is either a bare read input object or `{"tool":"read","arguments":{...}}` (`name` is accepted as an alias for `tool`). |
| Tool results | Standard tool-call envelope: `session/update` `tool_call` / `tool_call_update` with `kind: read`, `rawInput` / `rawOutput`. |
| Unknown tool | JSON-RPC `-32601` (`Method not found`) when `tool`/`name` is not `read`. |
| Invalid read params | JSON-RPC `-32602` with structured `data.code = "invalid_params"`. |
| Other tool failures | JSON-RPC application code `-32001` with structured `data` (`code`, `message`, `path`), plus a failed tool-call update. |
| Pre-initialize | JSON-RPC `-32600` with `data.code = "not_initialized"`. |
| Invalid envelope / unknown methods | Handled by the pinned ACP JSON-RPC stack (`-32600` / `-32601`). |

## Module layout

```text
crates/scv/
├── Cargo.toml          # [lib] path = "src/scv.rs"
├── README.md
├── schemas/
│   ├── read.input.schema.json
│   └── read.output.schema.json
└── src/
    ├── main.rs         # CLI, stderr tracing, exit status
    ├── scv.rs          # library root
    ├── server.rs       # ACP lifecycle + dispatch
    ├── protocol.rs     # ACP type adapter
    ├── read.rs         # validation, range, formatting
    ├── roots.rs        # root confinement
    └── error.rs        # JSON-RPC + tool errors
```

## Tests

```sh
cargo test -p scv
./script/clippy -p scv
```
