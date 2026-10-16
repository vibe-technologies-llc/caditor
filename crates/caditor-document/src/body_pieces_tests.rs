use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{EdgeReference, Solid};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::*;

pub(crate) struct Drawn {
    pub(crate) points: Vec<EntityId>,
    pub(crate) lines: Vec<EntityId>,
}

fn corners(min: (f64, f64), max: (f64, f64)) -> [Point2; 4] {
    [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
    ]
}

fn draw(sketch: &mut Sketch, min: (f64, f64), max: (f64, f64)) -> Drawn {
    let corners = corners(min, max);
    let mut drawn = Drawn {
        points: Vec::new(),
        lines: Vec::new(),
    };
    for index in 0..4 {
        let line = sketch.add_line(corners[index], corners[(index + 1) % 4]);
        if let Some(Entity::Line { start, end }) = sketch.entity(line) {
            drawn.points.extend([*start, *end]);
        }
        drawn.lines.push(line);
    }
    drawn
}

pub(crate) fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn edge_at(solid: &Solid, point: Point3) -> EdgeReference {
    let (id, _) = solid
        .edges()
        .find(|(_, edge)| {
            let parameter = edge.curve().closest_parameter(point, edge.interval());
            edge.curve().point(parameter).distance(point) < 1e-6
        })
        .unwrap();
    EdgeReference::capture(solid, id).unwrap()
}

pub(crate) fn piece_of(extrusion: FeatureId, drawn: &Drawn) -> FeatureId {
    extrusion
        .piece(PieceKey::of_entity(drawn.lines[0]).unwrap())
        .unwrap()
}

fn extrusion(sketch: FeatureId) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(Expression::parse_stored("2 mm").unwrap(), false),
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

pub(crate) struct Model {
    pub(crate) document: Document,
    pub(crate) engine: Recompute,
    pub(crate) sketch: FeatureId,
    pub(crate) extrusion: FeatureId,
    pub(crate) rectangles: [Drawn; 3],
}

pub(crate) fn three_rectangles() -> Model {
    let mut drawing = Sketch::new(Plane::XY);
    let rectangles = [
        draw(&mut drawing, (0.0, 0.0), (4.0, 4.0)),
        draw(&mut drawing, (10.0, 0.0), (14.0, 4.0)),
        draw(&mut drawing, (20.0, 0.0), (24.0, 4.0)),
    ];
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let sketch = transaction.add_feature("Shapes", FeatureKind::from(drawing));
    let mut extruded = Feature::new(
        FeatureId::from_raw(0),
        "Blocks".to_owned(),
        extrusion(sketch),
    );
    extruded.pieces = Pieces::BodyEach;
    let extrusion = transaction.add_copied_feature(extruded);
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        engine: Recompute::default(),
        sketch,
        extrusion,
        rectangles,
    }
}

fn fillet_on(model: &mut Model, body: FeatureId, at: Point3) -> FeatureId {
    let evaluation = evaluate(&model.document, &mut model.engine);
    let edge = edge_at(evaluation.body(body).unwrap(), at);
    let mut transaction = model.document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body,
            groups: Vec::new(),
            edges: vec![edge],
            size: Expression::parse_stored("0.5 mm").unwrap(),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
    fillet
}

fn moved(model: &mut Model, rectangle: usize, by: (f64, f64)) {
    let points = model.rectangles[rectangle].points.clone();
    let sketch = model
        .document
        .feature(model.sketch)
        .unwrap()
        .kind
        .sketch()
        .unwrap()
        .clone();
    let edits = points
        .into_iter()
        .map(|point| {
            let at = sketch.point(point).unwrap();
            Edit::SetSketchEntity {
                feature: model.sketch,
                id: point,
                entity: Entity::Point(Point2::new(at.x + by.0, at.y + by.1)),
            }
        })
        .collect();
    model
        .document
        .apply(Transaction::new("Move", edits))
        .unwrap();
}

