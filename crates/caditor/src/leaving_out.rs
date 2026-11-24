use caditor_document::{FailingPick, Feature, FeatureKind};
use caditor_kernel::{EdgeReference, FaceReference};

use crate::{
    blend_panel, blend_tools, delete_face_panel, draft_panel, feature_fields, field,
    model::{Action, Model},
    move_face_panel, offset_face_panel, replace_face_panel, shell_panel,
};

pub const LEAVE_EDGE_OUT: &str = "Leave this edge out";
pub const LEAVE_FACE_OUT: &str = "Leave this face out";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveTarget {
    Index(usize),
    BlendEdge(EdgeReference),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListedPick {
    pub target: LeaveTarget,
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
            target: LeaveTarget::Index(index),
            label: LEAVE_FACE_OUT,
        })
}

pub fn listed(model: &Model, feature: &Feature, pick: &FailingPick) -> Option<ListedPick> {
    match (&feature.kind, pick) {
        (FeatureKind::Blend(blend), FailingPick::Edge(edge)) => blend
            .edges
            .iter()
            .position(|listed| listed == edge)
            .map(|index| ListedPick {
                target: LeaveTarget::Index(index),
                label: LEAVE_EDGE_OUT,
            })
            .or_else(|| {
                blend_tools::grouped_edge(model, feature.id(), blend, edge).then_some(ListedPick {
                    target: LeaveTarget::BlendEdge(*edge),
                    label: LEAVE_EDGE_OUT,
                })
            }),
        (FeatureKind::Shell(shell), FailingPick::Face(face)) => face_at(
            shell
                .open
                .iter()
                .chain(shell.faces.iter().map(|own| &own.face)),
            face,
        ),
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

pub fn leave_out(model: &Model, feature: &Feature, target: LeaveTarget) -> Option<Action> {
    match (&feature.kind, target) {
        (FeatureKind::Blend(blend), LeaveTarget::Index(index)) => {
            Some(blend_panel::leave_out(model, feature, blend, index))
        }
        (FeatureKind::Blend(_), LeaveTarget::BlendEdge(edge)) => {
            let transaction = blend_tools::toggle_edge(model, feature.id(), edge.name())?;
            Some(feature_fields::applied(
                &feature.name,
                field::checked::<field::CheckError>(model.document(), transaction),
            ))
        }
        (FeatureKind::Shell(shell), LeaveTarget::Index(index)) => {
            Some(shell_panel::leave_out(model, feature, shell, index))
        }
        (FeatureKind::OffsetFace(offset), LeaveTarget::Index(index)) => {
            Some(offset_face_panel::leave_out(model, feature, offset, index))
        }
        (FeatureKind::MoveFace(moving), LeaveTarget::Index(index)) => {
            Some(move_face_panel::leave_out(model, feature, moving, index))
        }
        (FeatureKind::ReplaceFace(replace), LeaveTarget::Index(index)) => Some(
            replace_face_panel::leave_out(model, feature, replace, index),
        ),
        (FeatureKind::DeleteFace(deletion), LeaveTarget::Index(index)) => Some(
            delete_face_panel::leave_out(model, feature, deletion, index),
        ),
        (FeatureKind::Draft(draft), LeaveTarget::Index(index)) => {
            Some(draft_panel::leave_out(model, feature, draft, index))
        }
        _ => None,
    }
}
