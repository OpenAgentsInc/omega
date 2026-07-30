use std::sync::Arc;

use futures::{FutureExt as _, future::Shared};
use gpui::{App, AppContext as _, AsyncApp, Global, Task};
use omega_identity::{CustodyError, CustodyResult, CustodyState, IdentityService, ReceiptRef};

/// The receipt the silent first-launch provisioning writes into custody.
///
/// omega#164, owner direction 2026-07-29: there is no onboarding flow. The
/// receipt names the startup path so a custody audit can tell a background
/// launch keygen from an owner-clicked create, a hosted-lane provision
/// (`omega-device-pairing-provision-v1`), or a recovery.
const STARTUP_PROVISION_RECEIPT: &str = "omega-first-launch-background-keygen-v1";

type StartupProvision = Result<CustodyResult, Arc<CustodyError>>;

trait IdentityStartupBackend: Send + Sync {
    fn provision_for_process_start(&self) -> Result<CustodyResult, CustodyError>;
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
    fn provision_for_process_start(&self) -> Result<CustodyResult, CustodyError> {
        let receipt_ref = ReceiptRef::new(STARTUP_PROVISION_RECEIPT)
            .map_err(|_| CustodyError::CustodyDenied(CustodyState::Absent))?;
        self.service.provision_for_process_start(receipt_ref)
    }
}

/// One shared background provisioning task for the whole process.
///
/// `OMEGA-DELTA-0040` (amended by omega#164): startup provisions the Nostr
/// identity silently in the background and then opens the front door. Every
/// window path awaits the same shared task, so custody is inspected and
/// provisioned exactly once per launch no matter how many windows race, and no
/// user gesture is ever part of releasing the wait — the `onboarding::Finish`
/// dead-end class is structurally impossible because there is no completion
/// channel for a UI action to forget to complete.
struct IdentityStartupCoordinator {
    provision: Shared<Task<StartupProvision>>,
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

        let provision = cx
            .background_spawn(
                async move { backend.provision_for_process_start().map_err(Arc::new) },
            )
            .shared();
        cx.set_global(Self { provision });
    }
}

/// Install a coordinator whose provisioning succeeds without touching any
/// real profile.
///
/// Dependent crates' tests drive the real startup path
/// (`restore_or_create_workspace`, open requests), and the production backend
/// would inspect — and on `Absent`, create — identity files under the real
/// data root of whoever runs the tests. omega#110's rule holds for the silent
/// gate too: tests fabricate custody state; they never probe or write the
/// owner's. First install wins, so call this before anything awaits the gate.
#[cfg(any(test, feature = "test-support"))]
pub fn install_test_identity_startup(cx: &mut App) {
    struct ReadyWithoutCustody;

    impl IdentityStartupBackend for ReadyWithoutCustody {
        fn provision_for_process_start(&self) -> Result<CustodyResult, CustodyError> {
            Ok(CustodyResult {
                state: CustodyState::Ready,
                identity: None,
                receipt_ref: None,
            })
        }
    }

    IdentityStartupCoordinator::install_with_backend(Arc::new(ReadyWithoutCustody), cx);
}

/// Provision the Nostr identity in the background, then let startup proceed.
///
/// The gate's purpose survives the removed onboarding ceremony: no surface
/// opens before custody has been provisioned or has refused by name. The
/// refusal states (`Lost`, `Conflict`, `Incomplete`, reset) are logged rather
/// than blocking, because a launch that parks forever behind an unattended
/// custody problem is a worse product than a thread whose identity-consuming
/// surfaces refuse with the same named state when touched.
pub async fn await_identity_ready(cx: &mut AsyncApp) -> anyhow::Result<()> {
    let provision = cx.update(|cx| {
        IdentityStartupCoordinator::install(cx);
        cx.global::<IdentityStartupCoordinator>().provision.clone()
    });

    if let Err(error) = provision.await {
        zlog::error!(
            "first-launch identity provisioning refused; opening the front door without a ready identity: {error}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gpui::TestAppContext;
    use omega_identity::{IdentityRef, PublicIdentity};

    use super::*;

    struct FakeBackend {
        calls: Arc<AtomicUsize>,
        outcome: Result<CustodyState, CustodyState>,
    }

    impl IdentityStartupBackend for FakeBackend {
        fn provision_for_process_start(&self) -> Result<CustodyResult, CustodyError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.outcome {
                Ok(state) => Ok(CustodyResult {
                    state,
                    identity: Some(test_identity()),
                    receipt_ref: None,
                }),
                Err(state) => Err(CustodyError::CustodyDenied(state)),
            }
        }
    }

    fn test_identity() -> PublicIdentity {
        // The secp256k1 generator point's x-coordinate: a well-known valid
        // x-only public key that never corresponds to any profile's secret.
        let public_key_hex =
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".to_string();
        PublicIdentity::from_public_key_hex(
            IdentityRef::new(format!("omega-nostr-{public_key_hex}")).expect("valid identity ref"),
            public_key_hex,
        )
        .expect("valid test identity")
    }

    #[test]
    fn the_startup_receipt_is_a_valid_receipt_ref() {
        ReceiptRef::new(STARTUP_PROVISION_RECEIPT).expect("the startup receipt ref is valid");
    }

    /// One provisioning per process, no matter how many startup paths race.
    #[gpui::test]
    fn startup_provisioning_is_shared_by_concurrent_callers(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = Arc::new(FakeBackend {
            calls: calls.clone(),
            outcome: Ok(CustodyState::Ready),
        });
        let completed = Arc::new(AtomicUsize::new(0));

        cx.update(|cx| {
            IdentityStartupCoordinator::install_with_backend(backend, cx);
            let provision = cx.global::<IdentityStartupCoordinator>().provision.clone();
            for _ in 0..3 {
                let provision = provision.clone();
                let completed = completed.clone();
                cx.spawn(async move |_| {
                    let custody = provision.await.expect("startup provisioning");
                    assert_eq!(custody.state, CustodyState::Ready);
                    completed.fetch_add(1, Ordering::SeqCst);
                })
                .detach();
            }
        });
        cx.run_until_parked();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(completed.load(Ordering::SeqCst), 3);
    }

    /// A custody refusal is logged, not a dead end: `await_identity_ready`
    /// still returns `Ok` so startup opens the front door. This is the
    /// structural replacement for the removed `onboarding::Finish` release —
    /// no state of custody leaves a launch parked forever.
    #[gpui::test]
    fn a_provisioning_refusal_never_blocks_the_front_door(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = Arc::new(FakeBackend {
            calls: calls.clone(),
            outcome: Err(CustodyState::Lost),
        });
        let completed = Arc::new(AtomicUsize::new(0));

        cx.update(|cx| {
            IdentityStartupCoordinator::install_with_backend(backend, cx);
            for _ in 0..2 {
                let completed = completed.clone();
                cx.spawn(async move |cx| {
                    await_identity_ready(cx)
                        .await
                        .expect("a refusal must not become a startup error");
                    completed.fetch_add(1, Ordering::SeqCst);
                })
                .detach();
            }
        });
        cx.run_until_parked();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(completed.load(Ordering::SeqCst), 2);
    }
}
