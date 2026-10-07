use std::{collections::BTreeSet, f64::consts::PI};

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;

use crate::{
    combine_tests::{Pair, evaluate, pair, volume},
    *,
};

const PLATE: f64 = 20.0 * 10.0 * 4.0;

fn expression(document: &Document, text: &str) -> Expression {
    document.parse(text).unwrap()
}

fn top() -> Plane {
    Plane::from_frame(Point3::new(0.0, 0.0, 4.0), Vector3::Z, Vector3::X).unwrap()
}

fn drilled(
    pair: &mut Pair,
    points: &[(f64, f64)],
    style: impl FnOnce(&Document) -> HoleStyle,
    depth: impl FnOnce(&Document) -> HoleDepth,
    reversed: bool,
) -> FeatureId {
    let mut sketch = Sketch::new(top());
    for (x, y) in points {
        sketch.add_point(Point2::new(*x, *y));
    }
    let hole = Hole {
        sketch: FeatureId::from_raw(0),
        body: pair.plate,
        diameter: expression(&pair.document, "4 mm"),
        depth: depth(&pair.document),
        style: style(&pair.document),
        reversed,
        shape: HoleShape::Round,
        standard: None,
    };
    let mut transaction = pair.document.transaction("Drill");
    let sketch = transaction.add_feature("Hole sketch", FeatureKind::from(sketch));
    let feature = transaction.add_feature("Hole 1", FeatureKind::Hole(Hole { sketch, ..hole }));
    pair.document.apply(transaction.finish()).unwrap();
    feature
}

fn plain(pair: &mut Pair, points: &[(f64, f64)], depth: &str) -> FeatureId {
    drilled(
        pair,
        points,
        |_| HoleStyle::Plain,
        |document| HoleDepth::Blind(expression(document, depth)),
        false,
    )
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_plain_blind_hole_removes_a_cylinder_of_its_diameter_and_depth() {
    let mut pair = pair();
    plain(&mut pair, &[(5.0, 5.0)], "2 mm");
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let removed = PI * 2.0 * 2.0 * 2.0;
    let found = volume(&evaluation, pair.plate);
    assert!((PLATE - found - removed).abs() < 0.01 * removed, "{found}");
}

#[test]
fn each_point_of_the_sketch_gets_a_hole_and_curve_ends_do_not() {
    let mut pair = pair();
    let mut sketch = Sketch::new(top());
    sketch.add_point(Point2::new(4.0, 5.0));
    sketch.add_point(Point2::new(10.0, 5.0));
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(3.0, 3.0));
    let mut transaction = pair.document.transaction("Drill");
    let sketch_id = transaction.add_feature("Hole sketch", FeatureKind::from(sketch));
    transaction.add_feature(
        "Hole 1",
        FeatureKind::Hole(Hole {
            sketch: sketch_id,
            body: pair.plate,
            diameter: expression(&pair.document, "2 mm"),
            depth: HoleDepth::ThroughAll,
            style: HoleStyle::Plain,
            reversed: false,
            shape: HoleShape::Round,
            standard: None,
        }),
    );
    pair.document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let removed = 2.0 * PI * 1.0 * 4.0;
    let found = volume(&evaluation, pair.plate);
    assert!((PLATE - found - removed).abs() < 0.01 * removed, "{found}");
}

#[test]
fn a_through_hole_goes_all_the_way_and_a_reversed_one_drills_the_other_way() {
    let mut pair = pair();
    let hole = drilled(
        &mut pair,
        &[(5.0, 5.0)],
        |_| HoleStyle::Plain,
        |_| HoleDepth::ThroughAll,
        false,
    );
    let mut engine = Recompute::default();

    let through = evaluate(&pair.document, &mut engine);

    let removed = PI * 2.0 * 2.0 * 4.0;
    assert!((PLATE - volume(&through, pair.plate) - removed).abs() < 0.01 * removed);

    let kind = pair.document.feature(hole).unwrap().kind.clone();
    let FeatureKind::Hole(definition) = kind else {
        panic!("a hole");
    };
    pair.document
        .apply(Transaction::single(
            "Reverse",
            Edit::SetFeatureKind {
                id: hole,
                kind: FeatureKind::Hole(Hole {
                    reversed: true,
                    ..definition
                }),
            },
        ))
        .unwrap();

    let away = evaluate(&pair.document, &mut engine);

    let error = failure(&away, hole);
    assert!(
        error.reason.contains("nothing to go through"),
        "{}",
        error.reason
    );
}

