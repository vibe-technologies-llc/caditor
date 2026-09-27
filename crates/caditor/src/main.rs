mod app;
mod overlay;
mod panels;

use anyhow::Result;
use caditor_document::{Document, FeatureKind};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Sketch};
use winit::event_loop::EventLoop;

use crate::app::App;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::new()?;
    let mut app = App::new(sample_document());
    event_loop.run_app(&mut app)?;
    app.finish()
}

fn sample_document() -> Document {
    let mut document = Document::default();
    document.set_parameter("width", 40.0);
    document.set_parameter("height", 20.0);

    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    sketch.add_constraint(Constraint::Horizontal(base));
    document.add_feature("Base sketch", FeatureKind::Sketch(sketch));

    document
}
