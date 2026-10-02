pub mod document;
pub mod profile;
pub mod sketch;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{Interrupt, MeshQuality, Solid, interruptible};
use libfuzzer_sys::arbitrary::{Result, Unstructured};

pub const WORK_BUDGET: Duration = Duration::from_secs(3);

const GRID: f64 = 0.5;
const REACH: i16 = 24;
const LONGEST: i16 = 24;
const NUDGES: [f64; 8] = [0.0, 0.0, 0.0, 0.0, 1e-6, -1e-6, 1e-4, -2e-5];
const ANGLE_STEPS: i16 = 24;
const ANGLE_NUDGES: [f64; 4] = [0.0, 0.0, 1e-7, -1e-5];

pub fn interrupt_after(budget: Duration) -> Interrupt {
    let deadline = Instant::now() + budget;
    Arc::new(move || Instant::now() >= deadline)
}

pub fn coordinate(input: &mut Unstructured) -> Result<f64> {
    let cells = input.int_in_range(-REACH..=REACH)?;
    let nudge = *input.choose(&NUDGES)?;
    Ok(f64::from(cells) * GRID + nudge)
}

pub fn point(input: &mut Unstructured) -> Result<Point2> {
    Ok(Point2::new(coordinate(input)?, coordinate(input)?))
}

pub fn length(input: &mut Unstructured) -> Result<f64> {
    let cells = input.int_in_range(1..=LONGEST)?;
    let nudge = *input.choose(&NUDGES)?;
    Ok(f64::from(cells) * GRID + nudge)
}

pub fn angle(input: &mut Unstructured) -> Result<f64> {
    let steps = input.int_in_range(0..=ANGLE_STEPS - 1)?;
    let nudge = *input.choose(&ANGLE_NUDGES)?;
    Ok(std::f64::consts::TAU * f64::from(steps) / f64::from(ANGLE_STEPS) + nudge)
}

pub fn direction(input: &mut Unstructured) -> Result<Vector3> {
    let component = |input: &mut Unstructured| -> Result<f64> {
        Ok(f64::from(input.int_in_range(-2i8..=2)?) + *input.choose(&NUDGES)?)
    };
    Ok(Vector3::new(
        component(input)?,
        component(input)?,
        component(input)?,
    ))
}

pub fn plane(input: &mut Unstructured) -> Result<Plane> {
    Ok(match input.int_in_range(0u8..=3)? {
        0 => Plane::XY,
        1 => Plane::XZ,
        2 => Plane::YZ,
        _ => {
            let origin = Point3::new(coordinate(input)?, coordinate(input)?, coordinate(input)?);
            let (normal, x_axis) = (direction(input)?, direction(input)?);
            Plane::from_frame(origin, normal, x_axis).unwrap_or(Plane::XY)
        }
    })
}

pub fn check_solid(solid: &Solid) {
    let never: Interrupt = Arc::new(|| false);
    if let Err(error) = interruptible(never, || solid.validate()) {
        panic!("an operation returned an invalid solid: {error}");
    }
    if let Ok(mesh) = solid.display_mesh(&MeshQuality::COARSE) {
        let _ = mesh.mass_properties();
    }
}
