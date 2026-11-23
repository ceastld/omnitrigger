use std::{borrow::Cow, collections::HashSet};

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::de::DeserializeOwned;
use url::{Origin, Position, Url};

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
const MAX_QUERY_PARAMETER_UPDATES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UrlEncodingError {
    Incomplete,
    InvalidOperation,
    InvalidEscape,
    InvalidUtf8,
    NullCharacter,
    MissingParameters,
    InvalidUrl,
    InvalidUrlPair,
    InvalidUrlReference,
    InvalidAbsoluteUrlPair,
    RelativeUrlOrigin,
    RelativeUrlUnavailable,
    InvalidParameterPair,
    InvalidParameterList,
    InvalidParameterUpdates,
    InvalidParameterNames,
    ParameterLookupArguments,
    InvalidFilePath,
    InvalidFileUrl,
    OpaqueOrigin,
    InvalidOriginPair,
    OpaquePath,
    InvalidPathSegments,
    InvalidFragmentArgument,
    MissingFragment,
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
            Self::InvalidUrlPair => "用 JSON 字符串数组填写基准网址和相对链接",
            Self::InvalidUrlReference => "无法用该基准网址解析链接",
            Self::InvalidAbsoluteUrlPair => "用 JSON 字符串数组填写两个完整网址，基准网址在前",
            Self::RelativeUrlOrigin => "两个 HTTP(S) 网址须使用相同协议、主机、端口和登录信息",
            Self::RelativeUrlUnavailable => "无法用该基准网址生成可还原的相对链接",
            Self::InvalidParameterPair => "用 JSON 填写名称和值，例如 [\"tag\",\"茶\"]，再输入 URL",
            Self::InvalidParameterList => "用 JSON 数组填写多组名称和值，例如 [[\"tag\",\"茶\"]]",
            Self::InvalidParameterUpdates => {
                "用 JSON 数组填写 1 至 16 组名称和值，再输入 URL。设置时名称须互不重复"
            }
            Self::InvalidParameterNames => "用 JSON 字符串数组填写参数名，再输入 URL",
            Self::ParameterLookupArguments => "用双引号填写参数名，再输入 URL 或查询串",
            Self::InvalidFilePath => "输入绝对文件路径，例如 C:\\文档\\示例.txt",
            Self::InvalidFileUrl => "输入带绝对路径的 file: URL，例如 file:///C:/Docs/example.txt",
            Self::OpaqueOrigin => "请输入具有网络源的 URL，例如 https://example.invalid/",
            Self::InvalidOriginPair => "用 JSON 字符串数组填写两个具有网络源的完整网址",
            Self::OpaquePath => "请输入有层级路径的 URL，例如 https://example.invalid/",
            Self::InvalidPathSegments => {
                "用 JSON 字符串数组填写路径段，再输入 URL。各段不能是 . 或 .."
            }
            Self::InvalidFragmentArgument => "用双引号填写锚点文字，再输入完整 URL",
            Self::MissingFragment => "该 URL 没有 # 锚点",
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
        "表单解码" | "form-decode" => decode_form_value(value),
        "参数" | "查询参数" | "params" => decode_parameters(value, false),
        "参数json" | "params-json" => decode_parameters(value, true),
        "取参数" | "get-param" => lookup_parameter_values(value),
        "生成查询串" | "build-query" => encode_parameters(value),
        "拆解" | "inspect" => inspect_url(value),
        "路径分段" | "path-segments" => decode_path_segments(value),
        "设路径" | "set-path" => set_path_segments(value),
        "拼接" | "resolve" => resolve_url(value),
        "相对" | "relative" => relative_url(value),
        "源" | "源站" | "origin" => Ok(network_origin(value)?.ascii_serialization()),
        "同源" | "same-origin" => compare_url_origins(value),
        "去查询串" | "strip-query" => {
            let mut url = Url::parse(value).map_err(|_| UrlEncodingError::InvalidUrl)?;
            url.set_query(None);
            Ok(url.into())
        }
        "取锚点" | "get-fragment" => {
            let url = Url::parse(value).map_err(|_| UrlEncodingError::InvalidUrl)?;
            decode_value(url.fragment().ok_or(UrlEncodingError::MissingFragment)?)
        }
        "设锚点" | "set-fragment" => {
            let (fragment, address) =
                parameter_argument::<String>(value, UrlEncodingError::InvalidFragmentArgument)?;
            if fragment.contains('\0') {
                return Err(UrlEncodingError::NullCharacter);
            }
            let mut url = Url::parse(address).map_err(|_| UrlEncodingError::InvalidUrl)?;
            let fragment = utf8_percent_encode(&fragment, URL_COMPONENT).to_string();
            url.set_fragment(Some(&fragment));
            Ok(url.into())
        }
        "去锚点" | "strip-fragment" => {
            let mut url = Url::parse(value).map_err(|_| UrlEncodingError::InvalidUrl)?;
            url.set_fragment(None);
            Ok(url.into())
        }
        "去参数" | "strip-param" => remove_parameter(value),
        "去参数组" | "strip-params" => filter_parameter_group(value, false),
        "保留参数" | "keep-params" => filter_parameter_group(value, true),
        "排序参数" | "sort-params" => sort_query_parameters(value),
        "去重参数" | "unique-params" => deduplicate_query_parameters(value),
        "加参数" | "append-param" => write_parameter(value, false),
        "设参数" | "set-param" => write_parameter(value, true),
        "加参数组" | "append-params" => write_parameter_group(value, false),
        "设参数组" | "set-params" => write_parameter_group(value, true),
        "文件链接" | "file-url" => encode_file_path(value),
        "文件路径" | "file-path" => decode_file_path(value),
        _ => Err(UrlEncodingError::InvalidOperation),
    }
}

