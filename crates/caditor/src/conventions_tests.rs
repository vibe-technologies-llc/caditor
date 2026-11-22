use std::{
    fs,
    path::{Path, PathBuf},
};

const SKIPPED_DIRECTORIES: [&str; 4] = ["target", "corpus", "artifacts", "coverage"];
const UNSAFE_CRATES: [&str; 3] = ["caditor-wayland", "caditor-windows", "caditor-zstd"];
const ASSERTING_CRATES: [&str; 1] = ["caditor-checks"];
const PANIC_LINT: &str = "panic = \"deny\"";

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files_with_extension(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if !name.starts_with('.') && !SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|found| found == extension) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn first_rust_comment(source: &str) -> Option<usize> {
    let characters: Vec<char> = source.chars().collect();
    let at = |index: usize| characters.get(index).copied();
    let mut line = 1;
    let mut index = 0;

    while let Some(current) = at(index) {
        match current {
            '\n' => {
                line += 1;
                index += 1;
            }
            '/' if matches!(at(index + 1), Some('/' | '*')) => return Some(line),
            '"' => {
                index += 1;
                while let Some(inside) = at(index) {
                    match inside {
                        '\\' => index += 1,
                        '"' => break,
                        '\n' => line += 1,
                        _ => {}
                    }
                    index += 1;
                }
                index += 1;
            }
            '\'' => {
                if at(index + 1) == Some('\\') {
                    index += 2;
                    while at(index).is_some_and(|inside| inside != '\'') {
                        index += 1;
                    }
                    index += 1;
                } else if at(index + 2) == Some('\'') {
                    index += 3;
                } else {
                    index += 1;
                }
            }
            start if start.is_alphanumeric() || start == '_' => {
                let begin = index;

                while at(index).is_some_and(|inside| inside.is_alphanumeric() || inside == '_') {
                    index += 1;
                }

                let word: String = characters[begin..index].iter().collect();
                if matches!(word.as_str(), "r" | "br" | "cr") {
                    let mut hashes = 0;

                    while at(index + hashes) == Some('#') {
                        hashes += 1;
                    }
                    if at(index + hashes) == Some('"') {
                        index += hashes + 1;

                        let closing: Vec<char> = std::iter::once('"')
                            .chain("#".repeat(hashes).chars())
                            .collect();

                        while index < characters.len() && !characters[index..].starts_with(&closing)
                        {
                            if characters[index] == '\n' {
                                line += 1;
                            }
                            index += 1;
                        }
                        index += closing.len();
                    }
                }
            }
            _ => index += 1,
        }
    }
    None
}

fn first_toml_comment(source: &str) -> Option<usize> {
    let mut line = 1;
    let mut rest = source;

    while let Some(current) = rest.chars().next() {
        let quote = ["\"\"\"", "'''", "\"", "'"]
            .into_iter()
            .find(|quote| rest.starts_with(quote));

        if let Some(quote) = quote {
            rest = &rest[quote.len()..];

            let mut end = 0;

            loop {
                let remaining = &rest[end..];

                if remaining.is_empty() || remaining.starts_with(quote) {
                    break;
                }
                if quote.starts_with('"') && remaining.starts_with('\\') {
                    end += 1;
                }
                end += remaining[..].chars().next().map_or(1, char::len_utf8);
            }
            line += rest[..end.min(rest.len())].matches('\n').count();
            rest = &rest[(end + quote.len()).min(rest.len())..];
            continue;
        }
        match current {
            '#' => return Some(line),
            '\n' => line += 1,
            _ => {}
        }
        rest = &rest[current.len_utf8()..];
    }
    None
}

fn table_entries(manifest: &str, table: &str) -> Vec<String> {
    let header = format!("[{table}]");
    let mut entries: Vec<String> = manifest
        .lines()
        .skip_while(|line| line.trim() != header)
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();

    entries.sort();
    entries
}

#[test]
fn no_source_file_or_manifest_holds_a_comment() {
    let root = repository();
    let rust_files: Vec<PathBuf> = files_with_extension(&root.join("crates"), "rs")
        .into_iter()
        .chain(files_with_extension(&root.join("fuzz"), "rs"))
        .collect();
    let toml_files = files_with_extension(&root, "toml");

    assert!(
        rust_files
            .iter()
            .any(|path| path.ends_with("caditor/src/main.rs"))
    );
    assert!(toml_files.iter().any(|path| path.ends_with("deny.toml")));

    let rust = rust_files.into_iter().map(|path| {
        let comment = first_rust_comment(&fs::read_to_string(&path).unwrap());
        (path, comment)
    });
    let toml = toml_files.into_iter().map(|path| {
        let comment = first_toml_comment(&fs::read_to_string(&path).unwrap());
        (path, comment)
    });

    let commented: Vec<String> = rust
        .chain(toml)
        .filter_map(|(path, line)| Some(format!("{}:{}", path.display(), line?)))
        .collect();

    assert!(commented.is_empty(), "comments in {commented:#?}");
}

const RAW_WIDGETS: [&str; 14] = [
    "ui.button(",
    "ui.spinner(",
    "Spinner::new(",
    "ui.small_button(",
    "Button::new(",
    "Button::selectable(",
    ".selectable_label(",
    "ui.checkbox(",
    "Checkbox::new(",
    "Checkbox::without_text(",
    "ComboBox::",
    "ui.radio(",
    ".radio_value(",
    "RadioButton::new(",
];
const KIT_FILES: [&str; 1] = ["widgets.rs"];

fn raw_widget_at(line: &str) -> Option<&'static str> {
    RAW_WIDGETS.into_iter().find(|raw| {
        line.match_indices(raw).any(|(at, _)| {
            let before = line.get(..at).and_then(|start| start.chars().last());
            !before.is_some_and(|previous| previous.is_alphanumeric() || previous == '_')
        })
    })
}

