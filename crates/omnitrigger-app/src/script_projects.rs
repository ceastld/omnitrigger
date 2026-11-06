//! 脚本工程的用户入口；编辑器只接收文件参数，不通过脚本文件关联启动。
use crate::{
    button, caption, card, icon_button, Context, Div, Icon, MessageInput, Tone, Workspace,
};
use gpui::{div, prelude::*, px, Entity};
use omnitrigger_actions::ActionCall;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
};

pub(super) struct ScriptProjects {
    name: Entity<MessageInput>,
    creating: bool,
    editor: Option<Receiver<Result<String, String>>>,
}
impl ScriptProjects {
    pub fn new(cx: &mut Context<Workspace>) -> Self {
        let name = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(80);
            input.set_placeholder("例如：整理选中文字");
            input
        });
        cx.observe(&name, |_, _, cx| cx.notify()).detach();
        Self {
            name,
            creating: false,
            editor: None,
        }
    }
}

impl Workspace {
    pub(super) fn script_tools(&self, cx: &mut Context<Self>) -> Div {
        let creating = self.script_projects.creating;
        card()
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().child("JavaScript 脚本"))
                    .child(
                        icon_button(
                            "reload-script-projects",
                            Icon::Refresh,
                            "重新加载",
                            Tone::Secondary,
                            !self.recording,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.reload_script_projects(cx))),
                    )
                    .child(
                        button(
                            "new-script-project",
                            "新建脚本",
                            Tone::Primary,
                            !self.recording && !creating,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.script_projects.creating = true;
                            this.error = None;
                            cx.notify();
                        })),
                    ),
            )
            .when(creating, |element| {
                element
                    .child(Self::input_field(
                        "脚本名称",
                        &self.script_projects.name,
                        None,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                button(
                                    "create-script-project",
                                    "创建",
                                    Tone::Primary,
                                    !self.recording
                                        && !self
                                            .script_projects
                                            .name
                                            .read(cx)
                                            .text()
                                            .trim()
                                            .is_empty(),
                                )
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.create_script_project(cx)),
                                ),
                            )
                            .child(
                                button(
                                    "cancel-create-script-project",
                                    "取消",
                                    Tone::Secondary,
                                    true,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.script_projects.creating = false;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
            })
            .child(caption(
                "创建后可编辑代码并设置触发；保存代码后，下次运行即使用新内容。",
            ))
    }

    pub(super) fn reload_script_projects(&mut self, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        self.refresh_script_catalog();
        self.error = None;
        cx.notify();
    }

    pub(super) fn refresh_script_catalog(&mut self) {
        let (catalog, runtimes) = crate::action_packages::components(&crate::settings::path());
        let count = catalog.actions.len();
        let diagnostics = catalog.diagnostics.len();
        self.action_service.reload(catalog, runtimes);
        self.status = if diagnostics == 0 {
            format!("已重新加载 {count} 个动作。")
        } else {
            format!(
                "已重新加载 {count} 个动作，{diagnostics} 项需要处理；详细诊断可从开发命令读取。"
            )
        };
    }

    fn create_script_project(&mut self, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let name = self.script_projects.name.read(cx).text();
        match omnitrigger_actions::project::create_named_script_project(
            &crate::action_packages::root(&crate::settings::path()),
            &name,
        ) {
            Ok(_) => {
                self.refresh_script_catalog();
                self.script_projects.creating = false;
                self.script_projects
                    .name
                    .update(cx, |input, cx| input.set_text("", cx));
                self.action_search
                    .update(cx, |input, cx| input.set_text(name.trim().to_string(), cx));
                self.status = format!("已创建“{}”；可编辑代码，设置触发后生效。", name.trim());
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(super) fn edit_script_project(&mut self, call: ActionCall, cx: &mut Context<Self>) {
        if self.recording || self.script_projects.editor.is_some() {
            return;
        }
        let root = crate::action_packages::root(&crate::settings::path());
        let wake = self.ui_wake.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("script-editor".into())
            .spawn(move || {
                let result = open_editor(&root, &call);
                let _ = sender.send(result);
                wake.notify();
            }) {
            Ok(_) => {
                self.script_projects.editor = Some(receiver);
                self.status = "正在启动代码编辑器…".into();
                self.error = None;
            }
            Err(error) => self.error = Some(format!("无法启动编辑器工作线程：{error}")),
        }
        cx.notify();
    }

    pub(super) fn poll_script_editor(&mut self) -> bool {
        let Some(receiver) = &self.script_projects.editor else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("编辑器启动任务中断。".into()),
        };
        self.script_projects.editor = None;
        match result {
            Ok(editor) => {
                self.status =
                    format!("已启动{editor}，保存代码后可运行；修改动作信息后点击“重新加载”。")
            }
            Err(error) => self.error = Some(error),
        }
        true
    }
}

struct EditorPlan {
    program: PathBuf,
    arguments: Vec<OsString>,
    label: &'static str,
}

fn editor_plan(
    root: &Path,
    entry: &Path,
    configured: Option<PathBuf>,
    candidates: Vec<PathBuf>,
    notepad: PathBuf,
) -> Result<EditorPlan, String> {
    let checked = |path: &Path| -> Result<bool, String> {
        if !path.is_absolute() {
            return Err("编辑器需要完整本地路径。".into());
        }
        crate::script_runtime::check_local_path(path)?;
        Ok(path.is_file())
    };
    if let Some(program) = configured {
        if !program.is_absolute()
            || !program
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            || !checked(&program)?
        {
            return Err("OMNITRIGGER_EDITOR 需要存在的完整本地编辑器 EXE 路径。".into());
        }
        return Ok(EditorPlan {
            arguments: project_editor_arguments(&program, root, entry)
                .unwrap_or_else(|| vec![entry.as_os_str().into()]),
            program,
            label: "代码编辑器",
        });
    }
    for program in candidates {
        if checked(&program).unwrap_or(false) {
            return Ok(EditorPlan {
                program,
                arguments: vec![
                    "--new-window".into(),
                    root.as_os_str().into(),
                    entry.as_os_str().into(),
                ],
                label: " VS Code",
            });
        }
    }
    if !notepad.is_absolute() || !checked(&notepad)? {
        return Err("未找到可用的代码编辑器。".into());
    }
    Ok(EditorPlan {
        program: notepad,
        arguments: vec![entry.as_os_str().into()],
        label: "记事本",
    })
}

fn project_editor_arguments(program: &Path, root: &Path, entry: &Path) -> Option<Vec<OsString>> {
    let name = program.file_stem()?.to_str()?.to_ascii_lowercase();
    // 仅为具有已知文件夹参数协议的编辑器传递工程目录，不把目录误当成普通编辑器的文件。
    matches!(
        name.as_str(),
        "code" | "code - insiders" | "cursor" | "codium" | "windsurf"
    )
    .then(|| {
        vec![
            "--new-window".into(),
            root.as_os_str().into(),
            entry.as_os_str().into(),
        ]
    })
}

fn open_editor(root: &Path, call: &ActionCall) -> Result<String, String> {
    let (root, entry) =
        omnitrigger_actions::project::editable_script(root, call).map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Programs/Microsoft VS Code/Code.exe"));
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(folder) = std::env::var_os(variable) {
            candidates.push(PathBuf::from(folder).join("Microsoft VS Code/Code.exe"));
        }
    }
    let windows = std::env::var_os("SystemRoot").ok_or("无法定位系统编辑器。")?;
    let plan = editor_plan(
        &root,
        &entry,
        std::env::var_os("OMNITRIGGER_EDITOR").map(PathBuf::from),
        candidates,
        PathBuf::from(windows).join("System32/notepad.exe"),
    )?;
    let mut command = Command::new(&plan.program);
    command.args(&plan.arguments).current_dir(&root);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .spawn()
        .map_err(|e| format!("无法启动{}：{e}", plan.label))?;
    Ok(plan.label.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_arguments_are_literal_paths_and_never_file_associations() {
        let root = std::env::temp_dir().join(format!("omni-editor-plan-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let program = root.join("Code.exe");
        std::fs::write(&program, b"not executed").unwrap();
        let folder = root.join("中文 & folder");
        let entry = folder.join("main.js");
        let plan = editor_plan(
            &folder,
            &entry,
            None,
            vec![program.clone()],
            root.join("missing.exe"),
        )
        .unwrap();
        assert_eq!(plan.program, program);
        assert_eq!(
            plan.arguments,
            vec![
                OsString::from("--new-window"),
                folder.as_os_str().into(),
                entry.as_os_str().into()
            ]
        );
        let explicit = editor_plan(
            &folder,
            &entry,
            Some(program.clone()),
            vec![],
            root.join("missing.exe"),
        )
        .unwrap();
        assert_eq!(explicit.arguments, plan.arguments);
        let plain = editor_plan(&folder, &entry, None, vec![], program.clone()).unwrap();
        assert_eq!(plain.arguments, vec![entry.as_os_str().to_os_string()]);
        assert_eq!(
            project_editor_arguments(&root.join("Cursor.exe"), &folder, &entry),
            Some(plan.arguments.clone())
        );
        assert!(project_editor_arguments(&root.join("Other.exe"), &folder, &entry).is_none());
        let skipped = editor_plan(
            &folder,
            &entry,
            None,
            vec![root.join("OneDrive-placeholder/Code.exe")],
            program.clone(),
        )
        .unwrap();
        assert_eq!(skipped.arguments, plain.arguments);
        assert!(editor_plan(
            &folder,
            &entry,
            Some(root.join("missing.exe")),
            vec![program.clone()],
            program.clone()
        )
        .is_err());
        assert!(editor_plan(
            &folder,
            &entry,
            Some(entry.clone()),
            vec![program.clone()],
            program.clone()
        )
        .is_err());
        std::fs::remove_file(program).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
