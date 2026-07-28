//! What the owner's front door shows for one wrapped harness, and which of the
//! two pin controls it offers. `OMEGA-DELTA-0033`, omega#81.
//!
//! The first landing of omega#81 built every decision and rendered none of
//! them: `MaintenanceAffordance::Disabled { reason }` structurally could not be
//! built without a sentence, and nothing ever put that sentence on a screen. A
//! refusal that reaches the owner only as agent-launch error text is a refusal
//! the owner cannot act on, and the standing rule is that owner-facing
//! operations get working controls rather than a file to hand-edit.
//!
//! This module is the whole answer to *what does the settings row say and which
//! buttons are live*, as one value, computed by one function, with no
//! filesystem and no GPUI. The widget that renders it makes no judgement: it
//! matches on [`PinControl`] to decide which button exists, and it prints
//! sentences the decision layer produced. That split is why the rendered state
//! can be tested at the speed of a leaf crate, and why "the disabled control
//! carries its reason" is checkable without a window.
//!
//! # Every state is total
//!
//! [`PinControl`] has no "nothing to offer" that lacks a sentence, for the same
//! reason [`MaintenanceAffordance`] does not: a control that is absent and
//! unexplained reads to an owner as a bug in Omega rather than as a fact about
//! their machine.

use crate::{
    HarnessPin, InstallationProvenance, MaintenanceAffordance, MeasuredDigest, PinState,
    ProvenanceGap, ProvenanceVerdict, admits_package_manager_launch, decide_maintenance,
    update_affordance, verify_installation,
};

/// How a harness's bytes reach the machine.
///
/// Two variants, because pinning is possible in exactly one of them. A harness
/// whose bytes Omega downloads into a directory it owns can be hashed and
/// therefore frozen; a harness a package manager resolves at launch cannot be,
/// and the front door has to say which kind it is looking at rather than show a
/// pin button that would not hold.
///
/// Owner-named custom binaries are not here at all. They have no registry id,
/// no version Omega resolved, and no update path Omega drives, so there is no
/// maintenance state to show — the owner named the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HarnessDistribution {
    /// Omega downloads a release archive into a directory it owns.
    OwnedTree,
    /// A package manager resolves the bytes at launch.
    PackageManager {
        /// The resolver that owns them, e.g. `npx`.
        resolver: String,
    },
}

impl HarnessDistribution {
    /// The resolver's name, when a resolver owns the bytes.
    #[must_use]
    pub fn resolver(&self) -> Option<&str> {
        match self {
            Self::OwnedTree => None,
            Self::PackageManager { resolver } => Some(resolver.as_str()),
        }
    }
}

/// The one pin control the front door offers for a harness, if any.
///
/// One control, not two: "pin" and "unpin" are never both live, because a
/// harness is frozen or it is not. Re-pinning is deliberately two actions —
/// remove, then pin — so that moving a pin from one release to another is
/// something the owner did twice on purpose rather than something a single
/// click could do to a frozen harness by accident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinControl {
    /// Nothing is frozen and this host measured the installed tree, so a pin can
    /// be taken at bytes that were read rather than at a version that was typed.
    Take {
        version: String,
        digest: MeasuredDigest,
    },
    /// Frozen. The control removes the pin.
    Remove { pinned_version: String },
    /// Neither control is offered, and this is why.
    Unavailable { reason: String },
}

impl PinControl {
    /// The sentence to show where a control would have been.
    #[must_use]
    pub fn unavailable_reason(&self) -> Option<&str> {
        match self {
            Self::Take { .. } | Self::Remove { .. } => None,
            Self::Unavailable { reason } => Some(reason.as_str()),
        }
    }
}