fn state(evaluation: &Evaluation, feature: FeatureId) -> FeatureState {
    evaluation.feature(feature).unwrap().state.clone()
}

#[test]
fn three_separate_rectangles_extrude_into_three_bodies() {
    let mut model = three_rectangles();

    let evaluation = evaluate(&model.document, &mut model.engine);
    let bodies: Vec<FeatureId> = evaluation.bodies().map(|(body, _)| body).collect();
    let expected: Vec<FeatureId> = model
        .rectangles
        .iter()
        .map(|drawn| piece_of(model.extrusion, drawn))
        .collect();
    let shells: Vec<usize> = bodies
        .iter()
        .map(|body| evaluation.body(*body).unwrap().shells().count())
        .collect();
    let name = model.document.body_name(expected[1]).unwrap().into_owned();

    assert_eq!(state(&evaluation, model.extrusion), FeatureState::UpToDate);
    assert_eq!(bodies, expected);
    assert_eq!(shells, [1, 1, 1]);
    assert!(evaluation.body(model.extrusion).is_none());
    assert_eq!(
        name,
        format!("Blocks (Line {})", model.rectangles[1].lines[0])
    );
}

#[test]
fn one_body_keeps_the_separate_rectangles_as_lumps_of_one_body() {
    let mut model = three_rectangles();
    model
        .document
        .apply(Transaction::single(
            "One body",
            Edit::SetFeaturePieces {
                id: model.extrusion,
                pieces: Pieces::OneBody,
            },
        ))
        .unwrap();

    let evaluation = evaluate(&model.document, &mut model.engine);
    let bodies: Vec<FeatureId> = evaluation.bodies().map(|(body, _)| body).collect();

    assert_eq!(bodies, [model.extrusion]);
    assert_eq!(
        evaluation.body(model.extrusion).unwrap().shells().count(),
        3
    );
}

#[test]
fn a_fillet_on_the_second_body_survives_moving_the_first_and_adding_a_fourth() {
    let mut model = three_rectangles();
    let second = piece_of(model.extrusion, &model.rectangles[1]);
    let fillet = fillet_on(&mut model, second, Point3::new(12.0, 0.0, 2.0));
    let before = evaluate(&model.document, &mut model.engine);
    let filleted_faces = before.body(second).unwrap().faces().count();

    moved(&mut model, 0, (0.0, 30.0));
    let after_move = evaluate(&model.document, &mut model.engine);
    let mut transaction = model.document.transaction("Fourth");
    for (start, end) in [
        ((30.0, 0.0), (34.0, 0.0)),
        ((34.0, 0.0), (34.0, 4.0)),
        ((34.0, 4.0), (30.0, 4.0)),
        ((30.0, 4.0), (30.0, 0.0)),
    ] {
        let start = transaction
            .add_sketch_entity(model.sketch, Entity::Point(Point2::new(start.0, start.1)));
        let end =
            transaction.add_sketch_entity(model.sketch, Entity::Point(Point2::new(end.0, end.1)));
        transaction.add_sketch_entity(model.sketch, Entity::Line { start, end });
    }
    model.document.apply(transaction.finish()).unwrap();
    let after_adding = evaluate(&model.document, &mut model.engine);

    assert_eq!(state(&before, fillet), FeatureState::UpToDate);
    assert_eq!(state(&after_move, fillet), FeatureState::UpToDate);
    assert_eq!(state(&after_adding, fillet), FeatureState::UpToDate);
    assert_eq!(after_adding.bodies().count(), 4);
    assert_eq!(
        after_adding.body(second).unwrap().faces().count(),
        filleted_faces
    );
    assert!(
        after_adding
            .body(piece_of(model.extrusion, &model.rectangles[0]))
            .unwrap()
            .bounding_box()
            .unwrap()
            .min()
            .y
            > 29.0
    );
}

