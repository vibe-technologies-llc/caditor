use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::PI,
};

use caditor_geometry::Point2;

use super::*;
use crate::test_support::{arc, assert_cancelled_anywhere, circle, line, rectangle, spline};

fn profile(curves: &[ProfileCurve]) -> Profile {
    Profile::new(curves).unwrap()
}

fn keys(profile: &Profile) -> BTreeSet<RegionKey> {
    profile.regions().iter().map(Region::key).collect()
}

fn identities(region: &Region) -> BTreeSet<(PieceId, Side)> {
    region
        .pieces()
        .map(|piece| (piece.id().clone(), piece.side()))
        .collect()
}

fn signature(profile: &Profile) -> BTreeMap<RegionKey, BTreeSet<(PieceId, Side)>> {
    profile
        .regions()
        .iter()
        .map(|region| (region.key(), identities(region)))
        .collect()
}

fn areas(profile: &Profile) -> Vec<f64> {
    let mut areas: Vec<f64> = profile.regions().iter().map(Region::area).collect();
    areas.sort_by(f64::total_cmp);
    areas
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-9 * (1.0 + expected.abs())
}

fn assert_areas(profile: &Profile, expected: &[f64]) {
    let actual = areas(profile);
    assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
    let mut expected = expected.to_vec();
    expected.sort_by(f64::total_cmp);
    for (actual, expected) in actual.iter().zip(&expected) {
        assert!(close(*actual, *expected), "{actual} vs {expected}");
    }
}

fn assert_closed(region: &Region) {
    for profile_loop in region.loops() {
        let pieces = profile_loop.pieces();
        for (index, piece) in pieces.iter().enumerate() {
            let next = &pieces[(index + 1) % pieces.len()];
            assert!(
                piece.end().distance(next.start()) < 1e-9,
                "{:?} ends at {} but {:?} starts at {}",
                piece.id(),
                piece.end(),
                next.id(),
                next.start()
            );
        }
    }
    assert!(region.outer().signed_area() > 0.0);
    for hole in region.holes() {
        assert!(hole.signed_area() < 0.0);
    }
}

fn lens(radius: f64, distance: f64) -> f64 {
    let half = distance / 2.0;
    2.0 * radius * radius * (half / radius).acos()
        - 2.0 * half * (radius * radius - half * half).sqrt()
}

#[test]
fn a_rectangle_is_one_region() {
    let profile = profile(&rectangle(1, (0.0, 0.0), (4.0, 3.0)));
    assert_areas(&profile, &[12.0]);
    let region = &profile.regions()[0];
    assert_closed(region);
    assert_eq!(region.depth(), 0);
    assert_eq!(region.outer().pieces().len(), 4);
    assert!(region.holes().is_empty());
    for piece in region.pieces() {
        assert_eq!(piece.side(), Side::Left);
        assert_eq!(piece.id().start(), &PieceBound::Start);
        assert_eq!(piece.id().end(), &PieceBound::End);
    }
    let reversed: Vec<ProfileCurve> = rectangle(1, (0.0, 0.0), (4.0, 3.0))
        .into_iter()
        .map(|curve| match curve.shape {
            ProfileShape::Line { start, end } => ProfileCurve::line(curve.entity, end, start),
            _ => curve,
        })
        .collect();
    let clockwise = Profile::new(&reversed).unwrap();
    assert_areas(&clockwise, &[12.0]);
    assert!(
        clockwise.regions()[0]
            .pieces()
            .all(|piece| piece.side() == Side::Right)
    );
}

#[test]
fn a_circle_inside_a_rectangle_is_a_hole() {
    let mut curves = rectangle(1, (0.0, 0.0), (10.0, 8.0));
    curves.push(circle(5, (4.0, 4.0), 2.0));
    let profile = profile(&curves);
    let hole = 4.0 * PI;
    assert_areas(&profile, &[80.0 - hole, hole]);
    let plate = profile
        .regions()
        .iter()
        .find(|region| region.depth() == 0)
        .unwrap();
    assert_eq!(plate.holes().len(), 1);
    assert_closed(plate);
    let disc = profile
        .regions()
        .iter()
        .find(|region| region.depth() == 1)
        .unwrap();
    assert_eq!(disc.pieces().count(), 1);
    assert_eq!(profile.even_depth(), vec![plate.key()]);
    let chosen = profile.select(&Selection::EvenDepth).unwrap();
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].key(), plate.key());
    assert!(close(chosen[0].area(), 80.0 - hole));
}

