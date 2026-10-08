use std::{collections::BTreeMap, path::Path, sync::Arc, time::SystemTime};

use caditor_document::{
    BodyPlacement, Document, FeatureKind, Import, ParameterValues, Transaction,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Point3, Similarity, Vector3};
use caditor_kernel::{LINEAR_RESOLUTION, Solid};
use caditor_step::{
    Misplacement, ReadError, StepBody, StepCopy, read_step, read_step_copies, write_step,
};

use crate::{
    import::ImportError,
    read::{MAX_FILE_SIZE, read_file},
    reason::ReadFailure,
};

const LATIN_1_NOTE: &str = "The file is not UTF-8 text, so its names were read as Latin-1; \
                            letters outside it may look wrong.";

pub const STEP_IMPORT_EXTENSIONS: [&str; 4] = ["step", "stp", "p21", "stpz"];

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const GZIP_DEFLATE: u8 = 8;
const GZIP_FIXED_HEADER: usize = 10;
const GZIP_TRAILER: usize = 8;
const GZIP_HEADER_CRC: u8 = 2;
const GZIP_EXTRA: u8 = 4;
const GZIP_NAME: u8 = 8;
const GZIP_COMMENT: u8 = 16;
const GZIP_RESERVED: u8 = 0xe0;
const PLACEMENT_DIGITS: f64 = 1e9;
const PLACEMENT_SLACK: f64 = LINEAR_RESOLUTION / 10.0;
const GIMBAL_LOCK: f64 = 1e-9;

