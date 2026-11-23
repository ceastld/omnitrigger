use super::calculator::{evaluate, format_result, normalize};

const REFERENCE_NOTE: u8 = 69;
const REFERENCE_FREQUENCY: f64 = 440.0;
const MAX_MIDI_NOTE: u8 = 127;
const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PitchError {
    InvalidNote,
    MidiRange,
    FrequencyRange,
    InvalidFrequency,
    TransposeArguments,
    IntervalArguments,
    CentsArguments,
    CentsShiftArguments,
    ShiftFrequencyRange,
}

impl PitchError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::InvalidNote => "填写音名、MIDI 编号或频率，例如 A4、MIDI 69 或 442 Hz",
            Self::MidiRange => "音高须在 MIDI 0 至 127 的范围内",
            Self::FrequencyRange => "填写正频率，最接近的音高须在 MIDI 0 至 127 内",
            Self::InvalidFrequency => "频率须为有限正数，例如 442 Hz",
            Self::TransposeArguments => "用 @ 分隔音高和半音数，例如 Bb3 @ 2",
            Self::IntervalArguments => "用 @ 分隔两个音高，例如 C4 @ G4",
            Self::CentsArguments => "用 @ 分隔参考音高和待比较的音高，例如 A4 @ 442 Hz",
            Self::CentsShiftArguments => "用 @ 分隔音高和移调音分，例如 A4 @ 100",
            Self::ShiftFrequencyRange => "移调后的频率超出可表示范围",
        }
    }
}

pub(super) fn convert_pitch(query: &str) -> Result<String, PitchError> {
    let input = normalize(query);
    let input = input.trim();
    let (operation, arguments) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    match operation.to_ascii_lowercase().as_str() {
        "midi" => return format_pitch(parse_midi(arguments)?),
        "转调" | "transpose" => return transpose_pitch(arguments),
        "音程" | "interval" => return pitch_interval(arguments),
        "音分差" | "cents" => return compare_cents(arguments),
        "音分移调" | "transpose-cents" => return transpose_cents(arguments),
        _ => {}
    }
    if let Some(frequency) = parse_frequency(input).map_err(|_| PitchError::FrequencyRange)? {
        return nearest_pitch(frequency);
    }
    format_pitch(parse_note(input)?)
}

fn parse_frequency(input: &str) -> Result<Option<f64>, PitchError> {
    let lower = input.trim().to_ascii_lowercase();
    let units = [
        ("khz", 1000.0),
        ("千赫兹", 1000.0),
        ("hz", 1.0),
        ("赫兹", 1.0),
    ];
    let Some((number, multiplier)) = units
        .into_iter()
        .find_map(|(unit, multiplier)| lower.strip_suffix(unit).map(|number| (number, multiplier)))
    else {
        return Ok(None);
    };
    number
        .trim()
        .parse::<f64>()
        .ok()
        .map(|number| number * multiplier)
        .filter(|frequency| frequency.is_finite() && *frequency > 0.0)
        .map(Some)
        .ok_or(PitchError::InvalidFrequency)
}

fn parse_midi(input: &str) -> Result<u8, PitchError> {
    input
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|note| *note <= MAX_MIDI_NOTE)
        .ok_or(PitchError::MidiRange)
}

fn parse_written_pitch(input: &str) -> Result<u8, PitchError> {
    let input = input.trim();
    if let Some((_, arguments)) = input
        .split_once(char::is_whitespace)
        .filter(|(operation, _)| operation.eq_ignore_ascii_case("midi"))
    {
        return parse_midi(arguments);
    }
    parse_note(input)
}

fn transpose_pitch(input: &str) -> Result<String, PitchError> {
    let (note, shift) = input
        .split_once('@')
        .ok_or(PitchError::TransposeArguments)?;
    let note = i32::from(parse_written_pitch(note)?);
    let shift = shift
        .trim()
        .parse::<i16>()
        .map_err(|_| PitchError::TransposeArguments)?;
    let target = u8::try_from(note + i32::from(shift))
        .ok()
        .filter(|note| *note <= MAX_MIDI_NOTE)
        .ok_or(PitchError::MidiRange)?;
    format_pitch(target)
}

fn pitch_interval(input: &str) -> Result<String, PitchError> {
    let (first, second) = input.split_once('@').ok_or(PitchError::IntervalArguments)?;
    let distance = i32::from(parse_written_pitch(second)?) - i32::from(parse_written_pitch(first)?);
    Ok(format!("{distance} 半音 · {} 音分", distance * 100))
}

