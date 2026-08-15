# Artifact provenance and service identity boundaries

- Status: analysis / plan of record proposal
- Date: 2026-08-15

## 1. Purpose

This note proposes, for owner review, a boundary between artifact identity and
actor identity: artifacts are identified by what they are, actors are
identified by who is accountable for them. It answers Bazaar open decision #4
(key management for the native market lane) from the artifact side; the related
analysis is
[2026-08-07-bazaar-integration.md](2026-08-07-bazaar-integration.md), which
remains the plan of record for the market integration. This note is the
design-notes companion that separates artifact provenance from the identities
that sign and operate artifacts. Decisions rest with the owners; every
consequential claim below carries an [E], [D], or [P] tag.

## 2. Claim categories

- `[E]` Existing system behavior
- `[D]` Existing documented direction
- `[P]` Proposed future direction

[Taxonomy rule 4](taxonomy.md), "Say what is true now, not what is planned,"
is why every consequential claim is tagged. A target stated as a fact is how a
brief becomes wrong; this note is a proposal, so proposed direction is always
labeled as such and verified absences are marked.

## 3. The problem: artifact identity vs actor identity

Giving every artifact its own durable Nostr keypair is the intuitive answer
and, for the properties this proposal is after, the wrong one. The four
failure modes below are the analysis behind the central recommendation in
section 4 ([P]).

- **A private key bundled with code is copied by every downloader.** A key
  shipped inside a package is not a secret and provides no provenance: anyone
  holding a copy can sign as the artifact. It creates an impersonation
  surface, not a provenance anchor.
- **A freshly generated install key identifies the installation, not the
  original artifact.** Two installations of the same artifact hold different
  keys, so the key cannot be verified against the artifact, and each copy
  accumulates its own signature history.
- **A host-held key is really the host's or operator's key.** If the host
  signs as the artifact, the honest label is the operator's key. Presenting
  it as the artifact's key hides who actually acted.
- **Reputation attaches to the wrong thing.** Reputation follows the key. If
  the key belongs to a copyable artifact, an arbitrary fork or redeployment
  inherits reputation it did not earn, and the accountable actor becomes
  unreachable.

Against Omega today there is no artifact-key story to preserve ([E]): skills
are local `SKILL.md` files loaded from two local directories, with no remote
registry and no key material in a Skill
([E]; crates/agent_skills/README.md). Omega's `OmegaPlugin` is a statically
linked crate family compiled into the binary ([E]; crates/plugin_api), not a
distributable package. The authentication contract keeps four principals
distinct, Person account, Device identity, Agent identity, and Hosted user
([D]; nostr-authentication-contract.md). Actors already have durable identity
machinery; artifacts have none and should not need it. The recommended
boundary is to keep the two apart: content addressing and signatures for
artifacts, durable keys for actors.

## 4. Central recommendation

[P] Do not give every Skill, Plugin package, or local MCP instance its own
durable Nostr keypair by default.

- Use content-addressed artifacts plus publisher signatures for Skills and
  Plugin packages. [P]
- Use durable Nostr identities for actors that make ongoing claims, operate
  services, accept jobs, receive payments, or accrue reputation. [P]

Two vocabulary points, stated once here and used throughout.

- "Plugin package" is the general term for the conceptual model of a
  distributable artifact; it maps to the artifact Omega calls an extension, a
  WASM package whose manifest is `extension.toml` ([E]; the extensions/
  directory and crates/extension_host). It never means Omega's `OmegaPlugin`,
  a statically linked crate family compiled into the binary and registered in
  `PluginRegistry` at startup ([E]; crates/plugin_api).
- "Publisher" means a person-account role; it is new vocabulary in Omega,
  proposed here [P].

The recommendation is consistent with the identity contract, which gives each
admitted agent a separate Nostr identity under AUTH-09 ([D]). Actors already
receive durable keys; the proposal extends the same reasoning to artifacts,
never in reverse: no artifact receives a durable keypair, and no key signs as
an artifact that a person, agent, or operator could sign as instead.

