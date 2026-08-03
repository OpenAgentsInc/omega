# Omega repository work claims

Omega uses the OpenAgents All Work service as the one writer for repository
collision coordination. The native ledger has two related records:

- A **Work Packet** declares one bounded implementation scope, repository and
  Work identity, owned paths, hot files, hot contracts, and exact verification.
- A **Repository Work Claim** records the admitted holder, generation, claim
  and last-evidence times, evidence, state, and explicit release facts.

These records do not replace Work accountability or execution records. A claim
does not assign a person, delegate an agent, create a lease or process, prove
active work, permit a merge, verify a result, release a product, or record
owner acceptance.

## Command path

The development v0.2.0 Work Inspector reads `RepositoryClaimLedger` through
the generated Rust client. Its controls submit generated commands only:

1. **Claim packet** creates the issue-scoped packet if it does not exist, then
   claims that packet.
2. **Status** appends a bounded status record and evidence reference.
3. **Heartbeat** moves the last-evidence time without proving a live process.
4. **Block** keeps the claim owned and marks the packet blocked.
5. **Release** records the releaser, release time, and release evidence.
6. **Refresh** reads canonical state without writing it.

The client reads the latest revision before a command. The command carries
that expected revision, the exact claim generation, an idempotency key, the
local Effective Principal, and `capability:repository-claim:write`. It updates
the Inspector only from the returned canonical ledger. A conflict, stale
generation, capability refusal, service loss, or malformed response stays
visible as an error and does not synthesize success.

The command receipt must say `githubWriteCount: 0`. This enforces the cutover:
GitHub issue comments are no longer a writable claim authority. Historical
claim comments can be imported with source links and completeness or loss
facts, but their packets are canceled and inert.

## Collision and takeover rules

Before a claim mutation, the authority compares same-Work identity, declared
paths and hot contracts, and classified hot surfaces such as generated
artifacts, migrations, route tables, lockfiles, and shared schemas. A refusal
names the conflicting public claim reference without disclosing a local path
or private detail. Non-conflicting packets can proceed at the same time.

Elapsed time does not transfer a claim. Stale takeover needs both at least 90
minutes without evidence and a named process/worktree audit that found no
active work. The current claim generation fences late status and heartbeat
commands after release or takeover.

## Deferred final proof

The implementation and tests are authored, but their execution is deferred to
the single final omega#208 build gate. The close gate for omega#224 also needs
the two-client concurrency and staleness suite, signed projection checks from
OpenAgents #9304, a GitHub-unavailable smoke, and one real repository change
claimed, verified, landed, and released only through this ledger.
