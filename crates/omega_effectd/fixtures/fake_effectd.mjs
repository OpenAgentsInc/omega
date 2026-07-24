#!/usr/bin/env node
/**
 * Minimal omega-effectd framed-protocol fixture for Rust supervisor tests.
 * Speaks openagents.omega.effectd.v1 on stdin/stdout.
 */
import { createInterface } from "node:readline"
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs"
import path from "node:path"

const schema = "openagents.omega.effectd.v1"
const oversizedHealthResponse = process.argv.includes("--oversized-health-response")
const hostRequestHealth = process.argv.includes("--host-request-health")
const unavailableHostRequestHealth = process.argv.includes("--unavailable-host-request-health")
const staleHostRequestHealth = process.argv.includes("--stale-host-request-health")
const staleHealthResponse = process.argv.includes("--stale-health-response")
const dataRoot = process.env.OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT
if (!dataRoot) {
  console.error("OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT is required")
  process.exit(2)
}

const runsFile = path.join(dataRoot, "full-auto", "runs.json")
const bindingsFile = path.join(dataRoot, "full-auto", "native-bindings.json")
const agentComputerSessionsFile = path.join(dataRoot, "agent-computer", "sessions.json")
mkdirSync(path.join(dataRoot, "full-auto"), { recursive: true, mode: 0o700 })
mkdirSync(path.join(dataRoot, "agent-computer"), { recursive: true, mode: 0o700 })

let generation = 0
let running = false
let pendingHostHealth = null

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

const loadRuns = () => {
  if (!existsSync(runsFile)) return []
  try {
    const parsed = JSON.parse(readFileSync(runsFile, "utf8"))
    return Array.isArray(parsed.runs) ? parsed.runs : []
  } catch {
    return []
  }
}

const saveRuns = (runs) => {
  writeFileSync(
    runsFile,
    JSON.stringify(
      { schema: "openagents.desktop.full_auto_run_registry.v1", runs },
      null,
      2,
    ),
  )
}

const loadBindings = () => {
  if (!existsSync(bindingsFile)) return []
  try {
    const parsed = JSON.parse(readFileSync(bindingsFile, "utf8"))
    return Array.isArray(parsed.bindings) ? parsed.bindings : []
  } catch {
    return []
  }
}

const saveBindings = (bindings) => {
  writeFileSync(
    bindingsFile,
    JSON.stringify(
      {
        schema: "openagents.omega.full_auto_native_binding.v1",
        bindings,
      },
      null,
      2,
    ),
  )
}

const loadAgentComputerSessions = () => {
  if (!existsSync(agentComputerSessionsFile)) return []
  try {
    const parsed = JSON.parse(readFileSync(agentComputerSessionsFile, "utf8"))
    return Array.isArray(parsed.sessions) ? parsed.sessions : []
  } catch {
    return []
  }
}

const saveAgentComputerSessions = (sessions) => {
  writeFileSync(
    agentComputerSessionsFile,
    JSON.stringify(
      {
        schema: "openagents.omega.agent_computer_session.v1",
        sessions,
      },
      null,
      2,
    ),
  )
}

const projectAgentComputerSession = (params, state = "queued") => {
  const startedAt = new Date().toISOString()
  return {
    sessionRef: `ccs.fixture.${Date.now().toString(36)}`,
    environment: "openagents_cloud",
    controlPlaneBaseUrl: params.controlPlaneBaseUrl,
    repoRef: params.repoRef,
    objectiveDigest: "fixture-objective-digest",
    state,
    adapter: params.adapter ?? "codex",
    lane: params.lane ?? "cloud-gcp",
    placementRef: "placement.fixture.ac01",
    artifactRef: state === "completed" ? "artifact.fixture.ac01" : null,
    agentComputerRef: "agentcomputer.fixture.ac01",
    agentComputerState: state === "completed" ? "reclaimed" : "active",
    startedAt,
    updatedAt: startedAt,
  }
}

const listRuns = () =>
  loadRuns().map((run) => ({
    runRef: run.runRef,
    threadRef: run.threadRef ?? null,
    state: run.state,
    title: run.title,
    updatedAt: run.updatedAt,
  }))

