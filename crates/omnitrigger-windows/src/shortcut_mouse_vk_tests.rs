//! 纯计划回归；没有SendInput、GetAsyncKeyState、目标捕获或真实桌面调用。
use super::{
    plan, sequence_plan, tap_plan, validate_key_sequence, validate_key_tap, validate_shortcut,
};
use std::collections::BTreeSet;

#[test]
fn mouse_virtual_keys_never_create_a_keyboard_input_plan() {
    let held = BTreeSet::new();
    for key in [1_u16, 2, 4, 5, 6] {
        assert!(validate_shortcut(&[key]).is_err(), "{key}");
        assert!(validate_shortcut(&[0xa2, key]).is_err(), "{key}");
        assert!(validate_key_tap(key).is_err(), "{key}");
        assert!(validate_key_sequence(&[], &[key]).is_err(), "{key}");
        assert!(validate_key_sequence(&[key], &[0x41]).is_err(), "{key}");
        assert!(plan(&[key], &held).is_err(), "{key}");
        assert!(tap_plan(key, &held).is_err(), "{key}");
        assert!(sequence_plan(&[], &[key], &held).is_err(), "{key}");
    }
}
