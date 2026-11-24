use std::collections::BTreeSet;

use caditor_document::{FeatureId, Transaction};
use caditor_expression::{BinaryOperator, Dimension, EvalError, Expression, Quantity, Unit};
use caditor_geometry::Point2;
use caditor_sketch::{
    CircularPattern, CopyImage, Dimensioned, Distribution, Entity, EntityId, Faceting, Heading,
    PatternCopy, PatternError, PatternImage, PatternRow, RectangularPattern, Sides, Sketch, Spread,
};

use crate::{
    drawing::Preview,
    editing::Tool,
    feature_tree::count,
    field::{self, Expected, ExpressionError},
    mirroring,
    model::Model,
    modifying::{HighlightNeeded, Hint, Outcome, Prompt, Refusal, StepError},
    move_manipulator::segment_distance,
    snap::{self, Pointer, Screen},
    trimming::{self, capitalized},
    units::attach_unit,
};

pub const TRANSACTION: &str = "Pattern geometry";
pub const RECTANGULAR_SELECT_FIRST: &str = "Select what to repeat: click it or drag a box around \
                                            it, then set how many and how far apart in the panel";
pub const CIRCULAR_SELECT_FIRST: &str =
    "Select what to repeat: click it or drag a box around it, then press Enter";
pub const CIRCULAR_SELECT_MORE: &str = "Click or drag a box to add to what is repeated, then press Enter to choose the point to repeat it about";
pub const RECTANGULAR_PROMPT: &str =
    "Set how many and how far apart in the panel, then press Enter to make the copies";
pub const CIRCULAR_PROMPT: &str =
    "Set how many and over what angle in the panel, then press Enter to make the copies";
pub const CENTRE_PROMPT: &str = "Click the point to repeat the selection about";
pub const RECTANGULAR_GATHER_KEYS: &str =
    "Click: add or remove   Drag: add what the box takes   Esc: back to Select";
pub const CIRCULAR_GATHER_KEYS: &str = "Click: add or remove   Drag: add what the box takes   Enter: choose the point to repeat about   Esc: back to Select";
pub const CIRCULAR_GATHERED_KEYS: &str = "Click: add or remove   Drag: add what the box takes   Enter: choose the point to repeat about   Esc: let go of the selection";
pub const RECTANGULAR_KEYS: &str =
    "Enter: make the copies   Click or drag a box: change what is repeated   Esc: back to Select";
pub const RECTANGULAR_GATHERED_KEYS: &str = "Enter: make the copies   Click or drag a box: change what is repeated   Esc: let go of the selection";
pub const CIRCULAR_KEYS: &str =
    "Enter: make the copies   Click another point: repeat about it   Esc: choose another point";
pub const GATHERED_CENTRE_KEYS: &str = "Click a point, or Highlight the next item in the view and press Space   Esc: change what is repeated";
pub const DIRECTION_PROMPT: &str = "Click the line or axis this direction runs along";
pub const DIRECTION_KEYS: &str =
    "Click a line, or Highlight the next item in the view and press Space   Esc: stop choosing";
