//! 内置单色图标。GPUI 只取 SVG 的透明度作为蒙版，颜色由元素的文字颜色决定。

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! icon {
    ($body:expr) => {
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"##,
            $body,
            "</svg>"
        )
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Logo,
    Pause,
    Play,
    Refresh,
    Record,
    Keyboard,
    Clock,
    Pulse,
    Mouse,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Check,
    Alert,
    Trash,
}

const ICONS: &[(Icon, &str, &str)] = &[
    (
        Icon::Logo,
        "icons/logo.svg",
        icon!(
            r##"<rect x="5" y="5" width="14" height="11" rx="3" stroke-width="2.2"/><path d="M7.5 19.5h9" stroke-width="2.2"/>"##
        ),
    ),
    (
        Icon::Pause,
        "icons/pause.svg",
        icon!(r##"<path d="M9 5v14M15 5v14" stroke-width="2.4"/>"##),
    ),
    (
        Icon::Play,
        "icons/play.svg",
        icon!(r##"<path d="M7 4.5v15l12-7.5z" fill="#000" stroke="none"/>"##),
    ),
    (
        Icon::Refresh,
        "icons/refresh.svg",
        icon!(
            r##"<path d="M20 11a8 8 0 1 0-2.3 5.7" stroke-width="2.2"/><path d="M20 4v7h-7" stroke-width="2.2"/>"##
        ),
    ),
    (
        Icon::Record,
        "icons/record.svg",
        icon!(
            r##"<circle cx="12" cy="12" r="9" stroke-width="2.2"/><circle cx="12" cy="12" r="4" fill="#000" stroke="none"/>"##
        ),
    ),
    (
        Icon::Keyboard,
        "icons/keyboard.svg",
        icon!(
            r##"<rect x="3" y="6" width="18" height="12" rx="2.5"/><path d="M7 10h.01M11 10h.01M15 10h.01M8 14h8"/>"##
        ),
    ),
    (
        Icon::Clock,
        "icons/clock.svg",
        icon!(r##"<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>"##),
    ),
    (
        Icon::Pulse,
        "icons/pulse.svg",
        icon!(r##"<path d="M3 12h4l2.5-6 5 12 2.5-6h4"/>"##),
    ),
    (
        Icon::Mouse,
        "icons/mouse.svg",
        icon!(
            r##"<rect x="6" y="3" width="12" height="18" rx="6" stroke-width="2.2"/><path d="M12 7v4" stroke-width="2.2"/>"##
        ),
    ),
    (
        Icon::ArrowUp,
        "icons/arrow-up.svg",
        icon!(r##"<path d="M12 19V5M6 11l6-6 6 6" stroke-width="2.6"/>"##),
    ),
    (
        Icon::ArrowDown,
        "icons/arrow-down.svg",
        icon!(r##"<path d="M12 5v14M6 13l6 6 6-6" stroke-width="2.6"/>"##),
    ),
    (
        Icon::ArrowLeft,
        "icons/arrow-left.svg",
        icon!(r##"<path d="M19 12H5M11 6l-6 6 6 6" stroke-width="2.6"/>"##),
    ),
    (
        Icon::ArrowRight,
        "icons/arrow-right.svg",
        icon!(r##"<path d="M5 12h14M13 6l6 6-6 6" stroke-width="2.6"/>"##),
    ),
    (
        Icon::Check,
        "icons/check.svg",
        icon!(r##"<path d="M5 12.5 10 17 19 7.5" stroke-width="2.6"/>"##),
    ),
    (
        Icon::Alert,
        "icons/alert.svg",
        icon!(
            r##"<path d="M12 3.5 2.5 20h19z" stroke-width="2.2"/><path d="M12 10v4.5M12 17.5h.01" stroke-width="2.2"/>"##
        ),
    ),
    (
        Icon::Trash,
        "icons/trash.svg",
        icon!(r##"<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>"##),
    ),
];

impl Icon {
    pub fn path(self) -> &'static str {
        ICONS
            .iter()
            .find(|(icon, _, _)| *icon == self)
            .map(|(_, path, _)| *path)
            .unwrap_or("icons/missing.svg")
    }
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(_, candidate, _)| *candidate == path)
            .map(|(_, _, svg)| Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(_, candidate, _)| candidate.starts_with(path))
            .map(|(_, candidate, _)| SharedString::from(*candidate))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_resolves_to_a_distinct_svg() {
        let mut paths: Vec<_> = ICONS.iter().map(|(icon, _, _)| icon.path()).collect();
        let count = paths.len();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), count);
        for path in paths {
            let bytes = Assets.load(path).unwrap().unwrap();
            assert!(std::str::from_utf8(&bytes).unwrap().starts_with("<svg"));
        }
    }
}
