use caditor_document::{
    Blend, BlendKind, ChamferForm, Coil, CoilLength, CurvePattern, CurveSpacing, Draft, Edit,
    Emboss, Evaluation, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureKind, FeatureResult,
    Helix, HoleDepth, LinearDirection, LinearSpacing, Move, MoveAxis, MoveFace, ParameterValues,
    PatternKind, Pipe, PrimitiveShape, Revolve, RevolveExtent, Rib, Scale, SizeRule, SolidFeature,
    Sweep, ThreadLength, Transaction, coil_size_name, origin_feature, pipe_size_name,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Aabb, Point3};
use caditor_kernel::{Face, Solid};

use crate::{
    coil_panel, draft_panel, emboss_panel,
    feature_fields::Rule,
    field::{self, CheckError},
    helix_panel, helix_tools,
    model::Model,
    move_face_panel,
    move_manipulator::Reach,
    pipe_panel, reversing, rib_panel, solid_panel,
    value_shapes::{self, Drawn},
};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum ValueSlot {
    Extrude(Reach),
    RevolveAngle,
    RevolveTotal,
    RevolveForward,
    RevolveBackward,
    SweepTwist,
    SweepScale,
    PipeSize,
    PipeWall,
    PipeBend,
    CoilDiameter,
    CoilSection,
    CoilTaper,
    CoilTurns,
    CoilPitch,
    CoilHeight,
    HelixDiameter,
    HelixTaper,
    HelixTurns,
    HelixPitch,
    HelixHeight,
    HelixStart,
    RibThickness,
    RibDraft,
    EmbossDepth,
    EmbossDraft,
    MoveFaceDistance,
    MoveFaceAngle,
    DraftAngle,
    DraftBeyond,
    CurveSpacing,
    ScaleFactor,
    BlendSize,
    ShellThickness,
    HoleDiameter,
    HoleDepth,
    Count,
    Spacing,
    SecondCount,
    SecondSpacing,
    PatternAngle,
    ChamferSecondDistance,
    ChamferAngle,
    OffsetDistance,
    Primitive(PrimitiveSize),
    MoveOffset(MoveAxis),
    ThreadDepth,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrimitiveSize {
    Length,
    Width,
    Height,
    Diameter,
    TubeDiameter,
    BottomDiameter,
    TopDiameter,
    TopLength,
    Sides,
}

impl PrimitiveSize {
    fn named(what: &str) -> Option<Self> {
        Some(match what {
            "length" => Self::Length,
            "width" => Self::Width,
            "height" => Self::Height,
            "diameter" => Self::Diameter,
            "tube diameter" => Self::TubeDiameter,
            "bottom diameter" => Self::BottomDiameter,
            "top diameter" => Self::TopDiameter,
            "top length" => Self::TopLength,
            "number of sides" => Self::Sides,
            _ => return None,
        })
    }

    pub fn caption(self) -> &'static str {
        match self {
            Self::Length => "Length",
            Self::Width => "Width",
            Self::Height => "Height",
            Self::Diameter => "Diameter",
            Self::TubeDiameter => "Tube diameter",
            Self::BottomDiameter => "Bottom diameter",
            Self::TopDiameter => "Top diameter",
            Self::TopLength => "Top length",
            Self::Sides => "Number of sides",
        }
    }
}

fn move_caption(axis: MoveAxis) -> &'static str {
    match axis {
        MoveAxis::X => "Move along X",
        MoveAxis::Y => "Move along Y",
        MoveAxis::Z => "Move along Z",
    }
}

