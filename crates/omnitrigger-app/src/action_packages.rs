//! 动作包在应用层的组合根：目录、运行时、界面与无 Hook 命令行入口。
use crate::{
    button, caption, card, icon_button, icon_tile, settings, theme, ActionSpec, Context, Div, Icon,
    Page, Tone, Workspace,
};
use gpui::{div, prelude::*, px, FontWeight};
use omnitrigger_actions::{
    ActionCall, ActionOutcome, ActionService, NodeRuntime, PackageCatalog, RuntimeRegistry,
};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub fn root(config: &Path) -> PathBuf {
    config.with_file_name("actions")
}

fn node_program() -> Option<PathBuf> {
    if let Some(program) = std::env::var_os("OMNITRIGGER_NODE") {
        let path = PathBuf::from(program);
        return (path.is_absolute() && path.is_file()).then_some(path);
    }
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join(name))
            .find(|path| path.is_file())
    })
}

pub fn service(config: &Path) -> ActionService {
    let (catalog, runtimes) = components(config);
    ActionService::new(catalog, runtimes)
}

/// 只有输入引擎所有者显式启用；普通CLI的service仍无store。
pub fn owner_service(config: &Path) -> ActionService {
    let (mut catalog, runtimes) = components(config);
    let store = std::path::absolute(config)
        .map_err(|_| ())
        .and_then(|path| {
            crate::action_state_store::FileActionStateStorage::new(
                path.with_file_name("action-state"),
            )
            .map_err(|_| ())
        })
        .and_then(|storage| {
            omnitrigger_actions::state::ActionStateStore::new(Some(Arc::new(storage)))
                .map_err(|_| ())
        });
    match store {
        Ok(store) => ActionService::new(catalog, runtimes).with_state_store(Arc::new(store)),
        Err(()) => {
            catalog
                .diagnostics
                .push("动作状态宿主不可用；状态API将明确返回state_unavailable。".into());
            ActionService::new(catalog, runtimes)
        }
    }
}

pub(super) fn components(config: &Path) -> (PackageCatalog, RuntimeRegistry) {
    let mut catalog = PackageCatalog::load(&root(config));
    let mut runtimes = RuntimeRegistry::new();
    let needs_script = catalog
        .actions
        .iter()
        .any(|action| action.definition.runtime == "script");
    match crate::script_runtime::locate(config) {
        Ok(Some(host)) => {
            if let Err(error) = runtimes.register("script", Arc::new(host.runtime())) {
                catalog.diagnostics.push(error.to_string());
            }
        }
        Ok(None) if needs_script => catalog.diagnostics.push(
            "轻量脚本宿主未安装；开发时可用本机包安装接口或 OMNITRIGGER_SCRIPT_HOST 指定完整路径。"
                .into(),
        ),
        Err(error) if needs_script => catalog.diagnostics.push(error),
        _ => {}
    }
    if let Some(program) = node_program() {
        let runtime = NodeRuntime::new(program).with_browser_connector(
            crate::browser_connector::runtime_root(config)
                .join(crate::browser_connector::CONNECTOR_VERSION),
        );
        if let Err(error) = runtimes.register("node", Arc::new(runtime)) {
            catalog.diagnostics.push(error.to_string());
        }
    } else if catalog
        .actions
        .iter()
        .any(|action| action.definition.runtime == "node")
    {
        catalog.diagnostics.push("未找到 Node.js；请安装 Node.js，或用 OMNITRIGGER_NODE 指定 node.exe 的完整路径后重启。".into());
    }
    (catalog, runtimes)
}

pub fn submit(
    service: &ActionService,
    id: u64,
    call: ActionCall,
    target: Option<omnitrigger_windows::ShortcutTarget>,
) -> Result<u64, omnitrigger_actions::ActionError> {
    let desktop = crate::desktop_host::DesktopHost::capture(target);
    let target = desktop.choice_target();
    service.submit_with_host(
        id,
        call,
        Some(Arc::new(with_clipboard_history(
            crate::script_ui::Host::new(
                crate::script_browser::ActionHost::new(settings::path(), desktop),
                target,
            ),
        ))),
    )
}

