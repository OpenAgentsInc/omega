//! What a thread may say about the engine run it is linked to.
//! `OMEGA-DELTA-0030`.
//!
//! omega#77 gave every thread an executor line, so a thread executed by an
//! `omega-effectd` lane already names its run. Naming a run is not showing
//! one: the reader is told a reference and left to go and look somewhere else
//! for whether the work was verified, refused, or merely claimed. omega#80
//! closes that — the linked run renders its state and its receipt chain in the
//! thread, through the same inspector grammar the receipt pane uses.
//!
//! # The engine stays the sole run authority
//!
//! Nothing here holds run state. [`project_thread_run_link`] is a pure
//! function of (the thread's disclosure record, the engine's own records, the
//! clock), so the thread *projects* a run and can never become a second
//! opinion about one. Three consequences, each of them checked below:
//!
//! - The reference comes from [`ExecutorDisclosure::run_ref`], the typed
//!   record omega#77 stores, not from a string a surface assembled.
//! - The engine's answer expires. Past [`THREAD_RUN_LINK_MAX_AGE_MS`] the link
//!   renders `host_unavailable` instead of the last chain it saw, so a thread
//!   cannot outlive the authority it is projecting. A stale complete chain
//!   held on screen is precisely a panel entity becoming the source of truth.
//! - A state Omega does not recognise is not translated into the nearest one.
//!   An acknowledgement is not a completion, and a run that says `acknowledged`
//!   reports no state at all rather than being read as finished.
//!
//! # A broken chain is shown, not hidden
//!
//! The chain is produced by
//! [`workroom_receipts::project_issue31_evidence_pair`] — the same producer
//! that builds the phone's projection, so the desktop and the phone cannot
//! hold two opinions about one run. When it refuses, the refusal is rendered
//! with its reason, never as an absence and never as completion. A single
//! unshowable run must not blank the surface: one malformed record stopping
//! every device is a bug this workspace has already shipped once.

use serde_json::Value;
use workroom_receipts::{
    InspectorField, Issue31EvidenceChain, Issue31EvidenceUnavailableReason,
    Issue31FullAutoLifecycle, PublicRef, project_issue31_evidence_pair,
};

use omega_front_door::{ExecutorClass, ExecutorDisclosure};

/// How long the engine's answer about a run stays showable.
///
/// The panel re-reads every three seconds, so five missed reads is a host that
/// has stopped answering rather than one that is briefly busy. Rendering the
/// last known chain past that point would be the thread asserting a run state
/// on the engine's behalf.
pub const THREAD_RUN_LINK_MAX_AGE_MS: u64 = 15_000;

/// One reading of the engine's records for a linked run.
///
/// Raw host answers plus the instant they were read. Deliberately not a
/// projected view: caching the *conclusion* is what lets a surface disagree
/// with the engine, whereas caching the engine's own words and re-deriving
/// cannot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadRunRecords {
    /// When the engine answered, on the same clock as `now_ms`.
    pub read_at_ms: u64,
    /// The `get_run` record, when the engine answered.
    pub run: Option<Value>,
    /// The `get_report` record, when the engine answered.
    pub report: Option<Value>,
    /// The `get_receipt` record, when the engine answered.
    pub receipt: Option<Value>,
}

/// A thread's projection of the engine run that executed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadRunLink {
    /// The run, named by the thread's own disclosure record.
    pub run_ref: PublicRef,
    /// The agent the run delegated the work to, kept from the disclosure.
    pub agent_id: String,
    /// The run's lifecycle, when the engine reported one this contract models.
    ///
    /// `None` says the engine has not told this surface where the run stands.
    /// It is never a stand-in for "queued" or "running": showing a state the
    /// host did not report is the failure the whole surface exists to avoid.
    pub state: Option<Issue31FullAutoLifecycle>,
    /// The receipt chain, complete or unavailable-with-a-reason.
    pub chain: Issue31EvidenceChain,
}

