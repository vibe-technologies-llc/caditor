use std::{
    collections::BTreeSet,
    io::{self, Write as _},
    path::Component,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use caditor_document::{
    AxisMate, AxisReference, AxisSide, AxisTurn, Blend, BlendKind, BodyAppearance, BodyOperation,
    BodyPlacement, ChamferForm, Coil, CoilLength, CoilTurning, Combine, CombineOperation,
    CopyOrientation, CurvePattern, CurveSpacing, Datum, DatumAxis, DatumFrame, DatumPlane,
    DatumPoint, DeleteFace, EdgeGroup, EdgeRule, Emboss, EmbossForm, Extrude, ExtrudeEnd,
    ExtrudeExtent, FaceColour, FaceTangent, FaceThickness, Feature, FinePitch, Finish, Helix, Hole,
    HoleBottom, HoleDepth, HoleFit, HoleShape, HoleSizing, HoleStandard, HoleStep, HoleStyle,
    Import, Loft, LoftSection, Mate, MatePair, MetricSize, Mirror, MirrorOutcome, ModelProperties,
    Move, MoveFace, OffsetFace, Outline, ParameterOwner, ParameterValues, Pattern, PatternKind,
    PatternOutcome, PieceKey, PieceLooks, Pieces, Pipe, PlaneRotation, PlaneThrough, PointBy,
    PointReference, PointsPattern, PrincipalAxis, PrincipalGeometry, PrincipalShowing,
    ProjectionSource, RegionChoice, Remove, Revolve, RevolveAxis, RevolveExtent, Rgb, Rib,
    RibDirection, RibEnds, RibSide, Scale, Shell, ShellSide, SketchFeature, SolidFeature,
    SolidStart, Split, SplitAlong, SplitCarry, SplitFace, Sweep, SweepPath, TappedThread,
    ThreadFamily, ThreadForm, ThreadHand, TurnCentre, Wall, point_instance,
};
use caditor_kernel::{
    BoundaryPiece, CoilPlacement, CoilShape, EdgeName, EdgeReference, FaceName, FaceOrigin,
    FaceReference, LoftSkin, PipeShape, RegionKey, RegionReference, Side, Solid, SweepOrientation,
    VertexName, WallSide,
};
use caditor_sketch::{ChainSize, Gear, Text, TextAlign};
use serde::{
    Deserializer,
    de::{DeserializeOwned, Error as _, Visitor, value::Error as ValueError},
};
use serde_json::Value;

use super::{
    configurations::configured_model, seeds::seed_journal_by_reference, selection_sets::sets_of, *,
};
use crate::{
    binary::{
        ChunkKind, Codec, EncodeExtras, JOURNAL_MAGIC, MODEL_MAGIC, encode_keeping, save_bytes,
        testing::{records_as_json, sharing_from, with_slices_of},
    },
    format::{
        ConstraintKindRecord, EditRecord, EntityKindRecord, FEATURE_FIELDS, FORMAT_VERSION,
        FeatureKindRecord, Record, transaction_record,
    },
    journal::{JOURNAL_VERSION, JournalHead, Logged, decode_journal, encode_journal, replay},
    parameters::PARAMETERS_VERSION,
    preview::{Preview, PreviewSlot},
};

const WRITE_CORPUS: &str = "CADITOR_WRITE_FORMAT_CORPUS";
const ENTITY_FLAGS: [&str; 2] = ["construction", "split_from"];
const SLICE: usize = 512;
const STEP_PART: &str = include_str!("../../../../fuzz/seeds/step/horn_torus.step");

const KEPT: [(&str, &str); 67] = [
    (
        "journal-1/by-reference.journal",
        "fe98725da8d9df46b19d7cede1474bcfb92bbda00d4de9c890e5bca5d0f2899f",
    ),
    (
        "journal-1/every-edit.journal",
        "d5f3ffe41fdaf1a24552095ee7a8335337340fd0f9da665a39f0930f793cba29",
    ),
    (
        "journal-1/gear-pairs.journal",
        "2b7a8a5b7f0b10acb092524dee27d3c2e9e451fd7b72f3114c7114c9281c4ddf",
    ),
    (
        "journal-1/gears.journal",
        "4d52613675f7574742c9822a2110b70ffdfd87aa9c9560bd06a5a487ec92e81b",
    ),
    (
        "journal-1/principal-showing.journal",
        "cee3e4d7c1186712abda011b6a752903520a968cb53e541b788ce7b5de7815c6",
    ),
    (
        "journal-1/piece-stand-ins.journal",
        "77955267d53c73adf6e6a462bf5410f21baa36bdd36e8bf762b2c2dedf423b23",
    ),
    (
        "journal-1/rebased.journal",
        "29586076a032208d8c3f3630c649dbca0afb09ef7023c4b25c24d93860c6c741",
    ),
    (
        "journal-1/reference-images.journal",
        "817180e04d6aa6752c2000efc746d716fc67646baeb6f5b6f772d6537c2fdeb5",
    ),
    (
        "journal-1/separate-pieces.journal",
        "9878be6506ba65ce5af74de2f72f137ab88a29a868596a2d9dbc8966f06d1afe",
    ),
    (
        "journal-1/sliced.journal",
        "d5f3ffe41fdaf1a24552095ee7a8335337340fd0f9da665a39f0930f793cba29",
    ),
    (
        "journal-1/sprockets.journal",
        "6b95b0cdbdee3f00cec6c9f4a1ed10f79a944ab8a85f5e2e3cf788f8c912790b",
    ),
    (
        "journal-1/texts.journal",
        "1fcf1e3d2569d9ec5af5ccb0d12941560cc5e403a7a713a6d31048d8edc07412",
    ),
    (
        "model-1/bodies.caditor",
        "8e65cdd34444ca11d831fa7ae40b30d81c3de5e3386608a59b33fbe6e7a7f9c9",
    ),
    (
        "model-1/coils.caditor",
        "6e89dee38fd62faccef05a808eaf08d11d309bcabfa5d5423fb6fc73c80a94be",
    ),
    (
        "model-1/datums.caditor",
        "eccdc03a0f9999a951b010f295fb893e67f3b7d97174d58770ddfb15cf06d54e",
    ),
    (
        "model-1/deleted-faces.caditor",
        "859ae366d6abd4248f22417a2073e3c387a458bae7bc41805e35dfea7b4849c6",
    ),
    (
        "model-1/document.caditor",
        "d6f719b47c939f742905bc3c7764a6b115222984bdb00759edacc6872588d630",
    ),
    (
        "model-1/drafted-emboss.caditor",
        "672de18330c7fd9c02ee37352fc2aeba685690d39ee89e115b25993332982cf8",
    ),
    (
        "model-1/drafts.caditor",
        "dcb865f43993e3a12a35e0ac8abecded8dc91ac2fc081c2da43674d7e0f0d10d",
    ),
    (
        "model-1/edge-rules.caditor",
        "fb14fcd45c697696cb3a0e1032a2de11cca32d99e0b26f5627983fc2a6728b57",
    ),
    (
        "model-1/embossed-faces.caditor",
        "cc4a8b402d64629be2a45e51e2287d29947cb6ddd376d7bc14d2c60d6a5a250e",
    ),
    (
        "model-1/embossed-regions.caditor",
        "e4bc1d51ee9d19af18cf83df2118bf892ae14e48980e96bfdb49c49a8f8a6ffc",
    ),
    (
        "model-1/extents.caditor",
        "079e3aa06ee8c687d2462c189baf9cb8a61920c7b54af61b2995a61f1fa31e28",
    ),
    (
        "model-1/faces.caditor",
        "37c964744927f008ee9f4c449e6ac47935ae79f7f64aac3138bbe00b2b0a7831",
    ),
    (
        "model-1/flat-spirals.caditor",
        "69de63fecfc61688b6fb61d699aa11262d34a33caa8e7eb81fd492f2cf2657ac",
    ),
    (
        "model-1/gear-pairs.caditor",
        "1aface737ff0d328bdfe7672a0e44bd76fa8e5ba3414d09469bb4abdf3d6fd50",
    ),
    (
        "model-1/gears.caditor",
        "159ba946083ad16f8b6dc70f5c8404ce4f6b3e56ba56f5e318a458eac896cd73",
    ),
    (
        "model-1/helices.caditor",
        "2d1bcb33f8ca6c8bf457cffddf2a4d0d434917beb2645bc3c7e0bedc04181ced",
    ),
    (
        "model-1/helix-points.caditor",
        "9c301b01781f8da5d50049fc88a620b28910422cebf8a37e960406df862db21d",
    ),
    (
        "model-1/helix-starts.caditor",
        "dbafced7b7f2936d0eee9b58afe16a80ba1e9e7097126d8541328040232c08ac",
    ),
    (
        "model-1/holes.caditor",
        "2cf9f186f10bbb89e64b84564f0152e30129f9ba6b6dadf846310dcc401d8e27",
    ),
    (
        "model-1/imports.caditor",
        "f172dff8d721a9e2b87cec9c788a954cebb79dd198f0335636da641731addfbb",
    ),
    (
        "model-1/loft-ends.caditor",
        "6b7356cdd0983574e28a1a6ba92e83a1e24f31bdee719e25a43526f2a4effb13",
    ),
    (
        "model-1/lofts.caditor",
        "f3e8d60297f4e779d8444e9971f45d96b71642ea56001e1b590fc74cf41378f4",
    ),
    (
        "model-1/measured.caditor",
        "0e800cdb471610bba2498c6d12c71a93f2e0f90f0a59e967c085aa0ea91efef5",
    ),
    (
        "model-1/mirrored-bodies.caditor",
        "e297a90850dbadb84ef3f4fb786a12855818f0098f32ac2cbe0621557c9b865f",
    ),
    (
        "model-1/mirrored-several-bodies.caditor",
        "5efd5527146e63cd00fb3ffa2ab359dd8413ff2750f75c6976226c64d98ca0f0",
    ),
    (
        "model-1/modelled-threads.caditor",
        "f5ea168f759dd3556843b0360ba687d67c7f36d9dd6ff0a93bd5f4eecaeb8957",
    ),
    (
        "model-1/moved-faces.caditor",
        "ebd12b3472881edb223022fff6e699cbc57718da12f20195d0db58b882779352",
    ),
    (
        "model-1/path-sweeps.caditor",
        "b28dd7ffcc7c11d879e21bd9325de195e284df80b8f3d914ea2f54ed0466ed61",
    ),
    (
        "model-1/patterned-bodies.caditor",
        "1ac2e198ecc425e136f04ebbfa27637796e969d7117ae5ecdee8dde276f9ba49",
    ),
    (
        "model-1/patterns.caditor",
        "ec1a7a437ffe10cba80a68813ae78ff8513ee236945d2ded0811d26847b7f147",
    ),
    (
        "model-1/piece-stand-ins.caditor",
        "fedf5a994b37008d31dfd38f044bcd9e154003555217b18ee5135f539f96627e",
    ),
    (
        "model-1/pieces.caditor",
        "f4745712d55b915884d644d687473b879d96e9ba8f066938a5358ba1a16023b2",
    ),
    (
        "model-1/pipes.caditor",
        "909988d8544c3f301cd5ee4eaf0004e495666d337a5cbb5c912082cf7edc1667",
    ),
    (
        "model-1/point-targets.caditor",
        "63353e88ac0ec528f701fd78e6d927bd62bb1f737a17edda9f6588682c4ed3ce",
    ),
    (
        "model-1/preview.caditor",
        "e0b9a6958693e108c3977da6155140b63c688a441a090b38c486ff96282455c5",
    ),
    (
        "model-1/principal-showing.caditor",
        "5bd3b9bee3053244975a54f00dbbdf393a7aaf60d9c47ec0ec90628b7fa32d42",
    ),
    (
        "model-1/primitives.caditor",
        "a951119ab531bb07364933525a7861e66093225c496ef672a87f2cc03e4917f3",
    ),
    (
        "model-1/projected-datums.caditor",
        "ba72f19aff762f3bea392f66c955ceda4d55df4a927d8d7740bb883dfc560f0f",
    ),
    (
        "model-1/reference-images.caditor",
        "d7e693bdc1a275426b2507a33a627d7108a9e0753c9976ae82f24ca4e816d0b4",
    ),
    (
        "model-1/replaced-faces.caditor",
        "10a7960a405fc6dacbed55c51033b2e282b90b8759a85e2a947ae7ac40479422",
    ),
    (
        "model-1/ribs.caditor",
        "34bfe6046a6df636261e77d609a641b7bc0d8442011fb2b3e12d9e82808a9823",
    ),
    (
        "model-1/separate-pieces.caditor",
        "624f5fbd3edf58847ef20022d29608d450a4ff1bd4136cca9f5c0939ddbdd915",
    ),
    (
        "model-1/shaped-ribs.caditor",
        "c1cf1d5f8afd15042bd0f375633428262c667ccdcac4eb22e967f5688d06a76a",
    ),
    (
        "model-1/shell-walls.caditor",
        "79b207952979689eceef40c8454d6bc7e34a80f6f984691f4da6860ab5369673",
    ),
    (
        "model-1/sketches.caditor",
        "4d56af94bbefdc7ca4d04154800c5ee3b009a13b6083696abbaf7c5c728ead87",
    ),
    (
        "model-1/sliced.caditor",
        "4d56af94bbefdc7ca4d04154800c5ee3b009a13b6083696abbaf7c5c728ead87",
    ),
    (
        "model-1/spline-knots.caditor",
        "1d17cc7945db58ee4b2354f106d72212c46f6fd0f2a3c92ab69d45240525f002",
    ),
    (
        "model-1/spline-pieces.caditor",
        "d6b13fe6a8e0771a6d16da0077d3c92e4831b9ce1c312fb0420539f3fa3d6b3b",
    ),
    (
        "model-1/sprockets.caditor",
        "cbb479281b56d2a6521f90da30c8a564f80eb954718852b0e063820279335e90",
    ),
    (
        "model-1/starts.caditor",
        "638005d3194997ab315150f533e17db8422551613a547f3e41a3c40e73770f26",
    ),
    (
        "model-1/sweep-shapes.caditor",
        "aaa34b1bf8c902ad163fa3c4e1677b3563305022cb10c2c6ca008726cf8d0e30",
    ),
    (
        "model-1/sweeps.caditor",
        "89008f408c1da375ec6d456157b46c58f8a088794926c1ae8fc334d4b4466519",
    ),
    (
        "model-1/text-layouts.caditor",
        "cb95a9b077de032d1980e155ccbe40997c1e66d52a945a42d3fa577bbf88f8a1",
    ),
    (
        "model-1/texts.caditor",
        "6fa57e26743f627531ea7a1ae012af9582c9f7428c15c06edc375d0086393657",
    ),
    (
        "parameters-1/exported.csv",
        "db019c464b4481e079664044b287b18aea095446c2642d0fdfe4356833339d01",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Model,
    Journal,
    Parameters,
}

impl Format {
    const ALL: [Self; 3] = [Self::Model, Self::Journal, Self::Parameters];

    fn prefix(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Journal => "journal",
            Self::Parameters => "parameters",
        }
    }

    fn current_version(self) -> u32 {
        match self {
            Self::Model => FORMAT_VERSION,
            Self::Journal => JOURNAL_VERSION,
            Self::Parameters => PARAMETERS_VERSION,
        }
    }

    fn current_dir(self) -> String {
        format!("{}-{}", self.prefix(), self.current_version())
    }

    fn of(name: &str) -> Self {
        let dir = name.split('/').next().unwrap();
        let (prefix, version) = dir.rsplit_once('-').unwrap();
        assert!(version.parse::<u32>().is_ok(), "{name} names no version");
        Self::ALL
            .into_iter()
            .find(|format| format.prefix() == prefix)
            .unwrap_or_else(|| panic!("{name} is in no format's folder"))
    }
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus")
}

fn lexically_normal(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    normal
}

pub(super) fn inside_corpus(path: &Path) -> bool {
    lexically_normal(path).starts_with(lexically_normal(&corpus_dir()))
}

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn digest_of(description: &str) -> String {
    blake3::hash(description.as_bytes()).to_hex().to_string()
}

fn describe(name: &str, bytes: &[u8]) -> String {
    match Format::of(name) {
        Format::Model => describe_model(bytes),
        Format::Journal => describe_journal(bytes),
        Format::Parameters => describe_parameters(bytes),
    }
}

fn describe_model(bytes: &[u8]) -> String {
    let loaded = decode(bytes).unwrap();
    let mut text = String::from("model\n");
    for issue in &loaded.issues {
        text.push_str(&format!("issue {issue}\n"));
    }
    for line in lines_of(&loaded.document) {
        text.push_str(&format!("{line}\n"));
    }
    for version in binary::history(bytes).versions {
        let seconds = version
            .state
            .saved_at
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        text.push_str(&format!(
            "version {} saved {seconds} label {:?} kept {} available {}\n",
            version.index, version.state.label, version.kept, version.available
        ));
        let restored = binary::load_version(bytes, version.index).unwrap();
        for issue in &restored.issues {
            text.push_str(&format!("issue {issue}\n"));
        }
        for line in lines_of(&restored.document) {
            text.push_str(&format!("{line}\n"));
        }
    }
    if let Some(preview) = binary::preview_of_prefix(bytes) {
        text.push_str(&format!(
            "preview {} bytes {}\n",
            preview.png().len(),
            blake3::hash(preview.png()).to_hex()
        ));
    }
    text
}

fn transaction_json(transaction: &Transaction) -> String {
    serde_json::to_string(&transaction_record(transaction)).unwrap()
}

fn describe_journal(bytes: &[u8]) -> String {
    let contents = decode_journal(bytes).unwrap();
    let mut text = format!(
        "journal\nfile {:?} on disk {:?} loaded with problems {} folded {} unreadable entries {}\n",
        contents
            .file
            .as_ref()
            .map(|file| file.to_string_lossy().into_owned()),
        contents.on_disk.as_ref().map(|digest| digest.0.clone()),
        contents.loaded_with_problems,
        contents.folded,
        contents.unreadable_entries,
    );
    for issue in &contents.issues {
        text.push_str(&format!("issue {issue}\n"));
    }
    text.push_str("base\n");
    for line in lines_of(&contents.base) {
        text.push_str(&format!("{line}\n"));
    }
    for entry in &contents.entries {
        let line = match entry {
            Logged::Entry(JournalEntry::Apply(transaction)) => {
                format!("apply {}", transaction_json(transaction))
            }
            Logged::Entry(JournalEntry::Undo(transaction)) => {
                format!("undo {}", transaction_json(transaction))
            }
            Logged::Entry(JournalEntry::Redo(transaction)) => {
                format!("redo {}", transaction_json(transaction))
            }
            Logged::UndoLast => "undo last".to_owned(),
            Logged::RedoNext => "redo next".to_owned(),
        };
        text.push_str(&format!("{line}\n"));
    }
    let replayed = replay(contents.base, contents.entries);
    text.push_str(&format!(
        "replayed {} stopped early {}\n",
        replayed.entries.len(),
        replayed.stopped_early
    ));
    for line in lines_of(replayed.editor.document()) {
        text.push_str(&format!("{line}\n"));
    }
    text
}

fn describe_parameters(bytes: &[u8]) -> String {
    let text = std::str::from_utf8(bytes).unwrap();
    let mut description = String::from("parameters\n");
    for row in parse_parameters(text).unwrap() {
        description.push_str(&format!(
            "{:?} {:?} {:?}\n",
            row.name, row.expression, row.note
        ));
    }
    description
}

fn kept_files() -> BTreeSet<String> {
    fn walk(dir: &Path, prefix: &str, found: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), &relative, found);
            } else {
                found.insert(relative);
            }
        }
    }
    let mut found = BTreeSet::new();
    walk(&corpus_dir(), "", &mut found);
    found
}

