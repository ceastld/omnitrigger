//! 在单独窗口站验证真实 Win32 格式与内存交接，不访问交互桌面的剪贴板。
#![cfg(windows)]

use std::ffi::c_void;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Threading::GetCurrentThreadId,
        },
    },
};

#[link(name = "user32")]
unsafe extern "system" {
    fn CreateWindowStationW(
        name: *const u16,
        flags: u32,
        access: u32,
        security: *const c_void,
    ) -> isize;
    fn GetProcessWindowStation() -> isize;
    fn SetProcessWindowStation(station: isize) -> i32;
    fn CloseWindowStation(station: isize) -> i32;
    fn GetThreadDesktop(thread: u32) -> isize;
    fn SetThreadDesktop(desktop: isize) -> i32;
    fn CreateDesktopW(
        name: *const u16,
        device: *const u16,
        mode: *const c_void,
        flags: u32,
        access: u32,
        security: *const c_void,
    ) -> isize;
    fn CloseDesktop(desktop: isize) -> i32;
    fn GetUserObjectInformationW(
        object: isize,
        index: i32,
        info: *mut c_void,
        length: u32,
        needed: *mut u32,
    ) -> i32;
}

struct PrivateClipboard {
    old_station: isize,
    old_desktop: isize,
    station: isize,
    desktop: isize,
}

impl PrivateClipboard {
    fn create() -> Self {
        unsafe {
            let mut scope = Self {
                old_station: GetProcessWindowStation(),
                old_desktop: GetThreadDesktop(GetCurrentThreadId()),
                station: 0,
                desktop: 0,
            };
            // CREATE_ONLY + unique name: never join an unnamed station used by another probe.
            let station_name = format!(
                "OmniTriggerClipboardTest_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let wide: Vec<u16> = station_name.encode_utf16().chain([0]).collect();
            scope.station = CreateWindowStationW(wide.as_ptr(), 1, 0xF037F, std::ptr::null());
            assert_ne!(scope.station, 0, "{}", std::io::Error::last_os_error());
            assert_ne!(scope.station, scope.old_station);
            assert_ne!(SetProcessWindowStation(scope.station), 0);
            scope.desktop = CreateDesktopW(
                [b'D' as u16, b'e' as u16, b'f' as u16, 0].as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                0xF01FF,
                std::ptr::null(),
            );
            assert_ne!(scope.desktop, 0);
            assert_ne!(SetThreadDesktop(scope.desktop), 0);
            scope.verify();
            scope
        }
    }

    fn verify(&self) {
        #[repr(C)]
        struct Flags {
            inherit: i32,
            reserved: i32,
            flags: u32,
        }
        unsafe {
            let mut flags = Flags {
                inherit: 0,
                reserved: 0,
                flags: 0,
            };
            let mut needed = 0;
            assert_eq!(GetProcessWindowStation(), self.station);
            assert_eq!(GetThreadDesktop(GetCurrentThreadId()), self.desktop);
            assert_ne!(
                GetUserObjectInformationW(
                    self.station,
                    1,
                    (&mut flags as *mut Flags).cast(),
                    std::mem::size_of::<Flags>() as u32,
                    &mut needed
                ),
                0
            );
            assert_eq!(flags.flags & 1, 0, "Refusing an interactive window station");
        }
    }
}

impl Drop for PrivateClipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = SetThreadDesktop(self.old_desktop);
            let _ = SetProcessWindowStation(self.old_station);
            if self.desktop != 0 {
                let _ = CloseDesktop(self.desktop);
            }
            if self.station != 0 {
                let _ = CloseWindowStation(self.station);
            }
        }
    }
}

#[test]
#[ignore = "需要可创建命名窗口站的 Windows 管理员令牌；CI 显式运行，绝不回退交互剪贴板"]
fn native_exclusion_formats_are_present_before_later_plain_copy_clears_them() {
    let scope = PrivateClipboard::create();
    let names = [
        "OmniTrigger.TransientClipboard.v1",
        "ExcludeClipboardContentFromMonitorProcessing",
    ];
    let formats: Vec<u32> = names
        .iter()
        .map(|name| {
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let format = unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) };
            assert_ne!(format, 0);
            format
        })
        .collect();
    scope.verify();
    omnitrigger_windows::write_clipboard_text_excluding_history_if("literal 文😀", || Ok(()))
        .unwrap();
    assert_eq!(
        omnitrigger_windows::read_clipboard_text().unwrap(),
        "literal 文😀"
    );
    scope.verify();
    unsafe {
        OpenClipboard(None).unwrap();
        struct Close;
        impl Drop for Close {
            fn drop(&mut self) {
                let _ = unsafe { CloseClipboard() };
            }
        }
        let _close = Close;
        for format in &formats {
            IsClipboardFormatAvailable(*format).unwrap();
            let memory = HGLOBAL(GetClipboardData(*format).unwrap().0);
            assert!(GlobalSize(memory) >= 4);
            let pointer = GlobalLock(memory);
            assert!(!pointer.is_null());
            let payload = std::ptr::read_unaligned(pointer.cast::<u32>());
            let _ = GlobalUnlock(memory);
            assert_eq!(payload, 1);
        }
    }
    scope.verify();
    omnitrigger_windows::write_clipboard_text("later plain copy").unwrap();
    assert_eq!(
        omnitrigger_windows::read_clipboard_text().unwrap(),
        "later plain copy"
    );
    assert!(formats
        .iter()
        .all(|format| unsafe { IsClipboardFormatAvailable(*format) }.is_err()));
}