impl ValueSlot {
    pub fn words(self) -> &'static str {
        match self {
            Self::Extrude(reach) => reach.field_caption(),
            Self::RevolveAngle => "Angle",
            Self::RevolveTotal | Self::PatternAngle => "Total angle",
            Self::RevolveForward => "Forward angle",
            Self::RevolveBackward => "Backward angle",
            Self::SweepTwist => solid_panel::TWIST,
            Self::SweepScale => solid_panel::END_SCALE,
            Self::PipeSize => "Size",
            Self::PipeWall => pipe_panel::WALL_THICKNESS,
            Self::PipeBend => pipe_panel::BEND_RADIUS,
            Self::CoilDiameter | Self::HelixDiameter => coil_panel::DIAMETER,
            Self::CoilSection => "Section",
            Self::CoilTaper | Self::HelixTaper => coil_panel::TAPER,
            Self::CoilTurns | Self::HelixTurns => coil_panel::TURNS,
            Self::CoilPitch | Self::HelixPitch => coil_panel::PITCH,
            Self::CoilHeight | Self::HelixHeight => coil_panel::HEIGHT,
            Self::HelixStart => helix_panel::START_ANGLE,
            Self::RibThickness => rib_panel::THICKNESS,
            Self::RibDraft => rib_panel::DRAFT,
            Self::EmbossDepth => emboss_panel::DEPTH,
            Self::EmbossDraft => emboss_panel::DRAFT,
            Self::MoveFaceDistance => move_face_panel::DISTANCE,
            Self::MoveFaceAngle => move_face_panel::ANGLE,
            Self::DraftAngle => draft_panel::ANGLE,
            Self::DraftBeyond => draft_panel::BEYOND,
            Self::CurveSpacing => "Spacing",
            Self::ScaleFactor => "Factor",
            Self::BlendSize => "Size",
            Self::ShellThickness => "Thickness",
            Self::HoleDiameter => "Diameter",
            Self::HoleDepth => "Depth",
            Self::Count => COUNT,
            Self::Spacing => "Spacing",
            Self::SecondCount => SECOND_COUNT,
            Self::SecondSpacing => "Second spacing",
            Self::ChamferSecondDistance => "Second distance",
            Self::ChamferAngle => "Chamfer angle",
            Self::OffsetDistance => "Offset distance",
            Self::Primitive(size) => size.caption(),
            Self::MoveOffset(axis) => move_caption(axis),
            Self::ThreadDepth => "Thread depth",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureValue {
    pub feature: FeatureId,
    pub slot: ValueSlot,
    pub caption: &'static str,
    pub expression: Expression,
    pub dimension: Dimension,
    pub rule: Rule,
    pub drawn: Drawn,
    pub anchor: Point3,
}

struct Found {
    slot: ValueSlot,
    caption: &'static str,
    expression: Expression,
    dimension: Dimension,
    rule: Rule,
}

const COUNT: &str = "Count";
const SECOND_COUNT: &str = "Second count";

fn found(
    slot: ValueSlot,
    caption: &'static str,
    expression: &Expression,
    (dimension, rule): (Dimension, Rule),
) -> Found {
    Found {
        slot,
        caption,
        expression: expression.clone(),
        dimension,
        rule,
    }
}

fn degrees(parameters: &ParameterValues, angle: &Expression) -> f64 {
    parameters
        .evaluate_expression(angle)
        .map_or(0.0, |value| value.value)
}

fn extrude_values(extrude: &Extrude) -> Vec<Found> {
    let length = |rule| (Dimension::LENGTH, rule);
    let end = |reach: Reach, end: &ExtrudeEnd, rule| match end {
        ExtrudeEnd::Distance(distance) => Some(found(
            ValueSlot::Extrude(reach),
            reach.field_caption(),
            distance,
            length(rule),
        )),
        ExtrudeEnd::ThroughAll
        | ExtrudeEnd::UpToNext { .. }
        | ExtrudeEnd::UpToFace { .. }
        | ExtrudeEnd::UpToSurface { .. } => None,
    };
    match &extrude.extent {
        ExtrudeExtent::OneSide { end: only, .. } => end(Reach::Only, only, Rule::FlipWhenNegative)
            .into_iter()
            .collect(),
        ExtrudeExtent::Symmetric { distance } => vec![found(
            ValueSlot::Extrude(Reach::Symmetric),
            solid_panel::TOTAL_DISTANCE,
            distance,
            length(Rule::AboveZero),
        )],
        ExtrudeExtent::TwoSides { forward, backward } => [
            end(Reach::Forward, forward, Rule::AboveZero),
            end(Reach::Backward, backward, Rule::AboveZero),
        ]
        .into_iter()
        .flatten()
        .collect(),
    }
}

fn revolve_values(parameters: &ParameterValues, revolve: &Revolve) -> Vec<Found> {
    let angle = |rule| (Dimension::ANGLE, rule);
    match &revolve.extent {
        RevolveExtent::Full
        | RevolveExtent::UpTo { .. }
        | RevolveExtent::UpToNext { .. }
        | RevolveExtent::UpToSurface { .. } => Vec::new(),
        RevolveExtent::OneSide { angle: turned, .. } => vec![found(
            ValueSlot::RevolveAngle,
            "Angle",
            turned,
            angle(Rule::Turn),
        )],
        RevolveExtent::Symmetric { angle: turned } => vec![found(
            ValueSlot::RevolveTotal,
            "Total angle",
            turned,
            angle(Rule::Turn),
        )],
        RevolveExtent::TwoSides { forward, backward } => vec![
            found(
                ValueSlot::RevolveForward,
                "Forward",
                forward,
                angle(Rule::TurnBeside(degrees(parameters, backward))),
            ),
            found(
                ValueSlot::RevolveBackward,
                "Backward",
                backward,
                angle(Rule::TurnBeside(degrees(parameters, forward))),
            ),
        ],
    }
}

fn direction_values(
    direction: &LinearDirection,
    [count, spacing]: [ValueSlot; 2],
    [count_caption, spacing_caption]: [&'static str; 2],
) -> [Found; 2] {
    [
        found(
            count,
            count_caption,
            &direction.count,
            (Dimension::NONE, Rule::Count),
        ),
        found(
            spacing,
            spacing_caption,
            &direction.spacing,
            (Dimension::LENGTH, Rule::AboveZeroOrReverse),
        ),
    ]
}

fn spacing_caption(direction: &LinearDirection, first: bool) -> &'static str {
    match (direction.measured, first) {
        (LinearSpacing::BetweenCopies, true) => "Spacing",
        (LinearSpacing::Total, true) => "Total length",
        (LinearSpacing::BetweenCopies, false) => "Second spacing",
        (LinearSpacing::Total, false) => "Second total length",
    }
}

fn pipe_size_caption(shape: caditor_kernel::PipeShape) -> &'static str {
    match pipe_size_name(shape) {
        "diameter" => "Diameter",
        "width" => "Width",
        "side length" => "Side length",
        _ => "Size",
    }
}

fn coil_size_caption(shape: caditor_kernel::CoilShape) -> &'static str {
    match coil_size_name(shape) {
        "diameter" => "Diameter",
        "width" => "Width",
        _ => "Section",
    }
}

