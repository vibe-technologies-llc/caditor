use caditor_kernel::LINEAR_RESOLUTION;

use crate::{
    part21::Parameter,
    read::graph::{Entity, Graph},
};

const MAX_UNIT_DEPTH: usize = 8;
const COARSEST_PRECISION: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Units {
    pub length: f64,
    pub angle: f64,
    pub named: bool,
    pub precision: Option<f64>,
}

impl Default for Units {
    fn default() -> Self {
        Self {
            length: 1.0,
            angle: 1.0,
            named: false,
            precision: None,
        }
    }
}

impl Units {
    pub fn uncertainty(&self) -> f64 {
        self.precision.unwrap_or(LINEAR_RESOLUTION)
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
    let mut units = assigned_units(graph, entity);
    units.precision = declared_precision(graph, entity, units.length);
    units
}

fn assigned_units(graph: &Graph<'_>, context: Entity<'_>) -> Units {
    let Some(assigned) = context.find("GLOBAL_UNIT_ASSIGNED_CONTEXT") else {
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

fn declared_precision(graph: &Graph<'_>, context: Entity<'_>, length: f64) -> Option<f64> {
    let assigned = context.find("GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT")?;
    assigned
        .references(0)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| graph.entity(id).ok())
        .filter_map(|uncertainty| length_uncertainty(graph, uncertainty, length))
        .max_by(f64::total_cmp)
        .map(|precision| precision.clamp(LINEAR_RESOLUTION, COARSEST_PRECISION))
}

fn length_uncertainty(graph: &Graph<'_>, uncertainty: Entity<'_>, length: f64) -> Option<f64> {
    let fields = uncertainty
        .find("MEASURE_WITH_UNIT")
        .or_else(|| uncertainty.fields().ok())?;
    let value = fields.get(0).ok()?;
    let amount = value
        .real()
        .filter(|amount| amount.is_finite() && *amount > 0.0)?;
    let unit = fields
        .optional_reference(1)
        .and_then(|id| graph.entity(id).ok())
        .map(|unit| measure(graph, unit, 0));
    let typed_length =
        matches!(value, Parameter::Typed(typed) if typed.0.as_str() == "LENGTH_MEASURE");
    match unit {
        Some(Measure::Length(scale)) => Some(amount * scale),
        Some(Measure::Angle(_)) => None,
        _ if typed_length => Some(amount * length),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::part21::parse;

    fn precision(uncertainties: &str) -> Option<f64> {
        let text = format!(
            "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n\
             #1=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));\n\
             #2=(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.));\n\
             #3=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));\n\
             {uncertainties}\n\
             #9=(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#4,#5)) \
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2)) REPRESENTATION_CONTEXT('',''));\n\
             ENDSEC;\nEND-ISO-10303-21;\n"
        );
        let exchange = parse(&text).unwrap();
        context_units(&Graph::new(&exchange), 9).precision
    }

    #[test]
    fn the_declared_precision_is_converted_to_millimetres_and_kept_within_bounds() {
        let metres = precision(
            "#4=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(2.E-6),#1,'distance_accuracy_value','');",
        );
        let millimetres = precision(
            "#4=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(5.E-4),#3,'distance_accuracy_value','');",
        );
        let coarsest_length = precision(
            "#4=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-6),#1,'',''); \
             #5=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(3.E-6),#1,'','');",
        );
        let complex = precision(
            "#4=(LENGTH_MEASURE_WITH_UNIT() MEASURE_WITH_UNIT(LENGTH_MEASURE(4.E-6),#1) \
             UNCERTAINTY_MEASURE_WITH_UNIT('distance_accuracy_value',''));",
        );
        let unitless =
            precision("#4=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-6),$,'','');");
        let finer_than_caditor = precision(
            "#4=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-7),#3,'distance_accuracy_value','');",
        );
        let hostile = precision(
            "#4=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E300),#1,'distance_accuracy_value','');",
        );
        let unusable = [
            "#4=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.),#1,'distance_accuracy_value','');",
            "#4=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(-1.),#1,'distance_accuracy_value','');",
            "#4=UNCERTAINTY_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(1.E-3),#2,'angle','');",
            "#4=UNCERTAINTY_MEASURE_WITH_UNIT(RATIO_MEASURE(1.E-3),$,'ratio','');",
            "",
        ]
        .map(precision);

        let near = |value: Option<f64>, expected: f64| {
            value.is_some_and(|value| (value - expected).abs() <= 1e-12 * expected)
        };
        assert!(near(metres, 2e-3), "{metres:?}");
        assert!(near(millimetres, 5e-4), "{millimetres:?}");
        assert!(near(coarsest_length, 3e-3), "{coarsest_length:?}");
        assert!(near(complex, 4e-3), "{complex:?}");
        assert!(near(unitless, 1e-3), "{unitless:?}");
        assert_eq!(finer_than_caditor, Some(LINEAR_RESOLUTION));
        assert_eq!(hostile, Some(COARSEST_PRECISION));
        assert_eq!(unusable, [None; 5]);
    }
}
