use caditor_document::{
    AxisReference, CurveStation, Datum, DatumAxis, DatumKind, DatumPlane, DatumPoint, DatumResult,
    Document, Edit, Evaluation, FeatureId, FeatureKind, FeatureResult, PlaneReference,
    PlaneRotation, PlaneThrough, PointBy, PointReference, PrincipalPlane, Transaction,
};
use caditor_kernel::{EdgeReference, FaceReference, vertex_names};

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
pub const SEVERAL_PLANES: &str =
    "Several planes or flat faces are selected; select only the one to use";
pub const DATUM_NOT_A_PLANE: &str =
    "The selected datum is an axis or a point; select a plane or flat face";
pub const DATUM_MADE_LATER: &str =
    "The selected datum plane comes later in the tree; select one made before this feature";

pub const SEVERAL_AXES: &str =
    "Several axes, straight edges or round faces are selected; select only the one to use";

fn only<T: PartialEq>(
    candidates: impl Iterator<Item = T>,
    several: &'static str,
) -> Result<Option<T>, &'static str> {
    let mut found: Vec<T> = Vec::new();
    for candidate in candidates {
        if !found.contains(&candidate) {
            found.push(candidate);
        }
    }
    match found.len() {
        0 | 1 => Ok(found.pop()),
        _ => Err(several),
    }
}

pub fn only_plane(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<Option<PlaneReference>, &'static str> {
    only(
        selection
            .iter()
            .filter_map(|pickable| plane_reference(model, pickable, index)),
        SEVERAL_PLANES,
    )
}

pub fn only_axis(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<Option<AxisReference>, &'static str> {
    only(
        selection
            .iter()
            .filter_map(|pickable| axis_reference(model, pickable, index)),
        SEVERAL_AXES,
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChosenPlane {
    pub plane: PlaneReference,
    pub face: Option<Pickable>,
}

pub fn chosen_plane(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<Option<ChosenPlane>, &'static str> {
    let document = model.document();
    let mut planes = Vec::new();
    for pickable in selection.iter() {
        match pickable {
            Pickable::Plane(plane) => planes.push(PlaneReference::Principal(plane)),
            Pickable::Datum(feature) if !is_datum(document, feature, DatumKind::Plane) => {
                return Err(DATUM_NOT_A_PLANE);
            }
            Pickable::Datum(feature) if !comes_before(document, feature, index) => {
                return Err(DATUM_MADE_LATER);
            }
            Pickable::Datum(feature) => planes.push(PlaneReference::Datum(feature)),
            _ => {}
        }
    }
    match planes.as_slice() {
        [plane] => {
            return Ok(Some(ChosenPlane {
                plane: plane.clone(),
                face: None,
            }));
        }
        [_, _, ..] => return Err(SEVERAL_PLANES),
        [] => {}
    }
    let faces: Vec<ChosenPlane> = selection
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::Face { .. }))
        .filter_map(|face| {
            Some(ChosenPlane {
                plane: plane_reference(model, face, index)?,
                face: Some(face),
            })
        })
        .collect();
    match faces.as_slice() {
        [] => Ok(None),
        [face] => Ok(Some(face.clone())),
        [_, _, ..] => Err(SEVERAL_PLANES),
    }
}

pub fn result(evaluation: &Evaluation, feature: FeatureId) -> Option<DatumResult> {
    evaluation
        .feature(feature)?
        .result
        .as_deref()
        .and_then(FeatureResult::datum)
        .copied()
}

pub fn is_plane(document: &Document, feature: FeatureId) -> bool {
    is_datum(document, feature, DatumKind::Plane)
}

fn is_datum(document: &Document, feature: FeatureId, kind: DatumKind) -> bool {
    document
        .feature(feature)
        .and_then(|feature| feature.kind.datum())
        .is_some_and(|datum| datum.kind() == kind)
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
            if is_datum(document, feature, DatumKind::Plane)
                && comes_before(document, feature, index) =>
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
            if is_datum(document, feature, DatumKind::Axis)
                && comes_before(document, feature, index) =>
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
        Pickable::SketchEntity { feature, entity } if comes_before(document, feature, index) => {
            let sketch = document.feature(feature)?.kind.sketch()?;
            sketch.line_endpoints(entity)?;
            Some(AxisReference::Sketch {
                sketch: feature,
                entity,
            })
        }
        _ => None,
    }
}

