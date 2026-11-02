use crate::{caption, card, theme, updater, Workspace};
use gpui::{div, prelude::*, px, Context, FontWeight};
use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Default)]
pub(super) struct UpdateUi {
    events: Option<Receiver<updater::Event>>,
    release: Option<updater::Release>,
    status: String,
    failed: bool,
}

impl UpdateUi {
    pub fn new() -> Self {
        let error = std::env::var("OMNITRIGGER_UPDATE_ERROR").ok();
        std::env::remove_var("OMNITRIGGER_UPDATE_ERROR");
        Self {
            failed: error.is_some(),
            status: error.unwrap_or_default(),
            ..Self::default()
        }
    }
}

impl Workspace {
    fn update_application(&mut self, cx: &mut Context<Self>) {
        if self.update_ui.events.is_some() {
            return;
        }
        let operation = self
            .update_ui
            .release
            .clone()
            .map_or(updater::Operation::Check, updater::Operation::Install);
        match updater::start(operation, crate::settings::path()) {
            Ok(events) => {
                self.update_ui.events = Some(events);
                self.update_ui.failed = false;
                self.update_ui.status = if self.update_ui.release.is_some() {
                    "正在准备下载…"
                } else {
                    "正在检查更新…"
                }
                .into();
            }
            Err(error) => {
                self.update_ui.failed = true;
                self.update_ui.status = error;
            }
        }
        cx.notify();
    }

    pub(super) fn poll_update(&mut self, cx: &mut Context<Self>) -> bool {
        // 首个成功打开窗口且输入后端就绪的版本才允许更新器删除备份。
        if self.ready {
            updater::mark_healthy();
        }
        let Some(events) = self.update_ui.events.take() else {
            return false;
        };
        let mut terminal = false;
        let mut changed = false;
        loop {
            match events.try_recv() {
                Ok(updater::Event::Progress(message)) => self.update_ui.status = message,
                Ok(updater::Event::Checked(release)) => {
                    self.update_ui.status = match &release {
                        Some(release) => format!("发现新版本 {}", release.tag_name),
                        None => "当前已是最新版本。".into(),
                    };
                    self.update_ui.release = release;
                    terminal = true;
                }
                Ok(updater::Event::Failed(error)) => {
                    self.update_ui.status = format!("更新失败：{error}");
                    self.update_ui.failed = true;
                    terminal = true;
                }
                Ok(updater::Event::Restart) => {
                    self.update_ui.status = "更新已准备好，正在重启…".into();
                    self.backend.take();
                    cx.quit();
                    return true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !terminal {
                        self.update_ui.failed = true;
                        self.update_ui.status = "更新意外中断，请重试。".into();
                    }
                    terminal = true;
                    changed = true;
                    break;
                }
            }
            changed = true;
        }
        if !terminal {
            self.update_ui.events = Some(events);
        }
        changed
    }

    pub(super) fn render_update(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.update_ui.events.is_some();
        let available = self.update_ui.release.is_some();
        let label = if busy {
            "正在处理…"
        } else if available {
            "更新并重启"
        } else {
            "检查更新"
        };
        card()
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("应用更新"),
                            )
                            .child(caption(format!(
                                "OmniTrigger {} · Windows x64 预览版",
                                updater::VERSION
                            ))),
                    )
                    .child(
                        theme::button("application-update", label, theme::Tone::Primary, !busy)
                            .on_click(cx.listener(|this, _, _, cx| this.update_application(cx))),
                    ),
            )
            .child(caption(
                "下载在后台完成，校验通过后自动重启。你的触发器与设置会保留。",
            ))
            .when(!self.update_ui.status.is_empty(), |el| {
                el.child(
                    caption(self.update_ui.status.clone()).when(self.update_ui.failed, |el| {
                        el.text_color(theme::rgb(theme::DANGER))
                    }),
                )
            })
            .when_some(
                self.update_ui
                    .release
                    .as_ref()
                    .and_then(|release| release.body.clone()),
                |el, notes| {
                    el.child(
                        div()
                            .id("update-release-notes")
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .child(caption(notes)),
                    )
                },
            )
    }
}
