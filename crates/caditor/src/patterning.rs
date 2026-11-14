use caditor_document::{FeatureId, Transaction};
use caditor_expression::{BinaryOperator, Dimension, EvalError, Expression, Unit};
use caditor_geometry::Point2;
use caditor_sketch::{
    CircularPattern, Dimensioned, Entity, EntityId, Faceting, PatternError, PatternImage,
    PatternRow, RectangularPattern, Sketch, Spread,
};

use crate::{
    drawing::Preview,
    editing::Tool,
    feature_tree::count,
    field::{self, Expected, ExpressionError},
    model::Model,
    modifying::{HighlightNeeded, Hint, Outcome, Prompt, Refusal, StepError},
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
pub const CENTRE_KEYS: &str =
    "Click a point, or Highlight the next item in the view and press Space   Esc: back to Select";
pub const GATHERED_CENTRE_KEYS: &str = "Click a point, or Highlight the next item in the view and press Space   Esc: change what is repeated";
const CENTRE_FIRST: &str = "click the point to repeat the selection about first";
const VALUES_FIRST: &str = "mend the values in the panel first";
const QUARTER_TURN_DEGREES: f64 = 90.0;
const FULL_TURN_DEGREES: f64 = 360.0;
const WHOLE_NUMBER_TOLERANCE: f64 = 1e-9;
const FULL_TURN_TOLERANCE: f64 = 1e-9;
const SINGLE_ROW: usize = 1;
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

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
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

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternField {
    Count,
    Spacing,
    Angle,
    Rows,
    RowSpacing,
    RowAngle,
    Copies,
    TotalAngle,
}

impl PatternField {
    pub const RECTANGULAR: [Self; 4] = [Self::Count, Self::Spacing, Self::Angle, Self::Rows];
    pub const ROWS: [Self; 2] = [Self::RowSpacing, Self::RowAngle];
    pub const CIRCULAR: [Self; 2] = [Self::Copies, Self::TotalAngle];
    const ALL: [Self; 8] = [
        Self::Count,
        Self::Spacing,
        Self::Angle,
        Self::Rows,
        Self::RowSpacing,
        Self::RowAngle,
        Self::Copies,
        Self::TotalAngle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Count | Self::Copies => "Count",
            Self::Spacing => "Spacing",
            Self::Angle => "Angle",
            Self::Rows => "Rows",
            Self::RowSpacing => "Row spacing",
            Self::RowAngle => "Row angle",
            Self::TotalAngle => "Total angle",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Spacing => "spacing",
            Self::Angle => "angle",
            Self::Rows => "rows",
            Self::RowSpacing => "row-spacing",
            Self::RowAngle => "row-angle",
            Self::Copies => "copies",
            Self::TotalAngle => "total-angle",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Count => "How many along the row, the original included",
            Self::Spacing => "How far apart the copies along the row are",
            Self::Angle => "Which way the row runs, from the sketch's horizontal axis",
            Self::Rows => "How many rows, the original's included; 1 makes a single row",
            Self::RowSpacing => "How far apart the rows are",
            Self::RowAngle => {
                "Which way the rows step, from the sketch's horizontal axis; left empty, square \
                 to the row"
            }
            Self::Copies => "How many in all, the original included",
            Self::TotalAngle => {
                "360° spaces the copies evenly over a full turn; a smaller angle runs from the \
                 original to the last copy"
            }
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::RowAngle => "Square to the row",
            Self::Count
            | Self::Spacing
            | Self::Angle
            | Self::Rows
            | Self::RowSpacing
            | Self::Copies
            | Self::TotalAngle => "",
        }
    }

    fn initial(self) -> &'static str {
        match self {
            Self::Count => "3",
            Self::Spacing | Self::RowSpacing => "10 mm",
            Self::Angle => "0°",
            Self::Rows => "1",
            Self::RowAngle => "",
            Self::Copies => "6",
            Self::TotalAngle => "360°",
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or_default()
    }

    fn noun(self) -> String {
        self.label().to_lowercase()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SpecProblem {
    #[error("Mend the {} first", .field.noun())]
    MendFirst { field: PatternField },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternEvaluation {
    pub problems: Vec<(PatternField, PatternTextError)>,
    pub spec: Result<Spec, SpecProblem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternSettings {
    texts: [String; 8],
    evaluated: Option<Spec>,
}

impl Default for PatternSettings {
    fn default() -> Self {
        Self {
            texts: PatternField::ALL.map(|field| field.initial().to_owned()),
            evaluated: None,
        }
    }
}

impl PatternSettings {
    pub fn text(&self, field: PatternField) -> &str {
        self.texts.get(field.index()).map_or("", String::as_str)
    }

    pub fn text_mut(&mut self, field: PatternField) -> Option<&mut String> {
        self.texts.get_mut(field.index())
    }

    pub fn spec(&self, kind: PatternKind) -> Option<&Spec> {
        self.evaluated.as_ref().filter(|spec| spec.kind() == kind)
    }

    pub fn shows_rows(&self, model: &Model) -> bool {
        whole_count(model, self.text(PatternField::Rows), "rows")
            .is_ok_and(|rows| rows != SINGLE_ROW)
    }

    pub fn evaluate(&mut self, model: &Model, kind: PatternKind) -> PatternEvaluation {
        let mut problems = Vec::new();
        let spec = match kind {
            PatternKind::Rectangular => self.rectangular(model, &mut problems),
            PatternKind::Circular => self.circular(model, &mut problems),
        };
        let spec = match (problems.first(), spec) {
            (None, Some(spec)) => Ok(spec),
            (Some((field, _)), _) => Err(SpecProblem::MendFirst { field: *field }),
            (None, None) => Err(SpecProblem::MendFirst {
                field: match kind {
                    PatternKind::Rectangular => PatternField::Count,
                    PatternKind::Circular => PatternField::Copies,
                },
            }),
        };
        self.evaluated = spec.as_ref().ok().cloned();
        PatternEvaluation { problems, spec }
    }

    fn rectangular(
        &self,
        model: &Model,
        problems: &mut Vec<(PatternField, PatternTextError)>,
    ) -> Option<Spec> {
        let count = noted(
            problems,
            PatternField::Count,
            whole_count(model, self.text(PatternField::Count), "count"),
        );
        let spacing = noted(
            problems,
            PatternField::Spacing,
            quantity(model, self.text(PatternField::Spacing), SPACING, "spacing"),
        );
        let angle = noted(
            problems,
            PatternField::Angle,
            degrees(model, self.text(PatternField::Angle)),
        );
        let rows = noted(
            problems,
            PatternField::Rows,
            whole_count(model, self.text(PatternField::Rows), "rows"),
        );
        let second = match rows {
            Some(SINGLE_ROW) => Some(None),
            Some(rows) => {
                let spacing = noted(
                    problems,
                    PatternField::RowSpacing,
                    quantity(
                        model,
                        self.text(PatternField::RowSpacing),
                        SPACING,
                        "row spacing",
                    ),
                );
                let typed = self.text(PatternField::RowAngle);
                let angle = match (typed.trim().is_empty(), &angle) {
                    (true, Some(first)) => Some(square_to(first)),
                    (true, None) => None,
                    (false, _) => noted(problems, PatternField::RowAngle, degrees(model, typed)),
                };
                spacing.zip(angle).map(|(spacing, angle)| {
                    Some(PatternRow {
                        count: rows,
                        spacing,
                        angle,
                    })
                })
            }
            None => None,
        };
        let first = PatternRow {
            count: count?,
            spacing: spacing?,
            angle: angle?,
        };
        Some(Spec::Rectangular(RectangularPattern {
            first,
            second: second?,
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
            whole_count(model, self.text(PatternField::Copies), "count"),
        );
        let total = noted(
            problems,
            PatternField::TotalAngle,
            degrees(model, self.text(PatternField::TotalAngle)),
        );
        let total = total?;
        let full = (total.value.abs() - FULL_TURN_DEGREES).abs() <= FULL_TURN_TOLERANCE;
        Some(Spec::Circular(CircularPattern {
            count: count?,
            spread: if full {
                Spread::FullTurn
            } else {
                Spread::Total(total)
            },
        }))
    }
}

#[derive(Debug, Clone, PartialEq)]
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
            Self::Rectangular(pattern) => {
                let rows = pattern.second.as_ref().map_or(1, |row| row.count);
                pattern.first.count.saturating_mul(rows).saturating_sub(1)
            }
            Self::Circular(pattern) => pattern.count.saturating_sub(1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Patterning {
    kind: PatternKind,
    selected: Vec<EntityId>,
    gathering: bool,
    gathered: bool,
    selected_centre: Option<EntityId>,
    chosen: Option<EntityId>,
    hover: Option<EntityId>,
    highlight: Option<EntityId>,
    spec: Option<Spec>,
    image: Option<Result<PatternImage, PatternError>>,
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
            spec: None,
            image: None,
        }
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
        self.kind == PatternKind::Rectangular || self.gathering
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
        self.highlight = self.highlight.filter(exists);
    }

    pub fn hover(
        &mut self,
        sketch: &Sketch,
        screen: &impl Screen,
        pointer: Option<Pointer>,
        faceting: Faceting,
    ) {
        self.hover = match self.kind {
            PatternKind::Rectangular => None,
            PatternKind::Circular if self.gathering => None,
            PatternKind::Circular => {
                pointer.and_then(|pointer| point_under(sketch, screen, pointer))
            }
        };
        self.image = self.spec.as_ref().and_then(|spec| {
            if self.selected.is_empty() {
                return None;
            }
            match spec {
                Spec::Rectangular(pattern) => {
                    Some(sketch.rectangular_image(&self.selected, pattern, faceting))
                }
                Spec::Circular(pattern) => {
                    Some(sketch.circular_image(&self.selected, self.centre()?, pattern, faceting))
                }
            }
        });
    }

    pub fn leave(&mut self) {
        self.hover = None;
        if self.kind == PatternKind::Circular && self.centre().is_none() {
            self.image = None;
        }
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
            preview.curves = image.curves.clone();
            preview.points = image.points.clone();
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        self.centre()
            .into_iter()
            .filter(|point| !point.is_reference())
            .collect()
    }

    pub fn label(&self, sketch: &Sketch) -> Option<String> {
        let centre = self.centre();
        let items = self.repeated();
        match (self.spec.as_ref()?, &self.image) {
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
            (PatternKind::Circular, false) => match (self.centre(), self.gathered) {
                (None, true) => (CENTRE_PROMPT, GATHERED_CENTRE_KEYS),
                (None, false) => (CENTRE_PROMPT, CENTRE_KEYS),
                (Some(_), _) => (CIRCULAR_PROMPT, CIRCULAR_KEYS),
            },
        };
        Prompt {
            text,
            hint: Hint::Keys(keys),
        }
    }

    pub fn click(&mut self) -> Outcome {
        if let Some(point) = self.hover.filter(|_| self.kind == PatternKind::Circular) {
            self.chosen = Some(point);
            self.highlight = None;
        }
        Outcome::Nothing
    }

    pub fn finish(&mut self, model: &Model, feature: FeatureId) -> Outcome {
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
        if let Some(point) = self.highlight.take() {
            self.chosen = Some(point);
        }
        Outcome::Nothing
    }

    pub fn make(&self, model: &Model, feature: FeatureId) -> Result<Transaction, PatterningError> {
        let tool = self.kind.tool();
        let spec = self
            .spec
            .clone()
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
        self.kind == PatternKind::Circular && !self.gathering
    }

    pub fn steppable(&self) -> Result<(), StepError> {
        Ok(())
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        if self.kind != PatternKind::Circular {
            return;
        }
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
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(HighlightNeeded::Point),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some() || self.chosen.is_some() || self.regathers()
    }

    fn regathers(&self) -> bool {
        self.kind == PatternKind::Circular && self.gathered && !self.gathering
    }

    pub fn back_out(&mut self) {
        if self.highlight.take().is_some() {
            return;
        }
        if self.chosen.take().is_none() && self.regathers() {
            self.gathering = true;
            self.image = None;
        }
    }
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
    let angle = quantity(model, text, ANGLE, "angle")?;
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
