use std::f64::consts::{PI, TAU};

use caditor_geometry::{Maths, Plane, Point3, Vector3};

use super::*;
use crate::{
    build::{PathSegment, PipeSection, PipeShape, pipe},
    coil::{HelixSpec, HelixTurn, SpiralSpec, helix, spiral},
    fixtures,
    interval::Interval,
    numeric::GAUSS_LEGENDRE,
    surface::{BSplineSurface, Surface, SurfaceDerivatives},
    tally,
    tessellation::MassProperties,
    thread::{Handedness, ThreadEnd, ThreadForm, ThreadSpec, ThreadedFace, thread},
    tolerance::SamplingTolerance,
    topology::{Face, Solid},
};

const FEATURE: u64 = 7;
const PIPE_SIZE: f64 = 1.0;
const SECTION_AREA: f64 = PI * PIPE_SIZE * PIPE_SIZE / 4.0;
const MODELLED_PATH: f64 = 1e-6;
const DIRECT_REFERENCE: f64 = 1e-10;
const THREAD_REFERENCE: f64 = 1e-6;
const SPLIT_RATIONAL_SPANS: usize = 4;
const SPLIT_POLYNOMIAL_SPANS: usize = 2;

fn flat() -> Plane {
    Plane::with_x_axis(Point3::ZERO, Vector3::Z, Vector3::X).unwrap()
}

fn round() -> PipeSection {
    PipeSection {
        shape: PipeShape::Round,
        size: PIPE_SIZE,
        wall: None,
    }
}

fn piped(turns: &[HelixTurn]) -> Solid {
    let path: Vec<PathSegment> = turns
        .iter()
        .zip(1_u128..)
        .map(|(turn, key)| turn.path_segment(key))
        .collect();
    pipe(&path, &round(), 0.0, Vector3::Z, FEATURE).unwrap()
}

fn helix_spec() -> HelixSpec {
    HelixSpec {
        frame: flat(),
        radius: 10.0,
        pitch: 4.0,
        turns: 3.0,
        taper: 0.0,
        hand: Handedness::Right,
        start_angle: 0.0,
        ends: None,
        final_pitch: None,
        table: Vec::new(),
    }
}

fn spiral_spec() -> SpiralSpec {
    SpiralSpec {
        frame: flat(),
        radius: 5.0,
        pitch: 5.0,
        turns: 3.0,
        hand: Handedness::Right,
        start_angle: 0.0,
        final_pitch: None,
        table: Vec::new(),
    }
}

fn measured(solid: &Solid) -> (SolidMass, usize) {
    let tolerance = SamplingTolerance::new(1e-3, 0.1).unwrap();
    let (found, evaluations) = tally::measure(|| mass_properties_tessellating(solid, &tolerance));
    (found.unwrap(), evaluations)
}

fn assert_close(what: &str, found: f64, expected: f64, relative: f64) {
    assert!(
        (found - expected).abs() <= relative * expected.abs(),
        "{what}: {found} against {expected}"
    );
}

fn spans(knots: &[f64], domain: Interval, split: usize) -> Vec<(f64, f64)> {
    let mut ends: Vec<f64> = knots
        .iter()
        .copied()
        .filter(|knot| domain.contains(*knot))
        .collect();

    ends.dedup();
    ends.windows(2)
        .filter_map(|pair| Interval::new(*pair.first()?, *pair.get(1)?))
        .flat_map(|span| {
            (0..split).filter_map(move |part| {
                let at = |share: usize| span.at(share as f64 / split as f64);
                Interval::new(at(part), at(part + 1))
            })
        })
        .flat_map(|piece| {
            let half = 0.5 * piece.length();
            GAUSS_LEGENDRE
                .iter()
                .map(move |(node, weight)| (piece.middle() + half * node, weight * half))
        })
        .collect()
}

fn over_cells(
    spline: &BSplineSurface,
    along_split: usize,
    integrand: impl Fn(&SurfaceDerivatives) -> f64,
) -> f64 {
    let across = spans(spline.u_knots(), spline.u_domain(), SPLIT_RATIONAL_SPANS);
    let along = spans(spline.v_knots(), spline.v_domain(), along_split);
    let mut sum = 0.0;

    for (u, u_weight) in &across {
        for (v, v_weight) in &along {
            sum += u_weight * v_weight * integrand(&spline.evaluate(*u, *v));
        }
    }
    sum
}

