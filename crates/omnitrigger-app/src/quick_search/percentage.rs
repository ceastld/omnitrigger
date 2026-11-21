#[derive(Clone, Copy)]
pub(super) enum Percentage {
    Portion,
    Increase,
    Decrease,
    Discount,
}

impl Percentage {
    pub(super) fn apply(self, base: f64, rate: f64) -> f64 {
        let fraction = rate
            / if matches!(self, Self::Discount) {
                10.
            } else {
                100.
            };
        match self {
            Self::Portion | Self::Discount => base * fraction,
            Self::Increase => base * (1. + fraction),
            Self::Decrease => base * (1. - fraction),
        }
    }
}

/// 保留基数与比例两边的算式，使用统一的计算规则。
pub(super) fn percentage_operands(input: &str) -> Option<(Percentage, &str, &str)> {
    if !input.contains(['%', '折']) {
        return None;
    }
    let lower = input.to_ascii_lowercase();
    let separator = [
        ("的", Percentage::Portion),
        ("增加", Percentage::Increase),
        ("减少", Percentage::Decrease),
        ("打", Percentage::Discount),
        (" increased by ", Percentage::Increase),
        (" decreased by ", Percentage::Decrease),
    ]
    .into_iter()
    .filter_map(|(separator, mode)| {
        lower
            .rfind(separator)
            .map(|start| (start, start + separator.len(), mode))
    })
    .max_by_key(|(start, _, _)| *start);
    if let Some((start, end, mode)) = separator {
        let suffix = if matches!(mode, Percentage::Discount) {
            '折'
        } else {
            '%'
        };
        return Some((
            mode,
            input[..start].trim(),
            input[end..].trim().strip_suffix(suffix)?.trim_end(),
        ));
    }

    let (rate, tail) = input.rsplit_once('%')?;
    let mut tail = tail.trim_start().splitn(2, char::is_whitespace);
    if !tail.next()?.eq_ignore_ascii_case("of") {
        return None;
    }
    Some((
        Percentage::Portion,
        tail.next().unwrap_or_default().trim(),
        rate.trim_end(),
    ))
}
