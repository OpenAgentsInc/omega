//! The composer's executor selector. `OMEGA-DELTA-0115`.
//!
//! The owner asked for this while looking at the running app, in as many
//! words: *"I need to be able to switch the executor. No selection of model
//! like Gemini. You select between Omega, Exo, Codex, Claude, and Grok. If some of those
//! are not implemented yet, don't add them, but that's what I want. Whichever
//! of those are ready, put those in now."* Grok extends that runtime catalog.
//!
//! So the control this module renders names a **runtime**, never a model. What
//! it replaced in the composer bar named `google/gemini-3.6-flash`, which is
//! the answer to a question the owner was not asking.
//!
//! # One public executor, with Exo admitted only by an explicit launch
//!
//! [`SelectableExecutor`] retains the five runtime identities because routing,
//! disclosure, thread recovery, and adapter warming need to distinguish them.
//! This legacy router selector remains narrower: Omega and opted-in Exo only.
//! Direct ACP ownership is selected at the three-mode front door, where an
//! exact `Agent::Custom` conversation is constructed.
//!
//! [`ready`] remains the capability inventory used by background warming.
//! [`selectable`] applies the separate public-selection policy:
//!
//! - **Omega** is the native loop, which is compiled in. Always ready.
//! - **Codex**, **Claude**, and **Grok** are ready when `omega_agent_detect` finds their
//!   binary on `PATH` *and* `omega_agent_attach` can host them over ACP. Both
//!   halves matter: GitHub Copilot and Cursor are detected and have no ACP
//!   server here, so offering one would produce a failure at connect time
//!   rather than a thread.
//! - **Exo** is ready when a lane resolves — the owner's lane file, or a lane
//!   derived from an Exo install by
//!   `omega_agent_detect::exo::derive_lane_from_env`. Both are
//!   `ExoLaneConfig::resolve`, which is the exact predicate the router uses
//!   when it attaches, so the list cannot offer a lane the attach then
//!   declines.
//!
//! Keeping these lists separate lets the UI explain a detected but unusable
//! runtime without pretending that detection created an ACP session.
//!
//! # Why choosing re-attaches instead of re-pinning
//!
//! `OmegaAgentConnection` holds **one** external-ACP slot, filled once, when
//! the connection is built: the Exo lane if there is one, otherwise the
//! detected agent. `pin_session` chooses between the *classes* the router
//! already holds — it cannot make Claude reachable on a machine where Codex
//! filled that slot. So a pin alone could never honour the external choices
//! names, and a control that silently did nothing for two of them would be the
//! worst version of this.
//!
//! What actually switches is therefore what gets **attached**. [`select`]
//! records the person's standing choice, [`attach_plan`] turns it into the two
//! facts the router's `connect` needs, and the caller re-connects. That is the
//! shape `OMEGA-DELTA-0114` already established with
//! `omega_agent_attach::run_on_omegas_own_loop`: a person reads something,
//! presses a control, and the next connection attaches somewhere else. This
//! generalises the destination; it does not invent the mechanism.
//!
//! # What happens to a thread mid-conversation: nothing, and it says so
//!
//! The owner settled this directly: *"that control should be disabled mid-turn,
//! only settable on new convos - may revisit that later but thats it for now"*.
//!
//! So the control is live on a conversation that has not run anything and
//! disabled once it has, and the disabled tooltip says which executor is
//! running and that a new conversation is how to change it. It is the same
//! rule `OMEGA-DELTA-0094` holds for the audience control and for the same
//! reason: re-attaching underneath a transcript would leave entries above the
//! fold that one executor produced and entries below it that another did, with
//! one disclosure line over both.
//!
//! The alternative — re-attach for the next turn — is defensible and is not
//! what was asked for. What is not defensible is doing one while the label
//! implies the other, so the label is the disabled state's whole job.

use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use gpui::{AnyElement, App, Window};
use omega_agent_detect::DetectedAgent;
use omega_front_door::ExecutorClass;
use ui::{Button, ContextMenu, ContextMenuEntry, PopoverMenu, Tooltip, prelude::*};

use crate::omega_exo_connection::ExoLaneConfig;

/// The header over the menu's list.
pub const SELECTION_MENU_HEADER: &str = "Run this conversation on";

/// What the menu says under the list, on a conversation that can still change.
pub const CHOOSING_RECONNECTS: &str =
    "Switching reconnects this conversation to the executor you pick.";

/// What the disabled control says once a conversation has run something.
///
/// Written once, here, rather than at the call site, for the reason
/// `omega_audience`'s two sentences are: this is the least verifiable part of
/// the feature and the one a person actually reads.
pub const ONLY_BEFORE_THE_FIRST_MESSAGE: &str = "The executor is chosen before the first message. Start a new conversation \
     to run on a different one.";

/// The five executors a person may choose between.
///
/// Closed, and closed on purpose. The named choices are a
/// product decision, and a `&str` here would let a later edit add a fifth name
/// with no review — which is exactly how the wire tokens `OMEGA-DELTA-0055`
/// removed had reached a person in the first place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableExecutor {
    /// Omega's own agent loop, in `crates/agent`. Compiled in.
    Omega,
    /// The Exo harness lane, over ACP.
    Exo,
    /// Codex, through the `codex-acp` adapter.
    Codex,
    /// Claude, through the `claude-acp` adapter.
    Claude,
    /// Grok, through its native ACP server.
    Grok,
}

