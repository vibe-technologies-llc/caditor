use std::{borrow::Cow, collections::BTreeSet, time::Duration};

use caditor_document::{
    Document, Edit, Evaluation, Feature, FeatureId, FeatureKind, PrincipalAxis, PrincipalGeometry,
    PrincipalPlane, PrincipalShowing, Transaction,
};
use caditor_geometry::Vector3;
use egui::Id;

use crate::{
    editing::SketchEditing,
    selection::{Axis, Pickable, Selection},
};

const PRINCIPAL_GROUP: &str = "principal planes, axes and origin";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VisibilityError {
    #[error("Select a body, sketch, datum, principal plane or axis in the view or the tree first")]
    NothingToHide,
    #[error("Select what to keep in the view or the tree first")]
    NothingToIsolate,
    #[error("Everything else is already hidden")]
    AlreadyIsolated,
    #[error("Nothing is hidden")]
    NothingHidden,
    #[error("Only sketches, datums and features that make a body can be hidden")]
    NotHideable,
    #[error("The model has no {} to hide or show", kind.plural())]
    NoneOfKind { kind: Kind },
    #[error("The principal planes, axes and origin already show only when needed")]
    PrincipalAlreadyWhenNeeded,
}

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
            FeatureKind::Datum(_) | FeatureKind::Helix(_) | FeatureKind::ReferenceImage(_) => {
                self == Self::Datums
            }
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
        | FeatureKind::DeleteFace(_)
        | FeatureKind::Emboss(_)
        | FeatureKind::MoveFace(_)
        | FeatureKind::Draft(_)
        | FeatureKind::ReplaceFace(_)
        | FeatureKind::SplitFace(_)
        | FeatureKind::Primitive(_)
        | FeatureKind::Pipe(_)
        | FeatureKind::Loft(_)
        | FeatureKind::Coil(_)
        | FeatureKind::Helix(_)
        | FeatureKind::Rib(_)
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
    document.body_maker(feature).is_some() && !document.body_hidden(feature)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrincipalSet(u8);

impl PrincipalSet {
    pub const NONE: Self = Self(0);
    pub const PLANES: Self = Self(0b000_0111);
    pub const AXES: Self = Self(0b011_1000);
    pub const ORIGIN: Self = Self(0b100_0000);
    pub const ALL: Self = Self(0b111_1111);

    const fn bit(geometry: PrincipalGeometry) -> u8 {
        match geometry {
            PrincipalGeometry::Plane(PrincipalPlane::Xy) => 0b000_0001,
            PrincipalGeometry::Plane(PrincipalPlane::Xz) => 0b000_0010,
            PrincipalGeometry::Plane(PrincipalPlane::Yz) => 0b000_0100,
            PrincipalGeometry::Axis(PrincipalAxis::X) => 0b000_1000,
            PrincipalGeometry::Axis(PrincipalAxis::Y) => 0b001_0000,
            PrincipalGeometry::Axis(PrincipalAxis::Z) => 0b010_0000,
            PrincipalGeometry::Origin => 0b100_0000,
        }
    }

    pub const fn holds(self, geometry: PrincipalGeometry) -> bool {
        self.0 & Self::bit(geometry) != 0
    }

    pub const fn and(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn when(self, wanted: bool) -> Self {
        if wanted { self } else { Self::NONE }
    }

    pub fn with(self, geometries: impl IntoIterator<Item = PrincipalGeometry>) -> Self {
        Self(
            geometries
                .into_iter()
                .fold(self.0, |kept, geometry| kept | Self::bit(geometry)),
        )
    }
}

pub const FADE_STEPS: u8 = 12;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrincipalFading([u8; PrincipalGeometry::ALL.len()]);

impl PrincipalFading {
    fn index(geometry: PrincipalGeometry) -> usize {
        PrincipalGeometry::ALL
            .iter()
            .position(|listed| *listed == geometry)
            .unwrap_or_default()
    }

    pub fn steps_left(self, geometry: PrincipalGeometry) -> u8 {
        self.0
            .get(Self::index(geometry))
            .copied()
            .unwrap_or_default()
            .min(FADE_STEPS)
    }

