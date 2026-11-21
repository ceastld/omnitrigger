use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use url::Url;

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
    MissingParameters,
    InvalidUrl,
}

impl UrlEncodingError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入要编码或解码的文本，例如 URL: 编码 北京",
            Self::InvalidOperation => "输入 ? URL 查看可用操作",
            Self::InvalidEscape => "每个 % 后需要两个十六进制数字",
            Self::InvalidUtf8 => "解码结果无法表示为 UTF-8 文本",
            Self::NullCharacter => "解码结果含空字符，无法完整复制",
            Self::MissingParameters => "没有可解析的查询参数",
            Self::InvalidUrl => "输入带协议的 URL，例如 https://example.invalid/",
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
        "参数" | "查询参数" | "params" => decode_parameters(value),
        "拆解" | "inspect" => inspect_url(value),
        "去查询串" | "strip-query" => {
            let mut url = Url::parse(value).map_err(|_| UrlEncodingError::InvalidUrl)?;
            url.set_query(None);
            Ok(url.into())
        }
        _ => Err(UrlEncodingError::InvalidOperation),
    }
}

fn inspect_url(input: &str) -> Result<String, UrlEncodingError> {
    let url = Url::parse(input).map_err(|_| UrlEncodingError::InvalidUrl)?;
    let mut lines = vec![format!("协议 {}", url.scheme())];
    if let Some(host) = url.host_str() {
        lines.push(format!("主机 {host}"));
    }
    if let Some(port) = url.port_or_known_default() {
        lines.push(format!("有效端口 {port}"));
    }
    for (label, value) in [
        ("路径", Some(url.path())),
        ("查询串", url.query()),
        ("片段", url.fragment()),
    ] {
        if let Some(value) = value {
            lines.push(format!("{label} {value}"));
        }
    }
    Ok(lines.join("\n"))
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

fn query_text(value: &str) -> &str {
    let is_url = value.split_once("://").is_some_and(|(scheme, _)| {
        scheme.starts_with(|ch: char| ch.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte))
    });
    if value.starts_with(['?', '/']) || is_url {
        let before_fragment = value.split_once('#').map_or(value, |(prefix, _)| prefix);
        before_fragment
            .split_once('?')
            .map_or("", |(_, query)| query)
    } else {
        value
    }
}

fn decode_parameters(value: &str) -> Result<String, UrlEncodingError> {
    let quote = |text: &str| {
        let decoded = decode_value(&text.replace('+', " "))?;
        Ok::<_, UrlEncodingError>(serde_json::to_string(&decoded).expect("字符串可序列化"))
    };
    let mut lines = Vec::new();
    for parameter in query_text(value).split('&').filter(|part| !part.is_empty()) {
        let (name, value) = parameter.split_once('=').unwrap_or((parameter, ""));
        lines.push(format!("{} = {}", quote(name)?, quote(value)?));
    }
    if lines.is_empty() {
        return Err(UrlEncodingError::MissingParameters);
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_the_query_and_preserves_the_path_and_fragment() {
        for (input, expected) in [
            (
                "去查询串 https://example.invalid/a%3Fb?tag=x&source=mail#part?view=1",
                "https://example.invalid/a%3Fb#part?view=1",
            ),
            (
                "STRIP-QUERY https://example.invalid:8443/a?",
                "https://example.invalid:8443/a",
            ),
            (
                "去查询串 https://example.invalid/#?",
                "https://example.invalid/#?",
            ),
        ] {
            assert_eq!(
                convert_url_component(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            convert_url_component("去查询串 /doc?source=mail"),
            Err(UrlEncodingError::InvalidUrl)
        );
    }

    #[test]
    fn shows_parsed_url_components_and_effective_ports() {
        assert_eq!(
            convert_url_component("拆解 HTTPS://user:sample@Example.INVALID:8443/a%20b?x=a+b#part").as_deref(),
            Ok("协议 https\n主机 example.invalid\n有效端口 8443\n路径 /a%20b\n查询串 x=a+b\n片段 part")
        );
        assert_eq!(
            convert_url_component("inspect https://[::1]:443/").as_deref(),
            Ok("协议 https\n主机 [::1]\n有效端口 443\n路径 /")
        );
        assert_eq!(
            convert_url_component("拆解 mailto:dev@example.invalid").as_deref(),
            Ok("协议 mailto\n路径 dev@example.invalid")
        );
        assert_eq!(
            convert_url_component("拆解 /route?x=1"),
            Err(UrlEncodingError::InvalidUrl)
        );
    }

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
    fn reads_form_query_pairs_in_order_and_retains_duplicates() {
        let input = "参数 https://example.invalid/p?tag=a+1&tag=b%2Bc&empty=&q=a%3Db%23c#ignored";
        assert_eq!(
            convert_url_component(input).as_deref(),
            Ok("\"tag\" = \"a 1\"\n\"tag\" = \"b+c\"\n\"empty\" = \"\"\n\"q\" = \"a=b#c\"")
        );
        for (input, expected) in [
            ("params ?=value&flag", "\"\" = \"value\"\n\"flag\" = \"\""),
            ("参数 q=a?b#c", "\"q\" = \"a?b#c\""),
            ("参数 /p?line=x%0Ay", "\"line\" = \"x\\ny\""),
            (
                "参数 redirect=https://example.invalid/?x=y",
                "\"redirect\" = \"https://example.invalid/?x=y\"",
            ),
        ] {
            assert_eq!(
                convert_url_component(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            convert_url_component("参数 https://example.invalid/#?a=b"),
            Err(UrlEncodingError::MissingParameters)
        );
        assert_eq!(
            convert_url_component("参数 ?bad=%2G"),
            Err(UrlEncodingError::InvalidEscape)
        );
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
