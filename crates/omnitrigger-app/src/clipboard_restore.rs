//! 交互测试恢复剪贴板时也要排除采集；序号检查只防止已观察到的更新。

pub(super) const EXCLUSION_FORMATS: [&str; 2] = [
    "OmniTrigger.TransientClipboard.v1",
    "ExcludeClipboardContentFromMonitorProcessing",
];

pub(super) trait Backend {
    type Memory;
    fn register(&self, name: &str) -> Result<u32, String>;
    fn allocate(&self, bytes: &[u8]) -> Result<Self::Memory, String>;
    fn open(&self) -> Result<(), String>;
    fn close(&self) -> Result<(), String>;
    fn sequence(&self) -> u32;
    fn matches_write(&self, token: &str) -> Result<bool, String>;
    fn empty(&self) -> Result<(), String>;
    // 成功时转交内存；失败时 Memory 的析构负责释放。
    fn set(&self, format: u32, memory: &mut Self::Memory) -> Result<(), String>;
    fn verify(&self, format: u32, expected: &[u8]) -> Result<(), String>;
}

struct Lease<'a, B: Backend>(&'a B, bool);
impl<B: Backend> Drop for Lease<'_, B> {
    fn drop(&mut self) {
        if self.1 {
            let _ = self.0.close();
        }
    }
}

/// 输入只能来自已核验的有界 HGLOBAL 格式快照。保留载荷并规范化排除元数据，
/// 不声称整个格式集合与最初相同；失败后不自动重试或回退未标记写入。
pub(super) fn restore(
    entries: &[(u32, Vec<u8>)],
    expected_sequence: u32,
    expected_write_id: &str,
    backend: &impl Backend,
) -> Result<(), String> {
    if expected_write_id.len() != 32
        || !expected_write_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("缺少有效的本次写入标识；未开始恢复".into());
    }
    let mut total = 0usize;
    let mut seen = std::collections::HashSet::new();
    if entries.len() > 128
        || entries.iter().any(|(format, bytes)| {
            total = total.saturating_add(bytes.len());
            *format == 0 || bytes.is_empty() || !seen.insert(*format) || total > 16 * 1024 * 1024
        })
    {
        return Err("剪贴板恢复快照无效；未开始写入".into());
    }
    let formats = [
        backend.register(EXCLUSION_FORMATS[0])?,
        backend.register(EXCLUSION_FORMATS[1])?,
    ];
    if formats[0] == 0 || formats[1] == 0 || formats[0] == formats[1] {
        return Err("剪贴板排除格式注册无效；未开始写入".into());
    }
    let marker = 1u32.to_le_bytes();
    let mut allocations = Vec::new();
    for format in formats {
        allocations.push((format, marker.to_vec(), backend.allocate(&marker)?));
    }
    for (format, bytes) in entries
        .iter()
        .filter(|(format, _)| !formats.contains(format))
    {
        allocations.push((*format, bytes.clone(), backend.allocate(bytes)?));
    }
    backend.open()?;
    let mut lease = Lease(backend, true);
    if backend.sequence() != expected_sequence {
        return Err("打开剪贴板后序号已变化；未覆盖当前内容".into());
    }
    if !backend.matches_write(expected_write_id)? {
        return Err("剪贴板已不属于本次临时写入；未覆盖当前内容".into());
    }
    if backend.sequence() != expected_sequence {
        return Err("核验写入标识期间剪贴板已变化；未覆盖当前内容".into());
    }
    backend.empty()?;
    for (format, _, memory) in &mut allocations {
        backend
            .set(*format, memory)
            .map_err(|error| format!("剪贴板恢复未完成，原内容可能已清空；未自动重试：{error}"))?;
    }
    // 在同一打开期间读回，外部写入方尚不能插入。关闭失败仍报告失败。
    for (format, bytes, _) in &allocations {
        backend.verify(*format, bytes)?;
    }
    lease.1 = false;
    backend.close()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[derive(Default)]
    struct Fake {
        log: RefCell<Vec<String>>,
        fail: Option<u32>,
        sequence: u32,
        reject_token: bool,
    }
    impl Backend for Fake {
        type Memory = Vec<u8>;
        fn register(&self, name: &str) -> Result<u32, String> {
            Ok(if name == EXCLUSION_FORMATS[0] {
                50000
            } else {
                50001
            })
        }
        fn allocate(&self, bytes: &[u8]) -> Result<Vec<u8>, String> {
            Ok(bytes.to_vec())
        }
        fn open(&self) -> Result<(), String> {
            self.log.borrow_mut().push("open".into());
            Ok(())
        }
        fn close(&self) -> Result<(), String> {
            self.log.borrow_mut().push("close".into());
            Ok(())
        }
        fn sequence(&self) -> u32 {
            self.sequence
        }
        fn matches_write(&self, _: &str) -> Result<bool, String> {
            self.log.borrow_mut().push("owner".into());
            Ok(!self.reject_token)
        }
        fn empty(&self) -> Result<(), String> {
            self.log.borrow_mut().push("empty".into());
            Ok(())
        }
        fn set(&self, format: u32, _: &mut Vec<u8>) -> Result<(), String> {
            self.log.borrow_mut().push(format.to_string());
            if self.fail == Some(format) {
                Err("failed".into())
            } else {
                Ok(())
            }
        }
        fn verify(&self, format: u32, _: &[u8]) -> Result<(), String> {
            self.log.borrow_mut().push(format!("verify:{format}"));
            Ok(())
        }
    }

    #[test]
    fn restoration_marks_before_payload_and_normalizes_existing_metadata() {
        let backend = Fake::default();
        restore(
            &[(13, vec![65, 0, 0, 0]), (50000, vec![9]), (50001, vec![9])],
            0,
            "0123456789abcdef0123456789abcdef",
            &backend,
        )
        .unwrap();
        assert_eq!(
            *backend.log.borrow(),
            [
                "open",
                "owner",
                "empty",
                "50000",
                "50001",
                "13",
                "verify:50000",
                "verify:50001",
                "verify:13",
                "close"
            ]
        );
    }

    #[test]
    fn changed_sequence_preserves_newer_clipboard_and_closes_once() {
        let backend = Fake {
            sequence: 2,
            ..Default::default()
        };
        assert!(restore(
            &[(13, vec![65, 0, 0, 0])],
            1,
            "0123456789abcdef0123456789abcdef",
            &backend
        )
        .is_err());
        assert_eq!(*backend.log.borrow(), ["open", "close"]);
    }

    #[test]
    fn restoration_marker_failure_never_reaches_payload_or_retries() {
        for format in [50000, 50001] {
            let backend = Fake {
                fail: Some(format),
                ..Default::default()
            };
            assert!(restore(
                &[(13, vec![65, 0, 0, 0])],
                0,
                "0123456789abcdef0123456789abcdef",
                &backend
            )
            .is_err());
            let log = backend.log.borrow();
            assert!(!log
                .iter()
                .any(|step| step == "13" || step.starts_with("verify:")));
            assert_eq!(log.iter().filter(|step| *step == "close").count(), 1);
        }
    }

    #[test]
    fn observed_newer_sequence_cannot_adopt_another_writers_token() {
        let backend = Fake {
            reject_token: true,
            ..Default::default()
        };
        assert!(restore(
            &[(13, vec![65, 0, 0, 0])],
            0,
            "0123456789abcdef0123456789abcdef",
            &backend
        )
        .is_err());
        assert_eq!(*backend.log.borrow(), ["open", "owner", "close"]);
    }
}
