use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use lnmarkets_client::{
    DecimalAmount, FuturesCrossPosition, FuturesIsolatedTrade, LightningWithdrawal,
    OnChainWithdrawal,
};
use lnmarkets_data::{CollectorHandle, StoredMarketEvent};
use serde_json::{Map, Value};
use trading_ledger::{AssetId, Counterparty, CounterpartySnapshot};

const VENUE: &str = "lnmarkets";
const CROSS_POSITION_TOPIC: &str = "futures/inverse/btc_usd/cross/position";
const ISOLATED_TRADES_TOPIC: &str = "futures/inverse/btc_usd/isolated/trades";
const WITHDRAWAL_TOPIC: &str = "wallet/withdrawal";
const STREAM_HISTORY_LIMIT: usize = 1_000;

pub(crate) fn snapshot_from_account_surfaces(
    observed_at_ms: i64,
    cross_position: &FuturesCrossPosition,
    isolated_open_trades: &[FuturesIsolatedTrade],
    isolated_running_trades: &[FuturesIsolatedTrade],
    lightning_withdrawals: &[LightningWithdrawal],
    on_chain_withdrawals: &[OnChainWithdrawal],
) -> Result<CounterpartySnapshot> {
    let mut isolated_claims = BTreeMap::new();
    for trade in isolated_open_trades
        .iter()
        .chain(isolated_running_trades.iter())
    {
        isolated_claims.insert(
            trade.id.as_str(),
            decimal_amount_i64("isolated unrealized P&L", &trade.pl)?,
        );
    }
    let unrealized_claims = isolated_claims.values().try_fold(
        decimal_amount_i64("cross unrealized P&L", &cross_position.total_pl)?,
        |total, claim| {
            total
                .checked_add(*claim)
                .context("unrealized counterparty claims overflowed")
        },
    )?;
    let lightning_in_flight = lightning_withdrawals
        .iter()
        .filter(|withdrawal| withdrawal_is_in_flight(&withdrawal.status))
        .try_fold(0_i64, |total, withdrawal| {
            total
                .checked_add(decimal_amount_i64(
                    "pending Lightning withdrawal",
                    &withdrawal.amount,
                )?)
                .context("pending Lightning withdrawals overflowed")
        })?;
    let in_flight_transfers = on_chain_withdrawals
        .iter()
        .filter(|withdrawal| withdrawal_is_in_flight(&withdrawal.status))
        .try_fold(lightning_in_flight, |total, withdrawal| {
            total
                .checked_add(decimal_amount_i64(
                    "pending on-chain withdrawal",
                    &withdrawal.amount,
                )?)
                .context("pending withdrawals overflowed")
        })?;
    Ok(venue_snapshot(
        observed_at_ms,
        unrealized_claims,
        in_flight_transfers,
    ))
}

pub(crate) fn snapshot_from_collector(
    collector: &CollectorHandle,
    observed_at_ms: i64,
) -> Result<Option<CounterpartySnapshot>> {
    let cross_events = collector.recent(CROSS_POSITION_TOPIC, STREAM_HISTORY_LIMIT)?;
    let isolated_events = collector.recent(ISOLATED_TRADES_TOPIC, STREAM_HISTORY_LIMIT)?;
    let withdrawal_events = collector.recent(WITHDRAWAL_TOPIC, STREAM_HISTORY_LIMIT)?;
    snapshot_from_stream_events(
        observed_at_ms,
        &cross_events,
        &isolated_events,
        &withdrawal_events,
    )
}

fn snapshot_from_stream_events(
    observed_at_ms: i64,
    cross_events: &[StoredMarketEvent],
    isolated_events: &[StoredMarketEvent],
    withdrawal_events: &[StoredMarketEvent],
) -> Result<Option<CounterpartySnapshot>> {
    if cross_events.is_empty() && isolated_events.is_empty() && withdrawal_events.is_empty() {
        return Ok(None);
    }

    let cross_claim = cross_events
        .iter()
        .find_map(cross_claim_from_event)
        .transpose()?
        .unwrap_or_default();
    let mut isolated_claims = BTreeMap::<String, i64>::new();
    for event in isolated_events {
        for object in candidate_objects(&event.payload) {
            let Some(id) = string_field(object, &["id", "tradeId", "trade_id"]) else {
                continue;
            };
            if isolated_claims.contains_key(id) {
                continue;
            }
            let active = boolean_field(object, &["open"]).unwrap_or(false)
                || boolean_field(object, &["running"]).unwrap_or(false)
                || string_field(object, &["status", "state"]).is_some_and(|status| {
                    matches!(status.to_ascii_lowercase().as_str(), "open" | "running")
                });
            let claim = if active {
                value_field(object, &["pl", "totalPl", "total_pl"])
                    .map(|value| value_i64("streamed isolated unrealized P&L", value))
                    .transpose()?
                    .unwrap_or_default()
            } else {
                0
            };
            isolated_claims.insert(id.to_owned(), claim);
        }
    }
    let unrealized_claims = isolated_claims
        .values()
        .try_fold(cross_claim, |total, claim| {
            total
                .checked_add(*claim)
                .context("streamed unrealized counterparty claims overflowed")
        })?;

    let mut withdrawals = BTreeMap::<String, i64>::new();
    for event in withdrawal_events {
        for object in candidate_objects(&event.payload) {
            let Some(id) = string_field(object, &["id", "withdrawalId", "withdrawal_id"]) else {
                continue;
            };
            if withdrawals.contains_key(id) {
                continue;
            }
            let amount = match (
                string_field(object, &["status"]),
                value_field(object, &["amount"]),
            ) {
                (Some(status), Some(amount)) if withdrawal_is_in_flight(status) => {
                    value_i64("streamed pending withdrawal", amount)?
                }
                _ => 0,
            };
            withdrawals.insert(id.to_owned(), amount);
        }
    }
    let in_flight_transfers = withdrawals.values().try_fold(0_i64, |total, amount| {
        total
            .checked_add(*amount)
            .context("streamed pending withdrawals overflowed")
    })?;
    Ok(Some(venue_snapshot(
        observed_at_ms,
        unrealized_claims,
        in_flight_transfers,
    )))
}

