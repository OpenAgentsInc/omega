//! Operations Work over the runtime services this Omega process operates.
//!
//! Everything here is an observation of a service Omega is already running in
//! the current process. There is no fixture, no mock, and no remote authority:
//! if the observation is absent the row is absent, and if the observation is
//! ambiguous the row says so rather than guessing a healthy state.
//!
//! This module is deliberately pure. It has no `Window`, no `Context`, and no
//! primary-interface branch, so its behaviour is exercised on the same path the
//! application uses.

use std::collections::BTreeMap;

use language::{BinaryStatus, ServerHealth};
use omega_work_detail::{
    WorkBlock, WorkBlockFact, WorkBlockFactKind, WorkBlockFactState, WorkBlockKind,
};
use omega_work_index::{NativeRuntimeServiceRecord, NativeRuntimeServiceState, WorkIndexItem};
use sha2::{Digest as _, Sha256};

/// What this process has observed about one language service.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LanguageServiceObservation {
    /// The exact name the language registry uses.
    pub name: String,
    pub language: Option<String>,
    pub version: Option<String>,
    /// `None` means the process is not running yet, not that it is healthy.
    pub process_id: Option<u32>,
    pub scope: Option<String>,
    /// Titles of the work the service reports in flight.
    pub pending_work: Vec<String>,
    pub pending_diagnostics: bool,
    /// The last binary lifecycle status the registry broadcast, if any.
    pub binary: Option<BinaryStatus>,
    /// The last health the server reported, with its message, if any.
    pub health: Option<(ServerHealth, Option<String>)>,
}

/// The stable Work identity component for a language service.
///
/// Identity is the service name plus the scope it serves, because one window
/// can run the same service for two working folders and they are two operated
/// services. It deliberately excludes the process id, the server id, and the
/// version, so a restart or an upgrade keeps one Work identity.
///
/// A canonical reference admits only `[A-Za-z][A-Za-z0-9._:/-]*`, and both a
/// service name and a folder name are free text. Rather than silently
/// rewriting them into a different identity than the caller published, an
/// unrepresentable pair becomes an explicit opaque digest: stable across
/// restarts, distinct per pair, and honest that it is not the name.
pub fn language_service_ref(name: &str, scope: Option<&str>) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let scope = scope.map(str::trim).filter(|scope| !scope.is_empty());
    let readable = match scope {
        Some(scope) => format!("language:{name}:{scope}"),
        None => format!("language:{name}"),
    };
    if is_canonical_reference(&readable) {
        return Some(readable);
    }
    // The digest covers the exact untrimmed pair, so two names that would
    // collapse under any rewrite stay distinct.
    let mut digest = Sha256::new();
    digest.update(name.as_bytes());
    digest.update([0]);
    digest.update(scope.unwrap_or_default().as_bytes());
    Some(format!("language:opaque-{:x}", digest.finalize()))
}

fn is_canonical_reference(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '/' | '-')
        })
}

/// Reduce every observation of one language service to its exact operational
/// state.
///
/// The order is severity-first on purpose: a service that reported an error and
/// is also indexing is unavailable, not busy.
pub fn language_service_state(
    observation: &LanguageServiceObservation,
) -> NativeRuntimeServiceState {
    if let Some(BinaryStatus::Failed { error }) = &observation.binary {
        return NativeRuntimeServiceState::Unavailable {
            detail: error.clone(),
        };
    }
    if let Some((ServerHealth::Error, message)) = &observation.health {
        return NativeRuntimeServiceState::Unavailable {
            detail: message
                .clone()
                .unwrap_or_else(|| "the language service reported an error".into()),
        };
    }
    if matches!(
        observation.binary,
        Some(BinaryStatus::Stopped | BinaryStatus::Stopping)
    ) {
        return NativeRuntimeServiceState::Stopped;
    }
    if let Some((ServerHealth::Warning, message)) = &observation.health {
        return NativeRuntimeServiceState::Degraded {
            detail: message
                .clone()
                .unwrap_or_else(|| "the language service reported a warning".into()),
        };
    }
    if matches!(
        observation.binary,
        Some(BinaryStatus::CheckingForUpdate | BinaryStatus::Downloading | BinaryStatus::Starting)
    ) {
        return NativeRuntimeServiceState::Provisioning;
    }
    if observation.process_id.is_none() {
        // No process has been observed, so nothing proves this service serves.
        return NativeRuntimeServiceState::Provisioning;
    }
    let mut in_flight = observation.pending_work.clone();
    if observation.pending_diagnostics {
        in_flight.push("diagnostics".into());
    }
    if in_flight.is_empty() {
        NativeRuntimeServiceState::Serving
    } else {
        NativeRuntimeServiceState::Working {
            detail: in_flight.join(", "),
        }
    }
}