#[test]
fn an_island_inside_a_hole_is_chosen_again() {
    let mut curves = rectangle(1, (0.0, 0.0), (10.0, 10.0));
    curves.push(circle(5, (5.0, 5.0), 3.0));
    curves.push(circle(6, (5.0, 5.0), 1.0));
    let profile = profile(&curves);
    let depths: BTreeSet<usize> = profile.regions().iter().map(Region::depth).collect();
    assert_eq!(depths, BTreeSet::from([0, 1, 2]));
    let chosen = profile.select(&Selection::EvenDepth).unwrap();
    assert_eq!(chosen.len(), 2);
    let total: f64 = chosen.iter().map(Region::area).sum();
    assert!(close(total, 100.0 - 9.0 * PI + PI));
}

#[test]
fn overlapping_circles_give_three_regions() {
    let profile = profile(&[circle(1, (0.0, 0.0), 2.0), circle(2, (3.0, 0.0), 2.0)]);
    let overlap = lens(2.0, 3.0);
    let single = 4.0 * PI;
    assert_areas(&profile, &[single - overlap, single - overlap, overlap]);
    assert_eq!(keys(&profile).len(), 3);
    for region in profile.regions() {
        assert_closed(region);
        assert_eq!(region.depth(), 0);
    }
    let union = profile.select(&Selection::EvenDepth).unwrap();
    assert_eq!(union.len(), 1);
    assert!(close(union[0].area(), 2.0 * single - overlap));
    assert_eq!(union[0].pieces().count(), 2);
}

#[test]
fn a_line_across_a_rectangle_splits_it_in_two() {
    let mut curves = rectangle(1, (0.0, 0.0), (10.0, 4.0));
    curves.push(line(5, (3.0, -2.0), (5.0, 6.0)));
    let profile = profile(&curves);
    let left = 0.5 * (3.5 + 4.5) * 4.0;
    assert_areas(&profile, &[left, 40.0 - left]);
    for region in profile.regions() {
        assert_closed(region);
    }
    let union = profile.select(&Selection::EvenDepth).unwrap();
    assert_eq!(union.len(), 1);
    assert!(close(union[0].area(), 40.0));
    let single = profile
        .select(&Selection::Regions(vec![profile.regions()[0].key()]))
        .unwrap();
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].key(), profile.regions()[0].key());
}

#[test]
fn an_arc_and_a_line_make_a_d() {
    let profile = profile(&[
        arc(1, (0.0, 0.0), (3.0, 0.0), (-3.0, 0.0)),
        line(2, (-3.0, 0.0), (3.0, 0.0)),
    ]);
    assert_areas(&profile, &[4.5 * PI]);
    assert_closed(&profile.regions()[0]);
}

#[test]
fn a_spline_closed_by_a_line_encloses_its_parabolic_segment() {
    let profile = profile(&[
        spline(1, &[(0.0, 0.0), (1.0, 2.0), (2.0, 0.0)]),
        line(2, (2.0, 0.0), (0.0, 0.0)),
    ]);
    assert_areas(&profile, &[4.0 / 3.0]);
    let cubic = Profile::new(&[
        spline(
            1,
            &[(0.0, 0.0), (1.0, 3.0), (3.0, 3.0), (5.0, 1.0), (6.0, 0.0)],
        ),
        line(2, (6.0, 0.0), (0.0, 0.0)),
    ])
    .unwrap();
    assert_eq!(cubic.regions().len(), 1);
    assert!(cubic.regions()[0].area() > 0.0);
}

#[test]
fn a_spline_crossing_a_line_three_times_makes_distinct_keys() {
    let profile = profile(&[
        spline(
            1,
            &[
                (0.0, 0.0),
                (2.0, 3.0),
                (4.0, -3.0),
                (6.0, 3.0),
                (8.0, -3.0),
                (10.0, 0.0),
            ],
        ),
        line(2, (0.0, 0.0), (10.0, 0.0)),
    ]);
    assert!(profile.regions().len() >= 3, "{:?}", areas(&profile));
    assert_eq!(keys(&profile).len(), profile.regions().len());
    for region in profile.regions() {
        assert_closed(region);
    }
}

