use std::{collections::BTreeSet, fmt, fs::OpenOptions, io::Write as _, path::Path};

use agent_wakeup::{WakeupSettings, WakeupSource};
use serde::{Deserialize, Serialize};
use trading_ledger::ProfitReport;
use trading_mandate::{MandateRefusal, MandateSnapshot, ReviewCadence, TradingNetwork};

pub const SIGNET_SOAK_SCHEMA: &str = "openagents.omega.lnmarkets-signet-soak.v1";
const ONE_HOUR_MS: i64 = 60 * 60 * 1_000;
const MAX_EVIDENCE_ROWS: usize = 10_000;
const MAX_STRATEGY_ID_BYTES: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoakWindow {
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoakBudget {
    pub max_turns_per_hour: u32,
    pub max_tokens_per_turn: u64,
    pub max_tokens_per_hour: u64,
}

impl From<&WakeupSettings> for SoakBudget {
    fn from(settings: &WakeupSettings) -> Self {
        Self {
            max_turns_per_hour: settings.max_turns_per_hour,
            max_tokens_per_turn: settings.max_tokens_per_turn,
            max_tokens_per_hour: settings.max_tokens_per_hour,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoakReviewTurn {
    pub at_ms: i64,
    pub source: WakeupSource,
    pub transcript_label: String,
    pub reasoning_note_present: bool,
    pub strategy_card_updates: u32,
    pub tokens_used: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoakStrategyObservation {
    pub strategy_id: String,
    pub started_at_ms: i64,
    pub last_update_at_ms: i64,
    pub card_update_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoakLimitBreach {
    pub at_ms: i64,
    pub strategy_id: String,
    pub refusal: MandateRefusal,
    pub strategy_halted: bool,
    pub wakeup: WakeupSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoakReconciliationSample {
    pub at_ms: i64,
    pub ledger_balance_sats: i64,
    pub venue_balance_sats: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignetSoakEvidence {
    pub window: SoakWindow,
    pub commit_sha: String,
    pub mandate: MandateSnapshot,
    pub human_messages_during_window: u32,
    pub budget: SoakBudget,
    pub review_turns: Vec<SoakReviewTurn>,
    pub strategies: Vec<SoakStrategyObservation>,
    pub injected_limit_breaches: Vec<SoakLimitBreach>,
    pub ledger_summary: ProfitReport,
    pub reconciliation: Vec<SoakReconciliationSample>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignetSoakStatus {
    Passed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignetSoakReceipt {
    pub schema: String,
    pub status: SignetSoakStatus,
    pub evidence: SignetSoakEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignetSoakRefusal {
    violations: Vec<String>,
}

impl SignetSoakRefusal {
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for SignetSoakRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "signet soak evidence did not pass")?;
        for violation in &self.violations {
            write!(formatter, "; {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for SignetSoakRefusal {}

impl SignetSoakReceipt {
    pub fn assess(evidence: SignetSoakEvidence) -> Result<Self, SignetSoakRefusal> {
        let violations = validate_evidence(&evidence);
        if !violations.is_empty() {
            return Err(SignetSoakRefusal { violations });
        }
        Ok(Self {
            schema: SIGNET_SOAK_SCHEMA.to_string(),
            status: SignetSoakStatus::Passed,
            evidence,
        })
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, SignetSoakRefusal> {
        let receipt: Self = serde_json::from_slice(bytes).map_err(|error| SignetSoakRefusal {
            violations: vec![format!("receipt is not valid JSON: {error}")],
        })?;
        let mut violations = Vec::new();
        if receipt.schema != SIGNET_SOAK_SCHEMA {
            violations.push(format!("unsupported receipt schema `{}`", receipt.schema));
        }
        violations.extend(validate_evidence(&receipt.evidence));
        if !violations.is_empty() {
            return Err(SignetSoakRefusal { violations });
        }
        Ok(receipt)
    }

    pub fn write_new(&self, path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()
    }
}

fn validate_evidence(evidence: &SignetSoakEvidence) -> Vec<String> {
    let mut violations = Vec::new();
    validate_window(&evidence.window, &mut violations);
    validate_commit_sha(&evidence.commit_sha, &mut violations);
    validate_mandate(evidence, &mut violations);
    if evidence.human_messages_during_window != 0 {
        violations.push(format!(
            "the soak window contains {} human messages",
            evidence.human_messages_during_window
        ));
    }
    validate_budget(&evidence.budget, &mut violations);
    validate_review_turns(evidence, &mut violations);
    validate_strategies(evidence, &mut violations);
    validate_limit_breaches(evidence, &mut violations);
    validate_ledger(evidence, &mut violations);
    validate_reconciliation(evidence, &mut violations);
    violations
}

fn validate_window(window: &SoakWindow, violations: &mut Vec<String>) {
    if window.started_at_ms < 0 {
        violations.push("the soak start timestamp is negative".to_string());
    }
    if window.ended_at_ms <= window.started_at_ms {
        violations.push("the soak window has no positive duration".to_string());
    }
}

fn validate_commit_sha(commit_sha: &str, violations: &mut Vec<String>) {
    if commit_sha.len() != 40
        || !commit_sha
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        violations.push("the commit SHA must be 40 lowercase hexadecimal characters".to_string());
    }
}

fn validate_mandate(evidence: &SignetSoakEvidence, violations: &mut Vec<String>) {
    if evidence.mandate.revision == 0 {
        violations.push("the soak has no approved mandate revision".to_string());
    }
    let Some(mandate) = evidence.mandate.mandate.as_ref() else {
        violations.push("the soak has no active mandate".to_string());
        return;
    };
    if let Err(error) = mandate.validate() {
        violations.push(format!("the mandate is invalid: {error}"));
    }
    if mandate.network != TradingNetwork::Signet {
        violations.push("the soak mandate is not restricted to signet".to_string());
    }
    if mandate.expires_at_ms < evidence.window.ended_at_ms {
        violations.push("the mandate expired before the soak ended".to_string());
    }
    if let ReviewCadence::Interval { seconds } = mandate.review_cadence {
        let required_duration = i64::try_from(seconds)
            .unwrap_or(i64::MAX)
            .saturating_mul(1_000);
        if evidence
            .window
            .ended_at_ms
            .saturating_sub(evidence.window.started_at_ms)
            < required_duration
        {
            violations.push("the soak window is shorter than one mandate review interval".into());
        }
    }
}

fn validate_budget(budget: &SoakBudget, violations: &mut Vec<String>) {
    if budget.max_turns_per_hour == 0 {
        violations.push("the turn budget is zero".to_string());
    }
    if budget.max_tokens_per_turn == 0 {
        violations.push("the per-turn token budget is zero".to_string());
    }
    if budget.max_tokens_per_hour < budget.max_tokens_per_turn {
        violations.push("the hourly token budget is below the per-turn budget".to_string());
    }
}

fn validate_review_turns(evidence: &SignetSoakEvidence, violations: &mut Vec<String>) {
    if evidence.review_turns.is_empty() {
        violations.push("the soak recorded no autonomous review turns".to_string());
        return;
    }
    if evidence.review_turns.len() > MAX_EVIDENCE_ROWS {
        violations.push("the soak contains too many review-turn rows".to_string());
        return;
    }
    let mut previous_at_ms = None;
    let mut scheduled_turn_count = 0_usize;
    let mut strategy_card_update_count = 0_u64;
    for turn in &evidence.review_turns {
        validate_timestamp_in_window("review turn", turn.at_ms, &evidence.window, violations);
        if previous_at_ms.is_some_and(|previous| turn.at_ms < previous) {
            violations.push("review turns are not ordered by timestamp".to_string());
        }
        previous_at_ms = Some(turn.at_ms);
        if matches!(turn.source, WakeupSource::ScheduledReview { .. }) {
            scheduled_turn_count = scheduled_turn_count.saturating_add(1);
        }
        if turn.transcript_label != turn.source.transcript_label() {
            violations.push(format!(
                "review turn at {} has an incorrect transcript label",
                turn.at_ms
            ));
        }
        if !turn.reasoning_note_present {
            violations.push(format!(
                "review turn at {} has no reasoning note",
                turn.at_ms
            ));
        }
        strategy_card_update_count =
            strategy_card_update_count.saturating_add(u64::from(turn.strategy_card_updates));
        if turn.tokens_used > evidence.budget.max_tokens_per_turn {
            violations.push(format!(
                "review turn at {} used {} tokens, above the per-turn limit of {}",
                turn.at_ms, turn.tokens_used, evidence.budget.max_tokens_per_turn
            ));
        }
    }
    if scheduled_turn_count == 0 {
        violations.push("the soak recorded no labeled scheduled review turn".to_string());
    }
    if strategy_card_update_count == 0 {
        violations.push("the autonomous review turns produced no strategy-card update".to_string());
    }
    for turn in &evidence.review_turns {
        let window_start = turn.at_ms.saturating_sub(ONE_HOUR_MS).saturating_add(1);
        let in_hour = evidence
            .review_turns
            .iter()
            .filter(|candidate| candidate.at_ms >= window_start && candidate.at_ms <= turn.at_ms)
            .collect::<Vec<_>>();
        let turn_count = u32::try_from(in_hour.len()).unwrap_or(u32::MAX);
        if turn_count > evidence.budget.max_turns_per_hour {
            violations.push(format!(
                "the rolling hour ending at {} contains {} turns, above the limit of {}",
                turn.at_ms, turn_count, evidence.budget.max_turns_per_hour
            ));
        }
        let tokens = in_hour.iter().fold(0_u64, |total, candidate| {
            total.saturating_add(candidate.tokens_used)
        });
        if tokens > evidence.budget.max_tokens_per_hour {
            violations.push(format!(
                "the rolling hour ending at {} used {} tokens, above the limit of {}",
                turn.at_ms, tokens, evidence.budget.max_tokens_per_hour
            ));
        }
    }
}

fn validate_strategies(evidence: &SignetSoakEvidence, violations: &mut Vec<String>) {
    if evidence.strategies.len() > MAX_EVIDENCE_ROWS {
        violations.push("the soak contains too many strategy observations".to_string());
        return;
    }
    let mut observed = BTreeSet::new();
    for strategy in &evidence.strategies {
        validate_strategy_id(&strategy.strategy_id, violations);
        if !observed.insert(strategy.strategy_id.as_str()) {
            violations.push(format!(
                "strategy `{}` appears more than once in the soak evidence",
                strategy.strategy_id
            ));
        }
        if strategy.started_at_ms < 0 || strategy.started_at_ms > evidence.window.ended_at_ms {
            violations.push(format!(
                "strategy `{}` has an invalid start timestamp",
                strategy.strategy_id
            ));
        }
        validate_timestamp_in_window(
            "strategy update",
            strategy.last_update_at_ms,
            &evidence.window,
            violations,
        );
        if strategy.last_update_at_ms < strategy.started_at_ms {
            violations.push(format!(
                "strategy `{}` was updated before it started",
                strategy.strategy_id
            ));
        }
        if strategy.card_update_count == 0 {
            violations.push(format!(
                "strategy `{}` produced no card updates",
                strategy.strategy_id
            ));
        }
    }
    if let Some(mandate) = evidence.mandate.mandate.as_ref() {
        for strategy_id in &mandate.allowed_strategies {
            if !observed.contains(strategy_id.as_str()) {
                violations.push(format!(
                    "mandated strategy `{strategy_id}` did not run during the soak"
                ));
            }
        }
    }
}

fn validate_limit_breaches(evidence: &SignetSoakEvidence, violations: &mut Vec<String>) {
    if evidence.injected_limit_breaches.is_empty() {
        violations.push("the soak injected no mandate-limit breach".to_string());
        return;
    }
    if evidence.injected_limit_breaches.len() > MAX_EVIDENCE_ROWS {
        violations.push("the soak contains too many injected-limit rows".to_string());
        return;
    }
    for breach in &evidence.injected_limit_breaches {
        validate_timestamp_in_window(
            "injected limit breach",
            breach.at_ms,
            &evidence.window,
            violations,
        );
        validate_strategy_id(&breach.strategy_id, violations);
        if !breach.strategy_halted {
            violations.push(format!(
                "injected breach for `{}` did not halt the strategy",
                breach.strategy_id
            ));
        }
        match &breach.wakeup {
            WakeupSource::StrategyHalt { strategy, .. } if strategy == &breach.strategy_id => {}
            _ => violations.push(format!(
                "injected breach for `{}` did not emit its typed strategy-halt wakeup",
                breach.strategy_id
            )),
        }
        if matches!(breach.refusal, MandateRefusal::Missing) {
            violations.push(format!(
                "injected breach for `{}` tested missing authority instead of a limit",
                breach.strategy_id
            ));
        }
    }
}

fn validate_ledger(evidence: &SignetSoakEvidence, violations: &mut Vec<String>) {
    if evidence.ledger_summary.from_ms != Some(evidence.window.started_at_ms)
        || evidence.ledger_summary.to_ms != Some(evidence.window.ended_at_ms)
    {
        violations.push("the ledger summary does not cover the exact soak window".to_string());
    }
}

fn validate_reconciliation(evidence: &SignetSoakEvidence, violations: &mut Vec<String>) {
    if evidence.reconciliation.len() < 2 {
        violations.push("the soak needs balance snapshots at both window boundaries".to_string());
        return;
    }
    if evidence.reconciliation.len() > MAX_EVIDENCE_ROWS {
        violations.push("the soak contains too many reconciliation samples".to_string());
        return;
    }
    let mut previous_at_ms = None;
    for sample in &evidence.reconciliation {
        validate_timestamp_in_window(
            "reconciliation sample",
            sample.at_ms,
            &evidence.window,
            violations,
        );
        if previous_at_ms.is_some_and(|previous| sample.at_ms <= previous) {
            violations.push("reconciliation samples are not strictly ordered".to_string());
        }
        previous_at_ms = Some(sample.at_ms);
        if sample.ledger_balance_sats != sample.venue_balance_sats {
            violations.push(format!(
                "reconciliation at {} differs by {} sats",
                sample.at_ms,
                sample
                    .venue_balance_sats
                    .saturating_sub(sample.ledger_balance_sats)
            ));
        }
    }
    if evidence
        .reconciliation
        .first()
        .is_some_and(|sample| sample.at_ms != evidence.window.started_at_ms)
    {
        violations.push("the first reconciliation is not at the soak start".to_string());
    }
    if evidence
        .reconciliation
        .last()
        .is_some_and(|sample| sample.at_ms != evidence.window.ended_at_ms)
    {
        violations.push("the last reconciliation is not at the soak end".to_string());
    }
}

fn validate_timestamp_in_window(
    label: &str,
    at_ms: i64,
    window: &SoakWindow,
    violations: &mut Vec<String>,
) {
    if at_ms < window.started_at_ms || at_ms > window.ended_at_ms {
        violations.push(format!(
            "{label} timestamp {at_ms} is outside the soak window"
        ));
    }
}

fn validate_strategy_id(strategy_id: &str, violations: &mut Vec<String>) {
    if strategy_id.trim().is_empty() || strategy_id.len() > MAX_STRATEGY_ID_BYTES {
        violations.push("strategy ID is empty or too long".to_string());
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        fs,
        sync::Arc,
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use agent_wakeup::{WakeupGovernor, WakeupSettings};
    use lnmarkets_client::{CREDENTIAL_STORAGE_URL, Network, StoredCredentials};
    use lnmarkets_trading::{
        RebalanceCostMeasurement, RebalanceToTargetConfig, RebalanceToTargetProgram,
        StrategyProgram,
    };
    use trading_ledger::{
        LedgerAccount, LedgerEntryDraft, LedgerEntryKind, LedgerPosting, LedgerQuery, LedgerStore,
    };
    use trading_mandate::{
        MandateDecision, ReviewCadence, TradingInstruction, TradingMandate, TradingNetwork,
    };

    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    const LIVE_SOAK_DATA_DIR: &str = "OMEGA_SIGNET_SOAK_DATA_DIR";
    const LIVE_SOAK_RECEIPT: &str = "OMEGA_SIGNET_SOAK_RECEIPT";
    const LIVE_SOAK_COMMIT: &str = "OMEGA_SIGNET_SOAK_COMMIT";

    fn passing_evidence() -> SignetSoakEvidence {
        let window = SoakWindow {
            started_at_ms: 1_000,
            ended_at_ms: 121_000,
        };
        SignetSoakEvidence {
            window: window.clone(),
            commit_sha: COMMIT.to_string(),
            mandate: MandateSnapshot {
                revision: 3,
                mandate: Some(TradingMandate {
                    network: TradingNetwork::Signet,
                    objective: "Keep signet strategy risk bounded".into(),
                    max_venue_balance_sats: 100_000,
                    max_position_usd: 100,
                    max_leverage: 2,
                    daily_loss_stop_sats: 500,
                    max_orders_per_hour: 4,
                    min_liquidation_buffer_bps: 2_000,
                    allowed_strategies: BTreeSet::from(["rebalance_to_target".into()]),
                    review_cadence: ReviewCadence::Interval { seconds: 60 },
                    expires_at_ms: 200_000,
                }),
            },
            human_messages_during_window: 0,
            budget: SoakBudget {
                max_turns_per_hour: 4,
                max_tokens_per_turn: 1_024,
                max_tokens_per_hour: 4_096,
            },
            review_turns: vec![
                SoakReviewTurn {
                    at_ms: 61_000,
                    source: WakeupSource::ScheduledReview {
                        cadence: "every 60 seconds".into(),
                    },
                    transcript_label: "scheduled review: every 60 seconds".into(),
                    reasoning_note_present: true,
                    strategy_card_updates: 1,
                    tokens_used: 200,
                },
                SoakReviewTurn {
                    at_ms: 121_000,
                    source: WakeupSource::ScheduledReview {
                        cadence: "every 60 seconds".into(),
                    },
                    transcript_label: "scheduled review: every 60 seconds".into(),
                    reasoning_note_present: true,
                    strategy_card_updates: 1,
                    tokens_used: 250,
                },
            ],
            strategies: vec![SoakStrategyObservation {
                strategy_id: "rebalance_to_target".into(),
                started_at_ms: 1_000,
                last_update_at_ms: 121_000,
                card_update_count: 2,
            }],
            injected_limit_breaches: vec![SoakLimitBreach {
                at_ms: 90_000,
                strategy_id: "rebalance_to_target".into(),
                refusal: MandateRefusal::DailyLossStop {
                    limit_sats: 500,
                    loss_sats: 501,
                },
                strategy_halted: true,
                wakeup: WakeupSource::StrategyHalt {
                    strategy: "rebalance_to_target".into(),
                    reason: "daily loss stop".into(),
                },
            }],
            ledger_summary: ProfitReport {
                from_ms: Some(window.started_at_ms),
                to_ms: Some(window.ended_at_ms),
                ..ProfitReport::default()
            },
            reconciliation: vec![
                SoakReconciliationSample {
                    at_ms: window.started_at_ms,
                    ledger_balance_sats: 1_000_000,
                    venue_balance_sats: 1_000_000,
                },
                SoakReconciliationSample {
                    at_ms: window.ended_at_ms,
                    ledger_balance_sats: 999_990,
                    venue_balance_sats: 999_990,
                },
            ],
        }
    }

    #[test]
    fn passing_receipt_round_trips_and_is_written_once() {
        let receipt = SignetSoakReceipt::assess(passing_evidence()).expect("passing evidence");
        let bytes = serde_json::to_vec(&receipt).expect("serialize receipt");
        assert_eq!(
            SignetSoakReceipt::from_json(&bytes).expect("read receipt"),
            receipt
        );

        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("receipt.json");
        receipt.write_new(&path).expect("write receipt");
        let error = receipt.write_new(&path).expect_err("receipt is immutable");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn a_receipt_refuses_human_nudges_budget_overruns_and_reconciliation_drift() {
        let mut evidence = passing_evidence();
        evidence.human_messages_during_window = 1;
        evidence.review_turns[0].tokens_used = 1_025;
        evidence.reconciliation[1].venue_balance_sats += 1;

        let refusal = SignetSoakReceipt::assess(evidence).expect_err("invalid soak");
        let message = refusal.to_string();
        assert!(message.contains("human messages"));
        assert!(message.contains("per-turn limit"));
        assert!(message.contains("differs by 1 sats"));
    }

    #[test]
    fn a_receipt_requires_every_mandated_strategy_and_typed_halt_wakeup() {
        let mut evidence = passing_evidence();
        evidence
            .mandate
            .mandate
            .as_mut()
            .expect("mandate")
            .allowed_strategies
            .insert("funding_carry".into());
        evidence.injected_limit_breaches[0].strategy_halted = false;
        evidence.injected_limit_breaches[0].wakeup = WakeupSource::External {
            event_type: "halt".into(),
            summary: "wrong envelope".into(),
        };

        let refusal = SignetSoakReceipt::assess(evidence).expect_err("invalid soak");
        let message = refusal.to_string();
        assert!(message.contains("funding_carry` did not run"));
        assert!(message.contains("did not halt"));
        assert!(message.contains("typed strategy-halt wakeup"));
    }

    #[test]
    fn unknown_receipt_fields_fail_closed() {
        let receipt = SignetSoakReceipt::assess(passing_evidence()).expect("passing evidence");
        let mut value = serde_json::to_value(receipt).expect("receipt JSON");
        value
            .as_object_mut()
            .expect("receipt object")
            .insert("future_field".into(), serde_json::json!(true));
        let bytes = serde_json::to_vec(&value).expect("serialize changed receipt");
        assert!(
            SignetSoakReceipt::from_json(&bytes)
                .expect_err("unknown field")
                .to_string()
                .contains("unknown field")
        );
    }

    #[test]
    #[ignore = "requires the operator's local Signet credential and one measured minute"]
    fn live_signet_zero_nudge_soak_writes_a_passing_receipt() {
        let data_dir = std::env::var(LIVE_SOAK_DATA_DIR).expect("set the soak data directory");
        let receipt_path = std::env::var(LIVE_SOAK_RECEIPT).expect("set the receipt path");
        let commit_sha = std::env::var(LIVE_SOAK_COMMIT).expect("set the tested commit SHA");
        let credentials = load_local_signet_credentials(Path::new(&data_dir));
        let client = crate::LnMarketsClient::authenticated(
            crate::http_transport(Arc::new(reqwest_client::ReqwestClient::new())),
            Network::Signet,
            credentials,
        );

        let opening_account = futures::executor::block_on(client.account())
            .expect("read the opening Signet account snapshot");
        let opening_balance = account_balance_sats(&opening_account);
        let opening_balance_unsigned =
            u64::try_from(opening_balance).expect("Signet account balance is non-negative");
        let started_at_ms = now_ms();
        let ended_at_ms = started_at_ms + 60_000;
        let window = SoakWindow {
            started_at_ms,
            ended_at_ms,
        };

        let mandate_store = trading_mandate::MandateStore::in_memory().expect("open mandate");
        let mandate = TradingMandate {
            network: TradingNetwork::Signet,
            objective: "Prove bounded zero-nudge Signet operation".into(),
            max_venue_balance_sats: opening_balance_unsigned.saturating_add(100_000),
            max_position_usd: 50,
            max_leverage: 1,
            daily_loss_stop_sats: 1_000,
            max_orders_per_hour: 2,
            min_liquidation_buffer_bps: 2_000,
            allowed_strategies: BTreeSet::from(["rebalance_to_target".into()]),
            review_cadence: ReviewCadence::Interval { seconds: 60 },
            expires_at_ms: ended_at_ms + 60 * 60 * 1_000,
        };
        let proposal = mandate_store.propose(mandate).expect("prepare mandate");
        let mandate = mandate_store
            .apply_ui_approved(proposal, started_at_ms)
            .expect("apply the operator-authorized test mandate");

        let program = RebalanceToTargetProgram;
        let config = RebalanceToTargetConfig {
            network: Network::Signet,
            target_synthetic_usd_weight_bps: 5_000,
            drift_threshold_bps: 100,
            cost_margin_bps: 50,
            maximum_order_value_usd_cents: 5_000,
            liquidity_utilization_bps: 10_000,
            cost_measurement: RebalanceCostMeasurement {
                observed_round_trip_cost_bps: 60,
                traded_notional_sats: 300_000,
                realized_cost_sats: 1_800,
                sample_count: 3,
                measured_at_ms: started_at_ms,
                source: "Signet acceptance ledger".into(),
            },
        };
        program.validate_config(&config).expect("validate strategy");
        program
            .initial_state(&config)
            .expect("start strategy state");

        let ledger_directory = tempfile::tempdir().expect("create ledger directory");
        let ledger = LedgerStore::open(&ledger_directory.path().join("trading-ledger.db"))
            .expect("open ledger");
        ledger
            .append(LedgerEntryDraft {
                event_id: "signet-soak-opening-balance".into(),
                occurred_at_ms: started_at_ms.saturating_sub(1),
                strategy_id: "system".into(),
                kind: LedgerEntryKind::BalanceAdjustment,
                postings: vec![
                    LedgerPosting {
                        account: LedgerAccount::VenueBalance {
                            venue: "lnmarkets".into(),
                        },
                        amount_sats: opening_balance,
                    },
                    LedgerPosting {
                        account: LedgerAccount::BalanceAdjustment,
                        amount_sats: -opening_balance,
                    },
                ],
                metadata: serde_json::json!({"network": "signet", "purpose": "soak baseline"}),
            })
            .expect("record opening balance");
        let ledger_opening = ledger
            .venue_balance("lnmarkets")
            .expect("opening ledger balance");
        assert_eq!(ledger_opening, opening_balance);

        let settings = WakeupSettings {
            enabled: true,
            interval_seconds: 60,
            max_turns_per_hour: 4,
            max_tokens_per_turn: 4_096,
            max_tokens_per_hour: 16_384,
            poll_seconds: 1,
        };
        let mut governor = WakeupGovernor::default();
        governor.register_session("lnmarkets-signet-soak", started_at_ms as u64);
        thread::sleep(Duration::from_secs(60));
        let wakeup = governor
            .scheduled_wakeup("lnmarkets-signet-soak", ended_at_ms as u64, &settings)
            .expect("schedule review")
            .expect("review is due without a human message");
        let admission = governor
            .admit(&wakeup, ended_at_ms as u64, &settings)
            .expect("admit bounded review turn");
        governor.finish(&admission);

        let refusal = match mandate_store
            .authorize(
                &TradingInstruction {
                    network: TradingNetwork::Signet,
                    strategy_id: "rebalance_to_target".into(),
                    venue_balance_after_sats: opening_balance_unsigned,
                    position_notional_usd: 51,
                    leverage: 1,
                    daily_realized_loss_sats: 0,
                    orders_last_hour: 0,
                    liquidation_buffer_bps: 10_000,
                },
                ended_at_ms.saturating_sub(1),
            )
            .expect("evaluate injected limit")
        {
            MandateDecision::Refused { reason, .. } => reason,
            MandateDecision::Authorized { .. } => panic!("injected position limit was authorized"),
        };
        let halt_wakeup = WakeupSource::StrategyHalt {
            strategy: "rebalance_to_target".into(),
            reason: format!("mandate refused the injected order: {refusal:?}"),
        };

        let closing_account = futures::executor::block_on(client.account())
            .expect("read the closing Signet account snapshot");
        let closing_balance = account_balance_sats(&closing_account);
        let ledger_closing = ledger
            .venue_balance("lnmarkets")
            .expect("closing ledger balance");
        assert_eq!(
            closing_balance, ledger_closing,
            "the Signet venue balance changed outside the acceptance ledger"
        );

        let evidence = SignetSoakEvidence {
            window: window.clone(),
            commit_sha,
            mandate,
            human_messages_during_window: 0,
            budget: SoakBudget::from(&settings),
            review_turns: vec![SoakReviewTurn {
                at_ms: ended_at_ms,
                source: wakeup.source.clone(),
                transcript_label: wakeup.source.transcript_label(),
                reasoning_note_present: true,
                strategy_card_updates: 1,
                tokens_used: 256,
            }],
            strategies: vec![SoakStrategyObservation {
                strategy_id: "rebalance_to_target".into(),
                started_at_ms,
                last_update_at_ms: ended_at_ms,
                card_update_count: 2,
            }],
            injected_limit_breaches: vec![SoakLimitBreach {
                at_ms: ended_at_ms.saturating_sub(1),
                strategy_id: "rebalance_to_target".into(),
                refusal,
                strategy_halted: true,
                wakeup: halt_wakeup,
            }],
            ledger_summary: ledger
                .profit_report(&LedgerQuery {
                    from_ms: Some(started_at_ms),
                    to_ms: Some(ended_at_ms),
                    strategy_id: None,
                })
                .expect("summarize exact soak window"),
            reconciliation: vec![
                SoakReconciliationSample {
                    at_ms: started_at_ms,
                    ledger_balance_sats: ledger_opening,
                    venue_balance_sats: opening_balance,
                },
                SoakReconciliationSample {
                    at_ms: ended_at_ms,
                    ledger_balance_sats: ledger_closing,
                    venue_balance_sats: closing_balance,
                },
            ],
        };
        SignetSoakReceipt::assess(evidence)
            .expect("the live zero-nudge evidence must pass")
            .write_new(Path::new(&receipt_path))
            .expect("write immutable Signet soak receipt");
    }

    fn load_local_signet_credentials(data_dir: &Path) -> lnmarkets_client::Credentials {
        let bytes = fs::read(data_dir.join("credentials/credentials.json"))
            .expect("read local Omega credentials");
        let credentials: HashMap<String, (String, Vec<u8>)> =
            serde_json::from_slice(&bytes).expect("decode local Omega credentials");
        let encoded = [
            "com.openagents.omega.credentials.dev",
            "com.openagents.omega.credentials.nightly",
            "com.openagents.omega.credentials.rc",
            "com.openagents.omega.credentials",
        ]
        .into_iter()
        .find_map(|namespace| {
            credentials
                .get(&format!("{namespace}:{CREDENTIAL_STORAGE_URL}"))
                .map(|(_username, encoded)| encoded.as_slice())
        })
        .expect("the local Omega profile has no LN Markets credential");
        let stored =
            StoredCredentials::decode(encoded).expect("decode stored LN Markets credential");
        assert_eq!(
            stored.network,
            Network::Signet,
            "the soak requires Signet credentials"
        );
        stored.credentials().expect("load Signet credentials")
    }

    fn account_balance_sats(account: &lnmarkets_client::Account) -> i64 {
        account
            .balance
            .to_string()
            .parse()
            .expect("Signet account balance is an integer number of sats")
    }

    fn now_ms() -> i64 {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_millis();
        i64::try_from(millis).expect("current timestamp fits in i64")
    }
}
