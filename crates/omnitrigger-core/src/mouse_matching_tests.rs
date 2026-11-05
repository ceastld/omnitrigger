use super::*;

const RIGHT: Button = Button::Mouse(MouseButton::Right);
const CTRL: Button = Button::Key(0x11);
const LEFT_CTRL: Button = Button::Key(0xa2);
const RIGHT_CTRL: Button = Button::Key(0xa3);

fn event(kind: InputKind) -> InputEvent {
    InputEvent {
        kind,
        injected: false,
    }
}

fn wheel(delta: i32) -> InputKind {
    InputKind::Wheel {
        axis: Axis::Vertical,
        delta,
    }
}

fn spec(id: u64, modifier: Option<Button>, activation: Activation) -> TriggerSpec {
    TriggerSpec {
        id,
        held: modifier.into_iter().collect(),
        activation,
        keyboard: KeyboardPolicy {
            matching: KeyboardMatching::MouseExact {
                priority: 0,
                pointer_scope_priority: true,
            },
            ..Default::default()
        },
        wheel: WheelPolicy::Packet,
        conditions: vec![],
        pointer_scope: Default::default(),
        process_scope: Default::default(),
    }
}

fn scroll_spec(modifier: Option<Button>) -> TriggerSpec {
    spec(1, modifier, Activation::Wheel(WheelDirection::Up))
}

fn hit(id: u64) -> Decision {
    Decision {
        consume: true,
        triggered: vec![id],
        ..Default::default()
    }
}

fn ordered(mut spec: TriggerSpec, priority: u16) -> TriggerSpec {
    spec.keyboard.matching = KeyboardMatching::MouseExact {
        priority,
        pointer_scope_priority: true,
    };
    spec
}

fn top_boundary(mut spec: TriggerSpec) -> TriggerSpec {
    spec.conditions = vec![TriggerCondition::PointerRegion(
        PointerRegion::ScreenBoundary {
            edge: ScreenEdge::Top,
        },
    )];
    spec
}

#[test]
fn ordered_mouse_rules_choose_matching_priority_independent_of_registration_order() {
    let fallback = ordered(spec(1, None, Activation::Wheel(WheelDirection::Up)), 20);
    let top = top_boundary(ordered(
        spec(2, None, Activation::Wheel(WheelDirection::Up)),
        3,
    ));
    for rows in [
        vec![fallback.clone(), top.clone()],
        vec![top.clone(), fallback.clone()],
    ] {
        let mut engine = Engine::new(rows).unwrap();
        engine
            .set_screens(vec![ScreenRect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100,
            }])
            .unwrap();
        engine.pointer_moved((0, 0), 0);
        assert_eq!(engine.process(event(wheel(1))), hit(2));
        engine.pointer_moved((50, 50), 1);
        assert_eq!(engine.process(event(wheel(240))), hit(1));
        engine.pointer_moved((99, 0), 2);
        assert_eq!(engine.process(event(wheel(120))), hit(2));
    }
}

#[test]
fn pointer_scope_can_filter_without_reordering_mouse_rules_and_legacy_precedence_remains() {
    for pointer_scope_priority in [true, false] {
        let mut top = top_boundary(ordered(scroll_spec(None), 1));
        let mut right = ordered(spec(2, None, Activation::Wheel(WheelDirection::Up)), 2);
        right.pointer_scope.allow = vec!["editor".into()];
        right.conditions = vec![TriggerCondition::PointerRegion(
            PointerRegion::ScreenBoundary {
                edge: ScreenEdge::Right,
            },
        )];
        for row in [&mut top, &mut right] {
            let KeyboardMatching::MouseExact { priority, .. } = row.keyboard.matching else {
                unreachable!()
            };
            row.keyboard.matching = KeyboardMatching::MouseExact {
                priority,
                pointer_scope_priority,
            };
        }
        let mut engine = Engine::new(vec![right, top]).unwrap();
        engine
            .set_pointer_context(Some("editor.exe"), WindowRole::Application)
            .unwrap();
        engine
            .set_screens(vec![ScreenRect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100,
            }])
            .unwrap();
        engine.pointer_moved((99, 0), 0);
        assert_eq!(
            engine.process(event(wheel(120))),
            hit(if pointer_scope_priority { 2 } else { 1 })
        );
        engine.pointer_moved((99, 50), 1);
        assert_eq!(engine.process(event(wheel(120))), hit(2));
        engine
            .set_pointer_context(Some("reader.exe"), WindowRole::Application)
            .unwrap();
        assert_eq!(engine.process(event(wheel(120))), Decision::default());
        engine.pointer_moved((99, 0), 2);
        assert_eq!(engine.process(event(wheel(120))), hit(1));
    }
}

