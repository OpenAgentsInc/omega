//! The negotiated NIP-MKT session flow for the Markets panel (omega#244).
//!
//! This module drives the requester side of an MKT-SWP v1 no-spend session:
//! RFQ → Quote (indicative/firm, reservation class) → Order → per-signer
//! Status timeline with visible sequence gaps and forks → Cancel → Close.
//! Every signed record is constructed through
//! `immortal_client::mkt_swp_client::SwapRecordFactory`, carried as a NIP-59
//! gift wrap built by `immortal_client::market`, and validated on receipt by
//! the Immortal domain module. Omega owns only transport, persistence, keys,
//! and user-facing policy — never a parallel implementation of event,
//! signature, or MKT validation.
//!
//! Keys are throwaway per-session development keypairs. An Order authorizes
//! coordination only: there is no wallet, settlement, or custody surface
//! here, and the session targets the no-spend provider from the Immortal
//! local dev environment.
//!
//! # Persistence shape
//!
//! Each session is durably stored as one JSON document under the user data
//! directory (`<data dir>/market_sessions/<session-id>.json`):
//!
//! ```json
//! {
//!   "schema": "omega.market.session.v1",
//!   "session_id": "<64-lower-hex>",
//!   "requester_pubkey": "<64-lower-hex>",
//!   "provider_pubkey": "<64-lower-hex>",
//!   "offering_address": "39601:<provider>:<offering-id>",
//!   "records": [
//!     {
//!       "raw_signed_event_hex": "<exact signed record bytes>",
//!       "observed_at": 1700000000,
//!       "provenance": "locally_signed" | "gift_wrap",
//!       "wrap_event_id": "<64-lower-hex>" | null
//!     }
//!   ]
//! }
//! ```
//!
//! The exact signed bytes, receipt time, and delivery provenance satisfy the
//! MKT retention rule (transport step 7). On reconnect the session replays
//! its own signed records as freshly wrapped copies; identical inner bytes
//! are idempotent replay for the counterparty. The throwaway session secret
//! is deliberately not stored.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use immortal_client::domain::{
    Event, MKT_CANCEL_KIND, MKT_CLOSE_KIND, MKT_QUOTE_KIND, MKT_STATUS_KIND, MKT_SWP_PROFILE_ID,
    MKT_SWP_PROFILE_VERSION, MKT_SWP_SWAP_CONTRACT_KIND, MktProfileSupport,
};
use immortal_client::market::{
    DeliveredMktRecord, MarketSigner, WrapMaterial, WrappedMktRecord, wrap_mkt_record,
};
use immortal_client::mkt_swp_client::{
    Cancellation, CloseOutcome, DeliveryProvenance, MktSigningRequest, ParticipantRole,
    RequesterContractLocalInputs, RequesterContractSigningInput, RequesterExitPackageCommitment,
    RequesterOrderInput, RequesterSessionView, SignedRecordDelivery, StatusState, SwapClientConfig,
    SwapRecordFactory, SwapType,
};
use serde_json::{Map, Value, json};
use ui::prelude::SharedString;

use crate::discovery::OfferingListing;

pub const SESSION_FLOW_TRACKING_ISSUE: &str = "OpenAgentsInc/omega#244";
pub const SESSION_STORE_SCHEMA: &str = "omega.market.session.v1";
pub const SESSION_STORE_DIRECTORY: &str = "market_sessions";

const RFQ_LIFETIME_SECONDS: u64 = 900;
const CANCEL_REASON: &str = "omega_requester_cancelled";
const MAX_QUOTE_CANDIDATES: usize = 64;
const MAX_SESSION_RECORDS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFlowAvailability {
    Available,
}

/// The Markets panel renders session controls only while this reports
/// `Available`; the seam is kept so the gate stays capability-derived.
pub fn session_flow_availability() -> SessionFlowAvailability {
    SessionFlowAvailability::Available
}

/// The frozen no-spend submarine RFQ constraints. These mirror the pinned
/// Immortal fixture corpus (`tests/fixtures/nipmkt/swp-full-sessions-v1.json`,
/// submarine flow) because the dev no-spend provider answers only its frozen
/// request templates: the payment hash, invoice digest, and requester wallet
/// keys are published fixture commitments, not live wallet material.
fn no_spend_submarine_constraints() -> Value {
    json!({
        "constraints": {
            "allowed_script_modes": ["taproot-musig2-script-exit"],
            "asset_pair": [
                "swp:1:bip122:00000000000000000000000000000000:btc:chain",
                "swp:1:bip122:00000000000000000000000000000000:btc:lightning"
            ],
            "confirmation_policy": {
                "minimum_confirmations": "1",
                "rbf": "reject",
                "reorg_safety_blocks": "6",
                "replacement": "reject",
                "zero_confirmation": "forbidden"
            },
            "desired_completion_time": 2000,
            "firm_quote_required": true,
            "input_amount": "100000",
            "invoice_sha256": "b0a570bb4ee56b4c1a2dfa43e1238af4be827e9bee7b17dd5ab85e27f01fead6",
            "maximum_total_fee": "99000",
            "payment_hash": "96c772a829fb7c780410f1d85cf12a89e8b3c78c0bac5fb47f62758bf961ec30",
            "requester_public_keys": [
                {
                    "leg_id": "source",
                    "path": "refund",
                    "public_key": "716022efaca232dd8a7927619a9e5f1eb8f1c8b87436a52a03ae7e1239a1662a"
                }
            ],
            "swap_type": "submarine"
        }
    })
}

/// The presigned requester refund exit-package commitment from the same
/// fixture corpus; the no-spend Quote pins this digest as a frozen
/// commitment.
fn no_spend_submarine_exit_commitment() -> RequesterExitPackageCommitment {
    RequesterExitPackageCommitment {
        participant_role: "requester".to_owned(),
        leg_id: "source".to_owned(),
        path: "refund".to_owned(),
        package_mode: "presigned".to_owned(),
        package_sha256: "77abefe30c067cc2f46a9947c38c09a0f6bfd9aedff026fa3760ce1c319adb11"
            .to_owned(),
    }
}

