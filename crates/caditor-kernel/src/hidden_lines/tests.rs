use std::f64::consts::{FRAC_PI_2, PI, TAU};

use caditor_geometry::{Maths, Point2, Point3, RigidTransform, Vector3};

use super::{outline::outer_outline_on, *};
use crate::{
    fixtures::{
        Fixture, Tweak, add_cuboid, cone, cuboid, cylinder, extruded_spline, frustum, holed_block,
        perforated_plate, sphere, spline_topped_block, torus,
    },
    tally,
    test_support::{assert_cancelled_anywhere, polls_during},
};

const CLOSE: f64 = 1e-6;

fn tilted_from_axis(degrees: f64) -> HiddenLineView {
    let (sin, cos) = degrees.to_radians().sin_cos_libm();
    HiddenLineView::new(Vector3::new(0.0, sin, -cos), Vector3::new(0.0, cos, sin)).unwrap()
}

fn tolerance() -> SamplingTolerance {
    SamplingTolerance::new(0.01, 0.2).unwrap()
}

fn from_front() -> HiddenLineView {
    HiddenLineView::new(Vector3::Y, Vector3::Z).unwrap()
}

fn from_above() -> HiddenLineView {
    HiddenLineView::new(Vector3::NEG_Z, Vector3::Y).unwrap()
}

fn lines_of(solid: &Solid, view: &HiddenLineView) -> HiddenLines {
    let lines = hidden_lines(&[solid], view, &tolerance()).unwrap();
    assert_eq!(lines.undecided, 0, "no piece may be left undecided");
    lines
}

fn total(
    pieces: impl IntoIterator<Item = impl std::borrow::Borrow<ViewPiece>>,
    visibility: Visibility,
) -> f64 {
    pieces
        .into_iter()
        .filter(|piece| piece.borrow().visibility == visibility)
        .map(|piece| piece.borrow().length())
        .sum()
}

fn is_silhouette(piece: &ViewPiece) -> bool {
    matches!(piece.source, ViewSource::Silhouette { .. })
}

fn silhouettes(lines: &HiddenLines) -> Vec<&ViewPiece> {
    lines
        .pieces
        .iter()
        .filter(|piece| is_silhouette(piece))
        .collect()
}

fn edges(lines: &HiddenLines) -> Vec<&ViewPiece> {
    lines
        .pieces
        .iter()
        .filter(|piece| !is_silhouette(piece))
        .collect()
}

fn ends(piece: &ViewPiece) -> (Point2, Point2) {
    (
        piece.curve.point(piece.range.start()),
        piece.curve.point(piece.range.end()),
    )
}

fn middle(piece: &ViewPiece) -> Point2 {
    piece.curve.point(piece.range.middle())
}

fn assert_close(found: f64, wanted: f64, what: &str) {
    assert!(
        (found - wanted).abs() <= CLOSE,
        "{what}: {found} is not {wanted}"
    );
}

fn edge_totals(solid: &Solid, lines: &HiddenLines, view: &HiddenLineView) {
    let frame = view.frame();
    for (id, edge) in solid.edges() {
        let pieces: Vec<&ViewPiece> = lines
            .pieces
            .iter()
            .filter(|piece| matches!(piece.source, ViewSource::Edge { edge, .. } if edge == id))
            .collect();
        let seen = total(pieces.iter().copied(), Visibility::Visible);
        let hidden = total(pieces.iter().copied(), Visibility::Hidden);
        let projected = projected_length(edge.curve(), edge.interval(), frame);
        assert!(
            (seen + hidden - projected).abs() <= 2e-3 * (1.0 + projected),
            "edge {id:?}: visible {seen} and hidden {hidden} do not add up to {projected}"
        );
    }
}

fn projected_length(curve: &Curve, range: Interval, frame: &caditor_geometry::Plane) -> f64 {
    const STEPS: usize = 20_000;
    let mut last = frame.to_local(curve.point(range.start()));
    let mut length = 0.0;
    for step in 1..=STEPS {
        let at = frame.to_local(curve.point(range.at(step as f64 / STEPS as f64)));
        length += at.distance(last);
        last = at;
    }
    length
}

#[test]
fn a_block_seen_along_its_hole_shows_the_top_and_hides_the_bottom() {
    let solid = holed_block(10.0, 4.0, 2.5);
    let view = from_above();

    let lines = lines_of(&solid, &view);

    assert!(
        silhouettes(&lines).is_empty(),
        "a bore seen along has no silhouette"
    );
    assert_close(
        total(edges(&lines), Visibility::Visible),
        40.0 + TAU * 2.5,
        "the top rectangle and rim",
    );
    assert_close(
        total(edges(&lines), Visibility::Hidden),
        40.0 + TAU * 2.5,
        "the bottom rectangle and rim",
    );
    for piece in edges(&lines) {
        if let Curve2::Circle(circle) = &piece.curve {
            assert_close(circle.radius(), 2.5, "the rim stays a circle");
            assert!(piece.exact);
        }
    }
    edge_totals(&solid, &lines, &view);
}

