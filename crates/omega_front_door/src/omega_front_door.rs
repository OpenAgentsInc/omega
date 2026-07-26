//! The typed core of Omega's chat-first front door.
//!
//! Omega Agent is a router. It owns routing, disclosure, and receipts, and it
//! owns no execution. This crate holds the parts of the front door that are
//! decisions rather than pixels: where a fresh launch lands, what a thread
//! says about the executor that did the work, and which gestures are allowed
//! to start engine-lane authority.
//!
//! It is deliberately a leaf. It depends on nothing, so the laws below can be
//! checked in a second without building GPUI, and so the router packets
//! (omega#77, omega#78) can take this vocabulary without taking the UI crate.
//!
//! Product contract: `specs/omega/omega-agent.product-spec.md` revision 1 in
//! the openagents repository, admitted by the owner on 2026-07-25.

pub mod router;
mod send_during_turn;

pub use send_during_turn::{
    QueueItemState, Quiescence, SendCommand, SendDisposition, SendFallback, SteerCapability,
    SteerRefusal, disposition, may_promote,
};

pub use router::{
    EngineLane, EngineReadiness, EngineUnreachable, ExecutorPin, LaneState, RESERVED_RECORD_CHARACTERS,
    RouteDecision, RouteInputs, RouteReason, lane_ref_is_recordable, route, select_lane,
};

// -------------------------------------------------------------------------
// Where a fresh launch lands
// -------------------------------------------------------------------------

/// The surface Omega shows when a window opens with nothing to restore.
///
/// Upstream Zed calls `Editor::new_file` here, so the first thing a new user
/// meets is an empty untitled buffer. Omega's front door is the agent instead:
/// the window opens on the New Agent Thread surface with the composer focused,
/// and typing starts a thread. `OMEGA-DELTA-0019`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchSurface {
    /// The New Agent Thread surface, composer focused. Omega's front door.
    NewAgentThread,
    /// An empty untitled buffer. Upstream Zed's answer, kept only for the
    /// startup behaviours that explicitly ask for a file-first window.
    EmptyBuffer,
    /// Nothing: the launchpad startup behaviour opens no content at all, and
    /// overriding it would be Omega ignoring a setting the user set.
    Nothing,
}

/// The `restore_on_startup` values that reach the no-restorable-session path.
///
/// Mirrors `settings::RestoreOnStartupBehavior` without depending on it. The
/// mapping below is the whole reason this is a type and not a bool: the
/// front door replaces the *empty buffer*, and must not replace a deliberate
/// launchpad choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOnStartup {
    /// The user asked for the launchpad. Omega opens no content.
    Launchpad,
    /// Anything else. Upstream opens an empty buffer here; Omega opens the
    /// front door.
    Other,
}

/// Decide what a window with no restorable session opens on.
///
/// This is the single rule behind `OMEGA-DELTA-0019`, kept here so it can be
/// tested without a window.
#[must_use]
pub fn launch_surface(restore_on_startup: RestoreOnStartup) -> LaunchSurface {
    match restore_on_startup {
        RestoreOnStartup::Launchpad => LaunchSurface::Nothing,
        RestoreOnStartup::Other => LaunchSurface::NewAgentThread,
    }
}

// -------------------------------------------------------------------------
// Executor disclosure
// -------------------------------------------------------------------------

/// The three admitted executor classes.
///
/// ProductSpec `OMEGA-AGENT-AC-04` fixes the set at exactly three. A fourth
/// class needs a new spec revision, which is why this is a closed enum and not
/// a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorClass {
    /// The inherited native agent loop in `crates/agent`.
    NativeLoop,
    /// An external ACP agent reached through `crates/agent_servers`.
    ExternalAcp,
    /// An `omega-effectd` engine lane. Full Auto and Agent Computer.
    EngineLane,
}