fn sweep_values(sweep: &Sweep) -> Vec<Found> {
    vec![
        found(
            ValueSlot::SweepTwist,
            solid_panel::TWIST,
            &sweep.twist,
            (Dimension::ANGLE, Rule::Any),
        ),
        found(
            ValueSlot::SweepScale,
            solid_panel::END_SCALE,
            &sweep.scale,
            (Dimension::NONE, Rule::AboveZero),
        ),
    ]
}

fn pipe_values(pipe: &Pipe) -> Vec<Found> {
    let mut values = vec![found(
        ValueSlot::PipeSize,
        pipe_size_caption(pipe.shape),
        &pipe.size,
        (Dimension::LENGTH, Rule::AboveZero),
    )];

    if let Some(wall) = &pipe.wall {
        values.push(found(
            ValueSlot::PipeWall,
            pipe_panel::WALL_THICKNESS,
            wall,
            (Dimension::LENGTH, Rule::AboveZero),
        ));
    }
    values.push(found(
        ValueSlot::PipeBend,
        pipe_panel::BEND_RADIUS,
        &pipe.bend,
        (Dimension::LENGTH, Rule::ZeroOrMore),
    ));
    values
}

fn coil_length_values(
    length: &CoilLength,
    [turns_slot, pitch_slot, height_slot]: [ValueSlot; 3],
    height_caption: &'static str,
) -> Vec<Found> {
    match length {
        CoilLength::TurnsAndPitch { turns, pitch } => vec![
            found(
                turns_slot,
                coil_panel::TURNS,
                turns,
                (Dimension::NONE, Rule::CoilTurns),
            ),
            found(
                pitch_slot,
                coil_panel::PITCH,
                pitch,
                (Dimension::LENGTH, Rule::AboveZero),
            ),
        ],
        CoilLength::HeightAndTurns { height, turns } => vec![
            found(
                height_slot,
                height_caption,
                height,
                (Dimension::LENGTH, Rule::AboveZero),
            ),
            found(
                turns_slot,
                coil_panel::TURNS,
                turns,
                (Dimension::NONE, Rule::CoilTurns),
            ),
        ],
        CoilLength::HeightAndPitch { height, pitch } => vec![
            found(
                height_slot,
                height_caption,
                height,
                (Dimension::LENGTH, Rule::AboveZero),
            ),
            found(
                pitch_slot,
                coil_panel::PITCH,
                pitch,
                (Dimension::LENGTH, Rule::AboveZero),
            ),
        ],
    }
}

fn coil_values(coil: &Coil) -> Vec<Found> {
    let mut values = vec![
        found(
            ValueSlot::CoilDiameter,
            coil_panel::DIAMETER,
            &coil.diameter,
            (Dimension::LENGTH, Rule::AboveZero),
        ),
        found(
            ValueSlot::CoilSection,
            coil_size_caption(coil.shape),
            &coil.size,
            (Dimension::LENGTH, Rule::AboveZero),
        ),
        found(
            ValueSlot::CoilTaper,
            coil_panel::TAPER,
            &coil.taper,
            (Dimension::ANGLE, Rule::CoilTaper),
        ),
    ];

    values.extend(coil_length_values(
        &coil.length,
        [
            ValueSlot::CoilTurns,
            ValueSlot::CoilPitch,
            ValueSlot::CoilHeight,
        ],
        coil_panel::HEIGHT,
    ));
    values
}

fn helix_values(helix: &Helix) -> Vec<Found> {
    let flat = helix.is_flat();
    let mut values = vec![found(
        ValueSlot::HelixDiameter,
        if flat {
            helix_panel::INNER_DIAMETER
        } else {
            coil_panel::DIAMETER
        },
        &helix.diameter,
        (Dimension::LENGTH, Rule::AboveZero),
    )];

    if !flat {
        values.push(found(
            ValueSlot::HelixTaper,
            coil_panel::TAPER,
            &helix.taper,
            (Dimension::ANGLE, Rule::CoilTaper),
        ));
    }
    values.extend(coil_length_values(
        &helix.length,
        [
            ValueSlot::HelixTurns,
            ValueSlot::HelixPitch,
            ValueSlot::HelixHeight,
        ],
        if flat {
            helix_panel::OUTER_DIAMETER
        } else {
            coil_panel::HEIGHT
        },
    ));
    if helix.start_point.is_none() {
        values.push(found(
            ValueSlot::HelixStart,
            helix_panel::START_ANGLE,
            &helix.start,
            (Dimension::ANGLE, Rule::Any),
        ));
    }
    values
}

fn rib_values(rib: &Rib) -> Vec<Found> {
    vec![
        found(
            ValueSlot::RibThickness,
            rib_panel::THICKNESS,
            &rib.thickness,
            (Dimension::LENGTH, Rule::AboveZero),
        ),
        found(
            ValueSlot::RibDraft,
            rib_panel::DRAFT,
            &rib.draft,
            (Dimension::ANGLE, Rule::RibDraft),
        ),
    ]
}

fn emboss_values(emboss: &Emboss) -> Vec<Found> {
    vec![
        found(
            ValueSlot::EmbossDepth,
            emboss_panel::DEPTH,
            &emboss.depth,
            (Dimension::LENGTH, Rule::AboveZero),
        ),
        found(
            ValueSlot::EmbossDraft,
            emboss_panel::DRAFT,
            &emboss.draft,
            (Dimension::ANGLE, Rule::EmbossDraft),
        ),
    ]
}

