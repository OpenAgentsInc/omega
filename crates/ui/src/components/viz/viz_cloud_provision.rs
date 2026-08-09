use documented::Documented;
use gpui::px;

use crate::components::viz::VizProgressRail;
use crate::prelude::*;
use crate::traits::animation_ext::CommonAnimationExt as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudProvisionStage {
    Payment,
    Relay,
    Provider,
    Connected,
}

impl CloudProvisionStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Payment => "payment",
            Self::Relay => "relay",
            Self::Provider => "provider",
            Self::Connected => "connected",
        }
    }

    fn rail_position(self) -> (usize, Option<usize>) {
        match self {
            Self::Payment => (0, Some(0)),
            Self::Relay => (1, Some(1)),
            Self::Provider => (2, Some(2)),
            Self::Connected => (4, None),
        }
    }

    fn relay_ready(self) -> bool {
        matches!(self, Self::Relay | Self::Provider | Self::Connected)
    }

    fn provider_ready(self) -> bool {
        matches!(self, Self::Provider | Self::Connected)
    }
}

const PROVISION_STAGES: [&str; 4] = ["payment", "relay", "provider", "connected"];

/// An inline lifecycle card for a paid provider node and its cloud relay.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct CloudProvisionCard {
    provider_name: SharedString,
    region: SharedString,
    relay_id: SharedString,
    provider_id: SharedString,
    stage: CloudProvisionStage,
}

impl CloudProvisionCard {
    pub fn new(
        provider_name: impl Into<SharedString>,
        region: impl Into<SharedString>,
        relay_id: impl Into<SharedString>,
        provider_id: impl Into<SharedString>,
    ) -> Self {
        Self {
            provider_name: provider_name.into(),
            region: region.into(),
            relay_id: relay_id.into(),
            provider_id: provider_id.into(),
            stage: CloudProvisionStage::Payment,
        }
    }

    pub fn stage(mut self, stage: CloudProvisionStage) -> Self {
        self.stage = stage;
        self
    }
}

impl RenderOnce for CloudProvisionCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors();
        let card_border = colors.border.opacity(0.8);
        let header_background = colors
            .element_background
            .blend(colors.editor_foreground.opacity(0.025));
        let stage = self.stage;
        let (completed, active) = stage.rail_position();
        let mut rail = VizProgressRail::new(PROVISION_STAGES)
            .completed(completed)
            .scale(1.0);
        if let Some(active) = active {
            rail = rail.active(active);
        }

        let readiness_icon = |ready: bool| {
            if ready {
                Icon::new(IconName::Check)
                    .size(IconSize::Small)
                    .color(Color::Success)
                    .into_any_element()
            } else {
                Icon::new(IconName::TodoProgress)
                    .size(IconSize::Small)
                    .color(Color::Muted)
                    .with_rotate_animation(2)
                    .into_any_element()
            }
        };
        let header_icon = if stage == CloudProvisionStage::Connected {
            Icon::new(IconName::Check)
                .size(IconSize::Small)
                .color(Color::Success)
                .into_any_element()
        } else {
            Icon::new(IconName::TodoProgress)
                .size(IconSize::Small)
                .color(Color::Accent)
                .with_rotate_animation(2)
                .into_any_element()
        };

        v_flex()
            .w(px(420.))
            .my_1p5()
            .rounded_md()
            .border_1()
            .border_color(card_border)
            .bg(colors.editor_background)
            .overflow_hidden()
            .child(
                h_flex()
                    .h_8()
                    .w_full()
                    .p_1()
                    .px_2()
                    .gap_2()
                    .bg(header_background)
                    .child(
                        Label::new(self.provider_name)
                            .size(LabelSize::Custom(rems_from_px(13.)))
                            .buffer_font(cx),
                    )
                    .child(
                        Label::new(self.region)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .buffer_font(cx),
                    )
                    .child(div().flex_1())
                    .child(header_icon),
            )
            .child(div().px_2().child(rail))
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .child(
                        Icon::new(IconName::Public)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(Label::new("Relay").size(LabelSize::Small))
                    .child(
                        Label::new(self.relay_id)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .buffer_font(cx),
                    )
                    .child(div().flex_1())
                    .child(readiness_icon(stage.relay_ready())),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .border_t_1()
                    .border_color(card_border)
                    .child(
                        Icon::new(IconName::Server)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(Label::new("Provider").size(LabelSize::Small))
                    .child(
                        Label::new(self.provider_id)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .buffer_font(cx),
                    )
                    .child(div().flex_1())
                    .child(readiness_icon(stage.provider_ready())),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .gap_1p5()
                    .border_t_1()
                    .border_color(card_border)
                    .child(
                        Icon::new(IconName::Check)
                            .size(IconSize::XSmall)
                            .color(Color::Success),
                    )
                    .child(
                        Label::new("paid account · mock")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1())
                    .child(
                        Label::new("OpenAgents cloud")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
    }
}

impl Component for CloudProvisionCard {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "In conversation",
                vec![
                    single_example(
                        "Relay ready, provider provisioning",
                        CloudProvisionCard::new(
                            "Northstar",
                            "us-central1",
                            "mock-cloud-1-relay",
                            "mock-cloud-1-provider",
                        )
                        .stage(CloudProvisionStage::Provider)
                        .into_any_element(),
                    ),
                    single_example(
                        "Connected to OpenAgents cloud",
                        CloudProvisionCard::new(
                            "Northstar",
                            "us-central1",
                            "mock-cloud-1-relay",
                            "mock-cloud-1-provider",
                        )
                        .stage(CloudProvisionStage::Connected)
                        .into_any_element(),
                    ),
                ],
            ))
            .into_any_element()
    }
}
