//! The Markets dock panel (omega#244): NIP-MKT discovery over Omega's own
//! WebSocket against an Immortal relay, gated by NIP-11, plus the requester
//! session flow (RFQ → Quote → Order → Status → Cancel → Close) over an
//! authenticated gift-wrap lane.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use async_tungstenite::async_std::connect_async;
use async_tungstenite::tungstenite::Message;
use command_center_ui::{CommandCenterHeader, PortfolioSummary};
use futures::{AsyncReadExt as _, StreamExt as _};
use gpui::{
    App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Subscription, Task, WeakEntity, Window, px,
};
use http_client::{AsyncBody, HttpClient, Request};
use immortal_client::domain::{Event, MktEventIdAdmission, MktEventIdDeduplicator};
use immortal_client::mkt_swp_client::ParticipantRole;
use nautilus_sidecar::CommandOutcome;
use trading_ledger::{LedgerEntry, LedgerStore};
use ui::{
    BookSource, CandleSource, CandlestickChart, Indicator, MarketBadge, MarketBadgeKind,
    MarketEnvironment, OrderBookLadder, OrderConfirmDialog, OrderConfirmationAction, OrderTicket,
    VizChip, VizChipTone, prelude::*,
};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::discovery::{
    ConnectionState, MAX_RELAY_INFORMATION_BYTES, MarketDiscovery, MarketDiscoveryConfig,
    MarketRelayGate, NIP11_ACCEPT_MEDIA_TYPE, OfferingListing, validate_market_relay_information,
};
use crate::network_transport::{
    MultiRelayStatus, ProviderNetworkState, RelayAvailability, RelaySetPlan, fanout_exact_event,
};
use crate::session_flow::{
    IntentProgress, MarketSession, SESSION_STORE_DIRECTORY, SessionPhase, StatusSlot,
    throwaway_session_signer, wrap_for_transport,
};
use crate::session_transport::{SessionInbox, SessionSocketEvent, run_session_socket};
use crate::{
    NautilusBookSource, NautilusCandleSource, NautilusLiveSnapshot,
    NautilusOrderConfirmationSource, NautilusOrderTicketSource, RECEIPT_EXPORT_DIRECTORY,
    ReceiptVerification, Reconnect, ToggleFocus, export_verified_receipt, persist_verified_receipt,
};

const PANEL_KEY: &str = "market";
const MAX_SESSION_DIAGNOSTICS: usize = 8;

pub struct MarketPanel {
    focus_handle: FocusHandle,
    config: MarketDiscoveryConfig,
    gate: Option<MarketRelayGate>,
    discovery: MarketDiscovery,
    session: Option<SessionState>,
    receipt_ledger: Option<LedgerStore>,
    receipt_ledger_error: Option<String>,
    nautilus_stream: Option<Entity<nautilus_sidecar::NautilusStreamSource>>,
    _nautilus_stream_subscription: Option<Subscription>,
    live_order: crate::LiveOrderState,
    _live_order_task: Option<Task<()>>,
    _connection_task: Option<Task<()>>,
}

