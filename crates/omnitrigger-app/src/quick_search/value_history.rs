use std::{cell::RefCell, rc::Rc};

use super::InputSuggestion;

const HISTORY_LIMIT: usize = 12;
const MATCH_LIMIT: usize = 6;

/// 同一工作台打开的搜索框共享本次运行中确认过的查询。
#[derive(Clone, Default)]
pub(super) struct RecentValues(Rc<RefCell<Vec<InputSuggestion>>>);

impl RecentValues {
    pub(super) fn record(&self, expression: &str, result: &str) {
        let expression = expression.trim();
        let mut items = self.0.borrow_mut();
        items.retain(|item| item.expression != expression);
        items.insert(
            0,
            InputSuggestion {
                expression: expression.to_owned(),
                result: result.to_owned(),
            },
        );
        items.truncate(HISTORY_LIMIT);
    }

    pub(super) fn matching(&self, query: &str) -> Vec<InputSuggestion> {
        let query = query.to_lowercase();
        self.0
            .borrow()
            .iter()
            .filter(|item| {
                let expression = item.expression.to_lowercase();
                let result = item.result.to_lowercase();
                query
                    .split_whitespace()
                    .all(|term| expression.contains(term) || result.contains(term))
            })
            .take(MATCH_LIMIT)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_confirmed_calculations_in_recency_order_within_the_session() {
        let history = RecentValues::default();
        let reopened = history.clone();
        history.record(" 2**10 ", "1024");
        history.record("sqrt(9)", "3");
        history.record("2**10", "1024");
        assert_eq!(
            reopened
                .matching("")
                .iter()
                .map(|item| item.expression.as_str())
                .collect::<Vec<_>>(),
            ["2**10", "sqrt(9)"],
        );
        assert_eq!(reopened.matching("SQRT 3")[0].expression, "sqrt(9)");
        assert_eq!(reopened.matching("1024")[0].expression, "2**10");
        assert!(RecentValues::default().matching("").is_empty());
        for value in 0..HISTORY_LIMIT {
            history.record(&format!("1+{value}"), &(value + 1).to_string());
        }
        assert!(reopened.matching("2**10").is_empty());
        assert_eq!(reopened.matching("").len(), MATCH_LIMIT);
    }
}
