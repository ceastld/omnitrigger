//! 仅供单独进程中的串行原生测试使用，绝不加入已有窗口站。
use std::{ffi::c_void, io, io::Write};
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

#[repr(C)]
struct Flags {
    inherit: i32,
    reserved: i32,
    flags: u32,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn object_name(object: isize) -> io::Result<String> {
    let mut needed = 0;
    // UOI_NAME 的长度单位为字节；固定上限、至多两次查询，无重试循环。
    unsafe { GetUserObjectInformationW(object, 2, std::ptr::null_mut(), 0, &mut needed) };
    if !(2..=4096).contains(&needed) || needed % 2 != 0 {
        return Err(invalid("UOI_NAME length invalid"));
    }
    let capacity = needed;
    let mut buffer = vec![0u16; capacity as usize / 2];
    if unsafe {
        GetUserObjectInformationW(object, 2, buffer.as_mut_ptr().cast(), capacity, &mut needed)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if needed < 2 || needed > capacity || needed % 2 != 0 {
        return Err(invalid("UOI_NAME result truncated"));
    }
    let used = needed as usize / 2;
    if buffer[used - 1] != 0 || buffer[..used - 1].contains(&0) {
        return Err(invalid("UOI_NAME terminator invalid"));
    }
    let name =
        String::from_utf16(&buffer[..used - 1]).map_err(|_| invalid("UOI_NAME UTF16 invalid"))?;
    if name.is_empty() {
        return Err(invalid("UOI_NAME empty"));
    }
    Ok(name)
}

fn private_station_name(station: isize, original_name: &str) -> io::Result<String> {
    let mut flags = Flags {
        inherit: 0,
        reserved: 0,
        flags: 0,
    };
    let mut needed = 0;
    if unsafe {
        GetUserObjectInformationW(
            station,
            1,
            (&mut flags as *mut Flags).cast(),
            std::mem::size_of::<Flags>() as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if needed != std::mem::size_of::<Flags>() as u32 || flags.flags & 1 != 0 {
        return Err(invalid("Refusing visible or unverified window station"));
    }
    let name = object_name(station)?;
    if name.eq_ignore_ascii_case("WinSta0") || name.eq_ignore_ascii_case(original_name) {
        return Err(invalid("Refusing original or interactive station name"));
    }
    Ok(name)
}

#[derive(Debug)]
pub struct PrivateClipboard {
    old_station: isize,
    old_desktop: isize,
    station: isize,
    desktop: isize,
    old_station_name: String,
    old_desktop_name: String,
    station_name: String,
    owner_thread: u32,
    active: bool,
}

impl PrivateClipboard {
    #[allow(dead_code)] // station-only 探针只选择显式 unnamed 入口。
    pub fn create() -> Self {
        let station_name = format!(
            "OmniTriggerClipboardTest_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let wide: Vec<u16> = station_name.encode_utf16().chain([0]).collect();
        Self::create_with_name(Some(&wide)).expect("named private station creation failed")
    }

    /// 显式 NULL + CWF_CREATE_ONLY；已存在或权限不足即返回错误，不加入、不回退。
    #[allow(dead_code)] // 现有 named 测试不自动改为此环境适配入口。
    pub fn create_unnamed_create_only() -> io::Result<Self> {
        Self::create_with_name(None)
    }

    fn create_with_name(name: Option<&[u16]>) -> io::Result<Self> {
        unsafe {
            let old_station = GetProcessWindowStation();
            if old_station == 0 {
                return Err(io::Error::last_os_error());
            }
            let owner_thread = GetCurrentThreadId();
            let old_desktop = GetThreadDesktop(owner_thread);
            if old_desktop == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut scope = Self {
                old_station,
                old_desktop,
                station: 0,
                desktop: 0,
                old_station_name: object_name(old_station)?,
                old_desktop_name: object_name(old_desktop)?,
                station_name: String::new(),
                owner_thread,
                active: true,
            };
            let result = (|| -> io::Result<()> {
                scope.station = CreateWindowStationW(
                    name.map_or(std::ptr::null(), |wide| wide.as_ptr()),
                    1, // CWF_CREATE_ONLY，两个入口都不打开已存在窗口站。
                    0xF037F,
                    std::ptr::null(),
                );
                if scope.station == 0 {
                    return Err(io::Error::last_os_error());
                }
                if scope.station == scope.old_station {
                    // 身份不符时绝不把借用的原句柄放进关闭路径。
                    scope.station = 0;
                    return Err(invalid("Station handle equals original"));
                }
                // 必须先确认不可见及实际名称，再改变进程的 station。
                scope.station_name = private_station_name(scope.station, &scope.old_station_name)?;
                if GetProcessWindowStation() != scope.old_station {
                    return Err(invalid("Process station changed before explicit selection"));
                }
                if SetProcessWindowStation(scope.station) == 0 {
                    return Err(io::Error::last_os_error());
                }
                scope.desktop = CreateDesktopW(
                    [b'D' as u16, b'e' as u16, b'f' as u16, 0].as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    0xF01FF,
                    std::ptr::null(),
                );
                if scope.desktop == 0 {
                    return Err(io::Error::last_os_error());
                }
                if scope.desktop == scope.old_desktop {
                    scope.desktop = 0;
                    return Err(invalid("Desktop handle equals original"));
                }
                if object_name(scope.desktop)? != "Def" {
                    return Err(invalid("Private desktop identity invalid"));
                }
                if SetThreadDesktop(scope.desktop) == 0 {
                    return Err(io::Error::last_os_error());
                }
                scope.try_verify()
            })();
            if let Err(error) = result {
                if let Err(cleanup) = scope.restore_and_close() {
                    return Err(io::Error::other(format!(
                        "creation failed ({error}); cleanup unconfirmed ({cleanup})"
                    )));
                }
                return Err(error);
            }
            Ok(scope)
        }
    }

    pub fn verify(&self) {
        self.try_verify()
            .expect("private station/desktop verification failed");
    }

    fn try_verify(&self) -> io::Result<()> {
        unsafe {
            if !self.active
                || self.station == 0
                || self.desktop == 0
                || GetCurrentThreadId() != self.owner_thread
                || GetProcessWindowStation() != self.station
                || GetThreadDesktop(self.owner_thread) != self.desktop
                || private_station_name(self.station, &self.old_station_name)? != self.station_name
                || object_name(self.desktop)? != "Def"
            {
                return Err(invalid("Private station/desktop identity changed"));
            }
        }
        Ok(())
    }

    /// 固定次数恢复/关闭，无等待或重试循环；不关闭借用的原 station/desktop。
    pub fn restore_and_close(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        unsafe {
            if GetCurrentThreadId() != self.owner_thread {
                return Err(invalid("Private scope must close on its creating thread"));
            }
            // SetThreadDesktop 的目标须属于当前 process station，故 station 必须先恢复。
            if GetProcessWindowStation() != self.old_station
                && SetProcessWindowStation(self.old_station) == 0
            {
                return Err(io::Error::last_os_error());
            }
            if GetThreadDesktop(self.owner_thread) != self.old_desktop
                && SetThreadDesktop(self.old_desktop) == 0
            {
                return Err(io::Error::last_os_error());
            }
            if GetProcessWindowStation() != self.old_station
                || GetThreadDesktop(self.owner_thread) != self.old_desktop
                || object_name(self.old_station)? != self.old_station_name
                || object_name(self.old_desktop)? != self.old_desktop_name
            {
                return Err(invalid("Original station/desktop restoration unconfirmed"));
            }
            if self.desktop != 0 {
                if CloseDesktop(self.desktop) == 0 {
                    return Err(io::Error::last_os_error());
                }
                self.desktop = 0;
            }
            if self.station != 0 {
                if CloseWindowStation(self.station) == 0 {
                    return Err(io::Error::last_os_error());
                }
                self.station = 0;
            }
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for PrivateClipboard {
    fn drop(&mut self) {
        if self.active {
            if let Err(error) = self.restore_and_close() {
                let _ = writeln!(
                    io::stderr(),
                    "[private-station] restoration/close unconfirmed: {error}"
                );
            }
        }
    }
}
