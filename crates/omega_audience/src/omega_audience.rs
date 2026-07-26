//! Who can read a conversation.
//!
//! omega#107 asked for "workspace identity and selection". The concept is
//! right and the word is not, so this crate does not use it. Omega already
//! spends *project* on a directory (`OMEGA-DELTA-0054` gives zero base a
//! working directory to open), Zed spends `Workspace` on a window, and
//! `crates/workroom_receipts` spends *room* on a place a machine does work.
//! A fourth meaning for a word already carrying three is how a person ends up
//! reading "workspace" in the composer and thinking about their folder.
//!
//! So the thing a thread belongs to is an **audience**: the set of people who
//! can read it. That is the fact a person actually needs before they type, it
//! is the fact omega#108 puts a Forge repository behind, and it is the only
//! one of the four words nothing in this repository had already taken.
//!
//! # The one property that matters
//!
//! **An audience is recorded on the thread. It is never inferred from what is
//! selected now.**
//!
//! The alternative — read the current selection at draw time — looks identical
//! on a machine with one audience, and is a disclosure defect the moment there
//! are two. Selecting a community audience would repaint every thread already
//! on screen as belonging to it, so a conversation somebody held in private
//! last week would render as one they had held in public. Nothing would have
//! been published, and the person would have no way to know that. They would
//! reasonably conclude the opposite.
//!
//! [`AudienceBook`] therefore binds once and refuses to rebind, and
//! [`AudienceBook::audience_of`] falls back to [`Audience::local`] rather than
//! to a selection — see [`audience_for_opening`] for why the fallback direction
//! is the whole safety argument.
//!
//! # Why this crate holds no state of its own and touches no clock, disk or
//! socket
//!
//! The same reason `omega_workdir` and `omega_agent_detect` take their inputs
//! as parameters. The rules here decide what a shipped binary discloses, and a
//! rule that reads ambient state can only be tested on a machine that happens
//! to be in the right state. Everything below is a value or a function of
//! values. `crates/agent_ui/src/omega_audience_control.rs` is the only place
//! that knows about a key-value store or a window.

#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The reserved key of the audience that is always present.
///
/// Reserved in both directions: [`AudienceId::local`] produces it, and
/// [`AudienceId::joined`] refuses it, so a joined audience can never arrive
/// wearing the local one's name.
pub const LOCAL_KEY: &str = "local";

/// What a person reads in the composer when the audience is [`Audience::local`].
///
/// omega#107 acceptance 1 is "a fresh profile opens in **Local**, and says so",
/// so the word is pinned here rather than written at each call site.
pub const LOCAL_NAME: &str = "Local";

/// The sentence the local audience is entitled to make, in full.
///
/// Kept beside the name because the name alone is a label and the promise is
/// what a person is actually deciding on.
pub const LOCAL_DESCRIPTION: &str = "Only this computer. No account, no relay, no network.";

/// How far a conversation in an audience travels.
///
/// Two variants, deliberately, and the boundary between them is the one
/// omega#108 must not be able to blur: [`Reach::ThisComputer`] is defined by
/// what it *cannot* do, so adding a third kind of shared audience later cannot
/// quietly widen it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    /// Nowhere. No account, no relay, no network, no other reader.
    ///
    /// The default, and the only reach a fresh profile can be in.
    ThisComputer,
    /// Everyone admitted to a shared place.
    ///
    /// omega#108 puts a Forge repository behind this. Nothing in this crate
    /// knows how to reach one, which is the point: this is the audience
    /// concept, not the transport.
    Shared,
}

impl Reach {
    /// Does a conversation in this audience stay on the machine it was typed
    /// on?
    #[must_use]
    pub const fn is_private_to_this_computer(self) -> bool {
        matches!(self, Self::ThisComputer)
    }
}

/// Why an audience identifier was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudienceIdError {
    /// The key was empty, or only whitespace.
    Empty,
    /// The key was [`LOCAL_KEY`], which only [`AudienceId::local`] may produce.
    ///
    /// A joined audience calling itself `local` would render with the local
    /// name and the local promise while carrying [`Reach::Shared`], which is
    /// the disclosure defect this crate exists to prevent, arriving through the
    /// front door.
    ReservedLocalKey,
}