#[test]
fn a_block_seen_across_its_hole_hides_the_bore_behind_its_front_face() {
    let solid = holed_block(10.0, 4.0, 2.5);
    let view = from_front();

    let lines = lines_of(&solid, &view);

    let bore = silhouettes(&lines);
    assert_eq!(bore.len(), 2, "two lines bound the bore: {bore:#?}");
    for piece in &bore {
        assert_eq!(piece.visibility, Visibility::Hidden);
        assert_close(piece.length(), 4.0, "a bore line runs through the block");
        assert!(matches!(piece.curve, Curve2::Line(_)));
    }
    let xs: Vec<f64> = bore.iter().map(|piece| middle(piece).x).collect();
    assert!(
        xs.iter()
            .any(|x| (x + 2.5).abs() <= CLOSE || (x - 2.5).abs() <= CLOSE)
    );
    let sharp_seen: f64 = edges(&lines)
        .iter()
        .filter(|piece| {
            piece.visibility == Visibility::Visible
                && matches!(
                    piece.source,
                    ViewSource::Edge {
                        kind: ViewEdgeKind::Sharp,
                        ..
                    }
                )
        })
        .map(|piece| piece.length())
        .sum();
    assert_close(sharp_seen, 28.0, "the front face's outline");
    edge_totals(&solid, &lines, &view);
}

#[test]
fn a_cylinder_seen_side_on_has_two_silhouette_lines() {
    let solid = cylinder(3.0, 5.0);
    let view = from_front();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert_eq!(outline.len(), 2);
    for piece in &outline {
        assert_eq!(piece.visibility, Visibility::Visible);
        assert!(piece.exact);
        assert!(matches!(piece.curve, Curve2::Line(_)));
        assert_close(piece.length(), 5.0, "a silhouette line");
        assert_close(
            middle(piece).x.abs(),
            3.0,
            "a silhouette's distance from the axis",
        );
    }
    assert_close(
        total(edges(&lines), Visibility::Hidden) + total(edges(&lines), Visibility::Visible),
        24.0 + 5.0,
        "two rims seen as lines, twice across each, and the seam",
    );
    edge_totals(&solid, &lines, &view);
}

#[test]
fn a_cylinder_seen_end_on_is_a_circle() {
    let solid = cylinder(3.0, 5.0);
    let view = from_above();

    let lines = lines_of(&solid, &view);

    assert!(silhouettes(&lines).is_empty());
    let visible: Vec<&ViewPiece> = lines
        .pieces
        .iter()
        .filter(|piece| piece.visibility == Visibility::Visible)
        .collect();
    assert_eq!(visible.len(), 1, "only the top rim shows: {visible:#?}");
    let Curve2::Circle(circle) = &visible.first().unwrap().curve else {
        panic!("a rim seen square stays a circle");
    };
    assert_close(circle.radius(), 3.0, "its radius");
    assert_close(
        visible.first().unwrap().length(),
        TAU * 3.0,
        "the whole rim",
    );
    edge_totals(&solid, &lines, &view);
}

#[test]
fn a_tilted_cylinder_breaks_its_far_rim_where_the_silhouette_meets_it() {
    let solid = cylinder(3.0, 5.0);
    let view = HiddenLineView::new(
        Vector3::new(0.0, 1.0, -1.0).normalize(),
        Vector3::new(0.0, 1.0, 1.0),
    )
    .unwrap();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert_eq!(outline.len(), 2);
    for piece in &outline {
        assert_eq!(piece.visibility, Visibility::Visible);
    }
    let ellipses: Vec<&ViewPiece> = edges(&lines)
        .into_iter()
        .filter(|piece| matches!(piece.curve, Curve2::Ellipse(_)))
        .collect();
    let half = PI * 3.0;
    let ellipse_half = {
        let minor = 3.0 * FRAC_PI_2.cos_libm().abs().max(0.5_f64.sqrt());
        let Curve2::Ellipse(any) = &ellipses.first().unwrap().curve else {
            panic!("an oblique rim is an ellipse");
        };
        assert_close(any.minor_radius(), minor, "the rim's minor radius");
        assert_close(any.major_radius(), 3.0, "the rim's major radius");
        0.5 * ellipses.first().unwrap().curve.length(Interval::FULL_TURN)
    };
    assert!(ellipse_half < half);
    assert_close(
        total(ellipses.iter().copied(), Visibility::Hidden),
        ellipse_half,
        "the back half of the bottom rim",
    );
    assert_close(
        total(ellipses.iter().copied(), Visibility::Visible),
        3.0 * ellipse_half,
        "the whole top rim and the front half of the bottom one",
    );
    edge_totals(&solid, &lines, &view);
}

