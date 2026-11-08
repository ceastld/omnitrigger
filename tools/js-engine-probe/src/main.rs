use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, Read};

mod api;
mod modules;
mod native;
mod runtime_description;
const DESKTOP_API: &str =
    include_str!("../../../crates/omnitrigger-actions/runtime/desktop-api.js");
const STATE_API: &str = include_str!("../../../crates/omnitrigger-actions/runtime/state-api.js");
const ACTIONS_API: &str =
    include_str!("../../../crates/omnitrigger-actions/runtime/actions-api.js");
const SDK: &str = include_str!("sdk.js");
const TEXT_ENCODER: &str = include_str!("text-encoder.js");
const PROCESS_CAPTURE_API: &str =
    include_str!("../../../crates/omnitrigger-actions/runtime/process-capture-api.js");
const HISTORY_API: &str =
    include_str!("../../../crates/omnitrigger-actions/runtime/clipboard-history-api.js");
const BROWSER_API: &str =
    include_str!("../../../crates/omnitrigger-actions/runtime/browser-api.js");

#[cfg(any(
    all(feature = "boa", feature = "quickjs"),
    not(any(feature = "boa", feature = "quickjs"))
))]
compile_error!("请选择且只选择 boa 或 quickjs 引擎。");

const MAX_REQUEST: u64 = 256 * 1024;
const FAULT_FIELDS: &str = "(function(error) { try {\n\
    if (typeof error?.code !== 'string' || !['not_started','unknown'].includes(error.effectState)) return null;\n\
    return JSON.stringify({code:error.code,message:String(error.message),effect_state:error.effectState,method:error.method,required_permissions:error.requiredPermissions});\n\
    } catch (_) { return null; } })";

fn bounded_diagnostic(mut text: String) -> String {
    const TAIL: &str = "\n…（诊断已截断）";
    let mut limit = 16 * 1024;
    loop {
        if text.len() > limit {
            let separator = "\n…\n";
            let mut end = (limit - TAIL.len() - separator.len()) / 2;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let mut start = text.len() - (limit - TAIL.len() - separator.len() - end);
            while !text.is_char_boundary(start) {
                start += 1;
            }
            // 长消息保留首尾，避免把位于消息后面的源代码调用栈一并裁掉。
            text = format!("{}{separator}{}{TAIL}", &text[..end], &text[start..]);
        }
        // 控制字符在 JSON 中最多膨胀六倍，给响应信封与原生错误字段留出空间。
        if serde_json::to_string(&text).is_ok_and(|value| value.len() <= 32 * 1024) {
            return text;
        }
        limit /= 2;
    }
}

fn diagnostic_error(fields: Option<String>, rendered: String) -> Value {
    if let Some(mut fault) = fields
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(Value::is_object)
    {
        let message = fault["message"].as_str().unwrap_or("JavaScript 异常");
        fault["message"] = bounded_diagnostic(format!("{message}\nJavaScript：{rendered}")).into();
        fault
    } else {
        bounded_diagnostic(rendered).into()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    /// 函数体；通过 ctx.input 读取参数，返回 JSON 或最终解析为 JSON 的 Promise。
    #[serde(default)]
    script: Option<String>,
    /// 标准 ESM 入口；与 script 互斥，路径相对于 package_root。
    #[serde(default)]
    module_entry: Option<String>,
    input: Value,
    #[serde(default)]
    invocation: Option<Invocation>,
    #[serde(default)]
    package_root: Option<String>,
    #[serde(default)]
    native_context: Option<Value>,
    /// Host绑定由生产者明确标记，即使建桥失败也禁止本地桌面回退。
    #[serde(default)]
    desktop_mode: Option<api::DesktopMode>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    invocation_id: String,
    binding_id: String,
}

fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() == 1 && arguments[0] == "--describe-runtime" {
        println!("{}", runtime_description::describe());
        return;
    }
    if !arguments.is_empty() {
        println!(
            "{}",
            json!({"schema_version":1,"ok":false,"error":"未知宿主参数。"})
        );
        std::process::exit(1);
    }
    let response = match run() {
        Ok(value) => json!({"schema_version":1,"ok": true, "result": value}),
        Err(error) => json!({"schema_version":1,"ok": false, "error": error}),
    };
    println!("{response}");
    if response["ok"] != true {
        std::process::exit(1);
    }
}

