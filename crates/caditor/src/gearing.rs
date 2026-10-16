use caditor_document::{FeatureId, Transaction};
use caditor_expression::Dimension;
use caditor_geometry::Point2;
use caditor_sketch::{
    ChainSize, Entity, EntityId, Faceting, GearCentre, GearError, GearOutline, GearPair,
    MAX_SPROCKET_TEETH, MAX_TEETH, MIN_SPROCKET_TEETH, MIN_TEETH, Sketch, Sprocket,
    SprocketOutline, SpurGear,
};
use thiserror::Error;

use crate::{
    drawing::Preview,
    editing::Tool,
    field::Expected,
    model::Model,
    modifying::{HighlightNeeded, Hint, Outcome, Prompt, StepError, length_text},
    patterning,
    snap::{self, Pointer, Screen},
    trimming::{self, SketchGone, capitalized},
    units::Units,
};

pub const TRANSACTION: &str = "Draw spur gear";
pub const PAIR_TRANSACTION: &str = "Draw gear pair";
pub const SPROCKET_TRANSACTION: &str = "Draw sprocket";
pub const PROMPT: &str =
    "Click where the gear's centre goes: on a point, or anywhere in the sketch";
const KEYS: &str = "Enter: draw it at the selected point or the origin   Esc: back to Select";
const WHOLE_NUMBER_TOLERANCE: f64 = 1e-9;
const LENGTH: Expected = Expected {
    dimension: Some(Dimension::LENGTH),
    non_negative: false,
};
const ANGLE: Expected = Expected {
    dimension: Some(Dimension::ANGLE),
    non_negative: false,
};
const PLAIN: Expected = Expected {
    dimension: Some(Dimension::NONE),
    non_negative: false,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GearKind {
    #[default]
    Gear,
    Pair,
    Sprocket,
}

impl GearKind {
    pub const ALL: [Self; 3] = [Self::Gear, Self::Pair, Self::Sprocket];

    pub fn label(self) -> &'static str {
        match self {
            Self::Gear => "Single gear",
            Self::Pair => "Gear pair",
            Self::Sprocket => "Sprocket",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Gear => "One involute spur gear",
            Self::Pair => {
                "Two meshing gears at the right centre distance, the second to the right of the first"
            }
            Self::Sprocket => "A sprocket for ISO 606 roller chain",
        }
    }

    pub fn fields(self) -> &'static [GearField] {
        match self {
            Self::Gear => &[
                GearField::Module,
                GearField::Teeth,
                GearField::PressureAngle,
                GearField::ProfileShift,
                GearField::RootFillet,
                GearField::Bore,
            ],
            Self::Pair => &[
                GearField::Module,
                GearField::Teeth,
                GearField::PairTeeth,
                GearField::PressureAngle,
                GearField::ProfileShift,
                GearField::RootFillet,
                GearField::Bore,
                GearField::PairBore,
            ],
            Self::Sprocket => &[GearField::Teeth, GearField::Bore],
        }
    }

    fn teeth_range(self) -> (u32, u32) {
        match self {
            Self::Gear | Self::Pair => (MIN_TEETH, MAX_TEETH),
            Self::Sprocket => (MIN_SPROCKET_TEETH, MAX_SPROCKET_TEETH),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GearField {
    Module,
    Teeth,
    PairTeeth,
    PressureAngle,
    ProfileShift,
    RootFillet,
    Bore,
    PairBore,
}

impl GearField {
    pub const ALL: [Self; 8] = [
        Self::Module,
        Self::Teeth,
        Self::PairTeeth,
        Self::PressureAngle,
        Self::ProfileShift,
        Self::RootFillet,
        Self::Bore,
        Self::PairBore,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Module => "Module",
            Self::Teeth => "Teeth",
            Self::PairTeeth => "Meshing teeth",
            Self::PressureAngle => "Pressure angle",
            Self::ProfileShift => "Profile shift",
            Self::RootFillet => "Root fillet",
            Self::Bore => "Bore",
            Self::PairBore => "Meshing bore",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Module => "The pitch diameter over the number of teeth; meshing gears share it",
            Self::Teeth => "A whole number of teeth",
            Self::PairTeeth => "The second gear's whole number of teeth",
            Self::PressureAngle => "The angle the teeth push along, usually 20°",
            Self::ProfileShift => {
                "How far the teeth move out, in modules: positive to avoid undercut on few teeth; \
                 the second gear of a pair takes the opposite"
            }
            Self::RootFillet => "The radius rounding the root of each gap; 0 for a sharp root",
            Self::Bore => "The diameter of the hole in the middle; empty or 0 for none",
            Self::PairBore => "The diameter of the second gear's hole; empty or 0 for none",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Bore | Self::PairBore => "none",
            _ => "",
        }
    }

    fn initial(self) -> &'static str {
        match self {
            Self::Module => "2 mm",
            Self::Teeth => "20",
            Self::PairTeeth => "30",
            Self::PressureAngle => "20°",
            Self::ProfileShift => "0",
            Self::RootFillet => "0.75 mm",
            Self::Bore | Self::PairBore => "",
        }
    }

    fn expected(self) -> Expected {
        match self {
            Self::Module | Self::RootFillet | Self::Bore | Self::PairBore => LENGTH,
            Self::PressureAngle => ANGLE,
            Self::Teeth | Self::PairTeeth | Self::ProfileShift => PLAIN,
        }
    }

    fn may_be_empty(self) -> bool {
        matches!(self, Self::Bore | Self::PairBore)
    }

    fn is_count(self) -> bool {
        matches!(self, Self::Teeth | Self::PairTeeth)
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Part {
    Gear(SpurGear),
    Pair(GearPair),
    Sprocket(Sprocket),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Wheel {
    Gear(GearOutline),
    Sprocket(SprocketOutline),
}

impl Wheel {
    fn faceted(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        match self {
            Self::Gear(outline) => outline.faceted(faceting),
            Self::Sprocket(outline) => outline.faceted(faceting),
        }
    }

    fn construction(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        match self {
            Self::Gear(outline) => outline.construction(faceting),
            Self::Sprocket(outline) => outline.construction(faceting),
        }
    }
}

impl Part {
    pub fn transaction(&self) -> &'static str {
        match self {
            Self::Gear(_) => TRANSACTION,
            Self::Pair(_) => PAIR_TRANSACTION,
            Self::Sprocket(_) => SPROCKET_TRANSACTION,
        }
    }

    pub fn wheels(&self, at: Point2) -> Result<Vec<Wheel>, GearError> {
        match self {
            Self::Gear(gear) => Ok(vec![Wheel::Gear(gear.outline(at)?)]),
            Self::Pair(pair) => {
                let outlines = pair.outlines(at)?;
                Ok(vec![
                    Wheel::Gear(outlines.driver),
                    Wheel::Gear(outlines.partner),
                ])
            }
            Self::Sprocket(sprocket) => Ok(vec![Wheel::Sprocket(sprocket.outline(at)?)]),
        }
    }

    pub fn drawn_into(&self, sketch: &mut Sketch, centre: GearCentre) -> Result<(), GearError> {
        match self {
            Self::Gear(gear) => sketch.add_gear(gear, centre).map(|_| ()),
            Self::Pair(pair) => sketch.add_gear_pair(pair, centre).map(|_| ()),
            Self::Sprocket(sprocket) => sketch.add_sprocket(sprocket, centre).map(|_| ()),
        }
    }

    fn description(&self, unit: Units, wheels: &[Wheel]) -> String {
        let tip_across = |wheel: Option<&Wheel>| match wheel {
            Some(Wheel::Gear(outline)) => 2.0 * outline.circles.tip,
            Some(Wheel::Sprocket(outline)) => 2.0 * outline.circles.tip,
            None => 0.0,
        };
        match self {
            Self::Gear(gear) => format!(
                "Spur gear of {} teeth, module {}, {} across the tips",
                gear.teeth,
                length_text(unit, gear.module),
                length_text(unit, tip_across(wheels.first()))
            ),
            Self::Pair(pair) => format!(
                "Gear pair of {} and {} teeth, module {}, centres {} apart",
                pair.driver.teeth,
                pair.partner_teeth,
                length_text(unit, pair.driver.module),
                length_text(unit, pair.centre_distance())
            ),
            Self::Sprocket(sprocket) => format!(
                "Sprocket of {} teeth for {} chain, {} across the tips",
                sprocket.teeth,
                sprocket.chain.designation,
                length_text(unit, tip_across(wheels.first()))
            ),
        }
    }
}

pub fn chain_label(size: ChainSize, unit: Units) -> String {
    format!(
        "{} · {} pitch",
        size.designation,
        length_text(unit, size.pitch)
    )
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum FieldProblem {
    #[error(transparent)]
    Value(crate::patterning::PatternTextError),
    #[error("{label}: Use a whole number from {least} to {most}")]
    WholeNumber {
        label: String,
        least: u32,
        most: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum PartProblem {
    #[error("Mend the {} first", .field.label().to_lowercase())]
    MendFirst { field: GearField },
    #[error("{}", capitalized(&.0.to_string()))]
    Unbuildable(GearError),
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum GearingError {
    #[error(transparent)]
    SketchGone(#[from] SketchGone),
    #[error("{}: {source}.", Tool::Gear.label())]
    Refused { source: GearError },
    #[error("{}: {source}.", Tool::Gear.label())]
    Unusable { source: PartProblem },
    #[error("{}: its values are not read yet.", Tool::Gear.label())]
    NotRead,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GearSettings {
    kind: GearKind,
    chain: ChainSize,
    texts: [String; 8],
    evaluated: Option<Result<Part, PartProblem>>,
}

impl Default for GearSettings {
    fn default() -> Self {
        Self {
            kind: GearKind::default(),
            chain: ChainSize::DEFAULT,
            texts: GearField::ALL.map(|field| field.initial().to_owned()),
            evaluated: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub problems: Vec<(GearField, FieldProblem)>,
    pub part: Result<Part, PartProblem>,
}

impl GearSettings {
    pub fn kind(&self) -> GearKind {
        self.kind
    }

    pub fn set_kind(&mut self, kind: GearKind) {
        self.kind = kind;
        self.evaluated = None;
    }

    pub fn chain(&self) -> ChainSize {
        self.chain
    }

    pub fn set_chain(&mut self, chain: ChainSize) {
        self.chain = chain;
        self.evaluated = None;
    }

    pub fn text(&self, field: GearField) -> &str {
        self.texts.get(field.index()).map_or("", String::as_str)
    }

    pub fn text_mut(&mut self, field: GearField) -> Option<&mut String> {
        self.texts.get_mut(field.index())
    }

    pub fn part(&self) -> Option<&Result<Part, PartProblem>> {
        self.evaluated.as_ref()
    }

    pub fn evaluate(&mut self, model: &Model) -> Evaluation {
        let kind = self.kind;
        let mut problems = Vec::new();
        let mut values = [0.0; 8];
        for field in kind.fields() {
            match value_of(model, kind, *field, self.text(*field)) {
                Ok(value) => {
                    if let Some(slot) = values.get_mut(field.index()) {
                        *slot = value;
                    }
                }
                Err(problem) => problems.push((*field, problem)),
            }
        }
        let value = |field: GearField| values.get(field.index()).copied().unwrap_or_default();
        let part = match problems.first() {
            Some((field, _)) => Err(PartProblem::MendFirst { field: *field }),
            None => {
                let gear = SpurGear {
                    module: value(GearField::Module),
                    teeth: value(GearField::Teeth) as u32,
                    pressure_angle: value(GearField::PressureAngle),
                    profile_shift: value(GearField::ProfileShift),
                    root_fillet: value(GearField::RootFillet),
                    bore: value(GearField::Bore),
                };
                let part = match kind {
                    GearKind::Gear => Part::Gear(gear),
                    GearKind::Pair => Part::Pair(GearPair {
                        driver: gear,
                        partner_teeth: value(GearField::PairTeeth) as u32,
                        partner_bore: value(GearField::PairBore),
                    }),
                    GearKind::Sprocket => Part::Sprocket(Sprocket {
                        chain: self.chain,
                        teeth: gear.teeth,
                        bore: gear.bore,
                    }),
                };
                part.wheels(Point2::ZERO)
                    .map(|_| part)
                    .map_err(PartProblem::Unbuildable)
            }
        };
        self.evaluated = Some(part.clone());
        Evaluation { problems, part }
    }
}

fn value_of(
    model: &Model,
    kind: GearKind,
    field: GearField,
    text: &str,
) -> Result<f64, FieldProblem> {
    if field.may_be_empty() && text.trim().is_empty() {
        return Ok(0.0);
    }
    let label = field.label().to_lowercase();
    let value = patterning::quantity(model, text, field.expected(), &label)
        .map_err(FieldProblem::Value)?
        .value;
    if !field.is_count() {
        return Ok(value);
    }
    let (least, most) = kind.teeth_range();
    let rounded = value.round();
    if (value - rounded).abs() > WHOLE_NUMBER_TOLERANCE
        || !(f64::from(least)..=f64::from(most)).contains(&rounded)
    {
        return Err(FieldProblem::WholeNumber { label, least, most });
    }
    Ok(rounded)
}

pub fn lone_point(sketch: &Sketch, selected: &[EntityId]) -> Option<EntityId> {
    match selected {
        [only] if matches!(sketch.entity(*only), Some(Entity::Point(_))) => Some(*only),
        _ => None,
    }
}

pub fn centre_words(sketch: &Sketch, centre: Option<EntityId>) -> String {
    match centre {
        Some(point) => sketch.entity_label(point),
        None => "the origin".to_owned(),
    }
}

pub fn draw(
    model: &Model,
    feature: FeatureId,
    part: &Part,
    centre: GearCentre,
) -> Result<Transaction, GearingError> {
    trimming::reshape(model, feature, part.transaction().to_owned(), |sketch| {
        part.drawn_into(sketch, centre)
            .map_err(|source| GearingError::Refused { source })
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Aim {
    Point(EntityId, Point2),
    Free(Point2),
}

impl Aim {
    fn at(self) -> Point2 {
        match self {
            Self::Point(_, at) | Self::Free(at) => at,
        }
    }

    fn centre(self) -> GearCentre {
        match self {
            Self::Point(point, _) => GearCentre::Point(point),
            Self::Free(at) => GearCentre::Free(at),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Gearing {
    selected: Option<EntityId>,
    hover: Option<Aim>,
    highlight: Option<EntityId>,
    shown: Option<(Aim, Part, Result<Vec<Wheel>, PartProblem>)>,
}

impl Gearing {
    pub fn starting(sketch: &Sketch, selected: &[EntityId]) -> Self {
        Self {
            selected: lone_point(sketch, selected),
            ..Self::default()
        }
    }

    pub fn sync(&mut self, sketch: &Sketch) {
        let exists = |point: &EntityId| is_centre(sketch, *point);
        self.selected = self.selected.filter(exists);
        self.highlight = self.highlight.filter(exists);
    }

    pub fn hover(
        &mut self,
        sketch: &Sketch,
        screen: &impl Screen,
        pointer: Option<Pointer>,
        part: Option<&Result<Part, PartProblem>>,
    ) {
        self.hover = pointer.map(|pointer| match point_under(sketch, screen, pointer) {
            Some(point) => Aim::Point(point, sketch.point(point).unwrap_or(pointer.sketch)),
            None => Aim::Free(pointer.sketch),
        });
        self.refresh(sketch, part);
    }

    pub fn leave(&mut self) {
        self.hover = None;
        if self.highlight.is_none() {
            self.shown = None;
        }
    }

    fn aim(&self, sketch: &Sketch) -> Option<Aim> {
        self.highlight
            .and_then(|point| Some(Aim::Point(point, sketch.point(point)?)))
            .or(self.hover)
    }

    fn refresh(&mut self, sketch: &Sketch, part: Option<&Result<Part, PartProblem>>) {
        let (Some(aim), Some(Ok(part))) = (self.aim(sketch), part) else {
            self.shown = None;
            return;
        };
        let fresh =
            matches!(&self.shown, Some((shown, drawn, _)) if *shown == aim && drawn == part);
        if !fresh {
            let wheels = part.wheels(aim.at()).map_err(PartProblem::Unbuildable);
            self.shown = Some((aim, *part, wheels));
        }
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let mut preview = Preview::default();
        if let Some((aim, _, Ok(wheels))) = &self.shown {
            preview.curves = wheels
                .iter()
                .flat_map(|wheel| wheel.faceted(faceting))
                .collect();
            preview.guides = wheels
                .iter()
                .flat_map(|wheel| wheel.construction(faceting))
                .flat_map(|circle| {
                    circle
                        .windows(2)
                        .filter_map(|pair| match pair {
                            [from, to] => Some([*from, *to]),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            preview.points = vec![aim.at()];
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        let aimed = match self.shown {
            Some((Aim::Point(point, _), ..)) => Some(point),
            _ => self.highlight,
        };
        aimed
            .or(self.selected)
            .filter(|point| !point.is_reference())
            .into_iter()
            .collect()
    }

    pub fn label(
        &self,
        sketch: &Sketch,
        unit: Units,
        part: Option<&Result<Part, PartProblem>>,
    ) -> Option<String> {
        if let Some(Err(problem)) = part {
            return Some(problem.to_string());
        }
        let (aim, part, wheels) = self.shown.as_ref()?;
        Some(match wheels {
            Ok(wheels) => {
                let place = match aim {
                    Aim::Point(point, _) => format!(" centred on {}", sketch.entity_label(*point)),
                    Aim::Free(_) => String::new(),
                };
                format!("{}{place}", part.description(unit, wheels))
            }
            Err(problem) => problem.to_string(),
        })
    }

    pub fn prompt(&self) -> Prompt {
        Prompt {
            text: PROMPT,
            hint: Hint::Keys(KEYS),
        }
    }

    pub fn click(
        &mut self,
        model: &Model,
        feature: FeatureId,
        part: Option<&Result<Part, PartProblem>>,
    ) -> Outcome {
        match self.hover {
            Some(aim) => self.draw_at(model, feature, part, aim.centre()),
            None => Outcome::Nothing,
        }
    }

    pub fn finish(
        &mut self,
        model: &Model,
        feature: FeatureId,
        part: Option<&Result<Part, PartProblem>>,
    ) -> Outcome {
        let centre = self.highlight.or(self.selected).unwrap_or(EntityId::ORIGIN);
        self.highlight = None;
        self.draw_at(model, feature, part, GearCentre::Point(centre))
    }

    pub fn activate(
        &mut self,
        model: &Model,
        feature: FeatureId,
        part: Option<&Result<Part, PartProblem>>,
    ) -> Outcome {
        match self.highlight.take() {
            Some(point) => self.draw_at(model, feature, part, GearCentre::Point(point)),
            None => Outcome::Nothing,
        }
    }

    fn draw_at(
        &mut self,
        model: &Model,
        feature: FeatureId,
        part: Option<&Result<Part, PartProblem>>,
        centre: GearCentre,
    ) -> Outcome {
        let drawn = match part {
            Some(Ok(part)) => draw(model, feature, part, centre),
            Some(Err(source)) => Err(GearingError::Unusable {
                source: source.clone(),
            }),
            None => Err(GearingError::NotRead),
        };
        if drawn.is_ok() {
            self.shown = None;
        }
        drawn.into()
    }

    pub fn steppable(&self) -> Result<(), StepError> {
        Ok(())
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let points: Vec<EntityId> = std::iter::once(EntityId::ORIGIN)
            .chain(
                sketch
                    .entities()
                    .filter(|(_, entity)| matches!(entity, Entity::Point(_)))
                    .map(|(id, _)| id),
            )
            .collect();
        let total = points.len() as isize;
        let current = self
            .highlight
            .and_then(|aim| points.iter().position(|point| *point == aim));
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(total),
            None if step < 0 => total - 1,
            None => 0,
        };
        self.highlight = usize::try_from(next)
            .ok()
            .and_then(|next| points.get(next).copied());
    }

    pub fn highlight_needed(&self) -> Result<(), HighlightNeeded> {
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(HighlightNeeded::Point),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some()
    }

    pub fn back_out(&mut self) {
        self.highlight = None;
    }
}

fn is_centre(sketch: &Sketch, point: EntityId) -> bool {
    point == EntityId::ORIGIN || matches!(sketch.entity(point), Some(Entity::Point(_)))
}

fn point_under(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Option<EntityId> {
    sketch
        .entities()
        .filter_map(|(id, entity)| match entity {
            Entity::Point(position) => Some((id, *position)),
            Entity::Line { .. }
            | Entity::Circle { .. }
            | Entity::Arc { .. }
            | Entity::Spline { .. }
            | Entity::Ellipse { .. }
            | Entity::EllipticalArc { .. } => None,
        })
        .chain([(EntityId::ORIGIN, Point2::ZERO)])
        .filter_map(|(id, position)| {
            let offset = screen.to_screen(position)?.distance(pointer.screen);
            (offset <= snap::POINT_TOLERANCE).then_some((offset, id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}
