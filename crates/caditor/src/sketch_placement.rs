use caditor_document::{
    Document, Edit, EditError, Evaluation, FaceAttachment, Feature, FeatureId, FeatureKind,
    FeatureState, PrincipalPlane, SketchAttachment, SketchFeature, Transaction, body_part,
    describe_plane, displayed_frame, face_plane,
};
use caditor_geometry::{Plane, Vector3};
use caditor_kernel::{FaceReference, Solid};
use caditor_sketch::Sketch;

use crate::{
    bodies::{self, FaceKey},
    datum_tools, editing,
    model::Model,
    queued, scene,
    selection::{Pickable, Selection},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceChoice {
    pub body: FeatureId,
    pub face: FaceKey,
}

impl FaceChoice {
    pub fn of(pickable: Pickable) -> Option<Self> {
        match pickable {
            Pickable::Face { body, face } => Some(Self { body, face }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatumTarget {
    Plane(FeatureId),
    Frame {
        frame: FeatureId,
        plane: PrincipalPlane,
    },
}

impl DatumTarget {
    pub fn of(document: &Document, pickable: Pickable) -> Option<Self> {
        match pickable {
            Pickable::Datum(feature) if datum_tools::is_plane(document, feature) => {
                Some(Self::Plane(feature))
            }
            Pickable::FramePlane { feature, plane } => Some(Self::Frame {
                frame: feature,
                plane,
            }),
            _ => None,
        }
    }

    pub fn attachment(self) -> SketchAttachment {
        match self {
            Self::Plane(datum) => SketchAttachment::Datum(datum),
            Self::Frame { frame, plane } => SketchAttachment::Frame { frame, plane },
        }
    }

    pub fn plane(self, evaluation: &Evaluation) -> Option<Plane> {
        match self {
            Self::Plane(datum) => datum_tools::result(evaluation, datum)?.plane(),
            Self::Frame { frame, plane } => plane.in_frame(&displayed_frame(evaluation, frame)?),
        }
    }

    pub fn name(self, document: &Document) -> String {
        match self {
            Self::Plane(datum) => feature_name(document, datum),
            Self::Frame { frame, plane } => describe_plane(
                document,
                &caditor_document::PlaneReference::Frame { frame, plane },
            ),
        }
    }
}

pub fn selected_face(selection: &Selection) -> Option<FaceChoice> {
    let mut faces = selection.iter().filter_map(FaceChoice::of);
    let face = faces.next()?;
    faces.next().is_none().then_some(face)
}

pub const NOT_FLAT: &str = "The selected face is curved; sketches lie on planes and flat faces";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttachmentError {
    #[error("{}", GONE)]
    Gone,
    #[error("{}", NOT_FLAT)]
    NotFlat,
    #[error("{}", NOT_RECOMPUTED)]
    NotRecomputed,
    #[error("{}", FACE_MADE_AFTER)]
    MadeAfter,
    #[error("{}", NOT_FLAT_HERE)]
    NotFlatHere,
}

impl AttachmentError {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Gone => GONE,
            Self::NotFlat => NOT_FLAT,
            Self::NotRecomputed => NOT_RECOMPUTED,
            Self::MadeAfter => FACE_MADE_AFTER,
            Self::NotFlatHere => NOT_FLAT_HERE,
        }
    }
}

pub fn is_flat(model: &Model, choice: FaceChoice) -> bool {
    bodies::shown(model.evaluation(), choice.body).is_some_and(|shown| {
        bodies::find_face(shown, choice.face)
            .is_some_and(|face| face_plane(&shown.solid, face).is_some())
    })
}

pub fn outward_direction(model: &Model, choice: FaceChoice) -> Option<Vector3> {
    let shown = bodies::shown(model.evaluation(), choice.body)?;
    let face = bodies::find_face(shown, choice.face)?;
    face_plane(&shown.solid, face).map(|plane| plane.normal())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    NotRecomputed,
    MadeAfter,
}

impl StateError {
    pub fn face(self) -> AttachmentError {
        match self {
            Self::NotRecomputed => AttachmentError::NotRecomputed,
            Self::MadeAfter => AttachmentError::MadeAfter,
        }
    }

    pub fn edge(self) -> &'static str {
        match self {
            Self::NotRecomputed => NOT_RECOMPUTED,
            Self::MadeAfter => "The selected edge is made after this point in the tree",
        }
    }

    pub fn corner(self) -> &'static str {
        match self {
            Self::NotRecomputed => NOT_RECOMPUTED,
            Self::MadeAfter => "The selected corner is made after this point in the tree",
        }
    }
}

const NOT_RECOMPUTED: &str = queued::MODEL_UP_TO_THERE;
const GONE: &str = "The selected face is no longer part of the model";
const FACE_MADE_AFTER: &str = "The selected face is made after this point in the tree";
const NOT_FLAT_HERE: &str = "The selected face is not flat at this point in the tree";

pub fn body_state_before(
    model: &Model,
    body: FeatureId,
    index: usize,
) -> Result<&Solid, StateError> {
    body_state_in(model.document(), model.evaluation(), body, index)
}

fn shapes_body(feature: &Feature, body: FeatureId) -> bool {
    feature.bodies().contains(&body) || (body.is_piece() && feature.id() == body.maker())
}

pub fn body_state_in<'a>(
    document: &Document,
    evaluation: &'a Evaluation,
    body: FeatureId,
    index: usize,
) -> Result<&'a Solid, StateError> {
    for feature in document
        .features()
        .take(index)
        .rev()
        .filter(|feature| shapes_body(feature, body))
    {
        let Some(status) = evaluation.feature(feature.id()) else {
            return Err(StateError::NotRecomputed);
        };
        match &status.state {
            FeatureState::UpToDate => {
                return status
                    .result
                    .as_ref()
                    .and_then(|result| body_part(result, body))
                    .and_then(|part| part.solid())
                    .map(|part| &part.solid)
                    .ok_or(StateError::NotRecomputed);
            }
            FeatureState::Outdated => return Err(StateError::NotRecomputed),
            FeatureState::Failed(_)
            | FeatureState::Waiting(_)
            | FeatureState::Suppressed
            | FeatureState::RolledBack => {}
        }
    }
    Err(StateError::MadeAfter)
}