fn run() -> Result<Value, Value> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_REQUEST {
        return Err("请求超过 256 KiB。".into());
    }
    let request: Request = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if !request.input.is_object() {
        return Err("input 必须是 JSON 对象。".into());
    }
    if let Some(invocation) = &request.invocation {
        for id in [&invocation.invocation_id, &invocation.binding_id] {
            if id.is_empty()
                || !id.bytes().all(|b| b.is_ascii_digit())
                || id.parse::<u64>().is_err()
            {
                return Err("调用身份必须为 u64 十进制字符串。".into());
            }
        }
    }
    if request
        .package_root
        .as_ref()
        .is_some_and(|root| !std::path::Path::new(root).is_absolute())
    {
        return Err("package_root 必须为绝对路径。".into());
    }
    let modules = if let Some(entry) = &request.module_entry {
        if request.script.is_some() {
            return Err(modules::fault(
                "module_entry",
                "module_entry 与 script 不能同时提供。",
                "not_started",
            ));
        }
        Some(modules::PackageModules::new(
            request.package_root.as_deref().ok_or_else(|| {
                modules::fault("module_entry", "模块入口需要 package_root。", "not_started")
            })?,
            entry,
        )?)
    } else {
        if request.script.is_none() {
            return Err(modules::fault(
                "invalid_request",
                "请求需要 script 或 module_entry。",
                "not_started",
            ));
        }
        None
    };
    let desktop_mode =
        api::DesktopMode::resolve(request.desktop_mode, request.native_context.as_ref())
            .map_err(|error| serde_json::to_value(error).expect("API error JSON"))?;
    let _desktop_scope = api::DesktopScope::enter(desktop_mode);
    let _async_scope = api::AsyncScope::enter(request.native_context.as_ref());
    let metadata = json!({"invocation":request.invocation,"packageRoot":request.package_root,
        "__nativeContext":request.native_context,"__localDesktopAllowed":desktop_mode.local_allowed()});
    let metadata =
        serde_json::to_string(&metadata.to_string()).map_err(|error| error.to_string())?;
    // 将 JSON 编码为 JS 字符串再解析，避免 __proto__ 对象字面量语义及参数插值。
    let input = serde_json::to_string(&request.input).map_err(|error| error.to_string())?;
    let literal = serde_json::to_string(&input).map_err(|error| error.to_string())?;
    let source = format!(
        "(() => {{ const ctx = {{ input: JSON.parse({literal}), ...JSON.parse({metadata}) }};\n{TEXT_ENCODER}\n{DESKTOP_API}\n{BROWSER_API}\n{HISTORY_API}\n{PROCESS_CAPTURE_API}\n{ACTIONS_API}\n{STATE_API}\n{SDK}\n\
         return makeScriptRunner(); }})()",
    );
    // 首行不插入换行，避免把用户文件行号平移到 SDK 后面；首行列号含函数前缀。
    let entry = format!(
        "(function(ctx) {{{}\n}})",
        request.script.as_deref().unwrap_or_default()
    );
    let output = evaluate(&source, &entry, modules.as_ref())?;
    if output.len() > 80 * 1024 {
        return Err("结果超过 64 KiB。".into());
    }
    let envelope: Value = serde_json::from_str(&output).map_err(|error| error.to_string())?;
    match envelope["kind"].as_str() {
        Some("value") => {
            let result = envelope.get("value").ok_or("入口必须返回 JSON 结果")?;
            if result.to_string().len() > 64 * 1024 {
                return Err("结果超过 64 KiB。".into());
            }
            Ok(result.clone())
        }
        Some("fault") => Err(envelope["error"].clone()),
        _ => Err("引擎未返回合法结果封装。".into()),
    }
}

