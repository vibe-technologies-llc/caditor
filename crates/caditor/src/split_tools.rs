use std::borrow::Cow;

use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, FeatureResult, PlaneReference, PrincipalPlane, Split,
    SplitAlong, Transaction, capitalized, describe_plane, describe_surface, is_open_chain,
};

use crate::{
    body_selection,
    commands::Reason,
    datum_tools,
    editing::{self, EditingCommand},
    field::{self, CheckError},
    model::{Action, Model},
    move_tools,
    selection::{Pickable, Selection},
    sketch_placement::{self, FaceChoice},
};

pub const TITLE: &str = "Split body";
const NO_BODY: &str = "Select a face or edge of the body to split";
const DEFAULT_PLANE: PlaneReference = PlaneReference::Principal(PrincipalPlane::Yz);

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SplitError {
    #[error("Select a plane, face, sketch curve or another body made before this feature")]
    NoTool,
    #[error("The body is already split along the selected plane, curve or body")]
    Already,
    #[error(
        "Curves of several sketches are selected; select curves of only the one to split along"
    )]
    SeveralSketches,
    #[error("The selected sketch comes later in the tree; select one made before this feature")]
    SketchMadeLater { sketch: FeatureId },
    #[error("Several other bodies are selected; select only the one to split along")]
    SeveralTools,
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("{0}")]
    ChosenPlane(String),
    #[error(transparent)]
    SurfaceAttachment(crate::sketch_placement::AttachmentError),
    #[error(transparent)]
    ChosenBody(#[from] move_tools::BodyChoiceError),
    #[error(transparent)]
    Checked(#[from] CheckError),
}

impl Reason for SplitError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplitSource {
    pub body: FeatureId,
    pub along: Option<SplitAlong>,
}

pub fn describe(document: &Document, along: &SplitAlong) -> String {
    match along {
        SplitAlong::Plane(plane) => capitalized(&describe_plane(document, plane)),
        SplitAlong::Body(tool) => document
            .body_label(*tool)
            .unwrap_or_else(|| "A deleted body".to_owned()),
        SplitAlong::Sketch(sketch) => document.feature(*sketch).map_or_else(
            || "The curve of a deleted sketch".to_owned(),
            |sketch| format!("The curve of {}", sketch.name),
        ),
        SplitAlong::Surface(face) => capitalized(&describe_surface(document, face)),
    }
}

pub fn choices(model: &Model, feature: FeatureId, split: &Split) -> Vec<SplitAlong> {
    let document = model.document();
    let open = |sketch: FeatureId| {
        model
            .evaluation()
            .feature(sketch)
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::sketch)
            .is_some_and(|result| is_open_chain(&result.geometry))
    };
    let index = document.feature_index(feature).unwrap_or(0);
    let planes = datum_tools::listed_planes(document, feature)
        .into_iter()
        .map(SplitAlong::Plane);
    let bodies = document
        .bodies_before_in(feature, model.evaluation())
        .into_iter()
        .filter(|body| *body != split.body)
        .map(SplitAlong::Body);
    let sketches = document
        .features()
        .take(index)
        .filter(|earlier| matches!(earlier.kind, FeatureKind::Sketch(_)) && open(earlier.id()))
        .map(|sketch| SplitAlong::Sketch(sketch.id()));
    planes.chain(bodies).chain(sketches).collect()
}

fn chosen_sketch(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<Option<SplitAlong>, SplitError> {
    let document = model.document();
    let mut sketches: Vec<FeatureId> = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::SketchEntity { feature, .. } | Pickable::SketchRegion { feature, .. } =
            pickable
            && !sketches.contains(&feature)
        {
            sketches.push(feature);
        }
    }
    match sketches.as_slice() {
        [] => Ok(None),
        [sketch]
            if document
                .feature_index(*sketch)
                .is_some_and(|position| position < index) =>
        {
            Ok(Some(SplitAlong::Sketch(*sketch)))
        }
        [sketch] => Err(SplitError::SketchMadeLater { sketch: *sketch }),
        [_, _, ..] => Err(SplitError::SeveralSketches),
    }
}

