use caditor_expression::{Dimension, EvalError, Expression, ParameterId, Quantity};
use caditor_geometry::Vector2;
use caditor_sketch::{
    BlendEnd, ChainSize, Continuity, Dimensioned, EntityId, Gear, GearCentre, GearPair, Sketch,
    Sprocket, SpurGear, Text, TextAlign,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    blend_curving, clipboard,
    gearing::{self, Part, Spur, StoredPair, StoredSprocket},
    mcp::{
        features::Amount,
        references::{self, EntityName, Named},
        session::Session,
        sketch_edits::{Planned, SketchEditError, Target, amount},
        tools::ToolError,
        writing::{self, point},
    },
    tangent_circling,
    texting::{self, Lettering, TextAnchor},
};

const DEFAULT_MODULE: &str = "2 mm";
const DEFAULT_TEETH: u32 = 20;
const DEFAULT_PAIR_TEETH: u32 = 30;
const DEFAULT_PAIR_DIRECTION: &str = "0 deg";
const DEFAULT_PRESSURE_ANGLE: &str = "20 deg";
const DEFAULT_ROOT_FILLET: &str = "0.75 mm";
const DEFAULT_TEXT_HEIGHT: &str = "5 mm";
const DEFAULT_TEXT_ANGLE: &str = "0 deg";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TangentCircling {
    pub sketch: Named,
    pub curves: Vec<EntityName>,
    pub radius: Option<String>,
    pub near: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Smoothness {
    Tangent,
    Curvature,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlendOn {
    pub curve: EntityName,
    pub end: EntityName,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blending {
    pub sketch: Named,
    pub first: BlendOn,
    pub second: BlendOn,
    pub continuity: Option<Smoothness>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GearShape {
    Gear,
    Pair,
    Sprocket,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gearing {
    pub sketch: Named,
    pub shape: Option<GearShape>,
    pub center: Option<EntityName>,
    pub at: Option<[f64; 2]>,
    pub module: Option<String>,
    pub teeth: Option<u32>,
    pub pressure_angle: Option<String>,
    pub profile_shift: Option<f64>,
    pub root_fillet: Option<String>,
    pub bore: Option<String>,
    pub meshing_teeth: Option<u32>,
    pub meshing_bore: Option<String>,
    pub meshing_direction: Option<String>,
    pub chain: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearEditing {
    pub sketch: Named,
    pub gear: EntityName,
    pub module: Option<Amount>,
    pub teeth: Option<Amount>,
    pub pressure_angle: Option<Amount>,
    pub profile_shift: Option<Amount>,
    pub root_fillet: Option<Amount>,
    pub bore: Option<Amount>,
    pub turn: Option<Amount>,
    pub direction: Option<Amount>,
    pub chain: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Left,
    #[serde(alias = "center")]
    Centre,
    Right,
}

impl Alignment {
    fn text_align(self) -> TextAlign {
        match self {
            Self::Left => TextAlign::Left,
            Self::Centre => TextAlign::Centre,
            Self::Right => TextAlign::Right,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lettered {
    pub sketch: Named,
    pub content: String,
    pub height: Option<Amount>,
    pub angle: Option<Amount>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    pub align: Option<Alignment>,
    pub anchor: Option<EntityName>,
    pub at: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextEditing {
    pub sketch: Named,
    pub text: EntityName,
    pub content: Option<String>,
    pub height: Option<Amount>,
    pub angle: Option<Amount>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub align: Option<Alignment>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Copying {
    pub sketch: Named,
    pub from_sketch: Option<Named>,
    pub items: Vec<EntityName>,
    pub by: Option<[f64; 2]>,
    pub to: Option<[f64; 2]>,
}

pub(super) fn tangent_circle(
    session: &Session,
    target: &Target<'_>,
    asked: &TangentCircling,
) -> Result<Planned, ToolError> {
    let curves = target.entities(&asked.curves)?;
    let radius = asked
        .radius
        .as_ref()
        .map(|text| amount(session, "radius", text, Dimension::LENGTH))
        .transpose()?;
    let near = asked.near.map(point).transpose()?;
    let transaction = target.reshaped(tangent_circling::TRANSACTION.to_owned(), |sketch| {
        sketch
            .tangent_circle(&curves, radius.as_ref(), near)
            .map(|_| ())
            .map_err(|error| SketchEditError::TangentCircle(error).into())
    })?;
    Ok(Planned::of(transaction))
}

fn blend_end(target: &Target<'_>, on: &BlendOn) -> Result<BlendEnd, ToolError> {
    Ok(BlendEnd {
        curve: target.entity(&on.curve)?,
        point: target.point_of(&on.end)?,
    })
}

pub(super) fn blend(target: &Target<'_>, asked: &Blending) -> Result<Planned, ToolError> {
    let first = blend_end(target, &asked.first)?;
    let second = blend_end(target, &asked.second)?;
    let continuity = match asked.continuity {
        None | Some(Smoothness::Tangent) => Continuity::Tangent,
        Some(Smoothness::Curvature) => Continuity::Curvature,
    };
    let transaction = target.reshaped(blend_curving::TRANSACTION.to_owned(), |sketch| {
        sketch
            .blend(first, second, continuity)
            .map(|_| ())
            .map_err(|error| SketchEditError::Blend(error).into())
    })?;
    Ok(Planned::of(transaction))
}

fn length_or(
    session: &Session,
    field_name: &'static str,
    text: Option<&String>,
    otherwise: &str,
) -> Result<f64, ToolError> {
    Ok(amount(
        session,
        field_name,
        text.map_or(otherwise, String::as_str),
        Dimension::LENGTH,
    )?
    .value)
}

fn bore_of(
    session: &Session,
    field_name: &'static str,
    text: Option<&String>,
) -> Result<f64, ToolError> {
    match text {
        Some(text) => Ok(amount(session, field_name, text, Dimension::LENGTH)?.value),
        None => Ok(0.0),
    }
}

fn chain_named(name: Option<&String>) -> Result<ChainSize, ToolError> {
    name.map_or(Ok(ChainSize::DEFAULT), |name| chain_named_strictly(name))
}

fn chain_named_strictly(name: &str) -> Result<ChainSize, ToolError> {
    ChainSize::find(name).ok_or_else(|| {
        SketchEditError::UnknownChain {
            given: name.to_owned(),
        }
        .into()
    })
}

fn part_of(session: &Session, asked: &Gearing) -> Result<Part, ToolError> {
    let teeth = asked.teeth.unwrap_or(DEFAULT_TEETH);
    let shape = asked.shape.unwrap_or(GearShape::Gear);
    if shape == GearShape::Sprocket {
        return Ok(Part::Sprocket(Box::new(StoredSprocket::literal(
            Sprocket {
                chain: chain_named(asked.chain.as_ref())?,
                teeth,
                bore: bore_of(session, "bore", asked.bore.as_ref())?,
            },
        ))));
    }
    let gear = SpurGear {
        module: length_or(session, "module", asked.module.as_ref(), DEFAULT_MODULE)?,
        teeth,
        pressure_angle: amount(
            session,
            "pressure_angle",
            asked
                .pressure_angle
                .as_deref()
                .unwrap_or(DEFAULT_PRESSURE_ANGLE),
            Dimension::ANGLE,
        )?
        .value,
        profile_shift: asked.profile_shift.unwrap_or(0.0),
        root_fillet: length_or(
            session,
            "root_fillet",
            asked.root_fillet.as_ref(),
            DEFAULT_ROOT_FILLET,
        )?,
        bore: bore_of(session, "bore", asked.bore.as_ref())?,
    };
    Ok(match shape {
        GearShape::Pair => Part::Pair(Box::new(StoredPair::literal(GearPair {
            driver: gear,
            partner_teeth: asked.meshing_teeth.unwrap_or(DEFAULT_PAIR_TEETH),
            partner_bore: bore_of(session, "meshing_bore", asked.meshing_bore.as_ref())?,
            direction: amount(
                session,
                "meshing_direction",
                asked
                    .meshing_direction
                    .as_deref()
                    .unwrap_or(DEFAULT_PAIR_DIRECTION),
                Dimension::ANGLE,
            )?
            .value
            .to_radians(),
        }))),
        GearShape::Gear | GearShape::Sprocket => Part::Gear(Box::new(Spur::literal(gear))),
    })
}

pub(super) fn gear(
    session: &Session,
    target: &Target<'_>,
    asked: &Gearing,
) -> Result<Planned, ToolError> {
    let part = part_of(session, asked)?;
    let centre = match (&asked.center, asked.at) {
        (Some(named), None) => GearCentre::Point(target.point_of(named)?),
        (None, Some(at)) => GearCentre::Free(point(at)?),
        (None, None) => GearCentre::Point(EntityId::ORIGIN),
        (Some(_), Some(_)) => return Err(SketchEditError::GearCentre.into()),
    };
    let transaction = target.reshaped(part.transaction().to_owned(), |sketch| {
        part.drawn_into(sketch, centre)
            .map_err(|error| SketchEditError::Gear(error).into())
    })?;
    Ok(Planned::of(transaction))
}

fn given(
    session: &Session,
    field_name: &'static str,
    amount_given: Option<&Amount>,
    dimension: Dimension,
) -> Result<Option<Dimensioned>, ToolError> {
    amount_given
        .map(|given| amount(session, field_name, &given.to_string(), dimension))
        .transpose()
}

pub(super) fn edit_gear(
    session: &Session,
    target: &Target<'_>,
    asked: &GearEditing,
) -> Result<Planned, ToolError> {
    let (id, held) = references::gear(target.name, target.definition, &asked.gear)?;
    let mut gear = held.clone();
    let parameters = session.parameters();
    let value_of = |parameter| parameters.value(parameter);
    let mut changed = false;
    let mut set = |slot: &mut Expression, change: Option<Dimensioned>| {
        if let Some(change) = change {
            *slot = change.expression;
            changed = true;
        }
    };
    set(
        &mut gear.teeth,
        given(session, "teeth", asked.teeth.as_ref(), Dimension::NONE)?,
    );
    set(
        &mut gear.bore,
        given(session, "bore", asked.bore.as_ref(), Dimension::LENGTH)?,
    );
    let direction = given(
        session,
        "direction",
        asked.direction.as_ref(),
        Dimension::ANGLE,
    )?;
    let followed = [
        (
            "module",
            given(session, "module", asked.module.as_ref(), Dimension::LENGTH)?,
        ),
        (
            "pressure_angle",
            given(
                session,
                "pressure_angle",
                asked.pressure_angle.as_ref(),
                Dimension::ANGLE,
            )?,
        ),
        (
            "profile_shift",
            given(
                session,
                "profile_shift",
                asked.profile_shift.as_ref(),
                Dimension::NONE,
            )?,
        ),
        (
            "root_fillet",
            given(
                session,
                "root_fillet",
                asked.root_fillet.as_ref(),
                Dimension::LENGTH,
            )?,
        ),
        (
            "turn",
            given(session, "turn", asked.turn.as_ref(), Dimension::ANGLE)?,
        ),
    ];
    let label = target.definition.entity_label(id);
    let chain = asked
        .chain
        .as_deref()
        .map(chain_named_strictly)
        .transpose()?;
    match (gear.meshing.as_mut(), gear.chain) {
        (Some(meshing), _) => {
            let driver = meshing.driver;
            if let Some((field, _)) = followed.iter().find(|(_, change)| change.is_some()) {
                return Err(SketchEditError::Followed {
                    gear: label,
                    driver: target.definition.entity_label(driver),
                    field,
                }
                .into());
            }
            if chain.is_some() {
                return Err(SketchEditError::NoChain { gear: label }.into());
            }
            set(&mut meshing.direction, direction);
        }
        (None, Some(_)) => {
            if direction.is_some() {
                return Err(SketchEditError::NoDirection { gear: label }.into());
            }
            let [module, pressure_angle, profile_shift, root_fillet, turn] =
                followed.map(|(_, change)| change);
            let spur_only = [
                ("module", &module),
                ("pressure_angle", &pressure_angle),
                ("profile_shift", &profile_shift),
                ("root_fillet", &root_fillet),
            ];
            if let Some((field, _)) = spur_only.iter().find(|(_, change)| change.is_some()) {
                return Err(SketchEditError::SprocketHas { gear: label, field }.into());
            }
            set(&mut gear.turn, turn);
            if let Some(chain) = chain {
                gear.chain = Some(chain);
                changed = true;
            }
        }
        (None, None) => {
            if direction.is_some() {
                return Err(SketchEditError::NoDirection { gear: label }.into());
            }
            if chain.is_some() {
                return Err(SketchEditError::NoChain { gear: label }.into());
            }
            let [module, pressure_angle, profile_shift, root_fillet, turn] =
                followed.map(|(_, change)| change);
            set(&mut gear.module, module);
            set(&mut gear.pressure_angle, pressure_angle);
            set(&mut gear.profile_shift, profile_shift);
            set(&mut gear.root_fillet, root_fillet);
            set(&mut gear.turn, turn);
        }
    }
    if !changed {
        return Err(SketchEditError::NothingToEdit {
            sketch: target.name.to_owned(),
        }
        .into());
    }
    check_gear_values(target.definition, &gear, &value_of)?;
    let name = if gear.is_sprocket() {
        gearing::EDIT_SPROCKET_TRANSACTION
    } else {
        gearing::EDIT_TRANSACTION
    };
    let mut transaction = target.document.transaction(name);
    transaction.set_sketch_gear(target.feature, id, gear);
    let mut planned = Planned::of(transaction.finish());
    planned.details.insert("gear".to_owned(), json!(id.raw()));
    Ok(planned)
}

fn check_gear_values<F>(sketch: &Sketch, gear: &Gear, value_of: &F) -> Result<(), ToolError>
where
    F: Fn(ParameterId) -> Result<Quantity, EvalError>,
{
    let values = match gear.driver().and_then(|driver| sketch.gear(driver)) {
        None => gear.values(value_of),
        Some(driver) => driver
            .values(value_of)
            .and_then(|driver| gear.meshed_values(&driver, value_of)),
    };
    values
        .map(|_| ())
        .map_err(|error| SketchEditError::GearValues(error).into())
}

pub(super) fn text(
    session: &Session,
    target: &Target<'_>,
    asked: &Lettered,
) -> Result<Planned, ToolError> {
    Text::check_content(&asked.content).map_err(SketchEditError::TextValues)?;
    let height_text = asked
        .height
        .as_ref()
        .map_or_else(|| DEFAULT_TEXT_HEIGHT.to_owned(), ToString::to_string);
    let angle_text = asked
        .angle
        .as_ref()
        .map_or_else(|| DEFAULT_TEXT_ANGLE.to_owned(), ToString::to_string);
    let height = amount(session, "height", &height_text, Dimension::LENGTH)?;
    let angle = amount(session, "angle", &angle_text, Dimension::ANGLE)?;
    let anchor = match (&asked.anchor, asked.at) {
        (Some(named), None) => TextAnchor::Point(target.point_of(named)?),
        (None, Some(at)) => TextAnchor::Free(point(at)?),
        (None, None) => TextAnchor::Point(EntityId::ORIGIN),
        (Some(_), Some(_)) => return Err(SketchEditError::TextAnchor.into()),
    };
    let lettering = Lettering {
        content: asked.content.clone(),
        height,
        angle,
        bold: asked.bold,
        italic: asked.italic,
        align: asked.align.map_or(TextAlign::Left, Alignment::text_align),
    };
    let parameters = session.parameters();
    lettering
        .text(EntityId::ORIGIN)
        .size(&|parameter| parameters.value(parameter))
        .map_err(SketchEditError::TextValues)?;
    let transaction = target.reshaped(texting::TRANSACTION.to_owned(), |sketch| {
        texting::write_into(sketch, &lettering, anchor)
            .map(|_| ())
            .map_err(|error| SketchEditError::TextDrawn(error).into())
    })?;
    Ok(Planned::of(transaction))
}

pub(super) fn edit_text(
    session: &Session,
    target: &Target<'_>,
    asked: &TextEditing,
) -> Result<Planned, ToolError> {
    let (id, held) = references::text(target.name, target.definition, &asked.text)?;
    let mut text = held.clone();
    let height = given(session, "height", asked.height.as_ref(), Dimension::LENGTH)?;
    let angle = given(session, "angle", asked.angle.as_ref(), Dimension::ANGLE)?;
    let changed = asked.content.is_some()
        || height.is_some()
        || angle.is_some()
        || asked.bold.is_some()
        || asked.italic.is_some()
        || asked.align.is_some();
    if !changed {
        return Err(SketchEditError::NothingToEdit {
            sketch: target.name.to_owned(),
        }
        .into());
    }
    if let Some(content) = &asked.content {
        Text::check_content(content).map_err(SketchEditError::TextValues)?;
        text.content = content.replace("\r\n", "\n").replace('\r', "\n");
    }
    if let Some(height) = height {
        text.height = height.expression;
    }
    if let Some(angle) = angle {
        text.angle = angle.expression;
    }
    if let Some(bold) = asked.bold {
        text.bold = bold;
    }
    if let Some(italic) = asked.italic {
        text.italic = italic;
    }
    if let Some(align) = asked.align {
        text.align = align.text_align();
    }
    let parameters = session.parameters();
    text.size(&|parameter| parameters.value(parameter))
        .map_err(SketchEditError::TextValues)?;
    let mut transaction = target.document.transaction(texting::EDIT_TRANSACTION);
    transaction.set_sketch_text(target.feature, id, text);
    let mut planned = Planned::of(transaction.finish());
    planned.details.insert("text".to_owned(), json!(id.raw()));
    Ok(planned)
}

pub(super) fn copy(target: &Target<'_>, asked: &Copying) -> Result<Planned, ToolError> {
    let document = target.document;
    let (source_name, source) = match &asked.from_sketch {
        Some(named) => {
            let (feature, definition) = references::sketch(document, named)?;
            let shown = crate::sketch_status::up_to_date_result(target.evaluation, feature.id())
                .map_or(definition, |result| &result.geometry);
            (feature.name.as_str(), shown)
        }
        None => (target.name, target.shown),
    };
    if asked.items.is_empty() {
        return Err(writing::SketchingError::Nothing.into());
    }
    if asked.items.len() > writing::MOST_ITEMS {
        return Err(writing::SketchingError::TooMany.into());
    }
    let items = asked
        .items
        .iter()
        .map(|named| references::entity(source_name, source, named).map_err(ToolError::from))
        .collect::<Result<Vec<_>, _>>()?;
    let clip = source.clip(&items).map_err(SketchEditError::Clip)?;
    let same_sketch = asked.from_sketch.is_none();
    let offset = match (asked.by, asked.to) {
        (Some([x, y]), None) => Vector2::new(writing::coordinate(x)?, writing::coordinate(y)?),
        (None, Some(to)) => {
            let centre = clip.centre().unwrap_or_default();
            point(to)? - centre
        }
        (None, None) if same_sketch => clipboard::same_sketch_shift(&clip),
        (None, None) => Vector2::ZERO,
        (Some(_), Some(_)) => return Err(SketchEditError::CopyTarget.into()),
    };
    let mut placed = Vec::new();
    let label = format!("Paste {}", clipboard::described(&clip));
    let transaction = target.reshaped(label, |sketch| {
        placed = sketch
            .paste(&clip, offset)
            .map_err(|error| ToolError::from(SketchEditError::Clip(error)))?;
        Ok(())
    })?;
    let mut planned = Planned::of(transaction);
    planned
        .details
        .insert("pasted".to_owned(), writing::ids(&placed));
    planned
        .details
        .insert("offset".to_owned(), json!([offset.x, offset.y]));
    Ok(planned)
}
