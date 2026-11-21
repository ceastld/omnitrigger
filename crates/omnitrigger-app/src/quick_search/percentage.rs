/// 返回基数和百分数，保留两边的算式以便用同一套规则计算。
pub(super) fn percentage_operands(input: &str) -> Option<(&str, &str)> {
    if let Some((base, rate)) = input.rsplit_once('的') {
        return Some((base.trim(), rate.trim().strip_suffix('%')?.trim_end()));
    }

    let (rate, tail) = input.rsplit_once('%')?;
    let mut tail = tail.trim_start().splitn(2, char::is_whitespace);
    if !tail.next()?.eq_ignore_ascii_case("of") {
        return None;
    }
    Some((tail.next().unwrap_or_default().trim(), rate.trim_end()))
}
