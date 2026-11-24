use std::{borrow::Cow, collections::BTreeSet};

use caditor_document::{
    Document, Draft, Edit, Evaluation, FaceAngle, FeatureId, FeatureKind, PlaneReference,
    PrincipalPlane, Transaction, default_neutral, displayed_plane, flipped_toward,
};
use caditor_kernel::{FaceId, FaceReference, Solid, Surface, tangent_faces};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    commands::{Command, Reason},
    datum_tools::{self, OneChoiceError},
    draft_panel,
    editing::{self, EditingCommand},
    field::{self, CheckError},
    last_values::{Remembered, Starts},
    model::{Action, Model, Notice},
    offset_face_tools::{self, FacePurpose, FaceToolError},
    queued::{self, Pending, Rerun},
    selection::{Pickable, Selection},
    shell_tools::FaceSource,
};

pub const TITLE: &str = "Draft";
pub const OWN_ANGLE_TITLE: &str = "Own draft angle";
pub const DEFAULT_ANGLE: f64 = 3.0;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum DraftToolError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("Select a plane or flat face made before this draft")]
    NoPlane,
    #[error("The draft already starts from the selected plane or face")]
    AlreadyPlane,
    #[error(transparent)]
    ChosenPlane(OneChoiceError),
    #[error("{}", Pending::BodyShape)]
    NoShape { body: FeatureId },
    #[error("Click a face of the body being drafted")]
    NotAFace,
    #[error(
        "Only flat faces and round faces about the pull direction can be drafted, so that face \
         cannot take an angle of its own"
    )]
    NotDraftable,
    #[error(transparent)]
    Check(#[from] CheckError),
}