fn statements(text: &str) -> Vec<(usize, String)> {
    let mut joined: Vec<(usize, String)> = Vec::new();

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        match joined.last_mut() {
            Some((_, previous)) if trimmed.starts_with('.') => previous.push_str(trimmed),
            _ => joined.push((index + 1, line.to_owned())),
        }
    }
    joined
}

#[test]
fn buttons_outside_the_widget_kit_come_from_it() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let raw: Vec<String> = files_with_extension(&source, "rs")
        .into_iter()
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            !KIT_FILES.contains(&name.as_ref())
                && !name.ends_with("_tests.rs")
                && !path.components().any(|part| part.as_os_str() == "ui_tests")
        })
        .flat_map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            statements(&text)
                .into_iter()
                .filter_map(|(line, statement)| {
                    raw_widget_at(&statement).map(|raw| format!("{}:{line} {raw}", path.display()))
                })
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(raw.is_empty(), "use the widgets.rs kit instead of {raw:#?}");

    let split = statements("let response = ui\n    .button(title)\n    .on_hover_text(hover);");

    assert_eq!(split.len(), 1);
    assert_eq!(
        split
            .first()
            .and_then(|(_, statement)| raw_widget_at(statement)),
        Some("ui.button(")
    );
    assert_eq!(raw_widget_at("widgets::checkbox(ui, &mut on, label)"), None);
    assert_eq!(
        raw_widget_at("ui.checkbox(&mut on, label)"),
        Some("ui.checkbox(")
    );
    assert_eq!(raw_widget_at("ToolButton::new(glyph)"), None);
    assert_eq!(raw_widget_at("widgets::spinner(ui);"), None);
    assert_eq!(raw_widget_at("ui.spinner();"), Some("ui.spinner("));
    assert_eq!(
        raw_widget_at("egui::Button::new(text)"),
        Some("Button::new(")
    );
}

#[test]
fn the_comment_scanners_tell_comments_from_strings_and_lifetimes() {
    assert_eq!(first_rust_comment("let a = 1;\n// note\n"), Some(2));
    assert_eq!(first_rust_comment("let a = 1; /* note */"), Some(1));
    assert_eq!(first_rust_comment("let url = \"https://example\";"), None);
    assert_eq!(first_rust_comment("let raw = r#\"a \" // b\"#;"), None);
    assert_eq!(
        first_rust_comment("fn f<'a>(x: &'a str) -> char { '/' }"),
        None
    );
    assert_eq!(
        first_rust_comment("let c = '\\''; let d = \"\\\"//\";"),
        None
    );
    assert_eq!(first_rust_comment("let r#type = 1;\n// after"), Some(2));

    assert_eq!(first_toml_comment("a = 1\n# note\n"), Some(2));
    assert_eq!(first_toml_comment("a = \"#not\"\nb = '#not'"), None);
    assert_eq!(
        first_toml_comment("a = \"\"\"\n#not\n\"\"\"\nb = 1 # yes"),
        Some(4)
    );
}

fn is_identifier_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn string_end(characters: &[char], opening: usize) -> usize {
    let mut index = opening + 1;

    while let Some(inside) = characters.get(index) {
        match inside {
            '\\' => index += 2,
            '"' => return index + 1,
            _ => index += 1,
        }
    }
    index
}

fn raw_string_end(characters: &[char], start: usize) -> Option<usize> {
    let hashes = characters
        .iter()
        .skip(start + 1)
        .take_while(|character| **character == '#')
        .count();

    if characters.get(start + 1 + hashes) != Some(&'"') {
        return None;
    }

    let closing: Vec<char> = std::iter::once('"')
        .chain(std::iter::repeat_n('#', hashes))
        .collect();
    let body = start + 2 + hashes;
    (body..characters.len())
        .find(|index| characters.get(*index..*index + closing.len()) == Some(closing.as_slice()))
        .map(|index| index + closing.len())
}

fn character_end(characters: &[char], start: usize) -> Option<usize> {
    match (characters.get(start + 1), characters.get(start + 2)) {
        (Some('\\'), _) => (start + 3..start + 10)
            .find(|index| characters.get(*index) == Some(&'\''))
            .map(|index| index + 1),
        (Some(_), Some('\'')) => Some(start + 3),
        _ => None,
    }
}

fn literal_end(characters: &[char], start: usize) -> Option<usize> {
    let before_is_identifier = start
        .checked_sub(1)
        .and_then(|before| characters.get(before))
        .is_some_and(|before| is_identifier_character(*before));
    match characters.get(start)? {
        '"' => Some(string_end(characters, start)),
        '\'' => character_end(characters, start),
        'r' if !before_is_identifier => raw_string_end(characters, start),
        _ => None,
    }
}

const ELAPSED: &str = "elapsed";
const DURATION_READS: [&str; 8] = [
    "as_secs",
    "as_secs_f32",
    "as_secs_f64",
    "as_millis",
    "as_micros",
    "as_nanos",
    "subsec_millis",
    "subsec_nanos",
];
const COMPARISONS: [&str; 6] = ["<=", ">=", "==", "!=", "<", ">"];

struct CodeWord {
    text: String,
    start: usize,
    end: usize,
}

fn code_words(characters: &[char]) -> Vec<CodeWord> {
    let mut words = Vec::new();
    let mut index = 0;

    while let Some(current) = characters.get(index) {
        if let Some(end) = literal_end(characters, index) {
            index = end;
            continue;
        }
        if !is_identifier_character(*current) {
            index += 1;
            continue;
        }

        let text: String = characters
            .iter()
            .skip(index)
            .take_while(|character| is_identifier_character(**character))
            .collect();
        let end = index + text.chars().count();

        words.push(CodeWord {
            text,
            start: index,
            end,
        });
        index = end;
    }
    words
}