/// Decode a broadcast binary lifecycle status.
///
/// A failure that carries no message is refused: an unexplained failure is not
/// an exact observation, and this domain puts failures in the Inbox.
pub fn binary_status_from_proto(status: &proto::StatusUpdate, binary: i32) -> Option<BinaryStatus> {
    Some(match proto::ServerBinaryStatus::from_i32(binary)? {
        proto::ServerBinaryStatus::None => BinaryStatus::None,
        proto::ServerBinaryStatus::CheckingForUpdate => BinaryStatus::CheckingForUpdate,
        proto::ServerBinaryStatus::Downloading => BinaryStatus::Downloading,
        proto::ServerBinaryStatus::Starting => BinaryStatus::Starting,
        proto::ServerBinaryStatus::Stopping => BinaryStatus::Stopping,
        proto::ServerBinaryStatus::Stopped => BinaryStatus::Stopped,
        proto::ServerBinaryStatus::Failed => BinaryStatus::Failed {
            error: status.message.clone()?,
        },
    })
}

/// Decode a broadcast health status with its message.
pub fn health_from_proto(
    status: &proto::StatusUpdate,
    health: i32,
) -> Option<(ServerHealth, Option<String>)> {
    let health = match proto::ServerHealth::from_i32(health)? {
        proto::ServerHealth::Ok => ServerHealth::Ok,
        proto::ServerHealth::Warning => ServerHealth::Warning,
        proto::ServerHealth::Error => ServerHealth::Error,
    };
    Some((health, status.message.clone()))
}

/// Reduce one live language-server status, plus whatever binary lifecycle and
/// health this window has retained, into a single exact observation.
///
/// This is the mapping the panel uses. Keeping it here, taking the real
/// `LanguageServerStatus`, means the projection is exercised on the same values
/// the application reads rather than on a parallel test shape.
pub fn observation_from_language_server_status(
    status: &project::LanguageServerStatus,
    scope: Option<String>,
    binary: Option<BinaryStatus>,
    health: Option<(ServerHealth, Option<String>)>,
) -> LanguageServiceObservation {
    LanguageServiceObservation {
        name: status.name.0.to_string(),
        language: status.language_name.as_ref().map(ToString::to_string),
        version: status
            .server_readable_version
            .as_ref()
            .or(status.server_version.as_ref())
            .map(ToString::to_string),
        process_id: status.process_id,
        scope,
        pending_work: status
            .pending_work
            .values()
            .filter_map(|progress| progress.title.clone())
            .collect(),
        pending_diagnostics: status.has_pending_diagnostic_updates,
        binary,
        health,
    }
}

/// Project one observed language service as an Operations Work record.
///
/// Returns `None` when the service cannot carry a stable Work identity, so one
/// unnameable service loses its own row instead of the whole domain.
pub fn project_language_service(
    observation: &LanguageServiceObservation,
    observed_at: &str,
    revision: u64,
) -> Option<NativeRuntimeServiceRecord> {
    Some(NativeRuntimeServiceRecord {
        service_ref: language_service_ref(&observation.name, observation.scope.as_deref())?,
        display_name: observation.name.trim().to_string(),
        scope: observation.scope.clone(),
        state: language_service_state(observation),
        process_id: observation.process_id,
        version: observation.version.clone(),
        updated_at: observed_at.to_string(),
        observed_at: observed_at.to_string(),
        revision,
    })
}