impl Reason for DraftToolError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }

    fn pending(&self) -> Option<Pending> {
        matches!(self, Self::NoShape { .. }).then_some(Pending::BodyShape)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DraftSource {
    pub faces: FaceSource,
    pub neutral: Option<PlaneReference>,
}

pub fn selected_faces(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<DraftSource, FaceToolError> {
    let mut faces =
        offset_face_tools::faces_of_one_body(model, selection, tree, FacePurpose::Draft)?;
    let index = model.document().bar_index();
    let mut planes: Vec<PlaneReference> = Vec::new();

    for pickable in &faces.left_out {
        if let Some(plane) = datum_tools::plane_reference(model, *pickable, index)
            && !planes.contains(&plane)
        {
            planes.push(plane);
        }
    }

    let neutral = if let [only] = planes.as_slice() {
        let chosen = only.clone();

        faces.left_out.retain(|pickable| {
            datum_tools::plane_reference(model, *pickable, index).as_ref() != Some(&chosen)
        });
        Some(chosen)
    } else {
        None
    };
    Ok(DraftSource { faces, neutral })
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    source: &DraftSource,
    starts: &Starts,
) -> Result<(Transaction, FeatureId), FaceToolError> {
    let chosen = &source.faces;
    let shown = bodies::shown(evaluation, chosen.body).ok_or(FaceToolError::NoShape)?;

    let ids: Vec<FaceId> = chosen
        .faces
        .iter()
        .filter_map(|key| bodies::find_face(shown, *key))
        .collect();

    let faces: Vec<FaceReference> = ids
        .iter()
        .filter_map(|face| FaceReference::capture(&shown.solid, *face))
        .collect();

    if faces.len() < chosen.faces.len() {
        return Err(FaceToolError::FacesGone);
    }

    let (neutral, flipped) = match &source.neutral {
        Some(neutral) => (
            neutral.clone(),
            flipped_on_shown(evaluation, chosen.body, &faces, neutral),
        ),
        None => default_neutral(chosen.body, &shown.solid, &ids),
    };
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));

    let feature = transaction.add_feature(
        name,
        FeatureKind::Draft(Draft {
            body: chosen.body,
            faces,
            neutral,
            angle: starts.angle(Remembered::DraftAngle, DEFAULT_ANGLE),
            flipped,
            tangent: false,
            beyond: None,
            own: Vec::new(),
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: &DraftSource,
    starts: &Starts,
) -> Vec<Action> {
    match create(document, evaluation, source, starts) {
        Ok((transaction, feature)) => {
            let told = body_selection::left_out_words(&source.faces.left_out).map(|words| {
                Action::Inform(Notice::warning(format!(
                    "{TITLE} takes faces to draft and one neutral plane, so {words}."
                )))
            });
            [
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ]
            .into_iter()
            .chain(told)
            .collect()
        }
        Err(reason) => vec![queued::refused(Command::Draft, TITLE, &reason)],
    }
}

fn finish_edit(
    document: &Document,
    feature: FeatureId,
    label: String,
    draft: Draft,
) -> Transaction {
    let mut owned: Vec<(String, &caditor_expression::Expression)> =
        vec![(draft_panel::ANGLE.to_owned(), &draft.angle)];

    if let Some(beyond) = &draft.beyond {
        owned.push((draft_panel::BEYOND.to_owned(), beyond));
    }
    for (index, own) in draft.own.iter().enumerate() {
        owned.push((draft_panel::own_angle_caption(index), &own.angle));
    }

    let fields: Vec<(&str, &caditor_expression::Expression)> = owned
        .iter()
        .map(|(caption, expression)| (caption.as_str(), *expression))
        .collect();
    let transaction = Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Draft(draft.clone()),
        },
    );
    document.carrying_owners(transaction, feature, fields)
}

pub fn edit(document: &Document, feature: FeatureId, draft: Draft) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(finish_edit(
        document,
        feature,
        format!("Edit {name}"),
        draft,
    ))
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    draft: Draft,
) -> Result<Transaction, DraftToolError> {
    let document = model.document();
    let transaction = edit(document, feature, draft).ok_or(DraftToolError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn neutral_change(
    model: &Model,
    feature: FeatureId,
    draft: &Draft,
    neutral: PlaneReference,
) -> Result<Transaction, DraftToolError> {
    let evaluation = model.evaluation();
    let flipped = match (
        displayed_plane(evaluation, feature, &neutral),
        bodies::input(evaluation, feature),
    ) {
        (Some(plane), Some(input)) => {
            flipped_toward(&plane, &input.solid, &chosen(&input.solid, draft))
        }
        _ => draft.flipped,
    };
    change(
        model,
        feature,
        Draft {
            neutral,
            flipped,
            ..draft.clone()
        },
    )
}

pub fn plane_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    draft: &Draft,
) -> Result<Transaction, DraftToolError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(DraftToolError::Gone { feature })?;
    let plane = datum_tools::chosen_plane(model, selection, index)
        .map_err(DraftToolError::ChosenPlane)?
        .ok_or(DraftToolError::NoPlane)?
        .plane;

    if plane == draft.neutral {
        return Err(DraftToolError::AlreadyPlane);
    }
    neutral_change(model, feature, draft, plane)
}

fn resolved(solid: &Solid, faces: &[FaceReference]) -> Vec<FaceId> {
    faces
        .iter()
        .filter_map(|reference| reference.resolve(solid).ok())
        .collect()
}

pub fn default_neutral_on(
    evaluation: &Evaluation,
    body: FeatureId,
    faces: &[FaceReference],
) -> (PlaneReference, bool) {
    match bodies::shown(evaluation, body) {
        Some(shown) => default_neutral(body, &shown.solid, &resolved(&shown.solid, faces)),
        None => (PlaneReference::Principal(PrincipalPlane::Xy), false),
    }
}

pub fn flipped_on_shown(
    evaluation: &Evaluation,
    body: FeatureId,
    faces: &[FaceReference],
    neutral: &PlaneReference,
) -> bool {
    match (
        displayed_plane(evaluation, body, neutral),
        bodies::shown(evaluation, body),
    ) {
        (Some(plane), Some(shown)) => {
            flipped_toward(&plane, &shown.solid, &resolved(&shown.solid, faces))
        }
        _ => false,
    }
}

