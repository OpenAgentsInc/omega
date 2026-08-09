use anyhow::Result;
use gpui::SharedString;
use handlebars::Handlebars;
use rust_embed::RustEmbed;
use serde::Serialize;
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "src/templates"]
#[include = "*.hbs"]
struct Assets;

pub struct Templates(Handlebars<'static>);

impl Templates {
    pub fn new() -> Arc<Self> {
        let mut handlebars = Handlebars::new();
        handlebars.set_strict_mode(true);
        handlebars.register_helper("contains", Box::new(contains));
        handlebars.register_embed_templates::<Assets>().unwrap();
        Arc::new(Self(handlebars))
    }
}

pub trait Template: Sized {
    const TEMPLATE_NAME: &'static str;

    fn render(&self, templates: &Templates) -> Result<String>
    where
        Self: Serialize + Sized,
    {
        Ok(templates.0.render(Self::TEMPLATE_NAME, self)?)
    }
}

#[derive(Serialize)]
pub struct SystemPromptTemplate<'a> {
    #[serde(flatten)]
    pub project: &'a prompt_store::ProjectContext,
    pub available_tools: Vec<SharedString>,
    pub available_executors: Vec<crate::InstalledAgent>,
    pub model_name: Option<String>,
    pub date: String,
    /// Contents of the user-global `~/.config/zed/AGENTS.md` file (or the
    /// platform equivalent), if present and non-empty.
    pub user_agents_md: Option<SharedString>,
    /// Whether agent-run terminal commands are wrapped in an OS-level
    /// sandbox for this thread. When `true`, the rendered prompt
    /// describes the sandbox's read/write/network rules and the
    /// per-command flags the model can request to relax them. When
    /// `false`, the prompt omits the sandbox section entirely.
    pub sandboxing: bool,
    /// Whether the host is Linux. The writable-temp story differs by
    /// platform (Linux exposes an ephemeral `tmpfs` over `/tmp`; other
    /// platforms provide a persistent per-thread `$TMPDIR`), so the sandbox
    /// section describes the right one rather than advertising a `$TMPDIR`
    /// that doesn't behave as stated.
    pub is_linux: bool,
    /// Whether sandboxed terminal commands run through WSL on Windows.
    pub is_windows: bool,
}

impl Template for SystemPromptTemplate<'_> {
    const TEMPLATE_NAME: &'static str = "system_prompt.hbs";
}

impl SystemPromptTemplate<'_> {
    pub fn render_basic(&self, templates: &Templates) -> Result<String> {
        Ok(templates.0.render("basic_system_prompt.hbs", self)?)
    }
}