fn network_origin(input: &str) -> Result<Origin, UrlEncodingError> {
    let url = Url::parse(input).map_err(|_| UrlEncodingError::InvalidUrl)?;
    let origin = url.origin();
    origin
        .is_tuple()
        .then_some(origin)
        .ok_or(UrlEncodingError::OpaqueOrigin)
}

fn compare_url_origins(input: &str) -> Result<String, UrlEncodingError> {
    let [first, second] = url_pair(input, UrlEncodingError::InvalidOriginPair)?;
    let first = network_origin(&first)?;
    let second = network_origin(&second)?;
    let result = if first == second {
        format!("同源 · {}", first.ascii_serialization())
    } else {
        format!(
            "源不同\n第一个 {}\n第二个 {}",
            first.ascii_serialization(),
            second.ascii_serialization()
        )
    };
    Ok(result)
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

fn decode_path_segments(input: &str) -> Result<String, UrlEncodingError> {
    let url = Url::parse(input).map_err(|_| UrlEncodingError::InvalidUrl)?;
    let segments = url
        .path_segments()
        .ok_or(UrlEncodingError::OpaquePath)?
        .map(decode_percent_escapes)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::to_string(&segments).expect("路径段字符串可以序列化为 JSON"))
}

fn set_path_segments(input: &str) -> Result<String, UrlEncodingError> {
    let (segments, address) =
        parameter_argument::<Vec<String>>(input, UrlEncodingError::InvalidPathSegments)?;
    if segments
        .iter()
        .any(|segment| matches!(segment.as_str(), "." | ".."))
    {
        return Err(UrlEncodingError::InvalidPathSegments);
    }
    let mut url = Url::parse(address).map_err(|_| UrlEncodingError::InvalidUrl)?;
    url.path_segments_mut()
        .map_err(|()| UrlEncodingError::OpaquePath)?
        .clear()
        .extend(segments);
    Ok(url.into())
}

fn resolve_url(input: &str) -> Result<String, UrlEncodingError> {
    let [base, reference] = url_pair(input, UrlEncodingError::InvalidUrlPair)?;
    let base = Url::parse(&base).map_err(|_| UrlEncodingError::InvalidUrl)?;
    base.join(&reference)
        .map(Into::into)
        .map_err(|_| UrlEncodingError::InvalidUrlReference)
}

fn url_pair(input: &str, invalid_pair: UrlEncodingError) -> Result<[String; 2], UrlEncodingError> {
    serde_json::from_str(input).map_err(|error| {
        if error.is_eof() {
            UrlEncodingError::Incomplete
        } else {
            invalid_pair
        }
    })
}

fn relative_url(input: &str) -> Result<String, UrlEncodingError> {
    let [base, target] = url_pair(input, UrlEncodingError::InvalidAbsoluteUrlPair)?;
    let base = Url::parse(&base).map_err(|_| UrlEncodingError::InvalidUrl)?;
    let target = Url::parse(&target).map_err(|_| UrlEncodingError::InvalidUrl)?;
    if !matches!(base.scheme(), "http" | "https")
        || base.scheme() != target.scheme()
        || base.authority() != target.authority()
    {
        return Err(UrlEncodingError::RelativeUrlOrigin);
    }
    let round_trips = |reference: &str| {
        !reference.is_empty()
            && Url::parse(reference).is_err()
            && base
                .join(reference)
                .is_ok_and(|resolved| resolved == target)
    };
    if let Some(reference) = base
        .make_relative(&target)
        .filter(|reference| round_trips(reference))
    {
        return Ok(reference);
    }
    let suffix = &target[Position::BeforePath..];
    let reference = if suffix.starts_with("//") {
        // 先写点目录，避免把路径开头的双斜杠解析成另一台主机。
        format!("/.{suffix}")
    } else {
        suffix.to_owned()
    };
    if round_trips(&reference) {
        Ok(reference)
    } else {
        Err(UrlEncodingError::RelativeUrlUnavailable)
    }
}

fn encode_file_path(input: &str) -> Result<String, UrlEncodingError> {
    let path = super::path_tools::strip_path_quotes(input);
    if path.contains(['\0', '"']) {
        return Err(UrlEncodingError::InvalidFilePath);
    }
    let url = if path.ends_with(['\\', '/']) {
        Url::from_directory_path(path)
    } else {
        Url::from_file_path(path)
    };
    url.map(String::from)
        .map_err(|_| UrlEncodingError::InvalidFilePath)
}

