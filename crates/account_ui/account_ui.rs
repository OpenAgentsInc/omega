use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use agent_ui::omega_nostr_profile_transport::{
    ProfileChoice, ProfileChoiceOutcome, ProfilePublishError, apply_profile_choice,
};
use chrono::{DateTime, Local};
use editor::Editor;
use gpui::{
    Action, AnyElement, App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, PromptLevel, Render, SharedString, Task, Window,
};
use omega_actions::{OpenIdentityDashboard, OpenOnboarding, OpenRemoteSignerSetup};
use omega_effectd::{BindingProjection, BindingState, HostedSessionProjection, HostedSessionState};
use omega_identity::{
    AccountDashboardEntry, AccountDashboardProjection, AccountLifecycleState,
    AccountProfileSummary, AccountPurgeReport, AccountPurgeTarget, AccountPurgeVerification,
    AccountRef, AccountRegistryService, IdentityService, Nip46CapabilityMethod,
    Nip46ConnectionInput, Nip46InboundEvent, Nip46PairingFence, Nip46PairingSession,
    Nip46PairingUri, Nip46PermissionPreview, Nip46ReportedSigner, Nip46Service, PublicIdentity,
    ReceiptRef, RecoveryProtectionState, RelayAuthenticationProjection, RelayAuthenticationReceipt,
    RelayAuthenticationRefusal, RelayConnectionAuthenticationState, SignerAvailability, SignerKind,
};
use omega_identity_sync::{
    BulkDecryptConsentState, CacheFallbackReason, HydrationAccountFence, HydrationCache,
    HydrationCacheArea, HydrationReceipt, HydrationSource, HydrationSourceOutcome, HydrationState,
    HydrationTrigger, LocalProfileState, PlaintextPersistencePolicy, TimeoutScope,
};
use omega_signer_broker::{
    Nip46RelayCoordinator, Nip46RelayError, Nip46WebSocketTransport, RemoteSignerMetadata,
    SignerBroker, SignerRoute,
};
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
const PROFILE_PUBLISH_RELAY: &str = "wss://relay.openagents.com";
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
    fn inspect_identity_sync(&self) -> Result<IdentitySyncDashboardState, String>;
    fn record_profile_skipped(&self) -> Result<IdentitySyncDashboardState, String>;
    fn save_local_profile(
        &self,
        profile: serde_json::Value,
    ) -> Result<IdentitySyncDashboardState, String>;
    fn set_bulk_decrypt_consent(
        &self,
        state: BulkDecryptConsentState,
    ) -> Result<IdentitySyncDashboardState, String>;
    fn set_plaintext_policy(
        &self,
        policy: PlaintextPersistencePolicy,
    ) -> Result<IdentitySyncDashboardState, String>;
    fn start_identity_hydration(&self, trigger: HydrationTrigger) -> Result<(), String>;
}

struct SystemAccountDashboardBackend {
    service: AccountRegistryService,
}

#[derive(Clone, Debug, Default)]
struct IdentitySyncDashboardState {
    projection: Option<AccountDashboardProjection>,
    hydration_receipt: Option<HydrationReceipt>,
    profile_state: Option<LocalProfileState>,
    remote_signer: bool,
    bulk_decrypt_consent: BulkDecryptConsentState,
    bulk_decrypt_permission_available: bool,
    plaintext_policy: PlaintextPersistencePolicy,
}

fn active_hydration_cache(
    service: &AccountRegistryService,
) -> Result<(HydrationCache, Option<String>), String> {
    let selection = service
        .selection_token()
        .map_err(|error| error.to_string())?;
    let fence = HydrationAccountFence::new(
        selection.account_ref.clone(),
        selection.identity.public_key_hex().clone(),
        selection.generation,
    )
    .map_err(|error| error.to_string())?;
    let capability_ref = service
        .remote_signer_capability(&selection)
        .ok()
        .map(|capability| capability.capability_ref);
    let cache = HydrationCache::system(fence).map_err(|error| error.to_string())?;
    Ok((cache, capability_ref))
}

