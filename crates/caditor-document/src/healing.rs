use std::sync::Arc;

use caditor_kernel::{EdgeNaming, EdgeReference, FaceReference, RegionReference, resolve_regions};

use crate::{
    attachment::SketchAttachment,
    datum::{
        AxisReference, CurveStation, Datum, DatumAxis, PlaneReference, PlaneThrough, PointBy,
        PointReference,
    },
    document::{Document, Feature, FeatureId, FeatureKind, list_names},
    edit::{Edit, Transaction},
    mate::MatePair,
    movement::TurnCentre,
    pattern::PatternKind,
    recompute::Inputs,
    solid::{ExtrudeEnd, ExtrudeExtent, RegionChoice, RevolveAxis, SolidFeature, SolidStart},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Healing {
    pub matched: Vec<String>,
    pub dropped: Vec<String>,
    pub refreshed: FeatureKind,
    checked: Arc<Feature>,
    matched_count: usize,
    dropped_count: usize,
}

impl Healing {
    pub fn reason(&self) -> String {
        let were = |count: usize| if count == 1 { "was" } else { "were" };
        let matched = (!self.matched.is_empty()).then(|| {
            format!(
                "{} {} matched to the most similar geometry",
                list_names(&self.matched),
                were(self.matched_count)
            )
        });
        let dropped = (!self.dropped.is_empty()).then(|| {
            let exists = if self.dropped_count == 1 {
                "exists"
            } else {
                "exist"
            };
            format!(
                "{} no longer {exists} and {} left out",
                list_names(&self.dropped),
                were(self.dropped_count)
            )
        });
        let parts: Vec<String> = matched.into_iter().chain(dropped).collect();
        format!("After an upstream change, {}.", parts.join(", and "))
    }

    pub fn remedy(&self) -> &'static str {
        "Check the result, then update its references to keep it."
    }

    pub fn update(&self, document: &Document) -> Option<Transaction> {
        let current = document.feature(self.checked.id())?;
        if current.kind != self.checked.kind {
            return None;
        }
        let label = format!("Update references of {}", current.name);
        let edit = match (&current.kind, &self.refreshed) {
            (FeatureKind::Sketch(sketch), FeatureKind::Sketch(refreshed)) => {
                Edit::SetSketchPlacement {
                    feature: current.id(),
                    plane: sketch.sketch.plane(),
                    attachment: refreshed.attachment.clone(),
                }
            }
            _ => Edit::SetFeatureKind {
                id: current.id(),
                kind: self.refreshed.clone(),
            },
        };
        Some(Transaction::single(label, edit))
    }
}

pub(crate) fn check(feature: &Arc<Feature>, inputs: &Inputs<'_>) -> Option<Healing> {
    let mut healer = Healer {
        inputs,
        matched: Vec::new(),
        dropped: Vec::new(),
        matched_count: 0,
        dropped_count: 0,
    };
    let mut refreshed = feature.kind.clone();
    visit(&mut refreshed, &mut healer);
    (!healer.matched.is_empty() || !healer.dropped.is_empty()).then(|| Healing {
        matched: healer.matched,
        dropped: healer.dropped,
        refreshed,
        checked: Arc::clone(feature),
        matched_count: healer.matched_count,
        dropped_count: healer.dropped_count,
    })
}

pub(crate) trait ReferenceVisitor {
    fn face(&mut self, body: FeatureId, face: &mut FaceReference, what: &str);

    fn edges(
        &mut self,
        body: FeatureId,
        edges: &mut [EdgeReference],
        what: &dyn Fn(usize) -> String,
    );

    fn regions(&mut self, sketch: FeatureId, choice: &mut RegionChoice);
}

