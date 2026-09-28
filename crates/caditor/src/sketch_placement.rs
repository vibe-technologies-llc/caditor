use caditor_document::{
    Document, Edit, FaceAttachment, FeatureId, FeatureKind, FeatureState, SketchFeature,
    Transaction, face_plane,
};
use caditor_geometry::Plane;
use caditor_kernel::{FaceReference, Solid};
use caditor_sketch::Sketch;

use crate::{
    bodies::{self, FaceKey},
    editing,
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

pub fn selected_face(selection: &Selection) -> Option<FaceChoice> {
    let mut faces = selection.iter().filter_map(FaceChoice::of);
    let face = faces.next()?;
    faces.next().is_none().then_some(face)
}

pub fn is_flat(model: &Model, choice: FaceChoice) -> bool {
    model.evaluation().body(choice.body).is_some_and(|solid| {
        bodies::find_face(solid, choice.face).is_some_and(|face| face_plane(solid, face).is_some())
    })
}

fn body_state_before(model: &Model, body: FeatureId, index: usize) -> Option<&Solid> {
    let evaluation = model.evaluation();
    model
        .document()
        .features()
        .take(index)
        .rev()
        .filter(|feature| feature.body() == Some(body))
        .find_map(|feature| {
            let status = evaluation.feature(feature.id())?;
            if status.state != FeatureState::UpToDate {
                return None;
            }
            Some(&status.result.as_deref()?.solid()?.solid)
        })
}

fn attachment_at(
    model: &Model,
    choice: FaceChoice,
    index: usize,
) -> Result<(FaceAttachment, Plane), &'static str> {
    let shown = model
        .evaluation()
        .body(choice.body)
        .ok_or("The selected face is no longer part of the model")?;
    let face = bodies::find_face(shown, choice.face)
        .ok_or("The selected face is no longer part of the model")?;
    if face_plane(shown, face).is_none() {
        return Err("The selected face is not flat");
    }
    let reference = FaceReference::capture(shown, face)
        .ok_or("The selected face is no longer part of the model")?;
    let state = body_state_before(model, choice.body, index)
        .ok_or("The selected face is made after this sketch in the tree")?;
    let there = reference
        .resolve(state)
        .map_err(|_| "The selected face is made after this sketch in the tree")?;
    FaceAttachment::capture(choice.body, state, there)
        .ok_or("The selected face is not flat where this sketch is in the tree")
}

pub fn new_sketch(model: &Model, choice: FaceChoice) -> Option<(Transaction, FeatureId)> {
    let document = model.document();
    let (attachment, plane) = attachment_at(model, choice, document.features().len()).ok()?;
    let name = editing::next_sketch_name(document);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Sketch(SketchFeature::on_face(Sketch::new(plane), attachment)),
    );
    Some((transaction.finish(), feature))
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
            attachment: Some(attachment),
        },
    );
    document
        .check(&transaction)
        .map_err(|_| "The sketch cannot lie on the selected face")?;
    Ok(transaction)
}

pub fn detach(model: &Model, sketch: FeatureId) -> Option<Transaction> {
    let document = model.document();
    document.feature(sketch)?.kind.attachment()?;
    let plane = scene::sketch_plane(document, model.evaluation(), sketch)?;
    let name = feature_name(document, sketch);
    Some(Transaction::single(
        format!("Detach {name} from its face"),
        Edit::SetSketchPlacement {
            feature: sketch,
            plane,
            attachment: None,
        },
    ))
}

pub fn describe(document: &Document, attachment: &FaceAttachment) -> String {
    bodies::describe_origin(document, attachment.face.origin())
}

fn feature_name(document: &Document, feature: FeatureId) -> String {
    document
        .feature(feature)
        .map_or_else(|| "the sketch".to_owned(), |feature| feature.name.clone())
}
