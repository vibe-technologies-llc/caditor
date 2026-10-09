use std::collections::BTreeSet;

use caditor_geometry::{Aabb2, Plane, Point2, Vector2};
use caditor_kernel::{
    BooleanError, BooleanOperation, LINEAR_RESOLUTION, LinearExtent, Profile, ProfileCurve,
    ProfileError, ProfileShape, Solid, SweepError, boolean, extrude,
};
use caditor_sketch::Sketch;

use crate::{
    datum::{PlaneReference, Resolver, feature_name},
    document::{Document, Feature, FeatureId},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::{SolidResult, profile_curves},
    trouble,
};

const HALF_SPACE_SIDES: [u64; 4] = [1, 2, 3, 4];
const HALF_SPACE_REACH: f64 = 0.05;
const HALF_SPACE_MARGIN: f64 = 1.0;
const CLOSURE_ENTITIES: [u64; 7] = [
    u64::MAX - 8,
    u64::MAX - 9,
    u64::MAX - 10,
    u64::MAX - 11,
    u64::MAX - 12,
    u64::MAX - 13,
    u64::MAX - 14,
];

#[derive(Debug, Clone, PartialEq)]
pub enum SplitAlong {
    Plane(PlaneReference),
    Body(FeatureId),
    Sketch(FeatureId),
}

impl SplitAlong {
    pub fn plane(&self) -> Option<&PlaneReference> {
        match self {
            Self::Plane(plane) => Some(plane),
            Self::Body(_) | Self::Sketch(_) => None,
        }
    }

    pub fn plane_mut(&mut self) -> Option<&mut PlaneReference> {
        match self {
            Self::Plane(plane) => Some(plane),
            Self::Body(_) | Self::Sketch(_) => None,
        }
    }

    pub fn heap_size(&self) -> usize {
        self.plane().map_or(0, PlaneReference::heap_size)
    }

    pub fn datum(&self) -> Option<FeatureId> {
        self.plane().and_then(PlaneReference::datum)
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Plane(plane) => plane.body(),
            Self::Body(tool) => Some(*tool),
            Self::Sketch(_) => None,
        }
    }

    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::Sketch(sketch) => Some(*sketch),
            Self::Plane(_) | Self::Body(_) => None,
        }
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.plane()
            .map(PlaneReference::origin_features)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    pub body: FeatureId,
    pub along: SplitAlong,
    pub flipped: bool,
}

impl Split {
    pub fn heap_size(&self) -> usize {
        self.along.heap_size()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.along.datum());
        used.extend(self.along.body());
        used.extend(self.along.sketch());
        used
    }
}

struct Words {
    tool: String,
    misses: String,
    aim: String,
    adjust: String,
}

impl Words {
    fn new(document: &Document, along: &SplitAlong, body: &str) -> Self {
        match along {
            SplitAlong::Plane(_) => Self {
                tool: "the plane".to_owned(),
                misses: format!(
                    "The plane does not pass through the body of {body}, so nothing lies on one \
                     of its sides."
                ),
                aim: "Choose a plane that crosses the body.".to_owned(),
                adjust: "Move the plane".to_owned(),
            },
            SplitAlong::Body(tool) => {
                let tool = feature_name(document, *tool);
                Self {
                    misses: format!(
                        "{tool} does not cut through the body of {body}, so the body lies wholly \
                         inside it or wholly outside it."
                    ),
                    aim: format!("Choose a body that passes partly through {body}."),
                    adjust: format!("Move {tool}"),
                    tool,
                }
            }
            SplitAlong::Sketch(sketch) => {
                let sketch = feature_name(document, *sketch);
                Self {
                    tool: format!("the curve of {sketch}"),
                    misses: format!(
                        "The curve of {sketch}, carried on straight past its ends, does not cross \
                         the body of {body}, so nothing lies on one of its sides."
                    ),
                    aim: format!("Draw the curve of {sketch} across the body."),
                    adjust: format!("Move the curve of {sketch}"),
                }
            }
        }
    }
}

