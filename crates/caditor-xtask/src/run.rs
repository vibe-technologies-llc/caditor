use std::{
    env,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use crate::{
    error::{Error, Query},
    plan::{Step, Summary},
};

const CRATE_DEPTH: usize = 2;
const LINES_SHOWN_OF_A_FAILURE: usize = 60;
const LOGS: &str = "xtask";
const NEXTEST_SUMMARY: &str = "Summary [";
const PROGRESS: [&str; 13] = [
    "Compiling ",
    "Checking ",
    "Finished ",
    "Blocking ",
    "Downloading ",
    "Downloaded ",
    "Updating ",
    "Locking ",
    "Fresh ",
    "Running ",
    "PASS [",
    "START ",
    "SLOW ",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Kept,
    Streamed,
}

pub fn asked(command: &mut Command, program: &'static str, query: Query) -> Result<String, Error> {
    let answer = command
        .stderr(Stdio::inherit())
        .output()
        .map_err(|source| Error::Spawn { program, source })?;

    if answer.status.success() {
        Ok(String::from_utf8_lossy(&answer.stdout).into_owned())
    } else {
        Err(Error::Refused {
            query,
            code: answer.status.code(),
        })
    }
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(CRATE_DEPTH)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

pub fn all(root: &Path, steps: &[Step], output: Output, keep_going: bool) -> Result<(), Error> {
    if steps.is_empty() {
        println!("nothing to run");
        return Ok(());
    }

    let logs = logs_within(root);
    if output == Output::Kept {
        fresh(&logs)?;
    }

    let mut failed = 0;
    for (index, step) in steps.iter().enumerate() {
        let passed = match output {
            Output::Kept => kept(root, &logs, index, step)?,
            Output::Streamed => streamed(root, step)?,
        };
        if !passed {
            failed += 1;
            if !keep_going {
                break;
            }
        }
    }

    match failed {
        0 => Ok(()),
        count => Err(Error::Failed {
            count,
            of: steps.len(),
        }),
    }
}

fn fresh(logs: &Path) -> Result<(), Error> {
    let log_error = |source| Error::Log {
        path: logs.to_path_buf(),
        source,
    };

    if logs.exists() {
        fs::remove_dir_all(logs).map_err(log_error)?;
    }
    fs::create_dir_all(logs).map_err(log_error)
}

fn logs_within(root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), |target| root.join(target))
        .join(LOGS)
}

fn command(root: &Path, step: &Step) -> Command {
    let mut command = Command::new(step.program.path());
    command.current_dir(root).args(&step.args);
    command
}

fn kept(root: &Path, logs: &Path, index: usize, step: &Step) -> Result<bool, Error> {
    let path = logs.join(format!("{:02}-{}.log", index + 1, slug(&step.name)));
    let log_error = |source| Error::Log {
        path: path.clone(),
        source,
    };
    let log = File::create(&path).map_err(log_error)?;
    let errors = log.try_clone().map_err(log_error)?;

    print!("{} ... ", step.name);
    io::stdout().flush().ok();

    let began = Instant::now();
    let status = command(root, step)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(errors)
        .status()
        .map_err(|source| Error::Spawn {
            program: step.program.path(),
            source,
        })?;
    let took = began.elapsed().as_secs_f64();

    let written = fs::read_to_string(&path).unwrap_or_default();
    if status.success() {
        println!("ok ({took:.1}s)");
        if step.summary == Summary::Nextest
            && let Some(summary) = written.lines().rfind(|line| line.contains(NEXTEST_SUMMARY))
        {
            println!("    {}", summary.trim());
        }
    } else {
        println!("FAILED ({took:.1}s)");
        for line in failure_lines(&written) {
            println!("    {line}");
        }
        println!("    the whole output: {}", path.display());
    }
    Ok(status.success())
}

fn failure_lines(written: &str) -> impl Iterator<Item = &str> {
    let lines: Vec<&str> = written.lines().filter(|line| !is_progress(line)).collect();
    let skipped = lines.len().saturating_sub(LINES_SHOWN_OF_A_FAILURE);
    lines.into_iter().skip(skipped)
}

fn streamed(root: &Path, step: &Step) -> Result<bool, Error> {
    println!("== {}", step.name);

    let began = Instant::now();
    let status = command(root, step)
        .status()
        .map_err(|source| Error::Spawn {
            program: step.program.path(),
            source,
        })?;

    println!(
        "== {} {} ({:.1}s)",
        step.name,
        if status.success() { "ok" } else { "FAILED" },
        began.elapsed().as_secs_f64()
    );
    Ok(status.success())
}

fn is_progress(line: &str) -> bool {
    let line = line.trim_start();
    PROGRESS.iter().any(|word| line.starts_with(word))
}

fn slug(name: &str) -> String {
    name.split(|letter: char| !letter.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::{LINES_SHOWN_OF_A_FAILURE, failure_lines, is_progress, slug};

    #[test]
    fn a_failure_is_shown_without_the_build_s_progress() {
        assert!(is_progress("   Compiling caditor-kernel v0.1.0 (/work)"));
        assert!(is_progress(
            "        PASS [   0.004s] (1/9) caditor-kernel curve::tests::reads"
        ));
        assert!(!is_progress(
            "        FAIL [   0.004s] (2/9) caditor-kernel curve::tests::reads"
        ));
        assert!(!is_progress("error[E0308]: mismatched types"));
    }

    #[test]
    fn a_failure_shows_only_its_last_lines() {
        let written: String = (0..LINES_SHOWN_OF_A_FAILURE + 5)
            .map(|line| format!("line {line}\n"))
            .chain(["   Compiling late v0.1.0\n".to_owned()])
            .collect();

        let shown: Vec<&str> = failure_lines(&written).collect();

        assert_eq!(shown.len(), LINES_SHOWN_OF_A_FAILURE);
        assert_eq!(shown.first(), Some(&"line 5"));
        assert_eq!(
            shown.last(),
            Some(&format!("line {}", LINES_SHOWN_OF_A_FAILURE + 4).as_str())
        );
    }

    #[test]
    fn a_step_s_log_is_named_by_its_words() {
        assert_eq!(
            slug("clippy (x86_64-pc-windows-msvc)"),
            "clippy-x86-64-pc-windows-msvc"
        );
    }
}
