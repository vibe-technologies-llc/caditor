mod shape;
#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    fmt::{self, Write},
    time::SystemTime,
};

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
    pub colour: Option<[u8; 3]>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StepDetails<'a> {
    pub title: &'a str,
    pub part_number: &'a str,
    pub revision: &'a str,
    pub description: &'a str,
    pub author: &'a str,
    pub organisation: &'a str,
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
    text: String,
    entities: usize,
    unwritable: bool,
    points: BTreeMap<[u64; 3], Ref>,
    directions: BTreeMap<[u64; 3], Ref>,
    placements: BTreeMap<[Ref; 3], Ref>,
}

fn coordinate_key(coordinates: [f64; 3]) -> [u64; 3] {
    coordinates.map(|value| (value + 0.0).to_bits())
}

impl Data {
    fn after(header: String) -> Self {
        Self {
            text: header,
            ..Self::default()
        }
    }

    pub fn add(&mut self, entity: impl fmt::Display) -> Ref {
        self.entities += 1;
        let _ = writeln!(self.text, "#{}={entity};", self.entities);
        Ref(self.entities)
    }

    fn finish(mut self) -> String {
        self.text.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
        self.text
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

    fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            entities: self.entities,
            length: self.text.len(),
        }
    }

    fn roll_back(&mut self, Checkpoint { entities, length }: Checkpoint) {
        self.text.truncate(length);
        self.entities = entities;
        let checkpoint = entities;
        self.unwritable = false;
        self.points.retain(|_, written| written.0 <= checkpoint);
        self.directions.retain(|_, written| written.0 <= checkpoint);
        self.placements.retain(|_, written| written.0 <= checkpoint);
    }

    fn take_unwritable(&mut self) -> bool {
        std::mem::take(&mut self.unwritable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Checkpoint {
    entities: usize,
    length: usize,
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
    write_step_detailed(bodies, model_name, &StepDetails::default(), written)
}

pub fn write_step_detailed(
    bodies: &[StepBody<'_>],
    model_name: &str,
    details: &StepDetails<'_>,
    written: SystemTime,
) -> Result<StepWritten, WriteError> {
    if bodies.is_empty() {
        return Err(WriteError::Empty);
    }
    let mut data = Data::after(header(model_name, details, written));
    let application =
        data.add("APPLICATION_CONTEXT('core data for automotive mechanical design processes')");
    data.add(format!(
        "APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,{application})"
    ));
    let contexts = Contexts {
        product: data.add(format!("PRODUCT_CONTEXT('',{application},'mechanical')")),
        definition: data.add(format!(
            "PRODUCT_DEFINITION_CONTEXT('part definition',{application},'design')"
        )),
        representation: representation_context(&mut data),
    };
    let product_name = match bodies {
        _ if !details.title.is_empty() => details.title,
        [only] => only.name,
        _ => model_name,
    };
    let id = if details.part_number.is_empty() {
        product_name
    } else {
        details.part_number
    };
    let root = product(
        &mut data,
        &contexts,
        &ProductText {
            id,
            name: product_name,
            description: details.description,
            revision: details.revision,
        },
    );
    let origin = Shapes::origin(&mut data);
    let assembly = bodies.len() > 1;
    let mut items = vec![origin];
    let mut parts = Vec::new();
    let mut shapes = Shapes::new(&mut data);
    let mut left_out = Vec::new();
    let mut coloured = Vec::new();
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
            Ok(solids) => {
                if let Some(colour) = body.colour {
                    coloured.extend(solids.iter().map(|solid| (*solid, colour)));
                }
                if assembly {
                    parts.push(part(shapes.data(), &contexts, body.name, origin, &solids));
                } else {
                    items.extend(solids);
                }
            }
            Err(error) => {
                shapes.data().roll_back(checkpoint);
                left_out.push((index, error));
            }
        }
    }
    if left_out.len() == bodies.len() {
        return Err(left_out.remove(0).1);
    }
    let context = contexts.representation;
    if assembly {
        let representation = data.add(format!(
            "SHAPE_REPRESENTATION('',{},{context})",
            list(std::iter::repeat_n(origin, parts.len() + 1))
        ));
        data.add(format!(
            "SHAPE_DEFINITION_REPRESENTATION({},{representation})",
            root.shape
        ));
        for (index, part) in parts.iter().enumerate() {
            place_part(&mut data, &root, part, representation, index, origin);
        }
    } else {
        let representation = data.add(format!(
            "ADVANCED_BREP_SHAPE_REPRESENTATION('',{},{context})",
            list(items)
        ));
        data.add(format!(
            "SHAPE_DEFINITION_REPRESENTATION({},{representation})",
            root.shape
        ));
    }
    styles(&mut data, &coloured, context);
    Ok(StepWritten {
        text: data.finish(),
        left_out,
    })
}

struct Contexts {
    product: Ref,
    definition: Ref,
    representation: Ref,
}

struct ProductText<'a> {
    id: &'a str,
    name: &'a str,
    description: &'a str,
    revision: &'a str,
}