#[test]
fn a_counterbore_and_a_countersink_cut_their_wider_part_too() {
    let mut pair = pair();
    let bore = drilled(
        &mut pair,
        &[(5.0, 5.0)],
        |document| HoleStyle::Counterbore {
            diameter: expression(document, "6 mm"),
            depth: expression(document, "1 mm"),
        },
        |document| HoleDepth::Blind(expression(document, "3 mm")),
        false,
    );
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let removed = PI * 9.0 * 1.0 + PI * 4.0 * 2.0;
    let found = volume(&evaluation, pair.plate);
    assert!((PLATE - found - removed).abs() < 0.01 * removed, "{found}");

    let FeatureKind::Hole(definition) = pair.document.feature(bore).unwrap().kind.clone() else {
        panic!("a hole");
    };
    let sink = Hole {
        style: HoleStyle::Countersink {
            diameter: expression(&pair.document, "6 mm"),
            angle: expression(&pair.document, "90 deg"),
        },
        ..definition
    };
    pair.document
        .apply(Transaction::single(
            "Countersink",
            Edit::SetFeatureKind {
                id: bore,
                kind: FeatureKind::Hole(sink),
            },
        ))
        .unwrap();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let removed = PI / 3.0 * 1.0 * (9.0 + 6.0 + 4.0) + PI * 4.0 * 2.0;
    let found = volume(&evaluation, pair.plate);
    assert!((PLATE - found - removed).abs() < 0.01 * removed, "{found}");
}

#[test]
fn bad_sizes_fail_the_hole_alone_saying_which() {
    for (diameter, depth, style, expected) in [
        (
            "0 mm",
            "2 mm",
            HoleStyleCase::Plain,
            "The diameter must be more than zero.",
        ),
        (
            "4 mm",
            "5 deg",
            HoleStyleCase::Plain,
            "The depth cannot be evaluated",
        ),
        (
            "4 mm",
            "2 mm",
            HoleStyleCase::BoreNarrow,
            "The counterbore is not wider than the hole.",
        ),
        (
            "4 mm",
            "2 mm",
            HoleStyleCase::BoreDeep,
            "The counterbore is as deep as the whole hole.",
        ),
        (
            "4 mm",
            "2 mm",
            HoleStyleCase::SinkAngle,
            "The countersink angle must be above 0° and at most 179°.",
        ),
        (
            "4 mm",
            "0.5 mm",
            HoleStyleCase::SinkDeep,
            "The countersink is as deep as the whole hole.",
        ),
    ] {
        let mut pair = pair();
        let hole = drilled(
            &mut pair,
            &[(5.0, 5.0)],
            |document| style.build(document),
            |document| HoleDepth::Blind(expression(document, depth)),
            false,
        );
        let FeatureKind::Hole(mut definition) = pair.document.feature(hole).unwrap().kind.clone()
        else {
            panic!("a hole");
        };
        definition.diameter = expression(&pair.document, diameter);
        pair.document
            .apply(Transaction::single(
                "Resize",
                Edit::SetFeatureKind {
                    id: hole,
                    kind: FeatureKind::Hole(definition),
                },
            ))
            .unwrap();
        let mut engine = Recompute::default();

        let evaluation = evaluate(&pair.document, &mut engine);

        let error = failure(&evaluation, hole);
        assert!(
            error.reason.contains(expected),
            "{}: {}",
            expected,
            error.reason
        );
        assert_eq!(evaluation.failed_count(), 1);
        assert!(evaluation.body(pair.peg).is_some());
    }
}

#[derive(Clone, Copy)]
enum HoleStyleCase {
    Plain,
    BoreNarrow,
    BoreDeep,
    SinkAngle,
    SinkDeep,
}