#[test]
fn mouse_order_requires_distinct_numbers_and_valid_boundaries_without_changing_state() {
    let mut row = ordered(scroll_spec(None), 511);
    assert!(Engine::new(vec![row.clone()]).is_ok());
    for priority in [512, u16::MAX] {
        row.keyboard.matching = KeyboardMatching::MouseExact {
            priority,
            pointer_scope_priority: true,
        };
        assert_eq!(
            Engine::new(vec![row.clone()]).err().unwrap(),
            RegistrationError::InvalidMousePriority { id: 1, priority }
        );
    }
    let first = scroll_spec(None);
    let mut second = first.clone();
    second.id = 2;
    assert_eq!(
        Engine::new(vec![first.clone(), second.clone()])
            .err()
            .unwrap(),
        RegistrationError::DuplicateTrigger {
            first: 1,
            second: 2
        }
    );
    let mut engine = Engine::new(vec![first.clone()]).unwrap();
    engine.process(event(InputKind::Down(RIGHT)));
    assert!(engine.replace_bindings(vec![first, second]).is_err());
    assert_eq!(
        engine.process(event(InputKind::Up(RIGHT))),
        Decision::default()
    );
    assert_eq!(engine.process(event(wheel(120))), hit(1));
}

#[test]
fn mouse_order_does_not_override_application_priority_or_extra_held_exclusions() {
    let global = ordered(scroll_spec(Some(CTRL)), 0);
    let mut scoped = ordered(
        spec(2, Some(CTRL), Activation::Wheel(WheelDirection::Up)),
        511,
    );
    scoped.process_scope.allow = vec!["editor".into()];
    let mut engine = Engine::new(vec![global, scoped]).unwrap();
    engine.process(event(InputKind::Down(LEFT_CTRL)));
    assert_eq!(engine.process(event(wheel(120))), Decision::default());
    engine.set_foreground_process(Some("editor.exe")).unwrap();
    assert_eq!(engine.process(event(wheel(120))), hit(2));
    engine.process(event(InputKind::Down(Button::Key(0x41))));
    assert_eq!(engine.process(event(wheel(120))), Decision::default());
    engine.process(event(InputKind::Up(Button::Key(0x41))));
    engine.set_foreground_process(Some("reader.exe")).unwrap();
    assert_eq!(engine.process(event(wheel(120))), hit(1));
}

#[test]
fn ordered_mouse_press_keeps_its_release_and_never_retriggers_after_region_change() {
    let mut fallback = ordered(spec(1, None, Activation::Press(RIGHT)), 7);
    fallback.wheel = WheelPolicy::Notch;
    let mut top = top_boundary(ordered(spec(2, None, Activation::Press(RIGHT)), 1));
    top.wheel = WheelPolicy::Notch;
    let mut engine = Engine::new(vec![fallback, top]).unwrap();
    engine
        .set_screens(vec![ScreenRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        }])
        .unwrap();
    engine.pointer_moved((0, 0), 0);
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), hit(2));
    engine.pointer_moved((50, 50), 1);
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), consumed());
    assert_eq!(engine.process(event(InputKind::Up(RIGHT))), consumed());
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), hit(1));
    assert_eq!(engine.cancel(), Decision::default());
    assert_eq!(engine.process(event(InputKind::Up(RIGHT))), consumed());
}

