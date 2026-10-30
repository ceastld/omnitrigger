//! 音量与硬件背光操作，只能由动作工作线程调用，不参与输入 Hook。
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAdjustment {
    Volume { delta_percent: i16 },
    Brightness { delta_percent: i16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdjustmentKind {
    Volume,
    Brightness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrightnessMethod {
    DdcCi,
    Wmi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdjustmentResult {
    pub kind: AdjustmentKind,
    pub previous_percent: u8,
    pub requested_percent: u8,
    /// 原生接口读回的值，不将“成功提交”当成已达到请求值。
    pub current_percent: u8,
    pub target_label: String,
    /// 亮度动作实际选择的逻辑显示器设备名；音量不关联显示器。
    pub monitor_device: Option<String>,
    pub brightness_method: Option<BrightnessMethod>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdjustmentCapability {
    pub available: bool,
    pub current_percent: Option<u8>,
    pub target_label: String,
    pub brightness_method: Option<BrightnessMethod>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemAdjustmentCapabilities {
    pub volume: AdjustmentCapability,
    pub brightness: AdjustmentCapability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAdjustmentErrorCode {
    InvalidDelta,
    Initialization,
    AudioUnavailable,
    MonitorUnavailable,
    AmbiguousMonitor,
    UnsupportedBrightness,
    NativeFailure,
    ConfirmationUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemAdjustmentError {
    pub code: SystemAdjustmentErrorCode,
    pub message: String,
}

impl fmt::Display for SystemAdjustmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for SystemAdjustmentError {}

fn failure(code: SystemAdjustmentErrorCode, message: impl Into<String>) -> SystemAdjustmentError {
    SystemAdjustmentError {
        code,
        message: message.into(),
    }
}

pub fn validate_system_adjustment(action: SystemAdjustment) -> Result<(), SystemAdjustmentError> {
    let delta = match action {
        SystemAdjustment::Volume { delta_percent }
        | SystemAdjustment::Brightness { delta_percent } => delta_percent,
    };
    if delta == 0 || !(-100..=100).contains(&delta) {
        return Err(failure(
            SystemAdjustmentErrorCode::InvalidDelta,
            "调节步长应为 -100 至 100 个百分点，且不能为零。",
        ));
    }
    Ok(())
}

/// 捕获当前物理鼠标坐标，供 UI/CLI 在提交动作前冻结目标使用；不执行调节。
#[cfg(windows)]
pub fn capture_adjustment_point() -> Result<(i32, i32), SystemAdjustmentError> {
    use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetPhysicalCursorPos};
    let mut point = POINT::default();
    unsafe { GetPhysicalCursorPos(&mut point) }.map_err(|error| {
        failure(
            SystemAdjustmentErrorCode::MonitorUnavailable,
            format!("无法读取鼠标物理位置：{error}"),
        )
    })?;
    Ok((point.x, point.y))
}

fn volume_next(current: f32, delta: i16) -> Result<f32, SystemAdjustmentError> {
    if !current.is_finite() || !(0. ..=1.).contains(&current) {
        return Err(failure(
            SystemAdjustmentErrorCode::NativeFailure,
            "音量接口返回了无效范围。",
        ));
    }
    Ok((current + f32::from(delta) / 100.).clamp(0., 1.))
}

fn percent_from_raw(minimum: u32, current: u32, maximum: u32) -> Result<u8, SystemAdjustmentError> {
    if maximum <= minimum || !(minimum..=maximum).contains(&current) {
        return Err(failure(
            SystemAdjustmentErrorCode::NativeFailure,
            "亮度接口返回了无效范围。",
        ));
    }
    let value = f64::from(current - minimum) * 100. / f64::from(maximum - minimum);
    Ok(value.round() as u8)
}

fn raw_brightness_next(
    minimum: u32,
    current: u32,
    maximum: u32,
    delta: i16,
) -> Result<(u32, u8), SystemAdjustmentError> {
    percent_from_raw(minimum, current, maximum)?;
    let range = f64::from(maximum - minimum);
    let percent = (f64::from(current - minimum) * 100. / range + f64::from(delta)).clamp(0., 100.);
    let next = u64::from(minimum) + (range * percent / 100.).round() as u64;
    Ok((next.min(u64::from(maximum)) as u32, percent.round() as u8))
}

/// 使用调用方已经冻结的物理鼠标位置，不在工作线程重新读取鼠标。
/// Volume 操作默认多媒体输出端点；Brightness 只选择该位置的显示器。
#[cfg(windows)]
pub fn adjust_system_at(
    action: SystemAdjustment,
    point: (i32, i32),
) -> Result<AdjustmentResult, SystemAdjustmentError> {
    validate_system_adjustment(action)?;
    let _apartment = native::ComApartment::new()?;
    match action {
        SystemAdjustment::Volume { delta_percent } => native::adjust_volume(delta_percent),
        SystemAdjustment::Brightness { delta_percent } => {
            native::adjust_brightness(point, delta_percent)
        }
    }
}

/// 只查询能力和当前值，不改变音量、亮度或其他显示参数。需要在工作线程运行。
#[cfg(windows)]
pub fn query_system_adjustment_capabilities_at(point: (i32, i32)) -> SystemAdjustmentCapabilities {
    let result = native::ComApartment::new();
    let capability =
        |result: Result<AdjustmentResult, SystemAdjustmentError>, label: &str| match result {
            Ok(value) => AdjustmentCapability {
                available: true,
                current_percent: Some(value.current_percent),
                target_label: value.target_label,
                brightness_method: value.brightness_method,
                reason: None,
            },
            Err(error) => AdjustmentCapability {
                available: false,
                current_percent: None,
                target_label: label.into(),
                brightness_method: None,
                reason: Some(error.to_string()),
            },
        };
    match result {
        Ok(_apartment) => SystemAdjustmentCapabilities {
            volume: capability(native::read_volume(), "系统默认多媒体输出"),
            brightness: capability(native::read_brightness(point), "鼠标所在显示器"),
        },
        Err(error) => SystemAdjustmentCapabilities {
            volume: capability(Err(error.clone()), "系统默认多媒体输出"),
            brightness: capability(Err(error), "鼠标所在显示器"),
        },
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        mem::size_of,
        ptr,
        time::{Duration, Instant},
    };
    use windows::{
        core::{Interface, BSTR, PCWSTR},
        Win32::{
            Devices::Display::*,
            Foundation::{ERROR_INSUFFICIENT_BUFFER, POINT, RPC_E_CHANGED_MODE},
            Graphics::Gdi::{
                GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFOEXW, MONITOR_DEFAULTTONULL,
            },
            Media::Audio::{
                eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
                MMDeviceEnumerator,
            },
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CoUninitialize,
                    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE,
                    RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
                },
                Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE},
                Variant::VARIANT,
                Wmi::*,
            },
        },
    };

    fn native_error(
        code: SystemAdjustmentErrorCode,
        operation: &str,
        error: impl fmt::Display,
    ) -> SystemAdjustmentError {
        failure(code, format!("{operation}：{error}"))
    }

    pub(super) struct ComApartment(bool);
    impl ComApartment {
        pub(super) fn new() -> Result<Self, SystemAdjustmentError> {
            let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if status == RPC_E_CHANGED_MODE {
                return Ok(Self(false));
            }
            status.ok().map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::Initialization,
                    "无法初始化系统调节接口",
                    error,
                )
            })?;
            Ok(Self(true))
        }
    }
    impl Drop for ComApartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn audio_endpoint() -> Result<IAudioEndpointVolume, SystemAdjustmentError> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(
                |error| {
                    native_error(
                        SystemAdjustmentErrorCode::AudioUnavailable,
                        "无法访问音量设备",
                        error,
                    )
                },
            )?;
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }.map_err(
            |error| {
                native_error(
                    SystemAdjustmentErrorCode::AudioUnavailable,
                    "没有可用的默认多媒体输出设备",
                    error,
                )
            },
        )?;
        unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|error| {
            native_error(
                SystemAdjustmentErrorCode::AudioUnavailable,
                "输出设备不支持主音量接口",
                error,
            )
        })
    }

    fn audio_read(endpoint: &IAudioEndpointVolume) -> Result<f32, SystemAdjustmentError> {
        let value = unsafe { endpoint.GetMasterVolumeLevelScalar() }.map_err(|error| {
            native_error(
                SystemAdjustmentErrorCode::NativeFailure,
                "读取系统音量失败",
                error,
            )
        })?;
        volume_next(value, 0)?;
        Ok(value)
    }

    pub(super) fn read_volume() -> Result<AdjustmentResult, SystemAdjustmentError> {
        let value = (audio_read(&audio_endpoint()?)? * 100.).round() as u8;
        Ok(AdjustmentResult {
            kind: AdjustmentKind::Volume,
            previous_percent: value,
            current_percent: value,
            requested_percent: value,
            target_label: "系统默认多媒体输出".into(),
            monitor_device: None,
            brightness_method: None,
        })
    }

    pub(super) fn adjust_volume(delta: i16) -> Result<AdjustmentResult, SystemAdjustmentError> {
        let endpoint = audio_endpoint()?;
        let previous = audio_read(&endpoint)?;
        let requested = volume_next(previous, delta)?;
        unsafe { endpoint.SetMasterVolumeLevelScalar(requested, ptr::null()) }.map_err(
            |error| {
                native_error(
                    SystemAdjustmentErrorCode::NativeFailure,
                    "设置系统音量失败",
                    error,
                )
            },
        )?;
        let current = audio_read(&endpoint).map_err(|error| {
            native_error(
                SystemAdjustmentErrorCode::ConfirmationUnknown,
                "音量请求已提交，但无法读回结果",
                error,
            )
        })?;
        Ok(AdjustmentResult {
            kind: AdjustmentKind::Volume,
            previous_percent: (previous * 100.).round() as u8,
            requested_percent: (requested * 100.).round() as u8,
            current_percent: (current * 100.).round() as u8,
            target_label: "系统默认多媒体输出".into(),
            monitor_device: None,
            brightness_method: None,
        })
    }

    struct LogicalMonitor {
        handle: HMONITOR,
        name: String,
    }
    fn utf16_text(value: &[u16]) -> String {
        String::from_utf16_lossy(
            &value[..value.iter().position(|&c| c == 0).unwrap_or(value.len())],
        )
    }
    fn logical_monitor(point: (i32, i32)) -> Result<LogicalMonitor, SystemAdjustmentError> {
        let handle = unsafe {
            MonitorFromPoint(
                POINT {
                    x: point.0,
                    y: point.1,
                },
                MONITOR_DEFAULTTONULL,
            )
        };
        if handle.is_invalid() {
            return Err(failure(
                SystemAdjustmentErrorCode::MonitorUnavailable,
                "触发位置已不在任何显示器内。",
            ));
        }
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
            return Err(native_error(
                SystemAdjustmentErrorCode::MonitorUnavailable,
                "无法读取目标显示器",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(LogicalMonitor {
            handle,
            name: utf16_text(&info.szDevice),
        })
    }

    struct PhysicalMonitors(Vec<PHYSICAL_MONITOR>);
    impl Drop for PhysicalMonitors {
        fn drop(&mut self) {
            for monitor in &self.0 {
                let handle = monitor.hPhysicalMonitor;
                if !handle.is_invalid() {
                    let _ = unsafe { DestroyPhysicalMonitor(handle) };
                }
            }
        }
    }

    struct DdcMonitor {
        monitors: PhysicalMonitors,
        minimum: u32,
        current: u32,
        maximum: u32,
        label: String,
        monitor_device: String,
    }
    fn ddc_monitor(logical: &LogicalMonitor) -> Result<DdcMonitor, SystemAdjustmentError> {
        let mut count = 0;
        unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(logical.handle, &mut count) }.map_err(
            |error| {
                native_error(
                    SystemAdjustmentErrorCode::UnsupportedBrightness,
                    "目标显示器没有可用的硬件亮度接口",
                    error,
                )
            },
        )?;
        if count != 1 {
            return Err(failure(
                if count > 1 {
                    SystemAdjustmentErrorCode::AmbiguousMonitor
                } else {
                    SystemAdjustmentErrorCode::UnsupportedBrightness
                },
                format!("目标显示器关联 {count} 个物理面板，不能唯一选择背光目标。"),
            ));
        }
        let mut monitors = PhysicalMonitors(vec![PHYSICAL_MONITOR::default(); count as usize]);
        unsafe { GetPhysicalMonitorsFromHMONITOR(logical.handle, &mut monitors.0) }.map_err(
            |error| {
                native_error(
                    SystemAdjustmentErrorCode::UnsupportedBrightness,
                    "无法打开硬件亮度接口",
                    error,
                )
            },
        )?;
        let handle = monitors.0[0].hPhysicalMonitor;
        // 有些显示器的 MCCS 能力声明读取失败，但亮度接口本身可用；以实际读取和范围校验为准。
        let (mut minimum, mut current, mut maximum) = (0, 0, 0);
        if unsafe { GetMonitorBrightness(handle, &mut minimum, &mut current, &mut maximum) } == 0 {
            return Err(native_error(
                SystemAdjustmentErrorCode::UnsupportedBrightness,
                "读取硬件亮度失败",
                std::io::Error::last_os_error(),
            ));
        }
        percent_from_raw(minimum, current, maximum)?;
        let description_buffer = monitors.0[0].szPhysicalMonitorDescription;
        let description = utf16_text(&description_buffer);
        let label = if description.is_empty() {
            logical.name.clone()
        } else {
            description
        };
        Ok(DdcMonitor {
            monitors,
            minimum,
            current,
            maximum,
            label,
            monitor_device: logical.name.clone(),
        })
    }

    impl DdcMonitor {
        fn result(
            &self,
            requested: u8,
            previous: u8,
        ) -> Result<AdjustmentResult, SystemAdjustmentError> {
            Ok(AdjustmentResult {
                kind: AdjustmentKind::Brightness,
                previous_percent: previous,
                requested_percent: requested,
                current_percent: percent_from_raw(self.minimum, self.current, self.maximum)?,
                target_label: self.label.clone(),
                monitor_device: Some(self.monitor_device.clone()),
                brightness_method: Some(BrightnessMethod::DdcCi),
            })
        }
        fn adjust(mut self, delta: i16) -> Result<AdjustmentResult, SystemAdjustmentError> {
            let previous = percent_from_raw(self.minimum, self.current, self.maximum)?;
            let (next, requested) =
                raw_brightness_next(self.minimum, self.current, self.maximum, delta)?;
            let submitted = next != self.current;
            if submitted {
                let handle = self.monitors.0[0].hPhysicalMonitor;
                if unsafe { SetMonitorBrightness(handle, next) } == 0 {
                    return Err(native_error(
                        SystemAdjustmentErrorCode::ConfirmationUnknown,
                        "硬件亮度请求未获确认，当前结果未知",
                        std::io::Error::last_os_error(),
                    ));
                }
                if unsafe {
                    GetMonitorBrightness(
                        handle,
                        &mut self.minimum,
                        &mut self.current,
                        &mut self.maximum,
                    )
                } == 0
                {
                    return Err(native_error(
                        SystemAdjustmentErrorCode::ConfirmationUnknown,
                        "亮度请求已提交，但无法读回结果",
                        std::io::Error::last_os_error(),
                    ));
                }
            }
            let result = self.result(requested, previous);
            if submitted {
                confirm_brightness_result(result)
            } else {
                result
            }
        }
    }

    // 以下 WMI 路径只在同一目标显示器不支持 DDC/CI 时使用，必须先匹配精确设备实例。
    fn target_device_path(logical: &LogicalMonitor) -> Result<String, SystemAdjustmentError> {
        for _ in 0..3 {
            let (mut path_count, mut mode_count) = (0, 0);
            unsafe {
                GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            }
            .ok()
            .map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::MonitorUnavailable,
                    "无法读取显示拓扑",
                    error,
                )
            })?;
            if path_count > 128 || mode_count > 1024 {
                return Err(failure(
                    SystemAdjustmentErrorCode::MonitorUnavailable,
                    "显示拓扑超出可处理范围。",
                ));
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
            let status = unsafe {
                QueryDisplayConfig(
                    QDC_ONLY_ACTIVE_PATHS,
                    &mut path_count,
                    paths.as_mut_ptr(),
                    &mut mode_count,
                    modes.as_mut_ptr(),
                    None,
                )
            };
            if status == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            status.ok().map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::MonitorUnavailable,
                    "无法读取活动显示路径",
                    error,
                )
            })?;
            let mut matches = Vec::new();
            for path in paths.iter().take(path_count as usize) {
                let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                    header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                        size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                        adapterId: path.sourceInfo.adapterId,
                        id: path.sourceInfo.id,
                    },
                    ..Default::default()
                };
                if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0
                    || !utf16_text(&source.viewGdiDeviceName).eq_ignore_ascii_case(&logical.name)
                {
                    continue;
                }
                let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                    header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                        size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                        adapterId: path.targetInfo.adapterId,
                        id: path.targetInfo.id,
                    },
                    ..Default::default()
                };
                if unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } == 0 {
                    matches.push(utf16_text(&target.monitorDevicePath));
                }
            }
            matches.sort();
            matches.dedup();
            if matches.len() != 1 {
                return Err(failure(
                    SystemAdjustmentErrorCode::AmbiguousMonitor,
                    "显示器路径不能唯一关联到一个面板，不使用其他屏幕的亮度接口。",
                ));
            }
            return Ok(matches.remove(0));
        }
        Err(failure(
            SystemAdjustmentErrorCode::MonitorUnavailable,
            "显示器拓扑正在变化，请重试。",
        ))
    }

    fn wmi_connect() -> Result<IWbemServices, SystemAdjustmentError> {
        let locator: IWbemLocator =
            unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }.map_err(
                |error| {
                    native_error(
                        SystemAdjustmentErrorCode::UnsupportedBrightness,
                        "无法打开内屏亮度服务",
                        error,
                    )
                },
            )?;
        let empty = BSTR::default();
        let services = unsafe {
            locator.ConnectServer(
                &BSTR::from("ROOT\\WMI"),
                &empty,
                &empty,
                &empty,
                WBEM_FLAG_CONNECT_USE_MAX_WAIT.0,
                &empty,
                None,
            )
        }
        .map_err(|error| {
            native_error(
                SystemAdjustmentErrorCode::UnsupportedBrightness,
                "内屏亮度服务不可用",
                error,
            )
        })?;
        unsafe {
            CoSetProxyBlanket(
                &services,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )
        }
        .map_err(|error| {
            native_error(
                SystemAdjustmentErrorCode::UnsupportedBrightness,
                "无法访问内屏亮度服务",
                error,
            )
        })?;
        Ok(services)
    }

    fn wmi_rows(
        services: &IWbemServices,
        query: &str,
    ) -> Result<Vec<IWbemClassObject>, SystemAdjustmentError> {
        let enumerator = unsafe {
            services.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(query),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
        }
        .map_err(|error| {
            native_error(
                SystemAdjustmentErrorCode::UnsupportedBrightness,
                "读取内屏亮度能力失败",
                error,
            )
        })?;
        let mut result = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let mut values = [None];
            let mut returned = 0;
            let status = unsafe { enumerator.Next(500, &mut values, &mut returned) };
            status.ok().map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::UnsupportedBrightness,
                    "读取内屏亮度实例失败",
                    error,
                )
            })?;
            if returned == 0 {
                if status.0 == WBEM_S_TIMEDOUT.0 {
                    return Err(failure(
                        SystemAdjustmentErrorCode::UnsupportedBrightness,
                        "读取内屏亮度能力超时。",
                    ));
                }
                return Ok(result);
            }
            result.push(values[0].take().ok_or_else(|| {
                failure(
                    SystemAdjustmentErrorCode::NativeFailure,
                    "亮度服务返回了空实例。",
                )
            })?);
            if result.len() > 32 || Instant::now() >= deadline {
                return Err(failure(
                    SystemAdjustmentErrorCode::UnsupportedBrightness,
                    "内屏亮度实例过多或查询超时。",
                ));
            }
        }
    }

    fn property(object: &IWbemClassObject, key: PCWSTR) -> Result<VARIANT, SystemAdjustmentError> {
        let mut value = VARIANT::default();
        unsafe { object.Get(key, 0, &mut value, None, None) }.map_err(|error| {
            native_error(
                SystemAdjustmentErrorCode::NativeFailure,
                "读取亮度实例属性失败",
                error,
            )
        })?;
        Ok(value)
    }
    fn string_property(
        object: &IWbemClassObject,
        key: PCWSTR,
    ) -> Result<String, SystemAdjustmentError> {
        BSTR::try_from(&property(object, key)?)
            .map(|value| value.to_string())
            .map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::NativeFailure,
                    "亮度实例属性类型不正确",
                    error,
                )
            })
    }
    fn integer_property(
        object: &IWbemClassObject,
        key: PCWSTR,
    ) -> Result<u32, SystemAdjustmentError> {
        u32::try_from(&property(object, key)?).map_err(|error| {
            native_error(
                SystemAdjustmentErrorCode::NativeFailure,
                "亮度数值类型不正确",
                error,
            )
        })
    }

    struct WmiMonitor {
        services: IWbemServices,
        instance: String,
        method_path: String,
        current: u8,
        label: String,
    }
    fn wmi_monitor(logical: &LogicalMonitor) -> Result<WmiMonitor, SystemAdjustmentError> {
        let device =
            monitor_instance_from_path(&target_device_path(logical)?).ok_or_else(|| {
                failure(
                    SystemAdjustmentErrorCode::UnsupportedBrightness,
                    "无法识别目标显示器的设备实例，不选择其他屏幕。",
                )
            })?;
        let services = wmi_connect()?;
        let mut matched = Vec::new();
        for object in wmi_rows(
            &services,
            "SELECT InstanceName, CurrentBrightness FROM WmiMonitorBrightness WHERE Active = TRUE",
        )? {
            let instance = string_property(&object, windows::core::w!("InstanceName"))?;
            if normalize_wmi_instance(&instance).eq_ignore_ascii_case(&device) {
                let current = integer_property(&object, windows::core::w!("CurrentBrightness"))?;
                if current > 100 {
                    return Err(failure(
                        SystemAdjustmentErrorCode::NativeFailure,
                        "内屏返回了无效亮度。",
                    ));
                }
                matched.push((instance, current as u8));
            }
        }
        if matched.len() != 1 {
            return Err(failure(
                SystemAdjustmentErrorCode::UnsupportedBrightness,
                "目标显示器没有可唯一关联的 WMI 背光接口；该屏幕可能不支持系统亮度控制。",
            ));
        }
        let (instance, current) = matched.remove(0);
        let method_path = wmi_method_path(&services, &instance)?;
        Ok(WmiMonitor {
            services,
            instance,
            method_path,
            current,
            label: logical.name.clone(),
        })
    }

    fn wmi_method_path(
        services: &IWbemServices,
        instance: &str,
    ) -> Result<String, SystemAdjustmentError> {
        let objects = wmi_rows(
            services,
            "SELECT InstanceName, __PATH FROM WmiMonitorBrightnessMethods WHERE Active = TRUE",
        )?;
        let mut methods = Vec::new();
        for object in objects {
            if string_property(&object, windows::core::w!("InstanceName"))?
                .eq_ignore_ascii_case(instance)
            {
                methods.push(string_property(&object, windows::core::w!("__PATH"))?);
            }
        }
        if methods.len() != 1 {
            return Err(failure(
                SystemAdjustmentErrorCode::UnsupportedBrightness,
                "目标面板没有唯一的亮度设置方法。",
            ));
        }
        Ok(methods.remove(0))
    }

    fn wait_wmi_call_result(
        call: &IWbemCallResult,
    ) -> Result<Option<IWbemClassObject>, SystemAdjustmentError> {
        // 保留原始 HRESULT：正值 WBEM_S_TIMEDOUT 也表示尚未完成，不能按一般 SUCCEEDED 处理。
        let mut method_status = 0;
        let status =
            unsafe { (call.vtable().GetCallStatus)(call.as_raw(), 2000, &mut method_status) };
        if status.0 != 0 {
            return Err(failure(
                SystemAdjustmentErrorCode::ConfirmationUnknown,
                format!(
                    "亮度请求已提交，但无法及时确认调用完成（HRESULT 0x{:08X}）。",
                    status.0 as u32
                ),
            ));
        }
        if method_status != 0 {
            return Err(failure(
                SystemAdjustmentErrorCode::NativeFailure,
                format!(
                    "内屏亮度方法执行失败（HRESULT 0x{:08X}）。",
                    method_status as u32
                ),
            ));
        }
        let mut raw = ptr::null_mut();
        // 完成等待已限定为两秒；此处不再等待，也允许成功调用没有输出对象。
        let status = unsafe { (call.vtable().GetResultObject)(call.as_raw(), 0, &mut raw) };
        let response = if raw.is_null() {
            None
        } else {
            Some(unsafe { IWbemClassObject::from_raw(raw) })
        };
        if status.0 != 0 {
            return Err(failure(
                SystemAdjustmentErrorCode::ConfirmationUnknown,
                format!(
                    "亮度请求已提交，但无法取得方法输出（HRESULT 0x{:08X}）。",
                    status.0 as u32
                ),
            ));
        }
        Ok(response)
    }

    fn read_wmi_confirmation(
        requested: u8,
        has_response: bool,
        mut read_current: impl FnMut() -> Result<u8, SystemAdjustmentError>,
        mut wait_before_retry: impl FnMut(),
    ) -> Result<u8, SystemAdjustmentError> {
        let mut current = 0;
        for attempt in 0..3 {
            current = confirm_brightness_result(read_current())?;
            if current == requested || (has_response && attempt == 2) {
                return Ok(current);
            }
            if attempt < 2 {
                wait_before_retry();
            }
        }
        Err(failure(
            SystemAdjustmentErrorCode::ConfirmationUnknown,
            format!(
                "亮度请求已提交且调用完成，但没有方法输出，读回 {current}% 尚未达到请求的 {requested}%。"
            ),
        ))
    }

    impl WmiMonitor {
        fn read_current(&self) -> Result<u8, SystemAdjustmentError> {
            let mut matches = Vec::new();
            for object in wmi_rows(
                &self.services,
                "SELECT InstanceName, CurrentBrightness FROM WmiMonitorBrightness WHERE Active = TRUE",
            )? {
                if string_property(&object, windows::core::w!("InstanceName"))?
                    .eq_ignore_ascii_case(&self.instance)
                {
                    let value = integer_property(&object, windows::core::w!("CurrentBrightness"))?;
                    if value > 100 {
                        return Err(failure(
                            SystemAdjustmentErrorCode::NativeFailure,
                            "内屏返回了无效亮度。",
                        ));
                    }
                    matches.push(value as u8);
                }
            }
            if matches.len() != 1 {
                return Err(failure(
                    SystemAdjustmentErrorCode::MonitorUnavailable,
                    "目标亮度实例已变化。",
                ));
            }
            Ok(matches[0])
        }
        fn result(&self, previous: u8, requested: u8) -> AdjustmentResult {
            AdjustmentResult {
                kind: AdjustmentKind::Brightness,
                previous_percent: previous,
                requested_percent: requested,
                current_percent: self.current,
                target_label: self.label.clone(),
                monitor_device: Some(self.label.clone()),
                brightness_method: Some(BrightnessMethod::Wmi),
            }
        }
        fn adjust(mut self, delta: i16) -> Result<AdjustmentResult, SystemAdjustmentError> {
            let previous = self.current;
            let requested = (i16::from(previous) + delta).clamp(0, 100) as u8;
            if requested == previous {
                return Ok(self.result(previous, requested));
            }
            let mut class = None;
            unsafe {
                self.services.GetObject(
                    &BSTR::from("WmiMonitorBrightnessMethods"),
                    WBEM_GENERIC_FLAG_TYPE(0),
                    None,
                    Some(&mut class),
                    None,
                )
            }
            .map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::UnsupportedBrightness,
                    "无法取得亮度设置方法",
                    error,
                )
            })?;
            let class = class.ok_or_else(|| {
                failure(SystemAdjustmentErrorCode::NativeFailure, "亮度设置类为空。")
            })?;
            let (mut input, mut output) = (None, None);
            unsafe {
                class.GetMethod(
                    windows::core::w!("WmiSetBrightness"),
                    0,
                    &mut input,
                    &mut output,
                )
            }
            .map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::UnsupportedBrightness,
                    "亮度设置方法不可用",
                    error,
                )
            })?;
            let input = unsafe {
                input
                    .ok_or_else(|| {
                        failure(
                            SystemAdjustmentErrorCode::NativeFailure,
                            "亮度设置参数为空。",
                        )
                    })?
                    .SpawnInstance(0)
            }
            .map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::NativeFailure,
                    "无法准备亮度参数",
                    error,
                )
            })?;
            unsafe {
                // WMI 的 uint32 通过 Automation 使用 VT_I4，而不是 VT_UI4。
                input
                    .Put(windows::core::w!("Timeout"), 0, &VARIANT::from(0i32), 0)
                    .map_err(|error| {
                        native_error(
                            SystemAdjustmentErrorCode::NativeFailure,
                            "无法写入亮度超时参数",
                            error,
                        )
                    })?;
                input.Put(
                    windows::core::w!("Brightness"),
                    0,
                    &VARIANT::from(requested),
                    0,
                )
            }
            .map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::NativeFailure,
                    "无法写入亮度参数",
                    error,
                )
            })?;
            let mut call = None;
            unsafe {
                self.services.ExecMethod(
                    &BSTR::from(self.method_path.as_str()),
                    &BSTR::from("WmiSetBrightness"),
                    WBEM_FLAG_RETURN_IMMEDIATELY,
                    None,
                    &input,
                    None,
                    Some(&mut call),
                )
            }
            .map_err(|error| {
                native_error(
                    SystemAdjustmentErrorCode::ConfirmationUnknown,
                    "亮度请求未获确认，当前结果未知",
                    error,
                )
            })?;
            let call = call.ok_or_else(|| {
                failure(
                    SystemAdjustmentErrorCode::ConfirmationUnknown,
                    "亮度请求已提交，但没有返回确认句柄。",
                )
            })?;
            let response = wait_wmi_call_result(&call)?;
            if let Some(response) = &response {
                let status = confirm_brightness_result(integer_property(
                    response,
                    windows::core::w!("ReturnValue"),
                ))?;
                if status != 0 {
                    return Err(failure(
                        SystemAdjustmentErrorCode::NativeFailure,
                        format!("内屏拒绝亮度请求，错误码 {status}。"),
                    ));
                }
            }
            self.current = read_wmi_confirmation(
                requested,
                response.is_some(),
                || self.read_current(),
                || std::thread::sleep(Duration::from_millis(25)),
            )?;
            Ok(self.result(previous, requested))
        }
    }

    fn confirm_brightness_result<T>(
        result: Result<T, SystemAdjustmentError>,
    ) -> Result<T, SystemAdjustmentError> {
        result.map_err(|error| {
            failure(
                SystemAdjustmentErrorCode::ConfirmationUnknown,
                format!("亮度请求已提交，但无法确认结果：{}", error.message),
            )
        })
    }

    enum BrightnessTarget {
        Ddc(DdcMonitor),
        Wmi(WmiMonitor),
    }
    fn brightness_target(point: (i32, i32)) -> Result<BrightnessTarget, SystemAdjustmentError> {
        let logical = logical_monitor(point)?;
        match ddc_monitor(&logical) {
            Ok(target) => Ok(BrightnessTarget::Ddc(target)),
            Err(error) if error.code == SystemAdjustmentErrorCode::AmbiguousMonitor => Err(error),
            Err(ddc_error) => {
                wmi_monitor(&logical)
                    .map(BrightnessTarget::Wmi)
                    .map_err(|wmi_error| {
                        failure(
                            SystemAdjustmentErrorCode::UnsupportedBrightness,
                            format!("{}；{}", ddc_error.message, wmi_error.message),
                        )
                    })
            }
        }
    }

    pub(super) fn read_brightness(
        point: (i32, i32),
    ) -> Result<AdjustmentResult, SystemAdjustmentError> {
        match brightness_target(point)? {
            BrightnessTarget::Ddc(target) => {
                let value = percent_from_raw(target.minimum, target.current, target.maximum)?;
                target.result(value, value)
            }
            BrightnessTarget::Wmi(target) => Ok(target.result(target.current, target.current)),
        }
    }
    pub(super) fn adjust_brightness(
        point: (i32, i32),
        delta: i16,
    ) -> Result<AdjustmentResult, SystemAdjustmentError> {
        match brightness_target(point)? {
            BrightnessTarget::Ddc(target) => target.adjust(delta),
            BrightnessTarget::Wmi(target) => target.adjust(delta),
        }
    }
    #[cfg(test)]
    mod wmi_confirmation_tests {
        use super::*;
        use std::{cell::Cell, ffi::c_void};
        use windows::core::{IUnknown_Vtbl, GUID, HRESULT};

        #[repr(C)]
        struct FakeCall {
            vtable: *const IWbemCallResult_Vtbl,
            call_status: HRESULT,
            method_status: i32,
            object_status: HRESULT,
            call_timeout: Cell<i32>,
            object_timeout: Cell<i32>,
            object_calls: Cell<u32>,
        }

        unsafe extern "system" fn query(
            _: *mut c_void,
            _: *const GUID,
            output: *mut *mut c_void,
        ) -> HRESULT {
            unsafe { *output = ptr::null_mut() };
            HRESULT(0x80004002u32 as i32)
        }
        unsafe extern "system" fn reference(_: *mut c_void) -> u32 {
            1
        }
        unsafe extern "system" fn get_status(
            this: *mut c_void,
            timeout: i32,
            status: *mut i32,
        ) -> HRESULT {
            let fake = unsafe { &*(this as *const FakeCall) };
            fake.call_timeout.set(timeout);
            unsafe { *status = fake.method_status };
            fake.call_status
        }
        unsafe extern "system" fn get_object(
            this: *mut c_void,
            timeout: i32,
            output: *mut *mut c_void,
        ) -> HRESULT {
            let fake = unsafe { &*(this as *const FakeCall) };
            fake.object_timeout.set(timeout);
            fake.object_calls.set(fake.object_calls.get() + 1);
            unsafe { *output = ptr::null_mut() };
            fake.object_status
        }
        static VTABLE: IWbemCallResult_Vtbl = IWbemCallResult_Vtbl {
            base__: IUnknown_Vtbl {
                QueryInterface: query,
                AddRef: reference,
                Release: reference,
            },
            GetResultObject: get_object,
            GetResultString: get_object,
            GetResultServices: get_object,
            GetCallStatus: get_status,
        };
        impl FakeCall {
            fn new(call_status: i32, method_status: i32, object_status: i32) -> Self {
                Self {
                    vtable: &VTABLE,
                    call_status: HRESULT(call_status),
                    method_status,
                    object_status: HRESULT(object_status),
                    call_timeout: Cell::new(-1),
                    object_timeout: Cell::new(-1),
                    object_calls: Cell::new(0),
                }
            }
            fn run(&self) -> Result<Option<IWbemClassObject>, SystemAdjustmentError> {
                let raw = self as *const Self as *mut c_void;
                let call = unsafe { IWbemCallResult::from_raw_borrowed(&raw) }.unwrap();
                wait_wmi_call_result(call)
            }
        }

        #[test]
        fn completed_wmi_call_accepts_null_output_without_losing_the_hresult_or_wait_budget() {
            let call = FakeCall::new(0, 0, 0);
            assert!(call.run().unwrap().is_none());
            assert_eq!(call.call_timeout.get(), 2000);
            assert_eq!(call.object_timeout.get(), 0);
            assert_eq!(call.object_calls.get(), 1);
        }

        #[test]
        fn wmi_positive_timeout_and_real_method_failures_never_count_as_completion() {
            for call_status in [WBEM_S_TIMEDOUT.0, 1, 0x80070005u32 as i32] {
                let call = FakeCall::new(call_status, 0, 0);
                assert_eq!(
                    call.run().unwrap_err().code,
                    SystemAdjustmentErrorCode::ConfirmationUnknown
                );
                assert_eq!(call.object_calls.get(), 0);
            }
            let failed_method = FakeCall::new(0, 0x80041001u32 as i32, 0);
            assert_eq!(
                failed_method.run().unwrap_err().code,
                SystemAdjustmentErrorCode::NativeFailure
            );
            assert_eq!(failed_method.object_calls.get(), 0);
            for object_status in [WBEM_S_TIMEDOUT.0, 0x80041001u32 as i32] {
                let call = FakeCall::new(0, 0, object_status);
                assert_eq!(
                    call.run().unwrap_err().code,
                    SystemAdjustmentErrorCode::ConfirmationUnknown
                );
            }
        }

        #[test]
        fn missing_method_output_requires_matching_brightness_readback() {
            let mut values = [41, 45, 46].into_iter();
            let waits = Cell::new(0);
            let current = read_wmi_confirmation(
                46,
                false,
                || Ok(values.next().unwrap()),
                || waits.set(waits.get() + 1),
            )
            .unwrap();
            assert_eq!(current, 46);
            assert_eq!(waits.get(), 2);
            assert_eq!(
                read_wmi_confirmation(46, false, || Ok(41), || {})
                    .unwrap_err()
                    .code,
                SystemAdjustmentErrorCode::ConfirmationUnknown
            );
            assert_eq!(
                read_wmi_confirmation(46, true, || Ok(41), || {}).unwrap(),
                41
            );
            let error = read_wmi_confirmation(
                46,
                false,
                || {
                    Err(failure(
                        SystemAdjustmentErrorCode::MonitorUnavailable,
                        "目标实例已变化",
                    ))
                },
                || {},
            )
            .unwrap_err();
            assert_eq!(error.code, SystemAdjustmentErrorCode::ConfirmationUnknown);
            assert!(error.message.contains("目标实例已变化"));
        }
    }
}

