use std::{
    fmt::{self, Write as _},
    io::{Seek, Write},
};

use caditor_document::{CancelToken, ModelProperties, ModelProperty, ThreadSide};

use super::{
    APPLICATION, ExportError, ExportThread, Look, MeshBody, exported_properties,
    zip::{Deflating, ZipWriter},
};

pub(super) const MODEL_PATH: &str = "3D/3dmodel.model";
pub(super) const CONTENT_TYPES_PATH: &str = "[Content_Types].xml";
pub(super) const RELATIONSHIPS_PATH: &str = "_rels/.rels";
pub(super) const THUMBNAIL_PATH: &str = "Metadata/thumbnail.png";

const CONTENT_TYPES_START: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    r#"<Default Extension="rels" "#,
    r#"ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    r#"<Default Extension="model" "#,
    r#"ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
);
const PNG_CONTENT_TYPE: &str = r#"<Default Extension="png" ContentType="image/png"/>"#;
const CONTENT_TYPES_END: &str = "</Types>";

const RELATIONSHIPS_START: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Target="/3D/3dmodel.model" Id="rel0" "#,
    r#"Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>"#,
);
const THUMBNAIL_RELATIONSHIP: &str = concat!(
    r#"<Relationship Target="/Metadata/thumbnail.png" Id="rel1" "#,
    r#"Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail"/>"#,
);
const RELATIONSHIPS_END: &str = "</Relationships>";

const CORE_NAMESPACE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";

pub(super) const CADITOR_NAMESPACE: &str = "urn:caditor:3mf";
const CADITOR_PREFIX: &str = "caditor";

const FIXED_MODEL_TEXT: u64 = 4096;
const ESCAPED_BYTES: u64 = 6;
const OBJECT_TEXT: u64 = 256;
const THREAD_TEXT: u64 = 1024;
const VERTEX_TEXT: u64 = 24;
const TRIANGLE_TEXT: u64 = 60;

pub(super) fn write(
    out: impl Write + Seek,
    bodies: &[MeshBody<'_>],
    properties: &ModelProperties,
    thumbnail: Option<&[u8]>,
    cancel: &CancelToken,
) -> Result<(), ExportError> {
    let with_thumbnail = |start: &str, thumbnail_part: &str, end: &str| match thumbnail {
        Some(_) => format!("{start}{thumbnail_part}{end}"),
        None => format!("{start}{end}"),
    };
    let mut zip = ZipWriter::new(out);
    zip.add(
        CONTENT_TYPES_PATH,
        with_thumbnail(CONTENT_TYPES_START, PNG_CONTENT_TYPE, CONTENT_TYPES_END).as_bytes(),
    )?;
    zip.add(
        RELATIONSHIPS_PATH,
        with_thumbnail(
            RELATIONSHIPS_START,
            THUMBNAIL_RELATIONSHIP,
            RELATIONSHIPS_END,
        )
        .as_bytes(),
    )?;
    if let Some(png) = thumbnail {
        zip.add(THUMBNAIL_PATH, png)?;
    }
    zip.add_deflated(
        MODEL_PATH,
        model_size_bound(bodies, properties),
        |deflating| write_model(&mut Text::new(deflating), bodies, properties, cancel),
    )?;
    zip.finish()?;
    Ok(())
}

#[cfg(test)]
pub(super) fn encode(
    bodies: &[MeshBody<'_>],
    properties: &ModelProperties,
) -> Result<Vec<u8>, ExportError> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    write(&mut bytes, bodies, properties, None, &CancelToken::never())?;
    Ok(bytes.into_inner())
}

struct Text<'d, 'z, W> {
    deflating: &'d mut Deflating<'z, W>,
    failure: Option<ExportError>,
}

impl<'d, 'z, W: Write + Seek> Text<'d, 'z, W> {
    fn new(deflating: &'d mut Deflating<'z, W>) -> Self {
        Self {
            deflating,
            failure: None,
        }
    }

    fn checked(&mut self, written: fmt::Result) -> Result<(), ExportError> {
        written.map_err(|_| self.failure.take().unwrap_or(ExportError::Encoding))
    }
}

impl<W: Write + Seek> fmt::Write for Text<'_, '_, W> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.deflating
            .write_bytes(text.as_bytes())
            .map_err(|error| {
                self.failure = Some(error);
                fmt::Error
            })
    }
}

