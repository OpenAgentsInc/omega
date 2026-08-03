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
let allWorkVersion = "omega-effectd.v1"
const supportedAllWorkCapabilities = [
  "work.index.read",
  "work.snapshot.read",
  "planning.graph.read",
  "repository.claim.read",
  "repository.claim.execute",
  "workroom.activity.read",
  "workroom.activity.enqueue",
]
let allWorkCapabilities = []
let claimLedger = {
  contractVersion: "openagents.all_work_boundary.v1",
  revision: 0,
  eventCursor: "cursor:repository-claim:0",
  packets: [],
  claims: [],
  audit: [],
  completeness: { state: "complete", cursor: "cursor:repository-claim:0", gapRefs: [] },
  freshness: { state: "fresh", observedAt: "2026-08-03T05:00:00Z" },
}
let signedWorkroomLedger = {
  contractVersion: "openagents.all_work_boundary.v1",
  revision: 0,
  eventCursor: "cursor:signed-workroom:0",
  activities: [],
  outbox: [],
  completeness: { state: "complete", cursor: "cursor:signed-workroom:0", gapRefs: [] },
  freshness: { state: "fresh", observedAt: "2026-08-03T10:00:00Z" },
}

const allWorkSummary = {
  contractVersion: "openagents.all_work_boundary.v1",
  workRef: "work:fixture:1",
  title: "Fixture Work",
  domain: "general",
  workClass: "run",
  state: "active",
  priority: "normal",
  ownerRef: "principal:omega:owner",
  assignee: null,
  sourceAuthority: {
    kind: "effect_service",
    sourceRef: "run:fixture:1",
    adapterVersion: "omega-effectd-all-work-v1",
    writable: false,
  },
  revision: 1,
  updatedAt: "2026-08-02T12:00:00Z",
  freshness: {
    state: "fresh",
    observedAt: "2026-08-02T12:00:01Z",
    sourceUpdatedAt: "2026-08-02T12:00:00Z",
  },
  completeness: {
    state: "complete",
    cursor: "cursor:fixture:1",
    gapRefs: [],
  },
  redaction: {
    privacyClass: "owner_only",
    redactedFieldCount: 2,
    policyRef: "policy:omega:full-auto-work-summary-v1",
  },
}

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

// SARAH-NR-06 mock conversation store (in-memory; no Khala Sync).
const SARAH_CONVERSATION_DIGEST = "aaaaaaaaaaaaaaaaaaaaaaaa"
const SARAH_CONVERSATION_REF = `sarah.${SARAH_CONVERSATION_DIGEST}`
const SARAH_LEGACY_THREAD_REF = `thread.sarah.${SARAH_CONVERSATION_DIGEST}`
const sarahStore = {
  messages: [],
  messageSeq: 0,
  runState: "idle",
  activeTurnRef: null,
}

const sarahRoomState = () => ({
  method: "sarah_room_state",
  connection: "connected",
  freshness: "fresh",
  gapState: "none",
  connectedRelays: ["mock://local"],
  lastAcknowledgedEventId: sarahStore.messages.at(-1)?.eventId ?? null,
  lastAcknowledgedCursor:
    sarahStore.messages.length > 0
      ? `cursor.${sarahStore.messages.length - 1}`
      : null,
  authenticated: true,
  transport: "mock_relay",
})

const sarahSessionStatus = () => ({
  signedIn: true,
  accountLabel: "owner@example.com",
  bindingState: "bound",
  ownerPublicKeyHex: "b".repeat(64),
  bindingExpiresAt: null,
  transport: "mock_relay",
})

const sarahBootstrap = () => ({
  principalRef: "principal.sarah",
  displayName: "Sarah",
  role: "owner_orchestrator",
  conversationRef: SARAH_CONVERSATION_REF,
  legacyThreadRef: SARAH_LEGACY_THREAD_REF,
  ownerPublicKeyHex: "b".repeat(64),
  sarahPublicKeyHex: "c".repeat(64),
  authorityProfileRef: "docs/authority/SARAH_AUTHORITY.md",
  authorityProfileRevision: 7,
  roomState: sarahRoomState(),
})

