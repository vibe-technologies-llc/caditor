use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector2, Vector3};

use super::*;
use crate::{
    boolean::{BooleanOperation, boolean},
    build::{AngularExtent, Axis2, LinearExtent, extrude, revolve},
    fixtures::{
        bumped_block, bumped_sheet, cone, cuboid, cylinder, extruded_spline, holed_block, sphere,
        spline_profile, torus,
    },
    interval::Interval,
    numeric::integrate,
    profile::{Profile, Region, Selection},
    surface::Surface,
    tessellation::{MassProperties, SecondMoment},
    test_support::{line, spline},
    tolerance::SamplingTolerance,
    topology::Solid,
};

const RELATIVE: f64 = 1e-9;
const FEATURE: u64 = 3;

struct Expected {
    volume: f64,
    area: f64,
    centroid: Point3,
    about_origin: SecondMoment,
}

fn diagonal(xx: f64, yy: f64, zz: f64) -> SecondMoment {
    [[xx, 0.0, 0.0], [0.0, yy, 0.0], [0.0, 0.0, zz]]
}

fn shifted(about_centroid: SecondMoment, volume: f64, centroid: Point3) -> SecondMoment {
    MassProperties {
        volume,
        area: 0.0,
        centroid,
        second_moment: about_centroid,
    }
    .second_moment_about(Point3::ZERO)
}

fn measured(name: &str, solid: &Solid) -> MassProperties {
    let coarse = solid.tessellate(&solid.default_tolerance()).unwrap();
    let fine = solid
        .tessellate(&SamplingTolerance::new(1e-2, 0.1).unwrap())
        .unwrap();
    let found = mass_properties(solid, &coarse).unwrap();
    let again = mass_properties(solid, &fine).unwrap();
    assert!(
        found.meshed_faces.is_empty(),
        "{name}: {:?}",
        found.meshed_faces
    );
    assert_eq!(found, again, "{name}: the mesh changed the result");
    found.properties
}

fn close(name: &str, what: &str, found: f64, expected: f64, scale: f64) {
    assert!(
        (found - expected).abs() <= RELATIVE * scale,
        "{name}: {what} {found} instead of {expected}"
    );
}

fn check(name: &str, solid: &Solid, expected: &Expected) {
    let found = measured(name, solid);
    close(
        name,
        "volume",
        found.volume,
        expected.volume,
        expected.volume,
    );
    close(name, "area", found.area, expected.area, expected.area);
    let reach = expected.centroid.length().max(expected.volume.cbrt());
    for (found, wanted) in found
        .centroid
        .to_array()
        .into_iter()
        .zip(expected.centroid.to_array())
    {
        close(name, "centroid", found, wanted, reach);
    }
    let about = found.second_moment_about(Point3::ZERO);
    let largest = expected
        .about_origin
        .iter()
        .flatten()
        .fold(0.0_f64, |most, entry| most.max(entry.abs()));
    for (found_row, wanted_row) in about.iter().zip(expected.about_origin) {
        for (found, wanted) in found_row.iter().zip(wanted_row) {
            close(name, "second moment", *found, wanted, largest);
        }
    }
}

#[test]
fn a_cylinder_matches_its_closed_form() {
    let (radius, height) = (3.0, 5.0);
    let volume = PI * radius * radius * height;
    let centroid = Point3::new(0.0, 0.0, 0.5 * height);
    let across = volume * radius * radius / 4.0;

    check(
        "cylinder",
        &cylinder(radius, height),
        &Expected {
            volume,
            area: TAU * radius * height + TAU * radius * radius,
            centroid,
            about_origin: shifted(
                diagonal(across, across, volume * height * height / 12.0),
                volume,
                centroid,
            ),
        },
    );
}

#[test]
fn a_cone_matches_its_closed_form() {
    let (radius, height) = (3.0, 4.0);
    let volume = PI * radius * radius * height / 3.0;
    let centroid = Point3::new(0.0, 0.0, 0.25 * height);
    let across = 3.0 / 20.0 * volume * radius * radius;

    check(
        "cone",
        &cone(radius, height),
        &Expected {
            volume,
            area: PI * radius * radius + PI * radius * radius.hypot(height),
            centroid,
            about_origin: shifted(
                diagonal(across, across, 3.0 / 80.0 * volume * height * height),
                volume,
                centroid,
            ),
        },
    );
}

#[test]
fn a_sphere_matches_its_closed_form() {
    let radius: f64 = 4.0;
    let volume = 4.0 / 3.0 * PI * radius.powi(3);
    let along = volume * radius * radius / 5.0;

    check(
        "sphere",
        &sphere(radius),
        &Expected {
            volume,
            area: 4.0 * PI * radius * radius,
            centroid: Point3::ZERO,
            about_origin: diagonal(along, along, along),
        },
    );
}

