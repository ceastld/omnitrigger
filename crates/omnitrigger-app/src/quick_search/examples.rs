use crate::assets::Icon;

pub(super) struct Example {
    pub(super) icon: Icon,
    pub(super) title: &'static str,
    pub(super) query: &'static str,
}

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
        title: "筛选触发记录",
        query: "触发记录: ",
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
        icon: Icon::Clock,
        title: "将时间戳换算成本地时间",
        query: "时间戳: 1700000000 本地",
    },
    Example {
        icon: Icon::Clock,
        title: "查询日期是星期几",
        query: "日期: 2026-11-21 星期几",
    },
    Example {
        icon: Icon::Clock,
        title: "按周推算日期",
        query: "日期: 今天 + 2周",
    },
    Example {
        icon: Icon::Clock,
        title: "按月推算日期",
        query: "日期: 2024-01-31 + 1个月",
    },
    Example {
        icon: Icon::Clock,
        title: "按工作日推算日期",
        query: "日期: 2026-11-20 + 1工作日",
    },
    Example {
        icon: Icon::Clock,
        title: "计算两个日期相差的工作日",
        query: "日期: 2026-11-23 - 2026-11-20 工作日",
    },
    Example {
        icon: Icon::Layers,
        title: "查找触发预设",
        query: "预设: 顶部边缘",
    },
    Example {
        icon: Icon::Wheel,
        title: "查找轮盘槽位",
        query: "槽位: ",
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
        title: "按百分数增加",
        query: "200 增加 15%",
    },
    Example {
        icon: Icon::Terminal,
        title: "按百分数减少",
        query: "200 减少 15%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算打折后的价格",
        query: "200 打 8.5 折",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算平均值",
        query: "avg(12,18,30)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算中位数",
        query: "median(9,1,4)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算斜边或向量长度",
        query: "hypot(3,4,12)",
    },
    Example {
        icon: Icon::Terminal,
        title: "按象限计算方向角",
        query: "degrees(atan2(1,-1))",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算指定底数的对数",
        query: "log(1024,2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "用 pow 计算幂",
        query: "pow(2,10)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算反三角函数",
        query: "degrees(asin(0.5))",
    },
    Example {
        icon: Icon::Terminal,
        title: "取整（中点取偶）",
        query: "round(2.5)",
    },
    Example {
        icon: Icon::Terminal,
        title: "向零取整",
        query: "trunc(-1.8)",
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
    Example {
        icon: Icon::Layers,
        title: "换算能量",
        query: "1 kWh to MJ",
    },
    Example {
        icon: Icon::Layers,
        title: "换算功率",
        query: "500 mW to W",
    },
    Example {
        icon: Icon::Layers,
        title: "换算密度",
        query: "1000 kg/m³ to g/cm³",
    },
    Example {
        icon: Icon::Layers,
        title: "换算体积流量",
        query: "1 L/min to m³/h",
    },
    Example {
        icon: Icon::Layers,
        title: "换算 HEX 和 RGB 颜色",
        query: "颜色: #3A86FF",
    },
    Example {
        icon: Icon::Layers,
        title: "将 HSL 颜色转为 HEX",
        query: "颜色: hsl(210 100% 50%)",
    },
    Example {
        icon: Icon::Terminal,
        title: "找回用过的算式",
        query: "算式历史: ",
    },
    Example {
        icon: Icon::Layers,
        title: "选择换算目标单位",
        query: "10 km to",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计字符与 UTF-8 字节",
        query: "字数: 你好👩‍💻",
    },
    Example {
        icon: Icon::Browser,
        title: "编码 URL 参数值",
        query: "URL: 编码 北京 A+B",
    },
    Example {
        icon: Icon::Browser,
        title: "解码 URL 参数值",
        query: "URL: 解码 %E5%8C%97%E4%BA%AC%20A%2BB",
    },
    Example {
        icon: Icon::Terminal,
        title: "约分",
        query: "分数: 42/56",
    },
    Example {
        icon: Icon::Terminal,
        title: "从小数求分数",
        query: "分数: 0.125",
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
        .collect()
}
