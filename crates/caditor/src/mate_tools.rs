use caditor_document::{
    AngleMate, AngleSides, AxisMate, AxisReference, Document, Edit, FaceAttachment, FaceAxisMate,
    FaceMate, FaceOnRound, FacePair, FeatureId, FeatureKind, Mate, MatePair, PlaneReference,
    PointMate, PointReference, PointTarget, Transaction, round_face,
};
use caditor_kernel::FaceReference;

use crate::{
    bodies, datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model},
    move_tools,
    selection::{Pickable, Selection},
    sketch_placement, solid_tools,
};

const NAME: &str = "Mate";
const DEFAULT_ANGLE_DEGREES: f64 = 90.0;
const NO_PAIR: &str = "Select a flat face, axis, round face or corner of the body to move, then \
                       the face, plane, axis, round face or point to mate it onto; or a flat face \
                       and an axis of the body, then a face and an axis to mate them onto";
const NO_ANGLE_PAIR: &str = "Select a flat face or axis of the body to move, then the face, plane \
                             or axis to set its angle to";
const SAME_BODY: &str =
    "Select what to mate onto on another body, or a plane or axis; not on the body that moves";
const NO_FACE: &str = "Select a flat face of the body this mate moves";
const NO_AXIS: &str = "Select a straight edge or round face of the body this mate moves";
const NO_ROUND: &str = "Select a cylindrical or spherical face of the body this mate moves";
const NO_POINT: &str =
    "Select a corner, round edge or sphere of the body this mate moves for its point";
const NO_TARGET_PLANE: &str =
    "Select a flat face of another body, or a plane, made before this feature";
const NO_TARGET_AXIS: &str =
    "Select an axis, or a straight edge or round face of another body, made before this feature";
const NO_TARGET_ROUND: &str = "Select a cylindrical or spherical face of another body, made \
                               before this feature";
const NO_TARGET_POINT: &str = "Select a point, corner, flat face or plane made before this \
                               feature, not on the body that moves";
const NO_SECOND_PART: &str = "This mate holds one pair; only a flush and concentric mate also \
                              holds an axis";
