mod about;
mod annotation_layout;
mod annotations;
mod app;
mod appearance;
mod blend_panel;
mod blend_tools;
mod bodies;
mod cli;
mod commands;
mod datum_panel;
mod datum_tools;
mod drawing;
mod editing;
mod export;
mod feature_tree;
mod field;
mod files;
mod fonts;
mod history;
mod icons;
mod import;
mod menu_bar;
mod model;
mod onboarding;
mod overlay;
mod palette;
mod panels;
mod parameter_table;
mod preferences;
mod samples;
mod scene;
mod selection;
mod shell_panel;
mod shell_tools;
mod shortcut_editor;
mod sketch_placement;
mod sketch_status;
mod sketch_toolbar;
mod sketch_tools;
mod snap;
mod solid_panel;
mod solid_tools;
mod status_bar;
mod toolbar;
mod typed_point;
#[cfg(test)]
mod ui_tests;
mod units;
mod view_cube;
mod viewport;
mod widgets;

use std::{sync::Arc, thread, time::Duration};

use anyhow::{Result, bail};
use caditor_document::Document;
use caditor_file::StorageConfig;
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
    low_level::emulate_default_handler,
};
use winit::event_loop::EventLoop;

use crate::{
    app::{App, AppEvent},
    cli::Invocation,
    files::{Files, FilesConfig, NativeDialogs},
    model::{Model, PanicFlush, Services},
    preferences::Preferences,
};

const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
const TERMINATION_SIGNALS: [i32; 3] = [SIGTERM, SIGHUP, SIGINT];
const SIGNAL_EXIT_BASE: i32 = 128;

fn main() -> Result<()> {
    let open = match Invocation::parse(std::env::args_os().skip(1)) {
        Invocation::Run { open } => open,
        Invocation::Version => {
            println!("{}", about::version_line());
            return Ok(());
        }
        Invocation::Help => {
            print!("{}", cli::usage());
            return Ok(());
        }
        Invocation::Refused(reason) => bail!(reason),
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let panic_flush = PanicFlush::default();
    install_panic_hook(Arc::clone(&panic_flush));
    flush_on_termination(Arc::clone(&panic_flush));

    let state_dir = caditor_file::state_dir();
    if state_dir.is_none() {
        log::warn!("no state directory, so unsaved work cannot be protected against a crash");
    }
    let recovery_dir = state_dir.as_deref().map(caditor_file::recovery_dir);

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let model = Model::new(
        Document::default(),
        Services {
            make_waker: app::waker_factory(event_loop.create_proxy()),
            storage: StorageConfig {
                recovery_dir: recovery_dir.clone(),
            },
            panic_flush,
        },
    );
    let config_dir = caditor_file::config_dir();
    let files = Files::new(
        FilesConfig {
            state_dir,
            recovery_dir,
            config_dir: config_dir.clone(),
        },
        Box::new(NativeDialogs),
        app::waker_factory(event_loop.create_proxy()),
    );
    let preferences = Preferences::from_settings(
        config_dir
            .as_deref()
            .map(caditor_file::Settings::load)
            .unwrap_or_default(),
    );
    let mut app = App::new(model, files, preferences, open);
    event_loop.run_app(&mut app)?;
    app.finish()
}

fn install_panic_hook(panic_flush: PanicFlush) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        flush_journal(&panic_flush);
        previous(info);
    }));
}

fn flush_on_termination(panic_flush: PanicFlush) {
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
