//! OMEGA-MOB-31-03 headless Full Auto contract (omega#47).
//!
//! `issue31_host` says *which* capabilities this host projects and how fresh
//! each one is, but its record references are opaque. The content behind them
//! lives in three unrelated shapes: `full_auto_ui::panel` run rows (omega#41),
//! `full_auto_ui::provider_roster` (omega#42), and
//! `full_auto_ui::evidence_chain` (omega#43). None is consumable by the phone.
//!
//! This is the one headless contract those three collapse into. It is the
//! byte-shared peer of `packages/sarah/src/issue31-workroom/full-auto-adjunct.ts`
//! in the OpenAgents repository, and it enforces the same laws so a projection
//! the TypeScript reader would refuse cannot be produced here either.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{PublicRef, sanitize_public_ref};

pub const ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA: &str = "openagents.omega.issue31.fullauto.v1";
pub const MAX_ISSUE31_FULL_AUTO_RUNS: usize = 16;
pub const MAX_ISSUE31_FULL_AUTO_ACCOUNTS: usize = 32;
pub const MAX_ISSUE31_FULL_AUTO_HANDOFFS: usize = 16;
pub const MAX_ISSUE31_FULL_AUTO_CONTROLS: usize = 8;
pub const MAX_ISSUE31_TIMESTAMP_MS: u64 = 8_640_000_000_000_000;
/// One year. An unattended duration longer than this is a projection defect.
pub const MAX_ISSUE31_UNATTENDED_MS: u64 = 31_536_000_000;

const MAX_PUBLIC_TEXT: usize = 512;
const MAX_PUBLIC_LABEL: usize = 96;
const MAX_PUBLIC_COMMAND: usize = 256;

/// The ordered omega#43 chain. The order is normative: a viewer follows one
/// finished unit from objective through authority receipt.
pub const ISSUE31_EVIDENCE_HOPS: [Issue31EvidenceHopKind; 9] = [
    Issue31EvidenceHopKind::Objective,
    Issue31EvidenceHopKind::Turn,
    Issue31EvidenceHopKind::Change,
    Issue31EvidenceHopKind::ProjectGeneration,
    Issue31EvidenceHopKind::Test,
    Issue31EvidenceHopKind::TypedOutcome,
    Issue31EvidenceHopKind::HostVerification,
    Issue31EvidenceHopKind::AuthorityDecision,
    Issue31EvidenceHopKind::Receipt,
];

const FORBIDDEN_TEXT_FRAGMENTS: [&str; 14] = [
    "bearer ",
    "authorization:",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "client_secret",
    "private_key",
    "auth.json",
    "id_rsa",
    "begin rsa",
    "begin openssh",
    "begin private key",
    "openagents_agent_token",
];

const FORBIDDEN_TEXT_PREFIXES: [&str; 8] = [
    "sk-",
    "sk_",
    "ghp_",
    "gho_",
    "github_pat_",
    "xox",
    "nsec1",
    "ncryptsec1",
];

const FORBIDDEN_PATH_FRAGMENTS: [&str; 5] =
    ["/users/", "/home/", "/var/folders/", "/private/tmp/", "~/"];

