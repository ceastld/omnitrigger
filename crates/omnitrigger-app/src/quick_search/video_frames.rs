use std::time::Duration;

use num_bigint::{BigInt, Sign};

use super::calculator::{normalize, QUERY_LIMIT};
use super::duration::{elapsed_duration, format_clock_duration, DurationError, NANOS_PER_SECOND};
use super::fraction::{calculate_fraction, ensure_exact_size, FractionError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VideoFrameError {
    Incomplete,
    InvalidRate,
    InvalidCount,
    Exact(FractionError),
    Duration(DurationError),
}

impl VideoFrameError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入时长或帧数，再指定帧率，例如 125 帧 @ 25 fps",
            Self::InvalidRate => "帧率须大于零，例如 25 fps 或 30000/1001 fps",
            Self::InvalidCount => "帧数须为非负整数",
            Self::Exact(error) => error.message(),
            Self::Duration(error) => error.message(),
        }
    }
}

pub(super) fn convert_video_frames(query: &str) -> Result<String, VideoFrameError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(VideoFrameError::Exact(FractionError::TooLong));
    }
    let input = normalize(query).to_ascii_lowercase();
    let (source, rate) = input
        .split_once(['@', '按'])
        .ok_or(VideoFrameError::Incomplete)?;
    let source = source.trim();
    let rate = rate.trim();
    if source.is_empty() || rate.is_empty() {
        return Err(VideoFrameError::Incomplete);
    }
    let rate = rate
        .strip_suffix("fps")
        .or_else(|| rate.strip_suffix("帧/秒"))
        .ok_or(VideoFrameError::InvalidRate)?;
    let rate = calculate_fraction(rate).map_err(VideoFrameError::Exact)?;
    if rate.numer().sign() != Sign::Plus {
        return Err(VideoFrameError::InvalidRate);
    }
    let nanos_per_second = BigInt::from(NANOS_PER_SECOND);
    if let Some(frames) = ["frames", "frame", "帧"]
        .into_iter()
        .find_map(|suffix| source.strip_suffix(suffix))
    {
        let frames = calculate_fraction(frames).map_err(VideoFrameError::Exact)?;
        if !frames.is_integer() || frames.numer().sign() == Sign::Minus {
            return Err(VideoFrameError::InvalidCount);
        }
        let nanos = (frames * nanos_per_second / rate).round().to_integer();
        let nanos: u128 = nanos
            .try_into()
            .map_err(|_| VideoFrameError::Duration(DurationError::OutOfRange))?;
        if nanos > Duration::MAX.as_nanos() {
            return Err(VideoFrameError::Duration(DurationError::OutOfRange));
        }
        return Ok(format_clock_duration(nanos, ""));
    }
    let duration = elapsed_duration(source).map_err(VideoFrameError::Duration)?;
    let frames = (rate * BigInt::from(duration.as_nanos()) / nanos_per_second).round();
    ensure_exact_size(&frames).map_err(VideoFrameError::Exact)?;
    Ok(format!("{frames} 帧"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_frame_counts_and_durations_using_the_specified_rate() {
        for (query, expected) in [
            ("125 帧 @ 25 fps", "00:00:05"),
            ("00:00:05.25 @ 60 fps", "315 帧"),
            ("1 frame @ 30000/1001 fps", "00:00:00.033366667"),
            ("10秒 按 30000/1001 帧/秒", "300 帧"),
            ("５秒 ＠ １／２ ＦＰＳ", "3 帧"),
            ("0 frames @ 24 fps", "00:00:00"),
            ("1帧 @ 1/7200 fps", "02:00:00"),
        ] {
            assert_eq!(
                convert_video_frames(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("125 帧", VideoFrameError::Incomplete),
            ("125 帧 @ 0 fps", VideoFrameError::InvalidRate),
            ("1.5 帧 @ 60 fps", VideoFrameError::InvalidCount),
            ("-1 帧 @ 60 fps", VideoFrameError::InvalidCount),
            (
                "1 帧 @ 1/2**1024 fps",
                VideoFrameError::Duration(DurationError::OutOfRange),
            ),
        ] {
            assert_eq!(convert_video_frames(query), Err(error), "{query}");
        }
    }
}
