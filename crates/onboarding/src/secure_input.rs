use std::{fmt, ops::Range};

use gpui::{
    App, Bounds, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement,
    KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, Render, Role, ShapedLine, SharedString, Style, TextRun, UTF16Selection,
    UnderlineStyle, Window, div, fill, point, px, relative, size,
};
use ui::prelude::*;
use unicode_segmentation::UnicodeSegmentation;
use zeroize::{Zeroize, Zeroizing};

const MAX_SECRET_BYTES: usize = 1024;
const MASK_CHARACTER: char = '\u{2022}';

pub(crate) struct SecureInput {
    focus_handle: FocusHandle,
    content: Zeroizing<String>,
    placeholder: SharedString,
    aria_label: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    tab_index: isize,
    #[cfg(test)]
    drop_observer: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl SecureInput {
    pub(crate) fn new(
        placeholder: impl Into<SharedString>,
        aria_label: impl Into<SharedString>,
        tab_index: isize,
        cx: &mut App,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: Zeroizing::new(String::with_capacity(MAX_SECRET_BYTES)),
            placeholder: placeholder.into(),
            aria_label: aria_label.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            tab_index,
            #[cfg(test)]
            drop_observer: None,
        }
    }

    pub(crate) fn take(&mut self, cx: &mut Context<Self>) -> String {
        let content =
            std::mem::replace(&mut *self.content, String::with_capacity(MAX_SECRET_BYTES));
        self.reset_interaction_state();
        cx.notify();
        content
    }

    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        self.content.zeroize();
        self.reset_interaction_state();
        cx.notify();
    }

    fn reset_interaction_state(&mut self) {
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        self.last_bounds = None;
        self.is_selecting = false;
    }

    fn replace_range(&mut self, range: Range<usize>, new_text: &str) -> bool {
        if range.start > range.end
            || range.end > self.content.len()
            || !self.content.is_char_boundary(range.start)
            || !self.content.is_char_boundary(range.end)
        {
            return false;
        }

        let Some(new_length) = self
            .content
            .len()
            .checked_sub(range.len())
            .and_then(|length| length.checked_add(new_text.len()))
        else {
            return false;
        };
        if new_length > MAX_SECRET_BYTES {
            return false;
        }

        self.content.replace_range(range.clone(), new_text);
        let cursor = range.start + new_text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        true
    }

    fn left(&mut self, select: bool, cx: &mut Context<Self>) {
        if select {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        } else if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, select: bool, cx: &mut Context<Self>) {
        if select {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        } else if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn backspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let previous = self.previous_boundary(self.cursor_offset());
            if previous == self.cursor_offset() {
                window.play_system_bell();
                return;
            }
            self.select_to(previous, cx);
        }
        let selected_range = self.selected_range.clone();
        self.replace_range(selected_range, "");
        cx.notify();
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if next == self.cursor_offset() {
                window.play_system_bell();
                return;
            }
            self.select_to(next, cx);
        }
        let selected_range = self.selected_range.clone();
        self.replace_range(selected_range, "");
        cx.notify();
    }

    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let mut text = Zeroizing::new(text);
        text.retain(|character| character != '\n' && character != '\r');
        let selected_range = self.selected_range.clone();
        if !self.replace_range(selected_range, &text) {
            window.play_system_bell();
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        let handled = if modifiers.secondary() {
            match event.keystroke.key.as_str() {
                "a" => {
                    self.selected_range = 0..self.content.len();
                    self.selection_reversed = false;
                    cx.notify();
                    true
                }
                "v" => {
                    self.paste(window, cx);
                    true
                }
                // Consume these shortcuts so a parent action can never copy or cut the secret.
                "c" | "x" => true,
                _ => false,
            }
        } else {
            match event.keystroke.key.as_str() {
                "backspace" => {
                    self.backspace(window, cx);
                    true
                }
                "delete" => {
                    self.delete(window, cx);
                    true
                }
                "left" => {
                    self.left(modifiers.shift, cx);
                    true
                }
                "right" => {
                    self.right(modifiers.shift, cx);
                    true
                }
                "home" => {
                    if modifiers.shift {
                        self.select_to(0, cx);
                    } else {
                        self.move_to(0, cx);
                    }
                    true
                }
                "end" => {
                    if modifiers.shift {
                        self.select_to(self.content.len(), cx);
                    } else {
                        self.move_to(self.content.len(), cx);
                    }
                    true
                }
                _ => false,
            }
        };

        if handled {
            cx.stop_propagation();
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;
        let offset = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return self.content.len();
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        self.content_offset_for_display(line.closest_index_for_x(position.x - bounds.left()))
    }

    fn masked_text(&self) -> SharedString {
        std::iter::repeat_n(MASK_CHARACTER, self.content.chars().count())
            .collect::<String>()
            .into()
    }

    fn display_offset_for_content(&self, offset: usize) -> usize {
        self.content[..offset].chars().count() * MASK_CHARACTER.len_utf8()
    }

    fn content_offset_for_display(&self, offset: usize) -> usize {
        let character_index = offset / MASK_CHARACTER.len_utf8();
        self.content
            .char_indices()
            .nth(character_index)
            .map_or(self.content.len(), |(index, _)| index)
    }

    fn offset_from_utf16_in(text: &str, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_offset = 0;
        for character in text.chars() {
            if utf16_offset >= offset {
                break;
            }
            utf16_offset += character.len_utf16();
            utf8_offset += character.len_utf8();
        }
        utf8_offset
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        Self::offset_from_utf16_in(&self.content, offset)
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset].encode_utf16().count()
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn masked_text_for_range(&self, range: Range<usize>) -> Option<String> {
        let text = self.content.get(range)?;
        Some(std::iter::repeat_n(MASK_CHARACTER, text.encode_utf16().count()).collect::<String>())
    }
}

