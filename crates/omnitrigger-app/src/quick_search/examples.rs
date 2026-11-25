use std::{ops::Range, sync::OnceLock};

use crate::assets::Icon;

use super::calculator::narrow_ascii_graphic;
use super::data_pipeline::split_pipeline_input;
use super::json_tools::read_json_lines;
use super::operators::json_argument_and_tail;
use super::scientific::split_name;
use super::scope::{parse_scope, scoped_query, tool_example_keyword, Scope};
use super::text_tools::is_fixed_width_csv_operation;
use super::title_rank;

pub(super) struct Example {
    pub(super) icon: Icon,
    pub(super) title: &'static str,
    pub(super) query: &'static str,
}

impl Example {
    pub(super) fn data_start(&self) -> Option<usize> {
        let (scope, term) = scoped_query(self.query)?;
        let (operation, arguments) = term.split_once(char::is_whitespace)?;
        if scope == Scope::TextTools && is_fixed_width_csv_operation(operation) {
            let (_, data) = json_argument_and_tail::<Vec<usize>>(arguments)?;
            return (!data.is_empty()).then_some(self.query.len() - data.len());
        }
        if !matches!(scope, Scope::Csv | Scope::Json) {
            return None;
        }
        let operation = operation.to_ascii_lowercase();
        if scope == Scope::Json
            && !matches!(
                operation.as_str(),
                "行数组转csv"
                    | "rows-to-csv"
                    | "路径转csv"
                    | "pointers-to-csv"
                    | "行路径转csv"
                    | "jsonl-pointers-to-csv"
                    | "行分组统计csv"
                    | "jsonl-group-statistics-csv"
                    | "行分组中位数csv"
                    | "jsonl-group-median-csv"
                    | "行筛选"
                    | "filter-jsonl"
                    | "行任一筛选"
                    | "filter-any-jsonl"
                    | "行路径去重"
                    | "unique-jsonl-by-pointers"
                    | "行路径去重留末"
                    | "last-jsonl-by-pointers"
                    | "多路径去重留末"
                    | "last-by-pointers"
                    | "多条件筛选"
                    | "filter-all"
                    | "任一条件筛选"
                    | "filter-any"
                    | "选键"
                    | "pick-keys"
                    | "改键"
                    | "rename-key"
                    | "去键"
                    | "remove-keys"
            )
        {
            return None;
        }
        let (options, data) =
            json_argument_and_tail::<Box<serde_json::value::RawValue>>(arguments)?;
        if !options.get().starts_with(['[', '{']) {
            return None;
        }
        let has_data = if scope == Scope::Json {
            match operation.as_str() {
                "行路径转csv"
                | "jsonl-pointers-to-csv"
                | "行分组统计csv"
                | "jsonl-group-statistics-csv"
                | "行分组中位数csv"
                | "jsonl-group-median-csv"
                | "行筛选"
                | "filter-jsonl"
                | "行任一筛选"
                | "filter-any-jsonl"
                | "行路径去重"
                | "unique-jsonl-by-pointers"
                | "行路径去重留末"
                | "last-jsonl-by-pointers" => {
                    read_json_lines(data).is_ok_and(|rows| !rows.is_empty())
                }
                _ => serde_json::from_str::<&serde_json::value::RawValue>(data).is_ok(),
            }
        } else {
            data.contains(['\r', '\n']) && !data.trim().is_empty()
        };
        has_data.then_some(self.query.len() - data.len())
    }

    pub(super) fn argument_range(&self) -> Option<Range<usize>> {
        self.calculator_argument_range()
            .or_else(|| self.argument_start().map(|start| start..self.query.len()))
    }

    fn calculator_argument_range(&self) -> Option<Range<usize>> {
        let query = self.query.trim_end();
        let (expression, exact) = match scoped_query(query) {
            Some((Scope::Fraction, term)) => (fraction_command_expression(term)?, true),
            Some(_) => return None,
            None => (query.trim_start(), false),
        };
        let end = query.len();
        if let Some(arguments) = standalone_function_arguments(expression) {
            return Some(end - 1 - arguments.len()..end - 1);
        }
        (exact && !expression.is_empty()).then_some(end - expression.len()..end)
    }

    pub(super) fn argument_start(&self) -> Option<usize> {
        let (prefix, input) = self.query.split_once([':', '：'])?;
        let scope = parse_scope(prefix)?;
        if scope == Scope::DataPipeline {
            let (_, arguments) = split_pipeline_input(input);
            let (_, data) = json_argument_and_tail::<Vec<serde::de::IgnoredAny>>(arguments)?;
            return (!data.is_empty()).then_some(self.query.len() - data.len());
        }
        let input = input.trim_start();
        let operation_and_arguments = input.split_once(char::is_whitespace);
        let operation = operation_and_arguments.map_or(input, |(operation, _)| operation);
        let named_operation = replaceable_tool_operations(scope)
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(operation));
        if matches!(scope, Scope::Equation | Scope::LinearSystem) && !named_operation {
            return (!input.is_empty()).then_some(self.query.len() - input.len());
        }
        let (_, arguments) = operation_and_arguments?;
        if !matches!(
            scope,
            Scope::Json
                | Scope::Toml
                | Scope::Csv
                | Scope::TextTools
                | Scope::UrlEncoding
                | Scope::Path
                | Scope::Uuid
                | Scope::Ulid
                | Scope::Isbn
                | Scope::Circuit
                | Scope::Complex
                | Scope::Circle
                | Scope::Vector
                | Scope::Matrix
        ) && !named_operation
        {
            return None;
        }
        let arguments = if scope == Scope::Circuit && !named_operation {
            input
        } else {
            arguments.trim_start()
        };
        (!arguments.is_empty()).then_some(self.query.len() - arguments.len())
    }
}

