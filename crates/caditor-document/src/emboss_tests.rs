use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{FaceReference, SamplingTolerance, Solid, Surface};
use caditor_sketch::{Constraint, Entity, Sketch, Text, TextAlign};

use crate::*;

fn rectangle(min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    add_rectangle(&mut sketch, min, max);
    sketch
}

fn add_rectangle(sketch: &mut Sketch, min: (f64, f64), max: (f64, f64)) {
    let corners = [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn top_face(solid: &Solid) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal() * face.sense().sign() == Vector3::Z
                    && plane.frame().origin().z > 0.0
            }
            _ => false,
        })
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

fn walls(solid: &Solid) -> Vec<FaceReference> {
    solid
        .faces()
        .filter(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .map(|(id, _)| FaceReference::capture(solid, id).unwrap())
        .collect()
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

struct Model {
    document: Document,
    engine: Recompute,
    base: FeatureId,
    emboss: FeatureId,
}

fn extruded(sketch: FeatureId, transaction: &mut TransactionBuilder, height: &str) -> FeatureId {
    transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(transaction.parse(height).unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    )
}

fn plate(label: Sketch, form: EmbossForm, depth: &str) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = extruded(outline, &mut transaction, "4 mm");
    let label = transaction.add_feature("Label", FeatureKind::from(label));
    document.apply(transaction.finish()).unwrap();
    embossed(document, base, label, top_face, form, depth)
}

fn rod(label: Sketch, form: EmbossForm, depth: &str) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut outline = Sketch::new(Plane::XY);
    outline.add_circle(Point2::new(0.0, 0.0), 10.0);
    let outline = transaction.add_feature("Outline", FeatureKind::from(outline));
    let base = extruded(outline, &mut transaction, "30 mm");
    let label = transaction.add_feature("Label", FeatureKind::from(label));
    document.apply(transaction.finish()).unwrap();
    embossed(
        document,
        base,
        label,
        |solid| walls(solid).into_iter().next().unwrap(),
        form,
        depth,
    )
}

fn embossed(
    mut document: Document,
    base: FeatureId,
    label: FeatureId,
    face: impl Fn(&Solid) -> FaceReference,
    form: EmbossForm,
    depth: &str,
) -> Model {
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let faces = vec![face(evaluation.body(base).unwrap())];
    let mut transaction = document.transaction("Emboss");
    let depth = transaction.parse(depth).unwrap();
    let emboss = transaction.add_feature(
        "Emboss 1",
        FeatureKind::Emboss(Emboss {
            body: base,
            faces,
            sketch: Some(label),
            depth,
            form,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine,
        base,
        emboss,
    }
}

fn change(model: &mut Model, change: impl FnOnce(&mut Emboss)) {
    let mut definition = model
        .document
        .feature(model.emboss)
        .unwrap()
        .kind
        .emboss()
        .unwrap()
        .clone();
    change(&mut definition);
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.emboss,
                kind: FeatureKind::Emboss(definition),
            },
        ))
        .unwrap();
}

fn suppress(model: &mut Model, suppressed: bool) {
    model
        .document
        .apply(Transaction::single(
            "Suppress",
            Edit::SetFeatureSuppressed {
                id: model.emboss,
                suppressed,
            },
        ))
        .unwrap();
}

fn unembossed_volume(model: &mut Model) -> f64 {
    suppress(model, true);
    let evaluation = evaluate(&model.document, &mut model.engine);
    let found = volume(&evaluation, model.base);
    suppress(model, false);
    found
}

fn plate_volume() -> f64 {
    320.0
}

#[test]
fn a_raised_outline_stands_on_a_flat_face_by_its_depth() {
    let mut model = plate(
        rectangle((2.0, 2.0), (6.0, 5.0)),
        EmbossForm::Raised,
        "1 mm",
    );

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.base) - (plate_volume() + 12.0)).abs() < 1e-6);
}

#[test]
fn a_recessed_outline_sinks_into_a_flat_face_by_its_depth() {
    let mut model = plate(
        rectangle((2.0, 2.0), (6.0, 5.0)),
        EmbossForm::Recessed,
        "1 mm",
    );

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.base) - (plate_volume() - 12.0)).abs() < 1e-6);
}

