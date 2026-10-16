use std::borrow::Cow;

use caditor_document::{
    BodyOperation, Document, Edit, Evaluation, FeatureId, FeatureKind, Pieces, RegionChoice,
    SolidFeature, SolidResult, Sweep, SweepPath, Transaction, profile_curve,
};
use caditor_kernel::{EdgeName, EdgeNaming, EdgeReference};
use caditor_sketch::EntityId;

use crate::{
    bodies, body_selection,
    commands::{Command, Reason},
    editing::{self, EditingCommand, SketchEditing},
    field::{self, CheckError},
    helix_tools,
    model::{Action, Model},
    queued::{self, Pending},
    reference_picking::{Picking, Slot},
    selection::{Pickable, Selection},
    solid_tools::{self, SweepSource},
    visibility,
};

pub const TITLE: &str = "Sweep";
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SweepError {
    #[error(
        "Select the regions or curves of a sketch to sweep, with the \
         curves of another sketch or edges of a body to sweep along"
    )]
    NothingToSweep,
    #[error(
        "Curves of more than two sketches are selected; select the \
         profile's sketch and the curves of one sketch to follow"
    )]
    SeveralPathSketches,
    #[error("Edges of several bodies are selected; select the edges of the path on one body only")]
    PathEdgesOfSeveralBodies,
    #[error(
        "Both sketch curves and body edges are selected for the path; \
         select one or the other"
    )]
    CurvesAndEdges,
    #[error("Click a curve of a sketch other than the profile's, an edge of a body or a helix")]
    NotAPath,
    #[error(
        "Curves of more than one sketch are selected; select the curves of one sketch to follow"
    )]
    SeveralSketches,
    #[error("Click a curve of a sketch, an edge of a body or a helix")]
    NotACurveOrEdge,
    #[error("{}", Pending::BodyShape)]
    NoBodyShape,
    #[error("It already follows the selected path")]
    AlreadyFollowed,
    #[error(
        "A helix and curves or edges are selected for the path; select \
         one helix, or the curves or edges"
    )]
    HelixAndOthers,
    #[error("Some of the selected edges are no longer part of the model")]
    EdgesGone,
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error(transparent)]
    Check(#[from] CheckError),
}

