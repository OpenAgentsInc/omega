//! `OMEGA-DELTA-0111`. The agent-side half of `OMEGA-DELTA-0103`: every native
//! tool's result becomes a versioned artifact, and the event carries a preview.
//!
//! `OMEGA-DELTA-0103` bounded the terminal path, which is where the owner's
//! forty lines of Nostr hex came from. It left every other native tool
//! unbounded: `read_file`, `grep`, `edit_file`, `fetch`, `diagnostics` and the
//! MCP tools all put whatever they produced onto the record whole. The bound
//! has to be a property of the record, so it cannot be a property of one tool.
//!
//! The law itself — the artifact, the versioned address, the one truncation
//! sentence, and the reserve computed off that sentence — lives in
//! `acp_thread::tool_result_artifact` and is reused here rather than restated.
//! What this module adds is the part that was missing: **somewhere for the
//! address to resolve**. A marker naming an artifact the model has no way to
//! fetch is worse than no marker, because it reads as a fetch that is available
//! and is not.
//!
//! Nothing here can reach a file, a window, or a clock.

use std::sync::Arc;

use acp_thread::{
    TOOL_RESULT_PREVIEW_BYTE_BUDGET, ToolResultArtifact, ToolResultArtifactId,
    ToolResultArtifactStore, ToolResultPreview, preview_tool_result,
};
use collections::HashMap;

/// The prefix every native tool's artifact source carries.
///
/// The terminal path uses `terminal:<id>`; this one uses `tool:<tool_call_id>`.
/// Distinct prefixes are load-bearing: the two stores are separate, so an
/// address that resolved in the wrong one would hand back some other tool's
/// result under the name of this one.
pub const TOOL_ARTIFACT_SOURCE_PREFIX: &str = "tool:";

/// The separator between a source and its version in a rendered address.
pub const TOOL_ARTIFACT_VERSION_SEPARATOR: &str = "@v";

/// The source a tool call's results are recorded under.
pub fn tool_result_artifact_source(tool_call_id: &str) -> String {
    format!("{TOOL_ARTIFACT_SOURCE_PREFIX}{tool_call_id}")
}

/// Every complete tool result this thread produced, by source.
///
/// **Not persisted.** Like `Thread::sandbox_grants`, this lives and dies with
/// the thread's process. What that costs, and why it is not papered over, is
/// [`ArtifactLookup::Forgotten`].
#[derive(Debug, Default)]
pub struct ToolResultArtifactRegistry {
    stores: HashMap<Arc<str>, ToolResultArtifactStore>,
}

/// What an address resolved to. Three answers, not two.
///
/// A bare `Option` would collapse "this thread never recorded anything under
/// that source" into "that version does not exist", and the two send a reader
/// to different places: the first is a restart or a wrong address, the second
/// is an off-by-one in a version the reader can still ask for correctly.
#[derive(Debug, Clone)]
pub enum ArtifactLookup<'a> {
    /// The complete result.
    Found(&'a ToolResultArtifact),
    /// The source exists here; this version of it does not. Carries the
    /// versions that do, so the answer names a real address to ask with.
    NoSuchVersion { available: Vec<u32> },
    /// Nothing was ever recorded under this source *in this process*. This is
    /// also what a perfectly good address from before a restart gets, and the
    /// sentence for it must say so — see [`Self::sentence`].
    Forgotten,
}

/// Compared by address, not by body. Two lookups are the same answer when they
/// point at the same place; comparing the text would make an assertion about
/// *which* result resolved into an assertion about how long it is.
impl PartialEq for ArtifactLookup<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Found(left), Self::Found(right)) => left.id() == right.id(),
            (Self::NoSuchVersion { available: left }, Self::NoSuchVersion { available: right }) => {
                left == right
            }
            (Self::Forgotten, Self::Forgotten) => true,
            _ => false,
        }
    }
}

impl Eq for ArtifactLookup<'_> {}

