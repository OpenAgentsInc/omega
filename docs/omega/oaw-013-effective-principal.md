# OAW-013 Effective Principal

Omega chrome must say which principal and scope are effective without
collapsing identity, membership, signer capability, or synchronization into
one optimistic account badge.

## Admitted source

The account registry owns the selected account reference and generation, the
public identity, bounded public profile, account lifecycle, and signer kind and
availability. `crates/agent_ui/src/effective_principal.rs` validates that the
dashboard has exactly one active row matching the selected account before it
projects those facts into the footer.

The footer uses a bounded display name only when the selected account's public
profile supplies one. Empty, control-character, `Anonymous`, secret-shaped,
and private-key-shaped names fall back to the public identity fingerprint. The
projection never puts an `npub`, public key, credential reference, token, or
private value in the compact label.

Lifecycle and signer availability remain separate. The visible status can be
Local, Enrolled, Offline, Signer unavailable, Revoked, or Unverified. Its icon,
color, text, and accessibility label carry the same state, and each of the six
states has its own icon shape, so color is never the only visual cue.
Conflicting active rows, a conflicted, repair-required, or switching lifecycle,
and registry read failures fail closed as Unverified and do not display either
candidate identity, its principal reference, or its signer facts.

A degraded principal is never an Organization principal. Offline, locked,
signed-out, revoked, candidate, and unsettled accounts cannot activate an
Organization scope even when a structurally exact verified membership row is
present for them.

## Organization boundary

The generated `omega-effectd.v2` client now reads the Effect-owned
Organization-membership ledger from OpenAgents commit
`38dcd750d249b5af4705becaefb12212696a1416`. The account poll asks with the
selected account reference, exact account generation, and Nostr-derived
Effective Principal. It accepts at most one complete, fresh row and then runs
the existing generation and principal fence before it projects an Organization.
An empty ledger keeps `Local scope`; an unavailable process grants nothing;
partial, stale-source, duplicate, malformed, cross-account, or cross-generation
results fail closed. The v0.2.0 dogfood fixture remains development data and
cannot grant membership or scope.

The authority starts empty. Membership provisioning is an explicit owner-local
operation in OpenAgents; this change does not invent an OpenAgents membership,
activate a scope, or expose a nonfunctional Organization switcher. Once an
explicit verified row exists, the footer can display its bounded Organization
name and the Work command context can consume its exact Organization reference.

Organization selection remains a separate operation. A future switch must bind
selection to the source revision and account generation.
A switch is one transaction: fence new reads, clear prior-scope navigation,
caches, counts, search, recents, Work, Threads, and activity, then publish the
new scope. No prior-scope row may flash while the new source hydrates. Missing,
stale, conflicting, or revoked membership keeps the new scope unavailable.

The source-neutral transaction model now lives in
`crates/agent_ui/src/organization_scope.rs`. A membership projection must name
its membership, account, account generation, principal, Organization, source
revision, display name, and verified/stale/revoked state. A switch rejects a
stale account or membership fence and cannot commit until navigation, caches,
counts, search, recents, Work, Threads, and activity each return one exact clear
receipt. Duplicate and previous-generation receipts fail closed.

`EffectivePrincipalProjection` accepts this model as a separate input from the
generated read. An exact verified membership can supply the Organization label
and reference; stale, revoked, or conflicting membership remains visible as
degraded scope and cannot activate Work commands. With the current empty
authority, Omega continues to show `Local scope` and keeps
Organization-dependent controls disabled. A successful membership read is not
evidence that switching has occurred.

## Evidence boundary

Unit and deterministic UI coverage can prove projection, generated transport,
and transaction semantics. The executed coverage in `agent_ui` now includes
every lifecycle and signer degradation, the unsettled-account refusal, the
distinct per-state cue shape, the empty/incomplete/stale/duplicate/cross-account
and cross-generation membership reads, and a leakage check that no secret-shaped
Organization name, `npub`, `nsec`, `ncryptsec`, or public key hex reaches the
visible or accessible identity. `organization_scope` covers the fenced switch
transaction, the per-consumer clear receipts, and the stale, duplicate, and
revoked refusals.

Three acceptance criteria remain unproven and are not claimed. No real
Organization membership has been provisioned, so no enrolled Organization has
rendered in installed Omega. No production consumer yet emits a
`OrganizationScopeClearReceipt`, so the switch transaction is exercised only by
its own tests. There is no installed multi-Organization, multi-window, or
restart isolation evidence. omega#218 stays open until those three exist.
