use std::time::SystemTime;

use caditor_document::Import;
use caditor_kernel::{LinearExtent, Profile, ProfileCurve, Selection, extrude};
use caditor_step::{StepBody, write_step};

use super::*;
use crate::journal::{JournalHead, Logged, Mirror, encode_entry, encode_journal};

const PRISM_SIDES: usize = 6000;
const SKETCHES: usize = 2000;
const RUNS: usize = 5;
const DRAWN_LINES: usize = 2000;

fn prism_text(sides: usize) -> String {
    let corner = |index: usize| {
        let angle = std::f64::consts::TAU * index as f64 / sides as f64;
        Point2::new(50.0 * angle.cos(), 50.0 * angle.sin())
    };
    let curves: Vec<ProfileCurve> = (0..sides)
        .map(|index| ProfileCurve::line(index as u64 + 1, corner(index), corner(index + 1)))
        .collect();
    let regions = Profile::new(&curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let solid = extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(20.0).unwrap(),
        1,
    )
    .unwrap();
    write_step(
        &[StepBody {
            name: "Prism",
            solid: &solid,
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
        }],
        "Prism",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap()
}

fn import_transaction(document: &Document, text: &str) -> Transaction {
    let solid = crate::step_cache::first_solid(text).unwrap().unwrap();
    let mut transaction = document.transaction("Import");
    transaction.add_feature(
        "Prism",
        FeatureKind::Import(Import::shared("prism.step", solid, text)),
    );
    transaction.finish()
}

fn imported(text: &str) -> Document {
    let mut document = sample();
    document.apply(import_transaction(&document, text)).unwrap();
    document
}

fn many_sketches() -> Document {
    let mut document = sample();
    let width = document.parameter_named("width").unwrap().id();
    let mut transaction = document.transaction("Sketches");
    for index in 0..SKETCHES {
        transaction.add_feature(
            format!("Sketch {index}"),
            FeatureKind::from(dimensioned_line(
                Plane::XY,
                40.0,
                Expression::Parameter(width),
            )),
        );
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn drawing_transaction(document: &Document) -> Transaction {
    let mut sketch = Sketch::new(Plane::XY);
    for index in 0..DRAWN_LINES {
        let y = index as f64;
        sketch.add_line(Point2::new(0.0, y), Point2::new(10.0, y));
    }
    let mut transaction = document.transaction("Drawing");
    transaction.add_feature("Drawing", FeatureKind::from(sketch));
    transaction.finish()
}

fn edited(document: &Document, text: &str) -> Document {
    let mut edited = document.clone();
    edited.apply(edit_width(document, text)).unwrap();
    edited
}

fn best_of(mut run: impl FnMut()) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let started = Instant::now();
        run();
        best = best.min(started.elapsed());
    }
    best
}

fn report_costs(name: &str, document: &Document, appended: &Transaction) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.caditor");
    let versions = [edited(document, "41 mm"), edited(document, "42 mm")];
    let journal_head = JournalHead {
        file: Some(&path),
        on_disk: None,
        loaded_with_problems: false,
        folded: 0,
    };

    let fresh = best_of(|| {
        let _ = fs::remove_file(&path);
        save(document, &path, false).unwrap();
    });
    let mut turn = 0;
    let over = best_of(|| {
        turn += 1;
        save(&versions[turn % 2], &path, false).unwrap();
    });
    let rewrite = best_of(|| {
        encode_journal(&journal_head, document, &[]).unwrap();
    });
    let entry = Logged::Entry(JournalEntry::Apply(appended.clone()));
    let append = best_of(|| {
        encode_entry(&entry).unwrap();
    });
    let mut unfollowed: Vec<(Mirror, JournalEntry)> = (0..RUNS)
        .map(|_| {
            (
                Mirror::new(document.clone()),
                JournalEntry::Apply(appended.clone()),
            )
        })
        .collect();
    let mut followed = Vec::with_capacity(RUNS);
    let follow = best_of(|| {
        let (mut mirror, entry) = unfollowed.pop().unwrap();
        let logged = mirror.log(entry);
        followed.push((mirror, logged));
    });
    assert!(
        followed
            .iter()
            .all(|(_, logged)| matches!(logged, Logged::Entry(_)))
    );
    let load_time = best_of(|| {
        load(&path).unwrap();
    });
    let file = fs::metadata(&path).unwrap().len();
    let journal = encode_journal(&journal_head, document, &[]).unwrap().len();
    let appended_bytes = encode_entry(&entry).unwrap().len();

    println!(
        "{name}: save new {fresh:.1?}, save over {over:.1?} ({file} bytes with versions), \
         journal rewrite {rewrite:.1?} ({journal} bytes), journal append {append:.1?} \
         ({appended_bytes} bytes), following it {follow:.1?}, load {load_time:.1?}"
    );
}

#[test]
#[ignore = "a timing benchmark: cargo test --release -p caditor-file save_and_journal_costs -- --ignored --nocapture"]
fn save_and_journal_costs() {
    let text = prism_text(PRISM_SIDES);
    let import = imported(&text);
    let sketches = many_sketches();
    println!(
        "STEP text of {} bytes, read as a solid of about {} bytes",
        text.len(),
        crate::step_cache::first_solid(&text)
            .unwrap()
            .unwrap()
            .approximate_size()
    );

    report_costs("an import", &import, &import_transaction(&sample(), &text));
    report_costs(
        &format!("{SKETCHES} sketches"),
        &sketches,
        &drawing_transaction(&sketches),
    );
}
