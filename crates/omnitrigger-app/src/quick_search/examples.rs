use std::sync::OnceLock;

use crate::assets::Icon;

use super::calculator::narrow_ascii_graphic;
use super::scientific::split_name;
use super::scope::{scoped_query, Scope};
use super::title_rank;

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
        icon: Icon::Terminal,
        title: "计算平方根",
        query: "sqrt(2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算弧度的正弦",
        query: "sin(pi/2)",
    },
    Example {
        icon: Icon::Layers,
        title: "换算单位",
        query: "10 km to mi",
    },
    Example {
        icon: Icon::Grid,
        title: "按标题或程序名查找当前窗口",
        query: "窗口:",
    },
    Example {
        icon: Icon::Layers,
        title: "按符号筛选换算单位",
        query: "1 GiB to Ti",
    },
    Example {
        icon: Icon::Terminal,
        title: "转换整数进制",
        query: "0xff to dec",
    },
    Example {
        icon: Icon::Terminal,
        title: "选择目标进制",
        query: "0xff to",
    },
    Example {
        icon: Icon::Terminal,
        title: "在算式中混用进制",
        query: "0xFF + 0b1",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算按位与、或与异或",
        query: "bitand(0xff,0x0f) + bitor(8,5) + bitxor(10,12)",
    },
    Example {
        icon: Icon::Terminal,
        title: "按位移位",
        query: "shl(0b101,3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计二进制中的 1",
        query: "popcount(0xff)",
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
        icon: Icon::Grid,
        title: "搜索 Windows 应用",
        query: "应用:",
    },
    Example {
        icon: Icon::Keyboard,
        title: "搜索已启用的触发器",
        query: "已启用触发器: ",
    },
    Example {
        icon: Icon::Keyboard,
        title: "搜索已停用的触发器",
        query: "已停用触发器: ",
    },
    Example {
        icon: Icon::Keyboard,
        title: "搜索全局触发器",
        query: "全局触发器: ",
    },
    Example {
        icon: Icon::Keyboard,
        title: "搜索指定应用的触发器",
        query: "指定应用触发器: ",
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
        title: "毫秒时长相加",
        query: "时长: 00:00:01.25 + 500ms",
    },
    Example {
        icon: Icon::Clock,
        title: "时长乘以次数",
        query: "时长: 45分钟 * 12",
    },
    Example {
        icon: Icon::Clock,
        title: "时长换算为指定单位",
        query: "时长: 01:30:00 to min",
    },
    Example {
        icon: Icon::Clock,
        title: "估算下载时间",
        query: "下载: 1 GiB 按 100 Mbps",
    },
    Example {
        icon: Icon::Clock,
        title: "用传输量和耗时求平均速率",
        query: "下载: 600 MB 用时 2分钟",
    },
    Example {
        icon: Icon::Clock,
        title: "按带宽和用时估算传输量",
        query: "下载: 1 MiB/s 用时 2秒",
    },
    Example {
        icon: Icon::Layers,
        title: "化简宽高比",
        query: "宽高比: 1920×1080",
    },
    Example {
        icon: Icon::Layers,
        title: "按宽度等比缩放",
        query: "宽高比: 1920×1080 按宽2560",
    },
    Example {
        icon: Icon::Layers,
        title: "查看 IPv4 网段",
        query: "网段: 192.0.2.137/26",
    },
    Example {
        icon: Icon::Layers,
        title: "判断地址是否在网段",
        query: "网段: 2001:db8::/64 包含 2001:db8::1",
    },
    Example {
        icon: Icon::Clock,
        title: "Unix 时间戳",
        query: "时间戳: 1700000000123ms",
    },
    Example {
        icon: Icon::Clock,
        title: "微秒时间戳",
        query: "时间戳: 1700000000123456us",
    },
    Example {
        icon: Icon::Clock,
        title: "时间换算为纳秒",
        query: "时间戳: 1970-01-01T00:00:00.000000001Z",
    },
    Example {
        icon: Icon::Clock,
        title: "当前 Unix 时间戳",
        query: "unix: now",
    },
    Example {
        icon: Icon::Clock,
        title: "计算指定时长后的时间",
        query: "时间戳: 现在 + 90分钟 to UTC+8",
    },
    Example {
        icon: Icon::Clock,
        title: "将时间戳换算成本地时间",
        query: "时间戳: 1700000000 本地",
    },
    Example {
        icon: Icon::Clock,
        title: "换算到指定时区偏移",
        query: "时间戳: 1700000000123ms to +08:00",
    },
    Example {
        icon: Icon::Clock,
        title: "将带偏移量的时间换算为 UTC",
        query: "时间戳: 1970-01-01T08:00:00+08:00 to UTC",
    },
    Example {
        icon: Icon::Clock,
        title: "查询日期是星期几",
        query: "日期: 2026-11-21 星期几",
    },
    Example {
        icon: Icon::Clock,
        title: "查询中文日期的星期",
        query: "日期: 2026年11月21日 星期几",
    },
    Example {
        icon: Icon::Clock,
        title: "查询日期的 ISO 周号",
        query: "日期: 2027-01-01 第几周",
    },
    Example {
        icon: Icon::Clock,
        title: "用年内天数查日期",
        query: "日期: 2024 第60天",
    },
    Example {
        icon: Icon::Clock,
        title: "查询日期是一年第几天",
        query: "日期: 2024-02-29 第几天",
    },
    Example {
        icon: Icon::Clock,
        title: "查询月份范围与工作日数",
        query: "日期: 2024-02",
    },
    Example {
        icon: Icon::Clock,
        title: "查全年日期和工作日",
        query: "日期: 2024",
    },
    Example {
        icon: Icon::Clock,
        title: "查某月第几个星期几",
        query: "日期: 2026-11 第4个星期四",
    },
    Example {
        icon: Icon::Clock,
        title: "查月份最后一个周日",
        query: "日期: 2026-11 最后一个周日",
    },
    Example {
        icon: Icon::Clock,
        title: "查询季度范围与工作日数",
        query: "日期: 2026-Q4",
    },
    Example {
        icon: Icon::Clock,
        title: "查询 ISO 周号的日期范围",
        query: "日期: 2026-W47",
    },
    Example {
        icon: Icon::Clock,
        title: "把 ISO 周日期转成日历日期",
        query: "日期: 2026-W53-5",
    },
    Example {
        icon: Icon::Clock,
        title: "查询日期的 ISO 周日期",
        query: "日期: 2027-01-01 周日期",
    },
    Example {
        icon: Icon::Clock,
        title: "查询本周的日期范围",
        query: "日期: 本周",
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
        icon: Icon::Clock,
        title: "按年月日计算日期差",
        query: "日期: 2025-03-15 - 2024-01-31 年月日",
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
        title: "把百分数转换为小数",
        query: "12.5%",
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
        title: "计算加权平均值",
        query: "wavg(80,2,90,3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算几何平均值",
        query: "gmean(1,16)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算调和平均值",
        query: "hmean(1,4)",
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
        title: "在两个端点之间线性插值",
        query: "lerp(10,20,0.25)",
    },
    Example {
        icon: Icon::Terminal,
        title: "将数值映射到另一个区间",
        query: "remap(512,0,1024,0,100)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算正整数次方根",
        query: "root(-32,5)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算数值在线性区间中的位置",
        query: "invlerp(10,20,12.5)",
    },
    Example {
        icon: Icon::Terminal,
        title: "将数值限制在区间内",
        query: "clamp(18,0,10)",
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
        title: "使用根号计算",
        query: "√16 + ∛(-27)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算最大公约数",
        query: "gcd(24,-36,60)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算最小公倍数",
        query: "lcm(4,6,10)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算阶乘",
        query: "factorial(10)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算组合数",
        query: "comb(52,5)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算排列数",
        query: "perm(5,3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算样本标准差",
        query: "stdev(1,3,5)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算总体标准差",
        query: "pstdev(1,3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算均方根",
        query: "rms(0,6,0,-8)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算皮尔逊相关系数",
        query: "corr(1,2,2,4,3,6)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算分位数",
        query: "quantile(0.25,4,1,3,2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算百分位数",
        query: "percentile(90,10,20,30,40)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算四分位距",
        query: "iqr(4,1,3,2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算反三角函数",
        query: "asind(0.5)",
    },
    Example {
        icon: Icon::Terminal,
        title: "按度数求方向角",
        query: "atan2d(1,-1)",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接按度数求正弦",
        query: "sind(30)",
    },
    Example {
        icon: Icon::Terminal,
        title: "取整（中点取偶）",
        query: "round(2.5)",
    },
    Example {
        icon: Icon::Terminal,
        title: "按小数位舍入",
        query: "round(2.675, 2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "按整数位舍入",
        query: "round(1250, -2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算小量的 ln(1+x) 与 exp(x)−1",
        query: "log1p(1e-20) + expm1(1e-20)",
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
        title: "换算角分和角秒",
        query: "60′ to °",
    },
    Example {
        icon: Icon::Layers,
        title: "转成度分秒",
        query: "12.5° to dms",
    },
    Example {
        icon: Icon::Layers,
        title: "换算温差",
        query: "10 dC to dF",
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
        title: "换算电压",
        query: "3.3 V to mV",
    },
    Example {
        icon: Icon::Layers,
        title: "换算电流",
        query: "500 mA to A",
    },
    Example {
        icon: Icon::Layers,
        title: "换算电阻",
        query: "2.2 kΩ to Ω",
    },
    Example {
        icon: Icon::Layers,
        title: "换算 CSS 排版长度",
        query: "12 pt to px",
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
        title: "换算力",
        query: "1 kgf to N",
    },
    Example {
        icon: Icon::Layers,
        title: "换算加速度",
        query: "1 g0 to m/s²",
    },
    Example {
        icon: Icon::Layers,
        title: "换算纳米与微米",
        query: "500 nm to um",
    },
    Example {
        icon: Icon::Layers,
        title: "换算英尺和英寸的组合长度",
        query: "5 ft 10 in to cm",
    },
    Example {
        icon: Icon::Layers,
        title: "把厘米转成英尺与英寸",
        query: "177.8 cm to ft in",
    },
    Example {
        icon: Icon::Layers,
        title: "换算纳秒与微秒",
        query: "125 ns to us",
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
        icon: Icon::Layers,
        title: "查 CSS 色值",
        query: "颜色: rebeccapurple",
    },
    Example {
        icon: Icon::Layers,
        title: "将 HEX 颜色转为 HSL",
        query: "颜色: #FF800080 to hsl",
    },
    Example {
        icon: Icon::Layers,
        title: "换算 HWB 颜色",
        query: "颜色: hwb(210 10% 20%)",
    },
    Example {
        icon: Icon::Layers,
        title: "计算两个颜色的对比度",
        query: "对比度: #fff vs #000",
    },
    Example {
        icon: Icon::Layers,
        title: "将半透明颜色叠加到底色",
        query: "颜色: #0080FF80 叠加 #fff",
    },
    Example {
        icon: Icon::Terminal,
        title: "找回用过的算式",
        query: "算式历史: ",
    },
    Example {
        icon: Icon::Clock,
        title: "找回本次已用换算",
        query: "换算历史: ",
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
        icon: Icon::Terminal,
        title: "转成大写",
        query: "文本: 大写 Straße",
    },
    Example {
        icon: Icon::Terminal,
        title: "转成小写",
        query: "文本: 小写 ÉTÉ",
    },
    Example {
        icon: Icon::Terminal,
        title: "合并连续空白",
        query: "文本: 合并空白 A   B",
    },
    Example {
        icon: Icon::Terminal,
        title: "替换文字",
        query: r#"文本: 替换 "北京" "上海" 北京 A+B"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "倒序文字",
        query: "文本: 倒序 你好ABC",
    },
    Example {
        icon: Icon::Terminal,
        title: "按行去重",
        query: "文本: 按行去重 苹果\n梨\n苹果",
    },
    Example {
        icon: Icon::Terminal,
        title: "把全角字母和数字转为半角",
        query: "文本: 转半角 ＡＢＣ１２３！",
    },
    Example {
        icon: Icon::Terminal,
        title: "把半角字母和数字转为全角",
        query: "文本: 转全角 ABC123!",
    },
    Example {
        icon: Icon::Terminal,
        title: "转为下划线命名",
        query: "文本: 下划线命名 HTTPServer2URL",
    },
    Example {
        icon: Icon::Terminal,
        title: "转为小驼峰命名",
        query: "文本: 小驼峰命名 http_server2_url",
    },
    Example {
        icon: Icon::Terminal,
        title: "按行排序",
        query: "文本: 按行排序 B\nA\nC",
    },
    Example {
        icon: Icon::Terminal,
        title: "清理每行首尾空白",
        query: "文本: 逐行去空白 \"  苹果  \n 梨  \"",
    },
    Example {
        icon: Icon::Terminal,
        title: "按数字顺序排列文件名",
        query: "文本: 自然排序 照片10\n照片2\n照片1",
    },
    Example {
        icon: Icon::Terminal,
        title: "移除空行",
        query: "文本: 去空行 甲\n\n乙",
    },
    Example {
        icon: Icon::Terminal,
        title: "按行编号",
        query: "文本: 按行编号 苹果\n梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计每行出现次数",
        query: "文本: 按行计数 苹果\n梨\n苹果",
    },
    Example {
        icon: Icon::Terminal,
        title: "用分隔符拼接多行",
        query: "文本: 按行拼接 \", \" 苹果\n梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "按分隔符拆分成行",
        query: "文本: 拆分成行 \"、\" 苹果、梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "只保留含关键词的行",
        query: "文本: 保留行 \"错误\" 成功\n错误 A\n错误 B",
    },
    Example {
        icon: Icon::Terminal,
        title: "排除含关键词的行",
        query: "文本: 排除行 \"错误\" 成功 A\n错误 B\n成功 C",
    },
    Example {
        icon: Icon::Terminal,
        title: "提取制表符分隔文本的一列",
        query: "文本: 取列 \"\\t\" 2 苹果\t42\n梨\t7",
    },
    Example {
        icon: Icon::Terminal,
        title: "保留符合正则表达式的行",
        query: "文本: 正则保留行 \"^[0-9]+$\" 42\nA3\n007",
    },
    Example {
        icon: Icon::Terminal,
        title: "排除符合正则表达式的行",
        query: "文本: 正则排除行 \"^[0-9]+$\" 42\nA3\n007",
    },
    Example {
        icon: Icon::Terminal,
        title: "按行提取正则匹配",
        query: "文本: 正则提取 \"[0-9]+\" 文件 42 大小 12\n无编号\n文件 007",
    },
    Example {
        icon: Icon::Terminal,
        title: "用正则表达式替换文本",
        query: r#"文本: 正则替换 "([0-9]{4})/([0-9]{2})/([0-9]{2})" "$1-$2-$3" 2026/11/21"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "计算文本 SHA-256",
        query: "文本: SHA-256 北京",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算文本 SHA-512",
        query: "文本: SHA-512 abc",
    },
    Example {
        icon: Icon::Terminal,
        title: "Base64 编码文本",
        query: "文本: Base64编码 北京",
    },
    Example {
        icon: Icon::Terminal,
        title: "Base64 解码文本",
        query: "文本: Base64解码 5YyX5Lqs",
    },
    Example {
        icon: Icon::Terminal,
        title: "Base64URL 编码文本",
        query: "文本: Base64URL编码 😀",
    },
    Example {
        icon: Icon::Terminal,
        title: "Base64URL 解码文本",
        query: "文本: Base64URL解码 8J-YgA",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看 Unicode 码位",
        query: "文本: 码位 北京",
    },
    Example {
        icon: Icon::Terminal,
        title: "Unicode 码位转文字",
        query: "文本: 码位解码 U+5317 U+4EAC",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看 UTF-8 十六进制字节",
        query: "文本: UTF8编码 北京",
    },
    Example {
        icon: Icon::Terminal,
        title: "将 UTF-8 十六进制字节转回文字",
        query: "文本: UTF8解码 E5 8C 97 E4 BA AC",
    },
    Example {
        icon: Icon::Terminal,
        title: "转成 JSON 字符串",
        query: r#"文本: JSON转义 他写下 "你好""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "还原 JSON 字符串",
        query: r#"文本: JSON还原 "\u4F60\n好""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "格式化 JSON",
        query: r#"JSON: 格式化 {"名称":"北京","值":1}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "压缩 JSON",
        query: r#"JSON: 压缩 { "名称": "北京", "值": 1 }"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按路径提取 JSON 值",
        query: r#"JSON: 取值 /商品/0/价格 {"商品":[{"价格":9007199254740993}]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "提取 JSON 数组的一列",
        query: r#"JSON: 取列 /价格 [{"价格":9.90},{"价格":2e2}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "列出 JSON 对象的顶层键名",
        query: r#"JSON: 键名 {"城市":{"名称":"北京"},"数量":1}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "统计 JSON 顶层条目数",
        query: r#"JSON: 长度 [1,{"数量":2},null]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把数组展开为 JSONL",
        query: r#"JSON: 展开数组 [{"城市":"北京"},{"城市":"上海"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把 JSONL 合并成数组",
        query: "JSON: 合并行 {\"城市\":\"北京\"}\n{\"城市\":\"上海\"}",
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
        icon: Icon::Browser,
        title: "编码表单参数值",
        query: "URL: 表单编码 北京 A+B",
    },
    Example {
        icon: Icon::Browser,
        title: "解码表单参数值",
        query: "URL: 表单解码 %E5%8C%97%E4%BA%AC+A%2BB",
    },
    Example {
        icon: Icon::Browser,
        title: "拆解 URL 查询参数",
        query: "URL: 参数 https://example.invalid/?tag=a+1&tag=b%2Bc",
    },
    Example {
        icon: Icon::Browser,
        title: "拆解 URL 组成",
        query: "URL: 拆解 https://example.invalid:8443/a%20b?x=1#part",
    },
    Example {
        icon: Icon::Browser,
        title: "移除 URL 查询串",
        query: "URL: 去查询串 https://example.invalid/doc?source=mail#part",
    },
    Example {
        icon: Icon::Browser,
        title: "移除指定名称的 URL 参数",
        query: "URL: 去参数 source https://example.invalid/doc?q=a%20b&source=mail#part",
    },
    Example {
        icon: Icon::Browser,
        title: "提取 URL 的源",
        query: "URL: 源 https://example.invalid:8443/doc?x=1#part",
    },
    Example {
        icon: Icon::Browser,
        title: "把 Windows 文件路径转为链接",
        query: r#"URL: 文件链接 "C:\Docs\计划 #1.txt""#,
    },
    Example {
        icon: Icon::Browser,
        title: "从 file URL 提取 Windows 路径",
        query: "URL: 文件路径 file:///C:/Docs/a%20b.txt",
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
    Example {
        icon: Icon::Terminal,
        title: "计算变化率",
        query: "百分比: 变化 200 到 150",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算占比",
        query: "百分比: 占比 18 占 60",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算百分点差",
        query: "百分比: 百分点 15% 到 21.5%",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算分数算式",
        query: "分数: 计算 (1/2+1/3)*6",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算整数次幂",
        query: "分数: 计算 2**80",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算阶乘",
        query: "分数: 计算 50!",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确整除并向下取整",
        query: "分数: 计算 2**80//3",
    },
    Example {
        icon: Icon::Terminal,
        title: "把分数舍入为指定小数位",
        query: "分数: 小数 1/3 to 8",
    },
];

