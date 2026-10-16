use std::borrow::Cow;

use caditor_document::{
    BodyOperation, Document, Edit, Evaluation, FeatureId, FeatureKind, Loft, LoftSection,
    PointReference, SketchFeature, Transaction, TransactionBuilder,
};
use caditor_kernel::{LoftSkin, RegionKey};
use caditor_sketch::Sketch;

use crate::{
    bodies, body_selection,
    commands::{Command, Reason},
    datum_tools,
    editing::{self, EditingCommand},
    field::{self, CheckError},
    model::{Action, Model},
    projecting,
    queued::{self, Pending},
    reference_picking::{Picking, Slot},
    selection::{self, Pickable, Selection},
    sketch_placement::{self, FaceChoice},
    sweep_tools, visibility,
};

pub const TITLE: &str = "Loft";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LoftError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("Click a region or a curve of a sketch with a closed shape, or a flat face of a body")]
    NotAProfile,
    #[error(
        "Select a region or a curve of each sketch, or a flat face of a body, to loft through, \
         in order"
    )]
    NoProfileSelected,
    #[error("The selected face is no longer part of its body")]
    FaceGone,
    #[error("The selected face has no edges to outline")]
    NoFaceOutline,
    #[error("Select one corner, round edge, sphere, sketch point or datum point to end at")]
    NoPointSelected,
    #[error("The selected region is no longer part of its sketch")]
    RegionGone,
    #[error("It already lofts through the selected profiles")]
    AlreadyThrough,
    #[error("It already ends at that point")]
    AlreadyEnds,
    #[error(transparent)]
    FacePlacement(crate::sketch_placement::AttachmentError),
    #[error(transparent)]
    Check(#[from] CheckError),
}

