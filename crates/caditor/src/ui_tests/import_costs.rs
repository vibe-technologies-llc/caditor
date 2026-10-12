use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use caditor_document::{
    CancelToken, Document, Evaluation, FeatureResult, ModelEvaluator, Recompute,
};
use caditor_file::{
    ExportBody, ExportFormat, MeshOptions, bodies_transaction, export_bodies, read_step_file,
};
use caditor_geometry::{RigidTransform, Vector3};
use caditor_kernel::Solid;
use caditor_render::{Background, GraphicsSettings, ImageRequest, OffscreenRenderer, SurfaceSize};
use egui::{Event, Modifiers, PointerButton, Pos2};
use tempfile::TempDir;

use super::{CAMERA_SETTLE, Harness};
use crate::{
    app::{self, Workspace},
    bodies::BodyMeshing,
    bodies_tree,
    files::FileCommand,
    model::Action,
    samples::Sample,
    scene_palette::Canvas,
    selection::Pickable,
    snapshot, visibility,
};

const IMPORT_FILE: &str = "CADITOR_IMPORT_FILE";
const IN_THE_APP_ONLY: &str = "CADITOR_IMPORT_IN_THE_APP_ONLY";
const COPIES_PER_SAMPLE: usize = 100;
const SPACING: f64 = 150.0;
const ROW: usize = 20;
const IMPORT_TIMEOUT: Duration = Duration::from_secs(3600);
const SYNTHESISED_WITHIN: Duration = Duration::from_secs(30);
const STALL: Duration = Duration::from_millis(50);
const UPLOAD_BYTES_PER_FRAME: f64 = 8.0 * 1024.0 * 1024.0;
const VERTEX_BYTES: usize = 28;
const TRIANGLE_BYTES: usize = 12;
const LOADED_FRAMES: u32 = 40;
const SETTLING_FRAMES: u32 = 3;

fn within_timeout(started: Instant) -> bool {
    Instant::now() < started + IMPORT_TIMEOUT
}

fn report(stage: &str, started: Instant, detail: impl std::fmt::Display) {
    eprintln!(
        "{stage:<44} {:>10.3} s  {detail}",
        started.elapsed().as_secs_f64()
    );
}

fn synthesised(dir: &Path) -> PathBuf {
    let mut solids: Vec<(String, Solid)> = Vec::new();
    for (row, sample) in Sample::ALL.into_iter().enumerate() {
        let document = sample.document().unwrap();
        let evaluation = Recompute::default().run_without_display(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
        );
        let (body, _) = evaluation.bodies().next().unwrap();
        let solid = &evaluation.body_result(body).unwrap().solid().unwrap().solid;
        for copy in 0..COPIES_PER_SAMPLE {
            let along = (copy % ROW) as f64 * SPACING;
            let across = (row * COPIES_PER_SAMPLE / ROW + copy / ROW) as f64 * SPACING;
            let placed = solid
                .transformed(
                    &RigidTransform::translation(Vector3::new(along, across, 0.0)).unwrap(),
                )
                .unwrap();
            solids.push((format!("{} {copy}", sample.title()), placed));
        }
    }
    let bodies: Vec<ExportBody<'_>> = solids
        .iter()
        .map(|(name, solid)| ExportBody {
            name,
            solid,
            look: None,
            group: None,
            threads: &[],
            faces: &[],
        })
        .collect();
    let path = dir.join("assembly.step");
    export_bodies(
        &path,
        ExportFormat::Step,
        &MeshOptions::default(),
        &bodies,
        &caditor_document::ModelProperties::default(),
        &CancelToken::never(),
    )
    .unwrap();
    path
}

fn body_results(evaluation: &Evaluation) -> Vec<Arc<FeatureResult>> {
    evaluation
        .bodies()
        .filter_map(|(body, _)| evaluation.body_result(body))
        .cloned()
        .collect()
}

