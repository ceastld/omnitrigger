//! 仅供单独进程中的串行原生测试使用，绝不加入已有窗口站。
use std::ffi::c_void;
use windows::Win32::System::Threading::GetCurrentThreadId;

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

pub struct PrivateClipboard {
    old_station: isize,
    old_desktop: isize,
    station: isize,
    desktop: isize,
}

impl PrivateClipboard {
    pub fn create() -> Self {
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

    pub fn verify(&self) {
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
