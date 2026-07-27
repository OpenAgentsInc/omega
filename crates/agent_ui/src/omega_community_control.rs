//! The community room, operated from the conversation. `OMEGA-DELTA-0113`,
//! omega#108.
//!
//! `omega_community` holds the room's rules and cannot open a socket or read a
//! key. This is the edge that calls it: the durable record of what this profile
//! has joined, the one place a key is read, and the answers a person gets back
//! when they type an instruction into the chat.
//!
//! # Why the rooms live here and the roster is built from them
//!
//! [`joined_audiences`] is the seam `omega_audience_control` reads, and it is
//! derived from [`omega_community::JoinedRooms`] on every rebuild rather than
//! stored beside it. omega#107 is explicit that the roster is what the composer
//! offers; a second list of "audiences" kept in step with the rooms by hand is
//! the defect where a person leaves a room and the selector still offers it.
//!
//! # Why there is no settings page
//!
//! omega#108 deliverable 3: "the owner's requirement is that it is all operable
//! through the Omega Agent conversation rather than a separate administrative
//! surface. Joining, seeing who is present, and posting are conversation
//! actions." So the whole control surface is [`run`] — a line a person types —
//! and the grammar it accepts is `omega_community::parse_command`, which lives
//! in the crate that can be tested without a window.
//!
//! # What is honestly not here yet
//!
//! Nothing in this tree connects to a relay, and nothing here signs. So
//! `\/community post` runs the full authorization — the audience rule, the room
//! match, and the Forge's roles — composes the exact bytes, and then says that
//! there is nothing wired to sign and send them. That is the state of the tree,
//! and the alternative shapes are both worse: a message that reports "sent" and
//! went nowhere, or a verb the surface refuses to admit exists.
//!
//! The refusals, though, are real today, and they are omega#108's own
//! falsifiers: posting from a local thread is refused, posting into a room this
//! thread does not belong to is refused, and a `forge:viewer` is refused by the
//! Forge's own role.

use std::rc::Rc;

use db::kvp::KeyValueStore;
use gpui::{App, AppContext as _, Global, TaskExt as _};
use nostr::PublicKey;
use omega_audience::Audience;
use omega_community::{
    AuthorizedMessage, COMMAND_HELP, Command, Invitation, JoinOutcome, JoinedRooms, RoomPresence,
    parse_command,
};
use util::ResultExt as _;

use crate::omega_audience_control::{forget_roster, thread_audience};
use crate::thread_metadata_store::ThreadId;

/// Where the joined rooms live in the key-value store.
const NAMESPACE: &str = "omega_community";

/// The key holding every room this profile has joined.
const ROOMS_KEY: &str = "joined_rooms";

/// The rooms this profile is in, hydrated from the key-value store once.
#[derive(Default)]
struct OmegaCommunity {
    /// `None` until the first read, so a launch that never opens a thread pays
    /// nothing for a feature nobody on this machine has joined.
    rooms: Option<Rc<JoinedRooms>>,
    /// This profile's own key, and whether it has been looked for.
    ///
    /// Read through `omega_identity`, which touches the disk. Cached because
    /// the composer asks on every draw and a person's key does not change
    /// between two frames.
    author: Option<Option<PublicKey>>,
}

impl Global for OmegaCommunity {}

fn rooms(cx: &mut App) -> Rc<JoinedRooms> {
    if let Some(rooms) = cx.default_global::<OmegaCommunity>().rooms.clone() {
        return rooms;
    }

    let stored: JoinedRooms = KeyValueStore::global(cx)
        .scoped(NAMESPACE)
        .read(ROOMS_KEY)
        .log_err()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).log_err())
        .unwrap_or_default();

    let stored = Rc::new(stored);
    cx.default_global::<OmegaCommunity>().rooms = Some(stored.clone());
    stored
}

fn persist(rooms: &JoinedRooms, cx: &App) {
    let store = KeyValueStore::global(cx);
    let Some(payload) = serde_json::to_string(rooms).log_err() else {
        return;
    };
    cx.background_spawn(async move {
        store
            .scoped(NAMESPACE)
            .write(ROOMS_KEY.to_string(), payload)
            .await
    })
    .detach_and_log_err(cx);
}

/// This profile's own Nostr key, if custody has one.
///
/// The one place in this feature that reads a key, and it reads the *public*
/// half only. omega#108 deliverable 4 is that a person's identity is theirs;
/// `omega_community` names no key type outside its tests precisely so that the
/// reading happens here, where a check can see it.
///
/// `None` on a machine whose identity is not ready, which is a state to name
/// rather than an error: nothing about the room works without knowing who is
/// asking, and saying so is more useful than an empty list.
fn author(cx: &mut App) -> Option<PublicKey> {
    if let Some(author) = cx.default_global::<OmegaCommunity>().author {
        return author;
    }

    let author = omega_identity::IdentityService::system(*app_identity::CHANNEL)
        .inspect()
        .log_err()
        .and_then(|custody| custody.identity)
        .and_then(|identity| PublicKey::from_hex(identity.public_key_hex().as_str()).log_err());

    cx.default_global::<OmegaCommunity>().author = Some(author);
    author
}