/// Domain-specific Blocks for one operated runtime service.
///
/// The shared Work detail shell renders these; nothing here is a second
/// renderer, a second identity, or a mutation path.
pub fn project_runtime_service_work(
    item: &WorkIndexItem,
    observation: Option<&LanguageServiceObservation>,
) -> Result<Vec<WorkBlock>, omega_work_detail::WorkDetailError> {
    let source_ref = item.summary.source_authority.source_ref.clone();
    let profile = item.profile();
    let mut lifecycle = vec![fact(
        format!("fact:operations:state:{}", item.work_ref()),
        WorkBlockFactKind::Lifecycle,
        match item.summary.state {
            omega_effectd::all_work_contract::WorkState::Failed => WorkBlockFactState::Failed,
            omega_effectd::all_work_contract::WorkState::Waiting => WorkBlockFactState::Provisional,
            omega_effectd::all_work_contract::WorkState::Completed => WorkBlockFactState::Completed,
            _ => WorkBlockFactState::Active,
        },
        "Observed state",
        profile.state_label(&item.summary.state),
        [item.source_ref()],
    )];
    if let Some(detail) = item.summary.description.as_ref() {
        lifecycle.push(fact(
            format!("fact:operations:detail:{}", item.work_ref()),
            WorkBlockFactKind::Source,
            WorkBlockFactState::Observed,
            "Exact observation",
            detail.0.clone(),
            [item.source_ref()],
        ));
    }
    let mut activity = Vec::new();
    match observation {
        None => {
            lifecycle.push(fact(
                format!("fact:operations:unobserved:{}", item.work_ref()),
                WorkBlockFactKind::MissingInput,
                WorkBlockFactState::Missing,
                "Live observation",
                "This service is not observable from the current window.",
                [item.source_ref()],
            ));
        }
        Some(observation) => {
            lifecycle.push(fact(
                format!("fact:operations:process:{}", item.work_ref()),
                WorkBlockFactKind::Lifecycle,
                observation
                    .process_id
                    .map_or(WorkBlockFactState::Missing, |_| {
                        WorkBlockFactState::Observed
                    }),
                "Process",
                observation
                    .process_id
                    .map_or_else(|| "not running".to_string(), |pid| pid.to_string()),
                [item.source_ref()],
            ));
            if let Some(version) = &observation.version {
                lifecycle.push(fact(
                    format!("fact:operations:version:{}", item.work_ref()),
                    WorkBlockFactKind::Source,
                    WorkBlockFactState::Observed,
                    "Version",
                    version.clone(),
                    [item.source_ref()],
                ));
            }
            if let Some((health, message)) = &observation.health {
                activity.push(fact(
                    format!("fact:operations:health:{}", item.work_ref()),
                    WorkBlockFactKind::Lifecycle,
                    match health {
                        ServerHealth::Ok => WorkBlockFactState::Observed,
                        ServerHealth::Warning => WorkBlockFactState::Provisional,
                        ServerHealth::Error => WorkBlockFactState::Failed,
                    },
                    "Reported health",
                    message.clone().unwrap_or_else(|| format!("{health:?}")),
                    [item.source_ref()],
                ));
            }
            for (index, title) in observation.pending_work.iter().enumerate() {
                activity.push(fact(
                    format!("fact:operations:work:{}:{index}", item.work_ref()),
                    WorkBlockFactKind::Usage,
                    WorkBlockFactState::Active,
                    "In flight",
                    title.clone(),
                    [item.source_ref()],
                ));
            }
            if observation.pending_diagnostics {
                activity.push(fact(
                    format!("fact:operations:diagnostics:{}", item.work_ref()),
                    WorkBlockFactKind::Usage,
                    WorkBlockFactState::Active,
                    "In flight",
                    "diagnostics",
                    [item.source_ref()],
                ));
            }
        }
    }
    let blocks = vec![
        block(
            "block:omega:operations-lifecycle",
            item,
            WorkBlockKind::Lifecycle,
            "Service lifecycle",
            &source_ref,
            lifecycle,
        )?,
        block(
            "block:omega:operations-activity",
            item,
            WorkBlockKind::Metric,
            "Service activity",
            &source_ref,
            activity,
        )?,
    ];
    Ok(blocks)
}