fn decode_file_path(input: &str) -> Result<String, UrlEncodingError> {
    let url = Url::parse(input).map_err(|_| UrlEncodingError::InvalidFileUrl)?;
    if url.scheme() != "file" {
        return Err(UrlEncodingError::InvalidFileUrl);
    }
    url.to_file_path()
        .ok()
        .and_then(|path| path.into_os_string().into_string().ok())
        .filter(|path| !path.contains('\0'))
        .ok_or(UrlEncodingError::InvalidFileUrl)
}

fn decode_percent_escapes(value: &str) -> Result<Cow<'_, str>, UrlEncodingError> {
    let mut rest = value;
    while let Some((_, suffix)) = rest.split_once('%') {
        let digits = suffix
            .get(..2)
            .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or(UrlEncodingError::InvalidEscape)?;
        rest = &suffix[digits.len()..];
    }
    percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| UrlEncodingError::InvalidUtf8)
}

fn decode_value(value: &str) -> Result<String, UrlEncodingError> {
    let decoded = decode_percent_escapes(value)?;
    if decoded.contains('\0') {
        return Err(UrlEncodingError::NullCharacter);
    }
    Ok(decoded.into_owned())
}

fn decode_form_value(value: &str) -> Result<String, UrlEncodingError> {
    decode_value(&value.replace('+', " "))
}

fn decode_form_component(value: &str) -> Result<String, UrlEncodingError> {
    Ok(decode_percent_escapes(&value.replace('+', " "))?.into_owned())
}

fn remove_parameter(value: &str) -> Result<String, UrlEncodingError> {
    let (name, address) = value
        .split_once(char::is_whitespace)
        .ok_or(UrlEncodingError::Incomplete)?;
    let name = decode_form_value(name)?;
    filter_query_parameters(address.trim_start(), |key| key != Some(name.as_str()))
}

fn filter_parameter_group(input: &str, keep_matching: bool) -> Result<String, UrlEncodingError> {
    let (names, address) =
        parameter_argument::<HashSet<String>>(input, UrlEncodingError::InvalidParameterNames)?;
    if names.iter().any(|name| name.contains('\0')) {
        return Err(UrlEncodingError::InvalidParameterNames);
    }
    filter_query_parameters(address, |key| {
        key.is_some_and(|key| names.contains(key)) == keep_matching
    })
}

fn filter_query_parameters(
    address: &str,
    keep: impl Fn(Option<&str>) -> bool,
) -> Result<String, UrlEncodingError> {
    let mut url = Url::parse(address).map_err(|_| UrlEncodingError::InvalidUrl)?;
    let Some(query) = url.query() else {
        return Ok(url.into());
    };
    let mut parts = Vec::new();
    let mut removed = false;
    for part in query.split('&') {
        if keep(parameter_name(part).as_deref()) {
            parts.push(part);
        } else {
            removed = true;
        }
    }
    if removed {
        let query = parts
            .iter()
            .any(|part| !part.is_empty())
            .then(|| parts.join("&"));
        url.set_query(query.as_deref());
    }
    Ok(url.into())
}

fn parameter_name(part: &str) -> Option<String> {
    if part.is_empty() {
        return None;
    }
    let key = part.split_once('=').map_or(part, |(key, _)| key);
    decode_form_value(key).ok()
}

fn deduplicate_query_parameters(address: &str) -> Result<String, UrlEncodingError> {
    let mut url = Url::parse(address).map_err(|_| UrlEncodingError::InvalidUrl)?;
    let Some(query) = url.query() else {
        return Ok(url.into());
    };
    let mut seen = HashSet::new();
    let mut retained = Vec::new();
    for part in query.split('&') {
        if part.is_empty() {
            retained.push(part);
            continue;
        }
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        if seen.insert((decode_form_component(name)?, decode_form_component(value)?)) {
            retained.push(part);
        }
    }
    let query = retained.join("&");
    url.set_query(Some(&query));
    Ok(url.into())
}