impl fmt::Display for AudienceIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("an audience identifier cannot be empty"),
            Self::ReservedLocalKey => {
                formatter.write_str("`local` is reserved for the audience that is always present")
            }
        }
    }
}

/// The stable identity of an audience.
///
/// An opaque validated string rather than an enum, because omega#108's
/// community audience is identified by a Forge repository coordinate that this
/// crate has no business parsing. What it does enforce is that exactly one
/// value means "local".
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AudienceId(String);

impl AudienceId {
    /// The audience every profile has, before anything is joined.
    #[must_use]
    pub fn local() -> Self {
        Self(LOCAL_KEY.to_string())
    }

    /// An identifier for an audience somebody joined.
    ///
    /// # Errors
    ///
    /// [`AudienceIdError::Empty`] for a blank key, and
    /// [`AudienceIdError::ReservedLocalKey`] for an attempt to take the local
    /// audience's name.
    pub fn joined(key: impl Into<String>) -> Result<Self, AudienceIdError> {
        let key = key.into();
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err(AudienceIdError::Empty);
        }
        if trimmed == LOCAL_KEY {
            return Err(AudienceIdError::ReservedLocalKey);
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Is this the audience that is always present?
    #[must_use]
    pub fn is_local(&self) -> bool {
        self.0 == LOCAL_KEY
    }

    /// The stored form, for a key-value store or a record on disk.
    #[must_use]
    pub fn as_key(&self) -> &str {
        &self.0
    }

    /// Reads back a stored key.
    ///
    /// Unknown keys survive the round trip rather than becoming local, so that
    /// a thread recorded in an audience this build does not know about is
    /// reported as unresolved by [`AudienceRoster::resolve`] instead of being
    /// silently relabelled with the local promise. See
    /// [`ThreadAudience::Unresolved`].
    #[must_use]
    pub fn from_key(key: &str) -> Self {
        if key.trim() == LOCAL_KEY {
            Self::local()
        } else {
            Self(key.trim().to_string())
        }
    }
}

/// An audience a thread can belong to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Audience {
    id: AudienceId,
    name: String,
    reach: Reach,
}

impl Audience {
    /// The audience that is always present.
    ///
    /// Constructed rather than configured. There is no argument that makes it
    /// reachable over a network and no argument that renames it, because
    /// omega#107 acceptance 1 is that a fresh profile opens here and says so.
    #[must_use]
    pub fn local() -> Self {
        Self {
            id: AudienceId::local(),
            name: LOCAL_NAME.to_string(),
            reach: Reach::ThisComputer,
        }
    }

    /// An audience somebody joined.
    ///
    /// Always [`Reach::Shared`]. A joined audience that claimed to be private
    /// to this computer would be a lie with a checkbox in front of it, so the
    /// reach is a consequence of the constructor rather than a parameter.
    ///
    /// # Errors
    ///
    /// As [`AudienceId::joined`].
    pub fn joined(
        key: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, AudienceIdError> {
        Ok(Self {
            id: AudienceId::joined(key)?,
            name: name.into(),
            reach: Reach::Shared,
        })
    }

    /// This audience's stable identity.
    #[must_use]
    pub fn id(&self) -> &AudienceId {
        &self.id
    }

    /// What a person reads in the composer.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How far a conversation here travels.
    #[must_use]
    pub fn reach(&self) -> Reach {
        self.reach
    }

    /// Is this audience the one that needs nothing?
    #[must_use]
    pub fn is_local(&self) -> bool {
        self.id.is_local()
    }

    /// The full sentence, for a tooltip or a menu line.
    ///
    /// The local one is fixed text because it is a promise this crate keeps.
    /// A joined one describes its reach rather than its membership, because
    /// membership is omega#108's to answer and a guess here would be a
    /// confident wrong answer about who can read something.
    #[must_use]
    pub fn description(&self) -> String {
        if self.is_local() {
            LOCAL_DESCRIPTION.to_string()
        } else {
            format!("Shared with everyone in {}. Needs a network.", self.name)
        }
    }
}

/// What the selector offers.
///
/// Local first and local always, by construction rather than by convention:
/// [`AudienceRoster::entries`] yields it before anything else and
/// [`AudienceRoster::new`] cannot be handed a list that removes it. omega#107
/// acceptance 4 is that a profile which has joined nothing still sees Local and
/// does not read as broken, and an empty roster is the state every profile is
/// in until omega#108 exists.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AudienceRoster {
    joined: Vec<Audience>,
}

