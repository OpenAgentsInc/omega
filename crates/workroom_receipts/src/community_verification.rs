//! The verifier-signed independent verification event (omega#48).
//!
//! Contract: `SARAH-CW-00` §8.4, amendment `SARAH-CW-00-A1` (2026-07-25), in
//! `docs/omega/2026-07-24-community-workroom-contract.md` in the openagents
//! repository.
//!
//! ## Why this exists on the Rust side at all
//!
//! The contract's §4 writable-authority table already reserved a row for
//! independent verification with its authority named and its wire form left
//! unspecified. Every client therefore read independence off Sarah's
//! *arbitration decision* — the deciding key asserting, on the verifier's
//! behalf, that a verification happened. Step 6 of the §8.1 lifecycle was the
//! one step nobody signed.
//!
//! The amendment specifies the wire form. A second implementation must be able
//! to reach the same admission from the contract and the fixtures alone, so
//! this module decodes the **same fixture bytes** as
//! `packages/sarah/src/community-arbitration/verification.ts`. The digests in
//! [`SHARED_FIXTURE_DIGESTS`] are what makes that enforceable rather than
//! conventional: edit a fixture in either repository and that repository's test
//! goes red until the other agrees.
//!
//! ## What this module does not do
//!
//! It admits a verification. It never accepts a unit — acceptance is Sarah's,
//! per the §4 acceptance row — and it holds no state.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

/// Schema id of the record the verifying agent signs.
pub const COMMUNITY_INDEPENDENT_VERIFICATION_SCHEMA: &str =
    "openagents.sarah.community_independent_verification.v1";

/// Amendment packet that specified this wire form.
pub const COMMUNITY_INDEPENDENT_VERIFICATION_PACKET: &str = "SARAH-CW-00-A1";

/// NIP-90 feedback kind. The carrier the contract already used for the quote,
/// the acceptance, the decision, the appeal and the ruling. No new kind.
pub const COMMUNITY_FEEDBACK_KIND: u32 = 7000;

/// The `cw_feedback_type` value. A fourth value in an existing discriminator.
pub const INDEPENDENT_VERIFICATION_FEEDBACK_TYPE: &str = "independent_verification";

/// The verifier's verdict.
///
/// Deliberately not `accepted` / `rejected`: those are Sarah's words and this
/// event must not be mistakable for the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationVerdict {
    Reproduced,
    NotReproduced,
    Inconclusive,
}

impl VerificationVerdict {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Reproduced => "reproduced",
            Self::NotReproduced => "not_reproduced",
            Self::Inconclusive => "inconclusive",
        }
    }

    #[must_use]
    pub fn parse_token(token: &str) -> Option<Self> {
        match token {
            "reproduced" => Some(Self::Reproduced),
            "not_reproduced" => Some(Self::NotReproduced),
            "inconclusive" => Some(Self::Inconclusive),
            _ => None,
        }
    }
}

/// Why a verification event did not become an admitted verification.
///
/// Every one of these is a refusal a reader can render. A verification that
/// vanishes silently is indistinguishable from one that was never published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationRefusal {
    NotAVerificationEvent,
    Malformed,
    /// Somebody other than the verifier signed the verifier's claim. The state
    /// this amendment exists to end.
    VerifierNotAuthor,
    VerifierKeyBurned,
    VerifierBindingUnconfirmed,
    ProducerBindingUnconfirmed,
    SelfDealingOperators,
    DecidesPaymentForbidden,
}

impl VerificationRefusal {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::NotAVerificationEvent => "not_a_verification_event",
            Self::Malformed => "malformed",
            Self::VerifierNotAuthor => "verifier_not_author",
            Self::VerifierKeyBurned => "verifier_key_burned",
            Self::VerifierBindingUnconfirmed => "verifier_binding_unconfirmed",
            Self::ProducerBindingUnconfirmed => "producer_binding_unconfirmed",
            Self::SelfDealingOperators => "self_dealing_operators",
            Self::DecidesPaymentForbidden => "decides_payment_forbidden",
        }
    }
}

