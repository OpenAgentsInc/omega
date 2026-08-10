//! Event-only settlement-receipt verification and append-only ledger projection.
//!
//! Immortal owns the wire format and chain verifier. Omega projects only a
//! successfully verified, provider-signed claim into the trading ledger; the
//! projection deliberately does not describe the claim as external settlement
//! proof or as relay/provider custody.
//! Policy record: OMEGA-DELTA-0266.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use immortal_client::domain::{
    Event, MKT_SWP_SETTLEMENT_RECEIPT_KIND, MktProviderKeyChain, MktReceiptChainErrorCode,
    MktSettlementReceipt, verify_mkt_receipt_chain_parts,
    verify_mkt_receipt_chain_parts_with_provider_keys,
};
use serde::Serialize;
use serde_json::{Value, json};
use trading_ledger::{
    AssetId, LedgerAccount, LedgerEntry, LedgerEntryDraft, LedgerEntryKind, LedgerPosting,
    LedgerStore,
};

pub const RECEIPT_EXPORT_SCHEMA: &str = "omega.market.receipt-export.v1";
pub const RECEIPT_LEDGER_SCHEMA: &str = "omega.market.receipt-ledger.v1";
pub const RECEIPT_LEDGER_STRATEGY: &str = "mkt_swp_receipts";
pub const RECEIPT_EXPORT_DIRECTORY: &str = "market_receipts";

/// Omega's exact confidence boundary for one signed receipt event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptVerification {
    Incomplete {
        receipt_event_id: String,
        detail: String,
    },
    Invalid {
        receipt_event_id: String,
        detail: String,
    },
    /// The event-only chain verifies. This remains a provider claim and is
    /// never presented as proof of native-rail settlement.
    ProviderSigned {
        receipt_event_id: String,
        receipt: MktSettlementReceipt,
    },
}

impl ReceiptVerification {
    pub fn receipt_event_id(&self) -> &str {
        match self {
            Self::Incomplete {
                receipt_event_id, ..
            }
            | Self::Invalid {
                receipt_event_id, ..
            }
            | Self::ProviderSigned {
                receipt_event_id, ..
            } => receipt_event_id,
        }
    }

    pub fn receipt_id(&self) -> Option<&str> {
        match self {
            Self::ProviderSigned { receipt, .. } => Some(&receipt.receipt_id),
            _ => None,
        }
    }
}

/// Verifies every receipt without consulting relay or provider state. Missing
/// signed links remain `Incomplete`; they are never inferred to be failure or
/// settlement.
pub fn verify_receipts(records: &[Event]) -> Vec<ReceiptVerification> {
    verify_receipts_inner(records, None)
}

pub fn verify_receipts_with_provider_keys(
    records: &[Event],
    provider_keys: &MktProviderKeyChain,
) -> Vec<ReceiptVerification> {
    verify_receipts_inner(records, Some(provider_keys))
}

fn verify_receipts_inner(
    records: &[Event],
    provider_keys: Option<&MktProviderKeyChain>,
) -> Vec<ReceiptVerification> {
    let mut receipts = records
        .iter()
        .filter(|event| event.kind == MKT_SWP_SETTLEMENT_RECEIPT_KIND)
        .collect::<Vec<_>>();
    receipts.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    receipts
        .into_iter()
        .map(|receipt_event| {
            let intent = referenced_event(records, receipt_event, "intent");
            let acknowledgment = referenced_event(records, receipt_event, "ack");
            let quote = referenced_event(records, receipt_event, "quote");
            let outcome = referenced_event(records, receipt_event, "outcome");
            let confirmation = referenced_event(records, receipt_event, "client-confirmation");
            let verified = match provider_keys {
                Some(provider_keys) => verify_mkt_receipt_chain_parts_with_provider_keys(
                    receipt_event,
                    intent,
                    acknowledgment,
                    quote,
                    outcome,
                    confirmation,
                    provider_keys,
                ),
                None => verify_mkt_receipt_chain_parts(
                    receipt_event,
                    intent,
                    acknowledgment,
                    quote,
                    outcome,
                    confirmation,
                ),
            };
            match verified {
                Ok(receipt) => ReceiptVerification::ProviderSigned {
                    receipt_event_id: receipt_event.id.clone(),
                    receipt,
                },
                Err(error) if error.code == MktReceiptChainErrorCode::Incomplete => {
                    ReceiptVerification::Incomplete {
                        receipt_event_id: receipt_event.id.clone(),
                        detail: error.detail,
                    }
                }
                Err(error) => ReceiptVerification::Invalid {
                    receipt_event_id: receipt_event.id.clone(),
                    detail: error.detail,
                },
            }
        })
        .collect()
}

