use std::{cell::RefCell, rc::Rc};

use super::examples::search_key;
use super::InputSuggestion;

const HISTORY_LIMIT: usize = 12;

/// 同一工作台打开的搜索框共享本次运行中确认过的查询。
#[derive(Clone, Default)]
pub(super) struct RecentValues(Rc<RefCell<Vec<InputSuggestion>>>);

impl RecentValues {
    pub(super) fn record(&self, expression: &str, result: &str) {
        self.record_exact(expression.trim(), result);
    }

    pub(super) fn record_exact(&self, expression: &str, result: &str) {
        let mut items = self.0.borrow_mut();
        items.retain(|item| item.expression != expression);
        items.insert(
            0,
            InputSuggestion {
                expression: expression.to_owned(),
                label: None,
                result: result.to_owned(),
            },
        );
        items.truncate(HISTORY_LIMIT);
    }

    pub(super) fn latest_number(&self) -> Option<f64> {
        self.0
            .borrow()
            .iter()
            .find_map(|item| super::calculator::evaluate(&item.result).ok())
    }

    pub(super) fn matching(&self, query: &str) -> Vec<InputSuggestion> {
        if query.trim().is_empty() {
            return self.0.borrow().clone();
        }
        let query = search_key(query);
        self.0
            .borrow()
            .iter()
            .filter(|item| {
                let expression = search_key(&item.expression);
                let result = search_key(&item.result);
                query
                    .split_whitespace()
                    .all(|term| expression.contains(term) || result.contains(term))
            })
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
        assert_eq!(reopened.matching("ＳＱＲＴ　３")[0].expression, "sqrt(9)");
        assert_eq!(reopened.matching("1024")[0].expression, "2**10");
        assert!(RecentValues::default().matching("").is_empty());
        for value in 0..HISTORY_LIMIT {
            history.record(&format!("1+{value}"), &(value + 1).to_string());
        }
        assert!(reopened.matching("2**10").is_empty());
        assert_eq!(reopened.matching("").len(), HISTORY_LIMIT);
        assert_eq!(reopened.matching("").last().unwrap().expression, "1+0");
    }
}
