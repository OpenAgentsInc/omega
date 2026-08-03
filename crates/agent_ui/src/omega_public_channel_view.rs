//! The selected tester-channel timeline for Omega.
//!
//! This view owns one relay-qualified channel cache. A channel selection can
//! start its relay session, and leaving the channel stops that session without
//! deleting verified rows. Public writes are signed through `omega_identity`
//! and published through the existing authenticated relay edge; the view never
//! receives secret key material.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use editor::{Editor, EditorEvent};
use gpui::{
    Action as _, AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FollowMode, ImageSource, ListAlignment, ListSizingBehavior, ListState, ObjectFit,
    ParentElement as _, PromptLevel, Render, Role, SharedString, Styled as _, Task, Window, img,
    list, px,
};
use http_client::HttpClient;
use omega_actions::{
    IdentityActivationEvents, IdentityActivationOutcome, OpenOnboarding,
    community_sarah::{
        JoinRoom, LeaveRoom, ModeratorStop, RemoveSarah, SummonSarah, TalkToSarah, ToggleMute,
    },
};
use omega_identity::{
    AccountRegistryService, DurableIdentityActionDecision, HeldIdentityAction, SignerKind,
};
use omega_signer_broker::{
    Nip46WebSocketTransport, RemoteSignerMetadata, SignerBroker, SignerRoute,
};
use ui::{
    Banner, Button, ButtonSize, ButtonStyle, Color, CopyButton, IconButton, IconName, IconSize,
    Label, LabelSize, ScrollAxes, Scrollbars, Severity, Tooltip, WithScrollbar, prelude::*,
};

use crate::{
    omega_nostr_activity,
    omega_public_channel_livekit::{
        CommunityRoomMediaControl, CommunityRoomMediaEvent, start_community_room_media,
    },
    omega_public_channel_media::{
        PublicChannelAttachment, PublicChannelMediaFact, PublicChannelMediaIntent,
        PublicChannelMediaKey, PublicChannelMediaLifecycle, PublicChannelMediaState,
        PublicChannelMediaUnavailableReason, fetch_public_channel_media,
    },
    omega_public_channel_publish::{
        PreparedPublicChannelWrite, PublicChannelWrite, SignedPublicChannelWrite,
        authorize_prepared_write, authorize_remote_prepared_write, prepare_remote_write,
        prepare_write, publish_signed_write, sign_prepared_write, sign_remote_prepared_write,
    },
    omega_public_channel_relay::{
        RelayAdmissionLimits, RelayCursor, RelayGapReason, RelayIntent, RelayLifecycle,
        RelaySessionConfig, RelaySnapshot, run_relay_session,
    },
    omega_public_channel_sarah::{
        CommunityCallLifecycle, CommunityRoomAdmission, CommunityRoomContext,
        CommunitySarahControl, CommunitySarahIntent, CommunitySarahRoom, CommunitySarahState,
    },
    omega_public_channel_timeline::{
        ContentPart, DeletionKind, EventFacts, MediaFact, SignatureState, TimelineProjection,
        event_facts, project_timeline, stable_verified_events,
    },
    omega_public_channels::{ChannelCursor, ChannelDescriptor, ChannelLifecycle, ChannelSnapshot},
};

const RETIRED_SESSION_LIMIT: usize = 2;
const RETIRED_SARAH_MEDIA_LIMIT: usize = 2;
const FACTS_PANE_WIDTH: gpui::Pixels = px(336.);
const COMPACT_FACTS_THRESHOLD: gpui::Pixels = px(960.);

fn unix_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WriteStatus {
    Ready,
    SendingMessage,
    MessageSent(String),
    SendingReport,
    ReportSent,
    Failed(String),
}

enum PublicChannelWriteWork {
    Published(SignedPublicChannelWrite),
    ActivationRequired {
        prepared: PreparedPublicChannelWrite,
        intent: HeldIdentityAction,
        fingerprint: String,
    },
}

impl WriteStatus {
    fn is_sending(&self) -> bool {
        matches!(self, Self::SendingMessage | Self::SendingReport)
    }

    fn label(&self) -> String {
        match self {
            Self::Ready => "Ready to post public feedback.".to_string(),
            Self::SendingMessage => "Signing and sending public feedback…".to_string(),
            Self::MessageSent(event_id) => {
                format!("Sent public feedback as event {event_id}.")
            }
            Self::SendingReport => "Signing and sending a public report…".to_string(),
            Self::ReportSent => {
                "Report sent. Moderators decide whether to remove the message.".to_string()
            }
            Self::Failed(reason) => format!("Not sent. {reason}"),
        }
    }
}

impl PublicChannelMediaFact for MediaFact {
    fn url(&self) -> &str {
        &self.url
    }

    fn mime_type(&self) -> &str {
        &self.mime_type
    }

    fn digest(&self) -> Option<&str> {
        self.digest.as_deref()
    }

    fn size(&self) -> Option<usize> {
        self.size.and_then(|size| usize::try_from(size).ok())
    }

    fn alt(&self) -> Option<&str> {
        self.alt.as_deref()
    }

    fn blurhash(&self) -> Option<&str> {
        self.blurhash.as_deref()
    }

    fn dimensions(&self) -> Option<&str> {
        self.dimensions.as_deref()
    }

    fn duration_seconds(&self) -> Option<&str> {
        self.duration_seconds.as_deref()
    }

    fn thumbnail_url(&self) -> Option<&str> {
        self.thumbnail_url.as_deref()
    }

    fn waveform(&self) -> &[String] {
        &self.waveform
    }
}

#[derive(Clone, Debug)]
pub enum PublicChannelViewEvent {
    SnapshotChanged(ChannelSnapshot),
}

pub struct PublicChannelView {
    focus_handle: FocusHandle,
    descriptor: ChannelDescriptor,
    http_client: Arc<dyn HttpClient>,
    composer: Entity<Editor>,
    write_status: WriteStatus,
    write_task: Option<Task<()>>,
    relay_outage_observed: bool,
    relay_snapshot: RelaySnapshot,
    projection: TimelineProjection,
    list_state: ListState,
    revealed_content_warnings: BTreeSet<String>,
    selected_event_id: Option<String>,
    media_states: BTreeMap<PublicChannelMediaKey, PublicChannelMediaState>,
    media_tasks: BTreeMap<PublicChannelMediaKey, Task<()>>,
    #[cfg(test)]
    media_fetch_result: Option<PublicChannelMediaState>,
    #[cfg(test)]
    media_fetch_pending: bool,
    generation: u64,
    session_running: bool,
    relay_intent_sender: Option<async_channel::Sender<RelayIntent>>,
    relay_session_task: Option<Task<()>>,
    retired_session_tasks: VecDeque<Task<()>>,
    sarah_room: CommunitySarahRoom,
    sarah_control_task: Option<Task<()>>,
    sarah_media_controls: Option<async_channel::Sender<CommunityRoomMediaControl>>,
    sarah_media_task: Option<Task<()>>,
    sarah_refresh_task: Option<Task<()>>,
    retired_sarah_media_tasks: VecDeque<Task<()>>,
}

impl EventEmitter<PublicChannelViewEvent> for PublicChannelView {}

impl Focusable for PublicChannelView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl PublicChannelView {
    pub fn new(
        descriptor: ChannelDescriptor,
        http_client: Arc<dyn HttpClient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list_state = ListState::new(0, ListAlignment::Top, px(2048.));
        list_state.set_follow_mode(FollowMode::Tail);
        let composer = cx.new(|cx| {
            let mut editor = Editor::auto_height(1, 4, window, cx);
            editor.set_placeholder_text("Share public alpha feedback…", window, cx);
            editor
        });
        cx.subscribe(&composer, |_, _, event, cx| {
            if matches!(event, EditorEvent::BufferEdited) {
                cx.notify();
            }
        })
        .detach();
        let mut sarah_room = CommunitySarahRoom::default();
        sarah_room.configure(
            CommunityRoomContext {
                community_ref: descriptor.group_id.clone(),
                channel_ref: descriptor.channel_id.clone(),
            },
            omega_effectd::openagents_session_if_initialized(cx).is_some(),
        );
        Self {
            focus_handle: cx.focus_handle(),
            descriptor,
            http_client,
            composer,
            write_status: WriteStatus::Ready,
            write_task: None,
            relay_outage_observed: false,
            relay_snapshot: RelaySnapshot::default(),
            projection: TimelineProjection::default(),
            list_state,
            revealed_content_warnings: BTreeSet::new(),
            selected_event_id: None,
            media_states: BTreeMap::new(),
            media_tasks: BTreeMap::new(),
            #[cfg(test)]
            media_fetch_result: None,
            #[cfg(test)]
            media_fetch_pending: false,
            generation: 0,
            session_running: false,
            relay_intent_sender: None,
            relay_session_task: None,
            retired_session_tasks: VecDeque::new(),
            sarah_room,
            sarah_control_task: None,
            sarah_media_controls: None,
            sarah_media_task: None,
            sarah_refresh_task: None,
            retired_sarah_media_tasks: VecDeque::new(),
        }
    }

    pub fn descriptor(&self) -> &ChannelDescriptor {
        &self.descriptor
    }