fn move_face_values(moving: &MoveFace) -> Vec<Found> {
    let mut values = vec![found(
        ValueSlot::MoveFaceDistance,
        move_face_panel::DISTANCE,
        &moving.distance,
        (Dimension::LENGTH, Rule::Any),
    )];

    if moving.axis.is_some() {
        values.push(found(
            ValueSlot::MoveFaceAngle,
            move_face_panel::ANGLE,
            &moving.angle,
            (Dimension::ANGLE, Rule::Any),
        ));
    }
    values
}

fn draft_values(draft: &Draft) -> Vec<Found> {
    let mut values = vec![found(
        ValueSlot::DraftAngle,
        draft_panel::ANGLE,
        &draft.angle,
        (Dimension::ANGLE, Rule::Any),
    )];

    if let Some(beyond) = &draft.beyond {
        values.push(found(
            ValueSlot::DraftBeyond,
            draft_panel::BEYOND,
            beyond,
            (Dimension::ANGLE, Rule::Any),
        ));
    }
    values
}

fn curve_pattern_values(curve: &CurvePattern) -> Vec<Found> {
    let mut values = vec![found(
        ValueSlot::Count,
        COUNT,
        &curve.count,
        (Dimension::NONE, Rule::Count),
    )];

    if curve.measured == CurveSpacing::Distance {
        values.push(found(
            ValueSlot::CurveSpacing,
            "Spacing",
            &curve.spacing,
            (Dimension::LENGTH, Rule::AboveZeroOrReverse),
        ));
    }
    values
}

fn scale_values(scale: &Scale) -> Vec<Found> {
    vec![found(
        ValueSlot::ScaleFactor,
        "Factor",
        &scale.factor,
        (Dimension::NONE, Rule::AboveZero),
    )]
}

fn values_of(parameters: &ParameterValues, kind: &FeatureKind) -> Vec<Found> {
    let length = |rule| (Dimension::LENGTH, rule);
    match kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude_values(extrude),
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => revolve_values(parameters, revolve),
        FeatureKind::Solid(SolidFeature::Sweep(sweep)) => sweep_values(sweep),
        FeatureKind::Pipe(pipe) => pipe_values(pipe),
        FeatureKind::Coil(coil) => coil_values(coil),
        FeatureKind::Helix(helix) => helix_values(helix),
        FeatureKind::Rib(rib) => rib_values(rib),
        FeatureKind::Emboss(emboss) => emboss_values(emboss),
        FeatureKind::MoveFace(moving) => move_face_values(moving),
        FeatureKind::Draft(draft) => draft_values(draft),
        FeatureKind::Scale(scale) => scale_values(scale),
        FeatureKind::Loft(_) | FeatureKind::DeleteFace(_) | FeatureKind::ReplaceFace(_) => {
            Vec::new()
        }
        FeatureKind::Blend(blend) => blend_values(blend),
        FeatureKind::Shell(shell) => vec![found(
            ValueSlot::ShellThickness,
            "Thickness",
            &shell.thickness,
            length(Rule::AboveZero),
        )],
        FeatureKind::Hole(hole) => {
            let diameter = (!hole.sizing.by_circles()).then(|| {
                found(
                    ValueSlot::HoleDiameter,
                    "Diameter",
                    &hole.diameter,
                    length(Rule::AboveZero),
                )
            });
            let depth = match &hole.depth {
                HoleDepth::Blind(depth) => Some(found(
                    ValueSlot::HoleDepth,
                    "Depth",
                    depth,
                    length(Rule::AboveZero),
                )),
                _ => None,
            };
            [diameter, depth].into_iter().flatten().collect()
        }
        FeatureKind::Pattern(pattern) => match &pattern.kind {
            PatternKind::Linear { first, second } => {
                let mut values: Vec<Found> = direction_values(
                    first,
                    [ValueSlot::Count, ValueSlot::Spacing],
                    [COUNT, spacing_caption(first, true)],
                )
                .into_iter()
                .collect();

                if let Some(second) = second {
                    values.extend(direction_values(
                        second,
                        [ValueSlot::SecondCount, ValueSlot::SecondSpacing],
                        [SECOND_COUNT, spacing_caption(second, false)],
                    ));
                }
                values
            }
            PatternKind::Circular(circular) => vec![
                found(
                    ValueSlot::Count,
                    COUNT,
                    &circular.count,
                    (Dimension::NONE, Rule::Count),
                ),
                found(
                    ValueSlot::PatternAngle,
                    "Total angle",
                    &circular.angle,
                    (Dimension::ANGLE, Rule::Turn),
                ),
            ],
            PatternKind::Curve(curve) => curve_pattern_values(curve),
            PatternKind::Points(_) => Vec::new(),
        },
        FeatureKind::OffsetFace(offset) => vec![found(
            ValueSlot::OffsetDistance,
            "Distance",
            &offset.distance,
            length(Rule::Any),
        )],
        FeatureKind::Primitive(primitive) => primitive_values(&primitive.shape),
        FeatureKind::Move(movement) => move_values(parameters, movement),
        FeatureKind::Thread(thread) => match &thread.length {
            ThreadLength::Depth(depth) => vec![found(
                ValueSlot::ThreadDepth,
                "Depth",
                depth,
                length(Rule::AboveZero),
            )],
            ThreadLength::Full => Vec::new(),
        },
        FeatureKind::Sketch(_)
        | FeatureKind::SplitFace(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Mate(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Split(_)
        | FeatureKind::Datum(_)
        | FeatureKind::Import(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Measurement(_)
        | FeatureKind::ReferenceImage(_) => Vec::new(),
    }
}

fn blend_values(blend: &Blend) -> Vec<Found> {
    let caption = match (blend.kind, blend.chamfer_form()) {
        (BlendKind::Fillet, _) => blend.sizing.title(),
        (BlendKind::Chamfer, ChamferForm::TwoDistances { .. }) => "First distance",
        (BlendKind::Chamfer, ChamferForm::Equal | ChamferForm::DistanceAngle { .. }) => "Distance",
    };
    let size = found(
        ValueSlot::BlendSize,
        caption,
        &blend.size,
        (Dimension::LENGTH, Rule::AboveZero),
    );
    let second = match blend.chamfer_form() {
        ChamferForm::Equal => None,
        ChamferForm::TwoDistances { second } => Some(found(
            ValueSlot::ChamferSecondDistance,
            "Second distance",
            second,
            (Dimension::LENGTH, Rule::AboveZero),
        )),
        ChamferForm::DistanceAngle { angle } => Some(found(
            ValueSlot::ChamferAngle,
            "Angle",
            angle,
            (Dimension::ANGLE, Rule::ChamferAngle),
        )),
    };
    std::iter::once(size).chain(second).collect()
}

fn primitive_values(shape: &PrimitiveShape) -> Vec<Found> {
    shape
        .sizes()
        .into_iter()
        .zip(shape.rules())
        .filter_map(|((what, expression), rule)| {
            let size = PrimitiveSize::named(what)?;
            let checked = match rule {
                SizeRule::AboveZero => (Dimension::LENGTH, Rule::AboveZero),
                SizeRule::ZeroOrMore => (Dimension::LENGTH, Rule::ZeroOrMore),
                SizeRule::Sides => (Dimension::NONE, Rule::Sides),
            };
            Some(found(
                ValueSlot::Primitive(size),
                size.caption(),
                expression,
                checked,
            ))
        })
        .collect()
}

fn move_values(parameters: &ParameterValues, movement: &Move) -> Vec<Found> {
    MoveAxis::ALL
        .into_iter()
        .filter(|axis| {
            parameters
                .evaluate_expression(axis.of(&movement.offset))
                .is_ok_and(|value| value.value != 0.0)
        })
        .map(|axis| {
            found(
                ValueSlot::MoveOffset(axis),
                move_caption(axis),
                axis.of(&movement.offset),
                (Dimension::LENGTH, Rule::Any),
            )
        })
        .collect()
}

fn face_box(solid: &Solid, face: &Face) -> Option<Aabb> {
    face.loops()
        .iter()
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|found| found.coedges().iter())
        .filter_map(|coedge| solid.edge(solid.coedge(*coedge)?.edge()))
        .map(|edge| edge.curve().bounding_box(edge.interval()))
        .reduce(Aabb::union)
}

fn anchor_of(model: &Model, feature: FeatureId) -> Option<Point3> {
    let evaluation = model.evaluation();

    if let Some(body) = evaluation
        .feature(feature)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::solid)
    {
        return made_faces(&body.solid, feature)
            .map(|(_, bounds)| bounds)
            .reduce(Aabb::union)
            .or_else(|| body.bounding_box())
            .map(|bounds| bounds.center());
    }

    let points = helix_tools::points(evaluation, feature);
    points.get(points.len() / 2).copied()
}

