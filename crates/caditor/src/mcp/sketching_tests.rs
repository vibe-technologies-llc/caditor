use std::{
    f64::consts::{FRAC_PI_2, PI},
    path::PathBuf,
};

use caditor_document::{
    Datum, DatumFrame, DatumPlane, Document, FeatureKind, PlaneReference, PrincipalAxis,
    PrincipalPlane,
};
use caditor_expression::{Expression, Unit};
use caditor_sketch::Sketch;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::tests::{Client, opening};

const HEIGHT: f64 = 5.0;

fn started(folder: &TempDir) -> Client {
    let mut client = Client::start(opening(folder.path().join("part.caditor")));
    client.initialize("2025-06-18");
    client
}

fn new_sketch(client: &mut Client) -> Value {
    client.tool("add_sketch", json!({ "plane": "XY" }))["sketch"]["id"].clone()
}

fn draw(client: &mut Client, sketch: &Value, item: Value) -> Value {
    let drawn = client.tool(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [item] }),
    );
    assert_eq!(drawn["failing"], json!([]), "{drawn}");
    drawn["drawn"][0].clone()
}

fn volume_of_extrusion(client: &mut Client, sketch: &Value) -> f64 {
    let extruded = client.tool(
        "add_extrusion",
        json!({ "sketch": sketch, "distance": format!("{HEIGHT} mm") }),
    );
    assert_eq!(extruded["failing"], json!([]), "{extruded}");
    let bodies = client.tool("list_bodies", json!({}));
    bodies["bodies"][0]["volume"].as_f64().unwrap()
}

fn assert_area(volume: f64, area: f64) {
    let expected = area * HEIGHT;
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "{volume} != {expected}"
    );
}

fn entity_of<'a>(sketch: &'a Value, id: &Value) -> &'a Value {
    sketch["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| &entity["id"] == id)
        .unwrap_or_else(|| panic!("no entity {id} in {sketch}"))
}

#[test]
fn an_arc_is_drawn_about_a_centre_or_through_three_points() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let counter_clockwise = draw(
        &mut client,
        &sketch,
        json!({ "kind": "arc", "center": [0, 0], "start": [10, 0], "end": [0, 3] }),
    );
    let clockwise = draw(
        &mut client,
        &sketch,
        json!({ "kind": "arc", "center": [0, 0], "start": [10, 0], "end": [0, 3], "clockwise": true }),
    );
    let through = draw(
        &mut client,
        &sketch,
        json!({ "kind": "arc", "start": [-10, 0], "end": [10, 0], "through": [0, 10] }),
    );
    let described = client.tool("get_sketch", json!({ "sketch": sketch }));

    let first = entity_of(&described, &counter_clockwise["id"]);
    let second = entity_of(&described, &clockwise["id"]);
    assert_eq!(first["kind"], "arc");
    assert_eq!(first["start"], counter_clockwise["start"]);
    assert_eq!(second["start"], clockwise["start"]);
    assert_ne!(counter_clockwise["start"], clockwise["start"]);

    let ends = |arc: &Value| {
        let point = |id: &Value| entity_of(&described, id)["at"].clone();
        (point(&arc["start"]), point(&arc["end"]))
    };
    assert_eq!(ends(&counter_clockwise).0, json!([10.0, 0.0]));
    assert_eq!(ends(&clockwise).1, json!([10.0, 0.0]));
    let (end_x, end_y) = {
        let end = ends(&counter_clockwise).1;
        (end[0].as_f64().unwrap(), end[1].as_f64().unwrap())
    };
    assert!(
        end_x.abs() < 1e-9 && (end_y - 10.0).abs() < 1e-9,
        "{end_x} {end_y}"
    );
    assert_eq!(ends(&through).0, json!([10.0, 0.0]));
    assert_eq!(
        entity_of(&described, &through["center"])["at"],
        json!([0.0, 0.0])
    );

    let in_line = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [
            { "kind": "arc", "start": [0, 0], "end": [10, 0], "through": [5, 0] }
        ] }),
    );
    let both = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [
            { "kind": "arc", "center": [0, 0], "start": [1, 0], "end": [0, 1], "through": [1, 1] }
        ] }),
    );
    let flat = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [
            { "kind": "arc", "center": [0, 0], "start": [1, 0], "end": [5, 0] }
        ] }),
    );
    client.finish();

    assert!(in_line.contains("lie in a line"), "{in_line}");
    assert!(both.contains("not both and not neither"), "{both}");
    assert!(flat.contains("has no size"), "{flat}");
}

