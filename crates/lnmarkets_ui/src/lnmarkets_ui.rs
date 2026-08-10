use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use credentials_provider::CredentialsProvider;
use editor::Editor;
use gpui::{
    App, AppContext as _, Entity, Focusable as _, PromptLevel, ScrollHandle, Task, prelude::*,
};
use lnmarkets_client::{
    Account, CREDENTIAL_STORAGE_URL, Credentials, HttpTransport, LnMarketsClient, Network,
    StoredCredentials,
};
use trading_mandate::{
    AssetId, LEGACY_VENUE, MandateChangeClass, MandateSnapshot, MandateStore, ReviewCadence,
    TradingMandate, TradingNetwork,
};
use ui::{Divider, prelude::*};
use util::ResultExt as _;

mod card_renderers;
mod operator_panel;

pub use card_renderers::card_renderer_registrations;
pub use operator_panel::{
    LnMarketsOperatorPanel, OperatorBacktestSnapshot, OperatorConsoleSnapshot,
    OperatorConsoleSource, OperatorReviewTurn, OperatorStrategySnapshot,
    init as init_operator_panel,
};

pub struct LnMarketsSettingsPage {
    access_key: Entity<Editor>,
    secret: Entity<Editor>,
    passphrase: Entity<Editor>,
    network: Network,
    status: ConnectionStatus,
    http_transport: Arc<dyn HttpTransport>,
    credentials_provider: Arc<dyn CredentialsProvider>,
    operation: Option<Task<()>>,
    mandate_store: Option<MandateStore>,
    mandate_network: TradingNetwork,
    mandate_objective: Entity<Editor>,
    mandate_max_venue_balance: Entity<Editor>,
    mandate_max_position: Entity<Editor>,
    mandate_max_leverage: Entity<Editor>,
    mandate_daily_loss_stop: Entity<Editor>,
    mandate_max_orders_per_hour: Entity<Editor>,
    mandate_min_liquidation_buffer: Entity<Editor>,
    mandate_allowed_strategies: Entity<Editor>,
    mandate_review_cadence: ReviewCadence,
    mandate_review_interval: Entity<Editor>,
    mandate_expires_at: Entity<Editor>,
    mandate_status: MandateStatus,
    mandate_active_revision: Option<u64>,
    mandate_operation: Option<Task<()>>,
    scroll_handle: ScrollHandle,
}

enum ConnectionStatus {
    Loading,
    Empty,
    Saved,
    Testing,
    Connected(Account),
    Error(SharedString),
}

enum MandateStatus {
    Empty,
    Saved { revision: u64 },
    Saving,
    Error(SharedString),
}

impl LnMarketsSettingsPage {
    pub fn new(
        http_transport: Arc<dyn HttpTransport>,
        credentials_provider: Arc<dyn CredentialsProvider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (mandate_store, mandate_status) = match MandateStore::open_default() {
            Ok(store) => (Some(store), MandateStatus::Empty),
            Err(error) => (
                None,
                MandateStatus::Error(
                    format!("Could not open the trading mandate store: {error}").into(),
                ),
            ),
        };
        Self {
            access_key: new_masked_input("API key", window, cx),
            secret: new_masked_input("Secret", window, cx),
            passphrase: new_masked_input("Passphrase", window, cx),
            network: Network::Signet,
            status: ConnectionStatus::Loading,
            http_transport,
            credentials_provider,
            operation: None,
            mandate_store,
            mandate_network: TradingNetwork::Signet,
            mandate_objective: new_text_input(
                "Maximize ledger profit in sats",
                "Trading objective",
                window,
                cx,
            ),
            mandate_max_venue_balance: new_text_input(
                "100000",
                "Maximum venue balance in sats",
                window,
                cx,
            ),
            mandate_max_position: new_text_input("500", "Maximum USD notional", window, cx),
            mandate_max_leverage: new_text_input("3", "Maximum leverage", window, cx),
            mandate_daily_loss_stop: new_text_input("5000", "Daily loss stop in sats", window, cx),
            mandate_max_orders_per_hour: new_text_input(
                "12",
                "Maximum orders per hour",
                window,
                cx,
            ),
            mandate_min_liquidation_buffer: new_text_input(
                "1500",
                "Minimum liquidation buffer in basis points",
                window,
                cx,
            ),
            mandate_allowed_strategies: new_text_input(
                "rebalance_to_target,funding_carry,threshold_swing",
                "Comma-separated strategy IDs",
                window,
                cx,
            ),
            mandate_review_cadence: ReviewCadence::FundingSettlement,
            mandate_review_interval: new_text_input(
                "3600",
                "Review interval in seconds",
                window,
                cx,
            ),
            mandate_expires_at: new_text_input(
                &default_expiry_text(),
                "Expiry as Unix milliseconds",
                window,
                cx,
            ),
            mandate_status,
            mandate_active_revision: None,
            mandate_operation: None,
            scroll_handle: ScrollHandle::new(),
        }
    }

