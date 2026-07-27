//! The client. `OMEGA-DELTA-0091`, omega#104.
//!
//! One socket, on this machine, carrying eight read requests.
//!
//! # Loopback is the whole boundary, and it is checked twice
//!
//! `exo serve` has **no authentication**. Its own HTTP documentation says a
//! client may send a bearer token and the server never looks at it, and the
//! endpoint exposes every one of Exo's fifty-two requests including the ones
//! that read secrets. Exo's CLI refuses a non-loopback `--bind`; Omega refuses
//! a non-loopback *destination*, which is the other half of the same law.
//!
//! The address is parsed by [`LoopbackEndpoint`], so a value of this client
//! cannot hold a remote host. Then, because `localhost` is a *name* and a name
//! resolves through `/etc/hosts` and the resolver, the resolved socket address
//! is checked again before the connection is opened. A machine whose hosts file
//! points `localhost` at a Tailnet address gets a refusal, not a connection.
//!
//! # No bearer token, ever
//!
//! Omega sends no `Authorization` header. Sending one would assert an
//! authentication that does not exist, and the first reader to see it in a
//! capture would reasonably conclude the endpoint was protected.
//!
//! # Blocking, on purpose
//!
//! The transport is `std::net` and nothing else, so this crate stays a leaf
//! with two dependencies and its round trip is testable against a real socket
//! in a unit test. Callers run it off the main thread; the engine already owns
//! readiness truth, and this is a read that fits behind it.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use omega_exo_lane::{EXO_SERVE_DEFAULT_BIND, LoopbackEndpoint, OffLoopback};

use crate::history::{ExoArtifactSet, ExoHistory};
use crate::query::{ExoEventWindow, ExoId, ExoQuery};
use crate::record::{
    ExoAgentRecord, ExoArtifact, ExoArtifactVersion, ExoConversation, ExoEvent, ExoEventPage,
    ExoResponseTag,
};

/// The path `exo serve` answers requests on.
///
/// `HTTP_EXOHARNESS_REQUEST_PATH` at the pinned commit. `GET /health` is the
/// only other route and this crate does not need it.
pub const EXO_REQUEST_PATH: &str = "/request";

/// The port `exo serve` binds when nobody says otherwise.
///
/// Read off `EXO_SERVE_DEFAULT_BIND`, so the lane's default lives in one place.
#[must_use]
pub fn exo_default_port() -> u16 {
    EXO_SERVE_DEFAULT_BIND
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
        .unwrap_or(4766)
}

/// Why a read did not produce a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExoReadError {
    /// The configured endpoint is not on this machine. Carries the reason, so
    /// the refusal is a sentence a person can read rather than a boolean.
    OffLoopback(OffLoopback),
    /// The address parsed as loopback and resolved to something else. The
    /// second check, and the one a poisoned hosts file trips.
    ResolvedOffLoopback { resolved: String },
    /// The address named a host this machine could not resolve at all.
    Unresolvable,
    /// The socket failed.
    Transport(String),
    /// Exo answered, and said no.
    Refused(String),
    /// Exo answered with a payload this query did not ask for.
    WrongShape {
        expected: &'static str,
        received: String,
    },
    /// The reply was not the envelope Exo documents.
    Undecodable(String),
    /// The reply was larger than this client will hold.
    TooLarge { limit: usize },
}

impl std::fmt::Display for ExoReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OffLoopback(reason) => write!(
                formatter,
                "Omega will not read Exo's log there: {reason}. Exo's endpoint has no \
                 authentication, so loopback is the entire boundary."
            ),
            Self::ResolvedOffLoopback { resolved } => write!(
                formatter,
                "that name resolved to {resolved}, which is not this machine. Exo's endpoint \
                 has no authentication, so loopback is the entire boundary."
            ),
            Self::Unresolvable => formatter.write_str("that address did not resolve"),
            Self::Transport(detail) => write!(formatter, "could not reach Exo: {detail}"),
            Self::Refused(detail) => write!(formatter, "Exo refused the read: {detail}"),
            Self::WrongShape { expected, received } => write!(
                formatter,
                "Exo answered with a `{received}` payload where this read expects `{expected}`"
            ),
            Self::Undecodable(detail) => write!(formatter, "could not read Exo's reply: {detail}"),
            Self::TooLarge { limit } => {
                write!(formatter, "Exo's reply is larger than {limit} bytes")
            }
        }
    }
}

