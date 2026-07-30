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
use std::{
    collections::{BTreeMap, HashSet},
    rc::Rc,
    time::Duration,
};

use db::kvp::KeyValueStore;
use gpui::{App, AppContext as _, Global, Task, TaskExt as _};
use nostr::{Event, JsonUtil as _, PublicKey};
use omega_audience::Audience;
use omega_community::{
    AuthorizedMessage, COMMAND_HELP, Command, ForgeRepository, Invitation, JoinOutcome,
    JoinedRooms, Outbox, QueueOutcome, RelayOutcome, RoomPresence, SignedRecord, binding_of,
    parse_command,
};
use omega_identity::{
    AdmittedSigningRequest, DurableIdentityActionDecision, DurableIdentityActionDescriptor,
    DurableIdentityActionKind, IdentityService, ProofRef, PublicIdentity, ReceiptRef, ResourceRef,
    SigningPurpose, UnsignedEventTemplate,
};
use sha2::{Digest as _, Sha256};
use util::ResultExt as _;

use crate::account_scope::AccountScope;
use crate::omega_audience_control::{forget_roster, thread_audience};
use crate::thread_metadata_store::ThreadId;

/// Where the joined rooms live in the key-value store.
const NAMESPACE: &str = "omega_community";

/// The key holding every room this profile has joined.
const ROOMS_KEY: &str = "joined_rooms";
const OUTBOX_KEY: &str = "outbox";
const RECORDS_KEY: &str = "verified_records";
const MAX_RECORDS_PER_ROOM: usize = 128;
const MAX_RECORD_CONTENT_BYTES: usize = 64 * 1024;
const REFRESH_INTERVAL_SECONDS: u64 = 30;

/// The rooms this profile is in, hydrated from the key-value store once.
#[derive(Default)]
struct OmegaCommunity {
    scope: Option<AccountScope>,
    /// `None` until the first read, so a launch that never opens a thread pays
    /// nothing for a feature nobody on this machine has joined.
    rooms: Option<Rc<JoinedRooms>>,
    outbox: Option<Rc<Outbox>>,
    records: Option<Rc<BTreeMap<String, Vec<Event>>>>,
    deliveries: HashSet<String>,
    refreshes: HashSet<String>,
    refreshed_at: BTreeMap<String, u64>,
    identity: Option<Option<PublicIdentity>>,
}

impl Global for OmegaCommunity {}

pub fn purge_account(identity: &PublicIdentity, cx: &App) -> Task<anyhow::Result<()>> {
    let store = KeyValueStore::global(cx);
    let namespace = AccountScope::namespace_for_identity(NAMESPACE, identity);
    cx.background_spawn(async move {
        let scoped = store.scoped(&namespace);
        scoped.delete_all().await?;
        anyhow::ensure!(
            scoped.read(ROOMS_KEY)?.is_none() && scoped.read(RECORDS_KEY)?.is_none(),
            "the account community records remained after purge"
        );
        Ok(())
    })
}

pub fn purge_room_state(identity: &PublicIdentity, cx: &App) -> Task<anyhow::Result<()>> {
    let community = purge_account(identity, cx);
    let audience = crate::omega_audience_control::purge_account(identity, cx);
    cx.background_spawn(async move {
        community.await?;
        audience.await
    })
}

fn account_scope(cx: &mut App) -> AccountScope {
    let scope = AccountScope::observed();
    let state = cx.default_global::<OmegaCommunity>();
    if state.scope.as_ref() != Some(&scope) {
        state.scope = Some(scope.clone());
        state.rooms = None;
        state.outbox = None;
        state.records = None;
        state.deliveries.clear();
        state.refreshes.clear();
        state.refreshed_at.clear();
        state.identity = None;
    }
    scope
}

