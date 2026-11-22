use std::{
    borrow::Cow,
    cmp::{Ordering, Reverse},
    collections::{HashMap, HashSet},
    fmt::Write as _,
    sync::OnceLock,
};

use base64::{
    engine::general_purpose::{
        GeneralPurpose, STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD,
    },
    Engine,
};
use regex::{Regex, RegexBuilder};
use sha2::{Digest, Sha256, Sha512};
use unicode_segmentation::UnicodeSegmentation;

use super::{
    calculator::{narrow_ascii_graphic, FULLWIDTH_ASCII_OFFSET},
    csv_tools::{append_bounded_csv_row, csv_cell},
    fraction::parse_fraction_literal,
    operators::{json_argument_and_tail, select_index_ranges, IndexRangeError},
};

const MAX_EXPANDED_TEXT_BYTES: usize = 1024 * 1024;
const MAX_DIFF_LINES: usize = 512;
const MAX_PIPELINE_STEPS: usize = 8;

#[derive(Clone, Copy)]
pub(super) enum TextLengthUnit {
    Graphemes,
    CodePoints,
    Utf16Units,
    Utf8Bytes,
}

impl TextLengthUnit {
    pub(super) fn parse(label: &str) -> Option<Self> {
        match label.trim().to_ascii_lowercase().as_str() {
            "字素" | "字素簇" | "graphemes" => Some(Self::Graphemes),
            "码位" | "codepoints" => Some(Self::CodePoints),
            "utf-16 单元" | "utf16" => Some(Self::Utf16Units),
            "utf-8 字节" | "utf8" | "bytes" => Some(Self::Utf8Bytes),
            _ => None,
        }
    }

    pub(super) fn count(self, text: &str) -> usize {
        match self {
            Self::Graphemes => text.graphemes(true).count(),
            Self::CodePoints => text.chars().count(),
            Self::Utf16Units => text.encode_utf16().count(),
            Self::Utf8Bytes => text.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextError {
    MissingText,
    UnclosedQuote,
    IncompleteTransform,
    UnknownOperation,
    PipelineArguments,
    PipelineStep { index: usize, cause: &'static str },
    IncompleteReplacement,
    InvalidReplacementArguments,
    IncompleteTextArgument,
    InvalidTextArgument,
    IncompleteLineComparison,
    InvalidLineComparison,
    TooManyDiffLines,
    EmptyDelimiter,
    InvalidColumn,
    CsvColumnCount,
    MissingColumn,
    InvalidLineRange,
    InvalidGraphemeRange,
    MissingLine,
    InvalidNumberLine,
    InvalidLineEnding,
    EmptySearchText,
    InvalidRegex,
    IncompleteRegexGroup,
    UnknownRegexGroup,
    MissingRegexCaptures,
    ResultTooLarge,
    NullCharacter,
    InvalidBase64,
    InvalidHexBytes,
    InvalidUtf8,
    InvalidHexUtf16Units,
    InvalidUtf16,
    InvalidCodePoint,
    InvalidJsonString,
}

impl TextError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::MissingText => "输入要统计的文字",
            Self::UnclosedQuote => "补全引号后继续",
            Self::IncompleteTransform => "输入操作和文本，例如 文本: 大写 hello",
            Self::UnknownOperation => "输入 ? 文本 查看可用操作",
            Self::PipelineArguments => {
                "填写 1 至 8 个文本操作名及原文，例如 [\"去空行\",\"按行去重\"]"
            }
            Self::PipelineStep { cause, .. } => cause,
            Self::IncompleteReplacement => "输入查找内容、替换内容和原文",
            Self::InvalidReplacementArguments => "查找内容和替换内容要加引号，并用空格分隔",
            Self::IncompleteTextArgument => "输入带引号的内容和原文",
            Self::InvalidTextArgument => "第一个参数要加引号，并和原文用空格分开",
            Self::IncompleteLineComparison => "输入两组加引号的文本，组间用空格分开",
            Self::InvalidLineComparison => "用空格分隔两组加引号的文本",
            Self::TooManyDiffLines => "逐行比较最多支持每组 512 行",
            Self::EmptyDelimiter => "分隔符不能为空",
            Self::InvalidColumn => "输入从 1 开始的列号，例如 文本: 取列 \"\\t\" 2",
            Self::MissingColumn => "至少一行的列数不足",
            Self::CsvColumnCount => "填写 1 至 128 的列数，例如 文本: 按行分组CSV 3",
            Self::InvalidLineRange => "输入从 1 开始的行号或区间，例如 文本: 取行 2,4-6",
            Self::InvalidGraphemeRange => "输入从 1 开始的字符位置或区间，例如 文本: 逐行截取 2-4",
            Self::MissingLine => "指定行号超出了文本行数",
            Self::InvalidNumberLine => "每行请输入一个数字或分数，例如 -2.5、3/2 或 1e2",
            Self::InvalidLineEnding => "填写 LF 或 CRLF，然后粘贴原文",
            Self::EmptySearchText => "请输入要查找的文字",
            Self::InvalidRegex => "请检查或缩短正则表达式",
            Self::IncompleteRegexGroup => "输入组名或组号、带引号的正则表达式和原文",
            Self::UnknownRegexGroup => "正则表达式中没有指定的捕获组",
            Self::MissingRegexCaptures => "请用括号定义至少一个要输出为列的捕获组",
            Self::ResultTooLarge => "文本结果超过 1 MiB",
            Self::NullCharacter => "结果含空字符，无法完整复制",
            Self::InvalidBase64 => "Base64 编码无效",
            Self::InvalidHexBytes => "输入成对的十六进制数字，例如 E5 8C 97",
            Self::InvalidUtf8 => "解码结果不是 UTF-8 文本",
            Self::InvalidHexUtf16Units => {
                "每个 UTF-16 码元使用四位十六进制数字，例如 5317 D83D DE00"
            }
            Self::InvalidUtf16 => "UTF-16 代理码元必须成对且顺序正确",
            Self::InvalidCodePoint => "输入以空格分隔的 Unicode 码位，例如 U+5317 U+4EAC",
            Self::InvalidJsonString => "请输入带双引号的 JSON 字符串",
        }
    }
}

fn text_payload(input: &str) -> Result<&str, TextError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(TextError::MissingText);
    }
    match input.chars().next() {
        Some('"') => input[1..].strip_suffix('"').ok_or(TextError::UnclosedQuote),
        Some('“') => input['“'.len_utf8()..]
            .strip_suffix('”')
            .ok_or(TextError::UnclosedQuote),
        _ => Ok(input),
    }
}

pub(super) fn count_text(input: &str) -> Result<String, TextError> {
    let text = text_payload(input)?;
    let characters = text.graphemes(true).count();
    Ok(format!("{characters} 字符 · {} UTF-8 字节", text.len()))
}