impl std::error::Error for ExoReadError {}

/// A read-only client for one `exo serve`.
///
/// Construction is the only place an address is admitted, and it fails. Every
/// method takes an [`ExoQuery`], which has no write variant to pass.
#[derive(Clone, Debug)]
pub struct ExoReadClient {
    endpoint: LoopbackEndpoint,
    timeout: Duration,
    max_reply_bytes: usize,
}

impl ExoReadClient {
    /// This client will not hold a reply larger than this.
    ///
    /// An artifact read returns its bytes as a JSON array of numbers, which
    /// costs about four bytes of wire per byte of artifact, so this admits
    /// roughly a sixteen-megabyte artifact. A bound that exists is the point:
    /// the reply size is decided by whatever Exo's agent wrote.
    pub const DEFAULT_MAX_REPLY_BYTES: usize = 64 * 1024 * 1024;

    /// The default socket timeout.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    /// Point a client at an `exo serve` on this machine.
    ///
    /// # Errors
    ///
    /// [`ExoReadError::OffLoopback`] for any address that is not this machine,
    /// carrying the reason.
    pub fn open(address: &str) -> Result<Self, ExoReadError> {
        let endpoint = LoopbackEndpoint::parse(address).map_err(ExoReadError::OffLoopback)?;
        Ok(Self {
            endpoint,
            timeout: Self::DEFAULT_TIMEOUT,
            max_reply_bytes: Self::DEFAULT_MAX_REPLY_BYTES,
        })
    }

    /// Point a client at Exo's own default bind.
    ///
    /// # Errors
    ///
    /// Cannot fail at this pin; the signature keeps the one construction path.
    pub fn open_default() -> Result<Self, ExoReadError> {
        Self::open(EXO_SERVE_DEFAULT_BIND)
    }

    /// Replace the socket timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Replace the reply bound.
    #[must_use]
    pub const fn with_max_reply_bytes(mut self, bytes: usize) -> Self {
        self.max_reply_bytes = bytes;
        self
    }

    /// The endpoint, which is loopback because it is this type.
    #[must_use]
    pub const fn endpoint(&self) -> &LoopbackEndpoint {
        &self.endpoint
    }

    /// The agent record. Exo's `get_agent`.
    ///
    /// # Errors
    ///
    /// [`ExoReadError`] for any refusal, transport failure, or unexpected shape.
    pub fn agent(&self, query: &ExoQuery) -> Result<Option<ExoAgentRecord>, ExoReadError> {
        let payload = self.payload(query, ExoResponseTag::Agent)?;
        decode_optional(&payload, "agent")
    }

    /// The conversation record. Exo's `get_conversation`.
    ///
    /// # Errors
    ///
    /// [`ExoReadError`] for any refusal, transport failure, or unexpected shape.
    pub fn conversation(&self, query: &ExoQuery) -> Result<Option<ExoConversation>, ExoReadError> {
        let payload = self.payload(query, ExoResponseTag::Conversation)?;
        decode_optional(&payload, "conversation")
    }

    /// One page of the durable event log. Exo's `conversation_get_events`.
    ///
    /// # Errors
    ///
    /// [`ExoReadError`] for any refusal, transport failure, or unexpected shape.
    pub fn events(&self, query: &ExoQuery) -> Result<ExoEventPage, ExoReadError> {
        let payload = self.payload(query, ExoResponseTag::Events)?;
        decode_field(&payload, "result")
    }

    /// One event. Exo's `conversation_get_event`.
    ///
    /// # Errors
    ///
    /// [`ExoReadError`] for any refusal, transport failure, or unexpected shape.
    pub fn event(&self, query: &ExoQuery) -> Result<Option<ExoEvent>, ExoReadError> {
        let payload = self.payload(query, ExoResponseTag::Event)?;
        decode_optional(&payload, "event")
    }