impl Reason for LoftError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }

    fn pending(&self) -> Option<Pending> {
        match self {
            Self::FacePlacement(error) => error.pending(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfilePick {
    pub sketch: FeatureId,
    pub region: Option<RegionKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Sketch(ProfilePick),
    Face(FaceChoice),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoftStart {
    pub profiles: Vec<Pick>,
    pub end: Option<PointReference>,
    pub body: Option<FeatureId>,
}

impl LoftStart {
    fn rows(&self) -> usize {
        self.profiles.len() + usize::from(self.end.is_some())
    }
}

fn has_regions(evaluation: &Evaluation, sketch: FeatureId) -> bool {
    selection::sketch_regions(evaluation, sketch).is_some_and(|regions| !regions.is_empty())
}

fn pick_of(model: &Model, pickable: Pickable) -> Option<Pick> {
    match FaceChoice::of(pickable) {
        Some(face) if sketch_placement::is_flat(model, face) => Some(Pick::Face(face)),
        Some(_) => None,
        None => profile_of(model, pickable).map(Pick::Sketch),
    }
}

fn picks(model: &Model, selection: &Selection) -> Vec<Pick> {
    let mut picks: Vec<Pick> = Vec::new();
    for pickable in selection.in_pick_order() {
        let Some(pick) = pick_of(model, pickable) else {
            continue;
        };
        let known = picks.iter().any(|known| match (known, pick) {
            (Pick::Sketch(known), Pick::Sketch(pick)) => known.sketch == pick.sketch,
            (Pick::Face(known), Pick::Face(pick)) => *known == pick,
            (Pick::Sketch(_), Pick::Face(_)) | (Pick::Face(_), Pick::Sketch(_)) => false,
        });
        if !known {
            picks.push(pick);
        }
    }
    picks
}

fn face_sketch(
    model: &Model,
    transaction: &mut TransactionBuilder<'_>,
    (face, index): (FaceChoice, usize),
    name: String,
) -> Result<FeatureId, LoftError> {
    let (attachment, plane) =
        sketch_placement::attachment_at(model, face, index).map_err(LoftError::FacePlacement)?;
    let shown = bodies::shown(model.evaluation(), face.body).ok_or(LoftError::FaceGone)?;
    let found = bodies::find_face(shown, face.face).ok_or(LoftError::FaceGone)?;
    let outline = projecting::face_projections(shown, face.body, &[found], &plane);
    if outline.is_empty() {
        return Err(LoftError::NoFaceOutline);
    }
    let sketch = transaction.add_feature(
        name,
        FeatureKind::Sketch(SketchFeature::on_face(Sketch::new(plane), attachment)),
    );
    transaction.edit(Edit::SetFeatureHidden {
        id: sketch,
        hidden: true,
    });
    for (source, edges) in &outline {
        transaction.add_projection(sketch, source.clone(), edges);
    }
    Ok(sketch)
}

fn face_sketch_name(document: &Document, loft: &str, numbered: &mut usize) -> String {
    loop {
        *numbered += 1;
        let name = format!("{loft} profile {numbered}");
        if document.features().all(|feature| feature.name != name) {
            return name;
        }
    }
}

fn profile_of(model: &Model, pickable: Pickable) -> Option<ProfilePick> {
    match pickable {
        Pickable::SketchRegion { feature, region } => Some(ProfilePick {
            sketch: feature,
            region: Some(region),
        }),
        Pickable::SketchEntity { feature, entity }
            if sweep_tools::is_path_curve(model.document(), feature, entity)
                && has_regions(model.evaluation(), feature) =>
        {
            Some(ProfilePick {
                sketch: feature,
                region: None,
            })
        }
        _ => None,
    }
}

fn profile_picks(model: &Model, selection: &Selection) -> Vec<ProfilePick> {
    let mut picks: Vec<ProfilePick> = Vec::new();
    for pickable in selection.in_pick_order() {
        let Some(pick) = profile_of(model, pickable) else {
            continue;
        };
        if !picks.iter().any(|known| known.sketch == pick.sketch) {
            picks.push(pick);
        }
    }
    picks
}

pub fn source(model: &Model, selection: &Selection) -> Result<LoftStart, LoftError> {
    let document = model.document();
    let index = document.bar_index();
    let profiles = picks(model, selection);
    let end = selection
        .in_pick_order()
        .into_iter()
        .filter(|pickable| pick_of(model, *pickable).is_none())
        .find_map(|pickable| datum_tools::point_reference(model, pickable, index));
    let body = match body_selection::bodies_in(selection).as_slice() {
        [only] => Some(*only),
        _ => None,
    };
    Ok(LoftStart {
        profiles,
        end,
        body,
    })
}

fn captured(evaluation: &Evaluation, pick: ProfilePick) -> Result<LoftSection, LoftError> {
    let Some(key) = pick.region else {
        return Ok(LoftSection::whole(pick.sketch));
    };
    let region = selection::sketch_regions(evaluation, pick.sketch)
        .and_then(|regions| regions.iter().find(|region| region.region.key() == key))
        .ok_or(LoftError::RegionGone)?;
    Ok(LoftSection {
        sketch: pick.sketch,
        region: Some(region.reference()),
    })
}

pub fn create(model: &Model, start: &LoftStart) -> Result<(Transaction, FeatureId), LoftError> {
    let document = model.document();
    let index = document.bar_index();
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let mut profiles = Vec::with_capacity(start.profiles.len());
    let mut faces = 0;
    for pick in &start.profiles {
        profiles.push(match pick {
            Pick::Sketch(pick) => captured(model.evaluation(), *pick)?,
            Pick::Face(face) => {
                let named = face_sketch_name(document, &name, &mut faces);
                LoftSection::whole(face_sketch(model, &mut transaction, (*face, index), named)?)
            }
        });
    }
    let loft = Loft {
        end: start.end.clone(),
        operation: start
            .body
            .map_or(BodyOperation::NewBody, BodyOperation::Add),
        ..Loft::new(profiles)
    };
    let feature = transaction.add_feature(name, FeatureKind::Loft(loft));
    if start.rows() >= 2 {
        for pick in &start.profiles {
            if let Pick::Sketch(pick) = pick
                && visibility::is_shown(document, pick.sketch)
            {
                transaction.edit(Edit::SetFeatureHidden {
                    id: pick.sketch,
                    hidden: true,
                });
            }
        }
    }
    Ok((transaction.finish(), feature))
}

pub fn create_actions(model: &Model, start: &LoftStart) -> Vec<Action> {
    match create(model, start) {
        Ok((transaction, feature)) => {
            let mut actions = vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ];
            if start.rows() < 2 {
                actions.push(Action::Editing(EditingCommand::Pick(Picking::new(
                    feature,
                    Slot::LoftProfiles,
                ))));
            }
            actions
        }
        Err(reason) => vec![queued::refused(Command::Loft, TITLE, &reason)],
    }
}

pub fn edit(document: &Document, feature: FeatureId, loft: Loft) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Loft(loft),
        },
    ))
}