impl ExecutorClass {
    /// The stable wire token for this class.
    ///
    /// Persisted and compared. Never shown to a user on its own — the user
    /// sees [`ExecutorDisclosure::label`], which is derived.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::NativeLoop => "native_loop",
            Self::ExternalAcp => "external_acp",
            Self::EngineLane => "engine_lane",
        }
    }

    /// Every admitted class, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::NativeLoop, Self::ExternalAcp, Self::EngineLane]
    }
}

/// What a thread says about the executor that did its work.
///
/// **This is a record, not a label.** The owner admitted the Omega Agent shape
/// on 2026-07-25 on the recorded condition that the first-party agent does not
/// sign with its own principal *and* that disclosure is stored as a typed
/// record that a label renders. That condition is the only reason the identity
/// choice stays cheap to reverse: moving to a signing principal later then
/// needs a signer, not a rewrite of every stored thread record.
///
/// So [`label`](Self::label) is a function of the fields, and there is no
/// field to put a rendered label in. omega#76 fixed this shape; omega#77
/// populates it from live threads and renders the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorDisclosure {
    /// Which of the three admitted classes ran the work.
    pub class: ExecutorClass,
    /// The executor's own identifier, as the executor reports it.
    pub agent_id: String,
    /// The model provider that served the turn, where the executor reports
    /// one.
    ///
    /// `None` is *not disclosed*, and is different from an empty string, which
    /// is a bug. `AcpConnection` does not implement
    /// `AgentConnection::model_selector`, so an external ACP agent such as
    /// `codex-acp` genuinely does not tell Omega which model served a turn.
    /// Before omega#77 this was a `String`, which left only two options for
    /// that case — fabricate a model, or fail [`is_coherent`](Self::is_coherent)
    /// on every external thread. Saying "not disclosed" is the honest third.
    pub provider: Option<String>,
    /// The model that served the turn, where the executor reports one. See
    /// [`provider`](Self::provider) for why this is optional.
    pub model: Option<String>,
    /// The engine run this thread is bound to, where one applies.
    ///
    /// `Some` only for [`ExecutorClass::EngineLane`]: a native or ACP turn has
    /// no run authority to reference.
    pub run_ref: Option<String>,
    /// Why Omega Agent routed the thread here, where it routed the thread.
    ///
    /// A typed part, added by omega#78 — not a caption. `None` means this
    /// thread was not routed by the router: a thread restored from before
    /// `OMEGA-DELTA-0029`, or one opened directly on an executor. Saying "not
    /// routed" is different from claiming a reason nobody recorded, and the
    /// difference matters most for the fallback reasons, which are the ones a
    /// user needs to see.
    ///
    /// This is the field that makes an engine-down fallback *visible*. A
    /// fallback the user cannot see is the same defect class as a handoff with
    /// no system note.
    pub route: Option<RouteReason>,
}

impl ExecutorDisclosure {
    /// Render the disclosure line for a thread.
    ///
    /// Derived on every call. Nothing stores the output.
    ///
    /// An undisclosed model is *said*, not skipped. A line that quietly
    /// dropped the model segment would read as a complete disclosure, and the
    /// reader would have no way to tell "Omega did not ask" from "the executor
    /// would not say".
    #[must_use]
    pub fn label(&self) -> String {
        let model = match (&self.provider, &self.model) {
            (Some(provider), Some(model)) => format!("{provider}/{model}"),
            (Some(provider), None) => format!("{provider}/model not disclosed"),
            (None, Some(model)) => format!("provider not disclosed/{model}"),
            (None, None) => "model not disclosed".to_owned(),
        };
        let mut line = format!("{} · {} · {model}", self.class.token(), self.agent_id);
        if let Some(run_ref) = &self.run_ref {
            line.push_str(" · ");
            line.push_str(run_ref);
        }
        // omega#78. An unrouted thread says nothing here rather than claiming a
        // reason nobody recorded; a routed one always says why, including — and
        // especially — when a pin could not be honoured.
        if let Some(route) = self.route {
            line.push_str(" · routed: ");
            line.push_str(route.phrase());
        }
        line
    }