fn inspect_identity_sync(
    service: &AccountRegistryService,
) -> Result<IdentitySyncDashboardState, String> {
    let selection = match service.selection_token() {
        Ok(selection) => selection,
        Err(_) => return Ok(IdentitySyncDashboardState::default()),
    };
    let fence = HydrationAccountFence::new(
        selection.account_ref.clone(),
        selection.identity.public_key_hex().clone(),
        selection.generation,
    )
    .map_err(|error| error.to_string())?;
    let dashboard = service.inspect().map_err(|error| error.to_string())?;
    let remote_signer = dashboard
        .accounts
        .iter()
        .find(|account| account.is_active)
        .is_some_and(|account| account.signer.kind == SignerKind::RemoteNip46);
    let capability = remote_signer
        .then(|| service.remote_signer_capability(&selection).ok())
        .flatten();
    let capability_ref = capability
        .as_ref()
        .map(|capability| capability.capability_ref.clone());
    let permission_available = capability.as_ref().is_some_and(|capability| {
        capability
            .methods
            .contains(&Nip46CapabilityMethod::BulkDecrypt)
    });
    let cache = HydrationCache::system(fence).map_err(|error| error.to_string())?;
    let account_state = cache
        .inspect_account_state(capability_ref.as_deref().unwrap_or("local-native"))
        .map_err(|error| error.to_string())?;
    let profile_state = cache
        .read_local_profile_state()
        .map_err(|error| error.to_string())?;
    Ok(IdentitySyncDashboardState {
        projection: None,
        hydration_receipt: account_state.latest_receipt,
        profile_state,
        remote_signer,
        bulk_decrypt_consent: account_state.bulk_decrypt_consent,
        bulk_decrypt_permission_available: permission_available,
        plaintext_policy: account_state.plaintext_policy,
    })
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

    fn inspect_identity_sync(&self) -> Result<IdentitySyncDashboardState, String> {
        inspect_identity_sync(&self.service)
    }

    fn record_profile_skipped(&self) -> Result<IdentitySyncDashboardState, String> {
        let (cache, _) = active_hydration_cache(&self.service)?;
        cache
            .record_profile_skipped()
            .map_err(|error| error.to_string())?;
        inspect_identity_sync(&self.service)
    }

    fn save_local_profile(
        &self,
        profile: serde_json::Value,
    ) -> Result<IdentitySyncDashboardState, String> {
        let selection = self
            .service
            .selection_token()
            .map_err(|error| error.to_string())?;
        let (cache, _) = active_hydration_cache(&self.service)?;
        cache
            .save_local_profile(profile.clone())
            .map_err(|error| error.to_string())?;
        let projection = self
            .service
            .record_hydrated_profile(&selection, Some(profile_summary(&profile)))
            .map_err(|error| error.to_string())?;
        let mut state = inspect_identity_sync(&self.service)?;
        state.projection = Some(projection);
        Ok(state)
    }

    fn set_bulk_decrypt_consent(
        &self,
        state: BulkDecryptConsentState,
    ) -> Result<IdentitySyncDashboardState, String> {
        let (cache, capability_ref) = active_hydration_cache(&self.service)?;
        let capability_ref = capability_ref.ok_or_else(|| {
            "bulk decrypt consent requires an active remote signer capability".to_string()
        })?;
        cache
            .set_bulk_decrypt_consent(capability_ref, state)
            .map_err(|error| error.to_string())?;
        inspect_identity_sync(&self.service)
    }

    fn set_plaintext_policy(
        &self,
        policy: PlaintextPersistencePolicy,
    ) -> Result<IdentitySyncDashboardState, String> {
        let (cache, _) = active_hydration_cache(&self.service)?;
        cache
            .write_plaintext_persistence_policy(policy)
            .map_err(|error| error.to_string())?;
        inspect_identity_sync(&self.service)
    }

    fn start_identity_hydration(&self, trigger: HydrationTrigger) -> Result<(), String> {
        let selection = self
            .service
            .selection_token()
            .map_err(|error| error.to_string())?;
        let hydration = agent_ui::omega_nostr_profile_transport::start_system_identity_hydration(
            selection, trigger, false,
        )
        .map_err(|error| error.to_string())?;
        async_std::task::spawn(async move {
            if let Err(error) = hydration.await {
                zlog::error!("bounded identity hydration failed: {error}");
            }
        });
        Ok(())
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

fn account_operation_hydration_trigger(operation: &AccountOperation) -> Option<HydrationTrigger> {
    matches!(operation, AccountOperation::Switch(_)).then_some(HydrationTrigger::Switched)
}

#[derive(Clone, Copy)]
enum HostedOperation {
    Connect,
    Verify,
    Disconnect,
}

#[derive(Clone)]
enum IdentitySyncMutation {
    SkipProfile,
    SaveProfile(serde_json::Value),
    BulkDecryptConsent(BulkDecryptConsentState),
    PlaintextPolicy(PlaintextPersistencePolicy),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ProfileEditorState {
    #[default]
    NotStarted,
    Editing,
    Skipped,
    SavedLocally,
    Publishing,
    Published,
    Failed,
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
    profile_editor_open: bool,
    profile_editor_state: ProfileEditorState,
    profile_display_name_input: Entity<Editor>,
    profile_about_input: Entity<Editor>,
    profile_picture_input: Entity<Editor>,
    hydration_receipt: Option<HydrationReceipt>,
    local_profile_state: Option<LocalProfileState>,
    remote_signer: bool,
    bulk_decrypt_consent: BulkDecryptConsentState,
    bulk_decrypt_permission_available: bool,
    plaintext_policy: PlaintextPersistencePolicy,
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
        window: &mut Window,
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
        let profile_display_name_input = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Display name", window, cx);
            editor
        });
        let profile_about_input = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_placeholder_text("About", window, cx);
            editor
        });
        let profile_picture_input = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Picture URL", window, cx);
            editor
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
            profile_editor_open: false,
            profile_editor_state: ProfileEditorState::NotStarted,
            profile_display_name_input,
            profile_about_input,
            profile_picture_input,
            hydration_receipt: None,
            local_profile_state: None,
            remote_signer: false,
            bulk_decrypt_consent: BulkDecryptConsentState::Unknown,
            bulk_decrypt_permission_available: false,
            plaintext_policy: PlaintextPersistencePolicy::Never,
        });
        dashboard.update(cx, |dashboard, cx| dashboard.reload(cx));
        dashboard
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        self.message = None;
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let (result, identity_sync) = cx
                .background_spawn(async move {
                    let result = backend.inspect();
                    let identity_sync = backend.inspect_identity_sync();
                    (result, identity_sync)
                })
                .await;
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
                match identity_sync {
                    Ok(identity_sync) => this.apply_identity_sync_state(identity_sync),
                    Err(error) => {
                        zlog::error!("identity hydration state inspection failed: {error}");
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

    fn apply_identity_sync_state(&mut self, state: IdentitySyncDashboardState) {
        if let Some(projection) = state.projection {
            self.apply_projection(projection);
        }
        self.hydration_receipt = state.hydration_receipt;
        self.local_profile_state = state.profile_state.clone();
        self.remote_signer = state.remote_signer;
        self.bulk_decrypt_consent = state.bulk_decrypt_consent;
        self.bulk_decrypt_permission_available = state.bulk_decrypt_permission_available;
        self.plaintext_policy = state.plaintext_policy;
        if self.profile_editor_state != ProfileEditorState::Published {
            self.profile_editor_state = match state.profile_state {
                Some(LocalProfileState::Skipped { .. }) => ProfileEditorState::Skipped,
                Some(LocalProfileState::SavedLocally { .. }) => ProfileEditorState::SavedLocally,
                None => ProfileEditorState::NotStarted,
            };
        }
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
            let hydration_trigger = account_operation_hydration_trigger(&operation);
            let result = cx
                .background_spawn(async move {
                    let result = backend.apply(operation, expected_generation)?;
                    let hydration_error = hydration_trigger
                        .and_then(|trigger| backend.start_identity_hydration(trigger).err());
                    Ok::<_, String>((result, hydration_error))
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((result, hydration_error)) => {
                        this.purge_report = result.purge_report;
                        this.apply_projection(result.projection);
                        this.message = hydration_error.map(|error| {
                            zlog::error!("identity hydration could not start: {error}");
                            "The account switched, but background hydration could not start.".into()
                        });
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
            let identity_for_results = identity.clone();
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
                    let hydration_cache = HydrationAccountFence::new(
                        account_ref_for_results.clone(),
                        identity_for_results.public_key_hex().clone(),
                        expected_generation,
                    )
                    .map_err(|error| error.to_string())
                    .and_then(|fence| {
                        HydrationCache::system(fence).map_err(|error| error.to_string())
                    });
                    for (target, areas) in [
                        (
                            AccountPurgeTarget::DecryptedCache,
                            &[
                                HydrationCacheArea::Plaintext,
                                HydrationCacheArea::Ciphertext,
                            ][..],
                        ),
                        (
                            AccountPurgeTarget::RelayState,
                            &[
                                HydrationCacheArea::Profiles,
                                HydrationCacheArea::Relays,
                                HydrationCacheArea::Groups,
                                HydrationCacheArea::Receipts,
                            ][..],
                        ),
                        (
                            AccountPurgeTarget::SignerSessions,
                            &[HydrationCacheArea::Consent][..],
                        ),
                    ] {
                        let verification = match hydration_cache.as_ref() {
                            Ok(cache) => {
                                hydration_purge_verification(purge_hydration_areas(cache, areas))
                            }
                            Err(error) => AccountPurgeVerification::Failed {
                                reason: error.clone(),
                            },
                        };
                        backend_for_results.record_purge_target(
                            &account_ref_for_results,
                            &operation_ref_for_results,
                            target,
                            verification,
                        )?;
                    }
                    for target in [
                        AccountPurgeTarget::WalletState,
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
            .child(self.render_profile_editor(cx))
            .child(Divider::horizontal())
            .child(self.render_hydration_status())
            .when(is_remote, |this| {
                this.child(Divider::horizontal())
                    .child(self.render_external_signer_data_controls(cx))
            })
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

    fn open_profile_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (mut display_name, mut about, mut picture) = self.selected_entry().map_or_else(
            || (String::new(), String::new(), String::new()),
            |account| {
                account.profile.as_ref().map_or_else(
                    || (String::new(), String::new(), String::new()),
                    |profile| {
                        (
                            profile.display_name.clone().unwrap_or_default(),
                            String::new(),
                            profile.avatar_ref.clone().unwrap_or_default(),
                        )
                    },
                )
            },
        );
        if let Some(LocalProfileState::SavedLocally { profile, .. }) =
            self.local_profile_state.as_ref()
        {
            display_name = profile_string(profile, "display_name");
            about = profile_string(profile, "about");
            picture = profile_string(profile, "picture");
        }
        self.profile_display_name_input
            .update(cx, |editor, cx| editor.set_text(display_name, window, cx));
        self.profile_about_input
            .update(cx, |editor, cx| editor.set_text(about, window, cx));
        self.profile_picture_input
            .update(cx, |editor, cx| editor.set_text(picture, window, cx));
        self.profile_editor_open = true;
        self.profile_editor_state = ProfileEditorState::Editing;
        cx.notify();
    }

    fn skip_profile(&mut self, cx: &mut Context<Self>) {
        self.profile_editor_open = false;
        self.run_identity_sync_mutation(IdentitySyncMutation::SkipProfile, cx);
    }

    fn save_profile_locally(&mut self, cx: &mut Context<Self>) {
        let draft = self.profile_draft(cx);
        if let Err(message) = validate_profile_draft(&draft) {
            self.profile_editor_state = ProfileEditorState::Failed;
            self.message = Some(message.into());
            cx.notify();
            return;
        }
        self.run_identity_sync_mutation(
            IdentitySyncMutation::SaveProfile(profile_draft_json(&draft)),
            cx,
        );
    }

    fn publish_profile(&mut self, cx: &mut Context<Self>) {
        let draft = self.profile_draft(cx);
        if let Err(message) = validate_profile_draft(&draft) {
            self.profile_editor_state = ProfileEditorState::Failed;
            self.message = Some(message.into());
            cx.notify();
            return;
        }
        let profile = profile_draft_json(&draft);
        let timer = cx.background_executor().clone();
        self.profile_editor_state = ProfileEditorState::Publishing;
        self.busy = true;
        self.message = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let registry = AccountRegistryService::system(*app_identity::CHANNEL);
                    let selection = registry
                        .selection_token()
                        .map_err(|error| error.to_string())?;
                    let dashboard = registry.inspect().map_err(|error| error.to_string())?;
                    let active = dashboard
                        .accounts
                        .iter()
                        .find(|account| account.is_active)
                        .ok_or_else(|| "No active account can publish a profile.".to_string())?;
                    let route = match active.signer.kind {
                        SignerKind::LocalNative => SignerRoute::Local {
                            identity_service: Arc::new(IdentityService::system(
                                *app_identity::CHANNEL,
                            )),
                        },
                        SignerKind::RemoteNip46 => {
                            let capability = registry
                                .remote_signer_capability(&selection)
                                .map_err(|error| error.to_string())?;
                            SignerRoute::RemoteNip46 {
                                metadata: RemoteSignerMetadata { capability },
                                transport: Arc::new(Nip46WebSocketTransport::system()),
                            }
                        }
                        SignerKind::BrowserNip07
                        | SignerKind::AndroidNip55
                        | SignerKind::DeviceGrant
                        | SignerKind::AgentGrant => {
                            return Err(
                                "The active signer has no kind-0 publication route.".to_string()
                            );
                        }
                    };
                    let outcome = apply_profile_choice(
                        &SignerBroker::system(),
                        &route,
                        selection.clone(),
                        ProfileChoice::Publish(profile.clone()),
                        &[PROFILE_PUBLISH_RELAY.to_string()],
                        unix_time_seconds(),
                        timer,
                        |_| Ok(()),
                    )
                    .await
                    .map_err(|error| profile_publish_error_message(&error).to_string())?;
                    let profile_summary = profile_summary(&profile);
                    let fence = HydrationAccountFence::new(
                        selection.account_ref.clone(),
                        selection.identity.public_key_hex().clone(),
                        selection.generation,
                    )
                    .map_err(|error| error.to_string())?;
                    HydrationCache::system(fence)
                        .and_then(|cache| cache.save_local_profile(profile))
                        .map_err(|error| error.to_string())?;
                    let projection = registry
                        .record_hydrated_profile(&selection, Some(profile_summary))
                        .map_err(|error| error.to_string())?;
                    let mut identity_sync = inspect_identity_sync(&registry)?;
                    identity_sync.projection = Some(projection);
                    Ok::<_, String>((outcome, identity_sync))
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((ProfileChoiceOutcome::Published { .. }, state)) => {
                        this.apply_identity_sync_state(state);
                        this.profile_editor_open = false;
                        this.profile_editor_state = ProfileEditorState::Published;
                        this.message = Some("Profile published and acknowledged.".into());
                    }
                    Ok((ProfileChoiceOutcome::Skipped, _))
                    | Ok((ProfileChoiceOutcome::SavedLocally, _)) => {
                        this.profile_editor_state = ProfileEditorState::Failed;
                        this.message = Some("The profile publish outcome was invalid.".into());
                    }
                    Err(error) => {
                        zlog::error!("kind-0 profile publication failed: {error}");
                        this.profile_editor_state = ProfileEditorState::Failed;
                        this.message = Some(error.into());
                    }
                }
                this.busy = false;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn profile_draft(&self, cx: &App) -> ProfileDraft {
        ProfileDraft {
            display_name: self.profile_display_name_input.read(cx).text(cx),
            about: self.profile_about_input.read(cx).text(cx),
            picture: self.profile_picture_input.read(cx).text(cx),
        }
    }

    fn render_profile_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let active_selected = self
            .selected_entry()
            .is_some_and(|account| account.is_active);
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Nostr profile").size(LabelSize::Small))
                    .child(
                        Label::new(profile_editor_state_label(self.profile_editor_state))
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    ),
            )
            .when(!self.profile_editor_open, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("omega-profile-edit", "Edit profile")
                                .style(ButtonStyle::OutlinedGhost)
                                .size(ButtonSize::Compact)
                                .disabled(self.busy || !active_selected)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_profile_editor(window, cx)
                                })),
                        )
                        .child(
                            Button::new("omega-profile-skip", "Skip")
                                .style(ButtonStyle::OutlinedGhost)
                                .size(ButtonSize::Compact)
                                .disabled(self.busy || !active_selected)
                                .tooltip(Tooltip::text("No publish"))
                                .on_click(cx.listener(|this, _, _, cx| this.skip_profile(cx))),
                        ),
                )
            })
            .when(self.profile_editor_open, |this| {
                this.child(profile_input(
                    "Display name",
                    self.profile_display_name_input.clone(),
                    false,
                ))
                .child(profile_input(
                    "About",
                    self.profile_about_input.clone(),
                    true,
                ))
                .child(profile_input(
                    "Picture URL",
                    self.profile_picture_input.clone(),
                    false,
                ))
                .child(detail_row("Publish relay", PROFILE_PUBLISH_RELAY, None))
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("omega-profile-save-local", "Save locally")
                                .style(ButtonStyle::OutlinedGhost)
                                .size(ButtonSize::Compact)
                                .disabled(self.busy || !active_selected)
                                .tooltip(Tooltip::text("Local only"))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.save_profile_locally(cx)),
                                ),
                        )
                        .child(
                            Button::new("omega-profile-publish", "Publish profile")
                                .style(ButtonStyle::Filled)
                                .size(ButtonSize::Compact)
                                .disabled(self.busy || !active_selected)
                                .tooltip(Tooltip::text("Sign kind 0"))
                                .on_click(cx.listener(|this, _, _, cx| this.publish_profile(cx))),
                        )
                        .child(
                            Button::new("omega-profile-skip-editor", "Skip")
                                .style(ButtonStyle::OutlinedGhost)
                                .size(ButtonSize::Compact)
                                .disabled(self.busy || !active_selected)
                                .tooltip(Tooltip::text("No publish"))
                                .on_click(cx.listener(|this, _, _, cx| this.skip_profile(cx))),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_hydration_status(&self) -> AnyElement {
        let receipt = self
            .hydration_receipt
            .as_ref()
            .filter(|receipt| self.hydration_receipt_matches_selected(receipt));
        let source_rows = receipt
            .map(|receipt| {
                receipt
                    .sources
                    .iter()
                    .map(|source| {
                        detail_row(
                            hydration_source_label(source.source),
                            hydration_source_outcome_label(&source.outcome),
                            hydration_source_outcome_icon(&source.outcome),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        v_flex()
            .gap_2()
            .child(Label::new("Account hydration").size(LabelSize::Small))
            .child(detail_row(
                "Hydration",
                receipt.map_or("Not started", |receipt| {
                    hydration_state_label(receipt.state)
                }),
                receipt.and_then(|receipt| hydration_state_icon(receipt.state)),
            ))
            .children(source_rows)
            .when(
                receipt.is_some_and(|receipt| receipt.background_continuation_available),
                |this| {
                    this.child(detail_row(
                        "Background recovery",
                        "Continuing",
                        Some(
                            Icon::new(IconName::ArrowCircle)
                                .size(IconSize::Small)
                                .color(Color::Warning),
                        ),
                    ))
                },
            )
            .into_any_element()
    }

    fn hydration_receipt_matches_selected(&self, receipt: &HydrationReceipt) -> bool {
        let Some(account) = self.selected_entry().filter(|account| account.is_active) else {
            return false;
        };
        self.projection.as_ref().is_some_and(|projection| {
            receipt.fence.account_ref == account.account_ref
                && receipt.fence.public_key_hex == *account.identity.public_key_hex()
                && receipt.fence.generation == projection.active.generation
        })
    }

    fn set_bulk_decrypt_consent(
        &mut self,
        consent: BulkDecryptConsentState,
        cx: &mut Context<Self>,
    ) {
        self.run_identity_sync_mutation(IdentitySyncMutation::BulkDecryptConsent(consent), cx);
    }

    fn set_plaintext_policy(&mut self, policy: PlaintextPersistencePolicy, cx: &mut Context<Self>) {
        self.run_identity_sync_mutation(IdentitySyncMutation::PlaintextPolicy(policy), cx);
    }

    fn run_identity_sync_mutation(
        &mut self,
        mutation: IdentitySyncMutation,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let backend = self.backend.clone();
        self.busy = true;
        self.message = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let mutation_for_result = mutation.clone();
            let result = cx
                .background_spawn(async move {
                    match mutation {
                        IdentitySyncMutation::SkipProfile => backend.record_profile_skipped(),
                        IdentitySyncMutation::SaveProfile(profile) => {
                            backend.save_local_profile(profile)
                        }
                        IdentitySyncMutation::BulkDecryptConsent(state) => {
                            backend.set_bulk_decrypt_consent(state)
                        }
                        IdentitySyncMutation::PlaintextPolicy(policy) => {
                            backend.set_plaintext_policy(policy)
                        }
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(state) => {
                        this.apply_identity_sync_state(state);
                        this.message = Some(
                            identity_sync_mutation_success_message(&mutation_for_result).into(),
                        );
                    }
                    Err(error) => {
                        zlog::error!("identity hydration state update failed: {error}");
                        if matches!(
                            mutation_for_result,
                            IdentitySyncMutation::SkipProfile
                                | IdentitySyncMutation::SaveProfile(_)
                        ) {
                            this.profile_editor_state = ProfileEditorState::Failed;
                        }
                        this.message =
                            Some("The account setting could not be saved. Try again.".into());
                    }
                }
                this.busy = false;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn render_external_signer_data_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Label::new("External signer data").size(LabelSize::Small))
            .child(detail_row(
                "Bulk decrypt",
                bulk_decrypt_consent_label(
                    self.bulk_decrypt_consent,
                    self.remote_signer,
                    self.bulk_decrypt_permission_available,
                ),
                bulk_decrypt_consent_icon(
                    self.bulk_decrypt_consent,
                    self.remote_signer,
                    self.bulk_decrypt_permission_available,
                ),
            ))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("omega-bulk-decrypt-allow", "Allow bulk decrypt")
                            .style(ButtonStyle::OutlinedGhost)
                            .size(ButtonSize::Compact)
                            .disabled(
                                self.busy
                                    || !self.remote_signer
                                    || !self.bulk_decrypt_permission_available
                                    || self.bulk_decrypt_consent
                                        == BulkDecryptConsentState::Allowed,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_bulk_decrypt_consent(BulkDecryptConsentState::Allowed, cx)
                            })),
                    )
                    .child(
                        Button::new("omega-bulk-decrypt-decline", "Decline")
                            .style(ButtonStyle::OutlinedGhost)
                            .size(ButtonSize::Compact)
                            .disabled(
                                self.busy
                                    || !self.remote_signer
                                    || self.bulk_decrypt_consent
                                        == BulkDecryptConsentState::Declined,
                            )
                            .tooltip(Tooltip::text("Keep locked"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_bulk_decrypt_consent(BulkDecryptConsentState::Declined, cx)
                            })),
                    )
                    .when(
                        self.remote_signer && !self.bulk_decrypt_permission_available,
                        |this| {
                            this.child(
                                Button::new("omega-bulk-decrypt-reconnect", "Reconnect signer")
                                    .style(ButtonStyle::OutlinedGhost)
                                    .size(ButtonSize::Compact)
                                    .disabled(self.busy)
                                    .tooltip(Tooltip::text("Permission missing"))
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(
                                            OpenRemoteSignerSetup.boxed_clone(),
                                            cx,
                                        )
                                    }),
                            )
                        },
                    ),
            )
            .child(detail_row(
                "Plaintext cache",
                plaintext_policy_label(self.plaintext_policy),
                plaintext_policy_icon(self.plaintext_policy),
            ))
            .child(detail_row(
                "Plaintext storage",
                "Ordinary unencrypted account files",
                None,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("omega-plaintext-never", "Do not persist")
                            .style(ButtonStyle::OutlinedGhost)
                            .size(ButtonSize::Compact)
                            .disabled(
                                self.busy
                                    || self.plaintext_policy == PlaintextPersistencePolicy::Never,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_plaintext_policy(PlaintextPersistencePolicy::Never, cx)
                            })),
                    )
                    .child(
                        Button::new("omega-plaintext-bounded", "Cache public plaintext")
                            .style(ButtonStyle::OutlinedGhost)
                            .size(ButtonSize::Compact)
                            .disabled(
                                self.busy
                                    || self.plaintext_policy
                                        == PlaintextPersistencePolicy::NonPrivateNonExpiring,
                            )
                            .tooltip(Tooltip::text("Non-private only"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_plaintext_policy(
                                    PlaintextPersistencePolicy::NonPrivateNonExpiring,
                                    cx,
                                )
                            })),
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

