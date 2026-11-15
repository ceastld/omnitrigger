use std::{path::Path, process::Command};

#[test]
fn action_inspection_leaves_test_registration_and_interrupted_reset_untouched() {
    let root = std::env::temp_dir().join(format!("omnitrigger-cli-inspect-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_omnitrigger"))
            .args(args)
            .env("OMNITRIGGER_HOME", &root)
            .output()
            .unwrap()
    };
    let call = root.join("request.json");
    std::fs::write(
        &call,
        r#"{"package_id":"examples.n10-rust","action_id":"toggle","action_version":1}"#,
    )
    .unwrap();
    let check = ["--check-action", call.to_str().unwrap()];
    let queries = [&["--list-package-actions"][..], &check[..]];
    for query in queries {
        assert!(!run(&[&["--test-workspace"][..], query].concat())
            .status
            .success());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }
    let created = run(&["--profile-create", "testing", "--seed", "empty"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let entry: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    let dir = Path::new(entry["directory"].as_str().unwrap());
    let config = dir.join("bindings.json");
    let identity = std::fs::read(dir.join("profile.json")).unwrap();
    let registration = std::fs::read(root.join("profiles.json")).unwrap();
    let listed = run(&["--test-workspace", "--list-package-actions"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let checked = run(&[&["--profile", "testing"][..], &check].concat());
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(response["handler"], "workspace_text_history");
    assert!(!dir.with_extension("window.lock").exists());

    let tag = format!("{}-12-t0123456789ab", entry["id"].as_str().unwrap());
    let backups = root.join("backups/profile-reset");
    let backup = backups.join(&tag);
    let stage = backups.join(format!(".{tag}.tmp"));
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("profile.json"), &identity).unwrap();
    std::fs::rename(dir, &backup).unwrap();
    for selector in [
        &["--profile", "testing"][..],
        &["--test-workspace"],
        &["--config", config.to_str().unwrap()],
    ] {
        for query in queries {
            let result = run(&[selector, query].concat());
            assert!(!result.status.success());
            let error: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
            assert!(error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("测试档目录缺失"));
            assert!(!dir.exists());
            assert!(!dir.with_extension("window.lock").exists());
            assert_eq!(
                std::fs::read(backup.join("profile.json")).unwrap(),
                identity
            );
            assert_eq!(std::fs::read(stage.join("profile.json")).unwrap(), identity);
            assert_eq!(
                std::fs::read(root.join("profiles.json")).unwrap(),
                registration
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_config_reads_bom_in_hotkeys_and_referenced_wheel_without_rewriting() {
    let root = std::env::temp_dir().join(format!("omnitrigger-cli-bom-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let bindings = root.join("bindings.json");
    let wheels = root.join("wheels.json");
    let binding = r#"{"version":24,"bindings":[{"id":1,"held":[{"kind":"key","value":20}],"activation":{"kind":"press","value":{"kind":"key","value":87}},"message":"轮盘","enabled":true,"action":{"kind":"show_wheel","wheel_id":"notes"}}]}"#;
    let wheel = r#"{"version":6,"wheels":[{"id":"notes","name":"笔记","slots":[{"label":"提示","action":{"kind":"notification"}}]}]}"#;
    let check = || {
        Command::new(env!("CARGO_BIN_EXE_omnitrigger"))
            .arg("--data-root")
            .arg(&root)
            .arg("--check-config")
            .output()
            .unwrap()
    };
    for (hotkey_prefix, wheel_prefix) in [
        ("", ""),
        ("\u{feff}", ""),
        ("", "\u{feff}"),
        ("\u{feff}", "\u{feff}"),
    ] {
        let hotkeys = format!("{hotkey_prefix}{binding}");
        let layout = format!("{wheel_prefix}{wheel}");
        std::fs::write(&bindings, &hotkeys).unwrap();
        std::fs::write(&wheels, &layout).unwrap();
        let result = check();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("共 1 条绑定"));
        assert_eq!(std::fs::read(&bindings).unwrap(), hotkeys.as_bytes());
        assert_eq!(std::fs::read(&wheels).unwrap(), layout.as_bytes());
    }
    for invalid in [format!("\u{feff}\u{feff}{binding}"), "\u{feff}{".into()] {
        std::fs::write(&bindings, &invalid).unwrap();
        assert!(!check().status.success());
        assert_eq!(std::fs::read(&bindings).unwrap(), invalid.as_bytes());
    }
    std::fs::remove_dir_all(root).unwrap();
}

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