const ALREADY: &str = "The mate already uses the selected face or axis";
const GONE: &str = "The feature no longer exists";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatePart {
    Main,
    Axis,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MateSource {
    pub body: FeatureId,
    pub pair: MatePair,
}

fn moving_face(
    model: &Model,
    pickable: Pickable,
    index: usize,
) -> Option<(FeatureId, FaceReference)> {
    match datum_tools::plane_reference(model, pickable, index)? {
        PlaneReference::Face(attachment) => Some((attachment.body, attachment.face)),
        PlaneReference::Principal(_) | PlaneReference::Datum(_) | PlaneReference::Frame { .. } => {
            None
        }
    }
}

fn moving_axis(
    model: &Model,
    pickable: Pickable,
    index: usize,
) -> Option<(FeatureId, AxisReference)> {
    let axis = datum_tools::axis_reference(model, pickable, index)?;
    Some((axis.body()?, axis))
}

fn moving_round(
    model: &Model,
    pickable: Pickable,
    index: usize,
) -> Option<(FeatureId, FaceReference)> {
    let Pickable::Face { body, face } = pickable else {
        return None;
    };
    let shown = bodies::shown(model.evaluation(), body)?;
    let seen = FaceReference::capture(&shown.solid, bodies::find_face(shown, face)?)?;
    let state = sketch_placement::body_state_before(model, body, index).ok()?;
    let reference = FaceReference::capture(state, seen.resolve(state).ok()?)?;
    round_face(state, &reference).ok()?;
    Some((body, reference))
}

fn round_target(model: &Model, pickable: Pickable, index: usize) -> Option<FaceAttachment> {
    let (body, face) = moving_round(model, pickable, index)?;
    Some(FaceAttachment { body, face })
}

fn moving_point(
    model: &Model,
    pickable: Pickable,
    index: usize,
) -> Option<(FeatureId, PointReference)> {
    let point = datum_tools::point_reference(model, pickable, index)?;
    Some((point.body()?, point))
}

fn point_target(model: &Model, pickable: Pickable, index: usize) -> Option<PointTarget> {
    datum_tools::point_reference(model, pickable, index)
        .map(PointTarget::Point)
        .or_else(|| datum_tools::plane_reference(model, pickable, index).map(PointTarget::Plane))
}

fn one_pair(
    model: &Model,
    first: Pickable,
    second: Pickable,
    index: usize,
) -> Option<(FeatureId, MatePair)> {
    if let (Some((body, face)), Some(target)) = (
        moving_face(model, first, index),
        datum_tools::plane_reference(model, second, index),
    ) {
        let distance = model.length_unit().default_length(0.0);
        let faces = FaceMate {
            face,
            target,
            distance,
        };
        return Some((body, MatePair::Faces(Box::new(faces))));
    }
    if let (Some((body, axis)), Some(target)) = (
        moving_axis(model, first, index),
        datum_tools::axis_reference(model, second, index),
    ) {
        return Some((body, MatePair::Axes(Box::new(AxisMate { axis, target }))));
    }
    if let (Some((body, face)), Some(target)) = (
        moving_round(model, first, index),
        datum_tools::plane_reference(model, second, index),
    ) {
        return Some((body, MatePair::Tangent(Box::new(FacePair { face, target }))));
    }
    if let (Some((body, face)), Some(round)) = (
        moving_face(model, first, index),
        round_target(model, second, index),
    ) {
        let resting = FaceOnRound { face, round };
        return Some((body, MatePair::FaceOnRound(Box::new(resting))));
    }
    let (body, point) = moving_point(model, first, index)?;
    let target = point_target(model, second, index)?;
    Some((body, MatePair::Point(Box::new(PointMate { point, target }))))
}

fn face_and_axis(
    model: &Model,
    [first, second]: [Pickable; 2],
    index: usize,
) -> Option<(FeatureId, FaceReference, AxisReference)> {
    [(first, second), (second, first)]
        .into_iter()
        .find_map(|(face, axis)| {
            let (body, face) = moving_face(model, face, index)?;
            let (_, axis) = moving_axis(model, axis, index)?;
            Some((body, face, axis))
        })
}

fn plane_and_axis(
    model: &Model,
    [first, second]: [Pickable; 2],
    index: usize,
) -> Option<(PlaneReference, AxisReference)> {
    [(first, second), (second, first)]
        .into_iter()
        .find_map(|(plane, axis)| {
            Some((
                datum_tools::plane_reference(model, plane, index)?,
                datum_tools::axis_reference(model, axis, index)?,
            ))
        })
}

fn flush_and_concentric(
    model: &Model,
    picked: &[Pickable],
    body: FeatureId,
    index: usize,
) -> Option<(FeatureId, MatePair)> {
    let (on, off): (Vec<Pickable>, Vec<Pickable>) = picked
        .iter()
        .partition(|pickable| pickable.body() == Some(body));
    let (Ok(on), Ok(off)) = (
        <[Pickable; 2]>::try_from(on),
        <[Pickable; 2]>::try_from(off),
    ) else {
        return None;
    };
    let (body, face, axis) = face_and_axis(model, on, index)?;
    let (target, axis_target) = plane_and_axis(model, off, index)?;
    let faces = FaceMate {
        face,
        target,
        distance: model.length_unit().default_length(0.0),
    };
    let axes = AxisMate {
        axis,
        target: axis_target,
    };
    Some((
        body,
        MatePair::FaceAxis(Box::new(FaceAxisMate { faces, axes })),
    ))
}

fn shown(model: &Model, body: FeatureId) -> Result<FeatureId, &'static str> {
    move_tools::chosen_body(model, &Selection::default(), &[body], NO_PAIR)
}

pub fn source(model: &Model, selection: &Selection) -> Result<MateSource, &'static str> {
    let index = model.document().bar_index();
    let picked = selection.in_pick_order();
    let found = match picked.as_slice() {
        [first, second] => {
            let body = first.body().ok_or(NO_PAIR)?;
            if second.body() == Some(body) {
                return Err(SAME_BODY);
            }
            one_pair(model, *first, *second, index)
        }
        [first, _, _, _] => {
            let body = first.body().ok_or(NO_PAIR)?;
            flush_and_concentric(model, &picked, body, index)
        }
        _ => None,
    };
    let (body, pair) = found.ok_or(NO_PAIR)?;
    Ok(MateSource {
        body: shown(model, body)?,
        pair,
    })
}

