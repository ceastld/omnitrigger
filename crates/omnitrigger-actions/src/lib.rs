//! 与输入匹配及界面无关的应用动作契约。执行发生在后台，目录加载不执行用户代码。
pub mod catalog;
pub mod model;
mod native;
pub mod node;
pub mod project;
pub mod runtime_capabilities;
pub mod script_host;
pub mod service;

pub use catalog::{CatalogAction, PackageCatalog, ResolvedAction};
pub use model::{ActionCall, ActionDefinition, ActionError, PackageManifest};
pub use native::NativeHost;
pub use node::NodeRuntime;
pub use script_host::ScriptHostRuntime;
pub use service::{
    ActionCatalogDescription, ActionCatalogSnapshot, ActionEvent, ActionOutcome, ActionRuntime,
    ActionService, CancellationToken, InvocationContext, RuntimeRegistry,
};