struct Context<'a> {
    resolver: Resolver<'a>,
    body_name: String,
    words: Words,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: &str) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy: remedy.to_owned(),
            fix: Some(FixTarget::Feature(self.resolver.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn misses(&self) -> Failure {
        self.error(self.words.misses.clone(), &self.words.aim)
    }

    fn not_split(&self) -> String {
        format!(
            "The body of {} could not be split along {}.",
            self.body_name, self.words.tool
        )
    }

    fn unbuildable(&self, error: &dyn std::fmt::Display) -> Failure {
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        self.error(
            self.not_split(),
            &format!(
                "{} slightly, so it does not run exactly along a face or edge.",
                self.words.adjust
            ),
        )
    }

    fn boolean_failure(&self, solid: &Solid, tool: &Solid, error: BooleanError) -> Failure {
        match error {
            BooleanError::Cancelled(_) => Failure::Cancelled,
            BooleanError::Empty => self.misses(),
            other => {
                let document = self.resolver.inputs.document;
                let trouble =
                    trouble::boolean_trouble(document, [solid, tool], &other, &self.words.adjust);
                log::warn!("{} could not be built: {other}", self.resolver.feature.name);
                self.error(trouble.reason(self.not_split()), &trouble.remedy)
                    .placed(trouble.place)
            }
        }
    }

    fn swept_failure(&self, sketch: FeatureId, error: SweptError) -> Failure {
        let name = feature_name(self.resolver.inputs.document, sketch);
        let (reason, remedy) = match error {
            SweptError::NoCurves => (
                format!("{name} has no curve to split along."),
                format!("Draw an open line, arc or spline across the body in {name}."),
            ),
            SweptError::Closed => (
                format!(
                    "The curves of {name} close on themselves, so they do not run across the \
                     body from one side to the other."
                ),
                format!(
                    "Leave {name} one open chain of curves, or split along a body made from it \
                     instead."
                ),
            ),
            SweptError::NotOneChain => (
                format!("The curves of {name} do not form one chain joined end to end."),
                format!(
                    "Leave one open chain of curves in {name}, or make the others construction \
                     geometry."
                ),
            ),
            SweptError::CrossesItself => (
                format!(
                    "The curve of {name}, carried on straight past its ends, crosses itself, so \
                     it does not divide the body into two sides."
                ),
                format!(
                    "Lengthen the curve of {name} so it runs past the body, or draw it without \
                     crossing itself."
                ),
            ),
            other => return self.unbuildable(&other),
        };
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(sketch)),
            constraints: Vec::new(),
            place: None,
        }))
    }
}

enum Tool<'a> {
    Inside(Solid),
    Outside(&'a Solid),
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Split,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let body_name = inputs
        .document
        .feature(definition.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let words = Words::new(inputs.document, &definition.along, &body_name);
    let context = Context {
        resolver: Resolver { feature, inputs },
        body_name,
        words,
    };
    let plane = definition
        .along
        .plane()
        .map(|reference| context.resolver.plane(reference))
        .transpose()?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(Failure::Error(Box::new(FeatureError {
            reason: format!("The body made by {} has no shape.", context.body_name),
            remedy: format!("Fix {} first.", context.body_name),
            fix: Some(FixTarget::Feature(definition.body)),
            constraints: Vec::new(),
            place: None,
        })));
    };
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let tool = match (&definition.along, plane) {
        (_, Some(plane)) => Tool::Inside(half_space(
            &context,
            solid,
            &plane,
            definition.flipped,
            feature,
        )?),
        (SplitAlong::Body(tool), _) if *tool == definition.body => {
            return Err(context.error(
                format!(
                    "The body of {} cannot be split along itself.",
                    context.body_name
                ),
                "Choose another body to split it along.",
            ));
        }
        (SplitAlong::Body(tool), _) => Tool::Outside(context.resolver.body(*tool)?),
        (SplitAlong::Sketch(sketch), _) => Tool::Inside(swept(
            &context,
            solid,
            *sketch,
            definition.flipped,
            feature,
        )?),
        (SplitAlong::Plane(_), None) => return Err(context.misses()),
    };
    let (tool, keep_inside) = match &tool {
        Tool::Inside(tool) => (tool, true),
        Tool::Outside(tool) => (*tool, definition.flipped),
    };
    let inside = boolean(solid, tool, BooleanOperation::Intersection)
        .map_err(|error| context.boolean_failure(solid, tool, error))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let outside = boolean(solid, tool, BooleanOperation::Difference)
        .map_err(|error| context.boolean_failure(solid, tool, error))?;
    let (kept, apart) = if keep_inside {
        (inside, outside)
    } else {
        (outside, inside)
    };
    Ok(FeatureResult::Solid(
        SolidResult::new(definition.body, kept)
            .with_others([SolidResult::new(feature.id(), apart)]),
    ))
}