impl ArtifactLookup<'_> {
    #[must_use]
    pub fn artifact(&self) -> Option<&ToolResultArtifact> {
        match self {
            Self::Found(artifact) => Some(artifact),
            _ => None,
        }
    }

    /// The sentence the model is shown when the fetch does not resolve.
    ///
    /// Every one of these says what happened rather than "not found". An
    /// unresolvable address is the failure `OMEGA-DELTA-0103` exists to
    /// prevent, so when it happens anyway the reader is owed the reason: a
    /// reader told only "no such artifact" concludes the result never existed,
    /// which is the false-absence class the marker was built against.
    #[must_use]
    pub fn sentence(&self, address: &ToolResultArtifactId) -> Option<String> {
        match self {
            Self::Found(_) => None,
            Self::NoSuchVersion { available } => {
                let versions = available
                    .iter()
                    .map(|version| {
                        format!(
                            "{}{TOOL_ARTIFACT_VERSION_SEPARATOR}{version}",
                            address.source()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(if versions.is_empty() {
                    format!(
                        "No version of `{}` is recorded. {ARTIFACTS_ARE_NOT_PERSISTED}",
                        address.source()
                    )
                } else {
                    format!(
                        "There is no `{address}`. The versions recorded for that \
                         tool call are: {versions}."
                    )
                })
            }
            Self::Forgotten => Some(format!(
                "No tool result is recorded at `{address}` in this thread. \
                 {ARTIFACTS_ARE_NOT_PERSISTED}"
            )),
        }
    }
}

/// The one place the in-memory lifetime is put into words.
///
/// **This is the known gap in `OMEGA-DELTA-0103`, stated rather than hidden.**
/// Artifacts are not written to disk, so a thread that was closed and reopened
/// still carries the truncation markers in its saved messages — those are part
/// of the message text — while the results they address are gone. The marker
/// then points at nothing.
///
/// Made legible rather than fixed, deliberately. The alternative is persisting
/// every complete tool result into the thread blob, which puts the whole
/// unbounded result back on disk for the life of the thread and grows without
/// limit — the size property `OMEGA-DELTA-0103` exists to hold. Any bounded
/// version of that (evict the oldest, cap the total) has this same sentence at
/// its edge anyway, just later and with more machinery in front of it. So the
/// requirement is the one that survives either choice: **a fetch that does not
/// resolve says why, and never reads as "that result never existed."**
pub const ARTIFACTS_ARE_NOT_PERSISTED: &str = "Tool result artifacts live in \
     memory for as long as this thread is open and are never written to disk, \
     so an address from before this thread was last reopened no longer \
     resolves. The result existed — it is not recoverable from here. Re-run \
     the tool call if you still need it.";

impl ToolResultArtifactRegistry {
    /// Bound `text` for the event, recording the complete result first when it
    /// is over budget.
    ///
    /// The guard is the terminal path's, for the same reason: a result under
    /// the budget gets no artifact, no marker, and no version number — a marker
    /// on every result is a marker a reader stops reading, and a version that
    /// nothing addresses is an entry that only costs memory.
    pub fn bound(&mut self, source: &str, text: &str) -> ToolResultPreview {
        let total_lines = line_count(text);
        let artifact = (text.len() > TOOL_RESULT_PREVIEW_BYTE_BUDGET).then(|| {
            self.stores
                .entry(Arc::from(source))
                .or_insert_with(|| ToolResultArtifactStore::new(source))
                .record(text)
        });
        preview_tool_result(
            text,
            text.len(),
            total_lines,
            TOOL_RESULT_PREVIEW_BYTE_BUDGET,
            artifact,
        )
    }

    /// The fetch path the marker's address points at.
    #[must_use]
    pub fn lookup(&self, address: &ToolResultArtifactId) -> ArtifactLookup<'_> {
        let Some(store) = self.stores.get(address.source()) else {
            return ArtifactLookup::Forgotten;
        };
        match store.get(address) {
            Some(artifact) => ArtifactLookup::Found(artifact),
            None => ArtifactLookup::NoSuchVersion {
                available: (1..=u32::try_from(store.version_count()).unwrap_or(u32::MAX)).collect(),
            },
        }
    }

    #[must_use]
    pub fn source_count(&self) -> usize {
        self.stores.len()
    }
}

/// Why an address could not even be read as one.
///
/// Separate from [`ArtifactLookup`] on purpose: a malformed address is the
/// caller's typo and a well-formed one that does not resolve is a fact about
/// this thread. Answering both with "not found" would send a caller that
/// dropped the `@v1` looking for a result that is sitting right there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactAddressError {
    NoVersion,
    EmptySource,
    UnreadableVersion { version: String },
}

impl ArtifactAddressError {
    #[must_use]
    pub fn sentence(&self, given: &str) -> String {
        let detail = match self {
            Self::NoVersion => {
                format!("`{given}` has no `{TOOL_ARTIFACT_VERSION_SEPARATOR}<n>` version suffix")
            }
            Self::EmptySource => format!("`{given}` names no source before its version"),
            Self::UnreadableVersion { version } => {
                format!("`{version}` in `{given}` is not a version number")
            }
        };
        format!(
            "{detail}. An artifact address is the one the truncation marker at \
             the end of a bounded tool result hands out, and it looks like \
             `tool:4:toolu_01abc{TOOL_ARTIFACT_VERSION_SEPARATOR}1`. Copy it \
             from the marker rather than composing one."
        )
    }
}