    pub fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_mandate(window, cx);
        let credentials_provider = self.credentials_provider.clone();
        self.status = ConnectionStatus::Loading;
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = credentials_provider
                .read_credentials(CREDENTIAL_STORAGE_URL, &*cx)
                .await;
            this.update_in(cx, |this, window, cx| {
                this.operation = None;
                match result {
                    Ok(Some((_username, encoded))) => match StoredCredentials::decode(&encoded) {
                        Ok(stored) => match stored.credentials() {
                            Ok(credentials) => {
                                this.network = stored.network;
                                this.set_credentials(&credentials, window, cx);
                                this.status = ConnectionStatus::Saved;
                            }
                            Err(error) => {
                                this.status = ConnectionStatus::Error(error.to_string().into());
                            }
                        },
                        Err(error) => {
                            this.status = ConnectionStatus::Error(error.to_string().into());
                        }
                    },
                    Ok(None) => this.status = ConnectionStatus::Empty,
                    Err(error) => {
                        this.status = ConnectionStatus::Error(
                            format!("Could not read saved credentials: {error}").into(),
                        );
                    }
                }
                cx.notify();
            })
            .log_err();
        });
        self.operation = Some(task);
        cx.notify();
    }

    fn load_mandate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.mandate_store.clone() else {
            return;
        };
        match store.snapshot() {
            Ok(snapshot) => self.apply_mandate_snapshot(snapshot, window, cx),
            Err(error) => {
                self.mandate_status = MandateStatus::Error(
                    format!("Could not read the trading mandate: {error}").into(),
                );
            }
        }
    }

    fn apply_mandate_snapshot(
        &mut self,
        snapshot: MandateSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let revision = snapshot.revision;
        // This page edits the LN Markets mandate; other venues' mandates are
        // out of its scope.
        let Some(mandate) = snapshot
            .mandates
            .into_iter()
            .find(|mandate| mandate.venue == LEGACY_VENUE)
        else {
            self.mandate_status = MandateStatus::Empty;
            self.mandate_active_revision = None;
            cx.notify();
            return;
        };
        self.mandate_network = mandate.network;
        self.mandate_review_cadence = mandate.review_cadence.clone();
        set_editor_text(&self.mandate_objective, &mandate.objective, window, cx);
        set_editor_text(
            &self.mandate_max_venue_balance,
            &mandate.max_venue_balance.to_string(),
            window,
            cx,
        );
        set_editor_text(
            &self.mandate_max_position,
            &mandate.max_position_usd.to_string(),
            window,
            cx,
        );
        set_editor_text(
            &self.mandate_max_leverage,
            &mandate.max_leverage.to_string(),
            window,
            cx,
        );
        set_editor_text(
            &self.mandate_daily_loss_stop,
            &mandate.daily_loss_stop.to_string(),
            window,
            cx,
        );
        set_editor_text(
            &self.mandate_allowed_strategies,
            &mandate
                .allowed_strategies
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            window,
            cx,
        );
        if let ReviewCadence::Interval { seconds } = mandate.review_cadence {
            set_editor_text(
                &self.mandate_review_interval,
                &seconds.to_string(),
                window,
                cx,
            );
        }
        set_editor_text(
            &self.mandate_expires_at,
            &mandate.expires_at_ms.to_string(),
            window,
            cx,
        );
        self.mandate_status = MandateStatus::Saved { revision };
        self.mandate_active_revision = Some(revision);
        cx.notify();
    }

    fn mandate_candidate(&self, cx: &App) -> anyhow::Result<TradingMandate> {
        let objective = self.mandate_objective.read(cx).text(cx).trim().to_owned();
        let allowed_strategies = self
            .mandate_allowed_strategies
            .read(cx)
            .text(cx)
            .split(',')
            .map(str::trim)
            .filter(|strategy| !strategy.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let review_cadence = match &self.mandate_review_cadence {
            ReviewCadence::FundingSettlement => ReviewCadence::FundingSettlement,
            ReviewCadence::Interval { .. } => ReviewCadence::Interval {
                seconds: parse_editor(
                    &self.mandate_review_interval,
                    "review interval seconds",
                    cx,
                )?,
            },
        };
        let mandate = TradingMandate {
            venue: LEGACY_VENUE.to_owned(),
            network: self.mandate_network,
            collateral_asset: AssetId::sats(),
            objective,
            max_venue_balance: parse_editor(
                &self.mandate_max_venue_balance,
                "maximum venue balance",
                cx,
            )?,
            max_position_usd: parse_editor(&self.mandate_max_position, "maximum position", cx)?,
            max_leverage: parse_editor(&self.mandate_max_leverage, "maximum leverage", cx)?,
            daily_loss_stop: parse_editor(&self.mandate_daily_loss_stop, "daily loss stop", cx)?,
            max_orders_per_hour: parse_editor(
                &self.mandate_max_orders_per_hour,
                "maximum orders per hour",
                cx,
            )?,
            min_liquidation_buffer_bps: parse_editor(
                &self.mandate_min_liquidation_buffer,
                "minimum liquidation buffer",
                cx,
            )?,
            allowed_strategies,
            review_cadence,
            expires_at_ms: parse_editor(&self.mandate_expires_at, "mandate expiry", cx)?,
        };
        mandate.validate()?;
        Ok(mandate)
    }

    fn save_mandate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.mandate_store.clone() else {
            self.mandate_status =
                MandateStatus::Error("The trading mandate store is not available.".into());
            cx.notify();
            return;
        };
        let candidate = match self.mandate_candidate(cx) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.mandate_status = MandateStatus::Error(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let proposal = match store.propose(candidate) {
            Ok(proposal) => proposal,
            Err(error) => {
                self.mandate_status = MandateStatus::Error(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if !proposal.change_class().needs_ui_approval() {
            let result = unix_time_ms().and_then(|now_ms| store.save_restriction(proposal, now_ms));
            match result {
                Ok(snapshot) => self.apply_mandate_snapshot(snapshot, window, cx),
                Err(error) => {
                    self.mandate_status = MandateStatus::Error(error.to_string().into());
                    cx.notify();
                }
            }
            return;
        }

        let action = match proposal.change_class() {
            MandateChangeClass::Creation => "create",
            MandateChangeClass::Widening => "widen",
            MandateChangeClass::Restriction | MandateChangeClass::Unchanged => return,
        };
        let detail = mandate_approval_detail(proposal.candidate());
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Approve this request to {action} the trading mandate?"),
            Some(&detail),
            &["Approve mandate", "Cancel"],
            cx,
        );
        let base_revision = proposal.base_revision();
        self.mandate_status = MandateStatus::Saving;
        let task = cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                this.update(cx, |this, cx| {
                    this.mandate_operation = None;
                    this.mandate_status = if base_revision == 0 {
                        MandateStatus::Empty
                    } else {
                        MandateStatus::Saved {
                            revision: base_revision,
                        }
                    };
                    this.mandate_active_revision = (base_revision != 0).then_some(base_revision);
                    cx.notify();
                })
                .log_err();
                return;
            }
            let result = unix_time_ms()
                .and_then(|approved_at_ms| store.apply_ui_approved(proposal, approved_at_ms));
            this.update(cx, |this, cx| {
                this.mandate_operation = None;
                match result {
                    Ok(snapshot) => {
                        this.mandate_status = MandateStatus::Saved {
                            revision: snapshot.revision,
                        };
                        this.mandate_active_revision = Some(snapshot.revision);
                    }
                    Err(error) => {
                        this.mandate_status = MandateStatus::Error(error.to_string().into());
                    }
                }
                cx.notify();
            })
            .log_err();
        });
        self.mandate_operation = Some(task);
        cx.notify();
    }

    fn revoke_mandate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.mandate_store.clone() else {
            self.mandate_status =
                MandateStatus::Error("The trading mandate store is not available.".into());
            cx.notify();
            return;
        };
        let network = self.mandate_network;
        match unix_time_ms().and_then(|now_ms| store.revoke(LEGACY_VENUE, network, now_ms)) {
            Ok(snapshot) => self.apply_mandate_snapshot(snapshot, window, cx),
            Err(error) => {
                self.mandate_status = MandateStatus::Error(error.to_string().into());
                cx.notify();
            }
        }
    }

    fn set_credentials(
        &self,
        credentials: &Credentials,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.access_key.update(cx, |editor, cx| {
            editor.set_text(credentials.access_key(), window, cx)
        });
        self.secret.update(cx, |editor, cx| {
            editor.set_text(credentials.secret_value(), window, cx)
        });
        self.passphrase.update(cx, |editor, cx| {
            editor.set_text(credentials.passphrase(), window, cx)
        });
    }

    fn save_and_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let credentials = match Credentials::new(
            self.access_key.read(cx).text(cx),
            self.secret.read(cx).text(cx),
            self.passphrase.read(cx).text(cx),
        ) {
            Ok(credentials) => credentials,
            Err(error) => {
                self.status = ConnectionStatus::Error(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let network = self.network;
        let encoded = match StoredCredentials::new(network, &credentials).encode() {
            Ok(encoded) => encoded,
            Err(error) => {
                self.status = ConnectionStatus::Error(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let client =
            LnMarketsClient::authenticated(self.http_transport.clone(), network, credentials);
        let credentials_provider = self.credentials_provider.clone();
        self.status = ConnectionStatus::Testing;
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let account = client.account().await?;
                credentials_provider
                    .write_credentials(CREDENTIAL_STORAGE_URL, "LN Markets", &encoded, &*cx)
                    .await
                    .map_err(|error| anyhow::anyhow!("could not save credentials: {error}"))?;
                anyhow::Ok(account)
            }
            .await;
            this.update(cx, |this, cx| {
                this.operation = None;
                this.status = match result {
                    Ok(account) => ConnectionStatus::Connected(account),
                    Err(error) => ConnectionStatus::Error(error.to_string().into()),
                };
                cx.notify();
            })
            .log_err();
        });
        self.operation = Some(task);
        cx.notify();
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let credentials_provider = self.credentials_provider.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = credentials_provider
                .delete_credentials(CREDENTIAL_STORAGE_URL, &*cx)
                .await;
            this.update_in(cx, |this, window, cx| {
                this.operation = None;
                match result {
                    Ok(()) => {
                        this.access_key
                            .update(cx, |editor, cx| editor.set_text("", window, cx));
                        this.secret
                            .update(cx, |editor, cx| editor.set_text("", window, cx));
                        this.passphrase
                            .update(cx, |editor, cx| editor.set_text("", window, cx));
                        this.status = ConnectionStatus::Empty;
                    }
                    Err(error) => {
                        this.status = ConnectionStatus::Error(
                            format!("Could not remove credentials: {error}").into(),
                        );
                    }
                }
                cx.notify();
            })
            .log_err();
        });
        self.operation = Some(task);
        cx.notify();
    }
}

