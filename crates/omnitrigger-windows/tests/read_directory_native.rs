//! 显式指定现有本地测试父目录；只创建及删除其中唯一的新建子目录。
//! 默认 ignored；未配置时失败，不能静默略过并计作 Native 通过。
#![cfg(windows)]
use omnitrigger_windows::{read_local_directory, stat_local_path, LocalPathKind};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let parent = PathBuf::from(
            std::env::var_os("OMNITRIGGER_DIRECTORY_NATIVE_ROOT").expect("需要显式本地测试父目录"),
        );
        let text = parent.to_str().expect("需要有效 Unicode 路径");
        assert_eq!(
            stat_local_path(text, || Ok(())).unwrap(),
            LocalPathKind::Directory
        );
        let root = parent.join(format!(
            "directory-native-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }
    fn path(&self) -> &str {
        self.root.to_str().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // 不递归清理；只移除已知文件/空目录，异常额外项保留供排查。
        for name in ["a.txt", "z.txt"] {
            let _ = fs::remove_file(self.root.join(name));
        }
        for index in 0..100 {
            let _ = fs::remove_file(
                self.root
                    .join(format!("{index:03}{}.txt", "中".repeat(100))),
            );
        }
        for name in ["child", "OneDrive-fixture"] {
            let _ = fs::remove_dir(self.root.join(name));
        }
        let _ = fs::remove_dir(&self.root);
    }
}
#[test]
#[ignore = "Windows 真目录元数据大小实测；需显式 OMNITRIGGER_DIRECTORY_NATIVE_ROOT"]
fn read_directory_real_windows_rejects_total_metadata_budget_without_truncating() {
    let f = Fixture::new();
    for index in 0..100 {
        fs::write(
            f.root.join(format!("{index:03}{}.txt", "中".repeat(100))),
            b"synthetic",
        )
        .unwrap();
    }
    assert_eq!(
        read_local_directory(f.path(), 1024, || Ok(()))
            .unwrap_err()
            .code,
        "directory_too_large"
    );
}
#[test]
#[ignore = "Windows 真目录句柄实测；需显式 OMNITRIGGER_DIRECTORY_NATIVE_ROOT"]
fn read_directory_real_windows_empty_direct_children_and_count_limit() {
    let f = Fixture::new();
    assert!(read_local_directory(f.path(), 128, || Ok(()))
        .unwrap()
        .is_empty());
    fs::write(f.root.join("z.txt"), b"synthetic").unwrap();
    fs::write(f.root.join("a.txt"), b"synthetic").unwrap();
    fs::create_dir(f.root.join("child")).unwrap();
    fs::create_dir(f.root.join("OneDrive-fixture")).unwrap();
    let entries = read_local_directory(f.path(), 4, || Ok(())).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["OneDrive-fixture", "a.txt", "child", "z.txt"]
    );
    assert_eq!(entries[0].reason, Some("cloud_path"));
    assert_eq!(entries[0].kind, "unavailable");
    assert_eq!(entries[2].kind, "directory");
    assert_eq!(
        read_local_directory(f.path(), 3, || Ok(()))
            .unwrap_err()
            .code,
        "directory_too_large"
    );
    assert_eq!(
        read_local_directory(f.root.join("a.txt").to_str().unwrap(), 4, || Ok(()))
            .unwrap_err()
            .code,
        "not_directory"
    );
    assert_eq!(
        read_local_directory(f.root.join("missing").to_str().unwrap(), 4, || Ok(()))
            .unwrap_err()
            .code,
        "path_missing"
    );
}
#[test]
#[ignore = "Windows 目录链接实测；需 Developer Mode 或创建 symlink 权限，缺少权限应失败"]
fn read_directory_real_windows_does_not_follow_child_or_root_link() {
    let f = Fixture::new();
    fs::create_dir(f.root.join("child")).unwrap();
    let link = f.root.join("link");
    std::os::windows::fs::symlink_dir(f.root.join("child"), &link)
        .expect("链接测试需要创建符号链接权限");
    let entries = read_local_directory(f.path(), 4, || Ok(())).unwrap();
    let entry = entries.iter().find(|entry| entry.name == "link").unwrap();
    assert_eq!(entry.kind, "unavailable");
    assert_eq!(entry.reason, Some("reparse_point"));
    assert_eq!(
        read_local_directory(link.to_str().unwrap(), 4, || Ok(()))
            .unwrap_err()
            .code,
        "path_unavailable"
    );
    fs::remove_dir(link).unwrap();
}