fn reference_id<'a>(event: &'a Event, marker: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some("e")
            && values.get(3).map(String::as_str) == Some(marker))
        .then(|| values.get(1).map(String::as_str))
        .flatten()
    })
}

fn referenced_event<'a>(records: &'a [Event], event: &Event, marker: &str) -> Option<&'a Event> {
    let id = reference_id(event, marker)?;
    records.iter().find(|candidate| candidate.id == id)
}

/// Atomically appends all legs and fees of one verified receipt. Exact replay
/// returns the existing rows; any conflicting row rolls back the whole claim.
pub fn persist_verified_receipt(
    store: &LedgerStore,
    requester_pubkey: &str,
    provider_pubkey: &str,
    verification: &ReceiptVerification,
) -> Result<Vec<LedgerEntry>, String> {
    let ReceiptVerification::ProviderSigned {
        receipt_event_id,
        receipt,
    } = verification
    else {
        return Err("only a verified provider-signed receipt can enter the ledger".to_owned());
    };
    let drafts =
        receipt_ledger_drafts(receipt_event_id, receipt, requester_pubkey, provider_pubkey)?;
    store
        .append_batch(drafts)
        .map_err(|error| format!("could not append verified receipt: {error}"))
}

pub fn receipt_ledger_drafts(
    receipt_event_id: &str,
    receipt: &MktSettlementReceipt,
    requester_pubkey: &str,
    provider_pubkey: &str,
) -> Result<Vec<LedgerEntryDraft>, String> {
    let occurred_at_ms = i64::try_from(receipt.finished_at)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000))
        .ok_or_else(|| "receipt finish time exceeds the ledger range".to_owned())?;
    let mut drafts = Vec::with_capacity(receipt.legs.len() + receipt.fees.len());
    for leg in &receipt.legs {
        let amount = ledger_amount(&leg.net_amount, "leg net amount")?;
        let (requester_amount, provider_amount) = match leg.direction.as_str() {
            "provider-receives" => (-amount, amount),
            "provider-sends" => (amount, -amount),
            _ => return Err("verified receipt contains an unknown leg direction".to_owned()),
        };
        let asset = AssetId::new(leg.asset_id.clone())
            .map_err(|error| format!("receipt leg names an invalid ledger asset: {error}"))?;
        let mut draft = LedgerEntryDraft::new(
            format!("mkt-receipt:{}:leg:{}", receipt.receipt_id, leg.leg_id),
            occurred_at_ms,
            RECEIPT_LEDGER_STRATEGY,
            LedgerEntryKind::Fill,
        );
        draft.postings = vec![
            LedgerPosting::new(
                participant_account("requester", requester_pubkey),
                requester_amount,
                asset.clone(),
            ),
            LedgerPosting::new(
                participant_account("provider", provider_pubkey),
                provider_amount,
                asset,
            ),
        ];
        draft.metadata = receipt_metadata(
            receipt_event_id,
            receipt,
            "leg",
            &leg.leg_id,
            json!({
                "asset_id": leg.asset_id,
                "rail": leg.rail,
                "direction": leg.direction,
                "gross_amount": leg.gross_amount,
                "net_amount": leg.net_amount,
            }),
        );
        drafts.push(draft);
    }
    for fee in &receipt.fees {
        if fee.payer_role == fee.recipient_role {
            return Err(
                "receipt fee payer and recipient cannot share one ledger account".to_owned(),
            );
        }
        let amount = ledger_amount(&fee.amount, "fee amount")?;
        let asset = AssetId::new(fee.asset_id.clone())
            .map_err(|error| format!("receipt fee names an invalid ledger asset: {error}"))?;
        let mut draft = LedgerEntryDraft::new(
            format!("mkt-receipt:{}:fee:{}", receipt.receipt_id, fee.fee_id),
            occurred_at_ms,
            RECEIPT_LEDGER_STRATEGY,
            LedgerEntryKind::Fee,
        );
        draft.postings = vec![
            LedgerPosting::new(
                role_account(&fee.payer_role, requester_pubkey, provider_pubkey)?,
                -amount,
                asset.clone(),
            ),
            LedgerPosting::new(
                role_account(&fee.recipient_role, requester_pubkey, provider_pubkey)?,
                amount,
                asset,
            ),
        ];
        draft.metadata = receipt_metadata(
            receipt_event_id,
            receipt,
            "fee",
            &fee.fee_id,
            json!({
                "asset_id": fee.asset_id,
                "rail": fee.rail,
                "amount": fee.amount,
                "payer_role": fee.payer_role,
                "recipient_role": fee.recipient_role,
            }),
        );
        drafts.push(draft);
    }
    if drafts.is_empty() {
        return Err("verified receipt produced no ledger entries".to_owned());
    }
    Ok(drafts)
}

