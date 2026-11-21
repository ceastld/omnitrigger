use super::calculator::normalize;

const DIGITS: [char; 10] = ['零', '壹', '贰', '叁', '肆', '伍', '陆', '柒', '捌', '玖'];
const MAX_YUAN: u64 = 999_999_999_999;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RenminbiError {
    Incomplete,
    InvalidAmount,
    ExcessPrecision,
    TooLarge,
}

impl RenminbiError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入要转换的人民币金额，示例 1409.50",
            Self::InvalidAmount => "请输入非负的人民币金额，示例 1,409.50",
            Self::ExcessPrecision => "金额最多保留两位小数",
            Self::TooLarge => "金额不得超过 999,999,999,999.99 元",
        }
    }
}

pub(super) fn uppercase_renminbi(input: &str) -> Result<String, RenminbiError> {
    let normalized = normalize(input);
    let input = normalized.trim();
    let amount = input.strip_prefix(['¥', '￥']).unwrap_or(input).trim();
    let amount = amount.strip_suffix('元').unwrap_or(amount).trim_end();
    if amount.is_empty() {
        return Err(RenminbiError::Incomplete);
    }

    let (whole, fraction) = if let Some((whole, fraction)) = amount.split_once('.') {
        if fraction.is_empty() {
            return Err(RenminbiError::Incomplete);
        }
        (whole, fraction)
    } else {
        (amount, "")
    };
    let yuan = parse_yuan(whole)?;
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RenminbiError::InvalidAmount);
    }
    let (jiao, fen) = match fraction.as_bytes() {
        [] => (0, 0),
        [jiao] => (usize::from(jiao - b'0'), 0),
        [jiao, fen] => (usize::from(jiao - b'0'), usize::from(fen - b'0')),
        _ => return Err(RenminbiError::ExcessPrecision),
    };

    let mut result = String::from("人民币");
    write_yuan(&mut result, yuan);
    result.push('元');
    if jiao > 0 {
        result.push(DIGITS[jiao]);
        result.push('角');
    }
    if fen > 0 {
        if jiao == 0 {
            result.push('零');
        }
        result.push(DIGITS[fen]);
        result.push('分');
    } else {
        result.push('整');
    }
    Ok(result)
}

fn parse_yuan(whole: &str) -> Result<u64, RenminbiError> {
    let mut groups = whole.split(',').peekable();
    let first = groups.next().unwrap_or_default();
    if first.is_empty() || (groups.peek().is_some() && first.len() > 3) {
        return Err(RenminbiError::InvalidAmount);
    }

    let mut yuan = 0_u64;
    for (index, group) in std::iter::once(first).chain(groups).enumerate() {
        if (index > 0 && group.len() != 3) || !group.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(RenminbiError::InvalidAmount);
        }
        for byte in group.bytes() {
            yuan = yuan * 10 + u64::from(byte - b'0');
            if yuan > MAX_YUAN {
                return Err(RenminbiError::TooLarge);
            }
        }
    }
    Ok(yuan)
}

fn write_yuan(result: &mut String, yuan: u64) {
    if yuan == 0 {
        result.push('零');
        return;
    }

    let mut started = false;
    let mut skipped_zero = false;
    for (divisor, unit) in [(100_000_000, "亿"), (10_000, "万"), (1, "")] {
        let group = (yuan / divisor % 10_000) as usize;
        if group == 0 {
            skipped_zero |= started;
            continue;
        }
        if started && (skipped_zero || group < 1000) {
            result.push('零');
        }
        write_group(result, group);
        result.push_str(unit);
        started = true;
        skipped_zero = group.is_multiple_of(10);
    }
}

fn write_group(result: &mut String, group: usize) {
    let mut started = false;
    let mut skipped_digit = false;
    for (divisor, unit) in [(1000, "仟"), (100, "佰"), (10, "拾"), (1, "")] {
        let digit = group / divisor % 10;
        if digit == 0 {
            skipped_digit |= started;
            continue;
        }
        if skipped_digit {
            result.push('零');
        }
        result.push(DIGITS[digit]);
        result.push_str(unit);
        started = true;
        skipped_digit = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_exact_renminbi_amounts() {
        let cases = [
            ("0", "人民币零元整"),
            ("0.05", "人民币零元零伍分"),
            ("10", "人民币壹拾元整"),
            ("￥1,409.50", "人民币壹仟肆佰零玖元伍角整"),
            ("６，００７．１４ 元", "人民币陆仟零柒元壹角肆分"),
            ("1,680.32", "人民币壹仟陆佰捌拾元叁角贰分"),
            ("107,000.53", "人民币壹拾万零柒仟元伍角叁分"),
            ("¥16,409.02", "人民币壹万陆仟肆佰零玖元零贰分"),
            ("100000001", "人民币壹亿零壹元整"),
            ("100010001.9", "人民币壹亿零壹万零壹元玖角整"),
            ("100001000", "人民币壹亿零壹仟元整"),
            ("100100000", "人民币壹亿零壹拾万元整"),
            ("10001000", "人民币壹仟万零壹仟元整"),
            (
                "999,999,999,999.99",
                "人民币玖仟玖佰玖拾玖亿玖仟玖佰玖拾玖万玖仟玖佰玖拾玖元玖角玖分",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                uppercase_renminbi(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_malformed_or_out_of_range_renminbi_amounts() {
        for (input, expected) in [
            ("¥", RenminbiError::Incomplete),
            ("12.", RenminbiError::Incomplete),
            ("-1", RenminbiError::InvalidAmount),
            ("1e2", RenminbiError::InvalidAmount),
            ("1.2.3", RenminbiError::InvalidAmount),
            ("1,00", RenminbiError::InvalidAmount),
            ("1.002", RenminbiError::ExcessPrecision),
            ("1000000000000", RenminbiError::TooLarge),
            ("9999999999999999999999999", RenminbiError::TooLarge),
        ] {
            assert_eq!(uppercase_renminbi(input), Err(expected), "{input}");
        }
    }
}