pub fn angle_source(model: &Model, selection: &Selection) -> Result<MateSource, &'static str> {
    let index = model.document().bar_index();
    let picked = selection.in_pick_order();
    let [first, second] = picked.as_slice() else {
        return Err(NO_ANGLE_PAIR);
    };
    let body = first.body().ok_or(NO_ANGLE_PAIR)?;
    if second.body() == Some(body) {
        return Err(SAME_BODY);
    }
    let sides = match one_pair(model, *first, *second, index) {
        Some((_, MatePair::Faces(faces))) => AngleSides::Faces(FacePair {
            face: faces.face,
            target: faces.target,
        }),
        Some((_, MatePair::Axes(axes))) => AngleSides::Axes(*axes),
        Some(_) | None => return Err(NO_ANGLE_PAIR),
    };
    let angle = AngleMate {
        sides,
        angle: solid_tools::degrees(DEFAULT_ANGLE_DEGREES),
    };
    Ok(MateSource {
        body: shown(model, body)?,
        pair: MatePair::Angle(Box::new(angle)),
    })
}

pub fn create(document: &Document, source: &MateSource) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, NAME);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Mate(Mate {
            body: source.body,
            pair: source.pair.clone(),
            flipped: false,
        }),
    );
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, source: &MateSource) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), source);
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, mate: Mate) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Mate(mate),
        },
    ))
}

pub fn change(model: &Model, feature: FeatureId, mate: Mate) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, mate).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

fn index_of(model: &Model, feature: FeatureId) -> Result<usize, String> {
    model
        .document()
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())
}

fn changed(
    model: &Model,
    feature: FeatureId,
    mate: &Mate,
    pair: MatePair,
) -> Result<Transaction, String> {
    if pair == mate.pair {
        return Err(ALREADY.to_owned());
    }
    change(
        model,
        feature,
        Mate {
            pair,
            ..mate.clone()
        },
    )
}

struct Picks<'a> {
    model: &'a Model,
    picked: Vec<Pickable>,
    index: usize,
}

impl Picks<'_> {
    fn first<T>(
        &self,
        find: impl Fn(&Model, Pickable, usize) -> Option<T>,
        missing: &'static str,
    ) -> Result<T, &'static str> {
        self.picked
            .iter()
            .find_map(|pickable| find(self.model, *pickable, self.index))
            .ok_or(missing)
    }

    fn face(&self) -> Result<FaceReference, &'static str> {
        self.first(moving_face, NO_FACE).map(|(_, face)| face)
    }

    fn axis(&self) -> Result<AxisReference, &'static str> {
        self.first(moving_axis, NO_AXIS).map(|(_, axis)| axis)
    }

    fn plane(&self) -> Result<PlaneReference, &'static str> {
        self.first(datum_tools::plane_reference, NO_TARGET_PLANE)
    }

    fn target_axis(&self) -> Result<AxisReference, &'static str> {
        self.first(datum_tools::axis_reference, NO_TARGET_AXIS)
    }
}

fn moved_pair(
    picks: &Picks<'_>,
    pair: &MatePair,
    part: MatePart,
) -> Result<MatePair, &'static str> {
    Ok(match (pair, part) {
        (MatePair::Faces(faces), MatePart::Main) => MatePair::Faces(Box::new(FaceMate {
            face: picks.face()?,
            ..faces.as_ref().clone()
        })),
        (MatePair::Axes(axes), MatePart::Main) => MatePair::Axes(Box::new(AxisMate {
            axis: picks.axis()?,
            ..axes.as_ref().clone()
        })),
        (MatePair::FaceAxis(both), MatePart::Main) => {
            let mut both = both.as_ref().clone();
            both.faces.face = picks.face()?;
            MatePair::FaceAxis(Box::new(both))
        }
        (MatePair::FaceAxis(both), MatePart::Axis) => {
            let mut both = both.as_ref().clone();
            both.axes.axis = picks.axis()?;
            MatePair::FaceAxis(Box::new(both))
        }
        (MatePair::Angle(angle), MatePart::Main) => {
            let mut angle = angle.as_ref().clone();
            match &mut angle.sides {
                AngleSides::Faces(faces) => faces.face = picks.face()?,
                AngleSides::Axes(axes) => axes.axis = picks.axis()?,
            }
            MatePair::Angle(Box::new(angle))
        }
        (MatePair::Tangent(tangent), MatePart::Main) => MatePair::Tangent(Box::new(FacePair {
            face: picks.first(moving_round, NO_ROUND)?.1,
            ..tangent.as_ref().clone()
        })),
        (MatePair::Point(point), MatePart::Main) => MatePair::Point(Box::new(PointMate {
            point: picks.first(moving_point, NO_POINT)?.1,
            ..point.as_ref().clone()
        })),
        (MatePair::FaceOnRound(resting), MatePart::Main) => {
            MatePair::FaceOnRound(Box::new(FaceOnRound {
                face: picks.face()?,
                ..resting.as_ref().clone()
            }))
        }
        (
            MatePair::Faces(_)
            | MatePair::Axes(_)
            | MatePair::Angle(_)
            | MatePair::Tangent(_)
            | MatePair::Point(_)
            | MatePair::FaceOnRound(_),
            MatePart::Axis,
        ) => return Err(NO_SECOND_PART),
    })
}

