#!/usr/bin/env node
// A dependency-free MCP stdio server for the swap market.
//
// `market_network_status` can return a fixture or read the live public regtest network: it fetches
// the deployment's launch manifest for the pinned set, probes each relay's
// NIP-11, authenticates with an ephemeral key over NIP-42 (BIP-340 signing
// implemented below), and folds the kind-39600/39601 market heads it reads.
// Read-only; the ephemeral identity signs nothing but the auth event.
//
// Demo swaps remain deterministic fixtures. Regtest execution requires
// Omega's native Nostr-signed API path. Wired via `.omega/settings.json` `context_servers.market-demo`;
// taught to agents by the `market-demo` skill; disable either to turn the
// surface off.
//
// Transport: newline-delimited JSON-RPC 2.0 on stdio, per the MCP spec.

import { createHash, randomBytes } from "node:crypto";
import { createInterface } from "node:readline";

const MANIFEST_URL =
  process.env.MARKET_MANIFEST_URL ??
  "https://bazaar.openagents.com/bazaar-public-regtest.json";
const FALLBACK_RELAYS = [
  "wss://relay-a.34-41-78-122.nip.io",
  "wss://relay-b.34-41-78-122.sslip.io",
];
const RELAY_BUDGET_MS = 10000;

const LIVE_DISCLOSURE =
  "LIVE public regtest coordination data, read-only; relay and provider " +
  "claims are unverified until a requester verifies locally.";
const DEMO_DISCLOSURE =
  "DEMO DATA: deterministic fixture, not the live network; no real funds move.";
const MAINNET_WARNING =
  "Mainnet swap tools are blocked. No mainnet request was sent and no funds moved.";

// ---------------------------------------------------------------------------
// Minimal secp256k1 + BIP-340 Schnorr, enough to sign one NIP-42 auth event
// with a throwaway key. BigInt affine math; performance is irrelevant here.
// ---------------------------------------------------------------------------

const FIELD_P = 2n ** 256n - 2n ** 32n - 977n;
const CURVE_N =
  0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
const GENERATOR = [
  0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798n,
  0x483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8n,
];

const fieldMod = (a, m = FIELD_P) => ((a % m) + m) % m;

function modPow(base, exponent, modulus) {
  let result = 1n;
  base = fieldMod(base, modulus);
  while (exponent > 0n) {
    if (exponent & 1n) {
      result = (result * base) % modulus;
    }
    base = (base * base) % modulus;
    exponent >>= 1n;
  }
  return result;
}

const modInverse = (a, m = FIELD_P) => modPow(fieldMod(a, m), m - 2n, m);

function pointAdd(a, b) {
  if (!a) return b;
  if (!b) return a;
  const [ax, ay] = a;
  const [bx, by] = b;
  if (ax === bx) {
    if (fieldMod(ay + by) === 0n) {
      return null;
    }
    const lambda = fieldMod(3n * ax * ax * modInverse(2n * ay));
    const x = fieldMod(lambda * lambda - 2n * ax);
    return [x, fieldMod(lambda * (ax - x) - ay)];
  }
  const lambda = fieldMod((by - ay) * modInverse(bx - ax));
  const x = fieldMod(lambda * lambda - ax - bx);
  return [x, fieldMod(lambda * (ax - x) - ay)];
}

function pointMul(point, scalar) {
  let result = null;
  let addend = point;
  while (scalar > 0n) {
    if (scalar & 1n) {
      result = pointAdd(result, addend);
    }
    addend = pointAdd(addend, addend);
    scalar >>= 1n;
  }
  return result;
}

const bigToBytes = (n) => Buffer.from(n.toString(16).padStart(64, "0"), "hex");
const bytesToBig = (buffer) => BigInt("0x" + buffer.toString("hex"));
const sha256 = (...buffers) =>
  createHash("sha256").update(Buffer.concat(buffers)).digest();

function taggedHash(tag, ...buffers) {
  const tagDigest = sha256(Buffer.from(tag));
  return sha256(tagDigest, tagDigest, ...buffers);
}

