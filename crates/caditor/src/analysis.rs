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
use caditor_geometry::Vector3;
use caditor_render::{Color, Corner, Piece, ShadedMesh};
use parking_lot::Mutex;

use crate::{
    measure,
    model::{Model, Waker},
    scene_palette::ScenePalette,
    selection::{Axis, Pickable, Selection},
    variants::all_variants,
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
    UseSelected,
    Reverse,
}

all_variants!(AnalysisCommand: Draft, Radius, UseSelected, Reverse);

impl AnalysisCommand {
    pub fn id(self) -> &'static str {
        match self {
            Self::Draft => "view.analysis_draft",
            Self::Radius => "view.analysis_radius",
            Self::UseSelected => "view.analysis_pull_selected",
            Self::Reverse => "view.analysis_pull_reverse",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Draft => "Analyse draft",
            Self::Radius => "Analyse minimum radius",
            Self::UseSelected => "Pull along the selected axis, edge or face",
            Self::Reverse => "Reverse the pull direction",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FaceAnalysis {
    Draft { pull: Vector3, limit: f64 },
    Radius { limit: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Band {
    Drafted,
    TooLittleDraft,
    Undercut,
    TooTight,
}

impl Band {
    pub const fn class(self) -> u8 {
        self as u8 + 1
    }

    pub fn of_class(class: u8) -> Option<Self> {
        [
            Self::Drafted,
            Self::TooLittleDraft,
            Self::Undercut,
            Self::TooTight,
        ]
        .into_iter()
        .find(|band| band.class() == class)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Drafted => "Drafted enough",
            Self::TooLittleDraft => "Too little draft",
            Self::Undercut => "Faces away from the pull",
            Self::TooTight => "Too tight to reach",
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
        }
    }

    pub fn colour(self, palette: &ScenePalette) -> Color {
        match self {
            Self::Drafted => palette.bands.drafted,
            Self::TooLittleDraft => palette.bands.too_little_draft,
            Self::Undercut => palette.bands.undercut,
            Self::TooTight => palette.bands.too_tight,
        }
    }
}

impl FaceAnalysis {
    pub fn bands(self) -> &'static [Band] {
        match self {
            Self::Draft { .. } => &[Band::Drafted, Band::TooLittleDraft, Band::Undercut],
            Self::Radius { .. } => &[Band::TooTight],
        }
    }

    fn classify(self, corners: [Corner; 3]) -> u8 {
        match self {
            Self::Draft { pull, limit } => draft_class(pull, limit, corners),
            Self::Radius { limit } => radius_class(limit, corners),
        }
    }
}

fn draft_class(pull: Vector3, limit_degrees: f64, corners: [Corner; 3]) -> u8 {
    let normal = corners
        .iter()
        .fold(Vector3::ZERO, |sum, corner| sum + corner.normal.as_dvec3())
        .normalize_or_zero();
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
    fn of(mesh: &ShadedMesh, analysis: FaceAnalysis) -> Self {
        let division = mesh.divide(|corners| analysis.classify(corners));
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

    pub fn of(&self, mesh: &Arc<ShadedMesh>, analysis: FaceAnalysis) -> Outcome {
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
        if mesh.triangle_count() <= self.inline_triangles {
            let analysed = Arc::new(Analysed::of(mesh, analysis));
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
        self.spawn(Arc::clone(mesh), weak, analysis);
        Outcome::Working
    }

    fn spawn(&self, mesh: Arc<ShadedMesh>, weak: Weak<ShadedMesh>, analysis: FaceAnalysis) {
        let entries = Arc::clone(&self.entries);
        let finished = Arc::clone(&self.finished);
        let wake = Arc::clone(&self.wake);
        let spawned = thread::Builder::new()
            .name("analysis".to_owned())
            .spawn(move || {
                let analysed = panic::catch_unwind(AssertUnwindSafe(|| {
                    Arc::new(Analysed::of(&mesh, analysis))
                }));
                if analysed.is_err() {
                    log::error!("analysing the faces of a body panicked");
                }
                drop(mesh);
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

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };

    use caditor_document::{CancelToken, ModelEvaluator, Recompute};

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
        .map(|normal| Band::of_class(analysis.classify(flat(normal))));

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
            Band::of_class(up.classify(flat(Vector3::Z))),
            Some(Band::Drafted)
        );
        assert_eq!(
            Band::of_class(down.classify(flat(Vector3::Z))),
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

        assert_eq!(limit(5.0).classify(arc(4.0, false)), Band::TooTight.class());
        assert_eq!(limit(3.0).classify(arc(4.0, false)), 0);
        assert_eq!(limit(4.0).classify(arc(4.0, false)), 0);
        assert_eq!(limit(5.0).classify(arc(4.0, true)), 0);
        assert_eq!(limit(5.0).classify(flat(Vector3::Z)), 0);
    }

    #[test]
    fn classes_round_trip_through_their_bands_and_zero_is_no_band() {
        assert_eq!(Band::of_class(0), None);
        for band in [
            Band::Drafted,
            Band::TooLittleDraft,
            Band::Undercut,
            Band::TooTight,
        ] {
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
