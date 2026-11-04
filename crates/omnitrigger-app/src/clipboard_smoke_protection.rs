//! 仅测试夹具：用短时核验记录防止旧数据根或旧二进制误入交互实测。
//! 这是操作防错约束，不是可信本机代码之间的安全边界。
use serde::Deserialize;
use std::os::windows::process::CommandExt;
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ci {
    run_id: u64,
    head_sha: String,
    conclusion: String,
    checks: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Protection {
    version: u32,
    scenario: String,
    expires_at_unix_ms: u64,
    writer_sha256: String,
    host_sha256: String,
    data_manifest_sha256: String,
    producer: Ci,
    consumer: Ci,
}
impl Ci {
    fn valid(&self, checks: &[&str]) -> bool {
        self.run_id != 0
            && self.head_sha.len() == 40
            && self.head_sha.bytes().all(|byte| byte.is_ascii_hexdigit())
            && self.conclusion == "success"
            && checks
                .iter()
                .all(|check| self.checks.iter().any(|seen| seen == check))
    }
}
impl Protection {
    fn validate(&self, scenario: &str, now: u64, hashes: &[String]) -> Result<(), String> {
        if self.version != 1
            || self.scenario != scenario
            || !matches!(scenario, "clipboard-text-execute" | "saved-text-execute")
            || self.expires_at_unix_ms <= now
            || self.expires_at_unix_ms > now.saturating_add(600_000)
            || !self
                .producer
                .valid(&["native-write", "native-owned-restore"])
            || !self
                .consumer
                .valid(&["wpf-net472", "wpf-net10", "actual-factory"])
        {
            return Err("受保护夹具的验证范围、期限或 CI 记录不完整；未开始交互操作".into());
        }
        let expected = [
            &self.writer_sha256,
            &self.host_sha256,
            &self.data_manifest_sha256,
        ];
        if hashes.len() != expected.len()
            || expected.iter().zip(hashes).any(|(expected, actual)| {
                expected.len() != 64
                    || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
                    || !expected.eq_ignore_ascii_case(actual)
            })
        {
            return Err("受保护夹具的二进制或数据清单已变化；未开始交互操作".into());
        }
        Ok(())
    }
}

pub(super) fn preflight(
    root: &Path,
    scenario: &str,
    writer: &Path,
    host: &Path,
) -> Result<(), String> {
    if std::fs::read(root.join("legacy-standalone-diagnostic.marker"))
        .map_err(|error| error.to_string())?
        != b"legacy-standalone-diagnostic-v1"
    {
        return Err("需要本次诊断数据根，未开始交互操作".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(root.join("protected-clipboard-validation.json"))
        .map_err(|_| "缺少受保护剪贴板的短时核验记录，未开始交互操作".to_owned())?
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 16 * 1024 {
        return Err("剪贴板核验记录过大".into());
    }
    let proof: Protection = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let hashes = file_hashes(&[writer, host, &root.join("actual-data-manifest.json")])?;
    let now = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis(),
    )
    .map_err(|_| "系统时间超出范围")?;
    proof.validate(scenario, now, &hashes)
}

fn file_hashes(paths: &[&Path]) -> Result<Vec<String>, String> {
    // 路径通过环境中的 JSON 数组传入，不拼成 PowerShell 命令；无原生输入或剪贴板调用。
    let script = r#"$ErrorActionPreference='Stop'; try {
      $taskPaths=ConvertFrom-Json $env:OMNITRIGGER_CLIPBOARD_TEST_HASH_PATHS
      $taskHasher=[System.Security.Cryptography.SHA256]::Create()
      try { $taskHashes=@(foreach($taskPath in $taskPaths) {
        $taskStream=[System.IO.File]::OpenRead($taskPath)
        try { ([System.BitConverter]::ToString($taskHasher.ComputeHash($taskStream))).Replace('-','').ToLowerInvariant() }
        finally { $taskStream.Dispose() }
      }); ConvertTo-Json -InputObject $taskHashes -Compress } finally { $taskHasher.Dispose() }
    } catch { [Console]::Error.WriteLine($_.Exception.GetType().FullName); exit 1 }"#;
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env(
            "OMNITRIGGER_CLIPBOARD_TEST_HASH_PATHS",
            serde_json::to_string(paths).map_err(|error| error.to_string())?,
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|error| error.to_string())?;
    let began = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
        if began.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("本次文件核验超时，已结束自有核验进程；未开始交互操作".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return Err("文件摘要核验失败；未开始交互操作".into());
    }
    serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Protection {
        serde_json::from_value(serde_json::json!({
      "version":1,"scenario":"clipboard-text-execute","expires_at_unix_ms":2000,
      "writer_sha256":"a".repeat(64),"host_sha256":"b".repeat(64),"data_manifest_sha256":"c".repeat(64),
      "producer":{"run_id":1,"head_sha":"d".repeat(40),"conclusion":"success","checks":["native-write","native-owned-restore"]},
      "consumer":{"run_id":2,"head_sha":"e".repeat(40),"conclusion":"success","checks":["wpf-net472","wpf-net10","actual-factory"]}
    })).unwrap()
    }
    #[test]
    fn protected_file_hash_preserves_literal_paths_without_shell_interpolation() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../.tmp/clipboard-hash-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("literal $(should-not-run).bin");
        std::fs::write(&path, b"abc").unwrap();
        let hashes = file_hashes(&[&path]).unwrap();
        assert_eq!(
            hashes,
            ["ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"]
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn protected_clipboard_requires_current_scope_hashes_and_both_native_boundaries() {
        let hashes = vec!["a".repeat(64), "b".repeat(64), "c".repeat(64)];
        assert!(fixture()
            .validate("clipboard-text-execute", 1000, &hashes)
            .is_ok());
        assert!(fixture()
            .validate("saved-text-execute", 1000, &hashes)
            .is_err());
        assert!(fixture()
            .validate("clipboard-text-execute", 2000, &hashes)
            .is_err());
        assert!(fixture()
            .validate(
                "clipboard-text-execute",
                1000,
                &["f".repeat(64), hashes[1].clone(), hashes[2].clone()]
            )
            .is_err());
        let mut proof = fixture();
        proof.consumer.checks.pop();
        assert!(proof
            .validate("clipboard-text-execute", 1000, &hashes)
            .is_err());
        let mut proof = fixture();
        proof.producer.conclusion = "failure".into();
        assert!(proof
            .validate("clipboard-text-execute", 1000, &hashes)
            .is_err());
    }
}
