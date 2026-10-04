use std::fmt::{self, Write};

use super::{APPLICATION, ExportError, MeshBody, three_mf::Coordinate};

pub(super) fn encode(bodies: &[MeshBody<'_>]) -> Result<Vec<u8>, ExportError> {
    let mut text = String::new();
    write_objects(&mut text, bodies).map_err(|_| ExportError::Encoding)?;
    Ok(text.into_bytes())
}

fn write_objects(text: &mut String, bodies: &[MeshBody<'_>]) -> fmt::Result {
    writeln!(text, "# {APPLICATION}, millimetres, Z up")?;
    let mut first_vertex = 1_usize;
    for body in bodies.iter().filter(|body| !body.triangles.is_empty()) {
        writeln!(text, "o {}", Name(body.name))?;
        for position in &body.positions {
            writeln!(
                text,
                "v {} {} {}",
                Coordinate(position.x),
                Coordinate(position.y),
                Coordinate(position.z)
            )?;
        }
        for triangle in &body.triangles {
            let [a, b, c] = triangle.map(|index| first_vertex + index as usize);
            writeln!(text, "f {a} {b} {c}")?;
        }
        first_vertex += body.positions.len();
    }
    Ok(())
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