impl AudienceRoster {
    /// A roster of the audiences somebody has joined.
    ///
    /// Anything claiming to be local is dropped rather than accepted, so the
    /// first entry is the real one. `AudienceId::joined` already refuses the
    /// reserved key, and this is the second half of the same rule for a value
    /// that arrived some other way — a future decoder, say.
    #[must_use]
    pub fn new(joined: impl IntoIterator<Item = Audience>) -> Self {
        let mut seen: Vec<AudienceId> = vec![AudienceId::local()];
        let mut kept = Vec::new();
        for audience in joined {
            if audience.is_local() || seen.contains(audience.id()) {
                continue;
            }
            seen.push(audience.id().clone());
            kept.push(audience);
        }
        Self { joined: kept }
    }

    /// Every audience the selector shows, local first.
    pub fn entries(&self) -> impl Iterator<Item = Audience> + '_ {
        std::iter::once(Audience::local()).chain(self.joined.iter().cloned())
    }

    /// How many entries the selector shows. Never zero.
    #[must_use]
    pub fn len(&self) -> usize {
        self.joined.len() + 1
    }

    /// Always false. Present so a caller cannot write `entries().next().is_none()`
    /// and act on it.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Has this profile joined anything at all?
    ///
    /// The honest question to ask before showing a menu, as distinct from
    /// asking whether the roster is empty, which it never is.
    #[must_use]
    pub fn has_joined_anything(&self) -> bool {
        !self.joined.is_empty()
    }

    /// The audience with this identity, if this build knows it.
    #[must_use]
    pub fn resolve(&self, id: &AudienceId) -> Option<Audience> {
        self.entries().find(|audience| audience.id() == id)
    }

    /// What to draw for a thread bound to `id`.
    ///
    /// An identity the roster cannot resolve is reported as
    /// [`ThreadAudience::Unresolved`] rather than falling back to local. The
    /// fallback would be the safe direction for a *missing* record — see
    /// [`AudienceBook::audience_of`] — and the wrong one for a record that says
    /// the thread belongs somewhere this build cannot see. "This is private"
    /// and "I cannot tell you who can read this" are different sentences, and
    /// a person who is shown the first when the second is true has been misled
    /// by their own editor.
    #[must_use]
    pub fn describe(&self, id: &AudienceId) -> ThreadAudience {
        match self.resolve(id) {
            Some(audience) => ThreadAudience::Known(audience),
            None => ThreadAudience::Unresolved(id.clone()),
        }
    }
}

/// What a thread's recorded audience means to this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadAudience {
    /// The roster knows it, so the composer can name it and promise for it.
    Known(Audience),
    /// The thread names an audience this profile has not joined, or no longer
    /// has.
    ///
    /// Rendered as unknown, never as local.
    Unresolved(AudienceId),
}

impl ThreadAudience {
    /// What a person reads in the composer.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Known(audience) => audience.name().to_string(),
            Self::Unresolved(_) => "Unknown audience".to_string(),
        }
    }

    /// The full sentence, for a tooltip.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::Known(audience) => audience.description(),
            Self::Unresolved(id) => format!(
                "This thread was started in an audience this profile has not joined ({}). \
                 Omega cannot say who can read it.",
                id.as_key()
            ),
        }
    }

    /// Can Omega promise this thread stayed on this machine?
    ///
    /// `false` for an unresolved audience, because an unanswerable question is
    /// not a yes.
    #[must_use]
    pub fn is_private_to_this_computer(&self) -> bool {
        match self {
            Self::Known(audience) => audience.reach().is_private_to_this_computer(),
            Self::Unresolved(_) => false,
        }
    }
}

/// Why a thread may not publish.
///
/// omega#108's falsifier is "publish from a local thread: it must be refused.
/// Watch it be refused." The refusal belongs here rather than there, because
/// the audience is the authority on it and because a rule stated once cannot be
/// stated differently by the second caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishRefused {
    /// The thread is local. There is nowhere to publish to, and that is the
    /// point of it.
    ThreadIsLocal,
    /// The thread names an audience this profile cannot see.
    ///
    /// Refused rather than attempted: publishing into a place Omega cannot
    /// describe is publishing to an audience nobody can name.
    AudienceUnresolved(AudienceId),
}

