use std::sync::Arc;

use futures::{FutureExt as _, channel::oneshot, future::Shared};
use gpui::{App, AppContext as _, AsyncApp, Global, Task};
use omega_identity::{CustodyError, CustodyState, IdentityInspection, IdentityService};
use workspace::AppState;

use crate::show_onboarding_view;

type StartupInspection = Result<IdentityInspection, Arc<CustodyError>>;
type StartupCompletion = Result<(), Arc<anyhow::Error>>;

trait IdentityStartupBackend: Send + Sync {
    fn inspect_for_process_start(&self) -> Result<IdentityInspection, CustodyError>;
}

struct SystemIdentityStartupBackend {
    service: IdentityService,
}

impl SystemIdentityStartupBackend {
    fn new() -> Self {
        Self {
            service: IdentityService::system(*app_identity::CHANNEL),
        }
    }
}

impl IdentityStartupBackend for SystemIdentityStartupBackend {
    fn inspect_for_process_start(&self) -> Result<IdentityInspection, CustodyError> {
        self.service.inspect_for_process_start()
    }
}

struct IdentityStartupCoordinator {
    inspection: Shared<Task<StartupInspection>>,
    completion: Shared<Task<StartupCompletion>>,
    completion_sender: Option<oneshot::Sender<StartupCompletion>>,
    onboarding_open: bool,
    terminal: Option<StartupCompletion>,
}

impl Global for IdentityStartupCoordinator {}

impl IdentityStartupCoordinator {
    fn install(cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }
        Self::install_with_backend(Arc::new(SystemIdentityStartupBackend::new()), cx);
    }

    fn install_with_backend(backend: Arc<dyn IdentityStartupBackend>, cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }

        let inspection = cx
            .background_spawn(async move { backend.inspect_for_process_start().map_err(Arc::new) })
            .shared();
        let (completion_sender, completion_receiver) = oneshot::channel();
        let completion = cx
            .spawn(async move |_| match completion_receiver.await {
                Ok(completion) => completion,
                Err(_) => Err(Arc::new(anyhow::anyhow!(
                    "identity startup completion channel closed before release"
                ))),
            })
            .shared();

        cx.set_global(Self {
            inspection,
            completion,
            completion_sender: Some(completion_sender),
            onboarding_open: false,
            terminal: None,
        });
    }

    fn claim_onboarding(&mut self) -> bool {
        if self.terminal.is_some() || self.onboarding_open {
            return false;
        }
        self.onboarding_open = true;
        true
    }

    fn onboarding_opened(cx: &mut App) {
        if cx.has_global::<Self>() {
            cx.global_mut::<Self>().onboarding_open = true;
        }
    }

    fn onboarding_closed(cx: &mut App) {
        if cx.has_global::<Self>() {
            cx.global_mut::<Self>().onboarding_open = false;
        }
    }

    fn finish(completion: StartupCompletion, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        let coordinator = cx.global_mut::<Self>();
        if coordinator.terminal.is_some() {
            return;
        }

        coordinator.terminal = Some(completion.clone());
        if let Some(sender) = coordinator.completion_sender.take() {
            if sender.send(completion).is_err() {
                zlog::error!("identity startup waiters disappeared before release");
            }
        }
    }
}

pub async fn await_identity_ready(
    app_state: Arc<AppState>,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let (inspection, completion, terminal) = cx.update(|cx| {
        IdentityStartupCoordinator::install(cx);
        let coordinator = cx.global::<IdentityStartupCoordinator>();
        (
            coordinator.inspection.clone(),
            coordinator.completion.clone(),
            coordinator.terminal.is_some(),
        )
    });
    if terminal {
        return completion
            .await
            .map_err(|error| anyhow::anyhow!("{error:#}"));
    };

    let needs_onboarding = match inspection.await {
        Ok(inspection) => inspection.custody.state != CustodyState::Ready,
        Err(error) => {
            zlog::error!("identity startup inspection failed: {error}");
            true
        }
    };

    if !needs_onboarding {
        cx.update(|cx| IdentityStartupCoordinator::finish(Ok(()), cx));
        return Ok(());
    }

    let should_open = cx.update(|cx| {
        let coordinator = cx.global_mut::<IdentityStartupCoordinator>();
        coordinator.claim_onboarding()
    });
    if should_open {
        let open_task = cx.update(|cx| show_onboarding_view(app_state, cx));
        cx.spawn(async move |cx| {
            if let Err(error) = open_task.await {
                zlog::error!("failed to open identity onboarding: {error:#}");
                cx.update(|cx| {
                    IdentityStartupCoordinator::onboarding_closed(cx);
                    IdentityStartupCoordinator::finish(Err(Arc::new(error)), cx);
                });
            }
        })
        .detach();
    }

    completion
        .await
        .map_err(|error| anyhow::anyhow!("{error:#}"))
}

pub(crate) fn onboarding_opened(cx: &mut App) {
    IdentityStartupCoordinator::onboarding_opened(cx);
}

pub(crate) fn onboarding_closed(cx: &mut App) {
    IdentityStartupCoordinator::onboarding_closed(cx);
}