impl HoleStyleCase {
    fn build(self, document: &Document) -> HoleStyle {
        match self {
            Self::Plain => HoleStyle::Plain,
            Self::BoreNarrow => HoleStyle::Counterbore {
                diameter: expression(document, "3 mm"),
                depth: expression(document, "1 mm"),
            },
            Self::BoreDeep => HoleStyle::Counterbore {
                diameter: expression(document, "6 mm"),
                depth: expression(document, "2 mm"),
            },
            Self::SinkAngle => HoleStyle::Countersink {
                diameter: expression(document, "6 mm"),
                angle: expression(document, "180 deg"),
            },
            Self::SinkDeep => HoleStyle::Countersink {
                diameter: expression(document, "10 mm"),
                angle: expression(document, "90 deg"),
            },
        }
    }
}

#[test]
fn a_hole_that_misses_the_body_or_a_sketch_without_points_fails_in_words() {
    let mut pair = pair();
    let missing = plain(&mut pair, &[(50.0, 5.0)], "2 mm");
    let mut empty = Sketch::new(top());
    empty.add_line(Point2::new(0.0, 0.0), Point2::new(3.0, 3.0));
    let mut transaction = pair.document.transaction("Empty");
    let sketch = transaction.add_feature("Line sketch", FeatureKind::from(empty));
    let blank = transaction.add_feature(
        "Hole 2",
        FeatureKind::Hole(Hole {
            sketch,
            body: pair.plate,
            diameter: expression(&pair.document, "2 mm"),
            depth: HoleDepth::ThroughAll,
            style: HoleStyle::Plain,
            reversed: false,
            shape: HoleShape::Round,
            standard: None,
        }),
    );
    pair.document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert!(
        failure(&evaluation, missing)
            .reason
            .contains("does not reach")
    );
    assert!(
        failure(&evaluation, blank)
            .reason
            .contains("no points or circles to drill at")
    );
}

#[test]
fn a_hole_follows_its_parameters_and_names_its_faces_after_the_part_of_the_hole() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Size");
    let size = transaction.add_parameter("bore", transaction.parse("4 mm").unwrap());
    pair.document.apply(transaction.finish()).unwrap();
    let hole = plain(&mut pair, &[(5.0, 5.0)], "2 mm");
    let FeatureKind::Hole(definition) = pair.document.feature(hole).unwrap().kind.clone() else {
        panic!("a hole");
    };
    pair.document
        .apply(Transaction::single(
            "Link",
            Edit::SetFeatureKind {
                id: hole,
                kind: FeatureKind::Hole(Hole {
                    diameter: Expression::Parameter(size),
                    ..definition
                }),
            },
        ))
        .unwrap();
    let mut engine = Recompute::default();
    let first = evaluate(&pair.document, &mut engine);
    let expression = pair.document.parse("6 mm").unwrap();
    pair.document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: size,
                expression,
            },
        ))
        .unwrap();

    let second = evaluate(&pair.document, &mut engine);

    assert!(volume(&second, pair.plate) < volume(&first, pair.plate));
    let described: Vec<String> = second
        .body(pair.plate)
        .unwrap()
        .faces()
        .map(|(_, face)| describe_origin(&pair.document, face.origin()))
        .collect();
    assert!(
        described.contains(&"Hole 1 wall".to_owned()),
        "{described:?}"
    );
    assert!(
        described.contains(&"Hole 1 bottom".to_owned()),
        "{described:?}"
    );
    assert!(
        pair.document
            .feature(hole)
            .unwrap()
            .kind
            .uses_parameter(size)
    );
}

#[test]
fn a_hole_modifies_its_body_and_depends_on_the_body_and_its_sketch() {
    let mut pair = pair();
    let hole = plain(&mut pair, &[(5.0, 5.0)], "2 mm");
    let feature = pair.document.feature(hole).unwrap();
    let sketch = *feature
        .kind
        .features()
        .iter()
        .find(|id| **id != pair.plate)
        .unwrap();

    assert!(feature.kind.modifies_body());
    assert!(!feature.makes_body());
    assert_eq!(feature.body(), Some(pair.plate));
    assert_eq!(pair.document.dependents_of(&[pair.plate]), vec![hole]);
    assert_eq!(pair.document.dependents_of(&[sketch]), vec![hole]);
}

