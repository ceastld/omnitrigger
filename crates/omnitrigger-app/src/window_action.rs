//! 可持久化的窗口操作；目标捕获与执行校验由 Windows 适配层完成。
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowOperation {
    ToggleMaximize,
    Minimize,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowAction {
    pub operation: WindowOperation,
}

impl WindowAction {
    pub fn display(self) -> &'static str {
        match self.operation {
            WindowOperation::ToggleMaximize => "最大化 / 恢复",
            WindowOperation::Minimize => "最小化",
            WindowOperation::Close => "关闭窗口",
        }
    }

    pub fn platform_operation(self) -> omnitrigger_windows::WindowOperation {
        match self.operation {
            WindowOperation::ToggleMaximize => omnitrigger_windows::WindowOperation::ToggleMaximize,
            WindowOperation::Minimize => omnitrigger_windows::WindowOperation::Minimize,
            WindowOperation::Close => omnitrigger_windows::WindowOperation::Close,
        }
    }
}
