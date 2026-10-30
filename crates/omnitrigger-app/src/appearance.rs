//! 用户界面主题偏好，与动作绑定分开保存。

use std::path::{Path, PathBuf};

use gpui::WindowAppearance;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    System,
    Light,
    Dark,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "跟随系统",
            Self::Light => "浅色",
            Self::Dark => "深色",
        }
    }

    pub fn is_dark(self, appearance: WindowAppearance) -> bool {
        match self {
            Self::System => matches!(
                appearance,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preference {
    #[serde(default)]
    mode: Mode,
}

/// `bindings_path` 为当前使用的绑定配置文件，主题与其位于同一目录。
pub fn path_for(bindings_path: &Path) -> PathBuf {
    bindings_path.with_file_name("appearance.json")
}

pub fn load(bindings_path: &Path) -> Result<Mode, String> {
    let path = path_for(bindings_path);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Mode::default()),
        Err(error) => return Err(format!("读取主题设置失败（{}）：{error}", path.display())),
    };
    serde_json::from_slice::<Preference>(&bytes)
        .map(|settings| settings.mode)
        .map_err(|error| format!("主题设置格式无效（{}）：{error}", path.display()))
}

pub fn save(bindings_path: &Path, mode: Mode) -> Result<(), String> {
    let path = path_for(bindings_path);
    let bytes = serde_json::to_vec_pretty(&Preference { mode })
        .map_err(|error| format!("生成主题设置失败：{error}"))?;
    crate::settings::atomic_write(&path, &bytes)
        .map_err(|error| format!("保存主题设置失败（{}）：{error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_mode_tracks_both_standard_and_vibrant_window_appearance() {
        assert!(!Mode::System.is_dark(WindowAppearance::Light));
        assert!(!Mode::System.is_dark(WindowAppearance::VibrantLight));
        assert!(Mode::System.is_dark(WindowAppearance::Dark));
        assert!(Mode::System.is_dark(WindowAppearance::VibrantDark));
        assert!(!Mode::Light.is_dark(WindowAppearance::Dark));
        assert!(Mode::Dark.is_dark(WindowAppearance::Light));
    }

    #[test]
    fn preferences_are_explicit_and_do_not_accept_unknown_modes() {
        for mode in Mode::ALL {
            let bytes = serde_json::to_vec(&Preference { mode }).unwrap();
            assert_eq!(
                serde_json::from_slice::<Preference>(&bytes).unwrap().mode,
                mode
            );
        }
        assert_eq!(
            serde_json::from_str::<Preference>("{}").unwrap().mode,
            Mode::System
        );
        assert!(serde_json::from_str::<Preference>(r#"{"mode":"unknown"}"#).is_err());
        assert!(serde_json::from_str::<Preference>(r#"{"mode":"dark","extra":true}"#).is_err());
    }

    #[test]
    fn theme_path_is_adjacent_to_the_selected_binding_file() {
        assert_eq!(
            path_for(Path::new("custom/config/bindings.json")),
            PathBuf::from("custom/config/appearance.json")
        );
    }
}