fn after_group(characters: &[char], opening: usize) -> usize {
    let mut depth = 0_usize;
    let mut index = opening;

    while let Some(current) = characters.get(index) {
        if let Some(end) = literal_end(characters, index) {
            index = end;
            continue;
        }
        match current {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    index
}

fn after_statement(characters: &[char], start: usize) -> usize {
    let mut depth = 0_usize;
    let mut index = start;

    while let Some(current) = characters.get(index) {
        if let Some(end) = literal_end(characters, index) {
            index = end;
            continue;
        }
        match current {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => return index,
            _ => {}
        }
        index += 1;
    }
    index
}

fn is_clock_word(word: &str, bound: &[(String, usize)]) -> bool {
    word.contains(ELAPSED)
        || DURATION_READS.contains(&word)
        || bound.iter().any(|(name, _)| name == word)
}

fn word_depths(characters: &[char], words: &[CodeWord]) -> Vec<usize> {
    let mut depths = Vec::new();
    let mut depth = 0_usize;
    let mut index = 0;

    for word in words {
        while index < word.start {
            if let Some(end) = literal_end(characters, index) {
                index = end;
                continue;
            }
            match characters.get(index) {
                Some('{') => depth += 1,
                Some('}') => depth = depth.saturating_sub(1),
                _ => {}
            }
            index += 1;
        }
        depths.push(depth);
    }
    depths
}

fn bound_clock_name<'a>(
    characters: &[char],
    words: &'a [CodeWord],
    at: usize,
    bound: &[(String, usize)],
) -> Option<&'a str> {
    let keyword = words.get(at)?;
    let named = words
        .iter()
        .skip(at + 1)
        .find(|candidate| candidate.text != "mut")?;

    let pattern = characters
        .get(keyword.end..named.start)
        .is_some_and(|between| between.contains(&'('));
    let statement_end = after_statement(characters, named.end);
    let reads_clock = words
        .iter()
        .filter(|inner| inner.start >= named.end && inner.start < statement_end)
        .any(|inner| is_clock_word(&inner.text, bound));

    (!pattern && reads_clock).then_some(named.text.as_str())
}

fn assertion_spans(characters: &[char], words: &[CodeWord]) -> Vec<(usize, usize, usize)> {
    words
        .iter()
        .filter(|word| word.text.starts_with("assert") || word.text.starts_with("debug_assert"))
        .filter(|word| {
            characters.get(word.end) == Some(&'!') && characters.get(word.end + 1) == Some(&'(')
        })
        .map(|word| {
            (
                word.start,
                word.end + 1,
                after_group(characters, word.end + 1),
            )
        })
        .collect()
}

fn is_wait_condition(characters: &[char], words: &[CodeWord], at: usize) -> bool {
    let Some(word) = words.get(at) else {
        return false;
    };

    words.iter().take(at).rev().any(|earlier| {
        earlier.text == "while"
            && characters
                .get(earlier.end..word.start)
                .is_some_and(|between| !between.iter().any(|c| matches!(c, '{' | '}' | ';')))
    })
}

fn comparison_at(characters: &[char], index: usize) -> bool {
    COMPARISONS.iter().any(|operator| {
        operator
            .chars()
            .enumerate()
            .all(|(offset, expected)| characters.get(index + offset) == Some(&expected))
    })
}

fn comparison_after(characters: &[char], start: usize) -> bool {
    let mut index = start;

    loop {
        while characters
            .get(index)
            .is_some_and(|character| character.is_whitespace() || *character == '?')
        {
            index += 1;
        }
        match characters.get(index) {
            Some('(') => index = after_group(characters, index),
            Some('.') => {
                index += 1;
                while characters
                    .get(index)
                    .is_some_and(|character| is_identifier_character(*character))
                {
                    index += 1;
                }
            }
            _ => return comparison_at(characters, index),
        }
    }
}

fn comparison_before(characters: &[char], start: usize) -> bool {
    let mut index = start;

    while index > 0
        && characters
            .get(index - 1)
            .is_some_and(|character| is_identifier_character(*character) || *character == '.')
    {
        index -= 1;
    }
    while index > 0
        && characters
            .get(index - 1)
            .is_some_and(|character| character.is_whitespace())
    {
        index -= 1;
    }

    let last = index.checked_sub(1).and_then(|at| characters.get(at));
    let before = index.checked_sub(2).and_then(|at| characters.get(at));
    match last {
        Some('<') => true,
        Some('>') => !matches!(before, Some('-' | '=')),
        Some('=') => matches!(before, Some('=' | '!' | '<' | '>')),
        _ => false,
    }
}

fn first_clock_use(source: &str, compared: bool) -> Option<usize> {
    let characters: Vec<char> = source.chars().collect();
    let words = code_words(&characters);

    let depths = word_depths(&characters, &words);
    let spans = assertion_spans(&characters, &words);
    let mut bound: Vec<(String, usize)> = Vec::new();

    for (index, (word, depth)) in words.iter().zip(&depths).enumerate() {
        bound.retain(|(_, scope)| scope <= depth);

        if word.text == "let" {
            if let Some(name) = bound_clock_name(&characters, &words, index, &bound) {
                bound.push((name.to_owned(), *depth));
            }
            continue;
        }

        let clock = is_clock_word(&word.text, &bound);
        let constructor = words.get(index + 1).filter(|next| {
            word.text == "Duration"
                && next.text.starts_with("from_")
                && characters.get(word.end..next.start) == Some(&[':', ':'][..])
        });

        if !clock && constructor.is_none() {
            continue;
        }

        let asserted = spans
            .iter()
            .find(|(_, opening, closing)| word.start > *opening && word.start < *closing);
        let reported = match (asserted, clock, constructor) {
            (Some((start, _, _)), true, _) => Some(*start),
            (_, true, _) if compared => (comparison_after(&characters, word.end)
                || comparison_before(&characters, word.start))
            .then_some(word.start),
            (_, _, Some(next)) if compared => (comparison_after(&characters, next.end)
                || comparison_before(&characters, word.start))
            .then_some(word.start),
            _ => None,
        };
        let Some(position) = reported else {
            continue;
        };

        if asserted.is_none() && is_wait_condition(&characters, &words, index) {
            continue;
        }

        let line = characters
            .iter()
            .take(position)
            .filter(|character| **character == '\n')
            .count();

        return Some(line + 1);
    }
    None
}

