//! Sarah workroom dock panel (`OMEGA-SW-03` / `OMEGA-SW-04` / `OMEGA-SW-06` /
//! `SARAH-CW-08`).
//!
//! Projection + command entry only. Durable state lives in the record behind
//! supervised `omega-effectd`. Owner-private conversation header is "Sarah".
//! Community room header is "Community" — same dock pane, never a second pane.
//!
//! OMEGA-SW-04: interaction states (pending send, running after claim, ordered
//! tool ladder, answer block + completion, terminal reason, interrupt
//! pending→applied). Transport is SARAH-NR-06. Honest liveness is the tool
//! ladder — never fake token streaming.
//!
//! OMEGA-SW-06: local unread count + attention marker. Proactive tick turns
//! share the transcript projection (no new source). Read state is local only.
//!
//! SARAH-CW-08: switch between owner-private and community rooms in this pane.
//! Membership, work units, and experience rank are community-only projections.
//! Two-room rule: rooms never share membership or history.

use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agent_ui::{AgentPanel, ThreadId};
use anyhow::{Context as _, Result};
use audio::AudioSettings;
use editor::{Editor, SelectionEffects, actions as editor_actions, scroll::Autoscroll};
use gpui::{
    App, AppContext as _, AsyncWindowContext, Context, Entity, EntityId, EventEmitter, FocusHandle,
    Focusable, Global, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, WeakEntity, Window, div, px,
};
use omega_actions::{
    OpenSettingsPage,
    workroom::{
        ApproveSarahVoiceCommand, EndVoice, FocusComposer, InterruptTurn, InterruptVoice,
        OpenPanel, PrepareVoiceAdmission, RejectSarahVoiceCommand, RetryVoice, SendMessage,
        StartVoice, StartVoiceFromComposer, ToggleVoiceMute,
    },
};
use omega_effectd::{
    BindingProjection, BindingState, Issue31GrantProjection, OpenAgentsBinding,
    SharedOmegaEffectdSupervisor, shared_supervisor, try_openagents_binding,
};
use omega_identity::{
    DurableIdentityActionDecision, DurableIdentityActionDescriptor, DurableIdentityActionKind,
    IdentityService, ProofRef, ReceiptRef, ResourceRef,
};
use serde_json::{Value, json};
use settings::Settings;
use sha2::{Digest as _, Sha256};
use text::{Bias, Point};
use ui::{Button, ButtonStyle, Label, LabelSize, prelude::*};
use util::ResultExt as _;
use uuid::Uuid;
use workspace::{
    Save, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::attention::{AttentionMarker, OMEGA_AUTONOMOUS_TICK_ENABLED, empty_room_is_honest};
use crate::community::{
    COMMUNITY_ROOM_HEADER, COMMUNITY_ROOM_SUBTITLE, CommunityRoomProjection, EXPERIENCE_LABEL,
    OWNER_PRIVATE_ROOM_HEADER, RoomKind, V1_NO_PAY_FIRST_RUN_COPY, V1_NO_PAY_ROOM_DESCRIPTION,
};
use crate::full_auto::WorkroomFullAutoRun;
use crate::interaction::{AnswerState, InteractionEvent, InteractionState, TerminalOutcome};
use crate::projections::{
    ActivityProjection, ActivityRow, Freshness, GapState, InterruptIntentState, MessageAck,
    ProjectionMeta, ReceiptRow, ReceiptsProjection, RoomProjection, RunPhase, RunStateProjection,
    TranscriptProjection, TranscriptRow, WorkroomProjection, sources,
};
use crate::voice::{
    AgentThreadPresentation, ApprovedEditorAction, ManagedSarahVoiceClient,
    SARAH_VOICE_WORKSPACE_REF, SarahEditorCommand, SarahVoiceControl, SarahVoiceEvent,
    SarahVoiceState, VoiceCommandRequest, VoiceCommandResult, VoiceEditorTarget, VoiceParticipant,
    VoiceSelectionEffectBinding, VoiceTextPoint, VoiceTranscriptItem, VoiceTranscriptRecoveryGap,
};

const PANEL_KEY: &str = "SarahWorkroomPanel";
const MAX_NOSTR_RECORD_ROWS: usize = 64;
const MAX_VOICE_TRANSCRIPT_CHARS: usize = 16 * 1024;
const MAX_VOICE_SELECTION_SNAPSHOT_BYTES: usize = 64 * 1024;
const VOICE_TRANSCRIPT_PACING_INTERVAL: Duration = Duration::from_millis(75);
const VOICE_TRANSCRIPT_CHARACTERS_PER_SECOND: usize = 16;
// This keeps a new Sarah row from appearing empty without letting a full sentence precede audio.
const VOICE_TRANSCRIPT_INITIAL_LEAD_CHARS: usize = 12;

#[derive(Default)]
struct VoiceTranscriptPresentation {
    sarah_visible_chars: HashMap<(String, String, String), usize>,
    fractional_millichars: usize,
}

impl VoiceTranscriptPresentation {
    fn observe_authoritative_change(
        &mut self,
        item: &VoiceTranscriptItem,
        previous_char_count: usize,
    ) {
        let key = voice_transcript_item_key(item);
        if item.participant == VoiceParticipant::User {
            self.sarah_visible_chars.remove(&key);
            return;
        }

        let character_count = item.text.chars().count();
        let visible_chars = self.sarah_visible_chars.entry(key).or_insert_with(|| {
            previous_char_count
                .saturating_add(VOICE_TRANSCRIPT_INITIAL_LEAD_CHARS)
                .min(character_count)
        });
        *visible_chars = (*visible_chars).min(character_count);
    }

    fn visible_chars(&self, item: &VoiceTranscriptItem) -> usize {
        if item.participant == VoiceParticipant::User {
            return item.text.chars().count();
        }
        self.sarah_visible_chars
            .get(&voice_transcript_item_key(item))
            .copied()
            .unwrap_or_else(|| item.text.chars().count())
    }

    fn advance(&mut self, transcript: &[VoiceTranscriptItem]) -> bool {
        self.fractional_millichars = self.fractional_millichars.saturating_add(
            VOICE_TRANSCRIPT_CHARACTERS_PER_SECOND
                .saturating_mul(VOICE_TRANSCRIPT_PACING_INTERVAL.as_millis() as usize),
        );
        let mut remaining_chars = self.fractional_millichars / 1_000;
        self.fractional_millichars %= 1_000;
        if remaining_chars == 0 {
            return false;
        }

        let mut changed = false;
        for item in transcript {
            if item.participant != VoiceParticipant::Sarah || remaining_chars == 0 {
                continue;
            }
            let key = voice_transcript_item_key(item);
            let Some(visible_chars) = self.sarah_visible_chars.get_mut(&key) else {
                continue;
            };
            let hidden_chars = item.text.chars().count().saturating_sub(*visible_chars);
            let revealed_chars = hidden_chars.min(remaining_chars);
            *visible_chars = visible_chars.saturating_add(revealed_chars);
            remaining_chars -= revealed_chars;
            changed |= revealed_chars > 0;
        }
        changed
    }

    fn has_hidden_sarah_text(&self, transcript: &[VoiceTranscriptItem]) -> bool {
        transcript.iter().any(|item| {
            item.participant == VoiceParticipant::Sarah
                && self.visible_chars(item) < item.text.chars().count()
        })
    }

    fn flush(&mut self) {
        self.sarah_visible_chars.clear();
        self.fractional_millichars = 0;
    }

    fn forget(&mut self, item: &VoiceTranscriptItem) {
        self.sarah_visible_chars
            .remove(&voice_transcript_item_key(item));
    }
}

fn voice_transcript_item_key(item: &VoiceTranscriptItem) -> (String, String, String) {
    (
        item.thread_ref.clone(),
        item.session_ref.clone(),
        item.item_id.clone(),
    )
}

#[derive(Debug)]
struct VoiceCommandRefusal(String);

impl std::fmt::Display for VoiceCommandRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for VoiceCommandRefusal {}

fn refuse_voice_command(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(VoiceCommandRefusal(message.into()))
}

fn voice_text_point(point: Point) -> VoiceTextPoint {
    VoiceTextPoint {
        line: point.row,
        column: point.column,
    }
}

fn voice_document_version(snapshot: &text::BufferSnapshot) -> String {
    let entries = snapshot
        .version()
        .iter()
        .map(|timestamp| format!("{}={}", timestamp.replica_id.as_u16(), timestamp.value))
        .collect::<Vec<_>>()
        .join(",");
    format!("omega-buffer-v1:{entries}")
}

fn validate_voice_selection_effect(
    binding: &VoiceSelectionEffectBinding,
    target: &VoiceEditorTarget,
    current_path: &str,
    current_document_version: &str,
    current_selection_start: VoiceTextPoint,
    current_selection_end: VoiceTextPoint,
    current_selected_text: &str,
    replacement_text: &str,
) -> std::result::Result<(), String> {
    if target.workspace_ref != SARAH_VOICE_WORKSPACE_REF
        || binding.workspace_ref != target.workspace_ref
    {
        return Err("The target workspace no longer matches this Omega workspace.".into());
    }
    if binding.target_path != target.path || binding.target_path != current_path {
        return Err("The active file changed after Sarah proposed the replacement.".into());
    }
    if target.document_version.as_deref() != Some(binding.document_version.as_str())
        || binding.document_version != current_document_version
    {
        return Err("The document changed after Sarah proposed the replacement.".into());
    }
    if binding.selection_start != current_selection_start
        || binding.selection_end != current_selection_end
        || binding.selected_text != current_selected_text
    {
        return Err("The editor selection changed after Sarah proposed the replacement.".into());
    }
    if binding.replacement_text != replacement_text {
        return Err("The replacement text changed after confirmation.".into());
    }
    Ok(())
}

struct CurrentVoiceSelection {
    path: String,
    document_version: String,
    start: VoiceTextPoint,
    end: VoiceTextPoint,
    selected_text: String,
}

#[derive(Default)]
struct SarahVoicePanels(HashMap<EntityId, Entity<SarahWorkroomPanel>>);

impl Global for SarahVoicePanels {}

#[derive(Clone, Debug)]
struct NostrRecordRow {
    event_id: String,
    kind: u16,
    record_kind: String,
    author_fingerprint: String,
    created_at: String,
    source: String,
}

#[derive(Clone, Debug)]
struct NostrRecordsProjection {
    rows: Vec<NostrRecordRow>,
    cursor: Option<String>,
    next_cursor: Option<String>,
    gap: GapState,
    source: String,
    detail: Option<String>,
    truncated: bool,
}

impl NostrRecordsProjection {
    fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            rows: Vec::new(),
            cursor: None,
            next_cursor: None,
            gap: GapState::Unavailable,
            source: "confirmed_nostr".into(),
            detail: Some(detail.into()),
            truncated: false,
        }
    }
}

#[derive(Clone)]
struct SarahCreatedAgentThread {
    thread_id: ThreadId,
    presentation: AgentThreadPresentation,
    status: SharedString,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingSarahVoiceSettlement {
    thread_ref: String,
    session_ref: String,
}

const MAX_CONSECUTIVE_VOICE_RECONNECTS: usize = 3;
const STABLE_VOICE_CONNECTION_DURATION: Duration = Duration::from_secs(30);
const VOICE_RECONNECT_DELAYS: [Duration; MAX_CONSECUTIVE_VOICE_RECONNECTS] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

fn voice_reconnect_delay(attempt: usize) -> Duration {
    VOICE_RECONNECT_DELAYS
        .get(attempt.saturating_sub(1))
        .copied()
        .unwrap_or(Duration::from_secs(4))
}

fn voice_settlement_retry_delay(
    attempts_completed: usize,
    pending_or_retryable_error: bool,
) -> Option<Duration> {
    (pending_or_retryable_error && attempts_completed < MAX_CONSECUTIVE_VOICE_RECONNECTS)
        .then(|| voice_reconnect_delay(attempts_completed.saturating_add(1)))
}

fn voice_settlement_is_recovered(
    state: omega_effectd::SarahVoiceSettlementState,
    final_charge_msat: Option<u64>,
) -> bool {
    matches!(
        state,
        omega_effectd::SarahVoiceSettlementState::Settled
            | omega_effectd::SarahVoiceSettlementState::Released
    ) && final_charge_msat.is_some()
}

fn voice_admission_terms(
    admission: &omega_effectd::SarahVoiceAdmissionProjection,
) -> Option<agent_ui::composer_voice::SarahVoiceAdmissionTerms> {
    use agent_ui::composer_voice::{
        SarahVoiceAdmissionTerms, SarahVoiceCapability, SarahVoiceCapabilityId as UiCapabilityId,
        SarahVoiceConfirmation, SarahVoiceCreditMode as UiCreditMode, SarahVoiceExcludedAuthority,
    };
    use omega_effectd::{SarahVoiceCapabilityId, SarahVoiceCreditMode};

    let capability = |capability| match capability {
        SarahVoiceCapabilityId::ContextRead => UiCapabilityId::ContextRead,
        SarahVoiceCapabilityId::OpenPath => UiCapabilityId::OpenPath,
        SarahVoiceCapabilityId::RevealRange => UiCapabilityId::RevealRange,
        SarahVoiceCapabilityId::ReplaceSelection => UiCapabilityId::ReplaceSelection,
        SarahVoiceCapabilityId::SaveDocument => UiCapabilityId::SaveDocument,
        SarahVoiceCapabilityId::StartAgentThread => UiCapabilityId::StartAgentThread,
    };
    Some(SarahVoiceAdmissionTerms {
        client_profile: admission.client_profile.clone().into(),
        cohort_ref: admission.admission_cohort_ref.clone().into(),
        credit_mode: match admission.credit_mode {
            SarahVoiceCreditMode::Metered => UiCreditMode::Metered,
            SarahVoiceCreditMode::StagingOwnerEntitlement => {
                UiCreditMode::StagingOwnerEntitlement
            }
        },
        rate_msat_per_million_tokens: admission.credit_msat_per_million_tokens?,
        credit_hold_msat: admission.reserved_credit_msat,
        remaining_credit_msat: admission.remaining_credit_msat,
        max_duration_seconds: u32::try_from(admission.max_duration_seconds).ok()?,
        transcript_policy:
            "Final transcripts are stored locally, bounded to the current Sarah thread, and recovered with explicit gap state."
                .into(),
        capabilities: admission
            .commands
            .iter()
            .copied()
            .map(|command| SarahVoiceCapability {
                capability: capability(command),
                confirmation: if admission.confirmation_required.contains(&command) {
                    SarahVoiceConfirmation::ConfirmEachAction
                } else {
                    SarahVoiceConfirmation::NoExtraConfirmation
                },
            })
            .collect(),
        excluded_authorities: vec![
            SarahVoiceExcludedAuthority::DirectShell,
            SarahVoiceExcludedAuthority::DirectGit,
            SarahVoiceExcludedAuthority::Payment,
            SarahVoiceExcludedAuthority::CredentialAccess,
            SarahVoiceExcludedAuthority::DeviceControl,
        ],
    })
}

fn next_voice_reconnect_attempt(
    state: SarahVoiceState,
    retryable_failure: bool,
    previous_attempts: usize,
    connected_for: Duration,
) -> Option<usize> {
    if (!state.is_active() && !(state == SarahVoiceState::Error && retryable_failure))
        || state == SarahVoiceState::Ending
    {
        return None;
    }
    let next_attempt = if connected_for >= STABLE_VOICE_CONNECTION_DURATION {
        1
    } else {
        previous_attempts.saturating_add(1)
    };
    (next_attempt <= MAX_CONSECUTIVE_VOICE_RECONNECTS).then_some(next_attempt)
}

pub struct SarahWorkroomPanel {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    composer: Entity<Editor>,
    projection: WorkroomProjection,
    /// SARAH-CW-08: community room projections (isolated from owner-private).
    community: CommunityRoomProjection,
    /// SARAH-CW-08: which room this single pane is showing.
    active_room: RoomKind,
    /// OMEGA-SW-04 pure interaction projection (pending/send/ladder/terminal).
    interaction: InteractionState,
    status: SharedString,
    supervisor: Option<SharedOmegaEffectdSupervisor>,
    binding: Option<OpenAgentsBinding>,
    binding_projection: BindingProjection,
    binding_busy: bool,
    refreshing: bool,
    sending: bool,
    interrupting: bool,
    full_auto_runs: Vec<WorkroomFullAutoRun>,
    full_auto_detail: Option<String>,
    device_grants: Vec<Issue31GrantProjection>,
    device_grants_detail: Option<String>,
    nostr_records: NostrRecordsProjection,
    grant_busy: Option<String>,
    _refresh: Option<Task<()>>,
    voice_state: SarahVoiceState,
    voice_status: SharedString,
    voice_muted: bool,
    voice_retryable: bool,
    voice_access_required: bool,
    voice_session_id: Option<String>,
    voice_admission_terms: Option<agent_ui::composer_voice::SarahVoiceAdmissionTerms>,
    prepared_voice_admission: Option<omega_effectd::PreparedSarahVoiceAdmission>,
    pending_voice_settlement: Option<PendingSarahVoiceSettlement>,
    settlement_retrying: bool,
    /// `OMEGA-DELTA-0211`. A composer click is waiting on admission terms.
    start_voice_after_admission: bool,
    voice_transcript: Vec<VoiceTranscriptItem>,
    voice_transcript_presentation: VoiceTranscriptPresentation,
    voice_transcript_pacing_task: Option<Task<()>>,
    voice_transcript_recovery: SharedString,
    pending_voice_command: Option<VoiceCommandRequest>,
    created_agent_thread: Option<SarahCreatedAgentThread>,
    voice_controls: Option<async_channel::Sender<SarahVoiceControl>>,
    voice_task: Option<Task<()>>,
    public_demo: bool,
}

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, cx| {
        let workspace_id = cx.entity_id();
        cx.on_release(move |_, cx| {
            cx.default_global::<SarahVoicePanels>()
                .0
                .remove(&workspace_id);
            agent_ui::composer_voice::remove_composer_voice_status(workspace_id, cx);
            agent_ui::composer_voice::remove_composer_voice_notice(workspace_id, cx);
            agent_ui::composer_voice::remove_sarah_voice_admission(workspace_id, cx);
        })
        .detach();
        workspace
            .register_action(|workspace, _: &OpenPanel, window, cx| {
                workspace.focus_panel::<SarahWorkroomPanel>(window, cx);
                if let Some(panel) = workspace.panel::<SarahWorkroomPanel>(cx) {
                    // Local mark-read when the owner opens the room (OMEGA-SW-06).
                    panel.update(cx, |panel, cx| panel.mark_room_read(cx));
                }
            })
            .register_action(|workspace, _: &FocusComposer, window, cx| {
                if let Some(panel) = workspace.panel::<SarahWorkroomPanel>(cx) {
                    panel.update(cx, |panel, cx| panel.focus_composer(window, cx));
                }
                workspace.focus_panel::<SarahWorkroomPanel>(window, cx);
            })
            .register_action(|workspace, _: &SendMessage, window, cx| {
                if let Some(panel) = workspace.panel::<SarahWorkroomPanel>(cx) {
                    panel.update(cx, |panel, cx| panel.send_message(window, cx));
                }
                workspace.focus_panel::<SarahWorkroomPanel>(window, cx);
            })
            .register_action(|workspace, _: &InterruptTurn, window, cx| {
                if let Some(panel) = workspace.panel::<SarahWorkroomPanel>(cx) {
                    panel.update(cx, |panel, cx| panel.interrupt_turn(cx));
                }
                workspace.focus_panel::<SarahWorkroomPanel>(window, cx);
            })
            .register_action(|_workspace, _: &StartVoice, window, cx| {
                if let Some(panel) = voice_panel(cx.entity_id(), cx) {
                    panel.update(cx, |panel, cx| panel.start_voice(window, cx));
                }
            })
            .register_action(|_workspace, _: &StartVoiceFromComposer, window, cx| {
                if let Some(panel) = voice_panel(cx.entity_id(), cx) {
                    panel.update(cx, |panel, cx| panel.start_voice_from_composer(window, cx));
                }
            })
            .register_action(|_workspace, _: &PrepareVoiceAdmission, window, cx| {
                if let Some(panel) = voice_panel(cx.entity_id(), cx) {
                    panel.update(cx, |panel, cx| panel.prepare_voice_admission(window, cx));
                }
            })
            .register_action(|_workspace, _: &ToggleVoiceMute, _window, cx| {
                if let Some(panel) = voice_panel(cx.entity_id(), cx) {
                    panel.update(cx, |panel, cx| panel.toggle_voice_mute(cx));
                }
            })
            .register_action(|_workspace, _: &InterruptVoice, _window, cx| {
                if let Some(panel) = voice_panel(cx.entity_id(), cx) {
                    panel.update(cx, |panel, cx| panel.interrupt_voice(cx));
                }
            })
            .register_action(|_workspace, _: &ApproveSarahVoiceCommand, window, cx| {
                if let Some(panel) = voice_panel(cx.entity_id(), cx) {
                    panel.update(cx, |panel, cx| panel.approve_voice_command(window, cx));
                }
            })
            .register_action(|_workspace, _: &RejectSarahVoiceCommand, _window, cx| {
                if let Some(panel) = voice_panel(cx.entity_id(), cx) {
                    panel.update(cx, |panel, cx| panel.reject_voice_command(cx));
                }
            })
            .register_action(|_workspace, _: &EndVoice, _window, cx| {
                if let Some(panel) = voice_panel(cx.entity_id(), cx) {
                    panel.update(cx, |panel, cx| panel.end_voice(cx));
                }
            })
            .register_action(|_workspace, _: &RetryVoice, window, cx| {
                if let Some(panel) = voice_panel(cx.entity_id(), cx) {
                    panel.update(cx, |panel, cx| panel.retry_voice(window, cx));
                }
            });
    })
    .detach();
}

