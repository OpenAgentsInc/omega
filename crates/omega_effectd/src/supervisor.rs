//! Supervise packaged `omega-effectd` over newline-framed JSON stdio.
//!
//! Durable Full Auto run truth lives on disk under the injected data root.
//! This supervisor owns process life, health, restart, and generation fencing.
//! It must never become a second durable run authority (GPUI must not rewrite
//! runs after restart).

use std::ffi::OsStr;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use futures::io::{AsyncBufReadExt as _, BufReader};
use futures::{AsyncWriteExt as _, StreamExt as _};
use omega_forensics::{
    ForensicDispositionCommand, ForensicPriorWorkQuery, ForensicPriorWorkQueryResult,
    ForensicPriorWorkRecord, ForensicPriorWorkSubmission, ForensicRelationCommand,
};
use serde::Deserialize;
use serde_json::{Value, json};
use smol::process::ChildStdin;
use util::ResultExt as _;
use util::process::Child;
use util::redact::redact_command;

use crate::all_work::generated::{
    ContractValidate, OrganizationMembershipReadRequest, OrganizationMembershipReadResult,
    PlanningGraphReadRequest, PlanningGraphReadResult,
    ProtocolCapability as AllWorkProtocolCapability,
    ProtocolInitializeRequest as AllWorkProtocolInitializeRequest,
    ProtocolInitializeResult as AllWorkProtocolInitializeResult,
    ProtocolVersion as AllWorkProtocolVersion, RepositoryClaimExecuteRequest,
    RepositoryClaimExecuteResult, RepositoryClaimReadRequest, RepositoryClaimReadResult,
    SignedWorkroomCommitRequest, SignedWorkroomDeliveryRequest, SignedWorkroomDeliveryResult,
    SignedWorkroomEnqueueRequest, SignedWorkroomEnqueueResult, SignedWorkroomPrepareRequest,
    SignedWorkroomPrepareResult, SignedWorkroomPublishRequest, SignedWorkroomReadRequest,
    SignedWorkroomReadResult, StrictBugCandidateExecuteRequest, StrictBugCandidateExecuteResult,
    StrictBugCandidateReadRequest, StrictBugCandidateReadResult, WorkCommandExecuteRequest,
    WorkCommandExecuteResult, WorkCutoverExecuteRequest, WorkCutoverExecuteResult,
    WorkCutoverReadRequest, WorkCutoverReadResult, WorkIndexReadRequest, WorkIndexReadResult,
    WorkSnapshot, WorkSnapshotReadRequest, WorkSnapshotReadResult,
};
use crate::protocol::{
    HealthResult, HostMethod, HostRequestFrame, HostResponseError, HostResponseErrorCode,
    HostResponseFrame, InitializeResult, PROTOCOL_SCHEMA, ProtocolErrorCode, ResponseFrame,
    RunSnapshot, request_frame,
};

pub use crate::protocol::MAX_FRAME_BYTES;
const MAX_ALL_WORK_GRAPH_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(2);
const AGENT_COMPUTER_TURN_TIMEOUT: Duration = Duration::from_secs(180);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const DEFAULT_HOST_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HOST_ERROR_MESSAGE_BYTES: usize = 1024;

fn validate_work_execution_projections(snapshot: &WorkSnapshot) -> Result<()> {
    let sessions = snapshot.session_projections.as_deref().unwrap_or_default();
    for (index, session) in sessions.iter().enumerate() {
        if session.generation.0 == 0
            || sessions[..index]
                .iter()
                .any(|previous| previous.session_ref == session.session_ref)
            || !snapshot.session_refs.contains(&session.session_ref)
            || !snapshot.thread_refs.contains(&session.thread_ref)
            || !snapshot
                .agent_session_refs
                .contains(&session.agent_session_ref)
            || !snapshot.run_refs.contains(&session.run_ref)
        {
            bail!("Work snapshot contains an inconsistent Session projection");
        }
    }
    let activities = snapshot
        .agent_activity_projections
        .as_deref()
        .unwrap_or_default();
    for (index, activity) in activities.iter().enumerate() {
        if activity.generation.0 == 0
            || activities[..index]
                .iter()
                .any(|previous| previous.activity_ref == activity.activity_ref)
            || !snapshot
                .agent_activity_refs
                .contains(&activity.activity_ref)
            || !snapshot.session_refs.contains(&activity.session_ref)
            || !snapshot.run_refs.contains(&activity.run_ref)
            || !sessions.iter().any(|session| {
                session.session_ref == activity.session_ref
                    && session.run_ref == activity.run_ref
                    && session.generation == activity.generation
            })
        {
            bail!("Work snapshot contains an inconsistent Agent Activity projection");
        }
    }
    Ok(())
}

pub type OmegaEffectdHostFuture =
    Pin<Box<dyn Future<Output = std::result::Result<Value, HostResponseError>> + 'static>>;
pub type OmegaEffectdHostHandler = Rc<dyn Fn(HostRequestFrame) -> OmegaEffectdHostFuture + 'static>;

#[derive(Debug, Clone)]
pub struct OmegaEffectdCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OmegaEffectdSupervisorOptions {
    pub data_root: PathBuf,
    pub command: OmegaEffectdCommand,
    /// Initial generation. Each successful `restart` increments by one.
    pub initial_generation: u64,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttentionDecision {
    pub notify: bool,
    pub dedup_key: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
    #[error("stale generation")]
    StaleGeneration,
    #[error("protocol error ({code:?}): {message}")]
    Protocol {
        code: ProtocolErrorCode,
        message: String,
    },
    /// The running omega-effectd component answered `initialize` without an
    /// All Work negotiation at all. Every All Work resource is absent from
    /// that build, so a caller must not read this as a transient failure.
    ///
    /// omega#223: a packaged component older than the All Work boundary
    /// otherwise reports one generic `unknown_method` per call site, which
    /// reads as a bug in the calling feature rather than as a component that
    /// predates the whole surface.
    #[error(
        "the running omega-effectd component does not implement the All Work \
         boundary, so {method} is unavailable in this build"
    )]
    AllWorkBoundaryAbsent { method: &'static str },
    /// The component negotiated All Work but withheld this capability.
    #[error(
        "the running omega-effectd component did not negotiate {capability}, \
         so {method} is unavailable in this build"
    )]
    AllWorkCapabilityWithheld {
        method: &'static str,
        capability: String,
    },
}