#[test]
fn a_slot_is_joined_tangent_and_makes_its_own_region() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let slot = draw(
        &mut client,
        &sketch,
        json!({ "kind": "slot", "from": [0, 0], "to": [20, 0], "width": 10 }),
    );
    let volume = volume_of_extrusion(&mut client, &sketch);
    let described = client.tool("get_sketch", json!({ "sketch": sketch }));
    let tilted = draw(
        &mut client,
        &sketch,
        json!({ "kind": "slot", "from": [0, 20], "to": [10, 30], "width": 4 }),
    );
    let thin = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [
            { "kind": "slot", "from": [0, 0], "to": [20, 0], "width": 0 }
        ] }),
    );
    client.finish();

    assert_area(volume, 20.0 * 10.0 + PI * 25.0);
    assert_eq!(slot["arcs"].as_array().unwrap().len(), 2);
    assert_eq!(slot["lines"].as_array().unwrap().len(), 2);
    let descriptions: Vec<&str> = described["constraints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|constraint| constraint["description"].as_str().unwrap())
        .collect();
    let counted = |word: &str| {
        descriptions
            .iter()
            .filter(|description| description.to_lowercase().contains(word))
            .count()
    };
    assert_eq!(counted("tangent"), 4, "{descriptions:?}");
    assert_eq!(counted("horizontal"), 1, "{descriptions:?}");
    assert_eq!(tilted["centers"].as_array().unwrap().len(), 2);
    assert!(thin.contains("width"), "{thin}");
}

#[test]
fn an_arc_slot_follows_an_arc_about_its_centre() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    draw(
        &mut client,
        &sketch,
        json!({ "kind": "arc_slot", "center": [0, 0], "from": [20, 0], "to": [0, 20], "width": 6 }),
    );
    let volume = volume_of_extrusion(&mut client, &sketch);
    let too_wide = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [
            { "kind": "arc_slot", "center": [0, 0], "from": [2, 0], "to": [0, 2], "width": 10 }
        ] }),
    );
    client.finish();

    let band = FRAC_PI_2 * 20.0 * 6.0;
    assert_area(volume, band + PI * 9.0);
    assert!(too_wide.contains("narrower"), "{too_wide}");
}

#[test]
fn a_polygon_stands_on_a_construction_circle_with_equal_sides() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let hexagon = draw(
        &mut client,
        &sketch,
        json!({ "kind": "polygon", "center": [0, 0], "corner": [10, 0] }),
    );
    let volume = volume_of_extrusion(&mut client, &sketch);
    let described = client.tool("get_sketch", json!({ "sketch": sketch }));
    let dimensioned = client.tool(
        "add_sketch_constraints",
        json!({ "sketch": sketch, "constraints": [
            { "kind": "radius", "curve": hexagon["circle"], "value": "12 mm" }
        ] }),
    );
    client.finish();

    assert_area(volume, 3.0 * 3f64.sqrt() / 2.0 * 100.0);
    assert_eq!(hexagon["sides"], 6);
    assert_eq!(hexagon["lines"].as_array().unwrap().len(), 6);
    assert_eq!(hexagon["corners"].as_array().unwrap().len(), 6);
    assert_eq!(
        entity_of(&described, &hexagon["circle"])["construction"],
        true
    );
    assert_eq!(dimensioned["failing"], json!([]), "{dimensioned}");
}

#[test]
fn a_polygon_around_a_side_middle_has_its_first_side_there() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let square = draw(
        &mut client,
        &sketch,
        json!({ "kind": "polygon", "center": [0, 0], "side_middle": [5, 0], "sides": 4 }),
    );
    let volume = volume_of_extrusion(&mut client, &sketch);
    let neither = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [{ "kind": "polygon", "center": [0, 0] }] }),
    );
    let two_sides = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [
            { "kind": "polygon", "center": [0, 0], "corner": [1, 0], "sides": 2 }
        ] }),
    );
    client.finish();

    assert_area(volume, 100.0);
    assert!(square["inscribed_circle"].is_u64(), "{square}");
    assert!(neither.contains("corner"), "{neither}");
    assert!(two_sides.contains("3 to 64 sides"), "{two_sides}");
}