#[test]
fn a_self_crossing_spline_bounds_two_lobes() {
    let profile = profile(&[spline(
        1,
        &[
            (0.0, 0.0),
            (4.0, 4.0),
            (4.0, -4.0),
            (-4.0, 4.0),
            (-4.0, -4.0),
            (0.0, 0.0),
        ],
    )]);
    assert_eq!(profile.regions().len(), 2, "{:?}", areas(&profile));
    for region in profile.regions() {
        assert_closed(region);
    }
}

#[test]
fn dangling_curves_bound_nothing() {
    let plain = profile(&rectangle(1, (0.0, 0.0), (4.0, 3.0)));
    let mut curves = rectangle(1, (0.0, 0.0), (4.0, 3.0));
    curves.push(line(5, (4.0, 3.0), (6.0, 5.0)));
    curves.push(line(6, (20.0, 20.0), (25.0, 21.0)));
    curves.push(line(7, (2.0, 0.0), (2.0, -3.0)));
    curves.push(line(8, (1.0, 1.0), (2.0, 2.0)));
    let noisy = profile(&curves);
    assert_areas(&noisy, &[12.0]);
    assert_eq!(keys(&plain), keys(&noisy));
    let tree = profile(&[
        line(1, (0.0, 0.0), (1.0, 0.0)),
        line(2, (1.0, 0.0), (1.0, 1.0)),
    ]);
    assert!(tree.regions().is_empty());
    assert_eq!(
        tree.select(&Selection::EvenDepth),
        Err(ProfileError::NoClosedProfile)
    );
}

#[test]
fn nearly_coincident_endpoints_join() {
    let gap = 1e-9;
    let curves = vec![
        line(1, (0.0, 0.0), (4.0, gap)),
        line(2, (4.0 + gap, 0.0), (4.0, 3.0 - gap)),
        line(3, (4.0, 3.0), (-gap, 3.0)),
        line(4, (0.0, 3.0 + gap), (gap, 0.0)),
    ];
    let profile = profile(&curves);
    assert_eq!(profile.regions().len(), 1);
    assert!((profile.regions()[0].area() - 12.0).abs() < 1e-7);
    assert_closed(&profile.regions()[0]);
}

#[test]
fn tangent_circles_touch_at_one_point() {
    let outside = profile(&[circle(1, (0.0, 0.0), 2.0), circle(2, (5.0, 0.0), 3.0)]);
    assert_areas(&outside, &[4.0 * PI, 9.0 * PI]);
    let union = outside.select(&Selection::EvenDepth).unwrap();
    assert_eq!(union.len(), 2);
    let inside = profile(&[circle(1, (0.0, 0.0), 3.0), circle(2, (1.0, 0.0), 2.0)]);
    assert_areas(&inside, &[4.0 * PI, 5.0 * PI]);
    for region in inside.regions() {
        assert_closed(region);
    }
    let slightly = 1e-11;
    let solved = profile(&[
        circle(1, (0.0, 0.0), 2.0),
        circle(2, (5.0 + slightly, 0.0), 3.0),
    ]);
    assert_eq!(solved.regions().len(), 2);
    let with_line = profile(&[
        circle(1, (0.0, 0.0), 2.0),
        line(2, (-3.0, 2.0 + slightly), (3.0, 2.0)),
        line(3, (3.0, 2.0), (3.0, -3.0)),
        line(4, (3.0, -3.0), (-3.0, -3.0)),
        line(5, (-3.0, -3.0), (-3.0, 2.0 + slightly)),
    ]);
    assert_areas(&with_line, &[4.0 * PI, 30.0 - 4.0 * PI]);
}

