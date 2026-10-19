use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{EdgeReference, FaceId, FaceReference, SamplingTolerance, Solid, Surface};
use caditor_sketch::Sketch;

use crate::*;

const LONG: f64 = 10.0;
const WIDE: f64 = 8.0;

fn rectangle() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(0.0, 0.0),
        Point2::new(LONG, 0.0),
        Point2::new(LONG, WIDE),
        Point2::new(0.0, WIDE),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch
}

fn text(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
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

fn assert_volume(evaluation: &Evaluation, body: FeatureId, expected: f64) {
    let found = volume(evaluation, body);
    assert!(
        (found - expected).abs() < 2e-3 * expected,
        "volume {found} instead of {expected}"
    );
}

fn tapered(height: f64, degrees: f64) -> f64 {
    let inset = height * degrees.to_radians().tan();
    let area = |inset: f64| (LONG - 2.0 * inset) * (WIDE - 2.0 * inset);
    height / 6.0 * (area(0.0) + 4.0 * area(inset / 2.0) + area(inset))
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn flat_faces(solid: &Solid, square_to_z: bool) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(_, face)| match face.surface() {
            Surface::Plane(plane) => (plane.frame().normal().z.abs() < 0.5) == square_to_z,
            _ => false,
        })
        .map(|(id, _)| id)
        .collect()
}

fn face_at(solid: &Solid, height: f64) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal().z.abs() > 0.999
                    && (plane.frame().origin().z - height).abs() < 1e-9
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap()
}

fn has_corner(solid: &Solid, at: Point3) -> bool {
    solid
        .vertices()
        .any(|(_, vertex)| vertex.point().distance(at) < 1e-6)
}

struct Model {
    document: Document,
    engine: Recompute,
    angle: ParameterId,
    base: FeatureId,
    draft: FeatureId,
}

fn model(extent: ExtrudeExtent) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let angle = transaction.add_parameter("draft", transaction.parse("3 deg").unwrap());
    let outline = transaction.add_feature("Outline", FeatureKind::from(rectangle()));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent,
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap();
    let faces = flat_faces(solid, true)
        .into_iter()
        .map(|face| FaceReference::capture(solid, face).unwrap())
        .collect();
    let mut transaction = document.transaction("Draft");
    let draft = transaction.add_feature(
        "Draft 1",
        FeatureKind::Draft(Draft {
            body: base,
            faces,
            neutral: PlaneReference::Principal(PrincipalPlane::Xy),
            angle: Expression::Parameter(angle),
            flipped: false,
            tangent: false,
            beyond: None,
            own: Vec::new(),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine,
        angle,
        base,
        draft,
    }
}

fn standing() -> Model {
    model(ExtrudeExtent::one_side(text("4 mm"), false))
}

fn set_angle(model: &mut Model, value: &str) {
    let expression = model.document.parse(value).unwrap();
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: model.angle,
                expression,
            },
        ))
        .unwrap();
}

fn set_draft(model: &mut Model, change: impl FnOnce(&mut Draft, &Solid)) {
    let evaluation = evaluate(&model.document, &mut model.engine);
    let before = evaluation
        .body_seen_by(model.draft, model.base)
        .unwrap()
        .clone();
    let mut definition = model
        .document
        .feature(model.draft)
        .unwrap()
        .kind
        .draft()
        .unwrap()
        .clone();
    change(&mut definition, &before);
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.draft,
                kind: FeatureKind::Draft(definition),
            },
        ))
        .unwrap();
}

fn on_face(base: FeatureId, solid: &Solid, face: FaceId) -> PlaneReference {
    PlaneReference::Face(FaceAttachment::capture(base, solid, face).unwrap().0)
}

#[test]
fn walls_drafted_from_the_floor_narrow_toward_the_top_and_follow_the_angle() {
    let mut model = standing();

    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, tapered(4.0, 3.0));

    set_angle(&mut model, "5 deg");
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert_volume(&evaluation, model.base, tapered(4.0, 5.0));
}

