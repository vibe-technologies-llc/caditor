use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{
    AngularExtent, Axis2, BooleanOperation, LINEAR_RESOLUTION, LinearExtent, MAX_SIZE, Profile,
    ProfileCurve, Solid, boolean, extrude, revolve,
};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::{
    document::{Feature, FeatureId},
    hole_standard::HoleStandard,
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
    trouble::boolean_trouble,
};

pub const MAX_HOLES: usize = 100;
pub const MAX_HOLE_STEPS: usize = 8;
const PARTS: u64 = 16;
const STEP_SHIFT: u32 = 56;
const MARGIN: f64 = 1.0;
const THROUGH_ALL_REACH: f64 = 0.05;
pub const MAX_CONE_ANGLE: f64 = 179.0;

#[derive(Debug, Clone, PartialEq)]
pub enum HoleDepth {
    Blind(Expression),
    ThroughAll,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum HoleBottom {
    #[default]
    Flat,
    DrillPoint(Expression),
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
    Stepped(Vec<HoleStep>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HoleStep {
    pub diameter: Expression,
    pub depth: Expression,
}

impl HoleStyle {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Plain => Vec::new(),
            Self::Counterbore { diameter, depth } => vec![diameter, depth],
            Self::Countersink { diameter, angle } => vec![diameter, angle],
            Self::Stepped(steps) => steps
                .iter()
                .flat_map(|step| [&step.diameter, &step.depth])
                .collect(),
        }
    }

    fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        match self {
            Self::Plain => Vec::new(),
            Self::Counterbore { diameter, depth } => vec![diameter, depth],
            Self::Countersink { diameter, angle } => vec![diameter, angle],
            Self::Stepped(steps) => steps
                .iter_mut()
                .flat_map(|step| [&mut step.diameter, &mut step.depth])
                .collect(),
        }
    }

