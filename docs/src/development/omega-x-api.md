# Omega X API tooling

Omega can read public X posts and users through a small Rust crate:

- crate: `crates/x_api`
- binary: `x-api`
- base URL: `https://api.x.com/2`

This is **not** the xAI Grok inference API. Grok models live in `crates/x_ai` and call `https://api.x.ai`. Grok can also search X through the server-side **`x_search`** tool on the Responses API; that path is model-mediated and billed as an xAI tool call. Direct structured post IDs, expansions, and recent-search operators come from the X API crate.

## Two access paths

| Path | Host | Credential | Best for |
| --- | --- | --- | --- |
| X API v2 | `api.x.com` | App Bearer (`X_BEARER_TOKEN`) | Deterministic lookup, recent search operators, post IDs, metrics |
| xAI `x_search` | `api.x.ai` | xAI API key (`XAI_API_KEY`) | Natural-language research over X via Grok |

Purchase of X API credits can include promotional xAI credits (see X pricing docs). The products remain separate APIs.

## Supported X API operations (current crate)

- `GET /2/users/by/username/:username`
- `GET /2/tweets/:id`
- `GET /2/tweets?ids=...` (up to 100)
- `GET /2/tweets/search/recent`

App-only Bearer covers public reads. Posting, DMs, follows, and account management need user-context OAuth and are intentionally out of scope.

## Environment

```sh
export X_BEARER_TOKEN='…app-only bearer from console.x.com…'
# optional alias
export X_API_BEARER_TOKEN="$X_BEARER_TOKEN"
```

Use the token string exactly as issued. Do not commit credentials. Do not put tokens in Omega settings that get synced or logged.

Optional for the Grok path:

```sh
export XAI_API_KEY='…xAI key…'
```

## CLI examples

```sh
cargo run -p x_api -- user COLDCARDwallet
cargo run -p x_api -- search 'coldcard -is:retweet lang:en' --max-results 25
cargo run -p x_api -- post 2084632863756955661
cargo run -p x_api -- posts 2084632863756955661,2084596971268956161
```

Search returns flattened views by default (`author_username`, `url`, metrics) plus `meta.next_token` for pagination.

## Library sketch

```rust
use x_api::{Client, RecentSearchParams, posts_with_authors};

let client = Client::from_env()?;
let page = client.recent_search(RecentSearchParams {
    query: "from:COLDCARDwallet -is:retweet".into(),
    max_results: Some(15),
    ..Default::default()
})?;
let views = posts_with_authors(
    page.data.as_deref().unwrap_or(&[]),
    page.includes.as_ref(),
);
```

## Verification

```sh
cargo test -p x_api
cargo run -p x_api -- user XDevelopers   # requires X_BEARER_TOKEN
```

Live network calls are not required for unit tests. They only exercise serde joins and client construction.

## Fold-in notes for product work

- Keep secrets outside the Omega identity/Keychain surface until a deliberate secret-store design exists.
- Prefer `x_api` for forensic timelines, exact post IDs, and operator-driven monitoring.
- Prefer Grok `x_search` when an agent needs synthesis across many posts and can tolerate tool-call cost and non-deterministic retrieval.
- Public summaries that cite X posts should mark claims as external observation, not accepted forensic evidence.

## Upstream docs

- X overview: <https://docs.x.com/overview>
- Post lookup: <https://docs.x.com/x-api/posts/lookup/introduction>
- Search: <https://docs.x.com/x-api/posts/search/introduction>
- xAI X Search tool: <https://docs.x.ai/developers/tools/x-search>
