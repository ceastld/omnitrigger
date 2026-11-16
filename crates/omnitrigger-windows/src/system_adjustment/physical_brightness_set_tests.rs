use std::collections::VecDeque;
#[derive(Clone, Debug, PartialEq, Eq)]
enum Operation {
    Read(usize),
    Set(usize, u32),
}
struct Slot {
    reads: VecDeque<Result<PhysicalBrightnessReading, SystemAdjustmentError>>,
    fail_set: bool,
}
struct Fake {
    slots: Vec<Slot>,
    operations: Vec<Operation>,
    declared_count: Option<usize>,
}
impl PhysicalBrightnessPort for Fake {
    fn count(&self) -> usize {
        self.declared_count.unwrap_or(self.slots.len())
    }
    fn label(&self, index: usize) -> String {
        format!("合成物理屏幕{index}")
    }
    fn read(&mut self, index: usize) -> Result<PhysicalBrightnessReading, SystemAdjustmentError> {
        self.operations.push(Operation::Read(index));
        self.slots[index]
            .reads
            .pop_front()
            .expect("不得重读旧句柄或自动重试")
    }
    fn set(&mut self, index: usize, value: u32) -> Result<(), SystemAdjustmentError> {
        self.operations.push(Operation::Set(index, value));
        if self.slots[index].fail_set {
            Err(error())
        } else {
            Ok(())
        }
    }
}
fn error() -> SystemAdjustmentError {
    failure(SystemAdjustmentErrorCode::NativeFailure, "合成拒绝")
}
fn reading(minimum: u32, current: u32, maximum: u32) -> PhysicalBrightnessReading {
    PhysicalBrightnessReading {
        minimum,
        current,
        maximum,
    }
}
fn slot(values: Vec<Result<PhysicalBrightnessReading, SystemAdjustmentError>>) -> Slot {
    Slot {
        reads: values.into(),
        fail_set: false,
    }
}
fn fake(slots: Vec<Slot>) -> Fake {
    Fake {
        slots,
        operations: vec![],
        declared_count: None,
    }
}
#[test]
fn one_logical_screen_steps_every_original_physical_member_with_its_own_range() {
    let mut port = fake(vec![
        slot(vec![Ok(reading(0, 40, 100)), Ok(reading(0, 45, 100))]),
        slot(vec![Ok(reading(10, 70, 210)), Ok(reading(10, 80, 210))]),
    ]);
    let results = step_members(&mut port, BrightnessStepDirection::Increase).unwrap();
    assert_eq!(results.len(), 2);
    for (index, result) in results.iter().enumerate() {
        assert_eq!(result.index, index);
        assert_eq!(result.label, format!("合成物理屏幕{index}"));
        assert!(matches!(
            result.status,
            PhysicalBrightnessStatus::Confirmed { .. }
        ));
    }
    assert_eq!(
        port.operations,
        vec![
            Operation::Read(0),
            Operation::Set(0, 45),
            Operation::Read(0),
            Operation::Read(1),
            Operation::Set(1, 80),
            Operation::Read(1)
        ]
    );
}
#[test]
fn clamp_and_quantization_do_not_create_a_write_or_readback_when_value_is_unchanged() {
    let mut up = fake(vec![
        slot(vec![Ok(reading(0, 100, 100))]),
        slot(vec![Ok(reading(10, 10, 11))]),
    ]);
    let values = step_members(&mut up, BrightnessStepDirection::Increase).unwrap();
    assert!(values
        .iter()
        .all(|value| matches!(value.status, PhysicalBrightnessStatus::Unchanged { .. })));
    assert_eq!(up.operations, vec![Operation::Read(0), Operation::Read(1)]);
    let mut down = fake(vec![slot(vec![Ok(reading(10, 10, 210))])]);
    assert!(matches!(
        step_members(&mut down, BrightnessStepDirection::Decrease).unwrap()[0].status,
        PhysicalBrightnessStatus::Unchanged { .. }
    ));
    assert_eq!(down.operations, vec![Operation::Read(0)]);
}
#[test]
fn invalid_or_unreadable_member_is_not_written_and_does_not_hide_other_members() {
    let mut port = fake(vec![
        slot(vec![Err(error())]),
        slot(vec![Ok(reading(10, 9, 110))]),
        slot(vec![Ok(reading(8, 8, 8))]),
        slot(vec![Ok(reading(0, 20, 100)), Ok(reading(0, 25, 100))]),
    ]);
    let results = step_members(&mut port, BrightnessStepDirection::Increase).unwrap();
    for result in &results[..3] {
        assert!(matches!(
            result.status,
            PhysicalBrightnessStatus::ReadRejected { .. }
        ));
    }
    assert!(matches!(
        results[3].status,
        PhysicalBrightnessStatus::Confirmed { .. }
    ));
    assert_eq!(
        port.operations
            .iter()
            .filter(|event| matches!(event, Operation::Set(..)))
            .cloned()
            .collect::<Vec<_>>(),
        vec![Operation::Set(3, 25)]
    );
}
#[test]
fn failed_write_retains_uncertain_effect_without_retry_and_other_member_has_own_receipt() {
    let mut first = slot(vec![Ok(reading(0, 40, 100))]);
    first.fail_set = true;
    let mut port = fake(vec![
        first,
        slot(vec![Ok(reading(0, 80, 100)), Ok(reading(0, 85, 100))]),
    ]);
    let results = step_members(&mut port, BrightnessStepDirection::Increase).unwrap();
    assert!(matches!(
        results[0].status,
        PhysicalBrightnessStatus::SetUnconfirmed { requested: 45, .. }
    ));
    assert!(matches!(
        results[1].status,
        PhysicalBrightnessStatus::Confirmed { requested: 85, .. }
    ));
    assert_eq!(
        port.operations,
        vec![
            Operation::Read(0),
            Operation::Set(0, 45),
            Operation::Read(1),
            Operation::Set(1, 85),
            Operation::Read(1)
        ]
    );
}
#[test]
fn native_acceptance_does_not_imply_reached_value_and_range_drift_is_visible() {
    let before = reading(0, 40, 100);
    let mut port = fake(vec![
        slot(vec![Ok(before), Err(error())]),
        slot(vec![Ok(before), Ok(reading(0, 101, 100))]),
        slot(vec![Ok(before), Ok(reading(0, 44, 100))]),
        slot(vec![Ok(before), Ok(reading(0, 45, 200))]),
    ]);
    let results = step_members(&mut port, BrightnessStepDirection::Increase).unwrap();
    for result in &results[..2] {
        assert!(matches!(
            result.status,
            PhysicalBrightnessStatus::ReadbackUnknown { requested: 45, .. }
        ));
    }
    for result in &results[2..] {
        assert!(matches!(
            result.status,
            PhysicalBrightnessStatus::ReadbackDifferent { requested: 45, .. }
        ));
    }
    assert_eq!(
        port.operations
            .iter()
            .filter(|event| matches!(event, Operation::Set(..)))
            .count(),
        4
    );
}
#[test]
fn count_limit_rejects_before_any_io_and_maximum_has_one_receipt_per_member() {
    for count in [0, 65, usize::MAX] {
        let mut port = fake(vec![]);
        port.declared_count = Some(count);
        assert!(step_members(&mut port, BrightnessStepDirection::Increase).is_err());
        assert!(port.operations.is_empty());
    }
    let mut port = fake(
        (0..64)
            .map(|_| slot(vec![Ok(reading(0, 100, 100))]))
            .collect(),
    );
    assert_eq!(
        step_members(&mut port, BrightnessStepDirection::Increase)
            .unwrap()
            .len(),
        64
    );
    assert_eq!(port.operations.len(), 64);
}
#[test]
fn decrease_uses_each_device_range_and_extreme_values_do_not_wrap() {
    let mut port = fake(vec![
        slot(vec![Ok(reading(10, 70, 210)), Ok(reading(10, 60, 210))]),
        slot(vec![Ok(reading(0, 2, 100)), Ok(reading(0, 0, 100))]),
    ]);
    let results = step_members(&mut port, BrightnessStepDirection::Decrease).unwrap();
    assert!(matches!(
        results[0].status,
        PhysicalBrightnessStatus::Confirmed { requested: 60, .. }
    ));
    assert!(matches!(
        results[1].status,
        PhysicalBrightnessStatus::Confirmed { requested: 0, .. }
    ));
    // DDC 可设置0，不能将尚未实现的来源WMI零档跳过混进此路径。
    assert_eq!(
        raw_brightness_next(u32::MAX - 100, u32::MAX - 3, u32::MAX, 5)
            .unwrap()
            .0,
        u32::MAX
    );
    assert_eq!(raw_brightness_next(0, 3, u32::MAX, -5).unwrap().0, 0);
}