#[test]
fn a_sphere_has_one_silhouette_circle() {
    let solid = sphere(4.0);
    let view = HiddenLineView::new(Vector3::new(1.0, 2.0, -0.5).normalize(), Vector3::Z).unwrap();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert_eq!(outline.len(), 1);
    let piece = outline.first().unwrap();
    assert_eq!(piece.visibility, Visibility::Visible);
    assert!(matches!(piece.curve, Curve2::Circle(_)));
    assert_close(piece.length(), TAU * 4.0, "the whole circle");
    edge_totals(&solid, &lines, &view);
}

#[test]
fn a_sphere_seen_along_its_seam_plane_keeps_one_whole_silhouette() {
    let solid = sphere(4.0);
    let view = from_front();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert_eq!(outline.len(), 1, "{outline:#?}");
    assert_eq!(outline.first().unwrap().visibility, Visibility::Visible);
    assert_close(outline.first().unwrap().length(), TAU * 4.0, "the circle");
}

#[test]
fn a_torus_seen_along_its_axis_has_two_silhouette_circles() {
    let solid = torus(6.0, 2.0);
    let view = from_above();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert_eq!(outline.len(), 2);
    let mut radii: Vec<f64> = outline
        .iter()
        .map(|piece| match &piece.curve {
            Curve2::Circle(circle) => circle.radius(),
            other => panic!("not a circle: {other:?}"),
        })
        .collect();
    radii.sort_by(f64::total_cmp);
    assert_close(*radii.first().unwrap(), 4.0, "the hole");
    assert_close(*radii.last().unwrap(), 8.0, "the rim");
    assert!(
        outline
            .iter()
            .all(|piece| piece.visibility == Visibility::Visible)
    );
}

#[test]
fn a_torus_seen_side_on_hides_the_inner_half_of_its_tube_circles() {
    let solid = torus(6.0, 2.0);
    let view = from_front();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    let seen = total(outline.iter().copied(), Visibility::Visible);
    assert_close(
        seen,
        2.0 * 12.0 + TAU * 2.0,
        "two lines and two outer half circles",
    );
    for piece in &outline {
        if piece.visibility == Visibility::Visible {
            assert!(
                middle(piece).x.abs() >= 6.0 - CLOSE || matches!(piece.curve, Curve2::Line(_)),
                "{piece:#?}"
            );
        }
    }
    let hidden = total(outline.iter().copied(), Visibility::Hidden);
    assert!(
        hidden >= TAU * 2.0 - CLOSE,
        "the inner half circles hide: {hidden}"
    );
}

#[test]
fn a_tilted_torus_is_traced_within_the_tolerance_and_splits_where_the_outline_folds() {
    let solid = torus(6.0, 2.0);
    let view = tilted_from_axis(70.0);

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert!(outline.iter().all(|piece| !piece.exact));
    let mut outer = 0.0_f64;
    for piece in &outline {
        for step in 0..=32 {
            let at = piece.curve.point(piece.range.at(step as f64 / 32.0));
            outer = outer.max(at.x.abs());
            assert!(at.x.abs() <= 8.0 + 0.011, "{at:?}");
        }
    }
    assert!(
        (outer - 8.0).abs() <= 0.02,
        "the outline reaches the rim: {outer}"
    );
    assert!(total(outline.iter().copied(), Visibility::Hidden) > 0.0);
    assert!(total(outline.iter().copied(), Visibility::Visible) > TAU * 6.0);
    edge_totals(&solid, &lines, &view);
}

#[test]
fn a_torus_tilted_short_of_its_folding_angle_shows_its_whole_outline() {
    let solid = torus(6.0, 2.0);
    let lines = lines_of(&solid, &tilted_from_axis(45.0));

    let outline = silhouettes(&lines);
    assert!(outline.iter().all(|piece| !piece.exact));
    assert_close(
        total(outline.iter().copied(), Visibility::Hidden),
        0.0,
        "tan² of the tilt stays below (R - r) / r",
    );
}

#[test]
fn a_cone_seen_side_on_has_silhouette_rulings_to_its_apex() {
    let solid = cone(3.0, 4.0);
    let view = from_front();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert_eq!(outline.len(), 2);
    for piece in &outline {
        assert_eq!(piece.visibility, Visibility::Visible);
        assert_close(piece.length(), 5.0, "a ruling from rim to apex");
        assert!(matches!(piece.curve, Curve2::Line(_)));
    }
}

#[test]
fn a_cone_seen_from_its_tip_within_its_angle_has_no_silhouette() {
    let solid = frustum(4.0, 2.0, 5.0);
    let view = from_above();

    let lines = lines_of(&solid, &view);

    assert!(silhouettes(&lines).is_empty());
    assert_close(
        total(edges(&lines), Visibility::Visible),
        TAU * 2.0 + TAU * 4.0 + 2.0,
        "both rims and the seam",
    );
}

