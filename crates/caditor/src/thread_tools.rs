use caditor_document::{
    Bore, Document, Edit, Evaluation, FeatureId, FeatureKind, Resolution, Thread, ThreadFamily,
    ThreadHand, ThreadLength, Transaction,
};
use caditor_kernel::{FaceReference, Surface};

use crate::{
    bodies::{self, FaceKey},
    editing::EditingCommand,
    field,
    model::{Action, Model, Notice},
    selection::{Pickable, Selection},
    sketch_placement,
};

pub const TITLE: &str = "Thread";
const CHOOSE_FACE: &str = "Select the round face of a bore, shaft or boss to thread";
const ONE_FACE: &str = "Select one round face only";
const NOT_ROUND: &str = "Select a cylindrical face: a bore, a shaft or a boss";
const NO_SHAPE: &str = "The body has no shape yet; recompute the model, then try again";
const GONE: &str = "The selected face is no longer part of the model";
const SAME_FACE: &str = "The thread is already on the selected face";
const GONE_FEATURE: &str = "The feature no longer exists";
const MADE_LATER: &str =
    "The selected face is made further down the tree, after the thread; choose one made before it";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadSource {
    pub body: FeatureId,
    pub face: FaceKey,
}

pub fn selected_face(model: &Model, selection: &Selection) -> Result<ThreadSource, &'static str> {
    let mut faces = selection.iter().filter_map(|pickable| match pickable {
        Pickable::Face { body, face } => Some((body, face)),
        _ => None,
    });
    let (body, face) = faces.next().ok_or(CHOOSE_FACE)?;
    if faces.next().is_some() || selection.iter().count() > 1 {
        return Err(ONE_FACE);
    }
    let shown = bodies::shown(model.evaluation(), body).ok_or(CHOOSE_FACE)?;
    let id = bodies::find_face(shown, face).ok_or(CHOOSE_FACE)?;
    let round = shown
        .solid
        .face(id)
        .is_some_and(|face| matches!(face.surface(), Surface::Cylinder(_)));
    if !round {
        return Err(NOT_ROUND);
    }
    Ok(ThreadSource { body, face })
}

fn unique_name(document: &Document, base: &str) -> String {
    let taken = |name: &str| document.features().any(|feature| feature.name == name);
    if !taken(base) {
        return base.to_owned();
    }
    (2..=document.features().len() + 2)
        .map(|number| format!("{base} {number}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| base.to_owned())
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    source: ThreadSource,
) -> Result<(Transaction, FeatureId), &'static str> {
    let shown = bodies::shown(evaluation, source.body).ok_or(NO_SHAPE)?;
    let face = bodies::find_face(shown, source.face).ok_or(GONE)?;
    let reference = FaceReference::capture(&shown.solid, face).ok_or(GONE)?;
    let bore = Bore::of(&shown.solid, &[face]).map_err(|_| NOT_ROUND)?;
    let family = ThreadFamily::MetricCoarse;
    let size = family.nearest(bore.diameter, bore.side);
    let described = bodies::describe_face_id(document, shown, face);
    let name = unique_name(document, &format!("{TITLE} on {described}"));
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Thread(Thread {
            body: source.body,
            face: reference,
            size,
            class: family.default_class(bore.side),
            hand: ThreadHand::Right,
            length: ThreadLength::Full,
            reversed: false,
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: ThreadSource,
) -> Vec<Action> {
    match create(document, evaluation, source) {
        Ok((transaction, feature)) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::OpenSolid(feature)),
        ],
        Err(reason) => vec![Action::Inform(Notice::warning(format!(
            "{TITLE}: {reason}."
        )))],
    }
}

pub fn edit(document: &Document, feature: FeatureId, thread: Thread) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Thread(thread),
        },
    ))
}

pub fn face_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    thread: &Thread,
) -> Result<Transaction, String> {
    let source = selected_face(model, selection)?;
    let document = model.document();
    let index = document
        .feature_index(feature)
        .ok_or_else(|| GONE_FEATURE.to_owned())?;
    let shown = bodies::shown(model.evaluation(), source.body).ok_or(NO_SHAPE)?;
    let face = bodies::find_face(shown, source.face).ok_or(GONE)?;
    let reference = FaceReference::capture(&shown.solid, face).ok_or(GONE)?;
    let state = sketch_placement::body_state_before(model, source.body, index)
        .map_err(|_| MADE_LATER.to_owned())?;
    let found = reference
        .resolve(state)
        .map_err(|_| MADE_LATER.to_owned())?;
    let bore = Bore::of(state, &[found]).map_err(|_| NOT_ROUND)?;
    let family = thread.size.family();
    let size = if thread.size.fits(bore.diameter, bore.side) {
        thread.size
    } else {
        family.nearest(bore.diameter, bore.side)
    };
    let class = if family.classes(bore.side).contains(&thread.class) {
        thread.class
    } else {
        family.default_class(bore.side)
    };
    let changed = Thread {
        body: source.body,
        face: reference,
        size,
        class,
        ..thread.clone()
    };
    if changed == *thread {
        return Err(SAME_FACE.to_owned());
    }
    let transaction = edit(document, feature, changed).ok_or_else(|| GONE_FEATURE.to_owned())?;
    field::checked(document, transaction)
}

pub fn threaded_bore(evaluation: &Evaluation, feature: FeatureId, thread: &Thread) -> Option<Bore> {
    let solid = evaluation
        .body_seen_by(feature, thread.body)
        .or_else(|| evaluation.body(thread.body))?;
    match thread.resolution(solid) {
        Resolution::One(face) => Bore::of(solid, &[face]).ok(),
        Resolution::Pieces(pieces) => Bore::of(solid, &pieces).ok(),
        Resolution::Tied(_) | Resolution::Missing => None,
    }
}
