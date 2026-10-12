use std::path::Path;

use caditor_document::{
    Document, ImportedColumn, ImportedConfiguration, ImportedConfigurations, MAX_CONFIGURATIONS,
    MAX_CONFIGURED_VALUES,
};

use crate::{
    csv::{CsvError, csv_row, delimiter_in, records},
    read::read_file,
    reason::{ReadFailure, WriteFailure},
    save::write_atomically,
};

pub const MAX_CONFIGURATION_ROWS: usize = 4 * MAX_CONFIGURATIONS;
pub const MAX_CONFIGURATION_COLUMNS: usize = 4 * MAX_CONFIGURED_VALUES;
pub const MAX_CONFIGURATIONS_FILE: usize = 16 << 20;

const NAME_COLUMN: &str = "Configuration";
const NAME_ALIASES: [&str; 2] = ["configuration", "name"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigurationFileError {
    #[error("{0}")]
    Reading(ReadFailure),
    #[error("{0}")]
    Writing(WriteFailure),
    #[error("it is larger than the {} MiB a configurations file may be", MAX_CONFIGURATIONS_FILE >> 20)]
    TooLarge,
    #[error("it is not UTF-8 text")]
    NotText,
    #[error("its first row does not name a “Configuration” column")]
    NoHeader,
    #[error("a quoted value starting on line {line} is never closed")]
    UnclosedQuote { line: usize },
    #[error("it has more than {MAX_CONFIGURATION_ROWS} rows")]
    TooManyRows,
    #[error("it has more than {MAX_CONFIGURATION_COLUMNS} columns")]
    TooManyColumns,
    #[error("caditor ran into an internal error while handling it")]
    Internal,
}

pub fn configurations_csv(document: &Document) -> String {
    let configurations = document.configurations();
    let columns: Vec<_> = configurations
        .values
        .iter()
        .filter_map(|value| Some((*value, document.configuration_column_header(*value)?)))
        .collect();
    let mut header = vec![NAME_COLUMN];
    header.extend(columns.iter().map(|(_, header)| header.as_str()));
    let mut text = csv_row(&header);
    for row in &configurations.rows {
        let cells: Vec<String> = columns
            .iter()
            .map(|(value, _)| {
                document
                    .configuration_cell_text(row.id, *value)
                    .unwrap_or_default()
            })
            .collect();
        let mut line = vec![row.name.as_str()];
        line.extend(cells.iter().map(String::as_str));
        text.push_str(&csv_row(&line));
    }
    text
}

pub fn write_configurations(path: &Path, text: &str) -> Result<(), ConfigurationFileError> {
    write_atomically(path, text.as_bytes())
        .map_err(|error| ConfigurationFileError::Writing(WriteFailure::of(&error)))
}

pub fn read_configurations(path: &Path) -> Result<ImportedConfigurations, ConfigurationFileError> {
    let bytes = read_file(path)
        .map_err(|error| ConfigurationFileError::Reading(ReadFailure::of(&error)))?;
    if bytes.len() > MAX_CONFIGURATIONS_FILE {
        return Err(ConfigurationFileError::TooLarge);
    }
    let text = String::from_utf8(bytes).map_err(|_| ConfigurationFileError::NotText)?;
    parse_configurations(&text)
}

pub fn parse_configurations(text: &str) -> Result<ImportedConfigurations, ConfigurationFileError> {
    let text = text.trim_start_matches('\u{feff}');
    let delimiter = delimiter_in(text.lines().next().unwrap_or_default());
    let mut records = records(text, delimiter, MAX_CONFIGURATION_ROWS)
        .map_err(configuration_csv_error)?
        .into_iter();
    let header = records.next().ok_or(ConfigurationFileError::NoHeader)?;
    let name = header
        .iter()
        .position(|cell| NAME_ALIASES.contains(&cell.trim().to_lowercase().as_str()))
        .ok_or(ConfigurationFileError::NoHeader)?;
    let value_columns: Vec<usize> = header
        .iter()
        .enumerate()
        .filter(|(position, cell)| *position != name && !cell.trim().is_empty())
        .map(|(position, _)| position)
        .collect();
    if value_columns.len() > MAX_CONFIGURATION_COLUMNS {
        return Err(ConfigurationFileError::TooManyColumns);
    }
    let columns = value_columns
        .iter()
        .filter_map(|position| header.get(*position))
        .map(|cell| ImportedColumn::from_header(cell))
        .collect();
    let rows = records
        .filter(|record| record.iter().any(|cell| !cell.trim().is_empty()))
        .map(|record| ImportedConfiguration {
            name: record.get(name).cloned().unwrap_or_default(),
            cells: value_columns
                .iter()
                .map(|position| record.get(*position).cloned().unwrap_or_default())
                .collect(),
        })
        .collect();
    Ok(ImportedConfigurations { columns, rows })
}

fn configuration_csv_error(error: CsvError) -> ConfigurationFileError {
    match error {
        CsvError::UnclosedQuote { line } => ConfigurationFileError::UnclosedQuote { line },
        CsvError::TooManyRows => ConfigurationFileError::TooManyRows,
    }
}

#[cfg(test)]
mod tests {
    use caditor_document::{ColumnOutcome, ConfiguredValue, Editor, RowOutcome, Setting};

    use super::*;

    fn model() -> Document {
        let mut document = Document::default();
        let mut transaction = document.transaction("Parameters");
        let width = transaction.add_parameter("width", transaction.parse("40 mm").unwrap());
        transaction.add_parameter("turn", transaction.parse("-15 deg").unwrap());
        document.apply(transaction.finish()).unwrap();
        let mut editor = Editor::new(document);
        let configure = editor
            .document()
            .configuring(ConfiguredValue::Parameter(width))
            .unwrap();
        editor.apply(configure).unwrap();
        let turn = editor.document().parameter_named("turn").unwrap().id();
        let configure = editor
            .document()
            .configuring(ConfiguredValue::Parameter(turn))
            .unwrap();
        editor.apply(configure).unwrap();
        let (first, _) = editor
            .document()
            .adding_configuration(Some("Small, \"slim\""));
        editor.apply(first).unwrap();
        let (second, large) = editor.document().adding_configuration(Some("Large"));
        editor.apply(second).unwrap();
        let wide = editor
            .document()
            .transaction("Parse")
            .parse("60 mm")
            .unwrap();
        let change = editor
            .document()
            .setting_configuration(
                large,
                ConfiguredValue::Parameter(width),
                Setting::Expression(wide),
            )
            .unwrap();
        editor.apply(change).unwrap();
        editor.document().clone()
    }

    #[test]
    fn exported_configurations_read_back_and_import_as_no_change() {
        let document = model();

        let text = configurations_csv(&document);
        let file = parse_configurations(&text).unwrap();
        let plan = document
            .plan_configuration_import("Import", &file, true)
            .unwrap();

        assert!(text.starts_with(
            "Configuration,Parameter: width,Parameter: turn\r\n\"Small, \"\"slim\"\"\",40 mm,\"-15 deg\"\r\n"
        ));
        assert!(text.contains("\r\nLarge,60 mm,\"-15 deg\"\r\n"));
        assert_eq!(file.columns.len(), 2);
        assert_eq!(file.rows.len(), 2);
        assert_eq!(file.rows[0].name, "Small, \"slim\"");
        assert!(plan.transaction.is_none());
        assert!(
            plan.rows
                .iter()
                .all(|row| row.outcome == RowOutcome::Unchanged)
        );
        assert!(
            plan.columns
                .iter()
                .all(|column| column.outcome == ColumnOutcome::Known)
        );
    }

    #[test]
    fn an_exported_table_rebuilds_in_a_model_with_the_same_names() {
        let source = model();
        let text = configurations_csv(&source);
        let mut target = Document::default();
        let mut transaction = target.transaction("Parameters");
        transaction.add_parameter("width", transaction.parse("1 mm").unwrap());
        transaction.add_parameter("turn", transaction.parse("0 deg").unwrap());
        target.apply(transaction.finish()).unwrap();

        let file = parse_configurations(&text).unwrap();
        let plan = target
            .plan_configuration_import("Import", &file, true)
            .unwrap();
        target.apply(plan.transaction.unwrap()).unwrap();

        assert_eq!(configurations_csv(&target), text);
    }

    #[test]
    fn spreadsheet_variants_read_and_files_without_the_name_column_are_refused() {
        let semicolons = "\u{feff}Name;Parameter: width;;Colour: Body\r\nSmall;\"12,5 mm\";;#fff\r\n;;;\r\nLarge;3 mm";

        let file = parse_configurations(semicolons).unwrap();

        assert_eq!(file.columns.len(), 2);
        assert_eq!(file.columns[1].header, "Colour: Body");
        assert_eq!(file.rows.len(), 2);
        assert_eq!(file.rows[0].cells, ["12,5 mm", "#fff"]);
        assert_eq!(file.rows[1].cells, ["3 mm", ""]);
        assert_eq!(
            parse_configurations("a,b\n1,2"),
            Err(ConfigurationFileError::NoHeader)
        );
        assert_eq!(
            parse_configurations(""),
            Err(ConfigurationFileError::NoHeader)
        );
        assert_eq!(
            parse_configurations("Configuration,Parameter: x\nA,\"3 mm\nmore"),
            Err(ConfigurationFileError::UnclosedQuote { line: 2 })
        );
    }

    #[test]
    fn a_name_column_in_any_position_and_case_and_tab_separators_read() {
        let file = parse_configurations("Parameter: x\tCONFIGURATION\n5 mm\tA\n").unwrap();

        assert_eq!(file.rows[0].name, "A");
        assert_eq!(file.rows[0].cells, ["5 mm"]);
    }

    #[test]
    fn files_past_the_row_and_column_limits_are_refused_whole() {
        let rows = format!(
            "Configuration\n{}",
            "x\n".repeat(MAX_CONFIGURATION_ROWS + 1)
        );
        let columns = format!(
            "Configuration,{}\nA",
            (0..=MAX_CONFIGURATION_COLUMNS)
                .map(|n| format!("Parameter: p{n}"))
                .collect::<Vec<_>>()
                .join(",")
        );

        assert_eq!(
            parse_configurations(&rows),
            Err(ConfigurationFileError::TooManyRows)
        );
        assert_eq!(
            parse_configurations(&columns),
            Err(ConfigurationFileError::TooManyColumns)
        );
    }

    #[test]
    fn a_file_that_is_not_text_or_cannot_be_read_is_refused_in_words() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("table.csv");
        std::fs::write(&binary, [0xff, 0xfe, 0x00, 0x9f]).unwrap();

        let refused = read_configurations(&binary).unwrap_err();
        let missing = read_configurations(&dir.path().join("none.csv")).unwrap_err();

        assert_eq!(refused, ConfigurationFileError::NotText);
        assert!(matches!(missing, ConfigurationFileError::Reading(_)));
    }

    #[test]
    fn writing_replaces_the_file_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("table.csv");
        let document = model();

        write_configurations(&path, &configurations_csv(&document)).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            configurations_csv(&document)
        );
    }
}
