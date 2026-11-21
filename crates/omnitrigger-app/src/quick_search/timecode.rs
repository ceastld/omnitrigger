use super::calculator::normalize;
use super::fraction::calculate_fraction;

const MINUTES_PER_DAY: u64 = 24 * 60;
const SECONDS_PER_MINUTE: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TimecodeError {
    Incomplete,
    InvalidRate,
    InvalidCount,
    InvalidCode,
    InvalidDropFrame,
    ConflictingMode,
    SkippedLabel,
    OutOfRange,
}

impl TimecodeError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入帧数或时间码，再指定帧率，例如 1800 帧 @ 29.97 fps DF",
            Self::InvalidRate => "请指定支持的时间码帧率，例如 25 fps 或 30000/1001 fps",
            Self::InvalidCount => "帧数须为非负整数",
            Self::InvalidCode => "请输入有效的时间码，例如 00:00:05:00",
            Self::InvalidDropFrame => "DF 支持 29.97 和 59.94 fps",
            Self::ConflictingMode => "时间码分隔符与 DF 或 NDF 不一致",
            Self::SkippedLabel => "丢帧计数会跳过这个帧编号",
            Self::OutOfRange => "帧数超出 24 小时时间码范围",
        }
    }
}

pub(super) fn convert_timecode(input: &str) -> Result<String, TimecodeError> {
    let input = normalize(input).to_ascii_lowercase();
    let (source, rate) = input
        .split_once(['@', '按'])
        .ok_or(TimecodeError::Incomplete)?;
    let (source, rate) = (source.trim(), rate.trim());
    if source.is_empty() || rate.is_empty() {
        return Err(TimecodeError::Incomplete);
    }

    let (rate, requested_drop) = match rate.rsplit_once(char::is_whitespace) {
        Some((rate, "df")) => (rate.trim_end(), Some(true)),
        Some((rate, "ndf")) => (rate.trim_end(), Some(false)),
        _ => (rate, None),
    };
    let rate = rate
        .strip_suffix("fps")
        .or_else(|| rate.strip_suffix("帧/秒"))
        .ok_or(TimecodeError::InvalidRate)?;
    let (fps, drop_available) = timecode_rate(rate)?;

    if let Some((clock, separator, frame)) = split_timecode(source) {
        let drop_frame = separator == ';';
        if requested_drop.is_some_and(|requested| requested != drop_frame) {
            return Err(TimecodeError::ConflictingMode);
        }
        let drop = drop_count(drop_available, drop_frame)?;
        return timecode_to_frames(clock, frame, fps, drop).map(|frames| format!("{frames} 帧"));
    }

    let count = ["frames", "frame", "帧"]
        .into_iter()
        .find_map(|suffix| source.strip_suffix(suffix))
        .unwrap_or(source)
        .trim();
    let frames = count.parse().map_err(|_| TimecodeError::InvalidCount)?;
    let drop = drop_count(drop_available, requested_drop.unwrap_or(false))?;
    frames_to_timecode(frames, fps, drop)
}

fn timecode_rate(rate: &str) -> Result<(u64, u64), TimecodeError> {
    let rate = calculate_fraction(rate).map_err(|_| TimecodeError::InvalidRate)?;
    let numerator = u64::try_from(rate.numer()).map_err(|_| TimecodeError::InvalidRate)?;
    let denominator = u64::try_from(rate.denom()).map_err(|_| TimecodeError::InvalidRate)?;
    // 小数帧率简写与对应的 NTSC 分数使用相同的标称计数。
    match (numerator, denominator) {
        (24 | 25 | 30 | 48 | 50 | 60, 1) => Ok((numerator, 0)),
        (24_000, 1001) | (2997, 125) | (1199, 50) => Ok((24, 0)),
        (30_000, 1001) | (2997, 100) => Ok((30, 2)),
        (48_000, 1001) | (5994, 125) | (959, 20) => Ok((48, 0)),
        (60_000, 1001) | (2997, 50) => Ok((60, 4)),
        _ => Err(TimecodeError::InvalidRate),
    }
}

fn drop_count(available: u64, requested: bool) -> Result<u64, TimecodeError> {
    match (available, requested) {
        (0, true) => Err(TimecodeError::InvalidDropFrame),
        (available, true) => Ok(available),
        (_, false) => Ok(0),
    }
}

fn split_timecode(source: &str) -> Option<(&str, char, &str)> {
    let separator = source.rfind([':', ';'])?;
    let (clock, suffix) = source.split_at(separator);
    let mut chars = suffix.chars();
    Some((clock, chars.next()?, chars.as_str()))
}

