use super::calculator::normalize;

const GROUPS: [(u32, u16, u8, u8); 3] = [
    (6, 0o4000, b's', b'S'),
    (3, 0o2000, b's', b'S'),
    (0, 0o1000, b't', b'T'),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileModeError {
    Incomplete,
    InvalidMode,
    InvalidMask,
}

impl FileModeError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入八进制权限或 rwx 表示，例如 0755",
            Self::InvalidMode => "可用 0000 至 7777，或用 rwx、s/S、t/T 表示权限",
            Self::InvalidMask => "用 0000 至 0777 指定掩码，例如 0666 umask 0022",
        }
    }
}

pub(super) fn calculate_file_mode(query: &str) -> Result<String, FileModeError> {
    let input = normalize(query);
    let mut parts = input.split_whitespace();
    let source = parts.next().ok_or(FileModeError::Incomplete)?;
    if let Some(operation) = parts.next() {
        if operation != "掩码" && !operation.eq_ignore_ascii_case("umask") {
            return Err(FileModeError::InvalidMode);
        }
        let mask = parts.next().ok_or(FileModeError::Incomplete)?;
        if parts.next().is_some() {
            return Err(FileModeError::InvalidMask);
        }
        let mask = octal_mode(mask)
            .filter(|mask| *mask <= 0o777)
            .ok_or(FileModeError::InvalidMask)?;
        let source = octal_mode(source).map_or_else(|| symbolic_mode(source), Ok)?;
        let masked = source & !mask;
        return Ok(format!("{masked:04o} · {}", format_symbolic(masked)));
    }
    match octal_mode(source) {
        Some(mode) => Ok(format_symbolic(mode)),
        None => symbolic_mode(source).map(|mode| format!("{mode:04o}")),
    }
}

fn octal_mode(input: &str) -> Option<u16> {
    let input = input
        .strip_prefix("0o")
        .or_else(|| input.strip_prefix("0O"))
        .unwrap_or(input);
    if !(1..=4).contains(&input.len()) || !input.bytes().all(|digit| matches!(digit, b'0'..=b'7')) {
        return None;
    }
    u16::from_str_radix(input, 8).ok()
}

fn symbolic_mode(input: &str) -> Result<u16, FileModeError> {
    let bytes = input.as_bytes();
    let bytes = match bytes {
        [kind, parts @ ..] if bytes.len() == 10 && b"-dbclps".contains(kind) => parts,
        bytes => bytes,
    };
    let bytes: &[u8; 9] = bytes.try_into().map_err(|_| FileModeError::InvalidMode)?;
    let mut mode = 0;
    for (index, (shift, special, lower, upper)) in GROUPS.into_iter().enumerate() {
        let [read, write, execute] = [bytes[index * 3], bytes[index * 3 + 1], bytes[index * 3 + 2]];
        for (value, expected, permission) in [(read, b'r', 4), (write, b'w', 2)] {
            if value == expected {
                mode |= permission << shift;
            } else if value != b'-' {
                return Err(FileModeError::InvalidMode);
            }
        }
        mode |= match execute {
            b'-' => 0,
            b'x' => 1 << shift,
            flag if flag == lower => special | (1 << shift),
            flag if flag == upper => special,
            _ => return Err(FileModeError::InvalidMode),
        };
    }
    Ok(mode)
}

fn format_symbolic(mode: u16) -> String {
    let mut text = String::with_capacity(9);
    for (shift, special, lower, upper) in GROUPS {
        let group = (mode >> shift) & 7;
        text.push(if group & 4 != 0 { 'r' } else { '-' });
        text.push(if group & 2 != 0 { 'w' } else { '-' });
        text.push(match (mode & special != 0, group & 1 != 0) {
            (true, true) => char::from(lower),
            (true, false) => char::from(upper),
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_unix_permission_bits_and_calculates_creation_masks() {
        for mode in 0..=0o7777 {
            assert_eq!(symbolic_mode(&format_symbolic(mode)), Ok(mode));
        }
        for (query, expected) in [
            ("７５５", "rwxr-xr-x"),
            ("0o4755", "rwsr-xr-x"),
            ("4600", "rwS------"),
            ("drwxrwxrwt", "1777"),
            ("-rw-r-----", "0640"),
            ("0666 umask 0022", "0644 · rw-r--r--"),
            ("4755 掩码 0077", "4700 · rws------"),
            ("rwxrwxrwx UMASK 027", "0750 · rwxr-x---"),
        ] {
            assert_eq!(
                calculate_file_mode(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("0666 umask 1000", FileModeError::InvalidMask),
            ("0666 umask 022 more", FileModeError::InvalidMask),
            ("800", FileModeError::InvalidMode),
            ("rwt------", FileModeError::InvalidMode),
            ("0666 umask", FileModeError::Incomplete),
        ] {
            assert_eq!(calculate_file_mode(query), Err(error), "{query}");
        }
    }
}
