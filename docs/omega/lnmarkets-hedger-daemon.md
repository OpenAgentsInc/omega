# ADR: Incubate the LN Markets provider hedger in Omega

- Status: Accepted
- Date: 2026-08-09
- Issue: OpenAgentsInc/omega#268

## Context

An Immortal provider holds Bitcoin inventory across payment rails.
Its dollar exposure can make a fixed spread unsafe.
LN Markets can hedge this exposure with a cross-margin short or synthetic USD.

The provider and relay crates cannot hold a venue credential.
The OpenAgents API must not receive this credential.
The Omega desktop process must not own this daemon.

## Decision

Add the standalone `lnmarkets-hedger` program to the Omega repository.
Keep its library free of Omega and Immortal dependencies.
Reuse `lnmarkets_client` and the venue-neutral `trading_ledger` crate.

The daemon reads one deployment configuration file.
The file supplies the provider inventory target and hedge limits.
The daemon reads the venue credential from a separate mounted secret file.
An OpenAgents deployment must mount that file from Google Secret Manager.

The incubation program supports Signet only.
It rejects a Mainnet configuration before it creates a client.
One cycle makes at most one venue mutation.
The LN Markets client sends each mutation once and does not retry it.

The cross-margin mode maintains a short position against the inventory target.
It deposits a bounded margin top-up before it changes exposure when liquidation distance is low.
The synthetic-USD mode maintains the configured share of inventory in synthetic USD.

The daemon imports fills, fees, and funding settlements into the append-only sats ledger.
It records unexplained venue-balance movement as attributed trading profit or loss.
Each cycle reports profit, fees, funding, position state, and its one selected action.

## Consequences

The daemon can run beside a provider deployment without changing Immortal.
Its credential does not enter Omega, Immortal, Convex, or the OpenAgents API.
The daemon can restart without duplicating ledger events.

The repository placement is temporary.
Extraction requires the criteria in the LN Markets plan.
A Mainnet deployment needs a new decision, a passing evaluation receipt, and operator approval.

## Deployment contract

Build `lnmarkets-hedger` as a separate image or binary.
Mount a JSON secret at an owner-only path when the platform permits it.
Set `LNMARKETS_HEDGER_CREDENTIALS_FILE` to that path.
The JSON object contains `access_key`, `secret`, and `passphrase` strings.

Pass `--config` with a public deployment configuration file. Start from
`crates/lnmarkets_hedger/config.example.json` and set the provider inventory
and limits for that deployment.
Pass `--ledger` with a durable volume path.
Use `--once` to run one bounded cycle. That cycle can make one venue mutation.
Do not put the credential in an environment variable, command argument, log, or repository file.

## Verification

Unit tests use a fake venue and fake credentials only.
They verify cross-margin sizing, synthetic-USD sizing, margin protection, ledger idempotency, and evaluation math.
The repository policy test verifies the dependency and credential boundaries.
