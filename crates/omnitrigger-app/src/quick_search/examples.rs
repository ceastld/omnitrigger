use std::sync::OnceLock;

use crate::assets::Icon;

use super::calculator::narrow_ascii_graphic;
use super::scientific::split_name;
use super::scope::{parse_scope, scoped_query, tool_example_keyword, Scope};
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
        icon: Icon::Terminal,
        title: "按 a、b、c 解二次方程",
        query: "方程: 1, -3, 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "用根式表示二次方程的解",
        query: "方程: 1, -2, -1",
    },
    Example {
        icon: Icon::Terminal,
        title: "联立求解两个二元一次方程",
        query: "联立: 2,1,5; 1,-1,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "用最小二乘法拟合直线",
        query: "拟合: 0,1; 1,3; 2,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "用拟合直线预测给定 x 对应的 y",
        query: "拟合: 预测 10 @ 0,1; 1,3; 2,5",
    },
    Example {
        icon: Icon::Layers,
        title: "由三条边求三角形面积和周长",
        query: "三角形: 3,4,5",
    },
    Example {
        icon: Icon::Layers,
        title: "求一组数值的摘要统计",
        query: "统计: 1,3,5",
    },
    Example {
        icon: Icon::Layers,
        title: "计算总体方差和标准差",
        query: "统计: 总体 1,3,5",
    },
    Example {
        icon: Icon::Layers,
        title: "按已知频数汇总重复数值",
        query: "统计: 频数 1@2,3@1,5@2",
    },
    Example {
        icon: Icon::Layers,
        title: "求精确分位数",
        query: "统计: 分位数 25% 1/2,3/2",
    },
    Example {
        icon: Icon::Layers,
        title: "求四分位数和四分位距",
        query: "统计: 四分位 4,1,3,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求绝对偏差的中位数",
        query: "统计: 中位绝对偏差 0,1/2,1,10",
    },
    Example {
        icon: Icon::Layers,
        title: "按精确数值列出次数和频率",
        query: "统计: 频数表 3,1/2,0.5,3,5",
    },
    Example {
        icon: Icon::Layers,
        title: "找出出现次数最多的数值",
        query: "统计: 众数 3,1/2,0.5,3,5",
    },
    Example {
        icon: Icon::Layers,
        title: "用分数或百分数作为权重求平均值",
        query: "统计: 加权平均 80@30%,90@70%",
    },
    Example {
        icon: Icon::Terminal,
        title: "按固定窗口计算精确移动平均",
        query: "统计: 移动平均 2 1/2,1,2,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "按固定窗口计算精确移动中位数",
        query: "统计: 移动中位数 3 1/2,100,2,3,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐项计算精确累计合计",
        query: "统计: 累计合计 1/2,1,2,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐项计算精确累计平均",
        query: "统计: 累计平均 1/2,1,2,4",
    },
    Example {
        icon: Icon::Layers,
        title: "保留分数和根式的均方根",
        query: "统计: 均方根 1/2,3/2",
    },
    Example {
        icon: Icon::Layers,
        title: "精确计算两组数值的样本协方差",
        query: "统计: 协方差 0,0; 1,1; 2,1",
    },
    Example {
        icon: Icon::Layers,
        title: "精确计算两组数值的总体协方差",
        query: "统计: 总体协方差 0,0; 1,1; 2,1",
    },
    Example {
        icon: Icon::Layers,
        title: "用精确根式表示皮尔逊相关系数",
        query: "统计: 相关系数 0,0; 1,-1; 2,-1",
    },
    Example {
        icon: Icon::Layers,
        title: "用半径计算圆周长和面积",
        query: "圆: 半径 3",
    },
    Example {
        icon: Icon::Layers,
        title: "用直径和圆心角计算扇形弧长与面积",
        query: "圆: 直径 10 @ 90°",
    },
    Example {
        icon: Icon::Layers,
        title: "计算两个向量的点积",
        query: "向量: 点积 1,2,3; 4,5,6",
    },
    Example {
        icon: Icon::Layers,
        title: "计算三维向量的叉积",
        query: "向量: 叉积 1/2,0,0; 0,1/3,0",
    },
    Example {
        icon: Icon::Layers,
        title: "计算向量的长度",
        query: "向量: 长度 3/4,1",
    },
    Example {
        icon: Icon::Layers,
        title: "计算两个点的距离",
        query: "向量: 距离 0,0; 1,1",
    },
    Example {
        icon: Icon::Layers,
        title: "把第一个向量投影到第二个向量上",
        query: "向量: 投影 2,1; 1,1",
    },
    Example {
        icon: Icon::Layers,
        title: "将向量分解成平行和垂直分量",
        query: "向量: 分解 2,1; 1,1",
    },
    Example {
        icon: Icon::Layers,
        title: "求两个向量的精确夹角余弦",
        query: "向量: 夹角余弦 1,1; -1,0",
    },
    Example {
        icon: Icon::Layers,
        title: "计算方阵的行列式",
        query: "矩阵: 行列式 0,2; 3,4",
    },
    Example {
        icon: Icon::Layers,
        title: "计算两个矩阵的乘积",
        query: "矩阵: 乘法 1,2; 3,4 @ 5,6; 7,8",
    },
    Example {
        icon: Icon::Layers,
        title: "交换矩阵的行和列",
        query: "矩阵: 转置 1,2,3; 4,5,6",
    },
    Example {
        icon: Icon::Layers,
        title: "求线性方程组的唯一解",
        query: "矩阵: 求解 2,1; 1,-1 @ 5; 1",
    },
    Example {
        icon: Icon::Layers,
        title: "计算方阵的逆矩阵",
        query: "矩阵: 求逆 1,2; 3,4",
    },
    Example {
        icon: Icon::Layers,
        title: "精确计算矩阵的秩",
        query: "矩阵: 秩 1,2,3; 2,4,6",
    },
    Example {
        icon: Icon::Layers,
        title: "用精确消元求矩阵的行最简形",
        query: "矩阵: 行最简形 0,2,3; 1,4,5; 2,8,10",
    },
    Example {
        icon: Icon::Layers,
        title: "计算方阵主对角线的和",
        query: "矩阵: 迹 1/2,20; 3,-1/3",
    },
    Example {
        icon: Icon::Layers,
        title: "用同一个分数乘矩阵中的每个元素",
        query: "矩阵: 数乘 -3/2 @ 1/3,2; -1,0",
    },
    Example {
        icon: Icon::Layers,
        title: "按元素精确相加两个矩阵",
        query: "矩阵: 加法 1/2,2; 3,4 @ 1/3,5; -3,1",
    },
    Example {
        icon: Icon::Layers,
        title: "按元素精确相减两个矩阵",
        query: "矩阵: 减法 1/2,2; 3,4 @ 1/3,5; -3,1",
    },
    Example {
        icon: Icon::Layers,
        title: "换算单位",
        query: "10 km to mi",
    },
    Example {
        icon: Icon::Layers,
        title: "把多条单位和进制换算合成一列结果",
        query: "批量换算: 3 m to cm\n0xff to dec\n1 GiB to MiB",
    },
    Example {
        icon: Icon::Clock,
        title: "逐行换算分秒、ISO 和带单位的时长",
        query: "批量换算: 时长 01:30 to s\nPT1M30S to min\n1天2.5秒 to iso",
    },
    Example {
        icon: Icon::Clock,
        title: "逐行换算时间戳和带时区的日期时间",
        query: "批量换算: 时间戳 1700000000 to Asia/Shanghai\n1970-01-01T08:00:00+08:00",
    },
    Example {
        icon: Icon::Layers,
        title: "逐行提取文件名、扩展名和转换文件网址",
        query: "批量换算: 路径 文件名 \"C:\\工作\\报告 集.txt\"\n转文件网址 C:\\docs\\report 2026.txt\n扩展名 C:\\资料\\图.png",
    },
    Example {
        icon: Icon::Browser,
        title: "逐行处理网址、编码和查询参数",
        query: "批量换算: 网址 去查询串 https://example.invalid/doc?q=a#part\n表单解码 a%2Bb+c\n源 https://example.invalid:8443/doc",
    },
    Example {
        icon: Icon::Clock,
        title: "逐行计算日期、工作日和周号",
        query: "批量换算: 日期 2026-11-20 + 1工作日\n2024-02-29 + 1年\n2027-01-01 第几周",
    },
    Example {
        icon: Icon::Terminal,
        title: "一次算出多行算式",
        query: "批量计算: 2**8\n3^3\nsqrt(81)",
    },
    Example {
        icon: Icon::Terminal,
        title: "一次计算多行精确分数算式",
        query: "批量计算: 分数 1/3 + 1/6\n2**80",
    },
    Example {
        icon: Icon::Grid,
        title: "切换已打开的窗口",
        query: "窗口:",
    },
    Example {
        icon: Icon::Grid,
        title: "打开 Windows 的蓝牙设置",
        query: "系统设置: 蓝牙",
    },
    Example {
        icon: Icon::Layers,
        title: "打开 Windows 常用文件夹",
        query: "文件夹:",
    },
    Example {
        icon: Icon::Layers,
        title: "按名称查找本地目录中的文件",
        query: "文件:",
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
        title: "在 2 至 36 进制之间精确换算",
        query: "进制: 2_147_483_647 从 10 到 36",
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
        title: "读取 ISO 日时长",
        query: "时长: P1DT2.5S",
    },
    Example {
        icon: Icon::Clock,
        title: "把时长转成 ISO 格式",
        query: "时长: 1天2.5秒 to iso",
    },
    Example {
        icon: Icon::Clock,
        title: "计算跨午夜的时长",
        query: "时长: 23:45 到 01:15",
    },
    Example {
        icon: Icon::Clock,
        title: "合计多个时长",
        query: "时长: 合计 01:25,02:40,00:55",
    },
    Example {
        icon: Icon::Clock,
        title: "求多个时长的平均值",
        query: "时长: 平均 01:25,02:40,00:55",
    },
    Example {
        icon: Icon::Clock,
        title: "从一组时长中取中位数",
        query: "时长: 中位数 01:25,02:40,00:55",
    },
    Example {
        icon: Icon::Clock,
        title: "从一组时长中找出最短值",
        query: "时长: 最短 01:25,02:40,00:55",
    },
    Example {
        icon: Icon::Clock,
        title: "从一组时长中找出最长值",
        query: "时长: 最长 01:25,02:40,00:55",
    },
    Example {
        icon: Icon::Clock,
        title: "求最长和最短时长的差",
        query: "时长: 极差 01:25,02:40,00:55",
    },
    Example {
        icon: Icon::Clock,
        title: "按实际时长从短到长排列",
        query: "时长: 排序 02:40,PT2M,00:55,02:00",
    },
    Example {
        icon: Icon::Clock,
        title: "按实际时长从长到短排列",
        query: "时长: 降序 02:40,PT2M,00:55,02:00",
    },
    Example {
        icon: Icon::Clock,
        title: "按帧率把视频帧数换成时长",
        query: "帧数: 125 帧 @ 25 fps",
    },
    Example {
        icon: Icon::Clock,
        title: "按分数帧率把时长换成帧数",
        query: "帧数: 10秒 @ 30000/1001 fps",
    },
    Example {
        icon: Icon::Clock,
        title: "把帧数换成丢帧时间码",
        query: "时间码: 1800 帧 @ 29.97 fps DF",
    },
    Example {
        icon: Icon::Clock,
        title: "把时间码换成从零开始的帧数",
        query: "时间码: 00:00:05:00 @ 25 fps",
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
        title: "按距离和用时计算配速",
        query: "配速: 10 km 用时 52:30",
    },
    Example {
        icon: Icon::Layers,
        title: "把速度换算为配速",
        query: "配速: 12 km/h",
    },
    Example {
        icon: Icon::Layers,
        title: "把配速换算为速度",
        query: "配速: 5:00 /km",
    },
    Example {
        icon: Icon::Layers,
        title: "换算美制和英制油耗",
        query: "油耗: 7.5 L/100 km",
    },
    Example {
        icon: Icon::Layers,
        title: "换算每百公里耗油量",
        query: "油耗: 32 mpg-us to L/100 km",
    },
    Example {
        icon: Icon::Layers,
        title: "把包装价格换算成每千克单价",
        query: "单价: 36 元 / 750 g to kg",
    },
    Example {
        icon: Icon::Layers,
        title: "把包装价格换算到固定分量",
        query: "单价: 37.5 元 / 750 g to 500 g",
    },
    Example {
        icon: Icon::Layers,
        title: "把人民币金额转成中文大写",
        query: "人民币: 1409.50",
    },
    Example {
        icon: Icon::Layers,
        title: "按电压和电阻求电流及功率",
        query: "电路: 5 V, 20 Ω",
    },
    Example {
        icon: Icon::Layers,
        title: "按电压和电流求电阻及功率",
        query: "电路: 12 V, 1.5 A",
    },
    Example {
        icon: Icon::Clock,
        title: "把频率换算为周期",
        query: "周期: 50 Hz to ms",
    },
    Example {
        icon: Icon::Clock,
        title: "把周期换算为频率",
        query: "周期: 2.5 ms to Hz",
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
        title: "计算屏幕像素密度和尺寸",
        query: "像素密度: 3840×2160 @ 27 in",
    },
    Example {
        icon: Icon::Layers,
        title: "查询标准纸张尺寸",
        query: "纸张: B5",
    },
    Example {
        icon: Icon::Layers,
        title: "计算纸张对应的图像像素",
        query: "纸张: A4 @ 300 dpi",
    },
    Example {
        icon: Icon::Layers,
        title: "计算等比缩放到目标纸张的比例",
        query: "纸张: A4 to A5",
    },
    Example {
        icon: Icon::Layers,
        title: "生成鼠标触发条件的矩形坐标",
        query: "区域: -1920, 50 @ 100×200",
    },
    Example {
        icon: Icon::Layers,
        title: "计算矩形的宽高和中心",
        query: "区域: -1920, 50, -1820, 250",
    },
    Example {
        icon: Icon::Layers,
        title: "按物理像素偏移移动鼠标区域",
        query: "区域: -1920, 50, -1820, 250 移动 50, -10",
    },
    Example {
        icon: Icon::Layers,
        title: "求两个矩形重叠的鼠标区域",
        query: "区域: -20, 0, 20, 30 交集 -10, -5, 30, 15",
    },
    Example {
        icon: Icon::Terminal,
        title: "把八进制权限换算为 rwx",
        query: "权限: 4755",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 rwx 权限换算为八进制",
        query: "权限: -rw-r-----",
    },
    Example {
        icon: Icon::Terminal,
        title: "按 umask 计算文件权限位",
        query: "权限: 0666 umask 0022",
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
        icon: Icon::Terminal,
        title: "求覆盖地址区间的最小网段",
        query: "网段: 覆盖 192.0.2.1 至 192.0.2.6",
    },
    Example {
        icon: Icon::Terminal,
        title: "用 CIDR 网段精确表示地址区间",
        query: "网段: 分解 192.0.2.1 至 192.0.2.6",
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
        icon: Icon::Terminal,
        title: "统一 UUID 的大小写和连字符",
        query: "UUID: 格式 {550E8400-E29B-41D4-A716-446655440000}",
    },
    Example {
        icon: Icon::Terminal,
        title: "去掉 UUID 的连字符",
        query: "UUID: 紧凑 550e8400-e29b-41d4-a716-446655440000",
    },
    Example {
        icon: Icon::Terminal,
        title: "生成 UUID 的 URN 表示",
        query: "UUID: URN 550e8400-e29b-41d4-a716-446655440000",
    },
    Example {
        icon: Icon::Clock,
        title: "读取 UUID 中的 UTC 时间",
        query: "UUID: 时间 017f22e2-79b0-7cc3-98c4-dc0c0c07398f",
    },
    Example {
        icon: Icon::Terminal,
        title: "校验 ULID 并统一为大写",
        query: "ULID: 格式 01arz3ndektsv4rrffq69g5fav",
    },
    Example {
        icon: Icon::Clock,
        title: "读取 ULID 中的 UTC 时间",
        query: "ULID: 时间 01ARZ3NDEKTSV4RRFFQ69G5FAV",
    },
    Example {
        icon: Icon::Clock,
        title: "计算指定时长后的时间",
        query: "时间戳: 现在 + 90分钟 to UTC+8",
    },
    Example {
        icon: Icon::Clock,
        title: "计算跨夏令时的实际间隔",
        query:
            "时间戳: 间隔 2026-03-08 01:30 America/New_York 到 2026-03-08 03:30 America/New_York",
    },
    Example {
        icon: Icon::Clock,
        title: "把日期时间间隔转成 ISO 格式",
        query: "时间戳: 间隔 1970-01-01T00:00:00Z 到 1970-01-02T00:00:02.5Z to iso",
    },
    Example {
        icon: Icon::Clock,
        title: "求两个 Unix 时间戳之间的间隔",
        query: "时间戳: 间隔 -1ns 到 1ns to ns",
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
        title: "按地区时区换算时间",
        query: "时间戳: 1700000000 to America/New_York",
    },
    Example {
        icon: Icon::Clock,
        title: "输入当地时间和时区",
        query: "时间戳: 1970-01-01 08:00 Asia/Shanghai",
    },
    Example {
        icon: Icon::Clock,
        title: "指定秒级时区偏移",
        query: "时间戳: 0 to +00:09:21",
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
        title: "按自定义格式显示日期",
        query: r#"日期: 格式化 ["2026-11-21","%Y%m%d"]"#,
    },
    Example {
        icon: Icon::Clock,
        title: "按自定义格式读取日期",
        query: r#"日期: 解析 ["21/11/2026","%d/%m/%Y"]"#,
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
        title: "按周一开头查看整月日历",
        query: "日期: 月历 2024-02",
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
        title: "查询下周五的日期",
        query: "日期: 下周五",
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
        title: "列出日期区间内的所有周一",
        query: "日期: 列出 2026-11-01 到 2026-11-30 每周一",
    },
    Example {
        icon: Icon::Clock,
        title: "列出给定区间内的多种星期",
        query: "日期: 列出 2026-11-02 到 2026-11-08 每周一、三、五",
    },
    Example {
        icon: Icon::Clock,
        title: "从开始日期每隔几天列出日期",
        query: "日期: 列出 2024-02-27 到 2024-03-06 每隔 3 天",
    },
    Example {
        icon: Icon::Clock,
        title: "按固定月份间隔列出日期",
        query: "日期: 列出 2024-01-31 到 2024-05-31 每隔 1 个月",
    },
    Example {
        icon: Icon::Clock,
        title: "列出日期区间内的工作日",
        query: "日期: 列出 2026-11-20 到 2026-11-23 工作日",
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
        icon: Icon::Layers,
        title: "按配比分配总量",
        query: "配比: 120 @ 2:3:5",
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
        icon: Icon::Layers,
        title: "按比例混合两个颜色",
        query: "颜色: color-mix(in srgb, #fff 25%, #000)",
    },
    Example {
        icon: Icon::Layers,
        title: "选对比度更高的文字色",
        query: "颜色: 文字色 #3A86FF",
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
        title: "转为短横线命名",
        query: "文本: 短横线命名 HTTPServer2URL",
    },
    Example {
        icon: Icon::Terminal,
        title: "转为大驼峰命名",
        query: "文本: 大驼峰命名 http-server2-url",
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
        title: "去掉多行共有的缩进",
        query: "文本: 去公共缩进 \"    任务\n      子任务\n    完成\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "按数字顺序排列文件名",
        query: "文本: 自然排序 照片10\n照片2\n照片1",
    },
    Example {
        icon: Icon::Terminal,
        title: "按精确数值从小到大排列文本各行",
        query: "文本: 数值排序 2e0\n-1/2\n1.50\n3/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按精确数值从大到小排列文本各行",
        query: "文本: 数值降序 2e0\n-1/2\n1.50\n3/2",
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
        title: "用正则表达式匹配多种分隔符并拆分成行",
        query: r#"文本: 正则拆分成行 "[,;\\s]+" 苹果, 梨;茶  咖啡"#,
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
        title: "按行号取出文本中的几行",
        query: "文本: 取行 2,4-5 标题\n苹果\n略过\n梨\n茶",
    },
    Example {
        icon: Icon::Terminal,
        title: "给每行加前缀",
        query: "文本: 逐行加前缀 \"- \" 苹果\n梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "给每行加后缀",
        query: "文本: 逐行加后缀 \"。\" 苹果\n梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "去掉每行开头匹配的文字",
        query: "文本: 逐行去前缀 \"- \" - 苹果\n梨\n- - 茶",
    },
    Example {
        icon: Icon::Terminal,
        title: "去掉每行末尾匹配的文字",
        query: "文本: 逐行去后缀 \"。\" 苹果。\n梨\n茶。。",
    },
    Example {
        icon: Icon::Terminal,
        title: "找出两组文本共有的行",
        query: r#"文本: 共有行 "苹果\n梨\n茶" "茶\n苹果\n柿子""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "找出仅在第一组的行",
        query: r#"文本: 行差集 "苹果\n梨\n茶" "茶\n苹果\n柿子""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "合并两组文本的行并去重",
        query: r#"文本: 行并集 "苹果\n梨\n茶" "茶\n苹果\n柿子""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "找出只出现在其中一组的行",
        query: r#"文本: 行对称差 "苹果\n梨\n茶" "茶\n苹果\n柿子""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按顺序比较两段文本的新增与删除行",
        query: r#"文本: 逐行比较 "苹果\n梨\n茶" "苹果\n柿子\n茶""#,
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
        title: "依次提取全文中的正则匹配",
        query: r#"文本: 正则提取全部 "[0-9]+" A01 B02 C03 D04 E05 F06 G07 H08 I09"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "提取正则表达式指定的捕获组",
        query: r#"文本: 正则取组 id "ID=(?<id>[0-9]+)" ID=001 ID=002"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把每次正则匹配的捕获组提取为 CSV",
        query: r#"文本: 正则取表 "(?<id>[0-9]+):([^;]+)" 001:茶,咖啡;002:梨"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "用正则表达式替换文本",
        query: r#"文本: 正则替换 "([0-9]{4})/([0-9]{2})/([0-9]{2})" "$1-$2-$3" 2026/11/21"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把文本转义为正则字面量",
        query: "文本: 正则转义 a+b[0]",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计正则匹配总次数",
        query: r#"文本: 正则计数 "[0-9]+" A12 B34 C"#,
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
        title: "转义 HTML 特殊字符",
        query: "文本: HTML转义 <b>茶 & 咖啡</b>",
    },
    Example {
        icon: Icon::Terminal,
        title: "还原常见 HTML 字符引用",
        query: "文本: HTML还原 &lt;b&gt;茶 &amp; 咖啡&lt;/b&gt;",
    },
    Example {
        icon: Icon::Terminal,
        title: "转义 Markdown 正文中的标点",
        query: "文本: Markdown转义 *茶* [咖啡]",
    },
    Example {
        icon: Icon::Terminal,
        title: "还原 Markdown 的反斜杠转义",
        query: r"文本: Markdown还原 \*茶\* \[咖啡\]",
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
        title: "从同一份 JSON 按多个指针提取字段",
        query: r#"JSON: 取多路径 ["/订单/0/商品","/订单/0/价格"] {"订单":[{"商品":"茶","价格":9007199254740993}]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "列出 JSON 对象的顶层键名",
        query: r#"JSON: 键名 {"城市":{"名称":"北京"},"数量":1}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "排序 JSON 对象中的键名",
        query: r#"JSON: 键排序 {"z":1.20,"a":{"y":2,"b":3}}"#,
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
        icon: Icon::Terminal,
        title: "校验 JSON Lines 并指出每个无效行",
        query: "JSON: 校验行 {\"编号\":1}\n错误文本\n{\"编号\":\nnull",
    },
    Example {
        icon: Icon::Terminal,
        title: "合并数组里的 JSON 对象",
        query: r#"JSON: 合并对象 [{"名称":"初稿","版本":1},{"版本":2}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按路径合并每项中的 JSON 子数组",
        query: r#"JSON: 合并子数组 /商品 [{"商品":["茶",9.90]},{"商品":[]},{"商品":["咖啡",2e2]}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "改名一个顶层 JSON 键",
        query: r#"JSON: 改键 ["旧","新"] {"旧":1,"名称":"北京"}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "删除指定的顶层 JSON 键",
        query: r#"JSON: 去键 ["调试","备注"] {"值":1,"调试":true,"备注":"草稿"}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "移除 JSONC 注释并格式化 JSON",
        query: "JSON: 去注释 {\n// 说明\n\"网址\":\"https://example.invalid/\",/* 金额 */\n\"值\":1.50\n}",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 JSON 对象列表转换为 CSV",
        query: r#"JSON: 转CSV [{"编号":"001","数量":1.20},{"编号":"002","备注":"甲,乙"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按 JSON 指针把嵌套字段导出为 CSV",
        query: r#"JSON: 路径转CSV ["/订单/商品","/订单/金额"] [{"订单":{"商品":"茶,咖啡","金额":1.50}},{"订单":{"商品":"梨"}}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定顺序选出 JSON 对象的字段",
        query: r#"JSON: 选键 ["编号","状态"] [{"状态":true,"编号":"001","备注":"茶"},{"编号":"002"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "从键名和下标生成 JSON 指针",
        query: r#"JSON: 生成指针 ["商品",0,"a/b"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把 JSON 指针解析为路径分段",
        query: "JSON: 解析指针 /商品/0/a~1b",
    },
    Example {
        icon: Icon::Terminal,
        title: "列出 JSON 标量与空容器的指针",
        query: r#"JSON: 列指针 {"订单":[{"a/b":1},{}],"~":null}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "用 JSON 指针作键展平数据",
        query: r#"JSON: 展平 {"订单":[{"a/b":9007199254740993},{}]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "保留存在指定路径的 JSON 数组项",
        query: r#"JSON: 有路径 /状态 [{"编号":"001","状态":null},{"编号":"002"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "保留缺少指定路径的 JSON 数组项",
        query: r#"JSON: 缺路径 /状态 [{"编号":"001","状态":null},{"编号":"002"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按路径上的标量值筛选 JSON 数组",
        query: r#"JSON: 筛选 /状态 "完成" [{"编号":"001","状态":"完成"},{"编号":"002","状态":"待办"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按路径上的精确数值筛选 JSON 数组",
        query: r#"JSON: 数值筛选 /价格 >= 1.5 [{"价格":1.50},{"价格":1.49},{"价格":2e0}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "排除路径上具有指定标量值的 JSON 项",
        query: r#"JSON: 排除 /状态 "完成" [{"编号":"001","状态":"完成"},{"编号":"002","状态":"待办"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定路径的值对 JSON 数组去重",
        query: r#"JSON: 按值去重 /状态 [{"编号":"001","状态":"完成"},{"编号":"002","状态":"待办"},{"编号":"003","状态":"完成"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定 JSON 路径上的值计数",
        query: r#"JSON: 分组计数 /状态 [{"状态":"完成"},{"状态":"待办"},{"状态":"完成"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定 JSON 路径分组保留完整记录",
        query: r#"JSON: 分组明细 /状态 [{"状态":"完成","金额":1.50},{"状态":"待办","金额":2e0},{"状态":"完成","金额":3}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按 JSON 路径分组计算精确合计与平均",
        query: r#"JSON: 分组统计 ["/地区","/金额"] [{"地区":"北","金额":1.50},{"地区":"南","金额":4},{"地区":"北","金额":2e0}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按路径上的数字升序排列 JSON 项",
        query: r#"JSON: 数值排序 /价格 [{"编号":"001","价格":12.50},{"编号":"002","价格":-2.75},{"编号":"003","价格":1.5e0}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按文件名的自然顺序排列 JSON 数组",
        query: r#"JSON: 文本排序 /文件名 [{"文件名":"文件10"},{"文件名":"文件2"},{"文件名":"文件1"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定字符串的自然顺序反向排列 JSON 数组",
        query: r#"JSON: 文本降序 /文件名 [{"文件名":"文件10"},{"文件名":"文件2"},{"文件名":"文件1"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按路径上的数字降序排列 JSON 项",
        query: r#"JSON: 数值降序 /价格 [{"编号":"001","价格":12.50},{"编号":"002","价格":-2.75},{"编号":"003","价格":1.5e0}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把 CSV 转成 JSON 行数组",
        query: "CSV: 转JSON 编号,名称\n001,\"茶,咖啡\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "检查 CSV 记录的列数",
        query: "CSV: 校验 编号,说明\n001,\"茶\n咖啡\"\n002,梨,额外\n003",
    },
    Example {
        icon: Icon::Terminal,
        title: "按 CSV 表头生成 JSON 对象",
        query: "CSV: 对象 编号,名称\n001,茶",
    },
    Example {
        icon: Icon::Terminal,
        title: "由 CSV 生成 Markdown 表格",
        query: "CSV: Markdown 编号,备注\n001,茶|咖啡",
    },
    Example {
        icon: Icon::Terminal,
        title: "交换 CSV 的行和列",
        query: "CSV: 转置 名称,数量\n茶,2\n咖啡,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 CSV 转成制表符分隔的表格",
        query: "CSV: 转TSV 编号,备注\n001,\"茶,咖啡\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "把制表符分隔的表格转成 CSV",
        query: "CSV: 从TSV 编号\t备注\n001\t茶,咖啡",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定顺序选取 CSV 的列",
        query: "CSV: 选列 3,1 编号,名称,备注\n001,茶,\"含,逗号\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "按行号和区间取 CSV 数据行",
        query: "CSV: 取行 2,4 编号,备注\n001,茶\n002,梨\n003,苹果\n004,柿子",
    },
    Example {
        icon: Icon::Terminal,
        title: "按表头名称和指定顺序选 CSV 列",
        query: "CSV: 选列名 [\"备注\",\"编号\"] 编号,名称,备注\n001,茶,\"含,逗号\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "按名称或列号省去指定 CSV 列",
        query: "CSV: 去列 [\"备注\",2] 编号,状态,备注,金额\n001,完成,原样,1.50\n002,待办,\"甲,乙\",2e0",
    },
    Example {
        icon: Icon::Terminal,
        title: "改名 CSV 的一个表头",
        query: "CSV: 改列名 [\"旧名\",\"备注\"] 编号,旧名\n001,\"茶,咖啡\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "替换指定 CSV 列的完整单元格值",
        query: "CSV: 替换值 [\"状态\",\"待办\",\"进行中\"] 编号,状态,备注\n001,待办,待办\n002,完成,原样",
    },
    Example {
        icon: Icon::Terminal,
        title: "用数量和单价算出新的一列",
        query:
            "CSV: 计算列 [\"金额\",\"数量\",\"*\",\"单价\"] 商品,数量,单价\n茶,3,2.50\n咖啡,2,1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确算式给 CSV 追加一列",
        query: "CSV: 公式列 [\"含费金额\",\"数量\",3,\"x*y+1/6\"] 商品,数量,单价\n茶,3,2.50\n梨,2,1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "删除 CSV 中内容重复的行",
        query: "CSV: 去重 编号,备注\n001,\"茶,咖啡\"\n\"001\",\"茶,咖啡\"\n002,梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定的 CSV 列去重并保留整行",
        query: "CSV: 按列去重 1,2 编号,区域,备注\n001,东,\"茶,咖啡\"\n001,东,第二条\n001,西,第三条",
    },
    Example {
        icon: Icon::Terminal,
        title: "按单元格的完整内容筛选 CSV 行",
        query: "CSV: 筛选 2 \"完成\" 编号,状态\n001,完成\n002,待办\n003,完成",
    },
    Example {
        icon: Icon::Terminal,
        title: "排除单元格内容相同的 CSV 行",
        query: "CSV: 排除 2 \"完成\" 编号,状态\n001,完成\n002,待办\n003,完成",
    },
    Example {
        icon: Icon::Terminal,
        title: "按第二列的正则匹配筛选 CSV 行",
        query: "CSV: 正则筛选 2 \"^咖啡\" 编号,备注\n001,\"咖啡,热\"\n002,冷茶\n003,咖啡常温",
    },
    Example {
        icon: Icon::Terminal,
        title: "排除第二列符合正则表达式的 CSV 行",
        query: "CSV: 正则排除 2 \"^咖啡\" 编号,备注\n001,\"咖啡,热\"\n002,冷茶\n003,咖啡常温",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定表头列的精确数值筛选 CSV 行",
        query: "CSV: 数值筛选 \"价格\" >= 3/2 编号,价格\n001,1.50\n002,1.49\n003,2e0",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定 CSV 列的组合计数",
        query: "CSV: 分组计数 2 编号,状态\n001,完成\n002,待办\n003,完成",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定列分组汇总另一列的数值",
        query: "CSV: 分组合计 2 3 编号,状态,金额\n001,完成,1/2\n002,待办,2.5e0\n003,完成,1.5",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定列分组求另一列的平均值",
        query: "CSV: 分组平均 [\"状态\"] \"金额\" 编号,状态,金额\n001,完成,1/2\n002,待办,2.5e0\n003,完成,2/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "按两个分类透视汇总 CSV 数值",
        query: "CSV: 透视合计 [\"地区\",\"状态\",\"金额\"] 地区,状态,金额\n北,完成,1/2\n南,待办,0\n北,完成,2/3\n南,完成,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定列自然升序排列 CSV 行",
        query: "CSV: 排序 2 编号,名称\n001,文件10\n002,文件2\n003,文件20",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定列自然降序排列 CSV 行",
        query: "CSV: 降序 2 编号,名称\n001,文件10\n002,文件2\n003,文件20",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定列数值升序排列 CSV 行",
        query: "CSV: 数值排序 2 编号,金额\n001,12.50\n002,-2.75\n003,3/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按命名列对 CSV 数值降序排列",
        query: "CSV: 数值降序 [\"金额\"] 编号,金额\n001,1.50\n002,3/2\n003,2e0",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定列数值降序排列 CSV 行",
        query: "CSV: 数值降序 2 编号,金额\n001,12.50\n002,-2.75\n003,3/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计 CSV 一列中的精确数值",
        query: "CSV: 列统计 \"数值\" 编号,数值\n001,1/2\n002,1.5\n003,2.5e0",
    },
    Example {
        icon: Icon::Terminal,
        title: "按总体方差统计 CSV 数值列",
        query: "CSV: 列总体 \"数值\" 编号,数值\n001,1/2\n002,1.5\n003,2.5e0",
    },
    Example {
        icon: Icon::Layers,
        title: "从路径提取文件名",
        query: r#"路径: 文件名 "C:\工作\发布 包.tar.gz""#,
    },
    Example {
        icon: Icon::Layers,
        title: "提取去掉扩展名的文件主名",
        query: r"路径: 主名 C:\工作\发布 包.tar.gz",
    },
    Example {
        icon: Icon::Layers,
        title: "提取最后一个扩展名",
        query: r"路径: 扩展名 C:\工作\发布 包.tar.gz",
    },
    Example {
        icon: Icon::Layers,
        title: "替换或删除路径末尾的扩展名",
        query: r#"路径: 换扩展名 ["C:\\工作\\发布 包.tar.gz",".zip"]"#,
    },
    Example {
        icon: Icon::Layers,
        title: "把目录和文件名拼成完整路径",
        query: r#"路径: 拼接 ["C:\\工作","报告 集","2026.txt"]"#,
    },
    Example {
        icon: Icon::Layers,
        title: "从基准目录计算到目标的相对路径",
        query: r#"路径: 相对路径 ["C:\\工作\\资料","C:\\工作\\发布\\2026.txt"]"#,
    },
    Example {
        icon: Icon::Layers,
        title: "查找多条绝对路径共同的最长前缀",
        query: r#"路径: 共同路径 ["C:\\工作\\发布\\A.txt","C:\\工作\\草稿\\B.txt"]"#,
    },
    Example {
        icon: Icon::Layers,
        title: "把绝对路径转换为可复制的文件网址",
        query: r"路径: 转文件网址 C:\docs\report 2026.txt",
    },
    Example {
        icon: Icon::Layers,
        title: "从文件网址还原绝对路径",
        query: "路径: 从文件网址 file:///C:/docs/report%202026.txt",
    },
    Example {
        icon: Icon::Layers,
        title: "提取上一级目录",
        query: r"路径: 上一级 C:\工作\发布 包.tar.gz",
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
        title: "同时移除多个 URL 参数",
        query: r#"URL: 去参数组 ["source","campaign"] https://example.invalid/doc?source=mail&q=a%20b&campaign=autumn#part"#,
    },
    Example {
        icon: Icon::Browser,
        title: "只保留指定的 URL 参数",
        query: r#"URL: 保留参数 ["q","page"] https://example.invalid/doc?source=mail&q=a%20b&page=2&q=a+b#part"#,
    },
    Example {
        icon: Icon::Browser,
        title: "将相对链接解析成完整 URL",
        query: r#"URL: 拼接 ["https://example.invalid/docs/api/","../guide?lang=zh#intro"]"#,
    },
    Example {
        icon: Icon::Browser,
        title: "根据基准网址生成相对链接",
        query: r#"URL: 相对 ["https://example.invalid/docs/api/","https://example.invalid/docs/guide?lang=zh#intro"]"#,
    },
    Example {
        icon: Icon::Browser,
        title: "给网址追加一个查询参数",
        query: r#"URL: 加参数 ["tag","茶 + 咖啡"] https://example.invalid/doc?tag=first#part"#,
    },
    Example {
        icon: Icon::Browser,
        title: "设置查询参数并合并同名项",
        query: r#"URL: 设参数 ["page","2"] https://example.invalid/doc?page=1&q=a%20b&page=3#part"#,
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
        title: "连续应用多个百分比变化",
        query: "百分比: 连续变化 100 @ +10%, -10%",
    },
    Example {
        icon: Icon::Terminal,
        title: "按折扣反推折前价",
        query: "百分比: 折前价 170 按 8.5折",
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
        title: "用科学计数法做精确运算",
        query: "分数: 计算 2.5e-3 * 4e3",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算阶乘",
        query: "分数: 计算 50!",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算组合数",
        query: "分数: 计算 comb(100,50)",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算排列数",
        query: "分数: 计算 perm(100,20)",
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
    Example {
        icon: Icon::Terminal,
        title: "展开精确分数的循环小数",
        query: "分数: 循环小数 1/7",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算模幂或乘法逆元",
        query: "分数: 计算 pow(3,-1,11)",
    },
    Example {
        icon: Icon::Terminal,
        title: "给同一算式批量代入不同的 x",
        query: "分数: 取值表 [-2,0,3/2] x**2+1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "求有理数的有限连分数",
        query: "分数: 连分数 355/113",
    },
    Example {
        icon: Icon::Terminal,
        title: "将有限连分数还原为精确分数",
        query: "分数: 从连分数 [3,7,16]",
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

pub(super) fn tool_index_examples(query: &str) -> Vec<usize> {
    static EXAMPLE_SCOPES: OnceLock<Vec<Option<Scope>>> = OnceLock::new();
    let scopes = EXAMPLE_SCOPES.get_or_init(|| {
        EXAMPLES
            .iter()
            .map(|example| {
                let (prefix, _) = example.query.split_once([':', '：'])?;
                tool_example_keyword(prefix)?;
                scoped_query(example.query).map(|(scope, _)| scope)
            })
            .collect()
    });
    if let Some(index) = parse_scope(&search_key(query)).and_then(|scope| {
        scopes
            .iter()
            .position(|&candidate| candidate == Some(scope))
    }) {
        return vec![index];
    }
    let mut seen = Vec::new();
    matching_examples(query)
        .into_iter()
        .filter(|&index| {
            let Some(scope) = scopes[index] else {
                return false;
            };
            if seen.contains(&scope) {
                return false;
            }
            seen.push(scope);
            true
        })
        .collect()
}

pub(super) fn tool_operation_examples(scope: Scope, term: &str) -> Option<Vec<usize>> {
    match scope {
        Scope::BatchCalculation if matches!(term, "分" | "分数") => {}
        Scope::Fraction
            if !term.is_empty()
                && ["循环小数", "取值表", "连分数", "从连分数"]
                    .into_iter()
                    .any(|operation| operation.starts_with(term)) => {}
        Scope::BatchConversion
            if matches!(
                term,
                "时" | "时长" | "时间" | "时间戳" | "路" | "路径" | "网" | "网址" | "日" | "日期"
            ) => {}
        Scope::Date
            if !term.is_empty()
                && ["列出", "格式化", "解析", "月历"]
                    .into_iter()
                    .any(|operation| operation.starts_with(term)) => {}
        Scope::Duration
            if !term.is_empty()
                && [
                    "合计",
                    "平均",
                    "中位数",
                    "最短",
                    "最长",
                    "极差",
                    "排序",
                    "降序",
                ]
                .into_iter()
                .any(|operation| operation.starts_with(term)) => {}
        Scope::Statistics
            if matches!(
                term,
                "频" | "频数"
                    | "频数表"
                    | "众"
                    | "众数"
                    | "分位"
                    | "分位数"
                    | "四分位"
                    | "中"
                    | "中位"
                    | "中位绝"
                    | "中位绝对"
                    | "中位绝对偏"
                    | "中位绝对偏差"
                    | "加权"
                    | "加权平均"
                    | "移"
                    | "移动"
                    | "移动平"
                    | "移动平均"
                    | "移动中"
                    | "移动中位"
                    | "移动中位数"
                    | "累"
                    | "累计"
                    | "累计合"
                    | "累计合计"
                    | "累计平"
                    | "累计平均"
                    | "均"
                    | "均方"
                    | "均方根"
                    | "协"
                    | "协方"
                    | "协方差"
                    | "总体协"
                    | "总体协方"
                    | "总体协方差"
                    | "相"
                    | "相关"
                    | "相关系"
                    | "相关系数"
            ) => {}
        Scope::Json
        | Scope::Csv
        | Scope::TextTools
        | Scope::UrlEncoding
        | Scope::Path
        | Scope::Percentage
        | Scope::Uuid
        | Scope::Ulid
        | Scope::Vector
        | Scope::Matrix
        | Scope::Circle
            if !term.contains(char::is_whitespace) => {}
        Scope::Date
        | Scope::Duration
        | Scope::BatchConversion
        | Scope::BatchCalculation
        | Scope::VideoFrames
        | Scope::Timecode
        | Scope::TransferTime
        | Scope::Pace
        | Scope::FuelEconomy
        | Scope::UnitPrice
        | Scope::Renminbi
        | Scope::Circuit
        | Scope::Period
        | Scope::AspectRatio
        | Scope::PixelDensity
        | Scope::Paper
        | Scope::Rectangle
        | Scope::Cidr
        | Scope::FileMode
        | Scope::Timestamp
        | Scope::Color
        | Scope::ColorContrast
        | Scope::TextCount
        | Scope::Fraction
        | Scope::Equation
        | Scope::LinearSystem
        | Scope::Radix
        | Scope::LinearFit
        | Scope::Statistics
        | Scope::Triangle
        | Scope::RatioAllocation
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
            Scope::VideoFrames,
            Scope::Timecode,
            Scope::TransferTime,
            Scope::Pace,
            Scope::FuelEconomy,
            Scope::UnitPrice,
            Scope::Renminbi,
            Scope::Circuit,
            Scope::Period,
            Scope::AspectRatio,
            Scope::PixelDensity,
            Scope::Paper,
            Scope::Rectangle,
            Scope::Cidr,
            Scope::FileMode,
            Scope::Timestamp,
            Scope::Color,
            Scope::ColorContrast,
            Scope::TextCount,
            Scope::Fraction,
            Scope::Equation,
            Scope::LinearSystem,
            Scope::LinearFit,
            Scope::Radix,
            Scope::Triangle,
            Scope::RatioAllocation,
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
