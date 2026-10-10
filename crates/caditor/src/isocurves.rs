use std::{
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use caditor_document::FeatureResult;
use caditor_geometry::Point3;
use caditor_kernel::{
    Along, CurveDerivatives, FaceId, IsoparametricError, IsoparametricRun, Surface,
    SurfaceDerivatives,
};
use caditor_render::{Batch, Layer, Line, Stroke};
use parking_lot::Mutex;

use crate::{
    bodies,
    comb::{self, Comb, CombDrawing, Combed, HALO_WIDTH, Tooth},
    model::{Model, Waker},
    scene_palette::ScenePalette,
    selection::{Pickable, Selection},
};

pub const DEFAULT_LINES: usize = 5;
pub const MIN_LINES: usize = 1;
pub const MAX_LINES: usize = 16;
pub const DEFAULT_TEETH: usize = 20;
pub const MOST_FACES: usize = 16;
const STEPS_PER_LINE: usize = 64;
const FLAT_CURVATURE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Directions {
    #[default]
    Both,
    U,
    V,
}

impl Directions {
    pub const ALL: [Self; 3] = [Self::Both, Self::U, Self::V];

    pub fn title(self) -> &'static str {
        match self {
            Self::Both => "Both",
            Self::U => "U",
            Self::V => "V",
        }
    }

    fn along(self) -> &'static [Along] {
        match self {
            Self::Both => &[Along::U, Along::V],
            Self::U => &[Along::U],
            Self::V => &[Along::V],
        }
    }
}

