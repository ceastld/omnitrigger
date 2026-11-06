//! 内部 helper 的启动形态检查；不是同用户本机恶意代码的认证边界。
use std::{ffi::c_void, mem::size_of, ptr};
type Handle = *mut c_void;
#[repr(C)]
struct ProcessEntry {
    size: u32,
    usage: u32,
    pid: u32,
    heap: usize,
    module: u32,
    threads: u32,
    parent: u32,
    priority: i32,
    flags: u32,
    exe: [u16; 260],
}
#[repr(C)]
#[derive(Default)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    min_working: usize,
    max_working: usize,
    active: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}
#[repr(C)]
#[derive(Default)]
struct ExtendedLimits {
    basic: BasicLimits,
    io: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process: usize,
    peak_job: usize,
}
#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentProcessId() -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        image: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn GetProcessTimes(
        process: Handle,
        created: *mut FileTime,
        exited: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn GetStdHandle(which: u32) -> Handle;
    fn GetFileType(handle: Handle) -> u32;
    fn IsProcessInJob(process: Handle, job: Handle, result: *mut i32) -> i32;
    fn QueryInformationJobObject(
        job: Handle,
        class: i32,
        information: *mut c_void,
        size: u32,
        returned: *mut u32,
    ) -> i32;
}
struct Owned(Handle);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn created(process: Handle) -> Option<u64> {
    let (mut c, mut e, mut k, mut u) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    if unsafe { GetProcessTimes(process, &mut c, &mut e, &mut k, &mut u) } == 0 {
        return None;
    }
    Some((u64::from(c.high) << 32) | u64::from(c.low))
}
fn shape(parent_matches: bool, flags: u32, out_pipe: bool, err_pipe: bool) -> bool {
    parent_matches && flags & 0x2000 != 0 && flags & (0x800 | 0x1000) == 0 && out_pipe && err_pipe
}
pub(super) fn owned_capture_child() -> bool {
    // 当前 Job 必须 KILL_ON_JOB_CLOSE，且不能允许 breakaway；只接受私有输出管道。
    let current = unsafe { GetCurrentProcess() };
    let mut in_job = 0;
    if unsafe { IsProcessInJob(current, ptr::null_mut(), &mut in_job) } == 0 || in_job == 0 {
        return false;
    }
    let mut limits = ExtendedLimits::default();
    if unsafe {
        QueryInformationJobObject(
            ptr::null_mut(),
            9,
            (&mut limits as *mut ExtendedLimits).cast(),
            size_of::<ExtendedLimits>() as u32,
            ptr::null_mut(),
        )
    } == 0
    {
        return false;
    }
    let pipe = |which| {
        let h = unsafe { GetStdHandle(which) };
        !h.is_null() && h as isize != -1 && unsafe { GetFileType(h) } == 3
    };
    if !shape(
        true,
        limits.basic.flags,
        pipe(-11i32 as u32),
        pipe(-12i32 as u32),
    ) {
        return false;
    }
    let snapshot = unsafe { CreateToolhelp32Snapshot(2, 0) };
    if snapshot.is_null() || snapshot as isize == -1 {
        return false;
    }
    let snapshot = Owned(snapshot);
    let mut entry: ProcessEntry = unsafe { std::mem::zeroed() };
    entry.size = size_of::<ProcessEntry>() as u32;
    let own_pid = unsafe { GetCurrentProcessId() };
    let mut parent = 0;
    let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    // 有界检查进程元数据，不读命令行、环境或第三方进程内存。
    for _ in 0..65536 {
        if !found {
            break;
        }
        if entry.pid == own_pid {
            parent = entry.parent;
            break;
        }
        found = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    if parent == 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(0x1000, 0, parent) };
    if handle.is_null() {
        return false;
    }
    let parent = Owned(handle);
    let mut image = [0u16; 32768];
    let mut length = image.len() as u32;
    if unsafe { QueryFullProcessImageNameW(parent.0, 0, image.as_mut_ptr(), &mut length) } == 0 {
        return false;
    }
    let Ok(path) = String::from_utf16(&image[..length as usize]) else {
        return false;
    };
    let Ok(own) = std::env::current_exe() else {
        return false;
    };
    // 同一 exe 的应用进程发起；Node、PowerShell、cmd 直接启动均不满足。
    let Some(own) = own.to_str() else {
        return false;
    };
    let matching = path.eq_ignore_ascii_case(own)
        && matches!((created(parent.0),created(current)),(Some(p),Some(c)) if p <= c);
    shape(matching, limits.basic.flags, true, true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_requires_same_parent_bounded_job_and_both_pipes() {
        assert!(shape(true, 0x2000, true, true));
        for input in [
            (false, 0x2000, true, true),
            (true, 0, true, true),
            (true, 0x2800, true, true),
            (true, 0x3000, true, true),
            (true, 0x2000, false, true),
            (true, 0x2000, true, false),
        ] {
            assert!(!shape(input.0, input.1, input.2, input.3));
        }
    }
}
