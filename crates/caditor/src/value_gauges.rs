use std::f64::consts::{PI, TAU};

use caditor_document::{
    Blend, BlendKind, ChamferForm, CircularPattern, Coil, CoilLength, CurvePattern, CurveSpacing,
    Datum, DatumPlane, Draft, Emboss, EmbossForm, Evaluation, Extrude, ExtrudeEnd, ExtrudeExtent,
    FeatureId, FeatureKind, Helix, Hole, HoleDepth, HoleShape, HoleSizing, HoleStep, HoleStyle,
    LinearDirection, LinearSpacing, MAX_CONE_ANGLE, MAX_PATTERN_INSTANCES, MAX_PRISM_SIDES,
    MIN_PRISM_SIDES, MoveFace, OffsetFace, Pattern, PatternKind, PatternPath, Pipe, Primitive,
    PrimitiveAnchor, PrimitiveShape, ReachedEnd, ReachedSide, Resolution, Rib, RibSide, Shell,
    ShellSide, SizeRule, SketchRegion, SolidFeature, SolidStart, Wall, capitalized, circle_sizes,
    coil_size_name, displayed_axis, displayed_plane, face_plane, flat_normal, hole_centres,
    hole_thread, pipe_size_name, pipe_stand, rib_stand,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{
    BendMiddles, CoilPlacement, EdgeId, FaceId, HelixSpec, MAX_TAPER_DEGREES, PipeCorner,
    PipeShape, Region, STEEPEST_DRAFT, Solid, SpiralSpec, Surface, WallSide, measured_face,
    wrapped_stand,
};
use caditor_sketch::Entity;

use crate::{
    blend_panel, bodies, coil_panel,
    curve_track::{CurveTrack, TrackPoint},
    datum_tools, draft_panel, emboss_panel,
    feature_fields::Rule,
    handle_snap, helix_panel, hole_panel, hole_tools,
    model::Model,
    move_face_panel,
    move_manipulator::{ARROW_POINTS, Reach, step_for},
    pattern_panel, pattern_tools, pipe_panel, reach_handles,
    reversing::Flip,
    rib_panel, scene, selection, shell_panel, solid_panel,
    turn_handles::{STEP_DEGREES, Swing},
    units::Units,
};

const TAPER_STEP_DEGREES: f64 = 1.0;
const FULL_TURN_DEGREES: f64 = 360.0;
const HALF_TURN_DEGREES: f64 = 180.0;
const SMALLEST_OPENING: f64 = 1e-3;
const SMALLEST_RADIUS: f64 = 1e-6;
const OFF_THE_LINE: f64 = 1e-9;
const CURVE_SAMPLES: u32 = 64;
const SECTION_SAMPLES: u32 = 64;
const FOOT_ITERATIONS: usize = 64;
const FOOT_FIT: f64 = 1e-10;
const TRIANGLE_WALL_SHARE: f64 = 2.0 / 1.732_050_807_568_877_2;
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
            Self::Angle => units.angle_readout(value),
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
    CounterboreDiameter,
    CounterboreDepth,
    CountersinkDiameter,
    CountersinkAngle,
    StepDiameter(usize),
    StepDepth(usize),
    SlotLength,
    SlotAngle,
    ThreadDepth,
    HoleEndOffset,
    OffsetDistance,
    MoveDistance,
    MoveAngle,
    DraftAngle,
    PlaneOffset,
    PlaneAngle,
    BlendSize,
    BlendSecond,
    BlendAngle,
    ShellThickness,
    ShellFaceThickness(usize),
    WallThickness,
    PipeSize,
    PipeWall,
    PipeBend,
    Taper,
    EndOffset(Reach),
    PrimitiveSize(usize),
    PrismSides,
    PatternSpacing(usize),
    CircularAngle,
    CircularCount,
    CurveSpacing,
    CurveCount,
    CoilDiameter,
    CoilSection,
    CoilPitch,
    CoilHeight,
    RibThickness,
    EmbossDepth,
}

fn reached_side(reach: Reach) -> ReachedSide {
    match reach {
        Reach::Only | Reach::Symmetric => ReachedSide::Only,
        Reach::Forward => ReachedSide::Forward,
        Reach::Backward => ReachedSide::Backward,
    }
}

fn zero() -> Expression {
    Expression::number(0.0)
}

