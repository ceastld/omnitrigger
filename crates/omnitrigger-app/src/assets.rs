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
    Bell,
    Terminal,
    Capture,
    Search,
    Sliders,
    Close,
    Enter,
    Bolt,
    Grid,
    Layers,
    Panel,
}

const ICONS: &[(Icon, &str, &str)] = &[
    (
        Icon::Logo,
        "icons/logo.svg",
        icon!(
            r##"<rect x="3.5" y="3.5" width="17" height="14" rx="3.5"/><path d="M7 21h10"/><path d="M13.2 6.2 8.8 11.6h3.1l-.9 3.7 4.3-5.4h-3.1z" fill="#000" stroke-width="1.1"/>"##
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
    (
        Icon::Bell,
        "icons/bell.svg",
        icon!(
            r##"<path d="M6.5 16v-5a5.5 5.5 0 0 1 11 0v5l1.5 2h-14z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>"##
        ),
    ),
    (
        Icon::Terminal,
        "icons/terminal.svg",
        icon!(
            r##"<rect x="3" y="4.5" width="18" height="15" rx="3"/><path d="m7.5 9.5 3 2.5-3 2.5M13 15h4"/>"##
        ),
    ),
    (
        Icon::Capture,
        "icons/capture.svg",
        icon!(
            r##"<path d="M4 8V5.5A1.5 1.5 0 0 1 5.5 4H8M16 4h2.5A1.5 1.5 0 0 1 20 5.5V8M20 16v2.5a1.5 1.5 0 0 1-1.5 1.5H16M8 20H5.5A1.5 1.5 0 0 1 4 18.5V16"/><circle cx="12" cy="12" r="3"/>"##
        ),
    ),
    (
        Icon::Search,
        "icons/search.svg",
        icon!(
            r##"<circle cx="11" cy="11" r="6.5" stroke-width="2.2"/><path d="m20 20-4.2-4.2" stroke-width="2.2"/>"##
        ),
    ),
    (
        Icon::Sliders,
        "icons/sliders.svg",
        icon!(
            r##"<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>"##
        ),
    ),
    (
        Icon::Close,
        "icons/close.svg",
        icon!(r##"<path d="M6 6l12 12M18 6 6 18" stroke-width="2.2"/>"##),
    ),
    (
        Icon::Enter,
        "icons/enter.svg",
        icon!(
            r##"<path d="M19 5v5a3 3 0 0 1-3 3H6" stroke-width="2.4"/><path d="m9 9-4 4 4 4" stroke-width="2.4"/>"##
        ),
    ),
    (
        Icon::Bolt,
        "icons/bolt.svg",
        icon!(r##"<path d="M13.5 2.5 4.5 13.5h7l-1 8 9-11h-7z" fill="#000"/>"##),
    ),
    (
        Icon::Grid,
        "icons/grid.svg",
        icon!(
            r##"<rect x="4" y="4" width="6.5" height="6.5" rx="1.8"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="1.8"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="1.8"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="1.8"/>"##
        ),
    ),
    (
        Icon::Layers,
        "icons/layers.svg",
        icon!(
            r##"<path d="m12 3.5 8.5 4.5-8.5 4.5L3.5 8z"/><path d="m3.5 12.5 8.5 4.5 8.5-4.5M3.5 16.5 12 21l8.5-4.5"/>"##
        ),
    ),
    (
        Icon::Panel,
        "icons/panel.svg",
        icon!(
            r##"<rect x="3.5" y="4.5" width="17" height="15" rx="3"/><path d="M3.5 9.5h17"/><rect x="7" y="12.5" width="4" height="3.5" rx="1" fill="#000" stroke="none"/><rect x="13" y="12.5" width="4" height="3.5" rx="1" fill="#000" stroke="none"/>"##
        ),
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