fn read_scoped_or_migrate(
    scope: &AccountScope,
    key: &'static str,
    target_key: String,
    cx: &App,
) -> Option<String> {
    let store = KeyValueStore::global(cx);
    let namespace = scope.namespace(NAMESPACE);
    if let Some(value) = store
        .scoped(&namespace)
        .read(&target_key)
        .log_err()
        .flatten()
    {
        return Some(value);
    }
    let value = store.scoped(NAMESPACE).read(key).log_err().flatten()?;
    let migration_store = store;
    let migration_scope = scope.clone();
    let migration_value = value.clone();
    cx.background_spawn(async move {
        migration_scope.ensure_current()?;
        let target = migration_store.scoped(&namespace);
        target
            .write(target_key.clone(), migration_value.clone())
            .await?;
        if let Err(stale) = migration_scope.ensure_current() {
            if migration_scope.is_purge_barrier_active()? {
                target.delete_all().await?;
            }
            return Err(stale);
        }
        anyhow::ensure!(
            target.read(&target_key)?.as_deref() == Some(migration_value.as_str()),
            "the migrated community value could not be read back"
        );
        migration_store
            .scoped(NAMESPACE)
            .delete(key.to_string())
            .await
    })
    .detach_and_log_err(cx);
    Some(value)
}

fn rooms(cx: &mut App) -> Rc<JoinedRooms> {
    let scope = account_scope(cx);
    if let Some(rooms) = cx.default_global::<OmegaCommunity>().rooms.clone() {
        return rooms;
    }

    let stored: JoinedRooms =
        read_scoped_or_migrate(&scope, ROOMS_KEY, scope.profile_key(ROOMS_KEY), cx)
            .and_then(|raw| serde_json::from_str(&raw).log_err())
            .unwrap_or_default();

    let stored = Rc::new(stored);
    cx.default_global::<OmegaCommunity>().rooms = Some(stored.clone());
    stored
}

fn persist_value<T: serde::Serialize>(
    scope: AccountScope,
    key: &'static str,
    pending: bool,
    value: &T,
    cx: &App,
) {
    let store = KeyValueStore::global(cx);
    let namespace = scope.namespace(NAMESPACE);
    let key = if pending {
        scope.pending_key(key)
    } else {
        scope.profile_key(key)
    };
    let Some(payload) = serde_json::to_string(value).log_err() else {
        return;
    };
    cx.background_spawn(async move {
        scope.ensure_current()?;
        store.scoped(&namespace).write(key, payload).await?;
        if let Err(stale) = scope.ensure_current() {
            if scope.is_purge_barrier_active()? {
                store.scoped(&namespace).delete_all().await?;
            }
            return Err(stale);
        }
        Ok(())
    })
    .detach_and_log_err(cx);
}

fn outbox(cx: &mut App) -> Rc<Outbox> {
    let scope = account_scope(cx);
    if let Some(outbox) = cx.default_global::<OmegaCommunity>().outbox.clone() {
        return outbox;
    }

    let stored: Outbox =
        read_scoped_or_migrate(&scope, OUTBOX_KEY, scope.pending_key(OUTBOX_KEY), cx)
            .and_then(|raw| serde_json::from_str(&raw).log_err())
            .unwrap_or_default();
    let stored = Rc::new(stored);
    cx.default_global::<OmegaCommunity>().outbox = Some(stored.clone());
    stored
}

fn records(cx: &mut App) -> Rc<BTreeMap<String, Vec<Event>>> {
    let scope = account_scope(cx);
    if let Some(records) = cx.default_global::<OmegaCommunity>().records.clone() {
        return records;
    }

    let stored: BTreeMap<String, Vec<Event>> =
        read_scoped_or_migrate(&scope, RECORDS_KEY, scope.profile_key(RECORDS_KEY), cx)
            .and_then(|raw| serde_json::from_str(&raw).log_err())
            .unwrap_or_default();
    let stored = Rc::new(stored);
    cx.default_global::<OmegaCommunity>().records = Some(stored.clone());
    stored
}