fn half_space(
    context: &Context<'_>,
    solid: &Solid,
    plane: &Plane,
    flipped: bool,
    feature: &Feature,
) -> Result<Solid, Failure> {
    half_space_solid(solid, plane, flipped, feature.id().raw()).map_err(|error| match error {
        HalfSpaceError::Misses => context.misses(),
        other => context.unbuildable(&other),
    })
}

fn swept(
    context: &Context<'_>,
    solid: &Solid,
    sketch: FeatureId,
    flipped: bool,
    feature: &Feature,
) -> Result<Solid, Failure> {
    let inputs = context.resolver.inputs;
    let Some(result) = inputs
        .features
        .get(&sketch)
        .and_then(|result| result.sketch())
    else {
        let name = feature_name(inputs.document, sketch);
        return Err(context.resolver.error(
            format!("It uses {name}, which has an error."),
            format!("Fix {name} first."),
            sketch,
        ));
    };
    swept_half_space(solid, &result.geometry, flipped, feature.id().raw())
        .map_err(|error| context.swept_failure(sketch, error))
}

#[derive(Debug, thiserror::Error)]
pub enum HalfSpaceError {
    #[error("the body has no extent")]
    NoExtent,
    #[error("the plane does not pass through the body")]
    Misses,
    #[error("its outline could not be drawn: {0}")]
    Profile(ProfileError),
    #[error("it could not be extruded: {0}")]
    Sweep(SweepError),
}

#[derive(Debug, thiserror::Error)]
pub enum SweptError {
    #[error("the body has no extent")]
    NoExtent,
    #[error("the sketch has no curve")]
    NoCurves,
    #[error("the curves close on themselves")]
    Closed,
    #[error("the curves do not form one chain")]
    NotOneChain,
    #[error("a curve has no direction at its end")]
    Degenerate,
    #[error("the curve crosses itself")]
    CrossesItself,
    #[error("its outline could not be drawn: {0}")]
    Profile(ProfileError),
    #[error("it could not be extruded: {0}")]
    Sweep(SweepError),
}

struct Heights {
    low: f64,
    high: f64,
    margin: f64,
    across: Aabb2,
}

fn heights(solid: &Solid, plane: &Plane) -> Option<Heights> {
    let bounds = solid.bounding_box()?;
    let margin = bounds.diagonal() * HALF_SPACE_REACH + HALF_SPACE_MARGIN;
    let local: Vec<(Point2, f64)> = bounds
        .corners()
        .iter()
        .map(|corner| {
            let offset = *corner - plane.origin();
            (
                Point2::new(offset.dot(plane.x_axis()), offset.dot(plane.y_axis())),
                offset.dot(plane.normal()),
            )
        })
        .collect();
    let across = Aabb2::from_points(local.iter().map(|(point, _)| *point))?.expanded(margin);
    let low = local
        .iter()
        .map(|(_, height)| *height)
        .fold(f64::INFINITY, f64::min);
    let high = local
        .iter()
        .map(|(_, height)| *height)
        .fold(f64::NEG_INFINITY, f64::max);
    Some(Heights {
        low,
        high,
        margin,
        across,
    })
}

pub(crate) fn half_space_solid(
    solid: &Solid,
    plane: &Plane,
    flipped: bool,
    feature: u64,
) -> Result<Solid, HalfSpaceError> {
    let Heights {
        low,
        high,
        margin,
        across,
    } = heights(solid, plane).ok_or(HalfSpaceError::NoExtent)?;
    if high <= LINEAR_RESOLUTION || low >= -LINEAR_RESOLUTION {
        return Err(HalfSpaceError::Misses);
    }
    let corners = rectangle(&across);
    let curves: Vec<ProfileCurve> = HALF_SPACE_SIDES
        .iter()
        .zip(corners.iter().zip(corners.iter().cycle().skip(1)))
        .map(|(side, (start, end))| ProfileCurve::line(*side, *start, *end))
        .collect();
    let profile = Profile::new(&curves).map_err(HalfSpaceError::Profile)?;
    let extent = if flipped {
        LinearExtent::new(low - margin, 0.0)
    } else {
        LinearExtent::new(0.0, high + margin)
    }
    .map_err(HalfSpaceError::Sweep)?;
    extrude(plane, profile.regions(), extent, feature).map_err(HalfSpaceError::Sweep)
}

