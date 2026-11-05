use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, Read};

mod api;
mod modules;
mod native;
mod runtime_description;
const DESKTOP_API: &str =
    include_str!("../../../crates/omnitrigger-actions/runtime/desktop-api.js");
const SDK: &str = include_str!("sdk.js");
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
    /// 同步函数体；通过 ctx.input 读取参数，通过 return 返回 JSON 结果。
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
    let metadata = json!({"invocation":request.invocation,"packageRoot":request.package_root,"__nativeContext":request.native_context});
    let metadata =
        serde_json::to_string(&metadata.to_string()).map_err(|error| error.to_string())?;
    // 将 JSON 编码为 JS 字符串再解析，避免 __proto__ 对象字面量语义及参数插值。
    let input = serde_json::to_string(&request.input).map_err(|error| error.to_string())?;
    let literal = serde_json::to_string(&input).map_err(|error| error.to_string())?;
    let source = format!(
        "(() => {{ const ctx = {{ input: JSON.parse({literal}), ...JSON.parse({metadata}) }};\n{DESKTOP_API}\n{BROWSER_API}\n{HISTORY_API}\n{SDK}\n\
         return function(entry) {{ const result = entry(ctx);\n\
         if (result && typeof result.then === 'function') throw new Error('脚本入口必须同步返回 JSON 数据');\n\
         const output = JSON.stringify(result);\n\
         if (typeof output !== 'string') throw new Error('入口必须返回 JSON 结果');\n\
         return JSON.stringify({{kind:'value', value:JSON.parse(output)}});\n\
         }} }})()",
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
    let mut context = if let Some(loader) = &loader {
        Context::builder()
            .module_loader(std::rc::Rc::clone(loader))
            .build()
            .map_err(|e| e.to_string())?
    } else {
        Context::default()
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
    value
        .as_string()
        .map(|value| value.to_std_string_escaped())
        .ok_or_else(|| "引擎未返回 JSON 字符串。".into())
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
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    let runtime = Runtime::new().map_err(|error| error.to_string())?;
    if let Some(package) = modules {
        runtime.set_loader(
            modules::quickjs::Loader(std::rc::Rc::clone(package)),
            modules::quickjs::Loader(std::rc::Rc::clone(package)),
        );
    }
    runtime.set_memory_limit(32 * 1024 * 1024);
    runtime.set_max_stack_size(512 * 1024);
    let began = Instant::now();
    let host_time = Arc::new(AtomicU64::new(0));
    let elapsed = Arc::clone(&host_time);
    runtime.set_interrupt_handler(Some(Box::new(move || {
        began.elapsed()
            >= Duration::from_secs(2) + Duration::from_micros(elapsed.load(Ordering::Relaxed))
    })));
    let context = Context::full(&runtime).map_err(|error| error.to_string())?;
    context.with(|ctx| {
        ctx.globals()
            .set(
                "__hostCall",
                rquickjs::Function::new(ctx.clone(), move |request: String| {
                    let began = Instant::now();
                    let result = api::dispatch(request);
                    host_time.fetch_add(began.elapsed().as_micros() as u64, Ordering::Relaxed);
                    result
                })
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        let mut options = rquickjs::context::EvalOptions::default();
        options.filename = Some("omnitrigger-sdk.js".into());
        let fields: rquickjs::Function =
            ctx.eval(FAULT_FIELDS).map_err(|error| error.to_string())?;
        let runner: rquickjs::Function = ctx
            .eval_with_options(source, options)
            .map_err(|error| error.to_string())?;
        let mut options = rquickjs::context::EvalOptions::default();
        options.filename = Some("action.js".into());
        let entry: rquickjs::Function = if let Some(package) = modules {
            modules::quickjs::entry(package, &ctx, &fields)?
        } else {
            ctx.eval_with_options(entry, options)
                .map_err(|error| quickjs_error(&ctx, &fields, error))?
        };
        runner
            .call((entry,))
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
