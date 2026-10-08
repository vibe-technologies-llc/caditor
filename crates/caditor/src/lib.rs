#![recursion_limit = "256"]

mod about;
mod annotation_layout;
mod annotations;
mod app;
mod appearance;
mod blend_panel;
mod blend_tools;
mod bodies;
mod bodies_tree;
mod body_appearance;
mod body_selection;
mod box_selection;
mod canvas;
mod cli;
mod combine_panel;
mod combine_tools;
mod commands;
mod constraint_trial;
#[cfg(test)]
mod conventions_tests;
pub mod crash;
mod datum_panel;
mod datum_tools;
mod dialog_parts;
mod dimensioning;
mod display;
mod display_style;
mod drag_solver;
mod drawing;
mod drawing_export;
mod drop_target;
mod editing;
mod export;
mod faceting;
mod feature_fields;
mod feature_groups;
mod feature_tree;
mod field;
mod files;
mod filleting;
mod fonts;
#[cfg(feature = "fuzzing")]
pub mod fuzzing;
mod graphics;
mod headless;
mod history;
mod hole_panel;
mod hole_tools;
mod icons;
mod image_export;
mod import;
mod import_options;
mod import_panel;
mod interference;
mod interference_panel;
mod layout;
mod logging;
mod logo;
mod measure;
mod measure_panel;
mod menu_bar;
mod messages;
mod mirror_panel;
mod mirror_tools;
mod mirroring;
mod model;
mod model_properties;
mod modifying;
mod move_manipulator;
mod move_panel;
mod move_tools;
mod offers;
mod offsetting;
mod onboarding;
mod overlay;
mod palette;
mod panels;
mod parameter_table;
mod pattern_panel;
mod pattern_tools;
mod patterning;
mod portal;
mod preferences;
mod principal_tree;
mod projecting;
mod reference_picking;
mod reference_rows;
mod removal;
mod ribbon;
mod samples;
mod saved_views;
mod scale_panel;
mod scale_tools;
mod scene;
mod scene_cache;
mod scene_description;
mod scene_palette;
mod selection;
mod shape_modes;
mod shapes;
mod shell_panel;
mod shell_tools;
mod shortcut_editor;
mod sketch_drag;
mod sketch_placement;
mod sketch_status;
mod sketch_toolbar;
mod sketch_tools;
mod snap;
mod snapshot;
mod solid_panel;
mod solid_tools;
mod split_panel;
mod split_tools;
mod status_bar;
mod toolbar;
mod tracking;
mod tree_row;
mod trimming;
mod typed_point;
#[cfg(test)]
mod ui_tests;
mod undo_history;
mod units;
mod variants;
mod view_cube;
mod viewport;
mod visibility;
mod widgets;
mod window_frame;

use std::{path::PathBuf, process::ExitCode};

use anyhow::{Result, bail};
use caditor_document::Document;
use caditor_file::StorageConfig;
use winit::event_loop::EventLoop;

use crate::{
    app::{App, AppEvent},
    cli::Invocation,
    files::{Files, FilesConfig, NativeDialogs},
    logging::Logging,
    model::{Model, Notice, PanicFlush, Services},
    preferences::Preferences,
};

pub fn run() -> Result<ExitCode> {
    #[cfg(windows)]
    caditor_windows::attach_parent_console();
    let open = match Invocation::parse(std::env::args_os().skip(1)) {
        Invocation::Run { open } => open,
        Invocation::Convert(conversion) => return headless::run(&conversion),
        Invocation::Version => {
            println!("{}", about::version_line());
            return Ok(ExitCode::SUCCESS);
        }
        Invocation::Help => {
            print!("{}", cli::usage());
            return Ok(ExitCode::SUCCESS);
        }
        Invocation::Refused(reason) => bail!(reason),
    };
    let state_dir = caditor_file::state_dir();
    let logging = Logging::start(state_dir.as_deref());
    let result = run_session(open, state_dir, &logging);
    if let Err(error) = &result {
        log::error!("{error:#}");
        logging::show_failure(&logging::failure_text(error, logging.path()));
    }
    logging.end();
    result.map(|()| ExitCode::SUCCESS)
}

fn run_session(open: Option<PathBuf>, state_dir: Option<PathBuf>, logging: &Logging) -> Result<()> {
    let panic_flush = PanicFlush::default();
    crash::protect(&panic_flush, logging.ending());

    if state_dir.is_none() {
        log::warn!("no state directory, so unsaved work cannot be protected against a crash");
    }
    let recovery_dir = state_dir.as_deref().map(caditor_file::recovery_dir);
    let stopped_before = state_dir
        .as_deref()
        .and_then(|dir| logging.earlier_unexpected_end(dir));

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let mut model = Model::new(
        Document::default(),
        Services {
            make_waker: app::waker_factory(event_loop.create_proxy()),
            storage: StorageConfig {
                recovery_dir: recovery_dir.clone(),
                ..StorageConfig::default()
            },
            panic_flush,
        },
    );
    if let Some(log) = &stopped_before {
        model.set_notice(Notice::info(logging::unexpected_end_notice(log)));
    }
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
    let (settings, problem) = config_dir
        .as_deref()
        .map(caditor_file::Settings::load_reporting)
        .unwrap_or_default();
    if let Some(problem) = &problem {
        model.set_notice(preferences::unreadable_notice(problem));
    }
    let preferences = Preferences::from_settings(settings);
    let mut app = App::new(model, files, preferences, open, event_loop.create_proxy());
    event_loop.run_app(&mut app)?;
    app.finish()
}
