use std::{backtrace::Backtrace, sync::Arc, thread, time::Duration};

use caditor_file::SessionLog;
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
    low_level::emulate_default_handler,
};

pub use crate::model::PanicFlush;

const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
const TERMINATION_SIGNALS: [i32; 3] = [SIGTERM, SIGHUP, SIGINT];
const SIGNAL_EXIT_BASE: i32 = 128;

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

fn flush_on_termination(panic_flush: PanicFlush, log: Option<Arc<SessionLog>>) {
    let mut signals = match Signals::new(TERMINATION_SIGNALS) {
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