impl Render for LnMarketsSettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = matches!(
            self.status,
            ConnectionStatus::Loading | ConnectionStatus::Testing
        );
        let account = match &self.status {
            ConnectionStatus::Connected(account) => Some(account),
            _ => None,
        };
        let status = match &self.status {
            ConnectionStatus::Loading => Some((IconName::ArrowCircle, Color::Muted, "Loading")),
            ConnectionStatus::Saved => Some((IconName::Check, Color::Muted, "Saved")),
            ConnectionStatus::Testing => Some((IconName::ArrowCircle, Color::Accent, "Testing")),
            ConnectionStatus::Connected(_) => Some((IconName::Check, Color::Success, "Connected")),
            ConnectionStatus::Empty | ConnectionStatus::Error(_) => None,
        };
        let mandate_busy = matches!(self.mandate_status, MandateStatus::Saving);
        let mandate_active = self.mandate_active_revision.is_some();
        let mandate_status: (IconName, Color, SharedString) = match &self.mandate_status {
            MandateStatus::Empty => (IconName::Circle, Color::Muted, "No active mandate".into()),
            MandateStatus::Saved { revision } => (
                IconName::Check,
                Color::Success,
                format!("Active · revision {revision}").into(),
            ),
            MandateStatus::Saving => (IconName::ArrowCircle, Color::Accent, "Saving".into()),
            MandateStatus::Error(_) => (
                IconName::XCircle,
                Color::Error,
                self.mandate_active_revision.map_or_else(
                    || "Error".into(),
                    |revision| format!("Active · revision {revision} · edit error").into(),
                ),
            ),
        };

        v_flex()
            .id("lnmarkets-settings-page")
            .size_full()
            .px_8()
            .pb_16()
            .gap_4()
            .track_scroll(&self.scroll_handle)
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap_1()
                    .child(h_flex().gap_2().child(Label::new("LN Markets")).when_some(
                        status,
                        |row, (icon, color, label)| {
                            row.child(
                                h_flex()
                                    .gap_1()
                                    .child(Icon::new(icon).size(IconSize::XSmall).color(color))
                                    .child(Label::new(label).size(LabelSize::Small).color(color)),
                            )
                        },
                    ))
                    .child(
                        Label::new("Connect Omega directly to the LN Markets v3 API.")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(Divider::horizontal())
            .child(
                v_flex()
                    .gap_2()
                    .child(Label::new("Network").size(LabelSize::Small))
                    .child(
                        h_flex()
                            .gap_1()
                            .children([Network::Signet, Network::Mainnet].map(|network| {
                                let network_index = match network {
                                    Network::Signet => 0_usize,
                                    Network::Mainnet => 1_usize,
                                };
                                Button::new(("lnmarkets-network", network_index), network.label())
                                    .style(if self.network == network {
                                        ButtonStyle::Filled
                                    } else {
                                        ButtonStyle::Outlined
                                    })
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _window, cx| {
                                        this.network = network;
                                        if matches!(this.status, ConnectionStatus::Connected(_)) {
                                            this.status = ConnectionStatus::Saved;
                                        }
                                        cx.notify();
                                    }))
                            })),
                    )
                    .when(self.network == Network::Mainnet, |section| {
                        section.child(
                            Label::new(
                                "Mainnet swaps execute with real funds. The API key needs Trade permission.",
                            )
                            .size(LabelSize::Small)
                            .color(Color::Warning),
                        )
                    }),
            )
            .child(render_field("API Key", &self.access_key, cx))
            .child(render_field("Secret", &self.secret, cx))
            .child(render_field("Passphrase", &self.passphrase, cx))
            .when_some(account, |page, account| {
                page.child(Divider::horizontal()).child(
                    v_flex()
                        .gap_1()
                        .child(account_row("Username", account.username.clone()))
                        .child(account_row(
                            "BTC balance",
                            format!("{} sats", account.balance),
                        ))
                        .child(account_row(
                            "Synthetic USD",
                            account.synthetic_usd_balance.to_string(),
                        ))
                        .child(account_row("Fee tier", account.fee_tier.to_string())),
                )
            })
            .when_some(
                match &self.status {
                    ConnectionStatus::Error(error) => Some(error.clone()),
                    _ => None,
                },
                |page, error| {
                    page.child(
                        h_flex()
                            .gap_2()
                            .child(
                                Icon::new(IconName::XCircle)
                                    .size(IconSize::Small)
                                    .color(Color::Error),
                            )
                            .child(Label::new(error).size(LabelSize::Small).color(Color::Error)),
                    )
                },
            )
            .child(
                h_flex()
                    .gap_1()
                    .justify_end()
                    .child(
                        Button::new("lnmarkets-remove", "Remove")
                            .style(ButtonStyle::Outlined)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))),
                    )
                    .child(
                        Button::new("lnmarkets-save-test", "Save & Test")
                            .style(ButtonStyle::Filled)
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_and_test(window, cx)),
                            ),
                    ),
            )
            .child(Divider::horizontal())
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Label::new("Trading mandate"))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Icon::new(mandate_status.0)
                                            .size(IconSize::XSmall)
                                            .color(mandate_status.1),
                                    )
                                    .child(
                                        Label::new(mandate_status.2)
                                            .size(LabelSize::Small)
                                            .color(mandate_status.1),
                                    ),
                            ),
                    )
                    .child(
                        Label::new(
                            "No mandate means no trading. Creating or widening these limits requires a separate approval prompt. Restrictions and revocation take effect immediately.",
                        )
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                    )
                    .child(
                        Label::new(
                            "Funds held at a venue are counterparty exposure. Keep the venue balance limit small.",
                        )
                        .size(LabelSize::Small)
                        .color(Color::Warning),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(Label::new("Mandate network").size(LabelSize::Small))
                    .child(
                        h_flex()
                            .gap_1()
                            .children(
                                [TradingNetwork::Signet, TradingNetwork::Mainnet]
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, network)| {
                                        let label = match network {
                                            TradingNetwork::Signet => "Signet",
                                            TradingNetwork::Testnet => "Testnet",
                                            TradingNetwork::Mainnet => "Mainnet",
                                        };
                                        Button::new(("trading-mandate-network", index), label)
                                            .style(if self.mandate_network == network {
                                                ButtonStyle::Filled
                                            } else {
                                                ButtonStyle::Outlined
                                            })
                                            .disabled(mandate_busy)
                                            .on_click(cx.listener(
                                                move |this, _, _window, cx| {
                                                    this.mandate_network = network;
                                                    cx.notify();
                                                },
                                            ))
                                    }),
                            ),
                    ),
            )
            .child(render_field(
                "Objective",
                &self.mandate_objective,
                cx,
            ))
            .child(render_field(
                "Maximum venue balance (sats)",
                &self.mandate_max_venue_balance,
                cx,
            ))
            .child(render_field(
                "Maximum position (USD notional)",
                &self.mandate_max_position,
                cx,
            ))
            .child(render_field(
                "Maximum leverage",
                &self.mandate_max_leverage,
                cx,
            ))
            .child(render_field(
                "Daily loss stop (sats)",
                &self.mandate_daily_loss_stop,
                cx,
            ))
            .child(render_field(
                "Maximum orders per hour",
                &self.mandate_max_orders_per_hour,
                cx,
            ))
            .child(render_field(
                "Minimum liquidation buffer (basis points)",
                &self.mandate_min_liquidation_buffer,
                cx,
            ))
            .child(render_field(
                "Allowed strategies (comma-separated)",
                &self.mandate_allowed_strategies,
                cx,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(Label::new("Review cadence").size(LabelSize::Small))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new(
                                    "trading-mandate-cadence-funding",
                                    "Funding settlement",
                                )
                                .style(
                                    if matches!(
                                        self.mandate_review_cadence,
                                        ReviewCadence::FundingSettlement
                                    ) {
                                        ButtonStyle::Filled
                                    } else {
                                        ButtonStyle::Outlined
                                    },
                                )
                                .disabled(mandate_busy)
                                .on_click(cx.listener(|this, _, _window, cx| {
                                    this.mandate_review_cadence =
                                        ReviewCadence::FundingSettlement;
                                    cx.notify();
                                })),
                            )
                            .child(
                                Button::new("trading-mandate-cadence-interval", "Interval")
                                    .style(
                                        if matches!(
                                            self.mandate_review_cadence,
                                            ReviewCadence::Interval { .. }
                                        ) {
                                            ButtonStyle::Filled
                                        } else {
                                            ButtonStyle::Outlined
                                        },
                                    )
                                    .disabled(mandate_busy)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.mandate_review_cadence =
                                            ReviewCadence::Interval { seconds: 3_600 };
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when(
                matches!(
                    self.mandate_review_cadence,
                    ReviewCadence::Interval { .. }
                ),
                |page| {
                    page.child(render_field(
                        "Review interval (seconds)",
                        &self.mandate_review_interval,
                        cx,
                    ))
                },
            )
            .child(render_field(
                "Expires at (Unix milliseconds)",
                &self.mandate_expires_at,
                cx,
            ))
            .when_some(
                match &self.mandate_status {
                    MandateStatus::Error(error) => Some(error.clone()),
                    _ => None,
                },
                |page, error| {
                    page.child(
                        h_flex()
                            .gap_2()
                            .child(
                                Icon::new(IconName::XCircle)
                                    .size(IconSize::Small)
                                    .color(Color::Error),
                            )
                            .child(Label::new(error).size(LabelSize::Small).color(Color::Error)),
                    )
                },
            )
            .child(
                h_flex()
                    .gap_1()
                    .justify_end()
                    .child(
                        Button::new("trading-mandate-revoke", "Revoke")
                            .style(ButtonStyle::Outlined)
                            .disabled(mandate_busy || !mandate_active)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.revoke_mandate(window, cx)
                            })),
                    )
                    .child(
                        Button::new("trading-mandate-save", "Save mandate")
                            .style(ButtonStyle::Filled)
                            .disabled(mandate_busy || self.mandate_store.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_mandate(window, cx)
                            })),
                    ),
            )
    }
}

