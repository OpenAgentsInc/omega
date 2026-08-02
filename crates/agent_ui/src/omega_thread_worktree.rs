//! `OMEGA-DELTA-0214` — isolate, don't ask.
//!
//! Two write-capable agents in one checkout overwrite each other. That is
//! real, and `OMEGA-DELTA-0181` was right to refuse to let it happen silently.
//! What it got wrong was the mechanism: it stopped the person with a modal
//! titled *"Another agent is already using this worktree"*, four lines of
//! explanation, and a choice between **Cancel** and **Run here anyway** — at
//! the exact moment they had just pressed New Thread and wanted to type.
//!
//! The owner's verdict was *"i never want to see that shit. figure out better
//! workflow."*
//!
//! So Omega resolves the collision instead of narrating it. When a new
//! thread's root is already occupied by a live agent, Omega provisions a
//! linked git worktree for the new thread and runs there. That satisfies the
//! original safety property *more* strongly than the modal did: the modal only
//! disclosed the hazard and then let a person walk into it, while isolation
//! makes the hazard impossible. The person who genuinely wants concurrent
//! writes in one tree says so once, in `agent.thread_worktree: "shared"`,
//! rather than once per turn.
//!
//! A worktree provisioned for a draft the person then abandons is not
//! reclaimed. Neither is one belonging to an archived thread: the reclamation
//! machinery in `thread_worktree_archive.rs` exists and has never been wired
//! to anything. Both are the same missing reaper, recorded in
//! `OMEGA-DELTA-0214` and omega#155, and both are bounded by the fact that
//! provisioning only happens when a root is genuinely occupied.
//!
//! Provisioning happens when the thread is created, not when it sends. It has
//! to: `ConversationView` hands the working directories to `new_session`, and
//! every external ACP agent reports `supports_live_work_dir_updates() ==
//! false`, so a session that has already started cannot be moved. The thread
//! is created immediately and its session load waits on the provisioning task
//! (`ConversationView::set_pending_work_dirs`), so nothing about the new-thread
//! gesture blocks on git.

use std::path::PathBuf;

use agent_settings::AgentSettings;
use anyhow::{Context as _, Result};
use gpui::{App, Entity, Task};
use project::Project;
use settings::{Settings as _, ThreadWorktreeMode};
use workspace::PathList;

use crate::omega_agent_supervision::{AgentSupervision, WorktreeCollision};

/// The longest slug taken from a thread title. Long enough to recognize the
/// thread in `../worktrees/<name>/`, short enough to keep the path sane.
const MAX_TITLE_SLUG_LEN: usize = 32;

/// What a new thread should do about the roots it was opened against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadWorktreeResolution {
    /// Run in the requested roots: nothing live occupies them, or the person
    /// recorded `shared`, or there is nothing to isolate into.
    RunHere,
    /// A live agent occupies the roots. Provision an isolated worktree.
    Isolate(Box<WorktreeCollision>),
}

impl ThreadWorktreeResolution {
    pub fn collision(&self) -> Option<&WorktreeCollision> {
        match self {
            Self::RunHere => None,
            Self::Isolate(collision) => Some(collision),
        }
    }
}

pub fn mode(cx: &App) -> ThreadWorktreeMode {
    AgentSettings::get_global(cx).thread_worktree
}

/// Decides what a thread should do about `work_dirs`.
///
/// `OMEGA-DELTA-0214`. This never prompts and never fails: an undecidable case
/// resolves to [`ThreadWorktreeResolution::RunHere`], which is exactly the
/// behavior Omega had before isolation existed.
pub fn resolve(
    thread_key: &str,
    work_dirs: impl IntoIterator<Item = PathBuf>,
    project: Option<&Entity<Project>>,
    cx: &mut App,
) -> ThreadWorktreeResolution {
    if mode(cx) == ThreadWorktreeMode::Shared {
        // The person already made the explicit decision `OMEGA-DELTA-0181`
        // asked for. Making it again per turn is the defect.
        return ThreadWorktreeResolution::RunHere;
    }
    let remote_connection =
        project.and_then(|project| project.read(cx).remote_connection_options(cx));
    match AgentSupervision::global(cx).occupant_for(
        thread_key,
        work_dirs,
        remote_connection.as_ref(),
    ) {
        Some(collision) => ThreadWorktreeResolution::Isolate(Box::new(collision)),
        None => ThreadWorktreeResolution::RunHere,
    }
}

/// True when this project can actually be given a linked worktree.
///
/// A collab project or a project with no git repository has nowhere to
/// isolate into, so the honest answer there is disclosure, not a failed
/// `git worktree add`.
pub fn can_isolate(project: &Entity<Project>, cx: &App) -> bool {
    let project = project.read(cx);
    !project.is_via_collab() && !project.repositories(cx).is_empty()
}

