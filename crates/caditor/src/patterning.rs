use caditor_document::{FeatureId, Transaction};
use caditor_expression::{BinaryOperator, Dimension, Expression, Unit};
use caditor_geometry::Point2;
use caditor_sketch::{
    CircularPattern, Dimensioned, Entity, EntityId, Faceting, PatternError, PatternImage,
    PatternRow, RectangularPattern, Sketch, Spread,
};

use crate::{
    drawing::Preview,
    editing::Tool,
    feature_tree::count,
    field::{self, Expected},
    model::Model,
    modifying::{Hint, Outcome, Prompt, ValueField},
    snap::{self, Pointer, Screen},
    trimming::{self, capitalized},
    typed_point,
    units::attach_unit,
};

pub const TRANSACTION: &str = "Pattern geometry";
pub const RECTANGULAR_SELECT_FIRST: &str =
    "Select the geometry to repeat first, then choose Rectangular pattern";
pub const CIRCULAR_SELECT_FIRST: &str =
    "Select the geometry to repeat first, then choose Circular pattern";
pub const RECTANGULAR_PROMPT: &str =
    "Type how many and how far apart, such as 4 x 10, or 4 x 10, 3 x 15 for a grid";
pub const CIRCULAR_PROMPT: &str =
    "Type how many, such as 6 over a full turn, or 5 over 120 for part of one";
pub const CENTRE_PROMPT: &str = "Click the point to repeat the selection about";
pub const SELECT_KEYS: &str = "Esc: back to Select, then select what to repeat";
pub const RECTANGULAR_KEYS: &str = "Count x spacing, then < angle for a slanted direction; a second term adds rows   Esc: back to Select";
pub const CIRCULAR_KEYS: &str = "A count alone spaces them over a full turn; count over angle spreads them across it   Esc: choose another point";
pub const CENTRE_KEYS: &str =
    "Click a point, or Highlight the next item in the view and press Space   Esc: back to Select";
pub const RECTANGULAR_FIELD: ValueField = ValueField {
    label: "Repeat",
    placeholder: "4 x 10, 3 x 15",
    action: "repeat",
};
pub const CIRCULAR_FIELD: ValueField = ValueField {
    label: "Repeat",
    placeholder: "6  or  5 over 120",
    action: "repeat",
};
const NO_POINT_HIGHLIGHTED: &str =
    "Highlight a point first, with Highlight the next item in the view";
const CENTRE_FIRST: &str = "click the point to repeat the selection about first";
const QUARTER_TURN_DEGREES: f64 = 90.0;
const WHOLE_NUMBER_TOLERANCE: f64 = 1e-9;
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
const OVER: &str = "over";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternKind {
    Rectangular,
    Circular,
}

impl PatternKind {
    fn tool(self) -> Tool {
        match self {
            Self::Rectangular => Tool::RectangularPattern,
            Self::Circular => Tool::CircularPattern,
        }
    }

    pub fn field(self) -> ValueField {
        match self {
            Self::Rectangular => RECTANGULAR_FIELD,
            Self::Circular => CIRCULAR_FIELD,
        }
    }