#[test]
fn guard_cancels_between_members_without_second_native_read_or_retry() {
    use std::cell::Cell;
    let mut port = fake(vec![
        slot(vec![Ok(reading(0, 40, 100)), Ok(reading(0, 45, 100))]),
        slot(vec![]),
    ]);
    let checks = Cell::new(0);
    let result = step_members_if(&mut port, BrightnessStepDirection::Increase, &mut |_| {
        checks.set(checks.get() + 1);
        if checks.get() >= 8 {
            Err(error())
        } else {
            Ok(())
        }
    });
    let members = result.unwrap();
    assert_eq!(members.len(), 2);
    assert!(matches!(
        members[0].status,
        PhysicalBrightnessStatus::Confirmed { requested: 45, .. }
    ));
    assert!(matches!(
        members[1].status,
        PhysicalBrightnessStatus::ReadRejected { .. }
    ));
    assert_eq!(
        port.operations,
        vec![
            Operation::Read(0),
            Operation::Set(0, 45),
            Operation::Read(0)
        ]
    );
}
#[test]
fn guard_rejection_at_write_boundary_never_calls_native_set() {
    let mut port = fake(vec![slot(vec![Ok(reading(0, 40, 100))])]);
    assert!(step_members_if(
        &mut port,
        BrightnessStepDirection::Increase,
        &mut |write| if write { Err(error()) } else { Ok(()) }
    )
    .is_ok());
    assert_eq!(port.operations, vec![Operation::Read(0)]);
}

#[test]
fn cancellation_after_set_retains_unknown_member_and_blocks_remaining_io() {
    use std::cell::Cell;
    let mut port = fake(vec![slot(vec![Ok(reading(0, 40, 100))]), slot(vec![])]);
    let checks = Cell::new(0);
    let members = step_members_if(&mut port, BrightnessStepDirection::Increase, &mut |_| {
        checks.set(checks.get() + 1);
        if checks.get() >= 5 {
            Err(error())
        } else {
            Ok(())
        }
    })
    .unwrap();
    assert!(matches!(
        members[0].status,
        PhysicalBrightnessStatus::SetUnconfirmed { requested: 45, .. }
    ));
    assert!(matches!(
        members[1].status,
        PhysicalBrightnessStatus::ReadRejected { .. }
    ));
    assert_eq!(
        port.operations,
        vec![Operation::Read(0), Operation::Set(0, 45)]
    );
}