    pub fn last_current_at(&self) -> Option<u64> {
        self.relay_snapshot.last_current_at
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_channel_snapshot_for_tests(
        &mut self,
        snapshot: ChannelSnapshot,
        cx: &mut Context<Self>,
    ) {
        self.pause(cx);
        let lifecycle = match snapshot.lifecycle {
            ChannelLifecycle::Disconnected => RelayLifecycle::Disconnected,
            ChannelLifecycle::Connecting => RelayLifecycle::Connecting,
            ChannelLifecycle::Replaying => RelayLifecycle::Replaying,
            ChannelLifecycle::Current => RelayLifecycle::Current,
            ChannelLifecycle::Reconnecting => RelayLifecycle::Reconnecting,
            ChannelLifecycle::Stale => RelayLifecycle::Stale,
        };
        self.apply_relay_snapshot(
            RelaySnapshot {
                lifecycle,
                gap_reason: (lifecycle == RelayLifecycle::Stale)
                    .then_some(RelayGapReason::RelayUnavailable),
                cursor: snapshot.cursor.map(|cursor| RelayCursor {
                    created_at: cursor.created_at,
                    event_ids_at_created_at: cursor.event_ids_at_created_at,
                }),
                metadata_trusted: self.descriptor.expected_relay_self_pubkey.is_some(),
                ..Default::default()
            },
            cx,
        );
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        if self.session_running {
            return;
        }
        if let Some(task) = self.relay_session_task.take() {
            self.retired_session_tasks.push_back(task);
        }
        while self.retired_session_tasks.len() > RETIRED_SESSION_LIMIT {
            self.retired_session_tasks.pop_front();
        }

        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        let (intent_sender, intent_receiver) = async_channel::bounded(16);
        let (snapshot_sender, snapshot_receiver) = async_channel::bounded(16);
        self.relay_intent_sender = Some(intent_sender);
        self.session_running = true;
        let config = self.relay_config();
        let timer_executor = cx.background_executor().clone();
        self.relay_session_task = Some(cx.spawn(async move |this, cx| {
            let driver = cx.background_spawn(run_relay_session(
                config,
                intent_receiver,
                snapshot_sender,
                timer_executor,
            ));
            while let Ok(snapshot) = snapshot_receiver.recv().await {
                if this
                    .update(cx, |this, cx| {
                        if this.generation == generation {
                            this.apply_relay_snapshot(snapshot, cx);
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
            let result = driver.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.session_running = false;
                this.relay_intent_sender = None;
                if let Err(error) = result {
                    log::info!("public channel session stopped: {error:#}");
                    let mut snapshot = this.relay_snapshot.clone();
                    snapshot.lifecycle = RelayLifecycle::Stale;
                    snapshot.gap_reason = Some(RelayGapReason::RelayUnavailable);
                    this.apply_relay_snapshot(snapshot, cx);
                }
            })
            .unwrap_or_else(|error| {
                log::debug!("public channel view disappeared while its session stopped: {error:#}");
            });
        }));
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.saturating_add(1);
        self.session_running = false;
        if let Some(sender) = self.relay_intent_sender.take() {
            if let Err(error) = sender.try_send(RelayIntent::Close) {
                log::debug!("public channel close intent was not delivered: {error}");
            }
        }
        self.media_tasks.clear();
        self.sarah_control_task = None;
        self.stop_sarah_media();
        for state in self.media_states.values_mut() {
            if matches!(state, PublicChannelMediaState::Loading) {
                *state = PublicChannelMediaState::Gated;
            }
        }
        self.selected_event_id = None;
        cx.notify();
    }

    pub fn load_older(&mut self) {
        if let Some(sender) = &self.relay_intent_sender {
            if let Err(error) = sender.try_send(RelayIntent::LoadOlder) {
                log::debug!("public channel pagination intent was not delivered: {error}");
            }
        }
    }

    pub fn channel_snapshot(&self) -> ChannelSnapshot {
        ChannelSnapshot {
            relay_url: self.descriptor.relay_url.clone(),
            group_id: self.descriptor.group_id.clone(),
            lifecycle: channel_lifecycle(self.relay_snapshot.lifecycle),
            cursor: self
                .relay_snapshot
                .cursor
                .as_ref()
                .map(|cursor| ChannelCursor {
                    created_at: cursor.created_at,
                    event_ids_at_created_at: cursor.event_ids_at_created_at.clone(),
                }),
            event_ids: self
                .relay_snapshot
                .events
                .iter()
                .map(|event| event.id.clone())
                .collect(),
            cached: false,
        }
    }

    fn relay_config(&self) -> RelaySessionConfig {
        RelaySessionConfig {
            relay_url: self.descriptor.relay_url.clone(),
            group_id: self.descriptor.group_id.clone(),
            accepted_kinds: self.descriptor.accepted_kinds.clone(),
            group_state_kinds: self.descriptor.group_state_kinds.clone(),
            moderation_kinds: self.descriptor.moderation_kinds.clone(),
            expected_relay_self_pubkey: self.descriptor.expected_relay_self_pubkey.clone(),
            history_page_size: self.descriptor.limits.history_page_size,
            limits: RelayAdmissionLimits {
                content_bytes: self.descriptor.limits.content_bytes,
                event_bytes: self.descriptor.limits.event_bytes,
                future_skew_seconds: self.descriptor.limits.future_skew_seconds,
                max_age_seconds: self.descriptor.limits.max_age_seconds,
                tags: self.descriptor.limits.tags,
            },
        }
    }

    fn apply_relay_snapshot(&mut self, mut snapshot: RelaySnapshot, cx: &mut Context<Self>) {
        merge_retained_snapshot(&self.relay_snapshot, &mut snapshot);
        if snapshot.lifecycle == RelayLifecycle::Current {
            self.relay_outage_observed = false;
        } else if snapshot.gap_reason == Some(RelayGapReason::RelayUnavailable)
            || snapshot.lifecycle == RelayLifecycle::Stale
        {
            self.relay_outage_observed = true;
        }
        let old_event_ids = self
            .projection
            .rows
            .iter()
            .map(|row| row.event_id.as_str())
            .collect::<Vec<_>>();
        let was_following_tail = self.list_state.is_following_tail();
        let relay_self = snapshot
            .metadata_trusted
            .then_some(self.descriptor.expected_relay_self_pubkey.as_deref())
            .flatten();
        let projection = project_timeline(&snapshot.events, &self.descriptor.group_id, relay_self);
        let new_event_ids = projection
            .rows
            .iter()
            .map(|row| row.event_id.as_str())
            .collect::<Vec<_>>();

        if new_event_ids.ends_with(&old_event_ids) && new_event_ids.len() > old_event_ids.len() {
            let added = new_event_ids.len() - old_event_ids.len();
            self.list_state.splice(0..0, added);
        } else if new_event_ids.starts_with(&old_event_ids)
            && new_event_ids.len() > old_event_ids.len()
        {
            self.list_state.splice(
                old_event_ids.len()..old_event_ids.len(),
                new_event_ids.len() - old_event_ids.len(),
            );
        } else if new_event_ids != old_event_ids {
            self.list_state.reset(new_event_ids.len());
        } else if !new_event_ids.is_empty() {
            self.list_state.remeasure_items(0..new_event_ids.len());
        }
        if was_following_tail && !new_event_ids.is_empty() {
            self.list_state.scroll_to_end();
        }

        self.relay_snapshot = snapshot;
        self.projection = projection;
        self.reconcile_interaction_state();
        cx.emit(PublicChannelViewEvent::SnapshotChanged(
            self.channel_snapshot(),
        ));
        cx.notify();
    }

    fn reconcile_interaction_state(&mut self) {
        let retained_event_ids = self
            .projection
            .rows
            .iter()
            .map(|row| row.event_id.as_str())
            .collect::<BTreeSet<_>>();
        self.revealed_content_warnings
            .retain(|event_id| retained_event_ids.contains(event_id.as_str()));
        if self
            .selected_event_id
            .as_ref()
            .is_some_and(|event_id| !retained_event_ids.contains(event_id.as_str()))
        {
            self.selected_event_id = None;
        }

        let mut retained_media = BTreeSet::new();
        for row in &self.projection.rows {
            for (attachment_index, media) in row.media.iter().enumerate() {
                let key = PublicChannelMediaKey::new(
                    self.descriptor.channel_id.clone(),
                    row.event_id.clone(),
                    attachment_index,
                );
                if PublicChannelAttachment::try_from_media_fact(
                    media,
                    self.descriptor.limits.attachment_bytes,
                )
                .is_ok()
                {
                    retained_media.insert(key.clone());
                    self.media_states.entry(key).or_default();
                }
            }
        }
        self.media_states
            .retain(|key, _| retained_media.contains(key));
        self.media_tasks
            .retain(|key, _| retained_media.contains(key));
    }

    fn selected_facts(&self) -> Option<EventFacts> {
        let selected = self.selected_event_id.as_deref()?;
        let row = self
            .projection
            .rows
            .iter()
            .find(|row| row.event_id == selected)?;
        Some(event_facts(
            row,
            &self.descriptor.relay_url,
            &self.descriptor.group_id,
        ))
    }

    fn reveal_content(&mut self, event_id: &str, row_index: usize, cx: &mut Context<Self>) {
        self.revealed_content_warnings.insert(event_id.to_string());
        self.list_state.remeasure_items(row_index..row_index + 1);
        cx.notify();
    }

    fn inspect_event(&mut self, event_id: &str, cx: &mut Context<Self>) {
        if self
            .projection
            .rows
            .iter()
            .any(|row| row.event_id == event_id)
        {
            self.selected_event_id = Some(event_id.to_string());
            cx.notify();
        }
    }

    fn close_event_facts(&mut self, cx: &mut Context<Self>) {
        self.selected_event_id = None;
        cx.notify();
    }

    fn send_composer(&mut self, cx: &mut Context<Self>) {
        let content = self.composer.read(cx).text(cx).trim().to_string();
        if content.is_empty() {
            self.write_status = WriteStatus::Failed("Write a message before sending.".to_string());
            cx.notify();
            return;
        }
        self.start_write(PublicChannelWrite::Message { content }, cx);
    }

    fn start_write(&mut self, write: PublicChannelWrite, cx: &mut Context<Self>) {
        if self.write_status.is_sending() {
            return;
        }
        let is_report = matches!(write, PublicChannelWrite::Report { .. });
        self.write_status = if is_report {
            WriteStatus::SendingReport
        } else {
            WriteStatus::SendingMessage
        };
        cx.notify();

        let descriptor = self.descriptor.clone();
        let events = self.relay_snapshot.events.clone();
        let work = cx.background_spawn(async move {
            let registry = AccountRegistryService::for_channel(*app_identity::CHANNEL);
            let selection = registry.selection_token().map_err(anyhow::Error::from)?;
            let dashboard = registry.inspect().map_err(anyhow::Error::from)?;
            let remote_signer_selected = dashboard
                .accounts
                .iter()
                .any(|account| account.is_active && account.signer.kind == SignerKind::RemoteNip46);
            if remote_signer_selected {
                let capability = registry
                    .remote_signer_capability(&selection)
                    .map_err(anyhow::Error::from)?;
                let prepared = prepare_remote_write(&selection, &descriptor, write, &events)?;
                let authorization =
                    authorize_remote_prepared_write(&registry, &selection, &prepared)?;
                let route = SignerRoute::RemoteNip46 {
                    metadata: RemoteSignerMetadata { capability },
                    transport: Arc::new(Nip46WebSocketTransport::system()),
                };
                let signed = sign_remote_prepared_write(
                    &SignerBroker::system(),
                    &route,
                    selection,
                    &descriptor,
                    prepared,
                    &authorization,
                )
                .await?;
                publish_signed_write(&descriptor, &signed)?;
                return anyhow::Ok(PublicChannelWriteWork::Published(signed));
            }

            let identity_service = omega_identity::IdentityService::system(*app_identity::CHANNEL);
            let prepared = prepare_write(&identity_service, &descriptor, write, &events)?;
            match authorize_prepared_write(&identity_service, &prepared)? {
                DurableIdentityActionDecision::Authorized(authorization) => {
                    let signed = sign_prepared_write(
                        &identity_service,
                        &descriptor,
                        prepared,
                        &authorization,
                    )?;
                    publish_signed_write(&descriptor, &signed)?;
                    anyhow::Ok(PublicChannelWriteWork::Published(signed))
                }
                DurableIdentityActionDecision::ActivationRequired { account, intent } => {
                    anyhow::Ok(PublicChannelWriteWork::ActivationRequired {
                        prepared,
                        intent,
                        fingerprint: account.fingerprint_display(),
                    })
                }
            }
        });
        self.write_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            if let Err(error) = this.update_in(cx, move |this, window, cx| match result {
                Ok(PublicChannelWriteWork::Published(signed)) => {
                    this.finish_write(signed, window, cx)
                }
                Ok(PublicChannelWriteWork::ActivationRequired {
                    prepared,
                    intent,
                    fingerprint,
                }) => {
                    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
                        Ok(duration) => duration.as_secs(),
                        Err(error) => {
                            this.write_status = WriteStatus::Failed(format!(
                                "Could not retain this write because the system clock is invalid: {error}"
                            ));
                            cx.notify();
                            return;
                        }
                    };
                    let owner = cx.weak_entity();
                    let callback_intent = intent.clone();
                    match IdentityActivationEvents::register(
                        intent,
                        now,
                        move |outcome, cx| {
                            let resume_intent = callback_intent.clone();
                            if let Err(error) =
                                owner.update(cx, |this, cx| match outcome {
                                    IdentityActivationOutcome::Completed => {
                                        this.resume_prepared_write(
                                            prepared,
                                            resume_intent,
                                            cx,
                                        );
                                    }
                                    IdentityActivationOutcome::Cancelled => {
                                        this.write_status = WriteStatus::Failed(
                                            "Identity setup was cancelled; nothing was sent."
                                                .to_string(),
                                        );
                                        cx.notify();
                                    }
                                    IdentityActivationOutcome::Expired => {
                                        this.write_status = WriteStatus::Failed(
                                            "Identity setup expired; nothing was sent.".to_string(),
                                        );
                                        cx.notify();
                                    }
                                })
                            {
                                log::debug!(
                                    "public-channel activation finished after its owner closed: {error:#}"
                                );
                                if outcome == IdentityActivationOutcome::Completed {
                                    let identity_service =
                                        omega_identity::IdentityService::system(
                                            *app_identity::CHANNEL,
                                        );
                                    if let Err(error) = identity_service
                                        .take_activated_identity_action(&callback_intent)
                                    {
                                        log::warn!(
                                            "could not consume the completed public-write activation after its owner closed: {error}"
                                        );
                                    }
                                }
                            }
                        },
                        cx,
                    ) {
                        Ok(()) => {
                            this.write_status = WriteStatus::Failed(format!(
                                "Set up identity {fingerprint} to finish this exact write."
                            ));
                            window.dispatch_action(OpenOnboarding.boxed_clone(), cx);
                            cx.notify();
                        }
                        Err(error) => {
                            this.write_status = WriteStatus::Failed(format!(
                                "Could not retain this write for identity setup: {error}"
                            ));
                            cx.notify();
                        }
                    }
                }
                Err(error) => {
                    this.write_status = WriteStatus::Failed(error.to_string());
                    cx.notify();
                }
            }) {
                log::debug!("tester-channel write finished after its view closed: {error:#}");
            }
        }));
    }

    fn resume_prepared_write(
        &mut self,
        prepared: PreparedPublicChannelWrite,
        intent: HeldIdentityAction,
        cx: &mut Context<Self>,
    ) {
        if self.write_status.is_sending() {
            return;
        }
        self.write_status = if prepared.is_report() {
            WriteStatus::SendingReport
        } else {
            WriteStatus::SendingMessage
        };
        cx.notify();

        let descriptor = self.descriptor.clone();
        let work = cx.background_spawn(async move {
            let identity_service = omega_identity::IdentityService::system(*app_identity::CHANNEL);
            let authorization = identity_service
                .take_activated_identity_action(&intent)
                .map_err(anyhow::Error::from)?;
            let signed =
                sign_prepared_write(&identity_service, &descriptor, prepared, &authorization)?;
            publish_signed_write(&descriptor, &signed)?;
            anyhow::Ok(signed)
        });
        self.write_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            if let Err(error) = this.update_in(cx, move |this, window, cx| match result {
                Ok(signed) => this.finish_write(signed, window, cx),
                Err(error) => {
                    this.write_status = WriteStatus::Failed(format!(
                        "The prepared write was not sent after identity setup: {error}"
                    ));
                    cx.notify();
                }
            }) {
                log::debug!(
                    "prepared public-channel write finished after its view closed: {error:#}"
                );
            }
        }));
    }

    fn finish_write(
        &mut self,
        signed: SignedPublicChannelWrite,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if signed.is_report() {
            self.write_status = WriteStatus::ReportSent;
        } else {
            let event_id = signed.record().id.clone();
            let mut snapshot = self.relay_snapshot.clone();
            snapshot.events.push(signed.record().clone());
            self.write_status =
                WriteStatus::MessageSent(event_id.get(..12).unwrap_or(&event_id).to_string());
            self.composer.update(cx, |editor, cx| {
                editor.set_text("", window, cx);
            });
            self.apply_relay_snapshot(snapshot, cx);
        }
        cx.notify();
    }

    fn confirm_report(
        &mut self,
        event_id: String,
        author_public_key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.write_status.is_sending() {
            return;
        }
        let prompt = window.prompt(
            PromptLevel::Warning,
            "Report this public message?",
            Some(
                "The report is a public signed signal containing the message and author IDs. It does not copy the message body or remove the message; moderators decide what happens next.",
            ),
            &["Cancel", "Report"],
            cx,
        );
        cx.spawn(async move |this, cx| match prompt.await {
            Ok(1) => {
                if let Err(error) = this.update_in(cx, |this, _window, cx| {
                    this.start_write(
                        PublicChannelWrite::Report {
                            event_id,
                            author_public_key,
                        },
                        cx,
                    );
                }) {
                    log::debug!(
                        "tester-channel report was confirmed after the view closed: {error:#}"
                    );
                }
            }
            Ok(_) => {}
            Err(error) => log::debug!("tester-channel report prompt closed: {error:#}"),
        })
        .detach();
    }

    fn retry_relay(&mut self, cx: &mut Context<Self>) {
        self.pause(cx);
        self.resume(cx);
    }

    fn render_relay_fallback(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.relay_outage_observed.then(|| {
            v_flex()
                .id("omega-tester-channel-relay-fallback")
                .debug_selector(|| "omega-tester-channel-relay-fallback".to_string())
                .mx_3()
                .mt_2()
                .p_3()
                .gap_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().colors().border)
                .child(
                    Label::new("The public relay is unavailable; verified messages remain visible")
                        .size(LabelSize::Small)
                        .line_clamp(3),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("omega-tester-channel-retry-relay", "Retry relay")
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Compact)
                                .on_click(cx.listener(|this, _, _, cx| this.retry_relay(cx))),
                        )
                        .child(
                            Button::new("omega-tester-channel-open-support", "Open support")
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Compact)
                                .on_click(|_, _, cx| {
                                    cx.open_url(app_identity::PRODUCT_BUG_REPORT_URL)
                                }),
                        ),
                )
                .into_any_element()
        })
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let content_bytes = self.composer.read(cx).text(cx).trim().len();
        let disabled = content_bytes == 0
            || content_bytes > self.descriptor.limits.content_bytes
            || self.write_status.is_sending();
        v_flex()
            .id("omega-tester-channel-composer")
            .debug_selector(|| "omega-tester-channel-composer".to_string())
            .flex_none()
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .border_t_1()
            .border_color(cx.theme().colors().border)
            .child(
                Label::new(
                    "Public channel. Messages and reports are signed with your Omega identity and may be retained. Don’t post secrets, credentials, private code, customer data, prompts, local paths, or unredacted logs. Moderators may remove messages, but deletion cannot guarantee erasure.",
                )
                .size(LabelSize::XSmall)
                .color(Color::Muted)
                .line_clamp(5),
            )
            .child(
                div()
                    .w_full()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .child(self.composer.clone()),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        v_flex()
                            .id("omega-tester-channel-write-status")
                            .debug_selector(|| "omega-tester-channel-write-status".to_string())
                            .role(Role::Status)
                            .gap_0p5()
                            .child(
                                Label::new(self.write_status.label())
                                    .size(LabelSize::XSmall)
                                    .line_clamp(3)
                                    .color(if matches!(self.write_status, WriteStatus::Failed(_)) {
                                        Color::Error
                                    } else {
                                        Color::Muted
                                    }),
                            )
                            .child(
                                Label::new(format!(
                                    "{content_bytes} / {} bytes",
                                    self.descriptor.limits.content_bytes
                                ))
                                .size(LabelSize::XSmall)
                                .color(if content_bytes > self.descriptor.limits.content_bytes {
                                    Color::Error
                                } else {
                                    Color::Muted
                                }),
                            ),
                    )
                    .child(
                        Button::new("omega-tester-channel-send", "Send public feedback")
                            .style(ButtonStyle::Filled)
                            .size(ButtonSize::Compact)
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, _, cx| this.send_composer(cx))),
                    ),
            )
            .into_any_element()
    }

    fn begin_media_load(
        &mut self,
        key: PublicChannelMediaKey,
        row_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(media) = self
            .projection
            .rows
            .iter()
            .find(|row| row.event_id == key.event_id)
            .and_then(|row| row.media.get(key.attachment_index))
        else {
            return;
        };
        let attachment = match PublicChannelAttachment::try_from_media_fact(
            media,
            self.descriptor.limits.attachment_bytes,
        ) {
            Ok(attachment) => attachment,
            Err(reason) => {
                self.media_states
                    .insert(key, PublicChannelMediaState::Unavailable { reason });
                self.list_state.remeasure_items(row_index..row_index + 1);
                cx.notify();
                return;
            }
        };
        let state = self.media_states.entry(key.clone()).or_default();
        if !state.begin_load() {
            return;
        }
        self.list_state.remeasure_items(row_index..row_index + 1);
        cx.notify();

        let generation = self.generation;
        let max_bytes = self.descriptor.limits.attachment_bytes;
        let http_client = self.http_client.clone();
        let fetch = {
            #[cfg(test)]
            {
                if self.media_fetch_pending {
                    cx.background_spawn(std::future::pending())
                } else if let Some(state) = self.media_fetch_result.clone() {
                    cx.background_spawn(async move { state })
                } else {
                    cx.background_spawn(fetch_public_channel_media(
                        http_client,
                        attachment,
                        max_bytes,
                    ))
                }
            }
            #[cfg(not(test))]
            {
                cx.background_spawn(fetch_public_channel_media(
                    http_client,
                    attachment,
                    max_bytes,
                ))
            }
        };
        let key_for_task = key.clone();
        let task = cx.spawn(async move |this, cx| {
            let state = fetch.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(row_index) = this
                    .projection
                    .rows
                    .iter()
                    .position(|row| row.event_id == key_for_task.event_id)
                else {
                    return;
                };
                if let Some(current) = this.media_states.get_mut(&key_for_task)
                    && matches!(current, PublicChannelMediaState::Loading)
                {
                    *current = state;
                    this.list_state.remeasure_items(row_index..row_index + 1);
                    cx.notify();
                }
            })
            .unwrap_or_else(|error| {
                log::debug!("public channel view disappeared while media loaded: {error:#}");
            });
        });
        self.media_tasks.insert(key, task);
    }

    fn execute_media_intent(&mut self, key: &PublicChannelMediaKey, cx: &mut Context<Self>) {
        let Some(PublicChannelMediaState::Verified(media)) = self.media_states.get(key) else {
            return;
        };
        match media.intent() {
            PublicChannelMediaIntent::InlineImage(_) => {}
            PublicChannelMediaIntent::OpenWithSystem { path } => cx.open_with_system(&path),
            PublicChannelMediaIntent::SaveAs {
                source,
                suggested_name,
            } => {
                let receiver = cx.prompt_for_new_path(Path::new(""), Some(&suggested_name));
                cx.spawn(async move |_, _| {
                    if let Ok(Ok(Some(destination))) = receiver.await
                        && let Err(error) = smol::fs::copy(source, destination).await
                    {
                        log::info!("verified public channel media could not be saved: {error}");
                    }
                })
                .detach();
            }
        }
    }

    fn render_lifecycle_banner(&self) -> Option<Banner> {
        let (severity, text): (Severity, SharedString) =
            if let Some(reason) = self.relay_snapshot.gap_reason {
                (Severity::Warning, reason.gap_label().into())
            } else {
                match self.relay_snapshot.lifecycle {
                    RelayLifecycle::Disconnected => {
                        (Severity::Info, "This channel is not connected.".into())
                    }
                    RelayLifecycle::Connecting => {
                        (Severity::Info, "Connecting to signed relay history.".into())
                    }
                    RelayLifecycle::Replaying => (
                        Severity::Info,
                        "Repairing history until all required EOSE frames arrive.".into(),
                    ),
                    RelayLifecycle::Current => return None,
                    RelayLifecycle::Reconnecting => (
                        Severity::Warning,
                        "Reconnecting and repairing history.".into(),
                    ),
                    RelayLifecycle::Stale => (
                        Severity::Warning,
                        "Verified messages remain visible, but history can be stale.".into(),
                    ),
                }
            };
        Some(
            Banner::new()
                .severity(severity)
                .wrap_content(true)
                .child(Label::new(text).size(LabelSize::Small)),
        )
    }

    fn render_metadata_banner(&self) -> Option<Banner> {
        (!self.relay_snapshot.metadata_trusted).then(|| {
            Banner::new()
                .severity(Severity::Warning)
                .wrap_content(true)
                .child(
                    Label::new("Messages are verified; group metadata is not authenticated")
                        .size(LabelSize::Small),
                )
        })
    }

    fn render_timeline_row(
        &mut self,
        row_index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(row) = self.projection.rows.get(row_index).cloned() else {
            return div().into_any_element();
        };
        let event_id = row.event_id.clone();
        let inspect_event_id = event_id.clone();
        let inspect_selector = format!("inspect-{event_id}");
        let author = row
            .profile
            .as_ref()
            .and_then(|profile| profile.display_name.clone())
            .unwrap_or_else(|| omega_nostr_activity::short_key(&row.public_key));
        let bot = row.profile.as_ref().is_some_and(|profile| profile.bot);

        let mut card = v_flex()
            .id(SharedString::from(format!(
                "omega-public-channel-event-{}",
                row.event_id
            )))
            .mx_4()
            .my_1()
            .p_3()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Label::new(author).size(LabelSize::Small))
                            .when(bot, |this| {
                                this.child(
                                    Label::new("Agent")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                            })
                            .when(row.pinned, |this| {
                                this.child(
                                    Label::new("Pinned")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Accent),
                                )
                            }),
                    )
                    .child(
                        h_flex().debug_selector(move || inspect_selector).child(
                            Button::new(
                                SharedString::from(format!("inspect-{}", row.event_id)),
                                "Inspect",
                            )
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.inspect_event(&inspect_event_id, cx);
                                },
                            )),
                        ),
                    ),
            );

        if let Some(deletion) = row.deletion {
            let text = match deletion {
                DeletionKind::Author => "This message was removed by its author.",
                DeletionKind::Moderator => "This message was removed by a moderator.",
            };
            return card
                .child(Label::new(text).size(LabelSize::Small).color(Color::Muted))
                .into_any_element();
        }

        if row.content_warning && !self.revealed_content_warnings.contains(&row.event_id) {
            let reveal_event_id = event_id.clone();
            let reveal_selector = format!("reveal-{event_id}");
            return card
                .child(
                    Label::new("Content warning")
                        .size(LabelSize::Small)
                        .color(Color::Warning),
                )
                .child(
                    h_flex().debug_selector(move || reveal_selector).child(
                        Button::new(
                            SharedString::from(format!("reveal-{}", row.event_id)),
                            "Show content",
                        )
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.reveal_content(&reveal_event_id, row_index, cx);
                        })),
                    ),
                )
                .into_any_element();
        }

        let content = row.content_parts.iter().enumerate().fold(
            h_flex().min_w_0().flex_wrap().gap_0p5(),
            |content, (part_index, part)| match part {
                ContentPart::Text(text) => {
                    content.child(Label::new(text.clone()).size(LabelSize::Small))
                }
                ContentPart::HttpLink(url) => {
                    let url = url.clone();
                    content.child(
                        Button::new(
                            SharedString::from(format!("event-{}-link-{part_index}", row.event_id)),
                            url.clone(),
                        )
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.open_url(&url);
                        }),
                    )
                }
                ContentPart::NostrReference(reference) => content.child(
                    Label::new(reference.clone())
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            },
        );
        card = card.child(content);
        if row.kind == 1337 {
            card = card.child(
                Label::new("Code message")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
        }
        if !row.reactions.is_empty() {
            card = card.child(
                h_flex()
                    .gap_1()
                    .children(row.reactions.iter().map(|reaction| {
                        Label::new(format!("{} {}", reaction.value, reaction.count))
                            .size(LabelSize::XSmall)
                    })),
            );
        }
        for (attachment_index, media) in row.media.iter().enumerate() {
            let key = PublicChannelMediaKey::new(
                self.descriptor.channel_id.clone(),
                row.event_id.clone(),
                attachment_index,
            );
            card = card.child(self.render_media(&key, media, row_index, cx));
        }
        card.into_any_element()
    }

    fn render_media(
        &self,
        key: &PublicChannelMediaKey,
        media: &MediaFact,
        row_index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.media_states.get(key);
        let lifecycle = state
            .map(PublicChannelMediaState::lifecycle)
            .unwrap_or(PublicChannelMediaLifecycle::Unavailable);
        let lifecycle_selector = format!(
            "omega-public-channel-media-state-{}",
            match lifecycle {
                PublicChannelMediaLifecycle::Gated => "gated",
                PublicChannelMediaLifecycle::Loading => "loading",
                PublicChannelMediaLifecycle::Verified => "verified",
                PublicChannelMediaLifecycle::Mismatch => "mismatch",
                PublicChannelMediaLifecycle::Unavailable => "unavailable",
            }
        );
        let mut card = v_flex()
            .id(SharedString::from(format!(
                "media-{}-{}",
                key.event_id, key.attachment_index
            )))
            .debug_selector(move || lifecycle_selector)
            .p_2()
            .gap_1()
            .rounded_sm()
            .border_1()
            .border_color(cx.theme().colors().border)
            .child(
                Label::new(media.alt.clone().unwrap_or_else(|| media.mime_type.clone()))
                    .size(LabelSize::Small),
            )
            .child(
                Label::new(lifecycle.label())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
        match state {
            Some(PublicChannelMediaState::Verified(verified)) => {
                match &verified.presentation {
                    crate::omega_public_channel_media::PublicChannelMediaPresentation::InlineImage(
                        image,
                    ) => {
                        card = card.child(
                            img(ImageSource::Render(image.clone()))
                                .max_h(px(360.))
                                .w_full()
                                .object_fit(ObjectFit::Contain),
                        );
                    }
                    crate::omega_public_channel_media::PublicChannelMediaPresentation::OpenWithSystem
                    | crate::omega_public_channel_media::PublicChannelMediaPresentation::SaveOnly => {
                        let key = key.clone();
                        let label = verified.presentation.label();
                        card = card.child(
                            Button::new(
                                SharedString::from(format!(
                                    "open-media-{}-{}",
                                    key.event_id, key.attachment_index
                                )),
                                label,
                            )
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.execute_media_intent(&key, cx);
                            })),
                        );
                    }
                }
            }
            Some(PublicChannelMediaState::Mismatch { .. }) => {
                card = card.child(
                    Label::new("The signed digest does not match these bytes.")
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                );
            }
            Some(PublicChannelMediaState::Unavailable { reason }) => {
                card = card.child(
                    Label::new(reason.label())
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                );
            }
            Some(PublicChannelMediaState::Loading) => {}
            Some(PublicChannelMediaState::Gated) => {
                let key = key.clone();
                let load_selector = format!(
                    "load-media-{}-{}",
                    key.event_id, key.attachment_index
                );
                card = card.child(
                    h_flex()
                        .debug_selector(move || load_selector)
                        .child(
                            Button::new(
                                SharedString::from(format!(
                                    "load-media-{}-{}",
                                    key.event_id, key.attachment_index
                                )),
                                "Load media",
                            )
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.begin_media_load(key.clone(), row_index, cx);
                            })),
                        ),
                    )
                ;
            }
            None => {
                card = card.child(
                    Label::new(PublicChannelMediaUnavailableReason::UnsafeMetadata.label())
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                );
            }
        }
        card.into_any_element()
    }

    fn render_event_facts(&self, facts: EventFacts, cx: &mut Context<Self>) -> AnyElement {
        let event_id = facts.event_id.clone();
        let report_event_id = facts.event_id.clone();
        let report_author_public_key = facts.public_key.clone();
        let media = facts.media;
        let deletion = facts
            .deletion
            .map(|deletion| match deletion {
                DeletionKind::Author => "Author",
                DeletionKind::Moderator => "Moderator",
            })
            .unwrap_or("No");
        let signature = match facts.signature_state {
            SignatureState::Verified => "Verified",
        };
        let rows = [
            ("Public key", facts.public_key),
            ("Event ID", facts.event_id),
            ("Kind", facts.kind.to_string()),
            ("Relay", facts.relay_url),
            ("Group", facts.group_id),
            ("Signature", signature.to_string()),
            ("Created at", facts.created_at.to_string()),
            (
                "Pinned",
                if facts.pinned { "Yes" } else { "No" }.to_string(),
            ),
            ("Deletion", deletion.to_string()),
            ("Media", media.len().to_string()),
        ];
        let mut pane = v_flex()
            .id("omega-public-channel-event-facts")
            .debug_selector(|| "omega-public-channel-event-facts".to_string())
            .size_full()
            .p_3()
            .gap_2()
            .border_l_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Event facts").size(LabelSize::Large))
                    .child(
                        IconButton::new("close-public-channel-event-facts", IconName::Close)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Close event facts"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_event_facts(cx);
                            })),
                    ),
            )
            .children(rows.into_iter().enumerate().map(|(index, (label, value))| {
                v_flex()
                    .gap_0p5()
                    .child(
                        Label::new(label)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        h_flex()
                            .min_w_0()
                            .justify_between()
                            .gap_1()
                            .child(Label::new(value.clone()).size(LabelSize::Small))
                            .child(CopyButton::new(
                                SharedString::from(format!("copy-event-fact-{index}")),
                                value,
                            )),
                    )
            }));
        for (media_index, fact) in media.into_iter().enumerate() {
            let key = PublicChannelMediaKey::new(
                self.descriptor.channel_id.clone(),
                event_id.clone(),
                media_index,
            );
            let status = self
                .media_states
                .get(&key)
                .map(PublicChannelMediaState::lifecycle)
                .map(PublicChannelMediaLifecycle::label)
                .unwrap_or("Media unavailable");
            let media_rows = [
                ("URL", fact.url),
                ("MIME", fact.mime_type),
                (
                    "Digest",
                    fact.digest.unwrap_or_else(|| "Not supplied".to_string()),
                ),
                (
                    "Size",
                    fact.size
                        .map(|size| size.to_string())
                        .unwrap_or_else(|| "Not supplied".to_string()),
                ),
                ("Status", status.to_string()),
            ];
            pane = pane.child(
                v_flex()
                    .id(SharedString::from(format!(
                        "omega-public-channel-media-facts-{media_index}"
                    )))
                    .debug_selector(move || {
                        format!("omega-public-channel-media-facts-{media_index}")
                    })
                    .mt_2()
                    .gap_1()
                    .child(Label::new(format!("Media {}", media_index + 1)).size(LabelSize::Small))
                    .children(media_rows.into_iter().enumerate().map(
                        |(field_index, (label, value))| {
                            v_flex()
                                .gap_0p5()
                                .child(
                                    Label::new(label)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    h_flex()
                                        .min_w_0()
                                        .justify_between()
                                        .gap_1()
                                        .child(Label::new(value.clone()).size(LabelSize::Small))
                                        .child(CopyButton::new(
                                            SharedString::from(format!(
                                                "copy-media-fact-{media_index}-{field_index}"
                                            )),
                                            value,
                                        )),
                                )
                        },
                    )),
            );
        }
        pane = pane.child(
            h_flex()
                .debug_selector(|| "omega-tester-channel-report".to_string())
                .child(
                    Button::new("omega-tester-channel-report", "Report message")
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .disabled(self.write_status.is_sending())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_report(
                                report_event_id.clone(),
                                report_author_public_key.clone(),
                                window,
                                cx,
                            );
                        })),
                ),
        );
        pane.into_any_element()
    }

    fn render_empty_or_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.projection.rows.is_empty() {
            return match self.relay_snapshot.lifecycle {
                RelayLifecycle::Connecting | RelayLifecycle::Replaying => v_flex()
                    .id("omega-public-channel-history-loading")
                    .debug_selector(|| "omega-public-channel-history-loading".to_string())
                    .size_full()
                    .p_4()
                    .gap_3()
                    .child(
                        Label::new(
                            if matches!(self.relay_snapshot.lifecycle, RelayLifecycle::Connecting) {
                                "Loading signed history…"
                            } else {
                                "Repairing signed history…"
                            },
                        )
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                    )
                    .children((0..3).map(|index| {
                        div()
                            .id(SharedString::from(format!(
                                "omega-public-channel-loading-row-{index}"
                            )))
                            .h(px(52.))
                            .w_full()
                            .rounded_md()
                            .bg(cx.theme().colors().border.opacity(0.24))
                    }))
                    .into_any_element(),
                RelayLifecycle::Current => v_flex()
                    .id("omega-public-channel-quiet")
                    .debug_selector(|| "omega-public-channel-quiet".to_string())
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        Label::new("This channel is quiet.")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .into_any_element(),
                RelayLifecycle::Reconnecting => v_flex()
                    .id("omega-public-channel-reconnecting-empty")
                    .debug_selector(|| "omega-public-channel-reconnecting-empty".to_string())
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        Label::new("Reconnecting to repair signed history…")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .into_any_element(),
                RelayLifecycle::Stale => v_flex()
                    .id("omega-public-channel-stale-empty")
                    .debug_selector(|| "omega-public-channel-stale-empty".to_string())
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        Label::new("No verified messages are cached.")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .into_any_element(),
                RelayLifecycle::Disconnected => v_flex()
                    .id("omega-public-channel-disconnected-empty")
                    .debug_selector(|| "omega-public-channel-disconnected-empty".to_string())
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        Label::new("Select this channel to read its signed history.")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .into_any_element(),
            };
        }
        let list_state = self.list_state.clone();
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .px_4()
                    .py_1()
                    .child(
                        h_flex()
                            .debug_selector(|| "omega-public-channel-load-older".to_string())
                            .child(
                                Button::new("omega-public-channel-load-older", "Load older")
                                    .style(ButtonStyle::Subtle)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|this, _, _, _| this.load_older())),
                            ),
                    )
                    .when(!self.list_state.is_following_tail(), |this| {
                        this.child(
                            Button::new("omega-public-channel-jump-latest", "Jump to latest")
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Compact)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.list_state.scroll_to_end();
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                list(
                    self.list_state.clone(),
                    cx.processor(Self::render_timeline_row),
                )
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .flex_1()
                .size_full(),
            )
            .custom_scrollbars(
                Scrollbars::new(ScrollAxes::Vertical).tracked_scroll_handle(&list_state),
                window,
                cx,
            )
            .into_any_element()
    }

    fn begin_sarah_control(&mut self, control: CommunitySarahControl, cx: &mut Context<Self>) {
        let nonce = matches!(
            control,
            CommunitySarahControl::Talk | CommunitySarahControl::ModeratorStop
        )
        .then(|| uuid::Uuid::new_v4().simple().to_string());
        match self.sarah_room.begin(control, nonce.as_deref()) {
            Ok(intent) => {
                self.execute_sarah_intent(intent, cx);
                cx.notify();
            }
            Err(_) => {
                self.sarah_room.fail_closed("Room voice authority changed.");
                cx.notify();
            }
        }
    }

    fn execute_sarah_intent(&mut self, intent: CommunitySarahIntent, cx: &mut Context<Self>) {
        if matches!(intent, CommunitySarahIntent::Leave) {
            self.sarah_control_task = None;
            self.stop_sarah_media();
            self.sarah_room.leave();
            return;
        }
        if let CommunitySarahIntent::SetMuted(muted) = intent {
            if let Some(controls) = &self.sarah_media_controls
                && let Err(error) = controls.try_send(CommunityRoomMediaControl::SetMuted(
                    muted || !self.local_participant_holds_floor(),
                ))
            {
                log::debug!("community room mute control was not delivered: {error}");
                self.sarah_room
                    .fail_closed("Room voice media stopped unexpectedly.");
                self.stop_sarah_media();
            }
            return;
        }
        let Some(session) = omega_effectd::openagents_session_if_initialized(cx) else {
            self.sarah_room
                .fail_closed("Connect an OpenAgents account to use room voice.");
            return;
        };
        let Some(context) = self.sarah_room.context.clone() else {
            self.sarah_room
                .fail_closed("Room voice context is unavailable.");
            return;
        };
        let authority = self.sarah_room.authority.clone();
        self.sarah_control_task = Some(cx.spawn(async move |this, cx| {
            let result: anyhow::Result<Option<CommunityRoomAdmission>> = async {
                match intent {
                    CommunitySarahIntent::Join => {}
                    CommunitySarahIntent::Summon => {
                        let authority = authority.as_ref().ok_or_else(|| {
                            anyhow::anyhow!("room voice authority is unavailable")
                        })?;
                        session
                            .community_sarah_request::<serde_json::Value>(
                                "/api/sarah/livekit/room/summon",
                                &serde_json::json!({
                                    "presenceLeaseRef": authority.presence_lease_ref,
                                    "expectedRevision": authority.revision,
                                }),
                                cx,
                            )
                            .await?;
                    }
                    CommunitySarahIntent::Remove => {
                        let authority = authority.as_ref().ok_or_else(|| {
                            anyhow::anyhow!("room voice authority is unavailable")
                        })?;
                        session
                            .community_sarah_request::<serde_json::Value>(
                                "/api/sarah/livekit/room/remove",
                                &serde_json::json!({
                                    "presenceLeaseRef": authority.presence_lease_ref,
                                    "expectedRevision": authority.revision,
                                }),
                                cx,
                            )
                            .await?;
                        return Ok(None);
                    }
                    CommunitySarahIntent::AcquireFloor { body }
                    | CommunitySarahIntent::TransferFloor { body } => {
                        session
                            .community_sarah_request::<serde_json::Value>(
                                "/api/sarah/livekit/room/floor/member",
                                &body,
                                cx,
                            )
                            .await?;
                    }
                    CommunitySarahIntent::ModeratorStop { body } => {
                        session
                            .community_sarah_request::<serde_json::Value>(
                                "/api/sarah/livekit/room/floor/moderator",
                                &body,
                                cx,
                            )
                            .await?;
                    }
                    CommunitySarahIntent::Leave | CommunitySarahIntent::SetMuted(_) => {
                        return Ok(None);
                    }
                }
                let admission = session
                    .community_sarah_request::<CommunityRoomAdmission>(
                        "/api/sarah/livekit/room/join",
                        &serde_json::json!({
                            "communityRef": context.community_ref,
                            "channelRef": context.channel_ref,
                        }),
                        cx,
                    )
                    .await?;
                admission.validate(&context, unix_time_millis())?;
                Ok(Some(admission))
            }
            .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(Some(admission)) => {
                        let role = admission.role;
                        let media_running = this.sarah_media_controls.is_some();
                        let authority_result = if media_running {
                            this.sarah_room.refresh_authority(
                                admission.authority.clone(),
                                role,
                                unix_time_millis(),
                            )
                        } else {
                            this.sarah_room.apply_authority(
                                admission.authority.clone(),
                                role,
                                CommunitySarahState::Idle,
                                unix_time_millis(),
                            )
                        };
                        match authority_result {
                            Ok(()) => {
                                if media_running {
                                    this.update_sarah_media_authority();
                                } else if let Err(error) =
                                    this.start_sarah_media(admission, session.clone(), cx)
                                {
                                    log::error!("community Sarah media could not start: {error:#}");
                                    this.sarah_room
                                        .fail_closed("Room voice media could not start.");
                                    this.stop_sarah_media();
                                }
                            }
                            Err(error) => {
                                log::error!("community Sarah authority was refused: {error:#}");
                            }
                        }
                    }
                    Ok(None) => {
                        this.stop_sarah_media();
                        this.sarah_room.leave();
                    }
                    Err(error) => {
                        log::error!("community Sarah control failed: {error:#}");
                        this.sarah_room
                            .fail_closed("Room voice could not verify its authority.");
                        this.stop_sarah_media();
                    }
                }
                cx.notify();
            })
            .unwrap_or_else(|error| {
                log::debug!("public channel disappeared during a Sarah control: {error:#}");
            });
        }));
    }

    fn start_sarah_media(
        &mut self,
        admission: CommunityRoomAdmission,
        session: omega_effectd::OpenAgentsSession,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.stop_sarah_media();
        let context = self
            .sarah_room
            .context
            .clone()
            .ok_or_else(|| anyhow::anyhow!("community room context is unavailable"))?;
        let (media, playback) = start_community_room_media(admission, cx)?;
        let controls = media.controls.clone();
        let media_events = media.events;
        let media_runtime_task = media.task;
        self.sarah_media_controls = Some(controls.clone());
        self.sarah_room.media_connecting();
        self.sarah_media_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = media_events.recv().await {
                let should_stop = match event {
                    CommunityRoomMediaEvent::Connected | CommunityRoomMediaEvent::Reconnected => {
                        if let Err(error) = this.update(cx, |this, cx| {
                            this.sarah_room.media_connected();
                            cx.notify();
                        }) {
                            log::debug!(
                                "public channel disappeared during community media connect: {error:#}"
                            );
                            break;
                        }
                        false
                    }
                    CommunityRoomMediaEvent::Reconnecting => {
                        if let Err(error) = this.update(cx, |this, cx| {
                            this.sarah_room.media_connecting();
                            cx.notify();
                        }) {
                            log::debug!(
                                "public channel disappeared during community media reconnect: {error:#}"
                            );
                            break;
                        }
                        false
                    }
                    CommunityRoomMediaEvent::RosterRefreshRequired => false,
                    CommunityRoomMediaEvent::SarahSpeaking(speaking) => {
                        if let Err(error) = this.update(cx, |this, cx| {
                            this.sarah_room.set_sarah_speaking(speaking);
                            cx.notify();
                        }) {
                            log::debug!(
                                "public channel disappeared during Sarah speaker update: {error:#}"
                            );
                            break;
                        }
                        false
                    }
                    CommunityRoomMediaEvent::Audio(bytes) => {
                        if let Err(error) = playback.play(&bytes) {
                            log::error!("community Sarah audio playback failed: {error:#}");
                            true
                        } else {
                            false
                        }
                    }
                    CommunityRoomMediaEvent::Ended => true,
                    CommunityRoomMediaEvent::Error(message) => {
                        log::error!("community Sarah media failed: {message}");
                        true
                    }
                };
                if should_stop {
                    if let Err(error) = this.update(cx, |this, cx| {
                        this.sarah_room
                            .fail_closed("Room voice media disconnected.");
                        this.sarah_media_controls = None;
                        this.sarah_refresh_task = None;
                        cx.notify();
                    }) {
                        log::debug!(
                            "public channel disappeared during community media teardown: {error:#}"
                        );
                    }
                    break;
                }
            }
            if let Err(error) = media_runtime_task.await {
                log::debug!("community Sarah media task failed to join: {error}");
            }
        }));
        self.sarah_refresh_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let admission = match session
                    .community_sarah_request::<CommunityRoomAdmission>(
                        "/api/sarah/livekit/room/join",
                        &serde_json::json!({
                            "communityRef": context.community_ref,
                            "channelRef": context.channel_ref,
                        }),
                        cx,
                    )
                    .await
                {
                    Ok(admission) => admission,
                    Err(error) => {
                        log::error!("community Sarah authority refresh failed: {error:#}");
                        if controls
                            .send(CommunityRoomMediaControl::Close)
                            .await
                            .is_err()
                        {
                            log::debug!("community Sarah media stopped before authority failure");
                        }
                        if let Err(error) = this.update(cx, |this, cx| {
                            this.sarah_room
                                .fail_closed("Room voice authority could not be refreshed.");
                            this.sarah_media_controls = None;
                            cx.notify();
                        }) {
                            log::debug!(
                                "public channel disappeared during authority failure: {error:#}"
                            );
                        }
                        return;
                    }
                };
                if let Err(error) = admission.validate(&context, unix_time_millis()) {
                    log::error!("community Sarah authority refresh was invalid: {error:#}");
                    if controls
                        .send(CommunityRoomMediaControl::Close)
                        .await
                        .is_err()
                    {
                        log::debug!("community Sarah media stopped before invalid authority");
                    }
                    if let Err(error) = this.update(cx, |this, cx| {
                        this.sarah_room
                            .fail_closed("Room voice authority refresh was invalid.");
                        this.sarah_media_controls = None;
                        cx.notify();
                    }) {
                        log::debug!(
                            "public channel disappeared during invalid authority: {error:#}"
                        );
                    }
                    return;
                }
                let role = admission.role;
                let authority = admission.authority;
                let refresh_accepted = match this.update(cx, |this, cx| {
                    if let Err(error) =
                        this.sarah_room
                            .refresh_authority(authority, role, unix_time_millis())
                    {
                        log::error!("community Sarah authority refresh was refused: {error:#}");
                        this.sarah_media_controls = None;
                        cx.notify();
                        false
                    } else {
                        this.update_sarah_media_authority();
                        cx.notify();
                        true
                    }
                }) {
                    Ok(accepted) => accepted,
                    Err(_) => return,
                };
                if !refresh_accepted {
                    if controls
                        .send(CommunityRoomMediaControl::Close)
                        .await
                        .is_err()
                    {
                        log::debug!("community Sarah media stopped before stale authority");
                    }
                    return;
                }
            }
        }));
        Ok(())
    }

    fn update_sarah_media_authority(&self) {
        let Some(controls) = &self.sarah_media_controls else {
            return;
        };
        let Some(authority) = &self.sarah_room.authority else {
            return;
        };
        let participants = authority
            .verified_participants
            .iter()
            .map(|participant| participant.participant_ref.clone())
            .collect();
        if let Err(error) = controls.try_send(
            CommunityRoomMediaControl::UpdateVerifiedParticipants(participants),
        ) {
            log::debug!("community Sarah roster update was not delivered: {error}");
        }
        if let Err(error) = controls.try_send(CommunityRoomMediaControl::SetMuted(
            self.sarah_room.muted || !self.local_participant_holds_floor(),
        )) {
            log::debug!("community Sarah floor mute was not delivered: {error}");
        }
    }

    fn local_participant_holds_floor(&self) -> bool {
        let Some(authority) = &self.sarah_room.authority else {
            return false;
        };
        matches!(
            &authority.floor,
            crate::omega_public_channel_sarah::CommunityFloorState::Held { lease }
                if lease.holder_participant_ref == authority.local_participant.participant_ref
                    && lease.holder_user_ref_digest
                        == authority.local_participant.user_ref_digest
        )
    }

    fn stop_sarah_media(&mut self) {
        if let Some(controls) = self.sarah_media_controls.take()
            && let Err(error) = controls.try_send(CommunityRoomMediaControl::Close)
        {
            log::debug!("community Sarah close control was not delivered: {error}");
        }
        if let Some(task) = self.sarah_media_task.take() {
            self.retired_sarah_media_tasks.push_back(task);
        }
        while self.retired_sarah_media_tasks.len() > RETIRED_SARAH_MEDIA_LIMIT {
            self.retired_sarah_media_tasks.pop_front();
        }
        self.sarah_refresh_task = None;
    }

    fn render_sarah_room_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.sarah_room;
        let status_color = if state.lifecycle == CommunityCallLifecycle::Failed {
            Color::Error
        } else if state.lifecycle == CommunityCallLifecycle::Joined {
            Color::Accent
        } else {
            Color::Muted
        };
        v_flex()
            .id("omega-public-channel-sarah")
            .debug_selector(|| "omega-public-channel-sarah".to_string())
            .flex_none()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .min_w_0()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex()
                            .min_w_0()
                            .flex_wrap()
                            .gap_2()
                            .child(Label::new("Room voice").size(LabelSize::Small))
                            .child(
                                Label::new(state.lifecycle.label())
                                    .size(LabelSize::XSmall)
                                    .color(status_color),
                            )
                            .child(
                                Label::new(state.sarah_state.label())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(state.floor_label())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        Label::new(state.microphone_label())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                h_flex()
                    .id("omega-public-channel-sarah-controls")
                    .debug_selector(|| "omega-public-channel-sarah-controls".to_string())
                    .min_w_0()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("omega-room-voice-join", "Join")
                            .debug_selector(|| "omega-room-voice-join".to_string())
                            .tab_index(0isize)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Filled)
                            .disabled(!state.control_enabled(CommunitySarahControl::Join))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(JoinRoom.boxed_clone(), cx);
                            }),
                    )
                    .child(
                        Button::new("omega-room-voice-leave", "Leave")
                            .debug_selector(|| "omega-room-voice-leave".to_string())
                            .tab_index(1isize)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .disabled(!state.control_enabled(CommunitySarahControl::Leave))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(LeaveRoom.boxed_clone(), cx);
                            }),
                    )
                    .child(
                        Button::new(
                            "omega-room-voice-mute",
                            if state.muted { "Unmute" } else { "Mute" },
                        )
                        .debug_selector(|| "omega-room-voice-mute".to_string())
                        .tab_index(2isize)
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Subtle)
                        .disabled(!state.control_enabled(CommunitySarahControl::Mute))
                        .on_click(|_, window, cx| {
                            window.dispatch_action(ToggleMute.boxed_clone(), cx);
                        }),
                    )
                    .child(
                        Button::new("omega-room-sarah-summon", "Summon Sarah")
                            .debug_selector(|| "omega-room-sarah-summon".to_string())
                            .tab_index(3isize)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .disabled(!state.control_enabled(CommunitySarahControl::Summon))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(SummonSarah.boxed_clone(), cx);
                            }),
                    )
                    .child(
                        Button::new("omega-room-sarah-remove", "Remove Sarah")
                            .debug_selector(|| "omega-room-sarah-remove".to_string())
                            .tab_index(4isize)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .disabled(!state.control_enabled(CommunitySarahControl::Remove))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(RemoveSarah.boxed_clone(), cx);
                            }),
                    )
                    .child(
                        Button::new("omega-room-sarah-talk", "Talk to Sarah")
                            .debug_selector(|| "omega-room-sarah-talk".to_string())
                            .tab_index(5isize)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Filled)
                            .disabled(!state.control_enabled(CommunitySarahControl::Talk))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(TalkToSarah.boxed_clone(), cx);
                            }),
                    )
                    .child(
                        Button::new("omega-room-sarah-stop", "Stop")
                            .debug_selector(|| "omega-room-sarah-stop".to_string())
                            .tab_index(6isize)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .disabled(!state.control_enabled(CommunitySarahControl::ModeratorStop))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(ModeratorStop.boxed_clone(), cx);
                            }),
                    ),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .id("omega-room-sarah-disclosure-copy")
                            .debug_selector(|| "omega-room-sarah-disclosure-copy".to_string())
                            .child(
                                Label::new("Sarah voice uses OpenAgents and OpenAI.")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        div()
                            .id("omega-room-sarah-disclosure-hitbox")
                            .debug_selector(|| "omega-room-sarah-disclosure-hitbox".to_string())
                            .child(
                                Button::new(
                                    "omega-room-sarah-disclosure",
                                    if state.disclosure_acknowledged {
                                        "Acknowledged"
                                    } else {
                                        "Acknowledge"
                                    },
                                )
                                .tab_index(7isize)
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Subtle)
                                .disabled(state.disclosure_acknowledged)
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.sarah_room.disclosure_acknowledged = true;
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            )
            .when_some(state.failure.clone(), |this, failure| {
                this.child(
                    div()
                        .id("omega-room-sarah-failure")
                        .debug_selector(|| "omega-room-sarah-failure".to_string())
                        .child(
                            Label::new(failure)
                                .size(LabelSize::XSmall)
                                .color(Color::Error),
                        ),
                )
            })
            .into_any_element()
    }
}