#[test]
fn a_torus_matches_its_closed_form() {
    let (major, minor) = (6.0, 2.0);
    let volume = 2.0 * PI * PI * major * minor * minor;
    let across = 0.5 * volume * (major * major + 0.75 * minor * minor);

    check(
        "torus",
        &torus(major, minor),
        &Expected {
            volume,
            area: 4.0 * PI * PI * major * minor,
            centroid: Point3::ZERO,
            about_origin: diagonal(across, across, volume * minor * minor / 4.0),
        },
    );
}

#[test]
fn a_block_with_a_hole_matches_its_closed_form() {
    let (side, height, radius) = (10.0, 4.0, 2.5);
    let block = side * side * height;
    let bore = PI * radius * radius * height;
    let volume = block - bore;
    let centroid = Point3::new(0.5 * side, 0.5 * side, 0.5 * height);
    let across = block * side * side / 12.0 - bore * radius * radius / 4.0;
    let along = (block - bore) * height * height / 12.0;

    check(
        "holed block",
        &holed_block(side, height, radius),
        &Expected {
            volume,
            area: 2.0 * (side * side - PI * radius * radius)
                + 4.0 * side * height
                + TAU * radius * height,
            centroid,
            about_origin: shifted(diagonal(across, across, along), volume, centroid),
        },
    );
}

#[derive(Default)]
struct Section {
    area: f64,
    x: f64,
    y: f64,
    xx: f64,
    xy: f64,
    yy: f64,
}

fn spline_section() -> Section {
    let curve = spline_profile();
    let breaks: Vec<f64> = (0..=32).map(|step| step as f64 / 32.0).collect();
    let along = |integrand: &dyn Fn(f64, f64) -> f64| {
        integrate(&breaks, |parameter| {
            let derivatives = curve.evaluate(parameter);
            let (x, y) = (derivatives.point.x, derivatives.point.y);
            integrand(x, y) * derivatives.first.y
        })
    };
    let sign = along(&|x, _| x).signum();
    Section {
        area: sign * along(&|x, _| x),
        x: sign * along(&|x, _| 0.5 * x * x),
        y: sign * along(&|x, y| x * y),
        xx: sign * along(&|x, _| x * x * x / 3.0),
        xy: sign * along(&|x, y| 0.5 * x * x * y),
        yy: sign * along(&|x, y| x * y * y),
    }
}

fn spline_length() -> f64 {
    let curve = spline_profile();
    let breaks: Vec<f64> = (0..=256).map(|step| step as f64 / 256.0).collect();
    integrate(&breaks, |parameter| {
        curve.evaluate(parameter).first.length()
    })
}

#[test]
fn an_extruded_spline_matches_its_section() {
    let height = 5.0;
    let section = spline_section();
    let volume = section.area * height;
    let half = 0.5 * height * height;
    let third = height * height * height / 3.0;

    check(
        "extruded spline",
        &extruded_spline(height),
        &Expected {
            volume,
            area: 2.0 * section.area + (spline_length() + 20.0) * height,
            centroid: Point3::new(
                section.x / section.area,
                section.y / section.area,
                0.5 * height,
            ),
            about_origin: [
                [section.xx * height, section.xy * height, section.x * half],
                [section.xy * height, section.yy * height, section.y * half],
                [section.x * half, section.y * half, section.area * third],
            ],
        },
    );
}

fn profile_regions(curves: &[crate::profile::ProfileCurve]) -> Vec<Region> {
    Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap()
}

fn around_region(region: &Region, integrand: impl Fn(f64, f64) -> f64) -> f64 {
    region
        .pieces()
        .map(|piece| {
            let breaks: Vec<f64> = piece.range().split(64).collect();
            let integral = integrate(&breaks, |parameter| {
                let derivatives = piece.curve().evaluate(parameter);
                integrand(derivatives.point.x, derivatives.point.y) * derivatives.first.y
            });
            if piece.is_reversed() {
                -integral
            } else {
                integral
            }
        })
        .sum()
}

fn skin_of_turn(region: &Region) -> f64 {
    region
        .pieces()
        .map(|piece| {
            let breaks: Vec<f64> = piece.range().split(256).collect();
            TAU * integrate(&breaks, |parameter| {
                let derivatives = piece.curve().evaluate(parameter);
                derivatives.point.x * derivatives.first.length()
            })
        })
        .sum()
}

