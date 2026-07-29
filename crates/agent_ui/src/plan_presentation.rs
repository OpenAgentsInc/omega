use agent_client_protocol::schema::v1 as acp;
use gpui::{App, StrikethroughStyle, TextStyle, Window, px};
use markdown::{MarkdownFont, MarkdownStyle};
use theme::ActiveTheme;
use ui::{Color, IconName};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanStatusKind {
    Pending,
    InProgress,
    Completed,
    Unknown,
}

impl PlanStatusKind {
    pub fn from_acp(status: &acp::PlanEntryStatus) -> Self {
        match status {
            acp::PlanEntryStatus::Pending => Self::Pending,
            acp::PlanEntryStatus::InProgress => Self::InProgress,
            acp::PlanEntryStatus::Completed => Self::Completed,
            _ => Self::Unknown,
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Pending => IconName::TodoPending,
            Self::InProgress => IconName::TodoProgress,
            Self::Completed => IconName::TodoComplete,
            Self::Unknown => IconName::Warning,
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::Pending => Color::Muted,
            Self::InProgress => Color::Accent,
            Self::Completed => Color::Success,
            Self::Unknown => Color::Warning,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in progress",
            Self::Completed => "completed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanPriorityKind {
    High,
    Medium,
    Low,
    Unknown,
}

impl PlanPriorityKind {
    pub fn from_acp(priority: &acp::PlanEntryPriority) -> Self {
        match priority {
            acp::PlanEntryPriority::High => Self::High,
            acp::PlanEntryPriority::Medium => Self::Medium,
            acp::PlanEntryPriority::Low => Self::Low,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Unknown => "unknown",
        }
    }
}

pub fn plan_label_markdown_style(
    status: PlanStatusKind,
    window: &Window,
    cx: &App,
) -> MarkdownStyle {
    let default_markdown_style = MarkdownStyle::themed(MarkdownFont::Agent, window, cx);

    MarkdownStyle {
        base_text_style: TextStyle {
            color: cx.theme().colors().text_muted,
            strikethrough: if status == PlanStatusKind::Completed {
                Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(cx.theme().colors().text_muted.opacity(0.8)),
                })
            } else {
                None
            },
            ..default_markdown_style.base_text_style
        },
        ..default_markdown_style
    }
}