pub fn point_reference(model: &Model, pickable: Pickable, index: usize) -> Option<PointReference> {
    let document = model.document();
    match pickable {
        Pickable::Origin => Some(PointReference::Origin),
        Pickable::Datum(feature)
            if is_datum(document, feature, DatumKind::Point)
                && comes_before(document, feature, index) =>
        {
            Some(PointReference::Datum(feature))
        }
        Pickable::Vertex { body, vertex } => {
            let state = sketch_placement::body_state_before(model, body, index).ok()?;
            let named = vertex_names(state)
                .iter()
                .filter(|(_, name)| **name == vertex.name)
                .count();
            (named == 1).then_some(PointReference::Vertex {
                body,
                vertex: vertex.name,
            })
        }
        Pickable::Edge { body, edge } => {
            let shown = bodies::shown(model.evaluation(), body)?;
            let reference = EdgeReference::capture(&shown.solid, bodies::find_edge(shown, edge)?)?;
            let state = sketch_placement::body_state_before(model, body, index).ok()?;
            PointReference::capture_centre(body, state, reference.resolve(state).ok()?)
        }
        Pickable::Face { body, face } => {
            let shown = bodies::shown(model.evaluation(), body)?;
            let reference = FaceReference::capture(&shown.solid, bodies::find_face(shown, face)?)?;
            let state = sketch_placement::body_state_before(model, body, index).ok()?;
            PointReference::capture_surface_centre(body, state, reference.resolve(state).ok()?)
        }
        Pickable::SketchEntity { feature, entity } if comes_before(document, feature, index) => {
            let sketch = document.feature(feature)?.kind.sketch()?;
            sketch.point(entity)?;
            Some(PointReference::Sketch {
                sketch: feature,
                entity,
            })
        }
        _ => None,
    }
}

fn curve_station(model: &Model, pickable: Pickable, index: usize) -> Option<CurveStation> {
    let Pickable::Edge { body, edge } = pickable else {
        return None;
    };
    let shown = bodies::shown(model.evaluation(), body)?;
    let reference = EdgeReference::capture(&shown.solid, bodies::find_edge(shown, edge)?)?;
    let state = sketch_placement::body_state_before(model, body, index).ok()?;
    CurveStation::capture(
        body,
        state,
        reference.resolve(state).ok()?,
        model.length_unit().default_length(0.0),
    )
}

fn only_pickable(selection: &Selection) -> Option<Pickable> {
    let mut picked = selection.iter();
    let first = picked.next()?;
    picked.next().is_none().then_some(first)
}

struct Chosen {
    planes: Vec<PlaneReference>,
    axes: Vec<AxisReference>,
    points: Vec<PointReference>,
    unusable: Option<&'static str>,
}

fn chosen(model: &Model, selection: &Selection, index: usize) -> Chosen {
    let mut planes = Vec::new();
    let mut axes = Vec::new();
    let mut points = Vec::new();
    let mut unusable = None;
    for pickable in selection.iter() {
        if let Some(plane) = plane_reference(model, pickable, index) {
            planes.push(plane);
        } else if let Some(axis) = axis_reference(model, pickable, index) {
            axes.push(axis);
        } else if let Some(point) = point_reference(model, pickable, index) {
            points.push(point);
        } else if let Some(reason) = why_unusable(model, pickable, index) {
            unusable.get_or_insert(reason);
        }
    }
    Chosen {
        planes,
        axes,
        points,
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
            None => {
                "The selected edge is neither straight nor round, so it gives no axis or centre"
            }
        }),
        Pickable::Vertex { body, .. } => Some(match state(body) {
            Some(error) => error.corner(),
            None => "The selected corner is not found where this datum sits in the tree",
        }),
        Pickable::SketchEntity { .. } => {
            Some("Only sketch points made before this datum can place it")
        }
        Pickable::Datum(_) => {
            Some("The selected plane, axis or point comes after this point in the tree")
        }
        _ => None,
    }
}

pub const PLANE_CHOICES: &str = "Select a plane or flat face to offset (and an axis to turn about), \
                             three points, two planes to lie midway between, an axis and a point, \
                             two lines in one plane, or a round or curved edge to stand square to";

