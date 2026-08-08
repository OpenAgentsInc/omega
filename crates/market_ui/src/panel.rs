//! The Markets dock panel (omega#244): NIP-MKT discovery over Omega's own
//! WebSocket against an Immortal relay, gated by NIP-11, plus the requester
//! session flow (RFQ → Quote → Order → Status → Cancel → Close) over an
//! authenticated gift-wrap lane.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use async_tungstenite::async_std::connect_async;
use async_tungstenite::tungstenite::Message;
use futures::{AsyncReadExt as _, StreamExt as _};
use gpui::{
    App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Task, WeakEntity, Window, px,
};
use http_client::{AsyncBody, HttpClient, Request};
use immortal_client::domain::Event;
use immortal_client::mkt_swp_client::ParticipantRole;
use ui::{Indicator, VizChip, VizChipTone, prelude::*};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::discovery::{
    ConnectionState, MAX_RELAY_INFORMATION_BYTES, MarketDiscovery, MarketDiscoveryConfig,
    MarketRelayGate, NIP11_ACCEPT_MEDIA_TYPE, OfferingListing, validate_market_relay_information,
};
use crate::session_flow::{
    MarketSession, SESSION_STORE_DIRECTORY, SessionPhase, StatusSlot, throwaway_session_signer,
    wrap_for_transport,
};
use crate::session_transport::{SessionSocketEvent, run_session_socket};
use crate::{Reconnect, ToggleFocus};

const PANEL_KEY: &str = "market";
const MAX_SESSION_DIAGNOSTICS: usize = 8;

pub struct MarketPanel {
    focus_handle: FocusHandle,
    config: MarketDiscoveryConfig,
    gate: Option<MarketRelayGate>,
    discovery: MarketDiscovery,
    session: Option<SessionState>,
    _connection_task: Option<Task<()>>,
}

struct SessionState {
    session: MarketSession,
    outgoing: async_channel::Sender<Event>,
    live: bool,
    last_error: Option<String>,
    diagnostics: VecDeque<String>,
    _task: Task<()>,
}

enum SocketFrame {
    Opened,
    Text(String),
}

