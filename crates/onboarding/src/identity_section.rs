use std::{fmt, path::PathBuf, sync::Arc};

use db::kvp::KeyValueStore;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, IntoElement, PathPromptOptions, Render,
    SharedString, Task, Window,
};
use omega_identity::{
    CandidateRef, CustodyConflictReason, CustodyError, CustodyResult, CustodyState,
    IdentityInspection, IdentityRef, IdentityService, ImportedSecret, PendingIdentityOperation,
    PreparedRecovery, PublicIdentity, ReceiptRef, RecoveryCandidate, RecoveryPassword,
    RecoveryProtectionState, RecoveryResolution, RecoveryResolutionState,
};
use ui::prelude::*;
use ui_input::InputField;
use util::ResultExt as _;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    identity_controller::{
        IdentityControllerState, IdentityOperation, IdentityUiError, OperationToken,
    },
    identity_profile::{LocalIdentityProfile, install_local_avatar},
    secure_input::SecureInput,
};

const IDENTITY_TAB_SLOTS: isize = 12;

trait IdentityBackend: Send + Sync {
    fn inspect(&self) -> Result<IdentityInspection, CustodyError>;
    fn create(&self, receipt_ref: ReceiptRef) -> Result<IdentityInspection, CustodyError>;
    fn resume_incomplete_create(&self) -> Result<IdentityInspection, CustodyError>;
    fn prepare_import(
        &self,
        imported_secret: ImportedSecret,
    ) -> Result<PreparedRecovery, CustodyError>;
    fn discover_artifact(&self, path: PathBuf) -> Result<RecoveryCandidate, CustodyError>;
    fn prepare_artifact(
        &self,
        candidate: &RecoveryCandidate,
        password: RecoveryPassword,
    ) -> Result<PreparedRecovery, CustodyError>;
    fn reconcile(&self, candidates: &[&PreparedRecovery]) -> RecoveryResolution;
    fn adopt(
        &self,
        candidates: Vec<PreparedRecovery>,
        selected_candidate_ref: &CandidateRef,
        receipt_ref: ReceiptRef,
    ) -> Result<IdentityInspection, CustodyError>;
    fn resolve_conflict(
        &self,
        candidates: Vec<PreparedRecovery>,
        selected_candidate_ref: &CandidateRef,
        receipt_ref: ReceiptRef,
    ) -> Result<IdentityInspection, CustodyError>;
    fn export_artifact(
        &self,
        identity_ref: &IdentityRef,
        path: PathBuf,
        password: RecoveryPassword,
    ) -> Result<IdentityInspection, CustodyError>;
    fn reset(
        &self,
        identity_ref: &IdentityRef,
        receipt_ref: ReceiptRef,
    ) -> Result<CustodyResult, CustodyError>;
    fn resume_reset(&self) -> Result<CustodyResult, CustodyError>;
}

struct SystemIdentityBackend {
    service: IdentityService,
}

impl SystemIdentityBackend {
    fn new() -> Self {
        Self {
            service: IdentityService::system(*app_identity::CHANNEL),
        }
    }
}

impl IdentityBackend for SystemIdentityBackend {
    fn inspect(&self) -> Result<IdentityInspection, CustodyError> {
        self.service.inspect_details()
    }

    fn create(&self, receipt_ref: ReceiptRef) -> Result<IdentityInspection, CustodyError> {
        self.service.create(receipt_ref)?;
        self.service.inspect_details()
    }

    fn resume_incomplete_create(&self) -> Result<IdentityInspection, CustodyError> {
        self.service.resume_incomplete_create()?;
        self.service.inspect_details()
    }

    fn prepare_import(
        &self,
        imported_secret: ImportedSecret,
    ) -> Result<PreparedRecovery, CustodyError> {
        self.service.prepare_import(imported_secret)
    }

    fn discover_artifact(&self, path: PathBuf) -> Result<RecoveryCandidate, CustodyError> {
        self.service.discover_recovery_artifact(path)
    }

    fn prepare_artifact(
        &self,
        candidate: &RecoveryCandidate,
        password: RecoveryPassword,
    ) -> Result<PreparedRecovery, CustodyError> {
        self.service.prepare_recovery_artifact(candidate, password)
    }

    fn reconcile(&self, candidates: &[&PreparedRecovery]) -> RecoveryResolution {
        self.service.reconcile_recoveries(candidates)
    }

    fn adopt(
        &self,
        candidates: Vec<PreparedRecovery>,
        selected_candidate_ref: &CandidateRef,
        receipt_ref: ReceiptRef,
    ) -> Result<IdentityInspection, CustodyError> {
        let selected = self
            .service
            .select_recovery(candidates, selected_candidate_ref)?;
        self.service.adopt(selected, receipt_ref)?;
        self.service.inspect_details()
    }

    fn resolve_conflict(
        &self,
        candidates: Vec<PreparedRecovery>,
        selected_candidate_ref: &CandidateRef,
        receipt_ref: ReceiptRef,
    ) -> Result<IdentityInspection, CustodyError> {
        let selected = self
            .service
            .select_recovery(candidates, selected_candidate_ref)?;
        self.service.resolve_conflict(selected, receipt_ref)?;
        self.service.inspect_details()
    }

    fn export_artifact(
        &self,
        identity_ref: &IdentityRef,
        path: PathBuf,
        password: RecoveryPassword,
    ) -> Result<IdentityInspection, CustodyError> {
        self.service
            .export_recovery_artifact(identity_ref, &path, password)?;
        self.service.inspect_details()
    }

    fn reset(
        &self,
        identity_ref: &IdentityRef,
        receipt_ref: ReceiptRef,
    ) -> Result<CustodyResult, CustodyError> {
        self.service.reset(identity_ref, receipt_ref)
    }

