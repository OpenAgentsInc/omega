//! Durable admission for messages sent during a running turn (omega#79).
//!
//! `OMEGA-DELTA-0032`.
//!
//! ## What was wrong
//!
//! Omega inherited a real queue. `MessageQueue` distinguishes a steer from an
//! enqueue, and its state machine is careful about the cancel it has to absorb.
//! Two things were missing, and the issue's own falsifier names both.
//!
//! **"Queue state lives only in renderer memory."** It did. `MessageQueue` is a
//! field on `ThreadView`, holds live GPUI editor handles, and dies with
//! the view. Close the panel, reconnect, or restart, and a message the composer
//! had already acknowledged as queued was gone with no trace that it ever
//! existed. This module is the durable half: an item is written down *before*
//! the UI says it is queued, so what the user was told and what survives a
//! restart are the same fact.
//!
//! **"A second send reaches a running provider turn without a guard."**
//! `sync_queue_flag_to_native_thread` set the boundary flag on
//! `agent::Thread` and did nothing for anything else, so an external ACP thread
//! and an engine lane both fell through to `dispatch_queued_entry`'s
//! cancel-then-send. The guard is
//! [`omega_front_door::disposition`](omega_front_door::disposition), a total
//! law over all three executor classes; this module records what it decided
//! beside the item, so the reason a message was not steered outlives the
//! session that decided it.
//!
//! ## Shape
//!
//! Same shape as [`crate::omega_router::RouteJournal`], for the same reasons:
//! a `BTreeMap` so the file does not change when nothing decided differently,
//! an atomic temporary-file rewrite so a crash leaves the previous journal
//! rather than a truncated one, and typed round-tripping so a hand-edited file
//! is refused rather than believed.
//!
//! One rule this journal has that the route journal does not: **a terminal item
//! is never reopened.** `promoted`, `cancelled` and `failed` are final. A
//! restart that could move an item back to `queued` would promote it a second
//! time, which is exactly the duplication the acceptance criterion names.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, Global};
use omega_front_door::{
    ExecutorClass, QueueItemState, Quiescence, SendCommand, SendDisposition, SteerCapability,
    disposition, may_promote,
};
use serde_json::Value;

/// Schema of the durable queue document.
pub const SEND_QUEUE_JOURNAL_SCHEMA: &str = "openagents.omega.agent_send_queue.v1";

/// File the document lives in, under the Omega data directory.
pub const SEND_QUEUE_JOURNAL_FILE: &str = "agent-send-queue.json";

/// One admitted message, as it survives a restart.
///
/// The editor handle and the tracked buffers are deliberately absent: they are
/// live GPUI values and cannot be a durable fact. What has to survive is the
/// text the person wrote, where it was going, what they asked for, what Omega
/// decided, and where the item is in its life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedSend {
    /// Stable across restarts and unique within a thread. Ordering identity as
    /// well as name: promotion is by `(sequence, item_id)`, never by map order.
    pub item_id: String,
    pub thread_id: String,
    pub sequence: u64,
    /// The message body. Plain text only — a durable record of a message the
    /// user typed, not a serialised content-block tree.
    pub text: String,
    pub command: SendCommand,
    pub class: ExecutorClass,
    pub capability: SteerCapability,
    pub state: QueueItemState,
}

impl QueuedSend {
    /// What Omega will do with this item, derived from the parts.
    ///
    /// Derived on every call and never stored, the same rule
    /// `ExecutorDisclosure` holds to: a stored disposition could disagree with
    /// the law that produced it, and then the record would be the lie.
    #[must_use]
    pub const fn disposition(&self) -> SendDisposition {
        disposition(self.command, self.class, self.capability)
    }

    /// Whether this item may be promoted now.
    #[must_use]
    pub const fn may_promote(&self, quiescence: Quiescence) -> bool {
        may_promote(self.state, quiescence)
    }
}

/// Why an admission or a transition did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendQueueRefusal {
    /// The item is already terminal. Promoting it again would duplicate a turn.
    ItemIsTerminal,
    /// No item with this id.
    UnknownItem,
    /// The prior turn is not proven finished.
    NotQuiescent,
    /// The record could not be written, so the user was not told it was queued.
    NotPersisted,
    /// The existing journal could not be decoded. It is left untouched so a
    /// later write cannot erase input that may still be recoverable.
    JournalUnreadable,
}

impl SendQueueRefusal {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ItemIsTerminal => "item_is_terminal",
            Self::UnknownItem => "unknown_item",
            Self::NotQuiescent => "not_quiescent",
            Self::NotPersisted => "not_persisted",
            Self::JournalUnreadable => "journal_unreadable",
        }
    }
}

/// Whether a thread may automatically spend its next queued item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendQueueProcessingState {
    AutoProcess,
    Paused,
    AbsorbingCancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueueDispatchIntent {
    Automatic,
    UserRequested,
}

impl SendQueueProcessingState {
    const fn token(self) -> &'static str {
        match self {
            Self::AutoProcess => "auto_process",
            Self::Paused => "paused",
            Self::AbsorbingCancel => "absorbing_cancel",
        }
    }

    fn parse_token(token: &str) -> Option<Self> {
        match token {
            "auto_process" => Some(Self::AutoProcess),
            "paused" => Some(Self::Paused),
            "absorbing_cancel" => Some(Self::AbsorbingCancel),
            _ => None,
        }
    }
}