pub fn plane_from_selection(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<Datum, &'static str> {
    if let Some(station) = only_pickable(selection)
        .filter(|pickable| axis_reference(model, *pickable, index).is_none())
        .and_then(|pickable| curve_station(model, pickable, index))
    {
        return Ok(Datum::PlaneThrough(PlaneThrough::SquareToCurve(Box::new(
            station,
        ))));
    }
    let Chosen {
        planes,
        axes,
        points,
        unusable,
    } = chosen(model, selection, index);
    if let Some(reason) = unusable {
        return Err(reason);
    }
    match (planes.as_slice(), axes.as_slice(), points.as_slice()) {
        ([], [first, second], []) => {
            return Ok(Datum::PlaneThrough(PlaneThrough::Lines(
                first.clone(),
                second.clone(),
            )));
        }
        ([], [], [first, second, third]) => {
            return Ok(Datum::PlaneThrough(PlaneThrough::Points([
                first.clone(),
                second.clone(),
                third.clone(),
            ])));
        }
        ([first, second], [], []) => {
            return Ok(Datum::PlaneThrough(PlaneThrough::Midway(
                first.clone(),
                second.clone(),
            )));
        }
        ([], [axis], [point]) => {
            return Ok(Datum::PlaneThrough(PlaneThrough::AxisAndPoint(
                axis.clone(),
                point.clone(),
            )));
        }
        (_, _, [_, ..]) => return Err(PLANE_CHOICES),
        _ => {}
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
    Ok(Datum::Plane(DatumPlane {
        base,
        rotation,
        offset: model.length_unit().default_length(offset),
    }))
}

pub fn axis_from_selection(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<DatumAxis, &'static str> {
    let Chosen {
        planes,
        axes,
        points,
        unusable,
    } = chosen(model, selection, index);
    if let Some(reason) = unusable {
        return Err(reason);
    }
    match (planes.as_slice(), axes.as_slice(), points.as_slice()) {
        ([], [axis], []) => Ok(DatumAxis::Along(axis.clone())),
        ([first, second], [], []) => Ok(DatumAxis::Intersection(first.clone(), second.clone())),
        ([], [], [first, second]) => Ok(DatumAxis::Points(first.clone(), second.clone())),
        ([plane], [], [point]) => Ok(DatumAxis::NormalTo(plane.clone(), point.clone())),
        _ => Err(
            "Select one axis, straight edge or round face, two planes or flat faces that cross, \
             two points, or a plane and a point to stand square on",
        ),
    }
}

pub const POINT_CHOICES: &str = "Select one corner, round edge, sphere or torus, sketch point or \
                                 datum point to place it at, a straight or curved edge to measure \
                                 along, two lines that cross, a line and a plane, or three planes";

pub fn point_from_selection(
    model: &Model,
    selection: &Selection,
    index: usize,
) -> Result<Datum, &'static str> {
    let unit = model.length_unit();
    let placed = |base| {
        Datum::Point(DatumPoint {
            base,
            offset: [0.0; 3].map(|value| unit.default_length(value)),
        })
    };
    if let Some(pickable) = only_pickable(selection) {
        if matches!(pickable, Pickable::Face { .. })
            && let Some(centre) = point_reference(model, pickable, index)
        {
            return Ok(placed(centre));
        }
        if matches!(pickable, Pickable::Edge { .. })
            && point_reference(model, pickable, index).is_none()
            && let Some(station) = curve_station(model, pickable, index)
        {
            return Ok(Datum::PointBy(PointBy::Along(Box::new(station))));
        }
    }
    let Chosen {
        planes,
        axes,
        points,
        unusable,
    } = chosen(model, selection, index);
    if let Some(reason) = unusable {
        return Err(reason);
    }
    match (planes.as_slice(), axes.as_slice(), points.as_slice()) {
        ([], [first, second], []) => Ok(Datum::PointBy(PointBy::LinesCross(
            first.clone(),
            second.clone(),
        ))),
        ([plane], [axis], []) => Ok(Datum::PointBy(PointBy::AxisAndPlane(
            axis.clone(),
            plane.clone(),
        ))),
        ([first, second, third], [], []) => Ok(Datum::PointBy(PointBy::ThreePlanes([
            first.clone(),
            second.clone(),
            third.clone(),
        ]))),
        ([], [], [] | [_]) => Ok(placed(
            points.into_iter().next().unwrap_or(PointReference::Origin),
        )),
        _ => Err(POINT_CHOICES),
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