#[test]
fn a_revolved_spline_matches_its_profile() {
    let profile = profile_regions(&[
        spline(1, &[(1.0, 0.0), (3.0, 1.0), (3.0, 2.0), (1.0, 3.0)]),
        line(2, (1.0, 3.0), (1.0, 0.0)),
    ]);
    let region = &profile[0];
    let sign = around_region(region, |x, _| x).signum();
    let turn = |integrand: &dyn Fn(f64, f64) -> f64| sign * around_region(region, integrand);
    let volume = TAU * turn(&|x, _| 0.5 * x * x);
    let height = TAU * turn(&|x, y| 0.5 * x * x * y);
    let across = PI * turn(&|x, _| x.powi(4) / 4.0);
    let along = TAU * turn(&|x, y| 0.5 * x * x * y * y);
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).unwrap();

    check(
        "revolved spline",
        &revolve(&Plane::XY, &profile, axis, AngularExtent::full(), FEATURE).unwrap(),
        &Expected {
            volume,
            area: skin_of_turn(region),
            centroid: Point3::new(0.0, height / volume, 0.0),
            about_origin: diagonal(across, along, across),
        },
    );
}

#[test]
fn an_extruded_spline_sketch_matches_its_profile() {
    let profile = profile_regions(&[
        spline(1, &[(0.0, 0.0), (4.0, -2.0), (6.0, 3.0), (2.0, 5.0)]),
        line(2, (2.0, 5.0), (0.0, 0.0)),
    ]);
    let region = &profile[0];
    let sign = around_region(region, |x, _| x).signum();
    let flat = |integrand: &dyn Fn(f64, f64) -> f64| sign * around_region(region, integrand);
    let height = 3.0;
    let area = flat(&|x, _| x);
    let (x, y) = (flat(&|x, _| 0.5 * x * x), flat(&|x, y| x * y));
    let (xx, xy, yy) = (
        flat(&|x, _| x.powi(3) / 3.0),
        flat(&|x, y| 0.5 * x * x * y),
        flat(&|x, y| x * y * y),
    );
    let half = 0.5 * height * height;
    let rim: f64 = region
        .pieces()
        .map(|piece| {
            let breaks: Vec<f64> = piece.range().split(256).collect();
            integrate(&breaks, |parameter| {
                piece.curve().evaluate(parameter).first.length()
            })
        })
        .sum();

    check(
        "extruded spline sketch",
        &extrude(
            &Plane::XY,
            &profile,
            LinearExtent::one_side(height).unwrap(),
            FEATURE,
        )
        .unwrap(),
        &Expected {
            volume: area * height,
            area: 2.0 * area + rim * height,
            centroid: Point3::new(x / area, y / area, 0.5 * height),
            about_origin: [
                [xx * height, xy * height, x * half],
                [xy * height, yy * height, y * half],
                [x * half, y * half, area * height.powi(3) / 3.0],
            ],
        },
    );
}

fn block(min: Point3, max: Point3) -> Solid {
    cuboid(max - min)
        .transformed(&RigidTransform::translation(min - Point3::ZERO).unwrap())
        .unwrap()
}

#[test]
fn boolean_results_match_their_closed_forms() {
    let radius: f64 = 4.0;
    let ball = sphere(radius);
    let corner = block(Point3::ZERO, Point3::splat(10.0));
    let full = 4.0 / 3.0 * PI * radius.powi(3);
    let octant_volume = full / 8.0;
    let octant_centroid = Point3::splat(3.0 * radius / 8.0);
    let square = full * radius * radius / 40.0;
    let product = radius.powi(5) / 15.0;
    let octant_moment = [
        [square, product, product],
        [product, square, product],
        [product, product, square],
    ];
    let whole =
        diagonal(1.0, 1.0, 1.0).map(|row| row.map(|entry| entry * full * radius * radius / 5.0));
    let notched_moment: SecondMoment = std::array::from_fn(|row| {
        std::array::from_fn(|column| whole[row][column] - octant_moment[row][column])
    });
    let notched_volume = full - octant_volume;

    check(
        "octant",
        &boolean(&ball, &corner, BooleanOperation::Intersection).unwrap(),
        &Expected {
            volume: octant_volume,
            area: 1.25 * PI * radius * radius,
            centroid: octant_centroid,
            about_origin: octant_moment,
        },
    );
    check(
        "notched ball",
        &boolean(&ball, &corner, BooleanOperation::Difference).unwrap(),
        &Expected {
            volume: notched_volume,
            area: 3.5 * PI * radius * radius + 0.75 * PI * radius * radius,
            centroid: -octant_centroid * (octant_volume / notched_volume),
            about_origin: notched_moment,
        },
    );
}

