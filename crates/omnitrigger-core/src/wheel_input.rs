//! 显式可选的滚轮输入归一化。适配器提供时间与鼠标会话快照；不读取平台状态。
use crate::{Axis, Decision, Engine, InputEvent, InputKind};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WheelInputPolicy {
    /// 缺省保留输入，不改变既有Packet/Notch或录制结果。
    #[default]
    Unfiltered,
    /// 垂直增量严格位于(-10,10)且没有鼠标会话时，共享150ms间隔。
    /// 时间戳在匹配之前更新；被门禁排除的原事件透传，不消费、不缓存、不补发。
    SmallVertical150ms,
}

/// 必须由适配器明确提供。普通物理按住、键盘捕获层、正在收尾的旧按压，
/// 都不自动等于这里的鼠标会话；holding_move_key表示鼠标会话自身的持有键。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WheelSessionSnapshot {
    pub captured_mouse_buttons: bool,
    pub holding_move_key: bool,
}

impl WheelSessionSnapshot {
    pub fn is_active(self) -> bool {
        self.captured_mouse_buttons || self.holding_move_key
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WheelInputAdmission {
    /// 将未经修改的事件继续交给后续输入处理。
    Forward,
    /// 直接透传原事件；不送匹配器，不取消等待中的按压，也不补发。
    PassThrough,
}

/// 一个监听输入流共享一个实例，不能按规则、应用、方向或指针位置拆分。
/// 取消、释放、前台变化与暂停不重建实例；完整重启输入流才从零开始。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WheelInputNormalizer {
    policy: WheelInputPolicy,
    previous_small_ms: i64,
}

impl WheelInputNormalizer {
    pub fn new(policy: WheelInputPolicy) -> Self {
        Self {
            policy,
            previous_small_ms: 0,
        }
    }

    /// 更改显式策略不清空已有历史；暂停监听不应通过此方法切换策略。
    pub fn set_policy(&mut self, policy: WheelInputPolicy) {
        self.policy = policy;
    }

    /// gate_time_ms必须是明确提供的绝对毫秒时基（例如系统运行时间），
    /// 不应直接复用从监听启动归零的Engine时间。初始previous=0是显式语义：
    /// 首次输入在时基150ms之前也会被门禁排除。倒退时间不更新previous。
    ///
    /// 本方法有意先于调用者的注入/启用门禁：只要被观测到，小增量自注入或
    /// 暂停期间事件也可推进门禁历史；后续Engine仍忽略injected事件。
    pub fn admit_at(
        &mut self,
        event: InputEvent,
        gate_time_ms: i64,
        session: WheelSessionSnapshot,
    ) -> WheelInputAdmission {
        if self.policy == WheelInputPolicy::Unfiltered {
            return WheelInputAdmission::Forward;
        }
        if let InputKind::Wheel {
            axis: Axis::Vertical,
            delta,
        } = event.kind
            && (-9..=9).contains(&delta)
            && !session.is_active()
        {
            // 用宽整数计算差值，避免调用方极端时间戳使有界路径溢出。
            if i128::from(gate_time_ms) - i128::from(self.previous_small_ms) < 150 {
                return WheelInputAdmission::PassThrough;
            }
            self.previous_small_ms = gate_time_ms;
        }
        WheelInputAdmission::Forward
    }

    /// 可供无指针/前台更新的回放调用。真实适配器有位置或身份更新时，应先调用
    /// admit_at，再仅在Forward分支更新上下文并交付Engine；录制器始终接收原事件。
    pub fn process_at(
        &mut self,
        engine: &mut Engine,
        event: InputEvent,
        gate_time_ms: i64,
        engine_time_ms: u64,
        session: WheelSessionSnapshot,
    ) -> Decision {
        match self.admit_at(event, gate_time_ms, session) {
            WheelInputAdmission::Forward => engine.process_at(event, engine_time_ms),
            WheelInputAdmission::PassThrough => Decision::default(),
        }
    }
}
