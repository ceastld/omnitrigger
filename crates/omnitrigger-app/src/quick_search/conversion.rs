use std::f64::consts::{LN_10, PI, TAU};

use num_bigint::BigInt;
use num_rational::BigRational;

use super::aspect_ratio::{pixel_dimensions, AspectRatioError};
use super::batch::{batch_arguments, map_batch_csv, map_batch_lines, BatchError};
use super::calculator::{evaluate, format_result, looks_like_calculation, normalize, QUERY_LIMIT};
use super::csv_tools::{csv_cell, write_rows};
use super::duration::{convert_duration, elapsed_duration, DurationError};
use super::fraction::{
    calculate_fraction, ensure_exact_size, format_significant_digits, parse_fraction_literal,
};
use super::operators::split_top_level_commas;
use super::radix::Base;
use super::statistics::parse_probability;
use super::InputSuggestion;

const MAX_CONVERSION_TARGETS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConversionError {
    Incomplete,
    InvalidAmount,
    InvalidInteger,
    UnknownUnit,
    InvalidTargets,
    InvalidFontRelativeLength,
    InvalidViewportLength,
    ExactDataUnit,
    DifferentDimensions,
    InvalidTransfer,
    InvalidTransferDuration,
    InvalidPace,
    InvalidPixelDensity,
    ScaleArguments,
    InvalidScaleRatio,
    InvalidScaleLength,
    InvalidFuelEconomy,
    InvalidUnitPrice,
    InvalidCircuit,
    InvalidResistanceOperands,
    InvalidResistanceList,
    InvalidVoltageDivider,
    InvalidLoadedVoltageDivider,
    InvalidCurrentDivider,
    InvalidRcFilter,
    InvalidRcCharge,
    InvalidRcDischarge,
    InvalidRcDesign,
    InvalidRcResponse,
    InvalidRcResponseTable,
    InvalidEnergyUse,
    InvalidBatteryEnergy,
    InvalidPeriod,
    InvalidPeriodTarget,
    InvalidWave,
    InvalidWaveSpeed,
    InvalidWaveTarget,
    InvalidWaveOperands,
    InvalidWaveSpeedTarget,
    OutOfRange,
    TooLong,
}

impl ConversionError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入数值和两个单位，例如 10 km to mi",
            Self::InvalidAmount => "无法识别换算数值",
            Self::InvalidInteger => "进制转换需要整数，例如 0xff to dec",
            Self::UnknownUnit => "无法识别换算单位",
            Self::InvalidTargets => "用方括号和逗号填写 1 至 8 个目标单位，例如 [MiB, KiB, Gib]",
            Self::InvalidFontRelativeLength => {
                "用 px 填写正的参考字号或行高，例如 1.5em @ 18px to px"
            }
            Self::InvalidViewportLength => {
                "填写视口宽和高，换算 vw、vh、vmin、vmax 与 px，例如 2.5vw @ 1440×900 to px"
            }
            Self::ExactDataUnit => {
                "可精确换算数据量或数据速率，例如 1/3 KiB to B 或 1 MiB/s to MB/s"
            }
            Self::DifferentDimensions => "两个单位不属于同一类型",
            Self::InvalidTransfer => "输入非负的数据量和大于零的数据速率",
            Self::InvalidTransferDuration => {
                "输入非负的数据量或速率。按数据量求速率时，用时须大于零。"
            }
            Self::InvalidPace => "距离、用时和速度须大于零，例如 配速: 10 km 用时 52:30",
            Self::InvalidPixelDensity => "输入正整数分辨率和正的对角线尺寸，例如 3840×2160 @ 27 in",
            Self::ScaleArguments => "输入比例和距离，例如 1:50000 3.2 cm to km",
            Self::InvalidScaleRatio => "比例须由两个正数组成，例如 1:50000",
            Self::InvalidScaleLength => "使用长度单位和非负距离，求比例时两段距离须大于零",
            Self::InvalidFuelEconomy => "输入正数及油耗单位，可用 L/100 km、km/L、mpg-us 或 mpg-uk",
            Self::InvalidUnitPrice => "输入价格、正的数量和目标单位，例如 36 元 / 750 g to kg",
            Self::InvalidCircuit => {
                "输入两种电学量，例如 5 V, 20 Ω。电压、电流须为非负值，电阻须大于零"
            }
            Self::InvalidResistanceOperands => "求电阻时，电压和电流均须大于零",
            Self::InvalidResistanceList => "输入 2 至 32 个正电阻值，各项用逗号隔开",
            Self::InvalidVoltageDivider => "输入电压和两个正电阻，例如 分压 12 V @ 1 kΩ,2 kΩ",
            Self::InvalidLoadedVoltageDivider => {
                "输入电压、两个正电阻和负载电阻，例如 负载分压 12 V @ 1 kΩ,2 kΩ,2 kΩ"
            }
            Self::InvalidCurrentDivider => "输入总电流和两个正电阻，例如 分流 6 mA @ 1 kΩ,2 kΩ",
            Self::InvalidRcFilter => "输入正电阻和正电容，例如 RC 1 kΩ,1 μF",
            Self::InvalidRcCharge => "输入低于 100% 的目标充电比例，例如 RC充电 50% @ 1 kΩ,1 μF",
            Self::InvalidRcDischarge => "输入大于零的剩余比例，例如 RC放电 25% @ 1 kΩ,1 μF",
            Self::InvalidRcDesign => "输入已知元件和截止频率，例如 RC电容 1 kΩ @ 1 kHz",
            Self::InvalidRcResponse => {
                "填写正电阻、正电容和非负频率，例如 RC低通 1 kΩ,1 μF @ 1 kHz"
            }
            Self::InvalidRcResponseTable => {
                "填写电阻、电容和 1 至 32 个频率，例如 1 kΩ,1 μF @ [\"10 Hz\",\"1 kHz\"]"
            }
            Self::InvalidEnergyUse => "填写非负功率和用时，例如 耗电 1.5 kW @ 2h",
            Self::InvalidBatteryEnergy => "填写容量和标称电压，例如 电池能量 2500 mAh @ 3.7 V",
            Self::InvalidPeriod => "输入大于零的频率或周期，例如 50 Hz to ms 或 2 ms to Hz",
            Self::InvalidPeriodTarget => "频率请选择时间单位，周期请选择频率单位",
            Self::InvalidWave => "输入大于零的频率或波长，例如 2.4 GHz to mm 或 500 nm to THz",
            Self::InvalidWaveSpeed => "在 @ 后填写大于零的传播速度，例如 343 m/s",
            Self::InvalidWaveTarget => "频率请选择长度单位，波长请选择频率单位",
            Self::InvalidWaveOperands => "各填写一个大于零的频率和波长，例如 波速 1 kHz @ 34.3 cm",
            Self::InvalidWaveSpeedTarget => "传播速度请选择 m/s、km/h 等速度单位",
            Self::OutOfRange => "结果超出换算范围",
            Self::TooLong => "换算式过长",
        }
    }
}

#[derive(PartialEq, Eq)]
enum Dimension {
    Length,
    Mass,
    Time,
    Temperature,
    TemperatureDifference,
    Area,
    Volume,
    Speed,
    Data,
    DataRate,
    Angle,
    AngularSpeed,
    Pressure,
    Frequency,
    Energy,
    Power,
    Density,
    VolumeFlow,
    Force,
    Torque,
    Acceleration,
    Illuminance,
    ElectricCurrent,
    Voltage,
    Resistance,
}

impl Dimension {
    fn suggested_units(&self, source: &str) -> &'static [&'static str] {
        match self {
            Self::Length if matches!(source, "px" | "pt" | "pc" | "Q") => {
                &["px", "pt", "pc", "in", "cm", "mm", "Q"]
            }
            Self::Length if matches!(source, "nm" | "μm") => {
                &["nm", "μm", "mm", "cm", "m", "km", "ft", "in", "yd", "mi"]
            }
            Self::Length => &["m", "mi", "km", "cm", "ft", "mm", "in", "yd", "μm", "nm"],
            Self::Mass => &["kg", "g", "lb", "oz", "mg"],
            Self::Time if matches!(source, "ns" | "μs") => {
                &["ns", "μs", "ms", "s", "min", "h", "d"]
            }
            Self::Time => &["s", "min", "h", "ms", "d", "μs", "ns"],
            Self::Temperature => &["°C", "°F", "K"],
            Self::TemperatureDifference => &["Δ°C", "Δ°F", "ΔK"],
            Self::Area => &[
                "m²", "ft²", "ha", "km²", "cm²", "acre", "亩", "mm²", "in²", "yd²", "mi²",
            ],
            Self::Volume => &["L", "mL", "usgal", "impgal", "m³"],
            Self::Speed => &["km/h", "m/s", "mph", "kn"],
            Self::Data => &[
                "MB", "MiB", "GB", "GiB", "B", "kB", "KiB", "TB", "TiB", "b", "kb", "Mb", "Gb",
                "Tb", "Kib", "Mib", "Gib", "Tib",
            ],
            Self::DataRate => &[
                "MB/s", "Mbps", "MiB/s", "kB/s", "Gbps", "b/s", "B/s", "kbps", "KiB/s", "Mib/s",
                "GB/s", "GiB/s", "Gib/s", "TB/s", "TiB/s", "Tbps", "Tib/s", "Kib/s",
            ],
            Self::Angle => &["°", "rad", "turn", "′", "″"],
            Self::AngularSpeed => &["rpm", "rev/s", "rad/s", "°/s", "rad/min", "°/min"],
            Self::Pressure => &[
                "Pa", "kPa", "bar", "psi", "atm", "hPa", "MPa", "mPa", "Torr",
            ],
            Self::Frequency => &["Hz", "kHz", "MHz", "GHz", "mHz", "THz"],
            Self::Energy => &["J", "kJ", "MJ", "Wh", "kWh", "mJ", "mWh", "MWh"],
            Self::Power => &["W", "kW", "mW", "MW", "GW"],
            Self::Density => &["kg/m³", "g/cm³", "g/L", "kg/L"],
            Self::VolumeFlow => &["L/min", "L/s", "m³/h", "m³/s", "mL/s"],
            Self::Force => &["N", "kN", "kgf", "lbf", "mN", "MN", "dyn"],
            Self::Torque => &["N·m", "lbf·ft", "lbf·in", "N·cm", "kgf·m", "kN·m"],
            Self::Acceleration => &["m/s²", "g₀", "ft/s²", "cm/s²", "in/s²"],
            Self::Illuminance => &["lx", "fc", "klx", "phot"],
            Self::ElectricCurrent => &["A", "mA", "μA", "nA", "kA", "MA"],
            Self::Voltage => &["V", "mV", "kV", "μV", "MV"],
            Self::Resistance => &["Ω", "kΩ", "MΩ", "mΩ", "GΩ"],
        }
    }
}

struct Unit {
    symbol: &'static str,
    dimension: Dimension,
    scale: f64,
    origin: f64,
}

const STANDARD_GRAVITY: f64 = 9.806_65;
const FOOT_IN_METERS: f64 = 0.3048;
const NEWTONS_PER_POUND_FORCE: f64 = 4.448_221_615_260_5;
pub(super) const INCH_IN_METERS: f64 = 0.0254;
const METERS_PER_MILE: f64 = 1609.344;
const LITERS_PER_US_GALLON: f64 = 3.785_411_784;
const LITERS_PER_IMPERIAL_GALLON: f64 = 4.546_09;

impl Unit {
    fn linear(symbol: &'static str, dimension: Dimension, scale: f64) -> Self {
        Self {
            symbol,
            dimension,
            scale,
            origin: 0.,
        }
    }

    fn temperature(symbol: &'static str, scale: f64, origin: f64) -> Self {
        Self {
            symbol,
            dimension: Dimension::Temperature,
            scale,
            origin,
        }
    }

