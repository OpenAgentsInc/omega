#!/usr/bin/env node
/**
 * Minimal omega-effectd framed-protocol fixture for Rust supervisor tests.
 * Speaks openagents.omega.effectd.v1 on stdin/stdout.
 */
import { createInterface } from "node:readline"
import { mkdirSync, readFileSync, existsSync } from "node:fs"
import path from "node:path"

const schema = "openagents.omega.effectd.v1"
const dataRoot = process.env.OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT
if (!dataRoot) {
  console.error("OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT is required")
  process.exit(2)
}

mkdirSync(path.join(dataRoot, "full-auto"), { recursive: true, mode: 0o700 })

let generation = 0
let running = false

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

const listRuns = () => {
  const file = path.join(dataRoot, "full-auto", "runs.json")
  if (!existsSync(file)) return []
  try {
    const parsed = JSON.parse(readFileSync(file, "utf8"))
    const runs = Array.isArray(parsed.runs) ? parsed.runs : []
    return runs.map((run) => ({
      runRef: run.runRef,
      threadRef: run.threadRef ?? null,
      state: run.state,
      title: run.title,
      updatedAt: run.updatedAt,
    }))
  } catch {
    return []
  }
}

console.error(
  JSON.stringify({
    service: "fake-omega-effectd",
    status: "listening",
    protocol: schema,
    dataRoot,
  }),
)

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity })
for await (const line of rl) {
  const trimmed = line.trim()
  if (!trimmed) continue
  let request
  try {
    request = JSON.parse(trimmed)
  } catch {
    respond("invalid", generation, false, undefined, {
      code: "invalid_request",
      message: "Frame was not valid JSON.",
    })
    continue
  }
  if (request.method === "initialize") {
    generation = request.params?.generation ?? 1
    running = true
    respond(request.id, generation, true, {
      schema,
      protocolVersion: 1,
      serviceVersion: "0.1.0",
      generation,
      capabilities: ["health", "list_runs", "get_run", "pause", "resume", "stop"],
      dataRoot,
      activeRunLimit: 8,
    })
    continue
  }
  if (request.generation !== generation) {
    respond(request.id, generation, false, undefined, {
      code: "stale_generation",
      message: `Expected generation ${generation}, got ${request.generation}.`,
    })
    continue
  }
  if (request.method === "health") {
    respond(request.id, generation, true, {
      ok: true,
      status: running ? "running" : "stopped",
      generation,
      dataRoot,
      activeRunCount: listRuns().filter((run) =>
        ["running", "pausing", "paused", "retrying", "stalled"].includes(run.state),
      ).length,
    })
    continue
  }
  if (request.method === "list_runs") {
    respond(request.id, generation, true, { runs: listRuns() })
    continue
  }
  respond(request.id, generation, false, undefined, {
    code: "unknown_method",
    message: `Unknown method ${request.method}.`,
  })
}