/// An admitted verification, with both operators read from the folded record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedVerification {
    pub source_event_id: String,
    pub result_event_id: String,
    pub verifier_agent_pubkey: String,
    /// From the binding. Never from a tag.
    pub verifier_operator_pubkey: String,
    pub producer_agent_pubkey: String,
    /// From the binding. Never from a tag.
    pub producer_operator_pubkey: String,
    pub verdict: VerificationVerdict,
}

/// A signed event as it arrives from a relay.
#[derive(Debug, Clone, Deserialize)]
pub struct VerificationEvent {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

/// The agent → operator binding a caller resolves from its folded record.
///
/// A map plus a burn set rather than the whole ledger: this module has no
/// business folding membership, and taking only what it reads makes it obvious
/// that nothing here can be talked into a binding by an event.
#[derive(Debug, Clone, Default)]
pub struct CommunityBinding {
    operator_by_agent: BTreeMap<String, String>,
    burned_agent_keys: BTreeSet<String>,
}

impl CommunityBinding {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn bind(mut self, agent_pubkey: &str, operator_pubkey: &str) -> Self {
        self.operator_by_agent
            .insert(agent_pubkey.trim().to_ascii_lowercase(), operator_pubkey.trim().to_ascii_lowercase());
        self
    }

    #[must_use]
    pub fn burn(mut self, agent_pubkey: &str) -> Self {
        self.burned_agent_keys
            .insert(agent_pubkey.trim().to_ascii_lowercase());
        self
    }

    #[must_use]
    pub fn operator_for_agent(&self, agent_pubkey: &str) -> Option<&str> {
        self.operator_by_agent
            .get(&agent_pubkey.trim().to_ascii_lowercase())
            .map(String::as_str)
    }

    #[must_use]
    pub fn is_agent_key_burned(&self, agent_pubkey: &str) -> bool {
        self.burned_agent_keys
            .contains(&agent_pubkey.trim().to_ascii_lowercase())
    }
}

fn tag_value<'a>(event: &'a VerificationEvent, name: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|tag| tag.first().map(String::as_str) == Some(name))
        .and_then(|tag| tag.get(1))
        .map(String::as_str)
}

fn tagged_event_id<'a>(event: &'a VerificationEvent, marker: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|tag| {
            tag.first().map(String::as_str) == Some("e")
                && tag.get(3).map(String::as_str) == Some(marker)
        })
        .and_then(|tag| tag.get(1))
        .map(String::as_str)
}

fn is_hex_64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Admit one signed verification event against the folded record.
///
/// Both operators come from `binding`, never from a tag. `cw_verifier_operator_ref`
/// is what the *verifier* asserts about itself, and an agent key asserting its
/// own operator is exactly the self-dealing shape refused on the decision path.
///
/// # Errors
///
/// Returns the typed refusal. There is no silent drop.
pub fn admit_independent_verification(
    event: &VerificationEvent,
    binding: &CommunityBinding,
) -> Result<AdmittedVerification, VerificationRefusal> {
    let verifier_key = event.pubkey.trim().to_ascii_lowercase();

    let feedback_type = tag_value(event, "cw_feedback_type")
        .or_else(|| tag_value(event, "lbr_feedback_type"));
    if event.kind != COMMUNITY_FEEDBACK_KIND
        || feedback_type != Some(INDEPENDENT_VERIFICATION_FEEDBACK_TYPE)
    {
        return Err(VerificationRefusal::NotAVerificationEvent);
    }

    let result_event_id = tagged_event_id(event, "result").unwrap_or_default().to_owned();
    let producer_key = tag_value(event, "cw_producer_agent_pubkey")
        .or_else(|| tag_value(event, "p"))
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let verdict = tag_value(event, "status").and_then(VerificationVerdict::parse_token);

    let Some(verdict) = verdict else {
        return Err(VerificationRefusal::Malformed);
    };
    if !is_hex_64(&result_event_id) || !is_hex_64(&producer_key) {
        return Err(VerificationRefusal::Malformed);
    }

    if tag_value(event, "cw_decides_payment") != Some("false") {
        return Err(VerificationRefusal::DecidesPaymentForbidden);
    }

    // The whole point of the amendment.
    if let Some(claimed) = tag_value(event, "cw_verifier_agent_pubkey")
        && claimed.trim().to_ascii_lowercase() != verifier_key
    {
        return Err(VerificationRefusal::VerifierNotAuthor);
    }
    if verifier_key == producer_key {
        return Err(VerificationRefusal::SelfDealingOperators);
    }

    // Revocation binds the subject whatever it signs and whenever it arrives.
    // Checked before the binding lookup so a burned key with a live binding row
    // cannot pass as merely "unconfirmed".
    if binding.is_agent_key_burned(&verifier_key) {
        return Err(VerificationRefusal::VerifierKeyBurned);
    }

    let Some(verifier_operator) = binding.operator_for_agent(&verifier_key) else {
        return Err(VerificationRefusal::VerifierBindingUnconfirmed);
    };
    if let Some(claimed_operator) = tag_value(event, "cw_verifier_operator_ref")
        && claimed_operator.trim().to_ascii_lowercase() != verifier_operator
    {
        return Err(VerificationRefusal::VerifierBindingUnconfirmed);
    }

    let Some(producer_operator) = binding.operator_for_agent(&producer_key) else {
        return Err(VerificationRefusal::ProducerBindingUnconfirmed);
    };

    // Distinct *operators*, not merely distinct keys. One operator holding two
    // agent keys passes every key comparison above.
    if producer_operator == verifier_operator {
        return Err(VerificationRefusal::SelfDealingOperators);
    }

    Ok(AdmittedVerification {
        source_event_id: event.id.clone(),
        result_event_id,
        verifier_agent_pubkey: verifier_key,
        verifier_operator_pubkey: verifier_operator.to_owned(),
        producer_agent_pubkey: producer_key,
        producer_operator_pubkey: producer_operator.to_owned(),
        verdict,
    })
}

