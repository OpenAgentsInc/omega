use std::{ops::Range, sync::Arc};

use acp_thread::{AcpThread, AgentThreadEntry, AssistantMessageChunk};
use agent::ThreadStore;
use agent_client_protocol::schema::v1 as acp;
use agent_settings::AgentSettings;
use collections::{HashMap, HashSet};
use editor::{
    Editor, EditorEvent, EditorMode, MinimapVisibility, RestoreOnlyUnstagedDiffHunkDelegate,
    SizingBehavior,
};
use gpui::{
    AnyEntity, App, AppContext as _, Entity, EntityId, EventEmitter, FocusHandle, Focusable,
    ScrollHandle, TextStyleRefinement, WeakEntity, Window,
};
use language::{LanguageRegistry, language_settings::SoftWrap};
use markdown::{Markdown, MarkdownOptions};
use project::{AgentId, Project, project_settings::DiagnosticSeverity};
use rope::Point;
use settings::{Settings as _, ThinkingBlockDisplay};
use terminal_view::TerminalView;
use theme_settings::ThemeSettings;
use ui::{Context, SharedString, TextSize};
use workspace::Workspace;

use crate::message_editor::{MessageEditor, MessageEditorEvent, SharedSessionCapabilities};

/// Maps an entry index through the removal of `removed` (a contiguous range of
/// entries), returning `None` if the index referred to a removed entry.
fn reindex_after_removal(index: usize, removed: &Range<usize>) -> Option<usize> {
    if index < removed.start {
        Some(index)
    } else if index < removed.end {
        None
    } else {
        Some(index - removed.len())
    }
}

/// `OMEGA-DELTA-0080`. How many lines of a tool call's result body the thread
/// shows before it needs the reader's permission to show more.
///
/// Upstream Zed has no ceiling below `TerminalView::MAX_EMBEDDED_LINES`
/// (1,000), so a forty-line command result renders forty lines tall and pushes
/// the turn that produced it off the screen.
///
/// Sixteen is the height the tree already treats as a bounded terminal: the
/// scrollable fallback for a result over 1,000 lines is `h_72` (18rem, 288px),
/// which at the agent panel's text size is about sixteen lines. So a capped
/// result is the same size as a result that was already capped, and the
/// ceiling costs about a fifth of a full-height window instead of all of it.
pub(crate) const COLLAPSED_TOOL_OUTPUT_LINES: usize = 16;

/// `OMEGA-DELTA-0080`. The label of the control under a capped result body, or
/// `None` when the body needs no control.
///
/// A short result shows every line it has, so it gains nothing. A capped result
/// names the count of lines it is hiding, because "Show more" does not tell a
/// reader whether opening it is worth the screen it costs.
///
/// **This is the only place the sentence is formed.** `OMEGA-DELTA-0103`
/// bounds the *record*: a tool result becomes an artifact, and the event
/// carries a preview plus a marker naming the withheld amount. So the body this
/// function describes may itself be short of the result — `total_lines` counts
/// only what reached this surface, and on its own this label would state a
/// total that is not the total. A reader who lifted the ceiling and reached the
/// last line would conclude they had the whole result.
///
/// `record_total_lines` is the repair, and it is here rather than in a second
/// sentence elsewhere for the reason `OMEGA-DELTA-0060` gives: a reader who is
/// told about one bound and not the other has been told the body is complete.
/// Pass the line count of the complete artifact when the record holds more than
/// this surface does, and `None` when the body *is* the result — which is the
/// ordinary case, and must stay as silent as it was before.
pub(crate) fn tool_output_ceiling_label(
    total_lines: usize,
    displayed_lines: usize,
    is_capped: bool,
    record_total_lines: Option<usize>,
) -> Option<SharedString> {
    // A record that claims fewer lines than are already on screen is not a
    // record of this body; ignore it rather than report a negative remainder.
    let withheld = record_total_lines
        .unwrap_or(total_lines)
        .saturating_sub(total_lines);

    let toggle = if is_capped {
        match total_lines.saturating_sub(displayed_lines) {
            0 => None,
            1 => Some("Show 1 more line".to_owned()),
            hidden => Some(format!("Show {hidden} more lines")),
        }
    } else if total_lines > COLLAPSED_TOOL_OUTPUT_LINES {
        Some("Show fewer lines".to_owned())
    } else {
        None
    };

    match (toggle, withheld) {
        (None, 0) => None,
        (None, 1) => Some("1 more line is withheld from this result".into()),
        (None, withheld) => {
            Some(format!("{withheld} more lines are withheld from this result").into())
        }
        (Some(toggle), 0) => Some(toggle.into()),
        (Some(toggle), 1) => Some(format!("{toggle} · 1 more withheld").into()),
        (Some(toggle), withheld) => Some(format!("{toggle} · {withheld} more withheld").into()),
    }
}

/// `OMEGA-DELTA-0103`. Whether the ceiling control has anything to toggle.
///
/// A label can now exist for a body the ceiling never bound — one that is short
/// on screen because the *record* withheld the rest. The sentence still belongs
/// to `tool_output_ceiling_label`; what changes is that the control carrying it
/// must not pretend to open something.
pub(crate) fn tool_output_ceiling_is_toggleable(
    total_lines: usize,
    displayed_lines: usize,
    is_capped: bool,
) -> bool {
    if is_capped {
        total_lines > displayed_lines
    } else {
        total_lines > COLLAPSED_TOOL_OUTPUT_LINES
    }
}

/// `OMEGA-DELTA-0124`. What heads a thinking block that wrote no title of its
/// own, and could not be given one.
pub(crate) const UNTITLED_THOUGHT_HEADING: &str = "Thinking";

/// `OMEGA-DELTA-0124`. A thinking block, split into the lines its header
/// presents and the source that renders underneath them.
///
/// The header used to say the word "Thinking" and nothing else, so a run of
/// five thoughts read as five identical labels with the real content indented
/// beneath each. The owner, looking at three in a row while testing a build:
/// *"rather than showing 'Thinking' each time, where it says Thinking I want it
/// showing the actual thought, not on a separate line"*.
///
/// The trap in that is the second half. Moving the title *up* is one line of
/// view code; it was written that way once and shipped, and every thought then
/// rendered its title **twice** — muted in the header, and still bold in the
/// body underneath, because the header reads the same `Entity<Markdown>` the
/// body renders. Hence this split: `titles` are removed from `body`, so there
/// is no source left for the body to draw them from.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SplitThought {
    /// The titles the model itself wrote, in the order it wrote them. Every one
    /// of these has been taken out of `body`.
    ///
    /// A block can hold more than one thought, and the owner saw it: two titles
    /// under a single lightbulb, the second reading as a subheading of the
    /// first. His call — *"if theres 2 thoughts in a single 'thinking block' ...
    /// u can just show that same lightbulb line twice"* — so this is a list,
    /// and the header draws one row per entry.
    pub titles: Vec<SharedString>,
    /// What to head a thought that wrote no title with.
    ///
    /// This is a *preview* of prose, not a title, and it is deliberately still
    /// in `body`. A title is a label for the paragraph under it and says
    /// nothing that paragraph does not; a first sentence is the thought itself,
    /// and lifting it out of the body would delete content to avoid an echo.
    /// So the echo is accepted here and nowhere else, and it is why
    /// `titles` and `preview` are separate fields rather than one list: only
    /// the first kind is removed, and a check can tell them apart.
    pub preview: Option<SharedString>,
    /// The source that renders under the header. Contains no title line — see
    /// `a_title_never_appears_in_both_the_header_and_the_body`.
    pub body: String,
}

impl SplitThought {
    /// The muted lines the header draws, top to bottom. Never empty: a thought
    /// mid-stream has a row before it has words.
    pub(crate) fn headings(&self) -> Vec<SharedString> {
        if !self.titles.is_empty() {
            return self.titles.clone();
        }
        vec![
            self.preview
                .clone()
                .unwrap_or_else(|| SharedString::new_static(UNTITLED_THOUGHT_HEADING)),
        ]
    }
}