    fn set(&mut self, geometry: PrincipalGeometry, steps_left: u8) {
        if let Some(steps) = self.0.get_mut(Self::index(geometry)) {
            *steps = steps_left.min(FADE_STEPS);
        }
    }
}

pub const FADE_SECONDS: f64 = 0.16;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PrincipalFade {
    shown: Option<PrincipalSet>,
    left: [f64; PrincipalGeometry::ALL.len()],
}

impl PrincipalFade {
    pub fn follow(&mut self, shown: PrincipalSet) {
        if let Some(before) = self.shown {
            for (geometry, left) in PrincipalGeometry::ALL.into_iter().zip(&mut self.left) {
                if before.holds(geometry) != shown.holds(geometry) {
                    *left = 1.0 - *left;
                }
            }
        }
        self.shown = Some(shown);
    }

    pub fn advance(&mut self, elapsed: Duration, scale: f64) {
        let taken = if scale > 0.0 {
            elapsed.as_secs_f64() / (FADE_SECONDS * scale)
        } else {
            1.0
        };
        for left in &mut self.left {
            *left = (*left - taken).max(0.0);
        }
    }

    pub fn is_fading(&self) -> bool {
        self.left.iter().any(|left| *left > 0.0)
    }

    pub fn fading(&self) -> PrincipalFading {
        let mut fading = PrincipalFading::default();
        for (geometry, left) in PrincipalGeometry::ALL.into_iter().zip(self.left) {
            let steps = left * f64::from(FADE_STEPS);
            let rounded_up = (1..=FADE_STEPS)
                .find(|step| f64::from(*step) >= steps)
                .unwrap_or(FADE_STEPS);
            fading.set(geometry, if left > 0.0 { rounded_up } else { 0 });
        }
        fading
    }
}

pub fn shown_principal(document: &Document, view: PrincipalView) -> PrincipalSet {
    PrincipalSet::NONE.with(
        PrincipalGeometry::ALL
            .into_iter()
            .filter(|geometry| view.shows(document, *geometry)),
    )
}

pub const EDGE_ON_STEPS: u8 = 8;
pub const EDGE_ON_HIDDEN_DEGREES: f64 = 3.0;
pub const EDGE_ON_FADED_DEGREES: f64 = 12.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeOn([u8; PrincipalPlane::ALL.len()]);

impl EdgeOn {
    pub fn seen_along(sight: Vector3) -> Self {
        let mut edge_on = Self::default();
        for (plane, steps) in PrincipalPlane::ALL.into_iter().zip(&mut edge_on.0) {
            let degrees = plane
                .plane()
                .normal()
                .dot(sight)
                .abs()
                .min(1.0)
                .asin()
                .to_degrees();
            let toward = (EDGE_ON_FADED_DEGREES - degrees)
                / (EDGE_ON_FADED_DEGREES - EDGE_ON_HIDDEN_DEGREES)
                * f64::from(EDGE_ON_STEPS);
            *steps = (0..=EDGE_ON_STEPS)
                .find(|step| f64::from(*step) >= toward)
                .unwrap_or(EDGE_ON_STEPS);
        }
        edge_on
    }

