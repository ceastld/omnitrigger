//! 触发器页的键盘与鼠标视图：按键布局、前缀层与鼠标部位的归类。
//!
//! 只依据已筛选的摘要计算，结果保持绑定顺序；界面只负责绘制。

use omnitrigger_core::{MouseButton, WheelDirection};

use crate::trigger_browse::{Row, Terminal};

/// 键盘上的一个键位；坐标与宽度以标准键宽为单位。
#[derive(Clone, Copy, Debug)]
pub struct KeySlot {
    pub vk: u16,
    pub label: &'static str,
    pub x: f32,
    pub y: f32,
    pub w: f32,
}

const fn key(vk: u16, label: &'static str, x: f32, y: f32, w: f32) -> KeySlot {
    KeySlot { vk, label, x, y, w }
}

/// 布局总宽与总高（键宽单位）。
pub const KEYBOARD_WIDTH: f32 = 18.5;
pub const KEYBOARD_HEIGHT: f32 = 6.25;

/// 不含小键盘的常见美式布局。
pub const KEYBOARD: &[KeySlot] = &[
    key(0x1b, "Esc", 0., 0., 1.),
    key(0x70, "F1", 2., 0., 1.),
    key(0x71, "F2", 3., 0., 1.),
    key(0x72, "F3", 4., 0., 1.),
    key(0x73, "F4", 5., 0., 1.),
    key(0x74, "F5", 6.5, 0., 1.),
    key(0x75, "F6", 7.5, 0., 1.),
    key(0x76, "F7", 8.5, 0., 1.),
    key(0x77, "F8", 9.5, 0., 1.),
    key(0x78, "F9", 11., 0., 1.),
    key(0x79, "F10", 12., 0., 1.),
    key(0x7a, "F11", 13., 0., 1.),
    key(0x7b, "F12", 14., 0., 1.),
    key(0x2c, "PrtSc", 15.5, 0., 1.),
    key(0x91, "ScrLk", 16.5, 0., 1.),
    key(0x13, "Pause", 17.5, 0., 1.),
    key(0xc0, "`", 0., 1.25, 1.),
    key(0x31, "1", 1., 1.25, 1.),
    key(0x32, "2", 2., 1.25, 1.),
    key(0x33, "3", 3., 1.25, 1.),
    key(0x34, "4", 4., 1.25, 1.),
    key(0x35, "5", 5., 1.25, 1.),
    key(0x36, "6", 6., 1.25, 1.),
    key(0x37, "7", 7., 1.25, 1.),
    key(0x38, "8", 8., 1.25, 1.),
    key(0x39, "9", 9., 1.25, 1.),
    key(0x30, "0", 10., 1.25, 1.),
    key(0xbd, "-", 11., 1.25, 1.),
    key(0xbb, "=", 12., 1.25, 1.),
    key(0x08, "Backspace", 13., 1.25, 2.),
    key(0x2d, "Ins", 15.5, 1.25, 1.),
    key(0x24, "Home", 16.5, 1.25, 1.),
    key(0x21, "PgUp", 17.5, 1.25, 1.),
    key(0x09, "Tab", 0., 2.25, 1.5),
    key(0x51, "Q", 1.5, 2.25, 1.),
    key(0x57, "W", 2.5, 2.25, 1.),
    key(0x45, "E", 3.5, 2.25, 1.),
    key(0x52, "R", 4.5, 2.25, 1.),
    key(0x54, "T", 5.5, 2.25, 1.),
    key(0x59, "Y", 6.5, 2.25, 1.),
    key(0x55, "U", 7.5, 2.25, 1.),
    key(0x49, "I", 8.5, 2.25, 1.),
    key(0x4f, "O", 9.5, 2.25, 1.),
    key(0x50, "P", 10.5, 2.25, 1.),
    key(0xdb, "[", 11.5, 2.25, 1.),
    key(0xdd, "]", 12.5, 2.25, 1.),
    key(0xdc, "\\", 13.5, 2.25, 1.5),
    key(0x2e, "Del", 15.5, 2.25, 1.),
    key(0x23, "End", 16.5, 2.25, 1.),
    key(0x22, "PgDn", 17.5, 2.25, 1.),
    key(0x14, "Caps", 0., 3.25, 1.75),
    key(0x41, "A", 1.75, 3.25, 1.),
    key(0x53, "S", 2.75, 3.25, 1.),
    key(0x44, "D", 3.75, 3.25, 1.),
    key(0x46, "F", 4.75, 3.25, 1.),
    key(0x47, "G", 5.75, 3.25, 1.),
    key(0x48, "H", 6.75, 3.25, 1.),
    key(0x4a, "J", 7.75, 3.25, 1.),
    key(0x4b, "K", 8.75, 3.25, 1.),
    key(0x4c, "L", 9.75, 3.25, 1.),
    key(0xba, ";", 10.75, 3.25, 1.),
    key(0xde, "'", 11.75, 3.25, 1.),
    key(0x0d, "Enter", 12.75, 3.25, 2.25),
    key(0xa0, "Shift", 0., 4.25, 2.25),
    key(0x5a, "Z", 2.25, 4.25, 1.),
    key(0x58, "X", 3.25, 4.25, 1.),
    key(0x43, "C", 4.25, 4.25, 1.),
    key(0x56, "V", 5.25, 4.25, 1.),
    key(0x42, "B", 6.25, 4.25, 1.),
    key(0x4e, "N", 7.25, 4.25, 1.),
    key(0x4d, "M", 8.25, 4.25, 1.),
    key(0xbc, ",", 9.25, 4.25, 1.),
    key(0xbe, ".", 10.25, 4.25, 1.),
    key(0xbf, "/", 11.25, 4.25, 1.),
    key(0xa1, "Shift", 12.25, 4.25, 2.75),
    key(0x26, "↑", 16.5, 4.25, 1.),
    key(0xa2, "Ctrl", 0., 5.25, 1.25),
    key(0x5b, "Win", 1.25, 5.25, 1.25),
    key(0xa4, "Alt", 2.5, 5.25, 1.25),
    key(0x20, "Space", 3.75, 5.25, 6.25),
    key(0xa5, "Alt", 10., 5.25, 1.25),
    key(0x5c, "Win", 11.25, 5.25, 1.25),
    key(0x5d, "Menu", 12.5, 5.25, 1.25),
    key(0xa3, "Ctrl", 13.75, 5.25, 1.25),
    key(0x25, "←", 15.5, 5.25, 1.),
    key(0x28, "↓", 16.5, 5.25, 1.),
    key(0x27, "→", 17.5, 5.25, 1.),
];

