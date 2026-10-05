use std::{backtrace::Backtrace, sync::Arc, time::Duration};

use caditor_file::SessionLog;

pub use crate::model::PanicFlush;

const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(unix)]
const SIGNAL_EXIT_BASE: i32 = 128;
#[cfg(windows)]
const CONSOLE_STOP_EXIT: i32 = -1_073_741_510;

#[cfg(windows)]
static STOPPING: std::sync::OnceLock<(PanicFlush, Option<Arc<SessionLog>>)> =
    std::sync::OnceLock::new();

pub fn protect(panic_flush: &PanicFlush, log: Option<Arc<SessionLog>>) {
    install_panic_hook(PanicFlush::clone(panic_flush));
    flush_on_termination(PanicFlush::clone(panic_flush), log);
}

fn install_panic_hook(panic_flush: PanicFlush) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        flush_journal(&panic_flush);
        log::error!("caditor panicked: {info}\n{}", Backtrace::force_capture());
        previous(info);
    }));
}

#[cfg(unix)]
fn flush_on_termination(panic_flush: PanicFlush, log: Option<Arc<SessionLog>>) {
    use std::thread;

    use signal_hook::{
        consts::{SIGHUP, SIGINT, SIGTERM},
        iterator::Signals,
        low_level::emulate_default_handler,
    };

    let mut signals = match Signals::new([SIGTERM, SIGHUP, SIGINT]) {
        Ok(signals) => signals,
        Err(error) => {
            log::warn!("unsaved work cannot be flushed when caditor is told to stop: {error}");
            return;
        }
    };
    let spawned = thread::Builder::new()
        .name("signals".to_owned())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                log::info!("stopping on signal {signal}");
                flush_journal(&panic_flush);
                if let Some(log) = &log {
                    log.end();
                }
                if let Err(error) = emulate_default_handler(signal) {
                    log::error!("could not stop on signal {signal}: {error}");
                }
                std::process::exit(SIGNAL_EXIT_BASE.saturating_add(signal));
            }
        });
    if let Err(error) = spawned {
        log::warn!("unsaved work cannot be flushed when caditor is told to stop: {error}");
    }
}

#[cfg(windows)]
fn flush_on_termination(panic_flush: PanicFlush, log: Option<Arc<SessionLog>>) {
    if STOPPING.set((panic_flush, log)).is_err() {
        return;
    }
    let stopping = || {
        flush_before_stopping("the console it runs in is closing");
        std::process::exit(CONSOLE_STOP_EXIT);
    };
    if let Err(error) = caditor_windows::on_console_close(stopping) {
        log::warn!("unsaved work cannot be flushed when the console closes: {error}");
    }
}

#[cfg(windows)]
pub fn flush_when_the_session_ends(window: std::num::NonZeroIsize) {
    let ending = || flush_before_stopping("Windows is ending the session");
    if let Err(error) = caditor_windows::on_session_end(window, ending) {
        log::warn!("unsaved work cannot be flushed when Windows signs out: {error}");
    }
}

#[cfg(windows)]
fn flush_before_stopping(reason: &str) {
    log::info!("stopping because {reason}");
    if let Some((panic_flush, log)) = STOPPING.get() {
        flush_journal(panic_flush);
        if let Some(log) = log {
            log.end();
        }
    }
}

fn flush_journal(panic_flush: &PanicFlush) {
    let flusher = panic_flush
        .try_lock_for(FLUSH_TIMEOUT)
        .and_then(|flusher| flusher.clone());
    if let Some(flusher) = flusher
        && !flusher.flush(FLUSH_TIMEOUT)
    {
        log::error!("could not flush the recovery journal before stopping");
    }
}
