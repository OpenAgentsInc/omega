use anyhow::{Context as _, Result, bail};
use lnmarkets_client::Network;
use lnmarkets_data::{
    CollectorHandle, MarketDataStore, STREAM_LAST_PRICE_TOPIC, StoredMarketEvent,
};
use prediction_events::{OutcomeSource, PredictedDirection, PredictionEvent, ResolvedOutcome};

pub const STORED_LAST_PRICE_SOURCE: &str = "lnmarkets:stored_last_price";

pub struct StoredPriceOutcomeSource<'a> {
    store: &'a MarketDataStore,
    network: Network,
}

impl<'a> StoredPriceOutcomeSource<'a> {
    pub fn new(collector: &'a CollectorHandle) -> Self {
        Self {
            store: collector.store(),
            network: collector.health().network,
        }
    }
}

impl OutcomeSource for StoredPriceOutcomeSource<'_> {
    fn resolve(&self, prediction: &PredictionEvent) -> Result<Option<ResolvedOutcome>> {
        if prediction.draft.resolution_rule.source != STORED_LAST_PRICE_SOURCE {
            return Ok(None);
        }
        if self.network != Network::Signet {
            bail!("prediction resolution is restricted to collected signet data");
        }
        let mut events = self.store.range(
            Network::Signet,
            STREAM_LAST_PRICE_TOPIC,
            prediction.draft.resolution_rule.baseline_at_ms,
            Some(prediction.draft.resolution_rule.resolve_at_ms),
            10_000,
        )?;
        events.sort_by_key(|event| event.event_time_ms);
        if events.len() < 2
            || events.first().map(|event| event.event_time_ms)
                == events.last().map(|event| event.event_time_ms)
        {
            return Ok(None);
        }
        let Some(baseline) = events.first().map(stored_price).transpose()? else {
            return Ok(None);
        };
        let Some(resolved) = events.last().map(stored_price).transpose()? else {
            return Ok(None);
        };
        if baseline <= 0.0 || !baseline.is_finite() || !resolved.is_finite() {
            bail!("stored prediction prices must be positive finite numbers");
        }
        let change_bps = (resolved - baseline) / baseline * 10_000.0;
        let tolerance_bps = f64::from(prediction.draft.resolution_rule.flat_tolerance_bps);
        let direction = if change_bps > tolerance_bps {
            PredictedDirection::Up
        } else if change_bps < -tolerance_bps {
            PredictedDirection::Down
        } else {
            PredictedDirection::Flat
        };
        Ok(Some(ResolvedOutcome::Direction { direction }))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lnmarkets_client::{StreamEvent, StreamTopic};
    use prediction_events::{
        MandateScope, PREDICTION_SCHEMA_VERSION, PredictionActor, PredictionEventDraft,
        PredictionForecast, ResolutionRule, ScoringRule,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn fixed_horizon_resolves_from_stored_signet_prices() {
        let baseline_at_ms = 1_786_233_600_000_i64;
        let resolve_at_ms = baseline_at_ms + 100;
        let store = MarketDataStore::in_memory(Duration::from_secs(60)).expect("market data");
        let topic = StreamTopic::new(STREAM_LAST_PRICE_TOPIC).expect("topic");
        store
            .insert_stream_batch(
                Network::Signet,
                &[
                    StreamEvent {
                        topic: topic.clone(),
                        data: json!({"time": baseline_at_ms, "lastPrice": 100.0}),
                    },
                    StreamEvent {
                        topic,
                        data: json!({"time": resolve_at_ms, "lastPrice": 102.0}),
                    },
                ],
            )
            .expect("stored observations");
        let prediction = PredictionEvent {
            sequence: 1,
            prediction_id: "prediction:test".into(),
            draft: PredictionEventDraft {
                schema_version: PREDICTION_SCHEMA_VERSION,
                emitted_at_ms: baseline_at_ms,
                actor: PredictionActor::Agent {
                    agent_id: "session".into(),
                },
                mandate_scope: MandateScope {
                    venue: "lnmarkets".into(),
                    network: crate::TradingNetwork::Signet,
                },
                instrument: "btc_usd".into(),
                forecast: PredictionForecast::Directional {
                    direction: PredictedDirection::Up,
                    probability_micros: 600_000,
                },
                confidence_micros: 600_000,
                horizon_ms: 100,
                resolution_rule: ResolutionRule {
                    source: STORED_LAST_PRICE_SOURCE.into(),
                    baseline_at_ms,
                    resolve_at_ms,
                    flat_tolerance_bps: 10,
                },
                scoring_rule: ScoringRule::Brier,
                observation_refs: Vec::new(),
                private_payload_ref: None,
                subsequent_decision_id: "decision".into(),
            },
        };
        let source = StoredPriceOutcomeSource {
            store: &store,
            network: Network::Signet,
        };

        assert_eq!(
            source.resolve(&prediction).expect("resolution"),
            Some(ResolvedOutcome::Direction {
                direction: PredictedDirection::Up,
            })
        );
    }
}

fn stored_price(event: &StoredMarketEvent) -> Result<f64> {
    let value = event
        .payload
        .get("lastPrice")
        .context("stored last-price event has no lastPrice field")?;
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .context("stored last-price event has a non-numeric lastPrice field")
}