    fn heap_size(&self) -> usize {
        match self {
            Self::Stepped(steps) => size_of_val(steps.as_slice()),
            Self::Plain | Self::Counterbore { .. } | Self::Countersink { .. } => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HoleShape {
    Round,
    Slot {
        length: Expression,
        angle: Expression,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HoleSizing {
    #[default]
    Typed,
    Circles,
    CirclesAndHeads,
}

impl HoleSizing {
    pub fn by_circles(self) -> bool {
        matches!(self, Self::Circles | Self::CirclesAndHeads)
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
    pub shape: HoleShape,
    pub standard: Option<HoleStandard>,
    pub sizing: HoleSizing,
    pub bottom: HoleBottom,
}

impl Hole {
    pub fn expressions(&self) -> Vec<&Expression> {
        let mut expressions = vec![&self.diameter];
        if let HoleDepth::Blind(depth) = &self.depth {
            expressions.push(depth);
        }
        expressions.extend(self.style.expressions());
        if let HoleShape::Slot { length, angle } = &self.shape {
            expressions.extend([length, angle]);
        }
        if let HoleBottom::DrillPoint(angle) = &self.bottom {
            expressions.push(angle);
        }
        expressions
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        let mut expressions = vec![&mut self.diameter];
        if let HoleDepth::Blind(depth) = &mut self.depth {
            expressions.push(depth);
        }
        expressions.extend(self.style.expressions_mut());
        if let HoleShape::Slot { length, angle } = &mut self.shape {
            expressions.extend([length, angle]);
        }
        if let HoleBottom::DrillPoint(angle) = &mut self.bottom {
            expressions.push(angle);
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
        self.style.heap_size()
            + self
                .expressions()
                .into_iter()
                .map(Expression::heap_size)
                .sum::<usize>()
    }

    pub(crate) fn is_wall(entity: u64) -> bool {
        entity >> STEP_SHIFT == 0 && entity % PARTS == HolePart::Wall as u64
    }

    pub fn part_name(entity: u64) -> String {
        let step = entity >> STEP_SHIFT;
        let part = HolePart::ALL
            .into_iter()
            .find(|candidate| *candidate as u64 == entity % PARTS);
        match part {
            Some(part) if step > 0 => part.step_name(step + 1),
            Some(part) => part.name().to_owned(),
            None => "face".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HolePart {
    Top = 0,
    Wall = 1,
    Bottom = 2,
    Axis = 3,
    CounterboreWall = 4,
    CounterboreFloor = 5,
    Countersink = 6,
    SlotSide = 7,
    SlotEnd = 8,
    SlotOtherSide = 9,
    SlotOtherEnd = 10,
    CounterboreSide = 11,
    CounterboreEnd = 12,
    CounterboreOtherSide = 13,
    CounterboreOtherEnd = 14,
}

impl HolePart {
    const ALL: [Self; 15] = [
        Self::Top,
        Self::Wall,
        Self::Bottom,
        Self::Axis,
        Self::CounterboreWall,
        Self::CounterboreFloor,
        Self::Countersink,
        Self::SlotSide,
        Self::SlotEnd,
        Self::SlotOtherSide,
        Self::SlotOtherEnd,
        Self::CounterboreSide,
        Self::CounterboreEnd,
        Self::CounterboreOtherSide,
        Self::CounterboreOtherEnd,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Wall => "wall",
            Self::Bottom => "bottom",
            Self::CounterboreWall | Self::CounterboreSide | Self::CounterboreOtherSide => {
                "counterbore wall"
            }
            Self::CounterboreFloor => "counterbore floor",
            Self::Countersink => "countersink",
            Self::SlotSide | Self::SlotOtherSide => "slot side",
            Self::SlotEnd | Self::SlotOtherEnd => "slot end",
            Self::CounterboreEnd | Self::CounterboreOtherEnd => "counterbore end",
            Self::Top | Self::Axis => "face",
        }
    }

    fn step_name(self, step: u64) -> String {
        match self {
            Self::CounterboreWall
            | Self::CounterboreSide
            | Self::CounterboreOtherSide
            | Self::CounterboreEnd
            | Self::CounterboreOtherEnd => format!("step {step} wall"),
            Self::CounterboreFloor => format!("step {step} floor"),
            other => other.name().to_owned(),
        }
    }
}

fn step_entity(base: u64, step: usize, part: HolePart) -> u64 {
    let offset = (step as u64).wrapping_shl(STEP_SHIFT);
    base.wrapping_add(offset).wrapping_add(part as u64)
}

pub fn centres(sketch: &Sketch) -> Vec<(EntityId, Point2)> {
    let circled = sketch.entities().filter_map(|(id, entity)| match entity {
        Entity::Circle { center, .. } if !sketch.is_construction(id) => Some(*center),
        _ => None,
    });
    let points: BTreeSet<EntityId> = sketch.free_points().into_iter().chain(circled).collect();
    points
        .into_iter()
        .filter_map(|id| Some((id, sketch.point(id)?)))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CircleSize {
    pub circle: EntityId,
    pub diameter: f64,
}

pub fn circle_sizes(sketch: &Sketch) -> BTreeMap<EntityId, CircleSize> {
    let mut sizes: BTreeMap<EntityId, CircleSize> = BTreeMap::new();
    for (circle, entity) in sketch.entities() {
        let Entity::Circle { center, radius } = entity else {
            continue;
        };
        if sketch.is_construction(circle) {
            continue;
        }
        let size = CircleSize {
            circle,
            diameter: 2.0 * radius,
        };
        sizes
            .entry(*center)
            .and_modify(|smallest| {
                if size.diameter < smallest.diameter {
                    *smallest = size;
                }
            })
            .or_insert(size);
    }
    sizes
}

struct Sized {
    diameter: f64,
    circle: String,
    heads: bool,
}

struct Values {
    diameter: f64,
    depth: Option<f64>,
    style: StyleValues,
    slot: Option<SlotValues>,
    point_half_angle: Option<f64>,
}

#[derive(Clone, Copy)]
struct SlotValues {
    length: f64,
    angle: f64,
}

#[derive(Clone, Copy)]
struct StepValues {
    diameter: f64,
    floor: f64,
}

enum StyleValues {
    Plain,
    Stepped(Vec<StepValues>),
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

    fn steps(
        &self,
        steps: &[HoleStep],
        diameter: f64,
        depth: Option<f64>,
        head_scale: f64,
        hole: &str,
    ) -> Result<StyleValues, Failure> {
        if steps.is_empty() || steps.len() > MAX_HOLE_STEPS {
            return Err(self.error(
                format!(
                    "A stepped hole has {} steps, but it needs from 1 to {MAX_HOLE_STEPS}.",
                    steps.len()
                ),
                "Add or remove steps.".to_owned(),
            ));
        }
        let mut values: Vec<StepValues> = Vec::with_capacity(steps.len());
        for (index, step) in steps.iter().enumerate() {
            let number = index + 1;
            let wide =
                self.length(&step.diameter, &format!("step {number} diameter"))? * head_scale;
            let deep = self.length(&step.depth, &format!("step {number} depth"))? * head_scale;
            if let Some(above) = values.last()
                && wide >= above.diameter
            {
                return Err(self.error(
                    format!("Step {number} is not narrower than step {index} above it.",),
                    "Enter step diameters that get smaller going into the hole.".to_owned(),
                ));
            }
            if wide <= diameter {
                return Err(self.error(
                    format!("Step {number} is not wider than {hole}."),
                    format!("Enter a step {number} diameter above the hole diameter."),
                ));
            }
            let floor = values.last().map_or(0.0, |above| above.floor) + deep;
            values.push(StepValues {
                diameter: wide,
                floor,
            });
        }
        let reach = values.last().map_or(0.0, |deepest| deepest.floor);
        if depth.is_some_and(|depth| reach >= depth) {
            return Err(self.error(
                "The steps together are as deep as the whole hole.".to_owned(),
                "Make the hole deeper or the steps shallower.".to_owned(),
            ));
        }
        Ok(StyleValues::Stepped(values))
    }

    fn values(&self, definition: &Hole, sized: Option<&Sized>) -> Result<Values, Failure> {
        let typed = self.length(&definition.diameter, "diameter")?;
        let diameter = sized.map_or(typed, |sized| sized.diameter);
        let head_scale = sized
            .filter(|sized| sized.heads)
            .map_or(1.0, |sized| sized.diameter / typed);
        let hole = sized.map_or_else(|| "the hole".to_owned(), |sized| sized.circle.clone());
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
                let wide = self.length(wide, "counterbore diameter")? * head_scale;
                let shallow = self.length(shallow, "counterbore depth")? * head_scale;
                if wide <= diameter {
                    return Err(self.error(
                        format!("The counterbore is not wider than {hole}."),
                        "Enter a counterbore diameter above the hole diameter.".to_owned(),
                    ));
                }
                if depth.is_some_and(|depth| shallow >= depth) {
                    return Err(self.error(
                        "The counterbore is as deep as the whole hole.".to_owned(),
                        "Enter a counterbore depth below the hole depth.".to_owned(),
                    ));
                }
                StyleValues::Stepped(vec![StepValues {
                    diameter: wide,
                    floor: shallow,
                }])
            }
            HoleStyle::Stepped(steps) => self.steps(steps, diameter, depth, head_scale, &hole)?,
            HoleStyle::Countersink {
                diameter: wide,
                angle,
            } => {
                let wide = self.length(wide, "countersink diameter")? * head_scale;
                let angle = self.value(angle, Dimension::ANGLE, "countersink angle")?;
                if wide <= diameter {
                    return Err(self.error(
                        format!("The countersink is not wider than {hole}."),
                        "Enter a countersink diameter above the hole diameter.".to_owned(),
                    ));
                }
                if angle <= 0.0 || angle > MAX_CONE_ANGLE {
                    return Err(self.error(
                        format!(
                            "The countersink angle must be above 0° and at most {MAX_CONE_ANGLE}°."
                        ),
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
        let point_half_angle = match &definition.bottom {
            HoleBottom::Flat => None,
            HoleBottom::DrillPoint(angle) => {
                let angle = self.value(angle, Dimension::ANGLE, "drill point angle")?;
                if angle <= 0.0 || angle > MAX_CONE_ANGLE {
                    return Err(self.error(
                        format!(
                            "The drill point angle must be above 0° and at most {MAX_CONE_ANGLE}°."
                        ),
                        "Enter a drill point angle such as 118 deg.".to_owned(),
                    ));
                }
                depth.map(|_| (angle / 2.0).to_radians())
            }
        };
        let slot = match &definition.shape {
            HoleShape::Round => None,
            HoleShape::Slot { length, angle } => {
                if matches!(style, StyleValues::Countersink { .. }) {
                    return Err(self.error(
                        "A slot can be plain, counterbored or stepped, but not countersunk."
                            .to_owned(),
                        "Choose a plain, counterbored or stepped slot, or a round hole.".to_owned(),
                    ));
                }
                if point_half_angle.is_some() {
                    return Err(self.error(
                        "A slot ends flat, so it cannot have a drill point.".to_owned(),
                        "Turn the drill point off, or choose a round hole.".to_owned(),
                    ));
                }
                Some(SlotValues {
                    length: self.length(length, "slot length")?,
                    angle: self
                        .value(angle, Dimension::ANGLE, "slot angle")?
                        .to_radians(),
                })
            }
        };
        Ok(Values {
            diameter,
            depth,
            style,
            slot,
            point_half_angle,
        })
    }
}

fn outline(values: &Values, depth: f64, base: u64) -> Vec<(Point2, u64)> {
    let radius = values.diameter / 2.0;
    let top = MARGIN;
    let own = |part: HolePart| base.wrapping_add(part as u64);
    let tip = values
        .point_half_angle
        .map_or(0.0, |half_angle| radius / half_angle.tan());
    let bore = [
        (Point2::new(radius, -depth), own(HolePart::Bottom)),
        (Point2::new(0.0, -depth - tip), own(HolePart::Axis)),
    ];
    let mut outline = vec![(Point2::new(0.0, top), own(HolePart::Top))];
    match &values.style {
        StyleValues::Plain => outline.push((Point2::new(radius, top), own(HolePart::Wall))),
        StyleValues::Stepped(steps) => {
            let mut above = top;
            for (index, step) in steps.iter().enumerate() {
                let wide = step.diameter / 2.0;
                outline.extend([
                    (
                        Point2::new(wide, above),
                        step_entity(base, index, HolePart::CounterboreWall),
                    ),
                    (
                        Point2::new(wide, -step.floor),
                        step_entity(base, index, HolePart::CounterboreFloor),
                    ),
                ]);
                above = -step.floor;
            }
            outline.push((Point2::new(radius, above), own(HolePart::Wall)));
        }
        StyleValues::Countersink {
            diameter: wide,
            half_angle,
        } => {
            let slope = half_angle.tan();
            outline.extend([
                (
                    Point2::new(wide / 2.0 + top * slope, top),
                    own(HolePart::Countersink),
                ),
                (
                    Point2::new(radius, -(wide / 2.0 - radius) / slope),
                    own(HolePart::Wall),
                ),
            ]);
        }
    }
    outline.extend(bore);
    outline
}

fn tool(
    context: &Context<'_>,
    frame: &Plane,
    centre: Point3,
    up: Vector3,
    outline: &[(Point2, u64)],
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
        .map(|((start, entity), (end, _))| ProfileCurve::line(*entity, *start, *end))
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

struct SlotCut {
    radius: f64,
    depth: f64,
    parts: [HolePart; 4],
    step: usize,
}

fn slot_tool(
    context: &Context<'_>,
    frame: &Plane,
    centre: Point3,
    up: Vector3,
    slot: SlotValues,
    cut: SlotCut,
    base: u64,
) -> Result<Solid, Failure> {
    let unusable = || {
        context.error(
            "The slot cannot be shaped at this point.".to_owned(),
            "Move the point or change the slot's sizes.".to_owned(),
        )
    };
    let (sin, cos) = slot.angle.sin_cos();
    let along = frame.x_axis() * cos + frame.y_axis() * sin;
    let plane = Plane::from_frame(centre, up, along).ok_or_else(unusable)?;
    let SlotCut {
        radius,
        depth,
        parts,
        step,
    } = cut;
    let half = slot.length / 2.0;
    let [side, end, other_side, other_end] = parts.map(|part| step_entity(base, step, part));
    let first = Point2::new(-half, -radius);
    let second = Point2::new(half, -radius);
    let third = Point2::new(half, radius);
    let fourth = Point2::new(-half, radius);
    let curves = [
        ProfileCurve::line(side, first, second),
        ProfileCurve::arc(end, Point2::new(half, 0.0), second, third),
        ProfileCurve::line(other_side, third, fourth),
        ProfileCurve::arc(other_end, Point2::new(-half, 0.0), fourth, first),
    ];
    let profile = Profile::new(&curves).map_err(|_| unusable())?;
    let extent = LinearExtent::new(MARGIN, -depth).map_err(|_| unusable())?;
    extrude(
        &plane,
        profile.regions(),
        extent,
        context.feature.id().raw(),
    )
    .map_err(|_| unusable())
}

fn drills(
    context: &Context<'_>,
    values: &Values,
    frame: &Plane,
    centre: Point3,
    up: Vector3,
    depth: f64,
    base: u64,
) -> Result<Vec<Solid>, Failure> {
    let Some(slot) = values.slot else {
        let outline = outline(values, depth, base);
        let feature = context.feature.id().raw();
        return Ok(vec![tool(context, frame, centre, up, &outline, feature)?]);
    };
    let narrow = slot_tool(
        context,
        frame,
        centre,
        up,
        slot,
        SlotCut {
            radius: values.diameter / 2.0,
            depth,
            parts: [
                HolePart::SlotSide,
                HolePart::SlotEnd,
                HolePart::SlotOtherSide,
                HolePart::SlotOtherEnd,
            ],
            step: 0,
        },
        base,
    )?;
    let mut tools = vec![narrow];
    if let StyleValues::Stepped(steps) = &values.style {
        for (index, step) in steps.iter().enumerate() {
            tools.push(slot_tool(
                context,
                frame,
                centre,
                up,
                slot,
                SlotCut {
                    radius: step.diameter / 2.0,
                    depth: step.floor,
                    parts: [
                        HolePart::CounterboreSide,
                        HolePart::CounterboreEnd,
                        HolePart::CounterboreOtherSide,
                        HolePart::CounterboreOtherEnd,
                    ],
                    step: index,
                },
                base,
            )?);
        }
    }
    Ok(tools)
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
    let values = context.values(definition, None)?;
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
            format!("{sketch_name} has no points or circles to drill at."),
            "Place points or circles in the sketch where the holes go, or choose another sketch."
                .to_owned(),
            definition.sketch,
        ));
    }
    if centres.len() > MAX_HOLES {
        return Err(context.fix(
            format!(
                "{sketch_name} has {} points and circles, more than the {MAX_HOLES} holes a \
                 feature can drill.",
                centres.len()
            ),
            "Use fewer points, or split them between several holes.".to_owned(),
            definition.sketch,
        ));
    }
    let body_name = context.name(definition.body);
    let mut tools = Vec::new();
    let mut body = inputs
        .body(definition.body)
        .ok_or_else(|| inputs.missing_body(definition.body))?
        .clone();
    let frame = sketch.geometry.plane();
    let up = frame.normal() * if definition.reversed { -1.0 } else { 1.0 };
    let circles = match definition.sizing {
        HoleSizing::Circles | HoleSizing::CirclesAndHeads => circle_sizes(&sketch.geometry),
        HoleSizing::Typed => BTreeMap::new(),
    };
    for (point, position) in centres {
        let own = match circles.get(&point) {
            Some(size) => Some(context.values(
                definition,
                Some(&Sized {
                    diameter: size.diameter,
                    circle: sketch.geometry.entity_label(size.circle),
                    heads: definition.sizing == HoleSizing::CirclesAndHeads,
                }),
            )?),
            None => None,
        };
        let values = own.as_ref().unwrap_or(&values);
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
        let base = point.raw().wrapping_mul(PARTS);
        for drill in drills(&context, values, &frame, centre, up, depth, base)? {
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
            tools.push(drill);
        }
    }
    Ok(FeatureResult::Solid(
        SolidResult::new(definition.body, body).cutting(tools),
    ))
}
