use std::{path::Path, process::Command};

#[test]
fn restored_test_root_routes_copied_clipboard_to_its_own_history() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_omnitrigger"))
            .args(args)
            .env_remove("OMNITRIGGER_HOME")
            .env("LOCALAPPDATA", root)
            .output()
            .unwrap()
    };
    let created = run(&[
        "--data-root",
        root.to_str().unwrap(),
        "--profile-create",
        "notes",
        "--name",
        "笔记测试",
    ]);
    assert!(created.status.success());
    let entry: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    let identity = Path::new(entry["directory"].as_str().unwrap()).join("profile.json");
    let restored = root.join("OmniTrigger");
    std::fs::create_dir(&restored).unwrap();
    std::fs::copy(identity, restored.join("profile.json")).unwrap();
    let call = root.join("toggle.json");
    std::fs::write(
        &call,
        r#"{"package_id":"examples.n10-rust","action_id":"toggle","action_version":1}"#,
    )
    .unwrap();
    let directory = restored.to_str().unwrap();
    let config = restored.join("bindings.json");
    for selection in [
        &[][..],
        &["--data-root", directory],
        &["--config", config.to_str().unwrap()],
    ] {
        let output = run(&[selection, &["--check-action", call.to_str().unwrap()]].concat());
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let checked: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(checked["handler"], "workspace_text_history");
    }
    for args in [
        &["--profile-list"][..],
        &["--data-root", directory, "--profile-list"],
    ] {
        let listed = run(args);
        assert!(
            listed.status.success(),
            "{}",
            String::from_utf8_lossy(&listed.stderr)
        );
        let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
        assert_eq!(listed["profiles"][0]["kind"], "test");
        assert_eq!(listed["profiles"][0]["name"], "笔记测试");
    }
    assert_eq!(std::fs::read_dir(&restored).unwrap().count(), 1);
    // 即使解压目录恰好位于 profiles 下，也不能把未登记的测试数据交给日常动作。
    let other = root.join("other");
    std::fs::create_dir_all(other.join("profiles")).unwrap();
    std::fs::write(
        other.join("profiles.json"),
        r#"{"version":1,"profiles":[]}"#,
    )
    .unwrap();
    let beside = other.join("profiles").join(entry["id"].as_str().unwrap());
    std::fs::rename(&restored, &beside).unwrap();
    let checked = run(&[
        "--data-root",
        beside.to_str().unwrap(),
        "--check-action",
        call.to_str().unwrap(),
    ]);
    let checked: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(checked["handler"], "workspace_text_history");
}