fn search_key(text: &str) -> String {
    let narrowed: String = text.chars().map(narrow_ascii_graphic).collect();
    narrowed.to_lowercase()
}

pub(super) fn matching_examples(query: &str) -> Vec<usize> {
    let query = search_key(query.trim());
    if query.is_empty() {
        return (0..EXAMPLES.len()).collect();
    }
    static SEARCH_TEXT: OnceLock<Vec<(String, String)>> = OnceLock::new();
    let search_text = SEARCH_TEXT.get_or_init(|| {
        EXAMPLES
            .iter()
            .map(|example| (search_key(example.title), search_key(example.query)))
            .collect()
    });
    let mut matches: Vec<_> = search_text
        .iter()
        .enumerate()
        .filter_map(|(index, (title, example_query))| {
            if !query
                .split_whitespace()
                .all(|word| title.contains(word) || example_query.contains(word))
            {
                return None;
            }
            let command_rank = title_rank(example_query, &query);
            let rank = title_rank(title, &query).min(command_rank);
            Some((rank, command_rank, index))
        })
        .collect();
    matches.sort_unstable();
    matches.into_iter().map(|(_, _, index)| index).collect()
}

pub(super) fn tool_operation_examples(scope: Scope, term: &str) -> Option<Vec<usize>> {
    match scope {
        Scope::Json | Scope::TextTools | Scope::UrlEncoding | Scope::Percentage
            if !term.contains(char::is_whitespace) => {}
        Scope::Date
        | Scope::Duration
        | Scope::TransferTime
        | Scope::AspectRatio
        | Scope::Cidr
        | Scope::Timestamp
        | Scope::Color
        | Scope::ColorContrast
        | Scope::TextCount
        | Scope::Fraction
            if term.is_empty() => {}
        _ => return None,
    }
    let prefix = term.to_ascii_lowercase();
    let items: Vec<_> = EXAMPLES
        .iter()
        .enumerate()
        .filter_map(|(index, example)| {
            let (example_scope, input) = scoped_query(example.query)?;
            let operation = input.split_whitespace().next()?;
            (example_scope == scope && operation.to_ascii_lowercase().starts_with(&prefix))
                .then_some(index)
        })
        .collect();
    (!items.is_empty()).then_some(items)
}

