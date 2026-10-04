use std::{ffi::OsString, path::PathBuf};

use crate::about;

#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    Run { open: Option<PathBuf> },
    Version,
    Help,
    Refused(String),
}

impl Invocation {
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Self {
        let mut paths = Vec::new();
        let mut options_ended = false;
        for argument in arguments {
            if options_ended {
                paths.push(argument);
                continue;
            }
            match argument.to_str() {
                Some("--version" | "-V") => return Self::Version,
                Some("--help" | "-h") => return Self::Help,
                Some("--") => options_ended = true,
                Some(option) if option.starts_with('-') && option.len() > 1 => {
                    return Self::Refused(format!(
                        "unknown option {option}; run {} --help to see what it accepts",
                        about::NAME
                    ));
                }
                _ => paths.push(argument),
            }
        }
        if paths.len() > 1 {
            return Self::Refused(format!(
                "{} opens one model at a time; give it a single file",
                about::NAME
            ));
        }
        Self::Run {
            open: paths.pop().map(PathBuf::from),
        }
    }
}

pub fn usage() -> String {
    format!(
        "{}\nParametric CAD.\n\nUsage: {} [OPTIONS] [MODEL]\n\nArguments:\n  [MODEL]  \
         A .caditor model to open, or a .dxf or .step file to import\n\nOptions:\n  -h, --help     Show this help\n  -V, --version  \
         Show the version\n",
        about::version_line(),
        about::NAME
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Invocation {
        Invocation::parse(arguments.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_start_empty() {
        assert_eq!(parse(&[]), Invocation::Run { open: None });
    }

    #[test]
    fn a_path_is_opened() {
        assert_eq!(
            parse(&["/home/someone/part.caditor"]),
            Invocation::Run {
                open: Some(PathBuf::from("/home/someone/part.caditor"))
            }
        );
    }

    #[test]
    fn version_and_help_win_over_paths() {
        assert_eq!(parse(&["part.caditor", "--version"]), Invocation::Version);
        assert_eq!(parse(&["-V"]), Invocation::Version);
        assert_eq!(parse(&["--help", "part.caditor"]), Invocation::Help);
        assert_eq!(parse(&["-h"]), Invocation::Help);
    }

    #[test]
    fn a_path_after_the_separator_may_look_like_an_option() {
        assert_eq!(
            parse(&["--", "-odd.caditor"]),
            Invocation::Run {
                open: Some(PathBuf::from("-odd.caditor"))
            }
        );
        assert_eq!(
            parse(&["--", "--version"]),
            Invocation::Run {
                open: Some(PathBuf::from("--version"))
            }
        );
    }

    #[test]
    fn a_lone_dash_is_a_path() {
        assert_eq!(
            parse(&["-"]),
            Invocation::Run {
                open: Some(PathBuf::from("-"))
            }
        );
    }

    #[test]
    fn unknown_options_and_several_paths_are_refused() {
        assert_eq!(
            parse(&["--frobnicate"]),
            Invocation::Refused(
                "unknown option --frobnicate; run caditor --help to see what it accepts".to_owned()
            )
        );
        assert!(matches!(
            parse(&["a.caditor", "b.caditor"]),
            Invocation::Refused(_)
        ));
    }

    #[test]
    fn usage_names_the_version() {
        assert!(usage().starts_with(&about::version_line()));
    }
}