fn voice_panel(workspace_id: EntityId, cx: &mut App) -> Option<Entity<SarahWorkroomPanel>> {
    cx.default_global::<SarahVoicePanels>()
        .0
        .get(&workspace_id)
        .cloned()
}

impl SarahWorkroomPanel {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            let workspace_for_panel = workspace.clone();
            workspace.update_in(cx, |_workspace, window, cx| {
                let workspace_id = cx.entity_id();
                let panel = cx.new(|cx| Self::new(workspace_for_panel, window, cx));
                cx.default_global::<SarahVoicePanels>()
                    .0
                    .insert(workspace_id, panel.clone());
                let status = panel.read(cx).composer_voice_status();
                agent_ui::composer_voice::set_composer_voice_status(workspace_id, status, cx);
                cx.observe(&panel, move |_workspace, panel, cx| {
                    let status = panel.read(cx).composer_voice_status();
                    agent_ui::composer_voice::set_composer_voice_status(workspace_id, status, cx);
                })
                .detach();
                Ok(panel)
            })?
        })
    }

    fn composer_voice_status(&self) -> agent_ui::composer_voice::ComposerVoiceStatus {
        use agent_ui::composer_voice::{ComposerVoicePhase, ComposerVoiceStatus};

        let phase = match self.voice_state {
            SarahVoiceState::Idle => ComposerVoicePhase::Idle,
            SarahVoiceState::Authenticating => ComposerVoicePhase::Authenticating,
            SarahVoiceState::RequestingMicrophone => ComposerVoicePhase::RequestingMicrophone,
            SarahVoiceState::Connecting => ComposerVoicePhase::Connecting,
            SarahVoiceState::Listening => ComposerVoicePhase::Listening,
            SarahVoiceState::UserSpeaking => ComposerVoicePhase::UserSpeaking,
            SarahVoiceState::SarahSpeaking => ComposerVoicePhase::SarahSpeaking,
            SarahVoiceState::Reconnecting => ComposerVoicePhase::Reconnecting,
            SarahVoiceState::Ending => ComposerVoicePhase::Ending,
            SarahVoiceState::Error if self.voice_access_required => {
                ComposerVoicePhase::AccessRequired
            }
            SarahVoiceState::Error => ComposerVoicePhase::Error,
        };
        ComposerVoiceStatus {
            phase,
            detail: self.voice_status.clone(),
            muted: self.voice_muted,
            retryable: self.voice_retryable,
        }
    }

    fn publish_voice_admission(
        &self,
        projection: agent_ui::composer_voice::SarahVoiceAdmissionProjection,
        cx: &mut App,
    ) {
        if let Some(workspace) = self.workspace.upgrade() {
            agent_ui::composer_voice::set_sarah_voice_admission(
                workspace.entity_id(),
                projection,
                cx,
            );
        }
    }

    fn voice_session_artifacts(&self) -> agent_ui::composer_voice::SarahVoiceSessionArtifacts {
        use agent_ui::composer_voice::{
            SarahAgentThreadPresentation, SarahVoiceAgentThreadReceipt, SarahVoiceParticipant,
            SarahVoicePendingConfirmation, SarahVoiceSelectionEffectPreview,
            SarahVoiceSessionArtifacts, SarahVoiceTranscriptRow,
        };

        SarahVoiceSessionArtifacts {
            transcript: self
                .voice_transcript
                .iter()
                .map(|item| SarahVoiceTranscriptRow {
                    thread_ref: item.thread_ref.clone().into(),
                    session_ref: item.session_ref.clone().into(),
                    item_id: item.item_id.clone().into(),
                    participant: match item.participant {
                        VoiceParticipant::User => SarahVoiceParticipant::User,
                        VoiceParticipant::Sarah => SarahVoiceParticipant::Sarah,
                    },
                    text: item.text.clone().into(),
                    complete: item.complete,
                })
                .collect(),
            pending_confirmation: self.pending_voice_command.as_ref().map(|request| {
                SarahVoicePendingConfirmation {
                    request_id: request.request_id.clone().into(),
                    copy: request.command.confirmation_copy().into(),
                    detail: request.command.confirmation_detail().map(Into::into),
                    selection_effect: request.effect_binding.as_ref().map(|binding| {
                        SarahVoiceSelectionEffectPreview {
                            workspace_ref: binding.workspace_ref.clone().into(),
                            document_version: binding.document_version.clone().into(),
                            target_path: binding.target_path.clone().into(),
                            selection_start_line: binding.selection_start.line,
                            selection_start_column: binding.selection_start.column,
                            selection_end_line: binding.selection_end.line,
                            selection_end_column: binding.selection_end.column,
                            selected_text: binding.selected_text.clone().into(),
                            replacement_text: binding.replacement_text.clone().into(),
                        }
                    }),
                }
            }),
            created_agent_thread: self.created_agent_thread.as_ref().map(|created| {
                SarahVoiceAgentThreadReceipt {
                    thread_id: created.thread_id.to_key_string().into(),
                    presentation: match created.presentation {
                        AgentThreadPresentation::Foreground => {
                            SarahAgentThreadPresentation::Foreground
                        }
                        AgentThreadPresentation::Background => {
                            SarahAgentThreadPresentation::Background
                        }
                    },
                    status: created.status.clone(),
                }
            }),
        }
    }

    fn publish_active_voice_artifacts(&self, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let projection = agent_ui::composer_voice::sarah_voice_admission(workspace.entity_id(), cx)
            .read(cx)
            .clone();
        if let agent_ui::composer_voice::SarahVoiceAdmissionProjection::Active {
            terms,
            session_id,
            ..
        } = projection
        {
            self.publish_voice_admission(
                agent_ui::composer_voice::SarahVoiceAdmissionProjection::Active {
                    terms,
                    session_id,
                    artifacts: self.voice_session_artifacts(),
                },
                cx,
            );
        }
    }

    fn new(workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let public_demo = crate::public_demo_mode();
        let composer = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_placeholder_text("Message Sarah (text only).", window, cx);
            editor
        });
        let binding = if public_demo {
            None
        } else {
            try_openagents_binding(cx)
        };
        let binding_projection = binding
            .as_ref()
            .map(|binding| binding.load_projection())
            .unwrap_or_else(BindingProjection::unbound);
        let mut panel = Self {
            workspace,
            focus_handle: cx.focus_handle(),
            composer,
            projection: if public_demo {
                WorkroomProjection::public_demo()
            } else {
                WorkroomProjection::honest_unsubscribed()
            },
            community: CommunityRoomProjection::honest_unsubscribed(),
            active_room: RoomKind::OwnerPrivate,
            interaction: InteractionState::new(),
            status: if public_demo {
                "Public demo · fictional data · isolated profile".into()
            } else {
                binding_projection.state.status_line().into()
            },
            supervisor: None,
            binding,
            binding_projection,
            binding_busy: false,
            refreshing: false,
            sending: false,
            interrupting: false,
            full_auto_runs: Vec::new(),
            full_auto_detail: Some("Full Auto records have not been refreshed yet.".into()),
            device_grants: Vec::new(),
            device_grants_detail: Some("Device grants have not been refreshed yet.".into()),
            nostr_records: NostrRecordsProjection::unavailable(
                "Confirmed Nostr record references have not been refreshed yet.",
            ),
            grant_busy: None,
            _refresh: None,
            voice_state: SarahVoiceState::Idle,
            voice_status: "Managed voice is ready to start.".into(),
            voice_muted: false,
            voice_retryable: false,
            voice_access_required: false,
            voice_session_id: None,
            voice_admission_terms: None,
            prepared_voice_admission: None,
            pending_voice_settlement: None,
            settlement_retrying: false,
            start_voice_after_admission: false,
            voice_transcript: Vec::new(),
            voice_transcript_presentation: VoiceTranscriptPresentation::default(),
            voice_transcript_pacing_task: None,
            voice_transcript_recovery: "No prior transcript rows were recovered.".into(),
            pending_voice_command: None,
            created_agent_thread: None,
            voice_controls: None,
            voice_task: None,
            public_demo,
        };
        if !public_demo {
            panel.ensure_supervisor(cx);
            panel.refresh_from_effectd(cx);
            panel.schedule_refresh(cx);
        }
        panel
    }

    fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        self._refresh = Some(cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_secs(3)).await;
                if this
                    .update(cx, |panel, cx| panel.refresh_from_effectd(cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn bind_openagents_account(&mut self, cx: &mut Context<Self>) {
        if self.binding_busy {
            return;
        }
        let Some(binding) = self.binding.clone() else {
            self.status = "OpenAgents binding service unavailable.".into();
            cx.notify();
            return;
        };
        // Relation requires the active Omega Nostr public key from isolated custody.
        let omega_pubkey = match IdentityService::system(*app_identity::CHANNEL)
            .inspect()
            .ok()
            .and_then(|custody| custody.identity)
            .map(|identity| identity.public_key_hex().as_str().to_string())
        {
            Some(pubkey) if !pubkey.is_empty() => pubkey,
            _ => {
                self.status =
                    "Omega identity is not ready. Create or open an identity before binding."
                        .into();
                cx.notify();
                return;
            }
        };
        let payload_digest = format!(
            "{:x}",
            Sha256::digest(format!("openagents-account-link\0{omega_pubkey}"))
        );
        let intent_ref =
            match ReceiptRef::new(format!("omega-openagents-link-{}", &payload_digest[..32])) {
                Ok(intent_ref) => intent_ref,
                Err(error) => {
                    self.status = format!("OpenAgents link intent is invalid: {error}").into();
                    cx.notify();
                    return;
                }
            };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let activation = IdentityService::system(*app_identity::CHANNEL)
            .authorize_or_hold_identity_action(DurableIdentityActionDescriptor {
                authorization_ref: match ProofRef::new(format!(
                    "activation-{}",
                    intent_ref.as_str()
                )) {
                    Ok(authorization_ref) => authorization_ref,
                    Err(error) => {
                        self.status =
                            format!("OpenAgents link authorization is invalid: {error}").into();
                        cx.notify();
                        return;
                    }
                },
                intent_ref,
                kind: DurableIdentityActionKind::HostedAccountLink,
                destination_ref: match ResourceRef::new("openagents-account-service") {
                    Ok(destination_ref) => destination_ref,
                    Err(error) => {
                        self.status =
                            format!("OpenAgents link destination is invalid: {error}").into();
                        cx.notify();
                        return;
                    }
                },
                payload_digest,
                expires_at: now.saturating_add(600),
            });
        let authorization = match activation {
            Ok(DurableIdentityActionDecision::Authorized(authorization)) => authorization,
            Ok(DurableIdentityActionDecision::ActivationRequired { account, .. }) => {
                self.status = format!(
                    "Set up identity {} before binding an OpenAgents account.",
                    account.fingerprint_display()
                )
                .into();
                cx.notify();
                return;
            }
            Err(error) => {
                self.status = format!("Omega identity setup is unavailable: {error}").into();
                cx.notify();
                return;
            }
        };
        self.binding_busy = true;
        self.status = "Binding OpenAgents account securely…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let projection = binding
                .bind_authorized(&omega_pubkey, &authorization, cx)
                .await;
            this.update(cx, |panel, cx| {
                panel.binding_busy = false;
                panel.binding_projection = projection.clone();
                // Visible states only: unbound | bound | refused.
                // Refused must show the owner-scope message, never a network fault.
                panel.status = match projection.state {
                    BindingState::Unbound => "OpenAgents account unbound.".into(),
                    BindingState::Bound => format!(
                        "Bound OpenAgents account {} to Omega identity (metering attribution).",
                        projection
                            .openagents_account_id
                            .as_deref()
                            .unwrap_or("unknown")
                    )
                    .into(),
                    BindingState::Refused => projection
                        .gate_message
                        .clone()
                        .unwrap_or_else(|| BindingState::Refused.status_line().to_string())
                        .into(),
                };
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    fn clear_openagents_binding(&mut self, cx: &mut Context<Self>) {
        if self.binding_busy {
            return;
        }
        let Some(binding) = self.binding.clone() else {
            return;
        };
        self.binding_busy = true;
        cx.spawn(async move |this, cx| {
            let projection = binding.clear(cx).await;
            this.update(cx, |panel, cx| {
                panel.binding_busy = false;
                panel.binding_projection = projection;
                panel.status = "OpenAgents account unbound.".into();
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.focus_handle(cx).focus(window, cx);
        // Opening / focusing the room is a local mark-read (MVP).
        self.mark_room_read(cx);
        cx.notify();
    }

    /// Local mark-read only (OMEGA-SW-06). Does not publish NIP-RS / kind 30078.
    fn mark_room_read(&mut self, cx: &mut Context<Self>) {
        self.projection.mark_room_read();
        if self.projection.attention.unread_count == 0 {
            self.status = "Room marked read (local only).".into();
        }
        cx.notify();
    }

    fn ensure_supervisor(&mut self, cx: &mut Context<Self>) {
        if self.supervisor.is_some() {
            return;
        }
        match shared_supervisor(cx) {
            Ok(supervisor) => {
                self.supervisor = Some(supervisor);
                self.status = "Connected to omega-effectd supervisor.".into();
            }
            Err(error) => {
                let detail = format!("omega-effectd unavailable ({error}).");
                self.status = detail.clone().into();
                self.projection.mark_effectd_unavailable(detail);
            }
        }
        cx.notify();
    }

    fn refresh_from_effectd(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            return;
        };
        self.refreshing = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let bootstrap = {
                let mut guard = supervisor.lock().await;
                match guard.ensure_started().await {
                    Ok(()) => match guard.sarah_bootstrap().await {
                        Ok(value) => Ok(value),
                        Err(error) => Err(error.to_string()),
                    },
                    Err(error) => Err(error.to_string()),
                }
            };

            let snapshot = match &bootstrap {
                Ok(_) => {
                    let mut guard = supervisor.lock().await;
                    match guard
                        .sarah_room_snapshot(Some(json!({
                            "transcriptLimit": 50,
                            "activityLimit": 50,
                            "nostrLimit": 50,
                        })))
                        .await
                    {
                        Ok(value) => Ok(value),
                        Err(error) => Err(error.to_string()),
                    }
                }
                Err(error) => Err(error.clone()),
            };

            let device_grants = match &bootstrap {
                Ok(_) => {
                    let mut guard = supervisor.lock().await;
                    match guard.sarah_device_grants().await {
                        Ok(value) => value
                            .get("grants")
                            .cloned()
                            .ok_or_else(|| "device grant response omitted grants".to_string())
                            .and_then(|grants| {
                                serde_json::from_value::<Vec<Issue31GrantProjection>>(grants)
                                    .map_err(|error| error.to_string())
                            }),
                        Err(error) => Err(error.to_string()),
                    }
                }
                Err(error) => Err(error.clone()),
            };

            let full_auto = {
                let mut guard = supervisor.lock().await;
                match guard.list_runs().await {
                    Ok(rows) => {
                        let mut details = Vec::new();
                        let mut detail_error = None;
                        for row in rows {
                            match guard.get_run(&row.run_ref).await {
                                Ok(value) => details.push(value),
                                Err(error) => {
                                    detail_error = Some(error.to_string());
                                    break;
                                }
                            }
                        }
                        if let Some(error) = detail_error {
                            Err(error)
                        } else {
                            Ok(details)
                        }
                    }
                    Err(error) => Err(error.to_string()),
                }
            };

            this.update(cx, |panel, cx| {
                panel.refreshing = false;
                match device_grants {
                    Ok(grants) => {
                        panel.device_grants = grants;
                        panel.device_grants_detail = if panel.device_grants.is_empty() {
                            Some("No paired device grants.".into())
                        } else {
                            None
                        };
                    }
                    Err(error) => {
                        panel.device_grants.clear();
                        panel.device_grants_detail =
                            Some(format!("Device grants unavailable: {error}"));
                    }
                }
                match full_auto {
                    Ok(values) => {
                        panel.full_auto_runs = values
                            .iter()
                            .filter_map(|value| WorkroomFullAutoRun::from_value(value, chrono::Utc::now()))
                            .collect();
                        panel.full_auto_detail = if panel.full_auto_runs.is_empty() {
                            Some("No Full Auto run records.".into())
                        } else {
                            None
                        };
                    }
                    Err(error) => {
                        panel.full_auto_runs.clear();
                        panel.full_auto_detail = Some(format!("Full Auto records unavailable: {error}"));
                    }
                }
                match (bootstrap, snapshot) {
                    (Ok(boot), Ok(snap)) => {
                        panel.apply_bootstrap(&boot);
                        panel.apply_snapshot(&snap);
                        panel.status = "Room projection refreshed from omega-effectd.".into();
                        panel.sync_interaction_status();
                    }
                    (Ok(boot), Err(snap_err)) => {
                        panel.apply_bootstrap(&boot);
                        panel.projection.transcript.meta =
                            ProjectionMeta::unavailable(sources::TRANSCRIPT, &snap_err);
                        panel.projection.activity.meta =
                            ProjectionMeta::unavailable(sources::ACTIVITY, &snap_err);
                        panel.projection.receipts.meta =
                            ProjectionMeta::unavailable(sources::RECEIPTS, &snap_err);
                        panel.projection.run_state.meta =
                            ProjectionMeta::unavailable(sources::RUN_STATE, &snap_err);
                        panel.projection.run_state.reason = Some(snap_err.clone());
                        panel.nostr_records = NostrRecordsProjection::unavailable(format!(
                            "Confirmed Nostr record references unavailable: {snap_err}"
                        ));
                        panel.status = format!("Bootstrap ok; room snapshot unavailable: {snap_err}")
                            .into();
                    }
                    (Err(error), _) => {
                        // Methods may not exist until SARAH-NR-06. Stay honest.
                        panel.projection.mark_effectd_unavailable(error.clone());
                        panel.nostr_records = NostrRecordsProjection::unavailable(format!(
                            "Confirmed Nostr record source unavailable: {error}"
                        ));
                        panel.status = format!(
                            "Sarah record methods unavailable ({error}). Sources stay labeled missing."
                        )
                        .into();
                    }
                }
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    fn apply_bootstrap(&mut self, value: &Value) {
        // SARAH-NR-06 bootstrap is flat; SW-03 also accepted nested room/principal.
        let room = value.get("room").or_else(|| value.get("principal"));
        let root = Some(value);
        let principal_ref = string_field(room, &["principalRef", "principal_ref", "ref"])
            .or_else(|| string_field(root, &["principalRef", "principal_ref"]));
        let display_name = string_field(room, &["displayName", "display_name", "name"])
            .or_else(|| string_field(root, &["displayName", "display_name"]))
            .or_else(|| Some("Sarah".into()));
        let role = string_field(room, &["role"])
            .or_else(|| string_field(root, &["role"]))
            .or_else(|| Some("principal.sarah".into()));
        let thread_ref = string_field(
            value.get("thread").or(room),
            &["threadRef", "thread_ref", "ref", "conversation"],
        )
        .or_else(|| {
            string_field(
                root,
                &[
                    "conversationRef",
                    "conversation_ref",
                    "legacyThreadRef",
                    "legacy_thread_ref",
                    "threadRef",
                ],
            )
        });
        let authority_profile = string_field(
            value.get("authority").or(room),
            &["profile", "authorityProfile", "authority_profile"],
        )
        .or_else(|| {
            string_field(
                root,
                &[
                    "authorityProfileRef",
                    "authority_profile_ref",
                    "authorityProfile",
                ],
            )
        });
        let authority_revision = string_field(
            value.get("authority").or(room),
            &["revision", "authorityRevision", "authority_revision"],
        )
        .or_else(|| {
            value
                .get("authorityProfileRevision")
                .or_else(|| value.get("authority_profile_revision"))
                .map(|v| match v {
                    Value::Number(n) => n.to_string(),
                    Value::String(s) => s.clone(),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty())
        });

        self.projection.room = RoomProjection {
            meta: ProjectionMeta::fresh(sources::ROOM),
            principal_ref,
            display_name,
            role,
            thread_ref,
            authority_profile,
            authority_revision,
            detail: None,
        };
        self.projection.recompute_attention();
    }

    fn apply_snapshot(&mut self, value: &Value) {
        // Preserve local pending rows across refresh until record confirms them.
        let local_pending: Vec<TranscriptRow> = self
            .projection
            .transcript
            .rows
            .iter()
            .filter(|row| row.ack == MessageAck::Pending)
            .cloned()
            .collect();

        // Transcript — ordinary turns only (including proactive tick turns).
        // omega-effectd uses `entries`; older fixtures may use items/messages.
        let mut transcript = TranscriptProjection {
            meta: ProjectionMeta::fresh(sources::TRANSCRIPT),
            rows: Vec::new(),
            cursor: string_field(value.get("transcript"), &["cursor"]),
            truncated: false,
        };
        if let Some(items) = value
            .get("transcript")
            .and_then(|t| {
                t.get("entries")
                    .or_else(|| t.get("items"))
                    .or_else(|| t.get("messages"))
            })
            .and_then(|v| v.as_array())
        {
            for item in items {
                // Proactive tick turns and Q&A answers share this path.
                // Only an explicit pending ack/status stays non-confirmed.
                let ack = match item
                    .get("ack")
                    .or_else(|| item.get("status"))
                    .or_else(|| item.get("state"))
                    .and_then(|v| v.as_str())
                {
                    Some("pending") => MessageAck::Pending,
                    _ => MessageAck::Confirmed,
                };
                transcript.push_bounded(TranscriptRow {
                    message_ref: string_field(
                        Some(item),
                        &["messageRef", "eventId", "event_id", "id", "ref", "cursor"],
                    )
                    .unwrap_or_else(|| "unknown".into()),
                    role: string_field(Some(item), &["role"]).unwrap_or_else(|| "unknown".into()),
                    text: string_field(Some(item), &["text", "content"]).unwrap_or_default(),
                    ack,
                });
            }
        } else if value.get("transcript").is_none() {
            transcript.meta = ProjectionMeta::missing(sources::TRANSCRIPT);
        }
        // Re-attach unconfirmed local sends so refresh never drops optimistic rows.
        for pending in local_pending {
            if !transcript
                .rows
                .iter()
                .any(|row| row.message_ref == pending.message_ref)
            {
                transcript.push_bounded(pending);
            }
        }
        if let Some(gap) = value
            .get("transcript")
            .and_then(|t| t.get("gap").or_else(|| t.get("gapState")))
            .and_then(|v| v.as_str())
        {
            if gap != "none" {
                transcript.meta.gap = GapState::Gap;
                transcript.meta.freshness = Freshness::Stale;
            }
        }
        self.projection.transcript = transcript;

        // Activity — NR-06 uses `entries` with `entry` kind field.
        let mut activity = ActivityProjection {
            meta: ProjectionMeta::fresh(sources::ACTIVITY),
            rows: Vec::new(),
            cursor: string_field(value.get("activity"), &["cursor"]),
            truncated: false,
        };
        if let Some(items) = value
            .get("activity")
            .and_then(|a| {
                a.get("entries")
                    .or_else(|| a.get("items"))
                    .or_else(|| a.get("events"))
            })
            .and_then(|v| v.as_array())
        {
            for item in items {
                let kind = string_field(Some(item), &["entry", "kind", "type"])
                    .unwrap_or_else(|| "event".into());
                let event_ref = string_field(
                    Some(item),
                    &["eventRef", "eventId", "event_id", "id", "ref"],
                )
                .unwrap_or_else(|| "unknown".into());
                let summary =
                    string_field(Some(item), &["summary", "text"]).unwrap_or_else(|| kind.clone());
                let turn_ref = string_field(Some(item), &["turnRef", "turn_ref", "turn"]);
                activity.push_bounded(ActivityRow {
                    event_ref: event_ref.clone(),
                    kind: kind.clone(),
                    summary: summary.clone(),
                    turn_ref: turn_ref.clone(),
                });
                // Drive interaction ladder from snapshot activity (ordered).
                if let Some(event) = InteractionEvent::from_runtime_kind(
                    &kind,
                    event_ref,
                    turn_ref,
                    summary,
                    string_field(Some(item), &["toolRef", "tool_ref"]),
                    string_field(Some(item), &["reason"]),
                ) {
                    let _ = self.interaction.apply_event(event);
                }
            }
        } else if value.get("activity").is_none() {
            activity.meta = ProjectionMeta::missing(sources::ACTIVITY);
        }
        // Prefer interaction ladder order when it has more recent steps.
        if !self.interaction.tool_ladder.is_empty() {
            let mut ladder_activity = ActivityProjection {
                meta: ProjectionMeta::fresh(sources::ACTIVITY),
                rows: Vec::new(),
                cursor: activity.cursor.clone(),
                truncated: false,
            };
            for row in self.interaction.activity_rows() {
                ladder_activity.push_bounded(row);
            }
            activity = ladder_activity;
        }
        self.projection.activity = activity;

        // Receipts (stub refs only; deep inspector is OMEGA-SW-05).
        let mut receipts = ReceiptsProjection {
            meta: ProjectionMeta::fresh(sources::RECEIPTS),
            rows: Vec::new(),
            detail: Some("Receipt refs only. Deep inspector is OMEGA-SW-05.".into()),
        };
        if let Some(items) = value
            .get("receipts")
            .and_then(|r| r.get("items").or_else(|| r.as_array().map(|_| r)))
            .and_then(|v| {
                if v.is_array() {
                    v.as_array()
                } else {
                    v.get("items").and_then(|i| i.as_array())
                }
            })
        {
            for item in items {
                receipts.push_bounded(ReceiptRow {
                    receipt_ref: string_field(
                        Some(item),
                        &["receiptRef", "authorityReceiptRef", "ref", "id"],
                    )
                    .unwrap_or_else(|| "unknown".into()),
                    allowed: item.get("allowed").and_then(|v| v.as_bool()),
                    decision_ref: string_field(Some(item), &["decisionRef", "decision_ref"]),
                    tool_ref: string_field(Some(item), &["toolRef", "tool_ref"]),
                });
            }
        } else if value.get("receipts").is_none() {
            receipts.meta = ProjectionMeta::missing(sources::RECEIPTS);
            receipts.detail = Some("No receipt page in snapshot. Source labeled missing.".into());
        }
        self.projection.receipts = receipts;

        self.nostr_records = parse_nostr_records_projection(value.get("nostrRecords"));

        // Run state — NR-06 uses `state`; legacy used `phase`.
        let run = value.get("runState").or_else(|| value.get("run_state"));
        let phase_str = string_field(run, &["phase", "state", "status"]);
        let phase = phase_str
            .as_deref()
            .map(parse_run_phase)
            .unwrap_or(RunPhase::Unknown);
        let reason = string_field(run, &["reason", "finishReason", "finish_reason"]);
        let turn_ref = string_field(run, &["turnRef", "turn_ref"]);

        self.interaction.apply_snapshot_run(phase, turn_ref, reason);

        let mut run_state = self.interaction.run.clone();
        if run.is_none() {
            run_state.meta = ProjectionMeta::missing(sources::RUN_STATE);
            if run_state.reason.is_none() {
                run_state.reason = Some("Run state missing from snapshot.".into());
            }
        }
        self.projection.run_state = run_state;
        self.projection.connection_detail = Some("Snapshot applied from omega-effectd.".into());
        // OMEGA-SW-06: recompute local unread + attention after transcript page.
        // Never invent proactive rows when the autonomous tick is off.
        debug_assert!(
            empty_room_is_honest(&self.projection.transcript, OMEGA_AUTONOMOUS_TICK_ENABLED),
            "empty room must stay honest when autonomous tick is off"
        );
        self.projection.recompute_attention();
        self.sync_interaction_status();
    }

    /// Apply one ordered room/runtime event into interaction + projection.
    fn apply_interaction_event(&mut self, event: InteractionEvent) {
        let rows = self.interaction.apply_event(event);
        for row in rows {
            self.upsert_transcript_row(row);
        }
        // Refresh activity from ordered tool ladder.
        if !self.interaction.tool_ladder.is_empty() {
            let mut activity = ActivityProjection {
                meta: ProjectionMeta::fresh(sources::ACTIVITY),
                rows: Vec::new(),
                cursor: self.projection.activity.cursor.clone(),
                truncated: false,
            };
            for row in self.interaction.activity_rows() {
                activity.push_bounded(row);
            }
            self.projection.activity = activity;
        }
        self.projection.run_state = self.interaction.run.clone();
        self.projection.recompute_attention();
        self.sync_interaction_status();
    }

    fn upsert_transcript_row(&mut self, row: TranscriptRow) {
        if let Some(existing) = self
            .projection
            .transcript
            .rows
            .iter_mut()
            .find(|r| r.message_ref == row.message_ref)
        {
            *existing = row;
            self.projection.recompute_attention();
            return;
        }
        // Confirm may replace a local pending row by text/local ref.
        if row.ack == MessageAck::Confirmed {
            if let Some(idx) = self.projection.transcript.rows.iter().position(|r| {
                r.ack == MessageAck::Pending && r.role == row.role && r.text == row.text
            }) {
                self.projection.transcript.rows[idx] = row;
                self.projection.recompute_attention();
                return;
            }
        }
        self.projection.transcript.push_bounded(row);
        if self.projection.transcript.meta.gap == GapState::Unavailable {
            self.projection.transcript.meta = ProjectionMeta::fresh(sources::TRANSCRIPT);
        }
        self.projection.recompute_attention();
    }

    fn sync_interaction_status(&mut self) {
        // Prefer interaction status when a turn is active or pending; keep
        // attention mark-read messages otherwise.
        if self.interaction.pending_send_count() > 0
            || self.interaction.run.phase == RunPhase::Running
            || self.interaction.run.phase == RunPhase::Queued
            || self.interaction.terminal.is_terminal()
            || self.interaction.run.interrupt_intent != InterruptIntentState::None
            || !self.interaction.tool_ladder.is_empty()
            || self.interaction.answer != AnswerState::None
        {
            let mut status = self.interaction.status_line();
            if self.interaction.uses_honest_liveness() {
                status.push_str(" · liveness=tool_ladder");
            }
            self.status = status.into();
        }
    }

    /// OMEGA-SW-04: send composer text. Local pending until record confirms.
    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending {
            return;
        }
        let text = self.composer.read(cx).text(cx).trim().to_string();
        if text.is_empty() {
            self.status = "Message text is required.".into();
            cx.notify();
            return;
        }

        let (local_ref, pending_row) = self.interaction.begin_send(text.clone());
        self.upsert_transcript_row(pending_row);
        self.projection.transcript.meta = ProjectionMeta::pending(sources::TRANSCRIPT);
        self.status = format!("Pending local send {local_ref} until record confirms.").into();

        self.composer.update(cx, |editor, cx| {
            editor.clear(window, cx);
        });

        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            self.interaction.fail_send(&local_ref);
            self.projection
                .transcript
                .rows
                .retain(|row| row.message_ref != local_ref);
            self.projection.mark_effectd_unavailable("no supervisor");
            self.status = "omega-effectd unavailable; local pending send dropped.".into();
            cx.notify();
            return;
        };

        self.sending = true;
        let idempotency_ref = format!("idempotency.workroom.send.{}", Uuid::new_v4());
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = {
                let mut guard = supervisor.lock().await;
                match guard.ensure_started().await {
                    Ok(()) => guard.sarah_send_message(&text, &idempotency_ref).await,
                    Err(error) => Err(omega_effectd::SupervisorError::Anyhow(error)),
                }
            };
            this.update(cx, |panel, cx| {
                panel.sending = false;
                match result {
                    Ok(value) => {
                        let message_ref = string_field(
                            Some(&value),
                            &["messageRef", "message_ref", "eventId", "event_id"],
                        )
                        .unwrap_or_else(|| local_ref.clone());
                        let turn_ref = string_field(Some(&value), &["turnRef", "turn_ref"]);
                        let status = string_field(Some(&value), &["status"])
                            .unwrap_or_else(|| "accepted".into());
                        if let Some(confirmed) = panel.interaction.confirm_send(
                            &local_ref,
                            message_ref.clone(),
                            turn_ref,
                        ) {
                            panel.upsert_transcript_row(confirmed);
                        } else {
                            panel.upsert_transcript_row(TranscriptRow {
                                message_ref: message_ref.clone(),
                                role: "owner".into(),
                                text: text.clone(),
                                ack: MessageAck::Confirmed,
                            });
                        }
                        // Accepted on record — turn runs only after claim event.
                        panel.projection.run_state = panel.interaction.run.clone();
                        panel.projection.transcript.meta =
                            ProjectionMeta::fresh(sources::TRANSCRIPT);
                        panel.status = format!(
                            "Message confirmed ({status}) ref={message_ref}; turn claim pending."
                        )
                        .into();
                        panel.sync_interaction_status();
                    }
                    Err(error) => {
                        panel.interaction.fail_send(&local_ref);
                        panel
                            .projection
                            .transcript
                            .rows
                            .retain(|row| row.message_ref != local_ref);
                        panel.projection.recompute_attention();
                        panel.status =
                            format!("Send failed ({error}). Pending local row dropped.").into();
                    }
                }
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    fn renew_device_grant(&mut self, grant: Issue31GrantProjection, cx: &mut Context<Self>) {
        if self.grant_busy.is_some() || grant.status != "active" {
            return;
        }
        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            self.status = "omega-effectd unavailable; device grant was not renewed.".into();
            cx.notify();
            return;
        };
        let grant_ref = grant.grant_ref.clone();
        let scopes = grant.scopes;
        let expires_at = u64::try_from(chrono::Utc::now().timestamp().max(0))
            .unwrap_or(0)
            .saturating_add(24 * 60 * 60);
        let idempotency_ref = format!("idempotency.workroom.grant_renew.{}", Uuid::new_v4());
        self.grant_busy = Some(grant_ref.clone());
        self.status = format!("Renewing device grant {grant_ref}…").into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = {
                let mut guard = supervisor.lock().await;
                match guard.ensure_started().await {
                    Ok(()) => {
                        guard
                            .sarah_renew_device_grant(
                                &grant_ref,
                                &scopes,
                                expires_at,
                                &idempotency_ref,
                            )
                            .await
                    }
                    Err(error) => Err(omega_effectd::SupervisorError::Anyhow(error)),
                }
            };
            this.update(cx, |panel, cx| {
                panel.grant_busy = None;
                panel.status = match result {
                    Ok(_) => format!("Device grant {grant_ref} renewed.").into(),
                    Err(error) => format!("Device grant renewal failed: {error}").into(),
                };
                panel.refresh_from_effectd(cx);
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    fn revoke_device_grant(&mut self, grant_ref: String, cx: &mut Context<Self>) {
        if self.grant_busy.is_some() {
            return;
        }
        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            self.status = "omega-effectd unavailable; device grant was not revoked.".into();
            cx.notify();
            return;
        };
        let idempotency_ref = format!("idempotency.workroom.grant_revoke.{}", Uuid::new_v4());
        self.grant_busy = Some(grant_ref.clone());
        self.status = format!("Revoking device grant {grant_ref}…").into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = {
                let mut guard = supervisor.lock().await;
                match guard.ensure_started().await {
                    Ok(()) => {
                        guard
                            .sarah_revoke_device_grant(
                                &grant_ref,
                                "reason.omega.owner_revoked",
                                &idempotency_ref,
                            )
                            .await
                    }
                    Err(error) => Err(omega_effectd::SupervisorError::Anyhow(error)),
                }
            };
            this.update(cx, |panel, cx| {
                panel.grant_busy = None;
                panel.status = match result {
                    Ok(_) => format!("Device grant {grant_ref} revoked.").into(),
                    Err(error) => format!("Device grant revocation failed: {error}").into(),
                };
                panel.refresh_from_effectd(cx);
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    fn readmit_device(&mut self, grant_ref: String, cx: &mut Context<Self>) {
        if self.grant_busy.is_some() {
            return;
        }
        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            self.status = "omega-effectd unavailable; device was not re-admitted.".into();
            cx.notify();
            return;
        };
        let idempotency_ref = format!("idempotency.workroom.device_readmit.{}", Uuid::new_v4());
        self.grant_busy = Some(grant_ref.clone());
        self.status = format!("Re-admitting the device behind {grant_ref}…").into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = {
                let mut guard = supervisor.lock().await;
                match guard.ensure_started().await {
                    Ok(()) => {
                        guard
                            .sarah_readmit_device(&grant_ref, &idempotency_ref)
                            .await
                    }
                    Err(error) => Err(omega_effectd::SupervisorError::Anyhow(error)),
                }
            };
            this.update(cx, |panel, cx| {
                panel.grant_busy = None;
                panel.status = match result {
                    // Re-admission restores nothing by itself. The device still
                    // has to pair again, so the status must not read as access.
                    Ok(_) => "Device re-admitted. It must pair again before it has any access."
                        .to_string()
                        .into(),
                    Err(error) => format!("Device re-admission failed: {error}").into(),
                };
                panel.refresh_from_effectd(cx);
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    fn interrupt_turn(&mut self, cx: &mut Context<Self>) {
        if self.interrupting {
            return;
        }
        // Law: pending never renders as applied.
        self.interaction.begin_interrupt();
        self.projection.run_state = self.interaction.run.clone();
        self.status = "Interrupt intent pending until terminal turn event.".into();
        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            self.projection.run_state.meta =
                ProjectionMeta::unavailable(sources::EFFECTD, "no supervisor");
            cx.notify();
            return;
        };
        let turn_ref = self
            .interaction
            .run
            .turn_ref
            .clone()
            .or_else(|| self.projection.run_state.turn_ref.clone())
            .unwrap_or_else(|| "active".into());
        self.interrupting = true;
        let idempotency_ref = format!("idempotency.workroom.interrupt.{}", Uuid::new_v4());
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = {
                let mut guard = supervisor.lock().await;
                match guard.ensure_started().await {
                    Ok(()) => {
                        guard
                            .sarah_interrupt_turn(&turn_ref, &idempotency_ref)
                            .await
                    }
                    Err(error) => Err(omega_effectd::SupervisorError::Anyhow(error)),
                }
            };
            this.update(cx, |panel, cx| {
                panel.interrupting = false;
                match result {
                    Ok(value) => {
                        // Accepted intent only. Applied only after terminal event.
                        let state = value
                            .get("state")
                            .or_else(|| value.get("status"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("pending");
                        let pending = value
                            .get("pending")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        if (state == "applied" || state == "interrupted") && !pending {
                            if panel.interaction.run.phase == RunPhase::Interrupted {
                                panel.apply_interaction_event(InteractionEvent::TurnInterrupted {
                                    turn_ref: Some(turn_ref.clone()),
                                    reason: string_field(Some(&value), &["reason"])
                                        .unwrap_or_else(|| "owner_interrupt".into()),
                                });
                            } else {
                                panel.interaction.begin_interrupt();
                                panel.projection.run_state = panel.interaction.run.clone();
                            }
                        } else {
                            // Stay pending — never upgrade to Applied here.
                            panel.interaction.begin_interrupt();
                            panel.projection.run_state = panel.interaction.run.clone();
                        }
                        panel.status = format!(
                            "Interrupt intent: {state} (not applied until terminal event)."
                        )
                        .into();
                    }
                    Err(error) => {
                        // Keep intent visible as pending/unavailable, not applied.
                        panel.interaction.begin_interrupt();
                        panel.projection.run_state = panel.interaction.run.clone();
                        panel.projection.run_state.meta =
                            ProjectionMeta::pending(sources::RUN_STATE);
                        panel.status = format!(
                            "Interrupt request failed ({error}). Intent stays pending, not applied."
                        )
                        .into();
                    }
                }
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    /// `OMEGA-DELTA-0211`. The composer microphone's own start path.
    ///
    /// The composer never navigates to the admission page, so this is what a
    /// click has to do instead: open audio when the exact terms are already
    /// loaded and admitted, and otherwise load them and open audio when they
    /// arrive. Every refusal on this path raises the composer's one-line voice
    /// notice, because a click whose only consequence is a projection nobody
    /// draws is the silent no-op the crawl gate exists to forbid.
    fn start_voice_from_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.voice_state.is_active() {
            return;
        }
        if self.public_demo || self.active_room.is_community() {
            self.start_voice_after_admission = false;
            self.show_composer_voice_notice(cx);
            return;
        }
        let admission_is_ready = self.prepared_voice_admission.is_some()
            && self.workspace.upgrade().is_some_and(|workspace| {
                matches!(
                    agent_ui::composer_voice::sarah_voice_admission(workspace.entity_id(), cx)
                        .read(cx),
                    agent_ui::composer_voice::SarahVoiceAdmissionProjection::Ready { .. }
                )
            });
        if admission_is_ready {
            self.start_voice_after_admission = false;
            self.start_voice(window, cx);
            return;
        }
        self.start_voice_after_admission = true;
        self.prepare_voice_admission(window, cx);
    }

    /// `OMEGA-DELTA-0211`. Raise the composer's one-line notice for this
    /// workspace, leaving the conversation exactly where it was.
    fn show_composer_voice_notice(&self, cx: &mut App) {
        if let Some(workspace) = self.workspace.upgrade() {
            agent_ui::composer_voice::show_composer_voice_notice(workspace.entity_id(), cx);
        }
    }

    /// `OMEGA-DELTA-0211`. Report a refused composer-started attempt where the
    /// person clicked, and forget the pending intent.
    fn refuse_composer_voice_start(&mut self, cx: &mut App) {
        if !self.start_voice_after_admission {
            return;
        }
        self.start_voice_after_admission = false;
        self.show_composer_voice_notice(cx);
    }

    fn prepare_voice_admission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.public_demo || self.active_room.is_community() || self.voice_state.is_active() {
            self.refuse_composer_voice_start(cx);
            return;
        }
        if self.pending_voice_settlement.is_some() {
            self.publish_voice_admission(
                agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                    reason: "The previous Sarah voice session still needs final-charge recovery. Retry settlement before loading admission for another session."
                        .into(),
                    retryable: true,
                    cohort_ref: None,
                    refusal_reason: Some("settlement_retry_required".into()),
                },
                cx,
            );
            self.voice_status = "Previous-session settlement is still pending. Choose Retry settlement; its original thread and session references are retained.".into();
            self.refuse_composer_voice_start(cx);
            cx.notify();
            return;
        }
        self.prepared_voice_admission = None;
        self.voice_admission_terms = None;
        self.publish_voice_admission(
            agent_ui::composer_voice::SarahVoiceAdmissionProjection::Loading {
                detail: "Checking cohort membership, spendable credit, price, and the exact command boundary without opening the microphone or reserving credit."
                    .into(),
            },
            cx,
        );
        let openagents_session = omega_effectd::openagents_session(cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = openagents_session
                .prepare_sarah_voice_admission("sarah-owner-private", cx)
                .await;
            this.update_in(cx, |panel, window, cx| {
                match result {
                    Ok(admission) => {
                        let terms = voice_admission_terms(&admission.projection);
                        if let Some(terms) = terms {
                            panel.prepared_voice_admission = Some(admission);
                            panel.voice_admission_terms = Some(terms.clone());
                            panel.publish_voice_admission(
                                agent_ui::composer_voice::SarahVoiceAdmissionProjection::Ready {
                                    terms,
                                },
                                cx,
                            );
                            // `OMEGA-DELTA-0211`. A composer click is waiting
                            // on exactly these terms: open audio now rather
                            // than making the person find a second control.
                            if panel.start_voice_after_admission {
                                panel.start_voice_after_admission = false;
                                panel.start_voice(window, cx);
                            }
                        } else {
                            panel.publish_voice_admission(
                                agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                                    reason: "OpenAgents returned malformed or incomplete Sarah voice admission terms."
                                        .into(),
                                    retryable: true,
                                    cohort_ref: Some(admission.projection.admission_cohort_ref.into()),
                                    refusal_reason: Some("response_invalid".into()),
                                },
                                cx,
                            );
                            panel.refuse_composer_voice_start(cx);
                        }
                    }
                    Err(blocker) => {
                        let (cohort_ref, refusal_reason) = match &blocker {
                            omega_effectd::HostedSessionBlocker::VoiceCohortInactive {
                                cohort_ref,
                            } => (Some(cohort_ref.clone().into()), Some("cohort_inactive".into())),
                            omega_effectd::HostedSessionBlocker::VoiceAdmissionInsufficientCredit {
                                cohort_ref,
                            } => (
                                Some(cohort_ref.clone().into()),
                                Some("insufficient_credit".into()),
                            ),
                            _ => (None, None),
                        };
                        panel.publish_voice_admission(
                            agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                                reason: blocker.summary().into(),
                                retryable: blocker.is_retryable(),
                                cohort_ref,
                                refusal_reason,
                            },
                            cx,
                        );
                        panel.refuse_composer_voice_start(cx);
                    }
                }
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    fn start_voice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.public_demo || self.active_room.is_community() || self.voice_state.is_active() {
            return;
        }
        if self.pending_voice_settlement.is_some() {
            self.prepare_voice_admission(window, cx);
            return;
        }
        let admission_is_ready = self.workspace.upgrade().is_some_and(|workspace| {
            matches!(
                agent_ui::composer_voice::sarah_voice_admission(workspace.entity_id(), cx).read(cx),
                agent_ui::composer_voice::SarahVoiceAdmissionProjection::Ready { .. }
            )
        });
        if !admission_is_ready {
            self.prepared_voice_admission = None;
        }
        let Some(prepared_admission) = self.prepared_voice_admission.take() else {
            self.publish_voice_admission(
                agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                    reason: "Load and review current Sarah voice admission terms before opening the microphone."
                        .into(),
                    retryable: true,
                    cohort_ref: None,
                    refusal_reason: Some("preflight_required".into()),
                },
                cx,
            );
            self.refuse_composer_voice_start(cx);
            return;
        };
        let reviewed_admission = prepared_admission.projection.clone();

        self.voice_task.take();
        self.flush_voice_transcript_presentation();
        self.voice_controls.take();
        self.pending_voice_command = None;
        self.voice_retryable = false;
        self.voice_access_required = false;
        self.voice_state = SarahVoiceState::Authenticating;
        self.voice_status =
            "Requesting a generation-bound Sarah voice room before opening the microphone…".into();
        let openagents_session = omega_effectd::openagents_session(cx);
        let audio_settings = AudioSettings::get_global(cx);
        let input_device_id = audio_settings.input_audio_device.clone();
        let output_device_id = audio_settings.output_audio_device.clone();
        cx.notify();

        let voice_task = cx.spawn_in(window, async move |this, cx| {
            let mut reconnect_attempts = 0;
            let mut prepared_admission = Some(prepared_admission);
            loop {
                if reconnect_attempts > 0 {
                    cx.background_executor()
                        .timer(voice_reconnect_delay(reconnect_attempts))
                        .await;
                }
                let admission = match prepared_admission.take() {
                    Some(admission) => admission,
                    None => match openagents_session
                        .prepare_sarah_voice_admission("sarah-owner-private", cx)
                        .await
                    {
                        Ok(admission) => {
                            if !reviewed_admission
                                .has_same_reviewed_terms(&admission.projection)
                            {
                                this.update(cx, |panel, cx| {
                                    let terms = voice_admission_terms(&admission.projection);
                                    panel.voice_state = SarahVoiceState::Idle;
                                    panel.voice_retryable = false;
                                    panel.voice_access_required = false;
                                    panel.voice_status = "OpenAgents changed one or more Sarah voice admission terms. Review the new terms and choose Start voice again; the microphone stayed off and no ticket was requested.".into();
                                    if let Some(terms) = terms {
                                        panel.prepared_voice_admission = Some(admission);
                                        panel.voice_admission_terms = Some(terms.clone());
                                        panel.publish_voice_admission(
                                            agent_ui::composer_voice::SarahVoiceAdmissionProjection::Ready { terms },
                                            cx,
                                        );
                                    } else {
                                        panel.publish_voice_admission(
                                            agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                                                reason: "OpenAgents changed Sarah voice terms, but the replacement terms were incomplete. The microphone stayed off and no ticket was requested.".into(),
                                                retryable: true,
                                                cohort_ref: Some(admission.projection.admission_cohort_ref.into()),
                                                refusal_reason: Some("admission_terms_changed".into()),
                                            },
                                            cx,
                                        );
                                    }
                                    cx.notify();
                                })?;
                                return anyhow::Ok(());
                            }
                            admission
                        }
                        Err(blocker) => {
                            this.update(cx, |panel, cx| {
                                panel.voice_state = SarahVoiceState::Error;
                                panel.voice_retryable = blocker.is_retryable();
                                panel.voice_access_required = blocker.requires_voice_access();
                                panel.voice_status = blocker.summary().into();
                                cx.notify();
                            })?;
                            return anyhow::Ok(());
                        }
                    },
                };
                this.update(cx, |panel, cx| {
                    panel.voice_state = SarahVoiceState::Authenticating;
                    panel.voice_status = "Admission is ready. Requesting a generation-bound Sarah voice room before opening the microphone…".into();
                    cx.notify();
                })?;
                let managed_session = match openagents_session
                    .create_sarah_voice_session_from_admission(&admission, cx)
                    .await
                {
                    Ok(session) => session,
                    Err(blocker) => {
                        if blocker.is_retryable()
                            && reconnect_attempts < MAX_CONSECUTIVE_VOICE_RECONNECTS
                        {
                            reconnect_attempts = reconnect_attempts.saturating_add(1);
                            this.update(cx, |panel, cx| {
                                panel.voice_state = SarahVoiceState::Reconnecting;
                                panel.voice_retryable = false;
                                panel.voice_access_required = blocker.requires_voice_access();
                                panel.voice_status = if matches!(
                                    &blocker,
                                    omega_effectd::HostedSessionBlocker::VoiceSessionRejected {
                                        status: 409
                                    }
                                ) {
                                    "The previous Sarah session is settling. Waiting before requesting a fresh one-use ticket…".into()
                                } else {
                                    format!(
                                        "Sarah voice admission is temporarily unavailable ({}). Retrying with bounded backoff…",
                                        blocker.summary()
                                    )
                                    .into()
                                };
                                cx.notify();
                            })?;
                            continue;
                        }
                        this.update(cx, |panel, cx| {
                            panel.voice_state = SarahVoiceState::Error;
                            panel.voice_retryable = blocker.is_retryable();
                            panel.voice_access_required = blocker.requires_voice_access();
                            panel.voice_status = blocker.summary().into();
                            cx.notify();
                        })?;
                        return anyhow::Ok(());
                    }
                };
                this.update(cx, |panel, cx| {
                    panel.voice_state = SarahVoiceState::RequestingMicrophone;
                    panel.voice_status = if reconnect_attempts == 0 {
                        "The Sarah room is generation-bound. Requesting microphone access…".into()
                    } else {
                        "The replacement Sarah room is generation-bound. Reopening audio…".into()
                    };
                    cx.notify();
                })?;
                let prepared_devices = cx
                    .background_executor()
                    .spawn({
                        let input_device_id = input_device_id.clone();
                        let output_device_id = output_device_id.clone();
                        async move {
                            ManagedSarahVoiceClient::prepare_devices(
                                input_device_id,
                                output_device_id,
                            )
                        }
                    })
                    .await;
                let prepared_devices = match prepared_devices {
                    Ok(prepared_devices) => prepared_devices,
                    Err(error) => {
                        this.update(cx, |panel, cx| {
                            panel.voice_state = SarahVoiceState::Error;
                            panel.voice_retryable = true;
                            panel.voice_access_required = false;
                            panel.voice_status =
                                format!("Sarah voice could not open audio devices: {error:#}")
                                    .into();
                            cx.notify();
                        })?;
                        return anyhow::Ok(());
                    }
                };
                let settlement_thread_ref = managed_session.thread_ref.clone();
                let settlement_session_ref = managed_session.session_ref.clone();
                this.update(cx, |panel, _cx| {
                    panel.pending_voice_settlement = Some(PendingSarahVoiceSettlement {
                        thread_ref: settlement_thread_ref.clone(),
                        session_ref: settlement_session_ref.clone(),
                    });
                })?;
                let client = match ManagedSarahVoiceClient::from_managed_session_with_devices(
                    managed_session,
                    prepared_devices,
                ) {
                    Ok(client) => client,
                    Err(error) => {
                        this.update(cx, |panel, cx| {
                            panel.voice_state = SarahVoiceState::Error;
                            panel.voice_retryable = true;
                            panel.voice_access_required = false;
                            panel.voice_status =
                                format!("Sarah voice could not start: {error:#}").into();
                            cx.notify();
                        })?;
                        return anyhow::Ok(());
                    }
                };
                let connection_started_at = Instant::now();
                let connection = client.connect(cx);
                let controls = connection.controls;
                let events = connection.events;
                this.update(cx, |panel, cx| {
                    panel.voice_controls = Some(controls);
                    if panel.voice_muted {
                        panel.send_voice_control(SarahVoiceControl::SetMuted(true));
                    }
                    panel.voice_state = SarahVoiceState::Connecting;
                    panel.voice_status = panel.voice_state.label().into();
                    cx.notify();
                })?;

                let mut retryable_failure = false;
                while let Ok(event) = events.recv().await {
                    retryable_failure = matches!(
                        event,
                        SarahVoiceEvent::Error {
                            retryable: true,
                            ..
                        }
                    );
                    this.update_in(cx, |panel, window, cx| {
                        panel.handle_voice_event(event, window, cx);
                    })?;
                    if retryable_failure {
                        break;
                    }
                }
                let mut settlement_attempt = 0_usize;
                let mut settlement_recovered = false;
                loop {
                    let settlement = openagents_session
                        .read_sarah_voice_settlement(
                            &settlement_thread_ref,
                            &settlement_session_ref,
                            cx,
                        )
                        .await;
                    match settlement {
                        Ok(settlement)
                            if settlement.state
                                == omega_effectd::SarahVoiceSettlementState::Pending
                                && voice_settlement_retry_delay(settlement_attempt, true).is_some() =>
                        {
                            let Some(delay) =
                                voice_settlement_retry_delay(settlement_attempt, true)
                            else {
                                break;
                            };
                            settlement_attempt = settlement_attempt.saturating_add(1);
                            cx.background_executor().timer(delay).await;
                        }
                        Ok(settlement) => {
                            settlement_recovered = voice_settlement_is_recovered(
                                settlement.state,
                                settlement.final_charge_msat,
                            );
                            this.update_in(cx, |panel, window, cx| {
                                panel.handle_voice_event(
                                    SarahVoiceEvent::Settlement(settlement),
                                    window,
                                    cx,
                                );
                            })?;
                            break;
                        }
                        Err(blocker)
                            if blocker.is_retryable()
                                && voice_settlement_retry_delay(settlement_attempt, true).is_some() =>
                        {
                            let Some(delay) =
                                voice_settlement_retry_delay(settlement_attempt, true)
                            else {
                                break;
                            };
                            settlement_attempt = settlement_attempt.saturating_add(1);
                            cx.background_executor().timer(delay).await;
                        }
                        Err(blocker) => {
                            this.update(cx, |panel, cx| {
                                panel.publish_voice_admission(
                                    agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                                        reason: format!(
                                            "Sarah voice ended, but final-charge recovery for session {} is still unavailable: {}. The original settlement target is retained; choose Retry settlement to try again.",
                                            settlement_session_ref,
                                            blocker.summary(),
                                        )
                                        .into(),
                                        retryable: blocker.is_retryable(),
                                        cohort_ref: None,
                                        refusal_reason: Some("settlement_unavailable".into()),
                                    },
                                    cx,
                                );
                            })?;
                            break;
                        }
                    }
                }
                if !settlement_recovered {
                    this.update(cx, |panel, cx| {
                        panel.voice_controls = None;
                        panel.voice_state = SarahVoiceState::Error;
                        panel.voice_retryable = true;
                        panel.voice_status = "Sarah voice will not open a new session until the previous final charge is recovered. Choose Retry settlement; the original target is retained.".into();
                        cx.notify();
                    })?;
                    break;
                }
                let reconnect_attempt = this.update(cx, |panel, cx| {
                    panel.voice_controls = None;
                    let reconnect_attempt = next_voice_reconnect_attempt(
                        panel.voice_state,
                        retryable_failure,
                        reconnect_attempts,
                        connection_started_at.elapsed(),
                    );
                    if reconnect_attempt.is_some() {
                        panel.voice_state = SarahVoiceState::Reconnecting;
                        panel.voice_retryable = false;
                        panel.voice_status =
                            "The Sarah voice connection dropped. Reconnecting automatically…"
                                .into();
                    } else if panel.voice_state.is_active()
                        || (panel.voice_state == SarahVoiceState::Error && retryable_failure)
                    {
                        panel.voice_state = SarahVoiceState::Error;
                        panel.voice_retryable = true;
                        panel.voice_status = "Sarah voice could not restore a stable connection after three attempts. Retry when the network is stable.".into();
                    }
                    cx.notify();
                    reconnect_attempt
                })?;
                let Some(reconnect_attempt) = reconnect_attempt else {
                    break;
                };
                reconnect_attempts = reconnect_attempt;
            }
            anyhow::Ok(())
        });
        self.voice_task = Some(cx.spawn(async move |_, _| {
            if let Err(error) = voice_task.await {
                log::error!("Sarah voice UI task failed: {error:#}");
            }
        }));
    }

    fn toggle_voice_mute(&mut self, cx: &mut Context<Self>) {
        if self.voice_controls.is_none()
            || !self.voice_state.is_active()
            || self.voice_state == SarahVoiceState::Reconnecting
        {
            return;
        }
        self.voice_muted = !self.voice_muted;
        self.send_voice_control(SarahVoiceControl::SetMuted(self.voice_muted));
        self.voice_status = if self.voice_muted {
            "Microphone muted. Sarah cannot hear new audio.".into()
        } else {
            "Microphone unmuted. Sarah is listening.".into()
        };
        cx.notify();
    }

    fn interrupt_voice(&mut self, cx: &mut Context<Self>) {
        if self.voice_controls.is_none()
            || !self.voice_state.is_active()
            || self.voice_state == SarahVoiceState::Reconnecting
        {
            return;
        }
        self.send_voice_control(SarahVoiceControl::Interrupt);
        self.flush_voice_transcript_presentation();
        self.voice_state = SarahVoiceState::Listening;
        self.voice_status = "Sarah's spoken response was interrupted.".into();
        cx.notify();
    }

    fn end_voice(&mut self, cx: &mut Context<Self>) {
        if self.voice_controls.is_none() {
            self.cleanup_voice();
            cx.notify();
            return;
        }
        self.voice_state = SarahVoiceState::Ending;
        self.voice_status = self.voice_state.label().into();
        self.send_voice_control(SarahVoiceControl::Close);
        cx.notify();
    }

    fn retry_voice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_voice_settlement.is_some() {
            self.retry_voice_settlement(cx);
            return;
        }
        self.cleanup_voice();
        self.start_voice(window, cx);
    }

    fn retry_voice_settlement(&mut self, cx: &mut Context<Self>) {
        if self.settlement_retrying {
            return;
        }
        let Some(target) = self.pending_voice_settlement.clone() else {
            return;
        };
        self.settlement_retrying = true;
        self.voice_retryable = false;
        self.voice_status = format!(
            "Retrying final-charge recovery for Sarah session {}…",
            target.session_ref
        )
        .into();
        let openagents_session = omega_effectd::openagents_session(cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let mut attempt = 0_usize;
            loop {
                let result = openagents_session
                    .read_sarah_voice_settlement(&target.thread_ref, &target.session_ref, cx)
                    .await;
                match result {
                    Ok(settlement)
                        if settlement.state
                            == omega_effectd::SarahVoiceSettlementState::Pending
                            && voice_settlement_retry_delay(attempt, true).is_some() =>
                    {
                        let Some(delay) = voice_settlement_retry_delay(attempt, true) else {
                            break;
                        };
                        attempt = attempt.saturating_add(1);
                        cx.background_executor().timer(delay).await;
                    }
                    Err(blocker)
                        if blocker.is_retryable()
                            && voice_settlement_retry_delay(attempt, true).is_some() =>
                    {
                        let Some(delay) = voice_settlement_retry_delay(attempt, true) else {
                            break;
                        };
                        attempt = attempt.saturating_add(1);
                        cx.background_executor().timer(delay).await;
                    }
                    Ok(settlement) => {
                        this.update(cx, |panel, cx| {
                            panel.settlement_retrying = false;
                            panel.handle_voice_settlement(settlement, cx);
                            cx.notify();
                        })?;
                        break;
                    }
                    Err(blocker) => {
                        this.update(cx, |panel, cx| {
                            panel.settlement_retrying = false;
                            let retryable = blocker.is_retryable();
                            panel.voice_retryable = retryable;
                            panel.voice_status = format!(
                                "Final-charge recovery for Sarah session {} is still unavailable: {}. The original settlement target is retained{}.",
                                target.session_ref,
                                blocker.summary(),
                                if retryable {
                                    "; choose Retry settlement to try again"
                                } else {
                                    ""
                                },
                            )
                            .into();
                            panel.publish_voice_admission(
                                agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                                    reason: panel.voice_status.clone(),
                                    retryable,
                                    cohort_ref: None,
                                    refusal_reason: Some("settlement_unavailable".into()),
                                },
                                cx,
                            );
                            cx.notify();
                        })?;
                        break;
                    }
                }
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    fn cleanup_voice(&mut self) {
        if let Some(controls) = self.voice_controls.take()
            && controls.try_send(SarahVoiceControl::Close).is_err()
        {
            log::debug!("Sarah voice control channel was already closed during cleanup");
        }
        self.voice_task.take();
        self.flush_voice_transcript_presentation();
        self.pending_voice_command = None;
        self.voice_session_id = None;
        self.voice_muted = false;
        self.voice_retryable = false;
        self.voice_access_required = false;
        self.voice_state = SarahVoiceState::Idle;
        self.voice_status = "Managed voice is ready to start.".into();
    }

    fn send_voice_control(&mut self, control: SarahVoiceControl) {
        let Some(controls) = &self.voice_controls else {
            return;
        };
        if controls.try_send(control).is_err() {
            self.voice_state = SarahVoiceState::Reconnecting;
            self.voice_retryable = true;
            self.voice_status =
                "The Sarah voice connection is no longer writable. Retry to reconnect.".into();
        }
    }

    fn handle_voice_event(
        &mut self,
        event: SarahVoiceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SarahVoiceEvent::State(state) => {
                self.voice_state = state;
                self.voice_status = state.label().into();
            }
            SarahVoiceEvent::Admission(admission) => {
                self.voice_admission_terms = voice_admission_terms(&admission);
                if let Some(terms) = self.voice_admission_terms.clone() {
                    self.publish_voice_admission(
                        agent_ui::composer_voice::SarahVoiceAdmissionProjection::Ready { terms },
                        cx,
                    );
                } else {
                    self.voice_state = SarahVoiceState::Error;
                    self.voice_retryable = true;
                    self.voice_status = admission
                        .detail
                        .clone()
                        .unwrap_or_else(|| {
                            "OpenAgents did not return complete Sarah voice admission terms."
                                .to_string()
                        })
                        .into();
                    self.publish_voice_admission(
                        agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                            reason: self.voice_status.clone(),
                            retryable: true,
                            cohort_ref: Some(admission.admission_cohort_ref.into()),
                            refusal_reason: Some("response_invalid".into()),
                        },
                        cx,
                    );
                    self.send_voice_control(SarahVoiceControl::Close);
                }
            }
            SarahVoiceEvent::Ready { session_id } => {
                self.voice_session_id = Some(session_id.clone());
                self.voice_state = SarahVoiceState::Listening;
                self.voice_status =
                    "Connected through the managed OpenAgents Sarah voice service.".into();
                // omega#164. A live Sarah session is one of the events that
                // gives a background-created identity something to lose, so it
                // arms the quiet backup nudge. Fail-soft by design.
                cx.background_spawn(async {
                    if let Err(error) =
                        omega_identity::IdentityService::system(*app_identity::CHANNEL)
                            .record_backup_value_accrued(
                                omega_identity::BackupValueKind::SarahSession,
                            )
                    {
                        log::warn!("could not record identity backup value accrual: {error}");
                    }
                })
                .detach();
                if let Some(terms) = self.voice_admission_terms.clone() {
                    self.publish_voice_admission(
                        agent_ui::composer_voice::SarahVoiceAdmissionProjection::Active {
                            terms,
                            session_id: session_id.into(),
                            artifacts: self.voice_session_artifacts(),
                        },
                        cx,
                    );
                }
            }
            SarahVoiceEvent::TranscriptRecovered(recovery) => {
                let recovery_gap = recovery.gap;
                let recovery_detail = recovery.detail.clone();
                for item in recovery.items {
                    if !self.voice_transcript.iter().any(|existing| {
                        existing.thread_ref == item.thread_ref
                            && existing.session_ref == item.session_ref
                            && existing.item_id == item.item_id
                    }) {
                        self.voice_transcript.push(item);
                    }
                }
                if self.voice_transcript.len() > 100 {
                    let overflow = self.voice_transcript.len() - 100;
                    self.voice_transcript.drain(..overflow);
                }
                if recovery_gap != VoiceTranscriptRecoveryGap::Complete {
                    self.voice_status = recovery_detail
                        .clone()
                        .unwrap_or_else(|| "Recovered Sarah transcript has a declared gap.".into())
                        .into();
                }
                self.voice_transcript_recovery = match recovery_gap {
                    VoiceTranscriptRecoveryGap::Complete => {
                        "Recovered local transcript rows with no detected gap.".into()
                    }
                    VoiceTranscriptRecoveryGap::Truncated => recovery_detail
                        .unwrap_or_else(|| "Recovered transcript was bounded and truncated.".into())
                        .into(),
                    VoiceTranscriptRecoveryGap::Malformed => recovery_detail
                        .unwrap_or_else(|| {
                            "Malformed transcript rows were skipped and not presented as recovered."
                                .into()
                        })
                        .into(),
                };
                self.flush_voice_transcript_presentation();
            }
            SarahVoiceEvent::TranscriptDelta {
                thread_ref,
                session_ref,
                item_id,
                participant,
                delta,
            } => {
                self.append_voice_transcript_delta(
                    thread_ref,
                    session_ref,
                    item_id,
                    participant,
                    delta,
                    cx,
                );
            }
            SarahVoiceEvent::TranscriptCompleted {
                thread_ref,
                session_ref,
                item_id,
                participant,
                text,
            } => {
                self.complete_voice_transcript(
                    thread_ref,
                    session_ref,
                    item_id,
                    participant,
                    text,
                    cx,
                );
            }
            SarahVoiceEvent::CommandProposal(request) => {
                if self.pending_voice_command.is_some() {
                    self.send_voice_control(SarahVoiceControl::CommandDecision {
                        request_id: request.request_id,
                        approved: false,
                        effect_binding: None,
                    });
                } else {
                    let request_id = request.request_id.clone();
                    match self.bind_voice_command_proposal(request, cx) {
                        Ok(request) => {
                            self.voice_status = request.command.confirmation_copy().into();
                            self.pending_voice_command = Some(request);
                        }
                        Err(error) => {
                            self.send_voice_control(SarahVoiceControl::CommandDecision {
                                request_id,
                                approved: false,
                                effect_binding: None,
                            });
                            self.voice_status = format!(
                                "Sarah's replacement was refused because its editor context was not exact: {error:#}"
                            )
                            .into();
                        }
                    }
                }
            }
            SarahVoiceEvent::CommandRequest(request) => {
                let result = self.execute_voice_command(request, window, cx);
                self.send_voice_control(SarahVoiceControl::CommandResult(result));
            }
            SarahVoiceEvent::Error {
                message,
                retryable,
                action,
            } => {
                self.flush_voice_transcript_presentation();
                self.voice_state = SarahVoiceState::Error;
                self.voice_retryable = retryable;
                self.voice_status = match action {
                    Some(action) => format!("{message} {action}").into(),
                    None => message.into(),
                };
            }
            SarahVoiceEvent::Ended { reason } => {
                self.flush_voice_transcript_presentation();
                self.voice_controls = None;
                self.pending_voice_command = None;
                self.voice_session_id = None;
                if reason.as_deref() == Some("ended_by_user") {
                    self.voice_state = SarahVoiceState::Idle;
                    self.voice_retryable = false;
                    self.voice_status = "Voice session ended.".into();
                } else {
                    self.voice_state = SarahVoiceState::Reconnecting;
                    self.voice_retryable = true;
                    self.voice_status = format!(
                        "Sarah voice disconnected{}. Retry to reconnect.",
                        reason
                            .filter(|reason| !reason.is_empty())
                            .map(|reason| format!(" ({reason})"))
                            .unwrap_or_default()
                    )
                    .into();
                }
            }
            SarahVoiceEvent::Settlement(settlement) => {
                self.handle_voice_settlement(settlement, cx);
            }
        }
        self.publish_active_voice_artifacts(cx);
        cx.notify();
    }

    fn handle_voice_settlement(
        &mut self,
        settlement: omega_effectd::SarahVoiceSettlementProjection,
        cx: &mut App,
    ) {
        if voice_settlement_is_recovered(settlement.state, settlement.final_charge_msat) {
            if let Some(final_charge_msat) = settlement.final_charge_msat {
                self.pending_voice_settlement = None;
                self.publish_voice_admission(
                    agent_ui::composer_voice::SarahVoiceAdmissionProjection::Settled {
                        final_charge_msat,
                        remaining_credit_msat: settlement.remaining_credit_msat,
                        receipt_ref: settlement.receipt_ref.map(Into::into),
                        transcript_recovery: self.voice_transcript_recovery.clone(),
                        artifacts: self.voice_session_artifacts(),
                    },
                    cx,
                );
            } else {
                self.publish_voice_admission(
                    agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                        reason: "OpenAgents returned a terminal Sarah settlement without its final charge. The original session target is retained; choose Retry settlement to try again."
                            .into(),
                        retryable: true,
                        cohort_ref: None,
                        refusal_reason: Some("settlement_malformed".into()),
                    },
                    cx,
                );
            }
        } else {
            self.publish_voice_admission(
                agent_ui::composer_voice::SarahVoiceAdmissionProjection::Unavailable {
                    reason: settlement
                        .detail
                        .unwrap_or_else(|| {
                            "Sarah settlement readback is not available yet. The original session target is retained; choose Retry settlement to try again."
                                .into()
                        })
                        .into(),
                    retryable: true,
                    cohort_ref: None,
                    refusal_reason: Some("settlement_pending".into()),
                },
                cx,
            );
        }
    }

    fn append_voice_transcript_delta(
        &mut self,
        thread_ref: String,
        session_ref: String,
        item_id: String,
        participant: VoiceParticipant,
        delta: String,
        cx: &mut Context<Self>,
    ) {
        let transcript_key = (thread_ref.clone(), session_ref.clone(), item_id.clone());
        let item_index = self.voice_transcript.iter().position(|item| {
            item.thread_ref == thread_ref
                && item.session_ref == session_ref
                && item.item_id == item_id
        });
        let previous_char_count = item_index
            .and_then(|index| self.voice_transcript.get(index))
            .map_or(0, |item| item.text.chars().count());
        if let Some(item_index) = item_index {
            let item = &mut self.voice_transcript[item_index];
            item.text.push_str(&delta);
            item.text = truncate_chars(std::mem::take(&mut item.text), MAX_VOICE_TRANSCRIPT_CHARS);
            item.complete = false;
        } else {
            self.voice_transcript.push(VoiceTranscriptItem {
                thread_ref,
                session_ref,
                item_id,
                participant,
                text: truncate_chars(delta, MAX_VOICE_TRANSCRIPT_CHARS),
                complete: false,
            });
            if self.voice_transcript.len() > 100 {
                let removed_item = self.voice_transcript.remove(0);
                self.voice_transcript_presentation.forget(&removed_item);
            }
        }
        if let Some(item) = self.voice_transcript.iter().find(|item| {
            item.thread_ref == transcript_key.0
                && item.session_ref == transcript_key.1
                && item.item_id == transcript_key.2
        }) {
            self.voice_transcript_presentation
                .observe_authoritative_change(item, previous_char_count);
        }
        self.ensure_voice_transcript_pacing(cx);
    }

    fn complete_voice_transcript(
        &mut self,
        thread_ref: String,
        session_ref: String,
        item_id: String,
        participant: VoiceParticipant,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let transcript_key = (thread_ref.clone(), session_ref.clone(), item_id.clone());
        let item_index = self.voice_transcript.iter().position(|item| {
            item.thread_ref == thread_ref
                && item.session_ref == session_ref
                && item.item_id == item_id
        });
        let previous_char_count = item_index
            .and_then(|index| self.voice_transcript.get(index))
            .map_or(0, |item| item.text.chars().count());
        if let Some(item_index) = item_index {
            let item = &mut self.voice_transcript[item_index];
            item.participant = participant;
            item.text = truncate_chars(text, MAX_VOICE_TRANSCRIPT_CHARS);
            item.complete = true;
        } else {
            self.voice_transcript.push(VoiceTranscriptItem {
                thread_ref,
                session_ref,
                item_id,
                participant,
                text: truncate_chars(text, MAX_VOICE_TRANSCRIPT_CHARS),
                complete: true,
            });
            if self.voice_transcript.len() > 100 {
                let removed_item = self.voice_transcript.remove(0);
                self.voice_transcript_presentation.forget(&removed_item);
            }
        }
        if let Some(item) = self.voice_transcript.iter().find(|item| {
            item.thread_ref == transcript_key.0
                && item.session_ref == transcript_key.1
                && item.item_id == transcript_key.2
        }) {
            self.voice_transcript_presentation
                .observe_authoritative_change(item, previous_char_count);
        }
        self.ensure_voice_transcript_pacing(cx);
    }

    fn ensure_voice_transcript_pacing(&mut self, cx: &mut Context<Self>) {
        if cx.reduce_motion() {
            self.flush_voice_transcript_presentation();
            return;
        }
        if self.voice_transcript_pacing_task.is_some()
            || !self
                .voice_transcript_presentation
                .has_hidden_sarah_text(&self.voice_transcript)
        {
            return;
        }

        let executor = cx.background_executor().clone();
        self.voice_transcript_pacing_task = Some(cx.spawn(async move |this, cx| {
            loop {
                executor.timer(VOICE_TRANSCRIPT_PACING_INTERVAL).await;
                let should_continue = match this.update(cx, |panel, cx| {
                    if cx.reduce_motion() {
                        let had_hidden_text = panel
                            .voice_transcript_presentation
                            .has_hidden_sarah_text(&panel.voice_transcript);
                        panel.voice_transcript_presentation.flush();
                        panel.voice_transcript_pacing_task = None;
                        if had_hidden_text {
                            cx.notify();
                        }
                        return false;
                    }

                    let changed = panel
                        .voice_transcript_presentation
                        .advance(&panel.voice_transcript);
                    let has_hidden_text = panel
                        .voice_transcript_presentation
                        .has_hidden_sarah_text(&panel.voice_transcript);
                    if !has_hidden_text {
                        panel.voice_transcript_pacing_task = None;
                    }
                    if changed {
                        cx.notify();
                    }
                    has_hidden_text
                }) {
                    Ok(should_continue) => should_continue,
                    Err(error) => {
                        log::debug!("Sarah voice transcript pacing stopped: {error:#}");
                        break;
                    }
                };
                if !should_continue {
                    break;
                }
            }
        }));
    }

    fn flush_voice_transcript_presentation(&mut self) {
        self.voice_transcript_presentation.flush();
        self.voice_transcript_pacing_task.take();
    }

    fn current_voice_selection(&self, cx: &mut Context<Self>) -> Result<CurrentVoiceSelection> {
        let workspace = self
            .workspace
            .upgrade()
            .context("the workspace is no longer available")?;
        let (editor, path) = {
            let workspace = workspace.read(cx);
            let editor = workspace
                .active_item_as::<Editor>(cx)
                .context("open an editor before asking Sarah to replace a selection")?;
            let path = workspace
                .active_item(cx)
                .and_then(|item| item.project_path(cx))
                .context("save the active file before asking Sarah to replace a selection")?
                .path
                .as_ref()
                .as_unix_str()
                .to_string();
            (editor, path)
        };
        editor.update(cx, |editor, cx| {
            let display_snapshot = editor.display_snapshot(cx);
            let selection = editor.selections.newest::<Point>(&display_snapshot);
            let range = selection.range();
            let buffer_snapshot = editor.buffer().read(cx).snapshot(cx);
            let selected_text = buffer_snapshot
                .text_for_range(range.clone())
                .collect::<String>();
            if range.start == range.end {
                anyhow::bail!("select text before asking Sarah to replace it");
            }
            if selected_text.len() > MAX_VOICE_SELECTION_SNAPSHOT_BYTES {
                anyhow::bail!(
                    "the selected text exceeds the {MAX_VOICE_SELECTION_SNAPSHOT_BYTES}-byte confirmation limit"
                );
            }
            let document_version = buffer_snapshot
                .as_singleton()
                .map(|snapshot| voice_document_version(snapshot))
                .context("the active editor is not a single document")?;
            Ok(CurrentVoiceSelection {
                path,
                document_version,
                start: voice_text_point(range.start),
                end: voice_text_point(range.end),
                selected_text,
            })
        })
    }

    fn bind_voice_command_proposal(
        &self,
        mut request: VoiceCommandRequest,
        cx: &mut Context<Self>,
    ) -> Result<VoiceCommandRequest> {
        let SarahEditorCommand::ReplaceSelection { text } = &request.command else {
            return Ok(request);
        };
        let target = request
            .target
            .as_ref()
            .context("Sarah's replacement proposal omitted its editor target")?;
        let document_version = target
            .document_version
            .clone()
            .context("Sarah's replacement proposal omitted its document version")?;
        let current = self.current_voice_selection(cx)?;
        let binding = VoiceSelectionEffectBinding {
            workspace_ref: target.workspace_ref.clone(),
            document_version,
            target_path: target.path.clone(),
            selection_start: current.start,
            selection_end: current.end,
            selected_text: current.selected_text.clone(),
            replacement_text: text.clone(),
        };
        validate_voice_selection_effect(
            &binding,
            target,
            &current.path,
            &current.document_version,
            current.start,
            current.end,
            &current.selected_text,
            text,
        )
        .map_err(anyhow::Error::msg)?;
        request.effect_binding = Some(binding);
        Ok(request)
    }

    fn validate_pending_voice_effect(
        &self,
        request: &VoiceCommandRequest,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let SarahEditorCommand::ReplaceSelection { text } = &request.command else {
            return Ok(());
        };
        let target = request
            .target
            .as_ref()
            .context("Sarah's replacement proposal omitted its editor target")?;
        let binding = request
            .effect_binding
            .as_ref()
            .context("Sarah's replacement proposal omitted its editor-state binding")?;
        let current = self.current_voice_selection(cx)?;
        validate_voice_selection_effect(
            binding,
            target,
            &current.path,
            &current.document_version,
            current.start,
            current.end,
            &current.selected_text,
            text,
        )
        .map_err(anyhow::Error::msg)
    }

    fn approve_voice_command(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.pending_voice_command.take() else {
            return;
        };
        if let Err(error) = self.validate_pending_voice_effect(&request, cx) {
            self.send_voice_control(SarahVoiceControl::CommandDecision {
                request_id: request.request_id,
                approved: false,
                effect_binding: None,
            });
            self.voice_status = format!(
                "Sarah's command was not approved because its editor context changed: {error:#}"
            )
            .into();
            self.publish_active_voice_artifacts(cx);
            cx.notify();
            return;
        }
        self.send_voice_control(SarahVoiceControl::CommandDecision {
            request_id: request.request_id,
            approved: true,
            effect_binding: request.effect_binding,
        });
        self.voice_status = "Command approved once. Waiting for secure execution…".into();
        self.publish_active_voice_artifacts(cx);
        cx.notify();
    }

    fn reject_voice_command(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.pending_voice_command.take() else {
            return;
        };
        self.send_voice_control(SarahVoiceControl::CommandDecision {
            request_id: request.request_id,
            approved: false,
            effect_binding: None,
        });
        self.voice_status = "Sarah's command was declined.".into();
        self.publish_active_voice_artifacts(cx);
        cx.notify();
    }

    fn execute_voice_command(
        &mut self,
        request: VoiceCommandRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> VoiceCommandResult {
        let request_id = request.request_id;
        let target = request.target;
        let effect_binding = request.effect_binding;
        if request.expires_at_ms.is_some_and(|expires_at_ms| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|duration| u64::try_from(duration.as_millis()).ok())
                .is_none_or(|now_ms| now_ms > expires_at_ms)
        }) {
            self.voice_status = "Sarah's command proposal expired before execution.".into();
            return VoiceCommandResult::rejected(
                request_id,
                "The command proposal expired. Ask Sarah to try again.",
            );
        }
        let is_agent_thread_command = matches!(
            &request.command,
            SarahEditorCommand::StartAgentThread { .. }
        );
        let result = match self.workspace.upgrade() {
            Some(workspace) => match request.command {
                SarahEditorCommand::StartAgentThread {
                    message,
                    presentation,
                } => self.start_agent_thread(workspace, message, presentation, window, cx),
                command => Self::execute_editor_command(
                    workspace,
                    command,
                    target,
                    effect_binding,
                    window,
                    cx,
                ),
            },
            None => Err(anyhow::anyhow!("the workspace is no longer available")),
        };

        match result {
            Ok(output) => {
                if !is_agent_thread_command {
                    self.voice_status = "Sarah's editor command completed.".into();
                }
                VoiceCommandResult::completed(request_id, Some(output))
            }
            Err(error) => {
                if error.downcast_ref::<VoiceCommandRefusal>().is_some() {
                    self.voice_status = format!("Sarah's command was refused: {error:#}").into();
                    VoiceCommandResult::rejected(request_id, format!("{error:#}"))
                } else {
                    self.voice_status = format!("Sarah's command failed: {error:#}").into();
                    VoiceCommandResult::failed(request_id, format!("{error:#}"))
                }
            }
        }
    }

    fn execute_editor_command(
        workspace: Entity<Workspace>,
        command: SarahEditorCommand,
        target: Option<VoiceEditorTarget>,
        effect_binding: Option<VoiceSelectionEffectBinding>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value> {
        let (editor, project_path) = {
            let workspace = workspace.read(cx);
            (
                workspace.active_item_as::<Editor>(cx),
                workspace
                    .active_item(cx)
                    .and_then(|item| item.project_path(cx)),
            )
        };
        if let Some(target) = &target {
            if target.workspace_ref != SARAH_VOICE_WORKSPACE_REF {
                return Err(refuse_voice_command(
                    "The command targeted a different Omega workspace.",
                ));
            }
            let active_path = project_path
                .as_ref()
                .map(|path| path.path.as_ref().as_unix_str().to_string());
            if active_path.as_deref() != Some(target.path.as_str()) {
                return Err(refuse_voice_command(format!(
                    "Sarah's command targeted {}, but that file is not the active editor.",
                    target.path
                )));
            }
        }
        let editor = editor.context("open an editor before asking Sarah to edit")?;
        match command {
            SarahEditorCommand::ReadContext { max_chars } => editor.update(cx, |editor, cx| {
                let display_snapshot = editor.display_snapshot(cx);
                let selection = editor.selections.newest::<Point>(&display_snapshot);
                let cursor = selection.head();
                let buffer_snapshot = editor.buffer().read(cx).snapshot(cx);
                let document_version = buffer_snapshot
                    .as_singleton()
                    .map(|snapshot| voice_document_version(snapshot));
                let selected_text = buffer_snapshot
                    .text_for_range(selection.range())
                    .collect::<String>();
                let start = buffer_snapshot
                    .clip_point(Point::new(cursor.row.saturating_sub(80), 0), Bias::Left);
                let end = buffer_snapshot.clip_point(
                    Point::new(cursor.row.saturating_add(80), u32::MAX),
                    Bias::Right,
                );
                let max_chars = max_chars.unwrap_or(8 * 1024) as usize;
                let context = truncate_chars(
                    buffer_snapshot
                        .text_for_range(start..end)
                        .collect::<String>(),
                    max_chars,
                );
                Ok(json!({
                    "workspaceRef": SARAH_VOICE_WORKSPACE_REF,
                    "documentVersion": document_version,
                    "file": project_path
                        .as_ref()
                        .map(|path| path.path.as_ref().as_unix_str()),
                    "title": editor.title(cx),
                    "cursor": { "line": cursor.row, "column": cursor.column },
                    "selection": truncate_chars(selected_text, max_chars),
                    "context": context,
                }))
            }),
            SarahEditorCommand::Navigate { line, column } => {
                let point = editor.update(cx, |editor, cx| {
                    let snapshot = editor.buffer().read(cx).snapshot(cx);
                    let point = snapshot.clip_point(Point::new(line, column), Bias::Left);
                    editor.change_selections(
                        SelectionEffects::scroll(Autoscroll::center()),
                        window,
                        cx,
                        |selections| selections.select_ranges([point..point]),
                    );
                    point
                });
                Ok(json!({ "line": point.row, "column": point.column }))
            }
            SarahEditorCommand::Insert { text } => {
                let inserted_chars = text.chars().count();
                editor.update(cx, |editor, cx| {
                    let display_snapshot = editor.display_snapshot(cx);
                    let cursor = editor.selections.newest::<Point>(&display_snapshot).head();
                    editor.change_selections(
                        SelectionEffects::scroll(Autoscroll::fit()),
                        window,
                        cx,
                        |selections| selections.select_ranges([cursor..cursor]),
                    );
                    editor.insert(&text, window, cx);
                });
                Ok(json!({ "insertedChars": inserted_chars }))
            }
            SarahEditorCommand::ReplaceSelection { text } => {
                let replacement_chars = text.chars().count();
                let target = target
                    .as_ref()
                    .ok_or_else(|| refuse_voice_command("The replacement omitted its target."))?;
                let binding = effect_binding.as_ref().ok_or_else(|| {
                    refuse_voice_command("The replacement omitted its confirmed editor state.")
                })?;
                editor.update(cx, |editor, cx| {
                    let display_snapshot = editor.display_snapshot(cx);
                    let selection = editor.selections.newest::<Point>(&display_snapshot);
                    let range = selection.range();
                    let buffer_snapshot = editor.buffer().read(cx).snapshot(cx);
                    let document_version = buffer_snapshot
                        .as_singleton()
                        .map(|snapshot| voice_document_version(snapshot))
                        .ok_or_else(|| {
                            refuse_voice_command(
                                "The active editor is no longer a single document.",
                            )
                        })?;
                    let current_path = project_path
                        .as_ref()
                        .map(|path| path.path.as_ref().as_unix_str().to_string())
                        .ok_or_else(|| {
                            refuse_voice_command("The active editor no longer has a saved path.")
                        })?;
                    let selected_text = buffer_snapshot
                        .text_for_range(range.clone())
                        .collect::<String>();
                    validate_voice_selection_effect(
                        binding,
                        target,
                        &current_path,
                        &document_version,
                        voice_text_point(range.start),
                        voice_text_point(range.end),
                        &selected_text,
                        &text,
                    )
                    .map_err(refuse_voice_command)?;
                    editor.insert(&text, window, cx);
                    Ok::<(), anyhow::Error>(())
                })?;
                Ok(json!({ "replacementChars": replacement_chars }))
            }
            SarahEditorCommand::Action { action } => {
                match action {
                    ApprovedEditorAction::Undo => {
                        editor.update(cx, |editor, cx| {
                            editor.undo(&editor_actions::Undo, window, cx)
                        });
                    }
                    ApprovedEditorAction::Redo => {
                        editor.update(cx, |editor, cx| {
                            editor.redo(&editor_actions::Redo, window, cx)
                        });
                    }
                    ApprovedEditorAction::SaveActiveFile => {
                        window.dispatch_action(Box::new(Save { save_intent: None }), cx);
                    }
                }
                Ok(json!({ "action": action }))
            }
            SarahEditorCommand::StartAgentThread { .. } => {
                anyhow::bail!("agent-thread commands must use the Agent panel bridge")
            }
        }
    }

    fn start_agent_thread(
        &mut self,
        workspace: Entity<Workspace>,
        message: String,
        presentation: AgentThreadPresentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value> {
        let agent_panel = workspace
            .read(cx)
            .panel::<AgentPanel>(cx)
            .context("the Omega Agent panel is unavailable")?;
        let reveal = presentation == AgentThreadPresentation::Foreground;
        let thread_id = agent_panel
            .update(cx, |agent_panel, cx| {
                agent_panel.create_omega_thread_with_message(message, reveal, window, cx)
            })
            .context("open a project before asking Sarah to start an Agent thread")?;

        if reveal {
            workspace.update(cx, |workspace, cx| {
                workspace.focus_panel::<AgentPanel>(window, cx);
            });
        }

        let status: SharedString = match presentation {
            AgentThreadPresentation::Foreground => {
                "Message submitted to a new Omega Agent thread and opened.".into()
            }
            AgentThreadPresentation::Background => {
                "Message submitted to a new Omega Agent thread in the background.".into()
            }
        };
        self.voice_status = status.clone();
        self.created_agent_thread = Some(SarahCreatedAgentThread {
            thread_id,
            presentation,
            status,
        });
        self.publish_active_voice_artifacts(cx);
        Ok(json!({
            "threadId": thread_id.to_key_string(),
            "presentation": presentation,
            "status": "submitted",
        }))
    }

    fn open_created_agent_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(created) = self.created_agent_thread.clone() else {
            return;
        };
        let result = self
            .workspace
            .upgrade()
            .context("the workspace is no longer available")
            .and_then(|workspace| {
                let agent_panel = workspace
                    .read(cx)
                    .panel::<AgentPanel>(cx)
                    .context("the Omega Agent panel is unavailable")?;
                let opened = agent_panel.update(cx, |agent_panel, cx| {
                    agent_panel.reveal_omega_thread(created.thread_id, window, cx)
                });
                anyhow::ensure!(opened, "the created Agent thread is no longer available");
                workspace.update(cx, |workspace, cx| {
                    workspace.focus_panel::<AgentPanel>(window, cx);
                });
                Ok(())
            });
        match result {
            Ok(()) => {
                let status: SharedString = "Agent thread opened in the Agent panel.".into();
                self.voice_status = status.clone();
                if let Some(created) = &mut self.created_agent_thread {
                    created.status = status;
                }
                self.publish_active_voice_artifacts(cx);
                cx.notify();
            }
            Err(error) => {
                self.voice_status = format!("Could not open the Agent thread: {error:#}").into();
                cx.notify();
            }
        }
    }

    fn open_audio_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.dispatch_action(
            Box::new(OpenSettingsPage {
                page: "Collaboration".into(),
                target: None,
            }),
            cx,
        );
    }

    /// Test / inspection helper: current in-memory projection (not durable).
    pub fn projection(&self) -> &WorkroomProjection {
        &self.projection
    }

    /// Test / inspection helper: community room projection (not durable).
    pub fn community(&self) -> &CommunityRoomProjection {
        &self.community
    }

    /// Test / inspection helper: active room kind.
    pub fn active_room(&self) -> RoomKind {
        self.active_room
    }

    /// SARAH-CW-08: switch rooms inside the same pane (not a second dock panel).
    pub fn select_room(&mut self, kind: RoomKind, cx: &mut Context<Self>) {
        self.active_room = kind;
        self.status = match kind {
            RoomKind::OwnerPrivate => "Showing owner-private Sarah room.".into(),
            RoomKind::Community => {
                "Showing community room (separate membership and history).".into()
            }
        };
        // Composer stays one instance; community publish is not wired in this skeleton.
        if kind.is_community() {
            // Placeholder reminds the operator which room is active.
            // Full community compose is a later packet; do not invent a second Editor.
        }
        cx.notify();
    }

    /// Two-room isolation check for tests and honest UI guards.
    pub fn rooms_are_isolated(&self) -> bool {
        // Distinct identities when both known.
        if let (Some(thread), Some(group)) = (
            self.projection.room.thread_ref.as_deref(),
            self.community.room.group_ref.as_deref().or(self
                .community
                .membership
                .group_ref
                .as_deref()),
        ) {
            if thread == group {
                return false;
            }
        }
        let owner_refs: std::collections::BTreeSet<&str> = self
            .projection
            .transcript
            .rows
            .iter()
            .map(|r| r.message_ref.as_str())
            .collect();
        let community_refs: std::collections::BTreeSet<&str> = self
            .community
            .transcript
            .rows
            .iter()
            .map(|r| r.message_ref.as_str())
            .collect();
        owner_refs.is_disjoint(&community_refs)
    }

    /// Test / inspection helper: interaction state (not durable).
    pub fn interaction(&self) -> &InteractionState {
        &self.interaction
    }
}

fn parse_run_phase(s: &str) -> RunPhase {
    if s.starts_with("turn.") {
        return RunPhase::from_event_kind(s);
    }
    match s {
        "queued" => RunPhase::Queued,
        "running" => RunPhase::Running,
        "interrupted" => RunPhase::Interrupted,
        "interrupt_pending" => RunPhase::Running,
        "finished" | "completed" => RunPhase::Finished,
        "idle" => RunPhase::Idle,
        _ => RunPhase::Unknown,
    }
}

fn string_field(value: Option<&Value>, keys: &[&str]) -> Option<String> {
    let value = value?;
    for key in keys {
        if let Some(s) = value.get(*key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn truncate_chars(text: String, max_chars: usize) -> String {
    let Some((byte_offset, _)) = text.char_indices().nth(max_chars) else {
        return text;
    };
    text[..byte_offset].to_string()
}

impl EventEmitter<PanelEvent> for SarahWorkroomPanel {}

impl Drop for SarahWorkroomPanel {
    fn drop(&mut self) {
        if let Some(controls) = self.voice_controls.take()
            && controls.try_send(SarahVoiceControl::Close).is_err()
        {
            log::debug!("Sarah voice control channel was already closed while dropping the panel");
        }
        self.voice_task.take();
        self.voice_transcript_pacing_task.take();
    }
}

impl Focusable for SarahWorkroomPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for SarahWorkroomPanel {
    fn persistent_name() -> &'static str {
        "SarahWorkroomPanel"
    }

    fn panel_key() -> &'static str {
        PANEL_KEY
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        DockPosition::Right
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, _: DockPosition, _: &mut Window, _: &mut Context<Self>) {}

    fn default_size(&self, _: &Window, _: &App) -> gpui::Pixels {
        px(440.)
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::OmegaAgent)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        // Dock tooltip may describe the surface; the in-pane header is only "Sarah".
        Some("Sarah")
    }

    fn icon_label(&self, _: &Window, _: &App) -> Option<String> {
        // One unread count for the room (OMEGA-SW-06).
        self.projection.attention.icon_label()
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(OpenPanel)
    }

    fn activation_priority(&self) -> u32 {
        10
    }
}

impl Render for SarahWorkroomPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.projection;
        if self.public_demo {
            return v_flex()
                .id("sarah-workroom-panel")
                .size_full()
                .track_focus(&self.focus_handle)
                .gap_3()
                .p_3()
                .child(
                    h_flex()
                        .justify_between()
                        .child(Label::new("Sarah").size(LabelSize::Large))
                        .child(
                            Label::new("PUBLIC DEMO")
                                .color(Color::Accent)
                                .size(LabelSize::Small),
                        ),
                )
                .child(Label::new(self.status.clone()).color(Color::Muted))
                .child(Label::new("Orbit Notes launch room").size(LabelSize::Large))
                .child(
                    Label::new("Product engineering · completed · 12 checks passed")
                        .color(Color::Success),
                )
                .child(Label::new("Conversation").color(Color::Muted))
                .child(
                    v_flex()
                        .id("sarah-workroom-transcript")
                        .border_1()
                        .border_color(cx.theme().colors().border)
                        .rounded_md()
                        .p_2()
                        .child(transcript_body(&p.transcript)),
                )
                .child(Label::new("Recent activity").color(Color::Muted))
                .child(
                    v_flex()
                        .id("sarah-workroom-activity")
                        .border_1()
                        .border_color(cx.theme().colors().border)
                        .rounded_md()
                        .p_2()
                        .child(activity_body(&p.activity)),
                )
                .child(
                    Label::new("Offline fixture · no account, secret, or private path")
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                );
        }
        let community = &self.community;
        let active = self.active_room;
        let showing_community = active.is_community();
        let can_interrupt = !showing_community
            && !self.interrupting
            && matches!(
                p.run_state.phase,
                RunPhase::Running | RunPhase::Queued | RunPhase::Unknown
            );
        // One composer for the pane; community publish is not wired in CW-08 skeleton.
        let can_send = !showing_community && !self.sending;
        let answer = self.interaction.answer.clone();
        let terminal = self.interaction.terminal.clone();
        let honest = self.interaction.uses_honest_liveness();
        let header = active.header();
        let device_grants = device_grants_body(
            &self.device_grants,
            self.device_grants_detail.as_deref(),
            self.grant_busy.as_deref(),
            cx,
        );
        let voice_is_active = self.voice_state.is_active();
        let voice_can_control_audio = self.voice_controls.is_some()
            && voice_is_active
            && self.voice_state != SarahVoiceState::Reconnecting;
        let voice_can_end = self.voice_controls.is_some()
            || voice_is_active
            || self.voice_state == SarahVoiceState::Ending;
        let voice_status = self.voice_status.clone();
        let voice_state_label = if self.voice_access_required {
            "Voice credits required"
        } else {
            self.voice_state.label()
        };
        let voice_session_id = self.voice_session_id.clone();
        let pending_voice_command = self.pending_voice_command.clone();
        let created_agent_thread = self.created_agent_thread.clone();
        let voice_section = v_flex()
            .id("sarah-managed-voice")
            .gap_2()
            .border_1()
            .border_color(cx.theme().colors().border)
            .rounded_md()
            .p_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Live voice").size(LabelSize::Large))
                    .child(
                        Label::new(voice_state_label)
                            .color(if self.voice_state == SarahVoiceState::Error {
                                Color::Error
                            } else if voice_is_active {
                                Color::Accent
                            } else {
                                Color::Muted
                            })
                            .size(LabelSize::Small),
                    ),
            )
            .child(
                Label::new(voice_status).color(if self.voice_state == SarahVoiceState::Error {
                    Color::Error
                } else {
                    Color::Muted
                }),
            )
            .child(
                Label::new("Managed by OpenAgents · OpenAI Realtime gpt-realtime-2.1 · no API key")
                    .color(Color::Muted)
                    .size(LabelSize::Small),
            )
            .when_some(voice_session_id, |this, session_id| {
                this.child(
                    Label::new(format!("session={session_id}"))
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                )
            })
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        Button::new("sarah-voice-start", "Start voice")
                            .style(ButtonStyle::Filled)
                            .disabled(
                                voice_is_active
                                    || self.voice_state == SarahVoiceState::Authenticating
                                    || self.voice_state == SarahVoiceState::Ending,
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_voice(window, cx);
                            })),
                    )
                    .child(
                        Button::new(
                            "sarah-voice-mute",
                            if self.voice_muted { "Unmute" } else { "Mute" },
                        )
                        .style(ButtonStyle::Subtle)
                        .disabled(!voice_can_control_audio)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_voice_mute(cx))),
                    )
                    .child(
                        Button::new("sarah-voice-interrupt", "Interrupt speech")
                            .style(ButtonStyle::Subtle)
                            .disabled(!voice_can_control_audio)
                            .on_click(cx.listener(|this, _, _, cx| this.interrupt_voice(cx))),
                    )
                    .child(
                        Button::new("sarah-voice-end", "End voice")
                            .style(ButtonStyle::Subtle)
                            .disabled(!voice_can_end)
                            .on_click(cx.listener(|this, _, _, cx| this.end_voice(cx))),
                    )
                    .when(self.voice_retryable, |this| {
                        this.child(
                            Button::new("sarah-voice-retry", "Retry")
                                .style(ButtonStyle::Filled)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.retry_voice(window, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("sarah-voice-audio-settings", "Audio settings")
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_audio_settings(window, cx);
                            })),
                    ),
            )
            .when_some(pending_voice_command, |this, request| {
                this.child(
                    v_flex()
                        .id("sarah-voice-command-confirmation")
                        .gap_1()
                        .border_1()
                        .border_color(cx.theme().colors().border)
                        .rounded_md()
                        .p_2()
                        .child(Label::new("Sarah requests confirmation").color(Color::Warning))
                        .child(Label::new(request.command.confirmation_copy()))
                        .when_some(
                            request.command.confirmation_detail().map(ToOwned::to_owned),
                            |this, message| {
                                this.child(
                                    v_flex()
                                        .id("sarah-agent-message-confirmation")
                                        .max_h(px(160.))
                                        .overflow_y_scroll()
                                        .border_1()
                                        .border_color(cx.theme().colors().border)
                                        .rounded_md()
                                        .p_2()
                                        .child(
                                            Label::new("Message to submit")
                                                .color(Color::Muted)
                                                .size(LabelSize::Small),
                                        )
                                        .child(Label::new(message)),
                                )
                            },
                        )
                        .when_some(request.effect_binding, |this, binding| {
                            let selection = format!(
                                "{}:{}–{}:{} (1-based)",
                                binding.selection_start.line.saturating_add(1),
                                binding.selection_start.column.saturating_add(1),
                                binding.selection_end.line.saturating_add(1),
                                binding.selection_end.column.saturating_add(1),
                            );
                            this.child(
                                v_flex()
                                    .id("sarah-voice-selection-effect")
                                    .gap_1()
                                    .child(Label::new(format!(
                                        "Target: {} · Selection: {}",
                                        binding.target_path, selection
                                    )))
                                    .child(
                                        Label::new(format!(
                                            "Workspace: {} · Document version: {}",
                                            binding.workspace_ref, binding.document_version
                                        ))
                                        .color(Color::Muted)
                                        .size(LabelSize::XSmall),
                                    )
                                    .child(
                                        Label::new("Selected text")
                                            .color(Color::Muted)
                                            .size(LabelSize::Small),
                                    )
                                    .child(
                                        v_flex()
                                            .id("sarah-voice-selection-before")
                                            .max_h(px(160.))
                                            .overflow_y_scroll()
                                            .child(Label::new(binding.selected_text)),
                                    )
                                    .child(
                                        Label::new("Replacement")
                                            .color(Color::Muted)
                                            .size(LabelSize::Small),
                                    )
                                    .child(
                                        v_flex()
                                            .id("sarah-voice-selection-after")
                                            .max_h(px(160.))
                                            .overflow_y_scroll()
                                            .child(Label::new(binding.replacement_text)),
                                    ),
                            )
                        })
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new("sarah-voice-command-approve", "Allow once")
                                        .style(ButtonStyle::Filled)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.approve_voice_command(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("sarah-voice-command-reject", "Decline")
                                        .style(ButtonStyle::Subtle)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.reject_voice_command(cx);
                                        })),
                                ),
                        ),
                )
            })
            .when_some(created_agent_thread, |this, created| {
                let presentation = match created.presentation {
                    AgentThreadPresentation::Foreground => "Foreground · opened in Agent panel",
                    AgentThreadPresentation::Background => {
                        "Background · active view and focus were preserved"
                    }
                };
                this.child(
                    v_flex()
                        .id("sarah-created-agent-thread")
                        .gap_1()
                        .border_1()
                        .border_color(cx.theme().colors().border)
                        .rounded_md()
                        .p_2()
                        .child(Label::new("Omega Agent thread").color(Color::Accent))
                        .child(
                            Label::new(created.thread_id.to_key_string())
                                .color(Color::Muted)
                                .size(LabelSize::Small),
                        )
                        .child(Label::new(presentation).size(LabelSize::Small))
                        .child(Label::new(created.status))
                        .child(
                            Button::new("sarah-open-created-agent-thread", "Open Agent thread")
                                .style(ButtonStyle::Filled)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_created_agent_thread(window, cx);
                                })),
                        ),
                )
            })
            .child(Label::new("Voice transcript").color(Color::Muted))
            .child(
                v_flex()
                    .id("sarah-voice-transcript")
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .p_2()
                    .child(voice_transcript_body(
                        &self.voice_transcript,
                        &self.voice_transcript_presentation,
                        cx.reduce_motion(),
                    )),
            );

        v_flex()
            .id("sarah-workroom-panel")
            .size_full()
            .track_focus(&self.focus_handle)
            .gap_2()
            .p_3()
            // Active room header must be unmistakable (Sarah vs Community).
            .child(Label::new(header).size(LabelSize::Large))
            .when(showing_community, |this| {
                this.child(
                    Label::new(COMMUNITY_ROOM_SUBTITLE)
                        .color(Color::Accent)
                        .size(LabelSize::Small),
                )
            })
            .child(Label::new(self.status.clone()).color(Color::Muted))
            // SARAH-CW-08: room switcher — same pane, two rooms.
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("workroom-room-owner-private", OWNER_PRIVATE_ROOM_HEADER)
                            .style(if showing_community {
                                ButtonStyle::Subtle
                            } else {
                                ButtonStyle::Filled
                            })
                            .disabled(!showing_community)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_room(RoomKind::OwnerPrivate, cx);
                            })),
                    )
                    .child(
                        Button::new("workroom-room-community", COMMUNITY_ROOM_HEADER)
                            .style(if showing_community {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Subtle
                            })
                            .disabled(showing_community)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_room(RoomKind::Community, cx);
                            })),
                    ),
            )
            .when(!showing_community, |this| {
                this.when_some(p.connection_detail.clone(), |this, detail| {
                    this.child(Label::new(detail).color(Color::Muted))
                })
                .when(honest, |this| {
                    this.child(
                        Label::new("Liveness: ordered tool ladder (no token stream).")
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                    )
                })
            })
            .when(showing_community, |this| {
                this.when_some(community.connection_detail.clone(), |this, detail| {
                    this.child(Label::new(detail).color(Color::Muted))
                })
                .child(
                    Label::new(V1_NO_PAY_ROOM_DESCRIPTION)
                        .color(Color::Warning)
                        .size(LabelSize::Small),
                )
                .child(
                    Label::new(V1_NO_PAY_FIRST_RUN_COPY)
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                )
            })
            // OMEGA-SW-01: visible binding state (unbound | bound | refused).
            .child(binding_section(&self.binding_projection))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(
                            "sarah-workroom-bind",
                            if self.binding_busy {
                                "Binding…"
                            } else {
                                "Bind OpenAgents account"
                            },
                        )
                        .style(ButtonStyle::Subtle)
                        .disabled(
                            self.binding_busy
                                || self.binding_projection.state == BindingState::Bound,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.bind_openagents_account(cx))),
                    )
                    .child(
                        Button::new("sarah-workroom-unbind", "Clear binding")
                            .style(ButtonStyle::Subtle)
                            .disabled(
                                self.binding_busy
                                    || self.binding_projection.state == BindingState::Unbound,
                            )
                            .on_click(
                                cx.listener(|this, _, _, cx| this.clear_openagents_binding(cx)),
                            ),
                    ),
            )
            // --- Owner-private Sarah room ---
            .when(!showing_community, |this| {
                this.child(attention_body(&p.attention))
                    .child(voice_section)
                    .child(section_header("Room", &p.room.meta))
                    .child(room_body(&p.room))
                    .child(section_header("Transcript", &p.transcript.meta))
                    .child(
                        v_flex()
                            .id("sarah-workroom-transcript")
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .rounded_md()
                            .p_2()
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .child(transcript_body(&p.transcript)),
                    )
                    .child(section_header("Activity (tool ladder)", &p.activity.meta))
                    .child(
                        v_flex()
                            .id("sarah-workroom-activity")
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .rounded_md()
                            .p_2()
                            .max_h(px(120.))
                            .overflow_y_scroll()
                            .child(activity_body(&p.activity)),
                    )
                    .child(Label::new("Answer").color(Color::Muted))
                    .child(answer_body(&answer))
                    .child(section_header("Receipts", &p.receipts.meta))
                    .child(receipts_body(&p.receipts))
                    .child(section_header("Run state", &p.run_state.meta))
                    .child(run_state_body(&p.run_state, &terminal))
                    .child(Label::new("Full Auto work").color(Color::Muted))
                    .child(full_auto_body(
                        &self.full_auto_runs,
                        self.full_auto_detail.as_deref(),
                    ))
                    .child(Label::new("Paired devices").color(Color::Muted))
                    .child(device_grants)
            })
            // --- Community room (SARAH-CW-08) — same pane, separate history ---
            .when(showing_community, |this| {
                this.child(section_header("Community group", &community.room.meta))
                    .child(community_room_body(community))
                    .child(section_header("Membership", &community.membership.meta))
                    .child(membership_body(&community.membership))
                    .child(section_header("Work units", &community.work_units.meta))
                    .child(work_units_body(&community.work_units))
                    .child(section_header(
                        // Never "earnings" — experience only.
                        "Experience rank",
                        &community.experience.meta,
                    ))
                    .child(experience_body(&community.experience))
                    .child(section_header(
                        "Group transcript",
                        &community.transcript.meta,
                    ))
                    .child(
                        v_flex()
                            .id("community-workroom-transcript")
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .rounded_md()
                            .p_2()
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .child(transcript_body(&community.transcript)),
                    )
            })
            .child(Label::new("Confirmed Nostr records").color(Color::Muted))
            .child(
                v_flex()
                    .id("workroom-confirmed-nostr-records")
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .p_2()
                    .max_h(px(140.))
                    .overflow_y_scroll()
                    .child(nostr_records_body(&self.nostr_records)),
            )
            // One composer for the pane (not a second composer).
            .child(
                Label::new(if showing_community {
                    "Composer (community publish not wired — skeleton)"
                } else {
                    "Composer"
                })
                .color(Color::Muted),
            )
            .child(
                div()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .h(px(72.))
                    .child(self.composer.clone()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(
                            "sarah-workroom-send",
                            if self.sending { "Sending…" } else { "Send" },
                        )
                        .style(ButtonStyle::Filled)
                        .disabled(!can_send)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.send_message(window, cx);
                        })),
                    )
                    .child(
                        Button::new("sarah-workroom-refresh", "Refresh")
                            .style(ButtonStyle::Subtle)
                            .disabled(self.refreshing || showing_community)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_from_effectd(cx))),
                    )
                    .child(
                        Button::new("sarah-workroom-mark-read", "Mark read")
                            .style(ButtonStyle::Subtle)
                            .disabled(showing_community || p.attention.unread_count == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.mark_room_read(cx))),
                    )
                    .child(
                        Button::new(
                            "sarah-workroom-interrupt",
                            if self.interrupting {
                                "Interrupting…"
                            } else if p.run_state.interrupt_intent == InterruptIntentState::Pending
                            {
                                "Interrupt pending"
                            } else {
                                "Interrupt"
                            },
                        )
                        .style(ButtonStyle::Subtle)
                        .disabled(
                            !can_interrupt
                                && p.run_state.interrupt_intent != InterruptIntentState::Pending,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.interrupt_turn(cx))),
                    ),
            )
    }
}