fn sort_query_parameters(address: &str) -> Result<String, UrlEncodingError> {
    let mut url = Url::parse(address).map_err(|_| UrlEncodingError::InvalidUrl)?;
    let Some(query) = url.query().filter(|query| !query.is_empty()) else {
        return Ok(url.into());
    };
    let mut fields = Vec::new();
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let name = part.split_once('=').map_or(part, |(name, _)| name);
        let name = decode_form_component(name)?;
        fields.push((name.encode_utf16().collect::<Vec<_>>(), part));
    }
    fields.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut sorted = fields.into_iter().map(|(_, part)| part);
    let query = query
        .split('&')
        .map(|part| {
            if part.is_empty() {
                part
            } else {
                sorted.next().expect("每个非空参数在排序列表中出现一次")
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    url.set_query(Some(&query));
    Ok(url.into())
}

fn parameter_argument<T: DeserializeOwned>(
    input: &str,
    invalid_arguments: UrlEncodingError,
) -> Result<(T, &str), UrlEncodingError> {
    let mut arguments = serde_json::Deserializer::from_str(input).into_iter::<T>();
    let value = arguments
        .next()
        .ok_or(UrlEncodingError::Incomplete)?
        .map_err(|error| {
            if error.is_eof() {
                UrlEncodingError::Incomplete
            } else {
                invalid_arguments
            }
        })?;
    let address = &input[arguments.byte_offset()..];
    if address.is_empty() {
        return Err(UrlEncodingError::Incomplete);
    }
    if !address.starts_with(char::is_whitespace) {
        return Err(invalid_arguments);
    }
    Ok((value, address.trim_start()))
}

fn write_parameter(input: &str, replace_existing: bool) -> Result<String, UrlEncodingError> {
    let (pair, address) =
        parameter_argument::<[String; 2]>(input, UrlEncodingError::InvalidParameterPair)?;
    write_query_parameters(address, [pair], replace_existing)
}

fn write_parameter_group(input: &str, replace_existing: bool) -> Result<String, UrlEncodingError> {
    let (pairs, address) =
        parameter_argument::<Vec<[String; 2]>>(input, UrlEncodingError::InvalidParameterUpdates)?;
    let mut names = HashSet::new();
    if !(1..=MAX_QUERY_PARAMETER_UPDATES).contains(&pairs.len())
        || (replace_existing && pairs.iter().any(|[name, _]| !names.insert(name)))
    {
        return Err(UrlEncodingError::InvalidParameterUpdates);
    }
    write_query_parameters(address, pairs, replace_existing)
}

fn write_query_parameters(
    address: &str,
    pairs: impl IntoIterator<Item = [String; 2]>,
    replace_existing: bool,
) -> Result<String, UrlEncodingError> {
    let mut url = Url::parse(address).map_err(|_| UrlEncodingError::InvalidUrl)?;
    let mut pairs: Vec<_> = pairs
        .into_iter()
        .map(|[name, value]| {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer.append_pair(&name, &value);
            (name, Some(serializer.finish()))
        })
        .collect();
    let mut parts = Vec::new();
    if let Some(query) = url.query().filter(|query| !query.is_empty()) {
        for part in query.split('&') {
            let replacement = replace_existing
                .then(|| parameter_name(part))
                .flatten()
                .and_then(|name| pairs.iter_mut().find(|pair| pair.0 == name));
            if let Some((_, replacement)) = replacement {
                parts.extend(replacement.take().map(Cow::Owned));
            } else {
                parts.push(Cow::Borrowed(part));
            }
        }
    }
    parts.extend(
        pairs
            .into_iter()
            .filter_map(|(_, pair)| pair.map(Cow::Owned)),
    );
    url.set_query(Some(&parts.join("&")));
    Ok(url.into())
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

fn decode_parameters(value: &str, json: bool) -> Result<String, UrlEncodingError> {
    let decode = |value: &str| {
        if json {
            // JSON 转义可以表示空字符，生成的输出仍能作为文本复制。
            decode_form_component(value)
        } else {
            decode_form_value(value)
        }
    };
    let pairs = decode_query_pairs(value, decode)?;
    if json {
        return Ok(serde_json::to_string(&pairs).expect("字符串数组可序列化"));
    }
    if pairs.is_empty() {
        return Err(UrlEncodingError::MissingParameters);
    }
    let quote = |value: &str| serde_json::to_string(value).expect("字符串可序列化");
    Ok(pairs
        .iter()
        .map(|[name, value]| format!("{} = {}", quote(name), quote(value)))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn lookup_parameter_values(input: &str) -> Result<String, UrlEncodingError> {
    let (name, source) =
        parameter_argument::<String>(input, UrlEncodingError::ParameterLookupArguments)?;
    url_parameter_values(source, &name)
}

pub(super) fn url_parameter_values(source: &str, name: &str) -> Result<String, UrlEncodingError> {
    let values: Vec<_> = decode_query_pairs(source, decode_form_component)?
        .into_iter()
        .filter_map(|[key, value]| (key == name).then_some(value))
        .collect();
    Ok(serde_json::to_string(&values).expect("字符串数组可序列化"))
}

fn decode_query_pairs(
    source: &str,
    decode: impl Fn(&str) -> Result<String, UrlEncodingError>,
) -> Result<Vec<[String; 2]>, UrlEncodingError> {
    query_text(source)
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            Ok([decode(name)?, decode(value)?])
        })
        .collect()
}

fn encode_parameters(input: &str) -> Result<String, UrlEncodingError> {
    let pairs = serde_json::from_str::<Vec<[String; 2]>>(input).map_err(|error| {
        if error.is_eof() {
            UrlEncodingError::Incomplete
        } else {
            UrlEncodingError::InvalidParameterList
        }
    })?;
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for [name, value] in pairs {
        serializer.append_pair(&name, &value);
    }
    Ok(serializer.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gets_every_value_for_an_exact_decoded_query_parameter_name() {
        for (input, expected) in [
            (
                r#"取参数 "tag" https://example.invalid/?tag=a+1&t%61g=b%2Bc&Tag=别的&tag=#part"#,
                r#"["a 1","b+c",""]"#,
            ),
            (r#"get-param "q" ?x=1&q=%00&q=%0A"#, r#"["\u0000","\n"]"#),
            (
                r#"get-param "tag" https://example.invalid/?Tag=x#tag=a"#,
                "[]",
            ),
            (r#"get-param "" =空名&plain&="#, r#"["空名",""]"#),
        ] {
            assert_eq!(
                convert_url_component(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            convert_url_component(r#"取参数 "tag" tag=%GG"#),
            Err(UrlEncodingError::InvalidEscape)
        );
        assert_eq!(
            convert_url_component(r#"取参数 ["tag"] tag=x"#),
            Err(UrlEncodingError::ParameterLookupArguments)
        );
    }

    #[cfg(windows)]
    #[test]
    fn decodes_and_sets_hierarchical_path_segments_without_rewriting_the_query() {
        for (input, expected) in [
            (
                "路径分段 https://example.invalid/a%2Fb/+%20/?original=1#part",
                r#"["a/b","+ ",""]"#,
            ),
            (
                r#"设路径 ["文档","a/b",""] https://example.invalid/old?x=a%20b#keep"#,
                "https://example.invalid/%E6%96%87%E6%A1%A3/a%2Fb/?x=a%20b#keep",
            ),
            (
                "set-path [] https://example.invalid/old#keep",
                "https://example.invalid/#keep",
            ),
            ("path-segments https://example.invalid/", r#"[""]"#),
        ] {
            assert_eq!(
                convert_url_component(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            convert_url_component(r#"设路径 [".."] https://example.invalid/"#),
            Err(UrlEncodingError::InvalidPathSegments)
        );
        assert_eq!(
            convert_url_component("路径分段 mailto:test@example.invalid"),
            Err(UrlEncodingError::OpaquePath)
        );
        assert_eq!(
            convert_url_component("路径分段 https://example.invalid/%FF"),
            Err(UrlEncodingError::InvalidUtf8)
        );
    }

    #[test]
    fn round_trips_absolute_windows_and_unc_paths_as_file_urls() {
        for (path, url) in [
            (
                r"C:\Docs\北京 #1.txt",
                "file:///C:/Docs/%E5%8C%97%E4%BA%AC%20%231.txt",
            ),
            (
                r"\\example.invalid\share\a b.txt",
                "file://example.invalid/share/a%20b.txt",
            ),
            (r"C:\Docs\", "file:///C:/Docs/"),
        ] {
            assert_eq!(
                convert_url_component(&format!("文件链接 \"{path}\"")).as_deref(),
                Ok(url)
            );
            assert_eq!(
                convert_url_component(&format!("文件路径 {url}")).as_deref(),
                Ok(path)
            );
        }
        assert_eq!(
            convert_url_component(r"file-url C:Docs\a.txt"),
            Err(UrlEncodingError::InvalidFilePath)
        );
        assert_eq!(
            convert_url_component("file-path https://example.invalid/a"),
            Err(UrlEncodingError::InvalidFileUrl)
        );
        assert_eq!(
            convert_url_component("file-path file:///C:/bad%00name"),
            Err(UrlEncodingError::InvalidFileUrl)
        );
    }

    #[test]
    fn appends_or_sets_url_parameters_and_retains_unrelated_serialization() {
        for (query, expected) in [
            (
                r#"加参数 ["tag","茶 + 咖啡"] https://example.invalid/doc?tag=first#part"#,
                "https://example.invalid/doc?tag=first&tag=%E8%8C%B6+%2B+%E5%92%96%E5%95%A1#part",
            ),
            (
                r#"设参数 ["page","2"] https://example.invalid/doc?first&%70age=1&q=a%20b&page=3#part"#,
                "https://example.invalid/doc?first&page=2&q=a%20b#part",
            ),
            (
                r#"append-param ["a b",""] https://example.invalid/?x=%7e&&#f"#,
                "https://example.invalid/?x=%7e&&&a+b=#f",
            ),
            (
                r#"set-param ["tag"," "] https://example.invalid/doc?"#,
                "https://example.invalid/doc?tag=+",
            ),
            (
                r#"set-param ["a +","新"] https://example.invalid/?a+%2B=0&a%20%2b=1&%FF=4&foo"#,
                "https://example.invalid/?a+%2B=%E6%96%B0&%FF=4&foo",
            ),
        ] {
            assert_eq!(
                convert_url_component(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            convert_url_component(r#"set-param ["tag","val"]"#),
            Err(UrlEncodingError::Incomplete)
        );
        assert_eq!(
            convert_url_component(r#"set-param ["tag"] https://example.invalid/"#),
            Err(UrlEncodingError::InvalidParameterPair)
        );
    }

    #[test]
    fn sets_and_appends_parameter_groups_in_order_without_reencoding_unrelated_fields() {
        for (query, expected) in [
            (
                r#"设参数组 [["lang","zh"],["q","新 +"],["tag","茶"],["page","2"]] https://example.invalid/doc?page=1&raw=a%20b&q=first&%70age=3&Q=stay#x"#,
                "https://example.invalid/doc?page=2&raw=a%20b&q=%E6%96%B0+%2B&Q=stay&lang=zh&tag=%E8%8C%B6#x",
            ),
            (
                r#"append-params [["tag","a +"],["tag",""],["","v"]] https://example.invalid/?tag=first#x"#,
                "https://example.invalid/?tag=first&tag=a+%2B&tag=&=v#x",
            ),
            (
                r#"set-params [["","v"],["a +","b"]] https://example.invalid/?=0&&a%20%2b=1&%FF=2#x"#,
                "https://example.invalid/?=v&&a+%2B=b&%FF=2#x",
            ),
        ] {
            assert_eq!(convert_url_component(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            r#"设参数组 [["tag","a"],["tag","b"]] https://example.invalid/"#,
            r#"加参数组 [] https://example.invalid/"#,
            r#"设参数组 [["tag"]] https://example.invalid/"#,
        ] {
            assert_eq!(
                convert_url_component(query),
                Err(UrlEncodingError::InvalidParameterUpdates),
                "{query}"
            );
        }
        let repeated = ["[\"tag\",\"a\"]"; MAX_QUERY_PARAMETER_UPDATES + 1].join(",");
        let query = format!("加参数组 [{repeated}] https://example.invalid/");
        assert_eq!(
            convert_url_component(&query),
            Err(UrlEncodingError::InvalidParameterUpdates)
        );
    }

    #[test]
    fn removes_all_entries_for_one_decoded_parameter_and_keeps_the_other_pairs() {
        for (input, expected) in [
            (
                "去参数 source https://example.invalid/doc?q=a%20b&source=mail&%73ource=other#part",
                "https://example.invalid/doc?q=a%20b#part",
            ),
            (
                "strip-param a+b https://example.invalid/?a%20b=1&a+b&a%2Bb=3&%FF=4",
                "https://example.invalid/?a%2Bb=3&%FF=4",
            ),
            (
                "去参数 tag https://example.invalid/?tag=1&tag#part",
                "https://example.invalid/#part",
            ),
            (
                "去参数 Tag https://example.invalid/?tag=%2f&q=a+b",
                "https://example.invalid/?tag=%2f&q=a+b",
            ),
            (
                "去参数 tag https://example.invalid/path",
                "https://example.invalid/path",
            ),
        ] {
            assert_eq!(
                convert_url_component(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            convert_url_component("去参数 %GG https://example.invalid/"),
            Err(UrlEncodingError::InvalidEscape)
        );
        assert_eq!(
            convert_url_component("去参数 source /relative?source=mail"),
            Err(UrlEncodingError::InvalidUrl)
        );
    }

    #[test]
    fn compares_normalized_url_origin_tuples() {
        for (query, expected) in [
            (
                r#"同源 ["HTTPS://user:sample@Example.INVALID:443/a?x=1","https://example.invalid/b#p"]"#,
                "同源 · https://example.invalid",
            ),
            (
                r#"same-origin ["https://[2001:0db8::1]:443/a","https://[2001:db8::1]/b"]"#,
                "同源 · https://[2001:db8::1]",
            ),
            (
                r#"同源 ["https://example.invalid/a","https://example.invalid:8443/b"]"#,
                "源不同\n第一个 https://example.invalid\n第二个 https://example.invalid:8443",
            ),
            (
                r#"同源 ["http://example.invalid/a","https://example.invalid/b"]"#,
                "源不同\n第一个 http://example.invalid\n第二个 https://example.invalid",
            ),
        ] {
            assert_eq!(
                convert_url_component(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            convert_url_component(r#"同源 ["file:///C:/a","file:///C:/a"]"#),
            Err(UrlEncodingError::OpaqueOrigin)
        );
        assert_eq!(
            convert_url_component(r#"同源 ["https://example.invalid"]"#),
            Err(UrlEncodingError::InvalidOriginPair)
        );
    }

    #[test]
    fn extracts_the_normalized_network_origin() {
        for (input, expected) in [
            (
                "源 HTTPS://user:sample@Example.INVALID:8443/doc?x=1#part",
                "https://example.invalid:8443",
            ),
            (
                "origin https://[2001:db8::1]:443/path",
                "https://[2001:db8::1]",
            ),
            (
                "源站 https://bücher.example/",
                "https://xn--bcher-kva.example",
            ),
        ] {
            assert_eq!(
                convert_url_component(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            convert_url_component("源 mailto:dev@example.invalid"),
            Err(UrlEncodingError::OpaqueOrigin)
        );
        assert_eq!(
            convert_url_component("源 /doc"),
            Err(UrlEncodingError::InvalidUrl)
        );
    }

    #[test]
    fn makes_copyable_relative_urls_that_resolve_back_to_the_target() {
        assert_eq!(
            convert_url_component(r#"相对 ["https://example.invalid/docs/api/","https://example.invalid/docs/guide?lang=zh#intro"]"#).as_deref(),
            Ok("../guide?lang=zh#intro")
        );
        for (base, target) in [
            (
                "https://example.invalid/docs/?q=1",
                "https://example.invalid/docs/",
            ),
            (
                "https://example.invalid/docs/",
                "https://example.invalid/docs/",
            ),
            (
                "https://example.invalid/docs/",
                "https://example.invalid/docs/a:b",
            ),
            (
                "https://example.invalid/",
                "https://example.invalid//guide%20one#part",
            ),
            (
                "https://example.invalid/doc",
                "https://example.invalid/doc?#",
            ),
        ] {
            let pair = serde_json::to_string(&[base, target]).unwrap();
            let reference = convert_url_component(&format!("relative {pair}")).unwrap();
            assert!(!reference.is_empty());
            assert!(Url::parse(&reference).is_err());
            let joined = Url::parse(base).unwrap().join(&reference).unwrap();
            assert_eq!(joined, Url::parse(target).unwrap());
        }
        for (query, error) in [
            (
                r#"相对 ["https://example.invalid/"]"#,
                UrlEncodingError::InvalidAbsoluteUrlPair,
            ),
            (
                r#"相对 ["https://example.invalid/","/docs"]"#,
                UrlEncodingError::InvalidUrl,
            ),
            (
                r#"相对 ["http://example.invalid/","https://example.invalid/"]"#,
                UrlEncodingError::RelativeUrlOrigin,
            ),
            (
                r#"相对 ["https://example.invalid/","https://example.invalid:444/"]"#,
                UrlEncodingError::RelativeUrlOrigin,
            ),
            (
                r#"相对 ["https://example.invalid/","https://a@example.invalid/"]"#,
                UrlEncodingError::RelativeUrlOrigin,
            ),
        ] {
            assert_eq!(convert_url_component(query), Err(error), "{query}");
        }
    }

    #[test]
    fn resolves_url_references_with_the_base_urls_directory_and_query() {
        for (query, expected) in [
            (
                r#"拼接 ["https://example.invalid/docs/api/","../guide?lang=zh#intro"]"#,
                "https://example.invalid/docs/guide?lang=zh#intro",
            ),
            (
                r#"resolve ["https://example.invalid/docs/index.html","入 门.html"]"#,
                "https://example.invalid/docs/%E5%85%A5%20%E9%97%A8.html",
            ),
            (
                r##"resolve ["https://example.invalid/doc?x=1#old","#new"]"##,
                "https://example.invalid/doc?x=1#new",
            ),
            (
                r#"resolve ["https://example.invalid/doc","//cdn.example.invalid/a"]"#,
                "https://cdn.example.invalid/a",
            ),
        ] {
            assert_eq!(
                convert_url_component(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            (
                r#"拼接 ["https://example.invalid/"]"#,
                UrlEncodingError::InvalidUrlPair,
            ),
            (r#"拼接 ["/docs/","file"]"#, UrlEncodingError::InvalidUrl),
            (
                r#"拼接 ["mailto:hello@example.invalid","file"]"#,
                UrlEncodingError::InvalidUrlReference,
            ),
        ] {
            assert_eq!(convert_url_component(query), Err(error), "{query}");
        }
    }

    #[test]
    fn removes_multiple_decoded_query_names_without_reserializing_other_pairs() {
        for (input, expected) in [
            (
                r#"去参数组 ["source","campaign"] https://example.invalid/doc?source=mail&q=a%20b&campaign=autumn&%73ource=extra#part"#,
                "https://example.invalid/doc?q=a%20b#part",
            ),
            (
                r#"strip-params ["a b","a+b",""] https://example.invalid/?a+b=1&a%20b=2&a%2Bb=3&=0&x=%2f&x=1+2&%FF=kept"#,
                "https://example.invalid/?x=%2f&x=1+2&%FF=kept",
            ),
            (
                r#"去参数组 ["Tag"] https://example.invalid/?Tag&tag=keep"#,
                "https://example.invalid/?tag=keep",
            ),
            (
                r#"strip-params [] https://example.invalid/?#part"#,
                "https://example.invalid/?#part",
            ),
            (
                r#"去参数组 ["x","x"] https://example.invalid/?x=1&x=2#part"#,
                "https://example.invalid/#part",
            ),
        ] {
            assert_eq!(
                convert_url_component(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for input in [
            r#"去参数组 ["x",1] https://example.invalid/"#,
            r#"去参数组 ["x"]https://example.invalid/"#,
            r#"去参数组 ["\u0000"] https://example.invalid/"#,
        ] {
            assert_eq!(
                convert_url_component(input),
                Err(UrlEncodingError::InvalidParameterNames),
                "{input}"
            );
        }
    }

    #[test]
    fn keeps_the_first_spelling_of_each_decoded_parameter_pair() {
        for (query, expected) in [
            (
                "去重参数 https://example.invalid/doc?tag=a+b&&%74ag=a%20b&tag=a%2Bb&flag&flag=#part",
                "https://example.invalid/doc?tag=a+b&&tag=a%2Bb&flag#part",
            ),
            (
                "unique-params https://example.invalid/?x=1&x=01&X=1&x=1",
                "https://example.invalid/?x=1&x=01&X=1",
            ),
            (
                "去重参数 https://example.invalid/p?#part",
                "https://example.invalid/p?#part",
            ),
            (
                "去重参数 https://example.invalid/p#part",
                "https://example.invalid/p#part",
            ),
        ] {
            assert_eq!(convert_url_component(query).as_deref(), Ok(expected));
        }
        assert_eq!(
            convert_url_component("去重参数 https://example.invalid/?a=%ZZ"),
            Err(UrlEncodingError::InvalidEscape)
        );
    }

    #[test]
    fn sorts_decoded_names_stably_by_utf16_without_changing_raw_parameters() {
        for (query, expected) in [
            (
                "排序参数 https://example.invalid/p?z=a%20b&&%61=a+b&a=%2B&b+z=%ZZ#part",
                "https://example.invalid/p?%61=a+b&&a=%2B&b+z=%ZZ&z=a%20b#part",
            ),
            (
                "sort-params https://example.invalid/?%EE%80%80=2&%F0%9F%98%80=1",
                "https://example.invalid/?%F0%9F%98%80=1&%EE%80%80=2",
            ),
            (
                "sort-params https://example.invalid/p?#part",
                "https://example.invalid/p?#part",
            ),
        ] {
            assert_eq!(convert_url_component(query).as_deref(), Ok(expected));
        }
        assert_eq!(
            convert_url_component("排序参数 https://example.invalid/?%2G=1"),
            Err(UrlEncodingError::InvalidEscape)
        );
    }

    #[test]
    fn keeps_only_selected_query_names_in_their_original_order_and_spelling() {
        for (input, expected) in [
            (
                r#"保留参数 ["q","page"] https://example.invalid/doc?source=mail&q=a%20b&page=2&q=a+b#part"#,
                "https://example.invalid/doc?q=a%20b&page=2&q=a+b#part",
            ),
            (
                r#"keep-params ["tag","a b",""] https://example.invalid/?tag&Tag=1&a+b=%2f&&%74ag=2&a%2Bb=3&=0&%FF=x"#,
                "https://example.invalid/?tag&a+b=%2f&%74ag=2&=0",
            ),
            (
                r#"保留参数 [] https://example.invalid/?q=1#part"#,
                "https://example.invalid/#part",
            ),
            (
                r#"keep-params ["q"] https://example.invalid/#part"#,
                "https://example.invalid/#part",
            ),
        ] {
            assert_eq!(
                convert_url_component(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            convert_url_component(r#"保留参数 ["q",1] https://example.invalid/"#),
            Err(UrlEncodingError::InvalidParameterNames)
        );
    }

    #[test]
    fn gets_sets_and_removes_fragments_while_retaining_the_query() {
        for (query, expected) in [
            (
                r#"设锚点 "目录 #2 + 100%" https://example.invalid/doc?q=a%20b#old"#,
                "https://example.invalid/doc?q=a%20b#%E7%9B%AE%E5%BD%95%20%232%20%2B%20100%25",
            ),
            (
                "取锚点 https://example.invalid/doc?q=x#%E7%9B%AE%E5%BD%95%20%232+100%25",
                "目录 #2+100%",
            ),
            (
                "去锚点 https://example.invalid/a%23b?q=x%23y#part?view=1",
                "https://example.invalid/a%23b?q=x%23y",
            ),
            (
                r#"set-fragment "" https://example.invalid/doc?x=y"#,
                "https://example.invalid/doc?x=y#",
            ),
            ("get-fragment https://example.invalid/#", ""),
        ] {
            assert_eq!(
                convert_url_component(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            convert_url_component("取锚点 https://example.invalid/doc?q=x"),
            Err(UrlEncodingError::MissingFragment)
        );
        assert_eq!(
            convert_url_component("取锚点 https://example.invalid/doc#%ZZ"),
            Err(UrlEncodingError::InvalidEscape)
        );
        assert_eq!(
            convert_url_component(r#"设锚点 "\u0000" https://example.invalid/doc"#),
            Err(UrlEncodingError::NullCharacter)
        );
    }

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
    fn round_trips_ordered_query_pairs_with_duplicates_and_empty_strings() {
        let query = "tag=a+1&tag=b%2Bc&=blank&empty=&data=%00";
        let pairs = r#"[["tag","a 1"],["tag","b+c"],["","blank"],["empty",""],["data","\u0000"]]"#;
        assert_eq!(
            convert_url_component(&format!("参数JSON https://example.invalid/?{query}#x"))
                .as_deref(),
            Ok(pairs)
        );
        assert_eq!(
            convert_url_component(&format!("生成查询串 {pairs}")).as_deref(),
            Ok(query)
        );
        assert_eq!(convert_url_component("params-json ?").as_deref(), Ok("[]"));
        assert_eq!(convert_url_component("build-query []").as_deref(), Ok(""));
        assert_eq!(
            convert_url_component("参数JSON ?data=%GG"),
            Err(UrlEncodingError::InvalidEscape)
        );
        assert_eq!(
            convert_url_component(r#"生成查询串 [["tag",2]]"#),
            Err(UrlEncodingError::InvalidParameterList)
        );
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
