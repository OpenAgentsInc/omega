use std::collections::HashMap;

use gpui::{App, AppContext as _, Entity, EntityId, Global, SharedString};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ComposerVoicePhase {
    #[default]
    Unavailable,
    Idle,
    Authenticating,
    RequestingMicrophone,
    Connecting,
    Listening,
    UserSpeaking,
    SarahSpeaking,
    Reconnecting,
    Ending,
    AccessRequired,
    Error,
}

impl ComposerVoicePhase {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::RequestingMicrophone
                | Self::Connecting
                | Self::Listening
                | Self::UserSpeaking
                | Self::SarahSpeaking
                | Self::Reconnecting
        )
    }

    pub fn is_starting(self) -> bool {
        matches!(
            self,
            Self::Authenticating | Self::RequestingMicrophone | Self::Connecting
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Voice unavailable",
            Self::Idle => "Start voice",
            Self::Authenticating => "Authenticating…",
            Self::RequestingMicrophone => "Requesting microphone…",
            Self::Connecting => "Connecting…",
            Self::Listening => "Listening",
            Self::UserSpeaking => "You are speaking",
            Self::SarahSpeaking => "Sarah is speaking",
            Self::Reconnecting => "Reconnect required",
            Self::Ending => "Ending voice…",
            Self::AccessRequired => "Voice credits required",
            Self::Error => "Voice error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposerVoiceStatus {
    pub phase: ComposerVoicePhase,
    pub detail: SharedString,
    pub muted: bool,
    pub retryable: bool,
}

impl Default for ComposerVoiceStatus {
    fn default() -> Self {
        Self {
            phase: ComposerVoicePhase::Unavailable,
            detail: "Sarah voice is not available in this workspace.".into(),
            muted: false,
            retryable: false,
        }
    }
}

#[derive(Default)]
struct GlobalComposerVoiceStatus(HashMap<EntityId, Entity<ComposerVoiceStatus>>);

impl Global for GlobalComposerVoiceStatus {}

pub fn composer_voice_status(workspace_id: EntityId, cx: &mut App) -> Entity<ComposerVoiceStatus> {
    if let Some(status) = cx
        .default_global::<GlobalComposerVoiceStatus>()
        .0
        .get(&workspace_id)
        .cloned()
    {
        return status;
    }

    let status = cx.new(|_| ComposerVoiceStatus::default());
    cx.default_global::<GlobalComposerVoiceStatus>()
        .0
        .insert(workspace_id, status.clone());
    status
}

pub fn set_composer_voice_status(
    workspace_id: EntityId,
    status: ComposerVoiceStatus,
    cx: &mut App,
) {
    composer_voice_status(workspace_id, cx).update(cx, |current, cx| {
        if *current != status {
            *current = status;
            cx.notify();
        }
    });
}

pub fn remove_composer_voice_status(workspace_id: EntityId, cx: &mut App) {
    cx.default_global::<GlobalComposerVoiceStatus>()
        .0
        .remove(&workspace_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_phases_expose_compact_composer_semantics() {
        assert_eq!(ComposerVoicePhase::Idle.label(), "Start voice");
        assert!(ComposerVoicePhase::Connecting.is_starting());
        assert!(ComposerVoicePhase::Listening.is_active());
        assert!(!ComposerVoicePhase::Error.is_active());
        assert_eq!(
            ComposerVoicePhase::AccessRequired.label(),
            "Voice credits required"
        );
        assert_eq!(
            ComposerVoicePhase::SarahSpeaking.label(),
            "Sarah is speaking"
        );
    }
}
