use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use x_api::{Client, Includes, Post, RecentSearchParams, posts_with_authors};

#[derive(Parser, Debug)]
#[command(
    name = "x-api",
    about = "X API v2 CLI (app-only Bearer). Not the xAI Grok API.",
    long_about = "Query public posts and users via api.x.com.\n\
Set X_BEARER_TOKEN (or X_API_BEARER_TOKEN) from console.x.com.\n\
For model-mediated X search use the xAI x_search tool on api.x.ai instead."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Look up a user by username (without @).
    User { username: String },
    /// Look up a single post by id.
    Post {
        id: String,
        /// Emit the raw API envelope.
        #[arg(long, default_value_t = false)]
        raw: bool,
    },
    /// Look up up to 100 posts by comma-separated ids.
    Posts {
        /// Comma-separated post ids.
        ids: String,
        /// Emit the raw API envelope.
        #[arg(long, default_value_t = false)]
        raw: bool,
    },
    /// Recent search (last 7 days).
    Search {
        /// X query operators, e.g. `coldcard -is:retweet lang:en`.
        query: String,
        #[arg(long, default_value_t = 10)]
        max_results: u32,
        #[arg(long)]
        next_token: Option<String>,
        /// Emit the raw API envelope instead of flattened views.
        #[arg(long, default_value_t = false)]
        raw: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = Client::from_env()?;

    let value = match cli.command {
        Command::User { username } => serde_json::to_value(client.user_by_username(&username)?)?,
        Command::Post { id, raw } => {
            let response = client.post_by_id(&id)?;
            posts_payload(
                response.data.as_deref(),
                response.includes.as_ref(),
                raw,
                serde_json::to_value(&response)?,
            )?
        }
        Command::Posts { ids, raw } => {
            let list: Vec<String> = ids
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if list.is_empty() {
                bail!("provide at least one post id");
            }
            let response = client.posts_by_ids(&list)?;
            posts_payload(
                response.data.as_deref(),
                response.includes.as_ref(),
                raw,
                serde_json::to_value(&response)?,
            )?
        }
        Command::Search {
            query,
            max_results,
            next_token,
            raw,
        } => {
            let response = client.recent_search(RecentSearchParams {
                query,
                max_results: Some(max_results),
                next_token,
                ..Default::default()
            })?;
            if raw {
                serde_json::to_value(response)?
            } else {
                let views = posts_with_authors(
                    response.data.as_deref().unwrap_or(&[]),
                    response.includes.as_ref(),
                );
                json!({
                    "meta": response.meta,
                    "posts": views,
                })
            }
        }
    };

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn posts_payload(
    data: Option<&[Post]>,
    includes: Option<&Includes>,
    raw: bool,
    raw_value: Value,
) -> Result<Value> {
    if raw {
        return Ok(raw_value);
    }
    let views = posts_with_authors(data.unwrap_or(&[]), includes);
    Ok(json!({ "posts": views }))
}
