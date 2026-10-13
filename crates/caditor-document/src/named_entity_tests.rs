use std::collections::BTreeMap;

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::*;

const MISSING: EntityId = EntityId::from_raw(40);

struct Scene {
    document: Document,
    marks: FeatureId,
    line: EntityId,
    body: FeatureId,
}

fn scene() -> Scene {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");

    let mut marks = Sketch::new(Plane::XY);
    marks.add_point(Point2::new(1.0, 1.0));
    let line = marks.add_line(Point2::new(0.0, 0.0), Point2::new(5.0, 0.0));
    let mut outline = Sketch::new(Plane::XY);
    outline.add_line(Point2::new(0.0, 0.0), Point2::new(4.0, 0.0));
    outline.add_line(Point2::new(4.0, 0.0), Point2::new(4.0, 4.0));
    outline.add_line(Point2::new(4.0, 4.0), Point2::new(0.0, 4.0));
    outline.add_line(Point2::new(0.0, 4.0), Point2::new(0.0, 0.0));

    let marks = transaction.add_feature("Marks", FeatureKind::from(marks));
    let outline = transaction.add_feature("Outline", FeatureKind::from(outline));
    let body = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::number(3.0), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();

    Scene {
        document,
        marks,
        line,
        body,
    }
}

fn axis_reference(scene: &Scene) -> AxisReference {
    AxisReference::Sketch {
        sketch: scene.marks,
        entity: MISSING,
    }
}

fn point_reference(scene: &Scene) -> PointReference {
    PointReference::Sketch {
        sketch: scene.marks,
        entity: MISSING,
    }
}

fn zero_offset() -> [Expression; 3] {
    [0.0, 0.0, 0.0].map(Expression::number)
}

fn drawn_after_naming(scene: &mut Scene, user: FeatureKind) -> EntityId {
    let mut transaction = scene.document.transaction("Name");
    transaction.add_feature("User", user);
    scene.document.apply(transaction.finish()).unwrap();

    let before = scene.document.clone();
    let mut transaction = scene.document.transaction("Draw");
    let drawn = transaction.add_sketch_entity(scene.marks, Entity::Point(Point2::new(4.5, 11.5)));
    let inverse = scene.document.apply(transaction.finish()).unwrap();
    scene.document.apply(inverse).unwrap();
    assert!(scene.document.same_content(&before));

    drawn
}

fn assert_never_reused(user: impl FnOnce(&Scene) -> FeatureKind) {
    let mut scene = scene();
    let kind = user(&scene);

    let drawn = drawn_after_naming(&mut scene, kind);

    assert!(drawn.raw() > MISSING.raw(), "{drawn:?}");
}

#[test]
fn a_datum_point_on_a_missing_sketch_point_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: point_reference(scene),
            offset: zero_offset(),
        }))
    });
}

#[test]
fn a_datum_axis_along_a_missing_sketch_line_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(axis_reference(scene))))
    });
}

#[test]
fn a_datum_axis_through_a_missing_sketch_point_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::Datum(Datum::Axis(DatumAxis::Points(
            PointReference::Origin,
            point_reference(scene),
        )))
    });
}

#[test]
fn a_measurement_of_a_missing_sketch_entity_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::from(Measurement {
            reading: Reading::Of {
                quantity: Of::Length,
                item: MeasuredItem::Sketch {
                    sketch: scene.marks,
                    entity: MISSING,
                },
            },
            parameter: None,
        })
    });
}

#[test]
fn a_measurement_between_a_missing_sketch_point_and_the_origin_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::from(Measurement {
            reading: Reading::Between {
                quantity: Between::Distance,
                first: MeasuredItem::Point(point_reference(scene)),
                second: MeasuredItem::Point(PointReference::Origin),
            },
            parameter: None,
        })
    });
}

#[test]
fn a_measurement_along_a_missing_sketch_axis_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::from(Measurement {
            reading: Reading::Position {
                item: MeasuredItem::Point(PointReference::Origin),
                axis: MeasuredItem::Axis(axis_reference(scene)),
            },
            parameter: None,
        })
    });
}

