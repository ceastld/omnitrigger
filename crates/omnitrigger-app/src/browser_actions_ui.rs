//! 浏览器动作的紧凑编辑入口；编辑与保存不会执行 Chrome API。
use crate::browser_actions::{BrowserAction, BrowserCommand, BrowserOutcome};
use crate::{
    caption, card, icon_button, icon_tile, settings, theme, ActionSpec, Context, Div, Icon,
    MessageInput, Page, Tone, Workspace,
};
use gpui::{div, prelude::*, px, App, Entity, FontWeight};

pub(super) struct BrowserEditor {
    url: Entity<MessageInput>,
    method: Entity<MessageInput>,
    args: Entity<MessageInput>,
}

impl BrowserEditor {
    pub(super) fn new(cx: &mut Context<Workspace>) -> Self {
        let url = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(8192);
            input.set_placeholder("留空打开新标签页");
            input
        });
        let method = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(128);
            input.set_placeholder("例如 bookmarks.getTree");
            input.set_text("bookmarks.getTree", cx);
            input
        });
        let args = cx.new(|cx| {
            let mut input = MessageInput::new(cx)
                .with_json_text()
                .with_character_limit(64 * 1024);
            input.set_placeholder("JSON 参数数组，例如 []");
            input.set_text("[]", cx);
            input
        });
        for input in [&url, &method, &args] {
            cx.observe(input, |_, _, cx| cx.notify()).detach();
        }
        Self { url, method, args }
    }

    pub(super) fn load(&mut self, action: &BrowserAction, cx: &mut Context<Workspace>) {
        let url = match &action.command {
            BrowserCommand::NewTab { url } => url.clone().unwrap_or_default(),
            _ => String::new(),
        };
        let (method, args) = match &action.command {
            BrowserCommand::Call { method, args } => (
                method.clone(),
                serde_json::to_string(args).unwrap_or_else(|_| "[]".into()),
            ),
            _ => ("bookmarks.getTree".into(), "[]".into()),
        };
        self.url.update(cx, |input, cx| input.set_text(url, cx));
        self.method
            .update(cx, |input, cx| input.set_text(method, cx));
        self.args.update(cx, |input, cx| input.set_text(args, cx));
    }

    pub(super) fn snapshot(
        &self,
        action: &BrowserAction,
        cx: &App,
    ) -> Result<BrowserAction, String> {
        let mut action = action.clone();
        action.command = match &action.command {
            BrowserCommand::NewTab { .. } => {
                let text = self.url.read(cx).text().trim().to_string();
                BrowserCommand::NewTab {
                    url: (!text.is_empty()).then_some(text),
                }
            }
            BrowserCommand::Call { .. } => BrowserCommand::Call {
                method: self.method.read(cx).text().trim().to_string(),
                args: serde_json::from_str(&self.args.read(cx).text())
                    .map_err(|error| format!("Chrome API 参数需要 JSON 数组：{error}"))?,
            },
            command => command.clone(),
        };
        action.validate()?;
        Ok(action)
    }
}

fn commands() -> Vec<BrowserCommand> {
    vec![
        BrowserCommand::NewTab { url: None },
        BrowserCommand::CloseTab,
        BrowserCommand::ReloadTab,
        BrowserCommand::DuplicateTab,
        BrowserCommand::TogglePin,
        BrowserCommand::ToggleMute,
        BrowserCommand::NextTab,
        BrowserCommand::PreviousTab,
        BrowserCommand::GoBack,
        BrowserCommand::GoForward,
        BrowserCommand::RestoreTab,
        BrowserCommand::NewWindow,
        BrowserCommand::Call {
            method: "bookmarks.getTree".into(),
            args: vec![],
        },
    ]
}

fn short_label(command: &BrowserCommand) -> &'static str {
    match command {
        BrowserCommand::NewTab { .. } => "新建标签",
        BrowserCommand::CloseTab => "关闭标签",
        BrowserCommand::ReloadTab => "刷新标签",
        BrowserCommand::DuplicateTab => "复制标签",
        BrowserCommand::TogglePin => "固定 / 取消",
        BrowserCommand::ToggleMute => "静音 / 恢复",
        BrowserCommand::NextTab => "下一个",
        BrowserCommand::PreviousTab => "上一个",
        BrowserCommand::GoBack => "后退",
        BrowserCommand::GoForward => "前进",
        BrowserCommand::RestoreTab => "恢复关闭",
        BrowserCommand::NewWindow => "新建窗口",
        BrowserCommand::Call { .. } => "Chrome API",
    }
}

