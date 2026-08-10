//! Versioned presentation contract for the NIP-MKT requester surface.
//!
//! This module contains no transport, signing, wallet, or settlement logic.
//! It projects Immortal's verified requester view into bounded rendering data
//! shared by the native GPUI panel and the wasm fixture example.

use immortal_client::mkt_swp_client::{
    ParticipantRole, REQUESTER_SESSION_VIEW_SCHEMA, RequesterSessionView, RequesterTimelineKind,
};
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::receipt_ledger::ReceiptVerification;
#[cfg(not(target_arch = "wasm32"))]
use crate::session_flow::{IntentProgress, MarketSession, StatusSlot};

pub const MARKET_SESSION_VIEW_SCHEMA: &str = "openagents.omega.market-session-view.v1";
pub const MARKET_FIXTURE_CORPUS_SCHEMA: &str = "openagents.mkt-swp.client-engine-fixtures.v1";
pub const MKT_SWP_ERROR_CODES: &[&str] = &[
    "swp_unsupported_profile",
    "swp_unsupported_version",
    "swp_unsupported_critical_member",
    "swp_unsupported_extension",
    "swp_invalid_asset_id",
    "swp_invalid_pair",
    "swp_side_disabled",
    "swp_invalid_amount",
    "swp_invalid_fee",
    "swp_amount_equation_mismatch",
    "swp_quote_expired",
    "swp_order_selection_invalid",
    "swp_contract_missing",
    "swp_contract_signer_invalid",
    "swp_contract_digest_mismatch",
    "swp_contract_terms_mismatch",
    "swp_price_feed_invalid",
    "swp_price_feed_stale",
    "swp_terms_mismatch",
    "swp_reservation_missing",
    "swp_reservation_expired",
    "swp_reservation_overallocated",
    "swp_reservation_fork",
    "swp_reservation_proof_invalid",
    "swp_covenant_reserve_invalid",
    "swp_timeout_ladder_unsafe",
    "swp_invoice_invalid",
    "swp_payment_hash_mismatch",
    "swp_script_invalid",
    "swp_script_commitment_mismatch",
    "swp_liquid_network_mismatch",
    "swp_liquid_output_invalid",
    "swp_liquid_unblind_failed",
    "swp_liquid_unblind_mismatch",
    "swp_ark_operator_mismatch",
    "swp_ark_graph_invalid",
    "swp_ark_vtxo_invalid",
    "swp_ark_exit_unsafe",
    "swp_musig_transcript_invalid",
    "swp_confirmation_insufficient",
    "swp_rbf_policy_violation",
    "swp_zero_conf_not_allowed",
    "swp_zero_conf_unsafe_mempool",
    "swp_zero_conf_limit_exceeded",
    "swp_replacement",
    "swp_reorg",
    "swp_funding_not_authorized",
    "swp_status_signer_invalid",
    "swp_status_transition_invalid",
    "swp_status_gap",
    "swp_status_fork",
    "swp_cancel_ineffective",
    "swp_evidence_unavailable",
    "swp_evidence_mismatch",
    "swp_settlement_overclaim",
    "swp_exit_package_missing",
    "swp_exit_package_mismatch",
    "swp_exit_package_unusable",
    "swp_secret_material_forbidden",
    "swp_external_signature_invalid",
    "swp_external_signature_mismatch",
    "swp_privacy_violation",
    "swp_external_effect_conflict",
    "swp_idempotency_conflict",
    "swp_refund_failed",
    "swp_coordinator_unavailable",
    "swp_unresolved_loss",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetView {
    pub canonical_id: String,
    pub display_ticker: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderAssertionView {
    pub assertion: String,
    pub asserter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderView {
    pub provider_id: String,
    pub display_name: String,
    pub status: String,
    pub profiles: Vec<String>,
    pub assertions: Vec<ProviderAssertionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferingSideView {
    pub input: AssetView,
    pub output: AssetView,
    pub direction: String,
    pub minimum_amount: String,
    pub maximum_amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferingView {
    pub offering_id: String,
    pub provider_id: String,
    pub status: String,
    pub profile: String,
    pub version: u64,
    pub published_at: u64,
    pub sides: Vec<OfferingSideView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationProofClass {
    None,
    ProviderSigned,
    CovenantReserve,
    Other,
}

impl ReservationProofClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "no reserve proof",
            Self::ProviderSigned => "provider-signed claim",
            Self::CovenantReserve => "covenant reserve",
            Self::Other => "other typed proof",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReservationView {
    pub class: String,
    pub proof_class: ReservationProofClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustodyView {
    pub funds_control: String,
    pub key_control: String,
    pub recovery_control: String,
    pub counterparty_exposure: String,
    pub maximum_custody_duration_seconds: u64,
    pub exact_height_bound: Option<u64>,
    pub credential_exposure: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRung {
    Pledged,
    Reserved,
    Measured,
    Verified,
    Paid,
    Settled,
}

impl EvidenceRung {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pledged => "pledged",
            Self::Reserved => "reserved",
            Self::Measured => "measured",
            Self::Verified => "verified",
            Self::Paid => "paid",
            Self::Settled => "settled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeeBreakdownView {
    pub provider_fee: String,
    pub miner_fee_budget: String,
    pub lightning_routing_fee_budget: String,
    pub fee_payer: String,
    pub rounding_rule: String,
    pub amount_equation: String,
    pub maximum_total_fee: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceFeedView {
    pub url: String,
    pub value_pointer: String,
    pub observed_value: String,
    pub observed_at: u64,
    pub max_age_seconds: u64,
    pub response_sha256: String,
}

impl PriceFeedView {
    pub fn is_stale(&self, now: u64) -> bool {
        now > self.observed_at.saturating_add(self.max_age_seconds)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteView {
    pub quote_id: String,
    pub provider_id: String,
    pub quote_class: String,
    pub input: AssetView,
    pub output: AssetView,
    pub input_amount: String,
    pub output_amount: String,
    pub expires_at: u64,
    pub reservation: ReservationView,
    pub fees: FeeBreakdownView,
    pub price_feed: Option<PriceFeedView>,
    pub custody: CustodyView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineSlotState {
    Event,
    Gap,
    Fork,
    Malformed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineSlotView {
    pub sequence: Option<u64>,
    pub state: TimelineSlotState,
    pub labels: Vec<String>,
    pub event_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineLaneView {
    pub signer_role: String,
    pub signer_pubkey: String,
    pub slots: Vec<TimelineSlotView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyState {
    Pending,
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRowView {
    pub check_id: String,
    pub label: String,
    pub state: VerifyState,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyChecklistView {
    pub rows: Vec<VerifyRowView>,
    pub engine_funding_authorized: bool,
}

impl VerifyChecklistView {
    pub fn funding_authorized(&self) -> bool {
        self.engine_funding_authorized
            && !self.rows.is_empty()
            && self.rows.iter().all(|row| row.state == VerifyState::Passed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitPackageView {
    pub exists: bool,
    pub artifact_sha256: Option<String>,
    pub latest_safe_height: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptView {
    pub receipt_id: String,
    pub outcome: String,
    pub signer_claim: String,
    pub rung: EvidenceRung,
    pub redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedErrorView {
    pub code: String,
}

impl TypedErrorView {
    pub fn local_message(&self) -> &'static str {
        typed_error_message(&self.code)
    }
}

pub fn typed_error_message(code: &str) -> &'static str {
    match code {
        "swp_unsupported_profile" => "This market profile is not supported.",
        "swp_unsupported_version" => "This market profile version is not supported.",
        "swp_unsupported_critical_member" => {
            "The record requires a critical member this client does not support."
        }
        "swp_unsupported_extension" => "The requested swap extension is not supported.",
        "swp_invalid_asset_id" => "A signed asset identifier is invalid.",
        "swp_invalid_pair" => "The signed asset pair is invalid for this swap type.",
        "swp_side_disabled" => "This offering side is disabled.",
        "swp_invalid_amount" => "A signed amount is invalid or outside the offering range.",
        "swp_invalid_fee" => "A signed fee is invalid or outside policy.",
        "swp_amount_equation_mismatch" => {
            "The promised output does not reproduce from the signed amounts and fees."
        }
        "swp_quote_expired" => "The signed quote expired. Request a new quote.",
        "swp_order_selection_invalid" => "The order changes a non-selectable quote term.",
        "swp_contract_missing" => "The bilateral swap contract is incomplete.",
        "swp_contract_signer_invalid" => "A contract signer or role is invalid.",
        "swp_contract_digest_mismatch" => {
            "The two contract records do not commit to the same terms."
        }
        "swp_contract_terms_mismatch" => "The signed contract terms do not agree.",
        "swp_price_feed_invalid" => "The quote's exact price-feed proof is invalid.",
        "swp_price_feed_stale" => "The quote's exact price-feed observation is stale.",
        "swp_terms_mismatch" => "A rail object or status differs from the signed terms.",
        "swp_reservation_missing" => "Required reservation evidence is missing.",
        "swp_reservation_expired" => "The signed reservation expired before the order.",
        "swp_reservation_overallocated" => {
            "Active reservations exceed the provider's committed capacity."
        }
        "swp_reservation_fork" => "The provider signed conflicting reservation claims.",
        "swp_reservation_proof_invalid" => {
            "The reservation proof does not establish its claimed class."
        }
        "swp_covenant_reserve_invalid" => "The covenant reserve proof is invalid or reused.",
        "swp_timeout_ladder_unsafe" => "The signed recovery timelocks are not safe.",
        "swp_invoice_invalid" => "The invoice did not pass the local checks.",
        "swp_payment_hash_mismatch" => "The payment hash does not match the signed terms.",
        "swp_script_invalid" => "The signed script or Taproot tree is invalid.",
        "swp_script_commitment_mismatch" => "The lock script does not match the signed commitment.",
        "swp_liquid_network_mismatch" => "The Liquid network or pegged asset does not match.",
        "swp_liquid_output_invalid" => "The selected Liquid output is invalid.",
        "swp_liquid_unblind_failed" => "The selected Liquid output could not be unblinded.",
        "swp_liquid_unblind_mismatch" => {
            "The unblinded Liquid output differs from the signed terms."
        }
        "swp_ark_operator_mismatch" => "The Ark operator or policy does not match.",
        "swp_ark_graph_invalid" => "The signed Ark VTXO graph is invalid.",
        "swp_ark_vtxo_invalid" => "The selected Ark VTXO is invalid.",
        "swp_ark_exit_unsafe" => "The Ark exit package is incomplete or unsafe.",
        "swp_musig_transcript_invalid" => "The MuSig2 signing transcript is invalid.",
        "swp_confirmation_insufficient" => "The rail object has too few confirmations.",
        "swp_rbf_policy_violation" => "The funding transaction violates replacement policy.",
        "swp_zero_conf_not_allowed" => "Zero-confirmation acceptance is not allowed.",
        "swp_zero_conf_unsafe_mempool" => {
            "The local mempool view cannot safely authorize zero-confirmation acceptance."
        }
        "swp_zero_conf_limit_exceeded" => "The zero-confirmation exposure limit is exceeded.",
        "swp_replacement" => "A tracked transaction was replaced.",
        "swp_reorg" => "A tracked transaction was displaced by a reorganization.",
        "swp_funding_not_authorized" => "Local verification has not authorized funding.",
        "swp_status_signer_invalid" => "The signer cannot claim this status action.",
        "swp_status_transition_invalid" => "The signed status transition is invalid.",
        "swp_status_gap" => "A signer's status sequence has a visible gap.",
        "swp_status_fork" => "A signer produced conflicting status records.",
        "swp_cancel_ineffective" => "The signed cancellation cannot stop an irreversible effect.",
        "swp_evidence_unavailable" => "Bound verification evidence is unavailable.",
        "swp_evidence_mismatch" => "Bound verification evidence differs from the signed terms.",
        "swp_settlement_overclaim" => "The settlement claim exceeds its verifier evidence.",
        "swp_exit_package_missing" => "The required recovery package was not persisted.",
        "swp_exit_package_mismatch" => "The recovery package differs from the signed contract.",
        "swp_exit_package_unusable" => "The recovery package is not independently usable.",
        "swp_secret_material_forbidden" => {
            "A market artifact contains prohibited custody material."
        }
        "swp_external_signature_invalid" => "An external signer returned an invalid signature.",
        "swp_external_signature_mismatch" => {
            "An external signer changed the exact bytes it was given."
        }
        "swp_privacy_violation" => "The record exceeds its signed privacy audience.",
        "swp_external_effect_conflict" => {
            "One effect identifier refers to conflicting external operations."
        }
        "swp_idempotency_conflict" => "One idempotency key refers to changed signed input.",
        "swp_refund_failed" => "The required refund failed or became unsafe.",
        "swp_coordinator_unavailable" => "Coordination is unavailable. Enter direct recovery.",
        "swp_unresolved_loss" => "The session cannot prove its loss accounting.",
        "mkt-v2-intent-invalid" => "The provider rejected the versioned intent.",
        _ => "The market returned an unsupported typed error identifier.",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketSessionViewModel {
    pub schema: String,
    pub engine_schema: String,
    pub session_id: String,
    pub phase: String,
    pub provider: ProviderView,
    pub offering: Option<OfferingView>,
    pub quotes: Vec<QuoteView>,
    pub timeline: Vec<TimelineLaneView>,
    pub verification: VerifyChecklistView,
    pub exit_package: ExitPackageView,
    pub receipt: Option<ReceiptView>,
    pub errors: Vec<TypedErrorView>,
}

impl MarketSessionViewModel {
    pub fn from_engine_view(view: &RequesterSessionView, now: u64) -> Self {
        let quote = &view.quote;
        let quote_view = QuoteView {
            quote_id: quote.quote_id.clone(),
            provider_id: quote.provider_pubkey.clone(),
            quote_class: quote.quote_class.clone(),
            input: asset_view(&quote.input_asset_id),
            output: asset_view(&quote.output_asset_id),
            input_amount: quote.input_amount.clone(),
            output_amount: quote.output_amount.clone(),
            expires_at: quote.expires_at,
            reservation: ReservationView {
                class: quote.reservation_class.clone(),
                proof_class: reservation_proof_class(&quote.reservation_class),
            },
            fees: FeeBreakdownView {
                provider_fee: quote.fees.provider_fee.clone(),
                miner_fee_budget: quote.fees.miner_fee_budget.clone(),
                lightning_routing_fee_budget: quote.fees.lightning_routing_fee_budget.clone(),
                fee_payer: quote.fees.fee_payer.clone(),
                rounding_rule: quote.rounding.clone(),
                amount_equation: quote.amount_equation.clone(),
                maximum_total_fee: quote.fees.maximum_total_fee.clone(),
            },
            price_feed: quote.price_feed.as_ref().map(|feed| PriceFeedView {
                url: feed.url.clone(),
                value_pointer: feed.value_pointer.clone(),
                observed_value: feed.observed_value.clone(),
                observed_at: feed.observed_at,
                max_age_seconds: feed.max_age_seconds,
                response_sha256: feed.response_sha256.clone(),
            }),
            custody: CustodyView {
                funds_control: "requester until an explicitly authorized rail effect".to_owned(),
                key_control: "requester".to_owned(),
                recovery_control: "requester exit package".to_owned(),
                counterparty_exposure: quote.reservation_class.clone(),
                maximum_custody_duration_seconds: quote
                    .effective_acceptance_deadline
                    .saturating_sub(now),
                exact_height_bound: None,
                credential_exposure: "session-scoped requester key".to_owned(),
            },
        };
        let timeline = timeline_from_engine(view);
        let mut rows = vec![
            verify_row("signed_quote", "Signed quote", VerifyState::Passed),
            verify_row(
                "bilateral_contract",
                "Bilateral contract terms",
                if matches!(
                    view.verification.state,
                    immortal_client::mkt_swp_client::RequesterVerificationState::ContractTermsVerified
                        | immortal_client::mkt_swp_client::RequesterVerificationState::TerminalVerified
                ) {
                    VerifyState::Passed
                } else {
                    VerifyState::Pending
                },
            ),
        ];
        for check_id in [
            "lock_script_tree",
            "amounts",
            "payment_hash",
            "timelocks",
            "claim_path",
            "refund_path",
        ] {
            rows.push(verify_row(
                check_id,
                check_label(check_id),
                if view.verification.funding_authorized {
                    VerifyState::Passed
                } else {
                    VerifyState::Pending
                },
            ));
        }
        let errors = view
            .verification
            .invalid_status_claims
            .iter()
            .map(|code| TypedErrorView { code: code.clone() })
            .collect();
        Self {
            schema: MARKET_SESSION_VIEW_SCHEMA.to_owned(),
            engine_schema: view.schema.clone(),
            session_id: view.session_id.clone(),
            phase: format!("{:?}", view.verification.state).to_ascii_lowercase(),
            provider: ProviderView {
                provider_id: quote.provider_pubkey.clone(),
                display_name: short_id(&quote.provider_pubkey),
                status: "signed session counterparty".to_owned(),
                profiles: vec!["mkt-swp:1".to_owned()],
                assertions: Vec::new(),
            },
            offering: None,
            quotes: vec![quote_view],
            timeline,
            verification: VerifyChecklistView {
                rows,
                engine_funding_authorized: view.verification.funding_authorized,
            },
            exit_package: ExitPackageView {
                exists: view.verification.funding_authorized,
                artifact_sha256: None,
                latest_safe_height: None,
            },
            receipt: None,
            errors,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != MARKET_SESSION_VIEW_SCHEMA {
            return Err("market session view schema is unsupported".to_owned());
        }
        if self.engine_schema != REQUESTER_SESSION_VIEW_SCHEMA {
            return Err(
                "market session view does not identify the Immortal engine view".to_owned(),
            );
        }
        if self.session_id.len() != 64
            || !self
                .session_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("market session view session id is invalid".to_owned());
        }
        if self.verification.funding_authorized()
            && self
                .verification
                .rows
                .iter()
                .any(|row| row.state != VerifyState::Passed || row.error_code.is_some())
        {
            return Err("market session view advances funding past a failed check".to_owned());
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_market_session(session: &MarketSession, now: u64) -> Self {
        let quotes = session
            .quotes()
            .iter()
            .map(|quote| quote_from_market_session(quote, now))
            .collect();
        let timeline = session
            .status_lanes()
            .into_iter()
            .map(|lane| TimelineLaneView {
                signer_role: match lane.role {
                    ParticipantRole::Requester => "requester",
                    ParticipantRole::Provider => "provider",
                }
                .to_owned(),
                signer_pubkey: lane.pubkey,
                slots: lane
                    .slots
                    .into_iter()
                    .map(|slot| match slot {
                        StatusSlot::Gap { sequence } => TimelineSlotView {
                            sequence: Some(sequence),
                            state: TimelineSlotState::Gap,
                            labels: vec![format!("missing sequence {sequence}")],
                            event_ids: Vec::new(),
                        },
                        StatusSlot::Filled { sequence, entries } => TimelineSlotView {
                            sequence: Some(sequence),
                            state: if entries.len() > 1 {
                                TimelineSlotState::Fork
                            } else {
                                TimelineSlotState::Event
                            },
                            labels: entries.iter().map(|entry| entry.state.clone()).collect(),
                            event_ids: entries.into_iter().map(|entry| entry.event_id).collect(),
                        },
                    })
                    .chain(lane.malformed.into_iter().map(|entry| TimelineSlotView {
                        sequence: None,
                        state: TimelineSlotState::Malformed,
                        labels: vec![entry.state],
                        event_ids: vec![entry.event_id],
                    }))
                    .collect(),
            })
            .collect();
        let intent_progress = session.intent_progress(now);
        let acknowledgment_state = match &intent_progress {
            IntentProgress::NotOrdered | IntentProgress::AwaitingAcknowledgment { .. } => {
                VerifyState::Pending
            }
            IntentProgress::Rejected { .. } => VerifyState::Failed,
            IntentProgress::AwaitingOutcome { .. } | IntentProgress::OutcomeReceived { .. } => {
                VerifyState::Passed
            }
        };
        let terminal_state = if matches!(intent_progress, IntentProgress::OutcomeReceived { .. }) {
            VerifyState::Passed
        } else {
            VerifyState::Pending
        };
        let mut rows = vec![
            verify_row(
                "signed_quote",
                "Signed quote",
                if session.quotes().is_empty() {
                    VerifyState::Pending
                } else {
                    VerifyState::Passed
                },
            ),
            verify_row(
                "intent_acknowledgment",
                "Intent acknowledgment",
                acknowledgment_state,
            ),
            verify_row("terminal_outcome", "Terminal outcome", terminal_state),
        ];
        for check_id in [
            "lock_script_tree",
            "amounts",
            "payment_hash",
            "timelocks",
            "claim_path",
            "refund_path",
        ] {
            rows.push(verify_row(
                check_id,
                check_label(check_id),
                VerifyState::Pending,
            ));
        }
        let receipt_verifications = session.receipt_verifications();
        let receipt = receipt_verifications.iter().rev().find_map(|verification| {
            let ReceiptVerification::ProviderSigned { receipt, .. } = verification else {
                return None;
            };
            Some(ReceiptView {
                receipt_id: receipt.receipt_id.clone(),
                outcome: receipt.outcome.clone(),
                signer_claim: "provider-signed".to_owned(),
                rung: EvidenceRung::Pledged,
                redacted: true,
            })
        });
        let mut errors = Vec::new();
        if let IntentProgress::Rejected { error_code, .. } = &intent_progress {
            errors.push(TypedErrorView {
                code: error_code.clone(),
            });
        }
        errors.extend(
            receipt_verifications
                .iter()
                .filter_map(|verification| match verification {
                    ReceiptVerification::Invalid { .. } => Some(TypedErrorView {
                        code: "mkt_receipt_invalid".to_owned(),
                    }),
                    _ => None,
                }),
        );
        Self {
            schema: MARKET_SESSION_VIEW_SCHEMA.to_owned(),
            engine_schema: REQUESTER_SESSION_VIEW_SCHEMA.to_owned(),
            session_id: session.session_id().to_owned(),
            phase: session.phase().label().to_owned(),
            provider: ProviderView {
                provider_id: session.provider_id().to_owned(),
                display_name: short_id(session.provider_id()),
                status: "signed session counterparty".to_owned(),
                profiles: vec!["mkt-swp:1".to_owned()],
                assertions: Vec::new(),
            },
            offering: None,
            quotes,
            timeline,
            verification: VerifyChecklistView {
                rows,
                // #244 is a no-spend development flow. This presentation
                // cannot introduce wallet authority the engine never granted.
                engine_funding_authorized: false,
            },
            exit_package: ExitPackageView {
                exists: false,
                artifact_sha256: None,
                latest_safe_height: None,
            },
            receipt,
            errors,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn quote_from_market_session(quote: &crate::session_flow::QuoteCandidate, now: u64) -> QuoteView {
    let terms = serde_json::from_str::<serde_json::Value>(&quote.event.content)
        .ok()
        .and_then(|content| content.pointer("/mkt_swp/terms").cloned());
    let term = |name: &str| {
        terms
            .as_ref()
            .and_then(|terms| terms.get(name))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("not separately quoted")
            .to_owned()
    };
    let price_feed = terms
        .as_ref()
        .and_then(|terms| terms.get("price_feed"))
        .filter(|feed| !feed.is_null())
        .and_then(|feed| {
            Some(PriceFeedView {
                url: feed.get("url")?.as_str()?.to_owned(),
                value_pointer: feed.get("value_pointer")?.as_str()?.to_owned(),
                observed_value: feed.get("observed_value")?.as_str()?.to_owned(),
                observed_at: feed.get("observed_at")?.as_u64()?,
                max_age_seconds: feed.get("max_age_seconds")?.as_u64()?,
                response_sha256: feed.get("response_sha256")?.as_str()?.to_owned(),
            })
        });
    QuoteView {
        quote_id: quote.event.id.clone(),
        provider_id: quote.event.pubkey.clone(),
        quote_class: quote.quote_class.clone(),
        input: asset_view(&quote.asset_pair.0),
        output: asset_view(&quote.asset_pair.1),
        input_amount: quote.input_amount.clone(),
        output_amount: quote.output_amount.clone(),
        expires_at: quote.expires_at,
        reservation: ReservationView {
            class: quote.reservation.clone(),
            proof_class: reservation_proof_class(&quote.reservation),
        },
        fees: FeeBreakdownView {
            provider_fee: term("provider_fee"),
            miner_fee_budget: term("miner_fee_budget"),
            lightning_routing_fee_budget: term("lightning_routing_fee_budget"),
            fee_payer: term("fee_payer"),
            rounding_rule: term("rounding"),
            amount_equation: term("amount_equation"),
            maximum_total_fee: quote.maximum_total_fee.clone(),
        },
        price_feed,
        custody: CustodyView {
            funds_control: "no funds exist in the #244 development flow".to_owned(),
            key_control: "throwaway session key".to_owned(),
            recovery_control: "frozen no-spend fixture commitment".to_owned(),
            counterparty_exposure: quote.reservation.clone(),
            maximum_custody_duration_seconds: quote.expires_at.saturating_sub(now),
            exact_height_bound: None,
            credential_exposure: "session-scoped; secret is not persisted".to_owned(),
        },
    }
}

fn timeline_from_engine(view: &RequesterSessionView) -> Vec<TimelineLaneView> {
    let mut requester = TimelineLaneView {
        signer_role: "requester".to_owned(),
        signer_pubkey: "requester".to_owned(),
        slots: Vec::new(),
    };
    let mut provider = TimelineLaneView {
        signer_role: "provider".to_owned(),
        signer_pubkey: view.quote.provider_pubkey.clone(),
        slots: Vec::new(),
    };
    for entry in &view.timeline {
        let lane = match entry.author {
            ParticipantRole::Requester => &mut requester,
            ParticipantRole::Provider => &mut provider,
        };
        lane.slots.push(TimelineSlotView {
            sequence: entry.sequence,
            state: if entry.conflict.is_some() {
                TimelineSlotState::Fork
            } else {
                TimelineSlotState::Event
            },
            labels: vec![format!(
                "{}{}",
                timeline_kind_label(entry.kind),
                entry
                    .state
                    .as_ref()
                    .map(|state| format!(" · {state}"))
                    .unwrap_or_default()
            )],
            event_ids: vec![entry.event_id.clone()],
        });
    }
    for gap in &view.verification.status_gaps {
        provider.slots.push(TimelineSlotView {
            sequence: None,
            state: TimelineSlotState::Gap,
            labels: vec![gap.clone()],
            event_ids: Vec::new(),
        });
    }
    vec![requester, provider]
}

fn timeline_kind_label(kind: RequesterTimelineKind) -> &'static str {
    match kind {
        RequesterTimelineKind::Rfq => "RFQ",
        RequesterTimelineKind::Quote => "Quote",
        RequesterTimelineKind::Order => "Order",
        RequesterTimelineKind::Contract => "Contract",
        RequesterTimelineKind::Status => "Status",
        RequesterTimelineKind::Cancel => "Cancel",
        RequesterTimelineKind::Close => "Close",
    }
}

pub fn asset_view(asset_id: &str) -> AssetView {
    let ticker = match asset_id.rsplit(':').next() {
        Some("lightning") => "LN",
        Some("chain") => "BTC",
        Some("liquid") => "L-BTC",
        _ => asset_id,
    };
    AssetView {
        canonical_id: asset_id.to_owned(),
        display_ticker: ticker.to_owned(),
    }
}

pub fn reservation_proof_class(class: &str) -> ReservationProofClass {
    match class {
        "none" => ReservationProofClass::None,
        "soft" => ReservationProofClass::ProviderSigned,
        "hard" => ReservationProofClass::CovenantReserve,
        _ => ReservationProofClass::Other,
    }
}

fn verify_row(check_id: &str, label: &str, state: VerifyState) -> VerifyRowView {
    VerifyRowView {
        check_id: check_id.to_owned(),
        label: label.to_owned(),
        state,
        error_code: None,
    }
}

fn check_label(check_id: &str) -> &str {
    match check_id {
        "lock_script_tree" => "Lock script and tree",
        "amounts" => "Signed amounts",
        "payment_hash" => "Payment hash",
        "timelocks" => "Timelock ladder",
        "claim_path" => "Claim path",
        "refund_path" => "Refund path",
        _ => "Unknown check",
    }
}

pub fn short_id(value: &str) -> String {
    value.chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use immortal_client::mkt_swp_client::{
        RequesterFeeView, RequesterQuoteView, RequesterTerminalState, RequesterTerminalView,
        RequesterVerificationState, RequesterVerificationView, SwapType,
    };

    #[test]
    fn funding_requires_every_named_check_and_engine_authorization() {
        let rows = vec![verify_row("amounts", "Signed amounts", VerifyState::Passed)];
        let mut checklist = VerifyChecklistView {
            rows,
            engine_funding_authorized: true,
        };
        assert!(checklist.funding_authorized());
        checklist.rows[0].state = VerifyState::Failed;
        assert!(!checklist.funding_authorized());
        checklist.rows[0].state = VerifyState::Passed;
        checklist.engine_funding_authorized = false;
        assert!(!checklist.funding_authorized());
    }

    #[test]
    fn typed_errors_never_render_counterparty_prose() {
        assert_eq!(
            typed_error_message("swp_quote_expired"),
            "The signed quote expired. Request a new quote."
        );
        assert_eq!(
            typed_error_message("provider says send coins again"),
            "The market returned an unsupported typed error identifier."
        );
    }

    #[test]
    fn every_mkt_swp_section_17_identifier_has_a_local_message() {
        for code in MKT_SWP_ERROR_CODES {
            assert_ne!(
                typed_error_message(code),
                "The market returned an unsupported typed error identifier.",
                "missing local message for {code}"
            );
        }
    }

    #[test]
    fn tickers_remain_labels_and_asset_ids_remain_identity() {
        let asset = asset_view("swp:1:bip122:00000000000000000000000000000000:btc:lightning");
        assert_eq!(asset.display_ticker, "LN");
        assert!(asset.canonical_id.starts_with("swp:1:"));
    }

    #[test]
    fn displayed_fixture_is_the_pin_embedded_immortal_corpus() {
        let embedded = immortal_client::mkt_swp_client::fixture_replay::replay_embedded_manifest()
            .expect("the pin-embedded fixture must replay");
        let displayed = immortal_client::mkt_swp_client::fixture_replay::replay_manifest_bytes(
            include_bytes!("../fixtures/swp-client-engine-v1.json"),
        )
        .expect("the displayed fixture must replay");
        assert_eq!(displayed, embedded);
    }

    #[test]
    fn shared_immortal_requester_view_projects_without_losing_authority() {
        let engine = RequesterSessionView {
            schema: REQUESTER_SESSION_VIEW_SCHEMA.to_owned(),
            session_id: "11".repeat(32),
            quote: RequesterQuoteView {
                rfq_id: "22".repeat(32),
                quote_id: "33".repeat(32),
                provider_pubkey: "44".repeat(32),
                quote_class: "firm".to_owned(),
                reservation_class: "soft".to_owned(),
                swap_type: SwapType::Submarine,
                input_asset_id: "swp:1:bip122:00:btc:chain".to_owned(),
                output_asset_id: "swp:1:bip122:00:btc:lightning".to_owned(),
                input_amount: "100000".to_owned(),
                output_amount: "99000".to_owned(),
                amount_equation: "input_minus_fees".to_owned(),
                rounding: "floor_output_sats".to_owned(),
                clock_skew_seconds: "0".to_owned(),
                expires_at: 200,
                effective_acceptance_deadline: 180,
                fees: RequesterFeeView {
                    fee_bps: "100".to_owned(),
                    provider_fee: "500".to_owned(),
                    miner_fee_budget: "500".to_owned(),
                    lightning_routing_fee_budget: "0".to_owned(),
                    maximum_total_fee: "1000".to_owned(),
                    fee_payer: "requester".to_owned(),
                },
                price_feed: None,
            },
            timeline: Vec::new(),
            verification: RequesterVerificationView {
                state: RequesterVerificationState::ContractTermsVerified,
                local_verification_required: true,
                funding_authorized: true,
                status_gaps: Vec::new(),
                status_forks: Vec::new(),
                invalid_status_claims: Vec::new(),
            },
            terminal: RequesterTerminalView {
                claimed_state: RequesterTerminalState::Open,
                canonical_close_id: None,
                close_event_ids: Vec::new(),
                principal_unresolved: None,
                loss_accounting_complete: false,
                local_effects_verified: false,
                watch_terminal: false,
            },
            deliveries: Vec::new(),
        };

        let view = MarketSessionViewModel::from_engine_view(&engine, 100);
        assert_eq!(view.engine_schema, REQUESTER_SESSION_VIEW_SCHEMA);
        assert_eq!(view.provider.provider_id, engine.quote.provider_pubkey);
        assert!(view.verification.funding_authorized());
        assert!(view.validate().is_ok());
    }
}
