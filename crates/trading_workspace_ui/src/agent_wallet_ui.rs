use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use component::{Component, ComponentScope, example_group_with_title, single_example};
use credentials_provider::CredentialsProvider;
use editor::Editor;
use gpui::prelude::*;
use gpui::{
    App, ClipboardItem, Context, Entity, IntoElement, PromptLevel, Render, RenderOnce,
    ScrollHandle, SharedString, Subscription, Task, Window,
};
use nautilus_sidecar::{
    AGENT_WALLET_AUTHORITY_COPY, AgentApprovalStatus, AgentWalletHaltReason, AgentWalletSummary,
    NautilusCredentialSnapshot, NautilusCredentialState, NautilusStreamSource, Network,
    StreamEvent, credential_state, generate_and_store_agent_wallet, probe_official_venue_state,
    refresh_agent_wallet_approval,
};
use trading_ledger::{AssetId, LedgerEntryKind, LedgerQuery, LedgerStore};
use trading_mandate::{MandateStore, TradingNetwork};
use ui::{Divider, MarketTokens, prelude::*};
use util::ResultExt as _;

fn unix_ms() -> anyhow::Result<i64> {
    let milliseconds = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    i64::try_from(milliseconds).map_err(Into::into)
}

fn demo_summary() -> AgentWalletSummary {
    AgentWalletSummary {
        network: Network::Testnet,
        owner_address: "0x71c4…8e12".to_owned(),
        agent_address: "0xa021…0f73".to_owned(),
        agent_name: "omega-testnet".to_owned(),
        approval: AgentApprovalStatus::Approved {
            valid_until_ms: 1_781_000_000_000,
        },
    }
}

#[derive(IntoElement, RegisterComponent)]
pub struct AgentWalletAuthorityCard {
    summary: Option<AgentWalletSummary>,
    halt: Option<AgentWalletHaltReason>,
    tokens: Option<MarketTokens>,
}

impl AgentWalletAuthorityCard {
    pub fn new(summary: Option<AgentWalletSummary>, halt: Option<AgentWalletHaltReason>) -> Self {
        Self {
            summary,
            halt,
            tokens: None,
        }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

fn authority_status(
    summary: Option<&AgentWalletSummary>,
    halt: Option<&AgentWalletHaltReason>,
) -> (IconName, Color, SharedString) {
    if let Some(halt) = halt {
        return (
            IconName::XCircle,
            Color::Error,
            format!("Halted · {}", halt.code()).into(),
        );
    }
    match summary.map(|summary| &summary.approval) {
        Some(AgentApprovalStatus::Approved { valid_until_ms }) => (
            IconName::Check,
            Color::Success,
            format!("Approved · valid until {valid_until_ms}").into(),
        ),
        Some(AgentApprovalStatus::Pending) => (
            IconName::Circle,
            Color::Warning,
            "Awaiting owner approval".into(),
        ),
        Some(AgentApprovalStatus::Expired { valid_until_ms }) => (
            IconName::XCircle,
            Color::Error,
            format!("Expired · {valid_until_ms}").into(),
        ),
        Some(AgentApprovalStatus::Revoked) => (IconName::XCircle, Color::Error, "Revoked".into()),
        Some(AgentApprovalStatus::UnknownMode { .. }) => (
            IconName::XCircle,
            Color::Error,
            "Unknown approval mode".into(),
        ),
        None => (IconName::Circle, Color::Muted, "Not configured".into()),
    }
}

impl RenderOnce for AgentWalletAuthorityCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let grayscale = tokens == tokens.grayscale();
        let status = authority_status(self.summary.as_ref(), self.halt.as_ref());
        v_flex()
            .debug_selector(|| "nautilus.agent_wallet_authority".into())
            .when(grayscale, |this| {
                this.debug_selector(|| "nautilus.agent_wallet_authority_grayscale".into())
            })
            .gap_2()
            .p_3()
            .border_1()
            .border_color(tokens.grid)
            .rounded_md()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Hyperliquid agent wallet"))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Icon::new(status.0).size(IconSize::XSmall).color(status.1))
                            .child(Label::new(status.2).size(LabelSize::Small).color(status.1)),
                    ),
            )
            .child(
                Label::new(AGENT_WALLET_AUTHORITY_COPY)
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .when_some(self.summary, |card, summary| {
                card.child(
                    v_flex()
                        .gap_1()
                        .child(detail_row("Network", summary.network.label(), cx))
                        .child(detail_row("Name", summary.agent_name, cx))
                        .child(detail_row("Owner", summary.owner_address, cx))
                        .child(detail_row("Agent", summary.agent_address, cx)),
                )
            })
    }
}