#[test]
fn a_reference_to_the_wall_keeps_the_wall_when_the_style_changes() {
    let mut pair = pair();
    let hole = plain(&mut pair, &[(5.0, 5.0)], "3 mm");
    let mut engine = Recompute::default();
    let plain_body = evaluate(&pair.document, &mut engine)
        .body(pair.plate)
        .unwrap()
        .clone();
    let wall = plain_body
        .faces()
        .find(|(_, face)| describe_origin(&pair.document, face.origin()) == "Hole 1 wall")
        .map(|(id, _)| id)
        .unwrap();
    let reference = caditor_kernel::FaceReference::capture(&plain_body, wall).unwrap();
    let FeatureKind::Hole(definition) = pair.document.feature(hole).unwrap().kind.clone() else {
        panic!("a hole");
    };
    let style = HoleStyle::Counterbore {
        diameter: expression(&pair.document, "7 mm"),
        depth: expression(&pair.document, "1 mm"),
    };
    pair.document
        .apply(Transaction::single(
            "Counterbore",
            Edit::SetFeatureKind {
                id: hole,
                kind: FeatureKind::Hole(Hole {
                    style,
                    ..definition
                }),
            },
        ))
        .unwrap();

    let evaluation = evaluate(&pair.document, &mut engine);

    let bored = evaluation.body(pair.plate).unwrap();
    let resolved = reference.resolve(bored).unwrap();
    let described = describe_origin(&pair.document, bored.face(resolved).unwrap().origin());
    assert_eq!(described, "Hole 1 wall");
    let parts: BTreeSet<String> = bored
        .faces()
        .map(|(_, face)| describe_origin(&pair.document, face.origin()))
        .filter(|name| name.starts_with("Hole 1"))
        .collect();
    assert_eq!(
        parts,
        [
            "Hole 1 bottom",
            "Hole 1 counterbore floor",
            "Hole 1 counterbore wall",
            "Hole 1 wall",
        ]
        .map(str::to_owned)
        .into()
    );
}

#[test]
fn a_circle_drawn_where_a_hole_goes_drills_at_its_centre() {
    let mut pair = pair();
    let mut sketch = Sketch::new(top());
    sketch.add_circle(Point2::new(5.0, 5.0), 2.0);
    let construction = sketch.add_circle(Point2::new(12.0, 5.0), 2.0);
    sketch.set_construction(construction, true).unwrap();
    let centres = hole_centres(&sketch);
    let hole = Hole {
        sketch: FeatureId::from_raw(0),
        body: pair.plate,
        diameter: expression(&pair.document, "4 mm"),
        depth: HoleDepth::Blind(expression(&pair.document, "2 mm")),
        style: HoleStyle::Plain,
        reversed: false,
        shape: HoleShape::Round,
        standard: None,
    };
    let mut transaction = pair.document.transaction("Drill");
    let sketch = transaction.add_feature("Hole sketch", FeatureKind::from(sketch));
    transaction.add_feature("Hole 1", FeatureKind::Hole(Hole { sketch, ..hole }));
    pair.document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(centres.len(), 1);
    assert_eq!(centres[0].1, Point2::new(5.0, 5.0));
    assert_eq!(evaluation.failed_count(), 0);
    let removed = PI * 4.0 * 2.0;
    let found = volume(&evaluation, pair.plate);
    assert!((PLATE - found - removed).abs() < 0.01 * removed, "{found}");
}

fn slotted(
    pair: &mut Pair,
    at: (f64, f64),
    style: HoleStyle,
    length: &str,
    angle: &str,
) -> FeatureId {
    let hole = drilled(
        pair,
        &[at],
        |_| style,
        |document| HoleDepth::Blind(expression(document, "2 mm")),
        false,
    );
    let kind = pair.document.feature(hole).unwrap().kind.clone();
    let FeatureKind::Hole(definition) = kind else {
        panic!("a hole");
    };
    let slot = Hole {
        shape: HoleShape::Slot {
            length: expression(&pair.document, length),
            angle: expression(&pair.document, angle),
        },
        ..definition
    };
    pair.document
        .apply(Transaction::single(
            "Slot",
            Edit::SetFeatureKind {
                id: hole,
                kind: FeatureKind::Hole(slot),
            },
        ))
        .unwrap();
    hole
}

