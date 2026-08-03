//! Lossless shared-Work views over source-owned security analysis projections.
//!
//! This adapter does not become forensic authority. It preserves exact source
//! references and explicit uncertainty in inspectable Work Blocks.

use omega_effectd::all_work_contract::{
    EventRef, EvidenceRef, ReceiptRef, RunRef, SessionRef, SourceRef, WorkRef, WorkRelation,
    WorkRelationKind,
};
use omega_forensics::{
    ColdcardEvidenceWorkspaceProjection, EntropyCampaignProjection, EntropyRunProjection,
    ForensicExactness, ForensicLifecycleState, ForensicPromptArtifact,
    ForensicPublicationGateProjection, ForensicPublicationGateState, ForensicsLaunchIntent,
    ForensicsMatrixProjection, ForensicsReviewProjection, ForensicsRunPhase,
    ForensicsRunProjection, PreflightReadiness,
};
use omega_work_detail::{
    SnapshotLinks, WorkBlock, WorkBlockFact, WorkBlockFactKind, WorkBlockFactState, WorkBlockKind,
};
use omega_work_index::{WorkIndexItem, WorkSourceEntity};

pub struct ForensicsWorkProjection<'a> {
    pub item: &'a WorkIndexItem,
    pub related_run_refs: &'a [String],
    pub run: Option<&'a ForensicsRunProjection>,
    pub review: Option<&'a ForensicsReviewProjection>,
    pub matrix: Option<&'a ForensicsMatrixProjection>,
    pub entropy_run: Option<&'a EntropyRunProjection>,
    pub entropy_campaign: Option<&'a EntropyCampaignProjection>,
    pub publication: Option<&'a ForensicPublicationGateProjection>,
    pub readiness: Option<PreflightReadiness>,
    pub launch_intent: Option<&'a ForensicsLaunchIntent>,
    pub prompt: Option<&'a ForensicPromptArtifact>,
    pub coldcard_fixture: Option<&'a ColdcardEvidenceWorkspaceProjection>,
    pub source_loaded: bool,
}

pub struct ProjectedForensicsWork {
    pub links: SnapshotLinks,
    pub blocks: Vec<WorkBlock>,
}

pub fn project_forensics_work(
    source: ForensicsWorkProjection<'_>,
) -> Result<ProjectedForensicsWork, omega_effectd::all_work_contract::ContractValidationError> {
    let mut links = SnapshotLinks::default();
    project_relations(&source, &mut links)?;

    let mut case_facts = vec![fact(
        format!("fact:security-work:source:{}", source.item.source_ref()),
        WorkBlockFactKind::Source,
        if source.source_loaded {
            WorkBlockFactState::Observed
        } else {
            WorkBlockFactState::Unavailable
        },
        "Exact source authority",
        source.item.source_ref(),
        [source.item.source_ref()],
    )];
    if !source.source_loaded {
        case_facts.push(fact(
            format!("fact:security-work:unloaded:{}", source.item.work_ref()),
            WorkBlockFactKind::MissingInput,
            WorkBlockFactState::Missing,
            "Source projection",
            "Select this repository and refresh its source-owned analysis to load case details.",
            [source.item.source_ref()],
        ));
    }

    let mut lifecycle_facts = Vec::new();
    let mut evidence_facts = Vec::new();
    let mut model_facts = Vec::new();
    let mut publication_facts = Vec::new();

    if let Some(prompt) = source.prompt {
        case_facts.push(fact(
            format!("fact:security-work:prompt:{}", prompt.prompt_artifact_ref),
            WorkBlockFactKind::Source,
            WorkBlockFactState::Observed,
            "Immutable prompt lineage",
            format!(
                "digest {} · dataset {}",
                prompt.canonical_digest, prompt.dataset_revision_ref
            ),
            [&prompt.prompt_artifact_ref, &prompt.dataset_revision_ref],
        ));
    }
    if let Some(readiness) = source.readiness {
        lifecycle_facts.push(fact(
            format!("fact:security-work:preflight:{}", source.item.work_ref()),
            WorkBlockFactKind::Lifecycle,
            match readiness {
                PreflightReadiness::Ready => WorkBlockFactState::Completed,
                PreflightReadiness::AwaitingCoverage => WorkBlockFactState::Active,
                PreflightReadiness::IncompleteResearch => WorkBlockFactState::Missing,
                PreflightReadiness::Denied => WorkBlockFactState::Blocked,
            },
            "Preflight readiness",
            format!("{readiness:?}"),
            [source.item.source_ref()],
        ));
    }
    if let Some(intent) = source.launch_intent {
        lifecycle_facts.push(fact(
            format!("fact:security-work:launch-intent:{}", intent.preflight_ref),
            WorkBlockFactKind::Lifecycle,
            if intent.incomplete {
                WorkBlockFactState::Missing
            } else {
                WorkBlockFactState::Observed
            },
            "Prepared launch intent",
            format!(
                "coverage {:?} · model {} · effort {}",
                intent.coverage_status, intent.budget.model_ref, intent.budget.effort_ref
            ),
            [&intent.preflight_ref, &intent.operator_action_ref],
        ));
    }

    if let Some(run) = source.run {
        push_run(run, &mut lifecycle_facts, &mut links);
    }
    if let Some(review) = source.review {
        push_review(
            review,
            &mut case_facts,
            &mut lifecycle_facts,
            &mut evidence_facts,
            &mut model_facts,
            &mut links,
        );
    }
    if let Some(entropy) = source.entropy_run {
        push_entropy_run(
            entropy,
            &mut case_facts,
            &mut lifecycle_facts,
            &mut evidence_facts,
            &mut model_facts,
            &mut links,
        );
    }
    if let Some(campaign) = source.entropy_campaign {
        push_campaign(
            campaign,
            &mut case_facts,
            &mut lifecycle_facts,
            &mut evidence_facts,
            &mut model_facts,
            &mut links,
        );
    }
    if let Some(matrix) = source.matrix {
        push_matrix(matrix, &mut model_facts, &mut evidence_facts, &mut links);
    }
    if let Some(coldcard) = source.coldcard_fixture {
        push_coldcard_fixture(coldcard, &mut case_facts, &mut evidence_facts, &mut links);
    }
    push_publication(source.publication, &mut publication_facts, &mut links);

    if lifecycle_facts.is_empty() {
        lifecycle_facts.push(fact(
            format!("fact:security-work:lifecycle:{}", source.item.work_ref()),
            WorkBlockFactKind::Lifecycle,
            WorkBlockFactState::Unavailable,
            "Lifecycle",
            "No source-owned lifecycle projection is loaded.",
            [source.item.source_ref()],
        ));
    }
    if evidence_facts.is_empty() {
        evidence_facts.push(fact(
            format!("fact:security-work:evidence:{}", source.item.work_ref()),
            WorkBlockFactKind::Evidence,
            WorkBlockFactState::Unavailable,
            "Evidence",
            "No source-grounded findings, hypotheses, or limitations are loaded.",
            [source.item.source_ref()],
        ));
    }
    if model_facts.is_empty() {
        model_facts.push(fact(
            format!("fact:security-work:models:{}", source.item.work_ref()),
            WorkBlockFactKind::Model,
            WorkBlockFactState::Unavailable,
            "Models and usage",
            "No source-owned model comparison or qualified usage projection is loaded.",
            [source.item.source_ref()],
        ));
    }

    let blocks = vec![
        block(&source, WorkBlockKind::Case, case_facts)?,
        block(&source, WorkBlockKind::Lifecycle, lifecycle_facts)?,
        block(&source, WorkBlockKind::Evidence, evidence_facts)?,
        block(&source, WorkBlockKind::Models, model_facts)?,
        block(&source, WorkBlockKind::Publication, publication_facts)?,
    ];
    Ok(ProjectedForensicsWork { links, blocks })
}