pub(crate) fn visit(kind: &mut FeatureKind, visitor: &mut impl ReferenceVisitor) {
    match kind {
        FeatureKind::Sketch(sketch) => {
            if let Some(SketchAttachment::Face(attachment)) = &mut sketch.attachment {
                visitor.face(attachment.body, &mut attachment.face, "the face it lies on");
            }
        }
        FeatureKind::Solid(solid) => visit_solid(solid, visitor),
        FeatureKind::Blend(blend) => {
            let count = blend.edges.len();
            visitor.edges(blend.body, &mut blend.edges, &|index| {
                if count == 1 {
                    "its edge".to_owned()
                } else {
                    format!("edge {} of {count}", index + 1)
                }
            });
        }
        FeatureKind::Move(movement) => {
            if let TurnCentre::Axis(turn) = &mut movement.about {
                visit_axis(&mut turn.axis, "axis to turn about", visitor);
            }
        }
        FeatureKind::Mate(mate) => match &mut mate.pair {
            MatePair::Faces(faces) => {
                visitor.face(mate.body, &mut faces.face, "the face it mates");
                visit_plane(&mut faces.target, "the face it mates onto", visitor);
            }
            MatePair::Axes(axes) => {
                visit_axis(&mut axes.axis, "axis to mate", visitor);
                visit_axis(&mut axes.target, "axis to mate onto", visitor);
            }
        },
        FeatureKind::Combine(_)
        | FeatureKind::Scale(_)
        | FeatureKind::Hole(_)
        | FeatureKind::Remove(_) => {}
        FeatureKind::Mirror(mirror) => {
            visit_plane(&mut mirror.plane, "the face it mirrors across", visitor);
        }
        FeatureKind::Split(split) => {
            if let Some(plane) = split.along.plane_mut() {
                visit_plane(plane, "the face it splits along", visitor);
            }
        }
        FeatureKind::Primitive(primitive) => {
            visit_plane(&mut primitive.plane, "the face it stands on", visitor);
        }
        FeatureKind::Shell(shell) => {
            let count = shell.open.len();
            for (index, face) in shell.open.iter_mut().enumerate() {
                let what = if count == 1 {
                    "its open face".to_owned()
                } else {
                    format!("open face {} of {count}", index + 1)
                };
                visitor.face(shell.body, face, &what);
            }
        }
        FeatureKind::OffsetFace(offset) => {
            let count = offset.faces.len();
            for (index, face) in offset.faces.iter_mut().enumerate() {
                let what = if count == 1 {
                    "its moved face".to_owned()
                } else {
                    format!("moved face {} of {count}", index + 1)
                };
                visitor.face(offset.body, face, &what);
            }
        }
        FeatureKind::Thread(thread) => {
            visitor.face(thread.body, &mut thread.face, "its threaded face");
        }
        FeatureKind::Pattern(pattern) => match &mut pattern.kind {
            PatternKind::Linear { first, second } => {
                visit_axis(&mut first.axis, "first direction", visitor);
                if let Some(second) = second {
                    visit_axis(&mut second.axis, "second direction", visitor);
                }
            }
            PatternKind::Circular(circular) => visit_axis(&mut circular.axis, "axis", visitor),
        },
        FeatureKind::Datum(Datum::Plane(plane)) => {
            visit_plane(&mut plane.base, "the face it is based on", visitor);
            if let Some(rotation) = &mut plane.rotation {
                visit_axis(&mut rotation.axis, "rotation axis", visitor);
            }
        }
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(axis))) => {
            visit_axis(axis, "line", visitor);
        }
        FeatureKind::Datum(Datum::Axis(DatumAxis::Intersection(first, second))) => {
            visit_plane(first, "the first face it is the intersection of", visitor);
            visit_plane(second, "the second face it is the intersection of", visitor);
        }
        FeatureKind::Datum(Datum::Axis(DatumAxis::Points(first, second))) => {
            visit_point(first, "first point", visitor);
            visit_point(second, "second point", visitor);
        }
        FeatureKind::Datum(Datum::Axis(DatumAxis::NormalTo(plane, point))) => {
            visit_plane(plane, "the face it stands square to", visitor);
            visit_point(point, "point", visitor);
        }
        FeatureKind::Datum(Datum::Point(point)) => visit_point(&mut point.base, "place", visitor),
        FeatureKind::Datum(Datum::PlaneThrough(through)) => match through {
            PlaneThrough::Points(points) => {
                for (point, role) in
                    points
                        .iter_mut()
                        .zip(["first point", "second point", "third point"])
                {
                    visit_point(point, role, visitor);
                }
            }
            PlaneThrough::Midway(first, second) => {
                visit_plane(first, "the first face it lies midway between", visitor);
                visit_plane(second, "the second face it lies midway between", visitor);
            }
            PlaneThrough::AxisAndPoint(axis, point) | PlaneThrough::NormalTo(axis, point) => {
                visit_axis(axis, "axis", visitor);
                visit_point(point, "point", visitor);
            }
            PlaneThrough::Tangent(tangent) => {
                visitor.face(
                    tangent.body,
                    &mut tangent.face,
                    "the face it lies tangent to",
                );
                visit_point(&mut tangent.toward, "point", visitor);
            }
            PlaneThrough::SquareToCurve(station) => visit_station(station, visitor),
            PlaneThrough::Lines(first, second) => {
                visit_axis(first, "first line", visitor);
                visit_axis(second, "second line", visitor);
            }
        },
        FeatureKind::Datum(Datum::PointBy(by)) => match by {
            PointBy::LinesCross(first, second) => {
                visit_axis(first, "first line", visitor);
                visit_axis(second, "second line", visitor);
            }
            PointBy::AxisAndPlane(axis, plane) => {
                visit_axis(axis, "line", visitor);
                visit_plane(plane, "the face it meets the line at", visitor);
            }
            PointBy::ThreePlanes(planes) => {
                for (plane, role) in planes.iter_mut().zip([
                    "the first face it lies at",
                    "the second face it lies at",
                    "the third face it lies at",
                ]) {
                    visit_plane(plane, role, visitor);
                }
            }
            PointBy::Along(station) => visit_station(station, visitor),
        },
        FeatureKind::Import(_) => {}
    }
}

