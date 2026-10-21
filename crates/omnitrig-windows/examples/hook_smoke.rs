//! 安装真实 Hook、注册配置并立即卸载，不生成或注入输入事件。

use std::{sync::mpsc::RecvTimeoutError, time::Duration};

use omnitrig_core::{Activation, Button, MouseButton, TriggerSpec, WheelDirection};
use omnitrig_windows::{Backend, BackendCommand, BackendEvent};

fn main() -> Result<(), String> {
    let (backend, events) = Backend::start()?;
    match events.recv_timeout(Duration::from_secs(5)) {
        Ok(BackendEvent::Ready) => println!("真实键盘与鼠标 Hook 安装成功"),
        Ok(event) => return Err(format!("等待就绪时收到意外事件：{event:?}")),
        Err(error) => return Err(format!("输入线程未按时就绪：{error}")),
    }

    // 在注册测试配置前先暂停，避免冒烟期间影响用户的真实输入。
    backend.send(BackendCommand::SetEnabled(false))?;
    backend.send(BackendCommand::SetBindings(vec![
        TriggerSpec {
            id: 1,
            held: vec![Button::Key(0x14)],
            activation: Activation::Press(Button::Key(0x57)),
        },
        TriggerSpec {
            id: 2,
            held: vec![Button::Mouse(MouseButton::X1)],
            activation: Activation::Wheel(WheelDirection::Up),
        },
    ]))?;
    backend.send(BackendCommand::SetEnabled(false))?;
    drop(backend);

    // Drop 会等待工作线程完成卸载；检查事件发送端已销毁，同时报告注册错误。
    match events.recv_timeout(Duration::from_secs(1)) {
        Ok(BackendEvent::Error(error)) => return Err(error),
        Ok(event) => return Err(format!("收到意外事件：{event:?}")),
        Err(RecvTimeoutError::Disconnected) => {}
        Err(RecvTimeoutError::Timeout) => return Err("退出后输入线程仍持有通知通道".into()),
    }
    println!("配置注册成功，输入线程已退出，Hook 清理完成；未注入任何输入");
    Ok(())
}
