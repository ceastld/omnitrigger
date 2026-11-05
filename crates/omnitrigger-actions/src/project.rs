//! 标准 JS 工程与 SDK 的本地分发；不加载运行时，也不运行用户代码。
use crate::{ActionCall, ActionError};
use serde::Serialize;
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const SCRIPT_API: &str = include_str!("../runtime/script-types.d.ts");
const ENTRY: &str = include_str!("../../../examples/actions/script-modules/main.mjs");
const MODULE: &str = include_str!("../../../examples/actions/script-modules/lib/text.js");
const README: &str = include_str!("../../../examples/actions/script-modules/README.md");

#[derive(Debug, Serialize)]
pub struct ScriptProject {
    pub package_root: PathBuf,
    pub entry_path: PathBuf,
    pub call_path: PathBuf,
    pub sdk_path: PathBuf,
    pub call: ActionCall,
}

fn fault(code: &str, message: impl Into<String>, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, message);
    error.effect_state = Some(effect.into());
    error
}

fn valid_project_id(id: &str) -> bool {
    let upper = id.to_ascii_uppercase();
    !id.is_empty()
        && id.len() <= 64
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        && !matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

/// 在已有本地父目录下创建 actions 根目录与全新的包目录。
/// 不覆盖已有目录；写入失败可能保留部分工程，调用者应报告路径，不能盲目重试。
pub fn create_script_project(root: &Path, id: &str) -> Result<ScriptProject, ActionError> {
    create_project(root, id, id)
}

/// 用户界面只需名称；稳定标识由系统生成，不将显示名称当作目录或调用身份。
pub fn create_named_script_project(root: &Path, name: &str) -> Result<ScriptProject, ActionError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(fault(
            "invalid_project_name",
            "脚本名称需要 1–80 个可见字符。",
            "not_started",
        ));
    }
    let mut identity = [0u8; 8];
    getrandom::fill(&mut identity)
        .map_err(|error| fault("project_identity", error.to_string(), "not_started"))?;
    let id = format!("script-{}", u64::from_le_bytes(identity));
    create_project(root, &id, name)
}

/// 编辑入口重新验证磁盘目录，不依赖界面中的旧快照，不启动任何动作运行时。
pub fn editable_script(root: &Path, call: &ActionCall) -> Result<(PathBuf, PathBuf), ActionError> {
    let action = crate::PackageCatalog::load(root).resolve(call)?;
    if action.definition.runtime != "script" {
        return Err(fault(
            "not_script_project",
            "此动作不是轻量脚本。",
            "not_started",
        ));
    }
    let (root, entry) = crate::node::checked_entry(&action)?;
    Ok((
        crate::node::script_path(&root),
        crate::node::script_path(&entry),
    ))
}