/// The exact wire name of a negotiated All Work capability.
///
/// Derived from the generated contract's own serialization so a regenerated
/// capability set cannot drift away from the diagnosis text.
fn all_work_capability_label(capability: &AllWorkProtocolCapability) -> String {
    serde_json::to_value(capability)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{capability:?}"))
}

pub struct OmegaEffectdSupervisor {
    options: OmegaEffectdSupervisorOptions,
    generation: AtomicU64,
    next_request_id: AtomicU64,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<smol::process::ChildStdout>>,
    host_handler: Option<OmegaEffectdHostHandler>,
    host_request_timeout: Duration,
    /// What the *running* child negotiated, not what this build asked for.
    /// `None` while stopped, and also when the child answered `initialize`
    /// with no All Work block at all.
    negotiated_all_work: Option<AllWorkProtocolInitializeResult>,
}

impl OmegaEffectdSupervisor {
    pub fn new(options: OmegaEffectdSupervisorOptions) -> Self {
        let generation = options.initial_generation.max(1);
        Self {
            options,
            generation: AtomicU64::new(generation),
            next_request_id: AtomicU64::new(1),
            child: None,
            stdin: None,
            stdout: None,
            host_handler: None,
            host_request_timeout: DEFAULT_HOST_REQUEST_TIMEOUT,
            negotiated_all_work: None,
        }
    }

    pub fn set_host_handler(&mut self, handler: OmegaEffectdHostHandler) {
        self.host_handler = Some(handler);
    }

