use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, Read};

mod api;
mod native;
const DESKTOP_API: &str =
    include_str!("../../../crates/omnitrigger-actions/runtime/desktop-api.js");
const SDK: &str = include_str!("sdk.js");
const BROWSER_API: &str =
    include_str!("../../../crates/omnitrigger-actions/runtime/browser-api.js");

#[cfg(any(
    all(feature = "boa", feature = "quickjs"),
    not(any(feature = "boa", feature = "quickjs"))
))]
compile_error!("请选择且只选择 boa 或 quickjs 引擎。");

const MAX_REQUEST: u64 = 256 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    /// 同步函数体；通过 ctx.input 读取参数，通过 return 返回 JSON 结果。
    script: String,
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
        println!(
            "{}",
            json!({"schema_version":1,"product":"omnitrigger-script-host","version":env!("CARGO_PKG_VERSION"),"engine":if cfg!(feature="quickjs"){"quickjs"}else{"boa"},"api_version":1,"protocol_schema":1})
        );
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
    let metadata = json!({"invocation":request.invocation,"packageRoot":request.package_root,"__nativeContext":request.native_context});
    let metadata =
        serde_json::to_string(&metadata.to_string()).map_err(|error| error.to_string())?;
    // 将 JSON 编码为 JS 字符串再解析，避免 __proto__ 对象字面量语义及参数插值。
    let input = serde_json::to_string(&request.input).map_err(|error| error.to_string())?;
    let literal = serde_json::to_string(&input).map_err(|error| error.to_string())?;
    let source = format!(
        "(() => {{ const ctx = {{ input: JSON.parse({literal}), ...JSON.parse({metadata}) }};\n{DESKTOP_API}\n{BROWSER_API}\n{SDK}\n\
         try {{ const result = (function(ctx) {{\n{}\n}})(ctx);\n\
         if (result && typeof result.then === 'function') throw new Error('原型仅支持同步入口');\n\
         const output = JSON.stringify(result);\n\
         if (typeof output !== 'string') throw new Error('入口必须返回 JSON 结果');\n\
         return JSON.stringify({{kind:'value', value:JSON.parse(output)}});\n\
         }} catch(error) {{\n\
           if (typeof error?.code !== 'string' || !['not_started','unknown'].includes(error.effectState)) throw error;\n\
           return JSON.stringify({{kind:'fault',error:{{code:error.code,message:String(error.message),effect_state:error.effectState,method:error.method,required_permissions:error.requiredPermissions}}}});\n\
         }} }})()",
        request.script
    );
    let output = evaluate(&source)?;
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
fn evaluate(source: &str) -> Result<String, String> {
    use boa_engine::{Context, Source};
    let mut context = Context::default();
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
    let value = context
        .eval(Source::from_bytes(source))
        .map_err(|error| error.to_string())?;
    value
        .as_string()
        .map(|value| value.to_std_string_escaped())
        .ok_or_else(|| "引擎未返回 JSON 字符串。".into())
}

#[cfg(feature = "quickjs")]
fn evaluate(source: &str) -> Result<String, String> {
    use rquickjs::{Context, Runtime};
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    let runtime = Runtime::new().map_err(|error| error.to_string())?;
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
        ctx.eval::<String, _>(source).map_err(|error| {
            if error.is_exception() {
                let exception = ctx.catch();
                if let Some(exception) = exception.as_exception() {
                    return exception.to_string();
                }
                return format!("JavaScript 异常：{exception:?}");
            }
            error.to_string()
        })
    })
}