fn purge_hydration_areas(
    cache: &HydrationCache,
    areas: &[HydrationCacheArea],
) -> Result<(), String> {
    for area in areas {
        cache.purge_area(*area).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn hydration_purge_verification(result: Result<(), String>) -> AccountPurgeVerification {
    match result {
        Ok(()) => AccountPurgeVerification::VerifiedDeleted,
        Err(reason) => AccountPurgeVerification::Failed { reason },
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProfileDraft {
    display_name: String,
    about: String,
    picture: String,
}

fn validate_profile_draft(draft: &ProfileDraft) -> Result<(), &'static str> {
    if draft.display_name.len() > 128 {
        return Err("Display name must be 128 bytes or fewer.");
    }
    if draft.about.len() > 4_096 {
        return Err("About must be 4096 bytes or fewer.");
    }
    if draft.picture.len() > 2_048 {
        return Err("Picture URL must be 2048 bytes or fewer.");
    }
    if !draft.picture.trim().is_empty() && !draft.picture.trim().starts_with("https://") {
        return Err("Picture URL must use HTTPS.");
    }
    Ok(())
}

fn profile_draft_json(draft: &ProfileDraft) -> serde_json::Value {
    serde_json::json!({
        "display_name": draft.display_name.trim(),
        "about": draft.about.trim(),
        "picture": draft.picture.trim(),
    })
}

fn profile_string(profile: &serde_json::Value, field: &str) -> String {
    profile
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn profile_summary(profile: &serde_json::Value) -> AccountProfileSummary {
    let display_name = profile_string(profile, "display_name");
    let picture = profile_string(profile, "picture");
    AccountProfileSummary {
        display_name: (!display_name.is_empty()).then_some(display_name),
        avatar_ref: (!picture.is_empty()).then_some(picture),
    }
}

fn profile_publish_error_message(error: &ProfilePublishError) -> &'static str {
    match error {
        ProfilePublishError::RemotePermissionRequired => {
            "Reconnect the signer to add kind-0 profile permission."
        }
        ProfilePublishError::RelayAuthenticationRequired => {
            "The profile relay requires authentication before publication."
        }
        ProfilePublishError::RelayRejected(_) => "The profile relay rejected this kind-0 event.",
        ProfilePublishError::MissingAcknowledgement => {
            "The relay did not acknowledge this exact profile event. Try again."
        }
        ProfilePublishError::Other(_) => "The profile could not be signed or published.",
    }
}

fn identity_sync_mutation_success_message(mutation: &IdentitySyncMutation) -> &'static str {
    match mutation {
        IdentitySyncMutation::SkipProfile => "Profile skipped. Nothing was signed or published.",
        IdentitySyncMutation::SaveProfile(_) => {
            "Profile draft saved locally. Nothing was published."
        }
        IdentitySyncMutation::BulkDecryptConsent(BulkDecryptConsentState::Unknown) => {
            "Bulk decrypt consent reset. Omega will ask before bulk decrypt."
        }
        IdentitySyncMutation::BulkDecryptConsent(BulkDecryptConsentState::Allowed) => {
            "Bulk decrypt allowed for this account and signer capability."
        }
        IdentitySyncMutation::BulkDecryptConsent(BulkDecryptConsentState::Declined) => {
            "Bulk decrypt declined. Content remains locked and Omega will not ask again."
        }
        IdentitySyncMutation::PlaintextPolicy(PlaintextPersistencePolicy::Never) => {
            "Persistent plaintext disabled for this account."
        }
        IdentitySyncMutation::PlaintextPolicy(
            PlaintextPersistencePolicy::NonPrivateNonExpiring,
        ) => "Only non-private, non-expiring plaintext may be cached for this account.",
    }
}

fn profile_editor_state_label(state: ProfileEditorState) -> &'static str {
    match state {
        ProfileEditorState::NotStarted => "Optional",
        ProfileEditorState::Editing => "Editing",
        ProfileEditorState::Skipped => "Skipped",
        ProfileEditorState::SavedLocally => "Saved locally",
        ProfileEditorState::Publishing => "Publishing",
        ProfileEditorState::Published => "Published",
        ProfileEditorState::Failed => "Failed",
    }
}

fn profile_input(label: &'static str, editor: Entity<Editor>, multiline: bool) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            Label::new(label)
                .color(Color::Muted)
                .size(LabelSize::XSmall),
        )
        .child(
            div()
                .w_full()
                .when(multiline, |this| this.h_20())
                .when(!multiline, |this| this.h_8())
                .overflow_hidden()
                .border_1()
                .rounded_sm()
                .child(editor),
        )
        .into_any_element()
}

