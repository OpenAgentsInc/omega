//! X API v2 client for public post and user reads.
//!
//! App-only Bearer authentication against `https://api.x.com`. This is not the
//! xAI Grok inference API (`https://api.x.ai`); see `x_ai` for models and the
//! Grok `x_search` server tool for model-mediated X queries.

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::time::Duration;

pub const X_API_BASE_URL: &str = "https://api.x.com/2";
pub const DEFAULT_BEARER_ENV: &str = "X_BEARER_TOKEN";
pub const ALT_BEARER_ENV: &str = "X_API_BEARER_TOKEN";

const DEFAULT_TWEET_FIELDS: &str =
    "created_at,public_metrics,author_id,lang,conversation_id,entities,referenced_tweets";
const DEFAULT_USER_FIELDS: &str = "username,name,description,public_metrics,verified,created_at";

/// App-only X API v2 client.
#[derive(Clone, Debug)]
pub struct Client {
    base_url: String,
    bearer_token: String,
    http: reqwest::blocking::Client,
}

impl Client {
    pub fn new(bearer_token: impl Into<String>) -> Result<Self> {
        Self::with_base_url(X_API_BASE_URL, bearer_token)
    }

    pub fn with_base_url(
        base_url: impl Into<String>,
        bearer_token: impl Into<String>,
    ) -> Result<Self> {
        let bearer_token = bearer_token.into().trim().to_owned();
        if bearer_token.is_empty() {
            bail!("X API bearer token is empty");
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("omega-x-api/0.1")
            .build()
            .context("building X API HTTP client")?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            bearer_token,
            http,
        })
    }

    /// Load bearer from `X_BEARER_TOKEN` or `X_API_BEARER_TOKEN`.
    pub fn from_env() -> Result<Self> {
        let token = env::var(DEFAULT_BEARER_ENV)
            .or_else(|_| env::var(ALT_BEARER_ENV))
            .with_context(|| {
                format!(
                    "set {DEFAULT_BEARER_ENV} or {ALT_BEARER_ENV} to an X API app-only Bearer token"
                )
            })?;
        Self::new(token)
    }

    pub fn user_by_username(&self, username: &str) -> Result<UserLookupResponse> {
        let username = username.trim().trim_start_matches('@');
        if username.is_empty() {
            bail!("username is empty");
        }
        let path = format!("/users/by/username/{}", urlencoding::encode(username));
        let mut query = BTreeMap::new();
        query.insert("user.fields".to_owned(), DEFAULT_USER_FIELDS.to_owned());
        self.get_json(&path, &query)
    }

    pub fn post_by_id(&self, post_id: &str) -> Result<PostsResponse> {
        let post_id = post_id.trim();
        if post_id.is_empty() {
            bail!("post id is empty");
        }
        let path = format!("/tweets/{}", urlencoding::encode(post_id));
        self.get_json(&path, &self.default_post_query(true))
    }

    pub fn posts_by_ids(&self, post_ids: &[String]) -> Result<PostsResponse> {
        if post_ids.is_empty() {
            bail!("at least one post id is required");
        }
        if post_ids.len() > 100 {
            bail!("X API allows at most 100 post ids per request");
        }
        let mut query = self.default_post_query(true);
        query.insert("ids".to_owned(), post_ids.join(","));
        self.get_json("/tweets", &query)
    }

    pub fn recent_search(&self, params: RecentSearchParams) -> Result<SearchResponse> {
        let query_text = params.query.trim();
        if query_text.is_empty() {
            bail!("search query is empty");
        }
        let max_results = params.max_results.unwrap_or(10);
        if !(10..=100).contains(&max_results) {
            bail!("max_results must be between 10 and 100 inclusive (got {max_results})");
        }

        let mut query = self.default_post_query(true);
        query.insert("query".to_owned(), query_text.to_owned());
        query.insert("max_results".to_owned(), max_results.to_string());
        if let Some(next) = params
            .next_token
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            query.insert("next_token".to_owned(), next.to_owned());
        }
        if let Some(since_id) = params
            .since_id
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            query.insert("since_id".to_owned(), since_id.to_owned());
        }
        if let Some(until_id) = params
            .until_id
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            query.insert("until_id".to_owned(), until_id.to_owned());
        }
        self.get_json("/tweets/search/recent", &query)
    }

    fn default_post_query(&self, expand_author: bool) -> BTreeMap<String, String> {
        let mut query = BTreeMap::new();
        query.insert("tweet.fields".to_owned(), DEFAULT_TWEET_FIELDS.to_owned());
        if expand_author {
            query.insert("expansions".to_owned(), "author_id".to_owned());
            query.insert("user.fields".to_owned(), DEFAULT_USER_FIELDS.to_owned());
        }
        query
    }

    fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &BTreeMap<String, String>,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .http
            .get(&url)
            .query(query)
            .header("Authorization", format!("Bearer {}", self.bearer_token))
            .send()
            .with_context(|| format!("GET {url}"))?;

        let status = response.status();
        let body = response
            .text()
            .with_context(|| format!("reading body for GET {url}"))?;

        if !status.is_success() {
            bail!(
                "X API {} returned HTTP {}: {}",
                path,
                status.as_u16(),
                truncate_for_error(&body)
            );
        }

        // Surface API-level error envelopes even on 200 in some edge paths.
        if let Ok(envelope) = serde_json::from_str::<ApiErrorEnvelope>(&body) {
            if let Some(errors) = envelope.errors {
                if !errors.is_empty() && envelope.data.is_none() {
                    bail!(
                        "X API {} error: {}",
                        path,
                        truncate_for_error(&serde_json::to_string(&errors).unwrap_or_default())
                    );
                }
            }
            if let Some(title) = envelope.title {
                if envelope.data.is_none() && envelope.meta.is_none() {
                    bail!(
                        "X API {} error: {} — {}",
                        path,
                        title,
                        envelope.detail.unwrap_or_default()
                    );
                }
            }
        }

        serde_json::from_str(&body).with_context(|| {
            format!(
                "decoding X API JSON for {path}: {}",
                truncate_for_error(&body)
            )
        })
    }
}