#[test]
fn the_counter_of_an_outline_stays_open() {
    let mut label = rectangle((1.0, 1.0), (9.0, 7.0));
    add_rectangle(&mut label, (3.0, 3.0), (7.0, 5.0));
    let mut model = plate(label, EmbossForm::Raised, "2 mm");

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.base) - (plate_volume() + 2.0 * (48.0 - 8.0))).abs() < 1e-6);
}

#[test]
fn raised_and_recessed_outlines_follow_the_depth_expression() {
    let mut model = plate(
        rectangle((2.0, 2.0), (6.0, 5.0)),
        EmbossForm::Raised,
        "1 mm",
    );
    let deeper = model.document.parse("2.5 mm").unwrap();
    change(&mut model, |emboss| emboss.depth = deeper);

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.base) - (plate_volume() + 30.0)).abs() < 1e-6);
}

#[test]
fn an_outline_running_off_the_face_fails_in_words() {
    let mut model = plate(
        rectangle((6.0, 2.0), (12.0, 5.0)),
        EmbossForm::Raised,
        "1 mm",
    );

    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.emboss);

    assert!(
        error
            .reason()
            .starts_with("The outlines of Label reach the edge of"),
        "{}",
        error.reason()
    );
}

#[test]
fn an_outline_off_the_face_altogether_fails_in_words() {
    let mut model = plate(
        rectangle((20.0, 20.0), (22.0, 22.0)),
        EmbossForm::Raised,
        "1 mm",
    );

    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.emboss);

    assert_eq!(
        error.reason(),
        "The outlines of Label do not cross any of the chosen faces."
    );
}

#[test]
fn a_depth_of_nothing_fails_in_words() {
    let mut model = plate(
        rectangle((2.0, 2.0), (6.0, 5.0)),
        EmbossForm::Raised,
        "0 mm",
    );

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        failure(&evaluation, model.emboss).reason(),
        "The depth must be more than zero."
    );
}

#[test]
fn a_sketch_standing_across_the_face_fails_in_words() {
    let mut tilted = Sketch::new(Plane::XZ);
    add_rectangle(&mut tilted, (2.0, 0.0), (6.0, 3.0));
    let mut model = plate(tilted, EmbossForm::Raised, "1 mm");

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        failure(&evaluation, model.emboss).reason(),
        "Label does not lie parallel to the chosen faces, so its outlines cannot be raised from \
         them straight."
    );
}

#[test]
fn a_sketch_without_a_closed_outline_fails_in_words() {
    let mut open = Sketch::new(Plane::XY);
    open.add_line(Point2::new(1.0, 1.0), Point2::new(5.0, 4.0));
    let mut model = plate(open, EmbossForm::Raised, "1 mm");

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        failure(&evaluation, model.emboss).reason(),
        "Label has no closed outline or text to emboss: its curves do not enclose any region."
    );
}

#[test]
fn text_embosses_with_its_counters_open() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = extruded(outline, &mut transaction, "4 mm");
    let label = transaction.add_feature("Label", FeatureKind::from(Sketch::new(Plane::XY)));
    let anchor = transaction.add_sketch_entity(label, Entity::Point(Point2::new(2.0, 2.0)));
    transaction.add_sketch_constraint(
        label,
        Constraint::Fix {
            point: anchor,
            at: Point2::new(2.0, 2.0),
        },
    );
    transaction.add_sketch_text(
        label,
        Text {
            anchor,
            content: "O".to_owned(),
            height: Expression::measure(4.0, Unit::Millimetre),
            angle: Expression::measure(0.0, Unit::Degree),
            bold: false,
            italic: false,
            align: TextAlign::Left,
        },
    );
    document.apply(transaction.finish()).unwrap();
    let mut model = embossed(document, base, label, top_face, EmbossForm::Raised, "1 mm");

    let evaluation = evaluate(&model.document, &mut model.engine);
    let result = evaluation.feature(label).unwrap().result.clone().unwrap();
    let profile = result.sketch().unwrap().profile().unwrap();
    let letter: f64 = profile
        .regions()
        .iter()
        .filter(|region| region.depth() % 2 == 0)
        .filter_map(|region| region.section())
        .map(|section| section.area)
        .sum();

    assert_eq!(evaluation.failed_count(), 0);
    assert!(letter > 1.0);
    assert!((volume(&evaluation, model.base) - (plate_volume() + letter)).abs() < 0.02 * letter);
}

