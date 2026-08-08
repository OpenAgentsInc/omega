# Omega as a first-party Bazaar client

Date: 2026-08-07
Status: analysis / plan of record proposal
Repos surveyed: `~/work/omega`, `~/work/bazaar`, `~/work/immortal`

## 1. Purpose

Bazaar (the web client) and Immortal (the relay/provider backend) now specify an
open, Nostr-coordinated liquidity market. This document analyzes what Omega
should build to become a first-party client of that market — comparable in
capability to the Bazaar web app, plus the thing only Omega has: resident
agents. The goal is that a user can talk to any agent in Omega — the Omega
Agent, or an external ACP agent such as Claude Code or Codex — and that agent
can observe and operate the Bazaar programmatically, with the user's consent
mediated by Omega's existing permission machinery.

The short version of the conclusion: most of the integration is configuration
and skills, not new protocol code. Bazaar already ships a stdio MCP server
whose plan of record names Omega as the primary client and states that exposure
is "pure config, no omega code change." Omega already ships a production MCP
client, a built-in skills registry, per-tool permission policy with
`mcp:<server>:<tool>` addressing, and a gated NIP-MKT market panel pinned to
`immortal-client`. The work is to connect these deliberately, ship sane
defaults, and then decide how far the native (non-MCP) lane should go.

## 2. What exists today

### 2.1 Immortal — the backend

Immortal (`OpenAgentsInc/immortal`, CC0, Rust) is "hardened Rust infrastructure
for the open swap network: one binary and one Postgres database per product."
Four production crates plus a lab harness:

- `immortal-relay` (binary `immortal`) — a from-scratch Nostr relay that is
  also the market coordination fabric. Speaks NIP-01/09/11/17/29/32/40/42/45/
  50/65/70/86/94/98, the Block agent lane (NIP-OA/AA/AE/AM/AO/…), and the
  OpenAgents market lane (`nip-mkt`, `mkt-swp:1`, PFI/MINT/P2P/LSP profiles).
- `immortal-provider` — the liquidity provider daemon. It owns the money and
  drives bitcoind/CLN/LND/elementsd/arkd. The relay never touches a rail node.
- `immortal-core` / `immortal-client` — wasm-safe libraries. `immortal-client`
  contains the verify-before-fund MKT-SWP requester engine
  (`mkt_swp_client.rs`: `SwapRecordFactory`, `SwapSession<State>` typestates,
  `RequesterSessionView`, exit packages, `verify_before_fund`).

The market protocol is NIP-MKT (`immortal/nips/openagents/MKT.md`) with the
MKT-SWP swap profile (`MKT-SWP.md`): public heads for kind `39600` provider
profiles, `39601` offerings, `39602` descriptors, `39603` public receipts;
NIP-59 gift-wrapped private records for `39604` RFQ → `39605` Quote → `39606`
Order → `39610` bilateral Swap Contract → `39607` Status / `39608` Cancel /
`39609` Close. Private kinds are immutable per `(pubkey, kind, d)` — replay is
idempotent, mutation is `invalid: idempotency-conflict`. Swaps are atomic
HTLC/taproot swaps across Bitcoin, Lightning, and Liquid (Ark gated behind
regtest fixtures); amounts are canonical decimal satoshi strings, never JSON
numbers or floats.

The custody boundary is the design's spine and any Omega surface must repeat
it: the relay validates, indexes, routes, and coordinates, but never holds
funds, seeds, spend keys, unreleased preimages, macaroons, or bearer
credentials. `immortal-relay` has no dependency on `immortal-client` or
`immortal-provider` (a build fact, checked by `cargo tree`). Client-side, the
signer stays outside the engine: the engine emits signing requests, the host
signs, the engine verifies the exact result. Relay acceptance never authorizes
funding — verification runs locally against the client's own chain source.