    fn parse(name: &str) -> Result<Self, ConversionError> {
        use Dimension::*;
        let name = name.trim();
        if let Some(unit) = data_unit(name) {
            return Ok(unit);
        }
        // 毫和兆的符号大小写不同。
        match name {
            "mPa" => return Ok(Self::linear("mPa", Pressure, 0.001)),
            "MPa" => return Ok(Self::linear("MPa", Pressure, 1_000_000.)),
            "mHz" => return Ok(Self::linear("mHz", Frequency, 0.001)),
            "MHz" => return Ok(Self::linear("MHz", Frequency, 1_000_000.)),
            "mJ" => return Ok(Self::linear("mJ", Energy, 0.001)),
            "MJ" => return Ok(Self::linear("MJ", Energy, 1_000_000.)),
            "mWh" => return Ok(Self::linear("mWh", Energy, 3.6)),
            "MWh" => return Ok(Self::linear("MWh", Energy, 3_600_000_000.)),
            "mW" => return Ok(Self::linear("mW", Power, 0.001)),
            "MW" => return Ok(Self::linear("MW", Power, 1_000_000.)),
            "kN" => return Ok(Self::linear("kN", Force, 1000.)),
            "mN" => return Ok(Self::linear("mN", Force, 0.001)),
            "MN" => return Ok(Self::linear("MN", Force, 1_000_000.)),
            "nA" => return Ok(Self::linear("nA", ElectricCurrent, 1e-9)),
            "μA" | "µA" | "uA" => return Ok(Self::linear("μA", ElectricCurrent, 1e-6)),
            "mA" => return Ok(Self::linear("mA", ElectricCurrent, 1e-3)),
            "kA" => return Ok(Self::linear("kA", ElectricCurrent, 1e3)),
            "MA" => return Ok(Self::linear("MA", ElectricCurrent, 1e6)),
            "μV" | "µV" | "uV" => return Ok(Self::linear("μV", Voltage, 1e-6)),
            "mV" => return Ok(Self::linear("mV", Voltage, 1e-3)),
            "kV" => return Ok(Self::linear("kV", Voltage, 1e3)),
            "MV" => return Ok(Self::linear("MV", Voltage, 1e6)),
            "mΩ" | "mΩ" | "mohm" => return Ok(Self::linear("mΩ", Resistance, 1e-3)),
            "kΩ" | "kΩ" | "kohm" => return Ok(Self::linear("kΩ", Resistance, 1e3)),
            "MΩ" | "MΩ" | "Mohm" => return Ok(Self::linear("MΩ", Resistance, 1e6)),
            "GΩ" | "GΩ" | "Gohm" => return Ok(Self::linear("GΩ", Resistance, 1e9)),
            "Q" => return Ok(Self::linear("Q", Length, 0.00025)),
            _ => {}
        }
        let unit = match name.to_ascii_lowercase().as_str() {
            "nm" | "纳米" => Self::linear("nm", Length, 1e-9),
            "μm" | "µm" | "um" | "微米" => Self::linear("μm", Length, 1e-6),
            "mm" | "毫米" => Self::linear("mm", Length, 0.001),
            "cm" | "厘米" => Self::linear("cm", Length, 0.01),
            "m" | "米" => Self::linear("m", Length, 1.),
            "km" | "千米" | "公里" => Self::linear("km", Length, 1000.),
            "in" | "inch" | "inches" | "英寸" => Self::linear("in", Length, INCH_IN_METERS),
            "px" | "csspx" => Self::linear("px", Length, INCH_IN_METERS / 96.),
            "pt" | "csspt" | "排版点" => Self::linear("pt", Length, INCH_IN_METERS / 72.),
            "pc" | "csspc" => Self::linear("pc", Length, INCH_IN_METERS / 6.),
            "ft" | "foot" | "feet" | "英尺" => Self::linear("ft", Length, FOOT_IN_METERS),
            "yd" | "码" => Self::linear("yd", Length, 0.9144),
            "mi" | "mile" | "miles" | "英里" => Self::linear("mi", Length, METERS_PER_MILE),
            "mg" | "毫克" => Self::linear("mg", Mass, 0.001),
            "g" | "克" => Self::linear("g", Mass, 1.),
            "kg" | "千克" | "公斤" => Self::linear("kg", Mass, 1000.),
            "oz" | "盎司" => Self::linear("oz", Mass, 28.349523125),
            "lb" | "lbs" | "磅" => Self::linear("lb", Mass, 453.59237),
            "ns" | "纳秒" => Self::linear("ns", Time, 1e-9),
            "μs" | "µs" | "us" | "微秒" => Self::linear("μs", Time, 1e-6),
            "ms" | "毫秒" => Self::linear("ms", Time, 0.001),
            "s" | "sec" | "秒" => Self::linear("s", Time, 1.),
            "min" | "分钟" => Self::linear("min", Time, 60.),
            "h" | "hr" | "小时" => Self::linear("h", Time, 3600.),
            "d" | "day" | "days" | "天" => Self::linear("d", Time, 86400.),
            "c" | "°c" | "℃" | "摄氏度" => Self::temperature("°C", 1., 0.),
            "f" | "°f" | "℉" | "华氏度" => Self::temperature("°F", 5. / 9., 32.),
            "k" | "开尔文" => Self::temperature("K", 1., 273.15),
            "dc" | "deltac" | "Δ°c" | "δ°c" | "摄氏温差" => {
                Self::linear("Δ°C", TemperatureDifference, 1.)
            }
            "df" | "deltaf" | "Δ°f" | "δ°f" | "华氏温差" => {
                Self::linear("Δ°F", TemperatureDifference, 5. / 9.)
            }
            "dk" | "deltak" | "Δk" | "δk" | "开尔文温差" => {
                Self::linear("ΔK", TemperatureDifference, 1.)
            }
            "mm²" | "mm^2" | "mm2" | "平方毫米" => Self::linear("mm²", Area, 1e-6),
            "cm²" | "cm^2" | "cm2" | "平方厘米" => Self::linear("cm²", Area, 0.0001),
            "m²" | "m^2" | "m2" | "平方米" => Self::linear("m²", Area, 1.),
            "km²" | "km^2" | "km2" | "平方千米" | "平方公里" => {
                Self::linear("km²", Area, 1_000_000.)
            }
            "in²" | "in^2" | "in2" | "平方英寸" => {
                Self::linear("in²", Area, INCH_IN_METERS.powi(2))
            }
            "ft²" | "ft^2" | "ft2" | "平方英尺" => Self::linear("ft²", Area, 0.092_903_04),
            "yd²" | "yd^2" | "yd2" | "平方码" => Self::linear("yd²", Area, 0.9144_f64.powi(2)),
            "mi²" | "mi^2" | "mi2" | "平方英里" => {
                Self::linear("mi²", Area, METERS_PER_MILE.powi(2))
            }
            "ha" | "hectare" | "hectares" | "公顷" => Self::linear("ha", Area, 10_000.),
            "亩" | "市亩" | "mu" => Self::linear("亩", Area, 10_000. / 15.),
            "acre" | "acres" | "英亩" => Self::linear("acre", Area, 4_046.856_422_4),
            "ml" | "cm³" | "cm^3" | "cm3" | "毫升" => Self::linear("mL", Volume, 0.001),
            "l" | "liter" | "litre" | "升" => Self::linear("L", Volume, 1.),
            "m³" | "m^3" | "m3" | "立方米" => Self::linear("m³", Volume, 1000.),
            "usgal" | "美制加仑" => Self::linear("usgal", Volume, LITERS_PER_US_GALLON),
            "impgal" | "英制加仑" => Self::linear("impgal", Volume, LITERS_PER_IMPERIAL_GALLON),
            "m/s" | "米/秒" => Self::linear("m/s", Speed, 1.),
            "km/h" | "kph" | "千米/小时" | "公里/小时" => {
                Self::linear("km/h", Speed, 1. / 3.6)
            }
            "mph" | "英里/小时" => Self::linear("mph", Speed, 1_609.344 / 3_600.),
            "kn" | "knot" | "knots" | "节" => Self::linear("kn", Speed, 1_852. / 3_600.),
            "bit" | "bits" | "比特" => Self::linear("b", Data, 1.),
            "byte" | "bytes" | "字节" => Self::linear("B", Data, 8.),
            "°" | "deg" | "degree" | "degrees" | "度" | "角度" => {
                Self::linear("°", Angle, PI / 180.)
            }
            "rad" | "radian" | "radians" | "弧度" => Self::linear("rad", Angle, 1.),
            "′" | "arcmin" | "角分" => Self::linear("′", Angle, PI / 10_800.),
            "″" | "arcsec" | "角秒" => Self::linear("″", Angle, PI / 648_000.),
            "turn" | "turns" | "圈" => Self::linear("turn", Angle, TAU),
            "rpm" | "rev/min" | "r/min" | "圈/分" | "圈/分钟" => {
                Self::linear("rpm", AngularSpeed, TAU / 60.)
            }
            "rps" | "rev/s" | "r/s" | "圈/秒" => Self::linear("rev/s", AngularSpeed, TAU),
            "rad/s" | "弧度/秒" => Self::linear("rad/s", AngularSpeed, 1.),
            "°/s" | "deg/s" | "度/秒" => Self::linear("°/s", AngularSpeed, PI / 180.),
            "rad/min" | "弧度/分" | "弧度/分钟" => {
                Self::linear("rad/min", AngularSpeed, 1. / 60.)
            }
            "°/min" | "deg/min" | "度/分" | "度/分钟" => {
                Self::linear("°/min", AngularSpeed, PI / 10_800.)
            }
            "pa" | "帕" | "帕斯卡" => Self::linear("Pa", Pressure, 1.),
            "hpa" | "百帕" => Self::linear("hPa", Pressure, 100.),
            "kpa" | "千帕" => Self::linear("kPa", Pressure, 1000.),
            "毫帕" => Self::linear("mPa", Pressure, 0.001),
            "兆帕" => Self::linear("MPa", Pressure, 1_000_000.),
            "bar" | "巴" => Self::linear("bar", Pressure, 100_000.),
            "atm" | "标准大气压" => Self::linear("atm", Pressure, 101_325.),
            "torr" | "托" => Self::linear("Torr", Pressure, 101_325. / 760.),
            "psi" => Self::linear(
                "psi",
                Pressure,
                (0.453_592_37 * 9.806_65) / (0.0254 * 0.0254),
            ),
            "hz" | "赫兹" | "赫" => Self::linear("Hz", Frequency, 1.),
            "khz" | "千赫兹" | "千赫" => Self::linear("kHz", Frequency, 1000.),
            "毫赫兹" | "毫赫" => Self::linear("mHz", Frequency, 0.001),
            "兆赫兹" | "兆赫" => Self::linear("MHz", Frequency, 1_000_000.),
            "ghz" | "吉赫兹" | "吉赫" => Self::linear("GHz", Frequency, 1_000_000_000.),
            "thz" | "太赫兹" | "太赫" => Self::linear("THz", Frequency, 1_000_000_000_000.),
            "j" | "焦" | "焦耳" => Self::linear("J", Energy, 1.),
            "kj" | "千焦" | "千焦耳" => Self::linear("kJ", Energy, 1000.),
            "毫焦" | "毫焦耳" => Self::linear("mJ", Energy, 0.001),
            "兆焦" | "兆焦耳" => Self::linear("MJ", Energy, 1_000_000.),
            "wh" | "瓦时" => Self::linear("Wh", Energy, 3_600.),
            "kwh" | "千瓦时" | "度电" => Self::linear("kWh", Energy, 3_600_000.),
            "毫瓦时" => Self::linear("mWh", Energy, 3.6),
            "兆瓦时" => Self::linear("MWh", Energy, 3_600_000_000.),
            "w" | "瓦" | "瓦特" => Self::linear("W", Power, 1.),
            "kw" | "千瓦" | "千瓦特" => Self::linear("kW", Power, 1000.),
            "毫瓦" | "毫瓦特" => Self::linear("mW", Power, 0.001),
            "兆瓦" | "兆瓦特" => Self::linear("MW", Power, 1_000_000.),
            "gw" | "吉瓦" | "吉瓦特" => Self::linear("GW", Power, 1_000_000_000.),
            "kg/m³" | "kg/m^3" | "kg/m3" | "千克/立方米" | "公斤/立方米" => {
                Self::linear("kg/m³", Density, 1.)
            }
            "g/cm³" | "g/cm^3" | "g/cm3" | "克/立方厘米" => {
                Self::linear("g/cm³", Density, 1000.)
            }
            "g/l" | "克/升" => Self::linear("g/L", Density, 1.),
            "kg/l" | "千克/升" | "公斤/升" => Self::linear("kg/L", Density, 1000.),
            "m³/s" | "m^3/s" | "m3/s" | "立方米/秒" => Self::linear("m³/s", VolumeFlow, 1.),
            "m³/h" | "m^3/h" | "m3/h" | "立方米/小时" => {
                Self::linear("m³/h", VolumeFlow, 1. / 3600.)
            }
            "l/s" | "升/秒" => Self::linear("L/s", VolumeFlow, 0.001),
            "l/min" | "升/分钟" => Self::linear("L/min", VolumeFlow, 0.001 / 60.),
            "ml/s" | "毫升/秒" => Self::linear("mL/s", VolumeFlow, 0.000001),
            "n" | "牛" | "牛顿" => Self::linear("N", Force, 1.),
            "千牛" | "千牛顿" => Self::linear("kN", Force, 1000.),
            "毫牛" | "毫牛顿" => Self::linear("mN", Force, 0.001),
            "兆牛" | "兆牛顿" => Self::linear("MN", Force, 1_000_000.),
            "kgf" | "千克力" | "公斤力" => Self::linear("kgf", Force, STANDARD_GRAVITY),
            "lbf" | "磅力" => Self::linear("lbf", Force, NEWTONS_PER_POUND_FORCE),
            "dyn" | "达因" => Self::linear("dyn", Force, 1e-5),
            "n·m" | "n⋅m" | "n*m" | "n m" | "牛米" | "牛顿米" => {
                Self::linear("N·m", Torque, 1.)
            }
            "n·cm" | "n⋅cm" | "n*cm" | "n cm" | "牛厘米" => {
                Self::linear("N·cm", Torque, 0.01)
            }
            "kn·m" | "kn⋅m" | "kn*m" | "kn m" | "千牛米" => {
                Self::linear("kN·m", Torque, 1000.)
            }
            "kgf·m" | "kgf⋅m" | "kgf*m" | "kgf m" | "千克力米" | "公斤力米" => {
                Self::linear("kgf·m", Torque, STANDARD_GRAVITY)
            }
            "lbf·ft" | "lbf⋅ft" | "lbf*ft" | "lbf ft" | "ft·lbf" | "lb-ft" | "磅力英尺" => {
                Self::linear("lbf·ft", Torque, NEWTONS_PER_POUND_FORCE * FOOT_IN_METERS)
            }
            "lbf·in" | "lbf⋅in" | "lbf*in" | "lbf in" | "in·lbf" | "lb-in" | "磅力英寸" => {
                Self::linear("lbf·in", Torque, NEWTONS_PER_POUND_FORCE * INCH_IN_METERS)
            }
            "m/s²" | "m/s^2" | "m/s2" | "米/秒²" | "米/秒平方" => {
                Self::linear("m/s²", Acceleration, 1.)
            }
            "cm/s²" | "cm/s^2" | "cm/s2" | "gal" | "厘米/秒平方" => {
                Self::linear("cm/s²", Acceleration, 0.01)
            }
            "ft/s²" | "ft/s^2" | "ft/s2" | "英尺/秒平方" => {
                Self::linear("ft/s²", Acceleration, FOOT_IN_METERS)
            }
            "in/s²" | "in/s^2" | "in/s2" | "英寸/秒平方" => {
                Self::linear("in/s²", Acceleration, INCH_IN_METERS)
            }
            "g0" | "g₀" | "gₙ" | "标准重力加速度" => {
                Self::linear("g₀", Acceleration, STANDARD_GRAVITY)
            }
            "lx" | "lux" | "勒克斯" => Self::linear("lx", Illuminance, 1.),
            "klx" | "千勒克斯" => Self::linear("klx", Illuminance, 1_000.),
            "fc" | "footcandle" | "foot-candle" | "英尺烛光" | "尺烛光" => {
                Self::linear("fc", Illuminance, 1. / (FOOT_IN_METERS * FOOT_IN_METERS))
            }
            "phot" => Self::linear("phot", Illuminance, 10_000.),
            "a" | "amp" | "ampere" | "安" | "安培" => Self::linear("A", ElectricCurrent, 1.),
            "纳安" | "纳安培" => Self::linear("nA", ElectricCurrent, 1e-9),
            "微安" | "微安培" => Self::linear("μA", ElectricCurrent, 1e-6),
            "毫安" | "毫安培" => Self::linear("mA", ElectricCurrent, 1e-3),
            "千安" | "千安培" => Self::linear("kA", ElectricCurrent, 1e3),
            "兆安" | "兆安培" => Self::linear("MA", ElectricCurrent, 1e6),
            "v" | "volt" | "伏" | "伏特" => Self::linear("V", Voltage, 1.),
            "微伏" | "微伏特" => Self::linear("μV", Voltage, 1e-6),
            "毫伏" | "毫伏特" => Self::linear("mV", Voltage, 1e-3),
            "千伏" | "千伏特" => Self::linear("kV", Voltage, 1e3),
            "兆伏" | "兆伏特" => Self::linear("MV", Voltage, 1e6),
            "Ω" | "Ω" | "ohm" | "欧" | "欧姆" => Self::linear("Ω", Resistance, 1.),
            "毫欧" | "毫欧姆" => Self::linear("mΩ", Resistance, 1e-3),
            "千欧" | "千欧姆" => Self::linear("kΩ", Resistance, 1e3),
            "兆欧" | "兆欧姆" => Self::linear("MΩ", Resistance, 1e6),
            "吉欧" | "吉欧姆" => Self::linear("GΩ", Resistance, 1e9),
            "" => return Err(ConversionError::Incomplete),
            _ => return Err(ConversionError::UnknownUnit),
        };
        Ok(unit)
    }
}

fn data_unit(name: &str) -> Option<Unit> {
    let rate_base = name
        .strip_suffix("/s")
        .or_else(|| name.strip_suffix("/秒"))
        .or_else(|| name.strip_suffix("ps"));
    let dimension = if rate_base.is_some() {
        Dimension::DataRate
    } else {
        Dimension::Data
    };
    let name = rate_base.unwrap_or(name);
    // `B` 是字节、`b` 是比特，十进制和二进制前缀各自保持原比例。
    let (prefix, bytes) = if let Some(prefix) = name.strip_suffix('B') {
        (prefix, true)
    } else {
        (name.strip_suffix('b')?, false)
    };
    [
        ("", "b", "B", 1.),
        ("k", "kb", "kB", 1_000.),
        ("m", "Mb", "MB", 1_000_000.),
        ("g", "Gb", "GB", 1_000_000_000.),
        ("t", "Tb", "TB", 1_000_000_000_000.),
        ("ki", "Kib", "KiB", 1_024.),
        ("mi", "Mib", "MiB", 1_048_576.),
        ("gi", "Gib", "GiB", 1_073_741_824.),
        ("ti", "Tib", "TiB", 1_099_511_627_776.),
    ]
    .into_iter()
    .find(|(key, _, _, _)| prefix.eq_ignore_ascii_case(key))
    .map(|(_, bits, symbol, scale)| {
        if bytes {
            Unit::linear(symbol, dimension, scale * 8.)
        } else {
            Unit::linear(bits, dimension, scale)
        }
    })
}

pub(super) fn split_conversion(input: &str) -> Option<(&str, &str)> {
    let folded = input.to_ascii_lowercase();
    let (start, end) = [
        "换算成",
        "换算为",
        "转换成",
        "转换为",
        "换成",
        "转成",
        "转为",
        "换算",
        "转换",
        "转",
        "→",
        "->",
        " to ",
    ]
    .into_iter()
    .find_map(|separator| {
        folded
            .rfind(separator)
            .map(|start| (start, start + separator.len()))
    })
    .or_else(|| {
        folded
            .strip_suffix(" to")
            .map(|source| (source.len(), input.len()))
    })
    .or_else(|| {
        folded
            .rfind(" in ")
            .map(|start| (start, start + " in ".len()))
    })?;
    Some((input[..start].trim(), &input[end..]))
}

fn split_quantity(source: &str) -> (&str, &str) {
    if source.starts_with('(') {
        let mut depth = 0;
        for (index, character) in source.char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return source.split_at(index + 1);
                    }
                }
                _ => {}
            }
        }
    }
    source.rsplit_once(' ').unwrap_or_else(|| {
        let unit_start = source
            .find(|ch: char| !matches!(ch, '0'..='9' | '+' | '-' | '.' | 'e' | 'E' | '_'))
            .unwrap_or(source.len());
        source.split_at(unit_start)
    })
}

pub(super) fn exact_byte_quantity(source: &str) -> Option<u64> {
    let normalized = normalize(source);
    let (amount, unit) = split_quantity(normalized.trim());
    let multiplier = if unit.is_empty() {
        1
    } else {
        let unit = Unit::parse(unit).ok()?;
        if unit.dimension != Dimension::Data || !unit.symbol.ends_with('B') {
            return None;
        }
        // 数据单位表中的字节倍数均为可以精确表示的小整数。
        (unit.scale / 8.) as u64
    };
    let bytes = parse_fraction_literal(amount).ok()? * BigInt::from(multiplier);
    if !bytes.is_integer() {
        return None;
    }
    u64::try_from(bytes.numer()).ok()
}

fn feet_and_inches(source: &str) -> Option<(&str, &str)> {
    let folded = source.to_ascii_lowercase();
    let (feet, rest) = ["英尺", "feet", "foot", "ft", "′", "'"]
        .into_iter()
        .find_map(|unit| folded.split_once(unit))?;
    let inches_start = source.len() - rest.len();
    let inches = ["英寸", "inches", "inch", "in", "″", "\""]
        .into_iter()
        .find_map(|unit| rest.trim_end().strip_suffix(unit))?;
    Some((
        &source[..feet.len()],
        &source[inches_start..][..inches.len()],
    ))
}

fn degrees_minutes_seconds(source: &str) -> Option<(&str, &str, &str)> {
    let (degrees, rest) = source.split_once(['°', '度'])?;
    let (minutes, seconds) = rest.split_once(['′', '\'', '分'])?;
    let seconds = seconds.trim();
    let seconds = if seconds.is_empty() {
        "0"
    } else {
        seconds.strip_suffix(['″', '"', '秒'])?
    };
    Some((degrees, minutes, seconds))
}

