use gpui::{App, IntoElement, RenderOnce, Window};
use ui::prelude::*;

const FIXTURE_NPUB: &str = "npub1az708q3kd9zy6z6f44zav5ygvdwelkzspf6mtusttx47lft2z38sghk0w7";
const FIXTURE_FINGERPRINT: &str = "A7D3 8E02 3669 444D";
const MASKED_SECRET: &str = "•••• •••• •••• ••••";

fn wrapping_public_identity(value: &str) -> String {
    let mut display = String::with_capacity(value.len() + value.len() / 12);

    for (index, character) in value.chars().enumerate() {
        if index > 0 && index % 12 == 0 {
            display.push('\u{200b}');
        }
        display.push(character);
    }

    display
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum IdentityFixtureState {
    ResetFailed,
    Locked,
    RelaunchRequired,
    Conflict,
    Lost,
    Incomplete,
    Absent,
    Creating,
    Ready,
}

impl IdentityFixtureState {
    pub(crate) const ALL: [Self; 9] = [
        Self::ResetFailed,
        Self::Locked,
        Self::RelaunchRequired,
        Self::Conflict,
        Self::Lost,
        Self::Incomplete,
        Self::Absent,
        Self::Creating,
        Self::Ready,
    ];

    pub(crate) fn from_fixture_name(name: &str) -> Option<Self> {
        match name {
            "reset-failed" => Some(Self::ResetFailed),
            "locked" => Some(Self::Locked),
            "relaunch-required" => Some(Self::RelaunchRequired),
            "conflict" => Some(Self::Conflict),
            "lost" => Some(Self::Lost),
            "incomplete" => Some(Self::Incomplete),
            "absent" => Some(Self::Absent),
            "creating" => Some(Self::Creating),
            "ready" => Some(Self::Ready),
            _ => None,
        }
    }

    fn presentation(self) -> IdentityPresentation {
        match self {
            Self::ResetFailed => IdentityPresentation {
                title: "Reset didn't finish",
                description: "Omega kept identity setup blocked so the previous identity is not silently replaced.",
                icon: IconName::Warning,
                color: Color::Error,
                actions: &[IdentityAction::RetryReset, IdentityAction::Relaunch],
            },
            Self::Locked => IdentityPresentation {
                title: "System keychain locked",
                description: "Unlock the system keychain before Omega checks or uses your identity.",
                icon: IconName::Lock,
                color: Color::Warning,
                actions: &[IdentityAction::Retry],
            },
            Self::RelaunchRequired => IdentityPresentation {
                title: "Relaunch required",
                description: "Identity maintenance finished safely. Relaunch Omega to continue.",
                icon: IconName::Info,
                color: Color::Accent,
                actions: &[IdentityAction::Relaunch],
            },
            Self::Conflict => IdentityPresentation {
                title: "Identity choice required",
                description: "Omega found more than one public identity and will not choose between them for you.",
                icon: IconName::Warning,
                color: Color::Warning,
                actions: &[IdentityAction::Resolve],
            },
            Self::Lost => IdentityPresentation {
                title: "Recovery needed",
                description: "The public identity is known, but its signing key is not available in secure custody.",
                icon: IconName::LockOff,
                color: Color::Error,
                actions: &[IdentityAction::Recover, IdentityAction::Reset],
            },
            Self::Incomplete => IdentityPresentation {
                title: "Identity setup needs repair",
                description: "A prior identity transaction stopped before its public record was committed.",
                icon: IconName::Warning,
                color: Color::Warning,
                actions: &[IdentityAction::Retry],
            },
            Self::Absent => IdentityPresentation {
                title: "Create your Omega identity",
                description: "Create a local Nostr identity for signed work, portable social context, and agent coordination.",
                icon: IconName::Person,
                color: Color::Accent,
                actions: &[IdentityAction::Create, IdentityAction::Recover],
            },
            Self::Creating => IdentityPresentation {
                title: "Creating your identity…",
                description: "Omega is preparing secure local custody and verifying the public identity.",
                icon: IconName::Lock,
                color: Color::Accent,
                actions: &[],
            },
            Self::Ready => IdentityPresentation {
                title: "Identity ready",
                description: "Your public identity is available. The signing key remains in secure local custody.",
                icon: IconName::UserCheck,
                color: Color::Success,
                actions: &[IdentityAction::RecoveryOptions],
            },
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum IdentityAction {
    Create,
    Recover,
    Retry,
    Resolve,
    RetryReset,
    Reset,
    Relaunch,
    RecoveryOptions,
}

impl IdentityAction {
    fn id(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Recover => "recover",
            Self::Retry => "retry",
            Self::Resolve => "resolve",
            Self::RetryReset => "retry-reset",
            Self::Reset => "reset",
            Self::Relaunch => "relaunch",
            Self::RecoveryOptions => "recovery-options",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Create => "Create identity",
            Self::Recover => "Recover identity",
            Self::Retry => "Try again",
            Self::Resolve => "Review identities",
            Self::RetryReset => "Retry reset",
            Self::Reset => "Reset identity",
            Self::Relaunch => "Relaunch Omega",
            Self::RecoveryOptions => "Recovery options",
        }
    }

    fn is_primary(self) -> bool {
        matches!(
            self,
            Self::Create | Self::Retry | Self::Resolve | Self::RetryReset | Self::Relaunch
        )
    }
}

struct IdentityPresentation {
    title: &'static str,
    description: &'static str,
    icon: IconName,
    color: Color,
    actions: &'static [IdentityAction],
}

#[derive(IntoElement)]
struct MaskedImportFixture;

impl RenderOnce for MaskedImportFixture {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        v_flex()
            .min_w_0()
            .gap_3()
            .child(Label::new("Recover an existing identity"))
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        Label::new("Secret key")
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    )
                    .child(Label::new(MASKED_SECRET)),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        Label::new("Derived public identity")
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    )
                    .child(
                        Label::new(wrapping_public_identity(FIXTURE_NPUB)).size(LabelSize::Small),
                    ),
            )
            .child(
                Label::new("Fixture preview only — no secret is held or imported.")
                    .color(Color::Muted)
                    .size(LabelSize::XSmall),
            )
    }
}