pub fn attachment_at(
    model: &Model,
    choice: FaceChoice,
    index: usize,
) -> Result<(FaceAttachment, Plane), AttachmentError> {
    attachment_in(model.document(), model.evaluation(), choice, index)
}

pub fn attachment_in(
    document: &Document,
    evaluation: &Evaluation,
    choice: FaceChoice,
    index: usize,
) -> Result<(FaceAttachment, Plane), AttachmentError> {
    let body = bodies::shown(evaluation, choice.body).ok_or(AttachmentError::Gone)?;
    let face = bodies::find_face(body, choice.face).ok_or(AttachmentError::Gone)?;
    let shown = &body.solid;
    if face_plane(shown, face).is_none() {
        return Err(AttachmentError::NotFlat);
    }
    let reference = FaceReference::capture(shown, face).ok_or(AttachmentError::Gone)?;
    let state =
        body_state_in(document, evaluation, choice.body, index).map_err(StateError::face)?;
    let there = reference
        .resolve(state)
        .map_err(|_| AttachmentError::MadeAfter)?;
    FaceAttachment::capture(choice.body, state, there).ok_or(AttachmentError::NotFlatHere)
}

pub fn surface_at(
    model: &Model,
    choice: FaceChoice,
    index: usize,
) -> Result<FaceAttachment, AttachmentError> {
    let body = bodies::shown(model.evaluation(), choice.body).ok_or(AttachmentError::Gone)?;
    let face = bodies::find_face(body, choice.face).ok_or(AttachmentError::Gone)?;
    let reference = FaceReference::capture(&body.solid, face).ok_or(AttachmentError::Gone)?;
    let state = body_state_before(model, choice.body, index).map_err(StateError::face)?;
    let there = reference
        .resolve(state)
        .map_err(|_| AttachmentError::MadeAfter)?;
    let face = FaceReference::capture(state, there).ok_or(AttachmentError::Gone)?;
    Ok(FaceAttachment {
        body: choice.body,
        face,
    })
}

pub fn new_sketch(
    model: &Model,
    choice: FaceChoice,
) -> Result<(Transaction, FeatureId), AttachmentError> {
    let document = model.document();
    let (attachment, plane) = attachment_at(model, choice, document.bar_index())?;
    let name = editing::next_sketch_name(document);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Sketch(SketchFeature::on_face(Sketch::new(plane), attachment)),
    );
    Ok((transaction.finish(), feature))
}

pub fn place(
    model: &Model,
    sketch: FeatureId,
    choice: FaceChoice,
) -> Result<Transaction, PlaceError> {
    let document = model.document();
    let index = document
        .feature_index(sketch)
        .ok_or(PlaceError::SketchGone)?;
    let (attachment, plane) = attachment_at(model, choice, index)?;
    if document
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        .and_then(SketchAttachment::face)
        == Some(&attachment)
    {
        return Err(PlaceError::AlreadyOnFace);
    }
    let name = feature_name(document, sketch);
    let transaction = Transaction::single(
        format!("Place {name} on a face"),
        Edit::SetSketchPlacement {
            feature: sketch,
            plane,
            attachment: Some(SketchAttachment::Face(Box::new(attachment))),
        },
    );
    document
        .check(&transaction)
        .map_err(|_| PlaceError::CannotLieOnFace)?;
    Ok(transaction)
}

pub fn detach(model: &Model, sketch: FeatureId) -> Option<Transaction> {
    let document = model.document();
    let what = match document.feature(sketch)?.kind.attachment()? {
        SketchAttachment::Face(_) => "face",
        SketchAttachment::Datum(_) | SketchAttachment::Frame { .. } => "plane",
    };
    let plane = scene::sketch_plane(document, model.evaluation(), sketch)?;
    let name = feature_name(document, sketch);
    Some(Transaction::single(
        format!("Detach {name} from its {what}"),
        Edit::SetSketchPlacement {
            feature: sketch,
            plane,
            attachment: None,
        },
    ))
}

