//! 用专用注入标记验证真实 LL Hook 链顺序；竞争 Hook 吞掉全部探针，硬件输入照常透传。

use std::{
    mem::size_of,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use omnitrigger_windows::{
    Backend, BackendCommand, BackendEvent, ListenerDiagnostics, PRIORITY_PROBE_TAG,
};
use windows::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, PeekMessageW, SetWindowsHookExW, TranslateMessage,
            UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PM_REMOVE,
            WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
        },
    },
};

const F24: u16 = 0x87;
static COMPETITOR_EVENTS: AtomicU64 = AtomicU64::new(0);

struct Competitor(HHOOK);

impl Competitor {
    fn install() -> Result<Self, String> {
        pump_messages();
        let module = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
        unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(competitor_hook),
                Some(HINSTANCE(module.0)),
                0,
            )
        }
        .map(Self)
        .map_err(|error| format!("无法安装竞争 Hook：{error}"))
    }
}

impl Drop for Competitor {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

unsafe extern "system" fn competitor_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam.0 as u32, WM_KEYDOWN | WM_KEYUP) {
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if event.vkCode == u32::from(F24)
            && event.flags.contains(LLKHF_INJECTED)
            && event.dwExtraInfo == PRIORITY_PROBE_TAG
        {
            COMPETITOR_EVENTS.fetch_add(1, Ordering::Relaxed);
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn pump_messages() {
    let mut message = MSG::default();
    unsafe {
        // LL 回调通过发往安装线程的消息执行；PeekMessage 同时调度这些 sent message。
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn with_message_pump<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let _ = tx.send(work());
    });
    loop {
        pump_messages();
        match rx.try_recv() {
            Ok(result) => {
                worker
                    .join()
                    .map_err(|_| "探针工作线程异常退出".to_string())?;
                return Ok(result);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let _ = worker.join();
                return Err("探针工作线程在返回结果前异常退出".into());
            }
            Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
        }
    }
}

struct PumpedBackend {
    backend: Option<Backend>,
    events: mpsc::Receiver<BackendEvent>,
}

impl PumpedBackend {
    fn start() -> Result<Self, String> {
        let (backend, events) = with_message_pump(Backend::start)??;
        let mut instance = Self {
            backend: Some(backend),
            events,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            pump_messages();
            match instance.events.try_recv() {
                Ok(BackendEvent::Ready) => {
                    instance.send(BackendCommand::SetEnabled(false))?;
                    return Ok(instance);
                }
                Ok(event) => return Err(format!("等待就绪时收到意外事件：{event:?}")),
                Err(mpsc::TryRecvError::Disconnected) => return Err("输入线程已断开".into()),
                Err(mpsc::TryRecvError::Empty) if Instant::now() >= deadline => {
                    return Err("输入线程就绪超时".into())
                }
                Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
            }
        }
    }

    fn send(&mut self, command: BackendCommand) -> Result<(), String> {
        self.backend
            .as_ref()
            .ok_or("输入线程已经退出")?
            .send(command)
    }

    fn diagnostics(&mut self) -> Result<ListenerDiagnostics, String> {
        self.send(BackendCommand::GetDiagnostics)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            pump_messages();
            match self.events.try_recv() {
                Ok(BackendEvent::Diagnostics(snapshot)) => return Ok(*snapshot),
                Ok(event) => return Err(format!("读取快照时收到意外事件：{event:?}")),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("读取快照时输入线程已断开".into())
                }
                Err(mpsc::TryRecvError::Empty) if Instant::now() >= deadline => {
                    return Err("读取快照超时".into())
                }
                Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
            }
        }
    }
}

impl Drop for PumpedBackend {
    fn drop(&mut self) {
        if let Some(backend) = self.backend.take() {
            // 即便错误退出，等待 Backend::drop 时也持续调度竞争 Hook，避免互相等待。
            let _ = with_message_pump(move || drop(backend));
        }
    }
}

fn send_probe_pair() -> Result<(), String> {
    with_message_pump(|| {
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(F24),
                        dwExtraInfo: PRIORITY_PROBE_TAG,
                        ..KEYBDINPUT::default()
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(F24),
                        dwFlags: KEYEVENTF_KEYUP,
                        dwExtraInfo: PRIORITY_PROBE_TAG,
                        ..KEYBDINPUT::default()
                    },
                },
            },
        ];
        let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
        if sent == 2 {
            Ok(())
        } else {
            Err(format!("专用探针发送不完整：{sent}/2"))
        }
    })?
}

fn wait_for_competitor(expected: u64) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while COMPETITOR_EVENTS.load(Ordering::Relaxed) < expected {
        pump_messages();
        if Instant::now() >= deadline {
            return Err(format!(
                "竞争 Hook 等待探针超时：预期 {expected} 条，实际 {} 条",
                COMPETITOR_EVENTS.load(Ordering::Relaxed)
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let mut first = PumpedBackend::start()?;
    let competitor = Competitor::install()?;
    send_probe_pair()?;
    wait_for_competitor(2)?;
    let first_snapshot = first.diagnostics()?;
    let first_count = COMPETITOR_EVENTS.load(Ordering::Relaxed);
    if first_count != 2 || first_snapshot.probe_events != 0 {
        return Err(format!(
            "上游拦截验证失败：竞争 Hook={first_count}，OmniTrigger={}",
            first_snapshot.probe_events
        ));
    }
    println!("阶段 1 通过：后安装的竞争 Hook 吞掉 2 条探针，OmniTrigger 收到 0 条");
    drop(first);

    let mut second = PumpedBackend::start()?;
    send_probe_pair()?;
    wait_for_competitor(4)?;
    let second_snapshot = second.diagnostics()?;
    let second_count = COMPETITOR_EVENTS.load(Ordering::Relaxed);
    if second_count != 4 || second_snapshot.probe_events != 2 {
        return Err(format!(
            "监听重建验证失败：竞争 Hook={second_count}，OmniTrigger={}",
            second_snapshot.probe_events
        ));
    }
    println!("阶段 2 通过：重建后 OmniTrigger 先收到 2 条探针，竞争 Hook 累计吞掉 4 条");
    drop(second);
    drop(competitor);
    println!(
        "真实 Hook 链顺序验证完成；全部探针由竞争 Hook 消费，未发送硬件输入；测试 Hook 均已卸载"
    );
    Ok(())
}
