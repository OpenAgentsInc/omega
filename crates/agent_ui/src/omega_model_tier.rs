//! Omega model tier selector. Luna (default), Flash, and Pro.
//!
//! Product surface for the hosted OpenAgents inference lanes:
//! - **Luna** — `openagents/gpt-5.6-luna` (default; hosted OpenAI passthrough
//!   through OUR API — owner direction 2026-07-30: the core Omega Agent always
//!   hits our API first, Gemini is the backup)
//! - **Flash** — `openagents/gemini-3.6-flash` (backup; hosted Gemini path)
//! - **Pro** — `openagents/kimi-k3` (Fireworks via OpenAgents chat completions)
//!
//! The zero-base composer bar owns this control. It is a closed menu rather
//! than the full model picker, matching the owner's "dropdowns inside the
//! input bar" direction without reopening the full model catalog.

use std::rc::Rc;
use std::sync::Mutex;

use gpui::{AnyElement, App, Window};
use ui::{Button, ContextMenu, ContextMenuEntry, PopoverMenu, Tooltip, prelude::*};

/// Process-wide standing tier choice. Defaults to Luna.
static SELECTED: Mutex<ModelTier> = Mutex::new(ModelTier::Luna);

/// The product tiers Omega offers on the native loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelTier {
    /// GPT-5.6 Luna — default, hosted through the OpenAgents API.
    Luna,
    /// Gemini 3.6 Flash — backup, hosted through OpenAgents.
    Flash,
    /// Kimi K3 — stronger coding lane through OpenAgents Fireworks.
    Pro,
}

impl ModelTier {
    pub const ALL: &'static [Self] = &[Self::Luna, Self::Flash, Self::Pro];

    /// Label shown on the trigger and menu.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Luna => "Luna",
            Self::Flash => "Flash",
            Self::Pro => "Pro",
        }
    }

    /// One sentence for the menu documentation aside.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Luna => "GPT-5.6 Luna — default, hosted through the OpenAgents API.",
            Self::Flash => "Gemini 3.6 Flash — fast hosted backup lane.",
            Self::Pro => "Kimi K3 — stronger coding lane via OpenAgents Fireworks.",
        }
    }

    /// Agent model id the native agent selects for this tier.
    ///
    /// Format matches `LanguageModels::model_id`: `provider/model`.
    #[must_use]
    pub const fn agent_model_id(self) -> &'static str {
        match self {
            Self::Luna => "openagents/gpt-5.6-luna",
            Self::Flash => "openagents/gemini-3.6-flash",
            Self::Pro => "openagents/kimi-k3",
        }
    }

    /// Provider id for settings persistence.
    ///
    /// `OMEGA-DELTA-0202`. Flash was `google`, the direct provider, which needs
    /// the person's own Google AI credential. The middle rung of the always-work
    /// chain therefore reported "Gemini 3.6 Flash is unavailable" on a machine
    /// whose hosted Gemini lane was healthy and serving Luna and Pro on the same
    /// session. All three tiers are hosted lanes now, which is what this
    /// module's own documentation had claimed all along.
    #[must_use]
    pub const fn provider_id(self) -> &'static str {
        match self {
            Self::Luna | Self::Flash | Self::Pro => "openagents",
        }
    }

    /// Model id for settings persistence.
    #[must_use]
    pub const fn model_id(self) -> &'static str {
        match self {
            Self::Luna => "gpt-5.6-luna",
            Self::Flash => "gemini-3.6-flash",
            Self::Pro => "kimi-k3",
        }
    }

    /// The model's own name, as a person would say it.
    ///
    /// The tier name is Omega's product word for a lane; this is the thing that
    /// answers. `OMEGA-DELTA-0202` needs both, because a surface that shows only
    /// the tier word cannot be checked against the model that served the turn.
    #[must_use]
    pub const fn model_name(self) -> &'static str {
        match self {
            Self::Luna => "GPT-5.6 Luna",
            Self::Flash => "Gemini 3.6 Flash",
            Self::Pro => "Kimi K3",
        }
    }

    /// The tier a concrete provider/model pair is, when it is one of the three.
    ///
    /// `None` for anything else — a fallback rung on a direct-key provider, or
    /// a model chosen outside this control. A caller must not invent a tier
    /// name for a model that is not one: naming a lane the turn is not on is
    /// exactly the disagreement `OMEGA-DELTA-0131` forbids.
    #[must_use]
    pub fn for_model(provider_id: &str, model_id: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|tier| tier.provider_id() == provider_id && tier.model_id() == model_id)
    }
}