fn detail_row(label: &'static str, value: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    h_flex()
        .justify_between()
        .gap_3()
        .child(
            Label::new(label)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(Label::new(value).size(LabelSize::XSmall).buffer_font(cx))
}

impl Component for AgentWalletAuthorityCard {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Network-bound Hyperliquid agent-wallet authority, approval expiry, and halt state."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> gpui::AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Agent-wallet authority",
                vec![single_example(
                    "Approved testnet agent",
                    AgentWalletAuthorityCard::new(Some(demo_summary()), None).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Authority survives without color",
                    AgentWalletAuthorityCard::new(
                        Some(demo_summary()),
                        Some(AgentWalletHaltReason::Expired {
                            valid_until_ms: 1_700_000_000_000,
                        }),
                    )
                    .tokens(MarketTokens::from_theme(cx).grayscale())
                    .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

pub struct NautilusAgentWalletSettingsPage {
    owner_address: Entity<Editor>,
    network: Network,
    credentials: Arc<dyn CredentialsProvider>,
    credential_state: Option<Entity<NautilusCredentialState>>,
    mandate_store: Option<MandateStore>,
    ledger_store: Option<LedgerStore>,
    _credential_subscription: Option<Subscription>,
    operation: Option<Task<()>>,
    local_error: Option<SharedString>,
    scroll_handle: ScrollHandle,
}

impl NautilusAgentWalletSettingsPage {
    pub fn new(
        credentials: Arc<dyn CredentialsProvider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let owner_address = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("0x owner address", window, cx);
            editor
        });
        let credential_state = credential_state(cx);
        let credential_subscription = credential_state
            .as_ref()
            .map(|state| cx.observe(state, |_this, _state, cx| cx.notify()));
        let mandate_store = MandateStore::open_default();
        let ledger_store = LedgerStore::open_default();
        let local_error = mandate_store
            .as_ref()
            .err()
            .map(ToString::to_string)
            .or_else(|| ledger_store.as_ref().err().map(ToString::to_string))
            .map(Into::into);
        Self {
            owner_address,
            network: Network::Testnet,
            credentials,
            credential_state,
            mandate_store: mandate_store.ok(),
            ledger_store: ledger_store.ok(),
            _credential_subscription: credential_subscription,
            operation: None,
            local_error,
            scroll_handle: ScrollHandle::new(),
        }
    }

    fn snapshot(&self, cx: &App) -> NautilusCredentialSnapshot {
        self.credential_state
            .as_ref()
            .map(|state| state.read(cx).snapshot())
            .unwrap_or_default()
    }

    fn summary(&self, cx: &App) -> Option<AgentWalletSummary> {
        let snapshot = self.snapshot(cx);
        match self.network {
            Network::Testnet => snapshot.testnet,
            Network::Mainnet => snapshot.mainnet,
        }
    }

    fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owner_address = self.owner_address.read(cx).text(cx).trim().to_owned();
        let network = self.network;
        let existing = self.summary(cx).is_some();
        let approval = existing.then(|| {
            window.prompt(
                PromptLevel::Warning,
                "Rotate this Hyperliquid agent wallet?",
                Some("The existing agent key will stop being used. Approve the new address from the owner wallet before trading resumes."),
                &["Rotate", "Cancel"],
                cx,
            )
        });
        let credentials = self.credentials.clone();
        let credential_state = self.credential_state.clone();
        self.local_error = None;
        let task = cx.spawn_in(window, async move |this, cx| {
            if let Some(approval) = approval
                && approval.await != Ok(0)
            {
                this.update(cx, |this, cx| {
                    this.operation = None;
                    cx.notify();
                })
                .log_err();
                return;
            }
            let result = async {
                let created_at_ms = unix_ms()?;
                generate_and_store_agent_wallet(
                    &credentials,
                    network,
                    owner_address,
                    created_at_ms,
                    &*cx,
                )
                .await
            }
            .await;
            if let Some(credential_state) = credential_state {
                credential_state.update(cx, |state, cx| match &result {
                    Ok(summary) => state.apply_wallet(summary.clone(), cx),
                    Err(error) => state.apply_error(error.to_string(), cx),
                });
            }
            this.update(cx, |this, cx| {
                this.operation = None;
                this.local_error = result.err().map(|error| error.to_string().into());
                cx.notify();
            })
            .log_err();
        });
        self.operation = Some(task);
        cx.notify();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.network == Network::Mainnet {
            self.local_error = Some("Mainnet connections are locked until graduation.".into());
            cx.notify();
            return;
        }
        let credentials = self.credentials.clone();
        let credential_state = self.credential_state.clone();
        let http_client = cx.http_client();
        self.local_error = None;
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = match unix_ms() {
                Ok(now_ms) => {
                    refresh_agent_wallet_approval(
                        http_client,
                        &credentials,
                        Network::Testnet,
                        now_ms,
                        &*cx,
                    )
                    .await
                }
                Err(error) => Err(error),
            };
            if let Some(credential_state) = credential_state {
                credential_state.update(cx, |state, cx| match &result {
                    Ok(summary) => state.apply_wallet(summary.clone(), cx),
                    Err(error) => state.apply_error(error.to_string(), cx),
                });
            }
            this.update(cx, |this, cx| {
                this.operation = None;
                this.local_error = result.err().map(|error| error.to_string().into());
                cx.notify();
            })
            .log_err();
        });
        self.operation = Some(task);
        cx.notify();
    }

    fn renew_testnet_mandate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.mandate_store.clone() else {
            self.local_error = Some("The trading mandate store is unavailable.".into());
            cx.notify();
            return;
        };
        let now_ms = match unix_ms() {
            Ok(now_ms) => now_ms,
            Err(error) => {
                self.local_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let proposal = store.snapshot().and_then(|snapshot| {
            let mut candidate = snapshot
                .mandate_for("hyperliquid", TradingNetwork::Testnet)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!("No Hyperliquid Testnet mandate exists to renew.")
                })?;
            candidate.expires_at_ms = now_ms
                .checked_add(3_600_000)
                .ok_or_else(|| anyhow::anyhow!("mandate expiry overflowed"))?;
            store.propose(candidate)
        });
        let proposal = match proposal {
            Ok(proposal) => proposal,
            Err(error) => {
                self.local_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let candidate = proposal.candidate();
        let detail = format!(
            "Hyperliquid Testnet only · strategies {} · max position ${} · max leverage {}× · max orders {}/hour · expires {}",
            candidate
                .allowed_strategies
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            candidate.max_position_usd,
            candidate.max_leverage,
            candidate.max_orders_per_hour,
            candidate.expires_at_ms,
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            "Renew this bounded Hyperliquid Testnet mandate for one hour?",
            Some(&detail),
            &["Approve mandate", "Cancel"],
            cx,
        );
        self.local_error = None;
        self.operation = Some(cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                this.update(cx, |this, cx| {
                    this.operation = None;
                    cx.notify();
                })
                .log_err();
                return;
            }
            let result = unix_ms()
                .and_then(|approved_at_ms| store.apply_ui_approved(proposal, approved_at_ms));
            this.update(cx, |this, cx| {
                this.operation = None;
                this.local_error = result.err().map(|error| error.to_string().into());
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn resolve_testnet_reconciliation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.ledger_store.clone() else {
            self.local_error = Some("The trading ledger store is unavailable.".into());
            cx.notify();
            return;
        };
        let Some(summary) = self.summary(cx) else {
            self.local_error = Some("Configure the Hyperliquid Testnet agent wallet first.".into());
            cx.notify();
            return;
        };
        let observed = match current_flat_testnet_balance(cx) {
            Ok(observed) => observed,
            Err(error) => {
                self.local_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let alert = store.entries(&LedgerQuery::default()).and_then(|entries| {
            entries
                .into_iter()
                .rev()
                .find(|entry| {
                    matches!(
                        &entry.kind,
                        LedgerEntryKind::ReconciliationMismatch(mismatch)
                            if mismatch.venue == "hyperliquid"
                                && mismatch.asset == AssetId::usdc()
                    )
                })
                .ok_or_else(|| anyhow::anyhow!("No Hyperliquid USDC reconciliation alert exists."))
        });
        let alert = match alert {
            Ok(alert) => alert,
            Err(error) => {
                self.local_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let detail = format!(
            "Append a balanced resolution for alert entry {} ({}) after both the official venue API and Nautilus report zero open orders and zero positions. No history is changed or deleted.",
            alert.sequence, alert.entry_hash,
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            "Resolve this verified Testnet reconciliation gap?",
            Some(&detail),
            &["Append resolution", "Cancel"],
            cx,
        );
        let http_client = cx.http_client();
        let owner_address = summary.owner_address;
        self.local_error = None;
        self.operation = Some(cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                this.update(cx, |this, cx| {
                    this.operation = None;
                    cx.notify();
                })
                .log_err();
                return;
            }
            let official = probe_official_venue_state(
                http_client,
                Network::Testnet,
                &owner_address,
            )
            .await;
            let result = match official {
                Ok(official) if official.is_zero_exposure() => this
                    .update(cx, |_this, cx| {
                        let current = current_flat_testnet_balance(cx)?;
                        if current != observed {
                            anyhow::bail!(
                                "Nautilus account balance changed while reconciliation approval was open"
                            );
                        }
                        let now_ms = unix_ms()?;
                        store.resolve_reconciliation_gap(
                            format!("nautilus-reconciliation-resolution-{now_ms}"),
                            now_ms,
                            "OMEGA-BOUNDED-QUOTE-001",
                            "hyperliquid",
                            AssetId::usdc(),
                            observed,
                        )?;
                        Ok::<(), anyhow::Error>(())
                    })
                    .and_then(|result| result),
                Ok(_) => Err(anyhow::anyhow!(
                    "Official Hyperliquid Testnet state is not flat and order-free"
                )),
                Err(error) => Err(error),
            };
            this.update(cx, |this, cx| {
                this.operation = None;
                self::set_local_result(this, result);
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }
}

fn set_local_result(this: &mut NautilusAgentWalletSettingsPage, result: anyhow::Result<()>) {
    this.local_error = result.err().map(|error| error.to_string().into());
}

fn current_flat_testnet_balance(cx: &App) -> anyhow::Result<i64> {
    let source = NautilusStreamSource::try_global(cx)
        .ok_or_else(|| anyhow::anyhow!("Nautilus Testnet stream is unavailable"))?;
    let snapshot = source.read(cx).market_snapshot();
    if !snapshot.orders.is_empty() || !snapshot.positions.is_empty() {
        anyhow::bail!("Nautilus does not report a flat, order-free Testnet account");
    }
    let account = snapshot
        .account
        .ok_or_else(|| anyhow::anyhow!("Nautilus has not reported a Testnet account"))?;
    let StreamEvent::Account { state, .. } = account else {
        anyhow::bail!("Nautilus account projection has an unexpected event type");
    };
    let balances = state
        .get("balances")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Nautilus account has no balances"))?;
    let total = balances
        .iter()
        .find(|balance| {
            balance
                .get("currency")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|currency| currency.eq_ignore_ascii_case("USDC"))
        })
        .and_then(|balance| balance.get("total"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Nautilus account has no USDC total"))?;
    decimal_usdc_micros(total)
}

fn decimal_usdc_micros(value: &str) -> anyhow::Result<i64> {
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    if whole.starts_with('-')
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        anyhow::bail!("USDC balance is not a non-negative six-decimal value");
    }
    let (fractional, excess) = if fractional.len() > 6 {
        fractional.split_at(6)
    } else {
        (fractional, "")
    };
    if !excess.bytes().all(|byte| byte == b'0') {
        anyhow::bail!("USDC balance is not a non-negative six-decimal value");
    }
    let whole = whole.parse::<i64>()?;
    let mut padded = fractional.to_owned();
    padded.extend(std::iter::repeat_n('0', 6 - padded.len()));
    whole
        .checked_mul(1_000_000)
        .and_then(|whole| {
            padded
                .parse::<i64>()
                .ok()
                .and_then(|fraction| whole.checked_add(fraction))
        })
        .ok_or_else(|| anyhow::anyhow!("USDC balance exceeds the supported range"))
}

impl Render for NautilusAgentWalletSettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.operation.is_some();
        let snapshot = self.snapshot(cx);
        let summary = match self.network {
            Network::Testnet => snapshot.testnet,
            Network::Mainnet => snapshot.mainnet,
        };
        let configured = summary.is_some();
        let halt = (snapshot.selected_network == self.network)
            .then_some(snapshot.halt)
            .flatten();
        v_flex()
            .id("nautilus-agent-wallet-settings-page")
            .debug_selector(|| "nautilus.agent_wallet_settings".into())
            .size_full()
            .px_8()
            .pb_16()
            .gap_4()
            .track_scroll(&self.scroll_handle)
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap_1()
                    .child(Label::new("Hyperliquid agent wallet"))
                    .child(
                        Label::new(AGENT_WALLET_AUTHORITY_COPY)
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(Divider::horizontal())
            .child(
                h_flex().gap_1().children(
                    [Network::Testnet, Network::Mainnet]
                        .into_iter()
                        .enumerate()
                        .map(|(index, network)| {
                            Button::new(("nautilus-agent-network", index), network.label())
                                .style(if self.network == network {
                                    ButtonStyle::Filled
                                } else {
                                    ButtonStyle::Outlined
                                })
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _window, cx| {
                                    this.network = network;
                                    this.local_error = None;
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .when(self.network == Network::Mainnet, |page| {
                page.child(
                    Label::new("Mainnet execution and network checks are locked until graduation.")
                        .size(LabelSize::Small)
                        .color(Color::Warning),
                )
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(Label::new("Owner address").size(LabelSize::Small))
                    .child(
                        div()
                            .h_8()
                            .px_2()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .rounded_md()
                            .child(self.owner_address.clone()),
                    ),
            )
            .child(AgentWalletAuthorityCard::new(summary.clone(), halt))
            .when(self.network == Network::Testnet, |page| {
                let mandate_status = self
                    .mandate_store
                    .as_ref()
                    .and_then(|store| store.snapshot().ok())
                    .and_then(|snapshot| {
                        snapshot
                            .mandate_for("hyperliquid", TradingNetwork::Testnet)
                            .map(|mandate| {
                                format!(
                                    "Revision {} · expires {} · max position ${} · {} orders/hour",
                                    snapshot.revision,
                                    mandate.expires_at_ms,
                                    mandate.max_position_usd,
                                    mandate.max_orders_per_hour,
                                )
                            })
                    })
                    .unwrap_or_else(|| "No Hyperliquid Testnet mandate".to_owned());
                page.child(
                    v_flex()
                        .debug_selector(|| "nautilus.testnet_policy_recovery".into())
                        .gap_2()
                        .p_3()
                        .border_1()
                        .border_color(cx.theme().colors().border)
                        .rounded_md()
                        .child(Label::new("Testnet mandate and ledger recovery"))
                        .child(
                            Label::new(mandate_status)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("Reconciliation resolutions append a balanced, hash-linked correction only after the official venue API and Nautilus both report no open orders or positions. History is never reset.")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_1()
                                .child(
                                    Button::new(
                                        "nautilus-resolve-reconciliation",
                                        "Resolve verified ledger gap",
                                    )
                                    .style(ButtonStyle::Outlined)
                                    .disabled(busy || !configured)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.resolve_testnet_reconciliation(window, cx)
                                    })),
                                )
                                .child(
                                    Button::new(
                                        "nautilus-renew-mandate",
                                        "Renew bounded mandate for one hour",
                                    )
                                    .style(ButtonStyle::Outlined)
                                    .disabled(busy || !configured)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.renew_testnet_mandate(window, cx)
                                    })),
                                ),
                        ),
                )
            })
            .when_some(summary, |page, summary| {
                let agent_address = summary.agent_address;
                page.child(
                    Button::new("nautilus-copy-agent-address", "Copy agent address")
                        .style(ButtonStyle::Outlined)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(agent_address.clone()));
                        }),
                )
            })
            .when_some(
                self.local_error
                    .clone()
                    .or_else(|| snapshot.error.map(Into::into)),
                |page, error| {
                    page.child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(IconName::XCircle).color(Color::Error))
                            .child(Label::new(error).size(LabelSize::Small).color(Color::Error)),
                    )
                },
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_1()
                    .child(
                        Button::new("nautilus-refresh-agent", "Refresh approval")
                            .style(ButtonStyle::Outlined)
                            .disabled(busy || self.network == Network::Mainnet)
                            .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                    )
                    .child(
                        Button::new(
                            "nautilus-generate-agent",
                            if self.summary(cx).is_some() {
                                "Rotate agent wallet"
                            } else {
                                "Generate agent wallet"
                            },
                        )
                        .style(ButtonStyle::Filled)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.generate(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    struct AuthorityCards;

    impl Render for AuthorityCards {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .child(AgentWalletAuthorityCard::new(Some(demo_summary()), None))
                .child(
                    AgentWalletAuthorityCard::new(
                        Some(demo_summary()),
                        Some(AgentWalletHaltReason::UnknownMode {
                            raw: "future".to_owned(),
                        }),
                    )
                    .tokens(MarketTokens::from_theme(cx).grayscale()),
                )
        }
    }

    #[gpui::test]
    fn authority_card_paints_normal_and_grayscale(cx: &mut TestAppContext) {
        init_test(cx);
        let (_view, cx) = cx.add_window_view(|_, _| AuthorityCards);
        cx.run_until_parked();
        let rendered = cx.debug_render_snapshot();
        assert!(
            !rendered
                .occurrences("nautilus.agent_wallet_authority")
                .is_empty()
        );
        assert!(
            !rendered
                .occurrences("nautilus.agent_wallet_authority_grayscale")
                .is_empty()
        );
    }

    #[test]
    fn usdc_balance_parser_is_exact_and_fail_closed() {
        assert_eq!(
            decimal_usdc_micros("987.913135").expect("balance"),
            987_913_135
        );
        assert_eq!(
            decimal_usdc_micros("987.91313500").expect("canonical trailing zeros"),
            987_913_135
        );
        assert_eq!(decimal_usdc_micros("1").expect("whole balance"), 1_000_000);
        assert!(decimal_usdc_micros("1.0000001").is_err());
        assert!(decimal_usdc_micros("-1").is_err());
        assert!(decimal_usdc_micros("NaN").is_err());
    }
}