impl SelectableExecutor {
    /// Every name, in the order the menu offers them.
    ///
    /// Omega first because it is the one that is always there, and the order
    /// is fixed here rather than derived from what is installed so the menu
    /// does not reorder itself between launches.
    pub const ALL: &'static [Self] = &[
        Self::Omega,
        Self::Exo,
        Self::Codex,
        Self::Claude,
        Self::Grok,
    ];

    /// The name a person reads. This is the only rendering of this type.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Omega => "Omega",
            Self::Exo => "Exo",
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::Grok => "Grok",
        }
    }

    /// The name used in the public executor menu.
    #[must_use]
    pub const fn selector_name(self) -> &'static str {
        self.name()
    }

    /// The stable token this choice is logged under.
    ///
    /// Separate from [`name`](Self::name) because one is for a log and one is
    /// for a window, and a single string used for both drifts towards whichever
    /// reader complained last.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Omega => "omega",
            Self::Exo => "exo",
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Grok => "grok",
        }
    }

    /// One sentence saying what this actually is.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Omega => "Omega's own agent loop, running in this window.",
            Self::Exo => "The Exo harness lane, driving your Exo install.",
            Self::Codex => "Codex on this machine, through the codex-acp adapter.",
            Self::Claude => "Claude on this machine, through the claude-acp adapter.",
            Self::Grok => "Grok on this machine, through its native ACP server.",
        }
    }

    /// The ACP adapter id this name attaches through, for the three that are
    /// detected agents.
    ///
    /// Taken from `agent_servers` rather than spelled again, so an id renamed
    /// there cannot leave this pointing at an adapter that no longer exists.
    #[must_use]
    pub const fn adapter_id(self) -> Option<&'static str> {
        match self {
            Self::Codex => Some(agent_servers::CODEX_ID),
            Self::Claude => Some(agent_servers::CLAUDE_AGENT_ID),
            Self::Grok => Some(agent_servers::GROK_ID),
            Self::Omega | Self::Exo => None,
        }
    }

    /// Which of the five a live thread is running on, if it is one of them.
    ///
    /// `None` for an engine lane, and for an external agent that is not one of
    /// the three named here. Neither is a sixth choice: the first is Full Auto
    /// authority, which this control does not reach, and the second is
    /// somebody else's adapter. A control that answered `Omega` for either
    /// would be the dishonest attribution `omega#77` exists to stop.
    #[must_use]
    pub fn of(class: ExecutorClass, agent_id: &str) -> Option<Self> {
        match class {
            ExecutorClass::NativeLoop => Some(Self::Omega),
            ExecutorClass::EngineLane => None,
            ExecutorClass::ExternalAcp => {
                if agent_id == omega_exo_lane::EXO_HARNESS_ID {
                    Some(Self::Exo)
                } else {
                    Self::ALL
                        .iter()
                        .copied()
                        .find(|choice| choice.adapter_id() == Some(agent_id))
                }
            }
        }
    }
}

/// Executor names admitted by this process launch.
///
/// Exo remains part of the closed product vocabulary, but it is not loaded or
/// exposed unless the command line opted in before application startup.
#[must_use]
pub fn runtime_choices() -> &'static [SelectableExecutor] {
    const WITHOUT_EXO: &[SelectableExecutor] = &[
        SelectableExecutor::Omega,
        SelectableExecutor::Codex,
        SelectableExecutor::Claude,
        SelectableExecutor::Grok,
    ];

    if omega_front_door::exo_enabled() {
        SelectableExecutor::ALL
    } else {
        WITHOUT_EXO
    }
}

/// The names that can actually run a turn here, in [`SelectableExecutor::ALL`]
/// order.
///
/// A pure function of what was found, so the rule can be checked without a
/// machine that happens to have the right things installed — the same reason
/// `omega_agent_detect::detect_on_path` takes `PATH` as a parameter.
#[must_use]
pub fn ready(detected: &[DetectedAgent], exo_lane_resolves: bool) -> Vec<SelectableExecutor> {
    SelectableExecutor::ALL
        .iter()
        .copied()
        .filter(|choice| match choice {
            // Compiled in. There is no machine where this is missing, and a
            // list that could be empty would be a composer with no executor.
            SelectableExecutor::Omega => true,
            SelectableExecutor::Exo => exo_lane_resolves,
            SelectableExecutor::Codex | SelectableExecutor::Claude | SelectableExecutor::Grok => {
                let Some(adapter) = choice.adapter_id() else {
                    return false;
                };
                // Both halves. Detected proves the binary exists; drivable
                // proves Omega hosts an ACP adapter for it. Either alone
                // offers a name that fails when it is clicked.
                detected.iter().any(|agent| agent.id == adapter)
                    && crate::omega_agent_attach::DRIVABLE_AGENT_IDS.contains(&adapter)
            }
        })
        .collect()
}

/// [`ready`] against this machine.
#[must_use]
pub fn ready_here() -> Vec<SelectableExecutor> {
    let detected = omega_agent_detect::detected();
    if omega_front_door::exo_enabled() {
        ready(detected, exo_lane_resolves())
    } else {
        ready(detected, false)
            .into_iter()
            .filter(|choice| *choice != SelectableExecutor::Exo)
            .collect()
    }
}

/// Apply the public executor-selection policy to a capability inventory.
///
/// The legacy Omega-router selection policy. Direct ACP agents are not routed
/// through this selector; the front door constructs their exact owner.
#[must_use]
pub fn selectable(ready: &[SelectableExecutor], exo_enabled: bool) -> Vec<SelectableExecutor> {
    ready
        .iter()
        .copied()
        .filter(|choice| {
            *choice == SelectableExecutor::Omega
                || (exo_enabled && *choice == SelectableExecutor::Exo)
        })
        .collect()
}

/// The executors a person may select in this process.
#[must_use]
pub fn selectable_here() -> Vec<SelectableExecutor> {
    selectable(&ready_here(), omega_front_door::exo_enabled())
}