#[test]
fn an_emboss_waits_for_its_faces_and_its_sketch() {
    let mut model = plate(
        rectangle((2.0, 2.0), (6.0, 5.0)),
        EmbossForm::Raised,
        "1 mm",
    );
    change(&mut model, |emboss| emboss.sketch = None);
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        evaluation.feature(model.emboss).unwrap().state,
        FeatureState::Waiting(MissingInput::SketchToEmboss { body: model.base })
    );

    change(&mut model, |emboss| emboss.faces.clear());
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        evaluation.feature(model.emboss).unwrap().state,
        FeatureState::Waiting(MissingInput::FacesToEmboss { body: model.base })
    );
}

fn tangent_sketch(offset: f64) -> Sketch {
    Sketch::new(Plane::from_frame(Point3::new(0.0, offset, 0.0), Vector3::Y, Vector3::Z).unwrap())
}

fn patch() -> Sketch {
    let mut sketch = tangent_sketch(10.0);
    add_rectangle(&mut sketch, (10.0, -5.0), (22.0, 5.0));
    sketch
}

#[test]
fn a_raised_outline_wraps_round_a_cylinder_by_its_depth() {
    let mut model = rod(patch(), EmbossForm::Raised, "1 mm");
    let plain = unembossed_volume(&mut model);

    let evaluation = evaluate(&model.document, &mut model.engine);
    let grown = volume(&evaluation, model.base) - plain;

    assert_eq!(evaluation.failed_count(), 0);
    assert!((grown - 126.0).abs() < 126.0 * 0.01, "{grown}");
}

#[test]
fn a_recessed_outline_wraps_into_a_cylinder_by_its_depth() {
    let mut model = rod(patch(), EmbossForm::Recessed, "1 mm");
    let plain = unembossed_volume(&mut model);

    let evaluation = evaluate(&model.document, &mut model.engine);
    let cut = plain - volume(&evaluation, model.base);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((cut - 114.0).abs() < 114.0 * 0.01, "{cut}");
}

#[test]
fn a_wrapped_outline_may_be_deeper_than_a_quarter_of_the_radius() {
    let mut model = rod(patch(), EmbossForm::Raised, "4 mm");
    let plain = unembossed_volume(&mut model);

    let evaluation = evaluate(&model.document, &mut model.engine);
    let grown = volume(&evaluation, model.base) - plain;

    assert_eq!(evaluation.failed_count(), 0);
    assert!(
        (grown - 12.0 * (14.0_f64.powi(2) - 100.0) / 2.0).abs() < 7.0,
        "{grown}"
    );
}

#[test]
fn a_wrapped_outline_deeper_than_the_radius_fails_in_words() {
    let mut model = rod(patch(), EmbossForm::Recessed, "11 mm");

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        failure(&evaluation, model.emboss).reason(),
        "The depth reaches the axis of the cylinder, so there is no room to emboss that deep."
    );
}

#[test]
fn a_wrapped_outline_across_the_axis_fails_in_words() {
    let mut label = Sketch::new(Plane::XY);
    add_rectangle(&mut label, (10.0, -5.0), (22.0, 5.0));
    let mut model = rod(label, EmbossForm::Raised, "1 mm");

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        failure(&evaluation, model.emboss).reason(),
        "Label does not lie on a plane along the axis of the cylinder, so its outlines cannot be \
         wrapped round it."
    );
}

fn tube(label: Sketch, form: EmbossForm, depth: &str) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut outline = Sketch::new(Plane::XY);
    outline.add_circle(Point2::new(0.0, 0.0), 10.0);
    outline.add_circle(Point2::new(0.0, 0.0), 6.0);
    let outline = transaction.add_feature("Outline", FeatureKind::from(outline));
    let base = extruded(outline, &mut transaction, "30 mm");
    let label = transaction.add_feature("Label", FeatureKind::from(label));
    document.apply(transaction.finish()).unwrap();
    embossed(
        document,
        base,
        label,
        |solid| {
            solid
                .faces()
                .filter(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
                .filter(|(_, face)| !face.sense().is_same())
                .map(|(id, _)| FaceReference::capture(solid, id).unwrap())
                .next()
                .unwrap()
        },
        form,
        depth,
    )
}

