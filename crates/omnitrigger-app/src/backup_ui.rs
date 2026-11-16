//! 设置页在当前档选择历史版本，新建隔离测试副本。
use crate::{caption, card, profiles, settings, theme, Workspace};
use gpui::{div, prelude::*, px, Context, Div, FontWeight};
use std::{path::PathBuf, time::SystemTime};

#[derive(Default)]
pub(super) struct BackupRestore {
    choices: Option<[Versions; 2]>,
    busy: bool,
    alias: Option<String>,
    created: Option<(PathBuf, String)>,
    notice: Option<String>,
}

#[derive(Default)]
struct Versions {
    saved: Vec<(u64, SystemTime)>,
    index: usize, // 0 为当前文件，其余从最新的备份排起。
}

impl Versions {
    fn selected(&self) -> Option<(u64, SystemTime)> {
        self.index
            .checked_sub(1)
            .and_then(|i| self.saved.get(i))
            .copied()
    }

    fn label(&self) -> String {
        let Some((number, time)) = self.selected() else {
            return "当前配置".into();
        };
        let age = time.elapsed().unwrap_or_default().as_secs();
        let ago = match age {
            0..60 => "刚刚".into(),
            60..3600 => format!("{} 分钟前", age / 60),
            3600..86400 => format!("{} 小时前", age / 3600),
            _ => format!("{} 天前", age / 86400),
        };
        format!("第 {number} 份 · {ago}")
    }
}

impl Workspace {
    fn choose_backup_versions(&mut self, cx: &mut Context<Self>) {
        if self.backup_restore.busy {
            return;
        }
        self.backup_restore = BackupRestore {
            busy: true,
            ..Default::default()
        };
        let config = settings::path();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let read = |name| {
                        settings::edit_versions(&config, name)
                            .map(|saved| Versions { saved, index: 0 })
                    };
                    let choices = [read("bindings.json")?, read("wheels.json")?];
                    if choices.iter().all(|v| v.saved.is_empty()) {
                        return Err("本档还没有可恢复的编辑备份。".to_owned());
                    }
                    Ok(choices)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.backup_restore.busy = false;
                match result {
                    Ok(choices) => this.backup_restore.choices = Some(choices),
                    Err(error) => this.backup_restore.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn backup_version_row(
        &self,
        index: usize,
        title: &'static str,
        versions: &Versions,
        cx: &Context<Self>,
    ) -> Div {
        let newer = versions.index > 0 && !self.backup_restore.busy;
        let older = versions.index < versions.saved.len() && !self.backup_restore.busy;
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(title)
            .child(
                theme::button(
                    ("newer-backup", index),
                    "较新",
                    theme::Tone::Secondary,
                    newer,
                )
                .when(newer, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        this.change_backup_version(index, false, cx)
                    }))
                }),
            )
            .child(caption(versions.label()))
            .child(
                theme::button(
                    ("older-backup", index),
                    "较早",
                    theme::Tone::Secondary,
                    older,
                )
                .when(older, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        this.change_backup_version(index, true, cx)
                    }))
                }),
            )
    }

    fn change_backup_version(&mut self, index: usize, older: bool, cx: &mut Context<Self>) {
        if self.backup_restore.busy {
            return;
        }
        let Some(choices) = &mut self.backup_restore.choices else {
            return;
        };
        let row = &mut choices[index];
        row.index = if older {
            (row.index + 1).min(row.saved.len())
        } else {
            row.index.saturating_sub(1)
        };
        self.backup_restore.created = None;
        self.backup_restore.alias = None;
        self.backup_restore.notice = None;
        cx.notify();
    }

    fn restore_backup(&mut self, cx: &mut Context<Self>) {
        if self.backup_restore.busy {
            return;
        }
        let Some([bindings, wheels]) = self.backup_restore.choices.as_ref() else {
            return;
        };
        let bindings = bindings.selected().map(|v| v.0);
        let wheels = wheels.selected().map(|v| v.0);
        if bindings.is_none() && wheels.is_none() {
            return;
        }
        let config = settings::path();
        let created = self.backup_restore.created.clone();
        let alias = self
            .backup_restore
            .alias
            .get_or_insert_with(profiles::new_restore_alias)
            .clone();
        self.backup_restore.busy = true;
        self.backup_restore.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some((root, key)) = created {
                        profiles::open_test_profile(&root, &key)?;
                        Ok(None)
                    } else {
                        profiles::create_backup_copy(&config, &alias, bindings, wheels).map(Some)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.backup_restore.busy = false;
                match result {
                    Ok(Some((root, key))) => {
                        this.backup_restore.notice = Some(format!("已创建测试副本 {key}。"));
                        this.backup_restore.created = Some((root, key));
                    }
                    Ok(None) => {}
                    Err(error) => this.backup_restore.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_backup_settings(&self, cx: &Context<Self>) -> Div {
        let state = &self.backup_restore;
        let standard = settings::path()
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("bindings.json"));
        let choose = state.choices.is_none() && standard;
        card()
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("配置备份"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .when(choose, |el| {
                        el.child(
                            theme::button(
                                "choose-backup",
                                "从备份新建测试副本",
                                theme::Tone::Secondary,
                                !state.busy,
                            )
                            .on_click(
                                cx.listener(|this, _, _, cx| this.choose_backup_versions(cx)),
                            ),
                        )
                    })
                    .child(
                        theme::button(
                            "open-edit-history",
                            "查看配置备份",
                            theme::Tone::Secondary,
                            true,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.open_edit_history(cx))),
                    ),
            )
            .when_some(state.choices.as_ref(), |el, choices| {
                let enabled = !state.busy && choices.iter().any(|v| v.selected().is_some());
                let label = if state.busy {
                    "正在处理…"
                } else if state.created.is_some() {
                    "打开这个测试副本"
                } else {
                    "校验并新建测试副本"
                };
                el.child(self.backup_version_row(0, "热键", &choices[0], cx))
                    .child(self.backup_version_row(1, "轮盘", &choices[1], cx))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                theme::button(
                                    "restore-backup",
                                    label,
                                    theme::Tone::Primary,
                                    enabled,
                                )
                                .when(enabled, |el| {
                                    el.on_click(
                                        cx.listener(|this, _, _, cx| this.restore_backup(cx)),
                                    )
                                }),
                            )
                            .child(
                                theme::button(
                                    "close-backup-picker",
                                    "收起",
                                    theme::Tone::Secondary,
                                    !state.busy,
                                )
                                .when(!state.busy, |el| {
                                    el.on_click(cx.listener(|this, _, _, cx| {
                                        this.backup_restore = Default::default();
                                        cx.notify();
                                    }))
                                }),
                            ),
                    )
            })
            .when_some(state.notice.clone(), |el, msg| el.child(caption(msg)))
            .when_some(self.backup_notice.clone(), |el, msg| el.child(caption(msg)))
    }
}
