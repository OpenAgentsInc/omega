//! The composer's executor selector. `OMEGA-DELTA-0115`.
//!
//! The owner asked for this while looking at the running app, in as many
//! words: *"I need to be able to switch the executor. No selection of model
//! like Gemini. You select between Omega, Exo, Codex, Claude. If some of those
//! are not implemented yet, don't add them, but that's what I want. Whichever
//! of those are ready, put those in now. Those are the only four choices."*
//!
//! So the control this module renders names a **runtime**, never a model. What
//! it replaced in the composer bar named `google/gemini-3.6-flash`, which is
//! the answer to a question the owner was not asking.
//!
//! # Four names, and a name appears only when it can run
//!
//! [`SelectableExecutor`] is a closed enum of exactly four variants for the
//! same reason [`omega_front_door::ExecutorClass`] is closed at three: the set
//! is a product decision, and a string would let a later edit add a fifth
//! without anybody noticing. [`ready`] filters that set against what is
//! actually on this machine:
//!
//! - **Omega** is the native loop, which is compiled in. Always ready.
//! - **Codex** and **Claude** are ready when `omega_agent_detect` finds their
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
//! A shorter list is the point. A selector offering a name that fails when it
//! is clicked is worse than one that never offered it, because the person then
//! has to work out whether they broke something.
//!
//! # Why choosing re-attaches instead of re-pinning
//!
//! `OmegaAgentConnection` holds **one** external-ACP slot, filled once, when
//! the connection is built: the Exo lane if there is one, otherwise the
//! detected agent. `pin_session` chooses between the *classes* the router
//! already holds — it cannot make Claude reachable on a machine where Codex
//! filled that slot. So a pin alone could never honour three of the four
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

/// The four executors a person may choose between.
///
/// Closed, and closed on purpose. "Those are the only four choices" is a
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
}

impl SelectableExecutor {
    /// Every name, in the order the menu offers them.
    ///
    /// Omega first because it is the one that is always there, and the order
    /// is fixed here rather than derived from what is installed so the menu
    /// does not reorder itself between launches.
    pub const ALL: &'static [Self] = &[Self::Omega, Self::Exo, Self::Codex, Self::Claude];

