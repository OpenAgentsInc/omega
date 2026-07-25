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
/// field to put a rendered label in. omega#77 populates this; omega#76 fixes
/// its shape and leaves the front door room to render it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorDisclosure {
    /// Which of the three admitted classes ran the work.
    pub class: ExecutorClass,
    /// The executor's own identifier, as the executor reports it.
    pub agent_id: String,
    /// The model provider that served the turn.
    pub provider: String,
    /// The model that served the turn.
    pub model: String,
    /// The engine run this thread is bound to, where one applies.
    ///
    /// `Some` only for [`ExecutorClass::EngineLane`]: a native or ACP turn has
    /// no run authority to reference.
    pub run_ref: Option<String>,
}

impl ExecutorDisclosure {
    /// Render the disclosure line for a thread.
    ///
    /// Derived on every call. Nothing stores the output.
    #[must_use]
    pub fn label(&self) -> String {
        let mut line = format!(
            "{} · {} · {}/{}",
            self.class.token(),
            self.agent_id,
            self.provider,
            self.model
        );
        if let Some(run_ref) = &self.run_ref {
            line.push_str(" · ");
            line.push_str(run_ref);
        }
        line
    }

    /// Whether this record is internally consistent.
    ///
    /// A run reference on a native turn means something mislabelled a routed
    /// result, which `OMEGA-AGENT-AC-05` exists to prevent.
    #[must_use]
    pub fn is_coherent(&self) -> bool {
        !self.agent_id.is_empty()
            && !self.provider.is_empty()
            && !self.model.is_empty()
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
            provider: "google".into(),
            model: "gemini-3.6-flash".into(),
            run_ref: Some("run.abc".into()),
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

    /// A struct with no field to hold a rendered label cannot accidentally
    /// grow one without this test's author noticing.
    #[test]
    fn the_disclosure_record_holds_no_rendered_label() {
        let disclosure = ExecutorDisclosure {
            class: ExecutorClass::NativeLoop,
            agent_id: "a".into(),
            provider: "p".into(),
            model: "m".into(),
            run_ref: None,
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
            provider: "google".into(),
            model: "gemini-3.6-flash".into(),
            run_ref: Some("run.abc".into()),
        };
        assert!(base.is_coherent());

        let mut engine_without_run = base.clone();
        engine_without_run.run_ref = None;
        assert!(!engine_without_run.is_coherent());

        let mut native_with_run = base.clone();
        native_with_run.class = ExecutorClass::NativeLoop;
        assert!(!native_with_run.is_coherent());

        let mut blank_model = base;
        blank_model.model = String::new();
        assert!(!blank_model.is_coherent());
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