#[test]
fn collinear_overlapping_lines_merge() {
    let curves = vec![
        line(1, (0.0, 0.0), (6.0, 0.0)),
        line(2, (4.0, 0.0), (10.0, 0.0)),
        line(3, (10.0, 0.0), (10.0, 5.0)),
        line(4, (10.0, 5.0), (0.0, 5.0)),
        line(5, (0.0, 5.0), (0.0, 0.0)),
        line(6, (0.0, 5.0), (10.0, 5.0)),
    ];
    let profile = profile(&curves);
    assert_areas(&profile, &[50.0]);
    let region = &profile.regions()[0];
    assert_closed(region);
    let entities: BTreeSet<u64> = region.pieces().map(Piece::entity).collect();
    assert_eq!(entities, BTreeSet::from([1, 2, 3, 4, 5]));
    let arcs = Profile::new(&[
        arc(1, (0.0, 0.0), (2.0, 0.0), (-2.0, 0.0)),
        arc(2, (0.0, 0.0), (0.0, 2.0), (2.0, 0.0)),
        line(3, (-2.0, 0.0), (2.0, 0.0)),
    ])
    .unwrap();
    assert_eq!(arcs.regions().len(), 2);
    assert!(close(
        arcs.regions().iter().map(Region::area).sum(),
        2.0 * PI + 2.0 * PI
    ));
}

fn keyed_fixture(hole: (f64, f64), scale: f64) -> Vec<ProfileCurve> {
    let mut curves = rectangle(10, (0.0, 0.0), (10.0 * scale, 6.0 * scale));
    curves.push(circle(20, hole, 1.5));
    curves.push(circle(21, (hole.0 + 2.0, hole.1), 1.0));
    curves.push(line(30, (7.0 * scale, -1.0), (8.0 * scale, 7.0 * scale)));
    curves
}

#[test]
fn keys_ignore_the_order_of_curves() {
    let curves = keyed_fixture((3.0, 3.0), 1.0);
    let expected = signature(&profile(&curves));
    assert_eq!(expected.len(), 5);
    let mut shuffled = curves.clone();
    shuffled.reverse();
    shuffled.swap(1, 4);
    assert_eq!(signature(&profile(&shuffled)), expected);
}

#[test]
fn keys_survive_small_moves_and_scaling() {
    let expected = signature(&profile(&keyed_fixture((3.0, 3.0), 1.0)));
    let moved = signature(&profile(&keyed_fixture((3.2, 2.9), 1.0)));
    assert_eq!(moved, expected);
    let scaled = signature(&profile(&keyed_fixture((3.0, 3.0), 1.05)));
    assert_eq!(scaled, expected);
}

#[test]
fn keys_ignore_unrelated_curves_and_their_ids() {
    let curves = keyed_fixture((3.0, 3.0), 1.0);
    let expected = signature(&profile(&curves));
    let mut added = curves.clone();
    added.push(circle(40, (30.0, 30.0), 2.0));
    added.push(line(41, (-5.0, -5.0), (-4.0, -7.0)));
    let with_extra = signature(&profile(&added));
    for (key, pieces) in &expected {
        assert_eq!(with_extra.get(key), Some(pieces));
    }
    assert_eq!(with_extra.len(), expected.len() + 1);
    let mut renamed = curves.clone();
    renamed.push(circle(77, (30.0, 30.0), 2.0));
    renamed.push(line(78, (-5.0, -5.0), (-4.0, -7.0)));
    let with_renamed = signature(&profile(&renamed));
    for (key, pieces) in &expected {
        assert_eq!(with_renamed.get(key), Some(pieces));
    }
}

#[test]
fn keys_ignore_ids_of_curves_that_do_not_bound_the_region() {
    let mut curves = rectangle(1, (0.0, 0.0), (10.0, 10.0));
    curves.push(circle(5, (3.0, 3.0), 1.0));
    curves.push(circle(6, (7.0, 7.0), 1.0));
    let before = profile(&curves);
    let disc = before
        .regions()
        .iter()
        .find(|region| region.pieces().all(|piece| piece.entity() == 5))
        .unwrap()
        .key();
    curves[5].entity = 60;
    let after = profile(&curves);
    assert!(after.region(disc).is_some());
}