fn visit_axis(axis: &mut AxisReference, role: &str, visitor: &mut impl ReferenceVisitor) {
    match axis {
        AxisReference::Edge { body, edge } => {
            let what = format!("the edge giving its {role}");
            visitor.edges(*body, std::slice::from_mut(edge.as_mut()), &|_| {
                what.clone()
            });
        }
        AxisReference::Face { body, face } => {
            visitor.face(*body, face, &format!("the face giving its {role}"));
        }
        AxisReference::Principal(_) | AxisReference::Datum(_) | AxisReference::Sketch { .. } => {}
    }
}

fn visit_point(point: &mut PointReference, role: &str, visitor: &mut impl ReferenceVisitor) {
    match point {
        PointReference::Centre { body, edge } => {
            let what = format!("the round edge giving its {role}");
            visitor.edges(*body, std::slice::from_mut(edge.as_mut()), &|_| {
                what.clone()
            });
        }
        PointReference::SurfaceCentre { body, face } => {
            visitor.face(*body, face, &format!("the face giving its {role}"));
        }
        PointReference::Origin
        | PointReference::Datum(_)
        | PointReference::Vertex { .. }
        | PointReference::Sketch { .. } => {}
    }
}

fn visit_station(station: &mut CurveStation, visitor: &mut impl ReferenceVisitor) {
    visitor.edges(
        station.body,
        std::slice::from_mut(station.edge.as_mut()),
        &|_| "the edge it follows".to_owned(),
    );
}

fn visit_plane(plane: &mut PlaneReference, what: &str, visitor: &mut impl ReferenceVisitor) {
    if let PlaneReference::Face(attachment) = plane {
        visitor.face(attachment.body, &mut attachment.face, what);
    }
}

