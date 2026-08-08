#!/usr/bin/env node
// A dependency-free MCP stdio server exposing the swap market DEMO tools.
//
// Every tool returns deterministic demo data mirroring the Component Library
// fixtures (two pinned relays, three providers, provider-b at 22 bps). It
// never touches the network and never moves funds; it exists so the
// market conversation flow can be exercised end to end before the live
// wiring lands (omega#247, second pass). Wired via `.omega/settings.json`
// `context_servers.market-demo`; taught to agents by the `market-demo`
// skill; disable either to turn the surface off.
//
// Transport: newline-delimited JSON-RPC 2.0 on stdio, per the MCP spec.

import { createInterface } from "node:readline";

const DEMO_DISCLOSURE =
  "DEMO DATA: deterministic fixture, not the live network; no real funds move.";

const NETWORK = {
  schema: "omega.market-demo.network-status.v1",
  disclosure: DEMO_DISCLOSURE,
  name: "public regtest (demo)",
  relays: [
    { label: "relay-a", state: "ready", trust: "pinned" },
    { label: "relay-b", state: "ready", trust: "pinned" },
  ],
  providers: [
    {
      label: "provider-a",
      state: "ready",
      trust: "pinned",
      relays: ["relay-a", "relay-b"],
      fee_bps: 18,
      volume_sat_24h: 2400000,
    },
    {
      label: "provider-b",
      state: "ready",
      trust: "pinned",
      relays: ["relay-a", "relay-b"],
      fee_bps: 22,
      volume_sat_24h: 5100000,
    },
    {
      label: "joiner",
      state: "ready",
      trust: "discovered",
      relays: ["relay-b"],
      fee_bps: 30,
      volume_sat_24h: 150000,
    },
  ],
  stats: {
    swaps_24h: 128,
    volume_sat_24h: 7650000,
    operator_fee_sat_24h: 16830,
  },
};

const ASSETS = ["LN", "BTC", "L-BTC"];
const SWAP_STAGES = ["contract", "funding", "executing", "settled"];
const VERIFICATION = {
  contract: "exit package persisted before any funding",
  funding: "provider status is a claim · verifying locally",
  executing: "provider status is a claim · verifying locally",
  settled: "verified locally · zero-loss close",
};

// In-memory demo state: quotes and swaps advance one stage per status poll.
let quoteCounter = 0;
let swapCounter = 0;
const quotes = new Map();
const swaps = new Map();

