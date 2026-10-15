use caditor_expression::Dimension;
use caditor_geometry::Vector2;
use caditor_sketch::{
    BlendEnd, ChainSize, Continuity, EntityId, GearCentre, GearPair, Sprocket, SpurGear,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    blend_curving, clipboard,
    gearing::Part,
    mcp::{
        references::{self, EntityName, Named},
        session::Session,
        sketch_edits::{Planned, SketchEditError, Target, amount},
        tools::ToolError,
        writing::{self, point},
    },
    tangent_circling,
};

const DEFAULT_MODULE: &str = "2 mm";
const DEFAULT_TEETH: u32 = 20;
const DEFAULT_PAIR_TEETH: u32 = 30;
const DEFAULT_PRESSURE_ANGLE: &str = "20 deg";
const DEFAULT_ROOT_FILLET: &str = "0.75 mm";

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
    pub chain: Option<String>,
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
    let Some(name) = name else {
        return Ok(ChainSize::DEFAULT);
    };
    ChainSize::ALL
        .into_iter()
        .find(|size| size.designation.eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| {
            SketchEditError::UnknownChain {
                given: name.clone(),
            }
            .into()
        })
}

fn part_of(session: &Session, asked: &Gearing) -> Result<Part, ToolError> {
    let teeth = asked.teeth.unwrap_or(DEFAULT_TEETH);
    let shape = asked.shape.unwrap_or(GearShape::Gear);
    if shape == GearShape::Sprocket {
        return Ok(Part::Sprocket(Sprocket {
            chain: chain_named(asked.chain.as_ref())?,
            teeth,
            bore: bore_of(session, "bore", asked.bore.as_ref())?,
        }));
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
        GearShape::Pair => Part::Pair(GearPair {
            driver: gear,
            partner_teeth: asked.meshing_teeth.unwrap_or(DEFAULT_PAIR_TEETH),
            partner_bore: bore_of(session, "meshing_bore", asked.meshing_bore.as_ref())?,
        }),
        GearShape::Gear | GearShape::Sprocket => Part::Gear(gear),
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