fn ledger_amount(value: &str, label: &str) -> Result<i64, String> {
    let amount = value
        .parse::<i64>()
        .map_err(|_| format!("receipt {label} exceeds the ledger range"))?;
    if amount <= 0 {
        return Err(format!(
            "receipt {label} must be positive for ledger posting"
        ));
    }
    Ok(amount)
}

fn participant_account(role: &str, pubkey: &str) -> LedgerAccount {
    LedgerAccount::MarketParticipant {
        role: role.to_owned(),
        participant: pubkey.to_owned(),
    }
}

fn role_account(
    role: &str,
    requester_pubkey: &str,
    provider_pubkey: &str,
) -> Result<LedgerAccount, String> {
    match role {
        "requester" => Ok(participant_account(role, requester_pubkey)),
        "provider" => Ok(participant_account(role, provider_pubkey)),
        "external" => Ok(participant_account(role, "external")),
        _ => Err("verified receipt contains an unknown fee role".to_owned()),
    }
}

fn receipt_metadata(
    receipt_event_id: &str,
    receipt: &MktSettlementReceipt,
    item_type: &str,
    item_id: &str,
    claim: Value,
) -> Value {
    json!({
        "schema": RECEIPT_LEDGER_SCHEMA,
        "verification": "provider-signed",
        "receipt_event_id": receipt_event_id,
        "receipt_id": receipt.receipt_id,
        "intent_event_id": receipt.intent_event_id,
        "acknowledgment_event_id": receipt.acknowledgment_event_id,
        "quote_event_id": receipt.quote_event_id,
        "outcome_event_id": receipt.outcome_event_id,
        "client_confirmation_event_id": receipt.client_confirmation_event_id,
        "outcome": receipt.outcome,
        "item_type": item_type,
        "item_id": item_id,
        "claim": claim,
    })
}

#[derive(Serialize)]
struct ReceiptExportBundle<'a> {
    schema: &'static str,
    verification: &'static str,
    receipt_id: &'a str,
    receipt_event_id: &'a str,
    provider_network_events: &'a [Event],
    signed_events: Vec<&'a Event>,
    ledger_entries: &'a [LedgerEntry],
}

