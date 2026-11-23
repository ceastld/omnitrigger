use super::calculator::{format_result, normalize};

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
    TransposeArguments,
    IntervalArguments,
}

impl PitchError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::InvalidNote => "填写音名、MIDI 编号或频率，例如 A4、MIDI 69 或 442 Hz",
            Self::MidiRange => "音高须在 MIDI 0 至 127 的范围内",
            Self::FrequencyRange => "填写正频率，最接近的音高须在 MIDI 0 至 127 内",
            Self::TransposeArguments => "用 @ 分隔音高和半音数，例如 Bb3 @ 2",
            Self::IntervalArguments => "用 @ 分隔两个音高，例如 C4 @ G4",
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
        _ => {}
    }
    let folded = input.to_ascii_lowercase();
    for (unit, multiplier) in [
        ("khz", 1000.0),
        ("千赫兹", 1000.0),
        ("hz", 1.0),
        ("赫兹", 1.0),
    ] {
        if let Some(value) = folded.strip_suffix(unit) {
            let frequency = value
                .trim()
                .parse::<f64>()
                .ok()
                .map(|value| value * multiplier)
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or(PitchError::FrequencyRange)?;
            return nearest_pitch(frequency);
        }
    }
    format_pitch(parse_note(input)?)
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

fn format_pitch(midi: u8) -> Result<String, PitchError> {
    let semitones = f64::from(midi) - f64::from(REFERENCE_NOTE);
    let frequency = REFERENCE_FREQUENCY * 2.0_f64.powf(semitones / 12.0);
    let frequency = format_result(frequency).map_err(|_| PitchError::FrequencyRange)?;
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
    let cents = if cents == 0.0 {
        "0".to_owned()
    } else {
        format!("{cents:+.2}")
    };
    Ok(format!("{} · MIDI {midi} · {cents} 音分", note_name(midi)))
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