fn create_project(root: &Path, id: &str, name: &str) -> Result<ScriptProject, ActionError> {
    if !valid_project_id(id) {
        return Err(fault("invalid_project_id", "工程标识需为 1–64 个英文字母、数字、- 或 _，以字母或数字开头，不能使用 Windows 设备名。", "not_started"));
    }
    let root = std::path::absolute(root)
        .map_err(|error| fault("project_io", error.to_string(), "not_started"))?;
    crate::catalog::check_local_name(&root)?;
    #[cfg(windows)]
    if let Some(std::path::Component::Prefix(prefix)) = root.components().next() {
        if !matches!(
            prefix.kind(),
            std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
        ) {
            return Err(fault(
                "nonlocal_path",
                "脚本工程需要普通本地目录。",
                "not_started",
            ));
        }
    }
    crate::catalog::check_ancestors(&root)?;
    crate::catalog::validate_new_package(&root, id)?;
    match fs::symlink_metadata(&root) {
        Ok(metadata) => {
            crate::catalog::check_local(&root, &metadata)?;
            if !metadata.is_dir() {
                return Err(fault(
                    "invalid_project_root",
                    "动作目录不是文件夹。",
                    "not_started",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&root)
                .map_err(|error| fault("project_io", error.to_string(), "unknown"))?;
        }
        Err(error) => return Err(fault("project_io", error.to_string(), "not_started")),
    }
    let package_root = root.join(id);
    fs::create_dir(&package_root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            fault(
                "project_exists",
                format!("目标已存在，不会覆盖：{}", package_root.display()),
                "not_started",
            )
        } else {
            fault("project_io", error.to_string(), "unknown")
        }
    })?;
    let call = ActionCall {
        package_id: id.into(),
        action_id: "run".into(),
        action_version: 1,
        input: json!({"text":"  OmniTrigger  "}),
    };
    let manifest = json!({"schema_version":1,"id":id,"name":name,"version":"1.0.0","actions":[{"id":"run","title":"运行脚本","version":1,"runtime":"script","entry":"main.mjs"}]});
    let config = json!({"compilerOptions":{"allowJs":true,"checkJs":true,"strict":true,"noEmit":true,"target":"ES2020","module":"ESNext","moduleResolution":"Bundler","lib":["ES2020"],"types":[]},"include":["**/*.js","**/*.mjs","sdk/**/*.d.ts"]});
    let write = |name: &str, contents: &str| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(package_root.join(name))?;
        file.write_all(contents.as_bytes())
    };
    let result = (|| -> std::io::Result<()> {
        fs::create_dir(package_root.join("sdk"))?;
        fs::create_dir(package_root.join("lib"))?;
        write("sdk/api.d.ts", SCRIPT_API)?;
        write("main.mjs", ENTRY)?;
        write("lib/text.js", MODULE)?;
        write("call.json", &serde_json::to_string_pretty(&call)?)?;
        write("jsconfig.json", &config.to_string())?;
        write("README.md", README)?;
        // 清单最后写入；未生成入口和 SDK 的目录不会提前成为可加载动作包。
        write("action.json", &manifest.to_string())
    })();
    result.map_err(|error| {
        fault(
            "project_io",
            format!(
                "创建工程失败，可能保留部分文件，请检查 {}：{error}",
                package_root.display()
            ),
            "unknown",
        )
    })?;
    Ok(ScriptProject {
        entry_path: package_root.join("main.mjs"),
        call_path: package_root.join("call.json"),
        sdk_path: package_root.join("sdk/api.d.ts"),
        package_root,
        call,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{catalog::tests::Fixture, PackageCatalog};

    #[test]
    fn named_projects_preserve_display_names_and_edit_without_a_runtime() {
        let fixture = Fixture::new();
        let root = fixture.0.join("actions");
        for name in ["", "   ", "bad\nname"] {
            assert_eq!(
                create_named_script_project(&root, name).unwrap_err().code,
                "invalid_project_name"
            );
        }
        assert!(!root.exists());
        let first = create_named_script_project(&root, "  整理选中文字  ").unwrap();
        let second = create_named_script_project(&root, "整理选中文字").unwrap();
        assert_ne!(first.call.package_id, second.call.package_id);
        assert_ne!(first.package_root, second.package_root);
        let catalog = PackageCatalog::load(&root);
        assert_eq!(catalog.actions.len(), 2);
        assert!(catalog
            .actions
            .iter()
            .all(|a| a.package_name == "整理选中文字"));
        let (folder, entry) = editable_script(&root, &first.call).unwrap();
        assert_eq!(
            fs::canonicalize(folder).unwrap(),
            fs::canonicalize(&first.package_root).unwrap()
        );
        assert_eq!(
            fs::canonicalize(entry).unwrap(),
            fs::canonicalize(&first.entry_path).unwrap()
        );
        fs::remove_file(&first.entry_path).unwrap();
        assert!(editable_script(&root, &first.call).is_err());
    }

    #[test]
    fn generated_project_resolves_and_existing_author_edits_survive() {
        let fixture = Fixture::new();
        let root = fixture.0.join("actions");
        let project = create_script_project(&root, "my-script").unwrap();
        let catalog = PackageCatalog::load(&root);
        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        let action = catalog.resolve(&project.call).unwrap();
        assert_eq!(action.definition.runtime, "script");
        assert_eq!(fs::read_to_string(&project.sdk_path).unwrap(), SCRIPT_API);
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(project.package_root.join("jsconfig.json")).unwrap())
                .unwrap();
        assert_eq!(config["compilerOptions"]["module"], "ESNext");
        assert_eq!(
            config["include"],
            json!(["**/*.js", "**/*.mjs", "sdk/**/*.d.ts"])
        );
        assert_eq!(
            fs::read_to_string(project.package_root.join("lib/text.js")).unwrap(),
            MODULE
        );
        assert_eq!(action.definition.entry, "main.mjs");
        fs::write(
            &project.entry_path,
            "throw new Error('这是作者自己的内容');",
        )
        .unwrap();
        let before = fs::read(&project.entry_path).unwrap();
        assert_eq!(
            create_script_project(&root, "my-script").unwrap_err().code,
            "project_exists"
        );
        assert_eq!(fs::read(&project.entry_path).unwrap(), before);
    }

    #[test]
    fn invalid_ids_cloud_names_and_duplicate_manifest_identity_do_not_create_files() {
        let fixture = Fixture::new();
        let root = fixture.0.join("actions");
        for id in [
            "",
            "../escape",
            "a.b",
            "a/b",
            "C:escape",
            "_bad",
            "con",
            "COM1",
            "lpt9",
            "name.",
        ] {
            assert_eq!(
                create_script_project(&root, id).unwrap_err().code,
                "invalid_project_id"
            );
            assert!(!root.exists());
        }
        let cloud = fixture.0.join("OneDrive-test");
        assert_eq!(
            create_script_project(&cloud, "script").unwrap_err().code,
            "nonlocal_path"
        );
        assert!(!cloud.exists());
        let project = create_script_project(&root, "existing").unwrap();
        let renamed = root.join("renamed-folder");
        assert!(project.package_root.starts_with(&fixture.0) && renamed.starts_with(&fixture.0));
        fs::rename(&project.package_root, &renamed).unwrap();
        assert_eq!(
            create_script_project(&root, "existing").unwrap_err().code,
            "project_exists"
        );
        assert!(!root.join("existing").exists());
    }

    #[test]
    fn full_catalog_reserves_capacity_before_creating_an_unloadable_project() {
        let fixture = Fixture::new();
        for index in 0..128 {
            let id = format!("package-{index}");
            fixture.package(&id, &id, "script");
        }
        assert_eq!(
            create_script_project(&fixture.0, "overflow")
                .unwrap_err()
                .code,
            "catalog_limit"
        );
        assert!(!fixture.0.join("overflow").exists());
    }
}
