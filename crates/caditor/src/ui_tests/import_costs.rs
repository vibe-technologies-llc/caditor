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
use tempfile::TempDir;

use super::Harness;
use crate::{app::Workspace, bodies::BodyMeshing, files::FileCommand, samples::Sample, snapshot};

const IMPORT_FILE: &str = "CADITOR_IMPORT_FILE";
const COPIES_PER_SAMPLE: usize = 100;
const SPACING: f64 = 150.0;
const ROW: usize = 20;
const IMPORT_TIMEOUT: Duration = Duration::from_secs(3600);
const SYNTHESISED_WITHIN: Duration = Duration::from_secs(30);
const STALL: Duration = Duration::from_millis(50);
const UPLOAD_BYTES_PER_FRAME: f64 = 8.0 * 1024.0 * 1024.0;
const VERTEX_BYTES: usize = 28;
const TRIANGLE_BYTES: usize = 12;

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
        assert!(started.elapsed() < IMPORT_TIMEOUT);
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
        assert!(started.elapsed() < IMPORT_TIMEOUT);
        meshing.poll();
        thread::sleep(Duration::from_millis(2));
    }
    report("exact mass properties, asked of every body", started, "");

    let size = SurfaceSize {
        width: 1920,
        height: 1080,
    };
    let started = Instant::now();
    let snapshot = snapshot::of_bodies(&document, &evaluation, &meshes, size);
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
        assert!(
            started.elapsed() < IMPORT_TIMEOUT,
            "the import did not finish"
        );
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
    stages(&path);
    in_app(&path);
    let took = started.elapsed();

    assert!(
        !synthesised_here || took < SYNTHESISED_WITHIN,
        "the synthesised import took {took:?}"
    );
}