fn stages(path: &Path) {
    let cancel = CancelToken::never();
    let started = Instant::now();
    let imported = read_step_file(path, &cancel).unwrap();
    report(
        "read, healed and stored",
        started,
        format!("{} bodies", imported.bodies.len()),
    );

    let started = Instant::now();
    let mut document = Document::default();
    let transaction = bodies_transaction(&document, &imported.bodies, "Import");
    document.apply(transaction).unwrap();
    report("added to the model", started, "");
    drop(imported);

    let mut recompute = Recompute::default();
    let started = Instant::now();
    recompute.run_without_display(&document, &ModelEvaluator, &cancel);
    report("recomputed without display", started, "");

    let started = Instant::now();
    let evaluation = recompute.run(&document, &ModelEvaluator, &cancel, &|_, _| {});
    let results = body_results(&evaluation);
    let (mut vertices, mut triangles) = (0, 0);
    for result in &results {
        if let Some(mesh) = result.solid().and_then(|solid| solid.mesh()) {
            vertices += mesh.vertices().len();
            triangles += mesh.triangles().len();
        }
    }
    report(
        "tessellated",
        started,
        format!("{} bodies, {triangles} triangles", results.len()),
    );

    let started = Instant::now();
    let mut meshing = BodyMeshing::default();
    for result in &results {
        meshing.request(result, || Box::new(|| {}));
    }
    while meshing.is_pending() {
        assert!(within_timeout(started), "the import did not finish");
        meshing.poll();
        thread::sleep(Duration::from_millis(2));
    }
    report("converted for display (BodyMesh::build)", started, "");

    let started = Instant::now();
    let mut meshes = crate::bodies::BodyMeshes::default();
    meshes.update(&evaluation, &meshing);
    let shown = meshes.iter().count();
    report(
        "handed to the view (BodyMeshes::update)",
        started,
        format!("{shown} meshes"),
    );

    let started = Instant::now();
    let asked = meshes
        .iter()
        .filter(|(_, mesh)| mesh.mass().is_none())
        .count();
    while meshing.masses_measured() < asked as u64 {
        assert!(within_timeout(started), "the import did not finish");
        meshing.poll();
        thread::sleep(Duration::from_millis(2));
    }
    report("exact mass properties, asked of every body", started, "");

    let size = SurfaceSize {
        width: 1920,
        height: 1080,
    };
    let started = Instant::now();
    let snapshot = snapshot::of_bodies(&document, &evaluation, &meshes, size, Canvas::default());
    report(
        "scene built twice (fitting, then drawn)",
        started,
        format!("{} lines", snapshot.scene.lines().count()),
    );

    let bytes = vertices * VERTEX_BYTES + triangles * TRIANGLE_BYTES;
    eprintln!(
        "about {:.0} MiB of meshes, {:.0} frames at 8 MiB a frame",
        bytes as f64 / 1024.0 / 1024.0,
        (bytes as f64 / UPLOAD_BYTES_PER_FRAME).ceil()
    );
    let Ok(mut renderer) = OffscreenRenderer::new(GraphicsSettings::default()) else {
        eprintln!("no graphics adapter, so the upload is not timed");
        return;
    };
    let started = Instant::now();
    let drawn = renderer.render(&ImageRequest {
        size,
        view: &snapshot.view,
        scene: &snapshot.scene,
        pixels_per_point: 1.0,
        background: Background::Viewport,
    });
    report(
        "uploaded and drawn at once (offscreen)",
        started,
        if drawn.is_ok() { "" } else { "failed" },
    );
}