impl Render for PublicChannelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let facts = self.selected_facts();
        let compact = window.viewport_size().width < COMPACT_FACTS_THRESHOLD;
        let mut view = v_flex()
            .key_context("PublicChannelSarahRoom")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .on_action(cx.listener(|this, _: &JoinRoom, _, cx| {
                this.begin_sarah_control(CommunitySarahControl::Join, cx);
            }))
            .on_action(cx.listener(|this, _: &LeaveRoom, _, cx| {
                this.begin_sarah_control(CommunitySarahControl::Leave, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleMute, _, cx| {
                this.begin_sarah_control(CommunitySarahControl::Mute, cx);
            }))
            .on_action(cx.listener(|this, _: &SummonSarah, _, cx| {
                this.begin_sarah_control(CommunitySarahControl::Summon, cx);
            }))
            .on_action(cx.listener(|this, _: &RemoveSarah, _, cx| {
                this.begin_sarah_control(CommunitySarahControl::Remove, cx);
            }))
            .on_action(cx.listener(|this, _: &TalkToSarah, _, cx| {
                this.begin_sarah_control(CommunitySarahControl::Talk, cx);
            }))
            .on_action(cx.listener(|this, _: &ModeratorStop, _, cx| {
                this.begin_sarah_control(CommunitySarahControl::ModeratorStop, cx);
            }));
        if let Some(banner) = self.render_lifecycle_banner() {
            view = view.child(
                div()
                    .id("omega-public-channel-lifecycle-banner")
                    .debug_selector(|| "omega-public-channel-lifecycle-banner".to_string())
                    .px_3()
                    .pt_2()
                    .child(banner),
            );
        }
        if let Some(banner) = self.render_metadata_banner() {
            view = view.child(
                div()
                    .id("omega-public-channel-metadata-banner")
                    .debug_selector(|| "omega-public-channel-metadata-banner".to_string())
                    .px_3()
                    .pt_2()
                    .child(banner),
            );
        }
        if let Some(fallback) = self.render_relay_fallback(cx) {
            view = view.child(fallback);
        }
        view = view.child(self.render_sarah_room_controls(cx));
        if compact && let Some(facts) = facts {
            return view
                .child(
                    div()
                        .min_h_0()
                        .flex_1()
                        .overflow_hidden()
                        .child(self.render_event_facts(facts, cx)),
                )
                .child(self.render_composer(cx));
        }
        let timeline = self.render_empty_or_list(window, cx);
        view.child(
            h_flex()
                .min_h_0()
                .flex_1()
                .overflow_hidden()
                .child(div().min_w_0().h_full().flex_1().child(timeline))
                .when_some(facts, |this, facts| {
                    this.child(
                        div()
                            .w(FACTS_PANE_WIDTH)
                            .h_full()
                            .flex_none()
                            .child(self.render_event_facts(facts, cx)),
                    )
                }),
        )
        .child(self.render_composer(cx))
    }
}