    fn resume_reset(&self) -> Result<CustodyResult, CustodyError> {
        self.service.resume_pending_reset()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum IdentityAction {
    Create,
    Recover,
    Retry,
    ResumeCreate,
    RetryReset,
    Reset,
    Relaunch,
    Protect,
}

impl IdentityAction {
    fn id(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Recover => "recover",
            Self::Retry => "retry",
            Self::ResumeCreate => "resume-create",
            Self::RetryReset => "retry-reset",
            Self::Reset => "reset",
            Self::Relaunch => "relaunch",
            Self::Protect => "protect",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Create => "Create identity",
            Self::Recover => "Recover identity",
            Self::Retry => "Try again",
            Self::ResumeCreate => "Resume setup",
            Self::RetryReset => "Retry reset",
            Self::Reset => "Reset identity",
            Self::Relaunch => "Relaunch Omega",
            Self::Protect => "Protect recovery",
        }
    }

    fn primary(self) -> bool {
        matches!(
            self,
            Self::Create
                | Self::Retry
                | Self::ResumeCreate
                | Self::RetryReset
                | Self::Relaunch
                | Self::Protect
        )
    }
}

struct IdentityPresentation {
    title: &'static str,
    description: &'static str,
    icon: IconName,
    color: Color,
    actions: Vec<IdentityAction>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RecoveryMode {
    Choose,
    AdvancedImport,
    ArtifactPassword,
    Review,
    Protect,
}

struct RecoverySession {
    prepared: Vec<PreparedRecovery>,
    resolution: RecoveryResolution,
}

impl fmt::Debug for RecoverySession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoverySession")
            .field("candidate_count", &self.resolution.candidates.len())
            .field("resolution", &self.resolution.state)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

pub(crate) struct IdentitySection {
    backend: Arc<dyn IdentityBackend>,
    controller: IdentityControllerState,
    first_tab_index: isize,
    operation_task: Option<Task<()>>,
    profile_task: Option<Task<()>>,
    recovery_mode: Option<RecoveryMode>,
    recovery_candidate: Option<RecoveryCandidate>,
    recovery_session: Option<RecoverySession>,
    secret_input: Entity<SecureInput>,
    password_input: Entity<SecureInput>,
    confirmation_input: Entity<SecureInput>,
    display_name_input: Entity<InputField>,
    local_profile: Option<LocalIdentityProfile>,
    profile_message: Option<SharedString>,
}

impl IdentitySection {
    pub(crate) fn new(first_tab_index: isize, window: &mut Window, cx: &mut App) -> Entity<Self> {
        Self::new_with_backend(
            first_tab_index,
            Arc::new(SystemIdentityBackend::new()),
            window,
            cx,
        )
    }

    fn new_with_backend(
        first_tab_index: isize,
        backend: Arc<dyn IdentityBackend>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let secret_input = cx.new(|cx| {
            SecureInput::new(
                "Paste an nsec or 64-character secret",
                "Nostr secret key",
                first_tab_index + 2,
                cx,
            )
        });
        let password_input = cx.new(|cx| {
            SecureInput::new(
                "Recovery password",
                "Recovery password",
                first_tab_index + 2,
                cx,
            )
        });
        let confirmation_input = cx.new(|cx| {
            SecureInput::new(
                "Confirm recovery password",
                "Confirm recovery password",
                first_tab_index + 3,
                cx,
            )
        });
        let display_name_input = cx.new(|cx| {
            InputField::new(window, cx, "Optional local display name")
                .label("Local display name")
                .tab_index(first_tab_index + 8)
        });

        let section = cx.new(|_| Self {
            backend,
            controller: IdentityControllerState::default(),
            first_tab_index,
            operation_task: None,
            profile_task: None,
            recovery_mode: None,
            recovery_candidate: None,
            recovery_session: None,
            secret_input,
            password_input,
            confirmation_input,
            display_name_input,
            local_profile: None,
            profile_message: None,
        });
        section.update(cx, |section, cx| section.start_inspect(window, cx));
        section
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.controller
            .durable()
            .is_some_and(|inspection| inspection.custody.state == CustodyState::Ready)
    }

    pub(crate) fn clear_transient_state(&mut self, cx: &mut Context<Self>) {
        self.controller.cancel();
        self.operation_task = None;
        self.recovery_mode = None;
        self.recovery_candidate = None;
        self.recovery_session = None;
        self.secret_input.update(cx, |input, cx| input.clear(cx));
        self.password_input.update(cx, |input, cx| input.clear(cx));
        self.confirmation_input
            .update(cx, |input, cx| input.clear(cx));
        cx.notify();
    }

    pub(crate) fn deactivate_and_reinspect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.controller.cancel();
        self.controller.forget_durable();
        let previous_operation = self.operation_task.take();
        self.clear_recovery_material(cx);