fn project_relations(
    source: &ForensicsWorkProjection<'_>,
    links: &mut SnapshotLinks,
) -> Result<(), omega_effectd::all_work_contract::ContractValidationError> {
    match &source.item.source_entity {
        WorkSourceEntity::ForensicsCase { .. } => {
            for run_ref in source.related_run_refs {
                links.relations.push(WorkRelation {
                    kind: WorkRelationKind::Child,
                    target_work_ref: WorkRef::try_from(format!(
                        "work:omega:forensics-run:{run_ref}"
                    ))?,
                });
                push_run_ref(links, run_ref);
            }
        }
        WorkSourceEntity::ForensicsRun { case_ref, run_ref } => {
            links.relations.push(WorkRelation {
                kind: WorkRelationKind::Parent,
                target_work_ref: WorkRef::try_from(format!("work:omega:forensics:{case_ref}"))?,
            });
            push_run_ref(links, run_ref);
        }
        WorkSourceEntity::Thread { .. } | WorkSourceEntity::EffectWork { .. } => {}
    }
    Ok(())
}

fn block(
    source: &ForensicsWorkProjection<'_>,
    kind: WorkBlockKind,
    facts: Vec<WorkBlockFact>,
) -> Result<WorkBlock, omega_effectd::all_work_contract::ContractValidationError> {
    Ok(WorkBlock {
        block_ref: SourceRef::try_from(format!(
            "block:omega:security-work:{}:{}",
            source.item.work_ref(),
            kind.label().to_ascii_lowercase()
        ))?,
        work_ref: source.item.summary.work_ref.clone(),
        kind,
        title: omega_effectd::all_work_contract::ShortText::try_from(kind.label().to_string())?,
        source_ref: source.item.summary.source_authority.source_ref.clone(),
        available: source.source_loaded || kind == WorkBlockKind::Publication,
        facts,
    })
}

fn fact<const N: usize>(
    fact_ref: String,
    kind: WorkBlockFactKind,
    state: WorkBlockFactState,
    label: impl Into<String>,
    value: impl Into<String>,
    source_refs: [&str; N],
) -> WorkBlockFact {
    WorkBlockFact {
        fact_ref,
        kind,
        state,
        label: label.into(),
        value: value.into(),
        source_refs: source_refs.into_iter().map(str::to_string).collect(),
    }
}

fn fact_vec(
    fact_ref: String,
    kind: WorkBlockFactKind,
    state: WorkBlockFactState,
    label: impl Into<String>,
    value: impl Into<String>,
    source_refs: Vec<String>,
) -> WorkBlockFact {
    WorkBlockFact {
        fact_ref,
        kind,
        state,
        label: label.into(),
        value: value.into(),
        source_refs,
    }
}