#[cfg(feature = "boa")]
fn evaluate(
    source: &str,
    entry: &str,
    modules: Option<&std::rc::Rc<modules::PackageModules>>,
) -> Result<String, Value> {
    use boa_engine::{Context, Source};
    let loader = modules.map(|package| modules::boa::Loader::new(std::rc::Rc::clone(package)));
    let budget = ExecutionBudget::new();
    let jobs = std::rc::Rc::new(BoundedJobs {
        inner: std::rc::Rc::new(boa_engine::job::SimpleJobExecutor::new()),
        budget: budget.clone(),
    });
    let mut context = if let Some(loader) = &loader {
        Context::builder()
            .module_loader(std::rc::Rc::clone(loader))
            .job_executor(jobs.clone())
            .build()
            .map_err(|e| e.to_string())?
    } else {
        Context::builder()
            .job_executor(jobs.clone())
            .build()
            .map_err(|error| error.to_string())?
    };
    use boa_engine::{js_string, JsValue, NativeFunction};
    context
        .register_global_builtin_callable(
            js_string!("__hostCall"),
            1,
            NativeFunction::from_fn_ptr(|_, args, context| {
                let request = args
                    .first()
                    .unwrap_or(&JsValue::undefined())
                    .to_string(context)?
                    .to_std_string_escaped();
                Ok(js_string!(api::dispatch(request)).into())
            }),
        )
        .map_err(|error| error.to_string())?;
    context
        .register_global_builtin_callable(
            js_string!("__hostCallAsync"),
            1,
            NativeFunction::from_fn_ptr(|_, args, context| {
                let request = args
                    .first()
                    .unwrap_or(&JsValue::undefined())
                    .to_string(context)?
                    .to_std_string_escaped();
                Ok(js_string!(api::dispatch_async(request)).into())
            }),
        )
        .map_err(|error| error.to_string())?;
    context
        .register_global_builtin_callable(
            js_string!("__hostPollAsync"),
            0,
            NativeFunction::from_fn_ptr(|_, _, _| Ok(js_string!(api::poll_async()).into())),
        )
        .map_err(|error| error.to_string())?;
    context
        .register_global_builtin_callable(
            js_string!("__hostAsyncPending"),
            0,
            NativeFunction::from_fn_ptr(|_, _, _| Ok((api::async_pending() as u32).into())),
        )
        .map_err(|error| error.to_string())?;
    context
        .runtime_limits_mut()
        .set_loop_iteration_limit(1_000_000);
    context.runtime_limits_mut().set_recursion_limit(128);
    let fields = context
        .eval(Source::from_bytes(FAULT_FIELDS))
        .map_err(|error| error.to_string())?;
    let runner = context
        .eval(Source::from_bytes(source).with_path(std::path::Path::new("omnitrigger-sdk.js")))
        .map_err(|error| error.to_string())?;
    let entry = if let Some(loader) = &loader {
        modules::boa::entry(loader, &mut context, &fields)?
    } else {
        context
            .eval(Source::from_bytes(entry).with_path(std::path::Path::new("action.js")))
            .map_err(|error| {
                let rendered = format!("action.js\n{error}");
                diagnostic_error(None, rendered)
            })?
    };
    let value = runner
        .as_callable()
        .ok_or("SDK 未返回入口执行函数。")?
        .call(&JsValue::undefined(), &[entry], &mut context)
        .map_err(|error| boa_error(error, &fields, &mut context))?;
    budget.check()?;
    let runner_object = runner.as_object().ok_or("SDK未返回函数对象。")?;
    let poll = runner_object
        .get(js_string!("__pollAsync"), &mut context)
        .map_err(|error| boa_error(error, &fields, &mut context))?;
    let pending = runner_object
        .get(js_string!("__pendingAsync"), &mut context)
        .map_err(|error| boa_error(error, &fields, &mut context))?;
    let promise = value.as_object().and_then(|object| {
        boa_engine::object::builtins::JsPromise::from_object(object.clone()).ok()
    });
    if let Some(promise) = promise {
        loop {
            budget.check()?;
            if let Err(error) = context.run_jobs() {
                budget.check()?;
                return Err(boa_error(error, &fields, &mut context));
            }
            budget.check()?;
            match promise.state() {
                boa_engine::builtins::promise::PromiseState::Fulfilled(value) => {
                    let outstanding = pending
                        .as_callable()
                        .ok_or("异步计数函数缺失。")?
                        .call(&JsValue::undefined(), &[], &mut context)
                        .map_err(|error| boa_error(error, &fields, &mut context))?
                        .as_number()
                        .unwrap_or_default();
                    if api::async_pending() != 0 || outstanding != 0. {
                        return Err(unfinished_async());
                    }
                    return value
                        .as_string()
                        .map(|text| text.to_std_string_escaped())
                        .ok_or_else(|| Value::from("引擎未返回JSON字符串。"));
                }
                boa_engine::builtins::promise::PromiseState::Rejected(value) => {
                    return Err(boa_error(
                        boa_engine::JsError::from_opaque(value),
                        &fields,
                        &mut context,
                    ))
                }
                boa_engine::builtins::promise::PromiseState::Pending => {}
            }
            let completed = poll
                .as_callable()
                .ok_or("异步完成函数缺失。")?
                .call(&JsValue::undefined(), &[], &mut context)
                .map_err(|error| boa_error(error, &fields, &mut context))?
                .as_number()
                .unwrap_or_default();
            if completed != 0. {
                continue;
            }
            if api::async_pending() == 0 {
                return Err(modules::fault(
                    "script_promise_pending",
                    "Promise尚未结束且没有可等待的本机调用。",
                    "unknown",
                ));
            }
            api::wait_async();
        }
    }
    let outstanding = pending
        .as_callable()
        .ok_or("异步计数函数缺失。")?
        .call(&JsValue::undefined(), &[], &mut context)
        .map_err(|error| boa_error(error, &fields, &mut context))?
        .as_number()
        .unwrap_or_default();
    if api::async_pending() != 0 || outstanding != 0. {
        return Err(unfinished_async());
    }
    value
        .as_string()
        .map(|value| value.to_std_string_escaped())
        .ok_or_else(|| "引擎未返回JSON字符串。".into())
}

