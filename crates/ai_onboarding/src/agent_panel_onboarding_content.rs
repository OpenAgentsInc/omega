use std::sync::Arc;

use gpui::{Entity, IntoElement, ParentElement};
use language_model::{LanguageModelRegistry, ZED_CLOUD_PROVIDER_ID};
use ui::prelude::*;

use crate::{AgentPanelOnboardingCard, ApiKeysWithProviders, ApiKeysWithoutProviders};

pub struct AgentPanelOnboarding {
    has_configured_providers: bool,
    configured_providers: Entity<ApiKeysWithProviders>,
    continue_to_agent: Arc<dyn Fn(&mut Window, &mut App)>,
}

impl AgentPanelOnboarding {
    pub fn new(
        continue_to_agent: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(
            &LanguageModelRegistry::global(cx),
            |this: &mut Self, _registry, event: &language_model::Event, cx| match event {
                language_model::Event::ProviderStateChanged(_)
                | language_model::Event::AddedProvider(_)
                | language_model::Event::RemovedProvider(_)
                | language_model::Event::ProvidersChanged => {
                    this.has_configured_providers = Self::has_configured_providers(cx)
                }
                _ => {}
            },
        )
        .detach();

        Self {
            has_configured_providers: Self::has_configured_providers(cx),
            configured_providers: cx.new(ApiKeysWithProviders::new),
            continue_to_agent: Arc::new(continue_to_agent),
        }
    }

    fn has_configured_providers(cx: &App) -> bool {
        LanguageModelRegistry::read_global(cx)
            .visible_providers()
            .iter()
            .any(|provider| provider.is_authenticated(cx) && provider.id() != ZED_CLOUD_PROVIDER_ID)
    }
}

impl Render for AgentPanelOnboarding {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        AgentPanelOnboardingCard::new()
            .child(
                v_flex()
                    .gap_1()
                    .child(Headline::new("Connect an AI provider"))
                    .child(
                        Label::new(
                            "Omega uses providers and credentials you configure directly. Choose an existing provider or add one in Agent Settings.",
                        )
                        .color(Color::Muted),
                    ),
            )
            .when(self.has_configured_providers, |this| {
                let continue_to_agent = self.continue_to_agent.clone();
                this.child(self.configured_providers.clone()).child(
                    Button::new("continue-to-agent", "Continue")
                        .full_width()
                        .style(ButtonStyle::Filled)
                        .on_click(move |_, window, cx| continue_to_agent(window, cx)),
                )
            })
            .when(!self.has_configured_providers, |this| {
                this.child(ApiKeysWithoutProviders::new())
            })
    }
}
