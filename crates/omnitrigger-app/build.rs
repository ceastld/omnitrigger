fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/app.rc");
        println!("cargo:rerun-if-changed=assets/omnitrigger.ico");
        let result = embed_resource::compile("assets/app.rc", embed_resource::NONE);
        if let embed_resource::CompilationResult::NotAttempted(reason) = &result {
            println!("cargo:warning=未找到资源编译器，程序不含图标：{reason}");
        }
        result.manifest_optional().expect("无法嵌入应用图标资源");
    }
}