/// Every community audience this profile has joined.
///
/// The seam `omega_audience_control` reads when it builds the roster. Local is
/// not in here — `AudienceRoster` puts it first by construction, and a second
/// source of it would be a second thing to keep right.
pub fn joined_audiences(cx: &mut App) -> Vec<Audience> {
    rooms(cx)
        .rooms()
        .filter_map(|room| room.repository.audience().log_err())
        .collect()
}

/// Runs a line typed into the conversation, if it was addressed to the room.
///
/// `None` means the line was not an instruction about the room, which is almost
/// every line, and the caller should send it on untouched. `Some` is the answer
/// a person reads.
pub fn run(thread_id: ThreadId, line: &str, cx: &mut App) -> Option<String> {
    let command = match parse_command(line)? {
        Ok(command) => command,
        Err(refusal) => return Some(refusal.to_string()),
    };

    Some(match command {
        Command::Status => status(cx),
        Command::Join(invitation) => join(*invitation, cx),
        Command::Leave => leave(thread_id, cx),
        Command::Who => who(thread_id, cx),
        Command::Post(text) => post(thread_id, &text, cx),
    })
}

/// The seconds since the epoch, as the room's rules take them.
///
/// A clock reading, which is why it is here and not in `omega_community`: every
/// rule in that crate takes the time as a parameter so it can be tested on a
/// machine that is not at the right one.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn status(cx: &mut App) -> String {
    let rooms = rooms(cx);
    if rooms.is_empty() {
        return format!(
            "You have not joined a community workspace. Local is the only audience, and it \
             works with no account, no relay and no network.\n\n{COMMAND_HELP}"
        );
    }

    let mut lines = Vec::new();
    for room in rooms.rooms() {
        let roles: Vec<String> = room
            .membership
            .role_refs
            .iter()
            .cloned()
            .map(String::from)
            .collect();
        lines.push(format!(
            "{} — {}, as the Forge answered at {}.",
            room.name(),
            if roles.is_empty() {
                "no role".to_string()
            } else {
                roles.join(", ")
            },
            room.membership_as_of()
        ));
    }
    lines.push(NOTHING_IS_WIRED_TO_SEND.to_string());
    lines.join("\n")
}

fn join(invitation: Invitation, cx: &mut App) -> String {
    let mut rooms = rooms(cx);
    let report = match Rc::make_mut(&mut rooms).join(invitation, now()) {
        Ok(report) => report,
        Err(refusal) => return format!("Not joined. {refusal}"),
    };

    let roles = if report.roles.is_empty() {
        "no role".to_string()
    } else {
        report.roles.join(", ")
    };
    let what_you_may_do = if report.may_write {
        "You can read it and write in it."
    } else {
        "You can read it. The Forge did not grant you a role that writes."
    };

    match report.outcome {
        JoinOutcome::AlreadyJoined => {
            format!("You are already in {} ({roles}).", report.name)
        }
        outcome => {
            cx.default_global::<OmegaCommunity>().rooms = Some(rooms.clone());
            persist(&rooms, cx);
            // The roster is rebuilt from the rooms, so the composer's cached
            // copy has to be dropped or the room a person just joined does not
            // appear until the next launch.
            forget_roster(cx);

            let opening = match outcome {
                JoinOutcome::Refreshed => format!(
                    "Your membership of {} was updated ({roles}).",
                    report.name
                ),
                _ => format!("You joined {} ({roles}).", report.name),
            };
            format!(
                "{opening} {what_you_may_do}\n\nIt is in the composer's audience selector. \
                 Choosing it there changes the audience of the next thread you start, not this \
                 one.\n{NOTHING_IS_WIRED_TO_SEND}"
            )
        }
    }
}

fn leave(thread_id: ThreadId, cx: &mut App) -> String {
    let Some(room) = current_room(thread_id, cx) else {
        return LEAVE_NEEDS_A_ROOM.to_string();
    };

    let mut rooms = rooms(cx);
    if !Rc::make_mut(&mut rooms).leave(&room.audience) {
        return LEAVE_NEEDS_A_ROOM.to_string();
    }

    cx.default_global::<OmegaCommunity>().rooms = Some(rooms.clone());
    persist(&rooms, cx);
    forget_roster(cx);

    format!(
        "You left {}. The threads you held there keep the audience they were recorded in, and \
         now read as an audience this profile cannot resolve — they were never private, and \
         Omega will not start saying they were.",
        room.name
    )
}