#[cfg(any(windows, test))]
fn monitor_instance_from_path(path: &str) -> Option<String> {
    let path = path.strip_prefix("\\\\?\\")?;
    let parts = path.split('#').collect::<Vec<_>>();
    if parts.len() != 4
        || !parts[0].eq_ignore_ascii_case("DISPLAY")
        || parts[1].is_empty()
        || parts[2].is_empty()
        || !parts[3].starts_with('{')
    {
        return None;
    }
    Some(format!("DISPLAY\\{}\\{}", parts[1], parts[2]))
}

#[cfg(any(windows, test))]
fn normalize_wmi_instance(instance: &str) -> &str {
    match instance.rsplit_once('_') {
        Some((base, suffix))
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) =>
        {
            base
        }
        _ => instance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    #[ignore = "只读硬件能力诊断；需要本机音频/显示服务，不改变音量或亮度"]
    fn read_only_capabilities_diagnostic() {
        let point = capture_adjustment_point().unwrap();
        let capabilities = query_system_adjustment_capabilities_at(point);
        println!("系统调节能力（只读）：{capabilities:?}");
        for capability in [&capabilities.volume, &capabilities.brightness] {
            assert_eq!(capability.available, capability.current_percent.is_some());
            assert_eq!(capability.available, capability.reason.is_none());
        }
    }

    #[cfg(windows)]
    #[test]
    fn wmi_numeric_wire_types_preserve_the_provider_uint8_and_uint32_contract() {
        use windows::Win32::System::Variant::{VARIANT, VT_I4, VT_UI1};
        let timeout = VARIANT::from(0i32);
        let brightness = VARIANT::from(73u8);
        assert_eq!(timeout.vt(), VT_I4);
        assert_eq!(brightness.vt(), VT_UI1);
        assert_eq!(u32::try_from(&brightness).unwrap(), 73);
        assert_eq!(u32::try_from(&timeout).unwrap(), 0);
    }

    #[test]
    fn adjustment_steps_are_nonzero_bounded_and_percentages_clamp_without_changing_other_state() {
        for delta in [-101, 0, 101, i16::MIN, i16::MAX] {
            assert!(validate_system_adjustment(SystemAdjustment::Volume {
                delta_percent: delta
            })
            .is_err());
        }
        for delta in [-100, -5, 1, 5, 100] {
            validate_system_adjustment(SystemAdjustment::Brightness {
                delta_percent: delta,
            })
            .unwrap();
        }
        assert_eq!(volume_next(0.98, 5).unwrap(), 1.);
        assert_eq!(volume_next(0.01, -5).unwrap(), 0.);
        assert!((volume_next(0.433, 5).unwrap() - 0.483).abs() < 0.0001);
        for value in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            assert!(volume_next(value, 5).is_err());
        }
    }

    #[test]
    fn hardware_ranges_keep_nonzero_minimum_quantization_and_wide_integer_boundaries() {
        assert_eq!(raw_brightness_next(10, 35, 110, 5).unwrap(), (40, 30));
        assert_eq!(raw_brightness_next(10, 108, 110, 5).unwrap(), (110, 100));
        assert_eq!(raw_brightness_next(10, 12, 110, -5).unwrap(), (10, 0));
        assert_eq!(percent_from_raw(10, 60, 110).unwrap(), 50);
        assert_eq!(
            raw_brightness_next(0, u32::MAX, u32::MAX, -100).unwrap(),
            (0, 0)
        );
        assert!(raw_brightness_next(5, 5, 5, 5).is_err());
        assert!(raw_brightness_next(10, 9, 110, 5).is_err());
    }

    #[test]
    fn panel_identity_matching_never_uses_a_different_display_or_a_partial_name() {
        let path = "\\\\?\\DISPLAY#ABC123#4&abcd&0&UID256#{12345678-1234-1234-1234-123456789abc}";
        assert_eq!(
            monitor_instance_from_path(path).unwrap(),
            "DISPLAY\\ABC123\\4&abcd&0&UID256"
        );
        assert_eq!(
            normalize_wmi_instance("DISPLAY\\ABC123\\4&abcd&0&UID256_0"),
            "DISPLAY\\ABC123\\4&abcd&0&UID256"
        );
        assert_ne!(
            normalize_wmi_instance("DISPLAY\\ABC123\\other_0"),
            monitor_instance_from_path(path).unwrap()
        );
        assert_eq!(
            normalize_wmi_instance("DISPLAY\\ABC123\\instance_custom"),
            "DISPLAY\\ABC123\\instance_custom"
        );
        for path in [
            "DISPLAY#ABC123#instance#{guid}",
            "\\\\?\\MONITOR#ABC#instance#{guid}",
            "\\\\?\\DISPLAY##instance#{guid}",
        ] {
            assert!(monitor_instance_from_path(path).is_none());
        }
    }
}
