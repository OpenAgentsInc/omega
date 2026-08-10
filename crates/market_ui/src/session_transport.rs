//! The authenticated gift-wrap lane for negotiated sessions (omega#244).
//!
//! Discovery reads public heads without authentication; gift-wrap reads
//! always require NIP-42. This socket authenticates as the session's
//! throwaway requester key, subscribes to `kind:1059` wraps addressed to it,
//! publishes the wraps handed to it, and unwraps every delivery through
//! `immortal_client::market::unwrap_mkt_record_raw` so the exact outer bytes
//! stay bound to the delivery evidence.

use async_tungstenite::async_std::connect_async;
use async_tungstenite::tungstenite::Message;
use futures::{StreamExt as _, future};
use immortal_client::domain::{
    Event, MKT_SWP_KEY_ROTATION_KIND, MKT_SWP_RELAY_SET_KIND, Tag, validate_mkt_public_event,
};
use immortal_client::market::{DeliveredMktRecord, MarketSigner, unwrap_mkt_record_raw};
use serde_json::{Value, json};

use crate::session_flow::swp_profile_support;

pub const SESSION_SUBSCRIPTION_ID: &str = "omega-market-session";
pub const SESSION_NETWORK_SUBSCRIPTION_ID: &str = "omega-market-network";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionInbox {
    Requester,
    Response,
}

pub enum SessionSocketEvent {
    /// NIP-42 authentication succeeded and the gift-wrap subscription is
    /// requested.
    Authenticated {
        relay_url: String,
        inbox: SessionInbox,
    },
    /// The relay finished replaying stored wraps.
    SubscriptionLive {
        relay_url: String,
        inbox: SessionInbox,
    },
    /// One gift wrap addressed to the session key was unwrapped and fully
    /// validated.
    Delivered {
        relay_url: String,
        inbox: SessionInbox,
        delivered: Box<DeliveredMktRecord>,
        observed_at: u64,
    },
    /// One signed public provider-network event from this relay.
    ProviderNetwork { relay_url: String, event: Event },
    /// The relay answered a published event.
    PublishResult {
        relay_url: String,
        event_id: String,
        accepted: bool,
        message: String,
    },
    /// A frame was dropped with a reason; the socket keeps running.
    Diagnostic { relay_url: String, message: String },
    /// One authenticated inbox stopped. The other inbox may remain live.
    Disconnected {
        relay_url: String,
        inbox: SessionInbox,
        reason: String,
    },
}

