---
title: Agent Profiles - Zed
description: Configure Omega Agent profiles for model selection, built-in tool availability, and MCP tool availability.
---

# Agent Profiles

Agent profiles control how the [Omega Agent](./omega-agent.md) behaves in a thread. A profile can set a default model and choose which built-in tools and MCP tools are available.

Profiles do not decide whether a tool call is allowed automatically. Use [Tool Permissions](./tool-permissions.md) to control allow, deny, and confirm behavior.

## Built-in Profiles {#built-in-profiles}

Zed includes four built-in profiles:

- `Basic`: the default six-tool coding surface: `read`, `write`, `edit`,
  `bash`, `delegate`, and `resume_thread`. MCP/context-server tools are off.
- `Editor`: enables the broad built-in and context-server tool surface.
- `Ask`: focuses on read-only codebase questions.
- `Minimal`: uses no project tools.

## Configure Profiles {#configure-profiles}

Open the profile selector in the Agent Panel, then click `Configure`.

You can also run {#action agent::ManageProfiles} from the command palette.

From the profile modal, you can:

- create a custom profile
- fork an existing profile
- configure a profile default model
- configure built-in tools
- configure MCP tools
- delete custom profiles

## Profiles and Settings {#settings}

Profiles are stored under `agent.profiles` in your settings.

```json [settings]
{
  "agent": {
    "profiles": {
      "ask": {
        "name": "Ask",
        "tools": {
          "read_file": true,
          "grep": true,
          "terminal": false,
          "edit_file": false
        },
        "enable_all_context_servers": false,
        "context_servers": {},
        "default_model": {
          "provider": "zed.dev",
          "model": "claude-sonnet-4-5"
        }
      }
    }
  }
}
```

The exact model IDs and provider IDs depend on your configured [LLM Providers](./llm-providers.md).

## Profiles vs. Tool Permissions {#profiles-vs-tool-permissions}

| Setting          | Controls                                                              | Example                                   |
| ---------------- | --------------------------------------------------------------------- | ----------------------------------------- |
| Agent profile    | Whether a tool is available in a profile                              | Disable `terminal` in a read-only profile |
| Tool permissions | Whether a permission-gated tool call is allowed, denied, or confirmed | Always confirm `terminal` commands        |

If a tool is not available in the active profile, the Omega Agent cannot use it. If the tool is available and permission-gated, [Tool Permissions](./tool-permissions.md) still controls whether the tool call requires approval.

## Agent Path Boundaries {#agent-path-boundaries}

Agent profiles apply to the Omega Agent. External Agents and Terminal Threads do not use Omega Agent profiles unless their integration explicitly supports similar behavior.
