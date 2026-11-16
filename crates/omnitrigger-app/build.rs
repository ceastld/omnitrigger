fn main() {
    let version = env!("CARGO_PKG_VERSION");
    println!("cargo:rerun-if-env-changed=OMNITRIGGER_BUILD_COMMIT");
    let product_version = match std::env::var_os("OMNITRIGGER_BUILD_COMMIT") {
        Some(value) => {
            let commit = value.to_str().expect("构建提交须为十六进制字符串");
            assert!(
                matches!(commit.len(), 40 | 64) && commit.bytes().all(|c| c.is_ascii_hexdigit()),
                "构建提交须为完整的 Git 对象 ID"
            );
            format!("{version} ({})", &commit[..12])
        }
        None => version.to_owned(),
    };
    let build_label = format!("OmniTrigger {product_version}");
    println!("cargo:rustc-env=OMNITRIGGER_BUILD_LABEL={build_label}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/app.rc");
        println!("cargo:rerun-if-changed=assets/omnitrigger.ico");
        let [major, minor, patch] = [
            env!("CARGO_PKG_VERSION_MAJOR"),
            env!("CARGO_PKG_VERSION_MINOR"),
            env!("CARGO_PKG_VERSION_PATCH"),
        ]
        .map(|part| {
            part.parse::<u16>()
                .expect("Windows 文件版本的每一段须在 0–65535 之间")
        });
        let defines = [
            format!("OMNI_VERSION_PARTS={major},{minor},{patch},0"),
            format!("OMNI_FILE_VERSION=\"{version}\""),
            format!("OMNI_PRODUCT_VERSION=\"{product_version}\""),
            format!("OMNI_TRAY_LABEL=\"{build_label}\""),
        ];
        let result = embed_resource::compile("assets/app.rc", defines);
        if let embed_resource::CompilationResult::NotAttempted(reason) = &result {
            println!("cargo:warning=未找到资源编译器，程序不含图标和文件版本：{reason}");
        }
        result.manifest_optional().expect("无法嵌入应用资源");
    }
}
