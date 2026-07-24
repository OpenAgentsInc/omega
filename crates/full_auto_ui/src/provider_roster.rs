use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderAccountRow {
    pub account_ref: String,
    pub provider: String,
    pub label: String,
    pub readiness: String,
    pub quota: String,
    pub lane: String,
}

pub fn parse_provider_accounts(value: &Value) -> Vec<ProviderAccountRow> {
    value
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|account| {
            Some(ProviderAccountRow {
                account_ref: public_token(account.get("accountRef")?.as_str()?)?,
                provider: public_token(account.get("provider")?.as_str()?)?,
                label: public_label(account.get("label")?.as_str()?)?,
                readiness: public_token(account.get("state")?.as_str()?)?,
                quota: public_token(account.get("quotaState")?.as_str()?)?,
                lane: public_token(account.get("lane")?.as_str()?)?,
            })
        })
        .collect()
}

fn public_token(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".:_-".contains(character))
    {
        return None;
    }
    Some(value.to_string())
}

fn public_label(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 96
        || value.contains("Bearer ")
        || value.contains("/Users/")
        || value.contains("auth.json")
    {
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_multiple_accounts_and_lane_mapping() {
        let rows = parse_provider_accounts(&json!({
            "accounts": [
                {"accountRef":"account.codex.1","provider":"openai","label":"ChatGPT Personal","state":"ready","quotaState":"available","lane":"codex-local"},
                {"accountRef":"account.codex.2","provider":"openai","label":"ChatGPT Work","state":"busy","quotaState":"cooling","lane":"codex-local-2"},
                {"accountRef":"account.claude.1","provider":"anthropic","label":"Claude","state":"ready","quotaState":"available","lane":"claude-local"}
            ]
        }));

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].account_ref, "account.codex.2");
        assert_eq!(rows[1].lane, "codex-local-2");
        assert_eq!(rows[1].quota, "cooling");
    }

    #[test]
    fn never_projects_credentials_or_private_paths() {
        let rows = parse_provider_accounts(&json!({
            "accounts": [
                {"accountRef":"account.codex.1","provider":"openai","label":"Bearer secret","state":"ready","quotaState":"available","lane":"codex-local"},
                {"accountRef":"account.codex.2","provider":"openai","label":"/Users/owner/.codex/auth.json","state":"ready","quotaState":"available","lane":"codex-local-2"}
            ]
        }));
        assert!(rows.is_empty());
    }

    #[test]
    fn a_lane_is_never_invented_as_an_account() {
        let rows = parse_provider_accounts(&json!({
            "lanes": [{"lane":"codex-local","state":"available","activeRuns":0}]
        }));
        assert!(rows.is_empty());
    }
}
