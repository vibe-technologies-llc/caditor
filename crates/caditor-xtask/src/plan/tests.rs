use std::collections::BTreeSet;

use super::{Program, Selection, Step, Summary, Testing, deny, format, lint, test};

fn these(packages: &[&str], conventions: bool) -> Selection {
    Selection::These {
        packages: packages
            .iter()
            .map(|package| (*package).to_owned())
            .collect::<BTreeSet<_>>(),
        conventions,
    }
}

fn line(step: &Step) -> String {
    format!("{} {}", step.program.path(), step.args.join(" "))
}

fn lines(steps: &[Step]) -> Vec<String> {
    steps.iter().map(line).collect()
}

const PLAIN: Testing<'static> = Testing {
    locked: false,
    keep_going: false,
    passed_on: &[],
};

#[test]
fn the_whole_workspace_is_checked_by_the_lines_ci_runs() {
    let locked = Testing {
        locked: true,
        ..PLAIN
    };

    assert_eq!(
        lines(&lint(&Selection::Workspace, false, true)),
        ["cargo clippy --workspace --all-targets --all-features --locked -- -D warnings"]
    );
    assert_eq!(
        lines(&test(&Selection::Workspace, locked)),
        ["cargo nextest run --workspace --locked"]
    );
    assert_eq!(
        lines(&deny(true)),
        [
            "cargo deny --locked check",
            "cargo deny --manifest-path fuzz/Cargo.toml --config deny.toml --locked check"
        ]
    );
    assert_eq!(
        lines(&format(true)),
        ["rust-formatter --check", "rust-formatter --check fuzz"]
    );
}

#[test]
fn windows_lints_the_same_selection_through_xwin() {
    let steps = lint(&these(&["caditor-file"], true), true, false);

    assert_eq!(
        lines(&steps),
        [
            "cargo clippy -p caditor-file --all-targets --all-features -- -D warnings",
            "cargo xwin clippy --target x86_64-pc-windows-msvc -p caditor-file --all-targets \
             --all-features -- -D warnings"
        ]
    );
}

#[test]
fn changed_packages_are_tested_alone_with_the_conventions_tests_beside_them() {
    let steps = test(&these(&["caditor-kernel", "caditor-step"], true), PLAIN);

    assert_eq!(
        lines(&steps),
        [
            "cargo nextest run -p caditor-kernel -p caditor-step --no-tests=warn",
            "cargo nextest run -p caditor -E test(/^conventions_tests::/)"
        ]
    );
    assert!(steps.iter().all(|step| step.summary == Summary::Nextest));
    assert!(steps.iter().all(|step| step.program == Program::Cargo));
}

#[test]
fn a_change_outside_every_package_runs_the_conventions_tests_alone() {
    let docs = test(&these(&[], true), PLAIN);
    let nothing = test(&these(&[], false), PLAIN);

    assert_eq!(
        lines(&docs),
        ["cargo nextest run -p caditor -E test(/^conventions_tests::/)"]
    );
    assert!(nothing.is_empty());
    assert!(lint(&these(&[], true), true, false).is_empty());
}

#[test]
fn the_app_s_own_tests_already_hold_the_conventions_tests() {
    let steps = test(&these(&["caditor"], true), PLAIN);

    assert_eq!(
        lines(&steps),
        ["cargo nextest run -p caditor --no-tests=warn"]
    );
}

#[test]
fn keep_going_runs_every_test_and_passed_words_reach_the_main_run_alone() {
    let passed_on = ["--no-capture".to_owned()];
    let testing = Testing {
        locked: false,
        keep_going: true,
        passed_on: &passed_on,
    };

    assert_eq!(
        lines(&test(&these(&["caditor-sketch"], true), testing)),
        [
            "cargo nextest run -p caditor-sketch --no-fail-fast --no-tests=warn --no-capture",
            "cargo nextest run -p caditor --no-fail-fast -E test(/^conventions_tests::/)"
        ]
    );
}
