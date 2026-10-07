use std::fmt::{self, Write};

use caditor_document::{ModelProperties, ModelProperty};

use super::{
    APPLICATION, ExportError, Look, MeshBody, exported_properties,
    zip::{self, ZipEntry},
};

pub(super) const MODEL_PATH: &str = "3D/3dmodel.model";
pub(super) const CONTENT_TYPES_PATH: &str = "[Content_Types].xml";
pub(super) const RELATIONSHIPS_PATH: &str = "_rels/.rels";

const CONTENT_TYPES: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    r#"<Default Extension="rels" "#,
    r#"ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    r#"<Default Extension="model" "#,
    r#"ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
    "</Types>",
);

const RELATIONSHIPS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Target="/3D/3dmodel.model" Id="rel0" "#,
    r#"Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>"#,
    "</Relationships>",
);

const CORE_NAMESPACE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";

pub(super) fn encode(
    bodies: &[MeshBody<'_>],
    properties: &ModelProperties,
) -> Result<Vec<u8>, ExportError> {
    let mut model = String::new();
    write_model(&mut model, bodies, properties).map_err(|_| ExportError::Encoding)?;
    zip::archive(&[
        ZipEntry {
            name: CONTENT_TYPES_PATH,
            contents: CONTENT_TYPES.as_bytes(),
        },
        ZipEntry {
            name: RELATIONSHIPS_PATH,
            contents: RELATIONSHIPS.as_bytes(),
        },
        ZipEntry {
            name: MODEL_PATH,
            contents: model.as_bytes(),
        },
    ])
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

fn write_model(
    xml: &mut String,
    bodies: &[MeshBody<'_>],
    properties: &ModelProperties,
) -> fmt::Result {
    write!(
        xml,
        r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="{CORE_NAMESPACE}"><metadata name="Application">{APPLICATION}</metadata>"#
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
    xml.push_str("<resources>");
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
        xml.push_str("</basematerials>");
    }
    let mut next_look = 0;
    for (object, body) in object_ids(bodies) {
        write!(
            xml,
            r#"<object id="{object}" type="model" name="{}""#,
            Escaped(body.name)
        )?;
        if body.look.is_some() {
            write!(xml, r#" pid="{materials}" pindex="{next_look}""#)?;
            next_look += 1;
        }
        xml.push_str("><mesh><vertices>");
        for position in &body.positions {
            write!(
                xml,
                r#"<vertex x="{}" y="{}" z="{}"/>"#,
                Coordinate(position.x),
                Coordinate(position.y),
                Coordinate(position.z)
            )?;
        }
        xml.push_str("</vertices><triangles>");
        for [a, b, c] in &body.triangles {
            write!(xml, r#"<triangle v1="{a}" v2="{b}" v3="{c}"/>"#)?;
        }
        xml.push_str("</triangles></mesh></object>");
    }
    xml.push_str("</resources><build>");
    let part_number = match bodies {
        [_] if !properties.part_number.is_empty() => Some(&properties.part_number),
        _ => None,
    };
    for (object, _) in object_ids(bodies) {
        write!(xml, r#"<item objectid="{object}""#)?;
        if let Some(part_number) = part_number {
            write!(xml, r#" partnumber="{}""#, Escaped(part_number))?;
        }
        xml.push_str("/>");
    }
    xml.push_str("</build></model>");
    Ok(())
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
