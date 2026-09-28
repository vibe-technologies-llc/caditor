use caditor_geometry::{Point3, Vector3};

use super::{APPLICATION, ExportError, MeshBody};

pub(super) const HEADER_LENGTH: usize = 80;
pub(super) const FACET_LENGTH: usize = 50;
const COUNT_LENGTH: usize = 4;
const NO_ATTRIBUTES: [u8; 2] = [0; 2];

pub(super) fn encode(bodies: &[MeshBody<'_>]) -> Result<Vec<u8>, ExportError> {
    let facets: Vec<[Point3; 3]> = bodies.iter().flat_map(MeshBody::corners).collect();
    let count = u32::try_from(facets.len()).map_err(|_| ExportError::TooLarge)?;
    let length = facets
        .len()
        .checked_mul(FACET_LENGTH)
        .and_then(|facets| facets.checked_add(HEADER_LENGTH + COUNT_LENGTH))
        .ok_or(ExportError::TooLarge)?;
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(&header());
    bytes.extend_from_slice(&count.to_le_bytes());
    for [a, b, c] in facets {
        let normal = (b - a).cross(c - a).normalize_or_zero();
        for vector in [normal, a, b, c] {
            push_vector(&mut bytes, vector);
        }
        bytes.extend_from_slice(&NO_ATTRIBUTES);
    }
    Ok(bytes)
}

fn header() -> [u8; HEADER_LENGTH] {
    let mut header = [b' '; HEADER_LENGTH];
    let label = format!("{APPLICATION} binary STL, millimetres");
    for (slot, byte) in header.iter_mut().zip(label.bytes()) {
        *slot = byte;
    }
    header
}

fn push_vector(bytes: &mut Vec<u8>, vector: Vector3) {
    for coordinate in vector.to_array() {
        bytes.extend_from_slice(&(coordinate as f32).to_le_bytes());
    }
}
