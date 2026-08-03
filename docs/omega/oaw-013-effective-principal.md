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

The current production client has no authoritative Organization-membership or
Organization-selection projection. The v0.2.0 dogfood fixture is development
data and cannot grant membership or scope. Therefore this slice says `Local
scope`; it does not display the fixture's OpenAgents Organization in production
and does not expose a nonfunctional Organization switcher.

When the generated Work client supplies verified memberships, the next slice
must bind Organization selection to its source revision and account generation.
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

`EffectivePrincipalProjection` accepts this model as a separate optional input.
An exact verified membership can supply the Organization label and reference;
stale, revoked, or conflicting membership remains visible as degraded scope and
cannot activate Work commands. The production account poll still supplies no
membership input because no authoritative adapter exists. Omega therefore
continues to show `Local scope` and keeps Organization-dependent controls
disabled. The model is not evidence that switching has occurred.

## Evidence boundary

Unit and deterministic UI coverage can prove projection and transaction
semantics. omega#218 stays open until an authoritative membership adapter feeds
the model, all real scope consumers execute the clear transaction, an installed
enrolled identity and Organization render correctly, local/offline/degraded
states pass, and a multi-Organization switch demonstrates complete isolation
across windows and restart.
