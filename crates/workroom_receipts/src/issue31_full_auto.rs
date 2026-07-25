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

const FORBIDDEN_PATH_FRAGMENTS: [&str; 5] = [
    "/users/",
    "/home/",
    "/var/folders/",
    "/private/tmp/",
    "~/",
];

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31EvidenceUnavailableReason {
    HopMissing,
    HopMismatched,
    HopPrivate,
    SelfReported,
    HostUnavailable,
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
            Self::InvalidJson => "issue 31 full auto adjunct is not valid contract JSON",
            Self::InvalidSchema => "issue 31 full auto adjunct schema is not supported",
            Self::UnsafeReference => "issue 31 full auto adjunct contains an unsafe reference",
            Self::UnsafeText => "issue 31 full auto adjunct contains unsafe text",
            Self::BoundExceeded => "issue 31 full auto adjunct bound was exceeded",
            Self::DuplicateReference => "issue 31 full auto adjunct contains a duplicate reference",
            Self::InvalidTimestamp => "issue 31 full auto adjunct timestamp order is invalid",
            Self::InvalidRunState => "issue 31 full auto adjunct run state is invalid",
            Self::InvalidControlBinding => {
                "issue 31 full auto adjunct binds a control to a stale run generation"
            }
            Self::InvalidAccountState => "issue 31 full auto adjunct confuses a lane with an account",
            Self::InvalidHandoffState => "issue 31 full auto adjunct handoff state is invalid",
            Self::InvalidEvidenceChain => "issue 31 full auto adjunct evidence chain is invalid",
            Self::UnknownReference => "issue 31 full auto adjunct points at an unknown record",
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

    let runs = raw.runs.into_iter().map(project_run).collect::<AdjunctResult<Vec<_>>>()?;
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
            assert!(!rendered.contains("Bearer"), "fixture {name} echoed a token");
            assert!(!rendered.contains("/Users/"), "fixture {name} echoed a path");
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
