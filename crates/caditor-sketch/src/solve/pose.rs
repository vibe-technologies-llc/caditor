use std::{borrow::Cow, collections::BTreeMap};

use caditor_expression::{Expression, Unit};

use crate::{
    gear::{Gear, GearShape, GearValueError, GearValues, SpurGear, meshing_values},
    id::EntityId,
    sketch::{Sketch, SketchError},
    solve::Drag,
    text::{Text, TextSize},
};

const UNCHANGED: f64 = 1e-12;
const FULL_TURN_DEGREES: f64 = 360.0;
const HALF_TURN_DEGREES: f64 = 180.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GearPose {
    pub pitch_radius: f64,
    pub turn: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Poses {
    texts: BTreeMap<EntityId, (Text, TextSize)>,
    gears: BTreeMap<EntityId, (Gear, GearValues)>,
}

impl Poses {
    pub fn is_empty(&self) -> bool {
        self.texts.is_empty() && self.gears.is_empty()
    }

    pub fn text(&self, id: EntityId) -> Option<(&Text, TextSize)> {
        self.texts.get(&id).map(|(text, size)| (text, *size))
    }

    pub fn gear(&self, id: EntityId) -> Option<(&Gear, GearValues)> {
        self.gears.get(&id).map(|(gear, values)| (gear, *values))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Change {
    turned: f64,
    ratio: f64,
}

impl Change {
    const NONE: Self = Self {
        turned: 0.0,
        ratio: 1.0,
    };
}

fn angle_between(from: f64, to: f64) -> f64 {
    (to - from + HALF_TURN_DEGREES).rem_euclid(FULL_TURN_DEGREES) - HALF_TURN_DEGREES
}

impl Sketch {
    pub fn poses(&self, drags: &[Drag]) -> Result<Poses, SketchError> {
        let mut poses = Poses::default();

        for drag in drags {
            if let Drag::Text { text, to } = drag
                && let Some(posed) = self.posed_text(*text, *to)?
            {
                poses.texts.insert(*text, posed);
            }
        }

        let targets: BTreeMap<EntityId, GearPose> = drags
            .iter()
            .filter_map(|drag| match drag {
                Drag::Gear { gear, to } => Some((*gear, *to)),
                Drag::Point { .. } | Drag::Radius { .. } | Drag::Text { .. } => None,
            })
            .collect();
        let mut changes = BTreeMap::new();

        for (id, to) in &targets {
            let (Some(gear), Some(values)) = (self.gear(*id), self.gear_values(*id)) else {
                continue;
            };

            if gear.meshing.is_some() {
                continue;
            }

            let (posed, after, change) =
                posed_gear(gear, values, *to).map_err(|reason| self.gear_failed(*id, reason))?;

            changes.insert(*id, change);
            poses.gears.insert(*id, (posed, after));
        }
        for (id, gear) in self.gears() {
            let Some(meshing) = &gear.meshing else {
                continue;
            };
            let (Some(change), Some(driver), Some(values)) = (
                changes.get(&meshing.driver),
                poses.gear(meshing.driver).map(|(_, values)| values),
                self.gear_values(id),
            ) else {
                continue;
            };
            let change = if targets.contains_key(&id) {
                *change
            } else {
                Change::NONE
            };

            let posed = posed_partner(gear, values, &driver, change)
                .map_err(|reason| self.gear_failed(id, reason))?;

            poses.gears.insert(id, posed);
        }
        Ok(poses)
    }

    pub(crate) fn posed(&self, drags: &[Drag]) -> Result<Cow<'_, Self>, SketchError> {
        let poses = self.poses(drags)?;

        if poses.is_empty() {
            return Ok(Cow::Borrowed(self));
        }

        let mut posed = self.clone();

        for (id, (text, size)) in poses.texts {
            if self.text(id) != Some(&text) {
                posed.replace_text(id, text)?;
            }
            posed.keep_text_size(id, size);
        }
        for (id, (gear, values)) in poses.gears {
            if self.gear(id) != Some(&gear) {
                posed.replace_gear(id, gear)?;
            }
            posed.keep_gear_values(id, values);
        }
        Ok(Cow::Owned(posed))
    }

    fn posed_text(
        &self,
        id: EntityId,
        to: TextSize,
    ) -> Result<Option<(Text, TextSize)>, SketchError> {
        let (Some(text), Some(size)) = (self.text(id), self.text_size(id)) else {
            return Ok(None);
        };
        let mut posed = text.clone();
        let mut after = size;

        if is_free(&text.height) && differs(size.height, to.height) {
            posed.height = literal_like(&text.height, to.height, Unit::Millimetre);
            after.height = to.height;
        }
        if is_free(&text.angle) && angle_between(size.angle, to.angle).abs() > UNCHANGED {
            posed.angle = literal_like(&text.angle, to.angle, Unit::Degree);
            after.angle = to.angle;
        }

        let after = after.checked().map_err(|reason| SketchError::Text {
            text: id,
            label: self.entity_label(id),
            reason,
        })?;
        Ok(Some((posed, after)))
    }

    fn gear_failed(&self, id: EntityId, reason: GearValueError) -> SketchError {
        SketchError::Gear {
            gear: id,
            label: self.entity_label(id),
            reason,
        }
    }
}

fn posed_gear(
    gear: &Gear,
    values: GearValues,
    to: GearPose,
) -> Result<(Gear, GearValues, Change), GearValueError> {
    let mut posed = gear.clone();
    let mut after = values;
    let mut change = Change::NONE;

    if is_free(&gear.turn) && angle_between(values.turn, to.turn).abs() > UNCHANGED {
        posed.turn = literal_like(&gear.turn, to.turn, Unit::Degree);
        after.turn = to.turn;
        change.turned = angle_between(values.turn, to.turn);
    }
    if let GearShape::Spur(spur) = values.shape
        && is_free(&gear.module)
    {
        let ratio = to.pitch_radius / values.shape.pitch_radius();

        if ratio.is_finite() && ratio > 0.0 && differs(ratio, 1.0) {
            let mut scaled = SpurGear {
                module: spur.module * ratio,
                ..spur
            };

            posed.module = literal_like(&gear.module, scaled.module, Unit::Millimetre);
            if is_free(&gear.root_fillet) {
                scaled.root_fillet = spur.root_fillet * ratio;
                posed.root_fillet =
                    literal_like(&gear.root_fillet, scaled.root_fillet, Unit::Millimetre);
            }
            if is_free(&gear.bore) {
                scaled.bore = spur.bore * ratio;
                posed.bore = literal_like(&gear.bore, scaled.bore, Unit::Millimetre);
            }
            scaled
                .check()
                .map_err(|error| GearValueError::Build(Box::new(error)))?;
            after.shape = GearShape::Spur(scaled);
            change.ratio = ratio;
        }
    }
    Ok((posed, after, change))
}

fn posed_partner(
    gear: &Gear,
    values: GearValues,
    driver: &GearValues,
    change: Change,
) -> Result<(Gear, GearValues), GearValueError> {
    let (Some(meshing), Some(placement)) = (&gear.meshing, values.placement) else {
        return Err(GearValueError::DriverUnknown);
    };
    let mut posed = gear.clone();
    let mut direction = placement.direction;
    let mut bore = values.shape.bore();

    if is_free(&meshing.direction) && change.turned.abs() > UNCHANGED {
        direction += change.turned;
        if let Some(posed) = &mut posed.meshing {
            posed.direction = literal_like(&meshing.direction, direction, Unit::Degree);
        }
    }
    if is_free(&gear.bore) && differs(change.ratio, 1.0) {
        bore *= change.ratio;
        posed.bore = literal_like(&gear.bore, bore, Unit::Millimetre);
    }

    let after = meshing_values(
        meshing.driver,
        driver,
        values.shape.teeth(),
        bore,
        direction,
    )?;
    Ok((posed, after))
}

fn is_free(expression: &Expression) -> bool {
    expression.parameters().is_empty()
}

fn differs(from: f64, to: f64) -> bool {
    (to - from).abs() > UNCHANGED * from.abs().max(1.0)
}

fn literal_like(expression: &Expression, value: f64, base: Unit) -> Expression {
    let written = match expression {
        Expression::Measure(_, unit) => Some(*unit),
        Expression::Negate(inner) => match **inner {
            Expression::Measure(_, unit) => Some(unit),
            _ => None,
        },
        _ => None,
    };
    let unit = written
        .filter(|unit| unit.dimension() == base.dimension())
        .filter(|unit| !matches!(unit, Unit::Inch | Unit::Foot))
        .unwrap_or(base);
    Expression::measure(value / unit.in_base_units(), unit)
}