const TOOLS = [
  {
    name: "market_network_status",
    description:
      "Snapshot of the swap network: relays, providers, trust tiers, and " +
      "24h aggregates. " +
      DEMO_DISCLOSURE,
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
  {
    name: "market_swap_quote",
    description:
      "Request the best firm quote for a swap between LN, BTC, and L-BTC. " +
      "Returns a quote id to pass to market_execute_swap after the person " +
      "approves. " +
      DEMO_DISCLOSURE,
    inputSchema: {
      type: "object",
      properties: {
        from: { type: "string", enum: ASSETS },
        to: { type: "string", enum: ASSETS },
        amount_sats: { type: "integer", minimum: 1000, maximum: 10000000 },
      },
      required: ["from", "to", "amount_sats"],
      additionalProperties: false,
    },
  },
  {
    name: "market_execute_swap",
    description:
      "EFFECTFUL IN THE REAL FLOW — require explicit user approval before " +
      "calling. Executes a quoted swap and returns a swap id; poll " +
      "market_swap_status until the stage is settled. " +
      DEMO_DISCLOSURE,
    inputSchema: {
      type: "object",
      properties: { quote_id: { type: "string" } },
      required: ["quote_id"],
      additionalProperties: false,
    },
  },
  {
    name: "market_swap_status",
    description:
      "Current stage of a swap (contract → funding → executing → settled) " +
      "with its verification caption and stage timeline. Advances one stage " +
      "per poll in this demo. " +
      DEMO_DISCLOSURE,
    inputSchema: {
      type: "object",
      properties: { swap_id: { type: "string" } },
      required: ["swap_id"],
      additionalProperties: false,
    },
  },
];

function toolError(message) {
  return {
    content: [{ type: "text", text: JSON.stringify({ error: message }) }],
    isError: true,
  };
}

function toolResult(value) {
  return { content: [{ type: "text", text: JSON.stringify(value, null, 2) }] };
}

function swapView(swap) {
  const stage = SWAP_STAGES[swap.stageIndex];
  return {
    schema: "omega.market-demo.swap.v1",
    disclosure: DEMO_DISCLOSURE,
    swap_id: swap.id,
    from: swap.from,
    to: swap.to,
    amount_sats: swap.amountSats,
    provider: "provider-b",
    fee_bps: 22,
    stage,
    verification: VERIFICATION[stage],
    stages_completed: SWAP_STAGES.slice(0, swap.stageIndex),
    stages_remaining: SWAP_STAGES.slice(swap.stageIndex + 1),
  };
}

function callTool(name, args) {
  switch (name) {
    case "market_network_status":
      return toolResult(NETWORK);
    case "market_swap_quote": {
      const { from, to, amount_sats } = args;
      if (!ASSETS.includes(from) || !ASSETS.includes(to) || from === to) {
        return toolError("from and to must be distinct assets among LN, BTC, L-BTC");
      }
      if (!Number.isInteger(amount_sats) || amount_sats < 1000 || amount_sats > 10000000) {
        return toolError("amount_sats must be an integer between 1,000 and 10,000,000");
      }
      quoteCounter += 1;
      const id = `demo-quote-${quoteCounter}`;
      const feeSats = Math.ceil((amount_sats * 22) / 10000);
      const quote = {
        schema: "omega.market-demo.quote.v1",
        disclosure: DEMO_DISCLOSURE,
        quote_id: id,
        from,
        to,
        amount_sats,
        provider: "provider-b",
        fee_bps: 22,
        fee_sats: feeSats,
        miner_fee_budget_sats: 300,
        output_sats: amount_sats - feeSats - 300,
        kind: "firm",
        expires_in_seconds: 120,
      };
      quotes.set(id, quote);
      return toolResult(quote);
    }
    case "market_execute_swap": {
      const quote = quotes.get(args.quote_id);
      if (!quote) {
        return toolError(`unknown quote_id ${args.quote_id}; request a fresh quote first`);
      }
      swapCounter += 1;
      const swap = {
        id: `demo-swap-${swapCounter}`,
        from: quote.from,
        to: quote.to,
        amountSats: quote.amount_sats,
        stageIndex: 0,
      };
      swaps.set(swap.id, swap);
      return toolResult(swapView(swap));
    }
    case "market_swap_status": {
      const swap = swaps.get(args.swap_id);
      if (!swap) {
        return toolError(`unknown swap_id ${args.swap_id}`);
      }
      if (swap.stageIndex < SWAP_STAGES.length - 1) {
        swap.stageIndex += 1;
      }
      return toolResult(swapView(swap));
    }
    default:
      return null;
  }
}

function respond(id, result) {
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id, result }) + "\n");
}

function respondError(id, code, message) {
  process.stdout.write(
    JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } }) + "\n",
  );
}

const stdin = createInterface({ input: process.stdin });
stdin.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed) {
    return;
  }
  let message;
  try {
    message = JSON.parse(trimmed);
  } catch {
    return;
  }
  const { id, method, params } = message;
  if (id === undefined || id === null) {
    return; // A notification; nothing to answer.
  }
  switch (method) {
    case "initialize":
      respond(id, {
        protocolVersion: params?.protocolVersion ?? "2024-11-05",
        capabilities: { tools: {} },
        serverInfo: { name: "market-demo", version: "0.1.0" },
      });
      break;
    case "ping":
      respond(id, {});
      break;
    case "tools/list":
      respond(id, { tools: TOOLS });
      break;
    case "tools/call": {
      const result = callTool(params?.name, params?.arguments ?? {});
      if (result === null) {
        respondError(id, -32602, `unknown tool ${params?.name}`);
      } else {
        respond(id, result);
      }
      break;
    }
    default:
      respondError(id, -32601, `method not found: ${method}`);
  }
});
