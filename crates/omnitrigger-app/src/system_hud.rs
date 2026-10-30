//! 亮度提示仅消费本软件请求与回执；显示由独立 Win32 线程负责，不依赖 GPUI 帧。
use omnitrigger_windows::{AdjustmentKind, AdjustmentResult, NativeSystemHud, SystemHudPlacement};
use std::time::{Duration, Instant};

#[path = "system_hud_probe.rs"]
mod probe;
#[path = "system_hud_values.rs"]
mod values;

const HOLD: Duration = Duration::from_millis(1400);
const PENDING_HOLD: Duration = Duration::from_millis(1200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestPhase {
    Pending,
    Confirmed,
    Failed,
}

#[derive(Default)]
struct RequestTracker {
    latest: Option<(u64, RequestPhase)>,
}

impl RequestTracker {
    fn begin(&mut self, token: u64) {
        self.latest = Some((token, RequestPhase::Pending));
    }
    fn finish(&mut self, token: u64, phase: RequestPhase) -> bool {
        if self.latest != Some((token, RequestPhase::Pending)) {
            return false;
        }
        self.latest = Some((token, phase));
        true
    }
}

#[derive(Default)]
pub struct SystemHud {
    native: Option<NativeSystemHud>,
    requests: RequestTracker,
    values: values::HudValues,
}

impl SystemHud {
    pub fn take_error(&mut self) -> Option<String> {
        self.native.as_ref().and_then(NativeSystemHud::take_error)
    }

    /// 预热专用窗口线程、字体和位图；不创建 GPUI 窗口或查询亮度。
    pub fn prewarm(&mut self) -> Result<(), String> {
        if self.native.is_none() {
            self.native = Some(NativeSystemHud::new()?);
        }
        Ok(())
    }

    pub fn begin(
        &mut self,
        token: u64,
        point: (i32, i32),
        delta_percent: i16,
    ) -> Result<(), String> {
        self.requests.begin(token);
        let placement = self.locate(point, None)?;
        let percent = self
            .values
            .begin(token, &placement.monitor_device, i32::from(delta_percent));
        self.present(placement, percent, PENDING_HOLD)
    }

    pub fn complete(
        &mut self,
        token: u64,
        result: &AdjustmentResult,
        point: (i32, i32),
    ) -> Result<(), String> {
        if result.kind != AdjustmentKind::Brightness {
            return Ok(());
        }
        if let Some(monitor) = result.monitor_device.as_deref() {
            self.values.complete(token, monitor, result.current_percent);
        }
        if !self.requests.finish(token, RequestPhase::Confirmed) {
            return Ok(());
        }
        let placement = self.locate(point, result.monitor_device.as_deref())?;
        self.present(placement, Some(result.current_percent), HOLD)
    }

    pub fn fail(&mut self, token: u64) {
        self.values.fail(token);
        if self.requests.finish(token, RequestPhase::Failed) {
            if let Some(native) = &self.native {
                native.hide();
            }
        }
    }

    fn locate(
        &self,
        point: (i32, i32),
        expected: Option<&str>,
    ) -> Result<SystemHudPlacement, String> {
        let (width, height) = NativeSystemHud::logical_size();
        omnitrigger_windows::system_hud_placement(point, expected, width, height).inspect_err(
            |_| {
                if let Some(native) = &self.native {
                    native.hide();
                }
            },
        )
    }

    fn present(
        &mut self,
        placement: SystemHudPlacement,
        percent: Option<u8>,
        hold: Duration,
    ) -> Result<(), String> {
        self.prewarm()?;
        self.native
            .as_ref()
            .expect("已初始化原生亮度提示")
            .show(placement, percent, hold)
    }
}

/// 只读预览等待的是 Win32 提交完成，不进行任何亮度读取或写入。
fn wait_visible(native: &NativeSystemHud, after_generation: u64) -> Result<Duration, String> {
    let started = Instant::now();
    loop {
        if let Some(error) = native.take_error() {
            return Err(error);
        }
        let state = native.snapshot();
        if state.visible && state.generation > after_generation {
            return Ok(started.elapsed());
        }
        if started.elapsed() > Duration::from_secs(5) {
            return Err("原生亮度提示窗口未能显示。".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn wait_hidden(native: &NativeSystemHud, hold: Duration) -> Result<(), String> {
    let deadline = Instant::now() + hold + Duration::from_secs(3);
    while native.is_visible() {
        if let Some(error) = native.take_error() {
            return Err(error);
        }
        if Instant::now() >= deadline {
            return Err("原生亮度提示窗口未按期限收起。".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

pub struct Preview {
    kind: AdjustmentKind,
    percent: u8,
    point: (i32, i32),
    pending: bool,
    auto_dismiss: bool,
    probe_cycle: bool,
}

impl Preview {
    pub fn parse(arguments: &[String]) -> Option<Result<Self, String>> {
        let index = arguments
            .iter()
            .position(|argument| argument == "--preview-system-hud")?;
        Some((|| {
            let kind = match arguments.get(index + 1).map(String::as_str) {
                Some("brightness") => AdjustmentKind::Brightness,
                Some("volume") => AdjustmentKind::Volume,
                _ => return Err("--preview-system-hud 后需要 brightness 或 volume。".into()),
            };
            let pending = arguments.iter().any(|argument| argument == "--pending");
            let probe_cycle = arguments.iter().any(|argument| argument == "--probe-cycle");
            let auto_dismiss = arguments
                .iter()
                .any(|argument| argument == "--auto-dismiss");
            if probe_cycle
                && (kind != AdjustmentKind::Brightness
                    || pending
                    || auto_dismiss
                    || arguments.iter().any(|argument| argument == "--percent"))
            {
                return Err("复用诊断只与 brightness 和 --point 组合。".into());
            }
            if pending && kind != AdjustmentKind::Brightness {
                return Err("--pending 只用于亮度浮层外观预览。".into());
            }
            if pending && arguments.iter().any(|argument| argument == "--percent") {
                return Err("等待状态不显示百分比，请移除 --percent。".into());
            }
            for argument in arguments
                .iter()
                .filter(|argument| argument.starts_with("--"))
            {
                if !matches!(
                    argument.as_str(),
                    "--preview-system-hud"
                        | "--point"
                        | "--percent"
                        | "--pending"
                        | "--auto-dismiss"
                        | "--probe-cycle"
                ) {
                    return Err("浮层预览只能与 --point、--percent、--pending、--auto-dismiss 或 --probe-cycle 组合，不执行系统调节。".into());
                }
            }
            let value = |flag: &str| -> Result<Option<&str>, String> {
                arguments
                    .iter()
                    .position(|argument| argument == flag)
                    .map(|index| {
                        arguments
                            .get(index + 1)
                            .map(String::as_str)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| format!("{flag} 后需要参数。"))
                    })
                    .transpose()
            };
            let percent = value("--percent")?
                .unwrap_or("50")
                .parse::<u8>()
                .map_err(|_| "预览百分比需要 0–100。")?;
            if percent > 100 {
                return Err("预览百分比需要 0–100。".into());
            }
            let point = if let Some(value) = value("--point")? {
                let (x, y) = value.split_once(',').ok_or("坐标格式为 x,y。")?;
                (
                    x.trim().parse().map_err(|_| "横坐标无效。")?,
                    y.trim().parse().map_err(|_| "纵坐标无效。")?,
                )
            } else {
                omnitrigger_windows::capture_adjustment_point()
                    .map_err(|error| error.to_string())?
            };
            Ok(Self {
                kind,
                percent,
                point,
                pending,
                auto_dismiss,
                probe_cycle,
            })
        })())
    }

    pub fn run(self) {
        if let Err(error) = self.try_run() {
            eprintln!("浮层预览失败：{error}");
            std::process::exit(1);
        }
    }

    fn try_run(self) -> Result<(), String> {
        if self.kind != AdjustmentKind::Brightness {
            return Err("音量使用 Windows 原生提示；此预览只支持 brightness。".into());
        }
        if self.probe_cycle {
            return probe::run(self.point);
        }
        let (width, height) = NativeSystemHud::logical_size();
        let placement = omnitrigger_windows::system_hud_placement(self.point, None, width, height)?;
        let native = NativeSystemHud::new()?;
        let hold = if self.auto_dismiss {
            if self.pending {
                PENDING_HOLD
            } else {
                HOLD
            }
        } else {
            Duration::from_secs(30)
        };
        native.show(placement, (!self.pending).then_some(self.percent), hold)?;
        wait_visible(&native, 0)?;
        wait_hidden(&native, hold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_success_and_failure_never_finish_the_latest_request() {
        let mut requests = RequestTracker::default();
        requests.begin(10);
        requests.begin(11);
        assert!(!requests.finish(10, RequestPhase::Confirmed));
        assert!(!requests.finish(10, RequestPhase::Failed));
        assert!(requests.finish(11, RequestPhase::Confirmed));
        assert!(!requests.finish(11, RequestPhase::Failed));
        requests.begin(12);
        assert!(!requests.finish(11, RequestPhase::Failed));
        assert!(requests.finish(12, RequestPhase::Failed));
    }

    #[test]
    fn preview_never_accepts_hardware_actions() {
        let args = [
            "app",
            "--preview-system-hud",
            "brightness",
            "--adjust-brightness",
            "5",
        ]
        .map(String::from);
        assert!(Preview::parse(&args).unwrap().is_err());
    }

    #[test]
    fn preview_parser_accepts_only_coherent_read_only_modes() {
        let args = [
            "app",
            "--preview-system-hud",
            "brightness",
            "--point",
            "100,100",
        ]
        .map(String::from);
        let mut pending = args.to_vec();
        pending.extend(["--pending".into(), "--auto-dismiss".into()]);
        assert!(Preview::parse(&pending).unwrap().unwrap().pending);
        pending.extend(["--percent".into(), "80".into()]);
        assert!(Preview::parse(&pending).unwrap().is_err());
        let mut cycle = args.to_vec();
        cycle.push("--probe-cycle".into());
        assert!(Preview::parse(&cycle).unwrap().unwrap().probe_cycle);
        cycle.push("--pending".into());
        assert!(Preview::parse(&cycle).unwrap().is_err());
    }
}
