//! 本地步骤的小表单编辑器；保存只生成动作快照，不执行任何能力。
use crate::local_action::{LocalAction, LocalStep};
use crate::{
    caption, section_title, small, theme, ActionSpec, MessageInput, Page, Tone, Workspace,
};
use gpui::{div, prelude::*, px, rgb, App, Context, Div, Entity};

#[derive(Clone, Copy, PartialEq, Eq)]
enum StepKind {
    Shortcut,
    Tap,
    Sequence,
    Text,
    Clipboard,
    ReadClipboard,
    Variable,
    Open,
    Delay,
    CopySelection,
    WindowTitle,
    OpenUrls,
    WebSearch,
}
impl StepKind {
    fn label(self) -> &'static str {
        match self {
            Self::Shortcut => "发送快捷键",
            Self::Tap => "单独按键",
            Self::Sequence => "连续组合键",
            Self::Text => "输入文字",
            Self::Clipboard => "写入剪贴板",
            Self::ReadClipboard => "读取剪贴板",
            Self::Variable => "设置变量",
            Self::Open => "打开网址 / 文件",
            Self::Delay => "等待",
            Self::CopySelection => "复制选中文本",
            Self::WindowTitle => "读取窗口标题",
            Self::OpenUrls => "打开文本中的网址",
            Self::WebSearch => "网页搜索",
        }
    }
    fn default_step(self) -> LocalStep {
        match self {
            Self::Shortcut => LocalStep::SendShortcut {
                keys: vec![162, 67],
            },
            Self::Tap => LocalStep::KeyTap { key: 164 },
            Self::Sequence => LocalStep::KeySequence {
                modifiers: vec![162],
                keys: vec![75, 90],
            },
            Self::Text => LocalStep::InputText {
                text: "请输入文字".into(),
            },
            Self::Clipboard => LocalStep::SetClipboard {
                text: String::new(),
            },
            Self::ReadClipboard => LocalStep::ReadClipboard {
                variable: "clipboard".into(),
            },
            Self::Variable => LocalStep::SetVariable {
                name: "value".into(),
                value: String::new(),
            },
            Self::Open => LocalStep::Open {
                target: "https://example.com".into(),
            },
            Self::Delay => LocalStep::Delay { milliseconds: 100 },
            Self::CopySelection => LocalStep::CopySelection {
                variable: "selection".into(),
            },
            Self::WindowTitle => LocalStep::ReadWindowTitle {
                variable: "title".into(),
                strip_app_name: true,
            },
            Self::OpenUrls => LocalStep::OpenUrls {
                text: "${selection}".into(),
            },
            Self::WebSearch => LocalStep::WebSearch {
                url: "https://www.google.com/search?q=%s".into(),
                query: "${selection}".into(),
            },
        }
    }
}

pub fn initial_action() -> LocalAction {
    LocalAction {
        version: 1,
        steps: vec![LocalStep::Delay { milliseconds: 100 }],
    }
}

fn display_text(value: &str) -> String {
    if value.contains(['\n', '\r', '\t']) || value.starts_with('"') {
        serde_json::to_string(value).unwrap_or_default()
    } else {
        value.into()
    }
}
fn parse_text(value: &str) -> Result<String, String> {
    if value.starts_with('"') {
        serde_json::from_str(value)
            .map_err(|_| "以双引号开头时请填写完整 JSON 字符串；换行写为 \\n。".into())
    } else {
        Ok(value.into())
    }
}
fn parse_key(value: &str) -> Result<u16, String> {
    let explicit = value.trim().to_ascii_uppercase();
    if let Some(hex) = explicit.strip_prefix("VK_") {
        let key = u16::from_str_radix(hex, 16)
            .map_err(|_| "虚拟键码格式有误，例如 VK_10。".to_string())?;
        omnitrigger_windows::validate_key_tap(key)?;
        return Ok(key);
    }
    if let Ok(shortcut) = crate::shortcut::ShortcutAction::parse(value) {
        if shortcut.keys.len() == 1 {
            return Ok(shortcut.keys[0]);
        }
    }
    let combo = crate::shortcut::ShortcutAction::parse(&format!("{value}+F24"))?;
    if combo.keys.len() == 2 {
        Ok(combo.keys[0])
    } else {
        Err("请填写一个按键，例如 Alt、Enter、F5。".into())
    }
}

