pub mod generated {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/all-work-contract/generated/rust/all_work_v1.rs"
    ));
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use serde::de::DeserializeOwned;
    use sha2::{Digest, Sha256};

    use super::generated::{
        ContractValidate, WorkIndexReadRequest, WorkIndexSubscriptionEvent,
        WorkIndexSubscriptionRequest, WorkReadRequestFrame, WorkSnapshot, WorkSummary,
    };

    fn artifact_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("all-work-contract")
    }

    fn bytes(path: &str) -> Vec<u8> {
        fs::read(artifact_root().join(path)).expect("vendored All Work artifact must exist")
    }

    fn decode<T: DeserializeOwned + ContractValidate>(path: &str) -> Result<T, String> {
        let value: T = serde_json::from_slice(&bytes(path)).map_err(|error| error.to_string())?;
        value.validate().map_err(|error| error.to_string())?;
        Ok(value)
    }

    #[test]
    fn vendored_artifact_matches_immutable_source_receipt() {
        let compatibility: serde_json::Value =
            serde_json::from_slice(&bytes("generated/compatibility.json"))
                .expect("compatibility manifest");
        let source: serde_json::Value =
            serde_json::from_slice(&bytes("SOURCE.json")).expect("source receipt");
        let rust_digest = format!(
            "{:x}",
            Sha256::digest(bytes("generated/rust/all_work_v1.rs"))
        );
        assert_eq!(
            compatibility["artifacts"]["rustSha256"].as_str(),
            Some(rust_digest.as_str())
        );
        assert_eq!(source["rustSha256"].as_str(), Some(rust_digest.as_str()));
        assert_eq!(
            source["definitionSha256"].as_str(),
            compatibility["definitionSha256"].as_str()
        );
        assert_eq!(
            source["sourceCommit"].as_str(),
            Some("41ff4cb5327aac61e3f366dead7e508bdbe89340")
        );
    }

    #[test]
    fn omega_accepts_and_rejects_the_shared_cross_language_fixtures() {
        decode::<WorkSummary>("fixtures/valid/work-summary.json").expect("valid Work summary");
        decode::<WorkSnapshot>("fixtures/valid/work-snapshot.json").expect("valid Work snapshot");
        decode::<WorkIndexReadRequest>("fixtures/valid/work-index-request-absent.json")
            .expect("valid absent cursor");
        decode::<WorkIndexReadRequest>("fixtures/valid/work-index-request-null.json")
            .expect("valid null cursor");
        decode::<WorkIndexSubscriptionRequest>(
            "fixtures/valid/work-index-subscription-request.json",
        )
        .expect("valid subscription request");
        decode::<WorkIndexSubscriptionEvent>("fixtures/valid/work-index-subscription-gap.json")
            .expect("valid subscription event");
        decode::<WorkReadRequestFrame>("fixtures/valid/request-v2-index.json")
            .expect("valid v2 request");
        decode::<WorkReadRequestFrame>("fixtures/valid/request-v1-negotiate.json")
            .expect("valid explicit v1 negotiation");

        for path in [
            "fixtures/invalid/work-summary-unknown-field.json",
            "fixtures/invalid/work-summary-unsafe-integer.json",
            "fixtures/invalid/work-summary-bad-ref.json",
            "fixtures/invalid/work-summary-unknown-state.json",
            "fixtures/invalid/work-summary-missing-required-nullable.json",
        ] {
            assert!(decode::<WorkSummary>(path).is_err(), "accepted {path}");
        }
        assert!(
            decode::<WorkReadRequestFrame>("fixtures/invalid/request-unknown-method.json").is_err()
        );
    }
}