#[test]
fn a_spline_passes_through_its_points_or_is_shaped_by_them() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let ring = draw(
        &mut client,
        &sketch,
        json!({ "kind": "spline", "closed": true, "points": [[10, 0], [0, 10], [-10, 0], [0, -10]] }),
    );
    let volume = volume_of_extrusion(&mut client, &sketch);
    let open = draw(
        &mut client,
        &sketch,
        json!({ "kind": "spline", "form": "control", "points": [[20, 0], [30, 10], [40, 0]] }),
    );
    let described = client.tool("get_sketch", json!({ "sketch": sketch }));
    let short = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [
            { "kind": "spline", "closed": true, "points": [[0, 0], [1, 1]] }
        ] }),
    );
    client.finish();

    let area = volume / HEIGHT;
    assert!(area > 250.0 && area < 330.0, "{area}");
    assert_eq!(ring["form"], "fit");
    assert_eq!(open["form"], "control");
    assert_eq!(entity_of(&described, &ring["id"])["kind"], "spline");
    assert_eq!(
        entity_of(&described, &open["id"])["points"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(short.contains("at least 3 points"), "{short}");
}

#[test]
fn an_ellipse_has_a_major_end_and_a_minor_radius() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let ellipse = draw(
        &mut client,
        &sketch,
        json!({ "kind": "ellipse", "center": [0, 0], "major_end": [10, 0], "minor_radius": 5 }),
    );
    let volume = volume_of_extrusion(&mut client, &sketch);
    let described = client.tool("get_sketch", json!({ "sketch": sketch }));
    let flat = client.tool_error(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "items": [
            { "kind": "ellipse", "center": [0, 0], "major_end": [10, 0], "minor_radius": 0 }
        ] }),
    );
    client.finish();

    assert_area(volume, PI * 10.0 * 5.0);
    assert_eq!(entity_of(&described, &ellipse["id"])["kind"], "ellipse");
    assert!(flat.contains("minor radius"), "{flat}");
}

#[test]
fn shapes_drawn_as_construction_make_no_region() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let sketch = new_sketch(&mut client);

    let drawn = client.tool(
        "add_sketch_geometry",
        json!({ "sketch": sketch, "construction": true, "items": [
            { "kind": "ellipse", "center": [0, 0], "major_end": [10, 0], "minor_radius": 5 }
        ] }),
    );
    let described = client.tool("get_sketch", json!({ "sketch": sketch }));
    client.finish();

    assert_eq!(
        entity_of(&described, &drawn["drawn"][0]["id"])["construction"],
        true
    );
}

fn dimensioned_rectangle(client: &mut Client) -> (Value, Value, Value) {
    let sketch = new_sketch(client);
    let rectangle = draw(
        client,
        &sketch,
        json!({ "kind": "rectangle", "corner": [0, 0], "opposite": [10, 6] }),
    );
    let corners = rectangle["corners"].clone();
    let constrained = client.tool(
        "add_sketch_constraints",
        json!({ "sketch": sketch, "constraints": [
            { "kind": "coincident", "a": corners[0], "b": "origin" },
            { "kind": "distance", "from": corners[0], "to": corners[1], "value": "10 mm" },
            { "kind": "distance", "from": corners[0], "to": corners[3], "value": "6 mm" }
        ] }),
    );
    assert_eq!(constrained["sketch_status"], "Fully constrained");
    (sketch, corners, constrained["constraints"].clone())
}

