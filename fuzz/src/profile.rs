use caditor_geometry::{Point2, RigidTransform, Rotation3, Vector2, Vector3};
use caditor_kernel::{
    AngularExtent, Axis2, LinearBound, LinearExtent, Profile, ProfileCurve, Region, Selection,
    Solid, extrude, revolve,
};
use libfuzzer_sys::arbitrary::{Result, Unstructured};

use crate::{angle, coordinate, length, plane, point};

pub const FEATURE: u64 = 1;

const MOST_SIDES: u8 = 8;
const MOST_DEGREE: usize = 3;
const MOST_EXTRA_CONTROL_POINTS: usize = 4;
const KNOT_STEPS: u8 = 8;
const QUARTER_TURNS: u8 = 4;

#[derive(Default)]
struct Entities(u64);

impl Entities {
    fn next(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }
}

fn on_circle(center: Point2, radius: f64, angle: f64) -> Point2 {
    center + radius * Vector2::from_angle(angle)
}

fn spline(input: &mut Unstructured, entity: u64) -> Result<ProfileCurve> {
    let degree = input.int_in_range(1..=MOST_DEGREE)?;
    let count = degree + 1 + input.int_in_range(0..=MOST_EXTRA_CONTROL_POINTS)?;
    let control_points = (0..count)
        .map(|_| point(input))
        .collect::<Result<Vec<_>>>()?;
    let knots = if input.ratio(1u8, 8)? {
        (0..count + degree + 1)
            .map(|_| Ok(f64::from(input.int_in_range(0..=KNOT_STEPS)?) / f64::from(KNOT_STEPS)))
            .collect::<Result<Vec<_>>>()?
    } else {
        let spans = count - degree;
        let interior = (1..spans).map(|knot| knot as f64 / spans as f64);
        std::iter::repeat_n(0.0, degree + 1)
            .chain(interior)
            .chain(std::iter::repeat_n(1.0, degree + 1))
            .collect()
    };
    Ok(ProfileCurve::spline(entity, degree, knots, control_points))
}

fn shape(
    input: &mut Unstructured,
    entities: &mut Entities,
    curves: &mut Vec<ProfileCurve>,
) -> Result<()> {
    match input.int_in_range(0u8..=5)? {
        0 => curves.push(ProfileCurve::line(
            entities.next(),
            point(input)?,
            point(input)?,
        )),
        1 => curves.push(ProfileCurve::circle(
            entities.next(),
            point(input)?,
            length(input)?,
        )),
        2 => {
            let (center, radius) = (point(input)?, length(input)?);
            let (start, end) = (angle(input)?, angle(input)?);
            curves.push(ProfileCurve::arc(
                entities.next(),
                center,
                on_circle(center, radius, start),
                on_circle(center, radius, end),
            ));
        }
        3 => {
            let corner = point(input)?;
            let far = corner + Vector2::new(length(input)?, length(input)?);
            let corners = [
                corner,
                Point2::new(far.x, corner.y),
                far,
                Point2::new(corner.x, far.y),
            ];
            for (index, start) in corners.iter().enumerate() {
                let end = corners[(index + 1) % corners.len()];
                curves.push(ProfileCurve::line(entities.next(), *start, end));
            }
        }
        4 => {
            let (center, radius) = (point(input)?, length(input)?);
            let sides = input.int_in_range(3..=MOST_SIDES)?;
            let turn = angle(input)?;
            let corner = |side: u8| {
                let along = std::f64::consts::TAU * f64::from(side) / f64::from(sides);
                on_circle(center, radius, turn + along)
            };
            for side in 0..sides {
                curves.push(ProfileCurve::line(
                    entities.next(),
                    corner(side),
                    corner(side + 1),
                ));
            }
        }
        _ => {
            let entity = entities.next();
            curves.push(spline(input, entity)?);
        }
    }
    Ok(())
}

pub fn profile_curves(input: &mut Unstructured, most_shapes: usize) -> Result<Vec<ProfileCurve>> {
    let mut entities = Entities::default();
    let mut curves = Vec::new();
    for _ in 0..input.int_in_range(1..=most_shapes)? {
        shape(input, &mut entities, &mut curves)?;
    }
    Ok(curves)
}

pub fn selection(input: &mut Unstructured, profile: &Profile) -> Result<Selection> {
    if input.arbitrary::<bool>()? {
        return Ok(Selection::EvenDepth);
    }
    let mut keys = Vec::new();
    for region in profile.regions() {
        if input.arbitrary::<bool>()? {
            keys.push(region.key());
        }
    }
    Ok(Selection::Regions(keys))
}

fn signed_length(input: &mut Unstructured) -> Result<f64> {
    let length = length(input)?;
    Ok(if input.arbitrary::<bool>()? {
        -length
    } else {
        length
    })
}

pub fn linear_extent(input: &mut Unstructured) -> Result<Option<LinearExtent>> {
    let extent = match input.int_in_range(0u8..=3)? {
        0 => LinearExtent::one_side(signed_length(input)?),
        1 => LinearExtent::symmetric(length(input)?),
        2 => LinearExtent::two_sided(length(input)?, length(input)?),
        _ => LinearExtent::between(
            LinearBound::Offset(signed_length(input)?),
            LinearBound::Plane(plane(input)?),
        ),
    };
    Ok(extent.ok())
}

pub fn angular_extent(input: &mut Unstructured) -> Result<Option<AngularExtent>> {
    let extent = match input.int_in_range(0u8..=3)? {
        0 => Ok(AngularExtent::full()),
        1 => AngularExtent::one_side(angle(input)?),
        2 => AngularExtent::symmetric(angle(input)?),
        _ => AngularExtent::new(angle(input)?, angle(input)?),
    };
    Ok(extent.ok())
}

pub fn axis(input: &mut Unstructured) -> Result<Option<Axis2>> {
    Ok(Axis2::through(point(input)?, point(input)?).ok())
}

pub fn sweep(input: &mut Unstructured, regions: &[Region], feature: u64) -> Option<Solid> {
    let plane = plane(input).ok()?;
    if input.arbitrary::<bool>().ok()? {
        let extent = linear_extent(input).ok()??;
        extrude(&plane, regions, extent, feature).ok()
    } else {
        let axis = axis(input).ok()??;
        let extent = angular_extent(input).ok()??;
        revolve(&plane, regions, axis, extent, feature).ok()
    }
}

pub fn swept_solid(input: &mut Unstructured, most_shapes: usize, feature: u64) -> Option<Solid> {
    let curves = profile_curves(input, most_shapes).ok()?;
    let profile = Profile::new(&curves).ok()?;
    let chosen = selection(input, &profile).ok()?;
    let regions = profile.select(&chosen).ok()?;
    sweep(input, &regions, feature)
}

pub fn placement(input: &mut Unstructured) -> Result<RigidTransform> {
    let rotation = match input.int_in_range(0u8..=2)? {
        0 => Rotation3::IDENTITY,
        1 => Rotation3::from_rotation_z(
            std::f64::consts::FRAC_PI_2 * f64::from(input.int_in_range(0..=QUARTER_TURNS - 1)?),
        ),
        _ => Rotation3::from_axis_angle(
            crate::direction(input)?
                .try_normalize()
                .unwrap_or(Vector3::Z),
            angle(input)?,
        ),
    };
    let offset = Vector3::new(coordinate(input)?, coordinate(input)?, coordinate(input)?);
    Ok(RigidTransform::new(rotation, offset).unwrap_or(RigidTransform::IDENTITY))
}
