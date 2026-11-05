//! 真实格式测试仅在独立进程的非交互窗口站运行，绝不回退到个人剪贴板。
#![cfg(windows)]

use omnitrigger_windows::{
    read_clipboard_data, ClipboardData, ClipboardDataError, ClipboardDataSnapshot,
};
use windows::{
    core::w,
    Win32::{
        Foundation::{GlobalFree, HANDLE, HWND},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber, OpenClipboard,
                RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::{CF_HDROP, CF_UNICODETEXT},
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
        },
    },
};

#[path = "support/private_clipboard.rs"]
mod private_clipboard;
use private_clipboard::PrivateClipboard;

struct Owner(HWND);

impl Owner {
    fn create(scope: &PrivateClipboard) -> Self {
        scope.verify();
        Self(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("OmniTrigger bounded clipboard data test"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .expect("独立窗口站必须能创建隐藏所有者窗口")
        })
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

struct Close;

impl Drop for Close {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

/// 数据全部由本测试生成，未接触交互桌面，也不访问列表中的路径。
fn publish(scope: &PrivateClipboard, owner: &Owner, formats: &[(u32, Vec<u8>)], allocation: usize) {
    scope.verify();
    unsafe {
        OpenClipboard(Some(owner.0)).unwrap();
        let _close = Close;
        EmptyClipboard().unwrap();
        for (format, bytes) in formats {
            let memory = GlobalAlloc(GMEM_MOVEABLE, allocation.max(bytes.len()).max(1)).unwrap();
            let pointer = GlobalLock(memory);
            if pointer.is_null() {
                let _ = GlobalFree(Some(memory));
                panic!("无法锁定测试内存");
            }
            let size = GlobalSize(memory);
            assert!(size >= bytes.len());
            // 保证未终止样本不会意外借助分配器的零填充成为合法数据。
            std::ptr::write_bytes(pointer.cast::<u8>(), 0x61, size);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
            let _ = GlobalUnlock(memory);
            if SetClipboardData(*format, Some(HANDLE(memory.0))).is_err() {
                let _ = GlobalFree(Some(memory));
                panic!("独立窗口站无法发布测试格式");
            }
            // 成功后内存所有权属于系统，不能再 GlobalFree。
        }
    }
    scope.verify();
}

fn read(scope: &PrivateClipboard) -> Result<ClipboardDataSnapshot, ClipboardDataError> {
    scope.verify();
    let result = read_clipboard_data();
    scope.verify();
    result
}

fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn drop_data(wide: bool, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 20];
    bytes[..4].copy_from_slice(&20_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&u32::from(wide).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
#[ignore = "需可创建命名窗口站的 Windows 令牌；独立测试进程串行运行，禁止回退交互剪贴板"]
fn native_bounded_data_formats_in_private_window_station() {
    // 本测试目标仅有一个测试；调用方仍须使用独立进程及 --test-threads=1。
    let scope = PrivateClipboard::create();
    let owner = Owner::create(&scope);
    let text_format = u32::from(CF_UNICODETEXT.0);
    let files_format = u32::from(CF_HDROP.0);

    let custom =
        unsafe { RegisterClipboardFormatW(w!("OmniTrigger.ClipboardDataTest.Unsupported")) };
    assert_ne!(custom, 0);
    for formats in [vec![], vec![(custom, vec![1, 2, 3, 4])]] {
        publish(&scope, &owner, &formats, 0);
        assert_eq!(
            read(&scope).unwrap_err().code,
            "unsupported_clipboard_format"
        );
    }

    // 大分配尾部填充不等于超大文本；空文本也优先于文件列表。
    for text in ["", "literal 文😀\r\n"] {
        publish(
            &scope,
            &owner,
            &[
                (
                    files_format,
                    drop_data(true, &wide("C:\\not-opened.txt\0\0")),
                ),
                (text_format, wide(&format!("{text}\0"))),
            ],
            2 * 1024 * 1024 + 1,
        );
        let sequence = unsafe { GetClipboardSequenceNumber() };
        assert_eq!(
            read(&scope).unwrap(),
            ClipboardDataSnapshot {
                data: ClipboardData::Text(text.into()),
                sequence_number: sequence,
            }
        );
    }

    let mut large_gap = drop_data(true, &[]);
    let offset = 1024 * 1024 + 20;
    large_gap[..4].copy_from_slice(&(offset as u32).to_le_bytes());
    large_gap.resize(offset, 0xcc);
    large_gap.extend_from_slice(&wide("gap data\0\0"));
    let file_cases = [
        (
            drop_data(true, &wide("C:\\文😀.txt\0D:\\absent.txt\0\0")),
            vec!["C:\\文😀.txt", "D:\\absent.txt"],
        ),
        (
            drop_data(false, b"C:\\one.txt\0D:\\two.txt\0\0"),
            vec!["C:\\one.txt", "D:\\two.txt"],
        ),
        (drop_data(true, &wide("\0\0")), vec![]),
        (drop_data(false, &[0, 0]), vec![]),
        (large_gap, vec!["gap data"]),
    ];
    for (bytes, expected) in file_cases {
        publish(
            &scope,
            &owner,
            &[(files_format, bytes)],
            2 * 1024 * 1024 + 1,
        );
        assert_eq!(
            read(&scope).unwrap().data,
            ClipboardData::Files(expected.into_iter().map(str::to_owned).collect())
        );
    }

    let maximum = format!("{}\0", "a".repeat(65_536));
    publish(&scope, &owner, &[(text_format, wide(&maximum))], 0);
    assert_eq!(
        read(&scope).unwrap().data,
        ClipboardData::Text("a".repeat(65_536))
    );
    let over = format!("{}\0", "a".repeat(65_537));
    publish(&scope, &owner, &[(text_format, wide(&over))], 0);
    assert_eq!(read(&scope).unwrap_err().code, "clipboard_too_large");

    // 已选择的文本格式坏 Unicode 时不能静默转读合法文件列表。
    publish(
        &scope,
        &owner,
        &[
            (text_format, vec![0, 0xd8, 0, 0]),
            (files_format, drop_data(true, &wide("C:\\data.txt\0\0"))),
        ],
        0,
    );
    assert_eq!(read(&scope).unwrap_err().code, "invalid_clipboard_data");

    let maximum_files = format!("{}\0", "x\0".repeat(64));
    publish(
        &scope,
        &owner,
        &[(files_format, drop_data(true, &wide(&maximum_files)))],
        0,
    );
    assert_eq!(
        read(&scope).unwrap().data,
        ClipboardData::Files(vec!["x".into(); 64])
    );
    let maximum_path = "😀".repeat(32_768);
    publish(
        &scope,
        &owner,
        &[(
            files_format,
            drop_data(true, &wide(&format!("{maximum_path}\0\0"))),
        )],
        0,
    );
    assert_eq!(
        read(&scope).unwrap().data,
        ClipboardData::Files(vec![maximum_path])
    );

    let mut bad_offset = drop_data(true, &wide("x\0\0"));
    bad_offset[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    for bytes in [
        bad_offset,
        vec![1; 19],
        drop_data(true, &[0, 0xd8, 0, 0, 0, 0]),
        drop_data(true, &wide("x\0")),
        drop_data(false, b"x\0"),
    ] {
        publish(&scope, &owner, &[(files_format, bytes)], 0);
        assert_eq!(read(&scope).unwrap_err().code, "invalid_clipboard_data");
    }
    let too_many = format!("{}\0", "x\0".repeat(65));
    let too_long = format!("{}\0x\0\0", "😀".repeat(32_768));
    for text in [too_many, too_long] {
        publish(
            &scope,
            &owner,
            &[(files_format, drop_data(true, &wide(&text)))],
            0,
        );
        assert_eq!(read(&scope).unwrap_err().code, "clipboard_too_large");
    }

    // 所有错误路径均必须关闭读取租约；随后仍能发布并读取。
    publish(&scope, &owner, &[(text_format, wide("done\0"))], 0);
    assert_eq!(
        read(&scope).unwrap().data,
        ClipboardData::Text("done".into())
    );
    scope.verify();
}