fn device_grants_body(
    grants: &[Issue31GrantProjection],
    detail: Option<&str>,
    busy_grant_ref: Option<&str>,
    cx: &mut Context<SarahWorkroomPanel>,
) -> impl IntoElement + use<> {
    v_flex()
        .id("sarah-workroom-device-grants")
        .gap_1()
        .border_1()
        .rounded_md()
        .p_2()
        .when_some(detail.map(str::to_string), |this, detail| {
            this.child(Label::new(detail).color(Color::Muted))
        })
        .children(grants.iter().cloned().enumerate().map(|(index, grant)| {
            let renew_grant = grant.clone();
            let revoke_grant_ref = grant.grant_ref.clone();
            let readmit_grant_ref = grant.grant_ref.clone();
            let busy = busy_grant_ref.is_some();
            let active = grant.status == "active";
            let revoked = grant.status == "revoked";
            v_flex()
                .gap_0p5()
                .child(Label::new(format!(
                    "Device {} · {} · generation {}",
                    grant.device_fingerprint, grant.status, grant.generation
                )))
                .child(
                    Label::new(format!(
                        "grant={} · expires={} · scopes={:?}",
                        grant.grant_ref,
                        grant
                            .expires_at
                            .map(|expires_at| expires_at.to_string())
                            .unwrap_or_else(|| "none".into()),
                        grant.scopes
                    ))
                    .color(Color::Muted)
                    .size(LabelSize::Small),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new(("renew-device-grant", index), "Renew 24h")
                                .style(ButtonStyle::Subtle)
                                .disabled(busy || !active)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.renew_device_grant(renew_grant.clone(), cx);
                                })),
                        )
                        .child(
                            Button::new(("revoke-device-grant", index), "Revoke")
                                .style(ButtonStyle::Subtle)
                                .disabled(busy || revoked)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.revoke_device_grant(revoke_grant_ref.clone(), cx);
                                })),
                        )
                        .child(
                            Button::new(("readmit-device", index), "Re-admit device")
                                .style(ButtonStyle::Subtle)
                                .disabled(busy || !revoked)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.readmit_device(readmit_grant_ref.clone(), cx);
                                })),
                        ),
                )
        }))
}