impl MarketPanel {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            workspace.update_in(cx, |_workspace, _window, cx| cx.new(|cx| Self::new(cx)))
        })
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            config: MarketDiscoveryConfig::from_environment(),
            gate: None,
            discovery: MarketDiscovery::new(),
            session: None,
            _connection_task: None,
        };
        this.connect(cx);
        this
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        self.gate = None;
        if let Err(error) = self.config.validate() {
            self.discovery.gate_failed(error);
            cx.notify();
            return;
        }
        self.discovery.begin_gate_check();
        cx.notify();
        let config = self.config.clone();
        let http_client = cx.http_client();
        self._connection_task = Some(cx.spawn(async move |this, cx| {
            let gate = match fetch_market_relay_gate(http_client, &config).await {
                Ok(gate) => gate,
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.discovery.gate_failed(error);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let Ok(request) = this.update(cx, |this, cx| {
                this.discovery.begin_connect(&gate);
                this.gate = Some(gate);
                cx.notify();
                this.discovery.subscription_request()
            }) else {
                return;
            };
            let (frame_sender, frame_receiver) = async_channel::bounded(256);
            let socket_task = cx.background_spawn(run_socket(
                config.relay_websocket_url.clone(),
                request,
                frame_sender,
            ));
            while let Ok(frame) = frame_receiver.recv().await {
                let updated = this.update(cx, |this, cx| {
                    match frame {
                        SocketFrame::Opened => this.discovery.opened(),
                        SocketFrame::Text(text) => {
                            if let Err(error) = this.discovery.ingest_text(&text, unix_now()) {
                                log::warn!("market panel dropped a relay frame: {error}");
                            }
                        }
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    return;
                }
            }
            let reason = match socket_task.await {
                Ok(()) => "connection closed".to_owned(),
                Err(error) => error,
            };
            this.update(cx, |this, cx| {
                this.discovery.disconnected(reason);
                cx.notify();
            })
            .ok();
        }));
    }

    fn start_session(&mut self, offering: &OfferingListing, cx: &mut Context<Self>) {
        let now = unix_now();
        let session = throwaway_session_signer()
            .and_then(|signer| MarketSession::begin(signer, offering, now));
        let session = match session {
            Ok(session) => session,
            Err(error) => {
                self.discovery_diagnostic(error);
                cx.notify();
                return;
            }
        };
        let signer = session.signer().clone();
        let (outgoing_sender, outgoing_receiver) = async_channel::bounded(256);
        let (event_sender, event_receiver) = async_channel::bounded(256);
        let relay_url = self.config.relay_websocket_url.clone();
        let task = cx.spawn(async move |this, cx| {
            let socket_task = cx.background_spawn(run_session_socket(
                relay_url,
                signer,
                outgoing_receiver,
                event_sender,
                unix_now,
            ));
            while let Ok(event) = event_receiver.recv().await {
                let updated = this.update(cx, |this, cx| {
                    this.handle_session_event(event, cx);
                });
                if updated.is_err() {
                    return;
                }
            }
            let reason = match socket_task.await {
                Ok(()) => "session connection closed".to_owned(),
                Err(error) => error,
            };
            this.update(cx, |this, cx| {
                if let Some(session) = this.session.as_mut() {
                    session.live = false;
                    session.last_error = Some(reason);
                }
                cx.notify();
            })
            .ok();
        });
        self.session = Some(SessionState {
            session,
            outgoing: outgoing_sender,
            live: false,
            last_error: None,
            diagnostics: VecDeque::new(),
            _task: task,
        });
        self.persist_session();
        cx.notify();
    }

    fn handle_session_event(&mut self, event: SessionSocketEvent, cx: &mut Context<Self>) {
        let Some(state) = self.session.as_mut() else {
            return;
        };
        match event {
            SessionSocketEvent::Authenticated => {}
            SessionSocketEvent::SubscriptionLive => {
                state.live = true;
                // Replay every own signed record as fresh wraps; identical
                // inner bytes are idempotent for the counterparty.
                match state.session.replay_wraps(unix_now()) {
                    Ok(wraps) => {
                        for wrap in wraps {
                            if state.outgoing.try_send(wrap).is_err() {
                                state.last_error = Some("session publish queue is full".to_owned());
                                break;
                            }
                        }
                    }
                    Err(error) => state.last_error = Some(error),
                }
            }
            SessionSocketEvent::Delivered {
                delivered,
                observed_at,
            } => match state.session.admit_delivery(&delivered, observed_at) {
                Ok(_) => self.persist_session(),
                Err(error) => Self::push_session_diagnostic(state, error),
            },
            SessionSocketEvent::PublishResult {
                event_id,
                accepted,
                message,
            } => {
                if !accepted {
                    Self::push_session_diagnostic(
                        state,
                        format!("relay refused {event_id}: {message}"),
                    );
                }
            }
            SessionSocketEvent::Diagnostic(diagnostic) => {
                Self::push_session_diagnostic(state, diagnostic);
            }
        }
        cx.notify();
    }

    fn push_session_diagnostic(state: &mut SessionState, diagnostic: String) {
        if state.diagnostics.len() >= MAX_SESSION_DIAGNOSTICS {
            state.diagnostics.pop_front();
        }
        state.diagnostics.push_back(diagnostic);
    }

    fn publish_session_records(&mut self, records: Vec<Event>) {
        let Some(state) = self.session.as_mut() else {
            return;
        };
        let now = unix_now();
        for record in records {
            match wrap_for_transport(
                &record,
                state.session.signer(),
                state.session.provider_pubkey(),
                now,
            ) {
                Ok(wraps) => {
                    for wrap in wraps {
                        if state.outgoing.try_send(wrap.event).is_err() {
                            state.last_error = Some("session publish queue is full".to_owned());
                            return;
                        }
                    }
                }
                Err(error) => {
                    state.last_error = Some(error);
                    return;
                }
            }
        }
        self.persist_session();
    }

    fn order_selected_quote(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.session.as_mut() {
            match state.session.order_selected_quote(unix_now()) {
                Ok(records) => self.publish_session_records(records),
                Err(error) => state.last_error = Some(error),
            }
        }
        cx.notify();
    }

    fn request_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.session.as_mut() {
            match state.session.request_cancel(unix_now()) {
                Ok(record) => self.publish_session_records(vec![record]),
                Err(error) => state.last_error = Some(error),
            }
        }
        cx.notify();
    }

    fn close_session(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.session.as_mut() {
            match state.session.close_after_cancel(unix_now()) {
                Ok(record) => self.publish_session_records(vec![record]),
                Err(error) => state.last_error = Some(error),
            }
        }
        cx.notify();
    }

    fn end_session(&mut self, cx: &mut Context<Self>) {
        self.persist_session();
        self.session = None;
        cx.notify();
    }

    fn persist_session(&mut self) {
        let Some(state) = self.session.as_mut() else {
            return;
        };
        if let Err(error) = state.session.persist(&session_store_directory()) {
            state.last_error = Some(error);
        }
    }

    fn discovery_diagnostic(&mut self, diagnostic: String) {
        log::warn!("market panel: {diagnostic}");
    }

    fn connection_color(&self) -> Color {
        match self.discovery.connection() {
            ConnectionState::Idle => Color::Muted,
            ConnectionState::CheckingGate
            | ConnectionState::Connecting
            | ConnectionState::AwaitingSnapshot => Color::Warning,
            ConnectionState::Live => Color::Success,
            ConnectionState::GateFailed(_) | ConnectionState::Disconnected(_) => Color::Error,
        }
    }

    fn failure_reason(&self) -> Option<String> {
        match self.discovery.connection() {
            ConnectionState::GateFailed(reason) | ConnectionState::Disconnected(reason) => {
                Some(reason.clone())
            }
            _ => None,
        }
    }

    fn render_session(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let state = self.session.as_ref()?;
        let session = &state.session;
        let now = unix_now();
        let phase = session.phase();
        let selected = session.selected_quote(now);
        let short_id: String = session.session_id().chars().take(8).collect();
        let phase_tone = match phase {
            SessionPhase::AwaitingQuotes | SessionPhase::QuoteReceived => VizChipTone::Neutral,
            SessionPhase::OrderInFlight | SessionPhase::Active => VizChipTone::Active,
            SessionPhase::CancelRequested => VizChipTone::Warn,
            SessionPhase::Closed => VizChipTone::Ok,
        };

        let mut section = v_flex().gap_1p5().child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Label::new("Session")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(Indicator::dot().color(if state.live {
                            Color::Success
                        } else {
                            Color::Warning
                        }))
                        .child(
                            Label::new(format!("{short_id} · {}", session.offering_label()))
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(VizChip::new(phase.label()).tone(phase_tone).scale(1.0)),
                )
                .child(
                    Button::new("market-session-end", "End")
                        .label_size(LabelSize::Small)
                        .on_click(cx.listener(|this, _, _window, cx| this.end_session(cx))),
                ),
        );

        if !session.quotes().is_empty() {
            section = section.child(
                Label::new("Quotes")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
            for (index, quote) in session.quotes().iter().enumerate() {
                let is_selected = selected == Some(index);
                let class_tone = if quote.quote_class == "firm" {
                    VizChipTone::Ok
                } else {
                    VizChipTone::Neutral
                };
                let reservation_tone = match quote.reservation.as_str() {
                    "hard" => VizChipTone::Warn,
                    "soft" => VizChipTone::Active,
                    _ => VizChipTone::Neutral,
                };
                let expiry = if quote.expires_at > now {
                    format!("{}s", quote.expires_at - now)
                } else {
                    "expired".to_owned()
                };
                let mut row = h_flex()
                    .gap_1p5()
                    .items_center()
                    .flex_wrap()
                    .child(
                        VizChip::new(quote.quote_class.clone())
                            .kind(39_605)
                            .tone(class_tone)
                            .scale(1.0),
                    )
                    .child(
                        VizChip::new(quote.reservation.clone())
                            .tone(reservation_tone)
                            .scale(1.0),
                    )
                    .child(Label::new(format!(
                        "{} → {} sats",
                        quote.input_amount, quote.output_amount
                    )))
                    .child(
                        Label::new(format!("fee ≤ {} · {}", quote.maximum_total_fee, expiry))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    );
                if is_selected && session.can_order(now) {
                    row = row.child(
                        Button::new(("market-session-order", index), "Order")
                            .label_size(LabelSize::Small)
                            .on_click(
                                cx.listener(|this, _, _window, cx| this.order_selected_quote(cx)),
                            ),
                    );
                } else if is_selected {
                    row = row.child(
                        Label::new("selected")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    );
                }
                section = section.child(row);
            }
        }

        let lanes = session.status_lanes();
        if !lanes.is_empty() {
            section = section.child(
                Label::new("Status")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
            for lane in lanes {
                let role = match lane.role {
                    ParticipantRole::Requester => "requester",
                    ParticipantRole::Provider => "provider",
                };
                let mut row = h_flex()
                    .gap_1()
                    .items_center()
                    .flex_wrap()
                    .child(Label::new(role).size(LabelSize::XSmall).color(Color::Muted));
                for slot in &lane.slots {
                    match slot {
                        StatusSlot::Filled { sequence, entries } => {
                            let forked = entries.len() > 1;
                            for entry in entries {
                                let label = format!("s{sequence} {}", entry.state);
                                row = row.child(
                                    VizChip::new(label)
                                        .tone(if forked {
                                            VizChipTone::Warn
                                        } else {
                                            VizChipTone::Ok
                                        })
                                        .scale(1.0),
                                );
                            }
                            if forked {
                                row = row
                                    .child(VizChip::new("fork").tone(VizChipTone::Warn).scale(1.0));
                            }
                        }
                        StatusSlot::Gap { sequence } => {
                            row = row.child(
                                VizChip::new(format!("s{sequence} gap"))
                                    .tone(VizChipTone::Warn)
                                    .scale(1.0),
                            );
                        }
                    }
                }
                for entry in &lane.malformed {
                    row = row.child(
                        VizChip::new(format!("? {}", entry.state))
                            .tone(VizChipTone::Warn)
                            .scale(1.0),
                    );
                }
                section = section.child(row);
            }
        }

        let cancels = session.cancels();
        if !cancels.is_empty() {
            let mut row = h_flex().gap_1().items_center().flex_wrap().child(
                Label::new("cancel")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
            for cancel in cancels {
                row = row.child(
                    VizChip::new(cancel.action)
                        .kind(39_608)
                        .tone(VizChipTone::Warn)
                        .scale(1.0),
                );
            }
            section = section.child(row);
        }

        let closes = session.closes();
        if !closes.is_empty() {
            let mut row = h_flex().gap_1().items_center().flex_wrap().child(
                Label::new("close")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
            for close in closes {
                let role = match close.author {
                    ParticipantRole::Requester => "requester",
                    ParticipantRole::Provider => "provider",
                };
                row = row.child(
                    VizChip::new(format!("{role} {}", close.outcome))
                        .kind(39_609)
                        .tone(VizChipTone::Ok)
                        .scale(1.0),
                );
            }
            section = section.child(row);
        }

        let mut controls = h_flex().gap_2().items_center();
        if session.can_cancel() {
            controls = controls.child(
                Button::new("market-session-cancel", "Cancel")
                    .label_size(LabelSize::Small)
                    .on_click(cx.listener(|this, _, _window, cx| this.request_cancel(cx))),
            );
        }
        if session.can_close() {
            controls = controls.child(
                Button::new("market-session-close", "Close")
                    .label_size(LabelSize::Small)
                    .on_click(cx.listener(|this, _, _window, cx| this.close_session(cx))),
            );
        }
        section = section.child(controls);

        if let Some(error) = &state.last_error {
            section = section.child(
                Label::new(error.clone())
                    .size(LabelSize::Small)
                    .color(Color::Error),
            );
        }
        if let Some(diagnostic) = state.diagnostics.back() {
            section = section.child(
                Label::new(diagnostic.clone())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
        }
        Some(section.into_any_element())
    }
}

fn session_store_directory() -> PathBuf {
    paths::data_dir().join(SESSION_STORE_DIRECTORY)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

async fn fetch_market_relay_gate(
    http_client: Arc<dyn HttpClient>,
    config: &MarketDiscoveryConfig,
) -> Result<MarketRelayGate, String> {
    let information_url = config.relay_information_url()?;
    let request = Request::builder()
        .uri(information_url.as_str())
        .header("Accept", NIP11_ACCEPT_MEDIA_TYPE)
        .body(AsyncBody::empty())
        .map_err(|error| format!("relay information request failed: {error}"))?;
    let response = http_client
        .send(request)
        .await
        .map_err(|error| format!("relay information fetch failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "relay information fetch returned {}",
            response.status()
        ));
    }
    let mut text = String::new();
    response
        .into_body()
        .take(MAX_RELAY_INFORMATION_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .await
        .map_err(|error| format!("relay information read failed: {error}"))?;
    validate_market_relay_information(&text)
}

async fn run_socket(
    url: String,
    subscription_request: String,
    frames: async_channel::Sender<SocketFrame>,
) -> Result<(), String> {
    let (mut stream, _response) = connect_async(url.as_str())
        .await
        .map_err(|error| format!("relay connection failed: {error}"))?;
    if frames.send(SocketFrame::Opened).await.is_err() {
        return Ok(());
    }
    stream
        .send(Message::Text(subscription_request.into()))
        .await
        .map_err(|error| format!("subscription send failed: {error}"))?;
    while let Some(message) = stream.next().await {
        match message.map_err(|error| format!("relay read failed: {error}"))? {
            Message::Text(text) => {
                if frames
                    .send(SocketFrame::Text(text.to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Message::Ping(payload) => {
                stream
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|error| format!("relay pong failed: {error}"))?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

fn record_status_color(status: &str) -> Color {
    match status {
        "active" => Color::Success,
        "paused" | "exhausted" => Color::Warning,
        _ => Color::Muted,
    }
}

impl Render for MarketPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let providers = self.discovery.providers();
        let offerings = self.discovery.offerings();
        let synced = matches!(self.discovery.connection(), ConnectionState::Live);
        let session_active = self.session.is_some();
        let session_section = self.render_session(cx);

        v_flex()
            .id("market-panel")
            .key_context("MarketPanel")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Reconnect, _window, cx| this.connect(cx)))
            .size_full()
            .p_2()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(Label::new("Markets").size(LabelSize::Large))
                    .child(
                        Label::new("DEV")
                            .size(LabelSize::Small)
                            .color(Color::Warning),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Indicator::dot().color(self.connection_color()))
                    .when_some(self.gate.as_ref(), |this, gate| {
                        this.child(Indicator::dot().color(if gate.advertises_mkt_swp {
                            Color::Success
                        } else {
                            Color::Warning
                        }))
                    })
                    .child(
                        Label::new(self.config.relay_websocket_url.clone())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Button::new("market-reconnect", "Reconnect")
                            .label_size(LabelSize::Small)
                            .on_click(cx.listener(|this, _, _window, cx| this.connect(cx))),
                    ),
            )
            .when_some(self.failure_reason(), |this, reason| {
                this.child(
                    Label::new(reason)
                        .size(LabelSize::Small)
                        .color(Color::Error),
                )
            })
            .child(
                Label::new("Providers")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                v_flex()
                    .gap_1()
                    .when(synced && providers.is_empty(), |this| {
                        this.child(Label::new("—").color(Color::Muted))
                    })
                    .children(providers.into_iter().map(|provider| {
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Indicator::dot().color(record_status_color(&provider.status)))
                            .child(Label::new(
                                provider
                                    .name
                                    .unwrap_or_else(|| provider.provider_id.clone()),
                            ))
                            .child(
                                Label::new(provider.profiles.join(" "))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                    })),
            )
            .child(
                Label::new("Offerings")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                v_flex()
                    .gap_1()
                    .when(synced && offerings.is_empty(), |this| {
                        this.child(Label::new("—").color(Color::Muted))
                    })
                    .children(offerings.into_iter().enumerate().map(|(index, offering)| {
                        let quotable = !session_active
                            && offering.status == "active"
                            && offering.profile.starts_with("mkt-swp:");
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Indicator::dot().color(record_status_color(&offering.status)))
                            .child(Label::new(offering.offering_id.clone()))
                            .child(
                                Label::new(offering.profile.clone())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .when(quotable, |row| {
                                row.child(
                                    Button::new(("market-offering-quote", index), "Request quotes")
                                        .label_size(LabelSize::Small)
                                        .on_click(cx.listener(move |this, _, _window, cx| {
                                            let offering =
                                                this.discovery.offerings().into_iter().nth(index);
                                            if let Some(offering) = offering {
                                                this.start_session(&offering, cx);
                                            }
                                        })),
                                )
                            })
                    })),
            )
            .children(session_section)
    }
}

impl Focusable for MarketPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for MarketPanel {}

impl Panel for MarketPanel {
    fn persistent_name() -> &'static str {
        "MarketPanel"
    }

    fn panel_key() -> &'static str {
        PANEL_KEY
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        DockPosition::Right
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, _: DockPosition, _: &mut Window, _: &mut Context<Self>) {}

    fn default_size(&self, _: &Window, _: &App) -> gpui::Pixels {
        px(360.)
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::ArrowRightLeft)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Markets")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        10
    }
}
