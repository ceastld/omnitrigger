//! 触发器服务：绑定列表、编号分配、冲突检查、持久化与命中统计。
//!
//! 本模块不依赖 GPUI。界面只读取快照并调用命令，命令返回结构化错误；
//! 失败时内存与磁盘都保持原样，不静默改写用户录制的组合或绑定。

use crate::settings::{self, ActionSpec, Binding, Conflict};
use omnitrigger_core::TriggerSpec;
use std::collections::HashMap;

/// 绑定的持久化位置；测试用内存实现替换真实配置文件。
pub trait BindingStore {
    fn save(&mut self, bindings: &[Binding]) -> Result<(), String>;
}

/// 写入当前配置路径下的 `bindings.json`。
pub struct SettingsFile;

impl BindingStore for SettingsFile {
    fn save(&mut self, bindings: &[Binding]) -> Result<(), String> {
        settings::save(bindings)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerError {
    NotFound(u64),
    Conflict(Conflict),
    /// 配置校验或写入失败，原配置保持不变。
    Invalid(String),
    IdsExhausted,
}

impl TriggerError {
    pub fn message(&self) -> String {
        match self {
            Self::NotFound(_) => "触发器已不存在，请刷新后重试。".into(),
            Self::Conflict(conflict) => format!("{}：{}", conflict.title, conflict.detail),
            Self::Invalid(error) => error.clone(),
            Self::IdsExhausted => "触发器编号已用尽，无法添加新绑定。".into(),
        }
    }
}

/// 真实输入触发的命中统计；测试动作和面板点击不计入。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub hits: u64,
    pub last: Option<String>,
}

pub struct TriggerService<S = SettingsFile> {
    bindings: Vec<Binding>,
    next_id: Option<u64>,
    stats: HashMap<u64, Stats>,
    store: S,
}

impl<S: BindingStore> TriggerService<S> {
    pub fn new(bindings: Vec<Binding>, store: S) -> Self {
        let next_id = bindings
            .iter()
            .map(|binding| binding.spec.id)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        Self {
            bindings,
            next_id,
            stats: HashMap::new(),
            store,
        }
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub fn get(&self, id: u64) -> Option<&Binding> {
        self.bindings.iter().find(|binding| binding.spec.id == id)
    }

    /// 分配新编号；草稿、轮盘槽位运行编号与绑定共用同一序列，互不重复。
    pub fn allocate_id(&mut self) -> Result<u64, TriggerError> {
        let id = self.next_id.ok_or(TriggerError::IdsExhausted)?;
        self.next_id = id.checked_add(1);
        Ok(id)
    }

    /// 检查候选组合；`editing` 为正在修改的绑定，比较时排除它自身。
    pub fn check_candidate(
        &self,
        spec: &TriggerSpec,
        editing: Option<u64>,
    ) -> Result<(), Conflict> {
        let others: Vec<_> = self
            .bindings
            .iter()
            .filter(|binding| Some(binding.spec.id) != editing)
            .cloned()
            .collect();
        settings::check_candidate(&others, spec)
    }

    pub fn add(&mut self, binding: Binding) -> Result<u64, TriggerError> {
        let id = binding.spec.id;
        if self.get(id).is_some() {
            return Err(TriggerError::Invalid(format!("触发器编号 {id} 已被占用。")));
        }
        self.check_candidate(&binding.spec, None)
            .map_err(TriggerError::Conflict)?;
        let mut next = self.bindings.clone();
        next.push(binding);
        self.commit(next)?;
        if self.next_id.is_some_and(|next| next <= id) {
            self.next_id = id.checked_add(1);
        }
        Ok(id)
    }

    /// 替换已有绑定的组合、提示和动作，保留编号与启用状态。
    pub fn update(
        &mut self,
        id: u64,
        spec: TriggerSpec,
        message: String,
        action: ActionSpec,
    ) -> Result<(), TriggerError> {
        self.check_candidate(&spec, Some(id))
            .map_err(TriggerError::Conflict)?;
        let mut next = self.bindings.clone();
        let binding = next
            .iter_mut()
            .find(|binding| binding.spec.id == id)
            .ok_or(TriggerError::NotFound(id))?;
        binding.spec = TriggerSpec { id, ..spec };
        binding.message = message;
        binding.action = action;
        self.commit(next)
    }

    /// 切换启用状态，返回切换后的状态。
    pub fn toggle(&mut self, id: u64) -> Result<bool, TriggerError> {
        let mut next = self.bindings.clone();
        let binding = next
            .iter_mut()
            .find(|binding| binding.spec.id == id)
            .ok_or(TriggerError::NotFound(id))?;
        binding.enabled = !binding.enabled;
        let enabled = binding.enabled;
        self.commit(next)?;
        Ok(enabled)
    }

    /// 删除绑定及其统计，返回删除后建议选中的相邻绑定。
    pub fn remove(&mut self, id: u64) -> Result<Option<u64>, TriggerError> {
        let index = self
            .bindings
            .iter()
            .position(|binding| binding.spec.id == id)
            .ok_or(TriggerError::NotFound(id))?;
        let mut next = self.bindings.clone();
        next.remove(index);
        self.commit(next)?;
        self.stats.remove(&id);
        Ok(self
            .bindings
            .get(index)
            .or_else(|| self.bindings.last())
            .map(|binding| binding.spec.id))
    }

    /// 应交给输入后端注册的组合：已启用且动作当前可用。
    pub fn active_specs(&self, available: impl Fn(&ActionSpec) -> bool) -> Vec<TriggerSpec> {
        self.bindings
            .iter()
            .filter(|binding| binding.enabled && available(&binding.action))
            .map(|binding| binding.spec.clone())
            .collect()
    }

    /// 记录一次真实触发；时间由调用方传入，便于重放验证。
    pub fn record_hit(&mut self, id: u64, at: String) {
        let stats = self.stats.entry(id).or_default();
        stats.hits += 1;
        stats.last = Some(at);
    }

    pub fn stats(&self, id: u64) -> Stats {
        self.stats.get(&id).cloned().unwrap_or_default()
    }

    fn commit(&mut self, next: Vec<Binding>) -> Result<(), TriggerError> {
        settings::validate(&next).map_err(TriggerError::Invalid)?;
        self.store.save(&next).map_err(TriggerError::Invalid)?;
        self.bindings = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_core::{Activation, Button};

    #[derive(Default)]
    struct Memory {
        saved: Vec<Vec<Binding>>,
        fail: Option<String>,
    }

    impl BindingStore for Memory {
        fn save(&mut self, bindings: &[Binding]) -> Result<(), String> {
            if let Some(error) = &self.fail {
                return Err(error.clone());
            }
            self.saved.push(bindings.to_vec());
            Ok(())
        }
    }

    fn binding(id: u64, key: u16) -> Binding {
        Binding {
            spec: TriggerSpec {
                id,
                held: vec![Button::Key(0x14)],
                conditions: Vec::new(),
                activation: Activation::Press(Button::Key(key)),
            },
            message: format!("动作 {id}"),
            enabled: true,
            action: ActionSpec::Notification,
        }
    }

    fn service(bindings: Vec<Binding>) -> TriggerService<Memory> {
        TriggerService::new(bindings, Memory::default())
    }

    #[test]
    fn ids_continue_after_loaded_bindings_and_report_exhaustion() {
        let mut triggers = service(vec![binding(3, 0x41), binding(7, 0x42)]);
        assert_eq!(triggers.allocate_id(), Ok(8));
        assert_eq!(triggers.allocate_id(), Ok(9));

        let mut full = service(vec![binding(u64::MAX, 0x41)]);
        assert_eq!(full.allocate_id(), Err(TriggerError::IdsExhausted));
    }

    #[test]
    fn conflicting_add_is_rejected_without_saving_or_changing_bindings() {
        let mut triggers = service(vec![binding(1, 0x41)]);
        let error = triggers.add(binding(2, 0x41)).unwrap_err();
        assert!(matches!(
            error,
            TriggerError::Conflict(Conflict { other: Some(1), .. })
        ));
        assert_eq!(triggers.bindings(), &[binding(1, 0x41)]);
        assert!(triggers.store.saved.is_empty());

        assert_eq!(triggers.add(binding(2, 0x42)), Ok(2));
        assert_eq!(triggers.store.saved.last().unwrap().len(), 2);
        assert_eq!(triggers.allocate_id(), Ok(3));
    }

    #[test]
    fn update_ignores_its_own_combination_but_not_others() {
        let mut triggers = service(vec![binding(1, 0x41), binding(2, 0x42)]);
        let same = binding(1, 0x41);
        triggers
            .update(1, same.spec, "新提示".into(), ActionSpec::ShowPanel)
            .unwrap();
        assert_eq!(triggers.get(1).unwrap().message, "新提示");
        assert_eq!(triggers.get(1).unwrap().action, ActionSpec::ShowPanel);

        let taken = binding(1, 0x42).spec;
        assert!(matches!(
            triggers.update(1, taken, "x".into(), ActionSpec::Notification),
            Err(TriggerError::Conflict(Conflict { other: Some(2), .. }))
        ));
        assert_eq!(
            triggers.update(
                9,
                binding(9, 0x43).spec,
                "x".into(),
                ActionSpec::Notification
            ),
            Err(TriggerError::NotFound(9))
        );
    }

    #[test]
    fn failed_save_keeps_memory_state_unchanged() {
        let mut triggers = service(vec![binding(1, 0x41)]);
        triggers.store.fail = Some("磁盘已满".into());
        assert_eq!(
            triggers.toggle(1),
            Err(TriggerError::Invalid("磁盘已满".into()))
        );
        assert!(triggers.get(1).unwrap().enabled);
        assert_eq!(
            triggers.remove(1),
            Err(TriggerError::Invalid("磁盘已满".into()))
        );
        assert_eq!(triggers.bindings().len(), 1);
    }

    #[test]
    fn invalid_bindings_are_not_saved() {
        let mut triggers = service(vec![binding(1, 0x41)]);
        let mut empty = binding(2, 0x42);
        empty.message = "  ".into();
        assert!(matches!(triggers.add(empty), Err(TriggerError::Invalid(_))));
        assert!(triggers.store.saved.is_empty());
    }

    #[test]
    fn remove_selects_neighbor_and_drops_stats() {
        let mut triggers = service(vec![binding(1, 0x41), binding(2, 0x42), binding(3, 0x43)]);
        triggers.record_hit(2, "10:00:00".into());
        assert_eq!(triggers.remove(2), Ok(Some(3)));
        assert_eq!(triggers.stats(2), Stats::default());
        assert_eq!(triggers.remove(3), Ok(Some(1)));
        assert_eq!(triggers.remove(1), Ok(None));
    }

    #[test]
    fn active_specs_skip_disabled_and_unavailable_actions() {
        let mut panel = binding(3, 0x43);
        panel.action = ActionSpec::ShowPanel;
        let mut triggers = service(vec![binding(1, 0x41), binding(2, 0x42), panel]);
        triggers.toggle(2).unwrap();
        let ids: Vec<_> = triggers
            .active_specs(|action| *action != ActionSpec::ShowPanel)
            .into_iter()
            .map(|spec| spec.id)
            .collect();
        assert_eq!(ids, vec![1]);
    }

    #[test]
    fn hits_accumulate_with_caller_supplied_time() {
        let mut triggers = service(vec![binding(1, 0x41)]);
        triggers.record_hit(1, "09:00:00".into());
        triggers.record_hit(1, "09:00:05".into());
        assert_eq!(
            triggers.stats(1),
            Stats {
                hits: 2,
                last: Some("09:00:05".into())
            }
        );
    }
}
