//! 在单独窗口站验证真实 Win32 格式与内存交接，不访问交互桌面的剪贴板。
#![cfg(windows)]

use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
    },
};

#[path = "support/private_clipboard.rs"]
mod private_clipboard;
use private_clipboard::PrivateClipboard;

#[test]
#[ignore = "需要可创建命名窗口站的 Windows 管理员令牌；CI 显式运行，绝不回退交互剪贴板"]
fn native_exclusion_formats_are_present_before_later_plain_copy_clears_them() {
    let scope = PrivateClipboard::create();
    let names = [
        "OmniTrigger.TransientClipboard.v1",
        "ExcludeClipboardContentFromMonitorProcessing",
    ];
    let formats: Vec<u32> = names
        .iter()
        .map(|name| {
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let format = unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) };
            assert_ne!(format, 0);
            format
        })
        .collect();
    scope.verify();
    omnitrigger_windows::write_clipboard_text_excluding_history_if("literal 文😀", || Ok(()))
        .unwrap();
    assert_eq!(
        omnitrigger_windows::read_clipboard_text().unwrap(),
        "literal 文😀"
    );
    scope.verify();
    unsafe {
        OpenClipboard(None).unwrap();
        struct Close;
        impl Drop for Close {
            fn drop(&mut self) {
                let _ = unsafe { CloseClipboard() };
            }
        }
        let _close = Close;
        for format in &formats {
            IsClipboardFormatAvailable(*format).unwrap();
            let memory = HGLOBAL(GetClipboardData(*format).unwrap().0);
            assert!(GlobalSize(memory) >= 4);
            let pointer = GlobalLock(memory);
            assert!(!pointer.is_null());
            let payload = std::ptr::read_unaligned(pointer.cast::<u32>());
            let _ = GlobalUnlock(memory);
            assert_eq!(payload, 1);
        }
    }
    scope.verify();
    omnitrigger_windows::write_clipboard_text("later plain copy").unwrap();
    assert_eq!(
        omnitrigger_windows::read_clipboard_text().unwrap(),
        "later plain copy"
    );
    assert!(formats
        .iter()
        .all(|format| unsafe { IsClipboardFormatAvailable(*format) }.is_err()));
}