#[test]
fn a_dimension_changes_to_a_value_with_units_or_an_expression() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let (sketch, _, constraints) = dimensioned_rectangle(&mut client);
    let width = constraints[1].clone();

    client.tool(
        "set_parameter",
        json!({ "name": "base", "expression": "2 cm" }),
    );
    let in_units = client.tool(
        "edit_sketch_constraint",
        json!({ "sketch": sketch, "constraint": width, "value": "3 cm" }),
    );
    let volume_in_units = volume_of_extrusion(&mut client, &sketch);
    let by_parameter = client.tool(
        "edit_sketch_constraint",
        json!({ "sketch": sketch, "constraint": width, "value": "base + 5 mm" }),
    );
    let bodies = client.tool("list_bodies", json!({}));
    let undone = client.tool("undo", json!({}));
    let not_a_dimension = client.tool_error(
        "edit_sketch_constraint",
        json!({ "sketch": sketch, "constraint": constraints[0], "value": "3 mm" }),
    );
    let nothing = client.tool_error(
        "edit_sketch_constraint",
        json!({ "sketch": sketch, "constraint": width }),
    );
    let missing = client.tool_error(
        "edit_sketch_constraint",
        json!({ "sketch": sketch, "constraint": 9999, "value": "3 mm" }),
    );
    let wrong_unit = client.tool_error(
        "edit_sketch_constraint",
        json!({ "sketch": sketch, "constraint": width, "value": "3 deg" }),
    );
    client.finish();

    assert_eq!(in_units["value"], 30.0, "{in_units}");
    assert_eq!(in_units["failing"], json!([]));
    assert_area(volume_in_units, 30.0 * 6.0);
    assert_eq!(by_parameter["expression"], "base + 5 mm");
    assert_eq!(by_parameter["value"], 25.0);
    assert!(bodies["bodies"][0]["volume"].as_f64().unwrap() > 0.0);
    assert_eq!(
        bodies["bodies"][0]["volume"].as_f64().unwrap(),
        25.0 * 6.0 * HEIGHT
    );
    assert!(undone["undone"].as_str().unwrap().starts_with("Edit"));
    assert!(
        not_a_dimension.contains("not a dimension"),
        "{not_a_dimension}"
    );
    assert!(nothing.contains("Say what to change"), "{nothing}");
    assert!(missing.contains("has no constraint 9999"), "{missing}");
    assert!(!wrong_unit.is_empty());
}

#[test]
fn a_dimension_turned_off_becomes_a_reference_and_on_again_drives() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let (sketch, _, constraints) = dimensioned_rectangle(&mut client);
    let width = constraints[1].clone();

    let off = client.tool(
        "edit_sketch_constraint",
        json!({ "sketch": sketch, "constraint": width, "active": false }),
    );
    let loosened = client.tool("get_sketch", json!({ "sketch": sketch }));
    let on = client.tool(
        "edit_sketch_constraint",
        json!({ "sketch": sketch, "constraint": width, "active": true }),
    );
    let again = client.tool(
        "edit_sketch_constraint",
        json!({ "sketch": sketch, "constraint": width, "active": true }),
    );
    let tightened = client.tool("get_sketch", json!({ "sketch": sketch }));
    client.finish();

    assert_eq!(off["active"], false);
    assert_eq!(loosened["degrees_of_freedom"], 1);
    assert_eq!(on["active"], true);
    assert_eq!(tightened["degrees_of_freedom"], 0);
    assert_eq!(again["changed"], "nothing: the model already was that way");
}

#[test]
fn constraints_and_entities_are_removed_with_what_depends_on_them() {
    let folder = TempDir::new().unwrap();
    let mut client = started(&folder);
    let (sketch, corners, constraints) = dimensioned_rectangle(&mut client);

    let released = client.tool(
        "remove_sketch_items",
        json!({ "sketch": sketch, "constraints": [constraints[1]] }),
    );
    let after_dimension = client.tool("get_sketch", json!({ "sketch": sketch }));
    let corner_gone = client.tool(
        "remove_sketch_items",
        json!({ "sketch": sketch, "entities": [corners[2]] }),
    );
    let after_corner = client.tool("get_sketch", json!({ "sketch": sketch }));
    let undone = client.tool("undo", json!({}));
    let reference = client.tool_error(
        "remove_sketch_items",
        json!({ "sketch": sketch, "entities": ["origin"] }),
    );
    let missing = client.tool_error(
        "remove_sketch_items",
        json!({ "sketch": sketch, "constraints": [424242] }),
    );
    let nothing = client.tool_error("remove_sketch_items", json!({ "sketch": sketch }));
    client.finish();

    assert_eq!(released["removed_constraints"], json!([constraints[1]]));
    assert_eq!(released["removed_entities"], json!([]));
    assert_eq!(after_dimension["degrees_of_freedom"], 1);
    assert_eq!(released["change"], "Remove constraints from Sketch 1");

    let removed_lines = corner_gone["removed_entities"].as_array().unwrap();
    assert!(removed_lines.contains(&corners[2]), "{corner_gone}");
    assert!(removed_lines.len() >= 3, "{corner_gone}");
    assert!(
        after_corner["entities"].as_array().unwrap().len()
            < after_dimension["entities"].as_array().unwrap().len()
    );
    assert!(undone["undone"].as_str().unwrap().starts_with("Remove"));
    assert!(reference.contains("origin and axes"), "{reference}");
    assert!(missing.contains("has no constraint 424242"), "{missing}");
    assert!(
        nothing.contains("nothing to draw or constrain"),
        "{nothing}"
    );
}