pub fn change(model: &Model, feature: FeatureId, loft: Loft) -> Result<Transaction, LoftError> {
    let document = model.document();
    let transaction = edit(document, feature, loft).ok_or(LoftError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn with_operation(loft: &Loft, operation: BodyOperation) -> Loft {
    Loft {
        operation,
        ..loft.clone()
    }
}

pub fn with_skin(loft: &Loft, skin: LoftSkin) -> Loft {
    let ruled = skin == LoftSkin::Ruled;
    Loft {
        skin,
        square_start: if ruled {
            None
        } else {
            loft.square_start.clone()
        },
        square_end: if ruled { None } else { loft.square_end.clone() },
        ..loft.clone()
    }
}

pub fn with_closing(loft: &Loft, closed: bool) -> Loft {
    Loft {
        closed,
        end: if closed { None } else { loft.end.clone() },
        square_start: if closed {
            None
        } else {
            loft.square_start.clone()
        },
        square_end: if closed {
            None
        } else {
            loft.square_end.clone()
        },
        ..loft.clone()
    }
}

pub fn without_end(loft: &Loft) -> Loft {
    Loft {
        end: None,
        ..loft.clone()
    }
}

pub fn without_profile(loft: &Loft, index: usize) -> Loft {
    let mut profiles = loft.profiles.clone();
    if index < profiles.len() {
        profiles.remove(index);
    }
    Loft {
        profiles,
        ..loft.clone()
    }
}

pub fn moved(loft: &Loft, index: usize, earlier: bool) -> Option<Loft> {
    let other = if earlier {
        index.checked_sub(1)?
    } else {
        index + 1
    };
    if other >= loft.profiles.len() || index >= loft.profiles.len() {
        return None;
    }
    let mut profiles = loft.profiles.clone();
    profiles.swap(index, other);
    Some(Loft {
        profiles,
        ..loft.clone()
    })
}

fn is_pick(section: &LoftSection, pick: ProfilePick) -> bool {
    section.sketch == pick.sketch
        && match (pick.region, &section.region) {
            (None, _) => true,
            (Some(key), Some(reference)) => reference.key() == key,
            (Some(_), None) => false,
        }
}

pub fn profile_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    loft: &Loft,
) -> Result<Transaction, LoftError> {
    let chosen = picks(model, selection);
    let pick = match chosen.as_slice() {
        [Pick::Sketch(pick)] => pick,
        [Pick::Face(face)] => {
            return with_picks(
                model,
                feature,
                loft,
                (loft.profiles.clone(), &[Pick::Face(*face)]),
            );
        }
        _ => return Err(LoftError::NotAProfile),
    };
    let mut profiles = loft.profiles.clone();
    match profiles
        .iter()
        .position(|section| section.sketch == pick.sketch)
    {
        Some(index)
            if profiles
                .get(index)
                .is_some_and(|known| is_pick(known, *pick)) =>
        {
            profiles.remove(index);
        }
        Some(index) => {
            let section = captured(model.evaluation(), *pick)?;
            if let Some(slot) = profiles.get_mut(index) {
                *slot = section;
            }
        }
        None => profiles.push(captured(model.evaluation(), *pick)?),
    }
    change(
        model,
        feature,
        Loft {
            profiles,
            ..loft.clone()
        },
    )
}

pub fn selected_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    loft: &Loft,
) -> Result<Transaction, LoftError> {
    let chosen = picks(model, selection);
    if chosen.is_empty() {
        return Err(LoftError::NoProfileSelected);
    }
    if chosen.iter().any(|pick| matches!(pick, Pick::Face(_))) {
        return with_picks(model, feature, loft, (Vec::new(), &chosen));
    }
    let picks = profile_picks(model, selection);
    let profiles = picks
        .iter()
        .map(|pick| captured(model.evaluation(), *pick))
        .collect::<Result<Vec<_>, _>>()?;
    let unchanged = profiles.len() == loft.profiles.len()
        && picks.iter().zip(&loft.profiles).all(|(pick, known)| {
            is_pick(known, *pick) && pick.region.is_some() == known.region.is_some()
        });
    if unchanged {
        return Err(LoftError::AlreadyThrough);
    }
    change(
        model,
        feature,
        Loft {
            profiles,
            ..loft.clone()
        },
    )
}

pub fn end_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    loft: &Loft,
) -> Result<Transaction, LoftError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(LoftError::Gone { feature })?;
    let mut picked = selection.iter();
    let end = match (picked.next(), picked.next()) {
        (Some(only), None) => datum_tools::point_reference(model, only, index),
        _ => None,
    }
    .ok_or(LoftError::NoPointSelected)?;
    if loft.end.as_ref() == Some(&end) {
        return Err(LoftError::AlreadyEnds);
    }
    change(
        model,
        feature,
        Loft {
            end: Some(end),
            closed: false,
            square_end: None,
            ..loft.clone()
        },
    )
}

pub fn profile_words(document: &Document, section: &LoftSection) -> Option<String> {
    let name = &document.feature(section.sketch)?.name;
    Some(match section.region {
        Some(_) => format!("A region of {name}"),
        None => name.clone(),
    })
}

fn with_picks(
    model: &Model,
    feature: FeatureId,
    loft: &Loft,
    (mut profiles, chosen): (Vec<LoftSection>, &[Pick]),
) -> Result<Transaction, LoftError> {
    let document = model.document();
    let index = document
        .feature_index(feature)
        .ok_or(LoftError::Gone { feature })?;
    let name = document
        .feature(feature)
        .ok_or(LoftError::Gone { feature })?
        .name
        .clone();
    let mut transaction = document
        .transaction(format!("Edit {name}"))
        .inserting_at(index);
    let mut faces = 0;
    for pick in chosen {
        profiles.push(match pick {
            Pick::Sketch(pick) => captured(model.evaluation(), *pick)?,
            Pick::Face(face) => {
                let named = face_sketch_name(document, &name, &mut faces);
                LoftSection::whole(face_sketch(model, &mut transaction, (*face, index), named)?)
            }
        });
    }
    transaction.edit(Edit::SetFeatureKind {
        id: feature,
        kind: FeatureKind::Loft(Loft {
            profiles,
            ..loft.clone()
        }),
    });
    field::checked(document, transaction.finish())
}
