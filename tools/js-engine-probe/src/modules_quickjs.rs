use super::{fault, PackageModules};
use rquickjs::{
    loader::{ImportAttributes, Loader as ModuleLoader, Resolver},
    Ctx, Module,
};
use std::{path::Path, rc::Rc};

pub struct Loader(pub Rc<PackageModules>);

impl Loader {
    fn error(&self, ctx: &Ctx<'_>, error: serde_json::Value) -> rquickjs::Error {
        let error = self.0.execution_fault(error);
        let create = || -> rquickjs::Result<rquickjs::Error> {
            let exception = rquickjs::Exception::from_message(
                ctx.clone(),
                error["message"].as_str().unwrap_or("模块加载失败"),
            )?;
            exception
                .as_object()
                .set("code", error["code"].as_str().unwrap_or_default())?;
            exception.as_object().set(
                "effectState",
                error["effect_state"].as_str().unwrap_or_default(),
            )?;
            Ok(exception.throw())
        };
        let (Ok(error) | Err(error)) = create();
        error
    }
}

impl Resolver for Loader {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if attributes.is_some_and(|attrs| attrs.keys().next().is_some()) {
            return Err(self.error(
                ctx,
                fault(
                    "module_specifier",
                    "仅支持无导入属性的 JavaScript 模块。",
                    "not_started",
                ),
            ));
        }
        self.0
            .resolve(Path::new(base), name)
            .map(|path| path.to_str().expect("validated UTF-8 path").to_owned())
            .map_err(|error| self.error(ctx, error))
    }
}

impl ModuleLoader for Loader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        let source = self
            .0
            .source(Path::new(name))
            .map_err(|error| self.error(ctx, error))?;
        Module::declare(ctx.clone(), name, source).map_err(|error| {
            if error.is_exception() {
                let exception = ctx.catch();
                let structured = exception.as_object().is_some_and(|object| {
                    object
                        .get::<_, Option<String>>("code")
                        .ok()
                        .flatten()
                        .is_some()
                });
                if structured {
                    return ctx.throw(exception);
                }
                // 将原始语法异常放回，统一渲染其源文件和行号。
                let _ = ctx.throw(exception);
            }
            // 保留解析器的原生异常供上层渲染行号；只额外记录当前源文件。
            let rendered = crate::quickjs_exception(ctx, error);
            self.error(
                ctx,
                fault("module_parse", format!("{name}\n{rendered}"), "not_started"),
            )
        })
    }
}

pub fn entry<'js>(
    package: &PackageModules,
    ctx: &Ctx<'js>,
    fields: &rquickjs::Function<'js>,
) -> Result<rquickjs::Function<'js>, serde_json::Value> {
    let render = |error| crate::quickjs_error(ctx, fields, error);
    let source = package.source(&package.entry)?;
    let module = Module::declare(
        ctx.clone(),
        package.entry.to_str().expect("validated UTF-8 path"),
        source,
    )
    .map_err(|error| {
        let error = render(error);
        if error.is_object() {
            error
        } else {
            fault(
                "module_parse",
                format!(
                    "{}\n{}",
                    package.entry.display(),
                    error.as_str().unwrap_or_default()
                ),
                "not_started",
            )
        }
    })?;
    package.begin_execution();
    let (module, promise) = module.eval().map_err(render)?;
    promise.finish::<()>().map_err(|error| {
        if matches!(error, rquickjs::Error::WouldBlock) {
            fault(
                "module_pending",
                "模块初始化仍在等待 Promise；宿主没有异步事件循环。",
                "unknown",
            )
        } else {
            render(error)
        }
    })?;
    module.get::<_, rquickjs::Function>("main").map_err(|_| {
        // 清除缺失/非函数导出的转换异常，不把它混入后续诊断。
        let _ = ctx.catch();
        fault(
            "module_entry",
            format!(
                "{}：模块入口须具名导出同步函数 main(ctx)。",
                package.entry.display()
            ),
            "unknown",
        )
    })
}