const sarahSendMessage = (text) => {
  sarahStore.messageSeq += 1
  const turnRef = `turn.${sarahStore.messageSeq}`
  const messageRef = `msg.${sarahStore.messageSeq}`
  const eventId = `evt.msg.${sarahStore.messageSeq}`
  const cursor = `cursor.${sarahStore.messages.length}`
  sarahStore.activeTurnRef = turnRef
  sarahStore.runState = "running"
  sarahStore.messages.push({
    eventId,
    cursor,
    role: "owner",
    kind: "text",
    text: String(text).slice(0, 512),
    createdAt: new Date().toISOString(),
    status: "accepted",
    turnRef,
  })
  return {
    accepted: true,
    messageRef,
    turnRef,
    eventId,
    cursor,
    status: "accepted",
  }
}

const sarahRoomSnapshot = (params) => {
  const limit = Math.min(Math.max(Number(params.limit) || 32, 1), 64)
  const entries = sarahStore.messages.slice(-limit).map((row) => ({
    eventId: row.eventId,
    cursor: row.cursor,
    role: row.role,
    kind: row.kind,
    text: row.text,
    createdAt: row.createdAt,
    status: row.status,
  }))
  const cursor =
    entries.at(-1)?.cursor ?? params.cursor ?? "cursor.start"
  return {
    conversationRef: SARAH_CONVERSATION_REF,
    transcript: {
      entries,
      cursor,
      nextCursor: null,
      gapState: "none",
    },
    activity: {
      entries: [],
      cursor,
      nextCursor: null,
      gapState: "none",
    },
    runState: {
      state: sarahStore.runState,
      turnRef: sarahStore.activeTurnRef,
      reason: null,
    },
    roomState: sarahRoomState(),
  }
}

