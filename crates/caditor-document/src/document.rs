use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    sync::Arc,
};

use caditor_expression::{Expression, ParameterId, ParseError};
use caditor_kernel::FaceReference;
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch};

use crate::{
    attachment::{FaceAttachment, SketchAttachment, SketchFeature},
    blend::Blend,
    body_appearance::BodyAppearance,
    combine::Combine,
    datum::{Datum, PrincipalGeometry},
    edit::{Edit, Transaction},
    hole::Hole,
    import::Import,
    mirror::Mirror,
    movement::Move,
    parameter_list::ParameterList,
    pattern::Pattern,
    scaling::Scale,
    shell::Shell,
    solid::{BodyOperation, SolidFeature},
};

pub const FIRST_UNSTORABLE_ID: u64 = 1 << 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureId(u64);

impl FeatureId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for FeatureId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    id: ParameterId,
    pub name: String,
    pub expression: Expression,
}

impl Parameter {
    pub fn new(id: ParameterId, name: String, expression: Expression) -> Self {
        Self {
            id,
            name,
            expression,
        }
    }

    pub fn id(&self) -> ParameterId {
        self.id
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureKind {
    Sketch(SketchFeature),
    Solid(SolidFeature),
    Blend(Blend),
    Shell(Shell),
    Combine(Combine),
    Move(Move),
    Mirror(Mirror),
    Scale(Scale),
    Hole(Hole),
    Pattern(Box<Pattern>),
    Datum(Datum),
    Import(Import),
}

impl From<Pattern> for FeatureKind {
    fn from(pattern: Pattern) -> Self {
        Self::Pattern(Box::new(pattern))
    }
}

impl From<Sketch> for FeatureKind {
    fn from(sketch: Sketch) -> Self {
        Self::Sketch(SketchFeature::from(sketch))
    }
}

impl FeatureKind {
    pub fn approximate_size(&self) -> usize {
        let owned = match self {
            Self::Sketch(sketch) => {
                let entities = sketch.sketch.entities().len();
                let constraints = sketch.sketch.constraints().len();
                let owned_by_entities: usize = sketch
                    .sketch
                    .entities()
                    .map(|(_, entity)| entity.heap_size())
                    .sum();
                let owned_by_constraints: usize = sketch
                    .sketch
                    .constraints()
                    .map(|(_, constraint)| constraint.heap_size())
                    .sum();
                entities * (size_of::<EntityId>() + size_of::<Entity>())
                    + constraints * (size_of::<ConstraintId>() + size_of::<Constraint>())
                    + owned_by_entities
                    + owned_by_constraints
            }
            Self::Solid(solid) => solid.heap_size(),
            Self::Blend(blend) => size_of_val(blend.edges.as_slice()) + blend.size.heap_size(),
            Self::Shell(shell) => {
                size_of_val(shell.open.as_slice())
                    + shell
                        .open
                        .iter()
                        .map(FaceReference::heap_size)
                        .sum::<usize>()
                    + shell.thickness.heap_size()
            }
            Self::Combine(_) => 0,
            Self::Move(movement) => movement.heap_size(),
            Self::Mirror(mirror) => mirror.heap_size(),
            Self::Scale(scale) => scale.heap_size(),
            Self::Hole(hole) => hole.heap_size(),
            Self::Import(import) => {
                import.source.len() + import.step.len() + import.solid.approximate_size()
            }
            Self::Pattern(pattern) => size_of::<Pattern>() + pattern.heap_size(),
            Self::Datum(datum) => datum.heap_size(),
        };
        size_of::<Self>() + owned
    }

    pub fn sketch(&self) -> Option<&Sketch> {
        match self {
            Self::Sketch(sketch) => Some(&sketch.sketch),
            Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn sketch_mut(&mut self) -> Option<&mut Sketch> {
        match self {
            Self::Sketch(sketch) => Some(&mut sketch.sketch),
            Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn attachment(&self) -> Option<&SketchAttachment> {
        match self {
            Self::Sketch(sketch) => sketch.attachment.as_ref(),
            Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn body_input(&self) -> Option<FeatureId> {
        match self {
            Self::Sketch(sketch) => sketch.attachment.as_ref().and_then(SketchAttachment::body),
            Self::Solid(solid) => solid.operation().target(),
            Self::Blend(blend) => Some(blend.body),
            Self::Shell(shell) => Some(shell.body),
            Self::Combine(combine) => Some(combine.body),
            Self::Move(movement) => Some(movement.body),
            Self::Mirror(mirror) => Some(mirror.body),
            Self::Scale(scale) => Some(scale.body),
            Self::Hole(hole) => Some(hole.body),
            Self::Pattern(pattern) => Some(pattern.body),
            Self::Datum(_) | Self::Import(_) => None,
        }
    }

    pub fn bodies_used(&self) -> BTreeSet<FeatureId> {
        let mut used: BTreeSet<FeatureId> = self.body_input().into_iter().collect();
        match self {
            Self::Solid(solid) => {
                used.extend(solid.axis_body());
                used.extend(solid.end_bodies());
            }
            Self::Datum(datum) => used.extend(datum.bodies()),
            Self::Pattern(pattern) => used.extend(pattern.axis_bodies()),
            Self::Combine(combine) => {
                used.insert(combine.tool);
            }
            Self::Mirror(mirror) => used.extend(mirror.plane.body()),
            Self::Sketch(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Move(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Import(_) => {}
        }
        used
    }

    pub fn planes_used(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Sketch(sketch) => sketch
                .attachment
                .as_ref()
                .and_then(SketchAttachment::datum)
                .into_iter()
                .collect(),
            Self::Solid(solid) => solid.end_datums(),
            Self::Datum(datum) => datum.plane_datums(),
            Self::Mirror(mirror) => mirror.plane.datum().into_iter().collect(),
            Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Import(_) => BTreeSet::new(),
        }
    }

    pub fn axes_used(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Solid(solid) => solid.axis_datum().into_iter().collect(),
            Self::Datum(datum) => datum.axis_datums(),
            Self::Pattern(pattern) => pattern.axis_datums(),
            Self::Sketch(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Import(_) => BTreeSet::new(),
        }
    }

    pub fn consumed_bodies(&self) -> Vec<FeatureId> {
        match self {
            Self::Combine(combine) => vec![combine.tool],
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => Vec::new(),
        }
    }

    pub fn modifies_body(&self) -> bool {
        matches!(
            self,
            Self::Blend(_) | Self::Shell(_) | Self::Combine(_) | Self::Move(_) | Self::Hole(_)
        )
    }

    pub fn solid(&self) -> Option<&SolidFeature> {
        match self {
            Self::Solid(solid) => Some(solid),
            Self::Sketch(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn blend(&self) -> Option<&Blend> {
        match self {
            Self::Blend(blend) => Some(blend),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn shell(&self) -> Option<&Shell> {
        match self {
            Self::Shell(shell) => Some(shell),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn hole(&self) -> Option<&Hole> {
        match self {
            Self::Hole(hole) => Some(hole),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn movement(&self) -> Option<&Move> {
        match self {
            Self::Move(movement) => Some(movement),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn mirror(&self) -> Option<&Mirror> {
        match self {
            Self::Mirror(mirror) => Some(mirror),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn scale(&self) -> Option<&Scale> {
        match self {
            Self::Scale(scale) => Some(scale),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn combine(&self) -> Option<&Combine> {
        match self {
            Self::Combine(combine) => Some(combine),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn pattern(&self) -> Option<&Pattern> {
        match self {
            Self::Pattern(pattern) => Some(pattern.as_ref()),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Datum(_)
            | Self::Import(_) => None,
        }
    }

    pub fn datum(&self) -> Option<&Datum> {
        match self {
            Self::Datum(datum) => Some(datum),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Import(_) => None,
        }
    }

    pub fn import(&self) -> Option<&Import> {
        match self {
            Self::Import(import) => Some(import),
            Self::Sketch(_)
            | Self::Solid(_)
            | Self::Blend(_)
            | Self::Shell(_)
            | Self::Combine(_)
            | Self::Move(_)
            | Self::Mirror(_)
            | Self::Scale(_)
            | Self::Hole(_)
            | Self::Pattern(_)
            | Self::Datum(_) => None,
        }
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        match self {
            Self::Sketch(sketch) => sketch.sketch.parameters(),
            Self::Solid(solid) => solid.parameters(),
            Self::Blend(blend) => blend.parameters(),
            Self::Shell(shell) => shell.parameters(),
            Self::Combine(_) => BTreeSet::new(),
            Self::Move(movement) => movement.parameters(),
            Self::Mirror(_) => BTreeSet::new(),
            Self::Scale(scale) => scale.parameters(),
            Self::Hole(hole) => hole.parameters(),
            Self::Pattern(pattern) => pattern.parameters(),
            Self::Datum(datum) => datum.parameters(),
            Self::Import(_) => BTreeSet::new(),
        }
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        match self {
            Self::Sketch(sketch) => sketch.sketch.uses_parameter(parameter),
            Self::Solid(solid) => solid.uses_parameter(parameter),
            Self::Blend(blend) => blend.uses_parameter(parameter),
            Self::Shell(shell) => shell.uses_parameter(parameter),
            Self::Combine(_) => false,
            Self::Move(movement) => movement.uses_parameter(parameter),
            Self::Mirror(_) => false,
            Self::Scale(scale) => scale.uses_parameter(parameter),
            Self::Hole(hole) => hole.uses_parameter(parameter),
            Self::Pattern(pattern) => pattern.uses_parameter(parameter),
            Self::Datum(datum) => datum.uses_parameter(parameter),
            Self::Import(_) => false,
        }
    }

    pub fn same_content(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Sketch(own), Self::Sketch(theirs)) => {
                own.sketch.same_content(&theirs.sketch) && own.attachment == theirs.attachment
            }
            _ => self == other,
        }
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = match self {
            Self::Sketch(_) => BTreeSet::new(),
            Self::Solid(solid) => solid.features(),
            Self::Blend(blend) => blend.features(),
            Self::Shell(shell) => shell.features(),
            Self::Combine(combine) => combine.features(),
            Self::Move(movement) => movement.features(),
            Self::Mirror(mirror) => mirror.features(),
            Self::Scale(scale) => scale.features(),
            Self::Hole(hole) => hole.features(),
            Self::Pattern(pattern) => pattern.features(),
            Self::Datum(datum) => datum.features(),
            Self::Import(_) => BTreeSet::new(),
        };
        used.extend(self.bodies_used());
        used.extend(self.planes_used());
        used.extend(self.axes_used());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Sketch(sketch) => sketch
                .attachment
                .as_ref()
                .and_then(SketchAttachment::face)
                .map(FaceAttachment::origin_features)
                .unwrap_or_default(),
            Self::Solid(solid) => solid.origin_features(),
            Self::Blend(blend) => blend.origin_features(),
            Self::Shell(shell) => shell.origin_features(),
            Self::Combine(_) | Self::Move(_) | Self::Scale(_) | Self::Hole(_) => BTreeSet::new(),
            Self::Mirror(mirror) => mirror.plane.origin_features(),
            Self::Pattern(pattern) => pattern.origin_features(),
            Self::Datum(datum) => datum.origin_features(),
            Self::Import(_) => BTreeSet::new(),
        }
    }

    pub fn dependencies(&self) -> BTreeSet<FeatureId> {
        let mut used = self.features();
        used.extend(self.origin_features());
        used
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    id: FeatureId,
    pub name: String,
    pub kind: FeatureKind,
    pub hidden: bool,
    pub suppressed: bool,
    pub appearance: BodyAppearance,
}

impl Feature {
    pub fn new(id: FeatureId, name: String, kind: FeatureKind) -> Self {
        Self {
            id,
            name,
            kind,
            hidden: false,
            suppressed: false,
            appearance: BodyAppearance::default(),
        }
    }

    pub fn id(&self) -> FeatureId {
        self.id
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.hidden == other.hidden
            && self.suppressed == other.suppressed
            && self.appearance == other.appearance
            && self.kind.same_content(&other.kind)
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        let mut used = self.kind.parameters();
        used.extend(self.appearance.parameters());
        used
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.kind.uses_parameter(parameter) || self.appearance.uses_parameter(parameter)
    }

    pub fn heap_size(&self) -> usize {
        self.name.len() + self.kind.approximate_size() + self.appearance.heap_size()
    }

    pub fn body(&self) -> Option<FeatureId> {
        match &self.kind {
            FeatureKind::Solid(solid) => Some(solid.operation().target().unwrap_or(self.id)),
            FeatureKind::Blend(blend) => Some(blend.body),
            FeatureKind::Shell(shell) => Some(shell.body),
            FeatureKind::Combine(combine) => Some(combine.body),
            FeatureKind::Move(movement) => Some(movement.body),
            FeatureKind::Mirror(mirror) => Some(mirror.body),
            FeatureKind::Scale(scale) => Some(scale.body),
            FeatureKind::Hole(hole) => Some(hole.body),
            FeatureKind::Pattern(pattern) => Some(pattern.body),
            FeatureKind::Import(_) => Some(self.id),
            FeatureKind::Sketch(_) | FeatureKind::Datum(_) => None,
        }
    }

    pub fn makes_body(&self) -> bool {
        match &self.kind {
            FeatureKind::Solid(solid) => solid.operation() == BodyOperation::NewBody,
            FeatureKind::Import(_) => true,
            FeatureKind::Sketch(_)
            | FeatureKind::Blend(_)
            | FeatureKind::Shell(_)
            | FeatureKind::Combine(_)
            | FeatureKind::Move(_)
            | FeatureKind::Mirror(_)
            | FeatureKind::Scale(_)
            | FeatureKind::Hole(_)
            | FeatureKind::Pattern(_)
            | FeatureKind::Datum(_) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RollbackBar {
    #[default]
    AtEnd,
    Before(FeatureId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreeRow {
    Feature(FeatureId),
    Bar,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub(crate) parameters: ParameterList,
    pub(crate) features: Vec<Arc<Feature>>,
    pub(crate) next_parameter_id: u64,
    pub(crate) next_feature_id: u64,
    pub(crate) hidden_principal: BTreeSet<PrincipalGeometry>,
    pub(crate) rollback: RollbackBar,
}

impl Document {
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    pub fn parameter(&self, id: ParameterId) -> Option<&Parameter> {
        self.parameters.get(id)
    }

    pub fn parameter_named(&self, name: &str) -> Option<&Parameter> {
        self.parameters.named(name)
    }

    pub fn parameter_name(&self, id: ParameterId) -> Option<&str> {
        self.parameter(id).map(|parameter| parameter.name.as_str())
    }

    pub fn features(&self) -> impl ExactSizeIterator<Item = &Feature> + DoubleEndedIterator {
        self.features.iter().map(Arc::as_ref)
    }

    pub(crate) fn feature_handles(&self) -> &[Arc<Feature>] {
        &self.features
    }

    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features
            .iter()
            .find(|feature| feature.id == id)
            .map(Arc::as_ref)
    }

    pub fn feature_index(&self, id: FeatureId) -> Option<usize> {
        self.features.iter().position(|feature| feature.id == id)
    }

    pub fn next_parameter_id(&self) -> u64 {
        self.next_parameter_id
    }

    pub fn next_feature_id(&self) -> u64 {
        self.next_feature_id
    }

    pub fn is_principal_hidden(&self, geometry: PrincipalGeometry) -> bool {
        self.hidden_principal.contains(&geometry)
    }

    pub fn hidden_principal(&self) -> impl Iterator<Item = PrincipalGeometry> + '_ {
        self.hidden_principal.iter().copied()
    }

    pub fn rollback_bar(&self) -> RollbackBar {
        self.rollback
    }

    pub fn bar_index(&self) -> usize {
        match self.rollback {
            RollbackBar::AtEnd => self.features.len(),
            RollbackBar::Before(feature) => {
                self.feature_index(feature).unwrap_or(self.features.len())
            }
        }
    }

    pub fn is_rolled_back(&self, id: FeatureId) -> bool {
        self.feature_index(id)
            .is_some_and(|index| index >= self.bar_index())
    }

    pub fn is_active(&self, id: FeatureId) -> bool {
        self.feature(id)
            .is_some_and(|feature| !feature.suppressed && !self.is_rolled_back(id))
    }

    pub fn active_features(&self) -> impl DoubleEndedIterator<Item = &Feature> {
        self.features()
            .take(self.bar_index())
            .filter(|feature| !feature.suppressed)
    }

    pub fn bar_before_index(&self, index: usize) -> RollbackBar {
        self.features
            .get(index)
            .map_or(RollbackBar::AtEnd, |feature| {
                RollbackBar::Before(feature.id)
            })
    }

    pub fn tree_rows(&self) -> Vec<TreeRow> {
        let bar = self.bar_index();
        let mut rows: Vec<TreeRow> = self
            .features
            .iter()
            .map(|feature| TreeRow::Feature(feature.id))
            .collect();
        rows.insert(bar.min(rows.len()), TreeRow::Bar);
        rows
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.parameters == other.parameters
            && self.hidden_principal == other.hidden_principal
            && self.rollback == other.rollback
            && self.features.len() == other.features.len()
            && self
                .features
                .iter()
                .zip(&other.features)
                .all(|(own, theirs)| Arc::ptr_eq(own, theirs) || own.same_content(theirs))
    }

    pub fn transaction_to(&self, target: &Self, label: impl Into<String>) -> Transaction {
        let unroll = (self.rollback != RollbackBar::AtEnd).then_some(Edit::SetRollbackBar {
            bar: RollbackBar::AtEnd,
        });
        let removals = unroll
            .into_iter()
            .chain(
                self.features
                    .iter()
                    .rev()
                    .map(|feature| Edit::RemoveFeature { id: feature.id }),
            )
            .chain(
                self.parameters
                    .iter()
                    .map(|parameter| Edit::SetParameterExpression {
                        id: parameter.id,
                        expression: Expression::Number(0.0),
                    }),
            )
            .chain(
                self.parameters
                    .iter()
                    .rev()
                    .map(|parameter| Edit::RemoveParameter { id: parameter.id }),
            );
        let insertions = target
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| Edit::InsertParameter {
                index,
                parameter: Parameter::new(
                    parameter.id,
                    parameter.name.clone(),
                    Expression::Number(0.0),
                ),
            })
            .chain(
                target
                    .parameters
                    .iter()
                    .map(|parameter| Edit::SetParameterExpression {
                        id: parameter.id,
                        expression: parameter.expression.clone(),
                    }),
            )
            .chain(target.features.iter().enumerate().map(|(index, feature)| {
                Edit::InsertFeature {
                    index,
                    feature: self.keeping_sketch_ids(feature),
                }
            }));
        let visibility = PrincipalGeometry::ALL
            .into_iter()
            .filter(|geometry| {
                self.is_principal_hidden(*geometry) != target.is_principal_hidden(*geometry)
            })
            .map(|geometry| Edit::SetPrincipalHidden {
                geometry,
                hidden: target.is_principal_hidden(geometry),
            });
        let rollback = (target.rollback != RollbackBar::AtEnd).then_some(Edit::SetRollbackBar {
            bar: target.rollback,
        });
        Transaction::new(
            label,
            removals
                .chain(insertions)
                .chain(visibility)
                .chain(rollback)
                .collect(),
        )
    }

    fn keeping_sketch_ids(&self, restored: &Arc<Feature>) -> Arc<Feature> {
        let current = self
            .feature(restored.id)
            .and_then(|feature| feature.kind.sketch())
            .map(Sketch::next_id);
        match (restored.kind.sketch(), current) {
            (Some(sketch), Some(next_id)) if next_id > sketch.next_id() => {
                let mut raised = Feature::clone(restored);
                if let Some(sketch) = raised.kind.sketch_mut() {
                    sketch.reserve_ids_below(next_id);
                }
                Arc::new(raised)
            }
            _ => Arc::clone(restored),
        }
    }

    pub fn reserve_ids_below(&mut self, next_parameter_id: u64, next_feature_id: u64) {
        self.next_parameter_id = self
            .next_parameter_id
            .max(next_parameter_id.min(FIRST_UNSTORABLE_ID));
        self.next_feature_id = self
            .next_feature_id
            .max(next_feature_id.min(FIRST_UNSTORABLE_ID));
    }

    pub fn parse(&self, text: &str) -> Result<Expression, ParseError> {
        Expression::parse(text, &|name| self.parameter_named(name).map(Parameter::id))
    }

    pub fn expression_text(&self, expression: &Expression) -> String {
        expression.to_text(&|id| self.parameter_name(id))
    }

    pub fn used_parameters(&self) -> BTreeSet<ParameterId> {
        let by_parameters = self
            .parameters
            .iter()
            .flat_map(|parameter| parameter.expression.parameters());
        let by_features = self
            .features
            .iter()
            .flat_map(|feature| feature.kind.parameters());
        by_parameters.chain(by_features).collect()
    }

    pub fn parameter_users(&self, parameter: ParameterId) -> Vec<String> {
        let parameters = self
            .parameters
            .iter()
            .filter(|other| other.expression.uses(parameter))
            .map(|other| other.name.clone());
        let features = self
            .features
            .iter()
            .filter(|feature| feature.uses_parameter(parameter))
            .map(|feature| feature.name.clone());
        parameters.chain(features).collect()
    }

    pub(crate) fn parameter_dependencies(&self) -> BTreeMap<ParameterId, BTreeSet<ParameterId>> {
        let ids: BTreeSet<ParameterId> = self.parameters.iter().map(Parameter::id).collect();
        self.parameters
            .iter()
            .map(|parameter| {
                let existing = parameter
                    .expression
                    .parameters()
                    .into_iter()
                    .filter(|used| ids.contains(used))
                    .collect();
                (parameter.id, existing)
            })
            .collect()
    }

    pub fn cycle_through(
        &self,
        target: ParameterId,
        expression: &Expression,
    ) -> Option<Vec<ParameterId>> {
        path_to(
            target,
            expression.parameters(),
            &self.parameter_dependencies(),
        )
    }
}

pub(crate) fn path_to(
    target: ParameterId,
    first_steps: impl IntoIterator<Item = ParameterId>,
    dependencies: &BTreeMap<ParameterId, BTreeSet<ParameterId>>,
) -> Option<Vec<ParameterId>> {
    let mut parents = BTreeMap::new();
    let mut queue = VecDeque::new();
    for step in first_steps {
        if parents.insert(step, target).is_none() {
            queue.push_back(step);
        }
    }
    while let Some(current) = queue.pop_front() {
        if current == target {
            return Some(path_back_to(target, &parents));
        }
        for next in dependencies.get(&current).into_iter().flatten() {
            if !parents.contains_key(next) {
                parents.insert(*next, current);
                queue.push_back(*next);
            }
        }
    }
    None
}

fn path_back_to(
    target: ParameterId,
    parents: &BTreeMap<ParameterId, ParameterId>,
) -> Vec<ParameterId> {
    let mut path = vec![target];
    let mut current = parents.get(&target).copied();
    while let Some(step) = current {
        path.push(step);
        if step == target || path.len() > parents.len() + 1 {
            break;
        }
        current = parents.get(&step).copied();
    }
    path.reverse();
    path
}

pub(crate) fn list_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
