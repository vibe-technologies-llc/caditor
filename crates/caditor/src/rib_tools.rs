use caditor_document::{
    Document, Edit, Evaluation, FeatureId, FeatureKind, FeatureResult, Rib, RibDirection, RibSide,
    Transaction, is_rib_path, rib_facing,
};
use caditor_sketch::Sketch;

use crate::{
    body_selection,
    commands::Command,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model},
    queued,
    selection::{Pickable, Selection},
    units::LengthUnit,
};

pub const TITLE: &str = "Rib";
pub const DEFAULT_THICKNESS: f64 = 2.0;
pub const NO_BODY: &str = "Make a body first, then add a rib or web to it";
pub const NO_SKETCH: &str =
    "Draw the line or arc the rib runs along in a sketch, as an open chain, then choose Rib";
const SEVERAL_SKETCHES: &str =
    "Geometry of several sketches is selected; select the curves of the one the rib runs along";
const SEVERAL_BODIES: &str = "Several bodies are selected; select only the one the rib joins";
const MADE_LATER: &str = "The selected sketch is made after where the rib would go";
const NOT_OPEN: &str =
    "The curves of the selected sketch do not form open chains joined end to end for a rib";
const GONE: &str = "The feature no longer exists";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RibStart {
    pub sketch: FeatureId,
    pub body: FeatureId,
}

fn solved(evaluation: &Evaluation, sketch: FeatureId) -> Option<&Sketch> {
    evaluation
        .feature(sketch)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| &result.geometry)
}

pub fn qualifies(evaluation: &Evaluation, sketch: FeatureId) -> bool {
    solved(evaluation, sketch).is_some_and(is_rib_path)
}

fn used_after(document: &Document, sketch: FeatureId, index: usize) -> bool {
    document
        .features()
        .skip(index + 1)
        .any(|feature| feature.kind.features().contains(&sketch))
}

fn guess(model: &Model, before: usize) -> Option<FeatureId> {
    let document = model.document();
    document
        .features()
        .take(before)
        .enumerate()
        .filter(|(_, feature)| matches!(feature.kind, FeatureKind::Sketch(_)))
        .map(|(index, feature)| (index, feature.id()))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .find_map(|(index, sketch)| {
            (!used_after(document, sketch, index) && qualifies(model.evaluation(), sketch))
                .then_some(sketch)
        })
}

fn selected_sketch(
    model: &Model,
    selection: &Selection,
    rows: &[FeatureId],
    before: usize,
) -> Result<Option<FeatureId>, &'static str> {
    let document = model.document();
    let mut sketches: Vec<FeatureId> = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::SketchEntity { feature, .. } | Pickable::SketchRegion { feature, .. } => {
                Some(feature)
            }
            _ => None,
        })
        .chain(rows.iter().copied().filter(|row| {
            document
                .feature(*row)
                .is_some_and(|feature| matches!(feature.kind, FeatureKind::Sketch(_)))
        }))
        .collect();
    sketches.sort_unstable();
    sketches.dedup();
    let [sketch] = sketches.as_slice() else {
        return if sketches.is_empty() {
            Ok(None)
        } else {
            Err(SEVERAL_SKETCHES)
        };
    };
    if document
        .feature_index(*sketch)
        .is_none_or(|index| index >= before)
    {
        return Err(MADE_LATER);
    }
    if !qualifies(model.evaluation(), *sketch) {
        return Err(NOT_OPEN);
    }
    Ok(Some(*sketch))
}

fn chosen_body(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
    sketch: FeatureId,
) -> Result<FeatureId, &'static str> {
    let document = model.document();
    let standing = document.bodies_standing_in(model.evaluation());
    let picked: Vec<FeatureId> = if tree.is_empty() {
        body_selection::bodies_in(selection)
    } else {
        tree.to_vec()
    };
    match picked.as_slice() {
        [body] if standing.contains(body) => return Ok(*body),
        [_, _, ..] => return Err(SEVERAL_BODIES),
        [_] | [] => {}
    }
    let attached = document
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        .and_then(|attachment| attachment.body())
        .filter(|body| standing.contains(body));
    attached.or_else(|| standing.last().copied()).ok_or(NO_BODY)
}

pub fn source(
    model: &Model,
    selection: &Selection,
    (tree, rows): (&[FeatureId], &[FeatureId]),
) -> Result<RibStart, &'static str> {
    let before = model.document().bar_index();
    if model
        .document()
        .bodies_standing_in(model.evaluation())
        .is_empty()
    {
        return Err(NO_BODY);
    }
    let sketch = match selected_sketch(model, selection, rows, before)? {
        Some(sketch) => sketch,
        None => guess(model, before).ok_or(NO_SKETCH)?,
    };
    let body = chosen_body(model, selection, tree, sketch)?;
    Ok(RibStart { sketch, body })
}

pub fn facing_body(evaluation: &Evaluation, rib: Rib, at: Option<FeatureId>) -> Rib {
    let reversed = rib_facing(evaluation, &rib, at).unwrap_or(rib.reversed);
    Rib { reversed, ..rib }
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    start: RibStart,
    unit: LengthUnit,
) -> (Transaction, FeatureId) {
    let on_face = document
        .feature(start.sketch)
        .and_then(|feature| feature.kind.attachment())
        .and_then(|attachment| attachment.body())
        .is_some();
    let direction = if on_face {
        RibDirection::Square
    } else {
        RibDirection::Parallel
    };
    let rib = facing_body(
        evaluation,
        Rib {
            direction,
            ..Rib::new(
                start.sketch,
                start.body,
                unit.default_length(DEFAULT_THICKNESS),
            )
        },
        None,
    );
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, FeatureKind::Rib(rib));
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, start: &Result<RibStart, &'static str>) -> Vec<Action> {
    match start {
        Ok(start) => {
            let (transaction, feature) = create(
                model.document(),
                model.evaluation(),
                *start,
                model.length_unit(),
            );
            vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ]
        }
        Err(reason) => vec![queued::refused(Command::Rib, TITLE, reason)],
    }
}

pub fn edit(document: &Document, feature: FeatureId, rib: Rib) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    let transaction = Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Rib(rib),
        },
    );
    Some(document.carrying_owners(transaction, feature, []))
}

pub fn change(model: &Model, feature: FeatureId, rib: Rib) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, rib).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

pub fn with_direction(
    model: &Model,
    feature: FeatureId,
    rib: &Rib,
    direction: RibDirection,
) -> Rib {
    facing_body(
        model.evaluation(),
        Rib {
            direction,
            ..rib.clone()
        },
        Some(feature),
    )
}

pub fn with_side(rib: &Rib, side: RibSide) -> Rib {
    Rib {
        side,
        ..rib.clone()
    }
}

pub fn reversed(rib: &Rib) -> Rib {
    Rib {
        reversed: !rib.reversed,
        ..rib.clone()
    }
}
