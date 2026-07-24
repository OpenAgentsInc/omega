use chrono::{DateTime, Utc};
use serde_json::Value;

const ACTIVE_STATES: &[&str] = &["running", "pausing", "paused", "retrying", "stalled"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkroomFullAutoRun {
    pub run_ref: String,
    pub objective: String,
    pub lane: String,
    pub state: String,
    pub exact_unattended_duration: String,
    pub terminal_reason: Option<String>,
    pub latest_turn: Option<String>,
}

impl WorkroomFullAutoRun {
    pub fn from_value(value: &Value, now: DateTime<Utc>) -> Option<Self> {
        let run_ref = public_text(value, "runRef")?;
        let objective = public_text(value, "objective")?;
        let state = public_text(value, "state")?;
        let lane = public_text(value, "lane").unwrap_or_else(|| "unassigned".into());
        let started_at = value
            .get("startedAt")
            .or_else(|| value.get("createdAt"))
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let exact_unattended_duration = started_at
            .map(|started_at| format_duration((now - started_at).num_seconds().max(0)))
            .unwrap_or_else(|| "unavailable (record has no start time)".into());
        let terminal_reason = if ACTIVE_STATES.contains(&state.as_str()) {
            None
        } else {
            public_text(value, "terminalReason")
                .or_else(|| public_text(value, "stallCause"))
                .or_else(|| Some("record did not supply a terminal reason".into()))
        };
        let latest_turn = value
            .get("turns")
            .and_then(Value::as_array)
            .and_then(|turns| turns.last())
            .and_then(|turn| {
                let lane = public_text(turn, "lane")?;
                let summary = public_text(turn, "summary")?;
                Some(format!("{lane}: {summary}"))
            });

        Some(Self {
            run_ref,
            objective,
            lane,
            state,
            exact_unattended_duration,
            terminal_reason,
            latest_turn,
        })
    }
}

fn public_text(value: &Value, field: &str) -> Option<String> {
    let text = value.get(field)?.as_str()?.trim();
    if text.is_empty() || text.len() > 512 || text.contains("Bearer ") || text.contains("/Users/") {
        return None;
    }
    Some(text.to_string())
}

fn format_duration(total_seconds: i64) -> String {
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use serde_json::json;

    #[test]
    fn projects_exact_record_bound_duration_and_live_turn() {
        let now = Utc.with_ymd_and_hms(2026, 7, 24, 12, 1, 2).unwrap();
        let run = WorkroomFullAutoRun::from_value(
            &json!({
                "runRef": "run.fa.1",
                "objective": "Ship the verified change",
                "lane": "codex-local",
                "state": "running",
                "startedAt": "2026-07-24T10:00:00Z",
                "turns": [{"lane": "codex-local", "summary": "Tests are running"}],
            }),
            now,
        )
        .unwrap();

        assert_eq!(run.exact_unattended_duration, "02:01:02");
        assert_eq!(
            run.latest_turn.as_deref(),
            Some("codex-local: Tests are running")
        );
        assert_eq!(run.terminal_reason, None);
    }

    #[test]
    fn terminal_state_never_turns_silence_into_completion() {
        let run = WorkroomFullAutoRun::from_value(
            &json!({
                "runRef": "run.fa.2",
                "objective": "Verify",
                "lane": "claude-local",
                "state": "failed",
                "createdAt": "2026-07-24T10:00:00Z"
            }),
            Utc.with_ymd_and_hms(2026, 7, 24, 10, 0, 1).unwrap(),
        )
        .unwrap();

        assert_eq!(
            run.terminal_reason.as_deref(),
            Some("record did not supply a terminal reason")
        );
        assert_ne!(run.state, "completed");
    }

    #[test]
    fn drops_private_or_unbounded_projection_text() {
        let now = Utc::now();
        assert!(
            WorkroomFullAutoRun::from_value(
                &json!({"runRef":"run.1","objective":"/Users/owner/secret","state":"running"}),
                now,
            )
            .is_none()
        );
    }
}
