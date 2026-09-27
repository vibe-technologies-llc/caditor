mod app;
mod feature_tree;
mod field;
mod model;
mod overlay;
mod panels;
mod parameter_table;
mod scene;
mod selection;
mod toolbar;
#[cfg(test)]
mod ui_tests;
mod view_cube;
mod viewport;

use anyhow::Result;
use caditor_document::{Document, FeatureKind};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use winit::event_loop::EventLoop;

use crate::{
    app::{App, AppEvent},
    model::Model,
};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let model = Model::new(
        sample_document()?,
        app::waker_factory(event_loop.create_proxy()),
    );
    let mut app = App::new(model);
    event_loop.run_app(&mut app)?;
    app.finish()
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
    );
    transaction.add_feature("Base sketch", FeatureKind::Sketch(base));
    let side = dimensioned_line(
        Plane::XZ,
        Point2::new(0.0, 20.0),
        Constraint::Vertical,
        transaction.parse("height")?,
    );
    transaction.add_feature("Side sketch", FeatureKind::Sketch(side));

    document.apply(transaction.finish())?;
    Ok(document)
}

fn dimensioned_line(
    plane: Plane,
    end: Point2,
    direction: fn(EntityId) -> Constraint,
    value: Expression,
) -> Sketch {
    let mut sketch = Sketch::new(plane);
    let line = sketch.add_line(Point2::ZERO, end);
    sketch.add_constraint(direction(line));
    if let Some((from, to)) = endpoints(&sketch, line) {
        sketch.add_constraint(Constraint::Distance { from, to, value });
    }
    sketch
}

fn endpoints(sketch: &Sketch, line: EntityId) -> Option<(EntityId, EntityId)> {
    match sketch.entity(line)? {
        Entity::Line { start, end } => Some((*start, *end)),
        Entity::Point(_) => None,
    }
}
