use caditor_document::{
    BlendKind, Datum, Document, Edit, FeatureId, FeatureKind, PatternKind, SolidFeature,
    Transaction,
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use crate::{
    blend_tools, coil_tools, combine_tools, datum_tools, delete_face_tools, emboss_tools,
    helix_tools, hole_tools, loft_tools, mate_tools,
    mcp::{
        curves, datums,
        features::{self, Built},
        mates,
        places::PlaceError,
        references::{self, Named, ReferenceError},
        reshaping,
        session::{Session, Work},
        solids, sweeps,
        tools::{Answer, Tool, ToolError},
        writing::{apply, with_feature},
    },
    mirror_tools, move_face_tools, move_tools, offset_face_tools, pattern_tools, pipe_tools,
    primitive_tools, rib_tools, scale_tools, shell_tools, solid_tools, split_face_tools,
    split_tools, thread_tools,
};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Editing {
    pub feature: Named,
    #[serde(flatten)]
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FeatureEditError {
    #[error(
        "{name} is a sketch: change it with add_sketch_geometry, add_sketch_constraints, \
         modify_sketch, edit_sketch_constraint and remove_sketch_items"
    )]
    Sketch { name: String },
    #[error("{name} is a kind of feature this server cannot edit yet; edit it in the app")]
    NotEditable { name: String },
    #[error("say what to change in {name}, with the arguments {tool} takes")]
    NothingGiven { name: String, tool: &'static str },
    #[error(
        "{body} had no face {face} as {feature} received it; list_faces with before set to \
         {feature} gives the faces to name when editing it"
    )]
    NoFaceBefore {
        feature: String,
        body: String,
        face: String,
    },
    #[error(
        "{body} had no edge {edge} as {feature} received it; list_edges with before set to \
         {feature} gives the edges to name when editing it"
    )]
    NoEdgeBefore {
        feature: String,
        body: String,
        edge: String,
    },
    #[error("{name} works on no body that exists before it, so there is none to list")]
    NothingBefore { name: String },
}

fn as_received(error: ToolError, feature: &str) -> ToolError {
    match error {
        ToolError::Reference(ReferenceError::NoFace { body, face }) => {
            FeatureEditError::NoFaceBefore {
                feature: feature.to_owned(),
                body,
                face,
            }
            .into()
        }
        ToolError::Place(PlaceError::NoEdge { body, edge }) => FeatureEditError::NoEdgeBefore {
            feature: feature.to_owned(),
            body,
            edge,
        }
        .into(),
        other => other,
    }
}

pub fn viewed_before<T>(
    session: &mut Session,
    before: Option<&Named>,
    run: impl FnOnce(&Session) -> Result<T, ToolError>,
) -> Result<T, ToolError> {
    let Some(named) = before else {
        return run(session);
    };
    let found = references::feature(session.document(), named)?;
    let (feature, name) = (found.id(), found.name.clone());
    session
        .looking_before(feature, |session| run(session))
        .ok_or(FeatureEditError::NothingBefore { name })?
}

