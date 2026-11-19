//! 设置页在当前档选择历史版本，新建隔离测试副本。
use crate::{caption, card, profiles, settings, theme, Workspace};
use gpui::{div, prelude::*, px, Context, Div, FontWeight};
use std::time::SystemTime;

#[derive(Default)]
pub(super) struct BackupRestore {
    choices: Option<[Versions; 2]>,
    busy: bool,
    alias: Option<String>,
    created: Option<profiles::RestoredCopy>,
    saved: Option<Vec<profiles::RestoredCopy>>,
    archive: Option<std::path::PathBuf>,
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
    fn export_config_archive(&mut self, cx: &mut Context<Self>) {
        if self.backup_restore.busy {
            return;
        }
        let config = settings::path();
        self.backup_restore.busy = true;
        self.backup_restore.archive = None;
        self.backup_restore.notice = Some("正在导出并校验配置备份…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { profiles::export_config_archive(&config) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.backup_restore;
                state.busy = false;
                state.notice = Some(match result {
                    Ok(archive) => {
                        state.archive = Some(archive.path);
                        format!(
                            "已导出并校验 {} 个配置档、{} 个文件。",
                            archive.profiles, archive.files
                        )
                    }
                    Err(error) => error,
                });
                cx.notify();
            });
        })
        .detach();
    }

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
        if let Some(copy) = self.backup_restore.created.clone() {
            self.open_restored_copy(copy, cx);
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
                .spawn(
                    async move { profiles::create_backup_copy(&config, &alias, bindings, wheels) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.backup_restore.busy = false;
                match result {
                    Ok(copy) => {
                        this.backup_restore.notice =
                            Some(format!("已创建测试副本 {}。", copy.alias));
                        this.backup_restore.created = Some(copy);
                    }
                    Err(error) => this.backup_restore.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn show_saved_backup_copies(&mut self, cx: &mut Context<Self>) {
        if self.backup_restore.busy {
            return;
        }
        self.backup_restore.notice = None;
        if self.backup_restore.saved.take().is_some() {
            cx.notify();
            return;
        }
        self.backup_restore.busy = true;
        let config = settings::path();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { profiles::saved_backup_copies(&config) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.backup_restore.busy = false;
                match result {
                    Ok(list) => {
                        this.backup_restore.saved = Some(list.items);
                        this.backup_restore.notice = list.notice;
                    }
                    Err(error) => this.backup_restore.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_restored_copy(&mut self, copy: profiles::RestoredCopy, cx: &mut Context<Self>) {
        if self.backup_restore.busy {
            return;
        }
        self.backup_restore.busy = true;
        self.backup_restore.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { copy.open() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.backup_restore.busy = false;
                this.backup_restore.notice = result.err();
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
            .when(standard, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            theme::button(
                                "export-config-archive",
                                "导出配置 ZIP",
                                theme::Tone::Primary,
                                !state.busy,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.export_config_archive(cx))),
                        )
                        .when_some(state.archive.clone(), |row, path| {
                            row.child(
                                theme::button(
                                    "reveal-config-archive",
                                    "在文件夹中显示",
                                    theme::Tone::Secondary,
                                    true,
                                )
                                .on_click(move |_, _, cx| cx.reveal_path(&path)),
                            )
                        }),
                )
            })
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
                        .child(
                            theme::button(
                                "list-restored-copies",
                                if state.saved.is_some() {
                                    "收起已有副本"
                                } else {
                                    "查看已有恢复副本"
                                },
                                theme::Tone::Secondary,
                                !state.busy,
                            )
                            .on_click(
                                cx.listener(|this, _, _, cx| this.show_saved_backup_copies(cx)),
                            ),
                        )
                    })
                    .child(
                        theme::button(
                            "open-edit-history",
                            "查看编辑历史",
                            theme::Tone::Secondary,
                            true,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.open_edit_history(cx))),
                    ),
            )
            .when_some(state.saved.as_ref(), |el, copies| {
                el.when(copies.is_empty(), |el| {
                    el.child(caption("本档还没有从备份创建的测试副本。"))
                })
                .children(copies.iter().enumerate().map(|(i, copy)| {
                    let title = if copy.name == copy.alias {
                        copy.name.clone()
                    } else {
                        format!("{}（{}）", copy.name, copy.alias)
                    };
                    let item = copy.clone();
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .child(title)
                                .child(caption(copy.summary())),
                        )
                        .child(
                            theme::button(
                                ("open-saved-backup", i),
                                "打开",
                                theme::Tone::Secondary,
                                !state.busy,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.open_restored_copy(item.clone(), cx),
                            )),
                        )
                }))
            })
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
