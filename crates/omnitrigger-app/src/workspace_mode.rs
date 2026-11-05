//! 进程启动时确定的窗口职责；普通主窗口切换到文本历史页不会改变此模式。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkspaceMode {
    Main,
    HistoryWindow,
}

impl WorkspaceMode {
    /// 独立历史入口只允许自己的窗口参数；在任何用户CLI副作用前检查。
    pub fn from_arguments(arguments: &[String]) -> Result<Self, &'static str> {
        let invalid = "独立历史窗口仅接受 --text-history 和可选的 --config 路径。";
        // 不支持赋值形式，不能把明确的历史入口误降级为拥有Hook的普通主窗口。
        if arguments
            .iter()
            .skip(1)
            .any(|argument| argument.starts_with("--text-history="))
        {
            return Err(invalid);
        }
        if !arguments
            .iter()
            .skip(1)
            .any(|argument| argument == "--text-history")
        {
            return Ok(Self::Main);
        }
        let mut history_seen = false;
        let mut config_seen = false;
        let mut arguments = arguments.iter().skip(1);
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--text-history" if !history_seen => history_seen = true,
                "--config" if !config_seen => {
                    config_seen = true;
                    if !arguments
                        .next()
                        .is_some_and(|path| !path.is_empty() && !path.starts_with("--"))
                    {
                        return Err(invalid);
                    }
                }
                _ => return Err(invalid),
            }
        }
        Ok(Self::HistoryWindow)
    }

    pub fn from_text_history_flag(text_history: bool) -> Self {
        if text_history {
            Self::HistoryWindow
        } else {
            Self::Main
        }
    }

    pub fn owns_input_engine(self) -> bool {
        self == Self::Main
    }

    pub fn history_only(self) -> bool {
        self == Self::HistoryWindow
    }

    pub fn window_title(self) -> &'static str {
        match self {
            Self::Main => "OmniTrigger",
            Self::HistoryWindow => "OmniTrigger · 独立历史窗口",
        }
    }

    pub fn initial_status(self) -> &'static str {
        match self {
            Self::Main => "正在连接键盘和鼠标…",
            Self::HistoryWindow => "独立历史窗口 · 触发器监听未启动",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        std::iter::once("omnitrigger")
            .chain(values.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn explicit_history_entry_does_not_own_a_trigger_engine() {
        let mode = WorkspaceMode::from_text_history_flag(true);
        assert_eq!(mode, WorkspaceMode::HistoryWindow);
        assert!(!mode.owns_input_engine());
        assert!(mode.history_only());
        assert_eq!(mode.window_title(), "OmniTrigger · 独立历史窗口");
        assert_eq!(mode.initial_status(), "独立历史窗口 · 触发器监听未启动");
        assert_eq!(
            WorkspaceMode::from_arguments(&arguments(&["--text-history"])),
            Ok(mode)
        );
    }

    #[test]
    fn normal_main_window_keeps_engine_ownership_independent_of_visible_page() {
        let mode = WorkspaceMode::from_text_history_flag(false);
        assert_eq!(mode, WorkspaceMode::Main);
        assert!(mode.owns_input_engine());
        assert!(!mode.history_only());
        assert_eq!(mode.window_title(), "OmniTrigger");
        assert_eq!(
            WorkspaceMode::from_arguments(&arguments(&["--smoke-test"])),
            Ok(mode)
        );
    }

    #[test]
    fn history_window_accepts_explicit_config_in_either_order() {
        for values in [
            vec!["--text-history", "--config", "owned.json"],
            vec!["--config", "自建目录/历史.json", "--text-history"],
        ] {
            assert_eq!(
                WorkspaceMode::from_arguments(&arguments(&values)),
                Ok(WorkspaceMode::HistoryWindow)
            );
        }
    }

    #[test]
    fn history_window_rejects_engine_checks_and_other_commands_before_startup() {
        for other in [
            "--smoke-test",
            "--ui-smoke",
            "--settings",
            "--panel",
            "--wheel",
            "--wheel-editor",
            "--browser-connector-settings",
            "--run-action",
            "--check-config",
        ] {
            assert!(WorkspaceMode::from_arguments(&arguments(&["--text-history", other])).is_err());
        }
    }

    #[test]
    fn history_window_rejects_missing_config_and_duplicate_switches() {
        for values in [
            vec!["--text-history", "--config"],
            vec!["--text-history", "--text-history"],
            vec!["--text-history", "--config", "a.json", "--config", "b.json"],
        ] {
            assert!(WorkspaceMode::from_arguments(&arguments(&values)).is_err());
        }
    }
}

#[cfg(test)]
#[path = "workspace_mode_regression.rs"]
mod regression;
