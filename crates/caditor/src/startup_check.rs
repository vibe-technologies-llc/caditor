use std::{env, time::Duration};

use caditor_expression::Expression;

use crate::{
    files::Files,
    model::{Action, Model},
};

pub const VARIABLE: &str = "CADITOR_STARTUP_CHECK";
pub const FIRST_FRAME: &str = "the first frame was drawn";
pub const EDIT_JOURNALLED: &str = "startup check: an edit is in the recovery journal";
pub const PARAMETER: &str = "startup_check";
const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub struct StartupCheck {
    edit: bool,
    drawn: bool,
    edited: bool,
}

impl StartupCheck {
    pub fn from_environment() -> Self {
        Self {
            edit: env::var_os(VARIABLE).is_some_and(|value| !value.is_empty()),
            ..Self::default()
        }
    }

    pub fn after_frame(&mut self, model: &mut Model, files: &Files) {
        if !self.drawn {
            self.drawn = true;
            log::info!("{FIRST_FRAME}");
        }
        if self.edit && !self.edited && !files.is_blocking() {
            self.edited = true;
            edit_and_flush(model);
        }
    }
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
