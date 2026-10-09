use caditor_document::{
    Document, Edit, Evaluation, FaceAttachment, FeatureId, FeatureKind, FeatureState,
    PrincipalPlane, SketchAttachment, SketchFeature, Transaction, describe_plane, displayed_frame,
    face_plane,
};
use caditor_geometry::{Plane, Vector3};
use caditor_kernel::{FaceReference, Solid};
use caditor_sketch::Sketch;

use crate::{
    bodies::{self, FaceKey},
    datum_tools, editing,
    model::Model,
    scene,
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

    fn attachment(self) -> SketchAttachment {
        match self {
            Self::Plane(datum) => SketchAttachment::Datum(datum),
            Self::Frame { frame, plane } => SketchAttachment::Frame { frame, plane },
        }
    }

    fn plane(self, evaluation: &Evaluation) -> Option<Plane> {
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

pub fn is_flat(model: &Model, choice: FaceChoice) -> bool {
    bodies::shown(model.evaluation(), choice.body).is_some_and(|shown| {
        bodies::find_face(shown, choice.face)
            .is_some_and(|face| face_plane(&shown.solid, face).is_some())
    })
}

pub const NO_FACE_TO_LOOK_AT: &str = "Select one flat face of a body to look straight at it";

pub fn outward_direction(model: &Model, choice: FaceChoice) -> Option<Vector3> {
    let shown = bodies::shown(model.evaluation(), choice.body)?;
    let face = bodies::find_face(shown, choice.face)?;
    face_plane(&shown.solid, face).map(|plane| plane.normal())
}

pub fn face_to_look_at(model: &Model, selection: &Selection) -> Result<Vector3, &'static str> {
    let choice = selected_face(selection).ok_or(NO_FACE_TO_LOOK_AT)?;
    outward_direction(model, choice).ok_or(NOT_FLAT)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    NotRecomputed,
    MadeAfter,
}

impl StateError {
    pub fn face(self) -> &'static str {
        match self {
            Self::NotRecomputed => NOT_RECOMPUTED,
            Self::MadeAfter => "The selected face is made after this point in the tree",
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

const NOT_RECOMPUTED: &str =
    "The model is not recomputed up to there yet; recompute it, then try again";
const GONE: &str = "The selected face is no longer part of the model";

pub fn body_state_before(
    model: &Model,
    body: FeatureId,
    index: usize,
) -> Result<&Solid, StateError> {
    let evaluation = model.evaluation();
    for feature in model
        .document()
        .features()
        .take(index)
        .rev()
        .filter(|feature| feature.body() == Some(body))
    {
        let Some(status) = evaluation.feature(feature.id()) else {
            return Err(StateError::NotRecomputed);
        };
        match &status.state {
            FeatureState::UpToDate => {
                return status
                    .result
                    .as_deref()
                    .and_then(|result| result.solid())
                    .map(|result| &result.solid)
                    .ok_or(StateError::NotRecomputed);
            }
            FeatureState::Outdated => return Err(StateError::NotRecomputed),
            FeatureState::Failed(_) | FeatureState::Suppressed | FeatureState::RolledBack => {}
        }
    }
    Err(StateError::MadeAfter)
}

pub fn attachment_at(
    model: &Model,
    choice: FaceChoice,
    index: usize,
) -> Result<(FaceAttachment, Plane), &'static str> {
    let body = bodies::shown(model.evaluation(), choice.body).ok_or(GONE)?;
    let face = bodies::find_face(body, choice.face).ok_or(GONE)?;
    let shown = &body.solid;
    if face_plane(shown, face).is_none() {
        return Err(NOT_FLAT);
    }
    let reference = FaceReference::capture(shown, face).ok_or(GONE)?;
    let state = body_state_before(model, choice.body, index).map_err(StateError::face)?;
    let there = reference
        .resolve(state)
        .map_err(|_| StateError::MadeAfter.face())?;
    FaceAttachment::capture(choice.body, state, there)
        .ok_or("The selected face is not flat at this point in the tree")
}

pub fn new_sketch(
    model: &Model,
    choice: FaceChoice,
) -> Result<(Transaction, FeatureId), &'static str> {
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
) -> Result<Transaction, &'static str> {
    let document = model.document();
    let index = document
        .feature_index(sketch)
        .ok_or("The sketch no longer exists")?;
    let (attachment, plane) = attachment_at(model, choice, index)?;
    if document
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        .and_then(SketchAttachment::face)
        == Some(&attachment)
    {
        return Err("The sketch already lies on the selected face");
    }
    let name = feature_name(document, sketch);
    let transaction = Transaction::single(
        format!("Place {name} on a face"),
        Edit::SetSketchPlacement {
            feature: sketch,
            plane,
            attachment: Some(SketchAttachment::Face(attachment)),
        },
    );
    document
        .check(&transaction)
        .map_err(|_| "The sketch cannot lie on the selected face")?;
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

pub fn new_sketch_on_datum(
    model: &Model,
    datum: DatumTarget,
) -> Result<(Transaction, FeatureId), String> {
    let document = model.document();
    let plane = datum
        .plane(model.evaluation())
        .ok_or("The selected plane has no position yet")?;
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
    document
        .check(&transaction)
        .map_err(|error| error.to_string())?;
    Ok((transaction, feature))
}

pub fn place_on_datum(
    model: &Model,
    sketch: FeatureId,
    datum: DatumTarget,
) -> Result<Transaction, &'static str> {
    let document = model.document();
    let attachment = datum.attachment();
    if document
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        == Some(&attachment)
    {
        return Err("The sketch already lies on the selected plane");
    }
    let plane = datum
        .plane(model.evaluation())
        .ok_or("The selected plane has no position yet")?;
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
        .map_err(|_| "The selected plane comes after this sketch in the tree")?;
    Ok(transaction)
}

const NOTHING_TO_PLACE_ON: &str = "Select a datum plane or a flat face to place the sketch on";

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
) -> Result<Option<PlacementTarget>, &'static str> {
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
        [_, _, ..] => Err(SEVERAL_TO_SKETCH_ON),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchTarget {
    Principal(PrincipalPlane),
    Datum(DatumTarget),
    Face(FaceChoice),
    Choose,
}

pub fn sketch_target(model: &Model, selection: &Selection) -> Result<SketchTarget, &'static str> {
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
        [_, _, ..] => Err(SEVERAL_TO_SKETCH_ON),
    }
}

pub fn place_on_selection(
    model: &Model,
    selection: &Selection,
    sketch: FeatureId,
) -> Result<Transaction, &'static str> {
    match placement_target(model.document(), selection)?.ok_or(NOTHING_TO_PLACE_ON)? {
        PlacementTarget::Plane(datum) => place_on_datum(model, sketch, datum),
        PlacementTarget::Face(face) => place(model, sketch, face),
    }
}

fn feature_name(document: &Document, feature: FeatureId) -> String {
    document
        .feature(feature)
        .map_or_else(|| "the sketch".to_owned(), |feature| feature.name.clone())
}
