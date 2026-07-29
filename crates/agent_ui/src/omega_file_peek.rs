//! Opening files and generated text from the agent surface.
//!
//! `OMEGA-DELTA-0119`. The agent writes `crates/agent/src/templates/system_prompt.hbs`
//! into a message. The code-span resolver in [`crate::conversation_view`]
//! recognises it, resolves it against the project, and hands the markdown
//! renderer a `file://` URI — which is why the owner sees a blue underlined
//! link. Clicking it called `open_abs_path_at_point`, which opens an item in
//! the workspace's centre pane.
//!
//! `OMEGA-DELTA-0053` does not draw a centre pane once zero base is sealed. So
//! the click was never a no-op: it opened the file, moved focus into it, and
//! put it somewhere with no pixels. A person clicking a link and watching
//! nothing happen cannot tell that apart from an unimplemented handler, and the
//! second is the one they will report.
//!
//! `OMEGA-DELTA-0139` makes a transcript file link a deliberate two-way
//! interaction. A plain click reveals the workspace's ordinary centre pane and
//! opens the file there, so it is editable, saveable, and behaves like every
//! other editor tab. A secondary click (Command-click on macOS, Control-click
//! elsewhere) preserves the compact read-only modal from
//! `OMEGA-DELTA-0119`. Generated thread markdown and rules-menu entries remain
//! modal readers because they are separate commands, not transcript links.
//!
//! # There is no silent failure
//!
//! An agent can write a path that does not exist, or one relative to a root it
//! is not running in. When nothing resolves, the sheet still opens and says the
//! path as written and every directory it looked in. Refusing to draw would
//! reproduce the bug this delta exists to remove.
//!
//! # What `OMEGA-DELTA-0125` added
//!
//! The thread header's `…` menu had the same defect in three more places, and
//! the owner reported it the same way: *"literally nothing in this top right
//! menu does anything when i click on it."* **Open Thread as Markdown**,
//! **Open Project Rules (AGENTS.md)** and **Open Global Rules (AGENTS.md)** all
//! ended in the centre pane — the first through `add_item_to_active_pane`, the
//! other two through `open_abs_path`. So this module grew two more entry
//! points, [`open_file`] for a path the window already knows and [`open_text`]
//! for a thread rendered to markdown, which exists nowhere on disk.
//!
//! They are entry points into the *same* sheet rather than a second mechanism.
//! A menu that opened one kind of reader and a link that opened another would
//! be two surfaces to keep read-only, two to keep out of the composer's layout,
//! and two to fix the next time one of them silently stops drawing.

use std::path::{Path, PathBuf};

use editor::{Editor, SelectionEffects, scroll::Autoscroll};
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, MouseDownEvent,
    Render, Task, WeakEntity, Window, prelude::*, px,
};
use project::Project;
use rope::Point;
use ui::{Tooltip, prelude::*};
use util::ResultExt as _;
use util::paths::PathWithPosition;
use workspace::{ModalView, Workspace};

/// A line, and optionally a column and an end line, as a link wrote them.
///
/// One-based on both axes, because that is how a person reads `foo.rs:42` and
/// how `#L42:7` is written. The conversion to zero-based editor coordinates
/// happens once, at the point the selection is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeekAnchor {
    /// One-based line.
    pub line: u32,
    /// One-based column, when the link named one.
    pub column: Option<u32>,
    /// One-based last line, when the link named a range.
    pub end_line: Option<u32>,
}

/// A file link, parsed, with every path it could mean.
#[derive(Clone, Debug, PartialEq)]
pub struct PeekRequest {
    /// The path exactly as the link carried it. Failure prints this, never a
    /// rewritten or normalised form, so a person can compare it to what the
    /// agent wrote.
    pub written: SharedString,
    /// Absolute paths to try, in order. Empty is impossible: an absolute link
    /// contributes itself, and a relative one contributes nothing only when
    /// there are no roots at all, which the sheet reports as such.
    pub candidates: Vec<PathBuf>,
    /// The directories the relative candidates were formed from. Named in the
    /// failure sentence so "it looked in the wrong place" is visible rather
    /// than inferred.
    pub roots: Vec<PathBuf>,
    /// Where in the file to put the cursor, when the link said.
    pub anchor: Option<PeekAnchor>,
}