#[test]
fn a_flat_face_as_neutral_plane_keeps_its_size_and_pulls_along_its_normal_unless_flipped() {
    let mut model = standing();
    let base = model.base;

    set_draft(&mut model, |draft, solid| {
        draft.neutral = on_face(base, solid, face_at(solid, 4.0));
        draft.flipped = true;
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation.body(model.base).unwrap();
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, tapered(4.0, 3.0));
    assert!(has_corner(solid, Point3::new(LONG, WIDE, 4.0)));
    assert!(!has_corner(solid, Point3::new(LONG, WIDE, 0.0)));

    set_draft(&mut model, |draft, solid| {
        draft.neutral = on_face(base, solid, face_at(solid, 0.0));
        draft.flipped = false;
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation.body(model.base).unwrap();
    assert_volume(&evaluation, model.base, tapered(4.0, -3.0));
    assert!(has_corner(solid, Point3::new(LONG, WIDE, 0.0)));
}

#[test]
fn walls_split_at_the_neutral_plane_take_an_angle_on_each_side() {
    let mut model = model(ExtrudeExtent::two_sides(text("2 mm"), text("2 mm")));

    set_draft(&mut model, |draft, _| draft.beyond = Some(text("6 deg")));
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(
        &evaluation,
        model.base,
        tapered(2.0, 3.0) + tapered(2.0, 6.0),
    );
    assert_eq!(evaluation.body(model.base).unwrap().faces().count(), 10);
}

#[test]
fn a_draft_from_a_datum_plane_depends_on_it() {
    let mut model = standing();
    let mut transaction = model.document.transaction("Plane");
    let datum = transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: text("2 mm"),
        })),
    );
    model.document.apply(transaction.finish()).unwrap();
    model
        .document
        .apply(Transaction::single(
            "Reorder",
            Edit::MoveFeature {
                id: datum,
                index: 2,
            },
        ))
        .unwrap();

    set_draft(&mut model, |draft, _| {
        draft.neutral = PlaneReference::Datum(datum);
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let kind = &model.document.feature(model.draft).unwrap().kind;
    let inset = 2.0 * 3f64.to_radians().tan();

    assert_eq!(evaluation.failed_count(), 0);
    assert!(kind.dependencies().contains(&datum));
    assert!(kind.uses_parameter(model.angle));
    assert!(has_corner(
        evaluation.body(model.base).unwrap(),
        Point3::new(-inset, -inset, 0.0)
    ));
}

#[test]
fn draft_errors_name_the_problem_and_the_fix() {
    let mut model = standing();

    set_angle(&mut model, "0 deg");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.draft);
    assert_eq!(
        error.reason(),
        "Every face already makes the draft angle with the pull direction."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(model.draft)));

    set_angle(&mut model, "95 deg");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.draft);
    assert_eq!(error.reason(), "The draft angle is steeper than 89 deg.");
    assert_eq!(error.remedy(), "Enter an angle between -89 deg and 89 deg.");

    set_angle(&mut model, "4 mm");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.draft);
    assert_eq!(
        error.remedy(),
        "Edit the draft angle so it gives an angle, such as 3 deg."
    );

    set_angle(&mut model, "70 deg");
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.draft);
    assert!(error.reason().ends_with("with this draft."), "{error:?}");
    assert!(evaluation.body(model.base).is_some());

    set_angle(&mut model, "3 deg");
    set_draft(&mut model, |draft, solid| {
        draft.faces = vec![FaceReference::capture(solid, face_at(solid, 4.0)).unwrap()];
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.draft);
    assert!(
        error
            .reason()
            .ends_with("is parallel to the neutral plane, so it has no line to tilt about."),
        "{}",
        error.reason()
    );

    set_draft(&mut model, |draft, _| draft.faces.clear());
    let evaluation = evaluate(&model.document, &mut model.engine);
    assert!(matches!(
        evaluation.feature(model.draft).unwrap().state,
        FeatureState::Waiting(MissingInput::FacesToDraft { .. })
    ));
}

fn edge_through(solid: &Solid, middle: Point3) -> EdgeReference {
    solid
        .edges()
        .find(|(_, edge)| {
            edge.curve()
                .point(edge.interval().middle())
                .distance(middle)
                < 1e-6
        })
        .map(|(id, _)| EdgeReference::capture(solid, id).unwrap())
        .unwrap()
}

fn filleted(middle: Point3) -> Model {
    let mut model = standing();
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation.body_seen_by(model.draft, model.base).unwrap();
    let edge = edge_through(solid, middle);
    let mut transaction = model.document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: model.base,
            groups: Vec::new(),
            edges: vec![edge],
            size: text("1 mm"),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
    model
        .document
        .apply(Transaction::single(
            "Reorder",
            Edit::MoveFeature {
                id: fillet,
                index: 2,
            },
        ))
        .unwrap();
    set_draft(&mut model, |draft, solid| {
        draft.faces = vec![FaceReference::capture(solid, east_wall(solid)).unwrap()];
        draft.tangent = true;
    });
    model
}

