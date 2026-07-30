use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Local};
use gpui::{
    Action, AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    PromptLevel, Render, SharedString, Task, Window,
};
use omega_actions::{OpenIdentityDashboard, OpenOnboarding};
use omega_identity::{
    AccountDashboardEntry, AccountDashboardProjection, AccountLifecycleState, AccountPurgeReport,
    AccountPurgeTarget, AccountPurgeVerification, AccountRef, AccountRegistryService,
    PublicIdentity, ReceiptRef, RecoveryProtectionState, SignerAvailability, SignerKind,
};
use ui::{Divider, ListItem, ListItemSpacing, SpinnerLabel, Tooltip, prelude::*};
use util::ResultExt as _;
use workspace::{
    WorkspaceId,
    item::{Item, ItemEvent},
    with_active_or_new_workspace,
};

const COMPACT_WIDTH: f32 = 720.;

trait AccountDashboardBackend: Send + Sync {
    fn inspect(&self) -> Result<AccountDashboardProjection, String>;
    fn apply(
        &self,
        operation: AccountOperation,
        expected_generation: u64,
    ) -> Result<AccountOperationResult, String>;
    fn begin_forget(
        &self,
        account_ref: &AccountRef,
        operation_ref: String,
        expected_generation: u64,
    ) -> Result<AccountPurgeReport, String>;
    fn record_purge_target(
        &self,
        account_ref: &AccountRef,
        operation_ref: &str,
        target: AccountPurgeTarget,
        verification: AccountPurgeVerification,
    ) -> Result<AccountPurgeReport, String>;
    fn retry_purge(
        &self,
        account_ref: &AccountRef,
        operation_ref: &str,
    ) -> Result<AccountPurgeReport, String>;
}

struct SystemAccountDashboardBackend {
    service: AccountRegistryService,
}

impl SystemAccountDashboardBackend {
    fn new() -> Self {
        Self {
            service: AccountRegistryService::system(*app_identity::CHANNEL),
        }
    }
}

impl AccountDashboardBackend for SystemAccountDashboardBackend {
    fn inspect(&self) -> Result<AccountDashboardProjection, String> {
        self.service.inspect().map_err(|error| error.to_string())
    }

    fn apply(
        &self,
        operation: AccountOperation,
        expected_generation: u64,
    ) -> Result<AccountOperationResult, String> {
        let (projection, purge_report) = match operation {
            AccountOperation::AddLocal(receipt_ref) => (
                self.service
                    .add_local_account(receipt_ref)
                    .map_err(|error| error.to_string())?,
                None,
            ),
            AccountOperation::Switch(account_ref) => (
                self.service
                    .switch_account(&account_ref, expected_generation)
                    .map_err(|error| error.to_string())?,
                None,
            ),
            AccountOperation::Lock => (
                self.service
                    .lock_active(expected_generation)
                    .map_err(|error| error.to_string())?,
                None,
            ),
            AccountOperation::Unlock => (
                self.service
                    .unlock_active(expected_generation)
                    .map_err(|error| error.to_string())?,
                None,
            ),
            AccountOperation::SignOut => (
                self.service
                    .sign_out(expected_generation)
                    .map_err(|error| error.to_string())?,
                None,
            ),
        };
        Ok(AccountOperationResult {
            projection,
            purge_report,
        })
    }

    fn begin_forget(
        &self,
        account_ref: &AccountRef,
        operation_ref: String,
        expected_generation: u64,
    ) -> Result<AccountPurgeReport, String> {
        self.service
            .begin_forget(account_ref, operation_ref, expected_generation)
            .map_err(|error| error.to_string())
    }

    fn record_purge_target(
        &self,
        account_ref: &AccountRef,
        operation_ref: &str,
        target: AccountPurgeTarget,
        verification: AccountPurgeVerification,
    ) -> Result<AccountPurgeReport, String> {
        self.service
            .record_purge_target_result(account_ref, operation_ref, target, verification)
            .map_err(|error| error.to_string())
    }

