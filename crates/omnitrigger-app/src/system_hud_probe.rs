//! 独立原生窗口复用与提交耗时诊断；不安装 Hook、不读写真实亮度。
use super::*;
use serde_json::json;

pub(super) fn run(point: (i32, i32)) -> Result<(), String> {
    let native = NativeSystemHud::new()?;
    let (width, height) = NativeSystemHud::logical_size();
    let placement = omnitrigger_windows::system_hud_placement(point, None, width, height)?;
    for (cycle, percent) in [(1, 35), (2, 40)] {
        let before = native.snapshot();
        println!(
            "{}",
            json!({"stage": "before_show", "cycle": cycle, "visible": before.visible, "generation": before.generation})
        );

        let hold = Duration::from_secs(4);
        native.show(placement.clone(), Some(percent), hold)?;
        wait_visible(&native, before.generation)?;
        let shown = native.snapshot();
        println!(
            "{}",
            json!({"stage": "shown", "cycle": cycle, "submit_to_present_us": shown.last_present_us, "hwnd": shown.hwnd, "generation": shown.generation, "visible": shown.visible, "percent": shown.percent})
        );
        wait_hidden(&native, hold)?;
        println!(
            "{}",
            json!({"stage": "hidden", "cycle": cycle, "visible": native.is_visible()})
        );
    }
    Ok(())
}
