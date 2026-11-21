#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

pub const APP: &str = "caditor";
pub const CONVENTIONS: &str = "test(/^conventions_tests::/)";
pub const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

const FUZZ: &str = "fuzz";
const FUZZ_MANIFEST: &str = "fuzz/Cargo.toml";
const DENY_CONFIG: &str = "deny.toml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub name: String,
    pub program: Program,
    pub args: Vec<String>,
    pub summary: Summary,
}

impl Step {
    fn new(
        name: impl Into<String>,
        program: Program,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            program,
            args: args.into_iter().map(Into::into).collect(),
            summary: Summary::Nothing,
        }
    }

    fn summarised(self, summary: Summary) -> Self {
        Self { summary, ..self }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    Cargo,
    Formatter,
}

impl Program {
    pub const fn path(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Formatter => "rust-formatter",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summary {
    Nothing,
    Nextest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Workspace,
    These {
        packages: BTreeSet<String>,
        conventions: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Testing<'a> {
    pub locked: bool,
    pub keep_going: bool,
    pub passed_on: &'a [String],
}

pub fn format(check: bool) -> Vec<Step> {
    let checked: &[&str] = if check { &["--check"] } else { &[] };
    let verb = if check { "format check" } else { "format" };

    vec![
        Step::new(verb, Program::Formatter, checked.iter().copied()),
        Step::new(
            format!("{verb} {FUZZ}/"),
            Program::Formatter,
            checked.iter().copied().chain([FUZZ]),
        ),
    ]
}

fn scope(selection: &Selection) -> Option<Vec<String>> {
    match selection {
        Selection::Workspace => Some(vec!["--workspace".to_owned()]),
        Selection::These { packages, .. } if packages.is_empty() => None,
        Selection::These { packages, .. } => Some(each_package(packages.iter())),
    }
}

fn each_package<'a>(packages: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    packages
        .into_iter()
        .flat_map(|package| ["-p".to_owned(), package.clone()])
        .collect()
}

fn locked_flag(locked: bool) -> &'static [&'static str] {
    if locked { &["--locked"] } else { &[] }
}

pub fn lint(selection: &Selection, windows: bool, locked: bool) -> Vec<Step> {
    let Some(packages) = scope(selection) else {
        return Vec::new();
    };

    let clippy = |name: &str, before: &[&str]| {
        let args = before
            .iter()
            .map(|word| (*word).to_owned())
            .chain(packages.iter().cloned())
            .chain(["--all-targets", "--all-features"].map(str::to_owned))
            .chain(locked_flag(locked).iter().map(|word| (*word).to_owned()))
            .chain(["--", "-D", "warnings"].map(str::to_owned));

        Step::new(name, Program::Cargo, args)
    };

    let mut steps = vec![clippy("clippy", &["clippy"])];

    if windows {
        steps.push(clippy(
            &format!("clippy ({WINDOWS_TARGET})"),
            &["xwin", "clippy", "--target", WINDOWS_TARGET],
        ));
    }
    steps
}

pub fn test(selection: &Selection, testing: Testing<'_>) -> Vec<Step> {
    let nextest = |packages: Vec<String>, extra: &[String]| {
        ["nextest", "run"]
            .map(str::to_owned)
            .into_iter()
            .chain(packages)
            .chain(
                locked_flag(testing.locked)
                    .iter()
                    .map(|word| (*word).to_owned()),
            )
            .chain(testing.keep_going.then(|| "--no-fail-fast".to_owned()))
            .chain(extra.iter().cloned())
            .collect::<Vec<_>>()
    };

    let mut steps = Vec::new();

    if let Some(packages) = scope(selection) {
        let narrowed = matches!(selection, Selection::These { .. });
        let mut extra: Vec<String> = Vec::new();

        if narrowed {
            extra.push("--no-tests=warn".to_owned());
        }
        extra.extend(testing.passed_on.iter().cloned());

        steps.push(
            Step::new("nextest", Program::Cargo, nextest(packages, &extra))
                .summarised(Summary::Nextest),
        );
    }

    if let Selection::These {
        packages,
        conventions: true,
    } = selection
        && !packages.contains(APP)
    {
        steps.push(
            Step::new(
                "conventions tests",
                Program::Cargo,
                nextest(
                    vec!["-p".to_owned(), APP.to_owned()],
                    &["-E".to_owned(), CONVENTIONS.to_owned()],
                ),
            )
            .summarised(Summary::Nextest),
        );
    }

    steps
}

pub fn deny(locked: bool) -> Vec<Step> {
    let locked = locked_flag(locked).iter().copied();

    vec![
        Step::new(
            "deny",
            Program::Cargo,
            ["deny"].into_iter().chain(locked.clone()).chain(["check"]),
        ),
        Step::new(
            format!("deny {FUZZ}/"),
            Program::Cargo,
            [
                "deny",
                "--manifest-path",
                FUZZ_MANIFEST,
                "--config",
                DENY_CONFIG,
            ]
            .into_iter()
            .chain(locked)
            .chain(["check"]),
        ),
    ]
}