/// How a transcript file link should open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptFileOpenMode {
    /// Reveal the normal workspace editor and open a saveable file tab.
    EditablePane,
    /// Preserve the compact, read-only modal reader.
    ReadOnlyPeek,
}

/// Open a transcript file link, or decline the click.
///
/// Returns `true` when this took the click. Declining leaves
/// `thread_view::open_link` to do what it always did, which is what should
/// happen for a `http(s)` link, a thread link, and — deliberately — for every
/// link in a full editor, where opening the real editor is strictly better than
/// a read-only sheet.
///
/// The gate is [`omega_zero_base::is_sealed`] rather than `is_active`, because
/// sealing is the exact moment the centre pane stops being drawn. Before it,
/// `OMEGA-DELTA-0053` still renders the ordinary workspace for the identity
/// gate, and a link opened then lands somewhere a person can see.
pub fn open_from_transcript_link(
    url: &str,
    roots: &[PathBuf],
    mode: TranscriptFileOpenMode,
    workspace: &WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    if !omega_zero_base::is_sealed() {
        return false;
    }
    let Some(request) = parse_link(url, roots) else {
        return false;
    };
    let Some(workspace) = workspace.upgrade() else {
        return false;
    };

    workspace.update(cx, |workspace, cx| match mode {
        TranscriptFileOpenMode::EditablePane => {
            open_editable_request(workspace, request, window, cx);
        }
        TranscriptFileOpenMode::ReadOnlyPeek => {
            open_request(workspace, request, window, cx);
        }
    });
    true
}

