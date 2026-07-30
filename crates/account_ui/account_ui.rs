use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Local};
use gpui::{
    Action, AnyElement, App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, PromptLevel, Render, SharedString, Task, Window,
};
use omega_actions::{OpenIdentityDashboard, OpenOnboarding, OpenRemoteSignerSetup};
use omega_effectd::{BindingProjection, BindingState, HostedSessionProjection, HostedSessionState};
use omega_identity::{
    AccountDashboardEntry, AccountDashboardProjection, AccountLifecycleState, AccountPurgeReport,
    AccountPurgeTarget, AccountPurgeVerification, AccountRef, AccountRegistryService,
    Nip46CapabilityMethod, Nip46ConnectionInput, Nip46InboundEvent, Nip46PairingFence,
    Nip46PairingSession, Nip46PairingUri, Nip46PermissionPreview, Nip46ReportedSigner,
    Nip46Service, PublicIdentity, ReceiptRef, RecoveryProtectionState,
    RelayAuthenticationProjection, RelayAuthenticationReceipt, RelayAuthenticationRefusal,
    RelayConnectionAuthenticationState, SignerAvailability, SignerKind,
};
use omega_signer_broker::{Nip46RelayCoordinator, Nip46RelayError};
use onboarding::secure_input::SecureInput;
use ui::{Divider, ListItem, ListItemSpacing, SpinnerLabel, TintColor, Tooltip, prelude::*};
use util::ResultExt as _;
use workspace::{
    WorkspaceId,
    item::{Item, ItemEvent},
    with_active_or_new_workspace,
};
use zeroize::Zeroizing;

const COMPACT_WIDTH: f32 = 720.;
const NIP46_PAIRING_RELAY: &str = "wss://relay.openagents.com";
const NIP46_FIRST_WAVE_LIFETIME_SECONDS: u64 = 60 * 60 * 24 * 7;
const NIP46_EXCHANGE_TIMEOUT_SECONDS: u64 = 30;
const SIGN_OUT_LABEL: &str = "Sign out";
const DISCONNECT_SIGNER_LABEL: &str = "Disconnect signer";

fn unix_time_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

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
    fn preview_bunker(
        &self,
        connection_uri: Zeroizing<String>,
        now: u64,
    ) -> Result<RemoteSignerProposal, String>;
    fn preview_nostrconnect(&self, now: u64) -> Result<RemoteSignerProposal, String>;
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
            AccountOperation::DisconnectRemote(account_ref) => (
                self.service
                    .disconnect_remote_signer(&account_ref, expected_generation)
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

    fn preview_bunker(
        &self,
        connection_uri: Zeroizing<String>,
        now: u64,
    ) -> Result<RemoteSignerProposal, String> {
        let input = Nip46ConnectionInput::parse(connection_uri.as_str())
            .map_err(|error| error.to_string())?;
        let generation = self
            .service
            .inspect()
            .map_err(|error| error.to_string())?
            .active
            .generation;
        let preview = Nip46PermissionPreview::omega_first_profile(
            Some(input.public_key().clone()),
            input.relays().to_vec(),
            now,
            now.saturating_add(NIP46_FIRST_WAVE_LIFETIME_SECONDS),
        )
        .map_err(|error| error.to_string())?;
        Ok(RemoteSignerProposal::Bunker {
            input,
            preview,
            fence: Nip46PairingFence::new(generation).map_err(|error| error.to_string())?,
        })
    }

    fn preview_nostrconnect(&self, now: u64) -> Result<RemoteSignerProposal, String> {
        let generation = self
            .service
            .inspect()
            .map_err(|error| error.to_string())?
            .active
            .generation;
        let preview = Nip46PermissionPreview::omega_first_profile(
            None,
            vec![NIP46_PAIRING_RELAY.to_string()],
            now,
            now.saturating_add(NIP46_FIRST_WAVE_LIFETIME_SECONDS),
        )
        .map_err(|error| error.to_string())?;
        Ok(RemoteSignerProposal::NostrConnect {
            preview,
            fence: Nip46PairingFence::new(generation).map_err(|error| error.to_string())?,
        })
    }
}

enum RemoteSignerProposal {
    Bunker {
        input: Nip46ConnectionInput,
        preview: Nip46PermissionPreview,
        fence: Nip46PairingFence,
    },
    NostrConnect {
        preview: Nip46PermissionPreview,
        fence: Nip46PairingFence,
    },
}

impl RemoteSignerProposal {
    fn preview(&self) -> &Nip46PermissionPreview {
        match self {
            Self::Bunker { preview, .. } | Self::NostrConnect { preview, .. } => preview,
        }
    }
}

struct RemotePairingProgress {
    capability_ref: String,
    pairing_uri: Option<Nip46PairingUri>,
}

struct RemoteFinalApproval {
    capability_ref: String,
    reported_signer: Nip46ReportedSigner,
    registry_generation: u64,
}

