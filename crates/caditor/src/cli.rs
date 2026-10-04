use std::{ffi::OsString, path::PathBuf};

use caditor_file::MeshResolution;

use crate::{about, headless::Conversion};

#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    Run { open: Option<PathBuf> },
    Convert(Conversion),
    Version,
    Help,
    Refused(String),
}

impl Invocation {
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Self {
        let mut paths = Vec::new();
        let mut options_ended = false;
        let mut output = None;
        let mut resolution = None;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            if options_ended {
                paths.push(argument);
                continue;
            }
            match argument.to_str() {
                Some("--version" | "-V") => return Self::Version,
                Some("--help" | "-h") => return Self::Help,
                Some("--") => options_ended = true,
                Some("--export") => match arguments.next() {
                    Some(path) => output = Some(PathBuf::from(path)),
                    None => return Self::Refused(needs_a_value("--export", "an output file")),
                },
                Some("--resolution") => {
                    let chosen = arguments.next();
                    match chosen
                        .as_deref()
                        .and_then(|name| name.to_str())
                        .and_then(resolution_named)
                    {
                        Some(named) => resolution = Some(named),
                        None => {
                            return Self::Refused(
                                "--resolution takes coarse, standard or fine".to_owned(),
                            );
                        }
                    }
                }
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
        let model = paths.pop().map(PathBuf::from);
        match (output, model) {
            (Some(output), Some(model)) => Self::Convert(Conversion {
                model,
                output,
                resolution: resolution.unwrap_or_default(),
            }),
            (Some(_), None) => Self::Refused("--export needs the model to export".to_owned()),
            (None, _) if resolution.is_some() => {
                Self::Refused("--resolution only applies to --export".to_owned())
            }
            (None, open) => Self::Run { open },
        }
    }
}

fn needs_a_value(option: &str, what: &str) -> String {
    format!("{option} needs {what} after it")
}

fn resolution_named(name: &str) -> Option<MeshResolution> {
    MeshResolution::ALL
        .into_iter()
        .find(|resolution| resolution.name().eq_ignore_ascii_case(name))
}

pub fn usage() -> String {
    format!(
        "{}\nParametric CAD.\n\nUsage: {} [OPTIONS] [MODEL]\n\nArguments:\n  [MODEL]  \
         A .caditor model to open, or a .dxf or .step file to import\n\nOptions:\n  \
         --export <FILE>         Write MODEL's bodies to FILE (.stl, .3mf, .step or .stp) and \
         exit, without opening a window\n  --resolution <NAME>     Mesh quality for STL and 3MF: \
         coarse, standard (the default) or fine\n  -h, --help              Show this help\n  -V, \
         --version           Show the version\n",
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
    fn export_names_the_output_and_the_model_and_takes_a_resolution() {
        assert_eq!(
            parse(&["--export", "out.step", "part.caditor"]),
            Invocation::Convert(Conversion {
                model: PathBuf::from("part.caditor"),
                output: PathBuf::from("out.step"),
                resolution: MeshResolution::Standard,
            })
        );
        assert_eq!(
            parse(&[
                "part.caditor",
                "--resolution",
                "Fine",
                "--export",
                "out.stl"
            ]),
            Invocation::Convert(Conversion {
                model: PathBuf::from("part.caditor"),
                output: PathBuf::from("out.stl"),
                resolution: MeshResolution::Fine,
            })
        );
    }

    #[test]
    fn export_without_what_it_needs_is_refused_in_words() {
        let refused = |arguments: &[&str]| match parse(arguments) {
            Invocation::Refused(reason) => reason,
            other => panic!("{other:?}"),
        };

        assert_eq!(
            refused(&["part.caditor", "--export"]),
            "--export needs an output file after it"
        );
        assert_eq!(
            refused(&["--export", "out.step"]),
            "--export needs the model to export"
        );
        assert_eq!(
            refused(&["--resolution", "huge", "--export", "o.stl", "p.caditor"]),
            "--resolution takes coarse, standard or fine"
        );
        assert_eq!(
            refused(&["--resolution", "fine", "part.caditor"]),
            "--resolution only applies to --export"
        );
    }

    #[test]
    fn usage_names_the_version() {
        assert!(usage().starts_with(&about::version_line()));
    }
}
