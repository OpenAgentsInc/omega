use std::{env, fs};

use anyhow::{Context as _, Result, bail};
use nautilus_governance::{NautilusSoakManifest, NautilusSoakStore};

fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    let command = arguments
        .next()
        .context("expected create, status, or finish")?;
    let directory = arguments.next().context("expected soak directory")?;
    match command.as_str() {
        "create" => {
            let manifest_path = arguments.next().context("expected manifest JSON path")?;
            ensure_finished(arguments)?;
            let manifest: NautilusSoakManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
            let store = NautilusSoakStore::create(directory, manifest)?;
            println!("{}", serde_json::to_string(store.manifest())?);
        }
        "status" => {
            ensure_finished(arguments)?;
            let store = NautilusSoakStore::open(directory)?;
            let samples = store.health_samples()?;
            println!(
                "{}",
                serde_json::json!({
                    "segment_id": store.manifest().segment_id,
                    "health_samples": samples.len(),
                    "health_head": samples.last().map(|sample| &sample.entry_hash),
                    "last_observed_at_ms": samples.last().map(|sample| sample.draft.observed_at_ms),
                })
            );
        }
        "finish" => {
            let ended_at_ms = arguments
                .next()
                .context("expected segment end timestamp")?
                .parse::<i64>()?;
            let human_nudge_count = arguments
                .next()
                .context("expected human nudge count")?
                .parse::<u64>()?;
            ensure_finished(arguments)?;
            let receipt =
                NautilusSoakStore::open(directory)?.finish(ended_at_ms, human_nudge_count)?;
            println!("{}", serde_json::to_string(&receipt)?);
        }
        _ => bail!("expected create, status, or finish"),
    }
    Ok(())
}

fn ensure_finished(mut arguments: impl Iterator<Item = String>) -> Result<()> {
    if arguments.next().is_some() {
        bail!("unexpected extra arguments");
    }
    Ok(())
}
