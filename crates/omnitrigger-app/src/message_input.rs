//! 单行消息输入：文本由平台输入法提交，键绑定仅处理编辑命令。

use std::{collections::VecDeque, ops::Range};

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgba, size, App, Bounds, ClipboardItem,
    ContentMask, Context, CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ShapedLine, SharedString, Style, TextRun,
    UTF16Selection, UnderlineStyle, Window,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::{self, rgb};

const MAX_CHARACTERS: usize = 2000;
const EDIT_HISTORY_LIMIT: usize = 32;
const KEY_CONTEXT: &str = "OmniTriggerMessageInput";

actions!(
    omnitrigger_message_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        PreviousWord,
        NextWord,
        SelectPreviousWord,
        SelectNextWord,
        DeletePreviousWord,
        DeleteNextWord,
        Home,
        End,
        SelectHome,
        SelectEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some(KEY_CONTEXT)),
        KeyBinding::new("delete", Delete, Some(KEY_CONTEXT)),
        KeyBinding::new("left", Left, Some(KEY_CONTEXT)),
        KeyBinding::new("right", Right, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-left", PreviousWord, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-right", NextWord, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-left", SelectPreviousWord, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-right", SelectNextWord, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-backspace", DeletePreviousWord, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-delete", DeleteNextWord, Some(KEY_CONTEXT)),
        KeyBinding::new("home", Home, Some(KEY_CONTEXT)),
        KeyBinding::new("end", End, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-home", SelectHome, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-end", SelectEnd, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-c", Copy, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-x", Cut, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(KEY_CONTEXT)),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextMode {
    SingleLine,
    Json,
    PreserveWhitespace,
}

#[derive(Clone, Copy)]
pub(super) struct MessageSelection {
    anchor: usize,
    head: usize,
}

struct EditSnapshot {
    text: SharedString,
    selection: MessageSelection,
}

#[derive(Default)]
struct EditHistory {
    undo: VecDeque<EditSnapshot>,
    redo: Vec<EditSnapshot>,
    pending: Option<EditSnapshot>,
}

pub struct MessageInput {
    character_limit: usize,
    mode: TextMode,
    focus: FocusHandle,
    content: SharedString,
    edit_revision: u64,
    edit_history: Option<EditHistory>,
    placeholder: SharedString,
    anchor: usize,
    head: usize,
    marked: Option<Range<usize>>,
    dragging: bool,
    layout: Option<InputLayout>,
    bounds: Option<Bounds<Pixels>>,
    scroll_x: Pixels,
    blur_subscription: Option<gpui::Subscription>,
    embedded: bool,
    pass_empty_arrows: bool,
}