impl fmt::Debug for SecureInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureInput")
            .field("content", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl Drop for SecureInput {
    fn drop(&mut self) {
        self.content.zeroize();
        #[cfg(test)]
        if let Some(observer) = self.drop_observer.as_ref() {
            observer.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

impl EntityInputHandler for SecureInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        self.masked_text_for_range(range)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        if !self.replace_range(range, new_text) {
            window.play_system_bell();
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let start = range.start;
        if !self.replace_range(range, new_text) {
            window.play_system_bell();
            return;
        }

        self.marked_range = (!new_text.is_empty()).then_some(start..start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .map(|range| {
                let selected_start = Self::offset_from_utf16_in(new_text, range.start);
                let selected_end = Self::offset_from_utf16_in(new_text, range.end);
                start + selected_start..start + selected_end
            })
            .unwrap_or_else(|| {
                let end = start + new_text.len();
                end..end
            });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let start = self.display_offset_for_content(range.start);
        let end = self.display_offset_for_content(range.end);
        let line = self.last_layout.as_ref()?;
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(start), bounds.top()),
            point(bounds.left() + line.x_for_index(end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let local_point = bounds.localize(&point)?;
        let line = self.last_layout.as_ref()?;
        let display_index = line.index_for_x(point.x - local_point.x)?;
        Some(self.offset_to_utf16(self.content_offset_for_display(display_index)))
    }
}

struct SecureInputElement {
    input: Entity<SecureInput>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for SecureInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SecureInputElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let text_style = window.text_style();
        let is_placeholder = input.content.is_empty();
        let display_text = if is_placeholder {
            input.placeholder.clone()
        } else {
            input.masked_text()
        };
        let text_color = if is_placeholder {
            cx.theme().colors().text_muted
        } else {
            text_style.color
        };
        let base_run = TextRun {
            len: display_text.len(),
            font: text_style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked_range) = input.marked_range.as_ref() {
            let marked_start = input.display_offset_for_content(marked_range.start);
            let marked_end = input.display_offset_for_content(marked_range.end);
            vec![
                TextRun {
                    len: marked_start,
                    ..base_run.clone()
                },
                TextRun {
                    len: marked_end - marked_start,
                    underline: Some(UnderlineStyle {
                        color: Some(base_run.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..base_run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_end,
                    ..base_run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![base_run]
        };
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);
        let display_cursor = input.display_offset_for_content(cursor);
        let cursor_position = line.x_for_index(display_cursor);
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_position, bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    cx.theme().colors().text_accent,
                )),
            )
        } else {
            let selection_start = input.display_offset_for_content(selected_range.start);
            let selection_end = input.display_offset_for_content(selected_range.end);
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(selection_start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selection_end),
                            bounds.bottom(),
                        ),
                    ),
                    cx.theme().colors().element_selected,
                )),
                None,
            )
        };

        PrepaintState {
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        if let Some(line) = prepaint.line.take() {
            if let Err(error) = line.paint(
                bounds.origin,
                window.line_height(),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            ) {
                zlog::error!("failed to paint secure input: {error}");
            }
            self.input.update(cx, |input, _| {
                input.last_layout = Some(line);
                input.last_bounds = Some(bounds);
            });
        }
        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }
    }
}