#[test]
fn following_tangent_faces_round_a_rounded_corner_drafts_it_with_its_walls() {
    let mut model = filleted(Point3::new(LONG, 0.0, 2.0));
    let slope = 3f64.to_radians().tan();
    let area = |height: f64| {
        let inset = height * slope;
        let corner = 1.0 - inset;
        (LONG - inset) * (WIDE - inset) - (1.0 - std::f64::consts::FRAC_PI_4) * corner * corner
    };

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(
        &evaluation,
        model.base,
        4.0 / 6.0 * (area(0.0) + 4.0 * area(2.0) + area(4.0)),
    );
}

#[test]
fn following_tangent_faces_onto_a_fillet_across_the_pull_is_refused_in_words() {
    let mut model = filleted(Point3::new(LONG, WIDE / 2.0, 4.0));

    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.draft);

    assert!(
        error.reason().ends_with(
            "is round about an axis that does not run along the pull direction, and it joins \
             the chosen faces smoothly, so it cannot be drafted."
        ),
        "{}",
        error.reason()
    );
    assert!(error.remedy().starts_with("Turn off Follow tangent faces"));
}

fn bossed() -> (Document, Recompute, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(rectangle()));
    let base = transaction.add_feature("Base", extruded(outline, "4 mm", BodyOperation::NewBody));
    let mut round = Sketch::new(Plane::XY);
    round.add_circle(Point2::new(LONG / 2.0, WIDE / 2.0), 2.0);
    let circle = transaction.add_feature("Circle", FeatureKind::from(round));
    transaction.add_feature("Boss", extruded(circle, "7 mm", BodyOperation::Add(base)));
    document.apply(transaction.finish()).unwrap();
    (document, Recompute::default(), base)
}