pub(super) fn function_examples(query: &str) -> Option<Vec<usize>> {
    let query = search_key(query);
    let (name, exact) = query
        .strip_suffix('(')
        .map_or((query.as_str(), false), |name| (name, true));
    if name.len() < 2
        || !name.starts_with(|ch: char| ch.is_ascii_alphabetic())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    let mut matches: Vec<_> = EXAMPLES
        .iter()
        .enumerate()
        .filter_map(|(index, example)| {
            let (function, tail) = split_name(example.query);
            let name_matches = function == name || (!exact && function.starts_with(name));
            (tail.starts_with('(') && name_matches).then_some((function != name, index))
        })
        .collect();
    matches.sort_unstable();
    (!matches.is_empty()).then(|| matches.into_iter().map(|(_, index)| index).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_examples_for_empty_value_scopes_without_taking_over_the_input() {
        for scope in [
            Scope::Date,
            Scope::Duration,
            Scope::TransferTime,
            Scope::AspectRatio,
            Scope::Cidr,
            Scope::Timestamp,
            Scope::Color,
            Scope::ColorContrast,
            Scope::TextCount,
            Scope::Fraction,
        ] {
            let items = tool_operation_examples(scope, "").unwrap();
            assert!(items.into_iter().all(|index| {
                scoped_query(EXAMPLES[index].query)
                    .is_some_and(|(other, term)| other == scope && !term.is_empty())
            }));
            assert_eq!(tool_operation_examples(scope, "0"), None);
        }
        assert_eq!(tool_operation_examples(Scope::CalculationHistory, ""), None);
        assert_eq!(tool_operation_examples(Scope::Actions, ""), None);
    }
}