    /// Every artifact version in scope, named but not read.
    ///
    /// # Errors
    ///
    /// [`ExoReadError`] for any refusal, transport failure, or unexpected shape.
    pub fn artifact_versions(
        &self,
        query: &ExoQuery,
    ) -> Result<Vec<ExoArtifactVersion>, ExoReadError> {
        let payload = self.payload(query, ExoResponseTag::ArtifactVersions)?;
        decode_field(&payload, "artifacts")
    }

    /// One artifact, with its bytes. The read that carries tool results.
    ///
    /// # Errors
    ///
    /// [`ExoReadError`] for any refusal, transport failure, or unexpected shape.
    pub fn artifact(&self, query: &ExoQuery) -> Result<Option<ExoArtifact>, ExoReadError> {
        let payload = self.payload(query, ExoResponseTag::Artifact)?;
        decode_optional(&payload, "artifact")
    }

    /// One conversation's durable record, with the bodies its tool results
    /// went to. `OMEGA-DELTA-0107`, omega#104.
    ///
    /// # Two passes, and the second is the one with the history in it
    ///
    /// Exo's event log **names** artifacts and never contains them. So the
    /// first `ExoHistory::read` is not the answer — it is the question: it says
    /// which artifact versions would change the rendering. Then those are read,
    /// and the same events are rendered again against them. Skip the second
    /// pass and every artifact-backed tool result renders as [`ExoBody::NotRead`]
    /// — which is the crate saying so honestly, and is still not a history.
    ///
    /// Each unresolved reference is fetched at **the version it named**, not at
    /// `null`. An event that referenced version 1 asked for version 1's bytes;
    /// fetching the latest would fill that row with a body from a later point in
    /// the conversation, under the right name.
    ///
    /// # A read that fails leaves its row unread, and the row says so
    ///
    /// An artifact Exo has since dropped, or an id in the log that is not
    /// UUID-shaped, does not fail the whole history: forty rows are not lost
    /// because one body is. That row stays [`ExoBody::NotRead`] and
    /// [`ExoHistory::unread_artifact_rows`] counts it, which is exactly the
    /// state a partial read is in and exactly what the caller should surface.
    ///
    /// [`ExoBody::NotRead`]: crate::ExoBody::NotRead
    ///
    /// # Errors
    ///
    /// [`ExoReadError`] if the *event* read fails. That one is the history.
    pub fn conversation_history(
        &self,
        agent: &ExoId,
        conversation: &ExoId,
        limit: Option<u32>,
    ) -> Result<ExoHistory, ExoReadError> {
        let page = self.events(&ExoQuery::ConversationEvents {
            agent: agent.clone(),
            conversation: conversation.clone(),
            window: ExoEventWindow {
                limit,
                ..ExoEventWindow::default()
            },
        })?;
        let mut artifacts = ExoArtifactSet::new();
        let named = ExoHistory::read(&page.events, &artifacts);
        for reference in named.unresolved_artifacts(&artifacts) {
            let Ok(artifact) = ExoId::parse(&reference.artifact_id) else {
                continue;
            };
            let read = self.artifact(&ExoQuery::ConversationArtifact {
                agent: agent.clone(),
                conversation: conversation.clone(),
                artifact,
                version: reference.version,
            });
            if let Ok(Some(artifact)) = read {
                artifacts.insert(artifact);
            }
        }
        Ok(ExoHistory::read(&page.events, &artifacts))
    }

    /// Send one query and return the `response` object Exo replied with.
    ///
    /// `expected` is passed by the caller and compared against the query's own
    /// [`ExoQuery::expects`], so a method wired to the wrong reader fails here
    /// rather than returning an empty record.
    fn payload(
        &self,
        query: &ExoQuery,
        expected: ExoResponseTag,
    ) -> Result<serde_json::Value, ExoReadError> {
        if query.expects() != expected {
            return Err(ExoReadError::WrongShape {
                expected: expected.wire(),
                received: query.expects().wire().to_owned(),
            });
        }
        let body = self.request_body(query);
        let reply = self.post(&body)?;
        decode_envelope(&reply, expected)
    }

    /// The `ClientMessage::Request` frame for a query.
    ///
    /// The request id is fixed at 1 because this transport is one request per
    /// connection: there is no second reply for it to disambiguate, and a
    /// counter would only produce a number nobody compares.
    fn request_body(&self, query: &ExoQuery) -> String {
        serde_json::Value::Object(
            [
                (
                    "kind".to_owned(),
                    serde_json::Value::String("request".into()),
                ),
                ("id".to_owned(), serde_json::Value::from(1u64)),
                ("request".to_owned(), query.wire_request()),
            ]
            .into_iter()
            .collect(),
        )
        .to_string()
    }

