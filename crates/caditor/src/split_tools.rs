use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, FeatureResult, PlaneReference, PrincipalPlane, Split,
    SplitAlong, Transaction, capitalized, describe_plane, is_open_chain,
};

use crate::{
    body_selection, datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model},
    move_tools,
    selection::{Pickable, Selection},
};

pub const TITLE: &str = "Split body";
const NO_BODY: &str = "Select a face or edge of the body to split";
const NO_TOOL: &str =
    "Select a plane, flat face, sketch curve or another body made before this feature";
const ALREADY: &str = "The body is already split along the selected plane, curve or body";
const SEVERAL_SKETCHES: &str =
    "Curves of several sketches are selected; select curves of only the one to split along";
const SKETCH_MADE_LATER: &str =
    "The selected sketch comes later in the tree; select one made before this feature";
const SEVERAL_TOOLS: &str = "Several other bodies are selected; select only the one to split along";
const GONE: &str = "The feature no longer exists";
const DEFAULT_PLANE: PlaneReference = PlaneReference::Principal(PrincipalPlane::Yz);

#[derive(Debug, Clone, PartialEq)]
pub struct SplitSource {
    pub body: FeatureId,
    pub along: Option<SplitAlong>,
}

pub fn describe(document: &Document, along: &SplitAlong) -> String {
    match along {
        SplitAlong::Plane(plane) => capitalized(&describe_plane(document, plane)),
        SplitAlong::Body(tool) => document
            .feature(*tool)
            .map_or_else(|| "A deleted body".to_owned(), |tool| tool.name.clone()),
        SplitAlong::Sketch(sketch) => document.feature(*sketch).map_or_else(
            || "The curve of a deleted sketch".to_owned(),
            |sketch| format!("The curve of {}", sketch.name),
        ),
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
        .bodies_before(feature)
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
) -> Result<Option<SplitAlong>, &'static str> {
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
        [_] => Err(SKETCH_MADE_LATER),
        [_, _, ..] => Err(SEVERAL_SKETCHES),
    }
}

fn chosen_along(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<Option<(SplitAlong, Option<Pickable>)>, &'static str> {
    if let Some(chosen) = datum_tools::chosen_plane(model, selection, index)? {
        return Ok(Some((SplitAlong::Plane(chosen.plane), chosen.face)));
    }
    Ok(chosen_sketch(model, selection, index)?.map(|along| (along, None)))
}

pub fn source(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<SplitSource, &'static str> {
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
) -> Result<SplitSource, &'static str> {
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

pub fn change(model: &Model, feature: FeatureId, split: Split) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, split).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

pub fn along_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    split: &Split,
) -> Result<Transaction, String> {
    let document = model.document();
    let index = document
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())?;
    let along = match chosen_along(model, selection, index)? {
        Some((along, _)) => along,
        None => {
            let earlier = document.bodies_before(feature);
            let tools: Vec<FeatureId> = body_selection::bodies_in(selection)
                .into_iter()
                .filter(|body| *body != split.body && earlier.contains(body))
                .collect();
            match tools.as_slice() {
                [] => return Err(NO_TOOL.to_owned()),
                [tool] => SplitAlong::Body(*tool),
                [_, _, ..] => return Err(SEVERAL_TOOLS.to_owned()),
            }
        }
    };
    if along == split.along {
        return Err(ALREADY.to_owned());
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