/// 只确定应用私有文件位置；构造不读写磁盘，首次显式历史调用才访问存储。
fn with_clipboard_history<H: omnitrigger_actions::NativeHost>(
    inner: H,
) -> crate::process_capture_host::WithProcessCapture<
    crate::clipboard_history_host::WithClipboardHistory<
        H,
        Arc<crate::clipboard_history_store::ClipboardHistoryStore>,
    >,
> {
    let store = std::path::absolute(settings::path())
        .ok()
        .and_then(|path| {
            crate::clipboard_history_store::ClipboardHistoryStore::new(
                path.with_file_name("clipboard-history.json"),
            )
            .ok()
        })
        .map(Arc::new);
    crate::process_capture_host::WithProcessCapture::new(
        crate::clipboard_history_host::WithClipboardHistory::new(inner, store),
    )
}

/// 外部目录原样保留共享模型的可选策略字段，不读取绑定或执行动作入口。
fn catalog_json(service: &ActionService) -> Result<String, omnitrigger_actions::ActionError> {
    let output = serde_json::to_string(&service.describe_catalog()).map_err(|error| {
        omnitrigger_actions::ActionError::new("invalid_action_request", error.to_string())
    })?;
    if output.len() > 4 * 1024 * 1024 {
        return Err(omnitrigger_actions::ActionError::new(
            "catalog_too_large",
            "动作包目录输出超过 4 MiB，未返回部分目录。",
        ));
    }
    Ok(output)
}

