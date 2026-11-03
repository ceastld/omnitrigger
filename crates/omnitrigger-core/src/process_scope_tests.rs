use crate::*;

const CAPS: Button = Button::Key(20);
const K: Button = Button::Key(75);
const J: Button = Button::Key(74);

fn scope(allow: &[&str], deny: &[&str]) -> ProcessScope {
    ProcessScope {
        identity: Default::default(),
        allow: allow.iter().map(|s| s.to_string()).collect(),
        deny: deny.iter().map(|s| s.to_string()).collect(),
    }
}
fn press(id: u64, process_scope: ProcessScope) -> TriggerSpec {
    TriggerSpec {
        pointer_scope: Default::default(),
        wheel: Default::default(),
        keyboard: Default::default(),
        id,
        held: vec![CAPS],
        activation: Activation::Press(K),
        conditions: vec![],
        process_scope,
    }
}
fn input(engine: &mut Engine, kind: InputKind) -> Decision {
    engine.process(InputEvent {
        kind,
        injected: false,
    })
}
fn chord(engine: &mut Engine) -> Vec<u64> {
    input(engine, InputKind::Down(CAPS));
    let hit = input(engine, InputKind::Down(K));
    input(engine, InputKind::Up(K));
    input(engine, InputKind::Up(CAPS));
    hit.triggered
}

fn shell(allow: &[&str], deny: &[&str]) -> ProcessScope {
    ProcessScope {
        identity: ProcessIdentity::ShellContext,
        ..scope(allow, deny)
    }
}

#[test]
fn shell_context_roles_choose_disjoint_rules_without_replacing_the_real_process() {
    let mut engine = Engine::new(vec![
        press(1, shell(&["explorer"], &[])),
        press(2, shell(&["desktop"], &[])),
        press(3, shell(&["taskbar"], &[])),
        press(4, scope(&[], &[])),
    ])
    .unwrap();
    assert!(engine.needs_shell_context());
    for (role, expected) in [
        (WindowRole::Application, 1),
        (WindowRole::Desktop, 2),
        (WindowRole::Taskbar, 3),
    ] {
        engine
            .set_foreground_context(Some("Explorer.EXE"), role)
            .unwrap();
        assert_eq!(chord(&mut engine), vec![expected]);
        assert_eq!(engine.foreground_process(), Some("explorer"));
    }
    engine
        .set_foreground_context(Some("editor"), WindowRole::Application)
        .unwrap();
    assert_eq!(chord(&mut engine), vec![4]);
    engine
        .set_foreground_context(Some("explorer"), WindowRole::Unknown)
        .unwrap();
    assert!(chord(&mut engine).is_empty());
    engine
        .set_foreground_context(None, WindowRole::Desktop)
        .unwrap();
    assert!(chord(&mut engine).is_empty());
}

#[test]
fn shell_context_executable_domain_preserves_legacy_explorer_and_literal_aliases() {
    let mut engine = Engine::new(vec![press(1, scope(&["explorer"], &[]))]).unwrap();
    assert!(!engine.needs_shell_context());
    for role in [
        WindowRole::Unknown,
        WindowRole::Application,
        WindowRole::Desktop,
        WindowRole::Taskbar,
    ] {
        engine
            .set_foreground_context(Some("explorer.exe"), role)
            .unwrap();
        assert_eq!(chord(&mut engine), vec![1]);
    }
    assert!(!scope(&["desktop"], &[]).matches_context(Some("explorer"), WindowRole::Desktop));
    assert!(scope(&["desktop"], &[]).matches_context(Some("desktop"), WindowRole::Application));
}

