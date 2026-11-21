use super::calculator::CalculationError;

pub(super) fn split_top_level_commas(input: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0usize;
    input.split(move |ch| {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth == 0 && ch == ','
    })
}

#[derive(Clone, Copy)]
pub(super) enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
    FloorDivide,
    Remainder,
    Power,
}

impl Operator {
    pub(super) fn prefix(input: &str) -> Option<(Self, usize)> {
        if input.starts_with("**") {
            return Some((Self::Power, 2));
        }
        if input.starts_with("//") {
            return Some((Self::FloorDivide, 2));
        }
        let symbol = input.chars().next()?;
        let operator = match symbol {
            '+' => Self::Add,
            '-' => Self::Subtract,
            '*' | '×' => Self::Multiply,
            '/' => Self::Divide,
            '%' => Self::Remainder,
            '^' => Self::Power,
            _ => return None,
        };
        Some((operator, symbol.len_utf8()))
    }

    pub(super) fn binding_power(self) -> (u8, u8) {
        match self {
            Self::Add | Self::Subtract => (1, 2),
            Self::Multiply | Self::Divide | Self::FloorDivide | Self::Remainder => (3, 4),
            Self::Power => (6, 5),
        }
    }

    pub(super) fn apply(self, left: f64, right: f64) -> Result<f64, CalculationError> {
        if matches!(self, Self::Divide | Self::FloorDivide | Self::Remainder) && right == 0. {
            return Err(CalculationError::DivisionByZero);
        }
        Ok(match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide => left / right,
            Self::FloorDivide | Self::Remainder => {
                let remainder = left % right;
                let adjust =
                    remainder != 0. && remainder.is_sign_negative() != right.is_sign_negative();
                if matches!(self, Self::FloorDivide) {
                    // 先移除余数再还原整数商，避免直接相除时舍入到下一整数。
                    let quotient = (left - remainder) / right - if adjust { 1. } else { 0. };
                    let floor = quotient.floor();
                    if quotient - floor > 0.5 {
                        floor + 1.
                    } else {
                        floor
                    }
                } else if adjust {
                    remainder + right
                } else {
                    remainder
                }
            }
            Self::Power => left.powf(right),
        })
    }
}