fn parse_quantity(source: &str) -> Result<(f64, Unit), ConversionError> {
    let evaluate_amount =
        |amount: &str| evaluate(amount).map_err(|_| ConversionError::InvalidAmount);
    if let Some((degrees, minutes, seconds)) = degrees_minutes_seconds(source) {
        let degrees = evaluate_amount(degrees)?;
        let minutes = evaluate_amount(minutes)?;
        let seconds = evaluate_amount(seconds)?;
        if !(0. ..60.).contains(&minutes) || !(0. ..60.).contains(&seconds) {
            return Err(ConversionError::InvalidAmount);
        }
        let amount = (degrees.abs() + minutes / 60. + seconds / 3600.).copysign(degrees);
        return Ok((amount, Unit::parse("°")?));
    }
    if let Some((feet, inches)) = feet_and_inches(source) {
        let feet = evaluate_amount(feet)?;
        let inches = evaluate_amount(inches)?;
        if inches < 0. {
            return Err(ConversionError::InvalidAmount);
        }
        let amount = (feet.abs() * 12. + inches).copysign(feet);
        return Ok((amount, Unit::parse("in")?));
    }
    let (amount, name) = split_quantity(source);
    let quantity = Unit::parse(name).and_then(|unit| Ok((evaluate_amount(amount)?, unit)));
    quantity.or_else(|error| {
        let (amount, force) = amount.trim_end().rsplit_once(' ').ok_or(error)?;
        let torque = Unit::parse(&format!("{force} {}", name.trim()))
            .ok()
            .filter(|unit| unit.dimension == Dimension::Torque)
            .ok_or(error)?;
        Ok((evaluate_amount(amount)?, torque))
    })
}

pub(super) fn estimate_transfer(query: &str) -> Result<String, ConversionError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(ConversionError::TooLong);
    }
    let input = normalize(query).replace(char::is_whitespace, " ");
    if let Some((quantity, elapsed)) = input.split_once("用时") {
        return estimate_transfer_by_elapsed(quantity.trim(), elapsed.trim());
    }
    let (data, rate) = input
        .split_once('按')
        .or_else(|| {
            let index = input.to_ascii_lowercase().find(" at ")?;
            Some((&input[..index], &input[index + 4..]))
        })
        .ok_or(ConversionError::Incomplete)?;
    let (data, rate) = (data.trim(), rate.trim());
    if data.is_empty() || rate.is_empty() {
        return Err(ConversionError::Incomplete);
    }
    let (size, size_unit) = parse_quantity(data)?;
    let (speed, speed_unit) = parse_quantity(rate)?;
    if size_unit.dimension != Dimension::Data
        || speed_unit.dimension != Dimension::DataRate
        || size < 0.
        || speed <= 0.
    {
        return Err(ConversionError::InvalidTransfer);
    }
    let seconds = (size / speed) * (size_unit.scale / speed_unit.scale);
    if !seconds.is_finite() {
        return Err(ConversionError::OutOfRange);
    }
    convert_duration(&format!("{seconds:.3}s")).map_err(|_| ConversionError::OutOfRange)
}

fn estimate_transfer_by_elapsed(quantity: &str, elapsed: &str) -> Result<String, ConversionError> {
    if quantity.is_empty() || elapsed.is_empty() {
        return Err(ConversionError::Incomplete);
    }
    let (amount, unit) = parse_quantity(quantity)?;
    let elapsed = elapsed_duration(elapsed).map_err(|error| match error {
        DurationError::Incomplete => ConversionError::Incomplete,
        DurationError::OutOfRange => ConversionError::OutOfRange,
        _ => ConversionError::InvalidTransferDuration,
    })?;
    let seconds = elapsed.as_secs_f64();
    if amount < 0. {
        return Err(ConversionError::InvalidTransferDuration);
    }
    let (first, second) = match unit.dimension {
        Dimension::Data if seconds > 0. => ("MB/s", "Mbps"),
        Dimension::DataRate => ("MB", "MiB"),
        _ => return Err(ConversionError::InvalidTransferDuration),
    };
    let result = |symbol| {
        let target = Unit::parse(symbol)?;
        let scale = if unit.dimension == Dimension::DataRate {
            unit.scale / target.scale * seconds
        } else {
            unit.scale / (seconds * target.scale)
        };
        format_result(amount * scale).map_err(|_| ConversionError::OutOfRange)
    };
    Ok(format!(
        "{} {first} · {} {second}",
        result(first)?,
        result(second)?
    ))
}

pub(super) fn calculate_pace(query: &str) -> Result<String, ConversionError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(ConversionError::TooLong);
    }
    let input = normalize(query).replace(char::is_whitespace, " ");
    let input = input.trim();
    if input.is_empty() {
        return Err(ConversionError::Incomplete);
    }
    let seconds = |input: &str| {
        if input.is_empty() {
            return Err(ConversionError::Incomplete);
        }
        let elapsed = elapsed_duration(input).map_err(|error| match error {
            DurationError::Incomplete => ConversionError::Incomplete,
            DurationError::OutOfRange => ConversionError::OutOfRange,
            _ => ConversionError::InvalidPace,
        })?;
        if elapsed.is_zero() {
            return Err(ConversionError::InvalidPace);
        }
        Ok(elapsed.as_secs_f64())
    };
    let positive_quantity = |input: &str, dimension| {
        if input.is_empty() {
            return Err(ConversionError::Incomplete);
        }
        let (amount, unit) = parse_quantity(input)?;
        if amount <= 0. || unit.dimension != dimension {
            return Err(ConversionError::InvalidPace);
        }
        Ok(amount * unit.scale)
    };

    let distance_and_elapsed = input.split_once("用时").or_else(|| {
        let start = input.to_ascii_lowercase().rfind(" in ")?;
        Some((&input[..start], &input[start + 4..]))
    });
    let pace = input
        .rsplit_once('/')
        .and_then(|(elapsed, length)| Some((elapsed.trim(), Unit::parse(length).ok()?)))
        .filter(|(_, length)| length.dimension == Dimension::Length);
    if let Some((elapsed, length)) = pace {
        let speed = length.scale / seconds(elapsed)?;
        let result = |symbol| {
            let unit = Unit::parse(symbol)?;
            format_result(speed / unit.scale).map_err(|_| ConversionError::OutOfRange)
        };
        return Ok(format!("{} km/h · {} mph", result("km/h")?, result("mph")?));
    }

    let speed = if let Some((distance, elapsed)) = distance_and_elapsed {
        positive_quantity(distance.trim(), Dimension::Length)? / seconds(elapsed.trim())?
    } else {
        positive_quantity(input, Dimension::Speed)?
    };
    if !speed.is_finite() || speed <= 0. {
        return Err(ConversionError::OutOfRange);
    }
    let result = |symbol| {
        let unit = Unit::parse(symbol)?;
        let seconds = (unit.scale / speed).round();
        if !(0. ..u64::MAX as f64).contains(&seconds) {
            return Err(ConversionError::OutOfRange);
        }
        let seconds = seconds as u64;
        Ok(format!("{}:{:02} /{symbol}", seconds / 60, seconds % 60))
    };
    Ok(format!("{} · {}", result("km")?, result("mi")?))
}

pub(super) fn calculate_pixel_density(query: &str) -> Result<String, ConversionError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(ConversionError::TooLong);
    }
    let input = normalize(query).replace(char::is_whitespace, " ");
    let (resolution, diagonal) = input
        .split_once(['@', '按'])
        .ok_or(ConversionError::Incomplete)?;
    let (width, height) = pixel_dimensions(resolution).map_err(|error| match error {
        AspectRatioError::Incomplete => ConversionError::Incomplete,
        _ => ConversionError::InvalidPixelDensity,
    })?;
    let diagonal = diagonal.trim();
    if diagonal.is_empty() {
        return Err(ConversionError::Incomplete);
    }
    let (length, unit) = parse_quantity(diagonal)?;
    if length <= 0. || unit.dimension != Dimension::Length {
        return Err(ConversionError::InvalidPixelDensity);
    }
    let meters = length * unit.scale;
    let (width, height) = (f64::from(width), f64::from(height));
    let pixel_diagonal = width.hypot(height);
    let density = pixel_diagonal / (meters / INCH_IN_METERS);
    let width_cm = (width / pixel_diagonal) * meters * 100.;
    let height_cm = (height / pixel_diagonal) * meters * 100.;
    if [density, width_cm, height_cm]
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.)
    {
        return Err(ConversionError::OutOfRange);
    }
    Ok(format!(
        "{density:.2} PPI · {width_cm:.2} × {height_cm:.2} cm"
    ))
}

#[derive(Clone, Copy)]
struct FuelUnit {
    aliases: &'static [&'static str],
    label: &'static str,
    factor: f64,
    reciprocal: bool,
}

const LITERS_PER_100_KM: FuelUnit = FuelUnit {
    aliases: &["l/100 km", "l/100km", "升/百公里", "升/100公里"],
    label: "L/100 km",
    factor: 100.,
    reciprocal: true,
};
const KM_PER_LITER: FuelUnit = FuelUnit {
    aliases: &["km/l", "公里/升", "千米/升"],
    label: "km/L",
    factor: 1.,
    reciprocal: false,
};
const MPG_US: FuelUnit = FuelUnit {
    aliases: &["mpg-us", "mpg us", "us mpg"],
    label: "mpg US",
    factor: METERS_PER_MILE / 1000. / LITERS_PER_US_GALLON,
    reciprocal: false,
};
const MPG_UK: FuelUnit = FuelUnit {
    aliases: &["mpg-uk", "mpg uk", "uk mpg", "imperial mpg"],
    label: "mpg UK",
    factor: METERS_PER_MILE / 1000. / LITERS_PER_IMPERIAL_GALLON,
    reciprocal: false,
};
const FUEL_UNITS: [FuelUnit; 4] = [LITERS_PER_100_KM, KM_PER_LITER, MPG_US, MPG_UK];

pub(super) fn calculate_fuel_economy(query: &str) -> Result<String, ConversionError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(ConversionError::TooLong);
    }
    let input = normalize(query).to_ascii_lowercase();
    let input = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if input.is_empty() {
        return Err(ConversionError::Incomplete);
    }
    let (source, target) = split_conversion(&input)
        .map_or((input.as_str(), None), |(source, target)| {
            (source, Some(target.trim()))
        });
    let (amount, source) = FUEL_UNITS
        .into_iter()
        .find_map(|unit| {
            unit.aliases.iter().find_map(|&name| {
                source
                    .strip_suffix(name)
                    .map(|amount| (amount.trim_end(), unit))
            })
        })
        .ok_or(ConversionError::InvalidFuelEconomy)?;
    if amount.is_empty() {
        return Err(ConversionError::Incomplete);
    }
    let amount = evaluate(amount).map_err(|_| ConversionError::InvalidAmount)?;
    if amount <= 0. {
        return Err(ConversionError::InvalidFuelEconomy);
    }
    let result = |target: FuelUnit| {
        // 以每升公里数为基准。跨两种计量方向时对数值取倒数。
        let scale = if target.reciprocal {
            target.factor / source.factor
        } else {
            source.factor / target.factor
        };
        let value = if source.reciprocal == target.reciprocal {
            amount * scale
        } else {
            scale / amount
        };
        if !value.is_finite() || value <= 0. {
            return Err(ConversionError::OutOfRange);
        }
        Ok(format!("{value:.2} {}", target.label))
    };
    if let Some(target) = target {
        if target.is_empty() {
            return Err(ConversionError::Incomplete);
        }
        let unit = FUEL_UNITS
            .into_iter()
            .find(|unit| unit.aliases.contains(&target))
            .ok_or(ConversionError::InvalidFuelEconomy)?;
        result(unit)
    } else if source.reciprocal {
        Ok(format!("{} · {}", result(MPG_US)?, result(MPG_UK)?))
    } else {
        result(LITERS_PER_100_KM)
    }
}

pub(super) fn calculate_unit_price(query: &str) -> Result<String, ConversionError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(ConversionError::TooLong);
    }
    let input = normalize(query).replace(char::is_whitespace, " ");
    let (source, target) =
        split_conversion(input.trim()).ok_or(ConversionError::InvalidUnitPrice)?;
    let (price, quantity) = source
        .split_once('/')
        .ok_or(ConversionError::InvalidUnitPrice)?;
    let price = price.trim();
    let currency_mark = |ch| matches!(ch, '$' | '¥' | '￥' | '€' | '£' | '₩' | '₹');
    let prefix = price.chars().next().filter(|&ch| currency_mark(ch));
    let (amount, currency) = match prefix {
        Some(mark) => {
            let (currency, amount) = price.split_at(mark.len_utf8());
            (amount, currency)
        }
        None => split_quantity(price),
    };
    let currency = currency.to_ascii_uppercase();
    let is_symbol = currency.chars().count() == 1 && currency.starts_with(currency_mark);
    let is_code = currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase());
    if currency != "元" && !is_symbol && !is_code {
        return Err(ConversionError::InvalidUnitPrice);
    }
    let price = evaluate(amount).map_err(|_| ConversionError::InvalidAmount)?;
    let (quantity, source_unit) = parse_quantity(quantity.trim())?;
    let target = target.trim();
    let (target_amount, target_unit, explicit_amount) = match Unit::parse(target) {
        Ok(unit) => (1., unit, false),
        Err(_) => {
            let (amount, unit) = parse_quantity(target)?;
            (amount, unit, true)
        }
    };
    if source_unit.dimension != target_unit.dimension {
        return Err(ConversionError::DifferentDimensions);
    }
    if !matches!(
        source_unit.dimension,
        Dimension::Length
            | Dimension::Mass
            | Dimension::Time
            | Dimension::Area
            | Dimension::Volume
            | Dimension::Data
            | Dimension::Energy
    ) || price < 0.
        || !quantity.is_finite()
        || quantity <= 0.
        || !target_amount.is_finite()
        || target_amount <= 0.
    {
        return Err(ConversionError::InvalidUnitPrice);
    }
    let value = (target_amount / quantity) * (target_unit.scale / source_unit.scale) * price;
    if value == 0. && price != 0. {
        return Err(ConversionError::OutOfRange);
    }
    let display = |number| format_result(number).map_err(|_| ConversionError::OutOfRange);
    let value = display(value)?;
    let price = if prefix.is_some() {
        format!("{currency}{value}")
    } else {
        format!("{value} {currency}")
    };
    let unit = target_unit.symbol;
    if explicit_amount {
        Ok(format!("{price} / {} {unit}", display(target_amount)?))
    } else {
        Ok(format!("{price} / {unit}"))
    }
}

enum CircuitInput {
    Voltage(f64),
    Current(f64),
    Resistance(f64),
}

impl CircuitInput {
    fn parse(input: &str) -> Result<Self, ConversionError> {
        let (amount, unit) = parse_quantity(input.trim())?;
        let value = amount * unit.scale;
        if !value.is_finite() || (value == 0. && amount != 0.) {
            return Err(ConversionError::OutOfRange);
        }
        match unit.dimension {
            Dimension::Voltage if value >= 0. => Ok(Self::Voltage(value)),
            Dimension::ElectricCurrent if value >= 0. => Ok(Self::Current(value)),
            Dimension::Resistance if value > 0. => Ok(Self::Resistance(value)),
            _ => Err(ConversionError::InvalidCircuit),
        }
    }
}

pub(super) fn calculate_circuit(query: &str) -> Result<String, ConversionError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(ConversionError::TooLong);
    }
    let input = normalize(query).replace(char::is_whitespace, " ");
    if let Some((operation, values)) = input.trim().split_once(' ') {
        match operation.to_ascii_lowercase().as_str() {
            "串联" | "series" => return equivalent_resistance(values, false),
            "并联" | "parallel" => return equivalent_resistance(values, true),
            "分压" | "voltage-divider" => return calculate_divider(values, Divider::Voltage),
            "负载分压" | "loaded-voltage-divider" => {
                return calculate_divider(values, Divider::LoadedVoltage);
            }
            "分流" | "current-divider" => return calculate_divider(values, Divider::Current),
            "rc" | "rc滤波" | "rc-filter" => return calculate_rc_filter(values),
            "rc充电" | "rc-charge" => return calculate_rc_transition(values, true),
            "rc放电" | "rc-discharge" => return calculate_rc_transition(values, false),
            "rc电容" | "rc-capacitance" => return calculate_rc_component(values, false),
            "rc电阻" | "rc-resistance" => return calculate_rc_component(values, true),
            "rc低通" | "rc-low-pass" => return calculate_rc_response(values, false),
            "rc高通" | "rc-high-pass" => return calculate_rc_response(values, true),
            "rc低通表" | "rc-low-pass-table" => return rc_response_table(values, false),
            "rc高通表" | "rc-high-pass-table" => return rc_response_table(values, true),
            "耗电" | "energy-use" => return calculate_energy_use(values),
            "电池能量" | "battery-energy" => return calculate_battery_energy(values),
            _ => {}
        }
    }
    let (left, right) = input
        .split_once(',')
        .ok_or(ConversionError::InvalidCircuit)?;
    use CircuitInput::{Current, Resistance, Voltage};
    let (value, unit, power, positive) =
        match (CircuitInput::parse(left)?, CircuitInput::parse(right)?) {
            (Voltage(voltage), Current(current)) | (Current(current), Voltage(voltage))
                if voltage > 0. && current > 0. =>
            {
                (voltage / current, "Ω", voltage * current, true)
            }
            (Voltage(_), Current(_)) | (Current(_), Voltage(_)) => {
                return Err(ConversionError::InvalidResistanceOperands);
            }
            (Voltage(voltage), Resistance(resistance))
            | (Resistance(resistance), Voltage(voltage)) => {
                let current = voltage / resistance;
                (current, "A", voltage * current, voltage > 0.)
            }
            (Current(current), Resistance(resistance))
            | (Resistance(resistance), Current(current)) => {
                let voltage = current * resistance;
                (voltage, "V", voltage * current, current > 0.)
            }
            _ => return Err(ConversionError::InvalidCircuit),
        };
    if positive && (value == 0. || power == 0.) {
        return Err(ConversionError::OutOfRange);
    }
    Ok(format!(
        "{} · {}",
        format_circuit_value(value, unit)?,
        format_circuit_value(power, "W")?
    ))
}

#[derive(Clone, Copy)]
enum Divider {
    Voltage,
    LoadedVoltage,
    Current,
}