fn bore_patch() -> Sketch {
    let mut sketch = tangent_sketch(6.0);
    add_rectangle(&mut sketch, (10.0, -3.0), (22.0, 3.0));
    sketch
}

#[test]
fn a_raised_outline_wraps_into_a_bore_towards_its_axis() {
    let mut model = tube(bore_patch(), EmbossForm::Raised, "1 mm");
    let plain = unembossed_volume(&mut model);

    let evaluation = evaluate(&model.document, &mut model.engine);
    let grown = volume(&evaluation, model.base) - plain;

    assert_eq!(evaluation.failed_count(), 0);
    assert!((grown - 66.0).abs() < 66.0 * 0.01, "{grown}");
}

#[test]
fn a_recessed_outline_wraps_into_the_wall_of_a_bore() {
    let mut model = tube(bore_patch(), EmbossForm::Recessed, "1 mm");
    let plain = unembossed_volume(&mut model);

    let evaluation = evaluate(&model.document, &mut model.engine);
    let cut = plain - volume(&evaluation, model.base);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((cut - 78.0).abs() < 78.0 * 0.01, "{cut}");
}

fn cone(label: Sketch, form: EmbossForm, depth: &str) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mm = |text: &str| Expression::parse_stored(text).unwrap();
    let base = transaction.add_feature(
        "Cone",
        FeatureKind::Primitive(Primitive {
            shape: PrimitiveShape::Cone {
                bottom: mm("24 mm"),
                top: mm("12 mm"),
                height: mm("30 mm"),
            },
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            at: [mm("0 mm"), mm("0 mm")],
            anchor: PrimitiveAnchor::BaseCentre,
            reversed: false,
            operation: BodyOperation::NewBody,
        }),
    );
    let label = transaction.add_feature("Label", FeatureKind::from(label));
    document.apply(transaction.finish()).unwrap();
    embossed(
        document,
        base,
        label,
        |solid| {
            solid
                .faces()
                .filter(|(_, face)| matches!(face.surface(), Surface::Cone(_)))
                .map(|(id, _)| FaceReference::capture(solid, id).unwrap())
                .next()
                .unwrap()
        },
        form,
        depth,
    )
}

fn cone_patch() -> Sketch {
    let mut sketch = tangent_sketch(8.0);
    add_rectangle(&mut sketch, (8.0, -2.0), (14.0, 2.0));
    sketch
}

#[test]
fn a_raised_outline_wraps_round_a_cone_and_a_recessed_one_into_it() {
    let mut raised = cone(cone_patch(), EmbossForm::Raised, "1 mm");
    let plain = unembossed_volume(&mut raised);
    let evaluation = evaluate(&raised.document, &mut raised.engine);
    let grown = volume(&evaluation, raised.base) - plain;
    assert_eq!(evaluation.failed_count(), 0);
    assert!(grown > 20.0 && grown < 45.0, "{grown}");

    let mut recessed = cone(cone_patch(), EmbossForm::Recessed, "1 mm");
    let evaluation = evaluate(&recessed.document, &mut recessed.engine);
    let cut = plain - volume(&evaluation, recessed.base);
    assert_eq!(evaluation.failed_count(), 0);
    assert!(cut > 20.0 && cut < 45.0, "{cut}");
}

#[test]
fn a_sphere_cannot_be_embossed_and_says_why() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mm = |text: &str| Expression::parse_stored(text).unwrap();
    let base = transaction.add_feature(
        "Ball",
        FeatureKind::Primitive(Primitive {
            shape: PrimitiveShape::Sphere {
                diameter: mm("20 mm"),
            },
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            at: [mm("0 mm"), mm("0 mm")],
            anchor: PrimitiveAnchor::Centre,
            reversed: false,
            operation: BodyOperation::NewBody,
        }),
    );
    let label = transaction.add_feature("Label", FeatureKind::from(bore_patch()));
    document.apply(transaction.finish()).unwrap();
    let mut model = embossed(
        document,
        base,
        label,
        |solid| {
            let (id, _) = solid.faces().next().unwrap();
            FaceReference::capture(solid, id).unwrap()
        },
        EmbossForm::Raised,
        "1 mm",
    );

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        failure(&evaluation, model.emboss).reason(),
        "Ball surface is spherical, and a sphere cannot be unrolled flat, so the curves of Label \
         cannot be wrapped onto it."
    );
}
