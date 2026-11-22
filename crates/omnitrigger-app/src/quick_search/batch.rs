#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BatchError<E> {
    Empty,
    Row { line: usize, cause: E },
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
