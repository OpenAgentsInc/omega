// A daemon stub for testing `issue31_observation` (omega#97).
//
// This exists to drive the OBSERVER's own failure handling, which a real
// `omega-effectd` will not do on command: there is no way to ask a healthy
// daemon to list a run and then refuse to describe it.
//
// It is a test double for our error paths, never host authority. Nothing it
// emits is published to a relay by any test, and the omega#97 live proof
// (`a_running_daemon_supplies_the_reading_a_paired_device_reads_on_a_live_relay`)
// runs against the packaged `omega-effectd` instead — because putting recorded
// or scripted daemon output on the wire in place of live host state is exactly
// the substitution omega#49's exit forbids.
//
// The scenario lives in the data root so parallel tests cannot collide through
// a shared environment variable.

import { readFileSync } from "node:fs"
import path from "node:path"
import readline from "node:readline"

const SCHEMA = "openagents.omega.effectd.v1"
const dataRoot = process.env.OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT
if (!dataRoot) {
  process.stderr.write("OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT is required\n")
  process.exit(1)
}

const scenario = JSON.parse(readFileSync(path.join(dataRoot, "scenario.json"), "utf8"))
const runs = scenario.runs ?? []
const details = scenario.details ?? {}
const reports = scenario.reports ?? {}
const receipts = scenario.receipts ?? {}
const refuseGetRun = new Set(scenario.refuseGetRun ?? [])
const refuseListRuns = scenario.refuseListRuns === true
const refuseCapacity = scenario.refuseCapacity === true

const respond = (id, ok, result, error) => {
  const frame = { schema: SCHEMA, kind: "response", id, generation: 1, ok }
  if (result !== undefined) frame.result = result
  if (error !== undefined) frame.error = error
  process.stdout.write(`${JSON.stringify(frame)}\n`)
}

const refusal = (message) => ({ code: "internal", message })

readline.createInterface({ input: process.stdin }).on("line", (line) => {
  if (!line.trim()) return
  let request
  try {
    request = JSON.parse(line)
  } catch {
    return
  }
  if (request.kind !== "request") return
  const { id, method, params } = request

  switch (method) {
    case "initialize":
      respond(id, true, {
        schema: SCHEMA,
        protocolVersion: 1,
        serviceVersion: "0.0.0-observation-stub",
        generation: 1,
        capabilities: ["list_runs", "get_run", "get_capacity", "get_report", "get_receipt"],
        dataRoot,
        activeRunLimit: 8,
      })
      return
    case "list_runs":
      if (refuseListRuns) {
        respond(id, false, undefined, refusal("The stub was told to refuse list_runs."))
        return
      }
      respond(id, true, { runs })
      return
    case "get_capacity":
      if (refuseCapacity) {
        respond(id, false, undefined, refusal("The stub was told to refuse get_capacity."))
        return
      }
      respond(id, true, scenario.capacity ?? { activeRunLimit: 8, activeRunCount: 0, lanes: [] })
      return
    case "get_run": {
      const runRef = params?.runRef
      if (refuseGetRun.has(runRef)) {
        respond(id, false, undefined, refusal(`The stub was told to refuse get_run for ${runRef}.`))
        return
      }
      const run = details[runRef]
      if (!run) {
        respond(id, false, undefined, { code: "run_not_found", message: `No run ${runRef}.` })
        return
      }
      respond(id, true, { run })
      return
    }
    case "get_report": {
      const report = reports[params?.runRef]
      if (!report) {
        respond(id, false, undefined, { code: "run_not_found", message: "No report." })
        return
      }
      respond(id, true, { report })
      return
    }
    case "get_receipt": {
      const receipt = receipts[params?.runRef]
      if (!receipt) {
        respond(id, false, undefined, { code: "run_not_found", message: "No receipt." })
        return
      }
      respond(id, true, { receipt })
      return
    }
    default:
      respond(id, false, undefined, { code: "unsupported", message: `Unsupported ${method}.` })
  }
})
