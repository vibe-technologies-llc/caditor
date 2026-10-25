use std::{borrow::Cow, collections::BTreeSet};

use caditor_document::{
    Document, Edit, Emboss, EmbossForm, Evaluation, Feature, FeatureId, FeatureKind, RegionChoice,
    Transaction,
};
use caditor_kernel::{FaceId, FaceReference, RegionKey, Solid};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    commands::{Command, Reason},
    editing::{self, EditingCommand},
    field::{self, CheckError},
    last_values::{Remembered, Starts},
    model::{Action, Model, Notice},
    queued::{self, Pending},
    scene,
    selection::{self, Pickable, Selection},
    solid_tools::RegionError,
};

pub const TITLE: &str = "Emboss";
pub const DEFAULT_DEPTH: f64 = 0.5;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EmbossError {
    #[error("Select the face to emboss, or a sketch drawn on it")]
    ChooseFaces,
    #[error("Select faces of one body only")]
    SeveralBodies,
    #[error("{}", Pending::BodyShape)]
    NoShape,
    #[error("Some of the selected faces are no longer part of the model")]
    FacesGone,
    #[error("Select the outlines or text of one sketch only")]
    SeveralSketches,
    #[error("The sketch is made after this feature; select one made before it")]
    SketchMadeLater { sketch: FeatureId },
    #[error("Select the sketch of outlines or text to emboss")]
    NoSketch,
    #[error("The outlines of that sketch are already embossed")]
    Already,
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error(transparent)]
    Checked(#[from] CheckError),
}

impl Reason for EmbossError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }

    fn pending(&self) -> Option<Pending> {
        matches!(self, Self::NoShape).then_some(Pending::BodyShape)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbossSource {
    pub body: FeatureId,
    pub faces: Vec<FaceKey>,
    pub attached: Option<FaceReference>,
    pub sketch: Option<FeatureId>,
    pub regions: Vec<RegionKey>,
    pub left_out: Vec<Pickable>,
}

fn chosen_sketch(model: &Model, selection: &Selection) -> Result<Option<FeatureId>, EmbossError> {
    let document = model.document();
    let mut sketches: Vec<FeatureId> = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::SketchEntity { feature, .. } | Pickable::SketchRegion { feature, .. } =
            pickable
            && !sketches.contains(&feature)
        {
            sketches.push(feature);
        }
    }
    match sketches.as_slice() {
        [] => Ok(None),
        [sketch]
            if document
                .feature_index(*sketch)
                .is_some_and(|position| position < document.bar_index()) =>
        {
            Ok(Some(*sketch))
        }
        [sketch] => Err(EmbossError::SketchMadeLater { sketch: *sketch }),
        [_, _, ..] => Err(EmbossError::SeveralSketches),
    }
}

pub fn source(model: &Model, selection: &Selection) -> Result<EmbossSource, EmbossError> {
    let mut body = None;
    let mut faces = Vec::new();
    let mut left_out = Vec::new();
    for pickable in selection.iter() {
        match pickable {
            Pickable::Face { body: owner, face } => {
                match body {
                    Some(known) if known != owner => return Err(EmbossError::SeveralBodies),
                    _ => body = Some(owner),
                }
                faces.push(face);
            }
            Pickable::SketchEntity { .. } | Pickable::SketchRegion { .. } => {}
            other => left_out.push(other),
        }
    }
    let sketch = chosen_sketch(model, selection)?;
    let regions = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::SketchRegion { region, .. } => Some(region),
            _ => None,
        })
        .collect();
    let mut attached = None;
    if faces.is_empty() {
        let attachment = sketch
            .and_then(|sketch| model.document().feature(sketch))
            .and_then(|sketch| sketch.kind.attachment())
            .and_then(|attachment| attachment.face())
            .ok_or(EmbossError::ChooseFaces)?;
        body = Some(attachment.body);
        attached = Some(attachment.face.clone());
    }
    let body = body.ok_or(EmbossError::ChooseFaces)?;
    bodies::shown(model.evaluation(), body).ok_or(EmbossError::ChooseFaces)?;
    Ok(EmbossSource {
        body,
        faces,
        attached,
        sketch,
        regions,
        left_out,
    })
}