pub fn tool_for(kind: &FeatureKind) -> Option<Tool> {
    Some(match kind {
        FeatureKind::Solid(SolidFeature::Extrude(_)) => Tool::AddExtrusion,
        FeatureKind::Solid(SolidFeature::Revolve(_)) => Tool::AddRevolve,
        FeatureKind::Solid(SolidFeature::Sweep(_)) => Tool::AddSweep,
        FeatureKind::Hole(_) => Tool::AddHole,
        FeatureKind::Blend(blend) => match blend.kind {
            BlendKind::Fillet => Tool::AddFillet,
            BlendKind::Chamfer => Tool::AddChamfer,
        },
        FeatureKind::Shell(_) => Tool::AddShell,
        FeatureKind::OffsetFace(_) => Tool::AddOffsetFace,
        FeatureKind::Primitive(_) => Tool::AddPrimitive,
        FeatureKind::Mirror(_) => Tool::AddMirror,
        FeatureKind::Pattern(pattern) => match pattern.kind {
            PatternKind::Linear { .. } => Tool::AddLinearPattern,
            PatternKind::Circular(_) => Tool::AddCircularPattern,
            PatternKind::Curve(_) => Tool::AddCurvePattern,
            PatternKind::Points(_) => Tool::AddPointPattern,
        },
        FeatureKind::Combine(_) => Tool::AddCombine,
        FeatureKind::Move(_) => Tool::AddMove,
        FeatureKind::Datum(Datum::Plane(_) | Datum::PlaneThrough(_)) => Tool::AddDatumPlane,
        FeatureKind::Datum(Datum::Axis(_)) => Tool::AddDatumAxis,
        FeatureKind::Datum(Datum::Point(_) | Datum::PointBy(_)) => Tool::AddDatumPoint,
        FeatureKind::Datum(Datum::Frame(_)) => Tool::AddCoordinateSystem,
        FeatureKind::Mate(_) => Tool::AddMate,
        FeatureKind::Helix(_) => Tool::AddHelix,
        FeatureKind::Pipe(_) => Tool::AddPipe,
        FeatureKind::Loft(_) => Tool::AddLoft,
        FeatureKind::Coil(_) => Tool::AddCoil,
        FeatureKind::Rib(_) => Tool::AddRib,
        FeatureKind::Thread(_) => Tool::AddThread,
        FeatureKind::SplitFace(_) => Tool::AddSplitFace,
        FeatureKind::DeleteFace(_) => Tool::AddDeleteFace,
        FeatureKind::Emboss(_) => Tool::AddEmboss,
        FeatureKind::MoveFace(_) => Tool::AddMoveFace,
        FeatureKind::Split(_) => Tool::AddSplit,
        FeatureKind::Scale(_) => Tool::AddScale,
        FeatureKind::Sketch(_)
        | FeatureKind::Import(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Measurement(_)
        | FeatureKind::ReferenceImage(_) => return None,
    })
}

fn decoded<T: DeserializeOwned>(tool: Tool, values: &Map<String, Value>) -> Result<T, ToolError> {
    serde_json::from_value(Value::Object(values.clone())).map_err(|source| ToolError::Arguments {
        tool: tool.name(),
        source,
    })
}

fn rebuilt(
    session: &mut Session,
    work: &Work<'_>,
    (tool, kind): (Tool, &FeatureKind),
    values: &Map<String, Value>,
) -> Result<Option<Built>, ToolError> {
    Ok(Some(match kind {
        FeatureKind::Solid(SolidFeature::Extrude(base)) => {
            solids::extrude_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Solid(SolidFeature::Revolve(base)) => {
            features::revolve_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Solid(SolidFeature::Sweep(base)) => {
            sweeps::sweep_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Hole(base) => {
            solids::drill_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Blend(base) => match base.kind {
            BlendKind::Fillet => {
                features::fillet_built(session, work, decoded(tool, values)?, Some(base))?
            }
            BlendKind::Chamfer => {
                features::chamfer_built(session, work, decoded(tool, values)?, Some(base))?
            }
        },
        FeatureKind::Shell(base) => {
            features::shell_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::OffsetFace(base) => {
            features::offset_face_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Primitive(base) => {
            features::primitive_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Mirror(base) => {
            features::mirror_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Pattern(base) => match base.kind {
            PatternKind::Circular(_) => features::circular_pattern_built(
                session,
                work,
                decoded(tool, values)?,
                Some(&**base),
            )?,
            PatternKind::Curve(_) => {
                curves::curve_pattern_built(session, work, decoded(tool, values)?, Some(&**base))?
            }
            PatternKind::Points(_) => {
                curves::point_pattern_built(session, work, decoded(tool, values)?, Some(&**base))?
            }
            PatternKind::Linear { .. } => features::linear_pattern_built(
                session,
                work,
                decoded(tool, values)?,
                Some(&**base),
            )?,
        },
        FeatureKind::Combine(base) => {
            features::combine_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Move(base) => {
            features::move_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Datum(base @ (Datum::Plane(_) | Datum::PlaneThrough(_))) => {
            datums::datum_plane_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Datum(Datum::Axis(base)) => {
            datums::datum_axis_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Datum(base @ (Datum::Point(_) | Datum::PointBy(_))) => {
            datums::datum_point_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Datum(Datum::Frame(base)) => {
            datums::coordinate_system_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Mate(base) => {
            mates::mate_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Helix(base) => {
            curves::helix_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Pipe(base) => {
            sweeps::pipe_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Loft(base) => {
            sweeps::loft_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Coil(base) => {
            sweeps::coil_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Rib(base) => {
            sweeps::rib_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Thread(base) => {
            sweeps::thread_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::SplitFace(base) => {
            reshaping::split_face_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Emboss(base) => {
            reshaping::emboss_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::DeleteFace(base) => {
            reshaping::delete_face_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::MoveFace(base) => {
            reshaping::move_face_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Split(base) => {
            reshaping::split_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Scale(base) => {
            reshaping::scale_built(session, work, decoded(tool, values)?, Some(base))?
        }
        FeatureKind::Sketch(_)
        | FeatureKind::Import(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Measurement(_)
        | FeatureKind::ReferenceImage(_) => return Ok(None),
    }))
}

fn edit_transaction(
    document: &Document,
    feature: FeatureId,
    kind: FeatureKind,
) -> Option<Transaction> {
    match kind {
        FeatureKind::Solid(solid) => solid_tools::edit(document, feature, solid),
        FeatureKind::Blend(blend) => blend_tools::edit(document, feature, blend),
        FeatureKind::Hole(hole) => hole_tools::edit(document, feature, hole),
        FeatureKind::Shell(shell) => shell_tools::edit(document, feature, shell),
        FeatureKind::OffsetFace(offset) => offset_face_tools::edit(document, feature, offset),
        FeatureKind::Primitive(primitive) => primitive_tools::edit(document, feature, primitive),
        FeatureKind::Mirror(mirror) => mirror_tools::edit(document, feature, mirror),
        FeatureKind::Pattern(pattern) => pattern_tools::edit(document, feature, *pattern),
        FeatureKind::Combine(combine) => combine_tools::edit(document, feature, combine),
        FeatureKind::Move(movement) => move_tools::edit(document, feature, movement),
        FeatureKind::Datum(datum) => datum_tools::edit(document, feature, datum),
        FeatureKind::Pipe(pipe) => pipe_tools::edit(document, feature, pipe),
        FeatureKind::Loft(loft) => loft_tools::edit(document, feature, loft),
        FeatureKind::Coil(coil) => coil_tools::edit(document, feature, coil),
        FeatureKind::Rib(rib) => rib_tools::edit(document, feature, rib),
        FeatureKind::Thread(thread) => thread_tools::edit(document, feature, thread),
        FeatureKind::SplitFace(split) => split_face_tools::edit(document, feature, split),
        FeatureKind::DeleteFace(deletion) => delete_face_tools::edit(document, feature, deletion),
        FeatureKind::Emboss(emboss) => emboss_tools::edit(document, feature, emboss),
        FeatureKind::MoveFace(moving) => move_face_tools::edit(document, feature, moving),
        FeatureKind::Split(split) => split_tools::edit(document, feature, split),
        FeatureKind::Scale(scale) => scale_tools::edit(document, feature, scale),
        FeatureKind::Mate(mate) => mate_tools::edit(document, feature, mate),
        FeatureKind::Helix(helix) => helix_tools::edit(document, feature, helix),
        other => {
            let name = &document.feature(feature)?.name;
            Some(Transaction::single(
                format!("Edit {name}"),
                Edit::SetFeatureKind {
                    id: feature,
                    kind: other,
                },
            ))
        }
    }
}

pub fn edit(session: &mut Session, work: &Work<'_>, asked: Editing) -> Result<Answer, ToolError> {
    let found = references::feature(session.document(), &asked.feature)?;
    let feature = found.id();
    let name = found.name.clone();
    let kind = found.kind.clone();
    if kind.sketch().is_some() {
        return Err(FeatureEditError::Sketch { name }.into());
    }
    let Some(tool) = tool_for(&kind) else {
        return Err(FeatureEditError::NotEditable { name }.into());
    };
    if asked.values.is_empty() {
        return Err(FeatureEditError::NothingGiven {
            name,
            tool: tool.name(),
        }
        .into());
    }
    session.evaluate(work)?;
    let rebuilding = |session: &mut Session| rebuilt(session, work, (tool, &kind), &asked.values);
    let built = match session.looking_before(feature, rebuilding) {
        Some(result) => result.map_err(|error| as_received(error, &name))?,
        None => rebuilding(session)?,
    };
    let Some(built) = built else {
        return Err(FeatureEditError::NotEditable { name }.into());
    };
    let document = session.document();
    let transaction = edit_transaction(document, feature, built.kind)
        .ok_or_else(|| references::ReferenceError::NoFeature(asked.feature.clone()))?;
    let (label, mut edits) = transaction.into_parts();
    if let Some(renamed) = built.name.filter(|renamed| *renamed != name) {
        edits.push(Edit::RenameFeature {
            id: feature,
            name: renamed,
        });
    }
    let answer = apply(session, work, Transaction::new(label, edits), built.details)?;
    Ok(with_feature(session, answer, feature))
}