pub(super) fn transform_text(input: &str) -> Result<String, TextError> {
    let (operation, text) = input
        .split_once(char::is_whitespace)
        .ok_or(TextError::IncompleteTransform)?;
    let operation = operation.to_ascii_lowercase();
    match operation.as_str() {
        "依次处理" | "pipeline" => return transform_text_pipeline(text),
        "替换" | "replace" => return replace_text(text),
        "正则替换" | "regex-replace" => return replace_regex(text),
        "按行拼接" | "join-lines" => return join_lines(text),
        "逐行配对" | "zip-lines" => return pair_lines_as_csv(text),
        "按行分组csv" | "chunk-lines-csv" => return chunk_lines_as_csv(text),
        "拆分成行" | "split-lines" => return split_lines(text),
        "转换换行" | "convert-line-endings" => return convert_line_endings(text),
        "正则拆分成行" | "regex-split-lines" => {
            let (pattern, text) = regex_text_argument(text)?;
            return Ok(pattern.split(text).collect::<Vec<_>>().join("\n"));
        }
        "逐行加前缀" | "prefix-lines" => return decorate_lines(text, true),
        "逐行加后缀" | "suffix-lines" => return decorate_lines(text, false),
        "逐行去前缀" | "strip-line-prefix" => return strip_line_decoration(text, true),
        "逐行去后缀" | "strip-line-suffix" => return strip_line_decoration(text, false),
        "逐行截取" | "slice-lines" => return slice_line_graphemes(text),
        "取列" | "column" => return select_text_column(text),
        "取行" | "pick-lines" => return select_text_line_ranges(text, true),
        "删行" | "drop-lines" => return select_text_line_ranges(text, false),
        "保留行" | "keep-lines" => return filter_lines(text, true),
        "排除行" | "exclude-lines" => return filter_lines(text, false),
        "共有行" | "行交集" | "intersect-lines" => {
            return compare_line_sets(text, LineSetOperation::Intersection);
        }
        "行差集" | "subtract-lines" => {
            return compare_line_sets(text, LineSetOperation::FirstOnly);
        }
        "行并集" | "union-lines" => return compare_line_sets(text, LineSetOperation::Union),
        "行对称差" | "symmetric-difference-lines" => {
            return compare_line_sets(text, LineSetOperation::SymmetricDifference);
        }
        "逐行比较" | "diff-lines" => return diff_lines(text),
        "正则保留行" | "keep-regex-lines" => return filter_regex_lines(text, true),
        "正则排除行" | "exclude-regex-lines" => return filter_regex_lines(text, false),
        "正则计数" | "regex-count" => {
            let (pattern, text) = regex_text_argument(text)?;
            return Ok(format!("{} 处匹配", pattern.find_iter(text).count()));
        }
        "正则提取" | "regex-extract" => {
            let (pattern, text) = regex_text_argument(text)?;
            return Ok(text
                .lines()
                .filter_map(|line| pattern.find(line).map(|matched| matched.as_str()))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        "正则提取全部" | "regex-extract-all" => {
            let (pattern, text) = regex_text_argument(text)?;
            return Ok(pattern
                .find_iter(text)
                .filter(|matched| !matched.is_empty())
                .map(|matched| matched.as_str())
                .collect::<Vec<_>>()
                .join("\n"));
        }
        "正则取组" | "regex-capture" => return extract_regex_group(text),
        "正则取表" | "regex-to-csv" => return regex_captures_to_csv(text),
        "json还原" | "json-unquote" => return decode_json_string(text),
        _ => {}
    }
    transform_text_payload(&operation, text_payload(text)?)
}

fn transform_text_payload(operation: &str, text: &str) -> Result<String, TextError> {
    match operation {
        "大写" | "upper" | "uppercase" => Ok(text.to_uppercase()),
        "小写" | "lower" | "lowercase" => Ok(text.to_lowercase()),
        "长度详情" | "length-details" => Ok(format!(
            "{} 字素簇 · {} Unicode 码位\n{} UTF-16 单元 · {} UTF-8 字节",
            TextLengthUnit::Graphemes.count(text),
            TextLengthUnit::CodePoints.count(text),
            TextLengthUnit::Utf16Units.count(text),
            TextLengthUnit::Utf8Bytes.count(text)
        )),
        "显示空白" | "show-whitespace" => Ok(show_whitespace(text)),
        "字频" | "字符频数" | "grapheme-frequency" => Ok(count_graphemes(text)),
        "词频" | "word-frequency" => Ok(frequency_csv("词语", text.unicode_words())),
        "去ansi样式" | "strip-ansi-style" => Ok(strip_ansi_styles(text)),
        "下划线命名" | "snake-case" => Ok(join_identifier_words(text, "_")),
        "短横线命名" | "kebab-case" => Ok(join_identifier_words(text, "-")),
        "小驼峰命名" | "camel-case" => Ok(camel_case(text, false)),
        "大驼峰命名" | "pascal-case" => Ok(camel_case(text, true)),
        "转半角" | "halfwidth" => Ok(text
            .chars()
            .map(|ch| match ch {
                '\u{3000}' => ' ',
                _ => narrow_ascii_graphic(ch),
            })
            .collect()),
        "转全角" | "fullwidth" => Ok(text
            .chars()
            .map(|ch| match ch {
                ' ' => '\u{3000}',
                '!'..='~' => char::from_u32(u32::from(ch) + FULLWIDTH_ASCII_OFFSET).unwrap_or(ch),
                _ => ch,
            })
            .collect()),
        "合并空白" | "collapse" => Ok(text.split_whitespace().collect::<Vec<_>>().join(" ")),
        "正则转义" | "regex-escape" => Ok(regex::escape(text)),
        "逐行去空白" | "trim-lines" => {
            Ok(text.lines().map(str::trim).collect::<Vec<_>>().join("\n"))
        }
        "去公共缩进" | "dedent" => Ok(remove_common_indent(text)),
        "换行统计" | "line-ending-counts" => Ok(count_line_endings(text)),
        "倒序" | "reverse" => Ok(text.graphemes(true).rev().collect()),
        "按行倒序" | "reverse-lines" => Ok(text.lines().rev().collect::<Vec<_>>().join("\n")),
        "按行去重" | "unique-lines" => Ok(join_distinct_lines(text.lines())),
        "按行排序" | "sort-lines" => {
            let mut lines: Vec<_> = text.lines().collect();
            lines.sort_unstable();
            Ok(lines.join("\n"))
        }
        "自然排序" | "natural-sort" => {
            let mut lines: Vec<_> = text.lines().collect();
            lines.sort_by(|left, right| compare_natural(left, right));
            Ok(lines.join("\n"))
        }
        "数值排序" | "sort-numeric" => sort_numeric_lines(text, false),
        order @ ("按长度排序" | "sort-line-length" | "按长度降序" | "sort-line-length-desc") =>
        {
            let mut lines: Vec<_> = text.lines().collect();
            if matches!(order, "按长度降序" | "sort-line-length-desc") {
                lines.sort_by_cached_key(|line| Reverse(line.graphemes(true).count()));
            } else {
                lines.sort_by_cached_key(|line| line.graphemes(true).count());
            }
            Ok(lines.join("\n"))
        }
        "数值降序" | "sort-numeric-desc" => sort_numeric_lines(text, true),
        "去空行" | "remove-empty-lines" => Ok(text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")),
        "合并空行" | "collapse-empty-lines" => Ok(collapse_empty_lines(text)),
        "按行编号" | "number-lines" => Ok(text
            .lines()
            .enumerate()
            .map(|(index, line)| format!("{}\t{line}", index + 1))
            .collect::<Vec<_>>()
            .join("\n")),
        "按行计数" | "line-counts" => Ok(count_lines(text)),
        "sha256" | "sha-256" => Ok(format!("{:x}", Sha256::digest(text.as_bytes()))),
        "sha512" | "sha-512" => Ok(format!("{:x}", Sha512::digest(text.as_bytes()))),
        "base64编码" | "base64-encode" => Ok(STANDARD.encode(text)),
        "base64解码" | "base64-decode" => decode_base64(text, &STANDARD, &STANDARD_NO_PAD),
        "base64url编码" | "base64url-encode" => Ok(URL_SAFE_NO_PAD.encode(text)),
        "base64url解码" | "base64url-decode" => decode_base64(text, &URL_SAFE, &URL_SAFE_NO_PAD),
        "utf8编码" | "utf8-hex" => Ok(encode_hex_units(text.bytes(), 2)),
        "utf8解码" | "utf8-hex-decode" => decode_utf8_bytes(text),
        "utf16编码" | "utf16-hex" => Ok(encode_hex_units(text.encode_utf16(), 4)),
        "utf16解码" | "utf16-hex-decode" => decode_utf16_units(text),
        "码位" | "codepoints" => Ok(encode_code_points(text)),
        "码位解码" | "codepoints-decode" => decode_code_points(text),
        "json转义" | "json-quote" => {
            Ok(serde_json::to_string(text).expect("字符串可以序列化为 JSON"))
        }
        "json还原" | "json-unquote" => decode_json_string(text),
        "html转义" | "html-escape" => Ok(escape_html(text)),
        "html还原" | "html-unescape" => Ok(unescape_html(text)),
        "markdown转义" | "markdown-escape" => Ok(escape_markdown(text)),
        "markdown还原" | "markdown-unescape" => Ok(unescape_markdown(text)),
        _ => Err(TextError::UnknownOperation),
    }
}

fn transform_text_pipeline(input: &str) -> Result<String, TextError> {
    let (steps, data) =
        json_argument_and_tail::<Vec<String>>(input).ok_or(TextError::PipelineArguments)?;
    if !(1..=MAX_PIPELINE_STEPS).contains(&steps.len())
        || steps
            .iter()
            .any(|step| step.trim().is_empty() || step.contains(['\r', '\n', '\0']))
    {
        return Err(TextError::PipelineArguments);
    }
    let mut current = Cow::Borrowed(text_payload(data)?);
    for (index, step) in steps.iter().enumerate() {
        let step = step.trim().to_ascii_lowercase();
        let output =
            transform_text_payload(&step, &current).map_err(|error| TextError::PipelineStep {
                index: index + 1,
                cause: if error == TextError::UnknownOperation {
                    "请选择无需额外参数的文本操作，例如 逐行去空白"
                } else {
                    error.message()
                },
            })?;
        if output.len() > MAX_EXPANDED_TEXT_BYTES {
            return Err(TextError::PipelineStep {
                index: index + 1,
                cause: TextError::ResultTooLarge.message(),
            });
        }
        current = Cow::Owned(output);
    }
    Ok(current.into_owned())
}

fn identifier_words(text: &str) -> Vec<&str> {
    let mut graphemes = text.grapheme_indices(true).peekable();
    let mut words = Vec::new();
    let mut word_start = 0;
    let mut previous: Option<char> = None;
    while let Some((index, grapheme)) = graphemes.next() {
        let character = grapheme.chars().next().expect("字素至少包含一个字符");
        if !character.is_alphanumeric() {
            if previous.take().is_some() {
                words.push(&text[word_start..index]);
            }
            continue;
        }
        let follows_lowercase_or_digit =
            previous.is_some_and(|value| value.is_lowercase() || value.is_numeric());
        let follows_acronym = previous.is_some_and(char::is_uppercase)
            && graphemes
                .peek()
                .and_then(|(_, next)| next.chars().next())
                .is_some_and(char::is_lowercase);
        if character.is_uppercase() && (follows_lowercase_or_digit || follows_acronym) {
            words.push(&text[word_start..index]);
            word_start = index;
        } else if previous.is_none() {
            word_start = index;
        }
        previous = Some(character);
    }
    if previous.is_some() {
        words.push(&text[word_start..]);
    }
    words
}

fn join_identifier_words(text: &str, delimiter: &str) -> String {
    identifier_words(text)
        .into_iter()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(delimiter)
}

fn camel_case(text: &str, uppercase_first: bool) -> String {
    let mut words = identifier_words(text).into_iter();
    let mut output = String::new();
    if !uppercase_first {
        output = words.next().unwrap_or_default().to_lowercase();
    }
    for word in words {
        let mut characters = word.chars();
        let first = characters.next().expect("命名片段至少包含一个字符");
        output.extend(first.to_uppercase());
        output.push_str(&characters.as_str().to_lowercase());
    }
    output
}

pub(super) fn compare_natural(left: &str, right: &str) -> Ordering {
    let (mut left, mut right) = (left.as_bytes(), right.as_bytes());
    while let (Some((&left_byte, left_tail)), Some((&right_byte, right_tail))) =
        (left.split_first(), right.split_first())
    {
        let order = if left_byte.is_ascii_digit() && right_byte.is_ascii_digit() {
            let (left_number, next_left) = decimal_run(left);
            let (right_number, next_right) = decimal_run(right);
            (left, right) = (next_left, next_right);
            left_number
                .len()
                .cmp(&right_number.len())
                .then_with(|| left_number.cmp(right_number))
        } else {
            (left, right) = (left_tail, right_tail);
            left_byte.cmp(&right_byte)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

fn decimal_run(text: &[u8]) -> (&[u8], &[u8]) {
    let end = text
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(text.len());
    let (digits, rest) = text.split_at(end);
    let significant = digits
        .iter()
        .position(|&byte| byte != b'0')
        .unwrap_or(digits.len());
    (&digits[significant..], rest)
}

fn show_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            ' ' => result.push('␠'),
            '\t' => result.push('␉'),
            '\n' => result.push('␊'),
            '\r' => result.push('␍'),
            '\u{000B}' => result.push('␋'),
            '\u{000C}' => result.push('␌'),
            character if character.is_whitespace() => {
                write!(result, "<U+{:04X}>", u32::from(character)).expect("写入字符串不会失败");
            }
            character => result.push(character),
        }
    }
    result
}

fn strip_ansi_styles(text: &str) -> String {
    static STYLES: OnceLock<Regex> = OnceLock::new();
    let styles = STYLES.get_or_init(|| {
        Regex::new(r"(?:\x1B\[|\u{009B})[0-9:;]*m").expect("固定的终端样式表达式有效")
    });
    styles.replace_all(text, "").into_owned()
}

pub(super) fn newline_pattern() -> &'static Regex {
    static NEWLINES: OnceLock<Regex> = OnceLock::new();
    NEWLINES.get_or_init(|| Regex::new(r"\r\n|\n|\r").expect("固定的换行表达式有效"))
}

fn collapse_empty_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut start = 0;
    let mut previous_was_blank = false;
    let line_ends = newline_pattern()
        .find_iter(text)
        .map(|newline| (newline.start(), newline.end()))
        .chain(std::iter::once((text.len(), text.len())));
    for (content_end, end) in line_ends {
        let blank = text[start..content_end].trim().is_empty();
        if !blank || !previous_was_blank {
            result.push_str(&text[start..end]);
        }
        previous_was_blank = blank;
        start = end;
    }
    result
}

fn count_line_endings(text: &str) -> String {
    let (mut crlf, mut lf, mut cr) = (0, 0, 0);
    for newline in newline_pattern()
        .find_iter(text)
        .map(|found| found.as_str())
    {
        match newline {
            "\r\n" => crlf += 1,
            "\n" => lf += 1,
            "\r" => cr += 1,
            _ => unreachable!("只匹配三种换行符"),
        }
    }
    format!("CRLF {crlf} · LF {lf} · CR {cr}")
}

fn convert_line_endings(input: &str) -> Result<String, TextError> {
    let (target, text) = input
        .trim_start()
        .split_once(char::is_whitespace)
        .ok_or(TextError::InvalidLineEnding)?;
    let replacement = match target.to_ascii_lowercase().as_str() {
        "lf" => "\n",
        "crlf" => "\r\n",
        _ => return Err(TextError::InvalidLineEnding),
    };
    Ok(newline_pattern()
        .replace_all(text_payload(text)?, replacement)
        .into_owned())
}

fn remove_common_indent(text: &str) -> String {
    let mut indents = text.lines().filter_map(|line| {
        let content = line.trim_start_matches([' ', '\t']);
        (!content.is_empty()).then_some(&line[..line.len() - content.len()])
    });
    let Some(mut common) = indents.next() else {
        return text.to_owned();
    };
    for indent in indents {
        let length = common
            .bytes()
            .zip(indent.bytes())
            .take_while(|(left, right)| left == right)
            .count();
        if length == 0 {
            return text.to_owned();
        }
        common = &common[..length];
    }
    text.split_inclusive('\n')
        .map(|line| line.strip_prefix(common).unwrap_or(line))
        .collect()
}

fn sort_numeric_lines(text: &str, descending: bool) -> Result<String, TextError> {
    let mut values = text
        .lines()
        .map(|line| {
            parse_fraction_literal(line)
                .map(|value| (value, line))
                .map_err(|_| TextError::InvalidNumberLine)
        })
        .collect::<Result<Vec<_>, _>>()?;
    values.sort_by(|(left, _), (right, _)| {
        if descending {
            right.cmp(left)
        } else {
            left.cmp(right)
        }
    });
    Ok(values
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n"))
}

fn count_lines(text: &str) -> String {
    frequency_counts(text.lines())
        .into_iter()
        .map(|(line, count)| format!("{count}\t{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count_graphemes(text: &str) -> String {
    let graphemes = text
        .graphemes(true)
        .filter(|grapheme| !grapheme.chars().all(char::is_whitespace));
    frequency_csv("字符", graphemes)
}

fn frequency_csv<'a>(header: &str, items: impl Iterator<Item = &'a str>) -> String {
    let mut result = format!("{header},次数");
    for (item, count) in frequency_counts(items) {
        write!(result, "\r\n{},{count}", csv_cell(item)).expect("写入 String 不会出错");
    }
    result
}

fn frequency_counts<'a>(items: impl Iterator<Item = &'a str>) -> Vec<(&'a str, usize)> {
    let mut indices = HashMap::new();
    let mut counts = Vec::new();
    for item in items {
        let index = *indices.entry(item).or_insert_with(|| {
            counts.push((item, 0usize));
            counts.len() - 1
        });
        counts[index].1 += 1;
    }
    counts.sort_by_key(|&(_, count)| Reverse(count));
    counts
}

fn encode_hex_units<T: std::fmt::UpperHex>(units: impl Iterator<Item = T>, width: usize) -> String {
    let mut encoded = String::new();
    for unit in units {
        if !encoded.is_empty() {
            encoded.push(' ');
        }
        write!(encoded, "{unit:0width$X}").expect("写入 String 不会出错");
    }
    encoded
}

fn decode_utf8_bytes(text: &str) -> Result<String, TextError> {
    decoded_utf8(decode_hex_units(text, 2, TextError::InvalidHexBytes)?)
}

fn decode_utf16_units(text: &str) -> Result<String, TextError> {
    let units = decode_hex_units(text, 4, TextError::InvalidHexUtf16Units)?;
    decoded_text(String::from_utf16(&units).map_err(|_| TextError::InvalidUtf16)?)
}

fn decode_hex_units<T: TryFrom<u16>>(
    text: &str,
    width: usize,
    invalid: TextError,
) -> Result<Vec<T>, TextError> {
    let mut units = Vec::with_capacity(text.len() / width);
    for group in text.split_whitespace() {
        let digits = group
            .strip_prefix("0x")
            .or_else(|| group.strip_prefix("0X"))
            .unwrap_or(group);
        if digits.is_empty()
            || !digits.len().is_multiple_of(width)
            || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid);
        }
        for chunk in digits.as_bytes().chunks_exact(width) {
            let chunk = std::str::from_utf8(chunk).expect("十六进制数字为 ASCII");
            let value = u16::from_str_radix(chunk, 16).map_err(|_| invalid)?;
            units.push(T::try_from(value).map_err(|_| invalid)?);
        }
    }
    Ok(units)
}