async fn exchange_bunker_pairing(
    mut session: Nip46PairingSession,
    registry_generation: u64,
) -> Result<Nip46ReportedSigner, Nip46RelayError> {
    let coordinator = Nip46RelayCoordinator::default();
    let client_public_key = session.client_public_key().clone();
    let expected_signer = session.remote_signer_public_key().cloned();
    let connect = session
        .approve(unix_time_seconds(), NIP46_EXCHANGE_TIMEOUT_SECONDS)
        .map_err(Nip46RelayError::Protocol)?;
    let get_public_key = coordinator
        .exchange(
            &connect,
            expected_signer.as_ref(),
            &client_public_key,
            |relay_url, event_json, received_at| {
                session
                    .receive_acknowledgement(
                        registry_generation,
                        Nip46InboundEvent {
                            relay_url,
                            event_json,
                            received_at,
                        },
                        NIP46_EXCHANGE_TIMEOUT_SECONDS,
                    )
                    .map(Some)
            },
        )
        .await?;
    let expected_signer = session.remote_signer_public_key().cloned();
    coordinator
        .exchange(
            &get_public_key,
            expected_signer.as_ref(),
            &client_public_key,
            |relay_url, event_json, received_at| {
                session
                    .receive_user_public_key(
                        registry_generation,
                        Nip46InboundEvent {
                            relay_url,
                            event_json,
                            received_at,
                        },
                        NIP46_EXCHANGE_TIMEOUT_SECONDS,
                    )
                    .map(Some)
            },
        )
        .await
}

async fn exchange_nostrconnect_pairing(
    mut session: Nip46PairingSession,
    registry_generation: u64,
) -> Result<Nip46ReportedSigner, Nip46RelayError> {
    let coordinator = Nip46RelayCoordinator::default();
    let client_public_key = session.client_public_key().clone();
    let relay_urls = session.preview().relays.clone();
    let capability_ref = session.capability_ref().to_string();
    let get_public_key = coordinator
        .listen(
            &relay_urls,
            &capability_ref,
            None,
            &client_public_key,
            |relay_url, event_json, received_at| {
                session
                    .receive_nostrconnect_acknowledgement(
                        registry_generation,
                        Nip46InboundEvent {
                            relay_url,
                            event_json,
                            received_at,
                        },
                        NIP46_EXCHANGE_TIMEOUT_SECONDS,
                    )
                    .map(Some)
            },
        )
        .await?;
    let expected_signer = session.remote_signer_public_key().cloned();
    coordinator
        .exchange(
            &get_public_key,
            expected_signer.as_ref(),
            &client_public_key,
            |relay_url, event_json, received_at| {
                session
                    .receive_user_public_key(
                        registry_generation,
                        Nip46InboundEvent {
                            relay_url,
                            event_json,
                            received_at,
                        },
                        NIP46_EXCHANGE_TIMEOUT_SECONDS,
                    )
                    .map(Some)
            },
        )
        .await
}

