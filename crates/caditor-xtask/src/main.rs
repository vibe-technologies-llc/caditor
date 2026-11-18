mod changed;
mod error;
mod grammar;
mod plan;
mod run;

use std::{collections::BTreeSet, env, path::Path, process::ExitCode};

use crate::{
    changed::Touch,
    error::Error,
    grammar::{Asked, Scope, Task},
    plan::{Selection, Step, Testing},
};

struct Scoped {
    selection: Selection,
    dependencies_moved: bool,
}

fn main() -> ExitCode {
    match done() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn done() -> Result<(), Error> {
    let asked = grammar::parsed(env::args().skip(1))?;

    if asked.task == Task::Help {
        print!("{}", grammar::USAGE);
        return Ok(());
    }

    let root = run::workspace_root();
    let steps = steps_for(&asked, &root)?;

    run::all(&root, &steps, asked.output, asked.keep_going)
}

fn steps_for(asked: &Asked, root: &Path) -> Result<Vec<Step>, Error> {
    let testing = Testing {
        locked: asked.locked,
        keep_going: asked.keep_going,
        passed_on: &asked.passed_on,
    };

    Ok(match asked.task {
        Task::Help => Vec::new(),
        Task::Format => plan::format(asked.check),
        Task::Deny => plan::deny(asked.locked),
        Task::Lint => plan::lint(&scoped(asked, root)?.selection, asked.windows, asked.locked),
        Task::Test => plan::test(&scoped(asked, root)?.selection, testing),
        Task::Check => {
            let scoped = scoped(asked, root)?;

            let mut steps = plan::format(true);
            steps.extend(plan::lint(&scoped.selection, asked.windows, asked.locked));
            if scoped.dependencies_moved {
                steps.extend(plan::deny(asked.locked));
            }
            steps.extend(plan::test(&scoped.selection, testing));
            steps
        }
    })
}

fn scoped(asked: &Asked, root: &Path) -> Result<Scoped, Error> {
    let Scope::Changed { since } = &asked.scope else {
        return Ok(Scoped {
            selection: Selection::Workspace,
            dependencies_moved: true,
        });
    };

    let files = changed::files(root, since)?;
    let members = changed::members(root)?;

    let selection = match changed::owners(&files, &members) {
        Touch::Everything => Selection::Workspace,
        Touch::These(touched) => Selection::These {
            packages: if touched.is_empty() {
                touched
            } else {
                changed::dependents(root, &touched, &members)?
            },
            conventions: !files.is_empty(),
        },
    };

    tell(&selection, since, files.len());
    Ok(Scoped {
        selection,
        dependencies_moved: changed::moves_dependencies(&files),
    })
}

fn tell(selection: &Selection, since: &str, files: usize) {
    match selection {
        Selection::Workspace => {
            println!(
                "{files} files changed since {since}, one of them workspace-wide: every package"
            );
        }
        Selection::These { packages, .. } => println!(
            "{files} files changed since {since}: {} packages changed or depending on one{}",
            packages.len(),
            listed(packages)
        ),
    }
}

fn listed(packages: &BTreeSet<String>) -> String {
    if packages.is_empty() {
        String::new()
    } else {
        format!(
            ": {}",
            packages
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}