type Stored = Option<Vec<(Arc<Solid>, Arc<str>)>>;

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedBody {
    pub name: String,
    pub import: Import,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelImport {
    pub bodies: Vec<ImportedBody>,
    pub notes: Vec<String>,
}

pub fn read_step_file(path: &Path) -> Result<ModelImport, ImportError> {
    let bytes = read_file(path).map_err(|error| ImportError::Reading(ReadFailure::of(&error)))?;
    let bytes = if bytes.starts_with(&GZIP_MAGIC) {
        unpacked(&bytes)?
    } else {
        bytes
    };
    let source = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    match String::from_utf8(bytes) {
        Ok(text) => parse_step(&text, &source),
        Err(error) => {
            let text: String = error
                .as_bytes()
                .iter()
                .map(|byte| char::from(*byte))
                .collect();
            let mut import = parse_step(&text, &source)?;
            import.notes.push(LATIN_1_NOTE.to_owned());
            Ok(import)
        }
    }
}

fn unpacked(bytes: &[u8]) -> Result<Vec<u8>, ImportError> {
    let damaged = || ImportError::DamagedArchive;
    let Some([_, _, method, flags, ..]) = bytes.get(..GZIP_FIXED_HEADER) else {
        return Err(damaged());
    };
    let (method, flags) = (*method, *flags);
    if method != GZIP_DEFLATE || flags & GZIP_RESERVED != 0 {
        return Err(damaged());
    }
    let mut at = GZIP_FIXED_HEADER;
    if flags & GZIP_EXTRA != 0 {
        let Some(&[low, high]) = bytes
            .get(at..at + 2)
            .and_then(|length| <&[u8; 2]>::try_from(length).ok())
        else {
            return Err(damaged());
        };
        at += 2 + usize::from(u16::from_le_bytes([low, high]));
    }
    for text in [GZIP_NAME, GZIP_COMMENT] {
        if flags & text != 0 {
            let rest = bytes.get(at..).ok_or_else(damaged)?;
            at += rest
                .iter()
                .position(|byte| *byte == 0)
                .ok_or_else(damaged)?
                + 1;
        }
    }
    if flags & GZIP_HEADER_CRC != 0 {
        at += 2;
    }
    let end = bytes.len().checked_sub(GZIP_TRAILER).ok_or_else(damaged)?;
    let packed = bytes.get(at..end).ok_or_else(damaged)?;
    let limit = usize::try_from(MAX_FILE_SIZE).unwrap_or(usize::MAX);
    let unpacked =
        miniz_oxide::inflate::decompress_to_vec_with_limit(packed, limit).map_err(|error| {
            match error.status {
                miniz_oxide::inflate::TINFLStatus::HasMoreOutput => ImportError::UnpacksTooLarge,
                _ => damaged(),
            }
        })?;
    let Some(&[c0, c1, c2, c3, s0, s1, s2, s3]) = bytes
        .get(end..)
        .and_then(|trailer| <&[u8; GZIP_TRAILER]>::try_from(trailer).ok())
    else {
        return Err(damaged());
    };
    let checksum = u32::from_le_bytes([c0, c1, c2, c3]);
    let size = u32::from_le_bytes([s0, s1, s2, s3]);
    let matches_size = unpacked.len() as u64 & 0xffff_ffff == u64::from(size);
    if crc32fast::hash(&unpacked) != checksum || !matches_size {
        return Err(damaged());
    }
    Ok(unpacked)
}

pub fn parse_step(text: &str, source: &str) -> Result<ModelImport, ImportError> {
    let model = read_step_copies(text).map_err(|error| match error {
        ReadError::NotStep => ImportError::NotStep,
        other => ImportError::Step(other),
    })?;
    let mut imported = ModelImport {
        bodies: Vec::with_capacity(model.copies.len()),
        notes: model.notes,
    };
    let mut lost = Vec::new();
    let mut unplaceable: Vec<String> = Vec::new();
    let mut stored: BTreeMap<*const Solid, Stored> = BTreeMap::new();
    for copy in model.copies {
        let lumps = stored
            .entry(Arc::as_ptr(&copy.solid))
            .or_insert_with(|| shared(canonical(&copy.name, &copy.solid)))
            .clone();
        let Some(lumps) = lumps else {
            lost.push(copy.name);
            continue;
        };
        match placed_copies(source, &copy, &lumps) {
            Some(bodies) => imported.bodies.extend(bodies),
            None => {
                let StepCopy {
                    name,
                    solid,
                    placement,
                } = copy;
                match solid.mapped(&placement) {
                    Ok(mapped) => match canonical(&name, &mapped) {
                        Some(lumps) => {
                            imported
                                .bodies
                                .extend(lumps.into_iter().map(|(stored, step)| ImportedBody {
                                    import: Import::new(source, stored, step),
                                    name: name.clone(),
                                }))
                        }
                        None => lost.push(name),
                    },
                    Err(_) if unplaceable.contains(&name) => {}
                    Err(_) => unplaceable.push(name),
                }
            }
        }
    }
    for name in lost {
        imported.notes.push(format!(
            "“{name}” was read but could not be stored in the model, so it was left out."
        ));
    }
    imported.notes.extend(
        unplaceable
            .iter()
            .map(|name| Misplacement::CopyUnplaceable.note(name)),
    );
    if imported.bodies.is_empty() {
        return Err(ImportError::NothingStorable);
    }
    Ok(imported)
}

fn shared(lumps: Option<Vec<(Solid, String)>>) -> Stored {
    lumps.map(|lumps| {
        lumps
            .into_iter()
            .map(|(solid, step)| (Arc::new(solid), Arc::from(step)))
            .collect()
    })
}

fn placed_copies(
    source: &str,
    copy: &StepCopy,
    lumps: &[(Arc<Solid>, Arc<str>)],
) -> Option<Vec<ImportedBody>> {
    let placement = if copy.placement == Similarity::IDENTITY {
        BodyPlacement::default()
    } else {
        body_placement(&copy.placement, &copy.solid)?
    };
    Some(
        lumps
            .iter()
            .map(|(solid, step)| ImportedBody {
                import: Import::shared(source, Arc::clone(solid), Arc::clone(step))
                    .placed(placement.clone()),
                name: copy.name.clone(),
            })
            .collect(),
    )
}

fn body_placement(similarity: &Similarity, solid: &Solid) -> Option<BodyPlacement> {
    if !similarity.is_rigid() {
        return None;
    }
    let [x, y, z] = [Vector3::X, Vector3::Y, Vector3::Z].map(|axis| similarity.apply_vector(axis));
    let tilt = (-x.z).clamp(-1.0, 1.0).asin();
    let (about_x, about_z) = if tilt.cos() > GIMBAL_LOCK {
        (y.z.atan2(z.z), x.y.atan2(x.x))
    } else {
        (0.0, (-y.x).atan2(y.y))
    };
    let shift = similarity.apply_point(Point3::ZERO);
    let degrees = |radians: f64| Expression::measure(rounded(radians.to_degrees()), Unit::Degree);
    let millimetres = |length: f64| Expression::measure(rounded(length), Unit::Millimetre);
    let placement = BodyPlacement {
        offset: [shift.x, shift.y, shift.z].map(millimetres),
        turn: [about_x, tilt, about_z].map(degrees),
    };
    let transform = placement.transform(&ParameterValues::default())?;
    let bounds = solid.bounding_box()?;
    let matches = bounds.corners().iter().all(|corner| {
        (transform.apply_point(*corner) - similarity.apply_point(*corner)).length()
            <= PLACEMENT_SLACK
    });
    matches.then_some(placement)
}

fn rounded(value: f64) -> f64 {
    (value * PLACEMENT_DIGITS).round() / PLACEMENT_DIGITS + 0.0
}

pub(crate) fn canonical(name: &str, solid: &Solid) -> Option<Vec<(Solid, String)>> {
    let (step, mut again) = written_and_read(name, solid)?;
    match again.len() {
        0 => return None,
        1 => return Some(vec![(again.swap_remove(0), step)]),
        _ => {}
    }
    again
        .iter()
        .map(|lump| {
            let (step, mut alone) = written_and_read(name, lump)?;
            (alone.len() == 1).then(|| (alone.swap_remove(0), step))
        })
        .collect()
}

fn written_and_read(name: &str, solid: &Solid) -> Option<(String, Vec<Solid>)> {
    let step = write_step(
        &[StepBody {
            name,
            solid,
            colour: None,
        }],
        name,
        SystemTime::UNIX_EPOCH,
    )
    .ok()?;
    let again = read_step(&step).ok()?;
    Some((
        step,
        again.solids.into_iter().map(|read| read.solid).collect(),
    ))
}

pub fn bodies_transaction(
    document: &Document,
    bodies: &[ImportedBody],
    label: impl Into<String>,
) -> Transaction {
    let mut builder = document.transaction(label);
    let mut taken: Vec<String> = document
        .features()
        .map(|feature| feature.name.clone())
        .collect();
    for body in bodies {
        let name = unique_name(&body.name, &taken);
        taken.push(name.clone());
        builder.add_feature(name, FeatureKind::Import(body.import.clone()));
    }
    builder.finish()
}

fn unique_name(wanted: &str, taken: &[String]) -> String {
    let wanted = wanted.trim();
    let base = if wanted.is_empty() { "Import" } else { wanted };
    if !taken.iter().any(|name| name == base) {
        return base.to_owned();
    }
    (2..)
        .map(|number| format!("{base} {number}"))
        .find(|name| !taken.contains(name))
        .unwrap_or_else(|| base.to_owned())
}
