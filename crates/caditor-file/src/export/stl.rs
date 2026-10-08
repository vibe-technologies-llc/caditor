use std::io::{self, Write};

use caditor_document::CancelToken;
use caditor_geometry::{Aabb, Point3, Vector3};

use super::{APPLICATION, ExportError, MeshBody, three_mf::Coordinate, writing};

pub(super) const HEADER_LENGTH: usize = 80;
pub(super) const FACET_LENGTH: usize = 50;
pub(super) const PRECISE_REACH: f64 = 32_768.0;
const VECTOR_LENGTH: usize = 12;
const FLOAT_LENGTH: usize = 4;
const UNNAMED_SOLID: &str = "body";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StlEncoding {
    #[default]
    Binary,
    Text,
}

impl StlEncoding {
    pub const ALL: [Self; 2] = [Self::Binary, Self::Text];

    pub fn name(self) -> &'static str {
        match self {
            Self::Binary => "Binary",
            Self::Text => "Text",
        }
    }
}

pub(super) fn offset(bodies: &[MeshBody<'_>]) -> Option<Vector3> {
    let bounds = Aabb::from_points(
        bodies
            .iter()
            .flat_map(|body| body.positions.iter().copied()),
    )?;
    let reach = bounds.min().abs().max(bounds.max().abs()).max_element();
    (reach > PRECISE_REACH).then(|| Vector3::ZERO - bounds.center().round())
}

pub(super) fn write_binary(
    out: &mut impl Write,
    bodies: &[MeshBody<'_>],
    offset: Option<Vector3>,
    cancel: &CancelToken,
) -> Result<(), ExportError> {
    let total: usize = bodies.iter().map(|body| body.corners().count()).sum();
    let count = u32::try_from(total).map_err(|_| ExportError::TooLarge)?;
    out.write_all(&header(offset)).map_err(writing)?;
    out.write_all(&count.to_le_bytes()).map_err(writing)?;
    let shift = offset.unwrap_or(Vector3::ZERO);
    for body in bodies {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        for [a, b, c] in body.corners() {
            let vectors = [normal(a, b, c), a + shift, b + shift, c + shift];
            let mut facet = [0; FACET_LENGTH];
            for (slot, vector) in facet
                .as_chunks_mut::<VECTOR_LENGTH>()
                .0
                .iter_mut()
                .zip(vectors)
            {
                for (bytes, coordinate) in slot
                    .as_chunks_mut::<FLOAT_LENGTH>()
                    .0
                    .iter_mut()
                    .zip(vector.to_array())
                {
                    *bytes = (coordinate as f32).to_le_bytes();
                }
            }
            out.write_all(&facet).map_err(writing)?;
        }
    }
    Ok(())
}

pub(super) fn write_text(
    out: &mut impl Write,
    bodies: &[MeshBody<'_>],
    cancel: &CancelToken,
) -> Result<(), ExportError> {
    for body in bodies {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        write_solid(out, body).map_err(writing)?;
    }
    Ok(())
}

fn write_solid(out: &mut impl Write, body: &MeshBody<'_>) -> io::Result<()> {
    let name = solid_name(body.name);
    writeln!(out, "solid {name}")?;
    for [a, b, c] in body.corners() {
        let facing = normal(a, b, c);
        writeln!(
            out,
            "  facet normal {} {} {}\n    outer loop",
            Coordinate(facing.x),
            Coordinate(facing.y),
            Coordinate(facing.z)
        )?;
        for corner in [a, b, c] {
            writeln!(
                out,
                "      vertex {} {} {}",
                Coordinate(corner.x),
                Coordinate(corner.y),
                Coordinate(corner.z)
            )?;
        }
        writeln!(out, "    endloop\n  endfacet")?;
    }
    writeln!(out, "endsolid {name}")
}

fn normal(a: Point3, b: Point3, c: Point3) -> Vector3 {
    (b - a).cross(c - a).normalize_or_zero()
}

pub(super) fn solid_name(name: &str) -> String {
    let named: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_graphic() {
                character
            } else {
                '_'
            }
        })
        .collect();
    if named.is_empty() {
        UNNAMED_SOLID.to_owned()
    } else {
        named
    }
}

fn header(offset: Option<Vector3>) -> [u8; HEADER_LENGTH] {
    let plain = format!("{APPLICATION} binary STL, millimetres");
    let label = match offset {
        Some(offset) => {
            let moved = format!(
                "{APPLICATION} binary STL, mm, moved by {} {} {}",
                offset.x, offset.y, offset.z
            );
            if moved.len() <= HEADER_LENGTH {
                moved
            } else {
                plain
            }
        }
        None => plain,
    };
    let mut header = [b' '; HEADER_LENGTH];
    for (slot, byte) in header.iter_mut().zip(label.bytes()) {
        *slot = byte;
    }
    header
}
