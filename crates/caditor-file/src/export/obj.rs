use std::{
    fmt::{self, Write},
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use caditor_document::ModelProperties;

use super::{APPLICATION, ExportError, Look, MeshBody, exported_properties, three_mf::Coordinate};

const LIBRARY_EXTENSION: &str = "mtl";
const OWN_HEADER: &str = "# caditor ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Library {
    pub path: PathBuf,
    file_name: String,
}

impl Library {
    pub fn beside(path: &Path, bodies: &[MeshBody<'_>]) -> Option<Self> {
        if bodies.iter().all(|body| body.look.is_none()) {
            return None;
        }
        let path = path.with_extension(LIBRARY_EXTENSION);
        let file_name = path.file_name()?.to_str()?.to_owned();
        replaceable(&path).then_some(Self { path, file_name })
    }
}

fn replaceable(path: &Path) -> bool {
    match File::open(path) {
        Ok(file) => {
            let mut start = Vec::with_capacity(OWN_HEADER.len());
            let read = file.take(OWN_HEADER.len() as u64).read_to_end(&mut start);
            read.is_ok() && start == OWN_HEADER.as_bytes()
        }
        Err(error) => error.kind() == io::ErrorKind::NotFound,
    }
}

pub(super) fn encode(
    bodies: &[MeshBody<'_>],
    library: Option<&Library>,
    properties: &ModelProperties,
) -> Result<Vec<u8>, ExportError> {
    let mut text = String::new();
    write_objects(&mut text, bodies, library, properties).map_err(|_| ExportError::Encoding)?;
    Ok(text.into_bytes())
}

pub(super) fn encode_library(bodies: &[MeshBody<'_>]) -> Result<Vec<u8>, ExportError> {
    let mut text = String::new();
    write_library(&mut text, bodies).map_err(|_| ExportError::Encoding)?;
    Ok(text.into_bytes())
}

fn write_objects(
    text: &mut String,
    bodies: &[MeshBody<'_>],
    library: Option<&Library>,
    properties: &ModelProperties,
) -> fmt::Result {
    writeln!(text, "# {APPLICATION}, millimetres, Z up")?;
    for (property, value) in exported_properties(properties) {
        let line: String = value
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        writeln!(text, "# {}: {line}", property.label())?;
    }
    if let Some(library) = library {
        writeln!(text, "mtllib {}", library.file_name)?;
    }
    let mut first_vertex = 1_usize;
    for (index, body) in bodies
        .iter()
        .enumerate()
        .filter(|(_, body)| !body.triangles.is_empty())
    {
        writeln!(text, "o {}", Name(body.name))?;
        for thread in body.threads {
            writeln!(text, "# Thread: {}", thread.designation)?;
        }
        for position in &body.positions {
            writeln!(
                text,
                "v {} {} {}",
                Coordinate(position.x),
                Coordinate(position.y),
                Coordinate(position.z)
            )?;
        }
        if library.is_some()
            && let Some(look) = body.look
        {
            writeln!(text, "usemtl {}", MaterialName { index, body, look })?;
        }
        for triangle in &body.triangles {
            let [a, b, c] = triangle.map(|index| first_vertex + index as usize);
            writeln!(text, "f {a} {b} {c}")?;
        }
        first_vertex += body.positions.len();
    }
    Ok(())
}

fn write_library(text: &mut String, bodies: &[MeshBody<'_>]) -> fmt::Result {
    writeln!(text, "# {APPLICATION}, materials")?;
    for (index, body) in bodies.iter().enumerate() {
        let Some(look) = body.look else {
            continue;
        };
        let channel = |value: u8| f64::from(value) / 255.0;
        writeln!(text, "newmtl {}", MaterialName { index, body, look })?;
        writeln!(
            text,
            "Kd {:.4} {:.4} {:.4}",
            channel(look.colour.red),
            channel(look.colour.green),
            channel(look.colour.blue)
        )?;
        writeln!(text, "d 1\nillum 1")?;
    }
    Ok(())
}

struct MaterialName<'a, 'b> {
    index: usize,
    body: &'b MeshBody<'a>,
    look: Look<'a>,
}

impl fmt::Display for MaterialName<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let named = Name(self.look.material.unwrap_or(self.body.name)).to_string();
        let joined: String = named
            .chars()
            .map(|character| {
                if character.is_whitespace() {
                    '_'
                } else {
                    character
                }
            })
            .collect();
        write!(formatter, "{}_{joined}", self.index + 1)
    }
}

struct Name<'a>(&'a str);

impl fmt::Display for Name<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cleaned: String = self
            .0
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        match cleaned.trim() {
            "" => formatter.write_str("body"),
            name => formatter.write_str(name),
        }
    }
}