        let token = self.controller.replace(IdentityOperation::Inspect);
        let backend = self.backend.clone();
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(previous_operation) = previous_operation {
                previous_operation.await;
            }
            let result = cx
                .background_spawn(async move { backend.inspect() })
                .await
                .map_err(map_custody_error);
            this.update_in(cx, |this, window, cx| {
                this.apply_inspection_result(&token, result, window, cx);
            })
            .log_err();
        }));
        cx.notify();
    }

    fn next_receipt(prefix: &str) -> Result<ReceiptRef, IdentityUiError> {
        ReceiptRef::new(format!("{prefix}-{}", Uuid::new_v4().simple()))
            .map_err(|_| IdentityUiError::OperationFailed)
    }

    fn start_inspect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let token = self.controller.replace(IdentityOperation::Inspect);
        let backend = self.backend.clone();
        let background = cx.background_spawn(async move { backend.inspect() });
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = background.await.map_err(map_custody_error);
            this.update_in(cx, |this, window, cx| {
                this.apply_inspection_result(&token, result, window, cx);
            })
            .log_err();
        }));
        cx.notify();
    }

    fn apply_inspection_result(
        &mut self,
        token: &OperationToken,
        result: Result<IdentityInspection, IdentityUiError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.controller.apply(token, result) {
            self.clear_recovery_material(cx);
            self.load_local_profile(window, cx);
            cx.notify();
        }
    }

    fn start_inspection_operation(
        &mut self,
        operation: IdentityOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
        run: impl FnOnce(Arc<dyn IdentityBackend>) -> Result<IdentityInspection, CustodyError>
        + Send
        + 'static,
    ) {
        let Some(token) = self.controller.begin(operation) else {
            return;
        };
        let backend = self.backend.clone();
        let background = cx.background_spawn(async move { run(backend) });
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = background.await.map_err(map_custody_error);
            this.update_in(cx, |this, window, cx| {
                this.apply_inspection_result(&token, result, window, cx);
            })
            .log_err();
        }));
        cx.notify();
    }

    fn start_custody_operation(
        &mut self,
        operation: IdentityOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
        run: impl FnOnce(Arc<dyn IdentityBackend>) -> Result<CustodyResult, CustodyError>
        + Send
        + 'static,
    ) {
        let Some(token) = self.controller.begin(operation) else {
            return;
        };
        let backend = self.backend.clone();
        let background = cx.background_spawn(async move { run(backend) });
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = background.await.map_err(map_custody_error);
            this.update_in(cx, |this, _, cx| {
                if this.controller.apply_custody(&token, result) {
                    this.clear_recovery_material(cx);
                    cx.notify();
                }
            })
            .log_err();
        }));
        cx.notify();
    }

    fn clear_recovery_material(&mut self, cx: &mut Context<Self>) {
        self.recovery_mode = None;
        self.recovery_candidate = None;
        self.recovery_session = None;
        self.secret_input.update(cx, |input, cx| input.clear(cx));
        self.password_input.update(cx, |input, cx| input.clear(cx));
        self.confirmation_input
            .update(cx, |input, cx| input.clear(cx));
    }

    fn load_local_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(identity_ref) = self
            .controller
            .durable()
            .and_then(|inspection| inspection.custody.identity.as_ref())
            .map(|identity| identity.identity_ref().clone())
        else {
            self.local_profile = None;
            self.display_name_input
                .update(cx, |input, cx| input.clear(window, cx));
            return;
        };

        let profile = KeyValueStore::global(cx)
            .read_kvp(&LocalIdentityProfile::kvp_key(&identity_ref))
            .ok()
            .flatten()
            .and_then(|json| {
                LocalIdentityProfile::from_canonical_json(&json, &identity_ref).log_err()
            })
            .unwrap_or_else(|| LocalIdentityProfile::new(identity_ref));
        let display_name = profile.display_name().unwrap_or_default().to_string();
        self.display_name_input
            .update(cx, |input, cx| input.set_text(&display_name, window, cx));
        self.local_profile = Some(profile);
    }

    fn handle_action(
        &mut self,
        action: IdentityAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            IdentityAction::Create => {
                let Ok(receipt_ref) = Self::next_receipt("omega-onboarding-create") else {
                    self.controller
                        .report_error(IdentityUiError::OperationFailed);
                    cx.notify();
                    return;
                };
                self.start_inspection_operation(
                    IdentityOperation::Create {
                        receipt_ref: receipt_ref.clone(),
                    },
                    window,
                    cx,
                    move |backend| backend.create(receipt_ref),
                );
            }
            IdentityAction::Recover => {
                self.controller.cancel();
                self.clear_recovery_material(cx);
                self.recovery_mode = Some(RecoveryMode::Choose);
                cx.notify();
            }
            IdentityAction::Retry => self.start_inspect(window, cx),
            IdentityAction::ResumeCreate => self.start_inspection_operation(
                IdentityOperation::ResumeIncomplete,
                window,
                cx,
                |backend| backend.resume_incomplete_create(),
            ),
            IdentityAction::RetryReset => self.start_custody_operation(
                IdentityOperation::ResumeReset,
                window,
                cx,
                |backend| backend.resume_reset(),
            ),
            IdentityAction::Reset => self.confirm_reset(window, cx),
            IdentityAction::Relaunch => cx.restart(),
            IdentityAction::Protect => {
                self.controller.cancel();
                self.clear_recovery_material(cx);
                self.recovery_mode = Some(RecoveryMode::Protect);
                cx.notify();
            }
        }
    }

    fn confirm_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(identity_ref) = self
            .controller
            .durable()
            .and_then(|inspection| inspection.custody.identity.as_ref())
            .map(|identity| identity.identity_ref().clone())
        else {
            return;
        };
        let Ok(receipt_ref) = Self::next_receipt("omega-onboarding-reset") else {
            self.controller
                .report_error(IdentityUiError::OperationFailed);
            cx.notify();
            return;
        };
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let response = cx.prompt(
                gpui::PromptLevel::Critical,
                "Reset this Omega identity?",
                Some(
                    "Reset removes the signing key from secure custody. Continue only if you have a verified encrypted recovery file.",
                ),
                &["Reset identity", "Cancel"],
            );
            if response.await != Ok(0) {
                return;
            }
            this.update_in(cx, |this, window, cx| {
                this.start_custody_operation(
                    IdentityOperation::Reset {
                        receipt_ref: receipt_ref.clone(),
                    },
                    window,
                    cx,
                    move |backend| backend.reset(&identity_ref, receipt_ref),
                );
            })
            .log_err();
        }));
    }

    fn choose_recovery_artifact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(token) = self.controller.begin(IdentityOperation::PrepareRecovery) else {
            return;
        };
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose an encrypted Omega recovery file".into()),
        });
        let backend = self.backend.clone();
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let path = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let Some(path) = path else {
                this.update(cx, |this, cx| {
                    if this.controller.cancel_if_current(&token) {
                        cx.notify();
                    }
                })
                .log_err();
                return;
            };
            let discovery = cx
                .background_spawn(async move { backend.discover_artifact(path) })
                .await
                .map_err(map_custody_error);
            this.update(cx, |this, cx| match discovery {
                Ok(candidate) if this.controller.accept(&token) => {
                    this.recovery_candidate = Some(candidate);
                    this.recovery_mode = Some(RecoveryMode::ArtifactPassword);
                    cx.notify();
                }
                Err(error) if this.controller.reject(&token, error) => {
                    this.clear_recovery_material(cx);
                    this.recovery_mode = Some(RecoveryMode::Choose);
                    cx.notify();
                }
                _ => {}
            })
            .log_err();
        }));
        cx.notify();
    }

    fn submit_advanced_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw_secret = self.secret_input.update(cx, |input, cx| input.take(cx));
        let imported_secret = match ImportedSecret::new(raw_secret) {
            Ok(secret) => secret,
            Err(_) => {
                let token = self.controller.replace(IdentityOperation::PrepareRecovery);
                self.controller
                    .reject(&token, IdentityUiError::InvalidSecret);
                cx.notify();
                return;
            }
        };
        let existing = self
            .recovery_session
            .take()
            .map(|session| session.prepared)
            .unwrap_or_default();
        let Some(token) = self.controller.begin(IdentityOperation::PrepareRecovery) else {
            return;
        };
        let backend = self.backend.clone();
        let background = cx.background_spawn(async move {
            let mut prepared = existing;
            prepared.push(backend.prepare_import(imported_secret)?);
            let resolution = backend.reconcile(&prepared.iter().collect::<Vec<_>>());
            Ok::<_, CustodyError>(RecoverySession {
                prepared,
                resolution,
            })
        });
        self.await_recovery_session(token, background, window, cx);
    }

    fn submit_artifact_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw_password = self.password_input.update(cx, |input, cx| input.take(cx));
        let password = match RecoveryPassword::new(raw_password) {
            Ok(password) => password,
            Err(_) => {
                let token = self.controller.replace(IdentityOperation::PrepareRecovery);
                self.controller
                    .reject(&token, IdentityUiError::InvalidPassword);
                cx.notify();
                return;
            }
        };
        let Some(candidate) = self.recovery_candidate.take() else {
            return;
        };
        let existing = self
            .recovery_session
            .take()
            .map(|session| session.prepared)
            .unwrap_or_default();
        let Some(token) = self.controller.begin(IdentityOperation::PrepareRecovery) else {
            return;
        };
        let backend = self.backend.clone();
        let background = cx.background_spawn(async move {
            let mut prepared = existing;
            prepared.push(backend.prepare_artifact(&candidate, password)?);
            let resolution = backend.reconcile(&prepared.iter().collect::<Vec<_>>());
            Ok::<_, CustodyError>(RecoverySession {
                prepared,
                resolution,
            })
        });
        self.await_recovery_session(token, background, window, cx);
    }

    fn await_recovery_session(
        &mut self,
        token: OperationToken,
        background: Task<Result<RecoverySession, CustodyError>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = background.await.map_err(map_custody_error);
            this.update(cx, |this, cx| match result {
                Ok(session) if this.controller.accept(&token) => {
                    this.recovery_session = Some(session);
                    this.recovery_mode = Some(RecoveryMode::Review);
                    cx.notify();
                }
                Err(error) if this.controller.reject(&token, error) => {
                    this.clear_recovery_material(cx);
                    this.recovery_mode = Some(RecoveryMode::Choose);
                    cx.notify();
                }
                _ => {}
            })
            .log_err();
        }));
        cx.notify();
    }

    fn adopt_candidate(
        &mut self,
        candidate_ref: CandidateRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.recovery_session.take() else {
            return;
        };
        let resolving_conflict = self.controller.durable().is_some_and(|inspection| {
            inspection.custody.state == CustodyState::Conflict
                || inspection
                    .pending_transaction
                    .as_ref()
                    .is_some_and(|pending| {
                        pending.operation == PendingIdentityOperation::ResolveConflict
                    })
        });
        let receipt_ref = self
            .controller
            .durable()
            .and_then(|inspection| inspection.pending_transaction.as_ref())
            .filter(|pending| {
                pending.operation
                    == if resolving_conflict {
                        PendingIdentityOperation::ResolveConflict
                    } else {
                        PendingIdentityOperation::Import
                    }
            })
            .map(|pending| pending.receipt_ref.clone())
            .map(Ok)
            .unwrap_or_else(|| Self::next_receipt("omega-onboarding-recover"));
        let Ok(receipt_ref) = receipt_ref else {
            self.controller
                .report_error(IdentityUiError::OperationFailed);
            cx.notify();
            return;
        };
        let operation = if resolving_conflict {
            IdentityOperation::ResolveConflict {
                receipt_ref: receipt_ref.clone(),
            }
        } else {
            IdentityOperation::AdoptRecovery {
                receipt_ref: receipt_ref.clone(),
            }
        };
        self.start_inspection_operation(operation, window, cx, move |backend| {
            if resolving_conflict {
                backend.resolve_conflict(session.prepared, &candidate_ref, receipt_ref)
            } else {
                backend.adopt(session.prepared, &candidate_ref, receipt_ref)
            }
        });
    }

    fn export_recovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut raw_password =
            Zeroizing::new(self.password_input.update(cx, |input, cx| input.take(cx)));
        let confirmation = Zeroizing::new(
            self.confirmation_input
                .update(cx, |input, cx| input.take(cx)),
        );
        if raw_password != confirmation {
            let token = self.controller.replace(IdentityOperation::ExportRecovery);
            self.controller
                .reject(&token, IdentityUiError::InvalidPassword);
            cx.notify();
            return;
        }
        let password = match RecoveryPassword::new(std::mem::take(&mut *raw_password)) {
            Ok(password) => password,
            Err(_) => {
                let token = self.controller.replace(IdentityOperation::ExportRecovery);
                self.controller
                    .reject(&token, IdentityUiError::InvalidPassword);
                cx.notify();
                return;
            }
        };
        let Some(identity_ref) = self
            .controller
            .durable()
            .and_then(|inspection| inspection.custody.identity.as_ref())
            .map(|identity| identity.identity_ref().clone())
        else {
            return;
        };
        let Some(token) = self.controller.begin(IdentityOperation::ExportRecovery) else {
            return;
        };
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a folder for the encrypted recovery file".into()),
        });
        let backend = self.backend.clone();
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let directory = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let Some(directory) = directory else {
                this.update(cx, |this, cx| {
                    if this.controller.cancel_if_current(&token) {
                        cx.notify();
                    }
                })
                .log_err();
                return;
            };
            let path = directory.join("omega-identity-recovery.ncryptsec");
            let result = cx
                .background_spawn(
                    async move { backend.export_artifact(&identity_ref, path, password) },
                )
                .await
                .map_err(map_custody_error);
            this.update_in(cx, |this, window, cx| {
                this.apply_inspection_result(&token, result, window, cx);
            })
            .log_err();
        }));
        cx.notify();
    }

    fn save_local_profile(&mut self, cx: &mut Context<Self>) {
        let Some(mut profile) = self.local_profile.clone() else {
            return;
        };
        let display_name = self.display_name_input.read(cx).text(cx);
        let display_name = (!display_name.trim().is_empty()).then_some(display_name);
        if profile.set_display_name(display_name).is_err() {
            self.profile_message = Some("Display name must be 80 characters or fewer.".into());
            cx.notify();
            return;
        }
        let Ok(json) = profile.canonical_json() else {
            self.profile_message = Some("Local profile could not be saved.".into());
            cx.notify();
            return;
        };
        let key = LocalIdentityProfile::kvp_key(profile.identity_ref());
        let kvp = KeyValueStore::global(cx);
        let write = cx.background_spawn(async move { kvp.write_kvp(key, json).await });
        self.profile_task = Some(cx.spawn(async move |this, cx| {
            let result = write.await;
            this.update(cx, |this, cx| {
                this.profile_message = Some(if let Err(error) = result {
                    zlog::error!("failed to save local identity profile: {error:#}");
                    "Local profile could not be saved.".into()
                } else {
                    "Saved locally. Nothing was published.".into()
                });
                cx.notify();
            })
            .log_err();
        }));
        self.local_profile = Some(profile);
        self.profile_message = Some("Saving locally…".into());
        cx.notify();
    }

    fn choose_local_avatar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(identity_ref) = self
            .local_profile
            .as_ref()
            .map(|profile| profile.identity_ref().clone())
        else {
            return;
        };
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a local profile image".into()),
        });
        self.profile_task = Some(cx.spawn_in(window, async move |this, cx| {
            let source = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let Some(source) = source else {
                return;
            };
            let result = cx
                .background_spawn(async move { install_local_avatar(source) })
                .await;
            this.update(cx, |this, cx| {
                if !profile_matches_identity(this.local_profile.as_ref(), &identity_ref) {
                    return;
                }
                match result {
                    Ok(avatar_reference) => {
                        if let Some(profile) = &mut this.local_profile {
                            if profile.set_avatar_reference(Some(avatar_reference)).is_ok() {
                                this.profile_message = Some(
                                    "Avatar selected. Save the local profile to keep it.".into(),
                                );
                            } else {
                                this.profile_message =
                                    Some("The local avatar reference was invalid.".into());
                            }
                        }
                    }
                    Err(error) => {
                        zlog::error!("failed to install local identity avatar: {error}");
                        this.profile_message =
                            Some("The selected avatar could not be stored locally.".into());
                    }
                }
                cx.notify();
            })
            .log_err();
        }));
    }

    fn presentation(&self) -> IdentityPresentation {
        if let Some(operation) = self.controller.operation() {
            let (title, description) = match operation {
                IdentityOperation::Inspect => (
                    "Checking secure identity…",
                    "Omega is reading public identity facts and secure-custody availability.",
                ),
                IdentityOperation::Create { .. } => (
                    "Creating your identity…",
                    "Omega is creating one Nostr identity, installing it in secure custody, and verifying read-back.",
                ),
                IdentityOperation::ResumeIncomplete => (
                    "Resuming identity setup…",
                    "Omega is finishing the durable transaction without rotating the known identity.",
                ),
                IdentityOperation::PrepareRecovery => (
                    "Checking recovery candidate…",
                    "Omega is deriving public identity details in the background. No secret is shown or published.",
                ),
                IdentityOperation::AdoptRecovery { .. }
                | IdentityOperation::ResolveConflict { .. } => (
                    "Recovering your identity…",
                    "Omega is installing the explicitly selected identity in secure custody.",
                ),
                IdentityOperation::ExportRecovery => (
                    "Protecting recovery…",
                    "Omega is creating and verifying an encrypted NIP-49 recovery file.",
                ),
                IdentityOperation::Reset { .. } | IdentityOperation::ResumeReset => (
                    "Verifying identity reset…",
                    "Omega remains blocked until secure deletion and public-state cleanup are verified.",
                ),
            };
            return IdentityPresentation {
                title,
                description,
                icon: IconName::Lock,
                color: Color::Accent,
                actions: Vec::new(),
            };
        }

        let Some(inspection) = self.controller.durable() else {
            return IdentityPresentation {
                title: "Checking secure identity",
                description: "Omega has not established a durable identity fact yet.",
                icon: IconName::Lock,
                color: Color::Muted,
                actions: vec![IdentityAction::Retry],
            };
        };

        Self::durable_presentation(inspection)
    }

    fn error_message(&self) -> Option<&'static str> {
        self.controller.error().map(|error| match error {
            IdentityUiError::InvalidSecret => {
                "That secret key is invalid. The field was cleared; paste it again to retry."
            }
            IdentityUiError::InvalidPassword => {
                "The password was invalid or did not match. Both password fields were cleared."
            }
            IdentityUiError::UnsafeRecoveryFile => {
                "That recovery file is invalid or does not meet Omega's local safety checks."
            }
            IdentityUiError::RecoveryFileExists => {
                "The destination already contains an Omega recovery file. Choose another folder."
            }
            IdentityUiError::SecureStorageUnavailable => {
                "Secure identity storage is unavailable. Unlock it and try again."
            }
            IdentityUiError::OperationFailed => {
                "Identity setup did not finish. Omega kept the last durable state unchanged."
            }
        })
    }

    fn ready_identity(&self) -> Option<&PublicIdentity> {
        self.controller
            .durable()
            .filter(|inspection| inspection.custody.state == CustodyState::Ready)
            .and_then(|inspection| inspection.custody.identity.as_ref())
    }

    fn durable_presentation(inspection: &IdentityInspection) -> IdentityPresentation {
        match inspection.custody.state {
            CustodyState::ResetFailed => IdentityPresentation {
                title: "Reset didn't finish",
                description: "Omega kept identity setup blocked so the previous identity is not silently replaced.",
                icon: IconName::Warning,
                color: Color::Error,
                actions: vec![IdentityAction::RetryReset, IdentityAction::Relaunch],
            },
            CustodyState::Locked => IdentityPresentation {
                title: "System keychain locked",
                description: "Unlock the system keychain before Omega checks or uses your identity.",
                icon: IconName::Lock,
                color: Color::Warning,
                actions: vec![IdentityAction::Retry],
            },
            CustodyState::RelaunchRequired => IdentityPresentation {
                title: "Relaunch required",
                description: "Identity maintenance finished safely. Relaunch Omega to continue.",
                icon: IconName::Info,
                color: Color::Accent,
                actions: vec![IdentityAction::Relaunch],
            },
            CustodyState::Conflict => {
                let ambiguous = inspection.conflict.as_ref().is_some_and(|conflict| {
                    conflict.reason == CustodyConflictReason::AmbiguousSecureStore
                });
                IdentityPresentation {
                    title: if ambiguous {
                        "Secure storage needs attention"
                    } else {
                        "Identity choice required"
                    },
                    description: if ambiguous {
                        "The system keychain returned more than one matching credential. Omega will not guess which one to use."
                    } else {
                        "Public identity facts disagree. Recover the identity you own or explicitly reset after verifying a backup."
                    },
                    icon: IconName::Warning,
                    color: Color::Warning,
                    actions: if ambiguous {
                        vec![IdentityAction::Retry]
                    } else {
                        vec![IdentityAction::Recover]
                    },
                }
            }
            CustodyState::Lost => IdentityPresentation {
                title: "Recovery needed",
                description: "The public identity is known, but its signing key is not available in secure custody.",
                icon: IconName::LockOff,
                color: Color::Error,
                actions: vec![IdentityAction::Recover, IdentityAction::Reset],
            },
            CustodyState::Incomplete => {
                let pending_create = inspection
                    .pending_transaction
                    .as_ref()
                    .is_some_and(|pending| pending.operation == PendingIdentityOperation::Create);
                IdentityPresentation {
                    title: "Identity setup needs repair",
                    description: if pending_create {
                        "A prior Create transaction can be resumed with its original durable receipt."
                    } else {
                        "A prior recovery transaction needs the same owner-authorized identity candidate."
                    },
                    icon: IconName::Warning,
                    color: Color::Warning,
                    actions: if pending_create {
                        vec![IdentityAction::ResumeCreate]
                    } else {
                        vec![IdentityAction::Recover]
                    },
                }
            }
            CustodyState::Absent => IdentityPresentation {
                title: "Create your Omega identity",
                description: "Create a local Nostr identity for signed work, portable social context, and agent coordination.",
                icon: IconName::Person,
                color: Color::Accent,
                actions: vec![IdentityAction::Create, IdentityAction::Recover],
            },
            CustodyState::Ready => {
                let needs_recovery =
                    inspection.recovery_protection.state != RecoveryProtectionState::Protected;
                IdentityPresentation {
                    title: "Identity ready",
                    description: if needs_recovery {
                        "Your signing key is in secure local custody. Create an encrypted recovery file before relying on this identity."
                    } else {
                        "Your public identity is ready and an encrypted recovery file has been verified."
                    },
                    icon: IconName::UserCheck,
                    color: if needs_recovery {
                        Color::Warning
                    } else {
                        Color::Success
                    },
                    actions: vec![IdentityAction::Protect],
                }
            }
        }
    }

    fn render_recovery_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(mode) = self.recovery_mode else {
            return div().into_any_element();
        };
        let cancel = Button::new("omega-identity-recovery-cancel", "Cancel")
            .style(ButtonStyle::OutlinedGhost)
            .tab_index(self.first_tab_index + 6)
            .on_click(cx.listener(|this, _, _, cx| {
                this.clear_transient_state(cx);
            }));

        match mode {
            RecoveryMode::Choose => v_flex()
                .gap_2()
                .child(Label::new("Recover an existing identity"))
                .child(
                    Label::new(
                        "Use an encrypted NIP-49 recovery file, or open the advanced secret-key path.",
                    )
                    .color(Color::Muted)
                    .size(LabelSize::Small),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("omega-identity-recovery-file", "Choose recovery file")
                                .style(ButtonStyle::Filled)
                                .tab_index(self.first_tab_index + 2)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose_recovery_artifact(window, cx);
                                })),
                        )
                        .child(
                            Button::new(
                                "omega-identity-recovery-advanced",
                                "Advanced secret import",
                            )
                            .style(ButtonStyle::OutlinedGhost)
                            .tab_index(self.first_tab_index + 3)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.recovery_mode = Some(RecoveryMode::AdvancedImport);
                                cx.notify();
                            })),
                        )
                        .child(cancel),
                )
                .into_any_element(),
            RecoveryMode::AdvancedImport => v_flex()
                .gap_2()
                .child(Label::new("Advanced Nostr secret import"))
                .child(
                    Label::new(
                        "The value stays in a zeroizing field with copy and cut disabled, and is cleared before background validation.",
                    )
                    .color(Color::Muted)
                    .size(LabelSize::Small),
                )
                .child(self.secret_input.clone())
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("omega-identity-preview-secret", "Preview identity")
                                .style(ButtonStyle::Filled)
                                .tab_index(self.first_tab_index + 3)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_advanced_import(window, cx);
                                })),
                        )
                        .child(cancel),
                )
                .into_any_element(),
            RecoveryMode::ArtifactPassword => v_flex()
                .gap_2()
                .child(Label::new("Unlock the selected recovery file"))
                .child(
                    Label::new(
                        "Omega validates file safety and derives only public identity details for review.",
                    )
                    .color(Color::Muted)
                    .size(LabelSize::Small),
                )
                .child(self.password_input.clone())
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("omega-identity-preview-artifact", "Preview identity")
                                .style(ButtonStyle::Filled)
                                .tab_index(self.first_tab_index + 3)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_artifact_password(window, cx);
                                })),
                        )
                        .child(cancel),
                )
                .into_any_element(),
            RecoveryMode::Review => {
                let candidates = self
                    .recovery_session
                    .as_ref()
                    .map(|session| session.resolution.candidates.clone())
                    .unwrap_or_default();
                let requires_selection = self.recovery_session.as_ref().is_some_and(|session| {
                    session.resolution.state == RecoveryResolutionState::OwnerSelectionRequired
                });
                v_flex()
                    .gap_2()
                    .child(Label::new(if requires_selection {
                        "Choose the identity you own"
                    } else {
                        "Confirm recovered identity"
                    }))
                    .child(
                        Label::new(if requires_selection {
                            "The candidates resolve to different public identities. Omega will not choose for you."
                        } else {
                            "Only public identity details are shown. The secret remains opaque and will be consumed on adoption."
                        })
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                    )
                    .children(candidates.into_iter().enumerate().map(|(index, candidate)| {
                        let candidate_ref = candidate.candidate_ref.clone();
                        let npub = candidate.identity.npub().as_str().to_string();
                        let fingerprint = candidate.identity.fingerprint().display();
                        h_flex()
                            .min_w_0()
                            .gap_2()
                            .justify_between()
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(
                                        Label::new(wrapping_public_identity(&npub))
                                            .size(LabelSize::Small),
                                    )
                                    .child(
                                        Label::new(format!("Fingerprint {fingerprint}"))
                                            .color(Color::Muted)
                                            .size(LabelSize::XSmall),
                                    ),
                            )
                            .child(
                                Button::new(
                                    format!(
                                        "omega-identity-adopt-{}",
                                        candidate_ref.as_str()
                                    ),
                                    "Use this identity",
                                )
                                .style(ButtonStyle::Filled)
                                .tab_index(self.first_tab_index + 2 + index as isize)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.adopt_candidate(candidate_ref.clone(), window, cx);
                                })),
                            )
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(
                                    "omega-identity-add-recovery",
                                    "Add another recovery file",
                                )
                                .style(ButtonStyle::OutlinedGhost)
                                .tab_index(self.first_tab_index + 5)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose_recovery_artifact(window, cx);
                                })),
                            )
                            .child(cancel),
                    )
                    .into_any_element()
            }
            RecoveryMode::Protect => v_flex()
                .gap_2()
                .child(Label::new("Create encrypted recovery"))
                .child(
                    Label::new(
                        "Choose a strong password. Omega writes a standard NIP-49 file and never displays the raw secret.",
                    )
                    .color(Color::Muted)
                    .size(LabelSize::Small),
                )
                .child(self.password_input.clone())
                .child(self.confirmation_input.clone())
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(
                                "omega-identity-export-recovery",
                                "Choose folder and save",
                            )
                            .style(ButtonStyle::Filled)
                            .tab_index(self.first_tab_index + 4)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.export_recovery(window, cx);
                            })),
                        )
                        .child(cancel),
                )
                .into_any_element(),
        }
    }

    fn render_local_profile(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.ready_identity().is_none() {
            return div().into_any_element();
        }
        let avatar_status = if self
            .local_profile
            .as_ref()
            .and_then(LocalIdentityProfile::avatar_reference)
            .is_some()
        {
            "Local avatar selected"
        } else {
            "No local avatar selected"
        };
        v_flex()
            .gap_2()
            .child(Label::new("Local profile"))
            .child(
                Label::new(
                    "Display name and avatar stay on this device. Omega does not publish a Nostr kind 0 profile.",
                )
                .color(Color::Muted)
                .size(LabelSize::Small),
            )
            .child(self.display_name_input.clone())
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("omega-identity-choose-avatar", "Choose local avatar")
                            .style(ButtonStyle::OutlinedGhost)
                            .tab_index(self.first_tab_index + 9)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_local_avatar(window, cx);
                            })),
                    )
                    .child(
                        Button::new("omega-identity-save-profile", "Save locally")
                            .style(ButtonStyle::OutlinedGhost)
                            .tab_index(self.first_tab_index + 10)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_local_profile(cx);
                            })),
                    ),
            )
            .child(
                Label::new(avatar_status)
                    .color(Color::Muted)
                    .size(LabelSize::XSmall),
            )
            .when_some(self.profile_message.clone(), |this, message| {
                this.child(
                    Label::new(message)
                        .color(Color::Muted)
                        .size(LabelSize::XSmall),
                )
            })
            .into_any_element()
    }
}

