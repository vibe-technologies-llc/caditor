use std::collections::BTreeSet;

use caditor_document::{Document, Edit, Feature, FeatureId, FeatureKind, Transaction};

use crate::selection::{Pickable, Selection};

const NOTHING_TO_HIDE: &str = "Select a body, sketch or datum in the view first";
const NOTHING_HIDDEN: &str = "Nothing is hidden";
const NOT_HIDEABLE: &str = "Only sketches, datums and features that make a body can be hidden";

pub fn can_hide(feature: &Feature) -> bool {
    match &feature.kind {
        FeatureKind::Sketch(_) | FeatureKind::Datum(_) => true,
        FeatureKind::Solid(_)
        | FeatureKind::Import(_)
        | FeatureKind::Blend(_)
        | FeatureKind::Shell(_) => feature.makes_body(),
    }
}

pub fn is_shown(document: &Document, feature: FeatureId) -> bool {
    document
        .feature(feature)
        .is_some_and(|feature| !feature.hidden)
}

pub fn owner(pickable: Pickable) -> Option<FeatureId> {
    match pickable {
        Pickable::SketchEntity { feature, .. } | Pickable::Datum(feature) => Some(feature),
        Pickable::Face { body, .. } | Pickable::Edge { body, .. } => Some(body),
        Pickable::Origin
        | Pickable::Axis(_)
        | Pickable::Plane(_)
        | Pickable::SketchConstraint { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. } => None,
    }
}

fn set_hidden(id: FeatureId, hidden: bool) -> Edit {
    Edit::SetFeatureHidden { id, hidden }
}

pub fn toggle(feature: &Feature) -> Result<Transaction, String> {
    if !can_hide(feature) {
        return Err(NOT_HIDEABLE.to_owned());
    }
    let verb = if feature.hidden { "Show" } else { "Hide" };
    Ok(Transaction::single(
        format!("{verb} {}", feature.name),
        set_hidden(feature.id(), !feature.hidden),
    ))
}

pub fn hide_selection(
    document: &Document,
    selection: &Selection,
    edited: Option<FeatureId>,
) -> Result<Transaction, String> {
    let owners: BTreeSet<FeatureId> = selection
        .iter()
        .filter_map(owner)
        .filter(|owner| Some(*owner) != edited)
        .collect();
    let hiding: Vec<&Feature> = owners
        .into_iter()
        .filter_map(|id| document.feature(id))
        .filter(|feature| !feature.hidden && can_hide(feature))
        .collect();
    let label = match hiding.as_slice() {
        [] => return Err(NOTHING_TO_HIDE.to_owned()),
        [only] => format!("Hide {}", only.name),
        many => format!("Hide {} features", many.len()),
    };
    Ok(Transaction::new(
        label,
        hiding
            .iter()
            .map(|feature| set_hidden(feature.id(), true))
            .collect(),
    ))
}

pub fn show_all(document: &Document) -> Result<Transaction, String> {
    let edits: Vec<Edit> = document
        .features()
        .filter(|feature| feature.hidden)
        .map(|feature| set_hidden(feature.id(), false))
        .collect();
    if edits.is_empty() {
        return Err(NOTHING_HIDDEN.to_owned());
    }
    Ok(Transaction::new("Show everything", edits))
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;
    use caditor_sketch::Sketch;

    use super::*;

    #[test]
    fn hiding_the_selection_hides_each_owner_once_and_show_all_reverses_it() {
        let mut document = Document::default();
        let mut transaction = document.transaction("Sketches");
        let first = transaction.add_feature("First", FeatureKind::from(Sketch::new(Plane::XY)));
        let second = transaction.add_feature("Second", FeatureKind::from(Sketch::new(Plane::XZ)));
        document.apply(transaction.finish()).unwrap();
        let mut selection = Selection::default();
        for pickable in [
            Pickable::SketchEntity {
                feature: first,
                entity: caditor_sketch::EntityId::ORIGIN,
            },
            Pickable::SketchEntity {
                feature: second,
                entity: caditor_sketch::EntityId::ORIGIN,
            },
            Pickable::Plane(crate::selection::PrincipalPlane::Xy),
        ] {
            selection.toggle(pickable);
        }

        let hide = hide_selection(&document, &selection, None).unwrap();
        document.apply(hide.clone()).unwrap();
        let again = hide_selection(&document, &selection, None);
        let show = show_all(&document).unwrap();
        document.apply(show).unwrap();

        assert_eq!(hide.label(), "Hide 2 features");
        assert_eq!(hide.edits().len(), 2);
        assert_eq!(again, Err(NOTHING_TO_HIDE.to_owned()));
        assert!(is_shown(&document, first) && is_shown(&document, second));
        assert_eq!(show_all(&document), Err(NOTHING_HIDDEN.to_owned()));
        assert_eq!(
            toggle(document.feature(first).unwrap()).map(|toggle| toggle.label().to_owned()),
            Ok("Hide First".to_owned())
        );
    }
}
