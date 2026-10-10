use caditor_document::{
    Blend, BlendKind, CircularPattern, Datum, DatumPlane, Evaluation, Extrude, ExtrudeEnd,
    ExtrudeExtent, FeatureId, FeatureKind, Hole, HoleDepth, LinearDirection, LinearSpacing,
    MAX_PATTERN_INSTANCES, OffsetFace, Pattern, PatternKind, Primitive, PrimitiveAnchor,
    PrimitiveShape, Shell, SolidFeature, SolidStart, Wall, capitalized, displayed_axis,
    displayed_plane, face_plane, hole_centres,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{FaceId, MAX_TAPER_DEGREES, Solid, WallSide};
use caditor_sketch::Entity;

use crate::{
    blend_panel, bodies, handle_snap, hole_tools,
    model::Model,
    move_manipulator::{ARROW_POINTS, Reach, step_for},
    pattern_panel, pattern_tools, reach_handles, scene, solid_panel,
    turn_handles::{STEP_DEGREES, Swing},
    units::Units,
};

const TAPER_STEP_DEGREES: f64 = 1.0;
const FULL_TURN_DEGREES: f64 = 360.0;
const SMALLEST_OPENING: f64 = 1e-3;
const SMALLEST_RADIUS: f64 = 1e-6;
const OFF_THE_LINE: f64 = 1e-9;
pub const HOLE_DEPTH: &str = "Depth";
pub const HOLE_DIAMETER: &str = "Diameter";
pub const OFFSET_DISTANCE: &str = "Distance";
pub const PLANE_OFFSET: &str = "Offset";
pub const PLANE_ANGLE: &str = "Angle";
pub const SHELL_THICKNESS: &str = "Thickness";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Amount {
    Length,
    Angle,
    Count,
}

impl Amount {
    pub fn dimension(self) -> Dimension {
        match self {
            Self::Length => Dimension::LENGTH,
            Self::Angle => Dimension::ANGLE,
            Self::Count => Dimension::NONE,
        }
    }

    pub fn expression(self, units: Units, value: f64) -> Expression {
        match self {
            Self::Length => units.length.measured(value),
            Self::Angle => units.angle.measured(value),
            Self::Count => Expression::number(value.round()),
        }
    }

    pub fn text(self, units: Units, value: f64) -> String {
        match self {
            Self::Length => units.readout_text(value),
            Self::Angle => units.angle.readout_text(value),
            Self::Count => format!("{}", value.round()),
        }
    }

    fn evaluate(self, model: &Model, feature: FeatureId, expression: &Expression) -> Option<f64> {
        let parameters = model.shown_parameters(feature);
        expression
            .evaluate_as(self.dimension(), &|id| parameters.value(id))
            .ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measured {
    HoleDepth,
    HoleDiameter,
    OffsetDistance,
    PlaneOffset,
    PlaneAngle,
    BlendSize,
    ShellThickness,
    WallThickness,
    Taper,
    EndOffset(Reach),
    PrimitiveSize(usize),
    PatternSpacing(usize),
    PatternCount(usize),
    CircularAngle,
    CircularCount,
}

fn zero() -> Expression {
    Expression::number(0.0)
}

fn extrude_mut(kind: &mut FeatureKind) -> Option<&mut Extrude> {
    match kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => Some(extrude),
        _ => None,
    }
}

fn wall_mut(kind: &mut FeatureKind) -> Option<&mut Wall> {
    match kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude.wall.as_deref_mut(),
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => revolve.wall.as_deref_mut(),
        _ => None,
    }
}

fn end_mut(extent: &mut ExtrudeExtent, reach: Reach) -> Option<&mut ExtrudeEnd> {
    match (extent, reach) {
        (ExtrudeExtent::OneSide { end, .. }, Reach::Only)
        | (ExtrudeExtent::TwoSides { forward: end, .. }, Reach::Forward)
        | (ExtrudeExtent::TwoSides { backward: end, .. }, Reach::Backward) => Some(end),
        _ => None,
    }
}

fn end_offset_mut(end: &mut ExtrudeEnd) -> Option<&mut Option<Box<Expression>>> {
    match end {
        ExtrudeEnd::UpToNext { offset } | ExtrudeEnd::UpToFace { offset, .. } => Some(offset),
        ExtrudeEnd::Distance(_) | ExtrudeEnd::ThroughAll | ExtrudeEnd::UpToSurface { .. } => None,
    }
}

fn pattern_kind_mut(kind: &mut FeatureKind) -> Option<&mut PatternKind> {
    match kind {
        FeatureKind::Pattern(pattern) => Some(&mut pattern.kind),
        _ => None,
    }
}

fn direction_mut(kind: &mut FeatureKind, index: usize) -> Option<&mut LinearDirection> {
    match (pattern_kind_mut(kind)?, index) {
        (PatternKind::Linear { first, .. }, 0) => Some(first),
        (PatternKind::Linear { second, .. }, _) => second.as_mut(),
        _ => None,
    }
}

fn circular_mut(kind: &mut FeatureKind) -> Option<&mut CircularPattern> {
    match pattern_kind_mut(kind)? {
        PatternKind::Circular(circular) => Some(circular),
        _ => None,
    }
}

impl Measured {
    pub fn amount(self) -> Amount {
        match self {
            Self::PlaneAngle | Self::Taper | Self::CircularAngle => Amount::Angle,
            Self::PatternCount(_) | Self::CircularCount => Amount::Count,
            Self::HoleDepth
            | Self::HoleDiameter
            | Self::OffsetDistance
            | Self::PlaneOffset
            | Self::BlendSize
            | Self::ShellThickness
            | Self::WallThickness
            | Self::EndOffset(_)
            | Self::PrimitiveSize(_)
            | Self::PatternSpacing(_) => Amount::Length,
        }
    }

    pub fn caption(self, kind: &FeatureKind) -> Option<String> {
        let fixed = match self {
            Self::HoleDepth => HOLE_DEPTH,
            Self::HoleDiameter => HOLE_DIAMETER,
            Self::OffsetDistance => OFFSET_DISTANCE,
            Self::PlaneOffset => PLANE_OFFSET,
            Self::PlaneAngle => PLANE_ANGLE,
            Self::ShellThickness => SHELL_THICKNESS,
            Self::WallThickness => solid_panel::WALL_THICKNESS,
            Self::Taper => solid_panel::TAPER,
            Self::EndOffset(reach) => solid_panel::end_offset_caption(reach),
            Self::CircularAngle => pattern_panel::TOTAL_ANGLE,
            Self::CircularCount => pattern_panel::FIRST.count,
            Self::BlendSize => blend_panel::size_caption(kind.blend()?),
            Self::PrimitiveSize(index) => {
                let (what, _) = kind.primitive()?.shape.sizes().into_iter().nth(index)?;
                return Some(capitalized(what));
            }
            Self::PatternSpacing(index) | Self::PatternCount(index) => {
                let mut probe = kind.clone();
                let direction = direction_mut(&mut probe, index)?;
                let captions = pattern_panel::direction_captions(index);
                match (self, direction.measured) {
                    (Self::PatternCount(_), _) => captions.count,
                    (_, LinearSpacing::BetweenCopies) => captions.spacing,
                    (_, LinearSpacing::Total) => captions.total,
                }
            }
        };
        Some(fixed.to_owned())
    }

    pub fn words(self, kind: &FeatureKind) -> String {
        let fixed = match self {
            Self::HoleDepth => "Drag to change the hole's depth",
            Self::HoleDiameter => "Drag to change the hole's diameter",
            Self::OffsetDistance => "Drag to change how far the faces move",
            Self::PlaneOffset => "Drag to change the plane's offset",
            Self::PlaneAngle => "Drag to turn the plane about its axis",
            Self::ShellThickness => "Drag to change the shell's thickness",
            Self::WallThickness => "Drag to change the wall's thickness",
            Self::Taper => "Drag to change the extrusion's taper",
            Self::EndOffset(_) => "Drag to change how far the end goes past the face",
            Self::CircularAngle => "Drag to change the pattern's total angle",
            Self::PatternCount(_) | Self::CircularCount => "Drag to add or remove copies",
            Self::PatternSpacing(_) => match self.caption(kind).as_deref() {
                Some(caption) => {
                    return format!("Drag to change the pattern's {}", caption.to_lowercase());
                }
                None => "Drag to change the pattern's spacing",
            },
            Self::BlendSize => match kind.blend() {
                Some(blend) => {
                    return format!(
                        "Drag to change the {}'s {}",
                        blend.kind.noun(),
                        blend.kind.size_name()
                    );
                }
                None => "Drag to change the size",
            },
            Self::PrimitiveSize(index) => match kind
                .primitive()
                .and_then(|primitive| primitive.shape.sizes().into_iter().nth(index))
            {
                Some((what, _)) => return format!("Drag to change the shape's {what}"),
                None => "Drag to change the shape's size",
            },
        };
        fixed.to_owned()
    }

    pub fn slot(self, kind: &mut FeatureKind) -> Option<&mut Expression> {
        match self {
            Self::HoleDepth => match kind {
                FeatureKind::Hole(Hole {
                    depth: HoleDepth::Blind(depth),
                    ..
                }) => Some(depth),
                _ => None,
            },
            Self::HoleDiameter => match kind {
                FeatureKind::Hole(hole) => Some(&mut hole.diameter),
                _ => None,
            },
            Self::OffsetDistance => match kind {
                FeatureKind::OffsetFace(OffsetFace { distance, .. }) => Some(distance),
                _ => None,
            },
            Self::PlaneOffset => match kind {
                FeatureKind::Datum(Datum::Plane(DatumPlane { offset, .. })) => Some(offset),
                _ => None,
            },
            Self::PlaneAngle => match kind {
                FeatureKind::Datum(Datum::Plane(DatumPlane {
                    rotation: Some(rotation),
                    ..
                })) => Some(&mut rotation.angle),
                _ => None,
            },
            Self::BlendSize => match kind {
                FeatureKind::Blend(blend) => Some(&mut blend.size),
                _ => None,
            },
            Self::ShellThickness => match kind {
                FeatureKind::Shell(Shell { thickness, .. }) => Some(thickness),
                _ => None,
            },
            Self::WallThickness => Some(&mut wall_mut(kind)?.thickness),
            Self::Taper => Some(
                extrude_mut(kind)?
                    .taper
                    .get_or_insert_with(|| Box::new(zero()))
                    .as_mut(),
            ),
            Self::EndOffset(reach) => {
                let end = end_mut(&mut extrude_mut(kind)?.extent, reach)?;
                Some(
                    end_offset_mut(end)?
                        .get_or_insert_with(|| Box::new(zero()))
                        .as_mut(),
                )
            }
            Self::PrimitiveSize(index) => match kind {
                FeatureKind::Primitive(primitive) => {
                    primitive.shape.sizes_mut().into_iter().nth(index)
                }
                _ => None,
            },
            Self::PatternSpacing(index) => Some(&mut direction_mut(kind, index)?.spacing),
            Self::PatternCount(index) => Some(&mut direction_mut(kind, index)?.count),
            Self::CircularAngle => Some(&mut circular_mut(kind)?.angle),
            Self::CircularCount => Some(&mut circular_mut(kind)?.count),
        }
    }

    pub fn tidy(self, kind: &mut FeatureKind) {
        match self {
            Self::Taper => {
                if let Some(extrude) = extrude_mut(kind)
                    && extrude.taper.as_deref().is_some_and(solid_panel::is_zero)
                {
                    extrude.taper = None;
                }
            }
            Self::EndOffset(reach) => {
                if let Some(offset) = extrude_mut(kind)
                    .and_then(|extrude| end_mut(&mut extrude.extent, reach))
                    .and_then(end_offset_mut)
                    && offset.as_deref().is_some_and(solid_panel::is_zero)
                {
                    *offset = None;
                }
            }
            Self::HoleDiameter => {
                if let FeatureKind::Hole(hole) = kind {
                    hole.standard = None;
                }
            }
            Self::HoleDepth
            | Self::OffsetDistance
            | Self::PlaneOffset
            | Self::PlaneAngle
            | Self::BlendSize
            | Self::ShellThickness
            | Self::WallThickness
            | Self::PrimitiveSize(_)
            | Self::PatternSpacing(_)
            | Self::PatternCount(_)
            | Self::CircularAngle
            | Self::CircularCount => {}
        }
    }

    pub fn step(self, per_point: f64) -> f64 {
        match self {
            Self::Taper => TAPER_STEP_DEGREES,
            Self::PlaneAngle | Self::CircularAngle => STEP_DEGREES,
            Self::PatternCount(_) | Self::CircularCount => 1.0,
            Self::HoleDepth
            | Self::HoleDiameter
            | Self::OffsetDistance
            | Self::PlaneOffset
            | Self::BlendSize
            | Self::ShellThickness
            | Self::WallThickness
            | Self::EndOffset(_)
            | Self::PrimitiveSize(_)
            | Self::PatternSpacing(_) => step_for(per_point),
        }
    }

    pub fn accepts(self, value: f64, step: f64, kind: &FeatureKind) -> f64 {
        let most_copies = f64::from(MAX_PATTERN_INSTANCES);
        match self {
            Self::HoleDepth
            | Self::HoleDiameter
            | Self::BlendSize
            | Self::ShellThickness
            | Self::WallThickness
            | Self::PatternSpacing(_) => value.max(step),
            Self::PrimitiveSize(index) => {
                let may_vanish = matches!(
                    (kind.primitive().map(|primitive| &primitive.shape), index),
                    (Some(PrimitiveShape::Cone { .. }), 0 | 1)
                );
                value.max(if may_vanish { 0.0 } else { step })
            }
            Self::OffsetDistance if value == 0.0 => step,
            Self::OffsetDistance | Self::PlaneOffset | Self::PlaneAngle | Self::EndOffset(_) => {
                value
            }
            Self::Taper => {
                let steepest = MAX_TAPER_DEGREES - step;
                value.clamp(-steepest, steepest)
            }
            Self::CircularAngle => value.clamp(step, FULL_TURN_DEGREES),
            Self::PatternCount(_) | Self::CircularCount => value.round().clamp(1.0, most_copies),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Track {
    Line { base: Point3, direction: Vector3 },
    Turn(Swing),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mapping {
    Linear { scale: f64, offset: f64 },
    Tangent { arm: f64 },
}

impl Mapping {
    pub fn position(self, value: f64) -> f64 {
        match self {
            Self::Linear { scale, offset } => offset + scale * value,
            Self::Tangent { arm } => arm * value.to_radians().tan(),
        }
    }

    pub fn value(self, position: f64) -> Option<f64> {
        let value = match self {
            Self::Linear { scale, offset } if scale != 0.0 => (position - offset) / scale,
            Self::Tangent { arm } if arm > 0.0 => (position / arm).atan().to_degrees(),
            Self::Linear { .. } | Self::Tangent { .. } => return None,
        };
        value.is_finite().then_some(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gauge {
    pub measured: Measured,
    pub track: Track,
    pub mapping: Mapping,
    pub value: f64,
}

impl Gauge {
    fn line(
        measured: Measured,
        (base, direction): (Point3, Vector3),
        scale: f64,
        value: f64,
    ) -> Self {
        Self {
            measured,
            track: Track::Line { base, direction },
            mapping: Mapping::Linear { scale, offset: 0.0 },
            value,
        }
    }
}

pub struct Builder<'a> {
    pub model: &'a Model,
    pub feature: FeatureId,
    pub per_point: &'a dyn Fn(Point3) -> Option<f64>,
}

impl Builder<'_> {
    pub fn gauges(&self, kind: &FeatureKind) -> Vec<Gauge> {
        match kind {
            FeatureKind::Hole(hole) => [self.hole_depth(hole), self.hole_diameter(hole)]
                .into_iter()
                .flatten()
                .collect(),
            FeatureKind::OffsetFace(offset) => self.offset(offset).into_iter().collect(),
            FeatureKind::Datum(Datum::Plane(plane)) => {
                [self.plane_offset(plane), self.plane_angle(plane)]
                    .into_iter()
                    .flatten()
                    .collect()
            }
            FeatureKind::Blend(blend) => self.blend(blend).into_iter().collect(),
            FeatureKind::Shell(shell) => self.shell(shell).into_iter().collect(),
            FeatureKind::Solid(SolidFeature::Extrude(extrude)) => {
                let wall = extrude
                    .wall
                    .as_deref()
                    .and_then(|wall| self.wall(extrude.sketch, extrude.start.as_ref(), wall));
                let mut gauges: Vec<Gauge> = wall.into_iter().collect();
                gauges.extend(self.taper(extrude));
                gauges.extend(self.end_offsets(extrude));
                gauges
            }
            FeatureKind::Solid(SolidFeature::Revolve(revolve)) => revolve
                .wall
                .as_deref()
                .and_then(|wall| self.wall(revolve.sketch, revolve.start.as_ref(), wall))
                .into_iter()
                .collect(),
            FeatureKind::Primitive(primitive) => self.primitive(primitive),
            FeatureKind::Pattern(pattern) => self.pattern(pattern),
            _ => Vec::new(),
        }
    }

    fn length(&self, expression: &Expression) -> Option<f64> {
        Amount::Length.evaluate(self.model, self.feature, expression)
    }

    fn angle(&self, expression: &Expression) -> Option<f64> {
        Amount::Angle.evaluate(self.model, self.feature, expression)
    }

    fn count(&self, expression: &Expression) -> Option<f64> {
        Amount::Count.evaluate(self.model, self.feature, expression)
    }

    fn hole_top(&self, hole: &Hole) -> Option<(Plane, Point3, Vector3)> {
        let document = self.model.document();
        let sketch = document.feature(hole.sketch)?;
        let displayed = self.model.displayed_sketch(sketch)?;
        let (_, centre) = hole_centres(&displayed).into_iter().next()?;
        let plane = scene::sketch_plane(document, self.model.evaluation(), hole.sketch)?;
        let down = if hole.reversed {
            plane.normal()
        } else {
            -plane.normal()
        };
        Some((plane, plane.to_world(centre), down))
    }

    fn hole_depth(&self, hole: &Hole) -> Option<Gauge> {
        let HoleDepth::Blind(depth) = &hole.depth else {
            return None;
        };
        let value = self.length(depth)?;
        let (_, top, down) = self.hole_top(hole)?;
        Some(Gauge::line(Measured::HoleDepth, (top, down), 1.0, value))
    }

    fn hole_diameter(&self, hole: &Hole) -> Option<Gauge> {
        if hole.sizing.by_circles() {
            return None;
        }
        let value = self.length(&hole.diameter)?;
        let (plane, top, _) = self.hole_top(hole)?;
        Some(Gauge::line(
            Measured::HoleDiameter,
            (top, plane.x_axis()),
            0.5,
            value,
        ))
    }

    fn offset(&self, offset: &OffsetFace) -> Option<Gauge> {
        let value = self.length(&offset.distance)?;
        let before = self.model.evaluation().body_before(self.feature)?;
        let solid = &before.solid()?.solid;
        let face = offset.faces.first()?.resolve(solid).ok()?;
        let plane = face_plane(solid, face)?;
        let middle = hole_tools::middle_of(solid, face, &plane)?;
        Some(Gauge::line(
            Measured::OffsetDistance,
            (plane.to_world(middle), plane.normal()),
            1.0,
            value,
        ))
    }

    fn shown_evaluation(&self) -> &Evaluation {
        self.model
            .draft_evaluation_of(self.feature)
            .unwrap_or_else(|| self.model.evaluation())
    }

    fn shown_plane(&self) -> Option<Plane> {
        self.shown_evaluation()
            .feature(self.feature)?
            .result
            .as_deref()?
            .datum()?
            .plane()
    }

    fn plane_offset(&self, plane: &DatumPlane) -> Option<Gauge> {
        let value = self.length(&plane.offset)?;
        let shown = self.shown_plane()?;
        let base = shown.origin() - shown.normal() * value;
        Some(Gauge::line(
            Measured::PlaneOffset,
            (base, shown.normal()),
            1.0,
            value,
        ))
    }

    fn plane_angle(&self, plane: &DatumPlane) -> Option<Gauge> {
        let rotation = plane.rotation.as_ref()?;
        let value = self.angle(&rotation.angle)?;
        let shown = self.shown_plane()?;
        let axis = displayed_axis(self.shown_evaluation(), self.feature, &rotation.axis)?;
        let along = axis.direction();
        let foot = axis.origin() + along * (shown.origin() - axis.origin()).dot(along);
        let mut radial = shown.normal().cross(along).try_normalize()?;
        if (shown.origin() - foot).dot(radial) < 0.0 {
            radial = -radial;
        }
        let (sin, cos) = value.to_radians().sin_cos();
        let start = radial * cos - along.cross(radial) * sin;
        let radius = ARROW_POINTS * (self.per_point)(foot)?;
        Some(Gauge {
            measured: Measured::PlaneAngle,
            track: Track::Turn(Swing {
                foot,
                radial: start,
                normal: along.cross(start),
                radius,
            }),
            mapping: Mapping::Linear {
                scale: 1.0,
                offset: 0.0,
            },
            value,
        })
    }

    fn blend(&self, blend: &Blend) -> Option<Gauge> {
        let value = self.length(&blend.size)?;
        let source = handle_snap::before(self.model, self.feature)?;
        let solid = &source.solid;
        let id = bodies::find_edge(source, blend.edges.first()?.name())?;
        let edge = solid.edge(id)?;
        let middle = edge.interval().middle();
        let at = edge.curve().point(middle);
        let along = edge.curve().evaluate(middle).first.try_normalize()?;
        let mut sides = edge.coedges().iter().filter_map(|coedge| {
            let sense = solid.coedge(*coedge)?.sense().sign();
            let normal = outward(solid, solid.coedge_face(*coedge)?, at)?;
            Some((normal, along * sense))
        });
        let (first, _) = sides.next()?;
        let (second, second_along) = sides.next()?;
        let opening = first.dot(second).clamp(-1.0, 1.0).acos();
        if opening < SMALLEST_OPENING {
            return None;
        }
        let convex = second.cross(second_along).dot(first) < 0.0;
        let bisector = (first + second).try_normalize()?;
        let direction = if convex { -bisector } else { bisector };
        let half = opening / 2.0;
        let per_size = match blend.kind {
            BlendKind::Fillet => 1.0 / half.cos() - 1.0,
            BlendKind::Chamfer => half.sin(),
        };
        Some(Gauge::line(
            Measured::BlendSize,
            (at, direction),
            per_size,
            value,
        ))
    }

    fn shell(&self, shell: &Shell) -> Option<Gauge> {
        let value = self.length(&shell.thickness)?;
        let source = handle_snap::before(self.model, self.feature)?;
        let solid = &source.solid;
        let face = shell.open.first()?.resolve(solid).ok()?;
        face_plane(solid, face)?;
        let outer = solid.face(face)?.outer_loop()?;
        let first = *solid.face_loop(outer)?.coedges().first()?;
        let coedge = solid.coedge(first)?;
        let edge = solid.edge(coedge.edge())?;
        let middle = edge.interval().middle();
        let at = edge.curve().point(middle);
        let along = edge.curve().evaluate(middle).first.try_normalize()? * coedge.sense().sign();
        let inward = outward(solid, face, at)?.cross(along).try_normalize()?;
        Some(Gauge::line(
            Measured::ShellThickness,
            (at, inward),
            1.0,
            value,
        ))
    }

    fn wall(&self, sketch: FeatureId, start: Option<&SolidStart>, wall: &Wall) -> Option<Gauge> {
        let value = self.length(&wall.thickness)?;
        let document = self.model.document();
        let plane = scene::sketch_plane(document, self.model.evaluation(), sketch)?;
        let lift = reach_handles::start_offset(self.model, self.feature, &plane, start)?;
        let owner = document.feature(sketch)?;
        let displayed = self.model.displayed_sketch(owner)?;
        let centre = plane.to_local(self.model.sketch_bounds(owner)?.center());
        let (at, left) = displayed.entities().find_map(|(id, entity)| {
            if displayed.is_construction(id) {
                return None;
            }
            match entity {
                Entity::Line { .. } => {
                    let (start, end) = displayed.line_endpoints(id)?;
                    let along = (end - start).try_normalize()?;
                    Some((start.lerp(end, 0.5), along.perp()))
                }
                Entity::Circle { .. } => {
                    let (middle, radius) = displayed.circle(id)?;
                    Some((middle + Vector2::X * radius, -Vector2::X))
                }
                _ => None,
            }
        })?;
        let inside = if (centre - at).dot(left) < -OFF_THE_LINE {
            -left
        } else {
            left
        };
        let (toward, scale) = match wall.side {
            WallSide::Inside => (inside, 1.0),
            WallSide::Outside => (-inside, 1.0),
            WallSide::Centred => (-inside, 0.5),
        };
        let direction = plane.x_axis() * toward.x + plane.y_axis() * toward.y;
        let base = plane.to_world(at) + plane.normal() * lift;
        Some(Gauge::line(
            Measured::WallThickness,
            (base, direction),
            scale,
            value,
        ))
    }

    fn extrude_frame(&self, extrude: &Extrude) -> Option<(Plane, Point3, Point2)> {
        let document = self.model.document();
        let plane = scene::sketch_plane(document, self.model.evaluation(), extrude.sketch)?;
        let start =
            reach_handles::start_offset(self.model, self.feature, &plane, extrude.start.as_ref())?;
        let bounds = self
            .model
            .sketch_bounds(document.feature(extrude.sketch)?)?;
        let half = plane.to_local(bounds.max()) - plane.to_local(bounds.center());
        let middle = plane.to_world(plane.to_local(bounds.center())) + plane.normal() * start;
        Some((plane, middle, Point2::new(half.x.abs(), half.y.abs())))
    }

    fn taper(&self, extrude: &Extrude) -> Option<Gauge> {
        if extrude.direction.is_some() {
            return None;
        }
        let value = extrude
            .taper
            .as_deref()
            .map_or(Some(0.0), |taper| self.angle(taper))?;
        let (reach, sign) = match &extrude.extent {
            ExtrudeExtent::OneSide { end, reversed } => {
                (end.distance()?, if *reversed { -1.0 } else { 1.0 })
            }
            ExtrudeExtent::Symmetric { distance } => (distance, 1.0),
            ExtrudeExtent::TwoSides { forward, .. } => (forward.distance()?, 1.0),
        };
        let mut height = self.length(reach)?;
        if matches!(extrude.extent, ExtrudeExtent::Symmetric { .. }) {
            height /= 2.0;
        }
        let (plane, middle, half) = self.extrude_frame(extrude)?;
        let top = middle + plane.normal() * sign * height;
        let edge = top + plane.x_axis() * half.x;
        Some(Gauge {
            measured: Measured::Taper,
            track: Track::Line {
                base: edge,
                direction: -plane.x_axis(),
            },
            mapping: Mapping::Tangent { arm: height },
            value,
        })
    }

    fn end_offsets(&self, extrude: &Extrude) -> Vec<Gauge> {
        let sides: [(Reach, Option<&ExtrudeEnd>, f64); 2] = match &extrude.extent {
            ExtrudeExtent::OneSide { end, reversed } => [
                (Reach::Only, Some(end), if *reversed { -1.0 } else { 1.0 }),
                (Reach::Backward, None, -1.0),
            ],
            ExtrudeExtent::TwoSides { forward, backward } => [
                (Reach::Forward, Some(forward), 1.0),
                (Reach::Backward, Some(backward), -1.0),
            ],
            ExtrudeExtent::Symmetric { .. } => return Vec::new(),
        };
        if extrude.direction.is_some() {
            return Vec::new();
        }
        let Some((plane, middle, _)) = self.extrude_frame(extrude) else {
            return Vec::new();
        };
        let on_sketch = plane.to_world(plane.to_local(middle));
        sides
            .into_iter()
            .filter_map(|(reach, end, sign)| {
                let ExtrudeEnd::UpToFace { target, offset } = end? else {
                    return None;
                };
                let value = offset
                    .as_deref()
                    .map_or(Some(0.0), |offset| self.length(offset))?;
                let level = caditor_document::displayed_start_offset(
                    self.model.evaluation(),
                    self.feature,
                    &plane,
                    target,
                )?;
                let face = on_sketch + plane.normal() * level;
                Some(Gauge::line(
                    Measured::EndOffset(reach),
                    (face, plane.normal() * sign),
                    1.0,
                    value,
                ))
            })
            .collect()
    }

    fn primitive(&self, primitive: &Primitive) -> Vec<Gauge> {
        let Some(placed) = PlacedPrimitive::of(self, primitive) else {
            return Vec::new();
        };
        primitive_arrows(&primitive.shape)
            .iter()
            .filter_map(|arrow| placed.gauge(*arrow))
            .collect()
    }

    fn pattern(&self, pattern: &Pattern) -> Vec<Gauge> {
        let bounds = if pattern.repeated.is_empty() {
            handle_snap::before(self.model, self.feature).and_then(|body| body.bounding_box())
        } else {
            pattern_tools::repeated_bounds(self.model, &pattern.repeated)
        };
        let Some(centre) = bounds.map(|bounds| bounds.center()) else {
            return Vec::new();
        };
        match &pattern.kind {
            PatternKind::Linear { first, second } => [Some(first), second.as_ref()]
                .into_iter()
                .enumerate()
                .filter_map(|(index, direction)| Some((index, direction?)))
                .flat_map(|(index, direction)| self.linear(index, direction, centre))
                .collect(),
            PatternKind::Circular(circular) => self.circular(circular, centre),
            PatternKind::Curve(_) | PatternKind::Points(_) => Vec::new(),
        }
    }

    fn linear(&self, index: usize, direction: &LinearDirection, centre: Point3) -> Vec<Gauge> {
        let found = || {
            let ray = displayed_axis(self.model.evaluation(), self.feature, &direction.axis)?;
            let sign = if direction.reversed { -1.0 } else { 1.0 };
            Some((
                ray.direction() * sign,
                self.count(&direction.count)?.round(),
                self.length(&direction.spacing)?,
            ))
        };
        let Some((along, count, spacing)) = found() else {
            return Vec::new();
        };
        let (each, scale) = match direction.measured {
            LinearSpacing::BetweenCopies => (spacing, count - 1.0),
            LinearSpacing::Total if count > 1.0 => (spacing / (count - 1.0), 1.0),
            LinearSpacing::Total => (spacing, 1.0),
        };
        let spaced = (count >= 2.0).then(|| {
            Gauge::line(
                Measured::PatternSpacing(index),
                (centre, along),
                scale,
                spacing,
            )
        });
        let counted = (each > 0.0)
            .then(|| Gauge::line(Measured::PatternCount(index), (centre, along), each, count));
        spaced.into_iter().chain(counted).collect()
    }

    fn circular(&self, circular: &CircularPattern, centre: Point3) -> Vec<Gauge> {
        let found = || {
            let ray = displayed_axis(self.model.evaluation(), self.feature, &circular.axis)?;
            let sign = if circular.reversed { -1.0 } else { 1.0 };
            let along = ray.direction() * sign;
            let foot = ray.origin() + along * (centre - ray.origin()).dot(along);
            let offset = centre - foot;
            let radius = offset.length();
            (radius > SMALLEST_RADIUS).then_some(())?;
            let radial = offset / radius;
            Some((
                Swing {
                    foot,
                    radial,
                    normal: along.cross(radial),
                    radius,
                },
                self.count(&circular.count)?.round(),
                self.angle(&circular.angle)?,
            ))
        };
        let Some((swing, count, angle)) = found() else {
            return Vec::new();
        };
        let full = (angle - FULL_TURN_DEGREES).abs() < 1e-9;
        let (each, offset) = match (full, count > 1.0) {
            (true, _) => (FULL_TURN_DEGREES / count, -FULL_TURN_DEGREES / count / 2.0),
            (false, true) => (angle / (count - 1.0), 0.0),
            (false, false) => (0.0, 0.0),
        };
        let turned = |measured, mapping, value| Gauge {
            measured,
            track: Track::Turn(swing),
            mapping,
            value,
        };
        let spread = (count >= 2.0).then(|| {
            turned(
                Measured::CircularAngle,
                Mapping::Linear {
                    scale: 1.0,
                    offset: 0.0,
                },
                angle,
            )
        });
        let counted = (each > 0.0).then(|| {
            turned(
                Measured::CircularCount,
                Mapping::Linear {
                    scale: each,
                    offset,
                },
                count,
            )
        });
        spread.into_iter().chain(counted).collect()
    }
}

fn outward(solid: &Solid, face: FaceId, point: Point3) -> Option<Vector3> {
    let face = solid.face(face)?;
    let surface = face.surface();
    let uv = surface.project(point, None);
    Some(surface.normal(uv.x, uv.y)? * face.sense().sign())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Along {
    X,
    Y,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Bottom,
    Middle,
    Top,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reaches {
    Side,
    BottomRim,
    TopRim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrimitiveArrow {
    index: usize,
    along: Along,
    level: Level,
    reaches: Reaches,
}

const fn arrow(index: usize, along: Along, level: Level) -> PrimitiveArrow {
    PrimitiveArrow {
        index,
        along,
        level,
        reaches: Reaches::Side,
    }
}

fn primitive_arrows(shape: &PrimitiveShape) -> &'static [PrimitiveArrow] {
    const BOX: [PrimitiveArrow; 3] = [
        arrow(0, Along::X, Level::Middle),
        arrow(1, Along::Y, Level::Middle),
        arrow(2, Along::Up, Level::Middle),
    ];
    const ROUND: [PrimitiveArrow; 2] = [
        arrow(0, Along::X, Level::Middle),
        arrow(1, Along::Up, Level::Middle),
    ];
    const SPHERE: [PrimitiveArrow; 1] = [arrow(0, Along::X, Level::Middle)];
    const CONE: [PrimitiveArrow; 3] = [
        PrimitiveArrow {
            index: 0,
            along: Along::X,
            level: Level::Bottom,
            reaches: Reaches::BottomRim,
        },
        PrimitiveArrow {
            index: 1,
            along: Along::X,
            level: Level::Top,
            reaches: Reaches::TopRim,
        },
        arrow(2, Along::Up, Level::Middle),
    ];
    const WEDGE: [PrimitiveArrow; 3] = [
        arrow(0, Along::X, Level::Bottom),
        arrow(1, Along::Y, Level::Bottom),
        arrow(2, Along::Up, Level::Middle),
    ];
    const PRISM: [PrimitiveArrow; 2] = [
        arrow(1, Along::X, Level::Middle),
        arrow(2, Along::Up, Level::Middle),
    ];
    match shape {
        PrimitiveShape::Box { .. } => &BOX,
        PrimitiveShape::Cylinder { .. } | PrimitiveShape::Torus { .. } => &ROUND,
        PrimitiveShape::Sphere { .. } => &SPHERE,
        PrimitiveShape::Cone { .. } => &CONE,
        PrimitiveShape::Wedge { .. } => &WEDGE,
        PrimitiveShape::Prism { .. } => &PRISM,
    }
}

fn footprint(shape: &PrimitiveShape, values: &[f64]) -> Option<[f64; 3]> {
    let value = |index: usize| values.get(index).copied();
    Some(match shape {
        PrimitiveShape::Box { .. } | PrimitiveShape::Wedge { .. } => {
            [value(0)?, value(1)?, value(2)?]
        }
        PrimitiveShape::Cylinder { .. } => [value(0)?, value(0)?, value(1)?],
        PrimitiveShape::Sphere { .. } => [value(0)?; 3],
        PrimitiveShape::Torus { .. } => {
            let across = value(0)? + value(1)?;
            [across, across, value(1)?]
        }
        PrimitiveShape::Cone { .. } => {
            let widest = value(0)?.max(value(1)?);
            [widest, widest, value(2)?]
        }
        PrimitiveShape::Prism { .. } => [value(1)?, value(1)?, value(2)?],
    })
}

struct PlacedPrimitive<'a> {
    shape: &'a PrimitiveShape,
    plane: Plane,
    up: Vector3,
    at: Point2,
    anchor: PrimitiveAnchor,
    values: Vec<f64>,
}

impl<'a> PlacedPrimitive<'a> {
    fn of(builder: &Builder<'_>, primitive: &'a Primitive) -> Option<Self> {
        let model = builder.model;
        let plane = displayed_plane(model.evaluation(), builder.feature, &primitive.plane)?;
        let up = if primitive.reversed {
            -plane.normal()
        } else {
            plane.normal()
        };
        let [x, y] = primitive.at.each_ref().map(|at| builder.length(at));
        let sides = matches!(primitive.shape, PrimitiveShape::Prism { .. });
        let values = primitive
            .shape
            .sizes()
            .into_iter()
            .enumerate()
            .map(|(index, (_, size))| match (sides, index) {
                (true, 0) => builder.count(size),
                _ => builder.length(size),
            })
            .collect::<Option<Vec<f64>>>()?;
        Some(Self {
            shape: &primitive.shape,
            plane,
            up,
            at: Point2::new(x?, y?),
            anchor: primitive.anchor,
            values,
        })
    }

    fn low(&self, size: [f64; 3]) -> [f64; 3] {
        let [length, width, height] = size;
        let (shift, rise) = match self.anchor {
            PrimitiveAnchor::Corner => ([0.0, 0.0], 0.0),
            PrimitiveAnchor::BaseCentre => ([length / 2.0, width / 2.0], 0.0),
            PrimitiveAnchor::Centre => ([length / 2.0, width / 2.0], height / 2.0),
        };
        let [dx, dy] = shift;
        [self.at.x - dx, self.at.y - dy, -rise]
    }

    fn reach(&self, arrow: PrimitiveArrow, value: f64) -> Option<f64> {
        let mut values = self.values.clone();
        *values.get_mut(arrow.index)? = value;
        let size = footprint(self.shape, &values)?;
        let [low_x, low_y, low_z] = self.low(size);
        let [length, width, height] = size;
        let rim = |diameter: f64| low_x + length / 2.0 + diameter / 2.0;
        Some(match (arrow.along, arrow.reaches) {
            (Along::X, Reaches::BottomRim) => rim(*values.first()?),
            (Along::X, Reaches::TopRim) => rim(*values.get(1)?),
            (Along::X, Reaches::Side) => low_x + length,
            (Along::Y, _) => low_y + width,
            (Along::Up, _) => low_z + height,
        })
    }

    fn gauge(&self, arrow: PrimitiveArrow) -> Option<Gauge> {
        let value = *self.values.get(arrow.index)?;
        let size = footprint(self.shape, &self.values)?;
        let [low_x, low_y, low_z] = self.low(size);
        let [length, width, height] = size;
        let level = match arrow.level {
            Level::Bottom => low_z,
            Level::Middle => low_z + height / 2.0,
            Level::Top => low_z + height,
        };
        let middle = Point2::new(low_x + length / 2.0, low_y + width / 2.0);
        let (base, direction) = match arrow.along {
            Along::X => (Point2::new(0.0, middle.y), self.plane.x_axis()),
            Along::Y => (Point2::new(middle.x, 0.0), self.plane.y_axis()),
            Along::Up => (middle, self.up),
        };
        let lift = match arrow.along {
            Along::Up => 0.0,
            Along::X | Along::Y => level,
        };
        let world = self.plane.to_world(base) + self.up * lift;
        let here = self.reach(arrow, value)?;
        let slope = self.reach(arrow, value + 1.0)? - here;
        Some(Gauge {
            measured: Measured::PrimitiveSize(arrow.index),
            track: Track::Line {
                base: world,
                direction,
            },
            mapping: Mapping::Linear {
                scale: slope,
                offset: here - slope * value,
            },
            value,
        })
    }
}