fn full_auto_body(runs: &[WorkroomFullAutoRun], detail: Option<&str>) -> impl IntoElement {
    v_flex()
        .id("sarah-workroom-full-auto")
        .gap_1()
        .border_1()
        .rounded_md()
        .p_2()
        .when_some(detail.map(str::to_string), |this, detail| {
            this.child(Label::new(detail).color(Color::Muted))
        })
        .children(runs.iter().map(|run| {
            v_flex()
                .gap_0p5()
                .child(Label::new(format!(
                    "{} · {} · {}",
                    run.objective, run.lane, run.state
                )))
                .child(
                    Label::new(format!("Unattended: {}", run.exact_unattended_duration))
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                )
                .when_some(run.latest_turn.clone(), |this, turn| {
                    this.child(Label::new(format!("Live: {turn}")).color(Color::Accent))
                })
                .when_some(run.terminal_reason.clone(), |this, reason| {
                    this.child(Label::new(format!("Outcome: {reason}")).color(Color::Warning))
                })
        }))
}

fn community_room_body(community: &CommunityRoomProjection) -> impl IntoElement {
    let room = &community.room;
    v_flex()
        .gap_0p5()
        .child(Label::new(format!(
            "group={}",
            room.group_ref.as_deref().unwrap_or("(missing)")
        )))
        .child(Label::new(format!(
            "name={}",
            room.display_name.as_deref().unwrap_or("(missing)")
        )))
        .child(
            Label::new(format!("invitation_only={}", room.invitation_only))
                .color(Color::Muted)
                .size(LabelSize::Small),
        )
        .child(
            Label::new(room.description.clone())
                .color(Color::Muted)
                .size(LabelSize::Small),
        )
        .when_some(room.detail.clone(), |this, detail| {
            this.child(
                Label::new(detail)
                    .color(Color::Warning)
                    .size(LabelSize::Small),
            )
        })
}