fn visit_solid(solid: &mut SolidFeature, visitor: &mut impl ReferenceVisitor) {
    match solid {
        SolidFeature::Extrude(extrude) => {
            visitor.regions(extrude.sketch, &mut extrude.regions);
            let ends: Vec<&mut ExtrudeEnd> = match &mut extrude.extent {
                ExtrudeExtent::OneSide { end, .. } => vec![end],
                ExtrudeExtent::Symmetric { .. } => Vec::new(),
                ExtrudeExtent::TwoSides { forward, backward } => vec![forward, backward],
            };
            let count = ends.len();
            for (index, end) in ends.into_iter().enumerate() {
                if let ExtrudeEnd::UpToFace(target) = end {
                    let what = match (count, index) {
                        (1, _) => "the face its end runs up to",
                        (_, 0) => "the face its forward end runs up to",
                        _ => "the face its backward end runs up to",
                    };
                    visit_plane(target, what, visitor);
                }
            }
            if let Some(SolidStart::Plane(target)) = &mut extrude.start {
                visit_plane(target, "the face it starts from", visitor);
            }
        }
        SolidFeature::Revolve(revolve) => {
            visitor.regions(revolve.sketch, &mut revolve.regions);
            if let RevolveAxis::Model(axis) = &mut revolve.axis {
                visit_axis(axis, "axis", visitor);
            }
            if let Some(SolidStart::Plane(target)) = &mut revolve.start {
                visit_plane(target, "the face it starts from", visitor);
            }
        }
    }
}

struct Healer<'a> {
    inputs: &'a Inputs<'a>,
    matched: Vec<String>,
    dropped: Vec<String>,
    matched_count: usize,
    dropped_count: usize,
}

impl Healer<'_> {
    fn matched(&mut self, what: String, count: usize) {
        self.matched.push(what);
        self.matched_count += count;
    }
}

impl ReferenceVisitor for Healer<'_> {
    fn face(&mut self, body: FeatureId, face: &mut FaceReference, what: &str) {
        let Some(solid) = self.inputs.body(body) else {
            return;
        };
        let Ok(found) = face.resolve(solid) else {
            return;
        };
        if solid
            .face(found)
            .is_none_or(|definition| definition.name() == face.name())
        {
            return;
        }
        if let Some(captured) = FaceReference::capture(solid, found) {
            *face = captured;
            self.matched(what.to_owned(), 1);
        }
    }

    fn edges(
        &mut self,
        body: FeatureId,
        edges: &mut [EdgeReference],
        what: &dyn Fn(usize) -> String,
    ) {
        let Some(solid) = self.inputs.body(body) else {
            return;
        };
        let naming = EdgeNaming::new(solid);
        for (index, edge) in edges.iter_mut().enumerate() {
            let Ok(found) = edge.resolve_in(&naming) else {
                continue;
            };
            if solid
                .edge(found)
                .is_none_or(|definition| definition.name() == edge.name())
            {
                continue;
            }
            if let Some(captured) = EdgeReference::capture_in(&naming, found) {
                *edge = captured;
                self.matched(what(index), 1);
            }
        }
    }

    fn regions(&mut self, sketch: FeatureId, choice: &mut RegionChoice) {
        let RegionChoice::Chosen(references) = choice else {
            return;
        };
        let Some(profile) = self
            .inputs
            .features
            .get(&sketch)
            .and_then(|result| result.sketch())
            .and_then(|result| result.profile().ok())
        else {
            return;
        };
        let Ok(resolved) = resolve_regions(references, profile.regions()) else {
            return;
        };
        if resolved.healed.is_empty() && resolved.gone.is_empty() {
            return;
        }

        let sketch_name = self
            .inputs
            .document
            .feature(sketch)
            .map_or_else(|| "its sketch".to_owned(), |feature| feature.name.clone());
        let described = |count: usize| {
            if count == 1 {
                format!("a chosen region of {sketch_name}")
            } else {
                format!("{count} chosen regions of {sketch_name}")
            }
        };
        if !resolved.healed.is_empty() {
            self.matched(described(resolved.healed.len()), resolved.healed.len());
        }
        if !resolved.gone.is_empty() {
            self.dropped.push(described(resolved.gone.len()));
            self.dropped_count += resolved.gone.len();
        }

        let kept: Vec<RegionReference> = resolved
            .keys
            .iter()
            .filter_map(|key| {
                let unchanged = references.iter().find(|reference| reference.key() == *key);
                match unchanged {
                    Some(reference) => Some(reference.clone()),
                    None => {
                        let region = profile.region(*key)?;
                        Some(RegionReference::capture(region, region.anchor()))
                    }
                }
            })
            .collect();
        *references = kept;
    }
}