pub fn body_of(evaluation: &Evaluation, feature: FeatureId) -> Option<&Solid> {
    evaluation
        .feature(feature)?
        .result
        .as_deref()
        .and_then(FeatureResult::solid)
        .map(|body| &body.solid)
}

pub fn made_faces(solid: &Solid, feature: FeatureId) -> impl Iterator<Item = (&Face, Aabb)> {
    solid
        .faces()
        .filter(move |(_, face)| face.origin().map(origin_feature) == Some(feature))
        .filter_map(|(_, face)| Some((face, face_box(solid, face)?)))
}

pub fn sketch_of(kind: &FeatureKind) -> Option<FeatureId> {
    match kind {
        FeatureKind::Solid(solid) => Some(solid.sketch()),
        FeatureKind::Hole(hole) => Some(hole.sketch),
        _ => None,
    }
}

pub fn of(model: &Model, feature: FeatureId) -> Vec<FeatureValue> {
    let Some(owner) = model.document().feature(feature) else {
        return Vec::new();
    };
    let values = values_of(model.parameters(), &owner.kind);

    if values.is_empty() {
        return Vec::new();
    }

    let shapes = value_shapes::of(model, feature);
    let drawn_of = |slot: ValueSlot| {
        shapes
            .iter()
            .find(|(shaped, _)| *shaped == slot)
            .map_or(Drawn::Stacked, |(_, drawn)| drawn.clone())
    };
    let Some(anchor) =
        anchor_of(model, feature).or_else(|| shapes.iter().find_map(|(_, drawn)| drawn.middle()))
    else {
        return Vec::new();
    };
    values
        .into_iter()
        .map(|value| {
            let drawn = drawn_of(value.slot);
            FeatureValue {
                feature,
                slot: value.slot,
                caption: value.caption,
                expression: value.expression,
                dimension: value.dimension,
                rule: value.rule,
                anchor: drawn.middle().unwrap_or(anchor),
                drawn,
            }
        })
        .collect()
}