async fn exchange_final_approval(
    capability_ref: String,
    registry_generation: u64,
) -> Result<AccountDashboardProjection, String> {
    let service = Nip46Service::system(*app_identity::CHANNEL);
    let mut session = service
        .resume(&capability_ref)
        .map_err(|error| error.to_string())?;
    let challenge = session
        .approve_reported_signer(unix_time_seconds(), NIP46_EXCHANGE_TIMEOUT_SECONDS)
        .map_err(|error| error.to_string())?;
    let client_public_key = session.client_public_key().clone();
    let expected_signer = session.remote_signer_public_key().cloned();
    Nip46RelayCoordinator::default()
        .exchange(
            &challenge,
            expected_signer.as_ref(),
            &client_public_key,
            |relay_url, event_json, received_at| {
                session
                    .receive_signed_challenge(
                        registry_generation,
                        Nip46InboundEvent {
                            relay_url,
                            event_json,
                            received_at,
                        },
                    )
                    .map(Some)
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    AccountRegistryService::system(*app_identity::CHANNEL)
        .register_remote_account(&capability_ref, registry_generation)
        .map_err(|error| error.to_string())
}

fn remote_pairing_failure_message(error: &Nip46RelayError) -> &'static str {
    match error {
        Nip46RelayError::Offline | Nip46RelayError::Silence => {
            "The signer is offline or did not respond. Start the connection again when it is available."
        }
        Nip46RelayError::Timeout => {
            "The signer did not finish in time. Start the connection again to retry."
        }
        Nip46RelayError::Protocol(omega_identity::Nip46Error::Rejected) => {
            "The signer rejected this connection."
        }
        Nip46RelayError::Protocol(omega_identity::Nip46Error::Revoked) => {
            "This remote signer capability was revoked."
        }
        _ => "The signer response could not be verified. No account was connected.",
    }
}

#[derive(Clone)]
enum AccountOperation {
    AddLocal(ReceiptRef),
    Switch(AccountRef),
    Lock,
    Unlock,
    SignOut,
    DisconnectRemote(AccountRef),
}

#[derive(Clone, Copy)]
enum HostedOperation {
    Connect,
    Verify,
    Disconnect,
}

struct AccountOperationResult {
    projection: AccountDashboardProjection,
    purge_report: Option<AccountPurgeReport>,
}

pub fn init(cx: &mut App) {
    cx.on_action(|_: &OpenIdentityDashboard, cx| open_identity_dashboard(false, cx));
    cx.on_action(|_: &OpenRemoteSignerSetup, cx| open_identity_dashboard(true, cx));
}

fn open_identity_dashboard(open_remote_setup: bool, cx: &mut App) {
    with_active_or_new_workspace(cx, move |workspace, window, cx| {
        workspace
            .with_local_workspace(window, cx, move |workspace, window, cx| {
                let existing = workspace
                    .active_pane()
                    .read(cx)
                    .items()
                    .find_map(|item| item.downcast::<IdentityDashboard>());
                if let Some(existing) = existing {
                    if open_remote_setup {
                        existing.update(cx, |dashboard, cx| {
                            dashboard.remote_setup_open = true;
                            cx.notify();
                        });
                    }
                    workspace.activate_item(&existing, true, true, window, cx);
                } else {
                    let dashboard = IdentityDashboard::new(open_remote_setup, window, cx);
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
    remote_setup_open: bool,
    remote_uri_input: Entity<SecureInput>,
    remote_proposal: Option<RemoteSignerProposal>,
    remote_pairing: Option<RemotePairingProgress>,
    remote_final_approval: Option<RemoteFinalApproval>,
    relay_authentication: RelayAuthenticationProjection,
    hosted_session: HostedSessionProjection,
    hosted_binding: BindingProjection,
}

impl IdentityDashboard {
    fn new(open_remote_setup: bool, window: &mut Window, cx: &mut App) -> Entity<Self> {
        Self::new_with_backend(
            Arc::new(SystemAccountDashboardBackend::new()),
            open_remote_setup,
            window,
            cx,
        )
    }

    fn new_with_backend(
        backend: Arc<dyn AccountDashboardBackend>,
        open_remote_setup: bool,
        _window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let hosted_session = omega_effectd::openagents_session_if_initialized(cx)
            .map_or_else(HostedSessionProjection::default, |session| {
                session.projection()
            });
        let hosted_binding = omega_effectd::try_openagents_binding(cx)
            .map_or_else(BindingProjection::unbound, |binding| {
                binding.load_projection()
            });
        let remote_uri_input =
            cx.new(|cx| SecureInput::new("Paste bunker:// connection", "Bunker connection", 1, cx));
        let dashboard = cx.new(|cx| Self {
            backend,
            projection: None,
            selected_account: None,
            focus_handle: cx.focus_handle(),
            task: None,
            message: None,
            purge_report: None,
            busy: false,
            remote_setup_open: open_remote_setup,
            remote_uri_input,
            remote_proposal: None,
            remote_pairing: None,
            remote_final_approval: None,
            relay_authentication: RelayAuthenticationProjection::default(),
            hosted_session,
            hosted_binding,
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
                    Ok(projection) => {
                        this.apply_projection(projection);
                        this.relay_authentication =
                            omega_effectd::relay_authentication_projection();
                        this.hosted_session = omega_effectd::openagents_session_if_initialized(cx)
                            .map_or_else(HostedSessionProjection::default, |session| {
                                session.projection()
                            });
                        this.hosted_binding = omega_effectd::try_openagents_binding(cx)
                            .map_or_else(BindingProjection::unbound, |binding| {
                                binding.load_projection()
                            });
                    }
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

    fn run_hosted_operation(&mut self, operation: HostedOperation, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(session) = omega_effectd::openagents_session_if_initialized(cx) else {
            self.hosted_session = HostedSessionProjection {
                state: HostedSessionState::ServiceUnavailable,
                retryable: true,
                ..HostedSessionProjection::default()
            };
            self.message = Some("The hosted account service is unavailable.".into());
            cx.notify();
            return;
        };
        self.busy = true;
        self.message = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            match operation {
                HostedOperation::Connect => {
                    session.connect(cx).await;
                }
                HostedOperation::Verify => {
                    session.verify_public(cx).await;
                }
                HostedOperation::Disconnect => {
                    session.disconnect(cx).await;
                }
            }
            let projection = session.projection();
            this.update(cx, |this, cx| {
                this.hosted_session = projection;
                this.hosted_binding = omega_effectd::try_openagents_binding(cx)
                    .map_or_else(BindingProjection::unbound, |binding| {
                        binding.load_projection()
                    });
                this.message =
                    hosted_operation_message(operation, &this.hosted_session).map(Into::into);
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

    fn review_bunker_permissions(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let connection_uri = Zeroizing::new(self.remote_uri_input.update(cx, SecureInput::take));
        let backend = self.backend.clone();
        self.busy = true;
        self.message = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    backend.preview_bunker(connection_uri, unix_time_seconds())
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(proposal) => this.remote_proposal = Some(proposal),
                    Err(error) => {
                        zlog::error!("NIP-46 bunker preview failed: {error}");
                        this.message = Some("That bunker connection could not be verified.".into());
                    }
                }
                this.busy = false;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn review_nostrconnect_permissions(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let backend = self.backend.clone();
        self.busy = true;
        self.message = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { backend.preview_nostrconnect(unix_time_seconds()) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(proposal) => this.remote_proposal = Some(proposal),
                    Err(error) => {
                        zlog::error!("NIP-46 pairing preview failed: {error}");
                        this.message =
                            Some("Remote signer permissions could not be prepared.".into());
                    }
                }
                this.busy = false;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn approve_remote_proposal(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(proposal) = self.remote_proposal.take() else {
            return;
        };
        let service = Nip46Service::system(*app_identity::CHANNEL);
        let (session, pairing_uri, registry_generation, is_nostrconnect) = match proposal {
            RemoteSignerProposal::Bunker {
                input,
                preview,
                fence,
            } => {
                let registry_generation = fence.registry_generation;
                match service.begin_bunker_pairing(input, preview, fence) {
                    Ok(session) => (session, None, registry_generation, false),
                    Err(error) => {
                        zlog::error!("NIP-46 bunker pairing could not start: {error}");
                        self.message = Some("The remote connection could not be started.".into());
                        cx.notify();
                        return;
                    }
                }
            }
            RemoteSignerProposal::NostrConnect { preview, fence } => {
                let registry_generation = fence.registry_generation;
                match service.create_nostrconnect_pairing(preview, fence, "Omega") {
                    Ok((session, uri)) => (session, Some(uri), registry_generation, true),
                    Err(error) => {
                        zlog::error!("NIP-46 pairing link could not be created: {error}");
                        self.message = Some("The pairing link could not be created.".into());
                        cx.notify();
                        return;
                    }
                }
            }
        };
        let capability_ref = session.capability_ref().to_string();
        self.remote_pairing = Some(RemotePairingProgress {
            capability_ref: capability_ref.clone(),
            pairing_uri,
        });
        self.busy = true;
        self.message = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if is_nostrconnect {
                        exchange_nostrconnect_pairing(session, registry_generation).await
                    } else {
                        exchange_bunker_pairing(session, registry_generation).await
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(reported_signer) => {
                        this.remote_pairing = None;
                        this.remote_final_approval = Some(RemoteFinalApproval {
                            capability_ref,
                            reported_signer,
                            registry_generation,
                        });
                        this.message = None;
                    }
                    Err(error) => {
                        zlog::error!("NIP-46 pairing exchange failed: {error}");
                        this.remote_pairing = None;
                        this.message = Some(remote_pairing_failure_message(&error).into());
                    }
                }
                this.busy = false;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn copy_pairing_link(&self, cx: &mut Context<Self>) {
        let Some(uri) = self
            .remote_pairing
            .as_ref()
            .and_then(|progress| progress.pairing_uri.as_ref())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(uri.expose().to_string()));
    }

    fn open_pairing_link(&self, cx: &mut Context<Self>) {
        let Some(uri) = self
            .remote_pairing
            .as_ref()
            .and_then(|progress| progress.pairing_uri.as_ref())
        else {
            return;
        };
        cx.open_url(uri.expose());
    }

    fn reject_reported_signer(&mut self, cx: &mut Context<Self>) {
        let Some(approval) = self.remote_final_approval.take() else {
            return;
        };
        let service = Nip46Service::system(*app_identity::CHANNEL);
        match service
            .resume(&approval.capability_ref)
            .and_then(|mut session| session.reject())
        {
            Ok(()) => self.message = Some("Remote signer connection rejected.".into()),
            Err(error) => {
                zlog::error!("NIP-46 final approval rejection failed: {error}");
                self.message = Some("The connection could not be rejected cleanly.".into());
            }
        }
        cx.notify();
    }

    fn approve_reported_signer(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(approval) = self.remote_final_approval.take() else {
            return;
        };
        self.busy = true;
        self.message = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(exchange_final_approval(
                    approval.capability_ref,
                    approval.registry_generation,
                ))
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(projection) => {
                        this.apply_projection(projection);
                        this.remote_setup_open = false;
                        this.message = Some("Remote signer connected.".into());
                    }
                    Err(error) => {
                        zlog::error!("NIP-46 final approval failed: {error}");
                        this.message = Some(
                            "The signer proof could not be verified. No account was connected."
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

    fn close_remote_setup(&mut self, cx: &mut Context<Self>) {
        self.remote_uri_input.update(cx, SecureInput::clear);
        self.remote_proposal = None;
        self.remote_pairing = None;
        self.remote_final_approval = None;
        self.remote_setup_open = false;
        self.message = None;
        cx.notify();
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
            && account_switch_recovery_available(account.signer.kind, account.recovery);
        let is_active = account.is_active;
        let is_locked = is_active && account.lifecycle == AccountLifecycleState::Locked;
        let is_remote = account.signer.kind == SignerKind::RemoteNip46;
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
                format!(
                    "{} · {}",
                    signer_kind_label(account.signer.kind),
                    signer_availability_label(account.signer.availability)
                ),
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
            .child(self.render_authentication_authority(cx))
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
                        Button::new("omega-account-sign-out", SIGN_OUT_LABEL)
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(!is_active || busy)
                            .tooltip(Tooltip::text("Sign out"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_operation(AccountOperation::SignOut, cx)
                            })),
                    )
                    .when(is_remote, |this| {
                        this.child(
                            Button::new("omega-account-disconnect-signer", DISCONNECT_SIGNER_LABEL)
                                .style(ButtonStyle::Tinted(TintColor::Error))
                                .disabled(busy)
                                .tooltip(Tooltip::text(
                                    "Revoke this capability and delete its local client key",
                                ))
                                .on_click(cx.listener({
                                    let account_ref = account_ref.clone();
                                    move |this, _, _, cx| {
                                        this.run_operation(
                                            AccountOperation::DisconnectRemote(account_ref.clone()),
                                            cx,
                                        )
                                    }
                                })),
                        )
                    }),
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

    fn render_authentication_authority(&self, cx: &mut Context<Self>) -> AnyElement {
        let hosted_binding_matches_selected = self.hosted_session_binding_matches_selected();
        let public_binding_matches_selected = self.public_hosted_binding_matches_selected();
        let public_binding_state = if public_binding_matches_selected {
            self.hosted_binding.state
        } else {
            BindingState::Unbound
        };
        let hosted_has_no_binding = self.hosted_session.omega_public_key_hex.is_none()
            && self.hosted_session.account_generation.is_none();
        let hosted_state = if hosted_binding_matches_selected || hosted_has_no_binding {
            self.hosted_session.state
        } else {
            HostedSessionState::AccountMismatch
        };
        let relay_projection =
            self.selected_entry()
                .map_or_else(RelayAuthenticationProjection::default, |account| {
                    self.relay_authentication
                        .for_account_public_key_hex(account.identity.public_key_hex().as_str())
                });
        let relay_rows = relay_projection
            .relays
            .iter()
            .map(|receipt| {
                v_flex()
                    .gap_1()
                    .child(detail_row(
                        "Relay authenticated",
                        relay_authentication_detail(receipt),
                        relay_authentication_icon(receipt.state),
                    ))
                    .child(
                        Label::new(format!(
                            "{} · connection {}",
                            receipt.relay_url, receipt.connection_generation
                        ))
                        .color(Color::Muted)
                        .size(LabelSize::XSmall),
                    )
            })
            .collect::<Vec<_>>();

        v_flex()
            .gap_2()
            .child(Label::new("Authentication").size(LabelSize::Small))
            .child(detail_row(
                "Signer ready",
                self.selected_entry().map_or("Unavailable", |account| {
                    signer_availability_label(account.signer.availability)
                }),
                self.selected_entry()
                    .and_then(|account| signer_availability_icon(account.signer.availability)),
            ))
            .when(relay_rows.is_empty(), |this| {
                this.child(detail_row("Relay authenticated", "No relay receipt", None))
            })
            .children(relay_rows)
            .child(detail_row("Group admitted", "Not established", None))
            .child(detail_row(
                "Hosted linked",
                hosted_binding_label(public_binding_state),
                hosted_binding_icon(public_binding_state),
            ))
            .when(
                public_binding_matches_selected
                    && self.hosted_binding.openagents_account_id.is_some(),
                |this| {
                    this.child(detail_row(
                        "Hosted user",
                        self.hosted_binding
                            .openagents_account_id
                            .clone()
                            .unwrap_or_default(),
                        None,
                    ))
                },
            )
            .child(detail_row(
                "Hosted session",
                hosted_session_label(hosted_state),
                hosted_session_icon(hosted_state),
            ))
            .when(
                hosted_binding_matches_selected && self.hosted_session.expires_at.is_some(),
                |this| {
                    this.child(detail_row(
                        "Hosted expiry",
                        last_signer_use(self.hosted_session.expires_at),
                        None,
                    ))
                },
            )
            .child(detail_row("Action authorized", "Per action", None))
            .child(self.render_hosted_controls(cx, hosted_state))
            .into_any_element()
    }

    fn hosted_session_binding_matches_selected(&self) -> bool {
        let Some(account) = self.selected_entry().filter(|account| account.is_active) else {
            return false;
        };
        let Some(active_generation) = self
            .projection
            .as_ref()
            .map(|projection| projection.active.generation)
        else {
            return false;
        };
        hosted_binding_matches(
            &self.hosted_session,
            account.identity.public_key_hex().as_str(),
            active_generation,
        )
    }

    fn public_hosted_binding_matches_selected(&self) -> bool {
        let Some(account) = self.selected_entry() else {
            return false;
        };
        self.hosted_binding.omega_public_key_hex.as_deref()
            == Some(account.identity.public_key_hex().as_str())
    }

    fn render_hosted_controls(
        &self,
        cx: &mut Context<Self>,
        hosted_state: HostedSessionState,
    ) -> AnyElement {
        let is_active = self
            .selected_entry()
            .is_some_and(|account| account.is_active);
        let can_connect = matches!(
            hosted_state,
            HostedSessionState::Disconnected
                | HostedSessionState::Expired
                | HostedSessionState::Revoked
                | HostedSessionState::OwnerScopeRefused
                | HostedSessionState::AccountMismatch
                | HostedSessionState::ServiceUnavailable
                | HostedSessionState::StorageFailed
        );
        let can_verify = matches!(
            hosted_state,
            HostedSessionState::Verified
                | HostedSessionState::Rotating
                | HostedSessionState::AccountMismatch
                | HostedSessionState::ServiceUnavailable
                | HostedSessionState::StorageFailed
        );
        let can_disconnect = !matches!(
            hosted_state,
            HostedSessionState::Disconnected | HostedSessionState::Connecting
        );
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                Button::new("omega-hosted-connect", "Connect hosted")
                    .style(ButtonStyle::OutlinedGhost)
                    .size(ButtonSize::Compact)
                    .disabled(!is_active || self.busy || !can_connect)
                    .tooltip(Tooltip::text("Link"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.run_hosted_operation(HostedOperation::Connect, cx)
                    })),
            )
            .child(
                Button::new("omega-hosted-verify", "Verify or rotate")
                    .style(ButtonStyle::OutlinedGhost)
                    .size(ButtonSize::Compact)
                    .disabled(!is_active || self.busy || !can_verify)
                    .tooltip(Tooltip::text("Verify"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.run_hosted_operation(HostedOperation::Verify, cx)
                    })),
            )
            .child(
                Button::new("omega-hosted-disconnect", "Disconnect hosted")
                    .style(ButtonStyle::Tinted(TintColor::Error))
                    .size(ButtonSize::Compact)
                    .disabled(!is_active || self.busy || !can_disconnect)
                    .tooltip(Tooltip::text("Disconnect"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.run_hosted_operation(HostedOperation::Disconnect, cx)
                    })),
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

    fn render_remote_setup(&self, cx: &mut Context<Self>) -> AnyElement {
        let content = if let Some(approval) = self.remote_final_approval.as_ref() {
            let preview = &approval.reported_signer.preview;
            let methods = preview
                .methods
                .iter()
                .map(|method| nip46_method_label(*method))
                .collect::<Vec<_>>()
                .join(", ");
            let event_kinds = preview
                .event_kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            v_flex()
                .gap_3()
                .child(Label::new("Confirm the signer identity"))
                .child(detail_row(
                    "Nostr account",
                    approval.reported_signer.user_identity.npub().as_str(),
                    None,
                ))
                .child(detail_row(
                    "Signer device",
                    approval
                        .reported_signer
                        .remote_signer_public_key
                        .as_str(),
                    None,
                ))
                .child(detail_row("Methods", methods, None))
                .child(detail_row("Event kinds", event_kinds, None))
                .child(detail_row("Exact relays", preview.relays.join("\n"), None))
                .child(
                    Label::new(
                        "Approve only if these identities match the signer app. Omega will ask it to sign a one-time login proof before activating the account.",
                    )
                    .color(Color::Muted)
                    .size(LabelSize::Small),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("omega-remote-final-approve", "Approve signer")
                                .style(ButtonStyle::Filled)
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.approve_reported_signer(cx)
                                })),
                        )
                        .child(
                            Button::new("omega-remote-final-reject", "Reject")
                                .style(ButtonStyle::OutlinedGhost)
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reject_reported_signer(cx)
                                })),
                        ),
                )
                .into_any_element()
        } else if let Some(progress) = self.remote_pairing.as_ref() {
            v_flex()
                .gap_3()
                .child(Label::new("Waiting for the signer"))
                .when(progress.pairing_uri.is_some(), |this| {
                    this.child(
                        Label::new(
                            "Open or copy the temporary pairing link in your signer app. Treat the link like a password until pairing finishes.",
                        )
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("omega-remote-open-link", "Open pairing link")
                                    .style(ButtonStyle::Filled)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_pairing_link(cx)
                                    })),
                            )
                            .child(
                                Button::new("omega-remote-copy-link", "Copy pairing link")
                                    .style(ButtonStyle::OutlinedGhost)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.copy_pairing_link(cx)
                                    })),
                            ),
                    )
                })
                .child(
                    h_flex()
                        .gap_2()
                        .child(SpinnerLabel::new())
                        .child(Label::new("Waiting for a verified signer response…")),
                )
                .child(
                    Label::new(format!(
                        "Connection {} is pending. Omega will show the reported account and signer identity before final approval.",
                        progress.capability_ref
                    ))
                    .color(Color::Muted)
                    .size(LabelSize::XSmall),
                )
                .into_any_element()
        } else if let Some(proposal) = self.remote_proposal.as_ref() {
            let preview = proposal.preview();
            let expected_signer = preview.expected_signer.as_ref().map_or_else(
                || "Reported by the signer before final approval".to_string(),
                |signer| signer.as_str().to_string(),
            );
            let methods = preview
                .methods
                .iter()
                .map(|method| nip46_method_label(*method))
                .collect::<Vec<_>>()
                .join(", ");
            let event_kinds = preview
                .event_kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let relays = preview.relays.join("\n");
            let expires = last_signer_use(Some(preview.expires_at));

            v_flex()
                .gap_3()
                .child(Label::new("Review remote signer permissions"))
                .child(detail_row("Signer identity", expected_signer, None))
                .child(detail_row("Methods", methods, None))
                .child(detail_row("Event kinds", event_kinds, None))
                .child(detail_row("Exact relays", relays, None))
                .child(detail_row("Capability expires", expires, None))
                .child(detail_row(
                    "Recovery dependency",
                    "Remote signer access is required",
                    Some(
                        Icon::new(IconName::Warning)
                            .size(IconSize::Small)
                            .color(Color::Warning),
                    ),
                ))
                .child(
                    Label::new(
                        "Omega stores only the disposable client capability in an owner-only local file. Bulk decrypt is not included in this permission.",
                    )
                    .color(Color::Muted)
                    .size(LabelSize::Small),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("omega-remote-approve", "Approve connection")
                                .style(ButtonStyle::Filled)
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.approve_remote_proposal(cx)
                                })),
                        )
                        .child(
                            Button::new("omega-remote-review-back", "Back")
                                .style(ButtonStyle::OutlinedGhost)
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.remote_proposal = None;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element()
        } else {
            v_flex()
                .gap_4()
                .child(
                    v_flex()
                        .gap_1()
                        .child(Label::new("Connect a remote signer"))
                        .child(
                            Label::new(
                                "Your root Nostr secret stays in the signer. Omega requests a bounded disposable client capability.",
                            )
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                        ),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(Label::new("Connect to a bunker"))
                        .child(self.remote_uri_input.clone())
                        .child(
                            Button::new("omega-remote-review-bunker", "Review permissions")
                                .style(ButtonStyle::Filled)
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.review_bunker_permissions(cx)
                                })),
                        ),
                )
                .child(Divider::horizontal())
                .child(
                    v_flex()
                        .gap_2()
                        .child(Label::new("Pair with a signer app"))
                        .child(
                            Label::new(
                                "Review the permission profile before Omega creates a one-time nostrconnect pairing link.",
                            )
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                        )
                        .child(
                            Button::new(
                                "omega-remote-review-pairing",
                                "Review pairing permissions",
                            )
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.review_nostrconnect_permissions(cx)
                            })),
                        ),
                )
                .into_any_element()
        };

        v_flex()
            .gap_4()
            .child(content)
            .child(
                Button::new("omega-remote-close", "Cancel")
                    .style(ButtonStyle::OutlinedGhost)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.close_remote_setup(cx))),
            )
            .into_any_element()
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
            .when(self.remote_setup_open, |this| {
                this.child(self.render_remote_setup(cx))
            })
            .when(!self.remote_setup_open && compact, |this| {
                this.child(self.render_account_list(cx))
                    .child(Divider::horizontal())
                    .child(self.render_detail(cx))
            })
            .when(!self.remote_setup_open && !compact, |this| {
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

fn account_switch_recovery_available(
    signer_kind: SignerKind,
    recovery: RecoveryProtectionState,
) -> bool {
    match signer_kind {
        SignerKind::RemoteNip46 => recovery == RecoveryProtectionState::NotApplicable,
        _ => recovery == RecoveryProtectionState::Protected,
    }
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

fn signer_availability_label(availability: SignerAvailability) -> &'static str {
    match availability {
        SignerAvailability::Ready => "Ready",
        SignerAvailability::UserApprovalRequired => "Approval required",
        SignerAvailability::Offline => "Offline",
        SignerAvailability::Rejected => "Rejected",
        SignerAvailability::Revoked => "Revoked",
        SignerAvailability::Lost => "Lost",
    }
}

fn relay_authentication_label(state: RelayConnectionAuthenticationState) -> &'static str {
    match state {
        RelayConnectionAuthenticationState::Disconnected => "Disconnected",
        RelayConnectionAuthenticationState::ChallengePending => "Challenge pending",
        RelayConnectionAuthenticationState::Authenticated => "Authenticated",
        RelayConnectionAuthenticationState::Refused => "Refused",
        RelayConnectionAuthenticationState::Stale => "Stale",
    }
}

fn relay_refusal_label(refusal: RelayAuthenticationRefusal) -> &'static str {
    match refusal {
        RelayAuthenticationRefusal::MalformedChallenge => "Malformed challenge",
        RelayAuthenticationRefusal::InvalidEvent => "Invalid proof",
        RelayAuthenticationRefusal::WrongAccount => "Wrong account",
        RelayAuthenticationRefusal::StaleEvent => "Expired proof",
        RelayAuthenticationRefusal::ReplayedProof => "Reused proof",
        RelayAuthenticationRefusal::RelayRejected => "Relay refused",
        RelayAuthenticationRefusal::StaleConnection => "Old connection",
        RelayAuthenticationRefusal::AcknowledgementMissing => "No acknowledgement",
    }
}