fn target_pair(
    picks: &Picks<'_>,
    pair: &MatePair,
    part: MatePart,
) -> Result<MatePair, &'static str> {
    Ok(match (pair, part) {
        (MatePair::Faces(faces), MatePart::Main) => MatePair::Faces(Box::new(FaceMate {
            target: picks.plane()?,
            ..faces.as_ref().clone()
        })),
        (MatePair::Axes(axes), MatePart::Main) => MatePair::Axes(Box::new(AxisMate {
            target: picks.target_axis()?,
            ..axes.as_ref().clone()
        })),
        (MatePair::FaceAxis(both), MatePart::Main) => {
            let mut both = both.as_ref().clone();
            both.faces.target = picks.plane()?;
            MatePair::FaceAxis(Box::new(both))
        }
        (MatePair::FaceAxis(both), MatePart::Axis) => {
            let mut both = both.as_ref().clone();
            both.axes.target = picks.target_axis()?;
            MatePair::FaceAxis(Box::new(both))
        }
        (MatePair::Angle(angle), MatePart::Main) => {
            let mut angle = angle.as_ref().clone();
            match &mut angle.sides {
                AngleSides::Faces(faces) => faces.target = picks.plane()?,
                AngleSides::Axes(axes) => axes.target = picks.target_axis()?,
            }
            MatePair::Angle(Box::new(angle))
        }
        (MatePair::Tangent(tangent), MatePart::Main) => MatePair::Tangent(Box::new(FacePair {
            target: picks.plane()?,
            ..tangent.as_ref().clone()
        })),
        (MatePair::Point(point), MatePart::Main) => MatePair::Point(Box::new(PointMate {
            target: picks.first(point_target, NO_TARGET_POINT)?,
            ..point.as_ref().clone()
        })),
        (MatePair::FaceOnRound(resting), MatePart::Main) => {
            MatePair::FaceOnRound(Box::new(FaceOnRound {
                round: picks.first(round_target, NO_TARGET_ROUND)?,
                ..resting.as_ref().clone()
            }))
        }
        (
            MatePair::Faces(_)
            | MatePair::Axes(_)
            | MatePair::Angle(_)
            | MatePair::Tangent(_)
            | MatePair::Point(_)
            | MatePair::FaceOnRound(_),
            MatePart::Axis,
        ) => return Err(NO_SECOND_PART),
    })
}

pub fn moving_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mate: &Mate,
    part: MatePart,
) -> Result<Transaction, String> {
    let picks = Picks {
        model,
        picked: selection
            .in_pick_order()
            .into_iter()
            .filter(|pickable| pickable.body() == Some(mate.body))
            .collect(),
        index: index_of(model, feature)?,
    };
    let pair = moved_pair(&picks, &mate.pair, part)?;
    changed(model, feature, mate, pair)
}

pub fn target_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mate: &Mate,
    part: MatePart,
) -> Result<Transaction, String> {
    let picks = Picks {
        model,
        picked: selection
            .in_pick_order()
            .into_iter()
            .filter(|pickable| pickable.body() != Some(mate.body))
            .collect(),
        index: index_of(model, feature)?,
    };
    let pair = target_pair(&picks, &mate.pair, part)?;
    changed(model, feature, mate, pair)
}