#[test]
fn deleting_the_second_rectangle_fails_its_fillet_in_plain_words() {
    let mut model = three_rectangles();
    let second = piece_of(model.extrusion, &model.rectangles[1]);
    let fillet = fillet_on(&mut model, second, Point3::new(12.0, 0.0, 2.0));
    let doomed: Vec<EntityId> = model.rectangles[1]
        .lines
        .iter()
        .chain(&model.rectangles[1].points)
        .copied()
        .collect();
    let mut transaction = model.document.transaction("Delete");
    transaction.remove_sketch_items(model.sketch, doomed, []);
    model.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&model.document, &mut model.engine);
    let FeatureState::Failed(error) = state(&evaluation, fillet) else {
        panic!("the fillet should fail");
    };

    assert_eq!(evaluation.bodies().count(), 2);
    assert!(
        error.reason()
            == "The piece of Blocks drawn by a curve since deleted is no longer there, so there \
                is no body to work on.",
        "{}",
        error.reason()
    );
    assert_eq!(
        error.remedy(),
        "Draw the piece again in Shapes, or choose another body."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(model.sketch)));
}

#[test]
fn pieces_that_merge_keep_the_identity_of_the_one_drawn_first() {
    let mut model = three_rectangles();
    let first = piece_of(model.extrusion, &model.rectangles[0]);
    let second = piece_of(model.extrusion, &model.rectangles[1]);

    moved(&mut model, 1, (-6.0, 0.0));
    let evaluation = evaluate(&model.document, &mut model.engine);
    let bodies: Vec<FeatureId> = evaluation.bodies().map(|(body, _)| body).collect();

    assert_eq!(state(&evaluation, model.extrusion), FeatureState::UpToDate);
    assert_eq!(
        bodies,
        [first, piece_of(model.extrusion, &model.rectangles[2])]
    );
    assert!(evaluation.body(second).is_none());
}

#[test]
fn pieces_in_use_cannot_go_back_to_one_body() {
    let mut model = three_rectangles();
    let second = piece_of(model.extrusion, &model.rectangles[1]);
    fillet_on(&mut model, second, Point3::new(12.0, 0.0, 2.0));

    let refused = model.document.apply(Transaction::single(
        "One body",
        Edit::SetFeaturePieces {
            id: model.extrusion,
            pieces: Pieces::OneBody,
        },
    ));

    assert!(matches!(refused, Err(EditError::PiecesInUse { .. })));
}

#[test]
fn each_piece_is_hidden_and_coloured_on_its_own_and_undo_restores_it() {
    let mut model = three_rectangles();
    let first = piece_of(model.extrusion, &model.rectangles[0]);
    let second = piece_of(model.extrusion, &model.rectangles[1]);
    let red = BodyAppearance {
        colour: Some(Rgb {
            red: 200,
            green: 0,
            blue: 0,
        }),
        name: Some("Left block".to_owned()),
        ..BodyAppearance::default()
    };
    let before = model.document.clone();

    let undo = model
        .document
        .apply(Transaction::new(
            "Looks",
            vec![
                Edit::SetFeatureHidden {
                    id: second,
                    hidden: true,
                },
                Edit::SetBodyAppearance {
                    id: first,
                    appearance: red.clone(),
                },
            ],
        ))
        .unwrap();
    let hidden = (
        model.document.body_hidden(first),
        model.document.body_hidden(second),
    );
    let first_look = model.document.body_appearance(first).unwrap().into_owned();
    let second_look = model.document.body_appearance(second).unwrap().into_owned();
    let first_name = model.document.body_name(first).unwrap().into_owned();
    model.document.apply(undo).unwrap();

    assert_eq!(hidden, (false, true));
    assert_eq!(first_look, red);
    assert_eq!(second_look, BodyAppearance::default());
    assert_eq!(first_name, "Left block");
    assert!(model.document.same_content(&before));
}

