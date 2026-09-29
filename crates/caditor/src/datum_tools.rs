use caditor_document::{
    AxisReference, Datum, DatumAxis, DatumPlane, DatumResult, Document, Edit, Evaluation,
    FeatureId, FeatureKind, FeatureResult, PlaneReference, PlaneRotation, PrincipalPlane,
    Transaction,
};
use caditor_kernel::{EdgeReference, FaceReference};

use crate::{
    bodies,
    editing::{self, EditingCommand},
    model::{Action, Model, Notice},
    selection::{Pickable, Selection},
    sketch_placement::{self, FaceChoice},
    solid_tools,
};

pub const DEFAULT_OFFSET: f64 = 10.0;
pub const DEFAULT_ANGLE: f64 = 45.0;

pub fn result(evaluation: &Evaluation, feature: FeatureId) -> Option<DatumResult> {
    evaluation
        .feature(feature)?
        .result
        .as_deref()
        .and_then(FeatureResult::datum)
        .copied()
}

pub fn is_plane(document: &Document, feature: FeatureId) -> bool {
    is_datum(document, feature, true)
}

fn is_datum(document: &Document, feature: FeatureId, plane: bool) -> bool {
    document
        .feature(feature)
        .and_then(|feature| feature.kind.datum())
        .is_some_and(|datum| datum.is_plane() == plane)
}

fn comes_before(document: &Document, feature: FeatureId, index: usize) -> bool {
    document
        .feature_index(feature)
        .is_some_and(|position| position < index)
}

pub fn plane_reference(model: &Model, pickable: Pickable, index: usize) -> Option<PlaneReference> {
    let document = model.document();
    match pickable {
        Pickable::Plane(plane) => Some(PlaneReference::Principal(plane)),
        Pickable::Datum(feature)
            if is_datum(document, feature, true) && comes_before(document, feature, index) =>
        {
            Some(PlaneReference::Datum(feature))
        }
        pickable => {
            let choice = FaceChoice::of(pickable)?;
            let (attachment, _) = sketch_placement::attachment_at(model, choice, index).ok()?;
            Some(PlaneReference::Face(attachment))
        }
    }
}

pub fn axis_reference(model: &Model, pickable: Pickable, index: usize) -> Option<AxisReference> {
    let document = model.document();
    let evaluation = model.evaluation();
    match pickable {
        Pickable::Axis(axis) => Some(AxisReference::Principal(axis.principal())),
        Pickable::Datum(feature)
            if is_datum(document, feature, false) && comes_before(document, feature, index) =>
        {
            Some(AxisReference::Datum(feature))
        }
        Pickable::Edge { body, edge } => {
            let shown = bodies::shown(evaluation, body)?;
            let reference = EdgeReference::capture(&shown.solid, bodies::find_edge(shown, edge)?)?;
            let state = sketch_placement::body_state_before(model, body, index).ok()?;
            AxisReference::capture_edge(body, state, reference.resolve(state).ok()?)
        }
        Pickable::Face { body, face } => {
            let shown = bodies::shown(evaluation, body)?;
            let reference = FaceReference::capture(&shown.solid, bodies::find_face(shown, face)?)?;
            let state = sketch_placement::body_state_before(model, body, index).ok()?;
            AxisReference::capture_face(body, state, reference.resolve(state).ok()?)
        }
        _ => None,
    }
}

struct Chosen {
    planes: Vec<PlaneReference>,
    axes: Vec<AxisReference>,
    unusable: Option<&'static str>,
}

fn chosen(model: &Model, selection: &Selection, index: usize) -> Chosen {
    let mut planes = Vec::new();
    let mut axes = Vec::new();
    let mut unusable = None;
    for pickable in selection.iter() {
        if let Some(plane) = plane_reference(model, pickable, index) {
            planes.push(plane);
        } else if let Some(axis) = axis_reference(model, pickable, index) {
            axes.push(axis);
        } else if let Some(reason) = why_unusable(model, pickable, index) {
            unusable.get_or_insert(reason);
        }
    }
    Chosen {
        planes,
        axes,
        unusable,
    }
}

fn why_unusable(model: &Model, pickable: Pickable, index: usize) -> Option<&'static str> {
    let state = |body| sketch_placement::body_state_before(model, body, index).err();
    match pickable {
        Pickable::Face { body, face } => Some(
            match sketch_placement::attachment_at(model, FaceChoice { body, face }, index) {
                Err(sketch_placement::NOT_FLAT) => {
                    "The selected face is neither flat nor round, so it gives no plane or axis"
                }
                Err(reason) => reason,
                Ok(_) => return None,
            },
        ),
        Pickable::Edge { body, .. } => Some(match state(body) {
            Some(error) => error.edge(),
            None => "The selected edge is not straight, so it gives no axis",
        }),
        Pickable::Datum(_) => Some("The selected plane or axis comes after this point in the tree"),
        _ => None,
    }
}

pub fn plane_from_selection(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<DatumPlane, &'static str> {
    let Chosen {
        planes,
        axes,
        unusable,
    } = chosen(model, selection, index);
    if let Some(reason) = unusable {
        return Err(reason);
    }
    if planes.len() > 1 {
        return Err("Select only one plane or flat face to start from");
    }
    if axes.len() > 1 {
        return Err("Select only one axis, straight edge or round face to turn about");
    }
    let base = planes
        .into_iter()
        .next()
        .unwrap_or(PlaneReference::Principal(PrincipalPlane::Xy));
    let rotation = axes.into_iter().next().map(|axis| PlaneRotation {
        axis,
        angle: solid_tools::degrees(DEFAULT_ANGLE),
    });
    let offset = if rotation.is_some() {
        0.0
    } else {
        DEFAULT_OFFSET
    };
    Ok(DatumPlane {
        base,
        rotation,
        offset: model.length_unit().default_length(offset),
    })
}

pub fn axis_from_selection(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<DatumAxis, &'static str> {
    let Chosen {
        planes,
        axes,
        unusable,
    } = chosen(model, selection, index);
    if let Some(reason) = unusable {
        return Err(reason);
    }
    let mut planes = planes.into_iter();
    let mut axes = axes.into_iter();
    match (
        planes.next(),
        planes.next(),
        planes.next(),
        axes.next(),
        axes.next(),
    ) {
        (None, None, None, Some(axis), None) => Ok(DatumAxis::Along(axis)),
        (Some(first), Some(second), None, None, None) => Ok(DatumAxis::Intersection(first, second)),
        _ => Err(
            "Select one axis, straight edge or round face, or two planes or flat faces that cross",
        ),
    }
}

pub fn create(document: &Document, datum: Datum) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, datum.title());
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, FeatureKind::Datum(datum));
    (transaction.finish(), feature)
}

pub fn create_actions(document: &Document, datum: Datum) -> Vec<Action> {
    let title = datum.title();
    let (transaction, feature) = create(document, datum);
    if let Err(error) = document.check(&transaction) {
        return vec![Action::Inform(Notice::info(format!("{title}: {error}.")))];
    }
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, datum: Datum) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Datum(datum),
        },
    ))
}

pub fn selected_datum_plane(document: &Document, selection: &Selection) -> Option<FeatureId> {
    let mut planes = selection.iter().filter_map(|pickable| match pickable {
        Pickable::Datum(feature) if is_plane(document, feature) => Some(feature),
        _ => None,
    });
    let plane = planes.next()?;
    planes.next().is_none().then_some(plane)
}