    pub fn set_host_request_timeout(&mut self, timeout: Duration) {
        self.host_request_timeout = timeout;
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn data_root(&self) -> &Path {
        &self.options.data_root
    }

    pub async fn start(&mut self) -> Result<InitializeResult> {
        if self.child.is_some() {
            bail!("omega-effectd is already running");
        }
        self.spawn_child().await?;
        let generation = self.generation();
        let all_work = AllWorkProtocolInitializeRequest {
            supported_versions: vec![
                AllWorkProtocolVersion::OmegaEffectdV2,
                AllWorkProtocolVersion::OmegaEffectdV1,
            ],
            requested_capabilities: vec![
                AllWorkProtocolCapability::WorkIndexRead,
                AllWorkProtocolCapability::WorkIndexSubscribe,
                AllWorkProtocolCapability::WorkSnapshotRead,
                AllWorkProtocolCapability::PlanningGraphRead,
                AllWorkProtocolCapability::RepositoryClaimRead,
                AllWorkProtocolCapability::RepositoryClaimExecute,
                AllWorkProtocolCapability::WorkroomActivityRead,
                AllWorkProtocolCapability::WorkroomActivityPrepare,
                AllWorkProtocolCapability::WorkroomActivityCommit,
                AllWorkProtocolCapability::WorkroomActivityEnqueue,
                AllWorkProtocolCapability::WorkroomActivityDeliver,
                AllWorkProtocolCapability::WorkroomActivityPublish,
                AllWorkProtocolCapability::WorkCommandExecute,
                AllWorkProtocolCapability::WorkCutoverRead,
                AllWorkProtocolCapability::WorkCutoverExecute,
                AllWorkProtocolCapability::OrganizationMembershipRead,
                AllWorkProtocolCapability::StrictBugCandidateRead,
                AllWorkProtocolCapability::StrictBugCandidateExecute,
            ],
        };
        all_work
            .validate()
            .context("validate All Work negotiation")?;
        let result = self
            .request(
                "initialize",
                Some(json!({ "generation": generation, "allWork": all_work })),
                generation,
            )
            .await?;
        let result: InitializeResult =
            serde_json::from_value(result).context("decode initialize result")?;
        self.negotiated_all_work = result.all_work.clone();
        if self.negotiated_all_work.is_none() {
            log::warn!(
                "omega-effectd answered initialize without an All Work negotiation; \
                 every All Work resource is absent from this component build"
            );
        }
        Ok(result)
    }

    pub async fn ensure_started(&mut self) -> Result<()> {
        if self.child.is_none() {
            self.start().await?;
        }
        Ok(())
    }

    /// The All Work capabilities the *running* component granted, or `None`
    /// when it negotiated no All Work boundary at all.
    pub fn negotiated_all_work_capabilities(&self) -> Option<&[AllWorkProtocolCapability]> {
        self.negotiated_all_work
            .as_ref()
            .map(|negotiated| negotiated.capabilities.as_slice())
    }

    /// Refuse an All Work request the running component cannot serve, before
    /// the request reaches the wire.
    fn require_all_work_capability(
        &self,
        method: &'static str,
        capability: AllWorkProtocolCapability,
    ) -> Result<(), SupervisorError> {
        let Some(negotiated) = self.negotiated_all_work.as_ref() else {
            return Err(SupervisorError::AllWorkBoundaryAbsent { method });
        };
        if negotiated.capabilities.contains(&capability) {
            return Ok(());
        }
        Err(SupervisorError::AllWorkCapabilityWithheld {
            method,
            capability: all_work_capability_label(&capability),
        })
    }

    pub async fn health(&mut self) -> Result<HealthResult, SupervisorError> {
        let result = self.request("health", None, self.generation()).await?;
        Ok(serde_json::from_value(result).context("decode health result")?)
    }

    pub async fn read_work_index(
        &mut self,
        params: WorkIndexReadRequest,
    ) -> Result<WorkIndexReadResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "work.index.read",
            AllWorkProtocolCapability::WorkIndexRead,
        )?;
        let result = self
            .request(
                "work.index.read",
                Some(serde_json::to_value(params).context("encode Work Index request")?),
                self.generation(),
            )
            .await?;
        let result: WorkIndexReadResult =
            serde_json::from_value(result).context("decode Work Index result")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn read_work_snapshot(
        &mut self,
        params: WorkSnapshotReadRequest,
    ) -> Result<WorkSnapshotReadResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "work.snapshot.read",
            AllWorkProtocolCapability::WorkSnapshotRead,
        )?;
        let result = self
            .request(
                "work.snapshot.read",
                Some(serde_json::to_value(params).context("encode Work snapshot request")?),
                self.generation(),
            )
            .await?;
        let result: WorkSnapshotReadResult =
            serde_json::from_value(result).context("decode Work snapshot result")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        validate_work_execution_projections(&result.snapshot).map_err(SupervisorError::Anyhow)?;
        Ok(result)
    }

    pub async fn read_planning_graph(
        &mut self,
        params: PlanningGraphReadRequest,
    ) -> Result<PlanningGraphReadResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "planning.graph.read",
            AllWorkProtocolCapability::PlanningGraphRead,
        )?;
        let result = self
            .request(
                "planning.graph.read",
                Some(serde_json::to_value(params).context("encode planning graph request")?),
                self.generation(),
            )
            .await?;
        let result: PlanningGraphReadResult =
            serde_json::from_value(result).context("decode planning graph result")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn read_repository_claims(
        &mut self,
        params: RepositoryClaimReadRequest,
    ) -> Result<RepositoryClaimReadResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "repository.claim.read",
            AllWorkProtocolCapability::RepositoryClaimRead,
        )?;
        let result = self
            .request(
                "repository.claim.read",
                Some(serde_json::to_value(params).context("encode claim-ledger read")?),
                self.generation(),
            )
            .await?;
        let result: RepositoryClaimReadResult =
            serde_json::from_value(result).context("decode claim-ledger read")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn execute_repository_claim(
        &mut self,
        params: RepositoryClaimExecuteRequest,
    ) -> Result<RepositoryClaimExecuteResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "repository.claim.execute",
            AllWorkProtocolCapability::RepositoryClaimExecute,
        )?;
        let result = self
            .request(
                "repository.claim.execute",
                Some(serde_json::to_value(params).context("encode claim-ledger command")?),
                self.generation(),
            )
            .await?;
        let result: RepositoryClaimExecuteResult =
            serde_json::from_value(result).context("decode claim-ledger command")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn read_signed_workroom(
        &mut self,
        params: SignedWorkroomReadRequest,
    ) -> Result<SignedWorkroomReadResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "workroom.activity.read",
            AllWorkProtocolCapability::WorkroomActivityRead,
        )?;
        let result = self
            .request(
                "workroom.activity.read",
                Some(serde_json::to_value(params).context("encode signed Workroom read")?),
                self.generation(),
            )
            .await?;
        let result: SignedWorkroomReadResult =
            serde_json::from_value(result).context("decode signed Workroom read")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn enqueue_signed_workroom(
        &mut self,
        params: SignedWorkroomEnqueueRequest,
    ) -> Result<SignedWorkroomEnqueueResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "workroom.activity.enqueue",
            AllWorkProtocolCapability::WorkroomActivityEnqueue,
        )?;
        let result = self
            .request(
                "workroom.activity.enqueue",
                Some(serde_json::to_value(params).context("encode signed Workroom enqueue")?),
                self.generation(),
            )
            .await?;
        let result: SignedWorkroomEnqueueResult =
            serde_json::from_value(result).context("decode signed Workroom enqueue")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn prepare_signed_workroom(
        &mut self,
        params: SignedWorkroomPrepareRequest,
    ) -> Result<SignedWorkroomPrepareResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "workroom.activity.prepare",
            AllWorkProtocolCapability::WorkroomActivityPrepare,
        )?;
        let result = self
            .request(
                "workroom.activity.prepare",
                Some(serde_json::to_value(params).context("encode signed Workroom preparation")?),
                self.generation(),
            )
            .await?;
        let result: SignedWorkroomPrepareResult =
            serde_json::from_value(result).context("decode signed Workroom preparation")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn commit_signed_workroom(
        &mut self,
        params: SignedWorkroomCommitRequest,
    ) -> Result<SignedWorkroomEnqueueResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "workroom.activity.commit",
            AllWorkProtocolCapability::WorkroomActivityCommit,
        )?;
        let result = self
            .request(
                "workroom.activity.commit",
                Some(serde_json::to_value(params).context("encode signed Workroom commit")?),
                self.generation(),
            )
            .await?;
        let result: SignedWorkroomEnqueueResult =
            serde_json::from_value(result).context("decode signed Workroom commit")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn deliver_signed_workroom(
        &mut self,
        params: SignedWorkroomDeliveryRequest,
    ) -> Result<SignedWorkroomDeliveryResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "workroom.activity.deliver",
            AllWorkProtocolCapability::WorkroomActivityDeliver,
        )?;
        let result = self
            .request(
                "workroom.activity.deliver",
                Some(serde_json::to_value(params).context("encode signed Workroom delivery")?),
                self.generation(),
            )
            .await?;
        let result: SignedWorkroomDeliveryResult =
            serde_json::from_value(result).context("decode signed Workroom delivery")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn publish_signed_workroom(
        &mut self,
        params: SignedWorkroomPublishRequest,
    ) -> Result<SignedWorkroomDeliveryResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "workroom.activity.publish",
            AllWorkProtocolCapability::WorkroomActivityPublish,
        )?;
        let result = self
            .request(
                "workroom.activity.publish",
                Some(serde_json::to_value(params).context("encode signed Workroom publish")?),
                self.generation(),
            )
            .await?;
        let result: SignedWorkroomDeliveryResult =
            serde_json::from_value(result).context("decode signed Workroom publish")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn execute_work_command(
        &mut self,
        params: WorkCommandExecuteRequest,
    ) -> Result<WorkCommandExecuteResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "work.command.execute",
            AllWorkProtocolCapability::WorkCommandExecute,
        )?;
        let result = self
            .request(
                "work.command.execute",
                Some(serde_json::to_value(params).context("encode Work command")?),
                self.generation(),
            )
            .await?;
        let result: WorkCommandExecuteResult =
            serde_json::from_value(result).context("decode Work command result")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        validate_work_execution_projections(&result.snapshot).map_err(SupervisorError::Anyhow)?;
        if result.receipt.github_write_count.0 != 0 {
            return Err(SupervisorError::Anyhow(anyhow!(
                "Work command receipt reported a GitHub write"
            )));
        }
        Ok(result)
    }

    pub async fn read_work_cutover(
        &mut self,
        params: WorkCutoverReadRequest,
    ) -> Result<WorkCutoverReadResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "work.cutover.read",
            AllWorkProtocolCapability::WorkCutoverRead,
        )?;
        let result = self
            .request(
                "work.cutover.read",
                Some(serde_json::to_value(params).context("encode Work cutover read")?),
                self.generation(),
            )
            .await?;
        let result: WorkCutoverReadResult =
            serde_json::from_value(result).context("decode Work cutover read")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn execute_work_cutover(
        &mut self,
        params: WorkCutoverExecuteRequest,
    ) -> Result<WorkCutoverExecuteResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "work.cutover.execute",
            AllWorkProtocolCapability::WorkCutoverExecute,
        )?;
        let result = self
            .request(
                "work.cutover.execute",
                Some(serde_json::to_value(params).context("encode Work cutover command")?),
                self.generation(),
            )
            .await?;
        let result: WorkCutoverExecuteResult =
            serde_json::from_value(result).context("decode Work cutover command")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        if result.receipt.github_write_count.0 != 0 {
            return Err(SupervisorError::Anyhow(anyhow!(
                "Work cutover receipt reported a GitHub write"
            )));
        }
        Ok(result)
    }

    pub async fn read_organization_memberships(
        &mut self,
        params: OrganizationMembershipReadRequest,
    ) -> Result<OrganizationMembershipReadResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "organization.membership.read",
            AllWorkProtocolCapability::OrganizationMembershipRead,
        )?;
        let result = self
            .request(
                "organization.membership.read",
                Some(serde_json::to_value(params).context("encode Organization membership read")?),
                self.generation(),
            )
            .await?;
        let result: OrganizationMembershipReadResult =
            serde_json::from_value(result).context("decode Organization membership read")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn read_strict_bug_candidates(
        &mut self,
        params: StrictBugCandidateReadRequest,
    ) -> Result<StrictBugCandidateReadResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "strict_bug.candidate.read",
            AllWorkProtocolCapability::StrictBugCandidateRead,
        )?;
        let result = self
            .request(
                "strict_bug.candidate.read",
                Some(serde_json::to_value(params).context("encode strict bug candidate read")?),
                self.generation(),
            )
            .await?;
        let result: StrictBugCandidateReadResult =
            serde_json::from_value(result).context("decode strict bug candidate read")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn execute_strict_bug_candidate(
        &mut self,
        params: StrictBugCandidateExecuteRequest,
    ) -> Result<StrictBugCandidateExecuteResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        self.require_all_work_capability(
            "strict_bug.candidate.execute",
            AllWorkProtocolCapability::StrictBugCandidateExecute,
        )?;
        let result = self
            .request(
                "strict_bug.candidate.execute",
                Some(serde_json::to_value(params).context("encode strict bug candidate command")?),
                self.generation(),
            )
            .await?;
        let result: StrictBugCandidateExecuteResult =
            serde_json::from_value(result).context("decode strict bug candidate command")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        if result.receipt.github_write_count.0 != 0 {
            return Err(SupervisorError::Anyhow(anyhow!(
                "strict bug candidate receipt reported a GitHub write"
            )));
        }
        Ok(result)
    }

    pub async fn query_forensic_prior_work(
        &mut self,
        params: ForensicPriorWorkQuery,
    ) -> Result<ForensicPriorWorkQueryResult, SupervisorError> {
        params
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        let result = self
            .request(
                "forensics.prior_work.query",
                Some(serde_json::to_value(params).context("encode forensic prior-work query")?),
                self.generation(),
            )
            .await?;
        let result: ForensicPriorWorkQueryResult =
            serde_json::from_value(result).context("decode forensic prior-work query result")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn submit_forensic_prior_work(
        &mut self,
        params: ForensicPriorWorkSubmission,
    ) -> Result<ForensicPriorWorkRecord, SupervisorError> {
        let result = self
            .request(
                "forensics.prior_work.submit",
                Some(
                    serde_json::to_value(params)
                        .context("encode forensic prior-work submission")?,
                ),
                self.generation(),
            )
            .await?;
        let result: ForensicPriorWorkRecord =
            serde_json::from_value(result).context("decode forensic prior-work record")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn relate_forensic_prior_work(
        &mut self,
        params: ForensicRelationCommand,
    ) -> Result<ForensicPriorWorkRecord, SupervisorError> {
        let result = self
            .request(
                "forensics.prior_work.relate",
                Some(serde_json::to_value(params).context("encode forensic prior-work relation")?),
                self.generation(),
            )
            .await?;
        let result: ForensicPriorWorkRecord =
            serde_json::from_value(result).context("decode related forensic prior-work record")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn dispose_forensic_prior_work(
        &mut self,
        params: ForensicDispositionCommand,
    ) -> Result<ForensicPriorWorkRecord, SupervisorError> {
        let result = self
            .request(
                "forensics.prior_work.dispose",
                Some(serde_json::to_value(params).context("encode forensic disposition")?),
                self.generation(),
            )
            .await?;
        let result: ForensicPriorWorkRecord =
            serde_json::from_value(result).context("decode disposed forensic prior-work record")?;
        result
            .validate()
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        Ok(result)
    }

    pub async fn list_runs(&mut self) -> Result<Vec<RunSnapshot>, SupervisorError> {
        let result = self.request("list_runs", None, self.generation()).await?;
        let runs = result
            .get("runs")
            .cloned()
            .ok_or_else(|| anyhow!("list_runs missing runs"))?;
        Ok(serde_json::from_value(runs).context("decode list_runs")?)
    }

    pub async fn get_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "get_run",
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("run")
            .cloned()
            .ok_or_else(|| anyhow!("get_run missing run"))?)
    }

    pub async fn start_run(&mut self, params: Value) -> Result<Value, SupervisorError> {
        let result = self
            .request("start", Some(params), self.generation())
            .await?;
        Ok(result
            .get("run")
            .cloned()
            .ok_or_else(|| anyhow!("start missing run"))?)
    }

    pub async fn pause_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.mutate_run("pause", run_ref).await
    }

    pub async fn resume_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.mutate_run("resume", run_ref).await
    }

    pub async fn handoff_run(
        &mut self,
        run_ref: &str,
        target_lane_ref: &str,
    ) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "handoff",
                Some(json!({
                    "runRef": run_ref,
                    "targetLaneRef": target_lane_ref,
                })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("run")
            .cloned()
            .ok_or_else(|| anyhow!("handoff missing run"))?)
    }

    pub async fn stop_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.mutate_run("stop", run_ref).await
    }

    pub async fn retry_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.mutate_run("retry", run_ref).await
    }

    pub async fn get_capacity(&mut self) -> Result<Value, SupervisorError> {
        self.request("get_capacity", None, self.generation()).await
    }

    pub async fn decide_attention(
        &mut self,
        run_ref: &str,
        permission_granted: bool,
        previous_dedup_key: Option<&str>,
    ) -> Result<Option<AttentionDecision>, SupervisorError> {
        let result = self
            .request(
                "decide_attention",
                Some(json!({
                    "runRef": run_ref,
                    "permissionGranted": permission_granted,
                    "previousDedupKey": previous_dedup_key,
                })),
                self.generation(),
            )
            .await?;
        let attention = result.get("attention").cloned().unwrap_or(Value::Null);
        Ok(serde_json::from_value(attention).context("decode attention decision")?)
    }

    pub async fn get_report(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "get_report",
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("report")
            .cloned()
            .ok_or_else(|| anyhow!("get_report missing report"))?)
    }

    pub async fn get_receipt(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "get_receipt",
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("receipt")
            .cloned()
            .ok_or_else(|| anyhow!("get_receipt missing receipt"))?)
    }

    pub async fn apply_control_intent(
        &mut self,
        intent_id: &str,
        run_ref: &str,
        action: &str,
    ) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "apply_control_intent",
                Some(json!({
                    "intentId": intent_id,
                    "runRef": run_ref,
                    "action": action,
                })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("outcome")
            .cloned()
            .ok_or_else(|| anyhow!("apply_control_intent missing outcome"))?)
    }

    pub async fn get_sync_status(&mut self) -> Result<Value, SupervisorError> {
        self.request("get_sync_status", None, self.generation())
            .await
    }

    pub async fn publish_projection(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.request(
            "publish_projection",
            Some(json!({ "runRef": run_ref })),
            self.generation(),
        )
        .await
    }

    pub async fn get_native_binding(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "get_native_binding",
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result.get("binding").cloned().unwrap_or(Value::Null))
    }

    pub async fn assess_native_boundary(
        &mut self,
        run_ref: &str,
    ) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "assess_native_boundary",
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("assessment")
            .cloned()
            .ok_or_else(|| anyhow!("assess_native_boundary missing assessment"))?)
    }

    pub async fn start_agent_computer_session(
        &mut self,
        params: Value,
    ) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "start_agent_computer_session",
                Some(params),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("session")
            .cloned()
            .ok_or_else(|| anyhow!("start_agent_computer_session missing session"))?)
    }

    pub async fn refresh_agent_computer_session(
        &mut self,
        bearer_token: &str,
        session_ref: &str,
    ) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "refresh_agent_computer_session",
                Some(json!({
                    "bearerToken": bearer_token,
                    "sessionRef": session_ref,
                })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("session")
            .cloned()
            .ok_or_else(|| anyhow!("refresh_agent_computer_session missing session"))?)
    }

    pub async fn run_agent_computer_turn(
        &mut self,
        params: Value,
    ) -> Result<Value, SupervisorError> {
        self.request_with_timeout(
            "run_agent_computer_turn",
            Some(params),
            self.generation(),
            AGENT_COMPUTER_TURN_TIMEOUT,
        )
        .await
    }

    pub async fn get_agent_computer_session(
        &mut self,
        session_ref: &str,
    ) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "get_agent_computer_session",
                Some(json!({ "sessionRef": session_ref })),
                self.generation(),
            )
            .await?;
        Ok(result.get("session").cloned().unwrap_or(Value::Null))
    }

    pub async fn list_agent_computer_sessions(&mut self) -> Result<Value, SupervisorError> {
        self.request("list_agent_computer_sessions", None, self.generation())
            .await
    }

    /// SARAH-NR-06 / OMEGA-SW-03: public-safe session projection (never returns tokens).
    pub async fn sarah_session_status(&mut self) -> Result<Value, SupervisorError> {
        self.request("sarah_session_status", None, self.generation())
            .await
    }

    /// SARAH-NR-06 / OMEGA-SW-03: principal projection and conversation reference.
    pub async fn sarah_bootstrap(&mut self) -> Result<Value, SupervisorError> {
        self.request("sarah_bootstrap", None, self.generation())
            .await
    }

    /// SARAH-NR-06 / OMEGA-SW-03: bounded transcript/activity page with cursors and gap state.
    pub async fn sarah_room_snapshot(
        &mut self,
        params: Option<Value>,
    ) -> Result<Value, SupervisorError> {
        self.request("sarah_room_snapshot", params, self.generation())
            .await
    }

    /// SARAH-NR-06: publish an owner message onto the Nostr conversation.
    pub async fn sarah_send_message(
        &mut self,
        text: &str,
        idempotency_ref: &str,
    ) -> Result<Value, SupervisorError> {
        let generation = self.generation();
        self.request(
            "sarah_send_message",
            Some(json!({
                "text": text,
                "idempotencyRef": idempotency_ref,
                "expectedGeneration": generation,
            })),
            generation,
        )
        .await
    }

    /// SARAH-NR-06 / OMEGA-SW-03: publish a cancel_turn control intent (pending until settled).
    pub async fn sarah_interrupt_turn(
        &mut self,
        turn_ref: &str,
        idempotency_ref: &str,
    ) -> Result<Value, SupervisorError> {
        let generation = self.generation();
        self.request(
            "sarah_interrupt_turn",
            Some(json!({
                "turnRef": turn_ref,
                "idempotencyRef": idempotency_ref,
                "expectedGeneration": generation,
            })),
            generation,
        )
        .await
    }

    pub async fn sarah_device_grants(&mut self) -> Result<Value, SupervisorError> {
        self.request("sarah_device_grants", None, self.generation())
            .await
    }

    pub async fn sarah_renew_device_grant(
        &mut self,
        grant_ref: &str,
        scopes: &[crate::Issue31PairingScope],
        expires_at: u64,
        idempotency_ref: &str,
    ) -> Result<Value, SupervisorError> {
        let generation = self.generation();
        self.request(
            "sarah_renew_device_grant",
            Some(json!({
                "grantRef": grant_ref,
                "scopes": scopes,
                "expiresAt": expires_at,
                "idempotencyRef": idempotency_ref,
                "expectedGeneration": generation,
            })),
            generation,
        )
        .await
    }

    pub async fn sarah_revoke_device_grant(
        &mut self,
        grant_ref: &str,
        reason_ref: &str,
        idempotency_ref: &str,
    ) -> Result<Value, SupervisorError> {
        let generation = self.generation();
        self.request(
            "sarah_revoke_device_grant",
            Some(json!({
                "grantRef": grant_ref,
                "reasonRef": reason_ref,
                "idempotencyRef": idempotency_ref,
                "expectedGeneration": generation,
            })),
            generation,
        )
        .await
    }

    /// Re-admit the device behind a revoked grant so it may pair again.
    ///
    /// Revocation fails closed for the device, so this is the owner's only path
    /// back. It grants nothing on its own — the device must still complete a
    /// fresh signed pairing handshake.
    pub async fn sarah_readmit_device(
        &mut self,
        grant_ref: &str,
        idempotency_ref: &str,
    ) -> Result<Value, SupervisorError> {
        let generation = self.generation();
        self.request(
            "sarah_readmit_device",
            Some(json!({
                "grantRef": grant_ref,
                "idempotencyRef": idempotency_ref,
                "expectedGeneration": generation,
            })),
            generation,
        )
        .await
    }

    async fn mutate_run(&mut self, method: &str, run_ref: &str) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                method,
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("run")
            .cloned()
            .ok_or_else(|| anyhow!("{method} missing run"))?)
    }

    pub async fn restart(&mut self) -> Result<InitializeResult> {
        self.stop().await?;
        let next = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.generation.store(next, Ordering::SeqCst);
        self.start().await
    }

    pub async fn stop(&mut self) -> Result<()> {
        self.negotiated_all_work = None;
        if let Some(mut child) = self.child.take() {
            self.stdin.take();
            self.stdout.take();
            #[cfg(unix)]
            {
                let signal_result = unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
                if signal_result != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(error).context("terminate omega-effectd");
                    }
                }
            }

            #[cfg(unix)]
            let exited = smol::future::or(async { child.status().await.map(|_| true) }, async {
                runtime_delay(SHUTDOWN_GRACE_PERIOD).await;
                Ok(false)
            })
            .await
            .context("wait for omega-effectd shutdown")?;

            #[cfg(not(unix))]
            let exited = false;

            if !exited {
                child.kill().context("kill unresponsive omega-effectd")?;
                child.status().await.context("reap killed omega-effectd")?;
            }
        }
        Ok(())
    }

    async fn spawn_child(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.options.data_root)
            .with_context(|| format!("create data root {}", self.options.data_root.display()))?;

        let mut command = std::process::Command::new(&self.options.command.program);
        command.args(&self.options.command.args);
        command.env(
            "OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT",
            &self.options.data_root,
        );

        let mut child = Child::spawn(command, Stdio::piped(), Stdio::piped(), Stdio::piped())
            .with_context(|| {
                format!(
                    "spawn omega-effectd {}",
                    redact_command(&format!("{:?}", self.options.command))
                )
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("omega-effectd stdin missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("omega-effectd stdout missing"))?;
        if let Some(stderr) = child.stderr.take() {
            smol::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Some(line) = lines.next().await {
                    match line {
                        Ok(line) => eprintln!("omega-effectd: {}", redact_command(&line)),
                        Err(error) => {
                            eprintln!("omega-effectd stderr read failed: {error}");
                            break;
                        }
                    }
                }
            })
            .detach();
        }

        self.stdin = Some(stdin);
        self.stdout = Some(BufReader::new(stdout));
        self.child = Some(child);
        Ok(())
    }

    async fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
        generation: u64,
    ) -> Result<Value, SupervisorError> {
        self.request_with_timeout(method, params, generation, self.options.request_timeout)
            .await
    }

    async fn request_with_timeout(
        &mut self,
        method: &str,
        params: Option<Value>,
        generation: u64,
        timeout: Duration,
    ) -> Result<Value, SupervisorError> {
        let id = self
            .next_request_id
            .fetch_add(1, Ordering::SeqCst)
            .to_string();
        let frame = request_frame(id.clone(), generation, method, params);
        let line =
            serde_json::to_string(&frame).map_err(|error| SupervisorError::Anyhow(error.into()))?;
        if line.len() > MAX_FRAME_BYTES {
            return Err(SupervisorError::Anyhow(anyhow!(
                "omega-effectd request frame exceeds {MAX_FRAME_BYTES} bytes"
            )));
        }
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("omega-effectd not started"))?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        stdin
            .flush()
            .await
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;

        let response_result = smol::future::or(
            async {
                loop {
                    let response_limit = if matches!(
                        method,
                        "planning.graph.read"
                            | "repository.claim.read"
                            | "repository.claim.execute"
                            | "workroom.activity.read"
                            | "workroom.activity.prepare"
                            | "workroom.activity.commit"
                            | "workroom.activity.enqueue"
                            | "workroom.activity.deliver"
                            | "workroom.activity.publish"
                            | "work.command.execute"
                            | "work.cutover.read"
                            | "work.cutover.execute"
                            | "organization.membership.read"
                            | "strict_bug.candidate.read"
                            | "strict_bug.candidate.execute"
                            | "forensics.prior_work.query"
                            | "forensics.prior_work.submit"
                            | "forensics.prior_work.relate"
                            | "forensics.prior_work.dispose"
                    ) {
                        MAX_ALL_WORK_GRAPH_RESPONSE_BYTES
                    } else {
                        MAX_FRAME_BYTES
                    };
                    let line = read_bounded_line(
                        self.stdout
                            .as_mut()
                            .ok_or_else(|| anyhow!("omega-effectd stdout missing"))?,
                        response_limit,
                    )
                    .await?
                    .ok_or_else(|| anyhow!("omega-effectd closed stdout"))?;
                    let frame: Value = serde_json::from_str(&line)
                        .context("decode omega-effectd protocol frame")?;
                    match frame.get("kind").and_then(Value::as_str) {
                        Some("host_request") => {
                            if line.len() > MAX_FRAME_BYTES {
                                bail!("omega-effectd host request frame exceeds {MAX_FRAME_BYTES} bytes");
                            }
                            let request: HostRequestFrame = serde_json::from_value(frame)
                                .context("decode omega-effectd host request")?;
                            self.respond_to_host_request(request, generation).await?;
                            continue;
                        }
                        Some("event") => continue,
                        Some("response") => {}
                        _ => bail!("omega-effectd emitted an invalid frame kind"),
                    }
                    let response: ResponseFrame = serde_json::from_value(frame)
                        .context("decode omega-effectd response frame")?;
                    if response.schema != PROTOCOL_SCHEMA {
                        bail!("omega-effectd response used an invalid schema");
                    }
                    if response.id != id {
                        continue;
                    }
                    if response.generation != generation {
                        bail!(
                            "omega-effectd response used stale generation {}; expected {generation}",
                            response.generation
                        );
                    }
                    return Ok::<ResponseFrame, anyhow::Error>(response);
                }
            },
            async {
                runtime_delay(timeout).await;
                Err(anyhow!("omega-effectd request timed out after {timeout:?}"))
            },
        )
        .await;

        let response = match response_result {
            Ok(response) => response,
            Err(error) => {
                if let Err(stop_error) = self.stop().await {
                    return Err(SupervisorError::Anyhow(error.context(format!(
                        "omega-effectd request failed; child teardown also failed: {stop_error:#}"
                    ))));
                }
                return Err(SupervisorError::Anyhow(error));
            }
        };

        if !response.ok {
            let error = response.error.unwrap_or(crate::protocol::ProtocolError {
                code: ProtocolErrorCode::Internal,
                message: "request failed without error body".to_string(),
            });
            return Err(match error.code {
                ProtocolErrorCode::StaleGeneration => SupervisorError::StaleGeneration,
                code => SupervisorError::Protocol {
                    code,
                    message: error.message,
                },
            });
        }
        response
            .result
            .ok_or_else(|| SupervisorError::Anyhow(anyhow!("ok response missing result")))
    }

    async fn respond_to_host_request(
        &mut self,
        request: HostRequestFrame,
        expected_generation: u64,
    ) -> Result<()> {
        if request.schema != PROTOCOL_SCHEMA || request.kind != "host_request" {
            bail!("omega-effectd emitted an invalid host request envelope");
        }
        if request.id.is_empty() || request.id.len() > 180 {
            bail!("omega-effectd emitted an invalid host request id");
        }

        let response = if request.generation != expected_generation
            || request.generation != self.generation()
        {
            HostResponseFrame::failure(
                &request,
                HostResponseError {
                    code: HostResponseErrorCode::StaleGeneration,
                    message: format!(
                        "Host request generation {} does not match active generation {}.",
                        request.generation,
                        self.generation()
                    ),
                },
            )
        } else if request.method == HostMethod::Unsupported {
            HostResponseFrame::failure(
                &request,
                HostResponseError {
                    code: HostResponseErrorCode::Unsupported,
                    message: "The requested Omega host method is unsupported.".to_string(),
                },
            )
        } else if let Some(handler) = self.host_handler.clone() {
            let host_request_timeout = self.host_request_timeout;
            let result = smol::future::or(handler(request.clone()), async move {
                runtime_delay(host_request_timeout).await;
                Err(HostResponseError::unavailable(format!(
                    "Omega host authority timed out after {host_request_timeout:?}."
                )))
            })
            .await;
            match result {
                Ok(result) => HostResponseFrame::success(&request, result),
                Err(mut error) => {
                    error.message = truncate_utf8(
                        &redact_command(&error.message),
                        MAX_HOST_ERROR_MESSAGE_BYTES,
                    );
                    HostResponseFrame::failure(&request, error)
                }
            }
        } else {
            HostResponseFrame::failure(
                &request,
                HostResponseError::unavailable(format!(
                    "Omega host authority for {:?} is unavailable.",
                    request.method
                )),
            )
        };
        self.write_frame(&response).await
    }

    async fn write_frame(&mut self, frame: &impl serde::Serialize) -> Result<()> {
        let line = serde_json::to_string(frame).context("encode omega-effectd host response")?;
        if line.len() > MAX_FRAME_BYTES {
            bail!("omega-effectd host response frame exceeds {MAX_FRAME_BYTES} bytes");
        }
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("omega-effectd stdin missing"))?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }
}

