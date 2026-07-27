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
//!
//! # The three sentences nobody has watched anybody read
//!
//! [`SELECTION_MENU_HEADER`], [`SWITCHING_DOES_NOT_MOVE_A_THREAD`] and
//! [`THREAD_IS_NOT_IN_THE_SELECTION`] are the whole of what the menu says, and
//! they are the least verified thing in this feature. They are here, beside
//! the rules they describe, rather than inline in the render closure, because
//! the lane that wrote them named them as its own biggest guess and the next
//! person should be able to change them in one edit rather than by hunting
//! through a menu builder.
//!
//! **The guess.** A person picks a different audience, the menu closes, and
//! the button still reads what it read before — because choosing applies to
//! the next thread and this thread keeps the one it was started in. That is
//! [`AudienceBook::bind`]'s refusal, and it is correct. But *correct* and
//! *legible* are different properties: the same pixels are what a broken
//! dropdown looks like. These two lines are the only thing standing between
//! the two readings.
//!
//! **What would falsify them.** Somebody who has never seen this before picks
//! the second entry, and then says the control did not work, or picks it a
//! second time, or goes looking for a setting. Any of those means the sentence
//! did not land and the shape has to change — a confirmation on the button, a
//! different tense, or the choice applying visibly somewhere the eye is
//! already on. No check in this repository can see that happen; it needs a
//! window and a person.
//!
//! **What is checked.** Only that the three sentences exist in exactly one
//! place — `the_menus_sentences_are_written_once` in `omega_deltas` fails if a
//! literal reappears in the control — so that changing them stays one edit.
//!
//! # The row this control sits in
//!
//! `render_zero_base_executor_bar` is a `flex_wrap` row already carrying the
//! executor disclosure, a turn-phase dot, a provider notice, the model
//! selector and Send. The audience button was added to its left-hand group,
//! which does *not* wrap internally and whose other text — the
//! `OMEGA-DELTA-0021` executor disclosure — is `.truncate()`d. So an unbounded
//! audience name does not merely look wide: it takes room from the one label
//! in that row that is allowed to shrink, and the label it takes room from is
//! the mandatory attribution of which executor ran the turn.
//!
//! An audience name is not a value this repository chooses. omega#108's names
//! come from a Forge repository, and `OMEGA_AUDIENCE_PREVIEW` puts an
//! arbitrary environment string on the button today. So [`ThreadAudience::label`]
//! bounds what it returns to [`MAX_LABEL_CHARS`], and the full name stays in
//! the menu entry and the tooltip, which have room for it.
//!
//! [`MAX_LABEL_CHARS`] is 24 because it has to clear the names that will
//! really occur — `Omega development` is 17, `OpenAgentsInc/omega` is 19,
//! `Unknown audience` is 16 — while making the worst case a fixed one somebody
//! can look at on purpose rather than an unbounded one nobody will ever see.
//! Whether 24 characters plus the disclosure plus the model selector actually
//! fits a narrow dock is a rendered fact, and this bound does not decide it —
//! it only makes the question answerable once instead of per name.

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

/// The header the selection menu opens with.
///
/// One of the three sentences described under "The three sentences nobody
/// has watched anybody read" in the module comment.
pub const SELECTION_MENU_HEADER: &str = "New threads start in";

/// The line the selection menu ends on, always.
///
/// One of the three sentences described under "The three sentences nobody
/// has watched anybody read" in the module comment.
pub const SWITCHING_DOES_NOT_MOVE_A_THREAD: &str = "A thread keeps the audience it was started in.";

/// The second line, shown only when the thread on screen is in a different
/// audience from the one the next thread will start in.
///
/// One of the three sentences described under "The three sentences nobody
/// has watched anybody read" in the module comment.
pub const THREAD_IS_NOT_IN_THE_SELECTION: &str = "This thread is not in the selected audience.";

/// The longest audience name the composer will write on the control's face.
///
/// See "The row this control sits in" in the module comment for why there is
/// a bound at all, and why it is this number.
pub const MAX_LABEL_CHARS: usize = 24;