fn set_end(end: &mut ExtrudeEnd, value: Expression) -> bool {
    match end {
        ExtrudeEnd::Distance(distance) => {
            *distance = value;
            true
        }
        ExtrudeEnd::ThroughAll
        | ExtrudeEnd::UpToNext { .. }
        | ExtrudeEnd::UpToFace { .. }
        | ExtrudeEnd::UpToSurface { .. } => false,
    }
}

fn set_extrude(extent: &mut ExtrudeExtent, reach: Reach, value: Expression) -> bool {
    match (extent, reach) {
        (ExtrudeExtent::OneSide { end, .. }, Reach::Only)
        | (ExtrudeExtent::TwoSides { forward: end, .. }, Reach::Forward)
        | (ExtrudeExtent::TwoSides { backward: end, .. }, Reach::Backward) => set_end(end, value),
        (ExtrudeExtent::Symmetric { distance }, Reach::Symmetric) => {
            *distance = value;
            true
        }
        _ => false,
    }
}

fn set_revolve(extent: &mut RevolveExtent, slot: ValueSlot, value: Expression) -> bool {
    let target = match (extent, slot) {
        (RevolveExtent::OneSide { angle, .. }, ValueSlot::RevolveAngle)
        | (RevolveExtent::Symmetric { angle }, ValueSlot::RevolveTotal)
        | (RevolveExtent::TwoSides { forward: angle, .. }, ValueSlot::RevolveForward)
        | (
            RevolveExtent::TwoSides {
                backward: angle, ..
            },
            ValueSlot::RevolveBackward,
        ) => angle,
        _ => return false,
    };

    *target = value;
    true
}

fn set_pattern(kind: &mut PatternKind, slot: ValueSlot, value: Expression) -> bool {
    let target = match (kind, slot) {
        (PatternKind::Linear { first, .. }, ValueSlot::Count) => &mut first.count,
        (PatternKind::Linear { first, .. }, ValueSlot::Spacing) => &mut first.spacing,
        (
            PatternKind::Linear {
                second: Some(second),
                ..
            },
            ValueSlot::SecondCount,
        ) => &mut second.count,
        (
            PatternKind::Linear {
                second: Some(second),
                ..
            },
            ValueSlot::SecondSpacing,
        ) => &mut second.spacing,
        (PatternKind::Circular(circular), ValueSlot::Count) => &mut circular.count,
        (PatternKind::Circular(circular), ValueSlot::PatternAngle) => &mut circular.angle,
        (PatternKind::Curve(curve), ValueSlot::Count) => &mut curve.count,
        (PatternKind::Curve(curve), ValueSlot::CurveSpacing) => &mut curve.spacing,
        _ => return false,
    };

    *target = value;
    true
}

fn set_coil_length(
    length: &mut CoilLength,
    [turns_slot, pitch_slot, height_slot]: [ValueSlot; 3],
    slot: ValueSlot,
    value: Expression,
) -> bool {
    let target = match (length, slot) {
        (CoilLength::TurnsAndPitch { turns, .. } | CoilLength::HeightAndTurns { turns, .. }, s)
            if s == turns_slot =>
        {
            turns
        }
        (CoilLength::TurnsAndPitch { pitch, .. } | CoilLength::HeightAndPitch { pitch, .. }, s)
            if s == pitch_slot =>
        {
            pitch
        }
        (
            CoilLength::HeightAndTurns { height, .. } | CoilLength::HeightAndPitch { height, .. },
            s,
        ) if s == height_slot => height,
        _ => return false,
    };

    *target = value;
    true
}

fn set_chamfer_second(blend: &mut Blend, slot: ValueSlot, value: Expression) -> bool {
    let fits = matches!(
        (&blend.form, slot),
        (
            ChamferForm::TwoDistances { .. },
            ValueSlot::ChamferSecondDistance
        ) | (ChamferForm::DistanceAngle { .. }, ValueSlot::ChamferAngle)
    );
    match blend.form.expression_mut().filter(|_| fits) {
        Some(target) => {
            *target = value;
            true
        }
        None => false,
    }
}

fn set_primitive(shape: &mut PrimitiveShape, size: PrimitiveSize, value: Expression) -> bool {
    let index = shape
        .sizes()
        .into_iter()
        .position(|(what, _)| PrimitiveSize::named(what) == Some(size));
    match index.and_then(|index| shape.sizes_mut().into_iter().nth(index)) {
        Some(target) => {
            *target = value;
            true
        }
        None => false,
    }
}

