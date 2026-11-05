use super::*;
use std::{
    cell::Cell,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

fn later() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn assert_error(error: ActionError, code: &str) {
    assert_eq!(error.code, code);
    assert_eq!(error.method.as_deref(), Some("fs.stat"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
}

#[test]
fn fs_stat_rejects_wrong_params_without_calling_the_platform() {
    for params in [
        json!(null),
        json!([]),
        json!("C:\\file"),
        json!({}),
        json!({"path":null}),
        json!({"path":true}),
        json!({"path":42}),
        json!({"path":["C:\\file"]}),
        json!({"path":"C:\\file","extra":true}),
    ] {
        let error = stat_with(params, &CancellationToken::new(), later(), |_, _| {
            panic!("无效参数不得查询磁盘")
        })
        .unwrap_err();
        assert_error(error, "invalid_input");
    }
    for source in [r#"{"path":"C:\\a\ud800"}"#, r#"{"path":"C:\\a\udc00"}"#] {
        assert!(serde_json::from_str::<Value>(source).is_err());
    }
}

#[test]
fn fs_stat_rejects_ambiguous_windows_names_and_non_absolute_paths() {
    let mut paths: Vec<String> = [
        "",
        "file",
        "C:",
        "C:file",
        "\\file",
        "/file",
        "1:\\file",
        "中:\\file",
        "\\\\server\\share\\file",
        "//server/share/file",
        "\\\\?\\C:\\file",
        "\\\\.\\C:\\file",
        "C:\\\\file",
        "C:\\a\\\\file",
        "C:/a//file",
        "C:/a/\\file",
        "C:/a//",
        "C:\\\\",
        "C:/./file",
        "C:/../file",
        "C:/a/.",
        "C:/a/..",
        "C:/a ",
        "C:/a.",
        "C:/file:stream",
        "C:/OneDrive/file:stream",
        "C:/a*b",
        "C:/a?b",
        "C:/a<b",
        "C:/a>b",
        "C:/a|b",
        "C:/a\"b",
        "C:/CON",
        "C:/prn.txt",
        "C:/AUX.log",
        "C:/NuL",
        "C:/COM1",
        "C:/COM9.txt",
        "C:/LPT1.x",
        "C:/lpt9",
        "C:/CON .txt",
        "C:/a\0b",
        "C:/a\nb",
        "C:/a\tb",
        "C:/a\u{85}b",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    paths.push(format!("C:/{}", "x".repeat(32765)));
    paths.push(format!("C:/{}", "😀".repeat(16383)));
    for path in paths {
        assert_error(
            stat_with(
                json!({"path":path}),
                &CancellationToken::new(),
                later(),
                |_, _| panic!("词法无效路径不得查询磁盘"),
            )
            .unwrap_err(),
            "invalid_input",
        );
    }
}

#[test]
fn fs_stat_rejects_superscript_devices_and_extensions_in_every_component() {
    for prefix in ["COM", "LPT", "com", "lpt"] {
        for digit in ['¹', '²', '³'] {
            for extension in ["", ".txt", " .txt", "   .tar.gz"] {
                let name = format!("{prefix}{digit}{extension}");
                for path in [
                    format!("C:/safe/{name}"),
                    format!("C:/safe/{name}/file.txt"),
                ] {
                    let error = stat_with(
                        json!({"path":path}),
                        &CancellationToken::new(),
                        later(),
                        |_, _| panic!("Unicode 设备名不得查询磁盘"),
                    )
                    .unwrap_err();
                    assert_error(error, "invalid_input");
                }
            }
        }
    }
    for path in ["C:/COM¹0", "C:/LPT²note", "C:/XCOM³", "C:/COM⁴"] {
        let output = stat_with(
            json!({"path":path}),
            &CancellationToken::new(),
            later(),
            |_, _| Ok(LocalPathKind::Missing),
        )
        .unwrap();
        assert_eq!(output, json!({"kind":"missing"}));
    }
}

#[test]
fn fs_stat_cloud_components_are_unavailable_and_never_reach_disk() {
    for path in [
        "C:/OneDrive/file",
        "C:/safe/ONEDRIVE-Personal/file",
        "C:/iCloudDrive/file",
        "C:/ICLOUD/file",
        "C:/Dropbox/file",
        "C:/safe/dropbox",
    ] {
        let error = stat_with(
            json!({"path":path}),
            &CancellationToken::new(),
            later(),
            |_, _| panic!("云盘组件不得查询磁盘"),
        )
        .unwrap_err();
        assert_error(error, "path_unavailable");
    }
}

#[test]
fn fs_stat_valid_paths_keep_original_spelling_and_boundary_utf16_units() {
    let mut paths: Vec<String> = [
        "C:\\",
        "c:/",
        "D:\\source\\file.txt",
        "C:/目录/😀.txt",
        "C:/safe\\mixed/file",
        "C:/safe/",
        "C:\\safe\\",
        "C:/safe/.hidden",
        "C:/CONSOLE",
        "C:/COM10",
        "C:/LPT0",
        "C:/myDropbox/file",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    paths.push(format!("C:/{}", "x".repeat(32764)));
    paths.push(format!("C:/{}", "😀".repeat(16382)));
    for path in paths {
        let output = stat_with(
            json!({"path":path}),
            &CancellationToken::new(),
            later(),
            |actual, check| {
                assert_eq!(actual, path);
                assert!(check().is_ok());
                Ok(LocalPathKind::Missing)
            },
        )
        .unwrap();
        assert_eq!(output, json!({"kind":"missing"}));
    }
}

#[test]
fn fs_stat_maps_every_kind_without_collapsing_platform_errors_to_missing() {
    for (kind, expected) in [
        (LocalPathKind::File, "file"),
        (LocalPathKind::Directory, "directory"),
        (LocalPathKind::Missing, "missing"),
        (LocalPathKind::Other, "other"),
    ] {
        let output = stat_with(
            json!({"path":"C:/safe/file"}),
            &CancellationToken::new(),
            later(),
            |_, check| {
                assert!(check().is_ok());
                Ok(kind)
            },
        )
        .unwrap();
        assert_eq!(output, json!({"kind":expected}));
    }
    let private = "C:/private-example/credential-placeholder";
    for code in [
        "invalid_input",
        "path_not_local",
        "path_unavailable",
        "access_denied",
        "fs_error",
        "fs_unavailable",
        "cancelled",
        "timeout",
        "unrecognized_platform_error",
    ] {
        let error = stat_with(
            json!({"path":"C:/safe/file"}),
            &CancellationToken::new(),
            later(),
            |_, _| {
                Err(LocalStatError {
                    code,
                    message: private.into(),
                })
            },
        )
        .unwrap_err();
        assert!(!error.message.contains(private));
        assert_error(
            error,
            if code == "unrecognized_platform_error" {
                "fs_error"
            } else {
                code
            },
        );
    }
}

#[test]
fn fs_stat_checks_cancellation_and_deadline_before_the_first_metadata_call() {
    let token = CancellationToken::new();
    token.cancel();
    assert_error(
        stat_with(json!({"path":"C:/safe"}), &token, later(), |_, _| {
            panic!("取消后不得查询磁盘")
        })
        .unwrap_err(),
        "cancelled",
    );
    assert_error(
        stat_with(
            json!({"path":"C:/safe"}),
            &CancellationToken::new(),
            Instant::now(),
            |_, _| panic!("超时后不得查询磁盘"),
        )
        .unwrap_err(),
        "timeout",
    );
}

#[test]
fn fs_stat_callback_observes_cancellation_between_metadata_steps_and_after_query() {
    let token = CancellationToken::new();
    let steps = Cell::new(0);
    let error = stat_with(
        json!({"path":"C:/safe/file"}),
        &token,
        later(),
        |_, check| {
            assert!(check().is_ok());
            steps.set(steps.get() + 1);
            token.cancel();
            assert_eq!(check().unwrap_err().code, "cancelled");
            // 即使平台误返回了结果，外层最后一次检查仍会拒绝迟到成功。
            Ok(LocalPathKind::File)
        },
    )
    .unwrap_err();
    assert_error(error, "cancelled");
    assert_eq!(steps.get(), 1);
    assert_eq!(
        check(&CancellationToken::new(), Instant::now())
            .unwrap_err()
            .code,
        "timeout"
    );
}

#[test]
fn fs_stat_rejects_a_late_os_result_without_claiming_to_preempt_the_os_call() {
    let deadline = Instant::now() + Duration::from_millis(10);
    let error = stat_with(
        json!({"path":"C:/safe/file"}),
        &CancellationToken::new(),
        deadline,
        |_, check| {
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            );
            assert_eq!(check().unwrap_err().code, "timeout");
            Ok(LocalPathKind::Missing)
        },
    )
    .unwrap_err();
    assert_error(error, "timeout");
}

struct Fixture {
    root: PathBuf,
    file: PathBuf,
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let root = loop {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                ".script-fs-stat-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("无法创建本测试临时目录：{error}"),
            }
        };
        let file = root.join("元数据 😀.txt");
        let directory = root.join("empty-directory");
        let fixture = Self {
            root,
            file,
            directory,
        };
        std::fs::write(&fixture.file, b"").unwrap();
        std::fs::create_dir(&fixture.directory).unwrap();
        fixture
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // 只删除本测试明确创建的文件和空目录，不递归清理或跟随链接。
        let _ = std::fs::remove_file(&self.file);
        let _ = std::fs::remove_dir(&self.directory);
        let _ = std::fs::remove_dir(&self.root);
    }
}

#[test]
fn fs_stat_real_host_needs_neither_ui_initialization_nor_a_desktop_target() {
    use omnitrigger_actions::NativeHost;
    struct NoDesktop;
    impl NativeHost for NoDesktop {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            panic!("fs.stat 不得转发给桌面目标或要求 UI")
        }
    }
    let fixture = Fixture::new();
    let host = crate::script_ui::Host::new(NoDesktop, None);
    for (path, expected) in [
        (&fixture.file, "file"),
        (&fixture.directory, "directory"),
        (&fixture.root.join("missing"), "missing"),
        (&fixture.root.join("missing/child"), "missing"),
    ] {
        let result = host
            .call_with_deadline(
                "fs.stat",
                json!({"path":path.to_str().unwrap()}),
                &CancellationToken::new(),
                later(),
            )
            .unwrap();
        assert_eq!(result, json!({"kind":expected}));
    }
    assert_error(
        host.call_with_deadline(
            "fs.stat",
            json!({"path":"C:/safe","extra":1}),
            &CancellationToken::new(),
            later(),
        )
        .unwrap_err(),
        "invalid_input",
    );
}