fn read_kept(name: &str) -> Vec<u8> {
    fs::read(corpus_dir().join(name)).unwrap()
}

#[test]
fn the_format_corpus_holds_exactly_the_files_recorded() {
    let recorded: BTreeSet<String> = KEPT.iter().map(|(name, _)| (*name).to_owned()).collect();

    assert_eq!(recorded.len(), KEPT.len(), "a file is recorded twice");
    assert_eq!(kept_files(), recorded);
}

#[test]
fn the_format_corpus_reads_as_it_did_when_written() {
    for (name, recorded) in KEPT {
        let description = describe(name, &read_kept(name));
        let digest = digest_of(&description);
        assert!(
            digest == recorded,
            "{name} no longer reads as it did when written: its description hashes to {digest}, \
             not {recorded}. A file of a shipped format must keep reading the same; never \
             rewrite a corpus file to make this pass. It now reads as:\n{description}"
        );
    }
}

#[test]
fn every_corpus_file_loads_without_a_problem() {
    for (name, _) in KEPT {
        let bytes = read_kept(name);
        match Format::of(name) {
            Format::Model => {
                assert_eq!(
                    decode(&bytes).unwrap().issues,
                    Vec::<String>::new(),
                    "{name}"
                );
                for version in binary::history(&bytes).versions {
                    assert!(version.available, "{name}");
                    let restored = binary::load_version(&bytes, version.index).unwrap();
                    assert_eq!(restored.issues, Vec::<String>::new(), "{name}");
                }
            }
            Format::Journal => {
                let contents = decode_journal(&bytes).unwrap();
                assert_eq!(contents.issues, Vec::<String>::new(), "{name}");
                assert_eq!(contents.unreadable_entries, 0, "{name}");
                let count = contents.entries.len();
                let replayed = replay(contents.base, contents.entries);
                assert!(!replayed.stopped_early, "{name}");
                assert_eq!(replayed.entries.len(), count, "{name}");
            }
            Format::Parameters => {
                let text = String::from_utf8(bytes).unwrap();
                assert!(!parse_parameters(&text).unwrap().is_empty(), "{name}");
            }
        }
    }
}

#[test]
fn the_format_corpus_holds_files_of_every_current_version() {
    for format in Format::ALL {
        let dir = format!("{}/", format.current_dir());
        assert!(
            KEPT.iter().any(|(name, _)| name.starts_with(&dir)),
            "the format corpus holds no {} file of version {}: run the corpus writer \
             (file-format.md) and record what it prints",
            format.prefix(),
            format.current_version()
        );
    }
}

#[test]
fn the_format_corpus_holds_every_kind_this_version_writes() {
    let mut covered = Covered::default();
    for (name, _) in KEPT {
        let bytes = read_kept(name);
        match Format::of(name) {
            Format::Model => covered.model(&bytes),
            Format::Journal => covered.journal(&bytes),
            Format::Parameters => {}
        }
    }

    require_every("record", &variants_of::<Record>(), &covered.records);
    require_every(
        "feature record",
        &variants_of::<FeatureKindRecord>(),
        &covered.features,
    );
    require_every(
        "sketch entity",
        &variants_of::<EntityKindRecord>(),
        &covered.entities,
    );
    require_every(
        "sketch entity flag",
        &ENTITY_FLAGS.map(str::to_owned).into(),
        &covered.entity_flags,
    );
    require_every(
        "sketch constraint",
        &variants_of::<ConstraintKindRecord>(),
        &covered.constraints,
    );
    require_every(
        "journaled edit",
        &variants_of::<EditRecord>(),
        &covered.edits,
    );
    let chunks: BTreeSet<String> = ChunkKind::ALL
        .iter()
        .map(|kind| format!("{kind:?}"))
        .collect();
    require_every("chunk", &chunks, &covered.chunks);
    let codecs: BTreeSet<String> = Codec::ALL
        .iter()
        .map(|codec| format!("{codec:?}"))
        .collect();
    require_every("codec", &codecs, &covered.codecs);
    let containers: BTreeSet<String> = [Format::Model, Format::Journal]
        .map(|format| format.prefix().to_owned())
        .into();
    require_every("sliced content", &containers, &covered.sliced);
}

#[test]
#[ignore = "writes the format corpus files that do not exist yet when CADITOR_WRITE_FORMAT_CORPUS \
            is set"]
fn write_format_corpus() {
    if std::env::var_os(WRITE_CORPUS).is_none() {
        return;
    }
    for (name, bytes) in corpus_of_this_version() {
        let path = corpus_dir().join(&name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let created = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        match created {
            Ok(mut file) => {
                file.write_all(&bytes).unwrap();
                let digest = digest_of(&describe(&name, &bytes));
                println!("(\"{name}\", \"{digest}\"),");
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("{}: {error}", path.display()),
        }
    }
}

#[test]
fn the_seed_writer_cannot_reach_the_format_corpus() {
    let corpus_file = format!(
        "../../crates/caditor-file/corpus/{}/sample.caditor",
        Format::Model.current_dir()
    );

    assert!(inside_corpus(&corpus_dir().join("model-1/sample.caditor")));
    assert!(super::seeds::seed_path("model/datums.caditor").is_some());
    assert!(super::seeds::seed_path(&corpus_file).is_none());
}

struct VariantNames<'a>(&'a mut &'static [&'static str]);

impl<'de> Deserializer<'de> for VariantNames<'_> {
    type Error = ValueError;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, ValueError> {
        Err(ValueError::custom("only an enum names its variants"))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        variants: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, ValueError> {
        *self.0 = variants;
        Err(ValueError::custom(
            "the variants are all that was asked for",
        ))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct identifier
        ignored_any
    }
}

fn variants_of<T: DeserializeOwned>() -> BTreeSet<String> {
    let mut variants: &'static [&'static str] = &[];
    assert!(T::deserialize(VariantNames(&mut variants)).is_err());
    assert!(!variants.is_empty());
    variants
        .iter()
        .map(|variant| (*variant).to_owned())
        .collect()
}

#[derive(Debug, Default)]
struct Covered {
    records: BTreeSet<String>,
    features: BTreeSet<String>,
    entities: BTreeSet<String>,
    entity_flags: BTreeSet<String>,
    constraints: BTreeSet<String>,
    edits: BTreeSet<String>,
    chunks: BTreeSet<String>,
    codecs: BTreeSet<String>,
    sliced: BTreeSet<String>,
}