fn block(
    prefix: &str,
    item: &WorkIndexItem,
    kind: WorkBlockKind,
    title: &str,
    source_ref: &omega_effectd::all_work_contract::SourceRef,
    facts: Vec<WorkBlockFact>,
) -> Result<WorkBlock, omega_work_detail::WorkDetailError> {
    Ok(WorkBlock {
        block_ref: omega_effectd::all_work_contract::SourceRef::try_from(format!(
            "{prefix}:{}",
            item.source_ref()
        ))?,
        work_ref: item.summary.work_ref.clone(),
        kind,
        title: omega_effectd::all_work_contract::ShortText::try_from(title.to_string())?,
        source_ref: source_ref.clone(),
        available: !facts.is_empty(),
        facts,
    })
}

fn fact(
    fact_ref: String,
    kind: WorkBlockFactKind,
    state: WorkBlockFactState,
    label: &str,
    value: impl Into<String>,
    source_refs: impl IntoIterator<Item = impl Into<String>>,
) -> WorkBlockFact {
    WorkBlockFact {
        fact_ref,
        kind,
        state,
        label: label.to_string(),
        value: value.into(),
        source_refs: source_refs.into_iter().map(Into::into).collect(),
    }
}

/// Reduce a window's language-service observations to a stable, ordered set.
///
/// Ordering is by service reference so two refreshes of the same fleet produce
/// the same rows in the same order.
pub fn language_service_records(
    observations: &BTreeMap<String, LanguageServiceObservation>,
    observed_at: &str,
    revision: u64,
) -> Vec<NativeRuntimeServiceRecord> {
    observations
        .values()
        .filter_map(|observation| project_language_service(observation, observed_at, revision))
        .collect()
}

#[cfg(test)]
mod tests {
    use omega_work_index::adapt_runtime_service;

    use super::*;

    fn observation(name: &str) -> LanguageServiceObservation {
        LanguageServiceObservation {
            name: name.into(),
            language: Some("Rust".into()),
            version: Some("1.2.3".into()),
            process_id: Some(4321),
            scope: Some("omega".into()),
            pending_work: Vec::new(),
            pending_diagnostics: false,
            binary: None,
            health: None,
        }
    }

    #[test]
    fn severity_wins_over_activity_when_a_service_is_both_busy_and_broken() {
        let mut broken = observation("rust-analyzer");
        broken.pending_work = vec!["indexing".into()];
        broken.binary = Some(BinaryStatus::Failed {
            error: "could not start the server".into(),
        });
        assert_eq!(
            language_service_state(&broken),
            NativeRuntimeServiceState::Unavailable {
                detail: "could not start the server".into()
            }
        );

        let mut erroring = observation("rust-analyzer");
        erroring.pending_diagnostics = true;
        erroring.health = Some((ServerHealth::Error, Some("workspace load failed".into())));
        assert_eq!(
            language_service_state(&erroring),
            NativeRuntimeServiceState::Unavailable {
                detail: "workspace load failed".into()
            }
        );
    }

    #[test]
    fn a_warning_is_degraded_and_a_stop_is_stopped() {
        let mut warned = observation("rust-analyzer");
        warned.health = Some((ServerHealth::Warning, Some("proc macro disabled".into())));
        assert_eq!(
            language_service_state(&warned),
            NativeRuntimeServiceState::Degraded {
                detail: "proc macro disabled".into()
            }
        );

        let mut stopped = observation("rust-analyzer");
        stopped.binary = Some(BinaryStatus::Stopped);
        assert_eq!(
            language_service_state(&stopped),
            NativeRuntimeServiceState::Stopped
        );
    }

    #[test]
    fn a_service_without_an_observed_process_is_not_reported_as_serving() {
        let mut starting = observation("rust-analyzer");
        starting.process_id = None;
        assert_eq!(
            language_service_state(&starting),
            NativeRuntimeServiceState::Provisioning
        );
        assert_eq!(
            language_service_state(&observation("rust-analyzer")),
            NativeRuntimeServiceState::Serving
        );
    }

    #[test]
    fn in_flight_work_is_reported_exactly() {
        let mut busy = observation("rust-analyzer");
        busy.pending_work = vec!["indexing".into(), "building proc macros".into()];
        busy.pending_diagnostics = true;
        assert_eq!(
            language_service_state(&busy),
            NativeRuntimeServiceState::Working {
                detail: "indexing, building proc macros, diagnostics".into()
            }
        );
    }