fn push_run(
    run: &ForensicsRunProjection,
    lifecycle: &mut Vec<WorkBlockFact>,
    links: &mut SnapshotLinks,
) {
    push_run_ref(links, &run.run_ref);
    for event in &run.events {
        push_event_ref(links, &event.event_ref);
    }
    let state = match run.phase {
        ForensicsRunPhase::Prepared
        | ForensicsRunPhase::Admitting
        | ForensicsRunPhase::WorkerReady
        | ForensicsRunPhase::Running
        | ForensicsRunPhase::CancelRequested
        | ForensicsRunPhase::Interrupting
        | ForensicsRunPhase::Deleting => WorkBlockFactState::Active,
        ForensicsRunPhase::Settled | ForensicsRunPhase::Cleaned => WorkBlockFactState::Completed,
        ForensicsRunPhase::Refused | ForensicsRunPhase::Failed => WorkBlockFactState::Failed,
        ForensicsRunPhase::RecoveryRequired => WorkBlockFactState::Blocked,
    };
    lifecycle.push(fact(
        format!("fact:security-work:run-phase:{}", run.run_ref),
        WorkBlockFactKind::Lifecycle,
        state,
        "Run phase",
        format!("{:?} · event cursor {}", run.phase, run.event_cursor),
        [&run.run_ref],
    ));
    if let Some(placement) = &run.placement {
        let refs = [
            placement.placement_ref.as_str(),
            placement.sandbox_ref.as_str(),
        ];
        lifecycle.push(fact(
            format!("fact:security-work:placement:{}", placement.placement_ref),
            WorkBlockFactKind::Lifecycle,
            if matches!(
                placement.state,
                omega_forensics::WorkerPlacementState::Cleaned
            ) {
                WorkBlockFactState::Completed
            } else {
                WorkBlockFactState::Observed
            },
            "Generation-bound placement",
            format!(
                "{:?} · attachment {} · resource {}",
                placement.state, placement.attachment_generation, placement.resource_generation
            ),
            refs,
        ));
        for receipt in [
            placement.admission_receipt_ref.as_deref(),
            placement.readiness_receipt_ref.as_deref(),
            placement.stop_receipt_ref.as_deref(),
            placement.deletion_receipt_ref.as_deref(),
            placement.cleanup_receipt_ref.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            push_receipt_ref(links, receipt);
        }
    }
    if let Some(failure) = &run.failure {
        lifecycle.push(fact(
            format!("fact:security-work:failure:{}", failure.reason_ref),
            WorkBlockFactKind::Lifecycle,
            WorkBlockFactState::Failed,
            format!("{:?}", failure.class),
            failure.message.clone(),
            [&failure.reason_ref],
        ));
    }
}

fn push_review(
    review: &ForensicsReviewProjection,
    case: &mut Vec<WorkBlockFact>,
    lifecycle: &mut Vec<WorkBlockFact>,
    evidence: &mut Vec<WorkBlockFact>,
    models: &mut Vec<WorkBlockFact>,
    links: &mut SnapshotLinks,
) {
    push_run_ref(links, &review.run_ref);
    case.push(fact(
        format!("fact:security-work:review:{}", review.review_ref),
        WorkBlockFactKind::Source,
        WorkBlockFactState::Observed,
        "Pinned review source",
        format!(
            "{} at {} · prompt {}",
            review.repository_ref, review.commit, review.prompt_digest
        ),
        [&review.review_ref, &review.repository_ref],
    ));
    for stage in &review.lifecycle {
        if let Some(receipt) = stage.receipt_ref.as_deref() {
            push_receipt_ref(links, receipt);
        }
        lifecycle.push(fact(
            format!("fact:security-work:lifecycle-stage:{}", stage.stage_ref),
            WorkBlockFactKind::Lifecycle,
            lifecycle_state(stage.state),
            stage.label.clone(),
            stage
                .observed_at
                .clone()
                .unwrap_or_else(|| "Not observed".into()),
            [&stage.stage_ref],
        ));
    }
    lifecycle.push(fact(
        format!("fact:security-work:cleanup:{}", review.review_ref),
        WorkBlockFactKind::Cleanup,
        if review.cleanup_receipt_ref.is_some() {
            WorkBlockFactState::Completed
        } else if review.outcome == omega_forensics::ForensicReviewOutcome::CleanupFailed {
            WorkBlockFactState::Failed
        } else {
            WorkBlockFactState::Missing
        },
        "Cleanup truth",
        review.cleanup_state.clone(),
        [&review.review_ref],
    ));
    if let Some(receipt) = review.cleanup_receipt_ref.as_deref() {
        push_receipt_ref(links, receipt);
    }
    for finding in &review.findings {
        push_evidence_ref(links, &finding.finding_ref);
        let source_refs = finding
            .source_refs
            .iter()
            .map(|citation| citation.source_ref.clone())
            .chain(
                finding
                    .evidence_receipts
                    .iter()
                    .map(|receipt| receipt.receipt_ref.clone()),
            )
            .collect::<Vec<_>>();
        for receipt in &finding.evidence_receipts {
            push_receipt_ref(links, &receipt.receipt_ref);
        }
        evidence.push(fact_vec(
            format!("fact:security-work:finding:{}", finding.finding_ref),
            WorkBlockFactKind::Finding,
            if finding.claim_state.eq_ignore_ascii_case("accepted") {
                WorkBlockFactState::Accepted
            } else {
                WorkBlockFactState::Provisional
            },
            finding.title.clone(),
            format!(
                "{} · {} · {}",
                finding.severity,
                finding.evidence_tier.label(),
                finding.impact
            ),
            source_refs,
        ));
    }
    for hypothesis in &review.hypotheses {
        push_evidence_ref(links, &hypothesis.hypothesis_ref);
        evidence.push(fact_vec(
            format!(
                "fact:security-work:hypothesis:{}",
                hypothesis.hypothesis_ref
            ),
            WorkBlockFactKind::Hypothesis,
            WorkBlockFactState::Provisional,
            hypothesis.suspected_mechanism.clone(),
            format!("{} · next: {}", hypothesis.state, hypothesis.next_check),
            hypothesis.supporting_refs.clone(),
        ));
    }
    for decision in &review.decisions {
        evidence.push(fact(
            format!("fact:security-work:decision:{}", decision.decision_ref),
            WorkBlockFactKind::Dispute,
            match decision.decision {
                omega_forensics::ForensicReviewDecisionKind::Accept => WorkBlockFactState::Accepted,
                omega_forensics::ForensicReviewDecisionKind::Correct => {
                    WorkBlockFactState::Provisional
                }
                omega_forensics::ForensicReviewDecisionKind::Reject => WorkBlockFactState::Rejected,
            },
            format!("Review decision · {:?}", decision.decision),
            decision.reason.clone(),
            [&decision.finding_ref, &decision.reviewer_ref],
        ));
    }
    for metric in &review.metrics {
        for event_ref in &metric.source_event_refs {
            push_event_ref(links, event_ref);
        }
        for receipt_ref in &metric.source_receipt_refs {
            push_receipt_ref(links, receipt_ref);
        }
        models.push(fact_vec(
            format!("fact:security-work:metric:{}", metric.metric_ref),
            WorkBlockFactKind::Exactness,
            exactness_state(metric.exactness),
            metric.label.clone(),
            metric.display_value(),
            metric
                .source_event_refs
                .iter()
                .chain(&metric.source_receipt_refs)
                .cloned()
                .collect(),
        ));
    }
    models.push(fact(
        format!("fact:security-work:budget:{}", review.review_ref),
        WorkBlockFactKind::Usage,
        if review.budget_state == omega_forensics::ForensicBudgetState::Unmeasurable {
            WorkBlockFactState::Unavailable
        } else {
            WorkBlockFactState::Observed
        },
        "Budget state",
        format!("{:?}", review.budget_state),
        [&review.review_ref],
    ));
}

