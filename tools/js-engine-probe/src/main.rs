use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, Read};

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
}

fn main() {
    let response = match run() {
        Ok(value) => json!({"ok": true, "result": value}),
        Err(error) => json!({"ok": false, "error": error}),
    };
    println!("{response}");
    if response["ok"] != true {
        std::process::exit(1);
    }
}

fn run() -> Result<Value, String> {
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
    // 将 JSON 编码为 JS 字符串再解析，避免 __proto__ 对象字面量语义及参数插值。
    let input = serde_json::to_string(&request.input).map_err(|error| error.to_string())?;
    let literal = serde_json::to_string(&input).map_err(|error| error.to_string())?;
    let source = format!(
        "(() => {{ const ctx = {{ input: JSON.parse({literal}) }};\n\
         const result = (function(ctx) {{\n{}\n}})(ctx);\n\
         if (result && typeof result.then === 'function') throw new Error('原型仅支持同步入口');\n\
         const output = JSON.stringify(result);\n\
         if (typeof output !== 'string') throw new Error('入口必须返回 JSON 结果');\n\
         return output; }})()",
        request.script
    );
    let output = evaluate(&source)?;
    if output.len() > 64 * 1024 {
        return Err("结果超过 64 KiB。".into());
    }
    serde_json::from_str(&output).map_err(|error| error.to_string())
}

#[cfg(feature = "boa")]
fn evaluate(source: &str) -> Result<String, String> {
    use boa_engine::{Context, Source};
    let mut context = Context::default();
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
    use std::time::{Duration, Instant};
    let runtime = Runtime::new().map_err(|error| error.to_string())?;
    runtime.set_memory_limit(32 * 1024 * 1024);
    runtime.set_max_stack_size(512 * 1024);
    let deadline = Instant::now() + Duration::from_secs(2);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let context = Context::full(&runtime).map_err(|error| error.to_string())?;
    context.with(|ctx| {
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
