use std::{
    fs,
    path::{Path, PathBuf},
};

const SKIPPED_DIRECTORIES: [&str; 4] = ["target", "corpus", "artifacts", "coverage"];
const UNSAFE_CRATES: [&str; 2] = ["caditor-windows", "caditor-zstd"];

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

const RAW_WIDGETS: [&str; 7] = [
    "ui.button(",
    "ui.spinner(",
    "Spinner::new(",
    "ui.small_button(",
    "Button::new(",
    "Button::selectable(",
    ".selectable_label(",
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
            text.lines()
                .enumerate()
                .filter_map(|(index, line)| {
                    raw_widget_at(line).map(|raw| format!("{}:{} {raw}", path.display(), index + 1))
                })
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(raw.is_empty(), "use the widgets.rs kit instead of {raw:#?}");
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
        } else {
            assert_eq!(
                table_entries(&manifest, "lints"),
                ["workspace = true"],
                "{name} does not inherit the workspace lints"
            );
        }
    }
}