fn who(thread_id: ThreadId, cx: &mut App) -> String {
    let Some(room) = current_room(thread_id, cx) else {
        return WHO_NEEDS_A_ROOM.to_string();
    };
    let Some(author) = author(cx) else {
        return NO_KEY_YET.to_string();
    };

    let rooms = rooms(cx);
    let Some(joined) = rooms.room(&room.audience) else {
        return WHO_NEEDS_A_ROOM.to_string();
    };

    // No records, because nothing reads from a relay yet. The answer is
    // therefore this profile alone, and `RoomPresence::describe` says on what
    // basis — which is the sentence that stops it being read as a member list.
    RoomPresence::observed(&joined.repository, &joined.membership, author, []).describe()
}

fn post(thread_id: ThreadId, text: &str, cx: &mut App) -> String {
    let audience = thread_audience(thread_id, cx);
    let Some(author) = author(cx) else {
        return NO_KEY_YET.to_string();
    };

    let rooms = rooms(cx);
    // The room a *message* goes to is the one this thread belongs to, never the
    // one selected in the composer. omega#107's whole point: the selection
    // decides where the next thread starts, and a thread's own audience is the
    // fact recorded on it.
    let Some(joined) = current_room(thread_id, cx).and_then(|room| rooms.room(&room.audience))
    else {
        // Not a refusal this module composes. `AuthorizedMessage::prepare`
        // would refuse a local thread too, but it needs a room to refuse it
        // *against*, and there is none — so the sentence is about the thread.
        return format!(
            "This thread's audience is {}, so there is nothing to send it to. Start a thread in \
             a community workspace and post there.",
            audience.label()
        );
    };

    let _early = joined.repository.coordinate();
    let message = match AuthorizedMessage::prepare(
        &joined.repository,
        &joined.membership,
        &audience,
        author,
        text,
    ) {
        Ok(message) => message,
        Err(refusal) => return format!("Not sent. {refusal}"),
    };

    let mut unsigned = message.into_unsigned(now());
    format!(
        "Authorized, and composed as {} — the audience allowed it, the room matched, and the \
         Forge's roles admit you.\n\n{NOTHING_IS_WIRED_TO_SEND}",
        unsigned.id().to_hex()
    )
}

/// The room a thread belongs to, if this profile is in it.
struct CurrentRoom {
    audience: omega_audience::AudienceId,
    name: String,
}

fn current_room(thread_id: ThreadId, cx: &mut App) -> Option<CurrentRoom> {
    match thread_audience(thread_id, cx) {
        omega_audience::ThreadAudience::Known(audience) if !audience.is_local() => {
            Some(CurrentRoom {
                audience: audience.id().clone(),
                name: audience.name().to_string(),
            })
        }
        _ => None,
    }
}

/// Said once, because it is the one thing every answer here has to be honest
/// about and a second wording would eventually be a softer one.
const NOTHING_IS_WIRED_TO_SEND: &str =
    "Nothing in this build signs or reaches a relay yet, so a message is authorized and composed \
     and then goes no further. It is not queued and it is not lost — there is nowhere for it to \
     go, and Omega will not report a send that did not happen.";

const WHO_NEEDS_A_ROOM: &str =
    "This thread is not in a community workspace, so there is nobody to list. Open a thread in \
     one and ask again.";

const LEAVE_NEEDS_A_ROOM: &str =
    "This thread is not in a community workspace, so there is nothing to leave.";

const NO_KEY_YET: &str =
    "Omega does not have your key yet, so it cannot say who you are in a room. Nothing about a \
     community workspace works before that, and it is not something Omega can decide for you.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Every sentence this module can produce, so a wording change is a diff a
    /// reader can see rather than something noticed in a window.
    #[test]
    fn the_sentences_say_what_is_true_of_this_build() {
        assert!(
            NOTHING_IS_WIRED_TO_SEND.contains("goes no further"),
            "the one thing every answer has to be honest about is that nothing \
             is sent"
        );
        assert!(
            NOTHING_IS_WIRED_TO_SEND.contains("not queued")
                && NOTHING_IS_WIRED_TO_SEND.contains("not lost"),
            "a message that was never signed is not in the outbox, so it is \
             neither pending nor lost, and both halves have to be said or the \
             other one is assumed"
        );
        for sentence in [WHO_NEEDS_A_ROOM, LEAVE_NEEDS_A_ROOM, NO_KEY_YET] {
            assert!(
                !sentence.is_empty() && sentence.ends_with('.'),
                "a refusal a person reads is a sentence"
            );
        }
    }
}