struct GlobalSendQueueJournal {
    journal: Rc<SendQueueJournal>,
    #[cfg(any(test, feature = "test-support"))]
    _temporary_directory: Option<tempfile::TempDir>,
}

impl Global for GlobalSendQueueJournal {}

/// The durable queue.
///
/// One journal for the whole app, partitioned by thread id. Promotion is the
/// job of a single thread-owned scheduler, which asks
/// [`head_for`](Self::head_for) and then [`promote`](Self::promote).
pub struct SendQueueJournal {
    path: PathBuf,
    /// `BTreeMap` so the serialised file is a function of the items, not of
    /// hash order — a journal that changes when nothing decided differently is
    /// a journal nobody can diff.
    items: RefCell<BTreeMap<String, QueuedSend>>,
    processing_states: RefCell<BTreeMap<String, SendQueueProcessingState>>,
    next_sequence: RefCell<u64>,
    load_error: Option<String>,
}

impl SendQueueJournal {
    /// The single journal shared by every open conversation in this app.
    pub fn global(cx: &mut App) -> Rc<Self> {
        if let Some(global) = cx.try_global::<GlobalSendQueueJournal>() {
            return global.journal.clone();
        }

        #[cfg(any(test, feature = "test-support"))]
        let (journal, temporary_directory) = {
            let directory = tempfile::tempdir().expect("temporary send queue directory");
            let journal = Rc::new(Self::at(directory.path().join(SEND_QUEUE_JOURNAL_FILE)));
            (journal, Some(directory))
        };
        #[cfg(not(any(test, feature = "test-support")))]
        let journal = Rc::new(Self::at_data_dir());

        cx.set_global(GlobalSendQueueJournal {
            journal: journal.clone(),
            #[cfg(any(test, feature = "test-support"))]
            _temporary_directory: temporary_directory,
        });
        journal
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_global_for_tests(journal: Rc<Self>, cx: &mut App) {
        cx.set_global(GlobalSendQueueJournal {
            journal,
            _temporary_directory: None,
        });
    }

    /// The journal at the Omega data directory's usual place.
    #[must_use]
    pub fn at_data_dir() -> Self {
        Self::at(
            paths::data_dir()
                .join("openagents")
                .join(SEND_QUEUE_JOURNAL_FILE),
        )
    }

    /// The journal at an explicit path. Loads what is already there.
    ///
    /// An unreadable journal is retained and every mutation is refused. A
    /// later write must not overwrite input that may still be recoverable.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        let (items, processing_states, load_error) = match load(&path) {
            Ok((items, processing_states)) => (items, processing_states, None),
            Err(error) => {
                log::error!(
                    "OMEGA-DELTA-0032: send queue at {} could not be read ({error:#}); \
                     refusing to overwrite it",
                    path.display()
                );
                (BTreeMap::new(), BTreeMap::new(), Some(error.to_string()))
            }
        };
        let next = items.values().map(|item| item.sequence).max().unwrap_or(0) + 1;
        Self {
            path,
            items: RefCell::new(items),
            processing_states: RefCell::new(processing_states),
            next_sequence: RefCell::new(next),
            load_error,
        }
    }

    pub fn ensure_readable(&self) -> Result<(), SendQueueRefusal> {
        if self.load_error.is_some() {
            Err(SendQueueRefusal::JournalUnreadable)
        } else {
            Ok(())
        }
    }

    /// Admit a message to the queue, durably, before acknowledging it.
    ///
    /// The `Ok` here is the acknowledgement the UI is allowed to show. A caller
    /// that renders "queued" on the `Err` path has reintroduced the defect.
    ///
    /// # Errors
    ///
    /// [`SendQueueRefusal::NotPersisted`] when the record could not be written.
    pub fn admit(
        &self,
        thread_id: &str,
        item_id: &str,
        text: &str,
        command: SendCommand,
        class: ExecutorClass,
        capability: SteerCapability,
    ) -> Result<QueuedSend, SendQueueRefusal> {
        self.ensure_readable()?;
        let sequence = *self.next_sequence.borrow();
        let Some(next_sequence) = sequence.checked_add(1) else {
            log::error!(
                "OMEGA-DELTA-0032: {} cannot admit another item because its sequence space is exhausted",
                self.path.display()
            );
            return Err(SendQueueRefusal::NotPersisted);
        };
        let item = QueuedSend {
            item_id: item_id.to_owned(),
            thread_id: thread_id.to_owned(),
            sequence,
            text: text.to_owned(),
            command,
            class,
            capability,
            state: QueueItemState::Queued,
        };
        let key = Self::key(thread_id, item_id);
        let previous = self.items.borrow_mut().insert(key.clone(), item.clone());
        let previous_processing_state = self
            .processing_states
            .borrow_mut()
            .insert(thread_id.to_owned(), SendQueueProcessingState::AutoProcess);
        match self.persist() {
            Ok(()) => {
                *self.next_sequence.borrow_mut() = next_sequence;
                Ok(item)
            }
            Err(error) => {
                log::error!(
                    "OMEGA-DELTA-0032: {item_id} could not be admitted to {}: {error:#}",
                    self.path.display()
                );
                let mut items = self.items.borrow_mut();
                if let Some(previous) = previous {
                    items.insert(key, previous);
                } else {
                    items.remove(&key);
                }
                let mut states = self.processing_states.borrow_mut();
                if let Some(previous) = previous_processing_state {
                    states.insert(thread_id.to_owned(), previous);
                } else {
                    states.remove(thread_id);
                }
                Err(SendQueueRefusal::NotPersisted)
            }
        }
    }

    /// Replace the durable body and send intention of an open item.
    pub fn update(
        &self,
        thread_id: &str,
        item_id: &str,
        text: &str,
        command: SendCommand,
        class: ExecutorClass,
        capability: SteerCapability,
    ) -> Result<QueuedSend, SendQueueRefusal> {
        self.ensure_readable()?;
        let key = Self::key(thread_id, item_id);
        self.require_open(&key)?;
        let previous = {
            let mut items = self.items.borrow_mut();
            let item = items.get_mut(&key).ok_or(SendQueueRefusal::UnknownItem)?;
            let previous = item.clone();
            item.text = text.to_owned();
            item.command = command;
            item.class = class;
            item.capability = capability;
            previous
        };
        match self.persist() {
            Ok(()) => self
                .items
                .borrow()
                .get(&key)
                .cloned()
                .ok_or(SendQueueRefusal::UnknownItem),
            Err(error) => {
                log::error!("OMEGA-DELTA-0032: {key} could not save a queue edit: {error:#}");
                self.items.borrow_mut().insert(key, previous);
                Err(SendQueueRefusal::NotPersisted)
            }
        }
    }

    /// Every open item for a thread, in promotion order.
    #[must_use]
    pub fn open_items(&self, thread_id: &str) -> Vec<QueuedSend> {
        let mut items: Vec<QueuedSend> = self
            .items
            .borrow()
            .values()
            .filter(|item| item.thread_id == thread_id && item.state.is_open())
            .cloned()
            .collect();
        items.sort_by(|left, right| {
            left.sequence
                .cmp(&right.sequence)
                .then_with(|| left.item_id.cmp(&right.item_id))
        });
        items
    }

    /// Every item for a thread, including terminal ones, in the same order.
    #[must_use]
    pub fn items(&self, thread_id: &str) -> Vec<QueuedSend> {
        let mut items: Vec<QueuedSend> = self
            .items
            .borrow()
            .values()
            .filter(|item| item.thread_id == thread_id)
            .cloned()
            .collect();
        items.sort_by(|left, right| {
            left.sequence
                .cmp(&right.sequence)
                .then_with(|| left.item_id.cmp(&right.item_id))
        });
        items
    }

    /// The item the scheduler would promote next, if any.
    #[must_use]
    pub fn head_for(&self, thread_id: &str) -> Option<QueuedSend> {
        self.open_items(thread_id).into_iter().next()
    }

    #[must_use]
    pub fn processing_state(&self, thread_id: &str) -> SendQueueProcessingState {
        self.processing_states
            .borrow()
            .get(thread_id)
            .copied()
            .unwrap_or(SendQueueProcessingState::AutoProcess)
    }

    pub fn set_processing_state(
        &self,
        thread_id: &str,
        state: SendQueueProcessingState,
    ) -> Result<(), SendQueueRefusal> {
        self.ensure_readable()?;
        let previous = self
            .processing_states
            .borrow_mut()
            .insert(thread_id.to_owned(), state);
        if let Err(error) = self.persist() {
            log::error!(
                "OMEGA-DELTA-0032: {thread_id} could not save queue processing state: {error:#}"
            );
            let mut states = self.processing_states.borrow_mut();
            if let Some(previous) = previous {
                states.insert(thread_id.to_owned(), previous);
            } else {
                states.remove(thread_id);
            }
            return Err(SendQueueRefusal::NotPersisted);
        }
        Ok(())
    }

    /// Claim an item immediately before dispatching it.
    ///
    /// A running turn admits a user-requested dispatch or a disposition that
    /// reaches that turn. Automatic queued input still requires quiescence.
    pub(crate) fn claim_for_dispatch(
        &self,
        thread_id: &str,
        item_id: &str,
        quiescence: Quiescence,
        processing_state: SendQueueProcessingState,
        intent: QueueDispatchIntent,
    ) -> Result<QueuedSend, SendQueueRefusal> {
        self.ensure_readable()?;
        let key = Self::key(thread_id, item_id);
        let previous_item = self
            .items
            .borrow()
            .get(&key)
            .cloned()
            .ok_or(SendQueueRefusal::UnknownItem)?;
        if !previous_item.state.is_open() {
            return Err(SendQueueRefusal::ItemIsTerminal);
        }
        let may_dispatch = previous_item.may_promote(quiescence)
            || (quiescence == Quiescence::Running
                && (intent == QueueDispatchIntent::UserRequested
                    || previous_item.disposition().reaches_running_turn()));
        if !may_dispatch {
            return Err(SendQueueRefusal::NotQuiescent);
        }

        let updated = {
            let mut items = self.items.borrow_mut();
            let item = items.get_mut(&key).ok_or(SendQueueRefusal::UnknownItem)?;
            item.state = QueueItemState::Promoted;
            item.clone()
        };
        let previous_processing_state = self
            .processing_states
            .borrow_mut()
            .insert(thread_id.to_owned(), processing_state);
        if let Err(error) = self.persist() {
            log::error!("OMEGA-DELTA-0032: {key} could not be claimed: {error:#}");
            self.items.borrow_mut().insert(key, previous_item);
            let mut states = self.processing_states.borrow_mut();
            if let Some(previous) = previous_processing_state {
                states.insert(thread_id.to_owned(), previous);
            } else {
                states.remove(thread_id);
            }
            return Err(SendQueueRefusal::NotPersisted);
        }
        Ok(updated)
    }

    /// Cancel every open item for one thread in a single durable rewrite.
    pub fn cancel_all(&self, thread_id: &str) -> Result<(), SendQueueRefusal> {
        self.ensure_readable()?;
        let previous = self.items.borrow().clone();
        for item in self
            .items
            .borrow_mut()
            .values_mut()
            .filter(|item| item.thread_id == thread_id && item.state.is_open())
        {
            item.state = QueueItemState::Cancelled;
        }
        if let Err(error) = self.persist() {
            log::error!("OMEGA-DELTA-0032: {thread_id} could not clear its queue: {error:#}");
            *self.items.borrow_mut() = previous;
            return Err(SendQueueRefusal::NotPersisted);
        }
        Ok(())
    }

    /// Promote the queue head, but only after proven quiescence.
    ///
    /// # Errors
    ///
    /// [`SendQueueRefusal::NotQuiescent`] when the prior turn is running or its
    /// stop was never observed — the reconnect case, where promoting is how a
    /// message gets sent twice.
    pub fn promote(
        &self,
        thread_id: &str,
        item_id: &str,
        quiescence: Quiescence,
    ) -> Result<QueuedSend, SendQueueRefusal> {
        let key = Self::key(thread_id, item_id);
        let current = self
            .items
            .borrow()
            .get(&key)
            .cloned()
            .ok_or(SendQueueRefusal::UnknownItem)?;
        if !current.state.is_open() {
            return Err(SendQueueRefusal::ItemIsTerminal);
        }
        if !may_promote(current.state, quiescence) {
            return Err(SendQueueRefusal::NotQuiescent);
        }
        self.transition(&key, QueueItemState::Promoted)
    }

    /// Withdraw an item before it is promoted.
    ///
    /// # Errors
    ///
    /// [`SendQueueRefusal::ItemIsTerminal`] for an item that already ended.
    pub fn cancel(&self, thread_id: &str, item_id: &str) -> Result<QueuedSend, SendQueueRefusal> {
        let key = Self::key(thread_id, item_id);
        self.require_open(&key)?;
        self.transition(&key, QueueItemState::Cancelled)
    }

    /// Record that promotion was attempted and did not start a turn.
    ///
    /// # Errors
    ///
    /// [`SendQueueRefusal::ItemIsTerminal`] for an item that already ended.
    pub fn fail(&self, thread_id: &str, item_id: &str) -> Result<QueuedSend, SendQueueRefusal> {
        let key = Self::key(thread_id, item_id);
        self.require_open(&key)?;
        self.transition(&key, QueueItemState::Failed)
    }

    fn require_open(&self, key: &str) -> Result<(), SendQueueRefusal> {
        let items = self.items.borrow();
        let item = items.get(key).ok_or(SendQueueRefusal::UnknownItem)?;
        if item.state.is_open() {
            Ok(())
        } else {
            Err(SendQueueRefusal::ItemIsTerminal)
        }
    }

    fn transition(&self, key: &str, state: QueueItemState) -> Result<QueuedSend, SendQueueRefusal> {
        self.ensure_readable()?;
        let (previous, updated) = {
            let mut items = self.items.borrow_mut();
            let item = items.get_mut(key).ok_or(SendQueueRefusal::UnknownItem)?;
            let previous = item.clone();
            item.state = state;
            (previous, item.clone())
        };
        match self.persist() {
            Ok(()) => Ok(updated),
            Err(error) => {
                log::error!(
                    "OMEGA-DELTA-0032: {key} could not record {}: {error:#}",
                    state.token()
                );
                self.items.borrow_mut().insert(key.to_owned(), previous);
                Err(SendQueueRefusal::NotPersisted)
            }
        }
    }

    fn key(thread_id: &str, item_id: &str) -> String {
        format!("{thread_id}\u{1f}{item_id}")
    }

    fn persist(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.load_error.is_none(),
            "the existing send queue journal is unreadable"
        );
        let items = self.items.borrow();
        let processing_states = self.processing_states.borrow();
        let document = serde_json::json!({
            "schema": SEND_QUEUE_JOURNAL_SCHEMA,
            "items": items
                .values()
                .map(|item| serde_json::json!({
                    "itemId": item.item_id,
                    "threadId": item.thread_id,
                    "sequence": item.sequence,
                    "text": item.text,
                    "command": item.command.token(),
                    "class": item.class.token(),
                    "capability": item.capability.token(),
                    "state": item.state.token(),
                }))
                .collect::<Vec<_>>(),
            "threadStates": processing_states
                .iter()
                .map(|(thread_id, state)| serde_json::json!({
                    "threadId": thread_id,
                    "state": state.token(),
                }))
                .collect::<Vec<_>>(),
        });
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&document)?)?;
        std::fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}

