use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{
    AngularExtent, Axis2, BooleanOperation, LINEAR_RESOLUTION, MAX_SIZE, Profile, ProfileCurve,
    Solid, boolean, revolve,
};
use caditor_sketch::{EntityId, Sketch};

use crate::{
    document::{Feature, FeatureId},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
    trouble::boolean_trouble,
};

pub const MAX_HOLES: usize = 100;
const PARTS: u64 = 16;
const MARGIN: f64 = 1.0;
const THROUGH_ALL_REACH: f64 = 0.05;
const MAX_COUNTERSINK_ANGLE: f64 = 179.0;

#[derive(Debug, Clone, PartialEq)]
pub enum HoleDepth {
    Blind(Expression),
    ThroughAll,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HoleStyle {
    Plain,
    Counterbore {
        diameter: Expression,
        depth: Expression,
    },
    Countersink {
        diameter: Expression,
        angle: Expression,
    },
}

impl HoleStyle {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hole {
    pub sketch: FeatureId,
    pub body: FeatureId,
    pub diameter: Expression,
    pub depth: HoleDepth,
    pub style: HoleStyle,
    pub reversed: bool,
}

impl Hole {
    pub fn expressions(&self) -> Vec<&Expression> {
        let mut expressions = vec![&self.diameter];
        if let HoleDepth::Blind(depth) = &self.depth {
            expressions.push(depth);
        }
        match &self.style {
            HoleStyle::Plain => {}
            HoleStyle::Counterbore { diameter, depth } => expressions.extend([diameter, depth]),
            HoleStyle::Countersink { diameter, angle } => expressions.extend([diameter, angle]),
        }
        expressions
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
        BTreeSet::from([self.sketch, self.body])
    }

    pub fn heap_size(&self) -> usize {
        self.expressions()
            .into_iter()
            .map(Expression::heap_size)
            .sum()
    }

    pub fn part_name(&self, part: u64) -> &'static str {
        let part = part % PARTS;
        match (&self.style, part) {
            (HoleStyle::Plain, 1) | (HoleStyle::Counterbore { .. }, 3) => "wall",
            (HoleStyle::Plain, 2) | (HoleStyle::Counterbore { .. }, 4) => "bottom",
            (HoleStyle::Counterbore { .. }, 1) => "counterbore wall",
            (HoleStyle::Counterbore { .. }, 2) => "counterbore floor",
            (HoleStyle::Countersink { .. }, 1) => "countersink",
            (HoleStyle::Countersink { .. }, 2) => "wall",
            (HoleStyle::Countersink { .. }, 3) => "bottom",
            _ => "face",
        }
    }
}

pub fn centres(sketch: &Sketch) -> Vec<(EntityId, Point2)> {
    sketch
        .free_points()
        .into_iter()
        .filter_map(|id| Some((id, sketch.point(id)?)))
        .collect()
}

struct Values {
    diameter: f64,
    depth: Option<f64>,
    style: StyleValues,
}

enum StyleValues {
    Plain,
    Counterbore { diameter: f64, depth: f64 },
    Countersink { diameter: f64, half_angle: f64 },
}

struct Context<'a> {
    feature: &'a Feature,
    inputs: &'a Inputs<'a>,
}

