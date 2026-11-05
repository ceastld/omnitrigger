//! 两类自有历史helper的父进程生命周期边界；不执行采集，不安装服务。

use std::ffi::c_void;
use std::io::{self, Read};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::Child;

pub(crate) const CAPTURE_GATE: &[u8; 8] = b"OTHCAP01";

#[repr(C)]
#[derive(Default)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    minimum_working_set: usize,
    maximum_working_set: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}
#[repr(C)]
#[derive(Default)]
struct IoCounters {
    reads: u64,
    writes: u64,
    other: u64,
    read_bytes: u64,
    write_bytes: u64,
    other_bytes: u64,
}
#[repr(C)]
#[derive(Default)]
struct ExtendedLimits {
    basic: BasicLimits,
    io: IoCounters,
    process_memory: usize,
    job_memory: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(security: *const c_void, name: *const u16) -> *mut c_void;
    fn SetInformationJobObject(
        job: *mut c_void,
        class: i32,
        info: *const c_void,
        length: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    fn SetHandleInformation(handle: *mut c_void, mask: u32, flags: u32) -> i32;
}

/// 未命名、不可继承，只有父进程保存句柄；最后句柄关闭时由OS终止关联进程。
pub(crate) struct HistoryChildJob(OwnedHandle);
impl HistoryChildJob {
    pub(crate) fn new() -> io::Result<Self> {
        // NULL SECURITY_ATTRIBUTES本身就禁用继承；再明确清除继承位，任一步失败均关闭。
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        if unsafe { SetHandleInformation(job.0.as_raw_handle(), 1, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let limits = ExtendedLimits {
            basic: BasicLimits {
                flags: 0x0000_2000,
                ..BasicLimits::default()
            }, // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            ..ExtendedLimits::default()
        };
        if unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                9,
                (&limits as *const ExtendedLimits).cast(),
                std::mem::size_of::<ExtendedLimits>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
    pub(crate) fn assign(&self, child: &Child) -> io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// 一次性helper在触碰scope/剪贴板前等待父进程授权；未绑定窗口父退出只得到EOF。
pub(crate) fn await_capture_gate(input: &mut impl Read) -> io::Result<()> {
    let mut gate = [0; CAPTURE_GATE.len()];
    input.read_exact(&mut gate)?;
    if &gate != CAPTURE_GATE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "history_capture_gate",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_gate_accepts_only_complete_fixed_authorization_and_rejects_eof() {
        assert!(await_capture_gate(&mut &CAPTURE_GATE[..]).is_ok());
        for count in 0..CAPTURE_GATE.len() {
            assert_eq!(
                await_capture_gate(&mut &CAPTURE_GATE[..count])
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
        assert_eq!(
            await_capture_gate(&mut &b"WRONG001"[..])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn job_extended_limit_layout_matches_windows_abi() {
        assert_eq!(
            std::mem::size_of::<ExtendedLimits>(),
            if cfg!(target_pointer_width = "64") {
                144
            } else {
                112
            }
        );
        assert_eq!(std::mem::align_of::<ExtendedLimits>(), 8);
    }
}