fn chosen_surface(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<Option<(SplitAlong, Option<Pickable>)>, SplitError> {
    let curved: Vec<(Pickable, FaceChoice)> = selection
        .iter()
        .filter_map(|pickable| Some((pickable, FaceChoice::of(pickable)?)))
        .filter(|(pickable, _)| datum_tools::plane_reference(model, *pickable, index).is_none())
        .collect();
    let [(pickable, choice)] = curved.as_slice() else {
        return Ok(None);
    };
    let face = sketch_placement::surface_at(model, *choice, index)
        .map_err(SplitError::SurfaceAttachment)?;
    Ok(Some((SplitAlong::Surface(face), Some(*pickable))))
}

pub fn chosen_along(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<Option<(SplitAlong, Option<Pickable>)>, SplitError> {
    if let Some(chosen) = datum_tools::chosen_plane(model, selection, index)
        .map_err(|error| SplitError::ChosenPlane(error.into()))?
    {
        return Ok(Some((SplitAlong::Plane(chosen.plane), chosen.face)));
    }
    if let Some(along) = chosen_sketch(model, selection, index)? {
        return Ok(Some((along, None)));
    }
    chosen_surface(model, selection, index)
}

pub fn source(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<SplitSource, SplitError> {
    let index = model.document().bar_index();
    let chosen = chosen_along(model, selection, index)?;
    if chosen.is_none()
        && tree.is_empty()
        && let [body, tool] = body_selection::bodies_in(selection).as_slice()
    {
        let body = move_tools::chosen_body(model, &Selection::default(), &[*body], NO_BODY)?;
        return Ok(SplitSource {
            body,
            along: Some(SplitAlong::Body(*tool)),
        });
    }
    let face = chosen.as_ref().and_then(|(_, face)| *face);
    let body = move_tools::chosen_body_beside(model, selection, tree, face, NO_BODY)?;
    Ok(SplitSource {
        body,
        along: chosen.map(|(along, _)| along),
    })
}

pub fn source_for(
    model: &Model,
    selection: &Selection,
    body: FeatureId,
) -> Result<SplitSource, SplitError> {
    let along = chosen_along(model, selection, model.document().bar_index())?;
    Ok(SplitSource {
        body,
        along: along.map(|(along, _)| along),
    })
}

pub fn create(document: &Document, source: &SplitSource) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, "Split");
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Split(Split {
            body: source.body,
            along: source
                .along
                .clone()
                .unwrap_or(SplitAlong::Plane(DEFAULT_PLANE)),
            flipped: false,
        }),
    );
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, source: &SplitSource) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), source);
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, split: Split) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Split(split),
        },
    ))
}

pub fn change(model: &Model, feature: FeatureId, split: Split) -> Result<Transaction, SplitError> {
    let document = model.document();
    let transaction = edit(document, feature, split).ok_or(SplitError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn along_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    split: &Split,
) -> Result<Transaction, SplitError> {
    let document = model.document();
    let index = document
        .feature_index(feature)
        .ok_or(SplitError::Gone { feature })?;
    let along = match chosen_along(model, selection, index)? {
        Some((along, _)) => along,
        None => {
            let earlier = document.bodies_before_in(feature, model.evaluation());
            let tools: Vec<FeatureId> = body_selection::bodies_in(selection)
                .into_iter()
                .filter(|body| *body != split.body && earlier.contains(body))
                .collect();
            match tools.as_slice() {
                [] => return Err(SplitError::NoTool),
                [tool] => SplitAlong::Body(*tool),
                [_, _, ..] => return Err(SplitError::SeveralTools),
            }
        }
    };
    if along == split.along {
        return Err(SplitError::Already);
    }
    change(
        model,
        feature,
        Split {
            along,
            ..split.clone()
        },
    )
}