#[derive(IntoElement, RegisterComponent)]
pub(crate) struct IdentitySection {
    state: IdentityFixtureState,
    first_tab_index: isize,
    actions_enabled: bool,
    show_fixture_notice: bool,
}

impl IdentitySection {
    pub(crate) fn new(
        state: IdentityFixtureState,
        first_tab_index: isize,
        actions_enabled: bool,
    ) -> Self {
        Self {
            state,
            first_tab_index,
            actions_enabled,
            show_fixture_notice: false,
        }
    }

    fn show_fixture_notice(mut self) -> Self {
        self.show_fixture_notice = true;
        self
    }
}

impl RenderOnce for IdentitySection {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let presentation = self.state.presentation();
        let actions_enabled = self.actions_enabled;

        v_flex()
            .min_w_0()
            .gap_3()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(Label::new("Your identity"))
                    .child(
                        Label::new(
                            "Omega uses a Nostr key pair as your portable public identity. Your private key stays in secure local custody.",
                        )
                        .color(Color::Muted),
                    ),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .gap_2()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_2()
                            .child(
                                Icon::new(presentation.icon)
                                    .size(IconSize::Small)
                                    .color(presentation.color),
                            )
                            .child(Label::new(presentation.title)),
                    )
                    .child(
                        Label::new(presentation.description)
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                    )
                    .when(self.state == IdentityFixtureState::Ready, |this| {
                        this.child(
                            v_flex()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    Label::new("Public identity")
                                        .color(Color::Muted)
                                        .size(LabelSize::XSmall),
                                )
                                .child(
                                    Label::new(wrapping_public_identity(FIXTURE_NPUB))
                                        .size(LabelSize::Small),
                                )
                                .child(
                                    Label::new(format!("Fingerprint {FIXTURE_FINGERPRINT}"))
                                        .color(Color::Muted)
                                        .size(LabelSize::XSmall),
                                ),
                        )
                    })
                    .when(self.show_fixture_notice, |this| {
                        this.child(
                            Label::new("Fixture preview only — no key is created or stored.")
                                .color(Color::Muted)
                                .size(LabelSize::XSmall),
                        )
                    })
                    .when(
                        !actions_enabled && !presentation.actions.is_empty(),
                        |this| {
                            this.child(
                                Label::new(
                                    "Identity actions remain unavailable until secure custody is installed.",
                                )
                                .color(Color::Muted)
                                .size(LabelSize::XSmall),
                            )
                        },
                    )
                    .when(!presentation.actions.is_empty(), |this| {
                        this.child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .children(presentation.actions.iter().enumerate().map(
                                    |(index, action)| {
                                        Button::new(
                                            format!("omega-identity-{}", action.id()),
                                            action.label(),
                                        )
                                        .style(if action.is_primary() {
                                            ButtonStyle::Filled
                                        } else {
                                            ButtonStyle::OutlinedGhost
                                        })
                                        .size(ButtonSize::Medium)
                                        .tab_index(self.first_tab_index + index as isize)
                                        .disabled(!actions_enabled)
                                        .on_click(|_, _, _| {})
                                    },
                                )),
                        )
                    }),
            )
    }
}

