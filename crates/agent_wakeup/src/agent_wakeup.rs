use collections::HashMap;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const WAKEUP_SCHEMA_VERSION: u16 = 1;
const ONE_HOUR_MS: u64 = 60 * 60 * 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeupSettings {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub max_turns_per_hour: u32,
    pub max_tokens_per_turn: u64,
    pub max_tokens_per_hour: u64,
    pub poll_seconds: u64,
}

impl Default for WakeupSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: 15 * 60,
            max_turns_per_hour: 4,
            max_tokens_per_turn: 4_096,
            max_tokens_per_hour: 16_384,
            poll_seconds: 15,
        }
    }
}

impl WakeupSettings {
    pub fn validate(&self) -> Result<(), WakeupRefusal> {
        if self.interval_seconds < 60 {
            return Err(WakeupRefusal::InvalidSettings(
                "interval_seconds must be at least 60".to_string(),
            ));
        }
        if self.max_turns_per_hour == 0 {
            return Err(WakeupRefusal::InvalidSettings(
                "max_turns_per_hour must be greater than zero".to_string(),
            ));
        }
        if self.max_tokens_per_turn == 0 {
            return Err(WakeupRefusal::InvalidSettings(
                "max_tokens_per_turn must be greater than zero".to_string(),
            ));
        }
        if self.max_tokens_per_hour < self.max_tokens_per_turn {
            return Err(WakeupRefusal::InvalidSettings(
                "max_tokens_per_hour must be at least max_tokens_per_turn".to_string(),
            ));
        }
        if self.poll_seconds == 0 || self.poll_seconds > self.interval_seconds {
            return Err(WakeupRefusal::InvalidSettings(
                "poll_seconds must be between 1 and interval_seconds".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WakeupSource {
    ScheduledReview { cadence: String },
    FundingSignFlip { previous_bps: i64, current_bps: i64 },
    DrawdownLimitApproach { drawdown_sats: u64, limit_sats: u64 },
    LiquidationDistanceBreach { distance_bps: u32, limit_bps: u32 },
    VolatilityRegimeChange { previous: String, current: String },
    StrategyHalt { strategy: String, reason: String },
    DepositSeen { amount_sats: u64, reference: String },
    WithdrawalSeen { amount_sats: u64, reference: String },
    External { event_type: String, summary: String },
}

impl WakeupSource {
    pub fn transcript_label(&self) -> String {
        match self {
            Self::ScheduledReview { cadence } => format!("scheduled review: {cadence}"),
            Self::FundingSignFlip { .. } => "event: funding sign flip".to_string(),
            Self::DrawdownLimitApproach { .. } => "event: drawdown limit approach".to_string(),
            Self::LiquidationDistanceBreach { .. } => {
                "event: liquidation distance breach".to_string()
            }
            Self::VolatilityRegimeChange { .. } => "event: volatility regime change".to_string(),
            Self::StrategyHalt { strategy, .. } => format!("event: {strategy} strategy halt"),
            Self::DepositSeen { .. } => "event: deposit seen".to_string(),
            Self::WithdrawalSeen { .. } => "event: withdrawal seen".to_string(),
            Self::External { event_type, .. } => format!("event: {event_type}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWakeup {
    pub schema_version: u16,
    pub session_id: String,
    pub emitted_at_ms: u64,
    pub token_budget: u64,
    pub source: WakeupSource,
    pub instruction: String,
}

impl AgentWakeup {
    pub fn new(
        session_id: impl Into<String>,
        emitted_at_ms: u64,
        token_budget: u64,
        source: WakeupSource,
        instruction: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: WAKEUP_SCHEMA_VERSION,
            session_id: session_id.into(),
            emitted_at_ms,
            token_budget,
            source,
            instruction: instruction.into(),
        }
    }

    pub fn validate(&self) -> Result<(), WakeupRefusal> {
        if self.schema_version != WAKEUP_SCHEMA_VERSION {
            return Err(WakeupRefusal::UnsupportedSchema(self.schema_version));
        }
        if self.session_id.trim().is_empty() {
            return Err(WakeupRefusal::InvalidRequest(
                "session_id is required".to_string(),
            ));
        }
        if self.instruction.trim().is_empty() {
            return Err(WakeupRefusal::InvalidRequest(
                "instruction is required".to_string(),
            ));
        }
        if self.token_budget == 0 {
            return Err(WakeupRefusal::InvalidRequest(
                "token_budget must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }

    pub fn transcript_text(&self) -> String {
        format!(
            "[Agent wakeup · {} · token budget {}]\n{}",
            self.source.transcript_label(),
            self.token_budget,
            self.instruction.trim()
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeupAdmission {
    pub session_id: String,
    pub reserved_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WakeupRefusal {
    #[error("agent wakeups are disabled")]
    Disabled,
    #[error("invalid wakeup settings: {0}")]
    InvalidSettings(String),
    #[error("invalid wakeup request: {0}")]
    InvalidRequest(String),
    #[error("unsupported wakeup schema version {0}")]
    UnsupportedSchema(u16),
    #[error("the thread already has a turn in flight")]
    TurnInFlight,
    #[error("the hourly wakeup frequency cap was reached")]
    FrequencyCap,
    #[error("the wakeup token budget exceeds the per-turn cap")]
    PerTurnTokenCap,
    #[error("the hourly wakeup token budget cap was reached")]
    HourlyTokenCap,
}

#[derive(Clone, Debug)]
struct Reservation {
    admitted_at_ms: u64,
    tokens: u64,
}

#[derive(Clone, Debug, Default)]
struct SessionState {
    registered_at_ms: u64,
    last_scheduled_at_ms: Option<u64>,
    in_flight: bool,
    reservations: VecDeque<Reservation>,
}

#[derive(Clone, Debug, Default)]
pub struct WakeupGovernor {
    sessions: HashMap<String, SessionState>,
}

impl WakeupGovernor {
    pub fn register_session(&mut self, session_id: impl Into<String>, now_ms: u64) {
        self.sessions
            .entry(session_id.into())
            .or_insert_with(|| SessionState {
                registered_at_ms: now_ms,
                ..SessionState::default()
            });
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    pub fn scheduled_wakeup(
        &mut self,
        session_id: &str,
        now_ms: u64,
        settings: &WakeupSettings,
    ) -> Result<Option<AgentWakeup>, WakeupRefusal> {
        settings.validate()?;
        if !settings.enabled {
            return Ok(None);
        }
        let state = self
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionState {
                registered_at_ms: now_ms,
                ..SessionState::default()
            });
        let last_at = state.last_scheduled_at_ms.unwrap_or(state.registered_at_ms);
        let interval_ms = settings.interval_seconds.saturating_mul(1_000);
        if now_ms.saturating_sub(last_at) < interval_ms {
            return Ok(None);
        }
        state.last_scheduled_at_ms = Some(now_ms);
        Ok(Some(AgentWakeup::new(
            session_id,
            now_ms,
            settings.max_tokens_per_turn,
            WakeupSource::ScheduledReview {
                cadence: format!("every {} seconds", settings.interval_seconds),
            },
            "Review the current thread state and relevant events. Act only within the active mandate and report any required risk action.",
        )))
    }

    pub fn admit(
        &mut self,
        wakeup: &AgentWakeup,
        now_ms: u64,
        settings: &WakeupSettings,
    ) -> Result<WakeupAdmission, WakeupRefusal> {
        settings.validate()?;
        wakeup.validate()?;
        if !settings.enabled {
            return Err(WakeupRefusal::Disabled);
        }
        if wakeup.token_budget > settings.max_tokens_per_turn {
            return Err(WakeupRefusal::PerTurnTokenCap);
        }

        let state = self
            .sessions
            .entry(wakeup.session_id.clone())
            .or_insert_with(|| SessionState {
                registered_at_ms: now_ms,
                ..SessionState::default()
            });
        while state.reservations.front().is_some_and(|reservation| {
            now_ms.saturating_sub(reservation.admitted_at_ms) >= ONE_HOUR_MS
        }) {
            state.reservations.pop_front();
        }
        if state.in_flight {
            return Err(WakeupRefusal::TurnInFlight);
        }
        if state.reservations.len() >= settings.max_turns_per_hour as usize {
            return Err(WakeupRefusal::FrequencyCap);
        }
        let reserved_tokens = state
            .reservations
            .iter()
            .try_fold(0_u64, |total, reservation| {
                total.checked_add(reservation.tokens)
            })
            .unwrap_or(u64::MAX);
        if reserved_tokens.saturating_add(wakeup.token_budget) > settings.max_tokens_per_hour {
            return Err(WakeupRefusal::HourlyTokenCap);
        }
        state.in_flight = true;
        state.reservations.push_back(Reservation {
            admitted_at_ms: now_ms,
            tokens: wakeup.token_budget,
        });
        Ok(WakeupAdmission {
            session_id: wakeup.session_id.clone(),
            reserved_tokens: wakeup.token_budget,
        })
    }

    pub fn finish(&mut self, admission: &WakeupAdmission) {
        if let Some(state) = self.sessions.get_mut(&admission.session_id) {
            state.in_flight = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_settings() -> WakeupSettings {
        WakeupSettings {
            enabled: true,
            interval_seconds: 60,
            max_turns_per_hour: 2,
            max_tokens_per_turn: 100,
            max_tokens_per_hour: 150,
            poll_seconds: 10,
        }
    }

    fn event(at: u64, tokens: u64) -> AgentWakeup {
        AgentWakeup::new(
            "thread-1",
            at,
            tokens,
            WakeupSource::StrategyHalt {
                strategy: "carry".to_string(),
                reason: "drawdown".to_string(),
            },
            "Review the halt.",
        )
    }

    #[test]
    fn envelope_round_trips_for_desktop_and_cloud() {
        let wakeup = event(1_000, 75);
        let json = serde_json::to_string(&wakeup).expect("serialize wakeup");
        let decoded: AgentWakeup = serde_json::from_str(&json).expect("deserialize wakeup");
        assert_eq!(decoded, wakeup);
        assert!(decoded.transcript_text().contains("strategy halt"));
        assert!(decoded.transcript_text().contains("token budget 75"));
    }

    #[test]
    fn scheduled_turn_is_due_only_after_interval() {
        let settings = enabled_settings();
        let mut governor = WakeupGovernor::default();
        governor.register_session("thread-1", 10_000);
        assert_eq!(
            governor
                .scheduled_wakeup("thread-1", 69_999, &settings)
                .expect("check schedule"),
            None
        );
        let wakeup = governor
            .scheduled_wakeup("thread-1", 70_000, &settings)
            .expect("check schedule")
            .expect("scheduled wakeup");
        assert!(matches!(
            wakeup.source,
            WakeupSource::ScheduledReview { .. }
        ));
        assert_eq!(
            governor
                .scheduled_wakeup("thread-1", 70_001, &settings)
                .expect("check schedule"),
            None
        );
    }

    #[test]
    fn governor_prevents_concurrent_and_runaway_turns() {
        let settings = enabled_settings();
        let mut governor = WakeupGovernor::default();
        let first = governor
            .admit(&event(1_000, 75), 1_000, &settings)
            .expect("admit first turn");
        assert_eq!(
            governor.admit(&event(1_001, 50), 1_001, &settings),
            Err(WakeupRefusal::TurnInFlight)
        );
        governor.finish(&first);
        assert_eq!(
            governor.admit(&event(1_002, 76), 1_002, &settings),
            Err(WakeupRefusal::HourlyTokenCap)
        );
        let second = governor
            .admit(&event(1_003, 75), 1_003, &settings)
            .expect("admit second turn");
        governor.finish(&second);
        assert_eq!(
            governor.admit(&event(1_004, 1), 1_004, &settings),
            Err(WakeupRefusal::FrequencyCap)
        );
        assert!(
            governor
                .admit(
                    &event(ONE_HOUR_MS + 1_004, 100),
                    ONE_HOUR_MS + 1_004,
                    &settings
                )
                .is_ok()
        );
    }

    #[test]
    fn disabled_and_per_turn_caps_fail_closed() {
        let mut settings = enabled_settings();
        let mut governor = WakeupGovernor::default();
        assert_eq!(
            governor.admit(&event(1_000, 101), 1_000, &settings),
            Err(WakeupRefusal::PerTurnTokenCap)
        );
        settings.enabled = false;
        assert_eq!(
            governor.admit(&event(1_000, 50), 1_000, &settings),
            Err(WakeupRefusal::Disabled)
        );
    }
}
