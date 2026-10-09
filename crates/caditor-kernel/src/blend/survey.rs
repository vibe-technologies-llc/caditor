use std::f64::consts::PI;

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector2, Vector3};

use super::*;
use crate::{
    build::{AngularExtent, Axis2, LinearExtent, extrude, revolve},
    profile::{Profile, ProfileCurve, Selection},
    test_support::{arc, line},
};

fn moved(solid: Solid, offset: (f64, f64, f64)) -> Solid {
    let transform =
        RigidTransform::translation(Vector3::new(offset.0, offset.1, offset.2)).unwrap();
    solid.transformed(&transform).unwrap()
}

fn swept(plane: Plane, curves: &[ProfileCurve], height: f64) -> Solid {
    swept_as(plane, curves, height, 1)
}

fn swept_as(plane: Plane, curves: &[ProfileCurve], height: f64, feature: u64) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &plane,
        &regions,
        LinearExtent::one_side(height).unwrap(),
        feature,
    )
    .unwrap()
}

fn polygon(points: &[(f64, f64)]) -> Vec<ProfileCurve> {
    (0..points.len())
        .map(|index| {
            line(
                index as u64 + 1,
                points[index],
                points[(index + 1) % points.len()],
            )
        })
        .collect()
}

fn turned(curves: &[ProfileCurve]) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    revolve(
        &Plane::XZ,
        &regions,
        Axis2::new(Point2::ZERO, Vector2::Y).unwrap(),
        AngularExtent::full(),
        1,
    )
    .unwrap()
}