impl Workspace {
    pub(super) fn browser_catalog(&self, query: &str, cx: &mut Context<Self>) -> Option<Div> {
        let mut search = "浏览器 Chrome 标签页 书签 窗口 browser tabs bookmarks api".to_lowercase();
        for command in commands() {
            search.push_str(command.label());
            search.push_str(command.description());
        }
        if !search.contains(query) {
            return None;
        }
        Some(
            card()
                .p(px(16.))
                .flex()
                .items_center()
                .gap(px(14.))
                .child(icon_tile(Icon::Browser, theme::ACCENT_2, 40.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("浏览器操作"),
                        )
                        .child(caption(
                            "管理标签页与窗口、恢复关闭的页面，或直接调用 Chrome API。",
                        )),
                )
                .child(
                    icon_button(
                        "choose-browser-action",
                        Icon::Bolt,
                        "设置触发",
                        Tone::Primary,
                        !self.recording,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.choose_browser_action(cx))),
                ),
        )
    }

    pub(super) fn choose_browser_action(&mut self, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        if self.pending.is_none() && !(self.package_picker_for_draft && self.selected.is_some()) {
            let Some(id) = self.draft_id(None) else {
                cx.notify();
                return;
            };
            let spec = omnitrigger_core::TriggerSpec {
                id,
                held: vec![],
                conditions: vec![],
                activation: omnitrigger_core::Activation::Press(omnitrigger_core::Button::Key(
                    0x77,
                )),
            };
            self.load_trigger_fields(&spec, cx);
            self.conflict = self.check_draft_trigger(&spec, None).err();
            self.pending = Some(spec);
            self.edited_spec = None;
        }
        let action = BrowserAction {
            command: BrowserCommand::ReloadTab,
            profile: None,
        };
        self.message.update(cx, |input, cx| {
            input.set_text(action.label().to_string(), cx)
        });
        self.load_action(ActionSpec::Browser(action), cx);
        self.refresh_browser_profiles();
        self.package_picker_for_draft = false;
        self.open_page(Page::Triggers, cx);
        self.status = "浏览器动作草稿已就绪，保存触发组合后生效。".into();
        self.error = None;
        self.confirm_delete = None;
        cx.notify();
    }

    pub(super) fn browser_action_fields(&self, cx: &mut Context<Self>) -> Div {
        let ActionSpec::Browser(action) = &self.draft_action else {
            return div();
        };
        let mut body = div().flex().flex_col().gap(px(10.));
        if !self.browser_connector_ui.installed() {
            body = body
                .child(caption("请先安装浏览器连接器，并在 Chrome 中启用扩展。"))
                .child(
                    icon_button(
                        "browser-go-install",
                        Icon::Plug,
                        "前往扩展",
                        Tone::Primary,
                        !self.recording,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.open_page(Page::Extensions, cx))),
                );
        }
        let mut options = div().grid().grid_cols(3).gap(px(5.));
        for (index, command) in commands().into_iter().enumerate() {
            let candidate = BrowserAction {
                command,
                profile: action.profile.clone(),
            };
            let active = std::mem::discriminant(&candidate.command)
                == std::mem::discriminant(&action.command);
            options = options.child(
                crate::button(
                    ("browser-operation", index),
                    short_label(&candidate.command),
                    if active {
                        Tone::Primary
                    } else {
                        Tone::Secondary
                    },
                    !self.recording,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let previous_label = if let ActionSpec::Browser(previous) = &this.draft_action {
                        Some(previous.label())
                    } else {
                        None
                    };
                    let current_message = this.message.read(cx).text();
                    if previous_label.is_some_and(|label| current_message == label) {
                        this.message.update(cx, |input, cx| {
                            input.set_text(candidate.label().to_string(), cx)
                        });
                    }
                    this.load_action(ActionSpec::Browser(candidate.clone()), cx);
                    cx.notify();
                })),
            );
        }
        body = body.child(options).child(caption(action.description()));
        if matches!(action.command, BrowserCommand::NewTab { .. }) {
            body = body.child(Self::input_field(
                "打开网址",
                &self.browser_editor.url,
                None,
            ));
        }
        if matches!(action.command, BrowserCommand::Call { .. }) {
            body = body
                .child(Self::input_field(
                    "Chrome API",
                    &self.browser_editor.method,
                    Some("不带 chrome. 前缀，例如 bookmarks.search。"),
                ))
                .child(Self::input_field(
                    "参数数组",
                    &self.browser_editor.args,
                    Some(r#"例如 [{"query":"Rust"}]；无参数填 []。"#),
                ))
                .child(caption(
                    "调用保留 Chrome 的权限限制。需要额外权限时，在扩展弹窗中启用对应功能。",
                ));
        }
        if self.browser_connector_ui.profiles().len() > 1 || action.profile.is_some() {
            let profiles = self.browser_connector_ui.profiles();
            let mut choices = div()
                .flex()
                .flex_wrap()
                .gap(px(5.))
                .child(caption("目标浏览器"));
            choices = choices.child(
                crate::small(crate::button(
                    "browser-profile-auto",
                    "自动",
                    if action.profile.is_none() {
                        Tone::Primary
                    } else {
                        Tone::Secondary
                    },
                    !self.recording,
                ))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let ActionSpec::Browser(action) = &mut this.draft_action {
                        action.profile = None;
                    }
                    cx.notify();
                })),
            );
            for (index, profile) in profiles.iter().enumerate() {
                let Some(id) = profile["browser_instance_id"].as_str() else {
                    continue;
                };
                let id = id.to_string();
                let name = profile["name"].as_str().unwrap_or("Chrome").to_string();
                let selected = action.profile.as_deref() == Some(id.as_str());
                choices = choices.child(
                    crate::small(crate::button(
                        ("browser-profile", index),
                        name,
                        if selected {
                            Tone::Primary
                        } else {
                            Tone::Secondary
                        },
                        !self.recording,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let ActionSpec::Browser(action) = &mut this.draft_action {
                            action.profile = Some(id.clone());
                        }
                        cx.notify();
                    })),
                );
            }
            body = body.child(choices).child(caption(
                "自动模式仅在一个浏览器配置连接时执行；存在多个连接时请选择目标。",
            ));
        }
        body
    }

    pub(super) fn poll_browser_actions(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..16 {
            let Ok(event) = self.browser_executor.try_recv() else {
                break;
            };
            let (message, failed) = match event.outcome {
                BrowserOutcome::Started => ("正在执行浏览器动作".into(), false),
                BrowserOutcome::Succeeded { output: _ } => ("浏览器动作已完成".into(), false),
                BrowserOutcome::Failed {
                    code,
                    message,
                    effect_state,
                } => (
                    format!(
                        "{message}（{code}）{}",
                        if effect_state.as_deref() == Some("unknown") {
                            "；结果未知，未自动重试"
                        } else {
                            ""
                        }
                    ),
                    true,
                ),
            };
            if failed {
                self.action_error = Some((event.id, message.clone()));
            } else if self
                .action_error
                .as_ref()
                .is_some_and(|(id, _)| *id == event.id)
            {
                self.action_error = None;
            }
            self.record_action_status(event.id, message);
            changed = true;
        }
        changed
    }
}

