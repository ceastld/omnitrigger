//! 调试入口：真实脚本宿主经应用UI显示合成通知，不安装Hook或读取个人配置。
use gpui::{
    div, App, AppContext, Application, Context, IntoElement, Render, Window, WindowOptions,
};
use omnitrigger_actions::{
    ActionCall, ActionError, ActionRuntime, CancellationToken, InvocationContext, NativeHost,
    NodeRuntime, PackageCatalog, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

type Probe = SyncSender<SyncSender<Value>>;

struct SmokeHost;
impl Render for SmokeHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct NoDesktop;
impl NativeHost for NoDesktop {
    fn call(&self, method: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        let mut error = ActionError::new("unexpected_native_call", "通知验收不允许其它本机调用。");
        error.method = Some(method.into());
        error.effect_state = Some("not_started".into());
        Err(error)
    }
}

pub fn maybe_run(arguments: &[String]) -> bool {
    if !arguments.iter().any(|arg| arg == "--notification-smoke") {
        return false;
    }
    let result = if arguments.len() == 3 && arguments[1] == "--notification-smoke" {
        run(Path::new(&arguments[2]))
    } else {
        Err("--notification-smoke 只接受一个空的本地输出目录。".into())
    };
    if let Err(message) = result {
        eprintln!("{message}");
        std::process::exit(1);
    }
    true
}

fn snapshot(probe: &Probe) -> Result<Value, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    probe.try_send(sender).map_err(|e| e.to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|e| e.to_string())
}

fn require(condition: bool, message: impl ToString) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

fn wait_idle(probe: &Probe, timeout: Duration) -> Result<Value, String> {
    let start = Instant::now();
    loop {
        let value = snapshot(probe)?;
        if value["busy"] == false {
            return Ok(value);
        }
        require(start.elapsed() < timeout, "通知未按时释放名额。")?;
        thread::sleep(Duration::from_millis(20));
    }
}

fn fixture(root: &Path) -> Result<PackageCatalog, String> {
    let package = root.join("actions/notification-probe");
    fs::create_dir_all(&package).map_err(|e| e.to_string())?;
    fs::write(package.join("action.json"), json!({"schema_version":1,"id":"notification-probe","name":"通知验收","version":"1.0.0","actions":[
        {"id":"node","title":"Node通知验收","version":1,"runtime":"node","entry":"main.mjs"},
        {"id":"script","title":"轻量通知验收","version":1,"runtime":"script","entry":"main.js"}
    ]}).to_string()).map_err(|e| e.to_string())?;
    fs::write(
        package.join("main.mjs"),
        "export default async ctx => await ctx.ui.notify(ctx.input);",
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        package.join("main.js"),
        "function main(ctx) { return ctx.ui.notify(ctx.input); }",
    )
    .map_err(|e| e.to_string())?;
    Ok(PackageCatalog::load(&root.join("actions")))
}

fn exercise(
    root: &Path,
    host_path: PathBuf,
    probe: &Probe,
    foreground_hits: &AtomicU64,
    foreground_samples: &AtomicU64,
) -> Result<Value, String> {
    let catalog = fixture(root)?;
    let host = crate::script_ui::Host::new(NoDesktop, None);
    let node = NodeRuntime::default();
    let script = ScriptHostRuntime::new(host_path);
    let invoke = |runtime: &str, input| {
        let action = catalog.resolve(&ActionCall {
            package_id: "notification-probe".into(),
            action_id: runtime.into(),
            action_version: 1,
            input,
        })?;
        let runtime: &dyn ActionRuntime = if runtime == "node" { &node } else { &script };
        runtime.execute_with_host(
            &action,
            &InvocationContext {
                invocation_id: 1,
                binding_id: 0,
            },
            &CancellationToken::new(),
            Some(&host),
        )
    };
    let mut cases = Vec::new();
    for (index, (runtime, placement, level, duration)) in [
        ("node", "topRight", "info", 30000),
        ("script", "bottomRight", "success", 1000),
        ("node", "center", "warning", 1000),
        ("script", "topRight", "error", 1000),
    ]
    .into_iter()
    .enumerate()
    {
        let before = wait_idle(probe, Duration::from_secs(2))?;
        let before_count = before["closedCount"]
            .as_u64()
            .ok_or("通知诊断缺少关闭计数。")?;
        let value = invoke(runtime, json!({"title":"OmniTrigger 通知验收","message":"本轮合成通知\n中文 🦀","level":level,"placement":placement,"durationMs":duration}))
            .map_err(|error| format!("{error:?}"))?;
        require(
            value == json!({"submitted":true}),
            format!("通知回执异常：{value}"),
        )?;
        let active = snapshot(probe)?;
        require(active["busy"] == true, "通知回执后没有有效显示名额。")?;
        let busy = invoke(runtime, json!({"message":"不应创建的重叠通知"}));
        require(
            matches!(&busy,Err(error) if error.code=="ui_busy" && error.effect_state.as_deref()==Some("not_started")),
            format!("重叠通知未明确拒绝：{busy:?}"),
        )?;
        if index == 0 {
            fs::write(
                root.join("ready.json"),
                json!({"pid":std::process::id(),"ui":active}).to_string(),
            )
            .map_err(|e| e.to_string())?;
        }
        let closed = wait_idle(probe, Duration::from_millis(duration + 2000))?;
        require(
            closed["closedCount"]
                .as_u64()
                .is_some_and(|count| count > before_count),
            "通知到期没有关闭记录。",
        )?;
        cases.push(json!({"runtime":runtime,"placement":placement,"level":level,"durationMs":duration,"reply":value,"shown":active,"expired":closed,"busy_rejected":true}));
    }
    let exit_reply = invoke(
        "node",
        json!({"message":"应用退出收尾验收","durationMs":30000}),
    )
    .map_err(|error| format!("{error:?}"))?;
    require(
        exit_reply == json!({"submitted":true}),
        "退出用例通知未接纳。",
    )?;
    let exit_active = snapshot(probe)?;
    require(exit_active["busy"] == true, "退出前没有活动通知。")?;
    cases.push(
        json!({"runtime":"node","lifecycle":"app_exit","reply":exit_reply,"shown":exit_active}),
    );
    require(
        foreground_hits.load(Ordering::Acquire) == 0,
        "通知窗口曾被采样为前台。",
    )?;
    require(
        foreground_samples.load(Ordering::Acquire) > 0,
        "没有有效的前台采样，验收证据不足。",
    )?;
    Ok(
        json!({"ok":true,"cases":cases,"notification_foreground_samples":0,"foreground_samples":foreground_samples.load(Ordering::Acquire),"clipboard_accessed":false,"personal_configuration_accessed":false,"scope":"synthetic_node_and_script_notifications_in_owned_app"}),
    )
}