/// `OMEGA-DELTA-0124`. The title a line carries, if it is a title line.
///
/// **Emphasis marks a title, not position.** The second thought in a block is
/// the second *emphasised* line, not the second line, so this is asked of every
/// line rather than of the first one.
///
/// A streaming title has no closing marker — `**Search` arrives with nothing
/// after it and stays that way until the next token — so an unterminated `**`
/// run counts. Waiting for the close would leave the header blank for the whole
/// time a thought is being written, which is the only time anybody is watching
/// it.
///
/// A *closed* `**` run must own the whole line. `**Note** that the file is
/// gone` is a bold lead-in inside prose, and hoisting it would put half a
/// sentence in the header and delete it from the paragraph it belongs to.
pub(crate) fn thought_title(line: &str) -> Option<&str> {
    // Four spaces is an indented code block, where a `#` is a comment.
    if line.len() - line.trim_start().len() >= 4 {
        return None;
    }
    let line = line.trim();

    if line.starts_with('#') {
        let level = line
            .chars()
            .take_while(|&character| character == '#')
            .count();
        // `#######` is not a heading, and `#Title` is not one either — treating
        // it as one would take a line of prose out of the body.
        if level > 6 || !line[level..].starts_with(' ') {
            return None;
        }
        let title = line[level..].trim_matches(['#', ' '].as_slice());
        return (!title.is_empty()).then_some(title);
    }

    let rest = line.strip_prefix("**")?;
    match rest.find("**") {
        Some(end) => {
            if !rest[end + 2..].trim().is_empty() {
                return None;
            }
            let title = rest[..end].trim();
            (!title.is_empty()).then_some(title)
        }
        None => {
            let title = rest.trim();
            (!title.is_empty()).then_some(title)
        }
    }
}

/// The opening or closing marker of a fenced code block, if the line is one.
fn code_fence(line: &str) -> Option<&'static str> {
    ["```", "~~~"]
        .into_iter()
        .find(|marker| line.starts_with(marker))
}

/// `OMEGA-DELTA-0124`. Split a thinking block into its header lines and the
/// body that renders beneath them.
///
/// Every title found is *removed* from the body. That removal is the whole
/// point: the header and the body are drawn from the same block of text, so the
/// only way the title cannot appear twice is for the body's copy not to exist.
pub(crate) fn split_thought(source: &str) -> SplitThought {
    let mut titles = Vec::new();
    let mut body = String::with_capacity(source.len());
    let mut fence: Option<&str> = None;
    // A blank line is only worth writing if something follows it. Removing a
    // title from `prose\n\n**Title**\n\nprose` otherwise leaves two blank lines
    // where the model wrote one, and a leading one where it wrote none.
    let mut blank_is_owed = false;

    for line in source.split('\n') {
        let trimmed = line.trim();

        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            // Verbatim: a `# comment` in a shell snippet is not a heading, and
            // a line taken out of a fence changes what the code says.
            body.push_str(line);
            body.push('\n');
            continue;
        }

        if trimmed.is_empty() {
            blank_is_owed = !body.is_empty();
            continue;
        }

        if let Some(title) = thought_title(line) {
            titles.push(SharedString::from(title.to_owned()));
            continue;
        }

        // A fence opens *after* the title question, not before it: no line can
        // be both, and asking in this order keeps the two rules independent.
        fence = code_fence(trimmed);

        if blank_is_owed {
            body.push('\n');
            blank_is_owed = false;
        }
        body.push_str(line);
        body.push('\n');
    }

    // Only when the model wrote no title at all: the first line it did write,
    // stripped of the markers of the formatting it is no longer getting, so a
    // half-arrived `**` does not render as a row of asterisks.
    let preview = titles.is_empty().then(|| {
        source
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.trim_matches(['#', '*', '_', '`', ' '].as_slice()))
            .filter(|line| !line.is_empty())
            .map(|line| SharedString::from(line.to_owned()))
    });

    SplitThought {
        titles,
        preview: preview.flatten(),
        body: body.trim_end().to_owned(),
    }
}

pub struct EntryViewState {
    workspace: WeakEntity<Workspace>,
    project: WeakEntity<Project>,
    thread_store: Option<Entity<ThreadStore>>,
    entries: Vec<Entry>,
    session_capabilities: SharedSessionCapabilities,
    agent_id: AgentId,
    expanded_thinking_blocks: HashSet<(usize, usize)>,
    auto_expanded_thinking_block: Option<(usize, usize)>,
    user_toggled_thinking_blocks: HashSet<(usize, usize)>,
    expanded_compactions: HashSet<usize>,
    expanded_tool_calls: HashSet<acp::ToolCallId>,
}