/// 不安装 Hook 的开发入口；输出保留完整 JSON，界面日志只显示状态。
pub(super) fn cli(arguments: &[String]) -> Option<Result<String, String>> {
    let index = arguments.iter().position(|arg| arg == "--browser-action")?;
    Some((|| {
        use std::io::Read;
        let path = arguments
            .get(index + 1)
            .filter(|path| !path.starts_with("--"))
            .ok_or("--browser-action 后需要浏览器动作 JSON 文件路径。")?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|error| error.to_string())?
            .take(128 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > 128 * 1024 {
            return Err("浏览器动作不能超过 128 KiB。".into());
        }
        let action: BrowserAction = serde_json::from_slice(&bytes)
            .map_err(|error| format!("浏览器动作格式有误：{error}"))?;
        action.validate()?;
        let executor = crate::browser_actions::BrowserExecutor::new(settings::path());
        executor.submit(0, action)?;
        loop {
            match executor.try_recv() {
                Ok(event) => match event.outcome {
                    BrowserOutcome::Started => {},
                    BrowserOutcome::Succeeded { output } => return Ok(output.to_string()),
                    BrowserOutcome::Failed { code, message, effect_state } => return Err(serde_json::json!({"code":code,"message":message,"effect_state":effect_state}).to_string()),
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => std::thread::sleep(std::time::Duration::from_millis(20)),
                Err(_) => return Err("浏览器动作后台线程已退出。".into()),
            }
        }
    })())
}
