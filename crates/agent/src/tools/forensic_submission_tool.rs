use std::sync::Arc;

use agent_client_protocol::schema::v1 as acp;
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AgentTool, ToolCallEventStream, ToolInput};

/// A canonical forensic call bound to the run, task, actor, audience, source
/// bundle, prompt, model route, tool version, budget, and event cursor.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicSubmissionToolInput {
    pub call: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForensicSubmissionToolOutput {
    pub state: String,
    pub admission: String,
}

impl From<ForensicSubmissionToolOutput> for LanguageModelToolResultContent {
    fn from(value: ForensicSubmissionToolOutput) -> Self {
        serde_json::to_string(&value)
            .unwrap_or_else(|error| format!("forensic tool output serialization failed: {error}"))
            .into()
    }
}

macro_rules! forensic_tool {
    ($type:ident, $name:literal, $title:literal, $description:literal) => {
        pub struct $type;

        impl AgentTool for $type {
            type Input = ForensicSubmissionToolInput;
            type Output = ForensicSubmissionToolOutput;

            const NAME: &'static str = $name;

            fn kind() -> acp::ToolKind {
                acp::ToolKind::Other
            }

            fn description() -> SharedString {
                $description.into()
            }

            fn initial_title(
                &self,
                _input: Result<Self::Input, serde_json::Value>,
                _cx: &mut App,
            ) -> SharedString {
                $title.into()
            }

            fn run(
                self: Arc<Self>,
                input: ToolInput<Self::Input>,
                _event_stream: ToolCallEventStream,
                cx: &mut App,
            ) -> Task<Result<Self::Output, Self::Output>> {
                cx.spawn(async move |_cx| {
                    let input =
                        input
                            .recv()
                            .await
                            .map_err(|error| ForensicSubmissionToolOutput {
                                state: "rejected".into(),
                                admission: format!("typed input unavailable: {error}"),
                            })?;
                    if !input.call.is_object() {
                        return Err(ForensicSubmissionToolOutput {
                            state: "rejected".into(),
                            admission: "call must be a canonical structured object".into(),
                        });
                    }
                    Ok(ForensicSubmissionToolOutput {
                        state: "submitted".into(),
                        admission:
                            "Omega Forensics will validate this call against the live journal"
                                .into(),
                    })
                })
            }
        }
    };
}

forensic_tool!(
    QueryPriorForensicWorkTool,
    "query_prior_forensic_work",
    "Querying prior forensic Work",
    "Query authorized prior forensic Work using one canonical bound call."
);
forensic_tool!(
    GetForensicWorkByRefTool,
    "get_forensic_work_by_ref",
    "Getting prior forensic Work",
    "Get authorized forensic Work by stable Work reference using one canonical bound call."
);
forensic_tool!(
    SubmitForensicHypothesisTool,
    "submit_forensic_hypothesis",
    "Submitting forensic hypothesis",
    "Submit an uncertain forensic hypothesis with missing evidence and a required next check."
);
forensic_tool!(
    SubmitForensicFindingTool,
    "submit_forensic_finding",
    "Submitting forensic finding",
    "Submit a source-grounded forensic finding using one canonical bound call."
);
forensic_tool!(
    SubmitForensicLimitationTool,
    "submit_forensic_limitation",
    "Submitting forensic limitation",
    "Submit a forensic limitation with an exact required next check."
);
forensic_tool!(
    ValidateCandidateDiffApplicabilityTool,
    "validate_candidate_diff_applicability",
    "Validating candidate diff applicability",
    "Record artifact-only candidate diff applicability; this tool never records execution."
);
forensic_tool!(
    ExecuteIndependentControlTool,
    "execute_independent_control",
    "Recording independent executed control",
    "Record an executed control receipt. This tool is reserved for an independently identified verifier."
);