fn calculate_divider(input: &str, divider: Divider) -> Result<String, ConversionError> {
    let invalid = match divider {
        Divider::Voltage => ConversionError::InvalidVoltageDivider,
        Divider::LoadedVoltage => ConversionError::InvalidLoadedVoltageDivider,
        Divider::Current => ConversionError::InvalidCurrentDivider,
    };
    let (source, resistors) = input.split_once('@').ok_or(invalid)?;
    let resistors: Vec<_> = split_top_level_commas(resistors).take(4).collect();
    let (first, second, load) = match (divider, resistors.as_slice()) {
        (Divider::LoadedVoltage, [first, second, load]) => (first, second, Some(*load)),
        (Divider::Voltage | Divider::Current, [first, second]) => (first, second, None),
        _ => return Err(invalid),
    };
    let parse = |value| {
        CircuitInput::parse(value).map_err(|error| match error {
            ConversionError::OutOfRange => error,
            _ => invalid,
        })
    };
    let (source, CircuitInput::Resistance(first), CircuitInput::Resistance(second)) =
        (parse(source)?, parse(first)?, parse(second)?)
    else {
        return Err(invalid);
    };
    let (magnitude, unit, share, other, other_factor) = match (divider, source, load) {
        (Divider::Voltage, CircuitInput::Voltage(voltage), _) => (voltage, "V", second, first, 1.0),
        (Divider::Current, CircuitInput::Current(current), _) => (current, "A", first, second, 1.0),
        (Divider::LoadedVoltage, CircuitInput::Voltage(voltage), Some(load)) => {
            let CircuitInput::Resistance(load) = parse(load)? else {
                return Err(invalid);
            };
            // 将并联等效电阻的除数移到另一项，避免直接计算过小的等效电阻。
            let smaller = second.min(load);
            let factor = 1.0 + smaller / second.max(load);
            (voltage, "V", smaller, first, factor)
        }
        _ => return Err(invalid),
    };
    let scale = share.max(other);
    let scaled_share = share / scale;
    let total = scaled_share + (other / scale) * other_factor;
    let fraction = scaled_share / total;
    let output = if magnitude == 0.0 || fraction.is_normal() {
        magnitude * fraction
    } else {
        // 电阻比太小时，用对数保留乘以输入量后仍可表示的结果。
        (magnitude.ln() + share.ln() - scale.ln() - total.ln()).exp()
    };
    if magnitude > 0.0 && output == 0.0 {
        return Err(ConversionError::OutOfRange);
    }
    format_circuit_value(output, unit)
}

fn rc_time_constant(input: &str) -> Result<f64, ConversionError> {
    let invalid = ConversionError::InvalidRcFilter;
    let mut components = split_top_level_commas(input);
    let (Some(resistance), Some(capacitance), None) =
        (components.next(), components.next(), components.next())
    else {
        return Err(invalid);
    };
    let resistance = rc_resistance(resistance)?;
    let (amount, scale) = rc_capacitance(capacitance)?;
    let farads = amount * scale;
    let time = if farads.is_normal() {
        resistance * farads
    } else {
        // 先换算电阻的倍率，保留电容换算成法拉时可能下溢的结果。
        (resistance * scale) * amount
    };
    if !time.is_finite() || time == 0.0 {
        return Err(ConversionError::OutOfRange);
    }
    Ok(time)
}

fn rc_resistance(input: &str) -> Result<f64, ConversionError> {
    let invalid = ConversionError::InvalidRcFilter;
    let CircuitInput::Resistance(resistance) = CircuitInput::parse(input).map_err(|error| {
        if error == ConversionError::OutOfRange {
            error
        } else {
            invalid
        }
    })?
    else {
        return Err(invalid);
    };
    Ok(resistance)
}

fn rc_capacitance(input: &str) -> Result<(f64, f64), ConversionError> {
    let invalid = ConversionError::InvalidRcFilter;
    let farads = input.trim().strip_suffix(['F', 'f']).ok_or(invalid)?;
    let (amount, scale) = [
        ('m', 1e-3),
        ('μ', 1e-6),
        ('µ', 1e-6),
        ('u', 1e-6),
        ('n', 1e-9),
        ('p', 1e-12),
    ]
    .into_iter()
    .find_map(|(prefix, scale)| farads.strip_suffix(prefix).map(|amount| (amount, scale)))
    .unwrap_or((farads, 1.0));
    let amount = evaluate(amount.trim()).map_err(|_| invalid)?;
    if amount <= 0.0 {
        return Err(invalid);
    }
    Ok((amount, scale))
}

fn calculate_rc_filter(input: &str) -> Result<String, ConversionError> {
    let time = rc_time_constant(input)?;
    let frequency = TAU.recip() / time;
    let time = format_positive_quantity(time, &Unit::parse(default_time_unit(time))?)?;
    let frequency =
        format_positive_quantity(frequency, &Unit::parse(default_frequency_unit(frequency))?)?;
    Ok(format!("{time} · {frequency}"))
}

fn calculate_rc_response(input: &str, high_pass: bool) -> Result<String, ConversionError> {
    let (tau, frequency) = rc_response_arguments(input)?;
    let [amplitude, decibels, phase] = rc_response_at(tau, frequency, high_pass)?;
    let phase = if phase.is_empty() {
        "相位未定义".to_owned()
    } else {
        format!("{phase}°")
    };
    Ok(format!("{amplitude} · {decibels} dB · {phase}"))
}

fn rc_response_table(input: &str, high_pass: bool) -> Result<String, ConversionError> {
    let (tau, frequencies) = rc_response_arguments(input)?;
    let frequencies = serde_json::from_str::<Vec<String>>(frequencies)
        .map_err(|_| ConversionError::InvalidRcResponseTable)?;
    if !(1..=32).contains(&frequencies.len()) {
        return Err(ConversionError::InvalidRcResponseTable);
    }
    let mut output = String::from("频率,幅值比,增益dB,相位°");
    for frequency in frequencies {
        let [amplitude, decibels, phase] = rc_response_at(tau, &frequency, high_pass)?;
        let frequency = csv_cell(&frequency);
        let phase = csv_cell(&phase);
        output.push_str(&format!("\r\n{frequency},{amplitude},{decibels},{phase}"));
    }
    Ok(output)
}

fn rc_response_arguments(input: &str) -> Result<(f64, &str), ConversionError> {
    let invalid = ConversionError::InvalidRcResponse;
    let (components, frequency) = input.split_once('@').ok_or(invalid)?;
    let tau = rc_time_constant(components).map_err(|error| match error {
        ConversionError::InvalidRcFilter => invalid,
        _ => error,
    })?;
    Ok((tau, frequency))
}

fn rc_response_at(tau: f64, input: &str, high_pass: bool) -> Result<[String; 3], ConversionError> {
    let invalid = ConversionError::InvalidRcResponse;
    let (frequency, unit) = parse_quantity(input.trim()).map_err(|_| invalid)?;
    if frequency < 0.0 || unit.dimension != Dimension::Frequency {
        return Err(invalid);
    }
    if frequency == 0.0 {
        let parts = if high_pass {
            ["0", "−∞", ""]
        } else {
            ["1", "0", "0"]
        };
        return Ok(parts.map(str::to_owned));
    }
    let log_omega_tau = TAU.ln() + tau.ln() + frequency.ln() + unit.scale.ln();
    let (log_ratio, phase_sign) = if high_pass {
        (-log_omega_tau, 1.0)
    } else {
        (log_omega_tau, -1.0)
    };
    // 对数形式避免极端频率下溢出，并保留通带内的小衰减。
    let loss = log_ratio.max(0.0) + (-2.0 * log_ratio.abs()).exp().ln_1p() / 2.0;
    let display = |number| format_result(number).map_err(|_| ConversionError::OutOfRange);
    let amplitude = display((-loss).exp())?;
    let decibels = display(-20.0 * loss / LN_10)?;
    let phase = display(phase_sign * log_ratio.exp().atan().to_degrees())?;
    Ok([amplitude, decibels, phase])
}

fn calculate_rc_transition(input: &str, charging: bool) -> Result<String, ConversionError> {
    let invalid = if charging {
        ConversionError::InvalidRcCharge
    } else {
        ConversionError::InvalidRcDischarge
    };
    let (ratio, components) = input.split_once('@').ok_or(invalid)?;
    let ratio = parse_probability(ratio.trim()).map_err(|_| invalid)?;
    let one = BigRational::from_integer(1.into());
    let remaining = if charging { &one - ratio } else { ratio };
    if remaining.numer().bits() == 0 {
        return Err(invalid);
    }
    let tau = rc_time_constant(components).map_err(|error| match error {
        ConversionError::InvalidRcFilter => invalid,
        _ => error,
    })?;
    if remaining == one {
        return Ok("0 s".to_owned());
    }
    // 先用精确分数求补量，避免接近初始值时有效位丢失。
    let use_complement = remaining >= BigRational::new(1.into(), 2.into());
    let portion = if use_complement {
        one - remaining
    } else {
        remaining
    };
    let portion = format_significant_digits(portion, 17)
        .parse::<f64>()
        .map_err(|_| ConversionError::OutOfRange)?;
    let factor = if use_complement {
        -(-portion).ln_1p()
    } else {
        -portion.ln()
    };
    let time = tau * factor;
    format_positive_quantity(time, &Unit::parse(default_time_unit(time))?)
}

fn calculate_rc_component(input: &str, find_resistance: bool) -> Result<String, ConversionError> {
    let invalid = ConversionError::InvalidRcDesign;
    let (known, frequency) = input.split_once('@').ok_or(invalid)?;
    let (frequency, unit) = parse_quantity(frequency.trim()).map_err(|_| invalid)?;
    if frequency <= 0.0 || unit.dimension != Dimension::Frequency {
        return Err(invalid);
    }
    let (known, scale) = if find_resistance {
        rc_capacitance(known)
    } else {
        rc_resistance(known).map(|value| (value, 1.0))
    }
    .map_err(|error| match error {
        ConversionError::InvalidRcFilter => invalid,
        _ => error,
    })?;
    let product = (known * scale) * (frequency * unit.scale);
    let output = if product.is_normal() {
        TAU.recip() / product
    } else {
        // 避免中间乘积溢出或下溢时丢失可表示的元件值。
        (TAU.recip().ln() - known.ln() - scale.ln() - frequency.ln() - unit.scale.ln()).exp()
    };
    if !output.is_finite() || output <= 0.0 {
        return Err(ConversionError::OutOfRange);
    }
    if find_resistance {
        return format_circuit_value(output, "Ω");
    }
    let (factor, unit) = match output {
        value if value >= 1.0 => (1.0, "F"),
        value if value >= 1e-3 => (1e-3, "mF"),
        value if value >= 1e-6 => (1e-6, "μF"),
        value if value >= 1e-9 => (1e-9, "nF"),
        _ => (1e-12, "pF"),
    };
    let amount = format_result(output / factor).map_err(|_| ConversionError::OutOfRange)?;
    Ok(format!("{amount} {unit}"))
}

fn calculate_battery_energy(input: &str) -> Result<String, ConversionError> {
    let (capacity, voltage) = input
        .split_once('@')
        .ok_or(ConversionError::InvalidBatteryEnergy)?;
    let capacity = capacity.trim();
    let (capacity, amp_hour_scale) = capacity
        .strip_suffix("mAh")
        .or_else(|| capacity.strip_suffix("mah"))
        .map(|value| (value, 0.001))
        .or_else(|| {
            capacity
                .strip_suffix("Ah")
                .or_else(|| capacity.strip_suffix("ah"))
                .map(|value| (value, 1.))
        })
        .ok_or(ConversionError::InvalidBatteryEnergy)?;
    let capacity = evaluate(capacity.trim()).map_err(|_| ConversionError::InvalidBatteryEnergy)?;
    let (voltage, unit) = parse_quantity(voltage.trim())?;
    if capacity < 0. || voltage <= 0. || unit.dimension != Dimension::Voltage {
        return Err(ConversionError::InvalidBatteryEnergy);
    }
    let watt_hours = (capacity * amp_hour_scale) * (voltage * unit.scale);
    if !watt_hours.is_finite() || (capacity > 0. && watt_hours == 0.) {
        return Err(ConversionError::OutOfRange);
    }
    let (value, unit) = if watt_hours >= 1_000. {
        (watt_hours / 1_000., "kWh")
    } else {
        (watt_hours, "Wh")
    };
    let value = format_result(value).map_err(|_| ConversionError::OutOfRange)?;
    Ok(format!("{value} {unit}"))
}

fn calculate_energy_use(input: &str) -> Result<String, ConversionError> {
    let (power, time) = input
        .split_once('@')
        .ok_or(ConversionError::InvalidEnergyUse)?;
    let (amount, unit) = parse_quantity(power.trim())?;
    if unit.dimension != Dimension::Power || amount < 0. {
        return Err(ConversionError::InvalidEnergyUse);
    }
    let seconds = elapsed_duration(time.trim())
        .map_err(|error| match error {
            DurationError::OutOfRange => ConversionError::OutOfRange,
            _ => ConversionError::InvalidEnergyUse,
        })?
        .as_secs_f64();
    let watt_seconds_per_unit = unit.scale * seconds;
    let kilowatt_hours = amount * (watt_seconds_per_unit / 3_600_000.);
    let (result, unit) = if kilowatt_hours >= 1. {
        (kilowatt_hours, "kWh")
    } else {
        (amount * (watt_seconds_per_unit / 3600.), "Wh")
    };
    if !result.is_finite() || (amount > 0. && seconds > 0. && result == 0.) {
        return Err(ConversionError::OutOfRange);
    }
    let result = format_result(result).map_err(|_| ConversionError::OutOfRange)?;
    Ok(format!("{result} {unit}"))
}

fn equivalent_resistance(input: &str, parallel: bool) -> Result<String, ConversionError> {
    const MAX_RESISTORS: usize = 32;
    let resistors = split_top_level_commas(input)
        .take(MAX_RESISTORS + 1)
        .collect::<Vec<_>>();
    if !(2..=MAX_RESISTORS).contains(&resistors.len()) {
        return Err(ConversionError::InvalidResistanceList);
    }
    let resistors = resistors
        .into_iter()
        .map(|term| match CircuitInput::parse(term) {
            Ok(CircuitInput::Resistance(value)) => Ok(value),
            Err(ConversionError::OutOfRange) => Err(ConversionError::OutOfRange),
            _ => Err(ConversionError::InvalidResistanceList),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result = if parallel {
        // 以最小电阻为基准，避免直接取很小电阻的倒数时溢出。
        let minimum = resistors.iter().copied().fold(f64::INFINITY, f64::min);
        minimum / resistors.iter().map(|value| minimum / value).sum::<f64>()
    } else {
        resistors.into_iter().sum::<f64>()
    };
    if result == 0. {
        return Err(ConversionError::OutOfRange);
    }
    format_circuit_value(result, "Ω")
}

fn format_circuit_value(value: f64, unit: &str) -> Result<String, ConversionError> {
    let (factor, prefix) = match value {
        value if value >= 1e9 => (1e9, "G"),
        value if value >= 1e6 => (1e6, "M"),
        value if value >= 1e3 => (1e3, "k"),
        value if value > 0. && value < 1e-3 => (1e-6, "μ"),
        value if value > 0. && value < 1. => (1e-3, "m"),
        _ => (1., ""),
    };
    let value = format_result(value / factor).map_err(|_| ConversionError::OutOfRange)?;
    Ok(format!("{value} {prefix}{unit}"))
}

pub(super) fn convert_period(query: &str) -> Result<String, ConversionError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(ConversionError::TooLong);
    }
    let input = normalize(query).replace(char::is_whitespace, " ");
    let input = input.trim();
    let (source, target) = split_conversion(input).map_or((input, None), |(source, target)| {
        (source, Some(target.trim()))
    });
    let (amount, source) = parse_quantity(source)?;
    if amount <= 0. {
        return Err(ConversionError::InvalidPeriod);
    }
    let base = amount * source.scale;
    let reciprocal = base.recip();
    if !reciprocal.is_finite() || reciprocal == 0. {
        return Err(ConversionError::OutOfRange);
    }
    let (dimension, default) = match source.dimension {
        Dimension::Frequency => (Dimension::Time, default_time_unit(reciprocal)),
        Dimension::Time => (Dimension::Frequency, default_frequency_unit(reciprocal)),
        _ => return Err(ConversionError::InvalidPeriod),
    };
    let target = target.unwrap_or(default);
    if target.is_empty() {
        return Err(ConversionError::Incomplete);
    }
    let target = Unit::parse(target)?;
    if dimension != target.dimension {
        return Err(ConversionError::InvalidPeriodTarget);
    }
    format_positive_quantity(reciprocal, &target)
}

fn default_time_unit(value: f64) -> &'static str {
    match value {
        value if value >= 1. => "s",
        value if value >= 1e-3 => "ms",
        value if value >= 1e-6 => "us",
        _ => "ns",
    }
}

fn default_frequency_unit(value: f64) -> &'static str {
    match value {
        value if value >= 1e9 => "GHz",
        value if value >= 1e6 => "MHz",
        value if value >= 1e3 => "kHz",
        value if value < 1. => "mHz",
        _ => "Hz",
    }
}

