use super::{DEFAULT_SINCE, Scope, Task, parsed};
use crate::{error::Error, run::Output};

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn a_task_alone_runs_over_the_whole_workspace_and_keeps_its_output() {
    let asked = parsed(words("lint")).unwrap();

    assert_eq!(asked.task, Task::Lint);
    assert_eq!(asked.scope, Scope::Whole);
    assert_eq!(asked.output, Output::Kept);
    assert!(!asked.locked && !asked.keep_going && !asked.windows);
}

#[test]
fn changed_compares_against_origin_master_unless_since_names_another() {
    let changed = parsed(words("check --changed")).unwrap();
    let since = parsed(words("test --since HEAD~3")).unwrap();

    assert_eq!(
        changed.scope,
        Scope::Changed {
            since: DEFAULT_SINCE.to_owned()
        }
    );
    assert_eq!(
        since.scope,
        Scope::Changed {
            since: "HEAD~3".to_owned()
        }
    );
}

#[test]
fn what_follows_two_dashes_is_handed_to_nextest_whole() {
    let asked = parsed(words("test --locked -- -p caditor-kernel --no-capture")).unwrap();

    assert!(asked.locked);
    assert_eq!(asked.passed_on, words("-p caditor-kernel --no-capture"));
}

#[test]
fn verbose_streams_and_help_wins_over_any_task() {
    let verbose = parsed(words("check -v --windows")).unwrap();
    let help = parsed(words("lint --help")).unwrap();

    assert_eq!(verbose.output, Output::Streamed);
    assert!(verbose.windows);
    assert_eq!(help.task, Task::Help);
}

#[test]
fn a_flag_is_refused_by_a_task_it_does_not_apply_to() {
    let cases = [
        ("test --windows", "--windows", "test"),
        ("fmt --changed", "--changed", "fmt"),
        ("deny --since HEAD~1", "--changed", "deny"),
        ("lint --check", "--check", "lint"),
        ("fmt --locked", "--locked", "fmt"),
        ("check -- -p caditor", "--", "check"),
    ];

    for (line, refused, by) in cases {
        let error = parsed(words(line)).unwrap_err();

        assert!(
            matches!(error, Error::NotForTask { flag, task } if flag == refused && task == by),
            "{line}: {error:?}"
        );
    }
}

#[test]
fn unknown_words_and_missing_values_are_refused() {
    assert!(matches!(
        parsed(words("lint --fast")),
        Err(Error::UnknownFlag { .. })
    ));
    assert!(matches!(
        parsed(words("lint test")),
        Err(Error::UnknownTask { .. })
    ));
    assert!(matches!(
        parsed(words("test --since")),
        Err(Error::MissingValue { flag: "--since" })
    ));
    assert!(matches!(parsed(words("--locked")), Err(Error::NoTask)));
}