fn compare_cents(input: &str) -> Result<String, PitchError> {
    let (reference, target) = input
        .split_once('@')
        .filter(|(reference, target)| !reference.trim().is_empty() && !target.trim().is_empty())
        .ok_or(PitchError::CentsArguments)?;
    let reference = pitch_frequency(reference)?;
    let target = pitch_frequency(target)?;
    let ratio = target / reference;
    let octaves = if ratio.is_normal() {
        ratio.log2()
    } else {
        target.log2() - reference.log2()
    };
    let cents = (120_000.0 * octaves).round() / 100.0;
    Ok(format!("{} 音分", signed_cents(cents)))
}

fn transpose_cents(input: &str) -> Result<String, PitchError> {
    let (pitch, cents) = input
        .split_once('@')
        .filter(|(pitch, cents)| !pitch.trim().is_empty() && !cents.trim().is_empty())
        .ok_or(PitchError::CentsShiftArguments)?;
    let frequency = pitch_frequency(pitch)?;
    let octaves = evaluate(cents).map_err(|_| PitchError::CentsShiftArguments)? / 1200.0;
    let scale = octaves.exp2();
    let shifted = if scale.is_normal() {
        frequency * scale
    } else {
        (frequency.log2() + octaves).exp2()
    };
    if !shifted.is_finite() || shifted <= 0.0 {
        return Err(PitchError::ShiftFrequencyRange);
    }
    let shifted = format_result(shifted).map_err(|_| PitchError::ShiftFrequencyRange)?;
    Ok(format!("{shifted} Hz"))
}

fn pitch_frequency(input: &str) -> Result<f64, PitchError> {
    parse_frequency(input)?.map_or_else(|| parse_written_pitch(input).map(note_frequency), Ok)
}

fn parse_note(input: &str) -> Result<u8, PitchError> {
    let Some(letter) = input.chars().next() else {
        return Err(PitchError::InvalidNote);
    };
    let semitone = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return Err(PitchError::InvalidNote),
    };
    let rest = &input[letter.len_utf8()..];
    let (alteration, octave) = if let Some(octave) = rest.strip_prefix(['#', '♯']) {
        (1, octave)
    } else if let Some(octave) = rest.strip_prefix(['b', '♭']) {
        (-1, octave)
    } else {
        (0, rest)
    };
    let octave = octave.parse::<i32>().map_err(|_| PitchError::InvalidNote)?;
    if !(-1..=9).contains(&octave) {
        return Err(PitchError::MidiRange);
    }
    let midi = (octave + 1) * 12 + semitone + alteration;
    u8::try_from(midi)
        .ok()
        .filter(|note| *note <= MAX_MIDI_NOTE)
        .ok_or(PitchError::MidiRange)
}

fn note_name(midi: u8) -> String {
    let octave = i32::from(midi / 12) - 1;
    let letter = NOTE_NAMES[usize::from(midi % 12)];
    format!("{letter}{octave}")
}

fn note_frequency(midi: u8) -> f64 {
    let semitones = f64::from(midi) - f64::from(REFERENCE_NOTE);
    REFERENCE_FREQUENCY * 2.0_f64.powf(semitones / 12.0)
}

fn format_pitch(midi: u8) -> Result<String, PitchError> {
    let frequency = format_result(note_frequency(midi)).map_err(|_| PitchError::FrequencyRange)?;
    Ok(format!(
        "{} · MIDI {midi} · {frequency} Hz",
        note_name(midi)
    ))
}

fn nearest_pitch(frequency: f64) -> Result<String, PitchError> {
    let pitch = f64::from(REFERENCE_NOTE) + 12.0 * (frequency / REFERENCE_FREQUENCY).log2();
    let nearest = (pitch + 0.5).floor();
    if !(0.0..=f64::from(MAX_MIDI_NOTE)).contains(&nearest) {
        return Err(PitchError::FrequencyRange);
    }
    let midi = nearest as u8;
    let cents = ((pitch - nearest) * 10_000.0).round() / 100.0;
    let cents = signed_cents(cents);
    Ok(format!("{} · MIDI {midi} · {cents} 音分", note_name(midi)))
}