fn push_entropy_run(
    run: &EntropyRunProjection,
    case: &mut Vec<WorkBlockFact>,
    lifecycle: &mut Vec<WorkBlockFact>,
    evidence: &mut Vec<WorkBlockFact>,
    models: &mut Vec<WorkBlockFact>,
    links: &mut SnapshotLinks,
) {
    push_run_ref(links, &run.binding.run_ref);
    case.push(fact(
        format!(
            "fact:security-work:entropy-manifest:{}",
            run.binding.manifest_ref
        ),
        WorkBlockFactKind::Source,
        WorkBlockFactState::Observed,
        "Immutable entropy lineage",
        format!(
            "manifest {} · prompt {}",
            run.binding.manifest_digest, run.binding.prompt_digest
        ),
        [
            &run.binding.manifest_ref,
            &run.binding.prompt_snapshot.prompt_ref,
        ],
    ));
    lifecycle.push(fact(
        format!("fact:security-work:entropy-run:{}", run.binding.run_ref),
        WorkBlockFactKind::Lifecycle,
        entropy_phase_state(run.phase),
        "Entropy run",
        format!("{:?} · {} files", run.phase, run.files.len()),
        [&run.binding.run_ref],
    ));
    lifecycle.push(fact(
        format!("fact:security-work:entropy-summary:{}", run.binding.run_ref),
        WorkBlockFactKind::Lifecycle,
        entropy_accounting_state(run.summary.outcome),
        "Canonical entropy accounting",
        format!(
            "{:?} · {}/{} sessions settled · {}/{} tools available · {} findings · {} hypotheses · cleanup {:?}",
            run.summary.outcome,
            run.summary.sessions.settled,
            run.summary.source.eligible_focal_units,
            run.summary.tools.available,
            run.summary.tools.requested,
            run.summary.outputs.findings,
            run.summary.outputs.hypotheses,
            run.summary.cleanup.state,
        ),
        [&run.binding.run_ref, &run.summary.canonical_digest],
    ));
    models.push(fact(
        format!("fact:security-work:entropy-model:{}", run.binding.run_ref),
        WorkBlockFactKind::Model,
        WorkBlockFactState::Observed,
        "Entropy model route",
        run.binding.model_route_ref.clone(),
        [&run.binding.prompt_snapshot.prompt_ref],
    ));
    for limitation in &run.limitations {
        evidence.push(fact(
            format!("fact:security-work:limitation:{}", limitation.reason_ref),
            WorkBlockFactKind::Limitation,
            WorkBlockFactState::Missing,
            format!("{:?}", limitation.class),
            limitation.message.clone(),
            [&limitation.reason_ref],
        ));
    }
    for file in &run.files {
        for observation in &file.observations {
            push_evidence_ref(links, &observation.observation_ref);
            evidence.push(fact_vec(
                format!(
                    "fact:security-work:observation:{}",
                    observation.observation_ref
                ),
                WorkBlockFactKind::Finding,
                WorkBlockFactState::Observed,
                observation.title.clone(),
                format!(
                    "{} · {}",
                    observation.analyzed_file, observation.confidence_boundary
                ),
                observation
                    .source_refs
                    .iter()
                    .map(|source| source.source_ref.clone())
                    .collect(),
            ));
        }
        for hypothesis in &file.hypotheses {
            push_evidence_ref(links, &hypothesis.hypothesis_ref);
            evidence.push(fact_vec(
                format!(
                    "fact:security-work:hypothesis:{}",
                    hypothesis.hypothesis_ref
                ),
                WorkBlockFactKind::Hypothesis,
                WorkBlockFactState::Provisional,
                hypothesis.title.clone(),
                format!(
                    "{} · next: {}",
                    hypothesis.confidence_boundary, hypothesis.next_check
                ),
                hypothesis
                    .causal_links
                    .iter()
                    .flat_map(|link| {
                        link.source_refs
                            .iter()
                            .map(|source| source.source_ref.clone())
                    })
                    .collect(),
            ));
        }
    }
}