fn new_masked_input(
    placeholder: &str,
    window: &mut Window,
    cx: &mut Context<LnMarketsSettingsPage>,
) -> Entity<Editor> {
    cx.new(|cx| {
        let mut editor = Editor::single_line(window, cx);
        editor.set_placeholder_text(placeholder, window, cx);
        editor.set_masked(true, cx);
        editor
    })
}

fn new_text_input(
    text: &str,
    placeholder: &str,
    window: &mut Window,
    cx: &mut Context<LnMarketsSettingsPage>,
) -> Entity<Editor> {
    cx.new(|cx| {
        let mut editor = Editor::single_line(window, cx);
        editor.set_placeholder_text(placeholder, window, cx);
        editor.set_text(text, window, cx);
        editor
    })
}

fn set_editor_text(
    editor: &Entity<Editor>,
    text: &str,
    window: &mut Window,
    cx: &mut Context<LnMarketsSettingsPage>,
) {
    editor.update(cx, |editor, cx| editor.set_text(text, window, cx));
}

fn parse_editor<T>(editor: &Entity<Editor>, label: &str, cx: &App) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = editor.read(cx).text(cx);
    value
        .trim()
        .parse()
        .map_err(|error| anyhow::anyhow!("Invalid {label}: {error}"))
}

fn unix_time_ms() -> anyhow::Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| anyhow::anyhow!("system clock is before Unix epoch: {error}"))?;
    i64::try_from(duration.as_millis()).map_err(|_| anyhow::anyhow!("system time exceeded range"))
}