pub fn swp_profile_support() -> [MktProfileSupport<'static>; 1] {
    [MktProfileSupport {
        profile_id: MKT_SWP_PROFILE_ID,
        version: MKT_SWP_PROFILE_VERSION,
        critical_members: &[],
        understood_members: &[],
    }]
}

fn random_32() -> [u8; 32] {
    rand::random()
}

fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn random_hex_32() -> String {
    lower_hex(&random_32())
}

/// A throwaway per-session development keypair from operating-system
/// randomness. This is a Nostr identity key, never wallet material.
pub fn throwaway_session_signer() -> Result<MarketSigner, String> {
    for _ in 0..32 {
        if let Ok(signer) = MarketSigner::from_secret_bytes(random_32()) {
            return Ok(signer);
        }
    }
    Err("could not generate a session keypair".to_owned())
}

fn random_wrap_material(now: u64) -> WrapMaterial {
    let jitter = |bytes: [u8; 32]| u64::from(bytes[0]).saturating_mul(10);
    WrapMaterial {
        seal_created_at: now.saturating_sub(jitter(random_32())),
        wrap_created_at: now.saturating_sub(jitter(random_32())),
        rumor_identifier: random_32(),
        seal_nonce: random_32(),
        wrap_nonce: random_32(),
        wrap_secret: {
            let mut secret = random_32();
            for _ in 0..32 {
                if MarketSigner::from_secret_bytes(secret).is_ok() {
                    break;
                }
                secret = random_32();
            }
            secret
        },
    }
}

/// Wraps one signed record twice per the MKT transport steps: one gift wrap
/// for the counterparty and one sender-recovery copy, each with independent
/// NIP-59 material and randomized wrapper timestamps.
pub fn wrap_for_transport(
    event: &Event,
    signer: &MarketSigner,
    counterparty: &str,
    now: u64,
) -> Result<Vec<WrappedMktRecord>, String> {
    let raw = serde_json::to_vec(event)
        .map_err(|error| format!("could not serialize a signed record: {error}"))?;
    let mut wraps = Vec::with_capacity(2);
    for recipient in [counterparty, signer.pubkey()] {
        wraps.push(wrap_mkt_record(
            &raw,
            signer,
            recipient,
            random_wrap_material(now),
        )?);
    }
    Ok(wraps)
}

/// One quote received for the session's RFQ, projected for comparison. Tag
/// grammar and profile terms were already validated by the Immortal domain
/// during unwrap; extraction here fails closed on anything the comparison
/// needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteCandidate {
    pub event: Event,
    pub quote_class: String,
    pub reservation: String,
    pub input_amount: String,
    pub output_amount: String,
    pub maximum_total_fee: String,
    pub fee_bps: String,
    pub asset_pair: (String, String),
    pub expires_at: u64,
    pub previous: Option<String>,
}

impl QuoteCandidate {
    pub fn from_event(event: &Event, rfq_id: &str) -> Result<Self, String> {
        let references_rfq = event.tags.iter().any(|tag| {
            let slice = tag.as_slice();
            slice.first().map(String::as_str) == Some("e")
                && slice.get(1).map(String::as_str) == Some(rfq_id)
                && slice.get(3).map(String::as_str) == Some("rfq")
        });
        if !references_rfq {
            return Err("quote does not reference this session's RFQ".to_owned());
        }
        let tag = |name: &str| {
            event
                .tag_values(name)
                .next()
                .map(str::to_owned)
                .ok_or_else(|| format!("quote omits its {name} tag"))
        };
        let expires_at = tag("expiration")?
            .parse::<u64>()
            .map_err(|_| "quote expiration is not a unix time".to_owned())?;
        let content: Value = serde_json::from_str(&event.content)
            .map_err(|error| format!("quote content is not JSON: {error}"))?;
        let terms = content
            .pointer("/mkt_swp/terms")
            .and_then(Value::as_object)
            .ok_or_else(|| "quote has no MKT-SWP terms".to_owned())?;
        let amount = |name: &str| -> Result<String, String> {
            let value = terms
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("quote terms omit {name}"))?;
            if !is_canonical_amount(value) {
                return Err(format!("quote {name} is not a canonical amount"));
            }
            Ok(value.to_owned())
        };
        let asset_pair = terms
            .get("asset_pair")
            .and_then(Value::as_array)
            .filter(|pair| pair.len() == 2)
            .and_then(|pair| Some((pair[0].as_str()?.to_owned(), pair[1].as_str()?.to_owned())))
            .ok_or_else(|| "quote terms omit the asset pair".to_owned())?;
        let previous = event
            .tags
            .iter()
            .find_map(|tag| {
                let slice = tag.as_slice();
                (slice.first().map(String::as_str) == Some("e")
                    && slice.get(3).map(String::as_str) == Some("previous"))
                .then(|| slice.get(1).cloned())
            })
            .flatten();
        Ok(Self {
            event: event.clone(),
            quote_class: tag("quote")?,
            reservation: tag("reservation")?,
            input_amount: amount("input_amount")?,
            output_amount: amount("output_amount")?,
            maximum_total_fee: amount("maximum_total_fee")?,
            fee_bps: terms
                .get("fee_bps")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            asset_pair,
            expires_at,
            previous,
        })
    }

    pub fn usable(&self, now: u64) -> bool {
        self.expires_at > now
    }
}