/// The exact bytes both repositories hold, by SHA-256 of the file content.
///
/// Not a convenience. A fixture is only "byte-shared" if drift is detectable
/// from inside one repository without reading the other.
pub const SHARED_FIXTURE_DIGESTS: &[(&str, &str)] = &[
    (
        "openagents.sarah.community_independent_verification.v1.canonical.json",
        "992085428cf36e74ee0f1b6bdb6b38c494e7b7cc1b68d668e82bef74fcfa928f",
    ),
    (
        "openagents.sarah.community_independent_verification.v1.negative-operators-not-independent.json",
        "35061fb7846a3fa961ac7335e96dd9cbd21b080ba5bf2fe0389402ce22d3333b",
    ),
    (
        "openagents.sarah.community_independent_verification.v1.negative-verifier-key-burned.json",
        "0a01c8665d52cf0542084897bc79eac52a5cca3b3ae0dfb725ead4f5a8dbcc36",
    ),
    (
        "openagents.sarah.community_independent_verification.v1.negative-verifier-not-author.json",
        "c2f91af69fe979acdcbe8550060d782c72d963c7346d8d6febb5705f87338271",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL: &str = include_str!(
        "../fixtures/openagents.sarah.community_independent_verification.v1.canonical.json"
    );
    const NOT_AUTHOR: &str = include_str!(
        "../fixtures/openagents.sarah.community_independent_verification.v1.negative-verifier-not-author.json"
    );
    const NOT_INDEPENDENT: &str = include_str!(
        "../fixtures/openagents.sarah.community_independent_verification.v1.negative-operators-not-independent.json"
    );
    const BURNED: &str = include_str!(
        "../fixtures/openagents.sarah.community_independent_verification.v1.negative-verifier-key-burned.json"
    );

    #[derive(Debug, Deserialize)]
    struct BoundAgent {
        #[serde(rename = "agentPubkey")]
        agent_pubkey: String,
        #[serde(rename = "operatorPubkey")]
        operator_pubkey: String,
    }

    #[derive(Debug, Deserialize)]
    struct FixtureBinding {
        agents: Vec<BoundAgent>,
        #[serde(rename = "burnedAgentKeys")]
        burned_agent_keys: Vec<String>,
    }

    #[derive(Debug, Deserialize)]
    struct Fixture {
        fixture: String,
        packet: String,
        expect: String,
        binding: FixtureBinding,
        event: VerificationEvent,
        expected: serde_json::Value,
    }

    fn load(raw: &str) -> (Fixture, CommunityBinding) {
        let fixture: Fixture = serde_json::from_str(raw).expect("fixture decodes");
        let mut binding = CommunityBinding::new();
        for agent in &fixture.binding.agents {
            binding = binding.bind(&agent.agent_pubkey, &agent.operator_pubkey);
        }
        for key in &fixture.binding.burned_agent_keys {
            binding = binding.burn(key);
        }
        (fixture, binding)
    }

    /// The cross-repository check. These bytes are the same bytes
    /// `packages/sarah` decodes, and this test is what notices if they stop
    /// being.
    #[test]
    fn the_shared_fixtures_are_the_exact_bytes_the_typescript_side_holds() {
        use std::fmt::Write as _;
        for (name, expected) in SHARED_FIXTURE_DIGESTS {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures")
                .join(name);
            let bytes = std::fs::read(&path).expect("shared fixture is readable");
            let digest = sha256_hex(&bytes, &mut String::new());
            let mut rendered = String::new();
            write!(&mut rendered, "{digest}").expect("write");
            assert_eq!(
                &rendered, expected,
                "{name} drifted from the byte-shared copy in packages/sarah. \
                 Re-share the file rather than re-pinning one side."
            );
        }
    }

    /// Minimal SHA-256 so this crate keeps its two-dependency footprint.
    fn sha256_hex(bytes: &[u8], scratch: &mut String) -> String {
        const K: [u32; 64] = [
            0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1,
            0x923f_82a4, 0xab1c_5ed5, 0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3,
            0x72be_5d74, 0x80de_b1fe, 0x9bdc_06a7, 0xc19b_f174, 0xe49b_69c1, 0xefbe_4786,
            0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f, 0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da,
            0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7, 0xc6e0_0bf3, 0xd5a7_9147,
            0x06ca_6351, 0x1429_2967, 0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc, 0x5338_0d13,
            0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85, 0xa2bf_e8a1, 0xa81a_664b,
            0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070,
            0x19a4_c116, 0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a,
            0x5b9c_ca4f, 0x682e_6ff3, 0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208,
            0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7, 0xc671_78f2,
        ];
        let mut h: [u32; 8] = [
            0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c,
            0x1f83_d9ab, 0x5be0_cd19,
        ];
        let mut message = bytes.to_vec();
        let bit_len = (bytes.len() as u64) * 8;
        message.push(0x80);
        while message.len() % 64 != 56 {
            message.push(0);
        }
        message.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in message.chunks_exact(64) {
            let mut w = [0u32; 64];
            for (index, word) in w.iter_mut().enumerate().take(16) {
                let base = index * 4;
                *word = u32::from_be_bytes([
                    chunk[base],
                    chunk[base + 1],
                    chunk[base + 2],
                    chunk[base + 3],
                ]);
            }
            for index in 16..64 {
                let s0 = w[index - 15].rotate_right(7)
                    ^ w[index - 15].rotate_right(18)
                    ^ (w[index - 15] >> 3);
                let s1 = w[index - 2].rotate_right(17)
                    ^ w[index - 2].rotate_right(19)
                    ^ (w[index - 2] >> 10);
                w[index] = w[index - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[index - 7])
                    .wrapping_add(s1);
            }
            let mut v = h;
            for index in 0..64 {
                let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
                let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
                let temp1 = v[7]
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[index])
                    .wrapping_add(w[index]);
                let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
                let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
                let temp2 = s0.wrapping_add(maj);
                v[7] = v[6];
                v[6] = v[5];
                v[5] = v[4];
                v[4] = v[3].wrapping_add(temp1);
                v[3] = v[2];
                v[2] = v[1];
                v[1] = v[0];
                v[0] = temp1.wrapping_add(temp2);
            }
            for (slot, value) in h.iter_mut().zip(v) {
                *slot = slot.wrapping_add(value);
            }
        }
        scratch.clear();
        let mut out = String::with_capacity(64);
        for word in h {
            out.push_str(&format!("{word:08x}"));
        }
        out
    }

    #[test]
    fn the_canonical_fixture_admits_with_both_operators_from_the_record() {
        let (fixture, binding) = load(CANONICAL);
        assert_eq!(fixture.fixture, COMMUNITY_INDEPENDENT_VERIFICATION_SCHEMA);
        assert_eq!(fixture.packet, COMMUNITY_INDEPENDENT_VERIFICATION_PACKET);
        assert_eq!(fixture.expect, "admit");

        let admitted = admit_independent_verification(&fixture.event, &binding)
            .expect("the canonical verification is admitted");
        assert_eq!(admitted.verifier_agent_pubkey, fixture.event.pubkey);
        assert_ne!(
            admitted.verifier_operator_pubkey,
            admitted.producer_operator_pubkey
        );
        assert_eq!(admitted.verdict, VerificationVerdict::Reproduced);
        assert_eq!(
            admitted.verifier_operator_pubkey,
            fixture.expected["verifierOperatorPubkey"].as_str().unwrap()
        );
        assert_eq!(
            admitted.producer_operator_pubkey,
            fixture.expected["producerOperatorPubkey"].as_str().unwrap()
        );
    }

    /// The exact reason the amendment exists: the verifier signs, or nobody
    /// did.
    #[test]
    fn a_verification_signed_by_somebody_other_than_the_verifier_is_refused() {
        let (fixture, binding) = load(NOT_AUTHOR);
        let refusal = admit_independent_verification(&fixture.event, &binding)
            .expect_err("an impostor signature is refused");
        assert_eq!(refusal, VerificationRefusal::VerifierNotAuthor);
        assert_eq!(refusal.token(), fixture.expected["code"].as_str().unwrap());
    }

    #[test]
    fn distinct_keys_under_one_operator_are_not_independent() {
        let (fixture, binding) = load(NOT_INDEPENDENT);
        let refusal = admit_independent_verification(&fixture.event, &binding)
            .expect_err("one operator on both sides is self-dealing");
        assert_eq!(refusal, VerificationRefusal::SelfDealingOperators);
        assert_eq!(refusal.token(), fixture.expected["code"].as_str().unwrap());
    }

    /// A revocation binds the subject regardless of arrival order, and it binds
    /// it whatever the key goes on to sign.
    #[test]
    fn a_burned_verifier_key_is_refused_even_with_a_live_binding_row() {
        let (fixture, binding) = load(BURNED);
        assert!(
            binding.operator_for_agent(&fixture.event.pubkey).is_some(),
            "the burn must be what refuses this, not a missing binding"
        );
        let refusal = admit_independent_verification(&fixture.event, &binding)
            .expect_err("a burned key verifies nothing");
        assert_eq!(refusal, VerificationRefusal::VerifierKeyBurned);
    }

    /// `cw_verifier_operator_ref` is the verifier's own claim about itself.
    /// Believing it would rebuild the self-dealing hole on the new carrier.
    #[test]
    fn a_verifier_claiming_an_operator_the_record_denies_is_refused() {
        let (mut fixture, binding) = load(CANONICAL);
        for tag in &mut fixture.event.tags {
            if tag.first().map(String::as_str) == Some("cw_verifier_operator_ref") {
                tag[1] = "5".repeat(64);
            }
        }
        let refusal = admit_independent_verification(&fixture.event, &binding)
            .expect_err("a self-asserted operator is not a binding");
        assert_eq!(refusal, VerificationRefusal::VerifierBindingUnconfirmed);
    }

    /// This event is testimony, not a verdict. A verification that could say
    /// `accepted` would be a second acceptance authority.
    #[test]
    fn a_verification_cannot_carry_sarahs_words() {
        assert!(VerificationVerdict::parse_token("accepted").is_none());
        assert!(VerificationVerdict::parse_token("rejected").is_none());
        let (mut fixture, binding) = load(CANONICAL);
        for tag in &mut fixture.event.tags {
            if tag.first().map(String::as_str) == Some("status") {
                tag[1] = "accepted".to_owned();
            }
        }
        assert_eq!(
            admit_independent_verification(&fixture.event, &binding),
            Err(VerificationRefusal::Malformed)
        );
    }

    #[test]
    fn a_verification_that_does_not_disclaim_payment_is_refused() {
        let (mut fixture, binding) = load(CANONICAL);
        for tag in &mut fixture.event.tags {
            if tag.first().map(String::as_str) == Some("cw_decides_payment") {
                tag[1] = "true".to_owned();
            }
        }
        assert_eq!(
            admit_independent_verification(&fixture.event, &binding),
            Err(VerificationRefusal::DecidesPaymentForbidden)
        );
    }
}