/// Current standing tier. Luna when nobody has chosen.
#[must_use]
pub fn selected() -> ModelTier {
    *SELECTED
        .lock()
        .expect("the model tier selection is never held across a panic")
}

/// Record a person's tier choice for this process.
pub fn select(tier: ModelTier) {
    log::info!(
        "omega_model_tier: a person chose {} ({})",
        tier.name(),
        tier.agent_model_id()
    );
    *SELECTED
        .lock()
        .expect("the model tier selection is never held across a panic") = tier;
}

/// Choose a tier for a conversation whose session does not exist yet.
///
/// `OMEGA-DELTA-0204`. The connected composer's control reaches the running
/// session through its model selector, which also writes `agent.default_model`.
/// The pre-session composer has neither, and a control that moved a label
/// without moving the model would be the exact disagreement `OMEGA-DELTA-0202`
/// forbids. So this writes the one thing a session that does not exist yet
/// actually reads: the settings default the registry hands every new thread.
///
/// The thinking, effort, and speed fields come from the same
/// [`agent_settings::language_model_to_selection`] the connected path uses,
/// when the provider is authenticated and the model is therefore knowable. When
/// it is not, the pair is still written — a person who picks Pro before their
/// provider finishes loading gets Pro — with the conservative defaults the
/// model's own capabilities would otherwise supply.
pub fn select_before_session(tier: ModelTier, fs: std::sync::Arc<dyn fs::Fs>, cx: &mut App) {
    select(tier);
    select_model_before_session(tier.agent_model_id(), fs, cx);
}

pub fn select_model_before_session(
    agent_model_id: &str,
    fs: std::sync::Arc<dyn fs::Fs>,
    cx: &mut App,
) {
    use agent_settings::{AgentSettings, language_model_to_selection};
    use language_model::{LanguageModelId, LanguageModelProviderId, LanguageModelRegistry};
    use settings::{LanguageModelSelection, Settings as _, update_settings_file};

    let Some((provider_name, model_name)) = agent_model_id.split_once('/') else {
        log::error!("omega_model_tier: invalid agent model id {agent_model_id}");
        return;
    };

    let provider_id = LanguageModelProviderId::from(provider_name.to_owned());
    let model_id = LanguageModelId::from(model_name.to_owned());
    // `try_global`, not `read_global`: a tier choice is not worth panicking
    // over, and a process without a registry — a test harness, or a window
    // drawn before the providers install — must still write the pair.
    let model = LanguageModelRegistry::try_global(cx)
        .and_then(|registry| registry.read(cx).provider(&provider_id))
        .and_then(|provider| {
            provider
                .provided_models(cx)
                .iter()
                .find(|model| model.id() == model_id)
                .cloned()
        });

    let favorite = AgentSettings::get_global(cx)
        .favorite_models
        .iter()
        .find(|favorite| favorite.provider.0 == provider_name && favorite.model == model_name)
        .cloned();

    let selection = match model {
        Some(model) => language_model_to_selection(&model, favorite.as_ref()),
        None => LanguageModelSelection {
            provider: provider_name.to_owned().into(),
            model: model_name.to_owned(),
            ..favorite.unwrap_or_else(|| LanguageModelSelection {
                provider: provider_name.to_owned().into(),
                model: model_name.to_owned(),
                enable_thinking: false,
                effort: None,
                speed: None,
            })
        },
    };

    update_settings_file(fs, cx, move |settings, _cx| {
        settings.agent.get_or_insert_default().set_model(selection);
    });
}

/// Test-only reset to Luna.
#[cfg(any(test, feature = "test-support"))]
pub fn clear_selection_for_test() {
    *SELECTED
        .lock()
        .expect("the model tier selection is never held across a panic") = ModelTier::Luna;
}

/// Test-only setter without log side effects beyond select.
#[cfg(any(test, feature = "test-support"))]
pub fn select_for_test(tier: ModelTier) {
    select(tier);
}

const MENU_HEADER: &str = "Omega model";