impl ThreadRunLink {
    /// Whether this run's completion is backed by a resolvable receipt chain.
    ///
    /// omega#80's falsifier is "a completed run is claimed without its receipt
    /// refs resolvable". This is the predicate that makes such a claim
    /// impossible to make by accident: a terminal state is not enough, and an
    /// authority decision that refused is not a completion either.
    #[must_use]
    pub fn is_receipted(&self) -> bool {
        matches!(
            self.chain,
            Issue31EvidenceChain::Complete {
                authority_allowed: true,
                ..
            }
        )
    }

    /// Why the chain is not showable, when it is not.
    #[must_use]
    pub fn unavailable_reason(&self) -> Option<Issue31EvidenceUnavailableReason> {
        match self.chain {
            Issue31EvidenceChain::Unavailable { reason, .. } => Some(reason),
            Issue31EvidenceChain::Complete { .. } => None,
        }
    }

    /// The inspector rows this link renders.
    ///
    /// Same label/value grammar as `workroom_receipts::render_receipt_detail`,
    /// so the thread's chain reads like the receipt pane's rather than like a
    /// second format. The unavailable case still produces rows — `chain:
    /// unavailable` and `chain_reason: …` — because a surface that renders
    /// nothing has told the reader nothing, and silence reads as "no run".
    #[must_use]
    pub fn fields(&self) -> Vec<InspectorField> {
        let mut fields = vec![
            InspectorField::new("run_ref", self.run_ref.as_str()),
            InspectorField::new("executed_by", self.agent_id.as_str()),
            InspectorField::new(
                "run_state",
                match self.state {
                    Some(state) => lifecycle_token(state),
                    None => "not reported by the engine",
                },
            ),
        ];
        match &self.chain {
            Issue31EvidenceChain::Complete {
                authority_allowed,
                hops,
                ..
            } => {
                fields.push(InspectorField::new("chain", "complete"));
                fields.push(InspectorField::new(
                    "authority_allowed",
                    if *authority_allowed { "true" } else { "false" },
                ));
                for hop in hops {
                    fields.push(InspectorField::new(
                        hop.kind.token(),
                        match &hop.detail {
                            Some(detail) => format!("{} · {detail}", hop.reference.as_str()),
                            None => hop.reference.as_str().to_owned(),
                        },
                    ));
                }
            }
            Issue31EvidenceChain::Unavailable {
                reason, broken_at, ..
            } => {
                fields.push(InspectorField::new("chain", "unavailable"));
                fields.push(InspectorField::new("chain_reason", reason.token()));
                if let Some(broken_at) = broken_at {
                    fields.push(InspectorField::new("chain_broken_at", broken_at.token()));
                }
            }
        }
        fields
    }
}

/// The wire token for a lifecycle, for rendering.
fn lifecycle_token(state: Issue31FullAutoLifecycle) -> &'static str {
    match state {
        Issue31FullAutoLifecycle::Queued => "queued",
        Issue31FullAutoLifecycle::Running => "running",
        Issue31FullAutoLifecycle::Pausing => "pausing",
        Issue31FullAutoLifecycle::Paused => "paused",
        Issue31FullAutoLifecycle::Stopping => "stopping",
        Issue31FullAutoLifecycle::Retrying => "retrying",
        Issue31FullAutoLifecycle::Stalled => "stalled",
        Issue31FullAutoLifecycle::Succeeded => "succeeded",
        Issue31FullAutoLifecycle::Failed => "failed",
        Issue31FullAutoLifecycle::Stopped => "stopped",
        Issue31FullAutoLifecycle::Expired => "expired",
    }
}

fn lifecycle_from_host(state: &str) -> Option<Issue31FullAutoLifecycle> {
    let token = crate::issue31_adjunct::lifecycle_for_host_state(state)?;
    serde_json::from_value(Value::String(token.to_owned())).ok()
}

fn record_run_ref(record: &Value) -> Option<&str> {
    record.get("runRef")?.as_str()
}