#[test]
fn selection_errors_name_what_is_wrong() {
    let profile = profile(&rectangle(1, (0.0, 0.0), (4.0, 3.0)));
    let missing = RegionKey::from_digest(42);
    assert_eq!(
        profile.select(&Selection::Regions(vec![missing])),
        Err(ProfileError::MissingRegion(missing))
    );
    assert_eq!(
        profile.select(&Selection::Regions(Vec::new())),
        Err(ProfileError::EmptySelection)
    );
    assert_eq!(
        Profile::new(&[line(3, (1.0, 1.0), (1.0, 1.0))]).unwrap_err(),
        ProfileError::Degenerate { entity: 3 }
    );
    assert_eq!(
        Profile::new(&[
            line(3, (0.0, 0.0), (1.0, 1.0)),
            line(3, (0.0, 1.0), (1.0, 0.0))
        ])
        .unwrap_err(),
        ProfileError::DuplicateEntity { entity: 3 }
    );
    assert!(matches!(
        Profile::new(&[circle(4, (0.0, 0.0), f64::NAN)]),
        Err(ProfileError::InvalidCurve { entity: 4, .. })
    ));
    assert_eq!(
        Profile::new(&[arc(9, (0.0, 0.0), (1.0, 0.0), (1.0, 0.0))]).unwrap_err(),
        ProfileError::Degenerate { entity: 9 }
    );
    assert_eq!(
        ProfileError::Overlap {
            first: 1,
            second: 2
        }
        .entities(),
        vec![1, 2]
    );
}

#[test]
fn overlapping_splines_are_refused() {
    let straight = ProfileCurve::spline(
        1,
        1,
        vec![0.0, 0.0, 0.5, 1.0, 1.0],
        vec![Point2::ZERO, Point2::new(5.0, 0.0), Point2::new(10.0, 0.0)],
    );
    assert_eq!(
        Profile::new(&[straight, line(2, (2.0, 0.0), (8.0, 0.0))]).unwrap_err(),
        ProfileError::Overlap {
            first: 1,
            second: 2
        }
    );
    let doubled = ProfileCurve::spline(
        3,
        1,
        vec![0.0, 0.0, 0.5, 1.0, 1.0],
        vec![Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(5.0, 0.0)],
    );
    assert_eq!(
        Profile::new(&[doubled]).unwrap_err(),
        ProfileError::SelfOverlap { entity: 3 }
    );
}

#[test]
fn pieces_cut_by_other_curves_name_their_cutters() {
    let mut curves = rectangle(1, (0.0, 0.0), (10.0, 4.0));
    curves.push(line(5, (5.0, -2.0), (5.0, 6.0)));
    let profile = profile(&curves);
    let bottom: BTreeSet<PieceId> = profile
        .regions()
        .iter()
        .flat_map(Region::pieces)
        .filter(|piece| piece.entity() == 1)
        .map(|piece| piece.id().clone())
        .collect();
    let cut = PieceBound::Cut {
        entities: vec![5],
        occurrence: 0,
    };
    assert_eq!(
        bottom,
        BTreeSet::from([
            PieceId::new(1, PieceBound::Start, cut.clone()),
            PieceId::new(1, cut, PieceBound::End),
        ])
    );
}

#[test]
fn a_spline_tangent_to_a_line_touches_it_once() {
    let profile = profile(&[
        spline(1, &[(-1.0, 1.0), (0.0, -1.0), (1.0, 1.0)]),
        line(2, (-2.0, 0.0), (2.0, 0.0)),
        line(3, (2.0, 0.0), (1.0, 1.0)),
        line(4, (-1.0, 1.0), (-2.0, 0.0)),
    ]);
    assert_areas(&profile, &[5.0 / 6.0, 5.0 / 6.0]);
    for region in profile.regions() {
        assert_closed(region);
    }
}

#[test]
fn a_region_with_a_hole_triangulates_to_its_own_area() {
    let mut curves = rectangle(1, (0.0, 0.0), (20.0, 10.0));
    curves.push(circle(9, (10.0, 5.0), 3.0));
    let profile = profile(&curves);
    let ring = profile
        .regions()
        .iter()
        .find(|region| region.depth() == 0)
        .unwrap();
    let tolerance = SamplingTolerance::new(1e-3, 0.05).unwrap();

    let mesh = ring.triangulate(&tolerance).unwrap();

    let area: f64 = mesh
        .triangles
        .iter()
        .map(|[a, b, c]| {
            let [a, b, c] = [a, b, c].map(|index| mesh.points[*index as usize]);
            (b - a).perp_dot(c - a) / 2.0
        })
        .map(f64::abs)
        .sum();
    assert!(
        (area - ring.area()).abs() < 0.02,
        "{area} vs {}",
        ring.area()
    );
    assert!((ring.area() - (200.0 - 9.0 * PI)).abs() < 1e-6);
    assert_eq!(ring.polygons(&tolerance).len(), 2);
}