#[cfg(windows)]
#[test]
fn browser_action_reads_a_bom_prefixed_file_in_the_selected_profile() {
    let root = std::env::temp_dir().join(format!("omnitrigger-browser-cli-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let action = root.join("query.json");
    let json =
        "\u{feff}{\"command\":{\"operation\":\"call\",\"method\":\"tabs.query\",\"args\":[{}]}}";
    std::fs::write(&action, json).unwrap();
    let registration = root.join("browser-connector.json");
    let saved = "\u{feff}{\"version\":\"unknown\"}";
    std::fs::write(&registration, saved).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_omnitrigger"))
        .arg("--config")
        .arg(root.join("bindings.json"))
        .arg("--browser-action")
        .arg(&action)
        .env("LOCALAPPDATA", &root)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(error["code"], "version_incompatible");
    assert_eq!(std::fs::read(&action).unwrap(), json.as_bytes());
    assert_eq!(std::fs::read(&registration).unwrap(), saved.as_bytes());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn relayed_catalog_uses_caller_runtime_choices_and_profile_managed_host() {
    let root =
        std::env::temp_dir().join(format!("omnitrigger-cli-runtimes-{}", std::process::id()));
    let package = root.join("actions/demo");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("main.js"), "").unwrap();
    let manifest = serde_json::json!({
        "schema_version": 1, "id": "demo", "name": "测试", "version": "1.0.0",
        "actions": [
            {"id":"one","title":"一","version":1,"runtime":"node","entry":"main.js"},
            {"id":"two","title":"二","version":1,"runtime":"script","entry":"main.js"}
        ]
    });
    std::fs::write(package.join("action.json"), manifest.to_string()).unwrap();
    let runtime =
        root.join("runtimes/script")
            .join(if cfg!(windows) { "host.exe" } else { "host" });
    std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
    std::fs::write(&runtime, "查询夹具").unwrap();
    let receipt = root.join("relaunch.json");
    let link = receipt.with_extension("lnk");
    for (selection, node, script) in [
        (
            serde_json::json!({"node":null,"script":{"Err":"调用方宿主不可用"}}),
            false,
            false,
        ),
        (
            serde_json::json!({"node":runtime,"script":{"Ok":{"program":runtime,"integrity":null}}}),
            true,
            true,
        ),
        (serde_json::json!({"node":null,"script":null}), false, true),
    ] {
        std::fs::write(&receipt, selection.to_string()).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_omnitrigger"))
            .arg("--data-root")
            .arg(&root)
            .arg("--relaunched")
            .arg(&link)
            .arg("--relaunch-result")
            .arg(&receipt)
            .arg("--list-package-actions")
            .env("OMNITRIGGER_NODE", &runtime)
            .env("OMNITRIGGER_SCRIPT_HOST", root.join("missing.exe"))
            .output()
            .unwrap();
        let response: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        assert!(result.status.success(), "{response}");
        let report: serde_json::Value =
            serde_json::from_str(response["text"].as_str().unwrap()).unwrap();
        let actions = report["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        for (id, available) in [("one", node), ("two", script)] {
            let action = actions
                .iter()
                .find(|item| item["call"]["action_id"] == id)
                .unwrap();
            assert_eq!(action["reference_error"].is_null(), available);
            if !available {
                assert_eq!(action["reference_error"]["code"], "runtime_unavailable");
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn relayed_action_queries_match_direct_catalog_validation_and_errors() {
    let root =
        std::env::temp_dir().join(format!("omnitrigger-query-result-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let receipt = root.join("result.json");
    let run = |args: &[&str], relay: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_omnitrigger"));
        command.arg("--data-root").arg(&root).args(args);
        if relay {
            command.arg("--relaunch-result").arg(&receipt);
        }
        command.output().unwrap()
    };
    assert!(
        run(&["--profile-create", "testing", "--seed", "empty"], false)
            .status
            .success()
    );
    let request = root.join("调用 1.json");
    std::fs::write(
        &request,
        r#"{"package_id":"examples.n10-rust","action_id":"toggle","action_version":1}"#,
    )
    .unwrap();
    for (args, success) in [
        (&["--list-package-actions"][..], true),
        (
            &[
                "--profile",
                "testing",
                "--check-action",
                request.to_str().unwrap(),
            ],
            true,
        ),
        (&["--check-action", request.to_str().unwrap()], false),
        (&["--profile", "absent", "--list-package-actions"], false),
    ] {
        let direct = run(args, false);
        assert_eq!(direct.status.success(), success);
        let relayed = run(args, true);
        assert_eq!(relayed.status.success(), success);
        assert!(relayed.stdout.is_empty() && relayed.stderr.is_empty());
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        assert_eq!(result["ok"], success);
        let expected = if success {
            direct.stdout
        } else {
            direct.stderr
        };
        let expected: serde_json::Value = serde_json::from_slice(&expected).unwrap();
        let actual: serde_json::Value =
            serde_json::from_str(result["text"].as_str().unwrap()).unwrap();
        assert_eq!(actual, expected);
        std::fs::remove_file(&receipt).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tray_rejects_test_profiles_before_recovering_or_locking_them() {
    let root = std::env::temp_dir().join(format!(
        "omnitrigger-cli-tray-profile-{}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    let run = |home: &Path, args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_omnitrigger"))
            .args(args)
            .env("OMNITRIGGER_HOME", home)
            .output()
            .unwrap()
    };
    let created = run(&root, &["--profile-create", "testing"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let entry: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    let id = entry["id"].as_str().unwrap();
    let dir = Path::new(entry["directory"].as_str().unwrap());
    let config = dir.join("bindings.json");
    let registry = root.join("profiles.json");
    let registration = std::fs::read(&registry).unwrap();
    let identity = std::fs::read(dir.join("profile.json")).unwrap();
    let tag = format!("{id}-12-t0123456789ab");
    let backups = root.join("backups/profile-reset");
    let backup = backups.join(&tag);
    let stage = backups.join(format!(".{tag}.tmp"));
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("profile.json"), &identity).unwrap();
    std::fs::write(&config, b"my hotkeys").unwrap();
    std::fs::rename(dir, &backup).unwrap();

    for registered in [true, false] {
        if !registered {
            std::fs::remove_file(&registry).unwrap();
        }
        for (home, args) in [
            (root.as_path(), &["--profile", "testing", "--tray"][..]),
            (&root, &["--profile", id, "--tray"]),
            (&root, &["--config", config.to_str().unwrap(), "--tray"]),
            (&root, &["--data-root", dir.to_str().unwrap(), "--tray"]),
            (dir, &["--tray"]),
            (dir, &["--profile", "default", "--tray"]),
        ] {
            let output = run(home, args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert!(output.stdout.is_empty());
            let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(error["error"]["code"], "invalid_cli_arguments");
            assert_eq!(error["error"]["effect_state"], "not_started");
            assert!(!dir.exists() && !dir.with_extension("window.lock").exists());
            assert_eq!(
                std::fs::read(backup.join("bindings.json")).unwrap(),
                b"my hotkeys"
            );
            assert_eq!(std::fs::read(stage.join("profile.json")).unwrap(), identity);
            assert_eq!(
                std::fs::read(&registry).ok().as_ref(),
                registered.then_some(&registration)
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

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
    let absent = root.join("not-created");
    let list = run(&["--data-root", absent.to_str().unwrap(), "--profile-list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    assert!(!absent.exists());
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
    let children = run(&["--data-root", dir.to_str().unwrap(), "--profile-list"]);
    assert!(children.status.success());
    assert!(!dir.with_extension("window.lock").exists());
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
    let registry = root.join("profiles.json");
    std::fs::remove_file(&registry).unwrap();
    let list = run(&["--profile-list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let testing = list["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["alias"] == "testing")
        .unwrap();
    assert_eq!(testing["id"], entry["id"]);
    assert!(!registry.exists() && !dir.exists());
    for args in [
        &[
            "--config",
            config.to_str().unwrap(),
            "--list-package-actions",
        ][..],
        &["--test-workspace", "--check-config"],
        &["--data-root", dir.to_str().unwrap(), "--profile-list"],
    ] {
        let query = run(args);
        assert!(!query.status.success());
        assert!(String::from_utf8_lossy(&query.stderr).contains("测试档目录缺失"));
        assert!(!registry.exists() && !dir.exists());
        assert!(!dir.with_extension("window.lock").exists());
        assert_eq!(
            std::fs::read(backup.join("profile.json")).unwrap(),
            identity
        );
        assert_eq!(std::fs::read(stage.join("profile.json")).unwrap(), identity);
    }
    let copy = run(&["--profile-create", "copy", "--seed", "copy:testing"]);
    assert!(
        copy.status.success(),
        "{}",
        String::from_utf8_lossy(&copy.stderr)
    );
    assert_eq!(std::fs::read(dir.join("profile.json")).unwrap(), identity);
    assert!(registry.is_file() && !backup.exists());
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
fn check_config_reports_a_save_instead_of_an_incomplete_wheel_reference() {
    let root = std::env::temp_dir().join(format!("omnitrigger-cli-pair-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let bindings = root.join("bindings.json");
    let wheels = root.join("wheels.json");
    let receipt = root.join("result.json");
    let binding = r#"{"version":24,"bindings":[{"id":1,"held":[{"kind":"key","value":20}],"activation":{"kind":"press","value":{"kind":"key","value":87}},"message":"轮盘","enabled":true,"action":{"kind":"show_wheel","wheel_id":"notes"}}]}"#;
    let layout = r#"{"version":6,"wheels":[{"id":"notes","name":"笔记","slots":[{"label":"提示","action":{"kind":"notification"}}]}]}"#;
    std::fs::write(&bindings, binding).unwrap();
    std::fs::write(&wheels, layout).unwrap();
    let check = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_omnitrigger"))
            .arg("--check-config")
            .args(args)
            .env("OMNITRIGGER_HOME", &root)
            .output()
            .unwrap()
    };
    let explicit = ["--config", bindings.to_str().unwrap()];
    assert!(check(&explicit).status.success());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
    std::fs::write(&bindings, binding.replace("notes", "updated")).unwrap();
    for (file, kind) in [("bindings.json.lock", "热键"), ("wheels.json.lock", "轮盘")] {
        let lock = std::fs::File::create(root.join(file)).unwrap();
        lock.lock().unwrap();
        let result = check(&explicit);
        assert!(!result.status.success() && result.stdout.is_empty());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            error.contains(kind) && error.contains("稍后重试"),
            "{error}"
        );
        let relayed = check(&[
            "--data-root",
            root.to_str().unwrap(),
            "--relaunch-result",
            receipt.to_str().unwrap(),
        ]);
        assert!(
            !relayed.status.success() && relayed.stdout.is_empty() && relayed.stderr.is_empty()
        );
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        assert_eq!(report["ok"], false);
        assert_eq!(report["text"].as_str().unwrap(), error.trim_end());
        std::fs::remove_file(&receipt).unwrap();
    }
    let incomplete = check(&explicit);
    assert!(!incomplete.status.success());
    assert!(String::from_utf8_lossy(&incomplete.stderr).contains("不存在"));
    let updated = layout.replace("notes", "updated");
    std::fs::write(&wheels, &updated).unwrap();
    assert!(check(&explicit).status.success());
    assert_eq!(std::fs::read_to_string(wheels).unwrap(), updated);
    assert_eq!(
        std::fs::read_to_string(bindings).unwrap(),
        binding.replace("notes", "updated")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_config_rejects_ambiguous_input_before_creating_a_workspace() {
    let root =
        std::env::temp_dir().join(format!("omnitrigger-cli-check-args-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("鼠标.json"),
        include_bytes!("../../../examples/mouse-tab-navigation.json"),
    )
    .unwrap();
    std::fs::write(root.join("损坏.json"), "{").unwrap();
    std::fs::write(
        root.join("queue.json"),
        r#"{"bindings":[1],"policy":{"kind":"drop"}}"#,
    )
    .unwrap();
    let unused = root.join("未创建的测试根");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_omnitrigger"))
            .args(args)
            .current_dir(&root)
            .env("OMNITRIGGER_HOME", &unused)
            .env("LOCALAPPDATA", root.join("local"))
            .output()
            .unwrap()
    };

    for args in [
        vec![
            "--config",
            "鼠标.json",
            "--config",
            "损坏.json",
            "--check-config",
        ],
        vec![
            "--check-config",
            "--config",
            "损坏.json",
            "--config",
            "鼠标.json",
        ],
        vec!["--test-workspace", "--check-config", "--settings"],
        vec!["--check-config", "--check-config"],
    ] {
        let result = run(&args);
        assert_eq!(result.status.code(), Some(1), "{args:?}");
        let receipt: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
        assert_eq!(receipt["error"]["code"], "invalid_cli_arguments");
        assert_eq!(receipt["error"]["effect_state"], "not_started");
        assert!(result.stdout.is_empty() && !unused.exists());
    }
    let result = run(&[
        "--config",
        "鼠标.json",
        "--mouse-shortcut-queue",
        "queue.json",
        "--check-config",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("共 3 条绑定"));
    assert!(!unused.exists());
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
    for missing in [isolated.join("旧版热键.json"), unused.join("bindings.json")] {
        let result = run(&["--config", missing.to_str().unwrap()], &isolated);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8_lossy(&result.stderr).contains(missing.to_str().unwrap()));
        assert!(!missing.exists() && !unused.exists());
    }
    let explicit = run(&["--config", selected.to_str().unwrap()], &default);
    assert!(explicit.status.success());
    assert!(String::from_utf8_lossy(&explicit.stdout).contains("共 0 条绑定"));

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
