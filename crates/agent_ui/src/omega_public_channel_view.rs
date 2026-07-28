//! The read-only selected-channel timeline for Omega.
//!
//! This view owns one relay-qualified channel cache. A channel selection can
//! start its relay session, and leaving the channel stops that session without
//! deleting verified rows. The view has no composer, signer, authentication
//! response, join, or moderation action.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
    sync::Arc,
};

use gpui::{
    AnyElement, Context, EventEmitter, FollowMode, ImageSource, ListAlignment, ListSizingBehavior,
    ListState, ObjectFit, ParentElement as _, Render, SharedString, Styled as _, Task, Window, img,
    list, px,
};
use http_client::HttpClient;
use ui::{
    Banner, Button, ButtonSize, ButtonStyle, Color, CopyButton, IconButton, IconName, IconSize,
    Label, LabelSize, ScrollAxes, Scrollbars, Severity, Tooltip, WithScrollbar, prelude::*,
};

use crate::{
    omega_nostr_activity,
    omega_public_channel_media::{
        PublicChannelAttachment, PublicChannelMediaFact, PublicChannelMediaIntent,
        PublicChannelMediaKey, PublicChannelMediaLifecycle, PublicChannelMediaState,
        PublicChannelMediaUnavailableReason, fetch_public_channel_media,
    },
    omega_public_channel_relay::{
        RelayAdmissionLimits, RelayGapReason, RelayIntent, RelayLifecycle, RelaySessionConfig,
        RelaySnapshot, run_relay_session,
    },
    omega_public_channel_timeline::{
        ContentPart, DeletionKind, EventFacts, MediaFact, SignatureState, TimelineProjection,
        event_facts, project_timeline,
    },
    omega_public_channels::{ChannelCursor, ChannelDescriptor, ChannelLifecycle, ChannelSnapshot},
};

const RETIRED_SESSION_LIMIT: usize = 2;
const FACTS_PANE_WIDTH: gpui::Pixels = px(336.);
const COMPACT_FACTS_THRESHOLD: gpui::Pixels = px(960.);

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
    descriptor: ChannelDescriptor,
    http_client: Arc<dyn HttpClient>,
    relay_snapshot: RelaySnapshot,
    projection: TimelineProjection,
    list_state: ListState,
    revealed_content_warnings: BTreeSet<String>,
    selected_event_id: Option<String>,
    media_states: BTreeMap<PublicChannelMediaKey, PublicChannelMediaState>,
    media_tasks: BTreeMap<PublicChannelMediaKey, Task<()>>,
    generation: u64,
    session_running: bool,
    relay_intent_sender: Option<async_channel::Sender<RelayIntent>>,
    relay_session_task: Option<Task<()>>,
    retired_session_tasks: VecDeque<Task<()>>,
}

impl EventEmitter<PublicChannelViewEvent> for PublicChannelView {}

impl PublicChannelView {
    pub fn new(
        descriptor: ChannelDescriptor,
        http_client: Arc<dyn HttpClient>,
        _cx: &mut Context<Self>,
    ) -> Self {
        let list_state = ListState::new(0, ListAlignment::Top, px(2048.));
        list_state.set_follow_mode(FollowMode::Tail);
        Self {
            descriptor,
            http_client,
            relay_snapshot: RelaySnapshot::default(),
            projection: TimelineProjection::default(),
            list_state,
            revealed_content_warnings: BTreeSet::new(),
            selected_event_id: None,
            media_states: BTreeMap::new(),
            media_tasks: BTreeMap::new(),
            generation: 0,
            session_running: false,
            relay_intent_sender: None,
            relay_session_task: None,
            retired_session_tasks: VecDeque::new(),
        }
    }

    pub fn descriptor(&self) -> &ChannelDescriptor {
        &self.descriptor
    }

    pub fn last_current_at(&self) -> Option<u64> {
        self.relay_snapshot.last_current_at
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
        self.relay_session_task = Some(cx.spawn(async move |this, cx| {
            let driver =
                cx.background_spawn(run_relay_session(config, intent_receiver, snapshot_sender));
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
            .ok();
        }));
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.saturating_add(1);
        self.session_running = false;
        if let Some(sender) = self.relay_intent_sender.take() {
            sender.try_send(RelayIntent::Close).ok();
        }
        self.media_tasks.clear();
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
            sender.try_send(RelayIntent::LoadOlder).ok();
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

    fn apply_relay_snapshot(&mut self, snapshot: RelaySnapshot, cx: &mut Context<Self>) {
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
        let fetch = cx.background_spawn(fetch_public_channel_media(
            http_client,
            attachment,
            max_bytes,
        ));
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
            .ok();
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

    fn render_status_banner(&self) -> Option<Banner> {
        let (severity, text): (Severity, SharedString) = if !self.relay_snapshot.metadata_trusted {
            (
                Severity::Warning,
                "Messages are verified. Group metadata is not authenticated.".into(),
            )
        } else if let Some(reason) = self.relay_snapshot.gap_reason {
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
                        Button::new(
                            SharedString::from(format!("inspect-{}", row.event_id)),
                            "Inspect",
                        )
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.inspect_event(&inspect_event_id, cx);
                        })),
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
            return card
                .child(
                    Label::new("Content warning")
                        .size(LabelSize::Small)
                        .color(Color::Warning),
                )
                .child(
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
        let mut card = v_flex()
            .id(SharedString::from(format!(
                "media-{}-{}",
                key.event_id, key.attachment_index
            )))
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
                card = card.child(
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
                );
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
            ("Media", facts.media.len().to_string()),
        ];
        v_flex()
            .id("omega-public-channel-event-facts")
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
            }))
            .into_any_element()
    }

    fn render_empty_or_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.projection.rows.is_empty() {
            if matches!(self.relay_snapshot.lifecycle, RelayLifecycle::Connecting) {
                return v_flex()
                    .size_full()
                    .p_4()
                    .gap_3()
                    .child(
                        Label::new("Loading signed history…")
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
                    .into_any_element();
            }
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(
                    Label::new("This channel is quiet.")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .into_any_element();
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
                        Button::new("omega-public-channel-load-older", "Load older")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(|this, _, _, _| this.load_older())),
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
}

impl Render for PublicChannelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let facts = self.selected_facts();
        let compact = window.viewport_size().width < COMPACT_FACTS_THRESHOLD;
        let mut view = v_flex().size_full().min_h_0().overflow_hidden();
        if let Some(banner) = self.render_status_banner() {
            view = view.child(div().px_3().pt_2().child(banner));
        }
        if compact && let Some(facts) = facts {
            return view.child(self.render_event_facts(facts, cx));
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
    }
}

impl Drop for PublicChannelView {
    fn drop(&mut self) {
        if let Some(sender) = self.relay_intent_sender.take() {
            sender.try_send(RelayIntent::Close).ok();
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
