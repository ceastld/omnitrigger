//! 触发器页的搜索、筛选与分组。
//!
//! 只依据绑定本身和调用方给出的就绪状态计算，结果保持绑定顺序；界面只负责绘制。

use omnitrigger_core::{Activation, Button, MouseButton, TriggerSpec, WheelDirection};

use crate::settings::{ActionSpec, Binding};

/// 输入方式，按组合实际使用的输入划分。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Keyboard,
    MouseButton,
    Wheel,
    PointerRegion,
}

/// 动作类型，与列表图标的分类一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Panel,
    Wheel,
    Shortcut,
    Window,
    System,
    Local,
    Package,
    Browser,
    Program,
    External,
    Plugin,
    Notification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Global,
    Scoped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Enabled,
    Disabled,
    Unready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facet {
    Input,
    Action,
    Scope,
    Status,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacetValue {
    Input(InputKind),
    Action(ActionKind),
    Scope(ScopeKind),
    Status(StatusKind),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GroupBy {
    #[default]
    Prefix,
    Action,
    Scope,
    Input,
    None,
}

/// 每个维度至多选一项；`None` 表示不限。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub input: Option<InputKind>,
    pub action: Option<ActionKind>,
    pub scope: Option<ScopeKind>,
    pub status: Option<StatusKind>,
}

/// 参与筛选的一条绑定摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    /// 已转小写的搜索文本。
    pub text: String,
    /// 前缀层名称；没有前缀时为空。
    pub prefix: String,
    pub input: InputKind,
    pub action: ActionKind,
    pub scope: ScopeKind,
    pub status: StatusKind,
    /// 前缀中的键盘按键码。
    pub held_keys: Vec<u16>,
    /// 前缀中的鼠标按键。
    pub held_mouse: Vec<MouseButton>,
    pub terminal: Terminal,
    /// 是否带指针区域条件，例如屏幕边缘。
    pub has_region: bool,
}

/// 组合的目标输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminal {
    Key(u16),
    Mouse(MouseButton),
    Wheel(WheelDirection),
}

impl Terminal {
    pub fn of(spec: &TriggerSpec) -> Self {
        match spec.activation {
            Activation::Wheel(direction) => Self::Wheel(direction),
            Activation::Press(button)
            | Activation::Hold { button, .. }
            | Activation::Drag { button, .. } => match button {
                Button::Key(key) => Self::Key(key),
                Button::Mouse(mouse) => Self::Mouse(mouse),
            },
        }
    }
}

/// 拆分前缀中的键盘按键与鼠标按键。
pub fn split_held(held: &[Button]) -> (Vec<u16>, Vec<MouseButton>) {
    let mut keys = Vec::new();
    let mut mouse = Vec::new();
    for button in held {
        match *button {
            Button::Key(key) => keys.push(key),
            Button::Mouse(button) => mouse.push(button),
        }
    }
    (keys, mouse)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// 分组标识，也用于记录折叠状态；不分组时为空。
    pub key: String,
    pub label: String,
    pub ids: Vec<u64>,
    pub disabled: usize,
}

impl InputKind {
    pub const ALL: [Self; 4] = [
        Self::Keyboard,
        Self::MouseButton,
        Self::Wheel,
        Self::PointerRegion,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Keyboard => "键盘",
            Self::MouseButton => "鼠标按键",
            Self::Wheel => "滚轮",
            Self::PointerRegion => "屏幕区域",
        }
    }
}

impl ActionKind {
    pub const ALL: [Self; 12] = [
        Self::Panel,
        Self::Wheel,
        Self::Shortcut,
        Self::Window,
        Self::System,
        Self::Local,
        Self::Package,
        Self::Browser,
        Self::Program,
        Self::External,
        Self::Plugin,
        Self::Notification,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Panel => "触发面板",
            Self::Wheel => "轮盘",
            Self::Shortcut => "发送快捷键",
            Self::Window => "窗口",
            Self::System => "系统调节",
            Self::Local => "本地动作",
            Self::Package => "动作包",
            Self::Browser => "浏览器",
            Self::Program => "运行程序",
            Self::External => "外部动作",
            Self::Plugin => "插件",
            Self::Notification => "仅提示",
        }
    }
}

