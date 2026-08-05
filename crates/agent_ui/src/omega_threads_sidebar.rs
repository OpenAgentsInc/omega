//! Zero base's threads sidebar: the past conversations, newest-created first.
//!
//! `OMEGA-DELTA-0118`, omega#114. The owner pressed "Toggle Threads Sidebar" in
//! a zero-base build and nothing happened: *"this 'Toggle Threads Sidebar' does
//! nothing when i click on it but i want it. i want threads sidebar to see
//! historical chats."*
//!
//! # Why this exists rather than the sidebar that was already there
//!
//! The menu entry named `multi_workspace::ToggleWorkspaceSidebar`. That
//! namespace is outside zero base's admitted set, so `App::set_action_gate`
//! refused it before any listener ran — the entry was a control that is drawn
//! and denied, which is the failure `OMEGA-DELTA-0053` names in as many words.
//!
//! Admitting the namespace was the wrong repair. `multi_workspace`'s sidebar is
//! the project switcher: it carries projects, workspaces, `NewThread`, imports
//! and a folder picker, and it lives on `MultiWorkspace`, above the workspace
//! that `OMEGA-DELTA-0053` seals. Letting it in would put the editor's
//! navigation back inside a mode whose premise is that it is absent, which is
//! `OMEGA-DELTA-0052` weakened by one line in a constant. So zero base gets a
//! surface of its own, reachable through an action in the `agent` namespace it
//! already admits, and neither `ADMITTED_NAMESPACES` nor `ADMITTED_ACTIONS`
//! changes.
//!
//! # Why the rows are decided here and drawn there
//!
//! Everything below is a pure function of the metadata store and what this
//! machine can run. A window is not needed to ask whether a row is honest, and
//! the two questions that were actually got wrong — the order, and what happens
//! when the recorded executor is not the current one — are both answerable in a
//! unit test. The drawing lives in `agent_panel.rs` beside the state it toggles.

use chrono::{DateTime, Utc};
use collections::HashSet;
use gpui::SharedString;
use omega_front_door::ExecutorClass;
use project::AgentId;
use workspace::PathList;

use crate::Agent;
use crate::omega_agent_supervision::SupervisedThreadLifecycle;
use crate::omega_executor_selector::SelectableExecutor;
use crate::thread_metadata_store::{ConversationOwner, ThreadId, ThreadMetadata};

/// How many rows the sidebar will draw.
///
/// A person browsing for a conversation they had is looking at the top of this
/// list, not at row 900. The bound is here rather than at the query because the
/// store is already in memory: this caps the *drawing*, which is the cost.
pub const MAX_ROWS: usize = 200;

/// One past conversation, as the sidebar shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadRow {
    pub thread_id: ThreadId,
    pub agent_id: AgentId,
    pub created_at: DateTime<Utc>,
    /// The title, or the default when a thread never got one.
    pub title: SharedString,
    /// A compact age — `3s`, `4m`, `2h`, `5d`.
    ///
    /// omega#100 found this the hard way in the `@`-mention list: threads are
    /// named by a summarisation model, so two conversations often carry the
    /// same title, and the owner's words were "if the last two chats have same
    /// name i cant tell the difference". The list is ordered by this field.
    /// Showing it is what makes the order legible.
    pub age: SharedString,
    /// Who ran it, when that is not Omega's own loop.
    ///
    /// `None` means the native loop, and nothing is drawn for it.
    /// `OMEGA-DELTA-0021`'s convention: naming Omega inside Omega is noise, and
    /// absence already means the default everywhere else in this crate.
    pub executor: Option<SharedString>,
    /// The folders the thread was opened against, for reopening it in them.
    pub folder_paths: PathList,
    /// The mark a row that cannot be reopened here carries, before it is
    /// clicked: `Codex — not installed`.
    ///
    /// Present exactly when [`refusal`](Self::refusal) is, and drawn in place
    /// of [`executor`](Self::executor), which it already names. A list whose
    /// dead rows are identical to its live ones is a list of dead ends that
    /// only announce themselves one click at a time, and on a fresh install
    /// that is most of it.
    pub unavailable_note: Option<SharedString>,
    /// Why this row cannot be reopened on this machine, if it cannot.
    ///
    /// Present means the row is drawn muted, carries
    /// [`unavailable_note`](Self::unavailable_note), and clicking it says this
    /// sentence instead of opening anything.
    pub refusal: Option<SharedString>,
    pub lifecycle: SupervisedThreadLifecycle,
}