#[test]
fn a_long_wiggly_spline_closed_by_a_line_is_divided() {
    let wiggles: Vec<(f64, f64)> = (0..=600)
        .map(|index| {
            let x = index as f64;
            let y = if index == 0 || index == 600 {
                0.0
            } else {
                5.0 + (x / 3.0).sin()
            };
            (x, y)
        })
        .collect();
    let profile = profile(&[spline(1, &wiggles), line(2, (600.0, 0.0), (0.0, 0.0))]);
    assert_eq!(profile.regions().len(), 1, "{:?}", areas(&profile));
    assert_closed(&profile.regions()[0]);
}

#[test]
fn crossings_closer_than_a_thousandth_of_the_sketch_stay_apart() {
    let profile = profile(&[
        spline(
            1,
            &[
                (-500.0, 1.0),
                (-100.0, 1.0),
                (-1.0, 1.0),
                (-0.3, 1.0),
                (0.0, -2.0),
                (0.3, 1.0),
                (1.0, 1.0),
                (100.0, 1.0),
                (500.0, 1.0),
            ],
        ),
        line(2, (-500.0, 1.0), (-500.0, -400.0)),
        line(3, (-500.0, -400.0), (500.0, -400.0)),
        line(4, (500.0, -400.0), (500.0, 1.0)),
        line(5, (-500.0, 0.0), (500.0, 0.0)),
    ]);
    let small: Vec<f64> = areas(&profile)
        .into_iter()
        .filter(|area| *area < 1.0)
        .collect();
    assert_eq!(small.len(), 1, "{:?}", areas(&profile));
}

#[test]
fn a_region_keeps_its_key_whatever_else_is_chosen() {
    let profile = profile(&[
        spline(
            1,
            &[
                (0.0, 0.0),
                (2.0, 3.0),
                (4.0, -3.0),
                (6.0, 3.0),
                (8.0, -3.0),
                (10.0, 0.0),
            ],
        ),
        line(2, (0.0, 0.0), (10.0, 0.0)),
    ]);
    let all: Vec<RegionKey> = profile.regions().iter().map(Region::key).collect();
    for key in &all {
        let alone = profile.select(&Selection::Regions(vec![*key])).unwrap();
        assert_eq!(alone.len(), 1);
        assert_eq!(alone[0].key(), *key);
    }
    let separated: Vec<RegionKey> = all.iter().step_by(2).copied().collect();
    let together = profile
        .select(&Selection::Regions(separated.clone()))
        .unwrap();
    let mut found: Vec<RegionKey> = together.iter().map(Region::key).collect();
    found.sort();
    let mut expected = separated;
    expected.sort();
    assert_eq!(found, expected);
}

#[test]
fn many_squares_and_a_long_dangling_chain() {
    let mut curves = Vec::new();
    let mut entity = 0;
    let mut next = || {
        entity += 1;
        entity
    };
    for row in 0..10 {
        for column in 0..20 {
            let (x, y) = (f64::from(column) * 3.0, f64::from(row) * 3.0);
            let corners = [(x, y), (x + 2.0, y), (x + 2.0, y + 2.0), (x, y + 2.0)];
            for index in 0..4 {
                curves.push(line(next(), corners[index], corners[(index + 1) % 4]));
            }
        }
    }
    let zigzag = |step: i32| (f64::from(step) * 0.5, -5.0 - f64::from(step % 2) * 0.1);
    for step in 0..300 {
        curves.push(line(next(), zigzag(step), zigzag(step + 1)));
    }
    let profile = Profile::new(&curves).unwrap();
    let regions = profile.select(&Selection::EvenDepth).unwrap();
    assert_eq!(regions.len(), 200);
    assert!(regions.iter().all(|region| region.holes().is_empty()));
}