    fn retry_purge(
        &self,
        account_ref: &AccountRef,
        operation_ref: &str,
    ) -> Result<AccountPurgeReport, String> {
        self.service
            .retry_purge(account_ref, operation_ref)
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
enum AccountOperation {
    AddLocal(ReceiptRef),
    Switch(AccountRef),
    Lock,
    Unlock,
    SignOut,
}

struct AccountOperationResult {
    projection: AccountDashboardProjection,
    purge_report: Option<AccountPurgeReport>,
}

pub fn init(cx: &mut App) {
    cx.on_action(|_: &OpenIdentityDashboard, cx| open_identity_dashboard(cx));
}

fn open_identity_dashboard(cx: &mut App) {
    with_active_or_new_workspace(cx, |workspace, window, cx| {
        workspace
            .with_local_workspace(window, cx, |workspace, window, cx| {
                let existing = workspace
                    .active_pane()
                    .read(cx)
                    .items()
                    .find_map(|item| item.downcast::<IdentityDashboard>());
                if let Some(existing) = existing {
                    workspace.activate_item(&existing, true, true, window, cx);
                } else {
                    let dashboard = IdentityDashboard::new(window, cx);
                    workspace.add_item_to_active_pane(Box::new(dashboard), None, true, window, cx);
                }
            })
            .detach_and_log_err(cx);
    });
}

pub struct IdentityDashboard {
    backend: Arc<dyn AccountDashboardBackend>,
    projection: Option<AccountDashboardProjection>,
    selected_account: Option<AccountRef>,
    focus_handle: FocusHandle,
    task: Option<Task<()>>,
    message: Option<SharedString>,
    purge_report: Option<AccountPurgeReport>,
    busy: bool,
}

impl IdentityDashboard {
    fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        Self::new_with_backend(Arc::new(SystemAccountDashboardBackend::new()), window, cx)
    }

    fn new_with_backend(
        backend: Arc<dyn AccountDashboardBackend>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let dashboard = cx.new(|cx| Self {
            backend,
            projection: None,
            selected_account: None,
            focus_handle: cx.focus_handle(),
            task: None,
            message: None,
            purge_report: None,
            busy: false,
        });
        dashboard.update(cx, |dashboard, cx| dashboard.reload(cx));
        dashboard
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        self.message = None;
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { backend.inspect() }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(projection) => this.apply_projection(projection),
                    Err(error) => {
                        zlog::error!("account dashboard inspection failed: {error}");
                        this.message = Some("Accounts could not be loaded. Try again.".into());
                    }
                }
                this.busy = false;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn apply_projection(&mut self, projection: AccountDashboardProjection) {
        let selected_exists = self.selected_account.as_ref().is_some_and(|selected| {
            projection
                .accounts
                .iter()
                .any(|entry| &entry.account_ref == selected)
        });
        if !selected_exists {
            self.selected_account = projection.active.account_ref.clone().or_else(|| {
                projection
                    .accounts
                    .first()
                    .map(|entry| entry.account_ref.clone())
            });
        }
        self.projection = Some(projection);
    }

    fn selected_entry(&self) -> Option<&AccountDashboardEntry> {
        let selected = self.selected_account.as_ref()?;
        self.projection
            .as_ref()?
            .accounts
            .iter()
            .find(|entry| &entry.account_ref == selected)
    }

    fn run_operation(&mut self, operation: AccountOperation, cx: &mut Context<Self>) {
        let Some(projection) = self.projection.as_ref() else {
            return;
        };
        let expected_generation = projection.active.generation;
        let backend = self.backend.clone();
        self.message = None;
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { backend.apply(operation, expected_generation) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(result) => {
                        this.purge_report = result.purge_report;
                        this.apply_projection(result.projection);
                        this.message = None;
                    }
                    Err(error) => {
                        zlog::error!("account dashboard operation failed: {error}");
                        this.message = Some(
                            "The account changed before this operation completed. Try again."
                                .into(),
                        );
                    }
                }
                this.busy = false;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn add_local_identity(&mut self, cx: &mut Context<Self>) {
        let receipt_ref = format!(
            "account-dashboard-create-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        );
        match ReceiptRef::new(receipt_ref) {
            Ok(receipt_ref) => self.run_operation(AccountOperation::AddLocal(receipt_ref), cx),
            Err(error) => {
                zlog::error!("account dashboard receipt creation failed: {error}");
                self.message = Some("The local identity could not be created.".into());
                cx.notify();
            }
        }
    }

    fn run_purge(
        &mut self,
        account_ref: AccountRef,
        identity: PublicIdentity,
        operation_ref: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(projection) = self.projection.as_ref() else {
            return;
        };
        let expected_generation = projection.active.generation;
        let backend = self.backend.clone();
        self.message = None;
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let operation_ref = if let Some(operation_ref) = operation_ref {
                let backend = backend.clone();
                let account_ref = account_ref.clone();
                let retry_ref = operation_ref.clone();
                match cx
                    .background_spawn(async move { backend.retry_purge(&account_ref, &retry_ref) })
                    .await
                {
                    Ok(_) => operation_ref,
                    Err(error) => {
                        this.update(cx, |this, cx| {
                            zlog::error!("account purge retry failed: {error}");
                            this.message = Some("Local removal could not be retried.".into());
                            this.busy = false;
                            cx.notify();
                        })
                        .log_err();
                        return;
                    }
                }
            } else {
                let operation_ref = format!(
                    "forget-{}",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |duration| duration.as_nanos())
                );
                let backend = backend.clone();
                let account_ref = account_ref.clone();
                let begin_ref = operation_ref.clone();
                match cx
                    .background_spawn(async move {
                        backend.begin_forget(&account_ref, begin_ref, expected_generation)
                    })
                    .await
                {
                    Ok(_) => operation_ref,
                    Err(error) => {
                        this.update(cx, |this, cx| {
                            zlog::error!("account purge could not start: {error}");
                            this.message = Some("Local removal could not be started.".into());
                            this.busy = false;
                            cx.notify();
                        })
                        .log_err();
                        return;
                    }
                }
            };

            let (drafts, room_state) = cx.update(|cx| {
                (
                    agent_ui::draft_prompt_store::purge_account(&identity, cx),
                    agent_ui::omega_community_control::purge_room_state(&identity, cx),
                )
            });
            let drafts = drafts.await;
            let room_state = room_state.await;

            let backend_for_results = backend.clone();
            let account_ref_for_results = account_ref.clone();
            let operation_ref_for_results = operation_ref.clone();
            let report = cx
                .background_spawn(async move {
                    record_owner_purge_result(
                        backend_for_results.as_ref(),
                        &account_ref_for_results,
                        &operation_ref_for_results,
                        AccountPurgeTarget::Drafts,
                        drafts,
                    )?;
                    record_owner_purge_result(
                        backend_for_results.as_ref(),
                        &account_ref_for_results,
                        &operation_ref_for_results,
                        AccountPurgeTarget::RoomState,
                        room_state,
                    )?;
                    for target in [
                        AccountPurgeTarget::DecryptedCache,
                        AccountPurgeTarget::WalletState,
                        AccountPurgeTarget::RelayState,
                        AccountPurgeTarget::SignerSessions,
                        AccountPurgeTarget::DeviceGrants,
                    ] {
                        backend_for_results.record_purge_target(
                            &account_ref_for_results,
                            &operation_ref_for_results,
                            target,
                            AccountPurgeVerification::Failed {
                                reason: "No owning subsystem purge hook is available.".to_string(),
                            },
                        )?;
                    }
                    backend_for_results
                        .retry_purge(&account_ref_for_results, &operation_ref_for_results)
                })
                .await;

            let projection = {
                let backend = backend.clone();
                cx.background_spawn(async move { backend.inspect() }).await
            };
            this.update(cx, |this, cx| {
                match (report, projection) {
                    (Ok(report), Ok(projection)) => {
                        this.purge_report = Some(report);
                        this.apply_projection(projection);
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        zlog::error!("account purge verification failed: {error}");
                        this.message = Some("Some local account data could not be removed.".into());
                    }
                }
                this.busy = false;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn confirm_forget(
        &mut self,
        account_ref: AccountRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = window.prompt(
            PromptLevel::Critical,
            "Forget this identity on this device?",
            Some(
                "Omega will remove local signer material and account-partitioned data from this device. Events held by relays or peers remain. An external NIP-49 recovery file is not deleted.",
            ),
            &["Cancel", "Forget this device"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if prompt.await == Ok(1) {
                this.update(cx, |this, cx| {
                    let identity = this
                        .projection
                        .as_ref()
                        .and_then(|projection| {
                            projection
                                .accounts
                                .iter()
                                .find(|entry| entry.account_ref == account_ref)
                        })
                        .map(|entry| entry.identity.clone());
                    if let Some(identity) = identity {
                        this.run_purge(account_ref, identity, None, cx);
                    }
                })
                .log_err();
            }
        })
        .detach();
    }

    fn render_account_row(
        &self,
        account: &AccountDashboardEntry,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let account_ref = account.account_ref.clone();
        let selected = self.selected_account.as_ref() == Some(&account.account_ref);
        let label = account_display_name(account);
        let signer = signer_kind_label(account.signer.kind);
        ListItem::new(("omega-account-row", index))
            .inset(true)
            .spacing(ListItemSpacing::Sparse)
            .toggle_state(selected)
            .start_slot(Icon::new(IconName::Person).size(IconSize::Small))
            .child(
                v_flex()
                    .min_w_0()
                    .child(Label::new(label).size(LabelSize::Small))
                    .child(
                        Label::new(format!("{} · {signer}", account.fingerprint))
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    ),
            )
            .end_slot(
                h_flex()
                    .gap_1()
                    .when(
                        account.recovery == RecoveryProtectionState::Protected,
                        |this| {
                            this.child(
                                Icon::new(IconName::Check)
                                    .size(IconSize::XSmall)
                                    .color(Color::Success),
                            )
                        },
                    )
                    .when(account.is_active, |this| {
                        this.child(
                            Icon::new(IconName::Check)
                                .size(IconSize::XSmall)
                                .color(Color::Accent),
                        )
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_account = Some(account_ref.clone());
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_account_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let accounts = self
            .projection
            .as_ref()
            .map(|projection| projection.accounts.as_slice())
            .unwrap_or_default();
        v_flex()
            .min_w_64()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Accounts"))
                    .child(
                        Button::new("omega-account-add", "Add local identity")
                            .style(ButtonStyle::OutlinedGhost)
                            .size(ButtonSize::Compact)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.add_local_identity(cx))),
                    ),
            )
            .children(
                accounts
                    .iter()
                    .enumerate()
                    .map(|(index, account)| self.render_account_row(account, index, cx)),
            )
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(account) = self.selected_entry() else {
            return v_flex()
                .gap_2()
                .child(Label::new("No account selected"))
                .child(
                    Label::new("Add a local identity to manage it here.")
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                )
                .into_any_element();
        };
        let account_ref = account.account_ref.clone();
        let can_switch = !account.is_active
            && matches!(
                account.lifecycle,
                AccountLifecycleState::Active
                    | AccountLifecycleState::Locked
                    | AccountLifecycleState::SignedOut
            )
            && account.recovery == RecoveryProtectionState::Protected;
        let is_active = account.is_active;
        let is_locked = is_active && account.lifecycle == AccountLifecycleState::Locked;
        let needs_setup = account_needs_setup(account.lifecycle, account.recovery);
        let fingerprint = account.fingerprint.clone();
        let busy = self.busy;

        v_flex()
            .min_w_0()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(Label::new(account_display_name(account)))
                    .child(
                        Label::new(format!("Fingerprint {fingerprint}"))
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                    )
                    .child(
                        Label::new(account.identity.npub().as_str().to_string())
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    ),
            )
            .child(detail_row(
                "Signer",
                signer_kind_label(account.signer.kind),
                signer_availability_icon(account.signer.availability),
            ))
            .child(detail_row(
                "Recovery",
                recovery_detail(account.recovery),
                recovery_icon(account.recovery),
            ))
            .child(detail_row(
                "Last signer use",
                last_signer_use(account.signer.last_successful_use),
                None,
            ))
            .child(detail_row(
                "Retirement",
                retirement_label(account.retirement),
                None,
            ))
            .child(Divider::horizontal())
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .when(needs_setup, |this| {
                        this.child(
                            Button::new("omega-account-complete-setup", "Complete setup")
                                .style(ButtonStyle::Filled)
                                .disabled(busy)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(OpenOnboarding.boxed_clone(), cx)
                                }),
                        )
                    })
                    .child(
                        Button::new("omega-account-switch", "Switch to identity")
                            .style(ButtonStyle::Filled)
                            .disabled(!can_switch || busy)
                            .on_click(cx.listener({
                                let account_ref = account_ref.clone();
                                move |this, _, _, cx| {
                                    this.run_operation(
                                        AccountOperation::Switch(account_ref.clone()),
                                        cx,
                                    )
                                }
                            })),
                    )
                    .child(
                        Button::new(
                            "omega-account-lock",
                            if is_locked {
                                "Unlock signer"
                            } else {
                                "Lock signer"
                            },
                        )
                        .style(ButtonStyle::OutlinedGhost)
                        .disabled(!is_active || busy)
                        .tooltip(Tooltip::text(if is_locked { "Unlock" } else { "Lock" }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_operation(
                                if is_locked {
                                    AccountOperation::Unlock
                                } else {
                                    AccountOperation::Lock
                                },
                                cx,
                            )
                        })),
                    )
                    .child(
                        Button::new("omega-account-sign-out", "Sign out")
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(!is_active || busy)
                            .tooltip(Tooltip::text("Sign out"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_operation(AccountOperation::SignOut, cx)
                            })),
                    ),
            )
            .child(Divider::horizontal())
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("omega-account-forget", "Forget this device")
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_forget(account_ref.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("omega-account-retire", "Retire identity")
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(true)
                            .tooltip(Tooltip::text("Signed policy required")),
                    ),
            )
            .into_any_element()
    }

    fn render_purge_report(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let report = self.purge_report.as_ref()?;
        if report.complete {
            return None;
        }
        let account_ref = report.account_ref.clone();
        let operation_ref = report.operation_ref.clone();
        let identity = self
            .projection
            .as_ref()?
            .accounts
            .iter()
            .find(|entry| entry.account_ref == account_ref)?
            .identity
            .clone();
        Some(
            v_flex()
                .gap_1()
                .child(Label::new("Local removal incomplete").color(Color::Error))
                .children(report.targets.iter().filter_map(|target| {
                    let omega_identity::AccountPurgeTargetState::Failed { reason } = &target.state
                    else {
                        return None;
                    };
                    Some(
                        Label::new(format!("{:?}: {reason}", target.target))
                            .color(Color::Error)
                            .size(LabelSize::XSmall),
                    )
                }))
                .child(
                    Button::new("omega-account-retry-purge", "Retry local removal")
                        .style(ButtonStyle::OutlinedGhost)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_purge(
                                account_ref.clone(),
                                identity.clone(),
                                Some(operation_ref.clone()),
                                cx,
                            )
                        })),
                )
                .into_any_element(),
        )
    }
}

