mod shape;
#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, fmt, time::SystemTime};

use caditor_geometry::{Point3, Vector3};
use caditor_kernel::Solid;

use crate::write::shape::Shapes;

pub const SCHEMA: &str = "AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }";
const APPLICATION: &str = concat!("caditor ", env!("CARGO_PKG_VERSION"));
const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Debug, Clone, Copy)]
pub struct StepBody<'a> {
    pub name: &'a str,
    pub solid: &'a Solid,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("there are no bodies to write")]
    Empty,
    #[error("the body “{0}” has geometry that STEP cannot hold")]
    Geometry(String),
    #[error("the body “{0}” has several shells that could not be told apart as outer and inner")]
    Shells(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Ref(usize);

impl fmt::Display for Ref {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{}", self.0)
    }
}

#[derive(Default)]
pub(crate) struct Data {
    entities: Vec<String>,
    unwritable: bool,
    points: BTreeMap<[u64; 3], Ref>,
    directions: BTreeMap<[u64; 3], Ref>,
    placements: BTreeMap<[Ref; 3], Ref>,
}

fn coordinate_key(coordinates: [f64; 3]) -> [u64; 3] {
    coordinates.map(|value| (value + 0.0).to_bits())
}

impl Data {
    pub fn add(&mut self, entity: impl Into<String>) -> Ref {
        self.entities.push(entity.into());
        Ref(self.entities.len())
    }

    pub fn real(&mut self, value: f64) -> String {
        if !value.is_finite() {
            self.unwritable = true;
            return "0.".to_owned();
        }
        real(value)
    }

    pub fn reals(&mut self, values: impl IntoIterator<Item = f64>) -> String {
        let values: Vec<String> = values.into_iter().map(|value| self.real(value)).collect();
        list(values)
    }

    pub fn point(&mut self, point: Point3) -> Ref {
        let key = coordinate_key([point.x, point.y, point.z]);
        if let Some(existing) = self.points.get(&key) {
            return *existing;
        }
        let coordinates = self.reals([point.x, point.y, point.z]);
        let written = self.add(format!("CARTESIAN_POINT('',{coordinates})"));
        self.points.insert(key, written);
        written
    }

    pub fn direction(&mut self, direction: Vector3) -> Ref {
        let unit = direction.normalize_or_zero();
        let key = coordinate_key([unit.x, unit.y, unit.z]);
        if let Some(existing) = self.directions.get(&key) {
            return *existing;
        }
        let coordinates = self.reals([unit.x, unit.y, unit.z]);
        let written = self.add(format!("DIRECTION('',{coordinates})"));
        self.directions.insert(key, written);
        written
    }

    pub fn placement(&mut self, origin: Point3, axis: Vector3, reference: Vector3) -> Ref {
        let parts = [
            self.point(origin),
            self.direction(axis),
            self.direction(reference),
        ];
        if let Some(existing) = self.placements.get(&parts) {
            return *existing;
        }
        let [origin, axis, reference] = parts;
        let written = self.add(format!(
            "AXIS2_PLACEMENT_3D('',{origin},{axis},{reference})"
        ));
        self.placements.insert(parts, written);
        written
    }

    fn checkpoint(&self) -> usize {
        self.entities.len()
    }

    fn roll_back(&mut self, checkpoint: usize) {
        self.entities.truncate(checkpoint);
        self.unwritable = false;
        self.points.retain(|_, written| written.0 <= checkpoint);
        self.directions.retain(|_, written| written.0 <= checkpoint);
        self.placements.retain(|_, written| written.0 <= checkpoint);
    }

    fn take_unwritable(&mut self) -> bool {
        std::mem::take(&mut self.unwritable)
    }
}

pub(crate) fn real(value: f64) -> String {
    let shortest = format!("{value:?}");
    match shortest.split_once('e') {
        Some((mantissa, exponent)) => {
            let mantissa = if mantissa.contains('.') {
                mantissa.to_owned()
            } else {
                format!("{mantissa}.")
            };
            format!("{mantissa}E{exponent}")
        }
        None if shortest.contains('.') => shortest,
        None => format!("{shortest}."),
    }
}

pub(crate) fn text(value: &str) -> String {
    let mut quoted = String::from("'");
    for character in value.chars() {
        match character {
            '\'' => quoted.push_str("''"),
            '\\' => quoted.push_str("\\\\"),
            ' '..='~' => quoted.push(character),
            other if other.len_utf16() == 1 => {
                quoted.push_str(&format!("\\X2\\{:04X}\\X0\\", u32::from(other)));
            }
            other => quoted.push_str(&format!("\\X4\\{:08X}\\X0\\", u32::from(other))),
        }
    }
    quoted.push('\'');
    quoted
}

pub(crate) fn list<T: fmt::Display>(items: impl IntoIterator<Item = T>) -> String {
    let items: Vec<String> = items.into_iter().map(|item| item.to_string()).collect();
    format!("({})", items.join(","))
}