fn membership_body(membership: &crate::community::MembershipProjection) -> impl IntoElement {
    if membership.members.is_empty() {
        return v_flex().child(
            Label::new(
                membership
                    .detail
                    .clone()
                    .unwrap_or_else(|| "No members projected.".into()),
            )
            .color(Color::Muted)
            .size(LabelSize::Small),
        );
    }
    let mut col = v_flex().gap_0p5();
    for member in &membership.members {
        let agents = member
            .agents
            .iter()
            .map(|a| {
                format!(
                    "{ref}{attested}{revoked}",
                    ref = a.agent_ref,
                    attested = if a.attested { "·attested" } else { "" },
                    revoked = if a.revoked { "·revoked" } else { "" },
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        col = col.child(Label::new(format!(
            "{name} ({mref}) attested={attested} agents=[{agents}]",
            name = member.display_name.as_deref().unwrap_or("member"),
            mref = member.member_ref,
            attested = member.attested,
            agents = agents,
        )));
    }
    if membership.truncated {
        col = col.child(Label::new("…roster truncated").color(Color::Muted));
    }
    col
}

fn work_units_body(units: &crate::community::WorkUnitsProjection) -> impl IntoElement {
    if units.units.is_empty() {
        return v_flex().child(
            Label::new(
                units
                    .detail
                    .clone()
                    .unwrap_or_else(|| "No work units projected.".into()),
            )
            .color(Color::Muted)
            .size(LabelSize::Small),
        );
    }
    let mut col = v_flex().gap_0p5();
    for unit in &units.units {
        col = col.child(Label::new(format!(
            "{title} ({uref}) · {acceptance} · quotes={q}{tier}{reward}",
            title = unit.title,
            uref = unit.unit_ref,
            acceptance = unit.acceptance.label(),
            q = unit.quotes.len(),
            tier = unit
                .tier
                .map(|t| format!(" · tier={t}"))
                .unwrap_or_default(),
            reward = unit
                .reward_note
                .as_ref()
                .map(|n| format!(" · {n}"))
                .unwrap_or_default(),
        )));
    }
    if units.truncated {
        col = col.child(Label::new("…work units truncated").color(Color::Muted));
    }
    col
}

fn experience_body(experience: &crate::community::ExperienceRankProjection) -> impl IntoElement {
    // Structural: label is experience, never earnings.
    let summary = experience.summary_line();
    v_flex()
        .gap_0p5()
        .child(Label::new(summary))
        .child(
            Label::new(format!(
                "reward_label={label} (not {forbidden})",
                label = experience.reward_label,
                forbidden = crate::community::FORBIDDEN_EARNINGS_LABEL,
            ))
            .color(Color::Muted)
            .size(LabelSize::Small),
        )
        .when(experience.recent_awards.is_empty(), |this| {
            this.child(
                Label::new(
                    experience
                        .detail
                        .clone()
                        .unwrap_or_else(|| format!("No {EXPERIENCE_LABEL} awards projected.")),
                )
                .color(Color::Muted)
                .size(LabelSize::Small),
            )
        })
        .children(experience.recent_awards.iter().map(|award| {
            Label::new(format!(
                "+{pts} {kind} ({aref})",
                pts = award.points,
                kind = award.reason_kind,
                aref = award.award_ref,
            ))
        }))
}

fn binding_section(binding: &BindingProjection) -> impl IntoElement {
    // Projection is public-safe: never render tokens or credential material.
    let state_line = format!("binding={}", binding.state.label());
    let account_line = binding
        .openagents_account_id
        .as_ref()
        .map(|id| format!("account={id}"));
    let gate = binding.gate_message.clone();
    v_flex()
        .gap_0p5()
        .child(Label::new("OpenAgents binding").color(Color::Muted))
        .child(Label::new(state_line))
        .when_some(account_line, |this, line| {
            this.child(Label::new(line).color(Color::Muted).size(LabelSize::Small))
        })
        .when_some(gate, |this, message| {
            this.child(
                Label::new(message)
                    .color(Color::Warning)
                    .size(LabelSize::Small),
            )
        })
}

fn attention_body(attention: &crate::attention::RoomAttention) -> impl IntoElement {
    let marker_color = if attention.marker == AttentionMarker::NeedsAttention {
        Color::Accent
    } else {
        Color::Muted
    };
    let tick_note = attention.tick_note.map(|s| s.to_string());
    v_flex()
        .id("sarah-workroom-attention")
        .gap_0p5()
        .child(Label::new(attention.summary_line()).color(marker_color))
        .when_some(tick_note, |this, note| {
            this.child(Label::new(note).color(Color::Muted).size(LabelSize::Small))
        })
}

fn section_header(title: &'static str, meta: &ProjectionMeta) -> impl IntoElement {
    v_flex().gap_0p5().child(Label::new(title)).child(
        Label::new(meta.summary_line())
            .color(Color::Muted)
            .size(LabelSize::Small),
    )
}

fn room_body(room: &RoomProjection) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .when_some(room.detail.clone(), |this, detail| {
            this.child(Label::new(detail).color(Color::Warning))
        })
        .when_some(room.display_name.clone(), |this, name| {
            this.child(Label::new(format!("name={name}")))
        })
        .when_some(room.principal_ref.clone(), |this, r| {
            this.child(Label::new(format!("principal={r}")).color(Color::Muted))
        })
        .when_some(room.role.clone(), |this, role| {
            this.child(Label::new(format!("role={role}")).color(Color::Muted))
        })
        .when_some(room.thread_ref.clone(), |this, t| {
            this.child(Label::new(format!("thread={t}")).color(Color::Muted))
        })
        .when_some(room.authority_profile.clone(), |this, profile| {
            this.child(
                Label::new(format!(
                    "authority={} rev={}",
                    profile,
                    room.authority_revision.as_deref().unwrap_or("—")
                ))
                .color(Color::Muted),
            )
        })
        .when(
            room.display_name.is_none() && room.principal_ref.is_none() && room.detail.is_none(),
            |this| this.child(Label::new("No room fields.").color(Color::Muted)),
        )
}

fn transcript_body(transcript: &TranscriptProjection) -> impl IntoElement {
    if transcript.rows.is_empty() {
        return v_flex().child(
            Label::new(if transcript.meta.gap == GapState::Unavailable {
                "Transcript source unavailable (not an empty success)."
            } else {
                "No messages in page."
            })
            .color(Color::Muted),
        );
    }
    let mut col = v_flex().gap_1();
    for row in &transcript.rows {
        let line = format!(
            "[{}] {} · {}: {}",
            row.ack.label(),
            row.message_ref,
            row.role,
            row.text
        );
        let color = if row.ack == MessageAck::Pending {
            Color::Warning
        } else {
            Color::Default
        };
        col = col.child(Label::new(line).color(color));
    }
    if transcript.truncated {
        col = col.child(
            Label::new("(earlier rows truncated at capacity bound)")
                .color(Color::Muted)
                .size(LabelSize::Small),
        );
    }
    col
}

fn voice_transcript_body(
    transcript: &[VoiceTranscriptItem],
    presentation: &VoiceTranscriptPresentation,
    reduce_motion: bool,
) -> impl IntoElement {
    if transcript.is_empty() {
        return v_flex().child(
            Label::new("Start voice to see live speech transcripts here.")
                .color(Color::Muted)
                .size(LabelSize::Small),
        );
    }
    let mut column = v_flex().gap_1();
    for item in transcript {
        let (text, fully_presented) =
            visible_voice_transcript_text(item, presentation, reduce_motion);
        let suffix = if item.complete && fully_presented {
            ""
        } else {
            " …"
        };
        column = column.child(Label::new(format!(
            "{}: {}{}",
            item.participant.label(),
            text,
            suffix
        )));
    }
    column
}

fn visible_voice_transcript_text(
    item: &VoiceTranscriptItem,
    presentation: &VoiceTranscriptPresentation,
    reduce_motion: bool,
) -> (String, bool) {
    let authoritative_char_count = item.text.chars().count();
    let visible_char_count = if reduce_motion {
        authoritative_char_count
    } else {
        presentation.visible_chars(item)
    };
    let text = if visible_char_count < authoritative_char_count {
        item.text.chars().take(visible_char_count).collect()
    } else {
        item.text.clone()
    };
    (text, visible_char_count == authoritative_char_count)
}

fn activity_body(activity: &ActivityProjection) -> impl IntoElement {
    if activity.rows.is_empty() {
        return v_flex().child(
            Label::new(if activity.meta.gap == GapState::Unavailable {
                "Activity source unavailable (not an empty success)."
            } else {
                "No activity events in page."
            })
            .color(Color::Muted),
        );
    }
    let mut col = v_flex().gap_1();
    for row in &activity.rows {
        col = col.child(Label::new(format!(
            "{} · {} · {}",
            row.kind, row.event_ref, row.summary
        )));
    }
    if activity.truncated {
        col = col.child(
            Label::new("(earlier activity truncated at capacity bound)")
                .color(Color::Muted)
                .size(LabelSize::Small),
        );
    }
    col
}

fn parse_nostr_records_projection(record_page: Option<&Value>) -> NostrRecordsProjection {
    let Some(record_page) = record_page else {
        return NostrRecordsProjection::unavailable(
            "Snapshot omitted confirmed Nostr record references.",
        );
    };
    let source =
        string_field(Some(record_page), &["source"]).unwrap_or_else(|| "confirmed_nostr".into());
    let mut rows = Vec::new();
    let mut truncated = false;
    if let Some(items) = record_page.get("entries").and_then(Value::as_array) {
        for item in items {
            if rows.len() == MAX_NOSTR_RECORD_ROWS {
                truncated = true;
                break;
            }
            let Some(event_id) = string_field(Some(item), &["eventId", "event_id"]) else {
                continue;
            };
            let Some(kind) = item
                .get("kind")
                .and_then(Value::as_u64)
                .and_then(|kind| u16::try_from(kind).ok())
            else {
                continue;
            };
            rows.push(NostrRecordRow {
                event_id,
                kind,
                record_kind: string_field(Some(item), &["recordKind", "record_kind"])
                    .unwrap_or_else(|| "record".into()),
                author_fingerprint: string_field(
                    Some(item),
                    &["authorFingerprint", "author_fingerprint"],
                )
                .unwrap_or_else(|| "UNKNOWN".into()),
                created_at: string_field(Some(item), &["createdAt", "created_at"])
                    .unwrap_or_else(|| "unknown".into()),
                source: string_field(Some(item), &["source"]).unwrap_or_else(|| source.clone()),
            });
        }
    }
    let gap = match string_field(Some(record_page), &["gapState", "gap_state"]).as_deref() {
        Some("none") => GapState::None,
        Some(_) => GapState::Gap,
        None => GapState::Unavailable,
    };
    NostrRecordsProjection {
        rows,
        cursor: string_field(Some(record_page), &["cursor"]),
        next_cursor: string_field(Some(record_page), &["nextCursor", "next_cursor"]),
        gap,
        source,
        detail: (gap == GapState::Unavailable)
            .then(|| "Confirmed Nostr record page omitted gap state.".into()),
        truncated,
    }
}

fn nostr_records_body(records: &NostrRecordsProjection) -> impl IntoElement {
    let mut column = v_flex().gap_1().child(
        Label::new(format!(
            "source={} · gap={} · cursor={} · next={}",
            records.source,
            records.gap.label(),
            records.cursor.as_deref().unwrap_or("missing"),
            records.next_cursor.as_deref().unwrap_or("end")
        ))
        .color(if records.gap == GapState::None {
            Color::Muted
        } else {
            Color::Warning
        })
        .size(LabelSize::Small),
    );
    if let Some(detail) = &records.detail {
        column = column.child(Label::new(detail.clone()).color(Color::Muted));
    }
    if records.rows.is_empty() {
        return column.child(
            Label::new(if records.gap == GapState::Unavailable {
                "Confirmed Nostr record source unavailable (not an empty success)."
            } else {
                "No confirmed AE, RS, ER, NIP-29, or LBR references in this page."
            })
            .color(Color::Muted),
        );
    }
    for row in &records.rows {
        column = column.child(Label::new(format!(
            "kind={} · {} · {} · author={} · {} · source={}",
            row.kind,
            row.record_kind,
            row.event_id,
            row.author_fingerprint,
            row.created_at,
            row.source
        )));
    }
    if records.truncated {
        column = column.child(
            Label::new("(confirmed record references truncated at capacity bound)")
                .color(Color::Muted)
                .size(LabelSize::Small),
        );
    }
    column
}

fn receipts_body(receipts: &ReceiptsProjection) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .when_some(receipts.detail.clone(), |this, detail| {
            this.child(Label::new(detail).color(Color::Muted))
        })
        .map(|this| {
            if receipts.rows.is_empty() {
                this.child(
                    Label::new(if receipts.meta.gap == GapState::Unavailable {
                        "No receipt refs (source unavailable)."
                    } else {
                        "No receipt refs in page."
                    })
                    .color(Color::Muted),
                )
            } else {
                let mut col = this;
                for row in &receipts.rows {
                    col = col.child(Label::new(format!(
                        "receipt={} allowed={} decision={} tool={}",
                        row.receipt_ref,
                        row.allowed
                            .map(|a| if a { "true" } else { "false" })
                            .unwrap_or("—"),
                        row.decision_ref.as_deref().unwrap_or("—"),
                        row.tool_ref.as_deref().unwrap_or("—"),
                    )));
                }
                col
            }
        })
}

fn answer_body(answer: &AnswerState) -> impl IntoElement {
    match answer {
        AnswerState::None => v_flex().child(
            Label::new("No answer block yet (stream:false; not a token stream).")
                .color(Color::Muted),
        ),
        AnswerState::Text { text } => v_flex()
            .gap_0p5()
            .child(
                Label::new("state=text (block arrived)")
                    .color(Color::Muted)
                    .size(LabelSize::Small),
            )
            .child(Label::new(text.clone())),
        AnswerState::Completed { text } => v_flex()
            .gap_0p5()
            .child(
                Label::new("state=completed")
                    .color(Color::Muted)
                    .size(LabelSize::Small),
            )
            .child(Label::new(text.clone())),
    }
}

fn run_state_body(run: &RunStateProjection, terminal: &TerminalOutcome) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .child(Label::new(format!(
            "phase={} · interrupt={}",
            run.phase.label(),
            run.interrupt_intent.label()
        )))
        .when_some(run.turn_ref.clone(), |this, t| {
            this.child(Label::new(format!("turn={t}")).color(Color::Muted))
        })
        .child(Label::new(format!("terminal={}", terminal.label())).color(Color::Muted))
        .when_some(
            terminal
                .reason()
                .map(|r| r.to_string())
                .or_else(|| run.reason.clone()),
            |this, reason| {
                this.child(Label::new(format!("reason={reason}")).color(
                    if terminal.is_terminal() {
                        Color::Warning
                    } else {
                        Color::Muted
                    },
                ))
            },
        )
}