function schnorrSign(message, secretKey) {
  let d = bytesToBig(secretKey);
  if (d === 0n || d >= CURVE_N) {
    throw new Error("secret key out of range");
  }
  const publicPoint = pointMul(GENERATOR, d);
  if (publicPoint[1] % 2n !== 0n) {
    d = CURVE_N - d;
  }
  const publicX = bigToBytes(publicPoint[0]);
  const masked = bigToBytes(
    d ^ bytesToBig(taggedHash("BIP0340/aux", randomBytes(32))),
  );
  let k = fieldMod(
    bytesToBig(taggedHash("BIP0340/nonce", masked, publicX, message)),
    CURVE_N,
  );
  if (k === 0n) {
    throw new Error("zero nonce");
  }
  const noncePoint = pointMul(GENERATOR, k);
  if (noncePoint[1] % 2n !== 0n) {
    k = CURVE_N - k;
  }
  const challenge = fieldMod(
    bytesToBig(
      taggedHash("BIP0340/challenge", bigToBytes(noncePoint[0]), publicX, message),
    ),
    CURVE_N,
  );
  return Buffer.concat([
    bigToBytes(noncePoint[0]),
    bigToBytes(fieldMod(k + challenge * d, CURVE_N)),
  ]);
}

function publicKeyOf(secretKey) {
  const point = pointMul(GENERATOR, bytesToBig(secretKey));
  return bigToBytes(point[0]).toString("hex");
}

// ---------------------------------------------------------------------------
// Live network fold
// ---------------------------------------------------------------------------

async function fetchManifest() {
  const response = await fetch(MANIFEST_URL, {
    signal: AbortSignal.timeout(6000),
  });
  if (!response.ok) {
    throw new Error(`manifest fetch returned ${response.status}`);
  }
  const envelope = await response.json();
  return envelope.manifest ?? envelope;
}

function relayLabel(url) {
  try {
    return new URL(url).hostname.split(".")[0];
  } catch {
    return url;
  }
}

/// One relay lane: NIP-11 probe, then an authenticated bounded REQ for the
/// public market heads. Returns the relay's state and the events it served.
async function probeRelay(url) {
  let nip11Ok = false;
  try {
    const response = await fetch(url.replace("wss://", "https://"), {
      headers: { Accept: "application/nostr+json" },
      signal: AbortSignal.timeout(6000),
    });
    nip11Ok = response.ok;
  } catch {
    nip11Ok = false;
  }

  const events = [];
  const socketState = await new Promise((resolve) => {
    let settled = false;
    const finish = (state) => {
      if (!settled) {
        settled = true;
        try {
          socket.close();
        } catch {
          // Already closed.
        }
        resolve(state);
      }
    };
    const timer = setTimeout(() => finish("timeout"), RELAY_BUDGET_MS);
    let socket;
    try {
      socket = new WebSocket(url);
    } catch {
      clearTimeout(timer);
      resolve("unreachable");
      return;
    }
    const secretKey = randomBytes(32);
    const publicKey = publicKeyOf(secretKey);
    socket.onerror = () => finish("unreachable");
    socket.onclose = () => finish("closed_early");
    socket.onopen = () => {
      // Auth-optional relays never send AUTH; subscribe immediately and let
      // an auth-required relay CLOSE us into the AUTH path.
      socket.send(
        JSON.stringify(["REQ", "market", { kinds: [39600, 39601], limit: 256 }]),
      );
    };
    socket.onmessage = (message) => {
      let data;
      try {
        data = JSON.parse(message.data);
      } catch {
        return;
      }
      switch (data[0]) {
        case "AUTH": {
          const event = {
            pubkey: publicKey,
            created_at: Math.floor(Date.now() / 1000),
            kind: 22242,
            tags: [
              ["relay", url],
              ["challenge", data[1]],
            ],
            content: "",
          };
          const id = sha256(
            Buffer.from(
              JSON.stringify([
                0,
                event.pubkey,
                event.created_at,
                event.kind,
                event.tags,
                event.content,
              ]),
            ),
          ).toString("hex");
          const sig = schnorrSign(Buffer.from(id, "hex"), secretKey).toString("hex");
          socket.send(JSON.stringify(["AUTH", { ...event, id, sig }]));
          break;
        }
        case "OK":
          socket.send(
            JSON.stringify([
              "REQ",
              "market",
              { kinds: [39600, 39601], limit: 256 },
            ]),
          );
          break;
        case "EVENT":
          if (data[1] === "market" && data[2]) {
            events.push(data[2]);
          }
          break;
        case "EOSE":
          clearTimeout(timer);
          finish("ready");
          break;
        default:
          break;
      }
    };
  });

  const state =
    socketState === "ready"
      ? nip11Ok
        ? "ready"
        : "degraded"
      : socketState === "unreachable" && !nip11Ok
        ? "offline"
        : "degraded";
  return { url, label: relayLabel(url), state, events };
}