fn chosen(solid: &Solid, draft: &Draft) -> Vec<FaceId> {
    let picked: Vec<FaceId> = draft
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    if draft.tangent {
        tangent_faces(solid, &picked)
    } else {
        picked
    }
}

pub fn references(draft: &Draft) -> Vec<FaceReference> {
    draft
        .faces
        .iter()
        .chain(draft.own.iter().map(|own| &own.face))
        .cloned()
        .collect()
}

fn own_faces(solid: &Solid, draft: &Draft) -> Vec<FaceId> {
    draft
        .own_resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect()
}

pub fn drafted_faces(solid: &Solid, draft: &Draft) -> BTreeSet<FaceKey> {
    let drafted: BTreeSet<FaceId> = chosen(solid, draft)
        .into_iter()
        .chain(own_faces(solid, draft))
        .collect();
    bodies::face_keys(solid)
        .into_iter()
        .filter_map(|(id, key)| drafted.contains(&id).then_some(key))
        .collect()
}

fn waits_for_its_first_face(draft: &Draft, solid: &Solid) -> bool {
    let (started_on, started_flipped) = default_neutral(draft.body, solid, &[]);

    draft.faces.is_empty()
        && draft.own.is_empty()
        && draft.neutral == started_on
        && draft.flipped == started_flipped
}

fn with_faces(draft: &Draft, solid: &Solid, faces: Vec<FaceReference>) -> Draft {
    let (neutral, flipped) = if waits_for_its_first_face(draft, solid) {
        default_neutral(draft.body, solid, &resolved(solid, &faces))
    } else {
        (draft.neutral.clone(), draft.flipped)
    };
    Draft {
        faces,
        neutral,
        flipped,
        ..draft.clone()
    }
}

fn without_own_angle(draft: &mut Draft, solid: &Solid, faces: &BTreeSet<FaceId>) {
    let kept: Vec<FaceAngle> = draft
        .own
        .iter()
        .zip(draft.own_resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().iter().any(|face| faces.contains(face)))
        .map(|(own, _)| own.clone())
        .collect();

    draft.own = kept;
}

fn without_faces(draft: &mut Draft, solid: &Solid, faces: &BTreeSet<FaceId>) {
    let kept: Vec<FaceReference> = draft
        .faces
        .iter()
        .zip(draft.resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().iter().any(|face| faces.contains(face)))
        .map(|(reference, _)| reference.clone())
        .collect();

    draft.faces = kept;
}

fn is_draftable(solid: &Solid, face: FaceId) -> bool {
    solid.face(face).is_some_and(|face| {
        matches!(
            face.surface(),
            Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_)
        )
    })
}

fn own_angle_toggled(
    model: &Model,
    feature: FeatureId,
    clicked: FaceId,
) -> Result<Transaction, DraftToolError> {
    let document = model.document();
    let owner = document
        .feature(feature)
        .ok_or(DraftToolError::Gone { feature })?;
    let draft = owner.kind.draft().ok_or(DraftToolError::Gone { feature })?;
    let input = bodies::input(model.evaluation(), feature)
        .ok_or(DraftToolError::NoShape { body: draft.body })?;
    let solid = &input.solid;

    if !is_draftable(solid, clicked) {
        return Err(DraftToolError::NotDraftable);
    }

    let clicked_only = BTreeSet::from([clicked]);
    let mut changed = draft.clone();

    without_own_angle(&mut changed, solid, &clicked_only);

    let label = if changed.own.len() == draft.own.len() {
        without_faces(&mut changed, solid, &clicked_only);
        if waits_for_its_first_face(draft, solid) {
            let (neutral, flipped) = default_neutral(draft.body, solid, &[clicked]);

            changed.neutral = neutral;
            changed.flipped = flipped;
        }
        changed.own.push(FaceAngle {
            face: FaceReference::capture(solid, clicked)
                .ok_or(DraftToolError::NoShape { body: draft.body })?,
            angle: field::copied_value(document, feature, &draft.angle),
        });
        format!("Give a face of {} its own draft angle", owner.name)
    } else {
        changed.faces.push(
            FaceReference::capture(solid, clicked)
                .ok_or(DraftToolError::NoShape { body: draft.body })?,
        );
        format!("Give a face of {} the draft angle", owner.name)
    };
    field::checked(document, finish_edit(document, feature, label, changed))
}