fn truncate_for_error(body: &str) -> String {
    const LIMIT: usize = 800;
    let compact = body.replace('\n', " ");
    if compact.len() <= LIMIT {
        compact
    } else {
        format!("{}…", &compact[..LIMIT])
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecentSearchParams {
    pub query: String,
    pub max_results: Option<u32>,
    pub next_token: Option<String>,
    pub since_id: Option<String>,
    pub until_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserLookupResponse {
    pub data: Option<User>,
    #[serde(default)]
    pub errors: Option<Vec<Value>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PostsResponse {
    #[serde(default)]
    pub data: Option<Vec<Post>>,
    #[serde(default)]
    pub includes: Option<Includes>,
    #[serde(default)]
    pub errors: Option<Vec<Value>>,
    #[serde(default)]
    pub meta: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchResponse {
    #[serde(default)]
    pub data: Option<Vec<Post>>,
    #[serde(default)]
    pub includes: Option<Includes>,
    #[serde(default)]
    pub errors: Option<Vec<Value>>,
    #[serde(default)]
    pub meta: Option<SearchMeta>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchMeta {
    #[serde(default)]
    pub newest_id: Option<String>,
    #[serde(default)]
    pub oldest_id: Option<String>,
    #[serde(default)]
    pub result_count: Option<u32>,
    #[serde(default)]
    pub next_token: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Includes {
    #[serde(default)]
    pub users: Option<Vec<User>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub username: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub verified: Option<bool>,
    #[serde(default)]
    pub public_metrics: Option<PublicMetrics>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Post {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub author_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub public_metrics: Option<PublicMetrics>,
    #[serde(default)]
    pub entities: Option<Value>,
    #[serde(default)]
    pub referenced_tweets: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PublicMetrics {
    #[serde(default)]
    pub followers_count: Option<u64>,
    #[serde(default)]
    pub following_count: Option<u64>,
    #[serde(default)]
    pub tweet_count: Option<u64>,
    #[serde(default)]
    pub listed_count: Option<u64>,
    #[serde(default)]
    pub like_count: Option<u64>,
    #[serde(default)]
    pub retweet_count: Option<u64>,
    #[serde(default)]
    pub reply_count: Option<u64>,
    #[serde(default)]
    pub quote_count: Option<u64>,
    #[serde(default)]
    pub bookmark_count: Option<u64>,
    #[serde(default)]
    pub impression_count: Option<u64>,
}

#[derive(Deserialize)]
struct ApiErrorEnvelope {
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    meta: Option<Value>,
    #[serde(default)]
    errors: Option<Vec<Value>>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

/// Flatten posts with author username when includes are present.
pub fn posts_with_authors(posts: &[Post], includes: Option<&Includes>) -> Vec<PostView> {
    let users: BTreeMap<&str, &User> = includes
        .and_then(|includes| includes.users.as_ref())
        .map(|users| users.iter().map(|user| (user.id.as_str(), user)).collect())
        .unwrap_or_default();

    posts
        .iter()
        .map(|post| {
            let author = post
                .author_id
                .as_deref()
                .and_then(|id| users.get(id).copied());
            PostView {
                id: post.id.clone(),
                text: post.text.clone(),
                created_at: post.created_at.clone(),
                author_id: post.author_id.clone(),
                author_username: author.map(|user| user.username.clone()),
                author_name: author.map(|user| user.name.clone()),
                public_metrics: post.public_metrics.clone(),
                url: format!("https://x.com/i/web/status/{}", post.id),
            }
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PostView {
    pub id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_metrics: Option<PublicMetrics>,
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posts_with_authors_joins_includes() {
        let posts = vec![Post {
            id: "1".into(),
            text: "hello".into(),
            author_id: Some("42".into()),
            created_at: Some("2026-08-04T00:00:00.000Z".into()),
            lang: None,
            conversation_id: None,
            public_metrics: None,
            entities: None,
            referenced_tweets: None,
        }];
        let includes = Includes {
            users: Some(vec![User {
                id: "42".into(),
                name: "COLDCARD".into(),
                username: "COLDCARDwallet".into(),
                description: None,
                created_at: None,
                verified: None,
                public_metrics: None,
            }]),
        };
        let views = posts_with_authors(&posts, Some(&includes));
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].author_username.as_deref(), Some("COLDCARDwallet"));
        assert_eq!(views[0].url, "https://x.com/i/web/status/1");
    }

    #[test]
    fn rejects_empty_bearer() {
        let err = Client::new("   ").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn deserializes_search_payload() {
        let raw = r#"{
          "data": [{"id":"1","text":"hi","author_id":"9","created_at":"2026-08-04T15:00:00.000Z"}],
          "includes": {"users":[{"id":"9","name":"N","username":"n"}]},
          "meta": {"result_count": 1, "newest_id": "1", "oldest_id": "1"}
        }"#;
        let parsed: SearchResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.data.as_ref().unwrap().len(), 1);
        assert_eq!(parsed.meta.unwrap().result_count, Some(1));
    }
}
