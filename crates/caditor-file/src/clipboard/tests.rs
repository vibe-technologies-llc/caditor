use caditor_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureKind, ParameterValues, RegionChoice,
    SolidFeature, feature_parameters,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Constraint, Entity};

use super::*;

const HERE: &str = "here";
const THERE: &str = "there";

struct Source {
    document: Document,
    sketch: FeatureId,
    clip: SketchClip,
}

fn source() -> Source {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let width = transaction.add_parameter("width", transaction.parse("20 mm").unwrap());
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let circle = sketch.add_circle(Point2::new(5.0, 5.0), 2.0);
    sketch.set_construction(circle, true).unwrap();
    let Some(&Entity::Line { start, end }) = sketch.entity(line) else {
        panic!("a line");
    };
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    sketch
        .add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Parameter(width),
        })
        .unwrap();
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: circle,
            value: Expression::measure(2.0, Unit::Millimetre),
        })
        .unwrap();
    sketch.set_active(radius, false).unwrap();
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let clip = sketch.clip(&[line, circle]).unwrap();
    let feature = transaction.add_feature("Sketch 1", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();
    Source {
        document,
        sketch: feature,
        clip,
    }
}

fn carried_of(
    document: &Document,
    ids: impl IntoIterator<Item = ParameterId>,
) -> CarriedParameters {
    CarriedParameters::of(document, &ParameterValues::evaluate(document), ids)
}

fn copied_text(source: &Source) -> String {
    let parameters = carried_of(
        &source.document,
        source
            .document
            .parameters()
            .iter()
            .map(|parameter| parameter.id()),
    );
    sketch_clipboard_text(&source.clip, &parameters, HERE, source.sketch).unwrap()
}

#[test]
fn sketch_geometry_crosses_the_clipboard_with_its_constraints_and_a_recognisable_header() {
    let source = source();
    let text = copied_text(&source);

    assert!(text.starts_with("caditor clipboard: sketch geometry, version 1\n"));
    assert_eq!(clipboard_kind(&text), Ok(ClipboardKind::SketchGeometry));

    let same = read_sketch_clipboard(&text, &source.document, HERE).unwrap();
    assert_eq!(same.origin, PasteOrigin::ThisDocument);
    assert_eq!(same.sketch, Some(source.sketch));
    assert_eq!(same.inlined, 0);
    assert!(same.notes.is_empty(), "{:?}", same.notes);
    assert_eq!(same.clip.curve_count(), source.clip.curve_count());
    assert_eq!(same.clip.constraint_count(), source.clip.constraint_count());
    assert_eq!(same.clip.centre(), source.clip.centre());

    let elsewhere = Document::default();
    let pasted = read_sketch_clipboard(&text, &elsewhere, THERE).unwrap();
    assert_eq!(pasted.origin, PasteOrigin::Elsewhere);
    assert_eq!(pasted.inlined, 1);
    assert_eq!(pasted.clip.curve_count(), 2);
    assert_eq!(pasted.clip.constraint_count(), 3);
    let mut target = Sketch::new(Plane::XY);
    let items = target.paste(&pasted.clip, Vector2::ZERO).unwrap();
    assert_eq!(items.len(), 2);
    let inactive = target
        .constraints()
        .filter(|(id, _)| !target.is_active(*id))
        .count();
    assert_eq!(inactive, 1);
    let distance = target
        .constraints()
        .find_map(|(_, constraint)| match constraint {
            Constraint::Distance { value, .. } => Some(value.clone()),
            _ => None,
        })
        .unwrap();
    assert!(distance.parameters().is_empty());
    assert_eq!(distance.to_stored_text(), "20 mm");
    assert!(
        target
            .entities()
            .any(|(id, entity)| matches!(entity, Entity::Circle { .. })
                && target.is_construction(id))
    );
}