/// Project a thread's linked run from the engine's own records.
///
/// Returns `None` when the thread is not engine-lane work — a native or
/// external ACP thread has no run authority to project, and inventing an empty
/// run panel for it would suggest one exists.
///
/// An incoherent disclosure also returns `None`. A `native_loop` record
/// carrying a run reference is a routed result wearing the wrong name, and
/// `OMEGA-AGENT-AC-05` exists to stop exactly that from being rendered as a
/// run this thread owns.
#[must_use]
pub fn project_thread_run_link(
    disclosure: &ExecutorDisclosure,
    records: Option<&ThreadRunRecords>,
    now_ms: u64,
) -> Option<ThreadRunLink> {
    if disclosure.class != ExecutorClass::EngineLane || !disclosure.is_coherent() {
        return None;
    }
    // The reference is the one the typed disclosure record holds. Nothing here
    // parses it back out of the rendered line.
    let run_ref = PublicRef::new(disclosure.run_ref.as_ref()?)?;

    let fresh = records
        .filter(|records| now_ms.saturating_sub(records.read_at_ms) <= THREAD_RUN_LINK_MAX_AGE_MS);

    // A stale or absent reading is the host not answering, whatever it said
    // last time. `state` follows the chain into `None` rather than surviving
    // it: half a stale reading is still a stale reading.
    let Some(records) = fresh else {
        return Some(ThreadRunLink {
            run_ref: run_ref.clone(),
            agent_id: disclosure.agent_id.clone(),
            state: None,
            chain: Issue31EvidenceChain::Unavailable {
                run_ref,
                reason: Issue31EvidenceUnavailableReason::HostUnavailable,
                broken_at: None,
            },
        });
    };

    let state = records
        .run
        .as_ref()
        .filter(|run| record_run_ref(run) == Some(run_ref.as_str()))
        .and_then(|run| run.get("state"))
        .and_then(Value::as_str)
        .and_then(lifecycle_from_host);

    let chain = project_chain(&run_ref, records);

    Some(ThreadRunLink {
        run_ref,
        agent_id: disclosure.agent_id.clone(),
        state,
        chain,
    })
}

