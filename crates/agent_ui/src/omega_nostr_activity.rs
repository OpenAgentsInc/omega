//! The public NIP-29 group, read: the last few messages, and nothing else.
//!
//! `OMEGA-DELTA-0130`. The owner asked the sidebar to carry "nostr nip 29
//! activity too", and then said which activity: *"for initial nip29 shit i want
//! it showing the most recent 5 messages from the default channel, the one we
//! show at /agentchat in apps/openagents.com of openagents repo"*.
//!
//! That page is `apps/openagents.com/apps/start/src/routes/-public-nostr-chat-page.tsx`
//! in the `openagents` repository. It reads one relay, one group, one kind:
//! `wss://relay.openagents.com`, `openagents-public`, kind `9` with an `h` tag.
//! This module reads the same three things and draws five rows.
//!
//! # This is the first socket Omega opens to a relay from the UI
//!
//! `omega_community` says of itself that it "does not open a socket or touch a
//! key", and [`crate::omega_community_control`] still tells a person that
//! "nothing in this build signs or reaches a relay yet". Both remain true of
//! *writing*. This reads, and it reads the one thing that needs no key at all:
//! the manifest at `openagents.com` declares `auth.directRead: "public"`, and
//! the relay serves the group's history to an unauthenticated connection. So
//! there is no signer here, no secret key of any encoding, no NIP-42 exchange,
//! and no publish path. The whole capability is one `REQ` and the frames that
//! answer it.
//!
//! # What is not done, said plainly rather than implied
//!
//! - **No display names.** The web page issues a second `REQ` for each author's
//!   kind `0` and prefers `display_name`. This shows the same fallback that
//!   page shows while that request is outstanding — `abcdef01…89abcdef` — and
//!   makes no second request. A name is a nicety; five rows that arrive is the
//!   thing that was asked for.
//! - **No live subscription.** One connection, one `REQ`, frames until `EOSE`,
//!   then the socket closes. The sidebar re-reads when it is opened, not
//!   continuously. A held-open socket is a lifecycle to get wrong, and getting
//!   it wrong in a panel that must never interrupt is worse than being a minute
//!   stale.
//! - **No moderation state.** Deletions (kind `5`), reports (`1984`) and the
//!   relay's group-state events (`39000`-`39005`) are not read, so a message
//!   deleted after this window fetched it would still be drawn until the next
//!   fetch. This is stated rather than hidden because it is the one way these
//!   rows can be wrong about something a person would care about.
//!
//! Signatures **are** checked. `nostr::Event::verify` is already in this
//! binary's dependency graph, so serving a forged event through the relay is
//! not a way to put text in the owner's sidebar.
//!
//! # Why the parsing is here and the drawing is in the panel
//!
//! Everything above the socket is a function of bytes: a manifest is a config,
//! a relay frame is a frame, five rows out of a hundred events is a sort and a
//! truncation. None of it needs a window, and all of the parts that were
//! actually easy to get wrong — the relay's non-standard three-element `EOSE`,
//! the ordering, the deduplication — are answerable in a unit test.

use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, TimeZone as _, Utc};
use futures::{FutureExt as _, StreamExt as _};
use gpui::SharedString;
use serde_json::{Value, json};

/// Where the relay and group identifiers are published.
///
/// Read rather than compiled in. The built-in `public-nostr-chat` skill states
/// the rule this follows in as many words: "Do not put an OpenAgents host name
/// or group identifier in the protocol code." The URL of the manifest is the
/// one piece of configuration that has to be somewhere, and it is here.
pub const MANIFEST_URL: &str = "https://openagents.com/api/public/nostr-chat/manifest";

/// How many messages the section draws.
///
/// The owner's number: "the most recent 5 messages from the default channel".
pub const RECENT_MESSAGES: usize = 5;

/// The NIP-29 chat message kind. `h`-tagged, per the spec's "The `h` tag".
const CHAT_KIND: u64 = 9;

/// How long the whole read is allowed to take before the section gives up and
/// says so.
///
/// A sidebar section that hangs is a sidebar that hangs. Eight seconds is the
/// same bound `omega_effectd`'s relay adapter uses for a network round trip.
pub const READ_TIMEOUT: Duration = Duration::from_secs(8);

/// The relay and group this reads, as the manifest declares them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupConfig {
    /// `wss://…`, from `relay.websocketUrl`.
    pub relay_url: String,
    /// The NIP-29 group id, from `group.id`. This is the `h` tag's value.
    pub group_id: String,
}

