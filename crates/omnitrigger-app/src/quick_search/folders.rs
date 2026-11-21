use omnitrigger_windows::ShellFolder;

use super::{calculator::normalize, title_rank, Destination};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SearchFolder {
    pub(crate) kind: ShellFolder,
    pub(super) title: &'static str,
    aliases: &'static str,
}

const FOLDERS: &[SearchFolder] = &[
    SearchFolder {
        kind: ShellFolder::Desktop,
        title: "桌面",
        aliases: "desktop",
    },
    SearchFolder {
        kind: ShellFolder::Downloads,
        title: "下载",
        aliases: "downloads",
    },
    SearchFolder {
        kind: ShellFolder::Documents,
        title: "文档",
        aliases: "我的文档 documents",
    },
    SearchFolder {
        kind: ShellFolder::Pictures,
        title: "图片",
        aliases: "照片 pictures photos",
    },
    SearchFolder {
        kind: ShellFolder::Music,
        title: "音乐",
        aliases: "music audio",
    },
    SearchFolder {
        kind: ShellFolder::Videos,
        title: "视频",
        aliases: "videos movies",
    },
];

pub(super) fn matching_folders(query: &str) -> Vec<Destination> {
    let query = normalize(query).to_lowercase();
    let mut matches: Vec<_> = FOLDERS
        .iter()
        .copied()
        .filter(|folder| {
            query
                .split_whitespace()
                .all(|word| folder.title.contains(word) || folder.aliases.contains(word))
        })
        .collect();
    matches.sort_by_key(|folder| title_rank(folder.title, &query));
    matches.into_iter().map(Destination::KnownFolder).collect()
}
