use omnitrigger_windows::WindowsSettingsPage;

use super::{calculator::normalize, title_rank, Destination};

const SETTINGS: &[(&str, &str, WindowsSettingsPage)] = &[
    (
        "显示",
        "屏幕 分辨率 display screen resolution",
        WindowsSettingsPage::Display,
    ),
    (
        "声音",
        "音量 麦克风 sound audio volume microphone",
        WindowsSettingsPage::Sound,
    ),
    (
        "通知",
        "通知中心 notifications",
        WindowsSettingsPage::Notifications,
    ),
    ("蓝牙", "设备 bluetooth", WindowsSettingsPage::Bluetooth),
    (
        "存储",
        "磁盘 空间 storage disk",
        WindowsSettingsPage::Storage,
    ),
    (
        "已安装的应用",
        "应用 程序 卸载 installed apps uninstall",
        WindowsSettingsPage::InstalledApps,
    ),
    (
        "默认应用",
        "文件关联 default apps",
        WindowsSettingsPage::DefaultApps,
    ),
    (
        "启动应用",
        "开机 自启动 startup apps",
        WindowsSettingsPage::StartupApps,
    ),
];

pub(super) fn matching_windows_settings(query: &str) -> Vec<Destination> {
    let query = normalize(query).to_lowercase();
    let mut matches: Vec<_> = SETTINGS
        .iter()
        .copied()
        .filter(|(title, aliases, _)| {
            query
                .split_whitespace()
                .all(|word| title.contains(word) || aliases.contains(word))
        })
        .collect();
    matches.sort_by_key(|(title, _, _)| title_rank(title, &query));
    matches
        .into_iter()
        .map(|(title, _, page)| Destination::WindowsSetting { title, page })
        .collect()
}
