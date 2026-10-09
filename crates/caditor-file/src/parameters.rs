use std::path::Path;

use caditor_document::{Document, ImportedParameter, ParameterValues};

use crate::{
    read::read_file,
    reason::{ReadFailure, WriteFailure},
    save::write_atomically,
};

pub const PARAMETERS_EXTENSION: &str = "csv";
pub const MAX_PARAMETER_ROWS: usize = 10_000;
pub const MAX_PARAMETERS_FILE: usize = 16 << 20;

const NAME_COLUMN: &str = "name";
const EXPRESSION_COLUMN: &str = "expression";
const VALUE_COLUMN: &str = "value";
const NOTE_COLUMN: &str = "note";
const NOTE_ALIASES: [&str; 2] = [NOTE_COLUMN, "description"];
const DELIMITERS: [char; 3] = [',', ';', '\t'];
const UNEVALUATED: &str = "error";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParameterFileError {
    #[error("{0}")]
    Reading(ReadFailure),
    #[error("{0}")]
    Writing(WriteFailure),
    #[error("it is larger than the {} MiB a parameter file may be", MAX_PARAMETERS_FILE >> 20)]
    TooLarge,
    #[error("it is not UTF-8 text")]
    NotText,
    #[error("its first row does not name a “name” and an “expression” column")]
    NoHeader,
    #[error("a quoted value starting on line {line} is never closed")]
    UnclosedQuote { line: usize },
    #[error("it has more than {MAX_PARAMETER_ROWS} rows")]
    TooManyRows,
}

pub fn parameters_csv(document: &Document, values: &ParameterValues) -> String {
    let mut text = csv_row(&[NAME_COLUMN, EXPRESSION_COLUMN, VALUE_COLUMN, NOTE_COLUMN]);
    for parameter in document.parameters() {
        let expression = parameter
            .expression
            .to_text(&|id| document.parameter_name(id));
        let value = values
            .value(parameter.id())
            .map_or_else(|_| UNEVALUATED.to_owned(), |value| value.to_string());
        text.push_str(&csv_row(&[
            &parameter.name,
            &expression,
            &value,
            &parameter.note,
        ]));
    }
    text
}

pub fn write_parameters(path: &Path, text: &str) -> Result<(), ParameterFileError> {
    write_atomically(path, text.as_bytes())
        .map_err(|error| ParameterFileError::Writing(WriteFailure::of(&error)))
}

pub fn read_parameters(path: &Path) -> Result<Vec<ImportedParameter>, ParameterFileError> {
    let bytes =
        read_file(path).map_err(|error| ParameterFileError::Reading(ReadFailure::of(&error)))?;
    if bytes.len() > MAX_PARAMETERS_FILE {
        return Err(ParameterFileError::TooLarge);
    }
    let text = String::from_utf8(bytes).map_err(|_| ParameterFileError::NotText)?;
    parse_parameters(&text)
}

pub fn parse_parameters(text: &str) -> Result<Vec<ImportedParameter>, ParameterFileError> {
    let text = text.trim_start_matches('\u{feff}');
    let delimiter = delimiter_of(text);
    let records = records(text, delimiter)?;
    let mut records = records.into_iter();
    let header = records.next().ok_or(ParameterFileError::NoHeader)?;
    let column = |names: &[&str]| {
        header.iter().position(|cell| {
            let cell = cell.trim().to_lowercase();
            names.iter().any(|name| *name == cell)
        })
    };
    let (Some(name), Some(expression)) = (column(&[NAME_COLUMN]), column(&[EXPRESSION_COLUMN]))
    else {
        return Err(ParameterFileError::NoHeader);
    };
    let note = column(&NOTE_ALIASES);
    let cell = |record: &[String], index: usize| record.get(index).cloned().unwrap_or_default();
    let rows: Vec<ImportedParameter> = records
        .filter(|record| record.iter().any(|cell| !cell.trim().is_empty()))
        .map(|record| ImportedParameter {
            name: cell(&record, name),
            expression: cell(&record, expression),
            note: note.map(|note| cell(&record, note)).unwrap_or_default(),
        })
        .collect();
    if rows.len() > MAX_PARAMETER_ROWS {
        return Err(ParameterFileError::TooManyRows);
    }
    Ok(rows)
}