fn map_custody_error(error: CustodyError) -> IdentityUiError {
    match error {
        CustodyError::InvalidImportedSecret => IdentityUiError::InvalidSecret,
        CustodyError::RecoveryDecryptionFailed => IdentityUiError::InvalidPassword,
        CustodyError::InvalidRecoveryArtifact => IdentityUiError::UnsafeRecoveryFile,
        CustodyError::RecoveryArtifactExists => IdentityUiError::RecoveryFileExists,
        CustodyError::SecureStoreUnavailable | CustodyError::MutationLock => {
            IdentityUiError::SecureStorageUnavailable
        }
        _ => IdentityUiError::OperationFailed,
    }
}

fn profile_matches_identity(
    profile: Option<&LocalIdentityProfile>,
    expected_identity_ref: &IdentityRef,
) -> bool {
    profile.is_some_and(|profile| profile.identity_ref() == expected_identity_ref)
}

fn wrapping_public_identity(value: &str) -> String {
    let mut display = String::with_capacity(value.len() + value.len() / 12);
    for (index, character) in value.chars().enumerate() {
        if index > 0 && index % 12 == 0 {
            display.push('\u{200b}');
        }
        display.push(character);
    }
    display
}

impl Render for IdentitySection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let presentation = self.presentation();
        let public_identities = self
            .controller
            .durable()
            .map(|inspection| {
                inspection
                    .conflict
                    .as_ref()
                    .map(|conflict| conflict.identities.clone())
                    .filter(|identities| !identities.is_empty())
                    .unwrap_or_else(|| inspection.custody.identity.clone().into_iter().collect())
            })
            .unwrap_or_default();
        let error_message = self.error_message();

        v_flex()
            .min_w_0()
            .gap_3()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(Label::new("Your identity"))
                    .child(
                        Label::new(
                            "Omega uses a Nostr key pair as your portable public identity. Your private key stays in secure local custody.",
                        )
                        .color(Color::Muted),
                    ),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .gap_2()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_2()
                            .child(
                                Icon::new(presentation.icon)
                                    .size(IconSize::Small)
                                    .color(presentation.color),
                            )
                            .child(Label::new(presentation.title)),
                    )
                    .child(
                        Label::new(presentation.description)
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                    )
                    .children(public_identities.into_iter().enumerate().map(
                        |(index, identity)| {
                            v_flex()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    Label::new(if index == 0 {
                                        "Public identity"
                                    } else {
                                        "Conflicting public identity"
                                    })
                                    .color(Color::Muted)
                                    .size(LabelSize::XSmall),
                                )
                                .child(
                                    Label::new(wrapping_public_identity(identity.npub().as_str()))
                                        .size(LabelSize::Small),
                                )
                                .child(
                                    Label::new(format!(
                                        "Fingerprint {}",
                                        identity.fingerprint().display()
                                    ))
                                    .color(Color::Muted)
                                    .size(LabelSize::XSmall),
                                )
                        },
                    ))
                    .when_some(error_message, |this, message| {
                        this.child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::Warning)
                                        .size(IconSize::Small)
                                        .color(Color::Error),
                                )
                                .child(
                                    Label::new(message)
                                        .color(Color::Error)
                                        .size(LabelSize::Small),
                                ),
                        )
                    })
                    .when(!presentation.actions.is_empty(), |this| {
                        this.child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .children(presentation.actions.into_iter().enumerate().map(
                                    |(index, action)| {
                                        Button::new(
                                            format!("omega-identity-{}", action.id()),
                                            action.label(),
                                        )
                                        .style(if action.primary() {
                                            ButtonStyle::Filled
                                        } else {
                                            ButtonStyle::OutlinedGhost
                                        })
                                        .size(ButtonSize::Medium)
                                        .tab_index(self.first_tab_index + index as isize)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.handle_action(action, window, cx);
                                        }))
                                    },
                                )),
                        )
                    })
                    .when(self.recovery_mode.is_some(), |this| {
                        this.child(self.render_recovery_controls(cx))
                    })
                    .when(self.ready_identity().is_some(), |this| {
                        this.child(self.render_local_profile(cx))
                    }),
            )
    }
}

