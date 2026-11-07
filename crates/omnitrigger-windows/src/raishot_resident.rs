//! 固定来源协议：隐藏 RustShot.MVP.Window、WM_APP+3、两个零参数。
//! 不启动进程、查询依赖、发送按键或开放任意窗口消息。

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaiShotResident {
    pub window_handle: u64,
    pub process_id: u32,
    pub thread_id: u32,
    pub process_created_file_time: u64,
}

pub const RAISHOT_RESIDENT_CLASS: &str = "RustShot.MVP.Window";
pub const RAISHOT_REGION_MESSAGE: u32 = 32771;

#[cfg(windows)]
mod native {
    use super::*;
    use std::{ffi::c_void, ptr};

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> isize;
        fn IsWindow(window: isize) -> i32;
        fn GetClassNameW(window: isize, value: *mut u16, count: i32) -> i32;
        fn GetWindowThreadProcessId(window: isize, pid: *mut u32) -> u32;
        fn PostMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetProcessTimes(
            handle: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn GetLastError() -> u32;
    }
    struct Process(*mut c_void);
    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn class_matches(window: isize) -> bool {
        let mut value = [0u16; 128];
        let size = unsafe { GetClassNameW(window, value.as_mut_ptr(), value.len() as i32) };
        size > 0
            && String::from_utf16_lossy(&value[..size as usize])
                .eq_ignore_ascii_case(RAISHOT_RESIDENT_CLASS)
    }

    fn inspect(window: isize) -> Result<RaiShotResident, &'static str> {
        if window <= 0 || unsafe { IsWindow(window) } == 0 || !class_matches(window) {
            return Err("raishot_resident_changed");
        }
        let mut pid = 0;
        let thread = unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid == 0 || thread == 0 {
            return Err("raishot_resident_changed");
        }
        let process = Process(unsafe { OpenProcess(0x1000, 0, pid) });
        if process.0.is_null() {
            return Err("raishot_identity_unavailable");
        }
        let (mut created, mut exit, mut kernel, mut user) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        if unsafe { GetProcessTimes(process.0, &mut created, &mut exit, &mut kernel, &mut user) }
            == 0
        {
            return Err("raishot_identity_unavailable");
        }
        let mut final_pid = 0;
        if unsafe { GetWindowThreadProcessId(window, &mut final_pid) } != thread
            || final_pid != pid
            || !class_matches(window)
        {
            return Err("raishot_resident_changed");
        }
        Ok(RaiShotResident {
            window_handle: window as u64,
            process_id: pid,
            thread_id: thread,
            process_created_file_time: ((created.high as u64) << 32) | created.low as u64,
        })
    }

    pub fn find() -> Result<Option<RaiShotResident>, &'static str> {
        let class: Vec<_> = RAISHOT_RESIDENT_CLASS.encode_utf16().chain([0]).collect();
        let window = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
        if window == 0 {
            return Ok(None);
        }
        inspect(window).map(Some)
    }

    pub fn region(
        expected: &RaiShotResident,
        mut check: impl FnMut() -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        check()?;
        if expected.window_handle == 0 || expected.window_handle > isize::MAX as u64 {
            return Err("invalid_input");
        }
        let window = expected.window_handle as isize;
        if inspect(window)? != *expected {
            return Err("raishot_resident_changed");
        }
        // Win32 不提供原子身份比较加投递；复验只能缩小竞争窗口。
        // 入队不证明送达、区域界面显示或截图完成。
        check()?;
        if unsafe { PostMessageW(window, RAISHOT_REGION_MESSAGE, 0, 0) } == 0 {
            return Err(if unsafe { GetLastError() } == 5 {
                "access_denied"
            } else {
                "raishot_request_failed"
            });
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use native::{find as find_raishot_resident, region as request_raishot_resident_region};