fn first_timed_assertion(source: &str) -> Option<usize> {
    first_clock_use(source, false)
}

fn first_compared_duration(source: &str) -> Option<usize> {
    first_clock_use(source, true)
}

fn only_test_items(code: &str) -> String {
    let mut kept = Vec::new();
    let mut lines = code.lines();

    while let Some(line) = lines.next() {
        if line.trim() != "#[cfg(test)]" {
            kept.push("");
            continue;
        }
        kept.push(line);

        let mut depth = 0_i64;
        let mut opened = false;
        for inside in lines.by_ref() {
            let opening = inside.matches('{').count();
            let closing = inside.matches('}').count();

            depth += i64::try_from(opening).unwrap() - i64::try_from(closing).unwrap();
            opened |= opening > 0;
            kept.push(inside);
            if (opened && depth <= 0) || (!opened && inside.trim_end().ends_with(';')) {
                break;
            }
        }
    }
    kept.join("\n")
}

#[test]
fn no_assertion_reads_the_clock() {
    let root = repository();
    let timed: Vec<String> = files_with_extension(&root.join("crates"), "rs")
        .into_iter()
        .chain(files_with_extension(&root.join("fuzz"), "rs"))
        .filter_map(|path| {
            let line = first_timed_assertion(&fs::read_to_string(&path).unwrap())?;
            Some(format!("{}:{line}", path.display()))
        })
        .collect();

    assert!(
        timed.is_empty(),
        "bound the work done (a count or a tally), never wall-clock time, in {timed:#?}"
    );
}

#[test]
fn no_test_compares_a_duration() {
    let root = repository();
    let compared: Vec<String> = files_with_extension(&root.join("crates"), "rs")
        .into_iter()
        .chain(files_with_extension(&root.join("fuzz"), "rs"))
        .filter_map(|path| {
            let source = fs::read_to_string(&path).unwrap();
            let tests = if is_test_source(&path) {
                source
            } else {
                only_test_items(&source)
            };
            let line = first_compared_duration(&tests)?;
            Some(format!("{}:{line}", path.display()))
        })
        .collect();

    assert!(
        compared.is_empty(),
        "bound the work done (a count or a tally), never a duration compared in a test, in {compared:#?}"
    );
}

#[test]
fn the_duration_scanner_tells_compared_durations_from_printed_ones() {
    let ordered = "let a = 1;\nif started.elapsed() > LIMIT {\n    fail();\n}\n";
    let reversed = "if LIMIT > started.elapsed() {}";
    let chained = "if started.elapsed().as_secs_f64() < 2.0 {}";
    let bound = "let took = started.elapsed();\nlet slow = took > LIMIT;";
    let derived =
        "let took = started.elapsed();\nlet ratio = took.as_secs_f64() / base;\nif ratio > 2.0 {}";
    let constructed = "if Duration::from_secs(2) < took {}";
    let compared_to_constructed = "if slow >= Duration::from_millis(5) {}";

    assert_eq!(first_compared_duration(ordered), Some(2));
    assert_eq!(first_compared_duration(reversed), Some(1));
    assert_eq!(first_compared_duration(chained), Some(1));
    assert_eq!(first_compared_duration(bound), Some(2));
    assert_eq!(first_compared_duration(derived), Some(3));
    assert_eq!(first_compared_duration(constructed), Some(1));
    assert_eq!(first_compared_duration(compared_to_constructed), Some(1));

    assert_eq!(
        first_compared_duration("let took = started.elapsed();"),
        None
    );
    assert_eq!(
        first_compared_duration("println!(\"{:?}\", started.elapsed());"),
        None
    );
    assert_eq!(
        first_compared_duration("let limits: Vec<Duration> = Vec::new();"),
        None
    );
    assert_eq!(first_compared_duration("if count > 3 {}"), None);
    assert_eq!(first_compared_duration("fn f() -> Duration { x }"), None);
    assert_eq!(
        first_compared_duration("while quiet.elapsed() < QUIET {\n    wait();\n}"),
        None
    );
    assert_eq!(
        first_compared_duration(
            "while !done() && quiet.elapsed() < QUIET {}\nif quiet.elapsed() > A {}"
        ),
        Some(2)
    );
    assert_eq!(
        first_compared_duration(
            "fn a() {\n    let took = started.elapsed();\n}\nfn b() {\n    if took > 3 {}\n}"
        ),
        None
    );
    assert_eq!(
        first_compared_duration("let (mesh, took) = run(started.elapsed());\nif mesh > 3 {}"),
        None
    );
    assert_eq!(first_timed_assertion(ordered), None);
    assert_eq!(
        first_timed_assertion("let took = started.elapsed();\nassert!(took < LIMIT);"),
        Some(2)
    );
    assert_eq!(
        first_timed_assertion("assert!(a.as_secs_f64() < 2.0);"),
        Some(1)
    );
}

