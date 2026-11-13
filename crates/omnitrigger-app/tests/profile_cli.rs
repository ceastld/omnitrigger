use std::{path::Path, process::Command};

#[test]
fn check_config_uses_the_selected_data_root() {
    let root = std::env::temp_dir().join(format!("omnitrigger-cli-root-{}", std::process::id()));
    let default = root.join("local").join("OmniTrigger");
    let isolated = root.join("日常配置之外");
    for dir in [&default, &isolated] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let selected = isolated.join("bindings.json");
    std::fs::write(&selected, r#"{"version":24,"bindings":[]}"#).unwrap();
    let invalid = default.join("bindings.json");
    std::fs::write(&invalid, "invalid config").unwrap();
    let run = |args: &[&str], env_root: &Path| {
        Command::new(env!("CARGO_BIN_EXE_omnitrigger"))
            .arg("--check-config")
            .args(args)
            .env("LOCALAPPDATA", root.join("local"))
            .env("OMNITRIGGER_HOME", env_root)
            .output()
            .unwrap()
    };
    let result = run(&[], &isolated);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains(selected.to_str().unwrap()));
    assert!(!run(&[], Path::new("")).status.success());
    assert!(!run(&["--config", invalid.to_str().unwrap()], &isolated)
        .status
        .success());
    assert!(run(&["--data-root", isolated.to_str().unwrap()], &default)
        .status
        .success());
    let unused = root.join("未创建的配置目录");
    assert!(run(&[], &unused).status.success());
    assert!(!unused.exists());

    // 外壳重启的子进程通过同一回执转交成功文本、配置格式错误和配置档错误。
    let receipt = root.join("check-result.json");
    let relayed = |env_root, profile: &[&str]| {
        let mut args = vec!["--relaunch-result", receipt.to_str().unwrap()];
        args.extend_from_slice(profile);
        let result = run(&args, env_root);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        assert_eq!(value["ok"], result.status.success());
        assert!(result.stdout.is_empty() && result.stderr.is_empty());
        std::fs::remove_file(&receipt).unwrap();
        value["text"].as_str().unwrap().to_owned()
    };
    assert!(relayed(&isolated, &[]).contains("共 0 条绑定"));
    assert!(relayed(&default, &[]).contains("配置格式有误"));
    assert!(relayed(&isolated, &["--profile", "absent"]).contains("找不到配置档"));
    std::fs::remove_dir_all(root).unwrap();
}