impl Drop for PublicChannelView {
    fn drop(&mut self) {
        if let Some(sender) = self.relay_intent_sender.take() {
            if let Err(error) = sender.try_send(RelayIntent::Close) {
                log::debug!("public channel close intent was not delivered during drop: {error}");
            }
        }
    }
}

fn channel_lifecycle(lifecycle: RelayLifecycle) -> ChannelLifecycle {
    match lifecycle {
        RelayLifecycle::Disconnected => ChannelLifecycle::Disconnected,
        RelayLifecycle::Connecting => ChannelLifecycle::Connecting,
        RelayLifecycle::Replaying => ChannelLifecycle::Replaying,
        RelayLifecycle::Current => ChannelLifecycle::Current,
        RelayLifecycle::Reconnecting => ChannelLifecycle::Reconnecting,
        RelayLifecycle::Stale => ChannelLifecycle::Stale,
    }
}

fn merge_retained_snapshot(previous: &RelaySnapshot, next: &mut RelaySnapshot) {
    if next.last_current_at.is_none() {
        next.last_current_at = previous.last_current_at;
    }
    let mut events = previous.events.clone();
    events.extend(next.events.clone());
    next.events = stable_verified_events(&events);
    next.cursor = next.events.last().map(|latest| RelayCursor {
        created_at: latest.created_at,
        event_ids_at_created_at: next
            .events
            .iter()
            .filter(|event| event.created_at == latest.created_at)
            .map(|event| event.id.clone())
            .collect(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::omega_public_channel_sarah::{
        COMMUNITY_CAPABILITY_PROFILE, COMMUNITY_COHORT_POLICY, CommunityFloorState,
        CommunityRoomAuthority, CommunityRoomContext, CommunityRoomRole, CommunitySarahState,
        PROCESSOR_DISCLOSURE, ROOM_AUTHORITY_SCHEMA, SARAH_PRINCIPAL, VerifiedParticipantMapping,
    };
    use gpui::{Modifiers, TestAppContext, VisualTestContext, point, size};
    use http_client::FakeHttpClient;

    fn descriptor(channel_id: &str, relay_url: &str) -> ChannelDescriptor {
        ChannelDescriptor {
            schema_version: crate::omega_public_channels::DESCRIPTOR_SCHEMA.to_string(),
            channel_id: channel_id.to_string(),
            display_name: channel_id.to_string(),
            relay_url: relay_url.to_string(),
            group_id: "openagents-public".to_string(),
            accepted_kinds: vec![5, 7, 9, 1337, 1984],
            group_state_kinds: vec![39000, 39001, 39003, 39005],
            moderation_kinds: vec![9002, 9005, 9010],
            expected_relay_self_pubkey: None,
            relay_trust: crate::omega_public_channels::RelayTrust::MetadataUntrusted,
            limits: crate::omega_public_channels::ChannelLimits {
                attachment_count: 4,
                attachment_bytes: 1024,
                content_bytes: 8192,
                event_bytes: 32768,
                future_skew_seconds: 60,
                history_page_size: 50,
                max_age_seconds: 604800,
                tags: 64,
            },
            profile_version: "test".into(),
            rich_content_profile_version: "test".into(),
        }
    }

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    fn fixture_events() -> Vec<crate::omega_public_channel_timeline::NostrEventRecord> {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/agent-chat-parity.v1.json"))
                .expect("the pinned parity fixture");
        serde_json::from_value(fixture["projection"]["events"].clone()).expect("fixture events")
    }

    fn static_selector(selector: String) -> &'static str {
        Box::leak(selector.into_boxed_str())
    }

    fn community_sarah_authority() -> CommunityRoomAuthority {
        let digest = |byte: char| byte.to_string().repeat(64);
        let local_participant = VerifiedParticipantMapping {
            user_ref_digest: digest('b'),
            pubkey: digest('b'),
            participant_ref: "participant:local".into(),
            membership_revision: digest('a'),
            room_ref: "room:testers:voice".into(),
            room_epoch: 7,
        };
        CommunityRoomAuthority {
            schema: ROOM_AUTHORITY_SCHEMA.into(),
            principal: SARAH_PRINCIPAL.into(),
            capability_profile: COMMUNITY_CAPABILITY_PROFILE.into(),
            processor_disclosure: PROCESSOR_DISCLOSURE.into(),
            cohort_policy: COMMUNITY_COHORT_POLICY.into(),
            revision: 12,
            sarah_pubkey: digest('c'),
            presence_lease_ref: "presence:testers:7".into(),
            community_ref: "community:testers".into(),
            channel_ref: "channel:agent-chat".into(),
            membership_revision: digest('a'),
            e2ee_key_revision: digest('d'),
            room_ref: "room:testers:voice".into(),
            room_epoch: 7,
            sarah_participant_ref: SARAH_PRINCIPAL.into(),
            dispatch_ref: "dispatch:sarah".into(),
            session_ref: "session:sarah".into(),
            generation: 1,
            admission_digest: digest('e'),
            issued_at_ms: 900,
            expires_at_ms: 20_000,
            presence_active: true,
            local_participant: local_participant.clone(),
            verified_participants: vec![local_participant],
            floor: CommunityFloorState::Available {
                presence_lease_ref: "presence:testers:7".into(),
                issuance: 3,
            },
        }
    }

    #[test]
    fn lifecycle_mapping_keeps_all_six_visible_states_distinct() {
        assert_eq!(
            [
                RelayLifecycle::Disconnected,
                RelayLifecycle::Connecting,
                RelayLifecycle::Replaying,
                RelayLifecycle::Current,
                RelayLifecycle::Reconnecting,
                RelayLifecycle::Stale,
            ]
            .map(channel_lifecycle),
            [
                ChannelLifecycle::Disconnected,
                ChannelLifecycle::Connecting,
                ChannelLifecycle::Replaying,
                ChannelLifecycle::Current,
                ChannelLifecycle::Reconnecting,
                ChannelLifecycle::Stale,
            ]
        );
    }

    #[test]
    fn media_keys_include_the_channel_and_do_not_cross_equal_group_ids() {
        let left = PublicChannelMediaKey::new("agent-chat", "event", 0);
        let right = PublicChannelMediaKey::new("agent-lab", "event", 0);
        assert_ne!(left, right);
        assert_ne!(
            descriptor("agent-chat", "wss://relay.example").channel_id,
            descriptor("agent-lab", "wss://relay.example/lab").channel_id
        );
    }

    #[test]
    fn a_resumed_session_keeps_cached_verified_rows_and_last_current_time() {
        let events = fixture_events();
        let previous = RelaySnapshot {
            lifecycle: RelayLifecycle::Current,
            last_current_at: Some(42),
            events: events[..2].to_vec(),
            ..Default::default()
        };
        let mut resumed = RelaySnapshot {
            lifecycle: RelayLifecycle::Connecting,
            events: vec![events[1].clone(), events[2].clone()],
            ..Default::default()
        };
        merge_retained_snapshot(&previous, &mut resumed);
        assert_eq!(resumed.events.len(), 3);
        assert_eq!(resumed.last_current_at, Some(42));
        assert_eq!(
            resumed.cursor.as_ref().map(|cursor| cursor.created_at),
            resumed.events.last().map(|event| event.created_at)
        );
    }

    #[gpui::test]
    fn rendered_empty_and_error_states_keep_lifecycle_and_metadata_visible(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let window_handle = cx.add_window(|window, cx| {
            let http_client: Arc<dyn HttpClient> = FakeHttpClient::with_404_response();
            PublicChannelView::new(
                descriptor("agent-chat", "wss://relay.example"),
                http_client,
                window,
                cx,
            )
        });
        cx.run_until_parked();
        let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
        cx.simulate_resize(size(px(1200.), px(800.)));

        for (lifecycle, empty_selector) in [
            (
                RelayLifecycle::Disconnected,
                "omega-public-channel-disconnected-empty",
            ),
            (
                RelayLifecycle::Connecting,
                "omega-public-channel-history-loading",
            ),
            (
                RelayLifecycle::Replaying,
                "omega-public-channel-history-loading",
            ),
            (RelayLifecycle::Current, "omega-public-channel-quiet"),
            (
                RelayLifecycle::Reconnecting,
                "omega-public-channel-reconnecting-empty",
            ),
            (RelayLifecycle::Stale, "omega-public-channel-stale-empty"),
        ] {
            window_handle
                .update(&mut cx, |view, _, cx| {
                    view.apply_relay_snapshot(
                        RelaySnapshot {
                            lifecycle,
                            metadata_trusted: false,
                            ..Default::default()
                        },
                        cx,
                    );
                })
                .expect("update public channel view");
            cx.run_until_parked();
            assert!(
                cx.debug_bounds(empty_selector).is_some(),
                "{lifecycle:?} must have a distinct empty state"
            );
            assert!(
                cx.debug_bounds("omega-public-channel-metadata-banner")
                    .is_some(),
                "untrusted metadata must stay visible for {lifecycle:?}"
            );
            assert_eq!(
                cx.debug_bounds("omega-public-channel-lifecycle-banner")
                    .is_some(),
                lifecycle != RelayLifecycle::Current,
                "the lifecycle banner must match {lifecycle:?}"
            );
            assert!(
                cx.debug_bounds("omega-tester-channel-composer").is_some(),
                "the public privacy notice and composer must stay visible for {lifecycle:?}"
            );
            assert_eq!(
                cx.debug_bounds("omega-tester-channel-relay-fallback")
                    .is_some(),
                lifecycle == RelayLifecycle::Stale,
                "only a confirmed stale relay shows the independent support fallback"
            );
        }

        window_handle
            .update(&mut cx, |view, _, cx| {
                view.apply_relay_snapshot(
                    RelaySnapshot {
                        lifecycle: RelayLifecycle::Current,
                        gap_reason: Some(RelayGapReason::InvalidRelayFrame),
                        metadata_trusted: false,
                        ..Default::default()
                    },
                    cx,
                );
            })
            .expect("update public channel gap");
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("omega-public-channel-lifecycle-banner")
                .is_some(),
            "a gap reason must not be hidden by the metadata warning"
        );
        assert!(
            cx.debug_bounds("omega-public-channel-metadata-banner")
                .is_some()
        );
    }

    #[gpui::test]
    fn community_sarah_controls_are_compact_and_fail_closed_until_verified(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let window_handle = cx.add_window(|window, cx| {
            let http_client: Arc<dyn HttpClient> = FakeHttpClient::with_404_response();
            PublicChannelView::new(
                descriptor("agent-chat", "wss://relay.example"),
                http_client,
                window,
                cx,
            )
        });
        cx.run_until_parked();
        let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
        cx.simulate_resize(size(px(620.), px(720.)));

        assert!(cx.debug_bounds("omega-public-channel-sarah").is_some());
        window_handle
            .update(&mut cx, |view, _, cx| {
                assert!(!view.sarah_room.control_enabled(CommunitySarahControl::Join));
                assert!(!view.sarah_room.control_enabled(CommunitySarahControl::Talk));
                view.sarah_room.configure(
                    CommunityRoomContext {
                        community_ref: "community:testers".into(),
                        channel_ref: "channel:agent-chat".into(),
                    },
                    true,
                );
                view.sarah_room
                    .apply_authority(
                        community_sarah_authority(),
                        CommunityRoomRole::Moderator,
                        CommunitySarahState::Idle,
                        1_000,
                    )
                    .expect("verified test authority");
                cx.notify();
            })
            .expect("configure community Sarah room");
        cx.run_until_parked();

        let disclosure = cx
            .debug_bounds("omega-room-sarah-disclosure-hitbox")
            .expect("disclosure control");
        cx.simulate_click(disclosure.center(), Modifiers::default());
        window_handle
            .update(&mut cx, |view, _, _| {
                assert!(view.sarah_room.disclosure_acknowledged);
                assert!(view.sarah_room.control_enabled(CommunitySarahControl::Talk));
            })
            .expect("read community Sarah controls");
        assert!(
            cx.debug_bounds("omega-public-channel-sarah-controls")
                .is_some(),
            "the complete room-control row must remain discoverable at compact width"
        );
    }

    #[gpui::test]
    fn community_sarah_actions_drive_pointer_keyboard_and_direct_dispatch(cx: &mut TestAppContext) {
        init_test(cx);
        let window_handle = cx.add_window(|window, cx| {
            let http_client: Arc<dyn HttpClient> = FakeHttpClient::with_404_response();
            PublicChannelView::new(
                descriptor("agent-chat", "wss://relay.example"),
                http_client,
                window,
                cx,
            )
        });
        cx.update(|cx| {
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                "keymaps/default-macos.json",
                cx,
            )
            .expect("the shipped macOS keymap must contain the room actions");
            cx.bind_keys(bindings);
        });
        cx.run_until_parked();
        let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
        window_handle
            .update(&mut cx, |view, window, cx| {
                view.focus_handle(cx).focus(window, cx);
            })
            .expect("focus public channel action boundary");

        let join_for_test = |cx: &mut VisualTestContext| {
            window_handle
                .update(cx, |view, _, cx| {
                    view.sarah_room.configure(
                        CommunityRoomContext {
                            community_ref: "community:testers".into(),
                            channel_ref: "channel:agent-chat".into(),
                        },
                        true,
                    );
                    view.sarah_room
                        .apply_authority(
                            community_sarah_authority(),
                            CommunityRoomRole::Moderator,
                            CommunitySarahState::Idle,
                            1_000,
                        )
                        .expect("verified test authority");
                    cx.notify();
                })
                .expect("configure joined room");
            cx.run_until_parked();
        };
        let assert_left = |cx: &mut VisualTestContext| {
            window_handle
                .update(cx, |view, _, _| {
                    assert_eq!(
                        view.sarah_room.lifecycle,
                        CommunityCallLifecycle::ReadyToJoin
                    );
                })
                .expect("read room lifecycle");
        };

        join_for_test(&mut cx);
        let leave = cx
            .debug_bounds("omega-room-voice-leave")
            .expect("leave control");
        cx.simulate_click(leave.center(), Modifiers::default());
        assert_left(&mut cx);

        join_for_test(&mut cx);
        cx.dispatch_action(LeaveRoom);
        assert_left(&mut cx);

        join_for_test(&mut cx);
        cx.simulate_keystrokes("cmd-k l");
        assert_left(&mut cx);
    }

    #[gpui::test]
    fn rendered_timeline_interactions_keep_rows_bounded_and_media_gated(cx: &mut TestAppContext) {
        init_test(cx);
        let window_handle = cx.add_window(|window, cx| {
            let http_client: Arc<dyn HttpClient> = FakeHttpClient::with_404_response();
            PublicChannelView::new(
                descriptor("agent-chat", "wss://relay.example"),
                http_client,
                window,
                cx,
            )
        });
        cx.run_until_parked();
        let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
        cx.simulate_resize(size(px(1200.), px(800.)));

        let messages = stable_verified_events(&fixture_events())
            .into_iter()
            .filter(|event| matches!(event.kind, 9 | 1337))
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 3);
        window_handle
            .update(&mut cx, |view, _, cx| {
                view.apply_relay_snapshot(
                    RelaySnapshot {
                        lifecycle: RelayLifecycle::Current,
                        events: messages[1..].to_vec(),
                        metadata_trusted: false,
                        ..Default::default()
                    },
                    cx,
                );
                assert_eq!(view.list_state.item_count(), 2);
                view.apply_relay_snapshot(
                    RelaySnapshot {
                        lifecycle: RelayLifecycle::Current,
                        events: messages.clone(),
                        metadata_trusted: false,
                        ..Default::default()
                    },
                    cx,
                );
                assert_eq!(view.list_state.item_count(), 3);
            })
            .expect("apply paginated snapshots");
        cx.run_until_parked();

        let content_warning_event = messages
            .iter()
            .find(|event| event.tag_values("content-warning").next().is_some())
            .expect("content-warning fixture")
            .id
            .clone();
        let reveal = cx
            .debug_bounds(static_selector(format!("reveal-{content_warning_event}")))
            .expect("content-warning action");
        cx.simulate_click(reveal.center(), Modifiers::default());
        window_handle
            .update(&mut cx, |view, _, _| {
                assert!(
                    view.revealed_content_warnings
                        .contains(&content_warning_event)
                );
            })
            .expect("read content-warning state");

        let media_event = messages
            .iter()
            .find(|event| event.tag_values("imeta").next().is_some())
            .expect("media fixture")
            .id
            .clone();
        let inspect = cx
            .debug_bounds(static_selector(format!("inspect-{media_event}")))
            .expect("inspect action");
        cx.simulate_click(inspect.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("omega-public-channel-event-facts")
                .is_some()
        );
        assert!(
            cx.debug_bounds("omega-public-channel-media-facts-0")
                .is_some()
        );
        assert!(
            cx.debug_bounds("omega-tester-channel-report").is_some(),
            "event facts must expose the signed report action"
        );

        assert!(
            cx.debug_bounds("omega-public-channel-media-state-gated")
                .is_some()
        );
        let media_key = PublicChannelMediaKey::new("agent-chat", media_event.clone(), 0);
        window_handle
            .update(&mut cx, |view, _, _| {
                view.media_fetch_pending = true;
            })
            .expect("inject pending media result");
        let load_media = cx
            .debug_bounds(static_selector(format!("load-media-{media_event}-0")))
            .expect("gated media action");
        cx.simulate_click(
            point(load_media.origin.x + px(4.), load_media.center().y),
            Modifiers::default(),
        );
        cx.run_until_parked();
        window_handle
            .update(&mut cx, |view, _, _| {
                assert!(matches!(
                    view.media_states.get(&media_key),
                    Some(PublicChannelMediaState::Loading)
                ));
            })
            .expect("read loading media state");
        assert!(
            cx.debug_bounds("omega-public-channel-media-state-loading")
                .is_some()
        );

        window_handle
            .update(&mut cx, |view, _, cx| {
                view.media_tasks.remove(&media_key);
                view.media_fetch_pending = false;
                view.media_fetch_result = Some(PublicChannelMediaState::Unavailable {
                    reason: PublicChannelMediaUnavailableReason::Network,
                });
                view.media_states
                    .insert(media_key.clone(), PublicChannelMediaState::Gated);
                view.list_state
                    .remeasure_items(0..view.projection.rows.len());
                cx.notify();
            })
            .expect("prepare unavailable media result");
        cx.run_until_parked();
        let load_media = cx
            .debug_bounds(static_selector(format!("load-media-{media_event}-0")))
            .expect("unavailable media action");
        cx.simulate_click(
            point(load_media.origin.x + px(4.), load_media.center().y),
            Modifiers::default(),
        );
        cx.run_until_parked();
        window_handle
            .update(&mut cx, |view, _, _| {
                assert!(matches!(
                    view.media_states.get(&media_key),
                    Some(PublicChannelMediaState::Unavailable {
                        reason: PublicChannelMediaUnavailableReason::Network
                    })
                ));
                assert_eq!(view.projection.rows.len(), 3);
            })
            .expect("read media failure state");
        assert!(
            cx.debug_bounds("omega-public-channel-media-state-unavailable")
                .is_some()
        );

        window_handle
            .update(&mut cx, |view, _, cx| {
                view.media_fetch_result = Some(PublicChannelMediaState::Mismatch {
                    expected: "00".repeat(32),
                    actual: "11".repeat(32),
                });
                view.media_states
                    .insert(media_key.clone(), PublicChannelMediaState::Gated);
                view.list_state
                    .remeasure_items(0..view.projection.rows.len());
                cx.notify();
            })
            .expect("prepare mismatched media result");
        cx.run_until_parked();
        let load_media = cx
            .debug_bounds(static_selector(format!("load-media-{media_event}-0")))
            .expect("mismatched media action");
        cx.simulate_click(
            point(load_media.origin.x + px(4.), load_media.center().y),
            Modifiers::default(),
        );
        cx.run_until_parked();
        window_handle
            .update(&mut cx, |view, _, _| {
                assert!(matches!(
                    view.media_states.get(&media_key),
                    Some(PublicChannelMediaState::Mismatch { .. })
                ));
                assert_eq!(view.projection.rows.len(), 3);
            })
            .expect("read media mismatch state");
        assert!(
            cx.debug_bounds("omega-public-channel-media-state-mismatch")
                .is_some()
        );

        window_handle
            .update(&mut cx, |view, _, cx| {
                view.media_fetch_result =
                    Some(crate::omega_public_channel_media::verified_media_state_for_test());
                view.media_states
                    .insert(media_key.clone(), PublicChannelMediaState::Gated);
                view.list_state
                    .remeasure_items(0..view.projection.rows.len());
                cx.notify();
            })
            .expect("prepare verified media result");
        cx.run_until_parked();
        let load_media = cx
            .debug_bounds(static_selector(format!("load-media-{media_event}-0")))
            .expect("verified media action");
        cx.simulate_click(
            point(load_media.origin.x + px(4.), load_media.center().y),
            Modifiers::default(),
        );
        cx.run_until_parked();
        window_handle
            .update(&mut cx, |view, _, _| {
                assert!(matches!(
                    view.media_states.get(&media_key),
                    Some(PublicChannelMediaState::Verified(_))
                ));
                assert_eq!(view.projection.rows.len(), 3);
            })
            .expect("read verified media state");
        assert!(
            cx.debug_bounds("omega-public-channel-media-state-verified")
                .is_some()
        );

        let (intent_sender, intent_receiver) = async_channel::bounded(1);
        window_handle
            .update(&mut cx, |view, _, cx| {
                view.relay_intent_sender = Some(intent_sender);
                view.close_event_facts(cx);
            })
            .expect("prepare pagination interaction");
        cx.run_until_parked();
        let load_older = cx
            .debug_bounds("omega-public-channel-load-older")
            .expect("load older action");
        cx.simulate_click(load_older.center(), Modifiers::default());
        assert_eq!(
            intent_receiver.try_recv().expect("pagination intent"),
            RelayIntent::LoadOlder
        );
    }
}