struct Product {
    definition: Ref,
    shape: Ref,
}

struct Part {
    name: String,
    product: Product,
    representation: Ref,
}

fn product(data: &mut Data, contexts: &Contexts, product: &ProductText<'_>) -> Product {
    let product_context = contexts.product;
    let entity = data.add(format!(
        "PRODUCT({},{},{},({product_context}))",
        text(product.id),
        text(product.name),
        text(product.description)
    ));
    data.add(format!(
        "PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,({entity}))"
    ));
    let formation = data.add(format!(
        "PRODUCT_DEFINITION_FORMATION({},'',{entity})",
        text(product.revision)
    ));
    let definition = data.add(format!(
        "PRODUCT_DEFINITION('design','',{formation},{})",
        contexts.definition
    ));
    let shape = data.add(format!("PRODUCT_DEFINITION_SHAPE('','',{definition})"));
    Product { definition, shape }
}

fn part(data: &mut Data, contexts: &Contexts, name: &str, origin: Ref, solids: &[Ref]) -> Part {
    let product = product(
        data,
        contexts,
        &ProductText {
            id: name,
            name,
            description: "",
            revision: "",
        },
    );
    let representation = data.add(format!(
        "ADVANCED_BREP_SHAPE_REPRESENTATION({},{},{})",
        text(name),
        list(std::iter::once(origin).chain(solids.iter().copied())),
        contexts.representation
    ));
    data.add(format!(
        "SHAPE_DEFINITION_REPRESENTATION({},{representation})",
        product.shape
    ));
    Part {
        name: name.to_owned(),
        product,
        representation,
    }
}

fn place_part(
    data: &mut Data,
    root: &Product,
    part: &Part,
    parent: Ref,
    index: usize,
    origin: Ref,
) {
    let usage = data.add(format!(
        "NEXT_ASSEMBLY_USAGE_OCCURRENCE('{}',{},'',{},{},$)",
        index + 1,
        text(&part.name),
        root.definition,
        part.product.definition
    ));
    let placement_shape = data.add(format!(
        "PRODUCT_DEFINITION_SHAPE('Placement','Placement of an item',{usage})"
    ));
    let transformation = data.add(format!(
        "ITEM_DEFINED_TRANSFORMATION('','',{origin},{origin})"
    ));
    let relationship = data.add(format!(
        "(REPRESENTATION_RELATIONSHIP('','',{},{parent}) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION({transformation}) SHAPE_REPRESENTATION_RELATIONSHIP())",
        part.representation
    ));
    data.add(format!(
        "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION({relationship},{placement_shape})"
    ));
}

fn styles(data: &mut Data, coloured: &[(Ref, [u8; 3])], context: Ref) {
    if coloured.is_empty() {
        return;
    }
    let mut assignments = BTreeMap::new();
    let mut styled = Vec::new();
    for (solid, colour) in coloured {
        let assignment = *assignments
            .entry(*colour)
            .or_insert_with(|| style_assignment(data, *colour));
        styled.push(data.add(format!("STYLED_ITEM('color',({assignment}),{solid})")));
    }
    data.add(format!(
        "MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',{},{context})",
        list(styled)
    ));
}

fn style_assignment(data: &mut Data, [red, green, blue]: [u8; 3]) -> Ref {
    let channel = |value: u8| real(f64::from(value) / 255.0);
    let colour = data.add(format!(
        "COLOUR_RGB('',{},{},{})",
        channel(red),
        channel(green),
        channel(blue)
    ));
    let fill_colour = data.add(format!("FILL_AREA_STYLE_COLOUR('',{colour})"));
    let fill = data.add(format!("FILL_AREA_STYLE('',({fill_colour}))"));
    let area = data.add(format!("SURFACE_STYLE_FILL_AREA({fill})"));
    let side = data.add(format!("SURFACE_SIDE_STYLE('',({area}))"));
    let usage = data.add(format!("SURFACE_STYLE_USAGE(.BOTH.,{side})"));
    data.add(format!("PRESENTATION_STYLE_ASSIGNMENT(({usage}))"))
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

fn header(model_name: &str, details: &StepDetails<'_>, written: SystemTime) -> String {
    let description = if details.description.is_empty() {
        model_name
    } else {
        details.description
    };
    let mut out = String::new();
    out.push_str("ISO-10303-21;\nHEADER;\n");
    out.push_str(&format!(
        "FILE_DESCRIPTION(({}),'2;1');\n",
        text(description)
    ));
    out.push_str(&format!(
        "FILE_NAME({},{},({}),({}),{},{},'');\n",
        text(model_name),
        text(&timestamp(written)),
        text(details.author),
        text(details.organisation),
        text(APPLICATION),
        text(APPLICATION)
    ));
    out.push_str(&format!(
        "FILE_SCHEMA(({}));\nENDSEC;\nDATA;\n",
        text(SCHEMA)
    ));
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
