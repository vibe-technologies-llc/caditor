use caditor_geometry::{Plane, Point2, Point3, Vector3};

use super::{
    BlendShape, flat,
    section::{Blend, Section, SectionCurve, SectionSide, Side},
};
use crate::{
    intersect::solve_dense,
    surface::{Surface, SurfaceDerivatives},
    topology::{FaceContainment, FaceId, Solid, SolidClassifier},
};

const MAX_ITERATIONS: usize = 60;
const CONVERGED: f64 = 1e-12;
const SMALLEST_STEP: f64 = 1e-15;
const REACHES: f64 = 1e-9;
const FARTHEST_FOOT: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Contact {
    pub face: FaceId,
    pub uv: Point2,
    pub point: Point3,
    pub normal: Vector3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Cross {
    pub point: Point3,
    pub tangent: Vector3,
    pub feet: [Contact; 2],
    pub center: Option<Point3>,
    pub inside: [bool; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Missed {
    Short,
    Angle,
}

pub(super) struct Sides<'a> {
    pub solid: &'a Solid,
    pub classifier: &'a SolidClassifier<'a>,
    pub candidates: [&'a [FaceId]; 2],
    pub convex: bool,
}

#[derive(Debug, Clone, Copy)]
struct Local {
    point: Point3,
    du: Vector3,
    dv: Vector3,
    normal: Vector3,
    nu: Vector3,
    nv: Vector3,
}

pub(super) fn derivatives(surface: &Surface, uv: Point2) -> SurfaceDerivatives {
    match surface {
        Surface::BSpline(spline) => spline.extended_evaluate(uv.x, uv.y),
        other => other.evaluate(uv.x, uv.y),
    }
}

fn local(surface: &Surface, sign: f64, uv: Point2) -> Option<Local> {
    let derivatives = derivatives(surface, uv);
    let cross = derivatives.du.cross(derivatives.dv);
    let length = cross.length();
    if !length.is_finite() || length <= f64::MIN_POSITIVE {
        return None;
    }
    let normal = cross / length;
    let along_u = derivatives.duu.cross(derivatives.dv) + derivatives.du.cross(derivatives.duv);
    let along_v = derivatives.duv.cross(derivatives.dv) + derivatives.du.cross(derivatives.dvv);
    let turned = |change: Vector3| (change - normal * normal.dot(change)) / length * sign;
    Some(Local {
        point: derivatives.point,
        du: derivatives.du,
        dv: derivatives.dv,
        normal: normal * sign,
        nu: turned(along_u),
        nv: turned(along_v),
    })
}

pub(super) fn newton<const N: usize>(
    start: [f64; N],
    residual: impl Fn(&[f64; N]) -> Option<([f64; N], [[f64; N]; N])>,
) -> Option<[f64; N]> {
    let mut current = start;
    let norm = |values: &[f64; N]| values.iter().map(|value| value * value).sum::<f64>().sqrt();
    let (mut values, mut jacobian) = residual(&current)?;
    for _ in 0..MAX_ITERATIONS {
        if norm(&values) <= CONVERGED {
            return Some(current);
        }
        let matrix: Vec<f64> = jacobian.iter().flatten().copied().collect();
        let rhs: Vec<f64> = values.iter().map(|value| -value).collect();
        let step = solve_dense(N, matrix, rhs)?;
        let mut scale = 1.0;
        let mut accepted = None;
        while scale >= 1.0 / 1024.0 {
            let mut trial = current;
            for (slot, delta) in trial.iter_mut().zip(&step) {
                *slot += scale * delta;
            }
            if let Some(next) = residual(&trial)
                && norm(&next.0) < norm(&values)
            {
                accepted = Some((trial, next));
                break;
            }
            scale *= 0.5;
        }
        let (trial, next) = accepted?;
        let moved = trial
            .iter()
            .zip(&current)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        current = trial;
        (values, jacobian) = next;
        if moved <= SMALLEST_STEP {
            break;
        }
    }
    (norm(&values) <= CONVERGED * 1e3).then_some(current)
}

struct Face<'a> {
    id: FaceId,
    surface: &'a Surface,
    sign: f64,
}

fn face<'a>(solid: &'a Solid, id: FaceId) -> Option<Face<'a>> {
    let definition = solid.face(id)?;
    Some(Face {
        id,
        surface: definition.surface(),
        sign: definition.sense().sign(),
    })
}

fn contact(face: &Face<'_>, uv: Point2) -> Option<Contact> {
    let here = local(face.surface, face.sign, uv)?;
    Some(Contact {
        face: face.id,
        uv,
        point: here.point,
        normal: here.normal,
    })
}