#[test]
fn one_solid_hides_part_of_another() {
    let mut fixture = Fixture::new();
    add_cuboid(
        &mut fixture,
        Point3::new(-1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 1.0),
        Tweak::default(),
    );
    let front = fixture.build();
    let mut fixture = Fixture::new();
    add_cuboid(
        &mut fixture,
        Point3::new(-3.0, 5.0, 0.25),
        Point3::new(3.0, 6.0, 0.75),
        Tweak::default(),
    );
    let back = fixture.build();
    let view = from_front();

    let lines = hidden_lines(&[&front, &back], &view, &tolerance()).unwrap();

    assert_eq!(lines.undecided, 0);
    let long_seen: f64 = lines
        .pieces
        .iter()
        .filter(|piece| {
            matches!(piece.source, ViewSource::Edge { solid: 1, .. })
                && piece.visibility == Visibility::Visible
        })
        .map(ViewPiece::length)
        .sum();
    assert_close(
        long_seen,
        2.0 * (2.0 + 2.0) + 2.0 * 0.5,
        "the back block's ends stick out",
    );
    let hidden_behind = lines.pieces.iter().find(|piece| {
        matches!(piece.source, ViewSource::Edge { solid: 1, .. })
            && piece.visibility == Visibility::Hidden
            && piece.length() > 1.9
            && piece.length() < 2.1
    });
    let behind = hidden_behind.expect("a stretch of a long edge hides behind the front block");
    let (start, end) = ends(behind);
    assert_close(start.x.abs(), 1.0, "it starts at the front block's side");
    assert_close(end.x.abs(), 1.0, "it ends at the front block's side");
    edge_totals(
        &back,
        &HiddenLines {
            pieces: lines
                .pieces
                .iter()
                .filter(|piece| matches!(piece.source, ViewSource::Edge { solid: 1, .. }))
                .cloned()
                .collect(),
            chord: lines.chord,
            undecided: 0,
        },
        &view,
    );
}

#[test]
fn an_edge_entering_another_solid_splits_where_it_pierces_a_face() {
    let mut fixture = Fixture::new();
    add_cuboid(
        &mut fixture,
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(4.0, 1.0, 1.0),
        Tweak::default(),
    );
    let long = fixture.build();
    let mut fixture = Fixture::new();
    add_cuboid(
        &mut fixture,
        Point3::new(2.0, -1.0, -1.0),
        Point3::new(6.0, 2.0, 2.0),
        Tweak::default(),
    );
    let large = fixture.build();
    let view = HiddenLineView::new(Vector3::new(0.2, 1.0, -0.4).normalize(), Vector3::Z).unwrap();

    let lines = hidden_lines(&[&long, &large], &view, &tolerance()).unwrap();

    let sides: Vec<&ViewPiece> = lines
        .pieces
        .iter()
        .filter(|piece| matches!(piece.source, ViewSource::Edge { solid: 0, .. }))
        .filter(|piece| piece.visibility == Visibility::Hidden)
        .collect();
    assert!(
        !sides.is_empty(),
        "the part of the bar inside the block is hidden"
    );
}

#[test]
fn an_extruded_spline_has_traced_free_ruling_silhouettes_on_its_arch() {
    let solid = extruded_spline(5.0);
    let view = from_front();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert!(!outline.is_empty(), "the arch's sides show as rulings");
    for piece in &outline {
        assert!(matches!(piece.curve, Curve2::Line(_)));
        assert_close(piece.length(), 5.0, "a ruling of the extrusion");
    }
    edge_totals(&solid, &lines, &view);
}

#[test]
fn a_spline_dome_is_traced_within_its_face() {
    let solid = spline_topped_block(10.0, 4.0, 6.0);
    let view = from_front();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert!(
        !outline.is_empty(),
        "the dome has an outline seen from the front"
    );
    assert!(outline.iter().all(|piece| !piece.exact));
    let mut highest = 0.0_f64;
    for piece in &outline {
        for step in 0..=16 {
            highest = highest.max(piece.curve.point(piece.range.at(step as f64 / 16.0)).y);
        }
    }
    assert!(
        highest > 4.0 + 1.0,
        "the outline rises over the dome: {highest}"
    );
    assert!(highest < 4.0 + 6.0, "under the control net: {highest}");
    edge_totals(&solid, &lines, &view);
}