    #[test]
    fn an_unrepresentable_service_name_becomes_a_stable_opaque_identity() {
        assert_eq!(
            language_service_ref("rust-analyzer", None).as_deref(),
            Some("language:rust-analyzer")
        );
        assert_eq!(
            language_service_ref("rust-analyzer", Some("omega")).as_deref(),
            Some("language:rust-analyzer:omega")
        );
        assert_ne!(
            language_service_ref("rust-analyzer", Some("omega")),
            language_service_ref("rust-analyzer", Some("psionic")),
            "the same service in two working folders is two operated services"
        );
        assert_eq!(language_service_ref("   ", None), None);
        let first = language_service_ref("My Cool LSP", None).expect("an opaque identity");
        let second = language_service_ref("My Cool LSP", None).expect("an opaque identity");
        assert_eq!(first, second, "identity must survive a restart");
        assert_ne!(
            first,
            language_service_ref("My-Cool-LSP", None).expect("an opaque identity"),
            "two distinct names must not collapse onto one Work identity"
        );
        assert!(
            !first.contains("My Cool LSP") && first.starts_with("language:opaque-"),
            "an opaque identity must not pretend to be the name: {first}"
        );
        // The projected identity must be admissible by the index.
        let record =
            project_language_service(&observation("My Cool LSP"), "2026-08-03T00:00:00.000Z", 1)
                .expect("a projected record");
        let item = adapt_runtime_service(record).expect("an admitted Operations row");
        assert_eq!(item.summary.title.0, "My Cool LSP · omega");
    }

    #[test]
    fn one_unnameable_service_loses_its_row_and_not_the_domain() {
        let observations = BTreeMap::from([
            ("a".to_string(), observation("rust-analyzer")),
            ("b".to_string(), {
                let mut empty = observation("");
                empty.name = "   ".into();
                empty
            }),
            ("c".to_string(), observation("tsgo")),
        ]);
        let records = language_service_records(&observations, "2026-08-03T00:00:00.000Z", 1);
        assert_eq!(records.len(), 2);
        assert!(
            records
                .into_iter()
                .all(|record| adapt_runtime_service(record).is_ok())
        );
    }

    #[test]
    fn domain_blocks_carry_the_exact_observation_and_name_a_missing_one() {
        let mut broken = observation("rust-analyzer");
        broken.health = Some((ServerHealth::Error, Some("workspace load failed".into())));
        let record = project_language_service(&broken, "2026-08-03T00:00:00.000Z", 1)
            .expect("a projected record");
        let item = adapt_runtime_service(record).expect("an admitted Operations row");

        let blocks = project_runtime_service_work(&item, Some(&broken)).expect("domain Blocks");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, WorkBlockKind::Lifecycle);
        assert_eq!(blocks[1].kind, WorkBlockKind::Metric);
        let values = blocks
            .iter()
            .flat_map(|block| &block.facts)
            .map(|fact| fact.value.clone())
            .collect::<Vec<_>>();
        assert!(
            values.iter().any(|value| value == "Unavailable"),
            "the domain's own vocabulary must name the state: {values:?}"
        );
        assert!(
            values
                .iter()
                .any(|value| value.contains("workspace load failed")),
            "the exact observation must reach the Blocks: {values:?}"
        );