fn relay_authentication_detail(receipt: &RelayAuthenticationReceipt) -> String {
    receipt.refusal.map_or_else(
        || relay_authentication_label(receipt.state).to_string(),
        |refusal| relay_refusal_label(refusal).to_string(),
    )
}

fn relay_authentication_icon(state: RelayConnectionAuthenticationState) -> Option<Icon> {
    let (icon, color) = match state {
        RelayConnectionAuthenticationState::Authenticated => (IconName::Check, Color::Success),
        RelayConnectionAuthenticationState::ChallengePending => (IconName::Lock, Color::Warning),
        RelayConnectionAuthenticationState::Disconnected
        | RelayConnectionAuthenticationState::Refused
        | RelayConnectionAuthenticationState::Stale => (IconName::Warning, Color::Error),
    };
    Some(Icon::new(icon).size(IconSize::Small).color(color))
}

fn hosted_session_label(state: HostedSessionState) -> &'static str {
    match state {
        HostedSessionState::Disconnected => "Disconnected",
        HostedSessionState::Connecting => "Verifying",
        HostedSessionState::Verified => "Verified",
        HostedSessionState::Rotating => "Rotating",
        HostedSessionState::Expired => "Expired",
        HostedSessionState::Revoked => "Revoked",
        HostedSessionState::OwnerScopeRefused => "Owner refused",
        HostedSessionState::AccountMismatch => "Account mismatch",
        HostedSessionState::ServiceUnavailable => "Service unavailable",
        HostedSessionState::StorageFailed => "Storage failed",
        HostedSessionState::RevocationFailed => "Revocation failed",
    }
}