#[test]
fn shell_context_overlap_checks_both_domains_instead_of_alias_spelling() {
    assert!(scope(&["explorer"], &[]).overlaps(&shell(&["desktop"], &[])));
    assert!(!shell(&["explorer"], &[]).overlaps(&shell(&["desktop"], &[])));
    assert!(!shell(&["taskbar"], &[]).overlaps(&shell(&[], &["taskbar"])));
    assert!(matches!(
        Engine::new(vec![
            press(1, scope(&["explorer"], &[])),
            press(2, shell(&["desktop"], &[]))
        ]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
}

#[test]
fn shell_context_role_change_cancels_hold_even_when_process_name_is_unchanged() {
    let mut binding = press(1, shell(&["explorer"], &[]));
    binding.activation = Activation::Hold {
        button: K,
        duration_ms: 100,
    };
    let mut engine = Engine::new(vec![binding]).unwrap();
    engine
        .set_foreground_context(Some("explorer"), WindowRole::Application)
        .unwrap();
    input(&mut engine, InputKind::Down(CAPS));
    input(&mut engine, InputKind::Down(K));
    assert_eq!(
        engine
            .set_foreground_context(Some("Explorer.exe"), WindowRole::Application)
            .unwrap(),
        Decision::default()
    );
    assert_eq!(engine.next_deadline_ms(), Some(100));
    engine
        .set_foreground_context(Some("explorer"), WindowRole::Taskbar)
        .unwrap();
    assert!(engine.advance_time(100).triggered.is_empty());
    assert!(input(&mut engine, InputKind::Up(K)).consume);
    assert!(input(&mut engine, InputKind::Up(CAPS)).consume);
    assert!(chord(&mut engine).is_empty());
}

#[test]
fn shell_context_unknown_role_blocks_captured_fallback_but_not_unrelated_triggers() {
    let mut scoped = press(1, shell(&["desktop"], &[]));
    let mut global = press(2, scope(&[], &[]));
    global.activation = Activation::Press(J);
    for binding in [&mut scoped, &mut global] {
        binding.keyboard.matching = KeyboardMatching::Captured {
            priority: binding.id as u16,
        };
        binding.keyboard.dedicated_prefix = Some(CAPS);
    }
    let mut unrelated = press(3, scope(&[], &[]));
    unrelated.held = vec![];
    unrelated.activation = Activation::Press(Button::Key(80));
    let mut engine = Engine::new(vec![scoped, global, unrelated]).unwrap();
    engine.set_foreground_process(Some("explorer")).unwrap();
    assert!(!input(&mut engine, InputKind::Down(CAPS)).consume);
    assert!(input(&mut engine, InputKind::Down(J)).triggered.is_empty());
    input(&mut engine, InputKind::Up(J));
    input(&mut engine, InputKind::Up(CAPS));
    assert_eq!(
        input(&mut engine, InputKind::Down(Button::Key(80))).triggered,
        vec![3]
    );
}

#[test]
fn shell_context_retained_capture_and_tap_scopes_are_independent_from_action_scope() {
    let mut binding = press(1, shell(&["editor"], &[]));
    binding.keyboard = KeyboardPolicy {
        matching: KeyboardMatching::Captured { priority: 0 },
        retained_prefix: Some(Box::new(RetainedPrefix {
            key: CAPS,
            capture_scope: shell(&[], &["taskbar"]),
            quiet_ms: 0,
            fast_ms: 0,
            block_with_primary_mouse: false,
        })),
        restore_on_tap: true,
        tap_timeout_ms: Some(1500),
        tap_scope: shell(&["desktop"], &[]),
        ..Default::default()
    };
    let mut engine = Engine::new(vec![binding]).unwrap();
    for (role, captured, restored) in [
        (WindowRole::Unknown, false, false),
        (WindowRole::Taskbar, false, false),
        (WindowRole::Application, true, false),
        (WindowRole::Desktop, true, true),
    ] {
        engine
            .set_foreground_context(Some("explorer"), role)
            .unwrap();
        assert_eq!(input(&mut engine, InputKind::Down(CAPS)).consume, captured);
        let up = input(&mut engine, InputKind::Up(CAPS));
        assert_eq!(!up.replay.is_empty(), restored);
    }
}

#[test]
fn application_override_and_global_fallback_ignore_registration_order() {
    for reverse in [false, true] {
        let mut bindings = vec![
            press(1, scope(&[], &[])),
            press(2, scope(&["Code.EXE", "cursor"], &[])),
        ];
        if reverse {
            bindings.reverse();
        }
        let mut engine = Engine::new(bindings).unwrap();
        // 无法确认是否处于 Code 时，不允许误用全局回退。
        assert!(chord(&mut engine).is_empty());
        engine.set_foreground_process(Some("CODE.exe")).unwrap();
        assert_eq!(chord(&mut engine), vec![2]);
        engine.set_foreground_process(Some("other.exe")).unwrap();
        assert_eq!(chord(&mut engine), vec![1]);
        engine.set_foreground_process(None).unwrap();
        assert!(chord(&mut engine).is_empty());
    }
}

#[test]
fn allow_and_deny_ranges_register_disjoint_apps_and_reject_ambiguity() {
    let allowed = press(1, scope(&["code", "cursor"], &["cursor"]));
    let denied = press(2, scope(&[], &["code"]));
    let mut engine = Engine::new(vec![denied.clone(), allowed.clone()]).unwrap();
    engine.set_foreground_process(Some("cursor")).unwrap();
    assert_eq!(chord(&mut engine), vec![2]);
    engine.set_foreground_process(Some("code")).unwrap();
    assert_eq!(chord(&mut engine), vec![1]);
    assert!(matches!(
        Engine::new(vec![allowed, press(3, scope(&["CODE.exe"], &[]))]),
        Err(RegistrationError::DuplicateTrigger {
            first: 1,
            second: 3
        })
    ));
    assert!(
        Engine::new(vec![
            press(1, scope(&["one"], &[])),
            press(2, scope(&["two"], &[]))
        ])
        .is_ok()
    );
    assert!(matches!(
        Engine::new(vec![denied, press(3, scope(&[], &["other"]))]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
}

#[test]
fn unknown_context_does_not_block_unrelated_global_combinations() {
    let mut global = press(1, scope(&[], &[]));
    global.activation = Activation::Press(J);
    let mut engine = Engine::new(vec![global, press(2, scope(&["editor"], &[]))]).unwrap();
    input(&mut engine, InputKind::Down(CAPS));
    assert_eq!(input(&mut engine, InputKind::Down(J)).triggered, vec![1]);
}

#[test]
fn malformed_scope_or_context_cannot_cancel_an_existing_pending_press() {
    let mut engine = Engine::new(vec![press(1, scope(&["editor"], &[]))]).unwrap();
    engine.set_foreground_process(Some("editor")).unwrap();
    input(&mut engine, InputKind::Down(CAPS));
    assert!(
        engine
            .set_foreground_process(Some("C:\\editor.exe"))
            .is_err()
    );
    for invalid in [
        scope(&["Editor", "editor.exe"], &[]),
        scope(&["a"], &["a.exe"]),
        scope(&["*.exe"], &[]),
        scope(&[""], &[]),
        scope(&[".exe"], &[]),
        scope(&[" editor"], &[]),
        ProcessScope {
            identity: Default::default(),
            allow: vec!["a".into(); 65],
            deny: vec![],
        },
    ] {
        assert!(matches!(
            engine.replace_bindings(vec![press(9, invalid)]),
            Err(RegistrationError::InvalidProcessScope { id: 9, .. })
        ));
    }
    assert_eq!(input(&mut engine, InputKind::Down(K)).triggered, vec![1]);
}

#[test]
fn process_change_drains_old_press_and_does_not_borrow_a_passed_prefix() {
    let mut engine = Engine::new(vec![press(1, scope(&["editor"], &[]))]).unwrap();
    engine.set_foreground_process(Some("editor")).unwrap();
    input(&mut engine, InputKind::Down(CAPS));
    engine.set_foreground_process(Some("other")).unwrap();
    assert!(input(&mut engine, InputKind::Up(CAPS)).consume);
    assert!(!input(&mut engine, InputKind::Down(CAPS)).consume);
    engine.set_foreground_process(Some("editor")).unwrap();
    assert_eq!(input(&mut engine, InputKind::Down(K)), Decision::default());
    assert!(!input(&mut engine, InputKind::Up(K)).consume);
    assert!(!input(&mut engine, InputKind::Up(CAPS)).consume);
    assert_eq!(chord(&mut engine), vec![1]);
}

#[test]
fn app_hold_overrides_global_press_and_equal_context_keeps_its_timer() {
    let mut timed = press(2, scope(&["editor"], &[]));
    timed.activation = Activation::Hold {
        button: K,
        duration_ms: 100,
    };
    let mut engine = Engine::new(vec![press(1, scope(&[], &[])), timed]).unwrap();
    engine.set_foreground_process(Some("Editor.EXE")).unwrap();
    input(&mut engine, InputKind::Down(CAPS));
    assert!(input(&mut engine, InputKind::Down(K)).triggered.is_empty());
    assert_eq!(engine.next_deadline_ms(), Some(100));
    assert_eq!(
        engine.set_foreground_process(Some("editor")).unwrap(),
        Decision::default()
    );
    assert_eq!(engine.advance_time(100).triggered, vec![2]);
    engine.set_foreground_process(Some("other")).unwrap();
    assert!(input(&mut engine, InputKind::Up(K)).consume);
    assert!(input(&mut engine, InputKind::Up(CAPS)).consume);
    assert_eq!(chord(&mut engine), vec![1]);
}

#[test]
fn process_change_cancels_hold_and_discards_wheel_remainders() {
    let mut timed = press(1, scope(&["editor"], &[]));
    timed.activation = Activation::Hold {
        button: K,
        duration_ms: 100,
    };
    let mut wheel = press(2, scope(&["editor"], &[]));
    wheel.activation = Activation::Wheel(WheelDirection::Up);
    let mut engine = Engine::new(vec![timed, wheel]).unwrap();
    engine.set_foreground_process(Some("editor")).unwrap();
    input(&mut engine, InputKind::Down(CAPS));
    input(&mut engine, InputKind::Down(K));
    engine.set_foreground_process(None).unwrap();
    assert!(engine.advance_time(200).triggered.is_empty());
    input(&mut engine, InputKind::Up(K));
    input(&mut engine, InputKind::Up(CAPS));
    engine.set_foreground_process(Some("editor")).unwrap();
    input(&mut engine, InputKind::Down(CAPS));
    let half = InputKind::Wheel {
        axis: Axis::Vertical,
        delta: 60,
    };
    assert!(input(&mut engine, half).triggered.is_empty());
    engine.set_foreground_process(Some("other")).unwrap();
    input(&mut engine, InputKind::Up(CAPS));
    engine.set_foreground_process(Some("editor")).unwrap();
    input(&mut engine, InputKind::Down(CAPS));
    assert!(input(&mut engine, half).triggered.is_empty());
    assert_eq!(input(&mut engine, half).triggered, vec![2]);
}

#[test]
fn scope_and_region_both_apply_to_an_override() {
    let mut scoped = press(2, scope(&["editor"], &[]));
    scoped.conditions = vec![TriggerCondition::PointerRegion(PointerRegion::Rectangle(
        ScreenRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        },
    ))];
    let mut engine = Engine::new(vec![press(1, scope(&[], &[])), scoped]).unwrap();
    engine.set_foreground_process(Some("editor")).unwrap();
    engine.pointer_moved((10, 10), 0);
    assert_eq!(chord(&mut engine), vec![2]);
    engine.pointer_moved((200, 10), 0);
    assert_eq!(chord(&mut engine), vec![1]);
}

#[test]
fn terminal_prefix_conflicts_respect_disjoint_processes() {
    let terminal = TriggerSpec {
        pointer_scope: Default::default(),
        wheel: Default::default(),
        keyboard: Default::default(),
        id: 1,
        held: vec![],
        activation: Activation::Press(CAPS),
        conditions: vec![],
        process_scope: scope(&["one"], &[]),
    };
    assert!(Engine::new(vec![terminal.clone(), press(2, scope(&["two"], &[]))]).is_ok());
    assert!(matches!(
        Engine::new(vec![terminal, press(2, scope(&["one"], &[]))]),
        Err(RegistrationError::TerminalIsPrefix { .. })
    ));
}

#[test]
fn process_change_cancels_a_claimed_drag_and_replacement_rebuilds_scopes() {
    let right = Button::Mouse(MouseButton::Right);
    let drag = TriggerSpec {
        pointer_scope: Default::default(),
        wheel: Default::default(),
        keyboard: Default::default(),
        id: 3,
        held: vec![],
        process_scope: scope(&["editor"], &[]),
        conditions: vec![],
        activation: Activation::Drag {
            button: right,
            min_distance_px: 4,
            max_start_ms: 100,
        },
    };
    let mut engine = Engine::new(vec![drag]).unwrap();
    engine.set_foreground_process(Some("editor")).unwrap();
    engine.pointer_moved((0, 0), 0);
    input(&mut engine, InputKind::Down(right));
    let started = engine.pointer_moved((4, 0), 10);
    assert_eq!(started.drags[0].phase, DragPhase::Started);
    let cancelled = engine.set_foreground_process(Some("other")).unwrap();
    assert_eq!(cancelled.drags[0].phase, DragPhase::Cancelled);
    assert_eq!(cancelled.drags[0].session, started.drags[0].session);
    assert!(input(&mut engine, InputKind::Up(right)).consume);
    engine
        .replace_bindings(vec![
            press(1, scope(&[], &[])),
            press(2, scope(&["editor"], &[])),
        ])
        .unwrap();
    assert_eq!(chord(&mut engine), vec![1]);
    engine.set_foreground_process(Some("editor")).unwrap();
    assert_eq!(chord(&mut engine), vec![2]);
    engine.set_foreground_process(None).unwrap();
    engine
        .replace_bindings(vec![press(1, scope(&[], &[]))])
        .unwrap();
    assert_eq!(chord(&mut engine), vec![1]);
}