impl fmt::Display for PublishRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ThreadIsLocal => formatter.write_str(
                "this thread is local, so there is nobody to publish it to. Start a \
                 thread in a shared audience instead.",
            ),
            Self::AudienceUnresolved(id) => write!(
                formatter,
                "this thread belongs to `{}`, which this profile has not joined.",
                id.as_key()
            ),
        }
    }
}

/// May a thread publish, and to whom?
///
/// The authorization omega#108 has to perform **before** an effect rather than
/// after it. It is a function of the thread's recorded audience and of nothing
/// else — in particular not of the current selection, which is why it takes a
/// [`ThreadAudience`] and not an [`AudienceRoster`] plus a choice.
///
/// # Errors
///
/// [`PublishRefused`], which is a sentence a person can read.
pub fn may_publish(audience: &ThreadAudience) -> Result<&Audience, PublishRefused> {
    match audience {
        ThreadAudience::Known(audience) if audience.reach() == Reach::Shared => Ok(audience),
        ThreadAudience::Known(_) => Err(PublishRefused::ThreadIsLocal),
        ThreadAudience::Unresolved(id) => Err(PublishRefused::AudienceUnresolved(id.clone())),
    }
}

/// Why a rebinding was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebindRefused {
    /// The audience the thread was started in, which still stands.
    pub recorded: AudienceId,
    /// The audience something tried to move it to.
    pub attempted: AudienceId,
}

impl fmt::Display for RebindRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a thread keeps the audience it was started in: recorded `{}`, refused `{}`",
            self.recorded.as_key(),
            self.attempted.as_key()
        )
    }
}

/// How a thread arrived on screen.
///
/// The distinction is the whole of [`audience_for_opening`], and it has to be
/// taken from the caller rather than measured, because the two ways to measure
/// it at draw time are both wrong. `entries().is_empty()` is true of a resumed
/// thread whose entries have not loaded yet, and the current selection is the
/// thing that must never decide this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadOpening {
    /// A thread that did not exist a moment ago.
    ///
    /// In `agent_ui` this is `ConversationView::new` with no `resume_session_id`
    /// and no `thread_id`: nothing to resume and no record to reattach to.
    Started,
    /// A thread that already existed, being opened again.
    Resumed,
}

/// The audience a thread opening in this state belongs to.
///
/// Three rules, and the asymmetry between the last two is the safety argument:
///
/// - A thread with a record keeps it. Always, whatever is selected.
/// - A thread being **started** takes the selection. That is what selecting an
///   audience is for.
/// - A thread being **resumed** with no record is local — *not* the selection.
///
/// The third rule is the one worth arguing for. Threads predating this feature
/// have no record, and they are exactly the threads somebody held in private.
/// If resuming one adopted the current selection, then joining a community
/// audience and opening last month's conversation would render it as public.
/// Nothing would have been published; the person would simply have been told
/// their private thread was not private, by the only thing they could ask.
/// So the missing record resolves toward the promise Omega can keep.
#[must_use]
pub fn audience_for_opening(
    recorded: Option<&AudienceId>,
    selected: &AudienceId,
    opening: ThreadOpening,
) -> AudienceId {
    match (recorded, opening) {
        (Some(recorded), _) => recorded.clone(),
        (None, ThreadOpening::Started) => selected.clone(),
        (None, ThreadOpening::Resumed) => AudienceId::local(),
    }
}

/// Which audience each thread was started in.
///
/// A side record keyed by thread identity rather than a field on `AcpThread`,
/// for the reason `OMEGA-DELTA-0021` gives for the executor disclosure: putting
/// Omega's own state into a shared upstream type makes every rebase of that
/// crate a merge conflict. The binding is on the thread either way — what
/// matters is that it is *recorded*, not where the struct lives.
///
/// `T` is the thread key so `agent_ui` can use its own `ThreadId` without this
/// crate depending on it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AudienceBook<T: Ord> {
    bindings: BTreeMap<T, AudienceId>,
}

