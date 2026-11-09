//! 对已选择逻辑屏幕的物理显示器集合逐台执行 DDC 步进；不创建执行器。
use super::SystemAdjustmentError;
#[cfg(any(windows, test))]
use super::{failure, raw_brightness_next, SystemAdjustmentErrorCode};

pub const MAX_PHYSICAL_BRIGHTNESS_TARGETS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrightnessStepDirection {
    Increase,
    Decrease,
}
impl BrightnessStepDirection {
    #[cfg(any(windows, test))]
    fn delta(self) -> i16 {
        match self {
            Self::Increase => 5,
            Self::Decrease => -5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalBrightnessReading {
    pub minimum: u32,
    pub current: u32,
    pub maximum: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhysicalBrightnessStatus {
    /// 读取失败或范围无效，本台没有调用写接口。
    ReadRejected { error: SystemAdjustmentError },
    /// 量化/边界导致值未改变，本台没有调用写接口。
    Unchanged { reading: PhysicalBrightnessReading },
    /// 写接口失败可能已有影响，不改投其他实例或自动重试。
    SetUnconfirmed {
        before: PhysicalBrightnessReading,
        requested: u32,
        error: SystemAdjustmentError,
    },
    ReadbackUnknown {
        before: PhysicalBrightnessReading,
        requested: u32,
        error: SystemAdjustmentError,
    },
    /// 原生读回成功，但范围变化或当前值不同，不称为已达到请求值。
    ReadbackDifferent {
        before: PhysicalBrightnessReading,
        requested: u32,
        observed: PhysicalBrightnessReading,
    },
    Confirmed {
        before: PhysicalBrightnessReading,
        requested: u32,
        observed: PhysicalBrightnessReading,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalBrightnessMemberResult {
    /// 本次固定物理句柄集合中的序号，不作为跨调用设备标识。
    pub index: usize,
    pub label: String,
    pub status: PhysicalBrightnessStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalBrightnessSetResult {
    pub point: (i32, i32),
    pub logical_device: String,
    pub members: Vec<PhysicalBrightnessMemberResult>,
}

#[cfg(any(windows, test))]
pub(super) trait PhysicalBrightnessPort {
    fn count(&self) -> usize;
    fn label(&self, index: usize) -> String;
    fn read(&mut self, index: usize) -> Result<PhysicalBrightnessReading, SystemAdjustmentError>;
    fn set(&mut self, index: usize, value: u32) -> Result<(), SystemAdjustmentError>;
}

#[cfg(any(windows, test))]
pub(super) fn step_members(
    port: &mut impl PhysicalBrightnessPort,
    direction: BrightnessStepDirection,
) -> Result<Vec<PhysicalBrightnessMemberResult>, SystemAdjustmentError> {
    let count = port.count();
    if !(1..=MAX_PHYSICAL_BRIGHTNESS_TARGETS).contains(&count) {
        return Err(failure(
            SystemAdjustmentErrorCode::UnsupportedBrightness,
            "物理显示器数量必须为 1–64，未提交亮度请求。",
        ));
    }
    let mut members = Vec::with_capacity(count);
    for index in 0..count {
        let label = port.label(index);
        let status = step_one(port, index, direction);
        members.push(PhysicalBrightnessMemberResult {
            index,
            label,
            status,
        });
    }
    Ok(members)
}

#[cfg(any(windows, test))]
fn step_one(
    port: &mut impl PhysicalBrightnessPort,
    index: usize,
    direction: BrightnessStepDirection,
) -> PhysicalBrightnessStatus {
    use PhysicalBrightnessStatus::*;
    let before = match port.read(index) {
        Ok(reading) => reading,
        Err(error) => return ReadRejected { error },
    };
    let requested = match raw_brightness_next(
        before.minimum,
        before.current,
        before.maximum,
        direction.delta(),
    ) {
        Ok((raw, _)) => raw,
        Err(error) => return ReadRejected { error },
    };
    if requested == before.current {
        return Unchanged { reading: before };
    }
    if let Err(error) = port.set(index, requested) {
        return SetUnconfirmed {
            before,
            requested,
            error,
        };
    }
    let observed = match port.read(index) {
        Ok(reading) => reading,
        Err(error) => {
            return ReadbackUnknown {
                before,
                requested,
                error,
            }
        }
    };
    if let Err(error) =
        super::percent_from_raw(observed.minimum, observed.current, observed.maximum)
    {
        return ReadbackUnknown {
            before,
            requested,
            error,
        };
    }
    if observed.minimum == before.minimum
        && observed.maximum == before.maximum
        && observed.current == requested
    {
        Confirmed {
            before,
            requested,
            observed,
        }
    } else {
        ReadbackDifferent {
            before,
            requested,
            observed,
        }
    }
}

#[cfg(any(windows, test))]
pub(super) fn step_members_if(
    port: &mut impl PhysicalBrightnessPort,
    direction: BrightnessStepDirection,
    check: &mut dyn FnMut(bool) -> Result<(), SystemAdjustmentError>,
) -> Result<Vec<PhysicalBrightnessMemberResult>, SystemAdjustmentError> {
    struct Guarded<'a, P> {
        port: &'a mut P,
        check: &'a mut dyn FnMut(bool) -> Result<(), SystemAdjustmentError>,
    }
    impl<P: PhysicalBrightnessPort> PhysicalBrightnessPort for Guarded<'_, P> {
        fn count(&self) -> usize {
            self.port.count()
        }
        fn label(&self, index: usize) -> String {
            self.port.label(index)
        }
        fn read(
            &mut self,
            index: usize,
        ) -> Result<PhysicalBrightnessReading, SystemAdjustmentError> {
            (self.check)(false)?;
            let result = self.port.read(index);
            (self.check)(false)?;
            result
        }
        fn set(&mut self, index: usize, value: u32) -> Result<(), SystemAdjustmentError> {
            (self.check)(true)?;
            let result = self.port.set(index, value);
            (self.check)(false)?;
            result
        }
    }
    check(false)?;
    let result = step_members(&mut Guarded { port, check }, direction);
    // 守卫已阻止后续 I/O；末尾取消不能抹掉此前成员的已确认/未知回执。
    let _ = check(false);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("physical_brightness_set_tests.rs");
}