impl ScopeKind {
    pub const ALL: [Self; 2] = [Self::Global, Self::Scoped];

    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "所有应用",
            Self::Scoped => "指定应用",
        }
    }
}

impl StatusKind {
    pub const ALL: [Self; 3] = [Self::Enabled, Self::Disabled, Self::Unready];

    pub fn label(self) -> &'static str {
        match self {
            Self::Enabled => "已启用",
            Self::Disabled => "已停用",
            Self::Unready => "未就绪",
        }
    }
}

impl Facet {
    pub const ALL: [Self; 4] = [Self::Input, Self::Action, Self::Scope, Self::Status];

    pub fn label(self) -> &'static str {
        match self {
            Self::Input => "输入",
            Self::Action => "动作",
            Self::Scope => "范围",
            Self::Status => "状态",
        }
    }

    pub fn options(self) -> Vec<FacetValue> {
        match self {
            Self::Input => InputKind::ALL.map(FacetValue::Input).to_vec(),
            Self::Action => ActionKind::ALL.map(FacetValue::Action).to_vec(),
            Self::Scope => ScopeKind::ALL.map(FacetValue::Scope).to_vec(),
            Self::Status => StatusKind::ALL.map(FacetValue::Status).to_vec(),
        }
    }
}

impl FacetValue {
    pub fn label(self) -> &'static str {
        match self {
            Self::Input(value) => value.label(),
            Self::Action(value) => value.label(),
            Self::Scope(value) => value.label(),
            Self::Status(value) => value.label(),
        }
    }

    fn matches(self, row: &Row) -> bool {
        match self {
            Self::Input(value) => row.input == value,
            Self::Action(value) => row.action == value,
            Self::Scope(value) => row.scope == value,
            Self::Status(value) => row.status == value,
        }
    }
}

impl GroupBy {
    pub const ALL: [Self; 5] = [
        Self::Prefix,
        Self::Action,
        Self::Scope,
        Self::Input,
        Self::None,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Prefix => "按前缀分组",
            Self::Action => "按动作分组",
            Self::Scope => "按应用范围分组",
            Self::Input => "按输入方式分组",
            Self::None => "不分组",
        }
    }
}

impl Filter {
    pub fn get(&self, facet: Facet) -> Option<FacetValue> {
        match facet {
            Facet::Input => self.input.map(FacetValue::Input),
            Facet::Action => self.action.map(FacetValue::Action),
            Facet::Scope => self.scope.map(FacetValue::Scope),
            Facet::Status => self.status.map(FacetValue::Status),
        }
    }