impl<T: Ord + Clone> AudienceBook<T> {
    /// An empty book. Every thread in it is local.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: BTreeMap::new(),
        }
    }

    /// Records the audience a thread was started in.
    ///
    /// # Errors
    ///
    /// [`RebindRefused`] if the thread already has one. This is the mechanical
    /// half of omega#107 deliverable 5 — "switching does not move an existing
    /// thread" — and it is an error rather than a silent no-op so that a caller
    /// which believes it is moving a thread finds out that it is not.
    ///
    /// Recording the audience a thread already has is not a rebinding and
    /// succeeds, so an idempotent caller does not have to check first.
    pub fn bind(&mut self, thread: T, audience: AudienceId) -> Result<(), RebindRefused> {
        match self.bindings.get(&thread) {
            Some(recorded) if recorded == &audience => Ok(()),
            Some(recorded) => Err(RebindRefused {
                recorded: recorded.clone(),
                attempted: audience,
            }),
            None => {
                self.bindings.insert(thread, audience);
                Ok(())
            }
        }
    }

    /// The audience recorded for a thread, if there is one.
    ///
    /// The honest form, for a caller that needs to distinguish "recorded as
    /// local" from "not recorded". Everything that draws should use
    /// [`Self::audience_of`].
    #[must_use]
    pub fn recorded(&self, thread: &T) -> Option<&AudienceId> {
        self.bindings.get(thread)
    }

    /// The audience of a thread.
    ///
    /// A thread with no record is local. Never the selection — see
    /// [`audience_for_opening`]. This function does not take a selection, which
    /// is the point: there is no call site that could pass one.
    #[must_use]
    pub fn audience_of(&self, thread: &T) -> AudienceId {
        self.bindings
            .get(thread)
            .cloned()
            .unwrap_or_else(AudienceId::local)
    }

    /// How many threads have a recorded audience.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Has nothing been recorded yet?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// omega#107 acceptance 1.
    #[test]
    fn a_fresh_profile_is_local_and_local_needs_nothing() {
        let local = Audience::local();

        assert!(local.is_local());
        assert_eq!(local.name(), "Local");
        assert_eq!(local.reach(), Reach::ThisComputer);
        assert!(
            local.reach().is_private_to_this_computer(),
            "a fresh profile must reach an audience that needs no account, no \
             relay and no network"
        );
    }

    /// omega#107 acceptance 4, and the state every profile is in until #108.
    #[test]
    fn a_roster_with_nothing_joined_still_offers_local_and_does_not_read_as_empty() {
        let roster = AudienceRoster::default();

        assert!(!roster.is_empty());
        assert_eq!(roster.len(), 1);
        assert!(!roster.has_joined_anything());
        let entries: Vec<_> = roster.entries().collect();
        assert_eq!(entries, vec![Audience::local()]);
    }

    #[test]
    fn local_is_first_however_the_roster_was_built() {
        let roster = AudienceRoster::new([
            Audience::joined("forge:omega", "Omega development").expect("a joined audience"),
            Audience::joined("forge:other", "Other").expect("a joined audience"),
        ]);

        assert_eq!(roster.len(), 3);
        assert!(
            roster
                .entries()
                .next()
                .expect("a roster always has a first entry")
                .is_local(),
            "the audience that needs nothing is the one a person reaches first"
        );
    }

    /// The reserved key, refused at both doors.
    #[test]
    fn nothing_joined_can_call_itself_local() {
        assert_eq!(
            AudienceId::joined("local"),
            Err(AudienceIdError::ReservedLocalKey)
        );
        assert_eq!(
            AudienceId::joined("  local  "),
            Err(AudienceIdError::ReservedLocalKey),
            "whitespace is not a different name"
        );
        assert_eq!(AudienceId::joined("   "), Err(AudienceIdError::Empty));

        // And the second door: a value that arrived some other way is dropped
        // from the roster rather than shown beside the real local entry.
        let impostor = Audience {
            id: AudienceId::local(),
            name: "Local".to_string(),
            reach: Reach::Shared,
        };
        let roster = AudienceRoster::new([impostor]);
        assert_eq!(roster.len(), 1);
        assert_eq!(
            roster
                .resolve(&AudienceId::local())
                .expect("local resolves")
                .reach(),
            Reach::ThisComputer,
            "the local entry must be the constructed one, not a shared audience \
             wearing its name"
        );
    }

    #[test]
    fn a_joined_audience_is_always_shared() {
        let joined =
            Audience::joined("forge:omega", "Omega development").expect("a joined audience");

        assert_eq!(joined.reach(), Reach::Shared);
        assert!(!joined.reach().is_private_to_this_computer());
        assert!(joined.description().contains("Needs a network"));
    }

    /// omega#107 deliverable 5.
    #[test]
    fn a_thread_keeps_the_audience_it_was_started_in() {
        let community = AudienceId::joined("forge:omega").expect("a joined identity");
        let mut book = AudienceBook::new();

        book.bind("thread-a", AudienceId::local())
            .expect("a first binding is recorded");

        let refusal = book
            .bind("thread-a", community.clone())
            .expect_err("a bound thread cannot be moved");
        assert_eq!(
            refusal,
            RebindRefused {
                recorded: AudienceId::local(),
                attempted: community,
            }
        );
        assert_eq!(book.audience_of(&"thread-a"), AudienceId::local());
    }

    #[test]
    fn recording_the_audience_a_thread_already_has_is_not_a_rebinding() {
        let mut book = AudienceBook::new();
        book.bind("thread-a", AudienceId::local()).expect("bound");

        assert_eq!(
            book.bind("thread-a", AudienceId::local()),
            Ok(()),
            "an idempotent caller must not have to check first"
        );
    }

    /// The falsifier omega#107 names, executed rather than described.
    ///
    /// "Remove the workspace from the thread record and infer it from the
    /// current selection: switching must then appear to rewrite an old thread's
    /// audience, and a check must fail on exactly that."
    ///
    /// So the removed implementation is written out below and compared against
    /// the real one on the same inputs. The assertion is that they *disagree*:
    /// if `audience_of` ever starts consulting a selection, the two collapse
    /// onto the same answer and this test fails.
    #[test]
    fn inferring_from_the_selection_rewrites_an_old_threads_audience() {
        /// The implementation omega#107 forbids: no record, read the selection.
        fn inferred_from_selection(selected: &AudienceId) -> AudienceId {
            selected.clone()
        }

        let community = AudienceId::joined("forge:omega").expect("a joined identity");
        let mut book = AudienceBook::new();
        book.bind("yesterdays-private-thread", AudienceId::local())
            .expect("bound when it was started");

        // The person joins a community audience and selects it. Nothing about
        // yesterday's thread has changed.
        let selected = community.clone();

        assert_eq!(
            inferred_from_selection(&selected),
            community,
            "this is the defect, stated: the inferring implementation reports \
             yesterday's private thread as belonging to the audience selected \
             today"
        );
        assert_eq!(
            book.audience_of(&"yesterdays-private-thread"),
            AudienceId::local(),
            "the recorded implementation must not move it"
        );
        assert_ne!(
            book.audience_of(&"yesterdays-private-thread"),
            inferred_from_selection(&selected),
            "the record and the inference must disagree here. If they agree, \
             the audience is being read from the selection and a private \
             conversation renders as a public one."
        );
    }

    /// The fallback direction of [`AudienceBook::audience_of`], on its own.
    ///
    /// Added after a falsification found nothing holding it. Changing the
    /// `unwrap_or_else` to produce a shared audience left all fifteen other
    /// tests green, because every one of them binds the thread first and so
    /// never reaches the fallback at all. A thread with no record is the
    /// common case on the build where this feature first ships — every thread
    /// on the machine is one — so the untested branch was the one that would
    /// have run most.
    #[test]
    fn a_thread_with_no_record_at_all_is_local() {
        let book: AudienceBook<&str> = AudienceBook::new();

        assert!(book.is_empty());
        assert_eq!(book.recorded(&"never-seen"), None);
        assert_eq!(
            book.audience_of(&"never-seen"),
            AudienceId::local(),
            "an unrecorded thread must resolve to the audience Omega can keep a \
             promise about, and to nothing else"
        );
        assert!(
            AudienceRoster::default()
                .describe(&book.audience_of(&"never-seen"))
                .is_private_to_this_computer()
        );
    }

    /// The third rule of [`audience_for_opening`], which is the one that keeps
    /// threads written before this feature private.
    #[test]
    fn a_resumed_thread_with_no_record_is_local_not_the_selection() {
        let community = AudienceId::joined("forge:omega").expect("a joined identity");

        assert_eq!(
            audience_for_opening(None, &community, ThreadOpening::Resumed),
            AudienceId::local(),
            "a thread that predates the audience record is one somebody held in \
             private, and resuming it must not adopt today's selection"
        );
    }

    #[test]
    fn a_started_thread_takes_the_selection() {
        let community = AudienceId::joined("forge:omega").expect("a joined identity");

        assert_eq!(
            audience_for_opening(None, &community, ThreadOpening::Started),
            community,
            "selecting an audience is for the next thread; if it did not apply \
             there it would apply nowhere"
        );
    }

    #[test]
    fn a_recorded_thread_ignores_the_selection_however_it_opens() {
        let community = AudienceId::joined("forge:omega").expect("a joined identity");

        for opening in [ThreadOpening::Started, ThreadOpening::Resumed] {
            assert_eq!(
                audience_for_opening(Some(&AudienceId::local()), &community, opening),
                AudienceId::local(),
                "{opening:?}"
            );
        }
    }

    /// An audience the profile cannot see must not be relabelled as private.
    #[test]
    fn an_unresolved_audience_reads_as_unknown_and_never_as_local() {
        let roster = AudienceRoster::default();
        let departed = AudienceId::joined("forge:omega").expect("a joined identity");

        let described = roster.describe(&departed);
        assert_eq!(described, ThreadAudience::Unresolved(departed));
        assert_eq!(described.label(), "Unknown audience");
        assert!(
            !described.is_private_to_this_computer(),
            "an unanswerable question is not a yes"
        );
    }

    #[test]
    fn a_resolved_audience_carries_its_own_promise() {
        let roster = AudienceRoster::new([
            Audience::joined("forge:omega", "Omega development").expect("a joined audience")
        ]);

        let local = roster.describe(&AudienceId::local());
        assert_eq!(local.label(), "Local");
        assert_eq!(local.description(), LOCAL_DESCRIPTION);
        assert!(local.is_private_to_this_computer());

        let community =
            roster.describe(&AudienceId::joined("forge:omega").expect("a joined identity"));
        assert_eq!(community.label(), "Omega development");
        assert!(!community.is_private_to_this_computer());
    }

    /// omega#108's falsifier, held here so it cannot be restated there.
    #[test]
    fn a_local_thread_is_refused_before_it_can_publish() {
        let roster = AudienceRoster::new([
            Audience::joined("forge:omega", "Omega development").expect("a joined audience")
        ]);

        assert_eq!(
            may_publish(&roster.describe(&AudienceId::local())),
            Err(PublishRefused::ThreadIsLocal),
            "a local thread has nobody to publish to, and that is what local means"
        );
        assert_eq!(
            may_publish(
                &roster.describe(&AudienceId::joined("forge:absent").expect("an identity"))
            ),
            Err(PublishRefused::AudienceUnresolved(
                AudienceId::joined("forge:absent").expect("an identity")
            )),
            "publishing into a place Omega cannot describe is publishing to an \
             audience nobody can name"
        );
        assert_eq!(
            may_publish(&roster.describe(&AudienceId::joined("forge:omega").expect("an identity")))
                .expect("a joined audience may publish")
                .name(),
            "Omega development"
        );
    }

    /// A record has to survive a restart, or it is inferred after one.
    #[test]
    fn a_book_survives_the_round_trip_through_storage() {
        let mut book = AudienceBook::new();
        book.bind("thread-a".to_string(), AudienceId::local())
            .expect("bound");
        book.bind(
            "thread-b".to_string(),
            AudienceId::joined("forge:omega").expect("a joined identity"),
        )
        .expect("bound");

        let encoded = serde_json::to_string(&book).expect("a book encodes");
        let decoded: AudienceBook<String> = serde_json::from_str(&encoded).expect("a book decodes");

        assert_eq!(decoded, book);
        assert_eq!(
            decoded.recorded(&"thread-b".to_string()),
            Some(&AudienceId::joined("forge:omega").expect("a joined identity"))
        );
    }

    #[test]
    fn an_unknown_stored_key_survives_the_round_trip_rather_than_becoming_local() {
        let id = AudienceId::from_key("forge:something-this-build-never-joined");

        assert!(!id.is_local());
        assert_eq!(id.as_key(), "forge:something-this-build-never-joined");
        assert!(AudienceId::from_key("local").is_local());
    }
}