/// Why each name that cannot run a turn here is not on the ready list.
///
/// `OMEGA-DELTA-0123`. The complement of [`ready`], and the correction to the
/// version of this file that had only [`ready`].
///
/// A name that is simply left out is indistinguishable from a name that does
/// not exist. The owner opened this menu, did not find Exo, and had to ask why
/// — which is the same cost the short list was meant to stop somebody paying
/// after a click, moved to before it. Both halves are kept because they answer
/// different questions: [`ready`] decides what may be **clicked**, and this
/// decides what may be **read**.
///
/// `exo_absence` is `None` exactly when the lane resolves, so the two functions
/// cannot disagree about Exo — this one derives its own answer by calling
/// [`ready`], and `every_name_is_either_ready_or_explained` asserts the two
/// lists partition [`SelectableExecutor::ALL`] with nothing in both and nothing
/// in neither.
///
/// A pure function of what was found, for the reason [`ready`] is one.
#[must_use]
pub fn unavailable(
    detected: &[DetectedAgent],
    exo_absence: Option<&'static str>,
) -> Vec<(SelectableExecutor, &'static str)> {
    let ready = ready(detected, exo_absence.is_none());
    SelectableExecutor::ALL
        .iter()
        .copied()
        .filter(|choice| !ready.contains(choice))
        .map(|choice| {
            let reason = match choice {
                // Unreachable: `ready` never omits the native loop. Written as
                // a value rather than a panic because a menu is not worth
                // aborting over, and written at all because a total match is
                // what stops a fifth variant arriving here unnoticed.
                SelectableExecutor::Omega => "compiled in",
                SelectableExecutor::Exo => exo_absence.unwrap_or("no Exo lane"),
                SelectableExecutor::Codex
                | SelectableExecutor::Claude
                | SelectableExecutor::Grok => {
                    // The two halves `ready` checks, told apart. "Installed and
                    // Omega cannot drive it" and "not installed" are different
                    // things to do next, and collapsing them would send
                    // somebody to install what they already have.
                    if choice
                        .adapter_id()
                        .is_some_and(|adapter| detected.iter().any(|agent| agent.id == adapter))
                    {
                        "installed; Omega hosts no adapter for it"
                    } else {
                        "not installed"
                    }
                }
            };
            (choice, reason)
        })
        .collect()
}

/// [`unavailable`] against this machine.
#[must_use]
pub fn unavailable_here() -> Vec<(SelectableExecutor, &'static str)> {
    let detected = omega_agent_detect::detected();
    if omega_front_door::exo_enabled() {
        unavailable(detected, exo_absence_here())
    } else {
        unavailable(detected, Some("disabled for this launch"))
            .into_iter()
            .filter(|(choice, _)| *choice != SelectableExecutor::Exo)
            .collect()
    }
}

/// Disabled rows shown by the public executor menu.
///
/// Every unavailable row carries the machine-specific capability reason.
#[must_use]
pub fn selector_unavailable_here() -> Vec<(SelectableExecutor, &'static str)> {
    let selectable = selectable_here();
    let unavailable = unavailable_here();

    runtime_choices()
        .iter()
        .copied()
        .filter_map(|choice| {
            if selectable.contains(&choice) {
                return None;
            }
            unavailable
                .iter()
                .find(|(candidate, _)| *candidate == choice)
                .copied()
                .or_else(|| {
                    (choice == SelectableExecutor::Exo)
                        .then_some((choice, "Exo is unavailable for this launch"))
                })
        })
        .collect()
}

/// Why no Exo lane resolves on this machine, when none does.
///
/// `OMEGA-DELTA-0123`. The two rules are `ExoLaneConfig::resolve`'s own, in its
/// order, because a second opinion about which lane this machine has is exactly
/// the half-read configuration `OMEGA-DELTA-0042` exists to prevent:
///
/// 1. A lane file that **exists** is the answer, even when it is broken.
///    `resolve` does not fall through to derivation there, so neither does
///    this, and the sentence says the file rather than the install.
/// 2. Otherwise the derivation's own refusal, in [`ExoLaneUnderivable::summary`]
///    's width.
///
/// `resolve`'s third rule — derive only for the product's own lane path — is
/// satisfied by construction here, because the path is `data_dir_path()` and
/// nothing else can be passed in.
///
/// Cached for the life of the process for the reason
/// `omega_agent_detect::detected` is: the composer asks this on every draw and
/// answering it walks the filesystem.
///
/// [`ExoLaneUnderivable::summary`]: omega_agent_detect::exo::ExoLaneUnderivable::summary
#[must_use]
pub fn exo_absence_here() -> Option<&'static str> {
    if !omega_front_door::exo_enabled() {
        return Some("disabled for this launch");
    }
    static ABSENCE: OnceLock<Option<&'static str>> = OnceLock::new();
    *ABSENCE.get_or_init(|| {
        let path = ExoLaneConfig::data_dir_path();
        if path.exists() {
            return ExoLaneConfig::load(&path)
                .is_none()
                .then_some("its lane file cannot be read");
        }
        omega_agent_detect::exo::derive_lane_from_env()
            .err()
            .map(|underivable| {
                // The whole refusal, with every path it looked at, goes to the
                // log. The menu gets the sentence. Both, because the short one
                // is what gets read and the long one is what gets acted on.
                log::info!("OMEGA-DELTA-0123: Exo is not offered: {underivable}");
                underivable.summary()
            })
    })
}