/// One message, as the section shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    /// The event id. The deduplication key, and nothing else.
    pub id: String,
    /// The author's public key, lowercase hex.
    pub author: String,
    pub content: String,
    pub created_at: i64,
}

/// One drawn row: who, what, and how long ago.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityRow {
    /// `abcdef01…89abcdef`, the same fallback the web page shows for an author
    /// whose profile has not arrived.
    pub author: SharedString,
    /// The message text, as written. Never interpreted, never a link, never
    /// markdown: this is somebody else's text arriving over a public relay.
    pub content: SharedString,
    /// A compact age, the same shape the thread rows use.
    pub age: SharedString,
}

/// Pull the relay and the group out of the published manifest.
///
/// Only the two fields that are load-bearing are required. The manifest carries
/// fifteen more top-level keys, and a reader that demanded all of them would
/// break the next time one was added — which is the opposite of what reading a
/// manifest instead of hard-coding a host was for.
pub fn parse_manifest(json: &str) -> Result<GroupConfig> {
    let value: Value = serde_json::from_str(json).context("the manifest is not JSON")?;
    let relay_url = value
        .get("relay")
        .and_then(|relay| relay.get("websocketUrl"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("the manifest declares no `relay.websocketUrl`"))?;
    let group_id = value
        .get("group")
        .and_then(|group| group.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("the manifest declares no `group.id`"))?;
    if relay_url.is_empty() || group_id.is_empty() {
        return Err(anyhow!(
            "the manifest declares an empty relay or group identifier"
        ));
    }
    Ok(GroupConfig {
        relay_url: relay_url.to_string(),
        group_id: group_id.to_string(),
    })
}

/// The `REQ` frame this sends, as text.
///
/// The filter the manifest itself documents under `history.filter`, with the
/// owner's limit instead of the page's fifty.
#[must_use]
pub fn request_frame(subscription: &str, group_id: &str, limit: usize) -> String {
    json!([
        "REQ",
        subscription,
        { "kinds": [CHAT_KIND], "#h": [group_id], "limit": limit }
    ])
    .to_string()
}

/// A frame the relay sent, reduced to the three cases this cares about.
#[derive(Debug, PartialEq)]
pub enum Frame {
    /// An event, still as JSON. Verification happens in [`message`].
    Event(Value),
    /// The end of stored events for our subscription.
    Eose,
    /// A frame this read does not act on: `AUTH`, `NOTICE`, `OK`, `CLOSED`, an
    /// event for a subscription that is not ours, or anything unrecognised.
    ///
    /// `AUTH` is deliberately in this list. The relay sends a challenge to
    /// every connection whether or not it will require one, and this read needs
    /// no key, so the honest response to being offered authentication we do not
    /// need is to carry on reading.
    Ignored,
}

/// Read one relay frame.
///
/// # The three-element `EOSE`
///
/// NIP-01 defines `["EOSE", <subscription>]`. This relay sends
/// `["EOSE", <subscription>, ["more"]]`. A parser that matched on the array's
/// length would treat the end of history as an unknown frame and then wait for
/// an `EOSE` that had already been and gone, until the timeout — a section that
/// takes eight seconds to draw five rows it already had. So position is read
/// and length is not.
#[must_use]
pub fn read_frame(text: &str, subscription: &str) -> Frame {
    let Ok(Value::Array(frame)) = serde_json::from_str::<Value>(text) else {
        return Frame::Ignored;
    };
    let Some(label) = frame.first().and_then(Value::as_str) else {
        return Frame::Ignored;
    };
    // Every frame this acts on names our subscription in position 1. One that
    // names another is another reader's; there is only one subscription on this
    // socket today, and checking is what keeps that true if a second is added.
    let named_ours = frame.get(1).and_then(Value::as_str) == Some(subscription);
    match label {
        "EVENT" if named_ours => match frame.get(2) {
            Some(event) => Frame::Event(event.clone()),
            None => Frame::Ignored,
        },
        "EOSE" if named_ours => Frame::Eose,
        _ => Frame::Ignored,
    }
}

/// Turn a verified kind-9 event for this group into a message, or refuse it.
///
/// Four things must hold, and each one is a way somebody else's text could
/// otherwise reach the owner's window:
///
/// 1. The signature is the author's. Otherwise a relay could put words in
///    anybody's mouth, and the pubkey drawn beside them would be the lie.
/// 2. The kind is `9`. A `REQ` asks; it does not compel.
/// 3. The `h` tag is this group. Same reason.
/// 4. The content is not empty, once trimmed. A blank row says nothing and
///    takes the place of a message that would have.
#[must_use]
pub fn message(event: &Value, group_id: &str) -> Option<ChatMessage> {
    if event.get("kind").and_then(Value::as_u64) != Some(CHAT_KIND) {
        return None;
    }
    if !tags(event).any(|tag| {
        tag.first().map(String::as_str) == Some("h")
            && tag.get(1).map(String::as_str) == Some(group_id)
    }) {
        return None;
    }
    let id = event.get("id").and_then(Value::as_str)?.to_string();
    let author = event.get("pubkey").and_then(Value::as_str)?.to_string();
    let created_at = event.get("created_at").and_then(Value::as_i64)?;
    let content = event.get("content").and_then(Value::as_str)?.trim();
    if content.is_empty() {
        return None;
    }
    if !is_authentic(event) {
        return None;
    }
    Some(ChatMessage {
        id,
        author,
        content: content.to_string(),
        created_at,
    })
}

/// Whether the event's signature is its author's.
///
/// Delegated to `nostr`, which is already in this binary because
/// `omega_effectd` and `omega_identity` need it. Writing a second Schnorr
/// verification here would be a second place for it to be wrong.
fn is_authentic(event: &Value) -> bool {
    use nostr::JsonUtil as _;
    nostr::Event::from_json(event.to_string()).is_ok_and(|event| event.verify().is_ok())
}

fn tags(event: &Value) -> impl Iterator<Item = Vec<String>> + '_ {
    event
        .get("tags")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|tag| {
            Some(
                tag.as_array()?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            )
        })
}

