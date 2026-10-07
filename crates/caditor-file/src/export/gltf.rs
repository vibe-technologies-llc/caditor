use caditor_document::Rgb;
use serde_json::{Value, json};

use super::{APPLICATION, ExportError, MeshBody};

const MAGIC: u32 = 0x4654_6c67;
const VERSION: u32 = 2;
const JSON_CHUNK: u32 = 0x4e4f_534a;
const BINARY_CHUNK: u32 = 0x004e_4942;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;
const TRIANGLES: u32 = 4;
const METRES_PER_MILLIMETRE: f64 = 1e-3;
const ALIGNMENT: usize = 4;
const HEADER_LENGTH: usize = 12;
const CHUNK_HEADER_LENGTH: usize = 8;

struct Parts {
    binary: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    meshes: Vec<Value>,
    nodes: Vec<Value>,
    materials: Vec<Value>,
}

pub(super) fn encode(bodies: &[MeshBody<'_>]) -> Result<Vec<u8>, ExportError> {
    let mut parts = Parts {
        binary: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
        meshes: Vec::new(),
        nodes: Vec::new(),
        materials: Vec::new(),
    };
    for body in bodies.iter().filter(|body| !body.triangles.is_empty()) {
        add_body(&mut parts, body);
    }
    if parts.nodes.is_empty() {
        return Err(ExportError::Empty);
    }
    let binary_length = u32::try_from(parts.binary.len()).map_err(|_| ExportError::TooLarge)?;
    let node_indices: Vec<usize> = (0..parts.nodes.len()).collect();
    let mut document = json!({
        "asset": { "version": "2.0", "generator": APPLICATION },
        "scene": 0,
        "scenes": [{ "nodes": node_indices }],
        "nodes": parts.nodes,
        "meshes": parts.meshes,
        "accessors": parts.accessors,
        "bufferViews": parts.views,
        "buffers": [{ "byteLength": binary_length }],
    });
    if !parts.materials.is_empty()
        && let Some(fields) = document.as_object_mut()
    {
        fields.insert("materials".to_owned(), Value::from(parts.materials));
    }
    let text = serde_json::to_vec(&document).map_err(|_| ExportError::Encoding)?;
    container(text, parts.binary)
}

fn add_body(parts: &mut Parts, body: &MeshBody<'_>) {
    let points: Vec<[f32; 3]> = body
        .positions
        .iter()
        .map(|position| {
            [
                (position.x * METRES_PER_MILLIMETRE) as f32,
                (position.z * METRES_PER_MILLIMETRE) as f32,
                (-position.y * METRES_PER_MILLIMETRE) as f32,
            ]
        })
        .collect();
    let (minimum, maximum) = bounds(&points);

    let position_view = push_view(
        parts,
        points
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes()),
        ARRAY_BUFFER,
    );
    let position_accessor = parts.accessors.len();
    parts.accessors.push(json!({
        "bufferView": position_view,
        "componentType": FLOAT,
        "count": points.len(),
        "type": "VEC3",
        "min": minimum,
        "max": maximum,
    }));

    let index_view = push_view(
        parts,
        body.triangles
            .iter()
            .flatten()
            .flat_map(|index| index.to_le_bytes()),
        ELEMENT_ARRAY_BUFFER,
    );
    let index_accessor = parts.accessors.len();
    parts.accessors.push(json!({
        "bufferView": index_view,
        "componentType": UNSIGNED_INT,
        "count": body.triangles.len() * 3,
        "type": "SCALAR",
    }));

    let mut primitive = json!({
        "attributes": { "POSITION": position_accessor },
        "indices": index_accessor,
        "mode": TRIANGLES,
    });
    if let Some(look) = body.look
        && let Some(fields) = primitive.as_object_mut()
    {
        fields.insert("material".to_owned(), Value::from(parts.materials.len()));
        parts.materials.push(json!({
            "name": look.material.unwrap_or(body.name),
            "pbrMetallicRoughness": {
                "baseColorFactor": linear(look.colour),
                "metallicFactor": 0.0,
            },
        }));
    }
    let mesh = parts.meshes.len();
    parts.meshes.push(json!({
        "name": body.name,
        "primitives": [primitive],
    }));
    parts.nodes.push(json!({ "name": body.name, "mesh": mesh }));
}

fn linear(colour: Rgb) -> [f64; 4] {
    let channel = |value: u8| {
        let encoded = f64::from(value) / 255.0;
        if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        channel(colour.red),
        channel(colour.green),
        channel(colour.blue),
        1.0,
    ]
}

fn bounds(points: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for point in points {
        for axis in 0..3 {
            if let (Some(low), Some(high), Some(value)) = (
                minimum.get_mut(axis),
                maximum.get_mut(axis),
                point.get(axis),
            ) {
                *low = low.min(*value);
                *high = high.max(*value);
            }
        }
    }
    (minimum, maximum)
}

fn push_view(parts: &mut Parts, bytes: impl Iterator<Item = u8>, target: u32) -> usize {
    let offset = parts.binary.len();
    parts.binary.extend(bytes);
    let length = parts.binary.len() - offset;
    pad(&mut parts.binary, 0);
    let view = parts.views.len();
    parts.views.push(json!({
        "buffer": 0,
        "byteOffset": offset,
        "byteLength": length,
        "target": target,
    }));
    view
}

fn pad(bytes: &mut Vec<u8>, with: u8) {
    while !bytes.len().is_multiple_of(ALIGNMENT) {
        bytes.push(with);
    }
}

fn container(mut text: Vec<u8>, mut binary: Vec<u8>) -> Result<Vec<u8>, ExportError> {
    pad(&mut text, b' ');
    pad(&mut binary, 0);
    let total = HEADER_LENGTH + 2 * CHUNK_HEADER_LENGTH + text.len() + binary.len();
    let total = u32::try_from(total).map_err(|_| ExportError::TooLarge)?;
    let text_length = u32::try_from(text.len()).map_err(|_| ExportError::TooLarge)?;
    let binary_length = u32::try_from(binary.len()).map_err(|_| ExportError::TooLarge)?;
    let mut bytes = Vec::with_capacity(total as usize);
    for word in [MAGIC, VERSION, total, text_length, JSON_CHUNK] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&text);
    for word in [binary_length, BINARY_CHUNK] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&binary);
    Ok(bytes)
}