/// What a bounded name ends with, so a shortened one is visibly shortened.
pub const ELLIPSIS: char = '…';

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

/// The prefix that marks an identity as a rendering fixture and not a place.
///
/// Reserved in both directions, exactly as [`LOCAL_KEY`] is:
/// [`AudienceId::preview`] produces it and [`AudienceId::joined`] refuses it,
/// so omega#108's Forge coordinates cannot land on one and a fixture cannot be
/// minted by the path a real membership arrives through.
///
/// Reservation is what turns "cannot collide" from a claim into a mechanism.
/// `OMEGA-DELTA-0094` already asserted the fixture's identity "cannot be
/// mistaken for a Forge coordinate" on the strength of the prefix alone, which
/// is a naming convention — and a convention is a thing the code that has to
/// respect it has never heard of.
///
/// The comparison is exact rather than case-folded, for the same reason
/// [`LOCAL_KEY`]'s is. `Preview:x` is a *different* identity from `preview:x`,
/// so nothing can resolve one as the other; the only reader it could mislead
/// is a person, and a person reading identities is not the failure this
/// prevents.
pub const PREVIEW_PREFIX: &str = "preview:";

/// The identity of the one fixture audience.
///
/// It names itself. A key a person might find in their key-value store, or in
/// a thread record that outlived the environment variable, should say what it
/// is without anybody having to look it up.
pub const PREVIEW_KEY: &str = "preview:not-a-real-audience";

/// What the fixture is called when the environment asks for it by presence
/// rather than by name.
pub const PREVIEW_NAME: &str = "Preview audience";

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
    /// The key began with [`PREVIEW_PREFIX`], which only
    /// [`AudienceId::preview`] may produce.
    ///
    /// A joined audience wearing the fixture prefix would be refused at
    /// [`may_publish`] as though it were a fixture, so a real membership would
    /// silently stop being able to publish. The prefix means one thing.
    ReservedPreviewPrefix,
}