/// Composer-bar Luna / Flash / Pro dropdown.
///
/// `OMEGA-DELTA-0202`. `face` is the routed decision — what is *actually*
/// serving this thread — and never the process-wide standing choice. The face
/// used to read [`selected`], a static that resets to Luna at every launch, so
/// reopening a thread whose model was Kimi K3 showed **Luna** on the control
/// beside a disclosure line reading `openagents/kimi-k3`, with Kimi answering.
pub fn render_model_tier_selector(
    face: RoutedFace,
    enabled: bool,
    on_select: Rc<dyn Fn(ModelTier, &mut Window, &mut App)>,
) -> AnyElement {
    let trigger = Button::new("omega-model-tier-selector", face.label.clone())
        .label_size(LabelSize::XSmall)
        .color(Color::Muted)
        .disabled(!enabled)
        .end_icon(
            Icon::new(IconName::ChevronDown)
                .size(IconSize::XSmall)
                .color(Color::Muted),
        );

    let current = face.tier;
    let tooltip = SharedString::from(if enabled {
        format!(
            "{} Switch between Luna (GPT-5.6), Flash (Gemini 3.6) and Pro (Kimi K3).",
            face.serving_sentence()
        )
    } else {
        format!(
            "{} Model is fixed while a turn is running.",
            face.serving_sentence()
        )
    });

    PopoverMenu::new("omega-model-tier")
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
            if !enabled {
                return None;
            }
            Some(build_menu(current, on_select.clone(), window, cx))
        })
        .into_any_element()
}

/// What the tier control shows, derived from the routed decision.
///
/// `tier` is `None` when the thread is on a model that is not one of the three
/// — a fallback rung on a direct-key provider, for instance. The control then
/// names that model rather than checking a tier it is not on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedFace {
    pub label: SharedString,
    pub model_name: SharedString,
    pub tier: Option<ModelTier>,
    pub agent_model_id: SharedString,
}

impl RoutedFace {
    /// The face for a concrete provider/model pair that is serving a thread.
    #[must_use]
    pub fn for_model(provider_id: &str, model_id: &str) -> Self {
        match ModelTier::for_model(provider_id, model_id) {
            Some(tier) => Self {
                label: SharedString::from(tier.name()),
                model_name: SharedString::from(tier.model_name()),
                tier: Some(tier),
                agent_model_id: SharedString::from(tier.agent_model_id()),
            },
            None if provider_id == "deepseek" && model_id == "deepseek-v4-flash" => Self {
                label: "DeepSeek V4 Flash".into(),
                model_name: "DeepSeek V4 Flash".into(),
                tier: None,
                agent_model_id: "deepseek/deepseek-v4-flash".into(),
            },
            None => Self {
                label: SharedString::from(model_id.to_owned()),
                model_name: SharedString::from(model_id.to_owned()),
                tier: None,
                agent_model_id: SharedString::from(format!("{provider_id}/{model_id}")),
            },
        }
    }

    /// The face for a thread that has not resolved a model yet.
    ///
    /// Only here does the standing choice get to speak, and only because it is
    /// then a truthful statement about what the next turn will start on.
    #[must_use]
    pub fn pending(standing: ModelTier) -> Self {
        Self {
            label: SharedString::from(standing.name()),
            model_name: SharedString::from(standing.model_name()),
            tier: Some(standing),
            agent_model_id: SharedString::from(standing.agent_model_id()),
        }
    }

    fn serving_sentence(&self) -> String {
        format!("{} is serving this thread.", self.model_name)
    }
}

