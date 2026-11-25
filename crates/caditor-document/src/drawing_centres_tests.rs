use std::collections::BTreeMap;

use caditor_geometry::Point2;
use caditor_kernel::EdgeName;

use crate::{
    combine_tests::pair,
    drawing_callouts_tests::{drilled, lines_in, plate_drawing, round_edge_at},
    *,
};

fn near(left: Point2, right: Point2) -> bool {
    left.distance(right) < 1e-6
}

fn geometry_of(lines: &ViewLines) -> Vec<CentreGeometry> {
    centre_targets(lines)
        .into_iter()
        .map(|target| centre_geometry(target, lines).unwrap())
        .collect()
}

fn annotated(drawing: &Drawing, lines: &ViewLines, view: StandardView) -> SheetPlan {
    let drawn = DrawnView::Standard(view);
    let extents = BTreeMap::from([(drawn, lines.bounds.unwrap())]);
    let mut plan = lay_out_sheet(drawing, &TitleFields::default(), &extents, &BTreeMap::new());

    plan.annotate(
        drawing,
        &DrawnLines::from([(drawn, lines)]),
        &CalloutTexts::new(),
    );
    plan
}

#[test]
fn a_hole_seen_end_on_takes_one_centre_mark_whatever_circles_it_shows() {
    let drilled = drilled();
    let drawing = plate_drawing(&drilled.pair);

    let lines = lines_in(&drilled.pair.document, &drawing, StandardView::Top);

    let found = geometry_of(&lines);

    assert_eq!(found.len(), 1, "{found:?}");
    assert!(matches!(
        found[0],
        CentreGeometry::Mark { centre, radius }
            if near(centre, Point2::new(5.0, 5.0)) && (radius - 2.0).abs() < 1e-6
    ));
}

#[test]
fn a_hole_seen_from_the_side_takes_a_centreline_along_its_axis() {
    let drilled = drilled();
    let drawing = plate_drawing(&drilled.pair);

    let lines = lines_in(&drilled.pair.document, &drawing, StandardView::Front);

    let lines_found: Vec<(Point2, Point2)> = geometry_of(&lines)
        .into_iter()
        .filter_map(|geometry| match geometry {
            CentreGeometry::Line { from, to } => Some((from, to)),
            CentreGeometry::Mark { .. } => None,
        })
        .collect();

    assert_eq!(lines_found.len(), 1, "{lines_found:?}");

    let (from, to) = lines_found[0];

    let (low, high) = if from.y < to.y {
        (from, to)
    } else {
        (to, from)
    };

    assert!(near(low, Point2::new(5.0, 1.0)), "{low:?}");
    assert!(near(high, Point2::new(5.0, 4.0)), "{high:?}");
}

#[test]
fn a_plain_plate_has_nothing_to_mark() {
    let pair = pair();
    let drawing = plate_drawing(&pair);

    for view in [StandardView::Top, StandardView::Front] {
        assert!(centre_targets(&lines_in(&pair.document, &drawing, view)).is_empty());
    }
}

#[test]
fn a_centre_mark_crosses_its_circle_past_the_rim_and_is_kept_lost_when_its_edge_goes() {
    let drilled = drilled();
    let mut drawing = plate_drawing(&drilled.pair);

    let lines = lines_in(&drilled.pair.document, &drawing, StandardView::Top);
    let rim = round_edge_at(&lines, Point2::new(5.0, 5.0));
    let view = DrawnView::Standard(StandardView::Top);

    drawing.views = [StandardView::Top].into_iter().collect();
    drawing.centres.push(CentreMark {
        id: AnnotationId::from_raw(0),
        view,
        target: CentreTarget::Circle(rim),
    });

    assert!(new_centres(&drawing, view, &lines).is_empty());

    let plan = annotated(&drawing, &lines, StandardView::Top);
    let placed = plan.placed(view).unwrap();
    let laid = plan
        .annotation(Annotation::Centre(AnnotationId::from_raw(0)))
        .unwrap();
    let arm = 2.0 * placed.scale + CENTRE_OVERSHOOT;
    let centre = placed.on_sheet(Point2::new(5.0, 5.0));

    assert_eq!(laid.problem, None);
    assert_eq!(laid.marks.len(), 2);
    assert!(laid.marks.iter().all(|mark| match *mark {
        SheetMark::Line(from, to) => {
            (from.distance(centre) - arm).abs() < 1e-6 && (to.distance(centre) - arm).abs() < 1e-6
        }
        _ => false,
    }));

    drawing.centres[0].target = CentreTarget::Circle(EdgeName::from_digest(0xdead));

    let plan = annotated(&drawing, &lines, StandardView::Top);
    let laid = plan
        .annotation(Annotation::Centre(AnnotationId::from_raw(0)))
        .unwrap();

    assert_eq!(laid.problem, Some(AnnotationProblem::EdgeGone));
    assert!(laid.marks.is_empty());
    assert_eq!(laid.bounds(), None);
    assert_eq!(plan.annotation_at(centre, 10.0), None);
}

#[test]
fn a_centre_mark_follows_the_hole_when_its_diameter_changes() {
    let mut drilled = drilled();

    let drawing = plate_drawing(&drilled.pair);
    let before = lines_in(&drilled.pair.document, &drawing, StandardView::Top);
    let rim = round_edge_at(&before, Point2::new(5.0, 5.0));

    let mut hole = drilled
        .pair
        .document
        .feature(drilled.hole)
        .unwrap()
        .kind
        .hole()
        .unwrap()
        .clone();

    hole.diameter = drilled.pair.document.parse("6 mm").unwrap();
    drilled
        .pair
        .document
        .apply(Transaction::single(
            "Widen",
            Edit::SetFeatureKind {
                id: drilled.hole,
                kind: FeatureKind::Hole(hole),
            },
        ))
        .unwrap();

    let after = lines_in(&drilled.pair.document, &drawing, StandardView::Top);

    assert!(matches!(
        centre_geometry(CentreTarget::Circle(rim), &after),
        Ok(CentreGeometry::Mark { radius, .. }) if (radius - 3.0).abs() < 1e-6
    ));
}
