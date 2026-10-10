use std::{
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
    time::Duration,
};

use caditor_expression::Expression;

use crate::{
    files::{FileCommand, Files},
    model::{Action, Model},
    samples::Sample,
};

pub const VARIABLE: &str = "CADITOR_STARTUP_CHECK";
pub const FIRST_FRAME: &str = "the first frame was drawn";
pub const EDIT_JOURNALLED: &str = "startup check: an edit is in the recovery journal";
pub const MODEL_SAVED: &str = "startup check: saved the model to";
pub const PARAMETER: &str = "startup_check";
const RESTORE: &str = "restore";
const SAVE_PREFIX: &str = "save:";
const SAVED_SAMPLE: Sample = Sample::Plate;
const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Off,
    Edit,
    Save(PathBuf),
    Restore,
}

impl Mode {
    fn parse(value: &OsStr) -> Self {
        match value.to_str() {
            None => Self::Edit,
            Some("") => Self::Off,
            Some(RESTORE) => Self::Restore,
            Some(text) => text
                .strip_prefix(SAVE_PREFIX)
                .filter(|path| !path.is_empty())
                .map_or(Self::Edit, |path| Self::Save(PathBuf::from(path))),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Progress {
    #[default]
    Waiting,
    Saving,
    Done,
}

#[derive(Debug, Default)]
pub struct StartupCheck {
    mode: Mode,
    drawn: bool,
    progress: Progress,
}

impl StartupCheck {
    pub fn from_environment() -> Self {
        Self {
            mode: env::var_os(VARIABLE).map_or(Mode::Off, |value| Mode::parse(&value)),
            ..Self::default()
        }
    }

    pub fn after_frame(&mut self, model: &mut Model, files: &mut Files) {
        if !self.drawn {
            self.drawn = true;
            log::info!("{FIRST_FRAME}");
        }
        if self.progress == Progress::Done {
            return;
        }
        match &self.mode {
            Mode::Off => {}
            Mode::Edit if !files.is_blocking() => {
                self.progress = Progress::Done;
                edit_and_flush(model);
            }
            Mode::Save(path) if !files.is_blocking() => {
                self.progress = save_sample(path, self.progress, model);
            }
            Mode::Restore => {
                if restore_offered(model, files) {
                    self.progress = Progress::Done;
                }
            }
            Mode::Edit | Mode::Save(_) => {}
        }
    }
}

fn save_sample(path: &Path, progress: Progress, model: &mut Model) -> Progress {
    match progress {
        Progress::Waiting => match SAVED_SAMPLE.document() {
            Ok(document) => {
                model.replace(document, None, None, false);
                model.save_to(path.to_path_buf());
                Progress::Saving
            }
            Err(error) => {
                log::error!("startup check: the sample could not be built: {error:#}");
                Progress::Done
            }
        },
        Progress::Saving if !model.is_saving() => {
            if model.path() == Some(path) && !model.is_dirty() {
                log::info!("{MODEL_SAVED} {}", path.display());
            } else {
                log::error!(
                    "startup check: the model was not saved to {}",
                    path.display()
                );
            }
            Progress::Done
        }
        Progress::Saving | Progress::Done => progress,
    }
}

fn restore_offered(model: &mut Model, files: &mut Files) -> bool {
    let Some((journal, changes)) = files.first_recoverable() else {
        return false;
    };

    files.perform(FileCommand::Restore(journal), model);

    if model.document().parameter_named(PARAMETER).is_some() {
        log::info!(
            "startup check: restored {changes} changes of {}",
            model.display_name()
        );
    } else {
        log::error!("startup check: the restored document lacks the edit");
    }
    true
}

fn edit_and_flush(model: &mut Model) {
    let mut transaction = model.document().transaction("Startup check");
    transaction.add_parameter(PARAMETER, Expression::Number(1.0));
    let transaction = transaction.finish();

    model.perform(Action::Apply(transaction));

    if model.flush_journal(FLUSH_TIMEOUT) {
        log::info!("{EDIT_JOURNALLED}");
    } else {
        log::error!("startup check: the recovery journal did not take the edit");
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Instant};

    use caditor_document::Document;
    use caditor_file::StorageConfig;
    use tempfile::TempDir;

    use super::*;
    use crate::model::Services;

    #[test]
    fn the_variable_picks_the_check() {
        assert_eq!(Mode::parse(OsStr::new("")), Mode::Off);
        assert_eq!(Mode::parse(OsStr::new("1")), Mode::Edit);
        assert_eq!(Mode::parse(OsStr::new("restore")), Mode::Restore);
        assert_eq!(
            Mode::parse(OsStr::new("save:/tmp/a b/model.caditor")),
            Mode::Save(PathBuf::from("/tmp/a b/model.caditor"))
        );
        assert_eq!(Mode::parse(OsStr::new("save:")), Mode::Edit);
    }

    const RELEASED_WITHIN: Duration = Duration::from_secs(10);

    #[test]
    fn the_edit_reaches_a_journal_a_recovery_scan_restores() {
        let dir = TempDir::new().unwrap();
        let mut model = Model::new(
            Document::default(),
            Services {
                make_waker: Box::new(|| Box::new(|| {})),
                storage: StorageConfig {
                    recovery_dir: Some(dir.path().to_path_buf()),
                    ..StorageConfig::default()
                },
                panic_flush: Arc::default(),
            },
        );

        edit_and_flush(&mut model);
        drop(model);
        let deadline = Instant::now() + RELEASED_WITHIN;
        let mut recovered = caditor_file::scan(Some(dir.path()), &[]);
        while recovered.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            recovered = caditor_file::scan(Some(dir.path()), &[]);
        }

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].changes(), 1);
        assert!(
            recovered[0]
                .editor
                .document()
                .parameter_named(PARAMETER)
                .is_some()
        );
    }
}