impl Context<'_> {
    fn name(&self, id: FeatureId) -> String {
        self.inputs
            .document
            .feature(id)
            .map(|feature| feature.name.clone())
            .unwrap_or_default()
    }

    fn error(&self, reason: String, remedy: String) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn fix(&self, reason: String, remedy: String, target: FeatureId) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(target)),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn value(
        &self,
        expression: &Expression,
        dimension: Dimension,
        what: &str,
    ) -> Result<f64, Failure> {
        expression
            .evaluate_as(dimension, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the {what}."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } if dimension == Dimension::ANGLE => (
                        format!("Edit the {what} so it gives an angle, such as 90 deg."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    EvalError::WrongKind { .. } => (
                        format!("Edit the {what} so it gives a length, such as 6 mm."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    _ => (
                        format!("Edit the {what} or the parameters it uses."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                };
                Failure::Error(Box::new(FeatureError {
                    reason: format!("The {what} cannot be evaluated: {error}."),
                    remedy,
                    fix: Some(fix),
                    constraints: Vec::new(),
                    place: None,
                }))
            })
    }

    fn length(&self, expression: &Expression, what: &str) -> Result<f64, Failure> {
        let value = self.value(expression, Dimension::LENGTH, what)?;
        if value <= 0.0 {
            return Err(self.error(
                format!("The {what} must be more than zero."),
                format!("Enter a {what} above zero."),
            ));
        }
        if value > MAX_SIZE {
            return Err(self.error(
                format!("The {what} cannot be more than {} m.", MAX_SIZE / 1_000.0),
                format!("Enter a {what} of at most {} m.", MAX_SIZE / 1_000.0),
            ));
        }
        Ok(value)
    }

    fn values(&self, definition: &Hole) -> Result<Values, Failure> {
        let diameter = self.length(&definition.diameter, "diameter")?;
        let depth = match &definition.depth {
            HoleDepth::Blind(depth) => Some(self.length(depth, "depth")?),
            HoleDepth::ThroughAll => None,
        };
        let style = match &definition.style {
            HoleStyle::Plain => StyleValues::Plain,
            HoleStyle::Counterbore {
                diameter: wide,
                depth: shallow,
            } => {
                let wide = self.length(wide, "counterbore diameter")?;
                let shallow = self.length(shallow, "counterbore depth")?;
                if wide <= diameter {
                    return Err(self.error(
                        "The counterbore is not wider than the hole.".to_owned(),
                        "Enter a counterbore diameter above the hole diameter.".to_owned(),
                    ));
                }
                if depth.is_some_and(|depth| shallow >= depth) {
                    return Err(self.error(
                        "The counterbore is as deep as the whole hole.".to_owned(),
                        "Enter a counterbore depth below the hole depth.".to_owned(),
                    ));
                }
                StyleValues::Counterbore {
                    diameter: wide,
                    depth: shallow,
                }
            }
            HoleStyle::Countersink {
                diameter: wide,
                angle,
            } => {
                let wide = self.length(wide, "countersink diameter")?;
                let angle = self.value(angle, Dimension::ANGLE, "countersink angle")?;
                if wide <= diameter {
                    return Err(self.error(
                        "The countersink is not wider than the hole.".to_owned(),
                        "Enter a countersink diameter above the hole diameter.".to_owned(),
                    ));
                }
                if angle <= 0.0 || angle > MAX_COUNTERSINK_ANGLE {
                    return Err(self.error(
                        "The countersink angle must be above 0° and at most 179°.".to_owned(),
                        "Enter a countersink angle such as 90 deg.".to_owned(),
                    ));
                }
                let half_angle = (angle / 2.0).to_radians();
                let cone = (wide - diameter) / 2.0 / half_angle.tan();
                if depth.is_some_and(|depth| cone >= depth) {
                    return Err(self.error(
                        "The countersink is as deep as the whole hole.".to_owned(),
                        "Make the hole deeper, the countersink narrower or its angle wider."
                            .to_owned(),
                    ));
                }
                StyleValues::Countersink {
                    diameter: wide,
                    half_angle,
                }
            }
        };
        Ok(Values {
            diameter,
            depth,
            style,
        })
    }
}

fn outline(values: &Values, depth: f64) -> Vec<Point2> {
    let radius = values.diameter / 2.0;
    let top = MARGIN;
    match values.style {
        StyleValues::Plain => vec![
            Point2::new(0.0, top),
            Point2::new(radius, top),
            Point2::new(radius, -depth),
            Point2::new(0.0, -depth),
        ],
        StyleValues::Counterbore {
            diameter: wide,
            depth: shallow,
        } => vec![
            Point2::new(0.0, top),
            Point2::new(wide / 2.0, top),
            Point2::new(wide / 2.0, -shallow),
            Point2::new(radius, -shallow),
            Point2::new(radius, -depth),
            Point2::new(0.0, -depth),
        ],
        StyleValues::Countersink {
            diameter: wide,
            half_angle,
        } => {
            let slope = half_angle.tan();
            vec![
                Point2::new(0.0, top),
                Point2::new(wide / 2.0 + top * slope, top),
                Point2::new(radius, -(wide / 2.0 - radius) / slope),
                Point2::new(radius, -depth),
                Point2::new(0.0, -depth),
            ]
        }
    }
}

fn tool(
    context: &Context<'_>,
    frame: &Plane,
    centre: Point3,
    up: Vector3,
    outline: &[Point2],
    base: u64,
    feature: u64,
) -> Result<Solid, Failure> {
    let unusable = || {
        context.error(
            "The hole cannot be shaped at this point.".to_owned(),
            "Move the point or change the hole's sizes.".to_owned(),
        )
    };
    let across = frame.x_axis();
    let plane = Plane::from_frame(centre, across.cross(up), across).ok_or_else(unusable)?;
    let curves: Vec<ProfileCurve> = outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .enumerate()
        .map(|(index, (start, end))| ProfileCurve::line(base + index as u64, *start, *end))
        .collect();
    let profile = Profile::new(&curves).map_err(|_| unusable())?;
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).map_err(|_| unusable())?;
    revolve(
        &plane,
        profile.regions(),
        axis,
        AngularExtent::full(),
        feature,
    )
    .map_err(|_| unusable())
}

fn reach(body: &Solid, centre: Point3, down: Vector3) -> Option<f64> {
    let bounds = body.bounding_box()?;
    let farthest = bounds
        .corners()
        .iter()
        .map(|corner| (*corner - centre).dot(down))
        .fold(f64::NEG_INFINITY, f64::max);
    (farthest > LINEAR_RESOLUTION)
        .then(|| farthest + bounds.diagonal() * THROUGH_ALL_REACH + MARGIN)
        .map(|depth| depth.min(MAX_SIZE))
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Hole,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context { feature, inputs };
    let values = context.values(definition)?;
    let sketch_name = context.name(definition.sketch);
    let Some(FeatureResult::Sketch(sketch)) =
        inputs.features.get(&definition.sketch).map(AsRef::as_ref)
    else {
        return Err(context.fix(
            format!("It needs the shape of {sketch_name}, which is not available."),
            format!("Fix {sketch_name} first."),
            definition.sketch,
        ));
    };
    let centres = centres(&sketch.geometry);
    if centres.is_empty() {
        return Err(context.fix(
            format!("{sketch_name} has no points to drill at."),
            "Place points in the sketch where the holes go, or choose another sketch.".to_owned(),
            definition.sketch,
        ));
    }
    if centres.len() > MAX_HOLES {
        return Err(context.fix(
            format!(
                "{sketch_name} has {} points, more than the {MAX_HOLES} holes a feature can drill.",
                centres.len()
            ),
            "Use fewer points, or split them between several holes.".to_owned(),
            definition.sketch,
        ));
    }
    let body_name = context.name(definition.body);
    let mut body = inputs
        .body(definition.body)
        .ok_or_else(|| inputs.missing_body(definition.body))?
        .clone();
    let frame = sketch.geometry.plane();
    let up = frame.normal() * if definition.reversed { -1.0 } else { 1.0 };
    for (point, position) in centres {
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        let centre = frame.to_world(position);
        let depth = match values.depth {
            Some(depth) => depth,
            None => reach(&body, centre, -up).ok_or_else(|| {
                context.error(
                    format!(
                        "No part of the body of {body_name} lies beyond {} in the drilling \
                         direction, so there is nothing to go through.",
                        sketch.geometry.entity_label(point)
                    ),
                    "Turn the hole around, or move the point onto the body.".to_owned(),
                )
            })?,
        };
        let outline = outline(&values, depth);
        let base = point.raw().wrapping_mul(PARTS);
        let drill = tool(
            &context,
            &frame,
            centre,
            up,
            &outline,
            base,
            feature.id().raw(),
        )?;
        let cut = boolean(&body, &drill, BooleanOperation::Difference).map_err(|error| {
            use caditor_kernel::BooleanError;
            match error {
                BooleanError::Cancelled(_) => Failure::Cancelled,
                BooleanError::Empty => context.error(
                    format!(
                        "The hole at {} would remove all of {body_name}.",
                        sketch.geometry.entity_label(point)
                    ),
                    "Make the hole smaller or move the point.".to_owned(),
                ),
                other => {
                    log::warn!("{} could not drill: {other}", feature.name);
                    let trouble = boolean_trouble(
                        inputs.document,
                        [&body, &drill],
                        &other,
                        "Move the point or change the sizes",
                    );
                    context
                        .error(
                            trouble.reason(format!(
                                "The hole at {} could not be cut into {body_name}.",
                                sketch.geometry.entity_label(point)
                            )),
                            trouble.remedy,
                        )
                        .placed(trouble.place)
                }
            }
        })?;
        if cut.faces().count() <= body.faces().count() {
            return Err(context.error(
                format!(
                    "The hole at {} does not reach {body_name}.",
                    sketch.geometry.entity_label(point)
                ),
                "Move the point onto the body, or turn the hole around.".to_owned(),
            ));
        }
        body = cut;
    }
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        body,
    )))
}