impl Render for SecureInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let focus_handle = self
            .focus_handle
            .clone()
            .tab_index(self.tab_index)
            .tab_stop(true);
        div()
            .id(("secure-input", cx.entity_id()))
            .role(Role::PasswordInput)
            .aria_label(self.aria_label.clone())
            .aria_placeholder(self.placeholder.clone())
            .track_focus(&focus_handle)
            .key_context("SecureInput")
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .cursor(CursorStyle::IBeam)
            .h_8()
            .w_full()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.editor_background)
            .px_2()
            .py_1()
            .focus(|style| style.border_color(colors.border_focused))
            .child(SecureInputElement { input: cx.entity() })
    }
}

impl Focusable for SecureInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn enforces_byte_cap(cx: &mut TestAppContext) {
        let mut input = cx.update(|cx| SecureInput::new("Secret", "Secret value", 0, cx));
        assert!(input.replace_range(0..0, &"a".repeat(MAX_SECRET_BYTES)));
        assert!(!input.replace_range(MAX_SECRET_BYTES..MAX_SECRET_BYTES, "b"));
        assert_eq!(input.content.len(), MAX_SECRET_BYTES);
    }

    #[gpui::test]
    fn take_replaces_storage_with_empty_preallocated_buffer(cx: &mut TestAppContext) {
        let input = cx.update(|cx| cx.new(|cx| SecureInput::new("Secret", "Secret value", 0, cx)));
        cx.update(|cx| {
            input.update(cx, |input, _| {
                assert!(input.replace_range(0..0, "correct horse battery staple"));
            });
        });
        let mut taken = cx.update(|cx| input.update(cx, SecureInput::take));

        assert_eq!(taken, "correct horse battery staple");
        cx.update(|cx| {
            let input = input.read(cx);
            assert!(input.content.is_empty());
            assert!(input.content.capacity() >= MAX_SECRET_BYTES);
        });
        taken.zeroize();
    }

    #[gpui::test]
    fn clear_zeroizes_and_reuses_storage(cx: &mut TestAppContext) {
        let input = cx.update(|cx| cx.new(|cx| SecureInput::new("Secret", "Secret value", 0, cx)));
        cx.update(|cx| {
            input.update(cx, |input, _| {
                assert!(input.replace_range(0..0, "sensitive"));
            });
        });
        cx.update(|cx| input.update(cx, SecureInput::clear));

        cx.update(|cx| {
            let input = input.read(cx);
            assert!(input.content.is_empty());
            assert!(input.content.capacity() >= MAX_SECRET_BYTES);
            assert_eq!(input.selected_range, 0..0);
        });
    }

    #[gpui::test]
    fn drop_runs_zeroizing_destructor(cx: &mut TestAppContext) {
        let dropped = Arc::new(AtomicBool::new(false));
        let mut input = cx.update(|cx| SecureInput::new("Secret", "Secret value", 0, cx));
        input.drop_observer = Some(dropped.clone());
        assert!(input.replace_range(0..0, "sensitive"));

        drop(input);

        assert!(dropped.load(Ordering::SeqCst));
    }

    #[gpui::test]
    fn debug_output_is_redacted(cx: &mut TestAppContext) {
        let mut input = cx.update(|cx| SecureInput::new("Secret", "Secret value", 0, cx));
        assert!(input.replace_range(0..0, "never print this"));
        let debug = format!("{input:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("never print this"));
    }

    #[gpui::test]
    fn input_handler_ranges_return_only_utf16_aligned_masks(cx: &mut TestAppContext) {
        let mut input = cx.update(|cx| SecureInput::new("Secret", "Secret value", 0, cx));
        assert!(input.replace_range(0..0, "a🔑b"));

        let masked = input
            .masked_text_for_range(1..5)
            .expect("valid secret range");

        assert_eq!(masked, "••");
        assert!(!masked.contains('🔑'));
    }
}
