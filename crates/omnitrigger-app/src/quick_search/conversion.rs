use std::f64::consts::{PI, TAU};

use super::calculator::{evaluate, format_result, looks_like_calculation, normalize, QUERY_LIMIT};
use super::radix::Base;
use super::InputSuggestion;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConversionError {
    Incomplete,
    InvalidAmount,
    InvalidInteger,
    UnknownUnit,
    DifferentDimensions,
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
            Self::DifferentDimensions => "两个单位不属于同一类型",
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
    Pressure,
    Frequency,
    Energy,
    Power,
    Density,
    VolumeFlow,
    Force,
    Acceleration,
}

impl Dimension {
    fn suggested_units(&self, source: &str) -> &'static [&'static str] {
        match self {
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
            Self::Area => &["m²", "ft²", "ha", "km²", "cm²", "acre"],
            Self::Volume => &["L", "mL", "usgal", "m³"],
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
            Self::Pressure => &[
                "Pa", "kPa", "bar", "psi", "atm", "hPa", "MPa", "mPa", "Torr",
            ],
            Self::Frequency => &["Hz", "kHz", "MHz", "GHz", "mHz"],
            Self::Energy => &["J", "kJ", "MJ", "Wh", "kWh", "mJ", "mWh", "MWh"],
            Self::Power => &["W", "kW", "mW", "MW", "GW"],
            Self::Density => &["kg/m³", "g/cm³", "g/L", "kg/L"],
            Self::VolumeFlow => &["L/min", "L/s", "m³/h", "m³/s", "mL/s"],
            Self::Force => &["N", "kN", "kgf", "lbf", "mN", "MN", "dyn"],
            Self::Acceleration => &["m/s²", "g₀", "ft/s²", "cm/s²", "in/s²"],
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
            _ => {}
        }
        let unit = match name.to_ascii_lowercase().as_str() {
            "nm" | "纳米" => Self::linear("nm", Length, 1e-9),
            "μm" | "µm" | "um" | "微米" => Self::linear("μm", Length, 1e-6),
            "mm" | "毫米" => Self::linear("mm", Length, 0.001),
            "cm" | "厘米" => Self::linear("cm", Length, 0.01),
            "m" | "米" => Self::linear("m", Length, 1.),
            "km" | "千米" | "公里" => Self::linear("km", Length, 1000.),
            "in" | "inch" | "inches" | "英寸" => Self::linear("in", Length, 0.0254),
            "ft" | "foot" | "feet" | "英尺" => Self::linear("ft", Length, 0.3048),
            "yd" | "码" => Self::linear("yd", Length, 0.9144),
            "mi" | "mile" | "miles" | "英里" => Self::linear("mi", Length, 1609.344),
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
            "cm²" | "cm^2" | "cm2" | "平方厘米" => Self::linear("cm²", Area, 0.0001),
            "m²" | "m^2" | "m2" | "平方米" => Self::linear("m²", Area, 1.),
            "km²" | "km^2" | "km2" | "平方千米" => Self::linear("km²", Area, 1_000_000.),
            "ft²" | "ft^2" | "ft2" | "平方英尺" => Self::linear("ft²", Area, 0.092_903_04),
            "ha" | "公顷" => Self::linear("ha", Area, 10_000.),
            "acre" | "acres" | "英亩" => Self::linear("acre", Area, 4_046.856_422_4),
            "ml" | "cm³" | "cm^3" | "cm3" | "毫升" => Self::linear("mL", Volume, 0.001),
            "l" | "liter" | "litre" | "升" => Self::linear("L", Volume, 1.),
            "m³" | "m^3" | "m3" | "立方米" => Self::linear("m³", Volume, 1000.),
            "usgal" | "美制加仑" => Self::linear("usgal", Volume, 3.785_411_784),
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
            "lbf" | "磅力" => Self::linear("lbf", Force, 4.448_221_615_260_5),
            "dyn" | "达因" => Self::linear("dyn", Force, 1e-5),
            "m/s²" | "m/s^2" | "m/s2" | "米/秒²" | "米/秒平方" => {
                Self::linear("m/s²", Acceleration, 1.)
            }
            "cm/s²" | "cm/s^2" | "cm/s2" | "gal" | "厘米/秒平方" => {
                Self::linear("cm/s²", Acceleration, 0.01)
            }
            "ft/s²" | "ft/s^2" | "ft/s2" | "英尺/秒平方" => {
                Self::linear("ft/s²", Acceleration, 0.3048)
            }
            "in/s²" | "in/s^2" | "in/s2" | "英寸/秒平方" => {
                Self::linear("in/s²", Acceleration, 0.0254)
            }
            "g0" | "g₀" | "gₙ" | "标准重力加速度" => {
                Self::linear("g₀", Acceleration, STANDARD_GRAVITY)
            }
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
    source.rsplit_once(' ').unwrap_or_else(|| {
        let unit_start = source
            .find(|ch: char| !matches!(ch, '0'..='9' | '+' | '-' | '.' | 'e' | 'E' | '_'))
            .unwrap_or(source.len());
        source.split_at(unit_start)
    })
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

fn parse_quantity(source: &str) -> Result<(f64, Unit), ConversionError> {
    let evaluate_amount =
        |amount: &str| evaluate(amount).map_err(|_| ConversionError::InvalidAmount);
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
    let unit = Unit::parse(name)?;
    Ok((evaluate_amount(amount)?, unit))
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

pub(super) fn suggested_targets(query: &str) -> Option<Vec<InputSuggestion>> {
    let input = normalize(query).replace(char::is_whitespace, " ");
    let input = input.trim();
    let (source, stem, separator, prefix) = match split_conversion(input) {
        Some((source, target)) => {
            let prefix = target.trim();
            if Unit::parse(prefix).is_ok() {
                return None;
            }
            let stem = input[..input.len() - target.len()].trim_end();
            (source, stem, "", prefix)
        }
        None => (input, input, " to", ""),
    };
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
            let expression = format!("{stem}{separator} {name}");
            if expression.chars().count() > QUERY_LIMIT {
                return None;
            }
            Some(InputSuggestion {
                expression,
                result: format_conversion(amount, &source, &target).ok()?,
            })
        })
        .collect();
    (!items.is_empty()).then_some(items)
}

fn convert_quantity(source: &str, target: &str) -> Result<String, ConversionError> {
    let (amount, source) = parse_quantity(source)?;
    if matches!(
        target.trim().to_ascii_lowercase().as_str(),
        "ft in" | "feet inches" | "英尺英寸" | "英尺 英寸"
    ) {
        return format_feet_and_inches(amount, &source);
    }
    let target = Unit::parse(target)?;
    format_conversion(amount, &source, &target)
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

fn format_conversion(amount: f64, source: &Unit, target: &Unit) -> Result<String, ConversionError> {
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
    let value = format_result(converted).map_err(|_| ConversionError::OutOfRange)?;
    let suffix = if target.dimension == Dimension::DataRate {
        "/s"
    } else {
        ""
    };
    Ok(format!("{value} {}{suffix}", target.symbol))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for query in ["3 to", "10 km to mi", "0xff to", "100未知单位转"] {
            assert!(suggested_targets(query).is_none(), "{query}");
        }
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
    fn converts_area_volume_speed_and_data_with_case_sensitive_symbols() {
        for (query, expected) in [
            ("2 公顷转平方米", "20000 m²"),
            ("1 ft^2 to cm²", "929.0304 cm²"),
            ("1000mL 转 L", "1 L"),
            ("1 usgal to L", "3.785411784 L"),
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