fn build_menu(
    current: Option<ModelTier>,
    on_select: Rc<dyn Fn(ModelTier, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Entity<ContextMenu> {
    ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
        menu = menu.header(MENU_HEADER);
        for tier in ModelTier::ALL {
            let is_current = current == Some(*tier);
            let description = SharedString::from(tier.description());
            let on_select = on_select.clone();
            menu.push_item(
                ContextMenuEntry::new(SharedString::from(tier.name()))
                    .toggleable(IconPosition::End, is_current)
                    .documentation_aside(ui::DocumentationSide::Left, move |_| {
                        Label::new(description.clone()).into_any_element()
                    })
                    .handler(move |window, cx| {
                        if is_current {
                            return;
                        }
                        on_select(*tier, window, cx);
                    }),
            );
        }
        menu.key_context("OmegaModelTierSelector")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luna_is_the_default() {
        clear_selection_for_test();
        assert_eq!(selected(), ModelTier::Luna);
        assert_eq!(ModelTier::Luna.agent_model_id(), "openagents/gpt-5.6-luna");
    }

    /// `OMEGA-DELTA-0202`. Flash is a hosted lane, not the direct Google
    /// provider. Pointing it at `google` made the middle rung of the always-work
    /// chain unreachable for anyone without a personal Google AI credential,
    /// while the hosted Gemini lane on the same session was serving other tiers.
    #[test]
    fn flash_stays_reachable_as_the_hosted_gemini_backup() {
        assert_eq!(
            ModelTier::Flash.agent_model_id(),
            "openagents/gemini-3.6-flash"
        );
        assert_eq!(ModelTier::Flash.provider_id(), "openagents");
        assert_eq!(ModelTier::Flash.model_id(), "gemini-3.6-flash");
    }

    /// `OMEGA-DELTA-0202`. The face of the tier control is the routed decision.
    ///
    /// The live defect: a reopened thread whose model was Kimi K3 showed
    /// **Luna** on this control, because the face read the process-wide standing
    /// choice, which resets to Luna at every launch. The disclosure line beside
    /// it said `openagents/kimi-k3`, and Kimi is what answered.
    #[test]
    fn the_face_names_the_routed_model_not_the_standing_choice() {
        clear_selection_for_test();
        assert_eq!(selected(), ModelTier::Luna);

        let face = RoutedFace::for_model("openagents", "kimi-k3");
        assert_eq!(face.tier, Some(ModelTier::Pro));
        assert_eq!(face.label.as_ref(), "Pro");
        assert_eq!(face.model_name.as_ref(), "Kimi K3");
        assert_ne!(
            face.label.as_ref(),
            "Luna",
            "the standing choice must never reach the face of a thread that is \
             on another model"
        );
    }

    /// A model that is not one of the three is named, never dressed as a tier.
    #[test]
    fn an_off_tier_model_is_named_rather_than_labelled_with_a_tier() {
        let face = RoutedFace::for_model("anthropic", "claude-sonnet-4");
        assert_eq!(face.tier, None);
        assert_eq!(face.label.as_ref(), "claude-sonnet-4");
        assert_eq!(face.agent_model_id.as_ref(), "anthropic/claude-sonnet-4");
        assert!(
            !ModelTier::ALL
                .iter()
                .any(|tier| tier.name() == face.label.as_ref()),
            "an off-chain fallback rung must not borrow a tier name"
        );
    }

    #[test]
    fn deepseek_v4_flash_uses_its_display_name() {
        let face = RoutedFace::for_model("deepseek", "deepseek-v4-flash");
        assert_eq!(face.label.as_ref(), "DeepSeek V4 Flash");
        assert_eq!(face.model_name.as_ref(), "DeepSeek V4 Flash");
        assert_eq!(face.agent_model_id.as_ref(), "deepseek/deepseek-v4-flash");
    }

    #[test]
    fn every_tier_round_trips_through_its_wire_pair() {
        for tier in ModelTier::ALL {
            assert_eq!(
                ModelTier::for_model(tier.provider_id(), tier.model_id()),
                Some(*tier)
            );
            assert_eq!(
                format!("{}/{}", tier.provider_id(), tier.model_id()),
                tier.agent_model_id()
            );
        }
    }

    #[test]
    fn pro_selects_kimi_k3_on_openagents() {
        assert_eq!(ModelTier::Pro.agent_model_id(), "openagents/kimi-k3");
        assert_eq!(ModelTier::Pro.provider_id(), "openagents");
        assert_eq!(ModelTier::Pro.model_id(), "kimi-k3");
    }

    #[test]
    fn selection_is_process_wide() {
        clear_selection_for_test();
        select(ModelTier::Pro);
        assert_eq!(selected(), ModelTier::Pro);
        select(ModelTier::Luna);
        assert_eq!(selected(), ModelTier::Luna);
        clear_selection_for_test();
    }

    #[test]
    fn the_tier_order_leads_with_luna() {
        assert_eq!(
            ModelTier::ALL.iter().map(|t| t.name()).collect::<Vec<_>>(),
            vec!["Luna", "Flash", "Pro"]
        );
    }
}
