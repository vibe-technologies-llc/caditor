use std::{
    collections::BTreeMap,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use caditor_expression::{Dimension, Expression, Unit};
use caditor_geometry::{Point3, Vector3};
use caditor_render::{Color, Corner, Piece, ShadedMesh};
use parking_lot::Mutex;

use crate::{
    bodies::BodyMeshes,
    measure,
    model::{Model, Waker},
    reach::{FACING_SLACK, Grid, Occluders},
    scene_palette::ScenePalette,
    selection::{Axis, Pickable, Selection},
    variants::all_variants,
    visibility,
};

pub const INLINE_TRIANGLES: usize = 40_000;
pub const MAX_DRAFT_LIMIT: f64 = 90.0;
const MOST_CONSIDERED: usize = 16;
const DEFAULT_DRAFT_LIMIT: f64 = 3.0;
const DEFAULT_RADIUS_LIMIT: f64 = 2.0;
const SMALLEST_CHORD_SQUARED: f64 = 1e-12;
const RADIUS_SLACK: f64 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnalysisCommand {
    Draft,
    Radius,
    Reach,
    UseSelected,
    Reverse,
    Comb,
}

all_variants!(AnalysisCommand: Draft, Radius, Reach, UseSelected, Reverse, Comb);

impl AnalysisCommand {
    pub fn id(self) -> &'static str {
        match self {
            Self::Draft => "view.analysis_draft",
            Self::Radius => "view.analysis_radius",
            Self::Reach => "view.analysis_reach",
            Self::UseSelected => "view.analysis_pull_selected",
            Self::Reverse => "view.analysis_pull_reverse",
            Self::Comb => "view.curvature_comb",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Draft => "Analyse draft",
            Self::Radius => "Analyse minimum radius",
            Self::Reach => "Analyse tool reach",
            Self::UseSelected => "Pull or reach along the selected axis, edge or face",
            Self::Reverse => "Reverse the pull or reach direction",
            Self::Comb => "Show or hide the curvature comb",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FaceAnalysis {
    Draft { pull: Vector3, limit: f64 },
    Radius { limit: f64 },
    Reach { reach: Vector3, occluders: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Band {
    Drafted,
    TooLittleDraft,
    Undercut,
    TooTight,
    Reachable,
    Blocked,
    FacesAway,
}

impl Band {
    const ALL: [Self; 7] = [
        Self::Drafted,
        Self::TooLittleDraft,
        Self::Undercut,
        Self::TooTight,
        Self::Reachable,
        Self::Blocked,
        Self::FacesAway,
    ];

    pub const fn class(self) -> u8 {
        self as u8 + 1
    }

    pub fn of_class(class: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|band| band.class() == class)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Drafted => "Drafted enough",
            Self::TooLittleDraft => "Too little draft",
            Self::Undercut => "Faces away from the pull",
            Self::TooTight => "Too tight to reach",
            Self::Reachable => "Reached by the tool",
            Self::Blocked => "Hidden behind other faces",
            Self::FacesAway => "Faces away from the tool",
        }
    }

    pub fn meaning(self) -> &'static str {
        match self {
            Self::Drafted => "These faces lean away from the pull by at least the limit",
            Self::TooLittleDraft => {
                "These faces are nearly parallel to the pull, so the part may stick; a parting \
                 line lies in this band"
            }
            Self::Undercut => {
                "These faces turn from the pull and cannot be pulled out this way; reverse the \
                 pull to check the other half of the mould"
            }
            Self::TooTight => {
                "These concave faces curve tighter than the radius, so a cutter or nozzle that \
                 size cannot reach into them"
            }
            Self::Reachable => {
                "These faces look toward the tool and nothing lies between them and it, so a \
                 three-axis machine reaches them from this direction"
            }
            Self::Blocked => {
                "These faces look toward the tool, but another part of the bodies lies over them \
                 along the direction, so the tool cannot get to them from this side"
            }
            Self::FacesAway => {
                "These faces turn from the tool and are undercuts from this direction; reverse it \
                 or choose another setup to reach them"
            }
        }
    }

    pub fn colour(self, palette: &ScenePalette) -> Color {
        match self {
            Self::Drafted => palette.bands.drafted,
            Self::TooLittleDraft => palette.bands.too_little_draft,
            Self::Undercut => palette.bands.undercut,
            Self::TooTight => palette.bands.too_tight,
            Self::Reachable => palette.bands.drafted,
            Self::Blocked => palette.bands.blocked,
            Self::FacesAway => palette.bands.undercut,
        }
    }
}

impl FaceAnalysis {
    pub fn bands(self) -> &'static [Band] {
        match self {
            Self::Draft { .. } => &[Band::Drafted, Band::TooLittleDraft, Band::Undercut],
            Self::Radius { .. } => &[Band::TooTight],
            Self::Reach { .. } => &[Band::Reachable, Band::Blocked, Band::FacesAway],
        }
    }

    fn classify(self, origin: Point3, grid: Option<&Grid>, corners: [Corner; 3]) -> u8 {
        match self {
            Self::Draft { pull, limit } => draft_class(pull, limit, corners),
            Self::Radius { limit } => radius_class(limit, corners),
            Self::Reach { reach, .. } => {
                grid.map_or(0, |grid| reach_class(reach, grid, origin, corners))
            }
        }
    }
}

fn mean_normal(corners: [Corner; 3]) -> Vector3 {
    corners
        .iter()
        .fold(Vector3::ZERO, |sum, corner| sum + corner.normal.as_dvec3())
        .normalize_or_zero()
}

fn reach_class(reach: Vector3, grid: &Grid, origin: Point3, corners: [Corner; 3]) -> u8 {
    let normal = mean_normal(corners);
    if normal.dot(reach) < -FACING_SLACK {
        return Band::FacesAway.class();
    }
    let centroid = origin
        + corners.iter().fold(Vector3::ZERO, |sum, corner| {
            sum + corner.position.as_dvec3()
        }) / 3.0;
    if grid.blocks(centroid, normal) {
        Band::Blocked.class()
    } else {
        Band::Reachable.class()
    }
}

fn draft_class(pull: Vector3, limit_degrees: f64, corners: [Corner; 3]) -> u8 {
    let normal = mean_normal(corners);
    if normal == Vector3::ZERO {
        return Band::TooLittleDraft.class();
    }
    let angle = normal.dot(pull).clamp(-1.0, 1.0).asin().to_degrees();
    let band = if angle >= limit_degrees {
        Band::Drafted
    } else if angle <= -limit_degrees {
        Band::Undercut
    } else {
        Band::TooLittleDraft
    };
    band.class()
}

fn radius_class(limit: f64, corners: [Corner; 3]) -> u8 {
    let [a, b, c] = corners;
    let tight = [(a, b), (b, c), (c, a)].into_iter().any(|(from, to)| {
        let chord = to.position.as_dvec3() - from.position.as_dvec3();
        let squared = chord.length_squared();
        if squared < SMALLEST_CHORD_SQUARED {
            return false;
        }
        let turn = to.normal.as_dvec3() - from.normal.as_dvec3();
        let curvature = turn.dot(chord) / squared;
        curvature * limit < -(1.0 + RADIUS_SLACK)
    });
    if tight { Band::TooTight.class() } else { 0 }
}

#[derive(Debug)]
pub struct Analysed {
    pub mesh: Arc<ShadedMesh>,
    pub pieces: Vec<Piece>,
}

impl Analysed {
    fn of(mesh: &ShadedMesh, analysis: FaceAnalysis, occluders: Option<&Occluders>) -> Self {
        let origin = mesh.origin();
        let grid = occluders.map(Occluders::grid);
        let division = mesh.divide(|corners| analysis.classify(origin, grid, corners));
        Self {
            mesh: Arc::new(division.mesh),
            pieces: division.pieces,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Ready(Arc<Analysed>),
    Working,
    Failed,
}

enum State {
    Working,
    Ready(Arc<Analysed>),
    Failed,
}

struct Entry {
    mesh: Weak<ShadedMesh>,
    analysis: FaceAnalysis,
    state: State,
}

impl Entry {
    fn is_of(&self, mesh: &Weak<ShadedMesh>, analysis: FaceAnalysis) -> bool {
        self.mesh.ptr_eq(mesh) && self.analysis == analysis
    }
}

pub struct Analyses {
    entries: Arc<Mutex<Vec<Entry>>>,
    finished: Arc<AtomicU64>,
    wake: Arc<Mutex<Option<Waker>>>,
    occluders: Mutex<Option<Arc<Occluders>>>,
    inline_triangles: usize,
}

impl Default for Analyses {
    fn default() -> Self {
        Self::working_inline_up_to(INLINE_TRIANGLES)
    }
}

impl Analyses {
    pub fn working_inline_up_to(inline_triangles: usize) -> Self {
        Self {
            entries: Arc::default(),
            finished: Arc::default(),
            wake: Arc::default(),
            occluders: Mutex::default(),
            inline_triangles,
        }
    }

    pub fn wake_with(&self, make: impl FnOnce() -> Waker) {
        let mut wake = self.wake.lock();
        if wake.is_none() {
            *wake = Some(make());
        }
    }

    pub fn finished(&self) -> u64 {
        self.finished.load(Ordering::Acquire)
    }

    pub fn prepare(
        &self,
        analysis: Option<FaceAnalysis>,
        meshes: &[Arc<ShadedMesh>],
    ) -> Option<FaceAnalysis> {
        let mut occluders = self.occluders.lock();
        let Some(FaceAnalysis::Reach { reach, .. }) = analysis else {
            *occluders = None;
            return analysis;
        };
        let kept = occluders
            .as_ref()
            .map(|current| (current.generation, current.is_of(reach, meshes)));
        let generation = match kept {
            Some((generation, true)) => generation,
            Some((generation, false)) => generation + 1,
            None => 1,
        };
        if kept != Some((generation, true)) {
            *occluders = Some(Arc::new(Occluders::new(generation, reach, meshes)));
        }
        Some(FaceAnalysis::Reach {
            reach,
            occluders: generation,
        })
    }

    fn occluders_of(&self, analysis: FaceAnalysis) -> Result<Option<Arc<Occluders>>, Outcome> {
        let FaceAnalysis::Reach { occluders, .. } = analysis else {
            return Ok(None);
        };
        self.occluders
            .lock()
            .as_ref()
            .filter(|current| current.generation == occluders)
            .map(|current| Some(Arc::clone(current)))
            .ok_or(Outcome::Working)
    }

    pub fn of(&self, mesh: &Arc<ShadedMesh>, analysis: FaceAnalysis) -> Outcome {
        let occluders = match self.occluders_of(analysis) {
            Ok(occluders) => occluders,
            Err(outcome) => return outcome,
        };
        let weak = Arc::downgrade(mesh);
        let mut entries = self.entries.lock();
        entries.retain(|entry| entry.mesh.strong_count() > 0);
        if let Some(entry) = entries.iter().find(|entry| entry.is_of(&weak, analysis)) {
            return match &entry.state {
                State::Ready(analysed) => Outcome::Ready(Arc::clone(analysed)),
                State::Working => Outcome::Working,
                State::Failed => Outcome::Failed,
            };
        }
        entries.retain(|entry| !entry.mesh.ptr_eq(&weak));
        let work = mesh.triangle_count()
            + occluders
                .as_ref()
                .map_or(0, |occluders| occluders.triangles);
        if work <= self.inline_triangles {
            let analysed = Arc::new(Analysed::of(mesh, analysis, occluders.as_deref()));
            entries.push(Entry {
                mesh: weak,
                analysis,
                state: State::Ready(Arc::clone(&analysed)),
            });
            return Outcome::Ready(analysed);
        }
        entries.push(Entry {
            mesh: weak.clone(),
            analysis,
            state: State::Working,
        });
        drop(entries);
        self.spawn(Arc::clone(mesh), weak, analysis, occluders);
        Outcome::Working
    }

    fn spawn(
        &self,
        mesh: Arc<ShadedMesh>,
        weak: Weak<ShadedMesh>,
        analysis: FaceAnalysis,
        occluders: Option<Arc<Occluders>>,
    ) {
        let entries = Arc::clone(&self.entries);
        let finished = Arc::clone(&self.finished);
        let wake = Arc::clone(&self.wake);
        let spawned = thread::Builder::new()
            .name("analysis".to_owned())
            .spawn(move || {
                let analysed = panic::catch_unwind(AssertUnwindSafe(|| {
                    Arc::new(Analysed::of(&mesh, analysis, occluders.as_deref()))
                }));
                if analysed.is_err() {
                    log::error!("analysing the faces of a body panicked");
                }
                drop(mesh);
                drop(occluders);
                {
                    let mut entries = entries.lock();
                    if let Some(entry) = entries
                        .iter_mut()
                        .find(|entry| entry.is_of(&weak, analysis))
                    {
                        entry.state = match analysed {
                            Ok(analysed) => State::Ready(analysed),
                            Err(_) => State::Failed,
                        };
                    }
                }
                finished.fetch_add(1, Ordering::Release);
                if let Some(wake) = wake.lock().as_ref() {
                    wake();
                }
            });
        if let Err(error) = spawned {
            log::error!("could not start the analysis thread: {error}");
            let mut entries = self.entries.lock();
            for entry in entries.iter_mut() {
                if matches!(entry.state, State::Working) {
                    entry.state = State::Failed;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tally {
    pub areas: BTreeMap<Band, f64>,
    pub working: bool,
    pub failed: bool,
}

impl Tally {
    pub fn of<'a>(
        analyses: &Analyses,
        meshes: impl IntoIterator<Item = &'a Arc<ShadedMesh>>,
        analysis: FaceAnalysis,
    ) -> Self {
        let mut tally = Self::default();
        for band in analysis.bands() {
            tally.areas.insert(*band, 0.0);
        }
        for mesh in meshes {
            match analyses.of(mesh, analysis) {
                Outcome::Ready(analysed) => {
                    for piece in &analysed.pieces {
                        if let Some(band) = Band::of_class(piece.class)
                            && let Some(area) = tally.areas.get_mut(&band)
                        {
                            *area += piece.area;
                        }
                    }
                }
                Outcome::Working => tally.working = true,
                Outcome::Failed => tally.failed = true,
            }
        }
        tally
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Draft,
    Radius,
    Reach,
}

impl Kind {
    pub fn is_directed(self) -> bool {
        matches!(self, Self::Draft | Self::Reach)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pull {
    Axis(Axis),
    Picked(Pickable),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Problem {
    #[error(
        "The item chosen as the pull direction is gone or no longer gives a direction. Choose \
         another."
    )]
    PullGone,
    #[error("The limit could not be worked out. Enter it again.")]
    LimitUnreadable,
    #[error("Enter a draft limit from 0° to 90°")]
    DraftLimitOutOfRange,
    #[error("Enter a radius above zero")]
    RadiusNotAboveZero,
}

pub fn check_draft_limit(degrees: f64) -> Result<(), Problem> {
    if (0.0..=MAX_DRAFT_LIMIT).contains(&degrees) {
        Ok(())
    } else {
        Err(Problem::DraftLimitOutOfRange)
    }
}

pub fn check_radius_limit(millimetres: f64) -> Result<(), Problem> {
    if millimetres > 0.0 {
        Ok(())
    } else {
        Err(Problem::RadiusNotAboveZero)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Refusal {
    #[error("Select an axis, a straight edge, a flat face or a round face to pull along.")]
    NothingGivesADirection,
    #[error("{0} of the selected items give a direction. Select one to pull along.")]
    SeveralGiveADirection(usize),
    #[error("{0} items are selected. Select one axis, edge or face to pull along.")]
    TooManySelected(usize),
}

pub struct AnalysisTool {
    pub open: bool,
    pub kind: Kind,
    pub pull: Pull,
    pub reversed: bool,
    pub draft_limit: Expression,
    pub radius_limit: Expression,
}

impl Default for AnalysisTool {
    fn default() -> Self {
        Self {
            open: false,
            kind: Kind::Draft,
            pull: Pull::Axis(Axis::Z),
            reversed: false,
            draft_limit: Expression::measure(DEFAULT_DRAFT_LIMIT, Unit::Degree),
            radius_limit: Expression::measure(DEFAULT_RADIUS_LIMIT, Unit::Millimetre),
        }
    }
}

impl AnalysisTool {
    pub fn toggle(&mut self, kind: Kind) {
        if self.open && self.kind == kind {
            self.open = false;
        } else {
            self.open = true;
            self.kind = kind;
        }
    }

    pub fn pull_from(model: &Model, selection: &Selection) -> Result<Pull, Refusal> {
        if selection.len() > MOST_CONSIDERED {
            return Err(Refusal::TooManySelected(selection.len()));
        }
        let mut giving: Vec<Pull> = selection
            .in_pick_order()
            .into_iter()
            .filter(|pickable| measure::direction_of(model, *pickable).is_some())
            .map(|pickable| match pickable {
                Pickable::Axis(axis) => Pull::Axis(axis),
                other => Pull::Picked(other),
            })
            .collect();
        match giving.len() {
            0 => Err(Refusal::NothingGivesADirection),
            1 => giving.pop().ok_or(Refusal::NothingGivesADirection),
            several => Err(Refusal::SeveralGiveADirection(several)),
        }
    }

    pub fn pull_name(&self, model: &Model) -> String {
        match self.pull {
            Pull::Axis(axis) => axis.name().to_owned(),
            Pull::Picked(pickable) => pickable.describe(model.document(), model.evaluation()),
        }
    }

    pub fn analysis(&self, model: &Model) -> Result<FaceAnalysis, Problem> {
        match self.kind {
            Kind::Draft => {
                let limit = self.limit(model, &self.draft_limit, Dimension::ANGLE)?;
                check_draft_limit(limit)?;
                let pull = self.direction(model)?;
                Ok(FaceAnalysis::Draft {
                    pull: if self.reversed { -pull } else { pull },
                    limit,
                })
            }
            Kind::Radius => {
                let limit = self.limit(model, &self.radius_limit, Dimension::LENGTH)?;
                check_radius_limit(limit)?;
                Ok(FaceAnalysis::Radius { limit })
            }
            Kind::Reach => {
                let reach = self.direction(model)?;
                Ok(FaceAnalysis::Reach {
                    reach: if self.reversed { -reach } else { reach },
                    occluders: 0,
                })
            }
        }
    }

    fn limit(
        &self,
        model: &Model,
        expression: &Expression,
        dimension: Dimension,
    ) -> Result<f64, Problem> {
        let quantity = model
            .parameters()
            .evaluate_expression(expression)
            .map_err(|_| Problem::LimitUnreadable)?;
        if quantity.dimension != dimension || !quantity.value.is_finite() {
            return Err(Problem::LimitUnreadable);
        }
        Ok(quantity.value)
    }

    fn direction(&self, model: &Model) -> Result<Vector3, Problem> {
        let direction = match self.pull {
            Pull::Axis(axis) => Some(axis.direction()),
            Pull::Picked(pickable) => measure::direction_of(model, pickable),
        };
        direction
            .map(Vector3::normalize_or_zero)
            .filter(|direction| *direction != Vector3::ZERO)
            .ok_or(Problem::PullGone)
    }
}

pub fn shown_meshes(model: &Model, bodies: &BodyMeshes) -> Vec<Arc<ShadedMesh>> {
    bodies
        .iter()
        .filter(|(body, _)| visibility::is_shown(model.document(), *body))
        .map(|(_, mesh)| Arc::clone(&mesh.mesh))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };

    use caditor_document::{CancelToken, ModelEvaluator, Recompute};
    use caditor_render::{MeshFace, MeshPoint};

    use super::*;
    use crate::{bodies::BodyMeshes, samples::Sample, snapshot};

    fn corner(x: f64, y: f64, z: f64, normal: Vector3) -> Corner {
        Corner {
            position: Vector3::new(x, y, z).as_vec3(),
            normal: normal.as_vec3(),
        }
    }

    fn flat(normal: Vector3) -> [Corner; 3] {
        [
            corner(0.0, 0.0, 0.0, normal),
            corner(1.0, 0.0, 0.0, normal),
            corner(0.0, 1.0, 0.0, normal),
        ]
    }

    #[test]
    fn a_face_is_banded_by_the_angle_its_normal_makes_with_the_pull() {
        let analysis = FaceAnalysis::Draft {
            pull: Vector3::Z,
            limit: 3.0,
        };
        let leaning = |degrees: f64| {
            let tilt = degrees.to_radians();
            Vector3::new(tilt.cos(), 0.0, tilt.sin())
        };

        let bands = [
            leaning(45.0),
            leaning(5.0),
            leaning(2.0),
            leaning(0.0),
            leaning(-2.0),
            leaning(-5.0),
            Vector3::Z,
            -Vector3::Z,
        ]
        .map(|normal| Band::of_class(analysis.classify(Point3::ZERO, None, flat(normal))));

        assert_eq!(
            bands,
            [
                Some(Band::Drafted),
                Some(Band::Drafted),
                Some(Band::TooLittleDraft),
                Some(Band::TooLittleDraft),
                Some(Band::TooLittleDraft),
                Some(Band::Undercut),
                Some(Band::Drafted),
                Some(Band::Undercut),
            ]
        );
    }

    #[test]
    fn reversing_the_pull_swaps_what_is_drafted_and_what_is_an_undercut() {
        let up = FaceAnalysis::Draft {
            pull: Vector3::Z,
            limit: 3.0,
        };
        let down = FaceAnalysis::Draft {
            pull: -Vector3::Z,
            limit: 3.0,
        };

        assert_eq!(
            Band::of_class(up.classify(Point3::ZERO, None, flat(Vector3::Z))),
            Some(Band::Drafted)
        );
        assert_eq!(
            Band::of_class(down.classify(Point3::ZERO, None, flat(Vector3::Z))),
            Some(Band::Undercut)
        );
    }

    fn arc(radius: f64, outward: bool) -> [Corner; 3] {
        let at = |degrees: f64| {
            let angle = degrees.to_radians();
            let out = Vector3::new(angle.cos(), angle.sin(), 0.0);
            let normal = if outward { out } else { -out };
            corner(radius * out.x, radius * out.y, 0.0, normal)
        };
        [at(0.0), at(5.0), at(10.0)]
    }

    #[test]
    fn only_a_concave_face_tighter_than_the_radius_is_too_tight() {
        let limit = |limit: f64| FaceAnalysis::Radius { limit };

        assert_eq!(
            limit(5.0).classify(Point3::ZERO, None, arc(4.0, false)),
            Band::TooTight.class()
        );
        assert_eq!(limit(3.0).classify(Point3::ZERO, None, arc(4.0, false)), 0);
        assert_eq!(limit(4.0).classify(Point3::ZERO, None, arc(4.0, false)), 0);
        assert_eq!(limit(5.0).classify(Point3::ZERO, None, arc(4.0, true)), 0);
        assert_eq!(limit(5.0).classify(Point3::ZERO, None, flat(Vector3::Z)), 0);
    }

    #[test]
    fn classes_round_trip_through_their_bands_and_zero_is_no_band() {
        assert_eq!(Band::of_class(0), None);
        for band in Band::ALL {
            assert_eq!(Band::of_class(band.class()), Some(band));
        }
    }

    fn plate() -> Vec<Arc<ShadedMesh>> {
        let document = Sample::Plate.document().unwrap();
        let evaluation = Recompute::default().run(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        );
        let meshes: BodyMeshes = snapshot::meshed(&evaluation).unwrap();
        meshes
            .iter()
            .map(|(_, mesh)| Arc::clone(&mesh.mesh))
            .collect()
    }

    fn area_of(tally: &Tally, band: Band) -> f64 {
        tally.areas.get(&band).copied().unwrap_or(f64::NAN)
    }

    #[test]
    fn the_walls_of_the_plates_holes_are_too_tight_for_a_wider_cutter_only() {
        let meshes = plate();
        let analyses = Analyses::default();
        let walls = 2.0 * std::f64::consts::TAU * 4.0 * 6.0;

        let wide = Tally::of(&analyses, &meshes, FaceAnalysis::Radius { limit: 5.0 });
        let narrow = Tally::of(&analyses, &meshes, FaceAnalysis::Radius { limit: 3.0 });

        assert!((area_of(&wide, Band::TooTight) - walls).abs() / walls < 0.03);
        assert_eq!(area_of(&narrow, Band::TooTight), 0.0);
    }

    #[test]
    fn pulling_a_plate_along_z_drafts_its_top_undercuts_its_bottom_and_leaves_its_walls() {
        let meshes = plate();
        let analyses = Analyses::default();
        let face = 80.0 * 50.0 - 2.0 * std::f64::consts::PI * 16.0;
        let walls = 2.0 * (80.0 + 50.0) * 6.0 + 2.0 * std::f64::consts::TAU * 4.0 * 6.0;

        let tally = Tally::of(
            &analyses,
            &meshes,
            FaceAnalysis::Draft {
                pull: Vector3::Z,
                limit: 3.0,
            },
        );

        assert!((area_of(&tally, Band::Drafted) - face).abs() / face < 0.03);
        assert!((area_of(&tally, Band::Undercut) - face).abs() / face < 0.03);
        assert!((area_of(&tally, Band::TooLittleDraft) - walls).abs() / walls < 0.03);
    }

    fn reach_of(
        analyses: &Analyses,
        meshes: &[Arc<ShadedMesh>],
        reach: Vector3,
    ) -> (FaceAnalysis, Tally) {
        let analysis = analyses
            .prepare(
                Some(FaceAnalysis::Reach {
                    reach,
                    occluders: 0,
                }),
                meshes,
            )
            .unwrap();
        (analysis, Tally::of(analyses, meshes, analysis))
    }

    #[test]
    fn reaching_a_plate_from_above_gets_its_top_and_walls_and_not_its_bottom() {
        let meshes = plate();
        let analyses = Analyses::default();
        let face = 80.0 * 50.0 - 2.0 * std::f64::consts::PI * 16.0;
        let walls = 2.0 * (80.0 + 50.0) * 6.0 + 2.0 * std::f64::consts::TAU * 4.0 * 6.0;

        let (_, tally) = reach_of(&analyses, &meshes, Vector3::Z);

        assert!((area_of(&tally, Band::Reachable) - face - walls).abs() / (face + walls) < 0.03);
        assert!((area_of(&tally, Band::FacesAway) - face).abs() / face < 0.03);
        assert_eq!(area_of(&tally, Band::Blocked), 0.0);
    }

    fn slab(low: Point3, high: Point3) -> Arc<ShadedMesh> {
        let side = |normal: Vector3, corners: [Point3; 4]| MeshFace {
            points: corners
                .into_iter()
                .map(|position| MeshPoint { position, normal })
                .collect(),
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        };
        let at = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
        let (a, b) = (low, high);
        Arc::new(ShadedMesh::new([
            side(
                Vector3::Z,
                [
                    at(a.x, a.y, b.z),
                    at(b.x, a.y, b.z),
                    at(b.x, b.y, b.z),
                    at(a.x, b.y, b.z),
                ],
            ),
            side(
                -Vector3::Z,
                [
                    at(a.x, a.y, a.z),
                    at(a.x, b.y, a.z),
                    at(b.x, b.y, a.z),
                    at(b.x, a.y, a.z),
                ],
            ),
        ]))
    }

    #[test]
    fn a_body_over_another_hides_the_part_of_it_below_from_a_tool_above() {
        let analyses = Analyses::default();
        let floor = slab(Point3::ZERO, Point3::new(10.0, 10.0, 1.0));
        let roof = slab(Point3::new(-1.0, -1.0, 4.0), Point3::new(5.0, 12.0, 5.0));
        let meshes = vec![Arc::clone(&floor), roof];

        let (_, from_above) = reach_of(&analyses, &meshes, Vector3::Z);
        let (_, from_below) = reach_of(&analyses, &meshes, -Vector3::Z);
        let (_, floor_alone) = reach_of(&analyses, &[floor], Vector3::Z);

        assert_eq!(area_of(&from_above, Band::Blocked), 50.0);
        assert_eq!(area_of(&from_above, Band::Reachable), 50.0 + 78.0);
        assert_eq!(area_of(&from_above, Band::FacesAway), 100.0 + 78.0);
        assert_eq!(area_of(&from_below, Band::Blocked), 78.0);
        assert_eq!(area_of(&floor_alone, Band::Blocked), 0.0);
    }

    #[test]
    fn a_changed_set_of_bodies_or_direction_is_a_new_reach_analysis() {
        let analyses = Analyses::default();
        let meshes = plate();

        let (first, _) = reach_of(&analyses, &meshes, Vector3::Z);
        let (again, _) = reach_of(&analyses, &meshes, Vector3::Z);
        let (turned, _) = reach_of(&analyses, &meshes, Vector3::X);
        let (alone, _) = reach_of(&analyses, &[], Vector3::X);

        assert_eq!(first, again);
        assert_ne!(first, turned);
        assert_ne!(turned, alone);
        assert!(matches!(
            analyses.of(meshes.first().unwrap(), first),
            Outcome::Working
        ));
        assert_eq!(analyses.prepare(None, &meshes), None);
    }

    #[test]
    fn a_large_mesh_is_worked_out_on_a_thread_that_wakes_the_app_when_it_is_done() {
        let meshes = plate();
        let analyses = Analyses::working_inline_up_to(0);
        let woken = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&woken);
        analyses.wake_with(|| Box::new(move || flag.store(true, Ordering::Release)));
        let analysis = FaceAnalysis::Radius { limit: 5.0 };
        let mesh = meshes.first().unwrap();

        let first = analyses.of(mesh, analysis);
        let deadline = Instant::now() + Duration::from_secs(10);
        while analyses.finished() == 0 {
            assert!(Instant::now() < deadline, "the analysis never finished");
            thread::sleep(Duration::from_millis(2));
        }

        assert!(matches!(first, Outcome::Working));
        assert!(matches!(analyses.of(mesh, analysis), Outcome::Ready(_)));
        assert!(woken.load(Ordering::Acquire));
    }
}
