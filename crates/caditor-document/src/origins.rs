use std::collections::BTreeSet;

use caditor_kernel::{EdgeNaming, EdgeReference, FaceReference};

use crate::{
    attachment::SketchAttachment,
    datum::PlaneReference,
    document::{Document, FeatureId, FeatureKind, RollbackBar},
    edit::{Edit, Transaction},
    healing::{ReferenceVisitor, visit},
    recompute::{CancelToken, Evaluation, FeatureState, ModelEvaluator, Recompute},
    solid::RegionChoice,
};

const COMPLETION_LABEL: &str = "Complete references";

pub(crate) fn of_face(face: &FaceReference) -> BTreeSet<FeatureId> {
    face.origin()
        .into_iter()
        .flat_map(|origin| origin.features())
        .map(FeatureId::from_raw)
        .collect()
}

pub(crate) fn of_edge(edge: &EdgeReference) -> BTreeSet<FeatureId> {
    edge.origins()
        .into_iter()
        .flatten()
        .flat_map(|origin| origin.features())
        .map(FeatureId::from_raw)
        .collect()
}

pub fn complete_origins(document: &Document, cancel: &CancelToken) -> Transaction {
    let holders: Vec<(FeatureId, FeatureKind)> = document
        .features()
        .filter(|feature| may_hold_references(&feature.kind))
        .filter_map(|feature| {
            let mut kind = feature.kind.clone();
            let mut lacking = Lacking::default();
            visit(&mut kind, &mut lacking);
            lacking.found.then_some((feature.id(), kind))
        })
        .collect();
    let Some((last, _)) = holders.last() else {
        return Transaction::new(COMPLETION_LABEL, Vec::new());
    };

    let mut evaluated = document.clone();
    let bar = document
        .features()
        .skip_while(|feature| feature.id() != *last)
        .nth(1)
        .map_or(RollbackBar::AtEnd, |next| RollbackBar::Before(next.id()));
    if evaluated
        .apply(evaluated.roll_to(bar, COMPLETION_LABEL))
        .is_err()
    {
        return Transaction::new(COMPLETION_LABEL, Vec::new());
    }
    let evaluation = Recompute::default().run_without_display(&evaluated, &ModelEvaluator, cancel);

    let edits = holders
        .into_iter()
        .filter_map(|(feature, mut kind)| {
            let reached = evaluation
                .feature(feature)
                .is_some_and(|status| status.state != FeatureState::Outdated);
            let mut completer = Completer {
                feature,
                evaluation: &evaluation,
                completed: false,
            };
            if reached {
                visit(&mut kind, &mut completer);
            }
            if !completer.completed {
                return None;
            }
            completion_edit(document, feature, kind)
        })
        .collect();
    Transaction::new(COMPLETION_LABEL, edits)
}

fn may_hold_references(kind: &FeatureKind) -> bool {
    match kind {
        FeatureKind::Sketch(sketch) => {
            matches!(sketch.attachment, Some(SketchAttachment::Face(_)))
        }
        FeatureKind::Mirror(mirror) => matches!(mirror.plane, PlaneReference::Face(_)),
        FeatureKind::Split(split) => matches!(split.plane, PlaneReference::Face(_)),
        FeatureKind::Primitive(primitive) => matches!(primitive.plane, PlaneReference::Face(_)),
        FeatureKind::Move(movement) => movement.about.axis().is_some(),
        FeatureKind::Import(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Scale(_)
        | FeatureKind::Hole(_) => false,
        FeatureKind::Solid(_)
        | FeatureKind::Blend(_)
        | FeatureKind::Shell(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::Pattern(_)
        | FeatureKind::Datum(_)
        | FeatureKind::Thread(_) => true,
    }
}

fn completion_edit(document: &Document, feature: FeatureId, kind: FeatureKind) -> Option<Edit> {
    match (&document.feature(feature)?.kind, kind) {
        (FeatureKind::Sketch(current), FeatureKind::Sketch(completed)) => {
            Some(Edit::SetSketchPlacement {
                feature,
                plane: current.sketch.plane(),
                attachment: completed.attachment,
            })
        }
        (_, kind) => Some(Edit::SetFeatureKind { id: feature, kind }),
    }
}

#[derive(Default)]
struct Lacking {
    found: bool,
}

impl ReferenceVisitor for Lacking {
    fn face(&mut self, _: FeatureId, face: &mut FaceReference, _: &str) {
        self.found |= face.origin().is_none();
    }

    fn edges(&mut self, _: FeatureId, edges: &mut [EdgeReference], _: &dyn Fn(usize) -> String) {
        self.found |= edges.iter().any(lacks_origins);
    }

    fn regions(&mut self, _: FeatureId, _: &mut RegionChoice) {}
}

fn lacks_origins(edge: &EdgeReference) -> bool {
    edge.origins().iter().any(Option::is_none)
}

struct Completer<'a> {
    feature: FeatureId,
    evaluation: &'a Evaluation,
    completed: bool,
}

impl ReferenceVisitor for Completer<'_> {
    fn face(&mut self, body: FeatureId, face: &mut FaceReference, _: &str) {
        if face.origin().is_some() {
            return;
        }
        let Some(solid) = self.evaluation.body_seen_by(self.feature, body) else {
            return;
        };
        let Some(origin) = face
            .resolve(solid)
            .ok()
            .and_then(|found| solid.face(found))
            .filter(|found| found.name() == face.name())
            .and_then(|found| found.origin())
        else {
            return;
        };
        *face = FaceReference::new(face.name(), Some(origin), face.neighbours().iter().copied());
        self.completed = true;
    }

    fn edges(&mut self, body: FeatureId, edges: &mut [EdgeReference], _: &dyn Fn(usize) -> String) {
        if !edges.iter().any(lacks_origins) {
            return;
        }
        let Some(solid) = self.evaluation.body_seen_by(self.feature, body) else {
            return;
        };
        let naming = EdgeNaming::new(solid);
        for edge in edges.iter_mut().filter(|edge| lacks_origins(edge)) {
            let Some(captured) = edge
                .resolve_by_names_in(&naming)
                .ok()
                .and_then(|found| EdgeReference::capture_in(&naming, found))
                .filter(|captured| captured.faces() == edge.faces())
            else {
                continue;
            };
            if captured.origins() != edge.origins() {
                *edge = edge.with_origins(captured.origins());
                self.completed = true;
            }
        }
    }

    fn regions(&mut self, _: FeatureId, _: &mut RegionChoice) {}
}
