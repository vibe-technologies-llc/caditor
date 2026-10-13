use std::time::{Duration, UNIX_EPOCH};

use caditor_zstd::Level;

use super::*;
use crate::{
    MeshFormat, TextOutlines,
    binary::{
        JOURNAL_MAGIC, MODEL_MAGIC, reseal, save_bytes,
        testing::{stored_copy, with_slices_of},
    },
    import::mesh_tests::{binary_stl, cube_obj, placed_cube_3mf, text_stl},
    journal::{JournalHead, Mirror, decode_journal, encode_journal, replay},
};

const WRITE_SEEDS: &str = "CADITOR_WRITE_FUZZ_SEEDS";
const SEED_SLICE: usize = 512;
const MESH_FORMATS: [MeshFormat; 3] = [MeshFormat::Stl, MeshFormat::Obj, MeshFormat::ThreeMf];
const INTER: &[u8] = include_bytes!("../../../caditor/assets/fonts/InterVariable.ttf");
const INTER_ITALIC: &[u8] =
    include_bytes!("../../../caditor/assets/fonts/InterVariable-Italic.ttf");

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
    let (document, _) = with_added_kinds(document);
    save_bytes(&document, Some(&bytes), at(3_000), Some("Datums")).unwrap()
}

fn seed_journal() -> Vec<u8> {
    let (base, entries) = seed_journal_entries();
    let entries: Vec<_> = entries.into_iter().map(journal::Logged::Entry).collect();
    encode_journal(&seed_journal_head(0), &base, &entries).unwrap()
}

pub(super) fn seed_journal_by_reference(folded: usize) -> Vec<u8> {
    let (base, entries) = seed_journal_entries();
    let mut mirror = Mirror::new(base.clone());
    let entries: Vec<_> = entries.into_iter().map(|entry| mirror.log(entry)).collect();
    assert!(entries.contains(&journal::Logged::UndoLast));
    assert!(entries.contains(&journal::Logged::RedoNext));
    encode_journal(&seed_journal_head(folded), &base, &entries).unwrap()
}

fn seed_journal_head(folded: usize) -> JournalHead<'static> {
    JournalHead {
        file: Some(Path::new("/models/plate.caditor")),
        on_disk: None,
        loaded_with_problems: false,
        folded,
    }
}

fn seed_journal_entries() -> (Document, Vec<JournalEntry>) {
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
    let entries = vec![
        JournalEntry::Apply(first),
        JournalEntry::Apply(second),
        JournalEntry::Undo(undone),
        JournalEntry::Redo(redone),
    ];
    (base, entries)
}

fn seed_mesh(format: MeshFormat, file: &[u8]) -> Vec<u8> {
    let tag = MESH_FORMATS
        .iter()
        .position(|known| *known == format)
        .unwrap();
    let mut seed = vec![u8::try_from(tag).unwrap()];
    seed.extend_from_slice(file);
    seed
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
        ("journal/by-reference.journal", seed_journal_by_reference(0)),
        ("journal/rebased.journal", seed_journal_by_reference(12)),
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
    write_seeds(seeds);
}

#[test]
#[ignore = "writes the mesh fuzz seeds when CADITOR_WRITE_FUZZ_SEEDS is set"]
fn write_mesh_fuzz_seeds() {
    if std::env::var_os(WRITE_SEEDS).is_none() {
        return;
    }
    let seeds = [
        (
            "mesh/cube-binary.stl",
            seed_mesh(MeshFormat::Stl, &binary_stl()),
        ),
        (
            "mesh/cube-text.stl",
            seed_mesh(MeshFormat::Stl, text_stl(0).as_bytes()),
        ),
        (
            "mesh/cube-open.stl",
            seed_mesh(MeshFormat::Stl, text_stl(1).as_bytes()),
        ),
        (
            "mesh/cube.obj",
            seed_mesh(MeshFormat::Obj, cube_obj().as_bytes()),
        ),
        (
            "mesh/cube.3mf",
            seed_mesh(MeshFormat::ThreeMf, &placed_cube_3mf()),
        ),
    ];
    write_seeds(seeds);
}

#[test]
#[ignore = "writes the picture fuzz seeds when CADITOR_WRITE_FUZZ_SEEDS is set"]
fn write_picture_fuzz_seeds() {
    if std::env::var_os(WRITE_SEEDS).is_none() {
        return;
    }
    let quad = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 3, |x, y| {
        image::Rgba([
            if x < 2 { 255 } else { 0 },
            if y < 2 { 255 } else { 0 },
            128,
            255,
        ])
    }));
    let encoded = |format: image::ImageFormat| {
        let mut bytes = std::io::Cursor::new(Vec::new());
        quad.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    };
    let seeds = [
        ("picture/quad.png", encoded(image::ImageFormat::Png)),
        ("picture/quad.jpg", {
            let opaque = image::DynamicImage::ImageRgb8(quad.to_rgb8());
            let mut bytes = std::io::Cursor::new(Vec::new());
            opaque
                .write_to(&mut bytes, image::ImageFormat::Jpeg)
                .unwrap();
            bytes.into_inner()
        }),
        ("picture/quad.webp", encoded(image::ImageFormat::WebP)),
        ("picture/quad.gif", encoded(image::ImageFormat::Gif)),
        ("picture/quad.bmp", encoded(image::ImageFormat::Bmp)),
        ("picture/quad.tif", encoded(image::ImageFormat::Tiff)),
        ("picture/quad.qoi", encoded(image::ImageFormat::Qoi)),
        ("picture/quad.tga", encoded(image::ImageFormat::Tga)),
        (
            "picture/quad.jxl",
            include_bytes!("../../pictures/quad.jxl").to_vec(),
        ),
    ];
    write_seeds(seeds);
}

#[test]
fn picture_seeds_decode() {
    for (path, bytes) in seeds_in("picture") {
        let pixels = crate::decode_picture(&bytes)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(pixels.width > 0 && pixels.height > 0, "{}", path.display());
    }
}

pub(super) fn seed_path(name: &str) -> Option<PathBuf> {
    let path = seeds_dir().join(name);
    (!corpus::inside_corpus(&path)).then_some(path)
}

fn write_seeds<const N: usize>(seeds: [(&str, Vec<u8>); N]) {
    for (name, bytes) in seeds {
        let path = seed_path(name)
            .unwrap_or_else(|| panic!("the seed {name} would land in the format corpus"));
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
fn model_and_journal_seeds_are_written_in_format_one() {
    let header = |magic: &[u8]| [magic, &1_u32.to_le_bytes()].concat();
    for (kind, magic) in [("model", MODEL_MAGIC), ("journal", JOURNAL_MAGIC)] {
        for (path, bytes) in seeds_in(kind) {
            assert_eq!(
                bytes.get(..12),
                Some(header(&magic).as_slice()),
                "{}",
                path.display()
            );
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
fn step_dxf_and_svg_seeds_import() {
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
    for (path, bytes) in seeds_in("svg") {
        let drawing = parse_svg(
            &bytes,
            TextOutlines::InFont {
                upright: INTER,
                italic: Some(INTER_ITALIC),
            },
        )
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(!drawing.curves.is_empty(), "{}", path.display());
    }
}

#[test]
fn mesh_seeds_import() {
    for (path, bytes) in seeds_in("mesh") {
        let (&tag, file) = bytes.split_first().unwrap();
        let format = MESH_FORMATS[usize::from(tag)];
        let import = parse_mesh(file, format, "seed", "seed")
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(!import.bodies.is_empty(), "{}", path.display());
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
