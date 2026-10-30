//! 独立线程上的分层亮度提示；请求槽有界，不依赖 GPUI 绘制或主窗口状态。
use crate::{system_hud_paint::HudBitmap, system_hud_window::SystemHudPlacement};
use std::{
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::w,
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM},
        Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{
            HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
                IsIconic, KillTimer, PeekMessageW, PostThreadMessageW, RegisterClassW, SetTimer,
                SetWindowPos, ShowWindow, UpdateLayeredWindow, HTTRANSPARENT, HWND_TOPMOST,
                MA_NOACTIVATE, MSG, PM_NOREMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_APP, WM_ERASEBKGND,
                WM_MOUSEACTIVATE, WM_NCCALCSIZE, WM_NCHITTEST, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
                WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
            },
        },
    },
};

const WAKE: u32 = WM_APP + 0x4b1;
const LOGICAL_WIDTH: f32 = 228.;
const LOGICAL_HEIGHT: f32 = 76.;
const MAX_EXTENT: i32 = 4096;
const MAX_TIMER_MS: u128 = 0x7fff_ffff;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeSystemHudSnapshot {
    pub ready: bool,
    pub visible: bool,
    pub hwnd: isize,
    pub generation: u64,
    pub percent: Option<u8>,
    /// 从接受显示请求到原生窗口更新完成，不含显示器扫描输出。
    pub last_present_us: u64,
}

/// HWND、位图和计时器仅由后台窗口线程持有；销毁句柄不等待线程退出。
pub struct NativeSystemHud {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    thread_id: u32,
    closed: bool,
    wake_posted: bool,
    revision: u64,
    pending: Option<Command>,
    submitted_at: Option<(u64, Instant)>,
    error: Option<String>,
    snapshot: NativeSystemHudSnapshot,
}

enum Command {
    Show {
        generation: u64,
        placement: SystemHudPlacement,
        percent: Option<u8>,
        hold: Duration,
    },
    Hide,
    Quit,
}

impl NativeSystemHud {
    pub fn logical_size() -> (f32, f32) {
        (LOGICAL_WIDTH, LOGICAL_HEIGHT)
    }

    pub fn new() -> Result<Self, String> {
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("omnitrigger-brightness-hud".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(|| worker(&worker_shared));
                let mut state = worker_shared.lock();
                match outcome {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => state.error = Some(error),
                    Err(_) => state.error = Some("亮度提示窗口线程意外退出。".into()),
                }
                state.closed = true;
                state.thread_id = 0;
                state.pending = None;
                state.snapshot.ready = false;
                state.snapshot.visible = false;
                state.snapshot.hwnd = 0;
            })
            .map_err(|error| format!("无法启动亮度提示窗口线程：{error}"))?;
        Ok(Self { shared })
    }

    /// 最新请求替换尚未处理的旧请求；成功返回仅表示已接受显示命令。
    pub fn show(
        &self,
        placement: SystemHudPlacement,
        percent: Option<u8>,
        hold: Duration,
    ) -> Result<(), String> {
        let submitted_at = Instant::now();
        validate_placement(&placement)?;
        let mut state = self.shared.lock();
        if state.closed {
            return Err("亮度提示窗口线程已退出。".into());
        }
        let generation = state
            .revision
            .checked_add(1)
            .ok_or_else(|| "亮度提示窗口请求序号已耗尽。".to_owned())?;
        state.revision = generation;
        state.submitted_at = Some((generation, submitted_at));
        state.pending = Some(Command::Show {
            generation,
            placement,
            percent: percent.map(|value| value.min(100)),
            hold,
        });
        state.wake()
    }

    pub fn hide(&self) {
        self.shared.enqueue_control(Command::Hide);
    }

    pub fn take_error(&self) -> Option<String> {
        self.shared.lock().error.take()
    }

    pub fn is_visible(&self) -> bool {
        self.shared.lock().snapshot.visible
    }

    pub fn snapshot(&self) -> NativeSystemHudSnapshot {
        self.shared.lock().snapshot.clone()
    }
}

