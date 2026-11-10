//! 鼠标手势轨迹浮层：独立线程持有点击穿透的分层窗口并负责绘制。
//!
//! 输入 Hook 只把坐标放入有界队列；队列被占用或写满时丢弃轨迹点，不等待绘制线程。
//! 轨迹只用于显示，识别结果仍由核心状态机决定。
// 测试构造 ThreadState 时不启动窗口线程，绘制线程代码只在生产路径使用。
#![cfg_attr(test, allow(dead_code))]
use std::{
    collections::VecDeque,
    ffi::c_void,
    mem::size_of,
    ptr,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::w,
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM},
        Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject,
            AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
            DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
        },
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{
            HiDpi::{
                GetDpiForSystem, SetThreadDpiAwarenessContext,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
                KillTimer, PeekMessageW, PostThreadMessageW, RegisterClassW, SetTimer,
                SetWindowPos, ShowWindow, UpdateLayeredWindow, HTTRANSPARENT, HWND_TOPMOST,
                MA_NOACTIVATE, MSG, PM_NOREMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                SWP_SHOWWINDOW, SW_HIDE, ULW_ALPHA, WM_APP, WM_ERASEBKGND, WM_MOUSEACTIVATE,
                WM_NCCALCSIZE, WM_NCHITTEST, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
            },
        },
    },
};

const WAKE: u32 = WM_APP + 0x4b2;
/// Hook 到绘制线程的事件上限；超出时丢弃轨迹点，开始与结束仍保留。
const QUEUE_LIMIT: usize = 1024;
/// 单次手势保留的轨迹点上限；写满后隔点抽稀，形状不变而工作量有界。
const MAX_POINTS: usize = 2048;
/// 位图边长上限，超出部分不显示。
const MAX_EXTENT: i32 = 8192;
const FRAME: Duration = Duration::from_millis(12);
const FADE: Duration = Duration::from_millis(220);
const FRAME_TIMER: usize = 1;
const FADE_TIMER: usize = 2;
/// 绘制中与识别成功的颜色（薄荷绿），以及未匹配时的颜色（琥珀色）。
const ACTIVE: u32 = 0x83c8b5;
const UNMATCHED: u32 = 0xe3b977;
const OUTLINE: u32 = 0x10141a;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrailEvent {
    Begin { origin: (i32, i32) },
    Point((i32, i32)),
    End { matched: bool },
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
    queue: VecDeque<TrailEvent>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl State {
    fn push(&mut self, event: TrailEvent) {
        if self.closed {
            return;
        }
        if matches!(event, TrailEvent::Point(_)) && self.queue.len() >= QUEUE_LIMIT {
            return;
        }
        if !matches!(event, TrailEvent::Point(_)) && self.queue.len() >= QUEUE_LIMIT {
            // 开始与结束不能丢失；挤掉最旧的轨迹点。
            if let Some(index) = self
                .queue
                .iter()
                .position(|queued| matches!(queued, TrailEvent::Point(_)))
            {
                self.queue.remove(index);
            }
        }
        self.queue.push_back(event);
        if self.thread_id != 0 && !self.wake_posted {
            self.wake_posted =
                unsafe { PostThreadMessageW(self.thread_id, WAKE, WPARAM(0), LPARAM(0)) }.is_ok();
        }
    }
}

/// 绘制线程在首次创建时启动；销毁句柄只请求线程退出，不等待。
pub(crate) struct GestureTrail {
    shared: Arc<Shared>,
}

impl GestureTrail {
    pub(crate) fn new() -> Result<Self, String> {
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("omnitrigger-gesture-trail".into())
            .spawn(move || {
                let _ = std::panic::catch_unwind(|| worker(&worker_shared));
                let mut state = worker_shared.lock();
                state.closed = true;
                state.thread_id = 0;
                state.queue.clear();
            })
            .map_err(|error| format!("无法启动手势轨迹线程：{error}"))?;
        Ok(Self { shared })
    }

    /// Hook 同步路径调用：轨迹点只尝试加锁，锁被占用时直接丢弃。
    pub(crate) fn send(&self, event: TrailEvent) {
        match event {
            TrailEvent::Point(_) => {
                if let Ok(mut state) = self.shared.state.try_lock() {
                    state.push(event);
                }
            }
            _ => self.shared.lock().push(event),
        }
    }
}

impl Drop for GestureTrail {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.closed = true;
        if state.thread_id != 0 {
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    state.thread_id,
                    windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }
    }
}

