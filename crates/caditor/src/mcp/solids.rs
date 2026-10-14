use caditor_document::{
    BodyOperation, Document, Evaluation, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureKind, Hole,
    HoleDepth, HoleFit, HoleStandard, HoleStyle, MetricSize, RegionChoice, SolidFeature,
    SolidStart, ThreadForm, ThreadHand, Wall, hole_centres,
};
use caditor_expression::{Dimension, Expression};
use caditor_kernel::WallSide;
use serde::Deserialize;
use serde_json::json;

use crate::{
    extrusion_detection,
    hole_tools::{self, Kind},
    last_values::Starts,
    mcp::{
        features::{
            Amount, Built, FeatureError, added, amount, chosen_operation, length, parsed,
            signed_length, sketch_of, turn,
        },
        places::{self, PlaneChoice},
        references::{self, Named, body_label},
        session::{Session, Work},
        tools::{Answer, ToolError},
        writing::{ModellingError, OperationName, operation_words},
    },
    solid_tools::{self, Reach, Sweep},
    units::LengthUnit,
};

const THROUGH_ALL: [&str; 3] = ["through all", "through_all", "through"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NextFace {
    Next,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum UpTo {
    Next(NextFace),
    Plane(PlaneChoice),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallName {
    Inside,
    Outside,
    Centred,
}

impl WallName {
    fn side(self) -> WallSide {
        match self {
            Self::Inside => WallSide::Inside,
            Self::Outside => WallSide::Outside,
            Self::Centred => WallSide::Centred,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extrusion {
    pub sketch: Option<Named>,
    pub distance: Option<Amount>,
    pub through_all: Option<bool>,
    pub up_to: Option<UpTo>,
    pub end_offset: Option<Amount>,
    pub symmetric: Option<bool>,
    pub second_distance: Option<Amount>,
    pub start_offset: Option<Amount>,
    pub taper: Option<Amount>,
    pub thin_wall: Option<Amount>,
    pub wall_side: Option<WallName>,
    pub operation: Option<OperationName>,
    pub body: Option<Named>,
    pub reversed: Option<bool>,
    pub name: Option<String>,
}

impl Extrusion {
    fn changes_extent(&self) -> bool {
        self.distance.is_some()
            || self.through_all.is_some()
            || self.up_to.is_some()
            || self.end_offset.is_some()
            || self.symmetric.is_some()
            || self.second_distance.is_some()
            || self.reversed.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleName {
    Plain,
    Counterbore,
    Countersink,
}

impl StyleName {
    fn kind(self) -> Kind {
        match self {
            Self::Plain => Kind::Plain,
            Self::Counterbore => Kind::Counterbore,
            Self::Countersink => Kind::Countersink,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitName {
    Close,
    Normal,
    Loose,
    Tapped,
    TappedFine,
    HeatSetInsert,
}

impl FitName {
    fn fit(self) -> HoleFit {
        let id = match self {
            Self::Close => "close",
            Self::Normal => "normal",
            Self::Loose => "loose",
            Self::Tapped => "tapped",
            Self::TappedFine => "tapped_fine",
            Self::HeatSetInsert => "heat_set_insert",
        };
        HoleFit::from_id(id).unwrap_or(HoleFit::Normal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormName {
    Modelled,
    Cosmetic,
}

impl FormName {
    pub(super) fn form(self) -> ThreadForm {
        match self {
            Self::Modelled => ThreadForm::Modelled,
            Self::Cosmetic => ThreadForm::Cosmetic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandName {
    Right,
    Left,
}

impl HandName {
    pub(super) fn hand(self) -> ThreadHand {
        match self {
            Self::Right => ThreadHand::Right,
            Self::Left => ThreadHand::Left,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drilling {
    pub sketch: Option<Named>,
    pub diameter: Option<Amount>,
    pub depth: Option<Amount>,
    pub up_to: Option<UpTo>,
    pub end_offset: Option<Amount>,
    pub style: Option<StyleName>,
    pub head_diameter: Option<Amount>,
    pub head_depth: Option<Amount>,
    pub head_angle: Option<Amount>,
    pub standard: Option<String>,
    pub fit: Option<FitName>,
    pub thread_form: Option<FormName>,
    pub thread_hand: Option<HandName>,
    pub thread_depth: Option<Amount>,
    pub body: Option<Named>,
    pub reversed: Option<bool>,
    pub name: Option<String>,
}

enum End {
    Distance(Expression),
    ThroughAll,
    Next,
    Plane(caditor_document::PlaneReference),
}

fn end_asked(
    document: &Document,
    evaluation: &Evaluation,
    (distance, through_all, up_to): (Option<Expression>, bool, Option<&UpTo>),
) -> Result<Option<End>, ToolError> {
    let given =
        usize::from(distance.is_some()) + usize::from(through_all) + usize::from(up_to.is_some());
    if given > 1 {
        return Err(ModellingError::TwoEnds.into());
    }
    Ok(match (distance, through_all, up_to) {
        (Some(distance), _, _) => Some(End::Distance(distance)),
        (None, true, _) => Some(End::ThroughAll),
        (None, false, Some(UpTo::Next(NextFace::Next))) => Some(End::Next),
        (None, false, Some(UpTo::Plane(choice))) => {
            Some(End::Plane(places::plane(document, evaluation, choice)?))
        }
        (None, false, None) => None,
    })
}

impl End {
    fn extruded(self) -> ExtrudeEnd {
        match self {
            Self::Distance(distance) => ExtrudeEnd::Distance(distance),
            Self::ThroughAll => ExtrudeEnd::ThroughAll,
            Self::Next => ExtrudeEnd::up_to_next(),
            Self::Plane(plane) => ExtrudeEnd::up_to_face(plane),
        }
    }

    fn drilled(self) -> HoleDepth {
        match self {
            Self::Distance(depth) => HoleDepth::Blind(depth),
            Self::ThroughAll => HoleDepth::ThroughAll,
            Self::Next => HoleDepth::up_to_next(),
            Self::Plane(plane) => HoleDepth::up_to_face(plane),
        }
    }
}

struct ExtrudeSides {
    end: Option<ExtrudeEnd>,
    second: Option<ExtrudeEnd>,
    symmetric: bool,
    reversed: bool,
}

impl ExtrudeSides {
    fn of(extent: &ExtrudeExtent) -> Self {
        let (end, second, symmetric, reversed) = match extent {
            ExtrudeExtent::OneSide { end, reversed } => (Some(end.clone()), None, false, *reversed),
            ExtrudeExtent::Symmetric { distance } => (
                Some(ExtrudeEnd::Distance(distance.clone())),
                None,
                true,
                false,
            ),
            ExtrudeExtent::TwoSides { forward, backward } => {
                (Some(forward.clone()), Some(backward.clone()), false, false)
            }
        };
        Self {
            end,
            second,
            symmetric,
            reversed,
        }
    }

    fn extent(self) -> Result<ExtrudeExtent, ToolError> {
        match (self.end, self.second, self.symmetric) {
            (None, _, _) => Err(ModellingError::NoDistance.into()),
            (Some(_), Some(_), true) => Err(FeatureError::SymmetricTwoSides {
                feature: "extrusion",
                size: "distance",
                second: "second_distance",
            }
            .into()),
            (Some(ExtrudeEnd::Distance(distance)), None, true) => {
                Ok(ExtrudeExtent::Symmetric { distance })
            }
            (Some(_), None, true) => Err(ModellingError::SymmetricThroughAll.into()),
            (Some(forward), Some(backward), false) => {
                Ok(ExtrudeExtent::TwoSides { forward, backward })
            }
            (Some(end), None, false) => Ok(ExtrudeExtent::OneSide {
                end,
                reversed: self.reversed,
            }),
        }
    }
}

fn offset_end(end: ExtrudeEnd, offset: Option<Expression>) -> Result<ExtrudeEnd, ToolError> {
    match offset {
        None => Ok(end),
        Some(_) if !end.takes_offset() => Err(ModellingError::OffsetWithoutUpTo.into()),
        Some(offset) => Ok(end.with_offset(Some(offset))),
    }
}

fn wall_of(
    thickness: Option<Expression>,
    side: Option<WallName>,
    kept: Option<&Wall>,
) -> Result<Option<Box<Wall>>, ToolError> {
    Ok(match (thickness, kept) {
        (Some(thickness), kept) => Some(Box::new(Wall {
            thickness,
            side: side
                .map(WallName::side)
                .or_else(|| kept.map(|kept| kept.side))
                .unwrap_or(WallSide::Centred),
        })),
        (None, Some(kept)) => Some(Box::new(Wall {
            side: side.map_or(kept.side, WallName::side),
            ..kept.clone()
        })),
        (None, None) if side.is_some() => return Err(ModellingError::SideWithoutWall.into()),
        (None, None) => None,
    })
}

pub fn extrude_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Extrusion,
    base: Option<&Extrude>,
) -> Result<Built, ToolError> {
    let through_all = asked.through_all.unwrap_or(false);
    let cuts = matches!(
        asked.operation,
        Some(OperationName::Remove | OperationName::Intersect)
    );
    if through_all && asked.operation.is_some() && !cuts {
        return Err(ModellingError::ThroughAllAdds.into());
    }
    let distance = parsed(session, asked.distance.as_ref(), |session, distance| {
        length(session, "distance", distance)
    })?;
    let second = parsed(
        session,
        asked.second_distance.as_ref(),
        |session, distance| length(session, "second_distance", distance),
    )?;
    let end_offset = parsed(session, asked.end_offset.as_ref(), |session, offset| {
        signed_length(session, "end_offset", offset)
    })?;
    let start_offset = parsed(session, asked.start_offset.as_ref(), |session, offset| {
        signed_length(session, "start_offset", offset)
    })?;
    let taper = parsed(session, asked.taper.as_ref(), |session, taper| {
        turn(session, "taper", taper)
    })?;
    let thickness = parsed(session, asked.thin_wall.as_ref(), |session, wall| {
        length(session, "thin_wall", wall)
    })?;
    session.evaluate(work)?;
    let values = session.parameters();
    let document = session.document();
    let evaluation = session.evaluation();
    let (sketch, _, _) = sketch_of(
        document,
        asked.sketch.as_ref(),
        base.map(|base| base.sketch),
    )?;
    let end = end_asked(
        document,
        evaluation,
        (distance, through_all, asked.up_to.as_ref()),
    )?;
    let extent = match base {
        Some(base) if !asked.changes_extent() => base.extent.clone(),
        _ => {
            let kept = base.map_or(
                ExtrudeSides {
                    end: None,
                    second: None,
                    symmetric: false,
                    reversed: false,
                },
                |base| ExtrudeSides::of(&base.extent),
            );
            let symmetric = asked.symmetric.unwrap_or(kept.symmetric);
            let end = end
                .map(End::extruded)
                .or(kept.end)
                .map(|end| offset_end(end, end_offset))
                .transpose()?;
            ExtrudeSides {
                end,
                second: second
                    .map(ExtrudeEnd::Distance)
                    .or(kept.second.filter(|_| !symmetric)),
                symmetric,
                reversed: asked.reversed.unwrap_or(kept.reversed),
            }
            .extent()?
        }
    };
    let template = Extrude {
        sketch,
        regions: match base {
            Some(base) if base.sketch == sketch => base.regions.clone(),
            Some(_) | None => RegionChoice::All,
        },
        extent,
        operation: base.map_or(BodyOperation::NewBody, |base| base.operation),
        start: start_offset
            .map(SolidStart::Distance)
            .or_else(|| base.and_then(|base| base.start.clone())),
        other_bodies: base
            .map(|base| base.other_bodies.clone())
            .unwrap_or_default(),
        taper: taper
            .map(Box::new)
            .or_else(|| base.and_then(|base| base.taper.clone())),
        wall: wall_of(
            thickness,
            asked.wall_side,
            base.and_then(|base| base.wall.as_deref()),
        )?,
        direction: base.and_then(|base| base.direction.clone()),
    };
    let chosen = asked
        .body
        .as_ref()
        .map(|body| references::body(document, evaluation, body))
        .transpose()?;
    let standing_on = solid_tools::face_body(document, sketch);
    let (solid, told) = match (asked.operation, base) {
        (None, Some(base)) => {
            let operation = match (chosen, base.operation) {
                (None, kept) => kept,
                (Some(body), BodyOperation::Remove(_)) => BodyOperation::Remove(body),
                (Some(body), BodyOperation::Intersect(_)) => BodyOperation::Intersect(body),
                (Some(body), BodyOperation::Add(_) | BodyOperation::NewBody) => {
                    BodyOperation::Add(body)
                }
            };
            (
                Extrude {
                    operation,
                    ..template
                },
                None,
            )
        }
        (None, None) => {
            let detected = extrusion_detection::extrusion(
                document,
                Reach {
                    evaluation,
                    parameters: &values,
                },
                &template,
                standing_on,
                chosen,
            );
            (
                Extrude {
                    extent: detected.extent,
                    operation: detected.operation,
                    ..template
                },
                detected.told,
            )
        }
        (Some(name), _) => {
            let target = chosen
                .or(standing_on)
                .or_else(|| base.and_then(|base| base.operation.target()))
                .or_else(|| document.bodies_standing().last().copied());
            let operation = chosen_operation(document, name, target)?;
            let into_its_face = cuts && target.is_some() && target == standing_on;
            let mut solid = Extrude {
                operation,
                ..template
            };
            if base.is_none()
                && asked.reversed.is_none()
                && let ExtrudeExtent::OneSide { reversed, .. } = &mut solid.extent
            {
                *reversed = into_its_face;
            }
            (solid, None)
        }
    };
    let operation = solid.operation;
    let mut built = Built::new(
        FeatureKind::Solid(SolidFeature::Extrude(solid)),
        Sweep::Extrude.label(),
        asked.name,
    )
    .hiding(sketch)
    .telling("operation", json!(operation_words(document, operation)));
    if let Some(told) = told {
        built = built.telling("note", json!(told));
    }
    Ok(built)
}

pub fn extrude(
    session: &mut Session,
    work: &Work<'_>,
    asked: Extrusion,
) -> Result<Answer, ToolError> {
    let built = extrude_built(session, work, asked, None)?;
    added(session, work, built)
}

struct Head {
    diameter: Option<Expression>,
    depth: Option<Expression>,
    angle: Option<Expression>,
}

fn headed(style: &mut HoleStyle, head: Head) -> Result<(), ModellingError> {
    let refused = |size: &'static str, style: &'static str| {
        Err(ModellingError::HeadWithoutStyle { size, style })
    };
    match style {
        HoleStyle::Counterbore { diameter, depth } => {
            if head.angle.is_some() {
                return refused("head_angle", "countersink");
            }
            if let Some(given) = head.diameter {
                *diameter = given;
            }
            if let Some(given) = head.depth {
                *depth = given;
            }
        }
        HoleStyle::Countersink { diameter, angle } => {
            if head.depth.is_some() {
                return refused("head_depth", "counterbore");
            }
            if let Some(given) = head.diameter {
                *diameter = given;
            }
            if let Some(given) = head.angle {
                *angle = given;
            }
        }
        HoleStyle::Plain | HoleStyle::Stepped(_) => {
            if head.diameter.is_some() {
                return refused("head_diameter", "counterbore or countersink");
            }
            if head.depth.is_some() {
                return refused("head_depth", "counterbore");
            }
            if head.angle.is_some() {
                return refused("head_angle", "countersink");
            }
        }
    }
    Ok(())
}

fn standardised(
    hole: &mut Hole,
    standard: Option<&str>,
    fit: Option<FitName>,
) -> Result<(), ToolError> {
    let unit = LengthUnit::Millimetre;
    let kept = hole.standard;
    let chosen = match (standard, fit) {
        (Some(name), fit) => {
            let size = MetricSize::from_name(name.trim()).ok_or_else(|| {
                ModellingError::UnknownStandard {
                    name: name.to_owned(),
                    sizes: MetricSize::ALL
                        .iter()
                        .map(|size| size.name())
                        .collect::<Vec<_>>()
                        .join(", "),
                }
            })?;
            let fit = fit
                .map(FitName::fit)
                .or_else(|| kept.map(|kept| kept.fit))
                .unwrap_or(HoleFit::Normal);
            Some(HoleStandard::offered(size, fit))
        }
        (None, Some(fit)) => {
            let kept = kept.ok_or(ModellingError::FitWithoutStandard)?;
            Some(HoleStandard::offered(kept.size, fit.fit()))
        }
        (None, None) => None,
    };
    if let Some(standard) = chosen {
        *hole = hole_tools::with_standard(hole, standard, unit);
    }
    Ok(())
}

pub fn drill_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Drilling,
    base: Option<&Hole>,
) -> Result<Built, ToolError> {
    if asked.diameter.is_some() && asked.standard.is_some() {
        return Err(ModellingError::DiameterAndStandard.into());
    }
    let diameter = parsed(session, asked.diameter.as_ref(), |session, diameter| {
        length(session, "diameter", diameter)
    })?;
    let through = matches!(
        &asked.depth,
        Some(Amount::Text(text))
            if THROUGH_ALL.iter().any(|words| text.trim().eq_ignore_ascii_case(words))
    );
    let depth = match &asked.depth {
        Some(depth) if !through => Some(length(session, "depth", depth)?),
        Some(_) | None => None,
    };
    let end_offset = parsed(session, asked.end_offset.as_ref(), |session, offset| {
        signed_length(session, "end_offset", offset)
    })?;
    let head = Head {
        diameter: parsed(session, asked.head_diameter.as_ref(), |session, size| {
            length(session, "head_diameter", size)
        })?,
        depth: parsed(session, asked.head_depth.as_ref(), |session, size| {
            length(session, "head_depth", size)
        })?,
        angle: parsed(session, asked.head_angle.as_ref(), |session, angle| {
            amount(session, "head_angle", angle, Dimension::ANGLE, true)
        })?,
    };
    let thread_depth = parsed(session, asked.thread_depth.as_ref(), |session, depth| {
        length(session, "thread_depth", depth)
    })?;
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let (sketch, sketch_name, definition) = sketch_of(
        document,
        asked.sketch.as_ref(),
        base.map(|base| base.sketch),
    )?;
    let centres = hole_centres(definition).len();
    if centres == 0 && (asked.sketch.is_some() || base.is_none()) {
        return Err(ModellingError::NothingToDrill {
            sketch: sketch_name.to_owned(),
        }
        .into());
    }
    let body = match (&asked.body, base) {
        (Some(named), _) => references::body(document, evaluation, named)?,
        (None, Some(base)) => base.body,
        (None, None) => solid_tools::face_body(document, sketch)
            .or_else(|| document.bodies_standing().last().copied())
            .ok_or(ModellingError::NoBodyToDrill)?,
    };
    if base.is_none() && diameter.is_none() && asked.standard.is_none() {
        return Err(ModellingError::NoDiameter.into());
    }
    let end = end_asked(document, evaluation, (depth, through, asked.up_to.as_ref()))?;
    if base.is_none() && end.is_none() {
        return Err(ModellingError::NoDepth.into());
    }
    let mut kind = match base {
        Some(base) => FeatureKind::Hole(base.clone()),
        None => hole_tools::new_hole(sketch, body, &Starts::defaults(LengthUnit::Millimetre)),
    };
    if let FeatureKind::Hole(hole) = &mut kind {
        hole.sketch = sketch;
        hole.body = body;
        standardised(hole, asked.standard.as_deref(), asked.fit)?;
        if let Some(diameter) = diameter {
            hole.diameter = diameter;
            hole.standard = None;
        }
        if let Some(end) = end {
            hole.depth = end.drilled();
        }
        if let Some(offset) = end_offset {
            if !hole.depth.takes_offset() {
                return Err(ModellingError::OffsetWithoutUpTo.into());
            }
            hole.depth = hole.depth.clone().with_offset(Some(offset));
        }
        if let Some(style) = asked.style {
            hole.style = style
                .kind()
                .style_for(hole.standard, LengthUnit::Millimetre);
        }
        headed(&mut hole.style, head)?;
        let threads =
            asked.thread_form.is_some() || asked.thread_hand.is_some() || thread_depth.is_some();
        if threads {
            if !hole.standard.is_some_and(HoleStandard::is_threaded) {
                return Err(ModellingError::NotTapped.into());
            }
            if let Some(form) = asked.thread_form {
                hole.thread.form = form.form();
            }
            if let Some(hand) = asked.thread_hand {
                hole.thread.hand = hand.hand();
            }
            if thread_depth.is_some() {
                hole.thread.depth = thread_depth;
            }
        }
        if let Some(reversed) = asked.reversed {
            hole.reversed = reversed;
        }
    }
    Ok(Built::new(kind, hole_tools::TITLE, asked.name)
        .hiding(sketch)
        .telling("holes", json!(centres))
        .telling("body", json!(body_label(document, body))))
}

pub fn drill(session: &mut Session, work: &Work<'_>, asked: Drilling) -> Result<Answer, ToolError> {
    let built = drill_built(session, work, asked, None)?;
    added(session, work, built)
}