/// Whether an Exo lane resolves on this machine.
///
/// `ExoLaneConfig::resolve` is deliberately the predicate rather than
/// `derive_lane_from_env` alone: resolve is the owner's lane file *or* a
/// derived lane, and it is the exact call the router makes when it attaches.
/// Asking a narrower question here would hide Exo from a machine where the
/// owner wrote a lane file by hand, and asking a wider one would offer a lane
/// the attach then declines.
///
/// Defined as "there is no absence" rather than as a second reading of the same
/// files. `OMEGA-DELTA-0123` added [`exo_absence_here`], and two cached answers
/// to one question is how a menu ends up disabling a name it is also offering.
#[must_use]
pub fn exo_lane_resolves() -> bool {
    exo_absence_here().is_none()
}

/// A person's standing choice of executor, if they have made one.
///
/// Process-global and not persisted, exactly like
/// `omega_agent_attach::run_on_omegas_own_loop`'s flag, and for a sharper
/// version of the same reason: what is installed can change between launches,
/// and a persisted choice of an agent that is no longer here would be a
/// composer refusing to attach anything with no visible cause.
static SELECTED: Mutex<Option<SelectableExecutor>> = Mutex::new(None);

/// The executor a person chose, if they chose one.
///
/// `None` is *no choice made*, which is not the same as choosing Omega. It
/// means the router attaches by its own rule — the Exo lane when this launch
/// opted in, then the detected agent.
#[must_use]
pub fn selected() -> Option<SelectableExecutor> {
    let selected = *SELECTED
        .lock()
        .expect("the executor selection is never held across a panic");
    selected.filter(|choice| omega_front_door::exo_enabled() || *choice != SelectableExecutor::Exo)
}

/// Choose the executor the next connection attaches.
///
/// **Only a person may call this.** It is the whole of the switching mechanism,
/// and called from a turn, a tool, or a retry it would be a thread quietly
/// moving executors — which is the defect class `omega#77`'s disclosure exists
/// to make impossible.
pub fn select(choice: SelectableExecutor) {
    if matches!(
        choice,
        SelectableExecutor::Codex | SelectableExecutor::Claude | SelectableExecutor::Grok
    ) {
        log::warn!(
            "ignored routed {} selection; choose it as a Direct Agent owner at the front door",
            choice.selector_name()
        );
        return;
    }
    if choice == SelectableExecutor::Exo && !omega_front_door::exo_enabled() {
        log::warn!(
            "OMEGA-DELTA-0144: ignored an Exo selection because this process \
             was not launched with --enable-exo"
        );
        return;
    }
    log::info!(
        "OMEGA-DELTA-0115: a person chose {} ({}) as this session's executor",
        choice.name(),
        choice.token()
    );
    *SELECTED
        .lock()
        .expect("the executor selection is never held across a panic") = Some(choice);
}

/// Stand a selection up for a test, without pretending to be the control.
///
/// Deliberately not [`select`]. `OMEGA-DELTA-0115` holds that `select` is
/// called from the composer's own control and nowhere else — a choice made by a
/// turn, a tool or a retry is a thread moving executor without its reader
/// asking — and that check is worth keeping exact. A test needs the *state*,
/// not the entry point, so it takes the state directly and the rule about who
/// may choose stays as narrow as it was written.
#[cfg(any(test, feature = "test-support"))]
pub fn select_for_test(choice: SelectableExecutor) {
    *SELECTED
        .lock()
        .expect("the executor selection is never held across a panic") = Some(choice);
}

/// Put the process-wide selection back to *no choice made*.
///
/// Test-only, and only because the selection is a process global: a test that
/// chooses an executor would otherwise leave that choice standing for every
/// test that runs after it in the same binary.
#[cfg(any(test, feature = "test-support"))]
pub fn clear_selection_for_test() {
    *SELECTED
        .lock()
        .expect("the executor selection is never held across a panic") = None;
}

/// What the router should attach, given a person's standing choice.
///
/// Two facts and nothing else, because those are the two doors
/// `OmegaRouterServer::connect` has: whether to try the Exo lane, and which
/// detected agents to offer the attach. Everything else about the choice is
/// spent here so the router keeps one shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachPlan {
    /// Whether to look for an Exo lane at all.
    pub exo: bool,
    /// The detected agents the attach may choose from, in detection order.
    pub agents: Vec<DetectedAgent>,
}

/// Turn a choice into the plan the router's `connect` follows.
///
/// `None` tries the Exo lane only when this launch explicitly enabled it,
/// otherwise it proceeds directly to the detected agent.
#[must_use]
pub fn attach_plan(
    choice: Option<SelectableExecutor>,
    detected: &[DetectedAgent],
    exo_enabled: bool,
) -> AttachPlan {
    let Some(choice) = choice else {
        return AttachPlan {
            exo: exo_enabled,
            agents: detected.to_vec(),
        };
    };
    match choice {
        // Attach nothing external. The router's native loop is required, so
        // this is not an absence — it is Omega, which is what was asked for.
        SelectableExecutor::Omega => AttachPlan {
            exo: false,
            agents: Vec::new(),
        },
        // Exo only. Not "Exo first": a person who picked Exo on a machine
        // where the lane has since stopped resolving must land on the native
        // loop with a visible fallback reason, not silently on Codex.
        SelectableExecutor::Exo => AttachPlan {
            exo: exo_enabled,
            agents: Vec::new(),
        },
        SelectableExecutor::Codex | SelectableExecutor::Claude | SelectableExecutor::Grok => {
            AttachPlan {
                exo: false,
                agents: detected
                    .iter()
                    .filter(|agent| Some(agent.id) == choice.adapter_id())
                    .cloned()
                    .collect(),
            }
        }
    }
}