/// Exports the exact signed proof chain and its hash-chained ledger rows. An
/// existing byte-identical export is an idempotent success; another file is
/// never overwritten.
pub fn export_verified_receipt(
    directory: &Path,
    records: &[Event],
    provider_network_events: &[Event],
    verification: &ReceiptVerification,
    ledger_entries: &[LedgerEntry],
) -> Result<PathBuf, String> {
    let ReceiptVerification::ProviderSigned {
        receipt_event_id,
        receipt,
    } = verification
    else {
        return Err("only a verified provider-signed receipt can be exported".to_owned());
    };
    let receipt_event = records
        .iter()
        .find(|event| event.id == *receipt_event_id)
        .ok_or_else(|| "receipt event is absent from the retained session".to_owned())?;
    let mut signed_events = Vec::with_capacity(6);
    for marker in ["intent", "ack", "quote", "outcome"] {
        signed_events.push(
            referenced_event(records, receipt_event, marker)
                .ok_or_else(|| format!("receipt export is missing signed {marker} event"))?,
        );
    }
    if reference_id(receipt_event, "client-confirmation").is_some() {
        signed_events.push(
            referenced_event(records, receipt_event, "client-confirmation")
                .ok_or_else(|| "receipt export is missing client confirmation".to_owned())?,
        );
    }
    signed_events.push(receipt_event);
    let bundle = ReceiptExportBundle {
        schema: RECEIPT_EXPORT_SCHEMA,
        verification: "provider-signed",
        receipt_id: &receipt.receipt_id,
        receipt_event_id,
        provider_network_events,
        signed_events,
        ledger_entries,
    };
    let bytes = serde_json::to_vec_pretty(&bundle)
        .map_err(|error| format!("could not serialize receipt export: {error}"))?;
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create receipt export directory: {error}"))?;
    let path = directory.join(format!("{}.json", receipt.receipt_id));
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(&bytes)
                .map_err(|error| format!("could not write receipt export: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("could not sync receipt export: {error}"))?;
            Ok(path)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let existing = std::fs::read(&path).map_err(|read_error| {
                format!("could not read existing receipt export: {read_error}")
            })?;
            if existing == bytes {
                Ok(path)
            } else {
                Err("receipt export path already contains different bytes".to_owned())
            }
        }
        Err(error) => Err(format!("could not create receipt export: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use immortal_client::domain::{
        MKT_CLOSE_KIND, MKT_HARDENING_PROTOCOL_REVISION, MKT_HARDENING_SCHEMA,
        MKT_KEY_ROTATION_SCHEMA, MKT_NETWORK_VERSION, MKT_ORDER_KIND, MKT_QUOTE_KIND,
        MKT_RECEIPT_SCHEMA, MKT_RECEIPT_VERSION, MKT_SWP_INTENT_ACK_KIND,
        MKT_SWP_KEY_ROTATION_KIND, MKT_SWP_PROFILE_ID, MKT_SWP_PROFILE_VERSION, MktKeyRotation,
        MktReceiptFee, MktReceiptLeg, Tag, canonical_mkt_key_rotation_content,
        canonical_mkt_receipt_content, mkt_key_rotation_id, mkt_receipt_id,
    };
    use immortal_client::market::MarketSigner;

    use crate::network_transport::ProviderNetworkState;

    use super::*;

    const SESSION_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn claim() -> MktSettlementReceipt {
        let mut receipt = MktSettlementReceipt {
            schema: MKT_RECEIPT_SCHEMA.to_owned(),
            version: MKT_RECEIPT_VERSION,
            receipt_id: String::new(),
            intent_event_id: "11".repeat(32),
            acknowledgment_event_id: "22".repeat(32),
            quote_event_id: "33".repeat(32),
            outcome_event_id: "44".repeat(32),
            client_confirmation_event_id: None,
            outcome: "completed".to_owned(),
            failure_code: None,
            started_at: 1_000,
            finished_at: 1_090,
            legs: vec![
                MktReceiptLeg {
                    leg_id: "source".to_owned(),
                    asset_id: "swp:1:bip122:00000000000000000000000000000000:btc:chain".to_owned(),
                    rail: "bitcoin".to_owned(),
                    direction: "provider-receives".to_owned(),
                    gross_amount: "100000".to_owned(),
                    net_amount: "100000".to_owned(),
                },
                MktReceiptLeg {
                    leg_id: "destination".to_owned(),
                    asset_id: "swp:1:bip122:00000000000000000000000000000000:btc:lightning"
                        .to_owned(),
                    rail: "lightning".to_owned(),
                    direction: "provider-sends".to_owned(),
                    gross_amount: "99000".to_owned(),
                    net_amount: "99000".to_owned(),
                },
            ],
            fees: vec![MktReceiptFee {
                fee_id: "provider-fee".to_owned(),
                asset_id: "swp:1:bip122:00000000000000000000000000000000:btc:chain".to_owned(),
                rail: "bitcoin".to_owned(),
                amount: "1000".to_owned(),
                payer_role: "requester".to_owned(),
                recipient_role: "provider".to_owned(),
            }],
        };
        receipt.receipt_id = mkt_receipt_id(&receipt).expect("receipt digest");
        receipt
    }

    #[test]
    fn verified_receipt_projects_atomic_multi_asset_ledger_group() {
        let receipt = claim();
        let verification = ReceiptVerification::ProviderSigned {
            receipt_event_id: "55".repeat(32),
            receipt: receipt.clone(),
        };
        let store = LedgerStore::in_memory().expect("ledger");
        let entries =
            persist_verified_receipt(&store, &"66".repeat(32), &"77".repeat(32), &verification)
                .expect("verified receipt persists");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[2].sequence, 3);
        assert_eq!(entries[0].metadata["receipt_id"], receipt.receipt_id);
        assert!(entries.iter().all(|entry| entry.postings.len() == 2));
        let replay =
            persist_verified_receipt(&store, &"66".repeat(32), &"77".repeat(32), &verification)
                .expect("exact replay is idempotent");
        assert_eq!(replay, entries);
        assert_eq!(
            store
                .entries(&trading_ledger::LedgerQuery::default())
                .expect("entries")
                .len(),
            3
        );
    }

    #[test]
    fn unverified_and_out_of_range_receipts_never_enter_ledger() {
        let store = LedgerStore::in_memory().expect("ledger");
        let incomplete = ReceiptVerification::Incomplete {
            receipt_event_id: "55".repeat(32),
            detail: "missing signed quote event".to_owned(),
        };
        assert!(persist_verified_receipt(&store, "requester", "provider", &incomplete).is_err());
        let mut receipt = claim();
        receipt.legs[0].net_amount = u64::MAX.to_string();
        let verified = ReceiptVerification::ProviderSigned {
            receipt_event_id: "55".repeat(32),
            receipt,
        };
        assert!(persist_verified_receipt(&store, "requester", "provider", &verified).is_err());
        assert!(
            store
                .entries(&trading_ledger::LedgerQuery::default())
                .expect("entries")
                .is_empty()
        );
    }

    #[test]
    fn exact_events_verify_export_and_missing_links_stay_incomplete() {
        let records = signed_chain(false);
        let verification = verify_receipts(&records)
            .into_iter()
            .next()
            .expect("receipt projection");
        let ReceiptVerification::ProviderSigned { receipt, .. } = &verification else {
            panic!("canonical chain must verify");
        };
        let store = LedgerStore::in_memory().expect("ledger");
        let entries = persist_verified_receipt(
            &store,
            &records[1].pubkey,
            &records[0].pubkey,
            &verification,
        )
        .expect("receipt persists");
        let directory = std::env::temp_dir().join(format!(
            "omega-market-receipt-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let path = export_verified_receipt(&directory, &records, &[], &verification, &entries)
            .expect("receipt exports");
        assert_eq!(
            export_verified_receipt(&directory, &records, &[], &verification, &entries)
                .expect("identical export is idempotent"),
            path
        );
        let document: Value =
            serde_json::from_slice(&std::fs::read(&path).expect("receipt export reads"))
                .expect("receipt export is JSON");
        assert_eq!(document["schema"], RECEIPT_EXPORT_SCHEMA);
        assert_eq!(document["receipt_id"], receipt.receipt_id);
        assert_eq!(document["signed_events"].as_array().map(Vec::len), Some(5));
        assert_eq!(document["ledger_entries"].as_array().map(Vec::len), Some(3));
        std::fs::remove_dir_all(directory).expect("receipt export directory removes");

        let missing_quote = records
            .iter()
            .filter(|event| event.kind != MKT_QUOTE_KIND)
            .cloned()
            .collect::<Vec<_>>();
        assert!(matches!(
            verify_receipts(&missing_quote).as_slice(),
            [ReceiptVerification::Incomplete { .. }]
        ));

        let broken = signed_chain(true);
        assert!(matches!(
            verify_receipts(&broken).as_slice(),
            [ReceiptVerification::Invalid { .. }]
        ));
    }

    #[test]
    fn receipt_verification_honors_provider_rotation_mid_chain() {
        let mut records = signed_chain(false);
        let provider = signer(2);
        let successor = signer(3);
        let original = records[4].clone();
        records[4] = successor.sign(
            original.created_at,
            original.kind,
            original.tags,
            original.content,
        );
        assert!(matches!(
            verify_receipts(&records).as_slice(),
            [ReceiptVerification::Invalid { .. }]
        ));
        let rotation = key_rotation(&provider, &successor, 1_050, 1_095);
        let network = ProviderNetworkState::verify(provider.pubkey(), &[rotation])
            .expect("provider rotation chain");
        assert!(matches!(
            verify_receipts_with_provider_keys(&records, network.key_chain()).as_slice(),
            [ReceiptVerification::ProviderSigned { .. }]
        ));
    }

    fn signed_chain(broken_outcome: bool) -> Vec<Event> {
        let requester = signer(1);
        let provider = signer(2);
        let quote = provider.sign(
            990,
            MKT_QUOTE_KIND,
            common_tags("11", requester.pubkey(), "requester", "MKT-SWP Quote"),
            v1_content(),
        );
        let mut order_tags = common_tags("22", provider.pubkey(), "provider", "MKT-SWP Order");
        order_tags.extend([
            reference(&quote.id, "quote"),
            pair("intent", "effectful"),
            pair("nonce", &"cc".repeat(32)),
            pair("nonce_at", "1000"),
            pair("response", requester.pubkey()),
        ]);
        let order = requester.sign(
            1_000,
            MKT_ORDER_KIND,
            order_tags,
            json!({
                "schema": MKT_HARDENING_SCHEMA,
                "protocol_rev": MKT_HARDENING_PROTOCOL_REVISION,
                "profile": MKT_SWP_PROFILE_ID,
                "profile_version": MKT_SWP_PROFILE_VERSION,
                "session_id": SESSION_ID,
                "intent": {
                    "idempotency_key": "22".repeat(32),
                    "nonce": "cc".repeat(32),
                    "nonce_at": 1000,
                    "response_pubkey": requester.pubkey(),
                    "ack_deadline_seconds": 30,
                    "outcome_deadline_seconds": 300,
                },
                "mkt_swp": {},
            })
            .to_string(),
        );
        let mut acknowledgment_tags = common_tags(
            "33",
            requester.pubkey(),
            "requester",
            "MKT-SWP Intent Acknowledgment",
        );
        acknowledgment_tags.extend([
            reference(&order.id, "intent"),
            pair("ack", "accepted"),
            pair("response", requester.pubkey()),
            pair("expiration", "2000"),
        ]);
        let acknowledgment = provider.sign(
            1_001,
            MKT_SWP_INTENT_ACK_KIND,
            acknowledgment_tags,
            json!({
                "schema": MKT_HARDENING_SCHEMA,
                "protocol_rev": MKT_HARDENING_PROTOCOL_REVISION,
                "profile": MKT_SWP_PROFILE_ID,
                "profile_version": MKT_SWP_PROFILE_VERSION,
                "session_id": SESSION_ID,
                "ack": {
                    "intent_event_id": order.id,
                    "idempotency_key": "22".repeat(32),
                    "disposition": "accepted",
                    "accepted_at": 1001,
                    "error_code": null,
                },
            })
            .to_string(),
        );
        let outcome_order_id = if broken_outcome {
            "ff".repeat(32)
        } else {
            order.id.clone()
        };
        let mut outcome_tags = common_tags("44", requester.pubkey(), "requester", "MKT-SWP Close");
        outcome_tags.extend([
            reference(&outcome_order_id, "order"),
            pair("outcome", "completed"),
            pair("terminal_at", "1090"),
        ]);
        let outcome = provider.sign(1_090, MKT_CLOSE_KIND, outcome_tags, v1_content());
        let mut receipt = claim();
        receipt.intent_event_id = order.id.clone();
        receipt.acknowledgment_event_id = acknowledgment.id.clone();
        receipt.quote_event_id = quote.id.clone();
        receipt.outcome_event_id = outcome.id.clone();
        receipt.receipt_id = String::new();
        receipt.receipt_id = mkt_receipt_id(&receipt).expect("receipt digest");
        let mut receipt_tags = common_tags(
            &receipt.receipt_id,
            requester.pubkey(),
            "requester",
            "MKT-SWP Settlement Receipt",
        );
        receipt_tags.extend([
            reference(&receipt.intent_event_id, "intent"),
            reference(&receipt.acknowledgment_event_id, "ack"),
            reference(&receipt.quote_event_id, "quote"),
            reference(&receipt.outcome_event_id, "outcome"),
            pair("outcome", &receipt.outcome),
            pair("receipt", "1"),
        ]);
        let receipt_event = provider.sign(
            1_100,
            MKT_SWP_SETTLEMENT_RECEIPT_KIND,
            receipt_tags,
            canonical_mkt_receipt_content(SESSION_ID, &receipt).expect("canonical receipt content"),
        );
        vec![quote, order, acknowledgment, outcome, receipt_event]
    }

    fn signer(secret: u8) -> MarketSigner {
        MarketSigner::from_secret_bytes([secret; 32]).expect("test key")
    }

    fn key_rotation(
        old: &MarketSigner,
        new: &MarketSigner,
        created_at: u64,
        effective_at: u64,
    ) -> Event {
        let mut rotation = MktKeyRotation {
            schema: MKT_KEY_ROTATION_SCHEMA.to_owned(),
            version: MKT_NETWORK_VERSION,
            rotation_id: String::new(),
            provider_id: old.pubkey().to_owned(),
            generation: 1,
            previous_rotation_event_id: None,
            old_pubkey: old.pubkey().to_owned(),
            new_pubkey: new.pubkey().to_owned(),
            effective_at,
        };
        rotation.rotation_id = mkt_key_rotation_id(&rotation).expect("rotation digest");
        old.sign(
            created_at,
            MKT_SWP_KEY_ROTATION_KIND,
            vec![
                pair("d", &rotation.rotation_id),
                pair("provider", &rotation.provider_id),
                pair("generation", "1"),
                pair("effective_at", &effective_at.to_string()),
                Tag::new(vec![
                    "p".into(),
                    new.pubkey().to_owned(),
                    String::new(),
                    "successor".into(),
                ]),
                pair("alt", "MKT Provider Key Rotation"),
            ],
            canonical_mkt_key_rotation_content(&rotation).expect("canonical rotation"),
        )
    }

    fn common_tags(digit: &str, counterparty: &str, role: &str, alt: &str) -> Vec<Tag> {
        vec![
            pair(
                "d",
                &if digit.len() == 64 {
                    digit.to_owned()
                } else {
                    digit.repeat(32)
                },
            ),
            pair("session", SESSION_ID),
            Tag::new(vec![
                "profile".into(),
                MKT_SWP_PROFILE_ID.into(),
                MKT_SWP_PROFILE_VERSION.to_string(),
            ]),
            Tag::new(vec![
                "p".into(),
                counterparty.to_owned(),
                String::new(),
                role.to_owned(),
            ]),
            pair("alt", alt),
        ]
    }

    fn v1_content() -> String {
        json!({
            "schema": "openagents.mkt.v1",
            "profile": MKT_SWP_PROFILE_ID,
            "profile_version": MKT_SWP_PROFILE_VERSION,
            "session_id": SESSION_ID,
            "mkt_swp": {},
        })
        .to_string()
    }

    fn pair(name: &str, value: &str) -> Tag {
        Tag::new(vec![name.to_owned(), value.to_owned()])
    }

    fn reference(event_id: &str, marker: &str) -> Tag {
        Tag::new(vec![
            "e".into(),
            event_id.to_owned(),
            String::new(),
            marker.to_owned(),
        ])
    }
}
