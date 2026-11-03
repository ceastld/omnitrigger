use crate::*;

const SIDE: Button = Button::Mouse(MouseButton::X1);
const CAPS: Button = Button::Key(20);
const K: Button = Button::Key(75);
fn scope(name: &str) -> ProcessScope {
    ProcessScope {
        allow: vec![name.into()],
        ..Default::default()
    }
}
fn row(id: u64, pointer: ProcessScope) -> TriggerSpec {
    TriggerSpec {
        id,
        held: vec![SIDE],
        activation: Activation::Wheel(WheelDirection::Up),
        pointer_scope: pointer,
        process_scope: Default::default(),
        wheel: WheelPolicy::Packet,
        keyboard: Default::default(),
        conditions: vec![],
    }
}
fn input(e: &mut Engine, kind: InputKind) -> Decision {
    e.process(InputEvent {
        kind,
        injected: false,
    })
}
fn wheel(e: &mut Engine, delta: i32) -> Decision {
    input(
        e,
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta,
        },
    )
}
fn pointer(e: &mut Engine, name: Option<&str>) -> Decision {
    e.set_pointer_context(name, WindowRole::Application)
        .unwrap()
}

#[test]
fn pointer_scope_and_foreground_scope_are_independent_and_both_apply() {
    let mut a = row(1, scope("reader"));
    a.process_scope = scope("editor");
    let mut e = Engine::new(vec![a]).unwrap();
    e.set_foreground_process(Some("editor")).unwrap();
    pointer(&mut e, Some("other"));
    assert!(!input(&mut e, InputKind::Down(SIDE)).consume);
    input(&mut e, InputKind::Up(SIDE));
    pointer(&mut e, Some("READER.EXE"));
    assert_eq!(e.foreground_process(), Some("editor"));
    assert_eq!(e.pointer_process(), Some("reader"));
    assert!(input(&mut e, InputKind::Down(SIDE)).consume);
    assert_eq!(wheel(&mut e, 1).triggered, [1]);
    e.set_foreground_process(Some("other")).unwrap();
    assert!(!wheel(&mut e, 240).consume);
    assert!(input(&mut e, InputKind::Up(SIDE)).consume);
}

#[test]
fn pointer_scope_unknown_blocks_related_fallback_and_preserves_unrelated_keyboard_input() {
    let global = row(1, Default::default());
    let specific = row(2, scope("reader"));
    let mut key = row(3, Default::default());
    key.held = vec![CAPS];
    key.activation = Activation::Press(K);
    key.wheel = WheelPolicy::Notch;
    let mut e = Engine::new(vec![global, specific, key]).unwrap();
    assert!(!input(&mut e, InputKind::Down(SIDE)).consume);
    input(&mut e, InputKind::Up(SIDE));
    input(&mut e, InputKind::Down(CAPS));
    pointer(&mut e, Some("reader"));
    pointer(&mut e, None);
    assert_eq!(input(&mut e, InputKind::Down(K)).triggered, [3]);
    input(&mut e, InputKind::Up(K));
    input(&mut e, InputKind::Up(CAPS));
    pointer(&mut e, Some("reader"));
    input(&mut e, InputKind::Down(SIDE));
    assert_eq!(wheel(&mut e, 15).triggered, [2]);
    input(&mut e, InputKind::Up(SIDE));
    pointer(&mut e, Some("other"));
    input(&mut e, InputKind::Down(SIDE));
    assert_eq!(wheel(&mut e, 15).triggered, [1]);
}

#[test]
fn pointer_scope_loss_returns_raw_notch_tail_and_does_not_borrow_it_in_another_app() {
    let mut a = row(1, scope("reader"));
    a.wheel = WheelPolicy::Notch;
    let mut b = row(2, scope("other"));
    b.wheel = WheelPolicy::Notch;
    let mut e = Engine::new(vec![a, b]).unwrap();
    pointer(&mut e, Some("reader"));
    input(&mut e, InputKind::Down(SIDE));
    assert!(wheel(&mut e, 60).triggered.is_empty());
    let changed = pointer(&mut e, Some("other"));
    assert_eq!(
        changed.replay,
        [
            InputKind::Down(SIDE),
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 60
            }
        ]
    );
    // 原前缀已交还，不能从新窗口中途借回。
    assert!(!wheel(&mut e, 60).consume);
    input(&mut e, InputKind::Up(SIDE));
    input(&mut e, InputKind::Down(SIDE));
    assert!(wheel(&mut e, 60).triggered.is_empty());
    assert_eq!(wheel(&mut e, 60).triggered, [2]);
}

