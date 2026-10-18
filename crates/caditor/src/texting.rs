use caditor_document::{FeatureId, PlacedText, Transaction, lettered_at};
use caditor_expression::Dimension;
use caditor_geometry::Point2;
use caditor_sketch::{
    Constraint, Dimensioned, Entity, EntityId, Faceting, LINE_BREAK, Sketch, SketchError, Text,
    TextAlign, TextError, TextSize,
};
use thiserror::Error;

use crate::{
    drawing::Preview,
    editing::Tool,
    field::Expected,
    model::Model,
    modifying::{HighlightNeeded, Hint, Outcome, Prompt, StepError, length_text},
    patterning::{self, PatternTextError},
    snap::{self, Pointer, Screen},
    trimming::{self, SketchGone, capitalized},
    units::Units,
};

pub const TRANSACTION: &str = "Draw text";
pub const EDIT_TRANSACTION: &str = "Edit text";
pub const PROMPT: &str = "Click where the text is anchored, on the first line's baseline: on a \
                          point, or anywhere in the sketch";
pub const EDIT_PROMPT: &str = "Change the text in the Text panel, then update it";
const KEYS: &str = "Enter: write it at the selected point or the origin   Esc: back to Select";
const EDIT_KEYS: &str = "Esc: back to Select";
const MAX_LABEL_CHARS: usize = 24;
const LINE_SEPARATOR: &str = " / ";
const LENGTH: Expected = Expected {
    dimension: Some(Dimension::LENGTH),
    non_negative: false,
};
const ANGLE: Expected = Expected {
    dimension: Some(Dimension::ANGLE),
    non_negative: false,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextField {
    Content,
    Height,
    Angle,
}

impl TextField {
    pub const ALL: [Self; 3] = [Self::Content, Self::Height, Self::Angle];

    pub fn label(self) -> &'static str {
        match self {
            Self::Content => "Text",
            Self::Height => "Height",
            Self::Angle => "Angle",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Content => "Letters set in Inter; Enter starts a new line below",
            Self::Height => "How tall a capital letter stands above the baseline",
            Self::Angle => "How far the baseline turns from the sketch's horizontal axis",
        }
    }

    pub fn multiline(self) -> bool {
        self == Self::Content
    }

    fn initial(self) -> &'static str {
        match self {
            Self::Content => "Text",
            Self::Height => "5 mm",
            Self::Angle => "0°",
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lettering {
    pub content: String,
    pub height: Dimensioned,
    pub angle: Dimensioned,
    pub bold: bool,
    pub italic: bool,
    pub align: TextAlign,
}

impl Lettering {
    pub fn text(&self, anchor: EntityId) -> Text {
        Text {
            anchor,
            content: self.content.clone(),
            height: self.height.expression.clone(),
            angle: self.angle.expression.clone(),
            bold: self.bold,
            italic: self.italic,
            align: self.align,
        }
    }

    pub fn placed(&self, at: Point2) -> Option<PlacedText> {
        let size = TextSize {
            height: self.height.value,
            angle: self.angle.value,
        };
        lettered_at(&self.text(EntityId::ORIGIN), at, size)
    }

    fn description(&self, unit: Units) -> String {
        format!(
            "“{}”, {} high",
            shortened(&self.content),
            length_text(unit, self.height.value)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum FieldProblem {
    #[error(transparent)]
    Value(PatternTextError),
    #[error("{}", capitalized(&.0.to_string()))]
    Content(TextError),
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum LetteringProblem {
    #[error("Mend the {} first", .field.label().to_lowercase())]
    MendFirst { field: TextField },
    #[error("{}", capitalized(&.0.to_string()))]
    Unusable(TextError),
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum TextingError {
    #[error(transparent)]
    SketchGone(#[from] SketchGone),
    #[error("{}: {source}.", Tool::Text.label())]
    Refused { source: SketchError },
    #[error("{}: {source}.", Tool::Text.label())]
    Unusable { source: LetteringProblem },
    #[error("{}: its values are not read yet.", Tool::Text.label())]
    NotRead,
    #[error("{}: the text being edited no longer exists.", Tool::Text.label())]
    TextGone,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub problems: Vec<(TextField, FieldProblem)>,
    pub lettering: Result<Lettering, LetteringProblem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextSettings {
    texts: [String; 3],
    bold: bool,
    italic: bool,
    align: TextAlign,
    editing: Option<EntityId>,
    loaded: Option<EntityId>,
    evaluated: Option<Result<Lettering, LetteringProblem>>,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self {
            texts: TextField::ALL.map(|field| field.initial().to_owned()),
            bold: false,
            italic: false,
            align: TextAlign::Left,
            editing: None,
            loaded: None,
            evaluated: None,
        }
    }
}

impl TextSettings {
    pub fn text(&self, field: TextField) -> &str {
        self.texts.get(field.index()).map_or("", String::as_str)
    }

    pub fn text_mut(&mut self, field: TextField) -> Option<&mut String> {
        self.texts.get_mut(field.index())
    }

    pub fn bold_mut(&mut self) -> &mut bool {
        &mut self.bold
    }

    pub fn italic_mut(&mut self) -> &mut bool {
        &mut self.italic
    }

    pub fn align(&self) -> TextAlign {
        self.align
    }

    pub fn set_align(&mut self, align: TextAlign) {
        self.align = align;
    }

    pub fn editing(&self) -> Option<EntityId> {
        self.editing
    }

    pub fn begin(&mut self, sketch: Option<&Sketch>, selected: &[EntityId]) {
        self.editing = match (sketch, selected) {
            (Some(sketch), [only]) if sketch.text(*only).is_some() => Some(*only),
            _ => None,
        };
        self.loaded = None;
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
        let Some(text) = sketch.and_then(|sketch| sketch.text(editing)) else {
            self.stop_editing();
            return;
        };
        let document = model.document();
        self.texts = [
            text.content.clone(),
            document.expression_text(&text.height),
            document.expression_text(&text.angle),
        ];
        self.bold = text.bold;
        self.italic = text.italic;
        self.align = text.align;
        self.loaded = Some(editing);
        self.evaluated = None;
    }

    pub fn lettering(&self) -> Option<&Result<Lettering, LetteringProblem>> {
        self.evaluated.as_ref()
    }

    pub fn evaluate(&mut self, model: &Model) -> Evaluation {
        let mut problems = Vec::new();
        let content = self
            .text(TextField::Content)
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        if let Err(problem) = Text::check_content(&content) {
            problems.push((TextField::Content, FieldProblem::Content(problem)));
        }
        let mut value = |field: TextField, expected: Expected| {
            let label = field.label().to_lowercase();
            patterning::quantity(model, self.text(field), expected, &label)
                .map_err(|problem| problems.push((field, FieldProblem::Value(problem))))
                .ok()
        };
        let height = value(TextField::Height, LENGTH);
        let angle = value(TextField::Angle, ANGLE);
        let lettering = match (problems.first(), height, angle) {
            (None, Some(height), Some(angle)) => {
                let lettering = Lettering {
                    content,
                    height,
                    angle,
                    bold: self.bold,
                    italic: self.italic,
                    align: self.align,
                };
                TextSize {
                    height: lettering.height.value,
                    angle: lettering.angle.value,
                }
                .checked()
                .map(|_| lettering)
                .map_err(LetteringProblem::Unusable)
            }
            (Some((field, _)), ..) => Err(LetteringProblem::MendFirst { field: *field }),
            (None, None, _) => Err(LetteringProblem::MendFirst {
                field: TextField::Height,
            }),
            (None, _, None) => Err(LetteringProblem::MendFirst {
                field: TextField::Angle,
            }),
        };
        self.evaluated = Some(lettering.clone());
        Evaluation {
            problems,
            lettering,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextAnchor {
    Point(EntityId),
    Free(Point2),
}

pub fn lone_point(sketch: &Sketch, selected: &[EntityId]) -> Option<EntityId> {
    match selected {
        [only] if matches!(sketch.entity(*only), Some(Entity::Point(_))) => Some(*only),
        _ => None,
    }
}

pub fn write(
    model: &Model,
    feature: FeatureId,
    lettering: &Lettering,
    anchor: TextAnchor,
) -> Result<Transaction, TextingError> {
    trimming::reshape(model, feature, TRANSACTION.to_owned(), |sketch| {
        let anchor = match anchor {
            TextAnchor::Point(EntityId::ORIGIN) => {
                let point = sketch.add_point(Point2::ZERO);
                sketch
                    .add_constraint(Constraint::Coincident(point, EntityId::ORIGIN))
                    .map_err(|source| TextingError::Refused { source })?;
                point
            }
            TextAnchor::Point(point) => point,
            TextAnchor::Free(at) => sketch.add_point(at),
        };
        sketch
            .add_text(lettering.text(anchor))
            .map(|_| ())
            .map_err(|source| TextingError::Refused { source })
    })
}

pub fn update(
    model: &Model,
    feature: FeatureId,
    text: EntityId,
    lettering: &Lettering,
) -> Result<Transaction, TextingError> {
    let held = model
        .document()
        .feature(feature)
        .and_then(|owner| owner.kind.sketch())
        .ok_or(SketchGone)?
        .text(text)
        .ok_or(TextingError::TextGone)?;
    let mut transaction = model.document().transaction(EDIT_TRANSACTION);
    transaction.set_sketch_text(feature, text, lettering.text(held.anchor));
    Ok(transaction.finish())
}

pub fn shortened(content: &str) -> String {
    let content = content.replace(LINE_BREAK, LINE_SEPARATOR);
    let mut shown: String = content.chars().take(MAX_LABEL_CHARS).collect();
    if content.chars().count() > MAX_LABEL_CHARS {
        shown.push('…');
    }
    shown
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

    fn anchor(self) -> TextAnchor {
        match self {
            Self::Point(point, _) => TextAnchor::Point(point),
            Self::Free(at) => TextAnchor::Free(at),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Texting {
    selected: Option<EntityId>,
    editing: Option<EntityId>,
    hover: Option<Aim>,
    highlight: Option<EntityId>,
    shown: Option<(Aim, Lettering, Option<PlacedText>)>,
}

impl Texting {
    pub fn starting(sketch: &Sketch, selected: &[EntityId], editing: Option<EntityId>) -> Self {
        Self {
            selected: lone_point(sketch, selected),
            editing,
            ..Self::default()
        }
    }

    pub fn sync(&mut self, sketch: &Sketch, editing: Option<EntityId>) {
        let exists = |point: &EntityId| is_anchor(sketch, *point);
        self.selected = self.selected.filter(exists);
        self.highlight = self.highlight.filter(exists);
        self.editing = editing.filter(|text| sketch.text(*text).is_some());
        if self.editing.is_some() {
            self.shown = None;
        }
    }

    pub fn hover(
        &mut self,
        sketch: &Sketch,
        screen: &impl Screen,
        pointer: Option<Pointer>,
        lettering: Option<&Result<Lettering, LetteringProblem>>,
    ) {
        self.hover = pointer.map(|pointer| match point_under(sketch, screen, pointer) {
            Some(point) => Aim::Point(point, sketch.point(point).unwrap_or(pointer.sketch)),
            None => Aim::Free(pointer.sketch),
        });
        self.refresh(sketch, lettering);
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

    fn refresh(
        &mut self,
        sketch: &Sketch,
        lettering: Option<&Result<Lettering, LetteringProblem>>,
    ) {
        let (Some(aim), Some(Ok(lettering)), None) = (self.aim(sketch), lettering, self.editing)
        else {
            self.shown = None;
            return;
        };
        let fresh =
            matches!(&self.shown, Some((shown, drawn, _)) if *shown == aim && drawn == lettering);
        if !fresh {
            let placed = lettering.placed(aim.at());
            self.shown = Some((aim, lettering.clone(), placed));
        }
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let mut preview = Preview::default();
        if let Some((aim, _, placed)) = &self.shown {
            preview.curves = placed
                .as_ref()
                .map(|placed| placed.polylines(faceting))
                .unwrap_or_default();
            preview.points = vec![aim.at()];
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        if let Some(text) = self.editing {
            return vec![text];
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
        lettering: Option<&Result<Lettering, LetteringProblem>>,
    ) -> Option<String> {
        if let Some(text) = self.editing {
            return Some(format!("Editing {}", sketch.entity_label(text)));
        }
        if let Some(Err(problem)) = lettering {
            return Some(problem.to_string());
        }
        let (aim, lettering, _) = self.shown.as_ref()?;
        let place = match aim {
            Aim::Point(point, _) => format!(" at {}", sketch.entity_label(*point)),
            Aim::Free(_) => String::new(),
        };
        Some(format!("Text {}{place}", lettering.description(unit)))
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
        lettering: Option<&Result<Lettering, LetteringProblem>>,
    ) -> Outcome {
        match (self.hover, self.editing) {
            (Some(aim), None) => self.write_at(model, feature, lettering, aim.anchor()),
            _ => Outcome::Nothing,
        }
    }

    pub fn finish(
        &mut self,
        model: &Model,
        feature: FeatureId,
        lettering: Option<&Result<Lettering, LetteringProblem>>,
    ) -> Outcome {
        if let Some(text) = self.editing {
            return match lettering {
                Some(Ok(lettering)) => update(model, feature, text, lettering).into(),
                Some(Err(source)) => Outcome::Refused(
                    TextingError::Unusable {
                        source: source.clone(),
                    }
                    .into(),
                ),
                None => Outcome::Refused(TextingError::NotRead.into()),
            };
        }
        let point = self.highlight.or(self.selected).unwrap_or(EntityId::ORIGIN);
        self.highlight = None;
        self.write_at(model, feature, lettering, TextAnchor::Point(point))
    }

    pub fn activate(
        &mut self,
        model: &Model,
        feature: FeatureId,
        lettering: Option<&Result<Lettering, LetteringProblem>>,
    ) -> Outcome {
        match (self.highlight.take(), self.editing) {
            (Some(point), None) => {
                self.write_at(model, feature, lettering, TextAnchor::Point(point))
            }
            _ => Outcome::Nothing,
        }
    }

    fn write_at(
        &mut self,
        model: &Model,
        feature: FeatureId,
        lettering: Option<&Result<Lettering, LetteringProblem>>,
        anchor: TextAnchor,
    ) -> Outcome {
        let written = match lettering {
            Some(Ok(lettering)) => write(model, feature, lettering, anchor),
            Some(Err(source)) => Err(TextingError::Unusable {
                source: source.clone(),
            }),
            None => Err(TextingError::NotRead),
        };
        if written.is_ok() {
            self.shown = None;
        }
        written.into()
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

fn is_anchor(sketch: &Sketch, point: EntityId) -> bool {
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
