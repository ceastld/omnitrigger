use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

const URL_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
const FORM_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'*')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_');

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UrlEncodingError {
    Incomplete,
    InvalidOperation,
    InvalidEscape,
    InvalidUtf8,
    NullCharacter,
}

impl UrlEncodingError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入要编码或解码的文本，例如 URL: 编码 北京",
            Self::InvalidOperation => "输入 编码、解码、表单编码 或 表单解码，再输入文本",
            Self::InvalidEscape => "每个 % 后需要两个十六进制数字",
            Self::InvalidUtf8 => "解码结果无法表示为 UTF-8 文本",
            Self::NullCharacter => "解码结果含空字符，无法完整复制",
        }
    }
}

pub(super) fn convert_url_component(input: &str) -> Result<String, UrlEncodingError> {
    let (operation, value) = input
        .split_once(char::is_whitespace)
        .ok_or(UrlEncodingError::Incomplete)?;
    let value = value.trim();
    if value.is_empty() {
        return Err(UrlEncodingError::Incomplete);
    }
    match operation.to_ascii_lowercase().as_str() {
        "编码" | "encode" => Ok(utf8_percent_encode(value, URL_COMPONENT).to_string()),
        "解码" | "decode" => decode_value(value),
        "表单编码" | "form-encode" => Ok(utf8_percent_encode(value, FORM_VALUE)
            .to_string()
            .replace("%20", "+")),
        "表单解码" | "form-decode" => decode_value(&value.replace('+', " ")),
        _ => Err(UrlEncodingError::InvalidOperation),
    }
}

fn decode_value(value: &str) -> Result<String, UrlEncodingError> {
    let mut rest = value;
    while let Some((_, suffix)) = rest.split_once('%') {
        let digits = suffix
            .get(..2)
            .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or(UrlEncodingError::InvalidEscape)?;
        rest = &suffix[digits.len()..];
    }
    let decoded = percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| UrlEncodingError::InvalidUtf8)?;
    if decoded.contains('\0') {
        return Err(UrlEncodingError::NullCharacter);
    }
    Ok(decoded.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_url_components_with_unicode_spaces_and_literal_plus() {
        for (input, expected) in [
            ("编码 北京 A+B", "%E5%8C%97%E4%BA%AC%20A%2BB"),
            ("ENCODE a-z._~123 /", "a-z._~123%20%2F"),
            ("解码 %e5%8c%97%e4%ba%ac%20A%2BB", "北京 A+B"),
            ("decode %20a+b%20", " a+b "),
            ("decode %252F", "%2F"),
        ] {
            assert_eq!(convert_url_component(input).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn converts_form_values_with_spaces_and_literal_plus() {
        for (input, expected) in [
            ("表单编码 北京 A+B", "%E5%8C%97%E4%BA%AC+A%2BB"),
            ("FORM-DECODE %E5%8C%97%E4%BA%AC+A%2BB", "北京 A+B"),
            ("form-encode !*'()~-._", "%21*%27%28%29%7E-._"),
            ("form-decode +%2B+", " + "),
            ("form-decode %252B", "%2B"),
            ("decode +%2B+", "+++"),
        ] {
            assert_eq!(
                convert_url_component(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            convert_url_component("表单解码 +%2G"),
            Err(UrlEncodingError::InvalidEscape)
        );
        assert_eq!(
            convert_url_component("表单解码 +%FF"),
            Err(UrlEncodingError::InvalidUtf8)
        );
    }

    #[test]
    fn reports_invalid_percent_escapes_and_unrepresentable_text() {
        for (input, expected) in [
            ("编码", UrlEncodingError::Incomplete),
            ("invalid A", UrlEncodingError::InvalidOperation),
            ("解码 %2", UrlEncodingError::InvalidEscape),
            ("解码 %2G", UrlEncodingError::InvalidEscape),
            ("解码 %中", UrlEncodingError::InvalidEscape),
            ("解码 %FF", UrlEncodingError::InvalidUtf8),
            ("解码 A%00B", UrlEncodingError::NullCharacter),
        ] {
            assert_eq!(convert_url_component(input), Err(expected));
        }
    }
}
