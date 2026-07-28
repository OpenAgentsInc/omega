use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result};
use async_tungstenite::tungstenite::Message;
use futures::{FutureExt as _, StreamExt as _, pin_mut, select};
use serde_json::{Value, json};

use crate::omega_public_channel_timeline::{NostrEventRecord, stable_verified_events};

const RECONNECT_DELAY_MS: u64 = 1_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayAdmissionLimits {
    pub content_bytes: usize,
    pub event_bytes: usize,
    pub future_skew_seconds: u64,
    pub max_age_seconds: u64,
    pub tags: usize,
}

impl Default for RelayAdmissionLimits {
    fn default() -> Self {
        Self {
            content_bytes: 8_192,
            event_bytes: 32_768,
            future_skew_seconds: 60,
            max_age_seconds: 7 * 24 * 60 * 60,
            tags: 64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelaySessionConfig {
    pub relay_url: String,
    pub group_id: String,
    pub accepted_kinds: Vec<u16>,
    pub group_state_kinds: Vec<u16>,
    pub moderation_kinds: Vec<u16>,
    pub expected_relay_self_pubkey: Option<String>,
    pub history_page_size: usize,
    pub limits: RelayAdmissionLimits,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RelayLifecycle {
    #[default]
    Disconnected,
    Connecting,
    Replaying,
    Current,
    Reconnecting,
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayGapReason {
    AwaitingEose,
    RelayUnavailable,
    DisconnectBeforeEose,
    InvalidRelayFrame,
    InvalidEvent,
    SignatureInvalid,
    WrongGroup,
    WrongKind,
    EventTooLarge,
    ContentTooLarge,
    TooManyTags,
    StaleEvent,
    FutureEvent,
    RelaySelfUnavailable,
    SubscriptionClosed,
    RelayNotice,
    PaginationBoundarySaturated,
}

impl RelayGapReason {
    pub fn gap_label(self) -> &'static str {
        match self {
            Self::AwaitingEose => "Waiting for signed relay history.",
            Self::RelayUnavailable => "The relay is unavailable.",
            Self::DisconnectBeforeEose => "The relay disconnected before history was current.",
            Self::InvalidRelayFrame => "The relay sent an invalid frame.",
            Self::InvalidEvent => "The relay sent an invalid event.",
            Self::SignatureInvalid => "An event signature was invalid.",
            Self::WrongGroup => "An event was for a different group.",
            Self::WrongKind => "An event kind was not accepted.",
            Self::EventTooLarge => "An event exceeded the channel limit.",
            Self::ContentTooLarge => "Event content exceeded the channel limit.",
            Self::TooManyTags => "An event had too many tags.",
            Self::StaleEvent => "An event was older than the channel limit.",
            Self::FutureEvent => "An event time was too far in the future.",
            Self::RelaySelfUnavailable => "Group metadata could not be authenticated.",
            Self::SubscriptionClosed => "The relay closed the channel subscription.",
            Self::RelayNotice => "The relay reported a channel problem.",
            Self::PaginationBoundarySaturated => {
                "Older history could be incomplete at a one-second boundary."
            }
        }
    }
}

impl fmt::Display for RelayGapReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.gap_label())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelayCursor {
    pub created_at: i64,
    pub event_ids_at_created_at: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelaySnapshot {
    pub lifecycle: RelayLifecycle,
    pub gap_reason: Option<RelayGapReason>,
    pub last_current_at: Option<u64>,
    pub events: Vec<NostrEventRecord>,
    pub cursor: Option<RelayCursor>,
    pub metadata_trusted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayInput {
    ConnectRequested { now_ms: u64 },
    Connected { now_ms: u64 },
    TextFrame { text: String, now_ms: u64 },
    Disconnected { now_ms: u64 },
    ReconnectTimerFired { now_ms: u64 },
    LoadOlder { now_ms: u64 },
    CloseRequested { now_ms: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayCommand {
    Connect { relay_url: String },
    SendText(String),
    ScheduleReconnect { after_ms: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayIntent {
    LoadOlder,
    Close,
}

#[derive(Clone, Debug)]
struct PageRequest {
    subscription_id: String,
    until: i64,
    event_ids_before: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct RelaySession {
    config: RelaySessionConfig,
    lifecycle: RelayLifecycle,
    gap_reason: Option<RelayGapReason>,
    last_current_at: Option<u64>,
    events: BTreeMap<String, NostrEventRecord>,
    awaiting_eose: BTreeSet<String>,
    history_subscription_id: Option<String>,
    state_subscription_id: Option<String>,
    profile_subscription_ids: BTreeSet<String>,
    profile_authors: BTreeSet<String>,
    active_page: Option<PageRequest>,
    pagination_until_override: Option<i64>,
    sequence: u64,
    closed: bool,
}

impl RelaySession {
    pub fn new(config: RelaySessionConfig) -> Self {
        Self {
            config,
            lifecycle: RelayLifecycle::Disconnected,
            gap_reason: None,
            last_current_at: None,
            events: BTreeMap::new(),
            awaiting_eose: BTreeSet::new(),
            history_subscription_id: None,
            state_subscription_id: None,
            profile_subscription_ids: BTreeSet::new(),
            profile_authors: BTreeSet::new(),
            active_page: None,
            pagination_until_override: None,
            sequence: 0,
            closed: false,
        }
    }

    pub fn config(&self) -> &RelaySessionConfig {
        &self.config
    }

    pub fn snapshot(&self) -> RelaySnapshot {
        let events = stable_verified_events(
            &self
                .events
                .values()
                .cloned()
                .collect::<Vec<NostrEventRecord>>(),
        );
        let cursor = events.last().map(|last| RelayCursor {
            created_at: last.created_at,
            event_ids_at_created_at: events
                .iter()
                .filter(|event| event.created_at == last.created_at)
                .map(|event| event.id.clone())
                .collect(),
        });
        RelaySnapshot {
            lifecycle: self.lifecycle,
            gap_reason: self.gap_reason,
            last_current_at: self.last_current_at,
            events,
            cursor,
            metadata_trusted: self.config.expected_relay_self_pubkey.is_some(),
        }
    }

    pub fn apply(&mut self, input: RelayInput) -> Vec<RelayCommand> {
        match input {
            RelayInput::ConnectRequested { now_ms } => self.connect_requested(now_ms),
            RelayInput::Connected { now_ms } => self.connected(now_ms),
            RelayInput::TextFrame { text, now_ms } => self.text_frame(&text, now_ms),
            RelayInput::Disconnected { now_ms } => self.disconnected(now_ms),
            RelayInput::ReconnectTimerFired { now_ms: _ } => {
                if !self.closed {
                    self.lifecycle = RelayLifecycle::Reconnecting;
                    self.gap_reason = Some(RelayGapReason::AwaitingEose);
                }
                Vec::new()
            }
            RelayInput::LoadOlder { now_ms } => self.load_older(now_ms),
            RelayInput::CloseRequested { now_ms } => self.close_requested(now_ms),
        }
    }

    fn connect_requested(&mut self, _now_ms: u64) -> Vec<RelayCommand> {
        if self.closed {
            return Vec::new();
        }
        self.lifecycle = RelayLifecycle::Connecting;
        self.gap_reason = Some(RelayGapReason::AwaitingEose);
        vec![RelayCommand::Connect {
            relay_url: self.config.relay_url.clone(),
        }]
    }

    fn connected(&mut self, _now_ms: u64) -> Vec<RelayCommand> {
        if self.closed {
            return Vec::new();
        }
        self.lifecycle = RelayLifecycle::Replaying;
        self.gap_reason = Some(RelayGapReason::AwaitingEose);
        self.awaiting_eose.clear();
        self.profile_subscription_ids.clear();
        self.active_page = None;

        let history_subscription_id = self.next_subscription_id("history");
        self.awaiting_eose.insert(history_subscription_id.clone());
        self.history_subscription_id = Some(history_subscription_id.clone());
        let latest = self.events.values().map(|event| event.created_at).max();
        let mut history_filter = json!({
            "#h": [self.config.group_id],
            "kinds": self.history_kinds(),
            "limit": self.config.history_page_size,
        });
        if let Some(latest) = latest {
            history_filter["since"] = json!(latest.saturating_sub(1));
        }
        let mut commands = vec![RelayCommand::SendText(
            json!(["REQ", history_subscription_id, history_filter]).to_string(),
        )];

        if let Some(relay_self_public_key) = self.config.expected_relay_self_pubkey.clone() {
            let state_subscription_id = self.next_subscription_id("state");
            self.awaiting_eose.insert(state_subscription_id.clone());
            self.state_subscription_id = Some(state_subscription_id.clone());
            commands.push(RelayCommand::SendText(
                json!([
                    "REQ",
                    state_subscription_id,
                    {
                        "#d": [self.config.group_id],
                        "authors": [relay_self_public_key],
                        "kinds": self.config.group_state_kinds,
                    }
                ])
                .to_string(),
            ));
        } else {
            self.state_subscription_id = None;
        }
        commands
    }

    fn disconnected(&mut self, _now_ms: u64) -> Vec<RelayCommand> {
        if self.closed {
            return Vec::new();
        }
        let reason = if self.lifecycle == RelayLifecycle::Replaying {
            RelayGapReason::DisconnectBeforeEose
        } else {
            RelayGapReason::RelayUnavailable
        };
        self.lifecycle = RelayLifecycle::Stale;
        self.gap_reason = Some(reason);
        self.awaiting_eose.clear();
        self.history_subscription_id = None;
        self.state_subscription_id = None;
        self.profile_subscription_ids.clear();
        self.active_page = None;
        vec![RelayCommand::ScheduleReconnect {
            after_ms: RECONNECT_DELAY_MS,
        }]
    }

    fn load_older(&mut self, _now_ms: u64) -> Vec<RelayCommand> {
        if self.closed
            || self.active_page.is_some()
            || !matches!(
                self.lifecycle,
                RelayLifecycle::Replaying | RelayLifecycle::Current
            )
        {
            return Vec::new();
        }
        let oldest = self
            .events
            .values()
            .filter(|event| self.config.accepted_kinds.contains(&event.kind))
            .map(|event| event.created_at)
            .min();
        let Some(until) = self.pagination_until_override.take().or(oldest) else {
            return Vec::new();
        };
        let subscription_id = self.next_subscription_id("page");
        self.active_page = Some(PageRequest {
            subscription_id: subscription_id.clone(),
            until,
            event_ids_before: self.events.keys().cloned().collect(),
        });
        vec![RelayCommand::SendText(
            json!([
                "REQ",
                subscription_id,
                {
                    "#h": [self.config.group_id],
                    "kinds": self.history_kinds(),
                    "limit": self.config.history_page_size,
                    "until": until,
                }
            ])
            .to_string(),
        )]
    }

    fn close_requested(&mut self, _now_ms: u64) -> Vec<RelayCommand> {
        self.closed = true;
        self.lifecycle = RelayLifecycle::Disconnected;
        self.gap_reason = None;
        let mut subscription_ids = BTreeSet::new();
        subscription_ids.extend(self.history_subscription_id.take());
        subscription_ids.extend(self.state_subscription_id.take());
        subscription_ids.append(&mut self.profile_subscription_ids);
        if let Some(page) = self.active_page.take() {
            subscription_ids.insert(page.subscription_id);
        }
        self.awaiting_eose.clear();
        subscription_ids
            .into_iter()
            .map(|subscription_id| {
                RelayCommand::SendText(json!(["CLOSE", subscription_id]).to_string())
            })
            .collect()
    }

    fn text_frame(&mut self, text: &str, now_ms: u64) -> Vec<RelayCommand> {
        if text.len() > self.config.limits.event_bytes.saturating_add(1_024) {
            self.gap_reason = Some(RelayGapReason::InvalidRelayFrame);
            return Vec::new();
        }
        let Ok(Value::Array(frame)) = serde_json::from_str::<Value>(text) else {
            self.gap_reason = Some(RelayGapReason::InvalidRelayFrame);
            return Vec::new();
        };
        let Some(label) = frame.first().and_then(Value::as_str) else {
            self.gap_reason = Some(RelayGapReason::InvalidRelayFrame);
            return Vec::new();
        };
        match label {
            "EVENT" => self.event_frame(&frame, now_ms),
            "EOSE" => self.eose_frame(&frame, now_ms),
            "CLOSED" => self.closed_frame(&frame),
            "NOTICE" => {
                self.gap_reason = Some(RelayGapReason::RelayNotice);
                Vec::new()
            }
            "AUTH" | "OK" => Vec::new(),
            _ => Vec::new(),
        }
    }

    fn event_frame(&mut self, frame: &[Value], now_ms: u64) -> Vec<RelayCommand> {
        let Some(subscription_id) = frame.get(1).and_then(Value::as_str) else {
            self.gap_reason = Some(RelayGapReason::InvalidRelayFrame);
            return Vec::new();
        };
        if !self.owns_subscription(subscription_id) {
            return Vec::new();
        }
        let Some(event_value) = frame.get(2) else {
            self.gap_reason = Some(RelayGapReason::InvalidRelayFrame);
            return Vec::new();
        };
        let event_bytes = serde_json::to_vec(event_value)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX);
        if event_bytes > self.config.limits.event_bytes {
            self.gap_reason = Some(RelayGapReason::EventTooLarge);
            return Vec::new();
        }
        let Ok(event) = serde_json::from_value::<NostrEventRecord>(event_value.clone()) else {
            self.gap_reason = Some(RelayGapReason::InvalidEvent);
            return Vec::new();
        };
        let is_profile = self.profile_subscription_ids.contains(subscription_id);
        let is_state = self.state_subscription_id.as_deref() == Some(subscription_id);
        let admission = if is_profile {
            self.validate_profile(&event)
        } else if is_state {
            self.validate_group_state(&event)
        } else if self.config.moderation_kinds.contains(&event.kind) {
            self.validate_moderation(&event)
        } else {
            self.validate_channel_event(&event, now_ms)
        };
        if let Err(reason) = admission {
            self.gap_reason = Some(reason);
            return Vec::new();
        }

        let is_new = self
            .events
            .insert(event.id.clone(), event.clone())
            .is_none();
        if !is_new
            || is_profile
            || !self.config.accepted_kinds.contains(&event.kind)
            || !self.profile_authors.insert(event.public_key.clone())
        {
            return Vec::new();
        }
        let profile_subscription_id = self.next_subscription_id("profile");
        self.profile_subscription_ids
            .insert(profile_subscription_id.clone());
        vec![RelayCommand::SendText(
            json!([
                "REQ",
                profile_subscription_id,
                {
                    "authors": [event.public_key],
                    "kinds": [0],
                    "limit": 1,
                }
            ])
            .to_string(),
        )]
    }

    fn eose_frame(&mut self, frame: &[Value], now_ms: u64) -> Vec<RelayCommand> {
        let Some(subscription_id) = frame.get(1).and_then(Value::as_str) else {
            self.gap_reason = Some(RelayGapReason::InvalidRelayFrame);
            return Vec::new();
        };
        if self.awaiting_eose.remove(subscription_id) {
            if self.awaiting_eose.is_empty() {
                self.lifecycle = RelayLifecycle::Current;
                self.gap_reason = None;
                self.last_current_at = Some(now_ms);
            }
            return Vec::new();
        }
        if self.profile_subscription_ids.remove(subscription_id) {
            return vec![RelayCommand::SendText(
                json!(["CLOSE", subscription_id]).to_string(),
            )];
        }
        if let Some(page) = self
            .active_page
            .take_if(|page| page.subscription_id == subscription_id)
        {
            let has_new_event = self
                .events
                .keys()
                .any(|event_id| !page.event_ids_before.contains(event_id));
            if !has_new_event {
                self.pagination_until_override = Some(page.until.saturating_sub(1));
                self.gap_reason = Some(RelayGapReason::PaginationBoundarySaturated);
            } else if self.gap_reason == Some(RelayGapReason::PaginationBoundarySaturated) {
                self.gap_reason = None;
            }
            return vec![RelayCommand::SendText(
                json!(["CLOSE", subscription_id]).to_string(),
            )];
        }
        Vec::new()
    }

    fn closed_frame(&mut self, frame: &[Value]) -> Vec<RelayCommand> {
        let Some(subscription_id) = frame.get(1).and_then(Value::as_str) else {
            self.gap_reason = Some(RelayGapReason::InvalidRelayFrame);
            return Vec::new();
        };
        if self.owns_subscription(subscription_id) {
            self.lifecycle = RelayLifecycle::Stale;
            self.gap_reason = Some(RelayGapReason::SubscriptionClosed);
            self.awaiting_eose.clear();
            return vec![RelayCommand::ScheduleReconnect {
                after_ms: RECONNECT_DELAY_MS,
            }];
        }
        Vec::new()
    }

    fn validate_profile(&self, event: &NostrEventRecord) -> Result<(), RelayGapReason> {
        if event.kind != 0 {
            return Err(RelayGapReason::WrongKind);
        }
        if !event.is_verified() {
            return Err(RelayGapReason::SignatureInvalid);
        }
        Ok(())
    }

    fn validate_group_state(&self, event: &NostrEventRecord) -> Result<(), RelayGapReason> {
        let Some(relay_self_public_key) = &self.config.expected_relay_self_pubkey else {
            return Err(RelayGapReason::RelaySelfUnavailable);
        };
        if event.public_key != *relay_self_public_key
            || !self.config.group_state_kinds.contains(&event.kind)
            || !event.has_tag("d", &self.config.group_id)
        {
            return Err(RelayGapReason::RelaySelfUnavailable);
        }
        if !event.is_verified() {
            return Err(RelayGapReason::SignatureInvalid);
        }
        Ok(())
    }

    fn validate_moderation(&self, event: &NostrEventRecord) -> Result<(), RelayGapReason> {
        if !event.has_tag("h", &self.config.group_id) {
            return Err(RelayGapReason::WrongGroup);
        }
        if !event.is_verified() {
            return Err(RelayGapReason::SignatureInvalid);
        }
        Ok(())
    }

    fn validate_channel_event(
        &self,
        event: &NostrEventRecord,
        now_ms: u64,
    ) -> Result<(), RelayGapReason> {
        if !self.config.accepted_kinds.contains(&event.kind) {
            return Err(RelayGapReason::WrongKind);
        }
        if !event.has_tag("h", &self.config.group_id) {
            return Err(RelayGapReason::WrongGroup);
        }
        if event.tags.len() > self.config.limits.tags {
            return Err(RelayGapReason::TooManyTags);
        }
        if event.content.len() > self.config.limits.content_bytes {
            return Err(RelayGapReason::ContentTooLarge);
        }
        let now_seconds = i64::try_from(now_ms / 1_000).unwrap_or(i64::MAX);
        let future_limit = now_seconds.saturating_add(
            i64::try_from(self.config.limits.future_skew_seconds).unwrap_or(i64::MAX),
        );
        if event.created_at > future_limit {
            return Err(RelayGapReason::FutureEvent);
        }
        let oldest = now_seconds
            .saturating_sub(i64::try_from(self.config.limits.max_age_seconds).unwrap_or(i64::MAX));
        if event.created_at < oldest {
            return Err(RelayGapReason::StaleEvent);
        }
        if !event.is_verified() {
            return Err(RelayGapReason::SignatureInvalid);
        }
        Ok(())
    }

    fn owns_subscription(&self, subscription_id: &str) -> bool {
        self.history_subscription_id.as_deref() == Some(subscription_id)
            || self.state_subscription_id.as_deref() == Some(subscription_id)
            || self.profile_subscription_ids.contains(subscription_id)
            || self
                .active_page
                .as_ref()
                .is_some_and(|page| page.subscription_id == subscription_id)
    }

    fn history_kinds(&self) -> Vec<u16> {
        let mut kinds = self.config.accepted_kinds.clone();
        kinds.extend(self.config.moderation_kinds.iter().copied());
        kinds.sort_unstable();
        kinds.dedup();
        kinds
    }

    fn next_subscription_id(&mut self, role: &str) -> String {
        self.sequence = self.sequence.saturating_add(1);
        format!("omega-public-{role}-{}", self.sequence)
    }
}

pub async fn run_relay_session(
    config: RelaySessionConfig,
    intent_receiver: async_channel::Receiver<RelayIntent>,
    snapshot_sender: async_channel::Sender<RelaySnapshot>,
) -> Result<()> {
    let mut session = RelaySession::new(config);
    emit_snapshot(&session, &snapshot_sender).await?;
    let mut reconnect = false;
    loop {
        if reconnect {
            if wait_for_reconnect_or_close(&mut session, &intent_receiver, &snapshot_sender).await?
            {
                return Ok(());
            }
        }

        let commands = session.apply(RelayInput::ConnectRequested {
            now_ms: unix_time_ms(),
        });
        emit_snapshot(&session, &snapshot_sender).await?;
        let relay_url = commands.iter().find_map(|command| match command {
            RelayCommand::Connect { relay_url } => Some(relay_url.clone()),
            RelayCommand::SendText(_) | RelayCommand::ScheduleReconnect { .. } => None,
        });
        let Some(relay_url) = relay_url else {
            return Ok(());
        };

        let Some(mut socket) =
            connect_or_close(&relay_url, &intent_receiver, &mut session, &snapshot_sender).await?
        else {
            if session.snapshot().lifecycle == RelayLifecycle::Disconnected {
                return Ok(());
            }
            let commands = session.apply(RelayInput::Disconnected {
                now_ms: unix_time_ms(),
            });
            reconnect = commands
                .iter()
                .any(|command| matches!(command, RelayCommand::ScheduleReconnect { .. }));
            emit_snapshot(&session, &snapshot_sender).await?;
            continue;
        };

        let commands = session.apply(RelayInput::Connected {
            now_ms: unix_time_ms(),
        });
        emit_snapshot(&session, &snapshot_sender).await?;
        if send_text_commands(&mut socket, &commands).await.is_err() {
            let commands = session.apply(RelayInput::Disconnected {
                now_ms: unix_time_ms(),
            });
            reconnect = commands
                .iter()
                .any(|command| matches!(command, RelayCommand::ScheduleReconnect { .. }));
            emit_snapshot(&session, &snapshot_sender).await?;
            continue;
        }

        let closed = drive_connected_socket(
            &mut session,
            &mut socket,
            &intent_receiver,
            &snapshot_sender,
        )
        .await?;
        if closed {
            return Ok(());
        }
        let commands = session.apply(RelayInput::Disconnected {
            now_ms: unix_time_ms(),
        });
        reconnect = commands
            .iter()
            .any(|command| matches!(command, RelayCommand::ScheduleReconnect { .. }));
        emit_snapshot(&session, &snapshot_sender).await?;
    }
}

async fn connect_or_close(
    relay_url: &str,
    intent_receiver: &async_channel::Receiver<RelayIntent>,
    session: &mut RelaySession,
    snapshot_sender: &async_channel::Sender<RelaySnapshot>,
) -> Result<Option<async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>>>
{
    let connect = async_tungstenite::async_std::connect_async(relay_url).fuse();
    let timeout = futures::FutureExt::fuse(smol::Timer::after(CONNECT_TIMEOUT));
    pin_mut!(connect, timeout);
    loop {
        let intent = intent_receiver.recv().fuse();
        pin_mut!(intent);
        select! {
            result = connect => {
                return match result {
                    Ok((socket, _)) => Ok(Some(socket)),
                    Err(error) => {
                        log::debug!("public channel relay connect failed: {error}");
                        Ok(None)
                    }
                };
            }
            _ = timeout => return Ok(None),
            intent = intent => match intent {
                Ok(RelayIntent::LoadOlder) => {}
                Ok(RelayIntent::Close) | Err(_) => {
                    session.apply(RelayInput::CloseRequested { now_ms: unix_time_ms() });
                    emit_snapshot(session, snapshot_sender).await?;
                    return Ok(None);
                }
            }
        }
    }
}

async fn wait_for_reconnect_or_close(
    session: &mut RelaySession,
    intent_receiver: &async_channel::Receiver<RelayIntent>,
    snapshot_sender: &async_channel::Sender<RelaySnapshot>,
) -> Result<bool> {
    let timer = futures::FutureExt::fuse(smol::Timer::after(Duration::from_millis(
        RECONNECT_DELAY_MS,
    )));
    pin_mut!(timer);
    loop {
        let intent = intent_receiver.recv().fuse();
        pin_mut!(intent);
        select! {
            _ = timer => {
                session.apply(RelayInput::ReconnectTimerFired { now_ms: unix_time_ms() });
                emit_snapshot(session, snapshot_sender).await?;
                return Ok(false);
            }
            intent = intent => match intent {
                Ok(RelayIntent::LoadOlder) => {}
                Ok(RelayIntent::Close) | Err(_) => {
                    session.apply(RelayInput::CloseRequested { now_ms: unix_time_ms() });
                    emit_snapshot(session, snapshot_sender).await?;
                    return Ok(true);
                }
            }
        }
    }
}

async fn drive_connected_socket(
    session: &mut RelaySession,
    socket: &mut async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>,
    intent_receiver: &async_channel::Receiver<RelayIntent>,
    snapshot_sender: &async_channel::Sender<RelaySnapshot>,
) -> Result<bool> {
    loop {
        let incoming = socket.next().fuse();
        let intent = intent_receiver.recv().fuse();
        pin_mut!(incoming, intent);
        select! {
            incoming = incoming => {
                let Some(incoming) = incoming else {
                    return Ok(false);
                };
                let incoming = match incoming {
                    Ok(incoming) => incoming,
                    Err(error) => {
                        log::debug!("public channel relay read failed: {error}");
                        return Ok(false);
                    }
                };
                let text = match incoming {
                    Message::Text(text) => Some(text.to_string()),
                    Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                        Ok(text) => Some(text),
                        Err(error) => {
                            log::debug!("public channel relay sent non-UTF-8 binary data: {error}");
                            session.apply(RelayInput::TextFrame {
                                text: String::new(),
                                now_ms: unix_time_ms(),
                            });
                            emit_snapshot(session, snapshot_sender).await?;
                            None
                        }
                    },
                    Message::Ping(payload) => {
                        if let Err(error) = socket.send(Message::Pong(payload)).await {
                            log::debug!("public channel relay pong failed: {error}");
                            return Ok(false);
                        }
                        None
                    }
                    Message::Pong(_) | Message::Frame(_) => None,
                    Message::Close(_) => return Ok(false),
                };
                if let Some(text) = text {
                    let commands = session.apply(RelayInput::TextFrame {
                        text,
                        now_ms: unix_time_ms(),
                    });
                    emit_snapshot(session, snapshot_sender).await?;
                    if let Err(error) = send_text_commands(socket, &commands).await {
                        log::debug!("public channel relay write failed: {error:#}");
                        return Ok(false);
                    }
                    if commands.iter().any(|command| {
                        matches!(command, RelayCommand::ScheduleReconnect { .. })
                    }) {
                        return Ok(false);
                    }
                }
            }
            intent = intent => match intent {
                Ok(RelayIntent::LoadOlder) => {
                    let commands = session.apply(RelayInput::LoadOlder {
                        now_ms: unix_time_ms(),
                    });
                    emit_snapshot(session, snapshot_sender).await?;
                    if let Err(error) = send_text_commands(socket, &commands).await {
                        log::debug!("public channel pagination write failed: {error:#}");
                        return Ok(false);
                    }
                }
                Ok(RelayIntent::Close) | Err(_) => {
                    let commands = session.apply(RelayInput::CloseRequested {
                        now_ms: unix_time_ms(),
                    });
                    emit_snapshot(session, snapshot_sender).await?;
                    for command in &commands {
                        let RelayCommand::SendText(text) = command else {
                            continue;
                        };
                        if let Err(error) = socket.send(Message::Text(text.clone().into())).await {
                            log::debug!("public channel CLOSE frame failed: {error}");
                        }
                    }
                    if let Err(error) = socket.close(None).await {
                        log::debug!("public channel socket close failed: {error}");
                    }
                    return Ok(true);
                }
            }
        }
    }
}

async fn send_text_commands(
    socket: &mut async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>,
    commands: &[RelayCommand],
) -> Result<()> {
    for command in commands {
        if let RelayCommand::SendText(text) = command {
            socket
                .send(Message::Text(text.clone().into()))
                .await
                .context("writing to the public channel relay")?;
        }
    }
    Ok(())
}

async fn emit_snapshot(
    session: &RelaySession,
    snapshot_sender: &async_channel::Sender<RelaySnapshot>,
) -> Result<()> {
    snapshot_sender
        .send(session.snapshot())
        .await
        .context("sending the public channel snapshot")
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fixture {
        lifecycle: FixtureLifecycle,
        projection: FixtureProjection,
        source: FixtureSource,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureLifecycle {
        history_page_size: usize,
        latest_event_created_at: i64,
        reconnect_delay_ms: u64,
        reconnect_overlap_seconds: i64,
        relay_self_pubkey: String,
    }

    #[derive(Deserialize)]
    struct FixtureProjection {
        events: Vec<NostrEventRecord>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureSource {
        group_id: String,
        relay_url: String,
    }

    fn fixture() -> Fixture {
        serde_json::from_str(include_str!("../fixtures/agent-chat-parity.v1.json"))
            .expect("the pinned OpenAgents fixture must decode")
    }

    fn config(fixture: &Fixture, relay_self: bool) -> RelaySessionConfig {
        RelaySessionConfig {
            relay_url: fixture.source.relay_url.clone(),
            group_id: fixture.source.group_id.clone(),
            accepted_kinds: vec![5, 7, 9, 1337, 1984],
            group_state_kinds: vec![39000, 39001, 39003, 39005],
            moderation_kinds: vec![9002, 9005, 9010],
            expected_relay_self_pubkey: relay_self
                .then(|| fixture.lifecycle.relay_self_pubkey.clone()),
            history_page_size: fixture.lifecycle.history_page_size,
            limits: RelayAdmissionLimits::default(),
        }
    }

    fn sent_frames(commands: &[RelayCommand]) -> Vec<Value> {
        commands
            .iter()
            .filter_map(|command| match command {
                RelayCommand::SendText(text) => serde_json::from_str(text).ok(),
                RelayCommand::Connect { .. } | RelayCommand::ScheduleReconnect { .. } => None,
            })
            .collect()
    }

    fn subscription_for(frames: &[Value], filter_name: &str) -> String {
        frames
            .iter()
            .find(|frame| {
                frame
                    .get(2)
                    .and_then(|filter| filter.get(filter_name))
                    .is_some()
            })
            .and_then(|frame| frame.get(1))
            .and_then(Value::as_str)
            .expect("subscription")
            .to_string()
    }

    fn event_frame(subscription_id: &str, event: &NostrEventRecord) -> String {
        json!(["EVENT", subscription_id, event]).to_string()
    }

    fn signed_message(
        keys: &Keys,
        group: &str,
        created_at: u64,
        content: &str,
    ) -> NostrEventRecord {
        let event = EventBuilder::new(Kind::Custom(9), content)
            .custom_created_at(Timestamp::from_secs(created_at))
            .tag(Tag::parse(["h", group]).expect("group tag"))
            .sign_with_keys(keys)
            .expect("signed message");
        serde_json::from_value(serde_json::to_value(event).expect("event JSON"))
            .expect("event record")
    }

    #[test]
    fn dual_eose_gates_current_and_accepts_extra_eose_fields() {
        let fixture = fixture();
        let now_ms = u64::try_from(fixture.lifecycle.latest_event_created_at)
            .expect("fixture time")
            .saturating_mul(1_000);
        let mut session = RelaySession::new(config(&fixture, true));
        assert!(matches!(
            session
                .apply(RelayInput::ConnectRequested { now_ms })
                .as_slice(),
            [RelayCommand::Connect { .. }]
        ));
        let commands = session.apply(RelayInput::Connected { now_ms });
        let frames = sent_frames(&commands);
        let history = subscription_for(&frames, "#h");
        let state = subscription_for(&frames, "#d");
        let message = fixture
            .projection
            .events
            .iter()
            .find(|event| event.kind == 9)
            .expect("message");
        let state_event = fixture
            .projection
            .events
            .iter()
            .find(|event| event.kind == 39001)
            .expect("state");
        session.apply(RelayInput::TextFrame {
            text: event_frame(&history, message),
            now_ms,
        });
        session.apply(RelayInput::TextFrame {
            text: event_frame(&state, state_event),
            now_ms,
        });
        session.apply(RelayInput::TextFrame {
            text: json!(["EOSE", history, ["more"]]).to_string(),
            now_ms,
        });
        assert_eq!(session.snapshot().lifecycle, RelayLifecycle::Replaying);
        session.apply(RelayInput::TextFrame {
            text: json!(["EOSE", state]).to_string(),
            now_ms,
        });
        assert_eq!(session.snapshot().lifecycle, RelayLifecycle::Current);
        assert_eq!(session.snapshot().last_current_at, Some(now_ms));
    }

    #[test]
    fn reconnect_uses_the_fixture_overlap_and_keeps_verified_events() {
        let fixture = fixture();
        let now_ms = u64::try_from(fixture.lifecycle.latest_event_created_at)
            .expect("fixture time")
            .saturating_mul(1_000);
        let mut session = RelaySession::new(config(&fixture, true));
        session.apply(RelayInput::ConnectRequested { now_ms });
        session.apply(RelayInput::Connected { now_ms });
        for event in &fixture.projection.events {
            assert!(event.is_verified());
            session.events.insert(event.id.clone(), event.clone());
        }
        let event_count = session.snapshot().events.len();
        let commands = session.apply(RelayInput::Disconnected { now_ms });
        assert_eq!(session.snapshot().lifecycle, RelayLifecycle::Stale);
        assert_eq!(
            commands,
            vec![RelayCommand::ScheduleReconnect {
                after_ms: fixture.lifecycle.reconnect_delay_ms
            }]
        );
        session.apply(RelayInput::ReconnectTimerFired { now_ms });
        assert_eq!(session.snapshot().lifecycle, RelayLifecycle::Reconnecting);
        session.apply(RelayInput::ConnectRequested { now_ms });
        let replay = sent_frames(&session.apply(RelayInput::Connected { now_ms }));
        let history_request = replay
            .iter()
            .find(|frame| frame.get(2).and_then(|filter| filter.get("#h")).is_some())
            .expect("history request");
        assert_eq!(
            history_request[2]["since"],
            fixture.lifecycle.latest_event_created_at - fixture.lifecycle.reconnect_overlap_seconds
        );
        assert_eq!(session.snapshot().events.len(), event_count);
    }

    #[test]
    fn missing_relay_key_needs_only_history_eose() {
        let fixture = fixture();
        let mut session = RelaySession::new(config(&fixture, false));
        session.apply(RelayInput::ConnectRequested { now_ms: 1 });
        let frames = sent_frames(&session.apply(RelayInput::Connected { now_ms: 1 }));
        assert_eq!(frames.len(), 1);
        let history = subscription_for(&frames, "#h");
        session.apply(RelayInput::TextFrame {
            text: json!(["EOSE", history]).to_string(),
            now_ms: 2,
        });
        let snapshot = session.snapshot();
        assert_eq!(snapshot.lifecycle, RelayLifecycle::Current);
        assert!(!snapshot.metadata_trusted);
    }

    #[test]
    fn failed_connect_becomes_stale_and_schedules_reconnect() {
        let fixture = fixture();
        let mut session = RelaySession::new(config(&fixture, false));
        session.apply(RelayInput::ConnectRequested { now_ms: 1 });
        let commands = session.apply(RelayInput::Disconnected { now_ms: 2 });
        assert_eq!(session.snapshot().lifecycle, RelayLifecycle::Stale);
        assert_eq!(
            session.snapshot().gap_reason,
            Some(RelayGapReason::RelayUnavailable)
        );
        assert_eq!(
            commands,
            vec![RelayCommand::ScheduleReconnect {
                after_ms: RECONNECT_DELAY_MS
            }]
        );
    }

    #[test]
    fn invalid_input_never_removes_verified_history() {
        let fixture = fixture();
        let now_ms = u64::try_from(fixture.lifecycle.latest_event_created_at)
            .expect("fixture time")
            .saturating_mul(1_000);
        let mut session = RelaySession::new(config(&fixture, false));
        session.apply(RelayInput::ConnectRequested { now_ms });
        let frames = sent_frames(&session.apply(RelayInput::Connected { now_ms }));
        let history = subscription_for(&frames, "#h");
        let message = fixture
            .projection
            .events
            .iter()
            .find(|event| event.kind == 9)
            .expect("message");
        session.apply(RelayInput::TextFrame {
            text: event_frame(&history, message),
            now_ms,
        });
        session.apply(RelayInput::TextFrame {
            text: "not-json".into(),
            now_ms,
        });
        assert_eq!(session.snapshot().events, vec![message.clone()]);
        assert_eq!(
            session.snapshot().gap_reason,
            Some(RelayGapReason::InvalidRelayFrame)
        );
    }

    #[test]
    fn pagination_retains_same_time_events_then_moves_past_a_repeated_boundary() {
        let fixture = fixture();
        let keys = Keys::generate();
        let first = signed_message(&keys, &fixture.source.group_id, 100, "first");
        let second = signed_message(&keys, &fixture.source.group_id, 100, "second");
        let now_ms = 101_000;
        let mut session = RelaySession::new(config(&fixture, false));
        session.apply(RelayInput::ConnectRequested { now_ms });
        let frames = sent_frames(&session.apply(RelayInput::Connected { now_ms }));
        let history = subscription_for(&frames, "#h");
        session.apply(RelayInput::TextFrame {
            text: event_frame(&history, &first),
            now_ms,
        });
        session.apply(RelayInput::TextFrame {
            text: json!(["EOSE", history]).to_string(),
            now_ms,
        });

        let first_page = sent_frames(&session.apply(RelayInput::LoadOlder { now_ms }));
        let page = subscription_for(&first_page, "#h");
        assert_eq!(first_page[0][2]["until"], 100);
        session.apply(RelayInput::TextFrame {
            text: event_frame(&page, &second),
            now_ms,
        });
        session.apply(RelayInput::TextFrame {
            text: json!(["EOSE", page]).to_string(),
            now_ms,
        });
        assert_eq!(session.snapshot().events.len(), 2);

        let repeated_page = sent_frames(&session.apply(RelayInput::LoadOlder { now_ms }));
        let repeated = subscription_for(&repeated_page, "#h");
        assert_eq!(repeated_page[0][2]["until"], 100);
        session.apply(RelayInput::TextFrame {
            text: json!(["EOSE", repeated]).to_string(),
            now_ms,
        });
        assert_eq!(
            session.snapshot().gap_reason,
            Some(RelayGapReason::PaginationBoundarySaturated)
        );
        let next_page = sent_frames(&session.apply(RelayInput::LoadOlder { now_ms }));
        assert_eq!(next_page[0][2]["until"], 99);
    }

    #[test]
    fn auth_frames_do_not_create_signing_commands() {
        let fixture = fixture();
        let mut session = RelaySession::new(config(&fixture, false));
        assert!(
            session
                .apply(RelayInput::TextFrame {
                    text: json!(["AUTH", "challenge"]).to_string(),
                    now_ms: 0,
                })
                .is_empty()
        );
    }

    #[test]
    fn close_emits_close_for_each_live_subscription() {
        let fixture = fixture();
        let mut session = RelaySession::new(config(&fixture, true));
        session.apply(RelayInput::ConnectRequested { now_ms: 1 });
        let requests = sent_frames(&session.apply(RelayInput::Connected { now_ms: 1 }));
        assert_eq!(requests.len(), 2);

        let close_frames = sent_frames(&session.apply(RelayInput::CloseRequested { now_ms: 2 }));
        assert_eq!(close_frames.len(), 2);
        assert!(
            close_frames
                .iter()
                .all(|frame| frame.get(0) == Some(&Value::String("CLOSE".into())))
        );
        assert_eq!(session.snapshot().lifecycle, RelayLifecycle::Disconnected);
    }
}