fn model_with_datums(folder: &TempDir) -> PathBuf {
    let mut document = Document::default();
    let mut transaction = document.transaction("Datums");
    transaction.add_feature(
        "Raised plane",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: Expression::Measure(15.0, Unit::Millimetre),
        })),
    );
    let mut turned = DatumFrame::world();
    turned.x_axis = caditor_document::AxisReference::Principal(PrincipalAxis::Y);
    transaction.add_feature(
        "Turned frame",
        FeatureKind::Datum(Datum::Frame(Box::new(turned))),
    );
    transaction.add_feature(
        "Not a datum",
        FeatureKind::from(Sketch::new(PrincipalPlane::Xy.plane())),
    );
    document.apply(transaction.finish()).unwrap();

    let path = folder.path().join("datums.caditor");
    caditor_file::save(&document, &path, false).unwrap();
    path
}

#[test]
fn a_sketch_lies_on_a_datum_plane() {
    let folder = TempDir::new().unwrap();
    let mut client = Client::start(opening(model_with_datums(&folder)));
    client.initialize("2025-06-18");

    let added = client.tool(
        "add_sketch",
        json!({ "datum": "Raised plane", "name": "On datum" }),
    );
    let sketch = added["sketch"]["id"].clone();
    draw(
        &mut client,
        &sketch,
        json!({ "kind": "rectangle", "corner": [0, 0], "opposite": [10, 10] }),
    );
    let volume = volume_of_extrusion(&mut client, &sketch);
    let described = client.tool("get_sketch", json!({ "sketch": sketch }));
    let bodies = client.tool("list_bodies", json!({}));

    let both = client.tool_error(
        "add_sketch",
        json!({ "datum": "Raised plane", "plane": "XY" }),
    );
    let not_a_plane = client.tool_error("add_sketch", json!({ "datum": "Turned frame" }));
    let not_a_datum = client.tool_error("add_sketch", json!({ "datum": "Not a datum" }));
    client.finish();

    assert_eq!(added["sketch"]["plane"]["origin"], json!([0.0, 0.0, 15.0]));
    assert_eq!(added["failing"], json!([]));
    assert_eq!(described["lies_on"], "Raised plane");
    assert_area(volume, 100.0);
    assert_eq!(bodies["bodies"][0]["bounding_box"]["min"][2], 15.0);
    assert!(both.contains("not both"), "{both}");
    assert!(not_a_plane.contains("not a datum plane"), "{not_a_plane}");
    assert!(not_a_datum.contains("not a datum plane"), "{not_a_datum}");
}

#[test]
fn a_sketch_lies_on_a_plane_of_a_coordinate_system() {
    let folder = TempDir::new().unwrap();
    let mut client = Client::start(opening(model_with_datums(&folder)));
    client.initialize("2025-06-18");

    let added = client.tool(
        "add_sketch",
        json!({ "frame": "Turned frame", "plane": "XY" }),
    );
    let side = client.tool(
        "add_sketch",
        json!({ "frame": "Turned frame", "plane": "XZ" }),
    );
    let described = client.tool("get_sketch", json!({ "sketch": added["sketch"]["id"] }));
    let no_plane = client.tool_error("add_sketch", json!({ "frame": "Turned frame" }));
    let not_a_frame = client.tool_error(
        "add_sketch",
        json!({ "frame": "Raised plane", "plane": "XY" }),
    );
    let both = client.tool_error(
        "add_sketch",
        json!({ "frame": "Turned frame", "datum": "Raised plane", "plane": "XY" }),
    );
    client.finish();

    assert_eq!(added["sketch"]["plane"]["x_axis"], json!([0.0, 1.0, 0.0]));
    assert_eq!(side["sketch"]["plane"]["x_axis"], json!([0.0, 1.0, 0.0]));
    assert_eq!(side["sketch"]["plane"]["y_axis"], json!([0.0, 0.0, 1.0]));
    assert!(
        described["lies_on"]
            .as_str()
            .unwrap()
            .contains("Turned frame")
    );
    assert!(no_plane.contains("give plane"), "{no_plane}");
    assert!(
        not_a_frame.contains("not a coordinate system"),
        "{not_a_frame}"
    );
    assert!(both.contains("not both"), "{both}");
}