#[cfg(feature = "boa")]
fn boa_error(
    error: boa_engine::JsError,
    fields: &boa_engine::JsValue,
    context: &mut boa_engine::Context,
) -> Value {
    let rendered = error.to_string();
    // Boa 的不可捕获限额异常不能转换为 JS 对象，也不再执行错误字段读取代码。
    if error
        .as_native()
        .is_some_and(|error| error.is_runtime_limit())
    {
        return diagnostic_error(None, rendered);
    }
    let opaque = error.to_opaque(context);
    let fields = fields
        .as_callable()
        .and_then(|reader| {
            reader
                .call(&boa_engine::JsValue::undefined(), &[opaque], context)
                .ok()
        })
        .and_then(|value| value.as_string().map(|value| value.to_std_string_escaped()));
    diagnostic_error(fields, rendered)
}

#[cfg(feature = "quickjs")]
fn evaluate(
    source: &str,
    entry: &str,
    modules: Option<&std::rc::Rc<modules::PackageModules>>,
) -> Result<String, Value> {
    use rquickjs::{Context, Runtime};
    let budget = ExecutionBudget::new();
    let runtime = Runtime::new().map_err(|error| error.to_string())?;
    if let Some(package) = modules {
        runtime.set_loader(
            modules::quickjs::Loader(std::rc::Rc::clone(package)),
            modules::quickjs::Loader(std::rc::Rc::clone(package)),
        );
    }
    runtime.set_memory_limit(32 * 1024 * 1024);
    runtime.set_max_stack_size(512 * 1024);
    let interrupt = budget.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || interrupt.expired())));
    let context = Context::full(&runtime).map_err(|error| error.to_string())?;
    context.with(|ctx| {
        for (name, function) in [
            (
                "__hostCall",
                rquickjs::Function::new(ctx.clone(), api::dispatch),
            ),
            (
                "__hostCallAsync",
                rquickjs::Function::new(ctx.clone(), api::dispatch_async),
            ),
        ] {
            ctx.globals()
                .set(name, function.map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        }
        ctx.globals()
            .set(
                "__hostPollAsync",
                rquickjs::Function::new(ctx.clone(), api::poll_async).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        ctx.globals()
            .set(
                "__hostAsyncPending",
                rquickjs::Function::new(ctx.clone(), api::async_pending)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        let mut options = rquickjs::context::EvalOptions::default();
        options.filename = Some("omnitrigger-sdk.js".into());
        let fields: rquickjs::Function =
            ctx.eval(FAULT_FIELDS).map_err(|error| error.to_string())?;
        let runner: rquickjs::Function = ctx
            .eval_with_options(source, options)
            .map_err(|error| quickjs_error(&ctx, &fields, error))?;
        let poll: rquickjs::Function = runner
            .get("__pollAsync")
            .map_err(|error| quickjs_error(&ctx, &fields, error))?;
        let pending: rquickjs::Function = runner
            .get("__pendingAsync")
            .map_err(|error| quickjs_error(&ctx, &fields, error))?;
        let mut options = rquickjs::context::EvalOptions::default();
        options.filename = Some("action.js".into());
        let entry: rquickjs::Function = if let Some(package) = modules {
            modules::quickjs::entry(package, &ctx, &fields)?
        } else {
            ctx.eval_with_options(entry, options)
                .map_err(|error| quickjs_error(&ctx, &fields, error))?
        };
        let value: rquickjs::Value = runner
            .call((entry,))
            .map_err(|error| quickjs_error(&ctx, &fields, error))?;
        if let Some(promise) = value.as_promise() {
            loop {
                budget.check()?;
                while ctx.execute_pending_job() {
                    if !budget.next_job() {
                        return Err(modules::fault(
                            "script_budget_exceeded",
                            "Promise任务预算已耗尽。",
                            "unknown",
                        ));
                    }
                }
                if let Some(result) = promise.result::<String>() {
                    let result = result.map_err(|error| quickjs_error(&ctx, &fields, error))?;
                    if api::async_pending() != 0
                        || pending
                            .call::<_, usize>(())
                            .map_err(|error| quickjs_error(&ctx, &fields, error))?
                            != 0
                    {
                        return Err(unfinished_async());
                    }
                    return Ok(result);
                }
                let completed: usize = poll
                    .call(())
                    .map_err(|error| quickjs_error(&ctx, &fields, error))?;
                if completed != 0 {
                    continue;
                }
                if api::async_pending() == 0 {
                    return Err(modules::fault(
                        "script_promise_pending",
                        "Promise尚未结束且没有可等待的本机调用。",
                        "unknown",
                    ));
                }
                api::wait_async();
            }
        }
        if api::async_pending() != 0
            || pending
                .call::<_, usize>(())
                .map_err(|error| quickjs_error(&ctx, &fields, error))?
                != 0
        {
            return Err(unfinished_async());
        }
        value
            .as_string()
            .ok_or_else(|| Value::from("引擎未返回JSON字符串。"))?
            .to_string()
            .map_err(|error| quickjs_error(&ctx, &fields, error))
    })
}

#[cfg(feature = "quickjs")]
fn quickjs_exception(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error) -> String {
    if !error.is_exception() {
        return error.to_string();
    }
    let exception = ctx.catch();
    if let Some(error) = exception.as_exception() {
        error.to_string()
    } else {
        format!("JavaScript 异常：{exception:?}")
    }
}

#[cfg(feature = "quickjs")]
fn quickjs_error<'js>(
    ctx: &rquickjs::Ctx<'js>,
    fields: &rquickjs::Function<'js>,
    error: rquickjs::Error,
) -> Value {
    if !error.is_exception() {
        return error.to_string().into();
    }
    let exception = ctx.catch();
    let rendered = if let Some(error) = exception.as_exception() {
        error.to_string()
    } else if let Some(text) = exception.as_string() {
        format!("JavaScript 异常：{}", text.to_string().unwrap_or_default())
    } else {
        format!("JavaScript 异常：{exception:?}")
    };
    let fields = match fields.call::<_, Option<String>>((exception,)) {
        Ok(fields) => fields,
        Err(_) => {
            let _ = ctx.catch();
            None
        }
    };
    diagnostic_error(fields, rendered)
}

struct ExecutionBudget {
    began: std::time::Instant,
    host_baseline: u64,
    jobs: std::sync::atomic::AtomicUsize,
}
impl ExecutionBudget {
    fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            began: std::time::Instant::now(),
            host_baseline: api::excluded_host_micros(),
            jobs: std::sync::atomic::AtomicUsize::new(0),
        })
    }
    fn expired(&self) -> bool {
        let excluded = api::excluded_host_micros().saturating_sub(self.host_baseline);
        self.began.elapsed()
            >= std::time::Duration::from_secs(2) + std::time::Duration::from_micros(excluded)
    }
    fn next_job(&self) -> bool {
        self.jobs.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 10_000 && !self.expired()
    }
    fn check(&self) -> Result<(), Value> {
        if self.expired() || self.jobs.load(std::sync::atomic::Ordering::Relaxed) >= 10_000 {
            Err(modules::fault(
                "script_budget_exceeded",
                "脚本CPU或Promise任务预算已耗尽。",
                "unknown",
            ))
        } else {
            Ok(())
        }
    }
}