/// Everything the front door shows for one harness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessFrontDoorState {
    pub harness_id: String,
    /// The version the registry currently names.
    pub version: String,
    pub distribution: HarnessDistribution,
    /// The pin, when one covers this harness. Carried whole so the row can name
    /// the frozen version without re-reading the ledger.
    pub pin: Option<HarnessPin>,
    /// What the host can conclude about the bytes that would run.
    pub provenance: ProvenanceVerdict,
    /// Whether this harness may launch, and if not, why.
    ///
    /// The same value the launch path computes, from the same function. A front
    /// door that derived its own answer could disagree with the gate, and the
    /// disagreement would show up as a control that looks live and then fails.
    pub launch: MaintenanceAffordance,
    pub pin_control: PinControl,
}

impl HarnessFrontDoorState {
    /// Whether this row has anything to say beyond its name and version.
    #[must_use]
    pub fn has_detail(&self) -> bool {
        !self.launch.is_enabled()
            || self.pin.is_some()
            || !matches!(self.provenance, ProvenanceVerdict::Verified { .. })
    }
}

/// Compute the front door's state for one harness.
///
/// `measured` is `None` when this host could not read the installed tree, which
/// is a different fact from an empty tree and is not collapsed into one.
///
/// Everything here routes through the same decision functions the launch path
/// uses. There is no second opinion in this module: `launch` is
/// [`update_affordance`] over [`decide_maintenance`] for an owned tree and over
/// [`admits_package_manager_launch`] for a resolved one, which are exactly the
/// two gates `crates/project/src/harness_maintenance.rs` enforces.
#[must_use]
pub fn harness_front_door_state(
    harness_id: &str,
    version: &str,
    distribution: HarnessDistribution,
    pin_state: PinState<'_>,
    measured: Option<&MeasuredDigest>,
    provenance: &InstallationProvenance,
) -> HarnessFrontDoorState {
    let pin = match pin_state {
        PinState::Pinned(pin) => Some(pin.clone()),
        PinState::Unpinned | PinState::Unreadable => None,
    };

    let (launch, provenance_verdict, pin_control) = match &distribution {
        HarnessDistribution::PackageManager { resolver } => {
            let launch = match admits_package_manager_launch(pin_state, resolver) {
                None => MaintenanceAffordance::Enabled,
                Some(refusal) => MaintenanceAffordance::Disabled {
                    reason: refusal.reason(),
                },
            };
            let pin_control = match pin_state {
                PinState::Pinned(pin) => PinControl::Remove {
                    pinned_version: pin.version.clone(),
                },
                // Offering "pin" here would offer a freeze Omega cannot hold.
                // The sentence says so rather than leaving a gap where a
                // control is on every other row.
                PinState::Unpinned | PinState::Unreadable => PinControl::Unavailable {
                    reason: format!(
                        "This agent runs through {resolver}, which resolves its own bytes when \
                         it starts. Omega has nothing to freeze, so it cannot pin this agent."
                    ),
                },
            };
            (
                launch,
                ProvenanceVerdict::Refused(ProvenanceGap::ResolvedAtLaunch),
                pin_control,
            )
        }
        HarnessDistribution::OwnedTree => {
            let candidate = match measured {
                Some(digest) => crate::CandidateArtifact::Measured { version, digest },
                None => crate::CandidateArtifact::Unmeasured { version },
            };
            let decision = decide_maintenance(pin_state, candidate);
            let launch = update_affordance(&decision);

            let provenance_verdict = match measured {
                Some(digest) => verify_installation(provenance, digest),
                None => ProvenanceVerdict::Refused(ProvenanceGap::TreeUnreadable),
            };

            let pin_control = match (pin_state, measured) {
                (PinState::Pinned(pin), _) => PinControl::Remove {
                    pinned_version: pin.version.clone(),
                },
                // A pin is only worth taking at bytes this host read. Taking one
                // from an unreadable tree would freeze a digest nobody measured,
                // which is the failure the whole contract exists to prevent.
                (PinState::Unpinned, Some(digest)) => PinControl::Take {
                    version: version.to_string(),
                    digest: digest.clone(),
                },
                (PinState::Unpinned, None) => PinControl::Unavailable {
                    reason: "Omega could not read this agent's installed files, so it has no \
                             measurement to pin. Reinstall it, then pin."
                        .to_string(),
                },
                (PinState::Unreadable, _) => PinControl::Unavailable {
                    reason: "Omega could not read the pin ledger, so it cannot change what is \
                             frozen without risking overwriting a pin it cannot see."
                        .to_string(),
                },
            };

            (launch, provenance_verdict, pin_control)
        }
    };

    HarnessFrontDoorState {
        harness_id: harness_id.to_string(),
        version: version.to_string(),
        distribution,
        pin,
        provenance: provenance_verdict,
        launch,
        pin_control,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        HarnessMaintenanceRecord, HarnessPinLedger, LoadedPinLedger, MaintenanceAction,
        MaintenanceOutcomeInput, build_harness_maintenance_receipt,
    };

    const TREE: &[u8] = b"codex-acp 0.9.4 installed tree";
    const OTHER_TREE: &[u8] = b"codex-acp 0.9.5 installed tree";
    const NOW: u64 = 1_784_894_400_000;

    fn pinned_at(version: &str, bytes: &[u8]) -> LoadedPinLedger {
        let mut ledger = HarnessPinLedger::empty();
        ledger.set_pin("codex-acp", version, &MeasuredDigest::measure(bytes));
        LoadedPinLedger::Loaded(ledger)
    }

    fn attested(bytes: &[u8]) -> InstallationProvenance {
        let digest = MeasuredDigest::measure(bytes);
        let receipt = build_harness_maintenance_receipt(
            "host.omega.local",
            "codex-acp",
            NOW,
            MaintenanceAction::Install,
            &MaintenanceOutcomeInput::Applied {
                version: "0.9.4",
                digest: &digest,
            },
        )
        .expect("a receipt");
        InstallationProvenance::Recorded(HarnessMaintenanceRecord::Attested(receipt))
    }

    fn owned(
        ledger: &LoadedPinLedger,
        measured: Option<&MeasuredDigest>,
        provenance: &InstallationProvenance,
    ) -> HarnessFrontDoorState {
        harness_front_door_state(
            "codex-acp",
            "0.9.4",
            HarnessDistribution::OwnedTree,
            ledger.pin_state("codex-acp"),
            measured,
            provenance,
        )
    }

    /// The state a healthy, freshly installed, unpinned harness is in: it runs,
    /// its bytes are attested, and the owner is offered the one control that
    /// changes anything — a pin at the bytes this host just read.
    #[test]
    fn an_attested_unpinned_harness_offers_a_pin_at_the_measured_bytes() {
        let digest = MeasuredDigest::measure(TREE);
        let state = owned(&LoadedPinLedger::Absent, Some(&digest), &attested(TREE));

        assert!(state.launch.is_enabled());
        assert_eq!(
            state.provenance,
            ProvenanceVerdict::Verified {
                digest: digest.as_str().to_string()
            }
        );
        assert_eq!(
            state.pin_control,
            PinControl::Take {
                version: "0.9.4".to_string(),
                digest,
            }
        );
        assert!(state.pin.is_none());
    }

    /// The acceptance sentence's second half, as rendered state: a pin at one
    /// version disables the control for another, and the sentence beside the
    /// disabled control names both versions and what to do.
    #[test]
    fn a_pin_at_another_version_disables_the_launch_and_says_why() {
        let ledger = pinned_at("0.9.4", TREE);
        let digest = MeasuredDigest::measure(OTHER_TREE);
        let state = harness_front_door_state(
            "codex-acp",
            "0.9.5",
            HarnessDistribution::OwnedTree,
            ledger.pin_state("codex-acp"),
            Some(&digest),
            &attested(OTHER_TREE),
        );

        let reason = state
            .launch
            .reason()
            .expect("a disabled launch has a reason");
        assert!(
            reason.contains("0.9.4"),
            "names the pinned version: {reason}"
        );
        assert!(
            reason.contains("0.9.5"),
            "names the offered version: {reason}"
        );
        assert!(
            reason.contains("remove the pin"),
            "says what to do: {reason}"
        );
        assert_eq!(
            state.pin_control,
            PinControl::Remove {
                pinned_version: "0.9.4".to_string()
            }
        );
    }

    /// A pinned harness is never offered a second pin. There is exactly one
    /// control, and while a pin stands it is the one that removes it.
    #[test]
    fn a_pinned_harness_is_never_offered_a_pin() {
        for (version, bytes) in [("0.9.4", TREE), ("0.9.5", OTHER_TREE)] {
            let ledger = pinned_at("0.9.4", TREE);
            let digest = MeasuredDigest::measure(bytes);
            let state = harness_front_door_state(
                "codex-acp",
                version,
                HarnessDistribution::OwnedTree,
                ledger.pin_state("codex-acp"),
                Some(&digest),
                &attested(bytes),
            );
            assert!(
                !matches!(state.pin_control, PinControl::Take { .. }),
                "a pinned harness offered a pin at {version}"
            );
        }
    }

    /// An unreadable tree yields no pin control, because a pin taken from one
    /// would freeze a digest nobody measured.
    #[test]
    fn an_unreadable_tree_offers_no_pin_and_refuses_the_launch() {
        let state = owned(&LoadedPinLedger::Absent, None, &attested(TREE));
        assert!(!state.launch.is_enabled());
        assert_eq!(
            state.provenance,
            ProvenanceVerdict::Refused(ProvenanceGap::TreeUnreadable)
        );
        assert!(state.pin_control.unavailable_reason().is_some());
        assert!(!matches!(state.pin_control, PinControl::Take { .. }));
    }

    /// An unreadable ledger must not offer a control that writes to it: the
    /// write would drop pins the reader could not see.
    #[test]
    fn an_unreadable_ledger_offers_no_control_that_would_rewrite_it() {
        let ledger = LoadedPinLedger::Unreadable(crate::HarnessPinLedgerError::InvalidJson);
        let digest = MeasuredDigest::measure(TREE);
        let state = owned(&ledger, Some(&digest), &attested(TREE));
        assert!(!state.launch.is_enabled());
        assert!(
            state.pin_control.unavailable_reason().is_some(),
            "an unreadable ledger offered a write"
        );
    }

    /// An installation from before omega#81 runs — the bytes are measurable and
    /// nothing is frozen — but it is reported as unattested rather than
    /// upgraded by assumption.
    #[test]
    fn an_installation_with_no_record_runs_and_is_reported_unattested() {
        let digest = MeasuredDigest::measure(TREE);
        let state = owned(
            &LoadedPinLedger::Absent,
            Some(&digest),
            &InstallationProvenance::Unattested,
        );
        assert!(state.launch.is_enabled());
        assert_eq!(
            state.provenance,
            ProvenanceVerdict::Refused(ProvenanceGap::Unattested)
        );
    }

    /// The npx half of the gap this delta closes. An unpinned package-manager
    /// harness still launches — refusing every one of them is a different
    /// change than this issue asks for — but the row says there is nothing to
    /// attest and no pin to take, instead of showing a control that would not
    /// hold.
    #[test]
    fn an_unpinned_package_manager_harness_launches_and_says_it_cannot_be_pinned() {
        let state = harness_front_door_state(
            "some-npx-agent",
            "1.2.3",
            HarnessDistribution::PackageManager {
                resolver: "npx".to_string(),
            },
            LoadedPinLedger::Absent.pin_state("some-npx-agent"),
            None,
            &InstallationProvenance::Unattested,
        );
        assert!(state.launch.is_enabled());
        assert_eq!(
            state.provenance,
            ProvenanceVerdict::Refused(ProvenanceGap::ResolvedAtLaunch)
        );
        let reason = state
            .pin_control
            .unavailable_reason()
            .expect("a missing pin control has a reason");
        assert!(reason.contains("npx"), "names the resolver: {reason}");
    }

    /// The safety half: a pin on an npx harness is not silently ignored. It
    /// refuses the launch and offers removal, so the owner's "not that one"
    /// means something even where Omega cannot enforce a version.
    #[test]
    fn a_pinned_package_manager_harness_refuses_rather_than_ignoring_the_pin() {
        let ledger = pinned_at("1.2.3", TREE);
        let state = harness_front_door_state(
            "codex-acp",
            "1.2.3",
            HarnessDistribution::PackageManager {
                resolver: "npx".to_string(),
            },
            ledger.pin_state("codex-acp"),
            None,
            &InstallationProvenance::Unattested,
        );
        assert!(!state.launch.is_enabled());
        let reason = state.launch.reason().expect("a refusal has a reason");
        assert!(reason.contains("npx"), "names the resolver: {reason}");
        assert_eq!(
            state.pin_control,
            PinControl::Remove {
                pinned_version: "1.2.3".to_string()
            }
        );
    }

    /// The binding property of this module: the front door and the launch path
    /// must not be able to disagree. Whatever `decide_maintenance` says for an
    /// owned tree is what the row shows, across every combination of pin state
    /// and measurement.
    #[test]
    fn the_rendered_launch_state_equals_what_the_gate_would_decide() {
        let measured = MeasuredDigest::measure(TREE);
        let other = MeasuredDigest::measure(OTHER_TREE);
        let ledgers = [
            LoadedPinLedger::Absent,
            pinned_at("0.9.4", TREE),
            pinned_at("0.9.4", OTHER_TREE),
            pinned_at("0.9.5", TREE),
            LoadedPinLedger::Unreadable(crate::HarnessPinLedgerError::InvalidJson),
        ];

        for ledger in &ledgers {
            for candidate in [Some(&measured), Some(&other), None] {
                let state = owned(ledger, candidate, &attested(TREE));
                let expected = update_affordance(&decide_maintenance(
                    ledger.pin_state("codex-acp"),
                    match candidate {
                        Some(digest) => crate::CandidateArtifact::Measured {
                            version: "0.9.4",
                            digest,
                        },
                        None => crate::CandidateArtifact::Unmeasured { version: "0.9.4" },
                    },
                ));
                assert_eq!(
                    state.launch, expected,
                    "the row disagreed with the gate for {ledger:?}"
                );
            }
        }
    }

    /// Every state a row can be in either enables its control or explains
    /// itself. A blank disabled control is the defect this module exists to
    /// prevent, so it is asserted over the whole space rather than per case.
    #[test]
    fn no_reachable_state_withholds_a_control_without_a_sentence() {
        let measured = MeasuredDigest::measure(TREE);
        let distributions = [
            HarnessDistribution::OwnedTree,
            HarnessDistribution::PackageManager {
                resolver: "npx".to_string(),
            },
        ];
        let ledgers = [
            LoadedPinLedger::Absent,
            pinned_at("0.9.4", TREE),
            pinned_at("0.9.5", TREE),
            LoadedPinLedger::Unreadable(crate::HarnessPinLedgerError::InvalidJson),
        ];
        let provenances = [
            InstallationProvenance::Unattested,
            attested(TREE),
            attested(OTHER_TREE),
        ];

        for distribution in &distributions {
            for ledger in &ledgers {
                for measurement in [Some(&measured), None] {
                    for provenance in &provenances {
                        let state = harness_front_door_state(
                            "codex-acp",
                            "0.9.4",
                            distribution.clone(),
                            ledger.pin_state("codex-acp"),
                            measurement,
                            provenance,
                        );
                        if !state.launch.is_enabled() {
                            let reason = state.launch.reason().unwrap_or("");
                            assert!(
                                !reason.trim().is_empty(),
                                "a disabled launch with no sentence: {state:?}"
                            );
                        }
                        if let PinControl::Unavailable { reason } = &state.pin_control {
                            assert!(
                                !reason.trim().is_empty(),
                                "a withheld pin control with no sentence: {state:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