fn rebuilt(kind: &FeatureKind, slot: ValueSlot, value: Expression) -> Option<FeatureKind> {
    let mut changed = kind.clone();

    let set = match (&mut changed, slot) {
        (FeatureKind::Solid(SolidFeature::Extrude(extrude)), ValueSlot::Extrude(reach)) => {
            set_extrude(&mut extrude.extent, reach, value)
        }
        (FeatureKind::Solid(SolidFeature::Revolve(revolve)), _) => {
            set_revolve(&mut revolve.extent, slot, value)
        }
        (FeatureKind::Solid(SolidFeature::Sweep(sweep)), ValueSlot::SweepTwist) => {
            sweep.twist = value;
            true
        }
        (FeatureKind::Solid(SolidFeature::Sweep(sweep)), ValueSlot::SweepScale) => {
            sweep.scale = value;
            true
        }
        (FeatureKind::Pipe(pipe), ValueSlot::PipeSize) => {
            pipe.size = value;
            true
        }
        (FeatureKind::Pipe(pipe), ValueSlot::PipeWall) => match &mut pipe.wall {
            Some(wall) => {
                *wall = value;
                true
            }
            None => false,
        },
        (FeatureKind::Pipe(pipe), ValueSlot::PipeBend) => {
            pipe.bend = value;
            true
        }
        (FeatureKind::Coil(coil), ValueSlot::CoilDiameter) => {
            coil.diameter = value;
            true
        }
        (FeatureKind::Coil(coil), ValueSlot::CoilSection) => {
            coil.size = value;
            true
        }
        (FeatureKind::Coil(coil), ValueSlot::CoilTaper) => {
            coil.taper = value;
            true
        }
        (FeatureKind::Coil(coil), _) => set_coil_length(
            &mut coil.length,
            [
                ValueSlot::CoilTurns,
                ValueSlot::CoilPitch,
                ValueSlot::CoilHeight,
            ],
            slot,
            value,
        ),
        (FeatureKind::Helix(helix), ValueSlot::HelixDiameter) => {
            helix.diameter = value;
            true
        }
        (FeatureKind::Helix(helix), ValueSlot::HelixTaper) => {
            helix.taper = value;
            true
        }
        (FeatureKind::Helix(helix), ValueSlot::HelixStart) => {
            helix.start = value;
            true
        }
        (FeatureKind::Helix(helix), _) => set_coil_length(
            &mut helix.length,
            [
                ValueSlot::HelixTurns,
                ValueSlot::HelixPitch,
                ValueSlot::HelixHeight,
            ],
            slot,
            value,
        ),
        (FeatureKind::Rib(rib), ValueSlot::RibThickness) => {
            rib.thickness = value;
            true
        }
        (FeatureKind::Rib(rib), ValueSlot::RibDraft) => {
            rib.draft = value;
            true
        }
        (FeatureKind::Emboss(emboss), ValueSlot::EmbossDepth) => {
            emboss.depth = value;
            true
        }
        (FeatureKind::Emboss(emboss), ValueSlot::EmbossDraft) => {
            emboss.draft = value;
            true
        }
        (FeatureKind::MoveFace(moving), ValueSlot::MoveFaceDistance) => {
            moving.distance = value;
            true
        }
        (FeatureKind::MoveFace(moving), ValueSlot::MoveFaceAngle) => {
            moving.angle = value;
            true
        }
        (FeatureKind::Draft(draft), ValueSlot::DraftAngle) => {
            draft.angle = value;
            true
        }
        (FeatureKind::Draft(draft), ValueSlot::DraftBeyond) => match &mut draft.beyond {
            Some(beyond) => {
                *beyond = value;
                true
            }
            None => false,
        },
        (FeatureKind::Scale(scale), ValueSlot::ScaleFactor) => {
            scale.factor = value;
            true
        }
        (FeatureKind::Blend(blend), ValueSlot::BlendSize) => {
            blend.size = value;
            true
        }
        (FeatureKind::Shell(shell), ValueSlot::ShellThickness) => {
            shell.thickness = value;
            true
        }
        (FeatureKind::Hole(hole), ValueSlot::HoleDiameter) => {
            hole.diameter = value;
            hole.standard = None;
            true
        }
        (FeatureKind::Hole(hole), ValueSlot::HoleDepth) => match &mut hole.depth {
            HoleDepth::Blind(depth) => {
                *depth = value;
                true
            }
            _ => false,
        },
        (FeatureKind::Pattern(pattern), _) => set_pattern(&mut pattern.kind, slot, value),
        (FeatureKind::Blend(blend), ValueSlot::ChamferSecondDistance | ValueSlot::ChamferAngle) => {
            set_chamfer_second(blend, slot, value)
        }
        (FeatureKind::OffsetFace(offset), ValueSlot::OffsetDistance) => {
            offset.distance = value;
            true
        }
        (FeatureKind::Primitive(primitive), ValueSlot::Primitive(size)) => {
            set_primitive(&mut primitive.shape, size, value)
        }
        (FeatureKind::Move(movement), ValueSlot::MoveOffset(axis)) => {
            *axis.of_mut(&mut movement.offset) = value;
            true
        }
        (FeatureKind::Thread(thread), ValueSlot::ThreadDepth) => match &mut thread.length {
            ThreadLength::Depth(depth) => {
                *depth = value;
                true
            }
            ThreadLength::Full => false,
        },
        _ => false,
    };
    set.then_some(changed)
}