#[test]
fn ordered_mouse_wheel_replays_old_tail_before_a_higher_priority_region_takes_over() {
    let mut fallback = ordered(scroll_spec(None), 7);
    fallback.wheel = WheelPolicy::Notch;
    let mut top = top_boundary(ordered(
        spec(2, None, Activation::Wheel(WheelDirection::Up)),
        1,
    ));
    top.wheel = WheelPolicy::Notch;
    let mut engine = Engine::new(vec![fallback, top]).unwrap();
    engine
        .set_screens(vec![ScreenRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        }])
        .unwrap();
    engine.pointer_moved((50, 50), 0);
    assert_eq!(engine.process(event(wheel(60))), consumed());
    assert_eq!(engine.pointer_moved((0, 0), 1).replay, [wheel(60)]);
    assert_eq!(engine.process(event(wheel(60))), consumed());
    assert_eq!(engine.process(event(wheel(60))), hit(2));
    assert_eq!(engine.process(event(wheel(40))), consumed());
    assert_eq!(engine.pointer_moved((50, 50), 2).replay, [wheel(40)]);
    assert_eq!(engine.process(event(wheel(120))), hit(1));
}

fn consumed() -> Decision {
    Decision {
        consume: true,
        ..Default::default()
    }
}

#[test]
fn generic_modifiers_observe_each_side_both_sides_repeat_and_release() {
    for (generic, left, right) in [(0x10, 0xa0, 0xa1), (0x11, 0xa2, 0xa3), (0x12, 0xa4, 0xa5)] {
        for sides in [vec![left], vec![right], vec![left, right]] {
            let mut engine = Engine::new(vec![scroll_spec(Some(Button::Key(generic)))]).unwrap();
            for &side in &sides {
                assert_eq!(
                    engine.process_at(event(InputKind::Down(Button::Key(side))), 1),
                    Decision::default()
                );
                assert_eq!(
                    engine.process_at(event(InputKind::Down(Button::Key(side))), 2),
                    Decision::default()
                );
            }
            assert_eq!(engine.process_at(event(wheel(1)), 3), hit(1));
            assert_eq!(
                engine.process_at(event(InputKind::Up(Button::Key(sides[0]))), 4),
                Decision::default()
            );
            if sides.len() == 2 {
                assert_eq!(engine.process_at(event(wheel(1)), 5), hit(1));
                assert_eq!(
                    engine.process_at(event(InputKind::Up(Button::Key(sides[1]))), 6),
                    Decision::default()
                );
            }
            assert_eq!(engine.process_at(event(wheel(120)), 7), Decision::default());
        }
    }
}

#[test]
fn explicit_modifier_requires_its_side_and_excludes_an_extra_side() {
    for key in [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c] {
        let mut engine = Engine::new(vec![scroll_spec(Some(Button::Key(key)))]).unwrap();
        let other = if key == 0x5b {
            0x5c
        } else if key == 0x5c {
            0x5b
        } else {
            key ^ 1
        };
        assert_eq!(
            engine.process(event(InputKind::Down(Button::Key(other)))),
            Decision::default()
        );
        assert_eq!(engine.process(event(wheel(120))), Decision::default());
        engine.process(event(InputKind::Down(Button::Key(key))));
        assert_eq!(engine.process(event(wheel(120))), Decision::default());
        engine.process(event(InputKind::Up(Button::Key(other))));
        assert_eq!(engine.process(event(wheel(120))), hit(1));
        assert_eq!(
            engine.process(event(InputKind::Up(Button::Key(key)))),
            Decision::default()
        );
    }
}

