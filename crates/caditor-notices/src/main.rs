mod components;
mod error;
mod metadata;
mod render;

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use crate::error::NoticesError;

struct Arguments {
    target: Option<String>,
    output: PathBuf,
}

fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Arguments, NoticesError> {
    let mut target = None;
    let mut output: Option<PathBuf> = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--target" {
            target = Some(arguments.next().ok_or(NoticesError::TargetMissing)?);
        } else if argument.starts_with("--") {
            return Err(NoticesError::UnknownOption { argument });
        } else if let Some(first) = output.take() {
            return Err(NoticesError::TwoOutputs {
                first,
                second: PathBuf::from(argument),
            });
        } else {
            output = Some(PathBuf::from(argument));
        }
    }
    Ok(Arguments {
        target,
        output: output.ok_or(NoticesError::OutputMissing)?,
    })
}

fn write(path: &Path, notices: &str) -> Result<(), NoticesError> {
    let unwritable = |source| NoticesError::Unwritable {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(unwritable)?;
    }
    fs::write(path, notices).map_err(unwritable)
}

fn run() -> Result<(), NoticesError> {
    let arguments = parse_arguments(env::args().skip(1))?;
    let metadata = metadata::read(arguments.target.as_deref())?;
    let inventory = components::inventory(&metadata)?;
    write(&arguments.output, &render::render(&inventory))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("caditor-notices: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(words: &[&str]) -> Result<Arguments, NoticesError> {
        parse_arguments(words.iter().map(|word| (*word).to_owned()))
    }

    #[test]
    fn the_output_and_an_optional_target_are_read() {
        let plain = arguments(&["notices.txt"]).unwrap();
        let targeted = arguments(&["--target", "x86_64-pc-windows-msvc", "notices.txt"]).unwrap();

        assert_eq!(plain.target, None);
        assert_eq!(plain.output, PathBuf::from("notices.txt"));
        assert_eq!(targeted.target.as_deref(), Some("x86_64-pc-windows-msvc"));
    }

    #[test]
    fn a_wrong_command_line_is_refused_in_words() {
        assert!(matches!(arguments(&[]), Err(NoticesError::OutputMissing)));
        assert!(matches!(
            arguments(&["a", "--target"]),
            Err(NoticesError::TargetMissing)
        ));
        assert!(matches!(
            arguments(&["--verbose", "a"]),
            Err(NoticesError::UnknownOption { .. })
        ));
        assert!(matches!(
            arguments(&["a", "b"]),
            Err(NoticesError::TwoOutputs { .. })
        ));
    }
}
