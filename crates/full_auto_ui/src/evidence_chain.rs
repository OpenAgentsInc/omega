use serde_json::Value;
use workroom_receipts::{InspectorField, PublicRef};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FullAutoEvidenceView {
    pub fields: Vec<InspectorField>,
}

impl FullAutoEvidenceView {
    pub fn from_records(report: &Value, receipt: &Value) -> Option<Self> {
        let report_run = public_ref(report, "runRef")?;
        let receipt_run = public_ref(receipt, "runRef")?;
        if report_run != receipt_run {
            return None;
        }
        let evidence = report.get("evidence")?;
        let objective_ref = public_ref(evidence, "objectiveRef")?;
        let turn_ref = public_ref(evidence, "turnRef")?;
        let change_ref = public_ref(evidence, "changeRef")?;
        let project_generation = public_ref(evidence, "projectGeneration")?;
        let verification_ref = public_ref(evidence, "verificationRef")?;
        let test_outcome = public_ref(evidence, "testOutcome")?;
        let test_command = bounded_public_command(evidence.get("testCommand")?.as_str()?)?;
        let diff_summary = bounded_public_text(evidence.get("diffSummary")?.as_str()?)?;
        if evidence.get("hostExecuted").and_then(Value::as_bool) != Some(true) {
            return None;
        }

        for (field, expected) in [
            ("objectiveRef", &objective_ref),
            ("turnRef", &turn_ref),
            ("changeRef", &change_ref),
            ("verificationRef", &verification_ref),
        ] {
            if public_ref(receipt, field).as_ref() != Some(expected) {
                return None;
            }
        }
        let authority_receipt_ref = public_ref(receipt, "authorityReceiptRef")?;
        let decision_ref = public_ref(receipt, "decisionRef")?;
        let authority_allowed = receipt.get("allowed").and_then(Value::as_bool)?;

        Some(Self {
            fields: vec![
                InspectorField::new("objective_ref", objective_ref.as_str()),
                InspectorField::new("turn_ref", turn_ref.as_str()),
                InspectorField::new("change_ref", change_ref.as_str()),
                InspectorField::new("project_generation", project_generation.as_str()),
                InspectorField::new("diff", diff_summary),
                InspectorField::new("test_command", test_command),
                InspectorField::new("test_outcome", test_outcome.as_str()),
                InspectorField::new("verification_ref", verification_ref.as_str()),
                InspectorField::new("host_executed", "true"),
                InspectorField::new("authority_receipt_ref", authority_receipt_ref.as_str()),
                InspectorField::new("decision_ref", decision_ref.as_str()),
                InspectorField::new(
                    "authority_allowed",
                    if authority_allowed { "true" } else { "false" },
                ),
            ],
        })
    }
}

fn public_ref(value: &Value, field: &str) -> Option<PublicRef> {
    PublicRef::new(value.get(field)?.as_str()?)
}

fn bounded_public_command(value: &str) -> Option<String> {
    bounded_text(value, 256)
}

fn bounded_public_text(value: &str) -> Option<String> {
    bounded_text(value, 512)
}

fn bounded_text(value: &str, maximum_length: usize) -> Option<String> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if value.is_empty()
        || value.len() > maximum_length
        || value.contains('\0')
        || value.contains("/Users/")
        || value.contains("/home/")
        || lower.contains("bearer ")
        || lower.contains("auth.json")
        || lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower.contains("private_key")
    {
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn records() -> (Value, Value) {
        (
            json!({
                "runRef":"run.fa.1",
                "evidence":{
                    "objectiveRef":"objective.fa.1",
                    "turnRef":"turn.fa.4",
                    "changeRef":"change.fa.4",
                    "projectGeneration":"project.generation.7",
                    "diffSummary":"2 files changed, 18 insertions, 3 deletions",
                    "testCommand":"cargo test -p full_auto_ui",
                    "testOutcome":"passed",
                    "verificationRef":"verification.host.fa.4",
                    "hostExecuted":true
                }
            }),
            json!({
                "runRef":"run.fa.1",
                "objectiveRef":"objective.fa.1",
                "turnRef":"turn.fa.4",
                "changeRef":"change.fa.4",
                "verificationRef":"verification.host.fa.4",
                "authorityReceiptRef":"receipt.authority.fa.4",
                "decisionRef":"decision.authority.fa.4",
                "allowed":true
            }),
        )
    }

    #[test]
    fn renders_one_linked_object_in_inspector_grammar() {
        let (report, receipt) = records();
        let view = FullAutoEvidenceView::from_records(&report, &receipt).unwrap();
        let labels: Vec<_> = view.fields.iter().map(|field| field.label).collect();
        assert_eq!(
            labels,
            [
                "objective_ref",
                "turn_ref",
                "change_ref",
                "project_generation",
                "diff",
                "test_command",
                "test_outcome",
                "verification_ref",
                "host_executed",
                "authority_receipt_ref",
                "decision_ref",
                "authority_allowed"
            ]
        );
    }

    #[test]
    fn mismatched_hops_or_self_reported_verification_fail_closed() {
        let (report, mut receipt) = records();
        receipt["changeRef"] = json!("change.other");
        assert!(FullAutoEvidenceView::from_records(&report, &receipt).is_none());

        let (mut report, receipt) = records();
        report["evidence"]["hostExecuted"] = json!(false);
        assert!(FullAutoEvidenceView::from_records(&report, &receipt).is_none());
    }

    #[test]
    fn secret_or_private_material_never_reaches_the_view() {
        let (mut report, receipt) = records();
        report["evidence"]["testCommand"] = json!("cat /Users/owner/.codex/auth.json");
        assert!(FullAutoEvidenceView::from_records(&report, &receipt).is_none());
    }
}