struct SessionState {
    session: MarketSession,
    outgoing: BTreeMap<String, async_channel::Sender<Event>>,
    relay_status: MultiRelayStatus,
    network_event_ids: MktEventIdDeduplicator,
    network_events: BTreeMap<String, Event>,
    initial_replay_sent: bool,
    _response_outgoing: BTreeMap<String, async_channel::Sender<Event>>,
    last_error: Option<String>,
    diagnostics: VecDeque<String>,
    receipt_entries: BTreeMap<String, Vec<LedgerEntry>>,
    last_export: Option<PathBuf>,
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
        let (receipt_ledger, receipt_ledger_error) = match LedgerStore::open_default() {
            Ok(store) => (Some(store), None),
            Err(error) => (
                None,
                Some(format!("could not open settlement receipt ledger: {error}")),
            ),
        };
        let nautilus_stream = nautilus_sidecar::NautilusStreamSource::try_global(cx);
        let nautilus_stream_subscription = nautilus_stream
            .as_ref()
            .map(|source| cx.observe(source, |_, _, cx| cx.notify()));
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            config: MarketDiscoveryConfig::from_environment(),
            gate: None,
            discovery: MarketDiscovery::new(),
            session: None,
            receipt_ledger,
            receipt_ledger_error,
            nautilus_stream,
            _nautilus_stream_subscription: nautilus_stream_subscription,
            live_order: crate::LiveOrderState::Idle,
            _live_order_task: None,
            _connection_task: None,
        };
        this.connect(cx);
        this
    }

    fn prepare_testnet_probe(&mut self, cx: &mut Context<Self>) {
        let live = self
            .nautilus_stream
            .as_ref()
            .map(|stream| NautilusLiveSnapshot::new(stream.read(cx).market_snapshot()));
        let probe_input = live.as_ref().and_then(|live| {
            live.latest_quote()
                .map(|(bid, _)| bid)
                .zip(live.account_summary().available_margin_cents)
        });
        self.live_order = probe_input
            .and_then(|(bid, available_margin_cents)| {
                crate::NautilusOrderIntent::testnet_probe(
                    bid,
                    available_margin_cents,
                    unique_order_sequence(),
                )
            })
            .map_or_else(
                || crate::LiveOrderState::Failed {
                    command_id: "pre-dispatch".to_owned(),
                    detail: "typed testnet quote or collateral unavailable".to_owned(),
                },
                crate::LiveOrderState::Draft,
            );
        cx.notify();
    }

    fn confirm_testnet_probe(&mut self, cx: &mut Context<Self>) {
        let crate::LiveOrderState::Review(intent) = &self.live_order else {
            return;
        };
        let intent = intent.clone();
        let command_id = intent.command_id.clone();
        let Some(channel) = nautilus_sidecar::command_channel(cx) else {
            self.live_order = crate::LiveOrderState::Failed {
                command_id,
                detail: "Nautilus command channel unavailable".to_owned(),
            };
            cx.notify();
            return;
        };
        let request = intent.place_request();
        self.live_order = crate::LiveOrderState::Sending {
            command_id: command_id.clone(),
        };
        self._live_order_task = Some(cx.spawn(async move |this, cx| {
            let result = channel.send(request).await;
            this.update(cx, |this, cx| {
                this.live_order = match result {
                    Ok(receipt) => crate::LiveOrderState::Completed { intent, receipt },
                    Err(error) => crate::LiveOrderState::Failed {
                        command_id,
                        detail: error.to_string(),
                    },
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn discard_testnet_probe(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.live_order,
            crate::LiveOrderState::Draft(_) | crate::LiveOrderState::Review(_)
        ) {
            self.live_order = crate::LiveOrderState::Idle;
            cx.notify();
        }
    }

    fn review_testnet_probe(&mut self, command_id: &str, cx: &mut Context<Self>) {
        let crate::LiveOrderState::Draft(intent) = &self.live_order else {
            return;
        };
        if intent.command_id == command_id {
            self.live_order = crate::LiveOrderState::Review(intent.clone());
            cx.notify();
        }
    }

    fn cancel_testnet_probe(&mut self, cx: &mut Context<Self>) {
        let Some(intent) = self.live_order.accepted_intent().cloned() else {
            return;
        };
        let request = intent.cancel_request(unique_order_sequence());
        let command_id = request.command_id.clone();
        let Some(channel) = nautilus_sidecar::command_channel(cx) else {
            self.live_order = crate::LiveOrderState::Failed {
                command_id,
                detail: "Nautilus command channel unavailable".to_owned(),
            };
            cx.notify();
            return;
        };
        self.live_order = crate::LiveOrderState::Sending {
            command_id: command_id.clone(),
        };
        self._live_order_task = Some(cx.spawn(async move |this, cx| {
            let result = channel.send(request).await;
            this.update(cx, |this, cx| {
                this.live_order = match result {
                    Ok(receipt) => crate::LiveOrderState::Completed { intent, receipt },
                    Err(error) => crate::LiveOrderState::Failed {
                        command_id,
                        detail: error.to_string(),
                    },
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn render_nautilus(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let stream = self.nautilus_stream.as_ref()?;
        let snapshot = stream.read(cx).market_snapshot();
        let live = NautilusLiveSnapshot::new(snapshot.clone());
        let account = live.account_summary();
        let quote = live.latest_quote();
        let candle_source = NautilusCandleSource::new(snapshot.clone());
        let book_source = NautilusBookSource::new(snapshot);
        let candles = candle_source.series();
        let book = book_source.snapshot();
        let has_market_data =
            !candles.candles().is_empty() || !book.bids.is_empty() || !book.asks.is_empty();
        let order_controls = match &self.live_order {
            crate::LiveOrderState::Idle => h_flex()
                .child(
                    Button::new("nautilus-order-prepare", "Prepare testnet probe")
                        .label_size(LabelSize::Small)
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.prepare_testnet_probe(cx);
                        })),
                )
                .into_any_element(),
            crate::LiveOrderState::Draft(intent) => {
                let panel = cx.weak_entity();
                let expected_command_id = intent.command_id.clone();
                OrderTicket::from_source(&NautilusOrderTicketSource::new(intent))
                    .on_review(move |submitted, _window, cx| {
                        if submitted.draft.venue.as_ref() != "Hyperliquid" {
                            return;
                        }
                        panel
                            .update(cx, |panel, cx| {
                                panel.review_testnet_probe(&expected_command_id, cx);
                            })
                            .ok();
                    })
                    .into_any_element()
            }
            crate::LiveOrderState::Review(intent) => {
                let panel = cx.weak_entity();
                OrderConfirmDialog::from_source(&NautilusOrderConfirmationSource::new(intent))
                    .on_action(move |action, _window, cx| {
                        panel
                            .update(cx, |panel, cx| match action {
                                OrderConfirmationAction::Approve { .. } => {
                                    panel.confirm_testnet_probe(cx);
                                }
                                OrderConfirmationAction::Reject { .. } => {
                                    panel.discard_testnet_probe(cx);
                                }
                            })
                            .ok();
                    })
                    .into_any_element()
            }
            crate::LiveOrderState::Sending { command_id } => h_flex()
                .gap_1()
                .items_center()
                .child(Indicator::dot().color(Color::Warning))
                .child(
                    Label::new(format!("sending {command_id} once"))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .into_any_element(),
            crate::LiveOrderState::Completed { receipt, .. } => h_flex()
                .gap_1()
                .items_center()
                .flex_wrap()
                .child(
                    VizChip::new(command_outcome_label(&receipt.outcome))
                        .tone(
                            if matches!(
                                receipt.outcome,
                                CommandOutcome::OrderAccepted { .. }
                                    | CommandOutcome::OrderCanceled { .. }
                            ) {
                                VizChipTone::Ok
                            } else {
                                VizChipTone::Warn
                            },
                        )
                        .scale(1.0),
                )
                .child(
                    Label::new(format!(
                        "ack {} · sent {}",
                        receipt.acknowledged, receipt.sent,
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                )
                .when(self.live_order.accepted_intent().is_some(), |row| {
                    row.child(
                        Button::new("nautilus-order-cancel", "Cancel once")
                            .label_size(LabelSize::Small)
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.cancel_testnet_probe(cx);
                            })),
                    )
                })
                .into_any_element(),
            crate::LiveOrderState::Failed { command_id, detail } => v_flex()
                .gap_1()
                .child(
                    Label::new(format!("{command_id} · {detail}"))
                        .size(LabelSize::XSmall)
                        .color(Color::Error),
                )
                .child(
                    Button::new("nautilus-order-new", "New intent")
                        .label_size(LabelSize::Small)
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.prepare_testnet_probe(cx);
                        })),
                )
                .into_any_element(),
        };
        let ledger_header = match self.receipt_ledger.as_ref().map(|ledger| {
            PortfolioSummary::from_ledger(
                ledger,
                &["hyperliquid"],
                0,
                i64::try_from(unique_order_sequence()).unwrap_or(i64::MAX),
            )
        }) {
            Some(Ok(summary)) => CommandCenterHeader::new(summary).into_any_element(),
            Some(Err(error)) => Label::new(format!("ledger unavailable · {error}"))
                .size(LabelSize::XSmall)
                .color(Color::Error)
                .into_any_element(),
            None => Label::new("ledger unavailable")
                .size(LabelSize::XSmall)
                .color(Color::Error)
                .into_any_element(),
        };

        Some(
            v_flex()
                .id("market-nautilus-live")
                .debug_selector(|| "market.nautilus.live".into())
                .gap_2()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(Label::new("BTC-PERP").size(LabelSize::Small))
                                .child(MarketBadge::new(MarketBadgeKind::Environment(
                                    MarketEnvironment::Testnet,
                                )))
                                .child(MarketBadge::new(if has_market_data {
                                    MarketBadgeKind::VenueConnected
                                } else {
                                    MarketBadgeKind::VenueDegraded
                                })),
                        )
                        .when_some(quote, |row, (bid, ask)| {
                            row.child(
                                Label::new(format!("{bid:.2} / {ask:.2}"))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .child(
                            VizChip::new(if account.account_ready {
                                "account live"
                            } else {
                                "account waiting"
                            })
                            .tone(if account.account_ready {
                                VizChipTone::Ok
                            } else {
                                VizChipTone::Warn
                            })
                            .scale(1.0),
                        )
                        .when_some(account.account_id.clone(), |row, account_id| {
                            row.child(VizChip::new(account_id).scale(1.0))
                        })
                        .child(
                            VizChip::new(format!("{} balances", account.balance_count)).scale(1.0),
                        )
                        .when_some(account.available_margin_cents, |row, cents| {
                            row.child(
                                VizChip::new(format!(
                                    "${:.2} {} free",
                                    cents as f64 / 100.0,
                                    account.collateral_currency.as_deref().unwrap_or("USD"),
                                ))
                                .tone(VizChipTone::Ok)
                                .scale(1.0),
                            )
                        })
                        .child(VizChip::new(format!("{} orders", account.order_count)).scale(1.0))
                        .child(
                            VizChip::new(format!("{} positions", account.position_count))
                                .scale(1.0),
                        )
                        .child(VizChip::new(format!("{} fills", account.fill_count)).scale(1.0)),
                )
                .child(ledger_header)
                .child(order_controls)
                .when(!has_market_data, |section| {
                    section.child(
                        Label::new("Awaiting typed testnet stream")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                })
                .when(!candles.candles().is_empty(), |section| {
                    section.child(
                        CandlestickChart::new(candles)
                            .size(560.0, 260.0)
                            .volume(true),
                    )
                })
                .when(!book.bids.is_empty() || !book.asks.is_empty(), |section| {
                    section.child(OrderBookLadder::new(book).width(560.0).depth(10))
                })
                .into_any_element(),
        )
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
                            if let Some(state) = this.session.as_mut() {
                                let network_events = this
                                    .discovery
                                    .provider_network_events(state.session.provider_id());
                                let mut merge_error = None;
                                for event in network_events {
                                    match state.network_event_ids.observe(&event) {
                                        Ok(MktEventIdAdmission::New) => {
                                            state.network_events.insert(event.id.clone(), event);
                                        }
                                        Ok(MktEventIdAdmission::Duplicate) => {}
                                        Err(error) => {
                                            merge_error = Some(error.to_string());
                                            break;
                                        }
                                    }
                                }
                                if let Some(error) = merge_error {
                                    Self::push_session_diagnostic(state, error);
                                } else {
                                    let merged =
                                        state.network_events.values().cloned().collect::<Vec<_>>();
                                    if let Err(error) =
                                        state.session.refresh_provider_network(&merged)
                                    {
                                        Self::push_session_diagnostic(state, error);
                                    }
                                }
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
        let network_events = self.discovery.network_events();
        let provider_network =
            ProviderNetworkState::for_active_signer(&offering.pubkey, now, &network_events);
        let provider_network = match provider_network {
            Ok(network) => network,
            Err(error) => {
                self.discovery_diagnostic(error);
                cx.notify();
                return;
            }
        };
        let relay_plan = provider_network.relay_plan_at(now).unwrap_or_else(|| {
            RelaySetPlan::legacy_bootstrap(self.config.relay_websocket_url.clone())
        });
        let session = throwaway_session_signer().and_then(|signer| {
            MarketSession::begin_with_network(signer, offering, provider_network, now)
        });
        let session = match session {
            Ok(session) => session,
            Err(error) => {
                self.discovery_diagnostic(error);
                cx.notify();
                return;
            }
        };
        let signer = session.signer().clone();
        let response_signer = session.response_signer().clone();
        let provider_id = session.provider_id().to_owned();
        let mut network_event_ids = MktEventIdDeduplicator::default();
        let mut network_events = BTreeMap::new();
        let mut relay_status = match MultiRelayStatus::new(relay_plan.clone()) {
            Ok(status) => status,
            Err(error) => {
                self.discovery_diagnostic(error);
                cx.notify();
                return;
            }
        };
        for event in session.provider_network_events() {
            if let Err(error) = network_event_ids.observe(event) {
                self.discovery_diagnostic(error.to_string());
                cx.notify();
                return;
            }
            if let Err(error) = relay_status.seed_event(event) {
                self.discovery_diagnostic(error);
                cx.notify();
                return;
            }
            network_events.insert(event.id.clone(), event.clone());
        }
        let mut outgoing = BTreeMap::new();
        let mut response_outgoing = BTreeMap::new();
        let mut socket_inputs = Vec::with_capacity(relay_plan.relays.len());
        for relay_url in &relay_plan.relays {
            let (outgoing_sender, outgoing_receiver) = async_channel::bounded(256);
            let (response_sender, response_receiver) = async_channel::bounded(1);
            outgoing.insert(relay_url.clone(), outgoing_sender);
            response_outgoing.insert(relay_url.clone(), response_sender);
            socket_inputs.push((relay_url.clone(), outgoing_receiver, response_receiver));
        }
        let (event_sender, event_receiver) = async_channel::bounded(256);
        let task = cx.spawn(async move |this, cx| {
            for (relay_url, outgoing_receiver, response_receiver) in socket_inputs {
                let requester_events = event_sender.clone();
                let requester_url = relay_url.clone();
                let requester_signer = signer.clone();
                let requester_provider_id = provider_id.clone();
                cx.background_spawn(async move {
                    let result = run_session_socket(
                        requester_url.clone(),
                        requester_signer,
                        requester_provider_id,
                        SessionInbox::Requester,
                        outgoing_receiver,
                        requester_events.clone(),
                        unix_now,
                    )
                    .await;
                    let reason = match result {
                        Ok(()) => "requester inbox closed".to_owned(),
                        Err(error) => error,
                    };
                    if requester_events
                        .send(SessionSocketEvent::Disconnected {
                            relay_url: requester_url,
                            inbox: SessionInbox::Requester,
                            reason,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                })
                .detach();
                let response_events = event_sender.clone();
                let response_url = relay_url;
                let relay_response_signer = response_signer.clone();
                let response_provider_id = provider_id.clone();
                cx.background_spawn(async move {
                    let result = run_session_socket(
                        response_url.clone(),
                        relay_response_signer,
                        response_provider_id,
                        SessionInbox::Response,
                        response_receiver,
                        response_events.clone(),
                        unix_now,
                    )
                    .await;
                    let reason = match result {
                        Ok(()) => "response inbox closed".to_owned(),
                        Err(error) => error,
                    };
                    if response_events
                        .send(SessionSocketEvent::Disconnected {
                            relay_url: response_url,
                            inbox: SessionInbox::Response,
                            reason,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                })
                .detach();
            }
            while let Ok(event) = event_receiver.recv().await {
                let updated = this.update(cx, |this, cx| {
                    this.handle_session_event(event, cx);
                });
                if updated.is_err() {
                    return;
                }
            }
        });
        self.session = Some(SessionState {
            session,
            outgoing,
            relay_status,
            network_event_ids,
            network_events,
            initial_replay_sent: false,
            _response_outgoing: response_outgoing,
            last_error: None,
            diagnostics: VecDeque::new(),
            receipt_entries: BTreeMap::new(),
            last_export: None,
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
            SessionSocketEvent::Authenticated { .. } => {}
            SessionSocketEvent::SubscriptionLive { relay_url, inbox } => {
                if let Err(error) = state.relay_status.subscription_live(&relay_url, inbox) {
                    Self::push_session_diagnostic(state, error);
                    cx.notify();
                    return;
                }
                // Replay every own signed record as fresh wraps; identical
                // signed wraps fan out to all relays as one exact-byte batch.
                if inbox == SessionInbox::Requester
                    && !state.initial_replay_sent
                    && state.relay_status.read_availability(inbox) != RelayAvailability::Unavailable
                {
                    match state.session.replay_wraps(unix_now()) {
                        Ok(wraps) => {
                            for wrap in wraps {
                                if let Err(failed) = fanout_exact_event(&wrap, &state.outgoing) {
                                    Self::push_session_diagnostic(
                                        state,
                                        format!(
                                            "session replay queue failed for relays: {}",
                                            failed.join(", ")
                                        ),
                                    );
                                    break;
                                }
                            }
                            state.initial_replay_sent = true;
                        }
                        Err(error) => state.last_error = Some(error),
                    }
                }
            }
            SessionSocketEvent::ProviderNetwork { relay_url, event } => {
                match state.relay_status.observe_event(&relay_url, &event) {
                    Ok(MktEventIdAdmission::Duplicate) => {}
                    Ok(MktEventIdAdmission::New) => match state.network_event_ids.observe(&event) {
                        Ok(MktEventIdAdmission::Duplicate) => {}
                        Ok(MktEventIdAdmission::New) => {
                            state.network_events.insert(event.id.clone(), event);
                            let events = state.network_events.values().cloned().collect::<Vec<_>>();
                            if let Err(error) = state.session.refresh_provider_network(&events) {
                                Self::push_session_diagnostic(
                                    state,
                                    format!("relay {relay_url} provider network: {error}"),
                                );
                            }
                        }
                        Err(error) => Self::push_session_diagnostic(
                            state,
                            format!("relay {relay_url} provider network: {error}"),
                        ),
                    },
                    Err(error) => Self::push_session_diagnostic(
                        state,
                        format!("relay {relay_url} provider network: {error}"),
                    ),
                }
            }
            SessionSocketEvent::Delivered {
                relay_url,
                inbox: _,
                delivered,
                observed_at,
            } => match state
                .relay_status
                .observe_event(&relay_url, delivered.record().event())
            {
                Ok(MktEventIdAdmission::Duplicate) => {}
                Ok(MktEventIdAdmission::New) => {
                    match state.session.admit_delivery(&delivered, observed_at) {
                        Ok(_) => {
                            self.persist_session();
                            self.sync_verified_receipts();
                        }
                        Err(error) => Self::push_session_diagnostic(state, error),
                    }
                }
                Err(error) => Self::push_session_diagnostic(
                    state,
                    format!("relay {relay_url} event merge: {error}"),
                ),
            },
            SessionSocketEvent::PublishResult {
                relay_url,
                event_id,
                accepted,
                message,
            } => {
                if let Err(error) = state
                    .relay_status
                    .publish_result(&relay_url, &event_id, accepted)
                {
                    Self::push_session_diagnostic(state, error);
                }
                if !accepted {
                    Self::push_session_diagnostic(
                        state,
                        format!("relay {relay_url} refused {event_id}: {message}"),
                    );
                }
            }
            SessionSocketEvent::Diagnostic { relay_url, message } => {
                Self::push_session_diagnostic(state, format!("relay {relay_url}: {message}"));
            }
            SessionSocketEvent::Disconnected {
                relay_url,
                inbox,
                reason,
            } => {
                if let Err(error) = state.relay_status.disconnected(&relay_url, inbox) {
                    Self::push_session_diagnostic(state, error);
                }
                let availability = state.relay_status.read_availability(inbox);
                if availability == RelayAvailability::Unavailable {
                    state.last_error = Some(format!("relay {relay_url}: {reason}"));
                } else {
                    Self::push_session_diagnostic(
                        state,
                        format!("degraded relay {relay_url}: {reason}"),
                    );
                }
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
            let provider_recipient = state.session.provider_transport_pubkey(now).to_owned();
            match wrap_for_transport(&record, state.session.signer(), &provider_recipient, now) {
                Ok(wraps) => {
                    for wrap in wraps {
                        if let Err(failed) = fanout_exact_event(&wrap.event, &state.outgoing) {
                            let successes = state
                                .relay_status
                                .plan()
                                .relays
                                .len()
                                .saturating_sub(failed.len());
                            if successes < state.relay_status.plan().publish_minimum {
                                state.last_error = Some(format!(
                                    "session publish queue missed signed relay threshold: {}",
                                    failed.join(", ")
                                ));
                                return;
                            }
                            Self::push_session_diagnostic(
                                state,
                                format!("degraded relay publish queues: {}", failed.join(", ")),
                            );
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
        self.sync_verified_receipts();
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

    fn replay_stuck_intent(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.session.as_mut() {
            match state.session.replay_stuck_intent(unix_now()) {
                Ok(record) => self.publish_session_records(vec![record]),
                Err(error) => state.last_error = Some(error),
            }
        }
        cx.notify();
    }

    fn request_redrive(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.session.as_mut() {
            match state.session.request_redrive(unix_now()) {
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

    fn sync_verified_receipts(&mut self) {
        let Some(store) = self.receipt_ledger.as_ref() else {
            return;
        };
        let Some(state) = self.session.as_mut() else {
            return;
        };
        for verification in state.session.receipt_verifications() {
            let Some(receipt_id) = verification.receipt_id().map(str::to_owned) else {
                continue;
            };
            match persist_verified_receipt(
                store,
                state.session.requester_pubkey(),
                state.session.provider_id(),
                &verification,
            ) {
                Ok(entries) => {
                    state.receipt_entries.insert(receipt_id, entries);
                }
                Err(error) => Self::push_session_diagnostic(state, error),
            }
        }
    }

    fn export_receipt(&mut self, receipt_id: &str, cx: &mut Context<Self>) {
        let Some(state) = self.session.as_mut() else {
            return;
        };
        let verifications = state.session.receipt_verifications();
        let Some(verification) = verifications
            .iter()
            .find(|verification| verification.receipt_id() == Some(receipt_id))
        else {
            state.last_error = Some("verified receipt is no longer available".to_owned());
            cx.notify();
            return;
        };
        let Some(entries) = state.receipt_entries.get(receipt_id) else {
            state.last_error = Some("verified receipt has not entered the ledger".to_owned());
            cx.notify();
            return;
        };
        let directory = paths::data_dir().join(RECEIPT_EXPORT_DIRECTORY);
        match export_verified_receipt(
            &directory,
            state.session.records(),
            state.session.provider_network_events(),
            verification,
            entries,
        ) {
            Ok(path) => {
                state.last_export = Some(path);
                state.last_error = None;
            }
            Err(error) => state.last_error = Some(error),
        }
        cx.notify();
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
        let intent_progress = session.intent_progress(now);
        let receipt_verifications = session.receipt_verifications();
        let selected = session.selected_quote(now);
        let requester_availability = state
            .relay_status
            .read_availability(SessionInbox::Requester);
        let response_availability = state.relay_status.read_availability(SessionInbox::Response);
        let short_id: String = session.session_id().chars().take(8).collect();
        let phase_tone = match phase {
            SessionPhase::AwaitingQuotes | SessionPhase::QuoteReceived => VizChipTone::Neutral,
            SessionPhase::OrderInFlight | SessionPhase::Active => VizChipTone::Active,
            SessionPhase::CancelRequested => VizChipTone::Warn,
            SessionPhase::Closed => VizChipTone::Ok,
        };
        let (proof_label, proof_tone) = match &intent_progress {
            IntentProgress::NotOrdered => ("quote only".to_owned(), VizChipTone::Neutral),
            IntentProgress::AwaitingAcknowledgment {
                timed_out: false, ..
            } => ("awaiting ack".to_owned(), VizChipTone::Active),
            IntentProgress::AwaitingAcknowledgment {
                timed_out: true, ..
            } => ("ack overdue".to_owned(), VizChipTone::Warn),
            IntentProgress::Rejected { error_code, .. } => {
                (format!("rejected · {error_code}"), VizChipTone::Warn)
            }
            IntentProgress::AwaitingOutcome {
                timed_out: false, ..
            } => ("ack verified".to_owned(), VizChipTone::Ok),
            IntentProgress::AwaitingOutcome {
                timed_out: true, ..
            } => ("outcome overdue".to_owned(), VizChipTone::Warn),
            IntentProgress::OutcomeReceived { .. } => match receipt_verifications.last() {
                Some(ReceiptVerification::ProviderSigned { .. }) => {
                    ("receipt provider-signed".to_owned(), VizChipTone::Ok)
                }
                Some(ReceiptVerification::Incomplete { .. }) => {
                    ("receipt incomplete".to_owned(), VizChipTone::Warn)
                }
                Some(ReceiptVerification::Invalid { .. }) => {
                    ("receipt invalid".to_owned(), VizChipTone::Warn)
                }
                None => ("awaiting receipt".to_owned(), VizChipTone::Warn),
            },
        };

        let mut section = v_flex().gap_1p5().child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .flex_wrap()
                        .child(
                            Label::new("Session")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(Indicator::dot().color(match requester_availability {
                            RelayAvailability::Available => Color::Success,
                            RelayAvailability::Degraded | RelayAvailability::Unavailable => {
                                Color::Warning
                            }
                        }))
                        .child(
                            Label::new(format!("{short_id} · {}", session.offering_label()))
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(VizChip::new(phase.label()).tone(phase_tone).scale(1.0))
                        .child(VizChip::new(proof_label).tone(proof_tone).scale(1.0))
                        .child(
                            VizChip::new(match response_availability {
                                RelayAvailability::Available => "response relays live",
                                RelayAvailability::Degraded => "response relays degraded",
                                RelayAvailability::Unavailable => "response relays waiting",
                            })
                            .tone(match response_availability {
                                RelayAvailability::Available => VizChipTone::Ok,
                                RelayAvailability::Degraded | RelayAvailability::Unavailable => {
                                    VizChipTone::Warn
                                }
                            })
                            .scale(1.0),
                        )
                        .child(
                            VizChip::new(format!(
                                "{} {} relay{} · p{}/r{}",
                                if state.relay_status.plan().is_signed() {
                                    "signed"
                                } else {
                                    "bootstrap"
                                },
                                state.relay_status.plan().relays.len(),
                                if state.relay_status.plan().relays.len() == 1 {
                                    ""
                                } else {
                                    "s"
                                },
                                state.relay_status.plan().publish_minimum,
                                state.relay_status.plan().read_minimum,
                            ))
                            .tone(if state.relay_status.plan().is_signed() {
                                VizChipTone::Ok
                            } else {
                                VizChipTone::Neutral
                            })
                            .scale(1.0),
                        ),
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

        if !receipt_verifications.is_empty() {
            section = section.child(
                Label::new("Receipts")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
            for (receipt_index, verification) in receipt_verifications.iter().enumerate() {
                let short_event_id: String =
                    verification.receipt_event_id().chars().take(8).collect();
                match verification {
                    ReceiptVerification::Incomplete { detail, .. } => {
                        section = section.child(
                            v_flex()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .child(
                                            VizChip::new("incomplete")
                                                .kind(39_613)
                                                .tone(VizChipTone::Warn)
                                                .scale(1.0),
                                        )
                                        .child(
                                            Label::new(short_event_id)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        ),
                                )
                                .child(
                                    Label::new(detail.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        );
                    }
                    ReceiptVerification::Invalid { detail, .. } => {
                        section = section.child(
                            v_flex()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .child(
                                            VizChip::new("invalid")
                                                .kind(39_613)
                                                .tone(VizChipTone::Warn)
                                                .scale(1.0),
                                        )
                                        .child(
                                            Label::new(short_event_id)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        ),
                                )
                                .child(
                                    Label::new(detail.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Error),
                                ),
                        );
                    }
                    ReceiptVerification::ProviderSigned { receipt, .. } => {
                        let receipt_id = receipt.receipt_id.clone();
                        let ledger_count =
                            state.receipt_entries.get(&receipt_id).map_or(0, Vec::len);
                        section = section.child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .flex_wrap()
                                .child(
                                    VizChip::new("provider-signed")
                                        .kind(39_613)
                                        .tone(VizChipTone::Ok)
                                        .scale(1.0),
                                )
                                .child(Label::new(format!(
                                    "{} · {} legs · {} fees · {} ledger rows",
                                    receipt.outcome,
                                    receipt.legs.len(),
                                    receipt.fees.len(),
                                    ledger_count,
                                )))
                                .child(
                                    Label::new("external settlement not independently proven")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .when(ledger_count > 0, |row| {
                                    row.child(
                                        Button::new(
                                            ("market-receipt-export", receipt_index),
                                            "Export",
                                        )
                                        .label_size(LabelSize::Small)
                                        .on_click(
                                            cx.listener(move |this, _, _window, cx| {
                                                this.export_receipt(&receipt_id, cx);
                                            }),
                                        ),
                                    )
                                }),
                        );
                    }
                }
            }
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
        if session.can_replay_stuck_intent(now) {
            controls = controls.child(
                Button::new("market-session-replay-intent", "Replay exact intent")
                    .label_size(LabelSize::Small)
                    .on_click(cx.listener(|this, _, _window, cx| this.replay_stuck_intent(cx))),
            );
        }
        if session.can_redrive(now) {
            controls = controls.child(
                Button::new("market-session-redrive", "Re-drive")
                    .label_size(LabelSize::Small)
                    .on_click(cx.listener(|this, _, _window, cx| this.request_redrive(cx))),
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
        if let Some(path) = &state.last_export {
            section = section.child(
                Label::new(format!("Receipt export: {}", path.display()))
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

fn unique_order_sequence() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn command_outcome_label(outcome: &CommandOutcome) -> &'static str {
    match outcome {
        CommandOutcome::OrderAccepted { .. } => "order accepted",
        CommandOutcome::OrderCanceled { .. } => "order canceled",
        CommandOutcome::StrategyStarted { .. } => "strategy started",
        CommandOutcome::StrategyStopped { .. } => "strategy stopped",
        CommandOutcome::StrategyParametersApplied { .. } => "parameters applied",
        CommandOutcome::Refused { .. } => "refused",
        CommandOutcome::Unknown { .. } => "unknown — do not retry",
    }
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
        let nautilus_section = self.render_nautilus(cx);

        v_flex()
            .id("market-panel")
            .key_context("MarketPanel")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Reconnect, _window, cx| this.connect(cx)))
            .size_full()
            .overflow_y_scroll()
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
            .when_some(self.receipt_ledger_error.clone(), |this, reason| {
                this.child(
                    Label::new(reason)
                        .size(LabelSize::Small)
                        .color(Color::Error),
                )
            })
            .children(nautilus_section)
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
        px(600.)
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
