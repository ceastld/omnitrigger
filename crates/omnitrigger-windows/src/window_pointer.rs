//! 同一交互会话/输入桌面、原目标仍为前台时的有界采样；不安装Hook或注入输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButtonState {
    Down,
    NotDownOrUnavailable,
}
impl PointerButtonState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::NotDownOrUnavailable => "not_down_or_unavailable",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowPointerState {
    pub x: i32,
    pub y: i32,
    pub left: PointerButtonState,
    pub right: PointerButtonState,
    pub middle: PointerButtonState,
}

#[cfg(any(windows, test))]
fn sampled_with(
    mut check: impl FnMut() -> Result<(), crate::WindowGeometryError>,
    read: impl FnOnce() -> Result<WindowPointerState, crate::WindowGeometryError>,
) -> Result<WindowPointerState, crate::WindowGeometryError> {
    check()?;
    let value = read()?;
    check()?;
    Ok(value)
}

#[cfg(any(windows, test))]
fn button_state(raw: i16) -> PointerButtonState {
    if raw < 0 {
        PointerButtonState::Down
    } else {
        PointerButtonState::NotDownOrUnavailable
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::{ScriptTarget, ScriptTargetSnapshot, WindowGeometryError};
    use std::{ffi::c_void, mem::size_of};
    use windows::Win32::{
        Foundation::{HANDLE, POINT},
        System::{
            RemoteDesktop::{
                ProcessIdToSessionId, WTSActive, WTSConnectState, WTSFreeMemory,
                WTSQuerySessionInformationW, WTS_CONNECTSTATE_CLASS,
            },
            StationsAndDesktops::{
                CloseDesktop, GetProcessWindowStation, GetThreadDesktop, GetUserObjectInformationW,
                OpenInputDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, DESKTOP_HOOKCONTROL,
                DESKTOP_READOBJECTS, HDESK, UOI_FLAGS, UOI_NAME, USEROBJECTFLAGS,
            },
            Threading::{GetCurrentProcessId, GetCurrentThreadId},
        },
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON},
            WindowsAndMessaging::GetPhysicalCursorPos,
        },
    };
    fn unavailable() -> WindowGeometryError {
        WindowGeometryError::new(
            "input_context_unavailable",
            "无法确认原目标、当前线程与活动输入桌面属于同一交互会话。",
            "not_started",
        )
    }
    struct InputDesktop(HDESK);
    impl Drop for InputDesktop {
        fn drop(&mut self) {
            let _ = unsafe { CloseDesktop(self.0) };
        }
    }
    fn name(handle: HANDLE) -> Result<Vec<u16>, WindowGeometryError> {
        let mut data = [0u16; 512];
        let mut needed = 0;
        unsafe {
            GetUserObjectInformationW(
                handle,
                UOI_NAME,
                Some(data.as_mut_ptr().cast::<c_void>()),
                size_of::<[u16; 512]>() as u32,
                Some(&mut needed),
            )
        }
        .map_err(|_| unavailable())?;
        if needed < 2 || needed as usize > size_of::<[u16; 512]>() || needed % 2 != 0 {
            return Err(unavailable());
        }
        let end = data[..needed as usize / 2]
            .iter()
            .position(|v| *v == 0)
            .ok_or_else(unavailable)?;
        if end == 0 {
            return Err(unavailable());
        }
        Ok(data[..end].to_vec())
    }
    fn active_session(session: u32) -> Result<(), WindowGeometryError> {
        struct Buffer(windows::core::PWSTR);
        impl Drop for Buffer {
            fn drop(&mut self) {
                if !self.0 .0.is_null() {
                    unsafe { WTSFreeMemory(self.0 .0.cast()) };
                }
            }
        }
        let mut pointer = windows::core::PWSTR::null();
        let mut size = 0;
        let result = unsafe {
            WTSQuerySessionInformationW(None, session, WTSConnectState, &mut pointer, &mut size)
        };
        let memory = Buffer(pointer);
        result.map_err(|_| unavailable())?;
        if memory.0 .0.is_null() || size as usize != size_of::<WTS_CONNECTSTATE_CLASS>() {
            return Err(unavailable());
        }
        let state =
            unsafe { std::ptr::read_unaligned(memory.0 .0.cast::<WTS_CONNECTSTATE_CLASS>()) };
        if state != WTSActive {
            return Err(unavailable());
        }
        Ok(())
    }
    fn context(snapshot: ScriptTargetSnapshot) -> Result<InputDesktop, WindowGeometryError> {
        let mut current_session = 0;
        let mut target_session = 0;
        unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut current_session) }
            .map_err(|_| unavailable())?;
        unsafe { ProcessIdToSessionId(snapshot.process_id, &mut target_session) }
            .map_err(|_| unavailable())?;
        if current_session == 0 || current_session != target_session {
            return Err(unavailable());
        }
        // OpenInputDesktop在断开会话中也可能返回重连后使用的桌面，因此另验WTSActive。
        active_session(current_session)?;
        let station = unsafe { GetProcessWindowStation() }.map_err(|_| unavailable())?;
        let mut flags = USEROBJECTFLAGS::default();
        unsafe {
            GetUserObjectInformationW(
                HANDLE(station.0),
                UOI_FLAGS,
                Some((&mut flags as *mut USEROBJECTFLAGS).cast()),
                size_of::<USEROBJECTFLAGS>() as u32,
                None,
            )
        }
        .map_err(|_| unavailable())?;
        // WSF_VISIBLE=1；只接受当前交互WinSta0，不切桌面、不创建或打开其它站。
        if flags.dwFlags & 1 == 0
            || !String::from_utf16(&name(HANDLE(station.0))?)
                .map_err(|_| unavailable())?
                .eq_ignore_ascii_case("WinSta0")
        {
            return Err(unavailable());
        }
        let input = InputDesktop(
            unsafe {
                OpenInputDesktop(
                    DESKTOP_CONTROL_FLAGS(0),
                    false,
                    DESKTOP_ACCESS_FLAGS(DESKTOP_READOBJECTS.0 | DESKTOP_HOOKCONTROL.0),
                )
            }
            .map_err(|_| unavailable())?,
        );
        let current =
            unsafe { GetThreadDesktop(GetCurrentThreadId()) }.map_err(|_| unavailable())?;
        let target = unsafe { GetThreadDesktop(snapshot.thread_id) }.map_err(|_| unavailable())?;
        let input_name = name(HANDLE(input.0 .0))?;
        if name(HANDLE(current.0))? != input_name || name(HANDLE(target.0))? != input_name {
            return Err(unavailable());
        }
        Ok(input)
    }
    pub(crate) fn ensure_context(target: &ScriptTarget) -> Result<(), WindowGeometryError> {
        target.ensure_foreground().map_err(|message| {
            WindowGeometryError::new("target_changed", message, "not_started")
        })?;
        let snapshot = target.snapshot().map_err(|message| {
            WindowGeometryError::new("target_changed", message, "not_started")
        })?;
        let _input = context(snapshot)?;
        Ok(())
    }
    pub fn read(
        target: &ScriptTarget,
        mut check: impl FnMut() -> Result<(), WindowGeometryError>,
    ) -> Result<WindowPointerState, WindowGeometryError> {
        sampled_with(
            || {
                check()?;
                ensure_context(target)?;
                check()
            },
            || {
                let mut point = POINT::default();
                unsafe { GetPhysicalCursorPos(&mut point) }.map_err(|_| {
                    WindowGeometryError::new(
                        "pointer_unavailable",
                        "物理屏幕光标位置当前不可读。",
                        "not_started",
                    )
                })?;
                // 只用高位。非负值不能区分抬起和GetAsyncKeyState失败；不声称物理设备/注入来源。
                Ok(WindowPointerState {
                    x: point.x,
                    y: point.y,
                    left: button_state(unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) }),
                    right: button_state(unsafe { GetAsyncKeyState(i32::from(VK_RBUTTON.0)) }),
                    middle: button_state(unsafe { GetAsyncKeyState(i32::from(VK_MBUTTON.0)) }),
                })
            },
        )
    }
}
#[cfg(windows)]
pub(crate) use native::ensure_context;
#[cfg(windows)]
pub use native::read as read_window_pointer_state_if;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn zero_and_low_bit_do_not_claim_release_or_injection_provenance() {
        assert_eq!(button_state(i16::MIN), PointerButtonState::Down);
        for raw in [0, 1, 127] {
            assert_eq!(button_state(raw), PointerButtonState::NotDownOrUnavailable);
        }
    }
    #[test]
    fn unavailable_session_or_late_target_change_never_publishes_a_snapshot() {
        for stop in [1, 2] {
            let calls = Cell::new(0);
            let reads = Cell::new(0);
            let error = sampled_with(
                || {
                    calls.set(calls.get() + 1);
                    if calls.get() == stop {
                        Err(crate::WindowGeometryError::new(
                            "input_context_unavailable",
                            "合成会话变化",
                            "not_started",
                        ))
                    } else {
                        Ok(())
                    }
                },
                || {
                    reads.set(reads.get() + 1);
                    Ok(WindowPointerState {
                        x: -1200,
                        y: 30,
                        left: PointerButtonState::Down,
                        right: PointerButtonState::NotDownOrUnavailable,
                        middle: PointerButtonState::NotDownOrUnavailable,
                    })
                },
            )
            .unwrap_err();
            assert_eq!(reads.get(), if stop == 1 { 0 } else { 1 });
            assert_eq!(error.effect_state, "not_started");
        }
    }
}
