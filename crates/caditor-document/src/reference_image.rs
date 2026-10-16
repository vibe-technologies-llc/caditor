use std::{collections::BTreeSet, sync::Arc};

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::MAX_SIZE;

use crate::{
    datum::{PlaneReference, Resolver, feature_name},
    document::{Feature, FeatureId},
    failure::Cause,
    picture::Picture,
    recompute::{Failure, FeatureResult, Inputs},
};

pub const MIN_IMAGE_OPACITY_PERCENT: u8 = 5;
pub const IMAGE_OPAQUE_PERCENT: u8 = 100;
pub const DEFAULT_IMAGE_OPACITY_PERCENT: u8 = 60;
const LEAST_SPOT_GAP: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq)]
pub enum ImagePlane {
    Reference(PlaneReference),
    Sketch(FeatureId),
}

impl ImagePlane {
    pub fn reference(&self) -> Option<&PlaneReference> {
        match self {
            Self::Reference(reference) => Some(reference),
            Self::Sketch(_) => None,
        }
    }

    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::Sketch(sketch) => Some(*sketch),
            Self::Reference(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageSpot {
    pub across: f64,
    pub up: f64,
}

impl ImageSpot {
    pub const fn new(across: f64, up: f64) -> Self {
        Self { across, up }
    }

    pub fn settled(self) -> Self {
        let held = |value: f64| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.5
            }
        };
        Self {
            across: held(self.across),
            up: held(self.up),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    pub from: ImageSpot,
    pub to: ImageSpot,
    pub distance: Expression,
}

impl Calibration {
    pub fn spot_gap(&self, aspect: f64) -> f64 {
        let across = self.to.across - self.from.across;
        let up = (self.to.up - self.from.up) * aspect;
        across.hypot(up)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImageSize {
    Width(Expression),
    Calibrated(Calibration),
}

impl ImageSize {
    pub fn expression(&self) -> &Expression {
        match self {
            Self::Width(width) => width,
            Self::Calibrated(calibration) => &calibration.distance,
        }
    }

    pub fn expression_mut(&mut self) -> &mut Expression {
        match self {
            Self::Width(width) => width,
            Self::Calibrated(calibration) => &mut calibration.distance,
        }
    }

    pub fn calibration(&self) -> Option<&Calibration> {
        match self {
            Self::Calibrated(calibration) => Some(calibration),
            Self::Width(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceImage {
    pub picture: Arc<Picture>,
    pub plane: ImagePlane,
    pub at: [Expression; 2],
    pub turn: Expression,
    pub flipped: bool,
    pub size: ImageSize,
    pub opacity: u8,
    pub in_front: bool,
    pub locked: bool,
}

impl ReferenceImage {
    pub fn expressions(&self) -> Vec<&Expression> {
        self.at
            .iter()
            .chain([&self.turn, self.size.expression()])
            .collect()
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        let [along, across] = &mut self.at;
        vec![along, across, &mut self.turn, self.size.expression_mut()]
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .into_iter()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .into_iter()
            .any(|expression| expression.uses(parameter))
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        match &self.plane {
            ImagePlane::Reference(reference) => reference
                .datum()
                .into_iter()
                .chain(reference.body())
                .chain(reference.frame())
                .collect(),
            ImagePlane::Sketch(sketch) => BTreeSet::from([*sketch]),
        }
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.plane
            .reference()
            .map(PlaneReference::origin_features)
            .unwrap_or_default()
    }

    pub fn heap_size(&self) -> usize {
        size_of::<Self>()
            + self.plane.reference().map_or(0, PlaneReference::heap_size)
            + self
                .expressions()
                .into_iter()
                .map(Expression::heap_size)
                .sum::<usize>()
            + self.picture.held_bytes()
    }

    pub fn settle(&mut self) {
        self.opacity = self
            .opacity
            .clamp(MIN_IMAGE_OPACITY_PERCENT, IMAGE_OPAQUE_PERCENT);
        if let ImageSize::Calibrated(calibration) = &mut self.size {
            calibration.from = calibration.from.settled();
            calibration.to = calibration.to.settled();
        }
    }

    pub fn alpha(&self) -> f32 {
        f32::from(
            self.opacity
                .clamp(MIN_IMAGE_OPACITY_PERCENT, IMAGE_OPAQUE_PERCENT),
        ) / f32::from(IMAGE_OPAQUE_PERCENT)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedImage {
    pub centre: Point3,
    pub across: Vector3,
    pub up: Vector3,
    pub width: f64,
    pub height: f64,
    pub flipped: bool,
}

impl PlacedImage {
    pub fn normal(&self) -> Vector3 {
        self.across.cross(self.up)
    }

    pub fn plane(&self) -> Option<Plane> {
        Plane::from_frame(self.centre, self.normal(), self.across)
    }

    pub fn point_at(&self, spot: ImageSpot) -> Point3 {
        let across = if self.flipped {
            1.0 - spot.across
        } else {
            spot.across
        };
        self.centre
            + self.across * ((across - 0.5) * self.width)
            + self.up * ((spot.up - 0.5) * self.height)
    }

    pub fn spot_at(&self, point: Point3) -> ImageSpot {
        let offset = point - self.centre;
        let across = offset.dot(self.across) / self.width + 0.5;
        let up = offset.dot(self.up) / self.height + 0.5;
        ImageSpot::new(if self.flipped { 1.0 - across } else { across }, up)
    }

    pub fn holds(&self, spot: ImageSpot) -> bool {
        (0.0..=1.0).contains(&spot.across) && (0.0..=1.0).contains(&spot.up)
    }

    pub fn corners(&self) -> [Point3; 4] {
        [
            self.point_at(ImageSpot::new(0.0, 0.0)),
            self.point_at(ImageSpot::new(1.0, 0.0)),
            self.point_at(ImageSpot::new(1.0, 1.0)),
            self.point_at(ImageSpot::new(0.0, 1.0)),
        ]
    }
}

pub fn placed_on(
    plane: &Plane,
    at: Point2,
    turn_degrees: f64,
    width: f64,
    aspect: f64,
    flipped: bool,
) -> PlacedImage {
    let (sine, cosine) = turn_degrees.to_radians().sin_cos();
    let across = plane.x_axis() * cosine + plane.y_axis() * sine;
    let up = plane.normal().cross(across);
    PlacedImage {
        centre: plane.to_world(at),
        across,
        up,
        width,
        height: width * aspect,
        flipped,
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &ReferenceImage,
    inputs: &Inputs<'_>,
) -> Result<FeatureResult, Failure> {
    let resolver = Resolver { feature, inputs };
    let plane = match &definition.plane {
        ImagePlane::Reference(reference) => resolver.plane(reference)?,
        ImagePlane::Sketch(sketch) => sketch_plane(&resolver, *sketch)?,
    };
    let [along, across] = &definition.at;
    let at = Point2::new(
        within_reach(
            &resolver,
            resolver.value(along, "position along X", Dimension::LENGTH)?,
            "position along X",
        )?,
        within_reach(
            &resolver,
            resolver.value(across, "position along Y", Dimension::LENGTH)?,
            "position along Y",
        )?,
    );
    let turn = resolver.value(&definition.turn, "angle", Dimension::ANGLE)?;
    if !turn.is_finite() {
        return Err(resolver.error(ImageCause::AngleNotANumber));
    }
    let aspect = definition.picture.size().aspect();
    let width = image_width(&resolver, &definition.size, aspect)?;
    Ok(FeatureResult::Image(placed_on(
        &plane,
        at,
        turn,
        width,
        aspect,
        definition.flipped,
    )))
}

fn image_width(resolver: &Resolver<'_>, size: &ImageSize, aspect: f64) -> Result<f64, Failure> {
    let width = match size {
        ImageSize::Width(width) => {
            let width = resolver.value(width, "width", Dimension::LENGTH)?;
            if !is_above(width, 0.0) {
                return Err(resolver.error(ImageCause::WidthNotPositive));
            }
            width
        }
        ImageSize::Calibrated(calibration) => {
            let distance = resolver.value(
                &calibration.distance,
                "distance between the calibration points",
                Dimension::LENGTH,
            )?;
            if !is_above(distance, 0.0) {
                return Err(resolver.error(ImageCause::CalibrationDistance));
            }
            let gap = calibration.spot_gap(aspect);
            if !is_above(gap, LEAST_SPOT_GAP) {
                return Err(resolver.error(ImageCause::CalibrationPoints));
            }
            distance / gap
        }
    };
    if width * aspect.max(1.0) > MAX_SIZE {
        return Err(resolver.error(ImageCause::TooWide));
    }
    Ok(width)
}

fn is_above(value: f64, least: f64) -> bool {
    value.is_finite() && value > least
}

fn within_reach(resolver: &Resolver<'_>, value: f64, what: &str) -> Result<f64, Failure> {
    if value.is_finite() && value.abs() <= MAX_SIZE {
        return Ok(value);
    }
    Err(resolver.error(ImageCause::TooFar {
        what: what.to_owned(),
    }))
}

fn sketch_plane(resolver: &Resolver<'_>, sketch: FeatureId) -> Result<Plane, Failure> {
    let inputs = resolver.inputs;
    inputs
        .features
        .get(&sketch)
        .and_then(|result| result.sketch())
        .map(|result| result.geometry.plane())
        .ok_or_else(|| {
            let name = feature_name(inputs.document, sketch);
            resolver.error_at(ImageCause::SketchFailed { name }, sketch)
        })
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImageCause {
    AngleNotANumber,
    WidthNotPositive,
    CalibrationDistance,
    CalibrationPoints,
    TooWide,
    TooFar { what: String },
    SketchFailed { name: String },
}

impl ImageCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::AngleNotANumber => "The angle of the image is not a number.".to_owned(),
            Self::WidthNotPositive => "The width of the image must be more than zero.".to_owned(),
            Self::CalibrationDistance => {
                "The distance between the calibration points must be more than zero.".to_owned()
            }
            Self::CalibrationPoints => {
                "The two calibration points are at the same place on the image.".to_owned()
            }
            Self::TooWide => format!(
                "The image would be more than {} m across.",
                MAX_SIZE / 1_000.0
            ),
            Self::TooFar { what } => format!(
                "The {what} of the image is more than {} m.",
                MAX_SIZE / 1_000.0
            ),
            Self::SketchFailed { name } => format!("It lies on {name}, which has an error."),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::AngleNotANumber => "Type an angle, such as 30 deg.".to_owned(),
            Self::WidthNotPositive => "Type a width above zero, such as 100 mm.".to_owned(),
            Self::CalibrationDistance => {
                "Type the real distance between the two points, such as 50 mm.".to_owned()
            }
            Self::CalibrationPoints => {
                "Calibrate again, picking two points far apart on the image.".to_owned()
            }
            Self::TooWide => "Make the image smaller.".to_owned(),
            Self::TooFar { .. } => "Bring the image nearer the plane's origin.".to_owned(),
            Self::SketchFailed { name } => format!("Fix {name} first."),
        }
    }
}

impl From<ImageCause> for Cause {
    fn from(cause: ImageCause) -> Self {
        Self::Image(cause)
    }
}