/// A directory name for a thread's isolated worktree.
///
/// A real thread title becomes a slug so `../worktrees/` stays readable.
/// `None` — a thread still using a default placeholder, or a title with nothing
/// sluggable in it — hands the choice to `generate_worktree_name`, which picks
/// an adjective-noun pair that no existing worktree uses. Omega never asks a
/// person to name a worktree it decided to create.
pub fn worktree_name_for_thread(title: Option<&str>) -> Option<String> {
    let title = title?.trim();
    if title.is_empty()
        || title == crate::DEFAULT_THREAD_TITLE
        || title == crate::LEGACY_DEFAULT_THREAD_TITLE
    {
        return None;
    }

    let mut slug = String::with_capacity(MAX_TITLE_SLUG_LEN);
    let mut pending_separator = false;
    for character in title.chars() {
        if slug.len() >= MAX_TITLE_SLUG_LEN {
            break;
        }
        if character.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('-');
            }
            pending_separator = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            pending_separator = true;
        }
    }

    // A purely numeric name — a date, a sha, an issue number — reads as a git
    // ref and tells a person nothing, so it is not better than the generator.
    if slug.is_empty()
        || slug
            .chars()
            .all(|character| character.is_ascii_digit() || character == '-')
    {
        return None;
    }
    Some(slug)
}

/// Creates a linked git worktree for this project and returns the roots the
/// thread should run in.
///
/// This deliberately uses the worktree primitives directly rather than
/// `create_worktree_workspace`, because that path always ends in
/// `open_worktree_workspace` and adds a workspace tab. Isolation is a quiet
/// correction, not a navigation.
pub fn provision(
    project: Entity<Project>,
    name_hint: Option<String>,
    cx: &mut App,
) -> Task<Result<PathList>> {
    if !can_isolate(&project, cx) {
        return Task::ready(Err(anyhow::anyhow!(
            "this project has no git repository to isolate into"
        )));
    }
    let (git_repos, _non_git_paths) =
        git_ui::worktree_service::classify_worktrees(project.read(cx), cx);
    let remote_connection_options = project.read(cx).remote_connection_options(cx);

    cx.spawn(async move |cx| {
        let was_hinted = name_hint.is_some();
        let paths = git_ui::worktree_service::create_linked_worktrees(
            git_repos.clone(),
            name_hint,
            None,
            remote_connection_options.clone(),
            cx,
        )
        .await;
        // A hinted name can collide with a worktree that already exists, and a
        // failed creation is rolled back. The generator is defined to avoid
        // existing names, so one retry without the hint is the difference
        // between isolating and giving up. Retrying an already-unhinted
        // attempt would only repeat the same failure.
        let paths = match paths {
            Ok(paths) => paths,
            Err(error) if was_hinted => {
                log::warn!("named worktree isolation failed, retrying unnamed: {error:#}");
                git_ui::worktree_service::create_linked_worktrees(
                    git_repos,
                    None,
                    None,
                    remote_connection_options,
                    cx,
                )
                .await
                .context("could not provision an isolated worktree")?
            }
            Err(error) => return Err(error.context("could not provision an isolated worktree")),
        };
        anyhow::ensure!(
            !paths.is_empty(),
            "worktree isolation produced no working directory"
        );
        Ok(PathList::new(&paths))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_title_becomes_a_readable_slug() {
        assert_eq!(
            worktree_name_for_thread(Some("Fix the release gate")).as_deref(),
            Some("fix-the-release-gate")
        );
        assert_eq!(
            worktree_name_for_thread(Some("  Rewrite  request_worktree_admission!  ")).as_deref(),
            Some("rewrite-request-worktree-admissi")
        );
        assert_eq!(
            worktree_name_for_thread(Some("Épée & sabre")).as_deref(),
            Some("p-e-sabre")
        );
    }

    /// `OMEGA-DELTA-0214`. The generator, not a prompt, is the fallback — a
    /// brand-new thread has no title yet and must never be asked for one.
    #[test]
    fn an_unnamed_thread_defers_to_the_generator() {
        assert_eq!(worktree_name_for_thread(None), None);
        assert_eq!(worktree_name_for_thread(Some("")), None);
        assert_eq!(worktree_name_for_thread(Some("   ")), None);
        assert_eq!(
            worktree_name_for_thread(Some(crate::DEFAULT_THREAD_TITLE)),
            None
        );
        assert_eq!(worktree_name_for_thread(Some("!!!")), None);
        assert_eq!(worktree_name_for_thread(Some("2026-07-31")), None);
    }
}
