---
title: Omega Agent
description: Use Omega's first-party agent with Omega-configured models, tools, profiles, skills, instructions, and MCP servers.
---

# Omega Agent

Omega Agent is Omega's first-party agent path. Its current native executor runs
in the [Agent Panel](./agent-panel.md) and [Threads Sidebar](./parallel-agents.md#threads-sidebar),
uses models configured through [LLM Providers](./llm-providers.md), and
integrates with Omega's project, editor, terminal, and review surfaces.

The new-conversation front door prepares one Omega Agent conversation and
shows it as Ready once its router has a live, recorded executor inventory.
Selecting the row reveals that same prepared conversation. The first submitted
request supplies the task requirements for one deterministic route; only then
does the selected executor create its session. Its persisted owner remains
Omega Agent even when the route receipt names a different concrete executor.

An automatic route may select Omega's native loop or a ready ordinary ACP
agent. Full Auto engine lanes still require their own explicit human authority
and are not automatic chat candidates. A new-conversation override wins over
automatic routing, is consumed by that conversation, and cannot retarget an
existing transcript.

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

| Capability                 | Source of truth                           |
| -------------------------- | ----------------------------------------- |
| Model access               | [LLM Providers](./llm-providers.md)       |
| Panel workflow             | [Agent Panel](./agent-panel.md)           |
| Tool availability          | [Agent Profiles](./agent-profiles.md)     |
| Tool approval behavior     | [Tool Permissions](./tool-permissions.md) |
| Built-in tools             | [Tools](./tools.md)                       |
| External tools             | [MCP](./mcp.md)                           |
| Reusable task instructions | [Skills](./skills.md)                     |
| Always-on instructions     | [Instructions](./instructions.md)         |

## How It Differs from Other Agent Paths {#other-agent-paths}

| Agent path                                | Main difference                                                                              |
| ----------------------------------------- | -------------------------------------------------------------------------------------------- |
| [Omega Agent](./omega-agent.md)             | Uses Omega's model, tool, profile, skill, instruction, and MCP configuration                 |
| [External Agents](./external-agents.md)   | Use an ACP integration and often own auth, model, tool, and native instruction configuration |
| [Terminal Threads](./terminal-threads.md) | Run a CLI/TUI in a terminal-backed thread; the CLI owns auth and configuration               |

See [Agents](./agents.md) for the full comparison.
