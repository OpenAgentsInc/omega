//! Typed provider-network projection and multi-relay threshold accounting.
//!
//! Immortal verifies the signed relay-set and key-rotation chains. This module
//! applies the verified availability policy to Omega's independent sockets;
//! relay counts never upgrade signing, settlement, or custody authority.
//! Policy record: OMEGA-DELTA-0266.

use std::collections::{BTreeMap, BTreeSet};

use immortal_client::domain::{
    Event, MKT_SWP_KEY_ROTATION_KIND, MKT_SWP_RELAY_SET_KIND, MktEventIdAdmission,
    MktEventIdDeduplicator, MktProviderKeyChain, MktRelaySet, MktRelaySetChain,
    verify_mkt_key_rotation_chain, verify_mkt_relay_set_chain,
};
use immortal_client::market_network::MktRelaySetClient;

use crate::session_transport::SessionInbox;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelaySetPlan {
    pub relay_set_id: Option<String>,
    pub relays: Vec<String>,
    pub publish_minimum: usize,
    pub read_minimum: usize,
    verified_relay_set: Option<MktRelaySet>,
}

impl RelaySetPlan {
    pub fn from_verified(relay_set: &MktRelaySet) -> Self {
        Self {
            relay_set_id: Some(relay_set.relay_set_id.clone()),
            relays: relay_set.relays.clone(),
            publish_minimum: relay_set.publish_minimum,
            read_minimum: relay_set.read_minimum,
            verified_relay_set: Some(relay_set.clone()),
        }
    }

    /// Compatibility for a provider that has not published the optional
    /// network events. This is visibly a bootstrap path, never represented as
    /// a verified relay set.
    pub fn legacy_bootstrap(relay_url: String) -> Self {
        Self {
            relay_set_id: None,
            relays: vec![relay_url],
            publish_minimum: 1,
            read_minimum: 1,
            verified_relay_set: None,
        }
    }

