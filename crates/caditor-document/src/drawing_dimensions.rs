use std::{
    collections::BTreeMap,
    f64::consts::{FRAC_PI_2, PI},
};

use caditor_expression::format_decimals;
use caditor_geometry::{Aabb2, Point2, Vector2};
use caditor_kernel::{Curve2, EdgeId, EdgeName, Interval, ViewSource};
use caditor_sketch::annotation::{
    CIRCLE_LEADER_DIRECTION, DEGREE_SIGN, DIAMETER_SIGN, EXTENSION_GAP, EXTENSION_OVERSHOOT,
    LEADER_OVERSHOOT, LEADER_TEXT_GAP, RADIUS_PREFIX, TEXT_LIFT, arrowhead, extension_line,
    readable_angle,
};
use thiserror::Error;

use crate::{
    drawing_sheet::{PlacedView, SheetMark, SheetPlan, SheetText, TextAnchor, text_width},
    drawing_views::ViewLines,
    drawings::{
        Annotation, DetailParent, DimensionDirection, DimensionMeasure, Drawing, DrawingDimension,
        DrawingNote, DrawnView, EdgePoint, EdgeSpot, StandardView,
    },
};

pub const DIMENSION_TEXT_HEIGHT: f64 = 3.5;
pub const NOTE_TEXT_HEIGHT: f64 = 3.5;
const DEGENERATE: f64 = 1e-9;
const PARALLEL_SINE: f64 = 1e-6;
const ROUND_SHARE: f64 = 1e-6;
const FULL_TURN_SLACK: f64 = 1e-6;
const BROKEN_REACH: f64 = 2.0;
const LEADER_SHOULDER: f64 = 1.0;
const UNKNOWN_VALUE: &str = "?";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum AnnotationProblem {
    #[error("the view it is drawn in is no longer on the drawing")]
    ViewGone,
    #[error("the isometric view is not drawn true to size, so nothing is measured in it")]
    Isometric,
    #[error("an edge it refers to is no longer in this view")]
    EdgeGone,
    #[error("two edges in this view now carry the name of an edge it refers to")]
    EdgeAmbiguous,
    #[error("the edge it measures is no longer an arc or a circle seen round in this view")]
    NotRound,
    #[error("an edge it measures is no longer straight")]
    NotStraight,
    #[error("an edge it measures from no longer has an end there")]
    NoEnd,
    #[error("its edges are no longer parallel")]
    NotParallel,
    #[error("its edges are parallel, so they make no angle")]
    NoAngle,
    #[error("what it measures has shrunk to nothing")]
    Degenerate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum DimensionRefusal {
    #[error("both edges must be in the same view")]
    DifferentViews,
    #[error(
        "the isometric view is not drawn true to size, so nothing is measured in it; dimension \
         another view"
    )]
    Isometric,
    #[error("only straight edges, arcs and circles seen round are measured")]
    Unmeasurable,
    #[error("{0}")]
    Problem(#[from] AnnotationProblem),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeenEdge {
    pub curve: Curve2,
    pub range: Interval,
}

impl SeenEdge {
    pub fn middle(&self) -> Point2 {
        self.curve.point(self.range.middle())
    }

    fn is_full_turn(&self) -> bool {
        self.curve
            .period()
            .is_some_and(|period| self.range.length() >= period - FULL_TURN_SLACK)
    }

    pub fn straight(&self) -> Option<(Point2, Point2)> {
        match self.curve {
            Curve2::Line(_) => Some((
                self.curve.point(self.range.start()),
                self.curve.point(self.range.end()),
            )),
            _ => None,
        }
    }

    pub fn round(&self) -> Option<(Point2, f64)> {
        match &self.curve {
            Curve2::Circle(circle) => Some((circle.center(), circle.radius())),
            Curve2::Ellipse(ellipse) => {
                let (major, minor) = (ellipse.major_radius(), ellipse.minor_radius());
                ((major - minor).abs() <= ROUND_SHARE * major)
                    .then(|| (ellipse.center(), 0.5 * (major + minor)))
            }
            _ => None,
        }
    }

    pub fn is_whole_circle(&self) -> bool {
        self.round().is_some() && self.is_full_turn()
    }

