//! The Markets dock panel (omega#244): NIP-MKT discovery over Omega's own
//! WebSocket against an Immortal relay, gated by NIP-11.

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
use ui::{Indicator, prelude::*};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::discovery::{
    ConnectionState, MAX_RELAY_INFORMATION_BYTES, MarketDiscovery, MarketDiscoveryConfig,
    MarketRelayGate, NIP11_ACCEPT_MEDIA_TYPE, validate_market_relay_information,
};
use crate::session_flow::{SessionFlowAvailability, session_flow_availability};
use crate::{Reconnect, ToggleFocus};

const PANEL_KEY: &str = "market";

pub struct MarketPanel {
    focus_handle: FocusHandle,
    config: MarketDiscoveryConfig,
    gate: Option<MarketRelayGate>,
    discovery: MarketDiscovery,
    _connection_task: Option<Task<()>>,
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
        let session_controls = match session_flow_availability() {
            SessionFlowAvailability::NotImplemented { .. } => None::<gpui::AnyElement>,
        };

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
                    .children(offerings.into_iter().map(|offering| {
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
                    })),
            )
            .children(session_controls)
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
