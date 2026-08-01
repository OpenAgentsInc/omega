use gpui::{App, Context, FocusHandle, Focusable, Render, SharedString, Window};
use omega_forensics::{
    ColdcardBenchmarkArm, CoverageStatus, DependencyPolicy, ExplicitOperatorAction,
    ForensicsLaunchIntent, ForensicsPreflightProjection, PreflightReadiness, SourceState,
};
use omega_workbench_state::RepositoryBinding;
use ui::{
    Button, ButtonSize, ButtonStyle, Color, Icon, IconName, IconSize, Label, LabelSize, prelude::*,
    v_flex,
};

use crate::thread_identity::ThreadIdentityCandidate;

const PREPARE_ACTION_REF: &str = "operator-action-ref://omega/forensics/prepare-run";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsRepositoryContext {
    pub display_name: SharedString,
    pub clone_url: Option<SharedString>,
    pub commit: Option<SharedString>,
    pub dirty_files: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsWorkbenchSnapshot {
    pub binding: RepositoryBinding,
    pub selected_arm: ColdcardBenchmarkArm,
    pub readiness: Option<PreflightReadiness>,
    pub prepared_intent: Option<ForensicsLaunchIntent>,
    pub status: SharedString,
}

pub struct ForensicsWorkbenchSurface {
    focus_handle: FocusHandle,
    binding: RepositoryBinding,
    repository: ForensicsRepositoryContext,
    selected_arm: ColdcardBenchmarkArm,
    preflight: Option<ForensicsPreflightProjection>,
    prepared_intent: Option<ForensicsLaunchIntent>,
    status: SharedString,
}

impl ForensicsWorkbenchSurface {
    pub fn new(candidate: &ThreadIdentityCandidate, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            binding: candidate.binding.clone(),
            repository: ForensicsRepositoryContext {
                display_name: candidate.repository_name.clone(),
                clone_url: candidate.remote_url.clone(),
                commit: candidate.head_commit.clone(),
                dirty_files: candidate.git.dirty_files,
            },
            selected_arm: ColdcardBenchmarkArm::Vulnerable,
            preflight: None,
            prepared_intent: None,
            status: "Awaiting OpenAgents managed profile".into(),
        }
    }

    pub fn binding(&self) -> &RepositoryBinding {
        &self.binding
    }

    pub fn set_managed_preflight(
        &mut self,
        binding: &RepositoryBinding,
        projection: ForensicsPreflightProjection,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            binding == &self.binding,
            "the managed preflight belongs to a different repository binding"
        );
        projection.validate()?;
        let selected_arm = projection
            .target
            .benchmark_arm
            .unwrap_or(ColdcardBenchmarkArm::Vulnerable);
        self.selected_arm = selected_arm;
        self.prepared_intent = None;
        self.status = readiness_label(projection.readiness()).into();
        self.preflight = Some(projection);
        cx.notify();
        Ok(())
    }

    pub fn select_benchmark_arm(&mut self, arm: ColdcardBenchmarkArm, cx: &mut Context<Self>) {
        self.selected_arm = arm;
        self.prepared_intent = None;
        if let Some(preflight) = self.preflight.as_mut() {
            preflight.set_benchmark_arm(arm);
            self.status = "Coverage pending".into();
        }
        cx.notify();
    }

    pub fn acknowledge_incomplete(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let preflight = self
            .preflight
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the managed preflight is unavailable"))?;
        preflight.acknowledge_incomplete()?;
        self.status = "Incomplete research acknowledged".into();
        cx.notify();
        Ok(())
    }

    pub fn prepare_run(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let preflight = self
            .preflight
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("the managed preflight is unavailable"))?;
        let intent = preflight.request_launch(ExplicitOperatorAction {
            action_ref: PREPARE_ACTION_REF.into(),
        })?;
        self.prepared_intent = Some(intent);
        self.status = "Run prepared; no worker launched".into();
        cx.notify();
        Ok(())
    }

    pub fn snapshot(&self) -> ForensicsWorkbenchSnapshot {
        ForensicsWorkbenchSnapshot {
            binding: self.binding.clone(),
            selected_arm: self.selected_arm,
            readiness: self
                .preflight
                .as_ref()
                .map(|preflight| preflight.readiness()),
            prepared_intent: self.prepared_intent.clone(),
            status: self.status.clone(),
        }
    }

    fn render_fact(label: &'static str, value: impl Into<SharedString>) -> impl IntoElement {
        let value = value.into();
        h_flex()
            .w_full()
            .gap_2()
            .justify_between()
            .child(
                Label::new(label)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(Label::new(value).size(LabelSize::XSmall).line_clamp(1))
    }
}

