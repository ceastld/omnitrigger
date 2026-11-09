//! 只由ActionService创建的左键按下/释放作用域，不从脚本接收owner。
//! 实际后端必须共用Hook的输入账本；缺少该通道时NativeHost默认拒绝创建本类型。
use crate::{ActionError, CancellationToken};
use serde_json::{json, Value};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

pub const DOWN: &str = "mouse.left_down";
pub const UP: &str = "mouse.left_up";
const RELEASE_ATTEMPTS: u8 = 5;
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnedInputOwner(u64);
impl OwnedInputOwner {
    pub fn id(self) -> u64 {
        self.0
    }
}
pub(crate) fn new_owner() -> Result<OwnedInputOwner, ActionError> {
    NEXT_OWNER
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map(OwnedInputOwner)
        .map_err(|_| fault(DOWN, "input_owner_unavailable", false))
}
pub fn fault(method: &str, code: &str, unknown: bool) -> ActionError {
    let mut error = ActionError::new(code, "左键输入未获完整确认；不会改选目标或重放按下。");
    error.method = Some(method.into());
    error.effect_state = Some(if unknown { "unknown" } else { "not_started" }.into());
    error
}

/// 生产实现必须与ReplayRuntime共用所有权/交付账本，禁止另造裸SendInput发送器。
/// down_owned须先拒绝预先按住/不可判定状态，按原目标守卫确认一次按下交付。
/// release_owned只能结束指定owner已登记的按下；owner不拥有时必须无输入地成功。
/// 失败时保留未释放义务并阻止其它owner覆盖；清理不激活窗口、不改选目标。
pub trait OwnedLeftBackend: Send + Sync {
    fn check_original_target(&self) -> Result<(), ActionError>;
    /// cancelled须在交付前及等待中持续读取；它可同时观察调用与真实Service的取消。
    fn down_owned(
        &self,
        owner: OwnedInputOwner,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), ActionError>;
    fn release_owned(&self, owner: OwnedInputOwner) -> Result<(), ActionError>;
}

#[derive(Default)]
struct State {
    may_be_down: bool,
    release_attempts: u8,
    closed: bool,
}
pub struct OwnedLeftSession {
    owner: OwnedInputOwner,
    backend: Arc<dyn OwnedLeftBackend>,
    state: Mutex<State>,
}
impl OwnedLeftSession {
    pub fn new(owner: OwnedInputOwner, backend: Arc<dyn OwnedLeftBackend>) -> Self {
        Self {
            owner,
            backend,
            state: Mutex::new(State::default()),
        }
    }
    fn check(
        method: &str,
        cancelled: &dyn Fn() -> bool,
        deadline: Instant,
    ) -> Result<(), ActionError> {
        if cancelled() {
            return Err(fault(method, "cancelled", false));
        }
        if Instant::now() >= deadline {
            return Err(fault(method, "timeout", false));
        }
        Ok(())
    }
    fn release(&self, state: &mut State) -> Result<(), ActionError> {
        if !state.may_be_down {
            return Ok(());
        }
        while state.release_attempts != 0 {
            // 在可能panic的回调前记账；每个按压周期总共至多5次，finish/Drop不重置。
            state.release_attempts -= 1;
            if self.backend.release_owned(self.owner).is_ok() {
                state.may_be_down = false;
                return Ok(());
            }
        }
        Err(fault(UP, "input_cleanup_failed", true))
    }
    pub fn call(
        &self,
        method: &str,
        params: Value,
        token: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        self.call_if(method, params, deadline, || token.is_cancelled())
    }
    // 鼠标作用域专用谓词：持续观察调用与Service取消，不创建通用链接token或线程。
    pub(crate) fn call_if(
        &self,
        method: &str,
        params: Value,
        deadline: Instant,
        cancelled: impl Fn() -> bool,
    ) -> Result<Value, ActionError> {
        if !matches!(method, DOWN | UP) || params != json!({}) {
            return Err(fault(method, "invalid_input", false));
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err(fault(method, "action_inactive", false));
        }
        if method == DOWN {
            if state.may_be_down {
                return Err(fault(method, "input_busy", true));
            }
            Self::check(method, &cancelled, deadline)?;
            self.backend
                .check_original_target()
                .map_err(|error| fault(method, &error.code, false))?;
            state.may_be_down = true;
            state.release_attempts = RELEASE_ATTEMPTS;
            // 回调异常/交付结果不确定时也保留义务；release_owned自行核对真实owner账本。
            if let Err(error) = self.backend.down_owned(self.owner, deadline, &cancelled) {
                self.release(&mut state)?;
                return Err(fault(method, &error.code, true));
            }
            let after = Self::check(method, &cancelled, deadline)
                .and_then(|_| self.backend.check_original_target());
            if let Err(error) = after {
                self.release(&mut state)?;
                return Err(fault(method, &error.code, true));
            }
            Ok(json!({"submitted":true,"effectState":"unknown"}))
        } else {
            if !state.may_be_down {
                return Err(fault(method, "input_not_owned", false));
            }
            let guard = Self::check(method, &cancelled, deadline)
                .and_then(|_| self.backend.check_original_target());
            // 原目标变化/取消/到期均不能挡住已拥有按下的中性释放。
            self.release(&mut state)?;
            guard.map_err(|error| fault(method, &error.code, true))?;
            Ok(json!({"ownedPressEnded":true,"effectState":"unknown"}))
        }
    }
    pub fn finish(&self) -> Result<(), ActionError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        self.release(&mut state)?;
        Ok(())
    }
}
impl Drop for OwnedLeftSession {
    fn drop(&mut self) {
        let _ = catch_unwind(AssertUnwindSafe(|| self.finish()));
    }
}

#[cfg(test)]
#[path = "owned_mouse_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "owned_mouse_service_tests.rs"]
mod service_tests;