const healthResult = () => ({
  ok: true,
  status: running ? "running" : "stopped",
  generation,
  dataRoot,
  activeRunCount: listRuns().filter((run) =>
    ["running", "pausing", "paused", "retrying", "stalled"].includes(run.state),
  ).length,
})

const detail = (run) => {
  const binding = loadBindings().find((row) => row.runRef === run.runRef) ?? null
  return {
    runRef: run.runRef,
    threadRef: run.threadRef ?? null,
    state: run.state,
    title: run.title,
    objective: run.objective ?? "",
    doneCondition: run.doneCondition ?? "",
    workspaceRef: run.workspaceRef ?? null,
    lane: run.lane ?? "codex-local",
    turnCap: run.turnCap ?? 40,
    successfulAttempts: run.successfulAttempts ?? 0,
    failedAttempts: run.failedAttempts ?? 0,
    stallCause: run.stallCause ?? null,
    recoveryAction: run.recoveryAction ?? "none",
    terminalReason: run.terminalReason ?? null,
    updatedAt: run.updatedAt,
    turns: run.turns ?? [],
    nativeEvidence: binding
      ? {
          projectRef: binding.projectRef,
          worktreeRef: binding.worktreeRef,
          worktreePathDigest: binding.worktreePathDigest ?? null,
          gitHead: binding.gitHead ?? null,
        }
      : null,
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
  if (request.kind === "host_response" && pendingHostHealth !== null) {
    const pending = pendingHostHealth
    pendingHostHealth = null
    const expectedError = pending.mode === "stale" ? "stale_generation" : "unavailable"
    const accepted =
      request.id === pending.hostId &&
      request.generation === pending.hostGeneration &&
      (pending.mode === "success"
        ? request.ok === true && request.result?.workspaceRef === "workspace.omega.supervised"
        : request.ok === false && request.error?.code === expectedError)
    if (!accepted) {
      respond(pending.request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "Host response did not match the fixture contract.",
      })
      continue
    }
    respond(pending.request.id, generation, true, healthResult())
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
      capabilities: [
        "health",
        "list_runs",
        "get_run",
        "start",
        "pause",
        "resume",
        "handoff",
        "stop",
        "retry",
        "get_capacity",
        "decide_attention",
        "get_report",
        "get_receipt",
        "apply_control_intent",
        "get_sync_status",
        "publish_projection",
        "get_native_binding",
        "assess_native_boundary",
        "start_agent_computer_session",
        "refresh_agent_computer_session",
        "run_agent_computer_turn",
        "get_agent_computer_session",
        "list_agent_computer_sessions",
      ],
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
    if (oversizedHealthResponse) {
      respond(request.id, generation, true, {
        padding: "x".repeat(64 * 1024),
      })
      continue
    }
    if (hostRequestHealth || unavailableHostRequestHealth || staleHostRequestHealth) {
      const mode = staleHostRequestHealth
        ? "stale"
        : unavailableHostRequestHealth
          ? "unavailable"
          : "success"
      const hostGeneration = mode === "stale" ? generation - 1 : generation
      const hostId = `host.${hostGeneration}.fixture`
      pendingHostHealth = { request, hostId, hostGeneration, mode }
      process.stdout.write(
        `${JSON.stringify({
          schema,
          kind: "host_request",
          id: hostId,
          generation: hostGeneration,
          method: "resolve_workspace",
          params: { expectedWorkspaceRef: "workspace.omega.supervised" },
        })}\n`,
      )
      continue
    }
    respond(request.id, staleHealthResponse ? generation - 1 : generation, true, healthResult())
    continue
  }
  if (request.method === "list_runs") {
    respond(request.id, generation, true, { runs: listRuns() })
    continue
  }
  if (request.method === "get_run") {
    const run = loadRuns().find((row) => row.runRef === request.params?.runRef)
    if (!run) {
      respond(request.id, generation, false, undefined, {
        code: "run_not_found",
        message: "No Full Auto run exists for that runRef.",
      })
      continue
    }
    respond(request.id, generation, true, { run: detail(run) })
    continue
  }
  if (request.method === "start") {
    const params = request.params ?? {}
    if (!params.workspaceRef || !params.title || !params.objective || !params.doneCondition) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "start requires workspaceRef, title, objective, and doneCondition.",
      })
      continue
    }
    if (params.rebaseUnsafe === true) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "rebase_unsafe: refusing to start Full Auto on a rebase-unsafe worktree.",
      })
      continue
    }
    const now = new Date().toISOString()
    const run = {
      runRef: `run.full-auto.fixture.${Date.now().toString(36)}`,
      threadRef: `thread.omega.${Date.now().toString(36)}`,
      state: "running",
      title: params.title,
      objective: params.objective,
      doneCondition: params.doneCondition,
      workspaceRef: params.workspaceRef,
      lane: params.lane ?? "codex-local",
      turnCap: params.turnCap ?? 40,
      successfulAttempts: 0,
      failedAttempts: 0,
      stallCause: null,
      recoveryAction: "none",
      terminalReason: null,
      updatedAt: now,
      turns: [],
    }
    const runs = loadRuns()
    runs.push(run)
    saveRuns(runs)
    if (params.projectRef && params.worktreeRef) {
      const bindings = loadBindings()
      bindings.push({
        runRef: run.runRef,
        workspaceRef: params.workspaceRef,
        projectRef: params.projectRef,
        worktreeRef: params.worktreeRef,
        worktreePathDigest: params.worktreePathDigest ?? null,
        gitHead: params.gitHead ?? null,
        rebaseUnsafe: false,
        boundAt: now,
      })
      saveBindings(bindings)
    }
    respond(request.id, generation, true, { run: detail(run) })
    continue
  }
  if (["pause", "resume", "stop", "retry"].includes(request.method)) {
    const runs = loadRuns()
    const index = runs.findIndex((row) => row.runRef === request.params?.runRef)
    if (index < 0) {
      respond(request.id, generation, false, undefined, {
        code: "run_not_found",
        message: "No Full Auto run exists for that runRef.",
      })
      continue
    }
    const nextState =
      request.method === "pause"
        ? "paused"
        : request.method === "resume"
          ? "running"
          : request.method === "stop"
            ? "stopped"
            : "retrying"
    runs[index] = {
      ...runs[index],
      state: nextState,
      updatedAt: new Date().toISOString(),
    }
    saveRuns(runs)
    respond(request.id, generation, true, { run: detail(runs[index]) })
    continue
  }
  if (request.method === "handoff") {
    const runs = loadRuns()
    const index = runs.findIndex((row) => row.runRef === request.params?.runRef)
    if (index < 0) {
      respond(request.id, generation, false, undefined, {
        code: "run_not_found",
        message: "No Full Auto run exists for that runRef.",
      })
      continue
    }
    if (runs[index].state !== "paused") {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "A provider handoff is legal only while paused.",
      })
      continue
    }
    const targetLaneRef = request.params?.targetLaneRef
    if (!["codex-local", "claude-local"].includes(targetLaneRef)) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "That provider lane is not registered.",
      })
      continue
    }
    runs[index] = {
      ...runs[index],
      lane: targetLaneRef,
      updatedAt: new Date().toISOString(),
    }
    saveRuns(runs)
    respond(request.id, generation, true, {
      run: detail(runs[index]),
      transition: {
        from: targetLaneRef === "claude-local" ? "codex-local" : "claude-local",
        to: targetLaneRef,
        disposition: "complete_within_bounds",
      },
    })
    continue
  }
  if (request.method === "get_capacity") {
    respond(request.id, generation, true, {
      activeRunLimit: 8,
      activeRunCount: listRuns().filter((run) =>
        ["running", "pausing", "paused", "retrying", "stalled"].includes(run.state),
      ).length,
      lanes: [
        { lane: "codex-local", state: "available", activeRuns: 0, reason: "ready and idle" },
        { lane: "claude-local", state: "available", activeRuns: 0, reason: "ready and idle" },
      ],
      nonOverridableGuardrails: [
        "workspace_binding",
        "own_capacity_only",
        "no_rate_limit_reset_triggering",
      ],
      ownerConfigurableGuardrails: [
        "maxWallClockMs",
        "maxTurns",
        "maxPerTurnFailures",
        "tokenBudgetRef",
      ],
      enabledThreadsNeverEvicted: true,
    })
    continue
  }
  if (request.method === "decide_attention") {
    const run = loadRuns().find((row) => row.runRef === request.params?.runRef)
    if (!run) {
      respond(request.id, generation, false, undefined, {
        code: "run_not_found",
        message: "No Full Auto run exists for that runRef.",
      })
      continue
    }
    const state = run.state
    if (state !== "stalled" && state !== "retrying") {
      respond(request.id, generation, true, { attention: null })
      continue
    }
    respond(request.id, generation, true, {
      attention: {
        notify: request.params?.permissionGranted === true,
        dedupKey: `${run.runRef}:${state}:${run.stallCause ?? "none"}`,
        title: `Full Auto ${state}`,
        body: `${run.title} needs attention (${state}).`,
      },
    })
    continue
  }
  if (request.method === "get_report") {
    const run = loadRuns().find((row) => row.runRef === request.params?.runRef)
    if (!run) {
      respond(request.id, generation, false, undefined, {
        code: "run_not_found",
        message: "No Full Auto run exists for that runRef.",
      })
      continue
    }
    respond(request.id, generation, true, {
      report: {
        schema: "openagents.desktop.full_auto_run_report.v1",
        runRef: run.runRef,
        title: run.title,
        objective: run.objective,
        doneCondition: run.doneCondition,
        state: run.state,
        turns: run.turns ?? [],
      },
    })
    continue
  }
  if (request.method === "get_receipt") {
    const run = loadRuns().find((row) => row.runRef === request.params?.runRef)
    if (!run) {
      respond(request.id, generation, false, undefined, {
        code: "run_not_found",
        message: "No Full Auto run exists for that runRef.",
      })
      continue
    }
    respond(request.id, generation, true, {
      receipt: {
        schema: "openagents.desktop.full_auto_run_receipt.v1",
        runRef: run.runRef,
        objectiveDigest: "fixture-objective-digest",
        doneConditionDigest: "fixture-done-digest",
        objectiveRevisionCount: 1,
        turnCount: (run.turns ?? []).length,
        state: run.state,
      },
    })
    continue
  }
  if (request.method === "apply_control_intent") {
    const params = request.params ?? {}
    if (!params.intentId || !params.runRef || !["pause", "resume", "stop"].includes(params.action)) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "apply_control_intent requires intentId, runRef, and action pause|resume|stop.",
      })
      continue
    }
    const runs = loadRuns()
    const index = runs.findIndex((row) => row.runRef === params.runRef)
    if (index < 0) {
      respond(request.id, generation, true, {
        outcome: {
          intentId: params.intentId,
          status: "rejected",
          rejectionReason: "run_not_found",
        },
      })
      continue
    }
    const nextState =
      params.action === "pause" ? "paused" : params.action === "resume" ? "running" : "stopped"
    runs[index] = { ...runs[index], state: nextState, updatedAt: new Date().toISOString() }
    saveRuns(runs)
    respond(request.id, generation, true, {
      outcome: {
        intentId: params.intentId,
        status: "applied",
        resultLifecycleState: nextState,
      },
    })
    continue
  }
  if (request.method === "get_sync_status") {
    respond(request.id, generation, true, {
      available: false,
      publishBlocksDispatch: false,
      reason: "omega_khala_sync_session_unavailable",
    })
    continue
  }
  if (request.method === "publish_projection") {
    respond(request.id, generation, true, {
      ok: false,
      status: "sync_unavailable",
      reason: "omega_khala_sync_session_unavailable",
    })
    continue
  }
  if (request.method === "get_native_binding") {
    const run = loadRuns().find((row) => row.runRef === request.params?.runRef)
    if (!run) {
      respond(request.id, generation, false, undefined, {
        code: "run_not_found",
        message: "No Full Auto run exists for that runRef.",
      })
      continue
    }
    const binding = loadBindings().find((row) => row.runRef === run.runRef) ?? null
    respond(request.id, generation, true, { binding })
    continue
  }
  if (request.method === "assess_native_boundary") {
    const run = loadRuns().find((row) => row.runRef === request.params?.runRef)
    if (!run) {
      respond(request.id, generation, false, undefined, {
        code: "run_not_found",
        message: "No Full Auto run exists for that runRef.",
      })
      continue
    }
    const binding = loadBindings().find((row) => row.runRef === run.runRef) ?? null
    if (!binding) {
      respond(request.id, generation, true, {
        assessment: {
          ok: false,
          reason: "missing_binding",
          message: "No native project/worktree binding exists for this Full Auto run.",
        },
      })
      continue
    }
    if (binding.workspaceRef !== run.workspaceRef) {
      respond(request.id, generation, true, {
        assessment: {
          ok: false,
          reason: "workspace_mismatch",
          message: "The bound workspace does not match the currently resolved workspace.",
        },
      })
      continue
    }
    if (binding.rebaseUnsafe) {
      respond(request.id, generation, true, {
        assessment: {
          ok: false,
          reason: "rebase_unsafe",
          message: "The bound worktree is rebase-unsafe; Full Auto refuses to continue.",
        },
      })
      continue
    }
    respond(request.id, generation, true, {
      assessment: {
        ok: true,
        evidence: {
          projectRef: binding.projectRef,
          worktreeRef: binding.worktreeRef,
          worktreePathDigest: binding.worktreePathDigest ?? null,
          gitHead: binding.gitHead ?? null,
        },
      },
    })
    continue
  }
  if (request.method === "start_agent_computer_session") {
    const params = request.params ?? {}
    if (
      typeof params.bearerToken !== "string" ||
      typeof params.controlPlaneBaseUrl !== "string" ||
      typeof params.repoRef !== "string" ||
      typeof params.objective !== "string"
    ) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message:
          "start_agent_computer_session requires bearerToken, controlPlaneBaseUrl, repoRef, and objective.",
      })
      continue
    }
    const sessions = loadAgentComputerSessions()
    const session = projectAgentComputerSession(params, "queued")
    sessions.push(session)
    saveAgentComputerSessions(sessions)
    respond(request.id, generation, true, { session })
    continue
  }
  if (request.method === "refresh_agent_computer_session") {
    const params = request.params ?? {}
    const sessions = loadAgentComputerSessions()
    const index = sessions.findIndex((row) => row.sessionRef === params.sessionRef)
    if (index === -1) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "No Agent Computer session exists for that sessionRef.",
      })
      continue
    }
    const next = {
      ...sessions[index],
      state: "running",
      updatedAt: new Date().toISOString(),
    }
    sessions[index] = next
    saveAgentComputerSessions(sessions)
    respond(request.id, generation, true, { session: next })
    continue
  }
  if (request.method === "run_agent_computer_turn") {
    const params = request.params ?? {}
    if (
      typeof params.bearerToken !== "string" ||
      typeof params.controlPlaneBaseUrl !== "string" ||
      typeof params.repoRef !== "string" ||
      typeof params.objective !== "string"
    ) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message:
          "run_agent_computer_turn requires bearerToken, controlPlaneBaseUrl, repoRef, and objective.",
      })
      continue
    }
    const sessions = loadAgentComputerSessions()
    const session = projectAgentComputerSession(params, "completed")
    sessions.push(session)
    saveAgentComputerSessions(sessions)
    respond(request.id, generation, true, {
      session,
      finishReason: "stop",
      eventKinds: ["turn.started", "text.delta", "turn.finished"],
    })
    continue
  }
  if (request.method === "get_agent_computer_session") {
    const session =
      loadAgentComputerSessions().find((row) => row.sessionRef === request.params?.sessionRef) ??
      null
    respond(request.id, generation, true, { session })
    continue
  }
  if (request.method === "list_agent_computer_sessions") {
    respond(request.id, generation, true, { sessions: loadAgentComputerSessions() })
    continue
  }
  respond(request.id, generation, false, undefined, {
    code: "unknown_method",
    message: `Unknown method ${request.method}.`,
  })
}
