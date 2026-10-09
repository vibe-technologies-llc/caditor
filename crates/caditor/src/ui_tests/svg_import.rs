use caditor_sketch::Entity;
use tempfile::TempDir;

use super::Harness;
use crate::{files::FileCommand, import_options::ImportOptionsCommand};

const PLATE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="40mm" height="20mm" viewBox="0 0 40 20">
  <g inkscape:groupmode="layer" inkscape:label="Outline">
    <rect width="40" height="20"/>
  </g>
  <g inkscape:groupmode="layer" inkscape:label="Holes">
    <circle cx="20" cy="10" r="3"/>
    <text x="0" y="0">M3</text>
  </g>
</svg>"#;

#[test]
fn an_svg_imports_into_a_new_sketch_leaving_out_the_layers_unticked() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let plate = dir.path().join("plate.svg");
    std::fs::write(&plate, PLATE).unwrap();
    let features = harness.document().features().len();

    harness.answer_dialog(Some(plate));
    harness.command(FileCommand::Import { into: None });
    harness.wait_for_import_options("plate.svg");

    assert!(harness.shows("Outline (4 curves)"));
    assert!(harness.shows("Holes (1 curve)"));
    assert!(harness.shows("5 curves drawn, 40.000 mm wide and 20.000 mm high."));

    harness.command(FileCommand::ImportOptions(ImportOptionsCommand::Layer {
        layer: 1,
        included: false,
    }));
    harness.frame();
    harness.click("Import");
    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });

    let sketch = harness.document().features().last().unwrap().id();
    let count = |harness: &Harness, circle: bool| {
        harness
            .sketch(sketch)
            .entities()
            .filter(|(_, entity)| match entity {
                Entity::Circle { .. } => circle,
                Entity::Line { .. } => !circle,
                _ => false,
            })
            .count()
    };
    assert_eq!(harness.document().feature(sketch).unwrap().name, "plate");
    assert_eq!((count(&harness, false), count(&harness, true)), (4, 0));
    assert!(harness.shows(
        "1 text was left out, because sketches hold only points, lines, arcs, circles and \
         splines."
    ));
    assert!(harness.shows("1 curve on a layer you left out was not imported."));
}