fn signed_cents(cents: f64) -> String {
    if cents == 0.0 {
        "0".to_owned()
    } else {
        format!("{cents:+.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transposes_and_compares_written_pitches() {
        assert_eq!(convert_pitch("转调 Bb3 @ 2"), convert_pitch("C4"));
        assert_eq!(
            convert_pitch("TRANSPOSE MIDI 81 @ -12"),
            convert_pitch("A4")
        );
        assert_eq!(convert_pitch("转调 B#3 @ 0"), convert_pitch("C4"));
        for (query, expected) in [
            ("音程 C4 @ G4", "7 半音 · 700 音分"),
            ("音程 G4 @ C4", "-7 半音 · -700 音分"),
            ("interval MIDI 61 @ Db4", "0 半音 · 0 音分"),
        ] {
            assert_eq!(convert_pitch(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(convert_pitch("转调 G9 @ 1"), Err(PitchError::MidiRange));
        assert_eq!(
            convert_pitch("转调 A4 @ 1/2"),
            Err(PitchError::TransposeArguments)
        );
        assert_eq!(convert_pitch("音程 A4"), Err(PitchError::IntervalArguments));
    }

    #[test]
    fn compares_frequencies_and_written_notes_against_a_reference() {
        for (query, expected) in [
            ("音分差 A4 @ 442 Hz", "+7.85 音分"),
            ("CENTS 442 Hz @ MIDI 69", "-7.85 音分"),
            ("音分差 0.44 kHz @ 880赫兹", "+1200.00 音分"),
            ("音分差 20000 Hz @ 40000 Hz", "+1200.00 音分"),
            ("音分差 1e-300 Hz @ 1e300 Hz", "+2391788.23 音分"),
            ("音分差 MIDI 69 @ 440 Hz", "0 音分"),
        ] {
            assert_eq!(convert_pitch(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            convert_pitch("音分差 A4 @"),
            Err(PitchError::CentsArguments)
        );
        assert_eq!(
            convert_pitch("音分差 A4 @ 0 Hz"),
            Err(PitchError::InvalidFrequency)
        );
    }

    #[test]
    fn transposes_frequencies_by_fractional_or_integer_cents() {
        for (query, expected) in [
            ("音分移调 A4 @ 100", "466.16376151809 Hz"),
            ("TRANSPOSE-CENTS MIDI 69 @ 2*600", "880 Hz"),
            ("音分移调 442 Hz @ -1200", "221 Hz"),
            ("音分移调 0.442 kHz @ 0", "442 Hz"),
        ] {
            assert_eq!(convert_pitch(query).as_deref(), Ok(expected), "{query}");
        }
        let result = convert_pitch("音分移调 1e-300 Hz @ 1320000").unwrap();
        let result = result.strip_suffix(" Hz").unwrap().parse::<f64>().unwrap();
        let expected = (300.0 * -10.0_f64.log2() + 1100.0).exp2();
        assert!((result / expected - 1.0).abs() < 1e-12);
        assert_eq!(
            convert_pitch("音分移调 A4 @ 12000000"),
            Err(PitchError::ShiftFrequencyRange)
        );
        assert_eq!(
            convert_pitch("音分移调 A4 @"),
            Err(PitchError::CentsShiftArguments)
        );
    }

    #[test]
    fn converts_named_notes_midi_numbers_and_frequencies() {
        for (query, expected) in [
            ("A4", "A4 · MIDI 69 · 440 Hz"),
            ("ＭＩＤＩ ８１", "A5 · MIDI 81 · 880 Hz"),
            ("A5", "A5 · MIDI 81 · 880 Hz"),
            ("442 Hz", "A4 · MIDI 69 · +7.85 音分"),
            ("438 赫兹", "A4 · MIDI 69 · -7.89 音分"),
            ("1.76 kHz", "A6 · MIDI 93 · 0 音分"),
        ] {
            assert_eq!(convert_pitch(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(convert_pitch("Db4"), convert_pitch("C♯4"));
        assert_eq!(convert_pitch("B#3"), convert_pitch("C4"));
        assert_eq!(convert_pitch("C-1"), convert_pitch("MIDI 0"));
        assert_eq!(convert_pitch("G9"), convert_pitch("MIDI 127"));
        assert_eq!(convert_pitch("Gb9"), convert_pitch("F#9"));
        for (query, expected) in [
            ("MIDI 128", PitchError::MidiRange),
            ("C♭-1", PitchError::MidiRange),
            ("G#9", PitchError::MidiRange),
            ("A#", PitchError::InvalidNote),
            ("0 Hz", PitchError::FrequencyRange),
            ("NaN Hz", PitchError::FrequencyRange),
            ("1 Hz", PitchError::FrequencyRange),
            ("20 kHz", PitchError::FrequencyRange),
        ] {
            assert_eq!(convert_pitch(query), Err(expected), "{query}");
        }
    }
}