fn identity(cx: &mut App) -> Option<PublicIdentity> {
    let scope = account_scope(cx);
    if let Some(identity) = cx.default_global::<OmegaCommunity>().identity.clone() {
        return identity;
    }

    let identity = scope.identity().or_else(|| {
        IdentityService::system(*app_identity::CHANNEL)
            .inspect()
            .log_err()
            .and_then(|custody| custody.identity)
    });

    cx.default_global::<OmegaCommunity>().identity = Some(identity.clone());
    identity
}

fn author(cx: &mut App) -> Option<PublicKey> {
    identity(cx)
        .and_then(|identity| PublicKey::from_hex(identity.public_key_hex().as_str()).log_err())
}

/// Every community audience this profile has joined.
///
/// The seam `omega_audience_control` reads when it builds the roster. Local is
/// not in here — `AudienceRoster` puts it first by construction, and a second
/// source of it would be a second thing to keep right.
pub fn joined_audiences(cx: &mut App) -> Vec<Audience> {
    let rooms = rooms(cx);
    pump_pending(cx);
    for room in rooms.rooms() {
        refresh_room(room.repository.clone(), cx);
    }
    rooms
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

    pump_pending(cx);
    let mut lines = Vec::new();
    for room in rooms.rooms() {
        refresh_room(room.repository.clone(), cx);
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
    let outbox = outbox(cx);
    if !outbox.is_empty() {
        lines.push("\nMessages:".to_string());
        lines.extend(outbox.sorted().map(|entry| {
            format!(
                "{} — {}",
                short_event_id(&entry.event.id.to_hex()),
                entry.delivery.label()
            )
        }));
    }
    let records = records(cx);
    let visible = records.values().map(Vec::len).sum::<usize>();
    lines.push(format!(
        "\n{visible} verified room message{} cached. Relay refresh is running in the background.",
        if visible == 1 { "" } else { "s" }
    ));
    for room in rooms.rooms() {
        let Some(messages) = records.get(&room.repository.coordinate().to_string()) else {
            continue;
        };
        if !messages.is_empty() {
            lines.push(format!("\nRecent messages in {}:", room.name()));
        }
        lines.extend(messages.iter().rev().take(5).rev().map(|event| {
            format!(
                "{}: {}",
                short_event_id(&event.pubkey.to_hex()),
                event.content.trim()
            )
        }));
    }
    lines.join("\n")
}

fn join(invitation: Invitation, cx: &mut App) -> String {
    let invitation_text = match invitation.to_text() {
        Ok(invitation_text) => invitation_text,
        Err(error) => return format!("Not joined. {error}"),
    };
    if let Err(refusal) = require_active_identity_action(
        DurableIdentityActionKind::CommunityJoin,
        invitation.descriptor.coordinate.to_string(),
        invitation_text.as_bytes(),
    ) {
        return format!("Not joined. {refusal}");
    }
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
            persist_value(account_scope(cx), ROOMS_KEY, false, &*rooms, cx);
            // The roster is rebuilt from the rooms, so the composer's cached
            // copy has to be dropped or the room a person just joined does not
            // appear until the next launch.
            forget_roster(cx);
            for room in rooms.rooms() {
                refresh_room(room.repository.clone(), cx);
            }

            let opening = match outcome {
                JoinOutcome::Refreshed => {
                    format!("Your membership of {} was updated ({roles}).", report.name)
                }
                _ => format!("You joined {} ({roles}).", report.name),
            };
            format!(
                "{opening} {what_you_may_do}\n\nIt is in the composer's audience selector. \
                 Choosing it there changes the audience of the next thread you start, not this \
                 one. Relay messages are signed with your Omega identity and synchronized in the \
                 background."
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
    persist_value(account_scope(cx), ROOMS_KEY, false, &*rooms, cx);
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

    refresh_room(joined.repository.clone(), cx);
    let records = records(cx);
    let observed: Vec<SignedRecord> = records
        .get(&joined.repository.coordinate().to_string())
        .into_iter()
        .flatten()
        .filter_map(|event| SignedRecord::verify_received(&joined.repository, event.clone()).ok())
        .collect();
    let description =
        RoomPresence::observed(&joined.repository, &joined.membership, author, &observed)
            .describe();
    format!("{description}\n\nOmega is refreshing this room from the relay in the background.")
}

fn post(thread_id: ThreadId, text: &str, cx: &mut App) -> String {
    let audience = thread_audience(thread_id, cx);
    let Some(identity) = identity(cx) else {
        return NO_KEY_YET.to_string();
    };
    let Some(author) = PublicKey::from_hex(identity.public_key_hex().as_str()).log_err() else {
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
    let event_id = unsigned.id();
    let event = unsigned.event();
    let request_ref = match ReceiptRef::new(format!("omega.community.{}", &event_id.to_hex()[..32]))
    {
        Ok(request_ref) => request_ref,
        Err(error) => return format!("Not sent. The signing request was invalid: {error}"),
    };
    if let Err(refusal) = require_active_identity_action(
        DurableIdentityActionKind::PublicPost,
        joined.repository.coordinate().to_string(),
        event_id.as_bytes(),
    ) {
        return format!("Not sent. {refusal}");
    }
    let signed =
        match IdentityService::system(*app_identity::CHANNEL).sign(&AdmittedSigningRequest {
            request_ref,
            identity_ref: identity.identity_ref().clone(),
            purpose: SigningPurpose::NostrEvent,
            event: UnsignedEventTemplate {
                created_at: event.created_at.as_secs(),
                kind: event.kind.as_u16(),
                tags: event
                    .tags
                    .iter()
                    .map(|tag| tag.as_slice().to_vec())
                    .collect(),
                content: event.content.clone(),
            },
        }) {
            Ok(signed) => signed,
            Err(error) => return format!("Not sent. Omega could not sign the message: {error}"),
        };
    let signed = match Event::from_json(signed.signed_event_json)
        .map_err(|error| error.to_string())
        .and_then(|event| {
            unsigned
                .accept_signature(event)
                .map_err(|error| error.to_string())
        }) {
        Ok(signed) => signed,
        Err(error) => return format!("Not sent. The signed message was refused: {error}"),
    };

    let mut outbox = outbox(cx);
    let outcome = Rc::make_mut(&mut outbox).queue(&signed, now());
    cx.default_global::<OmegaCommunity>().outbox = Some(outbox.clone());
    // omega#164. A signed community post is the first kind of value a
    // background-created identity accrues, so it arms the quiet backup nudge.
    // Fail-soft: the durable record is a nudge input, never a send blocker.
    cx.background_spawn(async {
        if let Err(error) = IdentityService::system(*app_identity::CHANNEL)
            .record_backup_value_accrued(omega_identity::BackupValueKind::ChannelPost)
        {
            log::warn!("could not record identity backup value accrual: {error}");
        }
    })
    .detach();
    cache_verified_record(&joined.repository, signed.event().clone(), cx);
    start_delivery(
        signed.id(),
        signed.event().clone(),
        joined.repository.relays().to_vec(),
        cx,
    );

    match outcome {
        QueueOutcome::Queued => format!(
            "Queued {} for {}. It is signed by your Omega identity and will remain visible here \
             until the relay acknowledges it or bounded retries stop.",
            short_event_id(&signed.id().to_hex()),
            joined.name()
        ),
        QueueOutcome::AlreadyQueued => format!(
            "{} was already queued. Omega will send the same signed bytes rather than create a \
             duplicate.",
            short_event_id(&signed.id().to_hex())
        ),
    }
}

fn require_active_identity_action(
    kind: DurableIdentityActionKind,
    destination: String,
    payload: &[u8],
) -> Result<(), String> {
    require_active_identity_action_with(
        &IdentityService::system(*app_identity::CHANNEL),
        kind,
        destination,
        payload,
        now(),
    )
}

fn require_active_identity_action_with(
    identity_service: &IdentityService,
    kind: DurableIdentityActionKind,
    destination: String,
    payload: &[u8],
    issued_at: u64,
) -> Result<(), String> {
    let payload_digest = format!("{:x}", Sha256::digest(payload));
    let destination_digest = format!("{:x}", Sha256::digest(destination.as_bytes()));
    let intent_ref = ReceiptRef::new(format!("omega-community-action-{}", &payload_digest[..32]))
        .map_err(|error| format!("the activation intent is invalid: {error}"))?;
    let descriptor = DurableIdentityActionDescriptor {
        authorization_ref: ProofRef::new(format!("activation-{}", intent_ref.as_str()))
            .map_err(|error| format!("the activation authorization is invalid: {error}"))?,
        intent_ref,
        kind,
        destination_ref: ResourceRef::new(format!("community-{destination_digest}"))
            .map_err(|error| format!("the activation destination is invalid: {error}"))?,
        payload_digest,
        expires_at: issued_at
            .checked_add(600)
            .ok_or_else(|| "the activation window overflowed".to_string())?,
    };
    match identity_service
        .authorize_or_hold_identity_action(descriptor)
        .map_err(|error| format!("Omega identity is unavailable: {error}"))?
    {
        DurableIdentityActionDecision::Authorized(_) => Ok(()),
        DurableIdentityActionDecision::ActivationRequired { account, .. } => Err(format!(
            "Set up identity {} from Omega Identity before this action.",
            account.fingerprint_display()
        )),
    }
}

fn short_event_id(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}

fn cache_verified_record(repository: &ForgeRepository, event: Event, cx: &mut App) {
    cache_verified_records(repository, [event], cx);
}

fn cache_verified_records(
    repository: &ForgeRepository,
    events: impl IntoIterator<Item = Event>,
    cx: &mut App,
) {
    let coordinate = repository.coordinate().to_string();
    let mut records = records(cx);
    let room_records = Rc::make_mut(&mut records).entry(coordinate).or_default();
    let mut changed = false;
    for event in events {
        if event.content.len() > MAX_RECORD_CONTENT_BYTES
            || SignedRecord::verify_received(repository, event.clone()).is_err()
            || room_records.iter().any(|known| known.id == event.id)
        {
            continue;
        }
        room_records.push(event);
        changed = true;
    }
    if !changed {
        return;
    }
    room_records.sort_by_key(|event| (event.created_at, event.id));
    if room_records.len() > MAX_RECORDS_PER_ROOM {
        room_records.drain(..room_records.len() - MAX_RECORDS_PER_ROOM);
    }
    cx.default_global::<OmegaCommunity>().records = Some(records.clone());
    persist_value(account_scope(cx), RECORDS_KEY, false, &*records, cx);
}

fn refresh_room(repository: ForgeRepository, cx: &mut App) {
    let scope = account_scope(cx);
    let coordinate = repository.coordinate().to_string();
    let state = cx.default_global::<OmegaCommunity>();
    if state.refreshes.contains(&coordinate)
        || state
            .refreshed_at
            .get(&coordinate)
            .is_some_and(|last| now().saturating_sub(*last) < REFRESH_INTERVAL_SECONDS)
    {
        return;
    }
    state.refreshes.insert(coordinate.clone());
    state.refreshed_at.insert(coordinate.clone(), now());
    let relay_urls = repository.relays().to_vec();
    let query_coordinate = coordinate.clone();
    let query_scope = scope.clone();
    let query = cx.background_spawn(async move {
        let events = omega_effectd::query_community_events(relay_urls, &query_coordinate)?;
        query_scope.ensure_current()?;
        Ok::<_, anyhow::Error>(events)
    });
    cx.spawn(async move |cx| -> anyhow::Result<()> {
        let result = query.await;
        cx.update(|cx| {
            if cx.default_global::<OmegaCommunity>().scope.as_ref() != Some(&scope) {
                return;
            }
            cx.default_global::<OmegaCommunity>()
                .refreshes
                .remove(&coordinate);
            match result {
                Ok(events) => {
                    cache_verified_records(&repository, events, cx);
                    cx.refresh_windows();
                }
                Err(error) => {
                    log::warn!(
                        "OMEGA-WS-02: community relay refresh for {coordinate} failed: {error}"
                    );
                }
            }
        });
        Ok(())
    })
    .detach_and_log_err(cx);
}

fn start_delivery(event_id: nostr::EventId, event: Event, relay_urls: Vec<String>, cx: &mut App) {
    let scope = account_scope(cx);
    let event_id_hex = event_id.to_hex();
    if !cx
        .default_global::<OmegaCommunity>()
        .deliveries
        .insert(event_id_hex.clone())
    {
        return;
    }
    let initial_payload = match serde_json::to_string(&*outbox(cx)) {
        Ok(payload) => payload,
        Err(error) => {
            cx.default_global::<OmegaCommunity>()
                .deliveries
                .remove(&event_id_hex);
            log::error!("OMEGA-WS-02: could not persist queued event {event_id_hex}: {error}");
            return;
        }
    };
    let store = KeyValueStore::global(cx);
    let namespace = scope.namespace(NAMESPACE);
    let outbox_key = scope.pending_key(OUTBOX_KEY);

    cx.spawn(async move |cx| -> anyhow::Result<()> {
        let initial_store = store.clone();
        let initial_scope = scope.clone();
        let initial_key = outbox_key.clone();
        let initial_namespace = namespace.clone();
        if let Err(error) = cx
            .background_spawn(async move {
                initial_scope.ensure_current()?;
                initial_store
                    .scoped(&initial_namespace)
                    .write(initial_key, initial_payload)
                    .await?;
                if let Err(stale) = initial_scope.ensure_current() {
                    if initial_scope.is_purge_barrier_active()? {
                        initial_store
                            .scoped(&initial_namespace)
                            .delete_all()
                            .await?;
                    }
                    return Err(stale);
                }
                Ok(())
            })
            .await
        {
            cx.update(|cx| {
                if cx.default_global::<OmegaCommunity>().scope.as_ref() == Some(&scope) {
                    cx.default_global::<OmegaCommunity>()
                        .deliveries
                        .remove(&event_id_hex);
                }
            });
            return Err(error);
        }
        let mut backoff = Duration::from_secs(1);
        loop {
            let attempt_event = event.clone();
            let attempt_relays = relay_urls.clone();
            let attempt_scope = scope.clone();
            let result = cx
                .background_spawn(async move {
                    attempt_scope.ensure_current()?;
                    omega_effectd::publish_community_event(attempt_relays, &attempt_event)
                        .map_err(anyhow::Error::from)
                })
                .await;
            let outcome = match result {
                Ok(()) => RelayOutcome::Accepted,
                Err(error) => RelayOutcome::Unreachable {
                    message: error.to_string(),
                },
            };
            let updated = cx.update(|cx| {
                if cx.default_global::<OmegaCommunity>().scope.as_ref() != Some(&scope) {
                    return None;
                }
                let mut outbox = outbox(cx);
                let outbox_mut = Rc::make_mut(&mut outbox);
                let should_retry = match outbox_mut.record_attempt(event_id, &outcome, now()) {
                    Ok(entry) => entry.delivery.is_pending(),
                    Err(error) => {
                        log::error!(
                            "OMEGA-WS-02: delivery result for {event_id_hex} had no outbox entry: \
                             {error}"
                        );
                        false
                    }
                };
                cx.default_global::<OmegaCommunity>().outbox = Some(outbox.clone());
                cx.refresh_windows();
                Some((should_retry, serde_json::to_string(&*outbox)))
            });
            let Some((should_retry, payload)) = updated else {
                return Ok(());
            };
            let payload = match payload {
                Ok(payload) => payload,
                Err(error) => {
                    cx.update(|cx| {
                        cx.default_global::<OmegaCommunity>()
                            .deliveries
                            .remove(&event_id_hex);
                    });
                    return Err(error.into());
                }
            };
            let attempt_store = store.clone();
            let persist_scope = scope.clone();
            let persist_key = outbox_key.clone();
            let persist_namespace = namespace.clone();
            if let Err(error) = cx
                .background_spawn(async move {
                    persist_scope.ensure_current()?;
                    attempt_store
                        .scoped(&persist_namespace)
                        .write(persist_key, payload)
                        .await?;
                    if let Err(stale) = persist_scope.ensure_current() {
                        if persist_scope.is_purge_barrier_active()? {
                            attempt_store
                                .scoped(&persist_namespace)
                                .delete_all()
                                .await?;
                        }
                        return Err(stale);
                    }
                    Ok(())
                })
                .await
            {
                cx.update(|cx| {
                    if cx.default_global::<OmegaCommunity>().scope.as_ref() == Some(&scope) {
                        cx.default_global::<OmegaCommunity>()
                            .deliveries
                            .remove(&event_id_hex);
                    }
                });
                return Err(error);
            }
            if !should_retry {
                break;
            }
            cx.background_executor().timer(backoff).await;
            backoff = backoff.saturating_mul(2).min(Duration::from_secs(8));
        }
        cx.update(|cx| {
            if cx.default_global::<OmegaCommunity>().scope.as_ref() == Some(&scope) {
                cx.default_global::<OmegaCommunity>()
                    .deliveries
                    .remove(&event_id_hex);
            }
        });
        Ok(())
    })
    .detach_and_log_err(cx);
}

fn pump_pending(cx: &mut App) {
    let pending: Vec<Event> = outbox(cx)
        .pending()
        .map(|entry| entry.event.clone())
        .collect();
    let rooms = rooms(cx);
    for event in pending {
        let Ok(coordinate) = binding_of(&event) else {
            log::error!(
                "OMEGA-WS-02: an outbox event has no valid repository binding; refusing to send it"
            );
            continue;
        };
        let relay_urls = rooms
            .rooms()
            .find(|room| room.repository.coordinate() == &coordinate)
            .map(|room| room.repository.relays().to_vec());
        let Some(relay_urls) = relay_urls else {
            log::warn!(
                "OMEGA-WS-02: queued event {} belongs to a room this profile has left",
                event.id
            );
            continue;
        };
        start_delivery(event.id, event, relay_urls, cx);
    }
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

const WHO_NEEDS_A_ROOM: &str = "This thread is not in a community workspace, so there is nobody to list. Open a thread in \
     one and ask again.";

const LEAVE_NEEDS_A_ROOM: &str =
    "This thread is not in a community workspace, so there is nothing to leave.";

const NO_KEY_YET: &str = "Omega does not have your key yet, so it cannot say who you are in a room. Nothing about a \
     community workspace works before that, and it is not something Omega can decide for you.";

#[cfg(test)]
mod tests {
    use app_identity::AppChannel;
    use omega_identity::IdentityActivationState;

    use super::*;

    #[test]
    fn refusals_are_complete_sentences() {
        for sentence in [WHO_NEEDS_A_ROOM, LEAVE_NEEDS_A_ROOM, NO_KEY_YET] {
            assert!(
                !sentence.is_empty() && sentence.ends_with('.'),
                "a refusal a person reads is a sentence"
            );
        }
    }

    #[test]
    fn event_ids_are_shortened_without_panicking() {
        assert_eq!(short_event_id("0123456789abcdef"), "0123456789ab");
        assert_eq!(short_event_id("short"), "short");
    }

    #[test]
    fn candidate_community_action_is_held_before_room_or_network_mutation() {
        let directory = tempfile::tempdir().expect("identity directory");
        let service =
            IdentityService::for_channel_data_root(AppChannel::Dev, directory.path().to_path_buf());
        service
            .create(ReceiptRef::new("community-candidate-create").expect("create receipt"))
            .expect("create candidate identity");

        let refusal = require_active_identity_action_with(
            &service,
            DurableIdentityActionKind::CommunityJoin,
            "forge:tenant.openagents/vortex".to_string(),
            b"canonical invitation",
            now(),
        )
        .expect_err("candidate join must be held");
        assert!(refusal.contains("Set up identity"));
        assert_eq!(
            service.inspect_account().expect("candidate account").state,
            IdentityActivationState::Activating
        );
    }
}