/// 键位是否对应某个按键码；不分左右的 Shift、Ctrl、Alt 同时对应两侧键位。
pub fn slot_accepts(slot: u16, key: u16) -> bool {
    slot == key
        || match key {
            0x10 => matches!(slot, 0xa0 | 0xa1),
            0x11 => matches!(slot, 0xa2 | 0xa3),
            0x12 => matches!(slot, 0xa4 | 0xa5),
            _ => false,
        }
}

/// 以键盘按键结尾的前缀层，按首次出现排列；空名称表示无前缀。
pub fn key_layers<'a>(rows: &[&'a Row]) -> Vec<(&'a str, usize)> {
    let mut layers: Vec<(&str, usize)> = Vec::new();
    for row in rows {
        if !matches!(row.terminal, Terminal::Key(_)) {
            continue;
        }
        match layers.iter_mut().find(|(name, _)| *name == row.prefix) {
            Some((_, count)) => *count += 1,
            None => layers.push((&row.prefix, 1)),
        }
    }
    layers
}

/// 键盘视图的计算结果。
#[derive(Debug, Default, PartialEq)]
pub struct KeyboardView {
    /// 当前层中作为目标键已绑定的键位。
    pub bound: Vec<u16>,
    /// 其中所有绑定都已停用的键位。
    pub bound_disabled: Vec<u16>,
    /// 当前层需要按住的前缀键位。
    pub held: Vec<u16>,
    /// 下方列出的触发器。
    pub ids: Vec<u64>,
    /// 当前层中目标键不在布局上的条数，例如小键盘或媒体键。
    pub off_layout: usize,
}