fn in_app(path: &Path) {
    let mut harness = Harness::starting(None, Document::default(), Workspace::new());
    harness.answer_dialog(Some(path.to_path_buf()));
    let started = Instant::now();
    harness.command(FileCommand::Import { into: None });
    let mut added = None;
    let mut first_shown = None;
    let mut slow: Vec<(Duration, Duration, usize)> = Vec::new();
    let mut frames = 0;
    let mut previous_shown = 0;
    let mut steps = 0;
    loop {
        assert!(within_timeout(started), "the import did not finish");
        let frame = Instant::now();
        harness.pass();
        let took = frame.elapsed();
        frames += 1;
        let features = harness.document().features().len();
        if added.is_none() && features > 0 {
            added = Some(started.elapsed());
            report(
                "in the app: bodies added to the model",
                started,
                format!("{features} features"),
            );
        }
        let shown = harness.workspace.viewport.bodies().iter().count();
        if took > STALL {
            slow.push((started.elapsed(), took, shown));
        }
        if shown > 0 && first_shown.is_none() {
            first_shown = Some(started.elapsed());
            report(
                "in the app: first body shown",
                started,
                format!("{shown} shown"),
            );
        }
        if shown != previous_shown {
            steps += 1;
            previous_shown = shown;
        }
        if added.is_some() && !harness.computing() && shown > 0 {
            report(
                "in the app: every body shown",
                started,
                format!("{shown} shown in {steps} steps"),
            );
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    eprintln!("{frames} frames, {} longer than {STALL:?}:", slow.len());
    for (at, took, shown) in slow {
        eprintln!(
            "  at {:>8.3} s a frame of {took:?}, {shown} bodies shown",
            at.as_secs_f64()
        );
    }
    after_loading(&mut harness);
}

fn after_loading(harness: &mut Harness) {
    harness.settle();
    harness.command(FileCommand::DismissReport);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.pass();
    let centre = super::SCREEN.center();
    let aside = centre + egui::vec2(40.0, 25.0);

    timed_frames(harness, "loaded, idle", |_, _| {});
    timed_frames(
        harness,
        "loaded, pointer moving over the view",
        |harness, frame| {
            let position = if frame % 2 == 0 { centre } else { aside };
            harness.events.push(Event::PointerMoved(position));
        },
    );
    let faces = two_faces(harness);
    let before_hovering = harness.built().generation;
    timed_frames(
        harness,
        "loaded, pointer moving between two faces",
        |harness, frame| {
            let (position, face) = if frame % 2 == 0 {
                (centre, faces[0])
            } else {
                (aside, faces[1])
            };
            harness.events.push(Event::PointerMoved(position));
            harness.forced_hover = Some((position, face));
        },
    );
    assert!(
        harness.built().generation > before_hovering,
        "hovering the faces lit them"
    );

    harness.events.push(Event::PointerMoved(centre));
    harness.forced_hover = Some((centre, faces[0]));
    harness.pass();
    timed_change(harness, "loaded, a face clicked to select it", |harness| {
        press_and_release(harness, centre, PointerButton::Primary);
    });
    assert!(
        harness.workspace.viewport.selection().contains(faces[0]),
        "the click selected the face under the pointer"
    );
    timed_frames(harness, "loaded, a face selected, idle", |_, _| {});
    harness.forced_hover = None;
    timed_change(harness, "loaded, the selection cleared", |harness| {
        harness.workspace.viewport.selection_mut().clear();
    });

    dragged(
        harness,
        "loaded, orbiting",
        PointerButton::Secondary,
        centre,
    );
    dragged(harness, "loaded, panning", PointerButton::Middle, centre);
    timed_frames(harness, "loaded, zooming", |harness, frame| {
        let delta = if frame % 2 == 0 { 60.0 } else { -60.0 };
        harness.events.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
    });
    harness.settle();

    in_the_trees(harness);
}

fn two_faces(harness: &mut Harness) -> [Pickable; 2] {
    let picks = harness.built().picks;
    let mut faces = picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::Face { .. }));
    let first = faces.next().expect("a face is drawn");
    let second = faces
        .find(|face| visibility::owner(*face) != visibility::owner(first))
        .unwrap_or(first);
    [first, second]
}