    /// The name a person reads. This is the only rendering of this type.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Omega => "Omega",
            Self::Exo => "Exo",
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
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
        }
    }

    /// The ACP adapter id this name attaches through, for the two that are
    /// detected agents.
    ///
    /// Taken from `agent_servers` rather than spelled again, so an id renamed
    /// there cannot leave this pointing at an adapter that no longer exists.
    #[must_use]
    pub const fn adapter_id(self) -> Option<&'static str> {
        match self {
            Self::Codex => Some(agent_servers::CODEX_ID),
            Self::Claude => Some(agent_servers::CLAUDE_AGENT_ID),
            Self::Omega | Self::Exo => None,
        }
    }

    /// Which of the four a live thread is running on, if it is one of them.
    ///
    /// `None` for an engine lane, and for an external agent that is not one of
    /// the two named here. Neither is a fifth choice: the first is Full Auto
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
            SelectableExecutor::Codex | SelectableExecutor::Claude => {
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
    ready(omega_agent_detect::detected(), exo_lane_resolves())
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
/// Cached for the life of the process for the reason
/// `omega_agent_detect::detected` is: the composer asks this on every draw and
/// answering it walks the filesystem.
#[must_use]
pub fn exo_lane_resolves() -> bool {
    static RESOLVES: OnceLock<bool> = OnceLock::new();
    *RESOLVES.get_or_init(|| ExoLaneConfig::resolve(&ExoLaneConfig::data_dir_path()).is_some())
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
/// means the router attaches by its own rule — the Exo lane, then the detected
/// agent — and that rule is what a machine that has never touched this control
/// gets.
#[must_use]
pub fn selected() -> Option<SelectableExecutor> {
    *SELECTED
        .lock()
        .expect("the executor selection is never held across a panic")
}

/// Choose the executor the next connection attaches.
///
/// **Only a person may call this.** It is the whole of the switching mechanism,
/// and called from a turn, a tool, or a retry it would be a thread quietly
/// moving executors — which is the defect class `omega#77`'s disclosure exists
/// to make impossible.
pub fn select(choice: SelectableExecutor) {
    log::info!(
        "OMEGA-DELTA-0115: a person chose {} ({}) as this session's executor",
        choice.name(),
        choice.token()
    );
    *SELECTED
        .lock()
        .expect("the executor selection is never held across a panic") = Some(choice);
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
/// `None` reproduces the behaviour that existed before this control did: the
/// Exo lane if there is one, otherwise the detected agent. A machine whose
/// owner never opens the menu is therefore unchanged, which is what keeps this
/// from being a migration.
#[must_use]
pub fn attach_plan(choice: Option<SelectableExecutor>, detected: &[DetectedAgent]) -> AttachPlan {
    let Some(choice) = choice else {
        return AttachPlan {
            exo: true,
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
            exo: true,
            agents: Vec::new(),
        },
        SelectableExecutor::Codex | SelectableExecutor::Claude => AttachPlan {
            exo: false,
            agents: detected
                .iter()
                .filter(|agent| Some(agent.id) == choice.adapter_id())
                .cloned()
                .collect(),
        },
    }
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
    on_select: Rc<dyn Fn(SelectableExecutor, &mut Window, &mut App)>,
) -> AnyElement {
    // An executor that is not one of the four is named by its own id rather
    // than rounded to the nearest of them. This is an engine lane, or an
    // adapter Omega did not attach; both are facts, and neither is a fifth
    // choice, so the control reports and does not offer.
    let label = SharedString::from(current.map_or_else(
        || current_agent_id.to_string(),
        |choice| choice.name().to_owned(),
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
                ContextMenuEntry::new(SharedString::from(choice.name()))
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
        }
    }

    fn codex() -> DetectedAgent {
        detected(agent_servers::CODEX_ID, "Codex")
    }

    fn claude() -> DetectedAgent {
        detected(agent_servers::CLAUDE_AGENT_ID, "Claude")
    }

    fn copilot() -> DetectedAgent {
        detected("github-copilot-cli", "Copilot")
    }

    /// The owner's sentence, as a test: exactly these four names exist.
    #[test]
    fn there_are_four_names_and_no_others() {
        assert_eq!(
            SelectableExecutor::ALL
                .iter()
                .map(|choice| choice.name())
                .collect::<Vec<_>>(),
            vec!["Omega", "Exo", "Codex", "Claude"],
            "\"those are the only four choices\" is a product decision, and a \
             fifth entry here is the edit that quietly reverses it"
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

    /// A name appears only when it can run.
    #[test]
    fn a_detected_agent_is_offered_and_an_absent_one_is_not() {
        assert_eq!(
            ready(&[claude()], false),
            vec![SelectableExecutor::Omega, SelectableExecutor::Claude],
            "Claude is installed and Codex is not"
        );
        assert_eq!(
            ready(&[codex(), claude()], true),
            vec![
                SelectableExecutor::Omega,
                SelectableExecutor::Exo,
                SelectableExecutor::Codex,
                SelectableExecutor::Claude,
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
            attach_plan(None, &installed),
            AttachPlan {
                exo: true,
                agents: installed.clone(),
            },
            "a machine whose owner never opened the menu must attach exactly \
             what it attached before this control existed"
        );
    }

    /// Choosing Omega attaches nothing external, which is what Omega is.
    #[test]
    fn choosing_omega_attaches_nothing_external() {
        let plan = attach_plan(Some(SelectableExecutor::Omega), &[codex(), claude()]);

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
        let installed = vec![codex(), claude()];

        let plan = attach_plan(Some(SelectableExecutor::Claude), &installed);
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

        let plan = attach_plan(Some(SelectableExecutor::Codex), &installed);
        assert_eq!(
            plan.agents.iter().map(|agent| agent.id).collect::<Vec<_>>(),
            vec![agent_servers::CODEX_ID],
        );
    }

    /// Exo only. A lane that has stopped resolving must fall to the native
    /// loop with a visible reason, never silently to Codex.
    #[test]
    fn choosing_exo_does_not_fall_through_to_a_detected_agent() {
        let plan = attach_plan(Some(SelectableExecutor::Exo), &[codex(), claude()]);

        assert!(plan.exo);
        assert!(plan.agents.is_empty());
    }

    /// A choice that names an agent this machine does not have attaches
    /// nothing rather than substituting one.
    #[test]
    fn a_choice_for_an_absent_agent_substitutes_nothing() {
        let plan = attach_plan(Some(SelectableExecutor::Codex), &[claude()]);

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
            SelectableExecutor::of(ExecutorClass::ExternalAcp, omega_exo_lane::EXO_HARNESS_ID),
            Some(SelectableExecutor::Exo)
        );
    }

    /// An engine lane is not one of the four, and is not rounded to one.
    #[test]
    fn an_engine_lane_is_not_answered_with_one_of_the_four() {
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
        let installed = vec![codex(), claude()];

        for choice in ready(&installed, true) {
            let plan = attach_plan(Some(choice), &installed);
            match choice {
                SelectableExecutor::Omega => {
                    assert!(!plan.exo && plan.agents.is_empty());
                }
                SelectableExecutor::Exo => assert!(plan.exo),
                SelectableExecutor::Codex | SelectableExecutor::Claude => assert_eq!(
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
