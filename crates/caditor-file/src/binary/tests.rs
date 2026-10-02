use std::time::{Duration, SystemTime, UNIX_EPOCH};

use caditor_document::{Document, Edit, Transaction};
use serde::{Deserialize, Serialize};

use super::{
    model::{decode, encode_over, history, load_version, reads_back, save_bytes},
    testing::{
        corrupt_chunk, decompressing_at_most, files_of_at_most, model_chunk_count, push_foreign,
        records_as_json, sharing_from, with_slices_of,
    },
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

const SAVE_SPACING: u64 = 3_600;

fn later(count: u32) -> SystemTime {
    at(1_000 + u64::from(count) * SAVE_SPACING)
}

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn saved_series(count: u32) -> (Vec<Document>, Vec<u8>) {
    saved_series_every(count, SAVE_SPACING)
}

fn saved_series_every(count: u32, spacing: u64) -> (Vec<Document>, Vec<u8>) {
    let first = with_width(10);
    let mut documents = vec![first.clone()];
    let mut bytes = save_bytes(&first, None, at(1_000), Some("First")).unwrap();
    for step in 1..count {
        let next = edited(documents.last().unwrap(), 10 + step);
        bytes = save_step(&next, &bytes, step, spacing);
        documents.push(next);
    }
    (documents, bytes)
}

fn save_step(document: &Document, previous: &[u8], step: u32, spacing: u64) -> Vec<u8> {
    let label = format!("Step {step}");
    let time = at(1_000 + u64::from(step) * spacing);
    save_bytes(document, Some(previous), time, Some(&label)).unwrap()
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
        assert_eq!(
            version.state.saved_at,
            at(1_000 + (18 - index) as u64 * SAVE_SPACING)
        );
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

    let resaved = save_bytes(&documents[0], Some(&head_record), later(12), None).unwrap();
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
    assert_eq!(
        load_version(&bytes, 0).map(|loaded| loaded.document),
        Err(crate::LoadError::VersionUnavailable)
    );
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
    assert_eq!(damaged(&container.pieces), 1);
}

#[test]
fn a_model_cut_between_chunks_is_reported_as_incomplete() {
    let mut document = with_width(10);
    let mut transaction = document.transaction("More");
    transaction.add_parameter("depth", transaction.parse("5 mm").unwrap());
    document.apply(transaction.finish()).unwrap();
    let bytes = save_bytes(&document, None, at(1_000), None).unwrap();
    assert_eq!(decode(&bytes).unwrap().issues, Vec::<String>::new());

    let container = parse(&bytes, &MODEL_MAGIC).unwrap();
    let Piece::Chunk(last) = container.pieces[container.pieces.len() - 2] else {
        panic!("the record is whole");
    };
    let cut = last.whole.as_ptr() as usize - bytes.as_ptr() as usize;
    let loaded = decode(&bytes[..cut]).unwrap();
    assert!(loaded.document.parameter_named("width").is_some());
    assert!(loaded.document.parameter_named("depth").is_none());
    assert_eq!(loaded.issues.len(), 1);
    assert!(loaded.issues[0].starts_with("The file ends early"));
}

fn check_listed_versions(bytes: &[u8], documents: &[Document]) -> History {
    let listed = history(bytes);
    for version in listed.versions.iter().filter(|version| version.available) {
        let step = match version.state.label.as_deref() {
            Some("First") => 0,
            Some(label) => label.strip_prefix("Step ").unwrap().parse().unwrap(),
            None => panic!("every save in the series has a label"),
        };
        assert_eq!(
            load_version(bytes, version.index).unwrap().document,
            documents[step],
            "{:?}",
            version.state.label
        );
    }
    listed
}

#[test]
fn a_damaged_version_info_is_never_paired_with_another_versions_data() {
    let (documents, bytes) = saved_series(6);
    let first_info = 3;

    let lost_info = corrupt_chunk(&bytes, &MODEL_MAGIC, first_info + 2);
    let listed = check_listed_versions(&lost_info, &documents);
    assert_eq!(listed.versions.len(), 4);
    assert!(listed.versions.iter().all(|version| version.available));
    let resaved = save_bytes(&documents[0], Some(&lost_info), later(6), None).unwrap();
    let after = check_listed_versions(&resaved, &documents);
    assert_eq!(after.versions.len(), 5);
    assert!(after.versions.iter().all(|version| version.available));

    let lost_data_and_next_info = corrupt_chunk(
        &corrupt_chunk(&bytes, &MODEL_MAGIC, first_info + 2),
        &MODEL_MAGIC,
        first_info + 1,
    );
    let listed = check_listed_versions(&lost_data_and_next_info, &documents);
    let labels: Vec<_> = listed
        .versions
        .iter()
        .map(|version| version.state.label.as_deref().unwrap())
        .collect();
    assert_eq!(labels, ["Step 2", "Step 1", "First"]);
    let resaved = save_bytes(
        &documents[0],
        Some(&lost_data_and_next_info),
        later(6),
        None,
    )
    .unwrap();
    check_listed_versions(&resaved, &documents);
}

#[test]
fn an_unchanged_record_is_written_back_as_stored_with_fields_this_version_does_not_know() {
    let document = with_width(10);
    let bytes = save_bytes(&document, None, at(1_000), None).unwrap();
    let mut records = records_as_json(&bytes);
    records[0] = records[0].replace(
        "\"name\":\"width\"",
        "\"name\":\"width\",\"note\":\"future\"",
    );
    let future = super::testing::current_model_from_json(&records);
    let loaded = decode(&future).unwrap();
    assert_eq!(loaded.issues, Vec::<String>::new());

    let mut grown = loaded.document.clone();
    let mut transaction = grown.transaction("More");
    transaction.add_parameter("depth", transaction.parse("5 mm").unwrap());
    grown.apply(transaction.finish()).unwrap();
    let resaved = save_bytes(&grown, Some(&future), at(2_000), None).unwrap();
    let kept = records_as_json(&resaved);
    assert!(kept[0].contains("\"note\":\"future\""), "{kept:?}");
    let stored = parse(&future, &MODEL_MAGIC).unwrap();
    let Piece::Chunk(first) = stored.pieces[1] else {
        panic!("the record is whole");
    };
    assert!(
        resaved
            .windows(first.whole.len())
            .any(|window| window == first.whole)
    );
    assert_eq!(decode(&resaved).unwrap().document, grown);

    let edited = edited(&grown, 20);
    let rewritten = save_bytes(&edited, Some(&resaved), at(3_000), None).unwrap();
    assert!(!records_as_json(&rewritten)[0].contains("note"));
    assert_eq!(decode(&rewritten).unwrap().document, edited);
    assert_eq!(load_version(&rewritten, 0).unwrap().document, grown);
}

#[test]
fn unknown_chunks_are_kept_unless_they_must_be_understood() {
    let document = with_width(10);
    let mut bytes = save_bytes(&document, None, at(1_000), None).unwrap();
    push_foreign(&mut bytes, 200, 0, b"optional");
    let loaded = decode(&bytes).unwrap();
    assert_eq!(loaded.issues, Vec::<String>::new());
    let resaved = save_bytes(&edited(&document, 20), Some(&bytes), at(2_000), None).unwrap();
    assert!(resaved.windows(8).any(|window| window == b"optional"));

    push_foreign(&mut bytes, 201, MUST_UNDERSTAND, b"required");
    let loaded = decode(&bytes).unwrap();
    assert_eq!(loaded.issues.len(), 1);
    assert!(loaded.issues[0].contains("a newer version of caditor needs"));
    assert_eq!(loaded.document, document);
    let resaved = save_bytes(&loaded.document, Some(&bytes), at(2_000), None).unwrap();
    assert!(resaved.windows(8).any(|window| window == b"optional"));
    assert!(!resaved.windows(8).any(|window| window == b"required"));
}

fn scrambled(length: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

fn raw_chunk_count(bytes: &[u8]) -> usize {
    bytes
        .windows(SYNC.len())
        .filter(|window| *window == SYNC)
        .count()
}

#[test]
fn content_longer_than_a_chunk_holds_continues_in_the_next_ones() {
    let noise = scrambled(1_000);
    let repetitive = b"width = 10 mm; ".repeat(100);
    let mut bytes = start_file(&MODEL_MAGIC, 1);
    with_slices_of(64, || {
        push_packed(&mut bytes, ChunkKind::Record, &noise).unwrap();
        push_packed_after(&mut bytes, ChunkKind::VersionData, &repetitive, &noise).unwrap();
        push_packed(&mut bytes, ChunkKind::Record, b"").unwrap();
    });

    assert_eq!(raw_chunk_count(&bytes), 16 + 24 + 1);
    let container = parse(&bytes, &MODEL_MAGIC).unwrap();
    let chunks: Vec<Chunk<'_>> = container.chunks().collect();
    assert_eq!(container.pieces.len(), 3);
    assert_eq!(chunks[0].content_length(), noise.len());
    assert_eq!(chunks[0].unpack(None).unwrap(), noise);
    assert_eq!(chunks[1].codec, Some(Codec::ZstdAfterNewer));
    assert_eq!(chunks[1].unpack(Some(&noise)).unwrap(), repetitive);
    assert_eq!(chunks[2].unpack(None).unwrap(), b"");

    let middle_part = start_of(&bytes, &chunks[0]) + 5 * (CHUNK_HEADER_LENGTH + 64);
    let last_part = start_of(&bytes, &chunks[1]) - 1;
    let mut damaged = bytes.clone();
    damaged[middle_part] ^= 0x55;
    let container = parse(&damaged, &MODEL_MAGIC).unwrap();
    assert_eq!(container.pieces[0], Piece::Damaged);
    let chunks: Vec<Chunk<'_>> = container.chunks().collect();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].unpack(Some(&noise)).unwrap(), repetitive);

    let cut = parse(&bytes[..last_part], &MODEL_MAGIC).unwrap();
    assert_eq!(cut.pieces, [Piece::Damaged]);
}

fn start_of(bytes: &[u8], chunk: &Chunk<'_>) -> usize {
    chunk.whole.as_ptr() as usize - bytes.as_ptr() as usize
}

#[test]
fn a_continued_chunk_followed_by_another_kind_is_damaged_and_the_other_kept() {
    let mut bytes = start_file(&MODEL_MAGIC, 1);
    with_slices_of(4, || {
        push_packed(&mut bytes, ChunkKind::Record, b"12345678").unwrap();
    });
    let second_part = raw_positions(&bytes)[1];
    bytes.truncate(second_part);
    push_packed(&mut bytes, ChunkKind::Head, b"head").unwrap();

    let container = parse(&bytes, &MODEL_MAGIC).unwrap();
    assert_eq!(container.pieces.len(), 2);
    assert_eq!(container.pieces[0], Piece::Damaged);
    let head = container.chunks().next().unwrap();
    assert_eq!(head.kind, Some(ChunkKind::Head));
    assert_eq!(head.unpack(None).unwrap(), b"head");
}

fn raw_positions(bytes: &[u8]) -> Vec<usize> {
    bytes
        .windows(SYNC.len())
        .enumerate()
        .filter(|(_, window)| *window == SYNC)
        .map(|(position, _)| position)
        .collect()
}

#[test]
fn a_model_larger_than_a_chunk_keeps_saving_its_history() {
    with_slices_of(40, || {
        let (documents, bytes) = saved_series(12);
        assert!(raw_chunk_count(&bytes) > model_chunk_count(&bytes));
        assert_eq!(decode(&bytes).unwrap().issues, Vec::<String>::new());
        assert_eq!(decode(&bytes).unwrap().document, *documents.last().unwrap());
        let listed = check_listed_versions(&bytes, &documents);
        assert_eq!(listed.versions.len(), 11);
        assert!(listed.versions.iter().all(|version| version.available));

        let container = parse(&bytes, &MODEL_MAGIC).unwrap();
        let data = container
            .pieces
            .iter()
            .enumerate()
            .filter(|(_, piece)| {
                matches!(piece, Piece::Chunk(chunk) if chunk.kind == Some(ChunkKind::VersionData))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let damaged = corrupt_chunk(&bytes, &MODEL_MAGIC, data[2]);
        assert_eq!(
            decode(&damaged).unwrap().document,
            *documents.last().unwrap()
        );
        let listed = check_listed_versions(&damaged, &documents);
        assert_eq!(listed.versions.len(), 10);
        assert!(listed.versions.iter().any(|version| version.available));
    });
}

fn longest_delta_run(bytes: &[u8]) -> usize {
    let container = parse(bytes, &MODEL_MAGIC).unwrap();
    let mut longest = 0;
    let mut run = 0;
    for chunk in container.chunks() {
        match (chunk.kind, chunk.codec) {
            (Some(ChunkKind::VersionData), Some(Codec::ZstdAfterNewer)) => {
                run += 1;
                longest = longest.max(run);
            }
            (Some(ChunkKind::VersionData), _) => run = 0,
            _ => {}
        }
    }
    longest
}

#[test]
fn older_versions_thin_out_and_every_kept_one_loads_back_exactly() {
    const TEN_MINUTES: u64 = 600;
    let (documents, bytes) = saved_series_every(60, TEN_MINUTES);
    let listed = check_listed_versions(&bytes, &documents);
    assert!(
        (10 + 8..=10 + 10).contains(&listed.versions.len()),
        "{}",
        listed.versions.len()
    );
    assert!(listed.versions.iter().all(|version| version.available));
    assert!(longest_delta_run(&bytes) < 8);
    assert_eq!(decode(&bytes).unwrap().document, documents[59]);
}

#[test]
fn thinning_across_weeks_keeps_whole_versions_often_enough() {
    const SIX_HOURS: u64 = 6 * 3_600;
    let (documents, bytes) = saved_series_every(240, SIX_HOURS);
    let listed = check_listed_versions(&bytes, &documents);
    assert!(listed.versions.iter().all(|version| version.available));
    assert!(listed.versions.len() < 60, "{}", listed.versions.len());
    assert!(longest_delta_run(&bytes) < 8);
    let oldest = listed.versions.last().unwrap();
    assert!(oldest.state.saved_at <= at(1_000 + 30 * 24 * 3_600));
}

#[test]
fn thinning_never_passes_off_a_damaged_version_as_another() {
    const HALF_HOUR: u64 = 1_800;
    let (mut documents, bytes) = saved_series_every(30, HALF_HOUR);
    let container = parse(&bytes, &MODEL_MAGIC).unwrap();
    let data: Vec<usize> = container
        .pieces
        .iter()
        .enumerate()
        .filter(|(_, piece)| {
            matches!(piece, Piece::Chunk(chunk) if chunk.kind == Some(ChunkKind::VersionData))
        })
        .map(|(index, _)| index)
        .collect();
    let mut bytes = corrupt_chunk(&bytes, &MODEL_MAGIC, data[12]);
    for step in 30..60 {
        let next = edited(documents.last().unwrap(), 10 + step);
        bytes = save_step(&next, &bytes, step, HALF_HOUR);
        documents.push(next);
        check_listed_versions(&bytes, &documents);
    }
    assert!(longest_delta_run(&bytes) < 8);
}

fn snapshot_size(bytes: &[u8]) -> usize {
    parse(bytes, &MODEL_MAGIC)
        .unwrap()
        .chunks()
        .filter(|chunk| chunk.kind == Some(ChunkKind::Record))
        .map(|chunk| chunk.content_length() + 1)
        .sum()
}

#[test]
fn listing_a_long_history_bounds_memory_per_version_not_in_total() {
    let (documents, bytes) = saved_series(20);
    let size = snapshot_size(&bytes);

    decompressing_at_most(3 * size, || {
        let listed = check_listed_versions(&bytes, &documents);
        assert_eq!(listed.versions.len(), 19);
        assert!(listed.versions.iter().all(|version| version.available));
    });
}

#[test]
fn listing_an_undamaged_history_decodes_none_of_its_versions() {
    let (documents, bytes) = saved_series(20);
    let size = snapshot_size(&bytes);

    let listed = decompressing_at_most(size, || history(&bytes));

    assert_eq!(listed.versions.len(), 19);
    assert!(listed.versions.iter().all(|version| version.available));
    assert_eq!(check_listed_versions(&bytes, &documents), listed);
}

#[test]
fn a_version_that_cannot_be_rewritten_keeps_the_newer_ones_it_is_stored_against() {
    const TEN_MINUTES: u64 = 600;
    let (_, sample) = saved_series(2);
    let size = snapshot_size(&sample);

    let (documents, starved) =
        decompressing_at_most(3 * size, || saved_series_every(60, TEN_MINUTES));
    let (_, unstarved) = saved_series_every(60, TEN_MINUTES);

    let listed = check_listed_versions(&starved, &documents);
    assert!(listed.versions.iter().all(|version| version.available));
    assert!(listed.versions.len() > history(&unstarved).versions.len());
    assert_eq!(decode(&starved).unwrap().document, documents[59]);
}

#[test]
fn a_history_too_large_for_the_file_drops_its_oldest_versions_and_says_how_many() {
    let (mut documents, bytes) = saved_series(20);
    let next = edited(documents.last().unwrap(), 50);
    documents.push(next.clone());
    let largest = bytes.len();

    let encoded = files_of_at_most(largest, || {
        encode_over(&next, Some(&bytes), later(20), Some("Step 20")).unwrap()
    });
    let listed = check_listed_versions(&encoded.bytes, &documents);
    let newest: Vec<_> = listed
        .versions
        .iter()
        .map(|version| version.state.label.clone().unwrap())
        .collect();

    assert!(encoded.bytes.len() <= largest / 4 * 3);
    assert!(encoded.dropped_for_size > 0);
    assert_eq!(listed.versions.len() + encoded.dropped_for_size, 20);
    assert!(listed.versions.iter().all(|version| version.available));
    assert_eq!(newest.first().map(String::as_str), Some("Step 19"));
    assert_eq!(decode(&encoded.bytes).unwrap().document, next);
}

#[test]
fn a_history_that_fits_drops_nothing_for_size() {
    let (documents, bytes) = saved_series(5);
    let next = edited(documents.last().unwrap(), 50);

    let encoded = encode_over(&next, Some(&bytes), later(5), Some("Step 5")).unwrap();

    assert_eq!(encoded.dropped_for_size, 0);
    assert_eq!(history(&encoded.bytes).versions.len(), 5);
}

#[test]
fn a_model_larger_than_a_file_can_hold_is_refused() {
    let document = with_width(10);

    let refused = files_of_at_most(64, || encode_over(&document, None, at(1_000), None));

    assert!(matches!(
        refused,
        Err(EncodeError::ModelTooLarge { largest: 64, .. })
    ));
}

fn bulky() -> Document {
    let mut document = with_width(10);
    let mut transaction = document.transaction("Bulk");
    let mut state = 0x9e37_79b9_u64;
    for index in 0..400 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let text = format!("{}.{} mm", state >> 44, (state >> 20) & 0xffff);
        transaction.add_parameter(format!("extra{index}"), transaction.parse(&text).unwrap());
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn padding_chunks(bytes: &[u8]) -> usize {
    parse(bytes, &MODEL_MAGIC)
        .unwrap()
        .chunks()
        .filter(|chunk| chunk.kind == Some(ChunkKind::Padding))
        .count()
}

#[test]
fn unchanged_versions_are_placed_where_the_previous_file_can_share_them() {
    const TEN_MINUTES: u64 = 600;
    sharing_from(1, || {
        let mut documents = vec![bulky()];
        let mut bytes = save_bytes(&documents[0], None, at(1_000), Some("First")).unwrap();
        let mut shared = 0;
        for step in 1..60 {
            let next = edited(documents.last().unwrap(), 10 + step);
            let label = format!("Step {step}");
            let time = at(1_000 + u64::from(step) * TEN_MINUTES);
            let encoded = encode_over(&next, Some(&bytes), time, Some(&label)).unwrap();
            for range in &encoded.shared {
                assert_eq!(range.at % 4096, 0);
                assert_eq!(range.from % 4096, 0);
                assert!(range.length % 4096 == 0 || range.from + range.length == bytes.len());
                assert_eq!(
                    encoded.bytes[range.at..range.at + range.length],
                    bytes[range.from..range.from + range.length]
                );
            }
            shared += encoded
                .shared
                .iter()
                .map(|range| range.length)
                .sum::<usize>();
            bytes = encoded.bytes;
            documents.push(next);
            assert_eq!(decode(&bytes).unwrap().issues, Vec::<String>::new());
        }
        assert_eq!(decode(&bytes).unwrap().document, documents[59]);
        let listed = check_listed_versions(&bytes, &documents);
        assert!(listed.versions.iter().all(|version| version.available));
        assert!(longest_delta_run(&bytes) < 8);
        assert!(shared > 40 * 4096, "{shared} bytes shared");
        assert!(
            padding_chunks(&bytes) < listed.versions.len(),
            "{} padding chunks for {} versions",
            padding_chunks(&bytes),
            listed.versions.len()
        );
    });
}

#[test]
fn small_histories_are_written_without_padding() {
    let (documents, bytes) = saved_series(20);
    assert_eq!(padding_chunks(&bytes), 0);
    let encoded = encode_over(documents.last().unwrap(), Some(&bytes), later(30), None).unwrap();
    assert!(encoded.shared.is_empty());
}

#[test]
fn padding_is_neither_a_version_nor_something_from_a_newer_version() {
    sharing_from(1, || {
        let (documents, bytes) = saved_series(12);
        assert!(padding_chunks(&bytes) > 0);
        let loaded = decode(&bytes).unwrap();
        assert_eq!(loaded.issues, Vec::<String>::new());
        assert_eq!(loaded.document, documents[11]);
        let listed = check_listed_versions(&bytes, &documents);
        assert_eq!(listed.versions.len(), 11);
        assert!(listed.versions.iter().all(|version| version.available));
    });
    let (documents, bytes) = sharing_from(1, || saved_series(12));
    let resaved = save_bytes(&documents[11], Some(&bytes), later(20), None).unwrap();
    assert_eq!(padding_chunks(&resaved), 0);
    assert_eq!(history(&resaved), history(&bytes));
}

#[test]
fn a_save_reads_back_only_when_every_record_matches_its_head() {
    let document = with_width(10);
    let encoded = encode_over(&document, None, at(1_000), None).unwrap();
    let previous = encoded.bytes.clone();
    let resaved = encode_over(&edited(&document, 20), Some(&previous), at(2_000), None).unwrap();

    let damaged_record = corrupt_chunk(&encoded.bytes, &MODEL_MAGIC, 1);
    let cut_short = &encoded.bytes[..encoded.bytes.len() / 2];

    assert!(reads_back(&encoded.bytes, &encoded.digest));
    assert!(reads_back(&resaved.bytes, &resaved.digest));
    assert!(!reads_back(&encoded.bytes, &resaved.digest));
    assert!(!reads_back(&damaged_record, &encoded.digest));
    assert!(!reads_back(cut_short, &encoded.digest));
    assert!(!reads_back(b"not a model", &encoded.digest));
}

#[test]
fn records_beside_a_damaged_one_are_still_written_back_as_stored() {
    let mut document = with_width(10);
    let mut transaction = document.transaction("Depth");
    transaction.add_parameter("depth", transaction.parse("5 mm").unwrap());
    document.apply(transaction.finish()).unwrap();
    let bytes = save_bytes(&document, None, at(1_000), None).unwrap();
    let damaged = corrupt_chunk(&bytes, &MODEL_MAGIC, 1);
    let loaded = decode(&damaged).unwrap().document;
    let stored = parse(&damaged, &MODEL_MAGIC).unwrap();
    let Piece::Chunk(depth) = stored.pieces[2] else {
        panic!("the depth record is whole");
    };

    let resaved = encode_over(&loaded, Some(&damaged), at(2_000), None).unwrap();

    assert!(resaved.previous_damaged);
    assert!(
        resaved
            .bytes
            .windows(depth.whole.len())
            .any(|window| window == depth.whole)
    );
    assert!(reads_back(&resaved.bytes, &resaved.digest));
    assert_eq!(decode(&resaved.bytes).unwrap().document, loaded);
    assert!(history(&resaved.bytes).versions.is_empty());
}