For SDK builders, Immortal exports a machine contract:
`contract/immortal-contract.json` (kinds, tag grammar, limits, reason strings),
`contract/immortal-fixtures.json` (106 fixtures a conformant SDK must replay
byte-for-byte), and `tests/fixtures/nipmkt/swp-requester-api-v2.json` — the
versioned typed requester API surface (v1 is withdrawn). Consumption pattern
for embedders: pin the git rev, `default-features = false`, own your
WebSocket. There is no HTTP data API and none is planned; clients speak Nostr
directly (NIP-42 auth, then `{"kinds":[1059],"#p":["<self>"]}` for private
records).

Notably: the string "bazaar" appears nowhere in the Immortal repo, and neither
does "MCP." The integration pattern is documented from the client side; Omega
is already named as a downstream consumer (`contract/README.md`: "Omega pins
this crate without the `server` feature"; tracked as omega#244).

### 2.2 Bazaar — the web client, the MCP server, and the skills

Bazaar (`~/work/bazaar`, Next.js 16 + React 19 + Effect, AGPL-3.0) is "the
client surface for an open, Nostr-coordinated liquidity market"
(`bazaar/PRODUCT.md`), and its product register names agents as a first-class
user class. It has no server-side API: the browser is the protocol host. The
requester engine is the `immortal-client` crate compiled to WASM, pinned by
SHA-256 and verified before instantiation. Identity is a locally generated
Nostr key in IndexedDB; the relay session is NIP-42 authenticated; quotes are
folded and selected client-side (highest output → lowest max fee →
lexicographically lowest provider key); funding effects go through a
capability-authenticated public-regtest gateway (`Authorization:
ImmortalRegtest <64-hex>`, kind-27236 signed session manifests, kind-27237
signed launch envelopes).

**Caution: the Bazaar working tree at `~/work/bazaar` is stale.** Local `main`
(`437f741`) is 38 commits behind `origin/main` (`8509187`). Everything below —
the MCP server, the network map, the onboarding plan — exists on `origin/main`
only. Read with `git -C ~/work/bazaar show origin/main:<path>`.

The pieces that matter to Omega, all on Bazaar `origin/main`:

**`packages/immortal-mcp` — `@openagentsinc/immortal-mcp`** (CC0, stdio
transport, `dist/` checked in so `node dist/index.js` works without a build).
Eight v1 tools:

| Tool | Class | What it does |
|---|---|---|
| `network_status` | read-only | Fetch + verify the signed kind-27237 launch envelope, probe relay NIP-11s, one bounded REQ of kinds 39600/39601 per relay; returns a PanoramaNetwork-shaped snapshot. Stats it cannot prove are `null`, never fabricated. |
| `list_offerings` | read-only | Bounded kind-39601 head snapshot → normalized offerings (pairs, min/max, fee bps, provider pubkey). |
| `get_quotes` | read-only | Deliberate `not_implemented` stub — quoting requires the verified requester engine; "quotes are never faked." |
| `node_health` | read-only | `docker compose ps` + join-kit health JSON for a locally joined node. |
| `spin_up_node` | effectful | Runs `immortal/scripts/join-regtest.sh <provider|relay>` with progress streaming. |
| `join_network` | effectful | Discrete publish entrypoint of the join kit, when present. |
| `faucet_fund` | effectful | POST to the public-regtest gateway faucet, poll to `paid`. Regtest addresses only. |
| `request_listing` | effectful (pure) | Builds a prefilled GitHub issue URL requesting pinned-listing review; opens nothing itself. |

Hard boundaries are compiled into every tool description: regtest only, never
holds provider seeds, `rejectMainnetIdentifiers()` refuses `bc1`/`tb1`/`lnbc`
strings before any network effect, no tool can alter or sign the launch
manifest. There is no credential auth; the security model is boundary
validation plus MCP host approval gating — which is exactly the seam Omega's
`tool_permissions` fills.

**`docs/network-map-and-onboarding.md` §5 — the plan of record.** It names
Omega as the primary client and specifies the wiring:

```json
{ "context_servers": { "immortal": { "command": "bunx", "args": ["@openagentsinc/immortal-mcp"] } } }
```

with "Tool IDs surface as `mcp:immortal:<tool>` for omega's per-profile
allow/deny. Keep the four read-only tools `allow` and the four effectful tools
`approval_required`." It also records why the MCP server lives in Bazaar and
not Immortal: Bazaar owns the TS protocol client, and Immortal's dependency
allowlist and one-binary rule make it the wrong home for a Node server.

**Two skills, already dual-published** in both `.agents/skills/` (Omega
dialect) and `.claude/skills/` (Claude Code dialect):

- `join-immortal-network` — the six-step unattended flow: `network_status` →
  `spin_up_node` → poll `node_health` → `faucet_fund` → `network_status` again
  (expect `trust: "discovered"`) → `request_listing`. Includes a failure
  posture demanding honest typed-error reporting.
- `read-the-network-map` — read-only vocabulary: pinned vs discovered trust
  tiers, four health glyphs, five edge classes, how to read `network_status`
  JSON, and invariants such as "offline infrastructure stops pulsing instead
  of being hidden."

The acceptance criterion for the whole agent story (§6.4): a fresh machine
with only docker and an MCP-capable agent joins the network and appears on the
public map in under ten minutes, unattended.

**Live authorities** (per `docs/public-regtest-manifest.md`):
`https://bazaar.openagents.com`, `https://gateway.34-41-78-122.sslip.io`,
`wss://relay-a.34-41-78-122.nip.io`, `wss://relay-b.34-41-78-122.nip.io`.
Everything is public regtest; mainnet is explicitly out of scope pending a
separate hardening pass.

### 2.3 Omega — what is already in place

Omega has every substrate the plan of record assumes:

- **MCP client** (`crates/context_server`): stdio and HTTP transports, full
  OAuth (PKCE, DCR, keychain-persisted sessions), project-side
  `ContextServerStore` with status lifecycle, and
  `ContextServerRegistry` exposing MCP tools to the native agent with
  per-profile gating and `mcp:<server_id>:<tool_name>` permission ids
  (`crates/agent/src/tools/context_server_registry.rs:20`). MCP prompts
  surface as `/server.prompt` slash commands.
- **ACP forwarding**: `mcp_servers_for_project`
  (`crates/agent_servers/src/acp.rs:4810`) translates every configured context
  server into the ACP session request, so Claude Code, Codex, and other
  external agents receive the same MCP servers automatically. One
  `context_servers` entry reaches every agent class.
- **Tool permissions** (`crates/agent/src/tool_permissions.rs`,
  `crates/settings_content/src/agent.rs`): `ToolPermissionsContent` keys
  accept `mcp:server:tool` ids; modes are Allow/Deny/Confirm with deny-wins
  layering; MCP calls route through
  `ToolCallEventStream::authorize_third_party_tool`, which already renders
  "Always allow / always deny for {tool} MCP tool" options. Omega ships
  `"tool_permissions": { "default": "allow" }` (OMEGA-DELTA-0002), so
  effectful market tools need explicit `always_confirm` entries.
- **Skills** (`crates/agent_skills`): global `~/.agents/skills/`,
  project-local `.agents/skills/`, and built-in skills compiled into the
  binary via `BUILTIN_SKILL_ENTRIES`. The existing `public-nostr-chat`
  built-in (OMEGA-DELTA-0070) is the direct precedent for a first-party skill
  that teaches agents to talk to an OpenAgents-published endpoint while
  keeping host names configuration rather than code.
- **A native market lane already started** (`crates/market_ui`, omega#244):
  the NIP-MKT Markets panel, gated behind `OMEGA_MARKET_PANEL=1`, with relay
  discovery (`validate_market_relay_information` requiring the `nip-mkt`
  NIP-11 extension), offering/provider listing ingest on the pinned
  `immortal-client` crate, and an explicit unimplemented seam in
  `session_flow.rs` for the RFQ → Quote → Order → Status → Cancel/Close flow.
  `Cargo.toml:346` pins `immortal-client` at rev `a4fff098…`.
- **Network egress discipline**: `crates/app_identity` maintains
  `fixtures/endpoint_allowlist.json`; every approved host carries purpose,
  owner, disposition, expiry, and paths, enforced by tests.
- **Delta discipline**: any shipped default that diverges from upstream Zed
  needs an `OMEGA_DELTAS.md` entry plus a check and test in
  `crates/omega_deltas`.

The strings "bazaar" and "marketplace" appear nowhere in Omega today; the name
is free.

## 3. Drift ledger (fix before or during integration)

These are the cross-repo inconsistencies the integration will trip over. None
is hard to fix, but each will produce confusing failures if ignored.

1. **Bazaar checkout is 38 commits stale.** The MCP server, skills, and plan
   of record exist only on Bazaar `origin/main`. Any local testing must start
   with a pull.
2. **`immortal-client` pin skew.** Omega pins `a4fff098…`; Bazaar's WASM
   artifact pins `69a78231…`; the openagents `nip-mkt` package pins
   `15e77e0…`; Immortal `main` has moved on to Ark work (`eb7a364`). The
   native lane (workstream D) should re-pin Omega to a revision that includes
   the v2 requester API artifact, and the pin should be recorded next to the
   fixture digests it must replay.
3. **The public-regtest gateway lives on an Immortal side branch**
   (`codex/immortal-44-public-ops-fix`), not Immortal `main`. `faucet_fund`
   targets a deployment of that branch.
4. **The join kit does not exist yet.** `immortal/scripts/join-regtest.sh`
   (immortal#45) is planned; today `spin_up_node`/`join_network` return typed
   `join_script_not_found`, and the `join-immortal-network` skill cannot
   complete. Omega should ship the wiring anyway — the typed error is honest —
   but the ten-minute acceptance test is blocked on immortal#45.
5. **`get_quotes` is a stub by design.** Real quoting requires the verified
   requester engine. This is the strongest argument for Omega's native lane:
   Omega can do what the Node MCP server refuses to fake (§6).
6. **MCP client identity**: `crates/context_server/src/context_server.rs:152`
   still initializes as `Implementation { name: "Zed", … }`, and the OAuth
   client-metadata URL points at `zed.dev`. Cosmetic for stdio, but worth an
   Omega delta before Bazaar-branded HTTP servers appear in logs and consent
   screens.

## 4. Integration model

Two lanes, deliberately layered, sharing one permission and consent story:

```
                        ┌──────────────────────────────────────────┐
                        │ Immortal network (relays, providers,     │
                        │ public-regtest gateway, faucet)          │
                        └───────▲──────────────────▲───────────────┘
                                │ Nostr (NIP-01/42/59, kinds 396xx) │ HTTPS
        ┌───────────────────────┴────────┐   ┌─────────────────────┴──────┐
        │ Lane B: native protocol        │   │ Lane A: MCP                │
        │ immortal-client (Rust, pinned) │   │ @openagentsinc/immortal-mcp│
        │ market_ui panel + agent tools  │   │ (stdio, from Bazaar repo)  │
        └───────────────▲────────────────┘   └──────────▲─────────────────┘
                        │                               │
                 ┌──────┴───────────────────────────────┴──────┐
                 │ Omega: tool_permissions (mcp:immortal:*),    │
                 │ skills (built-in + .agents/skills),          │
                 │ ACP forwarding to external agents            │
                 └──────────────────────────────────────────────┘
```

**Lane A (MCP)** is the universal lane: one `context_servers` entry gives
observation and node-operation tools to the Omega Agent and to every external
ACP agent. It is config plus skills plus permission defaults. It cannot quote
or swap, by design.

**Lane B (native)** is the first-party advantage: the pinned `immortal-client`
engine inside Omega can run the full verify-before-fund requester flow that
the web app runs in WASM — with Omega's signer, persistence, and consent UI.
This is the existing omega#244 seam. Completing it makes Omega the only client
where an agent can carry a swap from discovery through a signed contract under
per-step human approval.

The lanes converge in one place: native agent tools. Once Lane B exists, Omega
can expose native tools (`market_list_offerings`, `market_request_quotes`,
eventually `market_prepare_swap`) that fill the gap the MCP server refuses to
fake — and those tools can also be published to external agents later if
warranted. Sequencing matters: Lane A is days of work and unlocks the agent
story immediately; Lane B is the durable differentiator.

## 5. Workstream A — wire the MCP server (config + policy + skills)

### A1. Ship the `immortal` context server default

Add to `assets/settings/default.json`:

```jsonc
"context_servers": {
  // OMEGA-DELTA-NNNN: first-party Immortal market MCP server (Bazaar plan of
  // record, bazaar docs/network-map-and-onboarding.md §5.1). Regtest-only.
  "immortal": {
    "enabled": false,          // opt-in at launch; see A4
    "command": "bunx",
    "args": ["@openagentsinc/immortal-mcp"]
  }
}
```

Mechanics this requires, all with existing precedent:

- A new `OMEGA-DELTA` entry plus check/test in `crates/omega_deltas` (the
  shipped `agent_servers` map is the precedent — OMEGA-DELTA-0027/0095/0203).
- Decide the runtime dependency story. `bunx`/`npx` requires a Node or Bun on
  PATH, same as the `claude-acp`/`codex-acp` agent servers already shipped.
  The package checks in `dist/`, so a vendored/pinned invocation
  (`node <path>/dist/index.js`) is available if we want to avoid registry
  fetch at first run; the harness-maintenance machinery
  (`crates/project/src/harness_maintenance.rs`) is the model for a
  pinned-artifact install if we outgrow `bunx`.
- Endpoint allowlist: the MCP server itself makes outbound calls (manifest
  fetch, relay NIP-11, gateway faucet) from its own process, so Omega's
  `endpoint_allowlist.json` is not the enforcement point for those; but any
  Omega-side fetch (e.g. a future manifest check or registry fetch for the
  package) needs entries with purpose/owner/expiry.

Because ACP forwarding is automatic, this single entry also delivers the tools
to Claude Code, Codex, Grok, and any registry agent. No second wiring exists
to maintain.

### A2. Permission defaults: read-only allow, effectful confirm

The plan of record's split maps directly onto `tool_permissions`:

```jsonc
"tool_permissions": {
  "tools": {
    "mcp:immortal:network_status": { "default": "allow" },
    "mcp:immortal:list_offerings": { "default": "allow" },
    "mcp:immortal:get_quotes":     { "default": "allow" },
    "mcp:immortal:node_health":    { "default": "allow" },
    "mcp:immortal:spin_up_node":   { "default": "confirm" },
    "mcp:immortal:join_network":   { "default": "confirm" },
    "mcp:immortal:faucet_fund":    { "default": "confirm" },
    "mcp:immortal:request_listing":{ "default": "confirm" }
  }
}
```

This matters more in Omega than in a stock MCP host because Omega ships
`"default": "allow"` globally (OMEGA-DELTA-0002). Without these entries, an
agent could spin up docker nodes and hit the faucet unprompted. The entries
ship in `default.json` alongside the server so the policy and the capability
land in the same delta. MCP tools are gated by tool id only (no per-input
regex), which is acceptable here because the server's own boundaries refuse
mainnet identifiers before any effect.

One repo-level check to add: the `tool_permissions` doc examples use the
`mcp:server:tool` id format — a test should assert the shipped ids match what
`mcp_tool_id()` produces, so a server rename cannot silently orphan the
policy.

### A3. Profile exposure

The default profile is `basic` with `enable_all_context_servers: false`, so
the native agent will not see the tools even with the server running. Options:

1. Add an explicit preset to the shipped profiles:
   `"profiles": { "basic": { "context_servers": { "immortal": { "tools": { … } } } } }`
   enumerating the four read-only tools `true` (and optionally the effectful
   four). This preserves `basic`'s conservatism for unknown servers while
   admitting the first-party one. Recommended.
2. Ship a dedicated `market` profile. Cleaner conceptually, but profile
   proliferation is a UX cost and the router would need to know when to use it.
3. Flip `enable_all_context_servers` for `basic`. Rejected: it changes the
   meaning of the default profile for every third-party server.

External ACP agents are unaffected by profiles (they receive the server over
ACP and apply their own policy), which is another reason A2's Omega-side
confirm gates matter: they are the only gate for the native agent, but Claude
Code has its own approval layer.

### A4. Enabled-by-default, or opt-in?

Recommendation: ship the entry `"enabled": false` at first, with a one-click
enable surface (A6 / settings page), then flip to enabled-by-default once
(a) immortal#45's join kit exists, (b) the endpoint set stabilizes, and (c)
the `Implementation{name}` / OAuth-metadata Zed leftovers are cleaned up.
Rationale: an enabled-by-default server means every Omega install runs `bunx`
downloading a package at first agent turn — that deserves the same
pinned-artifact treatment the harness work got, not a silent registry fetch.
`PRODUCT.md`'s anti-reference list ("a hosted-agent upsell disguised as local
setup") argues for the integration being visible and explicit, not ambient.

### A5. Ship the skills

Two moves:

1. **Built-in skills.** Add `bazaar-market` (or adopt Bazaar's names
   verbatim: `read-the-network-map`, `join-immortal-network`) to
   `crates/agent_skills/builtin/` and `BUILTIN_SKILL_ENTRIES`, sourced from
   the Bazaar repo's `.agents/skills/` copies. Built-ins reach the native
   agent's `<available_skills>` catalog and are invocable via the `skill`
   tool. Keep Bazaar as the authoring home (it owns the MCP server whose
   contract the skills describe) and treat the Omega copies as vendored with
   the source commit recorded — same posture as Immortal's pinned NIP
   snapshots. A small conformance test comparing the vendored body against a
   recorded digest keeps drift visible.
2. **External-agent reach.** Claude Code reads `~/.agents/skills/` /
   `.claude/skills/`, not Omega built-ins. For project-scoped work the
   dual-dialect copies in the Bazaar repo already cover agents working *in*
   that repo. For "any agent in any project can operate the market," the
   skills need to be installed globally. Omega already has skill install
   machinery (`zed://skill` share links, `SkillCreatorOpenMode::Install`); a
   settings-page affordance "install market skills for external agents" that
   writes them to `~/.agents/skills/` is the honest version of this — writing
   to the user's global directory should be a click, not a side effect.

A third, free move: the MCP server can serve MCP *prompts*, which Omega
surfaces as `/immortal.*` slash commands for the native agent. Worth filing
upstream in Bazaar (e.g. a `join` prompt that walks the six-step flow), since
it costs Omega nothing.

### A6. Minimal UI

Nothing new is strictly required — the server appears in the existing MCP
Servers settings page and the tools in the tool picker. Worth adding anyway:

- A settings sub-page (or a section on an existing page) named for the
  market, with the enable toggle, connection status, and the skill-install
  affordance. Pattern: `SubPageLink` in `crates/settings_ui/src/page_data.rs`
  next to "MCP Servers", route constant in `crates/omega_actions`.
- Status honesty: surface `ContextServerStatus` transitions
  (Starting/Running/Error) rather than a static "connected" claim, matching
  the network map's "null means unknown, never none" discipline.

## 6. Workstream B — the native lane (finish omega#244)

The MCP lane deliberately cannot quote or swap. The Bazaar web app does both
by running `immortal-client` compiled to WASM. Omega links the same crate
natively — the seam is already cut:

- `crates/market_ui/src/session_flow.rs` documents the intended flow and
  currently returns
  `SessionFlowAvailability::NotImplemented { tracking_issue: "OpenAgentsInc/omega#244" }`.
- The discovery layer (relay gate, NIP-11 validation, offering/provider
  ingest) works today behind `OMEGA_MARKET_PANEL=1` against
  `OMEGA_MARKET_RELAY_URL` (default `ws://127.0.0.1:18080`, the Immortal dev
  relay).

Recommended shape, in order:

1. **Re-pin and adopt the v2 requester API.** Move the `immortal-client` pin
   forward to a revision carrying `swp-requester-api-v2.json`, and add a
   conformance test replaying the relevant fixture corpus
   (`contract/immortal-fixtures.json` client scope) so the pin can never
   drift silently. The fixtures are the contract; Bazaar's WASM pin discipline
   is the model.
2. **Session flow: RFQ → Quote.** Implement gift-wrap send/receive
   (NIP-42 auth, `#p`-scoped 1059 subscription, wrap/unwrap via
   `immortal_core::market`), drive `SwapRecordFactory` for RFQs, fold quotes
   with Bazaar's selection policy (highest output → lowest max fee → lowest
   provider key). Keys come from Omega's existing identity/signing
   infrastructure (`omega_identity`, signer broker) — a market key should be
   distinct from the user's social identity key and labeled as such, matching
   Bazaar's `local_demo_identity_only_never_fund_or_reuse` posture until a
   real key-management story is decided.
3. **Native agent tools over the session flow.** `market_list_offerings` and
   `market_request_quotes` as `AgentTool` impls in `crates/agent/src/tools/`
   (registered via the `tools!` checklist), returning the typed session view
   (`RequesterSessionView` is custody-free by construction). This closes the
   `get_quotes` gap for the Omega Agent, with real engine-backed numbers.
4. **Order/Contract/funding — human-gated.** The engine's own law helps:
   funding requires an accepted quote and order, matching bilateral 39610
   contracts, RFC-8785 digest equality, local rail verification, and a
   persisted exit package before any external effect. Map each engine signing
   request and `FundingAuthorizationRequest` to an explicit Omega approval
   surface (the `authorize_third_party_tool`-style card, but first-party).
   Policy recommendation: agents may take a session as far as a contract
   *draft*; contract signing and funding effects always require human
   confirmation, regardless of `tool_permissions` defaults. Regtest-only at
   first, mirroring the ecosystem-wide mainnet boundary.
5. **Panel promotion.** Once session flow works, promote the Markets panel
   from `OMEGA_MARKET_PANEL=1` to capability-derived navigation per
   `PRODUCT.md` (the panel appears when a validated market relay is
   configured), rather than a global default-on.

The custody rules Omega inherits and must state in its own docs: Omega holds
keys and signs (host-owned signer), but never holds provider seeds, unreleased
preimages, or rail credentials; relay/provider claims render as unverified
counterparty claims until local verification passes; amounts stay
strings/BigInt-equivalents end to end; equivocation forks are surfaced, never
resolved by timestamp.

## 7. Open decisions

1. **Naming.** "Bazaar" is the web product; the network is the Immortal swap
   network; the protocol is NIP-MKT. Recommendation: the MCP server keeps its
   upstream id `immortal` (the permission ids and plan of record already use
   it); user-facing Omega surfaces say "Market" (the panel is already
   `market_ui`), with "Bazaar" reserved for references to the web app.
2. **Where the MCP server is maintained.** Today: Bazaar repo, by explicit
   plan-of-record reasoning. If Omega vendors/pins the artifact rather than
   `bunx`-fetching, decide the update cadence and who owns the pin bump.
3. **Dormant in-process MCP path.** `crates/context_server/src/listener.rs`
   has a Rust `McpServer` (Unix-socket only). Long-term, Omega could serve
   its *native* market tools over MCP to external agents from in-process,
   replacing the Node server for Omega users entirely. Explicitly not the
   first target (the plan of record says the same), but worth keeping on the
   map because it removes the Node dependency and the stub tools at once.
4. **Key management for the native lane.** Demo-grade throwaway key (Bazaar's
   current posture) vs. integration with Omega's identity system vs. a
   dedicated market keystore. Blocks workstream B step 2; does not block
   workstream A at all.
5. **How far agents may go unattended.** Proposed line: observe and quote
   freely; node operations and faucet with confirm; contract signing and
   funding always human-approved. This should be written into the built-in
   skill text as well as the permission defaults, so the agent's instructions
   and the enforcement agree.

## 8. Phasing and acceptance

**Phase 1 — agent observation (config + skills; days).**
Ship A1–A5 with the server opt-in. Acceptance: with the toggle on, asking the
Omega Agent "what does the market look like right now?" produces a
`network_status`-grounded answer with pinned/discovered distinction and honest
`null`s; the same works in a Claude Code thread inside Omega with no
additional setup.

**Phase 2 — agent node operation (blocked on immortal#45).**
Acceptance is Bazaar's own criterion, run from Omega: a fresh machine with
docker + Omega joins the public regtest network and appears on
`bazaar.openagents.com/network` in under ten minutes, unattended except for
the effectful-tool confirmations.

**Phase 3 — native quotes (omega#244 steps 1–3).**
Acceptance: `market_request_quotes` returns engine-verified quotes against the
dev relay (`scripts/dev-relay.sh` + `dev-market-provider.sh` loop from
Immortal), fixture conformance test green on the recorded pin.

**Phase 4 — human-gated swap execution (regtest).**
Acceptance: a full submarine swap on local regtest driven from a conversation,
where every signing request and funding effect passed through an explicit
approval card, and the exit package is persisted before funding — replaying
Bazaar's funded-regtest acceptance shape inside Omega.

## 9. Source index

Bazaar (read on `origin/main`): `PRODUCT.md`,
`docs/network-map-and-onboarding.md` (§5 client wiring, §6.4 acceptance),
`docs/public-regtest-manifest.md`, `packages/immortal-mcp/README.md`,
`packages/immortal-mcp/src/{server,boundaries}.ts`,
`.agents/skills/{join-immortal-network,read-the-network-map}/SKILL.md`,
`lib/immortal/{market,transport,store,public-session}.ts`.

Immortal: `README.md`, `AGENTS.md`, `docs/MONOREPO.md`,
`nips/openagents/{MKT,MKT-SWP}.md`, `docs/protocol/{nip-mkt-validation,mkt-swp-client,provider-contract}.md`,
`contract/{README.md,immortal-contract.json,immortal-fixtures.json}`,
`tests/fixtures/nipmkt/swp-requester-api-v2.json`,
`docs/deployment/{runbook-local-dev,swap-network-infrastructure,configuration}.md`.

Omega: `crates/context_server/` (client, OAuth, listener),
`crates/project/src/context_server_store.rs`,
`crates/agent/src/tools/context_server_registry.rs` (`mcp_tool_id`),
`crates/agent_servers/src/acp.rs` (`mcp_servers_for_project`),
`crates/agent/src/tool_permissions.rs`,
`crates/settings_content/src/agent.rs` (`ToolPermissionsContent`, profiles),
`crates/agent_skills/` (`BUILTIN_SKILL_ENTRIES`, `builtin/public-nostr-chat/`),
`crates/market_ui/` (`discovery.rs`, `session_flow.rs`),
`assets/settings/default.json`, `OMEGA_DELTAS.md`, `crates/omega_deltas/`,
`crates/app_identity/fixtures/endpoint_allowlist.json`, omega#244.