function minimumFeeBps(offeringContent) {
  try {
    const sides = JSON.parse(offeringContent)?.mkt_swp?.sides;
    if (!Array.isArray(sides)) {
      return null;
    }
    const fees = sides
      .map((side) => Number.parseInt(side.fee_bps, 10))
      .filter(Number.isFinite);
    return fees.length > 0 ? Math.min(...fees) : null;
  } catch {
    return null;
  }
}

function tagValue(event, name) {
  const tag = (event.tags ?? []).find((entry) => entry[0] === name);
  return tag ? tag[1] : undefined;
}

async function liveNetworkStatus() {
  let manifest = null;
  let manifestError = null;
  try {
    manifest = await fetchManifest();
  } catch (error) {
    manifestError = String(error.message ?? error);
  }
  const relayUrls =
    manifest?.relays?.map((relay) => relay.websocket_url) ?? FALLBACK_RELAYS;
  const pinnedProviders = new Map(
    (manifest?.providers ?? []).map((provider) => [provider.pubkey, provider.role]),
  );

  const lanes = await Promise.all(relayUrls.map(probeRelay));

  // Newest head per (kind, pubkey, d) across every relay, tracking which
  // relays served each provider's heads.
  const heads = new Map();
  const providerRelays = new Map();
  for (const lane of lanes) {
    for (const event of lane.events) {
      const key = `${event.kind}:${event.pubkey}:${tagValue(event, "d") ?? ""}`;
      const existing = heads.get(key);
      if (!existing || event.created_at > existing.created_at) {
        heads.set(key, event);
      }
      const relays = providerRelays.get(event.pubkey) ?? new Set();
      relays.add(lane.label);
      providerRelays.set(event.pubkey, relays);
    }
  }

  const providers = [];
  for (const event of heads.values()) {
    if (event.kind !== 39600 || tagValue(event, "status") !== "active") {
      continue;
    }
    let profileName = null;
    try {
      profileName = JSON.parse(event.content)?.name ?? null;
    } catch {
      profileName = null;
    }
    const offerings = [...heads.values()].filter(
      (head) =>
        head.kind === 39601 &&
        head.pubkey === event.pubkey &&
        tagValue(head, "status") === "active",
    );
    const fees = offerings
      .map((offering) => minimumFeeBps(offering.content))
      .filter((fee) => fee !== null);
    const pinnedRole = pinnedProviders.get(event.pubkey);
    providers.push({
      label: pinnedRole ?? profileName ?? event.pubkey.slice(0, 8),
      pubkey: event.pubkey,
      state: offerings.length > 0 ? "ready" : "starting",
      trust: pinnedRole ? "pinned" : "discovered",
      relays: [...(providerRelays.get(event.pubkey) ?? [])],
      fee_bps: fees.length > 0 ? Math.min(...fees) : null,
      active_offerings: offerings.length,
    });
  }
  providers.sort((a, b) => a.label.localeCompare(b.label));

  return {
    schema: "omega.market-demo.network-status.v1",
    source: "live",
    disclosure: LIVE_DISCLOSURE,
    name: "public regtest",
    manifest: manifest
      ? {
          service_state: manifest.service_state,
          bazaar_revision: manifest.bazaar_revision,
          immortal_revision: manifest.immortal_revision,
        }
      : { unavailable: manifestError },
    relays: lanes.map((lane) => ({
      label: lane.label,
      url: lane.url,
      state: lane.state,
      trust: "pinned",
    })),
    providers,
    // Receipt aggregation (kind 39603) is not deployed; unknown stays
    // unknown, never zero.
    stats: {},
  };
}

