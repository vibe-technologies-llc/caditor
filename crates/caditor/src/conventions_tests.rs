use std::{
    fs,
    path::{Path, PathBuf},
};

const SKIPPED_DIRECTORIES: [&str; 4] = ["target", "corpus", "artifacts", "coverage"];
const UNSAFE_CRATES: [&str; 3] = ["caditor-wayland", "caditor-windows", "caditor-zstd"];

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

fn mentions_elapsed(characters: &[char], opening: usize) -> bool {
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
                    return false;
                }
            }
            character if is_identifier_character(*character) => {
                let word: String = characters
                    .iter()
                    .skip(index)
                    .take_while(|character| is_identifier_character(**character))
                    .collect();
                if word.contains(ELAPSED) {
                    return true;
                }
                index += word.chars().count();
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    false
}

const ELAPSED: &str = "elapsed";

fn first_timed_assertion(source: &str) -> Option<usize> {
    let characters: Vec<char> = source.chars().collect();
    let mut index = 0;
    while let Some(current) = characters.get(index) {
        if let Some(end) = literal_end(&characters, index) {
            index = end;
            continue;
        }
        if !is_identifier_character(*current) {
            index += 1;
            continue;
        }
        let word: String = characters
            .iter()
            .skip(index)
            .take_while(|character| is_identifier_character(**character))
            .collect();
        let after = index + word.chars().count();
        let is_assertion = word.starts_with("assert") || word.starts_with("debug_assert");
        if is_assertion
            && characters.get(after) == Some(&'!')
            && characters.get(after + 1) == Some(&'(')
            && mentions_elapsed(&characters, after + 1)
        {
            let line = characters
                .iter()
                .take(index)
                .filter(|character| **character == '\n')
                .count();
            return Some(line + 1);
        }
        index = after;
    }
    None
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

fn matrix_targets(workflow: &str) -> Vec<(String, String, String)> {
    let field = |entry: &str, key: &str| {
        let after = entry.split_once(&format!("{key}: "))?.1;
        let end = after.find([',', ' ', '}']).unwrap_or(after.len());
        Some(after.get(..end)?.to_owned())
    };
    let mut targets: Vec<(String, String, String)> = workflow
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- { target: "))
        .map(|line| {
            (
                field(line, "target").unwrap(),
                field(line, "seeds").unwrap(),
                field(line, "dictionary").unwrap(),
            )
        })
        .collect();
    targets.sort();
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
    let from_matrix: Vec<String> = matrix.iter().map(|(target, _, _)| target.clone()).collect();

    assert!(from_files.len() > 1);
    assert_eq!(
        from_manifest, from_files,
        "fuzz/Cargo.toml and fuzz_targets"
    );
    assert_eq!(
        from_matrix, from_files,
        "the ci.yml fuzz matrix and fuzz_targets"
    );

    for (target, seeds, dictionary) in matrix {
        assert!(
            root.join("fuzz/seeds").join(&seeds).is_dir(),
            "{target} names seeds {seeds} that do not exist"
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