/// 轨迹点与光栅化；纯计算，不创建窗口。
#[derive(Default)]
pub(crate) struct TrailRaster {
    points: Vec<(i32, i32)>,
}

/// 屏幕坐标中的绘制范围。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TrailFrame {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl TrailRaster {
    pub(crate) fn clear(&mut self) {
        self.points.clear();
    }

    pub(crate) fn push(&mut self, point: (i32, i32)) {
        if self.points.last() == Some(&point) {
            return;
        }
        if self.points.len() >= MAX_POINTS {
            // 保留首尾并隔点抽稀，轨迹形状保持连续。
            let last = self.points.len() - 1;
            let mut index = 0;
            self.points.retain(|_| {
                let keep = index % 2 == 0 || index == last;
                index += 1;
                keep
            });
        }
        self.points.push(point);
    }

    pub(crate) fn len(&self) -> usize {
        self.points.len()
    }

    /// 轨迹外包框加上边距；超出位图上限时以最新点为中心截取。
    pub(crate) fn frame(&self, margin: i32) -> Option<TrailFrame> {
        let first = *self.points.first()?;
        let (mut left, mut top, mut right, mut bottom) = (first.0, first.1, first.0, first.1);
        for &(x, y) in &self.points {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        let left = i64::from(left) - i64::from(margin);
        let top = i64::from(top) - i64::from(margin);
        let mut width = i64::from(right) + i64::from(margin) - left + 1;
        let mut height = i64::from(bottom) + i64::from(margin) - top + 1;
        let (mut left, mut top) = (left, top);
        let last = *self.points.last()?;
        let limit = i64::from(MAX_EXTENT);
        if width > limit {
            left = (i64::from(last.0) - limit / 2).clamp(left, left + width - limit);
            width = limit;
        }
        if height > limit {
            top = (i64::from(last.1) - limit / 2).clamp(top, top + height - limit);
            height = limit;
        }
        Some(TrailFrame {
            left: left.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            top: top.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            width: width as i32,
            height: height as i32,
        })
    }

    /// 写入 BGRA 预乘像素：先画深色描边以保证浅色背景上的可见性，再画主色。
    pub(crate) fn render(&self, frame: TrailFrame, pixels: &mut [u32], color: u32, radius: f32) {
        let (width, height) = (frame.width as usize, frame.height as usize);
        let pixels = &mut pixels[..width * height];
        pixels.fill(0);
        for (stroke, stroke_radius, opacity) in
            [(OUTLINE, radius + 1.5, 0.55_f32), (color, radius, 1.)]
        {
            let local = |(x, y): (i32, i32)| {
                (
                    (i64::from(x) - i64::from(frame.left)) as f32,
                    (i64::from(y) - i64::from(frame.top)) as f32,
                )
            };
            let mut previous = None;
            for &point in &self.points {
                let current = local(point);
                let start = previous.unwrap_or(current);
                capsule(
                    pixels,
                    width,
                    height,
                    start,
                    current,
                    stroke_radius,
                    stroke,
                    opacity,
                );
                previous = Some(current);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn capsule(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    a: (f32, f32),
    b: (f32, f32),
    radius: f32,
    color: u32,
    opacity: f32,
) {
    let reach = radius + 1.;
    let min_x = (a.0.min(b.0) - reach).floor().max(0.) as usize;
    let min_y = (a.1.min(b.1) - reach).floor().max(0.) as usize;
    let max_x = ((a.0.max(b.0) + reach).ceil() as i64).clamp(0, width as i64 - 1) as usize;
    let max_y = ((a.1.max(b.1) + reach).ceil() as i64).clamp(0, height as i64 - 1) as usize;
    if a.0.max(b.0) + reach < 0. || a.1.max(b.1) + reach < 0. || min_x > max_x || min_y > max_y {
        return;
    }
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let (red, green, blue) = (
        ((color >> 16) & 0xff) as f32,
        ((color >> 8) & 0xff) as f32,
        (color & 0xff) as f32,
    );
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = if length > 0. {
                (((px - a.0) * dx + (py - a.1) * dy) / length).clamp(0., 1.)
            } else {
                0.
            };
            let (cx, cy) = (a.0 + dx * t - px, a.1 + dy * t - py);
            let coverage = (radius + 0.5 - (cx * cx + cy * cy).sqrt()).clamp(0., 1.) * opacity;
            if coverage <= 0. {
                continue;
            }
            // 预乘的 source-over；同色重叠只增加边缘覆盖，不改变颜色。
            let pixel = &mut pixels[y * width + x];
            let keep = 1. - coverage;
            let channel = |shift: u32, value: f32| {
                let old = ((*pixel >> shift) & 0xff) as f32;
                ((value * coverage + old * keep).round().clamp(0., 255.) as u32) << shift
            };
            *pixel = channel(24, 255.) | channel(16, red) | channel(8, green) | channel(0, blue);
        }
    }
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
    let instance =
        unsafe { GetModuleHandleW(None) }.map_err(|error| format!("无法读取模块：{error}"))?;
    CLASS
        .get_or_init(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("OmniTrigger.GestureTrail"),
                ..Default::default()
            };
            if unsafe { RegisterClassW(&class) } == 0 {
                Err(format!(
                    "无法注册手势轨迹窗口：{}",
                    std::io::Error::last_os_error()
                ))
            } else {
                Ok(())
            }
        })
        .clone()?;
    unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT | WS_EX_TOPMOST,
            w!("OmniTrigger.GestureTrail"),
            w!("OmniTrigger · 手势轨迹"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .map(NativeWindow)
    .map_err(|error| format!("无法创建手势轨迹窗口：{error}"))
}

/// 只增不减的 DIB；同一绘制线程使用。
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    width: i32,
    height: i32,
}

