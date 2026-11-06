//! 只读选区的纯文本契约；不访问平台或隐含状态。
pub const DEFAULT_SELECTION_UTF16_UNITS: usize = 128;
// 64KiB helper采集/动作结果界限；最坏JSON转义每UTF16单元6字节，保留协议余量。
pub const MAX_SELECTION_UTF16_UNITS: usize = 8192;
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

/// UIA 的有界哨兵请求；无效调用在任何 provider 工作之前拒绝。
#[cfg(any(windows, test))]
pub(crate) fn selection_request_units(max_utf16_units: usize) -> Result<i32, SelectionReadError> {
    if !(1..=MAX_SELECTION_UTF16_UNITS).contains(&max_utf16_units) {
        return Err(SelectionReadError::InvalidResponse);
    }
    Ok((max_utf16_units + 1) as i32)
}

// 默认入口继续供既有纯协议测试使用；生产平台统一走同一个带界限解码器。
#[cfg(test)]
pub(crate) fn selection_text(
    raw: &[u16],
    overflow: SelectionOverflow,
) -> Result<SelectionTextSnapshot, SelectionReadError> {
    selection_text_with_limit(raw, overflow, DEFAULT_SELECTION_UTF16_UNITS)
}
#[cfg(any(windows, test))]
pub(crate) fn selection_text_with_limit(
    raw: &[u16],
    overflow: SelectionOverflow,
    max_utf16_units: usize,
) -> Result<SelectionTextSnapshot, SelectionReadError> {
    selection_request_units(max_utf16_units)?;
    let truncated = raw.len() > max_utf16_units;
    if truncated && overflow == SelectionOverflow::Reject {
        return Err(SelectionReadError::TooLong);
    }
    let text = String::from_utf16(&raw[..raw.len().min(max_utf16_units)])
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

#[cfg(test)]
mod explicit_selection_limit_tests {
    use super::*;
    #[test]
    fn request_and_decoding_use_the_same_real_bound() {
        for limit in [1, 128, 129, 2048, 8192] {
            assert_eq!(selection_request_units(limit).unwrap(), (limit + 1) as i32);
            assert_eq!(
                selection_text_with_limit(&vec![65; limit], SelectionOverflow::Reject, limit)
                    .unwrap()
                    .text
                    .len(),
                limit
            );
            assert_eq!(
                selection_text_with_limit(&vec![65; limit + 1], SelectionOverflow::Reject, limit),
                Err(SelectionReadError::TooLong)
            );
            let value =
                selection_text_with_limit(&vec![65; limit + 1], SelectionOverflow::Truncate, limit)
                    .unwrap();
            assert_eq!(value.text.len(), limit);
            assert!(value.truncated);
        }
        for limit in [0, 8193, 65536, usize::MAX] {
            assert_eq!(
                selection_request_units(limit),
                Err(SelectionReadError::InvalidResponse)
            );
        }
    }
    #[test]
    fn empty_is_success_and_splitting_surrogates_is_never_repaired() {
        let complete: Vec<u16> = "😀".repeat(4096).encode_utf16().collect();
        assert_eq!(
            selection_text_with_limit(&complete, SelectionOverflow::Reject, 8192)
                .unwrap()
                .text,
            "😀".repeat(4096)
        );
        let split: Vec<u16> = ("A".repeat(8191) + "😀").encode_utf16().collect();
        assert_eq!(
            selection_text_with_limit(&split, SelectionOverflow::Truncate, 8192),
            Err(SelectionReadError::InvalidResponse)
        );
        assert_eq!(
            selection_text_with_limit(&split, SelectionOverflow::Reject, 8192),
            Err(SelectionReadError::TooLong)
        );
        assert_eq!(
            selection_text_with_limit(&[], SelectionOverflow::Reject, 8192).unwrap(),
            SelectionTextSnapshot {
                text: String::new(),
                truncated: false
            }
        );
        assert_eq!(
            selection_text_with_limit(&[0], SelectionOverflow::Reject, 8192),
            Err(SelectionReadError::InvalidResponse)
        );
        let oversized =
            selection_text_with_limit(&vec![65; 8194], SelectionOverflow::Truncate, 8192).unwrap();
        assert_eq!(oversized.text.len(), 8192);
        assert!(oversized.truncated);
    }
}