fn keys_without(value: &Value, fields: &[&str]) -> Vec<String> {
    value
        .as_object()
        .map(|object| {
            object
                .keys()
                .filter(|key| !fields.contains(&key.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn require_every(what: &str, every: &BTreeSet<String>, held: &BTreeSet<String>) {
    let missing: Vec<&String> = every.difference(held).collect();
    assert!(
        missing.is_empty(),
        "no file of the format corpus holds the {what} kinds {missing:?}: add a corpus file \
         holding them (file-format.md)"
    );
}

impl Covered {
    fn container(&mut self, bytes: &[u8], format: Format, magic: &binary::Magic) {
        for chunk in binary::parse(bytes, magic).unwrap().chunks() {
            self.chunks.insert(format!("{:?}", chunk.kind.unwrap()));
            self.codecs.insert(format!("{:?}", chunk.codec.unwrap()));
            if chunk.parts() > 1 {
                self.sliced.insert(format.prefix().to_owned());
            }
        }
    }

    fn model(&mut self, bytes: &[u8]) {
        self.container(bytes, Format::Model, &MODEL_MAGIC);
        for line in records_as_json(bytes) {
            let record: Value = serde_json::from_str(&line).unwrap();
            self.records.extend(keys_without(&record, &[]));
            if let Some(feature) = record.get("feature") {
                let fields: Vec<&str> = ["id", "name"].into_iter().chain(FEATURE_FIELDS).collect();
                for kind in keys_without(feature, &fields) {
                    self.feature(&kind, &feature[&kind]);
                }
            }
        }
    }

    fn feature(&mut self, kind: &str, value: &Value) {
        self.features.insert(kind.to_owned());
        let items = |field: &str| {
            value
                .get(field)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        for entity in items("entities") {
            let fields: Vec<&str> = ["id"].into_iter().chain(ENTITY_FLAGS).collect();
            self.entities.extend(keys_without(&entity, &fields));
            self.entity_flags.extend(
                ENTITY_FLAGS
                    .iter()
                    .filter(|flag| entity.get(**flag).is_some())
                    .map(|flag| (*flag).to_owned()),
            );
        }
        for constraint in items("constraints") {
            self.constraints
                .extend(keys_without(&constraint, &["id", "inactive", "label"]));
        }
        if let Some(inner) = value.get("feature") {
            for kind in keys_without(inner, &[]) {
                self.feature(&kind, &inner[&kind]);
            }
        }
    }

    fn journal(&mut self, bytes: &[u8]) {
        self.container(bytes, Format::Journal, &JOURNAL_MAGIC);
        for chunk in binary::parse(bytes, &JOURNAL_MAGIC).unwrap().chunks() {
            if !matches!(
                chunk.kind,
                Some(ChunkKind::Apply | ChunkKind::Undo | ChunkKind::Redo)
            ) {
                continue;
            }
            let content = chunk.unpack(None).unwrap();
            let transaction: Value = binary::value::from_bytes(&content).unwrap();
            for edit in transaction["edits"].as_array().unwrap() {
                self.edits.extend(keys_without(edit, &[]));
            }
        }
    }
}

fn corpus_of_this_version() -> Vec<(String, Vec<u8>)> {
    let model = |name: &str| format!("{}/{name}.caditor", Format::Model.current_dir());
    let journal = |name: &str| format!("{}/{name}.journal", Format::Journal.current_dir());
    let parameters = |name: &str| format!("{}/{name}.csv", Format::Parameters.current_dir());
    let saved = |document: &Document, label: &str| {
        save_bytes(document, None, at(1_700_000_000), Some(label)).unwrap()
    };
    let mut files = vec![
        (model("sketches"), saved(&sketches(), "Sketches")),
        (model("pieces"), saved(&pieces(), "Pieces")),
        (
            model("spline-pieces"),
            saved(&spline_pieces(), "Spline pieces"),
        ),
        (
            model("spline-knots"),
            saved(&spline_knots(), "Spline knots"),
        ),
        (model("texts"), saved(&texts().0, "Texts")),
        (model("gears"), saved(&gears().0, "Gears")),
        (model("gear-pairs"), saved(&gear_pairs().0, "Gear pairs")),
        (model("sprockets"), saved(&sprockets().0, "Sprockets")),
        (
            model("text-layouts"),
            saved(&text_layouts(), "Text layouts"),
        ),
        (
            model("projected-datums"),
            saved(&projected_datums(), "Projected datums"),
        ),
        (model("extents"), saved(&extents_model().0, "Extents")),
        (model("starts"), saved(&starts_model().0, "Starts")),
        (model("sweeps"), saved(&sweeps(), "Sweeps")),
        (model("path-sweeps"), saved(&path_sweeps(), "Path sweeps")),
        (
            model("sweep-shapes"),
            saved(&sweep_shapes(), "Sweep shapes"),
        ),
        (model("pipes"), saved(&pipes(), "Pipes")),
        (model("ribs"), saved(&ribs(), "Ribs")),
        (model("shaped-ribs"), saved(&shaped_ribs(), "Shaped ribs")),
        (model("coils"), saved(&coils(), "Coils")),
        (model("helices"), saved(&helices(), "Helices")),
        (
            model("helix-points"),
            saved(&helix_points(), "Helix points"),
        ),
        (
            model("helix-starts"),
            saved(&helix_starts(), "Helix starts"),
        ),
        (
            model("flat-spirals"),
            saved(&flat_spirals(), "Flat spirals"),
        ),
        (model("lofts"), saved(&lofts(), "Lofts")),
        (model("loft-ends"), saved(&loft_ends(), "Loft ends")),
        (
            model("separate-pieces"),
            saved(&separate_pieces(), "Separate pieces"),
        ),
        (
            model("piece-stand-ins"),
            saved(&piece_stand_ins(), "Piece stand-ins"),
        ),
        (
            model("point-targets"),
            saved(&point_targets(), "Point targets"),
        ),
        (model("holes"), saved(&holes(), "Holes")),
        (
            model("modelled-threads"),
            saved(&modelled_threads(), "Modelled threads"),
        ),
        (model("faces"), saved(&faces(), "Faces")),
        (model("edge-rules"), saved(&edge_rules(), "Edge rules")),
        (
            model("deleted-faces"),
            saved(&deleted_faces(), "Deleted faces"),
        ),
        (model("moved-faces"), saved(&moved_faces(), "Moved faces")),
        (model("drafts"), saved(&drafts(), "Drafts")),
        (
            model("replaced-faces"),
            saved(&replaced_faces(), "Replaced faces"),
        ),
        (
            model("embossed-faces"),
            saved(&embossed_faces(), "Embossed faces"),
        ),
        (
            model("embossed-regions"),
            saved(&embossed_regions(), "Embossed regions"),
        ),
        (
            model("drafted-emboss"),
            saved(&drafted_emboss(), "Drafted emboss"),
        ),
        (model("shell-walls"), saved(&shell_walls(), "Shell walls")),
        (model("datums"), saved(&datums(), "Datums")),
        (model("bodies"), saved(&bodies(), "Bodies")),
        (
            model("mirrored-bodies"),
            saved(&mirrored_bodies(), "Mirrored bodies"),
        ),
        (
            model("mirrored-several-bodies"),
            saved(&mirrored_several_bodies(), "Mirrored several bodies"),
        ),
        (model("patterns"), saved(&patterns(), "Patterns")),
        (
            model("patterned-bodies"),
            saved(&patterned_bodies(), "Patterned bodies"),
        ),
        (
            model("primitives"),
            saved(&primitive_model().0, "Primitives"),
        ),
        (model("measured"), saved(&measured_model().0, "Measure")),
        (model("imports"), saved(&imports(), "Imports")),
        (
            model("reference-images"),
            saved(&reference_images(), "Reference images"),
        ),
        (
            model("preview"),
            saved_with_preview(&primitive_model().0, "Preview"),
        ),
        (model("document"), document_with_history()),
        (
            model("sliced"),
            with_slices_of(SLICE, || saved(&sketches(), "Sliced")),
        ),
        (journal("every-edit"), every_edit_journal()),
        (journal("by-reference"), seed_journal_by_reference(0)),
        (journal("separate-pieces"), separate_pieces_journal()),
        (journal("piece-stand-ins"), piece_stand_ins_journal()),
        (journal("rebased"), seed_journal_by_reference(12)),
        (journal("sliced"), with_slices_of(SLICE, every_edit_journal)),
        (journal("reference-images"), reference_image_journal()),
        (journal("texts"), texts_journal()),
        (journal("gears"), gears_journal()),
        (journal("gear-pairs"), gear_pairs_journal()),
        (journal("sprockets"), sprockets_journal()),
        (
            model("principal-showing"),
            saved(&principal_showing().0, "Principal showing"),
        ),
        (journal("principal-showing"), principal_showing_journal()),
        (parameters("exported"), exported_parameters().into_bytes()),
    ];
    files.sort();
    files
}

fn edge(digest: u128) -> EdgeReference {
    EdgeReference::new(
        EdgeName::from_digest(digest),
        [FaceName::from_digest(1), FaceName::from_digest(2)],
        [VertexName::from_digest(3), VertexName::from_digest(4)],
    )
}

fn end_cap(digest: u128) -> FaceReference {
    FaceReference::new(
        FaceName::from_digest(digest),
        Some(FaceOrigin::EndCap { feature: 1 }),
        [FaceName::from_digest(2)],
    )
}

fn pieces() -> Document {
    let mut drawn = Sketch::new(Plane::XY);
    let line = drawn.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let arc = drawn.add_arc(Point2::new(20.0, 0.0), Point2::new(40.0, 0.0), Point2::ZERO);
    for x in [10.0, 15.0, 25.0, 30.0] {
        drawn.add_line(Point2::new(x, -5.0), Point2::new(x, 25.0));
    }
    drawn.trim(line, Point2::new(20.0, 0.0)).unwrap();
    drawn.trim(arc, Point2::new(20.0, 20.0)).unwrap();
    let pieces: Vec<EntityId> = drawn
        .entities()
        .map(|(id, _)| id)
        .filter(|id| drawn.split_from(*id).is_some())
        .collect();
    for piece in pieces {
        drawn.break_curve(piece).unwrap();
    }
    let mut document = Document::default();
    let mut transaction = document.transaction("Pieces");
    transaction.add_feature("Trimmed", FeatureKind::from(drawn));
    document.apply(transaction.finish()).unwrap();
    document
}

fn spline_pieces() -> Document {
    let mut drawn = Sketch::new(Plane::XY);
    let wave = [
        Point2::new(100.0, 100.0),
        Point2::new(110.0, 115.0),
        Point2::new(120.0, 90.0),
        Point2::new(130.0, 112.0),
        Point2::new(140.0, 86.0),
        Point2::new(150.0, 108.0),
        Point2::new(160.0, 100.0),
    ];
    let spline = drawn.add_spline(&wave);
    for x in [113.0, 147.0] {
        drawn.add_line(Point2::new(x, 60.0), Point2::new(x, 140.0));
    }
    drawn.trim(spline, Point2::new(101.0, 101.0)).unwrap();
    drawn.trim(spline, Point2::new(159.0, 101.0)).unwrap();
    let mut document = Document::default();
    let mut transaction = document.transaction("Spline pieces");
    transaction.add_feature("Trimmed spline", FeatureKind::from(drawn));
    document.apply(transaction.finish()).unwrap();
    document
}

fn spline_knots() -> Document {
    let mut drawn = Sketch::new(Plane::XY);
    let wave = [
        Point2::new(100.0, 100.0),
        Point2::new(110.0, 115.0),
        Point2::new(120.0, 90.0),
        Point2::new(130.0, 112.0),
        Point2::new(140.0, 86.0),
        Point2::new(150.0, 108.0),
        Point2::new(160.0, 100.0),
    ];
    let spline = drawn.add_spline(&wave);
    for at in [115.0, 135.0] {
        drawn
            .insert_control_point(spline, Some(Point2::new(at, 100.0)))
            .unwrap();
    }
    let mut document = Document::default();
    let mut transaction = document.transaction("Spline knots");
    transaction.add_feature("Refined spline", FeatureKind::from(drawn));
    document.apply(transaction.finish()).unwrap();
    document
}

fn blocks_sketch() -> (Sketch, [EntityId; 3]) {
    let mut drawn = Sketch::new(Plane::XY);
    let mut first_lines = [EntityId::from_raw(0); 3];
    for (index, x) in [0.0, 20.0, 40.0].into_iter().enumerate() {
        let corners = [
            Point2::new(x, 0.0),
            Point2::new(x + 10.0, 0.0),
            Point2::new(x + 10.0, 10.0),
            Point2::new(x, 10.0),
        ];
        for side in 0..4 {
            let line = drawn.add_line(corners[side], corners[(side + 1) % 4]);
            if side == 0
                && let Some(first) = first_lines.get_mut(index)
            {
                *first = line;
            }
        }
    }
    (drawn, first_lines)
}

fn blocks(sketch: FeatureId) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(Expression::parse_stored("5 mm").unwrap(), false),
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

fn piece_key(line: EntityId) -> PieceKey {
    PieceKey::of_entity(line).unwrap()
}

pub(super) fn separate_pieces() -> Document {
    let (drawn, lines) = blocks_sketch();
    let mut document = Document::default();
    let mut transaction = document.transaction("Separate pieces");
    let sketch = transaction.add_feature("Blocks sketch", FeatureKind::from(drawn));
    let mut extruded = Feature::new(FeatureId::from_raw(0), "Blocks".to_owned(), blocks(sketch));
    extruded.pieces = Pieces::BodyEach;
    extruded.piece_looks = PieceLooks::default()
        .with_appearance(
            piece_key(lines[0]),
            BodyAppearance {
                colour: Some(Rgb {
                    red: 200,
                    green: 40,
                    blue: 40,
                }),
                name: Some("Left block".to_owned()),
                ..BodyAppearance::default()
            },
        )
        .with_hidden(piece_key(lines[2]));
    let extruded = transaction.add_copied_feature(extruded);
    transaction.add_feature(
        "Rounded middle",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: extruded.piece(piece_key(lines[1])).unwrap(),
            groups: Vec::new(),
            edges: vec![edge(9)],
            size: Expression::parse_stored("1 mm").unwrap(),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn separated_blocks() -> (Document, FeatureId, [EntityId; 3]) {
    let (drawn, lines) = blocks_sketch();
    let mut document = Document::default();
    let mut transaction = document.transaction("Blocks");
    let sketch = transaction.add_feature("Blocks sketch", FeatureKind::from(drawn));
    let mut extruded = Feature::new(FeatureId::from_raw(0), "Blocks".to_owned(), blocks(sketch));
    extruded.pieces = Pieces::BodyEach;
    transaction.add_copied_feature(extruded);
    document.apply(transaction.finish()).unwrap();
    (document, sketch, lines)
}

pub(super) fn piece_stand_ins() -> Document {
    let (mut document, sketch, lines) = separated_blocks();
    let mut removal = document.transaction("Delete a side");
    removal.remove_sketch_items(sketch, [lines[1]], []);
    document.apply(removal.finish()).unwrap();
    let mut redraw = document.transaction("Draw the side again");
    let start = redraw.add_sketch_entity(sketch, Entity::Point(Point2::new(20.0, 0.0)));
    let end = redraw.add_sketch_entity(sketch, Entity::Point(Point2::new(30.0, 0.0)));
    redraw.add_sketch_entity(sketch, Entity::Line { start, end });
    document.apply(redraw.finish()).unwrap();
    document
}

fn texts() -> (Document, FeatureId, EntityId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Texts");
    let size = transaction.add_parameter("size", transaction.parse("6 mm").unwrap());
    let sketch = transaction.add_feature("Lettering", FeatureKind::from(Sketch::new(Plane::XY)));
    let first = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(2.0, 3.0)));
    let second = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(2.0, 20.0)));
    transaction.add_sketch_constraint(
        sketch,
        Constraint::Fix {
            point: first,
            at: Point2::new(2.0, 3.0),
        },
    );
    let plain = transaction.add_sketch_text(
        sketch,
        Text {
            anchor: first,
            content: "Part 7".to_owned(),
            height: Expression::Measure(5.0, Unit::Millimetre),
            angle: Expression::Measure(0.0, Unit::Degree),
            bold: false,
            italic: false,
            align: TextAlign::Left,
        },
    );
    transaction.add_sketch_text(
        sketch,
        Text {
            anchor: second,
            content: "Größe ½".to_owned(),
            height: Expression::Parameter(size),
            angle: Expression::Measure(15.0, Unit::Degree),
            bold: true,
            italic: true,
            align: TextAlign::Left,
        },
    );
    transaction.add_feature(
        "Raised letters",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Measure(1.5, Unit::Millimetre), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, sketch, plain)
}

fn text_layouts() -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Text layouts");
    let sketch = transaction.add_feature("Plaque", FeatureKind::from(Sketch::new(Plane::XY)));
    let middle = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(30.0, 20.0)));
    let end = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(60.0, 4.0)));
    for (anchor, content, align) in [
        (middle, "Caditor\nPlaque 7", TextAlign::Centre),
        (end, "Rev A\n\nRight", TextAlign::Right),
    ] {
        transaction.add_sketch_text(
            sketch,
            Text {
                anchor,
                content: content.to_owned(),
                height: Expression::Measure(4.0, Unit::Millimetre),
                angle: Expression::Measure(0.0, Unit::Degree),
                bold: false,
                italic: false,
                align,
            },
        );
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn gear_of(centre: EntityId, teeth: Expression) -> Gear {
    Gear {
        centre,
        module: Expression::Measure(2.0, Unit::Millimetre),
        teeth,
        pressure_angle: Expression::Measure(20.0, Unit::Degree),
        profile_shift: Expression::Number(0.0),
        root_fillet: Expression::Measure(0.5, Unit::Millimetre),
        bore: Expression::Measure(8.0, Unit::Millimetre),
        turn: Expression::Measure(0.0, Unit::Degree),
        meshing: None,
        chain: None,
    }
}

fn partner_of(centre: EntityId, driver: EntityId, teeth: Expression) -> Gear {
    Gear::meshed(
        centre,
        driver,
        teeth,
        Expression::Measure(6.0, Unit::Millimetre),
        Expression::Measure(30.0, Unit::Degree),
    )
}

fn gear_pairs() -> (Document, FeatureId, EntityId, EntityId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Gear pairs");
    let count = transaction.add_parameter("partner_teeth", transaction.parse("30").unwrap());
    let sketch = transaction.add_feature("Pair", FeatureKind::from(Sketch::new(Plane::XY)));
    let first = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(0.0, 0.0)));
    let second = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(43.3, 25.0)));
    transaction.add_sketch_constraint(sketch, Constraint::Coincident(first, EntityId::ORIGIN));
    let driver = transaction.add_sketch_gear(sketch, gear_of(first, Expression::Number(20.0)));
    let partner = transaction.add_sketch_gear(
        sketch,
        partner_of(second, driver, Expression::Parameter(count)),
    );
    document.apply(transaction.finish()).unwrap();
    (document, sketch, driver, partner)
}

fn gear_pairs_journal() -> Vec<u8> {
    let (base, sketch, driver, partner) = gear_pairs();
    let mut session = Session {
        editor: Editor::new(base.clone()),
        entries: Vec::new(),
    };
    let mut addition = base.transaction("Draw gear pair");
    let first = addition.add_sketch_entity(sketch, Entity::Point(Point2::new(0.0, 80.0)));
    let second = addition.add_sketch_entity(sketch, Entity::Point(Point2::new(50.0, 80.0)));
    let other = addition.add_sketch_gear(sketch, gear_of(first, Expression::Number(24.0)));
    addition.add_sketch_gear(sketch, partner_of(second, other, Expression::Number(26.0)));
    session.apply(addition.finish());
    let edited = session.editor.document().clone();
    let held = edited
        .feature(sketch)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|drawn| drawn.gear(partner))
        .cloned()
        .unwrap();
    let mut change = edited.transaction("Edit spur gear");
    change.set_sketch_gear(
        sketch,
        partner,
        Gear {
            teeth: Expression::Number(32.0),
            ..held
        },
    );
    session.apply(change.finish());
    let changed = session.editor.document().clone();
    let mut removal = changed.transaction("Delete gear");
    removal.remove_sketch_items(sketch, [driver], []);
    session.apply(removal.finish());
    let model = save_bytes(&base, None, at(1_700_000_000), Some("Gear pairs")).unwrap();
    let on_disk = decode(&model).unwrap().digest.unwrap();
    let head = JournalHead {
        file: Some(Path::new("/models/gear-pairs.caditor")),
        on_disk: Some(&on_disk),
        loaded_with_problems: false,
        folded: 0,
    };
    encode_journal(&head, &base, &session.entries).unwrap()
}

fn sprocket_of(centre: EntityId, teeth: Expression) -> Gear {
    Gear::sprocket(
        centre,
        ChainSize::DEFAULT,
        teeth,
        Expression::Measure(12.0, Unit::Millimetre),
        Expression::Measure(0.0, Unit::Degree),
    )
}

fn sprockets() -> (Document, FeatureId, EntityId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sprockets");
    let count = transaction.add_parameter("sprocket_teeth", transaction.parse("19").unwrap());
    let sketch = transaction.add_feature("Sprockets", FeatureKind::from(Sketch::new(Plane::XY)));
    let first = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(0.0, 0.0)));
    let second = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(120.0, 0.0)));
    transaction.add_sketch_constraint(sketch, Constraint::Coincident(first, EntityId::ORIGIN));
    let plain = transaction.add_sketch_gear(sketch, sprocket_of(first, Expression::Number(17.0)));
    transaction.add_sketch_gear(
        sketch,
        Gear {
            chain: ChainSize::find("10A"),
            turn: Expression::Measure(9.0, Unit::Degree),
            ..sprocket_of(second, Expression::Parameter(count))
        },
    );
    document.apply(transaction.finish()).unwrap();
    (document, sketch, plain)
}