#[test]
fn unmodified_mouse_rule_excludes_every_extra_held_input_until_release() {
    let keys = [
        0x10, 0x11, 0x12, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c, 0x20, 0x41,
    ];
    let buttons = keys.into_iter().map(Button::Key).chain(
        [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::X1,
            MouseButton::X2,
        ]
        .into_iter()
        .map(Button::Mouse),
    );
    for button in buttons {
        let mut engine = Engine::new(vec![scroll_spec(None)]).unwrap();
        assert_eq!(engine.process(event(wheel(120))), hit(1));
        assert_eq!(
            engine.process(event(InputKind::Down(button))),
            Decision::default()
        );
        assert_eq!(
            engine.process(event(InputKind::Down(button))),
            Decision::default()
        );
        assert_eq!(engine.process(event(wheel(120))), Decision::default());
        assert_eq!(
            engine.process(event(InputKind::Up(button))),
            Decision::default()
        );
        assert_eq!(engine.process(event(wheel(120))), hit(1));
    }
}

#[test]
fn unmatched_mouse_press_stays_passed_after_extra_key_is_released() {
    let mut binding = spec(1, None, Activation::Press(RIGHT));
    binding.wheel = WheelPolicy::Notch;
    let mut engine = Engine::new(vec![binding]).unwrap();
    let extra = Button::Key(0x41);
    assert_eq!(
        engine.process(event(InputKind::Down(extra))),
        Decision::default()
    );
    assert_eq!(
        engine.process(event(InputKind::Down(RIGHT))),
        Decision::default()
    );
    assert_eq!(
        engine.process(event(InputKind::Up(extra))),
        Decision::default()
    );
    assert_eq!(
        engine.process(event(InputKind::Down(RIGHT))),
        Decision::default()
    );
    assert_eq!(
        engine.process(event(InputKind::Up(RIGHT))),
        Decision::default()
    );
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), hit(1));
    assert_eq!(engine.process(event(InputKind::Up(RIGHT))), consumed());
    assert_eq!(
        engine.process(InputEvent {
            kind: InputKind::Down(extra),
            injected: true
        }),
        Decision::default()
    );
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), hit(1));
    assert_eq!(engine.process(event(InputKind::Up(RIGHT))), consumed());
}

#[test]
fn modifier_rule_rejects_extra_keys_and_mouse_buttons_without_capturing_them() {
    for extra in [
        Button::Key(0x20),
        Button::Key(0x5b),
        Button::Key(0xa0),
        Button::Mouse(MouseButton::Left),
        Button::Mouse(MouseButton::X1),
        RIGHT,
    ] {
        let mut engine = Engine::new(vec![scroll_spec(Some(CTRL))]).unwrap();
        engine.process(event(InputKind::Down(LEFT_CTRL)));
        assert_eq!(
            engine.process(event(InputKind::Down(extra))),
            Decision::default()
        );
        assert_eq!(engine.process(event(wheel(120))), Decision::default());
        assert_eq!(
            engine.process(event(InputKind::Up(extra))),
            Decision::default()
        );
        assert_eq!(engine.process(event(wheel(120))), hit(1));
        assert_eq!(
            engine.process(event(InputKind::Up(LEFT_CTRL))),
            Decision::default()
        );
    }
}