fn hydration_state_label(state: HydrationState) -> &'static str {
    match state {
        HydrationState::Complete => "Complete",
        HydrationState::Partial => "Partial",
        HydrationState::Offline => "Offline",
        HydrationState::Failed => "Failed",
        HydrationState::SkippedFresh => "Skipped fresh",
    }
}

fn hydration_state_icon(state: HydrationState) -> Option<Icon> {
    let (icon, color) = match state {
        HydrationState::Complete | HydrationState::SkippedFresh => {
            (IconName::Check, Color::Success)
        }
        HydrationState::Partial | HydrationState::Offline => (IconName::Warning, Color::Warning),
        HydrationState::Failed => (IconName::Warning, Color::Error),
    };
    Some(Icon::new(icon).size(IconSize::Small).color(color))
}

fn hydration_source_label(source: HydrationSource) -> &'static str {
    match source {
        HydrationSource::Profile => "Profile",
        HydrationSource::RelayPreferences => "Relay preferences",
        HydrationSource::Nip29GroupList => "NIP-29 groups",
        HydrationSource::MembershipMetadata => "Membership metadata",
        HydrationSource::RecentRooms => "Recent rooms",
        HydrationSource::HostedAccount => "Hosted account",
        HydrationSource::HostedDevice => "Hosted device",
        HydrationSource::BuzzProfile => "Buzz profile",
        HydrationSource::ArmadaProfile => "Armada profile",
    }
}

