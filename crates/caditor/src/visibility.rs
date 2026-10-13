use std::collections::BTreeSet;

use caditor_document::{
    Document, Edit, Feature, FeatureId, FeatureKind, PrincipalGeometry, Transaction,
};
use egui::Id;

use crate::{
    editing::SketchEditing,
    model::Action,
    selection::{Axis, Pickable, Selection},
};

const NOTHING_TO_HIDE: &str =
    "Select a body, sketch, datum, principal plane or axis in the view or the tree first";
const NOTHING_TO_ISOLATE: &str = "Select what to keep in the view or the tree first";
const ALREADY_ISOLATED: &str = "Everything else is already hidden";
const NOTHING_HIDDEN: &str = "Nothing is hidden";
const NOT_HIDEABLE: &str = "Only sketches, datums and features that make a body can be hidden";
const PRINCIPAL_GROUP: &str = "principal planes, axes and origin";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sketches,
    Datums,
    Bodies,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Sketches, Self::Datums, Self::Bodies];

    fn holds(self, feature: &Feature) -> bool {
        match &feature.kind {
            FeatureKind::Sketch(_) => self == Self::Sketches,
            FeatureKind::Datum(_) | FeatureKind::ReferenceImage(_) => self == Self::Datums,
            _ => self == Self::Bodies && can_hide(feature),
        }
    }

    fn singular(self) -> &'static str {
        match self {
            Self::Sketches => "sketch",
            Self::Datums => "datum",
            Self::Bodies => "body",
        }
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Sketches => "sketches",
            Self::Datums => "datums",
            Self::Bodies => "bodies",
        }
    }
}