pub(super) fn convert_wave(query: &str) -> Result<String, ConversionError> {
    const VACUUM_LIGHT_SPEED: f64 = 299_792_458.;

    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(ConversionError::TooLong);
    }
    let input = normalize(query).replace(char::is_whitespace, " ");
    let input = input.trim();
    let (source, target) = split_conversion(input).map_or((input, None), |(source, target)| {
        (source, Some(target.trim()))
    });
    let (operation, arguments) = source
        .split_once(char::is_whitespace)
        .unwrap_or((source, ""));
    if matches!(operation, "波速" | "速度") || operation.eq_ignore_ascii_case("speed") {
        return calculate_wave_speed(arguments, target);
    }
    let (source, speed) = if let Some((source, speed)) = source.split_once(['@', '＠']) {
        let (amount, unit) =
            parse_quantity(speed.trim()).map_err(|_| ConversionError::InvalidWaveSpeed)?;
        if unit.dimension != Dimension::Speed || amount <= 0. {
            return Err(ConversionError::InvalidWaveSpeed);
        }
        (source.trim_end(), amount * unit.scale)
    } else {
        (source, VACUUM_LIGHT_SPEED)
    };
    let (amount, unit) = parse_quantity(source)?;
    if amount <= 0. || !matches!(unit.dimension, Dimension::Frequency | Dimension::Length) {
        return Err(ConversionError::InvalidWave);
    }
    let value = speed / (amount * unit.scale);
    if !value.is_finite() || value <= 0. {
        return Err(ConversionError::OutOfRange);
    }
    let (dimension, default) = if unit.dimension == Dimension::Frequency {
        let default = match value {
            value if value >= 1e3 => "km",
            value if value >= 1. => "m",
            value if value >= 1e-2 => "cm",
            value if value >= 1e-3 => "mm",
            value if value >= 1e-6 => "um",
            _ => "nm",
        };
        (Dimension::Length, default)
    } else {
        let default = if value >= 1e12 {
            "THz"
        } else {
            default_frequency_unit(value)
        };
        (Dimension::Frequency, default)
    };
    let target = Unit::parse(target.unwrap_or(default))?;
    if target.dimension != dimension {
        return Err(ConversionError::InvalidWaveTarget);
    }
    format_positive_quantity(value, &target)
}

fn calculate_wave_speed(input: &str, target: Option<&str>) -> Result<String, ConversionError> {
    let (left, right) = input
        .split_once(['@', '＠'])
        .ok_or(ConversionError::InvalidWaveOperands)?;
    let (left_amount, left_unit) =
        parse_quantity(left.trim()).map_err(|_| ConversionError::InvalidWaveOperands)?;
    let (right_amount, right_unit) =
        parse_quantity(right.trim()).map_err(|_| ConversionError::InvalidWaveOperands)?;
    if left_amount <= 0.
        || right_amount <= 0.
        || !matches!(
            (&left_unit.dimension, &right_unit.dimension),
            (Dimension::Length, Dimension::Frequency) | (Dimension::Frequency, Dimension::Length)
        )
    {
        return Err(ConversionError::InvalidWaveOperands);
    }
    let target = Unit::parse(target.unwrap_or("m/s"))?;
    if target.dimension != Dimension::Speed {
        return Err(ConversionError::InvalidWaveSpeedTarget);
    }
    let speed = (left_amount * left_unit.scale) * (right_amount * right_unit.scale);
    format_positive_quantity(speed, &target)
}

fn format_positive_quantity(value: f64, target: &Unit) -> Result<String, ConversionError> {
    let result = value / target.scale;
    if result <= 0. {
        return Err(ConversionError::OutOfRange);
    }
    let result = format_result(result).map_err(|_| ConversionError::OutOfRange)?;
    Ok(format!("{result} {}", target.symbol))
}

pub(super) fn convert(query: &str) -> Option<Result<String, ConversionError>> {
    let input = normalize(query).replace(char::is_whitespace, " ");
    let (source, target) = split_conversion(input.trim())?;
    let base = Base::parse(target);
    let amount = if base.is_some() {
        source
    } else {
        split_quantity(source).0
    };
    if !looks_like_calculation(amount) {
        return None;
    }
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Some(Err(ConversionError::TooLong));
    }
    Some(match base {
        Some(base) => base.convert(amount),
        None => convert_quantity(source, target),
    })
}

pub(super) fn convert_exact_data(query: &str) -> Result<String, ConversionError> {
    let input = normalize(query).replace(char::is_whitespace, " ");
    let (source, target) = split_conversion(input.trim()).ok_or(ConversionError::Incomplete)?;
    let (bits, dimension) = exact_data_source(source)?;
    format_targets(target, |target| {
        let target = exact_data_unit(target)?;
        let converted = exact_data_target_amount(&bits, &dimension, &target)?;
        Ok(format_with_unit(converted, &target))
    })
}

pub(super) fn exact_data_csv_report(query: &str) -> Option<Result<String, ConversionError>> {
    batch_arguments(query.trim_start(), "单位表", "units-csv").map(exact_data_units_csv)
}

fn exact_data_units_csv(input: &str) -> Result<String, ConversionError> {
    let input = normalize(input).replace(char::is_whitespace, " ");
    let (units, source) = input
        .trim_start()
        .strip_prefix('[')
        .and_then(|input| input.split_once(']'))
        .ok_or(ConversionError::InvalidTargets)?;
    let units = parse_conversion_target_list(units)?;
    let (bits, dimension) = exact_data_source(source.trim())?;
    let mut rows = vec![["单位".to_owned(), "数值".to_owned()]];
    for unit in units {
        let target = exact_data_unit(unit)?;
        let amount = exact_data_target_amount(&bits, &dimension, &target)?;
        rows.push([unit.to_owned(), amount.to_string()]);
    }
    Ok(write_rows(&rows, ','))
}

pub(super) fn exact_data_converter(
    target: &str,
) -> Result<impl Fn(&str) -> Result<BigRational, ConversionError>, ConversionError> {
    let target = exact_data_unit(normalize(target).trim())?;
    Ok(move |source: &str| {
        let source = normalize(source).replace(char::is_whitespace, " ");
        let (bits, dimension) = exact_data_source(source.trim())?;
        exact_data_target_amount(&bits, &dimension, &target)
    })
}

pub(super) fn quantity_number_converter(
    target: &str,
) -> Result<impl Fn(&str) -> Result<String, ConversionError>, ConversionError> {
    let target = Unit::parse(normalize(target).trim())?;
    Ok(move |source: &str| {
        let source = normalize(source).replace(char::is_whitespace, " ");
        let (amount, source) = parse_quantity(source.trim())?;
        format_converted_amount(amount, &source, &target)
    })
}

fn exact_data_unit(input: &str) -> Result<Unit, ConversionError> {
    let unit = Unit::parse(input)?;
    if !matches!(unit.dimension, Dimension::Data | Dimension::DataRate) {
        return Err(ConversionError::ExactDataUnit);
    }
    Ok(unit)
}

fn exact_data_source(source: &str) -> Result<(BigRational, Dimension), ConversionError> {
    let (amount, source) = split_quantity(source);
    let source = exact_data_unit(source)?;
    let amount = calculate_fraction(amount).map_err(|_| ConversionError::InvalidAmount)?;
    // 数据量和数据速率单位表中的比例是可精确表示的整数。
    Ok((amount * BigInt::from(source.scale as u64), source.dimension))
}

fn exact_data_target_amount(
    bits: &BigRational,
    source_dimension: &Dimension,
    target: &Unit,
) -> Result<BigRational, ConversionError> {
    if source_dimension != &target.dimension {
        return Err(ConversionError::DifferentDimensions);
    }
    let converted = bits / BigInt::from(target.scale as u64);
    ensure_exact_size(&converted).map_err(|_| ConversionError::OutOfRange)?;
    Ok(converted)
}

pub(super) fn convert_batch(input: &str) -> Result<String, BatchError<ConversionError>> {
    map_batch_lines(input, convert_batch_line)
}

pub(super) fn convert_batch_csv(input: &str) -> Result<String, BatchError<ConversionError>> {
    map_batch_csv(input, "输入", convert_batch_line)
}

fn convert_batch_line(query: &str) -> Result<String, ConversionError> {
    convert(query).unwrap_or(Err(ConversionError::Incomplete))
}

pub(super) fn suggested_targets(query: &str) -> Option<Vec<InputSuggestion>> {
    let input = normalize(query).replace(char::is_whitespace, " ");
    let input = input.trim();
    let (source, stem, separator, prefix) = match split_conversion(input) {
        Some((source, target)) => {
            let prefix = target.trim();
            let stem = input[..input.len() - target.len()].trim_end();
            (source, stem, "", prefix)
        }
        None => (input, input, " to", ""),
    };
    let expression_for = |name: &str| {
        let expression = format!("{stem}{separator} {name}");
        (expression.chars().count() <= QUERY_LIMIT).then_some(expression)
    };
    let magnitude = source.strip_prefix(['-', '+']).unwrap_or(source);
    let (base, digits) = Base::prefixed(magnitude).unwrap_or((Base::Decimal, magnitude));
    if separator.is_empty() && base.parse_digits(digits).is_ok() {
        if Base::parse(prefix).is_some() {
            return None;
        }
        let items: Vec<_> = Base::matching_targets(prefix)
            .filter_map(|(name, base)| {
                Some(InputSuggestion {
                    expression: expression_for(name)?,
                    label: None,
                    result: base.convert(source).ok()?,
                })
            })
            .collect();
        return (!items.is_empty()).then_some(items);
    }
    if Unit::parse(prefix).is_ok() {
        return None;
    }
    let (amount, source) = parse_quantity(source).ok()?;
    let items: Vec<_> = source
        .dimension
        .suggested_units(source.symbol)
        .iter()
        .filter(|name| name.starts_with(prefix))
        .filter_map(|name| {
            let target = Unit::parse(name).ok()?;
            if target.symbol == source.symbol {
                return None;
            }
            Some(InputSuggestion {
                expression: expression_for(name)?,
                label: None,
                result: format_conversion(amount, &source, &target).ok()?,
            })
        })
        .collect();
    (!items.is_empty()).then_some(items)
}

pub(super) fn conversion_target_input(query: &str) -> Option<String> {
    let input = normalize(query).replace(char::is_whitespace, " ");
    let (source, _) = split_conversion(input.trim())?;
    let replacement = format!("{source} to ");
    suggested_targets(&replacement).map(|_| replacement)
}

fn convert_quantity(source: &str, target: &str) -> Result<String, ConversionError> {
    let (length, reference) = source
        .split_once('@')
        .map_or((source, None), |(length, reference)| {
            (length.trim_end(), Some(reference))
        });
    let (amount, unit) = split_quantity(length);
    // 紧贴数值的 em 中，e 可能被当作科学计数法的一部分。
    let (amount, unit) = amount
        .strip_suffix(['e', 'E'])
        .and_then(|number| {
            let suffix = &length[number.len()..];
            suffix
                .eq_ignore_ascii_case("em")
                .then_some((number, suffix))
        })
        .unwrap_or((amount, unit));
    if let Some(font_unit) = css_relative_unit(unit, target, &["rem", "em", "lh", "rlh"]) {
        return convert_font_relative_length(amount, unit, target, font_unit, reference);
    }
    if let Some(viewport_unit) = css_relative_unit(unit, target, &["vw", "vh", "vmin", "vmax"]) {
        return convert_viewport_length(amount, unit, target, viewport_unit, reference);
    }
    let (amount, source) = parse_quantity(source)?;
    format_targets(target, |target| {
        format_quantity_target(amount, &source, target)
    })
}

fn convert_font_relative_length(
    amount: &str,
    source: &str,
    target: &str,
    font_unit: &str,
    reference: Option<&str>,
) -> Result<String, ConversionError> {
    let (size, unit) = split_quantity(
        reference
            .ok_or(ConversionError::InvalidFontRelativeLength)?
            .trim(),
    );
    let size = evaluate(size).map_err(|_| ConversionError::InvalidFontRelativeLength)?;
    if !unit.trim().eq_ignore_ascii_case("px") || size <= 0. {
        return Err(ConversionError::InvalidFontRelativeLength);
    }
    format_scaled_css_length(
        amount,
        source,
        target,
        font_unit,
        size,
        ConversionError::InvalidFontRelativeLength,
    )
}

fn css_relative_unit(
    source: &str,
    target: &str,
    candidates: &[&'static str],
) -> Option<&'static str> {
    let source = source.trim();
    let target = target.trim();
    candidates
        .iter()
        .copied()
        .find(|name| name.eq_ignore_ascii_case(source) || name.eq_ignore_ascii_case(target))
}

fn convert_viewport_length(
    amount: &str,
    source: &str,
    target: &str,
    viewport_unit: &str,
    reference: Option<&str>,
) -> Result<String, ConversionError> {
    let (width, height) =
        pixel_dimensions(reference.ok_or(ConversionError::InvalidViewportLength)?)
            .map_err(|_| ConversionError::InvalidViewportLength)?;
    let pixels = match viewport_unit {
        "vw" => width,
        "vh" => height,
        "vmin" => width.min(height),
        "vmax" => width.max(height),
        _ => return Err(ConversionError::InvalidViewportLength),
    };
    let scale = f64::from(pixels) / 100.;
    format_scaled_css_length(
        amount,
        source,
        target,
        viewport_unit,
        scale,
        ConversionError::InvalidViewportLength,
    )
}

fn format_scaled_css_length(
    amount: &str,
    source: &str,
    target: &str,
    relative_unit: &str,
    scale: f64,
    invalid_pair: ConversionError,
) -> Result<String, ConversionError> {
    let amount = evaluate(amount).map_err(|_| ConversionError::InvalidAmount)?;
    let source = source.trim();
    let target = target.trim();
    let (value, unit) =
        if source.eq_ignore_ascii_case("px") && target.eq_ignore_ascii_case(relative_unit) {
            (amount / scale, relative_unit)
        } else if source.eq_ignore_ascii_case(relative_unit) && target.eq_ignore_ascii_case("px") {
            (amount * scale, "px")
        } else {
            return Err(invalid_pair);
        };
    let text = format_result(value).map_err(|_| ConversionError::OutOfRange)?;
    Ok(format!("{text}{unit}"))
}

fn format_targets(
    target: &str,
    mut format: impl FnMut(&str) -> Result<String, ConversionError>,
) -> Result<String, ConversionError> {
    let target = target.trim();
    if let Some(list) = target.strip_prefix('[') {
        let list = list
            .strip_suffix(']')
            .ok_or(ConversionError::InvalidTargets)?;
        return parse_conversion_target_list(list)?
            .into_iter()
            .map(format)
            .collect::<Result<Vec<_>, _>>()
            .map(|values| values.join("\n"));
    }
    format(target)
}

fn parse_conversion_target_list(list: &str) -> Result<Vec<&str>, ConversionError> {
    let targets: Vec<_> = list
        .split(',')
        .map(str::trim)
        .take(MAX_CONVERSION_TARGETS + 1)
        .collect();
    if targets.len() > MAX_CONVERSION_TARGETS || targets.contains(&"") {
        return Err(ConversionError::InvalidTargets);
    }
    Ok(targets)
}

fn format_quantity_target(
    amount: f64,
    source: &Unit,
    target: &str,
) -> Result<String, ConversionError> {
    match target.to_ascii_lowercase().as_str() {
        "ft in" | "feet inches" | "英尺英寸" | "英尺 英寸" => {
            format_feet_and_inches(amount, source)
        }
        "dms" | "度分秒" => format_degrees_minutes_seconds(amount, source),
        _ => {
            let target = Unit::parse(target)?;
            format_conversion(amount, source, &target)
        }
    }
}

fn format_feet_and_inches(amount: f64, source: &Unit) -> Result<String, ConversionError> {
    if source.dimension != Dimension::Length {
        return Err(ConversionError::DifferentDimensions);
    }
    let inch = Unit::parse("in")?;
    let total = amount.abs() * (source.scale / inch.scale);
    let remainder = total % 12.;
    let mut feet = ((total - remainder) / 12.).round();
    let mut inches = format_result(remainder).map_err(|_| ConversionError::OutOfRange)?;
    if inches == "12" {
        feet += 1.;
        inches = "0".to_owned();
    }
    let feet = format_result(feet).map_err(|_| ConversionError::OutOfRange)?;
    let sign = if amount < 0. { "-" } else { "" };
    Ok(format!("{sign}{feet} ft {inches} in"))
}

fn format_degrees_minutes_seconds(amount: f64, source: &Unit) -> Result<String, ConversionError> {
    if source.dimension != Dimension::Angle {
        return Err(ConversionError::DifferentDimensions);
    }
    let degree = Unit::parse("°")?;
    let total = amount.abs() * (source.scale / degree.scale);
    if !total.is_finite() {
        return Err(ConversionError::OutOfRange);
    }
    let mut degrees = total.floor();
    let minutes_with_fraction = (total - degrees) * 60.;
    let mut minutes = minutes_with_fraction.floor();
    let mut seconds = ((minutes_with_fraction - minutes) * 60. * 1_000_000.).round() / 1_000_000.;
    if seconds >= 60. {
        seconds = 0.;
        minutes += 1.;
    }
    if minutes >= 60. {
        minutes = 0.;
        degrees += 1.;
    }
    let sign = if amount < 0. && (degrees != 0. || minutes != 0. || seconds != 0.) {
        "-"
    } else {
        ""
    };
    let display = |value| format_result(value).map_err(|_| ConversionError::OutOfRange);
    Ok(format!(
        "{sign}{}° {}′ {}″",
        display(degrees)?,
        display(minutes)?,
        display(seconds)?
    ))
}

