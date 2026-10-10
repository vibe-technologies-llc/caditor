use caditor_document::{
    Datum, DatumPoint, Document, Edit, FaceTangent, FeatureId, FeatureKind, Hole, PlaneThrough,
    PointReference, SketchAttachment, SketchFeature, Transaction,
};
use caditor_geometry::{Plane, Point2, Point3, Ray, Vector3};
use caditor_kernel::{FaceId, RayCrossing, Solid};
use caditor_sketch::Sketch;

use crate::{
    bodies,
    body_selection::face_boundary,
    editing,
    hole_tools::{self, TITLE},
    last_values::Starts,
    model::Model,
    sketch_placement::{self, FaceChoice},
};

const FACE_SAMPLES: u32 = 16;
const NOT_ON_A_CURVE: &str = "This hole stands on a flat face or a plane; edit its sketch to \
                              place it on a curved face";
const NO_SPOT: &str = "No spot on that face could be found for the hole";
const GONE: &str = "The hole no longer exists";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mount {
    pub plane: FeatureId,
    pub point: FeatureId,
}

pub fn mount(document: &Document, hole: &Hole) -> Option<Mount> {
    let Some(SketchAttachment::Datum(plane)) = document.feature(hole.sketch)?.kind.attachment()
    else {
        return None;
    };
    let Some(Datum::PlaneThrough(PlaneThrough::TangentAt(tangent))) =
        document.feature(*plane)?.kind.datum()
    else {
        return None;
    };
    let PointReference::Datum(point) = tangent.toward else {
        return None;
    };
    matches!(
        document.feature(point)?.kind.datum(),
        Some(Datum::Point(DatumPoint {
            base: PointReference::Origin,
            ..
        }))
    )
    .then_some(Mount {
        plane: *plane,
        point,
    })
}

fn middle_of(solid: &Solid, face: FaceId) -> Option<Point3> {
    let samples: Vec<Point3> = face_boundary(solid, face)
        .into_iter()
        .filter_map(|edge| solid.edge(edge))
        .flat_map(|edge| {
            let interval = edge.interval();
            (0..FACE_SAMPLES).map(move |step| {
                let t = f64::from(step) / f64::from(FACE_SAMPLES);
                edge.curve()
                    .point(interval.start() + (interval.end() - interval.start()) * t)
            })
        })
        .collect();
    let count = samples.len() as f64;
    let mean = samples
        .into_iter()
        .fold(Point3::ZERO, |sum, point| sum + point)
        / count.max(1.0);
    let surface = solid.face(face)?.surface();
    let uv = surface.project(mean, None);
    Some(surface.point_at(uv))
}

fn spot(model: &Model, face: FaceChoice, ray: Option<Ray>) -> Option<(Point3, Vector3)> {
    let body = bodies::shown(model.evaluation(), face.body)?;
    let id = bodies::find_face(body, face.face)?;
    let solid = &body.solid;
    let hit = ray.and_then(|ray| {
        match solid
            .classifier()
            .first_crossing(ray.origin(), ray.direction(), 0.0)
        {
            RayCrossing::Crossing { distance, .. } => Some(ray.at(distance)),
            RayCrossing::Nothing | RayCrossing::Undecided => None,
        }
    });
    let near = match hit {
        Some(hit) => hit,
        None => middle_of(solid, id)?,
    };
    let definition = solid.face(id)?;
    let surface = definition.surface();
    let uv = surface.project(near, None);
    let at = surface.point_at(uv);
    let normal = surface.normal(uv.x, uv.y)? * definition.sense().sign();
    Some((at, normal))
}

fn offsets(model: &Model, at: Point3) -> [caditor_expression::Expression; 3] {
    let unit = model.length_unit();
    [at.x, at.y, at.z].map(|value| unit.measured(value))
}

