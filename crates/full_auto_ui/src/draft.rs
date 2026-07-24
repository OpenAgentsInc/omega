//! Pure launcher draft validation (shared by GPUI and tests).

pub const FULL_AUTO_ACTIVE_LIMIT: usize = 8;
pub const FULL_AUTO_WORKSPACE_REF: &str = "workspace.omega.supervised";
pub const DEFAULT_TURN_CAP: u32 = 40;
pub const DEFAULT_DONE_CONDITION: &str =
    "The outcome works in the real system and the named verification passes.";

#[derive(Debug, Clone)]
pub struct FullAutoLauncherDraft {
    pub title: String,
    pub objective: String,
    pub done_condition: String,
    pub workspace_ref: String,
    pub lane: String,
    pub model: String,
    pub turn_cap_text: String,
    pub max_wall_clock_minutes_text: String,
    pub fallback_lanes: Vec<String>,
    pub submitting: bool,
    pub error: Option<String>,
    pub advanced_open: bool,
}

impl Default for FullAutoLauncherDraft {
    fn default() -> Self {
        Self {
            title: String::new(),
            objective: String::new(),
            done_condition: String::new(),
            workspace_ref: FULL_AUTO_WORKSPACE_REF.to_string(),
            lane: "codex-local".to_string(),
            model: String::new(),
            turn_cap_text: DEFAULT_TURN_CAP.to_string(),
            max_wall_clock_minutes_text: String::new(),
            fallback_lanes: Vec::new(),
            submitting: false,
            error: None,
            advanced_open: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherValidation {
    pub ok: bool,
    pub message: Option<String>,
    pub title: String,
    pub objective: String,
    pub done_condition: String,
    pub turn_cap: u32,
}

pub fn validate_launcher_draft(draft: &FullAutoLauncherDraft) -> LauncherValidation {
    let objective = draft.objective.trim().to_string();
    if objective.is_empty() {
        return LauncherValidation {
            ok: false,
            message: Some("Describe the outcome Full Auto should accomplish.".into()),
            title: String::new(),
            objective,
            done_condition: String::new(),
            turn_cap: DEFAULT_TURN_CAP,
        };
    }
    let done_condition = {
        let trimmed = draft.done_condition.trim();
        if trimmed.is_empty() {
            DEFAULT_DONE_CONDITION.to_string()
        } else {
            trimmed.to_string()
        }
    };
    let title = {
        let trimmed = draft.title.trim();
        if trimmed.is_empty() {
            objective
                .lines()
                .next()
                .unwrap_or("Full Auto run")
                .chars()
                .take(80)
                .collect()
        } else {
            trimmed.to_string()
        }
    };
    let turn_cap = draft
        .turn_cap_text
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0 && *value <= 1000)
        .unwrap_or(DEFAULT_TURN_CAP);

    LauncherValidation {
        ok: true,
        message: None,
        title,
        objective,
        done_condition,
        turn_cap,
    }
}
