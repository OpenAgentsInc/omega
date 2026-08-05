use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;

use omega_front_door::{
    ExecutorClass, Quiescence, SendCommand, SendDisposition, SteerCapability, disposition,
};

use crate::omega_send_queue::{
    QueuedSend, SendQueueJournal, SendQueueProcessingState, SendQueueRefusal,
};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueueEntryId(usize);

pub struct QueueEntry {
    pub id: QueueEntryId,
    pub(crate) durable_item_id: Option<String>,
    pub content: Vec<acp::ContentBlock>,
    pub tracked_buffers: Vec<Entity<Buffer>>,
    /// When true, this message interrupts the agent at the next turn boundary
    /// instead of waiting for generation to fully complete. Only the front
    /// entry's value matters, since messages are delivered in FIFO order.
    pub steer: bool,
    pub(crate) executor_class: ExecutorClass,
    pub(crate) steer_capability: SteerCapability,
    pub(crate) can_dispatch: bool,
    pub editor: Entity<MessageEditor>,
    pub _subscription: Subscription,
}

impl QueueEntry {
    pub(crate) fn disposition(&self) -> SendDisposition {
        disposition(
            if self.steer {
                SendCommand::Steer
            } else {
                SendCommand::Enqueue
            },
            self.executor_class,
            self.steer_capability,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageQueueError {
    NotConfigured,
    UnsavedEntry,
    Journal(SendQueueRefusal),
}

impl MessageQueueError {
    pub(crate) fn user_message(self) -> SharedString {
        match self {
            Self::NotConfigured =>
                "Omega could not prepare durable queued input. Your text remains in the composer."
                    .into(),
            Self::UnsavedEntry =>
                "This queued message has edits that could not be saved. It will not send until those edits are saved or moved back to the composer."
                    .into(),
            Self::Journal(SendQueueRefusal::JournalUnreadable) =>
                "Omega could not restore queued input because its durable queue file is unreadable. The file was left untouched for recovery."
                    .into(),
            Self::Journal(_) =>
                "Omega could not save the queued input. Your message was not removed; check that Omega's data directory is writable."
                    .into(),
        }
    }
}

impl fmt::Display for MessageQueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.user_message())
    }
}

impl std::error::Error for MessageQueueError {}

impl From<SendQueueRefusal> for MessageQueueError {
    fn from(refusal: SendQueueRefusal) -> Self {
        Self::Journal(refusal)
    }
}

#[derive(Clone)]
struct DurableQueueBinding {
    journal: Rc<SendQueueJournal>,
    thread_id: String,
}

/// Holds follow-up messages typed while the agent is generating, along with
/// the state machine that decides when they're auto-sent.
pub struct MessageQueue {
    entries: VecDeque<QueueEntry>,
    processing_state: SendQueueProcessingState,
    can_fast_track: bool,
    next_id: usize,
    durable: Option<DurableQueueBinding>,
    pending_dispatch: Option<QueueEntryId>,
    pending_dispatch_was_fast_track: bool,
}

impl Default for MessageQueue {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            processing_state: SendQueueProcessingState::AutoProcess,
            can_fast_track: false,
            next_id: 0,
            durable: None,
            pending_dispatch: None,
            pending_dispatch_was_fast_track: false,
        }
    }
}

impl MessageQueue {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn first(&self) -> Option<&QueueEntry> {
        self.entries.front()
    }

    pub fn first_id(&self) -> Option<QueueEntryId> {
        self.entries.front().map(|entry| entry.id)
    }

    pub fn last_id(&self) -> Option<QueueEntryId> {
        self.entries.back().map(|entry| entry.id)
    }

    pub fn front_wants_steer(&self) -> bool {
        self.entries.front().is_some_and(|entry| entry.steer)
    }

    pub fn iter(&self) -> impl Iterator<Item = &QueueEntry> {
        self.entries.iter()
    }

    pub fn can_fast_track(&self) -> bool {
        self.can_fast_track && !self.entries.is_empty()
    }

    pub fn entry_by_id(&self, id: QueueEntryId) -> Option<&QueueEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn next_id(&mut self) -> QueueEntryId {
        let id = QueueEntryId(self.next_id);
        self.next_id += 1;
        id
    }

    pub(crate) fn new_durable_item_id() -> String {
        uuid::Uuid::new_v4().hyphenated().to_string()
    }