fn open_editable_request(
    workspace: &mut Workspace,
    request: PeekRequest,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let project = workspace.project().clone();
    let candidates = request.candidates.clone();
    let anchor = request.anchor;
    let workspace = cx.weak_entity();
    window
        .spawn(cx, async move |cx| {
            let Some(abs_path) = first_existing_file(&candidates, &project, cx).await else {
                workspace.update_in(cx, |workspace, window, cx| {
                    open_request(workspace, request, window, cx);
                })?;
                return anyhow::Ok(());
            };
            let point = anchor.map(|anchor| {
                Point::new(
                    anchor.line.saturating_sub(1),
                    anchor.column.unwrap_or(1).saturating_sub(1),
                )
            });
            workspace.update_in(cx, |workspace, window, cx| {
                crate::open_abs_path_at_point(workspace, abs_path, point, window, cx);
            })?;
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
}

/// Open the reader on a file this window already knows the path of.
///
/// `OMEGA-DELTA-0125`. The two AGENTS.md entries in the thread header's `…`
/// menu called `open_abs_path`, which opens an item in the centre pane — the
/// same invisible success `OMEGA-DELTA-0119` found behind a transcript link.
/// Returns `true` when this took the click, so a caller in a full editor falls
/// through to the editor, where a real pane is strictly better than a sheet.
///
/// No candidate search: the caller resolved the path already. The failure
/// states still apply, because a rules file the menu offers can be deleted
/// between the menu being built and the entry being clicked, and a sheet that
/// declined to draw for that would be the dead click again.
pub fn open_file(
    workspace: &mut Workspace,
    path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    if !omega_zero_base::is_sealed() {
        return false;
    }
    let request = PeekRequest {
        written: path.to_string_lossy().into_owned().into(),
        candidates: vec![path],
        roots: Vec::new(),
        anchor: None,
    };
    open_request(workspace, request, window, cx);
    true
}

/// Open the reader on text this window produced, which is on no disk.
///
/// `OMEGA-DELTA-0125`. **Open Thread as Markdown** renders the thread to a
/// string and put it in a scratch buffer in the centre pane. The string is the
/// whole point and it needs no file, so the sheet takes it directly.
///
/// Read-only for the reason [`FilePeek`] is read-only everywhere else: zero
/// base refuses `workspace::Save`, and a scratch buffer a person can type into
/// and never keep is a worse lie than the one being repaired. Copying out still
/// works — `markdown` is an admitted namespace.
pub fn open_text(
    workspace: &mut Workspace,
    title: impl Into<SharedString>,
    text: String,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    if !omega_zero_base::is_sealed() {
        return false;
    }
    let project = workspace.project().clone();
    let title = title.into();
    workspace.toggle_modal(window, cx, |window, cx| {
        FilePeek::for_text(title, text, project, window, cx)
    });
    true
}

/// Put the sheet on the screen, from a request that is already resolved.
fn open_request(
    workspace: &mut Workspace,
    request: PeekRequest,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let project = workspace.project().clone();
    workspace.toggle_modal(window, cx, |window, cx| {
        FilePeek::new(request, project, window, cx)
    });
}

/// Turn a link into the set of files it could mean, or `None` when it names no
/// file at all.
///
/// Kept free of `App` so the whole grammar is testable without a window: the
/// forms below are the ones that actually appear in transcripts, and a
/// regression in any of them is a click that stops working.
pub fn parse_link(url: &str, roots: &[PathBuf]) -> Option<PeekRequest> {
    let (path_text, anchor) = split_url(url)?;
    if path_text.is_empty() {
        return None;
    }
    // A directory is not a file, and the reader has nothing to show for one.
    // `MentionUri::Directory` marks itself by a trailing separator.
    if path_text.ends_with('/') || path_text.ends_with('\\') {
        return None;
    }

    let path = Path::new(path_text.as_ref());
    let candidates = if path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        candidates_under_roots(path, roots)
    };

    Some(PeekRequest {
        written: path_text,
        candidates,
        roots: roots.to_vec(),
        anchor,
    })
}

/// Split a link into a path and an anchor, declining anything that is not a
/// local file.
///
/// The forms, all of which occur:
///
/// - `file:///abs/path`, `file:///abs/path#L42`, `file:///abs/path#L1-L150`,
///   and `file:///abs/path?column=7#L42:7` — what the code-span resolver and
///   `MentionUri` emit, and therefore the owner's case.
/// - `crates/foo.rs` and `crates/foo.rs:42:7` — what an agent writes into a
///   markdown link target by hand.
/// - `crates/foo.rs#L42` — the same, with the fragment spelling.
///
/// Anything with another scheme is declined, so `https://`, `zed:///agent/...`
/// threads, fetches and rules keep the behaviour they already have.
fn split_url(url: &str) -> Option<(SharedString, Option<PeekAnchor>)> {
    if let Ok(parsed) = url::Url::parse(url) {
        if parsed.scheme() != "file" {
            return None;
        }
        // `to_file_path` percent-decodes and gives platform separators. It
        // refuses a few shapes `Url::parse` accepts, and for those the raw path
        // is still better than declining the click.
        let path = match parsed.to_file_path() {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(()) => parsed.path().to_string(),
        };
        // `MentionUri::Selection` carries the column in the query rather than
        // the fragment when the fragment holds a range.
        let query_column = parsed.query_pairs().find_map(|(key, value)| {
            (key == "column")
                .then(|| value.parse::<u32>().ok())
                .flatten()
        });
        let mut anchor = parsed.fragment().and_then(parse_fragment_anchor);
        if let Some(anchor) = anchor.as_mut()
            && anchor.column.is_none()
        {
            anchor.column = query_column;
        }
        // A trailing separator survives `to_file_path` and is how a directory
        // mention is told apart, so it is preserved rather than trimmed here.
        let path = if parsed.path().ends_with('/') && !path.ends_with('/') {
            format!("{path}/")
        } else {
            path
        };
        return Some((path.into(), anchor));
    }

    // No scheme: a bare path, possibly with `#L…` or `:line:col`.
    if let Some((path, fragment)) = url.split_once('#') {
        let anchor = parse_fragment_anchor(fragment);
        return Some((path.into(), anchor));
    }

    let parsed = PathWithPosition::parse_str(url);
    let anchor = parsed.row.map(|line| PeekAnchor {
        line,
        column: parsed.column,
        end_line: None,
    });
    let path = parsed.path.to_string_lossy().into_owned();
    Some((path.into(), anchor))
}

/// `L42`, `L42:7`, `L1-L150`, and the same without the `L` or in lower case.
///
/// Tolerant on purpose. The `L` is a convention rather than a rule, and an
/// anchor that fails to parse must degrade to "open the file at the top"
/// rather than to "this is not a file link".
fn parse_fragment_anchor(fragment: &str) -> Option<PeekAnchor> {
    let cleaned = fragment.replace(['L', 'l'], "");
    let mut halves = cleaned.split('-');
    let (line, column) = parse_line_column(halves.next()?)?;
    let end_line = halves
        .next()
        .and_then(|half| Some(parse_line_column(half)?.0));
    Some(PeekAnchor {
        line,
        column,
        end_line,
    })
}

fn parse_line_column(text: &str) -> Option<(u32, Option<u32>)> {
    match text.split_once(':') {
        Some((line, column)) => Some((line.parse().ok()?, column.parse().ok())),
        None => Some((text.parse().ok()?, None)),
    }
}

/// Every absolute path a relative link could mean, in the order they are tried.
///
/// Two spellings per root, because agents write both. `crates/foo.rs` is joined
/// to the root directly; `omega/crates/foo.rs` — where the agent has repeated
/// the root's own folder name, which it sees in its `cwd` — is joined with that
/// first component removed. The direct spelling is tried first so an actual
/// `omega/` subdirectory still wins over the stripped reading.
fn candidates_under_roots(path: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for root in roots {
        let direct = root.join(path);
        if !candidates.contains(&direct) {
            candidates.push(direct);
        }
        if let Some(root_name) = root.file_name()
            && let Ok(stripped) = path.strip_prefix(root_name)
            && stripped.as_os_str().len() > 0
        {
            let nested = root.join(stripped);
            if !candidates.contains(&nested) {
                candidates.push(nested);
            }
        }
    }
    candidates
}

/// What the sheet is showing.
enum PeekState {
    /// Looking for the file and opening its buffer.
    Loading,
    /// Something was found and can be read.
    ///
    /// `OMEGA-DELTA-0125`. `abs_path` is `None` for text that came from the
    /// window rather than the disk. The header prints the subject's own name
    /// then, because printing a path for something that has none is how a
    /// read-only sheet gets mistaken for a file a person can go and edit.
    Ready {
        editor: Entity<Editor>,
        abs_path: Option<PathBuf>,
    },
    /// Nothing exists at any candidate path. The sheet says where it looked.
    Unresolved,
    /// A file exists and could not be opened. The sheet says why.
    Failed { reason: SharedString },
}

/// What the sheet was asked to show.
///
/// `OMEGA-DELTA-0125`. Only a path can fail to resolve, and only a path has
/// roots to name when it does. Keeping the two apart in the type is what stops
/// the generated case borrowing the file case's failure prose and telling a
/// person it looked in directories it never had.
enum PeekSubject {
    /// A path something named: a transcript link, or a menu entry that would
    /// otherwise have opened a pane.
    Path(PeekRequest),
    /// Text this window rendered. It exists nowhere on disk, so it can be
    /// neither unresolved nor stale.
    Text { title: SharedString },
}

impl PeekSubject {
    /// The name the sheet prints for this subject before anything is opened.
    fn written(&self) -> SharedString {
        match self {
            Self::Path(request) => request.written.clone(),
            Self::Text { title } => title.clone(),
        }
    }
}

/// The read-only sheet a transcript file link — or a thread-header menu
/// entry — opens.
pub struct FilePeek {
    focus_handle: FocusHandle,
    subject: PeekSubject,
    state: PeekState,
    _open: Task<()>,
}

impl FilePeek {
    fn new(
        request: PeekRequest,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let candidates = request.candidates.clone();
        let anchor = request.anchor;
        let open = cx.spawn_in(window, async move |this, cx| {
            let found = first_existing_file(&candidates, &project, cx).await;
            let Some(abs_path) = found else {
                this.update(cx, |this, cx| {
                    this.state = PeekState::Unresolved;
                    cx.notify();
                })
                .log_err();
                return;
            };

            let buffer = project
                .update(cx, |project, cx| project.open_local_buffer(&abs_path, cx))
                .await;

            match buffer {
                Ok(buffer) => {
                    this.update_in(cx, |this, window, cx| {
                        let editor = cx.new(|cx| {
                            let mut editor =
                                Editor::for_buffer(buffer, Some(project.clone()), window, cx);
                            // `OMEGA-DELTA-0119`. Read-only is the whole
                            // contract of this surface: zero base refuses the
                            // save action, so a sheet that accepted typing
                            // would drop it.
                            editor.set_read_only(true);
                            editor.set_show_breakpoints(false, cx);
                            editor.set_show_code_actions(false, cx);
                            editor.set_show_runnables(false, cx);
                            editor
                        });
                        if let Some(anchor) = anchor {
                            let point = Point::new(
                                anchor.line.saturating_sub(1),
                                anchor.column.unwrap_or(1).saturating_sub(1),
                            );
                            editor.update(cx, |editor, cx| {
                                editor.change_selections(
                                    SelectionEffects::scroll(Autoscroll::center()),
                                    window,
                                    cx,
                                    |selections| selections.select_ranges([point..point]),
                                );
                            });
                        }
                        this.state = PeekState::Ready {
                            editor,
                            abs_path: Some(abs_path),
                        };
                        cx.notify();
                    })
                    .log_err();
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.state = PeekState::Failed {
                            reason: error.to_string().into(),
                        };
                        cx.notify();
                    })
                    .log_err();
                }
            }
        });

        Self {
            focus_handle: cx.focus_handle(),
            subject: PeekSubject::Path(request),
            state: PeekState::Loading,
            _open: open,
        }
    }

    /// The same sheet, over text that was never a file.
    ///
    /// `OMEGA-DELTA-0125`. A scratch buffer rather than an opened one, so
    /// nothing on disk is touched and there is nothing to save. The markdown
    /// language is asked for by name and its absence is not fatal: a thread
    /// with no syntax highlighting is readable, and refusing to draw over a
    /// missing grammar would be the silent click again.
    fn for_text(
        title: SharedString,
        text: String,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let markdown = project.read(cx).languages().language_for_name("Markdown");
        let open = cx.spawn_in(window, async move |this, cx| {
            let language = markdown.await.log_err();
            let buffer = project
                .update(cx, |project, cx| project.create_buffer(language, false, cx))
                .await;

            match buffer {
                Ok(buffer) => {
                    this.update_in(cx, |this, window, cx| {
                        buffer.update(cx, |buffer, cx| buffer.set_text(text, cx));
                        let editor = cx.new(|cx| {
                            let mut editor =
                                Editor::for_buffer(buffer, Some(project.clone()), window, cx);
                            // The read-only contract is the same one
                            // `OMEGA-DELTA-0119` set, and it matters more here:
                            // this buffer has no file behind it at all, so
                            // typing into it could not be kept even if zero
                            // base admitted `workspace::Save`.
                            editor.set_read_only(true);
                            editor.set_show_breakpoints(false, cx);
                            editor.set_show_code_actions(false, cx);
                            editor.set_show_runnables(false, cx);
                            editor
                        });
                        this.state = PeekState::Ready {
                            editor,
                            abs_path: None,
                        };
                        cx.notify();
                    })
                    .log_err();
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.state = PeekState::Failed {
                            reason: error.to_string().into(),
                        };
                        cx.notify();
                    })
                    .log_err();
                }
            }
        });

        Self {
            focus_handle: cx.focus_handle(),
            subject: PeekSubject::Text { title },
            state: PeekState::Loading,
            _open: open,
        }
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    /// The line the header prints beside the path, or nothing.
    fn anchor_label(&self) -> Option<SharedString> {
        let PeekSubject::Path(request) = &self.subject else {
            return None;
        };
        let anchor = request.anchor?;
        Some(
            match (anchor.end_line, anchor.column) {
                (Some(end), _) if end != anchor.line => format!("Lines {}–{}", anchor.line, end),
                (_, Some(column)) => format!("Line {}, column {}", anchor.line, column),
                _ => format!("Line {}", anchor.line),
            }
            .into(),
        )
    }

    /// The sentence a link that resolved to nothing prints.
    ///
    /// It names the path as written and every directory that was searched. The
    /// two failures a person actually hits — the agent invented the path, and
    /// the agent is running somewhere else — are told apart by reading the
    /// root list, so the list is never elided.
    fn render_unresolved(&self, cx: &App) -> AnyElement {
        let (written, roots) = match &self.subject {
            PeekSubject::Path(request) => (request.written.clone(), request.roots.clone()),
            // Unreachable: text is never looked for, so it is never missing.
            // Drawing the same shape rather than panicking keeps a future
            // fourth entry point from crashing the window it was meant to fix.
            PeekSubject::Text { title } => (title.clone(), Vec::new()),
        };
        v_flex()
            .p_4()
            .gap_2()
            .child(Label::new(format!("No file at “{written}”.")).color(Color::Error))
            .child(
                Label::new(if roots.is_empty() {
                    "This thread has no working directory, so a relative path \
                     has nothing to be relative to. Choose a folder beside the \
                     composer and click the link again."
                        .to_string()
                } else {
                    format!(
                        "Looked under {} director{}:",
                        roots.len(),
                        if roots.len() == 1 { "y" } else { "ies" }
                    )
                })
                .color(Color::Muted)
                .size(LabelSize::Small),
            )
            .children(roots.iter().map(|root| {
                Label::new(root.to_string_lossy().into_owned())
                    .color(Color::Muted)
                    .size(LabelSize::Small)
                    .buffer_font(cx)
            }))
            .into_any_element()
    }
}