fn hydration_source_outcome_label(outcome: &HydrationSourceOutcome) -> String {
    match outcome {
        HydrationSourceOutcome::Complete { items } => format!("Fresh · {items} items"),
        HydrationSourceOutcome::Cached { items, reason } => {
            let reason = match reason {
                CacheFallbackReason::Offline => "offline",
                CacheFallbackReason::Timeout => "timeout",
                CacheFallbackReason::Failure => "failed",
            };
            format!("Cached · {items} items · {reason}")
        }
        HydrationSourceOutcome::Stale { cached_items } => {
            format!("Stale · {cached_items} cached items")
        }
        HydrationSourceOutcome::Locked { ciphertext_items } => {
            format!("Locked · {ciphertext_items} ciphertext items")
        }
        HydrationSourceOutcome::Disabled => "Disabled".to_string(),
        HydrationSourceOutcome::Offline => "Offline".to_string(),
        HydrationSourceOutcome::TimedOut { scope } => match scope {
            TimeoutScope::Source => "Timeout · source".to_string(),
            TimeoutScope::Overall => "Timeout · overall".to_string(),
        },
        HydrationSourceOutcome::Failed => "Failed".to_string(),
    }
}

fn hydration_source_outcome_icon(outcome: &HydrationSourceOutcome) -> Option<Icon> {
    let (icon, color) = match outcome {
        HydrationSourceOutcome::Complete { .. } => (IconName::Check, Color::Success),
        HydrationSourceOutcome::Cached { .. }
        | HydrationSourceOutcome::Stale { .. }
        | HydrationSourceOutcome::Locked { .. }
        | HydrationSourceOutcome::Disabled
        | HydrationSourceOutcome::Offline
        | HydrationSourceOutcome::TimedOut { .. } => (IconName::Warning, Color::Warning),
        HydrationSourceOutcome::Failed => (IconName::Warning, Color::Error),
    };
    Some(Icon::new(icon).size(IconSize::Small).color(color))
}