    /// 设置一个维度；值与维度不符时忽略。
    pub fn set(&mut self, facet: Facet, value: Option<FacetValue>) {
        match (facet, value) {
            (Facet::Input, None) => self.input = None,
            (Facet::Action, None) => self.action = None,
            (Facet::Scope, None) => self.scope = None,
            (Facet::Status, None) => self.status = None,
            (Facet::Input, Some(FacetValue::Input(value))) => self.input = Some(value),
            (Facet::Action, Some(FacetValue::Action(value))) => self.action = Some(value),
            (Facet::Scope, Some(FacetValue::Scope(value))) => self.scope = Some(value),
            (Facet::Status, Some(FacetValue::Status(value))) => self.status = Some(value),
            _ => {}
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn accepts(&self, row: &Row, skip: Option<Facet>) -> bool {
        Facet::ALL.into_iter().all(|facet| {
            Some(facet) == skip || self.get(facet).is_none_or(|value| value.matches(row))
        })
    }
}

pub fn input_kind(spec: &TriggerSpec) -> InputKind {
    if !spec.conditions.is_empty() {
        return InputKind::PointerRegion;
    }
    let terminal = match spec.activation {
        Activation::Wheel(_) => return InputKind::Wheel,
        Activation::Press(button)
        | Activation::Hold { button, .. }
        | Activation::Drag { button, .. } => button,
    };
    if std::iter::once(&terminal)
        .chain(&spec.held)
        .any(|button| matches!(button, Button::Mouse(_)))
    {
        InputKind::MouseButton
    } else {
        InputKind::Keyboard
    }
}

pub fn action_kind(action: &ActionSpec) -> ActionKind {
    match action {
        ActionSpec::Notification => ActionKind::Notification,
        ActionSpec::ShowPanel => ActionKind::Panel,
        ActionSpec::ShowWheel(_) => ActionKind::Wheel,
        ActionSpec::SendShortcut(_) => ActionKind::Shortcut,
        ActionSpec::Window(_) => ActionKind::Window,
        ActionSpec::AdjustSystem(_) => ActionKind::System,
        ActionSpec::Local(_) => ActionKind::Local,
        ActionSpec::Invoke(_) => ActionKind::Package,
        ActionSpec::Browser(_) => ActionKind::Browser,
        ActionSpec::Command(_) => ActionKind::Program,
        ActionSpec::External(_) => ActionKind::External,
        ActionSpec::Plugin(_) => ActionKind::Plugin,
    }
}

pub fn scope_kind(spec: &TriggerSpec) -> ScopeKind {
    if spec.process_scope.is_global() && spec.pointer_scope.is_global() {
        ScopeKind::Global
    } else {
        ScopeKind::Scoped
    }
}

pub fn status_kind(binding: &Binding, ready: bool) -> StatusKind {
    if !binding.enabled {
        StatusKind::Disabled
    } else if ready {
        StatusKind::Enabled
    } else {
        StatusKind::Unready
    }
}

/// 把搜索框文字拆成小写词；每个词都需出现在搜索文本中。
pub fn tokens(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

fn matches_query(row: &Row, tokens: &[String]) -> bool {
    tokens.iter().all(|token| row.text.contains(token.as_str()))
}

/// 在其他维度和搜索条件下，某一选项还能命中多少条；用于下拉中的计数。
pub fn facet_count(rows: &[Row], tokens: &[String], filter: &Filter, value: FacetValue) -> usize {
    let facet = match value {
        FacetValue::Input(_) => Facet::Input,
        FacetValue::Action(_) => Facet::Action,
        FacetValue::Scope(_) => Facet::Scope,
        FacetValue::Status(_) => Facet::Status,
    };
    rows.iter()
        .filter(|row| {
            matches_query(row, tokens) && filter.accepts(row, Some(facet)) && value.matches(row)
        })
        .count()
}

/// 通过搜索与筛选的摘要，保持绑定顺序。
pub fn visible<'a>(rows: &'a [Row], tokens: &[String], filter: &Filter) -> Vec<&'a Row> {
    rows.iter()
        .filter(|row| matches_query(row, tokens) && filter.accepts(row, None))
        .collect()
}

/// 筛选并分组；组按首次出现排列，组内保持绑定顺序。
pub fn browse(rows: &[Row], tokens: &[String], filter: &Filter, group_by: GroupBy) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for row in rows
        .iter()
        .filter(|row| matches_query(row, tokens) && filter.accepts(row, None))
    {
        let (key, label) = match group_by {
            GroupBy::Prefix if row.prefix.is_empty() => (String::new(), "无前缀".to_string()),
            GroupBy::Prefix => (row.prefix.clone(), row.prefix.clone()),
            GroupBy::Action => (row.action.label().into(), row.action.label().into()),
            GroupBy::Scope => (row.scope.label().into(), row.scope.label().into()),
            GroupBy::Input => (row.input.label().into(), row.input.label().into()),
            GroupBy::None => (String::new(), String::new()),
        };
        let index = match groups.iter().position(|group| group.key == key) {
            Some(index) => index,
            None => {
                groups.push(Group {
                    key,
                    label,
                    ids: Vec::new(),
                    disabled: 0,
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[index];
        group.ids.push(row.id);
        if row.status == StatusKind::Disabled {
            group.disabled += 1;
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_core::{MouseButton, WheelDirection};

    fn row(id: u64, text: &str, prefix: &str, action: ActionKind, status: StatusKind) -> Row {
        Row {
            id,
            text: text.to_lowercase(),
            prefix: prefix.into(),
            input: InputKind::Keyboard,
            action,
            scope: ScopeKind::Global,
            status,
            held_keys: Vec::new(),
            held_mouse: Vec::new(),
            terminal: Terminal::Key(0x41),
            has_region: false,
        }
    }

    fn sample() -> Vec<Row> {
        vec![
            row(
                1,
                "光标左移 caps h",
                "Caps",
                ActionKind::Shortcut,
                StatusKind::Enabled,
            ),
            row(
                2,
                "打开终端 ctrl alt t",
                "Ctrl + Alt",
                ActionKind::Program,
                StatusKind::Enabled,
            ),
            row(
                3,
                "光标右移 caps l",
                "Caps",
                ActionKind::Shortcut,
                StatusKind::Disabled,
            ),
            row(
                4,
                "暂停媒体 pause",
                "",
                ActionKind::System,
                StatusKind::Unready,
            ),
            row(
                5,
                "截图 ctrl alt a",
                "Ctrl + Alt",
                ActionKind::Plugin,
                StatusKind::Enabled,
            ),
        ]
    }

    fn spec(held: Vec<Button>, activation: Activation) -> TriggerSpec {
        TriggerSpec {
            pointer_scope: Default::default(),
            wheel: Default::default(),
            keyboard: Default::default(),
            process_scope: Default::default(),
            id: 1,
            held,
            conditions: Vec::new(),
            activation,
        }
    }

    #[test]
    fn groups_by_prefix_in_first_seen_order_and_keep_binding_order() {
        let groups = browse(&sample(), &[], &Filter::default(), GroupBy::Prefix);
        let shape: Vec<_> = groups
            .iter()
            .map(|group| (group.label.as_str(), group.ids.clone(), group.disabled))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("Caps", vec![1, 3], 1),
                ("Ctrl + Alt", vec![2, 5], 0),
                ("无前缀", vec![4], 0),
            ]
        );
    }

    #[test]
    fn every_query_word_must_match() {
        let groups = browse(
            &sample(),
            &tokens("  CAPS   光标 l "),
            &Filter::default(),
            GroupBy::None,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].ids, vec![3]);
        assert!(browse(
            &sample(),
            &tokens("caps 终端"),
            &Filter::default(),
            GroupBy::None
        )
        .is_empty());
    }

    #[test]
    fn facets_combine_and_counts_ignore_their_own_selection() {
        let mut filter = Filter::default();
        filter.set(
            Facet::Action,
            Some(FacetValue::Action(ActionKind::Shortcut)),
        );
        filter.set(Facet::Status, Some(FacetValue::Status(StatusKind::Enabled)));
        let groups = browse(&sample(), &[], &filter, GroupBy::None);
        assert_eq!(groups[0].ids, vec![1]);

        // 状态下拉的计数只受动作筛选约束，不受当前选中的状态约束。
        let rows = sample();
        let count = |status| facet_count(&rows, &[], &filter, FacetValue::Status(status));
        assert_eq!(count(StatusKind::Enabled), 1);
        assert_eq!(count(StatusKind::Disabled), 1);
        assert_eq!(count(StatusKind::Unready), 0);
    }

    #[test]
    fn mismatched_facet_value_is_ignored_and_none_clears() {
        let mut filter = Filter::default();
        filter.set(Facet::Input, Some(FacetValue::Status(StatusKind::Enabled)));
        assert!(filter.is_empty());
        filter.set(Facet::Input, Some(FacetValue::Input(InputKind::Wheel)));
        assert_eq!(filter.input, Some(InputKind::Wheel));
        filter.set(Facet::Input, None);
        assert!(filter.is_empty());
    }

    #[test]
    fn input_kind_follows_terminal_held_and_conditions() {
        let caps = Button::Key(0x14);
        assert_eq!(
            input_kind(&spec(vec![caps], Activation::Press(Button::Key(0x48)))),
            InputKind::Keyboard
        );
        assert_eq!(
            input_kind(&spec(
                vec![Button::Mouse(MouseButton::Right)],
                Activation::Wheel(WheelDirection::Up)
            )),
            InputKind::Wheel
        );
        assert_eq!(
            input_kind(&spec(
                vec![],
                Activation::Hold {
                    button: Button::Mouse(MouseButton::Right),
                    duration_ms: 300
                }
            )),
            InputKind::MouseButton
        );
        assert_eq!(
            input_kind(&spec(
                vec![Button::Mouse(MouseButton::X1)],
                Activation::Press(Button::Key(0x41))
            )),
            InputKind::MouseButton
        );
    }
}