fn push_campaign(
    campaign: &EntropyCampaignProjection,
    case: &mut Vec<WorkBlockFact>,
    lifecycle: &mut Vec<WorkBlockFact>,
    evidence: &mut Vec<WorkBlockFact>,
    models: &mut Vec<WorkBlockFact>,
    links: &mut SnapshotLinks,
) {
    case.push(fact(
        format!(
            "fact:security-work:campaign:{}",
            campaign.binding.campaign_ref
        ),
        WorkBlockFactKind::Source,
        WorkBlockFactState::Observed,
        "Source-aware campaign",
        format!(
            "{} projects · catalog {}",
            campaign.projects.len(),
            campaign.binding.catalog_digest
        ),
        [
            &campaign.binding.campaign_ref,
            &campaign.binding.catalog_ref,
        ],
    ));
    lifecycle.push(fact(
        format!(
            "fact:security-work:campaign-phase:{}",
            campaign.binding.campaign_ref
        ),
        WorkBlockFactKind::Lifecycle,
        match campaign.phase {
            omega_forensics::EntropyCampaignPhase::Ready
            | omega_forensics::EntropyCampaignPhase::Running
            | omega_forensics::EntropyCampaignPhase::Paused => WorkBlockFactState::Active,
            omega_forensics::EntropyCampaignPhase::Completed
            | omega_forensics::EntropyCampaignPhase::CompletedWithLimitations => {
                WorkBlockFactState::Completed
            }
            omega_forensics::EntropyCampaignPhase::Cancelled => WorkBlockFactState::Canceled,
        },
        "Campaign phase",
        format!("{:?}", campaign.phase),
        [&campaign.binding.campaign_ref],
    ));
    models.push(fact(
        format!(
            "fact:security-work:campaign-model:{}",
            campaign.binding.campaign_ref
        ),
        WorkBlockFactKind::Model,
        WorkBlockFactState::Observed,
        "Campaign model route",
        format!(
            "{} · prompt {}",
            campaign.binding.model_route_ref, campaign.binding.prompt_digest
        ),
        [
            &campaign.binding.model_route_ref,
            &campaign.binding.prompt_snapshot.prompt_ref,
        ],
    ));
    for project in &campaign.projects {
        let refs = project
            .product
            .repository_ref
            .iter()
            .cloned()
            .chain(project.limitation_refs.iter().cloned())
            .collect::<Vec<_>>();
        lifecycle.push(fact_vec(
            format!(
                "fact:security-work:campaign-project:{}",
                project.product.product_ref
            ),
            WorkBlockFactKind::Lifecycle,
            match project.phase {
                omega_forensics::EntropyCampaignProjectPhase::Queued
                | omega_forensics::EntropyCampaignProjectPhase::Running => {
                    WorkBlockFactState::Active
                }
                omega_forensics::EntropyCampaignProjectPhase::Completed
                | omega_forensics::EntropyCampaignProjectPhase::CompletedWithLimitations => {
                    WorkBlockFactState::Completed
                }
                omega_forensics::EntropyCampaignProjectPhase::Cancelled => {
                    WorkBlockFactState::Canceled
                }
                omega_forensics::EntropyCampaignProjectPhase::SourceUnavailable
                | omega_forensics::EntropyCampaignProjectPhase::InputIncomplete => {
                    WorkBlockFactState::Missing
                }
                omega_forensics::EntropyCampaignProjectPhase::ProviderFailed
                | omega_forensics::EntropyCampaignProjectPhase::SourceFailed => {
                    WorkBlockFactState::Failed
                }
            },
            project.product.product_name.clone(),
            project.phase.label(),
            refs,
        ));
        if let Some(run) = &project.run {
            push_run_ref(links, &run.binding.run_ref);
        }
        for limitation in &project.limitation_refs {
            evidence.push(fact(
                format!("fact:security-work:campaign-limitation:{limitation}"),
                WorkBlockFactKind::Limitation,
                WorkBlockFactState::Missing,
                project.product.product_name.clone(),
                project.phase.label(),
                [limitation],
            ));
        }
        models.push(fact(
            format!(
                "fact:security-work:campaign-usage:{}",
                project.product.product_ref
            ),
            WorkBlockFactKind::Usage,
            if project.usage.total_tokens.is_some() {
                WorkBlockFactState::Observed
            } else {
                WorkBlockFactState::Unavailable
            },
            format!("{} usage", project.product.product_name),
            project
                .usage
                .total_tokens
                .map(|tokens| format!("{tokens} tokens · {:?}", project.usage.exactness))
                .unwrap_or_else(|| "Unavailable · exactness unavailable".into()),
            [&project.product.product_ref],
        ));
    }
}