fn encode_code_points(text: &str) -> String {
    let mut encoded = String::new();
    for character in text.chars() {
        if !encoded.is_empty() {
            encoded.push(' ');
        }
        write!(encoded, "U+{:04X}", u32::from(character)).expect("写入 String 不会出错");
    }
    encoded
}

fn decode_json_string(text: &str) -> Result<String, TextError> {
    let decoded: String =
        serde_json::from_str(text.trim()).map_err(|_| TextError::InvalidJsonString)?;
    if decoded.contains('\0') {
        return Err(TextError::NullCharacter);
    }
    Ok(decoded)
}

fn escape_markdown(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii_punctuation() {
            result.push('\\');
        }
        result.push(character);
    }
    result
}

fn unescape_markdown(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && characters.peek().is_some_and(char::is_ascii_punctuation) {
            result.push(characters.next().expect("下一个字符已检查"));
        } else {
            result.push(character);
        }
    }
    result
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn unescape_html(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some((before, reference)) = remaining.split_once('&') {
        decoded.push_str(before);
        let character = reference
            .find(['&', ';'])
            .filter(|&end| reference.as_bytes()[end] == b';')
            .and_then(|end| html_reference(&reference[..end]).map(|ch| (ch, end)));
        remaining = if let Some((ch, end)) = character {
            decoded.push(ch);
            &reference[end + 1..]
        } else {
            decoded.push('&');
            reference
        };
    }
    decoded.push_str(remaining);
    decoded
}

fn html_reference(reference: &str) -> Option<char> {
    const C1_REFERENCES: [u32; 32] = [
        0x20ac, 0x0081, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x008d, 0x017d, 0x008f, 0x0090, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x009d, 0x017e, 0x0178,
    ];
    Some(match reference {
        "amp" | "AMP" => '&',
        "lt" | "LT" => '<',
        "gt" | "GT" => '>',
        "quot" | "QUOT" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        _ => {
            let number = reference.strip_prefix('#')?;
            let (radix, digits) = number
                .strip_prefix(['x', 'X'])
                .map_or((10, number), |digits| (16, digits));
            if digits.is_empty() || !digits.chars().all(|ch| ch.is_digit(radix)) {
                return None;
            }
            let value = u32::from_str_radix(digits, radix).unwrap_or(u32::MAX);
            let value = match value {
                0 => 0xfffd,
                0x80..=0x9f => C1_REFERENCES[(value - 0x80) as usize],
                _ => value,
            };
            char::from_u32(value).unwrap_or(char::REPLACEMENT_CHARACTER)
        }
    })
}

fn decode_code_points(text: &str) -> Result<String, TextError> {
    text.split_whitespace()
        .map(|code| {
            let digits = code
                .strip_prefix("U+")
                .or_else(|| code.strip_prefix("u+"))
                .ok_or(TextError::InvalidCodePoint)?;
            if !(1..=6).contains(&digits.len())
                || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(TextError::InvalidCodePoint);
            }
            let character = u32::from_str_radix(digits, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or(TextError::InvalidCodePoint)?;
            if character == '\0' {
                return Err(TextError::NullCharacter);
            }
            Ok(character)
        })
        .collect()
}

fn decode_base64(
    text: &str,
    padded: &GeneralPurpose,
    unpadded: &GeneralPurpose,
) -> Result<String, TextError> {
    let encoded: Vec<_> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let decoder = if encoded.ends_with(b"=") {
        padded
    } else {
        unpadded
    };
    let bytes = decoder
        .decode(encoded)
        .map_err(|_| TextError::InvalidBase64)?;
    decoded_utf8(bytes)
}

fn decoded_utf8(bytes: Vec<u8>) -> Result<String, TextError> {
    decoded_text(String::from_utf8(bytes).map_err(|_| TextError::InvalidUtf8)?)
}

fn decoded_text(result: String) -> Result<String, TextError> {
    if result.contains('\0') {
        return Err(TextError::NullCharacter);
    }
    Ok(result)
}

fn replacement_argument(input: &str) -> Result<(String, &str), TextError> {
    let input = input.trim_start();
    let (operand, rest) = if let Some(quoted) = input.strip_prefix('“') {
        let (operand, rest) = quoted
            .split_once('”')
            .ok_or(TextError::IncompleteReplacement)?;
        (operand.to_owned(), rest)
    } else {
        let mut values = serde_json::Deserializer::from_str(input).into_iter::<String>();
        let operand = values
            .next()
            .ok_or(TextError::IncompleteReplacement)?
            .map_err(|error| {
                if error.is_eof() {
                    TextError::IncompleteReplacement
                } else {
                    TextError::InvalidReplacementArguments
                }
            })?;
        (operand, &input[values.byte_offset()..])
    };
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return Err(TextError::InvalidReplacementArguments);
    }
    Ok((operand, rest))
}

fn quoted_text_argument(input: &str) -> Result<(String, &str), TextError> {
    let (argument, rest) = replacement_argument(input).map_err(|error| match error {
        TextError::IncompleteReplacement => TextError::IncompleteTextArgument,
        TextError::InvalidReplacementArguments => TextError::InvalidTextArgument,
        _ => error,
    })?;
    let text = text_payload(rest).map_err(|error| match error {
        TextError::MissingText => TextError::IncompleteTextArgument,
        _ => error,
    })?;
    if argument.contains('\0') {
        return Err(TextError::NullCharacter);
    }
    Ok((argument, text))
}

fn join_lines(input: &str) -> Result<String, TextError> {
    let (delimiter, text) = quoted_text_argument(input)?;
    let lines: Vec<_> = text.lines().collect();
    let content_bytes = lines.iter().map(|line| line.len()).sum::<usize>();
    let delimiter_bytes = delimiter
        .len()
        .saturating_mul(lines.len().saturating_sub(1));
    if content_bytes.saturating_add(delimiter_bytes) > MAX_EXPANDED_TEXT_BYTES {
        return Err(TextError::ResultTooLarge);
    }
    Ok(lines.join(&delimiter))
}