impl Drop for NativeSystemHud {
    fn drop(&mut self) {
        self.shared.enqueue_control(Command::Quit);
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn enqueue_control(&self, command: Command) {
        let mut state = self.lock();
        if state.closed {
            return;
        }
        state.revision = state.revision.saturating_add(1);
        state.submitted_at = None;
        state.pending = Some(command);
        let _ = state.wake();
    }

    fn is_current(&self, generation: u64) -> bool {
        let state = self.lock();
        !state.closed && state.revision == generation
    }

    fn failed(&self, error: String) {
        self.lock().error = Some(error);
    }
}

impl State {
    fn wake(&mut self) -> Result<(), String> {
        // 未发布线程 ID 时只保留单槽命令；工作线程建立消息队列后负责首次唤醒。
        if self.thread_id == 0 || self.wake_posted {
            return Ok(());
        }
        match unsafe { PostThreadMessageW(self.thread_id, WAKE, WPARAM(0), LPARAM(0)) } {
            Ok(()) => {
                self.wake_posted = true;
                Ok(())
            }
            Err(error) => {
                let error = format!("无法提交亮度提示窗口消息：{error}");
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }
}

fn validate_placement(placement: &SystemHudPlacement) -> Result<(), String> {
    if !(1..=MAX_EXTENT).contains(&placement.width) || !(1..=MAX_EXTENT).contains(&placement.height)
    {
        return Err("亮度提示窗口尺寸无效或过大。".into());
    }
    if !placement.monitor_bounds.is_valid()
        || i64::from(placement.x) < i64::from(placement.monitor_bounds.left)
        || i64::from(placement.y) < i64::from(placement.monitor_bounds.top)
        || i64::from(placement.x) + i64::from(placement.width)
            > i64::from(placement.monitor_bounds.right)
        || i64::from(placement.y) + i64::from(placement.height)
            > i64::from(placement.monitor_bounds.bottom)
    {
        return Err("亮度提示窗口超出目标屏幕范围。".into());
    }
    Ok(())
}

struct NativeWindow(HWND);

impl Drop for NativeWindow {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCCALCSIZE => LRESULT(0),
        WM_ERASEBKGND => LRESULT(1),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn create_window() -> Result<NativeWindow, String> {
    static CLASS: OnceLock<Result<(), String>> = OnceLock::new();
    let instance = unsafe { GetModuleHandleW(None) }
        .map_err(|error| format!("无法读取亮度提示窗口模块：{error}"))?;
    CLASS
        .get_or_init(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("OmniTrigger.NativeBrightnessHud"),
                ..Default::default()
            };
            if unsafe { RegisterClassW(&class) } == 0 {
                Err(format!(
                    "无法注册亮度提示窗口：{}",
                    std::io::Error::last_os_error()
                ))
            } else {
                Ok(())
            }
        })
        .clone()?;
    unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
            w!("OmniTrigger.NativeBrightnessHud"),
            w!("OmniTrigger · 亮度"),
            WS_POPUP,
            0,
            0,
            LOGICAL_WIDTH as i32,
            LOGICAL_HEIGHT as i32,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .map(NativeWindow)
    .map_err(|error| format!("无法创建亮度提示窗口：{error}"))
}

#[derive(Clone, Copy)]
struct HideDeadline {
    timer_id: usize,
    generation: u64,
    at: Instant,
}

impl HideDeadline {
    fn expired(self, timer_id: usize, generation: u64, now: Instant) -> bool {
        self.timer_id == timer_id && self.generation == generation && now >= self.at
    }
}

struct Worker {
    window: NativeWindow,
    bitmap: HudBitmap,
    bitmap_size: (i32, i32),
    deadline: Option<HideDeadline>,
    next_timer_id: usize,
}

impl Worker {
    fn hide(&mut self, shared: &Shared) {
        if let Some(deadline) = self.deadline.take() {
            let _ = unsafe { KillTimer(Some(self.window.0), deadline.timer_id) };
        }
        let _ = unsafe { ShowWindow(self.window.0, SW_HIDE) };
        shared.lock().snapshot.visible = false;
    }