fn model_size_bound(bodies: &[MeshBody<'_>], properties: &ModelProperties) -> u64 {
    let escaped = |text: &str| (text.len() as u64).saturating_mul(ESCAPED_BYTES);
    let largest = bodies
        .iter()
        .flat_map(|body| body.positions.iter())
        .map(|position| position.abs().max_element())
        .fold(0.0, f64::max);
    let coordinate = Coordinate(-largest).to_string().len() as u64 + 1;
    let vertex = VERTEX_TEXT.saturating_add(coordinate.saturating_mul(3));
    let described = exported_properties(properties)
        .map(|(_, value)| escaped(value).saturating_add(OBJECT_TEXT))
        .fold(FIXED_MODEL_TEXT, u64::saturating_add);
    bodies.iter().fold(described, |total, body| {
        let named = escaped(body.name)
            .saturating_mul(2)
            .saturating_add(body.look.and_then(|look| look.material).map_or(0, escaped))
            .saturating_add(escaped(&properties.part_number))
            .saturating_add(
                body.threads
                    .iter()
                    .map(|thread| escaped(&thread.designation).saturating_add(THREAD_TEXT))
                    .fold(0, u64::saturating_add),
            )
            .saturating_add(OBJECT_TEXT);
        let mesh = (body.positions.len() as u64)
            .saturating_mul(vertex)
            .saturating_add((body.triangles.len() as u64).saturating_mul(TRIANGLE_TEXT));
        total.saturating_add(named).saturating_add(mesh)
    })
}

fn metadata_name(property: ModelProperty) -> Option<&'static str> {
    match property {
        ModelProperty::Title => Some("Title"),
        ModelProperty::Author => Some("Designer"),
        ModelProperty::Description => Some("Description"),
        ModelProperty::PartNumber
        | ModelProperty::Revision
        | ModelProperty::Organisation
        | ModelProperty::Notes => None,
    }
}