#[test]
fn a_silhouette_line_breaks_where_a_cross_hole_takes_the_wall_away() {
    let wall = cylinder(3.0, 5.0);
    let turn = RigidTransform::rotation_about(Point3::ZERO, Vector3::Y, FRAC_PI_2).unwrap();
    let raise = RigidTransform::translation(Vector3::new(-5.0, 0.0, 2.5)).unwrap();
    let drill = cylinder(0.5, 10.0).transformed(&turn.then(&raise)).unwrap();
    let solid = crate::boolean(&wall, &drill, crate::BooleanOperation::Difference).unwrap();

    let lines = lines_of(&solid, &from_front());

    let walls: Vec<&ViewPiece> = silhouettes(&lines)
        .into_iter()
        .filter(|piece| matches!(piece.curve, Curve2::Line(_)))
        .filter(|piece| (middle(piece).x.abs() - 3.0).abs() <= CLOSE)
        .collect();
    assert_eq!(walls.len(), 4, "{walls:#?}");
    assert_close(
        walls.iter().map(|piece| piece.length()).sum::<f64>(),
        2.0 * (5.0 - 1.0),
        "both sides less the hole's width",
    );
    assert!(
        walls
            .iter()
            .all(|piece| piece.visibility == Visibility::Visible)
    );
}

fn dome_of_revolution() -> Solid {
    let curves = vec![
        crate::test_support::spline(
            1,
            &[(0.0, 0.0), (3.0, 1.0), (3.0, 3.0), (2.0, 5.0), (0.0, 6.0)],
        ),
        crate::test_support::line(2, (0.0, 6.0), (0.0, 0.0)),
    ];
    let regions = crate::Profile::new(&curves)
        .unwrap()
        .select(&crate::Selection::EvenDepth)
        .unwrap();
    crate::revolve(
        &caditor_geometry::Plane::XY,
        &regions,
        crate::Axis2::new(Point2::ZERO, caditor_geometry::Vector2::Y).unwrap(),
        crate::AngularExtent::full(),
        76,
    )
    .unwrap()
}

#[test]
fn a_surface_of_revolution_is_traced_by_its_widest_profile() {
    let solid = dome_of_revolution();
    let view = HiddenLineView::new(Vector3::NEG_Z, Vector3::Y).unwrap();

    let lines = lines_of(&solid, &view);

    let outline = silhouettes(&lines);
    assert!(!outline.is_empty());
    let seen: Vec<&&ViewPiece> = outline
        .iter()
        .filter(|piece| piece.visibility == Visibility::Visible)
        .collect();
    let mut widest = 0.0_f64;
    for piece in &seen {
        assert!(!piece.exact);
        for step in 0..=32 {
            widest = widest.max(
                piece
                    .curve
                    .point(piece.range.at(step as f64 / 32.0))
                    .x
                    .abs(),
            );
        }
    }
    assert!((2.3..=3.0).contains(&widest), "widest {widest}");
    edge_totals(&solid, &lines, &view);
}

#[test]
fn smooth_edges_are_flagged_tangent_and_seams_as_seams() {
    let solid = cylinder(3.0, 5.0);
    let view = from_front();

    let lines = lines_of(&solid, &view);

    let seam = lines.pieces.iter().find(|piece| {
        matches!(
            piece.source,
            ViewSource::Edge {
                kind: ViewEdgeKind::Seam,
                ..
            }
        )
    });
    assert!(
        seam.is_some(),
        "a cylinder's seam is told apart from its rims"
    );
    let block = extrude_box();
    let upright: Vec<EdgeId> = block
        .edges()
        .filter(
            |(_, edge)| matches!(edge.curve(), Curve::Line(line) if line.direction().z.abs() > 0.5),
        )
        .map(|(id, _)| id)
        .take(1)
        .collect();
    let rounded = crate::blend(
        &block,
        &upright,
        crate::BlendShape::Fillet { radius: 1.0 },
        77,
    )
    .unwrap();
    let lines = lines_of(&rounded, &tilted_from_axis(60.0));
    assert!(lines.pieces.iter().any(|piece| matches!(
        piece.source,
        ViewSource::Edge {
            kind: ViewEdgeKind::Tangent,
            ..
        }
    )));
}

#[test]
fn the_lines_of_a_turned_body_are_the_same_as_those_of_a_turned_view() {
    let solid = holed_block(10.0, 4.0, 2.5);
    let turn = RigidTransform::rotation_about(Point3::ZERO, Vector3::Z, 0.7).unwrap();
    let turned = solid.transformed(&turn).unwrap();
    let view = HiddenLineView::new(Vector3::new(0.3, 1.0, -0.6).normalize(), Vector3::Z).unwrap();
    let turned_view = HiddenLineView::new(
        turn.rotation() * Vector3::new(0.3, 1.0, -0.6).normalize(),
        Vector3::Z,
    )
    .unwrap();

    let straight = lines_of(&solid, &view);
    let matched = lines_of(&turned, &turned_view);

    for visibility in [Visibility::Visible, Visibility::Hidden] {
        assert!(
            (total(&straight.pieces, visibility) - total(&matched.pieces, visibility)).abs()
                <= 1e-4
        );
    }
    edge_totals(&solid, &straight, &view);
    edge_totals(&turned, &matched, &turned_view);
}