## 5. Artifact identity vs actor identity

The conceptual model, proposed direction throughout [P]. "Durable Nostr
keypair default" means full secret and public key possession: "Yes" is
reserved for actors that sign over time; "No" means the entity is identified
without holding a key. In the table, DVM is the NIP-90 Data Vending Machine
job model.

| Entity                         | Primary identifier                                        | Durable Nostr keypair default             | Reason                                                                                     |
| ------------------------------ | --------------------------------------------------------- | ----------------------------------------- | ------------------------------------------------------------------------------------------ |
| Human, organization, publisher | Nostr pubkey                                              | Yes                                       | Signs releases, attestations, listings, and payout instructions                            |
| Agent or NIP-90 DVM operator   | Nostr pubkey                                              | Yes                                       | Accepts work, signs results, earns feedback/reputation, and receives payment               |
| Skill                          | Content digest + version + publisher signature            | No                                        | Context/instruction artifact; it does not independently act over time                      |
| Plugin package                 | Content digest + version + signed manifest                | No                                        | Immutable executable artifact; digest identifies exactly what ran                          |
| Installed Plugin instance      | Installation record + package digest                      | No                                        | Local copy, not an independent principal                                                   |
| Local MCP server               | Installer/user/agent identity + signed package provenance | No                                        | Local adapter operating under the user's authority                                         |
| Remote, paid MCP service       | Service-operator pubkey + endpoint/deployment identity    | Conditional / Yes when public and durable | Makes ongoing service, pricing, privacy, availability, and output claims                   |
| NIP-90 DVM provider            | Operator/service pubkey                                   | Yes                                       | Marketplace participant: accepts jobs, returns results, settles payment, receives feedback |

An artifact identifies what ran; an actor identifies who is accountable.

## 6. Ten rules

The rules below turn the recommendation into operational shape. Each rule is
[P] unless it cites [E] or [D].

1. A durable Nostr key identifies a continuing actor, not merely an artifact
   with a name. Keypair generation is cheap; accountability over time is not,
   and that is what a durable key should buy. [P]

2. Skills are context artifacts. They should normally have a canonical content
   hash, an exact version, an author or publisher pubkey, and optional signed
   attestations, not a private key embedded in the Skill. [P] Today a Skill is
   a local `SKILL.md` with no key material and no remote registry ([E]).

3. Plugin packages are already best identified by exact version plus a digest
   over their manifest, schemas, and WASM module. Their publisher or author
   signs the release; the Plugin package itself should not possess a private
   key. [P]

4. Plugin self-keypairs are harmful or misleading for three reasons. (a) A
   private key bundled with code is copied by every downloader; it signs
   nothing a forger cannot also sign. (b) A newly generated install key
   identifies the installation, not the original Plugin; it cannot be verified
   against the artifact. (c) A host-held key is really the host's or
   operator's key and must be described honestly as such. [P]

5. Paid Plugin execution should produce a signed receipt. [P] The receipt
   references: Plugin name and exact version; package or content digest; input
   and output digests; a capability log digest where applicable; the executing
   host or service operator identity; and an optional publisher or author
   payout allocation reference.

6. Local MCP servers should ordinarily inherit the installing user's or
   calling agent's authority and should not gain public reputation identities
   by default. [P]

7. A durable Nostr service identity becomes appropriate for an MCP deployment
   when it is a public or persistent remote service that has one or more of:
   multiple users or agents; stateful operation; L-402 (the Lightning payment
   challenge) collection; a public rate card or service terms; privacy,
   availability, correctness, or SLA claims; reputation that should survive
   endpoint or deployment rotation; NIP-90 DVM participation. [P]