#[test]
fn injected_events_never_change_observed_modifiers_or_retrigger_mouse_terminal() {
    let mut binding = spec(1, Some(CTRL), Activation::Press(RIGHT));
    binding.wheel = WheelPolicy::Notch;
    let mut engine = Engine::new(vec![binding]).unwrap();
    for kind in [
        InputKind::Down(LEFT_CTRL),
        InputKind::Down(RIGHT),
        wheel(120),
    ] {
        assert_eq!(
            engine.process_at(
                InputEvent {
                    kind,
                    injected: true
                },
                999
            ),
            Decision::default()
        );
    }
    assert_eq!(
        engine.process_at(event(InputKind::Down(RIGHT)), 1),
        Decision::default()
    );
    engine.process(event(InputKind::Up(RIGHT)));
    assert_eq!(
        engine.process_at(event(InputKind::Down(LEFT_CTRL)), 2),
        Decision::default()
    );
    assert_eq!(
        engine.process_at(
            InputEvent {
                kind: InputKind::Up(LEFT_CTRL),
                injected: true
            },
            999
        ),
        Decision::default()
    );
    assert_eq!(engine.process_at(event(InputKind::Down(RIGHT)), 3), hit(1));
    assert_eq!(
        engine.process_at(event(InputKind::Down(RIGHT)), 4),
        consumed()
    );
    assert_eq!(
        engine.process_at(
            InputEvent {
                kind: InputKind::Up(RIGHT),
                injected: true
            },
            999
        ),
        Decision::default()
    );
    assert_eq!(
        engine.process_at(event(InputKind::Up(LEFT_CTRL)), 5),
        Decision::default()
    );
    assert_eq!(
        engine.process_at(event(InputKind::Up(RIGHT)), 6),
        consumed()
    );
}

#[test]
fn cancellation_and_replacement_drain_consumed_terminal_but_pass_modifier_release() {
    let mut binding = spec(1, Some(CTRL), Activation::Press(RIGHT));
    binding.wheel = WheelPolicy::Notch;
    let mut engine = Engine::new(vec![binding.clone()]).unwrap();
    engine.process(event(InputKind::Down(LEFT_CTRL)));
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), hit(1));
    assert_eq!(engine.cancel(), Decision::default());
    assert_eq!(
        engine.replace_bindings(vec![binding]).unwrap(),
        Decision::default()
    );
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), consumed());
    assert_eq!(
        engine.process(event(InputKind::Up(LEFT_CTRL))),
        Decision::default()
    );
    assert_eq!(engine.process(event(InputKind::Up(RIGHT))), consumed());
    engine.set_enabled(false);
    engine.process(event(InputKind::Down(LEFT_CTRL)));
    assert_eq!(
        engine.process(event(InputKind::Down(RIGHT))),
        Decision::default()
    );
    engine.set_enabled(true);
    assert_eq!(
        engine.process(event(InputKind::Down(RIGHT))),
        Decision::default()
    );
    engine.process(event(InputKind::Up(RIGHT)));
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), hit(1));
}

#[test]
fn scope_changes_keep_consumed_mouse_release_owned_and_modifiers_observed() {
    let mut binding = spec(1, Some(CTRL), Activation::Press(RIGHT));
    binding.wheel = WheelPolicy::Notch;
    binding.pointer_scope.allow = vec!["editor".into()];
    let mut engine = Engine::new(vec![binding]).unwrap();
    engine
        .set_pointer_context(Some("editor.exe"), WindowRole::Application)
        .unwrap();
    engine.process(event(InputKind::Down(RIGHT_CTRL)));
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), hit(1));
    engine
        .set_pointer_context(Some("other.exe"), WindowRole::Application)
        .unwrap();
    assert_eq!(engine.process(event(InputKind::Up(RIGHT))), consumed());
    assert_eq!(
        engine.process(event(InputKind::Down(RIGHT))),
        Decision::default()
    );
    engine
        .set_pointer_context(Some("editor.exe"), WindowRole::Application)
        .unwrap();
    assert_eq!(
        engine.process(event(InputKind::Down(RIGHT))),
        Decision::default()
    );
    engine.process(event(InputKind::Up(RIGHT)));
    assert_eq!(engine.process(event(InputKind::Down(RIGHT))), hit(1));
    engine.set_foreground_process(Some("changed.exe")).unwrap();
    assert_eq!(engine.process(event(InputKind::Up(RIGHT))), consumed());
    assert_eq!(
        engine.process(event(InputKind::Up(RIGHT_CTRL))),
        Decision::default()
    );
}