pub(super) fn convert_map_scale(query: &str) -> Result<String, ConversionError> {
    let normalized = normalize(query);
    let input = normalized.trim();
    let (first, rest) = input
        .split_once(char::is_whitespace)
        .ok_or(ConversionError::ScaleArguments)?;
    let command = first.to_ascii_lowercase();
    let command = command.as_str();
    if matches!(command, "求" | "ratio") {
        let (map, ground) = rest
            .split_once('=')
            .ok_or(ConversionError::ScaleArguments)?;
        let (map, map_unit) = parse_scale_length(map)?;
        let (ground, ground_unit) = parse_scale_length(ground)?;
        if map == 0. || ground == 0. {
            return Err(ConversionError::InvalidScaleLength);
        }
        let scale = (ground / map) * (ground_unit.scale / map_unit.scale);
        let scale =
            format_result(checked_scale_factor(scale)?).map_err(|_| ConversionError::OutOfRange)?;
        return Ok(format!("1:{scale}"));
    }

    let (ratio, operands) = if matches!(command, "图上" | "map" | "实际" | "ground") {
        rest.trim_start()
            .split_once(char::is_whitespace)
            .ok_or(ConversionError::ScaleArguments)?
    } else {
        (first, rest)
    };
    let ground_to_map = matches!(command, "图上" | "map");
    let (map, ground) = ratio
        .split_once(':')
        .ok_or(ConversionError::InvalidScaleRatio)?;
    let positive = |text| {
        evaluate(text)
            .ok()
            .filter(|&value| value > 0.)
            .ok_or(ConversionError::InvalidScaleRatio)
    };
    let (map, ground) = (positive(map)?, positive(ground)?);
    let factor = checked_scale_factor(if ground_to_map {
        map / ground
    } else {
        ground / map
    })?;
    let (source, target) =
        split_conversion(operands.trim()).ok_or(ConversionError::ScaleArguments)?;
    let (amount, source) = parse_scale_length(source)?;
    let target = Unit::parse(target.trim())?;
    if target.dimension != Dimension::Length {
        return Err(ConversionError::InvalidScaleLength);
    }
    format_conversion(amount * factor, &source, &target)
}

fn parse_scale_length(input: &str) -> Result<(f64, Unit), ConversionError> {
    let (amount, unit) = parse_quantity(input.trim())?;
    if unit.dimension != Dimension::Length || amount < 0. {
        return Err(ConversionError::InvalidScaleLength);
    }
    Ok((amount, unit))
}

fn checked_scale_factor(factor: f64) -> Result<f64, ConversionError> {
    if factor.is_finite() && factor > 0. {
        Ok(factor)
    } else {
        Err(ConversionError::OutOfRange)
    }
}

fn format_conversion(amount: f64, source: &Unit, target: &Unit) -> Result<String, ConversionError> {
    let value = format_converted_amount(amount, source, target)?;
    Ok(format_with_unit(value, target))
}

fn format_converted_amount(
    amount: f64,
    source: &Unit,
    target: &Unit,
) -> Result<String, ConversionError> {
    if source.dimension != target.dimension {
        return Err(ConversionError::DifferentDimensions);
    }

    let converted = if source.symbol == target.symbol {
        amount
    } else if source.dimension == Dimension::Temperature {
        (amount - source.origin) * (source.scale / target.scale) + target.origin
    } else {
        amount * (source.scale / target.scale)
    };
    format_result(converted).map_err(|_| ConversionError::OutOfRange)
}

