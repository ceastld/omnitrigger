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
}

impl Dimension {
    fn suggested_units(&self) -> &'static [&'static str] {
        match self {
            Self::Length => &["m", "mi", "km", "cm", "ft"],
            Self::Mass => &["kg", "g", "lb", "oz", "mg"],
            Self::Time => &["s", "min", "h", "ms", "d"],
            Self::Temperature => &["°C", "°F", "K"],
            Self::Area => &["m²", "ft²", "ha", "km²", "cm²"],
            Self::Volume => &["L", "mL", "usgal", "m³"],
            Self::Speed => &["km/h", "m/s", "mph", "kn"],
            Self::Data => &["MB", "MiB", "GB", "GiB", "B"],
            Self::DataRate => &["MB/s", "Mbps", "MiB/s", "kB/s", "Gbps"],
            Self::Angle => &["°", "rad", "turn"],
            Self::Pressure => &["Pa", "kPa", "bar", "psi", "atm"],
            Self::Frequency => &["Hz", "kHz", "MHz", "GHz", "mHz"],
            Self::Energy => &["J", "kJ", "MJ", "Wh", "kWh"],
            Self::Power => &["W", "kW", "mW", "MW", "GW"],
            Self::Density => &["kg/m³", "g/cm³", "g/L", "kg/L"],
            Self::VolumeFlow => &["L/min", "L/s", "m³/h", "m³/s", "mL/s"],
        }
    }
}

struct Unit {
    symbol: &'static str,
    dimension: Dimension,
    scale: f64,
    origin: f64,
}

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
            _ => {}
        }
        let unit = match name.to_ascii_lowercase().as_str() {
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
            "ms" | "毫秒" => Self::linear("ms", Time, 0.001),
            "s" | "sec" | "秒" => Self::linear("s", Time, 1.),
            "min" | "分钟" => Self::linear("min", Time, 60.),
            "h" | "hr" | "小时" => Self::linear("h", Time, 3600.),
            "d" | "day" | "days" | "天" => Self::linear("d", Time, 86400.),
            "c" | "°c" | "℃" | "摄氏度" => Self::temperature("°C", 1., 0.),
            "f" | "°f" | "℉" | "华氏度" => Self::temperature("°F", 5. / 9., 32.),
            "k" | "开尔文" => Self::temperature("K", 1., 273.15),
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

fn split_conversion(input: &str) -> Option<(&str, &str)> {
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

pub(super) fn convert(query: &str) -> Option<Result<String, ConversionError>> {
    let input = normalize(query).replace(char::is_whitespace, " ");
    let (source, target) = split_conversion(input.trim())?;
    let base = Base::parse(target);
    let (amount, unit) = if base.is_some() {
        (source, "")
    } else {
        split_quantity(source)
    };
    if !looks_like_calculation(amount) {
        return None;
    }
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Some(Err(ConversionError::TooLong));
    }
    Some(match base {
        Some(base) => base.convert(amount),
        None => convert_quantity(amount, unit, target),
    })
}

pub(super) fn suggested_targets(query: &str) -> Option<Vec<InputSuggestion>> {
    let input = normalize(query).replace(char::is_whitespace, " ");
    let input = input.trim();
    let (source, target) = split_conversion(input)?;
    if !target.trim().is_empty() {
        return None;
    }
    let (amount, name) = split_quantity(source);
    let amount = evaluate(amount).ok()?;
    let source = Unit::parse(name).ok()?;
    let items: Vec<_> = source
        .dimension
        .suggested_units()
        .iter()
        .filter_map(|name| {
            let target = Unit::parse(name).ok()?;
            if target.symbol == source.symbol {
                return None;
            }
            let expression = format!("{input} {name}");
            if expression.chars().count() > QUERY_LIMIT {
                return None;
            }
            Some(InputSuggestion {
                expression,
                result: format_conversion(amount, &source, &target).ok()?,
            })
        })
        .take(4)
        .collect();
    (!items.is_empty()).then_some(items)
}

fn convert_quantity(amount: &str, name: &str, target: &str) -> Result<String, ConversionError> {
    let amount = evaluate(amount).map_err(|_| ConversionError::InvalidAmount)?;
    let source = Unit::parse(name)?;
    let target = Unit::parse(target)?;
    format_conversion(amount, &source, &target)
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