// ---------------------------------------------------------------------------
// Demo swap flow.
// ---------------------------------------------------------------------------

const ASSETS = ["LN", "BTC", "L-BTC"];
const NETWORKS = ["demo", "regtest", "mainnet"];
const SWAP_STAGES = ["contract", "funding", "executing", "settled"];
const VERIFICATION = {
  contract: "exit package persisted before any funding",
  funding: "provider status is a claim · verifying locally",
  executing: "provider status is a claim · verifying locally",
  settled: "verified locally · zero-loss close",
};

let quoteCounter = 0;
let swapCounter = 0;
const quotes = new Map();
const swaps = new Map();

const TOOLS = [
  {
    name: "market_network_status",
    description:
      "Read a representative demo network or the LIVE public regtest network. " +
      "Mainnet is blocked.",
    inputSchema: {
      type: "object",
      properties: { network: { type: "string", enum: NETWORKS } },
      required: ["network"],
      additionalProperties: false,
    },
  },
  {
    name: "market_swap_quote",
    description:
      "Request a demo fixture quote or an indicative live regtest route. " +
      "Mainnet is blocked.",
    inputSchema: {
      type: "object",
      properties: {
        network: { type: "string", enum: NETWORKS },
        from: { type: "string", enum: ASSETS },
        to: { type: "string", enum: ASSETS },
        amount_sats: { type: "integer", minimum: 1000, maximum: 10000000 },
      },
      required: ["network", "from", "to", "amount_sats"],
      additionalProperties: false,
    },
  },
  {
    name: "market_execute_swap",
    description:
      "Execute a demo fixture quote. Regtest execution uses Omega's native " +
      "Nostr-signed API path. Mainnet is blocked.",
    inputSchema: {
      type: "object",
      properties: {
        network: { type: "string", enum: NETWORKS },
        quote_id: { type: "string" },
      },
      required: ["network", "quote_id"],
      additionalProperties: false,
    },
  },
  {
    name: "market_swap_status",
    description:
      "Read a recorded demo swap. Regtest state is recorded by Omega's native " +
      "tool. Mainnet is blocked.",
    inputSchema: {
      type: "object",
      properties: {
        network: { type: "string", enum: NETWORKS },
        swap_id: { type: "string" },
      },
      required: ["network", "swap_id"],
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

function mainnetWarning(operation) {
  return toolResult({
    schema: "omega.market-demo.warning.v1",
    network: "mainnet",
    operation,
    blocked: true,
    warning: MAINNET_WARNING,
  });
}

function demoNetworkStatus() {
  return {
    schema: "omega.market-demo.network-status.v1",
    network: "demo",
    source: "fixture",
    disclosure: DEMO_DISCLOSURE,
    name: "representative demo network",
    manifest: {
      service_state: "ready",
      bazaar_revision: "demo-fixture",
      immortal_revision: "demo-fixture",
    },
    relays: [
      { label: "relay-a", url: "wss://relay-a.demo.invalid", state: "ready", trust: "fixture" },
      { label: "relay-b", url: "wss://relay-b.demo.invalid", state: "ready", trust: "fixture" },
    ],
    providers: [
      { label: "provider-b", pubkey: "demo-provider-b", state: "ready", trust: "fixture", relays: ["relay-a", "relay-b"], fee_bps: 22, active_offerings: 6 },
      { label: "provider-c", pubkey: "demo-provider-c", state: "ready", trust: "fixture", relays: ["relay-a", "relay-b"], fee_bps: 34, active_offerings: 6 },
    ],
    stats: {},
  };
}

function swapView(swap) {
  const stage = SWAP_STAGES[swap.stageIndex];
  return {
    schema: "omega.market-demo.swap.v1",
    network: "demo",
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

async function callTool(name, args) {
  switch (name) {
    case "market_network_status":
      if (args.network === "mainnet") return mainnetWarning(name);
      if (args.network === "demo") return toolResult(demoNetworkStatus());
      if (args.network !== "regtest") return toolError("network must be demo, regtest, or mainnet");
      try {
        const status = await liveNetworkStatus();
        status.network = "regtest";
        return toolResult(status);
      } catch (error) {
        return toolError(`live network read failed: ${error.message ?? error}`);
      }
    case "market_swap_quote": {
      const { network, from, to, amount_sats } = args;
      if (network === "mainnet") return mainnetWarning(name);
      if (!NETWORKS.includes(network)) return toolError("network must be demo, regtest, or mainnet");
      if (!ASSETS.includes(from) || !ASSETS.includes(to) || from === to) {
        return toolError("from and to must be distinct assets among LN, BTC, L-BTC");
      }
      const minimum = network === "regtest" ? 100000 : 1000;
      const maximum = network === "regtest" ? 1000000 : 10000000;
      if (!Number.isInteger(amount_sats) || amount_sats < minimum || amount_sats > maximum) {
        return toolError(
          `amount_sats must be an integer between ${minimum.toLocaleString()} and ${maximum.toLocaleString()}`,
        );
      }
      if (network === "regtest" && (from === "L-BTC" || to === "L-BTC")) {
        return toolError("the public regtest service does not support Liquid Bitcoin yet");
      }
      let provider = "provider-b";
      let feeBps = 22;
      if (network === "regtest") {
        const status = await liveNetworkStatus();
        const ready = status.providers
          .filter((candidate) => candidate.state === "ready" && Number.isInteger(candidate.fee_bps))
          .sort((left, right) => left.fee_bps - right.fee_bps)[0];
        if (!ready) return toolError("the live regtest network has no ready swap provider");
        provider = ready.label;
        feeBps = ready.fee_bps;
      }
      quoteCounter += 1;
      const id = `${network}-quote-${quoteCounter}`;
      const feeSats = Math.ceil((amount_sats * feeBps) / 10000);
      const quote = {
        schema: "omega.market-demo.quote.v1",
        network,
        disclosure: network === "demo" ? DEMO_DISCLOSURE : LIVE_DISCLOSURE,
        quote_id: id,
        from,
        to,
        amount_sats,
        provider,
        fee_bps: feeBps,
        fee_sats: feeSats,
        miner_fee_budget_sats: 300,
        output_sats: amount_sats - feeSats - 300,
        kind: network === "demo" ? "firm" : "indicative",
        expires_in_seconds: network === "demo" ? 120 : 30,
      };
      quotes.set(id, quote);
      return toolResult(quote);
    }
    case "market_execute_swap": {
      if (args.network === "mainnet") return mainnetWarning(name);
      if (args.network === "regtest") {
        return toolError("regtest execution requires Omega's native Nostr-signed market tool");
      }
      if (args.network !== "demo") return toolError("network must be demo, regtest, or mainnet");
      const quote = quotes.get(args.quote_id);
      if (!quote) {
        return toolError(`unknown quote_id ${args.quote_id}; request a fresh quote first`);
      }
      if (quote.network !== args.network) {
        return toolError(`quote_id ${args.quote_id} belongs to network ${quote.network}`);
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
      if (args.network === "mainnet") return mainnetWarning(name);
      if (args.network === "regtest") {
        return toolError("regtest status is recorded by Omega's native market tool");
      }
      if (args.network !== "demo") return toolError("network must be demo, regtest, or mainnet");
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
        serverInfo: { name: "market-demo", version: "0.2.0" },
      });
      break;
    case "ping":
      respond(id, {});
      break;
    case "tools/list":
      respond(id, { tools: TOOLS });
      break;
    case "tools/call": {
      Promise.resolve(callTool(params?.name, params?.arguments ?? {}))
        .then((result) => {
          if (result === null) {
            respondError(id, -32602, `unknown tool ${params?.name}`);
          } else {
            respond(id, result);
          }
        })
        .catch((error) => {
          respondError(id, -32603, String(error?.message ?? error));
        });
      break;
    }
    default:
      respondError(id, -32601, `method not found: ${method}`);
  }
});
