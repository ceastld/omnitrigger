//! Win10+ 创建期 Job 关联；不在 spawn 后留下 AssignProcessToJobObject 竞态。
use super::{fault, Backend, Failure, Options, ReadChunk, CHUNK};
use std::{
    ffi::c_void,
    mem::size_of,
    ptr, thread,
    time::{Duration, Instant},
};

type Handle = *mut c_void;
#[repr(C)]
struct Security {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}
#[repr(C)]
struct Startup {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_bytes: u16,
    reserved_data: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}
#[repr(C)]
struct StartupEx {
    startup: Startup,
    attributes: *mut c_void,
}
#[repr(C)]
struct ProcessInfo {
    process: Handle,
    thread: Handle,
    pid: u32,
    thread_id: u32,
}
#[repr(C)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    minimum_working_set: usize,
    maximum_working_set: usize,
    active_limit: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}
#[repr(C)]
struct ExtendedLimits {
    basic: BasicLimits,
    io: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process: usize,
    peak_job: usize,
}
#[repr(C)]
struct Accounting {
    times: [i64; 4],
    faults: u32,
    total: u32,
    active: u32,
    terminated: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(security: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(
        job: Handle,
        class: i32,
        information: *const c_void,
        length: u32,
    ) -> i32;
    fn QueryInformationJobObject(
        job: Handle,
        class: i32,
        information: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
    fn TerminateJobObject(job: Handle, exit: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn CreatePipe(
        read: *mut Handle,
        write: *mut Handle,
        security: *const Security,
        size: u32,
    ) -> i32;
    fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        sharing: u32,
        security: *const Security,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn PeekNamedPipe(
        pipe: Handle,
        buffer: *mut c_void,
        length: u32,
        read: *mut u32,
        available: *mut u32,
        remaining: *mut u32,
    ) -> i32;
    fn ReadFile(
        handle: Handle,
        buffer: *mut c_void,
        length: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *mut c_void,
        length: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_security: *const Security,
        thread_security: *const Security,
        inherit: i32,
        flags: u32,
        environment: *const c_void,
        directory: *const u16,
        startup: *const Startup,
        information: *mut ProcessInfo,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn GetLastError() -> u32;
}

struct Owned(Handle);
impl Owned {
    fn valid(&self) -> bool {
        !self.0.is_null() && self.0 as usize != usize::MAX
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if self.valid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new() -> Result<Self, Failure> {
        let mut bytes = 0;
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 2, 0, &mut bytes);
        }
        if bytes == 0 || bytes > 64 * 1024 {
            return Err(fault("process_setup_failed", false));
        }
        let mut result = Self {
            storage: vec![0; bytes.div_ceil(size_of::<usize>())],
            initialized: false,
        };
        if unsafe { InitializeProcThreadAttributeList(result.pointer(), 2, 0, &mut bytes) } == 0 {
            // 初始化未成功，不能调用 DeleteProcThreadAttributeList。
            return Err(fault("process_setup_failed", false));
        }
        result.initialized = true;
        Ok(result)
    }
    fn pointer(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast()
    }
    fn set(&mut self, attribute: usize, handles: &mut [Handle]) -> Result<(), Failure> {
        if unsafe {
            UpdateProcThreadAttribute(
                self.pointer(),
                0,
                attribute,
                handles.as_mut_ptr().cast(),
                std::mem::size_of_val(handles),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(fault("process_setup_failed", false));
        }
        Ok(())
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                DeleteProcThreadAttributeList(self.pointer());
            }
        }
    }
}

fn pipe(security: &Security) -> Result<(Owned, Owned), Failure> {
    let mut read = ptr::null_mut();
    let mut write = ptr::null_mut();
    let ok = unsafe { CreatePipe(&mut read, &mut write, security, 0) };
    let read = Owned(read);
    let write = Owned(write);
    if ok == 0 || unsafe { SetHandleInformation(read.0, 1, 0) } == 0 {
        return Err(fault("process_setup_failed", false));
    }
    Ok((read, write))
}

struct Child {
    job: Option<Owned>,
    process: Owned,
    stdout: Owned,
    stderr: Owned,
}
impl Child {
    fn stop(&mut self) -> Result<(), Failure> {
        let Some(job) = self.job.take() else {
            return Ok(());
        };
        let began = Instant::now();
        let mut empty = false;
        if unsafe { TerminateJobObject(job.0, 1) } != 0 {
            while began.elapsed() < Duration::from_secs(1) {
                // 全部为整数/POD；只查询本调用私有 Job 的 active 数量。
                let mut accounting: Accounting = unsafe { std::mem::zeroed() };
                if unsafe {
                    QueryInformationJobObject(
                        job.0,
                        1,
                        (&mut accounting as *mut Accounting).cast(),
                        size_of::<Accounting>() as u32,
                        ptr::null_mut(),
                    )
                } == 0
                {
                    break;
                }
                if accounting.active == 0 {
                    empty = true;
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        // KILL_ON_JOB_CLOSE 也是 Drop/宿主强杀时的最后防线；句柄不继承给子进程。
        drop(job);
        let remaining = 1000_u128.saturating_sub(began.elapsed().as_millis()) as u32;
        let waited = unsafe { WaitForSingleObject(self.process.0, remaining) } == 0;
        if !empty || !waited {
            return Err(fault("process_cleanup_failed", true));
        }
        Ok(())
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        // 异常栈展开也关闭私有 Job；不无限等待，未成功收尾不会交付输出。
        if self.job.is_some() {
            let _ = self.stop();
        }
    }
}

pub(super) struct Native {
    epoch: Instant,
    child: Option<Child>,
}
impl Default for Native {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            child: None,
        }
    }
}

impl Backend for Native {
    fn start(&mut self, program: &str, args: &[String], options: &Options) -> Result<(), Failure> {
        let job = Owned(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) });
        if !job.valid() {
            return Err(fault("process_setup_failed", false));
        }
        let mut limits: ExtendedLimits = unsafe { std::mem::zeroed() };
        limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE，未允许 breakaway。
        if unsafe {
            SetInformationJobObject(
                job.0,
                9,
                (&limits as *const ExtendedLimits).cast(),
                size_of::<ExtendedLimits>() as u32,
            )
        } == 0
            || unsafe { SetHandleInformation(job.0, 1, 0) } == 0
        {
            return Err(fault("process_setup_failed", false));
        }
        let security = Security {
            length: size_of::<Security>() as u32,
            descriptor: ptr::null_mut(),
            inherit: 1,
        };
        let (stdout, out_write) = pipe(&security)?;
        let (stderr, err_write) = pipe(&security)?;
        let null_name: Vec<u16> = "NUL".encode_utf16().chain([0]).collect();
        let stdin = Owned(unsafe {
            CreateFileW(
                null_name.as_ptr(),
                0x80000000,
                3,
                &security,
                3,
                0x80,
                ptr::null_mut(),
            )
        });
        if !stdin.valid() {
            return Err(fault("process_setup_failed", false));
        }
        let mut attributes = Attributes::new()?;
        let mut inherited = [stdin.0, out_write.0, err_write.0];
        let mut jobs = [job.0];
        attributes.set(0x20002, &mut inherited)?; // PROC_THREAD_ATTRIBUTE_HANDLE_LIST
        attributes.set(0x2000d, &mut jobs)?; // PROC_THREAD_ATTRIBUTE_JOB_LIST（Win10+）
        let mut startup: StartupEx = unsafe { std::mem::zeroed() };
        startup.startup.cb = size_of::<StartupEx>() as u32;
        startup.startup.flags = 0x100; // STARTF_USESTDHANDLES
        startup.startup.stdin = stdin.0;
        startup.startup.stdout = out_write.0;
        startup.startup.stderr = err_write.0;
        startup.attributes = attributes.pointer();
        let application: Vec<u16> = program.encode_utf16().chain([0]).collect();
        let mut command = super::command_line(program, args);
        let directory = options
            .cwd
            .as_ref()
            .map(|cwd| cwd.encode_utf16().chain([0]).collect::<Vec<_>>());
        let mut info: ProcessInfo = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                0x08000000 | 0x00080000 | 0x00000400,
                ptr::null(),
                directory
                    .as_ref()
                    .map_or(ptr::null(), |value| value.as_ptr()),
                &startup.startup,
                &mut info,
            )
        };
        if ok == 0 {
            return Err(fault("process_start_failed", false));
        }
        let process = Owned(info.process);
        let _thread = Owned(info.thread);
        // CreateProcess 返回时已经关联 Job；不经过未受保护的 spawn -> assign 窗口。
        self.child = Some(Child {
            job: Some(job),
            process,
            stdout,
            stderr,
        });
        // 此函数退出即关闭父端写句柄和 stdin，EOF 只取决于子进程/Job 内后代。
        Ok(())
    }

    fn read(&mut self, stderr: bool) -> Result<ReadChunk, Failure> {
        let child = self
            .child
            .as_ref()
            .ok_or_else(|| fault("process_read_failed", true))?;
        let handle = if stderr {
            child.stderr.0
        } else {
            child.stdout.0
        };
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                handle,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &mut available,
                ptr::null_mut(),
            )
        } == 0
        {
            return if matches!(unsafe { GetLastError() }, 109 | 232) {
                Ok(ReadChunk {
                    bytes: vec![],
                    eof: true,
                })
            } else {
                Err(fault("process_read_failed", true))
            };
        }
        if available == 0 {
            return Ok(ReadChunk {
                bytes: vec![],
                eof: false,
            });
        }
        let mut bytes = vec![0; CHUNK.min(available as usize)];
        let mut count = 0;
        if unsafe {
            ReadFile(
                handle,
                bytes.as_mut_ptr().cast(),
                bytes.len() as u32,
                &mut count,
                ptr::null_mut(),
            )
        } == 0
        {
            return if matches!(unsafe { GetLastError() }, 109 | 232) {
                Ok(ReadChunk {
                    bytes: vec![],
                    eof: true,
                })
            } else {
                Err(fault("process_read_failed", true))
            };
        }
        bytes.truncate(count as usize);
        Ok(ReadChunk {
            bytes,
            eof: count == 0,
        })
    }
    fn exit_code(&mut self) -> Result<Option<i32>, Failure> {
        let child = self
            .child
            .as_ref()
            .ok_or_else(|| fault("process_wait_failed", true))?;
        match unsafe { WaitForSingleObject(child.process.0, 0) } {
            258 => Ok(None),
            0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(child.process.0, &mut code) } == 0 {
                    return Err(fault("process_wait_failed", true));
                }
                Ok(Some(code as i32))
            }
            _ => Err(fault("process_wait_failed", true)),
        }
    }
    fn milliseconds(&self) -> u64 {
        self.epoch
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
    fn wait(&mut self) {
        thread::sleep(Duration::from_millis(2));
    }
    fn finish(&mut self) -> Result<(), Failure> {
        if let Some(mut child) = self.child.take() {
            child.stop()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn win64_struct_layouts_match_documented_abi() {
        if cfg!(target_pointer_width = "64") {
            assert_eq!(size_of::<Security>(), 24);
            assert_eq!(size_of::<Startup>(), 104);
            assert_eq!(size_of::<StartupEx>(), 112);
            assert_eq!(size_of::<ExtendedLimits>(), 144);
            assert_eq!(size_of::<Accounting>(), 48);
        }
    }
}