fn stadium(length: f64, diameter: f64) -> f64 {
    length * diameter + PI * (diameter / 2.0).powi(2)
}

#[test]
fn a_slot_removes_a_stadium_of_its_length_and_diameter() {
    let mut pair = pair();
    slotted(&mut pair, (10.0, 5.0), HoleStyle::Plain, "6 mm", "0 deg");
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let removed = stadium(6.0, 4.0) * 2.0;
    let found = volume(&evaluation, pair.plate);
    assert!((PLATE - found - removed).abs() < 0.01 * removed, "{found}");
}

#[test]
fn a_counterbored_slot_steps_out_to_the_counterbore_at_its_mouth() {
    let mut pair = pair();
    let style = HoleStyle::Counterbore {
        diameter: expression(&pair.document, "6 mm"),
        depth: expression(&pair.document, "1 mm"),
    };
    slotted(&mut pair, (10.0, 5.0), style, "6 mm", "0 deg");
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let removed = stadium(6.0, 4.0) * 2.0 + (stadium(6.0, 6.0) - stadium(6.0, 4.0)) * 1.0;
    let found = volume(&evaluation, pair.plate);
    assert!((PLATE - found - removed).abs() < 0.01 * removed, "{found}");
}

#[test]
fn a_slot_turns_by_its_angle_in_the_sketch_plane() {
    let mut along = pair();
    slotted(&mut along, (2.5, 5.0), HoleStyle::Plain, "4 mm", "0 deg");
    let mut across = pair();
    slotted(&mut across, (2.5, 5.0), HoleStyle::Plain, "4 mm", "90 deg");
    let mut engine = Recompute::default();

    let clipped = PLATE - volume(&evaluate(&along.document, &mut engine), along.plate);
    let mut engine = Recompute::default();
    let whole = PLATE - volume(&evaluate(&across.document, &mut engine), across.plate);

    let full = stadium(4.0, 4.0) * 2.0;
    assert!((whole - full).abs() < 0.01 * full, "{whole}");
    assert!(clipped < whole - 0.5, "{clipped} {whole}");
}

#[test]
fn a_countersunk_slot_is_refused_in_words() {
    let mut pair = pair();
    let style = HoleStyle::Countersink {
        diameter: expression(&pair.document, "6 mm"),
        angle: expression(&pair.document, "90 deg"),
    };
    let hole = slotted(&mut pair, (10.0, 5.0), style, "6 mm", "0 deg");
    let mut engine = Recompute::default();

    let error = failure(&evaluate(&pair.document, &mut engine), hole);

    assert_eq!(
        error.reason,
        "A slot can be plain or counterbored, but not countersunk."
    );
}

#[test]
fn metric_sizes_give_clearance_and_tap_drill_diameters_and_name_the_thread() {
    let normal = HoleStandard {
        size: MetricSize::M3,
        fit: HoleFit::Normal,
    };
    let tapped = HoleStandard {
        fit: HoleFit::Tapped,
        ..normal
    };

    assert_eq!(normal.diameter(), 3.4);
    assert_eq!(normal.counterbore(), (6.5, 3.4));
    assert_eq!(normal.thread(), None);
    assert_eq!(normal.label(), "M3 normal fit");
    assert_eq!(tapped.diameter(), 2.5);
    assert_eq!(tapped.thread().as_deref(), Some("M3 × 0.5"));
    assert_eq!(tapped.label(), "M3 × 0.5 tapped");
    assert_eq!(
        HoleStandard {
            size: MetricSize::M8,
            fit: HoleFit::Tapped
        }
        .thread()
        .as_deref(),
        Some("M8 × 1.25")
    );
    assert!(MetricSize::ALL.iter().all(|size| {
        let fits = HoleFit::ALL.map(|fit| HoleStandard { size: *size, fit }.diameter());
        let [close, normal, loose, tap] = fits;
        tap < size.major_diameter()
            && size.major_diameter() < close
            && close < normal
            && normal < loose
            && HoleStandard {
                size: *size,
                fit: HoleFit::Loose,
            }
            .counterbore()
            .0 > loose
            && MetricSize::from_name(size.name()) == Some(*size)
    }));
}