pub(crate) fn release_identity_waiters(cx: &mut App) {
    IdentityStartupCoordinator::finish(Ok(()), cx);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gpui::TestAppContext;
    use omega_identity::{CustodyResult, RecoveryProtectionState, RecoveryProtectionStatus};

    use super::*;

    struct FakeBackend {
        calls: Arc<AtomicUsize>,
        state: CustodyState,
    }

    impl IdentityStartupBackend for FakeBackend {
        fn inspect_for_process_start(&self) -> Result<IdentityInspection, CustodyError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(IdentityInspection {
                custody: CustodyResult {
                    state: self.state,
                    identity: None,
                    receipt_ref: None,
                },
                pending_transaction: None,
                conflict: None,
                recovery_protection: RecoveryProtectionStatus {
                    state: RecoveryProtectionState::NotApplicable,
                    record: None,
                },
            })
        }
    }

    #[gpui::test]
    fn startup_inspection_is_shared_by_concurrent_callers(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = Arc::new(FakeBackend {
            calls: calls.clone(),
            state: CustodyState::Ready,
        });
        let completed = Arc::new(AtomicUsize::new(0));

        cx.update(|cx| {
            IdentityStartupCoordinator::install_with_backend(backend, cx);
            let inspection = cx.global::<IdentityStartupCoordinator>().inspection.clone();
            for _ in 0..2 {
                let inspection = inspection.clone();
                let completed = completed.clone();
                cx.spawn(async move |_| {
                    inspection.await.expect("startup inspection");
                    completed.fetch_add(1, Ordering::SeqCst);
                })
                .detach();
            }
        });
        cx.run_until_parked();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(completed.load(Ordering::SeqCst), 2);
    }

    #[gpui::test]
    fn release_wakes_every_waiter_and_is_idempotent(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = Arc::new(FakeBackend {
            calls,
            state: CustodyState::Absent,
        });
        let completed = Arc::new(AtomicUsize::new(0));

        cx.update(|cx| {
            IdentityStartupCoordinator::install_with_backend(backend, cx);
            let completion = cx.global::<IdentityStartupCoordinator>().completion.clone();
            for _ in 0..3 {
                let completion = completion.clone();
                let completed = completed.clone();
                cx.spawn(async move |_| {
                    completion.await.expect("startup release");
                    completed.fetch_add(1, Ordering::SeqCst);
                })
                .detach();
            }
            IdentityStartupCoordinator::finish(Ok(()), cx);
            IdentityStartupCoordinator::finish(Ok(()), cx);
        });
        cx.run_until_parked();

        assert_eq!(completed.load(Ordering::SeqCst), 3);
        cx.update(|cx| {
            assert_eq!(
                cx.global::<IdentityStartupCoordinator>()
                    .terminal
                    .as_ref()
                    .map(Result::is_ok),
                Some(true)
            );
        });
    }

    #[gpui::test]
    fn failure_wakes_current_and_future_waiters(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = Arc::new(FakeBackend {
            calls,
            state: CustodyState::Absent,
        });
        let failures = Arc::new(AtomicUsize::new(0));

        cx.update(|cx| {
            IdentityStartupCoordinator::install_with_backend(backend, cx);
            let completion = cx.global::<IdentityStartupCoordinator>().completion.clone();
            for _ in 0..2 {
                let completion = completion.clone();
                let failures = failures.clone();
                cx.spawn(async move |_| {
                    if completion.await.is_err() {
                        failures.fetch_add(1, Ordering::SeqCst);
                    }
                })
                .detach();
            }
            IdentityStartupCoordinator::finish(
                Err(Arc::new(anyhow::anyhow!("failed to open onboarding"))),
                cx,
            );
        });
        cx.run_until_parked();

        cx.update(|cx| {
            let completion = cx.global::<IdentityStartupCoordinator>().completion.clone();
            let failures = failures.clone();
            cx.spawn(async move |_| {
                if completion.await.is_err() {
                    failures.fetch_add(1, Ordering::SeqCst);
                }
            })
            .detach();
        });
        cx.run_until_parked();

        assert_eq!(failures.load(Ordering::SeqCst), 3);
        cx.update(|cx| {
            assert_eq!(
                cx.global::<IdentityStartupCoordinator>()
                    .terminal
                    .as_ref()
                    .map(Result::is_err),
                Some(true)
            );
        });
    }

    #[gpui::test]
    fn closing_onboarding_allows_one_reopen_without_releasing(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = Arc::new(FakeBackend {
            calls,
            state: CustodyState::Absent,
        });

        cx.update(|cx| {
            IdentityStartupCoordinator::install_with_backend(backend, cx);
            let coordinator = cx.global_mut::<IdentityStartupCoordinator>();
            assert!(coordinator.claim_onboarding());
            assert!(!coordinator.claim_onboarding());
            IdentityStartupCoordinator::onboarding_closed(cx);
            assert!(
                cx.global_mut::<IdentityStartupCoordinator>()
                    .claim_onboarding()
            );
            assert!(cx.global::<IdentityStartupCoordinator>().terminal.is_none());
        });
    }
}