/// 校验与运行共用服务；读清单不会导入 JS，也不安装 Hook。
pub fn cli(arguments: &[String]) -> Option<Result<String, String>> {
    let install = arguments
        .iter()
        .position(|arg| arg == "--install-script-runtime");
    let check = arguments.iter().position(|arg| arg == "--check-action");
    let run = arguments.iter().position(|arg| arg == "--run-action");
    let create = arguments
        .iter()
        .position(|arg| arg == "--create-script-action");
    let list = arguments
        .iter()
        .position(|arg| arg == "--list-package-actions");
    let stream = arguments.iter().any(|arg| arg == "--stream-action-events");
    let explicit_target = arguments.iter().position(|arg| arg == "--target-context");
    let index = check
        .or(run)
        .or(create)
        .or(install)
        .or(list)
        .or(explicit_target)
        .or_else(|| {
            arguments
                .iter()
                .position(|arg| arg == "--stream-action-events")
        })?;
    Some((|| {
        crate::action_target_context::validate_cli(arguments).map_err(|message| {
            serde_json::json!({
            "status":"rejected","error":{"code":"invalid_cli_arguments", "message":message,
                "effect_state":"not_started"}})
            .to_string()
        })?;
        if let Some(index) = install {
            let hash = arguments
                .iter()
                .position(|arg| arg == "--sha256")
                .expect("validated");
            return crate::script_runtime::install_archive(
                &settings::path(), Path::new(&arguments[index + 1]), &arguments[hash + 1],
            ).map(|value| value.to_string()).map_err(|message| {
                serde_json::json!({"status":"failed","error":{"code":"runtime_install_failed","message":message}}).to_string()
            });
        }
        if let Some(index) = create {
            let project = omnitrigger_actions::project::create_script_project(
                &root(&settings::path()),
                &arguments[index + 1],
            )
            .map_err(|mut error| {
                error
                    .effect_state
                    .get_or_insert_with(|| "not_started".into());
                serde_json::json!({"status":"failed","error":error}).to_string()
            })?;
            return Ok(
                serde_json::json!({"schema_version":1,"status":"created","project":project})
                    .to_string(),
            );
        }
        let rejected = |mut error: omnitrigger_actions::ActionError| {
            error.effect_state = Some("not_started".into());
            serde_json::json!({"status":"rejected","error":error}).to_string()
        };
        let request_error = |message| {
            rejected(omnitrigger_actions::ActionError::new(
                "invalid_action_request",
                message,
            ))
        };
        if list.is_some() {
            return catalog_json(&service(&settings::path())).map_err(rejected);
        }
        let target_error = |message: String| {
            serde_json::json!({"status":"rejected","error":{
                "code":"invalid_target_context","message":message,"effect_state":"not_started"
            }})
            .to_string()
        };
        let target_path = crate::action_target_context::path(arguments).map_err(target_error)?;
        let path = arguments
            .get(index + 1)
            .filter(|path| !path.starts_with("--"))
            .ok_or_else(|| request_error("动作命令后需要 ActionCall JSON 文件路径。".into()))?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|error| request_error(format!("读取动作调用失败：{error}")))?
            .take(128 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| request_error(error.to_string()))?;
        if bytes.len() > 128 * 1024 {
            return Err(request_error("动作调用文件不能超过 128 KiB。".into()));
        }
        let call: ActionCall = serde_json::from_slice(&bytes)
            .map_err(|error| request_error(format!("动作调用格式有误：{error}")))?;
        if run.is_some() {
            crate::legacy_local_source::verify_cli_action(&settings::path(), &call)
                .map_err(rejected)?;
        }
        let explicit_host = target_path
            .map(|path| {
                let snapshot = crate::action_target_context::read(path)?;
                crate::desktop_host::DesktopHost::from_snapshot(snapshot)
                    .map_err(|error| error.to_string())
            })
            .transpose()
            .map_err(target_error)?;
        let service = service(&settings::path());
        service.validate(&call).map_err(rejected)?;
        if check.is_some() {
            return Ok(serde_json::json!({"valid":true,"call":call}).to_string());
        }
        let invocation_id = match explicit_host {
            Some(host) => {
                let target = host.choice_target();
                service.submit_with_host(
                    0,
                    call,
                    Some(Arc::new(with_clipboard_history(
                        crate::script_ui::Host::new(
                            crate::script_browser::ActionHost::new(settings::path(), host),
                            target,
                        ),
                    ))),
                )
            }
            None => submit(&service, 0, call, None),
        }
        .map_err(rejected)?;
        loop {
            match service.try_recv() {
                Ok(event) => match event.outcome {
                    ActionOutcome::Started => {
                        if stream {
                            println!("{}", serde_json::json!({"schema_version":1,"invocation_id":invocation_id,"status":"started"}));
                            std::io::stdout().flush().map_err(|error| serde_json::json!({"status":"failed","error":{"code":"event_stream_closed","message":error.to_string(),"effect_state":"unknown"}}).to_string())?;
                        }
                    },
                    ActionOutcome::Succeeded { output } => return Ok(serde_json::json!({"invocation_id":invocation_id,"status":"succeeded","output":output}).to_string()),
                    ActionOutcome::Failed { error } => return Err(serde_json::json!({"invocation_id":invocation_id,"status":"failed","error":error}).to_string()),
                    ActionOutcome::Cancelled => return Err(serde_json::json!({"invocation_id":invocation_id,"status":"cancelled","error":{"code":"cancelled","message":"动作已取消。","effect_state":"unknown"}}).to_string()),
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(20)),
                Err(error) => return Err(serde_json::json!({"invocation_id":invocation_id,"status":"failed","error":{"code":"action_channel_closed","message":format!("动作服务连接中断：{error}"),"effect_state":"unknown"}}).to_string()),
            }
        }
    })())
}