fn project_chain(run_ref: &PublicRef, records: &ThreadRunRecords) -> Issue31EvidenceChain {
    let unavailable = |reason| Issue31EvidenceChain::Unavailable {
        run_ref: run_ref.clone(),
        reason,
        broken_at: None,
    };

    let (Some(report), Some(receipt)) = (records.report.as_ref(), records.receipt.as_ref()) else {
        return unavailable(Issue31EvidenceUnavailableReason::HostUnavailable);
    };

    // A pair belonging to another run is not this run's proof. Checked before
    // the producer, because the producer is entitled to assume it was handed
    // one run's records and would otherwise report a foreign but internally
    // consistent chain as this thread's.
    if record_run_ref(report) != Some(run_ref.as_str())
        || record_run_ref(receipt) != Some(run_ref.as_str())
    {
        return unavailable(Issue31EvidenceUnavailableReason::HopMismatched);
    }

    match project_issue31_evidence_pair(report, receipt, Some(run_ref.as_str())) {
        Ok(chain) if chain.run_ref() == run_ref => chain,
        // The producer refuses to relabel a complete chain, so a chain naming
        // another run reaching here means the records contradict themselves.
        Ok(_) => unavailable(Issue31EvidenceUnavailableReason::HopMismatched),
        // The two reachable decoder errors are both "the host produced this and
        // this surface may not carry it": a hop reference outside the
        // public-reference character class, and a detail past the carried
        // bound. `hop_private` is the reason for exactly that situation.
        Err(_) => unavailable(Issue31EvidenceUnavailableReason::HopPrivate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use workroom_receipts::{ISSUE31_EVIDENCE_HOPS, Issue31EvidenceHopKind};

    const NOW: u64 = 1_784_894_400_000;

    fn engine_lane() -> ExecutorDisclosure {
        ExecutorDisclosure {
            class: ExecutorClass::EngineLane,
            agent_id: "codex-acp".into(),
            provider: None,
            model: None,
            run_ref: Some("run.fa.80".into()),
            // A thread restored from before the router landed carries no
            // recorded route. omega#78 says that is "not routed", not a route,
            // and it must not stop the run behind it from being inspectable.
            route: None,
        }
    }

    fn run_record(state: &str) -> Value {
        json!({
            "runRef": "run.fa.80",
            "title": "Receipts in thread",
            "state": state,
        })
    }

    fn evidence_pair() -> (Value, Value) {
        (
            json!({
                "runRef": "run.fa.80",
                "evidence": {
                    "objectiveRef": "objective.fa.80",
                    "turnRef": "turn.fa.80.11",
                    "changeRef": "change.fa.80.11",
                    "projectGeneration": "project.generation.9",
                    "diffSummary": "3 files changed, 210 insertions",
                    "testCommand": "cargo test -p full_auto_ui",
                    "testOutcome": "passed",
                    "verificationRef": "verification.host.fa.80.11",
                    "hostExecuted": true
                }
            }),
            json!({
                "runRef": "run.fa.80",
                "objectiveRef": "objective.fa.80",
                "turnRef": "turn.fa.80.11",
                "changeRef": "change.fa.80.11",
                "verificationRef": "verification.host.fa.80.11",
                "decisionRef": "decision.authority.fa.80.11",
                "authorityReceiptRef": "receipt.authority.fa.80.11",
                "allowed": true
            }),
        )
    }

    fn records(state: &str) -> ThreadRunRecords {
        let (report, receipt) = evidence_pair();
        ThreadRunRecords {
            read_at_ms: NOW,
            run: Some(run_record(state)),
            report: Some(report),
            receipt: Some(receipt),
        }
    }

    fn link(records: Option<&ThreadRunRecords>, now_ms: u64) -> ThreadRunLink {
        project_thread_run_link(&engine_lane(), records, now_ms).expect("an engine lane links")
    }

    #[test]
    fn a_finished_run_renders_its_chain_in_the_normative_order() {
        let records = records("completed");
        let link = link(Some(&records), NOW);
        assert_eq!(link.state, Some(Issue31FullAutoLifecycle::Succeeded));
        assert!(link.is_receipted());

        match &link.chain {
            Issue31EvidenceChain::Complete { hops, .. } => {
                let kinds: Vec<Issue31EvidenceHopKind> = hops.iter().map(|hop| hop.kind).collect();
                assert_eq!(kinds, ISSUE31_EVIDENCE_HOPS.to_vec());
            }
            Issue31EvidenceChain::Unavailable { .. } => panic!("expected a complete chain"),
        }

        let lines: Vec<String> = link.fields().iter().map(InspectorField::line).collect();
        let text = lines.join("\n");
        assert!(text.contains("run_ref: run.fa.80"), "{text}");
        assert!(text.contains("executed_by: codex-acp"), "{text}");
        assert!(text.contains("run_state: succeeded"), "{text}");
        assert!(text.contains("chain: complete"), "{text}");
        assert!(text.contains("authority_allowed: true"), "{text}");
        assert!(
            text.contains("host_verification: verification.host.fa.80.11"),
            "{text}"
        );
        assert!(
            text.contains("receipt: receipt.authority.fa.80.11"),
            "{text}"
        );
        assert!(
            text.contains("test: verification.host.fa.80.11 · cargo test -p full_auto_ui"),
            "the hop detail is the command the host ran: {text}"
        );
        assert!(!text.contains("/Users/"), "{text}");
    }

    /// Only engine-lane work has a run to project.
    #[test]
    fn a_thread_with_no_run_authority_has_no_run_link() {
        for class in [ExecutorClass::NativeLoop, ExecutorClass::ExternalAcp] {
            let disclosure = ExecutorDisclosure {
                class,
                run_ref: None,
                ..engine_lane()
            };
            assert!(disclosure.is_coherent());
            assert!(
                project_thread_run_link(&disclosure, Some(&records("running")), NOW).is_none(),
                "{class:?} threads own no run"
            );
        }
    }

    /// A routed result wearing the first-party name projects nothing.
    ///
    /// The incoherent record is the one omega#77 already refuses to call
    /// coherent; this proves the run surface refuses it too, rather than
    /// rendering a run panel on a thread that never had run authority.
    #[test]
    fn an_incoherent_disclosure_never_becomes_a_run_link() {
        let native_claiming_a_run = ExecutorDisclosure {
            class: ExecutorClass::NativeLoop,
            run_ref: Some("run.fa.80".into()),
            ..engine_lane()
        };
        assert!(!native_claiming_a_run.is_coherent());
        assert!(
            project_thread_run_link(&native_claiming_a_run, Some(&records("running")), NOW)
                .is_none()
        );

        let engine_without_a_run = ExecutorDisclosure {
            run_ref: None,
            ..engine_lane()
        };
        assert!(!engine_without_a_run.is_coherent());
        assert!(
            project_thread_run_link(&engine_without_a_run, Some(&records("running")), NOW)
                .is_none()
        );

        // omega#78: a fallback route means the router could not place the
        // turn and it ran on the native loop. A record claiming both a
        // fallback and an engine lane is two stories about one thread.
        let fallback_claiming_a_lane = ExecutorDisclosure {
            route: Some(omega_front_door::RouteReason::EngineUnreachable),
            ..engine_lane()
        };
        assert!(!fallback_claiming_a_lane.is_coherent());
        assert!(
            project_thread_run_link(&fallback_claiming_a_lane, Some(&records("running")), NOW)
                .is_none()
        );
    }

    /// A run reference this surface may not repeat is not repeated.
    #[test]
    fn a_private_run_reference_is_never_rendered() {
        let disclosure = ExecutorDisclosure {
            run_ref: Some("/Users/owner/.omega/runs/80".into()),
            ..engine_lane()
        };
        assert!(disclosure.is_coherent());
        assert!(project_thread_run_link(&disclosure, Some(&records("running")), NOW).is_none());
    }

    /// The engine's silence is rendered as silence, not as its last answer.
    ///
    /// This is the "engine is the sole run authority" property with teeth: a
    /// complete chain seen once must not stay on screen after the host stops
    /// answering, or the thread has become the authority for it.
    #[test]
    fn a_stale_reading_stops_being_shown_as_the_runs_state() {
        let records = records("completed");
        assert!(link(Some(&records), NOW + THREAD_RUN_LINK_MAX_AGE_MS).is_receipted());

        let stale = link(Some(&records), NOW + THREAD_RUN_LINK_MAX_AGE_MS + 1);
        assert!(!stale.is_receipted());
        assert_eq!(stale.state, None);
        assert_eq!(
            stale.unavailable_reason(),
            Some(Issue31EvidenceUnavailableReason::HostUnavailable)
        );
        // The reference still shows: the thread knows which run it belongs to
        // even when the engine is not answering about it.
        assert_eq!(stale.run_ref.as_str(), "run.fa.80");
    }

    /// A host that has not answered at all is named as such.
    #[test]
    fn an_unanswered_host_renders_a_reason_rather_than_an_absence() {
        let never_read = link(None, NOW);
        assert_eq!(
            never_read.unavailable_reason(),
            Some(Issue31EvidenceUnavailableReason::HostUnavailable)
        );
        assert_eq!(never_read.state, None);

        let no_receipt = ThreadRunRecords {
            receipt: None,
            ..records("running")
        };
        let partial = link(Some(&no_receipt), NOW);
        assert_eq!(
            partial.unavailable_reason(),
            Some(Issue31EvidenceUnavailableReason::HostUnavailable)
        );
        // The half the host did answer is still shown. A missing receipt does
        // not blank the run's state.
        assert_eq!(partial.state, Some(Issue31FullAutoLifecycle::Running));
    }

    /// Every refusal path, watched refusing, and none of them silent.
    #[test]
    fn every_broken_chain_is_shown_with_its_reason() {
        let reason = |mutate: &dyn Fn(&mut Value, &mut Value)| {
            let (mut report, mut receipt) = evidence_pair();
            mutate(&mut report, &mut receipt);
            let records = ThreadRunRecords {
                read_at_ms: NOW,
                run: Some(run_record("completed")),
                report: Some(report),
                receipt: Some(receipt),
            };
            let link = link(Some(&records), NOW);
            assert!(
                !link.is_receipted(),
                "a broken chain must never be claimed as receipted"
            );
            assert!(
                !link.fields().is_empty(),
                "a broken chain must still render rows"
            );
            link.unavailable_reason().expect("a named refusal")
        };

        assert_eq!(
            reason(&|report, _| {
                report["evidence"]
                    .as_object_mut()
                    .expect("evidence object")
                    .remove("projectGeneration");
            }),
            Issue31EvidenceUnavailableReason::HopMissing
        );
        assert_eq!(
            reason(&|_, receipt| receipt["changeRef"] = json!("change.someone-elses-work")),
            Issue31EvidenceUnavailableReason::HopMismatched
        );
        assert_eq!(
            reason(&|report, _| {
                report["evidence"]["testCommand"] = json!("cat /Users/owner/.codex/auth.json");
            }),
            Issue31EvidenceUnavailableReason::HopPrivate
        );
        assert_eq!(
            reason(&|report, _| report["evidence"]["hostExecuted"] = json!(false)),
            Issue31EvidenceUnavailableReason::SelfReported
        );
        // A chain the host holds but this surface's bound refuses to carry.
        assert_eq!(
            reason(&|report, _| {
                report["evidence"]["testCommand"] =
                    json!(format!("cargo test -p {}", "x".repeat(300)));
            }),
            Issue31EvidenceUnavailableReason::HopPrivate
        );
    }

    /// Another run's proof is not this run's proof.
    ///
    /// Both shapes are checked. A *complete* foreign chain is caught because
    /// the producer refuses to relabel it. A *broken* foreign chain is not:
    /// its refusal would arrive already wearing this run's reference, and
    /// without the pair check the reader would be told this run has a missing
    /// hop when in fact they are looking at somebody else's records.
    #[test]
    fn a_chain_belonging_to_another_run_is_refused_as_mismatched() {
        let foreign = |mutate: &dyn Fn(&mut Value, &mut Value)| {
            let (mut report, mut receipt) = evidence_pair();
            report["runRef"] = json!("run.fa.81");
            receipt["runRef"] = json!("run.fa.81");
            mutate(&mut report, &mut receipt);
            let records = ThreadRunRecords {
                read_at_ms: NOW,
                run: Some(run_record("completed")),
                report: Some(report),
                receipt: Some(receipt),
            };
            let link = link(Some(&records), NOW);
            assert!(!link.is_receipted());
            assert_eq!(link.run_ref.as_str(), "run.fa.80");
            link.unavailable_reason()
        };

        assert_eq!(
            foreign(&|_, _| {}),
            Some(Issue31EvidenceUnavailableReason::HopMismatched),
            "a complete chain proving another run is not this run's proof"
        );
        assert_eq!(
            foreign(&|report, _| {
                report["evidence"]
                    .as_object_mut()
                    .expect("evidence object")
                    .remove("turnRef");
            }),
            Some(Issue31EvidenceUnavailableReason::HopMismatched),
            "another run's broken records are a mismatch, not this run's \
             missing hop"
        );
    }

    /// A run record about a different run tells this thread nothing.
    #[test]
    fn a_state_from_another_runs_record_is_not_this_runs_state() {
        let records = ThreadRunRecords {
            run: Some(json!({"runRef": "run.fa.81", "state": "completed"})),
            ..records("running")
        };
        assert_eq!(link(Some(&records), NOW).state, None);
    }

    /// An acknowledgement is not a completed command.
    ///
    /// A relay or a UI can report that it accepted a request. That is a
    /// statement about a message, not about work, and it must not be
    /// translated into the nearest lifecycle this contract does model.
    #[test]
    fn an_acknowledgement_is_never_read_as_a_run_state() {
        for claimed in [
            "acknowledged",
            "accepted",
            "sent",
            "dispatched",
            "delivered",
            "ok",
            "done",
            "complete",
        ] {
            let records = ThreadRunRecords {
                run: Some(json!({
                    "runRef": "run.fa.80",
                    "state": claimed,
                    "acknowledged": true,
                    "acknowledgedAtMs": NOW,
                })),
                ..records("running")
            };
            assert_eq!(
                link(Some(&records), NOW).state,
                None,
                "{claimed:?} is not a lifecycle this contract models, and \
                 guessing the nearest one is how a stalled run reads as done"
            );
        }
    }

    /// A terminal state is not a receipt.
    ///
    /// omega#80's falsifier: "a completed run is claimed without its receipt
    /// refs resolvable". A succeeded run whose chain is unavailable renders
    /// both facts, and `is_receipted` stays false.
    #[test]
    fn a_succeeded_run_without_a_chain_is_not_claimed_as_receipted() {
        let records = ThreadRunRecords {
            report: None,
            receipt: None,
            ..records("completed")
        };
        let link = link(Some(&records), NOW);
        assert_eq!(link.state, Some(Issue31FullAutoLifecycle::Succeeded));
        assert!(!link.is_receipted());

        let text: Vec<String> = link.fields().iter().map(InspectorField::line).collect();
        let text = text.join("\n");
        assert!(text.contains("run_state: succeeded"), "{text}");
        assert!(text.contains("chain: unavailable"), "{text}");
        assert!(text.contains("chain_reason: host_unavailable"), "{text}");
    }

    /// A complete chain whose authority decision refused is not a receipt.
    ///
    /// The chain resolves — every hop is there and host-verified — and the
    /// answer at the end is "no". Reading that as a receipted completion would
    /// turn a refusal into an approval, which is the most consequential
    /// misreading available on this surface.
    #[test]
    fn a_refused_authority_decision_is_complete_but_not_receipted() {
        let (report, mut receipt) = evidence_pair();
        receipt["allowed"] = json!(false);
        let records = ThreadRunRecords {
            read_at_ms: NOW,
            run: Some(run_record("completed")),
            report: Some(report),
            receipt: Some(receipt),
        };
        let link = link(Some(&records), NOW);
        assert!(matches!(link.chain, Issue31EvidenceChain::Complete { .. }));
        assert!(link.unavailable_reason().is_none());
        assert!(
            !link.is_receipted(),
            "an allowed:false decision is a resolvable chain and a refusal"
        );

        let text: Vec<String> = link.fields().iter().map(InspectorField::line).collect();
        assert!(
            text.join("\n").contains("authority_allowed: false"),
            "{text:?}"
        );
    }

    /// The link is a function of its inputs, which is what keeps the engine
    /// the authority.
    ///
    /// Two projections from one reading are identical, and the record holds no
    /// field a caller could set to change what a run appears to be doing.
    #[test]
    fn the_link_is_a_projection_and_holds_no_authority_of_its_own() {
        let records = records("running");
        assert_eq!(link(Some(&records), NOW), link(Some(&records), NOW));

        let dumped = format!("{:?}", link(Some(&records), NOW));
        for absent in ["cached", "override", "assumed", "last_known"] {
            assert!(!dumped.contains(absent), "{dumped}");
        }

        // Changing only the engine's record changes the projection. Nothing in
        // between remembers the old answer.
        let stopped = ThreadRunRecords {
            run: Some(run_record("stopped")),
            ..records
        };
        assert_eq!(
            link(Some(&stopped), NOW).state,
            Some(Issue31FullAutoLifecycle::Stopped)
        );
    }
}