fn write_model<W: Write + Seek>(
    xml: &mut Text<'_, '_, W>,
    bodies: &[MeshBody<'_>],
    properties: &ModelProperties,
    cancel: &CancelToken,
) -> Result<(), ExportError> {
    let written = write_resources_start(xml, bodies, properties);
    xml.checked(written)?;
    let materials = bodies.len() + 1;
    let mut next_look = 0;
    for (object, body) in object_ids(bodies) {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let look = body.look.map(|_| {
            let index = next_look;
            next_look += 1;
            index
        });
        let written = write_object(xml, object, body, materials, look);
        xml.checked(written)?;
    }
    let written = write_build(xml, bodies, properties);
    xml.checked(written)
}

fn write_resources_start(
    xml: &mut impl fmt::Write,
    bodies: &[MeshBody<'_>],
    properties: &ModelProperties,
) -> fmt::Result {
    write!(
        xml,
        r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="{CORE_NAMESPACE}""#
    )?;
    if bodies.iter().any(|body| !body.threads.is_empty()) {
        write!(xml, r#" xmlns:{CADITOR_PREFIX}="{CADITOR_NAMESPACE}""#)?;
    }
    write!(
        xml,
        r#"><metadata name="Application">{APPLICATION}</metadata>"#
    )?;
    for (property, value) in exported_properties(properties) {
        if let Some(name) = metadata_name(property) {
            write!(
                xml,
                r#"<metadata name="{name}">{}</metadata>"#,
                Escaped(value)
            )?;
        }
    }
    xml.write_str("<resources>")?;
    let materials = bodies.len() + 1;
    let looks: Vec<(&MeshBody<'_>, Look<'_>)> = bodies
        .iter()
        .filter_map(|body| Some((body, body.look?)))
        .collect();
    if !looks.is_empty() {
        write!(xml, r#"<basematerials id="{materials}">"#)?;
        for (body, look) in &looks {
            write!(
                xml,
                r#"<base name="{}" displaycolor="{}"/>"#,
                Escaped(look.material.unwrap_or(body.name)),
                look.colour.hex().to_uppercase()
            )?;
        }
        xml.write_str("</basematerials>")?;
    }
    Ok(())
}

fn write_object(
    xml: &mut impl fmt::Write,
    object: usize,
    body: &MeshBody<'_>,
    materials: usize,
    look: Option<usize>,
) -> fmt::Result {
    write!(
        xml,
        r#"<object id="{object}" type="model" name="{}""#,
        Escaped(body.name)
    )?;
    if let Some(look) = look {
        write!(xml, r#" pid="{materials}" pindex="{look}""#)?;
    }
    xml.write_char('>')?;
    write_threads(xml, body.threads)?;
    xml.write_str("<mesh><vertices>")?;
    for position in &body.positions {
        write!(
            xml,
            r#"<vertex x="{}" y="{}" z="{}"/>"#,
            Coordinate(position.x),
            Coordinate(position.y),
            Coordinate(position.z)
        )?;
    }
    xml.write_str("</vertices><triangles>")?;
    for [a, b, c] in &body.triangles {
        write!(xml, r#"<triangle v1="{a}" v2="{b}" v3="{c}"/>"#)?;
    }
    xml.write_str("</triangles></mesh></object>")
}

fn write_threads(xml: &mut impl fmt::Write, threads: &[ExportThread]) -> fmt::Result {
    if threads.is_empty() {
        return Ok(());
    }
    xml.write_str("<metadatagroup>")?;
    for (index, thread) in threads.iter().enumerate() {
        let number = index + 1;
        let side = match thread.side {
            ThreadSide::Internal => "internal",
            ThreadSide::External => "external",
        };
        let entries: [(&str, &dyn fmt::Display); 6] = [
            ("designation", &Escaped(&thread.designation)),
            ("side", &side),
            ("pitch", &Coordinate(thread.pitch)),
            ("length", &Coordinate(thread.length)),
            ("start", &Triple(thread.start.to_array())),
            ("direction", &Triple(thread.direction.to_array())),
        ];
        for (field, value) in entries {
            write!(
                xml,
                r#"<metadata name="{CADITOR_PREFIX}:thread.{number}.{field}" preserve="1">{value}</metadata>"#
            )?;
        }
    }
    xml.write_str("</metadatagroup>")
}

fn write_build(
    xml: &mut impl fmt::Write,
    bodies: &[MeshBody<'_>],
    properties: &ModelProperties,
) -> fmt::Result {
    xml.write_str("</resources><build>")?;
    let part_number = match bodies {
        [_] if !properties.part_number.is_empty() => Some(&properties.part_number),
        _ => None,
    };
    for (object, _) in object_ids(bodies) {
        write!(xml, r#"<item objectid="{object}""#)?;
        if let Some(part_number) = part_number {
            write!(xml, r#" partnumber="{}""#, Escaped(part_number))?;
        }
        xml.write_str("/>")?;
    }
    xml.write_str("</build></model>")
}

fn object_ids<'a, 'b>(
    bodies: &'a [MeshBody<'b>],
) -> impl Iterator<Item = (usize, &'a MeshBody<'b>)> {
    bodies
        .iter()
        .enumerate()
        .map(|(index, body)| (index + 1, body))
}

const COORDINATE_DECIMALS: usize = 6;

pub(super) struct Coordinate(pub(super) f64);

impl fmt::Display for Coordinate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fixed = format!("{:.COORDINATE_DECIMALS$}", self.0);
        let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
        match trimmed {
            "-0" | "" => formatter.write_str("0"),
            trimmed => formatter.write_str(trimmed),
        }
    }
}

struct Triple([f64; 3]);

impl fmt::Display for Triple {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [x, y, z] = self.0;
        write!(
            formatter,
            "{} {} {}",
            Coordinate(x),
            Coordinate(y),
            Coordinate(z)
        )
    }
}

fn xml_character(character: char) -> bool {
    !character.is_control() && !matches!(character, '\u{FFFE}' | '\u{FFFF}')
}

struct Escaped<'a>(&'a str);

impl fmt::Display for Escaped<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for character in self.0.chars() {
            match character {
                '&' => formatter.write_str("&amp;")?,
                '<' => formatter.write_str("&lt;")?,
                '>' => formatter.write_str("&gt;")?,
                '"' => formatter.write_str("&quot;")?,
                '\'' => formatter.write_str("&apos;")?,
                character if !xml_character(character) => formatter.write_char(' ')?,
                character => formatter.write_char(character)?,
            }
        }
        Ok(())
    }
}