fn is_canonical_amount(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

fn compare_amounts(a: &str, b: &str) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// Quote selection policy: highest output amount, then lowest maximum total
/// fee, then lexicographically lowest provider pubkey. Returns the index of
/// the winner among usable candidates.
pub fn select_quote(candidates: &[QuoteCandidate], now: u64) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.usable(now))
        .max_by(|(_, a), (_, b)| {
            compare_amounts(&a.output_amount, &b.output_amount)
                .then_with(|| compare_amounts(&b.maximum_total_fee, &a.maximum_total_fee))
                .then_with(|| b.event.pubkey.cmp(&a.event.pubkey))
        })
        .map(|(index, _)| index)
}

/// Maps a MKT-SWP asset identifier's rail segment to the ticker the RFQ
/// comparison card displays. This is bounded-field parsing after the semantic
/// route was already chosen: unknown rails keep the full identifier so the
/// label is never a guess.
fn rail_ticker(asset_id: &str) -> SharedString {
    match asset_id.rsplit(':').next() {
        Some("lightning") => "LN".into(),
        Some("chain") => "BTC".into(),
        Some("liquid") => "L-BTC".into(),
        _ => asset_id.to_owned().into(),
    }
}

/// Projects the session's received quote candidates into the typed
/// `QuoteSet` rendered by `ui::RfqComparisonCard` — the wiring point between
/// the live NIP-MKT session flow and the RFQ comparison card. Candidates
/// whose canonical amounts overflow a `u64` are omitted rather than
/// misrendered; providers stay unrated until receipts-backed reputation
/// lands.
pub fn rfq_quote_set(
    candidates: &[QuoteCandidate],
    network: ui::SwapNetwork,
    now: u64,
) -> Option<ui::QuoteSet> {
    let seconds_until = |expires_at: u64| {
        i64::try_from(expires_at)
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::try_from(now).unwrap_or(i64::MAX))
    };
    let first = candidates
        .iter()
        .find(|candidate| candidate.input_amount.parse::<u64>().is_ok())?;
    let input_sats = first.input_amount.parse::<u64>().ok()?;
    let quotes: Vec<ui::RfqQuote> = candidates
        .iter()
        .filter_map(|candidate| {
            Some(ui::RfqQuote {
                provider: candidate.event.pubkey.clone().into(),
                reputation: ui::RfqReputation::Unrated,
                output_sats: candidate.output_amount.parse().ok()?,
                fee_sats: candidate.maximum_total_fee.parse().ok()?,
                fee_bps: candidate.fee_bps.parse().ok(),
                expires_in_secs: seconds_until(candidate.expires_at),
            })
        })
        .collect();
    if quotes.is_empty() {
        return None;
    }
    Some(ui::QuoteSet {
        from_ticker: rail_ticker(&first.asset_pair.0),
        to_ticker: rail_ticker(&first.asset_pair.1),
        input_sats,
        network,
        quotes,
    })
}

/// One rendered entry inside a per-signer Status lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub event_id: String,
    pub state: String,
    pub created_at: u64,
}

/// One sequence slot in a per-signer Status lane. Missing sequence numbers
/// are displayed gaps; two records at one sequence are an equivocation fork
/// and both entries are retained side by side, never resolved by timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusSlot {
    Filled {
        sequence: u64,
        entries: Vec<StatusEntry>,
    },
    Gap {
        sequence: u64,
    },
}

/// The Status records of one signer, folded into explicit sequence slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLane {
    pub role: ParticipantRole,
    pub pubkey: String,
    pub slots: Vec<StatusSlot>,
    /// Status records whose `seq` tag failed to parse; retained, never
    /// silently dropped.
    pub malformed: Vec<StatusEntry>,
}

/// Presentation fold only: the records were already admitted through the
/// Immortal domain validators.
pub fn fold_status_lanes(
    records: &[Event],
    requester_pubkey: &str,
    provider_pubkey: &str,
) -> Vec<StatusLane> {
    let mut lanes = Vec::new();
    for (role, pubkey) in [
        (ParticipantRole::Requester, requester_pubkey),
        (ParticipantRole::Provider, provider_pubkey),
    ] {
        let mut by_sequence: BTreeMap<u64, Vec<StatusEntry>> = BTreeMap::new();
        let mut malformed = Vec::new();
        for event in records
            .iter()
            .filter(|event| event.kind == MKT_STATUS_KIND && event.pubkey == pubkey)
        {
            let entry = StatusEntry {
                event_id: event.id.clone(),
                state: event
                    .tag_values("state")
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                created_at: event.created_at,
            };
            match event
                .tag_values("seq")
                .next()
                .and_then(|seq| seq.parse::<u64>().ok())
            {
                Some(sequence) => by_sequence.entry(sequence).or_default().push(entry),
                None => malformed.push(entry),
            }
        }
        if by_sequence.is_empty() && malformed.is_empty() {
            continue;
        }
        let mut slots = Vec::new();
        if let Some(highest) = by_sequence.keys().next_back().copied() {
            for sequence in 0..=highest {
                match by_sequence.remove(&sequence) {
                    Some(mut entries) => {
                        entries.sort_by(|a, b| a.event_id.cmp(&b.event_id));
                        slots.push(StatusSlot::Filled { sequence, entries });
                    }
                    None => slots.push(StatusSlot::Gap { sequence }),
                }
            }
        }
        lanes.push(StatusLane {
            role,
            pubkey: pubkey.to_owned(),
            slots,
            malformed,
        });
    }
    lanes
}

/// One Cancel record, projected for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelEntry {
    pub event_id: String,
    pub author: ParticipantRole,
    pub action: String,
    pub reason: String,
}

/// One Close record, projected for rendering. Conflicting Close records
/// remain separate evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseEntry {
    pub event_id: String,
    pub author: ParticipantRole,
    pub outcome: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    AwaitingQuotes,
    QuoteReceived,
    OrderInFlight,
    Active,
    CancelRequested,
    Closed,
}

