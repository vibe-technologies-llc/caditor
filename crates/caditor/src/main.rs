mod annotation_layout;
mod annotations;
mod app;
mod blend_panel;
mod blend_tools;
mod bodies;
mod commands;
mod datum_panel;
mod datum_tools;
mod drawing;
mod editing;
mod export;
mod feature_tree;
mod field;
mod files;
mod history;
mod import;
mod model;
mod overlay;
mod palette;
mod panels;
mod parameter_table;
mod preferences;
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
mod toolbar;
#[cfg(test)]
mod ui_tests;
mod units;
mod view_cube;
mod viewport;

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use caditor_document::{Document, FeatureKind};
use caditor_expression::Expression;
use caditor_file::StorageConfig;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use winit::event_loop::EventLoop;

use crate::{
    app::{App, AppEvent},
    files::{Files, FilesConfig, NativeDialogs},
    model::{Model, PanicFlush, Services},
    preferences::Preferences,
};

const PANIC_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let panic_flush = PanicFlush::default();
    install_panic_hook(Arc::clone(&panic_flush));

    let state_dir = caditor_file::state_dir();
    if state_dir.is_none() {
        log::warn!("no state directory, so unsaved work cannot be protected against a crash");
    }
    let recovery_dir = state_dir.as_deref().map(caditor_file::recovery_dir);

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let model = Model::new(
        sample_document()?,
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
    let open = std::env::args_os().nth(1).map(PathBuf::from);
    let mut app = App::new(model, files, preferences, open);
    event_loop.run_app(&mut app)?;
    app.finish()
}

fn install_panic_hook(panic_flush: PanicFlush) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let flusher = panic_flush
            .try_lock_for(PANIC_FLUSH_TIMEOUT)
            .and_then(|flusher| flusher.clone());
        if let Some(flusher) = flusher
            && !flusher.flush(PANIC_FLUSH_TIMEOUT)
        {
            log::error!("could not flush the recovery journal before the crash");
        }
        previous(info);
    }));
}

fn sample_document() -> Result<Document> {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sample model");
    let width = transaction.parse("40 mm")?;
    transaction.add_parameter("width", width);
    let height = transaction.parse("width / 2")?;
    transaction.add_parameter("height", height);

    let base = dimensioned_line(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Constraint::Horizontal,
        transaction.parse("width")?,
    )?;
    transaction.add_feature("Base sketch", FeatureKind::from(base));
    let side = dimensioned_line(
        Plane::XZ,
        Point2::new(0.0, 20.0),
        Constraint::Vertical,
        transaction.parse("height")?,
    )?;
    transaction.add_feature("Side sketch", FeatureKind::from(side));

    document.apply(transaction.finish())?;
    Ok(document)
}

fn dimensioned_line(
    plane: Plane,
    end: Point2,
    direction: fn(EntityId) -> Constraint,
    value: Expression,
) -> Result<Sketch> {
    let mut sketch = Sketch::new(plane);
    let line = sketch.add_line(Point2::ZERO, end);
    sketch.add_constraint(direction(line))?;
    if let Some((from, to)) = endpoints(&sketch, line) {
        sketch.add_constraint(Constraint::Distance { from, to, value })?;
    }
    Ok(sketch)
}

fn endpoints(sketch: &Sketch, line: EntityId) -> Option<(EntityId, EntityId)> {
    match sketch.entity(line)? {
        Entity::Line { start, end } => Some((*start, *end)),
        _ => None,
    }
}