/// Runs the session socket until the relay closes or `outgoing` is dropped.
/// `outgoing` carries `kind:1059` wrap events to publish; `events` reports
/// deliveries and lifecycle. `now` supplies trusted local observation time.
pub async fn run_session_socket(
    relay_url: String,
    signer: MarketSigner,
    provider_id: String,
    inbox: SessionInbox,
    outgoing: async_channel::Receiver<Event>,
    events: async_channel::Sender<SessionSocketEvent>,
    now: fn() -> u64,
) -> Result<(), String> {
    let (mut stream, _response) = connect_async(relay_url.as_str())
        .await
        .map_err(|error| format!("session relay connection failed: {error}"))?;
    let mut authenticated = false;
    let mut gift_wrap_eose = false;
    let mut network_eose = false;
    let mut subscription_live_sent = false;
    loop {
        let message = if authenticated {
            match future::select(std::pin::pin!(outgoing.recv()), stream.next()).await {
                future::Either::Left((wrap, _)) => {
                    let Ok(wrap) = wrap else {
                        return Ok(());
                    };
                    stream
                        .send(Message::Text(json!(["EVENT", wrap]).to_string().into()))
                        .await
                        .map_err(|error| format!("session publish failed: {error}"))?;
                    continue;
                }
                future::Either::Right((message, _)) => message,
            }
        } else {
            stream.next().await
        };
        let Some(message) = message else {
            return Err("session relay closed the connection".to_owned());
        };
        let message = message.map_err(|error| format!("session relay read failed: {error}"))?;
        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Ping(payload) => {
                stream
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|error| format!("session relay pong failed: {error}"))?;
                continue;
            }
            Message::Close(_) => return Err("session relay closed the connection".to_owned()),
            _ => continue,
        };
        let frame: Value = match serde_json::from_str(&text) {
            Ok(frame) => frame,
            Err(error) => {
                send_event(
                    &events,
                    SessionSocketEvent::Diagnostic {
                        relay_url: relay_url.clone(),
                        message: format!("session frame is not JSON: {error}"),
                    },
                )
                .await?;
                continue;
            }
        };
        let label = frame
            .get(0)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match label.as_str() {
            "AUTH" => {
                let Some(challenge) = frame.get(1).and_then(Value::as_str) else {
                    send_event(
                        &events,
                        SessionSocketEvent::Diagnostic {
                            relay_url: relay_url.clone(),
                            message: "relay AUTH frame has no challenge".to_owned(),
                        },
                    )
                    .await?;
                    continue;
                };
                let auth = signer.sign(
                    now(),
                    22_242,
                    vec![
                        Tag::new(vec!["relay".to_owned(), relay_url.clone()]),
                        Tag::new(vec!["challenge".to_owned(), challenge.to_owned()]),
                    ],
                    String::new(),
                );
                stream
                    .send(Message::Text(json!(["AUTH", auth]).to_string().into()))
                    .await
                    .map_err(|error| format!("session authentication failed: {error}"))?;
                stream
                    .send(Message::Text(
                        json!([
                            "REQ",
                            SESSION_SUBSCRIPTION_ID,
                            { "kinds": [1_059], "#p": [signer.pubkey()], "limit": 512 }
                        ])
                        .to_string()
                        .into(),
                    ))
                    .await
                    .map_err(|error| format!("session subscription failed: {error}"))?;
                stream
                    .send(Message::Text(
                        json!([
                            "REQ",
                            SESSION_NETWORK_SUBSCRIPTION_ID,
                            {
                                "kinds": [MKT_SWP_KEY_ROTATION_KIND, MKT_SWP_RELAY_SET_KIND],
                                "#provider": [provider_id],
                                "limit": 128
                            }
                        ])
                        .to_string()
                        .into(),
                    ))
                    .await
                    .map_err(|error| format!("provider network subscription failed: {error}"))?;
                authenticated = true;
                send_event(
                    &events,
                    SessionSocketEvent::Authenticated {
                        relay_url: relay_url.clone(),
                        inbox,
                    },
                )
                .await?;
            }
            "EOSE" => {
                match frame.get(1).and_then(Value::as_str) {
                    Some(SESSION_SUBSCRIPTION_ID) => gift_wrap_eose = true,
                    Some(SESSION_NETWORK_SUBSCRIPTION_ID) => network_eose = true,
                    _ => {}
                }
                if gift_wrap_eose && network_eose && !subscription_live_sent {
                    subscription_live_sent = true;
                    send_event(
                        &events,
                        SessionSocketEvent::SubscriptionLive {
                            relay_url: relay_url.clone(),
                            inbox,
                        },
                    )
                    .await?;
                }
            }
            "EVENT" => {
                let subscription = frame.get(1).and_then(Value::as_str);
                if subscription == Some(SESSION_NETWORK_SUBSCRIPTION_ID) {
                    let Some(value) = frame.get(2).cloned() else {
                        continue;
                    };
                    let network_event = serde_json::from_value::<Event>(value)
                        .map_err(|error| format!("provider network event shape failed: {error}"));
                    match network_event.and_then(|event| {
                        event
                            .validate_structure()
                            .and_then(|()| event.validate_crypto())
                            .map_err(|error| {
                                format!("provider network signature failed: {error}")
                            })?;
                        validate_mkt_public_event(&event).map_err(|error| {
                            format!("provider network contract failed: {error}")
                        })?;
                        if event.tag_values("provider").next() != Some(provider_id.as_str()) {
                            return Err("provider network event has another provider_id".to_owned());
                        }
                        Ok(event)
                    }) {
                        Ok(event) => {
                            send_event(
                                &events,
                                SessionSocketEvent::ProviderNetwork {
                                    relay_url: relay_url.clone(),
                                    event,
                                },
                            )
                            .await?;
                        }
                        Err(message) => {
                            send_event(
                                &events,
                                SessionSocketEvent::Diagnostic {
                                    relay_url: relay_url.clone(),
                                    message,
                                },
                            )
                            .await?;
                        }
                    }
                    continue;
                }
                if subscription != Some(SESSION_SUBSCRIPTION_ID) {
                    continue;
                }
                let Some(wrap) = frame.get(2) else {
                    continue;
                };
                let raw = match serde_json::to_vec(wrap) {
                    Ok(raw) => raw,
                    Err(error) => {
                        send_event(
                            &events,
                            SessionSocketEvent::Diagnostic {
                                relay_url: relay_url.clone(),
                                message: format!("session wrap could not be serialized: {error}"),
                            },
                        )
                        .await?;
                        continue;
                    }
                };
                match unwrap_mkt_record_raw(&raw, &signer, &swp_profile_support()) {
                    Ok(delivered) => {
                        send_event(
                            &events,
                            SessionSocketEvent::Delivered {
                                relay_url: relay_url.clone(),
                                inbox,
                                delivered: Box::new(delivered),
                                observed_at: now(),
                            },
                        )
                        .await?;
                    }
                    Err(error) => {
                        send_event(
                            &events,
                            SessionSocketEvent::Diagnostic {
                                relay_url: relay_url.clone(),
                                message: format!("session wrap was rejected: {error}"),
                            },
                        )
                        .await?;
                    }
                }
            }
            "OK" => {
                let event_id = frame
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let accepted = frame.get(2).and_then(Value::as_bool).unwrap_or(false);
                let message = frame
                    .get(3)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                send_event(
                    &events,
                    SessionSocketEvent::PublishResult {
                        relay_url: relay_url.clone(),
                        event_id,
                        accepted,
                        message,
                    },
                )
                .await?;
            }
            "CLOSED" => {
                let reason = frame
                    .get(2)
                    .and_then(Value::as_str)
                    .unwrap_or("subscription closed by the relay");
                return Err(format!("session subscription closed: {reason}"));
            }
            "NOTICE" => {
                let notice = frame.get(1).and_then(Value::as_str).unwrap_or("notice");
                send_event(
                    &events,
                    SessionSocketEvent::Diagnostic {
                        relay_url: relay_url.clone(),
                        message: format!("relay notice: {notice}"),
                    },
                )
                .await?;
            }
            _ => {}
        }
    }
}

async fn send_event(
    events: &async_channel::Sender<SessionSocketEvent>,
    event: SessionSocketEvent,
) -> Result<(), String> {
    events
        .send(event)
        .await
        .map_err(|_| "session event receiver was dropped".to_owned())
}