impl SessionPhase {
    pub fn label(&self) -> &'static str {
        match self {
            Self::AwaitingQuotes => "rfq sent",
            Self::QuoteReceived => "quotes",
            Self::OrderInFlight => "ordered",
            Self::Active => "active",
            Self::CancelRequested => "cancelling",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitOutcome {
    Admitted,
    Replay,
    OtherSession,
}

/// The requester-side session state machine. It owns no transport: callers
/// publish the signed records it returns (wrapped through
/// [`wrap_for_transport`]) and feed unwrapped deliveries back through
/// [`MarketSession::admit_delivery`].
pub struct MarketSession {
    signer: MarketSigner,
    factory: SwapRecordFactory,
    offering_label: String,
    records: Vec<Event>,
    record_ids: BTreeSet<String>,
    deliveries: Vec<SignedRecordDelivery>,
    wrap_ids: Vec<Option<String>>,
    rfq: Event,
    quotes: Vec<QuoteCandidate>,
    order: Option<Event>,
    accepted_quote: Option<QuoteCandidate>,
    provider_contract_received: bool,
    cancel_request_id: Option<String>,
}

impl MarketSession {
    /// Signs and admits the opening RFQ for the selected offering. The RFQ
    /// must be published (via [`MarketSession::replay_wraps`] or
    /// [`wrap_for_transport`]) by the caller.
    pub fn begin(
        signer: MarketSigner,
        offering: &OfferingListing,
        now: u64,
    ) -> Result<Self, String> {
        let config = SwapClientConfig {
            session_id: random_hex_32(),
            requester_pubkey: signer.pubkey().to_owned(),
            provider_pubkey: offering.pubkey.clone(),
            offering_address: format!("39601:{}:{}", offering.pubkey, offering.offering_id),
            provider_route: None,
        };
        let factory = SwapRecordFactory::new(config)
            .map_err(|error| format!("could not start the session: {error}"))?;
        let request = factory
            .rfq(
                now,
                &random_hex_32(),
                now.saturating_add(RFQ_LIFETIME_SECONDS),
                no_spend_submarine_constraints(),
            )
            .map_err(|error| format!("could not construct the RFQ: {error}"))?;
        let rfq = sign_request(&signer, &request)?;
        let mut session = Self {
            signer,
            factory,
            offering_label: offering.offering_id.clone(),
            records: Vec::new(),
            record_ids: BTreeSet::new(),
            deliveries: Vec::new(),
            wrap_ids: Vec::new(),
            rfq: rfq.clone(),
            quotes: Vec::new(),
            order: None,
            accepted_quote: None,
            provider_contract_received: false,
            cancel_request_id: None,
        };
        session.admit_own(rfq, now)?;
        Ok(session)
    }

    pub fn session_id(&self) -> &str {
        &self.factory.config().session_id
    }

    pub fn requester_pubkey(&self) -> &str {
        &self.factory.config().requester_pubkey
    }

    pub fn provider_pubkey(&self) -> &str {
        &self.factory.config().provider_pubkey
    }

    pub fn offering_label(&self) -> &str {
        &self.offering_label
    }

    pub fn signer(&self) -> &MarketSigner {
        &self.signer
    }

    pub fn quotes(&self) -> &[QuoteCandidate] {
        &self.quotes
    }

    pub fn selected_quote(&self, now: u64) -> Option<usize> {
        select_quote(&self.quotes, now)
    }

    pub fn accepted_quote(&self) -> Option<&QuoteCandidate> {
        self.accepted_quote.as_ref()
    }

    pub fn records(&self) -> &[Event] {
        &self.records
    }

    pub fn status_lanes(&self) -> Vec<StatusLane> {
        fold_status_lanes(
            &self.records,
            self.requester_pubkey(),
            self.provider_pubkey(),
        )
    }

    pub fn cancels(&self) -> Vec<CancelEntry> {
        self.records
            .iter()
            .filter(|event| event.kind == MKT_CANCEL_KIND)
            .map(|event| CancelEntry {
                event_id: event.id.clone(),
                author: self.author_role(&event.pubkey),
                action: event
                    .tag_values("action")
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                reason: event
                    .tag_values("reason")
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
            })
            .collect()
    }

    pub fn closes(&self) -> Vec<CloseEntry> {
        self.records
            .iter()
            .filter(|event| event.kind == MKT_CLOSE_KIND)
            .map(|event| CloseEntry {
                event_id: event.id.clone(),
                author: self.author_role(&event.pubkey),
                outcome: event
                    .tag_values("outcome")
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
            })
            .collect()
    }

    fn author_role(&self, pubkey: &str) -> ParticipantRole {
        if pubkey == self.requester_pubkey() {
            ParticipantRole::Requester
        } else {
            ParticipantRole::Provider
        }
    }

    pub fn phase(&self) -> SessionPhase {
        if !self.closes().is_empty() {
            SessionPhase::Closed
        } else if self.cancel_request_id.is_some() {
            SessionPhase::CancelRequested
        } else if self.provider_contract_received {
            SessionPhase::Active
        } else if self.order.is_some() {
            SessionPhase::OrderInFlight
        } else if !self.quotes.is_empty() {
            SessionPhase::QuoteReceived
        } else {
            SessionPhase::AwaitingQuotes
        }
    }

    pub fn can_order(&self, now: u64) -> bool {
        self.order.is_none() && self.selected_quote(now).is_some()
    }

    pub fn can_cancel(&self) -> bool {
        self.order.is_some() && self.cancel_request_id.is_none() && self.closes().is_empty()
    }

    pub fn can_close(&self) -> bool {
        self.effective_cancel_id().is_some()
            && !self
                .closes()
                .iter()
                .any(|close| close.author == ParticipantRole::Requester)
    }

    fn effective_cancel_id(&self) -> Option<String> {
        self.records
            .iter()
            .find(|event| {
                event.kind == MKT_CANCEL_KIND
                    && event.pubkey == *self.provider_pubkey()
                    && event.tag_values("action").next() == Some("effective")
            })
            .map(|event| event.id.clone())
    }

    fn next_created_at(&self, now: u64) -> u64 {
        let latest = self
            .records
            .iter()
            .map(|event| event.created_at)
            .max()
            .unwrap_or(0);
        now.max(latest.saturating_add(1))
    }

    fn admit_own(&mut self, event: Event, observed_at: u64) -> Result<(), String> {
        let raw = serde_json::to_vec(&event)
            .map_err(|error| format!("could not serialize a signed record: {error}"))?;
        let delivery = SignedRecordDelivery::from_locally_signed(raw, observed_at)
            .map_err(|error| format!("own record failed delivery validation: {error}"))?;
        self.push_record(event, delivery, None)
    }

    fn push_record(
        &mut self,
        event: Event,
        delivery: SignedRecordDelivery,
        wrap_id: Option<String>,
    ) -> Result<(), String> {
        if self.records.len() >= MAX_SESSION_RECORDS {
            return Err("the session record history is full".to_owned());
        }
        self.record_ids.insert(event.id.clone());
        self.records.push(event);
        self.deliveries.push(delivery);
        self.wrap_ids.push(wrap_id);
        Ok(())
    }

    /// Folds one unwrapped gift-wrap delivery into the session. The record
    /// was already fully validated (wrap, seal, inner signature, MKT base,
    /// and MKT-SWP profile) by `immortal_client::market::unwrap_mkt_record`.
    pub fn admit_delivery(
        &mut self,
        delivered: &DeliveredMktRecord,
        observed_at: u64,
    ) -> Result<AdmitOutcome, String> {
        let record = delivered.record();
        if record.envelope().session_id != self.session_id() {
            return Ok(AdmitOutcome::OtherSession);
        }
        let event = record.event().clone();
        if self.record_ids.contains(&event.id) {
            return Ok(AdmitOutcome::Replay);
        }
        let expected_sender = if event.pubkey == *self.requester_pubkey() {
            self.requester_pubkey()
        } else if event.pubkey == *self.provider_pubkey() {
            self.provider_pubkey()
        } else {
            return Err("record signer is not a session participant".to_owned());
        };
        if delivered.sender() != expected_sender {
            return Err("gift-wrap sender does not match the record signer".to_owned());
        }
        if event.kind == MKT_QUOTE_KIND {
            if self.quotes.len() >= MAX_QUOTE_CANDIDATES {
                return Err("the quote candidate list is full".to_owned());
            }
            let candidate = QuoteCandidate::from_event(&event, &self.rfq.id)?;
            self.quotes.push(candidate);
        }
        if event.kind == MKT_SWP_SWAP_CONTRACT_KIND && event.pubkey == *self.provider_pubkey() {
            self.provider_contract_received = true;
        }
        let delivery = SignedRecordDelivery::from_delivered(delivered, observed_at)
            .map_err(|error| format!("delivery failed validation: {error}"))?;
        self.push_record(event, delivery, Some(delivered.wrap_event_id().to_owned()))?;
        Ok(AdmitOutcome::Admitted)
    }

    /// Accepts the policy-selected quote: constructs, signs, and admits the
    /// Order, the requester's opening Status, and the requester Swap
    /// Contract. Returns the records to publish, in order.
    pub fn order_selected_quote(&mut self, now: u64) -> Result<Vec<Event>, String> {
        if self.order.is_some() {
            return Err("the session already has an order".to_owned());
        }
        let selected = self
            .selected_quote(now)
            .ok_or_else(|| "no usable quote to order".to_owned())?;
        let quote = self.quotes[selected].clone();
        let order_created_at = self.next_created_at(now);
        let order_request = self
            .factory
            .requester_order(RequesterOrderInput {
                rfq: &self.rfq,
                quote: &quote.event,
                created_at: order_created_at,
                observed_at: now,
                distinct: &random_hex_32(),
                selection: None,
            })
            .map_err(|error| format!("could not construct the order: {error}"))?;
        let order = sign_request(&self.signer, &order_request)?;

        let status_request = self
            .factory
            .status(
                ParticipantRole::Requester,
                order_created_at.saturating_add(1),
                &random_hex_32(),
                &order.id,
                StatusState {
                    sequence: 0,
                    previous: None,
                    base_state: "awaiting_input",
                    swp_state: "requester_verification_passed",
                },
                Map::new(),
            )
            .map_err(|error| format!("could not construct the opening status: {error}"))?;
        let status = sign_request(&self.signer, &status_request)?;

        let mut local_inputs = RequesterContractLocalInputs::for_swap_type(SwapType::Submarine);
        local_inputs.exit_package_commitments = vec![no_spend_submarine_exit_commitment()];
        let contract_value = self
            .factory
            .requester_contract_draft(&self.rfq, &quote.event, &order, now, local_inputs)
            .map_err(|error| format!("could not compose the contract: {error}"))?;
        let contract_request = self
            .factory
            .requester_contract(RequesterContractSigningInput {
                rfq: &self.rfq,
                quote: &quote.event,
                order: &order,
                order_observed_at: now,
                created_at: order_created_at.saturating_add(2),
                distinct: &random_hex_32(),
                contract: contract_value,
            })
            .map_err(|error| format!("could not construct the contract record: {error}"))?;
        let contract = sign_request(&self.signer, &contract_request)?;

        for event in [&order, &status, &contract] {
            self.admit_own(event.clone(), now)?;
        }
        self.order = Some(order.clone());
        self.accepted_quote = Some(quote);
        Ok(vec![order, status, contract])
    }

    /// Constructs, signs, and admits a cancellation request. Returns the
    /// record to publish.
    pub fn request_cancel(&mut self, now: u64) -> Result<Event, String> {
        if !self.can_cancel() {
            return Err("the session cannot be cancelled now".to_owned());
        }
        let order_id = self
            .order
            .as_ref()
            .map(|order| order.id.clone())
            .ok_or_else(|| "the session has no order".to_owned())?;
        let request = self
            .factory
            .cancel(
                ParticipantRole::Requester,
                self.next_created_at(now),
                &random_hex_32(),
                &order_id,
                Cancellation {
                    action: "request",
                    reason: CANCEL_REASON,
                    request_id: None,
                    accepted_id: None,
                },
                json!({ "disposition": "no_funding_authorized" }),
            )
            .map_err(|error| format!("could not construct the cancel request: {error}"))?;
        let cancel = sign_request(&self.signer, &request)?;
        self.admit_own(cancel.clone(), now)?;
        self.cancel_request_id = Some(cancel.id.clone());
        Ok(cancel)
    }

    /// Constructs, signs, and admits the requester's terminal Close after
    /// the provider's effective cancellation, with exact zero-spend loss
    /// accounting. Returns the record to publish.
    pub fn close_after_cancel(&mut self, now: u64) -> Result<Event, String> {
        if !self.can_close() {
            return Err("the session cannot be closed yet".to_owned());
        }
        let effective_cancel_id = self
            .effective_cancel_id()
            .ok_or_else(|| "no effective cancellation exists".to_owned())?;
        let order_id = self
            .order
            .as_ref()
            .map(|order| order.id.clone())
            .ok_or_else(|| "the session has no order".to_owned())?;
        let quote = self
            .accepted_quote
            .as_ref()
            .ok_or_else(|| "the session has no accepted quote".to_owned())?;
        let created_at = self.next_created_at(now);
        let request = self
            .factory
            .close(
                ParticipantRole::Requester,
                created_at,
                &random_hex_32(),
                &order_id,
                CloseOutcome {
                    outcome: "cancelled",
                    terminal_at: created_at,
                },
                json!({
                    "final_state": "cancelled",
                    "external_spend_effects": 0,
                    "loss_classification": "none",
                    "cancel_id": effective_cancel_id,
                    "loss_accounting": {
                        "input_asset_id": quote.asset_pair.0,
                        "output_asset_id": quote.asset_pair.1,
                        "input_committed": "0",
                        "input_recovered": "0",
                        "output_received": "0",
                        "provider_fee_paid": "0",
                        "miner_fee_paid": "0",
                        "lightning_routing_fee_paid": "0",
                        "guarantee_recovery_received": "0",
                        "principal_unresolved": "0",
                        "reservation_released": quote.output_amount,
                        "evidence_refs": [],
                        "unknown_fields": []
                    }
                }),
            )
            .map_err(|error| format!("could not construct the close: {error}"))?;
        let close = sign_request(&self.signer, &request)?;
        self.admit_own(close.clone(), now)?;
        Ok(close)
    }

    /// The session's own signed records in causal order, re-wrapped with
    /// fresh material for replay on (re)connect. Identical inner bytes are
    /// idempotent replay for the counterparty.
    pub fn replay_wraps(&self, now: u64) -> Result<Vec<Event>, String> {
        let mut wraps = Vec::new();
        for event in self
            .records
            .iter()
            .filter(|event| event.pubkey == *self.requester_pubkey())
        {
            for wrapped in wrap_for_transport(event, &self.signer, self.provider_pubkey(), now)? {
                wraps.push(wrapped.event);
            }
        }
        Ok(wraps)
    }

    /// The Immortal requester projection over the bound session (the RFQ,
    /// the accepted quote, and every subsequent record). Unaccepted quote
    /// candidates stay outside the bound history.
    pub fn requester_session_view(&self) -> Result<RequesterSessionView, String> {
        let accepted = self
            .accepted_quote
            .as_ref()
            .ok_or_else(|| "no quote was accepted yet".to_owned())?;
        let mut bound_records = Vec::new();
        let mut bound_deliveries = Vec::new();
        for (event, delivery) in self.records.iter().zip(&self.deliveries) {
            if event.kind == MKT_QUOTE_KIND && event.id != accepted.event.id {
                continue;
            }
            bound_records.push(event.clone());
            bound_deliveries.push(delivery.clone());
        }
        RequesterSessionView::from_signed_records(
            self.factory.config(),
            &bound_records,
            bound_deliveries,
        )
        .map_err(|error| format!("session view failed: {error}"))
    }

    /// Serializes the session for the durable store; the shape is documented
    /// in the module docs.
    pub fn store_document(&self) -> Result<Value, String> {
        let records = self
            .records
            .iter()
            .zip(self.deliveries.iter().zip(&self.wrap_ids))
            .map(|(event, (delivery, wrap_id))| {
                let raw = serde_json::to_vec(event)
                    .map_err(|error| format!("could not serialize a record: {error}"))?;
                Ok(json!({
                    "raw_signed_event_hex": lower_hex(&raw),
                    "observed_at": delivery.observed_at(),
                    "provenance": match delivery.provenance() {
                        DeliveryProvenance::LocallySigned => "locally_signed",
                        DeliveryProvenance::Direct => "direct",
                        DeliveryProvenance::GiftWrap => "gift_wrap",
                    },
                    "wrap_event_id": wrap_id,
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(json!({
            "schema": SESSION_STORE_SCHEMA,
            "session_id": self.session_id(),
            "requester_pubkey": self.requester_pubkey(),
            "provider_pubkey": self.provider_pubkey(),
            "offering_address": self.factory.config().offering_address,
            "records": records,
        }))
    }

    /// Writes the session document atomically under `directory`, returning
    /// the file path.
    pub fn persist(&self, directory: &Path) -> Result<PathBuf, String> {
        let document = self.store_document()?;
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("could not create the session store: {error}"))?;
        let path = directory.join(format!("{}.json", self.session_id()));
        let staged = directory.join(format!("{}.json.tmp", self.session_id()));
        let bytes = serde_json::to_vec_pretty(&document)
            .map_err(|error| format!("could not serialize the session store: {error}"))?;
        std::fs::write(&staged, bytes)
            .map_err(|error| format!("could not stage the session store: {error}"))?;
        std::fs::rename(&staged, &path)
            .map_err(|error| format!("could not commit the session store: {error}"))?;
        Ok(path)
    }
}

fn sign_request(signer: &MarketSigner, request: &MktSigningRequest) -> Result<Event, String> {
    let event = signer.sign(
        request.created_at,
        request.kind,
        request.tags.clone(),
        request.content.clone(),
    );
    request
        .verify_signed(event)
        .map_err(|error| format!("record signing failed verification: {error}"))
}

/// Loads a persisted session document and revalidates every stored record
/// through the Immortal domain before returning the events.
pub fn load_stored_records(path: &Path) -> Result<Vec<Event>, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read the session store: {error}"))?;
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("session store is not JSON: {error}"))?;
    if document.get("schema").and_then(Value::as_str) != Some(SESSION_STORE_SCHEMA) {
        return Err("session store has an unknown schema".to_owned());
    }
    let records = document
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| "session store has no records".to_owned())?;
    let mut events = Vec::new();
    for record in records {
        let hex = record
            .get("raw_signed_event_hex")
            .and_then(Value::as_str)
            .ok_or_else(|| "session store record has no bytes".to_owned())?;
        let raw = decode_hex(hex)?;
        let validated =
            immortal_client::domain::validate_mkt_private_raw(&raw, &swp_profile_support())
                .map_err(|error| format!("stored record failed validation: {error}"))?;
        events.push(validated.event().clone());
    }
    Ok(events)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("stored record hex has an odd length".to_owned());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                _ => Err("stored record hex is invalid".to_owned()),
            };
            Ok((digit(pair[0])? << 4) | digit(pair[1])?)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use immortal_client::domain::MKT_RFQ_KIND;
    use immortal_client::market::unwrap_mkt_record;

    use super::*;

    fn offering(provider: &MarketSigner) -> OfferingListing {
        OfferingListing {
            offering_id: "immortal-no-spend-swaps".to_owned(),
            pubkey: provider.pubkey().to_owned(),
            status: "active".to_owned(),
            profile: "mkt-swp:1".to_owned(),
            provider_address: format!("39600:{}:local", provider.pubkey()),
        }
    }

    fn candidate(output: &str, fee: &str, pubkey: &str, expires_at: u64) -> QuoteCandidate {
        QuoteCandidate {
            event: Event {
                id: format!("{pubkey}-{output}"),
                pubkey: pubkey.to_owned(),
                created_at: 1,
                kind: MKT_QUOTE_KIND,
                tags: Vec::new(),
                content: String::new(),
                sig: String::new(),
            },
            quote_class: "firm".to_owned(),
            reservation: "soft".to_owned(),
            input_amount: "100000".to_owned(),
            output_amount: output.to_owned(),
            maximum_total_fee: fee.to_owned(),
            fee_bps: "9800".to_owned(),
            asset_pair: ("a".to_owned(), "b".to_owned()),
            expires_at,
            previous: None,
        }
    }

    #[test]
    fn quote_selection_prefers_output_then_fee_then_pubkey() {
        let now = 100;
        let candidates = vec![
            candidate("2000", "50", "cc", 200),
            candidate("10000", "90", "bb", 200),
            candidate("10000", "40", "dd", 200),
            candidate("10000", "40", "aa", 200),
        ];
        // Highest output wins over lowest fee.
        assert_eq!(select_quote(&candidates[..2], now), Some(1));
        // Among equal outputs the lowest fee wins.
        assert_eq!(select_quote(&candidates[1..3], now), Some(1));
        // Among equal outputs and fees the lowest pubkey wins.
        assert_eq!(select_quote(&candidates, now), Some(3));
        // Amounts compare numerically, not lexicographically.
        let numeric = vec![
            candidate("900", "1", "aa", 200),
            candidate("1000", "1", "bb", 200),
        ];
        assert_eq!(select_quote(&numeric, now), Some(1));
    }

    #[test]
    fn quote_selection_skips_expired_candidates() {
        let now = 100;
        let candidates = vec![
            candidate("10000", "40", "aa", 90),
            candidate("2000", "50", "bb", 200),
        ];
        assert_eq!(select_quote(&candidates, now), Some(1));
        assert_eq!(select_quote(&candidates[..1], now), None);
    }

    #[test]
    fn the_rfq_card_adapter_agrees_with_the_selection_policy() {
        let now = 100;
        let mut candidates = vec![
            candidate("10000", "90", "bb", 200),
            candidate("10000", "40", "aa", 160),
            candidate("12000", "40", "cc", 90),
        ];
        candidates[0].asset_pair = (
            "swp:1:bip122:00000000000000000000000000000000:btc:lightning".to_owned(),
            "swp:1:bip122:00000000000000000000000000000000:btc:chain".to_owned(),
        );
        let quote_set = rfq_quote_set(&candidates, ui::SwapNetwork::Regtest, now)
            .expect("the candidates project to a quote set");
        assert_eq!(quote_set.from_ticker.as_ref(), "LN");
        assert_eq!(quote_set.to_ticker.as_ref(), "BTC");
        assert_eq!(quote_set.input_sats, 100_000);
        assert_eq!(quote_set.quotes.len(), 3);
        assert_eq!(quote_set.quotes[0].expires_in_secs, 100);
        assert_eq!(quote_set.quotes[2].expires_in_secs, -10);
        // The card's best highlight lands on the same quote the session
        // ordering policy would accept.
        assert_eq!(quote_set.best(), select_quote(&candidates, now));
        assert_eq!(quote_set.best(), Some(1));
    }

    #[test]
    fn the_rfq_card_adapter_omits_unrenderable_amounts() {
        let now = 100;
        let overflowing = "99999999999999999999999999";
        let candidates = vec![
            candidate(overflowing, "40", "aa", 200),
            candidate("10000", "90", "bb", 200),
        ];
        let quote_set = rfq_quote_set(&candidates, ui::SwapNetwork::Regtest, now)
            .expect("one renderable candidate remains");
        assert_eq!(quote_set.quotes.len(), 1);
        assert_eq!(quote_set.quotes[0].provider.as_ref(), "bb");
        assert!(rfq_quote_set(&candidates[..1], ui::SwapNetwork::Regtest, now).is_none());
    }

    fn status_event(signer: &MarketSigner, created_at: u64, sequence: &str, state: &str) -> Event {
        signer.sign(
            created_at,
            MKT_STATUS_KIND,
            vec![
                immortal_client::domain::Tag::new(vec!["seq".into(), sequence.into()]),
                immortal_client::domain::Tag::new(vec!["state".into(), state.into()]),
            ],
            String::new(),
        )
    }

    #[test]
    fn status_lanes_show_gaps_and_retain_forks() {
        let requester = MarketSigner::from_secret_bytes([3; 32]).expect("test key is valid");
        let provider = MarketSigner::from_secret_bytes([4; 32]).expect("test key is valid");
        let records = vec![
            status_event(&requester, 10, "0", "awaiting_input"),
            status_event(&provider, 20, "0", "accepted"),
            status_event(&provider, 40, "2", "executing"),
            // Same sequence twice from one signer: an equivocation fork.
            status_event(&provider, 30, "2", "failed"),
        ];
        let lanes = fold_status_lanes(&records, requester.pubkey(), provider.pubkey());
        assert_eq!(lanes.len(), 2);
        assert_eq!(lanes[0].role, ParticipantRole::Requester);
        assert_eq!(
            lanes[0].slots,
            vec![StatusSlot::Filled {
                sequence: 0,
                entries: vec![StatusEntry {
                    event_id: records[0].id.clone(),
                    state: "awaiting_input".to_owned(),
                    created_at: 10,
                }],
            }]
        );
        let provider_lane = &lanes[1];
        assert_eq!(provider_lane.role, ParticipantRole::Provider);
        assert_eq!(provider_lane.slots.len(), 3);
        assert!(matches!(
            provider_lane.slots[1],
            StatusSlot::Gap { sequence: 1 }
        ));
        let StatusSlot::Filled { entries, .. } = &provider_lane.slots[2] else {
            panic!("sequence 2 must be filled");
        };
        // Both fork entries are retained, ordered by event ID rather than
        // resolved by timestamp.
        assert_eq!(entries.len(), 2);
        let mut expected: Vec<String> = vec![records[2].id.clone(), records[3].id.clone()];
        expected.sort();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.event_id.clone())
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn malformed_sequences_are_retained_not_dropped() {
        let provider = MarketSigner::from_secret_bytes([4; 32]).expect("test key is valid");
        let records = vec![status_event(&provider, 20, "not-a-number", "accepted")];
        let lanes = fold_status_lanes(&records, &"11".repeat(32), provider.pubkey());
        assert_eq!(lanes.len(), 1);
        assert!(lanes[0].slots.is_empty());
        assert_eq!(lanes[0].malformed.len(), 1);
    }

    #[test]
    fn session_rfq_round_trips_the_gift_wrap_and_domain_validators() {
        let requester = throwaway_session_signer().expect("session key generates");
        let provider = MarketSigner::from_secret_bytes([7; 32]).expect("test key is valid");
        let session = MarketSession::begin(requester, &offering(&provider), 1_700_000_000)
            .expect("session begins with a valid fixture RFQ");
        assert_eq!(session.phase(), SessionPhase::AwaitingQuotes);
        assert_eq!(session.records().len(), 1);
        assert_eq!(session.records()[0].kind, MKT_RFQ_KIND);

        let wraps = session
            .replay_wraps(1_700_000_000)
            .expect("the RFQ wraps for transport");
        // One counterparty copy and one sender-recovery copy.
        assert_eq!(wraps.len(), 2);
        let delivered_to_provider = unwrap_mkt_record(&wraps[0], &provider, &swp_profile_support())
            .expect("the provider copy unwraps and validates");
        assert_eq!(
            delivered_to_provider.record().event().id,
            session.records()[0].id
        );
        assert_eq!(
            delivered_to_provider.record().envelope().session_id,
            session.session_id()
        );
        let recovery = unwrap_mkt_record(&wraps[1], session.signer(), &swp_profile_support())
            .expect("the sender-recovery copy unwraps and validates");
        assert_eq!(recovery.record().event().id, session.records()[0].id);
        assert_eq!(
            recovery.record().raw_signed_event(),
            delivered_to_provider.record().raw_signed_event()
        );
    }

    #[test]
    fn persisted_sessions_reload_through_the_domain_validators() {
        let requester = throwaway_session_signer().expect("session key generates");
        let provider = MarketSigner::from_secret_bytes([7; 32]).expect("test key is valid");
        let session = MarketSession::begin(requester, &offering(&provider), 1_700_000_000)
            .expect("session begins");
        let directory = std::env::temp_dir().join(format!(
            "omega-market-session-store-{}",
            session.session_id()
        ));
        let path = session.persist(&directory).expect("session persists");
        let events = load_stored_records(&path).expect("stored records reload and validate");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, session.records()[0].id);
        std::fs::remove_dir_all(&directory).expect("test store directory is removable");
    }
}