impl Surface {
    fn new(width: i32, height: i32) -> Result<Self, String> {
        let mut surface = Self {
            dc: unsafe { CreateCompatibleDC(None) },
            bitmap: HBITMAP::default(),
            previous: HGDIOBJ::default(),
            bits: ptr::null_mut(),
            width,
            height,
        };
        if surface.dc.is_invalid() {
            return Err("无法创建手势轨迹离屏 DC。".into());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = ptr::null_mut();
        surface.bitmap = unsafe {
            CreateDIBSection(Some(surface.dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        }
        .map_err(|error| format!("无法创建手势轨迹位图：{error}"))?;
        if bits.is_null() {
            return Err("手势轨迹位图没有可写像素。".into());
        }
        surface.bits = bits.cast();
        surface.previous = unsafe { SelectObject(surface.dc, HGDIOBJ(surface.bitmap.0)) };
        Ok(surface)
    }

    fn pixels(&mut self) -> &mut [u32] {
        unsafe {
            std::slice::from_raw_parts_mut(self.bits, self.width as usize * self.height as usize)
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if !self.previous.is_invalid() {
                let _ = SelectObject(self.dc, self.previous);
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            }
            if !self.dc.is_invalid() {
                let _ = DeleteDC(self.dc);
            }
        }
    }
}

struct Worker {
    window: NativeWindow,
    surface: Option<Surface>,
    raster: TrailRaster,
    radius: f32,
    drawing: bool,
    dirty: bool,
    last_frame: Option<Instant>,
    frame_timer: bool,
    fade: Option<(Instant, TrailFrame, u32)>,
}

impl Worker {
    fn hide(&mut self) {
        self.stop_timers();
        self.fade = None;
        let _ = unsafe { ShowWindow(self.window.0, SW_HIDE) };
    }

    fn stop_timers(&mut self) {
        let _ = unsafe { KillTimer(Some(self.window.0), FRAME_TIMER) };
        let _ = unsafe { KillTimer(Some(self.window.0), FADE_TIMER) };
        self.frame_timer = false;
    }

    fn apply(&mut self, event: TrailEvent) -> Result<(), String> {
        match event {
            TrailEvent::Begin { origin } => {
                self.hide();
                self.raster.clear();
                self.raster.push(origin);
                self.drawing = true;
                self.dirty = true;
            }
            TrailEvent::Point(point) if self.drawing => {
                self.raster.push(point);
                self.dirty = true;
            }
            TrailEvent::Point(_) => {}
            TrailEvent::End { matched } => {
                if !self.drawing {
                    return Ok(());
                }
                self.drawing = false;
                self.dirty = false;
                let color = if matched { ACTIVE } else { UNMATCHED };
                if let Some(frame) = self.paint(color, 255)? {
                    self.fade = Some((Instant::now(), frame, color));
                    let _ = unsafe { KillTimer(Some(self.window.0), FRAME_TIMER) };
                    self.frame_timer = false;
                    unsafe { SetTimer(Some(self.window.0), FADE_TIMER, 16, None) };
                } else {
                    self.hide();
                }
            }
        }
        Ok(())
    }

    /// 绘制不超过每帧一次；过早的更新交给计时器合并。
    fn flush(&mut self) -> Result<(), String> {
        if !self.dirty || !self.drawing || self.raster.len() < 2 {
            return Ok(());
        }
        let now = Instant::now();
        if self
            .last_frame
            .is_some_and(|last| now.duration_since(last) < FRAME)
        {
            if !self.frame_timer {
                self.frame_timer = true;
                unsafe {
                    SetTimer(
                        Some(self.window.0),
                        FRAME_TIMER,
                        FRAME.as_millis() as u32,
                        None,
                    )
                };
            }
            return Ok(());
        }
        self.dirty = false;
        self.last_frame = Some(now);
        self.paint(ACTIVE, 255).map(|_| ())
    }

    fn paint(&mut self, color: u32, alpha: u8) -> Result<Option<TrailFrame>, String> {
        let margin = self.radius.ceil() as i32 + 3;
        let Some(frame) = self.raster.frame(margin) else {
            return Ok(None);
        };
        let fits = self
            .surface
            .as_ref()
            .is_some_and(|surface| surface.width >= frame.width && surface.height >= frame.height);
        if !fits {
            // 按 256 像素取整扩容，避免轨迹每增长一点就重建位图。
            let round = |value: i32| ((value + 255) / 256 * 256).min(MAX_EXTENT);
            let (old_w, old_h) = self
                .surface
                .as_ref()
                .map_or((0, 0), |surface| (surface.width, surface.height));
            self.surface = None;
            self.surface = Some(Surface::new(
                round(frame.width.max(old_w)),
                round(frame.height.max(old_h)),
            )?);
        }
        let surface = self.surface.as_mut().expect("位图已创建");
        let stride = surface.width;
        let pixels = surface.pixels();
        // 位图行宽可能大于本帧宽度：先在前部紧凑绘制，再按行宽展开。
        self.raster.render(frame, pixels, color, self.radius);
        let (width, height) = (frame.width as usize, frame.height as usize);
        if stride as usize != width {
            for row in (1..height).rev() {
                pixels.copy_within(row * width..row * width + width, row * stride as usize);
            }
        }
        self.present(frame, alpha)?;
        Ok(Some(frame))
    }

    fn present(&mut self, frame: TrailFrame, alpha: u8) -> Result<(), String> {
        let surface = self.surface.as_ref().expect("位图已创建");
        let destination = POINT {
            x: frame.left,
            y: frame.top,
        };
        let size = SIZE {
            cx: frame.width,
            cy: frame.height,
        };
        let source = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                self.window.0,
                None,
                Some(&destination),
                Some(&size),
                Some(surface.dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
        .map_err(|error| format!("无法绘制手势轨迹：{error}"))?;
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
        .map_err(|error| format!("无法显示手势轨迹：{error}"))
    }

    fn fade_step(&mut self) -> Result<(), String> {
        let Some((started, frame, _)) = self.fade else {
            let _ = unsafe { KillTimer(Some(self.window.0), FADE_TIMER) };
            return Ok(());
        };
        let elapsed = started.elapsed();
        if elapsed >= FADE {
            self.hide();
            return Ok(());
        }
        let remaining = 1. - elapsed.as_secs_f32() / FADE.as_secs_f32();
        self.present(frame, (remaining * 255.).round() as u8)
    }
}

fn worker(shared: &Shared) -> Result<(), String> {
    let mut message = MSG::default();
    let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if previous.0.is_null() {
        return Err("无法设置手势轨迹 DPI 上下文。".into());
    }
    let window = create_window()?;
    let scale = unsafe { GetDpiForSystem() }.max(96) as f32 / 96.;
    let mut worker = Worker {
        window,
        surface: None,
        raster: TrailRaster::default(),
        radius: 2.5 * scale,
        drawing: false,
        dirty: false,
        last_frame: None,
        frame_timer: false,
        fade: None,
    };
    {
        let mut state = shared.lock();
        state.thread_id = unsafe { GetCurrentThreadId() };
        if !state.queue.is_empty() && !state.wake_posted {
            state.wake_posted =
                unsafe { PostThreadMessageW(state.thread_id, WAKE, WPARAM(0), LPARAM(0)) }.is_ok();
        }
    }
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result == -1 {
            return Err("无法读取手势轨迹消息。".into());
        }
        if result == 0 {
            return Ok(());
        }
        let outcome = if message.message == WAKE {
            let events = {
                let mut state = shared.lock();
                state.wake_posted = false;
                std::mem::take(&mut state.queue)
            };
            events
                .into_iter()
                .try_for_each(|event| worker.apply(event))
                .and_then(|()| worker.flush())
        } else if message.message == WM_TIMER && message.hwnd == worker.window.0 {
            if message.wParam.0 == FRAME_TIMER {
                let _ = unsafe { KillTimer(Some(worker.window.0), FRAME_TIMER) };
                worker.frame_timer = false;
                worker.flush()
            } else {
                worker.fade_step()
            }
        } else {
            let _ = unsafe { DispatchMessageW(&message) };
            Ok(())
        };
        if outcome.is_err() {
            // 轨迹只是提示；绘制失败时收起本次轨迹，不影响识别与动作。
            worker.drawing = false;
            worker.hide();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(pixel: u32) -> u32 {
        pixel >> 24
    }

    #[test]
    fn frame_covers_the_trail_with_margin_and_caps_the_extent() {
        let mut raster = TrailRaster::default();
        assert_eq!(raster.frame(4), None);
        raster.push((10, 20));
        raster.push((-30, 60));
        assert_eq!(
            raster.frame(4),
            Some(TrailFrame {
                left: -34,
                top: 16,
                width: 49,
                height: 49,
            })
        );
        raster.push((20_000, 60));
        let frame = raster.frame(4).unwrap();
        assert_eq!(frame.width, MAX_EXTENT);
        assert!(frame.left + frame.width > 20_000);
    }

    #[test]
    fn point_buffer_stays_bounded_and_keeps_endpoints() {
        let mut raster = TrailRaster::default();
        for index in 0..10_000 {
            raster.push((index, 0));
        }
        assert!(raster.len() <= MAX_POINTS);
        assert_eq!(raster.points.first(), Some(&(0, 0)));
        assert_eq!(raster.points.last(), Some(&(9_999, 0)));
        raster.push((9_999, 0));
        assert_eq!(raster.points.last(), Some(&(9_999, 0)));
    }

    #[test]
    fn render_draws_an_opaque_core_with_soft_premultiplied_edges() {
        let mut raster = TrailRaster::default();
        raster.push((10, 10));
        raster.push((40, 10));
        let frame = raster.frame(6).unwrap();
        let mut pixels = vec![0xdead_beef; (frame.width * frame.height) as usize];
        raster.render(frame, &mut pixels, ACTIVE, 2.5);
        let at =
            |x: i32, y: i32| pixels[((y - frame.top) * frame.width + (x - frame.left)) as usize];
        assert_eq!(at(25, 10), 0xff00_0000 | ACTIVE);
        assert_eq!(alpha(at(25, frame.top)), 0);
        let edge = at(25, 13);
        assert!(alpha(edge) > 0 && alpha(edge) < 255, "{edge:08x}");
        for pixel in &pixels {
            let a = alpha(*pixel);
            assert!((pixel >> 16 & 0xff) <= a && (pixel >> 8 & 0xff) <= a && (pixel & 0xff) <= a);
        }
    }

    #[test]
    fn queue_keeps_begin_and_end_when_points_overflow() {
        let mut state = State::default();
        state.push(TrailEvent::Begin { origin: (0, 0) });
        for index in 0..(QUEUE_LIMIT as i32 * 2) {
            state.push(TrailEvent::Point((index, 0)));
        }
        state.push(TrailEvent::End { matched: true });
        assert_eq!(state.queue.len(), QUEUE_LIMIT);
        assert_eq!(
            state.queue.front(),
            Some(&TrailEvent::Begin { origin: (0, 0) })
        );
        assert_eq!(state.queue.back(), Some(&TrailEvent::End { matched: true }));
    }
}
