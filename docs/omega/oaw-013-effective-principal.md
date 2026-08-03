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
color, text, and accessibility label carry the same state. Conflicting active
rows, a switching lifecycle, and registry read failures fail closed as
Unverified and do not display either candidate identity.

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
and transaction semantics. omega#218 stays open until an explicit real
membership is provisioned, all real scope consumers execute the clear
transaction, an installed enrolled identity and Organization render correctly,
local/offline/degraded states pass, and a multi-Organization switch demonstrates
complete isolation across windows and restart.
