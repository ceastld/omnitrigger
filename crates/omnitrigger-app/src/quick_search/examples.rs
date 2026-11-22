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
        title: "解一元二次不等式并返回区间",
        query: "方程: 不等式 >= 1,-3,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求二次函数的顶点和极值",
        query: "方程: 顶点 2,-4,3",
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
        icon: Icon::Terminal,
        title: "用同一条拟合直线预测多个 x",
        query: "拟合: 预测 1/2,0,3 @ 0,1; 1,3; 2,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看拟合直线对每个观测值的残差",
        query: "拟合: 残差 0,0; 1,1; 2,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "从表格粘贴两列数据进行直线拟合",
        query: "拟合: 0\t1\n1\t3\n2\t5",
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
        icon: Icon::Terminal,
        title: "去掉两端的观测后求截尾平均",
        query: "统计: 截尾平均 25% 1,2,3,100",
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
        title: "按固定窗口计算精确移动合计",
        query: "统计: 移动合计 2 1/2,1,2,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "按固定窗口计算精确样本方差",
        query: "统计: 移动样本方差 2 1/2,1,2,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "按固定窗口计算精确总体方差",
        query: "统计: 移动总体方差 2 1/2,1,2,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "按固定窗口计算精确移动中位数",
        query: "统计: 移动中位数 3 1/2,100,2,3,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看相邻数值各窗口的最小值",
        query: "统计: 移动最小 3 1/2,100,2,3,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看相邻数值各窗口的最大值",
        query: "统计: 移动最大 3 1/2,100,2,3,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "求相邻数值每个窗口的最大值与最小值之差",
        query: "统计: 移动极差 3 1/2,100,2,3,-1",
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
        icon: Icon::Terminal,
        title: "按数值查看累计次数和频率",
        query: "统计: 累计分布 3,1/2,0.5,3,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "查询小于或等于指定值的次数",
        query: "统计: 累计次数 2 @ 1,2,2,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "查询小于或等于指定值的比例",
        query: "统计: 累计频率 1/2 @ 频数 0.50@3,3/2@1",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定边界分箱并导出精确次数和频率",
        query: "统计: 分箱 0,1/2,2 @ -1,0,0.50,1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算恰好成功指定次数的二项概率",
        query: "统计: 二项概率 4,2,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算成功次数不超过指定值的二项概率",
        query: "统计: 二项累计 4,2,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算不放回抽取的精确概率",
        query: "统计: 超几何概率 10,4,3,2",
    },
    Example {
        icon: Icon::Layers,
        title: "保留分数和根式的均方根",
        query: "统计: 均方根 1/2,3/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求两段等距离行程的平均速度",
        query: "统计: 调和平均 36,72",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算样本均值的标准误",
        query: "统计: 标准误 1,3,5",
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
        title: "计算带并列名次的斯皮尔曼秩相关",
        query: "统计: 秩相关 0,0; 1,1; 2,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "按原顺序计算相邻数值的差",
        query: "统计: 差分 3/2,1,5/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按原顺序计算相邻数值的变化率",
        query: "统计: 变化率 20,15,45",
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
        title: "把非零向量缩放为单位向量",
        query: "向量: 单位向量 3,4",
    },
    Example {
        icon: Icon::Layers,
        title: "计算两个点的距离",
        query: "向量: 距离 0,0; 1,1",
    },
    Example {
        icon: Icon::Layers,
        title: "计算点到线段的距离和最近点",
        query: "向量: 线段距离 3,4 @ 0,0; 2,0",
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
        icon: Icon::Terminal,
        title: "精确计算矩阵的整数次幂",
        query: "矩阵: 乘方 1,1; 1,0 @ 5",
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
        icon: Icon::Terminal,
        title: "计算矩阵零空间的维数和基",
        query: "矩阵: 零空间 0,2,3; 0,4,6",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算矩阵行空间的维数和基",
        query: "矩阵: 行空间 1,2,3; 2,4,6; 0,1,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "从原矩阵主元列得到列空间的基",
        query: "矩阵: 列空间 1,2,3; 2,4,6; 0,1,1",
    },
    Example {
        icon: Icon::Layers,
        title: "计算方阵主对角线的和",
        query: "矩阵: 迹 1/2,20; 3,-1/3",
    },
    Example {
        icon: Icon::Layers,
        title: "精确计算二维方阵的特征值",
        query: "矩阵: 特征值 2,1; 1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算两个复数的和",
        query: "复数: 加法 1,2 @ 3,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算两个复数的差",
        query: "复数: 减法 1,2 @ 3,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算两个复数的积",
        query: "复数: 乘法 1,2 @ 3,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算两个复数的商",
        query: "复数: 除法 1,2 @ 3,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算复数的模",
        query: "复数: 模 3,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "求复数的共轭",
        query: "复数: 共轭 -2,3/4",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算复数的整数次幂",
        query: "复数: 乘方 1,1 @ 5",
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
        title: "把两个同尺寸矩阵对应的元素相乘",
        query: "矩阵: 逐项乘法 1/2,2; 3,4 @ 3,1/3; 0,-1",
    },
    Example {
        icon: Icon::Layers,
        title: "把矩阵各元素平方求和后开根号",
        query: "矩阵: F范数 1/2,1/2",
    },
    Example {
        icon: Icon::Layers,
        title: "换算单位",
        query: "10 km to mi",
    },
    Example {
        icon: Icon::Layers,
        title: "把同一个数值换算成多个目标单位",
        query: "1 GiB to [MiB, KiB, Gib]",
    },
    Example {
        icon: Icon::Layers,
        title: "精确换算大整数字节数",
        query: "数据换算: 9007199254740993 B to KiB",
    },
    Example {
        icon: Icon::Layers,
        title: "把数据速率换算成精确分数",
        query: "数据换算: 1 MiB/s to MB/s",
    },
    Example {
        icon: Icon::Layers,
        title: "一次输出三种单位的精确数据量",
        query: "数据换算: 1/3 KiB to [B, b, MiB]",
    },
    Example {
        icon: Icon::Layers,
        title: "换算括号算式计算出的数据量",
        query: "(2**10)B to KiB",
    },
    Example {
        icon: Icon::Layers,
        title: "把多条单位和进制换算合成一列结果",
        query: "批量换算: 3 m to cm\n0xff to dec\n1 GiB to MiB",
    },
    Example {
        icon: Icon::Layers,
        title: "以 CSV 导出单位与进制换算的输入和结果",
        query: "批量换算: 对照 3 m to cm\n0xff to dec\n1 GiB to MiB",
    },
    Example {
        icon: Icon::Clock,
        title: "逐行换算分秒、ISO 和带单位的时长",
        query: "批量换算: 时长 01:30 to s\nPT1M30S to min\n1天2.5秒 to iso",
    },
    Example {
        icon: Icon::Clock,
        title: "逐行计算时长并导出输入与结果 CSV",
        query: "批量换算: 时长对照 扣除休息 23:45 到 01:15 | 30分钟\n08:00 到 10:15 to min",
    },
    Example {
        icon: Icon::Clock,
        title: "逐行换算时间戳和带时区的日期时间",
        query: "批量换算: 时间戳 1700000000 to Asia/Shanghai\n1970-01-01T08:00:00+08:00",
    },
    Example {
        icon: Icon::Clock,
        title: "以 CSV 导出时间戳换算的输入和结果",
        query: "批量换算: 时间戳对照 0 to UTC+8\n1970-01-01T08:00:00+08:00",
    },
    Example {
        icon: Icon::Layers,
        title: "逐行提取文件名、扩展名和转换文件网址",
        query: "批量换算: 路径 文件名 \"C:\\工作\\报告 集.txt\"\n转文件网址 C:\\docs\\report 2026.txt\n扩展名 C:\\资料\\图.png",
    },
    Example {
        icon: Icon::Layers,
        title: "以 CSV 导出路径操作的输入和结果",
        query: "批量换算: 路径对照 文件名 \"C:\\工作\\报告 集.txt\"\n转文件网址 C:\\docs\\report 2026.txt",
    },
    Example {
        icon: Icon::Browser,
        title: "逐行处理网址、编码和查询参数",
        query: "批量换算: 网址 去查询串 https://example.invalid/doc?q=a#part\n表单解码 a%2Bb+c\n源 https://example.invalid:8443/doc",
    },
    Example {
        icon: Icon::Browser,
        title: "以 CSV 导出网址操作的输入和结果",
        query: "批量换算: 网址对照 去查询串 https://example.invalid/doc?q=a#part\n表单解码 a%2Bb+c",
    },
    Example {
        icon: Icon::Terminal,
        title: "把多种颜色写法逐行换算成 HEX",
        query: "批量换算: 颜色 #abc\nrgb(255, 0, 0)\ntransparent",
    },
    Example {
        icon: Icon::Terminal,
        title: "以 CSV 对照颜色原文和 HEX",
        query: "批量换算: 颜色对照 rebeccapurple\nrgb(0, 255, 0)\nrgba(255,0,0,0.5)",
    },
    Example {
        icon: Icon::Clock,
        title: "逐行计算日期、工作日和周号",
        query: "批量换算: 日期 2026-11-20 + 1工作日\n2024-02-29 + 1年\n2027-01-01 第几周",
    },
    Example {
        icon: Icon::Clock,
        title: "逐行计算日期并导出输入与结果 CSV",
        query: "批量换算: 日期对照 2024-02-29 + 1年\n2027-01-01 第几周",
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
        icon: Icon::Terminal,
        title: "把算式和结果输出为两列 CSV",
        query: "批量计算: 对照 2**10\navg(1,3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确分数输出算式和结果对照",
        query: "批量计算: 分数对照 1/3 + 1/6\n2**80",
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
        title: "扣掉休息计算净时长",
        query: "时长: 扣除休息 23:45 到 01:15 | 20分钟 + 10分钟",
    },
    Example {
        icon: Icon::Clock,
        title: "计算两个时长的精确比率",
        query: "时长: 比率 45分钟 @ 2小时",
    },
    Example {
        icon: Icon::Clock,
        title: "按精确分数缩放时长",
        query: "时长: 缩放 01:30:00 @ 2/3",
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
        icon: Icon::Terminal,
        title: "按各组出现次数求平均时长",
        query: "时长: 频数平均 01:00@3,02:00@1",
    },
    Example {
        icon: Icon::Clock,
        title: "按各组出现次数求中位时长",
        query: "时长: 频数中位数 01:00@2,02:00@2",
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
        icon: Icon::Terminal,
        title: "计算串联电阻的等效电阻",
        query: "电路: 串联 220 Ω,330 Ω,1 kΩ",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算并联电阻的等效电阻",
        query: "电路: 并联 220 Ω,330 Ω",
    },
    Example {
        icon: Icon::Terminal,
        title: "按功率和用时计算耗电量",
        query: "电路: 耗电 1.5 kW @ 2h",
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
        icon: Icon::Terminal,
        title: "按指定前缀长度划分子网",
        query: "网段: 子网 192.0.2.137/26 到 /28",
    },
    Example {
        icon: Icon::Terminal,
        title: "从网段中排除指定子网",
        query: "网段: 排除 192.0.2.0/28, 192.0.2.1/32",
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
        title: "把时间戳向下对齐到固定时长的边界",
        query: "时间戳: 向下对齐 15分钟 @ 1970-01-01T00:17:30Z",
    },
    Example {
        icon: Icon::Clock,
        title: "把时间戳向上对齐到固定时长的边界",
        query: "时间戳: 向上对齐 15分钟 @ 1970-01-01T00:17:30Z",
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
        title: "把同一时刻换算到多个时区",
        query: "时间戳: 2026-07-01 09:00 America/New_York to UTC, Asia/Shanghai, America/Los_Angeles",
    },
    Example {
        icon: Icon::Clock,
        title: "把同一时刻的多个时区结果导出为 CSV",
        query: "时间戳: 时区CSV 2026-07-01 09:00 America/New_York to UTC, Asia/Shanghai, America/Los_Angeles",
    },
    Example {
        icon: Icon::Clock,
        title: "列出夏令时回拨时重复出现的两个时刻",
        query: "时间戳: 时区候选 2026-11-01 01:30 America/New_York",
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
        title: "考虑休息例外后找当月第三个工作日",
        query: r#"日期: 工作日序号 2026-11 @ 3 例外 [["2026-11-02"],[]]"#,
    },
    Example {
        icon: Icon::Clock,
        title: "查看当月最后一个工作日",
        query: "日期: 工作日序号 2026-11 @ -1",
    },
    Example {
        icon: Icon::Terminal,
        title: "将相邻或重叠的日期区间合并",
        query: "日期: 合并区间 2026-11-01 至 2026-11-04; 2026-11-05 至 2026-11-08",
    },
    Example {
        icon: Icon::Clock,
        title: "按休息日和补班日调整工作日加减",
        query: r#"日期: 2026-11-20 + 1工作日 例外 [["2026-11-23"],["2026-11-21"]]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "找出多个日期区间重叠的日期",
        query: "日期: 区间交集 2026-11-01 至 2026-11-15; 2026-11-10 至 2026-11-20",
    },
    Example {
        icon: Icon::Clock,
        title: "从日期区间扣除多个占用时段",
        query: "日期: 扣除区间 2024-02-26 至 2024-03-05; 2024-02-28 至 2024-03-01; 2024-03-04 至 2024-03-07",
    },
    Example {
        icon: Icon::Clock,
        title: "按自然月拆分日期区间并计算各段天数",
        query: "日期: 按月拆分 2024-01-31 至 2024-03-02",
    },
    Example {
        icon: Icon::Clock,
        title: "按月输出日期区间天数和工作日 CSV",
        query: "日期: 按月拆分CSV 2024-01-31 至 2024-03-02",
    },
    Example {
        icon: Icon::Clock,
        title: "按周输出日期区间天数和工作日 CSV",
        query: "日期: 按周拆分CSV 2026-12-30 至 2027-01-05",
    },
    Example {
        icon: Icon::Terminal,
        title: "按自然季度导出日期片段、天数和工作日",
        query: "日期: 按季度拆分CSV 2024-03-30 至 2024-04-02",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐日导出星期和工作日标记",
        query: r#"日期: 逐日CSV 2024-02-29 至 2024-03-03 例外 [["2024-03-01"],["2024-03-02"]]"#,
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
        icon: Icon::Terminal,
        title: "查询下个自然季度的范围和工作日",
        query: "日期: 下季度",
    },
    Example {
        icon: Icon::Terminal,
        title: "以本月第一天为起点计算",
        query: "日期: 本月初 + 1天",
    },
    Example {
        icon: Icon::Clock,
        title: "以去年最后一天为起点计算",
        query: "日期: 去年底 + 1天",
    },
    Example {
        icon: Icon::Clock,
        title: "用季度首尾指定完整的日期范围",
        query: "日期: 按季度拆分CSV 本季度初至本季度末",
    },
    Example {
        icon: Icon::Terminal,
        title: "用月初和月底指定完整的日期范围",
        query: "日期: 按月拆分CSV 本月初至本月底",
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
        title: "按固定工作日间隔列出日期",
        query: "日期: 列出 2026-11-21 到 2026-12-01 每隔 3 个工作日",
    },
    Example {
        icon: Icon::Terminal,
        title: "列工作日时加入手动指定的休息日和补班日",
        query: r#"日期: 列出 2026-11-25 到 2026-11-30 工作日 例外 [["2026-11-26"],["2026-11-28"]]"#,
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
        title: "把每分钟转数换成每秒角度",
        query: "120 rpm to °/s",
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
        icon: Icon::Terminal,
        title: "按 sRGB 等距离生成两种颜色之间的色阶",
        query: "颜色: 色阶 #004080 到 #80C0FF @ 5",
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
        title: "找回用 Ctrl+D 收藏的查询",
        query: "收藏:",
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
        title: "按字素簇、码位和编码分别统计文本长度",
        query: "文本: 长度详情 \"👩‍💻 e\u{0301}\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "显示普通空格、制表符和不换行空格",
        query: "文本: 显示空白 \" 茶\t椅\u{00A0}咖啡 \"",
    },
    Example {
        icon: Icon::Terminal,
        title: "移除终端日志中的 ANSI 颜色和样式",
        query: "文本: 去ANSI样式 \x1b[31m错误\x1b[0m \x1b[1m完成\x1b[m",
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
        title: "按每行的字符数从少到多排序",
        query: "文本: 按长度排序 海边\n月\n青苹果",
    },
    Example {
        icon: Icon::Terminal,
        title: "按每行的字符数从多到少排序",
        query: "文本: 按长度降序 海边\n月\n青苹果",
    },
    Example {
        icon: Icon::Terminal,
        title: "从最后一行到第一行输出文本",
        query: "文本: 按行倒序 甲\n乙\n丙",
    },
    Example {
        icon: Icon::Terminal,
        title: "清理每行首尾空白",
        query: "文本: 逐行去空白 \"  苹果  \n 梨  \"",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计 CRLF、LF 和 CR 换行符",
        query: "文本: 换行统计 \"甲\r\n乙\r丙\n\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "把混合换行统一转成 LF",
        query: "文本: 转换换行 LF \"甲\r\n乙\r丙\n\"",
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
        title: "连续空行只保留一行",
        query: "文本: 合并空行 甲\n\n\n乙",
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
        title: "按行号删除指定文本行",
        query: "文本: 删行 1,3-4 标题\n苹果\n草稿\n注释\n梨",
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
        title: "查看 UTF-16 十六进制码元",
        query: "文本: UTF16编码 北😀",
    },
    Example {
        icon: Icon::Terminal,
        title: "将 UTF-16 十六进制码元转回文字",
        query: "文本: UTF16解码 5317 D83D DE00",
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
        title: "按指针替换 JSON 中已有的值",
        query: r#"JSON: 改值 /订单/0/金额 3.50 {"订单":[{"金额":1.00,"币种":"CNY"}],"税率":0.10}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按 JSON 路径新增字段或插入数组项",
        query: r#"JSON: 加路径 /订单/1 {"金额":3.50} {"订单":[{"金额":1.00}, {"金额":2e0}],"税率":0.10}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把原始 JSON 值复制到另一条路径",
        query: r#"JSON: 复制路径 ["/订单/0","/归档/-"] {"订单":[{"金额":1.50,"名字":"\u8336"}],"归档":[]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按两个 JSON 路径移动值",
        query: r#"JSON: 移动路径 ["/订单/0","/归档/-"] {"订单":[{"金额":1.50,"名字":"\u8336"}],"归档":[]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按顺序应用一组 JSON 补丁",
        query: r#"JSON: 应用补丁 [{"op":"test","path":"/状态","value":"初稿"},{"op":"replace","path":"/状态","value":"完成"}] {"版本":1.00,"状态":"初稿"}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "依次整理 JSON 字段并生成 CSV",
        query: r#"JSON: 依次处理 ["补字段 {\"币种\":\"CNY\"}","选键 [\"编号\",\"金额\",\"币种\"]","转CSV"] [{"编号":"001","金额":1.50},{"编号":"002","金额":2e0,"币种":"USD"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按路径对比两份 JSON 的变化",
        query: r#"JSON: 对比 {"a/b":[1.00,null],"旧":2e0} {"a/b":[1,true],"新":null}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "删除 JSON 深处的成员或数组项",
        query: r#"JSON: 删路径 /订单/0/备注 {"订单":[{"金额":1.50,"备注":"草稿"}],"税率":0.10}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "删除指定的顶层 JSON 键",
        query: r#"JSON: 去键 ["调试","备注"] {"值":1,"调试":true,"备注":"草稿"}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "移除对象中值为 null 的顶层字段",
        query: r#"JSON: 去空字段 [{"编号":"001","备注":null,"金额":1.50},{"编号":"002","备注":""}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "给缺少指定键的 JSON 对象补上默认值",
        query: r#"JSON: 补字段 ["金额",1.50] [{"编号":"001"},{"编号":"002","金额":null}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "一次补上多个缺少的 JSON 字段",
        query: r#"JSON: 补字段 {"币种":"CNY","金额":1.50} [{"编号":"001"},{"编号":"002","币种":"USD"}]"#,
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
        title: "同时按状态和金额筛选 JSON 数组",
        query: r#"JSON: 多条件筛选 [["/状态","=","完成"],["/金额",">=",1.5]] [{"状态":"完成","金额":1.50},{"状态":"待办","金额":2},{"状态":"完成","金额":1.49}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按任一条件筛选 JSON 数组",
        query: r#"JSON: 任一条件筛选 [["/状态","=","完成"],["/金额",">=",1.5]] [{"状态":"完成"},{"状态":"待办","金额":2e0},{"状态":"待办","金额":1.49}]"#,
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
        title: "按指定 JSON 路径计算各组中位数",
        query: r#"JSON: 分组中位数 ["/地区","/金额"] [{"地区":"北","金额":1.50},{"地区":"南","金额":4},{"地区":"北","金额":2e0}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按路径上的数字升序排列 JSON 项",
        query: r#"JSON: 数值排序 /价格 [{"编号":"001","价格":12.50},{"编号":"002","价格":-2.75},{"编号":"003","价格":1.5e0}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "先按分组文本再按金额排列 JSON",
        query: r#"JSON: 多路径排序 [["/组","文本升序"],["/金额","数值降序"]] [{"组":"文件10","金额":1},{"组":"文件2","金额":1.50},{"组":"文件2","金额":2e0}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按多个路径对 JSON 记录去重",
        query: r#"JSON: 多路径去重 ["/地区","/月份"] [{"地区":"北","月份":1.00,"编号":"001"},{"地区":"南","月份":1},{"地区":"北","月份":1e0,"编号":"002"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "找出多个路径值组合重复的全部 JSON 项",
        query: r#"JSON: 多路径查重复 ["/地区","/月份"] [{"地区":"北","月份":1.00,"编号":"001"},{"地区":"南","月份":1},{"地区":"北","月份":1e0,"编号":"002"}]"#,
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
        title: "定位 CSV 语法错误的记录和列",
        query: "CSV: 校验 编号,说明\n001,\"茶\n咖啡\"\n002,\"未闭合",
    },
    Example {
        icon: Icon::Terminal,
        title: "依次去掉空行、删除指定记录并选择列",
        query: "CSV: 依次处理 [\"去空行\",\"删行 2\",\"选列 2,1\"] 编号,商品\n001,茶\n\n002,梨\n003,咖啡",
    },
    Example {
        icon: Icon::Terminal,
        title: "用空单元格补齐 CSV 各条记录的列数",
        query: "CSV: 补齐列 编号,说明\n001,茶,1.50\n002",
    },
    Example {
        icon: Icon::Terminal,
        title: "按 CSV 表头生成 JSON 对象",
        query: "CSV: 对象 编号,名称\n001,茶",
    },
    Example {
        icon: Icon::Terminal,
        title: "把选中的 CSV 列转换成 JSON 数字",
        query: "CSV: 数值对象 [\"金额\",3] 编号,金额,数量\n001,1.50,9007199254740993\n002,,2e0",
    },
    Example {
        icon: Icon::Terminal,
        title: "由 CSV 生成 Markdown 表格",
        query: "CSV: Markdown 编号,备注\n001,茶|咖啡",
    },
    Example {
        icon: Icon::Terminal,
        title: "给 Markdown 表格的每一列指定对齐方式",
        query: "CSV: Markdown对齐 [\"左\",\"中\",\"右\"] 编号,商品,数量\n001,茶|咖啡,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 Markdown 表格转成 CSV",
        query: "CSV: 从Markdown | 编号 | 商品 | 备注 |\n| :--- | :---: | ---: |\n| 001 | 茶\\|咖啡 | 上<br>下 |\n| 002 | 米 |",
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
        title: "按行号删除指定 CSV 数据行",
        query: "CSV: 删行 2,4 编号,备注\n001,茶\n002,梨\n003,苹果\n004,柿子",
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
        title: "同时重命名多列并可交换列名",
        query: "CSV: 批量改列名 [[\"代码\",\"简称\"],[\"简称\",\"代码\"]] 代码,简称\n001,花茶",
    },
    Example {
        icon: Icon::Terminal,
        title: "替换指定 CSV 列的完整单元格值",
        query: "CSV: 替换值 [\"状态\",\"待办\",\"进行中\"] 编号,状态,备注\n001,待办,待办\n002,完成,原样",
    },
    Example {
        icon: Icon::Terminal,
        title: "按多组对应关系整理 CSV 单元格",
        query: "CSV: 映射值 [\"状态\",[[\"P\",\"待办\"],[\"D\",\"完成\"],[\"\",\"未填\"]]] 编号,状态\n001,P\n002,D\n003,",
    },
    Example {
        icon: Icon::Terminal,
        title: "清理 CSV 指定列的首尾空白",
        query: "CSV: 去空白 [\"名称\",2] 名称,备注,编号\n\" 茶 \",\"  热  \",001\n\" 梨 \",\" 冷 饮 \",002",
    },
    Example {
        icon: Icon::Terminal,
        title: "向下填充 CSV 指定列的空单元格",
        query: "CSV: 向下填充 [\"地区\",2] 地区,月份,编号\n华北,1月,001\n,,002\n华南,,003",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计 CSV 各列的空单元格",
        query: "CSV: 空值统计 [\"备注\",1] 编号,备注\n001,\n,\" \"\n002,茶",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位数值列里的文本或空单元格",
        query: "CSV: 检查数值列 [\"金额\"] 编号,金额\n001,1.50\n002,待确认\n003,\n004,1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "把选中的 CSV 数值列统一成固定小数位",
        query: "CSV: 小数列 [[\"金额\",3],2] 编号,金额,比率\n001,2.675,1/8\n002,,-0.005",
    },
    Example {
        icon: Icon::Terminal,
        title: "移除所有单元格都为空的 CSV 数据行",
        query: "CSV: 去空行 编号,备注\n001,茶\n\n,\n002,梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "为 CSV 每行追加固定文本列",
        query: "CSV: 常量列 [\"来源\",\"人工,复核\"] 编号,商品\n001,茶\n002,梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "给 CSV 数据行追加连续行号",
        query: "CSV: 加行号 [\"序号\",100] 编号,商品\n001,茶\n002,梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "把所选 CSV 单元格拼成一列",
        query: "CSV: 拼接列 [\"地址\",[\"省\",3],\"-\"] 编号,省,市\n001,福建,福州\n002,浙江,杭州",
    },
    Example {
        icon: Icon::Terminal,
        title: "把每行按顺序找到的第一个非空值写入新列",
        query: "CSV: 首个非空 [\"联系\",[\"手机\",\"座机\"]] 编号,手机,座机\n001,,021-1000\n002,010-2000,021-2000\n003,,",
    },
    Example {
        icon: Icon::Terminal,
        title: "按分隔符把一列拆到多个新列",
        query: "CSV: 拆分列 [\"地址\",[\"省\",\"市和区\"],\"/\"] 编号,地址\n001,福建/福州/鼓楼\n002,浙江",
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
        title: "找出 CSV 中有相同列值组合的记录",
        query: "CSV: 查重复 [\"编号\",2] 编号,地区,备注\n001,北,首条\n001,南,独有\n001,北,再次",
    },
    Example {
        icon: Icon::Terminal,
        title: "按精确数值对 CSV 数据行去重",
        query: "CSV: 数值去重 [\"金额\"] 编号,金额\n001,1.50\n002,3/2\n003,1.5e0\n004,5/4",
    },
    Example {
        icon: Icon::Terminal,
        title: "找出 CSV 中所选列精确数值重复的记录",
        query: "CSV: 数值查重复 [\"金额\"] 编号,金额\n001,1.50\n002,3/2\n003,1.5e0\n004,5/4",
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
        title: "按状态和精确金额筛选 CSV 行",
        query: "CSV: 多条件筛选 [[\"状态\",\"文本=\",\"完成\"],[3,\">=\",\"3/2\"]] 编号,状态,金额\n001,完成,1.50\n002,待确认,2e0\n003,完成,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "保留符合任一条件的 CSV 行",
        query: "CSV: 任一条件筛选 [[\"状态\",\"文本=\",\"完成\"],[\"商品\",\"正则\",\"^茶\"]] 编号,状态,商品\n001,完成,米\n002,待确认,茶粉\n003,待确认,面",
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
        title: "查看各组的最小金额",
        query: "CSV: 分组最小 [\"状态\"] \"金额\" 状态,金额\n完成,1/2\n待办,-2.50\n完成,2/3\n待办,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看各组的最大金额",
        query: "CSV: 分组最大 [\"状态\"] \"金额\" 状态,金额\n完成,1/2\n待办,-2.50\n完成,2/3\n待办,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "求各组最大值和最小值的精确差",
        query: "CSV: 分组极差 [\"状态\"] \"金额\" 状态,金额\n完成,1/2\n待办,-2.50\n完成,2/3\n待办,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "按分组选出精确中位数",
        query: "CSV: 分组中位数 [\"状态\"] \"金额\" 状态,金额\n完成,1/3\n待确认,2\n完成,1/2\n待确认,4\n待确认,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "保留每组金额最高的两条完整记录",
        query: "CSV: 组内最高 [[\"地区\"],\"金额\",2] 地区,金额,编号\n北,1/2,001\n南,3,002\n北,2e0,003\n北,0.50,004",
    },
    Example {
        icon: Icon::Terminal,
        title: "保留每组金额最低的一条完整记录",
        query: "CSV: 组内最低 [[\"地区\"],\"金额\",1] 地区,金额,编号\n北,2e0,001\n南,3,002\n北,1/2,003",
    },
    Example {
        icon: Icon::Terminal,
        title: "按两个分类透视汇总 CSV 数值",
        query: "CSV: 透视合计 [\"地区\",\"状态\",\"金额\"] 地区,状态,金额\n北,完成,1/2\n南,待办,0\n北,完成,2/3\n南,完成,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 CSV 的多列展开为列名和内容",
        query: "CSV: 逆透视 [[\"地区\"],\"月份\",\"金额\"] 地区,一月,二月\n北,2.50,\n南,1/3,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "按键把两张 CSV 表左连接",
        query: r#"CSV: 左连接 ["编号","键"] ["编号,商品\n001,茶\n002,米","键,金额\n001,1.50\n001,2e0"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按键保留两张 CSV 表匹配的行",
        query: r#"CSV: 内连接 [1,1] ["编号,商品\n001,茶\n002,米","键,金额\n001,1.50"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "合并两张 CSV 表并保留双方未匹配的行",
        query: r#"CSV: 全连接 ["编号","键"] ["编号,商品\n001,茶\n002,米","键,金额\n001,1.50\n003,2e0"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按地区和月份连接两张 CSV 表",
        query: r#"CSV: 全连接 [["区","月"],["区域","月份"]] ["区,月,品\n北,01,茶\n北,02,米","月份,区域,额\n01,北,1.50\n01,南,2e0"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "从左表找出右表没有的编号",
        query: r#"CSV: 左表独有 ["编号","键"] ["编号,商品\n001,茶\n002,米\n003,盐","键,金额\n001,1.50\n003,2e0"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "根据编号列出金额修改、删除和新增的记录",
        query: r#"CSV: 对比列 [["编号"],"金额"] ["编号,金额\n001,1.50\n002,3","金额,编号\n2e0,001\n4,003"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按精确数值比较两张表中的金额",
        query: r#"CSV: 数值对比列 [["编号"],"金额"] ["编号,金额\n001,1.50\n002,2\n003,","金额,编号\n3/2,001\n5/2,002\n0,003"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按地区和月份保留右表能匹配的左表行",
        query: r#"CSV: 左表匹配 [["区","月"],["区域","月份"]] ["区,月,额\n北,01,1.50\n北,02,2\n南,01,3","月份,区域\n01,北\n01,北\n01,南"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按表头名称把多张 CSV 表纵向合并",
        query: r#"CSV: 纵向合并 ["编号,金额\n001,1.50","金额,编号,备注\n2e0,002,\"茶,热\""]"#,
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
        title: "先按地区文本再按金额排列 CSV",
        query: "CSV: 多列排序 [[\"地区\",\"文本升序\"],[3,\"数值降序\"]] 编号,地区,金额\n001,区域10,1\n002,区域2,1.50\n003,区域2,2e0",
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
        title: "按指定边界统计 CSV 数值列的区间频率",
        query: "CSV: 列分箱 [\"金额\",\"0,1/2,2\"] 编号,金额\n001,-1\n002,0\n003,0.50\n004,1\n005,2",
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
        title: "按分段规范化 Windows 路径",
        query: r#"路径: 规范化 "C:/工作//发布/./临时/../报告 集.txt""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把标题整理为 Windows 文件名",
        query: "路径: 合法文件名 CON.报告?.txt",
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
        icon: Icon::Terminal,
        title: "把网址查询参数转成有序 JSON 数组",
        query: "URL: 参数JSON https://example.invalid/?tag=a+1&tag=b%2Bc",
    },
    Example {
        icon: Icon::Terminal,
        title: "提取指定 URL 查询参数的全部值",
        query: r#"URL: 取参数 "tag" https://example.invalid/?tag=a+1&page=2&tag=b%2Bc&tag="#,
    },
    Example {
        icon: Icon::Terminal,
        title: "从名称和值生成网址查询串",
        query: r#"URL: 生成查询串 [["tag","a 1"],["tag","b+c"]]"#,
    },
    Example {
        icon: Icon::Browser,
        title: "拆解 URL 组成",
        query: "URL: 拆解 https://example.invalid:8443/a%20b?x=1#part",
    },
    Example {
        icon: Icon::Browser,
        title: "把 URL 路径拆分成已解码的路径段",
        query: "URL: 路径分段 https://example.invalid/docs/a%2Fb/tea%20bag/",
    },
    Example {
        icon: Icon::Browser,
        title: "从独立路径段设置 URL 路径",
        query: r#"URL: 设路径 ["文档","a/b",""] https://example.invalid/old?x=a%20b#keep"#,
    },
    Example {
        icon: Icon::Browser,
        title: "移除 URL 查询串",
        query: "URL: 去查询串 https://example.invalid/doc?source=mail#part",
    },
    Example {
        icon: Icon::Terminal,
        title: "取出网址锚点并还原为文字",
        query: "URL: 取锚点 https://example.invalid/doc?q=a#%E7%9B%AE%E5%BD%95",
    },
    Example {
        icon: Icon::Terminal,
        title: "把文字写入网址锚点并编码",
        query: r#"URL: 设锚点 "目录 #2" https://example.invalid/doc?q=a#old"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "清除网址锚点并保留查询参数",
        query: "URL: 去锚点 https://example.invalid/doc?q=a#old",
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
        icon: Icon::Terminal,
        title: "按解码后的参数名稳定排列网址查询",
        query: "URL: 排序参数 https://example.invalid/doc?z=a%20b&%61=a+b&a=%2B#part",
    },
    Example {
        icon: Icon::Terminal,
        title: "去掉网址中名称和值均相同的参数",
        query: "URL: 去重参数 https://example.invalid/doc?tag=a+b&&%74ag=a%20b&tag=a%2Bb&flag&flag=#part",
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
        title: "同时设置多个查询参数",
        query: r#"URL: 设参数组 [["page","2"],["lang","zh"]] https://example.invalid/doc?page=1&q=a%20b&page=3#part"#,
    },
    Example {
        icon: Icon::Browser,
        title: "按顺序追加多个查询参数",
        query: r#"URL: 加参数组 [["tag","茶"],["tag","咖啡"]] https://example.invalid/doc?tag=first#part"#,
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
        title: "用精确分数求变化率并显示循环节",
        query: "百分比: 精确变化 3 到 4",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确分数计算占比",
        query: "百分比: 精确占比 -1 占 6",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算两个百分比之间的百分点",
        query: "百分比: 精确百分点 10% 到 31/3%",
    },
    Example {
        icon: Icon::Terminal,
        title: "连续应用多个百分比变化",
        query: "百分比: 连续变化 100 @ +10%, -10%",
    },
    Example {
        icon: Icon::Terminal,
        title: "按一次或多次百分比变化反推原值",
        query: "百分比: 原值 99 @ +10%, -10%",
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
        title: "精确舍入到最近的步长倍数",
        query: "分数: 就近对齐 2.675 to 0.05",
    },
    Example {
        icon: Icon::Terminal,
        title: "向下对齐到指定步长的倍数",
        query: "分数: 向下对齐 -2.675 to 0.05",
    },
    Example {
        icon: Icon::Terminal,
        title: "向上对齐到指定步长的倍数",
        query: "分数: 向上对齐 -2.675 to 0.05",
    },
    Example {
        icon: Icon::Terminal,
        title: "在精确分数算式里按位舍入",
        query: "分数: 计算 round(2.675,2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确分数在两个区间之间映射",
        query: "分数: 计算 remap(1/3,0,1,10,20)",
    },
    Example {
        icon: Icon::Terminal,
        title: "在分数算式中精确开方并取绝对值",
        query: "分数: 计算 sqrt(81/16)+abs(-1/2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "求大整数的平方根并向下取整",
        query: "分数: 计算 isqrt(2**100 - 1)",
    },
    Example {
        icon: Icon::Terminal,
        title: "求大整数的最大公约数",
        query: "分数: 计算 gcd(-3 * 2**80, 5 * 2**80)",
    },
    Example {
        icon: Icon::Terminal,
        title: "展开精确分数的循环小数",
        query: "分数: 循环小数 1/7",
    },
    Example {
        icon: Icon::Terminal,
        title: "把分数转成精确百分比",
        query: "分数: 百分比 1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "把百分比精确还原成分数",
        query: "分数: 从百分比 33.(3)%",
    },
    Example {
        icon: Icon::Terminal,
        title: "把带括号的循环小数精确还原成分数",
        query: "分数: 从循环小数 -2.1(6)",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算模幂或乘法逆元",
        query: "分数: 计算 pow(3,-1,11)",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算二进制、八进制和十六进制数",
        query: "分数: 计算 0x2000_0000_0000_01 + 0b10",
    },
    Example {
        icon: Icon::Terminal,
        title: "对超出浮点整数精度的数值做按位运算",
        query: "分数: 计算 bitxor(0x2000_0000_0000_01,0b1)",
    },
    Example {
        icon: Icon::Terminal,
        title: "给同一算式批量代入不同的 x",
        query: "分数: 取值表 [-2,0,3/2] x**2+1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "按首项和公差生成精确等差数列",
        query: "分数: 等差数列 1/3,1/6,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "按首项和公比生成精确等比数列",
        query: "分数: 等比数列 1,-1/2,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "求有理数的有限连分数",
        query: "分数: 连分数 355/113",
    },
    Example {
        icon: Icon::Terminal,
        title: "比较两个算式的精确分数值",
        query: "分数: 比较 1/3 < 0.34",
    },
    Example {
        icon: Icon::Terminal,
        title: "把假分数转换成带分数",
        query: "分数: 带分数 -7/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "把带分数还原成精确分数",
        query: "分数: 从带分数 -2 1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "限制分母并找最接近的分数",
        query: "分数: 限分母 0.3333 to 100",
    },
    Example {
        icon: Icon::Terminal,
        title: "把多项精确数值化为最简整数比",
        query: "分数: 化简比例 0.5,3/4,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "将有限连分数还原为精确分数",
        query: "分数: 从连分数 [3,7,16]",
    },
];