fn default_expiry_text() -> String {
    const THIRTY_DAYS_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
    match unix_time_ms()
        .and_then(|now_ms| {
            now_ms
                .checked_add(THIRTY_DAYS_MS)
                .ok_or_else(|| anyhow::anyhow!("default mandate expiry exceeded range"))
        })
        .map(|expires_at_ms| expires_at_ms.to_string())
    {
        Ok(expiry) => expiry,
        Err(error) => {
            log::error!("could not initialize the trading mandate expiry: {error}");
            String::new()
        }
    }
}

fn mandate_approval_detail(mandate: &TradingMandate) -> String {
    let network = match mandate.network {
        TradingNetwork::Signet => "signet",
        TradingNetwork::Testnet => "testnet",
        TradingNetwork::Mainnet => "mainnet",
    };
    let cadence = match &mandate.review_cadence {
        ReviewCadence::FundingSettlement => "each funding settlement".to_owned(),
        ReviewCadence::Interval { seconds } => format!("every {seconds} seconds"),
    };
    let asset = mandate.collateral_asset.as_str();
    format!(
        "Venue: {}\nNetwork: {network}\nObjective: {}\nMaximum venue balance: {} {asset}\nMaximum position: {} USD notional\nMaximum leverage: {}x\nDaily loss stop: {} {asset}\nMaximum orders: {} per hour\nMinimum liquidation buffer: {} bps\nAllowed strategies: {}\nReview: {cadence}\nExpires at: {} Unix ms",
        mandate.venue,
        mandate.objective,
        mandate.max_venue_balance,
        mandate.max_position_usd,
        mandate.max_leverage,
        mandate.daily_loss_stop,
        mandate.max_orders_per_hour,
        mandate.min_liquidation_buffer_bps,
        mandate
            .allowed_strategies
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        mandate.expires_at_ms,
    )
}

fn render_field(
    label: &'static str,
    editor: &Entity<Editor>,
    cx: &mut Context<LnMarketsSettingsPage>,
) -> AnyElement {
    let colors = cx.theme().colors();
    let focus_handle = editor.focus_handle(cx).tab_index(0).tab_stop(true);
    v_flex()
        .gap_1()
        .child(Label::new(label).size(LabelSize::Small))
        .child(
            h_flex()
                .w_full()
                .h_8()
                .px_2()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.editor_background)
                .track_focus(&focus_handle)
                .focus(|style| style.border_color(colors.border_focused))
                .child(editor.clone()),
        )
        .into_any_element()
}

fn account_row(label: &'static str, value: impl Into<SharedString>) -> impl IntoElement {
    h_flex()
        .justify_between()
        .child(Label::new(label).size(LabelSize::Small).color(Color::Muted))
        .child(Label::new(value.into()).size(LabelSize::Small))
}
