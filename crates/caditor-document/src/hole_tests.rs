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
        sizing: HoleSizing::Typed,
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
    let hole = transaction.add_feature(
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
            sizing: HoleSizing::Typed,
        }),
    );
    pair.document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(evaluation.cuts(hole).len(), 2);
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
            sizing: HoleSizing::Typed,
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
        sizing: HoleSizing::Typed,
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
        "A slot can be plain, counterbored or stepped, but not countersunk."
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
    let fine = HoleStandard {
        size: MetricSize::M10,
        fit: HoleFit::TappedFine(FinePitch::First),
    };
    assert_eq!(fine.diameter(), 8.75);
    assert_eq!(fine.label(), "M10 × 1.25 tapped");
    assert_eq!(
        HoleStandard {
            size: MetricSize::M2_5,
            fit: HoleFit::TappedFine(FinePitch::First)
        }
        .diameter(),
        2.15
    );
    assert_eq!(
        HoleFit::from_id("tapped_fine"),
        Some(HoleFit::TappedFine(FinePitch::First))
    );
    assert!(MetricSize::ALL.iter().all(|size| {
        let diameter = |fit| HoleStandard { size: *size, fit }.diameter();
        let [close, normal, loose, tap] = [
            HoleFit::Close,
            HoleFit::Normal,
            HoleFit::Loose,
            HoleFit::Tapped,
        ]
        .map(diameter);
        size.fine_fits().into_iter().all(|fine| {
            let pitch = HoleStandard {
                size: *size,
                fit: fine,
            }
            .thread_pitch()
            .unwrap();
            tap < diameter(fine) && diameter(fine) < size.major_diameter() && pitch < size.pitch()
        }) && size.fine_pitch() < size.pitch()
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

fn circled(
    pair: &mut Pair,
    sizing: HoleSizing,
    style: impl FnOnce(&Document) -> HoleStyle,
) -> FeatureId {
    let mut sketch = Sketch::new(top());
    sketch.add_circle(Point2::new(4.0, 5.0), 1.0);
    sketch.add_circle(Point2::new(10.0, 5.0), 2.5);
    sketch.add_point(Point2::new(16.0, 5.0));
    let mut transaction = pair.document.transaction("Drill");
    let sketch_id = transaction.add_feature("Hole sketch", FeatureKind::from(sketch));
    let hole = Hole {
        sketch: sketch_id,
        body: pair.plate,
        diameter: expression(&pair.document, "2 mm"),
        depth: HoleDepth::ThroughAll,
        style: style(&pair.document),
        reversed: false,
        shape: HoleShape::Round,
        standard: None,
        sizing,
    };
    let feature = transaction.add_feature("Hole 1", FeatureKind::Hole(hole));
    pair.document.apply(transaction.finish()).unwrap();
    feature
}

#[test]
fn holes_sized_by_circles_take_each_circle_s_diameter_and_points_the_typed_one() {
    let mut typed = pair();
    circled(&mut typed, HoleSizing::Typed, |_| HoleStyle::Plain);
    let mut by_circles = pair();
    circled(&mut by_circles, HoleSizing::Circles, |_| HoleStyle::Plain);

    let typed_evaluation = evaluate(&typed.document, &mut Recompute::default());
    let circled_evaluation = evaluate(&by_circles.document, &mut Recompute::default());

    let all_typed = 3.0 * PI * 1.0 * 1.0 * 4.0;
    let each_own = PI * (1.0 + 2.5 * 2.5 + 1.0) * 4.0;
    let typed_found = PLATE - volume(&typed_evaluation, typed.plate);
    let circled_found = PLATE - volume(&circled_evaluation, by_circles.plate);
    assert_eq!(circled_evaluation.failed_count(), 0);
    assert!(
        (typed_found - all_typed).abs() < 0.01 * all_typed,
        "{typed_found}"
    );
    assert!(
        (circled_found - each_own).abs() < 0.01 * each_own,
        "{circled_found}"
    );
}

#[test]
fn construction_circles_size_nothing() {
    let mut sketch = Sketch::new(top());
    let drawn = sketch.add_circle(Point2::new(10.0, 5.0), 2.0);
    let guide = sketch.add_circle(Point2::new(3.0, 3.0), 1.0);
    sketch.set_construction(guide, true).unwrap();

    let sizes = circle_sizes(&sketch);

    assert_eq!(
        sizes.values().copied().collect::<Vec<_>>(),
        [CircleSize {
            circle: drawn,
            diameter: 4.0
        }]
    );
}

#[test]
fn a_counterbore_narrower_than_a_circle_names_the_circle() {
    let mut pair = pair();
    let hole = circled(&mut pair, HoleSizing::Circles, |document| {
        HoleStyle::Counterbore {
            diameter: expression(document, "4.5 mm"),
            depth: expression(document, "1 mm"),
        }
    });

    let evaluation = evaluate(&pair.document, &mut Recompute::default());

    assert_eq!(
        failure(&evaluation, hole).reason,
        "The counterbore is not wider than Circle 3."
    );
}

#[test]
fn sizes_offer_every_iso_fine_pitch_and_the_first_one_reads_as_before() {
    let pitches = |size: MetricSize| {
        size.fine_fits()
            .into_iter()
            .map(|fit| HoleStandard { size, fit }.thread().unwrap())
            .collect::<Vec<_>>()
    };
    let second = HoleStandard {
        size: MetricSize::M10,
        fit: HoleFit::TappedFine(FinePitch::Second),
    };

    assert_eq!(pitches(MetricSize::M3), ["M3 × 0.35"]);
    assert_eq!(pitches(MetricSize::M8), ["M8 × 1", "M8 × 0.75"]);
    assert_eq!(
        pitches(MetricSize::M10),
        ["M10 × 1.25", "M10 × 1", "M10 × 0.75"]
    );
    assert_eq!(
        pitches(MetricSize::M12),
        ["M12 × 1.25", "M12 × 1.5", "M12 × 1"]
    );
    assert_eq!(
        pitches(MetricSize::M20),
        ["M20 × 1.5", "M20 × 2", "M20 × 1"]
    );
    assert_eq!(second.diameter(), 9.0);
    assert_eq!(second.fit.id(), "tapped_fine_2");
    assert_eq!(
        HoleFit::from_id("tapped_fine_3"),
        Some(HoleFit::TappedFine(FinePitch::Third))
    );
    assert!(!MetricSize::M3.offers(HoleFit::TappedFine(FinePitch::Second)));
    assert_eq!(
        HoleStandard::offered(MetricSize::M3, second.fit),
        HoleStandard {
            size: MetricSize::M3,
            fit: HoleFit::TappedFine(FinePitch::First)
        }
    );
}

#[test]
fn heat_set_insert_holes_take_the_common_insert_bores_from_m2_to_m8() {
    let bore = |size| {
        HoleStandard {
            size,
            fit: HoleFit::HeatSetInsert,
        }
        .diameter()
    };
    let m3 = HoleStandard {
        size: MetricSize::M3,
        fit: HoleFit::HeatSetInsert,
    };

    assert_eq!(
        [
            MetricSize::M2,
            MetricSize::M2_5,
            MetricSize::M3,
            MetricSize::M4,
            MetricSize::M5,
            MetricSize::M6,
            MetricSize::M8
        ]
        .map(bore),
        [3.2, 4.0, 4.0, 5.6, 6.4, 8.0, 9.7]
    );
    assert_eq!(m3.insert().map(|insert| insert.length), Some(5.7));
    assert_eq!(m3.thread(), None);
    assert_eq!(m3.label(), "M3 heat-set insert");
    assert_eq!(
        HoleFit::from_id("heat_set_insert"),
        Some(HoleFit::HeatSetInsert)
    );
    assert!(!MetricSize::M1_6.offers(HoleFit::HeatSetInsert));
    assert!(!MetricSize::M10.fits().contains(&HoleFit::HeatSetInsert));
    assert_eq!(
        HoleStandard::offered(MetricSize::M10, HoleFit::HeatSetInsert).fit,
        HoleFit::Normal
    );
    assert!(MetricSize::ALL.iter().all(|size| {
        size.heat_set_insert()
            .is_none_or(|insert| insert.hole > size.major_diameter())
    }));
}

#[test]
fn holes_scaled_by_circles_grow_their_counterbore_with_each_circle() {
    let counterbore = |document: &Document| HoleStyle::Counterbore {
        diameter: expression(document, "3 mm"),
        depth: expression(document, "1 mm"),
    };
    let mut kept = pair();
    let kept_hole = circled(&mut kept, HoleSizing::Circles, counterbore);
    let mut scaled = pair();
    circled(&mut scaled, HoleSizing::CirclesAndHeads, counterbore);

    let kept_evaluation = evaluate(&kept.document, &mut Recompute::default());
    let scaled_evaluation = evaluate(&scaled.document, &mut Recompute::default());

    let typed = PI * (1.0 * 3.0 + 1.5 * 1.5 * 1.0);
    let wide = PI * (2.5 * 2.5 * 1.5 + 3.75 * 3.75 * 2.5);
    let expected = 2.0 * typed + wide;
    let found = PLATE - volume(&scaled_evaluation, scaled.plate);
    assert_eq!(
        failure(&kept_evaluation, kept_hole).reason,
        "The counterbore is not wider than Circle 3."
    );
    assert_eq!(scaled_evaluation.failed_count(), 0);
    assert!(
        (found - expected).abs() < 0.01 * expected,
        "{found} {expected}"
    );
    assert!(HoleSizing::CirclesAndHeads.by_circles());
    assert!(!HoleSizing::Typed.by_circles());
}

fn steps(document: &Document, sizes: &[(&str, &str)]) -> HoleStyle {
    HoleStyle::Stepped(
        sizes
            .iter()
            .map(|(diameter, depth)| HoleStep {
                diameter: expression(document, diameter),
                depth: expression(document, depth),
            })
            .collect(),
    )
}

fn stepped(pair: &mut Pair, sizes: &[(&str, &str)], depth: &str) -> FeatureId {
    drilled(
        pair,
        &[(5.0, 5.0)],
        |document| steps(document, sizes),
        |document| HoleDepth::Blind(expression(document, depth)),
        false,
    )
}

fn hole_parts(evaluation: &Evaluation, pair: &Pair) -> BTreeSet<String> {
    evaluation
        .body(pair.plate)
        .unwrap()
        .faces()
        .map(|(_, face)| describe_origin(&pair.document, face.origin()))
        .filter(|name| name.starts_with("Hole 1"))
        .collect()
}

#[test]
fn a_stepped_hole_narrows_step_by_step_and_names_each_step() {
    let mut pair = pair();
    stepped(&mut pair, &[("8 mm", "0.5 mm"), ("6 mm", "1 mm")], "3 mm");
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let removed = PI * (16.0 * 0.5 + 9.0 * 1.0 + 4.0 * 1.5);
    let found = volume(&evaluation, pair.plate);
    assert!((PLATE - found - removed).abs() < 0.01 * removed, "{found}");
    assert_eq!(
        hole_parts(&evaluation, &pair),
        [
            "Hole 1 bottom",
            "Hole 1 counterbore floor",
            "Hole 1 counterbore wall",
            "Hole 1 step 2 floor",
            "Hole 1 step 2 wall",
            "Hole 1 wall",
        ]
        .map(str::to_owned)
        .into()
    );
}

#[test]
fn a_counterbore_turned_into_steps_keeps_its_top_step_and_wall() {
    let mut pair = pair();
    let hole = drilled(
        &mut pair,
        &[(5.0, 5.0)],
        |document| HoleStyle::Counterbore {
            diameter: expression(document, "8 mm"),
            depth: expression(document, "0.5 mm"),
        },
        |document| HoleDepth::Blind(expression(document, "3 mm")),
        false,
    );
    let mut engine = Recompute::default();
    let bored = evaluate(&pair.document, &mut engine)
        .body(pair.plate)
        .unwrap()
        .clone();
    let named = |name: &str| {
        let face = bored
            .faces()
            .find(|(_, face)| describe_origin(&pair.document, face.origin()) == name)
            .map(|(id, _)| id)
            .unwrap();
        caditor_kernel::FaceReference::capture(&bored, face).unwrap()
    };
    let references = [
        named("Hole 1 counterbore wall"),
        named("Hole 1 counterbore floor"),
        named("Hole 1 wall"),
    ];
    let FeatureKind::Hole(definition) = pair.document.feature(hole).unwrap().kind.clone() else {
        panic!("a hole");
    };
    let style = steps(&pair.document, &[("8 mm", "0.5 mm"), ("6 mm", "1 mm")]);
    pair.document
        .apply(Transaction::single(
            "Steps",
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

    let stepped = evaluation.body(pair.plate).unwrap();
    let found: Vec<String> = references
        .iter()
        .map(|reference| {
            let face = stepped.face(reference.resolve(stepped).unwrap()).unwrap();
            describe_origin(&pair.document, face.origin())
        })
        .collect();
    assert_eq!(
        found,
        [
            "Hole 1 counterbore wall",
            "Hole 1 counterbore floor",
            "Hole 1 wall"
        ]
    );
}

#[test]
fn steps_that_widen_reach_the_bottom_or_no_wider_than_the_hole_are_refused() {
    let cases = [
        (
            vec![("6 mm", "0.5 mm"), ("7 mm", "1 mm")],
            "3 mm",
            "Step 2 is not narrower than step 1 above it.",
        ),
        (
            vec![("8 mm", "1 mm"), ("6 mm", "2 mm")],
            "3 mm",
            "The steps together are as deep as the whole hole.",
        ),
        (
            vec![("8 mm", "0.5 mm"), ("4 mm", "1 mm")],
            "3 mm",
            "Step 2 is not wider than the hole.",
        ),
        (
            vec![("8 mm", "0 mm")],
            "3 mm",
            "The step 1 depth must be more than zero.",
        ),
        (
            Vec::new(),
            "3 mm",
            "A stepped hole has 0 steps, but it needs from 1 to 8.",
        ),
    ];
    for (sizes, depth, reason) in cases {
        let mut pair = pair();
        let hole = stepped(&mut pair, &sizes, depth);
        let mut engine = Recompute::default();

        let error = failure(&evaluate(&pair.document, &mut engine), hole);

        assert_eq!(error.reason, reason);
    }
}

#[test]
fn a_stepped_slot_steps_out_at_each_step() {
    let mut pair = pair();
    let style = steps(&pair.document, &[("8 mm", "0.5 mm"), ("6 mm", "0.5 mm")]);
    slotted(&mut pair, (10.0, 5.0), style, "6 mm", "0 deg");
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    let removed =
        stadium(6.0, 8.0) * 0.5 + stadium(6.0, 6.0) * 0.5 + stadium(6.0, 4.0) * (2.0 - 1.0);
    let found = volume(&evaluation, pair.plate);
    assert!((PLATE - found - removed).abs() < 0.01 * removed, "{found}");
}
