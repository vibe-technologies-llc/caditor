use caditor_document::{FailingPick, Feature, FeatureKind};
use caditor_kernel::FaceReference;

use crate::{
    blend_panel, delete_face_panel, draft_panel,
    model::{Action, Model},
    move_face_panel, offset_face_panel, replace_face_panel, shell_panel,
};

pub const LEAVE_EDGE_OUT: &str = "Leave this edge out";
pub const LEAVE_FACE_OUT: &str = "Leave this face out";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListedPick {
    pub index: usize,
    pub label: &'static str,
}

fn face_at<'a>(
    faces: impl IntoIterator<Item = &'a FaceReference>,
    face: &FaceReference,
) -> Option<ListedPick> {
    faces
        .into_iter()
        .position(|listed| listed == face)
        .map(|index| ListedPick {
            index,
            label: LEAVE_FACE_OUT,
        })
}

pub fn listed(feature: &Feature, pick: &FailingPick) -> Option<ListedPick> {
    match (&feature.kind, pick) {
        (FeatureKind::Blend(blend), FailingPick::Edge(edge)) => blend
            .edges
            .iter()
            .position(|listed| listed == edge)
            .map(|index| ListedPick {
                index,
                label: LEAVE_EDGE_OUT,
            }),
        (FeatureKind::Shell(shell), FailingPick::Face(face)) => face_at(&shell.open, face),
        (FeatureKind::OffsetFace(offset), FailingPick::Face(face)) => face_at(&offset.faces, face),
        (FeatureKind::MoveFace(moving), FailingPick::Face(face)) => face_at(&moving.faces, face),
        (FeatureKind::ReplaceFace(replace), FailingPick::Face(face)) => {
            face_at(&replace.faces, face)
        }
        (FeatureKind::DeleteFace(deletion), FailingPick::Face(face)) => {
            face_at(&deletion.faces, face)
        }
        (FeatureKind::Draft(draft), FailingPick::Face(face)) => face_at(
            draft
                .faces
                .iter()
                .chain(draft.own.iter().map(|own| &own.face)),
            face,
        ),
        _ => None,
    }
}

pub fn leave_out(model: &Model, feature: &Feature, index: usize) -> Option<Action> {
    match &feature.kind {
        FeatureKind::Blend(blend) => Some(blend_panel::leave_out(model, feature, blend, index)),
        FeatureKind::Shell(shell) => Some(shell_panel::leave_out(model, feature, shell, index)),
        FeatureKind::OffsetFace(offset) => {
            Some(offset_face_panel::leave_out(model, feature, offset, index))
        }
        FeatureKind::MoveFace(moving) => {
            Some(move_face_panel::leave_out(model, feature, moving, index))
        }
        FeatureKind::ReplaceFace(replace) => Some(replace_face_panel::leave_out(
            model, feature, replace, index,
        )),
        FeatureKind::DeleteFace(deletion) => Some(delete_face_panel::leave_out(
            model, feature, deletion, index,
        )),
        FeatureKind::Draft(draft) => Some(draft_panel::leave_out(model, feature, draft, index)),
        _ => None,
    }
}