#[cfg(test)]
mod panel_logic_tests {
    use super::*;
    use crate::projections::WorkroomProjection;
    use serde_json::json;

    fn voice_transcript_item(
        participant: VoiceParticipant,
        item_id: &str,
        text: &str,
        complete: bool,
    ) -> VoiceTranscriptItem {
        VoiceTranscriptItem {
            thread_ref: "thread".into(),
            session_ref: "session".into(),
            item_id: item_id.into(),
            participant,
            text: text.into(),
            complete,
        }
    }

    #[test]
    fn sarah_transcript_presentation_reveals_a_small_lead_then_speech_cadence() {
        let item = voice_transcript_item(
            VoiceParticipant::Sarah,
            "sarah",
            "abcdefghijklmnopqrstuvwxyz0123456789",
            false,
        );
        let mut presentation = VoiceTranscriptPresentation::default();
        presentation.observe_authoritative_change(&item, 0);

        assert_eq!(presentation.visible_chars(&item), 12);
        for _ in 0..10 {
            presentation.advance(std::slice::from_ref(&item));
        }
        assert_eq!(presentation.visible_chars(&item), 24);
    }

    #[test]
    fn transcript_pacing_counts_unicode_characters_and_never_delays_user_rows() {
        let sarah = voice_transcript_item(
            VoiceParticipant::Sarah,
            "sarah",
            "🙂é界abcdefghijklmno",
            true,
        );
        let user = voice_transcript_item(
            VoiceParticipant::User,
            "user",
            "user transcript is immediate",
            false,
        );
        let mut presentation = VoiceTranscriptPresentation::default();
        presentation.observe_authoritative_change(&sarah, 0);
        presentation.observe_authoritative_change(&user, 0);

        let (sarah_text, sarah_fully_presented) =
            visible_voice_transcript_text(&sarah, &presentation, false);
        assert_eq!(sarah_text, "🙂é界abcdefghi");
        assert!(!sarah_fully_presented);
        assert_eq!(presentation.visible_chars(&user), user.text.chars().count());
    }