/// Handlebars helper for checking if an item is in a list
fn contains(
    h: &handlebars::Helper,
    _: &handlebars::Handlebars,
    _: &handlebars::Context,
    _: &mut handlebars::RenderContext,
    out: &mut dyn handlebars::Output,
) -> handlebars::HelperResult {
    let list = h
        .param(0)
        .and_then(|v| v.value().as_array())
        .ok_or_else(|| {
            handlebars::RenderError::new("contains: missing or invalid list parameter")
        })?;
    let query = h.param(1).map(|v| v.value()).ok_or_else(|| {
        handlebars::RenderError::new("contains: missing or invalid query parameter")
    })?;

    if list.contains(query) {
        out.write("true")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompt_template() {
        let project = prompt_store::ProjectContext::default();
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec!["echo".into(), "spawn_agent".into()],
            available_executors: vec![
                crate::InstalledAgent::new("grok", "Grok"),
                crate::InstalledAgent::new("scv", "SCV"),
            ],
            model_name: Some("test-model".to_string()),
            date: "2026-01-01".to_string(),
            user_agents_md: None,
            sandboxing: false,
            is_linux: false,
            is_windows: false,
        };
        let templates = Templates::new();
        let rendered = template.render(&templates).unwrap();
        assert!(rendered.contains("You are the Omega coding agent"));
        assert!(rendered.contains("Today's Date: 2026-01-01"));
        assert!(rendered.contains("## Fixing Diagnostics"));
        assert!(rendered.contains("test-model"));
        assert!(rendered.contains("`grok` (Grok)"));
        assert!(rendered.contains("`scv` (SCV)"));
    }

    #[test]
    fn test_basic_system_prompt_is_measured() {
        const BASIC_SYSTEM_PROMPT_BYTE_CEILING: usize = 8_192;

        for sandboxing in [false, true] {
            let project = prompt_store::ProjectContext::default();
            let template = SystemPromptTemplate {
                project: &project,
                available_tools: vec![
                    "read".into(),
                    "write".into(),
                    "edit".into(),
                    "bash".into(),
                    "delegate".into(),
                ],
                available_executors: vec![
                    crate::InstalledAgent::new("codex-acp", "Codex"),
                    crate::InstalledAgent::new("claude-acp", "Claude"),
                    crate::InstalledAgent::new("grok", "Grok"),
                    crate::InstalledAgent::new("scv", "SCV"),
                ],
                model_name: Some("google/gemini-3.6-flash".to_string()),
                date: "2026-07-27".to_string(),
                user_agents_md: None,
                sandboxing,
                is_linux: false,
                is_windows: false,
            };
            let rendered = template.render_basic(&Templates::new()).unwrap();

            assert!(
                rendered.len() <= BASIC_SYSTEM_PROMPT_BYTE_CEILING,
                "basic prompt rendered to {} bytes with sandboxing={sandboxing}, exceeding the {}-byte ceiling",
                rendered.len(),
                BASIC_SYSTEM_PROMPT_BYTE_CEILING,
            );
            assert!(!rendered.contains("mermaid"));
            assert!(!rendered.contains("find_path"));
            assert!(!rendered.contains("language server"));
            assert!(!rendered.contains("## Skills"));
            assert!(!rendered.contains("## Instruction Files"));
        }
    }

    #[test]
    fn test_basic_system_prompt_keeps_required_section_order() {
        let project = prompt_store::ProjectContext::default();
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec![
                "read".into(),
                "write".into(),
                "edit".into(),
                "bash".into(),
                "delegate".into(),
            ],
            available_executors: vec![
                crate::InstalledAgent::new("codex-acp", "Codex"),
                crate::InstalledAgent::new("claude-acp", "Claude"),
                crate::InstalledAgent::new("grok", "Grok"),
                crate::InstalledAgent::new("scv", "SCV"),
            ],
            model_name: None,
            date: "2026-07-27".to_string(),
            user_agents_md: Some("personal".into()),
            sandboxing: true,
            is_linux: false,
            is_windows: false,
        };
        let rendered = template.render_basic(&Templates::new()).unwrap();
        let headings = [
            "## Communication",
            "## Tool Use",
            "## Work Safety",
            "## Task Execution",
            "## Delegation",
            "## System Information",
            "## Bash Sandbox",
            "## Instruction Files",
        ];
        let mut prior = 0;
        for heading in headings {
            let position = rendered
                .find(heading)
                .expect("required heading should render");
            assert!(position >= prior, "{heading} rendered out of order");
            prior = position;
        }
        assert!(rendered.contains(
            "never use `git checkout`, `git restore`, or `git stash` as an undo mechanism"
        ));
        assert!(rendered.contains("Never delegate when no executor exists"));
        assert!(
            rendered.contains("Never use `delegate` only to read a file, skill, or instruction")
        );
        assert!(rendered.contains("`codex-acp` (Codex)"));
        assert!(rendered.contains("`claude-acp` (Claude)"));
        assert!(rendered.contains("`grok` (Grok)"));
        assert!(rendered.contains("`scv` (SCV)"));
        assert!(rendered.contains("Your identity is Omega"));
        assert!(!rendered.contains("Model:"));
    }

    #[test]
    fn test_system_prompt_renders_user_agents_md_before_project_rules() {
        use prompt_store::{ProjectContext, RulesFileContext, WorktreeContext};
        use util::rel_path::RelPath;

        let worktrees = vec![WorktreeContext {
            root_name: "my-project".to_string(),
            abs_path: std::path::Path::new("/tmp/my-project").into(),
            rules_file: Some(RulesFileContext {
                path_in_worktree: RelPath::from_unix_str("AGENTS.md").unwrap().into(),
                text: "project-specific guidance".to_string(),
                project_entry_id: 1,
            }),
        }];
        let project = ProjectContext::new(worktrees);
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec!["echo".into()],
            available_executors: Vec::new(),
            model_name: Some("test-model".to_string()),
            date: "2026-01-01".to_string(),
            user_agents_md: Some("always be concise".into()),
            sandboxing: false,
            is_linux: false,
            is_windows: false,
        };
        let templates = Templates::new();
        let rendered = template.render(&templates).unwrap();

        assert!(rendered.contains("### Personal `AGENTS.md`"));
        assert!(rendered.contains("always be concise"));
        assert!(rendered.contains("### Project Rules"));
        assert!(rendered.contains("project-specific guidance"));

        let personal_idx = rendered.find("### Personal `AGENTS.md`").unwrap();
        let project_idx = rendered.find("### Project Rules").unwrap();
        assert!(
            personal_idx < project_idx,
            "personal AGENTS.md should render before project rules so project rules can override it"
        );
    }

    #[test]
    fn test_system_prompt_omits_sandbox_section_when_sandboxing_disabled() {
        let project = prompt_store::ProjectContext::default();
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec!["echo".into()],
            available_executors: Vec::new(),
            model_name: Some("test-model".to_string()),
            date: "2026-01-01".to_string(),
            user_agents_md: None,
            sandboxing: false,
            is_linux: false,
            is_windows: false,
        };
        let templates = Templates::new();
        let rendered = template.render(&templates).unwrap();
        assert!(!rendered.contains("## Terminal sandbox"));
        assert!(!rendered.contains("allow_hosts"));
    }

    #[test]
    fn test_system_prompt_renders_sandbox_section_with_worktrees_when_enabled() {
        use prompt_store::{ProjectContext, WorktreeContext};

        let worktrees = vec![
            WorktreeContext {
                root_name: "alpha".to_string(),
                abs_path: std::path::Path::new("/tmp/alpha").into(),
                rules_file: None,
            },
            WorktreeContext {
                root_name: "beta".to_string(),
                abs_path: std::path::Path::new("/tmp/beta").into(),
                rules_file: None,
            },
        ];
        let project = ProjectContext::new(worktrees);
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec!["echo".into()],
            available_executors: Vec::new(),
            model_name: Some("test-model".to_string()),
            date: "2026-01-01".to_string(),
            user_agents_md: None,
            sandboxing: true,
            is_linux: false,
            is_windows: false,
        };
        let templates = Templates::new();
        let rendered = template.render(&templates).unwrap();

        assert!(rendered.contains("## Terminal sandbox"));
        assert!(rendered.contains("`/tmp/alpha`"));
        assert!(rendered.contains("`/tmp/beta`"));
        assert!(rendered.contains("allow_hosts"));
        assert!(rendered.contains("allow_all_hosts: true"));
        assert!(rendered.contains("fs_write_paths"));
        assert!(rendered.contains("allow_fs_write_all: true"));
        assert!(rendered.contains("unsandboxed: true"));
        assert!(rendered.contains("`.git` directories remain protected"));
        assert!(rendered.contains("Git metadata writes are never grantable inside the sandbox"));
        assert!(rendered.contains("request `unsandboxed: true` with a reason"));
        assert!(rendered.contains("git --no-optional-locks status"));
        assert!(rendered.contains("for the rest of the thread"));
    }

    #[test]
    fn test_system_prompt_linux_sandbox_section_omits_tmpdir() {
        use prompt_store::{ProjectContext, WorktreeContext};

        let worktrees = vec![WorktreeContext {
            root_name: "alpha".to_string(),
            abs_path: std::path::Path::new("/tmp/alpha").into(),
            rules_file: None,
        }];
        let project = ProjectContext::new(worktrees);
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec!["echo".into()],
            available_executors: Vec::new(),
            model_name: Some("test-model".to_string()),
            date: "2026-01-01".to_string(),
            user_agents_md: None,
            sandboxing: true,
            is_linux: true,
            is_windows: false,
        };
        let templates = Templates::new();
        let rendered = template.render(&templates).unwrap();

        assert!(rendered.contains("## Terminal sandbox"));
        // On Linux we must not advertise the special persistent `$TMPDIR`.
        assert!(!rendered.contains("$TMPDIR"));
        assert!(rendered.contains("`/tmp` is writable"));
        assert!(rendered.contains("`/tmp/alpha`"));
    }

    #[test]
    fn test_system_prompt_windows_sandbox_section_rejects_host_specific_network() {
        use prompt_store::{ProjectContext, WorktreeContext};

        let worktrees = vec![WorktreeContext {
            root_name: "alpha".to_string(),
            abs_path: std::path::Path::new("C:/Users/me/project").into(),
            rules_file: None,
        }];
        let project = ProjectContext::new(worktrees);
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec!["echo".into()],
            available_executors: Vec::new(),
            model_name: Some("test-model".to_string()),
            date: "2026-01-01".to_string(),
            user_agents_md: None,
            sandboxing: true,
            is_linux: false,
            is_windows: true,
        };
        let templates = Templates::new();
        let rendered = template.render(&templates).unwrap();

        assert!(rendered.contains("commands run inside WSL under Bubblewrap"));
        assert!(rendered.contains("Protected Git metadata remains read-only"));
        assert!(rendered.contains("do not use this on Windows"));
        assert!(rendered.contains("such requests are rejected"));
        assert!(rendered.contains("allow_all_hosts: true"));
        assert!(rendered.contains("git --no-optional-locks status"));
    }

    #[test]
    fn test_system_prompt_sandbox_section_handles_zero_worktrees() {
        let project = prompt_store::ProjectContext::default();
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec!["echo".into()],
            available_executors: Vec::new(),
            model_name: Some("test-model".to_string()),
            date: "2026-01-01".to_string(),
            user_agents_md: None,
            sandboxing: true,
            is_linux: false,
            is_windows: false,
        };
        let templates = Templates::new();
        let rendered = template.render(&templates).unwrap();

        assert!(rendered.contains("## Terminal sandbox"));
        assert!(rendered.contains("No project directories are currently writable"));
    }

    #[test]
    fn test_system_prompt_omits_user_agents_md_section_when_absent() {
        let project = prompt_store::ProjectContext::default();
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec!["echo".into()],
            available_executors: Vec::new(),
            model_name: Some("test-model".to_string()),
            date: "2026-01-01".to_string(),
            user_agents_md: None,
            sandboxing: false,
            is_linux: false,
            is_windows: false,
        };
        let templates = Templates::new();
        let rendered = template.render(&templates).unwrap();
        assert!(!rendered.contains("### Personal `AGENTS.md`"));
    }

    #[test]
    fn test_system_prompt_does_not_render_legacy_zed_rules_section() {
        let project = prompt_store::ProjectContext::default();
        let template = SystemPromptTemplate {
            project: &project,
            available_tools: vec!["echo".into()],
            available_executors: Vec::new(),
            model_name: Some("test-model".to_string()),
            date: "2026-01-01".to_string(),
            user_agents_md: None,
            sandboxing: false,
            is_linux: false,
            is_windows: false,
        };
        let templates = Templates::new();
        let rendered = template.render(&templates).unwrap();

        assert!(!rendered.contains("The user has specified the following rules"));
        assert!(!rendered.contains("Rules title:"));
    }
}