    fn spot(&self, spot: EdgeSpot) -> Result<Point2, AnnotationProblem> {
        match spot {
            EdgeSpot::Centre => self
                .round()
                .map(|(centre, _)| centre)
                .ok_or(AnnotationProblem::NotRound),
            EdgeSpot::Start | EdgeSpot::End if self.is_full_turn() => Err(AnnotationProblem::NoEnd),
            EdgeSpot::Start => Ok(self.curve.point(self.range.start())),
            EdgeSpot::End => Ok(self.curve.point(self.range.end())),
        }
    }

    fn nearest_spot(&self, at: Point2) -> EdgeSpot {
        if self.round().is_some() {
            return EdgeSpot::Centre;
        }

        let start = self.curve.point(self.range.start());
        let end = self.curve.point(self.range.end());
        if at.distance(start) <= at.distance(end) {
            EdgeSpot::Start
        } else {
            EdgeSpot::End
        }
    }
}

pub fn seen_edge(lines: &ViewLines, name: EdgeName) -> Result<SeenEdge, AnnotationProblem> {
    let mut found: Option<(usize, EdgeId, &Curve2, f64, f64)> = None;

    for piece in &lines.pieces {
        let ViewSource::Edge {
            solid,
            edge,
            name: piece_name,
            ..
        } = piece.source
        else {
            continue;
        };

        if piece_name != name {
            continue;
        }

        let (start, end) = (piece.range.start(), piece.range.end());
        match &mut found {
            None => found = Some((solid, edge, &piece.curve, start, end)),
            Some((kept_solid, kept_edge, _, low, high))
                if *kept_solid == solid && *kept_edge == edge =>
            {
                *low = low.min(start);
                *high = high.max(end);
            }
            Some(_) => return Err(AnnotationProblem::EdgeAmbiguous),
        }
    }

    let (_, _, curve, low, high) = found.ok_or(AnnotationProblem::EdgeGone)?;

    let range = Interval::new(low, high).ok_or(AnnotationProblem::Degenerate)?;
    Ok(SeenEdge {
        curve: curve.clone(),
        range,
    })
}

pub fn is_measured_view(drawing: &Drawing, view: DrawnView) -> bool {
    match view {
        DrawnView::Standard(standard) => standard != StandardView::Isometric,
        DrawnView::Section(_) => true,
        DrawnView::Detail(letter) => drawing
            .detail(letter)
            .is_none_or(|detail| detail.parent != DetailParent::Standard(StandardView::Isometric)),
    }
}