    /// Whether this record is internally consistent.
    ///
    /// A run reference on a native turn means something mislabelled a routed
    /// result, which `OMEGA-AGENT-AC-05` exists to prevent. An identifier that
    /// is present but empty means something built the record out of a missing
    /// value instead of leaving it absent, so it stays incoherent.
    ///
    /// omega#78 adds two clauses about the route. A record that says a pin
    /// could not be honoured while showing an engine lane is claiming both that
    /// the router fell back and that it did not; and a record that says the
    /// thread was unpinned while showing an engine lane is owner gate 8 broken,
    /// because nothing but a human pin may reach Full Auto authority.
    #[must_use]
    pub fn is_coherent(&self) -> bool {
        let present_and_named =
            |value: &Option<String>| value.as_ref().is_none_or(|value| !value.is_empty());
        let route_agrees_with_the_class = match self.route {
            Some(route) if route.is_fallback() || route == RouteReason::UnpinnedDefault => {
                self.class == ExecutorClass::NativeLoop
            }
            Some(_) | None => true,
        };
        !self.agent_id.is_empty()
            && present_and_named(&self.provider)
            && present_and_named(&self.model)
            && route_agrees_with_the_class
            && match self.class {
                ExecutorClass::EngineLane => self.run_ref.is_some(),
                ExecutorClass::NativeLoop | ExecutorClass::ExternalAcp => self.run_ref.is_none(),
            }
    }
}

// -------------------------------------------------------------------------
// Who may start Full Auto
// -------------------------------------------------------------------------

/// Every gesture that may start Full Auto authority.
///
/// Owner gate 8, as restated for the fold: *no model-initiated path can start
/// Full Auto authority; only an explicit human action can, wherever that
/// action lives.* Folding Full Auto into chat moves where the action lives. It
/// does not add a new way to reach it.
///
/// Every variant here is a pointer device or keyboard gesture on a control the
/// user can see. There is deliberately no variant for a tool call, a slash
/// command, a restored draft, an agent turn, or a composer mode flag — and
/// `origins_are_all_human_gestures` fails if one appears, because the set is
/// asserted against a written allowlist rather than merely being short today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOrigin {
    /// The Full Auto entry in the agent panel's new-thread menu.
    NewThreadMenuItem,
    /// The `full_auto_panel::OpenLauncher` action, dispatched from a keymap or
    /// the command palette. Retained across the fold so no existing user
    /// keybinding stops working.
    OpenLauncherAction,
    /// The `+` button on the run monitor rail.
    RunMonitorNewRun,
    /// The "New run" button on a finished run's surface.
    RunSurfaceNewRun,
}

impl LaunchOrigin {
    /// Every admitted origin, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::NewThreadMenuItem,
            Self::OpenLauncherAction,
            Self::RunMonitorNewRun,
            Self::RunSurfaceNewRun,
        ]
    }

    /// The stable token for this origin, recorded with the run.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::NewThreadMenuItem => "new_thread_menu_item",
            Self::OpenLauncherAction => "open_launcher_action",
            Self::RunMonitorNewRun => "run_monitor_new_run",
            Self::RunSurfaceNewRun => "run_surface_new_run",
        }
    }
}

/// Reaching the launch surface is not starting a run.
///
/// Every [`LaunchOrigin`] opens the Full Auto launch surface with an unsent
/// draft. Starting the run is a second, separate human act on the "Start Full
/// Auto" button. Two gestures, both human, is the whole guard — and it is the
/// reason the fold does not need a mode flag on the composer.
#[must_use]
pub const fn origin_starts_a_run(_origin: LaunchOrigin) -> bool {
    false
}

// -------------------------------------------------------------------------
// The affordance ledger
// -------------------------------------------------------------------------