    #[test]
    fn transcript_presentation_flushes_for_recovery_and_reduced_motion() {
        let item = voice_transcript_item(
            VoiceParticipant::Sarah,
            "sarah",
            "This sentence is longer than the initial presentation lead.",
            true,
        );
        let mut presentation = VoiceTranscriptPresentation::default();
        presentation.observe_authoritative_change(&item, 0);
        assert!(presentation.has_hidden_sarah_text(std::slice::from_ref(&item)));

        let (reduced_motion_text, fully_presented) =
            visible_voice_transcript_text(&item, &presentation, true);
        assert_eq!(reduced_motion_text, item.text);
        assert!(fully_presented);

        presentation.flush();
        assert!(!presentation.has_hidden_sarah_text(std::slice::from_ref(&item)));
        assert_eq!(presentation.visible_chars(&item), item.text.chars().count());
    }

    fn selection_effect_fixture() -> (VoiceSelectionEffectBinding, VoiceEditorTarget) {
        (
            VoiceSelectionEffectBinding {
                workspace_ref: SARAH_VOICE_WORKSPACE_REF.into(),
                document_version: "omega-buffer-v1:0=7".into(),
                target_path: "src/main.rs".into(),
                selection_start: VoiceTextPoint { line: 2, column: 4 },
                selection_end: VoiceTextPoint {
                    line: 2,
                    column: 10,
                },
                selected_text: "before".into(),
                replacement_text: "after".into(),
            },
            VoiceEditorTarget {
                workspace_ref: SARAH_VOICE_WORKSPACE_REF.into(),
                path: "src/main.rs".into(),
                document_version: Some("omega-buffer-v1:0=7".into()),
            },
        )
    }