/// The rows, newest first, deduplicated, bounded.
///
/// A relay may send the same event twice across a reconnect, and the page this
/// mirrors deduplicates by event id for that reason. The tie-break on id is the
/// same one [`crate::omega_threads_sidebar::rows`] uses and for the same
/// reason: two messages written in the same second must not swap places between
/// renders.
#[must_use]
pub fn rows(messages: Vec<ChatMessage>, now: DateTime<Utc>, limit: usize) -> Vec<ActivityRow> {
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<ChatMessage> = messages
        .into_iter()
        .filter(|message| seen.insert(message.id.clone()))
        .collect();
    unique.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    unique
        .into_iter()
        .take(limit)
        .map(|message| ActivityRow {
            author: short_key(&message.author).into(),
            age: crate::omega_threads_sidebar::short_age(written_at(message.created_at), now)
                .into(),
            content: message.content.into(),
        })
        .collect()
}

/// A seconds-since-epoch stamp as a time.
///
/// A stamp outside the representable range reads as the epoch rather than
/// panicking. The age formatter clamps a future time to `now`, so a nonsense
/// stamp becomes a very old row instead of a crash in a sidebar.
fn written_at(created_at: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(created_at, 0)
        .single()
        .unwrap_or_else(|| DateTime::UNIX_EPOCH)
}

/// `abcdef01…89abcdef`, the same shape the web page falls back to.
///
/// Deliberately identical to that page's `shortKey`, including the single
/// ellipsis character, so the owner reading both surfaces sees one author
/// written one way. A key too short to elide is shown whole rather than
/// padded into a shape it does not have.
#[must_use]
pub fn short_key(pubkey: &str) -> String {
    if pubkey.len() <= 16 {
        return pubkey.to_string();
    }
    format!("{}…{}", &pubkey[..8], &pubkey[pubkey.len() - 8..])
}

