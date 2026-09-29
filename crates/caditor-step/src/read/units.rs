use crate::{
    part21::Parameter,
    read::graph::{Entity, Graph},
};

const MAX_UNIT_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Units {
    pub length: f64,
    pub angle: f64,
    pub named: bool,
}

impl Default for Units {
    fn default() -> Self {
        Self {
            length: 1.0,
            angle: 1.0,
            named: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Measure {
    Length(f64),
    Angle(f64),
    Other,
}

pub(crate) fn context_units(graph: &Graph<'_>, context: u64) -> Units {
    let Ok(entity) = graph.entity(context) else {
        return Units::default();
    };
    let Ok(assigned) = entity.record("GLOBAL_UNIT_ASSIGNED_CONTEXT") else {
        return Units::default();
    };
    let mut units = Units::default();
    for unit in assigned
        .references(0)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| graph.entity(id).ok())
    {
        match measure(graph, unit, 0) {
            Measure::Length(scale) => {
                units.length = scale;
                units.named = true;
            }
            Measure::Angle(scale) => units.angle = scale,
            Measure::Other => {}
        }
    }
    units
}

fn measure(graph: &Graph<'_>, unit: Entity<'_>, depth: usize) -> Measure {
    if depth > MAX_UNIT_DEPTH {
        return Measure::Other;
    }
    if let Some(si) = unit.find("SI_UNIT") {
        let count = unit
            .instance
            .record("SI_UNIT")
            .map_or(0, |record| record.parameters.len());
        let prefix = count
            .checked_sub(2)
            .and_then(|index| si.get(index).ok())
            .and_then(Parameter::enumeration);
        let name = count
            .checked_sub(1)
            .and_then(|index| si.get(index).ok())
            .and_then(Parameter::enumeration);
        let factor = prefix_factor(prefix);
        return match name {
            Some("METRE") => Measure::Length(1000.0 * factor),
            Some("RADIAN") => Measure::Angle(factor),
            _ => Measure::Other,
        };
    }
    if let Some(converted) = unit.find("CONVERSION_BASED_UNIT") {
        let Some(factor) = converted
            .reference(1)
            .ok()
            .and_then(|id| graph.entity(id).ok())
        else {
            return Measure::Other;
        };
        let Ok(fields) = factor
            .fields()
            .or_else(|_| factor.record("MEASURE_WITH_UNIT"))
        else {
            return Measure::Other;
        };
        let (Ok(value), Some(base)) = (
            fields.real(0),
            fields
                .optional_reference(1)
                .and_then(|id| graph.entity(id).ok()),
        ) else {
            return Measure::Other;
        };
        return match measure(graph, base, depth + 1) {
            Measure::Length(scale) => Measure::Length(value * scale),
            Measure::Angle(scale) => Measure::Angle(value * scale),
            Measure::Other if unit.is("LENGTH_UNIT") => Measure::Length(value),
            Measure::Other if unit.is("PLANE_ANGLE_UNIT") => Measure::Angle(value),
            Measure::Other => Measure::Other,
        };
    }
    Measure::Other
}

fn prefix_factor(prefix: Option<&str>) -> f64 {
    match prefix {
        Some("EXA") => 1e18,
        Some("PETA") => 1e15,
        Some("TERA") => 1e12,
        Some("GIGA") => 1e9,
        Some("MEGA") => 1e6,
        Some("KILO") => 1e3,
        Some("HECTO") => 1e2,
        Some("DECA") => 1e1,
        Some("DECI") => 1e-1,
        Some("CENTI") => 1e-2,
        Some("MILLI") => 1e-3,
        Some("MICRO") => 1e-6,
        Some("NANO") => 1e-9,
        Some("PICO") => 1e-12,
        Some("FEMTO") => 1e-15,
        Some("ATTO") => 1e-18,
        _ => 1.0,
    }
}