pub fn along_title(along: Along) -> &'static str {
    match along {
        Along::U => "U lines",
        Along::V => "V lines",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Isocurve {
    pub along: Along,
    pub points: Vec<Point3>,
    pub teeth: Vec<Tooth>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceLines {
    pub pickable: Pickable,
    pub name: String,
    pub curves: Vec<Isocurve>,
}

impl FaceLines {
    pub fn greatest_curvature(&self, along: Along) -> Option<f64> {
        let mut teeth = self
            .curves
            .iter()
            .filter(|curve| curve.along == along)
            .flat_map(|curve| curve.teeth.iter())
            .peekable();
        teeth.peek()?;
        Some(
            teeth
                .map(|tooth| tooth.curvature.length())
                .fold(0.0, f64::max),
        )
    }

    pub fn smallest_radius(&self, along: Along) -> Option<Option<f64>> {
        let greatest = self.greatest_curvature(along)?;
        Some((greatest > FLAT_CURVATURE).then(|| 1.0 / greatest))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Isocurves {
    pub faces: Vec<FaceLines>,
    pub gone: usize,
    pub failed: bool,
}

impl Isocurves {
    pub fn comb(&self) -> Comb {
        Comb {
            curves: self
                .faces
                .iter()
                .flat_map(|face| {
                    face.curves.iter().map(|curve| Combed {
                        pickable: face.pickable,
                        name: face.name.clone(),
                        teeth: curve.teeth.clone(),
                    })
                })
                .collect(),
            joints: Vec::new(),
            gone: 0,
        }
    }

    pub fn drawing(&self, scale: f64) -> IsocurveDrawing {
        IsocurveDrawing {
            lines: self
                .faces
                .iter()
                .flat_map(|face| face.curves.iter())
                .flat_map(|curve| {
                    curve.points.windows(2).filter_map(|pair| match pair {
                        [from, to] => Some([*from, *to]),
                        _ => None,
                    })
                })
                .collect(),
            comb: self.comb().drawing(scale),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IsocurveDrawing {
    pub lines: Vec<[Point3; 2]>,
    pub comb: CombDrawing,
}

impl IsocurveDrawing {
    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette) {
        let look = palette.comb;
        let line = |[start, end]: [Point3; 2], color, width| Line {
            start,
            end,
            color,
            width,
            layer: Layer::Front,
            pick: None,
            stroke: Stroke::Solid,
        };
        let halos = self
            .lines
            .iter()
            .map(|segment| line(*segment, palette.outline, look.isocurve_width + HALO_WIDTH));
        let lines = self
            .lines
            .iter()
            .map(|segment| line(*segment, look.isocurve, look.isocurve_width));
        batch.lines.extend(halos.chain(lines));
        self.comb.add_to(batch, palette);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub lines: usize,
    pub directions: Directions,
    pub teeth: usize,
}

impl Settings {
    fn clamped(self) -> Self {
        Self {
            lines: self.lines.clamp(MIN_LINES, MAX_LINES),
            directions: self.directions,
            teeth: self.teeth.clamp(comb::MIN_TEETH, comb::MAX_TEETH),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    faces: Vec<Pickable>,
    settings: Settings,
    revision: u64,
    evaluation: u64,
}

struct Chosen {
    pickable: Pickable,
    name: String,
    result: Arc<FeatureResult>,
    face: FaceId,
}

fn resolve(model: &Model, pickable: Pickable) -> Option<Chosen> {
    let Pickable::Face { body, face } = pickable else {
        return None;
    };
    let evaluation = model.evaluation();
    let result = evaluation.body_result(body)?;
    let face = bodies::find_face(result.solid()?, face)?;
    Some(Chosen {
        pickable,
        name: pickable.describe(model.document(), evaluation),
        result: Arc::clone(result),
        face,
    })
}

fn derivatives_along(along: Along, surface: &SurfaceDerivatives) -> CurveDerivatives {
    let (first, second) = match along {
        Along::U => (surface.du, surface.duu),
        Along::V => (surface.dv, surface.dvv),
    };
    CurveDerivatives {
        point: surface.point,
        first,
        second,
    }
}

fn isocurve(surface: &Surface, run: &IsoparametricRun, teeth: usize) -> Isocurve {
    let evaluate = |parameter: f64| {
        let uv = run.uv(parameter);
        derivatives_along(run.along, &surface.evaluate(uv.x, uv.y))
    };
    Isocurve {
        along: run.along,
        points: (0..=STEPS_PER_LINE)
            .map(|step| surface.point_at(run.uv(run.span.at(step as f64 / STEPS_PER_LINE as f64))))
            .collect(),
        teeth: comb::spaced_teeth_along(evaluate, run.span, teeth),
    }
}

fn lines_of(chosen: &Chosen, settings: Settings) -> Result<Option<FaceLines>, IsoparametricError> {
    let Some(solid) = chosen.result.solid().map(|result| &result.solid) else {
        return Ok(None);
    };
    let Some(face) = solid.face(chosen.face) else {
        return Ok(None);
    };
    let mut curves = Vec::new();
    for along in settings.directions.along() {
        let runs = match solid.isoparametric_runs(chosen.face, *along, settings.lines) {
            Ok(runs) => runs,
            Err(IsoparametricError::MissingFace) => return Ok(None),
            Err(IsoparametricError::Cancelled) => return Err(IsoparametricError::Cancelled),
        };
        curves.extend(
            runs.iter()
                .map(|run| isocurve(face.surface(), run, settings.teeth)),
        );
    }
    Ok(Some(FaceLines {
        pickable: chosen.pickable,
        name: chosen.name.clone(),
        curves,
    }))
}

fn work_out(chosen: &[Chosen], gone: usize, settings: Settings) -> Option<Isocurves> {
    let mut isocurves = Isocurves {
        gone,
        ..Isocurves::default()
    };
    for face in chosen {
        match lines_of(face, settings) {
            Ok(Some(lines)) => isocurves.faces.push(lines),
            Ok(None) => isocurves.gone += 1,
            Err(_) => return None,
        }
    }
    Some(isocurves)
}

type Finished = Arc<Mutex<Option<(u64, Arc<Isocurves>)>>>;

struct Job {
    basis: Basis,
    generation: u64,
    cancelled: Arc<AtomicBool>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub struct IsocurveTool {
    pub open: bool,
    pub lines: usize,
    pub directions: Directions,
    pub teeth: usize,
    pub scale: f64,
    faces: Vec<Pickable>,
    left_out: usize,
    followed: Option<u64>,
    job: Option<Job>,
    generation: u64,
    finished: Finished,
    shown: Option<(Basis, Arc<Isocurves>)>,
    drawn: Option<(f64, Arc<Isocurves>, Arc<IsocurveDrawing>)>,
}

impl Default for IsocurveTool {
    fn default() -> Self {
        Self {
            open: false,
            lines: DEFAULT_LINES,
            directions: Directions::Both,
            teeth: DEFAULT_TEETH,
            scale: comb::DEFAULT_SCALE,
            faces: Vec::new(),
            left_out: 0,
            followed: None,
            job: None,
            generation: 0,
            finished: Finished::default(),
            shown: None,
            drawn: None,
        }
    }
}

impl IsocurveTool {
    pub fn toggle(&mut self) {
        if self.open {
            *self = Self {
                lines: self.lines,
                directions: self.directions,
                teeth: self.teeth,
                scale: self.scale,
                ..Self::default()
            };
        } else {
            self.open = true;
        }
    }

    pub fn forget(&mut self) {
        self.cancel();
        self.faces.clear();
        self.left_out = 0;
        self.followed = None;
        self.shown = None;
        self.drawn = None;
    }

    fn cancel(&mut self) {
        self.job = None;
    }

    pub fn faces(&self) -> &[Pickable] {
        &self.faces
    }

    pub fn left_out(&self) -> usize {
        self.left_out
    }

    pub fn is_working(&self) -> bool {
        self.job.is_some()
    }

    pub fn follow(&mut self, selection: &Selection) {
        if self.followed == Some(selection.generation()) {
            return;
        }
        self.followed = Some(selection.generation());
        let chosen: Vec<Pickable> = selection
            .in_pick_order()
            .into_iter()
            .filter(|pickable| matches!(pickable, Pickable::Face { .. }))
            .collect();
        if chosen.is_empty() {
            return;
        }
        self.left_out = chosen.len().saturating_sub(MOST_FACES);
        self.faces = chosen.into_iter().take(MOST_FACES).collect();
    }

    fn settings(&self) -> Settings {
        Settings {
            lines: self.lines,
            directions: self.directions,
            teeth: self.teeth,
        }
        .clamped()
    }

    pub fn isocurves(&mut self, model: &Model) -> Option<Arc<Isocurves>> {
        let basis = Basis {
            faces: self.faces.clone(),
            settings: self.settings(),
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
        };
        if self.shows(&basis) {
            self.cancel();
        } else {
            self.take_finished();
            let started = self.job.as_ref().is_some_and(|job| job.basis == basis);
            if !self.shows(&basis) && !started {
                self.start(model, basis);
            }
        }
        self.shown.as_ref().map(|(_, shown)| Arc::clone(shown))
    }

    fn shows(&self, basis: &Basis) -> bool {
        self.shown.as_ref().is_some_and(|(shown, _)| shown == basis)
    }

    fn take_finished(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        let mut finished = self.finished.lock();
        if finished
            .as_ref()
            .is_some_and(|(generation, _)| *generation == job.generation)
            && let Some((_, isocurves)) = finished.take()
        {
            drop(finished);
            if let Some(job) = self.job.take() {
                self.shown = Some((job.basis.clone(), isocurves));
            }
        }
    }

    fn start(&mut self, model: &Model, basis: Basis) {
        self.cancel();
        let mut gone = 0;
        let chosen: Vec<Chosen> = basis
            .faces
            .iter()
            .filter_map(|pickable| {
                let resolved = resolve(model, *pickable);
                if resolved.is_none() {
                    gone += 1;
                }
                resolved
            })
            .collect();
        if chosen.is_empty() {
            self.shown = Some((
                basis,
                Arc::new(Isocurves {
                    gone,
                    ..Isocurves::default()
                }),
            ));
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        let cancelled = Arc::new(AtomicBool::new(false));
        let settings = basis.settings;
        let spawned = spawn(Work {
            chosen,
            gone,
            settings,
            generation,
            cancelled: Arc::clone(&cancelled),
            finished: Arc::clone(&self.finished),
            wake: model.waker(),
        });
        match spawned {
            Ok(()) => {
                self.job = Some(Job {
                    basis,
                    generation,
                    cancelled,
                });
            }
            Err(error) => {
                log::error!("could not start the isocurve thread: {error}");
                self.shown = Some((
                    basis,
                    Arc::new(Isocurves {
                        failed: true,
                        ..Isocurves::default()
                    }),
                ));
            }
        }
    }

    pub fn drawing(&mut self, isocurves: &Arc<Isocurves>) -> Arc<IsocurveDrawing> {
        let scale = self.scale.clamp(comb::MIN_SCALE, comb::MAX_SCALE);
        if let Some((drawn_scale, drawn, drawing)) = &self.drawn
            && *drawn_scale == scale
            && Arc::ptr_eq(drawn, isocurves)
        {
            return Arc::clone(drawing);
        }
        let drawing = Arc::new(isocurves.drawing(scale));
        self.drawn = Some((scale, Arc::clone(isocurves), Arc::clone(&drawing)));
        drawing
    }
}

struct Work {
    chosen: Vec<Chosen>,
    gone: usize,
    settings: Settings,
    generation: u64,
    cancelled: Arc<AtomicBool>,
    finished: Finished,
    wake: Waker,
}

fn spawn(work: Work) -> std::io::Result<()> {
    thread::Builder::new()
        .name("isocurves".to_owned())
        .spawn(move || {
            let Work {
                chosen,
                gone,
                settings,
                generation,
                cancelled,
                finished,
                wake,
            } = work;
            let stop = Arc::clone(&cancelled);
            let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                caditor_kernel::interruptible(
                    Arc::new(move || stop.load(Ordering::Acquire)),
                    || work_out(&chosen, gone, settings),
                )
            }));
            let isocurves = match outcome {
                Ok(Some(isocurves)) => isocurves,
                Ok(None) => return,
                Err(_) => {
                    log::error!("working out the isocurves of the chosen faces panicked");
                    Isocurves {
                        failed: true,
                        ..Isocurves::default()
                    }
                }
            };
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            *finished.lock() = Some((generation, Arc::new(isocurves)));
            wake();
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Plane, Vector3};
    use caditor_kernel::{BSplineSurface, Cylinder, Interval};

    use super::*;

    fn run(along: Along, fixed: f64, start: f64, end: f64) -> IsoparametricRun {
        IsoparametricRun {
            along,
            fixed,
            span: Interval::new(start, end).unwrap(),
        }
    }

    #[test]
    fn the_lines_round_a_cylinder_are_combed_by_its_radius_and_the_lines_along_it_are_straight() {
        let cylinder: Surface = Cylinder::new(Plane::XY, 4.0).unwrap().into();

        let around = isocurve(&cylinder, &run(Along::U, 2.0, 0.0, 3.0), 12);
        let along = isocurve(&cylinder, &run(Along::V, 1.0, 0.0, 10.0), 12);

        assert_eq!(around.teeth.len(), 13);
        assert_eq!(around.points.len(), STEPS_PER_LINE + 1);
        assert!(
            around
                .teeth
                .iter()
                .all(|tooth| (tooth.curvature.length() - 0.25).abs() < 1e-9)
        );
        assert!(
            around
                .points
                .iter()
                .all(|point| (point.z - 2.0).abs() < 1e-12)
        );
        assert!(
            along
                .teeth
                .iter()
                .all(|tooth| tooth.curvature.length() < 1e-12)
        );
        assert!(
            along
                .teeth
                .iter()
                .all(|tooth| tooth.tangent.distance(Vector3::Z) < 1e-12)
        );
    }

    #[test]
    fn a_spline_face_bending_one_way_combs_its_bend_and_reads_its_radius_per_direction() {
        let bent = BSplineSurface::new(
            2,
            1,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            3,
            vec![
                Point3::new(-1.0, 0.0, 0.0),
                Point3::new(0.0, 0.0, 2.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(-1.0, 4.0, 0.0),
                Point3::new(0.0, 4.0, 2.0),
                Point3::new(1.0, 4.0, 0.0),
            ],
            None,
        )
        .unwrap();
        let surface: Surface = bent.into();
        let face = FaceLines {
            pickable: Pickable::Origin,
            name: String::new(),
            curves: vec![
                isocurve(&surface, &run(Along::U, 0.5, 0.0, 1.0), 20),
                isocurve(&surface, &run(Along::V, 0.5, 0.0, 1.0), 20),
            ],
        };
        let isocurves = Isocurves {
            faces: vec![face.clone()],
            ..Isocurves::default()
        };

        let drawing = isocurves.drawing(1.0);
        let middle = face.curves[0].teeth[10];

        assert!((middle.curvature - Vector3::new(0.0, 0.0, -2.0)).length() < 1e-9);
        assert!((face.smallest_radius(Along::U).flatten().unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(face.smallest_radius(Along::V), Some(None));
        assert_eq!(drawing.lines.len(), 2 * STEPS_PER_LINE);
        assert_eq!(drawing.comb.teeth.len(), 21);
        assert_eq!(drawing.comb.envelope.len(), 40);
    }
}