/// The first candidate that is an existing file, or `None`.
///
/// A directory is skipped rather than accepted, so a link that happens to name
/// one falls through to the same visible failure as a link that names nothing.
async fn first_existing_file(
    candidates: &[PathBuf],
    project: &Entity<Project>,
    cx: &mut gpui::AsyncApp,
) -> Option<PathBuf> {
    let fs = project.read_with(cx, |project, _| project.fs().clone());
    for candidate in candidates {
        let Ok(Some(metadata)) = fs.metadata(candidate).await else {
            continue;
        };
        if !metadata.is_dir {
            return Some(candidate.clone());
        }
    }
    None
}

impl Focusable for FilePeek {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for FilePeek {}

impl ModalView for FilePeek {
    fn fade_out_background(&self) -> bool {
        true
    }
}

impl Render for FilePeek {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        // The modal layer anchors this 5rem from the top of the window and
        // gives it no height of its own. Bounding the height here is what keeps
        // the sheet off the composer: it takes part in no layout, so it can
        // never push or clip it, but it can cover it, and covering the one
        // control zero base has would be its own dead end.
        let height = (viewport.height * 0.62).max(px(220.));
        let width = (viewport.width - px(96.)).min(px(1100.)).max(px(320.));

        let written = self.subject.written();
        let path_label = match &self.state {
            PeekState::Ready {
                abs_path: Some(abs_path),
                ..
            } => abs_path.to_string_lossy().into_owned(),
            _ => written.to_string(),
        };

