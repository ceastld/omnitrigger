//! 同一 Windows 会话只允许一个 OmniTrigger 输入线程安装全局 Hook。
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{GetLastError, SetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS},
        System::Threading::CreateMutexW,
    },
};

pub(super) fn acquire() -> Result<OwnedHandle, String> {
    acquire_named(w!("Local\\OmniTrigger.InputListener"))
}

fn acquire_named(name: PCWSTR) -> Result<OwnedHandle, String> {
    // 以对象的存在表示占用，不等待互斥量。最后一个句柄随线程退出或进程终止而关闭。
    unsafe {
        SetLastError(ERROR_SUCCESS);
        let handle = CreateMutexW(None, false, name)
            .map_err(|error| format!("无法登记当前 Windows 会话的输入监听：{error}"))?;
        let occupied = GetLastError() == ERROR_ALREADY_EXISTS;
        let owner = OwnedHandle::from_raw_handle(handle.0);
        if occupied {
            return Err("另一个 OmniTrigger 实例正在监听。仍可编辑配置。退出该实例后，可在这里点击“重建监听”。".into());
        }
        Ok(owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Barrier, thread};

    #[test]
    fn concurrent_starts_have_one_owner_and_a_later_start_can_take_over() {
        let name: Vec<u16> = format!(
            "Local\\OmniTrigger.InputListener.Test.{}",
            std::process::id()
        )
        .encode_utf16()
        .chain([0])
        .collect();
        let acquire = || acquire_named(PCWSTR(name.as_ptr()));
        let barrier = Barrier::new(2);
        let (first, second) = thread::scope(|scope| {
            let other = scope.spawn(|| {
                barrier.wait();
                acquire()
            });
            barrier.wait();
            (acquire(), other.join().unwrap())
        });
        let (owner, message) = match (first, second) {
            (Ok(owner), Err(message)) | (Err(message), Ok(owner)) => (owner, message),
            _ => panic!("并发启动应只有一个监听实例"),
        };
        assert!(message.contains("另一个 OmniTrigger 实例正在监听"));
        assert!(acquire().is_err());
        drop(owner);
        assert!(acquire().is_ok());
    }
}