fn timecode_to_frames(clock: &str, frame: &str, fps: u64, drop: u64) -> Result<u64, TimecodeError> {
    let mut clock = clock.split(':');
    let hours = timecode_field(clock.next())?;
    let minutes = timecode_field(clock.next())?;
    let seconds = timecode_field(clock.next())?;
    let frame = timecode_field(Some(frame))?;
    if clock.next().is_some() || hours >= 24 || minutes >= 60 || seconds >= 60 || frame >= fps {
        return Err(TimecodeError::InvalidCode);
    }
    if seconds == 0 && !minutes.is_multiple_of(10) && frame < drop {
        return Err(TimecodeError::SkippedLabel);
    }

    let minutes = hours * 60 + minutes;
    let nominal = (minutes * SECONDS_PER_MINUTE + seconds) * fps + frame;
    Ok(nominal - drop * (minutes - minutes / 10))
}

fn timecode_field(input: Option<&str>) -> Result<u64, TimecodeError> {
    let input = input.ok_or(TimecodeError::InvalidCode)?;
    if input.len() != 2 || !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TimecodeError::InvalidCode);
    }
    input.parse().map_err(|_| TimecodeError::InvalidCode)
}

fn frames_to_timecode(frames: u64, fps: u64, drop: u64) -> Result<String, TimecodeError> {
    let frames_per_minute = fps * SECONDS_PER_MINUTE - drop;
    let frames_per_ten_minutes = frames_per_minute * 10 + drop;
    if frames >= frames_per_ten_minutes * (MINUTES_PER_DAY / 10) {
        return Err(TimecodeError::OutOfRange);
    }

    // 每十分钟的首分钟保留全部编号，其余九分钟各跳过 drop 个编号。
    let blocks = frames / frames_per_ten_minutes;
    let remainder = frames % frames_per_ten_minutes;
    let missed_minutes = blocks * 9 + remainder.saturating_sub(drop) / frames_per_minute;
    let nominal = frames + drop * missed_minutes;
    let seconds = nominal / fps;
    let (hours, minutes, seconds, frame) = (
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        nominal % fps,
    );
    let separator = if drop == 0 { ':' } else { ';' };
    Ok(format!(
        "{hours:02}:{minutes:02}:{seconds:02}{separator}{frame:02}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_frame_counts_and_timecodes_in_both_modes() {
        for (input, expected) in [
            ("125 帧 @ 25 fps", "00:00:05:00"),
            ("00:00:05:00 @ 25 fps", "125 帧"),
            ("1800 帧 @ 29.97 fps", "00:01:00:00"),
            ("1799 帧 @ 29.97 fps DF", "00:00:59;29"),
            ("1800 帧 @ 29.97 fps DF", "00:01:00;02"),
            ("00:01:00;02 @ 30000/1001 fps", "1800 帧"),
            ("00:10:00;00 @ 29.97 fps DF", "17982 帧"),
            ("01:00:00;00 @ 29.97 fps", "107892 帧"),
            ("3600 frames @ 60000/1001 fps DF", "00:01:00;04"),
            ("00:01:00;04 @ 59.94 fps", "3600 帧"),
            ("240 @ 24000/1001 fps NDF", "00:00:10:00"),
            ("00:00:00:47 @ 47.952 fps", "47 帧"),
            ("2589407 帧 @ 29.97 fps DF", "23:59:59;29"),
        ] {
            assert_eq!(convert_timecode(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn rejects_omitted_frame_labels_and_out_of_range_addresses() {
        for (input, error) in [
            ("00:01:00;00 @ 29.97 fps", TimecodeError::SkippedLabel),
            ("00:01:00;03 @ 59.94 fps", TimecodeError::SkippedLabel),
            ("00:01:02:05 @ 29.97 fps DF", TimecodeError::ConflictingMode),
            (
                "00:01:02;05 @ 29.97 fps NDF",
                TimecodeError::ConflictingMode,
            ),
            ("00:01:00;02 @ 30 fps", TimecodeError::InvalidDropFrame),
            ("00:01:00:30 @ 30 fps", TimecodeError::InvalidCode),
            ("24:00:00:00 @ 25 fps", TimecodeError::InvalidCode),
            ("2589408 帧 @ 29.97 fps DF", TimecodeError::OutOfRange),
            ("0 帧 @ 20 fps", TimecodeError::InvalidRate),
            ("-1 帧 @ 25 fps", TimecodeError::InvalidCount),
        ] {
            assert_eq!(convert_timecode(input), Err(error), "{input}");
        }
    }

    #[test]
    fn round_trips_the_first_ten_minutes_and_hour_boundary() {
        for (fps, drop) in [(30, 2), (60, 4)] {
            let ten_minutes = fps * 600 - drop * 9;
            let hour = ten_minutes * 6;
            for frames in (0..=ten_minutes).chain(hour - 3..=hour + 3) {
                let code = frames_to_timecode(frames, fps, drop).unwrap();
                let (clock, separator, frame) = split_timecode(&code).unwrap();
                assert_eq!(separator, ';');
                assert_eq!(timecode_to_frames(clock, frame, fps, drop), Ok(frames));
            }
        }
    }
}