fn bulk_decrypt_consent_label(
    consent: BulkDecryptConsentState,
    remote_signer: bool,
    permission_available: bool,
) -> &'static str {
    if !remote_signer {
        return "Not applicable";
    }
    if !permission_available {
        return "Reconnect signer";
    }
    match consent {
        BulkDecryptConsentState::Unknown => "Unknown",
        BulkDecryptConsentState::Allowed => "Allowed",
        BulkDecryptConsentState::Declined => "Declined",
    }
}

fn bulk_decrypt_consent_icon(
    consent: BulkDecryptConsentState,
    remote_signer: bool,
    permission_available: bool,
) -> Option<Icon> {
    let (icon, color) = if !remote_signer {
        (IconName::Info, Color::Muted)
    } else if !permission_available {
        (IconName::Warning, Color::Warning)
    } else {
        match consent {
            BulkDecryptConsentState::Unknown => (IconName::Info, Color::Muted),
            BulkDecryptConsentState::Allowed => (IconName::Check, Color::Success),
            BulkDecryptConsentState::Declined => (IconName::Lock, Color::Warning),
        }
    };
    Some(Icon::new(icon).size(IconSize::Small).color(color))
}

fn plaintext_policy_label(policy: PlaintextPersistencePolicy) -> &'static str {
    match policy {
        PlaintextPersistencePolicy::Never => "Do not persist",
        PlaintextPersistencePolicy::NonPrivateNonExpiring => "Non-private, non-expiring only",
    }
}

