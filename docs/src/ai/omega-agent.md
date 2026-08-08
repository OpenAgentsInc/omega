---
title: Omega Agent
description: Use Omega's first-party cloud agent with native editor tools, profiles, skills, instructions, and MCP servers.
---

# Omega Agent

Omega Agent is Omega's first-party cloud agent. It runs its project and tool
loop in Omega, sends inference requests to the OpenAgents Responses API, and
integrates with Omega's project, editor, terminal, and review surfaces. Omega
Agent has no client model selector. The service owns its provider model and
routing policy.

The new-conversation front door prepares one Omega Agent conversation and
shows it as Ready once its native connection is live. Detected local agents
are session context for the OpenAgents service. Omega does not start their ACP
adapters while Omega Agent connects.
Selecting the row reveals that same prepared conversation. The first submitted
request creates the native session and records its route. Its persisted owner
remains Omega Agent.

Codex, Claude, and Grok are separate local agent choices in the
new-conversation front door. Selecting Omega Agent does not launch them. Full
Auto engine lanes still require their own explicit human authority and are not
automatic chat candidates.

Before the first request is dispatched, Omega durably records the normalized
task requirements, the exact readiness snapshot, the policy version, the
selected executor, the reason, and any override or fallback. The thread shows
that receipt. If the selected executor disappears, Omega names it and stops;
it never retries the request on another executor.

Use Omega Agent when you want the agent to:

- read and search your project
- edit files
- run terminal commands
- use Omega-managed MCP tools
- follow [Agent Profiles](./agent-profiles.md)
- use Omega [Skills](./skills.md) and [Instructions](./instructions.md)
- show changes in Omega's review UI

## What Omega Agent Uses {#what-omega-agent-uses}

| Capability                  | Source of truth                           |
| --------------------------- | ----------------------------------------- |
| Inference and model routing | OpenAgents Responses API                  |
| Panel workflow              | [Agent Panel](./agent-panel.md)           |
| Tool availability           | [Agent Profiles](./agent-profiles.md)     |
| Tool approval behavior      | [Tool Permissions](./tool-permissions.md) |
| Built-in tools              | [Tools](./tools.md)                       |
| External tools              | [MCP](./mcp.md)                           |
| Reusable task instructions  | [Skills](./skills.md)                     |
| Always-on instructions      | [Instructions](./instructions.md)         |

## API Environment {#api-environment}

Open **Settings**, then enable **Use Development API** to send Omega Agent
response streams to `ws://127.0.0.1:8080/v1/responses`. Other development API
requests use `http://127.0.0.1:8080/v1`. Disable it to use
`https://api.openagents.com/v1`.

Or add this to your settings.json:

```json [settings]
{
  "language_models": {
    "openagents": {
      "use_development_api": true
    }
  }
}
```

The development endpoint must serve the same Open Responses profile as the
production endpoint. Omega signs each request with the active Nostr identity.

## How It Differs from Other Agent Paths {#other-agent-paths}

| Agent path                                | Main difference                                                                                     |
| ----------------------------------------- | --------------------------------------------------------------------------------------------------- |
| [Omega Agent](./omega-agent.md)           | Uses OpenAgents model routing with Omega's tool, profile, skill, instruction, and MCP configuration |
| [External Agents](./external-agents.md)   | Use an ACP integration and often own auth, model, tool, and native instruction configuration        |
| [Terminal Threads](./terminal-threads.md) | Run a CLI/TUI in a terminal-backed thread; the CLI owns auth and configuration                      |

See [Agents](./agents.md) for the full comparison.
