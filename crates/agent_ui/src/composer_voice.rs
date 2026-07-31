use std::collections::HashMap;

use gpui::{Action as _, AnyElement, App, AppContext as _, Entity, EntityId, Global, SharedString};
use ui::{TintColor, Tooltip, prelude::*};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SarahVoiceConfirmation {
    NoExtraConfirmation,
    ConfirmEachAction,
}

impl SarahVoiceConfirmation {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoExtraConfirmation => "No extra confirmation",
            Self::ConfirmEachAction => "Confirm each action",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SarahVoiceCapability {
    pub capability: SarahVoiceCapabilityId,
    pub confirmation: SarahVoiceConfirmation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SarahVoiceCapabilityId {
    ContextRead,
    OpenPath,
    RevealRange,
    ReplaceSelection,
    SaveDocument,
    StartAgentThread,
}

impl SarahVoiceCapabilityId {
    pub fn label(self) -> &'static str {
        match self {
            Self::ContextRead => "Read active editor context",
            Self::OpenPath => "Open a named path",
            Self::RevealRange => "Reveal a line range",
            Self::ReplaceSelection => "Replace the active selection",
            Self::SaveDocument => "Save the active document",
            Self::StartAgentThread => "Start a bounded Omega Agent thread",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SarahVoiceExcludedAuthority {
    DirectShell,
    DirectGit,
    Payment,
    CredentialAccess,
    DeviceControl,
}

impl SarahVoiceExcludedAuthority {
    pub fn label(self) -> &'static str {
        match self {
            Self::DirectShell => "direct shell",
            Self::DirectGit => "direct Git",
            Self::Payment => "payments",
            Self::CredentialAccess => "credential access",
            Self::DeviceControl => "device control",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SarahVoiceAdmissionTerms {
    pub client_profile: SharedString,
    pub cohort_ref: SharedString,
    pub credit_mode: SarahVoiceCreditMode,
    pub rate_msat_per_million_tokens: u64,
    pub credit_hold_msat: u64,
    pub remaining_credit_msat: Option<u64>,
    pub max_duration_seconds: u32,
    pub transcript_policy: SharedString,
    pub capabilities: Vec<SarahVoiceCapability>,
    pub excluded_authorities: Vec<SarahVoiceExcludedAuthority>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SarahVoiceCreditMode {
    Metered,
    StagingOwnerEntitlement,
}

impl SarahVoiceCreditMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Metered => "Metered credit",
            Self::StagingOwnerEntitlement => "Staging owner entitlement",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SarahVoiceParticipant {
    User,
    Sarah,
}

impl SarahVoiceParticipant {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "You",
            Self::Sarah => "Sarah",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SarahVoiceTranscriptRow {
    pub thread_ref: SharedString,
    pub session_ref: SharedString,
    pub item_id: SharedString,
    pub participant: SarahVoiceParticipant,
    pub text: SharedString,
    pub complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SarahVoiceSelectionEffectPreview {
    pub workspace_ref: SharedString,
    pub document_version: SharedString,
    pub target_path: SharedString,
    pub selection_start_line: u32,
    pub selection_start_column: u32,
    pub selection_end_line: u32,
    pub selection_end_column: u32,
    pub selected_text: SharedString,
    pub replacement_text: SharedString,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SarahVoicePendingConfirmation {
    pub request_id: SharedString,
    pub copy: SharedString,
    pub detail: Option<SharedString>,
    pub selection_effect: Option<SarahVoiceSelectionEffectPreview>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SarahAgentThreadPresentation {
    Foreground,
    Background,
}

impl SarahAgentThreadPresentation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Foreground => "Foreground · opened in Agent panel",
            Self::Background => "Background · active view and focus were preserved",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SarahVoiceAgentThreadReceipt {
    pub thread_id: SharedString,
    pub presentation: SarahAgentThreadPresentation,
    pub status: SharedString,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SarahVoiceSessionArtifacts {
    pub transcript: Vec<SarahVoiceTranscriptRow>,
    pub pending_confirmation: Option<SarahVoicePendingConfirmation>,
    pub created_agent_thread: Option<SarahVoiceAgentThreadReceipt>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SarahVoiceAdmissionProjection {
    Loading {
        detail: SharedString,
    },
    Ready {
        terms: SarahVoiceAdmissionTerms,
    },
    Active {
        terms: SarahVoiceAdmissionTerms,
        session_id: SharedString,
        artifacts: SarahVoiceSessionArtifacts,
    },
    Unavailable {
        reason: SharedString,
        retryable: bool,
        cohort_ref: Option<SharedString>,
        refusal_reason: Option<SharedString>,
    },
    Settled {
        final_charge_msat: u64,
        remaining_credit_msat: Option<u64>,
        receipt_ref: Option<SharedString>,
        transcript_recovery: SharedString,
        artifacts: SarahVoiceSessionArtifacts,
    },
}

impl Default for SarahVoiceAdmissionProjection {
    fn default() -> Self {
        Self::Unavailable {
            reason: "Sarah admission terms have not been loaded for this workspace.".into(),
            retryable: true,
            cohort_ref: None,
            refusal_reason: None,
        }
    }
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

#[derive(Default)]
struct GlobalSarahVoiceAdmission(HashMap<EntityId, Entity<SarahVoiceAdmissionProjection>>);

impl Global for GlobalSarahVoiceAdmission {}

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

pub fn sarah_voice_admission(
    workspace_id: EntityId,
    cx: &mut App,
) -> Entity<SarahVoiceAdmissionProjection> {
    if let Some(projection) = cx
        .default_global::<GlobalSarahVoiceAdmission>()
        .0
        .get(&workspace_id)
        .cloned()
    {
        return projection;
    }

    let projection = cx.new(|_| SarahVoiceAdmissionProjection::default());
    cx.default_global::<GlobalSarahVoiceAdmission>()
        .0
        .insert(workspace_id, projection.clone());
    projection
}

pub fn set_sarah_voice_admission(
    workspace_id: EntityId,
    projection: SarahVoiceAdmissionProjection,
    cx: &mut App,
) {
    sarah_voice_admission(workspace_id, cx).update(cx, |current, cx| {
        if *current != projection {
            *current = projection;
            cx.notify();
        }
    });
}

pub fn remove_sarah_voice_admission(workspace_id: EntityId, cx: &mut App) {
    cx.default_global::<GlobalSarahVoiceAdmission>()
        .0
        .remove(&workspace_id);
}

/// The composer's voice controls, for every composer.
///
/// `OMEGA-DELTA-0204`. This lived as a private method on `ThreadView`, so the
/// pre-session composer — the one a person meets first, on a brand new thread —
/// could not render it and did not. Voice is a property of the workspace's
/// Sarah session, not of an ACP session, so nothing about a thread that has not
/// connected yet makes the microphone unavailable; it was missing only because
/// the code that draws it was out of reach.
pub fn render_composer_voice_controls(workspace_id: EntityId, cx: &mut App) -> AnyElement {
    use crate::OpenSarahAdmission;
    use omega_actions::workroom::{EndVoice, ToggleVoiceMute};

    let status = composer_voice_status(workspace_id, cx).read(cx).clone();
    let phase = status.phase;
    let detail = status.detail.clone();
    let label = if status.muted {
        "Microphone muted"
    } else {
        phase.label()
    };
    let label_color = match phase {
        ComposerVoicePhase::AccessRequired
        | ComposerVoicePhase::Error
        | ComposerVoicePhase::Reconnecting => Color::Error,
        phase if phase.is_active() || phase.is_starting() => Color::Accent,
        _ => Color::Muted,
    };
    let primary_icon = if status.muted {
        IconName::MicMute
    } else {
        IconName::Mic
    };

    h_flex()
        .id("agent-composer-voice-controls")
        .debug_selector(|| "agent.composer.voice-controls".into())
        .gap_0p5()
        .when(
            phase.is_active()
                || phase.is_starting()
                || matches!(
                    phase,
                    ComposerVoicePhase::Ending
                        | ComposerVoicePhase::AccessRequired
                        | ComposerVoicePhase::Error
                ),
            |this| this.child(Label::new(label).size(LabelSize::XSmall).color(label_color)),
        )
        .child(
            IconButton::new("agent-composer-voice", primary_icon)
                .debug_selector(|| "agent.composer.voice".into())
                .icon_size(IconSize::Small)
                .icon_color(label_color)
                .style(if phase.is_active() {
                    ButtonStyle::Tinted(TintColor::Accent)
                } else {
                    ButtonStyle::Subtle
                })
                .toggle_state(phase.is_active() && !status.muted)
                .disabled(matches!(
                    phase,
                    ComposerVoicePhase::Authenticating | ComposerVoicePhase::Ending
                ))
                .aria_label(label)
                .aria_description(detail.clone())
                .tooltip(move |_, cx| Tooltip::with_meta(label, None, detail.clone(), cx))
                .on_click(move |_, window, cx| match phase {
                    ComposerVoicePhase::Idle
                    | ComposerVoicePhase::Unavailable
                    | ComposerVoicePhase::AccessRequired
                    | ComposerVoicePhase::Error
                    | ComposerVoicePhase::Reconnecting => {
                        window.dispatch_action(OpenSarahAdmission.boxed_clone(), cx)
                    }
                    phase if phase.is_active() => {
                        window.dispatch_action(ToggleVoiceMute.boxed_clone(), cx)
                    }
                    _ => {}
                }),
        )
        .when(
            phase.is_active()
                || phase.is_starting()
                || matches!(
                    phase,
                    ComposerVoicePhase::AccessRequired | ComposerVoicePhase::Error
                ),
            |this| {
                this.child(
                    IconButton::new("agent-composer-end-voice", IconName::Stop)
                        .debug_selector(|| "agent.composer.end-voice".into())
                        .icon_size(IconSize::Small)
                        .icon_color(Color::Error)
                        .aria_label("End voice")
                        .tooltip(Tooltip::text("End Sarah voice"))
                        .on_click(|_, window, cx| {
                            window.dispatch_action(EndVoice.boxed_clone(), cx);
                        }),
                )
            },
        )
        .into_any_element()
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

    #[test]
    fn admission_projection_keeps_exact_terms_and_confirmation_classes() {
        let terms = SarahVoiceAdmissionTerms {
            client_profile: "omega_editor".into(),
            cohort_ref: "alpha_v1".into(),
            credit_mode: SarahVoiceCreditMode::Metered,
            rate_msat_per_million_tokens: 64_000_000,
            credit_hold_msat: 256_000,
            remaining_credit_msat: Some(8_000_000),
            max_duration_seconds: 300,
            transcript_policy: "Stored locally for transcript recovery.".into(),
            capabilities: vec![SarahVoiceCapability {
                capability: SarahVoiceCapabilityId::SaveDocument,
                confirmation: SarahVoiceConfirmation::ConfirmEachAction,
            }],
            excluded_authorities: vec![SarahVoiceExcludedAuthority::DirectShell],
        };
        let projection = SarahVoiceAdmissionProjection::Ready {
            terms: terms.clone(),
        };

        assert_eq!(projection, SarahVoiceAdmissionProjection::Ready { terms });
        assert_eq!(
            SarahVoiceConfirmation::ConfirmEachAction.label(),
            "Confirm each action"
        );
    }
}