fn parse_class(token: &str) -> Option<ExecutorClass> {
    ExecutorClass::all()
        .iter()
        .copied()
        .find(|class| class.token() == token)
}

fn parse_capability(token: &str) -> Option<SteerCapability> {
    SteerCapability::all()
        .iter()
        .copied()
        .find(|capability| capability.token() == token)
}

fn load(
    path: &Path,
) -> anyhow::Result<(
    BTreeMap<String, QueuedSend>,
    BTreeMap<String, SendQueueProcessingState>,
)> {
    if !path.exists() {
        return Ok((BTreeMap::new(), BTreeMap::new()));
    }
    let document: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let schema = document.get("schema").and_then(Value::as_str);
    anyhow::ensure!(
        schema == Some(SEND_QUEUE_JOURNAL_SCHEMA),
        "unsupported send queue schema {schema:?}"
    );
    let mut items = BTreeMap::new();
    let entries = document
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("send queue items must be an array"))?;
    for entry in entries {
        let (
            Some(item_id),
            Some(thread_id),
            Some(sequence),
            Some(text),
            Some(command),
            Some(class),
            Some(capability),
            Some(state),
        ) = (
            entry.get("itemId").and_then(Value::as_str),
            entry.get("threadId").and_then(Value::as_str),
            entry.get("sequence").and_then(Value::as_u64),
            entry.get("text").and_then(Value::as_str),
            entry
                .get("command")
                .and_then(Value::as_str)
                .and_then(SendCommand::parse_token),
            entry
                .get("class")
                .and_then(Value::as_str)
                .and_then(parse_class),
            entry
                .get("capability")
                .and_then(Value::as_str)
                .and_then(parse_capability),
            entry
                .get("state")
                .and_then(Value::as_str)
                .and_then(QueueItemState::parse_token),
        )
        else {
            anyhow::bail!("send queue entry is not a complete queued send");
        };
        let key = SendQueueJournal::key(thread_id, item_id);
        anyhow::ensure!(
            !items.contains_key(&key),
            "duplicate send queue item identity for {thread_id}/{item_id}"
        );
        items.insert(
            key,
            QueuedSend {
                item_id: item_id.to_owned(),
                thread_id: thread_id.to_owned(),
                sequence,
                text: text.to_owned(),
                command,
                class,
                capability,
                state,
            },
        );
    }

    let mut processing_states = BTreeMap::new();
    let state_entries = document
        .get("threadStates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("send queue threadStates must be an array"))?;
    for entry in state_entries {
        let (Some(thread_id), Some(state)) = (
            entry.get("threadId").and_then(Value::as_str),
            entry
                .get("state")
                .and_then(Value::as_str)
                .and_then(SendQueueProcessingState::parse_token),
        ) else {
            anyhow::bail!("send queue thread state is incomplete");
        };
        anyhow::ensure!(
            !processing_states.contains_key(thread_id),
            "duplicate send queue thread state for {thread_id}"
        );
        processing_states.insert(thread_id.to_owned(), state);
    }
    anyhow::ensure!(
        items.values().all(|item| item.sequence < u64::MAX),
        "send queue sequence space is exhausted"
    );
    Ok((items, processing_states))
}