fn cross_claim_from_event(event: &StoredMarketEvent) -> Option<Result<i64>> {
    candidate_objects(&event.payload)
        .into_iter()
        .find_map(|object| {
            value_field(object, &["totalPl", "total_pl"])
                .map(|value| value_i64("streamed cross unrealized P&L", value))
        })
}

fn venue_snapshot(
    observed_at_ms: i64,
    unrealized_claims: i64,
    in_flight_transfers: i64,
) -> CounterpartySnapshot {
    CounterpartySnapshot {
        observed_at_ms,
        counterparty: Counterparty::Venue {
            venue: VENUE.to_owned(),
        },
        asset: AssetId::sats(),
        provider_balance_held: None,
        unrealized_claims,
        in_flight_transfers,
    }
}

fn decimal_amount_i64(label: &str, amount: &DecimalAmount) -> Result<i64> {
    number_text_i64(label, &amount.as_number().to_string())
}

fn value_i64(label: &str, value: &Value) -> Result<i64> {
    match value {
        Value::Number(number) => number_text_i64(label, &number.to_string()),
        Value::String(number) => number_text_i64(label, number),
        _ => bail!("{label} must be a decimal integer"),
    }
}

fn number_text_i64(label: &str, number: &str) -> Result<i64> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if fraction.chars().any(|digit| digit != '0') {
        bail!("{label} must be an integer in the asset's smallest unit");
    }
    whole
        .parse::<i64>()
        .with_context(|| format!("{label} is outside the supported integer range"))
}

fn withdrawal_is_in_flight(status: &str) -> bool {
    !matches!(
        status.to_ascii_lowercase().as_str(),
        "settled" | "completed" | "confirmed" | "failed" | "canceled" | "cancelled"
    )
}

fn candidate_objects(value: &Value) -> Vec<&Map<String, Value>> {
    let mut candidates = Vec::new();
    collect_candidate_objects(value, 0, &mut candidates);
    candidates
}

fn collect_candidate_objects<'a>(
    value: &'a Value,
    depth: usize,
    candidates: &mut Vec<&'a Map<String, Value>>,
) {
    if depth > 4 {
        return;
    }
    match value {
        Value::Object(object) => {
            candidates.push(object);
            for key in ["data", "payload", "withdrawal", "trade", "position"] {
                if let Some(nested) = object.get(key) {
                    collect_candidate_objects(nested, depth.saturating_add(1), candidates);
                }
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_candidate_objects(nested, depth.saturating_add(1), candidates);
            }
        }
        _ => {}
    }
}

fn value_field<'a>(object: &'a Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| object.get(*name))
}

fn string_field<'a>(object: &'a Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    value_field(object, names).and_then(Value::as_str)
}

fn boolean_field(object: &Map<String, Value>, names: &[&str]) -> Option<bool> {
    value_field(object, names).and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use lnmarkets_client::Network;
    use lnmarkets_data::EventSource;
    use serde_json::json;

    use super::*;

    #[test]
    fn private_stream_updates_are_deduplicated_by_entity() {
        let cross = [stream_event(
            CROSS_POSITION_TOPIC,
            4,
            json!({"totalPl": 40}),
        )];
        let isolated = [
            stream_event(
                ISOLATED_TRADES_TOPIC,
                3,
                json!({"id": "trade-1", "closed": true, "pl": 30}),
            ),
            stream_event(
                ISOLATED_TRADES_TOPIC,
                2,
                json!({"id": "trade-1", "running": true, "pl": 25}),
            ),
        ];
        let withdrawals = [stream_event(
            WITHDRAWAL_TOPIC,
            1,
            json!({"id": "withdrawal-1", "status": "pending", "amount": 20}),
        )];

        let snapshot = snapshot_from_stream_events(100, &cross, &isolated, &withdrawals)
            .expect("snapshot")
            .expect("private events");
        assert_eq!(snapshot.unrealized_claims, 40);
        assert_eq!(snapshot.in_flight_transfers, 20);
    }

    fn stream_event(topic: &str, event_time_ms: i64, payload: Value) -> StoredMarketEvent {
        StoredMarketEvent {
            network: Network::Signet,
            topic: topic.to_owned(),
            event_time_ms,
            received_at_ms: event_time_ms,
            source: EventSource::Stream,
            payload,
        }
    }
}
