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

    pub const ALL: [Self; 7] = [
        Self::Ready,
        Self::Running,
        Self::Complete,
        Self::Blocked,
        Self::Warning,
        Self::Failed,
        Self::Offline,
    ];

    /// Each status owns its own shape. Color repeats the meaning; it never
    /// carries it alone, so the cue survives a person who cannot separate the
    /// theme's success, warning, and error hues.
    const fn icon(self) -> IconName {
        match self {
            Self::Ready => IconName::Circle,
            Self::Running => IconName::LoadCircle,
            Self::Complete => IconName::Check,
            Self::Blocked => IconName::Slash,
            Self::Warning => IconName::Warning,
            Self::Failed => IconName::XCircle,
            Self::Offline => IconName::Disconnected,
        }
    }

    const fn color(self) -> Color {
        match self {
            Self::Ready | Self::Complete => Color::Success,
            Self::Running => Color::Accent,
            Self::Blocked | Self::Failed => Color::Error,
            Self::Warning => Color::Warning,
            Self::Offline => Color::Muted,
        }
    }
}

/// Render status without visible status prose. Shape, color, the one-word
/// tooltip, and the exact accessible context all carry the same state; none of
/// them is the only channel.
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
            Icon::new(status.icon())
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
        for status in OmegaStatus::ALL {
            assert_eq!(status.word().split_whitespace().count(), 1);
        }
    }

    #[test]
    fn every_status_word_is_registered_with_the_copy_lint() {
        let words = OmegaStatus::ALL
            .iter()
            .map(|status| status.word())
            .collect::<Vec<_>>();
        assert!(
            omega_control_crawl::lint_status_words(words).is_empty(),
            "the cue offers a status word the copy lint does not admit"
        );
    }

    #[test]
    fn color_is_never_the_only_cue() {
        let mut icons = Vec::new();
        for status in OmegaStatus::ALL {
            let icon = status.icon();
            assert!(
                !icons.contains(&icon),
                "{status:?} reuses a shape already used by another status"
            );
            icons.push(icon);
        }
        assert_eq!(icons.len(), OmegaStatus::ALL.len());
    }
}