fn rectangle(bounds: &Aabb2) -> [Point2; 4] {
    let (min, max) = (bounds.min(), bounds.max());
    [
        min,
        Point2::new(max.x, min.y),
        max,
        Point2::new(min.x, max.y),
    ]
}

#[derive(Debug, Clone, Copy)]
struct End {
    point: Point2,
    outward: Vector2,
    order: (u64, bool),
}

struct Chain {
    start: End,
    finish: End,
    bounds: Aabb2,
}

fn chain(curves: &[ProfileCurve]) -> Result<Chain, SweptError> {
    let mut ends = Vec::with_capacity(curves.len() * 2);
    let mut bounds: Option<Aabb2> = None;
    for curve in curves {
        if matches!(curve.shape, ProfileShape::Circle { .. }) {
            return Err(SweptError::Closed);
        }
        let (path, range) = curve.curve().map_err(SweptError::Profile)?;
        let first = path.evaluate(range.start());
        let last = path.evaluate(range.end());
        if first.point.distance(last.point) <= LINEAR_RESOLUTION {
            return Err(SweptError::Closed);
        }
        let box_of = path.bounding_box(range);
        bounds = Some(bounds.map_or(box_of, |bounds| bounds.union(box_of)));
        ends.push(End {
            point: first.point,
            outward: -first.first.try_normalize().ok_or(SweptError::Degenerate)?,
            order: (curve.entity, false),
        });
        ends.push(End {
            point: last.point,
            outward: last.first.try_normalize().ok_or(SweptError::Degenerate)?,
            order: (curve.entity, true),
        });
    }
    let bounds = bounds.ok_or(SweptError::NoCurves)?;
    let partners: Vec<Vec<usize>> = ends
        .iter()
        .enumerate()
        .map(|(index, end)| {
            ends.iter()
                .enumerate()
                .filter(|(other, near)| {
                    *other != index && near.point.distance(end.point) <= LINEAR_RESOLUTION
                })
                .map(|(other, _)| other)
                .collect()
        })
        .collect();
    if partners.iter().any(|joined| joined.len() > 1) {
        return Err(SweptError::NotOneChain);
    }
    let free: Vec<usize> = partners
        .iter()
        .enumerate()
        .filter(|(_, joined)| joined.is_empty())
        .map(|(index, _)| index)
        .collect();
    let [first, second] = free.as_slice() else {
        return Err(if free.is_empty() {
            SweptError::Closed
        } else {
            SweptError::NotOneChain
        });
    };
    if walked(&partners, *first) != Some((curves.len(), *second)) {
        return Err(SweptError::NotOneChain);
    }
    let end = |index: usize| ends.get(index).copied().ok_or(SweptError::NotOneChain);
    let (one, other) = (end(*first)?, end(*second)?);
    let (start, finish) = if one.order <= other.order {
        (one, other)
    } else {
        (other, one)
    };
    Ok(Chain {
        start,
        finish,
        bounds,
    })
}

fn walked(partners: &[Vec<usize>], from: usize) -> Option<(usize, usize)> {
    let mut at = from;
    let mut curves = 0;
    while curves < partners.len() / 2 {
        curves += 1;
        let across = at ^ 1;
        match partners.get(across)?.first() {
            Some(next) => at = *next,
            None => return Some((curves, across)),
        }
    }
    None
}

struct Perimeter {
    corners: [(f64, Point2); 4],
    length: f64,
}

impl Perimeter {
    fn new(bounds: &Aabb2) -> Self {
        let size = bounds.size();
        let [first, second, third, fourth] = rectangle(bounds);
        Self {
            corners: [
                (0.0, first),
                (size.x, second),
                (size.x + size.y, third),
                (2.0 * size.x + size.y, fourth),
            ],
            length: 2.0 * (size.x + size.y),
        }
    }