pub(crate) fn logical(value: bool) -> &'static str {
    if value { ".T." } else { ".F." }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepWritten {
    pub text: String,
    pub left_out: Vec<(usize, WriteError)>,
}

pub fn write_step(
    bodies: &[StepBody<'_>],
    model_name: &str,
    written: SystemTime,
) -> Result<String, WriteError> {
    let StepWritten { text, left_out } =
        write_step_keeping_what_can_be(bodies, model_name, written)?;
    match left_out.into_iter().next() {
        Some((_, error)) => Err(error),
        None => Ok(text),
    }
}

pub fn write_step_keeping_what_can_be(
    bodies: &[StepBody<'_>],
    model_name: &str,
    written: SystemTime,
) -> Result<StepWritten, WriteError> {
    if bodies.is_empty() {
        return Err(WriteError::Empty);
    }
    let mut data = Data::default();
    let application =
        data.add("APPLICATION_CONTEXT('core data for automotive mechanical design processes')");
    data.add(format!(
        "APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,{application})"
    ));
    let product_context = data.add(format!("PRODUCT_CONTEXT('',{application},'mechanical')"));
    let product_name = match bodies {
        [only] => only.name,
        _ => model_name,
    };
    let name = text(product_name);
    let product = data.add(format!("PRODUCT({name},{name},'',({product_context}))"));
    data.add(format!(
        "PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,({product}))"
    ));
    let formation = data.add(format!("PRODUCT_DEFINITION_FORMATION('','',{product})"));
    let definition_context = data.add(format!(
        "PRODUCT_DEFINITION_CONTEXT('part definition',{application},'design')"
    ));
    let definition = data.add(format!(
        "PRODUCT_DEFINITION('design','',{formation},{definition_context})"
    ));
    let shape = data.add(format!("PRODUCT_DEFINITION_SHAPE('','',{definition})"));
    let context = representation_context(&mut data);
    let mut items = vec![Shapes::origin(&mut data)];
    let mut shapes = Shapes::new(&mut data);
    let mut left_out = Vec::new();
    for (index, body) in bodies.iter().enumerate() {
        let checkpoint = shapes.data().checkpoint();
        let solids = shapes.body(body.solid, body.name);
        let outcome = match solids {
            _ if shapes.data().take_unwritable() => Err(WriteError::Geometry(body.name.to_owned())),
            Ok(solids) => Ok(solids),
            Err(shape::Unsupported::Geometry) => Err(WriteError::Geometry(body.name.to_owned())),
            Err(shape::Unsupported::Shells) => Err(WriteError::Shells(body.name.to_owned())),
        };
        match outcome {
            Ok(solids) => items.extend(solids),
            Err(error) => {
                shapes.data().roll_back(checkpoint);
                left_out.push((index, error));
            }
        }
    }
    if left_out.len() == bodies.len() {
        return Err(left_out.remove(0).1);
    }
    let representation = data.add(format!(
        "ADVANCED_BREP_SHAPE_REPRESENTATION('',{},{context})",
        list(items)
    ));
    data.add(format!(
        "SHAPE_DEFINITION_REPRESENTATION({shape},{representation})"
    ));
    Ok(StepWritten {
        text: document(&data, model_name, written),
        left_out,
    })
}

fn representation_context(data: &mut Data) -> Ref {
    let length = data.add("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))");
    let angle = data.add("(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.))");
    let solid_angle = data.add("(NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT())");
    let uncertainty = data.add(format!(
        "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE({}),{length},'distance_accuracy_value','confusion accuracy')",
        real(caditor_kernel::LINEAR_RESOLUTION)
    ));
    data.add(format!(
        "(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT(({uncertainty})) GLOBAL_UNIT_ASSIGNED_CONTEXT(({length},{angle},{solid_angle})) REPRESENTATION_CONTEXT('3D','3D context with units and uncertainty'))"
    ))
}

fn document(data: &Data, model_name: &str, written: SystemTime) -> String {
    let mut out = String::new();
    out.push_str("ISO-10303-21;\nHEADER;\n");
    out.push_str(&format!(
        "FILE_DESCRIPTION(({}),'2;1');\n",
        text(model_name)
    ));
    out.push_str(&format!(
        "FILE_NAME({},{},(''),(''),{},{},'');\n",
        text(model_name),
        text(&timestamp(written)),
        text(APPLICATION),
        text(APPLICATION)
    ));
    out.push_str(&format!(
        "FILE_SCHEMA(({}));\nENDSEC;\nDATA;\n",
        text(SCHEMA)
    ));
    for (index, entity) in data.entities.iter().enumerate() {
        out.push_str(&format!("#{}={entity};\n", index + 1));
    }
    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    out
}

fn timestamp(written: SystemTime) -> String {
    let seconds = written
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let days = (seconds / SECONDS_PER_DAY) as i64;
    let of_day = seconds % SECONDS_PER_DAY;
    let (year, month, day) = civil_date(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        of_day / 3600,
        of_day / 60 % 60,
        of_day % 60
    )
}

fn civil_date(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