#[test]
fn a_view_without_a_direction_or_with_up_along_it_is_refused() {
    assert_eq!(
        HiddenLineView::new(Vector3::ZERO, Vector3::Z),
        Err(HiddenLineError::NoLookDirection)
    );
    assert_eq!(
        HiddenLineView::new(Vector3::Z, Vector3::Z * 2.0),
        Err(HiddenLineError::UpAlongLook)
    );
}

#[test]
fn it_can_be_cancelled_anywhere() {
    let solid = torus(6.0, 2.0);
    let view = HiddenLineView::new(
        Vector3::new(0.0, 1.0, -1.0).normalize(),
        Vector3::new(0.0, 1.0, 1.0),
    )
    .unwrap();
    let total = assert_cancelled_anywhere(
        "hidden lines",
        || hidden_lines(&[&solid], &view, &tolerance()),
        |error| matches!(error, HiddenLineError::Cancelled(_)),
    );
    assert!(total > 1);
}

fn extrude_box() -> Solid {
    let curves = crate::test_support::rectangle(1, (0.0, 0.0), (6.0, 6.0));
    let regions = crate::Profile::new(&curves)
        .unwrap()
        .select(&crate::Selection::EvenDepth)
        .unwrap();
    crate::extrude(
        &caditor_geometry::Plane::XY,
        &regions,
        crate::LinearExtent::one_side(3.0).unwrap(),
        76,
    )
    .unwrap()
}

fn row_of_blocks(count: usize) -> Vec<Solid> {
    (0..count)
        .map(|index| {
            let mut fixture = Fixture::new();
            let left = 2.0 * index as f64;
            add_cuboid(
                &mut fixture,
                Point3::new(left, 0.0, 0.0),
                Point3::new(left + 1.0, 1.0, 1.0),
                Tweak::default(),
            );
            fixture.build()
        })
        .collect()
}

#[test]
fn work_on_a_row_of_blocks_grows_about_linearly() {
    let work = |count: usize| {
        let blocks = row_of_blocks(count);
        let refs: Vec<&Solid> = blocks.iter().collect();
        let view = HiddenLineView::new(Vector3::Y, Vector3::Z).unwrap();
        let (lines, units) =
            tally::measure(|| hidden_lines_on(&refs, &view, &tolerance(), 1).unwrap());
        assert_eq!(lines.undecided, 0);
        units
    };

    let (few, many) = (work(4), work(32));

    assert!(
        many <= 16 * few,
        "eight times the blocks stay within n log n, far under the 64 times of every ray meeting \
         every block: {many} against {few}"
    );
    let cube = cuboid(Vector3::splat(1.0));
    let (_, polls) = polls_during(|| hidden_lines(&[&cube], &from_front(), &tolerance()).unwrap());
    assert!(polls <= 400, "a cuboid took {polls} polls");
}

fn ray_hits_mesh(mesh: &crate::Mesh, origin: Point3, toward: Vector3, beyond: f64) -> bool {
    mesh.position_triangles().any(|[a, b, c]| {
        let positions = mesh.positions();
        let (a, b, c) = (
            positions[a as usize],
            positions[b as usize],
            positions[c as usize],
        );
        let (ab, ac) = (b - a, c - a);
        let across = toward.cross(ac);
        let determinant = ab.dot(across);
        if determinant.abs() <= 1e-14 {
            return false;
        }
        let offset = origin - a;
        let first = offset.dot(across) / determinant;
        let side = offset.cross(ab);
        let second = toward.dot(side) / determinant;
        let along = ac.dot(side) / determinant;
        first >= 0.0 && second >= 0.0 && first + second <= 1.0 && along > beyond
    })
}

fn grazes_its_face(solid: &Solid, track: &Track, point: Point3, toward: Vector3) -> bool {
    matches!(track.own, Own::Edge(_))
        && track.faces.iter().any(|face| {
            let surface = solid.face(*face).unwrap().surface();
            let uv = surface.project(point, None);
            is_curved(surface) && contour::facing(surface, uv, toward).abs() < 0.05
        })
}

fn passes_close_to_a_face(
    mesh: &crate::Mesh,
    point: Point3,
    plane: &caditor_geometry::Plane,
) -> bool {
    [
        plane.x_axis(),
        -plane.x_axis(),
        plane.y_axis(),
        -plane.y_axis(),
    ]
    .into_iter()
    .any(|across| ray_hits_mesh(mesh, point + across * 0.01, plane.normal(), 0.3))
}