#[test]
fn pointer_scope_loss_cancels_timed_action_without_retargeting_and_equal_context_keeps_it() {
    let mut a = row(1, scope("reader"));
    a.held.clear();
    a.wheel = WheelPolicy::Notch;
    a.activation = Activation::Hold {
        button: SIDE,
        duration_ms: 300,
    };
    let mut e = Engine::new(vec![a]).unwrap();
    pointer(&mut e, Some("reader"));
    input(&mut e, InputKind::Down(SIDE));
    assert!(pointer(&mut e, Some("reader")).replay.is_empty());
    assert_eq!(e.next_hold_deadline_ms(), Some(300));
    assert_eq!(pointer(&mut e, None).replay, [InputKind::Down(SIDE)]);
    pointer(&mut e, Some("reader"));
    assert!(e.advance_time(400).triggered.is_empty());
    assert!(!input(&mut e, InputKind::Up(SIDE)).consume);
}

#[test]
fn pointer_scope_conflicts_compare_each_subject_separately_and_reject_invalid_replacement() {
    assert!(Engine::new(vec![row(1, scope("reader")), row(2, scope("other"))]).is_ok());
    let mut foreground_only = row(2, Default::default());
    foreground_only.process_scope = scope("other");
    assert!(matches!(
        Engine::new(vec![row(1, scope("reader")), foreground_only]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
    let a = row(1, scope("reader"));
    let mut e = Engine::new(vec![a.clone()]).unwrap();
    pointer(&mut e, Some("reader"));
    input(&mut e, InputKind::Down(SIDE));
    let mut bad = a;
    bad.pointer_scope.allow = vec!["C:/reader.exe".into()];
    assert!(matches!(
        e.replace_bindings(vec![bad]),
        Err(RegistrationError::InvalidPointerProcessScope { id: 1, .. })
    ));
    assert!(
        e.set_pointer_context(Some("bad/name"), WindowRole::Desktop)
            .is_err()
    );
    assert_eq!(e.pointer_process(), Some("reader"));
    assert_eq!(wheel(&mut e, 1).triggered, [1]);
}

#[test]
fn pointer_scope_shell_role_unknown_never_guesses_global_fallback() {
    let mut shell = scope("desktop");
    shell.identity = ProcessIdentity::ShellContext;
    let mut e = Engine::new(vec![row(1, Default::default()), row(2, shell)]).unwrap();
    e.set_foreground_process(Some("editor")).unwrap();
    e.set_pointer_context(Some("explorer"), WindowRole::Unknown)
        .unwrap();
    assert!(!input(&mut e, InputKind::Down(SIDE)).consume);
    input(&mut e, InputKind::Up(SIDE));
    e.set_pointer_context(Some("explorer"), WindowRole::Desktop)
        .unwrap();
    input(&mut e, InputKind::Down(SIDE));
    assert_eq!(wheel(&mut e, 1).triggered, [2]);
    assert_eq!(e.foreground_process(), Some("editor"));
    assert_eq!(e.pointer_process(), Some("explorer"));
    assert_eq!(e.pointer_window_role(), WindowRole::Desktop);
}

#[test]
fn pointer_scope_does_not_control_retained_capture_scope_or_restore_unused_tap() {
    let mut a = row(1, scope("reader"));
    a.held = vec![CAPS];
    a.keyboard = KeyboardPolicy {
        matching: KeyboardMatching::Captured { priority: 1 },
        retained_prefix: Some(Box::new(RetainedPrefix {
            key: CAPS,
            capture_scope: Default::default(),
            quiet_ms: 50,
            fast_ms: 50,
            block_with_primary_mouse: false,
        })),
        restore_on_tap: true,
        tap_timeout_ms: Some(1500),
        ..Default::default()
    };
    let mut e = Engine::new(vec![a]).unwrap();
    assert!(input(&mut e, InputKind::Down(CAPS)).consume);
    pointer(&mut e, Some("reader"));
    let hit = e.process_at(
        InputEvent {
            kind: InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 1,
            },
            injected: false,
        },
        100,
    );
    assert_eq!(hit.triggered, [1]);
    pointer(&mut e, None);
    assert!(input(&mut e, InputKind::Up(CAPS)).replay.is_empty());
}

#[test]
fn pointer_scope_replacement_rebuilds_unknown_guards_and_combined_specificity() {
    let mut both = row(2, scope("reader"));
    both.process_scope = scope("editor");
    let mut e = Engine::new(vec![row(1, scope("reader")), both]).unwrap();
    e.set_foreground_process(Some("editor")).unwrap();
    pointer(&mut e, Some("reader"));
    input(&mut e, InputKind::Down(SIDE));
    assert_eq!(wheel(&mut e, 1).triggered, [2]);
    input(&mut e, InputKind::Up(SIDE));
    e.replace_bindings(vec![row(3, Default::default())])
        .unwrap();
    pointer(&mut e, None);
    assert!(!e.needs_pointer_context());
    input(&mut e, InputKind::Down(SIDE));
    assert_eq!(wheel(&mut e, 1).triggered, [3]);
}