fn pipe_mut(kind: &mut FeatureKind) -> Option<&mut Pipe> {
    match kind {
        FeatureKind::Pipe(pipe) => Some(pipe),
        _ => None,
    }
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

fn hole_offset_mut(depth: &mut HoleDepth) -> Option<&mut Option<Box<Expression>>> {
    match depth {
        HoleDepth::UpToNext { offset } | HoleDepth::UpToFace { offset, .. } => Some(offset),
        HoleDepth::Blind(_) | HoleDepth::ThroughAll => None,
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

fn curve_mut(kind: &mut FeatureKind) -> Option<&mut CurvePattern> {
    match pattern_kind_mut(kind)? {
        PatternKind::Curve(curve) => Some(curve),
        _ => None,
    }
}

fn coil_mut(kind: &mut FeatureKind) -> Option<&mut Coil> {
    match kind {
        FeatureKind::Coil(coil) => Some(coil),
        _ => None,
    }
}

fn winding_mut(kind: &mut FeatureKind) -> Option<(&mut Expression, &mut CoilLength)> {
    match kind {
        FeatureKind::Coil(coil) => Some((&mut coil.diameter, &mut coil.length)),
        FeatureKind::Helix(helix) => Some((&mut helix.diameter, &mut helix.length)),
        _ => None,
    }
}

fn winding_noun(kind: &FeatureKind) -> &'static str {
    match kind {
        FeatureKind::Helix(helix) if helix.is_flat() => "spiral",
        FeatureKind::Helix(_) => "helix",
        _ => "coil",
    }
}

fn is_flat_spiral(kind: &FeatureKind) -> bool {
    kind.helix().is_some_and(Helix::is_flat)
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
            Self::PlaneAngle
            | Self::Taper
            | Self::CircularAngle
            | Self::BlendAngle
            | Self::CountersinkAngle
            | Self::MoveAngle
            | Self::DraftAngle
            | Self::SlotAngle => Amount::Angle,
            Self::CircularCount | Self::PrismSides | Self::CurveCount => Amount::Count,
            Self::HoleDepth
            | Self::HoleDiameter
            | Self::CounterboreDiameter
            | Self::CounterboreDepth
            | Self::CountersinkDiameter
            | Self::StepDiameter(_)
            | Self::StepDepth(_)
            | Self::SlotLength
            | Self::ThreadDepth
            | Self::HoleEndOffset
            | Self::OffsetDistance
            | Self::MoveDistance
            | Self::PlaneOffset
            | Self::BlendSize
            | Self::BlendSecond
            | Self::ShellThickness
            | Self::ShellFaceThickness(_)
            | Self::WallThickness
            | Self::PipeSize
            | Self::PipeWall
            | Self::PipeBend
            | Self::EndOffset(_)
            | Self::PrimitiveSize(_)
            | Self::PatternSpacing(_)
            | Self::CurveSpacing
            | Self::CoilDiameter
            | Self::CoilSection
            | Self::CoilPitch
            | Self::CoilHeight
            | Self::RibThickness
            | Self::EmbossDepth => Amount::Length,
        }
    }

    pub fn rule(self, kind: &FeatureKind) -> Rule {
        match self {
            Self::HoleDepth
            | Self::HoleDiameter
            | Self::CounterboreDiameter
            | Self::CounterboreDepth
            | Self::CountersinkDiameter
            | Self::StepDiameter(_)
            | Self::StepDepth(_)
            | Self::SlotLength
            | Self::ThreadDepth
            | Self::BlendSize
            | Self::BlendSecond
            | Self::ShellThickness
            | Self::ShellFaceThickness(_)
            | Self::WallThickness
            | Self::PipeSize
            | Self::PipeWall
            | Self::CoilDiameter
            | Self::CoilSection
            | Self::CoilPitch
            | Self::CoilHeight
            | Self::RibThickness
            | Self::EmbossDepth => Rule::AboveZero,
            Self::PipeBend => Rule::ZeroOrMore,
            Self::CountersinkAngle => Rule::ConeAngle,
            Self::BlendAngle => Rule::ChamferAngle,
            Self::OffsetDistance
            | Self::MoveDistance
            | Self::MoveAngle
            | Self::DraftAngle
            | Self::PlaneOffset
            | Self::PlaneAngle
            | Self::EndOffset(_)
            | Self::SlotAngle
            | Self::HoleEndOffset => Rule::Any,
            Self::Taper => Rule::Taper,
            Self::PatternSpacing(_) | Self::CurveSpacing => Rule::AboveZeroOrReverse,
            Self::CircularCount | Self::CurveCount => Rule::Count,
            Self::PrismSides => Rule::Sides,
            Self::CircularAngle => Rule::Turn,
            Self::PrimitiveSize(index) => {
                let rule = kind
                    .primitive()
                    .and_then(|primitive| primitive.shape.rules().into_iter().nth(index));
                match rule {
                    Some(SizeRule::ZeroOrMore) => Rule::ZeroOrMore,
                    Some(SizeRule::Sides) => Rule::Sides,
                    Some(SizeRule::AboveZero) | None => Rule::AboveZero,
                }
            }
        }
    }

    pub fn caption(self, kind: &FeatureKind) -> Option<String> {
        let fixed = match self {
            Self::HoleDepth => HOLE_DEPTH,
            Self::HoleDiameter => HOLE_DIAMETER,
            Self::CounterboreDiameter => hole_panel::COUNTERBORE_DIAMETER,
            Self::CounterboreDepth => hole_panel::COUNTERBORE_DEPTH,
            Self::CountersinkDiameter => hole_panel::COUNTERSINK_DIAMETER,
            Self::CountersinkAngle => hole_panel::COUNTERSINK_ANGLE,
            Self::StepDiameter(index) => return Some(hole_panel::step_diameter_caption(index)),
            Self::StepDepth(index) => return Some(hole_panel::step_depth_caption(index)),
            Self::SlotLength => hole_panel::SLOT_LENGTH,
            Self::SlotAngle => hole_panel::SLOT_ANGLE,
            Self::ThreadDepth => hole_panel::THREAD_DEPTH,
            Self::HoleEndOffset => solid_panel::END_OFFSET,
            Self::OffsetDistance => OFFSET_DISTANCE,
            Self::MoveDistance => move_face_panel::DISTANCE,
            Self::MoveAngle => move_face_panel::ANGLE,
            Self::DraftAngle => draft_panel::ANGLE,
            Self::PlaneOffset => PLANE_OFFSET,
            Self::PlaneAngle => PLANE_ANGLE,
            Self::ShellThickness => SHELL_THICKNESS,
            Self::ShellFaceThickness(index) => {
                return Some(shell_panel::own_thickness_caption(index));
            }
            Self::WallThickness => solid_panel::WALL_THICKNESS,
            Self::PipeSize => return Some(capitalized(pipe_size_name(kind.pipe()?.shape))),
            Self::PipeWall => pipe_panel::WALL_THICKNESS,
            Self::PipeBend => pipe_panel::BEND_RADIUS,
            Self::Taper => solid_panel::TAPER,
            Self::EndOffset(reach) => solid_panel::end_offset_caption(reach),
            Self::CircularAngle => pattern_panel::TOTAL_ANGLE,
            Self::CircularCount | Self::CurveCount => pattern_panel::FIRST.count,
            Self::CurveSpacing => pattern_panel::FIRST.spacing,
            Self::CoilDiameter if is_flat_spiral(kind) => helix_panel::INNER_DIAMETER,
            Self::CoilDiameter => coil_panel::DIAMETER,
            Self::CoilSection => return Some(capitalized(coil_size_name(kind.coil()?.shape))),
            Self::CoilPitch => coil_panel::PITCH,
            Self::CoilHeight if is_flat_spiral(kind) => helix_panel::OUTER_DIAMETER,
            Self::CoilHeight => coil_panel::HEIGHT,
            Self::RibThickness => rib_panel::THICKNESS,
            Self::EmbossDepth => emboss_panel::DEPTH,
            Self::BlendSize => blend_panel::size_caption(kind.blend()?),
            Self::BlendSecond => blend_panel::SECOND_DISTANCE,
            Self::BlendAngle => blend_panel::CHAMFER_ANGLE,
            Self::PrimitiveSize(index) => {
                let (what, _) = kind.primitive()?.shape.sizes().into_iter().nth(index)?;
                return Some(capitalized(what));
            }
            Self::PrismSides => return Self::PrimitiveSize(0).caption(kind),
            Self::PatternSpacing(index) => {
                let mut probe = kind.clone();
                let direction = direction_mut(&mut probe, index)?;
                let captions = pattern_panel::direction_captions(index);
                match direction.measured {
                    LinearSpacing::BetweenCopies => captions.spacing,
                    LinearSpacing::Total => captions.total,
                }
            }
        };
        Some(fixed.to_owned())
    }

    pub fn words(self, kind: &FeatureKind) -> String {
        let fixed = match self {
            Self::HoleDepth => "Drag to change the hole's depth",
            Self::HoleDiameter => "Drag to change the hole's diameter",
            Self::CounterboreDiameter => "Drag to change the counterbore's diameter",
            Self::CounterboreDepth => "Drag to change the counterbore's depth",
            Self::CountersinkDiameter => "Drag to change the countersink's diameter",
            Self::CountersinkAngle => "Drag to change the countersink's angle",
            Self::StepDiameter(index) => {
                return format!("Drag to change the diameter of step {}", index + 1);
            }
            Self::StepDepth(index) => {
                return format!("Drag to change the depth of step {}", index + 1);
            }
            Self::SlotLength => "Drag to change the slot's length",
            Self::SlotAngle => "Drag to turn the slot",
            Self::ThreadDepth => "Drag to change how deep the thread goes",
            Self::HoleEndOffset => "Drag to change how far the hole goes past the face",
            Self::OffsetDistance => "Drag to change how far the faces move",
            Self::MoveDistance => "Drag to change how far the faces move",
            Self::MoveAngle => "Drag to tilt the faces about the axis",
            Self::DraftAngle => "Drag to change the draft angle",
            Self::PlaneOffset => "Drag to change the plane's offset",
            Self::PlaneAngle => "Drag to turn the plane about its axis",
            Self::ShellThickness => "Drag to change the shell's thickness",
            Self::ShellFaceThickness(index) => {
                return format!("Drag to change the thickness of face {}", index + 1);
            }
            Self::WallThickness => "Drag to change the wall's thickness",
            Self::PipeSize => match kind.pipe() {
                Some(pipe) => {
                    return format!("Drag to change the pipe's {}", pipe_size_name(pipe.shape));
                }
                None => "Drag to change the pipe's size",
            },
            Self::PipeWall => "Drag to change the pipe's wall thickness",
            Self::PipeBend => "Drag to change the pipe's bend radius",
            Self::Taper => "Drag to change the extrusion's taper",
            Self::EndOffset(_) => "Drag to change how far the end goes past the face",
            Self::CircularAngle => "Drag to change the pattern's total angle",
            Self::CircularCount | Self::CurveCount => "Drag to add or remove copies",
            Self::CurveSpacing => "Drag to change the pattern's spacing",
            Self::PatternSpacing(_) => match self.caption(kind).as_deref() {
                Some(caption) => {
                    return format!("Drag to change the pattern's {}", caption.to_lowercase());
                }
                None => "Drag to change the pattern's spacing",
            },
            Self::CoilDiameter if is_flat_spiral(kind) => {
                "Drag to change the spiral's inner diameter"
            }
            Self::CoilDiameter => {
                return format!("Drag to change the {}'s diameter", winding_noun(kind));
            }
            Self::CoilSection => match kind.coil() {
                Some(coil) => {
                    return format!("Drag to change the coil's {}", coil_size_name(coil.shape));
                }
                None => "Drag to change the coil's section",
            },
            Self::CoilPitch => {
                return format!("Drag to change the {}'s pitch", winding_noun(kind));
            }
            Self::CoilHeight if is_flat_spiral(kind) => {
                "Drag to change the spiral's outer diameter"
            }
            Self::CoilHeight => {
                return format!("Drag to change the {}'s height", winding_noun(kind));
            }
            Self::RibThickness => "Drag to change the rib's thickness",
            Self::EmbossDepth => "Drag to change how far the outlines stand out or sink in",
            Self::BlendSecond => "Drag to change the chamfer's second distance",
            Self::BlendAngle => "Drag to change the chamfer's angle",
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
            Self::PrismSides => return Self::PrimitiveSize(0).words(kind),
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
            Self::CounterboreDiameter => match kind {
                FeatureKind::Hole(Hole {
                    style: HoleStyle::Counterbore { diameter, .. },
                    ..
                }) => Some(diameter),
                _ => None,
            },
            Self::CounterboreDepth => match kind {
                FeatureKind::Hole(Hole {
                    style: HoleStyle::Counterbore { depth, .. },
                    ..
                }) => Some(depth),
                _ => None,
            },
            Self::CountersinkDiameter => match kind {
                FeatureKind::Hole(Hole {
                    style: HoleStyle::Countersink { diameter, .. },
                    ..
                }) => Some(diameter),
                _ => None,
            },
            Self::CountersinkAngle => match kind {
                FeatureKind::Hole(Hole {
                    style: HoleStyle::Countersink { angle, .. },
                    ..
                }) => Some(angle),
                _ => None,
            },
            Self::StepDiameter(index) => match kind {
                FeatureKind::Hole(Hole {
                    style: HoleStyle::Stepped(steps),
                    ..
                }) => Some(&mut steps.get_mut(index)?.diameter),
                _ => None,
            },
            Self::StepDepth(index) => match kind {
                FeatureKind::Hole(Hole {
                    style: HoleStyle::Stepped(steps),
                    ..
                }) => Some(&mut steps.get_mut(index)?.depth),
                _ => None,
            },
            Self::SlotLength => match kind {
                FeatureKind::Hole(Hole {
                    shape: HoleShape::Slot { length, .. },
                    ..
                }) => Some(length),
                _ => None,
            },
            Self::SlotAngle => match kind {
                FeatureKind::Hole(Hole {
                    shape: HoleShape::Slot { angle, .. },
                    ..
                }) => Some(angle),
                _ => None,
            },
            Self::ThreadDepth => match kind {
                FeatureKind::Hole(hole) => hole.thread.depth.as_mut(),
                _ => None,
            },
            Self::HoleEndOffset => match kind {
                FeatureKind::Hole(hole) => Some(
                    hole_offset_mut(&mut hole.depth)?
                        .get_or_insert_with(|| Box::new(zero()))
                        .as_mut(),
                ),
                _ => None,
            },
            Self::OffsetDistance => match kind {
                FeatureKind::OffsetFace(OffsetFace { distance, .. }) => Some(distance),
                _ => None,
            },
            Self::EmbossDepth => match kind {
                FeatureKind::Emboss(Emboss { depth, .. }) => Some(depth),
                _ => None,
            },
            Self::MoveDistance => match kind {
                FeatureKind::MoveFace(MoveFace { distance, .. }) => Some(distance),
                _ => None,
            },
            Self::MoveAngle => match kind {
                FeatureKind::MoveFace(MoveFace { angle, .. }) => Some(angle),
                _ => None,
            },
            Self::DraftAngle => match kind {
                FeatureKind::Draft(Draft { angle, .. }) => Some(angle),
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
            Self::BlendSecond => match kind {
                FeatureKind::Blend(Blend {
                    form: ChamferForm::TwoDistances { second },
                    ..
                }) => Some(second),
                _ => None,
            },
            Self::BlendAngle => match kind {
                FeatureKind::Blend(Blend {
                    form: ChamferForm::DistanceAngle { angle },
                    ..
                }) => Some(angle),
                _ => None,
            },
            Self::ShellThickness => match kind {
                FeatureKind::Shell(Shell { thickness, .. }) => Some(thickness),
                _ => None,
            },
            Self::ShellFaceThickness(index) => match kind {
                FeatureKind::Shell(Shell { faces, .. }) => {
                    Some(&mut faces.get_mut(index)?.thickness)
                }
                _ => None,
            },
            Self::WallThickness => Some(&mut wall_mut(kind)?.thickness),
            Self::PipeSize => Some(&mut pipe_mut(kind)?.size),
            Self::PipeWall => pipe_mut(kind)?.wall.as_mut(),
            Self::PipeBend => Some(&mut pipe_mut(kind)?.bend),
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
            Self::PrismSides => Self::PrimitiveSize(0).slot(kind),
            Self::PatternSpacing(index) => Some(&mut direction_mut(kind, index)?.spacing),
            Self::CircularAngle => Some(&mut circular_mut(kind)?.angle),
            Self::CircularCount => Some(&mut circular_mut(kind)?.count),
            Self::CurveSpacing => Some(&mut curve_mut(kind)?.spacing),
            Self::CurveCount => Some(&mut curve_mut(kind)?.count),
            Self::CoilDiameter => Some(winding_mut(kind)?.0),
            Self::CoilSection => Some(&mut coil_mut(kind)?.size),
            Self::CoilPitch => match winding_mut(kind)?.1 {
                CoilLength::TurnsAndPitch { pitch, .. }
                | CoilLength::HeightAndPitch { pitch, .. } => Some(pitch),
                CoilLength::HeightAndTurns { .. } => None,
            },
            Self::CoilHeight => match winding_mut(kind)?.1 {
                CoilLength::HeightAndTurns { height, .. }
                | CoilLength::HeightAndPitch { height, .. } => Some(height),
                CoilLength::TurnsAndPitch { .. } => None,
            },
            Self::RibThickness => match kind {
                FeatureKind::Rib(rib) => Some(&mut rib.thickness),
                _ => None,
            },
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
            Self::HoleDiameter
            | Self::CounterboreDiameter
            | Self::CounterboreDepth
            | Self::CountersinkDiameter
            | Self::CountersinkAngle
            | Self::StepDiameter(_)
            | Self::StepDepth(_) => {
                if let FeatureKind::Hole(hole) = kind {
                    hole.standard = None;
                }
            }
            Self::HoleEndOffset => {
                if let FeatureKind::Hole(hole) = kind
                    && let Some(offset) = hole_offset_mut(&mut hole.depth)
                    && offset.as_deref().is_some_and(solid_panel::is_zero)
                {
                    *offset = None;
                }
            }
            Self::HoleDepth
            | Self::SlotLength
            | Self::SlotAngle
            | Self::ThreadDepth
            | Self::OffsetDistance
            | Self::MoveDistance
            | Self::MoveAngle
            | Self::DraftAngle
            | Self::PlaneOffset
            | Self::PlaneAngle
            | Self::BlendSize
            | Self::BlendSecond
            | Self::BlendAngle
            | Self::ShellThickness
            | Self::ShellFaceThickness(_)
            | Self::WallThickness
            | Self::PipeSize
            | Self::PipeWall
            | Self::PipeBend
            | Self::PrimitiveSize(_)
            | Self::PrismSides
            | Self::PatternSpacing(_)
            | Self::CircularAngle
            | Self::CircularCount
            | Self::CurveSpacing
            | Self::CurveCount
            | Self::CoilDiameter
            | Self::CoilSection
            | Self::CoilPitch
            | Self::CoilHeight
            | Self::RibThickness
            | Self::EmbossDepth => {}
        }
    }

    pub fn step(self, per_point: f64) -> f64 {
        match self {
            Self::Taper => TAPER_STEP_DEGREES,
            Self::PlaneAngle
            | Self::CircularAngle
            | Self::BlendAngle
            | Self::CountersinkAngle
            | Self::MoveAngle
            | Self::DraftAngle
            | Self::SlotAngle => STEP_DEGREES,
            Self::CircularCount | Self::PrismSides | Self::CurveCount => 1.0,
            Self::HoleDepth
            | Self::HoleDiameter
            | Self::CounterboreDiameter
            | Self::CounterboreDepth
            | Self::CountersinkDiameter
            | Self::StepDiameter(_)
            | Self::StepDepth(_)
            | Self::SlotLength
            | Self::ThreadDepth
            | Self::HoleEndOffset
            | Self::OffsetDistance
            | Self::MoveDistance
            | Self::PlaneOffset
            | Self::BlendSize
            | Self::BlendSecond
            | Self::ShellThickness
            | Self::ShellFaceThickness(_)
            | Self::WallThickness
            | Self::PipeSize
            | Self::PipeWall
            | Self::PipeBend
            | Self::EndOffset(_)
            | Self::PrimitiveSize(_)
            | Self::PatternSpacing(_)
            | Self::CurveSpacing
            | Self::CoilDiameter
            | Self::CoilSection
            | Self::CoilPitch
            | Self::CoilHeight
            | Self::RibThickness
            | Self::EmbossDepth => step_for(per_point),
        }
    }

    pub fn accepts(self, value: f64, step: f64, kind: &FeatureKind) -> f64 {
        let most_copies = f64::from(MAX_PATTERN_INSTANCES);
        match self {
            Self::HoleDepth
            | Self::HoleDiameter
            | Self::CounterboreDiameter
            | Self::CounterboreDepth
            | Self::CountersinkDiameter
            | Self::StepDiameter(_)
            | Self::StepDepth(_)
            | Self::SlotLength
            | Self::ThreadDepth
            | Self::BlendSize
            | Self::BlendSecond
            | Self::ShellThickness
            | Self::ShellFaceThickness(_)
            | Self::WallThickness
            | Self::PipeSize
            | Self::PipeWall
            | Self::PatternSpacing(_)
            | Self::CurveSpacing
            | Self::CoilDiameter
            | Self::CoilSection
            | Self::CoilPitch
            | Self::CoilHeight
            | Self::RibThickness
            | Self::EmbossDepth => value.max(step),
            Self::PipeBend => value.max(0.0),
            Self::BlendAngle => value.clamp(step, HALF_TURN_DEGREES - step),
            Self::PrimitiveSize(index) => {
                let may_vanish = matches!(
                    (kind.primitive().map(|primitive| &primitive.shape), index),
                    (Some(PrimitiveShape::Cone { .. }), 0 | 1)
                        | (Some(PrimitiveShape::Wedge { .. }), 3)
                );
                value.max(if may_vanish { 0.0 } else { step })
            }
            Self::OffsetDistance if value == 0.0 => step,
            Self::DraftAngle => {
                let steepest = STEEPEST_DRAFT.to_degrees() - step;
                value.clamp(-steepest, steepest)
            }
            Self::OffsetDistance
            | Self::MoveDistance
            | Self::MoveAngle
            | Self::PlaneOffset
            | Self::PlaneAngle
            | Self::EndOffset(_)
            | Self::SlotAngle
            | Self::HoleEndOffset => value,
            Self::CountersinkAngle => value.clamp(step, MAX_CONE_ANGLE),
            Self::Taper => {
                let steepest = MAX_TAPER_DEGREES - step;
                value.clamp(-steepest, steepest)
            }
            Self::CircularAngle => value.clamp(step, FULL_TURN_DEGREES),
            Self::CircularCount => value.round().clamp(1.0, most_copies),
            Self::CurveCount => value.round().clamp(2.0, most_copies),
            Self::PrismSides => value
                .round()
                .clamp(f64::from(MIN_PRISM_SIDES), f64::from(MAX_PRISM_SIDES)),
        }
    }

    pub fn reversed(self, kind: &FeatureKind) -> Option<FeatureKind> {
        let mut reversed = kind.clone();
        match self {
            Self::HoleDepth => {
                let FeatureKind::Hole(hole) = &mut reversed else {
                    return None;
                };
                hole.reversed = !hole.reversed;
            }
            Self::PatternSpacing(index) => {
                let direction = direction_mut(&mut reversed, index)?;
                direction.reversed = !direction.reversed;
            }
            Self::CircularAngle => {
                let circular = circular_mut(&mut reversed)?;
                circular.reversed = !circular.reversed;
            }
            Self::CoilPitch | Self::CoilHeight => match &mut reversed {
                FeatureKind::Coil(coil) => coil.reversed = !coil.reversed,
                FeatureKind::Helix(helix) if !helix.is_flat() => helix.reversed = !helix.reversed,
                _ => return None,
            },
            Self::PrimitiveSize(index) => {
                let FeatureKind::Primitive(primitive) = &mut reversed else {
                    return None;
                };
                let grows_one_way = primitive.anchor != PrimitiveAnchor::Centre;
                (grows_one_way && height_index(&primitive.shape) == Some(index)).then_some(())?;
                primitive.reversed = !primitive.reversed;
            }
            Self::RibThickness => {
                let FeatureKind::Rib(rib) = &mut reversed else {
                    return None;
                };
                rib.side = match rib.side {
                    RibSide::OneSide => RibSide::OtherSide,
                    RibSide::OtherSide => RibSide::OneSide,
                    RibSide::Symmetric => return None,
                };
            }
            Self::ShellThickness => {
                let FeatureKind::Shell(shell) = &mut reversed else {
                    return None;
                };
                shell.side = match shell.side {
                    ShellSide::Inside => ShellSide::Outside,
                    ShellSide::Outside => ShellSide::Inside,
                    ShellSide::BothSides => return None,
                };
            }
            Self::WallThickness => {
                let wall = wall_mut(&mut reversed)?;
                wall.side = match wall.side {
                    WallSide::Inside => WallSide::Outside,
                    WallSide::Outside => WallSide::Inside,
                    WallSide::Centred => return None,
                };
            }
            Self::HoleDiameter
            | Self::CounterboreDiameter
            | Self::CounterboreDepth
            | Self::CountersinkDiameter
            | Self::CountersinkAngle
            | Self::StepDiameter(_)
            | Self::StepDepth(_)
            | Self::SlotLength
            | Self::SlotAngle
            | Self::ThreadDepth
            | Self::HoleEndOffset
            | Self::OffsetDistance
            | Self::MoveDistance
            | Self::MoveAngle
            | Self::DraftAngle
            | Self::PlaneOffset
            | Self::PlaneAngle
            | Self::BlendSize
            | Self::BlendSecond
            | Self::BlendAngle
            | Self::ShellFaceThickness(_)
            | Self::PipeSize
            | Self::PipeWall
            | Self::PipeBend
            | Self::Taper
            | Self::EndOffset(_)
            | Self::PrismSides
            | Self::CircularCount
            | Self::CurveSpacing
            | Self::CurveCount
            | Self::CoilDiameter
            | Self::CoilSection
            | Self::EmbossDepth => return None,
        }
        Some(reversed)
    }

    pub fn reversal_words(self) -> &'static str {
        match self {
            Self::RibThickness | Self::ShellThickness | Self::WallThickness => {
                "switched to the other side"
            }
            _ => Flip::Reversed.words(),
        }
    }
}