impl Workspace {
    /// 动作库中的动作包卡片；没有匹配项时返回 `None`，由调用方省略整个分组。
    pub(super) fn package_catalog(&self, query: &str, cx: &mut Context<Self>) -> Option<Div> {
        let mut list = div().flex().flex_col().gap(px(10.));
        let mut any = false;
        for (index, entry) in self.action_service.catalog().actions.iter().enumerate() {
            let title = format!("{} / {}", entry.package_name, entry.definition.title);
            if !query.is_empty()
                && !format!("{title} {} {}", entry.package_id, entry.definition.id)
                    .to_lowercase()
                    .contains(query)
            {
                continue;
            }
            any = true;
            let call = ActionCall {
                package_id: entry.package_id.clone(),
                action_id: entry.definition.id.clone(),
                action_version: entry.definition.version,
                input: serde_json::json!({}),
            };
            let reason = self.legacy_unavailable_reason(&call).or_else(|| {
                self.action_service
                    .validate(&call)
                    .err()
                    .map(|error| error.to_string())
            });
            let name = title.clone();
            let edit_call = call.clone();
            let is_script = entry.definition.runtime == "script";
            list = list.child(
                card()
                    .p(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(icon_tile(Icon::Layers, theme::ACCENT_2, 40.))
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
                                    .child(title),
                            )
                            .child(caption(format!(
                                "v{} · {}",
                                call.action_version, entry.definition.runtime
                            )))
                            .when_some(
                                self.package_feedback.action_hint_for_call(&call),
                                |element, text| element.child(action_version_hint(text)),
                            )
                            .when_some(reason, |element, reason| element.child(caption(reason))),
                    )
                    .when(is_script, |element| {
                        element.child(
                            icon_button(
                                ("edit-script", index as u64),
                                Icon::Terminal,
                                "编辑代码",
                                Tone::Secondary,
                                !self.recording,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.edit_script_project(edit_call.clone(), cx)
                                },
                            )),
                        )
                    })
                    .child(
                        icon_button(
                            ("bind-package", index as u64),
                            Icon::Bolt,
                            "设置触发",
                            Tone::Primary,
                            !self.recording,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_package(call.clone(), &name, cx)
                        })),
                    ),
            );
        }
        any.then_some(list)
    }

    fn choose_package(&mut self, call: ActionCall, title: &str, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        if self.pending.is_none() && !(self.package_picker_for_draft && self.selected.is_some()) {
            let Some(id) = self.draft_id(None) else {
                cx.notify();
                return;
            };
            let spec = omnitrigger_core::TriggerSpec {
                pointer_scope: Default::default(),
                wheel: Default::default(),
                keyboard: Default::default(),
                process_scope: Default::default(),
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
        self.load_action(ActionSpec::Invoke(call), cx);
        self.package_picker_for_draft = false;
        self.message
            .update(cx, |input, cx| input.set_text(title.to_string(), cx));
        self.page = Page::Triggers;
        self.status = "动作包草稿已就绪；编辑参数和触发组合，保存后生效。".into();
        self.error = None;
        self.confirm_delete = None;
        cx.notify();
    }

    pub(super) fn package_action_fields(&self, cx: &mut Context<Self>) -> Div {
        let id = self
            .pending
            .as_ref()
            .map(|spec| spec.id)
            .or(self.selected)
            .unwrap_or(0);
        let title = if let ActionSpec::Invoke(call) = &self.draft_action {
            self.action_service
                .catalog()
                .actions
                .iter()
                .find(|entry| {
                    entry.package_id == call.package_id && entry.definition.id == call.action_id
                })
                .map(|entry| format!("{} / {}", entry.package_name, entry.definition.title))
                .unwrap_or_else(|| "动作包尚未加载".into())
        } else {
            crate::action_detail(&self.draft_action)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(caption(title))
            .child(Self::input_field(
                "动作参数（JSON 对象）",
                &self.package_input,
                Some("参数原样传入 ctx.input；代码在动作包的入口文件中编辑。"),
            ))
            .when_some(self.draft_action_spec(cx).err(), |element, error| {
                element.child(caption(error))
            })
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        button(
                            "test-package",
                            "运行当前草稿",
                            Tone::Secondary,
                            !self.recording && !self.action_service.is_running(id),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.test_action(cx))),
                    )
                    .child(
                        button(
                            "cancel-package",
                            "停止此动作",
                            Tone::Danger,
                            self.action_service.is_running(id),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.action_service.cancel_id(id);
                            cx.notify();
                        })),
                    ),
            )
            .child(caption(
                "运行会真实执行脚本。停止不撤销已完成操作；请只使用可信动作包。",
            ))
    }

    pub(super) fn package_result(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.recording {
            return None;
        }
        let id = self
            .pending
            .as_ref()
            .map(|spec| spec.id)
            .or(self.selected)?;
        let ActionSpec::Invoke(call) = self.draft_action_spec(cx).ok()? else {
            return None;
        };
        let feedback = self.package_feedback.for_call(id, &call)?;
        Some(match feedback {
            crate::package_feedback::Feedback::Running(invocation) => div()
                .child(caption(format!("运行 #{invocation} · 正在处理")))
                .when_some(
                    self.package_feedback.action_hint(invocation),
                    |element, text| element.child(action_version_hint(text)),
                ),
            crate::package_feedback::Feedback::Completed(preview) => {
                let invocation = preview.invocation;
                let expanded = self.package_feedback.details_visible(invocation);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .when_some(
                        self.package_feedback.action_hint(invocation),
                        |element, text| element.child(action_version_hint(text)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().flex_1().child(caption(format!(
                                "上次运行 #{} · {}",
                                invocation, preview.label
                            ))))
                            .child(
                                crate::small(button(
                                    "toggle-package-result",
                                    if expanded {
                                        "收起结果"
                                    } else {
                                        "查看结果"
                                    },
                                    Tone::Ghost,
                                    true,
                                ))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.package_feedback.toggle_details(invocation);
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .when(expanded, |element| {
                        element.child(
                            div()
                                .id("package-result")
                                .max_h(px(160.))
                                .min_h_0()
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .p(px(10.))
                                .bg(crate::rgb(theme::ROW))
                                .rounded(px(6.))
                                .font_family(theme::MONO_FONT)
                                .text_size(px(12.))
                                .children(
                                    preview
                                        .text
                                        .lines()
                                        .map(|line| div().flex_none().child(line.to_string())),
                                )
                                .map(|element| self.smooth_scroll.wrap("package-result", element)),
                        )
                    })
                    .when(expanded && preview.truncated, |element| {
                        element.child(caption("结果过长，预览已截断。"))
                    })
            }
        })
    }

    pub(super) fn poll_packages(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..8 {
            let Ok(event) = self.action_service.try_recv() else {
                break;
            };
            changed = true;
            self.package_feedback.apply(&event);
            let (message, failed) = match event.outcome {
                ActionOutcome::Started => ("已开始".to_string(), false),
                ActionOutcome::Succeeded { output: _ } => ("已完成，动作已返回结果".into(), false),
                ActionOutcome::Failed { error } => (error.to_string(), true),
                ActionOutcome::Cancelled => ("已停止；已完成的操作不会撤销".into(), false),
            };
            let message = format!("动作包运行 {} · {message}", event.invocation_id);
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
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_actions::{
        ActionError, ActionRuntime, CancellationToken, InvocationContext, ResolvedAction,
    };
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct NeverExecute;
    impl ActionRuntime for NeverExecute {
        fn execute(
            &self,
            _: &ResolvedAction,
            _: &InvocationContext,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            panic!("动作目录和引用校验不能执行入口")
        }
    }

    struct Fixture {
        directory: PathBuf,
        config: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let directory = loop {
                let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                    ".single-instance-app-test-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&directory) {
                    Ok(()) => break directory,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("无法创建本测试临时目录：{error}"),
                }
            };
            let fixture = Self {
                config: directory.join("bindings.json"),
                directory,
            };
            std::fs::create_dir(root(&fixture.config)).unwrap();
            fixture
        }

        fn package(&self, policy: Option<Value>) -> ActionCall {
            let directory = root(&self.config).join("app-policy-test");
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(
                directory.join("main.js"),
                b"throw new Error('must not execute');",
            )
            .unwrap();
            let mut manifest = json!({"schema_version":1,"id":"app-policy-test","name":"目录策略测试","version":"1.0.0","actions":[{"id":"run","title":"只读校验","version":1,"runtime":"app-test","entry":"main.js"}]});
            if let Some(policy) = policy {
                manifest["actions"][0]
                    .as_object_mut()
                    .unwrap()
                    .insert("single_instance".into(), policy);
            }
            std::fs::write(
                directory.join("action.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            ActionCall {
                package_id: "app-policy-test".into(),
                action_id: "run".into(),
                action_version: 1,
                input: json!({}),
            }
        }

        fn service(&self, runtime: &str) -> ActionService {
            let catalog = PackageCatalog::load(&root(&self.config));
            let mut runtimes = RuntimeRegistry::new();
            runtimes.register(runtime, Arc::new(NeverExecute)).unwrap();
            ActionService::new(catalog, runtimes)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // 仅清理本测试已知文件与空目录，不递归清理或跟随链接。
            let actions = root(&self.config);
            for file in [
                "app-policy-test/main.js",
                "app-policy-test/action.json",
                "generated-script/action.json",
                "generated-script/main.mjs",
                "generated-script/lib/text.js",
                "generated-script/call.json",
                "generated-script/jsconfig.json",
                "generated-script/README.md",
                "generated-script/sdk/api.d.ts",
            ] {
                let _ = std::fs::remove_file(actions.join(file));
            }
            for directory in [
                "app-policy-test",
                "generated-script/lib",
                "generated-script/sdk",
                "generated-script",
            ] {
                let _ = std::fs::remove_dir(actions.join(directory));
            }
            let _ = std::fs::remove_dir(actions);
            let _ = std::fs::remove_dir(&self.directory);
        }
    }

    #[test]
    fn action_catalog_and_validation_preserve_optional_single_instance_without_execution() {
        for policy in [None, Some(json!(false)), Some(json!(true))] {
            let expected = policy == Some(json!(true));
            let fixture = Fixture::new();
            let call = fixture.package(policy);
            let service = fixture.service("app-test");
            assert!(service.catalog().diagnostics.is_empty());
            assert_eq!(
                service
                    .catalog()
                    .resolve(&call)
                    .unwrap()
                    .definition
                    .single_instance,
                expected
            );
            service.validate(&call).unwrap();
            let output: Value = serde_json::from_str(&catalog_json(&service).unwrap()).unwrap();
            assert_eq!(output["schema_version"], 1);
            assert_eq!(output["actions"].as_array().unwrap().len(), 1);
            let action = &output["actions"][0];
            assert_eq!(action["runtime"], "app-test");
            assert!(action["reference_error"].is_null());
            if expected {
                assert_eq!(action["single_instance"], true);
            } else {
                assert!(action.get("single_instance").is_none());
            }
            let serialized_call = serde_json::to_value(&call).unwrap();
            assert_eq!(action["call"], serialized_call);
            assert_eq!(serialized_call.as_object().unwrap().len(), 4);
            assert!(serialized_call.get("single_instance").is_none());
        }
    }

    #[test]
    fn invalid_single_instance_types_do_not_enter_the_app_catalog_or_validate() {
        for policy in [
            json!(null),
            json!("true"),
            json!("false"),
            json!(0),
            json!(1),
            json!([]),
            json!({}),
        ] {
            let fixture = Fixture::new();
            let call = fixture.package(Some(policy));
            let service = fixture.service("app-test");
            assert!(service.catalog().actions.is_empty());
            assert!(!service.catalog().diagnostics.is_empty());
            assert!(service.validate(&call).is_err());
            let output: Value = serde_json::from_str(&catalog_json(&service).unwrap()).unwrap();
            assert!(output["actions"].as_array().unwrap().is_empty());
            assert!(!output["diagnostics"].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn generated_script_keeps_legacy_default_and_accepts_author_enabled_single_instance() {
        let fixture = Fixture::new();
        fixture.package(Some(json!(true)));
        let actions = root(&fixture.config);
        let project =
            omnitrigger_actions::project::create_script_project(&actions, "generated-script")
                .unwrap();
        let manifest_path = project.package_root.join("action.json");
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["schema_version"], 1);
        assert!(manifest["actions"][0].get("single_instance").is_none());
        let service = fixture.service("script");
        assert!(
            !service
                .catalog()
                .resolve(&project.call)
                .unwrap()
                .definition
                .single_instance
        );
        service.validate(&project.call).unwrap();
        manifest["actions"][0]["single_instance"] = json!(true);
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let service = fixture.service("script");
        assert!(
            service
                .catalog()
                .resolve(&project.call)
                .unwrap()
                .definition
                .single_instance
        );
        service.validate(&project.call).unwrap();
        let (_, entry) =
            omnitrigger_actions::project::editable_script(&actions, &project.call).unwrap();
        assert_eq!(entry.file_name().unwrap(), "main.mjs");
        assert!(serde_json::to_value(project.call)
            .unwrap()
            .get("single_instance")
            .is_none());
    }
}

fn action_version_hint(text: &str) -> gpui::Div {
    gpui::div()
        .text_size(gpui::px(11.))
        .text_color(crate::rgb(0x008000))
        .child(text.to_owned())
}