fn track_verdicts_agree_with_a_fine_mesh(name: &str, solid: &Solid, view: &HiddenLineView) {
    let plane = view.frame();
    let scene = Scene::of(&[solid]);
    let (tracks, _) = tracks_of_scene(&scene, plane, &tolerance(), 1).unwrap();
    let mesh = solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.02).unwrap())
        .unwrap();
    let mut checked = 0;
    for track in &tracks {
        let ends = piece_ends(track, plane);
        for pair in ends.windows(2) {
            let range = Interval::new(pair[0], pair[1]).unwrap();
            let start = plane.to_local(track.curve.point(range.start()));
            let end = plane.to_local(track.curve.point(range.end()));
            if start.distance(end) < 0.05 {
                continue;
            }
            let at = range.middle();
            let found = visibility::verdict(&scene, track, at, plane.normal()).unwrap();
            let point = track.curve.point(at);
            let lifted = point + plane.normal() * 0.005;
            let beyond_touch = ray_hits_mesh(&mesh, lifted, plane.normal(), 0.0);
            let far = ray_hits_mesh(&mesh, point, plane.normal(), 0.3);
            if far {
                assert_eq!(
                    found,
                    Verdict::Hidden,
                    "{name}: {:?} at {point:?}",
                    track.source
                );
                checked += 1;
            } else if !beyond_touch
                && !grazes_its_face(solid, track, point, plane.normal())
                && !passes_close_to_a_face(&mesh, point, plane)
            {
                assert_eq!(
                    found,
                    Verdict::Visible,
                    "{name}: {:?} at {point:?}",
                    track.source
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 3,
        "{name}: only {checked} stretches were decided by the mesh"
    );
}

#[test]
fn verdicts_agree_with_rays_cast_at_a_fine_mesh() {
    let oblique =
        HiddenLineView::new(Vector3::new(0.35, 0.8, -0.5).normalize(), Vector3::Z).unwrap();
    for (name, solid) in crate::fixtures::every_solid() {
        for view in [oblique, tilted_from_axis(70.0), from_front()] {
            track_verdicts_agree_with_a_fine_mesh(name, &solid, &view);
        }
    }
}

fn outline_of(solid: &Solid, view: &HiddenLineView) -> Vec<OutlinePiece> {
    outer_outline(solid, view, &tolerance()).unwrap()
}

fn outline_length(pieces: &[OutlinePiece]) -> f64 {
    pieces
        .iter()
        .map(|piece| piece.curve.length(piece.range))
        .sum()
}

#[test]
fn the_outline_of_a_cylinder_seen_side_on_is_a_rectangle_of_four_lines() {
    let outline = outline_of(&cylinder(3.0, 5.0), &from_front());

    assert_eq!(outline.len(), 4, "{outline:#?}");
    assert!(
        outline
            .iter()
            .all(|piece| matches!(piece.curve, Curve2::Line(_)))
    );
    assert_close(
        outline_length(&outline),
        2.0 * (6.0 + 5.0),
        "the rectangle's perimeter",
    );
    let silhouette_lines = outline
        .iter()
        .filter(|piece| matches!(piece.source, ViewSource::Silhouette { .. }))
        .count();
    assert_eq!(silhouette_lines, 2);
}

#[test]
fn the_outline_of_a_cylinder_seen_end_on_is_one_circle() {
    let outline = outline_of(&cylinder(3.0, 5.0), &from_above());

    assert_eq!(outline.len(), 1, "{outline:#?}");
    assert!(matches!(outline.first().unwrap().curve, Curve2::Circle(_)));
    assert_close(outline_length(&outline), TAU * 3.0, "the rim");
}

#[test]
fn the_outline_of_a_sphere_is_its_silhouette_circle_without_its_seam() {
    let outline = outline_of(&sphere(4.0), &from_front());

    assert_eq!(outline.len(), 1, "{outline:#?}");
    assert!(matches!(
        outline.first().unwrap().source,
        ViewSource::Silhouette { .. }
    ));
    assert_close(outline_length(&outline), TAU * 4.0, "the circle");
}

#[test]
fn the_outline_leaves_out_a_hole_seen_through() {
    let outline = outline_of(&holed_block(10.0, 4.0, 2.5), &from_above());

    assert_eq!(outline.len(), 4, "{outline:#?}");
    assert_close(outline_length(&outline), 40.0, "the block's square");

    let ring = outline_of(&torus(6.0, 2.0), &from_above());
    assert_eq!(ring.len(), 1, "{ring:#?}");
    assert_close(
        outline_length(&ring),
        TAU * 8.0,
        "the torus's rim, not its hole",
    );
}

#[test]
fn the_outline_of_a_tilted_cylinder_runs_round_both_rims_and_silhouettes() {
    let outline = outline_of(&cylinder(3.0, 5.0), &tilted_from_axis(45.0));

    assert_eq!(outline.len(), 4, "{outline:#?}");
    let lines = outline
        .iter()
        .filter(|piece| matches!(piece.curve, Curve2::Line(_)))
        .count();
    assert_eq!(lines, 2);
    assert_eq!(outline.len() - lines, 2, "half of each rim");
}

#[test]
fn outline_pieces_of_one_source_are_numbered_along_the_view() {
    let outline = outline_of(&cylinder(3.0, 5.0), &from_front());

    let mut lines: Vec<&OutlinePiece> = outline
        .iter()
        .filter(|piece| matches!(piece.source, ViewSource::Silhouette { .. }))
        .collect();
    lines.sort_by_key(|piece| piece.occurrence);
    assert_eq!(
        lines
            .iter()
            .map(|piece| (piece.occurrence, piece.of))
            .collect::<Vec<_>>(),
        vec![(0, 2), (1, 2)]
    );
    let middle_x = |piece: &OutlinePiece| piece.curve.point(piece.range.middle()).x;
    assert!(middle_x(lines[0]) < middle_x(lines[1]));
}

fn oblique_over_a_plate() -> HiddenLineView {
    HiddenLineView::new(Vector3::new(0.3, 0.5, -0.8), Vector3::Z).unwrap()
}

#[test]
fn lines_found_on_many_threads_are_those_of_one_thread() {
    let plate = perforated_plate(3, 0);
    let views = [oblique_over_a_plate(), from_above(), from_front()];

    for view in &views {
        let alone = hidden_lines_on(&[&plate], view, &tolerance(), 1).unwrap();
        let shared = hidden_lines_on(&[&plate], view, &tolerance(), 6).unwrap();
        assert_eq!(alone, shared);
        assert!(alone.pieces.len() > 3 * TRACKS_PER_THREAD);

        let outline = outer_outline_on(&plate, view, &tolerance(), 1).unwrap();
        let shared_outline = outer_outline_on(&plate, view, &tolerance(), 6).unwrap();
        assert_eq!(outline, shared_outline);
    }
}

#[test]
fn solids_piercing_one_another_are_found_on_many_threads_as_on_one() {
    let plate = perforated_plate(3, 0);
    let pin = cylinder(4.0, 30.0)
        .transformed(&RigidTransform::translation(Vector3::new(22.0, 22.0, -10.0)).unwrap())
        .unwrap();
    let view = oblique_over_a_plate();

    let alone = hidden_lines_on(&[&plate, &pin], &view, &tolerance(), 1).unwrap();
    let shared = hidden_lines_on(&[&plate, &pin], &view, &tolerance(), 6).unwrap();

    assert_eq!(alone, shared);
    assert!(alone.pieces.len() > 3 * TRACKS_PER_THREAD);
}

#[test]
fn it_can_be_cancelled_anywhere_on_many_threads() {
    let plate = perforated_plate(2, 0);
    let view = oblique_over_a_plate();

    let lines = assert_cancelled_anywhere(
        "hidden lines on many threads",
        || hidden_lines_on(&[&plate], &view, &tolerance(), 4),
        |error| matches!(error, HiddenLineError::Cancelled(_)),
    );
    let outline = assert_cancelled_anywhere(
        "an outline on many threads",
        || outer_outline_on(&plate, &view, &tolerance(), 4),
        |error| matches!(error, HiddenLineError::Cancelled(_)),
    );

    assert!(lines > 1 && outline > 1);
}

#[test]
#[ignore = "a timing benchmark: cargo test --release -p caditor-kernel hidden_line_costs -- --ignored --nocapture"]
fn hidden_line_costs() {
    let median = |work: &dyn Fn()| {
        let runs = 15;
        let mut times: Vec<std::time::Duration> = (0..runs)
            .map(|_| {
                let started = std::time::Instant::now();
                work();
                started.elapsed()
            })
            .collect();
        times.sort();
        times[runs / 2]
    };
    let view = oblique_over_a_plate();
    let threads = crate::fan_out::available_threads();

    for holes in [4, 8, 14] {
        let plate = perforated_plate(holes, 0);
        let faces = plate.faces().count();
        let lines = |threads: usize| {
            median(&|| {
                hidden_lines_on(&[&plate], &view, &tolerance(), threads).unwrap();
            })
        };
        let outline = |threads: usize| {
            median(&|| {
                outer_outline_on(&plate, &view, &tolerance(), threads).unwrap();
            })
        };
        let scene = Scene::of(&[&plate]);
        let setup = |threads: usize| {
            median(&|| {
                tracks_of_scene(&scene, view.frame(), &tolerance(), threads).unwrap();
            })
        };

        println!(
            "{faces} faces: tracks one thread {:?}, {threads} threads {:?}; hidden lines one thread {:?}, {threads} threads \
             {:?}; outline one thread {:?}, {threads} threads {:?}",
            setup(1),
            setup(threads),
            lines(1),
            lines(threads),
            outline(1),
            outline(threads),
        );
    }
}