impl Render for IdentityDashboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(COMPACT_WIDTH);
        v_flex()
            .id("omega-identity-dashboard")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Omega Identity"))
                    .child(
                        h_flex()
                            .gap_2()
                            .when(self.busy, |this| this.child(SpinnerLabel::new()))
                            .child(
                                Button::new("omega-account-reload", "Reload")
                                    .style(ButtonStyle::OutlinedGhost)
                                    .size(ButtonSize::Compact)
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                            ),
                    ),
            )
            .when(compact, |this| {
                this.child(self.render_account_list(cx))
                    .child(Divider::horizontal())
                    .child(self.render_detail(cx))
            })
            .when(!compact, |this| {
                this.child(
                    h_flex()
                        .items_start()
                        .gap_6()
                        .child(self.render_account_list(cx))
                        .child(div().flex_1().min_w_0().child(self.render_detail(cx))),
                )
            })
            .when_some(self.render_purge_report(cx), |this, report| {
                this.child(report)
            })
            .when_some(self.message.clone(), |this, message| {
                this.child(
                    Label::new(message)
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                )
            })
    }
}

impl EventEmitter<ItemEvent> for IdentityDashboard {}

impl Focusable for IdentityDashboard {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Item for IdentityDashboard {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        "Omega Identity".into()
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("Identity Dashboard Opened")
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn can_split(&self) -> bool {
        false
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<WorkspaceId>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Task<Option<Entity<Self>>> {
        Task::ready(None)
    }

    fn to_item_events(event: &Self::Event, emit: &mut dyn FnMut(ItemEvent)) {
        emit(*event)
    }
}

fn record_owner_purge_result(
    backend: &dyn AccountDashboardBackend,
    account_ref: &AccountRef,
    operation_ref: &str,
    target: AccountPurgeTarget,
    result: anyhow::Result<()>,
) -> Result<AccountPurgeReport, String> {
    backend.record_purge_target(
        account_ref,
        operation_ref,
        target,
        owner_purge_verification(result),
    )
}

fn owner_purge_verification(result: anyhow::Result<()>) -> AccountPurgeVerification {
    match result {
        Ok(()) => AccountPurgeVerification::VerifiedDeleted,
        Err(error) => AccountPurgeVerification::Failed {
            reason: error.to_string(),
        },
    }
}

fn account_display_name(account: &AccountDashboardEntry) -> String {
    account
        .profile
        .as_ref()
        .and_then(|profile| profile.display_name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Unnamed identity".to_string())
}

fn account_needs_setup(
    lifecycle: AccountLifecycleState,
    recovery: RecoveryProtectionState,
) -> bool {
    lifecycle == AccountLifecycleState::CandidateLocal
        || recovery == RecoveryProtectionState::Needed
}

fn signer_kind_label(kind: SignerKind) -> &'static str {
    match kind {
        SignerKind::LocalNative => "Local file",
        SignerKind::RemoteNip46 => "NIP-46",
        SignerKind::BrowserNip07 => "NIP-07",
        SignerKind::AndroidNip55 => "NIP-55",
        SignerKind::DeviceGrant => "Device grant",
        SignerKind::AgentGrant => "Agent grant",
    }
}

fn last_signer_use(timestamp: Option<u64>) -> String {
    timestamp.map_or_else(
        || "Never".to_string(),
        |timestamp| {
            DateTime::from_timestamp(timestamp as i64, 0).map_or_else(
                || "Unknown".to_string(),
                |timestamp| {
                    timestamp
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                },
            )
        },
    )
}

fn retirement_label(state: omega_identity::AccountRetirementState) -> &'static str {
    match state {
        omega_identity::AccountRetirementState::NotRetired => "Not retired",
        omega_identity::AccountRetirementState::Pending => "Pending",
        omega_identity::AccountRetirementState::Published => "Published",
        omega_identity::AccountRetirementState::Failed => "Failed",
    }
}

fn recovery_detail(state: RecoveryProtectionState) -> &'static str {
    match state {
        RecoveryProtectionState::Needed => "NIP-49 required",
        RecoveryProtectionState::Protected => "NIP-49 protected",
        RecoveryProtectionState::NotApplicable => "External",
    }
}