impl Focusable for ForensicsWorkbenchSurface {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ForensicsWorkbenchSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = self.preflight.as_ref().map(|preflight| &preflight.target);
        let repository_name = target
            .map(|target| SharedString::from(target.display_name.clone()))
            .unwrap_or_else(|| self.repository.display_name.clone());
        let clone_url = target
            .map(|target| SharedString::from(target.clone_url.clone()))
            .or_else(|| self.repository.clone_url.clone())
            .unwrap_or_else(|| "No public HTTPS remote".into());
        let commit = target
            .map(|target| SharedString::from(target.commit.clone()))
            .or_else(|| self.repository.commit.clone())
            .unwrap_or_else(|| "Unborn".into());
        let source_state: SharedString = target
            .map(|target| source_state_label(target.source_state).into())
            .unwrap_or_else(|| {
                if self.repository.dirty_files == 0 {
                    "Clean".into()
                } else {
                    format!("Dirty · {} files", self.repository.dirty_files).into()
                }
            });
        let dependency_policy = target
            .map(|target| dependency_policy_label(target.dependency_policy))
            .unwrap_or("Pinned recursive");
        let readiness = self
            .preflight
            .as_ref()
            .map(|preflight| preflight.readiness());
        let coverage = self.preflight.as_ref().map(|preflight| &preflight.coverage);
        let can_prepare = self.preflight.as_ref().is_some_and(|preflight| {
            matches!(preflight.readiness(), PreflightReadiness::Ready)
                || (preflight.readiness() == PreflightReadiness::IncompleteResearch
                    && preflight.incomplete_acknowledged)
        });
        let needs_acknowledgment = self.preflight.as_ref().is_some_and(|preflight| {
            preflight.coverage.status == CoverageStatus::Incomplete
                && !preflight.incomplete_acknowledged
        });