/// The executor name the selector should show and whether that name is pending.
///
/// [`selected`] is a standing choice for the next connection, not proof that
/// the current thread is changing. It may outlive the thread where it was
/// chosen. Reading it unconditionally made a fresh Exo thread say `Omega…`
/// when Omega happened to be the previous choice, even though no switch had
/// been requested on that thread.
///
/// A choice replaces the attached executor on the face only during the
/// debounce started by the selector itself. At every other time the thread's
/// disclosure is the fact on screen.
#[must_use]
pub fn displayed_executor(
    attached: Option<SelectableExecutor>,
    choice: Option<SelectableExecutor>,
    switch_pending: bool,
) -> (Option<SelectableExecutor>, bool) {
    let current = if switch_pending {
        choice.or(attached)
    } else {
        attached
    };
    let connecting = switch_pending && current.is_some() && current != attached;
    (current, connecting)
}

/// Whether changing executors can discard anything the person can see.
///
/// A blank thread remains escapable even if an adapter transiently reports a
/// running turn during startup. Once the transcript has content, a running turn
/// is the only state in which switching would make its eventual answer arrive
/// in a conversation that no longer exists.
#[must_use]
pub const fn executor_switch_enabled(conversation_is_blank: bool, turn_is_running: bool) -> bool {
    conversation_is_blank || !turn_is_running
}

/// The composer's executor selector.
///
/// `current` is what this thread is **actually** running on, read from its
/// disclosure record — never from [`selected`]. That is the same rule
/// `OMEGA-DELTA-0094` holds for the audience control, and it matters more
/// here: the selection applies to the next connection, so a face that showed
/// it would repaint a running Codex thread as Claude the moment somebody
/// picked Claude, with nothing having moved.
///
/// `enabled` is false once the conversation has run something. The disabled
/// control is still rendered, and still names the executor, because that name
/// is the answer to "what is spending my budget" and a conversation in
/// progress is when a person most wants it.
pub fn render_executor_selector(
    current: Option<SelectableExecutor>,
    current_agent_id: SharedString,
    ready: Vec<SelectableExecutor>,
    enabled: bool,
    connecting: bool,
    on_select: Rc<dyn Fn(SelectableExecutor, &mut Window, &mut App)>,
) -> AnyElement {
    // An executor that is not one of the five is named by its own id rather
    // than rounded to the nearest of them. This is an engine lane, or an
    // adapter Omega did not attach; both are facts, and neither is a fifth
    // choice, so the control reports and does not offer.
    // `OMEGA-DELTA-0131`, omega#121. The trailing `\u{2026}` is the difference
    // between the name of a choice and the name of what is listening. The label
    // shows the choice, so the control moves the instant it is pressed; it
    // shows the choice *as pending* until the thread is actually on it, because
    // the owner selected Exo, was shown "Exo", asked "who are you", and Codex
    // answered. There is no state in which this control names an executor with
    // nothing to separate "is" from "will be".
    let label = SharedString::from(current.map_or_else(
        || current_agent_id.to_string(),
        |choice| {
            if connecting {
                format!("{}\u{2026}", choice.name())
            } else {
                choice.name().to_owned()
            }
        },
    ));
    let offering = enabled && current.is_some();

    let trigger = Button::new("omega-executor-selector", label)
        .label_size(LabelSize::XSmall)
        .color(Color::Muted)
        .disabled(!offering)
        .end_icon(
            Icon::new(IconName::ChevronDown)
                .size(IconSize::XSmall)
                .color(Color::Muted),
        );

    let tooltip = SharedString::from(match (offering, current) {
        (true, Some(choice)) if connecting => format!(
            "Connecting to {}. Until it answers, this thread is still on \
             whatever was attached before.",
            choice.name()
        ),
        (true, Some(choice)) => format!("{} {CHOOSING_RECONNECTS}", choice.description()),
        (false, Some(choice)) => {
            format!("{} {ONLY_BEFORE_THE_FIRST_MESSAGE}", choice.description())
        }
        // An engine lane. `render_zero_base_executor_bar`'s Full Auto surfaces
        // own that thread; this control does not reach it and says so.
        (_, None) => format!(
            "This conversation is running on {current_agent_id}, which is not \
             one of the executors this control switches between."
        ),
    });

    PopoverMenu::new("omega-executor")
        .trigger_with_tooltip(
            trigger,
            Tooltip::element(move |_window, _cx| {
                Label::new(tooltip.clone())
                    .size(LabelSize::Small)
                    .into_any_element()
            }),
        )
        .anchor(gpui::Anchor::BottomRight)
        .menu(move |window, cx| {
            if !offering {
                return None;
            }
            Some(build_menu(
                current,
                ready.clone(),
                on_select.clone(),
                window,
                cx,
            ))
        })
        .into_any_element()
}