pub struct LocalEditor {
    action: LocalAction,
    selected: Option<usize>,
    first: Entity<MessageInput>,
    second: Entity<MessageInput>,
}
impl LocalEditor {
    pub fn new(cx: &mut Context<Workspace>) -> Self {
        let first = cx.new(|cx| MessageInput::new(cx).with_character_limit(262144));
        let second = cx.new(|cx| MessageInput::new(cx).with_character_limit(262144));
        for input in [&first, &second] {
            cx.observe(input, |_, _, cx| cx.notify()).detach();
        }
        let mut editor = Self {
            action: initial_action(),
            selected: None,
            first,
            second,
        };
        editor.select(0, cx);
        editor
    }
    pub fn load(&mut self, action: &LocalAction, cx: &mut Context<Workspace>) {
        self.action = action.clone();
        self.selected = None;
        if !action.steps.is_empty() {
            self.select(0, cx);
        }
    }
    fn select(&mut self, index: usize, cx: &mut Context<Workspace>) {
        let Some(step) = self.action.steps.get(index) else {
            return;
        };
        let (first, second) = match step {
            LocalStep::SendShortcut { keys } => (
                crate::shortcut::ShortcutAction { keys: keys.clone() }.display(),
                String::new(),
            ),
            LocalStep::KeyTap { key } => (
                crate::shortcut::ShortcutAction { keys: vec![*key] }.display(),
                String::new(),
            ),
            LocalStep::KeySequence { modifiers, keys } => (
                modifiers
                    .iter()
                    .map(|key| crate::shortcut::ShortcutAction { keys: vec![*key] }.display())
                    .collect::<Vec<_>>()
                    .join("+"),
                keys.iter()
                    .map(|key| crate::shortcut::ShortcutAction { keys: vec![*key] }.display())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            LocalStep::InputText { text } | LocalStep::SetClipboard { text } => {
                (display_text(text), String::new())
            }
            LocalStep::ReadClipboard { variable } | LocalStep::CopySelection { variable } => {
                (variable.clone(), String::new())
            }
            LocalStep::ReadWindowTitle {
                variable,
                strip_app_name,
            } => (
                variable.clone(),
                if *strip_app_name { "是" } else { "" }.into(),
            ),
            LocalStep::OpenUrls { text } => (display_text(text), String::new()),
            LocalStep::WebSearch { url, query } => (url.clone(), display_text(query)),
            LocalStep::SetVariable { name, value } => (name.clone(), display_text(value)),
            LocalStep::Open { target } => (target.clone(), String::new()),
            LocalStep::Delay { milliseconds } => (milliseconds.to_string(), String::new()),
        };
        self.selected = Some(index);
        self.first.update(cx, |input, cx| input.set_text(first, cx));
        self.second
            .update(cx, |input, cx| input.set_text(second, cx));
    }
    fn draft(&self, cx: &App) -> Result<LocalAction, String> {
        let mut action = self.action.clone();
        if let Some(index) = self.selected {
            let first = self.first.read(cx).text();
            let second = self.second.read(cx).text();
            let step = action.steps.get_mut(index).ok_or("选中步骤已不存在。")?;
            *step = match step {
                LocalStep::SendShortcut { .. } => LocalStep::SendShortcut {
                    keys: crate::shortcut::ShortcutAction::parse(&first)?.keys,
                },
                LocalStep::KeyTap { .. } => LocalStep::KeyTap {
                    key: parse_key(&first)?,
                },
                LocalStep::KeySequence { .. } => LocalStep::KeySequence {
                    modifiers: if first.trim().is_empty() {
                        vec![]
                    } else {
                        first
                            .split('+')
                            .map(|part| parse_key(part.trim()))
                            .collect::<Result<_, _>>()?
                    },
                    keys: second
                        .split(',')
                        .map(|part| parse_key(part.trim()))
                        .collect::<Result<_, _>>()?,
                },
                LocalStep::InputText { .. } => LocalStep::InputText {
                    text: parse_text(&first)?,
                },
                LocalStep::SetClipboard { .. } => LocalStep::SetClipboard {
                    text: parse_text(&first)?,
                },
                LocalStep::ReadClipboard { .. } => LocalStep::ReadClipboard {
                    variable: first.trim().into(),
                },
                LocalStep::SetVariable { .. } => LocalStep::SetVariable {
                    name: first.trim().into(),
                    value: parse_text(&second)?,
                },
                LocalStep::Open { .. } => LocalStep::Open { target: first },
                LocalStep::CopySelection { .. } => LocalStep::CopySelection {
                    variable: first.trim().into(),
                },
                LocalStep::ReadWindowTitle { .. } => LocalStep::ReadWindowTitle {
                    variable: first.trim().into(),
                    strip_app_name: match second.trim() {
                        "" | "否" => false,
                        "是" => true,
                        _ => return Err("去掉应用名请填写“是”，或留空保留完整标题。".into()),
                    },
                },
                LocalStep::OpenUrls { .. } => LocalStep::OpenUrls {
                    text: parse_text(&first)?,
                },
                LocalStep::WebSearch { .. } => LocalStep::WebSearch {
                    url: first.trim().into(),
                    query: parse_text(&second)?,
                },
                LocalStep::Delay { .. } => LocalStep::Delay {
                    milliseconds: first
                        .trim()
                        .parse()
                        .map_err(|_| "等待时间应为 0–5000 的整数毫秒。")?,
                },
            };
        }
        Ok(action)
    }
    pub fn snapshot(&self, cx: &App) -> Result<LocalAction, String> {
        let action = self.draft(cx)?;
        action.validate()?;
        Ok(action)
    }
}

impl Workspace {
    pub(super) fn stop_local_action(&mut self, wheel: bool, cx: &mut Context<Self>) {
        let id = if wheel {
            self.wheel_selected
                .as_ref()
                .zip(self.wheel_slot_selected)
                .and_then(|(wheel, slot)| {
                    self.wheel_runtime_ids.get(&(wheel.clone(), slot)).copied()
                })
        } else {
            self.pending.as_ref().map(|spec| spec.id).or(self.selected)
        };
        let count = id.map_or(0, |id| {
            if let Some(invocation) = self.local_invocations.get(&id) {
                usize::from(self.local_executor.cancel(*invocation))
            } else {
                self.local_executor.cancel_id(id)
            }
        });
        self.status = format!("已请求取消 {count} 个本地动作；已完成的步骤不回滚。");
        cx.notify();
    }
    fn local_editor_ref(&self, wheel: bool) -> &LocalEditor {
        if wheel {
            &self.wheel_local_editor
        } else {
            &self.local_editor
        }
    }
    fn local_editor_mut(&mut self, wheel: bool) -> &mut LocalEditor {
        if wheel {
            &mut self.wheel_local_editor
        } else {
            &mut self.local_editor
        }
    }
    fn commit_local_draft(&mut self, wheel: bool, cx: &mut Context<Self>) -> bool {
        match self.local_editor_ref(wheel).draft(cx) {
            Ok(action) => {
                self.local_editor_mut(wheel).action = action;
                true
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                false
            }
        }
    }
    pub(super) fn choose_local_action(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.draft_action, ActionSpec::Local(_)) {
            let action = initial_action();
            self.local_editor.load(&action, cx);
            self.draft_action = ActionSpec::Local(action);
        }
        cx.notify();
    }
    pub(super) fn new_local_action(&mut self, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let Some(id) = self.next_binding_id else {
            self.error = Some("触发器编号已耗尽。".into());
            return;
        };
        self.next_binding_id = id.checked_add(1);
        self.clear_pending();
        let spec = omnitrigger_core::TriggerSpec {
            id,
            held: vec![],
            conditions: vec![],
            activation: omnitrigger_core::Activation::Press(omnitrigger_core::Button::Key(0x77)),
        };
        self.load_trigger_fields(&spec, cx);
        self.conflict = self.check_draft_trigger(&spec, None).err();
        self.error = None;
        self.confirm_delete = None;
        self.pending = Some(spec);
        self.edited_spec = None;
        self.draft_action = ActionSpec::Local(initial_action());
        self.local_editor.load(&initial_action(), cx);
        self.message
            .update(cx, |input, cx| input.set_text("本地步骤动作", cx));
        self.page = Page::Triggers;
        self.status = "已创建本地动作草稿；配置步骤与触发组合，保存后生效。".into();
        cx.notify();
    }
    pub(super) fn local_action_fields(&self, wheel: bool, cx: &mut Context<Self>) -> Div {
        let editor = self.local_editor_ref(wheel);
        let mut list = div().flex().flex_col().gap(px(5.));
        for (index, step) in editor.action.steps.iter().enumerate() {
            list = list.child(
                small(crate::button(
                    (
                        if wheel {
                            "wheel-local-step"
                        } else {
                            "local-step"
                        },
                        index as u64,
                    ),
                    format!("{:02}  {}", index + 1, step.description()),
                    if editor.selected == Some(index) {
                        Tone::Primary
                    } else {
                        Tone::Secondary
                    },
                    !self.recording,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.recording && this.commit_local_draft(wheel, cx) {
                        this.local_editor_mut(wheel).select(index, cx);
                        cx.notify();
                    }
                })),
            );
        }
        let mut fields = div()
            .flex()
            .flex_col()
            .gap(px(9.))
            .child(section_title("本地步骤"))
            .child(list);
        if let Some(index) = editor.selected {
            let step = &editor.action.steps[index];
            let (label,hint)=match step {
                LocalStep::SendShortcut {..}=>("快捷键",Some("如 Ctrl+C、Ctrl+Shift+T。")),
                LocalStep::KeyTap {..}=>("单独按键",Some("支持单独 Alt、Ctrl、Shift、Win，或 Enter、F5 等。")),
                LocalStep::KeySequence {..}=>("全程按住的修饰键",Some("如 Ctrl 或 Alt；可留空。下方按键依次按下和释放。")),
                LocalStep::InputText {..}|LocalStep::SetClipboard {..}=>("文字内容",Some("普通文字直接输入；多行可填 JSON 字符串，如 \"第一行\\n第二行\"。支持 ${变量名}，$$ 表示 $。")),
                LocalStep::ReadClipboard {..}|LocalStep::SetVariable {..}|LocalStep::ReadWindowTitle {..}=>("变量名",Some("字母或下划线开头，最多 32 个 ASCII 标识符字符。")),
                LocalStep::CopySelection {..}=>("变量名",Some("向原窗口发送 Ctrl+C 并保存复制的文字；复制结果会留在剪贴板。")),
                LocalStep::OpenUrls {..}=>("包含网址的文字",Some("通常填 ${selection}；按出现顺序打开其中的 http(s) 与 www. 网址，最多 8 个。")),
                LocalStep::WebSearch {..}=>("搜索网址",Some("用 %s 表示搜索词，例如 https://www.bing.com/search?q=%s。")),
                LocalStep::Open {..}=>("网址 / 绝对路径",Some("支持 http、https、mailto、about 和本地文件或文件夹。")),
                LocalStep::Delay {..}=>("等待时间（毫秒）",Some("0–5000；整个动作显式等待最多 10000ms。")),
            };
            fields = fields.child(Self::input_field(label, &editor.first, hint));
            if matches!(step, LocalStep::KeySequence { .. }) {
                fields = fields.child(Self::input_field(
                    "依次发送的按键",
                    &editor.second,
                    Some("逗号分隔，例如 K, Z；修饰键在整组按键之间保持按住。"),
                ));
            }
            if matches!(step, LocalStep::ReadWindowTitle { .. }) {
                fields = fields.child(Self::input_field(
                    "去掉应用名",
                    &editor.second,
                    Some("填“是”去掉标题末尾的“ - 应用名”；留空保留完整标题。"),
                ));
            }
            if matches!(step, LocalStep::WebSearch { .. }) {
                fields = fields.child(Self::input_field(
                    "搜索词",
                    &editor.second,
                    Some("可引用 ${变量名}；取前 128 个字符并自动编码。"),
                ));
            }
            if matches!(step, LocalStep::SetVariable { .. }) {
                fields = fields.child(Self::input_field(
                    "变量值",
                    &editor.second,
                    Some("可引用前面定义的 ${变量名}；普通文字可直接输入。"),
                ));
            }
            let count = editor.action.steps.len();
            let mut controls = div().flex().flex_wrap().gap(px(6.));
            for (key, label, delta, allowed) in [
                (0u64, "上移", -1isize, index > 0),
                (1, "下移", 1, index + 1 < count),
                (2, "删除", 0, count > 1),
            ] {
                controls = controls.child(
                    small(crate::button(
                        (
                            if wheel {
                                "wheel-local-order"
                            } else {
                                "local-order"
                            },
                            key,
                        ),
                        label,
                        Tone::Secondary,
                        allowed && !self.recording,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !allowed
                            || this.recording
                            || (delta != 0 && !this.commit_local_draft(wheel, cx))
                        {
                            return;
                        }
                        let editor = this.local_editor_mut(wheel);
                        let next = if delta == 0 {
                            editor.action.steps.remove(index);
                            index.min(editor.action.steps.len() - 1)
                        } else {
                            let next = index.checked_add_signed(delta).expect("已核验移动范围");
                            editor.action.steps.swap(index, next);
                            next
                        };
                        editor.select(next, cx);
                        cx.notify();
                    })),
                );
            }
            fields = fields.child(controls);
        }
        let mut add = div().grid().grid_cols(2).gap(px(5.));
        for (index, kind) in [
            StepKind::Shortcut,
            StepKind::Tap,
            StepKind::Sequence,
            StepKind::Text,
            StepKind::Clipboard,
            StepKind::ReadClipboard,
            StepKind::Variable,
            StepKind::Open,
            StepKind::Delay,
            StepKind::CopySelection,
            StepKind::WindowTitle,
            StepKind::OpenUrls,
            StepKind::WebSearch,
        ]
        .into_iter()
        .enumerate()
        {
            let enabled = editor.action.steps.len() < 32 && !self.recording;
            add = add.child(
                small(crate::button(
                    (
                        if wheel {
                            "wheel-local-add"
                        } else {
                            "local-add"
                        },
                        index as u64,
                    ),
                    format!("+ {}", kind.label()),
                    Tone::Ghost,
                    enabled,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !enabled || this.recording || !this.commit_local_draft(wheel, cx) {
                        return;
                    }
                    let editor = this.local_editor_mut(wheel);
                    editor.action.steps.push(kind.default_step());
                    editor.select(editor.action.steps.len() - 1, cx);
                    cx.notify();
                })),
            );
        }
        fields.child(section_title("添加步骤")).child(add)
            .child(caption("保存只更新配置；运行会真实执行步骤。失败或取消后停止后续步骤，已经完成的操作不会回滚。"))
            .when_some(editor.snapshot(cx).err(),|el,error|el.child(div().text_size(px(11.)).text_color(rgb(theme::DANGER)).child(error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_generic_modifier_taps_remain_editable_without_rewriting_the_key() {
        for key in [0x10, 0x11, 0x12] {
            let label = crate::shortcut::ShortcutAction { keys: vec![key] }.display();
            assert_eq!(parse_key(&label).unwrap(), key);
            assert_eq!(parse_key(&label.to_ascii_lowercase()).unwrap(), key);
        }
        assert!(parse_key("VK_0000").is_err());
        assert!(parse_key("VK_10000").is_err());
    }
    #[test]
    fn text_forms_preserve_quotes_newlines_paths_and_dollar_literals() {
        for value in [
            "普通文字",
            "C:\\new\\file",
            "第一行\n第二行",
            "\"引号\"",
            "${value} $$",
        ] {
            assert_eq!(parse_text(&display_text(value)).unwrap(), value);
        }
        assert!(parse_text("\"未闭合").is_err());
        assert_eq!(parse_key("Alt").unwrap(), 164);
        assert_eq!(parse_key("Enter").unwrap(), 13);
        assert!(parse_key("Ctrl+K").is_err());
    }
}