fn hosted_binding_label(state: BindingState) -> &'static str {
    match state {
        BindingState::Unbound => "Unlinked",
        BindingState::Bound => "Linked",
        BindingState::Refused => "Owner refused",
    }
}

fn hosted_binding_icon(state: BindingState) -> Option<Icon> {
    let (icon, color) = match state {
        BindingState::Bound => (IconName::Check, Color::Success),
        BindingState::Unbound => (IconName::Info, Color::Muted),
        BindingState::Refused => (IconName::Warning, Color::Error),
    };
    Some(Icon::new(icon).size(IconSize::Small).color(color))
}

fn hosted_session_icon(state: HostedSessionState) -> Option<Icon> {
    let (icon, color) = match state {
        HostedSessionState::Verified => (IconName::Check, Color::Success),
        HostedSessionState::Connecting | HostedSessionState::Rotating => {
            (IconName::Lock, Color::Warning)
        }
        HostedSessionState::Disconnected => (IconName::Info, Color::Muted),
        HostedSessionState::Expired
        | HostedSessionState::Revoked
        | HostedSessionState::OwnerScopeRefused
        | HostedSessionState::AccountMismatch
        | HostedSessionState::ServiceUnavailable
        | HostedSessionState::StorageFailed
        | HostedSessionState::RevocationFailed => (IconName::Warning, Color::Error),
    };
    Some(Icon::new(icon).size(IconSize::Small).color(color))
}

