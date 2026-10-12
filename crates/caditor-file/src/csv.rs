pub(crate) const DELIMITERS: [char; 3] = [',', ';', '\t'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CsvError {
    UnclosedQuote { line: usize },
    TooManyRows,
}

pub(crate) fn delimiter_in(header: &str) -> char {
    DELIMITERS
        .into_iter()
        .max_by_key(|delimiter| header.matches(*delimiter).count())
        .filter(|delimiter| header.contains(*delimiter))
        .unwrap_or(',')
}

pub(crate) fn records(
    text: &str,
    delimiter: char,
    max_rows: usize,
) -> Result<Vec<Vec<String>>, CsvError> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut line = 1;
    let mut quote_line = 1;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if quoted {
            match character {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                '\n' => {
                    line += 1;
                    cell.push('\n');
                }
                other => cell.push(other),
            }
            continue;
        }
        match character {
            '"' if cell.trim().is_empty() => {
                cell.clear();
                quoted = true;
                quote_line = line;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                line += 1;
                record.push(std::mem::take(&mut cell));
                records.push(std::mem::take(&mut record));
                if records.len() > max_rows + 1 {
                    return Err(CsvError::TooManyRows);
                }
            }
            other if other == delimiter => record.push(std::mem::take(&mut cell)),
            other => cell.push(other),
        }
    }
    if quoted {
        return Err(CsvError::UnclosedQuote { line: quote_line });
    }
    if !cell.is_empty() || !record.is_empty() {
        record.push(cell);
        records.push(record);
    }
    Ok(records)
}

pub(crate) fn csv_row(cells: &[&str]) -> String {
    let mut row = cells
        .iter()
        .map(|cell| csv_cell(cell))
        .collect::<Vec<_>>()
        .join(",");
    row.push_str("\r\n");
    row
}

fn csv_cell(cell: &str) -> String {
    let needs_quotes = cell.contains([',', ';', '\t', '"', '\n', '\r'])
        || cell.trim() != cell
        || cell.starts_with(['=', '+', '-', '@']);
    if needs_quotes {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_owned()
    }
}