    fn exit(&self, bounds: &Aabb2, end: &End) -> Option<(f64, Point2)> {
        let (min, max) = (bounds.min(), bounds.max());
        let reach = |from: f64, toward: f64, low: f64, high: f64| {
            if toward > 0.0 {
                (high - from) / toward
            } else if toward < 0.0 {
                (low - from) / toward
            } else {
                f64::INFINITY
            }
        };
        let across_x = reach(end.point.x, end.outward.x, min.x, max.x);
        let across_y = reach(end.point.y, end.outward.y, min.y, max.y);
        let distance = across_x.min(across_y);
        if !distance.is_finite() || distance < 0.0 {
            return None;
        }
        let point = end.point + end.outward * distance;
        let size = bounds.size();
        let place = if across_x <= across_y {
            if end.outward.x > 0.0 {
                (size.x + (point.y - min.y), Point2::new(max.x, point.y))
            } else {
                (
                    2.0 * size.x + size.y + (max.y - point.y),
                    Point2::new(min.x, point.y),
                )
            }
        } else if end.outward.y > 0.0 {
            (
                size.x + size.y + (max.x - point.x),
                Point2::new(point.x, max.y),
            )
        } else {
            (point.x - min.x, Point2::new(point.x, min.y))
        };
        Some(place)
    }

    fn walk(&self, from: (f64, Point2), to: (f64, Point2), forward: bool) -> Vec<Point2> {
        let ahead = |at: f64, of: f64| {
            if forward {
                (at - of).rem_euclid(self.length)
            } else {
                (of - at).rem_euclid(self.length)
            }
        };
        let span = ahead(to.0, from.0);
        let mut passed: Vec<(f64, Point2)> = self
            .corners
            .iter()
            .map(|(at, corner)| (ahead(*at, from.0), *corner))
            .filter(|(along, _)| *along > LINEAR_RESOLUTION && *along < span - LINEAR_RESOLUTION)
            .collect();
        passed.sort_by(|one, other| one.0.total_cmp(&other.0));
        std::iter::once(from.1)
            .chain(passed.into_iter().map(|(_, corner)| corner))
            .chain(std::iter::once(to.1))
            .collect()
    }
}

pub fn is_open_chain(sketch: &Sketch) -> bool {
    chain(&profile_curves(sketch)).is_ok()
}

pub(crate) fn swept_half_space(
    solid: &Solid,
    sketch: &Sketch,
    flipped: bool,
    feature: u64,
) -> Result<Solid, SweptError> {
    let plane = sketch.plane();
    let Heights {
        low,
        high,
        margin,
        across,
    } = heights(solid, &plane).ok_or(SweptError::NoExtent)?;
    let mut curves = profile_curves(sketch);
    let path = chain(&curves)?;
    let bounds = across.union(path.bounds.expanded(margin));
    let perimeter = Perimeter::new(&bounds);
    let start = perimeter
        .exit(&bounds, &path.start)
        .ok_or(SweptError::Degenerate)?;
    let finish = perimeter
        .exit(&bounds, &path.finish)
        .ok_or(SweptError::Degenerate)?;
    let corners = perimeter.walk(finish, start, !flipped);
    let closure = [(path.start.point, start.1), (path.finish.point, finish.1)]
        .into_iter()
        .chain(corners.iter().copied().zip(corners.iter().copied().skip(1)))
        .filter(|(from, to)| from.distance(*to) > LINEAR_RESOLUTION);
    for (entity, (from, to)) in CLOSURE_ENTITIES.iter().zip(closure) {
        curves.push(ProfileCurve::line(*entity, from, to));
    }
    let profile = Profile::new(&curves).map_err(|error| match error {
        ProfileError::NoClosedProfile { .. } => SweptError::CrossesItself,
        other => SweptError::Profile(other),
    })?;
    if profile.regions().len() != 1 {
        return Err(SweptError::CrossesItself);
    }
    let extent = LinearExtent::new(low - margin, high + margin).map_err(SweptError::Sweep)?;
    extrude(&plane, profile.regions(), extent, feature).map_err(SweptError::Sweep)
}