    fn select_first(self) -> &'static str {
        match self {
            Self::Rectangular => RECTANGULAR_SELECT_FIRST,
            Self::Circular => CIRCULAR_SELECT_FIRST,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Spec {
    Rectangular(RectangularPattern),
    Circular(CircularPattern),
}

impl Spec {
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
    selected_centre: Option<EntityId>,
    chosen: Option<EntityId>,
    hover: Option<EntityId>,
    highlight: Option<EntityId>,
    spec: Option<Result<Spec, String>>,
    image: Option<Result<PatternImage, PatternError>>,
}

impl Patterning {
    pub fn new(kind: PatternKind) -> Self {
        Self {
            kind,
            selected: Vec::new(),
            selected_centre: None,
            chosen: None,
            hover: None,
            highlight: None,
            spec: None,
            image: None,
        }
    }

    pub fn field(&self) -> ValueField {
        self.kind.field()
    }

    pub fn sync(&mut self, sketch: &Sketch, selected: &[EntityId]) {
        if self.selected != selected {
            self.selected = selected.to_vec();
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
            PatternKind::Circular => {
                pointer.and_then(|pointer| point_under(sketch, screen, pointer))
            }
        };
        self.image = self.spec.as_ref().and_then(|spec| {
            let spec = spec.as_ref().ok()?;
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
        if self.centre().is_none() {
            self.image = None;
        }
    }

    fn centre(&self) -> Option<EntityId> {
        self.chosen
            .or(self.highlight)
            .or(self.hover)
            .or(self.selected_centre)
    }

    pub fn show_text(&mut self, model: &Model, text: Option<&str>) {
        self.spec = text
            .filter(|text| !text.trim().is_empty())
            .map(|text| parse(model, self.kind, text));
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
        let items = self
            .selected
            .iter()
            .filter(|item| Some(**item) != centre)
            .count();
        match (self.spec.as_ref()?, &self.image) {
            (Err(_), _) => None,
            (Ok(Spec::Circular(_)), None) if centre.is_none() => Some(capitalized(CENTRE_PROMPT)),
            (Ok(spec), Some(Ok(_))) => {
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
            (Ok(_), Some(Err(error))) => Some(capitalized(&error.to_string())),
            (Ok(_), None) => None,
        }
    }

    pub fn prompt(&self) -> Prompt {
        if self.selected.is_empty() {
            return Prompt {
                text: self.kind.select_first(),
                hint: Hint::Keys(SELECT_KEYS),
            };
        }
        match (self.kind, self.centre()) {
            (PatternKind::Rectangular, _) => Prompt {
                text: RECTANGULAR_PROMPT,
                hint: Hint::Keys(RECTANGULAR_KEYS),
            },
            (PatternKind::Circular, None) => Prompt {
                text: CENTRE_PROMPT,
                hint: Hint::Keys(CENTRE_KEYS),
            },
            (PatternKind::Circular, Some(_)) => Prompt {
                text: CIRCULAR_PROMPT,
                hint: Hint::Keys(CIRCULAR_KEYS),
            },
        }
    }

    pub fn click(&mut self) -> Outcome {
        if let Some(point) = self.hover.filter(|_| self.kind == PatternKind::Circular) {
            self.chosen = Some(point);
            self.highlight = None;
        }
        Outcome::Nothing
    }

    pub fn activate(&mut self) -> Outcome {
        if let Some(point) = self.highlight.take() {
            self.chosen = Some(point);
        }
        Outcome::Nothing
    }

    pub fn enter_text(
        &mut self,
        model: &Model,
        feature: FeatureId,
        text: &str,
    ) -> Result<Outcome, String> {
        let tool = self.kind.tool();
        let spec = parse(model, self.kind, text)?;
        let selected = self.selected.clone();
        let centre = self.centre();
        let reason = |error: PatternError| trimming::refusal(tool, &error.to_string());
        let transaction: Result<Transaction, String> =
            trimming::reshaped(model, feature, TRANSACTION.to_owned(), |sketch| {
                match (&spec, centre) {
                    (Spec::Rectangular(pattern), _) => sketch
                        .rectangular_pattern(&selected, pattern)
                        .map(|_| ())
                        .map_err(reason),
                    (Spec::Circular(pattern), Some(centre)) => sketch
                        .circular_pattern(&selected, centre, pattern)
                        .map(|_| ())
                        .map_err(reason),
                    (Spec::Circular(_), None) => Err(trimming::refusal(tool, CENTRE_FIRST)),
                }
            });
        transaction.map(Outcome::Apply)
    }

    pub fn steps_targets(&self) -> bool {
        self.kind == PatternKind::Circular
    }

    pub fn steppable(&self) -> Result<(), &'static str> {
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

    pub fn highlight_needed(&self) -> Result<(), &'static str> {
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(NO_POINT_HIGHLIGHTED),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some() || self.chosen.is_some()
    }

    pub fn back_out(&mut self) {
        if self.highlight.take().is_none() {
            self.chosen = None;
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
            | Entity::Spline { .. } => None,
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

fn parse(model: &Model, kind: PatternKind, text: &str) -> Result<Spec, String> {
    match kind {
        PatternKind::Rectangular => parse_rectangular(model, text).map(Spec::Rectangular),
        PatternKind::Circular => parse_circular(model, text).map(Spec::Circular),
    }
}

fn parse_rectangular(model: &Model, text: &str) -> Result<RectangularPattern, String> {
    let terms = typed_point::coordinates(text);
    let (first, second) = match terms.as_slice() {
        [first] => (first, None),
        [first, second] => (first, Some(second)),
        _ => return Err(RECTANGULAR_FORMS.to_owned()),
    };
    let first = parse_row(model, first, None)?;
    let second = second
        .map(|term| parse_row(model, term, Some(&first.angle)))
        .transpose()?;
    Ok(RectangularPattern { first, second })
}

const RECTANGULAR_FORMS: &str =
    "Type a count and a spacing such as 4 x 10, and a second pair such as 3 x 15 for rows";

fn parse_row(
    model: &Model,
    term: &str,
    beside: Option<&Dimensioned>,
) -> Result<PatternRow, String> {
    let (body, angle) = match typed_point::polar(term).as_slice() {
        [body] => (*body, None),
        [body, angle] => (*body, Some(*angle)),
        _ => return Err(RECTANGULAR_FORMS.to_owned()),
    };
    let (count, spacing) =
        split_top_level(body, is_times).ok_or_else(|| RECTANGULAR_FORMS.to_owned())?;
    let count = whole_count(model, count)?;
    let spacing = quantity(model, spacing, SPACING, "spacing")?;
    let angle = match (angle, beside) {
        (Some(angle), _) => degrees(model, angle)?,
        (None, None) => turned(0.0),
        (None, Some(first)) => square_to(first),
    };
    Ok(PatternRow {
        count,
        spacing,
        angle,
    })
}

fn parse_circular(model: &Model, text: &str) -> Result<CircularPattern, String> {
    match split_top_level(text, is_over) {
        None => Ok(CircularPattern {
            count: whole_count(model, text)?,
            spread: Spread::FullTurn,
        }),
        Some((count, angle)) => Ok(CircularPattern {
            count: whole_count(model, count)?,
            spread: Spread::Total(degrees(model, angle)?),
        }),
    }
}

fn whole_count(model: &Model, text: &str) -> Result<usize, String> {
    let counted = quantity(model, text, COUNT, "count")?;
    let rounded = counted.value.round();
    let whole = (counted.value - rounded).abs() <= WHOLE_NUMBER_TOLERANCE;
    if !whole || rounded < 0.0 || rounded > f64::from(u32::MAX) {
        return Err("count: Use a whole number such as 4".to_owned());
    }
    Ok(rounded as usize)
}

pub(crate) fn degrees(model: &Model, text: &str) -> Result<Dimensioned, String> {
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

fn turned(angle: f64) -> Dimensioned {
    Dimensioned {
        expression: Expression::measure(angle, Unit::Degree),
        value: angle,
    }
}

fn square_to(first: &Dimensioned) -> Dimensioned {
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
) -> Result<Dimensioned, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("The {label} is missing"));
    }
    let expression = field::parse_expression(
        model.document(),
        model.parameters(),
        text,
        expected,
        model.units(),
    )
    .map_err(|error| format!("{label}: {error}"))?;
    let dimension = expected.dimension.unwrap_or(Dimension::NONE);
    let value = expression
        .evaluate_as(dimension, &|id| model.parameters().value(id))
        .map_err(|error| format!("{label}: {}", field::sentence(&error.to_string())))?;
    Ok(Dimensioned { expression, value })
}

fn is_times(previous: Option<char>, rest: &str) -> Option<usize> {
    let mut characters = rest.chars();
    let mark = characters
        .next()
        .filter(|mark| matches!(mark, 'x' | 'X' | '×'))?;
    let before = previous
        .is_none_or(|before| before.is_ascii_digit() || before.is_whitespace() || before == ')');
    let after = characters.next().is_none_or(|after| {
        after.is_whitespace() || after.is_ascii_digit() || matches!(after, '(' | '-' | '+' | '.')
    });
    (before && after).then_some(mark.len_utf8())
}

fn is_over(previous: Option<char>, rest: &str) -> Option<usize> {
    let word = rest.get(..OVER.len())?;
    if !word.eq_ignore_ascii_case(OVER) {
        return None;
    }
    let before = previous
        .is_none_or(|before| before.is_ascii_digit() || before.is_whitespace() || before == ')');
    let after = rest
        .get(OVER.len()..)?
        .chars()
        .next()
        .is_none_or(|after| after.is_whitespace() || after.is_ascii_digit() || after == '(');
    (before && after).then_some(OVER.len())
}

fn split_top_level(
    text: &str,
    separator: impl Fn(Option<char>, &str) -> Option<usize>,
) -> Option<(&str, &str)> {
    let mut depth = 0_usize;
    let mut previous = None;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => {
                if let Some(length) = text.get(index..).and_then(|rest| separator(previous, rest)) {
                    return Some((text.get(..index)?, text.get(index + length..)?));
                }
            }
            _ => {}
        }
        previous = Some(character);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_times_sign_separates_a_count_from_a_spacing_only_between_numbers_and_spaces() {
        assert_eq!(split_top_level("4 x 10", is_times), Some(("4 ", " 10")));
        assert_eq!(split_top_level("4x10", is_times), Some(("4", "10")));
        assert_eq!(
            split_top_level("4 X (2 + 3)", is_times),
            Some(("4 ", " (2 + 3)"))
        );
        assert_eq!(split_top_level("4 × 10", is_times), Some(("4 ", " 10")));
        assert_eq!(split_top_level("4 max(x, 2)", is_times), None);
        assert_eq!(split_top_level("4 xs", is_times), None);
        assert_eq!(split_top_level("max_x", is_times), None);
    }

    #[test]
    fn over_separates_a_count_from_an_angle_and_nothing_else_does() {
        assert_eq!(split_top_level("5 over 120", is_over), Some(("5 ", " 120")));
        assert_eq!(
            split_top_level("5 OVER 90 deg", is_over),
            Some(("5 ", " 90 deg"))
        );
        assert_eq!(split_top_level("6", is_over), None);
        assert_eq!(split_top_level("moreover", is_over), None);
    }

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
