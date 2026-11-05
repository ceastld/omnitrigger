//! 真实OS所有权验收源码：仅强制终止本测试spawn的controller，不访问剪贴板。
#![cfg(windows)]

#[path = "../src/clipboard_history_child_job.rs"]
mod job_support;
use job_support::{await_capture_gate, HistoryChildJob, CAPTURE_GATE};
use std::ffi::c_void;
use std::io::{Read, Write};
use std::os::windows::{
    io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::CommandExt,
};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    fn GetTickCount64() -> u64;
    fn PeekNamedPipe(
        pipe: *mut c_void,
        buffer: *mut c_void,
        size: u32,
        read: *mut u32,
        available: *mut u32,
        left: *mut u32,
    ) -> i32;
}

const ROLE: &str = "OMNITRIGGER_OWNED_JOB_FIXTURE_ROLE";

/// 测试父进程唯一能终止的对象，是直接spawn返回的controller Child。
struct Controller(Child);
impl Drop for Controller {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn fixture_command(test: &str, role: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            test,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ROLE, role)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000);
    command
}

fn marker(child: &mut Child, prefix: &str) -> (u32, u64) {
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut bytes = Vec::new();
    loop {
        if let Some(start) = bytes
            .windows(prefix.len())
            .position(|part| part == prefix.as_bytes())
        {
            let tail = &bytes[start + prefix.len()..];
            if let Some(end) = tail.iter().position(|byte| *byte == b'\n') {
                let line = std::str::from_utf8(&tail[..end]).unwrap().trim();
                let (pid, deadline) = line.split_once(':').unwrap();
                return (pid.parse().unwrap(), deadline.parse().unwrap());
            }
        }
        assert!(Instant::now() < deadline, "受控Job协议fixture握手超时");
        assert!(bytes.len() < 4096, "fixture输出越界");
        let output = child.stdout.as_mut().unwrap();
        let mut available = 0;
        assert_ne!(
            unsafe {
                PeekNamedPipe(
                    output.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            },
            0,
            "fixture提前关闭stdout"
        );
        if available > 0 {
            let mut buffer = [0; 256];
            let count = (available as usize)
                .min(buffer.len())
                .min(4096 - bytes.len());
            let read = output.read(&mut buffer[..count]).unwrap();
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
        } else {
            assert!(child.try_wait().unwrap().is_none(), "fixture提前退出");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn pin_observer(pid: u32) -> OwnedHandle {
    // 仅SYNCHRONIZE|QUERY_LIMITED_INFORMATION，无PROCESS_TERMINATE。
    // controller仍持有原Child句柄，观察句柄在杀controller之前打开，避免按旧PID补杀。
    let raw = unsafe { OpenProcess(0x0010_0000 | 0x1000, 0, pid) };
    assert!(!raw.is_null(), "不能观察自有fixture进程");
    unsafe { OwnedHandle::from_raw_handle(raw) }
}

#[test]
#[ignore = "显式串行真实OS验收；强制退出本次controller后Job须在自然期限前终止阻塞合成helper"]
fn parent_forced_exit_closes_private_job_and_terminates_blocked_fixture() {
    let mut controller = Controller(
        fixture_command("fixture_controller", "bound")
            .spawn()
            .unwrap(),
    );
    let (pid, natural_deadline) = marker(&mut controller.0, "OTJOB_CONTROL_BOUND:");
    let observer = pin_observer(pid);
    assert_eq!(
        unsafe { WaitForSingleObject(observer.as_raw_handle(), 0) },
        0x102
    );
    // 至少还剩20秒自然寿命，测试只给5秒：不能用自然退出伪证Job生效。
    assert!(unsafe { GetTickCount64() } + 20_000 < natural_deadline);
    controller.0.kill().unwrap();
    controller.0.wait().unwrap();
    assert_eq!(
        unsafe { WaitForSingleObject(observer.as_raw_handle(), 5000) },
        0,
        "父退出后合成helper仍未终止；不得按记录PID补杀"
    );
    assert!(unsafe { GetTickCount64() } < natural_deadline);
}

#[test]
#[ignore = "显式串行真实OS验收；未绑定/未授权窗口父退出时helper必须因stdin EOF退出而不进入阻塞体"]
fn parent_exit_before_assignment_gated_fixture_exits_without_entering_body() {
    let mut controller = Controller(
        fixture_command("fixture_controller", "unbound")
            .spawn()
            .unwrap(),
    );
    let (pid, deadline) = marker(&mut controller.0, "OTJOB_CONTROL_UNBOUND:");
    assert_eq!(deadline, 0);
    let observer = pin_observer(pid);
    assert_eq!(
        unsafe { WaitForSingleObject(observer.as_raw_handle(), 0) },
        0x102
    );
    controller.0.kill().unwrap();
    controller.0.wait().unwrap();
    assert_eq!(
        unsafe { WaitForSingleObject(observer.as_raw_handle(), 5000) },
        0,
        "未授权helper未因EOF退出；不允许按记录PID补杀"
    );
}

#[test]
#[ignore = "内部受控controller fixture；无显式ROLE时不创建任何进程"]
fn fixture_controller() {
    let role = std::env::var(ROLE).unwrap_or_default();
    if role != "bound" && role != "unbound" {
        return;
    }
    let job = HistoryChildJob::new().unwrap();
    let mut child = fixture_command("fixture_child", "child").spawn().unwrap();
    let (pid, _) = marker(&mut child, "OTJOB_CHILD_READY:");
    assert_eq!(pid, child.id());
    if role == "bound" {
        job.assign(&child).unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(CAPTURE_GATE)
            .unwrap();
        child.stdin.as_mut().unwrap().flush().unwrap();
        let (pid, natural_deadline) = marker(&mut child, "OTJOB_CHILD_BLOCKED:");
        println!("\nOTJOB_CONTROL_BOUND:{pid}:{natural_deadline}");
    } else {
        // 故意停在spawn与assign之间；不发送任何Capture授权。
        println!("\nOTJOB_CONTROL_UNBOUND:{pid}:0");
    }
    std::io::stdout().flush().unwrap();
    let mut hold = [0];
    let _ = std::io::stdin().read_exact(&mut hold);
    // 正常退出/测试中断时也先关闭Job；真正强制退出路径不会运行Rust Drop。
    drop(job);
    child.stdin.take();
    let _ = child.wait();
}

#[test]
#[ignore = "内部合成helper fixture；无显式ROLE时不访问系统或进入阻塞体"]
fn fixture_child() {
    if std::env::var(ROLE).as_deref() != Ok("child") {
        return;
    }
    println!("\nOTJOB_CHILD_READY:{}:0", std::process::id());
    std::io::stdout().flush().unwrap();
    if await_capture_gate(&mut std::io::stdin().lock()).is_err() {
        return;
    }
    let natural_deadline = unsafe { GetTickCount64() } + 60_000;
    println!(
        "\nOTJOB_CHILD_BLOCKED:{}:{natural_deadline}",
        std::process::id()
    );
    std::io::stdout().flush().unwrap();
    // 模拟已进入不可被stdin EOF中断的系统调用；完全不读剪贴板。
    std::thread::sleep(Duration::from_secs(60));
}