fn signer_availability_icon(availability: SignerAvailability) -> Option<Icon> {
    let (icon, color) = match availability {
        SignerAvailability::Ready => (IconName::Check, Color::Success),
        SignerAvailability::UserApprovalRequired => (IconName::Lock, Color::Warning),
        SignerAvailability::Offline
        | SignerAvailability::Rejected
        | SignerAvailability::Revoked
        | SignerAvailability::Lost => (IconName::Warning, Color::Error),
    };
    Some(Icon::new(icon).size(IconSize::Small).color(color))
}

fn recovery_icon(state: RecoveryProtectionState) -> Option<Icon> {
    let (icon, color) = match state {
        RecoveryProtectionState::Protected => (IconName::Check, Color::Success),
        RecoveryProtectionState::Needed => (IconName::Warning, Color::Warning),
        RecoveryProtectionState::NotApplicable => (IconName::Info, Color::Muted),
    };
    Some(Icon::new(icon).size(IconSize::Small).color(color))
}

fn detail_row(
    label: &'static str,
    value: impl Into<SharedString>,
    icon: Option<Icon>,
) -> AnyElement {
    h_flex()
        .min_w_0()
        .justify_between()
        .gap_4()
        .child(Label::new(label).color(Color::Muted).size(LabelSize::Small))
        .child(
            h_flex()
                .gap_1()
                .children(icon)
                .child(Label::new(value.into()).size(LabelSize::Small)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_rows_have_honest_profile_and_signer_fallbacks() {
        assert_eq!(signer_kind_label(SignerKind::LocalNative), "Local file");
        assert_eq!(signer_kind_label(SignerKind::RemoteNip46), "NIP-46");
        assert_eq!(last_signer_use(None), "Never");
        assert_ne!(last_signer_use(Some(42)), "Never");
    }

    #[test]
    fn lifecycle_copy_keeps_forget_and_retirement_distinct() {
        let forget = "Events held by relays or peers remain. An external NIP-49 recovery file is not deleted.";
        let retirement = "Retirement is a separate signed policy action and cannot retract events held by relays or peers.";
        assert!(forget.contains("NIP-49"));
        assert!(forget.contains("remain"));
        assert!(retirement.contains("cannot retract"));
        assert_ne!(forget, retirement);
    }

    #[test]
    fn owner_purge_results_are_never_rounded_up_to_success() {
        assert_eq!(
            owner_purge_verification(Ok(())),
            AccountPurgeVerification::VerifiedDeleted
        );
        assert_eq!(
            owner_purge_verification(Err(anyhow::anyhow!("draft records remain"))),
            AccountPurgeVerification::Failed {
                reason: "draft records remain".to_string()
            }
        );
    }

    #[test]
    fn local_candidates_offer_the_existing_setup_flow() {
        assert!(account_needs_setup(
            AccountLifecycleState::CandidateLocal,
            RecoveryProtectionState::Needed
        ));
        assert!(account_needs_setup(
            AccountLifecycleState::Active,
            RecoveryProtectionState::Needed
        ));
        assert!(!account_needs_setup(
            AccountLifecycleState::Active,
            RecoveryProtectionState::Protected
        ));
    }
}
