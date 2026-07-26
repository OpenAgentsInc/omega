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
}

impl SendQueueRefusal {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ItemIsTerminal => "item_is_terminal",
            Self::UnknownItem => "unknown_item",
            Self::NotQuiescent => "not_quiescent",
            Self::NotPersisted => "not_persisted",
        }
    }
}

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
    next_sequence: RefCell<u64>,
}

impl SendQueueJournal {
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
    /// An unreadable journal starts empty **and says so loudly**. It does not
    /// silently continue, because a queue that quietly forgot an admitted
    /// message is the defect this module exists to prevent, and a warning is
    /// the only honest thing available at this layer.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        let items = load(&path).unwrap_or_else(|error| {
            log::warn!(
                "OMEGA-DELTA-0032: send queue at {} could not be read ({error:#}); \
                 starting from empty. Any admitted message it held is lost.",
                path.display()
            );
            BTreeMap::new()
        });
        let next = items.values().map(|item| item.sequence).max().unwrap_or(0) + 1;
        Self {
            path,
            items: RefCell::new(items),
            next_sequence: RefCell::new(next),
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
        let sequence = {
            let mut next = self.next_sequence.borrow_mut();
            let sequence = *next;
            *next += 1;
            sequence
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
        self.items
            .borrow_mut()
            .insert(Self::key(thread_id, item_id), item.clone());
        match self.persist() {
            Ok(()) => Ok(item),
            Err(error) => {
                log::error!(
                    "OMEGA-DELTA-0032: {item_id} could not be admitted to {}: {error:#}",
                    self.path.display()
                );
                self.items
                    .borrow_mut()
                    .remove(&Self::key(thread_id, item_id));
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

    fn transition(
        &self,
        key: &str,
        state: QueueItemState,
    ) -> Result<QueuedSend, SendQueueRefusal> {
        let updated = {
            let mut items = self.items.borrow_mut();
            let item = items.get_mut(key).ok_or(SendQueueRefusal::UnknownItem)?;
            item.state = state;
            item.clone()
        };
        match self.persist() {
            Ok(()) => Ok(updated),
            Err(error) => {
                log::error!(
                    "OMEGA-DELTA-0032: {key} could not record {}: {error:#}",
                    state.token()
                );
                Err(SendQueueRefusal::NotPersisted)
            }
        }
    }

    fn key(thread_id: &str, item_id: &str) -> String {
        format!("{thread_id}\u{1f}{item_id}")
    }

    fn persist(&self) -> anyhow::Result<()> {
        let items = self.items.borrow();
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

fn load(path: &Path) -> anyhow::Result<BTreeMap<String, QueuedSend>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let document: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let schema = document.get("schema").and_then(Value::as_str);
    anyhow::ensure!(
        schema == Some(SEND_QUEUE_JOURNAL_SCHEMA),
        "unsupported send queue schema {schema:?}"
    );
    let mut items = BTreeMap::new();
    for entry in document
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
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
        items.insert(
            SendQueueJournal::key(thread_id, item_id),
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
    Ok(items)
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
            .admit("thread-1", item_id, "look at the other file too", command, class, capability)
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
        assert!(journal.promote("thread-1", "item-1", Quiescence::Proven).is_ok());
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
        assert_eq!(journal.head_for("thread-1").expect("head").item_id, "zzz-first");
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
        assert!(SendQueueJournal::at(path).open_items("thread-1").is_empty());
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