impl Reason for SweepError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }

    fn pending(&self) -> Option<Pending> {
        matches!(self, Self::NoBodyShape).then_some(Pending::BodyShape)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathPick {
    Unchosen,
    Curves {
        sketch: FeatureId,
        curves: Vec<EntityId>,
    },
    Edges {
        body: FeatureId,
        edges: Vec<EdgeName>,
    },
    Helix {
        helix: FeatureId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SweepStart {
    pub profile: SweepSource,
    pub path: PathPick,
}

pub fn is_path_curve(document: &Document, sketch: FeatureId, entity: EntityId) -> bool {
    entity.reference().is_none()
        && editing::edited_sketch(document, sketch)
            .and_then(|definition| profile_curve(definition, entity))
            .is_some()
}

pub fn selected_helices(document: &Document, selection: &Selection) -> Vec<FeatureId> {
    selection
        .in_pick_order()
        .into_iter()
        .filter_map(|pickable| match pickable {
            Pickable::Datum(feature) if helix_tools::is_helix(document, feature) => Some(feature),
            _ => None,
        })
        .collect()
}

pub fn helix_path(
    document: &Document,
    selection: &Selection,
    other_path: bool,
) -> Result<Option<FeatureId>, SweepError> {
    match (selected_helices(document, selection).as_slice(), other_path) {
        ([], _) => Ok(None),
        ([_], true) | ([_, _, ..], _) => Err(SweepError::HelixAndOthers),
        ([helix], false) => Ok(Some(*helix)),
    }
}

fn has_regions(evaluation: &Evaluation, sketch: FeatureId) -> bool {
    crate::selection::sketch_regions(evaluation, sketch).is_some_and(|regions| !regions.is_empty())
}

fn guessed_profile(
    document: &Document,
    evaluation: &Evaluation,
    path_sketch: Option<FeatureId>,
) -> Option<FeatureId> {
    document
        .active_features()
        .rev()
        .filter(|feature| feature.kind.sketch().is_some())
        .map(|feature| feature.id())
        .filter(|sketch| Some(*sketch) != path_sketch)
        .filter(|sketch| visibility::is_shown(document, *sketch))
        .filter(|sketch| has_regions(evaluation, *sketch))
        .find(|sketch| solid_tools::has_unswept_regions(document, evaluation, *sketch))
}

pub fn source(
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
) -> Result<SweepStart, SweepError> {
    let document = model.document();
    let evaluation = model.evaluation();
    let picked = selection.in_pick_order();
    let mut curve_sketches: Vec<FeatureId> = Vec::new();
    let mut region_sketch = None;
    let mut edges: Vec<(FeatureId, EdgeName)> = Vec::new();
    for pickable in &picked {
        match *pickable {
            Pickable::SketchRegion { feature, .. } => {
                region_sketch.get_or_insert(feature);
            }
            Pickable::SketchEntity { feature, entity }
                if is_path_curve(document, feature, entity)
                    && !curve_sketches.contains(&feature) =>
            {
                curve_sketches.push(feature);
            }
            Pickable::Edge { body, edge } => edges.push((body, edge)),
            _ => {}
        }
    }
    let chosen_profile = editing
        .feature()
        .or(region_sketch)
        .or_else(|| {
            curve_sketches
                .iter()
                .copied()
                .find(|sketch| has_regions(evaluation, *sketch))
                .filter(|_| curve_sketches.len() > 1 || !edges.is_empty())
        })
        .or_else(|| match curve_sketches.as_slice() {
            [only] if edges.is_empty() && has_regions(evaluation, *only) => Some(*only),
            _ => None,
        });
    let path_sketches: Vec<FeatureId> = curve_sketches
        .iter()
        .copied()
        .filter(|sketch| Some(*sketch) != chosen_profile)
        .collect();
    let helix = helix_path(
        document,
        selection,
        !path_sketches.is_empty() || !edges.is_empty(),
    )?;
    let path = match (path_sketches.as_slice(), edges.first()) {
        ([], None) => helix.map_or(PathPick::Unchosen, |helix| PathPick::Helix { helix }),
        ([_, ..], Some(_)) => return Err(SweepError::CurvesAndEdges),
        ([_, _, ..], None) => return Err(SweepError::SeveralPathSketches),
        ([sketch], None) => PathPick::Curves {
            sketch: *sketch,
            curves: picked
                .iter()
                .filter_map(|pickable| match *pickable {
                    Pickable::SketchEntity { feature, entity }
                        if feature == *sketch && is_path_curve(document, feature, entity) =>
                    {
                        Some(entity)
                    }
                    _ => None,
                })
                .collect(),
        },
        ([], Some((body, _))) => {
            if edges.iter().any(|(other, _)| other != body) {
                return Err(SweepError::PathEdgesOfSeveralBodies);
            }
            PathPick::Edges {
                body: *body,
                edges: edges.iter().map(|(_, edge)| *edge).collect(),
            }
        }
    };
    let path_sketch = match &path {
        PathPick::Curves { sketch, .. } => Some(*sketch),
        PathPick::Unchosen | PathPick::Edges { .. } | PathPick::Helix { .. } => None,
    };
    let sketch = chosen_profile
        .or_else(|| guessed_profile(document, evaluation, path_sketch))
        .ok_or(SweepError::NothingToSweep)?;
    let path_body = match &path {
        PathPick::Edges { body, .. } => Some(*body),
        PathPick::Unchosen | PathPick::Curves { .. } | PathPick::Helix { .. } => None,
    };
    let body = path_body.or_else(|| match body_selection::bodies_in(selection).as_slice() {
        [only] => Some(*only),
        _ => None,
    });
    let profile = solid_tools::with_selected_outline(
        model,
        solid_tools::Sweep::Extrude,
        SweepSource {
            sketch,
            axis: None,
            regions: RegionChoice::All,
            body,
        },
        selection,
    );
    Ok(SweepStart { profile, path })
}

fn captured_edges(
    body: &SolidResult,
    edges: &[EdgeName],
) -> Result<Vec<EdgeReference>, SweepError> {
    let naming = EdgeNaming::new(&body.solid);
    edges
        .iter()
        .map(|name| {
            bodies::find_edge(body, *name)
                .and_then(|edge| EdgeReference::capture_in(&naming, edge))
                .ok_or(SweepError::EdgesGone)
        })
        .collect()
}

pub fn captured(evaluation: &Evaluation, path: &PathPick) -> Result<SweepPath, SweepError> {
    Ok(match path {
        PathPick::Unchosen => SweepPath::Unchosen,
        PathPick::Curves { sketch, curves } => SweepPath::Curves {
            sketch: *sketch,
            curves: curves.clone(),
        },
        PathPick::Edges { body, edges } => {
            let shown = bodies::shown(evaluation, *body).ok_or(SweepError::NoBodyShape)?;
            SweepPath::Edges {
                body: *body,
                edges: captured_edges(shown, edges)?,
            }
        }
        PathPick::Helix { helix } => SweepPath::Helix { helix: *helix },
    })
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    start: &SweepStart,
    pieces: Pieces,
) -> Result<(Transaction, FeatureId), SweepError> {
    let path = captured(evaluation, &start.path)?;
    let name = editing::next_feature_name(document, TITLE);
    let operation = start
        .profile
        .body
        .map_or(BodyOperation::NewBody, BodyOperation::Add);
    let sweep = Sweep {
        regions: start.profile.regions.clone(),
        operation,
        ..Sweep::new(start.profile.sketch, path)
    };
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, FeatureKind::Solid(SolidFeature::Sweep(sweep)));
    if pieces == Pieces::BodyEach {
        transaction.edit(Edit::SetFeaturePieces {
            id: feature,
            pieces,
        });
    }
    if visibility::is_shown(document, start.profile.sketch) {
        transaction.edit(Edit::SetFeatureHidden {
            id: start.profile.sketch,
            hidden: true,
        });
    }
    Ok((transaction.finish(), feature))
}

pub fn create_actions(model: &Model, start: &SweepStart) -> Vec<Action> {
    match create(
        model.document(),
        model.evaluation(),
        start,
        model.new_body_pieces(),
    ) {
        Ok((transaction, feature)) => {
            let mut actions = vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ];
            if start.path == PathPick::Unchosen {
                actions.push(Action::Editing(EditingCommand::Pick(Picking::new(
                    feature,
                    Slot::SweepPath,
                ))));
            }
            actions
        }
        Err(reason) => vec![queued::refused(Command::Sweep, TITLE, &reason)],
    }
}

fn path_body_state(
    evaluation: &Evaluation,
    feature: FeatureId,
    body: FeatureId,
) -> Option<&SolidResult> {
    evaluation
        .body_result_seen_by(feature, body)
        .or_else(|| bodies::shown(evaluation, body))
}

pub fn change(model: &Model, feature: FeatureId, sweep: Sweep) -> Result<Transaction, SweepError> {
    let document = model.document();
    let transaction = solid_tools::edit(document, feature, SolidFeature::Sweep(sweep))
        .ok_or(SweepError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn selected_path(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    profile: Option<FeatureId>,
) -> Result<SweepPath, SweepError> {
    let document = model.document();
    let evaluation = model.evaluation();
    let mut curves: Vec<(FeatureId, EntityId)> = Vec::new();
    let mut edges: Vec<(FeatureId, EdgeName)> = Vec::new();
    for pickable in selection.in_pick_order() {
        match pickable {
            Pickable::SketchEntity {
                feature: sketch,
                entity,
            } if Some(sketch) != profile && is_path_curve(document, sketch, entity) => {
                curves.push((sketch, entity));
            }
            Pickable::Edge { body, edge } => edges.push((body, edge)),
            _ => {}
        }
    }
    if let Some(helix) = helix_path(document, selection, !curves.is_empty() || !edges.is_empty())? {
        return Ok(SweepPath::Helix { helix });
    }
    match (curves.first(), edges.first()) {
        (None, None) => Err(match profile {
            Some(_) => SweepError::NotAPath,
            None => SweepError::NotACurveOrEdge,
        }),
        (Some(_), Some(_)) => Err(SweepError::CurvesAndEdges),
        (Some((sketch, _)), None) => {
            if curves.iter().any(|(other, _)| other != sketch) {
                return Err(match profile {
                    Some(_) => SweepError::SeveralPathSketches,
                    None => SweepError::SeveralSketches,
                });
            }
            Ok(SweepPath::Curves {
                sketch: *sketch,
                curves: curves.iter().map(|(_, curve)| *curve).collect(),
            })
        }
        (None, Some((body, _))) => {
            if edges.iter().any(|(other, _)| other != body) {
                return Err(SweepError::PathEdgesOfSeveralBodies);
            }
            let state =
                path_body_state(evaluation, feature, *body).ok_or(SweepError::NoBodyShape)?;
            let names: Vec<EdgeName> = edges.iter().map(|(_, edge)| *edge).collect();
            Ok(SweepPath::Edges {
                body: *body,
                edges: captured_edges(state, &names)?,
            })
        }
    }
}

pub fn toggled_path(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    (current, profile): (&SweepPath, Option<FeatureId>),
) -> Result<SweepPath, SweepError> {
    let picked = selected_path(model, selection, feature, profile)?;
    Ok(match (current, picked) {
        (
            SweepPath::Curves { sketch, curves },
            SweepPath::Curves {
                sketch: clicked,
                curves: added,
            },
        ) if *sketch == clicked => {
            let mut kept = curves.clone();
            for curve in added {
                match kept.iter().position(|held| *held == curve) {
                    Some(index) => {
                        kept.remove(index);
                    }
                    None => kept.push(curve),
                }
            }
            if kept.is_empty() {
                SweepPath::Unchosen
            } else {
                SweepPath::Curves {
                    sketch: *sketch,
                    curves: kept,
                }
            }
        }
        (
            SweepPath::Edges { body, edges },
            SweepPath::Edges {
                body: clicked,
                edges: added,
            },
        ) if *body == clicked => {
            let mut kept = edges.clone();
            for edge in added {
                match kept.iter().position(|held| held.name() == edge.name()) {
                    Some(index) => {
                        kept.remove(index);
                    }
                    None => kept.push(edge),
                }
            }
            if kept.is_empty() {
                SweepPath::Unchosen
            } else {
                SweepPath::Edges {
                    body: *body,
                    edges: kept,
                }
            }
        }
        (_, picked) => picked,
    })
}

pub fn selected_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    sweep: &Sweep,
) -> Result<Transaction, SweepError> {
    let path = selected_path(model, selection, feature, Some(sweep.sketch))?;
    if path == sweep.path {
        return Err(SweepError::AlreadyFollowed);
    }
    change(
        model,
        feature,
        Sweep {
            path,
            ..sweep.clone()
        },
    )
}

pub fn path_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    sweep: &Sweep,
) -> Result<Transaction, SweepError> {
    let path = toggled_path(model, selection, feature, (&sweep.path, Some(sweep.sketch)))?;
    change(
        model,
        feature,
        Sweep {
            path,
            ..sweep.clone()
        },
    )
}

pub fn path_words(document: &Document, path: &SweepPath) -> String {
    let name = |feature: FeatureId| {
        document.feature(feature).map_or_else(
            || "a missing feature".to_owned(),
            |feature| feature.name.clone(),
        )
    };
    let count = |count: usize, one: &str, many: &str| {
        if count == 1 {
            format!("1 {one}")
        } else {
            format!("{count} {many}")
        }
    };
    match path {
        SweepPath::Unchosen => "None chosen".to_owned(),
        SweepPath::Curves { sketch, curves } => {
            format!(
                "{} of {}",
                count(curves.len(), "curve", "curves"),
                name(*sketch)
            )
        }
        SweepPath::Edges { body, edges } => {
            format!("{} of {}", count(edges.len(), "edge", "edges"), name(*body))
        }
        SweepPath::Helix { helix } => name(*helix),
    }
}