fn fillet_contacts(
    faces: [&Face<'_>; 2],
    starts: [Point2; 2],
    point: Point3,
    tangent: Vector3,
    radius: f64,
    convex: bool,
) -> Option<([Contact; 2], Point3)> {
    let toward = if convex { -radius } else { radius };
    let [first, second] = faces;
    let solved = newton(
        [starts[0].x, starts[0].y, starts[1].x, starts[1].y],
        |x: &[f64; 4]| {
            let a = local(first.surface, first.sign, Point2::new(x[0], x[1]))?;
            let b = local(second.surface, second.sign, Point2::new(x[2], x[3]))?;
            let center_a = a.point + a.normal * toward;
            let center_b = b.point + b.normal * toward;
            let gap = center_a - center_b;
            let columns = [
                a.du + a.nu * toward,
                a.dv + a.nv * toward,
                -(b.du + b.nu * toward),
                -(b.dv + b.nv * toward),
            ];
            let along = [columns[0].dot(tangent), columns[1].dot(tangent), 0.0, 0.0];
            let values = [gap.x, gap.y, gap.z, (center_a - point).dot(tangent)];
            let jacobian = [
                columns.map(|column| column.x),
                columns.map(|column| column.y),
                columns.map(|column| column.z),
                along,
            ];
            Some((values, jacobian))
        },
    )?;
    let feet = [
        contact(first, Point2::new(solved[0], solved[1]))?,
        contact(second, Point2::new(solved[2], solved[3]))?,
    ];
    let center = feet[0].point + feet[0].normal * toward;
    Some((feet, center))
}

fn chamfer_contact(
    face: &Face<'_>,
    start: Point2,
    point: Point3,
    tangent: Vector3,
    distance: f64,
) -> Option<Contact> {
    let solved = newton([start.x, start.y], |x: &[f64; 2]| {
        let here = local(face.surface, face.sign, Point2::new(x[0], x[1]))?;
        let offset = here.point - point;
        let values = [
            offset.length_squared() - distance * distance,
            offset.dot(tangent),
        ];
        let jacobian = [
            [2.0 * offset.dot(here.du), 2.0 * offset.dot(here.dv)],
            [here.du.dot(tangent), here.dv.dot(tangent)],
        ];
        Some((values, jacobian))
    })?;
    contact(face, Point2::new(solved[0], solved[1]))
}

fn line_contact(
    face: &Face<'_>,
    start: Point2,
    from: Point3,
    direction: Vector3,
) -> Option<(Contact, f64)> {
    let along = (face.surface.point_at(start) - from).dot(direction);
    let solved = newton([start.x, start.y, along], |x: &[f64; 3]| {
        let here = local(face.surface, face.sign, Point2::new(x[0], x[1]))?;
        let gap = here.point - (from + direction * x[2]);
        let columns = [here.du, here.dv, -direction];
        Some((
            [gap.x, gap.y, gap.z],
            [
                columns.map(|column| column.x),
                columns.map(|column| column.y),
                columns.map(|column| column.z),
            ],
        ))
    })?;
    Some((contact(face, Point2::new(solved[0], solved[1]))?, solved[2]))
}

fn rotated(vector: Vector3, axis: Vector3, angle: f64) -> Vector3 {
    let (sin, cos) = angle.sin_cos();
    vector * cos + axis.cross(vector) * sin + axis * axis.dot(vector) * (1.0 - cos)
}

struct Guess {
    frame: Plane,
    blend: Blend,
}

fn guess(
    point: Point3,
    tangent: Vector3,
    inward: [Vector3; 2],
    normals: [Vector3; 2],
    convex: bool,
    shape: BlendShape,
    measured_on: Side,
) -> Result<Guess, Missed> {
    let frame = Plane::from_frame(point, tangent, inward[0]).ok_or(Missed::Short)?;
    let side = |inward: Vector3, normal: Vector3| -> Option<SectionSide> {
        Some(SectionSide {
            curve: SectionCurve::Line,
            inward: flat(inward, &frame).try_normalize()?,
            normal: flat(normal, &frame).try_normalize()?,
        })
    };
    let section = Section {
        corner: Point2::ZERO,
        sides: [
            side(inward[0], normals[0]).ok_or(Missed::Short)?,
            side(inward[1], normals[1]).ok_or(Missed::Short)?,
        ],
        convex,
    };
    let blend = match shape {
        BlendShape::Fillet { radius } => section.fillet(radius),
        BlendShape::Chamfer { distance } => section.chamfer([distance, distance]),
        BlendShape::TwoDistanceChamfer { first, second, .. } => match measured_on {
            Side::First => section.chamfer([first, second]),
            Side::Second => section.chamfer([second, first]),
        },
        BlendShape::AngledChamfer {
            distance, angle, ..
        } => section.angled_chamfer(measured_on, distance, angle).ok(),
    }
    .ok_or(Missed::Short)?;
    Ok(Guess { frame, blend })
}

fn solve_with(
    faces: [&Face<'_>; 2],
    point: Point3,
    tangent: Vector3,
    guessed: &Guess,
    convex: bool,
    shape: BlendShape,
    measured_on: Side,
) -> Result<([Contact; 2], Option<Point3>), Missed> {
    let starts = [0, 1].map(|index| {
        let foot = guessed
            .blend
            .feet
            .get(index)
            .copied()
            .unwrap_or(Point2::ZERO);
        let at = guessed.frame.to_world(foot);
        faces
            .get(index)
            .map_or(Point2::ZERO, |face| face.surface.project(at, None))
    });
    match shape {
        BlendShape::Fillet { radius } => {
            let (feet, center) = fillet_contacts(faces, starts, point, tangent, radius, convex)
                .ok_or(Missed::Short)?;
            Ok((feet, Some(center)))
        }
        BlendShape::Chamfer { distance } => Ok((
            [
                chamfer_contact(faces[0], starts[0], point, tangent, distance)
                    .ok_or(Missed::Short)?,
                chamfer_contact(faces[1], starts[1], point, tangent, distance)
                    .ok_or(Missed::Short)?,
            ],
            None,
        )),
        BlendShape::TwoDistanceChamfer { first, second, .. } => {
            let distances = match measured_on {
                Side::First => [first, second],
                Side::Second => [second, first],
            };
            Ok((
                [
                    chamfer_contact(faces[0], starts[0], point, tangent, distances[0])
                        .ok_or(Missed::Short)?,
                    chamfer_contact(faces[1], starts[1], point, tangent, distances[1])
                        .ok_or(Missed::Short)?,
                ],
                None,
            ))
        }
        BlendShape::AngledChamfer {
            distance, angle, ..
        } => {
            let (near_index, far_index) = match measured_on {
                Side::First => (0, 1),
                Side::Second => (1, 0),
            };
            let (Some(near_face), Some(far_face), Some(near_start), Some(far_start)) = (
                faces.get(near_index),
                faces.get(far_index),
                starts.get(near_index),
                starts.get(far_index),
            ) else {
                return Err(Missed::Short);
            };
            let near = chamfer_contact(near_face, *near_start, point, tangent, distance)
                .ok_or(Missed::Short)?;
            let mut back = tangent.cross(near.normal);
            if back.dot(point - near.point) < 0.0 {
                back = -back;
            }
            let side = if convex { -1.0 } else { 1.0 };
            let direction = [angle, -angle]
                .map(|turn| rotated(back, tangent, turn))
                .into_iter()
                .find(|direction| direction.dot(near.normal) * side > 0.0)
                .ok_or(Missed::Angle)?;
            let (far, along) =
                line_contact(far_face, *far_start, near.point, direction).ok_or(Missed::Angle)?;
            if along <= REACHES {
                return Err(Missed::Angle);
            }
            let feet = match measured_on {
                Side::First => [near, far],
                Side::Second => [far, near],
            };
            Ok((feet, None))
        }
    }
}

fn is_inside(classifier: &SolidClassifier<'_>, foot: &Contact) -> bool {
    matches!(
        classifier.point_in_face(foot.face, foot.uv),
        Some(FaceContainment::Inside | FaceContainment::OnBoundary)
    )
}

pub(super) struct Request {
    pub point: Point3,
    pub tangent: Vector3,
    pub inward: [Vector3; 2],
    pub shape: BlendShape,
    pub measured_on: Side,
}

pub(super) fn cross(sides: &Sides<'_>, request: &Request) -> Result<Cross, Missed> {
    let Request {
        point,
        tangent,
        inward,
        shape,
        measured_on,
    } = *request;
    let own = [0, 1].map(|index| {
        sides
            .candidates
            .get(index)
            .and_then(|faces| faces.first())
            .and_then(|id| face(sides.solid, *id))
    });
    let [Some(first_own), Some(second_own)] = own else {
        return Err(Missed::Short);
    };
    let normals = [
        contact(&first_own, first_own.surface.project(point, None))
            .ok_or(Missed::Short)?
            .normal,
        contact(&second_own, second_own.surface.project(point, None))
            .ok_or(Missed::Short)?
            .normal,
    ];
    let guessed = guess(
        point,
        tangent,
        inward,
        normals,
        sides.convex,
        shape,
        measured_on,
    )?;
    let solve = |faces: [&Face<'_>; 2]| {
        solve_with(
            faces,
            point,
            tangent,
            &guessed,
            sides.convex,
            shape,
            measured_on,
        )
    };
    let guessed_feet = [0, 1].map(|index| {
        guessed.frame.to_world(
            guessed
                .blend
                .feet
                .get(index)
                .copied()
                .unwrap_or(Point2::ZERO),
        )
    });
    let guessed_reach = guessed_feet
        .iter()
        .map(|foot| foot.distance(point))
        .fold(0.0, f64::max);
    let near = |feet: &[Contact; 2]| {
        feet.iter()
            .all(|foot| foot.point.distance(point) <= FARTHEST_FOOT * guessed_reach)
    };
    let strayed = |feet: &[Contact; 2]| {
        feet.iter()
            .zip(&guessed_feet)
            .map(|(foot, guess)| foot.point.distance(*guess))
            .sum::<f64>()
    };
    let own_result = solve([&first_own, &second_own]);
    let mut best = own_result.as_ref().ok().map(|(feet, center)| {
        let inside = feet.map(|foot| is_inside(sides.classifier, &foot));
        (*feet, *center, inside)
    });
    let settled = best
        .as_ref()
        .is_some_and(|(feet, _, inside)| *inside == [true, true] && near(feet));
    if !settled {
        let nearby = |index: usize| -> Vec<Face<'_>> {
            let guess = guessed_feet.get(index).copied().unwrap_or(point);
            sides
                .candidates
                .get(index)
                .into_iter()
                .flat_map(|faces| faces.iter())
                .filter_map(|id| face(sides.solid, *id))
                .filter(|face| face.surface.distance(guess) <= FARTHEST_FOOT * guessed_reach)
                .collect()
        };
        let first_all = nearby(0);
        let second_all = nearby(1);
        let mut found: Option<([Contact; 2], Option<Point3>, f64)> = None;
        for a in &first_all {
            for b in &second_all {
                let Ok((feet, center)) = solve([a, b]) else {
                    continue;
                };
                let inside = feet.map(|foot| is_inside(sides.classifier, &foot));
                if inside != [true, true] || !near(&feet) {
                    continue;
                }
                let score = strayed(&feet);
                if found.as_ref().is_none_or(|(_, _, best)| score < *best) {
                    found = Some((feet, center, score));
                }
            }
        }
        if let Some((feet, center, _)) = found {
            best = Some((feet, center, [true, true]));
        }
    }
    let best = best.ok_or(Missed::Short)?;
    let (feet, center, inside) = best;
    let reaches = feet
        .iter()
        .zip(inward)
        .all(|(foot, inward)| (foot.point - point).dot(inward) > REACHES);
    if !reaches {
        return Err(Missed::Short);
    }
    Ok(Cross {
        point,
        tangent,
        feet,
        center,
        inside,
    })
}

impl Cross {
    pub fn arc(&self) -> Option<(Point3, f64)> {
        let center = self.center?;
        let [first, second] = self.feet.map(|foot| foot.point - center);
        let radius = first.length();
        let turned = first.angle_between(second);
        if !turned.is_finite() || turned <= 0.0 || turned >= std::f64::consts::PI {
            return None;
        }
        let half = 0.5 * turned;
        let middle = (first + second).try_normalize()? * (radius / half.cos());
        Some((center + middle, half.cos()))
    }
}

pub(super) fn on_both(surfaces: [&Surface; 2], point: Point3, across: Vector3) -> Option<Point3> {
    let starts = surfaces.map(|surface| surface.project(point, None));
    let solved = newton(
        [starts[0].x, starts[0].y, starts[1].x, starts[1].y],
        |x: &[f64; 4]| {
            let a = derivatives(surfaces[0], Point2::new(x[0], x[1]));
            let b = derivatives(surfaces[1], Point2::new(x[2], x[3]));
            let gap = a.point - b.point;
            let columns = [a.du, a.dv, -b.du, -b.dv];
            Some((
                [gap.x, gap.y, gap.z, (a.point - point).dot(across)],
                [
                    columns.map(|column| column.x),
                    columns.map(|column| column.y),
                    columns.map(|column| column.z),
                    [a.du.dot(across), a.dv.dot(across), 0.0, 0.0],
                ],
            ))
        },
    )?;
    Some(derivatives(surfaces[0], Point2::new(solved[0], solved[1])).point)
}