/// Read the manifest, connect, ask, and close.
///
/// Returns the messages the relay served for this group. Every failure — no
/// network, a manifest that changed shape, a relay that refuses the connection,
/// a socket that goes quiet — arrives as an `Err` for the caller to draw as one
/// quiet line. Nothing here retries, blocks a render, or raises anything.
///
/// `sleep` is the caller's timer so this stays testable and so the timeout is
/// the application's executor rather than a second runtime inside a panel.
pub async fn fetch(
    config: GroupConfig,
    limit: usize,
    sleep: impl std::future::Future<Output = ()>,
) -> Result<Vec<ChatMessage>> {
    let subscription = "omega-sidebar-nip29";
    let (mut socket, _) = futures::select! {
        connected = Box::pin(async_tungstenite::async_std::connect_async(
            config.relay_url.as_str(),
        )).fuse() => connected.with_context(|| format!("connecting to {}", config.relay_url))?,
        () = Box::pin(sleep).fuse() => return Err(anyhow!(
            "{} did not answer within {} seconds",
            config.relay_url,
            READ_TIMEOUT.as_secs()
        )),
    };

    socket
        .send(async_tungstenite::tungstenite::Message::Text(
            request_frame(subscription, &config.group_id, limit).into(),
        ))
        .await
        .context("asking the relay for the group's recent messages")?;

    let mut messages = Vec::new();
    // Bounded by the relay's answer, not by trust in it: a relay that streamed
    // events forever would otherwise hold this task open until the timeout, and
    // the caller only ever draws `limit` of them.
    while messages.len() < limit {
        let Some(frame) = socket.next().await else {
            break;
        };
        let frame = frame.context("reading a frame from the relay")?;
        let async_tungstenite::tungstenite::Message::Text(text) = frame else {
            continue;
        };
        match read_frame(&text, subscription) {
            Frame::Event(event) => {
                if let Some(message) = message(&event, &config.group_id) {
                    messages.push(message);
                }
            }
            Frame::Eose => break,
            Frame::Ignored => {}
        }
    }

    // Best-effort. The read is done; a relay that will not accept a close frame
    // does not make the messages less true.
    let _ = socket.close(None).await;
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest's own shape, trimmed to what this reads plus enough of the
    /// rest to prove the reader tolerates it.
    const MANIFEST: &str = r#"{
        "acceptedKinds": [5, 7, 9, 1337, 1984],
        "auth": { "directRead": "public" },
        "group": { "id": "openagents-public", "requiredTag": ["h", "openagents-public"] },
        "relay": { "websocketUrl": "wss://relay.openagents.com" },
        "readiness": "ready"
    }"#;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).single().expect("a time")
    }

    #[test]
    fn the_manifest_supplies_the_relay_and_the_group() {
        let config = parse_manifest(MANIFEST).expect("a manifest this shape parses");
        assert_eq!(config.relay_url, "wss://relay.openagents.com");
        assert_eq!(config.group_id, "openagents-public");
    }

    #[test]
    fn a_manifest_missing_what_this_needs_is_an_error_not_a_default() {
        for missing in [
            r#"{"group": {"id": "openagents-public"}}"#,
            r#"{"relay": {"websocketUrl": "wss://relay.openagents.com"}}"#,
            r#"{"relay": {"websocketUrl": ""}, "group": {"id": "openagents-public"}}"#,
            "not json at all",
        ] {
            assert!(
                parse_manifest(missing).is_err(),
                "a manifest without a usable relay and group must refuse rather \
                 than fall back to a compiled-in host: {missing}"
            );
        }
    }

    #[test]
    fn the_request_asks_for_this_group_and_no_more_than_asked() {
        let frame = request_frame("sub", "openagents-public", 5);
        let parsed: Value = serde_json::from_str(&frame).expect("a JSON frame");
        assert_eq!(parsed[0], "REQ");
        assert_eq!(parsed[1], "sub");
        assert_eq!(parsed[2]["kinds"], json!([9]));
        assert_eq!(parsed[2]["#h"], json!(["openagents-public"]));
        assert_eq!(parsed[2]["limit"], json!(5));
    }

    /// The defect this relay would actually have caused.
    #[test]
    fn the_relays_three_element_eose_still_ends_the_read() {
        assert_eq!(
            read_frame(r#"["EOSE","sub",["more"]]"#, "sub"),
            Frame::Eose,
            "this relay sends a third element NIP-01 does not define. A reader \
             that missed it would wait out the whole timeout holding rows it \
             already had."
        );
        assert_eq!(read_frame(r#"["EOSE","sub"]"#, "sub"), Frame::Eose);
    }

    #[test]
    fn an_auth_challenge_is_read_past_rather_than_answered() {
        assert_eq!(
            read_frame(r#"["AUTH","2f0c…"]"#, "sub"),
            Frame::Ignored,
            "the relay challenges every connection and requires no answer for \
             a read. Treating the challenge as a stopping point would make the \
             section permanently empty on a relay that is serving it fine."
        );
    }

    #[test]
    fn a_frame_for_somebody_elses_subscription_is_not_ours() {
        assert_eq!(
            read_frame(r#"["EVENT","other",{"kind":9}]"#, "sub"),
            Frame::Ignored
        );
        assert_eq!(read_frame(r#"["EOSE","other"]"#, "sub"), Frame::Ignored);
        assert!(matches!(
            read_frame(r#"["EVENT","sub",{"kind":9}]"#, "sub"),
            Frame::Event(_)
        ));
    }

    #[test]
    fn junk_on_the_socket_is_ignored_rather_than_fatal() {
        for junk in ["", "{}", "[]", "[123]", r#"["EVENT","sub"]"#, "<html>"] {
            assert_eq!(read_frame(junk, "sub"), Frame::Ignored, "{junk}");
        }
    }

    /// A real event this relay served, with its real signature. Verbatim, so
    /// the verification below is exercised against bytes that actually verify
    /// rather than against a fixture written to pass.
    const REAL_EVENT: &str = r#"{"kind":9,"id":"928297eed4e7eaec67a0ff451a026139f1c11afd40ab9ac8ecf26a147f079e2e","pubkey":"66277e0bcc147cd1ae116f64ba45eb2f7021db7159eeffdc5083112fa0fbb7e4","created_at":1785104056,"tags":[["h","openagents-public"],["previous","0c0a90e8"]],"content":"Testing NIP-29 message","sig":"fccac70134cf9165a2a38948e5061df1fce18e858a5939f70b126b350e9551b982389941b7b346a63c3ecc92b7bf81f79917851976fd21b6c8761ec52216e242"}"#;

    fn real_event() -> Value {
        serde_json::from_str(REAL_EVENT).expect("the captured event is JSON")
    }

    #[test]
    fn a_real_signed_message_for_this_group_is_accepted() {
        let message = message(&real_event(), "openagents-public")
            .expect("a real, correctly signed kind-9 event for this group");
        assert_eq!(message.content, "Testing NIP-29 message");
        assert_eq!(
            message.author,
            "66277e0bcc147cd1ae116f64ba45eb2f7021db7159eeffdc5083112fa0fbb7e4"
        );
        assert_eq!(message.created_at, 1785104056);
    }

    #[test]
    fn a_tampered_message_is_refused() {
        let mut forged = real_event();
        forged["content"] = json!("Testing NIP-29 message, plus a sentence nobody signed");
        assert_eq!(
            message(&forged, "openagents-public"),
            None,
            "the signature covers the content. A relay editing a message must \
             not be able to put words in the owner's sidebar under somebody \
             else's name."
        );
    }

    #[test]
    fn an_event_for_another_group_or_another_kind_is_refused() {
        assert_eq!(
            message(&real_event(), "some-other-group"),
            None,
            "a REQ asks for a group; it does not compel the relay to honour it."
        );
        let mut wrong_kind = real_event();
        wrong_kind["kind"] = json!(1);
        assert_eq!(message(&wrong_kind, "openagents-public"), None);
    }

    #[test]
    fn an_empty_message_takes_no_row() {
        let mut blank = real_event();
        blank["content"] = json!("   ");
        assert_eq!(
            message(&blank, "openagents-public"),
            None,
            "a blank row says nothing and takes the place of one that would."
        );
    }

    fn message_at(id: &str, created_at: i64) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            author: "66277e0bcc147cd1ae116f64ba45eb2f7021db7159eeffdc5083112fa0fbb7e4".to_string(),
            content: format!("message {id}"),
            created_at,
        }
    }

    #[test]
    fn rows_are_newest_first_bounded_and_aged() {
        let now = at(1_000);
        let drawn = rows(
            vec![
                message_at("a", 400),
                message_at("b", 940),
                message_at("c", 700),
            ],
            now,
            2,
        );
        assert_eq!(
            drawn.len(),
            2,
            "the bound is the owner's five, not the relay's"
        );
        assert_eq!(drawn[0].content.as_ref(), "message b");
        assert_eq!(drawn[0].age.as_ref(), "1m");
        assert_eq!(drawn[1].content.as_ref(), "message c");
        assert_eq!(drawn[1].age.as_ref(), "5m");
    }

    #[test]
    fn the_same_event_twice_is_one_row() {
        let drawn = rows(
            vec![message_at("a", 400), message_at("a", 400)],
            at(1_000),
            5,
        );
        assert_eq!(
            drawn.len(),
            1,
            "a relay may serve the same event across a reconnect; the event id \
             is the identity."
        );
    }

    #[test]
    fn two_messages_in_the_same_second_keep_a_stable_order() {
        let first = rows(
            vec![message_at("b", 500), message_at("a", 500)],
            at(1_000),
            5,
        );
        let second = rows(
            vec![message_at("a", 500), message_at("b", 500)],
            at(1_000),
            5,
        );
        assert_eq!(
            first, second,
            "a list that reorders itself between renders is worse than one that \
             is slightly wrong."
        );
    }

    #[test]
    fn an_author_is_written_the_way_the_web_page_writes_one() {
        assert_eq!(
            short_key("66277e0bcc147cd1ae116f64ba45eb2f7021db7159eeffdc5083112fa0fbb7e4"),
            "66277e0b…a0fbb7e4"
        );
        assert_eq!(
            short_key("short"),
            "short",
            "a key too short to elide is shown whole rather than padded into a \
             shape it does not have."
        );
    }
}
