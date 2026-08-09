# LN Markets endpoint parity

This checklist records the API surfaces exposed by the LN Markets TypeScript and Python SDKs that are present in the local SDK checkout. The audit used these sources on 2026-08-09:

- `sdk-typescript/src/rest/v3/routes`
- `sdk-python/src/lnmarkets_sdk/rest/v3/http/client`
- `api-js/src/rest.ts`
- `api-python/lnmarkets/rest.py`

The Rust client implements every v3 route below. A checked item means that a typed public method, request model, response model, and route test exist in `src/lnmarkets_client.rs`.

## v3 REST

### Service and market data

- [x] `GET /ping`
- [x] `GET /time`
- [x] `GET /futures/ticker`
- [x] `GET /futures/leaderboard`
- [x] `GET /futures/funding-settlements`
- [x] `GET /futures/candles`
- [x] `GET /oracle/index`
- [x] `GET /oracle/last-price`
- [x] `GET /synthetic-usd/best-price`
- [x] `GET /synthetic-usd/swaps`
- [x] `POST /synthetic-usd/swap`

### Account

- [x] `GET /account`
- [x] `GET /account/address/bitcoin`
- [x] `POST /account/address/bitcoin`
- [x] `POST /account/deposit/lightning`
- [x] `POST /account/withdraw/lightning`
- [x] `POST /account/withdraw/on-chain`
- [x] `GET /account/deposits/lightning`
- [x] `GET /account/withdrawals/lightning`
- [x] `GET /account/deposits/on-chain`
- [x] `GET /account/withdrawals/on-chain`
- [x] `GET /account/notifications`
- [x] `PUT /account/notifications`

### Futures cross margin

- [x] `GET /futures/cross/position`
- [x] `GET /futures/cross/orders/open`
- [x] `GET /futures/cross/orders/filled`
- [x] `GET /futures/cross/funding-fees`
- [x] `GET /futures/cross/transfers`
- [x] `POST /futures/cross/order`
- [x] `POST /futures/cross/order/cancel`
- [x] `POST /futures/cross/orders/cancel-all`
- [x] `POST /futures/cross/position/close`
- [x] `PUT /futures/cross/leverage`
- [x] `POST /futures/cross/deposit`
- [x] `POST /futures/cross/withdraw`

### Futures isolated margin

- [x] `GET /futures/isolated/trades/open`
- [x] `GET /futures/isolated/trades/running`
- [x] `GET /futures/isolated/trades/closed`
- [x] `GET /futures/isolated/trades/canceled` (Python SDK addition)
- [x] `GET /futures/isolated/funding-fees`
- [x] `POST /futures/isolated/trade`
- [x] `POST /futures/isolated/trade/close`
- [x] `POST /futures/isolated/trade/cancel`
- [x] `POST /futures/isolated/trades/cancel-all`
- [x] `POST /futures/isolated/trade/add-margin`
- [x] `POST /futures/isolated/trade/cash-in`
- [x] `PUT /futures/isolated/trade/stoploss`
- [x] `DELETE /futures/isolated/trade/stoploss`
- [x] `PUT /futures/isolated/trade/takeprofit`
- [x] `DELETE /futures/isolated/trade/takeprofit`

## v2 REST

The v2 clients contain old futures and account routes that v3 supersedes. The Rust client uses the v3 equivalents for those operations. It implements all v2-only surfaces:

- [x] `GET /options/instruments`
- [x] `GET /options/instrument`
- [x] `GET /options/market`
- [x] `GET /options/volatility-index`
- [x] `POST /options`
- [x] `GET /options`
- [x] `GET /options/trades/{id}`
- [x] `PUT /options`
- [x] `DELETE /options`
- [x] `DELETE /options/all/close`
- [x] `POST /user/transfer`
- [x] `POST /user/deposit/susd`
- [x] `POST /user/withdraw/susd`
- [x] `GET /lnurl/auth`
- [x] `POST /lnurl/auth`

LN Markets currently returns HTTP 404 for the v2 base URLs on Signet and Mainnet. The client keeps the models and signer, but callers must treat the v2 routes as unavailable until the venue restores or replaces them.

## Stream v1

- [x] `hello`
- [x] `ping`
- [x] `time`
- [x] `authenticate`
- [x] `whoami`
- [x] `subscribe`
- [x] `unsubscribe`
- [x] `unsubscribeAll`
- [x] All public and private SDK topics
- [x] Automatic reconnect with hello, authentication, and subscription replay
