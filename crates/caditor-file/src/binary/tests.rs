use std::time::{Duration, SystemTime, UNIX_EPOCH};

use caditor_document::{Document, Edit, Transaction};
use serde::{Deserialize, Serialize};

use super::{
    model::{decode, history, load_version, save_bytes},
    testing::{corrupt_chunk, model_chunk_count},
    value::{from_bytes, to_bytes},
    *,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Shape {
    Empty,
    Wrapped(i64),
    Pair(u8, String),
    Named { x: f64, tags: Vec<Option<bool>> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Holder {
    shapes: Vec<Shape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    unit: (),
    bytes: Vec<u8>,
}

#[test]
fn every_serde_shape_round_trips_exactly() {
    let holder = Holder {
        shapes: vec![
            Shape::Empty,
            Shape::Wrapped(i64::MIN),
            Shape::Wrapped(-1),
            Shape::Wrapped(i64::MAX),
            Shape::Pair(255, "naïve “quotes”".to_owned()),
            Shape::Named {
                x: -0.0,
                tags: vec![Some(true), None, Some(false)],
            },
            Shape::Named {
                x: f64::MIN_POSITIVE / 3.0,
                tags: Vec::new(),
            },
        ],
        note: None,
        unit: (),
        bytes: vec![0, 1, 254, 255],
    };
    let bytes = to_bytes(&holder).unwrap();
    let back: Holder = from_bytes(&bytes).unwrap();
    assert_eq!(back, holder);
    let Shape::Named { x, .. } = back.shapes[5] else {
        panic!("expected a named shape");
    };
    assert!(x.is_sign_negative());

    let as_json: serde_json::Value = from_bytes(&bytes).unwrap();
    assert_eq!(as_json["shapes"][1]["Wrapped"], i64::MIN);
    assert_eq!(as_json["shapes"][0], "Empty");
}

#[test]
fn malformed_values_are_errors_not_panics() {
    let bytes = to_bytes(&vec![vec![1_u64, 2], vec![3]]).unwrap();
    for end in 0..bytes.len() {
        assert!(from_bytes::<Vec<Vec<u64>>>(&bytes[..end]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(from_bytes::<Vec<Vec<u64>>>(&trailing).is_err());
    assert!(
        from_bytes::<u64>(&[
            0x03, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f
        ])
        .is_err()
    );
    assert!(from_bytes::<String>(&[0x06, 0x02, 0xc3, 0x28]).is_err());
    assert!(from_bytes::<serde_json::Value>(&[0x3f]).is_err());
    let deep = vec![0x08; 10_000];
    assert!(from_bytes::<serde_json::Value>(&deep).is_err());
}

fn with_width(millimetres: u32) -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Width");
    let text = format!("{millimetres} mm");
    transaction.add_parameter("width", transaction.parse(&text).unwrap());
    document.apply(transaction.finish()).unwrap();
    document
}

fn edited(document: &Document, millimetres: u32) -> Document {
    let mut document = document.clone();
    let id = document.parameter_named("width").unwrap().id();
    let expression = document.parse(&format!("{millimetres} mm")).unwrap();
    document
        .apply(Transaction::single(
            "Edit width",
            Edit::SetParameterExpression { id, expression },
        ))
        .unwrap();
    document
}

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn saved_series(count: u32) -> (Vec<Document>, Vec<u8>) {
    let first = with_width(10);
    let mut documents = vec![first.clone()];
    let mut bytes = save_bytes(&first, None, at(1_000), Some("First")).unwrap();
    for step in 1..count {
        let next = edited(documents.last().unwrap(), 10 + step);
        let label = format!("Step {step}");
        bytes = save_bytes(
            &next,
            Some(&bytes),
            at(1_000 + u64::from(step)),
            Some(&label),
        )
        .unwrap();
        documents.push(next);
    }
    (documents, bytes)
}

#[test]
fn every_save_keeps_the_previous_state_as_a_version_that_loads_back_exactly() {
    let (documents, bytes) = saved_series(20);
    assert_eq!(decode(&bytes).unwrap().document, *documents.last().unwrap());

    let listed = history(&bytes);
    assert_eq!(
        listed
            .current
            .as_ref()
            .and_then(|state| state.label.as_deref()),
        Some("Step 19")
    );
    assert_eq!(listed.versions.len(), 19);
    for (index, version) in listed.versions.iter().enumerate() {
        assert!(version.available);
        assert_eq!(version.index, index);
        let expected = &documents[documents.len() - 2 - index];
        let loaded = load_version(&bytes, index).unwrap();
        assert!(loaded.issues.is_empty());
        assert_eq!(loaded.document, *expected);
        assert_eq!(version.state.saved_at, at(1_000 + (18 - index) as u64));
    }
    assert_eq!(listed.versions[18].state.label.as_deref(), Some("First"));
    assert!(matches!(
        load_version(&bytes, 19),
        Err(crate::LoadError::VersionUnavailable)
    ));

    let (_, small) = saved_series(2);
    let per_version = (bytes.len() - small.len()) / 18;
    assert!(
        per_version < small.len() / 2,
        "{per_version} bytes per version"
    );
}

#[test]
fn saving_an_unchanged_model_adds_no_version() {
    let (documents, bytes) = saved_series(3);
    let again = save_bytes(
        documents.last().unwrap(),
        Some(&bytes),
        at(9_999),
        Some("Again"),
    )
    .unwrap();
    assert_eq!(history(&again), history(&bytes));
}

#[test]
fn a_damaged_part_loses_only_what_depends_on_it() {
    let (documents, bytes) = saved_series(12);
    let head_record = corrupt_chunk(&bytes, &MODEL_MAGIC, 1);
    let loaded = decode(&head_record).unwrap();
    assert_eq!(
        loaded.issues,
        ["A damaged part of the file was skipped; anything it held was left out."]
    );
    assert!(loaded.document.parameters().is_empty());

    let listed = history(&head_record);
    let available: Vec<bool> = listed
        .versions
        .iter()
        .map(|version| version.available)
        .collect();
    let keyframe = available.iter().position(|available| *available).unwrap();
    assert!(keyframe > 0 && keyframe < 8, "{available:?}");
    assert!(available[keyframe..].iter().all(|available| *available));
    assert_eq!(
        load_version(&head_record, keyframe).unwrap().document,
        documents[documents.len() - 2 - keyframe]
    );

    let resaved = save_bytes(&documents[0], Some(&head_record), at(5_000), None).unwrap();
    let after = history(&resaved);
    assert!(after.versions.iter().all(|version| version.available));
    assert_eq!(after.versions.len(), listed.versions.len() - keyframe);

    let middle = model_chunk_count(&bytes) / 2;
    let version_damage = corrupt_chunk(&bytes, &MODEL_MAGIC, middle);
    assert_eq!(
        decode(&version_damage).unwrap().document,
        *documents.last().unwrap()
    );
}

#[test]
fn files_that_are_not_models_are_refused() {
    assert_eq!(decode(b""), Err(crate::LoadError::Empty));
    assert_eq!(decode(b"\n "), Err(crate::LoadError::Empty));
    assert_eq!(
        decode(b"{\"format\":\"caditor\",\"version\":7}\n"),
        Err(crate::LoadError::NotAModel)
    );
    assert_eq!(decode(&MODEL_MAGIC[..5]), Err(crate::LoadError::NotAModel));
    assert_eq!(decode(&JOURNAL_MAGIC), Err(crate::LoadError::NotAModel));
}

#[test]
fn the_packaged_mime_type_matches_models_by_magic_and_extension() {
    let mime = include_str!("../../../../packaging/caditor-mime.xml");
    let escaped: String = MODEL_MAGIC
        .iter()
        .map(|&byte| match byte {
            b'\r' => "\\r".to_owned(),
            b'\n' => "\\n".to_owned(),
            0x21..=0x7e => char::from(byte).to_string(),
            _ => format!("\\x{byte:02x}"),
        })
        .collect();
    assert!(mime.contains(&format!(
        "<match type=\"string\" offset=\"0\" value=\"{escaped}\"/>"
    )));
    assert!(mime.contains(&format!("<glob pattern=\"*.{}\"/>", crate::FILE_EXTENSION)));
}

#[test]
fn a_save_time_beyond_what_the_clock_holds_does_not_break_the_history() {
    let state = serde_json::json!({ "saved_at": u64::MAX, "digest": "" });
    let mut bytes = start_file(&MODEL_MAGIC, 1);
    push_packed(&mut bytes, ChunkKind::Head, &to_bytes(&state).unwrap()).unwrap();
    push_packed(
        &mut bytes,
        ChunkKind::VersionInfo,
        &to_bytes(&state).unwrap(),
    )
    .unwrap();
    push_packed(&mut bytes, ChunkKind::VersionData, b"old").unwrap();
    let listed = history(&bytes);
    assert_eq!(listed.current.unwrap().saved_at, UNIX_EPOCH);
    assert_eq!(listed.versions[0].state.saved_at, UNIX_EPOCH);
    assert!(!listed.versions[0].available);
}

#[test]
fn forged_chunk_headers_cannot_make_the_resync_scan_quadratic() {
    const FORGED: usize = 40_000;
    let mut bytes = start_file(&MODEL_MAGIC, 1);
    let body_length = FORGED * CHUNK_HEADER_LENGTH;
    for index in 0..FORGED {
        let remaining = body_length - (index + 1) * CHUNK_HEADER_LENGTH;
        bytes.extend_from_slice(&SYNC);
        bytes.extend_from_slice(&[ChunkKind::Record as u8, Codec::Stored as u8, 0, 0]);
        bytes.extend_from_slice(&u32::try_from(remaining).unwrap().to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(remaining).unwrap().to_le_bytes());
        bytes.extend_from_slice(&[0; 8]);
    }
    let started = std::time::Instant::now();
    let container = parse(&bytes, &MODEL_MAGIC).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(container.chunks().count(), 0);
    assert_eq!(container.damaged(), 1);
}