/// `layer` 为 `None` 时汇总所有层；`picked` 为选中的键位。
pub fn keyboard_view(rows: &[&Row], layer: Option<&str>, picked: Option<u16>) -> KeyboardView {
    let mut view = KeyboardView::default();
    let mut enabled_slots: Vec<u16> = Vec::new();
    for row in rows {
        let Terminal::Key(key) = row.terminal else {
            continue;
        };
        if layer.is_some_and(|layer| layer != row.prefix) {
            continue;
        }
        if layer.is_some() && view.held.is_empty() {
            view.held = KEYBOARD
                .iter()
                .filter(|slot| {
                    row.held_keys
                        .iter()
                        .any(|&held| slot_accepts(slot.vk, held))
                })
                .map(|slot| slot.vk)
                .collect();
        }
        let mut matched = false;
        for slot in KEYBOARD.iter().filter(|slot| slot_accepts(slot.vk, key)) {
            matched = true;
            if !view.bound.contains(&slot.vk) {
                view.bound.push(slot.vk);
            }
            if row.status != crate::trigger_browse::StatusKind::Disabled
                && !enabled_slots.contains(&slot.vk)
            {
                enabled_slots.push(slot.vk);
            }
        }
        if !matched {
            view.off_layout += 1;
        }
        if picked.is_none_or(|slot| slot_accepts(slot, key)) {
            view.ids.push(row.id);
        }
    }
    view.bound_disabled = view
        .bound
        .iter()
        .copied()
        .filter(|slot| !enabled_slots.contains(slot))
        .collect();
    view
}

/// 鼠标视图中的部位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MousePart {
    Left,
    Right,
    Middle,
    X1,
    X2,
    WheelUp,
    WheelDown,
    WheelSide,
    Region,
}

impl MousePart {
    pub const ALL: [Self; 9] = [
        Self::Left,
        Self::Right,
        Self::Middle,
        Self::X1,
        Self::X2,
        Self::WheelUp,
        Self::WheelDown,
        Self::WheelSide,
        Self::Region,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "左键",
            Self::Right => "右键",
            Self::Middle => "中键",
            Self::X1 => "侧键 1",
            Self::X2 => "侧键 2",
            Self::WheelUp => "滚轮 ↑",
            Self::WheelDown => "滚轮 ↓",
            Self::WheelSide => "横向滚轮",
            Self::Region => "屏幕区域",
        }
    }

    fn of_button(button: MouseButton) -> Self {
        match button {
            MouseButton::Left => Self::Left,
            MouseButton::Right => Self::Right,
            MouseButton::Middle => Self::Middle,
            MouseButton::X1 => Self::X1,
            MouseButton::X2 => Self::X2,
        }
    }

    fn is_terminal_of(self, row: &Row) -> bool {
        match row.terminal {
            Terminal::Mouse(button) => Self::of_button(button) == self,
            Terminal::Wheel(WheelDirection::Up) => self == Self::WheelUp,
            Terminal::Wheel(WheelDirection::Down) => self == Self::WheelDown,
            Terminal::Wheel(_) => self == Self::WheelSide,
            Terminal::Key(_) => false,
        }
    }

    fn is_prefix_of(self, row: &Row) -> bool {
        match self {
            Self::Region => row.has_region,
            _ => row
                .held_mouse
                .iter()
                .any(|&button| Self::of_button(button) == self),
        }
    }
}

/// 某个部位作为目标与作为前缀的条数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartCount {
    pub terminal: usize,
    pub prefix: usize,
}

impl PartCount {
    pub fn total(self) -> usize {
        self.terminal + self.prefix
    }
}

pub fn part_count(rows: &[&Row], part: MousePart) -> PartCount {
    rows.iter().fold(PartCount::default(), |mut count, row| {
        if part.is_terminal_of(row) {
            count.terminal += 1;
        } else if part.is_prefix_of(row) {
            count.prefix += 1;
        }
        count
    })
}