impl Drop for OmegaEffectdSupervisor {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().log_err();
        }
    }
}

#[allow(clippy::disallowed_methods)]
async fn runtime_delay(duration: Duration) {
    // The supervisor is also used without a GPUI application context by protocol clients and tests.
    smol::Timer::after(duration).await;
}

fn truncate_utf8(message: &str, max_bytes: usize) -> String {
    if message.len() <= max_bytes {
        return message.to_string();
    }
    let mut boundary = max_bytes;
    while !message.is_char_boundary(boundary) {
        boundary -= 1;
    }
    message[..boundary].to_string()
}

async fn read_bounded_line(
    reader: &mut BufReader<smol::process::ChildStdout>,
    max_bytes: usize,
) -> Result<Option<String>> {
    let mut frame = Vec::new();
    loop {
        let (consumed, found_newline) = {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                if frame.is_empty() {
                    return Ok(None);
                }
                bail!("omega-effectd closed stdout with an incomplete frame");
            }
            let consumed = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            let payload_length = if available.get(consumed.saturating_sub(1)) == Some(&b'\n') {
                consumed - 1
            } else {
                consumed
            };
            if frame.len() + payload_length > max_bytes {
                bail!("omega-effectd response frame exceeds {max_bytes} bytes");
            }
            frame.extend_from_slice(&available[..payload_length]);
            (consumed, payload_length < consumed)
        };
        reader.consume_unpin(consumed);
        if found_newline {
            if frame.last() == Some(&b'\r') {
                frame.pop();
            }
            return String::from_utf8(frame)
                .context("omega-effectd response frame was not UTF-8")
                .map(Some);
        }
    }
}