const CENTRE_FIRST: &str = "click the point to repeat the selection about first";
const VALUES_FIRST: &str = "mend the values in the panel first";
const QUARTER_TURN_DEGREES: f64 = 90.0;
const FULL_TURN_DEGREES: f64 = 360.0;
const WHOLE_NUMBER_TOLERANCE: f64 = 1e-9;
const FULL_TURN_TOLERANCE: f64 = 1e-9;
const COUNT: Expected = Expected {
    dimension: Some(Dimension::NONE),
    non_negative: false,
};
const SPACING: Expected = Expected {
    dimension: Some(Dimension::LENGTH),
    non_negative: false,
};
const ANGLE: Expected = Expected {
    dimension: Some(Dimension::ANGLE),
    non_negative: false,
};

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PatternTextError {
    #[error("The {label} is missing")]
    MissingValue { label: String },
    #[error("{label}: {source}")]
    Unreadable {
        label: String,
        source: ExpressionError,
    },
    #[error("{label}: {}", field::sentence(&.source.to_string()))]
    Unevaluable { label: String, source: EvalError },
    #[error("{label}: Use a whole number such as 4")]
    NotWholeCount { label: String },
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PatterningError {
    #[error(transparent)]
    SketchGone(#[from] trimming::SketchGone),
    #[error("{}: {source}.", .tool.label())]
    Pattern { tool: Tool, source: PatternError },
    #[error("{}: {CENTRE_FIRST}.", .tool.label())]
    CentreFirst { tool: Tool },
    #[error("{}: {VALUES_FIRST}.", .tool.label())]
    ValuesFirst { tool: Tool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Rectangular,
    Circular,
}

impl PatternKind {
    pub const ALL: [Self; 2] = [Self::Rectangular, Self::Circular];

    pub fn tool(self) -> Tool {
        match self {
            Self::Rectangular => Tool::RectangularPattern,
            Self::Circular => Tool::CircularPattern,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangular => "Rectangular",
            Self::Circular => "Circular",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Rectangular => "Repeat in a row, or in a grid of rows",
            Self::Circular => "Repeat about a point",
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Direction(pub usize);

impl Direction {
    pub const FIRST: Self = Self(0);

    pub fn index(self) -> usize {
        self.0
    }

    fn number(self) -> usize {
        self.0.saturating_add(1)
    }

    pub fn title(self) -> String {
        format!("Direction {}", self.number())
    }

    pub fn choose_label(self) -> String {
        format!("Choose a line for direction {}", self.number())
    }

    pub fn angle_label(self) -> String {
        format!("Use an angle for direction {}", self.number())
    }

    pub fn flip_label(self) -> String {
        format!("Flip direction {}", self.number())
    }

    pub fn remove_label(self) -> String {
        format!("Remove direction {}", self.number())
    }

    fn runs(self) -> String {
        format!("Run direction {} along", self.number())
    }

    fn of(self) -> String {
        match self.0 {
            0 => String::new(),
            _ => format!(" of direction {}", self.number()),
        }
    }
}

pub const ADD_DIRECTION: &str = "Add a direction";
pub const ADD_DIRECTION_ABOUT: &str =
    "Repeat everything so far along one more direction, every combination of steps a copy";
pub const RUNS_ALONG: &str = "Runs along";
pub const SIDES_CAPTION: &str = "Sides";

pub const SIDES: [(Sides, &str, &str); 2] = [
    (
        Sides::One,
        "One side",
        "The copies run from the original one way",
    ),
    (
        Sides::Both,
        "Both sides",
        "The copies run from the original both ways, Count on each side",
    ),
];

pub const DISTRIBUTIONS: [(Distribution, &str, &str); 2] = [
    (
        Distribution::Spacing,
        "Spacing",
        "The distance is how far apart neighbouring copies are",
    ),
    (
        Distribution::Extent,
        "Extent",
        "The distance runs from the original to the last copy, the copies spread evenly over it",
    ),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TurnKind {
    #[default]
    FullTurn,
    Angle,
    Symmetric,
}

impl TurnKind {
    pub const ALL: [Self; 3] = [Self::FullTurn, Self::Angle, Self::Symmetric];

    pub fn label(self) -> &'static str {
        match self {
            Self::FullTurn => "Full",
            Self::Angle => "Angle",
            Self::Symmetric => "Symmetric",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::FullTurn => "The copies are spaced evenly over a whole turn",
            Self::Angle => {
                "The total angle runs from the original to the last copy; a negative angle turns \
                 clockwise"
            }
            Self::Symmetric => {
                "The copies turn both ways from the original, Count on each side, the angle \
                 running from the original to the last copy each way"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wording {
    distribution: Distribution,
    turn: TurnKind,
}

impl Wording {
    pub fn caption(self, field: PatternField) -> &'static str {
        match (field, self.distribution, self.turn) {
            (PatternField::Spacing(_), Distribution::Extent, _) => "Extent",
            (PatternField::TotalAngle, _, TurnKind::Symmetric) => "Angle each way",
            _ => field.label(),
        }
    }

    pub fn about(self, field: PatternField) -> &'static str {
        match (field, self.distribution, self.turn) {
            (PatternField::Spacing(_), Distribution::Extent, _) => {
                "How far from the original the last copy along this direction is"
            }
            (PatternField::TotalAngle, _, TurnKind::Symmetric) => {
                "How far each way the last copy turns from the original"
            }
            _ => field.about(),
        }
    }

    fn noun(self, field: PatternField) -> String {
        let of = field.direction().map(Direction::of).unwrap_or_default();
        format!("{}{of}", self.caption(field).to_lowercase())
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum PatternField {
    Count(Direction),
    Spacing(Direction),
    Angle(Direction),
    Copies,
    TotalAngle,
}

impl PatternField {
    pub const CIRCULAR: [Self; 2] = [Self::Copies, Self::TotalAngle];

    pub fn label(self) -> &'static str {
        match self {
            Self::Count(_) | Self::Copies => "Count",
            Self::Spacing(_) => "Spacing",
            Self::Angle(_) => "Angle",
            Self::TotalAngle => "Total angle",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Count(_) => "count",
            Self::Spacing(_) => "spacing",
            Self::Angle(_) => "angle",
            Self::Copies => "copies",
            Self::TotalAngle => "total-angle",
        }
    }

    pub fn direction(self) -> Option<Direction> {
        match self {
            Self::Count(direction) | Self::Spacing(direction) | Self::Angle(direction) => {
                Some(direction)
            }
            Self::Copies | Self::TotalAngle => None,
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Count(_) => {
                "How many along this direction, the original included; on each side when both \
                 sides are chosen"
            }
            Self::Spacing(_) => "How far apart the copies along this direction are",
            Self::Angle(Direction(0)) => {
                "Which way the copies run, from the sketch's horizontal axis"
            }
            Self::Angle(_) => {
                "Which way this direction runs, from the sketch's horizontal axis; left empty, \
                 square to direction 1"
            }
            Self::Copies => {
                "How many, the original included; on each side when the copies turn both ways"
            }
            Self::TotalAngle => "How far the last copy turns from the original",
        }
    }

    pub fn expected(self) -> Expected {
        match self {
            Self::Count(_) | Self::Copies => COUNT,
            Self::Spacing(_) => SPACING,
            Self::Angle(_) | Self::TotalAngle => ANGLE,
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Angle(Direction(0)) => "",
            Self::Angle(_) => "Square to direction 1",
            Self::Count(_) | Self::Spacing(_) | Self::Copies | Self::TotalAngle => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct DirectionSettings {
    count: String,
    spacing: String,
    angle: String,
    sides: Sides,
    reversed: bool,
}

impl DirectionSettings {
    fn initial(direction: Direction, spacing: String) -> Self {
        let (count, angle) = match direction.index() {
            0 => ("3", "0°"),
            1 => ("2", ""),
            _ => ("2", "45°"),
        };
        Self {
            count: count.to_owned(),
            spacing,
            angle: angle.to_owned(),
            sides: Sides::One,
            reversed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpecProblem {
    #[error("Mend the {} first", .wording.noun(*.field))]
    MendFirst {
        field: PatternField,
        wording: Wording,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PatternEvaluation {
    pub problems: Vec<(PatternField, PatternTextError)>,
    pub spec: Result<Spec, SpecProblem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PatternSettings {
    directions: Vec<DirectionSettings>,
    copies: String,
    total_angle: String,
    distribution: Distribution,
    turn: TurnKind,
    evaluated: Option<Spec>,
    unit_seeded: bool,
}

impl Default for PatternSettings {
    fn default() -> Self {
        Self {
            directions: vec![DirectionSettings::initial(
                Direction::FIRST,
                "10 mm".to_owned(),
            )],
            copies: "6".to_owned(),
            total_angle: "90°".to_owned(),
            distribution: Distribution::Spacing,
            turn: TurnKind::FullTurn,
            evaluated: None,
            unit_seeded: false,
        }
    }
}

impl PatternSettings {
    pub fn wording(&self) -> Wording {
        Wording {
            distribution: self.distribution,
            turn: self.turn,
        }
    }

    pub fn distribution(&self) -> Distribution {
        self.distribution
    }

    pub fn set_distribution(&mut self, distribution: Distribution) {
        self.distribution = distribution;
    }

    pub fn directions(&self) -> impl Iterator<Item = Direction> + use<> {
        (0..self.directions.len()).map(Direction)
    }

    pub fn add_direction(&mut self, model: &Model) -> Direction {
        let added = Direction(self.directions.len());
        let spacing = model.units().show(Quantity::length(10.0));

        self.directions
            .push(DirectionSettings::initial(added, spacing));
        added
    }

    pub fn remove_direction(&mut self, direction: Direction) {
        if direction != Direction::FIRST && direction.index() < self.directions.len() {
            self.directions.remove(direction.index());
        }
    }

    pub fn sides(&self, direction: Direction) -> Sides {
        self.directions
            .get(direction.index())
            .map(|kept| kept.sides)
            .unwrap_or_default()
    }

    pub fn set_sides(&mut self, direction: Direction, sides: Sides) {
        if let Some(kept) = self.directions.get_mut(direction.index()) {
            kept.sides = sides;
        }
    }

    pub fn reversed(&self, direction: Direction) -> bool {
        self.directions
            .get(direction.index())
            .is_some_and(|kept| kept.reversed)
    }

    pub fn set_reversed(&mut self, direction: Direction, reversed: bool) {
        if let Some(kept) = self.directions.get_mut(direction.index()) {
            kept.reversed = reversed;
        }
    }

    pub fn turn(&self) -> TurnKind {
        self.turn
    }

    pub fn set_turn(&mut self, turn: TurnKind) {
        self.turn = turn;
    }

    pub fn text_mut(&mut self, field: PatternField) -> Option<&mut String> {
        match field {
            PatternField::Copies => Some(&mut self.copies),
            PatternField::TotalAngle => Some(&mut self.total_angle),
            PatternField::Count(direction) => self
                .directions
                .get_mut(direction.index())
                .map(|kept| &mut kept.count),
            PatternField::Spacing(direction) => self
                .directions
                .get_mut(direction.index())
                .map(|kept| &mut kept.spacing),
            PatternField::Angle(direction) => self
                .directions
                .get_mut(direction.index())
                .map(|kept| &mut kept.angle),
        }
    }

    pub fn spec(&self, kind: PatternKind) -> Option<&Spec> {
        self.evaluated.as_ref().filter(|spec| spec.kind() == kind)
    }

    pub fn evaluate(
        &mut self,
        model: &Model,
        kind: PatternKind,
        lines: &[Option<EntityId>],
    ) -> PatternEvaluation {
        if !self.unit_seeded {
            let spacing = model.units().show(Quantity::length(10.0));

            for direction in &mut self.directions {
                if direction.spacing == "10 mm" {
                    direction.spacing = spacing.clone();
                }
            }
            self.unit_seeded = true;
        }

        let mut problems = Vec::new();

        let spec = match kind {
            PatternKind::Rectangular => self.rectangular(model, lines, &mut problems),
            PatternKind::Circular => self.circular(model, &mut problems),
        };
        let wording = self.wording();
        let spec = match (problems.first(), spec) {
            (None, Some(spec)) => Ok(spec),
            (Some((field, _)), _) => Err(SpecProblem::MendFirst {
                field: *field,
                wording,
            }),
            (None, None) => Err(SpecProblem::MendFirst {
                field: match kind {
                    PatternKind::Rectangular => PatternField::Count(Direction::FIRST),
                    PatternKind::Circular => PatternField::Copies,
                },
                wording,
            }),
        };

        self.evaluated = spec.as_ref().ok().cloned();
        PatternEvaluation { problems, spec }
    }

    pub fn shown_fields(&self, kind: PatternKind, lines: &[Option<EntityId>]) -> Vec<PatternField> {
        match kind {
            PatternKind::Rectangular => self
                .directions()
                .flat_map(|direction| {
                    let along_line = lines.get(direction.index()).copied().flatten().is_some();
                    [
                        Some(PatternField::Count(direction)),
                        Some(PatternField::Spacing(direction)),
                        (!along_line).then_some(PatternField::Angle(direction)),
                    ]
                })
                .flatten()
                .collect(),
            PatternKind::Circular => match self.turn {
                TurnKind::FullTurn => vec![PatternField::Copies],
                TurnKind::Angle | TurnKind::Symmetric => PatternField::CIRCULAR.to_vec(),
            },
        }
    }

    fn rectangular(
        &self,
        model: &Model,
        lines: &[Option<EntityId>],
        problems: &mut Vec<(PatternField, PatternTextError)>,
    ) -> Option<Spec> {
        let wording = self.wording();
        let mut first_heading: Option<Heading> = None;
        let mut rows = Vec::with_capacity(self.directions.len());

        for (index, kept) in self.directions.iter().enumerate() {
            let direction = Direction(index);
            let count = noted(
                problems,
                PatternField::Count(direction),
                whole_count(model, &kept.count, &format!("count{}", direction.of())),
            );
            let spacing = noted(
                problems,
                PatternField::Spacing(direction),
                quantity(
                    model,
                    &kept.spacing,
                    SPACING,
                    &wording.noun(PatternField::Spacing(direction)),
                ),
            );
            let line = lines.get(index).copied().flatten();
            let heading = match (line, kept.angle.trim().is_empty(), &first_heading, index) {
                (Some(line), ..) => Some(Heading::Along {
                    line,
                    square: false,
                }),
                (None, true, Some(Heading::Angle(first)), 1..) => {
                    Some(Heading::Angle(square_to(first)))
                }
                (None, true, Some(Heading::Along { line, .. }), 1..) => Some(Heading::Along {
                    line: *line,
                    square: true,
                }),
                (None, true, None, 1..) => None,
                (None, ..) => noted(
                    problems,
                    PatternField::Angle(direction),
                    named_degrees(
                        model,
                        &kept.angle,
                        &wording.noun(PatternField::Angle(direction)),
                    ),
                )
                .map(Heading::Angle),
            };

            if index == 0 {
                first_heading.clone_from(&heading);
            }
            rows.push(
                count
                    .zip(spacing)
                    .zip(heading)
                    .map(|((count, distance), heading)| PatternRow {
                        count,
                        distance,
                        heading,
                        reversed: kept.reversed,
                        sides: kept.sides,
                    }),
            );
        }

        let directions = rows.into_iter().collect::<Option<Vec<PatternRow>>>()?;
        Some(Spec::Rectangular(RectangularPattern {
            directions,
            distribution: self.distribution,
            left_out: BTreeSet::new(),
        }))
    }

    fn circular(
        &self,
        model: &Model,
        problems: &mut Vec<(PatternField, PatternTextError)>,
    ) -> Option<Spec> {
        let count = noted(
            problems,
            PatternField::Copies,
            whole_count(model, &self.copies, "count"),
        );
        let spread = match self.turn {
            TurnKind::FullTurn => Some(Spread::FullTurn),
            TurnKind::Angle | TurnKind::Symmetric => {
                let angle = noted(
                    problems,
                    PatternField::TotalAngle,
                    degrees(model, &self.total_angle),
                );
                angle.map(|angle| {
                    let full = (angle.value.abs() - FULL_TURN_DEGREES).abs() <= FULL_TURN_TOLERANCE;
                    match self.turn {
                        TurnKind::Symmetric => Spread::Symmetric(angle),
                        TurnKind::Angle | TurnKind::FullTurn if full => Spread::FullTurn,
                        TurnKind::Angle | TurnKind::FullTurn => Spread::Total(angle),
                    }
                })
            }
        };
        Some(Spec::Circular(CircularPattern {
            count: count?,
            spread: spread?,
            left_out: BTreeSet::new(),
        }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Spec {
    Rectangular(RectangularPattern),
    Circular(CircularPattern),
}

impl Spec {
    fn kind(&self) -> PatternKind {
        match self {
            Self::Rectangular(_) => PatternKind::Rectangular,
            Self::Circular(_) => PatternKind::Circular,
        }
    }

    fn extra_instances(&self) -> usize {
        match self {
            Self::Rectangular(pattern) => pattern
                .instances()
                .saturating_sub(1)
                .saturating_sub(pattern.left_out.len()),
            Self::Circular(pattern) => pattern
                .instances()
                .saturating_sub(1)
                .saturating_sub(pattern.left_out.len()),
        }
    }

    pub fn copies(&self) -> Vec<PatternCopy> {
        match self {
            Self::Rectangular(pattern) => pattern
                .copies()
                .into_iter()
                .map(PatternCopy::Steps)
                .collect(),
            Self::Circular(pattern) => pattern.turns().into_iter().map(PatternCopy::Turn).collect(),
        }
    }

    pub fn original(&self) -> PatternCopy {
        match self {
            Self::Rectangular(pattern) => PatternCopy::Steps(vec![0; pattern.directions.len()]),
            Self::Circular(_) => PatternCopy::Turn(0),
        }
    }

    fn has(&self, copy: &PatternCopy) -> bool {
        match (self, copy) {
            (Self::Rectangular(pattern), PatternCopy::Steps(steps)) => pattern.has_copy(steps),
            (Self::Circular(pattern), PatternCopy::Turn(turn)) => pattern.has_turn(*turn),
            (Self::Rectangular(_), PatternCopy::Turn(_))
            | (Self::Circular(_), PatternCopy::Steps(_)) => false,
        }
    }

    fn leaving_out(&self, left_out: &BTreeSet<PatternCopy>) -> Self {
        let mut spec = self.clone();
        let within = left_out.iter().filter(|copy| self.has(copy));

        match &mut spec {
            Self::Rectangular(pattern) => {
                pattern.left_out = within
                    .filter_map(|copy| match copy {
                        PatternCopy::Steps(steps) => Some(steps.clone()),
                        PatternCopy::Turn(_) => None,
                    })
                    .collect();
            }
            Self::Circular(pattern) => {
                pattern.left_out = within
                    .filter_map(|copy| match copy {
                        PatternCopy::Turn(turn) => Some(*turn),
                        PatternCopy::Steps(_) => None,
                    })
                    .collect();
            }
        }
        spec
    }
}

pub fn copy_name(copy: &PatternCopy) -> String {
    capitalized(&copy.to_string())
}

pub fn copy_about(copy: &PatternCopy, left_out: bool) -> String {
    let name = copy_name(copy);
    match (copy.is_original(), left_out) {
        (true, _) => format!("{name} always stays"),
        (false, false) => format!("{name} is made; click to leave it out"),
        (false, true) => format!("{name} is left out; click to make it again"),
    }
}

#[derive(Clone, Debug)]
pub struct Patterning {
    kind: PatternKind,
    selected: Vec<EntityId>,
    gathering: bool,
    gathered: bool,
    selected_centre: Option<EntityId>,
    chosen: Option<EntityId>,
    hover: Option<EntityId>,
    highlight: Option<EntityId>,
    directions: Vec<Option<EntityId>>,
    picking: Option<Direction>,
    spec: Option<Spec>,
    image: Option<Result<PatternImage, PatternError>>,
    left_out: BTreeSet<PatternCopy>,
    aimed: Option<PatternCopy>,
    panel_aimed: Option<PatternCopy>,
}

impl Patterning {
    pub fn starting(kind: PatternKind, selected: &[EntityId]) -> Self {
        Self {
            kind,
            selected: selected.to_vec(),
            gathering: selected.is_empty(),
            gathered: selected.is_empty(),
            selected_centre: None,
            chosen: None,
            hover: None,
            highlight: None,
            directions: Vec::new(),
            picking: None,
            spec: None,
            image: None,
            left_out: BTreeSet::new(),
            aimed: None,
            panel_aimed: None,
        }
    }

    pub fn set_panel_aimed(&mut self, copy: Option<PatternCopy>) {
        self.panel_aimed = copy;
    }

    pub fn is_left_out(&self, copy: &PatternCopy) -> bool {
        self.left_out.contains(copy)
    }

    pub fn left_out_of(&self, spec: &Spec) -> usize {
        self.left_out.iter().filter(|copy| spec.has(copy)).count()
    }

    pub fn toggle(&mut self, copy: &PatternCopy) {
        if copy.is_original() {
            return;
        }
        if !self.left_out.remove(copy) {
            self.left_out.insert(copy.clone());
        }
    }

    pub fn bring_back(&mut self) {
        self.left_out.clear();
    }

    pub fn leave_out_aimed(&mut self) -> bool {
        let Some(copy) = self.aimed.take() else {
            return false;
        };

        self.toggle(&copy);
        true
    }

    pub fn direction_added(&mut self) {
        self.left_out = std::mem::take(&mut self.left_out)
            .into_iter()
            .map(|copy| match copy {
                PatternCopy::Steps(mut steps) => {
                    steps.push(0);
                    PatternCopy::Steps(steps)
                }
                PatternCopy::Turn(turn) => PatternCopy::Turn(turn),
            })
            .collect();
    }

    pub fn directions(&self) -> &[Option<EntityId>] {
        &self.directions
    }

    pub fn line_of(&self, direction: Direction) -> Option<EntityId> {
        self.directions.get(direction.index()).copied().flatten()
    }

    pub fn remove_direction(&mut self, direction: Direction) {
        if direction == Direction::FIRST {
            return;
        }
        if direction.index() < self.directions.len() {
            self.directions.remove(direction.index());
        }
        self.left_out = std::mem::take(&mut self.left_out)
            .into_iter()
            .filter_map(|copy| match copy {
                PatternCopy::Steps(mut steps) => {
                    let removed = (direction.index() < steps.len())
                        .then(|| steps.remove(direction.index()))?;
                    (removed == 0).then_some(PatternCopy::Steps(steps))
                }
                PatternCopy::Turn(turn) => Some(PatternCopy::Turn(turn)),
            })
            .filter(|copy| !copy.is_original())
            .collect();
        self.picking = match self.picking {
            Some(picking) if picking == direction => None,
            Some(picking) if picking > direction => Some(Direction(picking.index() - 1)),
            other => other,
        };
    }

    pub fn picking(&self) -> Option<Direction> {
        self.picking
    }

    pub fn pick_direction(&mut self, direction: Direction) {
        self.picking = Some(direction);
        self.highlight = None;
        self.hover = None;
    }

    pub fn stop_picking(&mut self) {
        self.picking = None;
        self.highlight = None;
        self.hover = None;
    }

    pub fn use_angle(&mut self, direction: Direction) {
        if let Some(line) = self.directions.get_mut(direction.index()) {
            *line = None;
        }
        if self.picking == Some(direction) {
            self.stop_picking();
        }
    }

    fn choose_line(&mut self, line: EntityId) {
        if let Some(direction) = self.picking.take() {
            if self.directions.len() <= direction.index() {
                self.directions.resize(direction.index() + 1, None);
            }
            if let Some(kept) = self.directions.get_mut(direction.index()) {
                *kept = Some(line);
            }
        }
        self.highlight = None;
        self.hover = None;
    }

    pub fn kind(&self) -> PatternKind {
        self.kind
    }

    pub fn set_spec(&mut self, spec: Option<&Spec>) {
        self.spec = spec.filter(|spec| spec.kind() == self.kind).cloned();
    }

    pub fn repeated(&self) -> usize {
        let centre = self.centre();
        self.selected
            .iter()
            .filter(|item| Some(**item) != centre)
            .count()
    }

    pub fn chosen_centre(&self) -> Option<EntityId> {
        match self.kind {
            PatternKind::Rectangular => None,
            PatternKind::Circular => self.chosen.or(self.selected_centre),
        }
    }

    pub fn choose_centre(&mut self, point: EntityId) {
        self.chosen = Some(point);
        self.highlight = None;
        self.gathering = false;
    }

    pub fn problem(&self) -> Option<&PatternError> {
        self.image.as_ref().and_then(|image| image.as_ref().err())
    }

    pub fn gathers(&self) -> bool {
        match self.kind {
            PatternKind::Rectangular => self.picking.is_none(),
            PatternKind::Circular => self.gathering,
        }
    }

    pub fn gathered(&self) -> bool {
        self.gathered
    }

    pub fn sync(&mut self, sketch: &Sketch, selected: &[EntityId]) {
        if self.selected != selected {
            self.selected = selected.to_vec();
        }
        if selected.is_empty() {
            self.gathering = true;
            self.gathered = true;
        }
        self.selected_centre = match self.kind {
            PatternKind::Rectangular => None,
            PatternKind::Circular => lone_point(sketch, selected),
        };

        let exists = |point: &EntityId| is_centre(sketch, *point);

        self.chosen = self.chosen.filter(exists);
        for line in &mut self.directions {
            *line = line.filter(|line| mirroring::is_mirror(sketch, *line));
        }
        self.highlight = match self.picking {
            Some(_) => self
                .highlight
                .filter(|line| mirroring::is_mirror(sketch, *line)),
            None => self.highlight.filter(exists),
        };
    }

    pub fn hover(
        &mut self,
        sketch: &Sketch,
        screen: &impl Screen,
        pointer: Option<Pointer>,
        faceting: Faceting,
    ) {
        self.hover = match self.kind {
            PatternKind::Rectangular if self.picking.is_some() => {
                pointer.and_then(|pointer| mirroring::line_under(sketch, screen, pointer))
            }
            PatternKind::Rectangular => None,
            PatternKind::Circular if self.gathering => None,
            PatternKind::Circular => {
                pointer.and_then(|pointer| point_under(sketch, screen, pointer))
            }
        };
        self.image = self.applied().and_then(|spec| {
            if self.selected.is_empty() {
                return None;
            }
            match &spec {
                Spec::Rectangular(pattern) => {
                    Some(sketch.rectangular_image(&self.selected, pattern, faceting))
                }
                Spec::Circular(pattern) => {
                    Some(sketch.circular_image(&self.selected, self.centre()?, pattern, faceting))
                }
            }
        });
        self.aimed = pointer
            .filter(|_| self.aims_at_copies())
            .and_then(|pointer| self.copy_under(screen, pointer));
    }

    pub fn leave(&mut self) {
        self.hover = None;
        self.aimed = None;
        if self.kind == PatternKind::Circular && self.centre().is_none() {
            self.image = None;
        }
    }

    fn applied(&self) -> Option<Spec> {
        self.spec
            .as_ref()
            .map(|spec| spec.leaving_out(&self.left_out))
    }

    fn aims_at_copies(&self) -> bool {
        match self.kind {
            PatternKind::Rectangular => self.picking.is_none(),
            PatternKind::Circular => {
                !self.gathering
                    && self.hover.is_none()
                    && self.chosen.or(self.selected_centre).is_some()
            }
        }
    }

    fn copy_under(&self, screen: &impl Screen, pointer: Pointer) -> Option<PatternCopy> {
        let Some(Ok(image)) = &self.image else {
            return None;
        };
        image
            .copies
            .iter()
            .filter_map(|copy| Some((reach_to(copy, screen, pointer)?, copy)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, copy)| copy.copy.clone())
    }

    fn centre(&self) -> Option<EntityId> {
        self.chosen
            .or(self.highlight)
            .or(self.hover)
            .or(self.selected_centre)
    }

    pub fn preview(&self) -> Preview {
        let mut preview = Preview::default();

        if let Some(Ok(image)) = &self.image {
            let highlighted = self.aimed.as_ref().or(self.panel_aimed.as_ref());

            for copy in &image.copies {
                let (curves, points) = if copy.left_out {
                    (&mut preview.left_out, &mut preview.left_out_points)
                } else {
                    (&mut preview.curves, &mut preview.points)
                };

                curves.extend(copy.curves.iter().cloned());
                points.extend(copy.points.iter().copied());
                if highlighted == Some(&copy.copy) {
                    preview.aimed.extend(copy.curves.iter().cloned());
                    preview.aimed_points.extend(copy.points.iter().copied());
                }
            }
        }
        if self.picking.is_some()
            && let Some(axis) = self.highlight.or(self.hover).filter(|id| id.is_reference())
        {
            let span = preview
                .curves
                .iter()
                .flatten()
                .chain(preview.points.iter())
                .map(|point| point.abs().max_element())
                .fold(50.0_f64, f64::max)
                * 1.25;
            if axis == EntityId::HORIZONTAL_AXIS {
                preview
                    .guides
                    .push([Point2::new(-span, 0.0), Point2::new(span, 0.0)]);
            } else if axis == EntityId::VERTICAL_AXIS {
                preview
                    .guides
                    .push([Point2::new(0.0, -span), Point2::new(0.0, span)]);
            }
        } else if self.kind == PatternKind::Circular && self.centre() == Some(EntityId::ORIGIN) {
            preview.snap = Some(Point2::ZERO);
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        let aims = match self.picking {
            Some(_) => self
                .highlight
                .or(self.hover)
                .into_iter()
                .chain(self.directions.iter().copied().flatten())
                .collect(),
            None => self.centre().into_iter().collect::<Vec<_>>(),
        };
        aims.into_iter()
            .filter(|item| !item.is_reference())
            .collect()
    }

    pub fn label(&self, sketch: &Sketch) -> Option<String> {
        if let Some(direction) = self.picking {
            return Some(match self.highlight.or(self.hover) {
                Some(line) => format!("{} {}", direction.runs(), sketch.entity_label(line)),
                None => DIRECTION_PROMPT.to_owned(),
            });
        }

        if let Some(copy) = &self.aimed {
            return Some(aimed_words(copy, self.is_left_out(copy)));
        }

        let centre = self.centre();
        let items = self.repeated();
        match (self.applied()?, &self.image) {
            (Spec::Circular(_), None) if centre.is_none() => Some(capitalized(CENTRE_PROMPT)),
            (spec, Some(Ok(_))) => {
                let about = centre
                    .filter(|_| self.kind == PatternKind::Circular)
                    .map(|point| format!(" about {}", sketch.entity_label(point)))
                    .unwrap_or_default();
                Some(format!(
                    "Repeat {} {}{about}",
                    count(items, "item", "items"),
                    count(spec.extra_instances(), "more time", "more times"),
                ))
            }
            (_, Some(Err(error))) => Some(capitalized(&error.to_string())),
            (_, None) => None,
        }
    }

    pub fn prompt(&self) -> Prompt {
        if self.picking.is_some() {
            return Prompt {
                text: DIRECTION_PROMPT,
                hint: Hint::Keys(DIRECTION_KEYS),
            };
        }

        let (text, keys) = match (self.kind, self.selected.is_empty()) {
            (PatternKind::Rectangular, true) => (RECTANGULAR_SELECT_FIRST, RECTANGULAR_GATHER_KEYS),
            (PatternKind::Rectangular, false) if self.gathered => {
                (RECTANGULAR_PROMPT, RECTANGULAR_GATHERED_KEYS)
            }
            (PatternKind::Rectangular, false) => (RECTANGULAR_PROMPT, RECTANGULAR_KEYS),
            (PatternKind::Circular, true) => (CIRCULAR_SELECT_FIRST, CIRCULAR_GATHER_KEYS),
            (PatternKind::Circular, false) if self.gathering => {
                (CIRCULAR_SELECT_MORE, CIRCULAR_GATHERED_KEYS)
            }
            (PatternKind::Circular, false) => match self.centre() {
                None => (CENTRE_PROMPT, GATHERED_CENTRE_KEYS),
                Some(_) => (CIRCULAR_PROMPT, CIRCULAR_KEYS),
            },
        };
        Prompt {
            text,
            hint: Hint::Keys(keys),
        }
    }

    pub fn click(&mut self) -> Outcome {
        if self.picking.is_some() {
            if let Some(line) = self.hover {
                self.choose_line(line);
            }
            return Outcome::Nothing;
        }
        if let Some(point) = self.hover.filter(|_| self.kind == PatternKind::Circular) {
            self.chosen = Some(point);
            self.highlight = None;
        }
        Outcome::Nothing
    }

    pub fn finish(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        if self.picking.is_some()
            && let Some(line) = self.highlight
        {
            self.choose_line(line);
            return Outcome::Nothing;
        }
        if self.selected.is_empty() {
            return Outcome::Refused(Refusal::NothingToRepeat {
                tool: self.kind.tool(),
            });
        }
        if self.kind == PatternKind::Circular && self.gathering {
            self.gathering = false;
            return Outcome::Nothing;
        }
        if let Some(point) = self.highlight.take() {
            self.chosen = Some(point);
            return Outcome::Nothing;
        }
        self.make(model, feature).into()
    }

    pub fn activate(&mut self) -> Outcome {
        if self.picking.is_some() {
            if let Some(line) = self.highlight {
                self.choose_line(line);
            }
            return Outcome::Nothing;
        }
        if let Some(point) = self.highlight.take() {
            self.chosen = Some(point);
        }
        Outcome::Nothing
    }

    pub fn make(&self, model: &Model, feature: FeatureId) -> Result<Transaction, PatterningError> {
        let tool = self.kind.tool();
        let spec = self
            .applied()
            .ok_or(PatterningError::ValuesFirst { tool })?;
        let selected = self.selected.clone();
        let centre = self.centre();
        let reason = |source| PatterningError::Pattern { tool, source };
        trimming::reshape(model, feature, TRANSACTION.to_owned(), |sketch| {
            match (&spec, centre) {
                (Spec::Rectangular(pattern), _) => sketch
                    .rectangular_pattern(&selected, pattern)
                    .map(|_| ())
                    .map_err(reason),
                (Spec::Circular(pattern), Some(centre)) => sketch
                    .circular_pattern(&selected, centre, pattern)
                    .map(|_| ())
                    .map_err(reason),
                (Spec::Circular(_), None) => Err(PatterningError::CentreFirst { tool }),
            }
        })
    }

    pub fn steps_targets(&self) -> bool {
        (self.kind == PatternKind::Circular && !self.gathering) || self.picking.is_some()
    }

    pub fn steppable(&self) -> Result<(), StepError> {
        Ok(())
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let points: Vec<EntityId> = match (self.picking, self.kind) {
            (Some(_), _) => [EntityId::HORIZONTAL_AXIS, EntityId::VERTICAL_AXIS]
                .into_iter()
                .chain(
                    sketch
                        .entities()
                        .filter(|(_, entity)| matches!(entity, Entity::Line { .. }))
                        .map(|(id, _)| id),
                )
                .collect(),
            (None, PatternKind::Circular) => std::iter::once(EntityId::ORIGIN)
                .chain(
                    sketch
                        .entities()
                        .filter(|(_, entity)| matches!(entity, Entity::Point(_)))
                        .map(|(id, _)| id),
                )
                .collect(),
            (None, PatternKind::Rectangular) => return,
        };
        let total = points.len() as isize;
        let current = self
            .highlight
            .or(self.chosen)
            .or(self.hover)
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
        match (self.highlight, self.picking) {
            (Some(_), _) => Ok(()),
            (None, Some(_)) => Err(HighlightNeeded::Line),
            (None, None) => Err(HighlightNeeded::Point),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some()
            || self.picking.is_some()
            || self.chosen.is_some()
            || self.regathers()
    }

    fn regathers(&self) -> bool {
        self.kind == PatternKind::Circular && !self.gathering
    }

    pub fn back_out(&mut self) {
        if self.highlight.take().is_some() || self.picking.take().is_some() {
            return;
        }
        if self.chosen.take().is_none() && self.regathers() {
            self.gathering = true;
            self.gathered = true;
            self.image = None;
        }
    }
}

fn aimed_words(copy: &PatternCopy, left_out: bool) -> String {
    let name = copy.to_string();
    if left_out {
        format!("Click to make {name} again")
    } else {
        format!("Click to leave {name} out; click it again to bring it back")
    }
}

fn reach_to(copy: &CopyImage, screen: &impl Screen, pointer: Pointer) -> Option<f64> {
    let along_curves = copy
        .curves
        .iter()
        .flat_map(|curve| curve.windows(2))
        .filter_map(|pair| match pair {
            [from, to] => Some(segment_distance(
                screen.to_screen(*from)?,
                screen.to_screen(*to)?,
                pointer.screen,
            )),
            _ => None,
        })
        .filter(|offset| *offset <= snap::CURVE_TOLERANCE);
    let at_points = copy
        .points
        .iter()
        .filter_map(|point| Some(screen.to_screen(*point)?.distance(pointer.screen)))
        .filter(|offset| *offset <= snap::POINT_TOLERANCE);

    along_curves.chain(at_points).min_by(f64::total_cmp)
}

fn is_centre(sketch: &Sketch, point: EntityId) -> bool {
    point == EntityId::ORIGIN || matches!(sketch.entity(point), Some(Entity::Point(_)))
}

fn lone_point(sketch: &Sketch, selected: &[EntityId]) -> Option<EntityId> {
    let is_point = |item: &EntityId| matches!(sketch.entity(*item), Some(Entity::Point(_)));
    let used: Vec<EntityId> = selected
        .iter()
        .filter_map(|item| sketch.entity(*item))
        .flat_map(Entity::points)
        .collect();

    let mut lone = selected
        .iter()
        .copied()
        .filter(|item| is_point(item) && !used.contains(item));

    let point = lone.next()?;
    let others = selected.len() > 1;
    (lone.next().is_none() && others).then_some(point)
}

fn point_under(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Option<EntityId> {
    let points = sketch
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
        .chain([(EntityId::ORIGIN, Point2::ZERO)]);
    points
        .filter_map(|(id, position)| {
            let offset = screen.to_screen(position)?.distance(pointer.screen);
            (offset <= snap::POINT_TOLERANCE).then_some((offset, id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

fn noted<T>(
    problems: &mut Vec<(PatternField, PatternTextError)>,
    field: PatternField,
    read: Result<T, PatternTextError>,
) -> Option<T> {
    read.map_err(|problem| problems.push((field, problem))).ok()
}

fn whole_count(model: &Model, text: &str, label: &str) -> Result<usize, PatternTextError> {
    let counted = quantity(model, text, COUNT, label)?;
    let rounded = counted.value.round();

    let whole = (counted.value - rounded).abs() <= WHOLE_NUMBER_TOLERANCE;

    if !whole || rounded < 0.0 || rounded > f64::from(u32::MAX) {
        return Err(PatternTextError::NotWholeCount {
            label: label.to_owned(),
        });
    }
    Ok(rounded as usize)
}

pub(crate) fn degrees(model: &Model, text: &str) -> Result<Dimensioned, PatternTextError> {
    named_degrees(model, text, "angle")
}

fn named_degrees(model: &Model, text: &str, label: &str) -> Result<Dimensioned, PatternTextError> {
    let angle = quantity(model, text, ANGLE, label)?;
    let plain = model
        .parameters()
        .evaluate_expression(&angle.expression)
        .is_ok_and(|found| found.dimension.is_plain());
    Ok(if plain {
        Dimensioned {
            expression: attach_unit(angle.expression, Unit::Degree),
            value: angle.value,
        }
    } else {
        angle
    })
}

pub(crate) fn turned(angle: f64) -> Dimensioned {
    Dimensioned {
        expression: Expression::measure(angle, Unit::Degree),
        value: angle,
    }
}

pub(crate) fn square_to(first: &Dimensioned) -> Dimensioned {
    let value = first.value + QUARTER_TURN_DEGREES;

    if first.expression.is_literal() {
        return turned(value);
    }
    Dimensioned {
        expression: Expression::binary(
            BinaryOperator::Add,
            first.expression.clone(),
            Expression::Measure(QUARTER_TURN_DEGREES, Unit::Degree),
        ),
        value,
    }
}

pub(crate) fn quantity(
    model: &Model,
    text: &str,
    expected: Expected,
    label: &str,
) -> Result<Dimensioned, PatternTextError> {
    let text = text.trim();

    if text.is_empty() {
        return Err(PatternTextError::MissingValue {
            label: label.to_owned(),
        });
    }

    let expression = field::parse_expression(
        model.document(),
        model.parameters(),
        text,
        expected,
        model.units(),
    )
    .map_err(|source| PatternTextError::Unreadable {
        label: label.to_owned(),
        source,
    })?;
    let dimension = expected.dimension.unwrap_or(Dimension::NONE);

    let value = expression
        .evaluate_as(dimension, &|id| model.parameters().value(id))
        .map_err(|source| PatternTextError::Unevaluable {
            label: label.to_owned(),
            source,
        })?;
    Ok(Dimensioned { expression, value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_direction_defaults_to_square_to_the_first() {
        assert_eq!(square_to(&turned(30.0)).value, 120.0);

        let slanted = square_to(&Dimensioned {
            expression: Expression::Parameter(caditor_expression::ParameterId::from_raw(1)),
            value: 10.0,
        });

        assert_eq!(slanted.value, 100.0);
        assert!(!slanted.expression.is_literal());
    }
}
