//! 单行消息输入：文本由平台输入法提交，键绑定仅处理编辑命令。

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, rgba, size, App, Bounds,
    ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ShapedLine, SharedString, Style,
    TextRun, UTF16Selection, UnderlineStyle, Window,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme;

const MAX_CHARACTERS: usize = 2000;
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

pub struct MessageInput {
    focus: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    anchor: usize,
    head: usize,
    marked: Option<Range<usize>>,
    dragging: bool,
    layout: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    scroll_x: Pixels,
    blur_subscription: Option<gpui::Subscription>,
}

impl MessageInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: "触发时显示的文字".into(),
            anchor: 0,
            head: 0,
            marked: None,
            dragging: false,
            layout: None,
            bounds: None,
            scroll_x: px(0.),
            blur_subscription: None,
        }
    }

    pub fn text(&self) -> String {
        self.content.to_string()
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = single_line(&text.into(), MAX_CHARACTERS).into();
        // 载入已有消息时从开头显示，避免长消息只露出结尾。
        self.anchor = 0;
        self.head = 0;
        self.marked = None;
        self.dragging = false;
        self.layout = None;
        self.scroll_x = px(0.);
        cx.notify();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn move_to(&mut self, offset: usize, extend: bool, cx: &mut Context<Self>) {
        self.head = offset;
        if !extend {
            self.anchor = offset;
        }
        cx.notify();
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
        let offset = if self.selection().is_empty() {
            self.previous()
        } else {
            self.selection().start
        };
        self.move_to(offset, false, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
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

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, false, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), false, cx);
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

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection().is_empty() {
            self.anchor = self.previous();
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection().is_empty() {
            self.anchor = self.next();
        }
        self.replace_text_in_range(None, "", window, cx);
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
        range_utf16: Option<Range<usize>>,
        text: &str,
        marked: bool,
        selected: Option<Range<usize>>,
    ) {
        let range = range_utf16
            .map(|range| from_utf16(&self.content, range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let replacement = replace_content(&self.content, range, text, selected);
        self.content = replacement.text.into();
        self.marked = (marked && !replacement.inserted.is_empty()).then_some(replacement.inserted);
        self.anchor = replacement.selection.start;
        self.head = replacement.selection.end;
        // 重新绘制前旧字形布局不能用于命中或输入法坐标。
        self.layout = None;
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
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            .border_1()
            .rounded(px(9.))
            .border_color(if self.focus.is_focused(window) {
                rgb(0x4a7a64)
            } else {
                rgb(theme::BORDER_HI)
            })
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .px(px(12.))
            .py(px(8.))
            .text_size(px(13.))
            .line_height(px(22.))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
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

struct LinePaint {
    line: ShapedLine,
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
        let base = TextRun {
            len: input.content.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked) = &input.marked {
            vec![
                TextRun {
                    len: marked.start,
                    ..base.clone()
                },
                TextRun {
                    len: marked.len(),
                    underline: Some(UnderlineStyle {
                        color: Some(style.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..base.clone()
                },
                TextRun {
                    len: input.content.len() - marked.end,
                    ..base
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>()
        } else {
            vec![base]
        };
        let line = window.text_system().shape_line(
            input.content.clone(),
            style.font_size.to_pixels(window.rem_size()),
            &runs,
            None,
        );
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
        let maximum = if line.width > viewport {
            line.width - viewport
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
            let _ = paint.line.paint(origin, window.line_height(), window, cx);
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

fn single_line(text: &str, limit: usize) -> String {
    let normalized = text
        .replace("\r\n", " ")
        .chars()
        .map(|ch| match ch {
            '\r' | '\n' | '\t' | '\u{2028}' | '\u{2029}' => ' ',
            _ => ch,
        })
        .collect::<String>();
    let mut used = 0;
    normalized
        .graphemes(true)
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
) -> Replacement {
    let outside = content[..range.start].chars().count() + content[range.end..].chars().count();
    let insertion = single_line(text, MAX_CHARACTERS.saturating_sub(outside));
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
    fn ime_selection_is_relative_to_inserted_text_in_middle_of_content() {
        let replacement =
            replace_content("前旧文后", "前".len().."前旧文".len(), "🦀中", Some(2..3));
        assert_eq!(replacement.text, "前🦀中后");
        assert_eq!(&replacement.text[replacement.selection], "中");
        assert_eq!(&replacement.text[replacement.inserted], "🦀中");
    }

    #[test]
    fn replacing_at_limit_preserves_existing_suffix() {
        let existing = format!("{}尾", "中".repeat(1999));
        let replacement = replace_content(&existing, 0.."中".len(), "🦀新", None);
        assert_eq!(replacement.text.chars().count(), MAX_CHARACTERS);
        assert!(replacement.text.starts_with('🦀'));
        assert!(replacement.text.ends_with('尾'));
        assert_eq!(replacement.selection, "🦀".len().."🦀".len());
    }
}