pub(super) const EXAMPLES: &[Example] = &[
    Example {
        icon: Icon::Terminal,
        title: "计算算式",
        query: "2**10",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算带上标整数指数的算式",
        query: "2¹⁰",
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
        title: "直接解两边都是算式的不等式",
        query: "方程: 不等式 x² < 3x - 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求二次函数的顶点和极值",
        query: "方程: 顶点 2,-4,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看二次函数的值域、单调区间和零点",
        query: "方程: 摘要 y = x² - 2x - 1",
    },
    Example {
        icon: Icon::Terminal,
        title: "把二次式改写为配方后的形式",
        query: "方程: 配方 2,-3,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算二次方程的精确判别式",
        query: "方程: 判别式 1,0,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "用韦达定理计算两根之和与积",
        query: "方程: 韦达 2,-3,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "由两个有理数根求二次方程的系数",
        query: "方程: 由根 1/2,3/2,-2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求抛物线和直线的精确交点",
        query: "方程: 交点 1,0,0 ; 0,-1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求两条曲线围成的精确面积",
        query: "方程: 围成面积 1,0,0 ; 0,0,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "把二次函数系数写成标准式",
        query: "方程: 标准式 1/2,-1,3/4",
    },
    Example {
        icon: Icon::Terminal,
        title: "把二次式分解为整数一次因式",
        query: "方程: 因式分解 2x² - 5x + 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "把因式乘积展开为标准式",
        query: "方程: 展开 (2x - 1)(x - 2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "从因式乘积直接求根",
        query: "方程: (2x - 1)(x - 2) = 0",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接对函数标准式求根",
        query: "方程: 求根 y = 2x² - 5x + 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接求配方后的二次式的根",
        query: "方程: 求根 (x - 1)**2 - 4",
    },
    Example {
        icon: Icon::Terminal,
        title: "代入候选值逐个验根",
        query: "方程: 验根 (2x-1)(x-2) = 0 @ 1/2,1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "用函数展开式直接求积分",
        query: "方程: 定积分 y = x**2 @ 0 到 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接求展开的二次方程",
        query: "方程: x**2 - 3*x + 2 = 0",
    },
    Example {
        icon: Icon::Terminal,
        title: "从展开式提取精确系数",
        query: "方程: 取系数 (1/3+1/6)*x^2 - x + 3/4",
    },
    Example {
        icon: Icon::Terminal,
        title: "由三点求通过各点的函数系数",
        query: "方程: 过三点 0,3/4 ; 1/2,3/8 ; 2,3/4",
    },
    Example {
        icon: Icon::Terminal,
        title: "由顶点和开口系数求二次函数",
        query: "方程: 由顶点 3/2,-1/4,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求切线系数",
        query: "方程: 切线 1/2,-1,3/4 @ 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求穿过曲线上指定点并垂直于切线的直线",
        query: "方程: 法线 x² @ 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "绘制指定区间内的二次函数曲线",
        query: "方程: 函数图 (x-1)**2 - 1 @ -1 到 4",
    },
    Example {
        icon: Icon::Terminal,
        title: "在给定区间采样计算器函数并输出 CSV",
        query: "方程: 函数采样 cosd(x) @ 0 到 180; 3",
    },
    Example {
        icon: Icon::Terminal,
        title: "找出等距采样点中的最小值和最大值",
        query: "方程: 采样极值 cosd(x) @ 0..360; 5",
    },
    Example {
        icon: Icon::Terminal,
        title: "在单谷区间内估算极小值和横坐标",
        query: "方程: 数值极小 (x-3/2)^2 @ 0..3",
    },
    Example {
        icon: Icon::Terminal,
        title: "在单峰区间内估算极大值和横坐标",
        query: "方程: 数值极大 sin(x) @ 0..pi",
    },
    Example {
        icon: Icon::Terminal,
        title: "查找采样零值和相邻异号区间",
        query: "方程: 找异号区间 x²-2 @ 0..2; 3",
    },
    Example {
        icon: Icon::Terminal,
        title: "先采样再二分求出多个近似零点",
        query: "方程: 搜索零点 x²-2 @ -2..2; 5",
    },
    Example {
        icon: Icon::Terminal,
        title: "绘制计算器函数在指定区间的近似采样图",
        query: "方程: 采样图 cosd(x) @ 0 到 360; 33",
    },
    Example {
        icon: Icon::Terminal,
        title: "用两个点分隔函数图的区间端点",
        query: "方程: 采样图 f(x) = sin(x) @ 0..2*pi; 65",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算函数在指定横坐标的近似值",
        query: "方程: 函数值 sqrt(x) @ 9,-1,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐步缩小步长并比较中心点两侧的函数值",
        query: "方程: 两侧采样 sin(x)/x @ 0; 0.1",
    },
    Example {
        icon: Icon::Terminal,
        title: "用 f(x) 记法计算指定坐标的函数值",
        query: "方程: 函数值 f(x) = cosd(x) @ 180,0,90",
    },
    Example {
        icon: Icon::Terminal,
        title: "在相同横坐标上并排比较多个函数",
        query: r#"方程: 函数对照 ["cosd(x)","sind(x)"] @ 0,90,180"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "在同一个坐标系里比较多个函数的采样图",
        query: r#"方程: 函数对照图 ["cosd(x)","sind(x)"] @ 0..360; 33"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按参数采样平面坐标并导出 CSV",
        query: r#"方程: 参数采样 ["cosd(t)","sind(t)"] @ 0..360; 5"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按相同比例绘制平面参数采样图",
        query: r#"方程: 参数图 ["cosd(t)","sind(t)"] @ 0..360; 65"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "由半径算式绘制极坐标曲线",
        query: "方程: 极坐标图 1+cos(t) @ 0..2*pi; 65",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看极角、半径和对应的平面坐标",
        query: "方程: 极坐标采样 2*cos(3*t) @ 0..2*pi; 9",
    },
    Example {
        icon: Icon::Terminal,
        title: "估算极坐标半径在角度区间内扫过的面积",
        query: "方程: 极坐标面积 1+cos(t) @ 0..2*pi; 64",
    },
    Example {
        icon: Icon::Terminal,
        title: "用二分法求连续函数的一个近似零点",
        query: "方程: 二分求根 cos(x)-x @ 0 到 1",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接填写左右两式并求它们相等时的近似横坐标",
        query: "方程: 二分求根 cos(x)=x @ 0..1",
    },
    Example {
        icon: Icon::Terminal,
        title: "估算计算器函数在指定区间的积分",
        query: "方程: 数值积分 sin(x) @ 0 到 pi; 64",
    },
    Example {
        icon: Icon::Terminal,
        title: "对照不同采样密度下的数值积分",
        query: "方程: 积分对照 x^4 @ 0..1; 4",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算函数在整个区间上的近似平均值",
        query: "方程: 数值平均 x² @ 0..3; 8",
    },
    Example {
        icon: Icon::Terminal,
        title: "估算两条曲线在区间内围成的面积",
        query: "方程: 数值面积 [\"x\",\"0\"] @ -1..1; 8",
    },
    Example {
        icon: Icon::Terminal,
        title: "估算光滑曲线在给定区间内的弧长",
        query: "方程: 数值弧长 sin(x) @ 0..pi; 64",
    },
    Example {
        icon: Icon::Terminal,
        title: "用中心差分估算指定坐标处的导数",
        query: "方程: 数值导数 sin(x) @ 0; 0.0001",
    },
    Example {
        icon: Icon::Terminal,
        title: "由两个函数值求区间平均变化率",
        query: "方程: 平均变化率 x² @ 1..3",
    },
    Example {
        icon: Icon::Terminal,
        title: "用中心差分求任意光滑函数的近似切线",
        query: "方程: 数值切线 sin(x) @ 0; 0.0001",
    },
    Example {
        icon: Icon::Terminal,
        title: "在同一张图对照函数和指定点的近似切线",
        query: "方程: 数值切线图 sin(x) @ 0; -pi..pi; 33",
    },
    Example {
        icon: Icon::Terminal,
        title: "根据数值切线斜率求法线",
        query: "方程: 数值法线 x² @ 2; 0.5",
    },
    Example {
        icon: Icon::Terminal,
        title: "估算计算器函数在一点处的二阶导数",
        query: "方程: 数值二阶导数 x² @ 2; 0.5",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐点估算区间内的数值导数",
        query: "方程: 导数采样 x³ @ -2..2; 5",
    },
    Example {
        icon: Icon::Terminal,
        title: "绘制导数近似值并保留无定义样点",
        query: "方程: 导数采样图 sqrt(x) @ -1..2; 13",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看从区间左端点开始的累计积分",
        query: "方程: 积分采样 x @ 0..2; 3",
    },
    Example {
        icon: Icon::Terminal,
        title: "绘制区间上的累计近似积分",
        query: "方程: 积分采样图 sin(x) @ 0..pi; 17",
    },
    Example {
        icon: Icon::Terminal,
        title: "求二次函数在给定区间上的精确定积分",
        query: "方程: 定积分 1/2,-1,3/4 @ 0 到 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求精确的多项式导函数",
        query: "方程: 求导 y = x²/2 - x + 3/4",
    },
    Example {
        icon: Icon::Terminal,
        title: "求含积分常数的多项式原函数",
        query: "方程: 原函数 y = x²/2 - x + 3/4",
    },
    Example {
        icon: Icon::Terminal,
        title: "求二次函数在闭区间上的最小值和最大值",
        query: "方程: 区间极值 1/2,-1,3/4 @ 0 到 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "联立求解两个二元一次方程",
        query: "联立: 2,1,5; 1,-1,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "写出联立方程通解",
        query: "联立: 通解 1,2,3; 2,4,6",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接输入 x、y 的两条方程",
        query: "联立: x + y = 3; x - y = 1",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接解三元一次方程组",
        query: "联立: 三元 x+y+z=6; x-y=0; z=2",
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
        title: "用经过给定数据点的多项式求插值",
        query: "拟合: 插值 1/2,2 @ 0,1; 1,2; 2,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "取得插值多项式的系数表",
        query: "拟合: 插值系数 0,1; 1,2; 2,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "生成可交给计算器的插值式",
        query: "拟合: 插值式 0,1; 1,2; 2,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "用最小二乘拟合二次多项式",
        query: "拟合: 多项式 2 @ -1,2; 0,0; 1,2; 0,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐行查看多项式拟合残差",
        query: "拟合: 多项式残差 2 @ -1,2; 0,0; 1,2; 0,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看多项式拟合误差",
        query: "拟合: 多项式误差 2 @ -1,2; 0,0; 1,2; 0,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "用同一个多项式预测多个值",
        query: "拟合: 多项式预测 2 @ 1/2,2,0 @ -1,2; 0,0; 1,2; 0,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看多项式拟合式和 R²",
        query: "拟合: 多项式摘要 2 @ -1,2; 0,0; 1,2; 0,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按每条观测的权重拟合多项式",
        query: "拟合: 加权多项式 1 @ 0,0,3; 0,4,1; 1,3,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "拟合一条受异常值影响较小的直线",
        query: "拟合: 稳健直线 0,1; 1,3; 2,5; 3,7; 4,100",
    },
    Example {
        icon: Icon::Terminal,
        title: "在搜索框中查看数据点的散点图",
        query: "拟合: 散点图 -2,5; -1,2; 0,1; 1,2; 2,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "对照多项式拟合曲线与实际数据点",
        query: "拟合: 曲线图 2 @ -1,2; 0,0; 1,2; 0,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看各观测点在零残差线上下的位置",
        query: "拟合: 残差图 2 @ -1,2; 0,0; 1,2; 0,2",
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
        title: "计算拟合的残差平方和与均方根误差",
        query: "拟合: 误差 0,0; 1,1; 2,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "固定截距为零并拟合过原点的直线",
        query: "拟合: 过原点 1,2; 2,5; 3,7",
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
        title: "由两个直角边求斜边",
        query: "三角形: 斜边 1,1",
    },
    Example {
        icon: Icon::Layers,
        title: "由斜边和一个直角边求另一条边",
        query: "三角形: 直角边 5,3",
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
        title: "在同一张表中查看多个精确分位数",
        query: "统计: 分位表 [25%,50%,75%] 4,1,3,2",
    },
    Example {
        icon: Icon::Layers,
        title: "求四分位数和四分位距",
        query: "统计: 四分位 4,1,3,2",
    },
    Example {
        icon: Icon::Layers,
        title: "查看箱线图的四分位数、须端和离群值",
        query: "统计: 箱线摘要 100,3,1,4,2",
    },
    Example {
        icon: Icon::Layers,
        title: "在搜索框查看四分位数和离群观测的箱线图",
        query: "统计: 箱线图 1,2,3,4,8",
    },
    Example {
        icon: Icon::Terminal,
        title: "求绝对偏差的中位数",
        query: "统计: 中位绝对偏差 0,1/2,1,10",
    },
    Example {
        icon: Icon::Terminal,
        title: "求各值到样本平均值的平均距离",
        query: "统计: 平均绝对偏差 0,1/2,1,10",
    },
    Example {
        icon: Icon::Terminal,
        title: "去掉两端的观测后求截尾平均",
        query: "统计: 截尾平均 25% 1,2,3,100",
    },
    Example {
        icon: Icon::Terminal,
        title: "替换两端的极端观测后求缩尾平均",
        query: "统计: 缩尾平均 20% 0,2,3,30,100",
    },
    Example {
        icon: Icon::Terminal,
        title: "按非负观测值精确计算基尼系数",
        query: "统计: 基尼系数 0,0,4,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "求观测两两配对的平均绝对差",
        query: "统计: 基尼平均差 1,2,5",
    },
    Example {
        icon: Icon::Layers,
        title: "按精确数值列出次数和频率",
        query: "统计: 频数表 3,1/2,0.5,3,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "在表格中查看次数和精确频率",
        query: "统计: 频数CSV 3,1/2,0.5,3,5",
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
        icon: Icon::Layers,
        title: "粘贴数值和权重两列求加权平均值",
        query: "统计: 加权平均 80\t30%\n90\t70%",
    },
    Example {
        icon: Icon::Layers,
        title: "按权重求精确总体方差",
        query: "统计: 加权方差 0@1,4@3",
    },
    Example {
        icon: Icon::Layers,
        title: "按权重求精确总体标准差",
        query: "统计: 加权标准差 0@25%,4@75%",
    },
    Example {
        icon: Icon::Layers,
        title: "按累计权重求分位数",
        query: "统计: 加权分位数 60% 3@40%,1@10%,2@50%",
    },
    Example {
        icon: Icon::Layers,
        title: "一次预览多组加权分位数",
        query: "统计: 加权分位表 [25%,50%,75%] 1@1,2@1,4@2",
    },
    Example {
        icon: Icon::Layers,
        title: "预览累计权重和累计比例",
        query: "统计: 加权累积分布 4@25%,1/2@1/8,4@1/8,-1@0,2@1/4",
    },
    Example {
        icon: Icon::Terminal,
        title: "求小于或等于阈值的权重",
        query: "统计: 加权累计权重 2 @ 1@1,2@1/2,3@2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求小于或等于阈值的精确加权比例",
        query: "统计: 加权累计比例 2 @ 1@1,2@1/2,3@2",
    },
    Example {
        icon: Icon::Layers,
        title: "预览每个数值合并后的权重和精确占比",
        query: "统计: 加权频数CSV 2@2,0.5@1/4,-1@0,1/2@3/4",
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
        title: "查看每个窗口的精确样本标准差",
        query: "统计: 移动样本标准差 2 1/2,1,2,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看每个窗口的精确总体标准差",
        query: "统计: 移动总体标准差 2 1/2,1,2,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "按固定窗口计算精确移动中位数",
        query: "统计: 移动中位数 3 1/2,100,2,3,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐个窗口计算精确分位数",
        query: "统计: 移动分位数 3,25% 4,1,3,2,5",
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
        title: "用阶梯图查看每个值及以下的累计频率",
        query: "统计: 累计分布图 3,1/2,0.5,3,5",
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
        icon: Icon::Layers,
        title: "在搜索框对比各区间的观测次数",
        query: "统计: 分箱图 0,1/2,2 @ -1,0,0.50,1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算恰好成功指定次数的二项概率",
        query: "统计: 二项概率 4,2,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "表格查看二项分布的恰好、至多和至少概率",
        query: "统计: 二项概率表 3,1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "求二项分布达到目标累计概率的最小成功数",
        query: "统计: 二项分位点 3,1/3,75%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算二项分布的期望、方差和标准差",
        query: "统计: 二项摘要 4,1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算第几次试验达到目标成功数的概率",
        query: "统计: 负二项概率 3,5,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算指定次数内达到成功目标的概率",
        query: "统计: 负二项累计 3,5,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算达到成功目标至少需要指定次数的概率",
        query: "统计: 负二项至少 3,5,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算在给定试验次数区间内达到成功目标的概率",
        query: "统计: 负二项区间概率 2,3..4,50%",
    },
    Example {
        icon: Icon::Layers,
        title: "查看达到成功目标所需试验次数的概率表",
        query: "统计: 负二项概率表 2,4,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看达到目标成功次数所需试验数的期望和方差",
        query: "统计: 负二项摘要 3,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "求达到成功目标所需试验数的累计分位点",
        query: "统计: 负二项分位点 3,50%,75%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算在指定试验中首次成功的概率",
        query: "统计: 几何概率 3,25%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算在指定次数内至少成功一次的概率",
        query: "统计: 几何累计 3,25%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算首次成功至少需要指定次数的概率",
        query: "统计: 几何至少 3,25%",
    },
    Example {
        icon: Icon::Terminal,
        title: "找出达到累计成功概率的最少试验次数",
        query: "统计: 几何分位点 25%,90%",
    },
    Example {
        icon: Icon::Terminal,
        title: "按泊松分布计算恰好发生指定次数的概率",
        query: "统计: 泊松概率 2,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按泊松分布计算至多发生指定次数的概率",
        query: "统计: 泊松累计 2,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按泊松分布计算至少发生指定次数的概率",
        query: "统计: 泊松至少 2,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看泊松分布的期望方差及标准差",
        query: "统计: 泊松摘要 1/2",
    },
    Example {
        icon: Icon::Layers,
        title: "预览指定事件次数及之前的泊松概率",
        query: "统计: 泊松概率表 2,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算泊松事件次数落在指定区间的概率",
        query: "统计: 泊松区间概率 2..4,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算首次成功所需试验次数的期望和方差",
        query: "统计: 几何摘要 25%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算成功次数不超过指定值的二项概率",
        query: "统计: 二项累计 4,2,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算至少成功指定次数的二项概率",
        query: "统计: 二项至少 4,3,25%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算二项试验成功次数落在指定区间的概率",
        query: "统计: 二项区间概率 4,1..2,50%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算不放回抽取的精确概率",
        query: "统计: 超几何概率 10,4,3,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "表格查看不放回抽取的精确命中概率",
        query: "统计: 超几何概率表 10,4,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算不放回抽取至多命中指定次数的概率",
        query: "统计: 超几何累计 10,4,3,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算不放回抽取至少命中指定次数的概率",
        query: "统计: 超几何至少 10,4,3,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算不放回抽取的命中次数落在指定区间的概率",
        query: "统计: 超几何区间概率 10,4,3,1..2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求不放回抽取达到目标累计概率的最小命中数",
        query: "统计: 超几何分位点 10,4,3,90%",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算不放回抽取的期望、方差和标准差",
        query: "统计: 超几何摘要 10,4,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算列联表的皮尔逊卡方和自由度",
        query: "统计: 独立性卡方 1,2; 3,4",
    },
    Example {
        icon: Icon::Layers,
        title: "计算独立性假设下的各格期望频数",
        query: "统计: 期望频数 1,2; 3,4",
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
        icon: Icon::Terminal,
        title: "用样本标准差求每个值的标准分",
        query: "统计: 标准分 3,1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "求样本标准差与平均值的比值",
        query: "统计: 变异系数 1,2,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "按总体标准差求变异系数",
        query: "统计: 总体变异系数 1,2,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "用总体标准差求每个值的标准分",
        query: "统计: 总体标准分 0,2",
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
        icon: Icon::Layers,
        title: "计算考虑并列值的肯德尔秩相关",
        query: "统计: 肯德尔 0,0; 1,1; 2,1",
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
        icon: Icon::Terminal,
        title: "由三个二维点求圆心和半径",
        query: "圆: 三点 1,0; 3,0; 1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按圆心和半径判断两个圆的位置关系",
        query: "圆: 两圆关系 0,0 @ 1/2; 1,0 @ 1/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按圆心和半径计算两个圆的交点",
        query: "圆: 两圆交点 0,0 @ 1; 1,0 @ 1",
    },
    Example {
        icon: Icon::Terminal,
        title: "求直线与圆的交点，并保留分数和根式",
        query: "圆: 直线交点 0,0 @ 1; -1,1/2; 1,1/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "从圆外一点引出两条切线并求出切点",
        query: "圆: 切点 0,0 @ 1; 2,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算圆被直线截得的弦长和弦中点",
        query: "圆: 弦长 0,0 @ 1; -1,1/2; 1,1/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "判断分数坐标的点位于圆内、圆周上还是圆外",
        query: "圆: 包含 3/5,4/5 @ 0,0 @ 1",
    },
    Example {
        icon: Icon::Terminal,
        title: "由直径端点求圆心和半径",
        query: "圆: 直径端点 -1,0; 3,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "由顶点坐标计算凹多边形的精确面积",
        query: "多边形: 0,0;4,0;2,2;4,4;0,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "求均匀多边形的面积重心",
        query: "多边形: 重心 0,0;3,0;3,1;1,1;1,3;0,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "判断分数坐标的点是否位于多边形边界",
        query: "多边形: 包含 5/2,3/2 @ 0,0;4,0;2,2;4,4;0,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确分数和根式计算多边形周长",
        query: "多边形: 周长 0,0;1,1;2,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看多边形沿坐标轴的边界范围",
        query: "多边形: 边界框 -1/2,-2;2,-2;1,1;-1/2,1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "按轮廓顶点判断多边形是凸还是凹",
        query: "多边形: 凸性 0,0;4,0;2,2;4,4;0,4",
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
        title: "计算对应坐标绝对差的和",
        query: "向量: 曼哈顿距离 1/2,-2,3; -1,-1/4,3",
    },
    Example {
        icon: Icon::Layers,
        title: "计算对应坐标最大的绝对差",
        query: "向量: 切比雪夫距离 1/2,-2,3; -1,-1/4,3",
    },
    Example {
        icon: Icon::Layers,
        title: "计算点到线段的距离和最近点",
        query: "向量: 线段距离 3,4 @ 0,0; 2,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "求三维点到平面的距离和最近点",
        query: "向量: 平面距离 3,4,5 @ 0,0,0; 1,0,0; 0,1,0",
    },
    Example {
        icon: Icon::Layers,
        title: "计算点到整条直线的距离和垂足",
        query: "向量: 直线距离 3,4 @ 0,0; 2,0",
    },
    Example {
        icon: Icon::Layers,
        title: "求点关于整条直线的精确对称坐标",
        query: "向量: 直线对称 3,4 @ 0,0; 2,0",
    },
    Example {
        icon: Icon::Layers,
        title: "求三维点关于平面的精确对称坐标",
        query: "向量: 平面对称 1,1,1 @ 1,0,0; 0,1,0; 0,0,1",
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
        title: "求矩形矩阵对应线性系统的参数通解",
        query: "矩阵: 通解 1,0,1; 0,1,0 @ 2; 1",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确分数求矩阵的唯一最小二乘解",
        query: "矩阵: 最小二乘 1,0; 1,1; 1,2 @ 1; 2; 2",
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
        icon: Icon::Terminal,
        title: "求一对互为相反数的精确复数平方根",
        query: "复数: 平方根 3,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接输入带 i 的复数并求整数次幂",
        query: "复数: 乘方 3+4i @ 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看复数的精确模和以度表示的辐角",
        query: "复数: 极坐标 3+4i",
    },
    Example {
        icon: Icon::Terminal,
        title: "按模和辐角计算复数的直角式",
        query: "复数: 由极坐标 2 @ 30°",
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
        icon: Icon::Terminal,
        title: "计算矩阵各列绝对值之和的最大值",
        query: "矩阵: 1范数 1/2,-2;3,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算矩阵各行绝对值之和的最大值",
        query: "矩阵: 无穷范数 1/2,-2;3,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "用 1 范数求方阵的精确条件数",
        query: "矩阵: 条件数 1 @ 2,0; 0,1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "用顺序主子式检查矩阵正定性",
        query: "矩阵: 正定性 2,1; 1,2",
    },
    Example {
        icon: Icon::Layers,
        title: "换算单位",
        query: "10 km to mi",
    },
    Example {
        icon: Icon::Layers,
        title: "在公顷、亩和平方米等面积单位间换算",
        query: "2 ha to 亩",
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
        title: "在表格中对照多个精确数据单位",
        query: "数据换算: 单位表 [B,b,MiB] 1/3 KiB",
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
        title: "逐行把时长换算成精确分数",
        query: "批量换算: 精确时长 1秒 to min\nPT1M30S to h\n-0.25秒 to min",
    },
    Example {
        icon: Icon::Clock,
        title: "导出每行时长的原输入和精确换算结果",
        query: "批量换算: 精确时长对照 1秒 to min\n23:45 到 01:15 to d\n1ns to s",
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
        icon: Icon::Terminal,
        title: "以 CSV 对照进制和字节序操作的输入与结果",
        query: "批量换算: 进制对照 ff 从 16 到 10\n字节序 16 @ 258\n读字节序 小端 @ 02 01",
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
        title: "逐行换算节拍速度并导出输入与结果",
        query: "批量换算: 节拍对照 速度 1.5秒 @ 3\n速度 2秒 @ 3",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐行换算分贝、功率比和幅度比",
        query: "批量换算: 分贝 功率比 100\n转幅度比 20 dB",
    },
    Example {
        icon: Icon::Layers,
        title: "逐行计算立体表面积和体积并导出 CSV",
        query: "批量换算: 立体对照 圆柱 3,5\n球 1/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐行换算波长与频率并导出 CSV",
        query: "批量换算: 波长对照 1 MHz\n34.3 cm @ 343 m/s to kHz",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐行计算网段内的地址和子网",
        query: "批量换算: 网段 取地址 192.0.2.137/26,2\n取子网 192.0.2.137/26,/28,3",
    },
    Example {
        icon: Icon::Layers,
        title: "导出网段运算输入与结果的对照表",
        query: "批量换算: 网段对照 子网 192.0.2.0/30 到 /31\n偏移 2001:db8::3/126,1",
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
        icon: Icon::Terminal,
        title: "跨工具清理文本、转换 CSV 并取出 JSON 字段",
        query: "处理: [\"文本: 去空行\",\"CSV: 数值对象 [2]\",\"JSON: 取值 /1/金额\"] 编号,金额\n001,1.50\n\n002,2e0",
    },
    Example {
        icon: Icon::Terminal,
        title: "将每一步的输出与执行命令导出为 CSV",
        query: "处理: 步骤CSV [\"文本: 大写\",\"文本: 按行拼接 \\\", \\\"\"] 茶 a\n咖啡 b",
    },
    Example {
        icon: Icon::Layers,
        title: "对每条分数算式分别计算球的表面积和体积",
        query: "处理: 逐行 [\"分数: 计算\",\"立体: 球\"]\n1/4+1/4\n1",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐条提取 JSON 金额并输出精确分数",
        query: "处理: 逐行CSV [\"JSON: 取值 /金额\",\"分数:\"]\n{\"金额\":0.50}\n{\"金额\":1.25}",
    },
    Example {
        icon: Icon::Layers,
        title: "逐个处理 CSV 单元格并添加精确金额列",
        query: "处理: CSV列 [\"精确金额\",\"明细\",[\"JSON: 取值 /金额\",\"分数:\"]]\n编号,明细\n001,\"{\"\"金额\"\":0.50}\"\n002,\"{\"\"金额\"\":1.25}\"\n003,",
    },
    Example {
        icon: Icon::Layers,
        title: "逐项提取 JSON 数组中的金额并保留精确分数",
        query: r#"处理: JSON数组 ["JSON: 取值 /金额","分数:"] [{"金额":0.50},{"金额":1.25}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "逐个处理 JSON 对象的字段值",
        query: r#"处理: JSON对象 ["分数:"] {"乙":1.25,"甲":0.50}"#,
    },
    Example {
        icon: Icon::Layers,
        title: "提取逐行 JSON 的金额并输出精确分数",
        query: "处理: JSON行 [\"JSON: 取值 /金额\",\"分数:\"]\n{\"金额\":0.50}\n{\"金额\":1.25}",
    },
    Example {
        icon: Icon::Browser,
        title: "解码网址查询参数并取出第一个值",
        query: r#"处理: ["URL: 参数JSON","JSON: 取值 /0/1"] https://example.invalid/?q=%E8%8C%B6+1"#,
    },
    Example {
        icon: Icon::Clock,
        title: "生成月底日期并在一行中拼接",
        query: r#"处理: ["日期: 月末序列","文本: 按行拼接 \", \""] 2024-01,3"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "用 UTC 起止时刻创建两列 CSV",
        query: r#"处理: ["时间戳: 日区间","CSV: 选列名 [\"开始UTC\",\"结束UTC\"]"] 2026-03-08 America/New_York"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按匹配的 IP 网段筛选地址归属表",
        query: r#"处理: ["网段: 归属CSV [\"192.0.2.0/24\",\"192.0.2.128/25\"]","CSV: 搜索行 \"192.0.2.128/25\""] 192.0.2.190,192.0.2.1"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "提取 CSV 数值列并计算中位数",
        query: "处理: [\"CSV: 选列 2\",\"文本: 间隔取行 2,1\",\"统计: 分位数 50%\"] 名称,金额\n茶,1/2\n咖啡,5/2\n豆浆,3/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "先筛选表格数据，再在搜索框画散点图",
        query: "处理: [\"CSV: 数值筛选 2 >= 0\",\"CSV: 散点图 [2,3]\"] 编号,时间,温度\n001,-1,2\n002,0,1/2\n003,1,5/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "串联进制换算与音高查询",
        query: r#"处理: ["进制: 罗马数字","音高: MIDI"] LXIX"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "用算式算出拍数并换算节拍速度",
        query: r#"处理: ["分数: 计算","节拍: 速度 4秒 @"] 2*4"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "先求并联电阻的等效值，再计算分压",
        query: r#"处理: ["电路: 并联","电路: 分压 12 V @ 1 kΩ,"] 2 kΩ,2 kΩ"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把上一步结果填进命令中指定的位置",
        query: r#"处理: ["文本: 去空行",{"模板":"电路: 分压 {结果} @ 1 kΩ,2 kΩ"}] 12 V"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "在处理流程中从精确分数计算重复变动与总变化率",
        query: r#"处理: ["分数: 计算",{"模板":"百分比: 重复变化 {结果} @ +50%,2"},"百分比: 精确变化 2 到"] 3/2+1/2"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "串联复数的乘方、共轭和求模",
        query: r#"处理: [{"模板":"复数: 乘方 {结果} @ 2"},"复数: 共轭","复数: 模"] 3+4i"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "串接方程系数和求根",
        query: r#"处理: ["方程: 由根","方程:"] 1/2,3/2,-2"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "先转置矩阵，再求行列式",
        query: r#"处理: ["矩阵: 转置","矩阵: 行列式"] 1,2; 3,4"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "对数据插值并取出结果列",
        query: r#"处理: ["拟合: 插值 1/2,2 @","CSV: 选列名 [\"插值\"]"] 0,1; 1,2; 2,5"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把节拍时长换成时分秒格式",
        query: r#"处理: ["节拍: 时长","时长:"] 120 BPM @ 3/2"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把逐行输入的值相加并输出精确分数",
        query: "处理: [\"文本: 按行拼接 \\\"+\\\"\",\"分数: 计算\"] 1/3\n1/6",
    },
    Example {
        icon: Icon::Grid,
        title: "切换已打开的窗口",
        query: "窗口:",
    },
    Example {
        icon: Icon::Search,
        title: "筛选已启用的全局键盘触发器",
        query: "触发器: 状态=已启用 范围=全局 输入=键盘",
    },
    Example {
        icon: Icon::Search,
        title: "把筛选后的触发器导出为 CSV",
        query: "触发器: 输出=CSV 状态=已启用 输入=键盘",
    },
    Example {
        icon: Icon::Terminal,
        title: "把筛选后的触发器导出为 JSON",
        query: "触发器: 输出=JSON 状态=已启用 输入=键盘",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计筛选后各类触发器动作的数量",
        query: "触发器: 输出=动作统计 状态=已启用",
    },
    Example {
        icon: Icon::Search,
        title: "用正则表达式查找触发器名称",
        query: r"触发器: 名称正则=^打开触发",
    },
    Example {
        icon: Icon::Terminal,
        title: "按名称排序筛选后的触发器",
        query: "触发器: 状态=已启用 排序=名称",
    },
    Example {
        icon: Icon::Grid,
        title: "把指定程序的窗口列表导出为 JSON",
        query: "窗口: 输出=JSON 进程=notepad.exe",
    },
    Example {
        icon: Icon::Grid,
        title: "把匹配窗口的标题和程序文件名导出为 CSV",
        query: "窗口: 输出=CSV 进程=notepad.exe",
    },
    Example {
        icon: Icon::Grid,
        title: "在导出的 CSV 中查看窗口最小化状态",
        query: "窗口: 输出=状态CSV 排序=进程",
    },
    Example {
        icon: Icon::Grid,
        title: "按程序统计匹配窗口的数量",
        query: "窗口: 输出=进程统计",
    },
    Example {
        icon: Icon::Grid,
        title: "按进程统计窗口的最小化状态",
        query: "窗口: 输出=进程状态统计",
    },
    Example {
        icon: Icon::Search,
        title: "按程序文件名排除窗口",
        query: "窗口: 排除进程=notepad.exe 报告",
    },
    Example {
        icon: Icon::Search,
        title: "同时查找多个指定程序的窗口",
        query: r#"窗口: 任一进程=["code.exe","notepad.exe"] 报告"#,
    },
    Example {
        icon: Icon::Search,
        title: "一次排除多个程序的窗口",
        query: r#"窗口: 排除进程列表=["code.exe","notepad.exe"]"#,
    },
    Example {
        icon: Icon::Grid,
        title: "查找当前已最小化的窗口",
        query: "窗口: 状态=已最小化",
    },
    Example {
        icon: Icon::Search,
        title: "按正则表达式筛选窗口标题",
        query: r"窗口: 标题正则=^报告[0-9]+",
    },
    Example {
        icon: Icon::Search,
        title: "按正则表达式排除窗口标题",
        query: r"窗口: 标题正则=^报告 排除标题正则=草稿",
    },
    Example {
        icon: Icon::Grid,
        title: "把窗口按程序和标题排序",
        query: "窗口: 排序=进程",
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
        title: "按位宽查看整数的大端和小端字节序",
        query: "进制: 字节序 32 @ 0x12345678",
    },
    Example {
        icon: Icon::Terminal,
        title: "按位宽查看有符号整数的补码",
        query: "进制: 补码 16 @ -257",
    },
    Example {
        icon: Icon::Terminal,
        title: "把大端或小端十六进制字节读成整数",
        query: "进制: 读字节序 小端 @ 78 56 34 12",
    },
    Example {
        icon: Icon::Terminal,
        title: "把大端或小端补码读回有符号整数",
        query: "进制: 读补码 小端 @ FF FE",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计整数绝对值的二进制位数和置位数",
        query: "进制: 位统计 -0xFF00",
    },
    Example {
        icon: Icon::Terminal,
        title: "从非负整数截取一段二进制位",
        query: "进制: 取位 8-15 @ 0x12345678",
    },
    Example {
        icon: Icon::Terminal,
        title: "替换整数中指定范围的二进制位",
        query: "进制: 写位 8-15 @ 0x12345678 @ 0xAB",
    },
    Example {
        icon: Icon::Terminal,
        title: "在指定位宽内向左循环移动整数的二进制位",
        query: "进制: 循环左移 8,3 @ 0xA5",
    },
    Example {
        icon: Icon::Terminal,
        title: "在指定位宽内向右循环移动整数的二进制位",
        query: "进制: 循环右移 8,3 @ 0xA5",
    },
    Example {
        icon: Icon::Terminal,
        title: "在指定位宽内按位倒序",
        query: "进制: 位倒序 8 @ 0x06",
    },
    Example {
        icon: Icon::Terminal,
        title: "把整数编码为指定位宽的格雷码",
        query: "进制: 格雷码 8 @ 10",
    },
    Example {
        icon: Icon::Terminal,
        title: "把二进制格雷码还原为原整数",
        query: "进制: 读格雷码 0b1111",
    },
    Example {
        icon: Icon::Layers,
        title: "预览连续整数、格雷码和变动位",
        query: "进制: 格雷码表 3,2,4",
    },
    Example {
        icon: Icon::Layers,
        title: "查看单精度浮点数的符号、指数和尾数字段",
        query: "进制: 浮点位 32 @ 0.1",
    },
    Example {
        icon: Icon::Layers,
        title: "读取 64 位浮点位模式并查看浮点类别",
        query: "进制: 读浮点位 64 @ 0x8000000000000000",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看十进制数在浮点中实际表示的精确分数",
        query: "进制: 浮点分数 32 @ 0.1",
    },
    Example {
        icon: Icon::Terminal,
        title: "将原始浮点位模式换成精确分数",
        query: "进制: 读浮点分数 32 @ 0xBDCCCCCD",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看相邻的两个可表示浮点数",
        query: "进制: 浮点相邻 32 @ 0.1",
    },
    Example {
        icon: Icon::Terminal,
        title: "从位模式查看上下相邻的浮点数",
        query: "进制: 读浮点相邻 64 @ 0x8000000000000000",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计两个整数相差的二进制位数",
        query: "进制: 汉明距离 0b1010,0b1100",
    },
    Example {
        icon: Icon::Terminal,
        title: "列出两个整数中发生变化的位号",
        query: "进制: 位比对 0xA5,0xAD",
    },
    Example {
        icon: Icon::Layers,
        title: "查看变化位的原值和新值",
        query: "进制: 位比对CSV 0b1010,0b1100",
    },
    Example {
        icon: Icon::Terminal,
        title: "在指定位宽内按字节倒序",
        query: "进制: 字节倒序 32 @ 0x12345678",
    },
    Example {
        icon: Icon::Terminal,
        title: "把工作表的列字母转换为列号",
        query: "进制: 表格列号 AA",
    },
    Example {
        icon: Icon::Terminal,
        title: "在 A1 与 R1C1 格式间转换单元格地址",
        query: "进制: 单元格地址 $B$3",
    },
    Example {
        icon: Icon::Terminal,
        title: "把十进制整数转换为罗马数字",
        query: "进制: 罗马数字 2026",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确换算不同进制的小数",
        query: "进制: 小数 10.625 从 10 到 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "把带括号循环节的小数换回十进制",
        query: "进制: 小数 0.0(0011) 从 2 到 10",
    },
    Example {
        icon: Icon::Terminal,
        title: "把罗马数字转换为十进制整数",
        query: "进制: 罗马数字 CDXCIX",
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
        icon: Icon::Grid,
        title: "再次打开最近使用的应用",
        query: "应用历史:",
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
        title: "用分数精确换算时长单位",
        query: "时长: 精确换算 1秒 to min",
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
        title: "计算两个时刻区间的重叠时长",
        query: "时长: 重叠 23:00 到 01:00 | 00:30 到 02:00",
    },
    Example {
        icon: Icon::Clock,
        title: "按精确分数缩放时长",
        query: "时长: 缩放 01:30:00 @ 2/3",
    },
    Example {
        icon: Icon::Clock,
        title: "按比例分摊一段时长",
        query: "时长: 分摊 01:30:00 @ 1:2",
    },
    Example {
        icon: Icon::Clock,
        title: "就近舍入到指定的时长步长",
        query: "时长: 舍入 01:07:30 @ 15分钟",
    },
    Example {
        icon: Icon::Clock,
        title: "将时长向下取整到指定步长",
        query: "时长: 向下取整 01:07:30 @ 15分钟",
    },
    Example {
        icon: Icon::Clock,
        title: "将时长向上取整到指定步长",
        query: "时长: 向上取整 01:01:00 @ 15分钟",
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
        title: "求一组时长的 75% 分位数",
        query: "时长: 分位数 75% 3s,1s,2s,4s",
    },
    Example {
        icon: Icon::Clock,
        title: "在表格中比较多个时长分位数",
        query: r#"时长: 分位表 ["25%",0.5,"3/4"] 3s,1s,2s,4s"#,
    },
    Example {
        icon: Icon::Clock,
        title: "把同一个时长换算为多个精确单位",
        query: r#"时长: 单位表 ["ms","s","min"] 1秒"#,
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
        title: "计算两个串联电阻的空载分压",
        query: "电路: 分压 12 V @ 1 kΩ,2 kΩ",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算接入电阻负载后的分压",
        query: "电路: 负载分压 12 V @ 1 kΩ,2 kΩ,2 kΩ",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算两个并联电阻的支路分流",
        query: "电路: 分流 6 mA @ 1 kΩ,2 kΩ",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算 RC 时间常数和截止频率",
        query: "电路: RC 1 kΩ,1 μF",
    },
    Example {
        icon: Icon::Terminal,
        title: "估算 RC 充电达到指定比例的时间",
        query: "电路: RC充电 50% @ 1 kΩ,1 μF",
    },
    Example {
        icon: Icon::Terminal,
        title: "估算 RC 放电到指定比例的时间",
        query: "电路: RC放电 25% @ 1 kΩ,1 μF",
    },
    Example {
        icon: Icon::Terminal,
        title: "按截止频率反求 RC 电容",
        query: "电路: RC电容 1 kΩ @ 1 kHz",
    },
    Example {
        icon: Icon::Terminal,
        title: "按截止频率反求 RC 电阻",
        query: "电路: RC电阻 100 nF @ 1 kHz",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算一阶 RC 低通增益和相位",
        query: "电路: RC低通 1 kΩ,1 μF @ (1/(2*pi*0.001)) Hz",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算一阶 RC 高通增益和相位",
        query: "电路: RC高通 1 kΩ,1 μF @ 1 kHz",
    },
    Example {
        icon: Icon::Terminal,
        title: "导出一阶 RC 低通在多个频率的响应",
        query: "电路: RC低通表 1 kΩ,1 μF @ [\"0 Hz\",\"100 Hz\",\"1 kHz\"]",
    },
    Example {
        icon: Icon::Terminal,
        title: "导出一阶 RC 高通在多个频率的响应",
        query: "电路: RC高通表 1 kΩ,1 μF @ [\"0 Hz\",\"100 Hz\",\"1 kHz\"]",
    },
    Example {
        icon: Icon::Terminal,
        title: "按功率和用时计算耗电量",
        query: "电路: 耗电 1.5 kW @ 2h",
    },
    Example {
        icon: Icon::Terminal,
        title: "用电池容量和标称电压估算电能",
        query: "电路: 电池能量 2500 mAh @ 3.7 V",
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
        icon: Icon::Terminal,
        title: "把频率换算为真空中的波长",
        query: "波长: 2.4 GHz to mm",
    },
    Example {
        icon: Icon::Terminal,
        title: "用给定传播速度从波长计算频率",
        query: "波长: 34.3 cm @ 343 m/s to kHz",
    },
    Example {
        icon: Icon::Terminal,
        title: "用频率与波长求传播速度",
        query: "波长: 波速 1 kHz @ 34.3 cm",
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
        title: "按地图比例尺将图上长度换成实际距离",
        query: "比例尺: 1:50000 3.2 cm to km",
    },
    Example {
        icon: Icon::Terminal,
        title: "用音名或 MIDI 编号查询频率",
        query: "音高: A4",
    },
    Example {
        icon: Icon::Terminal,
        title: "用 MIDI 编号查询音名和频率",
        query: "音高: MIDI 69",
    },
    Example {
        icon: Icon::Terminal,
        title: "查找最接近输入频率的音名和音分偏差",
        query: "音高: 442 Hz",
    },
    Example {
        icon: Icon::Terminal,
        title: "把功率比换算为分贝",
        query: "分贝: 功率比 100",
    },
    Example {
        icon: Icon::Terminal,
        title: "把幅度比换算为分贝",
        query: "分贝: 幅度比 10",
    },
    Example {
        icon: Icon::Terminal,
        title: "按分贝反推功率比",
        query: "分贝: 转功率比 20 dB",
    },
    Example {
        icon: Icon::Terminal,
        title: "按分贝反推幅度比",
        query: "分贝: 转幅度比 20 dB",
    },
    Example {
        icon: Icon::Terminal,
        title: "叠加使用同一参考量的多个分贝值",
        query: "分贝: 叠加 60 dB,60 dB",
    },
    Example {
        icon: Icon::Terminal,
        title: "从总分贝值扣除一项功率",
        query: "分贝: 功率差 70 dB @ 60 dB",
    },
    Example {
        icon: Icon::Terminal,
        title: "对等权分贝值求能量平均",
        query: "分贝: 能量平均 60 dB,70 dB",
    },
    Example {
        icon: Icon::Terminal,
        title: "按持续时间加权计算分贝平均",
        query: "分贝: 加权平均 60 dB @ 2秒,70 dB @ 1秒",
    },
    Example {
        icon: Icon::Layers,
        title: "按半径和高求圆柱的表面积与体积",
        query: "立体: 圆柱 3,5",
    },
    Example {
        icon: Icon::Layers,
        title: "按半径和高求圆锥的表面积与体积",
        query: "立体: 圆锥 3,4",
    },
    Example {
        icon: Icon::Layers,
        title: "按两个底面半径和高求圆台的表面积与体积",
        query: "立体: 圆台 2,5,4",
    },
    Example {
        icon: Icon::Layers,
        title: "计算空心圆管的总表面积和材料体积",
        query: "立体: 圆管 3,2,5",
    },
    Example {
        icon: Icon::Layers,
        title: "按球半径和截取高度计算球冠",
        query: "立体: 球冠 3,1",
    },
    Example {
        icon: Icon::Layers,
        title: "按棱长计算正四面体表面积和体积",
        query: "立体: 正四面体 3",
    },
    Example {
        icon: Icon::Layers,
        title: "按半径求球的表面积与体积",
        query: "立体: 球 1/2",
    },
    Example {
        icon: Icon::Layers,
        title: "按边长求立方体的表面积与体积",
        query: "立体: 立方体 1/2",
    },
    Example {
        icon: Icon::Layers,
        title: "按长宽高求长方体的表面积与体积",
        query: "立体: 长方体 2,3,4",
    },
    Example {
        icon: Icon::Clock,
        title: "按每分钟拍数计算一拍时长",
        query: "节拍: 120 BPM",
    },
    Example {
        icon: Icon::Clock,
        title: "用分数表示拍数并计算时长",
        query: "节拍: 时长 120 BPM @ 3/2",
    },
    Example {
        icon: Icon::Clock,
        title: "按一段时长和拍数反推速度",
        query: "节拍: 速度 1.5秒 @ 3",
    },
    Example {
        icon: Icon::Clock,
        title: "按时长和速度计算拍数",
        query: "节拍: 拍数 90秒 @ 120 BPM",
    },
    Example {
        icon: Icon::Terminal,
        title: "按整数半音移动音高",
        query: "音高: 转调 Bb3 @ 2",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算两个音高之间的半音和音分",
        query: "音高: 音程 C4 @ G4",
    },
    Example {
        icon: Icon::Terminal,
        title: "按参考音高计算音分差",
        query: "音高: 音分差 A4 @ 442 Hz",
    },
    Example {
        icon: Icon::Terminal,
        title: "将音高按音分移调并计算频率",
        query: "音高: 音分移调 442 Hz @ -1200",
    },
    Example {
        icon: Icon::Layers,
        title: "用图上和实际距离反求地图比例",
        query: "比例尺: 求 3.2 cm = 1.6 km",
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
        title: "找出每个地址命中的最长前缀",
        query: r#"网段: 归属CSV ["192.0.2.0/24","192.0.2.128/25","2001:db8::/32"] 192.0.2.190,192.0.2.1,192.0.3.1,2001:db8::7"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "查询地址归属并显示匹配网段的名称",
        query: r#"网段: 归属CSV [["192.0.2.0/24","办公"],["192.0.2.128/25","实验室, Ａ"]] 192.0.2.190,192.0.2.1,192.0.3.1"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "查看多个网段的起止地址与精确地址数",
        query: "网段: 清单CSV 192.0.2.190/25,2001:db8::/126",
    },
    Example {
        icon: Icon::Terminal,
        title: "取出网段中指定序号的地址",
        query: "网段: 取地址 192.0.2.137/26,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "查出地址在网段中从一开始的序号",
        query: "网段: 地址序号 192.0.2.137/26,192.0.2.129",
    },
    Example {
        icon: Icon::Terminal,
        title: "向前或向后移动指定数量的相邻网段",
        query: "网段: 偏移 192.0.2.137/26,-2",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接取出父网段中的第几个子网",
        query: "网段: 取子网 192.0.2.137/26,/28,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看子网在同前缀划分中的序号",
        query: "网段: 子网序号 192.0.2.137/26,192.0.2.167/28",
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
        icon: Icon::Terminal,
        title: "合并相邻和重叠的 IPv4 或 IPv6 网段",
        query: "网段: 合并 192.0.2.128/25,192.0.2.0/25,2001:db8::/64,2001:db8:0:1::/64",
    },
    Example {
        icon: Icon::Terminal,
        title: "列出两组 IPv4 或 IPv6 网段共同覆盖的范围",
        query: "网段: 交集 [[\"192.0.2.0/24\",\"2001:db8::/63\"],[\"192.0.2.0/26\",\"192.0.2.128/25\",\"2001:db8:0:1::/64\"]]",
    },
    Example {
        icon: Icon::Terminal,
        title: "从第一组网段扣除第二组覆盖的地址",
        query: "网段: 差集 [[\"192.0.2.0/24\",\"2001:db8::/63\"],[\"192.0.2.0/26\",\"192.0.2.128/25\",\"2001:db8:0:1::/64\"]]",
    },
    Example {
        icon: Icon::Terminal,
        title: "找出两组网段中只被一方覆盖的地址",
        query: r#"网段: 对称差 [["192.0.2.0/25","2001:db8::/63"],["192.0.2.64/26","192.0.2.128/25","2001:db8:0:1::/64"]]"#,
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
        title: "用命名空间和名称生成可重复的 UUIDv5",
        query: "UUID: v5 DNS www.example.com",
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
        title: "检查图书 ISBN 的校验位",
        query: "ISBN: 校验 978-0-11-000222-4",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 10 位图书编号转成 13 位 ISBN",
        query: "ISBN: 转13位 0-8044-2957-X",
    },
    Example {
        icon: Icon::Terminal,
        title: "把以 978 开头的 ISBN 转回 10 位",
        query: "ISBN: 转10位 9780804429573",
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
        title: "把时间戳对齐到最近的时间边界",
        query: "时间戳: 就近对齐 15分钟 @ 1970-01-01T00:22:30Z",
    },
    Example {
        icon: Icon::Clock,
        title: "按固定时间间隔生成一组时刻",
        query: "时间戳: 序列 0 @ 15分钟 @ 3 to UTC+8",
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
        title: "以毫秒精确表示纪元前一纳秒",
        query: "时间戳: -1ns to ms",
    },
    Example {
        icon: Icon::Clock,
        title: "把带小数的毫秒时间戳换回日期",
        query: "时间戳: -0.000001 ms",
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
        title: "查当地日期在 UTC 中的起止时刻",
        query: "时间戳: 日区间 2026-03-08 America/New_York",
    },
    Example {
        icon: Icon::Clock,
        title: "把连续的当地日期映射到 UTC 区间",
        query: "时间戳: 日区间 2026-03-07 到 2026-03-09 America/New_York",
    },
    Example {
        icon: Icon::Clock,
        title: "查看 ISO 周对应的 UTC 时间范围",
        query: "时间戳: 周区间 2026-W10 America/New_York",
    },
    Example {
        icon: Icon::Clock,
        title: "查看当地日历月对应的 UTC 时间范围",
        query: "时间戳: 月区间 2026-03 America/New_York",
    },
    Example {
        icon: Icon::Clock,
        title: "查看当地季度对应的 UTC 时间范围",
        query: "时间戳: 季度区间 2026-Q1 America/New_York",
    },
    Example {
        icon: Icon::Clock,
        title: "查看整个日历年对应的 UTC 时间范围",
        query: "时间戳: 年区间 2024 UTC",
    },
    Example {
        icon: Icon::Clock,
        title: "合并有重叠或相邻的 UTC 时间区间",
        query: r#"时间戳: 区间合并 [["0s","10s"],["10s","20s"],["30s","40s"]]"#,
    },
    Example {
        icon: Icon::Clock,
        title: "求多个时间区间共同覆盖的时段",
        query: r#"时间戳: 区间交集 [["0s","10s"],["5s","15s"],["8s","9s"]]"#,
    },
    Example {
        icon: Icon::Clock,
        title: "从一段时间中扣除多个占用时段",
        query: r#"时间戳: 区间差集 [["0s","20s"],["2s","5s"],["8s","25s"]]"#,
    },
    Example {
        icon: Icon::Clock,
        title: "合计所有时间区间覆盖的实际时长",
        query: r#"时间戳: 覆盖时长 [["0s","10s"],["5s","15s"],["20s","26s"]]"#,
    },
    Example {
        icon: Icon::Clock,
        title: "按当地日期查看两个时刻之间的实际时长",
        query: "时间戳: 逐日时长 2026-03-07T14:00:00-05:00 到 2026-03-09T10:00:00-04:00 to America/New_York",
    },
    Example {
        icon: Icon::Clock,
        title: "查看某年时区偏移变更",
        query: "时间戳: 偏移变更 2026 America/New_York",
    },
    Example {
        icon: Icon::Clock,
        title: "用当地时区逐日列出同一个钟表时刻",
        query: "时间戳: 每日同刻 2026-03-07 02:30 America/New_York @ 3",
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
        icon: Icon::Clock,
        title: "查看某日期是月初或月末数来的第几个工作日",
        query: "日期: 工作日位置 2026-11-04",
    },
    Example {
        icon: Icon::Clock,
        title: "列出连续几个月的最后一个工作日",
        query: "日期: 工作日序列 2026-11,3 @ -1",
    },
    Example {
        icon: Icon::Clock,
        title: "列出连续几个月各自的最后一个周五",
        query: "日期: 星期序列 2026-10,4 @ 最后一个周五",
    },
    Example {
        icon: Icon::Clock,
        title: "查基准日期之后最近的周一",
        query: "日期: 下一个星期 2026-11-21 @ 周一",
    },
    Example {
        icon: Icon::Clock,
        title: "查基准日期之前最近的周四",
        query: "日期: 上一个星期 2024-03-01 @ 周四",
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
        title: "生成连续几个月的月初日期",
        query: "日期: 月初序列 2024-11,3",
    },
    Example {
        icon: Icon::Clock,
        title: "生成连续几个月的月末日期",
        query: "日期: 月末序列 2024-01,3",
    },
    Example {
        icon: Icon::Clock,
        title: "生成连续几个季度的季初日期",
        query: "日期: 季初序列 2024-11-20,3",
    },
    Example {
        icon: Icon::Clock,
        title: "生成连续几个季度的季末日期",
        query: "日期: 季末序列 2024-11-20,3",
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
        icon: Icon::Clock,
        title: "按自然半年拆分区间并导出工作日",
        query: "日期: 按半年拆分CSV 2024-06-29 至 2024-07-02",
    },
    Example {
        icon: Icon::Terminal,
        title: "按自然年汇总日期区间的天数和工作日",
        query: "日期: 按年拆分CSV 2023-12-30 至 2024-01-02",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐日导出星期和工作日标记",
        query: r#"日期: 逐日CSV 2024-02-29 至 2024-03-03 例外 [["2024-03-01"],["2024-03-02"]]"#,
    },
    Example {
        icon: Icon::Clock,
        title: "统计区间内每个星期的天数",
        query: "日期: 星期统计 2024-02-29 至 2024-03-09",
    },
    Example {
        icon: Icon::Clock,
        title: "把日期换成电子表格日期序号",
        query: "日期: 表格序号 2024-02-29",
    },
    Example {
        icon: Icon::Clock,
        title: "把 1904 日期系统的序号换回日期",
        query: "日期: 从表格序号 1904 @ 43889",
    },
    Example {
        icon: Icon::Clock,
        title: "把日期和钟表时刻换算为电子表格序号",
        query: "日期: 表格时刻序号 2024-02-29 12:00",
    },
    Example {
        icon: Icon::Clock,
        title: "把带小数的电子表格序号还原为日期和时刻",
        query: "日期: 从表格时刻序号 45351.5",
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
        title: "按整月和余下天数计算日期差",
        query: "日期: 年月日差 2024-01-31 至 2025-03-02",
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
        title: "将多个倍率相乘",
        query: "prod(1.05,1.02,1.01)",
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
        title: "计算一组数的平方和",
        query: "sumsq(-3,4)",
    },
    Example {
        icon: Icon::Terminal,
        title: "判断两个计算结果是否在容差内",
        query: "isclose(0.1+0.2,0.3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "大数相消时保留浮点加法中的小数值",
        query: "fsum(1e16,1,-1e16)",
    },
    Example {
        icon: Icon::Terminal,
        title: "将乘法与加法合并后进行一次舍入",
        query: "fma(1e308,2,-1e308)",
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
        title: "按根元素字号换算 CSS rem",
        query: "1.25rem @ 16px to px",
    },
    Example {
        icon: Icon::Layers,
        title: "按所填字号换算 CSS em",
        query: "1.5em @ 18px to px",
    },
    Example {
        icon: Icon::Layers,
        title: "按所填行高换算 CSS lh",
        query: "48px @ 24px to lh",
    },
    Example {
        icon: Icon::Layers,
        title: "按视口宽和高换算 CSS vw 和 vh",
        query: "2.5vw @ 1440×900 to px",
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
        icon: Icon::Terminal,
        title: "在牛米与英制扭矩单位之间换算",
        query: "1 lbf·ft to N·m",
    },
    Example {
        icon: Icon::Terminal,
        title: "在勒克斯与英尺烛光之间换算照度",
        query: "100 lx to fc",
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
        title: "在 OKLCH 和 sRGB 颜色之间换算",
        query: "颜色: oklch(70% 25% 180 / 50%)",
    },
    Example {
        icon: Icon::Layers,
        title: "把 sRGB 颜色换算为 OKLab",
        query: "颜色: #f00 to oklab",
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
        title: "在线性光空间混合颜色",
        query: "颜色: color-mix(in srgb-linear, white, black)",
    },
    Example {
        icon: Icon::Layers,
        title: "在 OKLab 空间混合颜色",
        query: "颜色: color-mix(in oklab, #000, #fff)",
    },
    Example {
        icon: Icon::Terminal,
        title: "按 sRGB 等距离生成两种颜色之间的色阶",
        query: "颜色: 色阶 #004080 到 #80C0FF @ 5",
    },
    Example {
        icon: Icon::Layers,
        title: "在线性光强空间生成两种颜色之间的色阶",
        query: "颜色: 线性色阶 #000 到 #fff @ 3",
    },
    Example {
        icon: Icon::Layers,
        title: "在 OKLab 空间生成色阶",
        query: "颜色: OKLab色阶 #000 到 #fff @ 3",
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
        icon: Icon::Clock,
        title: "找回复制或填回结果时使用过的查询",
        query: "查询历史: ",
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
        title: "查看文本原文并保留首尾空白",
        query: "文本: 原文   第一行\n第二行  ",
    },
    Example {
        icon: Icon::Terminal,
        title: "按字素簇、码位和编码分别统计文本长度",
        query: "文本: 长度详情 \"👩‍💻 e\u{0301}\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计字符出现次数并输出 CSV",
        query: "文本: 字频 茶👩‍💻茶👩‍💻🍵",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计按 Unicode 单词边界划分的词频",
        query: "文本: 词频 tea, coffee. tea Tea",
    },
    Example {
        icon: Icon::Terminal,
        title: "显示普通空格、制表符和不换行空格",
        query: "文本: 显示空白 \" 茶\t椅\u{00A0}咖啡 \"",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看字符的码位、字节位置和编码",
        query: "文本: 码位表 \"北\u{200b}😀\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "用 NFKC 统一全角字母和圈号数字的写法",
        query: "文本: Unicode规范化 NFKC ＡＢＣ①",
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
        title: "按行去重并保留每种原文最后一次出现的位置",
        query: "文本: 按行去重留末 茶\n咖啡\n茶\n水\n茶",
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
        title: "找出多行共有的开头",
        query: "文本: 公共前缀 Report-01\nReport-02\nReport-10",
    },
    Example {
        icon: Icon::Terminal,
        title: "找出多行共有的结尾",
        query: "文本: 公共后缀 image-01.png\nphoto-02.png\nscan-03.png",
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
        title: "指定每段连续空行的保留上限",
        query: "文本: 空行上限 2 标题\n\n\n\n正文\n附注",
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
        title: "按行把两段文本配成两列 CSV",
        query: r#"文本: 逐行配对 "苹果\n梨\n茶" "3\n2""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把多段文本逐行配对成多列 CSV",
        query: r#"文本: 逐行配对 ["苹果\n梨\n茶","3\n2","箱\n袋"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按固定列数把连续文本行分组为 CSV",
        query: "文本: 按行分组CSV 3 甲\n乙\n丙\n丁",
    },
    Example {
        icon: Icon::Layers,
        title: "按字符宽度拆分每行并预览 CSV",
        query: "文本: 定宽转CSV [3,2]\n001北京备注\n002上海\n003",
    },
    Example {
        icon: Icon::Layers,
        title: "逐行查看字符数与编码长度",
        query: "文本: 行长度CSV\n北京\nÁ😀\n\n茶 ",
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
        title: "保留词语匹配行和前后的文本",
        query: "文本: 匹配上下文 [\"错误\",1,1] 准备\n请求\n错误 A\n恢复\n结束",
    },
    Example {
        icon: Icon::Terminal,
        title: "导出匹配行及上下文的原行号",
        query: "文本: 匹配上下文CSV [\"错误\",1,1] 准备\n请求\n错误,重试\n恢复\n结束",
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
        title: "取出多行文本开头的指定行数",
        query: "文本: 取开头行 2 甲\n乙\n丙",
    },
    Example {
        icon: Icon::Terminal,
        title: "取出多行文本末尾的指定行数",
        query: "文本: 取末尾行 2 甲\n乙\n丙",
    },
    Example {
        icon: Icon::Terminal,
        title: "从指定行开始等间隔取行",
        query: "文本: 间隔取行 2,3 标题\n甲\n备注\n分隔\n乙\n备注\n分隔\n丙",
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
        title: "按模板填入每行的序号和原文",
        query: "文本: 逐行模板 \"- {序号}. {行}\" 苹果\n梨",
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
        title: "逐行截取第几个字符到第几个字符",
        query: "文本: 逐行截取 2-3 A👩‍💻茶Z\n甲乙丙丁\n北",
    },
    Example {
        icon: Icon::Terminal,
        title: "依次清理、去重和自然排序文本行",
        query: "文本: 依次处理 [\"逐行去空白\",\"去空行\",\"按行去重\",\"自然排序\"] \" report10 \nreport2\n\nreport2\n\"",
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
        title: "导出文本差异与原行号、新行号",
        query: r#"文本: 逐行比较CSV "苹果\n梨\n茶" "苹果\n柿子\n茶""#,
    },
    Example {
        icon: Icon::Layers,
        title: "预览文本变更及指定数量的上下文",
        query: r#"文本: 差异上下文CSV [1,1] "远处\n表头\n旧值\n保留\n较远" "远处\n表头\n新值\n保留\n较远""#,
    },
    Example {
        icon: Icon::Layers,
        title: "查看文本连续变更的起始行和行数",
        query: r#"文本: 差异块CSV "表头\n旧值\n保留\n删掉" "新增\n表头\n新值\n保留""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "计算两段文字在字素簇层面的编辑距离",
        query: r#"文本: 编辑距离 "猫👩‍💻" "猫🧑‍💻""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "找出两段文字中最长的连续公共片段",
        query: r#"文本: 公共片段 "北京报告2024.txt" "上海报告2024.csv""#,
    },
    Example {
        icon: Icon::Terminal,
        title: "保留符合正则表达式的行",
        query: "文本: 正则保留行 \"^[0-9]+$\" 42\nA3\n007",
    },
    Example {
        icon: Icon::Terminal,
        title: "按正则提取匹配行及前后的文本",
        query: "文本: 正则上下文 [\"^WARN\",0,1] INFO 开始\nWARN 重试\n请求完成\nINFO 结束",
    },
    Example {
        icon: Icon::Terminal,
        title: "将正则命中和邻近行导出为 CSV",
        query: "文本: 正则上下文CSV [\"^WARN\",0,1] INFO 开始\nWARN 重试\n请求完成\nINFO 结束",
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
        icon: Icon::Layers,
        title: "统计每种正则捕获值出现的次数",
        query: r#"文本: 正则频数CSV status "状态=(?<status>[A-Z]+)" 状态=B 状态=A 状态=B 状态=C"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把每次正则匹配的捕获组提取为 CSV",
        query: r#"文本: 正则取表 "(?<id>[0-9]+):([^;]+)" 001:茶,咖啡;002:梨"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "导出正则捕获组及其在原文中的起始位置",
        query: "文本: 正则定位取表 \"ID=(?<id>[0-9]+)\" INFO ID=001\n跳过\nID=007",
    },
    Example {
        icon: Icon::Terminal,
        title: "用正则表达式替换文本",
        query: r#"文本: 正则替换 "([0-9]{4})/([0-9]{2})/([0-9]{2})" "$1-$2-$3" 2026/11/21"#,
    },
    Example {
        icon: Icon::Layers,
        title: "预览正则每处匹配与展开后的替换值",
        query: concat!(
            r#"文本: 正则替换明细CSV "ID=(?<id>[0-9]+)" "编号:${id}" 北京 ID=007"#,
            "\n🫖 ID=9",
        ),
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
        title: "提取 Markdown 井号标题和原文行号",
        query: "文本: Markdown标题CSV # 采购清单\n\n## 茶,咖啡\n~~~text\n# 这行是代码\n~~~\n### 数量 ###",
    },
    Example {
        icon: Icon::Terminal,
        title: "格式化 JSON",
        query: r#"JSON: 格式化 {"名称":"北京","值":1}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "校验 TOML 配置并统计顶层键",
        query: "TOML: 校验 name = '茶'\nport = 8000",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位 TOML 配置中的重复键",
        query: "TOML: 校验\nname = '茶'\nname = '咖啡'",
    },
    Example {
        icon: Icon::Terminal,
        title: "按路径取出 TOML 配置中的数组元素",
        query: "TOML: 取值 /server/ports/1\n[server]\nports = [8000, 8080]",
    },
    Example {
        icon: Icon::Terminal,
        title: "将 TOML 配置转为可复制的 JSON",
        query: "TOML: 转JSON date = 2024-02-29\nnumber = 9_007_199_254_740_993",
    },
    Example {
        icon: Icon::Terminal,
        title: "用表格查看 TOML 路径、类型和数值",
        query: "TOML: 叶子CSV date = 2024-02-29\n[\"a/b\"]\nnote = '茶,咖啡'\nport = 8000",
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
        title: "用通配路径取出每条订单的金额",
        query: r#"JSON: 通配取值 ["订单",null,"金额"] {"订单":[{"金额":1.20},{"状态":"草稿"},{"金额":9007199254740993}]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "在表格中查看通配匹配的完整路径和原值",
        query: r#"JSON: 通配路径CSV ["订单",null,"金额"] {"订单":[{"金额":1.20},{"状态":"草稿"},{"金额":null},{"金额":9007199254740993}]}"#,
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
        title: "统计 JSON 嵌套深度和各类型的值",
        query: r#"JSON: 结构统计 {"订单":[{"金额":1},{"金额":null}],"完成":true}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "读取 JSON 指针指向的值类型",
        query: r#"JSON: 类型 /订单/0/金额 {"订单":[{"金额":9007199254740993}]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "递归移除 JSON 对象中的空值字段",
        query: r#"JSON: 递归去空字段 {"备注":null,"订单":[{"金额":1.50,"备注":null},null],"确认":false}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把数组展开为 JSONL",
        query: r#"JSON: 展开数组 [{"城市":"北京"},{"城市":"上海"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按下标和步长从 JSON 数组取项",
        query: r#"JSON: 切片 [1,4,2] ["茶",1.20e+2,"米",9007199254740993,"梨"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把 JSONL 合并成数组",
        query: "JSON: 合并行 {\"城市\":\"北京\"}\n{\"城市\":\"上海\"}",
    },
    Example {
        icon: Icon::Terminal,
        title: "把逐行 JSON 对象预览为 CSV 表格",
        query: "JSON: 行转CSV {\"编号\":\"001\",\"金额\":1.50}\n{\"编号\":\"002\",\"备注\":\"茶,咖啡\"}",
    },
    Example {
        icon: Icon::Terminal,
        title: "校验 JSON 并定位出错的字符",
        query: "JSON: 校验\n{\n  \"项目\": \"茶\"坏\n}",
    },
    Example {
        icon: Icon::Terminal,
        title: "校验 JSON Lines 并指出每个无效行",
        query: "JSON: 校验行 {\"编号\":1}\n错误文本\n{\"编号\":\nnull",
    },
    Example {
        icon: Icon::Terminal,
        title: "表格预览逐行 JSON 的类型和语法错误",
        query: "JSON: 校验行CSV\n{\"编号\":1}\n\n错误文本\n{\"编号\":\nnull",
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
        title: "在表格中查看 JSON 的修改、移除和新增",
        query: r#"JSON: 对比CSV {"a/b":[1.00,null],"旧":2e0} {"a/b":[1,true],"新":null}"#,
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
        title: "为 JSON 行数组指定表头并转成 CSV",
        query: r#"JSON: 行数组转CSV ["名称","金额"] [["茶,咖啡",1.50],["梨",null]]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按 JSON 指针把嵌套字段导出为 CSV",
        query: r#"JSON: 路径转CSV ["/订单/商品","/订单/金额"] [{"订单":{"商品":"茶,咖啡","金额":1.50}},{"订单":{"商品":"梨"}}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把逐行 JSON 的指定路径提取为 CSV 列",
        query: "JSON: 行路径转CSV [\"/订单/商品\",\"/订单/金额\"]\n{\"订单\":{\"商品\":\"茶,咖啡\",\"金额\":1.50}}\n{\"订单\":{\"商品\":\"梨\"}}",
    },
    Example {
        icon: Icon::Terminal,
        title: "按逐行 JSON 的字段值计数",
        query: "JSON: 行分组计数CSV /金额\n{\"金额\":1.50}\n{\"金额\":\"1.5\"}\n{\"金额\":15e-1}\n{\"金额\":null}",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算每组逐行 JSON 记录的精确合计和平均",
        query: "JSON: 行分组统计CSV [\"/组\",\"/金额\"]\n{\"组\":1.00,\"金额\":1.50}\n{\"组\":\"北\",\"金额\":2}\n{\"组\":1e0,\"金额\":0.75}",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算逐行 JSON 各组的精确中位数",
        query: "JSON: 行分组中位数CSV [\"/组\",\"/金额\"]\n{\"组\":1.00,\"金额\":1.50}\n{\"组\":\"北\",\"金额\":4}\n{\"组\":1e0,\"金额\":2}\n{\"组\":\"北\",\"金额\":1}\n{\"组\":\"北\",\"金额\":9}",
    },
    Example {
        icon: Icon::Terminal,
        title: "筛选同时满足条件的逐行 JSON",
        query: "JSON: 行筛选 [[\"/状态\",\"=\",\"完成\"],[\"/金额\",\">=\",1.5]]\n{\"编号\":\"001\",\"状态\":\"完成\",\"金额\":1.50}\n{\"编号\":\"002\",\"状态\":\"待办\",\"金额\":2}\n{\"编号\":\"003\",\"状态\":\"完成\",\"金额\":1.49}",
    },
    Example {
        icon: Icon::Terminal,
        title: "筛选满足任一条件的逐行 JSON",
        query: "JSON: 行任一筛选 [[\"/状态\",\"=\",\"完成\"],[\"/金额\",\">=\",1.5]]\n{\"编号\":\"001\",\"状态\":\"完成\",\"金额\":1.50}\n{\"编号\":\"002\",\"状态\":\"待办\",\"金额\":2}\n{\"编号\":\"003\",\"状态\":\"待办\",\"金额\":1.49}",
    },
    Example {
        icon: Icon::Terminal,
        title: "按多个字段保留每组逐行 JSON 的首条记录",
        query: "JSON: 行路径去重 [\"/编号\",\"/件数\"]\n{\"编号\":\"001\",\"件数\":1.00,\"状态\":\"首次\"}\n{\"编号\":\"001\",\"件数\":1e0,\"状态\":\"重复\"}\n{\"编号\":\"001\",\"件数\":\"1\",\"状态\":\"字符串\"}",
    },
    Example {
        icon: Icon::Terminal,
        title: "保留每组 JSONL 的最后一条",
        query: "JSON: 行路径去重留末 [\"/编号\"]\n{\"编号\":\"001\",\"金额\":1.00}\n{\"编号\":\"002\",\"金额\":9}\n{\"编号\":\"001\",\"金额\":1.20}",
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
        title: "按键名查找 JSON 中所有匹配的指针",
        query: r#"JSON: 找键 "编号" {"订单":[{"编号":1},{"编号":2}],"编号":"总表"}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按精确的 JSON 标量值查找指针",
        query: r#"JSON: 找值 1.5 {"金额":[1.50,"1.5",1.5e0]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "在表格中查看精确匹配的 JSON 数值",
        query: r#"JSON: 找值CSV 1.5 {"金额":[1.50,"1.5",1.5e0]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "在表格中查看匹配键名的路径和原值",
        query: r#"JSON: 找键CSV "金额" {"金额":1.50,"订单":[{"金额":2}]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "定位 JSON 中同一对象内重复的键",
        query: r#"JSON: 重复键 {"订单":[{"价格":1,"价格":2},{"价格":3}]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "保留 JSON 中每个重复键的首次出现",
        query: r#"JSON: 去重复键 留首 {"状态":"草稿","编号":1,"状态":"完成"}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "保留 JSON 中每个重复键的最后一次出现",
        query: r#"JSON: 去重复键 留末 {"状态":"草稿","编号":1,"状态":"完成"}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "用 JSON 指针作键展平数据",
        query: r#"JSON: 展平 {"订单":[{"a/b":9007199254740993},{}]}"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "把 JSON 标量和空容器的指针与类型导出为 CSV",
        query: r#"JSON: 叶子CSV {"金额":[1.20e+2,null],"a/b":{},"清单":[]}"#,
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
        title: "按多个 JSON 路径上的值组合计数",
        query: r#"JSON: 分组计数 ["/地区","/状态"] [{"地区":"北","状态":"完成"},{"地区":"南","状态":"完成"},{"地区":"北","状态":"完成"}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定 JSON 路径分组保留完整记录",
        query: r#"JSON: 分组明细 /状态 [{"状态":"完成","金额":1.50},{"状态":"待办","金额":2e0},{"状态":"完成","金额":3}]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "按多个 JSON 路径分组留存记录",
        query: r#"JSON: 分组明细 ["/地区","/状态"] [{"地区":"北","状态":"完成","金额":1.50},{"地区":"南","状态":"完成","金额":2e0},{"地区":"北","状态":"完成","金额":3}]"#,
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
        title: "保留每组 JSON 的末项",
        query: r#"JSON: 多路径去重留末 ["/地区","/月份"] [{"地区":"北","月份":1.00,"编号":"001"},{"地区":"南","月份":1},{"地区":"北","月份":1e0,"编号":"002"}]"#,
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
        title: "定位制表符分隔数据中列数不一致的记录",
        query: "CSV: 校验TSV 编号\t说明\n001\t\"茶\n咖啡\"\n002\t梨\t额外",
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
        title: "按列预览 CSV 并保留完整原文",
        query: "CSV: 表格预览 编号,金额,备注\n001,3.50,\"甲,乙\"\n002,2,\"带\n换行\"",
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
        title: "把分号分隔的表格转换成逗号分隔",
        query: "CSV: 分隔符转换 [\";\",\",\"] 编号;金额;备注\n001;1,50;\"茶;浓\"\n002;;待确认",
    },
    Example {
        icon: Icon::Terminal,
        title: "识别分号并把表格转成逗号分隔",
        query: "CSV: 自动转CSV 编号;金额;备注\n001;1,50;\"茶;浓\"\n002;;待确认",
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
        title: "按数据行和列名取一个 CSV 单元格",
        query: "CSV: 取单元格 [2,\"备注\"] 编号,备注\n001,茶\n002,\" 梨,苹果\n第二行 \"\n003,",
    },
    Example {
        icon: Icon::Terminal,
        title: "从 CSV 表格末尾倒数取单元格",
        query: "CSV: 取单元格 [-1,\"编号\"] 编号,备注\n001,茶\n002,\"梨\n苹果\"\n003,咖啡",
    },
    Example {
        icon: Icon::Layers,
        title: "按行范围和列顺序提取 CSV 区域",
        query: "CSV: 取区域 [\"2-3\",[\"备注\",1]] 编号,名称,备注\n001,茶,甲\n002,梨,\"乙,丙\"\n003,米,丁",
    },
    Example {
        icon: Icon::Layers,
        title: "按行号和列名修改一个 CSV 单元格",
        query: "CSV: 改单元格 [2,\"备注\",\"已核对\"] 编号,备注\n001,原样\n002,待核对\n003,原样",
    },
    Example {
        icon: Icon::Layers,
        title: "取 CSV 中各组最后一条完整记录",
        query: "CSV: 分组取行 [[\"组\"],-1] 组,编号,备注\nA,001,甲\nB,002,乙\nA,003,\"跨\n行\"\nA,004,丁",
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
        title: "用下方最近的值向上填充 CSV 空值",
        query: "CSV: 向上填充 [\"地区\",2] 地区,月份,编号\n,,001\n华北,,002\n,2月,003\n华南,,004\n,,005",
    },
    Example {
        icon: Icon::Terminal,
        title: "按组用精确线性插值填充 CSV 空值",
        query: "CSV: 插值填充 [\"金额\",[\"地区\"]] 地区,金额\n北,1/3\n南,8\n北,\n北,\n北,4/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计 CSV 各列的空单元格",
        query: "CSV: 空值统计 [\"备注\",1] 编号,备注\n001,\n,\" \"\n002,茶",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计各列的空值、不同值和重复值种数",
        query: "CSV: 列值概览 [\"备注\",1] 编号,备注\n001,\"茶,咖啡\"\n1,\"茶,咖啡\"\n001,\" \"\n,\"\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位数值列里的文本或空单元格",
        query: "CSV: 检查数值列 [\"金额\"] 编号,金额\n001,1.50\n002,待确认\n003,\n004,1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位日期列中的无效日期",
        query: "CSV: 检查日期列 [\"日期\"] 编号,日期\n001,2024-02-29\n002,2023-02-29\n003,\n004,2024-W09-4",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定日期格式检查表格",
        query: "CSV: 检查日期格式 [[\"日期\"],\"%d/%m/%Y\"] 编号,日期\nA,29/02/2024\nB,29/02/2023\nC,2024-02-29",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位文字长度超过限制的单元格",
        query: "CSV: 检查长度范围 [[\"备注\"],[\"UTF-8 字节\",0,5]] 编号,备注\nA,茶\nB,北京\nC,",
    },
    Example {
        icon: Icon::Terminal,
        title: "检查 CSV 表头的列名和顺序",
        query: "CSV: 检查表头 [\"编号\",\"金额\",\"备注\"] 金额,编号\n29,001",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位不符合正则规则的单元格",
        query: "CSV: 检查正则列 [[\"编号\"],\"^[A-Z][0-9]{2}$\"] 编号,状态\nA01,通过\nB7,待复核\n,未知\nC23,完成",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位超出数值范围的单元格",
        query: "CSV: 检查数值范围 [[\"比率\"],[\"1/4\",\"3/4\"]] 编号,比率\nA,1/4\nB,0.750\nC,0.751\nD,未填",
    },
    Example {
        icon: Icon::Terminal,
        title: "同时检查表格中的多条规则",
        query: "CSV: 检查规则 [[\"日期\",\"日期\"],[\"编号\",\"正则\",\"^A[0-9]+$\"],[3,\"数值范围\",\"0\",\"1\"]] 日期,编号,比率\n2024-02-29,A01,1\n2023-02-29,B02,1.01",
    },
    Example {
        icon: Icon::Terminal,
        title: "保留通过所有规则的行",
        query: "CSV: 规则保留行 [[\"日期\",\"日期\"],[\"编号\",\"正则\",\"^A[0-9]+$\"]] 日期,编号\n2024-02-29,A01\n2023-02-29,B02",
    },
    Example {
        icon: Icon::Terminal,
        title: "保留有规则失败的行",
        query: "CSV: 规则问题行 [[\"日期\",\"日期\"],[\"编号\",\"正则\",\"^A[0-9]+$\"]] 日期,编号\n2024-02-29,A01\n2023-02-29,B02",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计每条 CSV 规则的通过和失败数",
        query: "CSV: 规则统计 [[1,\"数值\"],[\"比率\",\"数值范围\",\"0\",\"1\"]] 比率\n1/3\n1.2\n未知",
    },
    Example {
        icon: Icon::Layers,
        title: "为原表每行追加通过全部规则的标记",
        query: "CSV: 规则标记列 [\"通过\",[[\"日期\",\"日期\"],[\"编号\",\"正则\",\"^A[0-9]+$\"]]] 日期,编号\n2024-02-29,A01\n2023-02-29,B02",
    },
    Example {
        icon: Icon::Layers,
        title: "为原表每行追加未通过的规则数",
        query: "CSV: 规则失败数列 [\"失败数\",[[\"日期\",\"日期\"],[\"编号\",\"正则\",\"^A[0-9]+$\"]]] 日期,编号\n2024-02-29,A01\n2023-02-29,B02",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位列表之外的单元格值",
        query: "CSV: 检查允许值 [[\"状态\"],[\"待办\",\"完成\",\"\"]] 编号,状态\nA,待办\nB,待确认\nC,\nD,完成",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位列中无效的 JSON 值",
        query: "CSV: 检查JSON列 [\"数据\"] 编号,数据\nA,null\nB,\"{\"\"x\"\":1}\"\nC,True\nD,",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位缺少偏移或无效的时间戳",
        query: "CSV: 检查时间戳列 [\"时间\"] 时间\n-0.000001ms\n2026-11-01T01:30:00-04:00\n2026-11-01 01:30 America/New_York\n0.1ns",
    },
    Example {
        icon: Icon::Terminal,
        title: "定位超出时间范围的单元格",
        query: "CSV: 检查时间范围 [[\"时间\"],[\"0\",\"1ns\"]] 时间\n1970-01-01T08:00:00+08:00\n0.000001ms\n2ns\n-1ns",
    },
    Example {
        icon: Icon::Terminal,
        title: "把选中的 CSV 数值列统一成固定小数位",
        query: "CSV: 小数列 [[\"金额\",3],2] 编号,金额,比率\n001,2.675,1/8\n002,,-0.005",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 CSV 数值列按指定有效数字写成科学记数法",
        query: "CSV: 有效数字列 [[2,\"比率\"],3] 编号,金额,比率\n001,9.995,1/3\n002,-1.245,0.0012345\n003,0,",
    },
    Example {
        icon: Icon::Terminal,
        title: "用工程计数法显示所选列",
        query: "CSV: 工程计数列 [[\"金额\",3],3] 编号,金额,比率\n001,9.995,0.0012345\n002,-999.5,1/3\n003,,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "移除所有单元格都为空的 CSV 数据行",
        query: "CSV: 去空行 编号,备注\n001,茶\n\n,\n002,梨",
    },
    Example {
        icon: Icon::Terminal,
        title: "移除所有数据单元格都为空的 CSV 列",
        query: "CSV: 去空列 编号,空备注,状态\n001,,0\n002,,完成",
    },
    Example {
        icon: Icon::Terminal,
        title: "筛出金额和日期均已填写的记录",
        query: "CSV: 完整行 [\"金额\",\"日期\"] 编号,金额,日期\n001,0,2024-02-29\n002,1/2, \n003,,2026-11-23",
    },
    Example {
        icon: Icon::Terminal,
        title: "找出指定列仍有空白的记录",
        query: "CSV: 缺失行 [\"金额\",\"日期\"] 编号,金额,日期\n001,0,2024-02-29\n002,1/2, \n003,,2026-11-23",
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
        title: "从 CSV 指定列提取第一个正则捕获组",
        query: "CSV: 正则提取列 [\"省份\",\"地址\",\"^(?P<province>[^省]+)省\",\"province\"]\n地址,编号\n江苏省南京,1\n上海市,2\n浙江省杭州,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "把每个单元格首次正则匹配的多个捕获组追加为新列",
        query: "CSV: 正则提取多列 [\"原文\",\"(?<key>[A-Z])(?:=([0-9]+))?\",[[\"标记\",\"key\"],[\"编号\",\"2\"]]]\n原文,备注\nA=007 B=99,第一条\nB,第二条\n略过,空结果",
    },
    Example {
        icon: Icon::Terminal,
        title: "按每个正则匹配展开为一行",
        query: "CSV: 正则展开行 [\"编号\",\"原文\",\"[A-Z](?:=([0-9]+))?\",\"1\"]\n原文,标签\nA=007 B=99,甲\nB,乙\n无,丙",
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
        title: "用单列公式逐行求平方",
        query: "CSV: 公式列 [\"平方\",\"数量\",\"x**2\"]\n数量\n1/3\n-3",
    },
    Example {
        icon: Icon::Terminal,
        title: "将一列数据代入计算器函数并追加近似值",
        query: "CSV: 函数列 [\"余弦\",\"角度\",\"cosd(x)\"]\n角度,编号\n0,001\n90,002\n,003\n180,004",
    },
    Example {
        icon: Icon::Terminal,
        title: "用两列数据计算近似函数值",
        query: "CSV: 函数列 [\"加权\",\"角度\",\"权重\",\"cosd(x)*y\"]\n角度,权重\n0,2\n180,3\n90,",
    },
    Example {
        icon: Icon::Terminal,
        title: "按照计算器函数的结果筛选原始行",
        query: "CSV: 函数筛选 [[\"角度\",\"权重\"],\"cosd(x)*y\",\">\",\"0\"]\n编号,角度,权重\n001,0,2\n002,180,3\n003,90,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "按目标值和绝对容差保留 CSV 原始行",
        query: "CSV: 函数近似筛选 [[2],\"x+0.2\",\"0.3\",\"1e-9\"]\n编号,数值\n001,0.1\n002,0.11\n003,\n004,0.10",
    },
    Example {
        icon: Icon::Terminal,
        title: "按函数计算值排序原始行",
        query: "CSV: 函数排序 [[\"角度\",\"权重\"],\"cosd(x)*y\",\"数值降序\"]\n编号,角度,权重\n001,0,3\n002,180,2\n003,360,3\n004,,9",
    },
    Example {
        icon: Icon::Terminal,
        title: "按函数值生成并列名次并保留行顺序",
        query: "CSV: 函数排名列 [\"名次\",[\"角度\",\"权重\"],\"cosd(x)*y\",\"数值降序\"]\n编号,角度,权重\n001,0,3\n002,180,2\n003,360,3\n004,,9",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计函数值的范围、均值、中位数和空值个数",
        query: "CSV: 函数摘要 [[\"角度\",\"权重\"],\"cosd(x)*y\"]\n角度,权重\n0,2\n180,3\n90,4\n,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "先按部门分组，再汇总每组的函数值",
        query: "CSV: 函数分组摘要 [[\"部门\"],[\"数量\",\"单价\"],\"x*y\"]\n部门,数量,单价\n甲,1,5\n乙,2,3\n甲,4,5\n乙,,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计 CSV 函数值落入各区间的次数和频率",
        query: "CSV: 函数分箱 [[1,2],\"x*y\",\"0,1/2,2\"]\n量,系数\n-1,2\n0,1\n1/2,1\n2,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "用分箱图查看 CSV 函数值的区间次数",
        query: "CSV: 函数分箱图 [[1,2],\"x*y\",\"0,1/2,2\"]\n量,系数\n-1,2\n0,1\n1/2,1\n2,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "按精确数值比较两列并标记是否达标",
        query: "CSV: 数值比较列 [\"达标\",\"实测\",2,\">=\"] 实测,门槛\n1.50,3/2\n1,2\n,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "按箱线图内围栏标记 CSV 数值离群值",
        query: "CSV: 离群列 [\"离群\",\"金额\"] 编号,金额\n001,100\n002,1\n003,2\n004,3\n005,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 CSV 中混用的容量单位换算为同一单位",
        query: "CSV: 数据换算列 [\"容量(KiB)\",\"容量\",\"KiB\"]\n文件,容量\n甲,9007199254740993 B\n乙,1/3 KiB\n丙,",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 CSV 温度列逐行换算为摄氏度",
        query: "CSV: 单位换算列 [\"摄氏\",\"温度\",\"°C\"]\n编号,温度\n001,32 °F\n002,212 °F\n003,",
    },
    Example {
        icon: Icon::Terminal,
        title: "为 CSV 中只有数字的长度列指定源单位",
        query: "CSV: 单位换算列 [\"厘米\",\"长度\",\"m\",\"cm\"]\n编号,长度\n001,1.5\n002,1/4\n003,",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐行把整数换算成指定进制",
        query: "CSV: 进制列 [\"十进制\",\"编号\",16,10] 项目,编号\n甲,ff\n乙,-a_b\n丙,20_0000_0000_0001\n丁,",
    },
    Example {
        icon: Icon::Terminal,
        title: "批量换算有限小数和循环小数的进制",
        query: "CSV: 进制小数列 [\"二进制\",\"值\",10,2]\n编号,值\n001,10.625\n002,0.1\n003,0.(9)\n004,",
    },
    Example {
        icon: Icon::Clock,
        title: "给 CSV 追加两列日期相差的天数",
        query: "CSV: 日期差列 [\"间隔天数\",\"开始\",\"结束\"]\n开始,结束\n2024-02-28,2024-03-01\n2024-03-03,2024-03-01",
    },
    Example {
        icon: Icon::Clock,
        title: "给 CSV 追加两个钟表时刻之间的时长",
        query: "CSV: 时刻差列 [\"经过时长\",\"开始\",\"结束\"]\n开始,结束\n23:45,01:15\n09:00:00.25,09:00:01",
    },
    Example {
        icon: Icon::Terminal,
        title: "用带时区的时间戳求每行实际时长",
        query: "CSV: 时间戳差列 [\"时长\",\"开始\",\"结束\"] 开始,结束\n2026-03-08T01:30:00-05:00,2026-03-08T03:30:00-04:00\n1ns,-1ns",
    },
    Example {
        icon: Icon::Layers,
        title: "按设备计算相邻有效时间戳的间隔",
        query: "CSV: 组内时间戳差列 [\"间隔\",\"时刻\",[\"设备\"]] 设备,时刻\nA,2026-03-08T01:30:00-05:00\nB,2ns\nA,\nA,2026-03-08T03:30:00-04:00\nB,1ns",
    },
    Example {
        icon: Icon::Layers,
        title: "计算每行距本组首个有效时刻的时长",
        query: "CSV: 组内时间戳偏移列 [\"偏移\",\"时刻\",[\"设备\"]] 设备,时刻\nA,3s\nB,10ns\nA,8s\nA,\nB,5ns\nA,1s",
    },
    Example {
        icon: Icon::Layers,
        title: "根据每组记录的时间间隔划分会话",
        query: "CSV: 会话编号列 [\"会话\",\"时刻\",[\"设备\"],\"30分钟\"] 设备,时刻\nA,0s\nB,0s\nA,1800s\nA,\nA,3601s\nB,120s",
    },
    Example {
        icon: Icon::Layers,
        title: "分组时间窗口计数",
        query: "CSV: 时间窗口计数列 [\"次数\",\"时刻\",[\"设备\"],\"30分钟\"] 设备,时刻\nA,1800s\nB,0s\nA,0s\nA,1800s\nA,3601s\nB,120s",
    },
    Example {
        icon: Icon::Layers,
        title: "分组时间窗口合计",
        query: "CSV: 时间窗口合计列 [\"合计\",\"时刻\",\"金额\",[\"设备\"],\"2ns\"] 设备,时刻,金额\nA,3ns,\nB,4ns,\nA,1ns,1/3\nA,3ns,0.5\nA,4ns,-0.5",
    },
    Example {
        icon: Icon::Layers,
        title: "分组时间窗口平均",
        query: "CSV: 时间窗口平均列 [\"平均\",\"时刻\",\"金额\",[\"设备\"],\"2ns\"] 设备,时刻,金额\nA,3ns,\nB,4ns,\nA,1ns,1/3\nA,3ns,0.5\nA,4ns,-0.5",
    },
    Example {
        icon: Icon::Layers,
        title: "查看每组最早最晚的时刻及实际跨度",
        query: "CSV: 分组时间戳摘要 [[\"设备\"],\"时刻\"] 设备,时刻\nA,2026-03-08T03:30:00-04:00\nB,2ns\nC,\nA,2026-03-08T01:30:00-05:00\nB,1ns",
    },
    Example {
        icon: Icon::Layers,
        title: "按实际时刻取各组最早的一条原记录",
        query: "CSV: 分组最早时间行 [[\"设备\"],\"时刻\"] 设备,时刻,标签\nA,2026-03-08T03:30:00-04:00,较晚\nB,1ns,首次\nA,2026-03-08T01:30:00-05:00,较早\nB,1970-01-01T00:00:00.000000001Z,同一时刻",
    },
    Example {
        icon: Icon::Layers,
        title: "按实际时刻取各组最晚的一条原记录",
        query: "CSV: 分组最晚时间行 [[\"设备\"],\"时刻\"] 设备,时刻,标签\nA,2026-03-08T03:30:00-04:00,较晚\nB,1ns,较早\nC,,无时刻\nA,2026-03-08T01:30:00-05:00,较早\nB,2ns,较晚",
    },
    Example {
        icon: Icon::Terminal,
        title: "将 CSV 的时间戳列换算到同一时区",
        query: "CSV: 时区换算列 [\"UTC时间\",\"时间\",\"UTC\"] 编号,时间\nA,1970-01-01T08:00:00+08:00\nB,-1ns",
    },
    Example {
        icon: Icon::Clock,
        title: "将每行时间戳对齐到固定的时间区间边界",
        query: "CSV: 时间戳对齐列 [\"区间开始\",\"时刻\",\"15分钟\",\"向下\"] 编号,时刻\nA,-1s\nB,1970-01-01T08:17:30+08:00\nC,900s\nD,",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐行将数值对齐到精确步长的倍数",
        query: "CSV: 数值对齐列 [\"对齐值\",\"金额\",\"1/4\",\"就近\"] 编号,金额\n001,-3/8\n002,0.625\n003,9007199254740993.125\n004,",
    },
    Example {
        icon: Icon::Clock,
        title: "按表格分组累计不同写法的实际时长",
        query: "CSV: 分组时长合计 [\"人员\"] \"时长\" 人员,时长\n北,01:30\n南,PT45S\n北,30.25秒\n空,\n南,-15秒\n负,-1ns",
    },
    Example {
        icon: Icon::Clock,
        title: "按目标单位给表格追加精确时长数值",
        query: "CSV: 时长数值列 [\"分钟\",\"时长\",\"min\"] 编号,时长\n001,1秒\n002,PT30S\n003,-0.25秒",
    },
    Example {
        icon: Icon::Terminal,
        title: "按地址在 CSV 中追加最长匹配网段",
        query: "CSV: 网段归属列 [\"网段\",\"地址\",[\"192.0.2.0/24\",\"192.0.2.128/25\",\"2001:db8::/32\"]] 编号,地址\n001,192.0.2.190\n002,2001:db8::7\n003,192.0.3.1",
    },
    Example {
        icon: Icon::Terminal,
        title: "按地址在 CSV 中追加匹配的网段名称",
        query: "CSV: 网段名称列 [\"名称\",\"地址\",[[\"192.0.2.0/24\",\"办公\"],[\"192.0.2.128/25\",\"实验室, Ａ\"]]] 编号,地址\n001,192.0.2.190\n002,192.0.2.1\n003,192.0.3.1",
    },
    Example {
        icon: Icon::Clock,
        title: "用 CSV 中每行的天数推算日期",
        query: "CSV: 日期加天列 [\"到期日期\",\"日期\",\"天数\"]\n日期,天数\n2024-02-28,2\n2024-03-01,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "按每行的整月数推算新的日期",
        query: "CSV: 日期加月列 [\"到期日期\",\"日期\",\"月数\"]\n日期,月数\n2024-01-31,1\n2024-03-31,-1\n2023-01-31,1\n2024-02-29,12",
    },
    Example {
        icon: Icon::Terminal,
        title: "跳过周末，按每行的工作日数推算日期",
        query: "CSV: 日期加工作日列 [\"到期日期\",\"日期\",\"工作日数\"]\n日期,工作日数\n2024-03-01,1\n2024-03-04,-1\n2024-03-02,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "把一列日期换成表格日期序号",
        query: "CSV: 表格序号列 [\"序号\",\"日期\",1900]\n日期,编号\n2024-02-29,001\n,002\n2024-W09-5,003",
    },
    Example {
        icon: Icon::Terminal,
        title: "把一列表格日期序号换回日期",
        query: "CSV: 序号转日期列 [\"日期\",1,1904]\n序号\n0\n39267",
    },
    Example {
        icon: Icon::Layers,
        title: "把一列日期和时刻换成表格时刻序号",
        query: "CSV: 表格时刻序号列 [\"序号\",\"时刻\",1904]\n设备,时刻\nA,1904-01-01 06:00\nB,\nC,1904-01-01 00:00:00.000000001",
    },
    Example {
        icon: Icon::Layers,
        title: "从一列表格时刻序号还原日期和时刻",
        query: "CSV: 序号转日期时刻列 [\"时刻\",2,1900]\n设备,序号\nA,45351.5\nB,",
    },
    Example {
        icon: Icon::Terminal,
        title: "以自定休息日和补班日推算 CSV 到期日期",
        query: "CSV: 日期加工作日列 [\"到期日期\",\"日期\",\"工作日数\",[[\"2024-03-04\"],[\"2024-03-02\"]]]\n日期,工作日数\n2024-03-01,1\n2024-03-01,2\n2024-03-05,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 CSV 中的日期按指定格式追加为一列",
        query: "CSV: 日期格式列 [\"年月\",\"日期\",\"%Y年%m月\"]\n日期,编号\n2024-02-29,甲\n2024-W09-5,乙\n2024年1月31日,丙",
    },
    Example {
        icon: Icon::Terminal,
        title: "把指定格式的 CSV 日期统一输出为 ISO 日期",
        query: "CSV: 日期解析列 [\"标准日期\",\"原日期\",\"%d/%m/%Y\"]\n原日期,编号\n31/01/2024,甲\n29/02/2024,乙\n,丙",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐行统计 CSV 单元格的字素长度",
        query: "CSV: 长度列 [\"长度\",\"文本\",\"字素\"]\n编号,文本\n1,👩‍💻\n2,e\u{301}\n3,",
    },
    Example {
        icon: Icon::Terminal,
        title: "为表格新增统一 Unicode 写法的文本列",
        query: "CSV: Unicode规范化列 [\"标准名\",\"名称\",\"NFKC\"]\n编号,名称\n001,ＡＢＣ①\n002,",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐行比较两个 CSV 文本列的编辑距离",
        query: "CSV: 编辑距离列 [\"距离\",\"原文\",\"现文\"]\n编号,原文,现文\n001,kitten,sitting\n002,猫👩‍💻,猫🧑‍💻\n003,,茶",
    },
    Example {
        icon: Icon::Terminal,
        title: "把两个文本列逐行共有的最长片段写入新列",
        query: "CSV: 公共片段列 [\"片段\",\"原名\",\"现名\"]\n编号,原名,现名\n001,北京报告2024.txt,上海报告2024.csv\n002,ab-cd,cd+ab\n003,,茶",
    },
    Example {
        icon: Icon::Terminal,
        title: "按字素位置截取 CSV 单元格的内容",
        query: "CSV: 截取列 [\"摘要\",\"文本\",\"2-3\"]\n编号,文本\n1,A👩‍💻e\u{301}Z\n2,北\n3,",
    },
    Example {
        icon: Icon::Terminal,
        title: "给 CSV 编号左侧补零至四位",
        query: "CSV: 左填充列 [\"新编号\",\"编号\",4,\"0\"]\n行,编号\n1,7\n2,12345\n3,",
    },
    Example {
        icon: Icon::Terminal,
        title: "给 CSV 单元格右侧填充字符",
        query: "CSV: 右填充列 [\"对齐\",\"编号\",3,\"·\"]\n行,编号\n1,甲\n2,乙丙",
    },
    Example {
        icon: Icon::Terminal,
        title: "替换 CSV 单元格中的指定文字并保留原列",
        query: "CSV: 替换列 [\"新地址\",\"地址\",\"旧路\",\"新路\"]\n编号,地址\n1,旧路12号旧路口\n2,北街\n3,",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计 CSV 单元格中正则表达式的匹配次数",
        query: "CSV: 正则计数列 [\"次数\",\"备注\",\"[0-9]+\"]\n编号,备注\n1,A12 B34 C5\n2,无编号\n3,",
    },
    Example {
        icon: Icon::Terminal,
        title: "用正则捕获组逐行转换 CSV 单元格",
        query: "CSV: 正则替换列 [\"日期\",\"原值\",\"([0-9]{4})/([0-9]{2})/([0-9]{2})\",\"$1-$2-$3\"]\n编号,原值\n1,2026/11/21\n2,待确认",
    },
    Example {
        icon: Icon::Terminal,
        title: "从 CSV 单元格的 JSON 中提取指定字段",
        query: "CSV: JSON取值列 [\"城市\",\"数据\",\"/城市\"]\n编号,数据\n1,\"{\"\"城市\"\":\"\"北京\"\"}\"\n2,\"{\"\"状态\"\":true}\"\n3,",
    },
    Example {
        icon: Icon::Terminal,
        title: "用通配路径从每个 JSON 单元格提取数组",
        query: "CSV: JSON通配列 [\"金额\",\"详情\",[\"订单\",null,\"金额\"]]\n编号,详情\n001,\"{\"\"订单\"\":[{\"\"金额\"\":1.20},{\"\"金额\"\":null}]}\"\n002,{}\n003,",
    },
    Example {
        icon: Icon::Terminal,
        title: "从 CSV 的网址列逐行提取查询参数",
        query: "CSV: URL参数列 [\"标签\",\"网址\",\"tag\"] 编号,网址\n1,https://example.invalid/?tag=a+1&tag=b%2Bc\n2,https://example.invalid/?other=tea\n3,",
    },
    Example {
        icon: Icon::Terminal,
        title: "检查 CSV 单元格中 JSON 字段的类型",
        query: "CSV: JSON类型列 [\"类型\",\"数据\",\"/金额\"]\n编号,数据\n1,\"{\"\"金额\"\":1.50}\"\n2,\"{\"\"金额\"\":\"\"1.50\"\"}\"\n3,\"{\"\"金额\"\":null}\"\n4,{}",
    },
    Example {
        icon: Icon::Terminal,
        title: "统计 CSV 单元格中 JSON 数组或对象的成员数",
        query: "CSV: JSON长度列 [\"项数\",\"数据\",\"/商品\"]\n编号,数据\n1,\"{\"\"商品\"\":[\"\"茶\"\",\"\"梨\"\"]}\"\n2,\"{\"\"商品\"\":{}}\"\n3,\"{\"\"商品\"\":\"\"茶\"\"}\"\n4,{}",
    },
    Example {
        icon: Icon::Terminal,
        title: "将 CSV 单元格中 JSON 字段是否存在写成一列",
        query: "CSV: JSON存在列 [\"有金额\",\"数据\",\"/金额\"]\n编号,数据\n1,\"{\"\"金额\"\":null}\"\n2,{}\n3,\n4,\"{\"\"金额\"\":0}\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 CSV 单元格中的 JSON 数组展开为多行",
        query: "CSV: JSON展开行 [\"商品\",\"数据\",\"/商品\"]\n订单,数据\n001,\"{\"\"商品\"\":[\"\"茶\"\",\"\"梨\"\"]}\"\n002,\"{\"\"商品\"\":[]}\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "按部门逐行累计 CSV 金额",
        query: "CSV: 累计合计列 [\"累计金额\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/3\n南,2\n北,0.5",
    },
    Example {
        icon: Icon::Terminal,
        title: "追加每个组的相邻金额差",
        query: "CSV: 差分列 [\"变化\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/3\n南,2\n北,0.5\n南,3/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "追加每组金额的相邻变化率",
        query: "CSV: 变化率列 [\"变化率\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/3\n南,2\n北,0.5\n南,3/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "追加每个组最近两项的移动平均",
        query: "CSV: 移动平均列 [\"近两笔\",\"金额\",[\"部门\"],2]\n部门,金额\n北,1/3\n南,2\n北,0.5\n南,3/2\n北,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "追加最近两项的移动合计",
        query: "CSV: 移动合计列 [\"近两笔\",\"金额\",[],2]\n金额\n1/3\n1/2\n2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按组追加最近两笔数值的样本方差",
        query: "CSV: 移动样本方差列 [\"方差\",\"金额\",[\"部门\"],2]\n部门,金额\n北,1/3\n南,3\n北,4/3\n南,7\n北,10/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "把每个窗口的总体标准差追加为一列",
        query: "CSV: 移动总体标准差列 [\"标准差\",\"金额\",[],2]\n金额\n1/3\n4/3\n10/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "按部门计算最近两笔金额的分位数",
        query: "CSV: 移动分位数列 [\"P25\",\"金额\",[\"部门\"],2,\"25%\"]\n部门,金额\n北,4\n南,8\n北,1\n南,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "给每个部门追加精确数值排名",
        query: "CSV: 排名列 [\"名次\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/2\n南,2\n北,3/2\n北,0.50\n南,1\n北,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "给整列追加从小到大的排名",
        query: "CSV: 升序排名列 [\"名次\",\"金额\",[]]\n金额\n1/2\n0.50\n3/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "并列值后按连续名次排名",
        query: "CSV: 升序密集排名列 [\"名次\",\"金额\",[]]\n金额\n1/2\n0.50\n3/2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按组追加精确的百分位排名",
        query: "CSV: 百分位排名列 [\"相对位置\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/2\n南,2\n北,3/2\n北,0.50\n北,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看同组小于或等于本行的数值所占比例",
        query: "CSV: 累积分布列 [\"累计比例\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/2\n南,2\n北,3/2\n北,0.50\n北,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算每行金额占本部门合计的比例",
        query: "CSV: 占比列 [\"占比\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/3\n南,2\n北,2/3\n南,6",
    },
    Example {
        icon: Icon::Terminal,
        title: "在每行追加同一部门和月份的记录数",
        query: "CSV: 组内计数列 [\"记录数\",[\"部门\",\"月份\"]]\n部门,月份,编号\n北,11,001\n南,11,002\n北,12,003\n北,11,004",
    },
    Example {
        icon: Icon::Terminal,
        title: "按原表顺序给各组记录编号",
        query: "CSV: 组内序号列 [\"序号\",[\"部门\",\"月份\"]]\n部门,月份,编号\n北,11,001\n南,11,002\n北,12,003\n北,11,004",
    },
    Example {
        icon: Icon::Terminal,
        title: "按相邻行的相同内容给连续段编号",
        query: "CSV: 连续分组列 [\"分组\",[\"部门\"]]\n部门,金额\n北,1\n北,2\n南,3\n北,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "为每段相同的相邻行从头编号",
        query: "CSV: 段内序号列 [\"位置\",[\"部门\"]]\n部门,金额\n北,1\n北,2\n南,3\n北,4",
    },
    Example {
        icon: Icon::Layers,
        title: "给每行追加所属连续段的总记录数",
        query: "CSV: 段内计数列 [\"段长\",[\"部门\"]]\n部门,金额\n北,1\n北,2\n南,3\n北,4",
    },
    Example {
        icon: Icon::Layers,
        title: "预览每段相邻记录的起止行号与数量",
        query: "CSV: 连续段摘要 [\"部门\"]\n部门,金额\n北,1\n北,2\n南,3\n北,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "从同组前一行取值并追加到 CSV",
        query: "CSV: 前值列 [\"上期\",2,1,[\"部门\"]] 部门,编号\n研发,001\n销售,101\n研发,002\n销售,\n研发,003",
    },
    Example {
        icon: Icon::Terminal,
        title: "从同组后一行取值并追加到 CSV",
        query: "CSV: 后值列 [\"下期\",2,1,[\"部门\"]] 部门,编号\n研发,001\n销售,101\n研发,002\n销售,\n研发,003",
    },
    Example {
        icon: Icon::Terminal,
        title: "按组逐行求 CSV 的累计平均值",
        query: "CSV: 累计平均列 [\"累计平均\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/3\n南,2\n北,0.5",
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
        title: "按指定的 CSV 列保留每组最后一条记录",
        query: "CSV: 按列去重留末 [\"编号\",2] 编号,地区,备注\n001,北,早期\n002,南,保留\n001,北,更新",
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
        title: "按精确数值保留 CSV 每组最后一条记录",
        query: "CSV: 数值去重留末 [\"金额\"] 编号,金额\n001,1.50\n002,2\n003,3/2\n004,1.5e0",
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
        title: "跨列查找同时包含多个关键词的记录",
        query: "CSV: 搜索行 [\"north\",\"茶\"] 编号,区域,品名\n001,North,绿茶\n002,South,绿茶\n003,North,咖啡\n004,ＮＯＲＴＨ,\"茶,咖啡\"",
    },
    Example {
        icon: Icon::Terminal,
        title: "查找指定列包含任一关键词的记录",
        query: "CSV: 搜索任一 {\"词\":[\"north\",\"茶\"],\"列\":[\"地区\",\"商品\"]} 编号,地区,商品,备注\n001,ＮＯＲＴＨ,咖啡,茶\n002,南,绿茶,米\n003,南,咖啡,茶\n004,North,茶,冰",
    },
    Example {
        icon: Icon::Terminal,
        title: "排除指定列含有任一关键词的记录",
        query: "CSV: 排除关键词 {\"词\":[\"north\",\"茶\"],\"列\":[\"地区\",\"商品\"]} 编号,地区,商品,备注\n001,ＮＯＲＴＨ,咖啡,茶\n002,南,绿茶,米\n003,南,咖啡,茶\n004,North,茶,冰",
    },
    Example {
        icon: Icon::Terminal,
        title: "查找关键词在表格中的行号、列号和原文",
        query: "CSV: 命中位置 [\"north\",\"茶\"] 区域,备注\nＮｏｒｔｈ,茶 North\n南区,米",
    },
    Example {
        icon: Icon::Terminal,
        title: "只在指定表头列中搜索关键词",
        query: "CSV: 搜索行 {\"词\":[\"茶\"],\"列\":[\"备注\"]} 区域,备注,货品\n北区,咖啡,茶\n南区,茶,米",
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
        title: "按出现次数排序，汇总各个状态的次数与精确占比",
        query: "CSV: 分组频数 [\"状态\"] 编号,状态\n001,完成\n002,待办\n003,完成\n004,\n005,完成",
    },
    Example {
        icon: Icon::Terminal,
        title: "绘制 CSV 中最常见的类别及其次数",
        query: "CSV: 分组频数图 [\"状态\"] 编号,状态\n001,完成\n002,待办\n003,完成\n004,\n005,完成\n006,待办",
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
        title: "按组求拟合直线的斜率、截距和 R²",
        query: "CSV: 分组拟合 [[\"地区\"],[\"x\",\"y\"]] 地区,x,y\n北,0,1/2\n南,2,1\n北,1,3/2\n南,2,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "在原表逐行追加同组拟合值和残差",
        query: "CSV: 拟合列 [[\"区域\"],[\"x\",\"y\"],[\"预测\",\"残差\"]] 区域,x,y\n北,0,0\n北,1,1\n北,2,1\n北,3,",
    },
    Example {
        icon: Icon::Terminal,
        title: "直接使用 CSV 指定的两列绘制散点图",
        query: "CSV: 散点图 [\"时间\",\"温度\"] 编号,时间,温度\n001,0,1/2\n002,1,2\n003,2,\n004,3,3/2\n005,4,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "沿横坐标顺序连起完整样点并查看原始分数",
        query: "CSV: 折线图 [\"时间\",\"高度\"] 编号,时间,高度\n003,3,2\n001,0,-1/2\n002,2,\n004,1,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "对表格里的分段直线求带符号的梯形积分",
        query: "CSV: 梯形积分 [\"时间\",\"高度\"] 时间,高度\n3,2\n0,-1\n2,\n1,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "按横坐标间距求采样值的精确加权平均",
        query: "CSV: 梯形平均 [\"时间\",\"高度\"] 时间,高度\n0,0\n1,2\n3,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按分段直线求样点与横轴之间的总面积",
        query: "CSV: 梯形面积 [\"时间\",\"高度\"] 时间,高度\n0,-1\n3,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确插值计算指定区间的梯形积分",
        query: "CSV: 梯形积分 [[\"时间\",\"高度\"],[\"1/2\",\"5/2\"]] 时间,高度\n3,2\n0,-2\n2,\n1,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "在原表追加从最小横坐标开始的累计梯形积分",
        query: "CSV: 梯形累计列 [\"积分\",[\"时间\",\"高度\"]] 时间,高度\n3,2\n0,-1\n2,\n1,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "为各完整样点追加与前一完整样点的差商",
        query: "CSV: 差商列 [\"斜率\",[\"时间\",\"高度\"]] 时间,高度\n3,3\n0,0\n2,\n1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按横坐标窗口为不等距样点追加加权平均",
        query: "CSV: 时间窗平均列 [\"窗均值\",[\"时间\",\"高度\"],\"3/2\"] 时间,高度\n3,4\n0,0\n2,\n1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "在原表追加向前回溯窗口内的精确积分",
        query: "CSV: 时间窗积分列 [\"窗积分\",[\"时间\",\"高度\"],\"3/2\"] 时间,高度\n3,4\n0,0\n2,\n1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "按指定横坐标查询分段直线上的精确插值",
        query: "CSV: 线性插值 [[\"时间\",\"高度\"],[\"1/2\",2]] 时间,高度\n3,3\n0,0\n2,\n1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "查询分段直线在指定横坐标两侧的精确斜率",
        query: "CSV: 曲线斜率 [[\"时间\",\"高度\"],[0,\"1/2\",1,3]] 时间,高度\n3,3\n0,0\n2,\n1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "导出相邻样点之间各段的范围、斜率和截距",
        query: "CSV: 分段直线 [\"时间\",\"高度\"] 时间,高度\n3,3\n0,-1/2\n2,\n1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "查找指定区间内的曲线极值和连续平坦段",
        query: "CSV: 曲线极值 [[\"时间\",\"高度\"],[\"1/2\",3]] 时间,高度\n4,2\n2,2\n0,0\n3,1\n1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "保留指定区间内的样点及两个精确端点",
        query: "CSV: 曲线截取 [[\"时间\",\"高度\"],[\"1/2\",\"5/2\"]] 时间,高度\n3,3\n1,2\n2,\n0,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "把指定区间内的纵坐标升降幅度相加",
        query: "CSV: 曲线变差 [[\"时间\",\"高度\"],[\"1/2\",\"5/2\"]] 时间,高度\n3,0\n1,2\n2,\n0,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "求各段的欧氏长度并保留精确根式",
        query: "CSV: 曲线长度 [\"x\",\"y\"] x,y\n0,0\n1,1\n2,0\n5,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "按横坐标间距求分段直线的精确均方根",
        query: "CSV: 曲线均方根 [\"时间\",\"高度\"] 时间,高度\n3,3\n0,0\n2,",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算曲线在区间内围绕均值的标准差",
        query: "CSV: 曲线标准差 [\"时间\",\"高度\"] 时间,高度\n3,3\n0,0\n2,",
    },
    Example {
        icon: Icon::Terminal,
        title: "按横坐标跨度求曲线高度的分位数",
        query: "CSV: 曲线分位数 [[\"时间\",\"高度\"],\"25%\"] 时间,高度\n7,2\n3,2\n6,\n1,0\n0,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "将间距不等的完整样点重采样到等距坐标",
        query: "CSV: 等距插值 [[\"时间\",\"高度\"],5] 时间,高度\n3,3\n0,0\n2,\n1,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "找出分段直线与目标水平线的精确交点",
        query: "CSV: 水平交点 [[\"时间\",\"高度\"],0] 时间,高度\n4,0\n0,0\n1,0\n2,2\n3,-2",
    },
    Example {
        icon: Icon::Terminal,
        title: "合计曲线严格高于给定值时的横坐标跨度",
        query: "CSV: 超阈跨度 [[\"时间\",\"高度\"],\"1/2\"] 时间,高度\n6,-1\n2,1\n4,1\n3,\n0,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "求曲线超出阈值部分的精确面积",
        query: "CSV: 超阈面积 [[\"时间\",\"高度\"],\"1/2\"] 时间,高度\n6,-1\n2,1\n4,1\n3,\n0,-1",
    },
    Example {
        icon: Icon::Terminal,
        title: "列出曲线严格高于阈值的各个区间",
        query: "CSV: 超阈区间 [[\"时间\",\"高度\"],0] 时间,高度\n6,-1\n1,0\n4,0\n3,0\n2,2\n5,1\n0,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "在共同横坐标区间对齐两条曲线并计算差值",
        query: "CSV: 曲线差值 [[\"时间\",\"实测\",\"基准\"],\"差值\"] 时间,实测,基准\n5,,10\n0,0,\n2,4,\n1,,3\n4,4,\n3,,6",
    },
    Example {
        icon: Icon::Terminal,
        title: "在共同区间绘制两条采样曲线并查看精确差值",
        query: "CSV: 曲线对比图 [[\"时间\",\"实测\",\"基准\"],\"差值\"] 时间,实测,基准\n5,,10\n0,0,\n2,4,\n1,,3\n4,4,\n3,,6",
    },
    Example {
        icon: Icon::Terminal,
        title: "查询两条采样曲线的交点和重合区间",
        query: "CSV: 曲线交点 [\"时间\",\"实测\",\"基准\"] 时间,实测,基准\n4,1,1\n0,0,0\n1,1,1\n2,2,\n3,0,1\n5,1,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "精确计算共同区间内两条曲线之间的总面积",
        query: "CSV: 曲线间面积 [\"时间\",\"实测\",\"基准\"] 时间,实测,基准\n3,0,3\n0,0,0\n1,2,",
    },
    Example {
        icon: Icon::Terminal,
        title: "对两条曲线的差值求带符号的积分",
        query: "CSV: 曲线差积分 [\"时间\",\"实测\",\"基准\"] 时间,实测,基准\n3,0,3\n0,0,0\n1,2,",
    },
    Example {
        icon: Icon::Terminal,
        title: "按横坐标绘制 CSV 的累计梯形积分",
        query: "CSV: 梯形累计图 [\"时间\",\"高度\"] 时间,高度\n3,2\n0,-1\n2,\n1,0",
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
        title: "逐个统计 CSV 两个分类的交叉组合",
        query: "CSV: 透视计数 [\"地区\",\"状态\"] 地区,状态\n南,\"待办,稍后\"\n北,完成\n南,完成\n南,完成",
    },
    Example {
        icon: Icon::Terminal,
        title: "把 CSV 的原始字段按行列组合转成交叉表",
        query: "CSV: 透视原值 [\"地区\",\"月份\",\"编号\"] 地区,月份,编号\n北,一月,001\n南,二月,待补\n北,二月,002",
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
        title: "按编号一次对比状态和金额两列",
        query: r#"CSV: 对比列 [["编号"],["状态","金额"]] ["编号,状态,金额\n001,草稿,1.50\n002,完成,3","金额,编号,状态\n1.5,001,完成\n4,003,待办"]"#,
    },
    Example {
        icon: Icon::Layers,
        title: "按编号自动对比两张表的每个数据列",
        query: r#"CSV: 对比全部列 ["编号"] ["编号,状态,金额\n001,草稿,1.50\n002,完成,3","金额,编号,状态\n1.5,001,完成\n4,003,待办"]"#,
    },
    Example {
        icon: Icon::Terminal,
        title: "查看两张 CSV 的列位置和表头增删",
        query: r#"CSV: 表头对比 ["编号,状态,金额","金额,编号,备注"]"#,
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
        title: "按 UTC 时刻排列 CSV 数据行",
        query: "CSV: 时间戳排序 [\"时间\"] 编号,时间\nA,1970-01-01T08:00:00+08:00\nB,1ns\nC,0\nD,-0.000001ms",
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
        icon: Icon::Layers,
        title: "先按组名再按实际时刻排列 CSV",
        query: "CSV: 多列排序 [[\"组\",\"文本升序\"],[\"时刻\",\"时间戳升序\"]] 编号,组,时刻\nA,组10,0\nB,组2,1ns\nC,组2,1970-01-01T01:00:00+01:00\nD,组2,0",
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
        icon: Icon::Layers,
        title: "直接查看指定 CSV 数值列的区间次数图",
        query: "CSV: 列分箱图 [\"金额\",\"0,1/2,2\"] 编号,金额\n001,-1\n002,0\n003,0.50\n004,1\n005,2",
    },
    Example {
        icon: Icon::Terminal,
        title: "同时查看多列的数值缺失情况和精确摘要",
        query: "CSV: 数值列摘要 [\"金额\",\"系数\"] 编号,金额,系数\n001,1/2,1.50\n002,待定,6/4\n003,2,",
    },
    Example {
        icon: Icon::Terminal,
        title: "从数值和权重两列求精确加权平均",
        query: "CSV: 加权平均 [\"成绩\",\"权重\"] 项目,成绩,权重\n试卷,80,30%\n实验,90,70%",
    },
    Example {
        icon: Icon::Terminal,
        title: "分组查看权重合计和精确加权平均",
        query: "CSV: 分组加权平均 [[\"区域\"],\"成绩\",\"权重\"] 区域,成绩,权重\n南,80,30%\n北,12,2\n南,90,70%\n东,6,0",
    },
    Example {
        icon: Icon::Layers,
        title: "按地区和状态预览权重合计及精确占比",
        query: "CSV: 分组加权频数 [[\"地区\",2],\"权重\"] 地区,状态,权重\n北,完成,0.5\n南,完成,3/2\n北,完成,25%\n西,待办,0\n北,待办,",
    },
    Example {
        icon: Icon::Layers,
        title: "选择 CSV 数值和权重列并预览多个分位数",
        query: "CSV: 加权分位表 [\"金额\",\"权重\",[\"25%\",0.5,\"3/4\"]] 编号,金额,权重\n001,1,1\n002,2,1\n003,4,50%\n004,4,3/2\n005,,9",
    },
    Example {
        icon: Icon::Terminal,
        title: "分组查看有效数值、缺失值与精确平均值",
        query: "CSV: 分组数值摘要 [\"地区\"] \"金额\" 地区,金额\n南,1/2\n北,待定\n南,2\n北,",
    },
    Example {
        icon: Icon::Layers,
        title: "按组预览金额的精确分位数",
        query: "CSV: 分组分位表 [[\"地区\"],\"金额\",[\"25%\",\"50%\"]] 地区,金额\n南,1/2\n北,10\n南,2\n西,",
    },
    Example {
        icon: Icon::Terminal,
        title: "按列名查看箱线图，跳过空白单元格",
        query: "CSV: 列箱线图 \"金额\" 编号,金额\nA,1\nB,2\nC,\nD,3\nE,4\nF,8",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看金额列的累计频率并跳过空白",
        query: "CSV: 列累计分布图 \"金额\" 编号,金额\nA,1/2\nB,0.5\nC,\nD,3\nE,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "查看选定 CSV 数值列两两之间的精确相关系数",
        query: "CSV: 相关矩阵 [\"x\",\"y\",\"z\",\"固定\"] x,y,z,固定\n-1,2,1,5\n0,0,-2,5\n1,-2,1,5\n8,,9,6",
    },
    Example {
        icon: Icon::Terminal,
        title: "用选定坐标计算 CSV 各行之间的欧氏距离",
        query: "CSV: 距离矩阵 [\"编号\",[\"x\",\"y\"]] 编号,x,y\n001,0,0\n002,3/5,4/5\n003,1,1",
    },
    Example {
        icon: Icon::Terminal,
        title: "按精确欧氏距离列出距离基准点最近的行",
        query: "CSV: 最近点 [\"编号\",[\"x\",\"y\"],\"001\",3] 编号,x,y\n001,0,0\n002,2,2\n003,1,0\n004,0,-1\n005,1/3,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算所选 CSV 列的精确样本协方差矩阵",
        query: "CSV: 协方差矩阵 [\"x\",\"y\",\"固定\"] x,y,固定\n-1,2,5\n0,0,5\n1,-2,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算所选 CSV 列的精确总体协方差矩阵",
        query: "CSV: 总体协方差矩阵 [1,2] x,y\n-1,2\n0,0\n1,-2",
    },
    Example {
        icon: Icon::Terminal,
        title: "比较多列顺序的斯皮尔曼秩相关，等值取平均名次",
        query: "CSV: 秩相关矩阵 [\"次序\",\"评分\",\"反向\"] 次序,评分,反向\n0,0,2\n1,1.0,1\n2,1/1,0",
    },
    Example {
        icon: Icon::Terminal,
        title: "用热图看正负相关，查看常数列与零相关",
        query: "CSV: 相关图 [\"x\",\"y\",\"z\",\"固定\"] x,y,z,固定\n-1,2,1,5\n0,0,-2,5\n1,-2,1,5\n8,,9,6",
    },
    Example {
        icon: Icon::Terminal,
        title: "筛选记录后查看秩相关热图",
        query: "处理: [\"CSV: 数值筛选 1 >= 0\",\"CSV: 秩相关图 [1,2,3]\"] 次序,评分,反向\n-1,20,2\n0,0,2\n1,1.0,1\n2,1/1,0",
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
        title: "从同一基准目录生成多条相对路径",
        query: r#"路径: 相对路径 ["C:\\工作\\资料","C:\\工作\\资料\\表.csv","C:\\工作\\发布\\报告.txt"]"#,
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
        icon: Icon::Terminal,
        title: "校验 Windows 文件名并显示长度",
        query: "路径: 校验文件名 报告集.txt",
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
        title: "查看网址参数的表格并区分空值和无等号参数",
        query: "URL: 参数CSV https://example.invalid/?q=%E8%8C%B6+1&q=b%2Bc&flag&flag=",
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
        title: "比较两个网址新增和移除的查询参数",
        query: r#"URL: 参数差异 ["https://example.invalid/a?page=1&tag=a+b&flag","https://example.invalid/b?tag=new&tag=a%20b&flag=&page=2&tag=new"]"#,
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
        title: "保留原值和顺序并修改 URL 参数名",
        query: r#"URL: 改参数名 ["tag","topic"] https://example.invalid/doc?tag=a%20b&source=x&%74ag=b+c&tag#part"#,
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
        title: "按主机统计逐行填写的网址",
        query: "URL: 主机统计CSV https://EXAMPLE.invalid/a\nhttps://example.invalid:8443/b\nhttp://api.example.invalid/c",
    },
    Example {
        icon: Icon::Browser,
        title: "按协议、主机和端口统计网址的源",
        query: "URL: 源统计CSV https://EXAMPLE.invalid:443/a\nhttps://example.invalid/b\nhttps://example.invalid:8443/c\nhttp://example.invalid/d",
    },
    Example {
        icon: Icon::Browser,
        title: "判断两个完整网址是否同源",
        query: "URL: 同源 [\"HTTPS://EXAMPLE.invalid:443/a?x=1\",\"https://example.invalid/b#p\"]",
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
        title: "在普通算式中计算相对变化",
        query: "relchange(200,150)",
    },
    Example {
        icon: Icon::Terminal,
        title: "在精确算式中计算相对变化",
        query: "分数: 计算 relchange(1/3,1/2)",
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
        title: "精确计算同一百分比重复变动后的值",
        query: "百分比: 重复变化 100 @ +10%,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "由重复百分比变动后的结果反推初值",
        query: "百分比: 重复原值 1331/10 @ +10%,3",
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
        title: "按有效数字输出科学计数法",
        query: "分数: 有效数字 9.995 to 3",
    },
    Example {
        icon: Icon::Terminal,
        title: "按有效数字输出工程计数法",
        query: "分数: 工程计数法 0.000012345 to 4",
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
        title: "计算精确数值区间的交集",
        query: "区间: 交集 [1/2,3); (2,5]",
    },
    Example {
        icon: Icon::Terminal,
        title: "按端点是否包含合并数值区间",
        query: "区间: 合并 [0,1); [1,2]; (3,4)",
    },
    Example {
        icon: Icon::Terminal,
        title: "从第一个区间扣除其余区间",
        query: "区间: 差集 [0,10]; [2,4]; (6,8)",
    },
    Example {
        icon: Icon::Terminal,
        title: "合并重叠区间后计算总覆盖长度",
        query: "区间: 长度 [0,3]; [2,5]; [8,9]",
    },
    Example {
        icon: Icon::Terminal,
        title: "保留只属于其中一组的区间，计算对称差",
        query: "区间: 对称差 [0,3]; [8,9] @ [2,5]",
    },
    Example {
        icon: Icon::Terminal,
        title: "检查一个数是否属于任一给定区间",
        query: "区间: 包含 1/2 @ [0,1/2); (2,4]",
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
        title: "求精确分数的整数次方根",
        query: "分数: 计算 root(-125/343,3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "比较多个精确分数取最小值",
        query: "分数: 计算 min(1/3,0.3334,-1/7)",
    },
    Example {
        icon: Icon::Terminal,
        title: "比较超出浮点精度的大整数取最大值",
        query: "分数: 计算 max(9007199254740992,9007199254740993)",
    },
    Example {
        icon: Icon::Terminal,
        title: "把精确数值限制在给定区间内",
        query: "分数: 计算 clamp(0.334,0,1/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "把精确分数向下取整",
        query: "分数: 计算 floor(-7/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "把精确分数向上取整",
        query: "分数: 计算 ceil(7/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "截去精确分数的小数部分",
        query: "分数: 计算 trunc(-7/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "求多个分数或小数的精确合计",
        query: "分数: 计算 sum(1e-3,1/4,3/4)",
    },
    Example {
        icon: Icon::Terminal,
        title: "将多个分数相乘",
        query: "分数: 计算 prod(1/3,1/2,5/7)",
    },
    Example {
        icon: Icon::Terminal,
        title: "求多个正分数的精确调和平均值",
        query: "分数: 计算 hmean(1/3,1/6)",
    },
    Example {
        icon: Icon::Terminal,
        title: "求多个正分数的精确几何平均值",
        query: "分数: 计算 gmean(1/4,9/16)",
    },
    Example {
        icon: Icon::Terminal,
        title: "求多个分数或小数的精确平均值",
        query: "分数: 计算 avg(1/3,1/6)",
    },
    Example {
        icon: Icon::Terminal,
        title: "求多个分数或小数的精确中位数",
        query: "分数: 计算 median(0.5,3,1/3,1/8)",
    },
    Example {
        icon: Icon::Terminal,
        title: "在精确算式中用概率求分位数",
        query: "分数: 计算 quantile(1/4,3/2,1/2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "在精确算式中用百分数求分位数",
        query: "分数: 计算 percentile(25,4,1,3,2)",
    },
    Example {
        icon: Icon::Terminal,
        title: "在精确算式中求四分位距",
        query: "分数: 计算 iqr(7/3,2/3,5/3,1/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确算式计算中位绝对偏差",
        query: "分数: 计算 mad(0,1/2,1,10)",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确算式计算平均绝对偏差",
        query: "分数: 计算 aad(0,1/2,1,10)",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确算式计算样本方差",
        query: "分数: 计算 variance(1/3,1,5/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确算式计算样本标准差",
        query: "分数: 计算 stdev(1/3,1,5/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确算式计算均方值",
        query: "分数: 计算 meansquare(1/3,2/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算多个分数的精确平方和",
        query: "分数: 计算 sumsq(-1/3,2/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "用精确算式计算均方根",
        query: "分数: 计算 rms(-1/3,7/3)",
    },
    Example {
        icon: Icon::Terminal,
        title: "在精确算式中计算加权平均值",
        query: "分数: 计算 wavg(1/3,2,1/6,1)",
    },
    Example {
        icon: Icon::Terminal,
        title: "在精确算式中计算加权总体方差",
        query: "分数: 计算 wpvariance(1/3,2,7/3,1)",
    },
    Example {
        icon: Icon::Terminal,
        title: "在算式中计算加权总体标准差",
        query: "wpstdev(-3,1,3,1)",
    },
    Example {
        icon: Icon::Terminal,
        title: "在精确算式中计算加权分位数",
        query: "分数: 计算 wquantile(1/2,4,2,1,1,2,1)",
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
        title: "按给定横坐标计算精确值并预览 CSV",
        query: "分数: 取值CSV [3/2,-2,1.50,0] x**2+1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "为精确算式生成等间距的 x 和 y 表格",
        query: "分数: 精确采样 [-1,1,5] x**2+1/3",
    },
    Example {
        icon: Icon::Terminal,
        title: "求两个分数坐标间的精确平均变化率",
        query: "分数: 平均变化率 [0,1/3] x**2",
    },
    Example {
        icon: Icon::Terminal,
        title: "生成可再次代入的精确割线算式",
        query: "分数: 割线式 [1/3,2/3] x**2",
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
        title: "求等差数列的前几项之和",
        query: "分数: 等差求和 1/3,1/6,5",
    },
    Example {
        icon: Icon::Terminal,
        title: "求等比数列的前几项之和",
        query: "分数: 等比求和 1,-1/2,4",
    },
    Example {
        icon: Icon::Terminal,
        title: "求有理数的有限连分数",
        query: "分数: 连分数 355/113",
    },
    Example {
        icon: Icon::Terminal,
        title: "逐项计算有限连分数的收敛分数",
        query: "分数: 收敛分数 355/113",
    },
    Example {
        icon: Icon::Terminal,
        title: "比较两个算式的精确分数值",
        query: "分数: 比较 1/3 < 0.34",
    },
    Example {
        icon: Icon::Terminal,
        title: "用多个比较符检查相邻算式的精确值",
        query: "分数: 比较 1/3 < 0.34 <= 1/2",
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
        title: "把整数分解为质因数的乘积",
        query: "分数: 质因数分解 360",
    },
    Example {
        icon: Icon::Terminal,
        title: "计算正因数个数和总和",
        query: "分数: 因数统计 12",
    },
    Example {
        icon: Icon::Terminal,
        title: "同时得到向下取整的商和精确余数",
        query: "分数: 商和余数 -7,3",
    },
    Example {
        icon: Icon::Terminal,
        title: "合并多组余数和模数，求同余方程的解",
        query: "分数: 同余方程 2,3; 3,5; 2,7",
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
    Example {
        icon: Icon::Terminal,
        title: "查找搜索框里的快捷键",
        query: "搜索快捷键:",
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
    let scope = query.split_once(':').and_then(|(name, terms)| {
        let prefix = tool_example_keyword(name)?;
        prefix
            .ends_with(':')
            .then(|| (search_key(prefix), terms.trim()))
    });
    let (prefix, query) = scope
        .as_ref()
        .map_or((None, query.as_str()), |(prefix, terms)| {
            (Some(prefix.as_str()), *terms)
        });
    let mut matches: Vec<_> = search_text
        .iter()
        .enumerate()
        .filter_map(|(index, (title, example_query))| {
            let example_query = match prefix {
                Some(prefix) => example_query.strip_prefix(prefix)?.trim_start(),
                None => example_query.as_str(),
            };
            if query.is_empty() {
                return Some((0, 0, index));
            }
            if !query
                .split_whitespace()
                .all(|word| title.contains(word) || example_query.contains(word))
            {
                return None;
            }
            let command_rank = title_rank(example_query, query);
            let rank = title_rank(title, query).min(command_rank);
            Some((rank, command_rank, index))
        })
        .collect();
    matches.sort_unstable();
    matches.into_iter().map(|(_, _, index)| index).collect()
}

pub(super) fn example_help_query(query: &str) -> Option<String> {
    let query = query.trim();
    let (name, _) = query.split_once([':', '：'])?;
    let keyword = tool_example_keyword(name)?;
    let operation = scoped_query(query).and_then(|(scope, term)| {
        let operation = term.split_whitespace().next()?;
        tool_operation_examples(scope, operation).map(|_| operation)
    });
    Some(match operation {
        Some(operation) => format!("? {keyword} {operation}"),
        None => format!("? {keyword}"),
    })
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
    let key = search_key(query.trim());
    let qualified = parse_scope(&key)
        .map(|scope| (scope, ""))
        .or_else(|| scoped_query(&key))
        .or_else(|| {
            let (name, term) = key.split_once(char::is_whitespace)?;
            Some((parse_scope(name)?, term.trim()))
        });
    if let Some((scope, term)) = qualified {
        if let Some(default) = scopes
            .iter()
            .position(|&candidate| candidate == Some(scope))
        {
            if term.is_empty() {
                return vec![default];
            }
            return matching_examples(term)
                .into_iter()
                .find(|&index| scopes[index] == Some(scope))
                .into_iter()
                .collect();
        }
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

fn replaceable_tool_operations(scope: Scope) -> &'static [&'static str] {
    match scope {
        Scope::Equation => &[
            "验根",
            "求根",
            "展开",
            "求导",
            "原函数",
            "摘要",
            "因式分解",
            "不等式",
            "顶点",
            "配方",
            "判别式",
            "韦达",
            "取系数",
            "由根",
            "交点",
            "围成面积",
            "标准式",
            "过三点",
            "由顶点",
            "切线",
            "法线",
            "函数图",
            "函数采样",
            "采样极值",
            "数值极小",
            "数值极大",
            "找异号区间",
            "搜索零点",
            "采样图",
            "函数值",
            "两侧采样",
            "函数对照",
            "函数对照图",
            "参数采样",
            "参数图",
            "极坐标图",
            "极坐标采样",
            "极坐标面积",
            "二分求根",
            "数值积分",
            "积分对照",
            "数值平均",
            "数值面积",
            "数值弧长",
            "数值导数",
            "平均变化率",
            "数值切线",
            "数值切线图",
            "数值法线",
            "数值二阶导数",
            "导数采样",
            "导数采样图",
            "积分采样",
            "积分采样图",
            "定积分",
            "区间极值",
        ],
        Scope::LinearSystem => &["通解", "三元"],
        Scope::LinearFit => &[
            "预测",
            "插值",
            "插值系数",
            "插值式",
            "多项式",
            "多项式残差",
            "多项式误差",
            "多项式预测",
            "多项式摘要",
            "加权多项式",
            "稳健直线",
            "散点图",
            "梯形积分",
            "折线图",
            "梯形平均",
            "梯形面积",
            "梯形累计列",
            "差商列",
            "时间窗平均列",
            "时间窗积分列",
            "线性插值",
            "曲线斜率",
            "分段直线",
            "曲线极值",
            "曲线截取",
            "曲线变差",
            "曲线长度",
            "曲线均方根",
            "曲线标准差",
            "曲线分位数",
            "等距插值",
            "水平交点",
            "超阈跨度",
            "超阈面积",
            "超阈区间",
            "曲线差值",
            "曲线对比图",
            "曲线交点",
            "曲线间面积",
            "曲线差积分",
            "梯形累计图",
            "曲线图",
            "残差图",
            "残差",
            "误差",
            "过原点",
        ],
        Scope::Date => &[
            "列出",
            "格式化",
            "解析",
            "月历",
            "年月日差",
            "工作日序号",
            "工作日位置",
            "工作日序列",
            "星期序列",
            "下一个星期",
            "上一个星期",
            "合并区间",
            "区间交集",
            "扣除区间",
            "按月拆分",
            "按月拆分CSV",
            "按周拆分CSV",
            "按季度拆分CSV",
            "按半年拆分CSV",
            "按年拆分CSV",
            "逐日CSV",
            "星期统计",
            "表格序号",
            "从表格序号",
            "表格时刻序号",
            "从表格时刻序号",
        ],
        Scope::Radix => &[
            "字节序",
            "读字节序",
            "补码",
            "读补码",
            "位统计",
            "取位",
            "写位",
            "循环左移",
            "循环右移",
            "位倒序",
            "字节倒序",
            "格雷码",
            "读格雷码",
            "格雷码表",
            "浮点位",
            "读浮点位",
            "浮点分数",
            "读浮点分数",
            "浮点相邻",
            "读浮点相邻",
            "汉明距离",
            "位比对",
            "位比对CSV",
            "表格列号",
            "单元格地址",
            "罗马数字",
            "小数",
        ],
        Scope::ExactData => &["单位表"],
        Scope::Duration => &[
            "精确换算",
            "合计",
            "平均",
            "频数平均",
            "频数中位数",
            "扣除休息",
            "重叠",
            "比率",
            "缩放",
            "分摊",
            "舍入",
            "向下取整",
            "向上取整",
            "中位数",
            "分位数",
            "分位表",
            "单位表",
            "最短",
            "最长",
            "极差",
            "排序",
            "降序",
        ],
        Scope::Pitch => &["MIDI", "转调", "音程", "音分差", "音分移调"],
        Scope::Statistics => &[
            "频数CSV",
            "分位表",
            "加权平均",
            "加权分位数",
            "加权分位表",
            "加权累积分布",
            "加权累计权重",
            "加权累计比例",
            "加权频数CSV",
            "加权方差",
            "加权标准差",
        ],
        Scope::Decibel => &[
            "功率比",
            "幅度比",
            "转功率比",
            "转幅度比",
            "叠加",
            "功率差",
            "能量平均",
            "加权平均",
        ],
        Scope::Tempo => &["时长", "速度", "拍数"],
        Scope::Circuit => &[
            "串联",
            "并联",
            "分压",
            "负载分压",
            "分流",
            "RC",
            "RC充电",
            "RC放电",
            "RC电容",
            "RC电阻",
            "RC低通",
            "RC高通",
            "RC低通表",
            "RC高通表",
            "耗电",
            "电池能量",
        ],
        Scope::Solid => &[
            "圆柱",
            "圆锥",
            "圆台",
            "圆管",
            "球",
            "球冠",
            "正四面体",
            "立方体",
            "长方体",
        ],
        _ => &[],
    }
}

pub(super) fn tool_operation_examples(scope: Scope, term: &str) -> Option<Vec<usize>> {
    // 示例补全只匹配操作名。已有参数的输入直接交给对应工具。
    if term.contains(char::is_whitespace) {
        return None;
    }
    match scope {
        Scope::Date
        | Scope::Radix
        | Scope::Duration
        | Scope::ExactData
        | Scope::Pitch
        | Scope::Decibel
        | Scope::Tempo
        | Scope::Circuit
        | Scope::Solid
        | Scope::Equation
        | Scope::LinearSystem
        | Scope::LinearFit
            if !term.is_empty()
                && replaceable_tool_operations(scope).iter().any(|operation| {
                    operation
                        .get(..term.len())
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(term))
                }) => {}
        Scope::Intervals
            if ["交集", "合并", "差集", "对称差", "长度", "包含"]
                .into_iter()
                .any(|operation| operation.starts_with(term)) => {}
        Scope::BatchCalculation
            if matches!(term, "分" | "分数" | "分数对" | "分数对照" | "对" | "对照") =>
            {}
        Scope::Fraction
            if !term.is_empty()
                && [
                    "循环小数",
                    "有效数字",
                    "工程计数法",
                    "百分比",
                    "从百分比",
                    "取值表",
                    "取值CSV",
                    "精确采样",
                    "平均变化率",
                    "割线式",
                    "等差数列",
                    "等比数列",
                    "等差求和",
                    "等比求和",
                    "连分数",
                    "收敛分数",
                    "从连分数",
                    "从循环小数",
                    "比较",
                    "带分数",
                    "从带分数",
                    "限分母",
                    "质因数分解",
                    "因数统计",
                    "商和余数",
                    "同余方程",
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
                    "精确时长",
                    "精确时长对照",
                    "时间戳对照",
                    "路径对照",
                    "网址对照",
                    "日期对照",
                    "颜色对照",
                    "进制对照",
                    "节拍对照",
                    "分贝对照",
                    "立体对照",
                    "波长对照",
                    "网段",
                    "网段对照",
                ]
                .into_iter()
                .any(|operation| operation.starts_with(term))) => {}
        Scope::Color
            if !term.is_empty()
                && ["色阶", "线性色阶", "OKLab色阶"]
                    .into_iter()
                    .any(|operation| operation.starts_with(term)) => {}
        Scope::Timestamp
            if !term.is_empty()
                && [
                    "向下对齐",
                    "向上对齐",
                    "就近对齐",
                    "时区CSV",
                    "时区候选",
                    "日区间",
                    "周区间",
                    "月区间",
                    "季度区间",
                    "年区间",
                    "区间合并",
                    "区间交集",
                    "区间差集",
                    "覆盖时长",
                    "逐日时长",
                    "偏移变更",
                    "每日同刻",
                    "序列",
                ]
                .into_iter()
                .any(|operation| operation.starts_with(term)) => {}
        Scope::Triangle
            if !term.is_empty()
                && ["斜边", "直角边"]
                    .into_iter()
                    .any(|operation| operation.starts_with(term)) => {}
        Scope::Cidr
            if !term.is_empty()
                && [
                    "子网",
                    "排除",
                    "归属CSV",
                    "清单CSV",
                    "合并",
                    "交集",
                    "差集",
                    "对称差",
                    "取地址",
                    "地址序号",
                    "偏移",
                    "取子网",
                    "子网序号",
                ]
                .into_iter()
                .any(|operation| operation.starts_with(term)) => {}
        Scope::Matrix
            if matches!(term, "乘方" | "特" | "特征" | "特征值")
                || (!term.is_empty()
                    && ["通解", "最小二乘", "1范数", "无穷范数", "条件数", "正定性"]
                        .iter()
                        .any(|op| op.starts_with(term))) => {}
        Scope::Statistics
            if !term.is_empty()
                && [
                    "二项概率",
                    "二项概率表",
                    "二项分位点",
                    "二项摘要",
                    "负二项概率",
                    "负二项累计",
                    "负二项至少",
                    "负二项区间概率",
                    "负二项概率表",
                    "负二项摘要",
                    "负二项分位点",
                    "二项累计",
                    "二项至少",
                    "二项区间概率",
                    "几何概率",
                    "几何累计",
                    "几何至少",
                    "几何分位点",
                    "泊松概率",
                    "泊松累计",
                    "泊松至少",
                    "泊松摘要",
                    "泊松概率表",
                    "泊松区间概率",
                    "几何摘要",
                    "超几何概率",
                    "超几何概率表",
                    "超几何累计",
                    "超几何至少",
                    "超几何区间概率",
                    "超几何分位点",
                    "超几何摘要",
                    "独立性卡方",
                    "期望频数",
                    "箱线摘要",
                    "箱线图",
                    "分箱图",
                    "累计分布图",
                    "截尾平均",
                    "缩尾平均",
                    "基尼系数",
                    "基尼平均差",
                    "移动合计",
                    "移动样本方差",
                    "移动总体方差",
                    "移动样本标准差",
                    "移动总体标准差",
                    "移动最小",
                    "移动最大",
                    "移动极差",
                    "移动分位数",
                    "变异系数",
                    "样本变异系数",
                    "总体变异系数",
                ]
                .into_iter()
                .any(|operation| operation.starts_with(term)) => {}
        Scope::Statistics
            if !term.is_empty()
                && replaceable_tool_operations(scope)
                    .iter()
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
                    | "平均绝对偏差"
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
                    | "标准分"
                    | "总体标"
                    | "总体标准"
                    | "总体标准分"
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
                    | "肯"
                    | "肯德"
                    | "肯德尔"
                    | "差"
                    | "差分"
                    | "变"
                    | "变化"
                    | "变化率"
            ) => {}
        Scope::Json
        | Scope::Toml
        | Scope::Csv
        | Scope::TextTools
        | Scope::UrlEncoding
        | Scope::Path
        | Scope::Percentage
        | Scope::Uuid
        | Scope::Ulid
        | Scope::Isbn
        | Scope::Vector
        | Scope::Matrix
        | Scope::Complex
        | Scope::Circle
            if !term.contains(char::is_whitespace) => {}
        Scope::Date
            if ["月初序列", "月末序列", "季初序列", "季末序列"]
                .into_iter()
                .any(|operation| operation.starts_with(term)) => {}
        Scope::Wave if "波速".starts_with(term) => {}
        Scope::DataPipeline
            if [
                "步骤csv",
                "逐行",
                "逐行csv",
                "csv列",
                "json数组",
                "json对象",
                "json行",
            ]
            .into_iter()
            .any(|operation| {
                operation
                    .get(..term.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(term))
            }) => {}
        Scope::Date
        | Scope::Duration
        | Scope::BatchConversion
        | Scope::BatchCalculation
        | Scope::DataPipeline
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
        | Scope::Wave
        | Scope::Pitch
        | Scope::Decibel
        | Scope::Solid
        | Scope::Tempo
        | Scope::AspectRatio
        | Scope::MapScale
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
        Scope::Polygon
            if ["重心", "包含", "周长", "边界框", "凸性"]
                .into_iter()
                .any(|operation| operation.starts_with(term)) => {}
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
    matching_function_examples(query, |example| Some(example.query))
}

pub(super) fn fraction_function_examples(query: &str) -> Option<Vec<usize>> {
    let query = search_key(query);
    matching_function_examples(fraction_command_expression(&query)?, |example| {
        fraction_command_expression(example.query.strip_prefix("分数: ")?)
    })
}

fn standalone_function_arguments(expression: &str) -> Option<&str> {
    let (name, rest) = split_name(expression);
    if !name.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
        return None;
    }
    let arguments = rest.trim_start().strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0usize;
    for byte in arguments.bytes() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.checked_sub(1)?,
            _ => {}
        }
    }
    (depth == 0 && !arguments.trim().is_empty()).then_some(arguments)
}

