use std::{
    collections::BTreeSet,
    io::{self, Write as _},
    path::Component,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use caditor_document::{
    AxisMate, AxisReference, AxisSide, AxisTurn, BlendKind, BodyAppearance, BodyOperation,
    BodyPlacement, ChamferForm, Combine, CombineOperation, CopyOrientation, CurvePattern,
    CurveSpacing, Datum, DatumAxis, DatumFrame, DatumPlane, DatumPoint, EdgeGroup, Extrude,
    ExtrudeEnd, ExtrudeExtent, FaceColour, FaceTangent, FinePitch, Finish, Hole, HoleBottom,
    HoleDepth, HoleFit, HoleShape, HoleSizing, HoleStandard, HoleStep, HoleStyle, Import, Mate,
    MatePair, MetricSize, Mirror, ModelProperties, Move, OffsetFace, Outline, ParameterOwner,
    ParameterValues, Pattern, PatternKind, PlaneRotation, PlaneThrough, PointBy, PointReference,
    PointsPattern, PrincipalAxis, PrincipalGeometry, ProjectionSource, RegionChoice, Remove,
    Revolve, RevolveAxis, RevolveExtent, Rgb, Scale, Shell, SketchFeature, SolidFeature,
    SolidStart, Split, SplitAlong, SplitCarry, SplitFace, Sweep, SweepPath, TappedThread,
    ThreadFamily, ThreadForm, ThreadHand, TurnCentre, Wall, point_instance,
};
use caditor_kernel::{
    EdgeName, EdgeReference, FaceName, FaceOrigin, FaceReference, Solid, SweepOrientation,
    VertexName, WallSide,
};
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
        ChunkKind, Codec, JOURNAL_MAGIC, MODEL_MAGIC, save_bytes,
        testing::{records_as_json, sharing_from, with_slices_of},
    },
    format::{
        ConstraintKindRecord, EditRecord, EntityKindRecord, FEATURE_FIELDS, FORMAT_VERSION,
        FeatureKindRecord, Record, transaction_record,
    },
    journal::{JOURNAL_VERSION, JournalHead, Logged, decode_journal, encode_journal, replay},
    parameters::PARAMETERS_VERSION,
};

const WRITE_CORPUS: &str = "CADITOR_WRITE_FORMAT_CORPUS";
const ENTITY_FLAGS: [&str; 2] = ["construction", "split_from"];
const SLICE: usize = 512;
const STEP_PART: &str = include_str!("../../../../fuzz/seeds/step/horn_torus.step");

const KEPT: [(&str, &str); 22] = [
    (
        "journal-1/by-reference.journal",
        "fe98725da8d9df46b19d7cede1474bcfb92bbda00d4de9c890e5bca5d0f2899f",
    ),
    (
        "journal-1/every-edit.journal",
        "d5f3ffe41fdaf1a24552095ee7a8335337340fd0f9da665a39f0930f793cba29",
    ),
    (
        "journal-1/rebased.journal",
        "29586076a032208d8c3f3630c649dbca0afb09ef7023c4b25c24d93860c6c741",
    ),
    (
        "journal-1/sliced.journal",
        "d5f3ffe41fdaf1a24552095ee7a8335337340fd0f9da665a39f0930f793cba29",
    ),
    (
        "model-1/bodies.caditor",
        "8e65cdd34444ca11d831fa7ae40b30d81c3de5e3386608a59b33fbe6e7a7f9c9",
    ),
    (
        "model-1/datums.caditor",
        "eccdc03a0f9999a951b010f295fb893e67f3b7d97174d58770ddfb15cf06d54e",
    ),
    (
        "model-1/document.caditor",
        "d6f719b47c939f742905bc3c7764a6b115222984bdb00759edacc6872588d630",
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
        "model-1/holes.caditor",
        "2cf9f186f10bbb89e64b84564f0152e30129f9ba6b6dadf846310dcc401d8e27",
    ),
    (
        "model-1/imports.caditor",
        "f172dff8d721a9e2b87cec9c788a954cebb79dd198f0335636da641731addfbb",
    ),
    (
        "model-1/measured.caditor",
        "0e800cdb471610bba2498c6d12c71a93f2e0f90f0a59e967c085aa0ea91efef5",
    ),
    (
        "model-1/modelled-threads.caditor",
        "f5ea168f759dd3556843b0360ba687d67c7f36d9dd6ff0a93bd5f4eecaeb8957",
    ),
    (
        "model-1/path-sweeps.caditor",
        "b28dd7ffcc7c11d879e21bd9325de195e284df80b8f3d914ea2f54ed0466ed61",
    ),
    (
        "model-1/patterns.caditor",
        "ec1a7a437ffe10cba80a68813ae78ff8513ee236945d2ded0811d26847b7f147",
    ),
    (
        "model-1/pieces.caditor",
        "f4745712d55b915884d644d687473b879d96e9ba8f066938a5358ba1a16023b2",
    ),
    (
        "model-1/primitives.caditor",
        "a951119ab531bb07364933525a7861e66093225c496ef672a87f2cc03e4917f3",
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
        "model-1/starts.caditor",
        "638005d3194997ab315150f533e17db8422551613a547f3e41a3c40e73770f26",
    ),
    (
        "model-1/sweeps.caditor",
        "89008f408c1da375ec6d456157b46c58f8a088794926c1ae8fc334d4b4466519",
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
        (model("extents"), saved(&extents_model().0, "Extents")),
        (model("starts"), saved(&starts_model().0, "Starts")),
        (model("sweeps"), saved(&sweeps(), "Sweeps")),
        (model("path-sweeps"), saved(&path_sweeps(), "Path sweeps")),
        (model("holes"), saved(&holes(), "Holes")),
        (
            model("modelled-threads"),
            saved(&modelled_threads(), "Modelled threads"),
        ),
        (model("faces"), saved(&faces(), "Faces")),
        (model("datums"), saved(&datums(), "Datums")),
        (model("bodies"), saved(&bodies(), "Bodies")),
        (model("patterns"), saved(&patterns(), "Patterns")),
        (
            model("primitives"),
            saved(&primitive_model().0, "Primitives"),
        ),
        (model("measured"), saved(&measured_model().0, "Measure")),
        (model("imports"), saved(&imports(), "Imports")),
        (model("document"), document_with_history()),
        (
            model("sliced"),
            with_slices_of(SLICE, || saved(&sketches(), "Sliced")),
        ),
        (journal("every-edit"), every_edit_journal()),
        (journal("by-reference"), seed_journal_by_reference(0)),
        (journal("rebased"), seed_journal_by_reference(12)),
        (journal("sliced"), with_slices_of(SLICE, every_edit_journal)),
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
            .map(|geometry| Edit::SetPrincipalHidden {
                geometry,
                hidden: true,
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
                Edit::SetPrincipalHidden {
                    geometry: PrincipalGeometry::Origin,
                    hidden: true,
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
