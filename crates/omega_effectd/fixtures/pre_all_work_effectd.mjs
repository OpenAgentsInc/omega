#!/usr/bin/env node
/**
 * A packaged omega-effectd component that predates the All Work boundary.
 *
 * This is not a hypothetical. It is the observed behaviour of the component
 * bundled at `Contents/Resources/omega-effectd/` in the shipped Omega release
 * candidate: `initialize` succeeds and reports its Full Auto / Agent Computer
 * capabilities, the `allWork` negotiation block that Omega sent is ignored and
 * absent from the result, and every All Work method answers `unknown_method`.
 *
 * omega#223 keeps this fixture so the client seam is proven to diagnose an
 * absent All Work surface once, up front, rather than emitting one generic
 * unknown-method error per feature call site.
 */
import { createInterface } from "node:readline"

const schema = "openagents.omega.effectd.v1"
const dataRoot = process.env.OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT
if (!dataRoot) {
  console.error("OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT is required")
  process.exit(2)
}

let generation = 0

const respond = (id, gen, ok, result, error) => {
  process.stdout.write(
    `${JSON.stringify({
      schema,
      kind: "response",
      id,
      generation: gen,
      ok,
      ...(result === undefined ? {} : { result }),
      ...(error === undefined ? {} : { error }),
    })}\n`,
  )
}

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity })
for await (const line of rl) {
  if (!line.trim()) continue
  let request
  try {
    request = JSON.parse(line)
  } catch {
    respond("invalid", generation, false, undefined, {
      code: "invalid_request",
      message: "Frame was not valid JSON.",
    })
    continue
  }
  if (request.method === "initialize") {
    generation = request.params?.generation ?? 1
    // The requested `allWork` block is read and discarded, exactly as the
    // shipped component does. No `allWork` key appears in the result.
    respond(request.id, generation, true, {
      schema,
      protocolVersion: 1,
      serviceVersion: "0.1.0",
      generation,
      capabilities: ["health", "list_runs", "get_run", "start", "stop"],
      dataRoot,
      activeRunLimit: 8,
    })
    continue
  }
  if (request.method === "health") {
    respond(request.id, generation, true, {
      ok: true,
      status: "listening",
      generation,
      dataRoot,
      activeRunCount: 0,
    })
    continue
  }
  respond(request.id, generation, false, undefined, {
    code: "unknown_method",
    message: `Unknown method ${request.method}.`,
  })
}
