use std::collections::BTreeSet;

use caditor_document::{
    Document, Edit, Feature, FeatureId, FeatureKind, PrincipalGeometry, Transaction,
};

use crate::selection::{Axis, Pickable, Selection};

const NOTHING_TO_HIDE: &str =
    "Select a body, sketch, datum, principal plane or axis in the view first";
const NOTHING_HIDDEN: &str = "Nothing is hidden";
const NOT_HIDEABLE: &str = "Only sketches, datums and features that make a body can be hidden";
const PRINCIPAL_GROUP: &str = "principal planes, axes and origin";

pub fn can_hide(feature: &Feature) -> bool {
    match &feature.kind {
        FeatureKind::Sketch(_) | FeatureKind::Datum(_) => true,
        FeatureKind::Solid(_)
        | FeatureKind::Import(_)
        | FeatureKind::Blend(_)
        | FeatureKind::Shell(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Pattern(_) => feature.makes_body(),
    }
}

pub fn is_shown(document: &Document, feature: FeatureId) -> bool {
    document
        .feature(feature)
        .is_some_and(|feature| !feature.hidden)
}

pub fn is_principal_shown(document: &Document, geometry: PrincipalGeometry) -> bool {
    !document.is_principal_hidden(geometry)
}

pub fn owner(pickable: Pickable) -> Option<FeatureId> {
    match pickable {
        Pickable::SketchEntity { feature, .. } | Pickable::Datum(feature) => Some(feature),
        Pickable::Face { body, .. }
        | Pickable::Edge { body, .. }
        | Pickable::Vertex { body, .. } => Some(body),
        Pickable::Origin
        | Pickable::Axis(_)
        | Pickable::Plane(_)
        | Pickable::SketchConstraint { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. } => None,
    }
}

pub fn principal(pickable: Pickable) -> Option<PrincipalGeometry> {
    match pickable {
        Pickable::Origin => Some(PrincipalGeometry::Origin),
        Pickable::Axis(axis) => Some(PrincipalGeometry::Axis(axis.principal())),
        Pickable::Plane(plane) => Some(PrincipalGeometry::Plane(plane)),
        Pickable::SketchEntity { .. }
        | Pickable::Datum(_)
        | Pickable::Face { .. }
        | Pickable::Edge { .. }
        | Pickable::Vertex { .. }
        | Pickable::SketchConstraint { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. } => None,
    }
}

pub fn pickable(geometry: PrincipalGeometry) -> Pickable {
    match geometry {
        PrincipalGeometry::Origin => Pickable::Origin,
        PrincipalGeometry::Axis(axis) => Pickable::Axis(Axis::of(axis)),
        PrincipalGeometry::Plane(plane) => Pickable::Plane(plane),
    }
}

fn set_hidden(id: FeatureId, hidden: bool) -> Edit {
    Edit::SetFeatureHidden { id, hidden }
}

fn set_principal_hidden(geometry: PrincipalGeometry, hidden: bool) -> Edit {
    Edit::SetPrincipalHidden { geometry, hidden }
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

pub fn toggle_principal(document: &Document, geometry: PrincipalGeometry) -> Transaction {
    let shown = is_principal_shown(document, geometry);
    let verb = if shown { "Hide" } else { "Show" };
    Transaction::single(
        format!("{verb} {}", geometry.name()),
        set_principal_hidden(geometry, shown),
    )
}

pub fn any_principal_shown(document: &Document) -> bool {
    PrincipalGeometry::ALL
        .into_iter()
        .any(|geometry| is_principal_shown(document, geometry))
}

pub fn toggle_principal_group(document: &Document) -> Transaction {
    let hide = any_principal_shown(document);
    let verb = if hide { "Hide" } else { "Show" };
    Transaction::new(
        format!("{verb} {PRINCIPAL_GROUP}"),
        PrincipalGeometry::ALL
            .into_iter()
            .filter(|geometry| is_principal_shown(document, *geometry) == hide)
            .map(|geometry| set_principal_hidden(geometry, hide))
            .collect(),
    )
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
    let principal: BTreeSet<PrincipalGeometry> = selection
        .iter()
        .filter_map(principal)
        .filter(|geometry| is_principal_shown(document, *geometry))
        .collect();
    let label = match (hiding.as_slice(), principal.len()) {
        ([], 0) => return Err(NOTHING_TO_HIDE.to_owned()),
        ([only], 0) => format!("Hide {}", only.name),
        ([], 1) => format!(
            "Hide {}",
            principal.first().map_or("", |geometry| geometry.name())
        ),
        (many, 0) => format!("Hide {} features", many.len()),
        (features, principal) => format!("Hide {} items", features.len() + principal),
    };
    Ok(Transaction::new(
        label,
        hiding
            .iter()
            .map(|feature| set_hidden(feature.id(), true))
            .chain(
                principal
                    .into_iter()
                    .map(|geometry| set_principal_hidden(geometry, true)),
            )
            .collect(),
    ))
}

pub fn show_all(document: &Document) -> Result<Transaction, String> {
    let edits: Vec<Edit> = document
        .features()
        .filter(|feature| feature.hidden)
        .map(|feature| set_hidden(feature.id(), false))
        .chain(
            document
                .hidden_principal()
                .map(|geometry| set_principal_hidden(geometry, false)),
        )
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

        let xy = PrincipalGeometry::Plane(crate::selection::PrincipalPlane::Xy);

        let hide = hide_selection(&document, &selection, None).unwrap();
        document.apply(hide.clone()).unwrap();
        let again = hide_selection(&document, &selection, None);
        let hidden_xy = document.is_principal_hidden(xy);
        let show = show_all(&document).unwrap();
        document.apply(show).unwrap();

        assert_eq!(hide.label(), "Hide 3 items");
        assert_eq!(hide.edits().len(), 3);
        assert_eq!(again, Err(NOTHING_TO_HIDE.to_owned()));
        assert!(hidden_xy);
        assert!(is_shown(&document, first) && is_shown(&document, second));
        assert!(is_principal_shown(&document, xy));
        assert_eq!(show_all(&document), Err(NOTHING_HIDDEN.to_owned()));
        assert_eq!(
            toggle(document.feature(first).unwrap()).map(|toggle| toggle.label().to_owned()),
            Ok("Hide First".to_owned())
        );
    }

    #[test]
    fn the_principal_group_hides_what_is_shown_and_then_shows_everything_again() {
        let mut document = Document::default();
        let origin = PrincipalGeometry::Origin;
        document.apply(toggle_principal(&document, origin)).unwrap();

        let hide_rest = toggle_principal_group(&document);
        document.apply(hide_rest.clone()).unwrap();
        let all_hidden = !any_principal_shown(&document);
        let show = toggle_principal_group(&document);
        document.apply(show.clone()).unwrap();

        assert_eq!(hide_rest.label(), "Hide principal planes, axes and origin");
        assert_eq!(hide_rest.edits().len(), 6);
        assert!(all_hidden);
        assert_eq!(show.label(), "Show principal planes, axes and origin");
        assert_eq!(show.edits().len(), 7);
        assert!(
            PrincipalGeometry::ALL
                .into_iter()
                .all(|geometry| is_principal_shown(&document, geometry))
        );
        assert_eq!(pickable(origin), Pickable::Origin);
        assert!(
            PrincipalGeometry::ALL
                .into_iter()
                .all(|geometry| principal(pickable(geometry)) == Some(geometry))
        );
    }
}