    /// Bind the renderer queue to the one logical conversation it belongs to.
    ///
    /// A process cannot receive the cancellation `Stopped` event it died
    /// before observing. Recover that state as paused, so the item stays
    /// visible and requires explicit engagement instead of being duplicated.
    pub(crate) fn bind(
        &mut self,
        journal: Rc<SendQueueJournal>,
        thread_id: String,
    ) -> Result<Vec<QueuedSend>, MessageQueueError> {
        journal.ensure_readable()?;
        let items = journal.open_items(&thread_id);
        let persisted_state = journal.processing_state(&thread_id);
        self.processing_state =
            if !items.is_empty() && persisted_state != SendQueueProcessingState::Paused {
                journal.set_processing_state(&thread_id, SendQueueProcessingState::Paused)?;
                SendQueueProcessingState::Paused
            } else {
                persisted_state
            };
        self.durable = Some(DurableQueueBinding { journal, thread_id });
        Ok(items)
    }

    pub(crate) fn restore(&mut self, entry: QueueEntry) {
        self.entries.push_back(entry);
        self.can_fast_track = false;
    }

    pub fn enqueue(&mut self, mut entry: QueueEntry) -> Result<(), MessageQueueError> {
        let durable = self.durable()?;
        if let Some(text) = durable_text(&entry.content) {
            let durable_item_id = entry
                .durable_item_id
                .as_deref()
                .ok_or(MessageQueueError::Journal(SendQueueRefusal::UnknownItem))?;
            durable.journal.admit(
                &durable.thread_id,
                durable_item_id,
                &text,
                if entry.steer {
                    SendCommand::Steer
                } else {
                    SendCommand::Enqueue
                },
                entry.executor_class,
                entry.steer_capability,
            )?;
        } else {
            entry.durable_item_id = None;
        }
        entry.can_dispatch = true;
        self.entries.push_back(entry);
        self.processing_state = SendQueueProcessingState::AutoProcess;
        self.can_fast_track = true;
        Ok(())
    }