fn fraction_command_expression(input: &str) -> Option<&str> {
    let (verb, expression) = input.split_once(char::is_whitespace)?;
    (verb == "计算" || verb.eq_ignore_ascii_case("calc")).then(|| expression.trim())
}

fn matching_function_examples(
    query: &str,
    expression: fn(&Example) -> Option<&str>,
) -> Option<Vec<usize>> {
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
            let (function, tail) = split_name(expression(example)?);
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
    fn selects_only_equation_inputs_and_suggests_named_operations() {
        for (scope, typed, query, arguments) in [
            (
                Scope::Equation,
                "切",
                "方程: 切线 1/2,-1,3/4 @ 2",
                "1/2,-1,3/4 @ 2",
            ),
            (
                Scope::Equation,
                "取",
                "方程: 取系数 (1/3+1/6)*x^2 - x + 3/4",
                "(1/3+1/6)*x^2 - x + 3/4",
            ),
            (
                Scope::LinearSystem,
                "通",
                "联立: 通解 1,2,3; 2,4,6",
                "1,2,3; 2,4,6",
            ),
            (
                Scope::Equation,
                "",
                "方程: x**2 - 3*x + 2 = 0",
                "x**2 - 3*x + 2 = 0",
            ),
            (
                Scope::LinearSystem,
                "",
                "联立: x + y = 3; x - y = 1",
                "x + y = 3; x - y = 1",
            ),
        ] {
            let matches = tool_operation_examples(scope, typed).unwrap();
            let example = matches
                .into_iter()
                .map(|index| &EXAMPLES[index])
                .find(|example| example.query == query)
                .expect(query);
            let start = example.argument_start().unwrap();
            assert_eq!(&query[start..], arguments);
        }
        assert!(tool_operation_examples(Scope::Equation, "切线 x²").is_none());
        assert!(tool_operation_examples(Scope::LinearSystem, "通解 x").is_none());
    }

    #[test]
    fn csv_example_data_selection_preserves_array_and_object_options() {
        for (query, expected) in [
            (
                "CSV: 选列名 [\"名称\"] 编号,名称\n001,茶",
                Some("编号,名称\n001,茶"),
            ),
            (
                "CSV: 去列 [1]\r\n 编号,值\r\n001,茶",
                Some(" 编号,值\r\n001,茶"),
            ),
            (
                "CSV: 搜索行 {\"词\":[\"茶\"],\"列\":[\"备注\"]} 区域,备注\n南区,茶",
                Some("区域,备注\n南区,茶"),
            ),
            (
                "CSV: 命中位置 {\"词\":[\"茶\"],\"列\":[2]}\r\n 编号,备注\r\n002,绿茶",
                Some(" 编号,备注\r\n002,绿茶"),
            ),
            ("CSV: 选列名 [\"名称\"]", None),
            ("CSV: 选列名 [\"名称\"]\n名称", None),
            ("CSV: 选列 1 名称,值\n茶,2", None),
            ("CSV: 转置\n编号,名称\n001,茶", None),
            ("文本: 逐行配对 [\"甲\"]\n茶\n米", None),
            (
                "文本: 定宽转CSV [3,2]\r\n 01北京  \r\n002上海 ",
                Some(" 01北京  \r\n002上海 "),
            ),
            ("文本: fixed-width-csv [3,2] 001北京", Some("001北京")),
            ("文本: 定宽转CSV [3,2]", None),
            (
                r#"JSON: 路径转CSV ["/金额"] [{"金额":1.50}]"#,
                Some(r#"[{"金额":1.50}]"#),
            ),
            (
                "JSON: 行路径转CSV [\"/金额\"]\n{\"金额\":1.50}\n{\"金额\":0}",
                Some("{\"金额\":1.50}\n{\"金额\":0}"),
            ),
            (
                "JSON: 行筛选 [[\"/x\",\"=\",1]]\n{\"x\":1}\n{\"x\":2}",
                Some("{\"x\":1}\n{\"x\":2}"),
            ),
            (
                r#"JSON: 多条件筛选 [["/x","=",1]] [{"x":1},{"x":2}]"#,
                Some(r#"[{"x":1},{"x":2}]"#),
            ),
            (r#"JSON: 路径转CSV ["/x"] ["#, None),
            (r#"JSON: 对比 [1] [2]"#, None),
        ] {
            let example = Example {
                icon: Icon::Terminal,
                title: "",
                query,
            };
            assert_eq!(example.data_start().map(|start| &query[start..]), expected);
        }
    }

    #[test]
    fn selects_function_parameters_or_the_whole_exact_expression() {
        for (query, expected) in [
            ("root(-125/343,3)", Some(("-125/343,3", ")"))),
            ("分数: 计算 root(-125/343,3)", Some(("-125/343,3", ")"))),
            ("sin(radians(90))", Some(("radians(90)", ")"))),
            (
                "分数: 计算 sqrt(4)+root(1/8,3)",
                Some(("sqrt(4)+root(1/8,3)", "")),
            ),
            ("分数: 计算 2**80", Some(("2**80", ""))),
            ("2**10", None),
            ("gcd()", None),
        ] {
            let example = Example {
                icon: Icon::Terminal,
                title: "函数参数",
                query,
            };
            let selected = example.argument_range().map(|range| {
                let tail = &query[range.end..];
                (&query[range], tail)
            });
            assert_eq!(selected, expected, "{query}");
        }
    }

    #[test]
    fn shows_scoped_command_completions_only_before_arguments() {
        for (scope, prefix, with_argument) in [
            (Scope::Csv, "排序", "排序 1 编号"),
            (Scope::Json, "改键", "改键 ["),
            (Scope::Date, "月末", "月末序列 2024-01,3"),
            (Scope::Statistics, "箱线", "箱线摘要 1,2,3"),
            (Scope::Statistics, "加权分位数", "加权分位数 60%"),
            (Scope::Statistics, "加权累积", "加权累积分布 1@2"),
            (Scope::Statistics, "加权标准差", "加权标准差 1@2"),
            (Scope::LinearFit, "多项", "多项式 2"),
            (Scope::TextTools, "取列", "取列 \"x\""),
        ] {
            assert!(tool_operation_examples(scope, prefix).is_some(), "{prefix}");
            assert_eq!(
                tool_operation_examples(scope, with_argument),
                None,
                "{with_argument}"
            );
        }
    }

    #[test]
    fn help_filters_by_tool_and_operation_without_including_query_arguments() {
        assert_eq!(
            example_help_query("ＪＳＯＮ：取值 /金额 {\"金额\":2}").as_deref(),
            Some("? JSON: 取值")
        );
        assert_eq!(
            example_help_query("CSV: 未知命令 [1,2] 表格").as_deref(),
            Some("? CSV:")
        );
        assert_eq!(example_help_query("2**10"), None);

        let items = matching_examples("ｊｓｏｎ：　取值");
        assert!(EXAMPLES[items[0]].query.starts_with("JSON: 取值 "));
        assert!(items
            .iter()
            .all(|&index| EXAMPLES[index].query.starts_with("JSON:")));
        assert!(matching_examples("取值")
            .iter()
            .any(|&index| EXAMPLES[index].query.starts_with("处理:")));
    }

    #[test]
    fn narrows_the_tool_index_to_a_named_tool_and_its_matching_examples() {
        for (query, expected) in [
            ("circle: 切点", "圆: 切点 "),
            ("vector 点积", "向量: 点积 "),
            ("complex 由极坐标", "复数: 由极坐标 "),
            ("ＣＳＶ　最近点", "CSV: 最近点 "),
            ("matrix 乘方", "矩阵: 乘方 "),
        ] {
            let items = tool_index_examples(query);
            assert_eq!(items.len(), 1, "{query}");
            assert!(EXAMPLES[items[0]].query.starts_with(expected), "{query}");
        }
        assert!(tool_index_examples("circle 查无此工具用法").is_empty());
    }

    #[test]
    fn selects_parameters_in_geometry_vector_matrix_and_complex_examples() {
        for (scope, operation, prefix, parameters) in [
            (Scope::Circle, "切点", "圆: 切点 ", "0,0 @ 1; 2,0"),
            (Scope::Complex, "极坐标", "复数: 极坐标 ", "3+4i"),
            (
                Scope::Vector,
                "直线距离",
                "向量: 直线距离 ",
                "3,4 @ 0,0; 2,0",
            ),
            (Scope::Matrix, "乘方", "矩阵: 乘方 ", "1,1; 1,0 @ 5"),
        ] {
            let items = tool_operation_examples(scope, operation).unwrap();
            let example = &EXAMPLES[items[0]];
            let (retained, selected) = example.query.split_at(example.argument_start().unwrap());
            assert_eq!((retained, selected), (prefix, parameters));
        }
    }

    #[test]
    fn selects_the_electrical_values_in_circuit_examples() {
        for (term, expected) in [
            ("", "5 V, 20 Ω"),
            ("分流", "6 mA @ 1 kΩ,2 kΩ"),
            ("负载", "12 V @ 1 kΩ,2 kΩ,2 kΩ"),
        ] {
            let items = tool_operation_examples(Scope::Circuit, term).unwrap();
            let example = &EXAMPLES[items[0]];
            assert_eq!(
                &example.query[example.argument_start().unwrap()..],
                expected
            );
        }
    }

    #[test]
    fn selects_only_the_table_in_the_csv_column_pipeline_example() {
        let items = tool_operation_examples(Scope::DataPipeline, "CSV列").unwrap();
        assert_eq!(items.len(), 1);
        let example = &EXAMPLES[items[0]];
        let start = example.argument_start().unwrap();
        assert!(example.query[..start].ends_with("\"分数:\"]]\n"));
        assert!(example.query[start..].starts_with("编号,明细\n"));
    }

    #[test]
    fn selects_only_data_in_the_row_pipeline_example() {
        for term in ["逐行CSV", "JSON行"] {
            let items = tool_operation_examples(Scope::DataPipeline, term).unwrap();
            assert_eq!(items.len(), 1);
            let example = &EXAMPLES[items[0]];
            let start = example.argument_start().unwrap();
            assert!(example.query[..start].ends_with("\"分数:\"]\n"));
            assert_eq!(&example.query[start..], "{\"金额\":0.50}\n{\"金额\":1.25}");
        }
    }

    #[test]
    fn selects_only_the_source_data_when_replacing_a_pipeline_example() {
        let items = tool_operation_examples(Scope::DataPipeline, "").unwrap();
        let example = &EXAMPLES[items[0]];
        let start = example.argument_start().unwrap();
        assert!(example.query[..start].ends_with(r#""JSON: 取值 /1/金额"] "#));
        assert_eq!(&example.query[start..], "编号,金额\n001,1.50\n\n002,2e0");
        assert!(tool_operation_examples(Scope::DataPipeline, "[\"文本: 去空行\"]").is_none());

        let items = tool_operation_examples(Scope::DataPipeline, "步骤CSV").unwrap();
        assert_eq!(items.len(), 1);
        let example = &EXAMPLES[items[0]];
        assert_eq!(
            &example.query[example.argument_start().unwrap()..],
            "茶 a\n咖啡 b"
        );
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
            Scope::Wave,
            Scope::Pitch,
            Scope::Decibel,
            Scope::Solid,
            Scope::Tempo,
            Scope::AspectRatio,
            Scope::MapScale,
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