pub fn resolve_effectd_command(
    override_program: Option<&OsStr>,
    app_executable: &Path,
) -> Result<OmegaEffectdCommand> {
    if let Some(program) = override_program.filter(|value| !value.is_empty()) {
        let program = PathBuf::from(program);
        if !program.is_file() {
            bail!(
                "OPENAGENTS_OMEGA_EFFECTD_BIN does not name a packaged executable: {}",
                program.display()
            );
        }
        return Ok(OmegaEffectdCommand {
            program,
            args: Vec::new(),
        });
    }

    let executable_dir = app_executable
        .parent()
        .ok_or_else(|| anyhow!("Omega executable has no parent directory"))?;
    let bundled_program = if executable_dir.file_name() == Some(OsStr::new("MacOS")) {
        executable_dir
            .parent()
            .ok_or_else(|| anyhow!("Omega macOS bundle has no Contents directory"))?
            .join("Resources/omega-effectd/bin/omega-effectd")
    } else {
        executable_dir.join("omega-effectd/bin/omega-effectd")
    };
    if !bundled_program.is_file() {
        bail!(
            "packaged omega-effectd component is unavailable at {}",
            bundled_program.display()
        );
    }
    Ok(OmegaEffectdCommand {
        program: bundled_program,
        args: Vec::new(),
    })
}

/// Shared test helper: fixture command that speaks the framed protocol.
pub fn fixture_command(fixture: &Path) -> OmegaEffectdCommand {
    let node = [
        std::env::var_os("NODE")
            .map(PathBuf::from)
            .filter(|path| path.exists()),
        which_node(),
        Some(PathBuf::from(
            "/Users/christopherdavid/.nvm/versions/node/v25.8.2/bin/node",
        ))
        .filter(|path| path.exists()),
        Some(PathBuf::from("/opt/homebrew/bin/node")).filter(|path| path.exists()),
        Some(PathBuf::from("/usr/local/bin/node")).filter(|path| path.exists()),
    ]
    .into_iter()
    .flatten()
    .next()
    .unwrap_or_else(|| PathBuf::from("node"));

    OmegaEffectdCommand {
        program: node,
        args: vec![fixture.display().to_string()],
    }
}

fn which_node() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("node");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

pub fn default_options(
    data_root: PathBuf,
    command: OmegaEffectdCommand,
) -> OmegaEffectdSupervisorOptions {
    OmegaEffectdSupervisorOptions {
        data_root,
        command,
        initial_generation: 1,
        request_timeout: DEFAULT_REQUEST_TIMEOUT,
    }
}