    pub fn update(
        &mut self,
        id: QueueEntryId,
        content: Vec<acp::ContentBlock>,
        tracked_buffers: Vec<Entity<Buffer>>,
    ) -> Result<(), MessageQueueError> {
        let durable = self.durable()?;
        let entry = self
            .entry_by_id(id)
            .ok_or(MessageQueueError::Journal(SendQueueRefusal::UnknownItem))?;
        let previous_durable_item_id = entry.durable_item_id.clone();
        let next_text = durable_text(&content);
        let next_durable_item_id = match (&previous_durable_item_id, next_text.as_deref()) {
            (Some(item_id), Some(text)) => durable
                .journal
                .update(
                    &durable.thread_id,
                    item_id,
                    text,
                    if entry.steer {
                        SendCommand::Steer
                    } else {
                        SendCommand::Enqueue
                    },
                    entry.executor_class,
                    entry.steer_capability,
                )
                .map(|_| Some(item_id.clone())),
            (Some(item_id), None) => durable
                .journal
                .cancel(&durable.thread_id, item_id)
                .map(|_| None),
            (None, Some(text)) => {
                let item_id = Self::new_durable_item_id();
                durable
                    .journal
                    .admit(
                        &durable.thread_id,
                        &item_id,
                        text,
                        if entry.steer {
                            SendCommand::Steer
                        } else {
                            SendCommand::Enqueue
                        },
                        entry.executor_class,
                        entry.steer_capability,
                    )
                    .map(|_| Some(item_id))
            }
            (None, None) => Ok(None),
        };
        let next_durable_item_id = match next_durable_item_id {
            Ok(item_id) => item_id,
            Err(error) => {
                if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
                    entry.content = content;
                    entry.tracked_buffers = tracked_buffers;
                    entry.can_dispatch = false;
                }
                return Err(error.into());
            }
        };
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(MessageQueueError::Journal(SendQueueRefusal::UnknownItem))?;
        entry.content = content;
        entry.tracked_buffers = tracked_buffers;
        entry.durable_item_id = next_durable_item_id;
        entry.can_dispatch = true;
        Ok(())
    }

    pub fn remove(&mut self, id: QueueEntryId) -> Result<Option<QueueEntry>, MessageQueueError> {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Ok(None);
        };
        if let Some(durable_item_id) = self.entries[index].durable_item_id.as_deref() {
            let durable = self.durable()?;
            durable
                .journal
                .cancel(&durable.thread_id, durable_item_id)?;
        }
        Ok(self.entries.remove(index))
    }

    pub fn clear(&mut self) -> Result<(), MessageQueueError> {
        if self.entries.is_empty() {
            return Ok(());
        }
        let durable = self.durable()?;
        durable.journal.cancel_all(&durable.thread_id)?;
        self.entries.clear();
        self.can_fast_track = false;
        Ok(())
    }

    pub fn toggle_steer(&mut self, id: QueueEntryId) -> Result<bool, MessageQueueError> {
        let durable = self.durable()?;
        let Some(entry) = self.entry_by_id(id) else {
            return Ok(false);
        };
        let steer = !entry.steer;
        if let Some(durable_item_id) = entry.durable_item_id.as_deref() {
            let text = durable_text(&entry.content)
                .ok_or(MessageQueueError::Journal(SendQueueRefusal::UnknownItem))?;
            durable.journal.update(
                &durable.thread_id,
                durable_item_id,
                &text,
                if steer {
                    SendCommand::Steer
                } else {
                    SendCommand::Enqueue
                },
                entry.executor_class,
                entry.steer_capability,
            )?;
        }
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.steer = steer;
            entry.can_dispatch = true;
        }
        Ok(true)
    }

    pub fn try_fast_track(
        &mut self,
        is_generating: bool,
    ) -> Result<Option<QueueEntryId>, MessageQueueError> {
        if !self.can_fast_track {
            return Ok(None);
        }
        let quiescence = if is_generating {
            Quiescence::Running
        } else {
            Quiescence::Proven
        };
        let candidate = self.select_front(quiescence)?;
        if candidate.is_some() {
            self.can_fast_track = false;
            self.pending_dispatch_was_fast_track = true;
        }
        Ok(candidate)
    }

    pub fn on_generation_stopped(
        &mut self,
        is_first_editor_focused: bool,
    ) -> Result<Option<QueueEntryId>, MessageQueueError> {
        match self.processing_state {
            SendQueueProcessingState::AbsorbingCancel => {
                self.set_processing_state(SendQueueProcessingState::AutoProcess)?;
                Ok(None)
            }
            SendQueueProcessingState::Paused => Ok(None),
            SendQueueProcessingState::AutoProcess => {
                if is_first_editor_focused {
                    Ok(None)
                } else {
                    self.select_front(Quiescence::Proven)
                }
            }
        }
    }

    pub fn send_now(
        &mut self,
        id: QueueEntryId,
        is_generating: bool,
    ) -> Result<Option<QueueEntryId>, MessageQueueError> {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Ok(None);
        };
        let quiescence = if is_generating {
            Quiescence::Running
        } else {
            Quiescence::Proven
        };
        let entry = &self.entries[index];
        if !entry.can_dispatch {
            return Err(MessageQueueError::UnsavedEntry);
        }
        if quiescence == Quiescence::Running && !entry.disposition().reaches_running_turn() {
            return Ok(None);
        }
        if self.pending_dispatch.is_some() {
            return Ok(None);
        }
        self.pending_dispatch = Some(id);
        Ok(Some(id))
    }

    pub fn pause(&mut self) -> Result<(), MessageQueueError> {
        self.set_processing_state(SendQueueProcessingState::Paused)
    }

    pub fn resume(&mut self) -> Result<(), MessageQueueError> {
        self.set_processing_state(SendQueueProcessingState::AutoProcess)
    }

    pub(crate) fn promote_for_dispatch(
        &mut self,
        id: QueueEntryId,
        quiescence: Quiescence,
    ) -> Result<Option<QueueEntry>, MessageQueueError> {
        if self.pending_dispatch != Some(id) {
            return Ok(None);
        }
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            self.finish_dispatch_attempt(id);
            return Ok(None);
        };
        let entry = &self.entries[index];
        if !entry.can_dispatch {
            return Err(MessageQueueError::UnsavedEntry);
        }
        let next_state = if quiescence == Quiescence::Running {
            SendQueueProcessingState::AbsorbingCancel
        } else {
            SendQueueProcessingState::AutoProcess
        };
        let Some(durable_item_id) = entry.durable_item_id.clone() else {
            self.set_processing_state(next_state)?;
            self.pending_dispatch = None;
            self.pending_dispatch_was_fast_track = false;
            return Ok(self.entries.remove(index));
        };
        let durable = self.durable()?;
        match durable.journal.claim_for_dispatch(
            &durable.thread_id,
            &durable_item_id,
            quiescence,
            next_state,
        ) {
            Ok(_) => {
                self.processing_state = next_state;
                self.pending_dispatch = None;
                self.pending_dispatch_was_fast_track = false;
                Ok(self.entries.remove(index))
            }
            Err(SendQueueRefusal::NotQuiescent) => {
                self.finish_dispatch_attempt(id);
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn finish_dispatch_attempt(&mut self, id: QueueEntryId) {
        if self.pending_dispatch == Some(id) {
            self.pending_dispatch = None;
            if self.pending_dispatch_was_fast_track {
                self.can_fast_track = true;
            }
            self.pending_dispatch_was_fast_track = false;
        }
    }

    fn select_front(
        &mut self,
        quiescence: Quiescence,
    ) -> Result<Option<QueueEntryId>, MessageQueueError> {
        if self.pending_dispatch.is_some() {
            return Ok(None);
        }
        let Some(entry) = self.entries.front() else {
            return Ok(None);
        };
        if !entry.can_dispatch {
            return Err(MessageQueueError::UnsavedEntry);
        }
        if quiescence == Quiescence::Running && !entry.disposition().reaches_running_turn() {
            return Ok(None);
        }
        let id = entry.id;
        self.pending_dispatch = Some(id);
        Ok(Some(id))
    }

    fn set_processing_state(
        &mut self,
        state: SendQueueProcessingState,
    ) -> Result<(), MessageQueueError> {
        let durable = self.durable()?;
        let previous = self.processing_state;
        self.processing_state = state;
        if let Err(error) = durable
            .journal
            .set_processing_state(&durable.thread_id, state)
        {
            self.processing_state = previous;
            return Err(error.into());
        }
        Ok(())
    }

    fn durable(&self) -> Result<DurableQueueBinding, MessageQueueError> {
        self.durable.clone().ok_or(MessageQueueError::NotConfigured)
    }
}

fn durable_text(content: &[acp::ContentBlock]) -> Option<String> {
    let mut text = String::new();
    for block in content {
        let acp::ContentBlock::Text(block) = block else {
            return None;
        };
        text.push_str(&block.text);
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_text_leaves_rich_content_for_the_in_memory_queue() {
        let content = vec![acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
            "README",
            "file:///README.md",
        ))];
        assert_eq!(durable_text(&content), None);
    }

    #[test]
    fn restart_mid_cancel_recovers_paused_with_the_item_still_visible() {
        let directory = tempfile::tempdir().expect("temporary queue directory");
        let path = directory.path().join("queue.json");
        {
            let journal = SendQueueJournal::at(path.clone());
            journal
                .admit(
                    "thread-1",
                    "item-1",
                    "already handed to the cancelling send",
                    SendCommand::Steer,
                    ExecutorClass::NativeLoop,
                    SteerCapability::Unknown,
                )
                .expect("queued item persisted");
            journal
                .admit(
                    "thread-1",
                    "item-2",
                    "keep this after cancellation",
                    SendCommand::Enqueue,
                    ExecutorClass::NativeLoop,
                    SteerCapability::Unknown,
                )
                .expect("second queued item persisted");
            journal
                .claim_for_dispatch(
                    "thread-1",
                    "item-1",
                    Quiescence::Running,
                    SendQueueProcessingState::AbsorbingCancel,
                )
                .expect("first item and mid-cancel state persisted atomically");
        }

        let reopened = Rc::new(SendQueueJournal::at(path));
        let mut queue = MessageQueue::default();
        let recovered = queue
            .bind(reopened.clone(), "thread-1".to_owned())
            .expect("queue rehydrated");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].text, "keep this after cancellation");
        assert_eq!(
            reopened.processing_state("thread-1"),
            SendQueueProcessingState::Paused,
            "a dead process cannot absorb a Stopped event, so recovery must require manual engagement"
        );
        assert_eq!(
            reopened.open_items("thread-1").len(),
            1,
            "normalizing cancellation recovery must not promote or discard the item"
        );
        assert_eq!(
            reopened.items("thread-1")[0].state,
            omega_front_door::QueueItemState::Promoted,
            "the item already handed to dispatch must not reopen and duplicate"
        );
    }

    #[test]
    fn restart_with_an_open_queue_requires_explicit_resume() {
        let directory = tempfile::tempdir().expect("temporary queue directory");
        let path = directory.path().join("queue.json");
        {
            let journal = SendQueueJournal::at(path.clone());
            journal
                .admit(
                    "thread-1",
                    "item-1",
                    "do not infer provider quiescence after restart",
                    SendCommand::Enqueue,
                    ExecutorClass::ExternalAcp,
                    SteerCapability::CanSteer,
                )
                .expect("queued item persisted");
        }

        let reopened = Rc::new(SendQueueJournal::at(path));
        let mut queue = MessageQueue::default();
        let recovered = queue
            .bind(reopened.clone(), "thread-1".to_owned())
            .expect("queue rehydrated");

        assert_eq!(recovered.len(), 1);
        assert_eq!(
            reopened.processing_state("thread-1"),
            SendQueueProcessingState::Paused,
            "a fresh local Idle value is not authoritative provider quiescence"
        );
        assert_eq!(
            queue.on_generation_stopped(false),
            Ok(None),
            "restored input must not auto-send before a person resumes it"
        );
    }
}
