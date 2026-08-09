use std::sync::Arc;

use credentials_provider::CredentialsProvider;
use editor::Editor;
use gpui::{AppContext as _, Entity, Focusable as _, ScrollHandle, Task, prelude::*};
use http_client::HttpClient;
use lnmarkets_client::{
    Account, CREDENTIAL_STORAGE_URL, Credentials, LnMarketsClient, Network, StoredCredentials,
};
use ui::{Divider, prelude::*};
use util::ResultExt as _;

use crate::SettingsWindow;

pub(crate) struct LnMarketsSettingsPage {
    access_key: Entity<Editor>,
    secret: Entity<Editor>,
    passphrase: Entity<Editor>,
    network: Network,
    status: ConnectionStatus,
    http_client: Arc<dyn HttpClient>,
    credentials_provider: Arc<dyn CredentialsProvider>,
    operation: Option<Task<()>>,
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

impl LnMarketsSettingsPage {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            access_key: new_masked_input("API key", window, cx),
            secret: new_masked_input("Secret", window, cx),
            passphrase: new_masked_input("Passphrase", window, cx),
            network: Network::Signet,
            status: ConnectionStatus::Loading,
            http_client: cx.http_client(),
            credentials_provider: zed_credentials_provider::global(cx),
            operation: None,
            scroll_handle: ScrollHandle::new(),
        }
    }

    pub(crate) fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let client = LnMarketsClient::authenticated(self.http_client.clone(), network, credentials);
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
    }
}

pub(crate) fn render_lnmarkets_settings_page(
    settings_window: &SettingsWindow,
    _scroll_handle: &ScrollHandle,
    _window: &mut Window,
    _cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    settings_window
        .lnmarkets_settings_page
        .clone()
        .into_any_element()
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