#[test]
fn extents_reach_the_curved_extremes() {
    let ball = extent(&sphere(4.0)).unwrap().unwrap();
    let ring = extent(&torus(6.0, 2.0)).unwrap().unwrap();
    let swept = extent(&extruded_spline(5.0)).unwrap().unwrap();
    let curve = spline_profile();
    let samples = (0..=100_000).map(|step| curve.point(step as f64 / 100_000.0));
    let (widest, tallest) = samples.fold((f64::MIN, f64::MIN), |(x, y), point| {
        (x.max(point.x), y.max(point.y))
    });

    assert!(ball.min().distance(Point3::splat(-4.0)) < 1e-12);
    assert!(ball.max().distance(Point3::splat(4.0)) < 1e-12);
    assert!(ring.min().distance(Point3::new(-8.0, -8.0, -2.0)) < 1e-12);
    assert!(ring.max().distance(Point3::new(8.0, 8.0, 2.0)) < 1e-12);
    assert!((swept.max().x - widest).abs() < 1e-9);
    assert!(swept.max().x >= widest);
    assert!((swept.max().y - tallest).abs() < 1e-9);
    assert!(swept.max().y >= tallest);
    assert!((swept.max().z - 5.0).abs() < 1e-12);
}

#[test]
fn a_block_under_a_spline_sheet_matches_a_direct_integral() {
    let (side, height, nodes, bump) = (10.0, 3.0, 12, 1.5);
    let sheet = bumped_sheet(side, height, nodes, bump);
    let spans: Vec<f64> = (0..=nodes - 3)
        .map(|knot| knot as f64 / (nodes - 3) as f64)
        .collect();
    let mut breaks: Vec<f64> = Vec::new();
    for pair in spans.windows(2) {
        breaks.extend(Interval::new(pair[0], pair[1]).unwrap().split(4));
    }
    breaks.dedup();
    let surface = Surface::BSpline(sheet);
    let over_sheet = |integrand: &dyn Fn(Point3, Vector3) -> f64| {
        integrate(&breaks, |u| {
            integrate(&breaks, |v| {
                let at = surface.evaluate(u, v);
                integrand(at.point, at.du.cross(at.dv))
            })
        })
    };
    let volume = over_sheet(&|point, normal| point.z * normal.z);
    let top = over_sheet(&|_, normal| normal.length());
    let first = Vector3::new(
        over_sheet(&|point, normal| point.x * point.z * normal.z),
        over_sheet(&|point, normal| point.y * point.z * normal.z),
        over_sheet(&|point, normal| 0.5 * point.z * point.z * normal.z),
    );
    let walls: f64 = (0..4)
        .map(|wall| {
            let edge = |t: f64| match wall {
                0 => (t, 0.0),
                1 => (1.0, t),
                2 => (t, 1.0),
                _ => (0.0, t),
            };
            integrate(&breaks, |t| {
                let (u, v) = edge(t);
                let at = surface.evaluate(u, v);
                let along = if wall % 2 == 0 { at.du } else { at.dv };
                at.point.z * along.length()
            })
        })
        .sum();
    let found = measured(
        "spline-topped block",
        &bumped_block(side, height, nodes, bump),
    );

    close(
        "spline-topped block",
        "volume",
        found.volume,
        volume,
        volume,
    );
    close(
        "spline-topped block",
        "area",
        found.area,
        top + side * side + walls,
        found.area,
    );
    let centroid = first / volume;
    assert!(
        found.centroid.distance(centroid) <= RELATIVE * side,
        "{found:?} {centroid}"
    );
}

#[test]
fn extents_find_a_bump_inside_a_spline_face() {
    let sheet = bumped_sheet(10.0, 3.0, 12, 1.5);
    let surface = Surface::BSpline(sheet);
    let highest_on = |center: Point2, reach: f64| {
        let steps = 200;
        (0..=steps)
            .flat_map(|row| (0..=steps).map(move |column| (row, column)))
            .map(|(row, column)| {
                let offset = Vector2::new(column as f64, row as f64) / steps as f64 - 0.5;
                let uv = (center + offset * 2.0 * reach).clamp(Point2::ZERO, Point2::ONE);
                (surface.point_at(uv).z, uv)
            })
            .fold((f64::MIN, center), |best, found| {
                if found.0 > best.0 { found } else { best }
            })
    };
    let (_, coarse) = highest_on(Point2::splat(0.5), 0.5);
    let (_, finer) = highest_on(coarse, 0.01);
    let (highest, _) = highest_on(finer, 1e-4);

    let bounds = extent(&bumped_block(10.0, 3.0, 12, 1.5)).unwrap().unwrap();

    assert!(bounds.max().z >= highest);
    assert!(bounds.max().z - highest < 1e-9);
}