fn hosted_operation_message(
    operation: HostedOperation,
    projection: &HostedSessionProjection,
) -> Option<&'static str> {
    match projection.state {
        HostedSessionState::Disconnected => Some("Hosted account disconnected."),
        HostedSessionState::Connecting => Some("Hosted account verification is still running."),
        HostedSessionState::Verified => match operation {
            HostedOperation::Connect => Some("Hosted account linked and verified."),
            HostedOperation::Verify => Some("Hosted account verified; tokens rotated if required."),
            HostedOperation::Disconnect => None,
        },
        HostedSessionState::Rotating => Some("Hosted account tokens are rotating."),
        HostedSessionState::Expired => Some("The hosted session expired. Link it again."),
        HostedSessionState::Revoked => Some("Hosted credentials were revoked and removed."),
        HostedSessionState::OwnerScopeRefused => {
            Some("OpenAgents refused this Omega identity for the requested owner.")
        }
        HostedSessionState::AccountMismatch => {
            Some("The hosted session belongs to a different Omega identity.")
        }
        HostedSessionState::ServiceUnavailable => {
            Some("The hosted account service is unavailable. Try again.")
        }
        HostedSessionState::StorageFailed => {
            Some("Hosted credential storage failed. Check local storage and retry.")
        }
        HostedSessionState::RevocationFailed => {
            Some("Hosted revocation failed. Disconnect again to retry.")
        }
    }
}