fn build_menu(
    current: Option<SelectableExecutor>,
    ready: Vec<SelectableExecutor>,
    on_select: Rc<dyn Fn(SelectableExecutor, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Entity<ContextMenu> {
    ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
        menu = menu.header(SELECTION_MENU_HEADER);

        for choice in ready.clone() {
            let is_current = current == Some(choice);
            let description = SharedString::from(choice.description());
            let on_select = on_select.clone();
            menu.push_item(
                ContextMenuEntry::new(SharedString::from(choice.selector_name()))
                    .toggleable(IconPosition::End, is_current)
                    .documentation_aside(ui::DocumentationSide::Left, move |_| {
                        Label::new(description.clone()).into_any_element()
                    })
                    .handler(move |window, cx| {
                        // Picking what is already running would reconnect a
                        // conversation for no change, which reads as the
                        // control losing the thread's place.
                        if is_current {
                            return;
                        }
                        on_select(choice, window, cx);
                    }),
            );
        }

        // `OMEGA-DELTA-0123`. The names that are not ready, and why, under the
        // ones that are.
        //
        // Read from this machine rather than passed in, matching
        // `exo_lane_resolves` above: this is the one part of the menu whose
        // content is a fact about the install rather than about the thread, and
        // threading it through `render_executor_selector` would put it in every
        // call site for the benefit of none of them.
        //
        // **The reason is in the label, not in a documentation aside.** That is
        // not a style choice. `ContextMenu::select_index` registers an aside
        // only for an item that `is_selectable`, and `is_selectable` is
        // `!disabled` for an entry — so an aside on a disabled entry is never
        // shown, and the `Info` icon the component draws beside one has nothing
        // behind it. A reason that cannot be reached is not a reason.
        // `a_disabled_menu_entry_still_cannot_be_selected` fails if that ever
        // stops being true, because then the long form becomes available and
        // this decision is worth revisiting.
        let unavailable = selector_unavailable_here();
        if !unavailable.is_empty() {
            menu = menu.separator();
            for (choice, reason) in unavailable {
                menu.push_item(
                    ContextMenuEntry::new(SharedString::from(format!(
                        "{} — {reason}",
                        choice.selector_name()
                    )))
                    .disabled(true),
                );
            }
        }

        // omega#112. No explanatory footer in the menu.
        //
        // The owner: "remove the explanatory message in this box, not
        // relevant". It was two sentences of policy under a list of four
        // words, and a person opening a four-item menu is choosing, not
        // reading. Both sentences still exist where they are actually needed:
        // `CHOOSING_RECONNECTS` and `ONLY_BEFORE_THE_FIRST_MESSAGE` are the
        // disabled control's tooltip, which is where someone who cannot click
        // the thing is owed an explanation.

        menu.key_context("OmegaExecutorSelector")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn detected(id: &'static str, name: &'static str) -> DetectedAgent {
        DetectedAgent {
            id,
            name,
            binary: PathBuf::from("/usr/local/bin").join(name.to_lowercase()),
            launch: omega_agent_detect::launch_for(id)
                .expect("the fixture agent is one detection knows about"),
            prompt: omega_agent_detect::prompt_contract_for(id)
                .expect("the fixture agent is one detection knows about"),
        }
    }

    fn codex() -> DetectedAgent {
        detected(agent_servers::CODEX_ID, "Codex")
    }

    fn claude() -> DetectedAgent {
        detected(agent_servers::CLAUDE_AGENT_ID, "Claude")
    }

    fn grok() -> DetectedAgent {
        detected(agent_servers::GROK_ID, "Grok")
    }

    fn copilot() -> DetectedAgent {
        detected("github-copilot-cli", "Copilot")
    }

    /// The complete named executor catalog.
    #[test]
    fn there_are_five_names_and_no_others() {
        assert_eq!(
            SelectableExecutor::ALL
                .iter()
                .map(|choice| choice.name())
                .collect::<Vec<_>>(),
            vec!["Omega", "Exo", "Codex", "Claude", "Grok"],
        );
    }

    /// A machine with nothing installed still has Omega, and nothing else.
    #[test]
    fn a_bare_machine_offers_omega_alone() {
        assert_eq!(
            ready(&[], false),
            vec![SelectableExecutor::Omega],
            "the native loop is compiled in, so the list is never empty — and \
             nothing else may be offered on a machine that cannot run it"
        );
    }

    #[test]
    fn only_omega_is_ordinary_public_selection() {
        let ready = SelectableExecutor::ALL.to_vec();

        assert_eq!(
            selectable(&ready, false),
            vec![SelectableExecutor::Omega],
            "External ACP executors stay ready for Omega's router without becoming \
             direct choices"
        );
        assert_eq!(
            selectable(&ready, true),
            vec![SelectableExecutor::Omega, SelectableExecutor::Exo],
            "Exo joins the public selector only for an opted-in launch"
        );
        assert_eq!(SelectableExecutor::Claude.selector_name(), "Claude");
    }

    #[test]
    fn a_standing_choice_never_relabels_a_new_thread() {
        assert_eq!(
            displayed_executor(
                Some(SelectableExecutor::Exo),
                Some(SelectableExecutor::Omega),
                false,
            ),
            (Some(SelectableExecutor::Exo), false),
            "a choice from an earlier conversation is not what this thread is \
             running on"
        );
        assert_eq!(
            displayed_executor(
                Some(SelectableExecutor::Exo),
                Some(SelectableExecutor::Omega),
                true,
            ),
            (Some(SelectableExecutor::Omega), true),
            "the choice appears as pending only while this thread is actually \
             switching"
        );
    }

    #[test]
    fn a_blank_thread_is_always_escapable() {
        assert!(executor_switch_enabled(true, true));
        assert!(executor_switch_enabled(false, false));
        assert!(!executor_switch_enabled(false, true));
    }

    /// A name appears only when it can run.
    #[test]
    fn a_detected_agent_is_offered_and_an_absent_one_is_not() {
        assert_eq!(
            ready(&[claude()], false),
            vec![SelectableExecutor::Omega, SelectableExecutor::Claude],
            "Claude is installed and Codex is not"
        );
        assert_eq!(
            ready(&[codex(), claude(), grok()], true),
            vec![
                SelectableExecutor::Omega,
                SelectableExecutor::Exo,
                SelectableExecutor::Codex,
                SelectableExecutor::Claude,
                SelectableExecutor::Grok,
            ],
            "the order is ALL's order, not detection's, so the menu does not \
             reorder itself between launches"
        );
    }

    /// Detected is not enough. Omega hosts no ACP adapter for Copilot, so
    /// offering it would produce a connect-time failure rather than a thread.
    #[test]
    fn an_agent_omega_cannot_host_is_never_a_name() {
        assert_eq!(ready(&[copilot()], false), vec![SelectableExecutor::Omega]);
    }

    /// Exo is a lane, not a binary on `PATH`.
    #[test]
    fn exo_is_offered_only_when_a_lane_resolves() {
        assert!(!ready(&[], false).contains(&SelectableExecutor::Exo));
        assert!(ready(&[], true).contains(&SelectableExecutor::Exo));
    }

    /// No choice made is not the same as choosing Omega.
    #[test]
    fn no_choice_leaves_the_routers_own_rule_alone() {
        let installed = vec![codex(), claude()];

        assert_eq!(
            attach_plan(None, &installed, true),
            AttachPlan {
                exo: true,
                agents: installed.clone(),
            },
            "a machine whose owner never opened the menu must attach exactly \
             what it attached before this control existed"
        );
    }

    #[test]
    fn a_default_launch_never_attaches_exo() {
        let installed = vec![codex(), claude()];

        assert_eq!(
            attach_plan(None, &installed, false),
            AttachPlan {
                exo: false,
                agents: installed,
            }
        );
        assert!(
            !attach_plan(Some(SelectableExecutor::Exo), &[], false).exo,
            "a stale or synthetic Exo choice cannot bypass the launch flag"
        );
    }

    /// Choosing Omega attaches nothing external, which is what Omega is.
    #[test]
    fn choosing_omega_attaches_nothing_external() {
        let plan = attach_plan(Some(SelectableExecutor::Omega), &[codex(), claude()], true);

        assert!(!plan.exo);
        assert!(
            plan.agents.is_empty(),
            "the router's native loop is required, so attaching nothing \
             external is the native loop and not an absence"
        );
    }

    /// Choosing one agent excludes the other, and excludes the Exo lane that
    /// would otherwise have taken the same slot.
    #[test]
    fn choosing_an_agent_excludes_every_other_external_executor() {
        let installed = vec![codex(), claude(), grok()];

        let plan = attach_plan(Some(SelectableExecutor::Claude), &installed, true);
        assert!(
            !plan.exo,
            "the Exo lane fills the same single external slot and wins by \
             default, so a person who asked for Claude would get Exo"
        );
        assert_eq!(
            plan.agents.iter().map(|agent| agent.id).collect::<Vec<_>>(),
            vec![agent_servers::CLAUDE_AGENT_ID],
            "Codex is first in candidate order and would otherwise be chosen"
        );

        let plan = attach_plan(Some(SelectableExecutor::Codex), &installed, true);
        assert_eq!(
            plan.agents.iter().map(|agent| agent.id).collect::<Vec<_>>(),
            vec![agent_servers::CODEX_ID],
        );

        let plan = attach_plan(Some(SelectableExecutor::Grok), &installed, true);
        assert_eq!(
            plan.agents.iter().map(|agent| agent.id).collect::<Vec<_>>(),
            vec![agent_servers::GROK_ID],
        );
    }

    /// Exo only. A lane that has stopped resolving must fall to the native
    /// loop with a visible reason, never silently to Codex.
    #[test]
    fn choosing_exo_does_not_fall_through_to_a_detected_agent() {
        let plan = attach_plan(Some(SelectableExecutor::Exo), &[codex(), claude()], true);

        assert!(plan.exo);
        assert!(plan.agents.is_empty());
    }

    /// A choice that names an agent this machine does not have attaches
    /// nothing rather than substituting one.
    #[test]
    fn a_choice_for_an_absent_agent_substitutes_nothing() {
        let plan = attach_plan(Some(SelectableExecutor::Codex), &[claude()], true);

        assert!(
            plan.agents.is_empty(),
            "silently running Claude for somebody who asked for Codex is the \
             substitution the whole disclosure surface exists to prevent"
        );
    }

    /// The face of the control is read from the thread's disclosure.
    #[test]
    fn a_live_thread_is_recognised_from_its_disclosure() {
        assert_eq!(
            SelectableExecutor::of(ExecutorClass::NativeLoop, "omega"),
            Some(SelectableExecutor::Omega)
        );
        assert_eq!(
            SelectableExecutor::of(ExecutorClass::ExternalAcp, agent_servers::CODEX_ID),
            Some(SelectableExecutor::Codex)
        );
        assert_eq!(
            SelectableExecutor::of(ExecutorClass::ExternalAcp, agent_servers::CLAUDE_AGENT_ID),
            Some(SelectableExecutor::Claude)
        );
        assert_eq!(
            SelectableExecutor::of(ExecutorClass::ExternalAcp, agent_servers::GROK_ID),
            Some(SelectableExecutor::Grok)
        );
        assert_eq!(
            SelectableExecutor::of(ExecutorClass::ExternalAcp, omega_exo_lane::EXO_HARNESS_ID),
            Some(SelectableExecutor::Exo)
        );
    }

    /// An engine lane is not one of the five, and is not rounded to one.
    #[test]
    fn an_engine_lane_is_not_answered_with_one_of_the_five() {
        assert_eq!(
            SelectableExecutor::of(ExecutorClass::EngineLane, agent_servers::CODEX_ID),
            None,
            "an engine lane is Full Auto authority; this control does not \
             reach it, and naming it Codex would attribute a run to the \
             adapter underneath it"
        );
        assert_eq!(
            SelectableExecutor::of(ExecutorClass::ExternalAcp, "somebody-elses-acp"),
            None,
            "an adapter Omega did not attach is reported by its own id, not \
             rounded to the nearest first-party name"
        );
    }

    /// Every name that is offered is one the attach can actually reach.
    ///
    /// The falsifier for this whole module: a list and a plan that disagree
    /// would be a menu whose entries fail when they are clicked.
    #[test]
    fn every_offered_name_produces_a_plan_that_reaches_it() {
        let installed = vec![codex(), claude(), grok()];

        for choice in ready(&installed, true) {
            let plan = attach_plan(Some(choice), &installed, true);
            match choice {
                SelectableExecutor::Omega => {
                    assert!(!plan.exo && plan.agents.is_empty());
                }
                SelectableExecutor::Exo => assert!(plan.exo),
                SelectableExecutor::Codex
                | SelectableExecutor::Claude
                | SelectableExecutor::Grok => assert_eq!(
                    plan.agents.iter().map(|agent| agent.id).collect::<Vec<_>>(),
                    vec![
                        choice
                            .adapter_id()
                            .expect("a detected agent has an adapter")
                    ],
                ),
            }
        }
    }

    /// `OMEGA-DELTA-0123`. Nothing is silently missing.
    ///
    /// The two lists partition the five names: nothing in both, and — the half
    /// that matters — nothing in neither. A name dropped from `ready` with no
    /// entry here is exactly the absence the owner had to ask about.
    #[test]
    fn every_name_is_either_ready_or_explained() {
        let machines: [(Vec<DetectedAgent>, Option<&'static str>); 4] = [
            (Vec::new(), Some("Exo has never been run here")),
            (vec![codex()], Some("Exo is not installed")),
            (vec![copilot()], Some("your Exo has no agent")),
            (vec![codex(), claude()], None),
        ];

        for (detected, absence) in machines {
            let ready = ready(&detected, absence.is_none());
            let unavailable = unavailable(&detected, absence);

            let mut named: Vec<SelectableExecutor> = ready
                .iter()
                .copied()
                .chain(unavailable.iter().map(|(choice, _)| *choice))
                .collect();
            named.sort_by_key(|choice| choice.token());

            let mut all = SelectableExecutor::ALL.to_vec();
            all.sort_by_key(|choice| choice.token());

            assert_eq!(
                named, all,
                "every name is either offered or explained, exactly once — \
                 {detected:?} with {absence:?} leaves one unaccounted for"
            );
        }
    }

    /// `OMEGA-DELTA-0123`. The native loop is never explained away.
    ///
    /// It is compiled in, so an entry saying why it is missing would be a menu
    /// disabling the one executor that cannot be absent.
    #[test]
    fn omega_is_never_among_the_unavailable() {
        for absence in [None, Some("Exo is not installed")] {
            assert!(
                !unavailable(&[], absence)
                    .iter()
                    .any(|(choice, _)| *choice == SelectableExecutor::Omega)
            );
        }
    }

    /// `OMEGA-DELTA-0123`. An agent that is not installed says so, and the
    /// other arm is unreachable while the two lists agree.
    ///
    /// The honest version of this check, and the second one written. The first
    /// claimed to hold "not installed" and "installed and undrivable" apart,
    /// and could not: for the drivable external agents the second case cannot happen at
    /// all. `ready` offers a name when it is **detected and drivable**, and
    /// `DRIVABLE_AGENT_IDS` names all of them — so a detected Codex is always
    /// offered and never explained, and the "installed; Omega hosts no adapter
    /// for it" arm is dead code. Collapsing that arm into the other passed the
    /// test, which is how the claim was found to be empty.
    ///
    /// So the arm is kept and the *reachability* is what is asserted. It is the
    /// truthful answer on the day somebody removes an id from
    /// `DRIVABLE_AGENT_IDS` while it is still one of the five names, and this
    /// check fails on exactly that edit — which is when the sentence starts
    /// being read by somebody.
    #[test]
    fn an_agent_that_is_not_installed_says_so() {
        let reason = |detected: &[DetectedAgent], choice: SelectableExecutor| {
            unavailable(detected, None)
                .into_iter()
                .find(|(name, _)| *name == choice)
                .map(|(_, reason)| reason)
        };

        assert_eq!(
            reason(&[], SelectableExecutor::Codex),
            Some("not installed")
        );
        assert_eq!(
            reason(&[claude()], SelectableExecutor::Codex),
            Some("not installed"),
            "one of the two being present says nothing about the other"
        );
        assert_eq!(
            reason(&[codex(), claude()], SelectableExecutor::Codex),
            None,
            "a detected, drivable agent is offered rather than explained"
        );

        for choice in SelectableExecutor::ALL {
            let Some(adapter) = choice.adapter_id() else {
                continue;
            };
            assert!(
                crate::omega_agent_attach::DRIVABLE_AGENT_IDS.contains(&adapter),
                "{} is one of the five names and Omega no longer hosts an \
                 adapter for it. The \"installed; Omega hosts no adapter for \
                 it\" reason becomes reachable at that moment — check it says \
                 something useful before shipping this.",
                choice.name()
            );
        }
    }

    /// `OMEGA-DELTA-0123`. Exo's reason is the derivation's, not one invented
    /// here.
    #[test]
    fn exos_reason_is_carried_through_rather_than_reworded() {
        let absence = omega_agent_detect::exo::ExoLaneUnderivable::NoStateRoot {
            searched: vec![PathBuf::from("/nowhere/.exo")],
        }
        .summary();

        assert_eq!(
            unavailable(&[], Some(absence))
                .into_iter()
                .find(|(choice, _)| *choice == SelectableExecutor::Exo)
                .map(|(_, reason)| reason),
            Some("Exo has never been run here"),
            "the menu says what the derivation refused, so a new refusal \
             variant reaches a person without an edit here"
        );
    }

    /// Nothing is chosen until a person chooses.
    #[test]
    fn nothing_is_selected_by_default() {
        assert!(
            selected().is_none(),
            "a fresh process attaches by the router's own rule; a choice is \
             something a person makes"
        );
    }
}
