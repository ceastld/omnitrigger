#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BatchError<E> {
    Empty,
    Row { line: usize, cause: E },
}

pub(super) fn batch_arguments<'a>(query: &'a str, name: &str, alias: &str) -> Option<&'a str> {
    let (operation, arguments) = query.split_once(char::is_whitespace).unwrap_or((query, ""));
    (operation == name || operation.eq_ignore_ascii_case(alias)).then_some(arguments)
}

pub(super) fn map_batch_lines<E>(
    input: &str,
    mut operation: impl FnMut(&str) -> Result<String, E>,
) -> Result<String, BatchError<E>> {
    let mut values = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let query = line.trim();
        if query.is_empty() {
            continue;
        }
        let value = operation(query).map_err(|cause| BatchError::Row {
            line: index + 1,
            cause,
        })?;
        values.push(value);
    }
    if values.is_empty() {
        return Err(BatchError::Empty);
    }
    Ok(values.join("\n"))
}