        v_flex()
            .id("omega.forensics.workbench")
            .debug_selector(|| "omega.forensics.workbench".to_string())
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .role(gpui::Role::Group)
            .aria_label("Forensics preflight workbench")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::Crosshair).size(IconSize::Small))
                    .child(Label::new("Forensics").size(LabelSize::Small)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(Self::render_fact("Repository", repository_name))
                    .child(Self::render_fact("Remote", clone_url))
                    .child(Self::render_fact("Commit", commit))
                    .child(Self::render_fact("Source", source_state))
                    .child(Self::render_fact("Dependencies", dependency_policy))
                    .when_some(target, |this, target| {
                        this.child(Self::render_fact(
                            "Scan profile",
                            target.scan_profile_ref.clone(),
                        ))
                    }),
            )
            .child(div().h_px().bg(cx.theme().colors().border))
            .child(
                Label::new("Coldcard benchmark")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                h_flex().flex_wrap().gap_1().children(
                    ColdcardBenchmarkArm::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(index, arm)| {
                            Button::new(("omega.forensics.benchmark", index), arm.label())
                                .size(ButtonSize::Compact)
                                .style(if arm == self.selected_arm {
                                    ButtonStyle::Tinted(ui::TintColor::Accent)
                                } else {
                                    ButtonStyle::Subtle
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_benchmark_arm(arm, cx)
                                }))
                        }),
                ),
            )
            .child(div().h_px().bg(cx.theme().colors().border))
            .child(
                Label::new("Managed worker")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .when_some(self.preflight.as_ref(), |this, preflight| {
                let worker = &preflight.worker;
                this.child(
                    v_flex()
                        .gap_1()
                        .child(Self::render_fact("Supply", "OpenAgents Cloud"))
                        .child(Self::render_fact("Provider", "Google Cloud"))
                        .child(Self::render_fact("Isolation", "GCE VM"))
                        .child(Self::render_fact("Adapter", worker.adapter_ref.clone()))
                        .child(Self::render_fact("Region", worker.region_ref.clone()))
                        .child(Self::render_fact("Custody", worker.custody_ref.clone()))
                        .child(Self::render_fact("Image", worker.image_digest.clone()))
                        .child(Self::render_fact("Profile", worker.profile_digest.clone()))
                        .child(Self::render_fact("Network", "Broker only"))
                        .child(Self::render_fact("Lease", worker.lease_ref.clone()))
                        .child(Self::render_fact(
                            "Lease bound",
                            format!("{} s", worker.lease_seconds),
                        ))
                        .child(Self::render_fact(
                            "Capabilities",
                            worker.capability_refs.len().to_string(),
                        )),
                )
            })
            .when(self.preflight.is_none(), |this| {
                this.child(
                    Label::new("Awaiting an admitted OpenAgents managed GCE profile")
                        .size(LabelSize::Small)
                        .color(Color::Warning),
                )
            })
            .when_some(self.preflight.as_ref(), |this, preflight| {
                let budget = &preflight.budget;
                this.child(div().h_px().bg(cx.theme().colors().border))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Run bounds")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(Self::render_fact("Model", budget.model_ref.clone()))
                            .child(Self::render_fact("Effort", budget.effort_ref.clone()))
                            .child(Self::render_fact(
                                "Concurrency",
                                budget.max_concurrency.to_string(),
                            ))
                            .child(Self::render_fact(
                                "Time",
                                format!("{} s", budget.max_time_seconds),
                            ))
                            .child(Self::render_fact("Tokens", budget.max_tokens.to_string()))
                            .child(Self::render_fact(
                                "Cost",
                                format!("{} µUSD", budget.max_cost_micros),
                            ))
                            .child(Self::render_fact(
                                "Artifacts",
                                format!("{} B", budget.max_artifact_bytes),
                            ))
                            .child(Self::render_fact(
                                "Network",
                                format!("{} B", budget.max_network_bytes),
                            )),
                    )
            })
            .when_some(coverage, |this, coverage| {
                this.child(div().h_px().bg(cx.theme().colors().border))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Coverage")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(Self::render_fact(
                                "State",
                                readiness_label(
                                    readiness.unwrap_or(PreflightReadiness::AwaitingCoverage),
                                ),
                            ))
                            .child(Self::render_fact("Present", coverage.present.to_string()))
                            .child(Self::render_fact("Missing", coverage.missing.to_string()))
                            .child(Self::render_fact("Excluded", coverage.excluded.to_string()))
                            .child(Self::render_fact(
                                "Generated",
                                coverage.generated.to_string(),
                            ))
                            .child(Self::render_fact(
                                "Oversized",
                                coverage.oversized.to_string(),
                            ))
                            .child(Self::render_fact(
                                "Dependency-owned",
                                coverage.dependency_owned.to_string(),
                            )),
                    )
            })
            .child(div().h_px().bg(cx.theme().colors().border))
            .child(
                Label::new(self.status.clone())
                    .size(LabelSize::XSmall)
                    .color(if can_prepare {
                        Color::Success
                    } else {
                        Color::Muted
                    }),
            )
            .when(needs_acknowledgment, |this| {
                this.child(
                    Button::new(
                        "omega.forensics.acknowledge-incomplete",
                        "Acknowledge incomplete",
                    )
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Subtle)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Err(error) = this.acknowledge_incomplete(cx) {
                            this.status = error.to_string().into();
                            cx.notify();
                        }
                    })),
                )
            })
            .child(
                Button::new("omega.forensics.prepare-run", "Prepare run")
                    .size(ButtonSize::Compact)
                    .disabled(!can_prepare)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Err(error) = this.prepare_run(cx) {
                            this.status = error.to_string().into();
                            cx.notify();
                        }
                    })),
            )
    }
}

fn readiness_label(readiness: PreflightReadiness) -> &'static str {
    match readiness {
        PreflightReadiness::AwaitingCoverage => "Coverage pending",
        PreflightReadiness::Ready => "Ready",
        PreflightReadiness::IncompleteResearch => "Incomplete research",
        PreflightReadiness::Denied => "Denied",
    }
}

fn source_state_label(source_state: SourceState) -> &'static str {
    match source_state {
        SourceState::Clean => "Clean",
        SourceState::Dirty => "Dirty",
        SourceState::ExternallyPrepared => "Externally prepared",
    }
}