fn plaintext_policy_icon(policy: PlaintextPersistencePolicy) -> Option<Icon> {
    let (icon, color) = match policy {
        PlaintextPersistencePolicy::Never => (IconName::Lock, Color::Success),
        PlaintextPersistencePolicy::NonPrivateNonExpiring => (IconName::Info, Color::Warning),
    };
    Some(Icon::new(icon).size(IconSize::Small).color(color))
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
    fn only_account_switch_starts_switch_hydration() {
        let account_ref = AccountRef::new("switch-target").expect("account ref");
        assert_eq!(
            account_operation_hydration_trigger(&AccountOperation::Switch(account_ref)),
            Some(HydrationTrigger::Switched)
        );
        assert_eq!(
            account_operation_hydration_trigger(&AccountOperation::Lock),
            None
        );
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

    #[test]
    fn optional_profile_choices_remain_distinct_and_bounded() {
        let draft = ProfileDraft {
            display_name: "Omega".to_string(),
            about: "A local draft".to_string(),
            picture: "https://example.com/avatar.png".to_string(),
        };
        assert!(validate_profile_draft(&draft).is_ok());
        assert_eq!(
            profile_draft_json(&draft)["display_name"],
            serde_json::json!("Omega")
        );
        assert_eq!(
            profile_editor_state_label(ProfileEditorState::Skipped),
            "Skipped"
        );
        assert_eq!(
            profile_editor_state_label(ProfileEditorState::SavedLocally),
            "Saved locally"
        );
        assert_eq!(
            profile_editor_state_label(ProfileEditorState::Published),
            "Published"
        );
        assert!(
            identity_sync_mutation_success_message(&IdentitySyncMutation::SkipProfile)
                .contains("Nothing was signed or published")
        );

        let invalid_picture = ProfileDraft {
            picture: "http://example.com/avatar.png".to_string(),
            ..draft
        };
        assert!(validate_profile_draft(&invalid_picture).is_err());
    }

    #[test]
    fn hydration_status_preserves_cache_lock_and_timeout_details() {
        assert_eq!(
            hydration_state_label(HydrationState::SkippedFresh),
            "Skipped fresh"
        );
        assert!(
            hydration_source_outcome_label(&HydrationSourceOutcome::Stale { cached_items: 3 })
                .contains("3 cached items")
        );
        assert!(
            hydration_source_outcome_label(&HydrationSourceOutcome::Locked {
                ciphertext_items: 2
            })
            .contains("Locked")
        );
        assert_eq!(
            hydration_source_outcome_label(&HydrationSourceOutcome::TimedOut {
                scope: TimeoutScope::Overall
            }),
            "Timeout · overall"
        );
    }

    #[test]
    fn bulk_decrypt_consent_is_external_signer_only() {
        assert_eq!(
            bulk_decrypt_consent_label(BulkDecryptConsentState::Unknown, false, false),
            "Not applicable"
        );
        assert_eq!(
            bulk_decrypt_consent_label(BulkDecryptConsentState::Unknown, true, false),
            "Reconnect signer"
        );
        assert_eq!(
            bulk_decrypt_consent_label(BulkDecryptConsentState::Declined, true, true),
            "Declined"
        );
    }

    #[test]
    fn hydration_purge_failures_are_not_rounded_up() {
        assert_eq!(
            hydration_purge_verification(Ok(())),
            AccountPurgeVerification::VerifiedDeleted
        );
        assert_eq!(
            hydration_purge_verification(Err("ciphertext remains".to_string())),
            AccountPurgeVerification::Failed {
                reason: "ciphertext remains".to_string()
            }
        );
    }
}