fn push_matrix(
    matrix: &ForensicsMatrixProjection,
    models: &mut Vec<WorkBlockFact>,
    evidence: &mut Vec<WorkBlockFact>,
    links: &mut SnapshotLinks,
) {
    models.push(fact(
        format!("fact:security-work:matrix:{}", matrix.matrix_ref),
        WorkBlockFactKind::Model,
        if matrix.promoted {
            WorkBlockFactState::Accepted
        } else {
            WorkBlockFactState::Provisional
        },
        "Model comparison matrix",
        format!(
            "{} arms · {} runs · promoted {}",
            matrix.arms.len(),
            matrix.runs.len(),
            matrix.promoted
        ),
        [&matrix.matrix_ref],
    ));
    for arm in &matrix.arms {
        push_session_ref(links, &arm.provider_session_ref);
        models.push(fact_vec(
            format!("fact:security-work:model-arm:{}", arm.arm_ref),
            WorkBlockFactKind::Model,
            WorkBlockFactState::Observed,
            format!("Model arm {}", arm.arm_ref),
            format!(
                "family {} · role {} · effort {} · seed {}",
                arm.model_family_ref, arm.role_ref, arm.effort_ref, arm.random_seed
            ),
            vec![
                arm.arm_ref.clone(),
                arm.prompt_digest.clone(),
                arm.model_digest.clone(),
                arm.tool_surface_digest.clone(),
                arm.source_bundle_digest.clone(),
                arm.provider_session_ref.clone(),
                arm.worker_state_ref.clone(),
            ],
        ));
    }
    for row in &matrix.rows {
        for run_ref in &row.run_refs {
            push_run_ref(links, run_ref);
        }
        for event_ref in &row.event_refs {
            push_event_ref(links, event_ref);
        }
        for receipt_ref in &row.receipt_refs {
            push_receipt_ref(links, receipt_ref);
        }
        models.push(fact_vec(
            format!("fact:security-work:matrix-row:{}", row.arm_ref),
            WorkBlockFactKind::Exactness,
            if matches!(row.token_exactness, ForensicExactness::Unavailable)
                || matches!(row.cost_exactness, ForensicExactness::Unavailable)
            {
                WorkBlockFactState::Unavailable
            } else {
                WorkBlockFactState::Observed
            },
            format!("Model arm {}", row.arm_ref),
            format!(
                "{} samples · tokens {:?} · cost {:?}",
                row.sample_count, row.token_exactness, row.cost_exactness
            ),
            row.run_refs.clone(),
        ));
    }
    for (arm, finding_refs) in &matrix.finding_divergence.unique_finding_refs_by_arm {
        evidence.push(fact_vec(
            format!("fact:security-work:disagreement:{}", arm),
            WorkBlockFactKind::Dispute,
            WorkBlockFactState::Provisional,
            format!("Model disagreement · {arm}"),
            format!("{} unique findings", finding_refs.len()),
            finding_refs.clone(),
        ));
    }
}

fn push_publication(
    publication: Option<&ForensicPublicationGateProjection>,
    facts: &mut Vec<WorkBlockFact>,
    links: &mut SnapshotLinks,
) {
    let Some(publication) = publication else {
        facts.push(fact(
            "fact:security-work:publication:unavailable".into(),
            WorkBlockFactKind::PublicationAuthority,
            WorkBlockFactState::Blocked,
            "Publication authority",
            "No source-owned publication gate projection is attached. The case remains private and unauthorized for publication.",
            [],
        ));
        facts.push(fact(
            "fact:security-work:privacy:private".into(),
            WorkBlockFactKind::Privacy,
            WorkBlockFactState::Observed,
            "Privacy",
            "Private by default. Visibility, relay state, model votes, completed processes, and clean results do not authorize a claim.",
            [],
        ));
        return;
    };
    facts.push(fact(
        format!("fact:security-work:publication:{}", publication.case_ref),
        WorkBlockFactKind::Privacy,
        if publication.private {
            WorkBlockFactState::Observed
        } else {
            WorkBlockFactState::Accepted
        },
        "Publication projection",
        format!(
            "private {} · synthetic {} · authorized {}",
            publication.private, publication.synthetic, publication.publication_authorized
        ),
        [&publication.case_ref],
    ));
    for gate in &publication.gates {
        if let Some(evidence_ref) = gate.evidence_ref.as_deref() {
            push_evidence_ref(links, evidence_ref);
        }
        facts.push(fact(
            format!("fact:security-work:publication-gate:{}", gate.gate_ref),
            WorkBlockFactKind::PublicationAuthority,
            match gate.state {
                ForensicPublicationGateState::Satisfied => WorkBlockFactState::Accepted,
                ForensicPublicationGateState::Denied | ForensicPublicationGateState::Rejected => {
                    WorkBlockFactState::Rejected
                }
                ForensicPublicationGateState::Blocked
                | ForensicPublicationGateState::AwaitingReview
                | ForensicPublicationGateState::Stale
                | ForensicPublicationGateState::EligibleNotAuthorized => {
                    WorkBlockFactState::Blocked
                }
            },
            format!("{:?}", gate.kind),
            format!("{} · next: {}", gate.blocker, gate.next_action),
            [&gate.gate_ref],
        ));
    }
}

