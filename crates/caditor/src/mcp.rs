mod files;
pub mod json_rpc;
mod reading;
mod references;
mod resources;
mod server;
mod session;
pub mod tools;
mod writing;

#[cfg(test)]
mod end_to_end_tests;
#[cfg(test)]
mod tests;

use std::{
    io::{self, BufReader},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use caditor_file::{SessionLog, StorageConfig};

use crate::{
    cli::Serving,
    crash,
    logging::Logging,
    mcp::session::{Opening, Session},
    model::{PanicFlush, display_name},
    preferences::Preferences,
};

const CLOSING_PATIENCE: Duration = Duration::from_secs(30);

pub fn run(serving: &Serving) -> Result<ExitCode> {
    let state_dir = caditor_file::state_dir();
    let logging = Logging::start(state_dir.as_deref());
    let served = serve_stdio(serving, state_dir, logging.ending());
    if let Err(error) = &served {
        log::error!("{error:#}");
    }
    logging.end();
    served.map(|()| ExitCode::SUCCESS)
}

fn serve_stdio(
    serving: &Serving,
    state_dir: Option<PathBuf>,
    ending: Option<Arc<SessionLog>>,
) -> Result<()> {
    let panic_flush = PanicFlush::default();
    crash::protect(&panic_flush, ending);
    let retention = caditor_file::config_dir()
        .map(|dir| caditor_file::Settings::load(&dir))
        .map(|settings| Preferences::from_settings(settings).retention)
        .unwrap_or_default();
    let session = Session::open(Opening {
        model: serving.model.clone(),
        folders: serving.folders.clone(),
        storage: StorageConfig {
            recovery_dir: state_dir.as_deref().map(caditor_file::recovery_dir),
            ..StorageConfig::default()
        },
        panic_flush,
        retention,
    })?;
    log::info!(
        "serving “{}” over the Model Context Protocol on standard input and output",
        display_name(Some(session.model()))
    );
    let session = server::serve(BufReader::new(io::stdin()), io::stdout(), session);
    if let Some(closing) = session.close()
        && !closing.wait(CLOSING_PATIENCE)
    {
        log::warn!("the recovery journal was not closed in time; it is kept for recovery");
    }
    Ok(())
}