    /// The resolved address, checked a second time.
    fn resolve(&self) -> Result<SocketAddr, ExoReadError> {
        let port = self.endpoint.port().unwrap_or_else(exo_default_port);
        let host = self.endpoint.host();
        let authority = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let mut resolved = authority
            .to_socket_addrs()
            .map_err(|_| ExoReadError::Unresolvable)?;
        let address = resolved.next().ok_or(ExoReadError::Unresolvable)?;
        if !address.ip().is_loopback() {
            return Err(ExoReadError::ResolvedOffLoopback {
                resolved: address.to_string(),
            });
        }
        Ok(address)
    }

    fn post(&self, body: &str) -> Result<String, ExoReadError> {
        let address = self.resolve()?;
        let stream = TcpStream::connect_timeout(&address, self.timeout)
            .map_err(|error| ExoReadError::Transport(error.to_string()))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.timeout)))
            .map_err(|error| ExoReadError::Transport(error.to_string()))?;
        let mut stream = stream;
        let request = format!(
            "POST {EXO_REQUEST_PATH} HTTP/1.1\r\n\
             host: {address}\r\n\
             content-type: application/json\r\n\
             accept: application/json\r\n\
             content-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|()| stream.flush())
            .map_err(|error| ExoReadError::Transport(error.to_string()))?;

        let mut raw = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let read = stream
                .read(&mut chunk)
                .map_err(|error| ExoReadError::Transport(error.to_string()))?;
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..read]);
            if raw.len() > self.max_reply_bytes {
                return Err(ExoReadError::TooLarge {
                    limit: self.max_reply_bytes,
                });
            }
        }
        http_body(&raw)
    }
}

/// Split an HTTP/1.1 reply into its body.
///
/// Exo's server answers exoharness-level failures with `200` and an `ok: false`
/// envelope, so a non-2xx status is a transport fault and is reported as one.
/// Both framings are handled: `content-length`, and `transfer-encoding:
/// chunked`, which is what a server sends when it streams a reply it has not
/// measured.
fn http_body(raw: &[u8]) -> Result<String, ExoReadError> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| ExoReadError::Undecodable("no HTTP header terminator".into()))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| ExoReadError::Undecodable("no HTTP status line".into()))?;
    let chunked = lines.any(|line| {
        let lowered = line.to_ascii_lowercase();
        lowered.starts_with("transfer-encoding:") && lowered.contains("chunked")
    });
    if !(200..300).contains(&status) {
        return Err(ExoReadError::Transport(format!("HTTP {status} from Exo")));
    }

    let body = &raw[split + 4..];
    let body = if chunked {
        dechunk(body)?
    } else {
        body.to_vec()
    };
    String::from_utf8(body).map_err(|error| ExoReadError::Undecodable(error.to_string()))
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, ExoReadError> {
    let mut out = Vec::new();
    loop {
        let end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| ExoReadError::Undecodable("truncated chunk header".into()))?;
        let header = String::from_utf8_lossy(&body[..end]);
        let size = usize::from_str_radix(header.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| ExoReadError::Undecodable(format!("chunk size `{header}`")))?;
        body = &body[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size {
            return Err(ExoReadError::Undecodable("truncated chunk body".into()));
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size..];
        if body.starts_with(b"\r\n") {
            body = &body[2..];
        }
    }
}

/// Read Exo's `ServerMessage` and return the `response` object.
fn decode_envelope(
    reply: &str,
    expected: ExoResponseTag,
) -> Result<serde_json::Value, ExoReadError> {
    let envelope: serde_json::Value = serde_json::from_str(reply)
        .map_err(|error| ExoReadError::Undecodable(error.to_string()))?;
    let ok = envelope
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| ExoReadError::Undecodable("no `ok` in Exo's envelope".into()))?;
    if !ok {
        return Err(ExoReadError::Refused(
            envelope
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Exo gave no reason")
                .to_owned(),
        ));
    }
    let payload = envelope
        .get("response")
        .filter(|payload| !payload.is_null())
        .ok_or_else(|| ExoReadError::Undecodable("Exo said ok and sent no payload".into()))?;
    let tag = payload
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if tag != expected.wire() {
        return Err(ExoReadError::WrongShape {
            expected: expected.wire(),
            received: tag.to_owned(),
        });
    }
    Ok(payload.clone())
}

