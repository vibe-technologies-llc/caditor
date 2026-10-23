use std::collections::BTreeMap;

use caditor_document::{
    BlendKind, ChamferForm, Document, Edit, ExtrudeEnd, ExtrudeExtent, FeatureKind, HoleDepth,
    LinearSpacing, ParameterValues, PatternKind, Pieces, RevolveExtent, SolidFeature, Transaction,
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
    RevolveAngle,
    WallThickness,
    PipeSize,
    PipeBend,
    RibThickness,
    OffsetDistance,
    LinearSpacing,
    EmbossDepth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sign {
    AboveZero,
    ZeroOrMore,
    NotZero,
}

impl Sign {
    fn accepts(self, value: f64) -> bool {
        match self {
            Self::AboveZero => value > 0.0,
            Self::ZeroOrMore => value >= 0.0,
            Self::NotZero => value != 0.0,
        }
    }
}

impl Remembered {
    pub const ALL: [Self; 19] = [
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
        Self::RevolveAngle,
        Self::WallThickness,
        Self::PipeSize,
        Self::PipeBend,
        Self::RibThickness,
        Self::OffsetDistance,
        Self::LinearSpacing,
        Self::EmbossDepth,
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
            Self::RevolveAngle => "revolve_angle",
            Self::WallThickness => "wall_thickness",
            Self::PipeSize => "pipe_size",
            Self::PipeBend => "pipe_bend",
            Self::RibThickness => "rib_thickness",
            Self::OffsetDistance => "offset_distance",
            Self::LinearSpacing => "linear_spacing",
            Self::EmbossDepth => "emboss_depth",
        }
    }

    fn key(self) -> String {
        format!("{KEY_PREFIX}{}", self.name())
    }

    fn dimension(self) -> Option<Dimension> {
        match self {
            Self::ChamferForm => None,
            Self::ChamferAngle | Self::RevolveAngle => Some(Dimension::ANGLE),
            Self::LinearCount | Self::CircularCount => Some(Dimension::NONE),
            Self::FilletSize
            | Self::ChamferSize
            | Self::ChamferSecond
            | Self::ShellThickness
            | Self::HoleDiameter
            | Self::HoleDepth
            | Self::ExtrudeDistance
            | Self::WallThickness
            | Self::PipeSize
            | Self::PipeBend
            | Self::RibThickness
            | Self::OffsetDistance
            | Self::EmbossDepth
            | Self::LinearSpacing => Some(Dimension::LENGTH),
        }
    }

    fn sign(self) -> Sign {
        match self {
            Self::PipeBend => Sign::ZeroOrMore,
            Self::OffsetDistance => Sign::NotZero,
            _ => Sign::AboveZero,
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
    pieces: Pieces,
    expressions: BTreeMap<Remembered, Expression>,
    forms: BTreeMap<Remembered, String>,
}

impl Starts {
    pub fn defaults(unit: LengthUnit) -> Self {
        Self {
            unit,
            pieces: Pieces::OneBody,
            expressions: BTreeMap::new(),
            forms: BTreeMap::new(),
        }
    }

    pub fn of(model: &Model) -> Self {
        Self {
            pieces: model.new_body_pieces(),
            ..Self::resolving(
                model.last_values(),
                model.document(),
                model.parameters(),
                model.units(),
            )
        }
    }

    pub fn pieces(&self) -> Pieces {
        self.pieces
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
            let sign = slot.sign();
            let expected = Expected {
                dimension: Some(dimension),
                non_negative: sign != Sign::NotZero,
            };
            let Ok(expression) =
                field::parse_expression(document, parameters, text, expected, units)
            else {
                continue;
            };
            let accepted = parameters
                .evaluate_expression(&expression)
                .is_ok_and(|quantity| sign.accepts(quantity.value));
            if accepted {
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
pub const FULL_TURN: &str = "full";

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
    } else if let Some(solid) = kind.solid() {
        match solid {
            SolidFeature::Extrude(extrude) => {
                if let ExtrudeExtent::OneSide {
                    end: ExtrudeEnd::Distance(distance),
                    ..
                } = &extrude.extent
                {
                    value(Remembered::ExtrudeDistance, distance);
                }
            }
            SolidFeature::Revolve(revolve) => match &revolve.extent {
                RevolveExtent::OneSide { angle, .. } => value(Remembered::RevolveAngle, angle),
                RevolveExtent::Full => {
                    held.push((Remembered::RevolveAngle, Held::Form(FULL_TURN)));
                }
                RevolveExtent::Symmetric { .. }
                | RevolveExtent::TwoSides { .. }
                | RevolveExtent::UpTo { .. } => {}
            },
            SolidFeature::Sweep(_) => {}
        }
        if let Some(wall) = solid.wall() {
            held.push((
                Remembered::WallThickness,
                Held::Value(wall.thickness.clone()),
            ));
        }
    } else if let Some(pipe) = kind.pipe() {
        value(Remembered::PipeSize, &pipe.size);
        value(Remembered::PipeBend, &pipe.bend);
    } else if let Some(rib) = kind.rib() {
        value(Remembered::RibThickness, &rib.thickness);
    } else if let Some(offset) = kind.offset_face() {
        value(Remembered::OffsetDistance, &offset.distance);
    } else if let Some(emboss) = kind.emboss() {
        value(Remembered::EmbossDepth, &emboss.depth);
    } else if let Some(pattern) = kind.pattern() {
        match &pattern.kind {
            PatternKind::Linear { first, .. } => {
                value(Remembered::LinearCount, &first.count);
                if first.measured == LinearSpacing::BetweenCopies {
                    value(Remembered::LinearSpacing, &first.spacing);
                }
            }
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
    use caditor_document::{
        AxisReference, BodyOperation, FeatureId, LinearDirection, OffsetFace, Pattern,
        PatternOutcome, Pipe, PrincipalAxis, RegionChoice, Revolve, RevolveAxis, Rib, SweepPath,
        Wall,
    };
    use caditor_expression::Unit;
    use caditor_kernel::WallSide;
    use caditor_sketch::EntityId;

    use super::*;

    fn remembered_from(kind: &FeatureKind) -> Starts {
        let document = Document::default();
        let parameters = ParameterValues::evaluate(&document);
        let changes = held(kind)
            .into_iter()
            .map(|(slot, held)| Change { slot, held })
            .collect();
        let mut last = LastValues::default();
        for (slot, text) in texts(&document, changes) {
            last.keep(slot, text);
        }
        Starts::resolving(&last, &document, &parameters, Units::default())
    }

    fn mm(value: f64) -> Expression {
        Expression::measure(value, Unit::Millimetre)
    }

    fn revolve(extent: RevolveExtent, wall: Option<Wall>) -> FeatureKind {
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch: FeatureId::from_raw(1),
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
            extent,
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            side: None,
            wall: wall.map(Box::new),
        }))
    }

    fn linear(spacing: Expression, measured: LinearSpacing) -> FeatureKind {
        FeatureKind::from(Pattern {
            body: FeatureId::from_raw(1),
            kind: PatternKind::Linear {
                first: LinearDirection {
                    axis: AxisReference::Principal(PrincipalAxis::X),
                    count: Expression::Number(3.0),
                    spacing,
                    measured,
                    reversed: false,
                },
                second: None,
            },
            skipped: Default::default(),
            repeated: Vec::new(),
            outcome: PatternOutcome::AddToBody,
        })
    }

    #[test]
    fn revolve_wall_pipe_rib_offset_and_spacing_values_are_remembered_and_read_back() {
        let quarter = Expression::measure(90.0, Unit::Degree);
        let walled = revolve(
            RevolveExtent::OneSide {
                angle: quarter.clone(),
                reversed: false,
            },
            Some(Wall {
                thickness: mm(2.0),
                side: WallSide::Inside,
            }),
        );
        let pipe = FeatureKind::Pipe(Pipe::new(SweepPath::Unchosen, mm(6.0), mm(0.0)));
        let rib = FeatureKind::Rib(Rib::new(
            FeatureId::from_raw(1),
            FeatureId::from_raw(2),
            mm(3.0),
        ));
        let offset = FeatureKind::OffsetFace(OffsetFace {
            body: FeatureId::from_raw(1),
            faces: Vec::new(),
            distance: mm(-2.0),
            tangent: false,
        });

        let turned = remembered_from(&walled);
        let full = remembered_from(&revolve(RevolveExtent::Full, None));
        let piped = remembered_from(&pipe);
        let ribbed = remembered_from(&rib);
        let offsetting = remembered_from(&offset);
        let spaced = remembered_from(&linear(mm(15.0), LinearSpacing::BetweenCopies));
        let overall = remembered_from(&linear(mm(60.0), LinearSpacing::Total));

        assert_eq!(turned.remembered(Remembered::RevolveAngle), Some(quarter));
        assert_eq!(turned.length(Remembered::WallThickness, 1.0), mm(2.0));
        assert_eq!(full.remembered(Remembered::RevolveAngle), None);
        assert_eq!(piped.length(Remembered::PipeSize, 10.0), mm(6.0));
        assert_eq!(piped.length(Remembered::PipeBend, 10.0), mm(0.0));
        assert_eq!(ribbed.length(Remembered::RibThickness, 2.0), mm(3.0));
        assert_eq!(offsetting.length(Remembered::OffsetDistance, 1.0), mm(-2.0));
        assert_eq!(spaced.remembered(Remembered::LinearSpacing), Some(mm(15.0)));
        assert_eq!(
            spaced.count(Remembered::LinearCount, 2.0),
            Expression::Number(3.0)
        );
        assert_eq!(overall.remembered(Remembered::LinearSpacing), None);
    }

    #[test]
    fn a_zero_offset_a_negative_bend_and_a_zero_rib_give_way_to_their_defaults() {
        let document = Document::default();
        let parameters = ParameterValues::evaluate(&document);
        let units = Units::default();
        let mut last = LastValues::default();

        last.keep(Remembered::OffsetDistance, "0 mm".to_owned());
        last.keep(Remembered::PipeBend, "-1 mm".to_owned());
        last.keep(Remembered::RibThickness, "0 mm".to_owned());
        last.keep(Remembered::RevolveAngle, FULL_TURN.to_owned());

        let starts = Starts::resolving(&last, &document, &parameters, units);

        assert_eq!(starts.remembered(Remembered::OffsetDistance), None);
        assert_eq!(starts.remembered(Remembered::PipeBend), None);
        assert_eq!(starts.remembered(Remembered::RibThickness), None);
        assert_eq!(starts.remembered(Remembered::RevolveAngle), None);
    }

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