fn chosen_regions(evaluation: &Evaluation, source: &EmbossSource) -> RegionChoice {
    let chosen: BTreeSet<RegionKey> = source.regions.iter().copied().collect();
    let Some(regions) = source
        .sketch
        .and_then(|sketch| selection::sketch_regions(evaluation, sketch))
    else {
        return RegionChoice::All;
    };
    if chosen.is_empty()
        || !regions
            .iter()
            .any(|region| chosen.contains(&region.region.key()))
    {
        return RegionChoice::All;
    }
    RegionChoice::Chosen(scene::region_references(&chosen, regions))
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    source: &EmbossSource,
    starts: &Starts,
) -> Result<(Transaction, FeatureId), EmbossError> {
    let shown = bodies::shown(evaluation, source.body).ok_or(EmbossError::NoShape)?;
    let mut faces: Vec<FaceReference> = source
        .faces
        .iter()
        .filter_map(|key| FaceReference::capture(&shown.solid, bodies::find_face(shown, *key)?))
        .collect();
    if faces.len() < source.faces.len() {
        return Err(EmbossError::FacesGone);
    }
    faces.extend(source.attached.clone());
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Emboss(Emboss {
            body: source.body,
            faces,
            sketch: source.sketch,
            regions: chosen_regions(evaluation, source),
            depth: starts.length(Remembered::EmbossDepth, DEFAULT_DEPTH),
            draft: Emboss::undrafted(),
            form: EmbossForm::Raised,
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: &EmbossSource,
    starts: &Starts,
) -> Vec<Action> {
    match create(document, evaluation, source, starts) {
        Ok((transaction, feature)) => {
            let told = body_selection::left_out_words(&source.left_out).map(|words| {
                Action::Inform(Notice::warning(format!(
                    "{TITLE} takes the faces to emboss and one sketch of outlines or text, so \
                     {words}."
                )))
            });
            [
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ]
            .into_iter()
            .chain(told)
            .collect()
        }
        Err(reason) => vec![queued::refused(Command::Emboss, TITLE, &reason)],
    }
}

pub fn edit(document: &Document, feature: FeatureId, emboss: Emboss) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Emboss(emboss),
        },
    ))
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    emboss: Emboss,
) -> Result<Transaction, EmbossError> {
    let document = model.document();
    let transaction = edit(document, feature, emboss).ok_or(EmbossError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn sketches(model: &Model, feature: FeatureId) -> Vec<FeatureId> {
    let document = model.document();
    let index = document.feature_index(feature).unwrap_or(0);
    document
        .features()
        .take(index)
        .filter(|earlier| matches!(earlier.kind, FeatureKind::Sketch(_)))
        .map(|sketch| sketch.id())
        .collect()
}

pub fn sketch_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    emboss: &Emboss,
) -> Result<Transaction, EmbossError> {
    let sketch = chosen_sketch(model, selection)?.ok_or(EmbossError::NoSketch)?;
    if emboss.sketch == Some(sketch) {
        return Err(EmbossError::Already);
    }
    change(
        model,
        feature,
        Emboss {
            sketch: Some(sketch),
            regions: RegionChoice::All,
            ..emboss.clone()
        },
    )
}

pub fn with_regions(emboss: &Emboss, regions: RegionChoice) -> Emboss {
    Emboss {
        regions,
        ..emboss.clone()
    }
}

pub fn chosen_region_keys(model: &Model, feature: FeatureId) -> Option<BTreeSet<RegionKey>> {
    let emboss = model.document().feature(feature)?.kind.emboss()?;
    let (_, regions) = selection::swept_regions(model.document(), model.evaluation(), feature)?;
    Some(scene::chosen_regions(&emboss.regions, regions))
}

pub fn toggle_region(model: &Model, feature: FeatureId, region: RegionKey) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let emboss = owner.kind.emboss()?;
    let (_, regions) = selection::swept_regions(document, model.evaluation(), feature)?;
    let mut chosen = scene::chosen_regions(&emboss.regions, regions);
    if !chosen.remove(&region) {
        chosen.insert(region);
    }
    let choice = RegionChoice::Chosen(scene::region_references(&chosen, regions));
    Some(Transaction::single(
        format!("Choose regions of {}", owner.name),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Emboss(with_regions(emboss, choice)),
        },
    ))
}

pub fn choose_every_region(model: &Model, feature: FeatureId) -> Result<Transaction, EmbossError> {
    let document = model.document();
    let gone = EmbossError::Gone { feature };
    let emboss = document
        .feature(feature)
        .and_then(|owner| owner.kind.emboss())
        .ok_or_else(|| gone.clone())?;
    let (_, regions) =
        selection::swept_regions(document, model.evaluation(), feature).ok_or(gone)?;
    let keys = scene::chosen_regions(&RegionChoice::All, regions);
    let chosen = RegionChoice::Chosen(scene::region_references(&keys, regions));
    change(model, feature, with_regions(emboss, chosen))
}

