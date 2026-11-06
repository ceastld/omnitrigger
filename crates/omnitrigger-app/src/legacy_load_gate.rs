//! 插件加载准入缓存。没有线程、进程、时钟、文件、队列、重试或目标捕获。
//! key只能来自已核验安装收据；不是用户ActionCall输入，也不是远端进程镜像证明。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegacyKey {
    pub package_id: String,
    pub artifact_fingerprint: String,
    pub fixed_scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LoadState {
    Disabled,
    Preparing,
    Ready,
    Unavailable(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrepareTicket {
    pub generation: u64,
    pub key: LegacyKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrepareResult {
    Ready,
    NotReady(&'static str),
}

pub(crate) struct LegacyLoadGate {
    key: Option<LegacyKey>,
    generation: u64,
    state: LoadState,
}

impl Default for LegacyLoadGate {
    fn default() -> Self {
        Self {
            key: None,
            generation: 0,
            state: LoadState::Disabled,
        }
    }
}

impl LegacyLoadGate {
    /// 安装完成/应用加载触发一次。相同generation的重复事件不会再排预备任务。
    /// explicit_refresh只用于现有“刷新/重试”用户操作，不能由定时器调用。
    pub fn begin(
        &mut self,
        key: LegacyKey,
        explicit_refresh: bool,
    ) -> Result<Option<PrepareTicket>, &'static str> {
        if self.state == LoadState::Preparing {
            // 更换安装指纹/显式刷新必须先由拥有者取消旧作业并disable；不能静默吞掉换代。
            return if self.key.as_ref() == Some(&key) && !explicit_refresh {
                Ok(None)
            } else {
                Err("legacy_preparation_in_progress")
            };
        }
        if self.key.as_ref() == Some(&key) && !explicit_refresh {
            return Ok(None);
        }
        let next = self
            .generation
            .checked_add(1)
            .ok_or("preparation_generation_exhausted")?;
        self.generation = next;
        self.key = Some(key.clone());
        self.state = LoadState::Preparing;
        Ok(Some(PrepareTicket {
            generation: next,
            key,
        }))
    }

    pub fn finish(&mut self, ticket: &PrepareTicket, result: PrepareResult) -> bool {
        if self.generation != ticket.generation
            || self.key.as_ref() != Some(&ticket.key)
            || self.state != LoadState::Preparing
        {
            return false;
        }
        self.state = match result {
            PrepareResult::Ready => LoadState::Ready,
            PrepareResult::NotReady(code) => LoadState::Unavailable(code),
        };
        true
    }

    pub fn begin_shutdown(&mut self, key: LegacyKey) -> Result<PrepareTicket, &'static str> {
        let next = self
            .generation
            .checked_add(1)
            .ok_or("preparation_generation_exhausted")?;
        self.generation = next;
        self.key = None;
        self.state = LoadState::Disabled;
        Ok(PrepareTicket {
            generation: next,
            key,
        })
    }

    pub fn disable(&mut self) {
        self.key = None;
        self.state = LoadState::Disabled;
        // 过期完成事件因key/state不匹配被拒绝；不向业务队列补发任何调用。
    }

    /// 必须在DesktopHost::capture/submit_with_host之前调用；Ready只是一份加载时观察。
    /// 真正Script仍执行既有live-status与最多一次send，不能因缓存Ready跳过目标/管道门。
    pub fn admit(&self, key: &LegacyKey) -> Result<(), &'static str> {
        if self.state == LoadState::Disabled {
            return Err("legacy_disabled");
        }
        if self.key.as_ref() != Some(key) {
            return Err("legacy_not_prepared");
        }
        match self.state {
            LoadState::Ready => Ok(()),
            LoadState::Preparing => Err("legacy_preparing"),
            LoadState::Disabled => Err("legacy_disabled"),
            LoadState::Unavailable(code) => Err(code),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> LegacyKey {
        LegacyKey {
            package_id: "synthetic.legacy".into(),
            artifact_fingerprint: "owned_fixture".into(),
            fixed_scope: "owned-neutral-root".into(),
        }
    }
    #[test]
    fn install_and_load_share_one_preparation_no_queued_business() {
        let mut gate = LegacyLoadGate::default();
        let key = key();
        let ticket = gate.begin(key.clone(), false).unwrap().unwrap();
        assert!(gate.begin(key.clone(), false).unwrap().is_none());
        assert_eq!(gate.admit(&key), Err("legacy_preparing"));
        assert!(gate.finish(&ticket, PrepareResult::Ready));
        assert!(gate.begin(key.clone(), false).unwrap().is_none());
        assert_eq!(gate.admit(&key), Ok(()));
    }
    #[test]
    fn failed_load_rejects_and_does_not_automatically_retry() {
        let mut gate = LegacyLoadGate::default();
        let key = key();
        let ticket = gate.begin(key.clone(), false).unwrap().unwrap();
        assert!(gate.finish(&ticket, PrepareResult::NotReady("host_stopping")));
        assert_eq!(gate.admit(&key), Err("host_stopping"));
        assert!(gate.begin(key.clone(), false).unwrap().is_none());
        assert!(gate.begin(key, true).unwrap().is_some());
    }
    #[test]
    fn disable_or_new_generation_ignores_late_completion() {
        let mut gate = LegacyLoadGate::default();
        let key = key();
        let old = gate.begin(key.clone(), false).unwrap().unwrap();
        gate.disable();
        assert!(!gate.finish(&old, PrepareResult::Ready));
        let next = gate.begin(key.clone(), false).unwrap().unwrap();
        assert!(!gate.finish(&old, PrepareResult::Ready));
        assert_eq!(gate.admit(&key), Err("legacy_preparing"));
        assert!(gate.finish(&next, PrepareResult::Ready));
    }
    #[test]
    fn changed_artifact_requires_owner_to_cancel_old_preparation() {
        let mut gate = LegacyLoadGate::default();
        let old_key = key();
        let old = gate.begin(old_key.clone(), false).unwrap().unwrap();
        let mut changed = old_key;
        changed.artifact_fingerprint = "replacement".into();
        assert_eq!(
            gate.begin(changed.clone(), false),
            Err("legacy_preparation_in_progress")
        );
        assert_eq!(
            gate.begin(key(), true),
            Err("legacy_preparation_in_progress")
        );
        gate.disable();
        let next = gate.begin(changed.clone(), false).unwrap().unwrap();
        assert!(!gate.finish(&old, PrepareResult::Ready));
        assert_eq!(gate.admit(&changed), Err("legacy_preparing"));
        assert!(gate.finish(&next, PrepareResult::Ready));
        assert_eq!(gate.admit(&changed), Ok(()));
    }
    #[test]
    fn shutdown_invalidates_ready_and_late_prepare_completion() {
        let mut gate = LegacyLoadGate::default();
        let key = key();
        let preparation = gate.begin(key.clone(), false).unwrap().unwrap();
        assert!(gate.finish(&preparation, PrepareResult::Ready));
        let shutdown = gate.begin_shutdown(key.clone()).unwrap();
        assert!(shutdown.generation > preparation.generation);
        assert_eq!(gate.admit(&key), Err("legacy_disabled"));
        assert!(!gate.finish(&preparation, PrepareResult::Ready));
        assert_eq!(gate.admit(&key), Err("legacy_disabled"));
    }
}
