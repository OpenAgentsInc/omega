//! Shared non-text status cue for Omega-owned chrome.

use gpui::{AnyElement, ElementId};
use ui::{Color, Icon, IconName, IconSize, Tooltip, h_flex, prelude::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmegaStatus {
    Ready,
    Running,
    Complete,
    Blocked,
    Warning,
    Failed,
    Offline,
}

impl OmegaStatus {
    pub const fn word(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Running => "Running",
            Self::Complete => "Complete",
            Self::Blocked => "Blocked",
            Self::Warning => "Warning",
            Self::Failed => "Failed",
            Self::Offline => "Offline",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Ready | Self::Complete => Color::Success,
            Self::Running => Color::Accent,
            Self::Blocked | Self::Failed => Color::Error,
            Self::Warning => Color::Warning,
            Self::Offline => Color::Muted,
        }
    }
}

/// Render status without visible status prose. Color is redundant with the
/// icon's one-word tooltip and exact accessible context.
pub fn omega_status_cue(
    id: impl Into<ElementId>,
    status: OmegaStatus,
    context: &str,
) -> AnyElement {
    let word = status.word();
    h_flex()
        .id(id)
        .role(gpui::Role::Status)
        .aria_label(format!("{context}: {word}"))
        .tooltip(Tooltip::text(word))
        .child(
            Icon::new(IconName::Circle)
                .size(IconSize::XSmall)
                .color(status.color()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tooltip_is_one_word() {
        for status in [
            OmegaStatus::Ready,
            OmegaStatus::Running,
            OmegaStatus::Complete,
            OmegaStatus::Blocked,
            OmegaStatus::Warning,
            OmegaStatus::Failed,
            OmegaStatus::Offline,
        ] {
            assert_eq!(status.word().split_whitespace().count(), 1);
        }
    }
}
