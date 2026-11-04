//! 动作包在应用层的组合根：目录、运行时、界面与无 Hook 命令行入口。
use crate::{
    button, caption, card, icon_button, icon_tile, settings, theme, ActionSpec, Context, Div, Icon,
    Page, Tone, Workspace,
};
use gpui::{div, prelude::*, px, FontWeight};
use omnitrigger_actions::{
    ActionCall, ActionOutcome, ActionService, NodeRuntime, PackageCatalog, RuntimeRegistry,
    ScriptHostRuntime,
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
    let mut catalog = PackageCatalog::load(&root(config));
    let mut runtimes = RuntimeRegistry::new();
    let host = std::env::var_os("OMNITRIGGER_SCRIPT_HOST")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::path::absolute(config)
                .unwrap_or_else(|_| config.to_path_buf())
                .with_file_name("runtimes")
                .join("script")
                .join(if cfg!(windows) { "host.exe" } else { "host" })
        });
    if host.is_absolute() && host.is_file() {
        if let Err(error) = runtimes.register("script", Arc::new(ScriptHostRuntime::new(host))) {
            catalog.diagnostics.push(error.to_string());
        }
    } else if catalog
        .actions
        .iter()
        .any(|action| action.definition.runtime == "script")
    {
        catalog.diagnostics.push(
            "轻量脚本宿主未安装；开发时可用 OMNITRIGGER_SCRIPT_HOST 指定本机构建的完整路径。"
                .into(),
        );
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
    ActionService::new(catalog, runtimes)
}

pub fn submit(
    service: &ActionService,
    id: u64,
    call: ActionCall,
    target: Option<omnitrigger_windows::ShortcutTarget>,
) -> Result<u64, omnitrigger_actions::ActionError> {
    service.submit_with_host(
        id,
        call,
        Some(Arc::new(crate::desktop_host::DesktopHost::capture(target))),
    )
}

/// 校验与运行共用服务；读清单不会导入 JS，也不安装 Hook。
pub fn cli(arguments: &[String]) -> Option<Result<String, String>> {
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
            let snapshot = service(&settings::path()).describe_catalog();
            let output = serde_json::to_string(&snapshot)
                .map_err(|error| request_error(error.to_string()))?;
            if output.len() > 4 * 1024 * 1024 {
                return Err(rejected(omnitrigger_actions::ActionError::new(
                    "catalog_too_large",
                    "动作包目录输出超过 4 MiB，未返回部分目录。",
                )));
            }
            return Ok(output);
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
            Some(host) => service.submit_with_host(0, call, Some(Arc::new(host))),
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
            let reason = self
                .action_service
                .validate(&call)
                .err()
                .map(|error| error.to_string());
            let name = title.clone();
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
                            .when_some(reason, |element, reason| element.child(caption(reason))),
                    )
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
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(caption(crate::action_detail(&self.draft_action)))
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

    pub(super) fn poll_packages(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..8 {
            let Ok(event) = self.action_service.try_recv() else {
                break;
            };
            changed = true;
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