#[test]
fn a_parameter_of_the_same_name_drives_pasted_dimensions() {
    let source = source();
    let text = copied_text(&source);
    let mut target = Document::default();
    let mut transaction = target.transaction("Width");
    let own = transaction.add_parameter("width", transaction.parse("35 mm").unwrap());
    target.apply(transaction.finish()).unwrap();

    let pasted = read_sketch_clipboard(&text, &target, THERE).unwrap();

    assert_eq!(pasted.inlined, 0);
    let mut sketch = Sketch::new(Plane::XY);
    sketch.paste(&pasted.clip, Vector2::ZERO).unwrap();
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| { constraint.dimension().is_some_and(|value| value.uses(own)) })
    );
}

#[test]
fn foreign_damaged_newer_and_other_kinds_of_text_are_refused_in_words() {
    let source = source();
    let text = copied_text(&source);
    let document = Document::default();
    let read = |text: &str| read_sketch_clipboard(text, &document, HERE).map(|_| ());

    assert_eq!(read("hello world"), Err(ClipboardError::Foreign));
    assert_eq!(read(""), Err(ClipboardError::Foreign));
    assert_eq!(
        read("caditor clipboard: sketch geometry, version 7\n{}"),
        Err(ClipboardError::Newer {
            kind: ClipboardKind::SketchGeometry,
            version: 7
        })
    );
    assert_eq!(
        read("caditor clipboard: drawings, version 1\n{}"),
        Err(ClipboardError::UnknownKind)
    );
    let truncated = &text[..text.len() / 2];
    assert_eq!(
        read(truncated),
        Err(ClipboardError::Damaged {
            kind: ClipboardKind::SketchGeometry
        })
    );
    assert_eq!(
        read(
            "caditor clipboard: sketch geometry, version 1\n{\"source\":\"x\",\"entities\":[],\"constraints\":[]}"
        ),
        Err(ClipboardError::NothingUsable)
    );
    let features = features_clipboard_text(&[], &CarriedParameters::default(), HERE).unwrap();
    assert_eq!(
        read(&features),
        Err(ClipboardError::OtherKind {
            found: ClipboardKind::Features,
            expected: ClipboardKind::SketchGeometry
        })
    );
    assert!(
        ClipboardError::Foreign
            .to_string()
            .contains("did not come from caditor")
    );

    let windows = text.replace('\n', "\r\n");
    assert!(read(&windows).is_ok());
}

#[test]
fn damaged_items_are_left_out_with_a_note_and_the_rest_pastes() {
    let source = source();
    let text = copied_text(&source);
    let damaged = text.replacen("\"line\":", "\"wobble\":", 1);

    let pasted = read_sketch_clipboard(&damaged, &Document::default(), THERE).unwrap();

    assert!(!pasted.notes.is_empty());
    assert_eq!(pasted.clip.curve_count(), 1);
}

#[test]
fn features_cross_the_clipboard_and_paste_into_another_model() {
    let source = source();
    let mut document = source.document;
    let mut transaction = document.transaction("Extrude");
    let extrude = transaction.add_feature(
        "Extrude 1",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: source.sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::measure(5.0, Unit::Millimetre), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let features: Vec<Feature> = [source.sketch, extrude]
        .iter()
        .map(|id| document.feature(*id).unwrap().clone())
        .collect();
    let parameters = carried_of(&document, features.iter().flat_map(feature_parameters));

    let text = features_clipboard_text(&features, &parameters, HERE).unwrap();
    let read = read_features_clipboard(&text, THERE).unwrap();

    assert!(text.starts_with("caditor clipboard: features, version 1\n"));
    assert_eq!(read.origin, PasteOrigin::Elsewhere);
    assert!(read.notes.is_empty(), "{:?}", read.notes);
    assert_eq!(read.features, features);
    let mut target = Document::default();
    let paste = target
        .paste_features(&read.features, &read.parameters, read.origin)
        .unwrap();
    assert_eq!(paste.pasted.len(), 2);
    assert_eq!(paste.inlined, 1);
    target.apply(paste.transaction).unwrap();
    assert_eq!(target.features().len(), 2);
    assert_eq!(
        read_features_clipboard(&text, HERE).unwrap().origin,
        PasteOrigin::ThisDocument
    );
}