fn press_and_release(harness: &mut Harness, position: Pos2, button: PointerButton) {
    for pressed in [true, false] {
        harness.events.push(Event::PointerButton {
            pos: position,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.pass();
    }
}

fn dragged(harness: &mut Harness, name: &str, button: PointerButton, centre: Pos2) {
    harness.events.push(Event::PointerMoved(centre));
    harness.pass();
    harness.events.push(Event::PointerButton {
        pos: centre,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.pass();
    timed_frames(harness, name, |harness, frame| {
        let along = (frame % 40) as f32 * 3.0;
        harness
            .events
            .push(Event::PointerMoved(centre + egui::vec2(along, along / 3.0)));
    });
    harness.events.push(Event::PointerButton {
        pos: centre,
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.pass();
}

fn in_the_trees(harness: &mut Harness) {
    let first = harness
        .document()
        .features()
        .next()
        .map(|feature| (feature.id(), feature.name.clone()))
        .expect("the import added features");
    let hideable = visibility::toggle(harness.document().feature(first.0).unwrap()).unwrap();

    harness.frame();
    let row = harness.position_of(&first.1);
    let over_the_tree = row + egui::vec2(0.0, 200.0);
    timed_change(harness, "loaded, a feature row clicked", |harness| {
        harness.events.push(Event::PointerMoved(row));
        harness.pass();
        press_and_release(harness, row, PointerButton::Primary);
    });
    timed_frames(harness, "loaded, a feature row chosen, idle", |_, _| {});
    timed_frames(
        harness,
        "loaded, the feature tree scrolled",
        |harness, frame| {
            scroll(harness, over_the_tree, frame);
        },
    );
    timed_change(harness, "loaded, a body hidden", |harness| {
        performed(harness, Action::Apply(hideable));
    });
    timed_change(harness, "loaded, the hiding undone", |harness| {
        performed(harness, Action::Undo);
    });

    let title = format!(
        "{} ({})",
        bodies_tree::GROUP_TITLE,
        harness.workspace.viewport.bodies().iter().count()
    );
    if !harness.shows(&title) {
        eprintln!("the Bodies group is not on screen, so its rows are not timed");
        return;
    }
    let title_at = harness.position_of(&title);
    timed_change(harness, "loaded, the Bodies group opened", |harness| {
        harness.events.push(Event::PointerMoved(title_at));
        harness.pass();
        press_and_release(harness, title_at, PointerButton::Primary);
    });
    timed_frames(harness, "loaded, Bodies open, idle", |_, _| {});
    timed_frames(
        harness,
        "loaded, Bodies open, scrolled",
        |harness, frame| {
            scroll(harness, over_the_tree, frame);
        },
    );
}

fn performed(harness: &mut Harness, action: Action) {
    app::perform(
        vec![action],
        &mut harness.model,
        &mut harness.files,
        &mut harness.workspace,
    );
}

fn scroll(harness: &mut Harness, over: Pos2, frame: u32) {
    let delta = if frame < LOADED_FRAMES / 2 {
        -60.0
    } else {
        60.0
    };
    harness.events.push(Event::PointerMoved(over));
    harness.events.push(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, delta),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
}

fn timed_change(harness: &mut Harness, name: &str, change: impl FnOnce(&mut Harness)) {
    let started = Instant::now();
    change(harness);
    let changed = started.elapsed();
    let mut frames = 0;
    while harness.computing() || frames < SETTLING_FRAMES {
        assert!(within_timeout(started), "{name} never settled");
        if harness.computing() {
            harness.model.poll();
            thread::sleep(Duration::from_millis(1));
        }
        harness.pass();
        frames += 1;
    }
    eprintln!(
        "{name:<44} {changed:?} for the change, {:?} until settled over {frames} more frames",
        started.elapsed()
    );
}

fn timed_frames(harness: &mut Harness, name: &str, mut change: impl FnMut(&mut Harness, u32)) {
    let mut took: Vec<Duration> = (0..LOADED_FRAMES)
        .map(|frame| {
            change(harness, frame);
            let started = Instant::now();
            harness.pass();
            started.elapsed()
        })
        .collect();
    took.sort();
    let total: Duration = took.iter().sum();
    eprintln!(
        "{name:<44} {:?} a frame on average, median {:?}, longest {:?}",
        total / LOADED_FRAMES,
        took.get(took.len() / 2).copied().unwrap_or_default(),
        took.last().copied().unwrap_or_default(),
    );
}

#[test]
#[ignore = "a timing benchmark: cargo test --release -p caditor import_costs -- --ignored --nocapture"]
fn a_large_step_import_is_timed_stage_by_stage() {
    let dir = TempDir::new().unwrap();
    let (path, synthesised_here) = match env::var_os(IMPORT_FILE) {
        Some(path) => (PathBuf::from(path), false),
        None => {
            let started = Instant::now();
            let path = synthesised(dir.path());
            report("synthesised", started, "");
            (path, true)
        }
    };
    let started = Instant::now();
    if env::var_os(IN_THE_APP_ONLY).is_none() {
        stages(&path);
    }
    in_app(&path);
    let took = started.elapsed();

    assert!(
        !synthesised_here || took < SYNTHESISED_WITHIN,
        "the synthesised import took {took:?}"
    );
}