pub const GONE: &str = "The value is no longer there";

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ChangeError {
    #[error("{}", GONE)]
    Gone { feature: FeatureId, slot: ValueSlot },
    #[error(transparent)]
    Rejected(#[from] CheckError),
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    slot: ValueSlot,
    value: Expression,
) -> Result<Transaction, ChangeError> {
    let document = model.document();
    let gone = ChangeError::Gone { feature, slot };

    let owner = document.feature(feature).ok_or_else(|| gone.clone())?;
    let (value, negative) = match slot {
        ValueSlot::Extrude(Reach::Only) => reversing::unsigned(model.parameters(), value),
        _ => (value, false),
    };

    let kind = rebuilt(&owner.kind, slot, value).ok_or(gone)?;
    let kind = reversing::flipping(kind, negative);
    Ok(field::checked::<CheckError>(
        document,
        Transaction::single(
            format!("Edit {}", owner.name),
            Edit::SetFeatureKind { id: feature, kind },
        ),
    )?)
}

#[cfg(test)]
mod tests {
    use caditor_document::{Blend, FilletSizing, Shell};
    use caditor_expression::Unit;

    use super::*;

    fn millimetres(value: f64) -> Expression {
        Expression::measure(value, Unit::Millimetre)
    }

    #[test]
    fn a_fillet_offers_its_radius_and_a_shell_its_thickness_each_set_back_into_its_kind() {
        let body = FeatureId::from_raw(1);
        let fillet = FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body,
            edges: Vec::new(),
            groups: Vec::new(),
            size: millimetres(2.0),
            form: ChamferForm::Equal,
            flipped: false,
            sizing: FilletSizing::Radius,
        });
        let shell = FeatureKind::Shell(Shell {
            body,
            open: Vec::new(),
            thickness: millimetres(1.0),
            side: caditor_document::ShellSide::Inside,
            faces: Vec::new(),
        });
        let parameters = ParameterValues::default();

        let radius = values_of(&parameters, &fillet);
        let thickness = values_of(&parameters, &shell);
        let rounder = rebuilt(&fillet, ValueSlot::BlendSize, millimetres(5.0)).unwrap();
        let thicker = rebuilt(&shell, ValueSlot::ShellThickness, millimetres(3.0)).unwrap();

        assert_eq!(radius.len(), 1);
        assert_eq!(radius[0].caption, "Radius");
        assert_eq!(thickness[0].caption, "Thickness");
        assert_eq!(
            rounder.blend().map(|blend| &blend.size),
            Some(&millimetres(5.0))
        );
        assert!(
            matches!(thicker, FeatureKind::Shell(shell) if shell.thickness == millimetres(3.0))
        );
        assert_eq!(
            rebuilt(&shell, ValueSlot::BlendSize, millimetres(3.0)),
            None
        );
    }

    #[test]
    fn sweep_pipe_rib_emboss_move_face_draft_scale_and_circle_hole_offer_their_values() {
        let body = FeatureId::from_raw(1);
        let sketch = FeatureId::from_raw(2);
        let parameters = ParameterValues::default();
        let degrees = |value: f64| Expression::measure(value, Unit::Degree);

        let sweep = FeatureKind::Solid(SolidFeature::Sweep(Sweep {
            sketch,
            regions: caditor_document::RegionChoice::All,
            path: caditor_document::SweepPath::Unchosen,
            orientation: caditor_kernel::SweepOrientation::default(),
            twist: degrees(15.0),
            scale: Expression::number(1.2),
            shape_law: Vec::new(),
            operation: caditor_document::BodyOperation::NewBody,
            other_bodies: Vec::new(),
        }));
        let pipe = FeatureKind::Pipe(Pipe {
            path: caditor_document::SweepPath::Unchosen,
            shape: caditor_kernel::PipeShape::Round,
            size: millimetres(6.0),
            wall: Some(millimetres(1.0)),
            bend: millimetres(4.0),
            operation: caditor_document::BodyOperation::NewBody,
        });
        let scale = FeatureKind::Scale(Scale {
            body,
            factor: Expression::number(1.5),
            center: [millimetres(0.0), millimetres(0.0), millimetres(0.0)],
            frame: None,
        });
        let moving = FeatureKind::MoveFace(MoveFace {
            body,
            faces: Vec::new(),
            direction: None,
            distance: millimetres(3.0),
            axis: None,
            angle: degrees(10.0),
        });
        let circle_hole = FeatureKind::Hole(caditor_document::Hole {
            body,
            sketch,
            diameter: millimetres(6.0),
            sizing: caditor_document::HoleSizing::Circles,
            depth: HoleDepth::Blind(millimetres(10.0)),
            style: caditor_document::HoleStyle::Plain,
            shape: caditor_document::HoleShape::Round,
            bottom: caditor_document::HoleBottom::Flat,
            reversed: false,
            standard: None,
            thread: caditor_document::TappedThread::default(),
        });

        let sweep_slots: Vec<_> = values_of(&parameters, &sweep)
            .into_iter()
            .map(|found| found.slot)
            .collect();
        let pipe_slots: Vec<_> = values_of(&parameters, &pipe)
            .into_iter()
            .map(|found| found.slot)
            .collect();
        let move_slots: Vec<_> = values_of(&parameters, &moving)
            .into_iter()
            .map(|found| found.slot)
            .collect();
        let hole_slots: Vec<_> = values_of(&parameters, &circle_hole)
            .into_iter()
            .map(|found| found.slot)
            .collect();

        assert_eq!(
            sweep_slots,
            vec![ValueSlot::SweepTwist, ValueSlot::SweepScale]
        );
        assert_eq!(
            pipe_slots,
            vec![
                ValueSlot::PipeSize,
                ValueSlot::PipeWall,
                ValueSlot::PipeBend
            ]
        );
        assert_eq!(move_slots, vec![ValueSlot::MoveFaceDistance]);
        assert_eq!(hole_slots, vec![ValueSlot::HoleDepth]);
        assert!(matches!(
            rebuilt(&scale, ValueSlot::ScaleFactor, Expression::number(2.0)),
            Some(FeatureKind::Scale(scaled)) if scaled.factor == Expression::number(2.0)
        ));
    }
}