8. NIP-90 reputation and feedback should attach to the DVM provider or
   operator identity, not to a generic MCP definition and not directly to a
   Plugin package. A good operator's reputation must not automatically
   transfer to an arbitrary fork or deployment of the same code. [P] Kind 7000
   already exists in Omega as the NIP-90 feedback carrier for community
   independent-verification events ([E]; `COMMUNITY_FEEDBACK_KIND` in
   crates/workroom_receipts); it is not a DVM job carrier, and Omega
   implements no DVM jobs ([E]).

9. If L-402 is later adopted, payment challenges and receipts should be bound
   to the remote service operator identity, while Plugin author revenue shares
   remain rules in signed listings and receipts, not evidence that a Plugin
   package itself is an economic actor. [P]

10. Prefer signed links over key proliferation: publisher -> artifact digest;
    operator -> remote endpoint and deployment; DVM operator -> NIP-90 offer,
    result, and feedback; agent -> request and payment authority. [P] Each
    link binds one durable key to exactly what that key is accountable for.

## 7. Non-goals / out of scope

This proposal explicitly does not cover:

- No per-artifact durable keys by default. [P]
- No Nostr identity implementation; no identity code, storage, or contract
  changes.
- No registry. No remote skill registry, plugin registry, or listing service
  is proposed; skills load only from local directories today ([E]).
- No L-402 implementation.
- No NIP-90 or DVM implementation.
- No MCP changes; local context servers and MCP tooling are untouched.
- No plugin marketplace.
- No change to the frozen authentication contract (AUTH-00 through AUTH-09) or
  to the four principals.
- No new event kinds, NIP numbers, or config schemas anywhere in this
  proposal.

To state the negatives accurately: Omega does not implement NIP-90/DVM jobs,
L-402, or an active plugin marketplace today ([E] verified by search of the
tree; the inherited Zed extension-registry client is disabled by default under
the `OMEGA_ALLOW_ZED_SERVICES` gate). Every mention of those in this note is
proposed future direction [P] or verified absence [E].

## 8. Open design questions

1. What is the exact Nostr registry or listing binding for public Plugin
   distribution? Skills have no remote registry today ([E]); the future
   mechanism for publishing and discovering a plugin package is undecided.
2. How are author, publisher, and operator identities and payout splits
   expressed? "Publisher" is introduced here as a person-account role [P]; the
   listing and receipt grammar for identity and payout references is open.
3. How are service keys delegated and rotated for remote MCP operators?
   Device grants (AUTH-08) and agent attestations (AUTH-09) are the documented
   precedents [D]; service-scoped delegation and rotation are undecided.
4. Is L-402 admitted for paid remote services, and if so, when? Omega has no
   L-402 today ([E]); admission, scope, and sequencing are open.

## 9. Related documents and evidence

- [taxonomy.md](taxonomy.md): glossary and naming rules; rule 4 is the claim
  discipline this note follows
- [nostr-authentication-contract.md](nostr-authentication-contract.md): four
  principals; AUTH-00 frozen, AUTH-01 through AUTH-09 implemented
- [2026-08-07-bazaar-integration.md](2026-08-07-bazaar-integration.md): Bazaar
  and Immortal market analysis; open decision #4 is key management for the
  native lane
- [2026-08-09-market-ui-component-inventory.md](2026-08-09-market-ui-component-inventory.md):
  market UI planning inventory
- [../../crates/agent_skills/README.md](../../crates/agent_skills/README.md):
  skills discovery; no remote registry, no user-configured paths
- [../../crates/plugin_api/src/plugin_api.rs](../../crates/plugin_api/src/plugin_api.rs):
  the OmegaPlugin registry; a statically linked crate family, not a package

Evidence cited in the text: kind 7000 exists as the NIP-90 feedback carrier in
`crates/workroom_receipts/src/community_verification.rs`
(`COMMUNITY_FEEDBACK_KIND`) [E]. There is no NIP-90/DVM job implementation and
no L-402 implementation in the repo ([E] verified by search of the tree).