#[test]
fn partial_notches_replay_on_modifier_release_or_extra_input_without_leaking_modifier_edges() {
    for ending in [InputKind::Up(LEFT_CTRL), InputKind::Down(Button::Key(0x20))] {
        let mut binding = scroll_spec(Some(CTRL));
        binding.wheel = WheelPolicy::Notch;
        let mut engine = Engine::new(vec![binding]).unwrap();
        assert_eq!(
            engine.process(event(InputKind::Down(LEFT_CTRL))),
            Decision::default()
        );
        assert_eq!(engine.process(event(wheel(60))), consumed());
        assert_eq!(
            engine.process(event(ending)),
            Decision {
                replay: vec![wheel(60)],
                ..Default::default()
            }
        );
        assert_eq!(engine.process(event(wheel(60))), Decision::default());
        if matches!(ending, InputKind::Down(_)) {
            engine.process(event(InputKind::Up(Button::Key(0x20))));
        } else {
            engine.process(event(InputKind::Down(LEFT_CTRL)));
        }
        assert_eq!(engine.process(event(wheel(60))), consumed());
        assert_eq!(engine.process(event(wheel(60))), hit(1));
        assert_eq!(
            engine.process(event(InputKind::Up(LEFT_CTRL))),
            Decision::default()
        );
    }
}

#[test]
fn leaving_region_replays_partial_notch_and_reentry_starts_fresh() {
    let mut binding = scroll_spec(Some(CTRL));
    binding.wheel = WheelPolicy::Notch;
    binding.conditions = vec![TriggerCondition::PointerRegion(PointerRegion::Rectangle(
        ScreenRect {
            left: 0,
            top: 0,
            right: 10,
            bottom: 10,
        },
    ))];
    let mut engine = Engine::new(vec![binding]).unwrap();
    engine.pointer_moved((0, 0), 0);
    engine.process(event(InputKind::Down(LEFT_CTRL)));
    assert_eq!(engine.process(event(wheel(60))), consumed());
    assert_eq!(
        engine.pointer_moved((10, 0), 1),
        Decision {
            replay: vec![wheel(60)],
            ..Default::default()
        }
    );
    assert_eq!(engine.process(event(wheel(60))), Decision::default());
    engine.pointer_moved((0, 0), 2);
    assert_eq!(engine.process(event(wheel(60))), consumed());
    assert_eq!(engine.process(event(wheel(60))), hit(1));
    assert_eq!(
        engine.process(event(InputKind::Up(LEFT_CTRL))),
        Decision::default()
    );
}

#[test]
fn another_trigger_cannot_commit_and_discard_an_observed_partial_notch() {
    let mut mouse = scroll_spec(None);
    mouse.wheel = WheelPolicy::Notch;
    let key = Button::Key(0x41);
    let keyboard = TriggerSpec {
        id: 2,
        held: vec![],
        activation: Activation::Press(key),
        keyboard: Default::default(),
        wheel: Default::default(),
        conditions: vec![],
        pointer_scope: Default::default(),
        process_scope: Default::default(),
    };
    let mut engine = Engine::new(vec![mouse, keyboard]).unwrap();
    assert_eq!(engine.process(event(wheel(60))), consumed());
    assert_eq!(
        engine.process(event(InputKind::Down(key))),
        Decision {
            consume: true,
            replay: vec![wheel(60)],
            triggered: vec![2],
            ..Default::default()
        }
    );
    assert_eq!(engine.process(event(wheel(120))), Decision::default());
    assert_eq!(engine.process(event(InputKind::Up(key))), consumed());
    assert_eq!(engine.process(event(wheel(60))), consumed());
    assert_eq!(engine.process(event(wheel(60))), hit(1));
}

#[test]
fn generic_and_side_specific_rules_conflict_when_physical_sets_overlap() {
    for modifier in [LEFT_CTRL, RIGHT_CTRL, CTRL] {
        let left = scroll_spec(Some(CTRL));
        let mut right = scroll_spec(Some(modifier));
        right.id = 2;
        assert!(matches!(
            Engine::new(vec![left, right]),
            Err(RegistrationError::DuplicateTrigger {
                first: 1,
                second: 2
            })
        ));
    }
    let left = scroll_spec(Some(LEFT_CTRL));
    let mut right = scroll_spec(Some(RIGHT_CTRL));
    right.id = 2;
    assert!(Engine::new(vec![left, right]).is_ok());
}

