//! 动作鼠标请求只经现有Hook/ReplayRuntime；此模块不直接SendInput。
use crate::{BackendCommand, ScriptTargetSnapshot};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Sender, SyncSender},
        Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug)]
pub struct OwnedLeftInputError {
    pub code: &'static str,
}
impl OwnedLeftInputError {
    pub(crate) fn new(code: &'static str) -> Self {
        Self { code }
    }
}
#[derive(Default, Debug)]
pub(crate) struct OwnedLeftGate {
    pub revoked: AtomicBool,
    pub cancelled: AtomicBool,
}
#[derive(Debug)]
struct Slot(Option<Arc<AtomicUsize>>);
impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(count) = &self.0 {
            count.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
#[derive(Debug)]
pub struct OwnedLeftRequest {
    pub(crate) owner: u64,
    pub(crate) down: bool,
    pub(crate) target: Option<ScriptTargetSnapshot>,
    pub(crate) deadline: Instant,
    pub(crate) gate: Arc<OwnedLeftGate>,
    pub(crate) reply: SyncSender<Result<(), OwnedLeftInputError>>,
    _slot: Slot,
}
impl OwnedLeftRequest {
    #[cfg(test)]
    pub(crate) fn fixture(
        owner: u64,
        down: bool,
        gate: Arc<OwnedLeftGate>,
    ) -> (Self, mpsc::Receiver<Result<(), OwnedLeftInputError>>) {
        let (reply, rx) = mpsc::sync_channel(1);
        (
            Self {
                owner,
                down,
                target: None,
                deadline: Instant::now() + Duration::from_secs(1),
                gate,
                reply,
                _slot: Slot(None),
            },
            rx,
        )
    }
    pub(crate) fn cleanup(owner: u64, gate: Arc<OwnedLeftGate>) -> Self {
        let (reply, _rx) = mpsc::sync_channel(1);
        Self {
            owner,
            down: false,
            target: None,
            deadline: Instant::now(),
            gate,
            reply,
            _slot: Slot(None),
        }
    }
}
pub(crate) struct Endpoint {
    commands: Sender<BackendCommand>,
    thread: u32,
    pub alive: AtomicBool,
    slots: Arc<AtomicUsize>,
}
static ACTIVE: OnceLock<Mutex<Weak<Endpoint>>> = OnceLock::new();
pub(crate) fn install(commands: Sender<BackendCommand>, thread: u32) -> Arc<Endpoint> {
    let endpoint = Arc::new(Endpoint {
        commands,
        thread,
        alive: AtomicBool::new(true),
        slots: Arc::new(AtomicUsize::new(0)),
    });
    *ACTIVE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(&endpoint);
    endpoint
}
#[derive(Clone)]
pub struct OwnedLeftInputClient(Arc<Endpoint>);
pub fn active_owned_left_input_client() -> Option<OwnedLeftInputClient> {
    let endpoint = ACTIVE
        .get_or_init(Default::default)
        .lock()
        .ok()?
        .upgrade()?;
    endpoint
        .alive
        .load(Ordering::Acquire)
        .then_some(OwnedLeftInputClient(endpoint))
}
impl OwnedLeftInputClient {
    pub fn down(
        &self,
        owner: u64,
        target: ScriptTargetSnapshot,
        deadline: Instant,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), OwnedLeftInputError> {
        self.request(owner, true, Some(target), deadline, cancelled)
    }
    pub fn release_owned(&self, owner: u64) -> Result<(), OwnedLeftInputError> {
        self.request(
            owner,
            false,
            None,
            Instant::now() + Duration::from_millis(250),
            || false,
        )
    }
    fn request(
        &self,
        owner: u64,
        down: bool,
        target: Option<ScriptTargetSnapshot>,
        deadline: Instant,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), OwnedLeftInputError> {
        if owner == 0 || !self.0.alive.load(Ordering::Acquire) {
            return Err(OwnedLeftInputError::new("input_owner_unavailable"));
        }
        self.0
            .slots
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 8).then_some(n + 1)
            })
            .map_err(|_| OwnedLeftInputError::new("input_busy"))?;
        let (reply, rx) = mpsc::sync_channel(1);
        let gate = Arc::new(OwnedLeftGate::default());
        let request = OwnedLeftRequest {
            owner,
            down,
            target,
            deadline,
            gate: gate.clone(),
            reply,
            _slot: Slot(Some(self.0.slots.clone())),
        };
        self.0
            .commands
            .send(BackendCommand::OwnedLeftInput(Box::new(request)))
            .map_err(|_| OwnedLeftInputError::new("input_owner_unavailable"))?;
        // 发送后唤醒失败也撤销Down资格；队列仍由旧端点/线程拥有，不能重选新Backend。
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                self.0.thread,
                super::platform::COMMAND_MESSAGE,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            )
        }
        .map_err(|_| {
            gate.cancelled.store(true, Ordering::Release);
            OwnedLeftInputError::new("input_owner_unavailable")
        })?;
        loop {
            match rx.recv_timeout(Duration::from_millis(2)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(OwnedLeftInputError::new("input_owner_unavailable"))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if cancelled() || Instant::now() >= deadline {
                // Up不被这个等待时限取消；已经入队的中性收尾继续由Hook账本完成。
                if down {
                    gate.cancelled.store(true, Ordering::Release);
                }
                return Err(OwnedLeftInputError::new(if cancelled() {
                    "cancelled"
                } else {
                    "timeout"
                }));
            }
        }
    }
}
pub(crate) struct OwnedLeftJobGuard {
    pub gate: Arc<OwnedLeftGate>,
    pub down: bool,
    pub target: Option<ScriptTargetSnapshot>,
    pub deadline: Instant,
}
impl OwnedLeftJobGuard {
    pub fn permits(&self) -> bool {
        if self.gate.revoked.load(Ordering::Acquire) {
            return false;
        }
        if !self.down {
            return true;
        }
        if self.gate.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            return false;
        }
        let Some(snapshot) = self.target else {
            return false;
        };
        let Ok(target) = crate::ScriptTarget::from_snapshot(snapshot) else {
            return false;
        };
        crate::read_pointing_root_if(&target, || {
            if self.gate.cancelled.load(Ordering::Acquire)
                || self.gate.revoked.load(Ordering::Acquire)
                || Instant::now() >= self.deadline
            {
                Err(crate::WindowGeometryError::new(
                    "cancelled",
                    "左键请求已失效。",
                    "not_started",
                ))
            } else {
                Ok(())
            }
        })
        .is_ok_and(|pointing| pointing.root_window == snapshot)
    }
}
pub fn owned_left_button_native_input(
    down: bool,
) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    crate::platform::owned_left_button_input(down)
}
#[cfg(test)]
mod tests {
    #[test]
    fn expired_cancelled_or_revoked_down_is_rejected_but_owned_up_ignores_target() {
        use super::*;
        let gate = Arc::new(OwnedLeftGate::default());
        let mut guard = OwnedLeftJobGuard {
            gate: gate.clone(),
            down: true,
            target: None,
            deadline: Instant::now(),
        };
        assert!(!guard.permits());
        gate.cancelled.store(true, Ordering::Release);
        assert!(!guard.permits());
        guard.down = false;
        assert!(guard.permits());
        gate.revoked.store(true, Ordering::Release);
        assert!(!guard.permits());
    }
    #[test]
    fn left_edges_are_mouse_inputs_with_paired_flags_and_existing_marker() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        };
        let down = super::owned_left_button_native_input(true);
        let up = super::owned_left_button_native_input(false);
        assert_eq!(down.r#type, INPUT_MOUSE);
        assert_eq!(up.r#type, INPUT_MOUSE);
        let (down, up) = unsafe { (down.Anonymous.mi, up.Anonymous.mi) };
        assert_eq!(down.dwFlags, MOUSEEVENTF_LEFTDOWN);
        assert_eq!(up.dwFlags, MOUSEEVENTF_LEFTUP);
        assert_eq!(down.dwExtraInfo, up.dwExtraInfo);
        assert_ne!(down.dwExtraInfo, 0);
    }
}
