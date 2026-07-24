//! Public-safe reference validation and redaction for the receipt inspector.
//!
//! Only bounded, opaque reference strings may reach the interface or a log.
//! Raw tokens, credentials, private filesystem paths, and unbounded prose are
//! rejected and never echoed.

use serde::{Deserialize, Serialize};

/// Maximum length for any public-safe reference shown in the inspector.
pub const PUBLIC_REF_MAX_LEN: usize = 256;

/// A reference that passed public-safety checks.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicRef(String);

impl PublicRef {
    /// Validate and wrap a candidate reference. Returns `None` when unsafe.
    pub fn new(raw: impl AsRef<str>) -> Option<Self> {
        sanitize_public_ref(raw.as_ref())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for PublicRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for PublicRef {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Return true when `raw` is a bounded public-safe reference.
pub fn is_public_safe_ref(raw: &str) -> bool {
    sanitize_public_ref(raw).is_some()
}

/// Sanitize a candidate reference.
///
/// Accepts the same character class as OpenAgents public ref segments:
/// ASCII letters, digits, `.`, `_`, `:`, and `-`, length 1..=256.
/// Rejects tokens, private paths, whitespace, and other unsafe material.
pub fn sanitize_public_ref(raw: &str) -> Option<PublicRef> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > PUBLIC_REF_MAX_LEN {
        return None;
    }
    if looks_like_private_path(trimmed) {
        return None;
    }
    if looks_like_secret_or_token(trimmed) {
        return None;
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c == '\0') {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
    {
        return None;
    }
    // Unbounded prose often lacks separators; public refs use dotted segments.
    // Allow short identifiers without dots (tool names, reason classes) up to
    // a modest length; longer undotted strings are treated as opaque dumps.
    if !trimmed.contains('.') && !trimmed.contains(':') && !trimmed.contains('_') && trimmed.len() > 64
    {
        return None;
    }
    Some(PublicRef(trimmed.to_string()))
}

fn looks_like_private_path(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("\\\\")
        || (value.len() > 2
            && value.as_bytes()[1] == b':'
            && (value.as_bytes()[2] == b'\\' || value.as_bytes()[2] == b'/'))
        || lower.contains("/users/")
        || lower.contains("/home/")
        || lower.contains("/private/")
        || lower.contains("\\users\\")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.contains("id_rsa")
        || lower.contains(".ssh/")
}

fn looks_like_secret_or_token(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.contains("bearer ")
        || lower.contains("authorization:")
        || lower.starts_with("sk-")
        || lower.starts_with("sk_")
        || lower.starts_with("ghp_")
        || lower.starts_with("gho_")
        || lower.starts_with("github_pat_")
        || lower.starts_with("xox")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower.contains("openagents_agent_token")
        || lower.contains("client_secret")
        || lower.contains("private_key")
    {
        return true;
    }
    // Long base64-ish blobs without ref structure are tokens, not refs.
    if value.len() >= 40
        && !value.contains('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_')
    {
        // Still allow dotted public refs; this branch is undotted/opaque.
        if !value.contains(':') {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_dotted_public_refs() {
        assert!(is_public_safe_ref("tool.sarah.list_full_auto_runs"));
        assert!(is_public_safe_ref(
            "receipt.authority.sarah.tool.abcdef.list_full_auto_runs.turn.1"
        ));
        assert!(is_public_safe_ref("decision.sarah.tool.call-1"));
        assert!(is_public_safe_ref("openagents.sarah-owner-orchestrator"));
        assert!(is_public_safe_ref("blocker.sarah.authority_refused"));
        assert!(is_public_safe_ref("reserved_action"));
    }

    #[test]
    fn rejects_private_paths() {
        assert!(!is_public_safe_ref("/Users/christopherdavid/.codex/auth.json"));
        assert!(!is_public_safe_ref("~/work/openagents/secrets"));
        assert!(!is_public_safe_ref("C:\\Users\\owner\\token.txt"));
        assert!(!is_public_safe_ref("/home/owner/.ssh/id_rsa"));
    }

    #[test]
    fn rejects_tokens_and_secrets() {
        assert!(!is_public_safe_ref("sk-ant-api03-WOOOOSECRETVALUEHERE"));
        assert!(!is_public_safe_ref(
            "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig"
        ));
        assert!(!is_public_safe_ref("OPENAGENTS_AGENT_TOKEN=abc123"));
        assert!(!is_public_safe_ref(
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789"
        ));
        assert!(!is_public_safe_ref(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
    }

    #[test]
    fn rejects_unbounded_output() {
        assert!(!is_public_safe_ref(
            "tool said: here is a long answer with spaces and prose"
        ));
        assert!(!is_public_safe_ref("line1\nline2\nline3"));
        assert!(!is_public_safe_ref(""));
        let too_long = "a".repeat(PUBLIC_REF_MAX_LEN + 1);
        assert!(!is_public_safe_ref(&too_long));
    }

    #[test]
    fn never_echoes_unsafe_input() {
        let raw = "sk-secret-token-value-that-must-not-leak";
        assert!(sanitize_public_ref(raw).is_none());
    }
}