fn hosted_binding_matches(
    projection: &HostedSessionProjection,
    public_key_hex: &str,
    account_generation: u64,
) -> bool {
    projection.omega_public_key_hex.as_deref() == Some(public_key_hex)
        && projection.account_generation == Some(account_generation)
}

fn nip46_method_label(method: Nip46CapabilityMethod) -> &'static str {
    match method {
        Nip46CapabilityMethod::LoginProof => "Login proof",
        Nip46CapabilityMethod::SignEvent => "Event signing",
        Nip46CapabilityMethod::Nip44Encrypt => "NIP-44 encrypt",
        Nip46CapabilityMethod::Nip44Decrypt => "NIP-44 decrypt",
        Nip46CapabilityMethod::BulkDecrypt => "Bulk decrypt",
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
        assert_eq!(
            signer_availability_label(SignerAvailability::Rejected),
            "Rejected"
        );
        assert_eq!(
            signer_availability_label(SignerAvailability::Offline),
            "Offline"
        );
        assert_eq!(
            signer_availability_label(SignerAvailability::Revoked),
            "Revoked"
        );
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

    #[test]
    fn remote_signer_consent_keeps_operations_distinct() {
        assert_eq!(
            nip46_method_label(Nip46CapabilityMethod::LoginProof),
            "Login proof"
        );
        assert_eq!(
            nip46_method_label(Nip46CapabilityMethod::SignEvent),
            "Event signing"
        );
        assert_eq!(
            nip46_method_label(Nip46CapabilityMethod::Nip44Encrypt),
            "NIP-44 encrypt"
        );
        assert_eq!(
            nip46_method_label(Nip46CapabilityMethod::Nip44Decrypt),
            "NIP-44 decrypt"
        );
        assert_eq!(
            nip46_method_label(Nip46CapabilityMethod::BulkDecrypt),
            "Bulk decrypt"
        );
    }

    #[test]
    fn remote_accounts_can_switch_without_local_recovery_artifacts() {
        assert!(account_switch_recovery_available(
            SignerKind::RemoteNip46,
            RecoveryProtectionState::NotApplicable
        ));
        assert!(!account_switch_recovery_available(
            SignerKind::RemoteNip46,
            RecoveryProtectionState::Protected
        ));
        assert!(account_switch_recovery_available(
            SignerKind::LocalNative,
            RecoveryProtectionState::Protected
        ));
    }

    #[test]
    fn remote_sign_out_and_disconnect_are_distinct_actions() {
        assert_ne!(SIGN_OUT_LABEL, DISCONNECT_SIGNER_LABEL);
        assert!(matches!(
            AccountOperation::SignOut,
            AccountOperation::SignOut
        ));
        let account_ref = AccountRef::new("remote-account").expect("account ref");
        assert!(matches!(
            AccountOperation::DisconnectRemote(account_ref),
            AccountOperation::DisconnectRemote(_)
        ));
    }

    #[test]
    fn authentication_copy_does_not_collapse_authority_domains() {
        let domains = [
            "Signer ready",
            "Relay authenticated",
            "Group admitted",
            "Hosted linked",
            "Action authorized",
        ];
        assert_eq!(
            domains
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            domains.len()
        );
        assert_eq!(
            relay_authentication_label(RelayConnectionAuthenticationState::Authenticated),
            "Authenticated"
        );
        assert_eq!(
            relay_refusal_label(RelayAuthenticationRefusal::AcknowledgementMissing),
            "No acknowledgement"
        );
    }

    #[test]
    fn hosted_projection_must_match_selected_identity_and_generation() {
        let projection = HostedSessionProjection {
            state: HostedSessionState::Verified,
            owner_user_id: Some("openagents-user".to_string()),
            omega_public_key_hex: Some("omega-public-key".to_string()),
            account_generation: Some(9),
            issued_at: Some(100),
            expires_at: Some(200),
            retryable: false,
        };
        assert!(hosted_binding_matches(&projection, "omega-public-key", 9));
        assert!(!hosted_binding_matches(
            &projection,
            "different-public-key",
            9
        ));
        assert!(!hosted_binding_matches(&projection, "omega-public-key", 10));
        assert_eq!(
            hosted_session_label(HostedSessionState::AccountMismatch),
            "Account mismatch"
        );
    }

    #[test]
    fn hosted_failures_name_retry_actions() {
        for state in [
            HostedSessionState::OwnerScopeRefused,
            HostedSessionState::ServiceUnavailable,
            HostedSessionState::StorageFailed,
            HostedSessionState::RevocationFailed,
        ] {
            let projection = HostedSessionProjection {
                state,
                retryable: true,
                ..HostedSessionProjection::default()
            };
            assert!(
                hosted_operation_message(HostedOperation::Verify, &projection)
                    .is_some_and(|message| !message.is_empty())
            );
        }
    }
}