fn sprockets_journal() -> Vec<u8> {
    let (base, sketch, plain) = sprockets();
    let mut session = Session {
        editor: Editor::new(base.clone()),
        entries: Vec::new(),
    };
    let mut addition = base.transaction("Draw sprocket");
    let centre = addition.add_sketch_entity(sketch, Entity::Point(Point2::new(0.0, 90.0)));
    addition.add_sketch_gear(sketch, sprocket_of(centre, Expression::Number(25.0)));
    session.apply(addition.finish());
    let edited = session.editor.document().clone();
    let held = edited
        .feature(sketch)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|drawn| drawn.gear(plain))
        .cloned()
        .unwrap();
    let mut change = edited.transaction("Edit sprocket");
    change.set_sketch_gear(
        sketch,
        plain,
        Gear {
            chain: ChainSize::find("12B"),
            teeth: Expression::Number(21.0),
            ..held
        },
    );
    session.apply(change.finish());
    let changed = session.editor.document().clone();
    let mut removal = changed.transaction("Delete sprocket");
    removal.remove_sketch_items(sketch, [plain], []);
    session.apply(removal.finish());
    let model = save_bytes(&base, None, at(1_700_000_000), Some("Sprockets")).unwrap();
    let on_disk = decode(&model).unwrap().digest.unwrap();
    let head = JournalHead {
        file: Some(Path::new("/models/sprockets.caditor")),
        on_disk: Some(&on_disk),
        loaded_with_problems: false,
        folded: 0,
    };
    encode_journal(&head, &base, &session.entries).unwrap()
}

fn gears() -> (Document, FeatureId, EntityId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Gears");
    let count = transaction.add_parameter("teeth", transaction.parse("24").unwrap());
    let sketch = transaction.add_feature("Wheels", FeatureKind::from(Sketch::new(Plane::XY)));
    let first = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(0.0, 0.0)));
    let second = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(60.0, 0.0)));
    transaction.add_sketch_constraint(sketch, Constraint::Coincident(first, EntityId::ORIGIN));
    let plain = transaction.add_sketch_gear(sketch, gear_of(first, Expression::Number(20.0)));
    transaction.add_sketch_gear(
        sketch,
        Gear {
            module: Expression::Measure(1.5, Unit::Millimetre),
            profile_shift: Expression::Number(0.2),
            bore: Expression::Measure(0.0, Unit::Millimetre),
            turn: Expression::Measure(7.5, Unit::Degree),
            ..gear_of(second, Expression::Parameter(count))
        },
    );
    transaction.add_feature(
        "Wheel body",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Measure(3.0, Unit::Millimetre), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, sketch, plain)
}

fn gears_journal() -> Vec<u8> {
    let (base, sketch, plain) = gears();
    let mut session = Session {
        editor: Editor::new(base.clone()),
        entries: Vec::new(),
    };
    let mut addition = base.transaction("Draw spur gear");
    let centre = addition.add_sketch_entity(sketch, Entity::Point(Point2::new(0.0, 50.0)));
    addition.add_sketch_gear(sketch, gear_of(centre, Expression::Number(30.0)));
    session.apply(addition.finish());
    let edited = session.editor.document().clone();
    let held = edited
        .feature(sketch)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|drawn| drawn.gear(plain))
        .cloned()
        .unwrap();
    let mut change = edited.transaction("Edit spur gear");
    change.set_sketch_gear(
        sketch,
        plain,
        Gear {
            teeth: Expression::Number(22.0),
            ..held
        },
    );
    session.apply(change.finish());
    let changed = session.editor.document().clone();
    let mut removal = changed.transaction("Delete gear");
    removal.remove_sketch_items(sketch, [plain], []);
    session.apply(removal.finish());
    let model = save_bytes(&base, None, at(1_700_000_000), Some("Gears")).unwrap();
    let on_disk = decode(&model).unwrap().digest.unwrap();
    let head = JournalHead {
        file: Some(Path::new("/models/gears.caditor")),
        on_disk: Some(&on_disk),
        loaded_with_problems: false,
        folded: 0,
    };
    encode_journal(&head, &base, &session.entries).unwrap()
}

fn texts_journal() -> Vec<u8> {
    let (base, sketch, plain) = texts();
    let mut session = Session {
        editor: Editor::new(base.clone()),
        entries: Vec::new(),
    };
    let mut addition = base.transaction("Draw text");
    let anchor = addition.add_sketch_entity(sketch, Entity::Point(Point2::new(30.0, 3.0)));
    addition.add_sketch_text(
        sketch,
        Text {
            anchor,
            content: "M8".to_owned(),
            height: Expression::Measure(4.0, Unit::Millimetre),
            angle: Expression::Measure(90.0, Unit::Degree),
            bold: false,
            italic: true,
            align: TextAlign::Left,
        },
    );
    session.apply(addition.finish());
    let edited = session.editor.document().clone();
    let held = edited
        .feature(sketch)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|drawn| drawn.text(plain))
        .cloned()
        .unwrap();
    let mut change = edited.transaction("Edit text");
    change.set_sketch_text(
        sketch,
        plain,
        Text {
            content: "Part 8".to_owned(),
            bold: true,
            ..held
        },
    );
    session.apply(change.finish());
    let changed = session.editor.document().clone();
    let mut removal = changed.transaction("Delete text");
    removal.remove_sketch_items(sketch, [plain], []);
    session.apply(removal.finish());
    let model = save_bytes(&base, None, at(1_700_000_000), Some("Texts")).unwrap();
    let on_disk = decode(&model).unwrap().digest.unwrap();
    let head = JournalHead {
        file: Some(Path::new("/models/texts.caditor")),
        on_disk: Some(&on_disk),
        loaded_with_problems: false,
        folded: 0,
    };
    encode_journal(&head, &base, &session.entries).unwrap()
}

fn principal_showing_set(showing: [(PrincipalGeometry, PrincipalShowing); 3]) -> Transaction {
    Transaction::new(
        "Show",
        showing
            .into_iter()
            .map(|(geometry, showing)| Edit::SetPrincipalShowing { geometry, showing })
            .collect(),
    )
}

fn principal_showing() -> (Document, Transaction) {
    let (mut document, _, _) = solid_model();
    document
        .apply(principal_showing_set([
            (
                PrincipalGeometry::Plane(PrincipalPlane::Xy),
                PrincipalShowing::Shown,
            ),
            (PrincipalGeometry::Origin, PrincipalShowing::Shown),
            (
                PrincipalGeometry::Axis(PrincipalAxis::Z),
                PrincipalShowing::Hidden,
            ),
        ]))
        .unwrap();
    let change = principal_showing_set([
        (
            PrincipalGeometry::Plane(PrincipalPlane::Xz),
            PrincipalShowing::Shown,
        ),
        (PrincipalGeometry::Origin, PrincipalShowing::WhenNeeded),
        (
            PrincipalGeometry::Axis(PrincipalAxis::Z),
            PrincipalShowing::Shown,
        ),
    ]);
    (document, change)
}

fn principal_showing_journal() -> Vec<u8> {
    let (base, change) = principal_showing();
    let mut session = Session {
        editor: Editor::new(base.clone()),
        entries: Vec::new(),
    };
    session.apply(change);
    let model = save_bytes(&base, None, at(1_700_000_000), Some("Principal showing")).unwrap();
    let on_disk = decode(&model).unwrap().digest.unwrap();
    let head = JournalHead {
        file: Some(Path::new("/models/principal-showing.caditor")),
        on_disk: Some(&on_disk),
        loaded_with_problems: false,
        folded: 0,
    };
    encode_journal(&head, &base, &session.entries).unwrap()
}

fn piece_stand_ins_journal() -> Vec<u8> {
    let (base, sketch, lines) = separated_blocks();
    let mut session = Session {
        editor: Editor::new(base.clone()),
        entries: Vec::new(),
    };
    let mut removal = base.transaction("Delete a side");
    removal.remove_sketch_items(sketch, [lines[1]], []);
    session.apply(removal.finish());
    let model = save_bytes(&base, None, at(1_700_000_000), Some("Blocks")).unwrap();
    let on_disk = decode(&model).unwrap().digest.unwrap();
    let head = JournalHead {
        file: Some(Path::new("/models/blocks.caditor")),
        on_disk: Some(&on_disk),
        loaded_with_problems: false,
        folded: 0,
    };
    encode_journal(&head, &base, &session.entries).unwrap()
}

fn separate_pieces_journal() -> Vec<u8> {
    let (drawn, lines) = blocks_sketch();
    let mut base = Document::default();
    let mut transaction = base.transaction("Blocks");
    let sketch = transaction.add_feature("Blocks sketch", FeatureKind::from(drawn));
    let extruded = transaction.add_feature("Blocks", blocks(sketch));
    base.apply(transaction.finish()).unwrap();
    let middle = extruded.piece(piece_key(lines[1])).unwrap();
    let mut session = Session {
        editor: Editor::new(base.clone()),
        entries: Vec::new(),
    };
    session.apply(Transaction::single(
        "A body each",
        Edit::SetFeaturePieces {
            id: extruded,
            pieces: Pieces::BodyEach,
        },
    ));
    session.apply(Transaction::new(
        "Looks",
        vec![
            Edit::SetBodyAppearance {
                id: middle,
                appearance: BodyAppearance {
                    name: Some("Middle block".to_owned()),
                    ..BodyAppearance::default()
                },
            },
            Edit::SetFeatureHidden {
                id: extruded.piece(piece_key(lines[2])).unwrap(),
                hidden: true,
            },
        ],
    ));
    let model = save_bytes(&base, None, at(1_700_000_000), Some("Blocks")).unwrap();
    let on_disk = decode(&model).unwrap().digest.unwrap();
    let head = JournalHead {
        file: Some(Path::new("/models/blocks.caditor")),
        on_disk: Some(&on_disk),
        loaded_with_problems: false,
        folded: 0,
    };
    encode_journal(&head, &base, &session.entries).unwrap()
}

fn sketches() -> Document {
    let (mut document, _) = with_added_kinds(every_kind().document);
    let mut transaction = document.transaction("Curves");
    let bore = transaction.add_parameter("bore", transaction.parse("30 mm").unwrap());
    let mut curves = Sketch::new(Plane::XZ);
    let ellipse = curves.add_ellipse(Point2::new(1.0, 2.0), Point2::new(11.0, 2.0), 4.0);
    let elliptical = curves.add_elliptical_arc(
        Point2::new(30.0, 0.0),
        Point2::new(30.0, 8.0),
        3.0,
        Point2::new(30.0, -8.0),
        Point2::new(30.0, 8.0),
    );
    let top = curves.add_point(Point2::new(1.0, 6.0));
    let through = [
        Point2::new(0.0, 20.0),
        Point2::new(10.0, 24.0),
        Point2::new(20.0, 18.0),
    ];
    curves.add_spline_of(&through, SplineKind::Control { closed: true });
    for closed in [false, true] {
        curves.add_spline_of(
            &through,
            SplineKind::Fit {
                closed,
                spacing: FitSpacing::Even,
            },
        );
        curves.add_spline_of(&through, SplineKind::fit(closed));
    }
    let conic = curves.add_spline_of(&through, SplineKind::Conic { rho: 0.3 });
    let first = curves.add_spline(&[
        Point2::new(0.0, 40.0),
        Point2::new(10.0, 50.0),
        Point2::new(20.0, 40.0),
    ]);
    let second = curves.add_spline(&[
        Point2::new(20.0, 40.0),
        Point2::new(25.0, 35.0),
        Point2::new(30.0, 25.0),
    ]);
    let ends = [first, second].map(|spline| match curves.entity(spline) {
        Some(Entity::Spline { points, .. }) => points.clone(),
        other => panic!("expected a spline, found {other:?}"),
    });
    let arc = curves.add_arc(
        Point2::new(50.0, 5.0),
        Point2::new(58.0, 5.0),
        Point2::new(50.0, 13.0),
    );
    let axis = curves.add_line(Point2::new(70.0, -10.0), Point2::new(70.0, 10.0));
    curves.set_construction(axis, true).unwrap();
    let rim = curves.add_point(Point2::new(82.0, 0.0));
    let line = curves.add_line(Point2::new(-5.0, 6.0), Point2::new(15.0, 6.0));
    let Some(Entity::Line { start, end }) = curves.entity(line).cloned() else {
        panic!("expected a line");
    };
    let mut add = |constraint| curves.add_constraint(constraint).unwrap();
    add(Constraint::Tangent(line, ellipse));
    add(Constraint::MajorRadius {
        ellipse,
        value: Expression::Measure(10.0, Unit::Millimetre),
    });
    add(Constraint::MinorRadius {
        ellipse: elliptical,
        value: Expression::Measure(3.0, Unit::Millimetre),
    });
    add(Constraint::Coincident(top, ellipse));
    add(Constraint::OnMinorAxis {
        point: top,
        ellipse,
    });
    add(Constraint::Rho {
        conic,
        value: Expression::Number(0.3),
    });
    add(Constraint::Coincident(ends[1][0], ends[0][2]));
    add(Constraint::Curvature(first, second));
    add(Constraint::ArcLength {
        arc,
        value: Expression::Measure(12.0, Unit::Millimetre),
    });
    add(Constraint::Sweep {
        arc,
        value: Expression::Measure(80.0, Unit::Degree),
    });
    add(Constraint::AxisDiameter {
        point: rim,
        axis,
        value: Expression::Parameter(bore),
    });
    let level = add(Constraint::Horizontal(line));
    let measured = add(Constraint::Distance {
        from: start,
        to: end,
        value: Expression::Measure(20.0, Unit::Millimetre),
    });
    curves.set_active(level, false).unwrap();
    curves
        .set_label_offset(measured, Some(Vector2::new(5.0, -12.5)))
        .unwrap();
    curves.set_construction(elliptical, true).unwrap();
    let projected = transaction.add_feature("Curves", FeatureKind::from(curves));
    let body = FeatureId::from_raw(90);
    let projections = [
        (
            ProjectionSource::SketchEntity {
                sketch: FeatureId::from_raw(0),
                entity: EntityId::from_raw(2),
            },
            Outline::Line {
                start: Point2::ZERO,
                end: Point2::new(4.0, 1.0),
            },
        ),
        (
            ProjectionSource::Edge {
                body,
                edge: edge(5),
            },
            Outline::Arc {
                center: Point2::ZERO,
                start: Point2::new(2.0, 0.0),
                end: Point2::new(0.0, 2.0),
            },
        ),
        (
            ProjectionSource::Vertex {
                body,
                vertex: VertexName::from_digest(7),
            },
            Outline::Point(Point2::new(-1.0, 3.0)),
        ),
        (
            ProjectionSource::Section {
                body,
                edge: edge(15),
            },
            Outline::Circle {
                center: Point2::ZERO,
                radius: 3.0,
            },
        ),
        (
            ProjectionSource::DatumPlane {
                datum: FeatureId::from_raw(91),
                reach: 25.0,
            },
            Outline::Line {
                start: Point2::new(0.0, -25.0),
                end: Point2::new(0.0, 25.0),
            },
        ),
        (
            ProjectionSource::PrincipalPlane {
                plane: PrincipalPlane::Xy,
                reach: 40.0,
            },
            Outline::Line {
                start: Point2::new(-40.0, 0.0),
                end: Point2::new(40.0, 0.0),
            },
        ),
    ];
    document.apply(transaction.finish()).unwrap();

    let mut transaction = document.transaction("Project");
    for (source, outline) in projections {
        transaction.add_projection(projected, source, &outline);
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn projected_datums() -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Datums");
    let spot = transaction.add_feature(
        "Spot",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: PointReference::Origin,
            offset: ["10 mm", "5 mm", "0 mm"].map(|text| Expression::parse_stored(text).unwrap()),
        })),
    );
    let post = transaction.add_feature(
        "Post",
        FeatureKind::Datum(Datum::Axis(DatumAxis::NormalTo(
            PlaneReference::Principal(PrincipalPlane::Xy),
            PointReference::Datum(spot),
        ))),
    );
    let side = transaction.add_feature("Side", FeatureKind::from(Sketch::new(Plane::XZ)));
    let projections = [
        (
            ProjectionSource::DatumAxis {
                datum: post,
                reach: 20.0,
            },
            Outline::Line {
                start: Point2::new(10.0, -20.0),
                end: Point2::new(10.0, 20.0),
            },
        ),
        (
            ProjectionSource::PrincipalAxis {
                axis: PrincipalAxis::X,
                reach: 30.0,
            },
            Outline::Line {
                start: Point2::new(-30.0, 0.0),
                end: Point2::new(30.0, 0.0),
            },
        ),
        (
            ProjectionSource::PrincipalAxis {
                axis: PrincipalAxis::Y,
                reach: 30.0,
            },
            Outline::Point(Point2::ZERO),
        ),
        (
            ProjectionSource::DatumPoint { datum: spot },
            Outline::Point(Point2::new(10.0, 0.0)),
        ),
        (ProjectionSource::Origin, Outline::Point(Point2::ZERO)),
    ];
    for (source, outline) in projections {
        transaction.add_projection(side, source, &outline);
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn plain_extrude(sketch: FeatureId, extent: ExtrudeExtent, operation: BodyOperation) -> Extrude {
    Extrude {
        sketch,
        regions: RegionChoice::All,
        extent,
        operation,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }
}

fn plain_revolve(sketch: FeatureId, operation: BodyOperation) -> Revolve {
    Revolve {
        sketch,
        regions: RegionChoice::All,
        axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
        extent: RevolveExtent::Full,
        operation,
        start: None,
        other_bodies: Vec::new(),
        side: None,
        wall: None,
    }
}

fn path_sweeps() -> Document {
    let (mut document, base, _) = solid_model();
    let sketch = named(&document, "Outline");
    let mut transaction = document.transaction("Path sweeps");
    let mut path = Sketch::new(Plane::YZ);
    let first = path.add_line(Point2::ZERO, Point2::new(0.0, 5.0));
    let second = path.add_arc(
        Point2::new(3.0, 5.0),
        Point2::new(3.0, 8.0),
        Point2::new(0.0, 5.0),
    );
    let path = transaction.add_feature("Path", FeatureKind::from(path));
    transaction.add_feature(
        "Along the path",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep::new(
            sketch,
            SweepPath::Curves {
                sketch: path,
                curves: vec![first, second],
            },
        ))),
    );
    transaction.add_feature(
        "Along an edge",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep {
            orientation: SweepOrientation::Parallel,
            operation: BodyOperation::Remove(base),
            ..Sweep::new(
                sketch,
                SweepPath::Edges {
                    body: base,
                    edges: vec![edge(0x5eed)],
                },
            )
        })),
    );
    transaction.add_feature(
        "Unchosen",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep::new(sketch, SweepPath::Unchosen))),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn sweep_shapes() -> Document {
    let (mut document, base, _) = solid_model();
    let sketch = named(&document, "Outline");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Sweep shapes");
    let mut path = Sketch::new(Plane::YZ);
    let line = path.add_line(Point2::ZERO, Point2::new(0.0, 20.0));
    let path = transaction.add_feature("Path", FeatureKind::from(path));
    let along = SweepPath::Curves {
        sketch: path,
        curves: vec![line],
    };
    transaction.add_feature(
        "Twisted",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep {
            twist: parse("90 deg"),
            ..Sweep::new(sketch, along.clone())
        })),
    );
    transaction.add_feature(
        "Tapered cut",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep {
            scale: parse("0.5"),
            operation: BodyOperation::Remove(base),
            ..Sweep::new(sketch, along)
        })),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn point_targets() -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Point targets");
    transaction.add_feature("Tracked", FeatureKind::from(tracked_sketch()));
    document.apply(transaction.finish()).unwrap();
    document
}