        v_flex()
            .id("omega-file-peek")
            .key_context("OmegaFilePeek")
            .w(width)
            .h(height)
            .elevation_3(cx)
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                if !this.focus_handle.contains_focused(window, cx) {
                    this.focus_handle.focus(window, cx);
                }
            }))
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                Label::new(path_label)
                                    .buffer_font(cx)
                                    .size(LabelSize::Small)
                                    .truncate(),
                            )
                            .children(self.anchor_label().map(|label| {
                                Label::new(label)
                                    .color(Color::Muted)
                                    .size(LabelSize::XSmall)
                            })),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .child(
                                Label::new("Read only")
                                    .color(Color::Muted)
                                    .size(LabelSize::XSmall),
                            )
                            .child(
                                IconButton::new("omega-file-peek-close", IconName::Close)
                                    .icon_size(IconSize::Small)
                                    .tooltip(Tooltip::text("Close (esc)"))
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(DismissEvent);
                                    })),
                            ),
                    ),
            )
            .child(match &self.state {
                PeekState::Loading => v_flex()
                    .flex_1()
                    .p_4()
                    .child(Label::new("Opening…").color(Color::Muted))
                    .into_any_element(),
                PeekState::Ready { editor, .. } => div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(editor.clone())
                    .into_any_element(),
                PeekState::Unresolved => self.render_unresolved(cx),
                PeekState::Failed { reason } => v_flex()
                    .flex_1()
                    .p_4()
                    .gap_2()
                    .child(Label::new(format!("Could not open “{written}”.")).color(Color::Error))
                    .child(
                        Label::new(reason.clone())
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                    )
                    .into_any_element(),
            })
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .px_3()
                    .py_1p5()
                    .border_t_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        Label::new(match &self.subject {
                            // The path exactly as it arrived, so a person can
                            // compare the sheet to the message above it.
                            PeekSubject::Path(request) => {
                                format!("As written: {}", request.written)
                            }
                            // OMEGA-DELTA-0125. There is no "as written" for a
                            // thread: nothing wrote it down. Saying where it
                            // came from is what stops the sheet reading like a
                            // file that has gone missing from the header.
                            PeekSubject::Text { .. } => {
                                "Rendered from this thread. Not a file on disk.".to_string()
                            }
                        })
                        .color(Color::Muted)
                        .size(LabelSize::XSmall)
                        .truncate(),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/Users/owner/omega")]
    }

    /// The owner's case. A code span the resolver already turned into a
    /// `file://` URI must come back out as the same absolute path.
    #[test]
    fn a_file_uri_is_the_path_it_names() {
        let request =
            parse_link("file:///Users/owner/omega/crates/agent/src/x.hbs", &roots()).unwrap();
        assert_eq!(
            request.candidates,
            vec![PathBuf::from("/Users/owner/omega/crates/agent/src/x.hbs")]
        );
        assert_eq!(request.anchor, None);
    }

    /// `crates/foo.rs:42` and `crates/foo.rs:42:7` appear in transcripts today.
    /// Both are anchors, and both resolve against the roots.
    #[test]
    fn a_relative_path_carries_its_line_and_column() {
        let request = parse_link("crates/foo.rs:42:7", &roots()).unwrap();
        assert_eq!(
            request.candidates,
            vec![PathBuf::from("/Users/owner/omega/crates/foo.rs")]
        );
        assert_eq!(
            request.anchor,
            Some(PeekAnchor {
                line: 42,
                column: Some(7),
                end_line: None
            })
        );

        let line_only = parse_link("crates/foo.rs:42", &roots()).unwrap();
        assert_eq!(
            line_only.anchor,
            Some(PeekAnchor {
                line: 42,
                column: None,
                end_line: None
            })
        );
    }

    /// The `#L1-L150` spelling, which is what a range in a mention URI looks
    /// like, and the lower-case and bare-number tolerances with it.
    #[test]
    fn a_fragment_range_is_a_range() {
        for url in [
            "file:///Users/owner/omega/a.rs#L1-L150",
            "file:///Users/owner/omega/a.rs#l1-l150",
            "file:///Users/owner/omega/a.rs#1-150",
        ] {
            let request = parse_link(url, &roots()).unwrap();
            assert_eq!(
                request.anchor,
                Some(PeekAnchor {
                    line: 1,
                    column: None,
                    end_line: Some(150)
                }),
                "{url} lost its range"
            );
        }
    }

    /// A fragment that is not a line number leaves the file openable at the
    /// top. Refusing the whole link over an unreadable anchor would turn a
    /// cosmetic problem into the dead click this delta removes.
    #[test]
    fn an_unreadable_anchor_still_opens_the_file() {
        let request = parse_link("file:///Users/owner/omega/a.rs#section-two", &roots()).unwrap();
        assert_eq!(
            request.candidates,
            vec![PathBuf::from("/Users/owner/omega/a.rs")]
        );
        assert_eq!(request.anchor, None);
    }

    /// A path that repeats the root's own folder name is tried both ways, and
    /// the literal join is tried first so a real `omega/` subdirectory wins.
    #[test]
    fn a_root_prefixed_path_is_tried_both_ways() {
        let request = parse_link("omega/crates/foo.rs", &roots()).unwrap();
        assert_eq!(
            request.candidates,
            vec![
                PathBuf::from("/Users/owner/omega/omega/crates/foo.rs"),
                PathBuf::from("/Users/owner/omega/crates/foo.rs"),
            ]
        );
    }

    /// Every root is tried, in the order given, so the thread's own working
    /// directory can be put ahead of the project's worktrees.
    #[test]
    fn every_root_is_a_candidate_in_order() {
        let roots = vec![PathBuf::from("/work/one"), PathBuf::from("/work/two")];
        let request = parse_link("a.rs", &roots).unwrap();
        assert_eq!(
            request.candidates,
            vec![
                PathBuf::from("/work/one/a.rs"),
                PathBuf::from("/work/two/a.rs")
            ]
        );
        assert_eq!(request.roots, roots);
    }

    /// A link with no root to resolve against still parses, so the sheet opens
    /// and says there is nowhere to look. Returning `None` here would hand the
    /// click back to the workspace and reproduce the invisible open.
    #[test]
    fn a_relative_path_with_no_roots_still_becomes_a_request() {
        let request = parse_link("crates/foo.rs", &[]).unwrap();
        assert!(request.candidates.is_empty());
        assert!(request.roots.is_empty());
        assert_eq!(request.written.as_ref(), "crates/foo.rs");
    }

    /// What the reader must not take. These have working destinations already,
    /// and a sheet that swallowed them would be a regression rather than a fix.
    #[test]
    fn the_reader_declines_what_is_not_a_local_file() {
        for url in [
            "https://example.com/a.rs",
            "http://example.com/a.rs",
            "zed:///agent/thread/abc",
            "zed:///agent/pasted-image?name=x",
            "file:///Users/owner/omega/crates/",
        ] {
            assert!(
                parse_link(url, &roots()).is_none(),
                "{url} must be left to the ordinary link handler"
            );
        }
    }

    /// The path a failure prints is the one the agent wrote, never a rewritten
    /// one, so a person can compare the sheet to the message above it.
    #[test]
    fn the_written_path_survives_into_the_request() {
        let request = parse_link("crates/agent/src/templates/system_prompt.hbs", &roots()).unwrap();
        assert_eq!(
            request.written.as_ref(),
            "crates/agent/src/templates/system_prompt.hbs"
        );
    }
}