#[test]
fn the_clock_scanner_tells_assertions_on_elapsed_time_from_other_code() {
    let timed = "let a = 1;\nassert!(started.elapsed() < LIMIT);\n";
    let split = "assert!(\n    ok,\n    \"{:?}\",\n    started.elapsed()\n);";
    let named = "assert_eq!(elapsed_ms(run()), 1);";
    let debug = "debug_assert!(clock.elapsed() < LIMIT);";

    assert_eq!(first_timed_assertion(timed), Some(2));
    assert_eq!(first_timed_assertion(split), Some(1));
    assert_eq!(first_timed_assertion(named), Some(1));
    assert_eq!(first_timed_assertion(debug), Some(1));

    assert_eq!(first_timed_assertion("let took = started.elapsed();"), None);
    assert_eq!(first_timed_assertion("assert!(\"elapsed\" == text);"), None);
    assert_eq!(
        first_timed_assertion("assert!(r#\"elapsed)\"# == text);"),
        None
    );
    assert_eq!(
        first_timed_assertion("assert!(count < 3);\nlet t = clock.elapsed();"),
        None
    );
    assert_eq!(
        first_timed_assertion("assert!(Instant::now() < deadline, \"late\");"),
        None
    );
    assert_eq!(
        first_timed_assertion("assert!(f(')') && clock.ok());"),
        None
    );
}

#[test]
fn every_member_inherits_the_workspace_lints() {
    let root = repository();
    let workspace = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let workspace_clippy = table_entries(&workspace, "workspace.lints.clippy");
    let mut members: Vec<PathBuf> = fs::read_dir(root.join("crates"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("Cargo.toml").exists())
        .collect();

    members.sort();

    assert!(!workspace_clippy.is_empty());
    assert!(members.len() > 1);

    for member in members {
        let manifest = fs::read_to_string(member.join("Cargo.toml")).unwrap();
        let name = member.file_name().unwrap().to_string_lossy().into_owned();
        if UNSAFE_CRATES.contains(&name.as_str()) {
            assert_eq!(
                table_entries(&manifest, "lints.clippy"),
                workspace_clippy,
                "{name} lists other clippy lints than the workspace"
            );
            assert_eq!(
                table_entries(&manifest, "lints.rust"),
                ["unsafe_code = \"deny\""],
                "{name} must deny unsafe code outside the items that allow it"
            );
        } else if ASSERTING_CRATES.contains(&name.as_str()) {
            let without_panic: Vec<String> = workspace_clippy
                .iter()
                .filter(|lint| lint.as_str() != PANIC_LINT)
                .cloned()
                .collect();

            assert_eq!(
                table_entries(&manifest, "lints.clippy"),
                without_panic,
                "{name} lists other clippy lints than the workspace's but panic"
            );
            assert_eq!(
                table_entries(&manifest, "lints.rust"),
                table_entries(&workspace, "workspace.lints.rust"),
                "{name} lists other rust lints than the workspace"
            );
        } else {
            assert_eq!(
                table_entries(&manifest, "lints"),
                ["workspace = true"],
                "{name} does not inherit the workspace lints"
            );
        }
    }
}

const RULES_FOLDER: &str = ".claude/rules";
const CROSS_CUTTING_PREFIX: &str = "**/";
const FRONTMATTER_FENCE: &str = "---";

struct Rule {
    name: String,
    scope: Vec<String>,
    body: String,
}

fn rules() -> Vec<Rule> {
    let mut rules: Vec<Rule> = fs::read_dir(repository().join(RULES_FOLDER))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|found| found == "md"))
        .map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let (scope, body) = split_frontmatter(&text);
            Rule { name, scope, body }
        })
        .collect();

    rules.sort_by(|left, right| left.name.cmp(&right.name));
    rules
}

fn split_frontmatter(text: &str) -> (Vec<String>, String) {
    let mut lines = text.lines();

    if lines.next() != Some(FRONTMATTER_FENCE) {
        return (Vec::new(), text.to_owned());
    }

    let mut scope = Vec::new();

    for line in lines.by_ref() {
        if line == FRONTMATTER_FENCE {
            break;
        }
        if let Some(pattern) = line.trim().strip_prefix("- ") {
            scope.push(pattern.trim_matches('"').to_owned());
        }
    }
    (scope, lines.collect::<Vec<_>>().join("\n"))
}

fn segment_matches(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((before, after)) => {
            name.len() >= before.len() + after.len()
                && name.starts_with(before)
                && name.ends_with(after)
        }
    }
}

fn glob_matches(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern.split_first(), path.split_first()) {
        (None, _) => path.is_empty(),
        (Some((&"**", rest)), _) => {
            (0..=path.len()).any(|skipped| glob_matches(rest, path.get(skipped..).unwrap()))
        }
        (Some(_), None) => false,
        (Some((segment, rest)), Some((name, remaining))) => {
            segment_matches(segment, name) && glob_matches(rest, remaining)
        }
    }
}

fn subject_patterns(rules: &[Rule]) -> Vec<(String, String)> {
    rules
        .iter()
        .flat_map(|rule| {
            rule.scope
                .iter()
                .filter(|pattern| !pattern.starts_with(CROSS_CUTTING_PREFIX))
                .map(|pattern| (rule.name.clone(), pattern.clone()))
        })
        .collect()
}

#[test]
fn every_source_file_loads_a_rule_about_it() {
    let root = repository();
    let patterns = subject_patterns(&rules());

    assert!(!patterns.is_empty());

    let unruled: Vec<String> = files_with_extension(&root, "rs")
        .into_iter()
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap()
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .filter(|path| {
            let segments: Vec<&str> = path.split('/').collect();
            !patterns.iter().any(|(_, pattern)| {
                glob_matches(&pattern.split('/').collect::<Vec<_>>(), &segments)
            })
        })
        .collect();

    assert!(
        unruled.is_empty(),
        "these files match no rule's paths, so editing them loads no rule; add each to the rule \
         describing it: {unruled:#?}"
    );
}