fn extruded(sketch: FeatureId, height: &str, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(text(height), false),
        operation,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

fn round_faces(solid: &Solid) -> Vec<FaceId> {
    solid
        .faces()
        .filter(|(_, face)| matches!(face.surface(), Surface::Cylinder(_) | Surface::Cone(_)))
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn a_boss_chosen_alone_narrows_upward_from_the_plate_it_stands_on() {
    let (mut document, mut engine, base) = bossed();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap().clone();
    let side = round_faces(&solid);
    let (neutral, flipped) = default_neutral(base, &solid, &side);
    let mut transaction = document.transaction("Draft");
    let draft = transaction.add_feature(
        "Draft 1",
        FeatureKind::Draft(Draft {
            body: base,
            faces: vec![FaceReference::capture(&solid, side[0]).unwrap()],
            neutral: neutral.clone(),
            angle: text("5 deg"),
            flipped,
            tangent: false,
            beyond: None,
            own: Vec::new(),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let top = 2.0 - 3.0 * 5f64.to_radians().tan();
    let frustum = std::f64::consts::PI * 3.0 * (4.0 + 2.0 * top + top * top) / 3.0;

    let evaluation = evaluate(&document, &mut engine);
    let drafted = evaluation.body(base).unwrap();

    assert_eq!(side.len(), 1);
    assert!(matches!(neutral, PlaneReference::Face(_)));
    assert!(!flipped);
    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.feature(draft)
    );
    assert_volume(&evaluation, base, LONG * WIDE * 4.0 + frustum);
    assert!(matches!(
        drafted.face(round_faces(drafted)[0]).unwrap().surface(),
        Surface::Cone(_)
    ));
}

#[test]
fn walls_chosen_alone_draft_from_the_floor_beside_them_by_default() {
    let mut model = standing();
    let evaluation = evaluate(&model.document, &mut model.engine);
    let solid = evaluation
        .body_seen_by(model.draft, model.base)
        .unwrap()
        .clone();
    let walls = flat_faces(&solid, true);

    let (neutral, flipped) = default_neutral(model.base, &solid, &walls);
    set_draft(&mut model, |draft, _| {
        draft.neutral = neutral.clone();
        draft.flipped = flipped;
    });
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert!(matches!(neutral, PlaneReference::Face(_)));
    assert!(flipped);
    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, model.base, tapered(4.0, 3.0));
    assert!(has_corner(
        evaluation.body(model.base).unwrap(),
        Point3::new(LONG, WIDE, 0.0)
    ));
}

fn tapered_sides(height: f64, [east, west, north, south]: [f64; 4]) -> f64 {
    let slope = |degrees: f64| degrees.to_radians().tan();
    let along = slope(east) + slope(west);
    let across = slope(north) + slope(south);
    let area = |z: f64| (LONG - along * z) * (WIDE - across * z);
    height / 6.0 * (area(0.0) + 4.0 * area(height / 2.0) + area(height))
}

fn east_wall(solid: &Solid) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => (plane.frame().normal() * face.sense().sign()).x > 0.999,
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap()
}

fn east_at_its_own_angle(draft: &mut Draft, solid: &Solid, angle: &str) {
    let east = east_wall(solid);
    draft.faces = flat_faces(solid, true)
        .into_iter()
        .filter(|face| *face != east)
        .map(|face| FaceReference::capture(solid, face).unwrap())
        .collect();
    draft.own = vec![FaceAngle {
        face: FaceReference::capture(solid, east).unwrap(),
        angle: text(angle),
    }];
}

#[test]
fn a_face_at_an_angle_of_its_own_leans_by_it_and_keeps_the_others_at_the_draft_angle() {
    let mut model = standing();

    set_draft(&mut model, |draft, solid| {
        east_at_its_own_angle(draft, solid, "8 deg");
    });
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(
        &evaluation,
        model.base,
        tapered_sides(4.0, [8.0, 3.0, 3.0, 3.0]),
    );

    set_draft(&mut model, |draft, _| draft.faces.clear());
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(
        &evaluation,
        model.base,
        tapered_sides(4.0, [8.0, 0.0, 0.0, 0.0]),
    );
}

#[test]
fn a_face_at_an_angle_of_its_own_split_at_the_neutral_plane_keeps_the_angle_beyond_below() {
    let mut model = model(ExtrudeExtent::two_sides(text("2 mm"), text("2 mm")));

    set_draft(&mut model, |draft, solid| {
        east_at_its_own_angle(draft, solid, "8 deg");
        draft.beyond = Some(text("6 deg"));
    });
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(
        &evaluation,
        model.base,
        tapered_sides(2.0, [8.0, 3.0, 3.0, 3.0]) + tapered(2.0, 6.0),
    );
}

#[test]
fn an_own_angle_too_steep_or_on_a_lost_face_fails_in_words() {
    let mut model = standing();

    set_draft(&mut model, |draft, solid| {
        east_at_its_own_angle(draft, solid, "89.5 deg");
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.draft);

    assert_eq!(
        error.reason(),
        "The own draft angle of a face is steeper than 89 deg."
    );

    set_draft(&mut model, |draft, _| {
        if let Some(own) = draft.own.first_mut() {
            own.angle = text("4 deg");
            own.face = FaceReference::new(caditor_kernel::FaceName::from_digest(0xdead), None, []);
        }
    });
    let evaluation = evaluate(&model.document, &mut model.engine);
    let error = failure(&evaluation, model.draft);

    assert!(
        error
            .reason()
            .contains("face given its own draft angle is no longer part of the body of Base"),
        "{}",
        error.reason()
    );
}

#[test]
fn a_post_split_at_the_neutral_plane_narrows_both_ways_from_its_parting_circle() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let mut round = Sketch::new(Plane::XY);
    round.add_circle(Point2::new(0.0, 0.0), 3.0);
    let circle = transaction.add_feature("Circle", FeatureKind::from(round));
    let post = transaction.add_feature(
        "Post",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: circle,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::two_sides(text("2 mm"), text("2 mm")),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(post).unwrap().clone();
    let mut transaction = document.transaction("Draft");
    transaction.add_feature(
        "Draft 1",
        FeatureKind::Draft(Draft {
            body: post,
            faces: round_faces(&solid)
                .into_iter()
                .map(|face| FaceReference::capture(&solid, face).unwrap())
                .collect(),
            neutral: PlaneReference::Principal(PrincipalPlane::Xy),
            angle: text("3 deg"),
            flipped: false,
            tangent: false,
            beyond: Some(text("6 deg")),
            own: Vec::new(),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let cone = |degrees: f64| {
        let end = 3.0 - 2.0 * degrees.to_radians().tan();
        std::f64::consts::PI * 2.0 * (9.0 + 3.0 * end + end * end) / 3.0
    };

    let evaluation = evaluate(&document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(&evaluation, post, cone(3.0) + cone(6.0));
    assert_eq!(round_faces(evaluation.body(post).unwrap()).len(), 2);
}
