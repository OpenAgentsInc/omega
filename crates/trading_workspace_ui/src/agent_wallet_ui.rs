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
    NautilusCredentialSnapshot, NautilusCredentialState, Network, credential_state,
    generate_and_store_agent_wallet, refresh_agent_wallet_approval,
};
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
        Self {
            owner_address,
            network: Network::Testnet,
            credentials,
            credential_state,
            _credential_subscription: credential_subscription,
            operation: None,
            local_error: None,
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
}

impl Render for NautilusAgentWalletSettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.operation.is_some();
        let snapshot = self.snapshot(cx);
        let summary = match self.network {
            Network::Testnet => snapshot.testnet,
            Network::Mainnet => snapshot.mainnet,
        };
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
}