#[test]
fn a_projection_of_a_missing_sketch_entity_reserves_its_number() {
    assert_never_reused(|scene| {
        let mut projecting = Sketch::new(Plane::XY);
        let line = projecting.add_line(Point2::new(0.0, 0.0), Point2::new(1.0, 0.0));
        FeatureKind::Sketch(SketchFeature {
            sketch: projecting,
            attachment: None,
            projections: BTreeMap::from([(
                line,
                ProjectionSource::SketchEntity {
                    sketch: scene.marks,
                    entity: MISSING,
                },
            )]),
        })
    });
}

#[test]
fn a_move_about_a_missing_sketch_axis_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::Move(Move {
            body: scene.body,
            offset: zero_offset(),
            turn: zero_offset(),
            copy: false,
            about: TurnCentre::Axis(Box::new(AxisTurn {
                axis: axis_reference(scene),
                angle: Expression::number(0.5),
            })),
            frame: None,
        })
    });
}

#[test]
fn a_mate_at_a_missing_sketch_point_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::Mate(Mate {
            body: scene.body,
            pair: MatePair::Point(Box::new(PointMate {
                point: point_reference(scene),
                target: PointTarget::Point(PointReference::Origin),
            })),
            flipped: false,
        })
    });
}

#[test]
fn a_circular_pattern_about_a_missing_sketch_axis_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::from(Pattern::new(
            scene.body,
            PatternKind::Circular(CircularPattern {
                axis: Some(axis_reference(scene)),
                count: Expression::number(3.0),
                angle: Expression::number(1.0),
                reversed: false,
            }),
        ))
    });
}

#[test]
fn a_point_pattern_based_on_a_missing_sketch_point_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::from(Pattern::new(
            scene.body,
            PatternKind::Points(PointsPattern {
                sketch: scene.marks,
                base: point_reference(scene),
            }),
        ))
    });
}

#[test]
fn a_revolve_about_a_missing_sketch_axis_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch: scene.marks,
            regions: RegionChoice::All,
            axis: RevolveAxis::Model(axis_reference(scene)),
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            side: None,
            wall: None,
        }))
    });
}

#[test]
fn a_split_carried_along_a_missing_sketch_axis_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::SplitFace(SplitFace {
            body: scene.body,
            faces: Vec::new(),
            along: SplitAlong::Sketch(scene.marks),
            carry: SplitCarry::Along(Box::new(axis_reference(scene))),
        })
    });
}

#[test]
fn deleting_a_sketch_entity_a_measurement_reads_is_undone_under_the_same_number() {
    let mut scene = scene();
    let mut transaction = scene.document.transaction("Measure");
    transaction.add_feature(
        "Length",
        FeatureKind::from(Measurement {
            reading: Reading::Of {
                quantity: Of::Length,
                item: MeasuredItem::Sketch {
                    sketch: scene.marks,
                    entity: scene.line,
                },
            },
            parameter: None,
        }),
    );
    scene.document.apply(transaction.finish()).unwrap();
    let before = scene.document.clone();

    let mut transaction = scene.document.transaction("Delete");
    transaction.remove_sketch_items(scene.marks, [scene.line], []);
    let inverse = scene.document.apply(transaction.finish()).unwrap();
    scene.document.apply(inverse).unwrap();

    assert!(scene.document.same_content(&before));
}

#[test]
fn a_coil_round_a_missing_sketch_line_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::Coil(Coil::new(
            axis_reference(scene),
            Expression::number(20.0),
            Expression::number(2.0),
        ))
    });
}

#[test]
fn a_rib_names_no_single_entity_since_it_follows_every_curve_of_its_sketch() {
    let scene = scene();

    let rib = FeatureKind::Rib(Rib::new(scene.marks, scene.body, Expression::number(2.0)));

    assert!(rib.named_entities().is_empty());
    assert!(rib.reference_sketches().contains(&scene.marks));
    assert!(rib.features().contains(&scene.marks));
}

#[test]
fn a_loft_ending_at_a_missing_sketch_point_reserves_its_number() {
    assert_never_reused(|scene| {
        FeatureKind::Loft(Loft {
            end: Some(point_reference(scene)),
            ..Loft::new(vec![LoftSection::whole(scene.marks)])
        })
    });
}
