//! WMI 支持档位选择；不按固定百分点猜测设备档位。
use super::{failure, SystemAdjustmentError, SystemAdjustmentErrorCode};

pub const MAX_WMI_BRIGHTNESS_LEVELS: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmiBrightnessStepDirection {
    Increase,
    Decrease,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmiBrightnessStepSelection {
    Set { requested: u8 },
    SkipZero,
    NoEligibleLevel,
}

pub fn plan_wmi_brightness_step(
    current: u8,
    levels: &[u8],
    direction: WmiBrightnessStepDirection,
) -> Result<WmiBrightnessStepSelection, SystemAdjustmentError> {
    if current > 100
        || levels.is_empty()
        || levels.len() > MAX_WMI_BRIGHTNESS_LEVELS
        || levels.iter().any(|value| *value > 100)
    {
        return Err(failure(
            SystemAdjustmentErrorCode::NativeFailure,
            "WMI 当前亮度或支持档位无效，未提交亮度请求。",
        ));
    }
    let threshold = match direction {
        WmiBrightnessStepDirection::Increase => i16::from(current) + 1,
        WmiBrightnessStepDirection::Decrease => i16::from(current) - 1,
    };
    let eligible = levels.iter().copied().filter(|level| match direction {
        WmiBrightnessStepDirection::Increase => i16::from(*level) > threshold,
        WmiBrightnessStepDirection::Decrease => i16::from(*level) < threshold,
    });
    let selected = match direction {
        WmiBrightnessStepDirection::Increase => eligible.min(),
        WmiBrightnessStepDirection::Decrease => eligible.max(),
    };
    Ok(match selected {
        Some(0) => WmiBrightnessStepSelection::SkipZero,
        Some(requested) => WmiBrightnessStepSelection::Set { requested },
        None => WmiBrightnessStepSelection::NoEligibleLevel,
    })
}

/// SAFEARRAY 的上下界允许非零/负下界；只接受一维、至多256项的有界档位数组。
#[cfg(any(windows, test))]
pub(super) fn wmi_level_array_count(
    dimensions: u32,
    lower: i32,
    upper: i32,
) -> Result<usize, SystemAdjustmentError> {
    let count = i64::from(upper) - i64::from(lower) + 1;
    if dimensions != 1 || !(1..=MAX_WMI_BRIGHTNESS_LEVELS as i64).contains(&count) {
        return Err(failure(
            SystemAdjustmentErrorCode::NativeFailure,
            "WMI Level 必须为一维且包含 1–256 项。",
        ));
    }
    Ok(count as usize)
}

#[cfg(any(windows, test))]
pub(super) fn submit_selected_wmi_step<T>(
    selection: WmiBrightnessStepSelection,
    submit: impl FnOnce(u8) -> Result<T, SystemAdjustmentError>,
) -> Result<Option<T>, SystemAdjustmentError> {
    match selection {
        WmiBrightnessStepSelection::Set { requested } => submit(requested).map(Some),
        WmiBrightnessStepSelection::SkipZero | WmiBrightnessStepSelection::NoEligibleLevel => {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("wmi_brightness_levels_tests.rs");
}