pub(crate) fn render_identity_section(
    tab_index: &mut isize,
    section: &Entity<IdentitySection>,
) -> impl IntoElement {
    *tab_index += IDENTITY_TAB_SLOTS;
    section.clone()
}

#[cfg(test)]
mod tests {
    use omega_identity::{CustodyConflict, PendingIdentityTransaction, RecoveryProtectionStatus};

    use super::*;

    fn inspection(state: CustodyState) -> IdentityInspection {
        IdentityInspection {
            custody: CustodyResult {
                state,
                identity: None,
                receipt_ref: None,
            },
            pending_transaction: None,
            conflict: None,
            recovery_protection: RecoveryProtectionStatus {
                state: RecoveryProtectionState::NotApplicable,
                record: None,
            },
        }
    }

    #[test]
    fn public_identity_has_safe_narrow_window_breaks() {
        let npub = "npub1lpkyfgk7jhv3fx63c63f4l4thgnycx8zlf7ynh5ngf9qc455w7zs7s8hua";
        assert_eq!(wrapping_public_identity(npub).replace('\u{200b}', ""), npub);
    }

    #[test]
    fn identity_reserves_focus_before_theme() {
        assert!(IDENTITY_TAB_SLOTS >= 10);
    }

    #[test]
    fn every_durable_custody_fact_has_an_explicit_presentation() {
        let cases = [
            (CustodyState::Absent, "Create your Omega identity"),
            (CustodyState::Ready, "Identity ready"),
            (CustodyState::Locked, "System keychain locked"),
            (CustodyState::Incomplete, "Identity setup needs repair"),
            (CustodyState::Lost, "Recovery needed"),
            (CustodyState::Conflict, "Identity choice required"),
            (CustodyState::ResetFailed, "Reset didn't finish"),
            (CustodyState::RelaunchRequired, "Relaunch required"),
        ];
        for (state, title) in cases {
            assert_eq!(
                IdentitySection::durable_presentation(&inspection(state)).title,
                title
            );
        }
    }

