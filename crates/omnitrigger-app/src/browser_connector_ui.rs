//! 浏览器连接器的简洁安装入口；下载、注册、启动均在后台完成。
use crate::theme::rgb;
use crate::{
    action_library::extension_card,
    browser_connector::{self, ConnectorEvent, Operation, Snapshot},
    caption, icon_button, settings, status_pill, theme, Context, Div, Icon, Tone, Workspace,
};
use gpui::{div, prelude::*, px};
use std::sync::mpsc::{Receiver, TryRecvError};

pub(super) struct BrowserConnectorUi {
    snapshot: Snapshot,
    events: Option<Receiver<ConnectorEvent>>,
    failed: bool,
    status: String,
    profiles: Vec<serde_json::Value>,
}

impl BrowserConnectorUi {
    pub(super) fn new(_: &mut Context<Workspace>) -> Self {
        let (snapshot, failed, status) = match browser_connector::load(&settings::path()) {
            Ok(snapshot) => (snapshot, false, String::new()),
            Err(_) => (
                Snapshot::default(),
                true,
                "暂时无法读取安装状态，请重试。".into(),
            ),
        };
        let events = if snapshot.installed && snapshot.registered && snapshot.enabled {
            browser_connector::start(settings::path(), Operation::Start).ok()
        } else {
            None
        };
        Self {
            snapshot,
            events,
            failed,
            status,
            profiles: Vec::new(),
        }
    }

    pub(super) fn installed(&self) -> bool {
        self.snapshot.installed && self.snapshot.registered && self.snapshot.enabled
    }

    pub(super) fn profiles(&self) -> &[serde_json::Value] {
        &self.profiles
    }
}

impl Workspace {
    pub(super) fn refresh_browser_profiles(&mut self) {
        if !self.browser_connector_ui.installed() || self.browser_connector_ui.events.is_some() {
            return;
        }
        if let Ok(events) = browser_connector::start(settings::path(), Operation::Refresh) {
            self.browser_connector_ui.events = Some(events);
        }
    }

    fn install_browser_connector(&mut self, cx: &mut Context<Self>) {
        if self.browser_connector_ui.events.is_some() {
            return;
        }
        match browser_connector::start(settings::path(), Operation::Install) {
            Ok(events) => {
                self.browser_connector_ui.events = Some(events);
                self.browser_connector_ui.failed = false;
                self.browser_connector_ui.status.clear();
            }
            Err(error) => {
                self.browser_connector_ui.failed = true;
                self.browser_connector_ui.status = error.message;
            }
        }
        cx.notify();
    }

    pub(super) fn poll_browser_connector(&mut self, _: &mut Context<Self>) -> bool {
        let Some(events) = self.browser_connector_ui.events.take() else {
            return false;
        };
        let mut terminal = false;
        let mut changed = false;
        loop {
            match events.try_recv() {
                Ok(ConnectorEvent::Progress(message)) => {
                    self.browser_connector_ui.status = message;
                }
                Ok(ConnectorEvent::Finished {
                    snapshot, profiles, ..
                }) => {
                    self.browser_connector_ui.snapshot = snapshot;
                    self.browser_connector_ui.profiles = profiles;
                    self.browser_connector_ui.failed = false;
                    self.browser_connector_ui.status.clear();
                    terminal = true;
                }
                Ok(ConnectorEvent::Failed(error)) => {
                    self.browser_connector_ui.failed = true;
                    self.browser_connector_ui.status = error.message;
                    terminal = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !terminal {
                        self.browser_connector_ui.failed = true;
                        self.browser_connector_ui.status = "安装未完成，请重试。".into();
                    }
                    terminal = true;
                    changed = true;
                    break;
                }
            }
            changed = true;
        }
        if !terminal {
            self.browser_connector_ui.events = Some(events);
        }
        changed
    }

    pub(super) fn render_browser_connector(&self, cx: &mut Context<Self>) -> Div {
        let ui = &self.browser_connector_ui;
        let busy = ui.events.is_some();
        let ready =
            ui.snapshot.installed && ui.snapshot.registered && ui.snapshot.enabled && !ui.failed;
        let trailing = if ready {
            status_pill("已安装", theme::ACCENT).into_any_element()
        } else {
            icon_button(
                "browser-install",
                Icon::ArrowDown,
                if busy {
                    "正在安装…"
                } else if ui.failed {
                    "重试"
                } else {
                    "安装"
                },
                Tone::Primary,
                !busy,
            )
            .when(!busy, |el| {
                el.on_click(cx.listener(|this, _, _, cx| this.install_browser_connector(cx)))
            })
            .into_any_element()
        };
        extension_card(
            Icon::Plug,
            theme::ACCENT,
            "浏览器连接器",
            "让 OmniTrigger 与 Chrome 连接，在浏览器中执行动作。",
            trailing,
        )
        .when(busy && !ui.status.is_empty(), |el| {
            el.child(caption(ui.status.clone()))
        })
        .when(ui.failed && !ui.status.is_empty(), |el| {
            el.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme::RECORD_TEXT))
                    .child(ui.status.clone()),
            )
        })
    }
}

/// 不打开 GPUI、不安装 Hook 的插件管理与验收入口。
pub(super) fn cli(arguments: &[String]) -> Option<Result<String, String>> {
    let index = arguments
        .iter()
        .position(|arg| arg == "--browser-connector")?;
    Some((|| {
        let value = |offset: usize| {
            arguments
                .get(index + offset)
                .filter(|s| !s.starts_with("--"))
                .cloned()
                .ok_or_else(|| "浏览器连接器命令缺少参数。".to_string())
        };
        let command = value(1)?;
        if command == "status" {
            let snapshot = browser_connector::load(&settings::path()).map_err(|e| e.to_string())?;
            return serde_json::to_string(&snapshot).map_err(|e| e.to_string());
        }
        let operation = match command.as_str() {
            "install" => Operation::Install,
            "register" => Operation::Register {
                extension_id: value(2)?,
            },
            "start" => Operation::Start,
            "stop" => Operation::Stop,
            "refresh" => Operation::Refresh,
            "uninstall" => Operation::Uninstall,
            _ => return Err("支持 status/install/register/start/stop/refresh/uninstall。".into()),
        };
        let events =
            browser_connector::start(settings::path(), operation).map_err(|e| e.to_string())?;
        loop {
            match events.recv().map_err(|_| "连接器后台任务未返回终态。")? {
                ConnectorEvent::Progress(_) => {}
                ConnectorEvent::Failed(error) => return Err(error.to_string()),
                ConnectorEvent::Finished {
                    snapshot,
                    profiles,
                    message,
                } => return Ok(
                    serde_json::json!({"snapshot":snapshot,"profiles":profiles,"message":message})
                        .to_string(),
                ),
            }
        }
    })())
}
