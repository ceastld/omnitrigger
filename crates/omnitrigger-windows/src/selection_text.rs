//! 只读选区的纯文本契约；不访问平台或隐含状态。
#[derive(Debug, PartialEq, Eq)]
pub struct SelectionTextSnapshot {
    pub text: String,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionOverflow {
    Reject,
    Truncate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionReadError {
    Cancelled,
    Timeout,
    TargetChanged,
    SelectionChanged,
    Unavailable,
    Empty,
    Multiple,
    Protected,
    TooLong,
    InvalidResponse,
}

impl SelectionReadError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::TargetChanged => "target_changed",
            Self::SelectionChanged => "selection_changed",
            Self::Unavailable => "selection_unavailable",
            Self::Empty => "selection_empty",
            Self::Multiple => "selection_multiple",
            Self::Protected => "selection_protected",
            Self::TooLong => "selection_too_long",
            Self::InvalidResponse => "invalid_response",
        }
    }
}

// 仅请求129哨兵；truncate精确取前128个UTF16单元，拆开的代理对按统一错误拒绝。
#[cfg(any(windows, test))]
pub(crate) fn selection_text(
    raw: &[u16],
    overflow: SelectionOverflow,
) -> Result<SelectionTextSnapshot, SelectionReadError> {
    let truncated = raw.len() > 128;
    if truncated && overflow == SelectionOverflow::Reject {
        return Err(SelectionReadError::TooLong);
    }
    let text = String::from_utf16(&raw[..raw.len().min(128)])
        .map_err(|_| SelectionReadError::InvalidResponse)?;
    if text.contains('\0') {
        return Err(SelectionReadError::InvalidResponse);
    }
    Ok(SelectionTextSnapshot { text, truncated })
}

// 只解释成功的UIA调用结果；provider失败不得进入这些分支。
#[cfg(any(windows, test))]
pub(crate) fn selection_has_range(count: i32) -> Result<bool, SelectionReadError> {
    match count {
        0 => Ok(false),
        1 => Ok(true),
        n if n > 1 => Err(SelectionReadError::Multiple),
        _ => Err(SelectionReadError::InvalidResponse),
    }
}
#[cfg(any(windows, test))]
pub(crate) fn selection_range_has_text(order: i32) -> Result<bool, SelectionReadError> {
    match order {
        0 => Ok(false),
        n if n < 0 => Ok(true),
        _ => Err(SelectionReadError::InvalidResponse),
    }
}

#[cfg(test)]
mod selection_read_pure_tests {
    use super::*;
    #[test]
    fn reject_is_default_contract_and_exact128_is_not_truncated() {
        for text in ["A".repeat(128), "字".repeat(128), "😀".repeat(64)] {
            let units: Vec<u16> = text.encode_utf16().collect();
            for mode in [SelectionOverflow::Reject, SelectionOverflow::Truncate] {
                assert_eq!(
                    selection_text(&units, mode).unwrap(),
                    SelectionTextSnapshot {
                        text: text.clone(),
                        truncated: false
                    }
                );
            }
        }
        assert_eq!(
            selection_text(&[65; 129], SelectionOverflow::Reject),
            Err(SelectionReadError::TooLong)
        );
        assert_eq!(
            selection_text(&[], SelectionOverflow::Reject).unwrap(),
            SelectionTextSnapshot {
                text: String::new(),
                truncated: false
            }
        );
        assert_eq!(
            selection_text(&[], SelectionOverflow::Truncate).unwrap(),
            SelectionTextSnapshot {
                text: String::new(),
                truncated: false
            }
        );
    }
    #[test]
    fn successful_absent_or_degenerate_range_is_empty_but_invalid_structure_is_error() {
        assert_eq!(selection_has_range(0), Ok(false));
        assert_eq!(selection_has_range(1), Ok(true));
        assert_eq!(selection_has_range(2), Err(SelectionReadError::Multiple));
        assert_eq!(
            selection_has_range(-1),
            Err(SelectionReadError::InvalidResponse)
        );
        assert_eq!(selection_range_has_text(0), Ok(false));
        assert_eq!(selection_range_has_text(-1), Ok(true));
        assert_eq!(
            selection_range_has_text(1),
            Err(SelectionReadError::InvalidResponse)
        );
        for mode in [SelectionOverflow::Reject, SelectionOverflow::Truncate] {
            assert_eq!(
                selection_text(&[], mode).unwrap(),
                SelectionTextSnapshot {
                    text: String::new(),
                    truncated: false
                }
            );
            assert_eq!(
                selection_text(&[0], mode),
                Err(SelectionReadError::InvalidResponse)
            );
        }
    }
    #[test]
    fn truncate_is_exact128_and_never_repairs_a_split_surrogate() {
        assert_eq!(
            selection_text(&[65; 129], SelectionOverflow::Truncate).unwrap(),
            SelectionTextSnapshot {
                text: "A".repeat(128),
                truncated: true
            }
        );
        let emoji65: Vec<u16> = "😀".repeat(65).encode_utf16().collect();
        // UIA GetText(129)返回的尾部可能是第65个emoji的高代理项，前128单元仍完整。
        assert_eq!(
            selection_text(&emoji65[..129], SelectionOverflow::Truncate).unwrap(),
            SelectionTextSnapshot {
                text: "😀".repeat(64),
                truncated: true
            }
        );
        let split: Vec<u16> = ("A".repeat(127) + "😀").encode_utf16().collect();
        assert_eq!(
            selection_text(&split, SelectionOverflow::Truncate),
            Err(SelectionReadError::InvalidResponse)
        );
        assert_eq!(
            selection_text(&split, SelectionOverflow::Reject),
            Err(SelectionReadError::TooLong)
        );
        assert_eq!(
            selection_text(&[0xd800], SelectionOverflow::Truncate),
            Err(SelectionReadError::InvalidResponse)
        );
        assert_eq!(
            selection_text(&[0], SelectionOverflow::Truncate),
            Err(SelectionReadError::InvalidResponse)
        );
    }
}
