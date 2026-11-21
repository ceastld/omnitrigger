use crate::assets::Icon;

pub(super) struct Example {
    pub(super) icon: Icon,
    pub(super) title: &'static str,
    pub(super) query: &'static str,
}

const RESULT_LIMIT: usize = 8;

pub(super) const EXAMPLES: &[Example] = &[
    Example {
        icon: Icon::Terminal,
        title: "计算算式",
        query: "2**10",
    },
    Example {
        icon: Icon::Layers,
        title: "换算单位",
        query: "10 km to mi",
    },
    Example {
        icon: Icon::Terminal,
        title: "转换整数进制",
        query: "0xff to dec",
    },
    Example {
        icon: Icon::Clock,
        title: "计算日期",
        query: "日期: 今天 + 7天",
    },
    Example {
        icon: Icon::Keyboard,
        title: "搜索触发器",
        query: "触发器: ",
    },
    Example {
        icon: Icon::Wheel,
        title: "搜索轮盘",
        query: "轮盘: ",
    },
    Example {
        icon: Icon::Grid,
        title: "筛选动作库",
        query: "动作: ",
    },
    Example {
        icon: Icon::Clock,
        title: "查找文本历史正文",
        query: "剪贴板: ",
    },
    Example {
        icon: Icon::Clock,
        title: "时长换算",
        query: "时长: 5400秒",
    },
    Example {
        icon: Icon::Clock,
        title: "时长加减",
        query: "时长: 01:30:00 - 25分钟",
    },
    Example {
        icon: Icon::Clock,
        title: "Unix 时间戳",
        query: "时间戳: 1700000000123ms",
    },
    Example {
        icon: Icon::Clock,
        title: "当前 Unix 时间戳",
        query: "unix: now",
    },
    Example {
        icon: Icon::Layers,
        title: "查找触发预设",
        query: "预设: 顶部边缘",
    },
    Example {
        icon: Icon::Sliders,
        title: "打开颜色主题设置",
        query: "设置: 主题",
    },
    Example {
        icon: Icon::Sliders,
        title: "打开配置备份设置",
        query: "settings: backup",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算百分数",
        query: "15% of 200",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算平均值",
        query: "avg(12,18,30)",
    },
    Example {
        icon: Icon::Layers,
        title: "换算数据速率",
        query: "8Mbps to MB/s",
    },
    Example {
        icon: Icon::Layers,
        title: "换算角度",
        query: "180° to rad",
    },
    Example {
        icon: Icon::Layers,
        title: "换算压强",
        query: "1 atm to kPa",
    },
    Example {
        icon: Icon::Layers,
        title: "换算频率",
        query: "2 MHz to kHz",
    },
];

pub(super) fn matching_examples(query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    EXAMPLES
        .iter()
        .enumerate()
        .filter(|(_, example)| {
            let title = example.title.to_lowercase();
            let example_query = example.query.to_lowercase();
            query
                .split_whitespace()
                .all(|word| title.contains(word) || example_query.contains(word))
        })
        .map(|(index, _)| index)
        .take(RESULT_LIMIT)
        .collect()
}