fn clicked_face(model: &Model, feature: FeatureId, pickable: Pickable) -> Option<FaceId> {
    let draft = model.document().feature(feature)?.kind.draft()?;
    let input = bodies::input(model.evaluation(), feature)?;
    match pickable {
        Pickable::ShellFace {
            feature: owner,
            face,
        } if owner == feature => bodies::find_face(input, face),
        Pickable::Face { body, face } if body == draft.body => bodies::find_face(input, face),
        _ => None,
    }
}

pub fn own_angle_click(model: &Model, feature: FeatureId, pickable: Pickable) -> Vec<Action> {
    if bodies::input(model.evaluation(), feature).is_none() {
        let rerun = Rerun::OwnAngle { feature, pickable };

        return vec![queued::refused_rerun(
            rerun,
            OWN_ANGLE_TITLE,
            &Pending::BodyShape,
        )];
    }

    let toggled = clicked_face(model, feature, pickable)
        .ok_or(DraftToolError::NotAFace)
        .and_then(|face| own_angle_toggled(model, feature, face));
    match toggled {
        Ok(transaction) => vec![Action::Apply(transaction)],
        Err(reason) => vec![Action::Inform(Notice::warning(format!("{reason}.")))],
    }
}

pub fn own_angle_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
) -> Result<Transaction, DraftToolError> {
    let mut picked = selection
        .iter()
        .filter_map(|pickable| clicked_face(model, feature, pickable));
    match (picked.next(), picked.next()) {
        (Some(face), None) => own_angle_toggled(model, feature, face),
        _ => Err(DraftToolError::NotAFace),
    }
}

pub fn toggle_face(model: &Model, feature: FeatureId, face: FaceKey) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let draft = owner.kind.draft()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_face(input, face)?;
    let solid = &input.solid;
    let clicked_only = BTreeSet::from([clicked]);
    let mut changed = draft.clone();

    without_faces(&mut changed, solid, &clicked_only);
    without_own_angle(&mut changed, solid, &clicked_only);

    let left_out = changed.faces.len() + changed.own.len() < draft.faces.len() + draft.own.len();
    let label = if left_out {
        format!("Leave a face out of {}", owner.name)
    } else {
        let mut faces = changed.faces.clone();

        faces.push(FaceReference::capture(solid, clicked)?);
        changed = with_faces(&changed, solid, faces);
        format!("Draft a face with {}", owner.name)
    };
    Some(finish_edit(model.document(), feature, label, changed))
}

pub fn with_selected_faces(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let draft = owner.kind.draft()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;

    let mut taken: BTreeSet<FaceId> = draft
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .chain(own_faces(solid, draft))
        .collect();
    let mut faces = draft.faces.clone();

    for pickable in selection.iter() {
        let Pickable::Face { body, face } = pickable else {
            continue;
        };

        let Some(found) = (body == draft.body)
            .then(|| bodies::find_face(input, face))
            .flatten()
            .filter(|found| !taken.contains(found))
        else {
            continue;
        };

        faces.push(FaceReference::capture(solid, found)?);
        taken.insert(found);
    }
    (faces.len() > draft.faces.len()).then(|| {
        finish_edit(
            document,
            feature,
            format!("Draft the selected faces with {}", owner.name),
            with_faces(draft, solid, faces),
        )
    })
}