fn decode_field<T: serde::de::DeserializeOwned>(
    payload: &serde_json::Value,
    field: &str,
) -> Result<T, ExoReadError> {
    let value = payload
        .get(field)
        .ok_or_else(|| ExoReadError::Undecodable(format!("no `{field}` in Exo's payload")))?;
    serde_json::from_value(value.clone())
        .map_err(|error| ExoReadError::Undecodable(error.to_string()))
}

fn decode_optional<T: serde::de::DeserializeOwned>(
    payload: &serde_json::Value,
    field: &str,
) -> Result<Option<T>, ExoReadError> {
    match payload.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| ExoReadError::Undecodable(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{ExoEventWindow, ExoId};
    use std::io::BufRead;
    use std::net::TcpListener;

    const AGENT: &str = "0198f3ec-1b7a-7c31-9f0e-6d2a4b8c1d55";
    const CONVERSATION: &str = "0198f3ec-2c8b-7d42-a01f-7e3b5c9d2e66";

    fn id(text: &str) -> ExoId {
        ExoId::parse(text).expect("a UUID")
    }

    fn events_query() -> ExoQuery {
        ExoQuery::ConversationEvents {
            agent: id(AGENT),
            conversation: id(CONVERSATION),
            window: ExoEventWindow::default(),
        }
    }

    /// A loopback server that answers one request and returns what it was sent.
    ///
    /// The thread ends when the connection does, so the test owns its whole
    /// lifetime and nothing outlives it.
    fn one_shot(reply: String) -> (ExoReadClient, std::thread::JoinHandle<String>) {
        let (client, server) = serving(vec![reply]);
        let handle =
            std::thread::spawn(move || server.join().expect("the server thread").remove(0));
        (client, handle)
    }

    /// A loopback server that answers `replies.len()` requests in order and
    /// returns what it was sent, in order.
    ///
    /// One request per connection — the client sends `connection: close` — so a
    /// two-pass read is two accepts. The thread ends when the scripted replies
    /// run out, so the test owns its whole lifetime and nothing outlives it.
    fn serving(replies: Vec<String>) -> (ExoReadClient, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let address = listener.local_addr().expect("a bound address");
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().expect("one connection");
                let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone"));
                let mut request = String::new();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("a header line");
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    let lowered = line.to_ascii_lowercase();
                    if let Some(value) = lowered.strip_prefix("content-length:") {
                        length = value.trim().parse().expect("a length");
                    }
                    request.push_str(&line);
                }
                let mut body = vec![0u8; length];
                std::io::Read::read_exact(&mut reader, &mut body).expect("the body");
                stream.write_all(reply.as_bytes()).expect("the reply");
                stream.flush().expect("flush");
                drop(stream);
                seen.push(format!("{request}\r\n{}", String::from_utf8_lossy(&body)));
            }
            seen
        });
        let client = ExoReadClient::open(&address.to_string()).expect("loopback");
        (client, handle)
    }

    fn measured(payload: serde_json::Value) -> String {
        let body = serde_json::json!({
            "kind": "response",
            "id": 1,
            "ok": true,
            "response": payload,
            "error": null,
        })
        .to_string();
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// The refusal this issue asks to be watched. A non-loopback endpoint does
    /// not produce a client, and the reason is a sentence.
    #[test]
    fn a_non_loopback_endpoint_is_refused_and_says_why() {
        for (address, reason) in [
            ("0.0.0.0:4766", OffLoopback::EveryInterface),
            ("10.0.0.4:4766", OffLoopback::RemoteHost),
            ("100.64.7.9:4766", OffLoopback::RemoteHost),
            ("http://exo.openagents.com/request", OffLoopback::RemoteHost),
            ("ssh://127.0.0.1:4766", OffLoopback::Unreadable),
        ] {
            let refusal = ExoReadClient::open(address).expect_err(address);
            assert_eq!(refusal, ExoReadError::OffLoopback(reason), "{address}");
            let sentence = refusal.to_string();
            assert!(sentence.contains("no authentication"), "{sentence}");
            assert!(sentence.contains("loopback"), "{sentence}");
        }
        assert!(ExoReadClient::open_default().is_ok());
    }

    /// A real socket, real HTTP framing, and Exo's documented envelope.
    #[test]
    fn a_real_loopback_round_trip_reads_the_durable_log() {
        let (client, server) = one_shot(measured(serde_json::json!({
            "type": "events",
            "result": {
                "events": [{
                    "id": "0198f3ec-4eaf-7f64-c231-9a5d7ebf4088",
                    "conversation_id": CONVERSATION,
                    "session_id": null,
                    "turn_id": null,
                    "created_at": "2026-07-26T09:15:00Z",
                    "data": { "type": "turn_ended" },
                }],
                "cursor": null,
            },
        })));
        let page = client.events(&events_query()).expect("a page");
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].tag(), "turn_ended");

        let seen = server.join().expect("the server thread");
        assert!(seen.contains("POST /request HTTP/1.1"), "{seen}");
        assert!(
            seen.contains("\"type\":\"conversation_get_events\""),
            "{seen}"
        );
        assert!(
            !seen.to_ascii_lowercase().contains("authorization"),
            "Omega must send no bearer token to an endpoint that never checks one: {seen}"
        );
    }

    #[test]
    fn a_chunked_reply_is_read() {
        let payload = serde_json::json!({
            "kind": "response", "id": 1, "ok": true,
            "response": { "type": "artifact_versions", "artifacts": [] },
            "error": null,
        })
        .to_string();
        let (first, second) = payload.split_at(20);
        let reply = format!(
            "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n\
             {:x}\r\n{first}\r\n{:x}\r\n{second}\r\n0\r\n\r\n",
            first.len(),
            second.len()
        );
        let (client, server) = one_shot(reply);
        let versions = client
            .artifact_versions(&ExoQuery::ConversationArtifacts {
                agent: id(AGENT),
                conversation: id(CONVERSATION),
            })
            .expect("an artifact list");
        assert!(versions.is_empty());
        server.join().expect("the server thread");
    }

    /// Exo answers its own failures with HTTP 200 and `ok: false`. The reason
    /// has to survive to the reader.
    #[test]
    fn an_exo_level_refusal_keeps_its_reason() {
        let body = serde_json::json!({
            "kind": "response", "id": 1, "ok": false, "response": null,
            "error": "conversation 0198f3ec not found",
        })
        .to_string();
        let (client, server) = one_shot(format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        ));
        let refusal = client.events(&events_query()).expect_err("a refusal");
        assert_eq!(
            refusal,
            ExoReadError::Refused("conversation 0198f3ec not found".into())
        );
        assert!(refusal.to_string().contains("not found"));
        server.join().expect("the server thread");
    }

    /// A payload for a different query must not decode into an empty page.
    #[test]
    fn a_payload_this_query_did_not_ask_for_is_a_failure() {
        let (client, server) = one_shot(measured(serde_json::json!({
            "type": "agents", "agents": [],
        })));
        assert_eq!(
            client.events(&events_query()).expect_err("wrong shape"),
            ExoReadError::WrongShape {
                expected: "events",
                received: "agents".into(),
            }
        );
        server.join().expect("the server thread");
    }

    /// Every reader is bound to one query shape, so a reader called with the
    /// wrong query refuses before it opens a socket.
    #[test]
    fn a_reader_called_with_the_wrong_query_never_reaches_the_socket() {
        let client = ExoReadClient::open_default().expect("loopback");
        assert_eq!(
            client.agent(&events_query()).expect_err("wrong query"),
            ExoReadError::WrongShape {
                expected: "agent",
                received: "events".into(),
            }
        );
    }

    const ARTIFACT: &str = "0198f3ec-3d9c-7e53-b120-8f4c6dae3f77";

    /// A turn as Exo records it: a tool call whose whole result went to
    /// version 1 of an artifact, with a preview left in the event.
    fn a_turn_with_an_artifact_backed_result() -> serde_json::Value {
        serde_json::json!({
            "type": "events",
            "result": {
                "events": [
                    {
                        "id": "0198f3ec-4eaf-7f64-c231-9a5d7ebf4088",
                        "conversation_id": CONVERSATION,
                        "session_id": null, "turn_id": null,
                        "created_at": "2026-07-26T09:15:00Z",
                        "data": {
                            "type": "tool_requested",
                            "tool_call_id": "call-1",
                            "request": { "function_name": "bash", "arguments": {} },
                        },
                    },
                    {
                        "id": "0198f3ec-4eaf-7f64-c231-9a5d7ebf4089",
                        "conversation_id": CONVERSATION,
                        "session_id": null, "turn_id": null,
                        "created_at": "2026-07-26T09:15:01Z",
                        "data": {
                            "type": "tool_result",
                            "tool_call_id": "call-1",
                            "result": { "artifact_id": ARTIFACT, "version": 1, "preview": "…" },
                        },
                    },
                ],
                "cursor": null,
            },
        })
    }

    /// `OMEGA-DELTA-0107`. The read is two passes, and the second one is what
    /// carries the tool results.
    ///
    /// Against a scripted loopback server rather than a live `exo serve`: the
    /// framing, the envelope, the request shapes and the ordering are real, and
    /// the answers are this test's. See omega#104 for what that does and does
    /// not prove.
    #[test]
    fn the_second_pass_carries_the_tool_results_the_first_only_named() {
        let (client, server) = serving(vec![
            measured(a_turn_with_an_artifact_backed_result()),
            measured(serde_json::json!({
                "type": "artifact",
                "artifact": {
                    "artifact_id": ARTIFACT,
                    "path": "scheduled-tasks/nightly/run-1.json",
                    "version": 1,
                    "created_at": "2026-07-26T09:15:01Z",
                    "size_bytes": 24,
                    "contents": "every line of the output".as_bytes(),
                },
            })),
        ]);

        let history = client
            .conversation_history(&id(AGENT), &id(CONVERSATION), Some(200))
            .expect("a durable history");
        assert_eq!(history.rows.len(), 2);
        assert_eq!(history.unread_artifact_rows, 0);
        assert!(
            history.to_text().contains("every line of the output"),
            "{}",
            history.to_text()
        );

        let seen = server.join().expect("the server thread");
        assert_eq!(seen.len(), 2, "the read is two requests, in order");
        assert!(
            seen[0].contains("\"type\":\"conversation_get_events\""),
            "{}",
            seen[0]
        );
        assert!(
            seen[1].contains("\"type\":\"conversation_read_artifact\""),
            "{}",
            seen[1]
        );
        assert!(
            seen[1].contains("\"version\":1"),
            "the second pass must ask for the version the event named, not the \
             latest Exo happens to hold: {}",
            seen[1]
        );
    }

    /// `OMEGA-DELTA-0107`. A body that could not be read costs its row's body
    /// and nothing else.
    ///
    /// The other half of the falsifier: the same events, with the artifact read
    /// refused. Every row survives, the tool result says its body was not read,
    /// and the count of unread rows is what a caller surfaces.
    #[test]
    fn an_artifact_read_that_fails_costs_one_body_and_no_rows() {
        let refusal = serde_json::json!({
            "kind": "response", "id": 1, "ok": false, "response": null,
            "error": "artifact not found",
        })
        .to_string();
        let (client, server) = serving(vec![
            measured(a_turn_with_an_artifact_backed_result()),
            format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{refusal}",
                refusal.len()
            ),
        ]);

        let history = client
            .conversation_history(&id(AGENT), &id(CONVERSATION), None)
            .expect("the events read is the history; one missing body is not");
        assert_eq!(history.rows.len(), 2, "no row was dropped");
        assert_eq!(history.unread_artifact_rows, 1);
        assert!(
            history.to_text().contains("body not read"),
            "{}",
            history.to_text()
        );
        assert_eq!(server.join().expect("the server thread").len(), 2);
    }

    #[test]
    fn the_default_port_is_read_off_the_lanes_own_default_bind() {
        assert_eq!(exo_default_port(), 4766);
        assert!(EXO_SERVE_DEFAULT_BIND.ends_with(&exo_default_port().to_string()));
    }
}