fn redraw_line(model: &mut Model, start: (f64, f64), end: (f64, f64)) -> EntityId {
    let mut transaction = model.document.transaction("Redraw");
    let start =
        transaction.add_sketch_entity(model.sketch, Entity::Point(Point2::new(start.0, start.1)));
    let end = transaction.add_sketch_entity(model.sketch, Entity::Point(Point2::new(end.0, end.1)));
    let line = transaction.add_sketch_entity(model.sketch, Entity::Line { start, end });
    model.document.apply(transaction.finish()).unwrap();
    line
}

#[test]
fn deleting_and_redrawing_the_curve_keying_a_piece_keeps_its_body_and_its_fillet() {
    let mut model = three_rectangles();
    let second = piece_of(model.extrusion, &model.rectangles[1]);
    let keying = model.rectangles[1].lines[0];
    let carrier = model.rectangles[1].lines[1];
    let fillet = fillet_on(&mut model, second, Point3::new(14.0, 2.0, 2.0));

    let mut transaction = model.document.transaction("Delete");
    transaction.remove_sketch_items(model.sketch, [keying], []);
    model.document.apply(transaction.finish()).unwrap();
    let stand_ins = model
        .document
        .feature(model.extrusion)
        .unwrap()
        .piece_stand_ins
        .clone();
    let opened = evaluate(&model.document, &mut model.engine);
    let redrawn = redraw_line(&mut model, (10.0, 0.0), (14.0, 0.0));
    let closed = evaluate(&model.document, &mut model.engine);
    let bodies: Vec<FeatureId> = closed.bodies().map(|(body, _)| body).collect();
    let name = model.document.body_name(second).unwrap().into_owned();

    assert_eq!(stand_ins, PieceStandIns::default().with(keying, carrier));
    assert!(opened.body(second).is_none());
    assert!(redrawn > carrier);
    assert_eq!(state(&closed, model.extrusion), FeatureState::UpToDate);
    assert_eq!(state(&closed, fillet), FeatureState::UpToDate);
    assert_eq!(
        bodies,
        model
            .rectangles
            .iter()
            .map(|drawn| piece_of(model.extrusion, drawn))
            .collect::<Vec<_>>()
    );
    assert_eq!(name, format!("Blocks (Line {carrier})"));
}

#[test]
fn undoing_the_deletion_of_a_keying_curve_restores_the_stand_ins_exactly() {
    let mut model = three_rectangles();
    let keying = model.rectangles[1].lines[0];
    let before = model.document.clone();

    let mut transaction = model.document.transaction("Delete");
    transaction.remove_sketch_items(model.sketch, [keying], []);
    let undo = model.document.apply(transaction.finish()).unwrap();
    let recorded = !model
        .document
        .feature(model.extrusion)
        .unwrap()
        .piece_stand_ins
        .is_empty();
    model.document.apply(undo).unwrap();

    assert!(recorded);
    assert!(model.document.same_content(&before));
}

#[test]
fn a_carrier_deleted_in_turn_hands_the_key_on_to_the_next_curve() {
    let mut model = three_rectangles();
    let second = piece_of(model.extrusion, &model.rectangles[1]);
    let lines = model.rectangles[1].lines.clone();

    for line in [lines[0], lines[1]] {
        let mut transaction = model.document.transaction("Delete");
        transaction.remove_sketch_items(model.sketch, [line], []);
        model.document.apply(transaction.finish()).unwrap();
    }
    let stand_ins = model
        .document
        .feature(model.extrusion)
        .unwrap()
        .piece_stand_ins
        .clone();
    redraw_line(&mut model, (10.0, 0.0), (14.0, 0.0));
    redraw_line(&mut model, (14.0, 0.0), (14.0, 4.0));
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(stand_ins, PieceStandIns::default().with(lines[0], lines[2]));
    assert!(evaluation.body(second).is_some());
    assert_eq!(evaluation.bodies().count(), 3);
}