fn pipes() -> Document {
    let (mut document, base, _) = solid_model();
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Pipes");
    let mut path = Sketch::new(Plane::XY);
    let first = path.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let second = path.add_line(Point2::new(20.0, 0.0), Point2::new(20.0, 30.0));
    let path = transaction.add_feature("Pipe path", FeatureKind::from(path));
    let curves = SweepPath::Curves {
        sketch: path,
        curves: vec![first, second],
    };
    transaction.add_feature(
        "Round",
        FeatureKind::Pipe(Pipe::new(curves.clone(), parse("4 mm"), parse("6 mm"))),
    );
    transaction.add_feature(
        "Square tube",
        FeatureKind::Pipe(Pipe {
            shape: PipeShape::Square,
            wall: Some(parse("1 mm")),
            ..Pipe::new(curves, parse("5 mm"), parse("0 mm"))
        }),
    );
    transaction.add_feature(
        "Triangular groove",
        FeatureKind::Pipe(Pipe {
            shape: PipeShape::Triangle,
            operation: BodyOperation::Remove(base),
            ..Pipe::new(
                SweepPath::Edges {
                    body: base,
                    edges: vec![edge(0x5eed)],
                },
                parse("2 mm"),
                parse("3 mm"),
            )
        }),
    );
    transaction.add_feature(
        "Unchosen pipe",
        FeatureKind::Pipe(Pipe::new(SweepPath::Unchosen, parse("4 mm"), parse("6 mm"))),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn coils() -> Document {
    let (mut document, base, _) = solid_model();
    let outline = named(&document, "Outline");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Coils");
    transaction.add_feature(
        "Spring",
        FeatureKind::Coil(Coil::new(
            AxisReference::Principal(PrincipalAxis::Z),
            parse("20 mm"),
            parse("2 mm"),
        )),
    );
    transaction.add_feature(
        "Square groove",
        FeatureKind::Coil(Coil {
            reversed: true,
            length: CoilLength::HeightAndTurns {
                height: parse("12 mm"),
                turns: parse("3"),
            },
            shape: CoilShape::Square,
            placement: CoilPlacement::Inside,
            operation: BodyOperation::Remove(base),
            ..Coil::new(
                AxisReference::Edge {
                    body: base,
                    edge: Box::new(edge(0xc011)),
                },
                parse("6 mm"),
                parse("1 mm"),
            )
        }),
    );
    transaction.add_feature(
        "Tapered spring",
        FeatureKind::Coil(Coil {
            length: CoilLength::HeightAndPitch {
                height: parse("30 mm"),
                pitch: parse("4 mm"),
            },
            placement: CoilPlacement::Outside,
            taper: parse("8 deg"),
            turning: CoilTurning::Clockwise,
            ..Coil::new(
                AxisReference::Sketch {
                    sketch: outline,
                    entity: EntityId::from_raw(1),
                },
                parse("15 mm"),
                parse("1.5 mm"),
            )
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn helices() -> Document {
    let (mut document, base, _) = solid_model();
    let outline = named(&document, "Outline");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Helices");
    let helix = transaction.add_feature(
        "Helix",
        FeatureKind::Helix(Helix::new(
            AxisReference::Principal(PrincipalAxis::Z),
            parse("20 mm"),
        )),
    );
    transaction.add_feature(
        "Tapered helix",
        FeatureKind::Helix(Helix {
            reversed: true,
            length: CoilLength::HeightAndPitch {
                height: parse("30 mm"),
                pitch: parse("4 mm"),
            },
            taper: parse("8 deg"),
            turning: CoilTurning::Clockwise,
            ..Helix::new(
                AxisReference::Sketch {
                    sketch: outline,
                    entity: EntityId::from_raw(1),
                },
                parse("15 mm"),
            )
        }),
    );
    transaction.add_feature(
        "Thread",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep {
            operation: BodyOperation::Remove(base),
            other_bodies: vec![base],
            ..Sweep::new(outline, SweepPath::Helix { helix })
        })),
    );
    transaction.add_feature(
        "Wire",
        FeatureKind::Pipe(Pipe::new(
            SweepPath::Helix { helix },
            parse("2 mm"),
            parse("0 mm"),
        )),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn helix_points() -> Document {
    let (mut document, _, _) = solid_model();
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Helix points");
    let mut marks = Sketch::new(Plane::XY);
    let mark = marks.add_point(Point2::new(6.0, 4.0));
    let marks = transaction.add_feature("Start mark", FeatureKind::from(marks));
    let helix = transaction.add_feature(
        "Pointed helix",
        FeatureKind::Helix(Helix {
            start: parse("45 deg"),
            start_point: Some(PointReference::Sketch {
                sketch: marks,
                entity: mark,
            }),
            ..Helix::new(AxisReference::Principal(PrincipalAxis::Z), parse("20 mm"))
        }),
    );
    transaction.add_feature(
        "Wire",
        FeatureKind::Pipe(Pipe::new(
            SweepPath::Helix { helix },
            parse("2 mm"),
            parse("0 mm"),
        )),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn helix_starts() -> Document {
    let (mut document, _, _) = solid_model();
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Helix starts");
    let helix = transaction.add_feature(
        "Started helix",
        FeatureKind::Helix(Helix {
            start: parse("45 deg"),
            turning: CoilTurning::Clockwise,
            ..Helix::new(AxisReference::Principal(PrincipalAxis::Z), parse("20 mm"))
        }),
    );
    transaction.add_feature(
        "Wire",
        FeatureKind::Pipe(Pipe::new(
            SweepPath::Helix { helix },
            parse("2 mm"),
            parse("0 mm"),
        )),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn flat_spirals() -> Document {
    let (mut document, _, _) = solid_model();
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Flat spirals");
    let spiral = transaction.add_feature(
        "Clock spring",
        FeatureKind::Helix(Helix {
            length: CoilLength::HeightAndTurns {
                height: parse("60 mm"),
                turns: parse("4"),
            },
            turning: CoilTurning::Clockwise,
            ..Helix::flat_spiral(AxisReference::Principal(PrincipalAxis::Z), parse("10 mm"))
        }),
    );
    transaction.add_feature(
        "Spring",
        FeatureKind::Pipe(Pipe::new(
            SweepPath::Helix { helix: spiral },
            parse("1.5 mm"),
            parse("0 mm"),
        )),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn loft_ends() -> Document {
    let (mut document, _, _) = solid_model();
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Loft ends");
    let mut profiles = Vec::new();
    for (index, angle) in [0.0_f64, 120.0, 240.0].into_iter().enumerate() {
        let radial = Vector3::new(angle.to_radians().cos(), angle.to_radians().sin(), 0.0);
        let plane = Plane::from_frame(
            Point3::ZERO + radial * 20.0,
            Vector3::Z.cross(radial),
            radial,
        )
        .unwrap();
        let mut sketch = Sketch::new(plane);
        sketch.add_circle(Point2::ZERO, 2.0);
        profiles.push(LoftSection::whole(transaction.add_feature(
            format!("Ring section {}", index + 1),
            FeatureKind::from(sketch),
        )));
    }
    let open = profiles.get(..2).unwrap().to_vec();
    transaction.add_feature(
        "Ring",
        FeatureKind::Loft(Loft {
            closed: true,
            ..Loft::new(profiles)
        }),
    );
    transaction.add_feature(
        "Square ends",
        FeatureKind::Loft(Loft {
            square_start: Some(parse("1")),
            square_end: Some(parse("1.5")),
            ..Loft::new(open)
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn lofts() -> Document {
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Lofts");
    let level = |height: f64| Plane::new(Point3::new(0.0, 0.0, height), Vector3::Z).unwrap();
    let mut low = Sketch::new(level(0.0));
    let corners = [(0.0, 0.0), (6.0, 0.0), (6.0, 4.0), (0.0, 4.0)];
    for index in 0..4 {
        let (a, b) = (corners[index], corners[(index + 1) % 4]);
        low.add_line(Point2::new(a.0, a.1), Point2::new(b.0, b.1));
    }
    let low = transaction.add_feature("Loft bottom", FeatureKind::from(low));
    let mut middle = Sketch::new(level(4.0));
    middle.add_circle(Point2::new(3.0, 2.0), 1.5);
    let middle = transaction.add_feature("Loft middle", FeatureKind::from(middle));
    let mut high = Sketch::new(level(8.0));
    let disc = high.add_circle(Point2::new(3.0, 2.0), 1.0);
    high.add_circle(Point2::new(9.0, 2.0), 1.0);
    let tip = high.add_point(Point2::new(3.0, 2.0));
    let high = transaction.add_feature("Loft top", FeatureKind::from(high));
    let chosen = RegionReference::new(
        RegionKey::from_digest(0x0abc_def0_1234_5678_9abc_def0_1234_5678),
        BTreeSet::from([BoundaryPiece {
            entity: disc.raw(),
            side: Side::Left,
            piece: 0x1111_2222_3333_4444_5555_6666_7777_8888,
        }]),
        Some(Point2::new(3.0, 2.0)),
    );
    transaction.add_feature(
        "Smooth loft",
        FeatureKind::Loft(Loft::new(vec![
            LoftSection::whole(low),
            LoftSection::whole(middle),
            LoftSection {
                sketch: high,
                region: Some(chosen),
            },
        ])),
    );
    transaction.add_feature(
        "Ruled spike",
        FeatureKind::Loft(Loft {
            end: Some(PointReference::Sketch {
                sketch: high,
                entity: tip,
            }),
            skin: LoftSkin::Ruled,
            operation: BodyOperation::Add(base),
            ..Loft::new(vec![LoftSection::whole(middle)])
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn ribs() -> Document {
    let (mut document, base, _) = solid_model();
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Ribs");
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(2.0, 2.0), Point2::new(8.0, 6.0));
    let sketch = transaction.add_feature("Rib line", FeatureKind::from(sketch));
    transaction.add_feature(
        "Rib",
        FeatureKind::Rib(Rib::new(sketch, base, parse("2 mm"))),
    );
    transaction.add_feature(
        "Web",
        FeatureKind::Rib(Rib {
            side: RibSide::OneSide,
            direction: RibDirection::Square,
            reversed: true,
            ..Rib::new(sketch, base, parse("1.5 mm"))
        }),
    );
    transaction.add_feature(
        "Other side",
        FeatureKind::Rib(Rib {
            side: RibSide::OtherSide,
            ..Rib::new(sketch, base, parse("1 mm"))
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn shaped_ribs() -> Document {
    let (mut document, base, _) = solid_model();
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Shaped ribs");
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(2.0, 2.0), Point2::new(8.0, 2.0));
    sketch.add_line(Point2::new(5.0, 2.0), Point2::new(5.0, 6.0));
    sketch.add_arc(
        Point2::new(5.0, 5.0),
        Point2::new(7.0, 5.0),
        Point2::new(5.0, 7.0),
    );
    let sketch = transaction.add_feature("Web lines", FeatureKind::from(sketch));
    transaction.add_feature(
        "Drafted web",
        FeatureKind::Rib(Rib {
            direction: RibDirection::Square,
            draft: parse("1.5 deg"),
            ..Rib::new(sketch, base, parse("1 mm"))
        }),
    );
    transaction.add_feature(
        "Round web",
        FeatureKind::Rib(Rib {
            direction: RibDirection::Square,
            ends: RibEnds::RoundArcs,
            draft: parse("2 deg"),
            reversed: true,
            ..Rib::new(sketch, base, parse("1 mm"))
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn sweeps() -> Document {
    let (mut document, base, _) = solid_model();
    let sketch = named(&document, "Outline");
    let mut transaction = document.transaction("Sweeps");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let one_side = |text: &str| ExtrudeExtent::one_side(parse(text), false);
    let level = transaction.add_feature(
        "Level",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: parse("20 mm"),
        })),
    );
    let leaning = transaction.add_feature(
        "Leaning",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            direction: Some(Box::new(AxisReference::Principal(PrincipalAxis::Z))),
            ..plain_extrude(sketch, one_side("10 mm"), BodyOperation::NewBody)
        })),
    );
    let extrusions = [
        (
            "Tapered",
            Extrude {
                taper: Some(Box::new(parse("-3 deg"))),
                wall: Some(Box::new(Wall {
                    thickness: parse("1.5 mm"),
                    side: WallSide::Outside,
                })),
                ..plain_extrude(sketch, one_side("10 mm"), BodyOperation::NewBody)
            },
        ),
        (
            "Through both",
            Extrude {
                other_bodies: vec![leaning],
                ..plain_extrude(sketch, one_side("4 mm"), BodyOperation::Remove(base))
            },
        ),
        (
            "Short",
            plain_extrude(
                sketch,
                ExtrudeExtent::OneSide {
                    end: ExtrudeEnd::up_to_face(PlaneReference::Datum(level))
                        .with_offset(Some(parse("-2 mm"))),
                    reversed: false,
                },
                BodyOperation::Remove(base),
            ),
        ),
        (
            "Up to it",
            plain_extrude(
                sketch,
                ExtrudeExtent::TwoSides {
                    forward: ExtrudeEnd::up_to_surface(caditor_document::FaceAttachment {
                        body: base,
                        face: FaceReference::new(
                            FaceName::from_digest(0x51de),
                            Some(FaceOrigin::EndCap {
                                feature: base.raw(),
                            }),
                            [FaceName::from_digest(7)],
                        ),
                    }),
                    backward: ExtrudeEnd::up_to_next().with_offset(Some(parse("1 mm"))),
                },
                BodyOperation::Add(base),
            ),
        ),
    ];
    for (name, extrude) in extrusions {
        transaction.add_feature(name, FeatureKind::Solid(SolidFeature::Extrude(extrude)));
    }
    let revolutions = [
        (
            "Up to the plane",
            Revolve {
                extent: RevolveExtent::up_to(PlaneReference::Principal(PrincipalPlane::Yz), true),
                ..plain_revolve(sketch, BodyOperation::NewBody)
            },
        ),
        (
            "Left",
            Revolve {
                side: Some(AxisSide::Left),
                ..plain_revolve(sketch, BodyOperation::NewBody)
            },
        ),
        (
            "Right cup",
            Revolve {
                side: Some(AxisSide::Right),
                start: Some(SolidStart::Distance(parse("3 mm"))),
                wall: Some(Box::new(Wall {
                    thickness: parse("1 mm"),
                    side: WallSide::Centred,
                })),
                ..plain_revolve(sketch, BodyOperation::Add(base))
            },
        ),
    ];
    for (name, revolve) in revolutions {
        transaction.add_feature(name, FeatureKind::Solid(SolidFeature::Revolve(revolve)));
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn holes() -> Document {
    let (mut document, base, _) = solid_model();
    let sketch = named(&document, "Outline");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let plain = Hole {
        sketch,
        body: base,
        diameter: parse("4 mm"),
        depth: HoleDepth::Blind(parse("$0 * 2")),
        style: HoleStyle::Plain,
        reversed: false,
        shape: HoleShape::Round,
        standard: None,
        sizing: HoleSizing::Typed,
        bottom: HoleBottom::Flat,
        thread: TappedThread::default(),
    };
    let mut transaction = document.transaction("Holes");
    let level = transaction.add_feature(
        "Level",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: parse("-3 mm"),
        })),
    );
    let stepped = HoleStyle::Stepped(vec![
        HoleStep {
            diameter: parse("10 mm"),
            depth: parse("1 mm"),
        },
        HoleStep {
            diameter: parse("7 mm"),
            depth: parse("$0 / 2"),
        },
    ]);
    let holes = [
        ("Plain", plain.clone()),
        (
            "Counterbored",
            Hole {
                depth: HoleDepth::ThroughAll,
                reversed: true,
                style: HoleStyle::Counterbore {
                    diameter: parse("8 mm"),
                    depth: parse("2 mm"),
                },
                ..plain.clone()
            },
        ),
        (
            "Countersunk",
            Hole {
                style: HoleStyle::Countersink {
                    diameter: parse("9 mm"),
                    angle: parse("90 deg"),
                },
                ..plain.clone()
            },
        ),
        (
            "Stepped",
            Hole {
                style: stepped.clone(),
                sizing: HoleSizing::Circles,
                ..plain.clone()
            },
        ),
        (
            "Drilled",
            Hole {
                style: stepped,
                bottom: HoleBottom::DrillPoint(parse("118 deg")),
                ..plain.clone()
            },
        ),
        (
            "By circles",
            Hole {
                sizing: HoleSizing::Circles,
                ..plain.clone()
            },
        ),
        (
            "Fine",
            Hole {
                sizing: HoleSizing::CirclesAndHeads,
                standard: Some(HoleStandard {
                    size: MetricSize::M10,
                    fit: HoleFit::TappedFine(FinePitch::Third),
                }),
                ..plain.clone()
            },
        ),
        (
            "To face",
            Hole {
                depth: HoleDepth::up_to_face(PlaneReference::Datum(level))
                    .with_offset(Some(parse("-1 mm"))),
                bottom: HoleBottom::DrillPoint(parse("118 deg")),
                ..plain.clone()
            },
        ),
        (
            "To next",
            Hole {
                depth: HoleDepth::up_to_next(),
                ..plain.clone()
            },
        ),
        (
            "Slot",
            Hole {
                shape: HoleShape::Slot {
                    length: parse("12 mm"),
                    angle: parse("30 deg"),
                },
                standard: Some(HoleStandard {
                    size: MetricSize::M2_5,
                    fit: HoleFit::Tapped,
                }),
                ..plain.clone()
            },
        ),
        (
            "Tapped",
            Hole {
                standard: Some(HoleStandard {
                    size: MetricSize::M6,
                    fit: HoleFit::Tapped,
                }),
                thread: TappedThread {
                    class: ThreadFamily::MetricCoarse.class_from_id("5H"),
                    hand: ThreadHand::Left,
                    depth: Some(parse("3 mm")),
                    form: ThreadForm::Cosmetic,
                },
                ..plain
            },
        ),
    ];
    for (name, hole) in holes {
        transaction.add_feature(name, FeatureKind::Hole(hole));
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn shell_walls() -> Document {
    let (mut document, _, _) = blended_model();
    let base = named(&document, "Base");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Shell walls");
    let shells = [
        (
            "Shell outside",
            ShellSide::Outside,
            vec![FaceThickness {
                face: end_cap(0xcafe),
                thickness: parse("$0 / 2"),
            }],
        ),
        ("Shell both sides", ShellSide::BothSides, Vec::new()),
    ];
    for (name, side, faces) in shells {
        transaction.add_feature(
            name,
            FeatureKind::Shell(Shell {
                body: base,
                open: vec![end_cap(0xbeef)],
                thickness: parse("$0 / 4"),
                side,
                faces,
            }),
        );
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn edge_rules() -> Document {
    let (mut document, fillet, _) = blended_model();
    let base = named(&document, "Base");
    let FeatureKind::Blend(rounded) = document.feature(fillet).unwrap().kind.clone() else {
        panic!("a blend");
    };
    let mut transaction = document.transaction("Edge rules");
    let rules = [
        ("Convex edges", EdgeRule::Convex),
        ("Concave edges", EdgeRule::Concave),
        ("Edges of Base", EdgeRule::MadeBy(base)),
    ];
    for (name, rule) in rules {
        transaction.add_feature(
            name,
            FeatureKind::Blend(caditor_document::Blend {
                groups: vec![EdgeGroup::Rule(rule)],
                ..rounded.clone()
            }),
        );
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn faces() -> Document {
    let (mut document, fillet, _) = blended_model();
    let base = named(&document, "Base");
    let outline = named(&document, "Outline");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let FeatureKind::Blend(rounded) = document.feature(fillet).unwrap().kind.clone() else {
        panic!("a blend");
    };
    let chamfer = |form, flipped, groups| {
        FeatureKind::Blend(caditor_document::Blend {
            kind: BlendKind::Chamfer,
            form,
            flipped,
            groups,
            ..rounded.clone()
        })
    };
    let split = |along, carry| {
        FeatureKind::SplitFace(SplitFace {
            body: base,
            faces: vec![end_cap(0xbeef)],
            along,
            carry,
        })
    };
    let mut transaction = document.transaction("Faces");
    let kinds = [
        (
            "Two distances",
            chamfer(
                ChamferForm::TwoDistances {
                    second: parse("2 mm"),
                },
                true,
                Vec::new(),
            ),
        ),
        (
            "Distance and angle",
            chamfer(
                ChamferForm::DistanceAngle {
                    angle: parse("30 deg"),
                },
                false,
                Vec::new(),
            ),
        ),
        (
            "Grouped",
            chamfer(
                ChamferForm::TwoDistances {
                    second: parse("2 mm"),
                },
                false,
                vec![EdgeGroup::Face(end_cap(0xbeef)), EdgeGroup::Body],
            ),
        ),
        (
            "Shell",
            FeatureKind::Shell(Shell {
                body: base,
                open: vec![end_cap(0xbeef)],
                thickness: parse("$0 / 4"),
                side: ShellSide::Inside,
                faces: Vec::new(),
            }),
        ),
        (
            "Offset face",
            FeatureKind::OffsetFace(OffsetFace {
                body: base,
                faces: vec![end_cap(0xbeef)],
                distance: parse("$0 / 4"),
                tangent: true,
            }),
        ),
        (
            "Split face",
            split(
                SplitAlong::Plane(PlaneReference::Principal(PrincipalPlane::Xz)),
                SplitCarry::Square,
            ),
        ),
        (
            "Split face along",
            split(
                SplitAlong::Sketch(outline),
                SplitCarry::Along(Box::new(AxisReference::Principal(PrincipalAxis::X))),
            ),
        ),
        (
            "Split face wrapped",
            split(SplitAlong::Sketch(outline), SplitCarry::Wrapped),
        ),
        (
            "Split face by a surface",
            split(SplitAlong::Surface(surface_face()), SplitCarry::Square),
        ),
        (
            "Split face by a body",
            split(SplitAlong::Body(base), SplitCarry::Square),
        ),
        ("Thread", thread_kind(ThreadForm::Cosmetic)),
    ];
    for (name, kind) in kinds {
        transaction.add_feature(name, kind);
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn deleted_faces() -> Document {
    let (mut document, _, _) = blended_model();
    let base = named(&document, "Base");
    let mut transaction = document.transaction("Delete face");
    transaction.add_feature(
        "Delete face",
        FeatureKind::DeleteFace(DeleteFace {
            body: base,
            faces: vec![end_cap(0xbeef), end_cap(0xcafe)],
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn embossed_faces() -> Document {
    let (mut document, _, _) = blended_model();
    let base = named(&document, "Base");
    let sketch = document
        .features()
        .find(|feature| feature.kind.sketch().is_some())
        .map(|feature| feature.id())
        .unwrap();
    let mut transaction = document.transaction("Emboss");
    transaction.add_feature(
        "Raised text",
        FeatureKind::Emboss(Emboss {
            body: base,
            faces: vec![end_cap(0xbeef)],
            sketch: Some(sketch),
            regions: RegionChoice::All,
            depth: Expression::parse_stored("0.5 mm").unwrap(),
            draft: Emboss::undrafted(),
            form: EmbossForm::Raised,
        }),
    );
    transaction.add_feature(
        "Recessed text",
        FeatureKind::Emboss(Emboss {
            body: base,
            faces: vec![end_cap(0xcafe), end_cap(0xbeef)],
            sketch: Some(sketch),
            regions: RegionChoice::All,
            depth: Expression::parse_stored("1 mm").unwrap(),
            draft: Emboss::undrafted(),
            form: EmbossForm::Recessed,
        }),
    );
    transaction.add_feature(
        "Unfinished emboss",
        FeatureKind::Emboss(Emboss {
            body: base,
            faces: Vec::new(),
            sketch: None,
            regions: RegionChoice::All,
            depth: Expression::parse_stored("2 mm").unwrap(),
            draft: Emboss::undrafted(),
            form: EmbossForm::Raised,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn drafted_emboss() -> Document {
    let (mut document, _, _) = blended_model();
    let base = named(&document, "Base");
    let sketch = document
        .features()
        .find(|feature| feature.kind.sketch().is_some())
        .map(|feature| feature.id())
        .unwrap();
    let mut transaction = document.transaction("Emboss");
    transaction.add_feature(
        "Drafted text",
        FeatureKind::Emboss(Emboss {
            body: base,
            faces: vec![end_cap(0xbeef)],
            sketch: Some(sketch),
            regions: RegionChoice::All,
            depth: Expression::parse_stored("0.8 mm").unwrap(),
            draft: Expression::parse_stored("7 deg").unwrap(),
            form: EmbossForm::Recessed,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn embossed_regions() -> Document {
    let (mut document, _, _) = blended_model();
    let base = named(&document, "Base");
    let sketch = document
        .features()
        .find(|feature| feature.kind.sketch().is_some())
        .map(|feature| feature.id())
        .unwrap();
    let region = |digest: u128, entity: u64| {
        RegionReference::new(
            RegionKey::from_digest(digest),
            BTreeSet::from([BoundaryPiece {
                entity,
                side: Side::Right,
                piece: digest.rotate_left(32),
            }]),
            Some(Point2::new(1.5, 2.5)),
        )
    };
    let mut transaction = document.transaction("Emboss");
    transaction.add_feature(
        "Some letters",
        FeatureKind::Emboss(Emboss {
            body: base,
            faces: vec![end_cap(0xbeef)],
            sketch: Some(sketch),
            regions: RegionChoice::Chosen(vec![
                region(0x00ab_cdef_0123_4567_89ab_cdef_0123_4567, 1),
                region(0x0fed_cba9_8765_4321_0fed_cba9_8765_4321, 2),
            ]),
            depth: Expression::parse_stored("0.3 mm").unwrap(),
            draft: Emboss::undrafted(),
            form: EmbossForm::Raised,
        }),
    );
    transaction.add_feature(
        "No letters",
        FeatureKind::Emboss(Emboss {
            body: base,
            faces: vec![end_cap(0xcafe)],
            sketch: Some(sketch),
            regions: RegionChoice::Chosen(Vec::new()),
            depth: Expression::parse_stored("0.3 mm").unwrap(),
            draft: Emboss::undrafted(),
            form: EmbossForm::Recessed,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn drafts() -> Document {
    use caditor_document::{Draft, FaceAngle, FaceAttachment, PlaneReference, PrincipalPlane};
    let (mut document, _, _) = blended_model();
    let base = named(&document, "Base");
    let mut transaction = document.transaction("Draft");
    transaction.add_feature(
        "Draft",
        FeatureKind::Draft(Draft {
            body: base,
            faces: vec![end_cap(0xbeef), end_cap(0xfeed)],
            neutral: PlaneReference::Principal(PrincipalPlane::Xy),
            angle: Expression::parse_stored("3 deg").unwrap(),
            flipped: false,
            tangent: false,
            beyond: None,
            own: Vec::new(),
        }),
    );
    transaction.add_feature(
        "Draft both ways from a face",
        FeatureKind::Draft(Draft {
            body: base,
            faces: vec![end_cap(0xcafe)],
            neutral: PlaneReference::Face(FaceAttachment {
                body: base,
                face: end_cap(0xd00d),
            }),
            angle: Expression::parse_stored("2 deg").unwrap(),
            flipped: true,
            tangent: true,
            beyond: Some(Expression::parse_stored("5 deg").unwrap()),
            own: vec![FaceAngle {
                face: end_cap(0xfade),
                angle: Expression::parse_stored("6 deg").unwrap(),
            }],
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn replaced_faces() -> Document {
    use caditor_document::{FaceAttachment, PlaneReference, PrincipalPlane, ReplaceFace};
    let (mut document, _, _) = blended_model();
    let base = named(&document, "Base");
    let mut transaction = document.transaction("Replace face");
    let plane = transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: Expression::parse_stored("12 mm").unwrap(),
        })),
    );
    transaction.add_feature(
        "Replace face",
        FeatureKind::ReplaceFace(ReplaceFace {
            body: base,
            faces: vec![end_cap(0xbeef), end_cap(0xfeed)],
            target: Some(PlaneReference::Datum(plane)),
        }),
    );
    transaction.add_feature(
        "Replace face waiting for a target",
        FeatureKind::ReplaceFace(ReplaceFace {
            body: base,
            faces: vec![end_cap(0xfade)],
            target: None,
        }),
    );
    transaction.add_feature(
        "Replace face with a face",
        FeatureKind::ReplaceFace(ReplaceFace {
            body: base,
            faces: vec![end_cap(0xcafe)],
            target: Some(PlaneReference::Face(FaceAttachment {
                body: base,
                face: end_cap(0xd00d),
            })),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn moved_faces() -> Document {
    let (mut document, _, _) = blended_model();
    let base = named(&document, "Base");
    let mut transaction = document.transaction("Move face");
    transaction.add_feature(
        "Move face",
        FeatureKind::MoveFace(MoveFace {
            body: base,
            faces: vec![end_cap(0xbeef)],
            distance: Expression::parse_stored("2 mm").unwrap(),
            direction: Some(AxisReference::Principal(PrincipalAxis::X)),
            angle: Expression::parse_stored("5 deg").unwrap(),
            axis: Some(AxisReference::Edge {
                body: base,
                edge: Box::new(edge(0xd00d)),
            }),
        }),
    );
    transaction.add_feature(
        "Move face along its normal",
        FeatureKind::MoveFace(MoveFace {
            body: base,
            faces: vec![end_cap(0xcafe)],
            distance: Expression::parse_stored("-1 mm").unwrap(),
            direction: None,
            angle: Expression::parse_stored("0 deg").unwrap(),
            axis: None,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn thread_kind(form: ThreadForm) -> FeatureKind {
    match threaded_model().0.features().last().unwrap().kind.clone() {
        FeatureKind::Thread(thread) => {
            FeatureKind::Thread(caditor_document::Thread { form, ..thread })
        }
        other => other,
    }
}

fn modelled_threads() -> Document {
    let (mut document, hole) = holed_model(HoleStyle::Plain, false);
    let FeatureKind::Hole(drilled) = document.feature(hole).unwrap().kind.clone() else {
        panic!("a hole");
    };
    let tapped = Hole {
        standard: Some(HoleStandard {
            size: MetricSize::M6,
            fit: HoleFit::Tapped,
        }),
        thread: TappedThread {
            form: ThreadForm::Modelled,
            ..TappedThread::default()
        },
        ..drilled
    };
    let mut transaction = document.transaction("Modelled threads");
    transaction.edit(Edit::SetFeatureKind {
        id: hole,
        kind: FeatureKind::Hole(tapped),
    });
    transaction.add_feature("Modelled thread", thread_kind(ThreadForm::Modelled));
    document.apply(transaction.finish()).unwrap();
    document
}

fn datums() -> Document {
    let (mut document, _) = constructed_datums_model();
    let base = named(&document, "Base");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let corner = PointReference::Vertex {
        body: base,
        vertex: VertexName::from_digest(0xc0),
    };
    let centre = PointReference::Centre {
        body: base,
        edge: Box::new(edge(0xed)),
    };
    let face = || FaceReference::new(FaceName::from_digest(0xfa), None, Vec::new());
    let tangent = || {
        Box::new(FaceTangent {
            body: base,
            face: face(),
            toward: PointReference::Origin,
        })
    };
    let mut transaction = document.transaction("Datums");
    let point = transaction.add_feature(
        "Corner point",
        FeatureKind::Datum(Datum::Point(DatumPoint {
            base: corner.clone(),
            offset: [parse("$0"), parse("0 mm"), parse("-1 mm")],
        })),
    );
    let through = [
        Datum::PlaneThrough(PlaneThrough::Points([
            PointReference::Origin,
            PointReference::Datum(point),
            centre.clone(),
        ])),
        Datum::PlaneThrough(PlaneThrough::Midway(
            PlaneReference::Principal(PrincipalPlane::Xy),
            PlaneReference::Principal(PrincipalPlane::Xz),
        )),
        Datum::PlaneThrough(PlaneThrough::AxisAndPoint(
            AxisReference::Principal(PrincipalAxis::Z),
            corner.clone(),
        )),
        Datum::PlaneThrough(PlaneThrough::NormalTo(
            AxisReference::Principal(PrincipalAxis::X),
            PointReference::Sketch {
                sketch: FeatureId::from_raw(0),
                entity: EntityId::from_raw(1),
            },
        )),
        Datum::Axis(DatumAxis::Points(PointReference::Datum(point), centre)),
        Datum::Axis(DatumAxis::NormalTo(
            PlaneReference::Principal(PrincipalPlane::Yz),
            corner.clone(),
        )),
        Datum::PlaneThrough(PlaneThrough::TangentAt(tangent())),
        Datum::Axis(DatumAxis::SquareToFace(tangent())),
        Datum::PointBy(PointBy::EdgeMiddle {
            body: base,
            edge: Box::new(edge(0xed)),
        }),
        Datum::PointBy(PointBy::FaceCentre {
            body: base,
            face: face(),
        }),
        Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xz),
            rotation: Some(PlaneRotation {
                axis: AxisReference::Edge {
                    body: base,
                    edge: Box::new(edge(0xed)),
                },
                angle: parse("30 deg"),
            }),
            offset: parse("$0"),
        }),
        Datum::Axis(DatumAxis::Intersection(
            PlaneReference::Principal(PrincipalPlane::Xy),
            PlaneReference::Principal(PrincipalPlane::Yz),
        )),
    ];
    for (index, datum) in through.into_iter().enumerate() {
        transaction.add_feature(format!("Through {index}"), FeatureKind::Datum(datum));
    }
    let frame = transaction.add_feature(
        "Coordinate system 1",
        FeatureKind::Datum(Datum::Frame(Box::new(DatumFrame {
            origin: corner,
            x_axis: AxisReference::Principal(PrincipalAxis::Y),
            plane: PlaneReference::Principal(PrincipalPlane::Xz),
            reverse_x: true,
            reverse_z: false,
        }))),
    );
    let in_frame = [
        (
            "Frame axis",
            FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Frame {
                frame,
                axis: PrincipalAxis::Z,
            }))),
        ),
        (
            "Frame plane",
            FeatureKind::Datum(Datum::Plane(DatumPlane {
                base: PlaneReference::Frame {
                    frame,
                    plane: PrincipalPlane::Yz,
                },
                rotation: None,
                offset: parse("3 mm"),
            })),
        ),
        (
            "Frame point",
            FeatureKind::Datum(Datum::Point(DatumPoint {
                base: PointReference::Frame(frame),
                offset: std::array::from_fn(|_| parse("1 mm")),
            })),
        ),
        (
            "On the system",
            FeatureKind::Sketch(SketchFeature {
                sketch: Sketch::new(Plane::XY),
                attachment: Some(SketchAttachment::Frame {
                    frame,
                    plane: PrincipalPlane::Xy,
                }),
                projections: Default::default(),
            }),
        ),
        (
            "Move in frame",
            FeatureKind::Move(Move {
                body: base,
                offset: std::array::from_fn(|_| parse("2 mm")),
                turn: std::array::from_fn(|_| parse("10 deg")),
                copy: false,
                about: TurnCentre::Body,
                frame: Some(frame),
            }),
        ),
        (
            "Scale in frame",
            FeatureKind::Scale(Scale {
                body: base,
                factor: parse("2"),
                center: std::array::from_fn(|_| parse("3 mm")),
                frame: Some(frame),
            }),
        ),
    ];
    for (name, kind) in in_frame {
        transaction.add_feature(name, kind);
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn moved(body: FeatureId, copy: bool, about: TurnCentre) -> FeatureKind {
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    FeatureKind::Move(Move {
        body,
        offset: ["1 mm", "0 mm", "$0"].map(parse),
        turn: ["0 deg", "30 deg", "0 deg"].map(parse),
        copy,
        about,
        frame: None,
    })
}

fn mirrored_bodies() -> Document {
    let (mut document, base, turned) = solid_model();
    let mut transaction = document.transaction("Mirrored bodies");
    let image = transaction.add_feature(
        "Mirror as a new body",
        FeatureKind::Mirror(
            Mirror::new(base, PlaneReference::Principal(PrincipalPlane::Yz))
                .with_outcome(MirrorOutcome::NewBody),
        ),
    );
    transaction.add_feature("Move the image", moved(image, false, TurnCentre::Body));
    transaction.add_feature(
        "Mirror features as a new body",
        FeatureKind::Mirror(
            Mirror::new(base, PlaneReference::Principal(PrincipalPlane::Xz))
                .with_outcome(MirrorOutcome::NewBody)
                .mirroring(vec![turned]),
        ),
    );
    transaction.add_feature(
        "Mirror faces as a new body",
        FeatureKind::Mirror(
            Mirror::new(base, PlaneReference::Principal(PrincipalPlane::Xz))
                .with_outcome(MirrorOutcome::NewBody)
                .mirroring_faces(vec![end_cap(0xbeef)]),
        ),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn mirrored_several_bodies() -> Document {
    let (mut document, base, _) = solid_model();
    let mut transaction = document.transaction("Mirrored several bodies");
    let copy = transaction.add_feature("Copy", moved(base, true, TurnCentre::Origin));
    transaction.add_feature(
        "Mirror both as new bodies",
        FeatureKind::Mirror(
            Mirror::new(base, PlaneReference::Principal(PrincipalPlane::Yz))
                .with_more_bodies(vec![copy])
                .with_outcome(MirrorOutcome::NewBody),
        ),
    );
    transaction.add_feature(
        "Mirror both onto themselves",
        FeatureKind::Mirror(
            Mirror::new(base, PlaneReference::Principal(PrincipalPlane::Xz))
                .with_more_bodies(vec![copy]),
        ),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn bodies() -> Document {
    let (mut document, base, turned) = solid_model();
    let sketch = named(&document, "Outline");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut transaction = document.transaction("Bodies");
    let tools = ["2 mm", "3 mm"].map(|depth| {
        transaction.add_feature(
            format!("Tool {depth}"),
            FeatureKind::Solid(SolidFeature::Extrude(plain_extrude(
                sketch,
                ExtrudeExtent::one_side(parse(depth), false),
                BodyOperation::NewBody,
            ))),
        )
    });
    let mut mates = vec![
        (faces_mate("1.5 mm"), false),
        (
            MatePair::Axes(Box::new(AxisMate {
                axis: AxisReference::Principal(PrincipalAxis::X),
                target: AxisReference::Principal(PrincipalAxis::Z),
            })),
            true,
        ),
    ];
    mates.extend(
        every_new_mate()
            .into_iter()
            .map(|(pair, _, flipped)| (pair, flipped)),
    );
    let kinds = [
        ("Move", moved(base, false, TurnCentre::Origin)),
        ("Copy", moved(base, true, TurnCentre::Origin)),
        ("Turn about centre", moved(base, true, TurnCentre::Body)),
        (
            "Turn about axis",
            moved(
                base,
                false,
                TurnCentre::Axis(Box::new(AxisTurn {
                    axis: AxisReference::Principal(PrincipalAxis::Y),
                    angle: parse("45 deg"),
                })),
            ),
        ),
        (
            "Mirror",
            FeatureKind::Mirror(Mirror::new(
                base,
                PlaneReference::Principal(PrincipalPlane::Yz),
            )),
        ),
        (
            "Mirror features",
            FeatureKind::Mirror(
                Mirror::new(base, PlaneReference::Principal(PrincipalPlane::Xz))
                    .mirroring(vec![turned]),
            ),
        ),
        (
            "Mirror faces",
            FeatureKind::Mirror(
                Mirror::new(base, PlaneReference::Principal(PrincipalPlane::Xz))
                    .mirroring_faces(vec![end_cap(0xbeef), end_cap(0xcafe)]),
            ),
        ),
        (
            "Combine",
            FeatureKind::Combine(Combine::new(base, tools[0], CombineOperation::Intersect)),
        ),
        (
            "Combine tools",
            FeatureKind::Combine(Combine {
                more_tools: vec![tools[1]],
                keep_tool: true,
                ..Combine::new(base, tools[0], CombineOperation::Cut)
            }),
        ),
        (
            "Scale",
            FeatureKind::Scale(Scale {
                body: base,
                factor: parse("2"),
                center: ["$0", "-4 mm", "0 mm"].map(parse),
                frame: None,
            }),
        ),
        (
            "Split",
            FeatureKind::Split(Split {
                body: base,
                along: SplitAlong::Plane(PlaneReference::Principal(PrincipalPlane::Yz)),
                flipped: true,
            }),
        ),
        (
            "Split by a body",
            FeatureKind::Split(Split {
                body: base,
                along: SplitAlong::Body(tools[0]),
                flipped: false,
            }),
        ),
        (
            "Split by a sketch",
            FeatureKind::Split(Split {
                body: base,
                along: SplitAlong::Sketch(sketch),
                flipped: true,
            }),
        ),
        (
            "Split by a surface",
            FeatureKind::Split(Split {
                body: base,
                along: SplitAlong::Surface(surface_face()),
                flipped: true,
            }),
        ),
        ("Remove", FeatureKind::Remove(Remove { body: tools[1] })),
    ];
    for (name, kind) in kinds {
        transaction.add_feature(name, kind);
    }
    for (index, (pair, flipped)) in mates.into_iter().enumerate() {
        transaction.add_feature(
            format!("Mate {index}"),
            FeatureKind::Mate(Mate {
                body: base,
                pair,
                flipped,
            }),
        );
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn patterns() -> Document {
    let (mut document, linear, circular) = patterned_model();
    let grids = [linear, circular].map(|id| document.feature(id).unwrap().kind.clone());
    spread_and_skipping(&mut document, linear, circular);
    let base = named(&document, "Base");
    let outline = named(&document, "Outline");
    let turned = named(&document, "Turned");
    let plain = document
        .feature(linear)
        .unwrap()
        .kind
        .pattern()
        .unwrap()
        .clone();
    let mut transaction = document.transaction("Patterns");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    for (name, kind) in ["Linear grid", "Circular grid"].into_iter().zip(grids) {
        transaction.add_feature(name, kind);
    }
    let mut along = Pattern::new(
        base,
        PatternKind::Curve(CurvePattern {
            sketch: outline,
            count: parse("$0 + 1"),
            spacing: parse("12 mm"),
            measured: CurveSpacing::Distance,
            orientation: CopyOrientation::Following,
            reversed: true,
        }),
    );
    along.skipped.insert([2, 0]);
    transaction.add_feature("Curve pattern", FeatureKind::from(along));
    let mut at_points = Pattern::new(
        base,
        PatternKind::Points(PointsPattern {
            sketch: outline,
            base: PointReference::Sketch {
                sketch: outline,
                entity: EntityId::from_raw(0),
            },
        }),
    );
    at_points
        .skipped
        .insert(point_instance(EntityId::from_raw(1 << 40)).unwrap());
    transaction.add_feature("Point pattern", FeatureKind::from(at_points));
    transaction.add_feature(
        "Feature pattern",
        FeatureKind::from(plain.repeating(vec![turned])),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn patterned_bodies() -> Document {
    let (mut document, linear, circular) = patterned_model();
    let apart = [linear, circular].map(|id| {
        document
            .feature(id)
            .unwrap()
            .kind
            .pattern()
            .unwrap()
            .clone()
            .with_outcome(PatternOutcome::NewBodies)
    });
    let turned = named(&document, "Turned");
    let mut transaction = document.transaction("Patterned bodies");
    let [linear_apart, circular_apart] = apart;
    transaction.add_feature("Linear bodies", FeatureKind::from(linear_apart.clone()));
    transaction.add_feature("Circular bodies", FeatureKind::from(circular_apart));
    transaction.add_feature(
        "Feature pattern kept apart",
        FeatureKind::from(linear_apart.repeating(vec![turned])),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn imports() -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Imports");
    let shift = transaction.add_parameter("shift", transaction.parse("4 mm").unwrap());
    let frame = transaction.add_feature(
        "Coordinate system 1",
        FeatureKind::Datum(Datum::Frame(Box::new(DatumFrame {
            origin: PointReference::Origin,
            x_axis: AxisReference::Principal(PrincipalAxis::X),
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            reverse_x: false,
            reverse_z: false,
        }))),
    );
    let part = Import::new("horn.step", Solid::default(), STEP_PART);
    let mut placed = BodyPlacement::default();
    placed.offset[2] = Expression::Parameter(shift);
    placed.turn[0] = Expression::Measure(30.0, Unit::Degree);
    let placements = [
        (
            "From a file",
            part.clone().from_file(PathBuf::from("/models/horn.step")),
        ),
        ("Placed copy", part.clone().placed(placed)),
        (
            "In inches",
            part.clone().placed(BodyPlacement {
                scale: Expression::Number(25.4),
                ..Default::default()
            }),
        ),
        (
            "In a frame",
            part.placed(BodyPlacement {
                frame: Some(frame),
                ..Default::default()
            }),
        ),
        (
            "Without a path",
            Import::new("old.step", Solid::default(), STEP_PART),
        ),
    ];
    for (name, import) in placements {
        transaction.add_feature(name, FeatureKind::Import(import));
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn document_edits(document: &Document) -> Vec<Transaction> {
    let base = named(document, "Base");
    let turned = named(document, "Turned");
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut steel = steel_appearance(document);
    steel.faces = vec![
        FaceColour {
            face: FaceReference::new(FaceName::from_digest(0xface), None, []),
            colour: Rgb::new(200, 64, 52),
            opacity: Some(25),
            finish: Some(Finish::Anodized),
        },
        FaceColour {
            face: FaceReference::new(FaceName::from_digest(0xbeef), None, []),
            colour: Rgb::new(38, 150, 150),
            opacity: None,
            finish: None,
        },
    ];
    let depth = document.parameter_named("depth").unwrap().id();
    let mut owned = document.transaction("Keep a reading");
    owned.add_owned_parameter(
        "clearance",
        parse("3 mm"),
        ParameterOwner::Feature {
            feature: turned,
            value: "Angle".to_owned(),
        },
    );
    vec![
        Transaction::new(
            "Hide the origin",
            [
                PrincipalGeometry::Origin,
                PrincipalGeometry::Axis(PrincipalAxis::Z),
                PrincipalGeometry::Plane(PrincipalPlane::Xz),
            ]
            .into_iter()
            .map(|geometry| Edit::SetPrincipalShowing {
                geometry,
                showing: PrincipalShowing::Hidden,
            })
            .collect(),
        ),
        Transaction::new(
            "Describe",
            vec![
                Edit::SetModelProperties {
                    properties: Box::new(bracket_properties()),
                },
                Edit::SetSavedViews {
                    views: Box::new(bore_views()),
                },
                Edit::SetSelectionSets {
                    sets: Box::new(sets_of(base, turned)),
                },
            ],
        ),
        Transaction::new(
            "Arrange",
            vec![
                Edit::SetBodyAppearance {
                    id: base,
                    appearance: steel,
                },
                Edit::SetFeatureGroup {
                    id: turned,
                    group: Some("Turned parts".to_owned()),
                },
                Edit::SetFeatureHidden {
                    id: turned,
                    hidden: true,
                },
                Edit::SetParameterNote {
                    id: depth,
                    note: "Plate thickness, \"nominal\"".to_owned(),
                },
            ],
        ),
        owned.finish(),
        Transaction::new(
            "Suppress and roll back",
            vec![
                Edit::SetFeatureSuppressed {
                    id: turned,
                    suppressed: true,
                },
                Edit::SetRollbackBar {
                    bar: RollbackBar::Before(turned),
                },
            ],
        ),
    ]
}

fn document_with_history() -> Vec<u8> {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("document.caditor");
    let (configured, _, _) = configured_model();
    let mut edits = document_edits(&configured);
    let last = edits.pop().unwrap();
    let mut editor = Editor::new(configured);
    sharing_from(1, || {
        save(&solid_model().0, &path, false).unwrap();
        save(editor.document(), &path, false).unwrap();
        for transaction in edits {
            editor.apply(transaction).unwrap();
            save(editor.document(), &path, false).unwrap();
        }
        let oldest = crate::history(&path).unwrap().versions.len() - 1;
        set_version_kept(&path, oldest, true, None).unwrap();
        editor.apply(last).unwrap();
        save(editor.document(), &path, false).unwrap();
    });
    fs::read(&path).unwrap()
}

struct Session {
    editor: Editor,
    entries: Vec<Logged>,
}

impl Session {
    fn apply(&mut self, transaction: Transaction) {
        self.editor.apply(transaction.clone()).unwrap();
        self.entries
            .push(Logged::Entry(JournalEntry::Apply(transaction)));
    }
}

fn every_edit_journal() -> Vec<u8> {
    let base = solid_model().0;
    let outline = named(&base, "Outline");
    let solid = named(&base, "Base");
    let turned = named(&base, "Turned");
    let depth = base.parameter_named("depth").unwrap().id();
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    let mut session = Session {
        editor: Editor::new(base.clone()),
        entries: Vec::new(),
    };

    let mut inserting = session.editor.document().transaction("Insert");
    let gap = inserting.add_parameter("gap", parse("2 mm"));
    let plate = inserting.add_feature("Plate", FeatureKind::from(Sketch::new(Plane::XY)));
    session.apply(inserting.finish());

    let mut drawing = session.editor.document().transaction("Draw");
    let start = drawing.add_sketch_entity(plate, Entity::Point(Point2::ZERO));
    let end = drawing.add_sketch_entity(plate, Entity::Point(Point2::new(10.0, 0.0)));
    let line = drawing.add_sketch_entity_as(plate, Entity::Line { start, end }, true);
    let spare = drawing.add_sketch_entity(plate, Entity::Point(Point2::new(5.0, 5.0)));
    let scrap = drawing.add_sketch_entity(plate, Entity::Point(Point2::new(-5.0, 5.0)));
    let length = drawing.add_sketch_constraint(
        plate,
        Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Parameter(gap),
        },
    );
    let level = drawing.add_sketch_constraint(plate, Constraint::Horizontal(line));
    session.apply(drawing.finish());

    let FeatureKind::Solid(SolidFeature::Revolve(turning)) =
        base.feature(turned).unwrap().kind.clone()
    else {
        panic!("a revolve");
    };
    let configured = configured_model().0.configurations().clone();
    let tall = configured.rows.last().unwrap().id;
    let edits = [
        Transaction::new(
            "Edit the parameters",
            vec![
                Edit::RenameParameter {
                    id: gap,
                    name: "spacing".to_owned(),
                },
                Edit::SetParameterExpression {
                    id: gap,
                    expression: parse("$0 / 2"),
                },
                Edit::MoveParameter { id: gap, index: 0 },
                Edit::SetParameterNote {
                    id: gap,
                    note: "Between the plates".to_owned(),
                },
                Edit::SetParameterOwner {
                    id: gap,
                    owner: Some(ParameterOwner::Dimension {
                        sketch: plate,
                        constraint: length,
                    }),
                },
            ],
        ),
        Transaction::new(
            "Edit the sketch",
            vec![
                Edit::SetSketchEntity {
                    feature: plate,
                    id: spare,
                    entity: Entity::Point(Point2::new(f64::MIN_POSITIVE, 1e300)),
                },
                Edit::SetSketchConstruction {
                    feature: plate,
                    id: line,
                    construction: false,
                },
                Edit::SetDimension {
                    feature: plate,
                    constraint: length,
                    value: parse("12 mm"),
                },
                Edit::SetSketchConstraintActive {
                    feature: plate,
                    id: level,
                    active: false,
                },
                Edit::SetSketchLabel {
                    feature: plate,
                    id: length,
                    offset: Some(Vector2::new(2.0, -3.5)),
                },
                Edit::SetSketchProjection {
                    feature: plate,
                    id: spare,
                    source: Some(ProjectionSource::Vertex {
                        body: solid,
                        vertex: VertexName::from_digest(7),
                    }),
                },
                Edit::SetSketchPlacement {
                    feature: plate,
                    plane: Plane::XZ,
                    attachment: None,
                },
                Edit::RemoveSketchConstraint {
                    feature: plate,
                    id: level,
                },
                Edit::RemoveSketchEntity {
                    feature: plate,
                    id: scrap,
                },
            ],
        ),
        Transaction::new(
            "Edit the features",
            vec![
                Edit::RenameFeature {
                    id: plate,
                    name: "Base plate".to_owned(),
                },
                Edit::MoveFeature {
                    id: plate,
                    index: 2,
                },
                Edit::SetFeatureHidden {
                    id: outline,
                    hidden: true,
                },
                Edit::SetFeatureGroup {
                    id: turned,
                    group: Some("Turned parts".to_owned()),
                },
                Edit::SetFeatureKind {
                    id: turned,
                    kind: FeatureKind::Solid(SolidFeature::Revolve(Revolve {
                        extent: RevolveExtent::Full,
                        ..turning
                    })),
                },
                Edit::SetBodyAppearance {
                    id: solid,
                    appearance: BodyAppearance {
                        colour: Some(Rgb::new(70, 130, 180)),
                        ..Default::default()
                    },
                },
                Edit::SetFeatureSuppressed {
                    id: turned,
                    suppressed: true,
                },
                Edit::SetRollbackBar {
                    bar: RollbackBar::Before(turned),
                },
            ],
        ),
        Transaction::new(
            "Edit the model",
            vec![
                Edit::SetPrincipalShowing {
                    geometry: PrincipalGeometry::Origin,
                    showing: PrincipalShowing::Hidden,
                },
                Edit::SetModelProperties {
                    properties: Box::new(ModelProperties {
                        title: "Bracket".to_owned(),
                        ..Default::default()
                    }),
                },
                Edit::SetSavedViews {
                    views: Box::new(bore_views()),
                },
                Edit::SetSelectionSets {
                    sets: Box::new(sets_of(solid, turned)),
                },
                Edit::SetConfigurations {
                    configurations: Box::new(configured),
                },
                Edit::SetActiveConfiguration { active: Some(tall) },
            ],
        ),
        Transaction::new(
            "Remove",
            vec![
                Edit::RemoveFeature { id: plate },
                Edit::RemoveParameter { id: gap },
                Edit::SetParameterExpression {
                    id: depth,
                    expression: parse("4 mm"),
                },
            ],
        ),
    ];
    for transaction in edits {
        session.apply(transaction);
    }

    let Session {
        mut editor,
        mut entries,
    } = session;
    let undone = editor.next_undo().cloned().unwrap();
    editor.undo().unwrap();
    entries.push(Logged::Entry(JournalEntry::Undo(undone)));
    let redone = editor.next_redo().cloned().unwrap();
    editor.redo().unwrap();
    entries.push(Logged::Entry(JournalEntry::Redo(redone)));

    let model = save_bytes(&base, None, at(1_700_000_000), Some("Solids")).unwrap();
    let on_disk = decode(&model).unwrap().digest.unwrap();
    let head = JournalHead {
        file: Some(Path::new("/models/plate.caditor")),
        on_disk: Some(&on_disk),
        loaded_with_problems: false,
        folded: 0,
    };
    encode_journal(&head, &base, &entries).unwrap()
}

fn saved_with_preview(document: &Document, label: &str) -> Vec<u8> {
    let pixels = image::RgbaImage::from_fn(64, 48, |x, y| {
        let shade = u8::try_from((x * 3 + y * 5) % 256).unwrap();
        image::Rgba([shade, 255 - shade, 160, 255])
    });
    let mut png = io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let (slot, drawing) = PreviewSlot::new();
    drawing.fill(Preview::from_png(png.into_inner()).unwrap());
    encode_keeping(
        document,
        None,
        (at(1_700_000_000), Retention::Standard),
        EncodeExtras {
            label: Some(label),
            preview: Some(&slot),
        },
    )
    .unwrap()
    .bytes
}

fn corpus_picture(width: u32, height: u32) -> std::sync::Arc<caditor_document::Picture> {
    let pixels = image::RgbaImage::from_fn(width, height, |x, y| {
        let shade = u8::try_from((x * 31 + y * 17) % 256).unwrap();
        image::Rgba([shade, 255 - shade, 128, 255])
    });
    let mut bytes = io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    crate::read_picture(bytes.into_inner()).unwrap().picture
}

fn reference_image(
    picture: &std::sync::Arc<caditor_document::Picture>,
    plane: caditor_document::ImagePlane,
) -> caditor_document::ReferenceImage {
    let parse = |text: &str| Expression::parse_stored(text).unwrap();
    caditor_document::ReferenceImage {
        picture: std::sync::Arc::clone(picture),
        plane,
        at: [parse("12 mm"), parse("-4 mm")],
        turn: parse("15 deg"),
        flipped: false,
        size: caditor_document::ImageSize::Width(parse("120 mm")),
        opacity: caditor_document::DEFAULT_IMAGE_OPACITY_PERCENT,
        in_front: false,
        locked: false,
    }
}

fn reference_images() -> Document {
    let photo = corpus_picture(6, 4);
    let scan = corpus_picture(3, 5);
    let mut document = Document::default();
    let mut transaction = document.transaction("Reference images");
    let width = transaction.add_parameter("width", transaction.parse("80 mm").unwrap());
    let sketch = transaction.add_feature("Tracing", FeatureKind::from(Sketch::new(Plane::YZ)));
    transaction.add_feature(
        "Photo",
        FeatureKind::ReferenceImage(Box::new(reference_image(
            &photo,
            caditor_document::ImagePlane::Reference(PlaneReference::Principal(PrincipalPlane::Xz)),
        ))),
    );
    transaction.add_feature(
        "Photo again",
        FeatureKind::ReferenceImage(Box::new(caditor_document::ReferenceImage {
            flipped: true,
            in_front: true,
            locked: true,
            opacity: 20,
            ..reference_image(
                &photo,
                caditor_document::ImagePlane::Reference(PlaneReference::Principal(
                    PrincipalPlane::Xy,
                )),
            )
        })),
    );
    transaction.add_feature(
        "Scan",
        FeatureKind::ReferenceImage(Box::new(caditor_document::ReferenceImage {
            size: caditor_document::ImageSize::Calibrated(caditor_document::Calibration {
                from: caditor_document::ImageSpot::new(0.25, 0.125),
                to: caditor_document::ImageSpot::new(0.75, 0.875),
                distance: Expression::Parameter(width),
            }),
            ..reference_image(&scan, caditor_document::ImagePlane::Sketch(sketch))
        })),
    );
    document.apply(transaction.finish()).unwrap();
    document
}

fn reference_image_journal() -> Vec<u8> {
    let base = reference_images();
    let photo = named(&base, "Photo");
    let mut session = Session {
        editor: Editor::new(base.clone()),
        entries: Vec::new(),
    };
    let mut inserting = session.editor.document().transaction("Insert");
    let drawing = inserting.add_feature(
        "Drawing",
        FeatureKind::ReferenceImage(Box::new(reference_image(
            &corpus_picture(4, 4),
            caditor_document::ImagePlane::Reference(PlaneReference::Principal(PrincipalPlane::Xy)),
        ))),
    );
    session.apply(inserting.finish());
    for (feature, opacity) in [(drawing, 40), (photo, 80), (drawing, 100)] {
        let FeatureKind::ReferenceImage(image) = session
            .editor
            .document()
            .feature(feature)
            .unwrap()
            .kind
            .clone()
        else {
            panic!("a reference image");
        };
        session.apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::ReferenceImage(Box::new(caditor_document::ReferenceImage {
                    opacity,
                    ..*image
                })),
            },
        ));
    }
    let head = JournalHead {
        file: None,
        on_disk: None,
        loaded_with_problems: false,
        folded: 0,
    };
    encode_journal(&head, &base, &session.entries).unwrap()
}

fn exported_parameters() -> String {
    let mut document = Document::default();
    let mut transaction = document.transaction("Parameters");
    transaction.add_parameter("width", transaction.parse("40 mm").unwrap());
    let height = transaction.add_parameter("height", transaction.parse("width / 2").unwrap());
    let tilt = transaction.add_parameter("tilt", transaction.parse("-15 deg").unwrap());
    transaction.add_parameter("undefined", transaction.parse("width / 0").unwrap());
    transaction.edit(Edit::SetParameterNote {
        id: height,
        note: "half the width, \"roughly\"\nsecond line".to_owned(),
    });
    transaction.edit(Edit::SetParameterNote {
        id: tilt,
        note: "a, b; c".to_owned(),
    });
    document.apply(transaction.finish()).unwrap();
    parameters_csv(&document, &ParameterValues::evaluate(&document))
}
