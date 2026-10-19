use caditor_document::{FeatureId, Transaction};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_geometry::Point2;
use caditor_sketch::{
    ChainSize, Dimensioned, Entity, EntityId, Faceting, Gear, GearCentre, GearError, GearOutline,
    GearPair, MAX_SPROCKET_TEETH, MAX_TEETH, MIN_SPROCKET_TEETH, MIN_TEETH, Sketch, Sprocket,
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
pub const EDIT_TRANSACTION: &str = "Edit spur gear";
pub const EDIT_SPROCKET_TRANSACTION: &str = "Edit sprocket";
pub const PAIR_TRANSACTION: &str = "Draw gear pair";
pub const SPROCKET_TRANSACTION: &str = "Draw sprocket";
pub const PROMPT: &str =
    "Click where the gear's centre goes: on a point, or anywhere in the sketch";
pub const EDIT_PROMPT: &str = "Change the gear in the Spur gear panel, then update it";
const KEYS: &str = "Enter: draw it at the selected point or the origin   Esc: back to Select";
const EDIT_KEYS: &str = "Esc: back to Select";
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
    Meshing,
}

impl GearKind {
    pub const ALL: [Self; 3] = [Self::Gear, Self::Pair, Self::Sprocket];

    pub fn label(self) -> &'static str {
        match self {
            Self::Gear => "Single gear",
            Self::Pair => "Gear pair",
            Self::Sprocket => "Sprocket",
            Self::Meshing => "Meshing gear",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Gear => "One involute spur gear",
            Self::Pair => {
                "Two meshing gears at the right centre distance; the second follows the first"
            }
            Self::Sprocket => "A sprocket for ISO 606 roller chain",
            Self::Meshing => "A gear meshing with another, following its values",
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
                GearField::Turn,
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
                GearField::Turn,
                GearField::Direction,
            ],
            Self::Sprocket => &[GearField::Teeth, GearField::Bore, GearField::Turn],
            Self::Meshing => &[
                GearField::PairTeeth,
                GearField::PairBore,
                GearField::Direction,
            ],
        }
    }

    fn teeth_range(self) -> (u32, u32) {
        match self {
            Self::Gear | Self::Pair | Self::Meshing => (MIN_TEETH, MAX_TEETH),
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
    Turn,
    Direction,
}

impl GearField {
    pub const ALL: [Self; 10] = [
        Self::Module,
        Self::Teeth,
        Self::PairTeeth,
        Self::PressureAngle,
        Self::ProfileShift,
        Self::RootFillet,
        Self::Bore,
        Self::PairBore,
        Self::Turn,
        Self::Direction,
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
            Self::Turn => "Rotation",
            Self::Direction => "Direction",
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
            Self::Turn => {
                "How far the whole gear turns about its centre; at 0° a tooth points along the \
                 sketch's horizontal axis"
            }
            Self::Direction => {
                "Where the second gear's centre lies from the first's, as an angle from the \
                 sketch's horizontal axis; at 0° it is to the right"
            }
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
            Self::Turn | Self::Direction => "0°",
        }
    }

    fn expected(self) -> Expected {
        match self {
            Self::Module | Self::RootFillet | Self::Bore | Self::PairBore => LENGTH,
            Self::PressureAngle | Self::Turn | Self::Direction => ANGLE,
            Self::Teeth | Self::PairTeeth | Self::ProfileShift => PLAIN,
        }
    }

    fn may_be_empty(self) -> bool {
        matches!(self, Self::Bore | Self::PairBore)
    }

    fn empty_expression(self) -> Expression {
        Expression::measure(0.0, Unit::Millimetre)
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

#[derive(Debug, Clone, PartialEq)]
pub struct Spur {
    pub values: SpurGear,
    pub turn: f64,
    pub stored: Gear,
}

impl Spur {
    pub fn literal(values: SpurGear) -> Self {
        let stored = Gear {
            centre: EntityId::ORIGIN,
            module: Expression::measure(values.module, Unit::Millimetre),
            teeth: Expression::number(f64::from(values.teeth)),
            pressure_angle: Expression::measure(values.pressure_angle, Unit::Degree),
            profile_shift: Expression::number(values.profile_shift),
            root_fillet: Expression::measure(values.root_fillet, Unit::Millimetre),
            bore: Expression::measure(values.bore, Unit::Millimetre),
            turn: Expression::measure(0.0, Unit::Degree),
            meshing: None,
            chain: None,
        };
        Self {
            values,
            turn: 0.0,
            stored,
        }
    }

    pub fn gear_at(&self, centre: EntityId) -> Gear {
        Gear {
            centre,
            ..self.stored.clone()
        }
    }

    fn outline(&self, at: Point2) -> Result<GearOutline, GearError> {
        self.values.outline_turned(at, self.turn.to_radians())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredSprocket {
    pub values: Sprocket,
    pub turn: f64,
    pub stored: Gear,
}

impl StoredSprocket {
    pub fn literal(values: Sprocket) -> Self {
        let stored = Gear::sprocket(
            EntityId::ORIGIN,
            values.chain,
            Expression::number(f64::from(values.teeth)),
            Expression::measure(values.bore, Unit::Millimetre),
            Expression::measure(0.0, Unit::Degree),
        );
        Self {
            values,
            turn: 0.0,
            stored,
        }
    }

    pub fn gear_at(&self, centre: EntityId) -> Gear {
        Gear {
            centre,
            ..self.stored.clone()
        }
    }

    fn outline(&self, at: Point2) -> Result<SprocketOutline, GearError> {
        self.values.outline_turned(at, self.turn.to_radians())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Gear(Box<Spur>),
    Pair(Box<StoredPair>),
    Sprocket(Box<StoredSprocket>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredPair {
    pub driver: Spur,
    pub placing: GearPair,
    pub partner: Gear,
}

impl StoredPair {
    pub fn literal(placing: GearPair) -> Self {
        let partner = Gear::meshed(
            EntityId::ORIGIN,
            EntityId::ORIGIN,
            Expression::number(f64::from(placing.partner_teeth)),
            Expression::measure(placing.partner_bore, Unit::Millimetre),
            Expression::measure(placing.direction.to_degrees(), Unit::Degree),
        );
        Self {
            driver: Spur::literal(placing.driver),
            placing,
            partner,
        }
    }

    fn outlines(&self, at: Point2) -> Result<[GearOutline; 2], GearError> {
        let outlines = self
            .placing
            .outlines_turned(at, self.driver.turn.to_radians())?;
        Ok([outlines.driver, outlines.partner])
    }
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
            Self::Gear(spur) => Ok(vec![Wheel::Gear(spur.outline(at)?)]),
            Self::Pair(pair) => {
                let [driver, partner] = pair.outlines(at)?;
                Ok(vec![Wheel::Gear(driver), Wheel::Gear(partner)])
            }
            Self::Sprocket(sprocket) => Ok(vec![Wheel::Sprocket(sprocket.outline(at)?)]),
        }
    }

    pub fn drawn_into(&self, sketch: &mut Sketch, centre: GearCentre) -> Result<(), GearError> {
        match self {
            Self::Gear(spur) => {
                spur.values.check()?;
                sketch
                    .add_stored_gear(centre, spur.stored.clone())
                    .map(|_| ())
            }
            Self::Pair(pair) => sketch
                .add_stored_pair(
                    centre,
                    pair.driver.stored.clone(),
                    pair.partner.clone(),
                    &pair.placing,
                )
                .map(|_| ()),
            Self::Sprocket(sprocket) => {
                sprocket.values.check()?;
                sketch
                    .add_stored_gear(centre, sprocket.stored.clone())
                    .map(|_| ())
            }
        }
    }

    fn description(&self, unit: Units, wheels: &[Wheel]) -> String {
        let tip_across = |wheel: Option<&Wheel>| match wheel {
            Some(Wheel::Gear(outline)) => 2.0 * outline.circles.tip,
            Some(Wheel::Sprocket(outline)) => 2.0 * outline.circles.tip,
            None => 0.0,
        };
        match self {
            Self::Gear(spur) => format!(
                "Spur gear of {} teeth, module {}, {} across the tips",
                spur.values.teeth,
                length_text(unit, spur.values.module),
                length_text(unit, tip_across(wheels.first()))
            ),
            Self::Pair(pair) => format!(
                "Gear pair of {} and {} teeth, module {}, centres {} apart",
                pair.placing.driver.teeth,
                pair.placing.partner_teeth,
                length_text(unit, pair.placing.driver.module),
                length_text(unit, pair.placing.centre_distance())
            ),
            Self::Sprocket(sprocket) => format!(
                "Sprocket of {} teeth for {} chain, {} across the tips",
                sprocket.values.teeth,
                sprocket.values.chain.designation,
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
    #[error("Mend {driver}, which this gear meshes with, first")]
    DriverBroken { driver: String },
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
    #[error("{}: the gear being edited no longer exists.", Tool::Gear.label())]
    GearGone,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GearSettings {
    kind: GearKind,
    chain: ChainSize,
    texts: [String; 10],
    editing: Option<EntityId>,
    loaded: Option<EntityId>,
    evaluated: Option<Result<Part, PartProblem>>,
}

impl Default for GearSettings {
    fn default() -> Self {
        Self {
            kind: GearKind::default(),
            chain: ChainSize::DEFAULT,
            texts: GearField::ALL.map(|field| field.initial().to_owned()),
            editing: None,
            loaded: None,
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

    pub fn editing(&self) -> Option<EntityId> {
        self.editing
    }

    pub fn begin(&mut self, sketch: Option<&Sketch>, selected: &[EntityId]) {
        self.editing = match (sketch, selected) {
            (Some(sketch), [only]) if sketch.gear(*only).is_some() => Some(*only),
            _ => None,
        };
        self.loaded = None;
        let held = sketch
            .zip(self.editing)
            .and_then(|(sketch, gear)| sketch.gear(gear));
        if let Some(gear) = held {
            self.set_kind(kind_of(gear));
        }
    }

    pub fn stop_editing(&mut self) {
        self.editing = None;
        self.loaded = None;
    }

    pub fn load(&mut self, model: &Model, sketch: Option<&Sketch>) {
        let Some(editing) = self.editing else {
            return;
        };
        if self.loaded == Some(editing) {
            return;
        }
        let Some(gear) = sketch.and_then(|sketch| sketch.gear(editing)) else {
            self.stop_editing();
            return;
        };
        let document = model.document();
        let stored = |field: GearField, expression: &Expression| {
            if field.may_be_empty() && *expression == field.empty_expression() {
                String::new()
            } else {
                document.expression_text(expression)
            }
        };
        let loaded = match (&gear.meshing, gear.chain) {
            (None, Some(_)) => vec![
                (GearField::Teeth, &gear.teeth),
                (GearField::Bore, &gear.bore),
                (GearField::Turn, &gear.turn),
            ],
            (None, None) => vec![
                (GearField::Module, &gear.module),
                (GearField::Teeth, &gear.teeth),
                (GearField::PressureAngle, &gear.pressure_angle),
                (GearField::ProfileShift, &gear.profile_shift),
                (GearField::RootFillet, &gear.root_fillet),
                (GearField::Bore, &gear.bore),
                (GearField::Turn, &gear.turn),
            ],
            (Some(meshing), _) => vec![
                (GearField::PairTeeth, &gear.teeth),
                (GearField::PairBore, &gear.bore),
                (GearField::Direction, &meshing.direction),
            ],
        };
        for (field, expression) in loaded {
            if let Some(text) = self.text_mut(field) {
                *text = stored(field, expression);
            }
        }
        self.kind = kind_of(gear);
        if let Some(chain) = gear.chain {
            self.chain = chain;
        }
        self.loaded = Some(editing);
        self.evaluated = None;
    }

    pub fn evaluate(&mut self, model: &Model, sketch: Option<&Sketch>) -> Evaluation {
        let kind = self.kind;
        let mut problems = Vec::new();
        let mut read: [Option<Dimensioned>; 10] = Default::default();
        for field in kind.fields() {
            match value_of(model, kind, *field, self.text(*field)) {
                Ok(value) => {
                    if let Some(slot) = read.get_mut(field.index()) {
                        *slot = Some(value);
                    }
                }
                Err(problem) => problems.push((*field, problem)),
            }
        }
        let held = |field: GearField| {
            read.get(field.index())
                .and_then(Option::as_ref)
                .cloned()
                .unwrap_or(Dimensioned {
                    expression: Expression::number(0.0),
                    value: 0.0,
                })
        };
        let value = |field: GearField| held(field).value;
        let spur = || {
            let values = SpurGear {
                module: value(GearField::Module),
                teeth: value(GearField::Teeth) as u32,
                pressure_angle: value(GearField::PressureAngle),
                profile_shift: value(GearField::ProfileShift),
                root_fillet: value(GearField::RootFillet),
                bore: value(GearField::Bore),
            };
            let turn = held(GearField::Turn);
            Spur {
                values,
                turn: turn.value,
                stored: Gear {
                    centre: EntityId::ORIGIN,
                    module: held(GearField::Module).expression,
                    teeth: held(GearField::Teeth).expression,
                    pressure_angle: held(GearField::PressureAngle).expression,
                    profile_shift: held(GearField::ProfileShift).expression,
                    root_fillet: held(GearField::RootFillet).expression,
                    bore: held(GearField::Bore).expression,
                    turn: turn.expression,
                    meshing: None,
                    chain: None,
                },
            }
        };
        let partner = |driver: EntityId| {
            Gear::meshed(
                EntityId::ORIGIN,
                driver,
                held(GearField::PairTeeth).expression,
                held(GearField::PairBore).expression,
                held(GearField::Direction).expression,
            )
        };
        let pair_around = |driver: Spur| StoredPair {
            placing: GearPair {
                driver: driver.values,
                partner_teeth: value(GearField::PairTeeth) as u32,
                partner_bore: value(GearField::PairBore),
                direction: value(GearField::Direction).to_radians(),
            },
            partner: partner(EntityId::ORIGIN),
            driver,
        };
        let part = match problems.first() {
            Some((field, _)) => Err(PartProblem::MendFirst { field: *field }),
            None => {
                let part = match kind {
                    GearKind::Gear => Ok(Part::Gear(Box::new(spur()))),
                    GearKind::Pair => Ok(Part::Pair(Box::new(pair_around(spur())))),
                    GearKind::Sprocket => Ok(Part::Sprocket(Box::new(StoredSprocket {
                        values: Sprocket {
                            chain: self.chain,
                            teeth: value(GearField::Teeth) as u32,
                            bore: value(GearField::Bore),
                        },
                        turn: value(GearField::Turn),
                        stored: Gear::sprocket(
                            EntityId::ORIGIN,
                            self.chain,
                            held(GearField::Teeth).expression,
                            held(GearField::Bore).expression,
                            held(GearField::Turn).expression,
                        ),
                    }))),
                    GearKind::Meshing => {
                        self.driver_of_editing(model, sketch).map(|(id, driver)| {
                            let mut pair = pair_around(driver);
                            pair.partner = partner(id);
                            Part::Pair(Box::new(pair))
                        })
                    }
                };
                part.and_then(|part| {
                    part.wheels(Point2::ZERO)
                        .map(|_| part)
                        .map_err(PartProblem::Unbuildable)
                })
            }
        };
        self.evaluated = Some(part.clone());
        Evaluation { problems, part }
    }

    fn driver_of_editing(
        &self,
        model: &Model,
        sketch: Option<&Sketch>,
    ) -> Result<(EntityId, Spur), PartProblem> {
        let broken = |driver: EntityId| PartProblem::DriverBroken {
            driver: sketch.map_or_else(
                || format!("Gear {driver}"),
                |sketch| sketch.entity_label(driver),
            ),
        };
        let held = sketch.zip(self.editing).and_then(|(sketch, id)| {
            let driver = sketch.gear(id)?.driver()?;
            Some((driver, sketch.gear(driver)?))
        });
        let (id, driver) = held.ok_or(PartProblem::DriverBroken {
            driver: "the gear it meshes with".to_owned(),
        })?;
        let parameters = model.parameters();
        let values = driver
            .values(&|parameter| parameters.value(parameter))
            .map_err(|_| broken(id))?;
        let spur = values.shape.spur().copied().ok_or_else(|| broken(id))?;
        Ok((
            id,
            Spur {
                values: spur,
                turn: values.turn,
                stored: driver.clone(),
            },
        ))
    }
}

fn kind_of(gear: &Gear) -> GearKind {
    match (&gear.meshing, gear.chain) {
        (Some(_), _) => GearKind::Meshing,
        (None, Some(_)) => GearKind::Sprocket,
        (None, None) => GearKind::Gear,
    }
}

fn value_of(
    model: &Model,
    kind: GearKind,
    field: GearField,
    text: &str,
) -> Result<Dimensioned, FieldProblem> {
    if field.may_be_empty() && text.trim().is_empty() {
        return Ok(Dimensioned {
            expression: field.empty_expression(),
            value: 0.0,
        });
    }
    let label = field.label().to_lowercase();
    let quantity =
        patterning::quantity(model, text, field.expected(), &label).map_err(FieldProblem::Value)?;
    if !field.is_count() {
        return Ok(quantity);
    }
    let (least, most) = kind.teeth_range();
    let rounded = quantity.value.round();
    if (quantity.value - rounded).abs() > WHOLE_NUMBER_TOLERANCE
        || !(f64::from(least)..=f64::from(most)).contains(&rounded)
    {
        return Err(FieldProblem::WholeNumber { label, least, most });
    }
    Ok(Dimensioned {
        expression: quantity.expression,
        value: rounded,
    })
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

pub fn update(
    model: &Model,
    feature: FeatureId,
    gear: EntityId,
    spur: &Spur,
) -> Result<Transaction, GearingError> {
    let held = model
        .document()
        .feature(feature)
        .and_then(|owner| owner.kind.sketch())
        .ok_or(SketchGone)?
        .gear(gear)
        .ok_or(GearingError::GearGone)?;
    let mut transaction = model.document().transaction(EDIT_TRANSACTION);
    transaction.set_sketch_gear(feature, gear, spur.gear_at(held.centre));
    Ok(transaction.finish())
}

pub fn update_sprocket(
    model: &Model,
    feature: FeatureId,
    gear: EntityId,
    sprocket: &StoredSprocket,
) -> Result<Transaction, GearingError> {
    let held = model
        .document()
        .feature(feature)
        .and_then(|owner| owner.kind.sketch())
        .ok_or(SketchGone)?
        .gear(gear)
        .ok_or(GearingError::GearGone)?;
    let mut transaction = model.document().transaction(EDIT_SPROCKET_TRANSACTION);
    transaction.set_sketch_gear(feature, gear, sprocket.gear_at(held.centre));
    Ok(transaction.finish())
}

pub fn update_meshing(
    model: &Model,
    feature: FeatureId,
    gear: EntityId,
    pair: &StoredPair,
) -> Result<Transaction, GearingError> {
    let held = model
        .document()
        .feature(feature)
        .and_then(|owner| owner.kind.sketch())
        .ok_or(SketchGone)?
        .gear(gear)
        .ok_or(GearingError::GearGone)?;
    let mut transaction = model.document().transaction(EDIT_TRANSACTION);
    transaction.set_sketch_gear(
        feature,
        gear,
        Gear {
            centre: held.centre,
            ..pair.partner.clone()
        },
    );
    Ok(transaction.finish())
}

#[derive(Debug, Clone, Default)]
pub struct Gearing {
    selected: Option<EntityId>,
    editing: Option<EntityId>,
    hover: Option<Aim>,
    highlight: Option<EntityId>,
    shown: Option<(Aim, Part, Result<Vec<Wheel>, PartProblem>)>,
}

impl Gearing {
    pub fn starting(sketch: &Sketch, selected: &[EntityId], editing: Option<EntityId>) -> Self {
        Self {
            selected: lone_point(sketch, selected),
            editing,
            ..Self::default()
        }
    }

    pub fn sync(&mut self, sketch: &Sketch, editing: Option<EntityId>) {
        let exists = |point: &EntityId| is_centre(sketch, *point);
        self.selected = self.selected.filter(exists);
        self.highlight = self.highlight.filter(exists);
        self.editing = editing.filter(|gear| sketch.gear(*gear).is_some());
        if self.editing.is_some() {
            self.shown = None;
        }
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
        let (Some(aim), Some(Ok(part)), None) = (self.aim(sketch), part, self.editing) else {
            self.shown = None;
            return;
        };
        let fresh =
            matches!(&self.shown, Some((shown, drawn, _)) if *shown == aim && drawn == part);
        if !fresh {
            let wheels = part.wheels(aim.at()).map_err(PartProblem::Unbuildable);
            self.shown = Some((aim, part.clone(), wheels));
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
        if let Some(gear) = self.editing {
            return vec![gear];
        }
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
        if let Some(gear) = self.editing {
            return Some(format!("Editing {}", sketch.entity_label(gear)));
        }
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
        match self.editing {
            Some(_) => Prompt {
                text: EDIT_PROMPT,
                hint: Hint::Keys(EDIT_KEYS),
            },
            None => Prompt {
                text: PROMPT,
                hint: Hint::Keys(KEYS),
            },
        }
    }

    pub fn click(
        &mut self,
        model: &Model,
        feature: FeatureId,
        part: Option<&Result<Part, PartProblem>>,
    ) -> Outcome {
        match (self.hover, self.editing) {
            (Some(aim), None) => self.draw_at(model, feature, part, aim.centre()),
            _ => Outcome::Nothing,
        }
    }

    pub fn finish(
        &mut self,
        model: &Model,
        feature: FeatureId,
        part: Option<&Result<Part, PartProblem>>,
    ) -> Outcome {
        if let Some(gear) = self.editing {
            return match part {
                Some(Ok(Part::Gear(spur))) => update(model, feature, gear, spur).into(),
                Some(Ok(Part::Pair(pair))) => update_meshing(model, feature, gear, pair).into(),
                Some(Ok(Part::Sprocket(sprocket))) => {
                    update_sprocket(model, feature, gear, sprocket).into()
                }
                None => Outcome::Refused(GearingError::NotRead.into()),
                Some(Err(source)) => Outcome::Refused(
                    GearingError::Unusable {
                        source: source.clone(),
                    }
                    .into(),
                ),
            };
        }
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
        match (self.highlight.take(), self.editing) {
            (Some(point), None) => self.draw_at(model, feature, part, GearCentre::Point(point)),
            _ => Outcome::Nothing,
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