/// The line the composer shows for one admitted item.
///
/// Derived from the item, so the queue cannot render a promise its disposition
/// does not support. Nothing stores this.
#[must_use]
pub fn queue_item_phrase(item: &QueuedSend) -> String {
    match item.state {
        QueueItemState::Queued => item.disposition().phrase(),
        QueueItemState::Promoted => "Sent.".to_owned(),
        QueueItemState::Cancelled => "Removed from the queue.".to_owned(),
        QueueItemState::Failed => "Could not be sent. Still in the queue history.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omega_front_door::{SendFallback, SteerRefusal};

    fn journal() -> (SendQueueJournal, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SEND_QUEUE_JOURNAL_FILE);
        (SendQueueJournal::at(path), dir)
    }

    fn admit(
        journal: &SendQueueJournal,
        item_id: &str,
        command: SendCommand,
        class: ExecutorClass,
        capability: SteerCapability,
    ) -> QueuedSend {
        journal
            .admit(
                "thread-1",
                item_id,
                "look at the other file too",
                command,
                class,
                capability,
            )
            .expect("admitted")
    }

    /// The falsifier's second half: "queue state lives only in renderer
    /// memory". A journal reopened from the same path is the restart.
    #[test]
    fn an_admitted_message_survives_a_restart() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SEND_QUEUE_JOURNAL_FILE);
        {
            let journal = SendQueueJournal::at(path.clone());
            journal
                .admit(
                    "thread-1",
                    "item-1",
                    "and check the tests",
                    SendCommand::Enqueue,
                    ExecutorClass::NativeLoop,
                    SteerCapability::Unknown,
                )
                .expect("admitted");
        }
        let reopened = SendQueueJournal::at(path);
        let items = reopened.open_items("thread-1");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "and check the tests");
        assert_eq!(items[0].state, QueueItemState::Queued);
    }

    /// A promoted item that a restart reopened would be sent twice. The
    /// acceptance criterion names this exact case.
    #[test]
    fn a_promoted_item_is_not_reopened_by_a_restart_and_cannot_promote_twice() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SEND_QUEUE_JOURNAL_FILE);
        {
            let journal = SendQueueJournal::at(path.clone());
            admit(
                &journal,
                "item-1",
                SendCommand::Enqueue,
                ExecutorClass::NativeLoop,
                SteerCapability::Unknown,
            );
            journal
                .promote("thread-1", "item-1", Quiescence::Proven)
                .expect("promoted once");
        }
        let reopened = SendQueueJournal::at(path);
        assert!(reopened.open_items("thread-1").is_empty());
        assert_eq!(reopened.items("thread-1").len(), 1);
        assert_eq!(
            reopened.promote("thread-1", "item-1", Quiescence::Proven),
            Err(SendQueueRefusal::ItemIsTerminal)
        );
    }

    /// After a reconnect Omega never saw the prior turn stop. Promoting there
    /// is how a queued message races the turn it was meant to follow.
    #[test]
    fn a_reconnect_does_not_promote_on_an_unobserved_stop() {
        let (journal, _dir) = journal();
        admit(
            &journal,
            "item-1",
            SendCommand::Enqueue,
            ExecutorClass::ExternalAcp,
            SteerCapability::Unknown,
        );
        assert_eq!(
            journal.promote("thread-1", "item-1", Quiescence::Unknown),
            Err(SendQueueRefusal::NotQuiescent)
        );
        assert_eq!(
            journal.promote("thread-1", "item-1", Quiescence::Running),
            Err(SendQueueRefusal::NotQuiescent)
        );
        assert!(
            journal
                .promote("thread-1", "item-1", Quiescence::Proven)
                .is_ok()
        );
    }

    #[test]
    fn only_user_requested_dispatch_claims_an_enqueue_during_a_running_turn() {
        let (journal, _directory) = journal();
        admit(
            &journal,
            "automatic-item",
            SendCommand::Enqueue,
            ExecutorClass::NativeLoop,
            SteerCapability::Unknown,
        );
        assert_eq!(
            journal.claim_for_dispatch(
                "thread-1",
                "automatic-item",
                Quiescence::Running,
                SendQueueProcessingState::AbsorbingCancel,
                QueueDispatchIntent::Automatic,
            ),
            Err(SendQueueRefusal::NotQuiescent)
        );

        admit(
            &journal,
            "user-requested-item",
            SendCommand::Enqueue,
            ExecutorClass::NativeLoop,
            SteerCapability::Unknown,
        );
        assert!(
            journal
                .claim_for_dispatch(
                    "thread-1",
                    "user-requested-item",
                    Quiescence::Running,
                    SendQueueProcessingState::AbsorbingCancel,
                    QueueDispatchIntent::UserRequested,
                )
                .is_ok()
        );
    }

    /// Ordering is the item's own, not the map's. Two items admitted in order
    /// promote in that order however their ids sort.
    #[test]
    fn promotion_order_is_admission_order_not_identifier_order() {
        let (journal, _dir) = journal();
        admit(
            &journal,
            "zzz-first",
            SendCommand::Enqueue,
            ExecutorClass::NativeLoop,
            SteerCapability::Unknown,
        );
        admit(
            &journal,
            "aaa-second",
            SendCommand::Enqueue,
            ExecutorClass::NativeLoop,
            SteerCapability::Unknown,
        );
        assert_eq!(
            journal.head_for("thread-1").expect("head").item_id,
            "zzz-first"
        );
    }

    /// Two threads, one file, no crossing. A queue that leaked between threads
    /// would deliver somebody's message to the wrong agent.
    #[test]
    fn items_do_not_cross_between_threads() {
        let (journal, _dir) = journal();
        admit(
            &journal,
            "item-1",
            SendCommand::Enqueue,
            ExecutorClass::NativeLoop,
            SteerCapability::Unknown,
        );
        journal
            .admit(
                "thread-2",
                "item-1",
                "different thread, same item id",
                SendCommand::Enqueue,
                ExecutorClass::ExternalAcp,
                SteerCapability::CanSteer,
            )
            .expect("admitted");
        assert_eq!(journal.open_items("thread-1").len(), 1);
        assert_eq!(journal.open_items("thread-2").len(), 1);
        assert_eq!(
            journal.open_items("thread-2")[0].text,
            "different thread, same item id"
        );
    }

    /// The disposition is derived from the stored parts on every read, so a
    /// restored item cannot claim an outcome the law does not give it.
    #[test]
    fn a_restored_item_re_derives_its_disposition_rather_than_replaying_one() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SEND_QUEUE_JOURNAL_FILE);
        {
            let journal = SendQueueJournal::at(path.clone());
            admit(
                &journal,
                "item-1",
                SendCommand::Steer,
                ExecutorClass::EngineLane,
                SteerCapability::CanSteer,
            );
        }
        let reopened = SendQueueJournal::at(path.clone());
        let item = reopened.head_for("thread-1").expect("head");
        assert_eq!(
            item.disposition(),
            SendDisposition::Refused {
                refusal: SteerRefusal::EngineLaneIsRunAuthority,
                fallback: SendFallback::HeldUntilQuiescent,
            }
        );
        assert!(!item.disposition().reaches_running_turn());
        // And the file itself holds no rendered disposition to disagree with.
        let raw = std::fs::read_to_string(&path).expect("readable");
        assert!(!raw.contains("held_until_quiescent"));
        assert!(!raw.contains("Queued:"));
    }

    /// A journal it cannot read starts empty rather than pretending. The
    /// schema check is what stops it adopting somebody else's file.
    #[test]
    fn a_foreign_document_is_refused_rather_than_adopted() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SEND_QUEUE_JOURNAL_FILE);
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema": "openagents.omega.agent_route_journal.v1",
                "items": [],
            }))
            .expect("json"),
        )
        .expect("written");
        assert!(load(&path).is_err());
        let original = std::fs::read(&path).expect("foreign document remains readable");
        let journal = SendQueueJournal::at(path.clone());
        assert!(journal.open_items("thread-1").is_empty());
        assert_eq!(
            journal.admit(
                "thread-1",
                "item-1",
                "do not erase the old file",
                SendCommand::Enqueue,
                ExecutorClass::NativeLoop,
                SteerCapability::Unknown,
            ),
            Err(SendQueueRefusal::JournalUnreadable)
        );
        assert_eq!(
            std::fs::read(path).expect("foreign document is not overwritten"),
            original
        );
    }

    #[test]
    fn malformed_required_collections_are_refused_without_overwrite() {
        for (name, document) in [
            (
                "missing-items",
                serde_json::json!({
                    "schema": SEND_QUEUE_JOURNAL_SCHEMA,
                    "threadStates": [],
                }),
            ),
            (
                "non-array-items",
                serde_json::json!({
                    "schema": SEND_QUEUE_JOURNAL_SCHEMA,
                    "items": {},
                    "threadStates": [],
                }),
            ),
            (
                "missing-thread-states",
                serde_json::json!({
                    "schema": SEND_QUEUE_JOURNAL_SCHEMA,
                    "items": [],
                }),
            ),
            (
                "non-array-thread-states",
                serde_json::json!({
                    "schema": SEND_QUEUE_JOURNAL_SCHEMA,
                    "items": [],
                    "threadStates": {},
                }),
            ),
            (
                "duplicate-item-identity",
                serde_json::json!({
                    "schema": SEND_QUEUE_JOURNAL_SCHEMA,
                    "items": [
                        {
                            "itemId": "item-1",
                            "threadId": "thread-1",
                            "sequence": 1,
                            "text": "first",
                            "command": SendCommand::Enqueue.token(),
                            "class": ExecutorClass::NativeLoop.token(),
                            "capability": SteerCapability::Unknown.token(),
                            "state": QueueItemState::Queued.token(),
                        },
                        {
                            "itemId": "item-1",
                            "threadId": "thread-1",
                            "sequence": 2,
                            "text": "second",
                            "command": SendCommand::Enqueue.token(),
                            "class": ExecutorClass::NativeLoop.token(),
                            "capability": SteerCapability::Unknown.token(),
                            "state": QueueItemState::Queued.token(),
                        },
                    ],
                    "threadStates": [],
                }),
            ),
            (
                "duplicate-thread-state",
                serde_json::json!({
                    "schema": SEND_QUEUE_JOURNAL_SCHEMA,
                    "items": [],
                    "threadStates": [
                        { "threadId": "thread-1", "state": "paused" },
                        { "threadId": "thread-1", "state": "auto_process" },
                    ],
                }),
            ),
            (
                "exhausted-sequence",
                serde_json::json!({
                    "schema": SEND_QUEUE_JOURNAL_SCHEMA,
                    "items": [{
                        "itemId": "item-1",
                        "threadId": "thread-1",
                        "sequence": u64::MAX,
                        "text": "cannot allocate a successor",
                        "command": SendCommand::Enqueue.token(),
                        "class": ExecutorClass::NativeLoop.token(),
                        "capability": SteerCapability::Unknown.token(),
                        "state": QueueItemState::Queued.token(),
                    }],
                    "threadStates": [],
                }),
            ),
        ] {
            let directory = tempfile::tempdir().expect("temporary queue directory");
            let path = directory.path().join(format!("{name}.json"));
            let original = serde_json::to_vec_pretty(&document).expect("JSON document");
            std::fs::write(&path, &original).expect("malformed journal written");

            let journal = SendQueueJournal::at(path.clone());
            assert_eq!(
                journal.admit(
                    "thread-1",
                    "item-1",
                    "must not erase malformed data",
                    SendCommand::Enqueue,
                    ExecutorClass::NativeLoop,
                    SteerCapability::Unknown,
                ),
                Err(SendQueueRefusal::JournalUnreadable),
                "{name} was accepted as an empty journal"
            );
            assert_eq!(
                std::fs::read(path).expect("malformed journal remains readable"),
                original,
                "{name} was overwritten"
            );
        }
    }

    #[test]
    fn a_full_sequence_space_refuses_admission_without_overwrite() {
        let directory = tempfile::tempdir().expect("temporary queue directory");
        let path = directory.path().join(SEND_QUEUE_JOURNAL_FILE);
        let document = serde_json::json!({
            "schema": SEND_QUEUE_JOURNAL_SCHEMA,
            "items": [{
                "itemId": "item-1",
                "threadId": "thread-1",
                "sequence": u64::MAX - 1,
                "text": "last admissible sequence",
                "command": SendCommand::Enqueue.token(),
                "class": ExecutorClass::NativeLoop.token(),
                "capability": SteerCapability::Unknown.token(),
                "state": QueueItemState::Queued.token(),
            }],
            "threadStates": [],
        });
        let original = serde_json::to_vec_pretty(&document).expect("JSON document");
        std::fs::write(&path, &original).expect("full journal written");

        let journal = SendQueueJournal::at(path.clone());
        assert_eq!(
            journal.admit(
                "thread-1",
                "item-2",
                "must not overflow",
                SendCommand::Enqueue,
                ExecutorClass::NativeLoop,
                SteerCapability::Unknown,
            ),
            Err(SendQueueRefusal::NotPersisted)
        );
        assert_eq!(
            std::fs::read(path).expect("full journal remains readable"),
            original
        );
    }

    #[test]
    fn a_failed_edit_write_rolls_back_the_durable_body() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SEND_QUEUE_JOURNAL_FILE);
        let journal = SendQueueJournal::at(path.clone());
        admit(
            &journal,
            "item-1",
            SendCommand::Enqueue,
            ExecutorClass::NativeLoop,
            SteerCapability::Unknown,
        );
        std::fs::remove_file(&path).expect("remove journal file");
        std::fs::create_dir(&path).expect("replace journal file with directory");

        assert_eq!(
            journal.update(
                "thread-1",
                "item-1",
                "new text that was not saved",
                SendCommand::Enqueue,
                ExecutorClass::NativeLoop,
                SteerCapability::Unknown,
            ),
            Err(SendQueueRefusal::NotPersisted)
        );
        assert_eq!(
            journal
                .head_for("thread-1")
                .expect("item remains queued")
                .text,
            "look at the other file too"
        );
    }

    #[test]
    fn pause_and_absorbing_cancel_survive_a_journal_restart() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SEND_QUEUE_JOURNAL_FILE);
        {
            let journal = SendQueueJournal::at(path.clone());
            journal
                .set_processing_state("paused-thread", SendQueueProcessingState::Paused)
                .expect("pause persisted");
            journal
                .set_processing_state(
                    "cancelling-thread",
                    SendQueueProcessingState::AbsorbingCancel,
                )
                .expect("absorbing state persisted");
        }

        let reopened = SendQueueJournal::at(path);
        assert_eq!(
            reopened.processing_state("paused-thread"),
            SendQueueProcessingState::Paused
        );
        assert_eq!(
            reopened.processing_state("cancelling-thread"),
            SendQueueProcessingState::AbsorbingCancel
        );
    }

    #[test]
    fn a_cancelled_item_leaves_the_queue_and_stays_gone() {
        let (journal, _dir) = journal();
        admit(
            &journal,
            "item-1",
            SendCommand::Enqueue,
            ExecutorClass::NativeLoop,
            SteerCapability::Unknown,
        );
        assert!(journal.cancel("thread-1", "item-1").is_ok());
        assert!(journal.open_items("thread-1").is_empty());
        assert_eq!(
            journal.promote("thread-1", "item-1", Quiescence::Proven),
            Err(SendQueueRefusal::ItemIsTerminal)
        );
        assert_eq!(
            journal.cancel("thread-1", "item-1"),
            Err(SendQueueRefusal::ItemIsTerminal)
        );
    }

    /// Every state a queued item can be in says something different to the
    /// person who queued it. A shared phrase would make two of them
    /// indistinguishable in the composer.
    #[test]
    fn every_queue_state_says_something_a_reader_can_tell_apart() {
        let (journal, _dir) = journal();
        let mut phrases = Vec::new();
        for (index, state) in QueueItemState::all().iter().enumerate() {
            let item_id = format!("item-{index}");
            let mut item = admit(
                &journal,
                &item_id,
                SendCommand::Enqueue,
                ExecutorClass::NativeLoop,
                SteerCapability::Unknown,
            );
            item.state = *state;
            phrases.push(queue_item_phrase(&item));
        }
        let unique: std::collections::BTreeSet<_> = phrases.iter().collect();
        assert_eq!(unique.len(), phrases.len(), "{phrases:?} are not distinct");
    }

    /// The two commands are not the same command with a flag. An enqueue never
    /// reaches the running turn on any class, and the record says which was
    /// asked for.
    #[test]
    fn steer_and_enqueue_are_recorded_as_different_commands() {
        let (journal, _dir) = journal();
        let steered = admit(
            &journal,
            "item-steer",
            SendCommand::Steer,
            ExecutorClass::NativeLoop,
            SteerCapability::Unknown,
        );
        let queued = admit(
            &journal,
            "item-queue",
            SendCommand::Enqueue,
            ExecutorClass::NativeLoop,
            SteerCapability::Unknown,
        );
        assert_ne!(steered.command, queued.command);
        assert!(steered.disposition().reaches_running_turn());
        assert!(!queued.disposition().reaches_running_turn());
    }
}