impl EntryViewState {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        project: WeakEntity<Project>,
        thread_store: Option<Entity<ThreadStore>>,
        session_capabilities: SharedSessionCapabilities,
        agent_id: AgentId,
    ) -> Self {
        Self {
            workspace,
            project,
            thread_store,
            entries: Vec::new(),
            session_capabilities,
            agent_id,
            expanded_thinking_blocks: HashSet::default(),
            auto_expanded_thinking_block: None,
            user_toggled_thinking_blocks: HashSet::default(),
            expanded_compactions: HashSet::default(),
            expanded_tool_calls: HashSet::default(),
        }
    }

    pub(crate) fn is_tool_call_expanded(&self, tool_call_id: &acp::ToolCallId) -> bool {
        self.expanded_tool_calls.contains(tool_call_id)
    }

    pub(crate) fn expand_tool_call(&mut self, tool_call_id: acp::ToolCallId) {
        self.expanded_tool_calls.insert(tool_call_id);
    }

    pub(crate) fn collapse_tool_call(&mut self, tool_call_id: &acp::ToolCallId) {
        self.expanded_tool_calls.remove(tool_call_id);
    }

    pub(crate) fn toggle_tool_call_expansion(&mut self, tool_call_id: &acp::ToolCallId) {
        if !self.expanded_tool_calls.remove(tool_call_id) {
            self.expanded_tool_calls.insert(tool_call_id.clone());
        }
    }

    pub(crate) fn is_compaction_expanded(&self, entry_ix: usize) -> bool {
        self.expanded_compactions.contains(&entry_ix)
    }

    pub(crate) fn collapse_compaction(&mut self, entry_ix: usize) {
        self.expanded_compactions.remove(&entry_ix);
    }

    pub(crate) fn toggle_compaction_expansion(&mut self, entry_ix: usize) {
        if !self.expanded_compactions.remove(&entry_ix) {
            self.expanded_compactions.insert(entry_ix);
        }
    }

    pub(crate) fn clear_auto_expand_tracking(&mut self) {
        self.auto_expanded_thinking_block = None;
    }

    pub(crate) fn is_auto_expanded_thinking_block(&self, key: (usize, usize)) -> bool {
        self.auto_expanded_thinking_block == Some(key)
    }

    pub(crate) fn auto_expand_streaming_thought(&mut self, thread: &AcpThread, cx: &App) -> bool {
        let thinking_display = AgentSettings::get_global(cx).thinking_display;

        if !matches!(
            thinking_display,
            ThinkingBlockDisplay::Auto | ThinkingBlockDisplay::Preview
        ) {
            return false;
        }

        let last_ix = thread.entries().len().saturating_sub(1);
        let key = match thread.entries().get(last_ix) {
            Some(AgentThreadEntry::AssistantMessage(message)) => match message.chunks.last() {
                Some(AssistantMessageChunk::Thought { .. }) => {
                    Some((last_ix, message.chunks.len() - 1))
                }
                _ => None,
            },
            _ => None,
        };

        if let Some(key) = key {
            if self.auto_expanded_thinking_block != Some(key) {
                self.auto_expanded_thinking_block = Some(key);
                self.expanded_thinking_blocks.insert(key);
                return true;
            }
        } else if self.auto_expanded_thinking_block.is_some() {
            if thinking_display == ThinkingBlockDisplay::Auto
                && let Some(key) = self.auto_expanded_thinking_block
                && !self.user_toggled_thinking_blocks.contains(&key)
            {
                self.expanded_thinking_blocks.remove(&key);
            }
            self.auto_expanded_thinking_block = None;
            return true;
        }

        false
    }

    pub(crate) fn toggle_thinking_block_expansion(&mut self, key: (usize, usize), cx: &App) {
        match AgentSettings::get_global(cx).thinking_display {
            ThinkingBlockDisplay::Auto => {
                let is_open = self.expanded_thinking_blocks.contains(&key)
                    || self.user_toggled_thinking_blocks.contains(&key);

                if is_open {
                    self.expanded_thinking_blocks.remove(&key);
                    self.user_toggled_thinking_blocks.remove(&key);
                } else {
                    self.expanded_thinking_blocks.insert(key);
                    self.user_toggled_thinking_blocks.insert(key);
                }
            }
            ThinkingBlockDisplay::Preview => {
                let is_user_expanded = self.user_toggled_thinking_blocks.contains(&key);
                let is_in_expanded_set = self.expanded_thinking_blocks.contains(&key);

                if is_user_expanded {
                    self.user_toggled_thinking_blocks.remove(&key);
                    self.expanded_thinking_blocks.remove(&key);
                } else if is_in_expanded_set {
                    self.user_toggled_thinking_blocks.insert(key);
                } else {
                    self.expanded_thinking_blocks.insert(key);
                    self.user_toggled_thinking_blocks.insert(key);
                }
            }
            ThinkingBlockDisplay::AlwaysExpanded => {
                if self.user_toggled_thinking_blocks.contains(&key) {
                    self.user_toggled_thinking_blocks.remove(&key);
                } else {
                    self.user_toggled_thinking_blocks.insert(key);
                }
            }
            ThinkingBlockDisplay::AlwaysCollapsed => {
                if self.user_toggled_thinking_blocks.contains(&key) {
                    self.user_toggled_thinking_blocks.remove(&key);
                    self.expanded_thinking_blocks.remove(&key);
                } else {
                    self.expanded_thinking_blocks.insert(key);
                    self.user_toggled_thinking_blocks.insert(key);
                }
            }
        }
    }

    pub(crate) fn thinking_block_state(&self, key: (usize, usize), cx: &App) -> (bool, bool) {
        let is_user_toggled = self.user_toggled_thinking_blocks.contains(&key);
        let is_in_expanded_set = self.expanded_thinking_blocks.contains(&key);

        match AgentSettings::get_global(cx).thinking_display {
            ThinkingBlockDisplay::Auto => {
                let is_open = is_user_toggled || is_in_expanded_set;
                (is_open, false)
            }
            ThinkingBlockDisplay::Preview => {
                let is_open = is_user_toggled || is_in_expanded_set;
                let is_constrained = is_in_expanded_set && !is_user_toggled;
                (is_open, is_constrained)
            }
            ThinkingBlockDisplay::AlwaysExpanded => (!is_user_toggled, false),
            ThinkingBlockDisplay::AlwaysCollapsed => (is_user_toggled, false),
        }
    }

    pub fn entry(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index)
    }

    pub fn sync_missing_entries(
        &mut self,
        thread: &Entity<AcpThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let start = self.entries.len();
        let end = thread.read(cx).entries().len();
        if start >= end {
            return end..end;
        }
        for index in start..end {
            self.sync_entry(index, thread, window, cx);
        }
        start..end
    }

    pub fn sync_entry(
        &mut self,
        index: usize,
        thread: &Entity<AcpThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread_entry) = thread.read(cx).entries().get(index) else {
            return;
        };

        match thread_entry {
            AgentThreadEntry::UserMessage(message) => {
                let can_rewind = thread.read(cx).supports_truncate(cx);
                let has_client_id = message.client_id.is_some();
                let is_subagent = thread.read(cx).parent_session_id().is_some();
                let chunks = message.chunks.clone();
                if let Some(Entry::UserMessage(editor)) = self.entries.get_mut(index) {
                    if !editor.focus_handle(cx).is_focused(window) {
                        // Only update if we are not editing.
                        // If we are, cancelling the edit will set the message to the newest content.
                        editor.update(cx, |editor, cx| {
                            editor.set_message(chunks, window, cx);
                        });
                    }
                } else {
                    let message_editor = cx.new(|cx| {
                        let mut editor = MessageEditor::new(
                            self.workspace.clone(),
                            self.project.clone(),
                            self.thread_store.clone(),
                            self.session_capabilities.clone(),
                            self.agent_id.clone(),
                            "Edit message － @ to include context",
                            editor::EditorMode::AutoHeight {
                                min_lines: 1,
                                max_lines: None,
                            },
                            window,
                            cx,
                        );
                        if !can_rewind || !has_client_id || is_subagent {
                            editor.set_read_only(true, cx);
                        }
                        editor.set_message(chunks, window, cx);
                        editor
                    });
                    cx.subscribe(&message_editor, move |_, editor, event, cx| {
                        cx.emit(EntryViewEvent {
                            entry_index: index,
                            view_event: ViewEvent::MessageEditorEvent(editor, event.clone()),
                        })
                    })
                    .detach();
                    self.set_entry(index, Entry::UserMessage(message_editor));
                }
            }
            AgentThreadEntry::ToolCall(tool_call) => {
                let id = tool_call.id.clone();
                let terminals = tool_call.terminals().cloned().collect::<Vec<_>>();
                let diffs = tool_call.diffs().cloned().collect::<Vec<_>>();

                let views = if let Some(Entry::ToolCall(tool_call)) = self.entries.get_mut(index) {
                    &mut tool_call.content
                } else {
                    self.set_entry(
                        index,
                        Entry::ToolCall(ToolCallEntry {
                            content: HashMap::default(),
                            focus_handle: cx.focus_handle(),
                        }),
                    );
                    let Some(Entry::ToolCall(tool_call)) = self.entries.get_mut(index) else {
                        unreachable!()
                    };
                    &mut tool_call.content
                };

                let is_tool_call_completed =
                    matches!(tool_call.status, acp_thread::ToolCallStatus::Completed);

                for terminal in terminals {
                    match views.entry(terminal.entity_id()) {
                        collections::hash_map::Entry::Vacant(entry) => {
                            let element = create_terminal(
                                self.workspace.clone(),
                                self.project.clone(),
                                terminal.clone(),
                                window,
                                cx,
                            )
                            .into_any();
                            cx.emit(EntryViewEvent {
                                entry_index: index,
                                view_event: ViewEvent::NewTerminal(id.clone()),
                            });
                            entry.insert(element);
                        }
                        collections::hash_map::Entry::Occupied(_entry) => {
                            if is_tool_call_completed && terminal.read(cx).output().is_none() {
                                cx.emit(EntryViewEvent {
                                    entry_index: index,
                                    view_event: ViewEvent::TerminalMovedToBackground(id.clone()),
                                });
                            }
                        }
                    }
                }

                for diff in diffs {
                    views.entry(diff.entity_id()).or_insert_with(|| {
                        let editor = create_editor_diff(diff.clone(), window, cx);
                        cx.subscribe(&editor, {
                            let diff = diff.clone();
                            let entry_index = index;
                            move |_this, _editor, event: &EditorEvent, cx| {
                                if let EditorEvent::OpenExcerptsRequested {
                                    selections_by_buffer,
                                    split,
                                } = event
                                {
                                    let multibuffer = diff.read(cx).multibuffer();
                                    if let Some((buffer_id, (ranges, _))) =
                                        selections_by_buffer.iter().next()
                                    {
                                        if let Some(buffer) =
                                            multibuffer.read(cx).buffer(*buffer_id)
                                        {
                                            if let Some(range) = ranges.first() {
                                                let point =
                                                    buffer.read(cx).offset_to_point(range.start.0);
                                                if let Some(path) = diff.read(cx).file_path(cx) {
                                                    cx.emit(EntryViewEvent {
                                                        entry_index,
                                                        view_event: ViewEvent::OpenDiffLocation {
                                                            path,
                                                            position: point,
                                                            split: *split,
                                                        },
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        })
                        .detach();
                        cx.emit(EntryViewEvent {
                            entry_index: index,
                            view_event: ViewEvent::NewDiff(id.clone()),
                        });
                        editor.into_any()
                    });
                }
            }
            AgentThreadEntry::Elicitation(_) => {
                if !matches!(self.entries.get(index), Some(Entry::Elicitation { .. })) {
                    self.set_entry(
                        index,
                        Entry::Elicitation {
                            focus_handle: cx.focus_handle(),
                        },
                    );
                }
            }
            AgentThreadEntry::AssistantMessage(message) => {
                // `OMEGA-DELTA-0124`. Read before the entry is borrowed
                // mutably: the body a thought's header is split from is parsed
                // with the same language registry its own markdown was.
                let language_registry = self
                    .project
                    .upgrade()
                    .map(|project| project.read(cx).languages().clone());
                let message = MessageThoughts::new(message);
                let entry = if let Some(Entry::AssistantMessage(entry)) =
                    self.entries.get_mut(index)
                {
                    entry
                } else {
                    self.set_entry(
                        index,
                        Entry::AssistantMessage(AssistantMessageEntry {
                            scroll_handles_by_chunk_index: HashMap::default(),
                            thoughts_by_chunk_index: HashMap::default(),
                            focus_handle: cx.focus_handle(),
                        }),
                    );
                    let Some(Entry::AssistantMessage(entry)) = self.entries.get_mut(index) else {
                        unreachable!()
                    };
                    entry
                };
                entry.sync(message, language_registry, cx);
            }
            AgentThreadEntry::CompletedPlan(_) => {
                if !matches!(self.entries.get(index), Some(Entry::CompletedPlan)) {
                    self.set_entry(index, Entry::CompletedPlan);
                }
            }
            AgentThreadEntry::ContextCompaction(_) => {
                if !matches!(self.entries.get(index), Some(Entry::ContextCompaction)) {
                    self.set_entry(index, Entry::ContextCompaction);
                }
            }
            // OMEGA-DELTA-0045. A host-authored note has no editor, no tool
            // content and no focusable body — it is a rendered line of text.
            AgentThreadEntry::SystemNote(_) => {
                if !matches!(self.entries.get(index), Some(Entry::SystemNote)) {
                    self.set_entry(index, Entry::SystemNote);
                }
            }
        };
    }

    fn set_entry(&mut self, index: usize, entry: Entry) {
        if index == self.entries.len() {
            self.entries.push(entry);
        } else {
            self.entries[index] = entry;
        }
    }

    pub fn remove(&mut self, range: Range<usize>) {
        self.entries.drain(range.clone());

        self.expanded_compactions = self
            .expanded_compactions
            .iter()
            .filter_map(|&entry_ix| reindex_after_removal(entry_ix, &range))
            .collect();
        self.expanded_thinking_blocks = self
            .expanded_thinking_blocks
            .iter()
            .filter_map(|&(entry_ix, chunk_ix)| {
                reindex_after_removal(entry_ix, &range).map(|entry_ix| (entry_ix, chunk_ix))
            })
            .collect();
        self.user_toggled_thinking_blocks = self
            .user_toggled_thinking_blocks
            .iter()
            .filter_map(|&(entry_ix, chunk_ix)| {
                reindex_after_removal(entry_ix, &range).map(|entry_ix| (entry_ix, chunk_ix))
            })
            .collect();
        self.auto_expanded_thinking_block =
            self.auto_expanded_thinking_block
                .and_then(|(entry_ix, chunk_ix)| {
                    reindex_after_removal(entry_ix, &range).map(|entry_ix| (entry_ix, chunk_ix))
                });
    }

    pub fn agent_ui_font_size_changed(&mut self, cx: &mut App) {
        for entry in self.entries.iter() {
            match entry {
                Entry::UserMessage { .. }
                | Entry::AssistantMessage { .. }
                | Entry::Elicitation { .. }
                | Entry::CompletedPlan
                | Entry::ContextCompaction
                | Entry::SystemNote => {}
                Entry::ToolCall(ToolCallEntry { content, .. }) => {
                    for view in content.values() {
                        if let Ok(diff_editor) = view.clone().downcast::<Editor>() {
                            diff_editor.update(cx, |diff_editor, cx| {
                                diff_editor.set_text_style_refinement(
                                    diff_editor_text_style_refinement(cx),
                                );
                                cx.notify();
                            })
                        }
                    }
                }
            }
        }
    }
}

impl EventEmitter<EntryViewEvent> for EntryViewState {}

pub struct EntryViewEvent {
    pub entry_index: usize,
    pub view_event: ViewEvent,
}

pub enum ViewEvent {
    NewDiff(acp::ToolCallId),
    NewTerminal(acp::ToolCallId),
    TerminalMovedToBackground(acp::ToolCallId),
    MessageEditorEvent(Entity<MessageEditor>, MessageEditorEvent),
    OpenDiffLocation {
        path: String,
        position: Point,
        split: bool,
    },
}

/// `OMEGA-DELTA-0124`. The thinking blocks of one assistant message, lifted out
/// of the thread so the entry can be synced without holding a borrow of it.
///
/// Syncing needs `&mut App` — a split thought's body is a second `Markdown`
/// entity — and the message is read through the very `App` it would borrow.
/// `AssistantMessageChunk` is not `Clone`, so this carries the two facts the
/// sync actually uses.
pub struct MessageThoughts {
    chunk_count: usize,
    last_chunk_is_thought: bool,
    /// Each thinking block's own markdown, by chunk index.
    thoughts: Vec<(usize, Entity<Markdown>)>,
}

impl MessageThoughts {
    fn new(message: &acp_thread::AssistantMessage) -> Self {
        Self {
            chunk_count: message.chunks.len(),
            last_chunk_is_thought: matches!(
                message.chunks.last(),
                Some(AssistantMessageChunk::Thought { .. })
            ),
            thoughts: message
                .chunks
                .iter()
                .enumerate()
                .filter_map(|(ix, chunk)| match chunk {
                    AssistantMessageChunk::Thought { block, .. } => {
                        Some((ix, block.markdown()?.clone()))
                    }
                    AssistantMessageChunk::Message { .. } => None,
                })
                .collect(),
        }
    }
}

/// `OMEGA-DELTA-0124`. One thinking block as the view draws it: the muted lines
/// its header shows, and a body with those lines taken out.
///
/// **This is a cache, and it is why the split is done here and not in the
/// renderer.** A thought streams in token by token, and its markdown is
/// re-derived on every arriving chunk — but only on an arriving chunk.
/// `render_thinking_block` runs on every frame and holds `&self`, so it can
/// neither build an entity nor memoise one; doing the work there would
/// construct a `Markdown` per frame per visible thought. `source` is the text
/// this was derived from, and a sync that finds it unchanged does nothing.
#[derive(Debug)]
pub struct ThoughtView {
    source: SharedString,
    headings: Vec<SharedString>,
    body: Option<Entity<Markdown>>,
}

impl ThoughtView {
    /// The muted lines the header draws, one per thought. Never empty.
    pub fn headings(&self) -> &[SharedString] {
        &self.headings
    }

    /// The source under the header, or `None` when the block is titles alone —
    /// which is what a thought looks like for the moment between its title
    /// arriving and its first sentence.
    pub fn body(&self) -> Option<&Entity<Markdown>> {
        self.body.as_ref()
    }
}

#[derive(Debug)]
pub struct AssistantMessageEntry {
    scroll_handles_by_chunk_index: HashMap<usize, ScrollHandle>,
    thoughts_by_chunk_index: HashMap<usize, ThoughtView>,
    focus_handle: FocusHandle,
}

impl AssistantMessageEntry {
    pub fn scroll_handle_for_chunk(&self, ix: usize) -> Option<ScrollHandle> {
        self.scroll_handles_by_chunk_index.get(&ix).cloned()
    }

    pub fn thought_for_chunk(&self, ix: usize) -> Option<&ThoughtView> {
        self.thoughts_by_chunk_index.get(&ix)
    }

    pub fn sync(
        &mut self,
        message: MessageThoughts,
        language_registry: Option<Arc<LanguageRegistry>>,
        cx: &mut App,
    ) {
        if message.last_chunk_is_thought {
            let ix = message.chunk_count - 1;
            let handle = self.scroll_handles_by_chunk_index.entry(ix).or_default();
            handle.scroll_to_bottom();
        }

        // `OMEGA-DELTA-0124`. Re-split each thought whose text moved, and only
        // those. A turn holds every thought it has already finished, and their
        // sources stop changing the moment the next one starts.
        self.thoughts_by_chunk_index.retain(|ix, _| {
            message
                .thoughts
                .iter()
                .any(|(thought_ix, _)| thought_ix == ix)
        });
        for (ix, block) in message.thoughts {
            let source = block.read(cx).source().clone();
            if self
                .thoughts_by_chunk_index
                .get(&ix)
                .is_some_and(|thought| thought.source == source)
            {
                continue;
            }

            let split = split_thought(&source);
            let headings = split.headings();
            let body = if split.body.is_empty() {
                None
            } else if let Some(body) = self
                .thoughts_by_chunk_index
                .get(&ix)
                .and_then(|thought| thought.body.clone())
            {
                body.update(cx, |body, cx| body.replace(split.body, cx));
                Some(body)
            } else {
                // The same options the thought's own markdown was built with in
                // `acp_thread`, so the body renders as it did when it was one
                // entity rather than two.
                Some(cx.new(|cx| {
                    Markdown::new_with_options(
                        split.body.into(),
                        language_registry.clone(),
                        None,
                        MarkdownOptions {
                            render_mermaid_diagrams: true,
                            render_metadata_blocks: true,
                            ..Default::default()
                        },
                        cx,
                    )
                }))
            };

            self.thoughts_by_chunk_index.insert(
                ix,
                ThoughtView {
                    source,
                    headings,
                    body,
                },
            );
        }
    }
}

#[derive(Debug)]
pub struct ToolCallEntry {
    content: HashMap<EntityId, AnyEntity>,
    focus_handle: FocusHandle,
}

#[derive(Debug)]
pub enum Entry {
    UserMessage(Entity<MessageEditor>),
    AssistantMessage(AssistantMessageEntry),
    ToolCall(ToolCallEntry),
    Elicitation {
        focus_handle: FocusHandle,
    },
    CompletedPlan,
    ContextCompaction,
    /// OMEGA-DELTA-0045. The view side of [`AgentThreadEntry::SystemNote`].
    SystemNote,
}

impl Entry {
    pub fn focus_handle(&self, cx: &App) -> Option<FocusHandle> {
        match self {
            Self::UserMessage(editor) => Some(editor.read(cx).focus_handle(cx)),
            Self::AssistantMessage(message) => Some(message.focus_handle.clone()),
            Self::ToolCall(tool_call) => Some(tool_call.focus_handle.clone()),
            Self::Elicitation { focus_handle } => Some(focus_handle.clone()),
            Self::CompletedPlan | Self::ContextCompaction | Self::SystemNote => None,
        }
    }

    pub fn message_editor(&self) -> Option<&Entity<MessageEditor>> {
        match self {
            Self::UserMessage(editor) => Some(editor),
            Self::AssistantMessage(_)
            | Self::ToolCall(_)
            | Self::Elicitation { .. }
            | Self::CompletedPlan
            | Self::ContextCompaction
            | Self::SystemNote => None,
        }
    }

    pub fn editor_for_diff(&self, diff: &Entity<acp_thread::Diff>) -> Option<Entity<Editor>> {
        self.content_map()?
            .get(&diff.entity_id())
            .cloned()
            .and_then(|entity| entity.downcast::<Editor>().ok())
    }

    pub fn terminal(
        &self,
        terminal: &Entity<acp_thread::Terminal>,
    ) -> Option<Entity<TerminalView>> {
        self.content_map()?
            .get(&terminal.entity_id())
            .cloned()
            .and_then(|entity| entity.downcast::<TerminalView>().ok())
    }

    pub fn scroll_handle_for_assistant_message_chunk(
        &self,
        chunk_ix: usize,
    ) -> Option<ScrollHandle> {
        match self {
            Self::AssistantMessage(message) => message.scroll_handle_for_chunk(chunk_ix),
            Self::UserMessage(_)
            | Self::ToolCall(_)
            | Self::Elicitation { .. }
            | Self::CompletedPlan
            | Self::ContextCompaction
            | Self::SystemNote => None,
        }
    }

    /// `OMEGA-DELTA-0124`. The split of a thinking block: its header lines, and
    /// the body those lines have been taken out of.
    pub fn thought_for_assistant_message_chunk(&self, chunk_ix: usize) -> Option<&ThoughtView> {
        match self {
            Self::AssistantMessage(message) => message.thought_for_chunk(chunk_ix),
            Self::UserMessage(_)
            | Self::ToolCall(_)
            | Self::Elicitation { .. }
            | Self::CompletedPlan
            | Self::ContextCompaction
            | Self::SystemNote => None,
        }
    }

    fn content_map(&self) -> Option<&HashMap<EntityId, AnyEntity>> {
        match self {
            Self::ToolCall(ToolCallEntry { content, .. }) => Some(content),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn has_content(&self) -> bool {
        match self {
            Self::ToolCall(ToolCallEntry { content, .. }) => !content.is_empty(),
            Self::UserMessage(_)
            | Self::AssistantMessage(_)
            | Self::Elicitation { .. }
            | Self::CompletedPlan
            | Self::ContextCompaction
            | Self::SystemNote => false,
        }
    }
}

impl Focusable for ToolCallEntry {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Focusable for Entry {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Self::UserMessage(editor) => editor.read(cx).focus_handle(cx),
            Self::AssistantMessage(message) => message.focus_handle.clone(),
            Self::ToolCall(tool_call) => tool_call.focus_handle.clone(),
            Self::Elicitation { focus_handle } => focus_handle.clone(),
            Self::CompletedPlan | Self::ContextCompaction | Self::SystemNote => cx.focus_handle(),
        }
    }
}

fn create_terminal(
    workspace: WeakEntity<Workspace>,
    project: WeakEntity<Project>,
    terminal: Entity<acp_thread::Terminal>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TerminalView> {
    cx.new(|cx| {
        let mut view = TerminalView::new(
            terminal.read(cx).inner().clone(),
            workspace,
            None,
            project,
            window,
            cx,
        );
        view.set_embedded_mode(Some(1000), cx);
        // `OMEGA-DELTA-0080`. A tool result opens at a ceiling, not at its
        // natural height. The reader lifts it from the card footer.
        view.set_embedded_max_lines(Some(COLLAPSED_TOOL_OUTPUT_LINES), cx);
        view
    })
}

fn create_editor_diff(
    diff: Entity<acp_thread::Diff>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Editor> {
    cx.new(|cx| {
        let mut editor = Editor::new(
            EditorMode::Full {
                scale_ui_elements_with_buffer_font_size: false,
                show_active_line_background: false,
                sizing_behavior: SizingBehavior::SizeByContent,
            },
            diff.read(cx).multibuffer().clone(),
            None,
            window,
            cx,
        );
        editor.set_show_gutter(false, cx);
        editor.disable_diagnostics(cx);
        editor.set_max_diagnostics_severity(DiagnosticSeverity::Off, cx);
        editor.disable_expand_excerpt_buttons(cx);
        editor.set_show_vertical_scrollbar(false, cx);
        editor.set_minimap_visibility(MinimapVisibility::Disabled, window, cx);
        editor.set_soft_wrap_mode(SoftWrap::None, cx);
        editor.set_forbid_vertical_scroll(true);
        editor.set_show_indent_guides(false, cx);
        editor.set_read_only(true);
        editor.set_delegate_open_excerpts(true);
        editor.set_show_bookmarks(false, cx);
        editor.set_show_breakpoints(false, cx);
        editor.set_show_code_actions(false, cx);
        editor.set_show_git_diff_gutter(false, cx);
        editor.set_expand_all_diff_hunks(cx);
        editor.set_diff_hunk_delegate(Some(Arc::new(RestoreOnlyUnstagedDiffHunkDelegate)), cx);
        editor.set_text_style_refinement(diff_editor_text_style_refinement(cx));
        editor
    })
}

fn diff_editor_text_style_refinement(cx: &mut App) -> TextStyleRefinement {
    TextStyleRefinement {
        font_size: Some(
            TextSize::Small
                .rems(cx)
                .to_pixels(ThemeSettings::get_global(cx).agent_ui_font_size(cx))
                .into(),
        ),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::rc::Rc;
    use std::sync::Arc;

    use acp_thread::{AgentConnection, StubAgentConnection};
    use agent_client_protocol::schema::v1 as acp;
    use buffer_diff::{DiffHunkStatus, DiffHunkStatusKind};
    use editor::RowInfo;
    use fs::FakeFs;
    use gpui::{AppContext as _, TestAppContext};
    use parking_lot::RwLock;

    use crate::entry_view_state::{Entry, EntryViewState};
    use crate::message_editor::SessionCapabilities;
    use multi_buffer::MultiBufferRow;
    use pretty_assertions::assert_matches;
    use project::Project;
    use serde_json::json;
    use settings::SettingsStore;
    use ui::SharedString;
    use util::path;
    use workspace::{MultiWorkspace, PathList};

    /// `OMEGA-DELTA-0124`. Every thinking block this splitter has ever been
    /// shown, as text.
    ///
    /// One table, used by every check below, because the property that matters
    /// is a property of *all* of them: no title in a header is still in the
    /// body under it. Checking each case's shape separately is how the one case
    /// that duplicates gets added without anybody noticing.
    const THOUGHTS: &[&str] = &[
        // The ordinary one: a bold title, then prose.
        "**Planning plugin uninstall strategy**\n\nThe user wants the plugin gone.",
        // Two thoughts in one block. The owner saw this and asked for the
        // lightbulb line twice.
        "**Planning plugin uninstall strategy**\n\nThe user wants it gone.\n\n\
         **Searching for the config**\n\nIt is under `~/.config`.",
        // A heading rather than emphasis.
        "# Planning\n\nThe user wants the plugin gone.",
        "### Planning\n\nThe user wants the plugin gone.",
        // Mid-stream: the title has arrived, its closing marker has not.
        "**Search",
        "**Searching for the con",
        // Mid-stream, earlier still: the markers before any word.
        "**",
        "*",
        "",
        // Title and no prose yet.
        "**Planning plugin uninstall strategy**",
        // Prose with no title at all.
        "The user wants the plugin gone. I should look at the manifest first.",
        // A bold lead-in inside prose. Not a title: hoisting it would put half
        // a sentence in the header and delete it from the paragraph.
        "**Note** that the manifest is gone, so the uninstall cannot proceed.",
        // A `#` inside fenced code is a comment, not a heading.
        "**Checking the config**\n\n```bash\n# the config lives here\ncat ~/.config/x\n```",
        // A title after a fence closes is still a title.
        "```bash\n# not a heading\n```\n\n**Reading the manifest**\n\nIt is empty.",
        // Four spaces is an indented code block.
        "Consider:\n\n    # not a heading\n    cat x\n",
        // Not headings: seven hashes, and a hash with no space after it.
        "####### seven\n\n#nospace",
    ];

    /// `OMEGA-DELTA-0124`. **A title never appears in both the header and the
    /// body.**
    ///
    /// This is the check for the defect that shipped. The header was given the
    /// thought's title and the body kept rendering the same markdown, so every
    /// thought drew its title twice — muted above, bold below. The owner saw it
    /// on the first build.
    ///
    /// Stated as an idempotence: split a body that has already been split and
    /// there is no title left to find. Anything that puts a title in the header
    /// while leaving it in the body fails here, whatever route it took.
    ///
    /// It is the *splitter* that is re-run, not the line predicate. Asking each
    /// body line on its own was the first version of this check, and it failed
    /// on a `# comment` inside a fenced shell snippet — which is not a title,
    /// is not in any header, and must stay exactly where the model put it. A
    /// check that cannot tell those apart would have been satisfied by hoisting
    /// the comment out of the code.
    #[test]
    fn a_title_never_appears_in_both_the_header_and_the_body() {
        use super::split_thought;

        for source in THOUGHTS {
            let split = split_thought(source);

            let titles_left_in_body = split_thought(&split.body).titles;
            assert!(
                titles_left_in_body.is_empty(),
                "OMEGA-DELTA-0124: the body of {source:?} still holds title \
                 line(s) {titles_left_in_body:?}. The header draws the same \
                 words, so the thought renders its title twice — muted above \
                 and bold below. That is the defect this delta exists for.",
            );

            for title in &split.titles {
                assert!(
                    !split.body.contains(title.as_ref()),
                    "OMEGA-DELTA-0124: {title:?} is a header line of {source:?} \
                     and is still somewhere in the body under it.",
                );
            }

            // The preview is the *other* kind of header line, and the two must
            // stay distinguishable: a preview is prose that stays in the body,
            // so a title recorded as one would silently escape the check above.
            assert!(
                split.preview.is_none() || split.titles.is_empty(),
                "OMEGA-DELTA-0124: {source:?} produced both a title and a \
                 prose preview. A block is headed by one or the other.",
            );
        }
    }

    /// `OMEGA-DELTA-0124`. A row is never blank, whatever has arrived so far.
    #[test]
    fn a_thought_always_has_a_heading() {
        use super::{UNTITLED_THOUGHT_HEADING, split_thought};

        for source in THOUGHTS {
            let headings = split_thought(source).headings();
            assert!(
                !headings.is_empty() && headings.iter().all(|heading| !heading.trim().is_empty()),
                "OMEGA-DELTA-0124: {source:?} headed a thinking block with \
                 nothing. Mid-stream is exactly when somebody is looking at it.",
            );
        }

        // A thought that has arrived as markers alone has no words to show, and
        // falls all the way back to the word the header used to always say.
        assert_eq!(
            split_thought("**").headings(),
            vec![SharedString::new_static(UNTITLED_THOUGHT_HEADING)],
        );
        assert_eq!(
            split_thought("").headings(),
            vec![SharedString::new_static(UNTITLED_THOUGHT_HEADING)],
        );
    }

    /// `OMEGA-DELTA-0124`. What each shape of block actually splits into.
    #[test]
    fn a_thinking_block_splits_into_its_titles_and_the_rest() {
        use super::split_thought;

        let one =
            split_thought("**Planning plugin uninstall strategy**\n\nThe user wants it gone.");
        assert_eq!(one.titles, vec!["Planning plugin uninstall strategy"]);
        assert_eq!(one.body, "The user wants it gone.");
        assert_eq!(one.preview, None);

        // Two thoughts, two rows, one body with neither title in it.
        let two = split_thought(
            "**Planning the uninstall**\n\nThe user wants it gone.\n\n\
             **Searching for the config**\n\nIt is under `~/.config`.",
        );
        assert_eq!(
            two.titles,
            vec!["Planning the uninstall", "Searching for the config"],
        );
        assert_eq!(
            two.body,
            "The user wants it gone.\n\nIt is under `~/.config`.",
        );

        // Emphasis marks a title, not position: the second thought is the
        // second *emphasised* line, not the second line.
        let interleaved = split_thought("Some prose first.\n\n**A title after it**\n\nMore prose.");
        assert_eq!(interleaved.titles, vec!["A title after it"]);
        assert_eq!(interleaved.body, "Some prose first.\n\nMore prose.");

        // A title still arriving has no closing marker, and renders anyway.
        assert_eq!(split_thought("**Search").titles, vec!["Search"]);
        assert_eq!(split_thought("**Search").body, "");

        // Headings count too.
        assert_eq!(
            split_thought("# Planning\n\nProse.").titles,
            vec!["Planning"]
        );
        assert_eq!(
            split_thought("### Planning ###\n\nProse.").titles,
            vec!["Planning"],
        );

        // A block with no title keeps every word it has, and previews the first
        // line rather than eating it.
        let untitled =
            split_thought("The user wants the plugin gone.\nI will look at the manifest.");
        assert!(untitled.titles.is_empty());
        assert_eq!(
            untitled.preview.as_deref(),
            Some("The user wants the plugin gone."),
        );
        assert_eq!(
            untitled.body,
            "The user wants the plugin gone.\nI will look at the manifest.",
        );

        // A bold lead-in is prose. It keeps its whole sentence.
        let lead_in = split_thought("**Note** that the manifest is gone.");
        assert!(lead_in.titles.is_empty());
        assert_eq!(lead_in.body, "**Note** that the manifest is gone.");
    }

    /// `OMEGA-DELTA-0124`. Fenced and indented code is not prose, and nothing
    /// is lifted out of it.
    #[test]
    fn a_hash_inside_code_is_a_comment_and_stays_where_it_is() {
        use super::split_thought;

        let fenced = split_thought(
            "**Checking the config**\n\n```bash\n# the config lives here\ncat x\n```",
        );
        assert_eq!(fenced.titles, vec!["Checking the config"]);
        assert_eq!(
            fenced.body, "```bash\n# the config lives here\ncat x\n```",
            "a shell comment was taken for a heading and hoisted out of the code",
        );

        let indented = split_thought("Consider:\n\n    # not a heading\n    cat x");
        assert!(indented.titles.is_empty());
        assert_eq!(indented.body, "Consider:\n\n    # not a heading\n    cat x");

        // A blank line inside a fence is part of the code and survives the
        // collapsing that removing a title leaves behind.
        let blank_in_fence = split_thought("**T**\n\n```\na\n\n\nb\n```");
        assert_eq!(blank_in_fence.body, "```\na\n\n\nb\n```");
    }

    /// `OMEGA-DELTA-0080`. The control appears only when it has something to
    /// offer, and it says how much.
    #[test]
    fn test_tool_output_ceiling_label() {
        use super::{COLLAPSED_TOOL_OUTPUT_LINES, tool_output_ceiling_label};

        // A short result is whole on screen, so no control is drawn — capped or
        // not, the ceiling never bound.
        assert_eq!(tool_output_ceiling_label(2, 2, true, None), None);
        assert_eq!(tool_output_ceiling_label(2, 2, false, None), None);
        assert_eq!(
            tool_output_ceiling_label(
                COLLAPSED_TOOL_OUTPUT_LINES,
                COLLAPSED_TOOL_OUTPUT_LINES,
                true,
                None
            ),
            None
        );

        // A capped result names the exact count it is hiding.
        assert_eq!(
            tool_output_ceiling_label(40, COLLAPSED_TOOL_OUTPUT_LINES, true, None),
            Some("Show 24 more lines".into())
        );
        assert_eq!(
            tool_output_ceiling_label(COLLAPSED_TOOL_OUTPUT_LINES + 1, 16, true, None),
            Some("Show 1 more line".into())
        );

        // With the ceiling lifted, the control offers the way back — and only
        // for a result the ceiling would have bound.
        assert_eq!(
            tool_output_ceiling_label(40, 40, false, None),
            Some("Show fewer lines".into())
        );
        assert_eq!(
            tool_output_ceiling_label(COLLAPSED_TOOL_OUTPUT_LINES, 16, false, None),
            None
        );

        // A record that holds no more than this surface changes nothing. This
        // is the whole of today's terminal-backed path, and it must read
        // exactly as it did before `OMEGA-DELTA-0103`.
        assert_eq!(
            tool_output_ceiling_label(40, COLLAPSED_TOOL_OUTPUT_LINES, true, Some(40)),
            Some("Show 24 more lines".into())
        );
        assert_eq!(tool_output_ceiling_label(2, 2, true, Some(2)), None);
        assert_eq!(tool_output_ceiling_label(2, 2, true, Some(1)), None);
    }

    /// `OMEGA-DELTA-0103`. The falsifier. Strip the record's total from the
    /// label and a body that is a preview reads exactly like a complete one.
    #[test]
    fn test_tool_output_ceiling_label_names_what_the_record_withheld() {
        use super::{
            COLLAPSED_TOOL_OUTPUT_LINES, tool_output_ceiling_is_toggleable,
            tool_output_ceiling_label,
        };

        // The owner's case, bounded twice: forty lines reached this surface out
        // of four hundred in the record, and sixteen of the forty are on
        // screen. Both remainders are named, and neither is the other.
        let capped = tool_output_ceiling_label(40, COLLAPSED_TOOL_OUTPUT_LINES, true, Some(400))
            .expect("a doubly-bounded body draws a control");
        assert_eq!(capped, "Show 24 more lines · 360 more withheld");
        assert_ne!(
            Some(capped),
            tool_output_ceiling_label(40, COLLAPSED_TOOL_OUTPUT_LINES, true, None),
            "a preview and a complete result of the same height read the same"
        );

        // Lifted, the reader can reach the last line on screen — which is
        // exactly when being told the record holds more matters most.
        assert_eq!(
            tool_output_ceiling_label(40, 40, false, Some(400)),
            Some("Show fewer lines · 360 more withheld".into())
        );

        // A body short enough that the ceiling never bound it still has to say
        // so, and the control that carries the sentence must not claim to open
        // anything.
        assert_eq!(
            tool_output_ceiling_label(3, 3, true, Some(400)),
            Some("397 more lines are withheld from this result".into())
        );
        assert!(!tool_output_ceiling_is_toggleable(3, 3, true));
        assert_eq!(
            tool_output_ceiling_label(3, 3, true, Some(4)),
            Some("1 more line is withheld from this result".into())
        );
        assert!(tool_output_ceiling_is_toggleable(
            40,
            COLLAPSED_TOOL_OUTPUT_LINES,
            true
        ));
        assert!(tool_output_ceiling_is_toggleable(40, 40, false));
        assert!(!tool_output_ceiling_is_toggleable(40, 40, true));
    }

    #[test]
    fn test_reindex_after_removal() {
        use super::reindex_after_removal;

        // Entries before the removed range keep their index.
        assert_eq!(reindex_after_removal(0, &(2..4)), Some(0));
        assert_eq!(reindex_after_removal(1, &(2..4)), Some(1));
        // Entries inside the removed range are dropped.
        assert_eq!(reindex_after_removal(2, &(2..4)), None);
        assert_eq!(reindex_after_removal(3, &(2..4)), None);
        // Entries after the removed range slide down by its length.
        assert_eq!(reindex_after_removal(4, &(2..4)), Some(2));
        assert_eq!(reindex_after_removal(5, &(2..4)), Some(3));
        // An empty removal range leaves indices untouched.
        assert_eq!(reindex_after_removal(3, &(2..2)), Some(3));
    }

    /// The subagent card's expansion is a door, and a door opens both ways.
    ///
    /// The card drew its close control only on hover and its always-visible
    /// strip offered only "full screen", so a reader who opened a card had no
    /// drawn way to shut it. The state underneath was always reversible; the
    /// defect was that nothing reached this. Both controls the card now draws
    /// land here, so the round trip is checked where it is decided.
    #[gpui::test]
    async fn a_subagent_card_closes_by_the_same_state_that_opened_it(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let view_state = cx.new(|_cx| {
            EntryViewState::new(
                workspace.downgrade(),
                project.downgrade(),
                None,
                Arc::new(RwLock::new(SessionCapabilities::default())),
                "Test Agent".into(),
            )
        });

        let opened = acp::ToolCallId::new("subagent-card");
        let other = acp::ToolCallId::new("another-card");

        view_state.update(cx, |view_state, _cx| {
            assert!(!view_state.is_tool_call_expanded(&opened));

            // The header chevron.
            view_state.toggle_tool_call_expansion(&opened);
            assert!(view_state.is_tool_call_expanded(&opened));

            // The strip along the bottom of the open card, which only ever
            // closes — a second press of a toggle would have re-opened it.
            view_state.collapse_tool_call(&opened);
            assert!(!view_state.is_tool_call_expanded(&opened));
            view_state.collapse_tool_call(&opened);
            assert!(!view_state.is_tool_call_expanded(&opened));

            // And the round trip returns to the same state rather than to a
            // fresh one, for every card independently: closing one card is not
            // an opinion about any other.
            view_state.toggle_tool_call_expansion(&opened);
            view_state.toggle_tool_call_expansion(&other);
            view_state.collapse_tool_call(&other);
            assert!(view_state.is_tool_call_expanded(&opened));
            assert!(!view_state.is_tool_call_expanded(&other));
        });
    }

    #[gpui::test(iterations = 8)]
    async fn test_burst_sync_materializes_every_missing_entry(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({})).await;
        let project = Project::test(fs, [Path::new(path!("/project"))], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |workspace, _| workspace.workspace().clone());
        let connection = Rc::new(StubAgentConnection::new());
        let thread = cx
            .update(|_, cx| {
                connection.clone().new_session(
                    project.clone(),
                    PathList::new(&[Path::new(path!("/project"))]),
                    cx,
                )
            })
            .await
            .expect("test session should start");
        let session_id = thread.read_with(cx, |thread, _| thread.session_id().clone());

        cx.update(|_, cx| {
            for index in 0..4 {
                connection.send_update(
                    session_id.clone(),
                    acp::SessionUpdate::ToolCall(acp::ToolCall::new(
                        format!("tool-{index}"),
                        format!("Tool {index}"),
                    )),
                    cx,
                );
            }
        });
        assert_eq!(thread.read_with(cx, |thread, _| thread.entries().len()), 4);

        let view_state = cx.new(|_| {
            EntryViewState::new(
                workspace.downgrade(),
                project.downgrade(),
                None,
                Arc::new(RwLock::new(SessionCapabilities::default())),
                "Test Agent".into(),
            )
        });
        view_state.update_in(cx, |view_state, window, cx| {
            assert_eq!(view_state.sync_missing_entries(&thread, window, cx), 0..4);
            assert!((0..4).all(|index| view_state.entry(index).is_some()));
            assert_eq!(view_state.sync_missing_entries(&thread, window, cx), 4..4);
        });
    }

    #[gpui::test]
    async fn test_diff_sync(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            "/project",
            json!({
                "hello.txt": "hi world"
            }),
        )
        .await;
        let project = Project::test(fs, [Path::new(path!("/project"))], cx).await;

        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let tool_call = acp::ToolCall::new("tool", "Tool call")
            .status(acp::ToolCallStatus::InProgress)
            .content(vec![acp::ToolCallContent::Diff(
                acp::Diff::new("/project/hello.txt", "hello world").old_text("hi world"),
            )]);
        let connection = Rc::new(StubAgentConnection::new());
        let thread = cx
            .update(|_, cx| {
                connection.clone().new_session(
                    project.clone(),
                    PathList::new(&[Path::new(path!("/project"))]),
                    cx,
                )
            })
            .await
            .unwrap();
        let session_id = thread.update(cx, |thread, _| thread.session_id().clone());

        cx.update(|_, cx| {
            connection.send_update(session_id, acp::SessionUpdate::ToolCall(tool_call), cx)
        });

        let thread_store = None;

        let view_state = cx.new(|_cx| {
            EntryViewState::new(
                workspace.downgrade(),
                project.downgrade(),
                thread_store,
                Arc::new(RwLock::new(SessionCapabilities::default())),
                "Test Agent".into(),
            )
        });

        view_state.update_in(cx, |view_state, window, cx| {
            view_state.sync_entry(0, &thread, window, cx)
        });

        let diff = thread.read_with(cx, |thread, _| {
            thread
                .entries()
                .get(0)
                .unwrap()
                .diffs()
                .next()
                .unwrap()
                .clone()
        });

        cx.run_until_parked();

        let diff_editor = view_state.read_with(cx, |view_state, _cx| {
            view_state.entry(0).unwrap().editor_for_diff(&diff).unwrap()
        });
        assert_eq!(
            diff_editor.read_with(cx, |editor, cx| editor.text(cx)),
            "hi world\nhello world"
        );
        let row_infos = diff_editor.read_with(cx, |editor, cx| {
            let multibuffer = editor.buffer().read(cx);
            multibuffer
                .snapshot(cx)
                .row_infos(MultiBufferRow(0))
                .collect::<Vec<_>>()
        });
        assert_matches!(
            row_infos.as_slice(),
            [
                RowInfo {
                    multibuffer_row: Some(MultiBufferRow(0)),
                    diff_status: Some(DiffHunkStatus {
                        kind: DiffHunkStatusKind::Deleted,
                        ..
                    }),
                    ..
                },
                RowInfo {
                    multibuffer_row: Some(MultiBufferRow(1)),
                    diff_status: Some(DiffHunkStatus {
                        kind: DiffHunkStatusKind::Added,
                        ..
                    }),
                    ..
                }
            ]
        );
    }

    #[gpui::test]
    async fn test_elicitation_preserves_entry_index(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({})).await;
        let project = Project::test(fs, [Path::new(path!("/project"))], cx).await;

        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let connection = Rc::new(StubAgentConnection::new());
        let thread = cx
            .update(|_, cx| {
                connection.clone().new_session(
                    project.clone(),
                    PathList::new(&[Path::new(path!("/project"))]),
                    cx,
                )
            })
            .await
            .unwrap();
        let session_id = thread.update(cx, |thread, _| thread.session_id().clone());

        let _response_task = thread.update(cx, |thread, cx| {
            thread
                .request_elicitation(
                    acp::CreateElicitationRequest::new(
                        acp::ElicitationFormMode::new(
                            acp::ElicitationSessionScope::new(session_id.clone()),
                            acp::ElicitationSchema::new().string("name", true),
                        ),
                        "Provide a name",
                    ),
                    cx,
                )
                .unwrap()
        });
        cx.update(|_, cx| {
            connection.send_update(
                session_id,
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    acp::ContentBlock::Text(acp::TextContent::new("hello")),
                )),
                cx,
            );
        });

        let view_state = cx.new(|_cx| {
            EntryViewState::new(
                workspace.downgrade(),
                project.downgrade(),
                None,
                Arc::new(RwLock::new(SessionCapabilities::default())),
                "Test Agent".into(),
            )
        });

        view_state.update_in(cx, |view_state, window, cx| {
            view_state.sync_entry(0, &thread, window, cx);
            view_state.sync_entry(1, &thread, window, cx);
        });

        view_state.read_with(cx, |view_state, _cx| {
            assert!(matches!(
                view_state.entry(0),
                Some(Entry::Elicitation { .. })
            ));
            assert!(matches!(
                view_state.entry(1),
                Some(Entry::AssistantMessage(_))
            ));
        });
    }

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut settings_store = SettingsStore::test(cx);
            settings_store.register_setting::<feature_flags::FeatureFlagsSettings>();
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
        });
    }
}