fn decorate_lines(input: &str, prepend: bool) -> Result<String, TextError> {
    let (decoration, text) = quoted_text_argument(input)?;
    let lines: Vec<_> = text.lines().collect();
    let content_bytes = lines.iter().map(|line| line.len()).sum::<usize>();
    let result_bytes = content_bytes
        .saturating_add(lines.len().saturating_sub(1))
        .saturating_add(decoration.len().saturating_mul(lines.len()));
    if result_bytes > MAX_EXPANDED_TEXT_BYTES {
        return Err(TextError::ResultTooLarge);
    }
    let (prefix, suffix) = if prepend {
        (decoration.as_str(), "")
    } else {
        ("", decoration.as_str())
    };
    let mut result = String::with_capacity(result_bytes);
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            result.push('\n');
        }
        result.push_str(prefix);
        result.push_str(line);
        result.push_str(suffix);
    }
    Ok(result)
}

fn strip_line_decoration(input: &str, prefix: bool) -> Result<String, TextError> {
    let (decoration, text) = quoted_text_argument(input)?;
    let mut result = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let content = line.lines().next().unwrap_or_default();
        let remaining = if prefix {
            content.strip_prefix(decoration.as_str())
        } else {
            content.strip_suffix(decoration.as_str())
        };
        result.push_str(remaining.unwrap_or(content));
        result.push_str(&line[content.len()..]);
    }
    Ok(result)
}

pub(super) struct GraphemeRange {
    start: usize,
    end: usize,
}

impl GraphemeRange {
    pub(super) fn parse(input: &str) -> Result<Self, TextError> {
        let (start, end) = input.split_once('-').unwrap_or((input, input));
        let start = start
            .parse::<usize>()
            .map_err(|_| TextError::InvalidGraphemeRange)?;
        let end = end
            .parse::<usize>()
            .map_err(|_| TextError::InvalidGraphemeRange)?;
        if start == 0 || end < start {
            return Err(TextError::InvalidGraphemeRange);
        }
        Ok(Self { start, end })
    }

    pub(super) fn slice<'a>(&self, text: &'a str) -> &'a str {
        let mut graphemes = text.grapheme_indices(true);
        let Some((start, _)) = graphemes.nth(self.start - 1) else {
            return "";
        };
        let end = graphemes
            .nth(self.end - self.start)
            .map_or(text.len(), |(offset, _)| offset);
        &text[start..end]
    }
}

fn slice_line_graphemes(input: &str) -> Result<String, TextError> {
    let (range, text) = input
        .trim_start()
        .split_once(char::is_whitespace)
        .ok_or(TextError::InvalidGraphemeRange)?;
    let range = GraphemeRange::parse(range)?;
    let text = text_payload(text)?;
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for newline in newline_pattern().find_iter(text) {
        result.push_str(range.slice(&text[cursor..newline.start()]));
        result.push_str(newline.as_str());
        cursor = newline.end();
    }
    result.push_str(range.slice(&text[cursor..]));
    Ok(result)
}

fn split_lines(input: &str) -> Result<String, TextError> {
    let (delimiter, text) = quoted_text_argument(input)?;
    if delimiter.is_empty() {
        return Err(TextError::EmptyDelimiter);
    }
    Ok(text.replace(&delimiter, "\n"))
}

fn filter_lines(input: &str, keep_matches: bool) -> Result<String, TextError> {
    let (needle, text) = quoted_text_argument(input)?;
    if needle.is_empty() {
        return Err(TextError::EmptySearchText);
    }
    Ok(select_lines(text, |line| {
        line.contains(needle.as_str()) == keep_matches
    }))
}

fn filter_regex_lines(input: &str, keep_matches: bool) -> Result<String, TextError> {
    let (pattern, text) = regex_text_argument(input)?;
    Ok(select_lines(text, |line| {
        pattern.is_match(line) == keep_matches
    }))
}