pub fn create(
    model: &Model,
    face: FaceChoice,
) -> Result<(Transaction, FeatureId, String), &'static str> {
    let document = model.document();
    let attachment = sketch_placement::surface_at(model, face, document.bar_index())?;
    let (at, normal) = spot(model, face, None).ok_or(NO_SPOT)?;
    let plane = Plane::new(at, normal).ok_or(NO_SPOT)?;
    let name = editing::next_feature_name(document, TITLE);
    let sketch_name = editing::next_sketch_name(document);
    let mut transaction = document.transaction(format!("Create {name}"));
    let point = transaction.add_feature(
        editing::next_feature_name(document, "Point"),
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::Origin,
            offset: offsets(model, at),
        })),
    );
    let tangent = transaction.add_feature(
        editing::next_feature_name(document, "Plane"),
        FeatureKind::Datum(Datum::PlaneThrough(PlaneThrough::TangentAt(Box::new(
            FaceTangent {
                body: attachment.body,
                face: attachment.face,
                toward: PointReference::Datum(point),
            },
        )))),
    );
    let mut sketch = Sketch::new(plane);
    sketch.add_point(Point2::ZERO);
    let placed = transaction.add_feature(
        sketch_name.clone(),
        FeatureKind::Sketch(SketchFeature {
            attachment: Some(SketchAttachment::Datum(tangent)),
            ..SketchFeature::from(sketch)
        }),
    );
    for id in [point, tangent, placed] {
        transaction.edit(Edit::SetFeatureHidden { id, hidden: true });
    }
    let feature = transaction.add_feature(
        name.clone(),
        hole_tools::new_hole(placed, face.body, &Starts::of(model)),
    );
    let told = format!(
        "{name} is drilled square into the curved face. Click where it goes on the face, or \
         choose another spot later from its panel's Placed on row."
    );
    Ok((transaction.finish(), feature, told))
}

pub fn moved(
    model: &Model,
    feature: FeatureId,
    hole: &Hole,
    face: FaceChoice,
    ray: Option<Ray>,
) -> Result<Transaction, String> {
    let document = model.document();
    let name = &document.feature(feature).ok_or(GONE)?.name;
    let mount = mount(document, hole).ok_or(NOT_ON_A_CURVE)?;
    let index = document.feature_index(mount.plane).ok_or(GONE)?;
    let attachment = sketch_placement::surface_at(model, face, index)?;
    let (at, _) = spot(model, face, ray).ok_or(NO_SPOT)?;
    let mut transaction = document.transaction(format!("Move {name}"));
    transaction.edit(Edit::SetFeatureKind {
        id: mount.point,
        kind: FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::Origin,
            offset: offsets(model, at),
        })),
    });
    transaction.edit(Edit::SetFeatureKind {
        id: mount.plane,
        kind: FeatureKind::Datum(Datum::PlaneThrough(PlaneThrough::TangentAt(Box::new(
            FaceTangent {
                body: attachment.body,
                face: attachment.face,
                toward: PointReference::Datum(mount.point),
            },
        )))),
    });
    if hole.body != face.body {
        transaction.edit(Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Hole(Hole {
                body: face.body,
                ..hole.clone()
            }),
        });
    }
    let transaction = transaction.finish();
    document
        .check(&transaction)
        .map_err(|_| "The hole cannot stand on that face".to_owned())?;
    Ok(transaction)
}

pub fn face_words(document: &Document, mount: Mount) -> Option<String> {
    let Some(Datum::PlaneThrough(PlaneThrough::TangentAt(tangent))) =
        document.feature(mount.plane)?.kind.datum()
    else {
        return None;
    };
    Some(bodies::describe_origin(document, tangent.face.origin()))
}

pub fn moved_to(
    model: &Model,
    feature: FeatureId,
    hole: &Hole,
    near: Point3,
) -> Result<Transaction, String> {
    let document = model.document();
    let name = &document.feature(feature).ok_or(GONE)?.name;
    let mount = mount(document, hole).ok_or(NOT_ON_A_CURVE)?;
    let Some(Datum::PlaneThrough(PlaneThrough::TangentAt(tangent))) = document
        .feature(mount.plane)
        .and_then(|plane| plane.kind.datum())
    else {
        return Err(NOT_ON_A_CURVE.to_owned());
    };
    let body = bodies::shown(model.evaluation(), tangent.body).ok_or(NO_SPOT)?;
    let face = tangent.face.resolve(&body.solid).map_err(|_| NO_SPOT)?;
    let surface = body.solid.face(face).ok_or(NO_SPOT)?.surface();
    let at = surface.point_at(surface.project(near, None));
    let transaction = Transaction::single(
        format!("Move {name}"),
        Edit::SetFeatureKind {
            id: mount.point,
            kind: FeatureKind::Datum(Datum::Point(DatumPoint {
                base: PointReference::Origin,
                offset: offsets(model, at),
            })),
        },
    );
    document
        .check(&transaction)
        .map_err(|_| "The hole cannot stand there".to_owned())?;
    Ok(transaction)
}