pub fn select_all_regions(model: &Model, feature: &Feature) -> Result<Transaction, RegionError> {
    let Some(emboss) = feature.kind.emboss() else {
        return Err(RegionError::TakesNoRegions {
            name: feature.name.clone(),
        });
    };
    if matches!(emboss.regions, RegionChoice::All) {
        return Err(RegionError::AlreadyEvery {
            name: feature.name.clone(),
        });
    }
    let sketch = emboss
        .sketch
        .and_then(|sketch| model.document().feature(sketch))
        .map_or("the sketch", |sketch| sketch.name.as_str());
    Ok(Transaction::single(
        format!("Select all regions of {sketch} for {}", feature.name),
        Edit::SetFeatureKind {
            id: feature.id(),
            kind: FeatureKind::Emboss(with_regions(emboss, RegionChoice::All)),
        },
    ))
}

pub fn take_sketch_regions(
    model: &Model,
    feature: &Feature,
    sketch: FeatureId,
) -> Result<Transaction, RegionError> {
    let Some(emboss) = feature.kind.emboss() else {
        return Err(RegionError::TakesNoRegions {
            name: feature.name.clone(),
        });
    };
    if emboss.sketch == Some(sketch) {
        return select_all_regions(model, feature);
    }
    let document = model.document();
    let name = document
        .feature(sketch)
        .filter(|candidate| candidate.kind.sketch().is_some())
        .map(|sketch| sketch.name.clone())
        .ok_or(RegionError::NotASketch { feature: sketch })?;
    let transaction = Transaction::single(
        format!("Take all regions of {name} for {}", feature.name),
        Edit::SetFeatureKind {
            id: feature.id(),
            kind: FeatureKind::Emboss(Emboss {
                sketch: Some(sketch),
                regions: RegionChoice::All,
                ..emboss.clone()
            }),
        },
    );
    field::checked(document, transaction)
}

pub fn clear_regions(model: &Model, feature: &Feature) -> Result<Transaction, RegionError> {
    let Some(emboss) = feature.kind.emboss() else {
        return Err(RegionError::TakesNoRegions {
            name: feature.name.clone(),
        });
    };
    let nothing_chosen =
        chosen_region_keys(model, feature.id()).is_some_and(|keys| keys.is_empty());
    if nothing_chosen {
        return Err(RegionError::NoneChosen {
            name: feature.name.clone(),
        });
    }
    Ok(Transaction::single(
        format!("Clear the chosen regions of {}", feature.name),
        Edit::SetFeatureKind {
            id: feature.id(),
            kind: FeatureKind::Emboss(with_regions(emboss, RegionChoice::Chosen(Vec::new()))),
        },
    ))
}

pub fn chosen_faces(solid: &Solid, emboss: &Emboss) -> BTreeSet<FaceKey> {
    let chosen: BTreeSet<FaceId> = emboss
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    bodies::face_keys(solid)
        .into_iter()
        .filter_map(|(id, key)| chosen.contains(&id).then_some(key))
        .collect()
}

pub fn toggle_face(model: &Model, feature: FeatureId, face: FaceKey) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let emboss = owner.kind.emboss()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_face(input, face)?;
    let solid = &input.solid;
    let mut changed = emboss.clone();
    changed.faces = emboss
        .faces
        .iter()
        .zip(emboss.resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().contains(&clicked))
        .map(|(reference, _)| reference.clone())
        .collect();
    let label = if changed.faces.len() == emboss.faces.len() {
        changed.faces.push(FaceReference::capture(solid, clicked)?);
        format!("Emboss a face with {}", owner.name)
    } else {
        format!("Leave a face out of {}", owner.name)
    };
    Some(Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Emboss(changed),
        },
    ))
}

pub fn with_selected_faces(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let emboss = owner.kind.emboss()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;
    let mut taken: BTreeSet<FaceId> = emboss
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    let mut changed = emboss.clone();
    for pickable in selection.iter() {
        let Pickable::Face { body, face } = pickable else {
            continue;
        };
        let Some(found) = (body == emboss.body)
            .then(|| bodies::find_face(input, face))
            .flatten()
            .filter(|found| !taken.contains(found))
        else {
            continue;
        };
        changed.faces.push(FaceReference::capture(solid, found)?);
        taken.insert(found);
    }
    (changed.faces.len() > emboss.faces.len()).then(|| {
        Transaction::single(
            format!("Emboss the selected faces with {}", owner.name),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::Emboss(changed),
            },
        )
    })
}