impl ThreadRow {
    #[must_use]
    pub fn is_reopenable(&self) -> bool {
        self.refusal.is_none()
    }
}

/// The rows, newest-created first.
///
/// `entries` is the whole store; the ordering, the exclusions and the bound are
/// applied here so there is one answer to "what does the sidebar list" rather
/// than one per caller.
///
/// Drafts are excluded. A draft is a thread whose first message was never sent
/// — it has no session id and therefore no transcript — and the owner asked for
/// historical chats. A first message accepted while an executor is connecting
/// is the exception: it has no session id yet, but it is already a conversation
/// and remains listed while session creation catches up.
///
/// Archived threads are excluded for the same reason they are excluded
/// everywhere else: archiving is the act of saying "not in the list".
///
/// `unavailable` is `OMEGA-DELTA-0123`'s list, verbatim: the executors that
/// cannot run here, each with the reason the composer's selector shows beside
/// its greyed-out entry. It is passed in rather than recomputed so the sidebar
/// and that menu cannot give two answers to "why can I not use Codex" — the
/// same window giving two answers to one question is the failure omega#99
/// records about the onboarding card.
pub fn rows<'a>(
    entries: impl Iterator<Item = &'a ThreadMetadata>,
    now: DateTime<Utc>,
    unavailable: &[(SelectableExecutor, &'static str)],
    registered_agents: &[AgentId],
) -> Vec<ThreadRow> {
    rows_with_submitted_drafts(
        entries,
        now,
        unavailable,
        registered_agents,
        &HashSet::default(),
    )
}

/// Includes sessionless conversations whose first message was accepted while
/// their executor was still connecting.
pub fn rows_with_submitted_drafts<'a>(
    entries: impl Iterator<Item = &'a ThreadMetadata>,
    now: DateTime<Utc>,
    unavailable: &[(SelectableExecutor, &'static str)],
    registered_agents: &[AgentId],
    submitted_drafts: &HashSet<ThreadId>,
) -> Vec<ThreadRow> {
    let mut listed: Vec<&ThreadMetadata> = entries
        .filter(|thread| !thread.archived)
        .filter(|thread| !thread.is_draft() || submitted_drafts.contains(&thread.thread_id))
        .collect();

    // Creation time is immutable, so reading or resuming a conversation cannot
    // move it while somebody is navigating the list. Legacy rows without a
    // recorded creation time fall back to their first available timestamp.
    listed.sort_by(|left, right| {
        let left_created_at = left.created_at.unwrap_or(left.updated_at);
        let right_created_at = right.created_at.unwrap_or(right.updated_at);
        right_created_at.cmp(&left_created_at).then_with(|| {
            left.thread_id
                .to_key_string()
                .cmp(&right.thread_id.to_key_string())
        })
    });

    listed
        .into_iter()
        .take(MAX_ROWS)
        .map(|thread| {
            let created_at = thread.created_at.unwrap_or(thread.updated_at);
            let executor = recorded_executor(&thread.agent_id);
            let unreopenable = match thread.conversation_owner() {
                // Legacy owner ambiguity stays internal (omega#152 versioned
                // owner metadata). No sidebar annotation — the click still
                // refuses so a person is not dumped into a guessed session.
                ConversationOwner::LegacyAmbiguous(ref agent_id) => Some(Unreopenable {
                    note: None,
                    refusal: format!(
                        "This thread predates verified Direct Agent ownership. Its recorded id \
                         `{agent_id}` may be an owner or a routed executor, so Omega will not \
                         guess which agent owns the session."
                    )
                    .into(),
                }),
                ConversationOwner::LegacyOmega | ConversationOwner::Exact(_) => {
                    reopen_refusal(executor, &thread.agent_id, unavailable, registered_agents)
                }
            };
            ThreadRow {
                thread_id: thread.thread_id,
                agent_id: thread.agent_id.clone(),
                created_at,
                title: thread.display_title(),
                age: short_age(created_at, now).into(),
                executor: executor
                    .filter(|executor| *executor != SelectableExecutor::Omega)
                    .map(|executor| SharedString::new_static(executor.name())),
                folder_paths: thread.folder_paths().clone(),
                unavailable_note: unreopenable
                    .as_ref()
                    .and_then(|unreopenable| unreopenable.note.clone()),
                refusal: unreopenable.map(|unreopenable| unreopenable.refusal),
                lifecycle: thread.lifecycle,
            }
        })
        .collect()
}

/// Which of the four names ran this thread, if it is one of them.
///
/// Read from the `agent_id` the store recorded, which is the same field
/// `load_agent_thread` is handed when the thread is reopened. A row that named
/// one executor and reopened under another would be the dishonest attribution
/// `OMEGA-DELTA-0021` exists to stop.
#[must_use]
pub fn recorded_executor(agent_id: &AgentId) -> Option<SelectableExecutor> {
    if Agent::from(agent_id.clone()).is_native() {
        return Some(SelectableExecutor::Omega);
    }
    SelectableExecutor::of(ExecutorClass::ExternalAcp, agent_id.as_ref())
}

/// What a row that cannot be reopened here says, before and after the click.
///
/// One value with both halves because they are one fact told at two lengths.
/// Computed separately they could disagree, and a row marked live that refuses
/// — or marked dead that opens — is worse than the defect either was meant to
/// repair.
struct Unreopenable {
    /// On the row, unclicked: `Codex — not installed`.
    ///
    /// `None` when the row refuses without a pre-click annotation — legacy
    /// owner-ambiguous threads keep the fact internal (`OMEGA-DELTA-0189`).
    note: Option<SharedString>,
    /// After the click: the same reason, and what follows from it.
    refusal: SharedString,
}

/// Why this thread cannot be reopened here, if it cannot.
///
/// # The decision this encodes
///
/// **A thread is reopened under the executor that recorded it, never under the
/// one currently selected.** A session id is not portable: it names a
/// conversation inside the agent server that created it, and resuming a Codex
/// session on Claude's connection reaches an adapter that has never heard of it
/// and answers `no rollout found for thread id ...`. That sentence is about a
/// rollout file; the person's question was "can I have my chat back". So the
/// executor travels with the thread, which is what `load_agent_thread` already
/// does when it is handed `metadata.agent_id`.
///
/// That leaves exactly one case this cannot serve: the recorded executor is not
/// runnable on this machine at all. Nothing can be done about it here — the
/// transcript lives inside a program that is not here — so the row refuses in a
/// sentence rather than dispatching a load that fails three layers down in
/// somebody else's error text.
///
/// **The reason is `OMEGA-DELTA-0123`'s, not a second one written here.** That
/// delta made the composer's selector explain every name it cannot offer, and
/// `unavailable` is where those explanations live. A row that said "Codex is not
/// installed" while the menu two inches below said "installed; Omega hosts no
/// adapter for it" would send somebody to install what they already have. What
/// this adds is the consequence, which the menu has no reason to know: the
/// transcript is inside that executor, so no other one can produce it.
///
/// **The note is that menu's form as well as its reason.** A disabled entry
/// there reads `name — reason`; so does the row, so the two places a person
/// meets the same fact look like the same fact. The name is
/// [`SelectableExecutor::name`] rather than `selector_name`, because the row is
/// already labelled by `name` and the refusal sentence already spells it — a
/// row reading `Claude Code` above a sentence reading `Claude` would be a third
/// answer invented by the shorter of the two.
fn reopen_refusal(
    executor: Option<SelectableExecutor>,
    agent_id: &AgentId,
    unavailable: &[(SelectableExecutor, &'static str)],
    registered_agents: &[AgentId],
) -> Option<Unreopenable> {
    let consequence = "A thread's session belongs to the executor that made it, so opening it \
                       on a different one finds no transcript.";

    let Some(executor) = executor else {
        // Somebody else's adapter: not one of the four, so `unavailable` has no
        // opinion about it. The registry is the only honest test available.
        if registered_agents.iter().any(|known| known == agent_id) {
            return None;
        }
        let agent_id = agent_id.as_ref();
        return Some(Unreopenable {
            note: Some(format!("{agent_id} — not registered in this window").into()),
            refusal: format!(
                "This thread ran on the agent `{agent_id}`, which is not registered in \
                 this window. {consequence}"
            )
            .into(),
        });
    };

    let (_, reason) = unavailable
        .iter()
        .find(|(candidate, _)| *candidate == executor)?;
    let name = executor.name();

    Some(Unreopenable {
        note: Some(format!("{name} — {reason}").into()),
        refusal: format!(
            "This thread ran on {name}, which cannot run here: {reason}. {consequence}"
        )
        .into(),
    })
}

/// A short age, for sitting beside a thread title.
///
/// The same shape as the `@`-mention list's, and for the same reason: the row
/// is narrow, so "4 minutes ago" would push the title out of view, which is the
/// opposite of telling two same-named threads apart.
///
/// A future timestamp reads as `now` rather than a negative age. Clocks move
/// backwards, and a thread claiming to be `-3s` old would look like a defect in
/// the list rather than a defect in the clock.
///
/// `OMEGA-DELTA-0130` made this public rather than copying it. The sidebar's
/// public-chat section ages a Nostr message the same way, and two age
/// formatters in one column would eventually disagree about what `1h` means.
pub fn short_age(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(updated_at).num_seconds();
    if seconds <= 0 {
        return "now".to_string();
    }
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1 as acp;
    use chrono::TimeZone as _;

    /// A machine where all four executors run, expressed the way `rows` takes
    /// it: `OMEGA-DELTA-0123`'s `unavailable` list, with nothing in it.
    const ALL_READY: &[(SelectableExecutor, &'static str)] = &[];

    /// The native loop's own agent id.
    ///
    /// `"omega"` is not it. `Agent::from` recognises `agent::OMEGA_AGENT_ID`,
    /// which is `"Omega Agent"`, and every fixture here spelled the short word
    /// — so what these tests called a native thread was an unregistered
    /// adapter that happened to answer `None` to the only question they asked
    /// it. Found when `a_row_that_cannot_be_reopened_is_marked_before_it_is_clicked`
    /// asked a second question and got `omega — not registered in this window`.
    fn omega() -> &'static str {
        agent::OMEGA_AGENT_ID.as_ref()
    }

    fn thread(
        title: &str,
        agent: &str,
        updated_at: DateTime<Utc>,
        session: bool,
    ) -> ThreadMetadata {
        ThreadMetadata {
            thread_id: ThreadId::new(),
            session_id: session.then(|| acp::SessionId::new("session")),
            agent_id: AgentId::new(agent),
            conversation_owner_version: crate::thread_metadata_store::ConversationOwnerVersion::V1,
            title: Some(title.into()),
            title_override: None,
            updated_at,
            created_at: Some(updated_at),
            interacted_at: None,
            worktree_paths: Default::default(),
            remote_connection: None,
            archived: false,
            lifecycle: SupervisedThreadLifecycle::Completed,
        }
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + minutes * 60, 0)
            .single()
            .expect("a valid timestamp")
    }

    #[test]
    fn rows_are_newest_created_first_and_carry_an_age() {
        let now = at(10);
        let entries = [
            thread("older", omega(), at(0), true),
            thread("newer", omega(), at(9), true),
        ];
        let rows = rows(entries.iter(), now, ALL_READY, &[]);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title.as_ref(), "newer");
        assert_eq!(rows[0].age.as_ref(), "1m");
        assert_eq!(rows[1].title.as_ref(), "older");
        assert_eq!(rows[1].age.as_ref(), "10m");
    }

    #[test]
    fn rows_stay_in_creation_order_when_a_thread_is_updated() {
        let now = at(10);
        let mut older = thread("older", omega(), at(9), true);
        older.created_at = Some(at(0));
        let newer = thread("newer", omega(), at(5), true);

        let rows = rows([older, newer].iter(), now, ALL_READY, &[]);

        assert_eq!(rows[0].title.as_ref(), "newer");
        assert_eq!(rows[0].age.as_ref(), "5m");
        assert_eq!(rows[1].title.as_ref(), "older");
        assert_eq!(rows[1].age.as_ref(), "10m");
    }

    #[test]
    fn a_draft_is_not_a_historical_chat() {
        let now = at(10);
        let entries = [
            thread("sent", omega(), at(9), true),
            thread("never sent", omega(), at(9), false),
        ];
        let rows = rows(entries.iter(), now, ALL_READY, &[]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title.as_ref(), "sent");
    }

    #[test]
    fn a_submitted_draft_is_listed_while_its_session_is_connecting() {
        let now = at(10);
        let submitted = thread("first message", omega(), at(9), false);
        let submitted_drafts = [submitted.thread_id].into_iter().collect();

        let rows = rows_with_submitted_drafts(
            [&submitted].into_iter(),
            now,
            ALL_READY,
            &[],
            &submitted_drafts,
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].thread_id, submitted.thread_id);
    }

    #[test]
    fn an_archived_thread_is_not_listed() {
        let now = at(10);
        let mut archived = thread("archived", omega(), at(9), true);
        archived.archived = true;
        let entries = [archived];
        let rows = rows(entries.iter(), now, ALL_READY, &[]);

        assert!(rows.is_empty());
    }

    #[test]
    fn omega_is_not_named_on_a_row_and_another_executor_is() {
        let now = at(10);
        let entries = [
            thread("native", omega(), at(9), true),
            thread("codex", agent_servers::CODEX_ID, at(8), true),
        ];
        let rows = rows(entries.iter(), now, ALL_READY, &[]);

        assert_eq!(rows[0].executor, None);
        assert_eq!(rows[1].executor.as_deref(), Some("Codex"));
    }

    #[test]
    fn a_thread_reopens_under_its_own_executor_while_that_executor_can_run() {
        let now = at(10);
        let entries = [thread("codex", agent_servers::CODEX_ID, at(9), true)];

        // Claude selected, Codex installed: the thread is still reopenable,
        // because it reopens on Codex rather than on whatever is selected.
        let rows = rows(entries.iter(), now, ALL_READY, &[]);
        assert!(rows[0].is_reopenable(), "{:?}", rows[0].refusal);
    }

    #[test]
    fn a_thread_whose_executor_cannot_run_here_refuses_by_name() {
        let now = at(10);
        let entries = [thread("codex", agent_servers::CODEX_ID, at(9), true)];
        let rows = rows(
            entries.iter(),
            now,
            &[(SelectableExecutor::Codex, "not installed")],
            &[],
        );

        let refusal = rows[0]
            .refusal
            .as_ref()
            .expect("a thread whose executor is missing must refuse");
        assert!(refusal.contains("Codex"), "{refusal}");
        assert!(
            refusal.contains("not installed"),
            "the reason must be the one the composer's selector shows, not a \
             second one written here: {refusal}"
        );
        assert!(
            !refusal.contains("rollout"),
            "the refusal must be about the executor, not about somebody \
             else's adapter error: {refusal}"
        );
    }

    /// The row says it before it is clicked, not after.
    ///
    /// The owner met this list on a machine without Codex, clicked several rows
    /// in a row, and got the refusal each time: *"You're showing me histories I
    /// click on and it's a yellow warning."* Every one of those rows already
    /// knew. What it did not do was say so, so the mark and the reason are
    /// asserted on the unclicked row here.
    #[test]
    fn a_row_that_cannot_be_reopened_is_marked_before_it_is_clicked() {
        let now = at(10);
        let entries = [
            thread("codex", agent_servers::CODEX_ID, at(9), true),
            thread("native", omega(), at(8), true),
        ];
        let rows = rows(
            entries.iter(),
            now,
            &[(SelectableExecutor::Codex, "not installed")],
            &[],
        );

        let note = rows[0]
            .unavailable_note
            .as_ref()
            .expect("a row that will refuse must say so before the click");
        assert_eq!(
            note.as_ref(),
            "Codex — not installed",
            "the mark is the composer selector's form and the composer \
             selector's reason, so one window gives one answer"
        );
        assert!(
            rows[0]
                .refusal
                .as_ref()
                .is_some_and(|refusal| refusal.contains("not installed")),
            "the mark and the sentence are one fact, so neither may exist \
             without the other: {:?}",
            rows[0].refusal
        );

        assert!(
            rows[1].unavailable_note.is_none() && rows[1].is_reopenable(),
            "a row that opens carries no mark, or the mark means nothing: {:?}",
            rows[1].unavailable_note
        );
    }

    #[test]
    fn an_unknown_adapter_is_reopenable_only_while_it_is_registered() {
        let now = at(10);
        let entries = [thread("other", "somebody-elses-agent", at(9), true)];

        let refused = rows(entries.iter(), now, ALL_READY, &[]);
        assert!(
            refused[0]
                .refusal
                .as_ref()
                .is_some_and(|refusal| refusal.contains("somebody-elses-agent")),
            "{:?}",
            refused[0].refusal
        );
        assert_eq!(
            refused[0].unavailable_note.as_deref(),
            Some("somebody-elses-agent — not registered in this window"),
            "an adapter that is not one of the four is still a dead row, and a \
             dead row that looks live is the whole defect"
        );

        let registered = [AgentId::new("somebody-elses-agent")];
        let admitted = rows(entries.iter(), now, ALL_READY, &registered);
        assert!(admitted[0].is_reopenable(), "{:?}", admitted[0].refusal);
        assert!(admitted[0].unavailable_note.is_none());
    }

    /// Legacy owner-ambiguous threads stay reopen-refused, but carry no
    /// sidebar annotation. OMEGA-DELTA-0189 / omega#160 item 9.
    #[test]
    fn a_legacy_ambiguous_thread_has_no_sidebar_annotation() {
        let now = at(10);
        let mut legacy = thread("legacy codex", agent_servers::CODEX_ID, at(9), true);
        legacy.conversation_owner_version =
            crate::thread_metadata_store::ConversationOwnerVersion::Legacy;
        let rows = rows([legacy].iter(), now, ALL_READY, &[]);

        assert!(
            rows[0].unavailable_note.is_none(),
            "legacy ownership stays internal — no sidebar annotation: {:?}",
            rows[0].unavailable_note
        );
        assert!(
            rows[0]
                .refusal
                .as_ref()
                .is_some_and(|refusal| refusal.contains("predates verified")),
            "click still refuses so the session is not guessed: {:?}",
            rows[0].refusal
        );
        assert!(!rows[0].is_reopenable());
    }

    #[test]
    fn the_list_is_bounded() {
        let now = at(10_000);
        let entries: Vec<ThreadMetadata> = (0..MAX_ROWS + 50)
            .map(|index| thread("thread", omega(), at(index as i64), true))
            .collect();
        let rows = rows(entries.iter(), now, ALL_READY, &[]);

        assert_eq!(rows.len(), MAX_ROWS);
    }

    #[test]
    fn a_future_timestamp_reads_as_now() {
        assert_eq!(short_age(at(11), at(10)), "now");
    }
}