/// Where a Full Auto panel affordance lives after the fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldedHome {
    /// The same view, rendered by the agent panel instead of by a dock panel.
    /// Nothing about the control changed except its parent.
    AgentPanelFullAutoSurface,
    /// Withdrawn, with the reason. Read the reason before assuming it was an
    /// oversight.
    Withdrawn(&'static str),
}

/// One interactive affordance of the Full Auto panel, and its home after the
/// fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Affordance {
    /// The GPUI element id, exactly as written in
    /// `crates/full_auto_ui/src/`.
    pub element_id: &'static str,
    /// What the control does, in the user's terms.
    pub does: &'static str,
    /// Where it lives now.
    pub home: FoldedHome,
}

/// Every interactive affordance the Full Auto panel expressed before the fold.
///
/// The owner asked for the panel to be folded into the Omega chat UI. "Folded"
/// is not "reduced", so this table exists to make a loss impossible to commit
/// by accident: `every_full_auto_affordance_is_mapped` scans
/// `crates/full_auto_ui/src/` for element ids and fails if one is missing here
/// or listed here and gone from the source.
///
/// If you are adding a control to the Full Auto surface and this test failed:
/// add a row. That is the whole fix, and the row is the record of where your
/// control lives after the fold.
pub const FULL_AUTO_AFFORDANCES: &[Affordance] = &[
    Affordance {
        element_id: "full-auto-panel",
        does: "The launch and run surface root, and its focus target.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-openagents-connect",
        does: "Connect or reconnect the OpenAgents Sync account.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-openagents-disconnect",
        does: "Revoke the OpenAgents Sync account credentials.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-provider-account",
        does: "One connected provider account: readiness, quota, lane, ref.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-advanced-toggle",
        does: "Reveal title, done condition, and turn cap on the draft.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-start",
        does: "Start the run. The only path to engine-lane run authority.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-cancel",
        does: "Clear the draft without starting anything.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-pause",
        does: "Pause a running run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-resume",
        does: "Resume a paused run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-handoff",
        does: "Hand a paused run to the other local lane.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-retry",
        does: "Retry a stalled run whose recovery action is retry_now.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-stop",
        does: "Stop a non-terminal run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-new",
        does: "Return to the launch surface from a run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-evidence-chain",
        does: "The host-verified evidence chain for the active run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-monitor",
        does: "The concurrent run monitor rail.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-monitor-new",
        does: "Start a new Full Auto draft from the monitor rail.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-run-row",
        does: "Open one run from the monitor rail.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
];

/// What the fold costs, stated rather than discovered.
///
/// Every *control* survives the fold — the ledger above proves that
/// mechanically, because the same views render under a new parent. Two
/// capabilities of the panel were not controls, and they do not survive. They
/// are recorded here because the owner asked for a fold, not a reduction, and
/// a reduction that nobody wrote down is indistinguishable from a bug.
pub const FOLD_COSTS: &[&str] = &[
    "Independent dock placement. Full Auto had its own DockPosition::Right and \
     its own 520px default width, so it could sit opposite the agent panel. It \
     now inherits the agent panel's dock and size.",
    "Simultaneous full detail. A separate dock panel could show a run's full \
     detail while the agent panel showed a chat thread. One panel shows one \
     surface at a time, so watching a run in full while typing in a thread is \
     no longer possible. Active runs still render on the front door beneath \
     the composer, and the monitor rail still lists them, so noticing a run is \
     preserved; reading one in full alongside a thread is not.",
];

// -------------------------------------------------------------------------
// Checks
// -------------------------------------------------------------------------

/// Repository-root-relative path resolution.
///
/// `CARGO_MANIFEST_DIR` is `crates/omega_front_door`, so the root is two
/// levels up.
#[must_use]
pub fn repository_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Every `"full-auto-…"` string literal in a source file.
///
/// GPUI element ids are string literals, so this is a lexical scan. It is
/// deliberately generous: it will pick up a `"full-auto-…"` literal that is
/// not an element id, which fails the ledger check and forces a human to look.
/// The opposite error — missing a real control — is the one that would let a
/// capability disappear silently.
#[must_use]
pub fn full_auto_element_ids(source: &str) -> std::collections::BTreeSet<String> {
    let mut ids = std::collections::BTreeSet::new();
    let bytes = source.as_bytes();
    let needle = b"\"full-auto";
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        if &bytes[index..index + needle.len()] == needle {
            let start = index + 1;
            if let Some(offset) = source[start..].find('"') {
                ids.insert(source[start..start + offset].to_owned());
                index = start + offset + 1;
                continue;
            }
        }
        index += 1;
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The front door replaces the empty buffer and nothing else.
    #[test]
    fn a_fresh_launch_lands_on_the_new_agent_thread_surface() {
        assert_eq!(
            launch_surface(RestoreOnStartup::Other),
            LaunchSurface::NewAgentThread,
            "a window with nothing to restore must open on the front door, \
             not on upstream Zed's empty untitled buffer"
        );
    }

    /// Overriding a deliberate launchpad choice would be Omega ignoring a
    /// setting, which is a different bug from the one the front door fixes.
    #[test]
    fn the_launchpad_startup_behaviour_still_wins() {
        assert_eq!(
            launch_surface(RestoreOnStartup::Launchpad),
            LaunchSurface::Nothing
        );
        assert_ne!(
            launch_surface(RestoreOnStartup::Launchpad),
            LaunchSurface::EmptyBuffer,
            "the launchpad path never opened a buffer and must not start"
        );
    }

    /// The disclosure is a record. A label is derived from it.
    ///
    /// The failure this guards is subtle: storing the rendered line would make
    /// the owner's reversible identity decision irreversible, because moving to
    /// a signing principal would then need every stored thread rewritten.
    #[test]
    fn disclosure_renders_from_its_fields() {
        let disclosure = ExecutorDisclosure {
            class: ExecutorClass::EngineLane,
            agent_id: "codex-local".into(),
            provider: Some("google".into()),
            model: Some("gemini-3.6-flash".into()),
            run_ref: Some("run.abc".into()),
            route: None,
        };
        assert_eq!(
            disclosure.label(),
            "engine_lane · codex-local · google/gemini-3.6-flash · run.abc"
        );

        // Change one field and the rendered line follows, which is only true
        // because nothing cached it.
        let mut moved = disclosure;
        moved.class = ExecutorClass::NativeLoop;
        moved.run_ref = None;
        assert_eq!(
            moved.label(),
            "native_loop · codex-local · google/gemini-3.6-flash"
        );
    }

    /// An executor that does not report a model is disclosed as not reporting
    /// one. omega#77.
    ///
    /// The failure this guards is a line that reads as complete while a part
    /// of it is missing: `external_acp · codex-acp` alone gives the reader no
    /// way to tell a fully disclosed thread from a partly disclosed one.
    #[test]
    fn an_undisclosed_model_is_said_rather_than_skipped() {
        let disclosure = ExecutorDisclosure {
            class: ExecutorClass::ExternalAcp,
            agent_id: "codex-acp".into(),
            provider: None,
            model: None,
            run_ref: None,
            route: None,
        };
        assert!(disclosure.is_coherent());
        assert_eq!(
            disclosure.label(),
            "external_acp · codex-acp · model not disclosed"
        );

        let half_known = ExecutorDisclosure {
            provider: Some("openai".into()),
            ..disclosure
        };
        assert_eq!(
            half_known.label(),
            "external_acp · codex-acp · openai/model not disclosed"
        );
    }

    /// A struct with no field to hold a rendered label cannot accidentally
    /// grow one without this test's author noticing.
    #[test]
    fn the_disclosure_record_holds_no_rendered_label() {
        let disclosure = ExecutorDisclosure {
            class: ExecutorClass::NativeLoop,
            agent_id: "a".into(),
            provider: Some("p".into()),
            model: Some("m".into()),
            run_ref: None,
            route: None,
        };
        // Debug is the closest thing to reflection available here: if a
        // `label` field is ever added, it shows up in the struct dump.
        let dumped = format!("{disclosure:?}");
        assert!(
            !dumped.contains("label"),
            "ExecutorDisclosure grew a stored label field: {dumped}. \
             The owner admitted the non-signing identity choice on the \
             condition that disclosure stays a typed record a label renders."
        );
    }

    /// A run reference on a native turn is a routed result wearing the wrong
    /// name.
    #[test]
    fn only_an_engine_lane_carries_a_run_reference() {
        let base = ExecutorDisclosure {
            class: ExecutorClass::EngineLane,
            agent_id: "codex-local".into(),
            provider: Some("google".into()),
            model: Some("gemini-3.6-flash".into()),
            run_ref: Some("run.abc".into()),
            route: None,
        };
        assert!(base.is_coherent());

        let mut engine_without_run = base.clone();
        engine_without_run.run_ref = None;
        assert!(!engine_without_run.is_coherent());

        let mut native_with_run = base.clone();
        native_with_run.class = ExecutorClass::NativeLoop;
        assert!(!native_with_run.is_coherent());

        // A present-but-empty identifier stays incoherent after omega#77 made
        // these optional. Absent means "not disclosed"; empty means something
        // built the record out of a missing value and lost the distinction.
        let mut blank_model = base.clone();
        blank_model.model = Some(String::new());
        assert!(!blank_model.is_coherent());

        let mut blank_provider = base;
        blank_provider.provider = Some(String::new());
        assert!(!blank_provider.is_coherent());
    }

    /// omega#78. A thread routed by a fallback says so on its own line.
    ///
    /// This is the "prove it is disclosed rather than silent" half of the
    /// engine-down exit property. The record carries the typed reason; the line
    /// renders it; neither stores a sentence.
    #[test]
    fn an_engine_down_fallback_is_visible_on_the_disclosure_line() {
        let decision = route(
            &RouteInputs::native_only()
                .with_engine(EngineReadiness::Unreachable(EngineUnreachable::NotRunning))
                .pinned(ExecutorPin::on_lane("claude-local")),
        );
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);

        let disclosure = ExecutorDisclosure {
            class: decision.chosen,
            agent_id: "omega-agent".into(),
            provider: Some("anthropic".into()),
            model: Some("claude-opus-5".into()),
            run_ref: None,
            route: Some(decision.disclosed_route()),
        };
        assert!(disclosure.is_coherent());

        let line = disclosure.label();
        assert!(
            line.contains("engine unreachable, fell back to the native loop"),
            "the thread ran somewhere the user did not pin and the line does \
             not say so: {line}"
        );

        // The same record with the route dropped renders a line that reads as
        // an ordinary native thread. That silence is the defect.
        let silent = ExecutorDisclosure {
            route: None,
            ..disclosure
        };
        assert!(!silent.label().contains("fell back"));
    }

    /// omega#78. A record cannot say both "the pin was not honoured" and "an
    /// engine lane ran it", and cannot say an unpinned thread reached Full Auto
    /// authority.
    #[test]
    fn a_route_that_contradicts_the_executor_is_incoherent() {
        let engine_thread = ExecutorDisclosure {
            class: ExecutorClass::EngineLane,
            agent_id: "codex-local".into(),
            provider: None,
            model: None,
            run_ref: Some("run.abc".into()),
            route: Some(RouteReason::PinHonored),
        };
        assert!(engine_thread.is_coherent());

        for contradiction in [
            RouteReason::EngineUnreachable,
            RouteReason::EngineAtCapacity,
            RouteReason::EngineHasNoReadyLane,
            RouteReason::PinnedLaneUnavailable,
            RouteReason::ExternalAcpUnavailable,
            RouteReason::UnrecordableLane,
            // Owner gate 8: nothing unpinned reaches an engine lane.
            RouteReason::UnpinnedDefault,
        ] {
            let lying = ExecutorDisclosure {
                route: Some(contradiction),
                ..engine_thread.clone()
            };
            assert!(
                !lying.is_coherent(),
                "{} on an engine-lane thread passed coherence",
                contradiction.token()
            );
        }
    }

    /// `OMEGA-AGENT-AC-04` fixes the executor set at exactly three.
    #[test]
    fn the_admitted_executor_set_is_exactly_three() {
        let tokens: Vec<&str> = ExecutorClass::all().iter().map(|c| c.token()).collect();
        assert_eq!(tokens, ["native_loop", "external_acp", "engine_lane"]);
    }

    /// Owner gate 8. The allowlist is written out so that adding an origin is
    /// a deliberate edit to a test that says why the list is closed.
    #[test]
    fn origins_are_all_human_gestures() {
        let tokens: Vec<&str> = LaunchOrigin::all().iter().map(|o| o.token()).collect();
        assert_eq!(
            tokens,
            [
                "new_thread_menu_item",
                "open_launcher_action",
                "run_monitor_new_run",
                "run_surface_new_run",
            ],
            "every Full Auto launch origin must be a visible control a person \
             operates. A tool call, a slash command, a restored draft, an agent \
             turn, or a composer mode flag is not one. If you are adding an \
             origin, prove it is a human gesture before you edit this list."
        );
    }

    /// Reaching the launch surface is not starting a run.
    #[test]
    fn no_origin_starts_a_run_by_itself() {
        for origin in LaunchOrigin::all() {
            assert!(
                !origin_starts_a_run(*origin),
                "{} reached the launch surface and started a run in one \
                 gesture; the Start button is the second human act",
                origin.token()
            );
        }
    }

    /// The fold carries every control, and the ledger proves it against the
    /// source rather than against its author's memory.
    #[test]
    fn every_full_auto_affordance_is_mapped() {
        let directory = repository_path("crates/full_auto_ui/src");
        let mut found = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(&directory).expect("full_auto_ui sources are readable") {
            let path = entry.expect("directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("source is readable");
            found.extend(full_auto_element_ids(&source));
        }

        assert!(
            !found.is_empty(),
            "the element-id scan found nothing in {}; the ledger check would \
             be vacuous",
            directory.display()
        );

        let mapped: std::collections::BTreeSet<String> = FULL_AUTO_AFFORDANCES
            .iter()
            .map(|a| a.element_id.to_owned())
            .collect();

        let unmapped: Vec<&String> = found.difference(&mapped).collect();
        assert!(
            unmapped.is_empty(),
            "Full Auto controls with no entry in FULL_AUTO_AFFORDANCES: \
             {unmapped:?}. The owner asked for Full Auto to be folded into the \
             chat UI, not reduced. Add a row saying where your control lives \
             after the fold."
        );

        let stale: Vec<&String> = mapped.difference(&found).collect();
        assert!(
            stale.is_empty(),
            "FULL_AUTO_AFFORDANCES maps controls that no longer exist: \
             {stale:?}. A ledger that outlives its source stops being evidence."
        );
    }

    /// A scan that reaches nothing passes every check it is asked to make.
    #[test]
    fn the_element_id_scan_reaches_real_ids() {
        let ids = full_auto_element_ids(
            r#"Button::new("full-auto-start", "Start").id(("full-auto-run-row", index))"#,
        );
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("full-auto-start"));
        assert!(ids.contains("full-auto-run-row"));

        // An unterminated literal must not swallow the rest of the file.
        assert!(full_auto_element_ids("\"full-auto-start").is_empty());
    }

    /// The costs are stated, not discovered.
    #[test]
    fn the_fold_costs_are_written_down() {
        assert_eq!(
            FOLD_COSTS.len(),
            2,
            "FOLD_COSTS is the honest half of the ledger. If the fold now costs \
             more or less, say so here."
        );
        for cost in FOLD_COSTS {
            assert!(cost.len() > 80, "a cost stated in a phrase is not stated");
        }
    }
}