impl fmt::Display for AudienceIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("an audience identifier cannot be empty"),
            Self::ReservedLocalKey => {
                formatter.write_str("`local` is reserved for the audience that is always present")
            }
            Self::ReservedPreviewPrefix => formatter
                .write_str("`preview:` is reserved for the rendering fixture and names no place"),
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

    /// The identity of the rendering fixture.
    ///
    /// The only constructor that may produce a [`PREVIEW_PREFIX`] key, so a
    /// fixture is a thing this crate mints and never a thing that arrives.
    #[must_use]
    pub fn preview() -> Self {
        Self(PREVIEW_KEY.to_string())
    }

    /// An identifier for an audience somebody joined.
    ///
    /// # Errors
    ///
    /// [`AudienceIdError::Empty`] for a blank key,
    /// [`AudienceIdError::ReservedLocalKey`] for an attempt to take the local
    /// audience's name, and [`AudienceIdError::ReservedPreviewPrefix`] for an
    /// attempt to take the fixture's prefix.
    pub fn joined(key: impl Into<String>) -> Result<Self, AudienceIdError> {
        let key = key.into();
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err(AudienceIdError::Empty);
        }
        if trimmed == LOCAL_KEY {
            return Err(AudienceIdError::ReservedLocalKey);
        }
        if trimmed.starts_with(PREVIEW_PREFIX) {
            return Err(AudienceIdError::ReservedPreviewPrefix);
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Is this the audience that is always present?
    #[must_use]
    pub fn is_local(&self) -> bool {
        self.0 == LOCAL_KEY
    }

    /// Is this a rendering fixture rather than a place?
    ///
    /// Read off the prefix rather than off equality with [`PREVIEW_KEY`], so a
    /// second fixture added later is a fixture too and does not have to be
    /// remembered at [`may_publish`].
    #[must_use]
    pub fn is_preview(&self) -> bool {
        self.0.starts_with(PREVIEW_PREFIX)
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

    /// The rendering fixture, so more than one audience can be looked at
    /// before omega#108 exists.
    ///
    /// [`Reach::Shared`] on purpose. The fixture exists to make the *not
    /// private* case observable, and an entry that claimed to be private would
    /// make the two rendered acceptance items it was added for meaningless.
    /// What keeps that honest is that [`may_publish`] refuses it by identity —
    /// see [`PublishRefused::AudienceIsAFixture`] — so "not private" here never
    /// becomes "may be published to".
    #[must_use]
    pub fn preview(name: impl Into<String>) -> Self {
        Self {
            id: AudienceId::preview(),
            name: name.into(),
            reach: Reach::Shared,
        }
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

    /// Is this a rendering fixture rather than a place?
    #[must_use]
    pub fn is_preview(&self) -> bool {
        self.id.is_preview()
    }

    /// The full sentence, for a tooltip or a menu line.
    ///
    /// The local one is fixed text because it is a promise this crate keeps.
    /// A joined one describes its reach rather than its membership, because
    /// membership is omega#108's to answer and a guess here would be a
    /// confident wrong answer about who can read something.
    ///
    /// The fixture says what it is. "Shared with everyone in Preview
    /// audience. Needs a network." is the sentence a real audience makes, and
    /// it is false of this one in both halves: there is nobody in it and it
    /// reaches no network. A fixture that describes itself as a place is the
    /// one way this fixture could mislead somebody.
    #[must_use]
    pub fn description(&self) -> String {
        if self.is_local() {
            LOCAL_DESCRIPTION.to_string()
        } else if self.is_preview() {
            format!(
                "A rendering fixture, not a place. Nothing is published, joined or sent. \
                 Present because {PREVIEW_ENV_VAR} is set."
            )
        } else {
            format!("Shared with everyone in {}. Needs a network.", self.name)
        }
    }
}

/// The environment variable that asks for the rendering fixture.
///
/// Named here rather than only in `agent_ui` so the sentence the fixture makes
/// about itself can say which switch produced it. Reading the variable is
/// still the control's job — this crate reads no ambient state.
pub const PREVIEW_ENV_VAR: &str = "OMEGA_AUDIENCE_PREVIEW";

/// The fixture audience an environment value asks for, if it asks for one.
///
/// Takes the value rather than reading it, so the rule is testable on a
/// machine that is not in the right state — the discipline the whole crate
/// keeps. `agent_ui` supplies `std::env::var(PREVIEW_ENV_VAR).ok()`.
///
/// - Absent, empty, and `0` produce nothing. Absent is every machine that has
///   not deliberately asked, and `0` is what somebody writes when they mean to
///   turn it off.
/// - `1` means "on, do not make me name it" and produces [`PREVIEW_NAME`].
/// - Anything else is the name, so the long-name case in the composer row can
///   be looked at on purpose.
///
/// The result is never local, never the selection, and never publishable. It
/// is an extra roster entry and nothing else.
#[must_use]
pub fn preview_audience(requested: Option<&str>) -> Option<Audience> {
    let requested = requested?;
    let requested = requested.trim();
    if requested.is_empty() || requested == "0" {
        return None;
    }
    let name = if requested == "1" {
        PREVIEW_NAME
    } else {
        requested
    };
    Some(Audience::preview(name))
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

/// A name bounded to [`MAX_LABEL_CHARS`], counted in characters.
///
/// Characters rather than bytes, because a byte slice of a name in any script
/// that is not ASCII either panics on a boundary or produces mojibake, and an
/// audience name is somebody else's text.
#[must_use]
fn bounded(name: &str) -> String {
    if name.chars().count() <= MAX_LABEL_CHARS {
        return name.to_string();
    }
    let mut bounded: String = name.chars().take(MAX_LABEL_CHARS - 1).collect();
    bounded.push(ELLIPSIS);
    bounded
}

impl ThreadAudience {
    /// What a person reads in the composer, bounded to [`MAX_LABEL_CHARS`].
    ///
    /// The bound is here rather than at the call site because this is the only
    /// function that produces the face of the control, and a length rule
    /// applied where a string is drawn is a rule the next drawing site does not
    /// have. See "The row this control sits in" in the module comment.
    ///
    /// The menu entry and the tooltip use [`Audience::name`] and
    /// [`Audience::description`], which are unbounded on purpose: they have the
    /// room, and a name a person cannot read in full anywhere is a name they
    /// cannot check.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Known(audience) => bounded(audience.name()),
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
    /// The thread was started in a rendering fixture, which is not a place.
    ///
    /// The fixture carries [`Reach::Shared`] because it exists to make the
    /// not-private case observable, and that made it indistinguishable here
    /// from a real audience: `may_publish` is the one gate omega#108 is told to
    /// perform *before* an effect, and it returned `Ok` for the fixture.
    /// Nothing publishes today, so nothing left any machine — but the first
    /// transport wired behind this gate would have been authorised, on any
    /// machine with [`PREVIEW_ENV_VAR`] set, to publish into an audience that
    /// does not exist.
    AudienceIsAFixture(AudienceId),
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
            Self::AudienceIsAFixture(id) => write!(
                formatter,
                "`{}` is a rendering fixture, not a place. There is nobody in it to publish to.",
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
/// The fixture is refused before its reach is consulted, because its reach is
/// `Shared` and reach alone cannot tell a place from a stand-in for one.
///
/// # Errors
///
/// [`PublishRefused`], which is a sentence a person can read.
pub fn may_publish(audience: &ThreadAudience) -> Result<&Audience, PublishRefused> {
    match audience {
        ThreadAudience::Known(audience) if audience.is_preview() => {
            Err(PublishRefused::AudienceIsAFixture(audience.id().clone()))
        }
        ThreadAudience::Known(audience) if audience.reach() == Reach::Shared => Ok(audience),
        ThreadAudience::Known(_) => Err(PublishRefused::ThreadIsLocal),
        ThreadAudience::Unresolved(id) if id.is_preview() => {
            Err(PublishRefused::AudienceIsAFixture(id.clone()))
        }
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

    // ------------------------------------------------------------------
    // OMEGA-DELTA-0105. What could be proved without a window, proved.
    // ------------------------------------------------------------------

    /// omega#107 acceptance 1, composed rather than asserted in pieces.
    ///
    /// A profile that has joined nothing, has recorded nothing, and has
    /// selected nothing is the state of every machine this ships to. Every
    /// value the composer can read in that state is checked here, so the only
    /// thing left for a window is whether the word appears where somebody sees
    /// it.
    ///
    /// Written to reach the *unbound* branch on purpose. The lane before this
    /// one found that all fifteen of its tests bound a thread first, so the
    /// fallback that runs on every thread on a fresh machine had nothing
    /// holding it. This test binds nothing.
    #[test]
    fn a_fresh_profile_reads_local_and_nothing_else() {
        let book: AudienceBook<&str> = AudienceBook::default();
        let roster = AudienceRoster::default();

        assert!(book.is_empty(), "a fresh profile has recorded nothing");
        assert!(!roster.has_joined_anything());

        // No selection was ever written, so the composer's selection is the
        // constructed local one. `audience_for_opening` is what the control
        // calls, and on a fresh profile both of its inputs are local.
        let selected = AudienceId::local();
        assert_eq!(
            audience_for_opening(None, &selected, ThreadOpening::Started),
            AudienceId::local()
        );

        // And what the control writes on its own face, for a thread it has
        // never seen, without anything being bound.
        let described = roster.describe(&book.audience_of(&"a-thread-nothing-recorded"));
        assert_eq!(described.label(), LOCAL_NAME);
        assert_eq!(described.description(), LOCAL_DESCRIPTION);
        assert!(described.is_private_to_this_computer());
        assert_eq!(
            may_publish(&described),
            Err(PublishRefused::ThreadIsLocal),
            "the audience a fresh profile is in has nowhere to publish to"
        );
    }

    /// The bound on the face of the control.
    ///
    /// An audience name is somebody else's text: omega#108's come from a Forge
    /// repository and the fixture's comes from an environment variable. The
    /// row it is drawn in gives its only other label — the `OMEGA-DELTA-0021`
    /// executor disclosure — a `.truncate()`, so an unbounded name takes room
    /// from the mandatory attribution of which executor ran the turn.
    #[test]
    fn the_composer_label_is_bounded_however_long_the_name_is() {
        let long = "A community audience with a name far longer than any row";
        let roster =
            AudienceRoster::new([Audience::joined("forge:long", long).expect("a joined audience")]);
        let described = roster.describe(&AudienceId::joined("forge:long").expect("an identity"));

        let label = described.label();
        assert!(
            label.chars().count() <= MAX_LABEL_CHARS,
            "the control's face must be bounded, got {} characters: {label:?}",
            label.chars().count()
        );
        assert!(
            label.ends_with(ELLIPSIS),
            "a shortened name must look shortened, or a person reads a \
             different audience's name as this one's: {label:?}"
        );
        assert!(
            described.description().contains(long),
            "the tooltip has the room, and a name nobody can read in full \
             anywhere is a name nobody can check"
        );

        // The boundary, both sides. A name that fits is untouched.
        let exact: String = "x".repeat(MAX_LABEL_CHARS);
        assert_eq!(bounded(&exact), exact);
        assert_eq!(
            bounded(&"x".repeat(MAX_LABEL_CHARS + 1)).chars().count(),
            MAX_LABEL_CHARS
        );

        // Somebody else's text is not ASCII. Counting bytes here panics.
        let cyrillic = "Сообщество разработчиков Омеги".to_string();
        assert!(cyrillic.chars().count() > MAX_LABEL_CHARS);
        assert_eq!(bounded(&cyrillic).chars().count(), MAX_LABEL_CHARS);
    }

    /// The reserved prefix, refused at the door a real membership comes
    /// through.
    ///
    /// `OMEGA-DELTA-0094` recorded that the fixture's identity "cannot be
    /// mistaken for a Forge coordinate" because it is `preview:` prefixed.
    /// That was a naming convention, and a convention is a thing the code that
    /// has to respect it has never heard of. This makes it a mechanism.
    #[test]
    fn no_forge_coordinate_can_wear_the_fixture_prefix() {
        assert_eq!(
            AudienceId::joined(PREVIEW_KEY),
            Err(AudienceIdError::ReservedPreviewPrefix)
        );
        assert_eq!(
            AudienceId::joined("preview:anything-at-all"),
            Err(AudienceIdError::ReservedPreviewPrefix)
        );
        assert_eq!(
            AudienceId::joined("  preview:padded  "),
            Err(AudienceIdError::ReservedPreviewPrefix),
            "whitespace is not a different prefix"
        );

        // The coordinates omega#108 will actually mint are unaffected, and so
        // is a name that merely contains the word.
        assert!(AudienceId::joined("forge:OpenAgentsInc/omega").is_ok());
        assert!(AudienceId::joined("naddr1preview:not-a-prefix").is_ok());
        assert!(AudienceId::joined("a-preview:audience").is_ok());

        // And the fixture is still recognisable in a record that outlived the
        // environment variable, because reading a stored key does not
        // validate.
        assert!(AudienceId::from_key(PREVIEW_KEY).is_preview());
        assert!(!AudienceId::local().is_preview());
        assert!(
            !AudienceId::joined("forge:omega")
                .expect("an identity")
                .is_preview()
        );
    }

    /// The fixture is absent unless somebody asked for it, in as many ways as
    /// somebody can decline.
    #[test]
    fn the_fixture_is_absent_unless_the_environment_asks_for_it() {
        for declined in [None, Some(""), Some("   "), Some("0"), Some(" 0 ")] {
            assert_eq!(
                preview_audience(declined),
                None,
                "{declined:?} must not put a second audience in the roster"
            );
        }

        let by_presence = preview_audience(Some("1")).expect("`1` asks for the fixture");
        assert_eq!(by_presence.name(), PREVIEW_NAME);

        let by_name = preview_audience(Some("Omega development")).expect("a named fixture");
        assert_eq!(by_name.name(), "Omega development");
        assert_eq!(
            by_name.id(),
            by_presence.id(),
            "naming the fixture must not mint a second identity"
        );
    }

    /// What the fixture is, checked on the fixture itself rather than on the
    /// sentence `OMEGA_DELTAS.md` writes about it.
    ///
    /// Three claims were recorded against it and none were held by anything:
    /// that it publishes nothing, that it cannot become the default, and that
    /// its identity cannot collide with omega#108's.
    #[test]
    fn the_fixture_is_not_a_place_and_cannot_be_published_to() {
        let fixture = preview_audience(Some("1")).expect("the fixture");
        let roster = AudienceRoster::new([fixture.clone()]);

        // Not the default. It is the second entry of a roster whose first is
        // always the constructed local one, and it reaches nobody by being
        // present — only by being chosen.
        assert_eq!(roster.len(), 2);
        assert!(
            roster
                .entries()
                .next()
                .expect("a roster always has a first entry")
                .is_local()
        );
        assert_eq!(
            audience_for_opening(None, &AudienceId::local(), ThreadOpening::Started),
            AudienceId::local(),
            "a fixture in the roster must not change what a fresh profile \
             starts threads in"
        );

        // Publishes nothing, at the gate omega#108 is told to ask before an
        // effect — not merely because no transport is wired yet. The fixture
        // is `Reach::Shared`, so reach alone answered this wrongly.
        let described = roster.describe(fixture.id());
        assert!(
            !described.is_private_to_this_computer(),
            "the fixture exists to make the not-private case observable"
        );
        assert_eq!(
            may_publish(&described),
            Err(PublishRefused::AudienceIsAFixture(AudienceId::preview())),
            "a fixture is not a place, and `may_publish` is the one question \
             asked before an effect"
        );

        // Including through a record that outlived the environment variable,
        // where the roster can no longer resolve it.
        assert_eq!(
            may_publish(&AudienceRoster::default().describe(&AudienceId::preview())),
            Err(PublishRefused::AudienceIsAFixture(AudienceId::preview()))
        );

        // And it says what it is, rather than describing a membership.
        assert!(fixture.description().contains("not a place"));
        assert!(fixture.description().contains(PREVIEW_ENV_VAR));
        assert!(
            !fixture.description().contains("Shared with everyone in"),
            "a fixture that describes itself as a place is the one way it can \
             mislead somebody"
        );
    }

    /// The three sentences the menu says, in one place.
    ///
    /// The lane that wrote them called them its own biggest guess. Nothing can
    /// check whether they land — that needs a window and a person — so what is
    /// checked is that changing them is one edit.
    #[test]
    fn the_menus_sentences_are_here_and_say_what_they_mean() {
        assert!(SWITCHING_DOES_NOT_MOVE_A_THREAD.contains("keeps the audience it was started in"));
        assert!(THREAD_IS_NOT_IN_THE_SELECTION.contains("not in the selected audience"));
        assert!(SELECTION_MENU_HEADER.contains("New threads"));
        for sentence in [
            SELECTION_MENU_HEADER,
            SWITCHING_DOES_NOT_MOVE_A_THREAD,
            THREAD_IS_NOT_IN_THE_SELECTION,
        ] {
            assert!(!sentence.trim().is_empty());
        }

        // The rule the first sentence describes is the one `bind` enforces, so
        // if the refusal were ever dropped the sentence would become a false
        // statement rather than an unread one.
        let mut book = AudienceBook::new();
        book.bind("a", AudienceId::local()).expect("bound");
        assert!(
            book.bind("a", AudienceId::joined("forge:omega").expect("an identity"))
                .is_err(),
            "the menu says a thread keeps its audience; something has to make \
             that true"
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