#[test]
fn a_plate_with_thousands_of_holes_is_divided_and_selected() {
    let mut curves = vec![
        line(1, (-1.0, -1.0), (120.0, -1.0)),
        line(2, (120.0, -1.0), (120.0, 120.0)),
        line(3, (120.0, 120.0), (-1.0, 120.0)),
        line(4, (-1.0, 120.0), (-1.0, -1.0)),
    ];
    let mut entity = 4;
    for row in 0..40 {
        for column in 0..40 {
            let (x, y) = (f64::from(column) * 3.0, f64::from(row) * 3.0);
            let corners = [(x, y), (x + 2.0, y), (x + 2.0, y + 2.0), (x, y + 2.0)];
            for index in 0..4 {
                entity += 1;
                curves.push(line(entity, corners[index], corners[(index + 1) % 4]));
            }
        }
    }

    let profile = profile(&curves);
    let plate = profile.select(&Selection::EvenDepth).unwrap();

    assert_eq!(profile.regions().len(), 1601);
    assert_eq!(keys(&profile).len(), 1601);
    assert_eq!(plate.len(), 1);
    assert_eq!(plate[0].holes().len(), 1600);
}

#[test]
fn unresolved_curves_are_narrowed_to_those_failing_together() {
    let curves: Vec<ProfileCurve> = (1..=20)
        .map(|entity| {
            let x = entity as f64;
            line(entity, (x, 0.0), (x, 1.0))
        })
        .collect();
    let together = |subset: &[ProfileCurve]| {
        [7, 13]
            .iter()
            .all(|entity| subset.iter().any(|curve| curve.entity == *entity))
    };
    let never = |_: &[ProfileCurve]| false;

    assert_eq!(culprits::culprits(&curves, together), vec![7, 13]);
    assert!(culprits::culprits(&curves, never).is_empty());
    assert_eq!(culprits::culprits(&curves[..3], never), vec![1, 2, 3]);
    assert_eq!(
        ProfileError::Unresolved {
            entities: vec![7, 13]
        }
        .entities(),
        vec![7, 13]
    );
}

#[test]
fn digests_of_piece_ids_and_tiebroken_keys_never_change() {
    let cut = PieceId::new(
        4,
        PieceBound::Cut {
            entities: vec![5, 9],
            occurrence: 1,
        },
        PieceBound::End,
    );
    let wiggle = profile(&[
        spline(
            1,
            &[
                (0.0, 0.0),
                (2.0, 3.0),
                (4.0, -3.0),
                (6.0, 3.0),
                (8.0, -3.0),
                (10.0, 0.0),
            ],
        ),
        line(2, (0.0, 0.0), (10.0, 0.0)),
    ]);

    let shared: Vec<u128> = wiggle.ambiguous.iter().map(|key| key.digest()).collect();
    let tiebroken: BTreeSet<u128> = wiggle
        .regions()
        .iter()
        .map(|region| region.key().digest())
        .collect();

    assert_eq!(cut.digest(), 0xbead_433d_ef24_0826_b5b7_084a_6ec7_522f);
    assert_eq!(
        shared,
        vec![
            0x2f87_f177_b11f_76c1_1600_ede9_4f64_339f,
            0xe544_dce8_c817_9bd4_8241_5c26_494e_a7cd,
        ]
    );
    assert_eq!(
        tiebroken,
        BTreeSet::from([
            0xa207_268b_93a8_942f_2e11_0fb4_ecc8_75fe,
            0xa661_8434_7219_b786_38f4_844a_d742_d115,
            0xb6a4_4692_e8cd_1ef9_4eb2_99ef_bc18_096a,
            0xff95_3a0f_ce4e_e7c9_5c24_9c4f_1997_ce0c,
        ])
    );
}

#[test]
fn a_profile_cancelled_anywhere_stops_with_cancelled() {
    let curves = [
        spline(
            1,
            &[
                (0.0, 0.0),
                (2.0, 3.0),
                (4.0, -3.0),
                (6.0, 3.0),
                (8.0, -3.0),
                (10.0, 0.0),
            ],
        ),
        spline(
            2,
            &[
                (0.0, 1.0),
                (3.0, -2.0),
                (5.0, 4.0),
                (7.0, -2.0),
                (10.0, 1.0),
            ],
        ),
        line(3, (0.0, 0.0), (10.0, 0.0)),
        circle(4, (5.0, 0.0), 2.5),
    ];

    let polls = assert_cancelled_anywhere(
        "profile",
        || Profile::new(&curves),
        |error| matches!(error, ProfileError::Cancelled(_)),
    );

    assert!(polls > 10, "only {polls} polls");
}
