use caditor_geometry::{Plane, Point3, Ray, RigidTransform, Vector3};

const ALIGNED_ANGLE: f64 = 1e-12;
const DISTINCT_SINE: f64 = 1e-9;
const SLIDING_COSINE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Round {
    Cylinder { axis: Ray, radius: f64 },
    Sphere { centre: Point3, radius: f64 },
}

impl Round {
    pub(crate) fn same_as(&self, other: &Self, tolerance: f64) -> bool {
        match (self, other) {
            (
                Self::Cylinder { axis, radius },
                Self::Cylinder {
                    axis: other_axis,
                    radius: other_radius,
                },
            ) => {
                let along = axis.direction().normalize_or_zero();
                let offset = other_axis.origin() - axis.origin();
                (radius - other_radius).abs() <= tolerance
                    && along
                        .cross(other_axis.direction().normalize_or_zero())
                        .length()
                        <= DISTINCT_SINE
                    && (offset - along * offset.dot(along)).length() <= tolerance
            }
            (
                Self::Sphere { centre, radius },
                Self::Sphere {
                    centre: other_centre,
                    radius: other_radius,
                },
            ) => {
                (radius - other_radius).abs() <= tolerance
                    && centre.distance(*other_centre) <= tolerance
            }
            (Self::Cylinder { .. }, Self::Sphere { .. })
            | (Self::Sphere { .. }, Self::Cylinder { .. }) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Convexity {
    Convex,
    Concave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unplaced {
    TooFar,
    AxisAlongPlane,
}

fn placed(transform: Option<RigidTransform>) -> Result<RigidTransform, Unplaced> {
    transform.ok_or(Unplaced::TooFar)
}

fn turned_to_angle(
    from: Vector3,
    to: Vector3,
    angle: f64,
    about: Point3,
    across: Vector3,
) -> Option<RigidTransform> {
    let from = from.try_normalize()?;
    let to = to.try_normalize()?;
    let cross = from.cross(to);
    let current = cross.length().atan2(from.dot(to));
    let turn = current - angle;
    if turn.abs() <= ALIGNED_ANGLE {
        return Some(RigidTransform::IDENTITY);
    }
    let axis = if cross.length() > DISTINCT_SINE {
        cross
    } else {
        across
    };
    RigidTransform::rotation_about(about, axis, turn)
}

fn turning(from: Vector3, to: Vector3, about: Point3, across: Vector3) -> Option<RigidTransform> {
    turned_to_angle(from, to, 0.0, about, across)
}

fn nearest_on_line(line: Ray, point: Point3) -> Option<Point3> {
    let along = line.direction().try_normalize()?;
    Some(line.origin() + along * (point - line.origin()).dot(along))
}

pub(crate) fn faces(
    moving: &Plane,
    target: &Plane,
    distance: f64,
    flipped: bool,
    centre: Point3,
) -> Result<RigidTransform, Unplaced> {
    placed(face_placement(moving, target, distance, flipped, centre))
}

fn face_placement(
    moving: &Plane,
    target: &Plane,
    distance: f64,
    flipped: bool,
    centre: Point3,
) -> Option<RigidTransform> {
    let normal = target.normal().try_normalize()?;
    let facing = if flipped { normal } else { -normal };
    let turned = turning(moving.normal(), facing, centre, moving.x_axis())?;
    let origin = turned.apply_point(moving.origin());
    let goal = target.origin() + normal * distance;
    let shift = RigidTransform::translation(normal * (goal - origin).dot(normal))?;
    Some(turned.then(&shift))
}

pub(crate) fn axes(
    moving: Ray,
    target: Ray,
    flipped: bool,
    centre: Point3,
) -> Result<RigidTransform, Unplaced> {
    placed(axis_placement(moving, target, flipped, centre))
}

fn axis_placement(
    moving: Ray,
    target: Ray,
    flipped: bool,
    centre: Point3,
) -> Option<RigidTransform> {
    let along = moving.direction().try_normalize()?;
    let direction = target.direction().try_normalize()?;
    let direction = if flipped { -direction } else { direction };
    let pivot = nearest_on_line(moving, centre)?;
    let turned = turning(along, direction, pivot, along.any_orthonormal_vector())?;
    let point = turned.apply_point(pivot);
    let foot = target.origin() + direction * (point - target.origin()).dot(direction);
    let shift = RigidTransform::translation(foot - point)?;
    Some(turned.then(&shift))
}

pub(crate) struct FlushAndConcentric<'a> {
    pub face: &'a Plane,
    pub onto: &'a Plane,
    pub distance: f64,
    pub axis: Ray,
    pub along: Ray,
}

pub(crate) fn flush_and_concentric(
    pair: &FlushAndConcentric<'_>,
    flipped: bool,
    centre: Point3,
) -> Result<RigidTransform, Unplaced> {
    let normal = pair.onto.normal().try_normalize().ok_or(Unplaced::TooFar)?;
    let concentric = placed(axis_placement(pair.axis, pair.along, false, centre))?;
    let facing = concentric.apply_vector(pair.face.normal()).dot(normal);
    let opposed = facing <= 0.0;
    let concentric = if opposed == !flipped {
        concentric
    } else {
        placed(axis_placement(pair.axis, pair.along, true, centre))?
    };
    let direction = pair
        .along
        .direction()
        .try_normalize()
        .ok_or(Unplaced::TooFar)?;
    let slide = direction.dot(normal);
    if slide.abs() <= SLIDING_COSINE {
        return Err(Unplaced::AxisAlongPlane);
    }
    let origin = concentric.apply_point(pair.face.origin());
    let goal = pair.onto.origin() + normal * pair.distance;
    let along = (goal - origin).dot(normal) / slide;
    let shift = placed(RigidTransform::translation(direction * along))?;
    Ok(concentric.then(&shift))
}

fn crossing_nearest(first: &Plane, second: &Plane, centre: Point3) -> Option<Point3> {
    let first_normal = first.normal().try_normalize()?;
    let second_normal = second.normal().try_normalize()?;
    let cosine = first_normal.dot(second_normal);
    let determinant = 1.0 - cosine * cosine;
    if determinant <= DISTINCT_SINE {
        return None;
    }
    let first_gap = (first.origin() - centre).dot(first_normal);
    let second_gap = (second.origin() - centre).dot(second_normal);
    let first_share = (first_gap - cosine * second_gap) / determinant;
    let second_share = (second_gap - cosine * first_gap) / determinant;
    Some(centre + first_normal * first_share + second_normal * second_share)
}

fn closest_on_first(first: Ray, second: Ray, centre: Point3) -> Option<Point3> {
    let along = first.direction().try_normalize()?;
    let other = second.direction().try_normalize()?;
    let cosine = along.dot(other);
    let determinant = 1.0 - cosine * cosine;
    if determinant <= DISTINCT_SINE {
        return nearest_on_line(first, centre);
    }
    let gap = first.origin() - second.origin();
    let share = (cosine * other.dot(gap) - along.dot(gap)) / determinant;
    Some(first.origin() + along * share)
}

pub(crate) fn face_angle(
    moving: &Plane,
    target: &Plane,
    angle: f64,
    centre: Point3,
) -> Result<RigidTransform, Unplaced> {
    let pivot = crossing_nearest(moving, target, centre).unwrap_or(centre);
    placed(turned_to_angle(
        moving.normal(),
        -target.normal(),
        angle,
        pivot,
        moving.x_axis(),
    ))
}

pub(crate) fn axis_angle(
    moving: Ray,
    target: Ray,
    angle: f64,
    centre: Point3,
) -> Result<RigidTransform, Unplaced> {
    let pivot = closest_on_first(moving, target, centre).ok_or(Unplaced::TooFar)?;
    let along = moving.direction();
    placed(turned_to_angle(
        along,
        target.direction(),
        angle,
        pivot,
        along.any_orthonormal_vector(),
    ))
}

pub(crate) fn tangent(
    round: Round,
    target: &Plane,
    flipped: bool,
    centre: Point3,
) -> Result<RigidTransform, Unplaced> {
    placed(tangent_placement(round, target, flipped, centre))
}

fn tangent_placement(
    round: Round,
    target: &Plane,
    flipped: bool,
    centre: Point3,
) -> Option<RigidTransform> {
    let normal = target.normal().try_normalize()?;
    let (turned, middle, radius) = match round {
        Round::Cylinder { axis, radius } => {
            let along = axis.direction().try_normalize()?;
            let pivot = nearest_on_line(axis, centre)?;
            let lying = (along - normal * along.dot(normal))
                .try_normalize()
                .unwrap_or_else(|| normal.any_orthonormal_vector());
            let turned = turning(along, lying, pivot, along.any_orthonormal_vector())?;
            (turned, pivot, radius)
        }
        Round::Sphere { centre, radius } => (RigidTransform::IDENTITY, centre, radius),
    };
    let side = if flipped { -radius } else { radius };
    let height = (turned.apply_point(middle) - target.origin()).dot(normal);
    let shift = RigidTransform::translation(normal * (side - height))?;
    Some(turned.then(&shift))
}

pub(crate) fn face_on_round(
    moving: &Plane,
    round: Round,
    convexity: Convexity,
    flipped: bool,
    centre: Point3,
) -> Result<RigidTransform, Unplaced> {
    placed(face_on_round_placement(
        moving, round, convexity, flipped, centre,
    ))
}

fn face_on_round_placement(
    moving: &Plane,
    round: Round,
    convexity: Convexity,
    flipped: bool,
    centre: Point3,
) -> Option<RigidTransform> {
    let normal = moving.normal().try_normalize()?;
    let (turned, middle, radius) = match round {
        Round::Cylinder { axis, radius } => {
            let along = axis.direction().try_normalize()?;
            let lying = (normal - along * normal.dot(along))
                .try_normalize()
                .unwrap_or_else(|| along.any_orthonormal_vector());
            let turned = turning(normal, lying, centre, moving.x_axis())?;
            (turned, axis.origin(), radius)
        }
        Round::Sphere { centre, radius } => (RigidTransform::IDENTITY, centre, radius),
    };
    let facing = turned.apply_vector(normal);
    let origin = turned.apply_point(moving.origin());
    let outside = (convexity == Convexity::Convex) != flipped;
    let side = if outside { radius } else { -radius };
    let height = (middle - origin).dot(facing);
    let shift = RigidTransform::translation(facing * (height - side))?;
    Some(turned.then(&shift))
}

pub(crate) fn point_onto_point(moving: Point3, target: Point3) -> Result<RigidTransform, Unplaced> {
    placed(RigidTransform::translation(target - moving))
}

pub(crate) fn point_onto_plane(moving: Point3, target: &Plane) -> Result<RigidTransform, Unplaced> {
    let normal = target.normal().try_normalize().ok_or(Unplaced::TooFar)?;
    placed(RigidTransform::translation(
        normal * (target.origin() - moving).dot(normal),
    ))
}