const sarahInterruptTurn = (turnRef) => {
  sarahStore.runState = "interrupt_pending"
  return {
    accepted: true,
    turnRef,
    intentRef: `intent.interrupt.${sarahStore.messageSeq + 1}`,
    status: "pending",
    pending: true,
  }
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
    startedAt: run.startedAt ?? run.createdAt ?? null,
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
    allWorkVersion = request.params?.allWork?.supportedVersions?.includes("omega-effectd.v2")
      ? "omega-effectd.v2"
      : "omega-effectd.v1"
    allWorkCapabilities =
      allWorkVersion === "omega-effectd.v2"
        ? supportedAllWorkCapabilities.filter((capability) =>
            request.params?.allWork?.requestedCapabilities?.includes(capability),
          )
        : []
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
        "sarah_session_status",
        "sarah_bootstrap",
        "sarah_room_snapshot",
        "sarah_send_message",
        "sarah_interrupt_turn",
        ...allWorkCapabilities,
      ],
      allWork: {
        selectedVersion: allWorkVersion,
        contractRef: "openagents.all_work_boundary.v1",
        contractDigest: "e504e3084007e8bddab99e9703f1f62c6bf62e1aa7a9f612de75fb522848b628",
        capabilities: allWorkCapabilities,
      },
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
  if (
    request.method === "work.index.read" ||
    request.method === "work.snapshot.read" ||
    request.method === "planning.graph.read"
  ) {
    if (allWorkVersion !== "omega-effectd.v2") {
      respond(request.id, generation, false, undefined, {
        code: "incompatible_version",
        message: `${request.method} requires omega-effectd.v2.`,
      })
      continue
    }
    if (request.method === "work.index.read") {
      respond(request.id, generation, true, {
        items: [allWorkSummary],
        nextCursor: null,
        completeness: { state: "complete", cursor: null, gapRefs: [] },
        generatedAt: "2026-08-02T12:00:01Z",
      })
      continue
    }
    if (request.method === "planning.graph.read") {
      respond(request.id, generation, true, {
        graph: {
          contractVersion: "openagents.all_work_boundary.v1",
          graphRef: "planning-graph:fixture",
          revision: 1,
          eventCursor: "cursor:planning:1",
          reconciliationDigest:
            "f41f9e8b44f95936694c74799027fa78b9e35ffe102a1a85e4b86027bb15748b",
          generatedAt: "2026-08-03T05:00:00Z",
          resources: [],
          work: [],
          planningLinks: [],
          labelLinks: [],
          textRecords: [],
          releaseScopeLinks: [],
          sourceCoordinates: [],
          projectionIssues: [],
          completeness: { state: "complete", cursor: "cursor:planning:1", gapRefs: [] },
          freshness: { state: "fresh", observedAt: "2026-08-03T05:00:00Z" },
        },
      })
      continue
    }
    if (request.params?.workRef !== allWorkSummary.workRef) {
      respond(request.id, generation, false, undefined, {
        code: "not_found",
        message: "No Work snapshot exists for that Work reference.",
      })
      continue
    }
    respond(request.id, generation, true, {
      snapshot: {
        summary: allWorkSummary,
        relations: [],
        threadRefs: ["thread:fixture:1"],
        sessionRefs: [],
        agentSessionRefs: [],
        agentActivityRefs: [],
        runRefs: ["run:fixture:1"],
        intentRefs: [],
        eventRefs: [],
        receiptRefs: [],
        evidenceRefs: [],
        verificationRefs: [],
        ownerDispositionRefs: [],
      },
    })
    continue
  }
  if (request.method === "repository.claim.read") {
    if (!allWorkCapabilities.includes("repository.claim.read")) {
      respond(request.id, generation, false, undefined, {
        code: "incompatible_version",
        message: "repository.claim.read was not negotiated.",
      })
      continue
    }
    respond(request.id, generation, true, { ledger: claimLedger })
    continue
  }
  if (request.method === "repository.claim.execute") {
    if (!allWorkCapabilities.includes("repository.claim.execute")) {
      respond(request.id, generation, false, undefined, {
        code: "incompatible_version",
        message: "repository.claim.execute was not negotiated.",
      })
      continue
    }
    const input = request.params
    const command = input?.command
    const previousRevision = claimLedger.revision
    const revision = previousRevision + 1
    let claimRef = null
    if (command?.command === "create_packet") {
      claimLedger.packets.push({
        packetRef: command.packetRef,
        workRef: command.workRef,
        repositoryRef: command.repositoryRef,
        title: command.title,
        scope: command.scope,
        ownedPaths: command.ownedPaths,
        hotFiles: command.hotFiles,
        hotContracts: command.hotContracts,
        verification: command.verification,
        state: "ready",
        revision: 1,
        createdAt: input.occurredAt,
        updatedAt: input.occurredAt,
      })
      claimLedger.audit.push({
        eventRef: `claim-event:${revision}`,
        kind: "packet_created",
        packetRef: command.packetRef,
        claimRef: null,
        principalRef: input.effectivePrincipalRef,
        generation: 0,
        occurredAt: input.occurredAt,
        evidenceRefs: [],
        detail: "Fixture Work Packet created without claim authority.",
      })
    } else if (command?.command === "claim_packet") {
      const packet = claimLedger.packets.find((value) => value.packetRef === command.packetRef)
      if (!packet) {
        respond(request.id, generation, false, undefined, {
          code: "not_found",
          message: "Fixture Work Packet not found.",
        })
        continue
      }
      claimRef = command.claimRef
      packet.state = "claimed"
      packet.revision += 1
      packet.updatedAt = input.occurredAt
      claimLedger.claims.push({
        claimRef,
        packetRef: packet.packetRef,
        workRef: packet.workRef,
        repositoryRef: packet.repositoryRef,
        holderRef: input.effectivePrincipalRef,
        scope: packet.scope,
        ownedPaths: packet.ownedPaths,
        hotFiles: packet.hotFiles,
        hotContracts: packet.hotContracts,
        claimedAt: input.occurredAt,
        lastEvidenceAt: input.occurredAt,
        evidenceRefs: [],
        state: "claimed",
        generation: 1,
        revision: 1,
        releasedAt: null,
        releaserRef: null,
        releaseEvidenceRefs: [],
      })
      claimLedger.audit.push({
        eventRef: `claim-event:${revision}`,
        kind: "claimed",
        packetRef: packet.packetRef,
        claimRef,
        principalRef: input.effectivePrincipalRef,
        generation: 1,
        occurredAt: input.occurredAt,
        evidenceRefs: [],
        detail: "Fixture Repository Work Claim admitted.",
      })
    } else if (["status", "heartbeat", "block", "release"].includes(command?.command)) {
      const claim = claimLedger.claims.find((value) => value.claimRef === command.claimRef)
      if (!claim || claim.generation !== command.expectedGeneration) {
        respond(request.id, generation, false, undefined, {
          code: "stale_generation",
          message: "Fixture Repository Work Claim generation does not match.",
        })
        continue
      }
      if (!["claimed", "blocked"].includes(claim.state)) {
        respond(request.id, generation, false, undefined, {
          code: "conflict",
          message: "Fixture Repository Work Claim is not active.",
        })
        continue
      }
      const packet = claimLedger.packets.find((value) => value.packetRef === claim.packetRef)
      claimRef = claim.claimRef
      claim.lastEvidenceAt = input.occurredAt
      claim.evidenceRefs = [...claim.evidenceRefs, ...(command.evidenceRefs ?? [])]
      claim.revision += 1
      if (command.command === "block") {
        claim.state = "blocked"
        if (packet) packet.state = "blocked"
      } else if (command.command === "release") {
        claim.state = "released"
        claim.releasedAt = input.occurredAt
        claim.releaserRef = input.effectivePrincipalRef
        claim.releaseEvidenceRefs = command.evidenceRefs ?? []
        if (packet) packet.state = "released"
      }
      if (packet) {
        packet.revision += 1
        packet.updatedAt = input.occurredAt
      }
      claimLedger.audit.push({
        eventRef: `claim-event:${revision}`,
        kind: command.command,
        packetRef: claim.packetRef,
        claimRef,
        principalRef: input.effectivePrincipalRef,
        generation: claim.generation,
        occurredAt: input.occurredAt,
        evidenceRefs: command.evidenceRefs ?? [],
        detail: command.detail ?? `Fixture ${command.command} accepted.`,
      })
    } else {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "Unsupported fixture claim command.",
      })
      continue
    }
    claimLedger = {
      ...claimLedger,
      revision,
      eventCursor: `cursor:repository-claim:${revision}`,
      completeness: {
        state: "complete",
        cursor: `cursor:repository-claim:${revision}`,
        gapRefs: [],
      },
      freshness: { state: "fresh", observedAt: input.occurredAt },
    }
    respond(request.id, generation, true, {
      ledger: claimLedger,
      receipt: {
        requestRef: input.requestRef,
        idempotencyKey: input.idempotencyKey,
        commandDigest: "0".repeat(64),
        previousRevision,
        revision,
        eventCursor: claimLedger.eventCursor,
        effectivePrincipalRef: input.effectivePrincipalRef,
        claimRef,
        acceptedAt: input.occurredAt,
        admitted: true,
        refusalReason: null,
        githubWriteCount: 0,
      },
    })
    continue
  }
  if (request.method === "workroom.activity.read") {
    if (!allWorkCapabilities.includes("workroom.activity.read")) {
      respond(request.id, generation, false, undefined, {
        code: "incompatible_version",
        message: "workroom.activity.read was not negotiated.",
      })
      continue
    }
    respond(request.id, generation, true, { ledger: signedWorkroomLedger })
    continue
  }
  if (request.method === "workroom.activity.enqueue") {
    if (!allWorkCapabilities.includes("workroom.activity.enqueue")) {
      respond(request.id, generation, false, undefined, {
        code: "incompatible_version",
        message: "workroom.activity.enqueue was not negotiated.",
      })
      continue
    }
    const input = request.params
    const previousRevision = signedWorkroomLedger.revision
    const revision = previousRevision + 1
    const eventCursor = `cursor:signed-workroom:${revision}`
    signedWorkroomLedger = {
      ...signedWorkroomLedger,
      revision,
      eventCursor,
      activities: [...signedWorkroomLedger.activities, input.activity],
      outbox: [...signedWorkroomLedger.outbox, {
        activity: input.activity,
        canonicalPersistedAt: input.activity.occurredAt,
        state: "pending",
        relayUrls: input.relayUrls,
        acceptedRelayUrls: [],
        attemptCount: 0,
        lastAttemptAt: null,
        lastError: null,
      }],
      completeness: { state: "complete", cursor: eventCursor, gapRefs: [] },
      freshness: { state: "fresh", observedAt: input.activity.occurredAt },
    }
    respond(request.id, generation, true, {
      ledger: signedWorkroomLedger,
      receipt: {
        idempotencyKey: input.idempotencyKey,
        previousRevision,
        revision,
        eventCursor,
        eventRef: input.activity.eventRef,
        persistedBeforePublish: true,
        relayAcceptanceIsAuthority: false,
        admittedEffect: false,
      },
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
      accounts: [
        { accountRef: "account.codex.fixture", provider: "openai", label: "ChatGPT fixture", state: "ready", quotaState: "available", lane: "codex-local" },
        { accountRef: "account.claude.fixture", provider: "anthropic", label: "Claude fixture", state: "ready", quotaState: "available", lane: "claude-local" },
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
    const dedupKey = `${run.runRef}:${state}:${run.stallCause ?? "none"}`
    if (request.params?.previousDedupKey === dedupKey) {
      respond(request.id, generation, true, { attention: null })
      continue
    }
    respond(request.id, generation, true, {
      attention: {
        notify: request.params?.permissionGranted === true,
        dedupKey,
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
        evidence: {
          objectiveRef: `objective.${run.runRef}`,
          turnRef: `turn.${run.runRef}.latest`,
          changeRef: `change.${run.runRef}.latest`,
          projectGeneration: "project.generation.fixture.1",
          diffSummary: "2 files changed, 18 insertions, 3 deletions",
          testCommand: "cargo test -p full_auto_ui",
          testOutcome: "passed",
          verificationRef: `verification.host.${run.runRef}.latest`,
          hostExecuted: true,
        },
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
        objectiveRef: `objective.${run.runRef}`,
        turnRef: `turn.${run.runRef}.latest`,
        changeRef: `change.${run.runRef}.latest`,
        verificationRef: `verification.host.${run.runRef}.latest`,
        authorityReceiptRef: `receipt.authority.${run.runRef}.latest`,
        decisionRef: `decision.authority.${run.runRef}.latest`,
        allowed: true,
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
  // SARAH-NR-06: Nostr conversation methods (mock transport; no Khala Sync).
  if (request.method === "sarah_session_status") {
    respond(request.id, generation, true, sarahSessionStatus())
    continue
  }
  if (request.method === "sarah_bootstrap") {
    respond(request.id, generation, true, sarahBootstrap())
    continue
  }
  if (request.method === "sarah_room_snapshot") {
    respond(request.id, generation, true, sarahRoomSnapshot(request.params ?? {}))
    continue
  }
  if (request.method === "sarah_send_message") {
    const text = request.params?.text
    if (typeof text !== "string" || text.trim().length === 0) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "sarah_send_message requires text.",
      })
      continue
    }
    if (/bearer\s|sk-|authorization:/i.test(text)) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "message must not carry raw credentials.",
      })
      continue
    }
    respond(request.id, generation, true, sarahSendMessage(text))
    continue
  }
  if (request.method === "sarah_interrupt_turn") {
    const turnRef = request.params?.turnRef
    if (typeof turnRef !== "string" || turnRef.trim().length === 0) {
      respond(request.id, generation, false, undefined, {
        code: "invalid_request",
        message: "sarah_interrupt_turn requires turnRef.",
      })
      continue
    }
    respond(request.id, generation, true, sarahInterruptTurn(turnRef))
    continue
  }
  respond(request.id, generation, false, undefined, {
    code: "unknown_method",
    message: `Unknown method ${request.method}.`,
  })
}