fn dependency_policy_label(dependency_policy: DependencyPolicy) -> &'static str {
    match dependency_policy {
        DependencyPolicy::PinnedRecursive => "Pinned recursive",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_identity::{BranchIdentity, GitIdentitySummary};
    use omega_forensics::{
        BROKER_NETWORK_POLICY_REF, CoverageSummaryProjection, ForensicBudgetProjection,
        GCE_ADAPTER_REF, MANAGED_TARGET_REF, ManagedIsolation, ManagedProvider, ManagedTargetClass,
        ManagedWorkerProjection, PREFLIGHT_SCHEMA_V1, RepositoryTargetProjection,
    };
    use std::path::PathBuf;

    fn candidate(binding: RepositoryBinding) -> ThreadIdentityCandidate {
        ThreadIdentityCandidate {
            binding,
            git_repository_id: Some(1),
            project_name: "Omega".into(),
            repository_name: "omega".into(),
            worktree_name: "omega".into(),
            worktree_abs_path: PathBuf::from("/work/omega"),
            worktree_path: "/work/omega".into(),
            remote_url: Some("https://github.com/OpenAgentsInc/omega.git".into()),
            head_commit: Some("0123456789abcdef0123456789abcdef01234567".into()),
            branch: BranchIdentity::Branch("main".into()),
            git: GitIdentitySummary::default(),
            source_revision: 1,
        }
    }

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn complete_preflight() -> ForensicsPreflightProjection {
        ForensicsPreflightProjection {
            schema: PREFLIGHT_SCHEMA_V1.into(),
            preflight_ref: "preflight-ref://omega/coldcard-v1".into(),
            repository_binding_ref: "repository-binding-ref://omega/current-worktree".into(),
            target: RepositoryTargetProjection {
                source_state: SourceState::Clean,
                dependency_policy: DependencyPolicy::PinnedRecursive,
                ..RepositoryTargetProjection::coldcard(ColdcardBenchmarkArm::Vulnerable)
            },
            worker: ManagedWorkerProjection {
                target_ref: MANAGED_TARGET_REF.into(),
                target_class: ManagedTargetClass::OpenagentsManaged,
                provider: ManagedProvider::GoogleCloud,
                adapter_ref: GCE_ADAPTER_REF.into(),
                isolation: ManagedIsolation::GceVm,
                region_ref: "region-ref://openagents/us-central1".into(),
                custody_ref: "custody-ref://openagents/operator-owned-v1".into(),
                image_digest: digest('a'),
                profile_digest: digest('b'),
                network_policy_ref: BROKER_NETWORK_POLICY_REF.into(),
                lease_ref: "lease-ref://openagents/forensics/coldcard-v1".into(),
                lease_seconds: 900,
                capability_refs: vec!["capability-ref://forensics/source-read".into()],
            },
            budget: ForensicBudgetProjection {
                model_ref: "model-ref://openai/gpt-5.6".into(),
                effort_ref: "effort-ref://high".into(),
                max_concurrency: 2,
                max_time_seconds: 900,
                max_tokens: 100_000,
                max_cost_micros: 5_000_000,
                max_artifact_bytes: 10_000_000,
                max_network_bytes: 0,
            },
            coverage: CoverageSummaryProjection {
                manifest_ref: Some("coverage-manifest-ref://coldcard/complete-v1".into()),
                status: CoverageStatus::Complete,
                present: 103,
                missing: 0,
                excluded: 0,
                generated: 3,
                oversized: 0,
                dependency_owned: 4,
                reason_refs: Vec::new(),
            },
            incomplete_acknowledged: false,
        }
    }

    #[gpui::test]
    fn benchmark_arms_are_operator_selectable_without_a_managed_profile(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            for arm in ColdcardBenchmarkArm::ALL {
                surface.update(cx, |surface, cx| surface.select_benchmark_arm(arm, cx));
                assert_eq!(surface.read(cx).snapshot().selected_arm, arm);
                assert_eq!(surface.read(cx).snapshot().readiness, None);
            }
        });
    }

    #[gpui::test]
    fn preflight_is_bound_and_only_an_explicit_action_prepares_a_run(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let foreign = RepositoryBinding::new("other-repo", "worktree").expect("valid binding");
            let surface =
                cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding.clone()), cx));
            assert!(
                surface
                    .update(cx, |surface, cx| surface.set_managed_preflight(
                        &foreign,
                        complete_preflight(),
                        cx
                    ))
                    .is_err()
            );
            surface
                .update(cx, |surface, cx| {
                    surface.set_managed_preflight(&binding, complete_preflight(), cx)
                })
                .expect("matching managed preflight");
            assert_eq!(surface.read(cx).snapshot().prepared_intent, None);
            surface
                .update(cx, |surface, cx| surface.prepare_run(cx))
                .expect("explicit operator action prepares a run");
            let snapshot = surface.read(cx).snapshot();
            assert_eq!(snapshot.readiness, Some(PreflightReadiness::Ready));
            assert_eq!(
                snapshot
                    .prepared_intent
                    .as_ref()
                    .map(|intent| intent.operator_action_ref.as_str()),
                Some(PREPARE_ACTION_REF)
            );
        });
    }
}