    #[test]
    fn replacement_effect_requires_the_exact_confirmed_editor_state() {
        let (binding, target) = selection_effect_fixture();
        let validate = |binding: &VoiceSelectionEffectBinding,
                        target: &VoiceEditorTarget,
                        path: &str,
                        version: &str,
                        start: VoiceTextPoint,
                        end: VoiceTextPoint,
                        selected_text: &str,
                        replacement_text: &str| {
            validate_voice_selection_effect(
                binding,
                target,
                path,
                version,
                start,
                end,
                selected_text,
                replacement_text,
            )
        };

        assert!(
            validate(
                &binding,
                &target,
                "src/main.rs",
                "omega-buffer-v1:0=7",
                binding.selection_start,
                binding.selection_end,
                "before",
                "after",
            )
            .is_ok()
        );

        let mut other_workspace = target.clone();
        other_workspace.workspace_ref = "workspace.other".into();
        assert!(
            validate(
                &binding,
                &other_workspace,
                "src/main.rs",
                "omega-buffer-v1:0=7",
                binding.selection_start,
                binding.selection_end,
                "before",
                "after",
            )
            .is_err()
        );
        assert!(
            validate(
                &binding,
                &target,
                "src/other.rs",
                "omega-buffer-v1:0=7",
                binding.selection_start,
                binding.selection_end,
                "before",
                "after",
            )
            .is_err()
        );
        assert!(
            validate(
                &binding,
                &target,
                "src/main.rs",
                "omega-buffer-v1:0=8",
                binding.selection_start,
                binding.selection_end,
                "before",
                "after",
            )
            .is_err()
        );
        assert!(
            validate(
                &binding,
                &target,
                "src/main.rs",
                "omega-buffer-v1:0=7",
                VoiceTextPoint { line: 2, column: 5 },
                binding.selection_end,
                "before",
                "after",
            )
            .is_err()
        );
        assert!(
            validate(
                &binding,
                &target,
                "src/main.rs",
                "omega-buffer-v1:0=7",
                binding.selection_start,
                binding.selection_end,
                "changed",
                "after",
            )
            .is_err()
        );
        assert!(
            validate(
                &binding,
                &target,
                "src/main.rs",
                "omega-buffer-v1:0=7",
                binding.selection_start,
                binding.selection_end,
                "before",
                "changed",
            )
            .is_err()
        );
    }

    #[test]
    fn apply_bootstrap_maps_room_fields() {
        let mut projection = WorkroomProjection::honest_unsubscribed();
        let value = json!({
            "room": {
                "principalRef": "principal.sarah",
                "displayName": "Sarah",
                "role": "principal.sarah",
                "threadRef": "thread.sarah.abc",
            },
            "authority": {
                "profile": "sarah",
                "revision": "7"
            }
        });

        let room = value.get("room");
        projection.room = RoomProjection {
            meta: ProjectionMeta::fresh(sources::ROOM),
            principal_ref: string_field(room, &["principalRef"]),
            display_name: string_field(room, &["displayName"]),
            role: string_field(room, &["role"]),
            thread_ref: string_field(room, &["threadRef"]),
            authority_profile: string_field(value.get("authority"), &["profile"]),
            authority_revision: string_field(value.get("authority"), &["revision"]),
            detail: None,
        };

        assert_eq!(
            projection.room.principal_ref.as_deref(),
            Some("principal.sarah")
        );
        assert_eq!(projection.room.display_name.as_deref(), Some("Sarah"));
        assert_eq!(
            projection.room.thread_ref.as_deref(),
            Some("thread.sarah.abc")
        );
        assert_eq!(projection.room.authority_revision.as_deref(), Some("7"));
        assert_eq!(projection.room.meta.freshness, Freshness::Fresh);
    }

    #[test]
    fn workroom_and_voice_actions_are_registered_names() {
        let _open = OpenPanel;
        let _focus = FocusComposer;
        let _send = SendMessage;
        let _interrupt = InterruptTurn;
        let _prepare_voice = PrepareVoiceAdmission;
        let _start_voice = StartVoice;
        let _toggle_voice_mute = ToggleVoiceMute;
        let _interrupt_voice = InterruptVoice;
        let _approve_voice = ApproveSarahVoiceCommand;
        let _reject_voice = RejectSarahVoiceCommand;
        let _end_voice = EndVoice;
        let _retry_voice = RetryVoice;
        assert_eq!(WorkroomProjection::header(), "Sarah");
        assert_eq!(PANEL_KEY, "SarahWorkroomPanel");
    }

    #[test]
    fn transient_voice_disconnects_reconnect_but_terminal_states_do_not() {
        assert_eq!(voice_reconnect_delay(1), Duration::from_secs(1));
        assert_eq!(voice_reconnect_delay(2), Duration::from_secs(2));
        assert_eq!(voice_reconnect_delay(3), Duration::from_secs(4));
        assert_eq!(voice_reconnect_delay(20), Duration::from_secs(4));
        assert_eq!(
            voice_settlement_retry_delay(0, true),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            voice_settlement_retry_delay(1, true),
            Some(Duration::from_secs(2))
        );
        assert_eq!(
            voice_settlement_retry_delay(2, true),
            Some(Duration::from_secs(4))
        );
        assert_eq!(voice_settlement_retry_delay(3, true), None);
        assert_eq!(voice_settlement_retry_delay(0, false), None);
        for blocker in [
            omega_effectd::HostedSessionBlocker::ServiceUnreachable,
            omega_effectd::HostedSessionBlocker::ServiceUnavailable { status: 503 },
        ] {
            assert_eq!(
                voice_settlement_retry_delay(0, blocker.is_retryable()),
                Some(Duration::from_secs(1))
            );
        }
        assert_eq!(
            voice_settlement_retry_delay(
                0,
                omega_effectd::HostedSessionBlocker::ProofRejected { status: 403 }.is_retryable(),
            ),
            None
        );
        assert!(!voice_settlement_is_recovered(
            omega_effectd::SarahVoiceSettlementState::Pending,
            None,
        ));
        assert!(!voice_settlement_is_recovered(
            omega_effectd::SarahVoiceSettlementState::Settled,
            None,
        ));
        assert!(voice_settlement_is_recovered(
            omega_effectd::SarahVoiceSettlementState::Settled,
            Some(0),
        ));
        assert!(voice_settlement_is_recovered(
            omega_effectd::SarahVoiceSettlementState::Released,
            Some(0),
        ));
        assert_eq!(
            next_voice_reconnect_attempt(
                SarahVoiceState::Reconnecting,
                false,
                0,
                Duration::from_secs(65)
            ),
            Some(1)
        );
        assert_eq!(
            next_voice_reconnect_attempt(
                SarahVoiceState::Reconnecting,
                false,
                2,
                Duration::from_secs(1)
            ),
            Some(3)
        );
        assert_eq!(
            next_voice_reconnect_attempt(
                SarahVoiceState::Reconnecting,
                false,
                3,
                Duration::from_secs(1)
            ),
            None
        );
        assert_eq!(
            next_voice_reconnect_attempt(SarahVoiceState::Error, true, 0, Duration::from_secs(65)),
            Some(1)
        );
        assert_eq!(
            next_voice_reconnect_attempt(SarahVoiceState::Idle, false, 0, Duration::from_secs(65)),
            None
        );
        assert_eq!(
            next_voice_reconnect_attempt(
                SarahVoiceState::Ending,
                false,
                0,
                Duration::from_secs(65)
            ),
            None
        );
        assert_eq!(
            next_voice_reconnect_attempt(SarahVoiceState::Error, false, 0, Duration::from_secs(65)),
            None
        );
    }

    #[test]
    fn community_room_headers_and_copy_are_distinct_and_unpaid() {
        assert_eq!(OWNER_PRIVATE_ROOM_HEADER, "Sarah");
        assert_eq!(COMMUNITY_ROOM_HEADER, "Community");
        assert_ne!(OWNER_PRIVATE_ROOM_HEADER, COMMUNITY_ROOM_HEADER);
        assert!(COMMUNITY_ROOM_SUBTITLE.contains("separate"));
        assert!(V1_NO_PAY_ROOM_DESCRIPTION.contains("experience"));
        assert!(V1_NO_PAY_ROOM_DESCRIPTION.contains("not money"));
        assert!(
            !V1_NO_PAY_ROOM_DESCRIPTION
                .to_ascii_lowercase()
                .contains("earnings")
        );
        assert!(V1_NO_PAY_FIRST_RUN_COPY.contains("does not pay"));
        assert_eq!(EXPERIENCE_LABEL, "experience");
        let community = CommunityRoomProjection::honest_unsubscribed();
        assert!(community.is_v1_compliant());
        assert!(community.membership.is_honest_missing());
        assert!(community.work_units.is_honest_missing());
        assert!(community.experience.is_v1_experience_only());
    }

    #[test]
    fn two_room_isolation_on_panel_fields() {
        // Mirrors SarahWorkroomPanel field layout without constructing GPUI.
        let mut projection = WorkroomProjection::honest_unsubscribed();
        let mut community = CommunityRoomProjection::honest_unsubscribed();
        projection.room.thread_ref = Some("thread.sarah.1".into());
        projection.transcript.push_bounded(TranscriptRow {
            message_ref: "private.1".into(),
            role: "owner".into(),
            text: "secret".into(),
            ack: MessageAck::Confirmed,
        });
        community.room.group_ref = Some("group.community.1".into());
        community.push_untrusted_message("community.1".into(), "member".into(), "hello".into());
        let owner_refs: std::collections::BTreeSet<&str> = projection
            .transcript
            .rows
            .iter()
            .map(|r| r.message_ref.as_str())
            .collect();
        let community_refs: std::collections::BTreeSet<&str> = community
            .transcript
            .rows
            .iter()
            .map(|r| r.message_ref.as_str())
            .collect();
        assert!(owner_refs.is_disjoint(&community_refs));
        assert_ne!(
            projection.room.thread_ref.as_deref(),
            community.room.group_ref.as_deref()
        );
        // Switch kind only changes active room — both stores remain.
        let active = RoomKind::Community;
        assert_eq!(active.header(), COMMUNITY_ROOM_HEADER);
        assert_eq!(RoomKind::OwnerPrivate.header(), OWNER_PRIVATE_ROOM_HEADER);
        assert_eq!(projection.transcript.rows.len(), 1);
        assert_eq!(community.transcript.rows.len(), 1);
    }

    #[test]
    fn interrupt_pending_law_on_run_state() {
        let mut run = RunStateProjection {
            meta: ProjectionMeta::fresh(sources::RUN_STATE),
            phase: RunPhase::Running,
            reason: None,
            turn_ref: Some("turn:1".into()),
            interrupt_intent: InterruptIntentState::None,
        };
        run.mark_interrupt_pending();
        assert_eq!(run.interrupt_intent, InterruptIntentState::Pending);
        assert_eq!(run.phase, RunPhase::Running);
        assert_ne!(run.interrupt_intent, InterruptIntentState::Applied);
    }

    #[test]
    fn parse_run_phase_covers_nr06_and_event_kinds() {
        assert_eq!(parse_run_phase("running"), RunPhase::Running);
        assert_eq!(parse_run_phase("interrupt_pending"), RunPhase::Running);
        assert_eq!(parse_run_phase("turn.started"), RunPhase::Running);
        assert_eq!(parse_run_phase("turn.finished"), RunPhase::Finished);
        assert_eq!(parse_run_phase("interrupted"), RunPhase::Interrupted);
    }

    #[test]
    fn confirmed_nostr_record_projection_keeps_only_bounded_public_refs() {
        let value = json!({
            "entries": [{
                "eventId": "a".repeat(64),
                "kind": 30174,
                "recordKind": "memory",
                "authorFingerprint": "0123456789ABCDEF",
                "createdAt": "2026-07-24T00:00:00Z",
                "source": "confirmed_nostr"
            }],
            "cursor": format!("cursor.1.{}", "a".repeat(64)),
            "nextCursor": null,
            "gapState": "possible",
            "source": "confirmed_nostr"
        });
        let projection = parse_nostr_records_projection(Some(&value));
        assert_eq!(projection.rows.len(), 1);
        assert_eq!(projection.rows[0].kind, 30_174);
        assert_eq!(projection.rows[0].record_kind, "memory");
        assert_eq!(projection.rows[0].source, "confirmed_nostr");
        assert_eq!(projection.gap, GapState::Gap);
        assert_eq!(projection.source, "confirmed_nostr");
        assert!(projection.next_cursor.is_none());

        let missing = parse_nostr_records_projection(None);
        assert_eq!(missing.gap, GapState::Unavailable);
        assert!(missing.rows.is_empty());
    }

    #[test]
    fn apply_snapshot_maps_entries_and_proactive_turns_as_ordinary_rows() {
        let value = json!({
            "transcript": {
                "entries": [
                    {
                        "eventId": "evt.owner.1",
                        "role": "owner",
                        "kind": "text",
                        "text": "status?",
                        "status": "accepted"
                    },
                    {
                        "eventId": "message.sarah_auto.tick.1",
                        "role": "sarah",
                        "kind": "text",
                        "text": "Release is green.",
                        "status": "confirmed"
                    }
                ],
                "cursor": "cursor.1",
                "gapState": "none"
            },
            "activity": { "entries": [], "gapState": "none" },
            "runState": { "state": "idle", "turnRef": null }
        });

        let mut projection = WorkroomProjection::honest_unsubscribed();
        let mut transcript = TranscriptProjection {
            meta: ProjectionMeta::fresh(sources::TRANSCRIPT),
            rows: Vec::new(),
            cursor: string_field(value.get("transcript"), &["cursor"]),
            truncated: false,
        };
        if let Some(items) = value
            .get("transcript")
            .and_then(|t| {
                t.get("entries")
                    .or_else(|| t.get("items"))
                    .or_else(|| t.get("messages"))
            })
            .and_then(|v| v.as_array())
        {
            for item in items {
                let ack = match item
                    .get("ack")
                    .or_else(|| item.get("status"))
                    .or_else(|| item.get("state"))
                    .and_then(|v| v.as_str())
                {
                    Some("pending") => MessageAck::Pending,
                    _ => MessageAck::Confirmed,
                };
                transcript.push_bounded(TranscriptRow {
                    message_ref: string_field(
                        Some(item),
                        &["messageRef", "eventId", "id", "ref", "cursor"],
                    )
                    .unwrap_or_else(|| "unknown".into()),
                    role: string_field(Some(item), &["role"]).unwrap_or_else(|| "unknown".into()),
                    text: string_field(Some(item), &["text", "content"]).unwrap_or_default(),
                    ack,
                });
            }
        }
        projection.transcript = transcript;
        projection.room.thread_ref = Some("thread.sarah.abc".into());
        projection.recompute_attention();

        assert_eq!(projection.transcript.rows.len(), 2);
        assert_eq!(projection.transcript.rows[1].role, "sarah");
        assert_eq!(
            projection.transcript.rows[1].message_ref,
            "message.sarah_auto.tick.1"
        );
        assert_eq!(projection.transcript.rows[1].ack, MessageAck::Confirmed);
        // Proactive update raises the same attention path as a Q&A answer.
        assert_eq!(projection.attention.unread_count, 1);
        assert_eq!(projection.attention.marker, AttentionMarker::NeedsAttention);
        assert!(empty_room_is_honest(
            &projection.transcript,
            OMEGA_AUTONOMOUS_TICK_ENABLED
        ));

        projection.mark_room_read();
        assert_eq!(projection.attention.unread_count, 0);
        assert_eq!(projection.attention.marker, AttentionMarker::None);
    }

    #[test]
    fn tick_off_empty_snapshot_stays_honest() {
        assert!(!OMEGA_AUTONOMOUS_TICK_ENABLED);
        let empty = TranscriptProjection {
            meta: ProjectionMeta::fresh(sources::TRANSCRIPT),
            rows: Vec::new(),
            cursor: None,
            truncated: false,
        };
        assert!(empty_room_is_honest(&empty, OMEGA_AUTONOMOUS_TICK_ENABLED));
        let mut p = WorkroomProjection::honest_unsubscribed();
        p.transcript = empty;
        p.recompute_attention();
        assert_eq!(p.attention.unread_count, 0);
        assert!(!p.attention.marker.is_set());
        assert!(p.attention.tick_note.is_some());
    }

    #[test]
    fn apply_snapshot_nr06_shape_maps_entries_and_run_state() {
        let value = json!({
            "transcript": {
                "entries": [{
                    "eventId": "evt1",
                    "role": "owner",
                    "text": "hello",
                    "status": "confirmed"
                }],
                "cursor": "cursor.0",
                "gapState": "none"
            },
            "activity": {
                "entries": [{
                    "eventId": "act1",
                    "entry": "tool.call",
                    "turnRef": "turn.1",
                    "summary": "capacity"
                }],
                "cursor": "cursor.1"
            },
            "runState": {
                "state": "running",
                "turnRef": "turn.1",
                "reason": null
            }
        });

        let items = value
            .get("transcript")
            .and_then(|t| t.get("entries"))
            .and_then(|v| v.as_array())
            .expect("entries");
        assert_eq!(items.len(), 1);
        assert_eq!(
            string_field(Some(&items[0]), &["eventId"]).as_deref(),
            Some("evt1")
        );
        let phase = parse_run_phase(
            string_field(value.get("runState"), &["state"])
                .as_deref()
                .unwrap(),
        );
        assert_eq!(phase, RunPhase::Running);
        let entry_kind = string_field(
            value
                .get("activity")
                .and_then(|a| a.get("entries"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first()),
            &["entry", "kind"],
        );
        assert_eq!(entry_kind.as_deref(), Some("tool.call"));
    }
}