fn brick(min: (f64, f64, f64), size: (f64, f64, f64), feature: u64) -> Solid {
    let (x, y, z) = min;
    let (w, d, h) = size;
    let curves = polygon(&[(x, y), (x + w, y), (x + w, y + d), (x, y + d)]);
    let regions = Profile::new(&curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let plane = Plane::from_frame(Point3::new(0.0, 0.0, z), Vector3::Z, Vector3::X).unwrap();
    extrude(
        &plane,
        &regions,
        LinearExtent::one_side(h).unwrap(),
        feature,
    )
    .unwrap()
}

fn rod(center: (f64, f64, f64), radius: f64, height: f64, feature: u64) -> Solid {
    let curves = [crate::test_support::circle(1, (center.0, center.1), radius)];
    let regions = Profile::new(&curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let plane = Plane::from_frame(Point3::new(0.0, 0.0, center.2), Vector3::Z, Vector3::X).unwrap();
    extrude(
        &plane,
        &regions,
        LinearExtent::one_side(height).unwrap(),
        feature,
    )
    .unwrap()
}

fn bodies() -> Vec<(&'static str, Solid)> {
    let block = brick((0.0, 0.0, 0.0), (10.0, 10.0, 10.0), 1);
    let pocket = boolean(
        &block,
        &brick((3.0, 3.0, 6.0), (4.0, 4.0, 4.0), 11),
        BooleanOperation::Difference,
    )
    .unwrap();
    let boss = boolean(
        &block,
        &brick((3.0, 3.0, 10.0), (4.0, 4.0, 4.0), 12),
        BooleanOperation::Union,
    )
    .unwrap();
    let notch = boolean(
        &block,
        &brick((3.0, -1.0, 6.0), (4.0, 12.0, 4.0), 13),
        BooleanOperation::Difference,
    )
    .unwrap();
    let edge_notch = boolean(
        &block,
        &brick((8.0, 3.0, 6.0), (4.0, 4.0, 4.0), 14),
        BooleanOperation::Difference,
    )
    .unwrap();
    let corner_cut = boolean(
        &block,
        &brick((8.0, 8.0, 8.0), (4.0, 4.0, 4.0), 15),
        BooleanOperation::Difference,
    )
    .unwrap();
    let round_boss = boolean(
        &block,
        &rod((5.0, 5.0, 10.0), 2.0, 4.0, 18),
        BooleanOperation::Union,
    )
    .unwrap();
    let hole = boolean(
        &block,
        &rod((5.0, 5.0, -1.0), 2.0, 12.0, 19),
        BooleanOperation::Difference,
    )
    .unwrap();
    let blind = boolean(
        &block,
        &rod((5.0, 5.0, 5.0), 2.0, 6.0, 20),
        BooleanOperation::Difference,
    )
    .unwrap();
    let edge_hole = boolean(
        &block,
        &rod((10.0, 5.0, -1.0), 2.0, 12.0, 21),
        BooleanOperation::Difference,
    )
    .unwrap();
    let wedge = swept(
        Plane::XY,
        &polygon(&[(0.0, 0.0), (10.0, 0.0), (8.0, 6.0), (0.0, 6.0)]),
        4.0,
    );
    let acute = swept(
        Plane::XY,
        &polygon(&[(0.0, 0.0), (10.0, 0.0), (0.0, 5.0)]),
        4.0,
    );
    let l_shape = swept(
        Plane::XY,
        &polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 4.0),
            (4.0, 4.0),
            (4.0, 10.0),
            (0.0, 10.0),
        ]),
        5.0,
    );
    let pentagon: Vec<(f64, f64)> = (0..5)
        .map(|index| {
            let angle = 2.0 * PI / 5.0 * index as f64;
            (5.0 * angle.cos(), 5.0 * angle.sin())
        })
        .collect();
    let pentagon = swept(Plane::XY, &polygon(&pentagon), 3.0);
    let stadium = swept(
        Plane::XY,
        &[
            line(1, (0.0, 0.0), (10.0, 0.0)),
            arc(2, (10.0, 2.0), (10.0, 0.0), (10.0, 4.0)),
            line(3, (10.0, 4.0), (0.0, 4.0)),
            arc(4, (0.0, 2.0), (0.0, 4.0), (0.0, 0.0)),
        ],
        3.0,
    );
    let d_shape = swept(
        Plane::XY,
        &[
            line(1, (0.0, 0.0), (10.0, 0.0)),
            arc(2, (5.0, 0.0), (10.0, 0.0), (0.0, 0.0)),
        ],
        3.0,
    );
    let shaft = turned(&polygon(&[
        (0.0, 0.0),
        (5.0, 0.0),
        (5.0, 4.0),
        (3.0, 4.0),
        (3.0, 10.0),
        (0.0, 10.0),
    ]));
    let tube = turned(&polygon(&[(2.0, 0.0), (5.0, 0.0), (5.0, 6.0), (2.0, 6.0)]));
    let cone_top = turned(&polygon(&[
        (0.0, 0.0),
        (5.0, 0.0),
        (5.0, 4.0),
        (2.0, 7.0),
        (0.0, 7.0),
    ]));
    let slanted_corner = {
        let cutter = swept_as(
            Plane::from_frame(
                Point3::new(10.0, 10.0, 10.0),
                Vector3::new(1.0, 1.0, 1.0),
                Vector3::new(1.0, -1.0, 0.0),
            )
            .unwrap(),
            &polygon(&[(-20.0, -20.0), (20.0, -20.0), (20.0, 20.0), (-20.0, 20.0)]),
            20.0,
            40,
        );
        let cutter = moved(cutter, (-2.0, -2.0, -2.0));
        boolean(&block, &cutter, BooleanOperation::Difference).unwrap()
    };
    let cylinder_on_side = boolean(
        &block,
        &rod((5.0, 5.0, -2.0), 3.0, 14.0, 22),
        BooleanOperation::Union,
    )
    .unwrap();
    let frustum = {
        let regions = Profile::new(&polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]))
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
        crate::build::extrude_tapered(
            &Plane::XY,
            &regions,
            LinearExtent::one_side(4.0).unwrap(),
            0.25,
            1,
        )
        .unwrap()
    };
    let step = swept(
        Plane::XZ,
        &polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 3.0),
            (4.0, 3.0),
            (4.0, 6.0),
            (0.0, 6.0),
        ]),
        -5.0,
    );
    vec![
        ("frustum", frustum),
        ("step", step),
        ("block", block),
        ("pocket", pocket),
        ("boss", boss),
        ("notch", notch),
        ("edge_notch", edge_notch),
        ("corner_cut", corner_cut),
        ("round_boss", round_boss),
        ("hole", hole),
        ("blind", blind),
        ("edge_hole", edge_hole),
        ("wedge", wedge),
        ("acute", acute),
        ("l_shape", l_shape),
        ("pentagon", pentagon),
        ("stadium", stadium),
        ("d_shape", d_shape),
        ("shaft", shaft),
        ("tube", tube),
        ("cone_top", cone_top),
        ("slanted_corner", slanted_corner),
        ("cylinder_through", cylinder_on_side),
    ]
}