#[test]
fn observed_and_captured_modifier_delivery_conflict_is_structured() {
    for activation in [
        Activation::Press(Button::Key(0x41)),
        Activation::Press(LEFT_CTRL),
    ] {
        let observed = scroll_spec(Some(CTRL));
        let captured = TriggerSpec {
            id: 2,
            held: if activation == Activation::Press(LEFT_CTRL) {
                vec![]
            } else {
                vec![LEFT_CTRL]
            },
            activation,
            keyboard: Default::default(),
            wheel: Default::default(),
            conditions: vec![],
            pointer_scope: Default::default(),
            process_scope: Default::default(),
        };
        assert!(matches!(
            Engine::new(vec![observed.clone(), captured.clone()]),
            Err(RegistrationError::ConflictingModifierDelivery {
                first: 1,
                second: 2
            })
        ));
        let mut a = observed;
        a.process_scope.allow = vec!["editor".into()];
        let mut b = captured;
        b.process_scope.allow = vec!["reader".into()];
        assert!(Engine::new(vec![a, b]).is_ok());
    }
}

#[test]
fn invalid_mouse_policies_never_fall_back_to_another_matching_mode() {
    let mut variants = vec![];
    let base = scroll_spec(Some(CTRL));
    let mut row = base.clone();
    row.held.push(LEFT_CTRL);
    variants.push(row);
    let mut row = base.clone();
    row.held = vec![Button::Key(0x20)];
    variants.push(row);
    let mut row = base.clone();
    row.held = vec![RIGHT];
    variants.push(row);
    let mut row = base.clone();
    row.activation = Activation::Press(Button::Key(0x41));
    variants.push(row);
    let mut row = base.clone();
    row.activation = Activation::Hold {
        button: RIGHT,
        duration_ms: 100,
    };
    variants.push(row);
    let mut row = base.clone();
    row.keyboard.repeat = true;
    variants.push(row);
    let mut row = base.clone();
    row.keyboard.dedicated_prefix = Some(CTRL);
    variants.push(row);
    let mut row = base.clone();
    row.keyboard.restore_on_tap = true;
    variants.push(row);
    let mut row = base.clone();
    row.keyboard.tap_timeout_ms = Some(100);
    variants.push(row);
    let mut row = base;
    row.keyboard.tap_scope.allow = vec!["editor".into()];
    variants.push(row);
    for row in variants {
        assert!(matches!(
            Engine::new(vec![row]),
            Err(RegistrationError::InvalidMouseMatching { id: 1 })
        ));
    }
}

#[test]
fn overlapping_generic_and_side_scopes_wait_for_known_identity_and_choose_priority() {
    let global = scroll_spec(Some(CTRL));
    let mut scoped = scroll_spec(Some(LEFT_CTRL));
    scoped.id = 2;
    scoped.process_scope.allow = vec!["editor".into()];
    let mut engine = Engine::new(vec![global, scoped]).unwrap();
    engine.process(event(InputKind::Down(LEFT_CTRL)));
    assert_eq!(engine.process(event(wheel(120))), Decision::default());
    engine.set_foreground_process(Some("editor.exe")).unwrap();
    assert_eq!(engine.process(event(wheel(120))), hit(2));
    engine.process(event(InputKind::Up(LEFT_CTRL)));
    engine.process(event(InputKind::Down(RIGHT_CTRL)));
    assert_eq!(engine.process(event(wheel(120))), hit(1));
    engine.set_foreground_process(Some("reader.exe")).unwrap();
    assert_eq!(engine.process(event(wheel(120))), hit(1));
}
