use super::{fault, PackageModules};
use boa_engine::{
    module::Referrer, Context, JsError, JsNativeError, JsResult, JsString, Module, Source,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

pub struct Loader {
    pub package: Rc<PackageModules>,
    parsed: RefCell<HashMap<PathBuf, Module>>,
}

impl Loader {
    pub fn new(package: Rc<PackageModules>) -> Rc<Self> {
        Rc::new(Self {
            package,
            parsed: RefCell::new(HashMap::new()),
        })
    }

    pub fn parse(&self, path: &Path, context: &mut Context) -> JsResult<Module> {
        if let Some(module) = self.parsed.borrow().get(path) {
            return Ok(module.clone());
        }
        let source = self
            .package
            .source(path)
            .map_err(|error| self.error(error, context))?;
        let module = Module::parse(Source::from_bytes(&source).with_path(path), None, context)
            .map_err(|error| {
                self.error(
                    fault(
                        "module_parse",
                        format!("{}\n{error}", path.display()),
                        "not_started",
                    ),
                    context,
                )
            })?;
        // 先缓存解析记录再链接依赖，循环导入由原生 ESM 处理。
        self.parsed
            .borrow_mut()
            .insert(path.to_path_buf(), module.clone());
        Ok(module)
    }

    fn error(&self, error: serde_json::Value, context: &mut Context) -> JsError {
        let error = self.package.execution_fault(error);
        let object = JsNativeError::typ()
            .with_message(
                error["message"]
                    .as_str()
                    .unwrap_or("模块加载失败")
                    .to_owned(),
            )
            .to_opaque(context);
        for (key, value) in [
            ("code", &error["code"]),
            ("effectState", &error["effect_state"]),
        ] {
            if let Err(error) = object.set(
                boa_engine::js_string!(key),
                boa_engine::js_string!(value.as_str().unwrap_or_default()),
                false,
                context,
            ) {
                return error;
            }
        }
        JsError::from_opaque(object.into())
    }
}

impl boa_engine::module::ModuleLoader for Loader {
    async fn load_imported_module(
        self: Rc<Self>,
        referrer: Referrer,
        specifier: JsString,
        context: &RefCell<&mut Context>,
    ) -> JsResult<Module> {
        let path = self
            .package
            .resolve(
                referrer.path().unwrap_or(Path::new("")),
                &specifier.to_std_string_escaped(),
            )
            .map_err(|error| self.error(error, &mut context.borrow_mut()))?;
        self.parse(&path, &mut context.borrow_mut())
    }
}

pub fn entry(
    loader: &Loader,
    context: &mut Context,
    fields: &boa_engine::JsValue,
) -> Result<boa_engine::JsValue, serde_json::Value> {
    use boa_engine::builtins::promise::PromiseState;
    let render = |error, context: &mut Context| crate::boa_error(error, fields, context);
    let module = loader
        .parse(&loader.package.entry, context)
        .map_err(|e| render(e, context))?;
    let promise = module.load(context);
    context.run_jobs().map_err(|e| render(e, context))?;
    match promise.state() {
        PromiseState::Fulfilled(_) => {}
        PromiseState::Rejected(value) => return Err(render(JsError::from_opaque(value), context)),
        PromiseState::Pending => {
            return Err(fault(
                "module_pending",
                "模块依赖加载未完成。",
                "not_started",
            ))
        }
    }
    module.link(context).map_err(|e| render(e, context))?;
    loader.package.begin_execution();
    let promise = module.evaluate(context);
    context.run_jobs().map_err(|e| render(e, context))?;
    match promise.state() {
        PromiseState::Fulfilled(_) => {}
        PromiseState::Rejected(value) => return Err(render(JsError::from_opaque(value), context)),
        PromiseState::Pending => {
            return Err(fault(
                "module_pending",
                "模块初始化存在未完成的顶层 Promise；异步本机等待仅支持 main(ctx)。",
                "unknown",
            ))
        }
    }
    let main = module
        .get_value(boa_engine::js_string!("main"), context)
        .map_err(|e| render(e, context))?;
    if main.as_callable().is_none() {
        return Err(fault(
            "module_entry",
            format!(
                "{}：模块入口须具名导出函数 main(ctx)。",
                loader.package.entry.display()
            ),
            "unknown",
        ));
    }
    Ok(main)
}
