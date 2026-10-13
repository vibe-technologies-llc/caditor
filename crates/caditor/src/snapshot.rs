use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use caditor_document::{CancelToken, Document, Evaluation};
use caditor_render::{Scene, SurfaceSize, View};

use crate::{
    analysis::Analyses,
    bodies::{BodyMeshes, BodyMeshing},
    display::DisplayedSketches,
    display_style::DisplayStyle,
    editing::Context,
    faceting::FacetLevel,
    image_textures::ImageTextures,
    model::Waker,
    saved_views,
    scene::{self, Highlight, SketchShapes, Sources},
    scene_palette::{Canvas, Contrast},
    selection::Selection,
    view_aids::ViewAids,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unmeshed {
    #[error("the bodies were not meshed within {} seconds", MESHING_TIMEOUT.as_secs())]
    TimedOut,
    #[error("meshing the bodies was cancelled")]
    Cancelled,
}

pub fn take(document: &Document, evaluation: &Evaluation, size: SurfaceSize) -> Result<Snapshot> {
    let pictures = ImageTextures::decoded_for(document);
    let meshes = meshed(evaluation, &CancelToken::never())?;
    Ok(of_bodies(
        document,
        evaluation,
        &meshes,
        size,
        Canvas::default(),
        Some(&pictures),
    ))
}

pub fn take_unless(
    document: &Document,
    evaluation: &Evaluation,
    size: SurfaceSize,
    canvas: Canvas,
    pictures: &ImageTextures,
    cancel: &CancelToken,
) -> Result<Snapshot, Unmeshed> {
    let meshes = meshed(evaluation, cancel)?;
    Ok(of_bodies(
        document,
        evaluation,
        &meshes,
        size,
        canvas,
        Some(pictures),
    ))
}

pub fn of_bodies(
    document: &Document,
    evaluation: &Evaluation,
    meshes: &BodyMeshes,
    size: SurfaceSize,
    canvas: Canvas,
    pictures: Option<&ImageTextures>,
) -> Snapshot {
    let sketches = DisplayedSketches::default();
    let sources = Sources {
        document,
        evaluation,
        bodies: meshes,
        sketches: &sketches,
        style: DisplayStyle::default(),
        aids: ViewAids::default(),
        analyses: &Analyses::default(),
        contrast: Contrast::default(),
        canvas,
        draft: None,
        pictures,
    };
    let (width, height) = (f64::from(size.width), f64::from(size.height));

    let first = build(&sources, FacetLevel::WITHOUT_A_VIEW);
    let starting = document
        .saved_views()
        .home
        .map_or_else(initial_viewpoint, saved_views::viewpoint);
    let initial = View::new(starting, width, height);
    let framed = View::new(initial.fitted(first.fit_all()), width, height);
    let level = FacetLevel::fitting(FacetLevel::wanted_chord(&framed));
    let built = build(&sources, level);

    Snapshot {
        view: framed.reaching(built.everything),
        scene: built.scene,
        pixels_per_point: size.height as f32 / REFERENCE_HEIGHT_POINTS,
    }
}

fn build(sources: &Sources<'_>, level: FacetLevel) -> scene::BuiltScene {
    let context = Context::default();
    let unselected = Selection::default();
    let mut built = scene::build(
        sources,
        &Highlight {
            selection: &unselected,
            hovered: &[],
            chosen_rows: &[],
        },
        context,
        &mut SketchShapes::new(scene::drawn_faceting(sources, context, level.faceting())),
    );
    built.scene.grid = None;
    built
}

pub fn meshed(evaluation: &Evaluation, cancel: &CancelToken) -> Result<BodyMeshes, Unmeshed> {
    let mut meshing = BodyMeshing::default();
    for (body, _) in evaluation.bodies() {
        if let Some(result) = evaluation.body_result(body) {
            meshing.request(result, no_wake);
        }
    }
    let deadline = Instant::now() + MESHING_TIMEOUT;
    while meshing.is_pending() {
        if cancel.is_cancelled() {
            return Err(Unmeshed::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(Unmeshed::TimedOut);
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

#[cfg(test)]
mod tests {
    use caditor_document::{ModelEvaluator, Recompute};

    use super::*;
    use crate::samples::Sample;

    #[test]
    fn a_snapshot_is_drawn_in_the_palette_of_the_canvas_it_is_shown_on() {
        let document = Sample::Plate.document().unwrap();
        let evaluation = Recompute::default().run(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        );
        let meshes = meshed(&evaluation, &CancelToken::never()).unwrap();
        let size = SurfaceSize {
            width: 320,
            height: 200,
        };

        for canvas in Canvas::ALL {
            let snapshot = of_bodies(&document, &evaluation, &meshes, size, canvas, None);
            let origin = Contrast::Standard.palette(canvas).origin;
            let other = Contrast::Standard
                .palette(if canvas.is_dark() {
                    Canvas::Light
                } else {
                    Canvas::Dark
                })
                .origin;

            assert!(
                snapshot
                    .scene
                    .markers()
                    .any(|marker| marker.color == origin),
                "{canvas:?}"
            );
            assert!(
                snapshot.scene.markers().all(|marker| marker.color != other),
                "{canvas:?}"
            );
        }
    }
}