/// 鼠标视图下方列出的触发器：选中部位时列出与它相关的，否则列出所有用到鼠标或屏幕区域的。
pub fn mouse_ids(rows: &[&Row], picked: Option<MousePart>) -> Vec<u64> {
    rows.iter()
        .filter(|row| match picked {
            Some(part) => part.is_terminal_of(row) || part.is_prefix_of(row),
            None => MousePart::ALL
                .iter()
                .any(|part| part.is_terminal_of(row) || part.is_prefix_of(row)),
        })
        .map(|row| row.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger_browse::{ActionKind, InputKind, ScopeKind, StatusKind};

    fn row(id: u64, prefix: &str, held_keys: &[u16], terminal: Terminal) -> Row {
        Row {
            id,
            text: String::new(),
            prefix: prefix.into(),
            input: InputKind::Keyboard,
            action: ActionKind::Notification,
            scope: ScopeKind::Global,
            status: StatusKind::Enabled,
            held_keys: held_keys.to_vec(),
            held_mouse: Vec::new(),
            terminal,
            has_region: false,
        }
    }

    #[test]
    fn layout_has_unique_slots() {
        for (index, slot) in KEYBOARD.iter().enumerate() {
            assert!(
                KEYBOARD[index + 1..]
                    .iter()
                    .all(|other| other.vk != slot.vk),
                "{} 重复",
                slot.label
            );
            assert!(slot.x + slot.w <= KEYBOARD_WIDTH && slot.y + 1. <= KEYBOARD_HEIGHT);
        }
    }

    #[test]
    fn generic_modifiers_match_both_sides() {
        assert!(slot_accepts(0xa2, 0x11) && slot_accepts(0xa3, 0x11));
        assert!(slot_accepts(0xa2, 0xa2) && !slot_accepts(0xa3, 0xa2));
        assert!(!slot_accepts(0x41, 0x11));
    }

    #[test]
    fn layer_marks_held_and_bound_keys_and_picks_list() {
        let rows = [
            row(1, "CapsLock", &[0x14], Terminal::Key(0x48)),
            row(2, "CapsLock", &[0x14], Terminal::Key(0x4a)),
            row(3, "Ctrl + Alt", &[0x11, 0x12], Terminal::Key(0x48)),
            row(4, "", &[], Terminal::Key(0x6b)),
            row(5, "右键", &[], Terminal::Wheel(WheelDirection::Up)),
        ];
        let rows: Vec<&Row> = rows.iter().collect();
        assert_eq!(
            key_layers(&rows),
            vec![("CapsLock", 2), ("Ctrl + Alt", 1), ("", 1)]
        );

        let caps = keyboard_view(&rows, Some("CapsLock"), None);
        assert_eq!(caps.held, vec![0x14]);
        assert_eq!(caps.bound, vec![0x48, 0x4a]);
        assert_eq!(caps.ids, vec![1, 2]);

        let ctrl_alt = keyboard_view(&rows, Some("Ctrl + Alt"), None);
        assert_eq!(ctrl_alt.held, vec![0xa2, 0xa4, 0xa5, 0xa3]);

        let all = keyboard_view(&rows, None, Some(0x48));
        assert!(all.held.is_empty());
        assert_eq!(all.ids, vec![1, 3]);
        assert_eq!(all.off_layout, 1);
    }

    #[test]
    fn disabled_only_keys_are_reported() {
        let mut off = row(1, "", &[], Terminal::Key(0x41));
        off.status = StatusKind::Disabled;
        let rows = [off, row(2, "", &[], Terminal::Key(0x42))];
        let rows: Vec<&Row> = rows.iter().collect();
        let view = keyboard_view(&rows, Some(""), None);
        assert_eq!(view.bound_disabled, vec![0x41]);
    }

    #[test]
    fn mouse_parts_count_terminal_before_prefix() {
        let mut tab = row(1, "右键", &[], Terminal::Wheel(WheelDirection::Up));
        tab.held_mouse = vec![MouseButton::Right];
        let hold = row(2, "", &[], Terminal::Mouse(MouseButton::Right));
        let mut edge = row(3, "左边缘", &[], Terminal::Wheel(WheelDirection::Down));
        edge.has_region = true;
        let keys = row(4, "", &[], Terminal::Key(0x41));
        let rows = [tab, hold, edge, keys];
        let rows: Vec<&Row> = rows.iter().collect();
        assert_eq!(
            part_count(&rows, MousePart::Right),
            PartCount {
                terminal: 1,
                prefix: 1
            }
        );
        assert_eq!(part_count(&rows, MousePart::Region).prefix, 1);
        assert_eq!(mouse_ids(&rows, Some(MousePart::Right)), vec![1, 2]);
        assert_eq!(mouse_ids(&rows, None), vec![1, 2, 3]);
    }
}