fn describe(solid: &Solid, edge: EdgeId) -> String {
    let Some(definition) = solid.edge(edge) else {
        return format!("{edge:?}");
    };
    let middle = definition.curve().point(definition.interval().middle());
    let kind = match definition.curve() {
        Curve::Line(_) => "line",
        Curve::Circle(_) => "circle",
        _ => "other",
    };
    let convex = analyze(solid, edge).map_or("?", |geometry| {
        if geometry.section.convex {
            "convex"
        } else {
            "concave"
        }
    });
    format!(
        "{kind} {convex} @({:.2},{:.2},{:.2})",
        middle.x, middle.y, middle.z
    )
}

fn verdict(solid: &Solid, result: &Result<Solid, BlendError>) -> Option<String> {
    match result {
        Ok(result) => match result.validate() {
            Ok(()) => match result.tessellate(&result.default_tolerance()) {
                Ok(_) => None,
                Err(error) => Some(format!("mesh {error:?}")),
            },
            Err(error) => Some(format!("invalid {error:?}")),
        },
        Err(error) => Some(format!(
            "{error:?} [{}]",
            error
                .edge()
                .map_or(String::new(), |edge| describe(solid, edge))
        )),
    }
}

fn corner_sets(solid: &Solid, sharp: &BTreeSet<EdgeId>) -> BTreeSet<Vec<EdgeId>> {
    let topology = Topology::new(solid);
    let mut sets: BTreeSet<Vec<EdgeId>> = sharp.iter().map(|edge| vec![*edge]).collect();
    sets.insert(sharp.iter().copied().collect());
    for (vertex, _) in solid.vertices() {
        let at: Vec<EdgeId> = topology
            .edges_at(vertex)
            .iter()
            .copied()
            .filter(|edge| sharp.contains(edge))
            .take(5)
            .collect();
        for mask in 1..(1u32 << at.len()) {
            let set: Vec<EdgeId> = at
                .iter()
                .enumerate()
                .filter(|(bit, _)| mask & (1 << bit) != 0)
                .map(|(_, edge)| *edge)
                .collect();
            if set.len() >= 2 {
                sets.insert(
                    set.into_iter()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                );
            }
        }
    }
    sets
}

fn survey_body(name: &str, solid: &Solid, distance: f64) -> Vec<String> {
    let sharp: BTreeSet<EdgeId> = solid
        .edges()
        .map(|(id, _)| id)
        .filter(|edge| analyze(solid, *edge).is_ok())
        .collect();
    corner_sets(solid, &sharp)
        .iter()
        .filter_map(|set| {
            let result = blend(solid, set, BlendShape::Chamfer { distance }, 50);
            let why = verdict(solid, &result)?;
            let edges: Vec<String> = set.iter().map(|edge| describe(solid, *edge)).collect();
            Some(format!("{name} {}: {why}", edges.join(" + ")))
        })
        .collect()
}

#[test]
#[ignore = "a survey of every corner of twenty bodies, about a minute in a debug build"]
fn every_corner_chamfers_but_a_convex_edge_rising_from_bevelled_concave_ones() {
    let failures: Vec<String> = bodies()
        .iter()
        .flat_map(|(name, solid)| survey_body(name, solid, 0.5))
        .collect();
    let unexpected: Vec<&String> = failures
        .iter()
        .filter(|failure| !(failure.starts_with("boss ") && failure.contains("UnsupportedEnd")))
        .collect();

    assert!(unexpected.is_empty(), "{unexpected:#?}");
    assert_eq!(failures.len(), 5);
}
