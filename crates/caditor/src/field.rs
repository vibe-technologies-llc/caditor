use caditor_document::{Document, Edit, FeatureId, ParameterValues, Transaction};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_sketch::{Constraint, ConstraintId};
use egui::{Align, Id, Key, Response, Stroke, StrokeKind, TextEdit, Ui, vec2};

use crate::units::{LengthUnit, attach_unit};

const ERROR_OUTLINE_WIDTH: f32 = 1.5;
const ERROR_OUTLINE_RADIUS: f32 = 2.0;

#[derive(Debug, Clone, Default)]
struct Draft {
    text: String,
    error: Option<String>,
    stored: String,
}

pub struct FieldResponse<T> {
    pub committed: Option<T>,
    pub error: Option<String>,
    pub response: Response,
}

pub fn commit_field<T>(
    ui: &mut Ui,
    id: Id,
    stored: &str,
    width: f32,
    focus: bool,
    validate: impl FnOnce(&str) -> Result<T, String>,
) -> FieldResponse<T> {
    let editing = ui.memory(|memory| memory.has_focus(id));
    let mut draft = ui
        .data(|data| data.get_temp::<Draft>(id))
        .filter(|draft| editing || draft.stored == stored);
    let mut text = draft
        .as_ref()
        .map_or_else(|| stored.to_owned(), |draft| draft.text.clone());
    let response = ui.add(
        TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(width)
            .min_size(vec2(width, 0.0)),
    );
    if focus {
        response.request_focus();
        response.scroll_to_me(Some(Align::Center));
    }
    if response.changed() {
        draft = Some(Draft {
            text: text.clone(),
            error: None,
            stored: stored.to_owned(),
        });
    }

    let mut committed = None;
    if response.lost_focus() {
        let reverted = ui.input(|input| input.key_pressed(Key::Escape));
        if reverted || text.trim() == stored {
            draft = None;
        } else {
            match validate(text.trim()) {
                Ok(value) => {
                    committed = Some(value);
                    draft = None;
                }
                Err(message) => {
                    draft = Some(Draft {
                        text,
                        error: Some(message),
                        stored: stored.to_owned(),
                    });
                }
            }
        }
    }

    let error = draft.as_ref().and_then(|draft| draft.error.clone());
    if error.is_some() {
        ui.painter().rect_stroke(
            response.rect,
            ERROR_OUTLINE_RADIUS,
            Stroke::new(ERROR_OUTLINE_WIDTH, ui.visuals().error_fg_color),
            StrokeKind::Outside,
        );
    }
    ui.data_mut(|data| match draft {
        Some(draft) => {
            data.insert_temp(id, draft);
        }
        None => data.remove::<Draft>(id),
    });
    FieldResponse {
        committed,
        error,
        response,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expected {
    pub dimension: Option<Dimension>,
    pub non_negative: bool,
}

impl Expected {
    pub const ANYTHING: Self = Self {
        dimension: None,
        non_negative: false,
    };
}

pub fn parse_expression(
    document: &Document,
    parameters: &ParameterValues,
    text: &str,
    expected: Expected,
    unit: LengthUnit,
) -> Result<Expression, String> {
    let expression = document.parse(text).map_err(|error| error.to_string())?;
    let value = parameters
        .evaluate_expression(&expression)
        .map_err(|error| sentence(&error.to_string()))?;
    if let Some(dimension) = expected.dimension {
        expression
            .evaluate_as(dimension, &|id| parameters.value(id))
            .map_err(|error| sentence(&error.to_string()))?;
    }
    if expected.non_negative && value.value < 0.0 {
        return Err("The value cannot be negative".to_owned());
    }
    if unit.applies_to(expected.dimension, value.dimension) {
        return Ok(unit.attach(expression));
    }
    Ok(expression)
}

pub fn parameter_expression(
    document: &Document,
    parameters: &ParameterValues,
    text: &str,
    current: Option<Dimension>,
    unit: LengthUnit,
) -> Result<Expression, String> {
    let expression = parse_expression(document, parameters, text, Expected::ANYTHING, unit)?;
    let plain = parameters
        .evaluate_expression(&expression)
        .is_ok_and(|value| value.dimension.is_plain());
    Ok(match current {
        Some(Dimension::LENGTH) if plain => unit.attach(expression),
        Some(Dimension::ANGLE) if plain => attach_unit(expression, Unit::Degree),
        _ => expression,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimensionTarget {
    pub feature: FeatureId,
    pub constraint: ConstraintId,
}

pub fn dimension_transaction(
    document: &Document,
    parameters: &ParameterValues,
    target: DimensionTarget,
    text: &str,
    unit: LengthUnit,
) -> Result<Transaction, String> {
    let owner = document
        .feature(target.feature)
        .ok_or_else(|| "The sketch no longer exists".to_owned())?;
    let definition = owner
        .kind
        .sketch()
        .and_then(|sketch| sketch.constraint(target.constraint))
        .ok_or_else(|| "The dimension no longer exists".to_owned())?;
    let expected = Expected {
        dimension: definition.dimension_kind(),
        non_negative: !matches!(definition, Constraint::Angle { .. }),
    };
    let value = parse_expression(document, parameters, text, expected, unit)?;
    let quantity = parameters
        .evaluate_expression(&value)
        .map_err(|error| sentence(&error.to_string()))?;
    definition
        .check_dimension_value(quantity.value)
        .map_err(|error| sentence(&error.to_string()))?;
    checked(
        document,
        Transaction::single(
            format!("Edit dimension in {}", owner.name),
            Edit::SetDimension {
                feature: target.feature,
                constraint: target.constraint,
                value,
            },
        ),
    )
}

pub fn checked(document: &Document, transaction: Transaction) -> Result<Transaction, String> {
    document
        .check(&transaction)
        .map(|()| transaction)
        .map_err(|error| error.to_string())
}

pub fn value_preview(
    parameters: &ParameterValues,
    expression: &Expression,
    unit: LengthUnit,
) -> Option<String> {
    if expression.is_literal() {
        return None;
    }
    parameters
        .evaluate_expression(expression)
        .ok()
        .map(|value| format!("= {}", unit.show(value)))
}

pub fn sentence(clause: &str) -> String {
    let mut characters = clause.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use caditor_expression::Unit;

    use super::*;

    fn document() -> Document {
        let mut document = Document::default();
        let mut transaction = document.transaction("Parameters");
        transaction.add_parameter("width", Expression::Measure(40.0, Unit::Millimetre));
        transaction.add_parameter("zero", Expression::Number(0.0));
        document.apply(transaction.finish()).unwrap();
        document
    }

    fn parse(text: &str, expected: Expected) -> Result<Expression, String> {
        let document = document();
        let parameters = ParameterValues::evaluate(&document);
        parse_expression(
            &document,
            &parameters,
            text,
            expected,
            LengthUnit::Millimetre,
        )
    }

    const LENGTH: Expected = Expected {
        dimension: Some(Dimension::LENGTH),
        non_negative: true,
    };

    #[test]
    fn field_input_is_parsed_evaluated_and_checked() {
        assert!(parse("width / 2", LENGTH).is_ok());
        assert!(parse("12", LENGTH).is_ok());
        assert_eq!(
            parse("width * width", LENGTH),
            Err("It gives an area, but a length is needed".to_owned())
        );
        assert_eq!(
            parse("-width", LENGTH),
            Err("The value cannot be negative".to_owned())
        );
        assert_eq!(
            parse("width / zero", Expected::ANYTHING),
            Err("It divides by zero".to_owned())
        );
        assert_eq!(
            parse("wdth", Expected::ANYTHING),
            Err("There is no parameter named 'wdth'".to_owned())
        );
        assert!(parse("-width * width", Expected::ANYTHING).is_ok());
    }

    #[test]
    fn a_plain_parameter_value_keeps_the_kind_of_the_parameter() {
        let document = document();
        let parameters = ParameterValues::evaluate(&document);
        let text = |input: &str, current: Option<Dimension>| {
            parameter_expression(
                &document,
                &parameters,
                input,
                current,
                LengthUnit::Centimetre,
            )
            .map(|expression| document.expression_text(&expression))
        };
        let length = Some(Dimension::LENGTH);
        let angle = Some(Dimension::ANGLE);
        assert_eq!(text("12", length).unwrap(), "12 cm");
        assert_eq!(text("-2", length).unwrap(), "-2 cm");
        assert_eq!(text("2 * 3", length).unwrap(), "(2 * 3) cm");
        assert_eq!(text("width / 2", length).unwrap(), "width / 2");
        assert_eq!(text("30", angle).unwrap(), "30 deg");
        assert_eq!(text("5 mm", angle).unwrap(), "5 mm");
        assert_eq!(text("4", Some(Dimension::NONE)).unwrap(), "4");
        assert_eq!(text("4", None).unwrap(), "4");
    }

    #[test]
    fn a_dimension_value_must_suit_its_constraint() {
        let mut document = document();
        let mut sketch = caditor_sketch::Sketch::new(caditor_geometry::Plane::XY);
        let circle = sketch.add_circle(caditor_geometry::Point2::ZERO, 5.0);
        let radius = sketch
            .add_constraint(Constraint::Radius {
                entity: circle,
                value: Expression::Measure(5.0, Unit::Millimetre),
            })
            .unwrap();
        let mut transaction = document.transaction("Sketch");
        let feature = transaction.add_feature("Holes", caditor_document::FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let parameters = ParameterValues::evaluate(&document);
        let target = DimensionTarget {
            feature,
            constraint: radius,
        };
        let edit = |text| {
            dimension_transaction(&document, &parameters, target, text, LengthUnit::Millimetre)
        };

        assert_eq!(
            edit("width - 40 mm").err(),
            Some("A radius must be greater than zero".to_owned())
        );
        assert_eq!(
            edit("-2 mm").err(),
            Some("The value cannot be negative".to_owned())
        );
        assert_eq!(
            edit("5 deg").err(),
            Some("It gives an angle, but a length is needed".to_owned())
        );
        assert!(edit("width / 8").is_ok());
    }

    #[test]
    fn previews_show_computed_values_only_for_non_literals() {
        let document = document();
        let parameters = ParameterValues::evaluate(&document);
        let preview = |text: &str| {
            value_preview(
                &parameters,
                &document.parse(text).unwrap(),
                LengthUnit::Millimetre,
            )
        };
        assert_eq!(preview("width / 4"), Some("= 10 mm".to_owned()));
        assert_eq!(preview("2 cm"), None);
        assert_eq!(preview("width / zero"), None);
    }
}