fn push_coldcard_fixture(
    workspace: &ColdcardEvidenceWorkspaceProjection,
    case: &mut Vec<WorkBlockFact>,
    evidence: &mut Vec<WorkBlockFact>,
    links: &mut SnapshotLinks,
) {
    push_run_ref(links, &workspace.run_ref);
    case.push(fact(
        format!(
            "fact:security-work:coldcard-fixture:{}",
            workspace.workspace_ref
        ),
        WorkBlockFactKind::Source,
        WorkBlockFactState::Provisional,
        "Development evidence fixture",
        "Synthetic Coldcard evidence is visible only because the explicit development/mock fixture gate is active.",
        [&workspace.workspace_ref, &workspace.run_ref],
    ));
    for rung in &workspace.ladder {
        for evidence_ref in &rung.evidence_refs {
            push_evidence_ref(links, evidence_ref);
        }
        evidence.push(fact_vec(
            format!(
                "fact:security-work:coldcard-rung:{}:{:?}",
                workspace.workspace_ref, rung.rung
            ),
            WorkBlockFactKind::Evidence,
            match rung.state {
                omega_forensics::ColdcardRungState::Missing => WorkBlockFactState::Missing,
                omega_forensics::ColdcardRungState::Provisional => WorkBlockFactState::Provisional,
                omega_forensics::ColdcardRungState::Qualified => WorkBlockFactState::Observed,
                omega_forensics::ColdcardRungState::IndependentlyVerified => {
                    WorkBlockFactState::Accepted
                }
            },
            rung.rung.label(),
            format!(
                "{} · does not imply {}",
                rung.verifier_state,
                rung.non_implications.join(", ")
            ),
            rung.evidence_refs.clone(),
        ));
    }
    for reconciliation in &workspace.reconciliation {
        evidence.push(fact_vec(
            format!(
                "fact:security-work:coldcard-reconciliation:{}",
                reconciliation.metric_ref
            ),
            WorkBlockFactKind::Reconciliation,
            match reconciliation.status {
                omega_forensics::ColdcardReconciliationStatus::Match => {
                    WorkBlockFactState::Observed
                }
                omega_forensics::ColdcardReconciliationStatus::Drift => {
                    WorkBlockFactState::Provisional
                }
                omega_forensics::ColdcardReconciliationStatus::Unavailable => {
                    WorkBlockFactState::Unavailable
                }
            },
            format!("Reconciliation · {}", reconciliation.metric_ref),
            format!(
                "derived {} · published {} · precision {}",
                reconciliation
                    .derived_value
                    .as_deref()
                    .unwrap_or("unavailable"),
                reconciliation
                    .published_value
                    .as_deref()
                    .unwrap_or("unavailable"),
                reconciliation.precision_ref
            ),
            reconciliation.source_refs.clone(),
        ));
    }
    for correction in &workspace.corrections {
        evidence.push(fact_vec(
            format!(
                "fact:security-work:coldcard-correction:{}:{}",
                correction.claim_ref, correction.sequence
            ),
            WorkBlockFactKind::Dispute,
            WorkBlockFactState::Observed,
            format!("Claim correction · {}", correction.claim_ref),
            format!(
                "{} → {} · {}",
                correction.prior_value, correction.corrected_value, correction.reason_ref
            ),
            correction.appended_evidence_refs.clone(),
        ));
    }
}

fn lifecycle_state(state: ForensicLifecycleState) -> WorkBlockFactState {
    match state {
        ForensicLifecycleState::Pending => WorkBlockFactState::Missing,
        ForensicLifecycleState::Active => WorkBlockFactState::Active,
        ForensicLifecycleState::Succeeded => WorkBlockFactState::Completed,
        ForensicLifecycleState::Failed => WorkBlockFactState::Failed,
        ForensicLifecycleState::Cancelled | ForensicLifecycleState::Censored => {
            WorkBlockFactState::Canceled
        }
    }
}

fn entropy_phase_state(phase: omega_forensics::EntropyRunPhase) -> WorkBlockFactState {
    match phase {
        omega_forensics::EntropyRunPhase::Ready
        | omega_forensics::EntropyRunPhase::Running
        | omega_forensics::EntropyRunPhase::CancelRequested
        | omega_forensics::EntropyRunPhase::AwaitingCleanup => WorkBlockFactState::Active,
        omega_forensics::EntropyRunPhase::Completed
        | omega_forensics::EntropyRunPhase::CompletedWithLimitations => {
            WorkBlockFactState::Completed
        }
        omega_forensics::EntropyRunPhase::Failed
        | omega_forensics::EntropyRunPhase::FailedWithPartialOutput => WorkBlockFactState::Failed,
        omega_forensics::EntropyRunPhase::Cancelled => WorkBlockFactState::Canceled,
    }
}

fn entropy_accounting_state(
    outcome: omega_forensics::EntropyAccountingOutcome,
) -> WorkBlockFactState {
    match outcome {
        omega_forensics::EntropyAccountingOutcome::Active
        | omega_forensics::EntropyAccountingOutcome::RecoveryRequired => WorkBlockFactState::Active,
        omega_forensics::EntropyAccountingOutcome::Completed
        | omega_forensics::EntropyAccountingOutcome::CompletedIncomplete => {
            WorkBlockFactState::Completed
        }
        omega_forensics::EntropyAccountingOutcome::Failed
        | omega_forensics::EntropyAccountingOutcome::FailedWithPartialOutput => {
            WorkBlockFactState::Failed
        }
        omega_forensics::EntropyAccountingOutcome::Cancelled => WorkBlockFactState::Canceled,
    }
}

fn exactness_state(exactness: ForensicExactness) -> WorkBlockFactState {
    match exactness {
        ForensicExactness::Exact => WorkBlockFactState::Observed,
        ForensicExactness::Estimated | ForensicExactness::UpperBound => {
            WorkBlockFactState::Provisional
        }
        ForensicExactness::Unavailable => WorkBlockFactState::Unavailable,
    }
}

fn push_run_ref(links: &mut SnapshotLinks, value: &str) {
    if let Ok(value) = RunRef::try_from(value.to_string())
        && !links.run_refs.contains(&value)
    {
        links.run_refs.push(value);
    }
}

fn push_event_ref(links: &mut SnapshotLinks, value: &str) {
    if let Ok(value) = EventRef::try_from(value.to_string())
        && !links.event_refs.contains(&value)
    {
        links.event_refs.push(value);
    }
}

fn push_session_ref(links: &mut SnapshotLinks, value: &str) {
    if let Ok(value) = SessionRef::try_from(value.to_string())
        && !links.session_refs.contains(&value)
    {
        links.session_refs.push(value);
    }
}

fn push_receipt_ref(links: &mut SnapshotLinks, value: &str) {
    if let Ok(value) = ReceiptRef::try_from(value.to_string())
        && !links.receipt_refs.contains(&value)
    {
        links.receipt_refs.push(value);
    }
}

fn push_evidence_ref(links: &mut SnapshotLinks, value: &str) {
    if let Ok(value) = EvidenceRef::try_from(value.to_string())
        && !links.evidence_refs.contains(&value)
    {
        links.evidence_refs.push(value);
    }
}