    #[test]
    fn incomplete_create_uses_the_durable_resume_path() {
        let mut inspection = inspection(CustodyState::Incomplete);
        inspection.pending_transaction = Some(PendingIdentityTransaction {
            operation: PendingIdentityOperation::Create,
            receipt_ref: ReceiptRef::new("original-create").expect("valid receipt"),
            expected_identity: None,
        });
        assert_eq!(
            IdentitySection::durable_presentation(&inspection).actions,
            vec![IdentityAction::ResumeCreate]
        );
    }

    #[test]
    fn ambiguous_keychain_conflict_never_offers_fake_owner_selection() {
        let mut inspection = inspection(CustodyState::Conflict);
        inspection.conflict = Some(CustodyConflict {
            reason: CustodyConflictReason::AmbiguousSecureStore,
            identities: Vec::new(),
        });
        let presentation = IdentitySection::durable_presentation(&inspection);
        assert_eq!(presentation.title, "Secure storage needs attention");
        assert_eq!(presentation.actions, vec![IdentityAction::Retry]);
    }

    #[test]
    fn public_identity_mismatch_requires_proven_recovery_selection() {
        let mut inspection = inspection(CustodyState::Conflict);
        inspection.conflict = Some(CustodyConflict {
            reason: CustodyConflictReason::PublicManifestCustodyMismatch,
            identities: Vec::new(),
        });
        let presentation = IdentitySection::durable_presentation(&inspection);
        assert_eq!(presentation.title, "Identity choice required");
        assert_eq!(presentation.actions, vec![IdentityAction::Recover]);
    }

    #[test]
    fn ready_without_a_durable_recovery_fact_stays_visibly_unprotected() {
        let mut inspection = inspection(CustodyState::Ready);
        inspection.recovery_protection = RecoveryProtectionStatus {
            state: RecoveryProtectionState::Needed,
            record: None,
        };
        let presentation = IdentitySection::durable_presentation(&inspection);
        assert!(presentation.description.contains("encrypted recovery"));
        assert_eq!(presentation.actions, vec![IdentityAction::Protect]);
    }

    #[test]
    fn avatar_completion_is_fenced_to_the_identity_that_started_it() {
        let first_identity = IdentityRef::new("omega-first").expect("valid identity");
        let second_identity = IdentityRef::new("omega-second").expect("valid identity");
        let current_profile = LocalIdentityProfile::new(second_identity);

        assert!(!profile_matches_identity(
            Some(&current_profile),
            &first_identity
        ));
        assert!(!profile_matches_identity(None, &first_identity));
    }
}
