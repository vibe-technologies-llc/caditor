use std::collections::BTreeMap;

use caditor_document::{
    BlendKind, ChamferForm, Document, Edit, ExtrudeEnd, ExtrudeExtent, FeatureKind, HoleDepth,
    ParameterValues, PatternKind, SolidFeature, Transaction,
};
use caditor_expression::{Dimension, Expression};
use caditor_file::Settings;

use crate::{
    field::{self, Expected},
    model::Model,
    units::{LengthUnit, Units},
};

const KEY_PREFIX: &str = "modelling.last.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Remembered {
    FilletSize,
    ChamferSize,
    ChamferForm,
    ChamferSecond,
    ChamferAngle,
    ShellThickness,
    HoleDiameter,
    HoleDepth,
    ExtrudeDistance,
    LinearCount,
    CircularCount,
}

impl Remembered {
    pub const ALL: [Self; 11] = [
        Self::FilletSize,
        Self::ChamferSize,
        Self::ChamferForm,
        Self::ChamferSecond,
        Self::ChamferAngle,
        Self::ShellThickness,
        Self::HoleDiameter,
        Self::HoleDepth,
        Self::ExtrudeDistance,
        Self::LinearCount,
        Self::CircularCount,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::FilletSize => "fillet_size",
            Self::ChamferSize => "chamfer_size",
            Self::ChamferForm => "chamfer_form",
            Self::ChamferSecond => "chamfer_second",
            Self::ChamferAngle => "chamfer_angle",
            Self::ShellThickness => "shell_thickness",
            Self::HoleDiameter => "hole_diameter",
            Self::HoleDepth => "hole_depth",
            Self::ExtrudeDistance => "extrude_distance",
            Self::LinearCount => "linear_count",
            Self::CircularCount => "circular_count",
        }
    }

    fn key(self) -> String {
        format!("{KEY_PREFIX}{}", self.name())
    }

    fn dimension(self) -> Option<Dimension> {
        match self {
            Self::ChamferForm => None,
            Self::ChamferAngle => Some(Dimension::ANGLE),
            Self::LinearCount | Self::CircularCount => Some(Dimension::NONE),
            Self::FilletSize
            | Self::ChamferSize
            | Self::ChamferSecond
            | Self::ShellThickness
            | Self::HoleDiameter
            | Self::HoleDepth
            | Self::ExtrudeDistance => Some(Dimension::LENGTH),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastValues {
    texts: BTreeMap<Remembered, String>,
}

impl LastValues {
    pub fn from_settings(settings: &Settings) -> Self {
        let texts = Remembered::ALL
            .into_iter()
            .filter_map(|slot| {
                let text = settings.text(&slot.key())?.trim();
                (!text.is_empty()).then(|| (slot, text.to_owned()))
            })
            .collect();
        Self { texts }
    }

    pub fn write(&self, settings: &mut Settings) {
        for slot in Remembered::ALL {
            match self.texts.get(&slot) {
                Some(text) => settings.set_text(&slot.key(), text),
                None => settings.remove(&slot.key()),
            }
        }
    }

    pub fn keep(&mut self, slot: Remembered, text: String) -> bool {
        self.texts.insert(slot, text.clone()) != Some(text)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Starts {
    unit: LengthUnit,
    expressions: BTreeMap<Remembered, Expression>,
    forms: BTreeMap<Remembered, String>,
}

impl Starts {
    pub fn defaults(unit: LengthUnit) -> Self {
        Self {
            unit,
            expressions: BTreeMap::new(),
            forms: BTreeMap::new(),
        }
    }

    pub fn of(model: &Model) -> Self {
        Self::resolving(
            model.last_values(),
            model.document(),
            model.parameters(),
            model.units(),
        )
    }

    pub fn resolving(
        last: &LastValues,
        document: &Document,
        parameters: &ParameterValues,
        units: Units,
    ) -> Self {
        let mut starts = Self::defaults(units.length);
        for (slot, text) in &last.texts {
            let Some(dimension) = slot.dimension() else {
                starts.forms.insert(*slot, text.clone());
                continue;
            };
            let expected = Expected {
                dimension: Some(dimension),
                non_negative: true,
            };
            let Ok(expression) =
                field::parse_expression(document, parameters, text, expected, units)
            else {
                continue;
            };
            let positive = parameters
                .evaluate_expression(&expression)
                .is_ok_and(|quantity| quantity.value > 0.0);
            if positive {
                starts.expressions.insert(*slot, expression);
            }
        }
        starts
    }

    pub fn unit(&self) -> LengthUnit {
        self.unit
    }

    pub fn length(&self, slot: Remembered, default_millimetres: f64) -> Expression {
        self.expressions
            .get(&slot)
            .cloned()
            .unwrap_or_else(|| self.unit.default_length(default_millimetres))
    }

    pub fn remembered(&self, slot: Remembered) -> Option<Expression> {
        self.expressions.get(&slot).cloned()
    }

    pub fn count(&self, slot: Remembered, default: f64) -> Expression {
        self.remembered(slot).unwrap_or(Expression::Number(default))
    }

    pub fn form(&self, slot: Remembered) -> Option<&str> {
        self.forms.get(&slot).map(String::as_str)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Held {
    Value(Expression),
    Form(&'static str),
}

pub const FORM_EQUAL: &str = "equal";
pub const FORM_TWO_DISTANCES: &str = "two_distances";
pub const FORM_DISTANCE_ANGLE: &str = "distance_angle";

fn held(kind: &FeatureKind) -> Vec<(Remembered, Held)> {
    let mut held = Vec::new();
    let mut value = |slot, expression: &Expression| {
        held.push((slot, Held::Value(expression.clone())));
    };
    if let Some(blend) = kind.blend() {
        match blend.kind {
            BlendKind::Fillet => value(Remembered::FilletSize, &blend.size),
            BlendKind::Chamfer => value(Remembered::ChamferSize, &blend.size),
        }
        if blend.kind == BlendKind::Chamfer {
            let (form, extra) = match &blend.form {
                ChamferForm::Equal => (FORM_EQUAL, None),
                ChamferForm::TwoDistances { second } => (
                    FORM_TWO_DISTANCES,
                    Some((Remembered::ChamferSecond, second)),
                ),
                ChamferForm::DistanceAngle { angle } => {
                    (FORM_DISTANCE_ANGLE, Some((Remembered::ChamferAngle, angle)))
                }
            };
            held.push((Remembered::ChamferForm, Held::Form(form)));
            if let Some((slot, expression)) = extra {
                held.push((slot, Held::Value(expression.clone())));
            }
        }
    } else if let Some(shell) = kind.shell() {
        value(Remembered::ShellThickness, &shell.thickness);
    } else if let Some(hole) = kind.hole() {
        value(Remembered::HoleDiameter, &hole.diameter);
        if let HoleDepth::Blind(depth) = &hole.depth {
            value(Remembered::HoleDepth, depth);
        }
    } else if let Some(SolidFeature::Extrude(extrude)) = kind.solid() {
        if let ExtrudeExtent::OneSide {
            end: ExtrudeEnd::Distance(distance),
            ..
        } = &extrude.extent
        {
            value(Remembered::ExtrudeDistance, distance);
        }
    } else if let Some(pattern) = kind.pattern() {
        match &pattern.kind {
            PatternKind::Linear { first, .. } => value(Remembered::LinearCount, &first.count),
            PatternKind::Circular(circular) => value(Remembered::CircularCount, &circular.count),
            PatternKind::Curve(_) | PatternKind::Points(_) => {}
        }
    }
    held
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    slot: Remembered,
    held: Held,
}

pub fn changes(document: &Document, transaction: &Transaction) -> Vec<Change> {
    let mut changes = Vec::new();
    for edit in transaction.edits() {
        let Edit::SetFeatureKind { id, kind } = edit else {
            continue;
        };
        let Some(before) = document.feature(*id).map(|feature| held(&feature.kind)) else {
            continue;
        };
        for (slot, now) in held(kind) {
            let unchanged = before
                .iter()
                .any(|(earlier, was)| *earlier == slot && *was == now);
            if !unchanged {
                changes.push(Change { slot, held: now });
            }
        }
    }
    changes
}

pub fn texts(document: &Document, changes: Vec<Change>) -> Vec<(Remembered, String)> {
    changes
        .into_iter()
        .map(|Change { slot, held }| {
            let text = match held {
                Held::Form(form) => form.to_owned(),
                Held::Value(expression) => value_text(document, &expression),
            };
            (slot, text)
        })
        .collect()
}

fn value_text(document: &Document, expression: &Expression) -> String {
    let owned = match expression {
        Expression::Parameter(id) => document
            .parameter(*id)
            .filter(|parameter| parameter.owner.is_some()),
        _ => None,
    };
    match owned {
        Some(parameter) => document.expression_text(&parameter.expression),
        None => document.expression_text(expression),
    }
}

#[cfg(test)]
mod tests {
    use caditor_expression::Unit;

    use super::*;

    #[test]
    fn remembered_texts_survive_a_round_trip_through_settings() {
        let mut last = LastValues::default();

        assert!(last.keep(Remembered::FilletSize, "3 mm".to_owned()));
        assert!(!last.keep(Remembered::FilletSize, "3 mm".to_owned()));
        assert!(last.keep(Remembered::ChamferForm, FORM_TWO_DISTANCES.to_owned()));

        let mut settings = Settings::default();
        last.write(&mut settings);

        assert_eq!(LastValues::from_settings(&settings), last);
        assert!(
            LastValues::from_settings(&Settings::default())
                .texts
                .is_empty()
        );
    }

    #[test]
    fn a_remembered_text_that_does_not_evaluate_falls_back_to_the_default() {
        let document = Document::default();
        let parameters = ParameterValues::evaluate(&document);
        let units = Units::default();
        let mut last = LastValues::default();

        last.keep(Remembered::FilletSize, "3 mm".to_owned());
        last.keep(Remembered::ShellThickness, "wall".to_owned());
        last.keep(Remembered::HoleDiameter, "-2 mm".to_owned());

        let starts = Starts::resolving(&last, &document, &parameters, units);

        assert_eq!(
            starts.length(Remembered::FilletSize, 1.0),
            Expression::measure(3.0, Unit::Millimetre)
        );
        assert_eq!(
            starts.length(Remembered::ShellThickness, 1.0),
            units.length.default_length(1.0)
        );
        assert_eq!(
            starts.length(Remembered::HoleDiameter, 6.0),
            units.length.default_length(6.0)
        );
    }
}