fn format_with_unit(value: impl std::fmt::Display, target: &Unit) -> String {
    let suffix = if target.dimension == Dimension::DataRate {
        "/s"
    } else {
        ""
    };
    format!("{value} {}{suffix}", target.symbol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_between_diagram_and_ground_lengths_and_derives_the_scale() {
        for (query, expected) in [
            ("1:50000 3.2 cm to km", "1.6 km"),
            ("图上 1：50000 1.6 km 转 cm", "3.2 cm"),
            ("实际 1:24000 1 in to ft", "2000 ft"),
            ("求 3.2 cm = 1.6 km", "1:50000"),
            ("ratio 4 m = 2 m", "1:0.5"),
            ("2:3 0 cm to mm", "0 mm"),
        ] {
            assert_eq!(convert_map_scale(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("1:0 2 cm to m", ConversionError::InvalidScaleRatio),
            ("1:2 1 kg to g", ConversionError::InvalidScaleLength),
            ("1:2 1 cm to kg", ConversionError::InvalidScaleLength),
            ("1:2 -1 cm to m", ConversionError::InvalidScaleLength),
            ("求 0 mm = 10 m", ConversionError::InvalidScaleLength),
            ("1:2 2 m", ConversionError::ScaleArguments),
        ] {
            assert_eq!(convert_map_scale(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_chinese_and_square_imperial_area_units() {
        for (query, expected) in [
            ("2 ha to 亩", "30 亩"),
            ("3 市亩 to m²", "2000 m²"),
            ("1 平方公里 to mu", "1500 亩"),
            ("645.16 mm² to in²", "1 in²"),
            ("9 ft² to yd2", "1 yd²"),
            ("1 mi^2 to 英亩", "640 acre"),
        ] {
            assert_eq!(convert(query).unwrap().as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            convert("1 亩 to m"),
            Some(Err(ConversionError::DifferentDimensions))
        );
        let targets = suggested_targets("2 ha").unwrap();
        assert!(targets.iter().any(|item| item.expression == "2 ha to 亩"));
    }

    #[test]
    fn converts_data_amounts_and_rates_without_rounding_the_input_or_result() {
        for (query, expected) in [
            ("9007199254740993 B to KiB", "9007199254740993/1024 KiB"),
            ("(2**53 + 1)B to KiB", "9007199254740993/1024 KiB"),
            ("1/3 KiB to B", "1024/3 B"),
            ("1/3 KiB to [B,b,MiB]", "1024/3 B\n8192/3 b\n1/3072 MiB"),
            ("1 MiB/s to MB/s", "16384/15625 MB/s"),
            (
                "(1/3)MiB/s to [MB/s,KiB/s]",
                "16384/46875 MB/s\n1024/3 KiB/s",
            ),
            ("8 b to B", "1 B"),
            ("1 字节 to 比特", "8 b"),
            ("-1 kB to B", "-1000 B"),
        ] {
            assert_eq!(
                convert_exact_data(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            convert_exact_data("1 B to b/s"),
            Err(ConversionError::DifferentDimensions)
        );
        assert_eq!(
            convert_exact_data("3 m to cm"),
            Err(ConversionError::ExactDataUnit)
        );
        assert_eq!(
            convert_exact_data("1 B to [KiB,]"),
            Err(ConversionError::InvalidTargets)
        );
        assert_eq!(
            convert_exact_data("1 B to [KiB,MiB/s]"),
            Err(ConversionError::DifferentDimensions)
        );
    }

    #[test]
    fn calculates_pace_from_distance_and_time_and_converts_between_pace_and_speed() {
        for (query, expected) in [
            ("10 km 用时 52:30", "5:15 /km · 8:27 /mi"),
            ("１０ km IN 00:52:30", "5:15 /km · 8:27 /mi"),
            ("1 英里 用时 8分钟", "4:58 /km · 8:00 /mi"),
            ("12 km/h", "5:00 /km · 8:03 /mi"),
            ("5:00 /km", "12 km/h · 7.45645430684801 mph"),
            ("8:00 /mi", "12.07008 km/h · 7.5 mph"),
        ] {
            assert_eq!(calculate_pace(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("", ConversionError::Incomplete),
            ("10 km 用时", ConversionError::Incomplete),
            ("0 km 用时 5:00", ConversionError::InvalidPace),
            ("10 kg 用时 5:00", ConversionError::InvalidPace),
            ("12 km", ConversionError::InvalidPace),
            ("-12 km/h", ConversionError::InvalidPace),
            ("5:60 /km", ConversionError::InvalidPace),
            ("0:00 /mi", ConversionError::InvalidPace),
            ("1e-300 km/h", ConversionError::OutOfRange),
        ] {
            assert_eq!(calculate_pace(query), Err(error), "{query}");
        }
    }

    #[test]
    fn calculates_physical_pixel_density_with_a_length_unit_and_square_pixels() {
        for (query, expected) in [
            ("3840x2160 @ 27 in", "163.18 PPI · 59.77 × 33.62 cm"),
            (
                "１９２０×１０８０ 按 15.6英寸",
                "141.21 PPI · 34.54 × 19.43 cm",
            ),
            ("800:600 @ 63.5 cm", "40.00 PPI · 50.80 × 38.10 cm"),
            ("3 X 4 @ 10 in", "0.50 PPI · 15.24 × 20.32 cm"),
        ] {
            assert_eq!(
                calculate_pixel_density(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("1920x1080", ConversionError::Incomplete),
            ("1920x1080 @", ConversionError::Incomplete),
            ("1920x1080 @ 15 kg", ConversionError::InvalidPixelDensity),
            ("1920x1080 @ 0 in", ConversionError::InvalidPixelDensity),
            ("0x1080 @ 15 in", ConversionError::InvalidPixelDensity),
            ("1920x1080 @ 1e308 km", ConversionError::OutOfRange),
        ] {
            assert_eq!(calculate_pixel_density(query), Err(error), "{query}");
        }
    }

    #[test]
    fn calculates_missing_resistor_measurement_and_power() {
        for (query, expected) in [
            ("5 V, 20 Ω", "250 mA · 1.25 W"),
            ("12 V, 1.5 A", "8 Ω · 18 W"),
            ("220Ω,20mA", "4.4 V · 88 mW"),
            ("３００ｍＶ，３０ｍＡ", "10 Ω · 9 mW"),
            ("0 V, 1 kΩ", "0 A · 0 W"),
            ("0 A, 1 kΩ", "0 V · 0 W"),
        ] {
            assert_eq!(calculate_circuit(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["1 V, 2 V", "-1 V, 20 Ω", "5 V, 0 Ω", "1 m, 10 Ω"] {
            assert_eq!(
                calculate_circuit(query),
                Err(ConversionError::InvalidCircuit),
                "{query}"
            );
        }
        assert_eq!(
            calculate_circuit("0 V, 0 A"),
            Err(ConversionError::InvalidResistanceOperands)
        );
        assert_eq!(
            calculate_circuit("1e-300 V, 1e300 Ω"),
            Err(ConversionError::OutOfRange)
        );
    }

    #[test]
    fn converts_linearly_scaled_time_units_and_frequency_by_reciprocal() {
        for (query, expected) in [
            ("50 Hz to ms", "20 ms"),
            ("2.5 ms to Hz", "400 Hz"),
            ("１ＧＨｚ", "1 ns"),
            ("1 us", "1 MHz"),
            ("2.5 ms", "400 Hz"),
            ("2 mHz", "500 s"),
        ] {
            assert_eq!(convert_period(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("50 Hz to kHz", ConversionError::InvalidPeriodTarget),
            ("0 Hz", ConversionError::InvalidPeriod),
            ("-5 ms", ConversionError::InvalidPeriod),
            ("5 m", ConversionError::InvalidPeriod),
            ("2e-308 ns", ConversionError::OutOfRange),
        ] {
            assert_eq!(convert_period(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_wavelengths_and_frequencies_using_a_positive_propagation_speed() {
        for (query, expected) in [
            ("2.4 GHz to mm", "124.913524166667 mm"),
            ("1 MHz", "299.792458 m"),
            ("500 nm", "599.584916 THz"),
            ("1000 Hz @ 343 m/s to cm", "34.3 cm"),
            ("34.3 cm ＠ 343 m/s to kHz", "1 kHz"),
            ("10 Hz @ 36 km/h", "1 m"),
        ] {
            assert_eq!(convert_wave(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(convert("1 THz to GHz"), Some(Ok("1000 GHz".into())));
        for (query, error) in [
            ("0 Hz", ConversionError::InvalidWave),
            ("-5 m", ConversionError::InvalidWave),
            ("1 s", ConversionError::InvalidWave),
            ("1 Hz @ 0 m/s", ConversionError::InvalidWaveSpeed),
            ("1 Hz @ 5 kg", ConversionError::InvalidWaveSpeed),
            ("1 Hz @", ConversionError::InvalidWaveSpeed),
            ("1 Hz to MHz", ConversionError::InvalidWaveTarget),
            ("1 m to cm", ConversionError::InvalidWaveTarget),
            ("2e-308 nm", ConversionError::OutOfRange),
        ] {
            assert_eq!(convert_wave(query), Err(error), "{query}");
        }
    }

    #[test]
    fn calculates_wave_speed_from_frequency_and_wavelength_in_either_order() {
        for (query, expected) in [
            ("波速 1 kHz @ 34.3 cm", "343 m/s"),
            ("速度 34.3 cm ＠ 1 kHz to km/h", "1234.8 km/h"),
            ("SPEED 600 THz @ 500 nm to m/s", "300000000 m/s"),
        ] {
            assert_eq!(convert_wave(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("波速 1 kHz", ConversionError::InvalidWaveOperands),
            ("波速 1 kHz @ 2 Hz", ConversionError::InvalidWaveOperands),
            ("波速 0 cm @ 1 kHz", ConversionError::InvalidWaveOperands),
            (
                "波速 1 kHz @ 34.3 cm to kHz",
                ConversionError::InvalidWaveSpeedTarget,
            ),
            ("波速 1e308 Hz @ 1e308 m", ConversionError::OutOfRange),
        ] {
            assert_eq!(convert_wave(query), Err(error), "{query}");
        }
    }

    #[test]
    fn normalizes_prices_to_a_common_unit_or_quantity() {
        for (query, expected) in [
            ("36 元 / 750 g to kg", "48 元 / kg"),
            ("３７．５元／７５０克转５００克", "25 元 / 500 g"),
            ("$12 / 1.5 L to 250 mL", "$2 / 250 mL"),
            ("18 cny / 3 h to min", "0.1 CNY / min"),
            ("0 EUR / 50 cm to m", "0 EUR / m"),
        ] {
            assert_eq!(
                calculate_unit_price(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("12 元 / 0 kg to g", ConversionError::InvalidUnitPrice),
            ("12 元 / 1 kg to 0 g", ConversionError::InvalidUnitPrice),
            ("-12 元 / 1 kg to g", ConversionError::InvalidUnitPrice),
            ("12 元 / 1 kg to m", ConversionError::DifferentDimensions),
            ("12 元 / 1 °C to °F", ConversionError::InvalidUnitPrice),
            ("12 / 1 kg to g", ConversionError::InvalidUnitPrice),
        ] {
            assert_eq!(calculate_unit_price(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_fuel_economy_with_explicit_us_and_imperial_gallons() {
        for (query, expected) in [
            ("7.5 L/100 km", "31.36 mpg US · 37.66 mpg UK"),
            ("32 mpg-us to L/100 km", "7.35 L/100 km"),
            ("40 mpg UK", "7.06 L/100 km"),
            ("30 mpg UK to mpg-us", "24.98 mpg US"),
            ("5 升/百公里 转 km/L", "20.00 km/L"),
            ("２０ 公里/升", "5.00 L/100 km"),
            ("(2+3) L/100   km to L/100km", "5.00 L/100 km"),
        ] {
            assert_eq!(
                calculate_fuel_economy(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("", ConversionError::Incomplete),
            ("7.5 L/100 km to", ConversionError::Incomplete),
            ("30 mpg", ConversionError::InvalidFuelEconomy),
            ("0 L/100km", ConversionError::InvalidFuelEconomy),
            ("-10 km/L", ConversionError::InvalidFuelEconomy),
            ("7.5 L/100km to kg", ConversionError::InvalidFuelEconomy),
        ] {
            assert_eq!(calculate_fuel_economy(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_viewport_units_against_the_supplied_dimensions() {
        for (query, expected) in [
            ("2.5vw @ 1440×900 to px", "36px"),
            ("36px @ 1440x900 to vw", "2.5vw"),
            ("9px @ 1440x900 to vh", "1vh"),
            ("５０ＶＭＩＮ＠１４４０×９００ to px", "450px"),
            ("720px @ 900x1440 to vmax", "50vmax"),
            ("-0.5vh @ 1200x800 to px", "-4px"),
        ] {
            assert_eq!(convert(query).unwrap().as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "1vw to px",
            "1vh @ 1920x0 to px",
            "1vmin @ 1920 to px",
            "1vw @ 1200x800 to vh",
        ] {
            assert_eq!(
                convert(query),
                Some(Err(ConversionError::InvalidViewportLength)),
                "{query}"
            );
        }
    }

    #[test]
    fn converts_root_font_lengths_to_css_dimensions_with_an_explicit_reference() {
        for (query, expected) in [
            ("1.25rem @ 16px to px", "20px"),
            ("20 px @ 16 px to rem", "1.25rem"),
            ("−１．５ＲＥＭ＠１８ＰＸ → px", "-27px"),
            ("(1/4) rem @ 20px to px", "5px"),
            ("0px @ 16px to rem", "0rem"),
        ] {
            assert_eq!(convert(query).unwrap().as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "1rem to px",
            "1rem @ 0px to px",
            "8px @ -16px to rem",
            "1rem @ 12pt to px",
            "1rem @ 16px to in",
        ] {
            assert_eq!(
                convert(query),
                Some(Err(ConversionError::InvalidFontRelativeLength)),
                "{query}"
            );
        }
    }

    #[test]
    fn converts_relative_font_sizes_and_line_heights_with_the_given_px_reference() {
        for (query, expected) in [
            ("1.5em @ 18px to px", "27px"),
            ("1e1em @ 18px to px", "180px"),
            ("48px @ 24px to lh", "2lh"),
            ("3rlh @ 20px to px", "60px"),
            ("-24px @ 12px to EM", "-2em"),
        ] {
            assert_eq!(convert(query).unwrap().as_deref(), Ok(expected), "{query}");
        }
        for query in ["1em to px", "2lh @ 0px to px", "1em @ 16px to rem"] {
            assert_eq!(
                convert(query),
                Some(Err(ConversionError::InvalidFontRelativeLength)),
                "{query}"
            );
        }
    }

    #[test]
    fn converts_css_absolute_lengths_and_suggests_related_units() {
        for (query, expected) in [
            ("12 pt to px", "16 px"),
            ("96 csspx to in", "1 in"),
            ("1 pc to pt", "12 pt"),
            ("100 Q to cm", "2.5 cm"),
            ("72 pt to mm", "25.4 mm"),
        ] {
            assert_eq!(convert(query).unwrap().as_deref(), Ok(expected), "{query}");
        }
        let suggestions = suggested_targets("12 pt").unwrap();
        assert_eq!(suggestions[0].expression, "12 pt to px");
        assert_eq!(suggestions[0].result, "16 px");
    }

    #[test]
    fn converts_one_quantity_to_ordered_targets_including_batch_csv() {
        for (query, expected) in [
            ("1 GiB to [MiB, KiB, Gib]", "1024 MiB\n1048576 KiB\n8 Gib"),
            ("0 °C to [°F,K]", "32 °F\n273.15 K"),
            ("177.8 cm to [cm, ft in]", "177.8 cm\n5 ft 10 in"),
            ("3 m to [cm]", "300 cm"),
        ] {
            assert_eq!(
                convert(query).as_ref().map(|result| result.as_deref()),
                Some(Ok(expected)),
                "{query}"
            );
        }
        for query in [
            "3 m to [",
            "3 m to []",
            "3 m to [cm,]",
            "3 m to [cm,,mm]",
            "3 m to [m,m,m,m,m,m,m,m,m]",
        ] {
            assert_eq!(
                convert(query),
                Some(Err(ConversionError::InvalidTargets)),
                "{query}"
            );
        }
        assert_eq!(
            convert("3 m to [cm,kg]"),
            Some(Err(ConversionError::DifferentDimensions))
        );
        assert_eq!(
            convert("3 m to [unknown,cm]"),
            Some(Err(ConversionError::UnknownUnit))
        );
        assert_eq!(
            convert_batch_csv("3 m to [cm, mm]\n0xff to dec").as_deref(),
            Ok("输入,结果\r\n\"3 m to [cm, mm]\",\"300 cm\n3000 mm\"\r\n0xff to dec,255")
        );
    }

    #[test]
    fn converts_combinations_of_feet_and_inches_with_a_single_sign() {
        for (query, expected) in [
            ("5 ft 10 in to cm", "177.8 cm"),
            ("5ft10in to cm", "177.8 cm"),
            ("5'10\" to cm", "177.8 cm"),
            ("-5′10″ to cm", "-177.8 cm"),
            ("5英尺10.5英寸转厘米", "179.07 cm"),
            ("-0 ft 10 in to cm", "-25.4 cm"),
            ("5 feet 10 inches to ft", "5.83333333333333 ft"),
            ("177.8 cm to ft in", "5 ft 10 in"),
            ("30.48 cm to ft in", "1 ft 0 in"),
            ("-25.4 cm 转 英尺英寸", "-0 ft 10 in"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.to_owned())), "{query}");
        }
        assert_eq!(
            convert("5 ft -10 in to cm"),
            Some(Err(ConversionError::InvalidAmount))
        );
        assert_eq!(
            convert("1 kg to ft in"),
            Some(Err(ConversionError::DifferentDimensions))
        );
        assert!(suggested_targets("5 ft 10 in")
            .unwrap()
            .iter()
            .any(|item| { item.expression == "5 ft 10 in to cm" && item.result == "177.8 cm" }));
    }

    #[test]
    fn converts_micro_and_nano_units_and_suggests_nearby_scales() {
        for (query, expected) in [
            ("500 nm to um", "0.5 μm"),
            ("1 µm to nm", "1000 nm"),
            ("1000 μs to ms", "1 ms"),
            ("125 纳秒 转 微秒", "0.125 μs"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.to_owned())), "{query}");
        }
        for (query, expected) in [("500 nm", "0.5 μm"), ("125 ns", "0.125 μs")] {
            let results = suggested_targets(query).unwrap();
            assert_eq!(results[0].result, expected, "{query}");
        }
    }

    #[test]
    fn converts_electrical_units_with_case_sensitive_prefixes() {
        for (query, expected) in [
            ("3.3 V to mV", "3300 mV"),
            ("500mA to A", "0.5 A"),
            ("750 uA to mA", "0.75 mA"),
            ("2 mV to μV", "2000 μV"),
            ("1 MV to kV", "1000 kV"),
            ("2.2 kΩ to Ω", "2200 Ω"),
            ("1 MΩ to kohm", "1000 kΩ"),
            ("250 mohm to Ω", "0.25 Ω"),
            ("１.５ 千欧 转 欧姆", "1500 Ω"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.to_owned())), "{query}");
        }
        assert_eq!(
            convert("1 V to mA"),
            Some(Err(ConversionError::DifferentDimensions)),
        );
        let targets = suggested_targets("3.3 V to").unwrap();
        assert_eq!(targets[0].expression, "3.3 V to mV");
        assert_eq!(targets[0].result, "3300 mV");
    }

    #[test]
    fn converts_torque_units_and_preserves_nanometer_and_energy_unit_types() {
        for (query, expected) in [
            ("100 N·cm to N·m", "1 N·m"),
            ("1 lbf·ft to N·m", "1.3558179483314 N·m"),
            ("12 lb-in to N*m", "1.3558179483314 N·m"),
            ("1 kgf⋅m to 牛米", "9.80665 N·m"),
            ("2 kN m to N·m", "2000 N·m"),
            ("2   kN   m to N·m", "2000 N·m"),
            ("500 nm to μm", "0.5 μm"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.to_owned())), "{query}");
        }
        assert_eq!(
            convert("1 N·m to J"),
            Some(Err(ConversionError::DifferentDimensions)),
        );
        let targets = suggested_targets("12 N·m to").unwrap();
        assert_eq!(targets[0].expression, "12 N·m to lbf·ft");
        assert!(targets
            .iter()
            .any(|target| target.expression == "12 N·m to kgf·m"));
    }

    #[test]
    fn converts_illuminance_and_suggests_only_illuminance_units() {
        for (query, expected) in [
            ("1 foot-candle to lx", "10.7639104167097 lx"),
            ("100 勒克斯 to fc", "9.290304 fc"),
            ("2 klx to lux", "2000 lx"),
            ("2 phot to klx", "20 klx"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.to_owned())), "{query}");
        }
        assert_eq!(
            convert("100 lx to W"),
            Some(Err(ConversionError::DifferentDimensions)),
        );
        let targets = suggested_targets("100 lx to").unwrap();
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].expression, "100 lx to fc");
    }

    #[test]
    fn converts_force_and_acceleration_without_confusing_knots_or_mass() {
        for (query, expected) in [
            ("1 kgf to N", "9.80665 N"),
            ("1 lbf to N", "4.4482216152605 N"),
            ("2 kN to N", "2000 N"),
            ("2 kn to km/h", "3.704 km/h"),
            ("500 mN to N", "0.5 N"),
            ("1 MN to N", "1000000 N"),
            ("1 g0 to m/s^2", "9.80665 m/s²"),
            ("1 ft/s² to cm/s²", "30.48 cm/s²"),
            ("100 gal to m/s2", "1 m/s²"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.to_owned())), "{query}");
        }
        assert_eq!(
            convert("1 g to m/s²"),
            Some(Err(ConversionError::DifferentDimensions)),
        );
    }

    #[test]
    fn converts_density_and_volume_flow_using_separate_dimensions() {
        for (query, expected) in [
            ("1000 kg/m3 to g/cm3", "1 g/cm³"),
            ("1克/升 转 千克/立方米", "1 kg/m³"),
            ("1 L/min to m³/h", "0.06 m³/h"),
            ("1立方米/秒 转 升/秒", "1000 L/s"),
            ("1 mL/s to L/min", "0.06 L/min"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.to_owned())), "{query}");
        }
        assert_eq!(
            convert("1 L/min to kg/m³"),
            Some(Err(ConversionError::DifferentDimensions)),
        );
    }

    #[test]
    fn converts_degree_minute_second_angles_in_both_directions() {
        for (query, expected) in [
            ("30° 15′ 30″ to °", "30.2583333333333 °"),
            ("12度30分15秒转度", "12.5041666666667 °"),
            ("30°15'30\" to °", "30.2583333333333 °"),
            ("-0° 30′ to °", "-0.5 °"),
            ("12.5° to dms", "12° 30′ 0″"),
            ("-0.5° to 度分秒", "-0° 30′ 0″"),
            ("1 rad to dms", "57° 17′ 44.806247″"),
            ("359.99999999999° to dms", "360° 0′ 0″"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.into())), "{query}");
        }
        assert_eq!(
            convert("1°60′ to °"),
            Some(Err(ConversionError::InvalidAmount))
        );
        assert_eq!(
            convert("1°20′60″ to °"),
            Some(Err(ConversionError::InvalidAmount))
        );
        assert_eq!(
            convert("1 km to dms"),
            Some(Err(ConversionError::DifferentDimensions))
        );
        assert_eq!(convert("5′10″ to cm"), Some(Ok("177.8 cm".into())));
    }

    #[test]
    fn converts_temperature_differences_and_sexagesimal_angle_units() {
        for (query, expected) in [
            ("10 dC to dF", "18 Δ°F"),
            ("-18 Δ°F to ΔK", "-10 ΔK"),
            ("1开尔文温差转摄氏温差", "1 Δ°C"),
            ("10°C to °F", "50 °F"),
            ("60′ to °", "1 °"),
            ("3600″ to °", "1 °"),
            ("1.5° to arcmin", "90 ′"),
            ("0.5 turn to 角秒", "648000 ″"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.into())), "{query}");
        }
        for query in ["10°C to dF", "1 dK to K", "60 min to °"] {
            assert_eq!(
                convert(query),
                Some(Err(ConversionError::DifferentDimensions))
            );
        }
    }

    #[test]
    fn extends_conversion_choices_and_filters_partial_unit_symbols() {
        let items = suggested_targets("10 km to").unwrap();
        assert_eq!(items.len(), 9);
        assert_eq!(items[4].expression, "10 km to mm");
        assert_eq!(items[4].result, "10000000 mm");
        assert_eq!(suggested_targets("1 GiB to").unwrap().len(), 17);
        assert_eq!(suggested_targets("1 GiB/s to").unwrap().len(), 17);
        let items = suggested_targets("1 GiB to Ti").unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.expression.as_str())
                .collect::<Vec<_>>(),
            ["1 GiB to TiB", "1 GiB to Tib"],
        );
        let items = suggested_targets("1 m转y").unwrap();
        assert_eq!(items[0].expression, "1 m转 yd");
        assert!(suggested_targets("10 km to nonexistent").is_none());
    }

    #[test]
    fn estimates_transfer_time_using_storage_and_rate_units() {
        for (query, expected) in [
            ("1 GB 按 100 Mbps", "00:01:20"),
            ("1 GiB at 100 Mbps", "00:01:25.899"),
            ("2 GiB AT 10 MiB/s", "00:03:24.8"),
            ("600 Mb按 10 MB/s", "00:00:07.5"),
            ("0 B at 5 B/s", "00:00:00"),
        ] {
            assert_eq!(estimate_transfer(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, expected) in [
            ("1 GB 按", ConversionError::Incomplete),
            ("1 kg at 10 Mbps", ConversionError::InvalidTransfer),
            ("1 GB at 10 MB", ConversionError::InvalidTransfer),
            ("-2 GB at 100 Mbps", ConversionError::InvalidTransfer),
            ("1 GB at 0 Mbps", ConversionError::InvalidTransfer),
            ("1e308 TB at 1 b/s", ConversionError::OutOfRange),
        ] {
            assert_eq!(estimate_transfer(query), Err(expected), "{query}");
        }
    }

    #[test]
    fn estimates_average_transfer_rate_from_data_and_elapsed_time() {
        for (query, expected) in [
            ("600 MB 用时 2分钟", "5 MB/s · 40 Mbps"),
            ("16 Mb 用时 00:02", "1 MB/s · 8 Mbps"),
            ("1 GiB 用时 2秒", "536.870912 MB/s · 4294.967296 Mbps"),
            ("500 kB 用时 500ms", "1 MB/s · 8 Mbps"),
            ("0 B 用时 1s", "0 MB/s · 0 Mbps"),
        ] {
            assert_eq!(estimate_transfer(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("600 MB 用时", ConversionError::Incomplete),
            ("600 MB 用时 0秒", ConversionError::InvalidTransferDuration),
            (
                "600 MB 用时 -2分钟",
                ConversionError::InvalidTransferDuration,
            ),
            ("600 m 用时 2分钟", ConversionError::InvalidTransferDuration),
        ] {
            assert_eq!(estimate_transfer(query), Err(error), "{query}");
        }
    }

    #[test]
    fn estimates_transfer_data_from_rate_and_elapsed_time() {
        for (query, expected) in [
            ("100 Mbps 用时 2分钟", "1500 MB · 1430.51147460938 MiB"),
            ("1 MiB/s 用时 2秒", "2.097152 MB · 2 MiB"),
            ("8 Mbps 用时 500ms", "0.5 MB · 0.476837158203125 MiB"),
            ("8 Mbps 用时 0s", "0 MB · 0 MiB"),
            ("0 Mbps 用时 2s", "0 MB · 0 MiB"),
        ] {
            assert_eq!(estimate_transfer(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            estimate_transfer("-8 Mbps 用时 2s"),
            Err(ConversionError::InvalidTransferDuration)
        );
        assert_eq!(
            estimate_transfer("1e308 MB/s 用时 1h"),
            Err(ConversionError::OutOfRange)
        );
    }

    #[test]
    fn suggests_exact_radix_targets_for_integers() {
        for query in ["0xff to", "255 to"] {
            let items = suggested_targets(query).unwrap();
            assert_eq!(
                items
                    .iter()
                    .map(|item| item.result.as_str())
                    .collect::<Vec<_>>(),
                ["255", "0b11111111", "0o377", "0xff"]
            );
        }
        assert_eq!(
            suggested_targets("-0xFF to Bi").unwrap()[0].result,
            "-0b11111111"
        );
        assert_eq!(
            suggested_targets("0o377 转十六").unwrap()[0].expression,
            "0o377 转 hex"
        );
        let max = "0xffffffffffffffffffffffffffffffff to";
        assert_eq!(
            suggested_targets(max).unwrap()[0].result,
            u128::MAX.to_string()
        );
        assert!(suggested_targets("0x10 m to")
            .unwrap()
            .iter()
            .any(|item| item.result == "1600 cm"));
        for query in ["0xff", "255", "0xff to oct", "0x_1 to", "0xff to unknown"] {
            assert!(suggested_targets(query).is_none(), "{query}");
        }
    }

    #[test]
    fn suggests_compatible_targets_and_previews_their_conversions() {
        for (query, expression, result) in [
            ("10 km to", "10 km to mi", "6.21371192237334 mi"),
            ("10 in to", "10 in to m", "0.254 m"),
            ("8Mbps转", "8Mbps转 MB/s", "1 MB/s"),
            ("100°C to ", "100°C to °F", "212 °F"),
            ("500mW to", "500mW to W", "0.5 W"),
        ] {
            let items = suggested_targets(query).unwrap();
            assert!(
                items
                    .iter()
                    .any(|item| item.expression == expression && item.result == result),
                "{query}"
            );
            for item in items {
                assert_eq!(convert(&item.expression), Some(Ok(item.result)));
            }
        }
        for query in ["3.5 to", "10 km to mi", "100未知单位转"] {
            assert!(suggested_targets(query).is_none(), "{query}");
        }
    }

    #[test]
    fn converts_multiple_units_and_radices_in_order_and_reports_the_invalid_line() {
        assert_eq!(
            convert_batch("3 m to cm\r\n \r\n0xff to dec\n1 GiB to MiB").as_deref(),
            Ok("300 cm\n255\n1024 MiB")
        );
        assert_eq!(
            convert_batch("3 m to cm\n\n未知"),
            Err(BatchError::Row {
                line: 3,
                cause: ConversionError::Incomplete,
            })
        );
        assert_eq!(convert_batch(" \r\n "), Err(BatchError::Empty));
    }

    #[test]
    fn estimates_battery_energy_from_capacity_and_nominal_voltage() {
        for (query, expected) in [
            ("电池能量 2500 mAh @ 3.7 V", "9.25 Wh"),
            ("battery-energy 5/2Ah @ 3700mV", "9.25 Wh"),
            ("电池能量 5 Ah @ 250 V", "1.25 kWh"),
            ("电池能量 0 mAh @ 5 V", "0 Wh"),
        ] {
            assert_eq!(calculate_circuit(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "电池能量 2 Ah @ 0 V",
            "电池能量 -2 Ah @ 3 V",
            "电池能量 2 A @ 3 V",
            "电池能量 2 Ah @ 3 W",
        ] {
            assert_eq!(
                calculate_circuit(query),
                Err(ConversionError::InvalidBatteryEnergy),
                "{query}"
            );
        }
    }

    #[test]
    fn calculates_energy_use_from_power_and_elapsed_duration() {
        for (query, expected) in [
            ("耗电 1.5 kW @ 2h", "3 kWh"),
            ("energy-use 10 W @ PT30M", "5 Wh"),
            ("耗电 3600 mW @ 00:01", "0.001 Wh"),
            ("耗电 0 W @ 3h", "0 Wh"),
            ("耗电 10 W @ 0s", "0 Wh"),
        ] {
            assert_eq!(calculate_circuit(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "耗电 1 W",
            "耗电 12 V @ 2h",
            "耗电 -1 W @ 2h",
            "耗电 1 W @ -2h",
        ] {
            assert_eq!(
                calculate_circuit(query),
                Err(ConversionError::InvalidEnergyUse),
                "{query}"
            );
        }
        assert_eq!(
            calculate_circuit("耗电 1e308 MW @ 2h"),
            Err(ConversionError::OutOfRange)
        );
    }

    #[test]
    fn calculates_the_voltage_across_the_second_resistor_in_an_unloaded_divider() {
        for (query, expected) in [
            ("分压 12 V @ 1 kΩ,2 kΩ", "8 V"),
            ("VOLTAGE-DIVIDER 3 V @ (2+1)kΩ,(2**0)kΩ", "750 mV"),
            ("分压 12 V @ 1e308 Ω,1e308 Ω", "6 V"),
            ("分压 0 V @ 2 Ω,3 Ω", "0 V"),
        ] {
            assert_eq!(calculate_circuit(query).as_deref(), Ok(expected), "{query}");
        }
        let output = calculate_circuit("分压 1e300 V @ 1e300 Ω,1e-300 Ω").unwrap();
        let microvolts = output.strip_suffix(" μV").unwrap().parse::<f64>().unwrap();
        assert!((microvolts / 1e-294 - 1.0).abs() < 1e-12, "{output}");
        for query in [
            "分压 12 V @ 1 kΩ",
            "分压 12 V @ 1 kΩ,2 kΩ,3 kΩ",
            "分压 12 V @ 0 Ω,2 kΩ",
            "分压 12 A @ 1 kΩ,2 kΩ",
        ] {
            assert_eq!(
                calculate_circuit(query),
                Err(ConversionError::InvalidVoltageDivider)
            );
        }
    }

    #[test]
    fn calculates_voltage_across_a_parallel_load() {
        for (query, expected) in [
            ("负载分压 12 V @ 1 kΩ,2 kΩ,2 kΩ", "6 V"),
            ("LOADED-VOLTAGE-DIVIDER 9 V @ 1 kΩ,1 kΩ,(2*500)Ω", "3 V"),
            ("负载分压 12 V @ 1e308 Ω,1e308 Ω,1e308 Ω", "4 V"),
            ("负载分压 0 V @ 1 kΩ,2 kΩ,2 kΩ", "0 V"),
        ] {
            assert_eq!(calculate_circuit(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "负载分压 12 V @ 1 kΩ,2 kΩ",
            "负载分压 12 V @ 1 kΩ,2 kΩ,0 Ω",
            "负载分压 12 V @ 1 kΩ,2 kΩ,5 mA",
            "负载分压 12 V @ 1 kΩ,2 kΩ,3 kΩ,4 kΩ",
        ] {
            assert_eq!(
                calculate_circuit(query),
                Err(ConversionError::InvalidLoadedVoltageDivider),
                "{query}"
            );
        }
    }

    #[test]
    fn exports_rc_responses_in_frequency_order_with_a_blank_phase_at_dc() {
        let input = r#"RC高通表 1 kΩ,1 μF @ ["0 Hz","(1/(2*pi*0.001)) Hz","0 kHz"]"#;
        let result = calculate_circuit(input).unwrap();
        let rows: Vec<_> = result.split("\r\n").collect();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], "频率,幅值比,增益dB,相位°");
        assert_eq!(rows[1], "0 Hz,0,−∞,\"\"");
        assert!(rows[2].starts_with("(1/(2*pi*0.001)) Hz,0.7071"));
        assert_eq!(rows[3], "0 kHz,0,−∞,\"\"");
        assert_eq!(
            calculate_circuit("RC低通表 1 kΩ,1 μF @ []"),
            Err(ConversionError::InvalidRcResponseTable),
        );
    }

    #[test]
    fn calculates_rc_filter_magnitude_gain_and_phase_at_cutoff_and_dc() {
        for (filter, phase) in [("RC低通", -45.0), ("RC高通", 45.0)] {
            let query = format!("{filter} 1 kΩ,1 μF @ (1/(2*pi*0.001)) Hz");
            let output = calculate_circuit(&query).unwrap();
            let parts: Vec<_> = output.split(" · ").collect();
            let amplitude = parts[0].parse::<f64>().unwrap();
            let gain = parts[1]
                .strip_suffix(" dB")
                .unwrap()
                .parse::<f64>()
                .unwrap();
            let angle = parts[2].strip_suffix('°').unwrap().parse::<f64>().unwrap();
            assert!((amplitude - 2.0_f64.sqrt().recip()).abs() < 1e-12);
            assert!((gain + 3.01029995664).abs() < 1e-11);
            assert!((angle - phase).abs() < 1e-10);
        }
        assert_eq!(
            calculate_circuit("RC低通 1 kΩ,1 μF @ 0 Hz").as_deref(),
            Ok("1 · 0 dB · 0°"),
        );
        assert_eq!(
            calculate_circuit("rc-high-pass 1 kΩ,1 μF @ 0 Hz").as_deref(),
            Ok("0 · −∞ dB · 相位未定义"),
        );
        let extreme = calculate_circuit("rc-low-pass 1 kΩ,1 μF @ 1e308 GHz").unwrap();
        let (ratio, rest) = extreme.split_once(" · ").unwrap();
        let ratio = ratio.parse::<f64>().unwrap();
        assert!(ratio > 0.0 && ratio < 1e-300);
        assert!(rest.starts_with('-') && rest.ends_with(" · -90°"));
        assert_eq!(
            calculate_circuit("RC高通 1 kΩ,1 μF @ -1 Hz"),
            Err(ConversionError::InvalidRcResponse),
        );
    }

    #[test]
    fn calculates_the_missing_component_for_an_rc_cutoff_frequency() {
        for (query, unit, expected) in [
            ("RC电容 1 kΩ @ 1 kHz", " nF", 159.154943091895),
            ("rc-resistance 100 nF @ 1 kHz", " kΩ", 1.59154943091895),
        ] {
            let value = calculate_circuit(query).unwrap();
            let amount = value.strip_suffix(unit).unwrap().parse::<f64>().unwrap();
            assert!((amount - expected).abs() < 1e-10, "{query}: {value}");
        }
        for query in [
            "RC电容 1 μF @ 1 kHz",
            "RC电阻 0 F @ 1 Hz",
            "RC电阻 100 nF @ 0 Hz",
            "RC电容 1 kΩ @ 1 s",
        ] {
            assert_eq!(
                calculate_circuit(query),
                Err(ConversionError::InvalidRcDesign)
            );
        }
    }

    #[test]
    fn calculates_duration_to_reach_an_rc_charging_or_discharging_fraction() {
        for (query, unit, expected) in [
            ("RC充电 50% @ 1 kΩ,1 μF", " μs", 693.147180559945),
            ("rc-discharge 1/4 @ 1 kΩ,1 μF", " ms", 1.38629436111989),
            ("RC充电 1-1e-21 @ 1 kΩ,1 μF", " ms", 48.3542869528750),
        ] {
            let result = calculate_circuit(query).unwrap();
            let duration = result.strip_suffix(unit).unwrap().parse::<f64>().unwrap();
            assert!((duration - expected).abs() < 1e-10, "{query}: {result}");
        }
        assert_eq!(
            calculate_circuit("RC充电 0% @ 1 kΩ,1 μF").as_deref(),
            Ok("0 s")
        );
        assert_eq!(
            calculate_circuit("RC放电 100% @ 1 kΩ,1 μF").as_deref(),
            Ok("0 s")
        );
        assert_eq!(
            calculate_circuit("RC充电 100% @ 1 kΩ,1 μF"),
            Err(ConversionError::InvalidRcCharge),
        );
        assert_eq!(
            calculate_circuit("RC放电 0% @ 1 kΩ,1 μF"),
            Err(ConversionError::InvalidRcDischarge),
        );
    }

    #[test]
    fn calculates_first_order_rc_time_constants_and_cutoff_frequencies() {
        for query in [
            "RC 1 kΩ,1 μF",
            "rc-filter (1+1)kΩ,500 nF",
            "RC滤波 1000 Ω,1 µF",
        ] {
            let value = calculate_circuit(query).unwrap();
            let frequency = value
                .strip_prefix("1 ms · ")
                .unwrap()
                .strip_suffix(" Hz")
                .unwrap()
                .parse::<f64>()
                .unwrap();
            assert!((frequency - 159.154943091895).abs() < 1e-10, "{query}");
        }
        let nanoseconds = calculate_circuit("RC 1 Ω,1 nF").unwrap();
        assert!(nanoseconds.starts_with("1 ns · "), "{nanoseconds}");
        for query in [
            "RC 1 kΩ,0 F",
            "RC 1 V,1 μF",
            "RC 1 kΩ,1 s",
            "RC 1 kΩ,1 μF,2 μF",
        ] {
            assert_eq!(
                calculate_circuit(query),
                Err(ConversionError::InvalidRcFilter)
            );
        }
        assert_eq!(
            calculate_circuit("RC 1e308 Ω,10 F"),
            Err(ConversionError::OutOfRange),
        );
    }

    #[test]
    fn calculates_the_second_branch_current_in_a_parallel_divider() {
        for (query, expected) in [
            ("分流 6 mA @ 1 kΩ,2 kΩ", "2 mA"),
            ("CURRENT-DIVIDER (2*3)mA @ 2000 Ω,1 kΩ", "4 mA"),
            ("分流 0 A @ 2 Ω,3 Ω", "0 A"),
        ] {
            assert_eq!(calculate_circuit(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "分流 6 V @ 1 kΩ,2 kΩ",
            "分流 6 mA @ 1 kΩ",
            "分流 6 mA @ 1 kΩ,0 Ω",
        ] {
            assert_eq!(
                calculate_circuit(query),
                Err(ConversionError::InvalidCurrentDivider),
                "{query}"
            );
        }
    }

    #[test]
    fn calculates_series_and_parallel_resistance_with_mixed_scales() {
        for (query, expected) in [
            ("串联 220 Ω,330 Ω,1 kΩ", "1.55 kΩ"),
            ("series 220 Ω, (2**5+1)Ω", "253 Ω"),
            ("并联 220 Ω, 330Ω", "132 Ω"),
            ("parallel 2 kΩ,2kΩ", "1 kΩ"),
            ("并联 900 mΩ, 900mΩ,900mΩ", "300 mΩ"),
        ] {
            assert_eq!(calculate_circuit(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["并联 2 kΩ", "串联 2Ω,0Ω", "parallel 1 V,3Ω"] {
            assert_eq!(
                calculate_circuit(query),
                Err(ConversionError::InvalidResistanceList),
                "{query}"
            );
        }
    }

    #[test]
    fn converts_parenthesized_amounts_without_a_space_before_the_unit() {
        for (query, expected) in [
            ("(2**10)B to KiB", "1 KiB"),
            ("(2^10)B to KiB", "1 KiB"),
            ("((2 + 1/2) * 2)km to m", "5000 m"),
            ("（10+5）cm转mm", "150 mm"),
        ] {
            assert_eq!(convert(query).unwrap().as_deref(), Ok(expected), "{query}");
        }
        assert!(suggested_targets("(2**10)B")
            .unwrap()
            .iter()
            .any(|item| { item.expression == "(2**10)B to KiB" && item.result == "1 KiB" }));
    }

    #[test]
    fn converts_compatible_units_with_chinese_and_ascii_syntax() {
        for (query, expected) in [
            ("1 km to m", "1000 m"),
            ("1_000ms to s", "1 s"),
            ("10 in in cm", "25.4 cm"),
            ("３千克转克", "3000 g"),
            ("1e3ms TO s", "1 s"),
            ("100 ℃ → ℉", "212 °F"),
            ("32°F to °C", "0 °C"),
            ("-40摄氏度转华氏度", "-40 °F"),
            ("0 °C to K", "273.15 K"),
            ("2小时转分钟", "120 min"),
            ("(2+3) km to m", "5000 m"),
            ("sqrt(16) m to cm", "400 cm"),
            ("avg(2, 6) h to min", "240 min"),
            ("15% of 200 km to m", "30000 m"),
            ("100 增加 20% km to m", "120000 m"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.into())), "{query}");
        }
    }

    #[test]
    fn reports_invalid_conversions_and_leaves_other_queries_alone() {
        for (query, expected) in [
            ("10 cm to kg", ConversionError::DifferentDimensions),
            ("2 不存在 to km", ConversionError::UnknownUnit),
            ("3 to km", ConversionError::Incomplete),
            ("3 kg to", ConversionError::Incomplete),
            ("3e kg to g", ConversionError::InvalidAmount),
            ("1e308 km to mm", ConversionError::OutOfRange),
        ] {
            assert_eq!(convert(query), Some(Err(expected)), "{query}");
        }
        assert_eq!(convert("2**10"), None);
        assert_eq!(convert("10 in"), None);
        assert_eq!(convert("页面转设置"), None);
    }

    #[test]
    fn converts_angles_pressure_and_frequency_with_exact_unit_case() {
        for (query, expected) in [
            ("π rad to deg", "180 °"),
            ("０．５圈转度", "180 °"),
            ("180° to rad", "3.14159265358979 rad"),
            ("1 atm to kPa", "101.325 kPa"),
            ("760 Torr to atm", "1 atm"),
            ("1 psi to kPa", "6.89475729316836 kPa"),
            ("1 MPa to mPa", "1000000000 mPa"),
            ("2 MHz to kHz", "2000 kHz"),
            ("500 mHz to Hz", "0.5 Hz"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.into())), "{query}");
        }
        assert_eq!(
            convert("1 mpa to Pa"),
            Some(Err(ConversionError::UnknownUnit))
        );
        assert_eq!(
            convert("10 Pa to Hz"),
            Some(Err(ConversionError::DifferentDimensions))
        );
    }

    #[test]
    fn converts_rotation_speed_with_angle_and_time_units_together() {
        for (query, expected) in [
            ("120 rpm to °/s", "720 °/s"),
            ("(pi)rad/s to rpm", "30 rpm"),
            ("-0.5 圈/秒 to rpm", "-30 rpm"),
            ("180 deg/min to rpm", "0.5 rpm"),
            ("60 r/min to rev/s", "1 rev/s"),
            ("0°/s to rad/min", "0 rad/min"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.into())), "{query}");
        }
        assert_eq!(
            convert("1 rpm to rad"),
            Some(Err(ConversionError::DifferentDimensions))
        );
        let suggestions = suggested_targets("120 rpm to rad/").unwrap();
        assert_eq!(suggestions.len(), 2);
    }

    #[test]
    fn converts_area_volume_speed_and_data_with_case_sensitive_symbols() {
        for (query, expected) in [
            ("2 公顷转平方米", "20000 m²"),
            ("1 ft^2 to cm²", "929.0304 cm²"),
            ("1000mL 转 L", "1 L"),
            ("1 usgal to L", "3.785411784 L"),
            ("1 impgal to L", "4.54609 L"),
            ("36 km/h to m/s", "10 m/s"),
            ("36 公里/小时转换为米/秒", "10 m/s"),
            ("1 GB to MB", "1000 MB"),
            ("1GiB to MiB", "1024 MiB"),
            ("1 GiB to MB", "1073.741824 MB"),
            ("8 Mb to MB", "1 MB"),
            ("1E3kB\tTO\tMB", "1 MB"),
            ("1字节转比特", "8 b"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.into())), "{query}");
        }
        assert_eq!(
            convert("10 GB to km/h"),
            Some(Err(ConversionError::DifferentDimensions))
        );
    }

    #[test]
    fn converts_data_rates_with_bits_bytes_and_binary_prefixes() {
        for (query, expected) in [
            ("8Mbps to MB/s", "1 MB/s"),
            ("1 MiB/s to kB/s", "1048.576 kB/s"),
            ("500 kB/秒转Mb/s", "4 Mb/s"),
            ("1 GiB/s to Mib/s", "8192 Mib/s"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.into())), "{query}");
        }
        assert_eq!(
            convert("1 MB to MB/s"),
            Some(Err(ConversionError::DifferentDimensions))
        );
    }

    #[test]
    fn converts_energy_and_power_without_mixing_dimensions() {
        for (query, expected) in [
            ("1度电转MJ", "3.6 MJ"),
            ("250 mWh to J", "900 J"),
            ("1 MWh to kWh", "1000 kWh"),
            ("1 MJ to mJ", "1000000000 mJ"),
            ("500 mW to W", "0.5 W"),
            ("2 MW to kW", "2000 kW"),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.into())), "{query}");
        }
        assert_eq!(
            convert("1 Wh to W"),
            Some(Err(ConversionError::DifferentDimensions))
        );
    }

    #[test]
    fn converts_signed_integers_between_bases_without_losing_precision() {
        for (query, expected) in [
            ("0xff to dec", "255"),
            ("255 to HEX", "0xff"),
            ("-0XFF 转 二进制", "-0b11111111"),
            ("0b1010_1111 to oct", "0o257"),
            ("-0o0 to binary", "0b0"),
            ("9007199254740993 to hex", "0x20000000000001"),
            (
                "0xffffffffffffffffffffffffffffffff to dec",
                "340282366920938463463374607431768211455",
            ),
        ] {
            assert_eq!(convert(query), Some(Ok(expected.into())), "{query}");
        }
        for query in ["0b102 to dec", "3.5 to hex", "0x_42 to oct", "++2 to bin"] {
            assert_eq!(convert(query), Some(Err(ConversionError::InvalidInteger)));
        }
        assert_eq!(
            convert("340282366920938463463374607431768211456 to hex"),
            Some(Err(ConversionError::OutOfRange))
        );
        assert_eq!(convert("转换为十六进制"), None);
    }
}