impl Component for IdentitySection {
    fn scope() -> ComponentScope {
        ComponentScope::Onboarding
    }

    fn name() -> &'static str {
        "Omega Identity Section"
    }

    fn description() -> &'static str {
        "Fixture-backed public states for Omega identity-first onboarding."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .p_4()
            .children(IdentityFixtureState::ALL.into_iter().map(|state| {
                single_example(
                    format!("{state:?}"),
                    IdentitySection::new(state, 0, true)
                        .show_fixture_notice()
                        .into_any_element(),
                )
            }))
            .child(single_example(
                "Masked import",
                MaskedImportFixture.into_any_element(),
            ))
            .into_any_element()
    }
}

pub(crate) fn fixture_state_for_current_build() -> IdentityFixtureState {
    if cfg!(debug_assertions)
        && let Ok(fixture_name) = std::env::var("OMEGA_IDENTITY_FIXTURE")
        && let Some(state) = IdentityFixtureState::from_fixture_name(&fixture_name)
    {
        return state;
    }

    IdentityFixtureState::Absent
}

pub(crate) fn render_identity_section(
    tab_index: &mut isize,
    state: IdentityFixtureState,
) -> impl IntoElement {
    let first_tab_index = *tab_index;
    *tab_index += state.presentation().actions.len() as isize;

    IdentitySection::new(state, first_tab_index, false).show_fixture_notice()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn fixture_states_are_exhaustive_and_unique() {
        assert_eq!(
            IdentityFixtureState::ALL
                .into_iter()
                .collect::<HashSet<_>>()
                .len(),
            9
        );
    }

    #[test]
    fn fixture_names_cover_every_state() {
        for name in [
            "reset-failed",
            "locked",
            "relaunch-required",
            "conflict",
            "lost",
            "incomplete",
            "absent",
            "creating",
            "ready",
        ] {
            assert!(IdentityFixtureState::from_fixture_name(name).is_some());
        }
    }

    #[test]
    fn ready_fixture_contains_only_public_identity_data() {
        assert!(FIXTURE_NPUB.starts_with("npub1"));
        assert!(!FIXTURE_NPUB.to_lowercase().contains("secret"));
        assert!(!FIXTURE_FINGERPRINT.to_lowercase().contains("secret"));
    }

    #[test]
    fn identity_actions_reserve_focus_before_theme() {
        let mut tab_index = 0;
        {
            let _section = render_identity_section(&mut tab_index, IdentityFixtureState::Absent);
        }
        assert_eq!(tab_index, 2);
    }

    #[test]
    fn masked_import_fixture_contains_no_secret_material() {
        assert!(
            MASKED_SECRET
                .chars()
                .all(|character| character == '•' || character.is_whitespace())
        );
        assert!(!MASKED_SECRET.contains("nsec"));
    }

    #[test]
    fn public_identity_has_safe_narrow_window_breaks() {
        assert_eq!(
            wrapping_public_identity(FIXTURE_NPUB).replace('\u{200b}', ""),
            FIXTURE_NPUB
        );
    }
}
