# x_api

Rust client and CLI for the **X API v2** (posts, users, recent search).

This is separate from the **xAI Grok API** (`crates/x_ai`). Grok can also search X through the server-side `x_search` tool on `api.x.ai`; this crate talks to `api.x.com` directly with an app-only Bearer token.

## Auth

Set one of:

- `X_BEARER_TOKEN` (preferred)
- `X_API_BEARER_TOKEN`

Use the Bearer token **exactly** as issued in the [X Developer Console](https://console.x.com). Do not commit tokens.

App-only Bearer auth covers public read endpoints (lookup, recent search). User-context actions (post, DM, follows) need OAuth 1.0a or OAuth 2.0 user tokens and are out of scope for this crate.

## CLI

```sh
# From the omega workspace root
cargo run -p x_api -- user XDevelopers
cargo run -p x_api -- search 'coldcard -is:retweet lang:en' --max-results 10
cargo run -p x_api -- post 2084632863756955661
cargo run -p x_api -- posts 2084632863756955661,2084596971268956161
```

JSON on stdout. Errors on stderr with non-zero exit.

## Library

```rust
use x_api::{Client, RecentSearchParams};

let client = Client::from_env()?;
let user = client.user_by_username("COLDCARDwallet")?;
let page = client.recent_search(RecentSearchParams {
    query: "from:COLDCARDwallet -is:retweet".into(),
    max_results: Some(10),
    ..Default::default()
})?;
```

## Docs

- Design and access notes: `docs/src/development/omega-x-api.md`
- Upstream X API: <https://docs.x.com/overview>
- Upstream xAI X Search tool: <https://docs.x.ai/developers/tools/x-search>