fn unfinished_async() -> Value {
    modules::fault(
        "async_calls_pending",
        "入口已结束，但仍有未完成的异步浏览器调用；本次资源将取消收尾。",
        "unknown",
    )
}

#[cfg(feature = "boa")]
struct BoundedJobs {
    inner: std::rc::Rc<boa_engine::job::SimpleJobExecutor>,
    budget: std::sync::Arc<ExecutionBudget>,
}
#[cfg(feature = "boa")]
impl boa_engine::job::JobExecutor for BoundedJobs {
    fn enqueue_job(
        self: std::rc::Rc<Self>,
        job: boa_engine::job::Job,
        context: &mut boa_engine::Context,
    ) {
        use boa_engine::job::{GenericJob, Job, PromiseJob};
        let budget = self.budget.clone();
        let check = move || -> boa_engine::JsResult<()> {
            if budget.next_job() {
                Ok(())
            } else {
                Err(boa_engine::JsNativeError::error()
                    .with_message("脚本Promise任务预算已耗尽。")
                    .into())
            }
        };
        let wrapped = match job {
            Job::PromiseJob(job) => {
                let realm = job.realm().cloned();
                let run = move |context: &mut boa_engine::Context| {
                    check()?;
                    job.call(context)
                };
                Job::PromiseJob(if let Some(realm) = realm {
                    PromiseJob::with_realm(run, realm)
                } else {
                    PromiseJob::new(run)
                })
            }
            Job::GenericJob(job) => {
                let realm = job.realm().clone();
                Job::GenericJob(GenericJob::new(
                    move |context| {
                        check()?;
                        job.call(context)
                    },
                    realm,
                ))
            }
            other => other,
        };
        boa_engine::job::JobExecutor::enqueue_job(self.inner.clone(), wrapped, context);
    }
    fn run_jobs(
        self: std::rc::Rc<Self>,
        context: &mut boa_engine::Context,
    ) -> boa_engine::JsResult<()> {
        boa_engine::job::JobExecutor::run_jobs(self.inner.clone(), context)
    }
}