fn select_text_column(input: &str) -> Result<String, TextError> {
    let (delimiter, input) = quoted_text_argument(input)?;
    if delimiter.is_empty() {
        return Err(TextError::EmptyDelimiter);
    }
    let (column, text) = input
        .split_once(char::is_whitespace)
        .ok_or(TextError::IncompleteTextArgument)?;
    let column = column
        .parse::<usize>()
        .ok()
        .and_then(|value| value.checked_sub(1))
        .ok_or(TextError::InvalidColumn)?;
    let values = text_payload(text)?
        .lines()
        .map(|line| {
            line.split(&delimiter)
                .nth(column)
                .ok_or(TextError::MissingColumn)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(values.join("\n"))
}

fn select_text_line_ranges(input: &str, keep_selected: bool) -> Result<String, TextError> {
    let (ranges, text) = input
        .split_once(char::is_whitespace)
        .ok_or(TextError::IncompleteTextArgument)?;
    let text = text_payload(text)?;
    let lines: Vec<_> = text.lines().collect();
    let selected = select_index_ranges(ranges, lines.len()).map_err(|error| match error {
        IndexRangeError::Invalid => TextError::InvalidLineRange,
        IndexRangeError::OutOfBounds => TextError::MissingLine,
    })?;
    Ok(lines
        .into_iter()
        .zip(selected)
        .filter_map(|(line, selected)| (selected == keep_selected).then_some(line))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn regex_text_argument(input: &str) -> Result<(Regex, &str), TextError> {
    let (pattern, text) = quoted_text_argument(input)?;
    Ok((compile_regex(&pattern)?, text))
}

fn extract_regex_group(input: &str) -> Result<String, TextError> {
    let (group, input) = input
        .trim_start()
        .split_once(char::is_whitespace)
        .ok_or(TextError::IncompleteRegexGroup)?;
    let (pattern, text) = regex_text_argument(input).map_err(|error| match error {
        TextError::IncompleteTextArgument => TextError::IncompleteRegexGroup,
        _ => error,
    })?;
    let index = regex_group_index(&pattern, group)?;

    Ok(pattern
        .captures_iter(text)
        .filter_map(|captures| captures.get(index))
        .filter(|matched| !matched.is_empty())
        .map(|matched| matched.as_str())
        .collect::<Vec<_>>()
        .join("\n"))
}

pub(super) fn regex_group_index(pattern: &Regex, group: &str) -> Result<usize, TextError> {
    group
        .parse::<usize>()
        .ok()
        .filter(|&index| index < pattern.captures_len())
        .or_else(|| pattern.capture_names().position(|name| name == Some(group)))
        .ok_or(TextError::UnknownRegexGroup)
}

fn regex_captures_to_csv(input: &str) -> Result<String, TextError> {
    let (pattern, text) = regex_text_argument(input)?;
    if pattern.captures_len() <= 1 {
        return Err(TextError::MissingRegexCaptures);
    }
    let mut result = pattern
        .capture_names()
        .enumerate()
        .skip(1)
        .map(|(index, name)| name.map_or_else(|| index.to_string(), str::to_owned))
        .collect::<Vec<_>>()
        .join(",");
    for captures in pattern.captures_iter(text) {
        result.push_str("\r\n");
        for (column, matched) in captures.iter().skip(1).enumerate() {
            if column > 0 {
                result.push(',');
            }
            let value = matched.map_or("", |matched| matched.as_str());
            if value.contains('\0') {
                return Err(TextError::NullCharacter);
            }
            result.push_str(&csv_cell(value));
        }
        if result.len() > MAX_EXPANDED_TEXT_BYTES {
            return Err(TextError::ResultTooLarge);
        }
    }
    Ok(result)
}

pub(super) fn compile_regex(pattern: &str) -> Result<Regex, TextError> {
    if pattern.is_empty() {
        return Err(TextError::EmptySearchText);
    }
    RegexBuilder::new(pattern)
        .size_limit(256 * 1024)
        .dfa_size_limit(256 * 1024)
        .build()
        .map_err(|_| TextError::InvalidRegex)
}

fn select_lines(text: &str, matches: impl Fn(&str) -> bool) -> String {
    text.lines()
        .filter(|line| matches(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_line_comparison(input: &str) -> Result<(String, String), TextError> {
    let read_lists = || {
        let (first, rest) = replacement_argument(input)?;
        let (second, rest) = replacement_argument(rest)?;
        if !rest.trim().is_empty() {
            return Err(TextError::InvalidReplacementArguments);
        }
        Ok((first, second))
    };
    let (first, second) = read_lists().map_err(|error| match error {
        TextError::IncompleteReplacement => TextError::IncompleteLineComparison,
        TextError::InvalidReplacementArguments => TextError::InvalidLineComparison,
        _ => error,
    })?;
    if first.contains('\0') || second.contains('\0') {
        return Err(TextError::NullCharacter);
    }
    Ok((first, second))
}

fn pair_lines_as_csv(input: &str) -> Result<String, TextError> {
    let (left, right) = read_line_comparison(input)?;
    let (mut left, mut right) = (left.lines(), right.lines());
    let mut output = String::new();
    loop {
        let (first, second) = (left.next(), right.next());
        if first.is_none() && second.is_none() {
            return Ok(output);
        }
        append_bounded_csv_row(
            &mut output,
            [first.unwrap_or_default(), second.unwrap_or_default()].into_iter(),
            TextError::ResultTooLarge,
        )?;
    }
}

fn chunk_lines_as_csv(input: &str) -> Result<String, TextError> {
    let (columns, text) = input
        .trim_start()
        .split_once(char::is_whitespace)
        .ok_or(TextError::CsvColumnCount)?;
    let columns = columns
        .parse::<usize>()
        .ok()
        .filter(|columns| (1..=128).contains(columns))
        .ok_or(TextError::CsvColumnCount)?;
    let mut lines = text_payload(text)?.lines().peekable();
    let mut output = String::new();
    while lines.peek().is_some() {
        let fields = lines
            .by_ref()
            .take(columns)
            .chain(std::iter::repeat(""))
            .take(columns);
        append_bounded_csv_row(&mut output, fields, TextError::ResultTooLarge)?;
    }
    Ok(output)
}

enum LineSetOperation {
    Intersection,
    FirstOnly,
    Union,
    SymmetricDifference,
}

fn compare_line_sets(input: &str, operation: LineSetOperation) -> Result<String, TextError> {
    let (first, second) = read_line_comparison(input)?;
    if matches!(operation, LineSetOperation::Union) {
        return Ok(join_distinct_lines(first.lines().chain(second.lines())));
    }
    let second_values: HashSet<_> = second.lines().collect();
    let keep_common = matches!(operation, LineSetOperation::Intersection);
    let selected_first_lines = first
        .lines()
        .filter(|&line| second_values.contains(line) == keep_common);
    if matches!(operation, LineSetOperation::SymmetricDifference) {
        let first_values: HashSet<_> = first.lines().collect();
        let selected_second_lines = second.lines().filter(|&line| !first_values.contains(line));
        return Ok(join_distinct_lines(
            selected_first_lines.chain(selected_second_lines),
        ));
    }
    Ok(join_distinct_lines(selected_first_lines))
}

fn join_distinct_lines<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut seen = HashSet::new();
    lines
        .filter(|&line| seen.insert(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn diff_lines(input: &str) -> Result<String, TextError> {
    let (before, after) = read_line_comparison(input)?;
    let before: Vec<_> = before.lines().take(MAX_DIFF_LINES + 1).collect();
    let after: Vec<_> = after.lines().take(MAX_DIFF_LINES + 1).collect();
    if before.len() > MAX_DIFF_LINES || after.len() > MAX_DIFF_LINES {
        return Err(TextError::TooManyDiffLines);
    }
    if before == after {
        return Ok("两组文本各行相同".to_owned());
    }
    let columns = after.len() + 1;
    let index = |left, right| left * columns + right;
    let mut common = vec![0_u16; (before.len() + 1) * columns];
    // 记录两组后缀的最长公共子序列长度，从首行回溯增删。
    for (left, before_line) in before.iter().enumerate().rev() {
        for (right, after_line) in after.iter().enumerate().rev() {
            common[index(left, right)] = if before_line == after_line {
                common[index(left + 1, right + 1)] + 1
            } else {
                common[index(left + 1, right)].max(common[index(left, right + 1)])
            };
        }
    }
    let mut result = String::new();
    let (mut left, mut right) = (0, 0);
    while left < before.len() || right < after.len() {
        let (prefix, line) =
            if left < before.len() && right < after.len() && before[left] == after[right] {
                let line = before[left];
                left += 1;
                right += 1;
                ("  ", line)
            } else if right == after.len()
                || (left < before.len()
                    && common[index(left + 1, right)] >= common[index(left, right + 1)])
            {
                let line = before[left];
                left += 1;
                ("- ", line)
            } else {
                let line = after[right];
                right += 1;
                ("+ ", line)
            };
        if result.len() + prefix.len() + line.len() + 1 > MAX_EXPANDED_TEXT_BYTES {
            return Err(TextError::ResultTooLarge);
        }
        writeln!(result, "{prefix}{line}").expect("写入 String 不会出错");
    }
    result.pop();
    Ok(result)
}

fn replacement_parts(input: &str) -> Result<(String, String, &str), TextError> {
    let (needle, rest) = replacement_argument(input)?;
    let (replacement, rest) = replacement_argument(rest)?;
    let text = text_payload(rest).map_err(|error| match error {
        TextError::MissingText => TextError::IncompleteReplacement,
        _ => error,
    })?;
    Ok((needle, replacement, text))
}

fn replace_text(input: &str) -> Result<String, TextError> {
    let (needle, replacement, text) = replacement_parts(input)?;
    Ok(replace_literal_matches(text, &needle, &replacement)?.into_owned())
}

pub(super) fn replace_literal_matches<'a>(
    text: &'a str,
    needle: &str,
    replacement: &str,
) -> Result<Cow<'a, str>, TextError> {
    if needle.is_empty() {
        return Err(TextError::EmptySearchText);
    }
    let matches = text.matches(needle).count();
    let kept_bytes = text.len() - matches * needle.len();
    let result_bytes = kept_bytes.saturating_add(matches.saturating_mul(replacement.len()));
    if result_bytes > MAX_EXPANDED_TEXT_BYTES {
        return Err(TextError::ResultTooLarge);
    }
    if matches != 0 && replacement.contains('\0') {
        return Err(TextError::NullCharacter);
    }
    Ok(if matches == 0 {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.replace(needle, replacement))
    })
}

fn replace_regex(input: &str) -> Result<String, TextError> {
    let (pattern, replacement, text) = replacement_parts(input)?;
    let pattern = compile_regex(&pattern)?;
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;
    for captures in pattern.captures_iter(text) {
        let matched = captures.get_match();
        result.push_str(&text[last_end..matched.start()]);
        captures.expand(&replacement, &mut result);
        if result.len() > MAX_EXPANDED_TEXT_BYTES {
            return Err(TextError::ResultTooLarge);
        }
        last_end = matched.end();
    }
    if result.len().saturating_add(text.len() - last_end) > MAX_EXPANDED_TEXT_BYTES {
        return Err(TextError::ResultTooLarge);
    }
    result.push_str(&text[last_end..]);
    if result.contains('\0') {
        return Err(TextError::NullCharacter);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_complete_ansi_style_sequences() {
        for (query, expected) in [
            (
                "去ANSI样式 \x1b[31m错误\x1b[0m \u{009B}1m完成\u{009B}m",
                "错误 完成",
            ),
            (
                "strip-ansi-style \" \x1b[38;2;1;2;3m茶\x1b[0m\t\x1b[4:2m咖啡\x1b[m \"",
                " 茶\t咖啡 ",
            ),
            (
                "去ANSI样式 \x1b[2J原文\\x1b[31m\x1b[31",
                "\x1b[2J原文\\x1b[31m\x1b[31",
            ),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query:?}");
        }
    }

    #[test]
    fn converts_utf16_units_and_requires_ordered_surrogate_pairs() {
        for (input, expected) in [
            ("UTF16编码 北😀", "5317 D83D DE00"),
            ("utf16-hex-decode 0X5317d83dDe00", "北😀"),
            ("UTF16解码 5317\t0xD83D\nDE00", "北😀"),
            ("UTF16编码 “ A ”", "0020 0041 0020"),
            ("UTF16解码 \"\"", ""),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected));
        }
        for (input, expected) in [
            ("UTF16解码 D83", TextError::InvalidHexUtf16Units),
            ("UTF16解码 D83D", TextError::InvalidUtf16),
            ("UTF16解码 DE00 D83D", TextError::InvalidUtf16),
            ("UTF16解码 0000", TextError::NullCharacter),
        ] {
            assert_eq!(transform_text(input), Err(expected));
        }
    }

    #[test]
    fn makes_ascii_and_unicode_whitespace_visible_without_changing_emoji() {
        for (input, expected) in [
            ("显示空白 \" 茶\t椅\u{00A0}咖啡 \"", "␠茶␉椅<U+00A0>咖啡␠"),
            ("show-whitespace \"a\r\nb\nc\rd\"", "a␍␊b␊c␍d"),
            ("显示空白 \"\u{000B}\u{000C}👩‍💻\u{3000}\"", "␋␌👩‍💻<U+3000>"),
            ("显示空白 \"\"", ""),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn exports_each_regex_match_as_one_csv_row_with_optional_groups() {
        for (query, expected) in [
            (
                r#"正则取表 "(?<id>[0-9]+):([^;]+)" 001:茶,咖啡;002:梨"#,
                "id,2\r\n001,\"茶,咖啡\"\r\n002,梨",
            ),
            (
                r#"regex-to-csv "([0-9]+)(?:-([A-Z]))?" 007-A 08"#,
                "1,2\r\n007,A\r\n08,\"\"",
            ),
            (r#"正则取表 "(?<id>[0-9]+)" 无编号"#, "id"),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            transform_text(r#"正则取表 "[0-9]+" 123"#),
            Err(TextError::MissingRegexCaptures)
        );
        assert_eq!(
            transform_text("正则取表 \"(A.)\" A\0"),
            Err(TextError::NullCharacter)
        );
    }

    #[test]
    fn decorates_each_line_and_bounds_the_expanded_text() {
        for (query, expected) in [
            ("逐行加前缀 \"- \" \"苹果\r\n\r\n梨\"", "- 苹果\n- \n- 梨"),
            ("逐行加后缀 \"。\" 苹果\n梨", "苹果。\n梨。"),
            ("prefix-lines \"\\t\" \" a \n b \"", "\t a \n\t b "),
            ("suffix-lines \"!\" \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        let large = format!("逐行加前缀 \"{}\" {}", "x".repeat(1024), "A\n".repeat(1100));
        assert_eq!(transform_text(&large), Err(TextError::ResultTooLarge));
    }

    #[test]
    fn splits_text_on_regex_matches_and_preserves_empty_fields() {
        for (query, expected) in [
            (
                r#"正则拆分成行 "[,;\\s]+" 苹果, 梨;茶  咖啡"#,
                "苹果\n梨\n茶\n咖啡",
            ),
            (r#"regex-split-lines "[,;]" ",甲,;乙;" "#, "\n甲\n\n乙\n"),
            (r#"正则拆分成行 "\\d+" 北京"#, "北京"),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            transform_text(r#"正则拆分成行 "[" 北京"#),
            Err(TextError::InvalidRegex)
        );
    }

    #[test]
    fn unions_line_sets_and_keeps_lines_unique_to_either_group_in_order() {
        for (query, expected) in [
            (r#"行并集 "A\r\nB\nA" "B\nC\nc""#, "A\nB\nC\nc"),
            (r#"行对称差 "A\nB\nA\nX " "B\nC\nX\nC""#, "A\nX \nC\nX"),
            (r#"union-lines "" "梨\n梨""#, "梨"),
            (r#"symmetric-difference-lines "梨\n茶" "茶\n梨""#, ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn chains_text_operations_without_reinterpreting_intermediate_whitespace_or_quotes() {
        let cleanup = "依次处理 [\"逐行去空白\",\"去空行\",\"按行去重\",\"自然排序\"] \" report10 \nreport2\n\nreport2\n\"";
        assert_eq!(transform_text(cleanup).as_deref(), Ok("report2\nreport10"));
        let quoted = r#"依次处理 ["大写","JSON转义","JSON还原"] " a"b ""#;
        assert_eq!(transform_text(quoted).as_deref(), Ok(" A\"B "));
        assert_eq!(
            transform_text("依次处理 [\"去空行\",\"JSON转义\"] \" \n \"").as_deref(),
            Ok("\"\"")
        );
        assert_eq!(
            transform_text("依次处理 [] 原文"),
            Err(TextError::PipelineArguments)
        );
        assert!(matches!(
            transform_text("依次处理 [\"大写\",\"依次处理\"] 原文"),
            Err(TextError::PipelineStep { index: 2, .. })
        ));
        let large = format!(
            "依次处理 [\"utf8编码\"] {}",
            "A".repeat(MAX_EXPANDED_TEXT_BYTES / 2)
        );
        assert!(matches!(
            transform_text(&large),
            Err(TextError::PipelineStep { index: 1, .. })
        ));
    }

    #[test]
    fn slices_line_graphemes_and_preserves_each_line_ending() {
        assert_eq!(
            transform_text("逐行截取 2-3 \"A👩‍💻e\u{301}Z\r\n北\r甲乙丙丁\n\"").as_deref(),
            Ok("👩‍💻e\u{301}\r\n\r乙丙\n")
        );
        assert_eq!(
            transform_text("slice-lines 2 ABC\nXYZ").as_deref(),
            Ok("B\nY")
        );
        for range in ["0", "3-2", "1-2-3", "一"] {
            assert_eq!(
                transform_text(&format!("逐行截取 {range} 文本")),
                Err(TextError::InvalidGraphemeRange)
            );
        }
    }

    #[test]
    fn removes_one_literal_line_affix_and_preserves_line_endings() {
        for (query, expected) in [
            (
                "逐行去前缀 \"- \" \"- 甲\r\n\r\n乙\n- - 丙\n\"",
                "甲\r\n\r\n乙\n- 丙\n",
            ),
            (
                "strip-line-suffix \"。\" \"甲。\r\n乙\n丙。。\n\"",
                "甲\r\n乙\n丙。\n",
            ),
            ("逐行去后缀 \" \" \"甲 \n乙\"", "甲\n乙"),
            ("逐行去前缀 \"\" \" 甲\n\"", " 甲\n"),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn counts_and_converts_mixed_line_endings_including_terminal_blank_lines() {
        let mixed = "\" 甲\r\n乙\r\r丙\n\n\"";
        assert_eq!(
            transform_text(&format!("换行统计 {mixed}")).as_deref(),
            Ok("CRLF 1 · LF 2 · CR 2")
        );
        assert_eq!(
            transform_text(&format!("转换换行 LF {mixed}")).as_deref(),
            Ok(" 甲\n乙\n\n丙\n\n")
        );
        assert_eq!(
            transform_text(&format!("convert-line-endings crlf {mixed}")).as_deref(),
            Ok(" 甲\r\n乙\r\n\r\n丙\r\n\r\n")
        );
        assert_eq!(
            transform_text("换行统计 不换行\u{2028}另一段").as_deref(),
            Ok("CRLF 0 · LF 0 · CR 0")
        );
        assert_eq!(
            transform_text("转换换行 CR 文本"),
            Err(TextError::InvalidLineEnding)
        );
    }

    #[test]
    fn strips_only_matching_common_indentation_and_keeps_line_endings() {
        for (query, expected) in [
            (
                "去公共缩进 \"    甲\r\n      乙\r\n\r\n    丙\n\"",
                "甲\r\n  乙\r\n\r\n丙\n",
            ),
            ("dedent \"\t  甲\n\t  \t乙\"", "甲\n\t乙"),
            ("dedent \"   甲\n\t乙\"", "   甲\n\t乙"),
            ("dedent \" \n\t\"", " \n\t"),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn reports_line_insertions_and_deletions_with_repetitions_and_blank_lines() {
        for (query, expected) in [
            (
                r#"逐行比较 "苹果\n梨\n茶" "苹果\n柿子\n茶""#,
                "  苹果\n- 梨\n+ 柿子\n  茶",
            ),
            (r#"diff-lines "a\n\na\n" "a\r\na""#, "  a\n- \n  a"),
            (r#"diff-lines "" " a ""#, "+  a "),
            (r#"diff-lines "b\na" "a\nb""#, "- b\n  a\n+ b"),
            (r#"diff-lines "甲\r\n乙\r\n" "甲\n乙""#, "两组文本各行相同"),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected));
        }
        let oversize = format!("diff-lines \"{}\" \"\"", "a\\n".repeat(MAX_DIFF_LINES + 1));
        assert_eq!(transform_text(&oversize), Err(TextError::TooManyDiffLines));
        assert_eq!(
            transform_text(r#"逐行比较 "一" 二"#),
            Err(TextError::InvalidLineComparison)
        );
    }

    #[test]
    fn compares_two_quoted_line_lists_and_preserves_first_list_order() {
        for (input, expected) in [
            (
                r#"共有行 "苹果\n梨\n苹果\n茶" "茶\r\n苹果\r\n柿子""#,
                "苹果\n茶",
            ),
            (r#"行差集 "苹果\n梨\n苹果\n茶" "茶\n苹果\n柿子""#, "梨"),
            ("行差集 “A\r\n A\r\nB \r\nA” “A\nB”", " A\nB "),
            (r#"intersect-lines "A\nB" "a""#, ""),
            (r#"subtract-lines "A\nB\nA" """#, "A\nB"),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, error) in [
            ("共有行 \"苹果\"", TextError::IncompleteLineComparison),
            ("共有行 \"苹果\" \"梨", TextError::IncompleteLineComparison),
            ("共有行 \"苹果\" 梨", TextError::InvalidLineComparison),
            (
                "行差集 \"苹果\" \"梨\" \"茶\"",
                TextError::InvalidLineComparison,
            ),
            (r#"行差集 "苹果\u0000" "梨""#, TextError::NullCharacter),
        ] {
            assert_eq!(transform_text(input), Err(error), "{input}");
        }
    }

    #[test]
    fn picks_one_based_line_ranges_in_source_order_without_repeats() {
        assert_eq!(
            transform_text("取行 4-5,2-4,2 顶行\r\n 二 \r\n\r\n四\r\n五\r\n末行").as_deref(),
            Ok(" 二 \n\n四\n五")
        );
        assert_eq!(transform_text("pick-lines 1 A\n").as_deref(), Ok("A"));
        assert_eq!(
            transform_text("删行 2,4-5,4 \" A\r\n标题\r\n\r\n四\r\n五\r\n末行 \"").as_deref(),
            Ok(" A\n\n末行 ")
        );
        assert_eq!(transform_text("drop-lines 1-2 A\r\nB").as_deref(), Ok(""));
        for (query, error) in [
            ("取行 0 A", TextError::InvalidLineRange),
            ("取行 2-1 A\nB", TextError::InvalidLineRange),
            ("取行 1, A", TextError::InvalidLineRange),
            ("取行 2 A\n", TextError::MissingLine),
            ("取行 1-9 A\nB", TextError::MissingLine),
            ("删行 2 A", TextError::MissingLine),
        ] {
            assert_eq!(transform_text(query), Err(error), "{query}");
        }
    }

    #[test]
    fn selects_a_one_based_column_in_each_line_including_empty_fields() {
        for (input, expected) in [
            ("取列 \"\\t\" 2 苹果\t42\r\n梨\t7", "42\n7"),
            ("column \"||\" 2 A||||B\nC||D||E", "\nD"),
            ("取列 \",\" 2 \"A, 示例 \"", " 示例 "),
            ("取列 \",\" 3 \"A,B,\nC,D,尾巴\"", "\n尾巴"),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, expected) in [
            ("取列 \",\" 0 A,B", TextError::InvalidColumn),
            ("取列 \",\" 2 A,B\nC", TextError::MissingColumn),
            ("取列 \"\" 1 A,B", TextError::EmptyDelimiter),
        ] {
            assert_eq!(transform_text(input), Err(expected), "{input}");
        }
    }

    #[test]
    fn replaces_each_regex_match_with_expanded_captures_and_bounds_the_result() {
        for (query, expected) in [
            (
                r#"正则替换 "([0-9]{4})/([0-9]{2})/([0-9]{2})" "$1-$2-$3" 2026/11/21、2027/01/02"#,
                "2026-11-21、2027-01-02",
            ),
            (
                r#"regex-replace "(?<name>[A-Z]+)=([0-9]+)" "$${${name}: $2}" A=15"#,
                "${A: 15}",
            ),
            ("正则替换 “\\d+” \"\" 北京42上海", "北京上海"),
            (r#"正则替换 "a" "b" 北京"#, "北京"),
            (r#"正则替换 "x?" "!" ab"#, "!a!b!"),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (r#"正则替换 "[" "x" A"#, TextError::InvalidRegex),
            (r#"正则替换 "" "x" A"#, TextError::EmptySearchText),
            (r#"正则替换 "A" "x""#, TextError::IncompleteReplacement),
            (r#"正则替换 "A" "\u0000" A"#, TextError::NullCharacter),
        ] {
            assert_eq!(transform_text(query), Err(error), "{query}");
        }
        let query = format!(
            "正则替换 \"a\" \"{}\" {}",
            "x".repeat(1500),
            "a".repeat(1500)
        );
        assert_eq!(transform_text(&query), Err(TextError::ResultTooLarge));
    }

    #[test]
    fn converts_identifier_styles_with_acronyms_digits_and_combining_marks() {
        for (input, expected) in [
            ("下划线命名 HTTPServer2URL", "http_server2_url"),
            ("snake-case XML-HTTP Request", "xml_http_request"),
            ("小驼峰命名 --HTTP_server-2_URL", "httpServer2Url"),
            ("camel-case user__HTTP_server", "userHttpServer"),
            ("下划线命名 Cafe\u{301}Name", "cafe\u{301}_name"),
            ("下划线命名 用户 ID", "用户_id"),
            ("小驼峰命名 --_--", ""),
            ("短横线命名 HTTPServer2URL", "http-server2-url"),
            ("kebab-case 用户_ID", "用户-id"),
            ("大驼峰命名 http-server2-url", "HttpServer2Url"),
            ("pascal-case cafe\u{301}_name", "Cafe\u{301}Name"),
            ("大驼峰命名 --_--", ""),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn escapes_regex_metacharacters_and_counts_matches_across_lines() {
        assert_eq!(
            transform_text("正则转义 a+b[0]").as_deref(),
            Ok(r"a\+b\[0\]")
        );
        assert_eq!(
            transform_text("正则计数 \"[0-9]+\" A12 B34\nC007").as_deref(),
            Ok("3 处匹配")
        );
        assert_eq!(
            transform_text(r#"regex-count "aa" aaa"#).as_deref(),
            Ok("1 处匹配")
        );
        assert_eq!(
            transform_text(r#"正则计数 "[0-9]+" 没有数字"#).as_deref(),
            Ok("0 处匹配")
        );
    }

    #[test]
    fn extracts_all_nonempty_regex_matches_across_the_input() {
        for (query, expected) in [
            (
                "正则提取全部 “\\d+” 文件 42 大小 12\r\n无编号\r\n文件 007",
                "42\n12\n007",
            ),
            (r#"regex-extract-all "0*" A00B"#, "00"),
            ("regex-extract-all \"(?s)甲.*?乙\" A甲\n乙B", "甲\n乙"),
            (r#"正则提取全部 "[0-9]+" 没有数字"#, ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn extracts_a_numbered_or_named_regex_group_in_match_order() {
        for (query, expected) in [
            (r#"正则取组 id "(?<id>[0-9]+)[A-Z]" 001A 002B"#, "001\n002"),
            (r#"regex-capture 2 "([A-Z])([0-9]+)?" A12 B C03"#, "12\n03"),
            (r#"regex-capture 1 "A([0-9]*)" A A7"#, "7"),
            (r#"regex-capture 0 "[A-Z][0-9]+" A7 B03"#, "A7\nB03"),
            (
                "正则取组 1 “文件\\s+([0-9]+)” 文件\r\n004 文件 005",
                "004\n005",
            ),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (
                r#"正则取组 无 "(?<id>[0-9]+)" 7"#,
                TextError::UnknownRegexGroup,
            ),
            (r#"正则取组 9 "([0-9]+)" 7"#, TextError::UnknownRegexGroup),
            (r#"正则取组 1 "([0-9]+)""#, TextError::IncompleteRegexGroup),
            (r#"正则取组 1 "[" 7"#, TextError::InvalidRegex),
        ] {
            assert_eq!(transform_text(query), Err(error), "{query}");
        }
    }

    #[test]
    fn extracts_the_first_regex_match_from_each_line_in_order() {
        assert_eq!(
            transform_text("正则提取 “\\d+” 文件 42 大小 12\r\n无编号\r\n文件 007").as_deref(),
            Ok("42\n007")
        );
        assert_eq!(
            transform_text("regex-extract \"[A-Z]+\" aABCdDEF\n2\nDEF").as_deref(),
            Ok("ABC\nDEF")
        );
        assert_eq!(
            transform_text("正则提取 \"[\" 测试"),
            Err(TextError::InvalidRegex)
        );
    }

    #[test]
    fn trims_each_line_and_keeps_internal_spaces_and_blank_rows() {
        for (input, expected) in [
            (
                "逐行去空白 \"\t苹果  \r\n \r\n\u{3000}梨\t  \"",
                "苹果\n\n梨",
            ),
            ("trim-lines \" a  b\n  c\t d \"", "a  b\nc\t d"),
            ("逐行去空白 \"\"", ""),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn filters_individual_lines_with_a_quoted_regular_expression() {
        for (input, expected) in [
            ("正则保留行 “^\\d+$” 42\r\nA3\r\n007", "42\n007"),
            ("正则排除行 \"^[0-9]+$\" 42\nA3\n007\nA3", "A3\nA3"),
            (
                "keep-regex-lines \"^\\\\p{Han}+$\" 北京\nOslo\n上海",
                "北京\n上海",
            ),
            ("exclude-regex-lines \"(?i)^ok$\" ok\n错误\nOK", "错误"),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, error) in [
            ("正则保留行 \"[\" A", TextError::InvalidRegex),
            ("正则保留行 \"\" A", TextError::EmptySearchText),
            ("正则排除行 “^\\d+", TextError::IncompleteTextArgument),
            ("正则保留行 \"[0-9]{1000000}\" A1", TextError::InvalidRegex),
        ] {
            assert_eq!(transform_text(input), Err(error), "{input}");
        }
    }

    #[test]
    fn converts_ascii_width_including_space_and_preserves_other_characters() {
        for (input, expected) in [
            ("转半角 ＡＢＣ１２３！\u{3000}北京。", "ABC123! 北京。"),
            ("转全角 ABC123! 北京。", "ＡＢＣ１２３！\u{3000}北京。"),
            ("halfwidth \"\u{3000}！～ｶ−\t\n\u{3000}\"", " !~ｶ−\t\n "),
            ("fullwidth \" !~ｶ−\t\n \"", "\u{3000}！～ｶ−\t\n\u{3000}"),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn sorts_signed_rational_lines_without_rounding_or_rewriting_equal_values() {
        let numbers = "2e0\r\n-1/2\r\n1.50\r\n3/2\r\n1.499999999999999999";
        assert_eq!(
            transform_text(&format!("数值排序 {numbers}")).as_deref(),
            Ok("-1/2\n1.499999999999999999\n1.50\n3/2\n2e0")
        );
        assert_eq!(
            transform_text(&format!("sort-numeric-desc {numbers}")).as_deref(),
            Ok("2e0\n1.50\n3/2\n1.499999999999999999\n-1/2")
        );
        assert_eq!(
            transform_text("数值排序 \" 2 \n 1 \"").as_deref(),
            Ok(" 1 \n 2 ")
        );
        assert_eq!(
            transform_text("数值排序 2\n未知"),
            Err(TextError::InvalidNumberLine)
        );
    }

    #[test]
    fn sorts_digit_runs_by_value_and_keeps_equivalent_lines_in_input_order() {
        assert_eq!(
            transform_text("自然排序 照片10\r\n照片02\r\n照片1\r\n照片2").as_deref(),
            Ok("照片1\n照片02\n照片2\n照片10")
        );
        assert_eq!(
            transform_text("natural-sort p2-q10\np02-q2\np2-q02\np1").as_deref(),
            Ok("p1\np02-q2\np2-q02\np2-q10")
        );
        assert_eq!(
            transform_text("自然排序 p1000000000000000000000000000000000000000\np999999999999999999999999999999999999999\np0\np00").as_deref(),
            Ok("p0\np00\np999999999999999999999999999999999999999\np1000000000000000000000000000000000000000")
        );
        assert_eq!(transform_text("按行排序 p2\np10").as_deref(), Ok("p10\np2"));
    }

    #[test]
    fn round_trips_utf8_bytes_in_hex_and_validates_decoded_text() {
        for (query, expected) in [
            ("UTF8编码 北京", "E5 8C 97 E4 BA AC"),
            ("UTF8编码 “ A ”", "20 41 20"),
            ("UTF8解码 e58c97 E4BAAC", "北京"),
            ("utf8-hex-decode 0xf0 0X9F\r\n98 80", "😀"),
            ("UTF8解码 20 41 20", " A "),
            ("UTF8编码 \"\"", ""),
            ("UTF8解码 \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("UTF8解码 4 1", TextError::InvalidHexBytes),
            ("UTF8解码 0x", TextError::InvalidHexBytes),
            ("UTF8解码 +A", TextError::InvalidHexBytes),
            ("UTF8解码 FF", TextError::InvalidUtf8),
            ("UTF8解码 E5 8C", TextError::InvalidUtf8),
            ("UTF8解码 00", TextError::NullCharacter),
        ] {
            assert_eq!(transform_text(query), Err(error), "{query}");
        }
    }

    #[test]
    fn sorts_lines_by_grapheme_count_and_preserves_input_order_for_equal_lengths() {
        assert_eq!(
            transform_text("按长度排序 医生\r\ne\u{0301}\n👩‍💻\nABC").as_deref(),
            Ok("e\u{0301}\n👩‍💻\n医生\nABC")
        );
        assert_eq!(
            transform_text("sort-line-length-desc \"医生\ne\u{0301}\n\n👩‍💻\nABC\"").as_deref(),
            Ok("ABC\n医生\ne\u{0301}\n👩‍💻\n")
        );
    }

    #[test]
    fn edits_pasted_lines_without_changing_text_on_each_line() {
        for (query, expected) in [
            ("按行去重 菠萝\r\n苹果\n菠萝\n苹果 ", "菠萝\n苹果"),
            ("按行排序 B\r\nA\nb\nA", "A\nA\nB\nb"),
            ("按行倒序 \" 甲\r\n\r\n乙 \n丙\"", "丙\n乙 \n\n 甲"),
            ("reverse-lines \"\"", ""),
            ("去空行 \" 甲\r\n  \t\r\n乙 \n\n丙\"", " 甲\n乙 \n丙"),
            ("按行去重 \"A\nA \nA\"", "A\nA "),
            ("去空行 \"\n\t\n \"", ""),
            (
                "合并空行 \" 甲\r\n\r\n \t\r乙 \n\n\n丙\"",
                " 甲\r\n\r\n乙 \n\n丙",
            ),
            (
                "collapse-empty-lines \"\t\r\n\n甲\r\r\n\n\"",
                "\t\r\n甲\r\r\n",
            ),
            ("合并空行 \"甲\n \t\n \t\"", "甲\n \t\n"),
            ("合并空行 \"\"", ""),
            ("按行编号 \" 甲\r\n\r\n乙 \"", "1\t 甲\n2\t\n3\t乙 "),
            ("number-lines \"A\n\"", "1\tA"),
            (
                "按行计数 梨\n苹果\n梨\n苹果\n苹果\n桃",
                "3\t苹果\n2\t梨\n1\t桃",
            ),
            (
                "line-counts \"B\r\nA\nB\nA\nA \n\n\"",
                "2\tB\n2\tA\n1\tA \n1\t",
            ),
            ("按行计数 \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn base64_handles_utf8_both_alphabets_and_wrapped_input() {
        for (input, expected) in [
            ("Base64编码 北京", "5YyX5Lqs"),
            ("Base64解码 5YyX\r\n5Lqs", "北京"),
            ("Base64编码 😀", "8J+YgA=="),
            ("Base64解码 8J+YgA", "😀"),
            ("Base64URL编码 😀", "8J-YgA"),
            ("Base64URL解码 8J-YgA==", "😀"),
            (r#"base64-encode " a ""#, "IGEg"),
            (r#"base64-decode """#, ""),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
        for input in [
            "Base64解码 8J-YgA",
            "Base64URL解码 8J+YgA",
            "Base64解码 AB==",
            "Base64解码 AA=",
        ] {
            assert_eq!(
                transform_text(input),
                Err(TextError::InvalidBase64),
                "{input}"
            );
        }
        assert_eq!(
            transform_text("Base64解码 /w=="),
            Err(TextError::InvalidUtf8)
        );
        assert_eq!(
            transform_text("Base64解码 AA=="),
            Err(TextError::NullCharacter)
        );
    }

    #[test]
    fn quotes_and_decodes_json_strings_with_unicode_and_control_characters() {
        for (query, expected) in [
            (r#"JSON转义 他写下 "你好""#, r#""他写下 \"你好\"""#),
            ("JSON转义 “ 甲\r\n乙\t ”", r#"" 甲\r\n乙\t ""#),
            (r#"JSON还原 "\u4F60\n好""#, "你\n好"),
            (r#"json-unquote "He said \"hi\"""#, r#"He said "hi""#),
            ("json-quote \"\"", "\"\""),
            ("JSON还原 \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "JSON还原 123",
            r#"JSON还原 ["你好"]"#,
            r#"JSON还原 "\uD800""#,
            r#"JSON还原 "A" "B""#,
        ] {
            assert_eq!(
                transform_text(query),
                Err(TextError::InvalidJsonString),
                "{query}"
            );
        }
        assert_eq!(
            transform_text(r#"JSON还原 "\u0000""#),
            Err(TextError::NullCharacter)
        );
    }

    #[test]
    fn escapes_commonmark_punctuation_and_restores_only_ascii_escapes() {
        assert_eq!(
            transform_text("Markdown转义 *茶* [咖啡]").as_deref(),
            Ok(r"\*茶\* \[咖啡\]")
        );
        assert_eq!(
            transform_text(r"Markdown还原 \*茶\* \[咖啡\]").as_deref(),
            Ok("*茶* [咖啡]")
        );
        assert_eq!(
            transform_text(r"markdown-unescape \q \中 \? \\!").as_deref(),
            Ok(r"\q \中 ? \!")
        );
        let punctuation: String = (b' '..=b'~')
            .filter(u8::is_ascii_punctuation)
            .map(char::from)
            .collect();
        let source = format!("茶 {punctuation}\n咖啡");
        assert_eq!(unescape_markdown(&escape_markdown(&source)), source);
        assert_eq!(unescape_markdown("\\\n"), "\\\n");
    }

    #[test]
    fn escapes_html_characters_and_decodes_supported_references_once() {
        for (query, expected) in [
            (
                r#"HTML转义 <b title="甲">茶 & '咖啡'</b>"#,
                "&lt;b title=&quot;甲&quot;&gt;茶 &amp; &#39;咖啡&#39;&lt;/b&gt;",
            ),
            (
                "HTML还原 &lt;b&gt;茶 &amp; 咖啡&#X1F600;&lt;/b&gt;",
                "<b>茶 & 咖啡😀</b>",
            ),
            (
                "html-unescape &AMP;&apos;&nbsp;&#128;&#xD800;&#0;&#999999999999;",
                "&'\u{a0}€���",
            ),
            (
                "HTML还原 &amp;lt; &unknown; &#x; &#-1; &lt&GT;",
                "&lt; &unknown; &#x; &#-1; &lt>",
            ),
            ("html-escape “ A & B ”", " A &amp; B "),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn converts_unicode_code_points_without_combining_or_losing_scalars() {
        for (query, expected) in [
            ("码位 你👩‍💻", "U+4F60 U+1F469 U+200D U+1F4BB"),
            ("码位 “ e\u{301} ”", "U+0020 U+0065 U+0301 U+0020"),
            ("码位解码 u+4f60 U+1F469\nU+200D U+1F4BB", "你👩‍💻"),
            ("codepoints-decode U+000A U+9", "\n\t"),
            ("codepoints \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for code in ["U+", "U++0041", "U+D800", "U+110000", "U+0000041", "0041"] {
            assert_eq!(
                transform_text(&format!("码位解码 {code}")),
                Err(TextError::InvalidCodePoint),
                "{code}"
            );
        }
        assert_eq!(
            transform_text("码位解码 U+0061 U+0000"),
            Err(TextError::NullCharacter)
        );
    }

    #[test]
    fn distinguishes_grapheme_scalar_utf16_and_utf8_lengths() {
        assert_eq!(
            transform_text("长度详情 \"👩‍💻 e\u{0301}\"").as_deref(),
            Ok("3 字素簇 · 6 Unicode 码位\n8 UTF-16 单元 · 15 UTF-8 字节")
        );
        assert_eq!(
            transform_text("length-details \"\r\n\"").as_deref(),
            Ok("1 字素簇 · 2 Unicode 码位\n2 UTF-16 单元 · 2 UTF-8 字节")
        );
        assert_eq!(
            transform_text("长度详情 \"\"").as_deref(),
            Ok("0 字素簇 · 0 Unicode 码位\n0 UTF-16 单元 · 0 UTF-8 字节")
        );
    }

    #[test]
    fn lists_unicode_word_counts_in_stable_frequency_order() {
        assert_eq!(
            transform_text("词频 tea, coffee. tea Tea").as_deref(),
            Ok("词语,次数\r\ntea,2\r\ncoffee,1\r\nTea,1")
        );
        assert_eq!(
            transform_text("word-frequency e\u{0301} e\u{0301} CAFÉ").as_deref(),
            Ok("词语,次数\r\ne\u{0301},2\r\nCAFÉ,1")
        );
        assert_eq!(
            transform_text("词频 ... -- !!!").as_deref(),
            Ok("词语,次数")
        );
    }

    #[test]
    fn reports_frequencies_of_complete_non_whitespace_graphemes_as_csv() {
        assert_eq!(
            transform_text("字频 \"茶👩‍💻茶👩‍💻,e\u{0301}　a\"").as_deref(),
            Ok("字符,次数\r\n茶,2\r\n👩‍💻,2\r\n\",\",1\r\ne\u{0301},1\r\na,1")
        );
        assert_eq!(transform_text("字频 \" \t \"").as_deref(), Ok("字符,次数"));
    }

    #[test]
    fn counts_graphemes_and_utf8_bytes_including_quoted_spaces() {
        for (input, expected) in [
            ("你好👩‍💻", "3 字符 · 17 UTF-8 字节"),
            ("e\u{301}", "1 字符 · 3 UTF-8 字节"),
            ("\" a \"", "3 字符 · 3 UTF-8 字节"),
            ("“ 你 ”", "3 字符 · 5 UTF-8 字节"),
            ("\"\"", "0 字符 · 0 UTF-8 字节"),
        ] {
            assert_eq!(count_text(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(count_text(""), Err(TextError::MissingText));
        assert_eq!(count_text("“你好"), Err(TextError::UnclosedQuote));
    }

    #[test]
    fn replaces_literal_text_with_quoted_operands_and_reverses_graphemes() {
        for (input, expected) in [
            (r#"替换 "A B" "X" A B | A B"#, "X | X"),
            (r#"replace "," "\n" a,b,c"#, "a\nb\nc"),
            (r#"替换 "." "" a.b.c"#, "abc"),
            ("替换 “甲” “乙” “ 甲 甲 ”", " 乙 乙 "),
            ("倒序 A👩‍💻中", "中👩‍💻A"),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, expected) in [
            (r#"替换 "a""b" a"#, TextError::InvalidReplacementArguments),
            (r#"替换 "a" "#, TextError::IncompleteReplacement),
            (r#"替换 "" "x" a"#, TextError::EmptySearchText),
            (r#"替换 "a" "\u0000" a"#, TextError::NullCharacter),
        ] {
            assert_eq!(transform_text(input), Err(expected), "{input}");
        }
        let large = format!(r#"替换 "x" "{}" {}"#, "Y".repeat(1500), "x".repeat(1500));
        assert_eq!(transform_text(&large), Err(TextError::ResultTooLarge));
    }

    #[test]
    fn groups_lines_into_fixed_width_csv_records_and_pads_the_final_record() {
        for (query, expected) in [
            ("按行分组CSV 3 甲\n乙\n丙\n丁", "甲,乙,丙\r\n丁,\"\",\"\""),
            (
                "chunk-lines-csv 2 a,b\r\n\r\n\"c\"",
                "\"a,b\",\"\"\r\n\"\"\"c\"\"\",\"\"",
            ),
            ("按行分组CSV 1 甲\r\n乙\n", "甲\r\n乙"),
            ("按行分组CSV 2 \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for width in ["0", "129", "2.5"] {
            assert_eq!(
                transform_text(&format!("按行分组CSV {width} 甲")),
                Err(TextError::CsvColumnCount)
            );
        }
    }

    #[test]
    fn pairs_text_lines_as_csv_with_empty_cells_and_escaped_quotes() {
        assert_eq!(
            transform_text(r#"逐行配对 "a,b\n\"quote\"" "甲\r\n乙\r\n丙""#).as_deref(),
            Ok("\"a,b\",甲\r\n\"\"\"quote\"\"\",乙\r\n\"\",丙")
        );
        assert_eq!(
            transform_text(r#"zip-lines "A\n\nB" "1\n2""#).as_deref(),
            Ok("A,1\r\n\"\",2\r\nB,\"\"")
        );
        assert_eq!(transform_text(r#"逐行配对 "" """#).as_deref(), Ok(""));
        assert_eq!(
            transform_text(r#"逐行配对 "A" "B" "C""#),
            Err(TextError::InvalidLineComparison)
        );
        let long = "a".repeat(MAX_EXPANDED_TEXT_BYTES / 2);
        assert_eq!(
            transform_text(&format!("逐行配对 \"{long}\" \"{long}\"")),
            Err(TextError::ResultTooLarge)
        );
    }

    #[test]
    fn joins_and_splits_lines_with_literal_quoted_delimiters() {
        for (query, expected) in [
            ("按行拼接 \", \" 苹果\r\n梨", "苹果, 梨"),
            ("按行拼接 “” 甲\n乙", "甲乙"),
            ("join-lines \"\\t\" \" 甲\n\n乙 \"", " 甲\t\t乙 "),
            ("拆分成行 “、” 苹果、梨", "苹果\n梨"),
            ("split-lines \"::\" ::A::::B::", "\nA\n\nB\n"),
            ("拆分成行 \",\" \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (r#"拆分成行 "" A"#, TextError::EmptyDelimiter),
            (r#"拆分成行 ",""#, TextError::IncompleteTextArgument),
            (r#"按行拼接 ","A"#, TextError::InvalidTextArgument),
            (r#"按行拼接 "\u0000" A"#, TextError::NullCharacter),
        ] {
            assert_eq!(transform_text(query), Err(error), "{query}");
        }
        let large = format!(r#"按行拼接 "{}" {}"#, ",".repeat(1500), "x\n".repeat(1000));
        assert_eq!(transform_text(&large), Err(TextError::ResultTooLarge));
    }

    #[test]
    fn includes_or_excludes_literal_matching_lines_in_input_order() {
        for (query, expected) in [
            ("保留行 \"错误\" 成功\r\n错误 A\n错误 B", "错误 A\n错误 B"),
            ("排除行 “错误” 成功 A\r\n错误 B\n成功 C", "成功 A\n成功 C"),
            ("keep-lines \"b.\" beta\nb.\nB.", "b."),
            ("exclude-lines \"x\" \" 甲\nx\n乙 \"", " 甲\n乙 "),
            ("keep-lines \"x\" abc", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            transform_text(r#"保留行 "" 内容"#),
            Err(TextError::EmptySearchText)
        );
    }

    #[test]
    fn hashes_the_exact_utf8_text() {
        for (input, expected) in [
            (
                "SHA-256 北京",
                "68c77e155132565e1cbbace574552fedbb9e769c147d0e9e5ed64701f0cdf82d",
            ),
            (
                "sha512 abc",
                concat!(
                    "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a",
                    "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
                ),
            ),
            (
                "SHA256 \"\"",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn changes_unicode_case_and_collapses_whitespace() {
        for (input, expected) in [
            ("大写 Straße", "STRASSE"),
            ("LOWER “ ÉTÉ ”", " été "),
            ("小写 你好ABC", "你好abc"),
            ("合并空白 A　 B\tC", "A B C"),
            ("collapse \"  A   B  \"", "A B"),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, error) in [
            ("大写", TextError::IncompleteTransform),
            ("小写 “ABC", TextError::UnclosedQuote),
            ("缩写 ABC", TextError::UnknownOperation),
        ] {
            assert_eq!(transform_text(input), Err(error), "{input}");
        }
    }
}