fn face_flux(face: &Face) -> f64 {
    let flux = match face.surface() {
        Surface::BSpline(spline) => over_cells(spline, 1, |at| {
            (at.point - Point3::ZERO).dot(at.du.cross(at.dv))
        }),
        Surface::Plane(plane) => {
            let frame = plane.frame();
            (frame.origin() - Point3::ZERO).dot(frame.normal()) * SECTION_AREA
        }
        other => panic!("a pipe face on {other:?}"),
    };
    flux * face.sense().sign()
}

fn pipe_face_area(face: &Face) -> f64 {
    match face.surface() {
        Surface::BSpline(spline) => over_cells(spline, SPLIT_POLYNOMIAL_SPANS, |at| {
            at.du.cross(at.dv).length()
        }),
        Surface::Plane(_) => SECTION_AREA,
        other => panic!("a pipe face on {other:?}"),
    }
}

fn direct_volume(solid: &Solid) -> f64 {
    let flux: f64 = solid.faces().map(|(_, face)| face_flux(face)).sum();
    flux / 3.0
}

fn direct_area(solid: &Solid) -> f64 {
    solid.faces().map(|(_, face)| pipe_face_area(face)).sum()
}

fn measured_exactly(name: &str, solid: &Solid, most_evaluations: usize) -> MassProperties {
    let (found, evaluations) = measured(solid);

    assert!(
        found.meshed_faces.is_empty(),
        "{name}: {:?}",
        found.meshed_faces
    );
    assert!(
        evaluations <= most_evaluations,
        "{name}: {evaluations} evaluations"
    );
    found.properties
}

fn assert_pipe_volume(name: &str, solid: &Solid, found: &MassProperties, modelled: f64) {
    assert_close(name, found.volume, modelled, MODELLED_PATH);
    assert_close(name, found.volume, direct_volume(solid), DIRECT_REFERENCE);
    assert_close(name, found.area, direct_area(solid), DIRECT_REFERENCE);
}

#[test]
fn a_pipe_along_a_helix_measures_exactly_in_few_evaluations() {
    let spec = helix_spec();
    let solid = piped(&helix(&spec).unwrap());

    let turn = (TAU * spec.radius).hypot_libm(spec.pitch);
    let length = spec.turns * turn;
    let found = measured_exactly("helix", &solid, 800_000);

    assert_pipe_volume("helix", &solid, &found, SECTION_AREA * length);
    assert_close(
        "helix",
        found.area,
        PI * PIPE_SIZE * length + 2.0 * SECTION_AREA,
        MODELLED_PATH,
    );
}

#[test]
fn a_pipe_along_a_flat_spiral_measures_exactly_in_few_evaluations() {
    let spec = spiral_spec();
    let solid = piped(&spiral(&spec).unwrap());

    let growth = spec.pitch / TAU;
    let reach = spec.radius.hypot_libm(growth);
    let sweep = spec.sweep();

    let swept = ((spec.radius * spec.radius + growth * growth) * sweep
        + growth * spec.radius * sweep * sweep / 2.0)
        / reach;

    let found = measured_exactly("spiral", &solid, 1_600_000);

    assert_pipe_volume("spiral", &solid, &found, SECTION_AREA * swept);
}

fn groove_per_length(form: &ThreadForm, from: f64, to: f64) -> f64 {
    let steps = 20_000;
    let width = (to - from) / f64::from(steps);

    let moment: f64 = (0..steps)
        .map(|step| {
            let radius = from + width * (f64::from(step) + 0.5);
            2.0 * form.half_width_at(radius) * radius * width
        })
        .sum();
    TAU * moment / form.pitch
}

#[test]
fn a_threaded_m4_shaft_measures_exactly_in_few_evaluations() {
    let (radius, length, pitch) = (2.0, 20.0, 0.7);
    let fundamental = 3.0_f64.sqrt() / 2.0 * pitch;

    let form = ThreadForm {
        pitch,
        flank_angle: 30.0_f64.to_radians(),
        root_radius: radius - 5.0 / 8.0 * fundamental,
        root_width: pitch / 4.0,
        crest_radius: radius,
    };
    let spec = ThreadSpec {
        frame: Plane::from_frame(Point3::new(0.0, 0.0, length), -Vector3::Z, Vector3::X).unwrap(),
        face: ThreadedFace::Shaft,
        face_radius: radius,
        form,
        hand: Handedness::Right,
        length,
        start: ThreadEnd::RunsOut,
        end: ThreadEnd::RunsOut,
    };
    let threaded = thread(&fixtures::cylinder(radius, length), &spec, FEATURE).unwrap();

    let expected =
        PI * radius * radius * length - groove_per_length(&form, form.root_radius, radius) * length;
    let found = measured_exactly("M4x20", &threaded, 650_000);

    assert_close("M4x20", found.volume, expected, THREAD_REFERENCE);
}