fn height_index(shape: &PrimitiveShape) -> Option<usize> {
    match shape {
        PrimitiveShape::Box { .. }
        | PrimitiveShape::Cone { .. }
        | PrimitiveShape::Wedge { .. }
        | PrimitiveShape::Prism { .. } => Some(2),
        PrimitiveShape::Cylinder { .. } => Some(1),
        PrimitiveShape::Sphere { .. } | PrimitiveShape::Torus { .. } => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Track {
    Line { base: Point3, direction: Vector3 },
    Turn(Swing),
    Curve(CurveTrack),
}

impl Track {
    pub fn at(&self, position: f64) -> Point3 {
        match self {
            Self::Line { base, direction } => *base + *direction * position,
            Self::Turn(swing) => swing.at(position),
            Self::Curve(track) => track.at(position),
        }
    }

    pub fn direction(&self, position: f64) -> Vector3 {
        match self {
            Self::Line { direction, .. } => *direction,
            Self::Turn(swing) => swing.tangent(position),
            Self::Curve(track) => track.tangent(position),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mapping {
    Linear { scale: f64, offset: f64 },
    Tangent { arm: f64 },
    Reciprocal { turn: f64, less: f64 },
    Cotangent { run: f64 },
}

impl Mapping {
    pub fn position(self, value: f64) -> f64 {
        match self {
            Self::Linear { scale, offset } => offset + scale * value,
            Self::Tangent { arm } => arm * value.to_radians().tan(),
            Self::Reciprocal { turn, less } => turn / (value - less),
            Self::Cotangent { run } => run / (value / 2.0).to_radians().tan(),
        }
    }

    pub fn value(self, position: f64) -> Option<f64> {
        let value = match self {
            Self::Linear { scale, offset } if scale != 0.0 => (position - offset) / scale,
            Self::Tangent { arm } if arm > 0.0 => (position / arm).atan().to_degrees(),
            Self::Reciprocal { turn, less } if position > 0.0 => less + turn / position,
            Self::Cotangent { run } if position > 0.0 => 2.0 * (run / position).atan().to_degrees(),
            Self::Linear { .. }
            | Self::Tangent { .. }
            | Self::Reciprocal { .. }
            | Self::Cotangent { .. } => return None,
        };
        value.is_finite().then_some(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
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
            FeatureKind::Hole(hole) => self.hole(hole),
            FeatureKind::OffsetFace(offset) => self.offset(offset).into_iter().collect(),
            FeatureKind::Emboss(emboss) => self.emboss(emboss).into_iter().collect(),
            FeatureKind::MoveFace(moving) => self.moving(moving),
            FeatureKind::Draft(draft) => self.draft(draft).into_iter().collect(),
            FeatureKind::Datum(Datum::Plane(plane)) => {
                [self.plane_offset(plane), self.plane_angle(plane)]
                    .into_iter()
                    .flatten()
                    .collect()
            }
            FeatureKind::Blend(blend) => self.blend(blend),
            FeatureKind::Shell(shell) => {
                let mut gauges: Vec<Gauge> = self.shell(shell).into_iter().collect();
                gauges.extend(self.shell_faces(shell));
                gauges
            }
            FeatureKind::Pipe(pipe) => self.pipe(pipe),
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
            FeatureKind::Coil(coil) => self.coil(coil),
            FeatureKind::Helix(helix) => self.helix(helix),
            FeatureKind::Rib(rib) => self.rib(rib).into_iter().collect(),
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

    fn hole_top(&self, hole: &Hole) -> Option<HoleTop> {
        let document = self.model.document();
        let sketch = document.feature(hole.sketch)?;
        let displayed = self.model.displayed_sketch(sketch)?;
        let (point, centre) = hole_centres(&displayed).into_iter().next()?;
        let circle = hole
            .sizing
            .by_circles()
            .then(|| circle_sizes(&displayed).get(&point).copied())
            .flatten();
        let typed = self.length(&hole.diameter);
        let bore = circle.map(|circle| circle.diameter).or(typed);
        let heads = match circle {
            Some(circle) if hole.sizing == HoleSizing::CirclesAndHeads => typed
                .filter(|typed| *typed > 0.0)
                .map(|typed| circle.diameter / typed),
            _ => Some(1.0),
        };
        let plane = scene::sketch_plane(document, self.model.evaluation(), hole.sketch)?;
        let down = if hole.reversed {
            plane.normal()
        } else {
            -plane.normal()
        };
        Some(HoleTop {
            plane,
            top: plane.to_world(centre),
            down,
            bore,
            heads,
        })
    }

    fn hole(&self, hole: &Hole) -> Vec<Gauge> {
        let Some(top) = self.hole_top(hole) else {
            return Vec::new();
        };
        let mut gauges: Vec<Gauge> = [self.hole_depth(hole, &top), self.hole_diameter(hole, &top)]
            .into_iter()
            .flatten()
            .collect();
        gauges.extend(self.hole_style(hole, &top));
        gauges.extend(self.hole_slot(hole, &top));
        gauges.extend(self.hole_thread_depth(hole, &top));
        gauges.extend(self.hole_end_offset(hole, &top));
        gauges
    }

    fn hole_depth(&self, hole: &Hole, top: &HoleTop) -> Option<Gauge> {
        let HoleDepth::Blind(depth) = &hole.depth else {
            return None;
        };
        let value = self.length(depth)?;
        Some(Gauge::line(
            Measured::HoleDepth,
            (top.top, top.down),
            1.0,
            value,
        ))
    }

    fn hole_width(&self, hole: &Hole) -> Option<f64> {
        if hole.sizing.by_circles() {
            return None;
        }
        self.length(&hole.diameter)
    }

    fn bore_level(&self, hole: &Hole, top: &HoleTop, width: f64) -> f64 {
        let heads = top.heads.unwrap_or(1.0);
        let level = match &hole.style {
            HoleStyle::Plain => Some(0.0),
            HoleStyle::Counterbore { depth, .. } => self.length(depth).map(|depth| depth * heads),
            HoleStyle::Countersink { diameter, angle } => {
                let wide = self.length(diameter).map(|wide| wide * heads);
                let angle = self.angle(angle);
                wide.zip(angle).map(|(wide, angle)| {
                    ((wide - width) / 2.0 / (angle / 2.0).to_radians().tan()).max(0.0)
                })
            }
            HoleStyle::Stepped(steps) => steps
                .iter()
                .map(|step| self.length(&step.depth))
                .sum::<Option<f64>>()
                .map(|depth| depth * heads),
        };
        level.filter(|level| level.is_finite()).unwrap_or(0.0)
    }

    fn hole_diameter(&self, hole: &Hole, top: &HoleTop) -> Option<Gauge> {
        let value = self.hole_width(hole)?;
        let level = self.bore_level(hole, top, value);
        let (base, direction) = top.rim(hole, self);
        Some(Gauge::line(
            Measured::HoleDiameter,
            (base + top.down * level, direction),
            0.5,
            value,
        ))
    }

    fn hole_style(&self, hole: &Hole, top: &HoleTop) -> Vec<Gauge> {
        let (Some(width), Some(heads)) = (top.bore, top.heads) else {
            return Vec::new();
        };
        let (rim_base, rim) = top.rim(hole, self);
        let (wall_base, wall) = top.wall(hole, self);
        match &hole.style {
            HoleStyle::Counterbore { diameter, depth } => {
                let (Some(wide), Some(deep)) = (self.length(diameter), self.length(depth)) else {
                    return Vec::new();
                };
                vec![
                    Gauge::line(
                        Measured::CounterboreDiameter,
                        (rim_base, rim),
                        0.5 * heads,
                        wide,
                    ),
                    Gauge::line(
                        Measured::CounterboreDepth,
                        (wall_base + wall * (wide * heads / 2.0), top.down),
                        heads,
                        deep,
                    ),
                ]
            }
            HoleStyle::Countersink { diameter, angle }
                if matches!(hole.shape, HoleShape::Round) =>
            {
                let (Some(wide), Some(angle)) = (self.length(diameter), self.angle(angle)) else {
                    return Vec::new();
                };
                let mouth = Gauge::line(
                    Measured::CountersinkDiameter,
                    (rim_base, rim),
                    0.5 * heads,
                    wide,
                );
                let run = (wide * heads - width) / 2.0;
                let flare = (run > SMALLEST_RADIUS).then(|| Gauge {
                    measured: Measured::CountersinkAngle,
                    track: Track::Line {
                        base: wall_base + wall * (width / 2.0),
                        direction: top.down,
                    },
                    mapping: Mapping::Cotangent { run },
                    value: angle,
                });
                std::iter::once(mouth).chain(flare).collect()
            }
            HoleStyle::Stepped(steps) => self.hole_steps(steps, hole, top),
            HoleStyle::Plain | HoleStyle::Countersink { .. } => Vec::new(),
        }
    }

    fn hole_steps(&self, steps: &[HoleStep], hole: &Hole, top: &HoleTop) -> Vec<Gauge> {
        let heads = top.heads.unwrap_or(1.0);
        let (rim_base, rim) = top.rim(hole, self);
        let (wall_base, wall) = top.wall(hole, self);
        let mut gauges = Vec::new();
        let mut above = 0.0;
        for (index, step) in steps.iter().enumerate() {
            let (Some(wide), Some(deep)) = (self.length(&step.diameter), self.length(&step.depth))
            else {
                break;
            };
            let level = top.down * above;
            gauges.push(Gauge::line(
                Measured::StepDiameter(index),
                (rim_base + level, rim),
                0.5 * heads,
                wide,
            ));
            gauges.push(Gauge::line(
                Measured::StepDepth(index),
                (wall_base + level + wall * (wide * heads / 2.0), top.down),
                heads,
                deep,
            ));
            above += deep * heads;
        }
        gauges
    }

    fn hole_slot(&self, hole: &Hole, top: &HoleTop) -> Vec<Gauge> {
        let HoleShape::Slot { angle, .. } = &hole.shape else {
            return Vec::new();
        };
        let (Some(angle), Some((along, _, length)), Some(width)) =
            (self.angle(angle), top.slot(hole, self), top.bore)
        else {
            return Vec::new();
        };
        let tip = Gauge {
            measured: Measured::SlotLength,
            track: Track::Line {
                base: top.top,
                direction: along,
            },
            mapping: Mapping::Linear {
                scale: 0.5,
                offset: width / 2.0,
            },
            value: length,
        };
        let turn = Gauge {
            measured: Measured::SlotAngle,
            track: Track::Turn(Swing {
                foot: top.top,
                radial: -top.plane.x_axis(),
                normal: -top.plane.y_axis(),
                radius: length / 2.0 + width / 2.0,
            }),
            mapping: Mapping::Linear {
                scale: 1.0,
                offset: 0.0,
            },
            value: angle,
        };
        vec![tip, turn]
    }

    fn hole_thread_depth(&self, hole: &Hole, top: &HoleTop) -> Option<Gauge> {
        hole_thread(hole)?;
        let depth = hole.thread.depth.as_ref()?;
        let value = self.length(depth)?;
        let width = top.bore?;
        let level = self.bore_level(hole, top, width);
        let (wall_base, wall) = top.wall(hole, self);
        Some(Gauge::line(
            Measured::ThreadDepth,
            (
                wall_base + top.down * level + wall * (width / 2.0),
                top.down,
            ),
            1.0,
            value,
        ))
    }

    fn hole_end_offset(&self, hole: &Hole, top: &HoleTop) -> Option<Gauge> {
        let (face, offset) = match &hole.depth {
            HoleDepth::UpToFace { target, offset } => (
                displayed_plane(self.model.evaluation(), self.feature, target)?,
                offset,
            ),
            HoleDepth::UpToNext { offset } => (self.reached_end(ReachedSide::Only)?.plane, offset),
            HoleDepth::Blind(_) | HoleDepth::ThroughAll => return None,
        };
        let value = offset
            .as_deref()
            .map_or(Some(0.0), |offset| self.length(offset))?;
        let facing = face.normal().dot(top.down).abs();
        (facing > OFF_THE_LINE).then_some(())?;
        let reached = -face.signed_distance(top.top) / face.normal().dot(top.down);
        Some(Gauge::line(
            Measured::HoleEndOffset,
            (top.top + top.down * reached, top.down),
            1.0 / facing,
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

    fn moving(&self, moving: &MoveFace) -> Vec<Gauge> {
        let stand = || {
            let solid = &handle_snap::before(self.model, self.feature)?.solid;
            let faces = moving.resolve(solid).ok()?;
            let first = *faces.first()?;
            let (middle, _) = face_stand(solid, first)?;
            let (rim, _) = rim_stand(solid, first)?;
            Some((solid, faces, middle, rim))
        };
        let Some((solid, faces, middle, rim)) = stand() else {
            return Vec::new();
        };
        let distance = self.move_distance(moving, solid, &faces, middle);
        let angle = self.move_angle(moving, rim);
        distance.into_iter().chain(angle).collect()
    }

    fn move_distance(
        &self,
        moving: &MoveFace,
        solid: &Solid,
        faces: &[FaceId],
        at: Point3,
    ) -> Option<Gauge> {
        let value = self.length(&moving.distance)?;
        let along = match &moving.direction {
            Some(reference) => {
                displayed_axis(self.shown_evaluation(), self.feature, reference)?.direction()
            }
            None => faces.iter().find_map(|face| flat_normal(solid, *face))?,
        };
        Some(Gauge::line(
            Measured::MoveDistance,
            (at, along.try_normalize()?),
            1.0,
            value,
        ))
    }

    fn move_angle(&self, moving: &MoveFace, at: Point3) -> Option<Gauge> {
        let value = self.angle(&moving.angle)?;
        let axis = displayed_axis(self.shown_evaluation(), self.feature, moving.axis.as_ref()?)?;
        let along = axis.direction().try_normalize()?;
        let foot = axis.origin() + along * (at - axis.origin()).dot(along);
        let swing = self.swing_to(foot, at - foot, along, along.any_orthonormal_vector())?;
        Some(Gauge {
            measured: Measured::MoveAngle,
            track: Track::Turn(swing),
            mapping: Mapping::Linear {
                scale: 1.0,
                offset: 0.0,
            },
            value,
        })
    }

    fn swing_to(
        &self,
        foot: Point3,
        reach: Vector3,
        axis: Vector3,
        otherwise: Vector3,
    ) -> Option<Swing> {
        let arm = ARROW_POINTS * (self.per_point)(foot)?;
        let radius = reach.length().max(arm);
        let radial = reach.try_normalize().unwrap_or(otherwise);
        Some(Swing {
            foot,
            radial,
            normal: axis.cross(radial),
            radius,
        })
    }

    fn draft(&self, draft: &Draft) -> Option<Gauge> {
        let value = self.angle(&draft.angle)?;
        let solid = &handle_snap::before(self.model, self.feature)?.solid;
        let faces = draft.resolve(solid).ok()?;
        let (face, plane) = faces
            .iter()
            .find_map(|face| Some((*face, face_plane(solid, *face)?)))
            .or_else(|| faces.iter().find_map(|face| round_tangent(solid, *face)))?;
        let neutral = displayed_plane(self.shown_evaluation(), self.feature, &draft.neutral)?;
        let pull = if draft.flipped {
            -neutral.normal()
        } else {
            neutral.normal()
        };
        let normal = plane.normal();
        let hinge = normal.cross(pull).try_normalize()?;
        let across = hinge.cross(normal);
        let toward = pull.dot(neutral.origin() - plane.origin()) / pull.dot(across);
        let on_hinge = plane.origin() + across * toward;
        let (at, _) = face_stand(solid, face)?;
        let foot = on_hinge + hinge * (at - on_hinge).dot(hinge);
        let swing = self.swing_to(foot, at - foot, hinge, across)?;
        let (sin, cos) = normal.dot(pull).clamp(-1.0, 1.0).asin().sin_cos();
        let radial = swing.radial * cos - hinge.cross(swing.radial) * sin;
        Some(Gauge {
            measured: Measured::DraftAngle,
            track: Track::Turn(Swing {
                radial,
                normal: hinge.cross(radial),
                ..swing
            }),
            mapping: Mapping::Linear {
                scale: 1.0,
                offset: 0.0,
            },
            value,
        })
    }

    fn emboss(&self, emboss: &Emboss) -> Option<Gauge> {
        let value = self.length(&emboss.depth)?;
        let document = self.model.document();
        let evaluation = self.model.evaluation();
        let solid = &evaluation.body_before(self.feature)?.solid()?.solid;
        let faces = emboss.resolve(solid).ok()?;
        let (sketch, regions) = selection::swept_regions(document, evaluation, self.feature)?;
        let sketch_plane = scene::sketch_plane(document, evaluation, sketch)?;
        let chosen = scene::chosen_regions(&emboss.regions, regions);
        let chosen: Vec<&SketchRegion> = regions
            .iter()
            .filter(|region| chosen.contains(&region.region.key()))
            .collect();
        let inside = chosen
            .iter()
            .find_map(|region| region.reference().anchor())?;
        let (on_face, normal) = match faces
            .iter()
            .map(|face| face_plane(solid, *face))
            .collect::<Option<Vec<Plane>>>()
        {
            Some(planes) => {
                let face = planes.first()?;
                (
                    face.to_world(face.to_local(sketch_plane.to_world(inside))),
                    face.normal(),
                )
            }
            None => {
                let face = solid.face(*faces.first()?)?;
                let outlines: Vec<Region> =
                    chosen.iter().map(|region| region.region.clone()).collect();
                let (stand, outward) =
                    wrapped_stand(&sketch_plane, &outlines, face.surface(), inside).ok()?;
                (stand, outward * face.sense().sign())
            }
        };
        let outward = match emboss.form {
            EmbossForm::Raised => normal,
            EmbossForm::Recessed => -normal,
        };
        Some(Gauge::line(
            Measured::EmbossDepth,
            (on_face, outward),
            1.0,
            value,
        ))
    }

    fn shown_evaluation(&self) -> &Evaluation {
        self.model
            .draft_evaluation_of(self.feature)
            .unwrap_or_else(|| self.model.evaluation())
    }

    fn reached_end(&self, side: ReachedSide) -> Option<ReachedEnd> {
        self.shown_evaluation()
            .feature(self.feature)?
            .result
            .as_deref()?
            .solid()?
            .reached()
            .iter()
            .find(|end| end.side == side)
            .copied()
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
        let middle = datum_tools::plane_middle(
            self.model.document(),
            self.shown_evaluation(),
            self.feature,
            &shown,
        )
        .unwrap_or_else(|| shown.origin());
        let base = middle - shown.normal() * value;
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

    fn blend(&self, blend: &Blend) -> Vec<Gauge> {
        self.blend_gauges(blend).unwrap_or_default()
    }

    fn blend_gauges(&self, blend: &Blend) -> Option<Vec<Gauge>> {
        let size = self.length(&blend.size)?;
        let source = handle_snap::before(self.model, self.feature)?;
        let solid = &source.solid;
        let edge = match blend.edges.first() {
            Some(first) => bodies::find_edge(source, first.name())?,
            None => *blend.resolve(self.model.document(), solid).ok()?.first()?,
        };
        let corner = Corner::of(solid, edge)?;
        let measured = || {
            let edges = blend
                .resolve(self.model.document(), solid)
                .unwrap_or_else(|_| vec![edge]);
            corner.measured(measured_face(solid, &edges, edge, blend.flipped)?)
        };
        let gauges = match (blend.kind, blend.chamfer_form()) {
            (BlendKind::Fillet, _) => {
                let half = corner.opening / 2.0;
                vec![corner.bisected(size, 1.0 / half.cos() - 1.0)]
            }
            (BlendKind::Chamfer, ChamferForm::Equal) => {
                vec![corner.bisected(size, (corner.opening / 2.0).sin())]
            }
            (BlendKind::Chamfer, ChamferForm::TwoDistances { second }) => {
                let (near, far) = measured()?;
                let second = self.length(second)?;
                vec![
                    corner.on_face(solid, near, Measured::BlendSize, size),
                    corner.on_face(solid, far, Measured::BlendSecond, second),
                ]
            }
            (BlendKind::Chamfer, ChamferForm::DistanceAngle { angle }) => {
                let (near, far) = measured()?;
                let angle = self.angle(angle)?;
                let first = corner.on_face(solid, near, Measured::BlendSize, size);
                let foot = first.track.at(size);
                let back = corner.back_from(solid, near, foot)?;
                let toward_far = far.inward - back * far.inward.dot(back);
                let turned = toward_far.try_normalize()?;
                let reach = corner.cut_length(solid, foot, (back, turned), angle, far);
                let radius = reach.or_else(|| Some(ARROW_POINTS * (self.per_point)(foot)?))?;
                vec![
                    first,
                    Gauge {
                        measured: Measured::BlendAngle,
                        track: Track::Turn(Swing {
                            foot,
                            radial: back,
                            normal: turned,
                            radius,
                        }),
                        mapping: Mapping::Linear {
                            scale: 1.0,
                            offset: 0.0,
                        },
                        value: angle,
                    },
                ]
            }
        };
        Some(gauges)
    }

    fn shell(&self, shell: &Shell) -> Option<Gauge> {
        let value = self.length(&shell.thickness)?;
        let source = handle_snap::before(self.model, self.feature)?;
        let solid = &source.solid;
        let face = shell.open.iter().find_map(|reference| {
            let face = reference.resolve(solid).ok()?;
            face_plane(solid, face).map(|_| face)
        })?;
        let outer = solid.face(face)?.outer_loop()?;
        let first = *solid.face_loop(outer)?.coedges().first()?;
        let coedge = solid.coedge(first)?;
        let edge = solid.edge(coedge.edge())?;
        let middle = edge.interval().middle();
        let at = edge.curve().point(middle);
        let along = edge.curve().evaluate(middle).first.try_normalize()? * coedge.sense().sign();
        let inward = outward(solid, face, at)?.cross(along).try_normalize()?;
        let (direction, scale) = match shell.side {
            ShellSide::Inside => (inward, 1.0),
            ShellSide::Outside => (-inward, 1.0),
            ShellSide::BothSides => (-inward, 0.5),
        };
        Some(Gauge::line(
            Measured::ShellThickness,
            (at, direction),
            scale,
            value,
        ))
    }

    fn shell_faces(&self, shell: &Shell) -> Vec<Gauge> {
        let Some(source) = handle_snap::before(self.model, self.feature) else {
            return Vec::new();
        };
        let solid = &source.solid;
        let (direction_sign, scale) = match shell.side {
            ShellSide::Inside => (-1.0, 1.0),
            ShellSide::Outside => (1.0, 1.0),
            ShellSide::BothSides => (1.0, 0.5),
        };
        shell
            .wall_resolutions(solid)
            .iter()
            .zip(&shell.faces)
            .enumerate()
            .filter_map(|(index, (resolution, own))| {
                let Resolution::One(face) = resolution else {
                    return None;
                };
                let value = self.length(&own.thickness)?;
                let (at, outward) = face_stand(solid, *face)?;
                Some(Gauge::line(
                    Measured::ShellFaceThickness(index),
                    (at, outward * direction_sign),
                    scale,
                    value,
                ))
            })
            .collect()
    }

    fn rib(&self, rib: &Rib) -> Option<Gauge> {
        let value = self.length(&rib.thickness)?;
        let stand = rib_stand(self.model.evaluation(), rib)?;
        Some(Gauge::line(
            Measured::RibThickness,
            (stand.base, stand.direction),
            stand.share,
            value,
        ))
    }

    fn coil_span(&self, length: &CoilLength) -> Option<(f64, f64)> {
        let (pitch, turns) = match length {
            CoilLength::TurnsAndPitch { turns, pitch } => (self.length(pitch)?, self.count(turns)?),
            CoilLength::HeightAndTurns { height, turns } => {
                let turns = self.count(turns)?;
                (self.length(height)? / turns, turns)
            }
            CoilLength::HeightAndPitch { height, pitch } => {
                let pitch = self.length(pitch)?;
                (pitch, self.length(height)? / pitch)
            }
        };
        (pitch.is_finite() && pitch > 0.0 && turns.is_finite() && turns > 0.0)
            .then_some((pitch, turns))
    }

    fn coil(&self, coil: &Coil) -> Vec<Gauge> {
        let (Some(diameter), Some(size), Some(taper)) = (
            self.length(&coil.diameter),
            self.length(&coil.size),
            self.angle(&coil.taper),
        ) else {
            return Vec::new();
        };
        let Some((pitch, turns)) = self.coil_span(&coil.length) else {
            return Vec::new();
        };
        let Some(frame) = displayed_axis(self.model.evaluation(), self.feature, &coil.axis)
            .and_then(|axis| coil.frame(axis))
        else {
            return Vec::new();
        };
        let radius = diameter / 2.0;
        let centre = match coil.placement {
            CoilPlacement::Inside => radius - size / 2.0,
            CoilPlacement::Centred => radius,
            CoilPlacement::Outside => radius + size / 2.0,
        };
        let slope = pitch * taper.to_radians().tan() / TAU;
        let sense = coil.turning.sign();
        let sweep = TAU * turns;
        let wire_at = |angle: f64| {
            let around = frame.x_axis() * angle.cos() + frame.y_axis() * (sense * angle.sin());
            frame.origin() + around * (centre + slope * angle)
        };
        let rise = |angle: f64| frame.normal() * (pitch * angle / TAU);
        let half_turn = PI.min(sweep);
        let mut gauges = vec![
            Gauge::line(
                Measured::CoilDiameter,
                (frame.origin(), frame.x_axis()),
                0.5,
                diameter,
            ),
            Gauge::line(
                Measured::CoilSection,
                (wire_at(half_turn) + rise(half_turn), frame.normal()),
                0.5,
                size,
            ),
        ];
        if coil.length.pitch().is_some() {
            gauges.push(Gauge::line(
                Measured::CoilPitch,
                (wire_at(TAU), frame.normal()),
                1.0,
                pitch,
            ));
        }
        if coil.length.height().is_some() {
            gauges.push(Gauge::line(
                Measured::CoilHeight,
                (wire_at(sweep), frame.normal()),
                1.0,
                pitch * turns,
            ));
        }
        gauges
    }

    fn spiral_span(&self, length: &CoilLength, inner: f64) -> Option<(f64, f64)> {
        let width = |outer: &Expression| Some(0.5 * (self.length(outer)? - inner));
        let (pitch, turns) = match length {
            CoilLength::TurnsAndPitch { turns, pitch } => (self.length(pitch)?, self.count(turns)?),
            CoilLength::HeightAndTurns { height, turns } => {
                let turns = self.count(turns)?;
                (width(height)? / turns, turns)
            }
            CoilLength::HeightAndPitch { height, pitch } => {
                let pitch = self.length(pitch)?;
                (pitch, width(height)? / pitch)
            }
        };
        (pitch.is_finite() && pitch > 0.0 && turns.is_finite() && turns > 0.0)
            .then_some((pitch, turns))
    }

    fn spiral(&self, helix: &Helix, diameter: f64, start: f64) -> Vec<Gauge> {
        let Some((pitch, turns)) = self.spiral_span(&helix.length, diameter) else {
            return Vec::new();
        };
        let Some(frame) = displayed_axis(self.model.evaluation(), self.feature, &helix.axis)
            .and_then(|axis| helix.frame(axis))
        else {
            return Vec::new();
        };
        let spec = SpiralSpec {
            frame,
            radius: diameter / 2.0,
            pitch,
            turns,
            hand: helix.turning.hand(),
            start_angle: start.to_radians(),
        };
        let Some(started) = spec.start_frame() else {
            return Vec::new();
        };
        let mut gauges = vec![Gauge::line(
            Measured::CoilDiameter,
            (started.origin(), started.x_axis()),
            0.5,
            diameter,
        )];
        if helix.length.pitch().is_some()
            && let Some(first) = spec.start()
        {
            gauges.push(Gauge::line(
                Measured::CoilPitch,
                (first, started.x_axis()),
                1.0,
                pitch,
            ));
        }
        if helix.length.height().is_some()
            && let Some(end) = spec.point_at(spec.sweep())
            && let Some(outward) = (end - started.origin()).try_normalize()
        {
            gauges.push(Gauge::line(
                Measured::CoilHeight,
                (started.origin(), outward),
                0.5,
                2.0 * spec.outer_radius(),
            ));
        }
        gauges
    }

    fn helix(&self, helix: &Helix) -> Vec<Gauge> {
        let (Some(diameter), Some(start)) =
            (self.length(&helix.diameter), self.angle(&helix.start))
        else {
            return Vec::new();
        };
        if helix.is_flat() {
            return self.spiral(helix, diameter, start);
        }
        let Some(taper) = self.angle(&helix.taper) else {
            return Vec::new();
        };
        let Some((pitch, turns)) = self.coil_span(&helix.length) else {
            return Vec::new();
        };
        let Some(frame) = displayed_axis(self.model.evaluation(), self.feature, &helix.axis)
            .and_then(|axis| helix.frame(axis))
        else {
            return Vec::new();
        };
        let spec = HelixSpec {
            frame,
            radius: diameter / 2.0,
            pitch,
            turns,
            taper: taper.to_radians(),
            hand: helix.turning.hand(),
            start_angle: start.to_radians(),
        };
        let Some(started) = spec.start_frame() else {
            return Vec::new();
        };
        let mut gauges = vec![Gauge::line(
            Measured::CoilDiameter,
            (started.origin(), started.x_axis()),
            0.5,
            diameter,
        )];
        if helix.length.pitch().is_some()
            && let Some(turned) = spec.point_at(TAU)
        {
            gauges.push(Gauge::line(
                Measured::CoilPitch,
                (turned, frame.normal()),
                1.0,
                pitch,
            ));
        }
        if helix.length.height().is_some()
            && let Some(end) = spec.point_at(TAU * turns)
        {
            gauges.push(Gauge::line(
                Measured::CoilHeight,
                (end, frame.normal()),
                1.0,
                pitch * turns,
            ));
        }
        gauges
    }

    fn pipe(&self, pipe: &Pipe) -> Vec<Gauge> {
        let document = self.model.document();
        let Some(feature) = document.feature(self.feature) else {
            return Vec::new();
        };
        let (Some(size), Some(bend)) = (self.length(&pipe.size), self.length(&pipe.bend)) else {
            return Vec::new();
        };
        let Some(stand) = pipe_stand(document, self.model.evaluation(), feature, pipe, bend) else {
            return Vec::new();
        };
        let section = stand.section;
        let across = section.x_axis();
        let (outer, share) = match pipe.shape {
            PipeShape::Round | PipeShape::Square => (0.5, 1.0),
            PipeShape::Triangle => (1.0 / 3.0, TRIANGLE_WALL_SHARE),
        };
        let mut gauges = vec![Gauge::line(
            Measured::PipeSize,
            (section.origin(), across),
            outer,
            size,
        )];
        if let Some(wall) = pipe.wall.as_ref().and_then(|wall| self.length(wall)) {
            let rim = section.origin() + across * (size * outer);
            gauges.push(Gauge::line(Measured::PipeWall, (rim, -across), share, wall));
        }
        gauges.extend(stand.corner.and_then(|corner| bend_gauge(&corner, bend)));
        gauges
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
                let (offset, level) = match end? {
                    ExtrudeEnd::UpToFace { target, offset } => (
                        offset,
                        caditor_document::displayed_start_offset(
                            self.model.evaluation(),
                            self.feature,
                            &plane,
                            target,
                        )?,
                    ),
                    ExtrudeEnd::UpToNext { offset } => (
                        offset,
                        self.reached_end(reached_side(reach))?.level_over(&plane)?,
                    ),
                    ExtrudeEnd::Distance(_)
                    | ExtrudeEnd::ThroughAll
                    | ExtrudeEnd::UpToSurface { .. } => return None,
                };
                let value = offset
                    .as_deref()
                    .map_or(Some(0.0), |offset| self.length(offset))?;
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
        let mut gauges: Vec<Gauge> = primitive_arrows(&primitive.shape)
            .iter()
            .filter_map(|arrow| placed.gauge(*arrow))
            .collect();
        gauges.extend(placed.sides_gauge());
        gauges
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
            PatternKind::Curve(curve) => self.curve(curve, centre),
            PatternKind::Points(_) => Vec::new(),
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
        if count < 2.0 {
            return Vec::new();
        }
        let scale = match direction.measured {
            LinearSpacing::BetweenCopies => count - 1.0,
            LinearSpacing::Total => 1.0,
        };
        vec![Gauge::line(
            Measured::PatternSpacing(index),
            (centre, along),
            scale,
            spacing,
        )]
    }

    fn curve(&self, curve: &CurvePattern, centre: Point3) -> Vec<Gauge> {
        self.curve_gauge(curve, centre).into_iter().collect()
    }

    fn curve_gauge(&self, curve: &CurvePattern, centre: Point3) -> Option<Gauge> {
        let sketch = self
            .model
            .displayed_sketch(self.model.document().feature(curve.sketch)?)?;
        let path = PatternPath::of(&sketch)?;
        let count = self.count(&curve.count)?.round();
        (count >= 2.0).then_some(())?;
        let steps = count - 1.0;
        let length = path.length();
        let (measured, mapping, value, now) = match curve.measured {
            CurveSpacing::Distance => {
                let spacing = self.length(&curve.spacing)?;
                (spacing > 0.0).then_some(())?;
                (
                    Measured::CurveSpacing,
                    Mapping::Linear {
                        scale: steps,
                        offset: 0.0,
                    },
                    spacing,
                    spacing * steps,
                )
            }
            CurveSpacing::Spread => {
                let less = if path.is_closed() { 0.0 } else { 1.0 };
                let mapping = Mapping::Reciprocal { turn: length, less };
                (
                    Measured::CurveCount,
                    mapping,
                    count,
                    mapping.position(count),
                )
            }
        };
        let reaches = (0..=CURVE_SAMPLES)
            .map(|index| length * f64::from(index) / f64::from(CURVE_SAMPLES))
            .chain(std::iter::once(now));
        let track = CurveTrack::new(reaches.filter_map(|reach| {
            let stop = path.copy_at(reach, curve.reversed, curve.orientation)?;
            Some(TrackPoint {
                position: reach,
                point: stop.placement.apply_point(centre),
            })
        }))?;
        Some(Gauge {
            measured,
            track: Track::Curve(track),
            mapping,
            value,
        })
    }

    fn circular(&self, circular: &CircularPattern, centre: Point3) -> Vec<Gauge> {
        let found = || {
            let axis = circular.axis.as_ref()?;
            let ray = displayed_axis(self.model.evaluation(), self.feature, axis)?;
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

#[derive(Debug, Clone, Copy, PartialEq)]
struct Side {
    face: FaceId,
    normal: Vector3,
    inward: Vector3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Corner {
    at: Point3,
    along: Vector3,
    sides: [Side; 2],
    opening: f64,
    convex: bool,
}

impl Corner {
    fn of(solid: &Solid, edge: EdgeId) -> Option<Self> {
        let edge = solid.edge(edge)?;
        let middle = edge.interval().middle();
        let at = edge.curve().point(middle);
        let along = edge.curve().evaluate(middle).first.try_normalize()?;
        let mut sides = edge.coedges().iter().filter_map(|coedge| {
            let sense = solid.coedge(*coedge)?.sense().sign();
            let face = solid.coedge_face(*coedge)?;
            let normal = outward(solid, face, at)?;
            Some(Side {
                face,
                normal,
                inward: normal.cross(along * sense).try_normalize()?,
            })
        });
        let (first, second) = (sides.next()?, sides.next()?);
        let opening = first.normal.dot(second.normal).clamp(-1.0, 1.0).acos();
        (opening >= SMALLEST_OPENING).then_some(())?;
        Some(Self {
            at,
            along,
            sides: [first, second],
            opening,
            convex: second.inward.dot(first.normal) < 0.0,
        })
    }

    fn bisected(&self, value: f64, per_size: f64) -> Gauge {
        let [first, second] = self.sides;
        let bisector = (first.normal + second.normal)
            .try_normalize()
            .unwrap_or(first.normal);
        let direction = if self.convex { -bisector } else { bisector };
        Gauge::line(Measured::BlendSize, (self.at, direction), per_size, value)
    }

    fn measured(&self, face: FaceId) -> Option<(Side, Side)> {
        let [first, second] = self.sides;
        match face {
            face if face == first.face => Some((first, second)),
            face if face == second.face => Some((second, first)),
            _ => None,
        }
    }

    fn track_on(&self, solid: &Solid, side: Side, now: f64) -> Track {
        let straight = Track::Line {
            base: self.at,
            direction: side.inward,
        };
        if face_plane(solid, side.face).is_some() {
            return straight;
        }
        section_track(solid, side, self.at, self.along, now).map_or(straight, Track::Curve)
    }

    fn on_face(&self, solid: &Solid, side: Side, measured: Measured, value: f64) -> Gauge {
        Gauge {
            measured,
            track: self.track_on(solid, side, value),
            mapping: Mapping::Linear {
                scale: 1.0,
                offset: 0.0,
            },
            value,
        }
    }

    fn back_from(&self, solid: &Solid, near: Side, foot: Point3) -> Option<Vector3> {
        if face_plane(solid, near.face).is_some() {
            return Some(-near.inward);
        }
        let back = self
            .along
            .cross(outward(solid, near.face, foot)?)
            .try_normalize()?;
        Some(if back.dot(self.at - foot) < 0.0 {
            -back
        } else {
            back
        })
    }

    fn cut_length(
        &self,
        solid: &Solid,
        foot: Point3,
        (back, turned): (Vector3, Vector3),
        angle: f64,
        far: Side,
    ) -> Option<f64> {
        let (sin, cos) = angle.to_radians().sin_cos();
        let direction = back * cos + turned * sin;
        if let Track::Curve(track) = self.track_on(solid, far, 0.0) {
            return track.first_crossing(foot, direction);
        }
        let section = back.cross(far.inward);
        let from_edge = foot - self.at;
        let reach =
            -from_edge.cross(far.inward).dot(section) / direction.cross(far.inward).dot(section);
        let landing = from_edge + direction * reach;
        (reach.is_finite() && reach > OFF_THE_LINE && landing.dot(far.inward) > OFF_THE_LINE)
            .then_some(reach)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct HoleTop {
    plane: Plane,
    top: Point3,
    down: Vector3,
    bore: Option<f64>,
    heads: Option<f64>,
}

impl HoleTop {
    fn slot(&self, hole: &Hole, builder: &Builder<'_>) -> Option<(Vector3, Vector3, f64)> {
        let HoleShape::Slot { length, angle } = &hole.shape else {
            return None;
        };
        let (sin, cos) = builder.angle(angle)?.to_radians().sin_cos();
        let along = self.plane.x_axis() * cos + self.plane.y_axis() * sin;
        let across = self.plane.y_axis() * cos - self.plane.x_axis() * sin;
        Some((along, across, builder.length(length)?))
    }

    fn rim(&self, hole: &Hole, builder: &Builder<'_>) -> (Point3, Vector3) {
        match self.slot(hole, builder) {
            Some((along, across, length)) => (self.top + along * (length / 4.0), across),
            None => (
                self.top,
                (self.plane.x_axis() + self.plane.y_axis()).normalize(),
            ),
        }
    }

    fn wall(&self, hole: &Hole, builder: &Builder<'_>) -> (Point3, Vector3) {
        match self.slot(hole, builder) {
            Some((along, across, length)) => (self.top + along * (length / 4.0), -across),
            None => (
                self.top,
                (self.plane.x_axis() - self.plane.y_axis()).normalize(),
            ),
        }
    }
}

fn bend_gauge(corner: &PipeCorner, bend: f64) -> Option<Gauge> {
    match &corner.middles {
        BendMiddles::AlongBisector => {
            let bisector = (corner.outgoing - corner.incoming).try_normalize()?;
            let turn = corner.incoming.angle_between(corner.outgoing);
            let scale = 1.0 / (turn / 2.0).cos() - 1.0;
            (scale.is_finite() && scale > SMALLEST_OPENING)
                .then(|| Gauge::line(Measured::PipeBend, (corner.point, bisector), scale, bend))
        }
        BendMiddles::Sampled(middles) => {
            let track = CurveTrack::new(middles.iter().map(|middle| TrackPoint {
                position: middle.radius,
                point: middle.point,
            }))?;
            Some(Gauge {
                measured: Measured::PipeBend,
                track: Track::Curve(track),
                mapping: Mapping::Linear {
                    scale: 1.0,
                    offset: 0.0,
                },
                value: bend,
            })
        }
    }
}

fn section_track(
    solid: &Solid,
    side: Side,
    at: Point3,
    along: Vector3,
    now: f64,
) -> Option<CurveTrack> {
    let surface = solid.face(side.face)?.surface();
    let reach = solid.bounding_box()?.diagonal();
    let mut distances: Vec<f64> = (1..=SECTION_SAMPLES)
        .map(|index| reach * f64::from(index) / f64::from(SECTION_SAMPLES))
        .chain((now > 0.0).then_some(now))
        .collect();
    distances.sort_by(f64::total_cmp);
    let mut points = vec![TrackPoint {
        position: 0.0,
        point: at,
    }];
    let mut heading = side.inward;
    let mut hint = None;
    for distance in distances {
        let Some((point, uv)) = chord_foot(surface, (at, along), heading, distance, hint) else {
            break;
        };
        heading = (point - at).try_normalize().unwrap_or(heading);
        hint = Some(uv);
        points.push(TrackPoint {
            position: distance,
            point,
        });
    }
    CurveTrack::new(points)
}

fn chord_foot(
    surface: &Surface,
    (at, along): (Point3, Vector3),
    heading: Vector3,
    distance: f64,
    hint: Option<Point2>,
) -> Option<(Point3, Point2)> {
    let fit = FOOT_FIT * distance.max(1.0);
    let mut guess = at + heading * distance;
    let mut uv = hint;
    for _ in 0..FOOT_ITERATIONS {
        let found = surface.project(guess, uv);
        uv = Some(found);
        let on = surface.point(found.x, found.y);
        let flat = on - along * (on - at).dot(along);
        guess = at + (flat - at).try_normalize()? * distance;
        if on.distance(guess) <= fit {
            return Some((guess, found));
        }
    }
    None
}

fn face_stand(solid: &Solid, face: FaceId) -> Option<(Point3, Vector3)> {
    if let Some(plane) = face_plane(solid, face) {
        let middle = hole_tools::middle_of(solid, face, &plane)?;
        return Some((plane.to_world(middle), plane.normal()));
    }
    rim_stand(solid, face)
}

fn round_tangent(solid: &Solid, face: FaceId) -> Option<(FaceId, Plane)> {
    let round = matches!(
        solid.face(face)?.surface(),
        Surface::Cylinder(_) | Surface::Cone(_)
    );
    let (at, normal) = rim_stand(solid, face).filter(|_| round)?;
    Some((face, Plane::new(at, normal)?))
}

fn rim_stand(solid: &Solid, face: FaceId) -> Option<(Point3, Vector3)> {
    let outer = solid.face(face)?.outer_loop()?;
    let first = *solid.face_loop(outer)?.coedges().first()?;
    let edge = solid.edge(solid.coedge(first)?.edge())?;
    let at = edge.curve().point(edge.interval().middle());
    Some((at, outward(solid, face, at)?))
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
    TopLength,
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
    const WEDGE: [PrimitiveArrow; 4] = [
        arrow(0, Along::X, Level::Bottom),
        arrow(1, Along::Y, Level::Bottom),
        arrow(2, Along::Up, Level::Middle),
        PrimitiveArrow {
            index: 3,
            along: Along::X,
            level: Level::Top,
            reaches: Reaches::TopLength,
        },
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
            (Along::X, Reaches::TopLength) => low_x + *values.get(3)?,
            (Along::X, Reaches::Side) => low_x + length,
            (Along::Y, _) => low_y + width,
            (Along::Up, _) => low_z + height,
        })
    }

    fn sides_gauge(&self) -> Option<Gauge> {
        if !matches!(self.shape, PrimitiveShape::Prism { .. }) {
            return None;
        }
        let sides = *self.values.first()?;
        let size = footprint(self.shape, &self.values)?;
        let [low_x, low_y, low_z] = self.low(size);
        let [diameter, _, height] = size;
        let radius = diameter / 2.0;
        let middle = Point2::new(low_x + radius, low_y + radius);
        Some(Gauge {
            measured: Measured::PrismSides,
            track: Track::Turn(Swing {
                foot: self.plane.to_world(middle) + self.up * (low_z + height),
                radial: -self.plane.y_axis(),
                normal: self.plane.x_axis(),
                radius,
            }),
            mapping: Mapping::Reciprocal {
                turn: HALF_TURN_DEGREES,
                less: 0.0,
            },
            value: sides,
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