impl MessageInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            character_limit: MAX_CHARACTERS,
            mode: TextMode::SingleLine,
            focus: cx.focus_handle(),
            content: SharedString::default(),
            edit_revision: 0,
            edit_history: None,
            placeholder: "触发时显示的文字".into(),
            anchor: 0,
            head: 0,
            marked: None,
            dragging: false,
            layout: None,
            bounds: None,
            scroll_x: px(0.),
            blur_subscription: None,
            embedded: false,
            pass_empty_arrows: false,
        }
    }

    /// 去掉自带边框、底色与内边距，由外层容器绘制输入框外观。
    pub fn embedded(mut self) -> Self {
        self.embedded = true;
        self
    }

    /// 内容为空时不消费左右键，交给外层处理选择。
    pub fn pass_arrows_when_empty(mut self) -> Self {
        self.pass_empty_arrows = true;
        self
    }

    pub fn text(&self) -> String {
        self.content.to_string()
    }

    pub(super) fn text_ref(&self) -> &str {
        &self.content
    }

    pub(super) fn edit_revision(&self) -> u64 {
        self.edit_revision
    }

    pub(super) fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    pub fn set_placeholder(&mut self, text: impl Into<SharedString>) {
        self.placeholder = text.into();
    }

    pub fn with_character_limit(mut self, limit: usize) -> Self {
        self.character_limit = limit;
        self
    }

    /// JSON 字符串中的 Unicode 行分隔符必须转义，不能像提示文字一样改成空格。
    pub fn with_json_text(mut self) -> Self {
        self.mode = TextMode::Json;
        self
    }

    /// 保留换行和制表符，绘制时在同一行展示对应符号。
    pub(super) fn with_preserved_whitespace(mut self) -> Self {
        self.mode = TextMode::PreserveWhitespace;
        self
    }

    pub(super) fn with_undo_history(mut self) -> Self {
        self.edit_history = Some(EditHistory::default());
        self
    }

    pub(super) fn replace_leading_text(
        &mut self,
        prefix: &str,
        replacement: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.content.starts_with(prefix) || self.is_composing() {
            return;
        }
        let selection = self.selection_position();
        self.replace_text_in_range(
            Some(0..prefix.encode_utf16().count()),
            replacement,
            window,
            cx,
        );
        let relocate = |offset: usize| offset.saturating_sub(prefix.len()) + replacement.len();
        self.restore_selection(
            MessageSelection {
                anchor: relocate(selection.anchor),
                head: relocate(selection.head),
            },
            cx,
        );
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        let text = if self.mode == TextMode::Json {
            json_line_separators(&text)
        } else {
            text.to_string()
        };
        self.content = prepare_input(&text, self.character_limit, self.mode).into();
        self.edit_revision = self.edit_revision.wrapping_add(1);
        if let Some(history) = &mut self.edit_history {
            *history = EditHistory::default();
        }
        // 载入已有消息时从开头显示，避免长消息只露出结尾。
        self.anchor = 0;
        self.head = 0;
        self.marked = None;
        self.dragging = false;
        self.layout = None;
        self.scroll_x = px(0.);
        cx.notify();
    }

    pub(super) fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub(super) fn selection_position(&self) -> MessageSelection {
        MessageSelection {
            anchor: self.anchor,
            head: self.head,
        }
    }

    fn edit_snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            text: self.content.clone(),
            selection: self.selection_position(),
        }
    }

    fn finish_edit(&mut self) {
        let Some(history) = &mut self.edit_history else {
            return;
        };
        let Some(previous) = history.pending.take() else {
            return;
        };
        if previous.text != self.content {
            history.redo.clear();
            if history.undo.len() == EDIT_HISTORY_LIMIT {
                history.undo.pop_front();
            }
            history.undo.push_back(previous);
        }
    }

    pub(super) fn step_edit_history(&mut self, redo: bool, cx: &mut Context<Self>) {
        if self.is_composing() {
            return;
        }
        let current = self.edit_snapshot();
        let Some(history) = &mut self.edit_history else {
            return;
        };
        if history.pending.is_some() {
            return;
        }
        let previous = if redo {
            history
                .redo
                .pop()
                .inspect(|_| history.undo.push_back(current))
        } else {
            history
                .undo
                .pop_back()
                .inspect(|_| history.redo.push(current))
        };
        if let Some(previous) = previous {
            self.content = previous.text;
            self.edit_revision = self.edit_revision.wrapping_add(1);
            self.layout = None;
            self.dragging = false;
            self.restore_selection(previous.selection, cx);
        }
    }

    pub(super) fn restore_selection(
        &mut self,
        selection: MessageSelection,
        cx: &mut Context<Self>,
    ) {
        if self.content.is_char_boundary(selection.anchor)
            && self.content.is_char_boundary(selection.head)
        {
            self.anchor = selection.anchor;
            self.move_to(selection.head, true, cx);
        }
    }

    fn move_to(&mut self, offset: usize, extend: bool, cx: &mut Context<Self>) {
        self.head = offset;
        if !extend {
            self.anchor = offset;
        }
        cx.notify();
    }

    pub(super) fn move_cursor_to_end(&mut self, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), false, cx);
    }

    pub(super) fn move_to_line_edge(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        let line = logical_line_range(&self.content, self.head);
        self.move_to(if end { line.end } else { line.start }, extend, cx);
    }

    pub(super) fn select_current_line(&mut self, cx: &mut Context<Self>) {
        self.select_range(logical_line_range(&self.content, self.head), cx);
    }

    pub(super) fn jump_logical_line(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let offset = logical_line_jump(&self.content, self.head, down);
        self.move_to(offset, extend, cx);
    }

    pub(super) fn delete_logical_line(&mut self, cx: &mut Context<Self>) {
        let range = logical_line_deletion_range(&self.content, self.head);
        if !range.is_empty() {
            self.replace(Some(range), "", false, None);
            cx.notify();
        }
    }

    pub(super) fn move_cursor_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.content.is_char_boundary(offset) {
            self.move_to(offset, false, cx);
        }
    }

    pub(super) fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        if range.start <= range.end
            && self.content.is_char_boundary(range.start)
            && self.content.is_char_boundary(range.end)
        {
            self.anchor = range.end;
            self.move_to(range.start, true, cx);
        }
    }

    fn previous(&self) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(offset, _)| (offset < self.head).then_some(offset))
            .unwrap_or(0)
    }

    fn next(&self) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(offset, _)| (offset > self.head).then_some(offset))
            .unwrap_or(self.content.len())
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.pass_empty_arrows && self.content.is_empty() {
            cx.propagate();
            return;
        }
        let offset = if self.selection().is_empty() {
            self.previous()
        } else {
            self.selection().start
        };
        self.move_to(offset, false, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.pass_empty_arrows && self.content.is_empty() {
            cx.propagate();
            return;
        }
        let offset = if self.selection().is_empty() {
            self.next()
        } else {
            self.selection().end
        };
        self.move_to(offset, false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous(), true, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next(), true, cx);
    }

    fn previous_word(&mut self, _: &PreviousWord, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(word_start_before(&self.content, self.head), false, cx);
    }

    fn next_word(&mut self, _: &NextWord, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(word_start_after(&self.content, self.head), false, cx);
    }

    fn select_previous_word(
        &mut self,
        _: &SelectPreviousWord,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(word_start_before(&self.content, self.head), true, cx);
    }

    fn select_next_word(&mut self, _: &SelectNextWord, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(word_start_after(&self.content, self.head), true, cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, false, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor_to_end(cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, true, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.anchor = 0;
        self.move_to(self.content.len(), true, cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_toward(Self::previous, cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_toward(Self::next, cx);
    }

    fn delete_previous_word(
        &mut self,
        _: &DeletePreviousWord,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_toward(|input| word_start_before(&input.content, input.head), cx);
    }

    fn delete_next_word(&mut self, _: &DeleteNextWord, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_toward(|input| word_start_after(&input.content, input.head), cx);
    }

    fn delete_toward(&mut self, offset: impl FnOnce(&Self) -> usize, cx: &mut Context<Self>) {
        let range = self.marked.clone().or_else(|| {
            self.selection().is_empty().then(|| {
                let offset = offset(self);
                self.head.min(offset)..self.head.max(offset)
            })
        });
        self.replace(range, "", false, None);
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selection()].into(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selection().is_empty() {
            self.copy(&Copy, window, cx);
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn index_at(&self, position: Point<Pixels>) -> usize {
        match (&self.layout, self.bounds) {
            (Some(line), Some(bounds)) => {
                line.closest_index_for_x(position.x - bounds.left() + self.scroll_x)
            }
            _ => self.head,
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.dragging = true;
        self.move_to(self.index_at(event.position), event.modifiers.shift, cx);
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging {
            self.move_to(self.index_at(event.position), true, cx);
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.dragging = false;
    }

    fn replace(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        marked: bool,
        selected: Option<Range<usize>>,
    ) {
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let escaped;
        let (text, selected) = if self.mode == TextMode::Json {
            let selected = selected.map(|selection| {
                let range = from_utf16(text, selection);
                json_line_separators(&text[..range.start])
                    .encode_utf16()
                    .count()
                    ..json_line_separators(&text[..range.end])
                        .encode_utf16()
                        .count()
            });
            escaped = json_line_separators(text);
            (escaped.as_str(), selected)
        } else {
            (text, selected)
        };
        let replacement = replace_content(
            &self.content,
            range,
            text,
            selected,
            self.character_limit,
            self.mode,
        );
        let selection_before_edit = self.selection_position();
        if let Some(history) = &mut self.edit_history {
            // 一段输入法组合只保留开始前的快照。
            history.pending.get_or_insert_with(|| EditSnapshot {
                text: self.content.clone(),
                selection: selection_before_edit,
            });
        }
        self.content = replacement.text.into();
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.marked = (marked && !replacement.inserted.is_empty()).then_some(replacement.inserted);
        self.anchor = replacement.selection.start;
        self.head = replacement.selection.end;
        // 重新绘制前旧字形布局不能用于命中或输入法坐标。
        self.layout = None;
        if !marked {
            self.finish_edit();
        }
    }
}

impl EntityInputHandler for MessageInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = from_utf16(&self.content, range);
        *actual = Some(to_utf16(&self.content, range.clone()));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: to_utf16(&self.content, self.selection()),
            reversed: self.head < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .clone()
            .map(|range| to_utf16(&self.content, range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.finish_edit();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.map(|range| from_utf16(&self.content, range));
        self.replace(range, text, false, None);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.map(|range| from_utf16(&self.content, range));
        self.replace(range, text, true, selected);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.layout.as_ref()?;
        let range = from_utf16(&self.content, range);
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(range.start) - self.scroll_x,
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(range.end) - self.scroll_x,
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.bounds?.localize(&position)?;
        Some(
            self.content[..self.index_at(position)]
                .encode_utf16()
                .count(),
        )
    }
}

impl Focusable for MessageInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MessageInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.blur_subscription.is_none() {
            self.blur_subscription =
                Some(cx.on_blur(&self.focus.clone(), window, |input, _, cx| {
                    input.marked = None;
                    input.finish_edit();
                    input.dragging = false;
                    cx.notify();
                }));
        }
        div()
            .id("omnitrigger-message-input")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .when(!self.embedded, |el| {
                let focused = self.focus.is_focused(window);
                el.border_1()
                    .rounded(px(6.))
                    .border_color(if focused {
                        theme::hsla(theme::ACCENT, 0.7)
                    } else {
                        rgb(theme::BORDER_MID).into()
                    })
                    .bg(rgb(theme::BACKGROUND))
                    .px(px(12.))
                    .py(px(8.))
            })
            .text_color(rgb(theme::TEXT))
            .text_size(px(13.))
            .line_height(px(if self.embedded { 20. } else { 22. }))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::previous_word))
            .on_action(cx.listener(Self::next_word))
            .on_action(cx.listener(Self::select_previous_word))
            .on_action(cx.listener(Self::select_next_word))
            .on_action(cx.listener(Self::delete_previous_word))
            .on_action(cx.listener(Self::delete_next_word))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .child(MessageLine { input: cx.entity() })
    }
}

struct MessageLine {
    input: Entity<MessageInput>,
}

#[derive(Clone)]
struct InputLayout {
    shaped: ShapedLine,
    offsets: LineOffsets,
}

impl InputLayout {
    fn x_for_index(&self, index: usize) -> Pixels {
        self.shaped.x_for_index(self.offsets.to_visible(index))
    }

    fn closest_index_for_x(&self, position: Pixels) -> usize {
        self.offsets
            .to_source(self.shaped.closest_index_for_x(position))
    }
}

struct LinePaint {
    line: InputLayout,
    placeholder: Option<ShapedLine>,
    scroll_x: Pixels,
}

impl IntoElement for MessageLine {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for MessageLine {
    type RequestLayoutState = ();
    type PrepaintState = LinePaint;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> LinePaint {
        let input = self.input.read(cx);
        let style = window.text_style();
        let (display, offsets) = visible_input(&input.content);
        let base = TextRun {
            len: display.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked) = &input.marked {
            let start = offsets.to_visible(marked.start);
            let end = offsets.to_visible(marked.end);
            vec![
                TextRun {
                    len: start,
                    ..base.clone()
                },
                TextRun {
                    len: end - start,
                    underline: Some(UnderlineStyle {
                        color: Some(style.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..base.clone()
                },
                TextRun {
                    len: display.len() - end,
                    ..base
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>()
        } else {
            vec![base]
        };
        let shaped = window.text_system().shape_line(
            display,
            style.font_size.to_pixels(window.rem_size()),
            &runs,
            None,
        );
        let line = InputLayout { shaped, offsets };
        // 占位文字只绘制，不参与命中、选区或输入法坐标。
        let placeholder = (input.content.is_empty() && !input.placeholder.is_empty()).then(|| {
            window.text_system().shape_line(
                input.placeholder.clone(),
                style.font_size.to_pixels(window.rem_size()),
                &[TextRun {
                    len: input.placeholder.len(),
                    font: style.font(),
                    color: rgb(theme::TEXT_FAINT).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
        });
        let caret = line.x_for_index(input.head);
        let viewport = if bounds.size.width > px(3.) {
            bounds.size.width - px(3.)
        } else {
            px(0.)
        };
        let mut scroll_x = input.scroll_x;
        if caret < scroll_x {
            scroll_x = caret;
        }
        if caret > scroll_x + viewport {
            scroll_x = caret - viewport;
        }
        let maximum = if line.shaped.width > viewport {
            line.shaped.width - viewport
        } else {
            px(0.)
        };
        if scroll_x > maximum {
            scroll_x = maximum;
        }
        if scroll_x < px(0.) {
            scroll_x = px(0.);
        }
        LinePaint {
            line,
            placeholder,
            scroll_x,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        paint: &mut LinePaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = self.input.read(cx);
        let focus = input.focus.clone();
        let selection = input.selection();
        let head = input.head;
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let origin = point(bounds.left() - paint.scroll_x, bounds.top());
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if !selection.is_empty() {
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(
                            origin.x + paint.line.x_for_index(selection.start),
                            bounds.top(),
                        ),
                        point(
                            origin.x + paint.line.x_for_index(selection.end),
                            bounds.bottom(),
                        ),
                    ),
                    rgba(0x7ddfb04d),
                ));
            }
            if let Some(placeholder) = &paint.placeholder {
                let _ = placeholder.paint(origin, window.line_height(), window, cx);
            }
            let _ = paint
                .line
                .shaped
                .paint(origin, window.line_height(), window, cx);
            if focus.is_focused(window) && selection.is_empty() {
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + paint.line.x_for_index(head), bounds.top()),
                        size(px(1.5), bounds.size.height),
                    ),
                    rgb(theme::ACCENT),
                ));
            }
        });
        self.input.update(cx, |input, _| {
            input.layout = Some(paint.line.clone());
            input.bounds = Some(bounds);
            input.scroll_x = paint.scroll_x;
        });
    }
}

#[derive(Clone, Default)]
struct LineOffsets(Vec<(usize, usize)>);

impl LineOffsets {
    fn to_visible(&self, source: usize) -> usize {
        let count = self.0.partition_point(|&(end, _)| end <= source);
        source
            + self.0[..count]
                .last()
                .map_or(0, |&(end, visual)| visual - end)
    }

    fn to_source(&self, visible: usize) -> usize {
        let count = self.0.partition_point(|&(_, end)| end <= visible);
        let source = visible
            - self.0[..count]
                .last()
                .map_or(0, |&(start, end)| end - start);
        // 变化的字节数来自一个 ASCII 控制字符。其可见符号内部仍映射到原字符开头。
        self.0
            .get(count)
            .map_or(source, |&(end, _)| source.min(end - 1))
    }
}

fn visible_input(content: &SharedString) -> (SharedString, LineOffsets) {
    if !content.contains(['\r', '\n', '\t', '\u{2028}', '\u{2029}']) {
        return (content.clone(), LineOffsets::default());
    }
    let mut visible = String::with_capacity(content.len());
    let mut offsets = Vec::new();
    for (index, ch) in content.char_indices() {
        let symbol = match ch {
            '\r' => '␍',
            '\n' => '␊',
            '\t' => '⇥',
            '\u{2028}' | '\u{2029}' => '↵',
            _ => ch,
        };
        visible.push(symbol);
        if ch.len_utf8() != symbol.len_utf8() {
            offsets.push((index + ch.len_utf8(), visible.len()));
        }
    }
    (visible.into(), LineOffsets(offsets))
}

pub(super) fn input_preview(text: impl Into<SharedString>) -> SharedString {
    visible_input(&text.into()).0
}

fn logical_line_range(text: &str, cursor: usize) -> Range<usize> {
    let bytes = text.as_bytes();
    // 把 CRLF 当作一个换行，落在两字符之间的光标仍属于上一行。
    let cursor = if cursor > 0 && bytes[cursor - 1] == b'\r' && bytes.get(cursor) == Some(&b'\n') {
        cursor - 1
    } else {
        cursor
    };
    let is_line_break = |ch| matches!(ch, '\r' | '\n' | '\u{2028}' | '\u{2029}');
    let start = text[..cursor]
        .char_indices()
        .rfind(|&(_, ch)| is_line_break(ch))
        .map_or(0, |(offset, ch)| offset + ch.len_utf8());
    let end = text[cursor..]
        .find(is_line_break)
        .map_or(text.len(), |offset| cursor + offset);
    start..end
}

fn logical_line_jump(text: &str, cursor: usize, down: bool) -> usize {
    let line = logical_line_range(text, cursor);
    if down {
        // 行尾的换行符是一个字素，包含 CRLF。
        let break_len = text[line.end..].graphemes(true).next().map_or(0, str::len);
        return line.end + break_len;
    }
    if cursor != line.start {
        return line.start;
    }
    let previous_end = text[..line.start]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(offset, _)| offset);
    logical_line_range(text, previous_end).start
}

fn logical_line_deletion_range(text: &str, cursor: usize) -> Range<usize> {
    let line = logical_line_range(text, cursor);
    if line.end < text.len() {
        return line.start..logical_line_jump(text, line.end, true);
    }
    if line.start > 0 {
        let previous = logical_line_jump(text, line.start, false);
        return logical_line_range(text, previous).end..line.end;
    }
    line
}

fn word_start_before(text: &str, offset: usize) -> usize {
    text.unicode_word_indices()
        .take_while(|&(start, _)| start < offset)
        .last()
        .map_or(0, |(start, _)| start)
}

fn word_start_after(text: &str, offset: usize) -> usize {
    text.unicode_word_indices()
        .find(|&(start, _)| start > offset)
        .map_or(text.len(), |(start, _)| start)
}

fn json_line_separators(text: &str) -> String {
    text.replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn single_line(text: &str, limit: usize) -> String {
    let normalized = text
        .replace("\r\n", " ")
        .chars()
        .map(|ch| match ch {
            '\r' | '\n' | '\t' | '\u{2028}' | '\u{2029}' => ' ',
            _ => ch,
        })
        .collect::<String>();
    limit_text(&normalized, limit)
}

fn prepare_input(text: &str, limit: usize, mode: TextMode) -> String {
    if mode == TextMode::PreserveWhitespace {
        limit_text(text, limit)
    } else {
        single_line(text, limit)
    }
}

fn limit_text(text: &str, limit: usize) -> String {
    let mut used = 0;
    text.graphemes(true)
        .take_while(|grapheme| {
            used += grapheme.chars().count();
            used <= limit
        })
        .collect()
}

struct Replacement {
    text: String,
    inserted: Range<usize>,
    selection: Range<usize>,
}

fn replace_content(
    content: &str,
    range: Range<usize>,
    text: &str,
    selected: Option<Range<usize>>,
    character_limit: usize,
    mode: TextMode,
) -> Replacement {
    let outside = content[..range.start].chars().count() + content[range.end..].chars().count();
    let insertion = prepare_input(text, character_limit.saturating_sub(outside), mode);
    let mut updated = content[..range.start].to_owned();
    updated.push_str(&insertion);
    updated.push_str(&content[range.end..]);
    // 输入法的选区相对于本次插入内容，不相对于替换前的整行。
    let selection = selected
        .map(|range| from_utf16(&insertion, range))
        .unwrap_or(insertion.len()..insertion.len());
    Replacement {
        text: updated,
        inserted: range.start..range.start + insertion.len(),
        selection: range.start + selection.start..range.start + selection.end,
    }
}

fn utf16_boundary(text: &str, offset: usize, round_up: bool) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= offset {
            return byte;
        }
        let next = units + ch.len_utf16();
        if offset < next {
            return if round_up { byte + ch.len_utf8() } else { byte };
        }
        units = next;
    }
    text.len()
}

fn from_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    let start = utf16_boundary(text, range.start.min(range.end), false);
    let end = if range.is_empty() {
        start
    } else {
        utf16_boundary(text, range.start.max(range.end), true)
    };
    start..end
}

fn to_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    text[..range.start].encode_utf16().count()..text[..range.end].encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_logical_lines_with_mixed_newlines_and_unicode() {
        let text = "甲\r\n乙\n\r丙";
        for (cursor, expected) in [
            (0, 0..3),
            (3, 0..3),
            (4, 0..3),
            (5, 5..8),
            (8, 5..8),
            (9, 9..9),
            (text.len(), 10..text.len()),
        ] {
            assert_eq!(logical_line_range(text, cursor), expected);
        }
        assert_eq!(logical_line_range("", 0), 0..0);
        let text = "甲\u{2028}乙\u{2029}丙";
        assert_eq!(logical_line_range(text, 9), 6..9);
        assert_eq!(logical_line_range(text, 12), 12..text.len());
    }

    #[test]
    fn jumps_logical_lines_without_stopping_inside_line_breaks() {
        let text = "甲\r\n乙\n\r🍵\u{2028}尾";
        for (cursor, up, down) in [
            (0, 0, 5),
            (3, 0, 5),
            (4, 0, 5),
            (5, 0, 9),
            (8, 5, 9),
            (9, 5, 10),
            (10, 9, 17),
            (17, 10, 20),
            (text.len(), 17, text.len()),
        ] {
            assert_eq!(logical_line_jump(text, cursor, false), up);
            assert_eq!(logical_line_jump(text, cursor, true), down);
        }
        assert_eq!(logical_line_jump("", 0, false), 0);
        assert_eq!(logical_line_jump("", 0, true), 0);
    }

    #[test]
    fn deletes_whole_logical_lines_with_complete_line_breaks() {
        let text = "甲\r\n乙\n\r🍵\u{2028}尾";
        for (cursor, expected) in [
            (0, 0..5),
            (4, 0..5),
            (5, 5..9),
            (9, 9..10),
            (10, 10..17),
            (17, 14..20),
            (20, 14..20),
        ] {
            assert_eq!(logical_line_deletion_range(text, cursor), expected);
        }
        assert_eq!(logical_line_deletion_range("a\r\n", 3), 1..3);
        assert_eq!(logical_line_deletion_range("甲", 3), 0..3);
        assert_eq!(logical_line_deletion_range("", 0), 0..0);
    }

    #[test]
    fn finds_word_starts_across_operators_whitespace_and_combining_characters() {
        let text = "e\u{301}lan + 2**80\r\nnext";
        let base = text.find('2').unwrap();
        let exponent = text.find("80").unwrap();
        let next = text.find("next").unwrap();
        assert_eq!(word_start_before(text, "e\u{301}".len()), 0);
        assert_eq!(word_start_before(text, next), exponent);
        assert_eq!(word_start_before(text, exponent), base);
        assert_eq!(word_start_after(text, 0), base);
        assert_eq!(word_start_after(text, base), exponent);
        assert_eq!(word_start_after(text, exponent), next);
        assert_eq!(word_start_after(text, next), text.len());
    }

    #[test]
    fn json_parameters_preserve_unicode_separators_when_loaded_or_pasted() {
        let value = serde_json::json!({"text":"中\u{2028}文\u{2029}🦀\n\t", "literal":"\\u2028"});
        let serialized = value.to_string();
        let line = single_line(&json_line_separators(&serialized), 128 * 1024);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap(),
            value
        );
        let pasted = replace_content(
            "{}",
            0..2,
            &json_line_separators(&serialized),
            None,
            128 * 1024,
            TextMode::Json,
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&pasted.text).unwrap(),
            value
        );
        let large = serde_json::json!({"text":"\u{2028}".repeat(21_000)});
        let line = single_line(&json_line_separators(&large.to_string()), 128 * 1024);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap(),
            large
        );
    }

    #[test]
    fn utf16_ranges_keep_chinese_and_astral_characters_whole() {
        let text = "中🦀文";
        assert_eq!(&text[from_utf16(text, 1..3)], "🦀");
        assert_eq!(to_utf16(text, "中".len().."中🦀".len()), 1..3);
        assert_eq!(&text[from_utf16(text, 2..3)], "🦀");
        assert_eq!(from_utf16(text, 2..2), "中".len().."中".len());
        assert_eq!(from_utf16(text, 99..99), text.len()..text.len());
    }

    #[test]
    fn paste_is_single_line_and_character_limit_is_not_byte_limit() {
        assert_eq!(
            single_line("第一行\r\n第二行\n尾部", 2000),
            "第一行 第二行 尾部"
        );
        assert_eq!(single_line("中🦀文", 2), "中🦀");
        assert_eq!(single_line(&"中".repeat(2001), 2000).chars().count(), 2000);
        assert_eq!(single_line("中e\u{301}", 2), "中");
        assert_eq!(single_line("👨‍👩‍👧‍👦", 3), "");
    }

    #[test]
    fn search_paste_preserves_whitespace_and_maps_edited_and_displayed_offsets() {
        let fixture = "中\r\n\t🦀\u{2028}前";
        let replacement =
            replace_content("[]", 1..1, fixture, None, 50, TextMode::PreserveWhitespace);
        assert_eq!(&replacement.text[replacement.inserted], fixture);
        assert_eq!(
            prepare_input("甲\r\n乙", 2, TextMode::PreserveWhitespace),
            "甲"
        );
        let content = SharedString::from(replacement.text);
        let (shown, offsets) = visible_input(&content);
        assert_eq!(shown.as_ref(), "[中␍␊⇥🦀↵前]");
        for source in content
            .char_indices()
            .map(|(index, _)| index)
            .chain([content.len()])
        {
            let visible = offsets.to_visible(source);
            assert!(shown.is_char_boundary(visible));
            assert_eq!(offsets.to_source(visible), source);
        }
        let carriage_return = "[中".len();
        for byte in offsets.to_visible(carriage_return)..offsets.to_visible(carriage_return + 1) {
            assert_eq!(offsets.to_source(byte), carriage_return);
        }
    }

    #[test]
    fn ime_selection_is_relative_to_inserted_text_in_middle_of_content() {
        let replacement = replace_content(
            "前旧文后",
            "前".len().."前旧文".len(),
            "🦀中",
            Some(2..3),
            MAX_CHARACTERS,
            TextMode::SingleLine,
        );
        assert_eq!(replacement.text, "前🦀中后");
        assert_eq!(&replacement.text[replacement.selection], "中");
        assert_eq!(&replacement.text[replacement.inserted], "🦀中");
    }

    #[test]
    fn replacing_at_limit_preserves_existing_suffix() {
        let existing = format!("{}尾", "中".repeat(1999));
        let replacement = replace_content(
            &existing,
            0.."中".len(),
            "🦀新",
            None,
            MAX_CHARACTERS,
            TextMode::SingleLine,
        );
        assert_eq!(replacement.text.chars().count(), MAX_CHARACTERS);
        assert!(replacement.text.starts_with('🦀'));
        assert!(replacement.text.ends_with('尾'));
        assert_eq!(replacement.selection, "🦀".len().."🦀".len());
    }
}
