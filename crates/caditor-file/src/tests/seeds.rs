use std::time::{Duration, UNIX_EPOCH};

use caditor_zstd::Level;

use super::*;
use crate::{
    binary::{
        JOURNAL_MAGIC, MODEL_MAGIC, reseal, save_bytes,
        testing::{stored_copy, with_slices_of},
    },
    journal::{decode_journal, encode_journal, replay},
};

const WRITE_SEEDS: &str = "CADITOR_WRITE_FUZZ_SEEDS";
const SEED_SLICE: usize = 512;

fn seeds_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds")
}

fn seeds_in(kind: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let dir = seeds_dir().join(kind);
    let mut seeds: Vec<(PathBuf, Vec<u8>)> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    seeds.sort();
    assert!(!seeds.is_empty(), "no seeds in {}", dir.display());
    seeds
}

fn seed_model() -> Vec<u8> {
    let (document, _, _) = datum_model();
    let at = |seconds| UNIX_EPOCH + Duration::from_secs(seconds);
    let mut bytes = save_bytes(&sample(), None, at(1_000), Some("Sample")).unwrap();
    let mut editor = Editor::new(sample());
    for (index, width) in ["41 mm", "42 mm", "43 mm"].into_iter().enumerate() {
        let edit = edit_width(editor.document(), width);
        editor.apply(edit).unwrap();
        let time = at(2_000 + index as u64);
        bytes = save_bytes(editor.document(), Some(&bytes), time, Some("Edit width")).unwrap();
    }
    save_bytes(&document, Some(&bytes), at(3_000), Some("Datums")).unwrap()
}

fn seed_journal() -> Vec<u8> {
    let base = sample();
    let first = edit_width(&base, "50 mm");
    let mut editor = Editor::new(base.clone());
    editor.apply(first.clone()).unwrap();
    let second = edit_width(editor.document(), "45 mm");
    editor.apply(second.clone()).unwrap();
    let undone = editor.next_undo().cloned().unwrap();
    editor.undo().unwrap();
    let redone = editor.next_redo().cloned().unwrap();
    editor.redo().unwrap();
    let entries = [
        JournalEntry::Apply(first),
        JournalEntry::Apply(second),
        JournalEntry::Undo(undone),
        JournalEntry::Redo(redone),
    ];
    encode_journal(
        Some(Path::new("/models/plate.caditor")),
        false,
        &base,
        &entries,
    )
    .unwrap()
}

fn seed_frame(prefix: &[u8], data: &[u8]) -> Vec<u8> {
    let frame = caditor_zstd::compress_after(data, prefix, Level::FAST).unwrap();
    let mut seed = vec![u8::try_from(prefix.len()).unwrap()];
    seed.extend_from_slice(prefix);
    seed.extend_from_slice(&frame);
    seed
}

#[test]
#[ignore = "writes the fuzz seeds when CADITOR_WRITE_FUZZ_SEEDS is set"]
fn write_fuzz_seeds() {
    if std::env::var_os(WRITE_SEEDS).is_none() {
        return;
    }
    let model = seed_model();
    let journal = seed_journal();
    let text = b"width = 40 mm; height = width / 2 + 0.1 mm; depth = 3 mm";
    let seeds = [
        ("model/datums.caditor", model.clone()),
        ("model/stored.caditor", stored_copy(&model, &MODEL_MAGIC)),
        ("journal/plate.journal", journal.clone()),
        (
            "model/sliced.caditor",
            with_slices_of(SEED_SLICE, seed_model),
        ),
        (
            "journal/stored.journal",
            stored_copy(&journal, &JOURNAL_MAGIC),
        ),
        (
            "journal/sliced.journal",
            with_slices_of(SEED_SLICE, seed_journal),
        ),
        ("zstd/plain", seed_frame(&[], text)),
        ("zstd/after-prefix", seed_frame(&text[..32], text)),
    ];
    for (name, bytes) in seeds {
        let path = seeds_dir().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

#[test]
fn model_seeds_load_cleanly_with_every_version() {
    for (path, bytes) in seeds_in("model") {
        let loaded = decode(&bytes).unwrap();
        assert_eq!(loaded.issues, Vec::<String>::new(), "{}", path.display());
        let history = binary::history(&bytes);
        assert!(history.versions.len() >= 3, "{}", path.display());
        for version in &history.versions {
            assert!(version.available, "{}", path.display());
            binary::load_version(&bytes, version.index).unwrap();
        }
    }
}

#[test]
fn journal_seeds_replay_every_entry() {
    for (path, bytes) in seeds_in("journal") {
        let contents = decode_journal(&bytes).unwrap();
        assert_eq!(contents.issues, Vec::<String>::new(), "{}", path.display());
        assert_eq!(contents.unreadable_entries, 0, "{}", path.display());
        let count = contents.entries.len();
        let replayed = replay(contents.base, contents.entries);
        assert!(!replayed.stopped_early, "{}", path.display());
        assert_eq!(replayed.entries.len(), count, "{}", path.display());
    }
}

#[test]
fn zstd_seeds_decompress_after_their_prefix() {
    for (path, bytes) in seeds_in("zstd") {
        let (&split, rest) = bytes.split_first().unwrap();
        let (prefix, frame) = rest.split_at(usize::from(split));
        caditor_zstd::decompress_after(frame, prefix, 1 << 20)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }
}

#[test]
fn step_and_dxf_seeds_import() {
    for (path, bytes) in seeds_in("step") {
        let text = String::from_utf8(bytes).unwrap();
        let import = parse_step(&text, "seed.step")
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(!import.bodies.is_empty(), "{}", path.display());
    }
    for (path, bytes) in seeds_in("dxf") {
        let drawing =
            parse_dxf(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(!drawing.curves.is_empty(), "{}", path.display());
    }
}

#[test]
fn expression_seeds_parse() {
    for (path, bytes) in seeds_in("expression") {
        let text = String::from_utf8(bytes).unwrap();
        let known =
            |name: &str| (name == "width").then(|| caditor_expression::ParameterId::from_raw(1));
        let parsed =
            Expression::parse(&text, &known).is_ok() || Expression::parse_stored(&text).is_ok();
        assert!(parsed, "{}", path.display());
    }
}

#[test]
fn resealing_restores_checksums_after_a_payload_changes() {
    let bytes = stored_copy(&seed_model(), &MODEL_MAGIC);
    let mut changed = bytes.clone();
    let last = changed.len() - 1;
    changed[last] ^= 0x01;
    assert_eq!(
        binary::history(&changed).versions.len() + 1,
        binary::history(&bytes).versions.len()
    );
    assert_eq!(reseal(&changed).len(), changed.len());
    assert_eq!(
        binary::history(&reseal(&changed)).versions.len(),
        binary::history(&bytes).versions.len()
    );
    assert_eq!(reseal(&bytes), bytes);
}