/// Read a `<source>@v<n>` address.
///
/// Split from the right, because a source is a tool call id and is allowed to
/// contain anything a provider puts in one.
pub fn parse_artifact_address(given: &str) -> Result<ToolResultArtifactId, ArtifactAddressError> {
    let given = given.trim();
    let Some((source, version)) = given.rsplit_once(TOOL_ARTIFACT_VERSION_SEPARATOR) else {
        return Err(ArtifactAddressError::NoVersion);
    };
    if source.is_empty() {
        return Err(ArtifactAddressError::EmptySource);
    }
    let version = version
        .parse::<u32>()
        .map_err(|_| ArtifactAddressError::UnreadableVersion {
            version: version.to_owned(),
        })?;
    Ok(ToolResultArtifactId::new(source, version))
}

/// Lines in `text`, counting a final line with no trailing newline.
///
/// The same definition `acp_thread` uses, which is why the totals handed to
/// `preview_tool_result` from here agree with the ones it computes for itself.
fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(lines: usize) -> String {
        (0..lines)
            .map(|index| {
                format!(
                    "{{\"id\":\"{index:064x}\",\"sig\":\"{}\"}}",
                    "ab".repeat(64)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_large_tool_result_leaves_a_bounded_event_and_a_reachable_artifact() {
        let mut registry = ToolResultArtifactRegistry::default();
        let full = blob(200);
        let source = tool_result_artifact_source("4:toolu_01abc");

        let preview = registry.bound(&source, &full);

        assert!(preview.is_truncated());
        assert!(preview.text.len() <= TOOL_RESULT_PREVIEW_BYTE_BUDGET);
        let address = preview
            .artifact
            .clone()
            .expect("a truncated preview names the artifact to fetch");
        assert_eq!(address.to_string(), "tool:4:toolu_01abc@v1");
        assert!(preview.text.contains(&format!("artifact {address}")));

        // The address the marker printed is the address the fetch path takes.
        // This is the half `OMEGA-DELTA-0103` left unbuilt for native tools.
        let printed = preview
            .text
            .rsplit_once("artifact ")
            .and_then(|(_, rest)| rest.split_once('.'))
            .map(|(address, _)| address.to_owned())
            .expect("the marker prints the address");
        let parsed = parse_artifact_address(&printed).expect("the printed address parses");
        assert_eq!(
            registry
                .lookup(&parsed)
                .artifact()
                .map(ToolResultArtifact::text),
            Some(full.as_str()),
            "the address the marker handed out does not resolve to the result \
             it named, so the marker is unspendable"
        );
    }

    #[test]
    fn a_small_tool_result_gets_no_artifact_no_marker_and_no_version() {
        let mut registry = ToolResultArtifactRegistry::default();
        let body = "publishing to relay.openagents.com... success.\n";

        let preview = registry.bound(&tool_result_artifact_source("1:x"), body);

        assert_eq!(preview.text, body);
        assert!(!preview.is_truncated());
        assert_eq!(preview.artifact, None);
        assert_eq!(
            registry.source_count(),
            0,
            "a result nothing will ever fetch was still recorded, so the \
             registry grows with every small tool call"
        );
    }

    #[test]
    fn two_calls_of_one_tool_stay_separately_addressable() {
        let mut registry = ToolResultArtifactRegistry::default();
        let source = tool_result_artifact_source("2:toolu_x");
        let first = blob(200);
        let second = blob(300);

        let first_address = registry.bound(&source, &first).artifact.expect("recorded");
        let second_address = registry.bound(&source, &second).artifact.expect("recorded");

        assert_ne!(first_address, second_address);
        assert_eq!(
            registry
                .lookup(&first_address)
                .artifact()
                .map(ToolResultArtifact::text),
            Some(first.as_str()),
            "the earlier result stopped resolving when a later one arrived, so \
             a reader quoting it is quoting output that no longer exists"
        );
        assert_eq!(
            registry
                .lookup(&second_address)
                .artifact()
                .map(ToolResultArtifact::text),
            Some(second.as_str())
        );
    }

    #[test]
    fn an_address_from_another_tool_call_does_not_resolve_here() {
        let mut registry = ToolResultArtifactRegistry::default();
        registry.bound(&tool_result_artifact_source("1:a"), &blob(200));

        let foreign = ToolResultArtifactId::new(tool_result_artifact_source("1:b"), 1);
        assert_eq!(registry.lookup(&foreign), ArtifactLookup::Forgotten);
    }

    #[test]
    fn a_forgotten_address_says_artifacts_do_not_survive_a_restart() {
        // The known gap, held to the standard the delta sets for it: when the
        // fetch cannot resolve, the reader is told why, and never that the
        // result never existed.
        let registry = ToolResultArtifactRegistry::default();
        let address = ToolResultArtifactId::new(tool_result_artifact_source("7:gone"), 1);

        let sentence = registry
            .lookup(&address)
            .sentence(&address)
            .expect("a fetch that does not resolve is a sentence, not a silence");

        assert!(sentence.contains(&address.to_string()));
        assert!(
            sentence.contains("never written to disk") && sentence.contains("reopened"),
            "the refusal does not name the lifetime that caused it, so a \
             reader concludes the result never existed: {sentence}"
        );
        assert!(
            !sentence.contains("check the ID") && !sentence.contains("does not exist"),
            "the refusal reads as the caller's mistake: {sentence}"
        );
    }

    #[test]
    fn a_wrong_version_names_the_versions_that_exist_instead() {
        let mut registry = ToolResultArtifactRegistry::default();
        let source = tool_result_artifact_source("3:c");
        registry.bound(&source, &blob(200));
        registry.bound(&source, &blob(300));

        let wrong = ToolResultArtifactId::new(source.clone(), 9);
        let lookup = registry.lookup(&wrong);
        assert_eq!(
            lookup,
            ArtifactLookup::NoSuchVersion {
                available: vec![1, 2]
            }
        );
        let sentence = lookup.sentence(&wrong).expect("a refusal");
        assert!(sentence.contains(&format!("{source}@v1")));
        assert!(sentence.contains(&format!("{source}@v2")));
        assert!(
            !sentence.contains("never written to disk"),
            "a version that is off by one is answered as a restart, sending \
             the reader after a result that is sitting right there: {sentence}"
        );
    }

    #[test]
    fn a_found_artifact_has_no_refusal_sentence() {
        let mut registry = ToolResultArtifactRegistry::default();
        let address = registry
            .bound(&tool_result_artifact_source("1:a"), &blob(200))
            .artifact
            .expect("recorded");
        assert_eq!(registry.lookup(&address).sentence(&address), None);
    }

    #[test]
    fn an_address_is_read_back_from_the_form_the_marker_prints() {
        let id = ToolResultArtifactId::new("tool:12:toolu_01@weird", 3);
        assert_eq!(
            parse_artifact_address(&id.to_string()),
            Ok(id),
            "an address is split from the left, so a tool call id containing \
             the separator is read as some other artifact"
        );
    }

    #[test]
    fn a_malformed_address_is_answered_as_a_typo_and_not_as_a_missing_result() {
        for (given, expected) in [
            ("tool:1:a", ArtifactAddressError::NoVersion),
            ("@v1", ArtifactAddressError::EmptySource),
            (
                "tool:1:a@vlatest",
                ArtifactAddressError::UnreadableVersion {
                    version: "latest".to_owned(),
                },
            ),
        ] {
            let error = parse_artifact_address(given).expect_err("malformed");
            assert_eq!(error, expected, "for `{given}`");
            let sentence = error.sentence(given);
            assert!(
                sentence.contains("truncation marker"),
                "the parse failure does not point at where a real address \
                 comes from: {sentence}"
            );
            assert!(
                !sentence.contains("never written to disk"),
                "a typo is answered as a restart: {sentence}"
            );
        }
    }

    #[test]
    fn whitespace_around_a_pasted_address_is_not_a_typo() {
        let id = ToolResultArtifactId::new("tool:1:a", 2);
        assert_eq!(parse_artifact_address("  tool:1:a@v2\n"), Ok(id));
    }

    #[test]
    fn the_line_count_here_agrees_with_the_one_the_law_uses() {
        // The totals in the marker come from this function; if it disagreed
        // with `acp_thread`'s, `preview_tool_result` would silently raise them
        // with its own `max`, and the marker's arithmetic would stop being the
        // arithmetic of the body it is attached to.
        for text in ["", "one", "one\n", "one\ntwo", "one\ntwo\n"] {
            let mut registry = ToolResultArtifactRegistry::default();
            let preview = registry.bound("tool:x", text);
            assert_eq!(
                preview.total_lines,
                line_count(text),
                "line count disagreed for {text:?}"
            );
        }
    }
}