        let unobserved = project_runtime_service_work(&item, None).expect("domain Blocks");
        assert!(
            unobserved
                .iter()
                .flat_map(|block| &block.facts)
                .any(|fact| fact.kind == WorkBlockFactKind::MissingInput),
            "an unobservable service must say so rather than look healthy"
        );
        assert!(
            !unobserved[1].available,
            "an empty activity Block must not claim to be available"
        );
    }

    fn language_server_status(name: &str) -> project::LanguageServerStatus {
        project::LanguageServerStatus {
            name: lsp::LanguageServerName(name.to_string().into()),
            language_name: Some(language::LanguageName::new("Rust")),
            server_version: Some("raw 1.2.3".into()),
            server_readable_version: Some("1.2.3".into()),
            pending_work: std::collections::BTreeMap::new(),
            has_pending_diagnostic_updates: false,
            progress_tokens: Default::default(),
            worktree: None,
            binary: None,
            configuration: None,
            workspace_folders: Default::default(),
            process_id: Some(4321),
        }
    }

    #[test]
    fn an_unexplained_binary_failure_is_refused_rather_than_shown_as_an_unexplained_inbox_row() {
        let explained = proto::StatusUpdate {
            message: Some("could not start the server".into()),
            status: None,
        };
        assert_eq!(
            binary_status_from_proto(&explained, proto::ServerBinaryStatus::Failed as i32),
            Some(BinaryStatus::Failed {
                error: "could not start the server".into()
            })
        );
        let unexplained = proto::StatusUpdate {
            message: None,
            status: None,
        };
        assert_eq!(
            binary_status_from_proto(&unexplained, proto::ServerBinaryStatus::Failed as i32),
            None,
            "an unexplained failure is not an exact observation"
        );
        assert_eq!(
            binary_status_from_proto(&unexplained, proto::ServerBinaryStatus::Stopped as i32),
            Some(BinaryStatus::Stopped),
            "a lifecycle status that needs no message is still exact"
        );
        assert_eq!(
            binary_status_from_proto(&unexplained, 9999),
            None,
            "an unknown status must not be decoded into a known one"
        );
        assert_eq!(
            health_from_proto(&explained, proto::ServerHealth::Warning as i32),
            Some((
                ServerHealth::Warning,
                Some("could not start the server".to_string())
            ))
        );
        assert_eq!(health_from_proto(&explained, 9999), None);
    }

    #[test]
    fn a_live_language_server_status_maps_to_one_exact_observation() {
        let mut status = language_server_status("rust-analyzer");
        status.pending_work.insert(
            project::ProgressToken::Number(1),
            project::LanguageServerProgress {
                is_disk_based_diagnostics_progress: false,
                is_cancellable: false,
                title: Some("indexing".into()),
                message: None,
                percentage: None,
                last_update_at: std::time::Instant::now(),
            },
        );
        // Progress with no title carries no fact and must not become one.
        status.pending_work.insert(
            project::ProgressToken::Number(2),
            project::LanguageServerProgress {
                is_disk_based_diagnostics_progress: false,
                is_cancellable: false,
                title: None,
                message: Some("still going".into()),
                percentage: Some(40),
                last_update_at: std::time::Instant::now(),
            },
        );
        let observed = observation_from_language_server_status(
            &status,
            Some("omega".into()),
            Some(BinaryStatus::Starting),
            Some((ServerHealth::Warning, Some("proc macro disabled".into()))),
        );
        assert_eq!(observed.name, "rust-analyzer");
        assert_eq!(observed.pending_work, vec!["indexing".to_string()]);
        assert_eq!(observed.process_id, Some(4321));
        assert_eq!(
            observed.version.as_deref(),
            Some("1.2.3"),
            "the readable version must win over the raw one"
        );
        assert_eq!(observed.scope.as_deref(), Some("omega"));
        // Retained health outranks a binary that is still starting.
        assert_eq!(
            language_service_state(&observed),
            NativeRuntimeServiceState::Degraded {
                detail: "proc macro disabled".into()
            }
        );

        let unobserved = observation_from_language_server_status(
            &language_server_status("rust-analyzer"),
            None,
            None,
            None,
        );
        assert_eq!(unobserved.binary, None);
        assert_eq!(unobserved.health, None);
        assert_eq!(
            language_service_state(&unobserved),
            NativeRuntimeServiceState::Serving
        );
    }

    #[test]
    fn the_shared_shell_gives_an_operations_row_its_domain_blocks_without_an_entity_branch() {
        let record =
            project_language_service(&observation("rust-analyzer"), "2026-08-03T00:00:00.000Z", 1)
                .expect("a projected record");
        let item = adapt_runtime_service(record).expect("an admitted Operations row");
        let kinds = omega_work_detail::default_blocks(&item)
            .expect("default Blocks")
            .into_iter()
            .map(|block| block.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![WorkBlockKind::Metric, WorkBlockKind::Log],
            "Operations Blocks must come from the domain profile"
        );
    }
}