#[cfg(test)]
mod tests {
    use omega_forensics::{
        ForensicPublicationGate, ForensicPublicationGateKind, ForensicPublicationGateProjection,
        ForensicPublicationGateState, ForensicsRunProjection, PUBLICATION_GATE_SCHEMA_V1,
    };
    use omega_work_index::{NativeForensicsPhase, NativeForensicsRecord, adapt_forensics};

    use super::*;

    fn case_and_run() -> Vec<WorkIndexItem> {
        adapt_forensics(NativeForensicsRecord {
            case_ref: "repository:7".into(),
            repository_name: "Omega".into(),
            updated_at: "2026-08-03T00:00:00Z".into(),
            observed_at: "2026-08-03T00:00:01Z".into(),
            revision: 4,
            phase: NativeForensicsPhase::Running,
            run_ref: Some("run:security:7".into()),
            child_run_refs: Vec::new(),
        })
        .expect("valid source rows")
    }

    #[test]
    fn case_and_run_preserve_bidirectional_identity_and_fail_closed_authority() {
        let rows = case_and_run();
        let related = vec!["run:security:7".to_string()];
        let case = project_forensics_work(ForensicsWorkProjection {
            item: &rows[0],
            related_run_refs: &related,
            run: None,
            review: None,
            matrix: None,
            entropy_run: None,
            entropy_campaign: None,
            publication: None,
            readiness: None,
            launch_intent: None,
            prompt: None,
            coldcard_fixture: None,
            source_loaded: false,
        })
        .expect("project case");
        assert_eq!(case.blocks.len(), 5);
        assert_eq!(case.links.relations.len(), 1);
        assert_eq!(case.links.relations[0].kind, WorkRelationKind::Child);
        assert!(case.blocks.iter().any(|block| {
            block.kind == WorkBlockKind::Publication
                && block.facts.iter().any(|fact| {
                    fact.kind == WorkBlockFactKind::PublicationAuthority
                        && fact.state == WorkBlockFactState::Blocked
                })
        }));

        let run = project_forensics_work(ForensicsWorkProjection {
            item: &rows[1],
            related_run_refs: &related,
            run: None,
            review: None,
            matrix: None,
            entropy_run: None,
            entropy_campaign: None,
            publication: None,
            readiness: None,
            launch_intent: None,
            prompt: None,
            coldcard_fixture: None,
            source_loaded: false,
        })
        .expect("project run");
        assert_eq!(run.links.relations[0].kind, WorkRelationKind::Parent);
        assert_eq!(
            run.links.relations[0].target_work_ref.0,
            rows[0].summary.work_ref.0
        );
    }

    #[test]
    fn source_run_and_publication_refs_round_trip_without_authority_inference() {
        let rows = case_and_run();
        let related = vec!["run:security:7".to_string()];
        let mut run =
            ForensicsRunProjection::prepared("run:security:7".into()).expect("source-owned run");
        run.phase = ForensicsRunPhase::Cleaned;
        let gate_kinds = [
            ForensicPublicationGateKind::Redaction,
            ForensicPublicationGateKind::IndependentReview,
            ForensicPublicationGateKind::DisclosureScope,
            ForensicPublicationGateKind::MaintainerDecision,
            ForensicPublicationGateKind::PublicationAuthority,
        ];
        let publication = ForensicPublicationGateProjection {
            schema: PUBLICATION_GATE_SCHEMA_V1.into(),
            case_ref: "case.security.7".into(),
            private: true,
            synthetic: false,
            operator_ready: false,
            maintainer_approved: false,
            publication_authorized: false,
            gates: gate_kinds
                .into_iter()
                .enumerate()
                .map(|(index, kind)| ForensicPublicationGate {
                    gate_ref: format!("gate.security.7.{}", index + 1),
                    kind,
                    state: ForensicPublicationGateState::Blocked,
                    evidence_ref: (index == 0).then(|| "evidence.security.redaction.7".into()),
                    blocker: "Source authority is incomplete".into(),
                    next_action: "Attach the exact source-owned decision".into(),
                })
                .collect(),
        };
        publication
            .validate()
            .expect("valid blocked gate projection");
        let projected = project_forensics_work(ForensicsWorkProjection {
            item: &rows[0],
            related_run_refs: &related,
            run: Some(&run),
            review: None,
            matrix: None,
            entropy_run: None,
            entropy_campaign: None,
            publication: Some(&publication),
            readiness: Some(PreflightReadiness::Ready),
            launch_intent: None,
            prompt: None,
            coldcard_fixture: None,
            source_loaded: true,
        })
        .expect("project source data");

        assert!(
            projected
                .links
                .run_refs
                .iter()
                .any(|value| value.0 == run.run_ref)
        );
        assert!(
            projected
                .links
                .evidence_refs
                .iter()
                .any(|value| { value.0 == "evidence.security.redaction.7" })
        );
        let publication_block = projected
            .blocks
            .iter()
            .find(|block| block.kind == WorkBlockKind::Publication)
            .expect("Publication Block");
        assert_eq!(publication_block.facts.len(), 6);
        assert!(publication_block.facts.iter().all(|fact| {
            fact.kind != WorkBlockFactKind::PublicationAuthority
                || fact.state == WorkBlockFactState::Blocked
        }));
        assert!(projected.blocks.iter().any(|block| {
            block.kind == WorkBlockKind::Lifecycle
                && block.facts.iter().any(|fact| {
                    fact.fact_ref == "fact:security-work:run-phase:run:security:7"
                        && fact.state == WorkBlockFactState::Completed
                })
        }));
    }
}