pub fn describe(document: &Document, attachment: &SketchAttachment) -> String {
    match attachment {
        SketchAttachment::Face(face) => bodies::describe_origin(document, face.face.origin()),
        SketchAttachment::Datum(datum) => feature_name(document, *datum),
        SketchAttachment::Frame { frame, plane } => DatumTarget::Frame {
            frame: *frame,
            plane: *plane,
        }
        .name(document),
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum NewSketchError {
    #[error("The selected plane has no position yet")]
    PlaneUnplaced { datum: DatumTarget },
    #[error(transparent)]
    Rejected(#[from] EditError),
}

pub fn new_sketch_on_datum(
    model: &Model,
    datum: DatumTarget,
) -> Result<(Transaction, FeatureId), NewSketchError> {
    let document = model.document();
    let plane = datum
        .plane(model.evaluation())
        .ok_or(NewSketchError::PlaneUnplaced { datum })?;
    let name = editing::next_sketch_name(document);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Sketch(SketchFeature {
            attachment: Some(datum.attachment()),
            ..SketchFeature::from(Sketch::new(plane))
        }),
    );
    let transaction = transaction.finish();
    document.check(&transaction)?;
    Ok((transaction, feature))
}

pub fn place_on_datum(
    model: &Model,
    sketch: FeatureId,
    datum: DatumTarget,
) -> Result<Transaction, PlaceError> {
    let document = model.document();
    let attachment = datum.attachment();
    if document
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        == Some(&attachment)
    {
        return Err(PlaceError::AlreadyOnPlane);
    }
    let plane = datum
        .plane(model.evaluation())
        .ok_or(PlaceError::PlaneUnplaced)?;
    let name = feature_name(document, sketch);
    let transaction = Transaction::single(
        format!("Place {name} on {}", datum.name(document)),
        Edit::SetSketchPlacement {
            feature: sketch,
            plane,
            attachment: Some(attachment),
        },
    );
    document
        .check(&transaction)
        .map_err(|_| PlaceError::PlaneAfterSketch)?;
    Ok(transaction)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlaceError {
    #[error("The sketch no longer exists")]
    SketchGone,
    #[error("The sketch already lies on the selected face")]
    AlreadyOnFace,
    #[error("The sketch already lies on the selected plane")]
    AlreadyOnPlane,
    #[error("The sketch cannot lie on the selected face")]
    CannotLieOnFace,
    #[error("The selected plane has no position yet")]
    PlaneUnplaced,
    #[error("The selected plane comes after this sketch in the tree")]
    PlaneAfterSketch,
    #[error("Select a datum plane or a flat face to place the sketch on")]
    NothingToPlaceOn,
    #[error(transparent)]
    Target(#[from] TargetError),
    #[error(transparent)]
    Attachment(#[from] AttachmentError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    #[error("{}", SEVERAL_TO_SKETCH_ON)]
    Several,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementTarget {
    Plane(DatumTarget),
    Face(FaceChoice),
}

pub const SEVERAL_TO_SKETCH_ON: &str =
    "Several planes or faces are selected; select only the one to sketch on";

pub fn placement_target(
    document: &Document,
    selection: &Selection,
) -> Result<Option<PlacementTarget>, TargetError> {
    let targets: Vec<PlacementTarget> = selection
        .iter()
        .filter_map(|pickable| match DatumTarget::of(document, pickable) {
            Some(datum) => Some(PlacementTarget::Plane(datum)),
            None => FaceChoice::of(pickable).map(PlacementTarget::Face),
        })
        .collect();
    match targets.as_slice() {
        [] => Ok(None),
        [target] => Ok(Some(*target)),
        [_, _, ..] => Err(TargetError::Several),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchTarget {
    Principal(PrincipalPlane),
    Datum(DatumTarget),
    Face(FaceChoice),
    Choose,
}

pub fn sketch_target(model: &Model, selection: &Selection) -> Result<SketchTarget, TargetError> {
    let document = model.document();
    let targets: Vec<SketchTarget> = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Plane(plane) => Some(SketchTarget::Principal(plane)),
            pickable => match DatumTarget::of(document, pickable) {
                Some(datum) => Some(SketchTarget::Datum(datum)),
                None => FaceChoice::of(pickable).map(SketchTarget::Face),
            },
        })
        .collect();
    match targets.as_slice() {
        [] => Ok(SketchTarget::Choose),
        [SketchTarget::Face(face)] if !is_flat(model, *face) => Ok(SketchTarget::Choose),
        [target] => Ok(*target),
        [_, _, ..] => Err(TargetError::Several),
    }
}

pub fn place_on_selection(
    model: &Model,
    selection: &Selection,
    sketch: FeatureId,
) -> Result<Transaction, PlaceError> {
    match placement_target(model.document(), selection)?.ok_or(PlaceError::NothingToPlaceOn)? {
        PlacementTarget::Plane(datum) => place_on_datum(model, sketch, datum),
        PlacementTarget::Face(face) => place(model, sketch, face),
    }
}

fn feature_name(document: &Document, feature: FeatureId) -> String {
    document
        .feature(feature)
        .map_or_else(|| "the sketch".to_owned(), |feature| feature.name.clone())
}