fn delimiter_of(text: &str) -> char {
    let header = text.lines().next().unwrap_or_default();
    DELIMITERS
        .into_iter()
        .max_by_key(|delimiter| header.matches(*delimiter).count())
        .filter(|delimiter| header.contains(*delimiter))
        .unwrap_or(',')
}

fn records(text: &str, delimiter: char) -> Result<Vec<Vec<String>>, ParameterFileError> {
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
                if records.len() > MAX_PARAMETER_ROWS + 1 {
                    return Err(ParameterFileError::TooManyRows);
                }
            }
            other if other == delimiter => record.push(std::mem::take(&mut cell)),
            other => cell.push(other),
        }
    }
    if quoted {
        return Err(ParameterFileError::UnclosedQuote { line: quote_line });
    }
    if !cell.is_empty() || !record.is_empty() {
        record.push(cell);
        records.push(record);
    }
    Ok(records)
}

fn csv_row(cells: &[&str]) -> String {
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

#[cfg(test)]
mod tests {
    use caditor_expression::Quantity;

    use super::*;

    fn document() -> Document {
        let mut document = Document::default();
        let mut transaction = document.transaction("Parameters");
        transaction.add_parameter("width", transaction.parse("40 mm").unwrap());
        let height = transaction.add_parameter("height", transaction.parse("width / 2").unwrap());
        transaction.add_parameter("turn", transaction.parse("-15 deg").unwrap());
        let mut document_transaction = transaction.finish();
        let note = caditor_document::Edit::SetParameterNote {
            id: height,
            note: "half the width, \"roughly\"\nsecond line".to_owned(),
        };
        let (label, mut edits) = document_transaction.into_parts();
        edits.push(note);
        document_transaction = caditor_document::Transaction::new(label, edits);
        document.apply(document_transaction).unwrap();
        document
    }

    #[test]
    fn exported_parameters_read_back_with_names_expressions_and_notes() {
        let document = document();
        let values = ParameterValues::evaluate(&document);

        let text = parameters_csv(&document, &values);
        let rows = parse_parameters(&text).unwrap();

        assert!(text.starts_with("name,expression,value,note\r\nwidth,40 mm,40 mm,\r\n"));
        assert!(text.contains("\"-15 deg\",\"-15°\""));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].name, "height");
        assert_eq!(rows[1].expression, "width / 2");
        assert_eq!(rows[1].note, "half the width, \"roughly\"\nsecond line");
        assert_eq!(rows[2].expression, "-15 deg");
        let mut target = Document::default();
        let plan = target.plan_parameter_import("Import", &rows, true).unwrap();
        target.apply(plan.transaction.unwrap()).unwrap();
        let values = ParameterValues::evaluate(&target);
        let height = target.parameter_named("height").unwrap().id();
        assert_eq!(values.value(height), Ok(Quantity::length(20.0)));
    }

    #[test]
    fn spreadsheet_variants_read_and_files_without_the_columns_are_refused() {
        let semicolons =
            "\u{feff}Name;Description;Expression\r\nwidth;outer;\"12,5 mm\"\r\n;;\r\nhole;;3 mm";
        let rows = parse_parameters(semicolons).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].note, "outer");
        assert_eq!(rows[0].expression, "12,5 mm");
        assert_eq!(rows[1].name, "hole");

        assert_eq!(
            parse_parameters("a,b\n1,2"),
            Err(ParameterFileError::NoHeader)
        );
        assert_eq!(parse_parameters(""), Err(ParameterFileError::NoHeader));
        assert_eq!(
            parse_parameters("name,expression\nx,\"3 mm\nmore"),
            Err(ParameterFileError::UnclosedQuote { line: 2 })
        );
        let many = format!(
            "name,expression\n{}",
            "x,1\n".repeat(MAX_PARAMETER_ROWS + 1)
        );
        assert_eq!(
            parse_parameters(&many),
            Err(ParameterFileError::TooManyRows)
        );
    }
}