pub(super) fn search_key(text: &str) -> String {
    let narrowed: String = text
        .chars()
        .map(|character| match character {
            '\u{3000}' => ' ',
            _ => narrow_ascii_graphic(character),
        })
        .collect();
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
    // 示例补全只匹配操作名。已有参数的输入直接交给对应工具。
    if term.contains(char::is_whitespace) {
        return None;
    }
    match scope {
        Scope::BatchCalculation
            if matches!(term, "分" | "分数" | "分数对" | "分数对照" | "对" | "对照") =>
            {}
        Scope::Fraction
            if !term.is_empty()
                && [
                    "循环小数",
                    "百分比",
                    "从百分比",
                    "取值表",
                    "等差数列",
                    "等比数列",
                    "连分数",
                    "从连分数",
                    "从循环小数",
                    "比较",
                    "带分数",
                    "从带分数",
                    "限分母",
                    "化简比例",
                    "就近对齐",
                    "向下对齐",
                    "向上对齐",
                ]
                .into_iter()
                .any(|operation| operation.starts_with(term)) => {}
        Scope::BatchConversion
            if matches!(
                term,
                "时" | "时长"
                    | "时间"
                    | "时间戳"
                    | "路"
                    | "路径"
                    | "网"
                    | "网址"
                    | "日"
                    | "日期"
                    | "颜"
                    | "颜色"
            ) || (!term.is_empty()
                && [
                    "对照",
                    "时长对照",
                    "时间戳对照",
                    "路径对照",
                    "网址对照",
                    "日期对照",
                    "颜色对照",
                ]
                .into_iter()
                .any(|operation| operation.starts_with(term))) => {}
        Scope::Date
            if !term.is_empty()
                && [
                    "列出",
                    "格式化",
                    "解析",
                    "月历",
                    "工作日序号",
                    "合并区间",
                    "区间交集",
                    "扣除区间",
                    "按月拆分",
                    "按月拆分CSV",
                    "按周拆分CSV",
                    "按季度拆分CSV",
                    "逐日CSV",
                ]
                .into_iter()
                .any(|operation| operation.starts_with(term)) => {}
        Scope::Color if !term.is_empty() && "色阶".starts_with(term) => {}
        Scope::Timestamp
            if !term.is_empty()
                && ["向下对齐", "向上对齐", "时区CSV", "时区候选"]
                    .into_iter()
                    .any(|operation| operation.starts_with(term)) => {}
        Scope::Circuit
            if !term.is_empty()
                && ["串联", "并联", "耗电"]
                    .iter()
                    .any(|operation| operation.starts_with(term)) => {}
        Scope::Equation
            if !term.is_empty()
                && ["不等式", "顶点"]
                    .iter()
                    .any(|operation| operation.starts_with(term)) => {}
        Scope::LinearFit
            if !term.is_empty()
                && ["预测", "残差"]
                    .into_iter()
                    .any(|operation| operation.starts_with(term)) => {}
        Scope::Cidr
            if !term.is_empty()
                && ["子网", "排除"]
                    .into_iter()
                    .any(|operation| operation.starts_with(term)) => {}
        Scope::Matrix if matches!(term, "乘方" | "特" | "特征" | "特征值") => {}
        Scope::Duration
            if !term.is_empty()
                && [
                    "合计",
                    "平均",
                    "频数平均",
                    "频数中位数",
                    "扣除休息",
                    "比率",
                    "缩放",
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
            if !term.is_empty()
                && [
                    "二项概率",
                    "二项累计",
                    "超几何概率",
                    "截尾平均",
                    "移动合计",
                    "移动样本方差",
                    "移动总体方差",
                    "移动最小",
                    "移动最大",
                    "移动极差",
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
                    | "累计分"
                    | "累计分布"
                    | "累计次"
                    | "累计次数"
                    | "累计频"
                    | "累计频率"
                    | "分箱"
                    | "均"
                    | "均方"
                    | "均方根"
                    | "调"
                    | "调和"
                    | "调和平"
                    | "调和平均"
                    | "标"
                    | "标准"
                    | "标准误"
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
                    | "秩"
                    | "秩相"
                    | "秩相关"
                    | "差"
                    | "差分"
                    | "变"
                    | "变化"
                    | "变化率"
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
        | Scope::Complex
        | Scope::Circle
            if !term.contains(char::is_whitespace) => {}
        Scope::Date
        | Scope::Duration
        | Scope::BatchConversion
        | Scope::BatchCalculation
        | Scope::VideoFrames
        | Scope::Timecode
        | Scope::TransferTime
        | Scope::ExactData
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
    static OPERATIONS: OnceLock<Vec<(usize, Scope, String)>> = OnceLock::new();
    let operations = OPERATIONS.get_or_init(|| {
        EXAMPLES
            .iter()
            .enumerate()
            .filter_map(|(index, example)| {
                let (scope, input) = scoped_query(example.query)?;
                let operation = input.split_whitespace().next()?.to_ascii_lowercase();
                Some((index, scope, operation))
            })
            .collect()
    });
    let items: Vec<_> = operations
        .iter()
        .filter_map(|(index, example_scope, operation)| {
            (*example_scope == scope && operation.starts_with(&prefix)).then_some(*index)
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
    fn shows_scoped_command_completions_only_before_arguments() {
        for (scope, prefix, with_argument) in [
            (Scope::Csv, "排序", "排序 1 编号"),
            (Scope::Json, "改键", "改键 ["),
            (Scope::TextTools, "取列", "取列 \"x\""),
        ] {
            assert!(tool_operation_examples(scope, prefix).is_some());
            assert_eq!(tool_operation_examples(scope, with_argument), None);
        }
    }

    #[test]
    fn suggests_examples_for_empty_value_scopes_without_taking_over_the_input() {
        for scope in [
            Scope::Date,
            Scope::Duration,
            Scope::VideoFrames,
            Scope::Timecode,
            Scope::TransferTime,
            Scope::ExactData,
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