#[test]
fn every_rules_file_holds_its_rules_and_the_rules_table_lists_each_one() {
    let rules = rules();
    let guide = fs::read_to_string(repository().join("CLAUDE.md")).unwrap();
    let table = guide.split("## Rules").nth(1).unwrap();
    let listed: Vec<&str> = table
        .lines()
        .filter_map(|line| line.strip_prefix("| "))
        .filter_map(|row| row.split(" | ").next())
        .flat_map(|cell| cell.split('`'))
        .filter(|name| name.ends_with(".md"))
        .collect();
    let names: Vec<&str> = rules.iter().map(|rule| rule.name.as_str()).collect();

    assert!(rules.len() > 1);

    for rule in &rules {
        let rules_given = rule
            .body
            .lines()
            .map(str::trim)
            .any(|line| !line.is_empty() && !line.starts_with('#'));

        assert!(
            rules_given,
            "{} holds nothing under its title; was it emptied by mistake?",
            rule.name
        );
        assert!(
            listed.contains(&rule.name.as_str()),
            "{} is missing from the rules table in CLAUDE.md",
            rule.name
        );
    }
    for name in listed {
        assert!(
            names.contains(&name),
            "CLAUDE.md lists {name}, which is not in {RULES_FOLDER}"
        );
    }
}

#[test]
fn rule_paths_match_like_globs() {
    let matches = |pattern: &str, path: &str| {
        glob_matches(
            &pattern.split('/').collect::<Vec<_>>(),
            &path.split('/').collect::<Vec<_>>(),
        )
    };

    assert!(matches(
        "crates/caditor-kernel/**",
        "crates/caditor-kernel/src/lib.rs"
    ));
    assert!(matches("**/*.rs", "crates/caditor/src/app.rs"));
    assert!(matches(
        "crates/caditor/src/app.rs",
        "crates/caditor/src/app.rs"
    ));
    assert!(!matches(
        "crates/caditor/src/app.rs",
        "crates/caditor/src/apps.rs"
    ));
    assert!(!matches(
        "crates/caditor/src/files/**",
        "crates/caditor/src/files.rs"
    ));
    assert!(!matches("**/*.rs", "crates/caditor/Cargo.toml"));
}

const GITHUB_FOLDER: &str = ".github";
const COMMIT_LENGTH: usize = 40;

fn workflow_files() -> Vec<(String, String)> {
    let root = repository().join(GITHUB_FOLDER);
    let mut files: Vec<(String, String)> = ["workflows", "actions/install-rust"]
        .into_iter()
        .flat_map(|folder| {
            fs::read_dir(root.join(folder))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|found| found == "yml" || found == "yaml")
                })
                .collect::<Vec<_>>()
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&path).unwrap())
        })
        .collect();

    files.sort();
    files
}

struct MatrixTarget {
    target: String,
    seeds: String,
    dictionary: String,
    max_len: usize,
}