    fn show(
        &mut self,
        shared: &Shared,
        generation: u64,
        placement: SystemHudPlacement,
        percent: Option<u8>,
        hold: Duration,
    ) -> Result<(), String> {
        if !shared.is_current(generation) {
            return Ok(());
        }
        if self.bitmap_size != (placement.width, placement.height) {
            self.bitmap = HudBitmap::new(placement.width, placement.height)?;
            self.bitmap_size = (placement.width, placement.height);
        }
        let scale =
            (placement.width as f32 / LOGICAL_WIDTH).min(placement.height as f32 / LOGICAL_HEIGHT);
        self.bitmap.render(percent, scale)?;
        if !shared.is_current(generation) {
            return Ok(());
        }
        let destination = POINT {
            x: placement.x,
            y: placement.y,
        };
        let size = SIZE {
            cx: placement.width,
            cy: placement.height,
        };
        let source = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                self.window.0,
                None,
                Some(&destination),
                Some(&size),
                Some(self.bitmap.hdc()),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
        .map_err(|error| format!("无法绘制亮度提示窗口：{error}"))?;
        if unsafe { IsIconic(self.window.0) }.as_bool() {
            let _ = unsafe { ShowWindow(self.window.0, SW_SHOWNOACTIVATE) };
        }
        unsafe {
            SetWindowPos(
                self.window.0,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .map_err(|error| format!("无法显示亮度提示窗口：{error}"))?;

        if let Some(deadline) = self.deadline.take() {
            let _ = unsafe { KillTimer(Some(self.window.0), deadline.timer_id) };
        }
        self.next_timer_id = self.next_timer_id.wrapping_add(1).max(1);
        let timer_ms = hold.as_millis().clamp(1, MAX_TIMER_MS) as u32;
        let at = Instant::now() + Duration::from_millis(u64::from(timer_ms));
        let timer_id = unsafe { SetTimer(Some(self.window.0), self.next_timer_id, timer_ms, None) };
        if timer_id == 0 {
            return Err(format!(
                "无法设置亮度提示收起计时器：{}",
                std::io::Error::last_os_error()
            ));
        }
        self.deadline = Some(HideDeadline {
            timer_id,
            generation,
            at,
        });
        let mut state = shared.lock();
        state.snapshot.last_present_us = state
            .submitted_at
            .filter(|(request, _)| *request == generation)
            .map_or(0, |(_, at)| {
                at.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
            });
        state.snapshot.visible = true;
        state.snapshot.generation = generation;
        state.snapshot.percent = percent;
        Ok(())
    }
}

fn worker(shared: &Shared) -> Result<(), String> {
    let mut message = MSG::default();
    // PostThreadMessage 需要先建立本线程的消息队列。
    let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    // DPI_AWARENESS_CONTEXT_UNAWARE (-1) 是合法的旧上下文；只有空值表示失败。
    if previous.0.is_null() {
        return Err(format!(
            "无法设置亮度提示窗口 DPI 上下文：{}",
            std::io::Error::last_os_error()
        ));
    }
    let window = create_window()?;
    // 只在启动时读取一次鼠标和屏幕几何，按当前屏幕 DPI 预热；不读取硬件亮度。
    let initial_size = crate::capture_adjustment_point()
        .ok()
        .and_then(|point| {
            crate::system_hud_placement(point, None, LOGICAL_WIDTH, LOGICAL_HEIGHT).ok()
        })
        .map_or((LOGICAL_WIDTH as i32, LOGICAL_HEIGHT as i32), |placement| {
            (placement.width, placement.height)
        });
    let initial_scale =
        (initial_size.0 as f32 / LOGICAL_WIDTH).min(initial_size.1 as f32 / LOGICAL_HEIGHT);
    let mut bitmap = HudBitmap::new(initial_size.0, initial_size.1)?;
    bitmap.render(Some(100), initial_scale)?;
    bitmap.render(None, initial_scale)?;
    let mut worker = Worker {
        window,
        bitmap,
        bitmap_size: initial_size,
        deadline: None,
        next_timer_id: 0,
    };
    {
        let mut state = shared.lock();
        state.thread_id = unsafe { GetCurrentThreadId() };
        state.snapshot.ready = true;
        state.snapshot.hwnd = worker.window.0 .0 as isize;
        if state.pending.is_some() {
            state.wake()?;
        }
    }

    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result == -1 {
            return Err(format!(
                "无法读取亮度提示窗口消息：{}",
                std::io::Error::last_os_error()
            ));
        }
        if result == 0 {
            return Ok(());
        }
        if message.message == WAKE {
            let command = {
                let mut state = shared.lock();
                state.wake_posted = false;
                state.pending.take()
            };
            match command {
                Some(Command::Show {
                    generation,
                    placement,
                    percent,
                    hold,
                }) => {
                    if let Err(error) = worker.show(shared, generation, placement, percent, hold) {
                        worker.hide(shared);
                        shared.failed(error);
                    }
                }
                Some(Command::Hide) => worker.hide(shared),
                Some(Command::Quit) => {
                    worker.hide(shared);
                    return Ok(());
                }
                None => {}
            }
        } else if message.message == WM_TIMER && message.hwnd == worker.window.0 {
            if let Some(deadline) = worker.deadline {
                let now = Instant::now();
                // 以已呈现帧为准；新请求若投递失败，也不能阻止旧窗口按原期限收起。
                let generation = shared.lock().snapshot.generation;
                if deadline.expired(message.wParam.0, generation, now) {
                    worker.hide(shared);
                } else if deadline.timer_id == message.wParam.0 && deadline.generation == generation
                {
                    // 提前到达的低精度消息只等待剩余时间，不能多等一个完整停留周期。
                    let remaining = deadline.at.saturating_duration_since(now).as_millis() + 1;
                    if unsafe {
                        SetTimer(
                            Some(worker.window.0),
                            deadline.timer_id,
                            remaining.clamp(1, MAX_TIMER_MS) as u32,
                            None,
                        )
                    } == 0
                    {
                        worker.hide(shared);
                        shared.failed("无法继续亮度提示收起计时器。".into());
                    }
                }
            }
        } else {
            let _ = unsafe { DispatchMessageW(&message) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_core::ScreenRect;

    fn placement() -> SystemHudPlacement {
        SystemHudPlacement {
            monitor_device: "test".into(),
            monitor_bounds: ScreenRect {
                left: -1920,
                top: -1080,
                right: 0,
                bottom: 0,
            },
            x: -1000,
            y: -200,
            width: 228,
            height: 76,
        }
    }

    #[test]
    fn startup_queue_keeps_only_latest_show_without_needing_a_thread_id() {
        let hud = NativeSystemHud {
            shared: Arc::new(Shared::default()),
        };
        for percent in 0..=100 {
            hud.show(placement(), Some(percent), Duration::from_secs(1))
                .unwrap();
        }
        let state = hud.shared.lock();
        assert_eq!(state.thread_id, 0);
        assert!(!state.wake_posted);
        assert!(matches!(
            state.pending,
            Some(Command::Show {
                percent: Some(100),
                ..
            })
        ));
        assert_eq!(state.revision, 101);
    }

    #[test]
    fn hide_and_drop_replace_queued_show_and_do_not_wait_for_ready() {
        let hud = NativeSystemHud {
            shared: Arc::new(Shared::default()),
        };
        hud.show(placement(), Some(35), Duration::from_secs(1))
            .unwrap();
        hud.hide();
        assert!(matches!(hud.shared.lock().pending, Some(Command::Hide)));
        let shared = Arc::clone(&hud.shared);
        drop(hud);
        assert!(matches!(shared.lock().pending, Some(Command::Quit)));
    }

    #[test]
    fn invalid_placement_and_exhausted_generation_preserve_pending_command() {
        let hud = NativeSystemHud {
            shared: Arc::new(Shared::default()),
        };
        hud.show(placement(), None, Duration::from_secs(1)).unwrap();
        let mut invalid = placement();
        invalid.width = i32::MAX;
        assert!(hud.show(invalid, Some(40), Duration::from_secs(1)).is_err());
        assert_eq!(hud.shared.lock().revision, 1);
        hud.shared.lock().revision = u64::MAX;
        assert!(hud
            .show(placement(), Some(40), Duration::from_secs(1))
            .is_err());
        assert!(matches!(
            hud.shared.lock().pending,
            Some(Command::Show { percent: None, .. })
        ));
    }

    #[test]
    fn old_or_early_timer_cannot_hide_current_generation() {
        let now = Instant::now();
        let deadline = HideDeadline {
            timer_id: 2,
            generation: 4,
            at: now + Duration::from_secs(1),
        };
        assert!(!deadline.expired(1, 4, now + Duration::from_secs(2)));
        assert!(!deadline.expired(2, 5, now + Duration::from_secs(2)));
        assert!(!deadline.expired(2, 4, now));
        assert!(deadline.expired(2, 4, now + Duration::from_secs(1)));
    }

    #[test]
    fn failed_new_submission_does_not_extend_the_visible_frame_deadline() {
        let now = Instant::now();
        let mut state = State {
            revision: 5,
            ..State::default()
        };
        state.snapshot.generation = 4;
        state.snapshot.visible = true;
        let deadline = HideDeadline {
            timer_id: 2,
            generation: 4,
            at: now,
        };
        assert!(deadline.expired(2, state.snapshot.generation, now));
        assert!(!deadline.expired(2, state.revision, now));
    }
}
