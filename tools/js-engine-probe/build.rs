fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_BOA").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Boa 解析共享 SDK 的 Debug 路径需要超过 Windows 默认的 1 MiB 主栈。
        // 仅增加原生栈保留量；JS 循环、递归和外层动作期限保持原值。
        println!("cargo:rustc-link-arg-bin=omnitrigger-js-probe=/STACK:8388608");
    }
}