fn matrix_targets(workflow: &str) -> Vec<MatrixTarget> {
    let field = |entry: &str, key: &str| {
        let after = entry.split_once(&format!("{key}: "))?.1;
        let end = after.find([',', ' ', '}']).unwrap_or(after.len());
        Some(after.get(..end)?.to_owned())
    };
    let mut targets: Vec<MatrixTarget> = workflow
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- { target: "))
        .map(|line| MatrixTarget {
            target: field(line, "target").unwrap(),
            seeds: field(line, "seeds").unwrap(),
            dictionary: field(line, "dictionary").unwrap(),
            max_len: field(line, "max_len").unwrap().parse().unwrap(),
        })
        .collect();

    targets.sort_by(|a, b| a.target.cmp(&b.target));
    targets
}

#[test]
fn fuzz_targets_are_listed_alike_by_files_manifest_and_ci_matrix() {
    let root = repository();
    let from_files: Vec<String> = {
        let mut names: Vec<String> = fs::read_dir(root.join("fuzz/fuzz_targets"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter_map(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
            .collect();

        names.sort();
        names
    };
    let manifest = fs::read_to_string(root.join("fuzz/Cargo.toml")).unwrap();
    let from_manifest: Vec<String> = {
        let mut names: Vec<String> = manifest
            .lines()
            .filter_map(|line| line.trim().strip_prefix("name = \""))
            .filter_map(|rest| Some(rest.strip_suffix('"')?.to_owned()))
            .filter(|name| name != "caditor-fuzz")
            .collect();

        names.sort();
        names
    };
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let matrix = matrix_targets(&workflow);
    let from_matrix: Vec<String> = matrix.iter().map(|entry| entry.target.clone()).collect();

    assert!(from_files.len() > 1);
    assert_eq!(
        from_manifest, from_files,
        "fuzz/Cargo.toml and fuzz_targets"
    );
    assert_eq!(
        from_matrix, from_files,
        "the ci.yml fuzz matrix and fuzz_targets"
    );

    for MatrixTarget {
        target,
        seeds,
        dictionary,
        max_len,
    } in matrix
    {
        let seeds_dir = root.join("fuzz/seeds").join(&seeds);

        assert!(
            seeds_dir.is_dir(),
            "{target} names seeds {seeds} that do not exist"
        );

        let largest_seed = fs::read_dir(&seeds_dir)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len() as usize)
            .max()
            .unwrap();

        assert!(
            max_len >= largest_seed,
            "{target} has a max_len of {max_len}, below its largest seed of {largest_seed} bytes"
        );
        assert!(
            dictionary == "none"
                || root
                    .join("fuzz/dictionaries")
                    .join(format!("{dictionary}.dict"))
                    .is_file(),
            "{target} names a dictionary {dictionary} that does not exist"
        );
    }
}

#[test]
fn every_action_is_pinned_by_commit_with_its_version_in_the_step_name() {
    let files = workflow_files();

    assert!(files.iter().any(|(name, _)| name == "ci.yml"));

    for (file, text) in files {
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        for (index, line) in lines.iter().enumerate() {
            let Some(used) = line.strip_prefix("uses: ") else {
                continue;
            };

            if used.starts_with("./") {
                continue;
            }

            let (action, commit) = used
                .split_once('@')
                .unwrap_or_else(|| panic!("{file}: {used} names no commit"));

            assert!(
                commit.len() == COMMIT_LENGTH && commit.chars().all(|c| c.is_ascii_hexdigit()),
                "{file}: {used} is not pinned by a full commit"
            );

            let step_name = lines
                .get(..index)
                .unwrap()
                .iter()
                .rev()
                .find_map(|earlier| earlier.strip_prefix("- name: "))
                .unwrap_or_else(|| panic!("{file}: {used} has no step name"));

            assert!(
                step_name.contains(&format!("({action} v")),
                "{file}: the step '{step_name}' does not name {action}'s version"
            );
        }
    }
}

#[test]
fn no_pin_is_copied_into_a_workflow() {
    let versions =
        fs::read_to_string(repository().join(GITHUB_FOLDER).join("versions.env")).unwrap();
    let pins: Vec<(&str, &str)> = versions
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();

    assert!(pins.len() > 5);

    for (file, text) in workflow_files() {
        for (name, value) in &pins {
            assert!(
                !text.contains(value),
                "{file} holds the value of {name}; read it from the environment"
            );
        }
    }
}

const TEXT_PAYLOADS: [&str; 5] = ["reason", "message", "detail", "words", "refusal"];
const TEXT_TYPES: [&str; 2] = ["String", "&'static str"];

fn is_test_source(path: &Path) -> bool {
    let name = path.file_name().unwrap().to_string_lossy();
    name.ends_with("tests.rs")
        || path
            .components()
            .any(|component| matches!(component.as_os_str().to_str(), Some("tests" | "ui_tests")))
}

fn without_literals(source: &str) -> String {
    let characters: Vec<char> = source.chars().collect();
    let mut code = String::new();
    let mut index = 0;

    while let Some(current) = characters.get(index) {
        match literal_end(&characters, index) {
            Some(end) => {
                code.push_str("\"\"");
                index = end;
            }
            None => {
                code.push(*current);
                index += 1;
            }
        }
    }
    code
}

fn without_test_items(code: &str) -> String {
    let mut kept = Vec::new();
    let mut lines = code.lines();

    while let Some(line) = lines.next() {
        if line.trim() != "#[cfg(test)]" {
            kept.push(line);
            continue;
        }

        let mut depth = 0_i64;
        let mut opened = false;
        for skipped in lines.by_ref() {
            let opening = skipped.matches('{').count();
            let closing = skipped.matches('}').count();

            depth += i64::try_from(opening).unwrap() - i64::try_from(closing).unwrap();
            opened |= opening > 0;
            if (opened && depth <= 0) || (!opened && skipped.trim_end().ends_with(';')) {
                break;
            }
        }
    }
    kept.join("\n")
}

fn flattened(code: &str) -> String {
    code.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn generic_arguments(code: &str, opening: usize) -> Option<&str> {
    let mut depth = 0_usize;
    let mut previous = ' ';

    for (offset, character) in code.get(opening..)?.char_indices() {
        match character {
            '<' => depth += 1,
            '>' if previous != '-' => {
                depth -= 1;
                if depth == 0 {
                    return code.get(opening + 1..opening + offset);
                }
            }
            _ => {}
        }
        previous = character;
    }
    None
}

fn last_argument(arguments: &str) -> Option<&str> {
    let mut depth = 0_usize;
    let mut last = None;

    for (offset, character) in arguments.char_indices() {
        match character {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => last = Some(offset + 1),
            _ => {}
        }
    }
    arguments.get(last?..).map(str::trim)
}

fn first_text_error(source: &str) -> Option<String> {
    text_error_in(source, &TEXT_TYPES)
}

fn text_error_in(source: &str, texts: &[&str]) -> Option<String> {
    let code = flattened(&without_test_items(&without_literals(source)));
    let string_result = code.match_indices("Result<").find_map(|(at, word)| {
        let preceded = code[..at]
            .chars()
            .next_back()
            .is_some_and(is_identifier_character);
        let error = last_argument(generic_arguments(&code, at + word.len() - 1)?)?;
        (!preceded && texts.contains(&error)).then(|| format!("Result<…, {error}>"))
    });
    let bridge = code.match_indices(" for String {").find_map(|(at, _)| {
        let start = code[..at].rfind("impl")?;
        code[start..at]
            .contains("From<")
            .then(|| code[start..at + 13].to_owned())
    });
    let payload = TEXT_PAYLOADS.iter().find_map(|field| {
        texts.iter().find_map(|text| {
            [
                format!("{{ {field}: {text} }}"),
                format!("{{ {field}: {text}, }}"),
            ]
            .into_iter()
            .find(|shape| code.contains(shape.as_str()))
        })
    });
    let tuple = texts
        .contains(&"&'static str")
        .then_some(())
        .and_then(|()| {
            code.match_indices("(&'static str)").find_map(|(at, _)| {
                let name_start = code[..at].trim_end_matches(is_identifier_character).len();
                let attribute_before = code[..name_start].trim_end().ends_with(")]");
                attribute_before.then(|| code[name_start..at + 14].to_owned())
            })
        });
    string_result.or(bridge).or(payload).or(tuple)
}

#[test]
fn errors_are_typed_rather_than_text() {
    let root = repository();
    let texts: Vec<String> = files_with_extension(&root.join("crates"), "rs")
        .into_iter()
        .filter(|path| !is_test_source(path))
        .filter_map(|path| {
            let source = fs::read_to_string(&path).unwrap();
            let shape = first_text_error(&source)?;
            Some(format!("{}: {shape}", path.display()))
        })
        .collect();

    assert!(
        texts.is_empty(),
        "give each failure a variant of an error enum with typed data, worded by its Display, \
         rather than text, in {texts:#?}"
    );
}

#[test]
fn the_text_error_scanner_finds_string_errors_outside_tests_only() {
    let result = "fn f() -> Result<u8, String> { Ok(1) }";
    let nested = "fn f() -> Result<Vec<(u8, u8)>, String> { todo() }";
    let bridge =
        "impl From<Error> for String {\n    fn from(e: Error) -> Self { e.to_string() }\n}";
    let generic = "impl<P: Display> From<Edit<P>> for String { }";
    let payload = "enum E {\n    Refused {\n        reason: String,\n    },\n}";

    assert!(first_text_error(result).is_some());
    assert!(first_text_error(nested).is_some());
    assert!(first_text_error(bridge).is_some());
    assert!(first_text_error(generic).is_some());
    assert!(first_text_error(payload).is_some());

    assert_eq!(
        first_text_error("fn f() -> io::Result<String> { g() }"),
        None
    );
    assert_eq!(
        first_text_error("fn f() -> Result<String, Error> { g() }"),
        None
    );
    assert_eq!(
        first_text_error("fn f() -> TryResult<u8, String> { g() }"),
        None
    );
    assert_eq!(
        first_text_error("const T: &str = \"Result<u8, String>\";"),
        None
    );
    assert!(first_text_error("fn f() -> Result<u8, &'static str> { g() }").is_some());
    assert!(
        first_text_error("enum E {\n    #[error(\"{0}\")]\n    Unfitting(&'static str),\n}")
            .is_some()
    );
    assert!(first_text_error("enum E { Refused { words: &'static str } }").is_some());
    assert_eq!(first_text_error("enum E { Keys(&'static str) }"), None);
    assert!(first_text_error("fn f() -> Result<u8, String> { g() }").is_some());
    assert_eq!(
        first_text_error("fn f() -> Result<&'static str, E> { g() }"),
        None
    );
    assert_eq!(first_text_error("impl Reason for String { }"), None);
    assert_eq!(first_text_error("enum E { Named { name: String } }"), None);
    assert_eq!(
        first_text_error("enum E { Refused { reason: String, remedy: String } }"),
        None
    );
    assert_eq!(
        first_text_error(
            "#[cfg(test)]\nmod tests {\n    fn f() -> Result<u8, String> { Ok(1) }\n}\n"
        ),
        None
    );
    assert_eq!(
        first_text_error("#[cfg(test)]\nfn f() -> Result<u8, String> {\n    Ok(1)\n}\nfn g() {}"),
        None
    );
    assert!(
        first_text_error("#[cfg(test)]\nuse x;\nfn f() -> Result<u8, String> { Ok(1) }").is_some()
    );
}

const BLOCKING_COMPARISON: &str = "same_file_blocking";
const OFF_THREAD_FUNCTIONS: [&str; 3] =
    ["use caditor_file", "fn open_file(", "fn check_save_target("];
const HEADLESS_SOURCE: &str = "headless.rs";

fn enclosing_item(lines: &[&str], index: usize) -> String {
    lines
        .get(..=index)
        .unwrap_or_default()
        .iter()
        .rev()
        .find(|line| line.starts_with(|first: char| first.is_alphabetic()))
        .map(|line| (*line).to_owned())
        .unwrap_or_default()
}

fn blocking_comparisons_outside_jobs(name: &str, source: &str) -> Vec<String> {
    if name == HEADLESS_SOURCE {
        return Vec::new();
    }

    let code = without_test_items(source);
    let lines: Vec<&str> = code.lines().collect();

    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains(BLOCKING_COMPARISON))
        .map(|(index, _)| enclosing_item(&lines, index))
        .filter(|item| {
            !(name == "files.rs"
                && OFF_THREAD_FUNCTIONS
                    .iter()
                    .any(|allowed| item.contains(allowed)))
        })
        .collect()
}

#[test]
fn the_interface_thread_never_compares_files_by_identity() {
    let root = repository();
    let found: Vec<String> = files_with_extension(&root.join("crates/caditor/src"), "rs")
        .into_iter()
        .filter(|path| !is_test_source(path))
        .flat_map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = fs::read_to_string(&path).unwrap();

            blocking_comparisons_outside_jobs(&name, &source)
                .into_iter()
                .map(move |item| format!("{}: {item}", path.display()))
        })
        .collect();

    assert!(
        found.is_empty(),
        "same_file_blocking stats the disk, so only the open and Save As jobs and the headless \
         export call it, never code the interface thread runs, in {found:#?}"
    );
}

#[test]
fn the_identity_comparison_scanner_allows_the_jobs_and_flags_methods() {
    let jobs = "use caditor_file::{\n    same_file_blocking,\n};\n\nfn open_file(\n    path: &Path,\n) -> Outcome {\n    if same_file_blocking(a, b) {\n    }\n}\n";
    let method = "impl Files {\n    fn poll(&mut self) {\n        if same_file_blocking(a, b) {\n        }\n    }\n}\n";

    assert!(blocking_comparisons_outside_jobs("files.rs", jobs).is_empty());
    assert_eq!(
        blocking_comparisons_outside_jobs("files.rs", method).len(),
        1
    );
    assert_eq!(blocking_comparisons_outside_jobs("model.rs", jobs).len(), 2);
}