pub fn can_hide(feature: &Feature) -> bool {
    match &feature.kind {
        FeatureKind::Sketch(_)
        | FeatureKind::Datum(_)
        | FeatureKind::Thread(_)
        | FeatureKind::Measurement(_)
        | FeatureKind::ReferenceImage(_) => true,
        FeatureKind::Solid(_)
        | FeatureKind::Import(_)
        | FeatureKind::Blend(_)
        | FeatureKind::Shell(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::SplitFace(_)
        | FeatureKind::Primitive(_)
        | FeatureKind::Pipe(_)
        | FeatureKind::Loft(_)
        | FeatureKind::Coil(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Move(_)
        | FeatureKind::Mate(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Split(_)
        | FeatureKind::Scale(_)
        | FeatureKind::Hole(_)
        | FeatureKind::Pattern(_)
        | FeatureKind::Remove(_) => feature.makes_body(),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrincipalSet(u8);

impl PrincipalSet {
    fn bit(geometry: PrincipalGeometry) -> u8 {
        PrincipalGeometry::ALL
            .iter()
            .position(|listed| *listed == geometry)
            .and_then(|index| u32::try_from(index).ok())
            .and_then(|index| 1_u8.checked_shl(index))
            .unwrap_or_default()
    }

    pub fn holds(self, geometry: PrincipalGeometry) -> bool {
        self.0 & Self::bit(geometry) != 0
    }

    pub fn with(self, geometries: impl IntoIterator<Item = PrincipalGeometry>) -> Self {
        Self(
            geometries
                .into_iter()
                .fold(self.0, |kept, geometry| kept | Self::bit(geometry)),
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrincipalView {
    pub tucked_away: bool,
    pub kept: PrincipalSet,
    pub selected: PrincipalSet,
}

impl PrincipalView {
    pub fn shows(self, document: &Document, geometry: PrincipalGeometry) -> bool {
        is_principal_shown(document, geometry) && !self.tucks(geometry)
    }

    pub fn tucks(self, geometry: PrincipalGeometry) -> bool {
        self.tucked_away && !self.kept.holds(geometry) && !self.selected.holds(geometry)
    }
}

pub struct Needs<'a> {
    pub editing: &'a SketchEditing,
    pub measuring: bool,
    pub listing: bool,
}

pub fn selected_principal(selection: &Selection) -> PrincipalSet {
    PrincipalSet::default().with(selection.iter().filter_map(principal))
}

pub fn has_body(document: &Document) -> bool {
    document.active_features().any(Feature::makes_body)
}

pub fn principal_needed(document: &Document, needs: &Needs<'_>) -> bool {
    let editing = needs.editing;
    let datum_open = editing
        .solid()
        .and_then(|id| document.feature(id))
        .is_some_and(|feature| feature.kind.datum().is_some());
    needs.measuring
        || needs.listing
        || datum_open
        || editing.is_choosing_plane()
        || editing.context().intersecting
        || editing.context().projecting
        || editing
            .picking()
            .is_some_and(|picking| picking.slot.takes_principal_geometry())
}

pub fn tucked_away(document: &Document, when_needed: bool, needs: &Needs<'_>) -> bool {
    when_needed && has_body(document) && !principal_needed(document, needs)
}

fn listing_key() -> Id {
    Id::new("principal-listing")
}

pub fn listing_principal(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(listing_key(), true));
}

pub fn take_listing(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| {
        let listed = data.get_temp::<bool>(listing_key()).unwrap_or(false);
        data.remove::<bool>(listing_key());
        listed
    })
}

pub fn owner(pickable: Pickable) -> Option<FeatureId> {
    match pickable {
        Pickable::SketchEntity { feature, .. }
        | Pickable::SketchRegion { feature, .. }
        | Pickable::Datum(feature)
        | Pickable::FrameAxis { feature, .. }
        | Pickable::FramePlane { feature, .. } => Some(feature),
        Pickable::Face { body, .. }
        | Pickable::Edge { body, .. }
        | Pickable::Vertex { body, .. }
        | Pickable::CentreOfMass(body) => Some(body),
        Pickable::Origin
        | Pickable::Axis(_)
        | Pickable::Plane(_)
        | Pickable::SketchConstraint { .. }
        | Pickable::FeatureValue { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. }
        | Pickable::BodyItem { .. } => None,
    }
}

pub fn principal(pickable: Pickable) -> Option<PrincipalGeometry> {
    match pickable {
        Pickable::Origin => Some(PrincipalGeometry::Origin),
        Pickable::Axis(axis) => Some(PrincipalGeometry::Axis(axis.principal())),
        Pickable::Plane(plane) => Some(PrincipalGeometry::Plane(plane)),
        Pickable::SketchEntity { .. }
        | Pickable::Datum(_)
        | Pickable::FrameAxis { .. }
        | Pickable::FramePlane { .. }
        | Pickable::CentreOfMass(_)
        | Pickable::Face { .. }
        | Pickable::Edge { .. }
        | Pickable::Vertex { .. }
        | Pickable::SketchConstraint { .. }
        | Pickable::SketchRegion { .. }
        | Pickable::FeatureValue { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. }
        | Pickable::BodyItem { .. } => None,
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

pub fn toggle_body(document: &Document, body: FeatureId) -> Result<Transaction, String> {
    if !body.is_piece() {
        return document
            .feature(body)
            .ok_or_else(|| NOT_HIDEABLE.to_owned())
            .and_then(toggle);
    }
    let maker = document
        .body_maker(body)
        .ok_or_else(|| NOT_HIDEABLE.to_owned())?;
    let name = document.body_name(body).unwrap_or_default();
    let hidden = document.body_hidden(body);
    let verb = if hidden { "Show" } else { "Hide" };
    let mut edits = vec![set_hidden(body, !hidden)];
    if hidden && maker.hidden {
        edits.push(set_hidden(maker.id(), false));
    }
    Ok(Transaction::new(format!("{verb} {name}"), edits))
}

pub fn hideable_rows(document: &Document, rows: &[FeatureId]) -> Vec<FeatureId> {
    let wanted: BTreeSet<FeatureId> = rows
        .iter()
        .filter_map(|id| document.feature(*id))
        .filter_map(|feature| {
            if can_hide(feature) {
                Some(feature.id())
            } else {
                feature.body()
            }
        })
        .collect();
    document
        .features()
        .filter(|feature| wanted.contains(&feature.id()) && can_hide(feature))
        .map(Feature::id)
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowToggle {
    pub hiding: bool,
    pub transaction: Transaction,
}

pub fn toggle_rows(document: &Document, rows: &[FeatureId]) -> Result<RowToggle, String> {
    let toggled: Vec<&Feature> = hideable_rows(document, rows)
        .into_iter()
        .filter_map(|id| document.feature(id))
        .collect();
    let hiding = toggled.iter().any(|feature| !feature.hidden);
    let verb = if hiding { "Hide" } else { "Show" };
    let label = match toggled.as_slice() {
        [] => return Err(NOT_HIDEABLE.to_owned()),
        [only] => format!("{verb} {}", only.name),
        many => format!("{verb} {} features", many.len()),
    };
    let edits = toggled
        .iter()
        .filter(|feature| feature.hidden != hiding)
        .map(|feature| set_hidden(feature.id(), hiding))
        .collect();
    Ok(RowToggle {
        hiding,
        transaction: Transaction::new(label, edits),
    })
}

pub struct PrincipalToggle {
    pub label: String,
    pub actions: Vec<Action>,
}

fn showing(
    document: &Document,
    label: String,
    geometries: impl IntoIterator<Item = PrincipalGeometry> + Clone,
) -> PrincipalToggle {
    let edits: Vec<Edit> = geometries
        .clone()
        .into_iter()
        .filter(|geometry| document.is_principal_hidden(*geometry))
        .map(|geometry| set_principal_hidden(geometry, false))
        .collect();
    let applied =
        (!edits.is_empty()).then(|| Action::Apply(Transaction::new(label.clone(), edits)));
    PrincipalToggle {
        label,
        actions: applied
            .into_iter()
            .chain([Action::KeepPrincipalShown(geometries.into_iter().collect())])
            .collect(),
    }
}

fn hiding(label: String, edits: Vec<Edit>) -> PrincipalToggle {
    PrincipalToggle {
        actions: vec![Action::Apply(Transaction::new(label.clone(), edits))],
        label,
    }
}

pub fn toggle_principal(
    document: &Document,
    view: PrincipalView,
    geometry: PrincipalGeometry,
) -> PrincipalToggle {
    if view.shows(document, geometry) {
        hiding(
            format!("Hide {}", geometry.name()),
            vec![set_principal_hidden(geometry, true)],
        )
    } else {
        showing(document, format!("Show {}", geometry.name()), [geometry])
    }
}

pub fn any_principal_shown(document: &Document, view: PrincipalView) -> bool {
    PrincipalGeometry::ALL
        .into_iter()
        .any(|geometry| view.shows(document, geometry))
}

pub fn toggle_principal_group(document: &Document, view: PrincipalView) -> PrincipalToggle {
    if any_principal_shown(document, view) {
        hiding(
            format!("Hide {PRINCIPAL_GROUP}"),
            PrincipalGeometry::ALL
                .into_iter()
                .filter(|geometry| view.shows(document, *geometry))
                .map(|geometry| set_principal_hidden(geometry, true))
                .collect(),
        )
    } else {
        showing(
            document,
            format!("Show {PRINCIPAL_GROUP}"),
            PrincipalGeometry::ALL,
        )
    }
}

pub fn toggle_kind(
    document: &Document,
    kind: Kind,
    edited: Option<FeatureId>,
) -> Result<Transaction, String> {
    let held: Vec<&Feature> = document
        .features()
        .filter(|feature| kind.holds(feature) && Some(feature.id()) != edited)
        .collect();
    if held.is_empty() {
        return Err(format!(
            "The model has no {} to hide or show",
            kind.plural()
        ));
    }
    let hide = held.iter().any(|feature| !feature.hidden);
    let verb = if hide { "Hide" } else { "Show" };
    Ok(Transaction::new(
        format!("{verb} every {}", kind.singular()),
        held.into_iter()
            .filter(|feature| feature.hidden != hide)
            .map(|feature| set_hidden(feature.id(), hide))
            .collect(),
    ))
}

pub fn hide_selection(
    document: &Document,
    selection: &Selection,
    rows: &[FeatureId],
    edited: Option<FeatureId>,
) -> Result<Transaction, String> {
    let owners: BTreeSet<FeatureId> = selection
        .iter()
        .filter_map(owner)
        .chain(hideable_rows(document, rows))
        .filter(|owner| Some(*owner) != edited)
        .collect();
    let pieces: Vec<FeatureId> = owners
        .iter()
        .copied()
        .filter(|owner| owner.is_piece() && !document.body_hidden(*owner))
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
    let label = match (hiding.as_slice(), principal.len() + pieces.len()) {
        ([], 0) => return Err(NOTHING_TO_HIDE.to_owned()),
        ([only], 0) => format!("Hide {}", only.name),
        ([], 1) if let [piece] = pieces.as_slice() => {
            format!("Hide {}", document.body_name(*piece).unwrap_or_default())
        }
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
            .chain(pieces.into_iter().map(|piece| set_hidden(piece, true)))
            .chain(
                principal
                    .into_iter()
                    .map(|geometry| set_principal_hidden(geometry, true)),
            )
            .collect(),
    ))
}

pub fn hide_others(
    document: &Document,
    view: PrincipalView,
    selection: &Selection,
    rows: &[FeatureId],
    edited: Option<FeatureId>,
) -> Result<Transaction, String> {
    let kept_features: BTreeSet<FeatureId> = selection
        .iter()
        .filter_map(owner)
        .map(FeatureId::maker)
        .chain(hideable_rows(document, rows))
        .collect();
    let kept_principal: BTreeSet<PrincipalGeometry> =
        selection.iter().filter_map(principal).collect();
    let kept = kept_features
        .iter()
        .filter_map(|id| document.feature(id.to_owned()))
        .filter(|feature| can_hide(feature))
        .count()
        + kept_principal.len();
    if kept == 0 {
        return Err(NOTHING_TO_ISOLATE.to_owned());
    }
    let edits: Vec<Edit> = document
        .features()
        .filter(|feature| {
            !feature.hidden
                && can_hide(feature)
                && !kept_features.contains(&feature.id())
                && Some(feature.id()) != edited
        })
        .map(|feature| set_hidden(feature.id(), true))
        .chain(
            PrincipalGeometry::ALL
                .into_iter()
                .filter(|geometry| {
                    view.shows(document, *geometry) && !kept_principal.contains(geometry)
                })
                .map(|geometry| set_principal_hidden(geometry, true)),
        )
        .collect();
    if edits.is_empty() {
        return Err(ALREADY_ISOLATED.to_owned());
    }
    let label = if kept == 1 {
        let name = kept_features
            .iter()
            .filter_map(|id| document.feature(*id))
            .find(|feature| can_hide(feature))
            .map(|feature| feature.name.clone())
            .or_else(|| {
                kept_principal
                    .first()
                    .map(|geometry| geometry.name().to_owned())
            })
            .unwrap_or_default();
        format!("Hide everything but {name}")
    } else {
        format!("Hide everything but {kept} items")
    };
    Ok(Transaction::new(label, edits))
}

pub fn show_all(document: &Document) -> Result<Transaction, String> {
    let hidden_pieces = document.features().flat_map(|feature| {
        feature
            .piece_looks
            .hidden()
            .filter_map(|key| feature.id().piece(key))
            .map(|piece| set_hidden(piece, false))
            .collect::<Vec<_>>()
    });
    let edits: Vec<Edit> = document
        .features()
        .filter(|feature| feature.hidden)
        .map(|feature| set_hidden(feature.id(), false))
        .chain(hidden_pieces)
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

        let hide = hide_selection(&document, &selection, &[], None).unwrap();
        document.apply(hide.clone()).unwrap();
        let again = hide_selection(&document, &selection, &[], None);
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
    fn hiding_others_keeps_the_selected_owners_and_principal_geometry() {
        let mut document = Document::default();
        let mut transaction = document.transaction("Sketches");
        let first = transaction.add_feature("First", FeatureKind::from(Sketch::new(Plane::XY)));
        let second = transaction.add_feature("Second", FeatureKind::from(Sketch::new(Plane::XZ)));
        let third = transaction.add_feature("Third", FeatureKind::from(Sketch::new(Plane::YZ)));
        document.apply(transaction.finish()).unwrap();
        let mut selection = Selection::default();
        selection.toggle(Pickable::SketchEntity {
            feature: first,
            entity: caditor_sketch::EntityId::ORIGIN,
        });

        let nothing = hide_others(
            &document,
            PrincipalView::default(),
            &Selection::default(),
            &[],
            None,
        );
        let isolate =
            hide_others(&document, PrincipalView::default(), &selection, &[], None).unwrap();
        document.apply(isolate.clone()).unwrap();
        let again = hide_others(&document, PrincipalView::default(), &selection, &[], None);

        assert_eq!(nothing, Err(NOTHING_TO_ISOLATE.to_owned()));
        assert_eq!(isolate.label(), "Hide everything but First");
        assert_eq!(isolate.edits().len(), 2 + PrincipalGeometry::ALL.len());
        assert!(is_shown(&document, first));
        assert!(!is_shown(&document, second) && !is_shown(&document, third));
        assert!(!any_principal_shown(&document, PrincipalView::default()));
        assert_eq!(again, Err(ALREADY_ISOLATED.to_owned()));
    }

    #[test]
    fn a_kind_hides_while_any_of_it_is_shown_then_shows_it_all_again() {
        let mut document = Document::default();
        let mut transaction = document.transaction("Sketches");
        let first = transaction.add_feature("First", FeatureKind::from(Sketch::new(Plane::XY)));
        let second = transaction.add_feature("Second", FeatureKind::from(Sketch::new(Plane::XZ)));
        let edited = transaction.add_feature("Edited", FeatureKind::from(Sketch::new(Plane::YZ)));
        document.apply(transaction.finish()).unwrap();
        document
            .apply(toggle(document.feature(second).unwrap()).unwrap())
            .unwrap();

        let hide = toggle_kind(&document, Kind::Sketches, Some(edited)).unwrap();
        document.apply(hide.clone()).unwrap();
        let show = toggle_kind(&document, Kind::Sketches, Some(edited)).unwrap();
        document.apply(show.clone()).unwrap();

        assert_eq!(hide.label(), "Hide every sketch");
        assert_eq!(hide.edits().len(), 1);
        assert_eq!(show.label(), "Show every sketch");
        assert_eq!(show.edits().len(), 2);
        assert!(
            [first, second, edited]
                .iter()
                .all(|id| is_shown(&document, *id))
        );
        assert_eq!(
            toggle_kind(&document, Kind::Datums, None),
            Err("The model has no datums to hide or show".to_owned())
        );
        assert!(toggle_kind(&document, Kind::Bodies, None).is_err());
    }

    fn applied(
        document: &mut Document,
        toggle: &PrincipalToggle,
    ) -> (usize, Vec<PrincipalGeometry>) {
        let mut edits = 0;
        let mut kept = Vec::new();
        for action in &toggle.actions {
            match action {
                Action::Apply(transaction) => {
                    edits += transaction.edits().len();
                    document.apply(transaction.clone()).unwrap();
                }
                Action::KeepPrincipalShown(geometries) => kept.extend(geometries),
                other => panic!("unexpected {other:?}"),
            }
        }
        (edits, kept)
    }

    #[test]
    fn the_principal_group_hides_what_is_shown_and_then_shows_everything_again() {
        let mut document = Document::default();
        let shown = PrincipalView::default();
        let origin = PrincipalGeometry::Origin;
        let hide_origin = toggle_principal(&document, shown, origin);
        applied(&mut document, &hide_origin);

        let hide_rest = toggle_principal_group(&document, shown);
        let (hidden, _) = applied(&mut document, &hide_rest);
        let all_hidden = !any_principal_shown(&document, shown);
        let show = toggle_principal_group(&document, shown);
        let (unhidden, kept) = applied(&mut document, &show);

        assert_eq!(hide_rest.label, "Hide principal planes, axes and origin");
        assert_eq!(hidden, 6);
        assert!(all_hidden);
        assert_eq!(show.label, "Show principal planes, axes and origin");
        assert_eq!(unhidden, 7);
        assert_eq!(kept, PrincipalGeometry::ALL);
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

    #[test]
    fn tucked_away_principal_geometry_is_kept_shown_without_a_change_and_hidden_by_one() {
        let mut document = Document::default();
        let xy = PrincipalGeometry::Plane(crate::selection::PrincipalPlane::Xy);
        let tucked = PrincipalView {
            tucked_away: true,
            ..PrincipalView::default()
        };

        let hidden_while_tucked = !any_principal_shown(&document, tucked);
        let show = toggle_principal(&document, tucked, xy);
        let (show_edits, kept) = applied(&mut document, &show);
        let keeping = PrincipalView {
            kept: tucked.kept.with(kept.iter().copied()),
            ..tucked
        };
        let shown_kept = keeping.shows(&document, xy);
        let others_tucked = PrincipalGeometry::ALL
            .into_iter()
            .filter(|geometry| *geometry != xy)
            .all(|geometry| !keeping.shows(&document, geometry));
        let hide = toggle_principal(&document, keeping, xy);
        let (hide_edits, _) = applied(&mut document, &hide);
        let group = toggle_principal_group(&document, keeping);

        assert!(hidden_while_tucked);
        assert_eq!(show.label, "Show XY plane");
        assert_eq!(show_edits, 0);
        assert_eq!(kept, [xy]);
        assert!(shown_kept && others_tucked);
        assert_eq!(hide.label, "Hide XY plane");
        assert_eq!(hide_edits, 1);
        assert!(document.is_principal_hidden(xy));
        assert!(!keeping.shows(&document, xy));
        assert_eq!(group.label, "Show principal planes, axes and origin");
        assert!(!PrincipalView::default().tucks(xy));
    }
}