/// True when bounded owner-facing text carries no credential or private-path
/// shape. The provider boundary in omega#47 is absolute: the phone never sees a
/// token, an authorization response, a private path, or raw credential state,
/// so this rejects rather than redacts.
pub fn is_issue31_public_text(value: &str, maximum_length: usize) -> bool {
    if value.is_empty() || value.trim() != value || value.chars().count() > maximum_length {
        return false;
    }
    // Control characters can forge line structure in the owner transcript.
    if value.chars().any(|character| {
        character.is_control() && character != '\u{0009}' || character == '\u{007f}'
    }) {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if FORBIDDEN_TEXT_FRAGMENTS
        .iter()
        .any(|fragment| lower.contains(fragment))
    {
        return false;
    }
    if FORBIDDEN_TEXT_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return false;
    }
    !FORBIDDEN_PATH_FRAGMENTS
        .iter()
        .any(|fragment| lower.contains(fragment))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31FullAutoLifecycle {
    Queued,
    Running,
    Pausing,
    Paused,
    Stopping,
    /// Omega's Full Auto panel distinguishes these from a healthy run.
    /// Collapsing them into `Running` would show a stalled run as progressing.
    Retrying,
    Stalled,
    Succeeded,
    Failed,
    Stopped,
    Expired,
}

impl Issue31FullAutoLifecycle {
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Stopped | Self::Expired
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31FullAutoControlKind {
    Pause,
    Resume,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31ProviderReadiness {
    Ready,
    Busy,
    Exhausted,
    RateLimited,
    Revoked,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31ProviderQuota {
    Available,
    Cooling,
    Depleted,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31ProviderHandoffState {
    Requested,
    Active,
    Completed,
    Refused,
    Failed,
    Expired,
}

impl Issue31ProviderHandoffState {
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Refused | Self::Failed | Self::Expired
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31EvidenceHopKind {
    Objective,
    Turn,
    Change,
    ProjectGeneration,
    Test,
    TypedOutcome,
    HostVerification,
    AuthorityDecision,
    Receipt,
}

impl Issue31EvidenceHopKind {
    /// The wire token for this hop.
    ///
    /// The same string the contract serialises, exposed so a renderer can
    /// label a hop without re-deriving a name of its own.
    /// `hop_tokens_are_the_wire_tokens` pins the two together, so a rename in
    /// the contract cannot leave a surface labelling hops by an older name.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Objective => "objective",
            Self::Turn => "turn",
            Self::Change => "change",
            Self::ProjectGeneration => "project_generation",
            Self::Test => "test",
            Self::TypedOutcome => "typed_outcome",
            Self::HostVerification => "host_verification",
            Self::AuthorityDecision => "authority_decision",
            Self::Receipt => "receipt",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31EvidenceUnavailableReason {
    HopMissing,
    HopMismatched,
    HopPrivate,
    SelfReported,
    HostUnavailable,
}

impl Issue31EvidenceUnavailableReason {
    /// The wire token for this refusal.
    ///
    /// A reader must be told *why* a chain is unavailable, and the four
    /// record-level reasons are not interchangeable: `hop_missing` says the
    /// host never produced the step, `hop_mismatched` says two records tell
    /// two stories about one run, `hop_private` says the host produced it and
    /// this surface may not carry it, and `self_reported` says the run
    /// vouched for itself. Collapsing them into one "unavailable" word is how
    /// a contradicted chain gets read as merely incomplete.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::HopMissing => "hop_missing",
            Self::HopMismatched => "hop_mismatched",
            Self::HopPrivate => "hop_private",
            Self::SelfReported => "self_reported",
            Self::HostUnavailable => "host_unavailable",
        }
    }

    /// Every admitted reason, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::HopMissing,
            Self::HopMismatched,
            Self::HopPrivate,
            Self::SelfReported,
            Self::HostUnavailable,
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue31FullAutoControl {
    pub action_ref: PublicRef,
    pub kind: Issue31FullAutoControlKind,
    pub run_generation: u64,
    pub idempotency_ref: PublicRef,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue31FullAutoRun {
    pub run_ref: PublicRef,
    pub objective: String,
    pub lane_ref: PublicRef,
    pub lifecycle: Issue31FullAutoLifecycle,
    pub generation: u64,
    pub unattended_ms: u64,
    pub live_work_ref: Option<PublicRef>,
    pub terminal_reason_ref: Option<PublicRef>,
    pub controls: Vec<Issue31FullAutoControl>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue31ProviderAccount {
    pub account_ref: PublicRef,
    pub provider: PublicRef,
    pub label: String,
    pub readiness: Issue31ProviderReadiness,
    pub quota: Issue31ProviderQuota,
    pub lane_ref: PublicRef,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue31ProviderHandoff {
    pub handoff_ref: PublicRef,
    pub provider: PublicRef,
    pub state: Issue31ProviderHandoffState,
    pub requested_at_ms: u64,
    pub account_ref: Option<PublicRef>,
    pub reason_class: Option<PublicRef>,
    pub outcome_ref: Option<PublicRef>,
    pub receipt_ref: Option<PublicRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue31EvidenceHop {
    pub kind: Issue31EvidenceHopKind,
    pub reference: PublicRef,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Issue31EvidenceChain {
    Complete {
        run_ref: PublicRef,
        authority_allowed: bool,
        hops: Vec<Issue31EvidenceHop>,
    },
    Unavailable {
        run_ref: PublicRef,
        reason: Issue31EvidenceUnavailableReason,
        broken_at: Option<Issue31EvidenceHopKind>,
    },
}

impl Issue31EvidenceChain {
    #[must_use]
    pub fn run_ref(&self) -> &PublicRef {
        match self {
            Self::Complete { run_ref, .. } | Self::Unavailable { run_ref, .. } => run_ref,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue31FullAutoAdjunct {
    pub schema: &'static str,
    pub host_ref: PublicRef,
    pub snapshot_ref: PublicRef,
    pub generated_at_ms: u64,
    pub runs: Vec<Issue31FullAutoRun>,
    pub accounts: Vec<Issue31ProviderAccount>,
    pub handoffs: Vec<Issue31ProviderHandoff>,
    pub evidence: Vec<Issue31EvidenceChain>,
}

impl Issue31FullAutoAdjunct {
    /// True when this detail projection belongs to the exact `host.v1` snapshot
    /// that advertised it. A detail payload from a different snapshot is stale
    /// content wearing a current label.
    #[must_use]
    pub fn is_bound_to(&self, host_ref: &PublicRef, snapshot_ref: &PublicRef) -> bool {
        &self.host_ref == host_ref && &self.snapshot_ref == snapshot_ref
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue31FullAutoAdjunctError {
    InvalidJson,
    InvalidSchema,
    UnsafeReference,
    UnsafeText,
    BoundExceeded,
    DuplicateReference,
    InvalidTimestamp,
    InvalidRunState,
    InvalidControlBinding,
    InvalidAccountState,
    InvalidHandoffState,
    InvalidEvidenceChain,
    UnknownReference,
}

impl std::fmt::Display for Issue31FullAutoAdjunctError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidJson => "device mirror full auto adjunct is not valid contract JSON",
            Self::InvalidSchema => "device mirror full auto adjunct schema is not supported",
            Self::UnsafeReference => "device mirror full auto adjunct contains an unsafe reference",
            Self::UnsafeText => "device mirror full auto adjunct contains unsafe text",
            Self::BoundExceeded => "device mirror full auto adjunct bound was exceeded",
            Self::DuplicateReference => {
                "device mirror full auto adjunct contains a duplicate reference"
            }
            Self::InvalidTimestamp => "device mirror full auto adjunct timestamp order is invalid",
            Self::InvalidRunState => "device mirror full auto adjunct run state is invalid",
            Self::InvalidControlBinding => {
                "device mirror full auto adjunct binds a control to a stale run generation"
            }
            Self::InvalidAccountState => {
                "device mirror full auto adjunct confuses a lane with an account"
            }
            Self::InvalidHandoffState => "device mirror full auto adjunct handoff state is invalid",
            Self::InvalidEvidenceChain => {
                "device mirror full auto adjunct evidence chain is invalid"
            }
            Self::UnknownReference => "device mirror full auto adjunct points at an unknown record",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for Issue31FullAutoAdjunctError {}

type AdjunctResult<T> = Result<T, Issue31FullAutoAdjunctError>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAdjunct {
    schema: String,
    host_ref: String,
    snapshot_ref: String,
    generated_at_ms: u64,
    runs: Vec<RawRun>,
    accounts: Vec<RawAccount>,
    handoffs: Vec<RawHandoff>,
    evidence: Vec<RawEvidence>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRun {
    run_ref: String,
    objective: String,
    lane_ref: String,
    lifecycle: Issue31FullAutoLifecycle,
    generation: u64,
    unattended_ms: u64,
    live_work_ref: Option<String>,
    terminal_reason_ref: Option<String>,
    controls: Vec<RawControl>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawControl {
    action_ref: String,
    kind: Issue31FullAutoControlKind,
    run_generation: u64,
    idempotency_ref: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAccount {
    account_ref: String,
    provider: String,
    label: String,
    readiness: Issue31ProviderReadiness,
    quota: Issue31ProviderQuota,
    lane_ref: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHandoff {
    handoff_ref: String,
    provider: String,
    state: Issue31ProviderHandoffState,
    requested_at_ms: u64,
    account_ref: Option<String>,
    reason_class: Option<String>,
    outcome_ref: Option<String>,
    receipt_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "completeness", rename_all = "snake_case", deny_unknown_fields)]
enum RawEvidence {
    Complete {
        #[serde(rename = "runRef")]
        run_ref: String,
        #[serde(rename = "hostExecuted")]
        host_executed: bool,
        #[serde(rename = "authorityAllowed")]
        authority_allowed: bool,
        hops: Vec<RawHop>,
    },
    Unavailable {
        #[serde(rename = "runRef")]
        run_ref: String,
        #[serde(rename = "reasonClass")]
        reason_class: Issue31EvidenceUnavailableReason,
        #[serde(rename = "brokenAt")]
        broken_at: Option<Issue31EvidenceHopKind>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHop {
    kind: Issue31EvidenceHopKind,
    #[serde(rename = "ref")]
    reference: String,
    detail: Option<String>,
}

pub fn decode_issue31_full_auto_adjunct(input: &str) -> AdjunctResult<Issue31FullAutoAdjunct> {
    let raw: RawAdjunct =
        serde_json::from_str(input).map_err(|_| Issue31FullAutoAdjunctError::InvalidJson)?;
    if raw.schema != ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA {
        return Err(Issue31FullAutoAdjunctError::InvalidSchema);
    }
    if raw.generated_at_ms > MAX_ISSUE31_TIMESTAMP_MS {
        return Err(Issue31FullAutoAdjunctError::InvalidTimestamp);
    }
    if raw.runs.len() > MAX_ISSUE31_FULL_AUTO_RUNS
        || raw.accounts.len() > MAX_ISSUE31_FULL_AUTO_ACCOUNTS
        || raw.handoffs.len() > MAX_ISSUE31_FULL_AUTO_HANDOFFS
        || raw.evidence.len() > MAX_ISSUE31_FULL_AUTO_RUNS
    {
        return Err(Issue31FullAutoAdjunctError::BoundExceeded);
    }

    let host_ref = public_ref(raw.host_ref)?;
    let snapshot_ref = public_ref(raw.snapshot_ref)?;

    let runs = raw
        .runs
        .into_iter()
        .map(project_run)
        .collect::<AdjunctResult<Vec<_>>>()?;
    let accounts = raw
        .accounts
        .into_iter()
        .map(project_account)
        .collect::<AdjunctResult<Vec<_>>>()?;
    let handoffs = raw
        .handoffs
        .into_iter()
        .map(|handoff| project_handoff(handoff, raw.generated_at_ms))
        .collect::<AdjunctResult<Vec<_>>>()?;
    let evidence = raw
        .evidence
        .into_iter()
        .map(project_evidence)
        .collect::<AdjunctResult<Vec<_>>>()?;

    assert_unique(runs.iter().map(|run| run.run_ref.as_str()))?;
    assert_unique(accounts.iter().map(|account| account.account_ref.as_str()))?;
    assert_unique(handoffs.iter().map(|handoff| handoff.handoff_ref.as_str()))?;
    assert_unique(evidence.iter().map(|chain| chain.run_ref().as_str()))?;

    // Evidence and handoffs must point at things this snapshot actually
    // carries, otherwise the phone renders a chain for a run it cannot show.
    let known_runs: HashSet<&str> = runs.iter().map(|run| run.run_ref.as_str()).collect();
    if evidence
        .iter()
        .any(|chain| !known_runs.contains(chain.run_ref().as_str()))
    {
        return Err(Issue31FullAutoAdjunctError::UnknownReference);
    }
    let known_accounts: HashSet<&str> = accounts
        .iter()
        .map(|account| account.account_ref.as_str())
        .collect();
    if handoffs.iter().any(|handoff| {
        handoff
            .account_ref
            .as_ref()
            .is_some_and(|account| !known_accounts.contains(account.as_str()))
    }) {
        return Err(Issue31FullAutoAdjunctError::UnknownReference);
    }

    Ok(Issue31FullAutoAdjunct {
        schema: ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA,
        host_ref,
        snapshot_ref,
        generated_at_ms: raw.generated_at_ms,
        runs,
        accounts,
        handoffs,
        evidence,
    })
}

fn project_run(raw: RawRun) -> AdjunctResult<Issue31FullAutoRun> {
    if raw.controls.len() > MAX_ISSUE31_FULL_AUTO_CONTROLS {
        return Err(Issue31FullAutoAdjunctError::BoundExceeded);
    }
    if raw.unattended_ms > MAX_ISSUE31_UNATTENDED_MS {
        return Err(Issue31FullAutoAdjunctError::InvalidRunState);
    }
    if !is_issue31_public_text(&raw.objective, MAX_PUBLIC_TEXT) {
        return Err(Issue31FullAutoAdjunctError::UnsafeText);
    }

    let run_ref = public_ref(raw.run_ref)?;
    let lane_ref = public_ref(raw.lane_ref)?;
    let live_work_ref = raw.live_work_ref.map(public_ref).transpose()?;
    let terminal_reason_ref = raw.terminal_reason_ref.map(public_ref).transpose()?;

    if raw.lifecycle.is_terminal() {
        // A finished run offers no controls. Otherwise the phone can present a
        // button whose completion can never arrive.
        if live_work_ref.is_some() || terminal_reason_ref.is_none() || !raw.controls.is_empty() {
            return Err(Issue31FullAutoAdjunctError::InvalidRunState);
        }
    } else if terminal_reason_ref.is_some() {
        return Err(Issue31FullAutoAdjunctError::InvalidRunState);
    }

    let mut controls = Vec::with_capacity(raw.controls.len());
    for control in raw.controls {
        if control.run_generation != raw.generation {
            return Err(Issue31FullAutoAdjunctError::InvalidControlBinding);
        }
        controls.push(Issue31FullAutoControl {
            action_ref: public_ref(control.action_ref)?,
            kind: control.kind,
            run_generation: control.run_generation,
            idempotency_ref: public_ref(control.idempotency_ref)?,
        });
    }
    assert_unique(controls.iter().map(|control| control.action_ref.as_str()))?;
    let kinds: HashSet<Issue31FullAutoControlKind> =
        controls.iter().map(|control| control.kind).collect();
    if kinds.len() != controls.len() {
        return Err(Issue31FullAutoAdjunctError::DuplicateReference);
    }

    Ok(Issue31FullAutoRun {
        run_ref,
        objective: raw.objective,
        lane_ref,
        lifecycle: raw.lifecycle,
        generation: raw.generation,
        unattended_ms: raw.unattended_ms,
        live_work_ref,
        terminal_reason_ref,
        controls,
    })
}

fn project_account(raw: RawAccount) -> AdjunctResult<Issue31ProviderAccount> {
    if !is_issue31_public_text(&raw.label, MAX_PUBLIC_LABEL) {
        return Err(Issue31FullAutoAdjunctError::UnsafeText);
    }
    let account_ref = public_ref(raw.account_ref)?;
    let lane_ref = public_ref(raw.lane_ref)?;
    // A lane reference that is literally the account reference collapses the
    // two concepts omega#47 insists on keeping distinct.
    if account_ref == lane_ref {
        return Err(Issue31FullAutoAdjunctError::InvalidAccountState);
    }
    Ok(Issue31ProviderAccount {
        account_ref,
        provider: public_ref(raw.provider)?,
        label: raw.label,
        readiness: raw.readiness,
        quota: raw.quota,
        lane_ref,
    })
}

fn project_handoff(raw: RawHandoff, generated_at_ms: u64) -> AdjunctResult<Issue31ProviderHandoff> {
    if raw.requested_at_ms > generated_at_ms || raw.requested_at_ms > MAX_ISSUE31_TIMESTAMP_MS {
        return Err(Issue31FullAutoAdjunctError::InvalidTimestamp);
    }
    let account_ref = raw.account_ref.map(public_ref).transpose()?;
    let reason_class = raw.reason_class.map(public_ref).transpose()?;
    let outcome_ref = raw.outcome_ref.map(public_ref).transpose()?;

    if raw.state.is_terminal() {
        // The exit is "a provider connection handoff reports its exact
        // host-owned outcome". A terminal state with no host outcome reference
        // is a claim the host never made.
        if outcome_ref.is_none() {
            return Err(Issue31FullAutoAdjunctError::InvalidHandoffState);
        }
        if raw.state == Issue31ProviderHandoffState::Completed {
            if account_ref.is_none() {
                return Err(Issue31FullAutoAdjunctError::InvalidHandoffState);
            }
        } else if reason_class.is_none() {
            return Err(Issue31FullAutoAdjunctError::InvalidHandoffState);
        }
    } else if outcome_ref.is_some() {
        return Err(Issue31FullAutoAdjunctError::InvalidHandoffState);
    }

    Ok(Issue31ProviderHandoff {
        handoff_ref: public_ref(raw.handoff_ref)?,
        provider: public_ref(raw.provider)?,
        state: raw.state,
        requested_at_ms: raw.requested_at_ms,
        account_ref,
        reason_class,
        outcome_ref,
        receipt_ref: raw.receipt_ref.map(public_ref).transpose()?,
    })
}

/// Decode exactly one provider connection handoff row.
///
/// This is the same `project_handoff` the whole-document decoder applies, with
/// no second copy of the law: a producer that routes a row through this cannot
/// write a handoff the reader of the assembled adjunct would then refuse.
///
/// It deliberately does not check the one law that is not local to a row — a
/// bound `accountRef` must name an account the same snapshot carries. That
/// relation belongs to the document, and a producer holding only a row cannot
/// state it. `decode_issue31_full_auto_adjunct` still enforces it.
pub fn decode_issue31_provider_handoff(
    value: &serde_json::Value,
    generated_at_ms: u64,
) -> AdjunctResult<Issue31ProviderHandoff> {
    let raw: RawHandoff = serde_json::from_value(value.clone())
        .map_err(|_| Issue31FullAutoAdjunctError::InvalidJson)?;
    project_handoff(raw, generated_at_ms)
}

fn project_evidence(raw: RawEvidence) -> AdjunctResult<Issue31EvidenceChain> {
    match raw {
        RawEvidence::Unavailable {
            run_ref,
            reason_class,
            broken_at,
        } => Ok(Issue31EvidenceChain::Unavailable {
            run_ref: public_ref(run_ref)?,
            reason: reason_class,
            broken_at,
        }),
        RawEvidence::Complete {
            run_ref,
            host_executed,
            authority_allowed,
            hops,
        } => {
            // A run that reports its own success is self-reported, not
            // verified, so a complete chain cannot say otherwise.
            if !host_executed {
                return Err(Issue31FullAutoAdjunctError::InvalidEvidenceChain);
            }
            if hops.len() != ISSUE31_EVIDENCE_HOPS.len() {
                return Err(Issue31FullAutoAdjunctError::InvalidEvidenceChain);
            }
            let mut projected = Vec::with_capacity(hops.len());
            for (index, hop) in hops.into_iter().enumerate() {
                if hop.kind != ISSUE31_EVIDENCE_HOPS[index] {
                    return Err(Issue31FullAutoAdjunctError::InvalidEvidenceChain);
                }
                if let Some(detail) = hop.detail.as_deref()
                    && !is_issue31_public_text(detail, MAX_PUBLIC_COMMAND)
                {
                    return Err(Issue31FullAutoAdjunctError::UnsafeText);
                }
                projected.push(Issue31EvidenceHop {
                    kind: hop.kind,
                    reference: public_ref(hop.reference)?,
                    detail: hop.detail,
                });
            }
            Ok(Issue31EvidenceChain::Complete {
                run_ref: public_ref(run_ref)?,
                authority_allowed,
                hops: projected,
            })
        }
    }
}

fn assert_unique<'a>(values: impl Iterator<Item = &'a str>) -> AdjunctResult<()> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(Issue31FullAutoAdjunctError::DuplicateReference);
        }
    }
    Ok(())
}

fn public_ref(raw: String) -> AdjunctResult<PublicRef> {
    sanitize_public_ref(&raw).ok_or(Issue31FullAutoAdjunctError::UnsafeReference)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL: &str =
        include_str!("../fixtures/openagents.omega.issue31.fullauto.v1.canonical.json");

    fn negative(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(format!(
                "openagents.omega.issue31.fullauto.v1.negative-{name}.json"
            ));
        std::fs::read_to_string(path).expect("negative fixture is readable")
    }

    #[test]
    fn decodes_the_byte_shared_canonical_fixture() {
        let adjunct = decode_issue31_full_auto_adjunct(CANONICAL).expect("canonical decodes");
        assert_eq!(adjunct.schema, ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA);
        assert_eq!(adjunct.runs.len(), 2);
        assert_eq!(adjunct.accounts.len(), 3);
        assert_eq!(adjunct.handoffs.len(), 3);
        assert_eq!(adjunct.evidence.len(), 2);
    }

    /// The bytes `omega_effectd`'s handoff ledger actually emits (omega#91),
    /// byte-shared with `packages/sarah`.
    const HOST_PRODUCED_HANDOFFS: &str = include_str!(
        "../fixtures/openagents.omega.issue31.fullauto.v1.host-produced-handoffs.json"
    );

    #[test]
    fn decodes_every_host_produced_handoff_lifecycle() {
        let adjunct = decode_issue31_full_auto_adjunct(HOST_PRODUCED_HANDOFFS)
            .expect("host-produced decodes");
        let states: Vec<Issue31ProviderHandoffState> = adjunct
            .handoffs
            .iter()
            .map(|handoff| handoff.state)
            .collect();
        for state in [
            Issue31ProviderHandoffState::Requested,
            Issue31ProviderHandoffState::Active,
            Issue31ProviderHandoffState::Completed,
            Issue31ProviderHandoffState::Refused,
            Issue31ProviderHandoffState::Failed,
            Issue31ProviderHandoffState::Expired,
        ] {
            assert!(states.contains(&state), "{state:?} is unrepresented");
        }
        // A bound handoff and the account it chose are readable together, so a
        // viewer can follow the lane the host picked rather than infer it.
        let accounts: HashSet<&str> = adjunct
            .accounts
            .iter()
            .map(|account| account.account_ref.as_str())
            .collect();
        for handoff in &adjunct.handoffs {
            if let Some(account_ref) = &handoff.account_ref {
                assert!(accounts.contains(account_ref.as_str()));
            }
        }
    }

    #[test]
    fn projects_the_explicit_account_to_lane_relation() {
        let adjunct = decode_issue31_full_auto_adjunct(CANONICAL).expect("canonical decodes");
        let pairs: Vec<(&str, &str)> = adjunct
            .accounts
            .iter()
            .map(|account| (account.account_ref.as_str(), account.lane_ref.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("account.codex.1", "lane.codex-local"),
                ("account.codex.2", "lane.codex-local-2"),
                ("account.claude.1", "lane.claude-local"),
            ]
        );
    }

    #[test]
    fn keeps_every_control_bound_to_the_exact_run_generation() {
        let adjunct = decode_issue31_full_auto_adjunct(CANONICAL).expect("canonical decodes");
        for run in &adjunct.runs {
            for control in &run.controls {
                assert_eq!(control.run_generation, run.generation);
                assert!(!control.idempotency_ref.as_str().is_empty());
            }
        }
    }

    #[test]
    fn orders_a_complete_chain_from_objective_through_receipt() {
        let adjunct = decode_issue31_full_auto_adjunct(CANONICAL).expect("canonical decodes");
        let complete = adjunct
            .evidence
            .iter()
            .find_map(|chain| match chain {
                Issue31EvidenceChain::Complete { hops, .. } => Some(hops),
                Issue31EvidenceChain::Unavailable { .. } => None,
            })
            .expect("canonical carries a complete chain");
        let kinds: Vec<Issue31EvidenceHopKind> = complete.iter().map(|hop| hop.kind).collect();
        assert_eq!(kinds, ISSUE31_EVIDENCE_HOPS.to_vec());
    }

    #[test]
    fn a_broken_chain_is_unavailable_and_carries_no_partial_hops() {
        let adjunct = decode_issue31_full_auto_adjunct(CANONICAL).expect("canonical decodes");
        let broken = adjunct
            .evidence
            .iter()
            .find(|chain| matches!(chain, Issue31EvidenceChain::Unavailable { .. }))
            .expect("canonical carries an unavailable chain");
        match broken {
            Issue31EvidenceChain::Unavailable {
                reason, broken_at, ..
            } => {
                assert_eq!(*reason, Issue31EvidenceUnavailableReason::HopMissing);
                assert_eq!(*broken_at, Some(Issue31EvidenceHopKind::HostVerification));
            }
            Issue31EvidenceChain::Complete { .. } => unreachable!(),
        }
    }

    #[test]
    fn refuses_every_boundary_violation_without_echoing_the_offending_value() {
        for (name, expected) in [
            ("credential-label", Issue31FullAutoAdjunctError::UnsafeText),
            ("private-path", Issue31FullAutoAdjunctError::UnsafeText),
            (
                "stale-generation",
                Issue31FullAutoAdjunctError::InvalidControlBinding,
            ),
            (
                "lane-as-account",
                Issue31FullAutoAdjunctError::InvalidAccountState,
            ),
            (
                "partial-chain",
                Issue31FullAutoAdjunctError::InvalidEvidenceChain,
            ),
            (
                "self-reported",
                Issue31FullAutoAdjunctError::InvalidEvidenceChain,
            ),
            (
                "terminal-control",
                Issue31FullAutoAdjunctError::InvalidRunState,
            ),
            (
                "handoff-no-outcome",
                Issue31FullAutoAdjunctError::InvalidHandoffState,
            ),
        ] {
            let error = decode_issue31_full_auto_adjunct(&negative(name))
                .expect_err("negative fixture must be refused");
            assert_eq!(error, expected, "fixture {name}");
            let rendered = error.to_string();
            assert!(
                !rendered.contains("Bearer"),
                "fixture {name} echoed a token"
            );
            assert!(
                !rendered.contains("/Users/"),
                "fixture {name} echoed a path"
            );
        }
    }

    #[test]
    fn binds_only_to_the_snapshot_that_advertised_the_capabilities() {
        let adjunct = decode_issue31_full_auto_adjunct(CANONICAL).expect("canonical decodes");
        let host = sanitize_public_ref("host.omega.device-alpha").expect("safe ref");
        let snapshot = sanitize_public_ref("snapshot.omega.issue31.000042").expect("safe ref");
        let other = sanitize_public_ref("snapshot.omega.issue31.000043").expect("safe ref");
        assert!(adjunct.is_bound_to(&host, &snapshot));
        assert!(!adjunct.is_bound_to(&host, &other));
    }

    #[test]
    fn rejects_credential_and_private_path_text_directly() {
        for value in [
            "Bearer abc123",
            "sk-live-0000",
            "ghp_0000000000",
            "Authorization: token",
            "/Users/owner/.codex/auth.json",
            "~/.codex/auth.json",
            "-----BEGIN PRIVATE KEY-----",
            " leading space",
            "",
        ] {
            assert!(
                !is_issue31_public_text(value, MAX_PUBLIC_TEXT),
                "expected {value:?} to be refused"
            );
        }
        assert!(is_issue31_public_text(
            "3 files changed, 214 insertions, 6 deletions",
            MAX_PUBLIC_TEXT
        ));
    }
}

// ---------------------------------------------------------------------------
// Emitter
// ---------------------------------------------------------------------------

/// Build the adjunct from the same host records the Full Auto panels read.
///
/// The panels in `full_auto_ui` parse these values into display rows. This
/// produces the headless contract from the identical source, so the phone and
/// the panel cannot disagree about what the host said.
///
/// It deliberately builds a JSON document and hands it to
/// `decode_issue31_full_auto_adjunct` rather than constructing the typed value
/// directly. There is then exactly one place where the issue's boundaries are
/// enforced, and the emitter cannot produce anything the reader would refuse.
pub fn build_issue31_full_auto_adjunct(
    host_ref: &str,
    snapshot_ref: &str,
    generated_at_ms: u64,
    runs: &serde_json::Value,
    accounts: &serde_json::Value,
    handoffs: &serde_json::Value,
    evidence: &[(serde_json::Value, serde_json::Value)],
) -> AdjunctResult<Issue31FullAutoAdjunct> {
    build_issue31_full_auto_adjunct_document(
        host_ref,
        snapshot_ref,
        generated_at_ms,
        runs,
        accounts,
        handoffs,
        evidence,
    )
    .map(|(adjunct, _)| adjunct)
}

/// The same detail projection, plus the exact bytes the decoder accepted.
pub fn build_issue31_full_auto_adjunct_document(
    host_ref: &str,
    snapshot_ref: &str,
    generated_at_ms: u64,
    runs: &serde_json::Value,
    accounts: &serde_json::Value,
    handoffs: &serde_json::Value,
    evidence: &[(serde_json::Value, serde_json::Value)],
) -> AdjunctResult<(Issue31FullAutoAdjunct, serde_json::Value)> {
    let document = serde_json::json!({
        "schema": ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA,
        "hostRef": host_ref,
        "snapshotRef": snapshot_ref,
        "generatedAtMs": generated_at_ms,
        "runs": build_runs(runs, generated_at_ms)?,
        "accounts": build_accounts(accounts)?,
        "handoffs": build_handoffs(handoffs)?,
        "evidence": build_evidence(evidence),
    });
    let serialized =
        serde_json::to_string(&document).map_err(|_| Issue31FullAutoAdjunctError::InvalidJson)?;
    let adjunct = decode_issue31_full_auto_adjunct(&serialized)?;
    Ok((adjunct, document))
}

/// Map an Omega Full Auto panel state onto the contract lifecycle.
///
/// Unknown states are refused rather than folded into `running`. A state this
/// build does not understand is not evidence that a run is healthy.
fn lifecycle_from_state(state: &str) -> Option<&'static str> {
    Some(match state {
        "queued" => "queued",
        "running" => "running",
        "pausing" => "pausing",
        "paused" => "paused",
        "stopping" => "stopping",
        "retrying" => "retrying",
        "stalled" => "stalled",
        "succeeded" | "completed" => "succeeded",
        "failed" => "failed",
        "stopped" | "cancelled" => "stopped",
        "expired" => "expired",
        _ => return None,
    })
}

fn is_terminal_state(lifecycle: &str) -> bool {
    matches!(lifecycle, "succeeded" | "failed" | "stopped" | "expired")
}

fn build_runs(
    runs: &serde_json::Value,
    generated_at_ms: u64,
) -> AdjunctResult<Vec<serde_json::Value>> {
    let mut built = Vec::new();
    for run in runs
        .get("runs")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let state = run
            .get("state")
            .and_then(serde_json::Value::as_str)
            .ok_or(Issue31FullAutoAdjunctError::InvalidRunState)?;
        let lifecycle =
            lifecycle_from_state(state).ok_or(Issue31FullAutoAdjunctError::InvalidRunState)?;
        let terminal = is_terminal_state(lifecycle);
        let generation = run
            .get("generation")
            .and_then(serde_json::Value::as_u64)
            .ok_or(Issue31FullAutoAdjunctError::InvalidRunState)?;
        let run_ref = run
            .get("runRef")
            .and_then(serde_json::Value::as_str)
            .ok_or(Issue31FullAutoAdjunctError::UnsafeReference)?;

        // Measured from the host's own start time, never re-derived downstream.
        // A run whose start the host never recorded has no known unattended
        // duration, and `0` would read on the phone as "just started" — a claim
        // nothing supports. The exact duration is part of this issue's outcome,
        // so its absence is a refusal rather than a zero.
        let started_at_ms = run
            .get("startedAtMs")
            .and_then(serde_json::Value::as_u64)
            .ok_or(Issue31FullAutoAdjunctError::InvalidRunState)?;
        if started_at_ms > generated_at_ms {
            return Err(Issue31FullAutoAdjunctError::InvalidTimestamp);
        }
        let unattended_ms = generated_at_ms - started_at_ms;

        let mut entry = serde_json::json!({
            "runRef": run_ref,
            "objective": run.get("objective").cloned().unwrap_or(serde_json::Value::Null),
            "laneRef": run.get("laneRef").or_else(|| run.get("lane")).cloned().unwrap_or(serde_json::Value::Null),
            "lifecycle": lifecycle,
            "generation": generation,
            "unattendedMs": unattended_ms,
            "controls": build_controls(run, generation, terminal),
        });
        let object = entry
            .as_object_mut()
            .ok_or(Issue31FullAutoAdjunctError::InvalidJson)?;
        if terminal {
            // The contract requires a reason on a finished run, and a run that
            // ended without the host recording why is a gap the owner should
            // see rather than a blank.
            let reason = run
                .get("terminalReasonRef")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("reason.full-auto.unrecorded"));
            object.insert("terminalReasonRef".into(), reason);
        } else if let Some(live) = run.get("liveWorkRef").cloned() {
            object.insert("liveWorkRef".into(), live);
        }
        built.push(entry);
    }
    Ok(built)
}

/// A control the host is willing to accept, bound to this exact generation.
fn build_controls(
    run: &serde_json::Value,
    generation: u64,
    terminal: bool,
) -> Vec<serde_json::Value> {
    if terminal {
        return Vec::new();
    }
    let run_ref = run
        .get("runRef")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("run");
    run.get("permittedControls")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|kind| matches!(*kind, "pause" | "resume" | "stop"))
        .map(|kind| {
            serde_json::json!({
                "actionRef": format!("action.full-auto.{kind}"),
                "kind": kind,
                "runGeneration": generation,
                "idempotencyRef": format!("idem.{run_ref}.{kind}.{generation}"),
            })
        })
        .collect()
}

fn build_accounts(accounts: &serde_json::Value) -> AdjunctResult<Vec<serde_json::Value>> {
    let mut built = Vec::new();
    for account in accounts
        .get("accounts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        built.push(serde_json::json!({
            "accountRef": account.get("accountRef").cloned().unwrap_or(serde_json::Value::Null),
            "provider": account.get("provider").cloned().unwrap_or(serde_json::Value::Null),
            "label": account.get("label").cloned().unwrap_or(serde_json::Value::Null),
            "readiness": normalize_readiness(account.get("state").and_then(serde_json::Value::as_str)),
            "quota": normalize_quota(account.get("quotaState").and_then(serde_json::Value::as_str)),
            // The panel calls this `lane`. The contract insists an account
            // states its lane, so a missing one becomes an explicit decode
            // failure rather than a silently unmapped account row.
            "laneRef": account.get("laneRef").or_else(|| account.get("lane")).cloned().unwrap_or(serde_json::Value::Null),
        }));
    }
    Ok(built)
}

/// Project each host-owned provider connection handoff onto the contract.
///
/// The handoff record on the host side carries the isolated provider home, the
/// browser or device login it drove, and whatever the provider returned. Only
/// the fields named here are copied, so a private path or an authorization
/// response present on the host record cannot reach the phone even by accident,
/// and cannot make the whole projection unreadable through an unknown field
/// either.
fn build_handoffs(handoffs: &serde_json::Value) -> AdjunctResult<Vec<serde_json::Value>> {
    let mut built = Vec::new();
    for handoff in handoffs
        .get("handoffs")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let mut entry = serde_json::json!({
            "handoffRef": handoff.get("handoffRef").cloned().unwrap_or(serde_json::Value::Null),
            "provider": handoff.get("provider").cloned().unwrap_or(serde_json::Value::Null),
            "state": handoff.get("state").cloned().unwrap_or(serde_json::Value::Null),
            "requestedAtMs": handoff.get("requestedAtMs").cloned().unwrap_or(serde_json::Value::Null),
        });
        let object = entry
            .as_object_mut()
            .ok_or(Issue31FullAutoAdjunctError::InvalidJson)?;
        for field in ["accountRef", "reasonClass", "outcomeRef", "receiptRef"] {
            if let Some(value) = handoff.get(field)
                && !value.is_null()
            {
                object.insert(field.into(), value.clone());
            }
        }
        built.push(entry);
    }
    Ok(built)
}

fn normalize_readiness(state: Option<&str>) -> &'static str {
    match state {
        Some("ready") => "ready",
        Some("busy") => "busy",
        Some("exhausted") => "exhausted",
        Some("rate_limited") => "rate_limited",
        Some("revoked") => "revoked",
        _ => "unknown",
    }
}

fn normalize_quota(state: Option<&str>) -> &'static str {
    match state {
        Some("available") => "available",
        Some("cooling") => "cooling",
        Some("depleted") => "depleted",
        _ => "unknown",
    }
}

/// Project each (report, receipt) pair through the same rules the omega#43
/// inspector uses, and fail closed to `unavailable` when the pair does not
/// form one complete chain.
fn build_evidence(pairs: &[(serde_json::Value, serde_json::Value)]) -> Vec<serde_json::Value> {
    pairs
        .iter()
        .map(|(report, receipt)| {
            let run_ref = report
                .get("runRef")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("run.unknown");
            match complete_chain(report, receipt) {
                Some(chain) => chain,
                None => serde_json::json!({
                    "completeness": "unavailable",
                    "runRef": run_ref,
                    "reasonClass": unavailable_reason(report, receipt),
                }),
            }
        })
        .collect()
}

/// Project one `(get_report, get_receipt)` pair into the chain it forms.
///
/// The single-pair entry point to the producer `build_evidence` already runs,
/// exposed so a surface that shows one run's chain — the desktop thread link
/// in omega#80 — reads the same chain the phone reads instead of assembling a
/// second opinion. There is deliberately no second implementation here: the
/// body emits through `build_evidence` and reads the result back through
/// `project_evidence`, which is the decoder the adjunct itself goes through.
/// That is what makes the public-safety bound non-negotiable at this entry
/// point too — a hop the decoder refuses cannot be returned as a chain.
///
/// `run_ref_override` is the reference the *caller already proved* public-safe,
/// used when the pair is unavailable. `build_evidence` would otherwise echo the
/// report's own `runRef`, and a report whose reference fails the bound would
/// turn a reportable refusal into an undecodable one.
///
/// # Errors
///
/// Returns the decoder's error when the host produced a chain this surface is
/// not allowed to carry — an unbounded detail, or a hop reference outside the
/// public-reference character class. The caller's honest rendering of that is
/// [`Issue31EvidenceUnavailableReason::HopPrivate`]: the host has the step and
/// this surface may not show it.
pub fn project_issue31_evidence_pair(
    report: &serde_json::Value,
    receipt: &serde_json::Value,
    run_ref_override: Option<&str>,
) -> AdjunctResult<Issue31EvidenceChain> {
    let pair = [(report.clone(), receipt.clone())];
    let mut emitted = build_evidence(&pair)
        .pop()
        .ok_or(Issue31FullAutoAdjunctError::InvalidEvidenceChain)?;
    if let Some(run_ref) = run_ref_override
        && emitted
            .get("completeness")
            .and_then(serde_json::Value::as_str)
            == Some("unavailable")
        && let Some(object) = emitted.as_object_mut()
    {
        object.insert("runRef".into(), serde_json::json!(run_ref));
    }
    let raw: RawEvidence = serde_json::from_value(emitted)
        .map_err(|_| Issue31FullAutoAdjunctError::InvalidEvidenceChain)?;
    project_evidence(raw)
}

/// The hops the omega#43 report and the authority receipt both carry. They must
/// agree on all of them for the pair to be one chain.
const SHARED_EVIDENCE_HOP_FIELDS: [&str; 4] =
    ["objectiveRef", "turnRef", "changeRef", "verificationRef"];

fn unavailable_reason(report: &serde_json::Value, receipt: &serde_json::Value) -> &'static str {
    let Some(evidence) = report.get("evidence") else {
        return "hop_missing";
    };
    // Disagreement on any hop the two records share is two stories about one
    // run, not a hop the host failed to produce. Checking only `runRef` here
    // would report a contradicted chain as merely incomplete.
    if report.get("runRef") != receipt.get("runRef")
        || SHARED_EVIDENCE_HOP_FIELDS.iter().any(|field| {
            let reported = evidence.get(field);
            let received = receipt.get(field);
            reported.is_some() && received.is_some() && reported != received
        })
    {
        return "hop_mismatched";
    }
    if evidence
        .get("hostExecuted")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return "self_reported";
    }
    // A hop the host recorded but cannot show publicly is not a missing hop.
    // Naming it `hop_private` keeps the owner's two situations distinct: the
    // host never produced this step, versus the host produced it and this
    // surface is not allowed to carry it.
    if carries_private_hop(evidence) || carries_private_hop(receipt) {
        return "hop_private";
    }
    "hop_missing"
}

/// True when any owner-facing hop value on this record would be refused by the
/// contract's public-text rule.
fn carries_private_hop(record: &serde_json::Value) -> bool {
    record
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, value)| value.as_str())
        .any(|value| !is_issue31_public_text(value, MAX_PUBLIC_TEXT))
}

fn complete_chain(
    report: &serde_json::Value,
    receipt: &serde_json::Value,
) -> Option<serde_json::Value> {
    let run_ref = report.get("runRef")?.as_str()?;
    if receipt.get("runRef")?.as_str()? != run_ref {
        return None;
    }
    let evidence = report.get("evidence")?;
    if evidence
        .get("hostExecuted")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return None;
    }
    // A private hop makes this one chain unavailable. Letting it through to the
    // decoder instead would abort the whole projection, so a single unshowable
    // detail would hide every other run the owner is entitled to see.
    if carries_private_hop(evidence) || carries_private_hop(receipt) {
        return None;
    }
    // The receipt must agree with the report on every shared hop, or the chain
    // is two stories rather than one.
    for field in SHARED_EVIDENCE_HOP_FIELDS {
        if receipt.get(field)? != evidence.get(field)? {
            return None;
        }
    }
    let hop = |kind: &str, value: Option<&serde_json::Value>| -> Option<serde_json::Value> {
        Some(serde_json::json!({ "kind": kind, "ref": value?.as_str()? }))
    };
    let detailed =
        |kind: &str, value: Option<&serde_json::Value>, detail: Option<&serde_json::Value>| {
            let mut entry = serde_json::json!({ "kind": kind, "ref": value?.as_str()? });
            if let Some(detail) = detail.and_then(serde_json::Value::as_str) {
                entry
                    .as_object_mut()?
                    .insert("detail".into(), serde_json::json!(detail));
            }
            Some(entry)
        };

    let hops = vec![
        hop("objective", evidence.get("objectiveRef"))?,
        hop("turn", evidence.get("turnRef"))?,
        detailed(
            "change",
            evidence.get("changeRef"),
            evidence.get("diffSummary"),
        )?,
        hop("project_generation", evidence.get("projectGeneration"))?,
        detailed(
            "test",
            evidence
                .get("testRef")
                .or_else(|| evidence.get("verificationRef")),
            evidence.get("testCommand"),
        )?,
        hop("typed_outcome", evidence.get("testOutcome"))?,
        hop("host_verification", evidence.get("verificationRef"))?,
        hop("authority_decision", receipt.get("decisionRef"))?,
        hop("receipt", receipt.get("authorityReceiptRef"))?,
    ];

    Some(serde_json::json!({
        "completeness": "complete",
        "runRef": run_ref,
        "hostExecuted": true,
        "authorityAllowed": receipt.get("allowed").and_then(serde_json::Value::as_bool)?,
        "hops": hops,
    }))
}

#[cfg(test)]
mod emitter_tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_784_894_400_000;

    fn runs() -> serde_json::Value {
        json!({"runs": [
            {
                "runRef": "run.full-auto.run-01",
                "objective": "Finish the device mirror mobile workroom.",
                "lane": "lane.codex-local",
                "state": "running",
                "generation": 7,
                "startedAtMs": NOW - 5_400_000,
                "liveWorkRef": "work.run-01.unit-14",
                "permittedControls": ["pause", "stop"]
            }
        ]})
    }

    fn accounts() -> serde_json::Value {
        json!({"accounts": [
            {"accountRef":"account.codex.1","provider":"openai","label":"ChatGPT Personal","state":"busy","quotaState":"available","lane":"lane.codex-local"}
        ]})
    }

    fn build(
        runs: &serde_json::Value,
        accounts: &serde_json::Value,
        evidence: &[(serde_json::Value, serde_json::Value)],
    ) -> AdjunctResult<Issue31FullAutoAdjunct> {
        build_issue31_full_auto_adjunct(
            "host.omega.device-alpha",
            "snapshot.omega.issue31.000042",
            NOW,
            runs,
            accounts,
            &json!({}),
            evidence,
        )
    }

    #[test]
    fn builds_from_the_same_records_the_panels_read() {
        let adjunct = build(&runs(), &accounts(), &[]).expect("emits a valid adjunct");
        assert_eq!(adjunct.runs.len(), 1);
        let run = &adjunct.runs[0];
        assert_eq!(run.lifecycle, Issue31FullAutoLifecycle::Running);
        assert_eq!(run.unattended_ms, 5_400_000);
        assert_eq!(run.controls.len(), 2);
        assert!(
            run.controls
                .iter()
                .all(|control| control.run_generation == 7)
        );
        assert_eq!(adjunct.accounts[0].lane_ref.as_str(), "lane.codex-local");
    }

    #[test]
    fn refuses_a_state_this_build_does_not_understand() {
        let mut value = runs();
        value["runs"][0]["state"] = json!("vibing");
        // Folding an unknown state into `running` would report a run as healthy
        // on the strength of a string nobody has defined.
        assert_eq!(
            build(&value, &accounts(), &[]).expect_err("must refuse"),
            Issue31FullAutoAdjunctError::InvalidRunState
        );
    }

    #[test]
    fn a_stalled_run_is_not_reported_as_running() {
        let mut value = runs();
        value["runs"][0]["state"] = json!("stalled");
        let adjunct = build(&value, &accounts(), &[]).expect("stalled is a known state");
        assert_eq!(adjunct.runs[0].lifecycle, Issue31FullAutoLifecycle::Stalled);
        assert!(!adjunct.runs[0].lifecycle.is_terminal());
    }

    #[test]
    fn a_finished_run_loses_its_controls_and_gains_a_reason() {
        let mut value = runs();
        value["runs"][0]["state"] = json!("succeeded");
        let adjunct = build(&value, &accounts(), &[]).expect("emits");
        let run = &adjunct.runs[0];
        assert!(run.controls.is_empty());
        assert!(run.live_work_ref.is_none());
        assert_eq!(
            run.terminal_reason_ref.as_ref().map(PublicRef::as_str),
            Some("reason.full-auto.unrecorded")
        );
    }

    #[test]
    fn an_account_without_a_lane_is_refused_rather_than_shown_unmapped() {
        let mut value = accounts();
        value["accounts"][0]
            .as_object_mut()
            .expect("object")
            .remove("lane");
        assert!(build(&runs(), &value, &[]).is_err());
    }

    #[test]
    fn a_credential_shaped_label_cannot_be_emitted() {
        let mut value = accounts();
        value["accounts"][0]["label"] = json!("Bearer sk-live-abc");
        assert_eq!(
            build(&runs(), &value, &[]).expect_err("must refuse"),
            Issue31FullAutoAdjunctError::UnsafeText
        );
    }

    fn evidence_pair() -> (serde_json::Value, serde_json::Value) {
        (
            json!({
                "runRef": "run.full-auto.run-01",
                "evidence": {
                    "objectiveRef": "objective.run-01",
                    "turnRef": "turn.run-01.11",
                    "changeRef": "change.run-01.11",
                    "projectGeneration": "generation.project.00219",
                    "verificationRef": "verification.run-01.11",
                    "testOutcome": "outcome.test.passed",
                    "testCommand": "cargo test -p workroom_receipts",
                    "diffSummary": "3 files changed",
                    "hostExecuted": true
                }
            }),
            json!({
                "runRef": "run.full-auto.run-01",
                "objectiveRef": "objective.run-01",
                "turnRef": "turn.run-01.11",
                "changeRef": "change.run-01.11",
                "verificationRef": "verification.run-01.11",
                "decisionRef": "decision.run-01.11",
                "authorityReceiptRef": "receipt.run-01.11",
                "allowed": true
            }),
        )
    }

    #[test]
    fn projects_one_complete_chain_in_the_normative_order() {
        let adjunct = build(&runs(), &accounts(), &[evidence_pair()]).expect("emits");
        match &adjunct.evidence[0] {
            Issue31EvidenceChain::Complete { hops, .. } => {
                let kinds: Vec<Issue31EvidenceHopKind> = hops.iter().map(|hop| hop.kind).collect();
                assert_eq!(kinds, ISSUE31_EVIDENCE_HOPS.to_vec());
            }
            Issue31EvidenceChain::Unavailable { .. } => panic!("expected a complete chain"),
        }
    }

    #[test]
    fn a_self_reported_chain_fails_closed_to_unavailable() {
        let (mut report, receipt) = evidence_pair();
        report["evidence"]["hostExecuted"] = json!(false);
        let adjunct = build(&runs(), &accounts(), &[(report, receipt)]).expect("emits");
        match &adjunct.evidence[0] {
            Issue31EvidenceChain::Unavailable { reason, .. } => {
                assert_eq!(*reason, Issue31EvidenceUnavailableReason::SelfReported);
            }
            Issue31EvidenceChain::Complete { .. } => {
                panic!("a run reporting its own success is not verified")
            }
        }
    }

    #[test]
    fn a_receipt_that_disagrees_with_the_report_fails_closed() {
        let (report, mut receipt) = evidence_pair();
        receipt["changeRef"] = json!("change.someone-elses-work");
        let adjunct = build(&runs(), &accounts(), &[(report, receipt)]).expect("emits");
        // Two stories about one run is not a chain.
        assert!(matches!(
            adjunct.evidence[0],
            Issue31EvidenceChain::Unavailable { .. }
        ));
    }

    fn unavailable_reason_of(adjunct: &Issue31FullAutoAdjunct) -> Issue31EvidenceUnavailableReason {
        match &adjunct.evidence[0] {
            Issue31EvidenceChain::Unavailable { reason, .. } => *reason,
            Issue31EvidenceChain::Complete { .. } => {
                panic!("a broken chain must never render as complete")
            }
        }
    }

    /// Falsification of every way omega#47 says a chain can be a story rather
    /// than a proof. Each is watched refusing: a fail-closed path nobody saw
    /// refuse is not proven to fail closed.
    #[test]
    fn each_broken_evidence_hop_is_refused_with_its_own_reason() {
        let (mut report, receipt) = evidence_pair();
        report["evidence"]
            .as_object_mut()
            .expect("evidence object")
            .remove("turnRef");
        let missing = build(&runs(), &accounts(), &[(report, receipt)]).expect("emits");
        assert_eq!(
            unavailable_reason_of(&missing),
            Issue31EvidenceUnavailableReason::HopMissing
        );

        let (report, mut receipt) = evidence_pair();
        receipt["turnRef"] = json!("turn.some-other-run.3");
        let mismatched = build(&runs(), &accounts(), &[(report, receipt)]).expect("emits");
        assert_eq!(
            unavailable_reason_of(&mismatched),
            Issue31EvidenceUnavailableReason::HopMismatched
        );

        let (mut report, receipt) = evidence_pair();
        report["evidence"]["testCommand"] = json!("cat /Users/owner/.codex/auth.json");
        let private = build(&runs(), &accounts(), &[(report, receipt)]).expect("emits");
        assert_eq!(
            unavailable_reason_of(&private),
            Issue31EvidenceUnavailableReason::HopPrivate
        );

        let (mut report, receipt) = evidence_pair();
        report["evidence"]["hostExecuted"] = json!(false);
        let self_reported = build(&runs(), &accounts(), &[(report, receipt)]).expect("emits");
        assert_eq!(
            unavailable_reason_of(&self_reported),
            Issue31EvidenceUnavailableReason::SelfReported
        );
    }

    #[test]
    fn one_unshowable_hop_never_hides_the_runs_beside_it() {
        let (mut report, receipt) = evidence_pair();
        report["evidence"]["diffSummary"] = json!("wrote /Users/owner/.codex/auth.json");
        let adjunct = build(&runs(), &accounts(), &[(report, receipt)]).expect(
            "a private hop makes one chain unavailable, it does not abort the whole projection",
        );
        assert_eq!(adjunct.runs.len(), 1);
        assert_eq!(
            unavailable_reason_of(&adjunct),
            Issue31EvidenceUnavailableReason::HopPrivate
        );
    }

    #[test]
    fn a_run_whose_start_the_host_never_recorded_is_refused_not_shown_as_zero() {
        let mut value = runs();
        value["runs"][0]
            .as_object_mut()
            .expect("run object")
            .remove("startedAtMs");
        // The exact unattended duration is part of this issue's outcome. Zero
        // would read on the phone as "just started".
        assert_eq!(
            build(&value, &accounts(), &[]).expect_err("must refuse"),
            Issue31FullAutoAdjunctError::InvalidRunState
        );
    }

    #[test]
    fn a_terminal_handoff_without_a_host_outcome_cannot_be_emitted() {
        let handoffs = json!({"handoffs": [{
            "handoffRef": "handoff.codex.1",
            "provider": "openai",
            "state": "completed",
            "requestedAtMs": NOW - 60_000,
            "accountRef": "account.codex.1"
        }]});
        let error = build_issue31_full_auto_adjunct(
            "host.omega.device-alpha",
            "snapshot.omega.issue31.000042",
            NOW,
            &runs(),
            &accounts(),
            &handoffs,
            &[],
        )
        .expect_err("a completed handoff with no host outcome is a claim the host never made");
        assert_eq!(error, Issue31FullAutoAdjunctError::InvalidHandoffState);
    }

    #[test]
    fn a_private_field_on_a_host_handoff_record_never_reaches_the_contract() {
        let handoffs = json!({"handoffs": [{
            "handoffRef": "handoff.codex.1",
            "provider": "openai",
            "state": "completed",
            "requestedAtMs": NOW - 60_000,
            "accountRef": "account.codex.1",
            "outcomeRef": "outcome.handoff.linked",
            // The host record legitimately holds these. The phone never may.
            "providerHome": "/Users/owner/.pylon/accounts/codex/codex-2",
            "authorizationResponse": "Bearer sk-live-0000"
        }]});
        let adjunct = build_issue31_full_auto_adjunct(
            "host.omega.device-alpha",
            "snapshot.omega.issue31.000042",
            NOW,
            &runs(),
            &accounts(),
            &handoffs,
            &[],
        )
        .expect("admitted fields are projected, private ones are not carried");
        assert_eq!(adjunct.handoffs.len(), 1);
        let encoded = serde_json::to_string(&serde_json::json!({
            "handoffRef": adjunct.handoffs[0].handoff_ref.as_str(),
            "outcomeRef": adjunct.handoffs[0].outcome_ref.as_ref().map(PublicRef::as_str),
        }))
        .expect("encodes");
        assert!(!encoded.contains("/Users/"));
        assert!(!encoded.contains("Bearer"));
    }

    /// The tokens a renderer labels hops with are the tokens the contract
    /// serialises.
    ///
    /// A second name for the same hop is how a desktop surface and a phone
    /// end up describing one chain differently, so the two are compared
    /// rather than trusted to have been written consistently.
    #[test]
    fn hop_tokens_are_the_wire_tokens() {
        for kind in ISSUE31_EVIDENCE_HOPS {
            let wire = serde_json::to_value(kind).expect("hop kind serialises");
            assert_eq!(wire.as_str(), Some(kind.token()), "{kind:?}");
        }
        for reason in Issue31EvidenceUnavailableReason::all() {
            let wire = serde_json::to_value(reason).expect("reason serialises");
            assert_eq!(wire.as_str(), Some(reason.token()), "{reason:?}");
        }
        assert_eq!(
            Issue31EvidenceUnavailableReason::all().len(),
            5,
            "the reason vocabulary is byte-shared with packages/sarah. Adding \
             a sixth reason is a contract change, not a rendering convenience."
        );
    }

    /// One pair projects to the same chain through the single-pair entry point
    /// as through the whole adjunct.
    ///
    /// The point of the entry point is that omega#80's thread link reads the
    /// chain the phone reads. A second implementation would satisfy every
    /// other test in this file and still disagree here.
    #[test]
    fn the_single_pair_entry_point_agrees_with_the_adjunct() {
        let (report, receipt) = evidence_pair();
        let through_adjunct = build(&runs(), &accounts(), &[(report.clone(), receipt.clone())])
            .expect("emits")
            .evidence
            .remove(0);
        let direct = project_issue31_evidence_pair(&report, &receipt, None).expect("projects");
        assert_eq!(direct, through_adjunct);
        assert!(matches!(direct, Issue31EvidenceChain::Complete { .. }));
    }

    /// Every refusal reason survives the single-pair entry point, named.
    ///
    /// Falsified one at a time: a surface that reports "unavailable" without
    /// the reason has told the reader nothing they can act on, and a
    /// contradicted chain reported as merely incomplete is the specific bug
    /// this vocabulary exists to prevent.
    #[test]
    fn the_single_pair_entry_point_names_each_refusal() {
        let reason_of = |report: &serde_json::Value, receipt: &serde_json::Value| {
            match project_issue31_evidence_pair(report, receipt, None).expect("projects") {
                Issue31EvidenceChain::Unavailable { reason, .. } => reason,
                Issue31EvidenceChain::Complete { .. } => {
                    panic!("a broken chain must never render as complete")
                }
            }
        };

        let (mut report, receipt) = evidence_pair();
        report["evidence"]
            .as_object_mut()
            .expect("evidence object")
            .remove("changeRef");
        assert_eq!(
            reason_of(&report, &receipt),
            Issue31EvidenceUnavailableReason::HopMissing
        );

        let (report, mut receipt) = evidence_pair();
        receipt["changeRef"] = json!("change.someone-elses-work");
        assert_eq!(
            reason_of(&report, &receipt),
            Issue31EvidenceUnavailableReason::HopMismatched
        );

        let (mut report, receipt) = evidence_pair();
        report["evidence"]["testCommand"] = json!("cat /Users/owner/.codex/auth.json");
        assert_eq!(
            reason_of(&report, &receipt),
            Issue31EvidenceUnavailableReason::HopPrivate
        );

        let (mut report, receipt) = evidence_pair();
        report["evidence"]["hostExecuted"] = json!(false);
        assert_eq!(
            reason_of(&report, &receipt),
            Issue31EvidenceUnavailableReason::SelfReported
        );
    }

    /// A report whose own `runRef` is not public-safe still produces a
    /// reportable refusal, because the caller supplies the reference it
    /// already proved.
    ///
    /// Without the override this pair decodes to nothing at all, and a surface
    /// with nothing to render shows the reader an empty space where a refusal
    /// belongs.
    #[test]
    fn an_unsafe_report_reference_still_yields_a_named_refusal() {
        let (mut report, mut receipt) = evidence_pair();
        // Both records agree on the reference, so this is not a mismatch. The
        // host simply names the run in a way this surface may not repeat.
        report["runRef"] = json!("/Users/owner/runs/17");
        receipt["runRef"] = json!("/Users/owner/runs/17");
        report["evidence"]["hostExecuted"] = json!(false);

        assert_eq!(
            project_issue31_evidence_pair(&report, &receipt, None),
            Err(Issue31FullAutoAdjunctError::UnsafeReference),
            "without a proven reference there is nothing safe to name the \
             refusal after"
        );

        let named = project_issue31_evidence_pair(&report, &receipt, Some("run.full-auto.run-01"))
            .expect("the caller's proven reference names the refusal");
        match named {
            Issue31EvidenceChain::Unavailable {
                run_ref, reason, ..
            } => {
                assert_eq!(run_ref.as_str(), "run.full-auto.run-01");
                assert_eq!(reason, Issue31EvidenceUnavailableReason::SelfReported);
            }
            Issue31EvidenceChain::Complete { .. } => panic!("self-reported is not complete"),
        }
    }

    /// The override never rewrites a complete chain's identity.
    ///
    /// If it did, a chain proving one run could be relabelled as another
    /// run's proof by the surface displaying it, which is the forgery the
    /// whole hop-mismatch rule exists to catch.
    #[test]
    fn the_override_cannot_relabel_a_complete_chain() {
        let (report, receipt) = evidence_pair();
        let chain = project_issue31_evidence_pair(&report, &receipt, Some("run.some.other.run"))
            .expect("projects");
        assert_eq!(chain.run_ref().as_str(), "run.full-auto.run-01");
    }

    /// A chain the host produced but this surface may not carry is an error,
    /// not a complete chain and not a silent drop.
    ///
    /// The detail bound at decode (256) is tighter than the record scan
    /// (512), so a long-but-clean test command passes `carries_private_hop`
    /// and is then refused by the decoder. The caller's honest rendering of
    /// that error is `hop_private`.
    #[test]
    fn a_detail_past_the_carried_bound_is_refused_rather_than_shown() {
        let (mut report, receipt) = evidence_pair();
        report["evidence"]["testCommand"] = json!(format!("cargo test -p {}", "x".repeat(300)));
        assert_eq!(
            project_issue31_evidence_pair(&report, &receipt, Some("run.full-auto.run-01")),
            Err(Issue31FullAutoAdjunctError::UnsafeText)
        );
    }
}