    pub fn is_signed(&self) -> bool {
        self.relay_set_id.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderNetworkState {
    provider_id: String,
    events: Vec<Event>,
    key_chain: MktProviderKeyChain,
    relay_set_chain: Option<MktRelaySetChain>,
}

impl ProviderNetworkState {
    pub fn for_active_signer(
        signer_pubkey: &str,
        observed_at: u64,
        events: &[Event],
    ) -> Result<Self, String> {
        let provider_ids = events
            .iter()
            .filter_map(|event| event.tag_values("provider").next())
            .collect::<BTreeSet<_>>();
        let mut selected = None;
        for provider_id in provider_ids {
            let provider_events = events
                .iter()
                .filter(|event| event.tag_values("provider").next() == Some(provider_id))
                .cloned()
                .collect::<Vec<_>>();
            let could_authorize_signer = provider_id == signer_pubkey
                || provider_events.iter().any(|event| {
                    event.pubkey == signer_pubkey
                        || event.tags.iter().any(|tag| {
                            tag.name() == Some("p")
                                && tag.value() == Some(signer_pubkey)
                                && tag.as_slice().get(3).map(String::as_str) == Some("successor")
                        })
                });
            if !could_authorize_signer {
                continue;
            }
            let state = Self::verify(provider_id, &provider_events)?;
            if state.active_pubkey_at(observed_at) == signer_pubkey {
                if selected.is_some() {
                    return Err(
                        "provider signer matches more than one verified stable identity".to_owned(),
                    );
                }
                selected = Some(state);
            }
        }
        selected.map_or_else(|| Self::verify(signer_pubkey, &[]), Ok)
    }

    pub fn verify(provider_id: &str, events: &[Event]) -> Result<Self, String> {
        let mut network_events = events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    MKT_SWP_KEY_ROTATION_KIND | MKT_SWP_RELAY_SET_KIND
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        network_events.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        let rotations = network_events
            .iter()
            .filter(|event| event.kind == MKT_SWP_KEY_ROTATION_KIND)
            .cloned()
            .collect::<Vec<_>>();
        let relay_sets = network_events
            .iter()
            .filter(|event| event.kind == MKT_SWP_RELAY_SET_KIND)
            .cloned()
            .collect::<Vec<_>>();
        let key_chain = verify_mkt_key_rotation_chain(provider_id, &rotations)
            .map_err(|error| error.to_string())?;
        let relay_set_chain = if relay_sets.is_empty() {
            None
        } else {
            Some(
                verify_mkt_relay_set_chain(provider_id, &relay_sets, &key_chain)
                    .map_err(|error| error.to_string())?,
            )
        };
        Ok(Self {
            provider_id: provider_id.to_owned(),
            events: network_events,
            key_chain,
            relay_set_chain,
        })
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn active_pubkey_at(&self, created_at: u64) -> &str {
        self.key_chain.active_pubkey_at(created_at)
    }

    pub fn validate_provider_event(&self, event: &Event) -> Result<(), String> {
        self.key_chain
            .validate_provider_event(event)
            .map_err(|error| error.to_string())
    }

    pub fn key_chain(&self) -> &MktProviderKeyChain {
        &self.key_chain
    }

    pub fn relay_plan_at(&self, observed_at: u64) -> Option<RelaySetPlan> {
        self.relay_set_chain
            .as_ref()
            .and_then(|chain| chain.effective_at(observed_at))
            .map(RelaySetPlan::from_verified)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayAvailability {
    Unavailable,
    Available,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiRelayStatus {
    plan: RelaySetPlan,
    requester_live: BTreeSet<String>,
    response_live: BTreeSet<String>,
    disconnected: BTreeSet<String>,
    publish_acks: BTreeMap<String, BTreeSet<String>>,
    publish_failures: BTreeMap<String, BTreeSet<String>>,
    signed_requester: Option<MktRelaySetClient>,
    signed_response: Option<MktRelaySetClient>,
    legacy_deduplicator: MktEventIdDeduplicator,
}

impl MultiRelayStatus {
    pub fn new(plan: RelaySetPlan) -> Result<Self, String> {
        let signed_requester = plan
            .verified_relay_set
            .clone()
            .map(MktRelaySetClient::new)
            .transpose()?;
        let signed_response = plan
            .verified_relay_set
            .clone()
            .map(MktRelaySetClient::new)
            .transpose()?;
        Ok(Self {
            plan,
            requester_live: BTreeSet::new(),
            response_live: BTreeSet::new(),
            disconnected: BTreeSet::new(),
            publish_acks: BTreeMap::new(),
            publish_failures: BTreeMap::new(),
            signed_requester,
            signed_response,
            legacy_deduplicator: MktEventIdDeduplicator::default(),
        })
    }

    pub fn plan(&self) -> &RelaySetPlan {
        &self.plan
    }

    pub fn subscription_live(
        &mut self,
        relay_url: &str,
        inbox: SessionInbox,
    ) -> Result<(), String> {
        self.disconnected.remove(relay_url);
        match inbox {
            SessionInbox::Requester => &mut self.requester_live,
            SessionInbox::Response => &mut self.response_live,
        }
        .insert(relay_url.to_owned());
        let client = match inbox {
            SessionInbox::Requester => self.signed_requester.as_mut(),
            SessionInbox::Response => self.signed_response.as_mut(),
        };
        if let Some(client) = client {
            client.mark_read_ready(relay_url)?;
        }
        Ok(())
    }

    pub fn disconnected(&mut self, relay_url: &str, inbox: SessionInbox) -> Result<(), String> {
        match inbox {
            SessionInbox::Requester => &mut self.requester_live,
            SessionInbox::Response => &mut self.response_live,
        }
        .remove(relay_url);
        self.disconnected.insert(relay_url.to_owned());
        let client = match inbox {
            SessionInbox::Requester => self.signed_requester.as_mut(),
            SessionInbox::Response => self.signed_response.as_mut(),
        };
        if let Some(client) = client {
            client.mark_unavailable(relay_url)?;
        }
        Ok(())
    }

    pub fn publish_result(
        &mut self,
        relay_url: &str,
        event_id: &str,
        accepted: bool,
    ) -> Result<(), String> {
        let target = if accepted {
            &mut self.publish_acks
        } else {
            &mut self.publish_failures
        };
        target
            .entry(event_id.to_owned())
            .or_default()
            .insert(relay_url.to_owned());
        if let Some(client) = self.signed_requester.as_mut() {
            client.record_publication_ack(relay_url, event_id, accepted)?;
        }
        Ok(())
    }

    pub fn read_availability(&self, inbox: SessionInbox) -> RelayAvailability {
        let live = match inbox {
            SessionInbox::Requester => &self.requester_live,
            SessionInbox::Response => &self.response_live,
        };
        let signed = match inbox {
            SessionInbox::Requester => self.signed_requester.as_ref(),
            SessionInbox::Response => self.signed_response.as_ref(),
        };
        if let Some(client) = signed {
            if !client.read_available() {
                RelayAvailability::Unavailable
            } else if client.is_degraded() || live.len() < self.plan.relays.len() {
                RelayAvailability::Degraded
            } else {
                RelayAvailability::Available
            }
        } else {
            availability(live.len(), self.plan.read_minimum, self.plan.relays.len())
        }
    }

    pub fn publish_availability(&self, event_id: &str) -> RelayAvailability {
        let acknowledged = self.publish_acks.get(event_id).map_or(0, BTreeSet::len);
        if let Some(client) = self.signed_requester.as_ref() {
            if !client.publication_available(event_id) {
                RelayAvailability::Unavailable
            } else if acknowledged < self.plan.relays.len() {
                RelayAvailability::Degraded
            } else {
                RelayAvailability::Available
            }
        } else {
            availability(
                acknowledged,
                self.plan.publish_minimum,
                self.plan.relays.len(),
            )
        }
    }

    pub fn observe_event(
        &mut self,
        relay_url: &str,
        event: &Event,
    ) -> Result<MktEventIdAdmission, String> {
        match self.signed_requester.as_mut() {
            Some(client) => client
                .observe_event(relay_url, event)
                .map_err(|error| error.to_string()),
            None => self
                .legacy_deduplicator
                .observe(event)
                .map_err(|error| error.to_string()),
        }
    }

    pub fn seed_event(&mut self, event: &Event) -> Result<MktEventIdAdmission, String> {
        match self.signed_requester.as_mut() {
            Some(client) => {
                let relay = self
                    .plan
                    .relays
                    .first()
                    .ok_or_else(|| "signed relay plan is empty".to_owned())?;
                client
                    .observe_event(relay, event)
                    .map_err(|error| error.to_string())
            }
            None => self
                .legacy_deduplicator
                .observe(event)
                .map_err(|error| error.to_string()),
        }
    }
}

fn availability(observed: usize, required: usize, total: usize) -> RelayAvailability {
    if observed < required {
        RelayAvailability::Unavailable
    } else if observed < total {
        RelayAvailability::Degraded
    } else {
        RelayAvailability::Available
    }
}

/// Queues one already-signed event, byte-identically after serialization, for
/// every independent relay publisher. The caller applies the signed
/// `publish_minimum` to relay acknowledgments, not to queue success.
pub fn fanout_exact_event(
    event: &Event,
    outgoing: &BTreeMap<String, async_channel::Sender<Event>>,
) -> Result<(), Vec<String>> {
    let mut failed = Vec::new();
    for (relay, sender) in outgoing {
        if sender.try_send(event.clone()).is_err() {
            failed.push(relay.clone());
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed)
    }
}

#[cfg(test)]
mod tests {
    use immortal_client::domain::{MKT_NETWORK_VERSION, MKT_RELAY_SET_SCHEMA};
    use immortal_client::market::MarketSigner;

    use super::*;

    fn plan(publish_minimum: usize, read_minimum: usize) -> RelaySetPlan {
        RelaySetPlan::from_verified(&MktRelaySet {
            schema: MKT_RELAY_SET_SCHEMA.to_owned(),
            version: MKT_NETWORK_VERSION,
            relay_set_id: "11".repeat(32),
            provider_id: "22".repeat(32),
            generation: 1,
            previous_relay_set_event_id: None,
            effective_at: 1,
            relays: vec!["wss://a.example".to_owned(), "wss://b.example".to_owned()],
            publish_minimum,
            read_minimum,
        })
    }

    #[test]
    fn one_relay_down_remains_typed_degraded_at_threshold_one() {
        let mut status = MultiRelayStatus::new(plan(1, 1)).expect("valid relay plan");
        status
            .subscription_live("wss://a.example", SessionInbox::Requester)
            .expect("requester live");
        status
            .subscription_live("wss://a.example", SessionInbox::Response)
            .expect("response live");
        status
            .disconnected("wss://b.example", SessionInbox::Requester)
            .expect("requester unavailable");
        status
            .disconnected("wss://b.example", SessionInbox::Response)
            .expect("response unavailable");
        assert_eq!(
            status.read_availability(SessionInbox::Requester),
            RelayAvailability::Degraded
        );
        assert_eq!(
            status.read_availability(SessionInbox::Response),
            RelayAvailability::Degraded
        );
        status
            .publish_result("wss://a.example", "event", true)
            .expect("publish accepted");
        status
            .publish_result("wss://b.example", "event", false)
            .expect("publish refused");
        assert_eq!(
            status.publish_availability("event"),
            RelayAvailability::Degraded
        );
    }

    #[test]
    fn stricter_signed_threshold_is_never_weakened() {
        let mut status = MultiRelayStatus::new(plan(2, 2)).expect("valid relay plan");
        status
            .subscription_live("wss://a.example", SessionInbox::Requester)
            .expect("requester live");
        status
            .publish_result("wss://a.example", "event", true)
            .expect("publish accepted");
        assert_eq!(
            status.read_availability(SessionInbox::Requester),
            RelayAvailability::Unavailable
        );
        assert_eq!(
            status.publish_availability("event"),
            RelayAvailability::Unavailable
        );
    }

    #[test]
    fn fanout_sends_the_same_signed_event_to_every_relay() {
        let signer = MarketSigner::from_secret_bytes([7; 32]).expect("test key");
        let event = signer.sign(100, 39_605, Vec::new(), "{}".to_owned());
        let (sender_a, receiver_a) = async_channel::bounded(1);
        let (sender_b, receiver_b) = async_channel::bounded(1);
        let outgoing = BTreeMap::from([
            ("wss://a.example".to_owned(), sender_a),
            ("wss://b.example".to_owned(), sender_b),
        ]);
        fanout_exact_event(&event, &outgoing).expect("fanout queues");
        assert_eq!(receiver_a.try_recv(), Ok(event.clone()));
        assert_eq!(receiver_b.try_recv(), Ok(event));
    }
}