    pub fn share_seen(self, plane: PrincipalPlane) -> f32 {
        let steps = PrincipalPlane::ALL
            .iter()
            .position(|listed| *listed == plane)
            .and_then(|index| self.0.get(index))
            .copied()
            .unwrap_or_default()
            .min(EDGE_ON_STEPS);
        1.0 - f32::from(steps) / f32::from(EDGE_ON_STEPS)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrincipalView {
    pub tucking: bool,
    pub chosen: PrincipalSet,
    pub used: PrincipalSet,
    pub previewed: PrincipalSet,
    pub fading: PrincipalFading,
    pub edge_on: EdgeOn,
}

impl PrincipalView {
    pub fn shows(self, document: &Document, geometry: PrincipalGeometry) -> bool {
        match document.principal_showing(geometry) {
            PrincipalShowing::Shown => true,
            PrincipalShowing::Hidden => self.chosen.holds(geometry),
            PrincipalShowing::WhenNeeded => !self.tucks(geometry),
        }
    }

    pub fn tucks(self, geometry: PrincipalGeometry) -> bool {
        self.tucking
            && !self.chosen.holds(geometry)
            && !self.used.holds(geometry)
            && !self.previewed.holds(geometry)
    }

    pub fn reads_shown(self, document: &Document, geometry: PrincipalGeometry) -> bool {
        !document.is_principal_hidden(geometry) && self.shows(document, geometry)
    }

    pub fn opacity(self, document: &Document, geometry: PrincipalGeometry) -> Option<f32> {
        let left = f32::from(self.fading.steps_left(geometry)) / f32::from(FADE_STEPS);
        if self.shows(document, geometry) {
            Some(1.0 - left)
        } else {
            (left > 0.0).then_some(left)
        }
    }
}

pub struct Needs<'a> {
    pub editing: &'a SketchEditing,
    pub measuring: bool,
    pub listing: PrincipalSet,
}

pub fn previewed_principal(selection: &Selection, hovered: Option<Pickable>) -> PrincipalSet {
    PrincipalSet::default().with(selection.iter().chain(hovered).filter_map(principal))
}

pub fn has_shown_body(document: &Document) -> bool {
    document
        .active_features()
        .any(|feature| feature.makes_body() && !feature.hidden)
}

pub fn chosen_principal(needs: &Needs<'_>) -> PrincipalSet {
    let editing = needs.editing;
    let context = editing.context();
    editing
        .picking()
        .map_or(PrincipalSet::NONE, |picking| {
            picking.slot.principal_wanted()
        })
        .and(PrincipalSet::PLANES.when(
            editing.is_choosing_plane() || editing.is_choosing_mirrored() || context.intersecting,
        ))
        .and(
            PrincipalSet::AXES
                .and(PrincipalSet::ORIGIN)
                .when(context.projecting),
        )
}

pub fn used_principal(document: &Document, needs: &Needs<'_>) -> PrincipalSet {
    let open = needs.editing.solid().and_then(|id| document.feature(id));
    let datum_open = open.is_some_and(|feature| feature.kind.datum().is_some());
    let mirror_open = open.is_some_and(|feature| {
        matches!(&feature.kind, FeatureKind::Mirror(mirror) if mirror.mirrors_whole_body())
    });
    needs
        .listing
        .and(PrincipalSet::ALL.when(needs.measuring || datum_open))
        .and(PrincipalSet::PLANES.when(mirror_open))
}

pub fn principal_view(
    document: &Document,
    when_needed: bool,
    needs: &Needs<'_>,
    previewed: PrincipalSet,
) -> PrincipalView {
    PrincipalView {
        tucking: when_needed && has_shown_body(document),
        chosen: chosen_principal(needs),
        used: used_principal(document, needs),
        previewed,
        ..PrincipalView::default()
    }
}

fn listing_key() -> Id {
    Id::unique("principal-listing")
}

pub fn listing_principal(ctx: &egui::Context, listed: PrincipalSet) {
    ctx.data_mut(|data| {
        let before = data
            .get_temp::<PrincipalSet>(listing_key())
            .unwrap_or_default();
        data.insert_temp(listing_key(), before.and(listed));
    });
}

pub fn take_listing(ctx: &egui::Context) -> PrincipalSet {
    ctx.data_mut(|data| {
        let listed = data
            .get_temp::<PrincipalSet>(listing_key())
            .unwrap_or_default();
        data.remove::<PrincipalSet>(listing_key());
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

fn set_principal_showing(geometry: PrincipalGeometry, showing: PrincipalShowing) -> Edit {
    Edit::SetPrincipalShowing { geometry, showing }
}

pub fn toggle(feature: &Feature) -> Result<Transaction, VisibilityError> {
    if !can_hide(feature) {
        return Err(VisibilityError::NotHideable);
    }
    let verb = if feature.hidden { "Show" } else { "Hide" };
    Ok(Transaction::single(
        format!("{verb} {}", feature.name),
        set_hidden(feature.id(), !feature.hidden),
    ))
}

pub fn toggle_body(document: &Document, body: FeatureId) -> Result<Transaction, VisibilityError> {
    if !body.is_piece() {
        return document
            .feature(body)
            .ok_or(VisibilityError::NotHideable)
            .and_then(toggle);
    }
    let maker = document
        .body_maker(body)
        .ok_or(VisibilityError::NotHideable)?;
    let name = document.body_name(body).unwrap_or_default();
    let hidden = document.body_hidden(body);
    let verb = if hidden { "Show" } else { "Hide" };
    let mut edits = vec![set_hidden(body, !hidden)];
    if hidden && maker.hidden {
        edits.push(set_hidden(maker.id(), false));
    }
    Ok(Transaction::new(format!("{verb} {name}"), edits))
}

pub fn toggle_bodies(
    document: &Document,
    bodies: &[FeatureId],
    name: &str,
) -> Result<Transaction, VisibilityError> {
    let hiding = bodies.iter().any(|body| !document.body_hidden(*body));
    let mut edits = Vec::new();
    let mut makers_shown = BTreeSet::new();
    for body in bodies {
        let maker = document
            .body_maker(*body)
            .ok_or(VisibilityError::NotHideable)?;
        if document.body_hidden(*body) == hiding {
            continue;
        }
        edits.push(set_hidden(*body, hiding));
        if !hiding && body.is_piece() && maker.hidden && makers_shown.insert(maker.id()) {
            edits.push(set_hidden(maker.id(), false));
        }
    }
    let verb = if hiding { "Hide" } else { "Show" };
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

pub fn toggle_rows(document: &Document, rows: &[FeatureId]) -> Result<RowToggle, VisibilityError> {
    let toggled: Vec<&Feature> = hideable_rows(document, rows)
        .into_iter()
        .filter_map(|id| document.feature(id))
        .collect();
    let hiding = toggled.iter().any(|feature| !feature.hidden);
    let verb = if hiding { "Hide" } else { "Show" };
    let label = match toggled.as_slice() {
        [] => return Err(VisibilityError::NotHideable),
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

pub fn toggle_principal(
    document: &Document,
    view: PrincipalView,
    geometry: PrincipalGeometry,
) -> Transaction {
    let (verb, showing) = if view.reads_shown(document, geometry) {
        ("Hide", PrincipalShowing::Hidden)
    } else {
        ("Show", PrincipalShowing::Shown)
    };
    Transaction::single(
        format!("{verb} {}", geometry.name()),
        set_principal_showing(geometry, showing),
    )
}

pub fn any_principal_shown(document: &Document, view: PrincipalView) -> bool {
    PrincipalGeometry::ALL
        .into_iter()
        .any(|geometry| view.reads_shown(document, geometry))
}

pub fn toggle_principal_group(document: &Document, view: PrincipalView) -> Transaction {
    let hiding = any_principal_shown(document, view);
    let (verb, showing) = if hiding {
        ("Hide", PrincipalShowing::Hidden)
    } else {
        ("Show", PrincipalShowing::Shown)
    };
    Transaction::new(
        format!("{verb} {PRINCIPAL_GROUP}"),
        PrincipalGeometry::ALL
            .into_iter()
            .filter(|geometry| !hiding || view.reads_shown(document, *geometry))
            .filter(|geometry| document.principal_showing(*geometry) != showing)
            .map(|geometry| set_principal_showing(geometry, showing))
            .collect(),
    )
}

pub fn principal_only_when_needed(document: &Document) -> Result<Transaction, VisibilityError> {
    let edits: Vec<Edit> = PrincipalGeometry::ALL
        .into_iter()
        .filter(|geometry| document.principal_showing(*geometry) != PrincipalShowing::WhenNeeded)
        .map(|geometry| set_principal_showing(geometry, PrincipalShowing::WhenNeeded))
        .collect();
    if edits.is_empty() {
        return Err(VisibilityError::PrincipalAlreadyWhenNeeded);
    }
    Ok(Transaction::new(
        format!("Show {PRINCIPAL_GROUP} only when needed"),
        edits,
    ))
}

pub fn toggle_kind(
    document: &Document,
    kind: Kind,
    edited: Option<FeatureId>,
) -> Result<Transaction, VisibilityError> {
    let held: Vec<&Feature> = document
        .features()
        .filter(|feature| kind.holds(feature) && Some(feature.id()) != edited)
        .collect();
    if held.is_empty() {
        return Err(VisibilityError::NoneOfKind { kind });
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
) -> Result<Transaction, VisibilityError> {
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
        .filter(|geometry| !document.is_principal_hidden(*geometry))
        .collect();
    let label = match (hiding.as_slice(), principal.len() + pieces.len()) {
        ([], 0) => return Err(VisibilityError::NothingToHide),
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
                    .map(|geometry| set_principal_showing(geometry, PrincipalShowing::Hidden)),
            )
            .collect(),
    ))
}

pub fn hide_others(
    document: &Document,
    evaluation: &Evaluation,
    view: PrincipalView,
    selection: &Selection,
    rows: &[FeatureId],
    edited: Option<FeatureId>,
) -> Result<Transaction, VisibilityError> {
    let owners: BTreeSet<FeatureId> = selection.iter().filter_map(owner).collect();
    let kept_pieces: BTreeSet<FeatureId> = owners
        .iter()
        .copied()
        .filter(|owner| owner.is_piece())
        .collect();
    let kept_whole: BTreeSet<FeatureId> = owners
        .iter()
        .copied()
        .filter(|owner| !owner.is_piece())
        .chain(hideable_rows(document, rows))
        .collect();
    let kept_features: BTreeSet<FeatureId> = kept_pieces
        .iter()
        .map(|piece| piece.maker())
        .chain(kept_whole.iter().copied())
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
        return Err(VisibilityError::NothingToIsolate);
    }
    let other_pieces: Vec<FeatureId> = kept_pieces
        .iter()
        .map(|piece| piece.maker())
        .collect::<BTreeSet<FeatureId>>()
        .into_iter()
        .filter(|maker| !kept_whole.contains(maker))
        .flat_map(|maker| evaluation.pieces_made_by(maker))
        .filter(|piece| !kept_pieces.contains(piece) && !document.body_hidden(*piece))
        .collect();
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
            other_pieces
                .into_iter()
                .map(|piece| set_hidden(piece, true)),
        )
        .chain(
            PrincipalGeometry::ALL
                .into_iter()
                .filter(|geometry| {
                    view.reads_shown(document, *geometry) && !kept_principal.contains(geometry)
                })
                .map(|geometry| set_principal_showing(geometry, PrincipalShowing::Hidden)),
        )
        .collect();
    if edits.is_empty() {
        return Err(VisibilityError::AlreadyIsolated);
    }
    let label = if kept == 1 {
        let name = match (kept_pieces.first(), kept_pieces.len()) {
            (Some(piece), 1) if !kept_whole.contains(&piece.maker()) => {
                document.body_name(*piece).map(Cow::into_owned)
            }
            _ => kept_features
                .iter()
                .filter_map(|id| document.feature(*id))
                .find(|feature| can_hide(feature))
                .map(|feature| feature.name.clone()),
        }
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

pub fn show_all(document: &Document) -> Result<Transaction, VisibilityError> {
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
                .map(|geometry| set_principal_showing(geometry, PrincipalShowing::Shown)),
        )
        .collect();
    if edits.is_empty() {
        return Err(VisibilityError::NothingHidden);
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
        assert_eq!(again, Err(VisibilityError::NothingToHide));
        assert!(hidden_xy);
        assert!(is_shown(&document, first) && is_shown(&document, second));
        assert_eq!(document.principal_showing(xy), PrincipalShowing::Shown);
        assert_eq!(show_all(&document), Err(VisibilityError::NothingHidden));
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

        let evaluation = Evaluation::default();
        let nothing = hide_others(
            &document,
            &evaluation,
            PrincipalView::default(),
            &Selection::default(),
            &[],
            None,
        );
        let isolate = hide_others(
            &document,
            &evaluation,
            PrincipalView::default(),
            &selection,
            &[],
            None,
        )
        .unwrap();
        document.apply(isolate.clone()).unwrap();
        let again = hide_others(
            &document,
            &evaluation,
            PrincipalView::default(),
            &selection,
            &[],
            None,
        );

        assert_eq!(nothing, Err(VisibilityError::NothingToIsolate));
        assert_eq!(isolate.label(), "Hide everything but First");
        assert_eq!(isolate.edits().len(), 2 + PrincipalGeometry::ALL.len());
        assert!(is_shown(&document, first));
        assert!(!is_shown(&document, second) && !is_shown(&document, third));
        assert!(!any_principal_shown(&document, PrincipalView::default()));
        assert_eq!(again, Err(VisibilityError::AlreadyIsolated));
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
            Err(VisibilityError::NoneOfKind { kind: Kind::Datums })
        );
        assert!(toggle_kind(&document, Kind::Bodies, None).is_err());
    }

    fn applied(document: &mut Document, transaction: &Transaction) -> usize {
        document.apply(transaction.clone()).unwrap();
        transaction.edits().len()
    }

    fn tucking() -> PrincipalView {
        PrincipalView {
            tucking: true,
            ..PrincipalView::default()
        }
    }

    #[test]
    fn the_principal_group_hides_what_is_shown_and_then_shows_everything_again() {
        let mut document = Document::default();
        let shown = PrincipalView::default();
        let origin = PrincipalGeometry::Origin;
        let hide_origin = toggle_principal(&document, shown, origin);
        applied(&mut document, &hide_origin);

        let hide_rest = toggle_principal_group(&document, shown);
        let hidden = applied(&mut document, &hide_rest);
        let all_hidden = !any_principal_shown(&document, shown);
        let show = toggle_principal_group(&document, shown);
        let unhidden = applied(&mut document, &show);

        assert_eq!(hide_origin.label(), "Hide Origin");
        assert_eq!(hide_rest.label(), "Hide principal planes, axes and origin");
        assert_eq!(hidden, 6);
        assert!(all_hidden);
        assert_eq!(show.label(), "Show principal planes, axes and origin");
        assert_eq!(unhidden, 7);
        assert!(
            PrincipalGeometry::ALL
                .into_iter()
                .all(|geometry| document.principal_showing(geometry) == PrincipalShowing::Shown)
        );
        assert_eq!(pickable(origin), Pickable::Origin);
        assert!(
            PrincipalGeometry::ALL
                .into_iter()
                .all(|geometry| principal(pickable(geometry)) == Some(geometry))
        );
    }

    #[test]
    fn principal_geometry_shown_by_hand_stays_shown_and_hidden_by_hand_stays_hidden() {
        let mut document = Document::default();
        let xy = PrincipalGeometry::Plane(crate::selection::PrincipalPlane::Xy);
        let x = PrincipalGeometry::Axis(PrincipalAxis::X);

        let tucked_at_first = !any_principal_shown(&document, tucking());
        let show = toggle_principal(&document, tucking(), xy);
        applied(&mut document, &show);
        let kept = tucking().shows(&document, xy);
        let others_tucked = PrincipalGeometry::ALL
            .into_iter()
            .filter(|geometry| *geometry != xy)
            .all(|geometry| !tucking().shows(&document, geometry));
        let hide = toggle_principal(&document, tucking(), xy);
        applied(&mut document, &hide);
        let measuring = PrincipalView {
            used: PrincipalSet::ALL,
            ..tucking()
        };
        let choosing_planes = PrincipalView {
            chosen: PrincipalSet::PLANES,
            ..tucking()
        };

        assert!(tucked_at_first);
        assert_eq!(show.label(), "Show XY plane");
        assert!(kept && others_tucked);
        assert_eq!(hide.label(), "Hide XY plane");
        assert_eq!(document.principal_showing(xy), PrincipalShowing::Hidden);
        assert!(!measuring.shows(&document, xy));
        assert!(measuring.shows(&document, x));
        assert!(choosing_planes.shows(&document, xy));
        assert!(!choosing_planes.reads_shown(&document, xy));
        assert!(!choosing_planes.shows(&document, x));
        assert_eq!(
            toggle_principal_group(&document, tucking()).label(),
            "Show principal planes, axes and origin"
        );
        assert!(!PrincipalView::default().tucks(xy));
    }

    #[test]
    fn only_when_needed_returns_every_principal_item_to_its_default() {
        let mut document = Document::default();
        let xy = PrincipalGeometry::Plane(crate::selection::PrincipalPlane::Xy);
        let nothing_to_do = principal_only_when_needed(&document);
        for geometry in [xy, PrincipalGeometry::Origin, PrincipalGeometry::Origin] {
            let toggle = toggle_principal(&document, tucking(), geometry);
            applied(&mut document, &toggle);
        }

        let restore = principal_only_when_needed(&document).unwrap();
        applied(&mut document, &restore);

        assert_eq!(
            nothing_to_do,
            Err(VisibilityError::PrincipalAlreadyWhenNeeded)
        );
        assert_eq!(
            restore.label(),
            "Show principal planes, axes and origin only when needed"
        );
        assert_eq!(restore.edits().len(), 2);
        assert!(
            PrincipalGeometry::ALL.into_iter().all(
                |geometry| document.principal_showing(geometry) == PrincipalShowing::WhenNeeded
            )
        );
    }

    #[test]
    fn a_fade_runs_its_course_reverses_midway_and_is_instant_without_transitions() {
        let xy = PrincipalGeometry::Plane(crate::selection::PrincipalPlane::Xy);
        let half = Duration::from_secs_f64(FADE_SECONDS / 2.0);
        let mut fade = PrincipalFade::default();

        fade.follow(PrincipalSet::ALL);
        let settled_at_first = !fade.is_fading();
        fade.follow(PrincipalSet::AXES);
        let starting = fade.fading().steps_left(xy);
        fade.advance(half, 1.0);
        let halfway = fade.fading().steps_left(xy);
        fade.follow(PrincipalSet::ALL);
        let reversed = fade.fading().steps_left(xy);
        fade.advance(half * 2, 1.0);
        let finished = !fade.is_fading();
        fade.follow(PrincipalSet::NONE);
        fade.advance(Duration::ZERO, 0.0);
        let instant = !fade.is_fading();

        assert!(settled_at_first);
        assert_eq!(starting, FADE_STEPS);
        assert_eq!(halfway, FADE_STEPS / 2);
        assert_eq!(reversed, FADE_STEPS / 2);
        assert!(finished && instant);
    }

    #[test]
    fn fading_geometry_is_drawn_fainter_but_only_what_shows_is_picked() {
        let document = Document::default();
        let xy = PrincipalGeometry::Plane(crate::selection::PrincipalPlane::Xy);
        let mut fading = PrincipalFading::default();
        fading.set(xy, FADE_STEPS / 2);
        let leaving = PrincipalView {
            fading,
            ..tucking()
        };
        let arriving = PrincipalView {
            fading,
            ..PrincipalView::default()
        };

        assert_eq!(leaving.opacity(&document, xy), Some(0.5));
        assert!(!leaving.shows(&document, xy));
        assert_eq!(arriving.opacity(&document, xy), Some(0.5));
        assert!(arriving.shows(&document, xy));
        assert_eq!(
            tucking().opacity(&document, PrincipalGeometry::Origin),
            None
        );
    }

    #[test]
    fn a_plane_fades_as_it_turns_edge_on_and_is_gone_when_seen_along_it() {
        let [xy, xz, yz] = crate::selection::PrincipalPlane::ALL;
        let from_above = EdgeOn::seen_along(-Vector3::Z);
        let turned = |degrees: f64| {
            let radians = degrees.to_radians();
            EdgeOn::seen_along(Vector3::new(0.0, radians.cos(), -radians.sin()))
        };

        assert_eq!(from_above.share_seen(xy), 1.0);
        assert_eq!(from_above.share_seen(xz), 0.0);
        assert_eq!(from_above.share_seen(yz), 0.0);
        assert_eq!(turned(EDGE_ON_HIDDEN_DEGREES / 2.0).share_seen(xy), 0.0);
        assert_eq!(turned(EDGE_ON_FADED_DEGREES + 1.0).share_seen(xy), 1.0);
        let midway = turned((EDGE_ON_HIDDEN_DEGREES + EDGE_ON_FADED_DEGREES) / 2.0).share_seen(xy);
        assert!(midway > 0.0 && midway < 1.0);
        assert_eq!(EdgeOn::default().share_seen(xz), 1.0);
    }
}
