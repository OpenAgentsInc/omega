//! Omega model tier selector. Flash (default) and Pro.
//!
//! Product surface for the two hosted OpenAgents inference lanes:
//! - **Flash** — `google/gemini-3.6-flash` (default; hosted Gemini path)
//! - **Pro** — `openagents/kimi-k3` (Fireworks via OpenAgents chat completions)
//!
//! The zero-base composer bar owns this control. It is a closed two-item menu
//! rather than the full model picker, matching the owner's "dropdowns inside
//! the input bar" direction without reopening the full model catalog.

use std::rc::Rc;
use std::sync::Mutex;

use gpui::{AnyElement, App, Window};
use ui::{Button, ContextMenu, ContextMenuEntry, PopoverMenu, Tooltip, prelude::*};

/// Process-wide standing tier choice. Defaults to Flash.
static SELECTED: Mutex<ModelTier> = Mutex::new(ModelTier::Flash);

/// The two product tiers Omega offers on the native loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelTier {
    /// Gemini 3.6 Flash — default, hosted through OpenAgents when zero base.
    Flash,
    /// Kimi K3 — stronger coding lane through OpenAgents Fireworks.
    Pro,
}

impl ModelTier {
    pub const ALL: &'static [Self] = &[Self::Flash, Self::Pro];

    /// Label shown on the trigger and menu.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Flash => "Flash",
            Self::Pro => "Pro",
        }
    }

    /// One sentence for the menu documentation aside.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Flash => "Gemini 3.6 Flash — default, fast hosted lane.",
            Self::Pro => "Kimi K3 — stronger coding lane via OpenAgents Fireworks.",
        }
    }

    /// Agent model id the native agent selects for this tier.
    ///
    /// Format matches `LanguageModels::model_id`: `provider/model`.
    #[must_use]
    pub const fn agent_model_id(self) -> &'static str {
        match self {
            Self::Flash => "google/gemini-3.6-flash",
            Self::Pro => "openagents/kimi-k3",
        }
    }

    /// Provider id for settings persistence.
    #[must_use]
    pub const fn provider_id(self) -> &'static str {
        match self {
            Self::Flash => "google",
            Self::Pro => "openagents",
        }
    }

    /// Model id for settings persistence.
    #[must_use]
    pub const fn model_id(self) -> &'static str {
        match self {
            Self::Flash => "gemini-3.6-flash",
            Self::Pro => "kimi-k3",
        }
    }
}

/// Current standing tier. Flash when nobody has chosen.
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

/// Test-only reset to Flash.
#[cfg(any(test, feature = "test-support"))]
pub fn clear_selection_for_test() {
    *SELECTED
        .lock()
        .expect("the model tier selection is never held across a panic") = ModelTier::Flash;
}

/// Test-only setter without log side effects beyond select.
#[cfg(any(test, feature = "test-support"))]
pub fn select_for_test(tier: ModelTier) {
    select(tier);
}

const MENU_HEADER: &str = "Omega model";

/// Composer-bar Flash / Pro dropdown.
pub fn render_model_tier_selector(
    current: ModelTier,
    enabled: bool,
    on_select: Rc<dyn Fn(ModelTier, &mut Window, &mut App)>,
) -> AnyElement {
    let label = SharedString::from(current.name());
    let trigger = Button::new("omega-model-tier-selector", label)
        .label_size(LabelSize::XSmall)
        .color(Color::Muted)
        .disabled(!enabled)
        .end_icon(
            Icon::new(IconName::ChevronDown)
                .size(IconSize::XSmall)
                .color(Color::Muted),
        );

    let tooltip = SharedString::from(if enabled {
        format!(
            "{}. Switch between Flash (Gemini 3.6) and Pro (Kimi K3).",
            current.description()
        )
    } else {
        format!(
            "{}. Model is fixed while a turn is running.",
            current.description()
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

fn build_menu(
    current: ModelTier,
    on_select: Rc<dyn Fn(ModelTier, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Entity<ContextMenu> {
    ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
        menu = menu.header(MENU_HEADER);
        for tier in ModelTier::ALL {
            let is_current = current == *tier;
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
    fn flash_is_the_default() {
        clear_selection_for_test();
        assert_eq!(selected(), ModelTier::Flash);
        assert_eq!(ModelTier::Flash.agent_model_id(), "google/gemini-3.6-flash");
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
        select(ModelTier::Flash);
        assert_eq!(selected(), ModelTier::Flash);
        clear_selection_for_test();
    }

    #[test]
    fn there_are_exactly_two_tiers() {
        assert_eq!(
            ModelTier::ALL.iter().map(|t| t.name()).collect::<Vec<_>>(),
            vec!["Flash", "Pro"]
        );
    }
}