pub fn measured_lines_of(drawing: &Drawing, view: DrawnView) -> DrawnView {
    match view {
        DrawnView::Detail(letter) => drawing
            .detail(letter)
            .map_or(view, |detail| detail.parent.drawn()),
        DrawnView::Standard(_) | DrawnView::Section(_) => view,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AngleRay {
    pub direction: Vector2,
    pub reach: (f64, f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DimensionGeometry {
    Linear {
        first: Point2,
        second: Point2,
        along: Vector2,
        value: f64,
    },
    Radial {
        centre: Point2,
        radius: f64,
        diameter: bool,
    },
    Angle {
        vertex: Point2,
        first: AngleRay,
        second: AngleRay,
        sweep: f64,
    },
}

impl DimensionGeometry {
    pub fn value(&self) -> f64 {
        match *self {
            Self::Linear { value, .. } => value,
            Self::Radial {
                radius, diameter, ..
            } => {
                if diameter {
                    2.0 * radius
                } else {
                    radius
                }
            }
            Self::Angle { sweep, .. } => sweep.to_degrees(),
        }
    }
}

fn straight_of(traced: &SeenEdge) -> Result<(Point2, Point2), AnnotationProblem> {
    traced.straight().ok_or(AnnotationProblem::NotStraight)
}

fn clamped_on(segment: (Point2, Point2), point: Point2) -> Point2 {
    let along = segment.1 - segment.0;
    let length = along.length_squared();

    if length <= DEGENERATE {
        return segment.0;
    }
    segment.0 + along * ((point - segment.0).dot(along) / length).clamp(0.0, 1.0)
}

fn parallel(
    first: (Point2, Point2),
    second: (Point2, Point2),
    place: Point2,
) -> Result<DimensionGeometry, AnnotationProblem> {
    let along_first = (first.1 - first.0)
        .try_normalize()
        .ok_or(AnnotationProblem::Degenerate)?;
    let along_second = (second.1 - second.0)
        .try_normalize()
        .ok_or(AnnotationProblem::Degenerate)?;

    if along_first.perp_dot(along_second).abs() > PARALLEL_SINE {
        return Err(AnnotationProblem::NotParallel);
    }

    let normal = along_first.perp();
    let value = (second.0 - first.0).dot(normal).abs();

    if value <= DEGENERATE {
        return Err(AnnotationProblem::Degenerate);
    }
    Ok(DimensionGeometry::Linear {
        first: clamped_on(first, place),
        second: clamped_on(second, place),
        along: normal,
        value,
    })
}

fn contains_between(from: Vector2, to: Vector2, toward: Vector2) -> bool {
    let turn = from.perp_dot(to);
    turn.abs() > DEGENERATE
        && from.perp_dot(toward) * turn >= 0.0
        && toward.perp_dot(to) * turn >= 0.0
}

fn angle(
    first: (Point2, Point2),
    second: (Point2, Point2),
    place: Point2,
) -> Result<DimensionGeometry, AnnotationProblem> {
    let along_first = (first.1 - first.0)
        .try_normalize()
        .ok_or(AnnotationProblem::Degenerate)?;
    let along_second = (second.1 - second.0)
        .try_normalize()
        .ok_or(AnnotationProblem::Degenerate)?;

    let turn = along_first.perp_dot(along_second);

    if turn.abs() <= PARALLEL_SINE {
        return Err(AnnotationProblem::NoAngle);
    }

    let vertex = first.0 + along_first * (second.0 - first.0).perp_dot(along_second) / turn;
    let toward = (place - vertex).try_normalize().unwrap_or(along_first);

    let (from, to) = [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)]
        .into_iter()
        .map(|(one, other)| (along_first * one, along_second * other))
        .find(|(from, to)| contains_between(*from, *to, toward))
        .unwrap_or((along_first, along_second));

    let reach = |segment: (Point2, Point2), direction: Vector2| {
        let ends = [segment.0, segment.1].map(|end| (end - vertex).dot(direction));
        (ends[0].min(ends[1]), ends[0].max(ends[1]))
    };
    let first_ray = AngleRay {
        direction: from,
        reach: reach(first, from),
    };
    let second_ray = AngleRay {
        direction: to,
        reach: reach(second, to),
    };

    let (first, second) = if from.perp_dot(to) >= 0.0 {
        (first_ray, second_ray)
    } else {
        (second_ray, first_ray)
    };
    Ok(DimensionGeometry::Angle {
        vertex,
        first,
        second,
        sweep: from.dot(to).clamp(-1.0, 1.0).acos(),
    })
}

pub fn dimension_geometry(
    measure: &DimensionMeasure,
    place: Point2,
    lines: &ViewLines,
) -> Result<DimensionGeometry, AnnotationProblem> {
    match *measure {
        DimensionMeasure::Linear {
            from,
            to,
            direction,
        } => {
            let first = seen_edge(lines, from.edge)?.spot(from.spot)?;
            let second = seen_edge(lines, to.edge)?.spot(to.spot)?;

            let along = direction
                .along(first, second)
                .ok_or(AnnotationProblem::Degenerate)?;
            let value = (second - first).dot(along).abs();

            if value <= DEGENERATE {
                return Err(AnnotationProblem::Degenerate);
            }
            Ok(DimensionGeometry::Linear {
                first,
                second,
                along,
                value,
            })
        }
        DimensionMeasure::Parallel { first, second } => parallel(
            straight_of(&seen_edge(lines, first)?)?,
            straight_of(&seen_edge(lines, second)?)?,
            place,
        ),
        DimensionMeasure::Radius(edge) | DimensionMeasure::Diameter(edge) => {
            let (centre, radius) = seen_edge(lines, edge)?
                .round()
                .ok_or(AnnotationProblem::NotRound)?;
            Ok(DimensionGeometry::Radial {
                centre,
                radius,
                diameter: matches!(measure, DimensionMeasure::Diameter(_)),
            })
        }
        DimensionMeasure::Angle { first, second } => angle(
            straight_of(&seen_edge(lines, first)?)?,
            straight_of(&seen_edge(lines, second)?)?,
            place,
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgePick {
    pub view: DrawnView,
    pub edge: EdgeName,
    pub at: Point2,
}

pub type DrawnLines<'a> = BTreeMap<DrawnView, &'a ViewLines>;

pub fn pick_edge(
    plan: &SheetPlan,
    lines: &DrawnLines<'_>,
    hidden: bool,
    point: Point2,
    reach: f64,
) -> Option<EdgePick> {
    let mut best: Option<(f64, EdgePick)> = None;

    for placed in &plan.views {
        if !placed.extent.expanded(reach).contains(point) {
            continue;
        }

        let Some(view_lines) = lines.get(&placed.view) else {
            continue;
        };
        let scale = placed.scale.max(f64::MIN_POSITIVE);
        let local = (point - placed.origin) / scale;

        for piece in view_lines.drawn_pieces(hidden) {
            let ViewSource::Edge { name, .. } = piece.source else {
                continue;
            };

            let on = piece
                .curve
                .point(piece.curve.closest_parameter(local, piece.range));
            let distance = on.distance(local) * scale;

            if distance <= reach && best.is_none_or(|(nearest, _)| distance < nearest) {
                best = Some((
                    distance,
                    EdgePick {
                        view: placed.view,
                        edge: name,
                        at: on,
                    },
                ));
            }
        }
    }
    best.map(|(_, pick)| pick)
}

pub fn dimension_from_picks(
    drawing: &Drawing,
    lines: &DrawnLines<'_>,
    first: EdgePick,
    second: EdgePick,
    place: Point2,
) -> Result<DimensionMeasure, DimensionRefusal> {
    if first.view != second.view {
        return Err(DimensionRefusal::DifferentViews);
    }
    if !is_measured_view(drawing, first.view) {
        return Err(DimensionRefusal::Isometric);
    }

    let measured = lines
        .get(&measured_lines_of(drawing, first.view))
        .ok_or(AnnotationProblem::EdgeGone)?;
    let one = seen_edge(measured, first.edge)?;
    let other = seen_edge(measured, second.edge)?;

    if first.edge == second.edge {
        return match (one.straight(), one.round()) {
            (Some((start, end)), _) => Ok(DimensionMeasure::Linear {
                from: EdgePoint {
                    edge: first.edge,
                    spot: EdgeSpot::Start,
                },
                to: EdgePoint {
                    edge: first.edge,
                    spot: EdgeSpot::End,
                },
                direction: DimensionDirection::from_place(start, end, place),
            }),
            (None, Some(_)) if one.is_whole_circle() => Ok(DimensionMeasure::Diameter(first.edge)),
            (None, Some(_)) => Ok(DimensionMeasure::Radius(first.edge)),
            (None, None) => Err(DimensionRefusal::Unmeasurable),
        };
    }

    if let (Some(a), Some(b)) = (one.straight(), other.straight()) {
        let measure = match parallel(a, b, place) {
            Ok(_) => DimensionMeasure::Parallel {
                first: first.edge,
                second: second.edge,
            },
            Err(AnnotationProblem::NotParallel) => DimensionMeasure::Angle {
                first: first.edge,
                second: second.edge,
            },
            Err(problem) => return Err(problem.into()),
        };

        return Ok(measure);
    }

    let usable = |traced: &SeenEdge| traced.straight().is_some() || traced.round().is_some();

    if !usable(&one) || !usable(&other) {
        return Err(DimensionRefusal::Unmeasurable);
    }

    let from = EdgePoint {
        edge: first.edge,
        spot: one.nearest_spot(first.at),
    };
    let to = EdgePoint {
        edge: second.edge,
        spot: other.nearest_spot(second.at),
    };
    let start = one.spot(from.spot)?;
    let end = other.spot(to.spot)?;

    if start.distance(end) <= DEGENERATE {
        return Err(AnnotationProblem::Degenerate.into());
    }
    Ok(DimensionMeasure::Linear {
        from,
        to,
        direction: DimensionDirection::from_place(start, end, place),
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SheetMeasure {
    Linear {
        first: Point2,
        second: Point2,
        line: Point2,
        angle: f64,
    },
    Angular {
        vertex: Point2,
        first: Point2,
        second: Point2,
        arc: Point2,
    },
    Radius {
        center: Point2,
        on_curve: Point2,
    },
    Diameter {
        near: Point2,
        far: Point2,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SheetAnnotation {
    pub annotation: Annotation,
    pub marks: Vec<SheetMark>,
    pub text: SheetText,
    pub measure: Option<SheetMeasure>,
    pub problem: Option<AnnotationProblem>,
}

impl SheetAnnotation {
    fn text_frame(&self) -> (Vector2, Vector2, f64) {
        let along = Vector2::from_angle(self.text.angle);
        (
            along,
            along.perp(),
            text_width(&self.text.content, self.text.height),
        )
    }

    pub fn text_corners(&self) -> [Point2; 4] {
        let (along, up, width) = self.text_frame();
        let (half_width, half_height) = (width / 2.0, self.text.height / 2.0);

        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, y)| self.text.at + along * (x * half_width) + up * (y * half_height))
    }

    pub fn distance_to(&self, point: Point2) -> f64 {
        let (along, up, width) = self.text_frame();
        let offset = point - self.text.at;

        let outside = Vector2::new(
            (offset.dot(along).abs() - width / 2.0).max(0.0),
            (offset.dot(up).abs() - self.text.height / 2.0).max(0.0),
        );

        self.marks
            .iter()
            .map(|mark| mark_distance(mark, point))
            .fold(outside.length(), f64::min)
    }

    pub fn bounds(&self) -> Aabb2 {
        let points = self
            .text_corners()
            .into_iter()
            .chain(self.marks.iter().flat_map(mark_points));

        Aabb2::from_points(points).unwrap_or_else(|| Aabb2::from_point(self.text.at))
    }
}

fn segment_distance(from: Point2, to: Point2, point: Point2) -> f64 {
    clamped_on((from, to), point).distance(point)
}

fn arc_point(center: Point2, radius: f64, angle: f64) -> Point2 {
    center + Vector2::from_angle(angle) * radius
}

fn mark_distance(mark: &SheetMark, point: Point2) -> f64 {
    match *mark {
        SheetMark::Line(from, to) => segment_distance(from, to, point),
        SheetMark::Circle { center, radius } => ((point - center).length() - radius).abs(),
        SheetMark::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let toward = point - center;
            let angle = toward.y.atan2(toward.x);
            let inside = (angle - start).rem_euclid(2.0 * PI) <= end - start;

            if inside {
                (toward.length() - radius).abs()
            } else {
                arc_point(center, radius, start)
                    .distance(point)
                    .min(arc_point(center, radius, end).distance(point))
            }
        }
    }
}

fn mark_points(mark: &SheetMark) -> Vec<Point2> {
    match *mark {
        SheetMark::Line(from, to) => vec![from, to],
        SheetMark::Circle { center, radius } => {
            vec![
                center - Vector2::splat(radius),
                center + Vector2::splat(radius),
            ]
        }
        SheetMark::Arc {
            center,
            radius,
            start,
            end,
        } => (0..=8)
            .map(|step| {
                arc_point(
                    center,
                    radius,
                    start + (end - start) * f64::from(step) / 8.0,
                )
            })
            .collect(),
    }
}

fn lines_of(pairs: impl IntoIterator<Item = (Point2, Point2)>) -> Vec<SheetMark> {
    pairs
        .into_iter()
        .map(|(from, to)| SheetMark::Line(from, to))
        .collect()
}

fn dimension_text(at: Point2, angle: f64, content: String) -> SheetText {
    SheetText {
        at,
        height: DIMENSION_TEXT_HEIGHT,
        content,
        anchor: TextAnchor::Middle,
        angle,
    }
}

fn value_text(measure: &DimensionMeasure, value: Option<f64>, decimals: u8) -> String {
    let number = value.map_or_else(
        || UNKNOWN_VALUE.to_owned(),
        |value| format_decimals(value, usize::from(decimals)),
    );
    match measure {
        DimensionMeasure::Linear { .. } | DimensionMeasure::Parallel { .. } => number,
        DimensionMeasure::Radius(_) => format!("{RADIUS_PREFIX}{number}"),
        DimensionMeasure::Diameter(_) => format!("{DIAMETER_SIGN}{number}"),
        DimensionMeasure::Angle { .. } => format!("{number}{DEGREE_SIGN}"),
    }
}

struct Laid {
    marks: Vec<SheetMark>,
    text: SheetText,
    measure: SheetMeasure,
}

fn linear_on_sheet(
    (first, second): (Point2, Point2),
    along: Vector2,
    place: Point2,
    content: String,
    text_offset: f64,
) -> Option<Laid> {
    let height = DIMENSION_TEXT_HEIGHT;
    let normal = along.perp();
    let level = place.dot(normal);

    let foot = |point: Point2| point + normal * (level - point.dot(normal));
    let (start, end) = (foot(first), foot(second));

    let span = (end - start).try_normalize()?;
    let mut marks = Vec::new();

    for (point, on_line) in [(first, start), (second, end)] {
        let reach = level - point.dot(normal);

        if reach.abs() > EXTENSION_GAP * height {
            let outward = if reach >= 0.0 { normal } else { -normal };
            let (from, to) = extension_line(point, on_line, outward, height);

            marks.push(SheetMark::Line(from, to));
        }
    }
    marks.push(SheetMark::Line(start, end));
    marks.extend(lines_of(arrowhead(start, -span, height)));
    marks.extend(lines_of(arrowhead(end, span, height)));

    let angle = readable_angle(span.y.atan2(span.x));
    let text_along = Vector2::from_angle(angle);
    let at =
        (start + end) / 2.0 + text_along * text_offset + text_along.perp() * TEXT_LIFT * height;
    Some(Laid {
        marks,
        text: dimension_text(at, angle, content),
        measure: SheetMeasure::Linear {
            first,
            second,
            line: end,
            angle: span.y.atan2(span.x),
        },
    })
}

fn radial_on_sheet(
    (centre, radius): (Point2, f64),
    diameter: bool,
    place: Point2,
    content: String,
    text_offset: f64,
) -> Laid {
    let height = DIMENSION_TEXT_HEIGHT;
    let toward = (place - centre)
        .try_normalize()
        .unwrap_or(CIRCLE_LEADER_DIRECTION);
    let on_curve = centre + toward * radius;
    let out = (place - centre)
        .length()
        .max(radius + LEADER_OVERSHOOT * height);

    let reach = centre + toward * out;
    let width = text_width(&content, height);

    let at = reach + toward * (LEADER_TEXT_GAP * height + width / 2.0 + text_offset);

    let mut marks = Vec::new();

    let measure = if diameter {
        let far = centre - toward * radius;

        marks.push(SheetMark::Line(far, reach));
        marks.extend(lines_of(arrowhead(far, -toward, height)));
        SheetMeasure::Diameter {
            near: on_curve,
            far,
        }
    } else {
        marks.push(SheetMark::Line(centre, reach));
        SheetMeasure::Radius {
            center: centre,
            on_curve,
        }
    };

    marks.extend(lines_of(arrowhead(on_curve, toward, height)));
    Laid {
        marks,
        text: dimension_text(at, 0.0, content),
        measure,
    }
}

fn angle_on_sheet(
    vertex: Point2,
    (first, second): (AngleRay, AngleRay),
    sweep: f64,
    place: Point2,
    content: String,
    text_offset: f64,
) -> Laid {
    let height = DIMENSION_TEXT_HEIGHT;
    let radius = vertex.distance(place).max(2.0 * height);
    let start = first.direction.y.atan2(first.direction.x);
    let end = start + sweep;

    let middle = start + sweep / 2.0 + text_offset / radius;
    let mut marks = vec![SheetMark::Arc {
        center: vertex,
        radius,
        start,
        end,
    }];

    for ray in [first, second] {
        let (low, high) = ray.reach;
        let at = |distance: f64| vertex + ray.direction * distance;

        if radius > high + EXTENSION_GAP * height {
            marks.push(SheetMark::Line(
                at(high + EXTENSION_GAP * height),
                at(radius + EXTENSION_OVERSHOOT * height),
            ));
        } else if radius < low - EXTENSION_GAP * height {
            marks.push(SheetMark::Line(
                at(radius - EXTENSION_OVERSHOOT * height),
                at(low - EXTENSION_GAP * height),
            ));
        }
    }

    let from = arc_point(vertex, radius, start);
    let to = arc_point(vertex, radius, end);

    marks.extend(lines_of(arrowhead(
        from,
        -Vector2::from_angle(start).perp(),
        height,
    )));
    marks.extend(lines_of(arrowhead(
        to,
        Vector2::from_angle(end).perp(),
        height,
    )));
    Laid {
        marks,
        text: dimension_text(
            arc_point(vertex, radius + TEXT_LIFT * height, middle),
            readable_angle(middle - FRAC_PI_2),
            content,
        ),
        measure: SheetMeasure::Angular {
            vertex,
            first: from,
            second: to,
            arc: arc_point(vertex, radius, start + sweep / 2.0),
        },
    }
}

fn on_sheet_of(placed: &PlacedView, geometry: DimensionGeometry) -> DimensionGeometry {
    let scale = placed.scale;
    match geometry {
        DimensionGeometry::Linear {
            first,
            second,
            along,
            value,
        } => DimensionGeometry::Linear {
            first: placed.on_sheet(first),
            second: placed.on_sheet(second),
            along,
            value,
        },
        DimensionGeometry::Radial {
            centre,
            radius,
            diameter,
        } => DimensionGeometry::Radial {
            centre: placed.on_sheet(centre),
            radius: radius * scale,
            diameter,
        },
        DimensionGeometry::Angle {
            vertex,
            first,
            second,
            sweep,
        } => {
            let scaled = |ray: AngleRay| AngleRay {
                reach: (ray.reach.0 * scale, ray.reach.1 * scale),
                ..ray
            };
            DimensionGeometry::Angle {
                vertex: placed.on_sheet(vertex),
                first: scaled(first),
                second: scaled(second),
                sweep,
            }
        }
    }
}

fn laid_dimension(
    dimension: &DrawingDimension,
    geometry: DimensionGeometry,
    placed: &PlacedView,
) -> Option<Laid> {
    let content = value_text(
        &dimension.measure,
        Some(geometry.value()),
        dimension.decimals,
    );
    let place = placed.on_sheet(dimension.place);
    let offset = dimension.text_offset;

    match on_sheet_of(placed, geometry) {
        DimensionGeometry::Linear {
            first,
            second,
            along,
            ..
        } => linear_on_sheet((first, second), along, place, content, offset),
        DimensionGeometry::Radial {
            centre,
            radius,
            diameter,
        } => Some(radial_on_sheet(
            (centre, radius),
            diameter,
            place,
            content,
            offset,
        )),
        DimensionGeometry::Angle {
            vertex,
            first,
            second,
            sweep,
        } => Some(angle_on_sheet(
            vertex,
            (first, second),
            sweep,
            place,
            content,
            offset,
        )),
    }
}

fn broken_dimension(
    dimension: &DrawingDimension,
    placed: &PlacedView,
    problem: AnnotationProblem,
) -> SheetAnnotation {
    let at = placed.on_sheet(dimension.place);
    let content = value_text(&dimension.measure, None, dimension.decimals);
    let reach = Vector2::new(
        text_width(&content, DIMENSION_TEXT_HEIGHT) / 2.0 + BROKEN_REACH * DIMENSION_TEXT_HEIGHT,
        0.0,
    );
    SheetAnnotation {
        annotation: Annotation::Dimension(dimension.id),
        marks: vec![SheetMark::Line(at - reach, at + reach)],
        text: dimension_text(
            at + Vector2::new(0.0, TEXT_LIFT * DIMENSION_TEXT_HEIGHT),
            0.0,
            content,
        ),
        measure: None,
        problem: Some(problem),
    }
}

pub fn dimension_problem(
    drawing: &Drawing,
    dimension: &DrawingDimension,
    lines: &DrawnLines<'_>,
) -> Option<Result<DimensionGeometry, AnnotationProblem>> {
    if !drawing.has_view(dimension.view) {
        return Some(Err(AnnotationProblem::ViewGone));
    }
    if !is_measured_view(drawing, dimension.view) {
        return Some(Err(AnnotationProblem::Isometric));
    }

    let measured = lines.get(&measured_lines_of(drawing, dimension.view))?;
    Some(dimension_geometry(
        &dimension.measure,
        dimension.place,
        measured,
    ))
}

fn sheet_dimension(
    drawing: &Drawing,
    dimension: &DrawingDimension,
    plan: &SheetPlan,
    lines: &DrawnLines<'_>,
) -> Option<SheetAnnotation> {
    let placed = plan.placed(dimension.view)?;
    let outcome = dimension_problem(drawing, dimension, lines)?;

    let laid = outcome.and_then(|geometry| {
        laid_dimension(dimension, geometry, placed).ok_or(AnnotationProblem::Degenerate)
    });

    Some(match laid {
        Ok(laid) => SheetAnnotation {
            annotation: Annotation::Dimension(dimension.id),
            marks: laid.marks,
            text: laid.text,
            measure: Some(laid.measure),
            problem: None,
        },
        Err(problem) => broken_dimension(dimension, placed, problem),
    })
}

pub fn note_on_sheet(plan: &SheetPlan, at: Point2) -> Point2 {
    let half = plan.view_area.size() / 2.0;
    plan.view_area.center() + at.clamp(-half, half)
}

pub fn note_place(plan: &SheetPlan, on_sheet: Point2) -> Point2 {
    let half = plan.view_area.size() / 2.0;
    (on_sheet - plan.view_area.center()).clamp(-half, half)
}

fn leader_target(
    note: &DrawingNote,
    plan: &SheetPlan,
    lines: &DrawnLines<'_>,
) -> Option<Result<Point2, AnnotationProblem>> {
    let leader = note.leader?;
    let Some(placed) = plan.placed(leader.view) else {
        return Some(Err(AnnotationProblem::ViewGone));
    };
    let view_lines = lines.get(&leader.view)?;
    Some(seen_edge(view_lines, leader.edge).map(|traced| placed.on_sheet(traced.middle())))
}

fn sheet_note(note: &DrawingNote, plan: &SheetPlan, lines: &DrawnLines<'_>) -> SheetAnnotation {
    let height = NOTE_TEXT_HEIGHT;
    let at = note_on_sheet(plan, note.at);

    let text = SheetText {
        at,
        height,
        content: note.text.clone(),
        anchor: TextAnchor::Middle,
        angle: 0.0,
    };
    let mut marks = Vec::new();
    let mut problem = None;

    match leader_target(note, plan, lines) {
        Some(Ok(target)) => {
            let half = text_width(&note.text, height) / 2.0 + LEADER_SHOULDER * height / 2.0;
            let side = if target.x >= at.x { 1.0 } else { -1.0 };

            let shoulder = at + Vector2::new(side * half, 0.0);
            let elbow = shoulder + Vector2::new(side * LEADER_SHOULDER * height, 0.0);

            marks.push(SheetMark::Line(shoulder, elbow));
            if let Some(pointing) = (target - elbow).try_normalize() {
                marks.push(SheetMark::Line(elbow, target));
                marks.extend(lines_of(arrowhead(target, pointing, height)));
            }
        }
        Some(Err(found)) => problem = Some(found),
        None => {}
    }
    SheetAnnotation {
        annotation: Annotation::Note(note.id),
        marks,
        text,
        measure: None,
        problem,
    }
}

impl SheetPlan {
    pub fn annotate(&mut self, drawing: &Drawing, lines: &DrawnLines<'_>) {
        let dimensions = drawing
            .dimensions
            .iter()
            .filter_map(|dimension| sheet_dimension(drawing, dimension, self, lines));
        let notes = drawing
            .notes
            .iter()
            .map(|note| sheet_note(note, self, lines));

        self.annotations = dimensions.chain(notes).collect();
    }

    pub fn annotation(&self, annotation: Annotation) -> Option<&SheetAnnotation> {
        self.annotations
            .iter()
            .find(|laid| laid.annotation == annotation)
    }

    pub fn annotation_at(&self, point: Point2, reach: f64) -> Option<Annotation> {
        self.annotations
            .iter()
            .map(|laid| (laid.distance_to(point), laid.annotation))
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, annotation)| annotation)
    }

    pub fn moved_annotation(&self, drawing: &mut Drawing, annotation: Annotation, by: Vector2) {
        match annotation {
            Annotation::Dimension(id) => {
                let Some(dimension) = drawing.dimension(id).copied() else {
                    return;
                };
                let Some(placed) = self.placed(dimension.view) else {
                    return;
                };

                let moved = dimension.place + by / placed.scale.max(f64::MIN_POSITIVE);

                if let Some(dimension) = drawing.dimension_mut(id) {
                    dimension.place = moved;
                }
            }
            Annotation::Note(id) => {
                let Some(note) = drawing.note(id) else {
                    return;
                };
                let moved = note_place(self, note_on_sheet(self, note.at) + by);

                if let Some(note) = drawing.note_mut(id) {
                    note.at = moved;
                }
            }
        }
    }
}

pub fn view_place(placed: &PlacedView, on_sheet: Point2) -> Point2 {
    (on_sheet - placed.origin) / placed.scale.max(f64::MIN_POSITIVE)
}
