use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use caditor_document::{Document, Evaluation};
use caditor_render::{Scene, SurfaceSize, View};

use crate::{
    bodies::{BodyMeshes, BodyMeshing},
    display::DisplayedSketches,
    display_style::DisplayStyle,
    editing::Context,
    faceting::FacetLevel,
    model::Waker,
    scene::{self, Highlight, SketchShapes, Sources},
    scene_palette::Contrast,
    selection::Selection,
    viewport::initial_viewpoint,
};

const MESHING_TIMEOUT: Duration = Duration::from_secs(120);
const MESHING_POLL: Duration = Duration::from_millis(2);
const REFERENCE_HEIGHT_POINTS: f32 = 720.0;

pub struct Snapshot {
    pub view: View,
    pub scene: Scene,
    pub pixels_per_point: f32,
}

pub fn take(document: &Document, evaluation: &Evaluation, size: SurfaceSize) -> Result<Snapshot> {
    let meshes = meshed(evaluation)?;
    let sketches = DisplayedSketches::default();
    let sources = Sources {
        document,
        evaluation,
        bodies: &meshes,
        sketches: &sketches,
        style: DisplayStyle::default(),
        contrast: Contrast::default(),
    };
    let (width, height) = (f64::from(size.width), f64::from(size.height));

    let first = build(&sources, FacetLevel::WITHOUT_A_VIEW);
    let initial = View::new(initial_viewpoint(), width, height);
    let framed = View::new(initial.fitted(first.fit_all()), width, height);
    let level = FacetLevel::fitting(FacetLevel::wanted_chord(&framed));
    let built = build(&sources, level);

    Ok(Snapshot {
        view: framed.reaching(built.everything),
        scene: built.scene,
        pixels_per_point: size.height as f32 / REFERENCE_HEIGHT_POINTS,
    })
}

fn build(sources: &Sources<'_>, level: FacetLevel) -> scene::BuiltScene {
    let context = Context::default();
    let unselected = Selection::default();
    let mut built = scene::build(
        sources,
        &Highlight {
            selection: &unselected,
            hovered: &[],
        },
        context,
        &mut SketchShapes::new(scene::drawn_faceting(sources, context, level.faceting())),
    );
    built.scene.grid = None;
    built
}

fn meshed(evaluation: &Evaluation) -> Result<BodyMeshes> {
    let mut meshing = BodyMeshing::default();
    for (body, _) in evaluation.bodies() {
        if let Some(result) = evaluation.body_result(body) {
            meshing.request(result, no_wake);
        }
    }
    let deadline = Instant::now() + MESHING_TIMEOUT;
    while meshing.is_pending() {
        if Instant::now() >= deadline {
            bail!(
                "the bodies were not meshed within {} seconds",
                MESHING_TIMEOUT.as_secs()
            );
        }
        meshing.poll();
        thread::sleep(MESHING_POLL);
    }
    let mut meshes = BodyMeshes::default();
    meshes.update(evaluation, &meshing);
    Ok(meshes)
}

fn no_wake() -> Waker {
    Box::new(|| {})
}