fn run(directory: &Path) -> Result<(), String> {
    let root = std::path::absolute(directory).map_err(|e| e.to_string())?;
    crate::script_runtime::check_local_path(&root)?;
    require(
        root.is_dir()
            && fs::read_dir(&root)
                .map_err(|e| e.to_string())?
                .next()
                .is_none(),
        "验收输出目录必须存在且为空。",
    )?;
    let host_path = PathBuf::from(
        std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST")
            .ok_or("需要 OMNITRIGGER_TEST_SCRIPT_HOST。")?,
    );
    crate::script_runtime::check_local_path(&host_path)?;
    require(
        host_path.is_absolute() && host_path.is_file(),
        "需要完整的真实轻量宿主路径。",
    )?;
    let done = Arc::new(AtomicBool::new(false));
    let foreground_hits = Arc::new(AtomicU64::new(0));
    let foreground_samples = Arc::new(AtomicU64::new(0));
    let final_hits = foreground_hits.clone();
    let final_samples = foreground_samples.clone();
    let (sender, receiver) = mpsc::sync_channel::<SyncSender<Value>>(1);
    let result_path = root.join("report.json");
    let worker_done = done.clone();
    let worker_hits = foreground_hits.clone();
    let worker_samples = foreground_samples.clone();
    Application::new()
        .with_assets(crate::Assets)
        .run(move |cx: &mut App| {
            // GPUI 默认在最后一个窗口关闭后退出。用隐藏宿主模拟正常主界面，
            // 让多条通知依次到期；该窗口从不显示或激活。
            cx.open_window(
                WindowOptions {
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                |_, cx| cx.new(|_| SmokeHost),
            )
            .expect("创建隐藏验收宿主");
            crate::message_input::init(cx);
            crate::script_ui::init(cx);
            thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    exercise(&root, host_path, &sender, &worker_hits, &worker_samples)
                }));
                let report = match result {
                    Ok(Ok(value)) => value,
                    Ok(Err(error)) => json!({"ok":false,"error":error}),
                    Err(_) => json!({"ok":false,"error":"验收线程异常。"}),
                };
                let _ = fs::write(root.join("report.json"), report.to_string());
                worker_done.store(true, Ordering::Release);
            });
            cx.spawn(async move |cx| loop {
                cx.background_executor()
                    .timer(Duration::from_millis(10))
                    .await;
                if cx
                    .update(|cx| {
                        if let Ok(target) = omnitrigger_windows::capture_shortcut_target()
                            .and_then(omnitrigger_windows::ScriptTarget::capture)
                            .and_then(|target| target.snapshot())
                        {
                            foreground_samples.fetch_add(1, Ordering::Relaxed);
                            if target.process_id == std::process::id() {
                                foreground_hits.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        while let Ok(response) = receiver.try_recv() {
                            let _ = response.try_send(crate::script_notify::diagnostics());
                        }
                        if done.load(Ordering::Acquire) {
                            cx.quit();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            })
            .detach();
        });
    let mut result: Value =
        serde_json::from_slice(&fs::read(&result_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    require(result["ok"] == true, result.to_string())?;
    let after_exit = crate::script_notify::diagnostics();
    require(
        after_exit["busy"] == false && after_exit["windowId"].is_null(),
        "应用退出后通知状态未清理。",
    )?;
    require(
        after_exit["presentedCount"] == after_exit["closedCount"],
        "应用退出后仍有未关闭通知。",
    )?;
    require(
        final_hits.load(Ordering::Acquire) == 0,
        "通知窗口曾被采样为前台。",
    )?;
    result["after_app_exit"] = after_exit;
    result["foreground_samples"] = final_samples.load(Ordering::Acquire).into();
    fs::write(result_path, result.to_string()).map_err(|e| e.to_string())?;
    println!("Node 与轻量通知的显示、忙碌拒绝、到期关闭及前台采样验收通过。");
    Ok(())
}
