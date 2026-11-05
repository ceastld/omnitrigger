//! 独立历史入口的参数回归；只调用纯模式解析，不运行任何CLI或输入引擎。

use super::WorkspaceMode;

fn arguments(values: &[&str]) -> Vec<String> {
    std::iter::once("omnitrigger")
        .chain(values.iter().copied())
        .map(str::to_owned)
        .collect()
}

#[test]
fn unsupported_history_assignment_cannot_fall_back_to_main_mode() {
    for value in [
        "--text-history=true",
        "--text-history=false",
        "--text-history=1",
        "--text-history=",
    ] {
        assert!(WorkspaceMode::from_arguments(&arguments(&[value])).is_err());
    }
}

#[test]
fn assigned_history_switch_with_effectful_command_is_rejected_before_dispatch() {
    for values in [
        vec!["--text-history=true", "--smoke-test"],
        vec!["--run-action", "owned/action.json", "--text-history=true"],
    ] {
        let arguments = arguments(&values);
        // 没有显式滚轮策略时，该前置解析器不会替模式门拒绝未知历史写法。
        assert!(crate::wheel_input_cli::parse(&arguments).is_ok());
        assert!(WorkspaceMode::from_arguments(&arguments).is_err());
    }
}

#[test]
fn exact_history_switch_does_not_make_assignment_alias_valid() {
    assert!(
        WorkspaceMode::from_arguments(&arguments(&["--text-history", "--text-history=true"]))
            .is_err()
    );
}

#[test]
fn ordinary_config_filename_containing_history_text_keeps_main_mode() {
    assert_eq!(
        WorkspaceMode::from_arguments(&arguments(&["--config", "owned/--text-history=true.json"])),
        Ok(WorkspaceMode::Main)
    );
}

#[test]
fn history_config_filename_containing_history_text_remains_a_path() {
    let result = WorkspaceMode::from_arguments(&arguments(&[
        "--config",
        "自建目录/--text-history=true.json",
        "--text-history",
    ]));
    assert_eq!(result, Ok(WorkspaceMode::HistoryWindow));
    assert!(!result.unwrap().owns_input_engine());
}
