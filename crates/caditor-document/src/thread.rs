use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_geometry::{Plane, Point3, Vector3};
use caditor_kernel::{
    BooleanOperation, FaceCopy, FaceId, FaceOrigin, FaceReference, GROOVE_PARTS, Handedness,
    MAX_SIZE, MAX_THREAD_TURNS, RING_ENTITY, Solid, Surface, ThreadEnd, ThreadError, ThreadSpec,
    ThreadedFace, boolean, thread_tools, thread_tools_numbered,
};

use crate::{
    describe::{describe_origin, edge_faces, lowercase_first},
    document::{Document, Feature, FeatureId},
    hole::{Hole, HoleShape},
    hole_standard::pitch_text,
    origins,
    pieces::{Resolution, pieces_of_one_face},
    recompute::{
        CancelToken, Evaluation, Failure, FeatureError, FeatureResult, FeatureState, FixTarget,
        Inputs,
    },
    solid::SolidResult,
    thread_standard::{ThreadClass, ThreadDesignation, ThreadHand, ThreadSide, ThreadSize},
    trouble,
    values::ParameterValues,
};

const EDGE_SAMPLES: usize = 16;
const END_TOLERANCE: f64 = 1e-6;
const FACING_OUT: f64 = 1e-3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ThreadForm {
    Modelled,
    #[default]
    Cosmetic,
}

impl ThreadForm {
    pub const ALL: [Self; 2] = [Self::Modelled, Self::Cosmetic];

    pub fn label(self) -> &'static str {
        match self {
            Self::Modelled => "Modelled",
            Self::Cosmetic => "Cosmetic",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThreadLength {
    Full,
    Depth(Expression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    pub body: FeatureId,
    pub face: FaceReference,
    pub size: ThreadSize,
    pub class: ThreadClass,
    pub hand: ThreadHand,
    pub length: ThreadLength,
    pub reversed: bool,
    pub form: ThreadForm,
}

impl Thread {
    pub fn is_modelled(&self) -> bool {
        self.form == ThreadForm::Modelled && self.size.family().can_be_modelled()
    }

    pub fn designation(&self) -> ThreadDesignation {
        ThreadDesignation {
            size: self.size,
            class: self.class,
            hand: self.hand,
        }
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        match &self.length {
            ThreadLength::Full => BTreeSet::new(),
            ThreadLength::Depth(depth) => depth.parameters().into_iter().collect(),
        }
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        match &self.length {
            ThreadLength::Full => false,
            ThreadLength::Depth(depth) => depth.uses(parameter),
        }
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        BTreeSet::from([self.body])
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        origins::of_face(&self.face)
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        match &mut self.length {
            ThreadLength::Full => Vec::new(),
            ThreadLength::Depth(depth) => vec![depth],
        }
    }

    pub fn heap_size(&self) -> usize {
        self.face.heap_size()
            + match &self.length {
                ThreadLength::Full => 0,
                ThreadLength::Depth(depth) => depth.heap_size(),
            }
    }

    pub fn resolution(&self, solid: &Solid) -> Resolution<FaceId> {
        Resolution::of(self.face.resolve(solid), |pieces| {
            pieces_of_one_face(solid, pieces)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadPlacement {
    pub start: Point3,
    pub direction: Vector3,
    pub length: f64,
    pub radius: f64,
    pub thread_radius: f64,
}

impl ThreadPlacement {
    pub fn end(&self) -> Point3 {
        self.start + self.direction * self.length
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadResult {
    pub side: ThreadSide,
    pub designation: String,
    pub depth: Option<f64>,
    pub placement: ThreadPlacement,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedThread {
    pub feature: FeatureId,
    pub body: FeatureId,
    pub designation: String,
    pub side: ThreadSide,
    pub pitch: f64,
    pub placement: ThreadPlacement,
    pub modelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bore {
    pub side: ThreadSide,
    pub diameter: f64,
    origin: Point3,
    axis: Vector3,
    low: f64,
    high: f64,
    open_low: bool,
    open_high: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoreError {
    NotCylinder(FaceId),
    NoExtent,
}

impl Bore {
    pub fn of(solid: &Solid, faces: &[FaceId]) -> Result<Self, BoreError> {
        let first = faces.first().copied().ok_or(BoreError::NoExtent)?;
        let face = solid.face(first).ok_or(BoreError::NoExtent)?;
        let Surface::Cylinder(cylinder) = face.surface() else {
            return Err(BoreError::NotCylinder(first));
        };
        let frame = cylinder.frame();
        let origin = frame.origin();
        let axis = frame.normal().normalize_or_zero();
        for other in faces {
            let shape = solid.face(*other).map(|face| face.surface());
            if !matches!(shape, Some(Surface::Cylinder(_))) {
                return Err(BoreError::NotCylinder(*other));
            }
        }
        let side = if face.sense().is_same() {
            ThreadSide::External
        } else {
            ThreadSide::Internal
        };
        let along = |point: Point3| (point - origin).dot(axis);
        let edges = face_edges(solid, faces);
        let samples: Vec<(usize, f64)> = edges
            .iter()
            .enumerate()
            .flat_map(|(index, (_, points))| points.iter().map(move |point| (index, along(*point))))
            .collect();
        let low = samples
            .iter()
            .map(|(_, at)| *at)
            .fold(f64::INFINITY, f64::min);
        let high = samples
            .iter()
            .map(|(_, at)| *at)
            .fold(f64::NEG_INFINITY, f64::max);
        if !low.is_finite() || !high.is_finite() || high - low <= END_TOLERANCE {
            return Err(BoreError::NoExtent);
        }
        let reach = END_TOLERANCE.max((high - low) * 1e-6);
        let mut open_low = false;
        let mut open_high = false;
        for (edge, points) in &edges {
            let ats: Vec<f64> = points.iter().map(|point| along(*point)).collect();
            let at_low = ats.iter().all(|at| (at - low).abs() <= reach);
            let at_high = ats.iter().all(|at| (at - high).abs() <= reach);
            if !at_low && !at_high {
                continue;
            }
            let outward = if at_high { axis } else { -axis };
            let Some(middle) = points.get(points.len() / 2).copied() else {
                continue;
            };
            let facing_out = edge_faces(solid, *edge)
                .into_iter()
                .filter(|neighbour| !faces.contains(neighbour))
                .filter_map(|neighbour| face_normal(solid, neighbour, middle))
                .any(|normal| normal.dot(outward) > FACING_OUT);
            if facing_out {
                if at_high {
                    open_high = true;
                } else {
                    open_low = true;
                }
            }
        }
        Ok(Self {
            side,
            diameter: 2.0 * cylinder.radius(),
            origin,
            axis,
            low,
            high,
            open_low,
            open_high,
        })
    }

    pub fn length(&self) -> f64 {
        self.high - self.low
    }

    fn starts_high(&self, reversed: bool) -> bool {
        (self.open_high || !self.open_low) != reversed
    }

    fn ends(&self, depth: Option<f64>, reversed: bool) -> (ThreadEnd, ThreadEnd) {
        let (start_open, far_open) = if self.starts_high(reversed) {
            (self.open_high, self.open_low)
        } else {
            (self.open_low, self.open_high)
        };
        let whole = depth.is_none_or(|depth| depth >= self.length() - END_TOLERANCE);
        let finish = |open: bool| {
            if open {
                ThreadEnd::RunsOut
            } else {
                ThreadEnd::Stops
            }
        };
        (finish(start_open), finish(whole && far_open))
    }

    pub fn modelled_spec(
        &self,
        size: ThreadSize,
        hand: ThreadHand,
        depth: Option<f64>,
        reversed: bool,
    ) -> Option<ThreadSpec> {
        let form = size.modelled_profile(self.side)?;
        let placement = self.placement(depth, reversed, size.drawn_diameter(self.side));
        let frame = Plane::from_frame(
            placement.start,
            placement.direction,
            placement.direction.any_orthonormal_vector(),
        )?;
        let (start, end) = self.ends(depth, reversed);
        Some(ThreadSpec {
            frame,
            face: match self.side {
                ThreadSide::Internal => ThreadedFace::Bore,
                ThreadSide::External => ThreadedFace::Shaft,
            },
            face_radius: self.diameter / 2.0,
            form,
            hand: match hand {
                ThreadHand::Right => Handedness::Right,
                ThreadHand::Left => Handedness::Left,
            },
            length: placement.length,
            start,
            end,
        })
    }

    pub fn placement(
        &self,
        depth: Option<f64>,
        reversed: bool,
        thread_diameter: f64,
    ) -> ThreadPlacement {
        let from_high = self.starts_high(reversed);
        let (at, direction) = if from_high {
            (self.high, -self.axis)
        } else {
            (self.low, self.axis)
        };
        let full = self.length();
        ThreadPlacement {
            start: self.origin + self.axis * at,
            direction,
            length: depth.map_or(full, |depth| depth.min(full)),
            radius: self.diameter / 2.0,
            thread_radius: thread_diameter / 2.0,
        }
    }
}

fn face_edges(solid: &Solid, faces: &[FaceId]) -> Vec<(caditor_kernel::EdgeId, Vec<Point3>)> {
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();
    for face in faces {
        let Some(definition) = solid.face(*face) else {
            continue;
        };
        for loop_id in definition.loops() {
            let Some(face_loop) = solid.face_loop(*loop_id) else {
                continue;
            };
            for coedge in face_loop.coedges() {
                let Some(edge_id) = solid.coedge(*coedge).map(|coedge| coedge.edge()) else {
                    continue;
                };
                if !seen.insert(edge_id) {
                    continue;
                }
                let Some(edge) = solid.edge(edge_id) else {
                    continue;
                };
                let interval = edge.interval();
                let points = (0..=EDGE_SAMPLES)
                    .map(|step| {
                        let fraction = step as f64 / EDGE_SAMPLES as f64;
                        edge.curve().point(interval.at(fraction))
                    })
                    .collect();
                edges.push((edge_id, points));
            }
        }
    }
    edges
}

fn face_normal(solid: &Solid, face: FaceId, point: Point3) -> Option<Vector3> {
    let face = solid.face(face)?;
    let surface = face.surface();
    let foot = surface.project(point, None);
    let normal = surface.normal(foot.x, foot.y)?;
    Some(normal * face.sense().sign())
}

struct Context<'a> {
    feature: &'a Feature,
    definition: &'a Thread,
    inputs: &'a Inputs<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: String) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn depth(&self, depth: &Expression) -> Result<f64, Failure> {
        let value = depth
            .evaluate_as(Dimension::LENGTH, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the depth."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } => (
                        "Edit the depth so it gives a length, such as 10 mm.".to_owned(),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    _ => (
                        "Edit the depth or the parameters it uses.".to_owned(),
                        FixTarget::Feature(self.feature.id()),
                    ),
                };
                Failure::Error(Box::new(FeatureError {
                    reason: format!("The thread's depth cannot be evaluated: {error}."),
                    remedy,
                    fix: Some(fix),
                    constraints: Vec::new(),
                    place: None,
                }))
            })?;
        if value.is_finite() && value > 0.0 {
            Ok(value)
        } else {
            Err(self.error(
                "The thread's depth must be more than 0 mm.".to_owned(),
                "Enter a larger depth, or let the thread run the whole face.".to_owned(),
            ))
        }
    }

    fn describe_face(&self, solid: &Solid, face: FaceId) -> String {
        describe_origin(
            self.inputs.document,
            solid.face(face).and_then(|face| face.origin()),
        )
    }

    fn unresolved(&self, resolution: &Resolution<FaceId>) -> Failure {
        let body = &self.body_name;
        let reason = match resolution {
            Resolution::Tied(_) => {
                format!(
                    "The threaded face now matches several separate faces of the body of {body}."
                )
            }
            Resolution::One(_) | Resolution::Pieces(_) | Resolution::Missing => {
                format!("The threaded face is no longer part of the body of {body}.")
            }
        };
        self.error(
            reason,
            "Choose the face to thread again, or undo the change that removed it.".to_owned(),
        )
    }

    fn not_round(&self, solid: &Solid, face: FaceId) -> Failure {
        self.error(
            format!(
                "{} is not cylindrical, so it cannot carry a thread.",
                crate::datum::capitalized(&self.describe_face(solid, face))
            ),
            "Choose a cylindrical face: a bore, a shaft or a boss.".to_owned(),
        )
    }

    fn wrong_class(&self, side: ThreadSide) -> Failure {
        let family = self.definition.size.family();
        let (what, wanted) = match side {
            ThreadSide::Internal => ("a hole", "an internal thread"),
            ThreadSide::External => ("a shaft", "an external thread"),
        };
        let class = self.definition.class.id();
        let suggestion = family.default_class(side).id();
        let remedy = if suggestion.is_empty() {
            format!("Choose the class for {wanted}.")
        } else {
            format!("Choose a class for {wanted}, such as {suggestion}.")
        };
        self.error(
            format!(
                "The threaded face is {what}, but the class {class} is not one {} offers for \
                 {wanted}.",
                family.label()
            ),
            remedy,
        )
    }

    fn misfit(&self, bore: &Bore) -> Failure {
        let size = self.definition.size;
        let side = bore.side;
        let what = match side {
            ThreadSide::Internal => "bore",
            ThreadSide::External => "shaft",
        };
        let across = millimetres(bore.diameter);
        let (too, limit, limit_name) = if bore.diameter >= size.major_diameter(side) {
            ("wide", size.major_diameter(side), "major")
        } else {
            ("narrow", size.minor_diameter(side), "minor")
        };
        let reason = format!(
            "The {what} is {across} across, too {too} for the {} thread, whose {limit_name} \
             diameter is {}.",
            size.label(),
            millimetres(limit)
        );
        let nearest = size.family().nearest(bore.diameter, side);
        let remedy = if nearest.fits(bore.diameter, side) {
            format!(
                "Choose {} instead, or change the {what}'s diameter.",
                nearest.label()
            )
        } else {
            format!("Choose a thread of another standard, or change the {what}'s diameter.")
        };
        self.error(reason, remedy)
    }
}

fn millimetres(value: f64) -> String {
    format!("{} mm", pitch_text(value))
}

pub(crate) fn cut_thread(
    body: &Solid,
    spec: &ThreadSpec,
    feature: u64,
    numbered: Option<&dyn Fn(u64) -> u64>,
) -> Result<(Solid, Vec<Solid>), ThreadError> {
    let tools = match numbered {
        Some(number) => thread_tools_numbered(spec, feature, number)?,
        None => thread_tools(spec, feature)?,
    };
    let mut threaded = body.clone();
    for tool in &tools {
        threaded = boolean(&threaded, tool, BooleanOperation::Difference)?;
    }
    Ok((threaded, tools))
}

pub(crate) struct ThreadWords<'a> {
    pub size: ThreadSize,
    pub side: ThreadSide,
    pub face: &'a str,
}

pub(crate) struct Worded {
    pub reason: String,
    pub remedy: String,
    pub place: Option<Point3>,
}

impl ThreadWords<'_> {
    pub fn words(&self, error: &ThreadError) -> Option<Worded> {
        let size = self.size.label();
        let face = self.face;
        let cosmetic = "or make the thread cosmetic";
        let worded = |reason: String, remedy: String| Worded {
            reason,
            remedy,
            place: None,
        };
        Some(match error {
            ThreadError::Cancelled(_) => return None,
            ThreadError::TooShort { length, needed } => worded(
                format!(
                    "The modelled {size} thread on {face} is {} long, shorter than the {} its \
                     form needs.",
                    millimetres(*length),
                    millimetres(*needed)
                ),
                format!("Thread a longer length or choose a finer pitch, {cosmetic}."),
            ),
            ThreadError::TooManyTurns { turns } => worded(
                format!(
                    "The modelled {size} thread on {face} would turn {turns:.0} times, more than \
                     the {MAX_THREAD_TURNS} a modelled thread can."
                ),
                format!("Thread a shorter length, {cosmetic}."),
            ),
            ThreadError::TooLarge => worded(
                format!(
                    "The modelled {size} thread on {face} would reach farther than {MAX_SIZE} mm \
                     from the origin."
                ),
                format!("Move the body nearer the origin, {cosmetic}."),
            ),
            ThreadError::RootOutsideFace => {
                let (what, too) = match self.side {
                    ThreadSide::Internal => ("bore", "wide"),
                    ThreadSide::External => ("shaft", "narrow"),
                };
                worded(
                    format!(
                        "The {what} at {face} is too {too} for the root of a modelled {size} \
                         thread, so there is nothing to cut."
                    ),
                    format!("Choose another size or change the {what}'s diameter, {cosmetic}."),
                )
            }
            ThreadError::NonFinite
            | ThreadError::NoPitch
            | ThreadError::FlankAngle
            | ThreadError::NoRootWidth
            | ThreadError::GrooveOverlaps
            | ThreadError::CrestBeyondRoot => worded(
                format!("The form of the {size} thread on {face} cannot be modelled."),
                format!("Choose another size, {cosmetic}."),
            ),
            ThreadError::Geometry(_)
            | ThreadError::GrooveUnassembled
            | ThreadError::Groove(_)
            | ThreadError::Ring(_) => worded(
                format!("The groove of the modelled {size} thread on {face} could not be built."),
                format!("Change the thread's length or size slightly, {cosmetic}."),
            ),
            ThreadError::Cut(error) => Worded {
                reason: format!(
                    "The modelled {size} thread could not be cut into {face}: where it meets \
                     the faces around it, the result would not be a valid solid."
                ),
                remedy: format!(
                    "Change the thread's length slightly or move what touches its ends, \
                     {cosmetic}."
                ),
                place: trouble::place(error),
            },
        })
    }
}

pub(crate) fn part_name(part: u64) -> &'static str {
    if part >= RING_ENTITY {
        return "thread crest";
    }
    let (_, side) = GROOVE_PARTS.turn_of(part);
    match side {
        side if side == GROOVE_PARTS.root => "thread root",
        side if side == GROOVE_PARTS.leading_flank || side == GROOVE_PARTS.trailing_flank => {
            "thread flank"
        }
        side if side == GROOVE_PARTS.far => "thread groove",
        _ => "thread end",
    }
}

fn root_turn(part: u64) -> Option<u64> {
    if part >= RING_ENTITY {
        return None;
    }
    let (turn, side) = GROOVE_PARTS.turn_of(part);
    (side == GROOVE_PARTS.root).then_some(turn)
}

fn mean_along(solid: &Solid, faces: &[FaceId], along: &dyn Fn(Point3) -> f64) -> Option<f64> {
    let samples: Vec<f64> = face_edges(solid, faces)
        .into_iter()
        .flat_map(|(_, points)| points)
        .map(along)
        .collect();
    (!samples.is_empty()).then(|| samples.iter().sum::<f64>() / samples.len() as f64)
}

fn modelled_placement(
    solid: &Solid,
    faces: &[(FaceId, u64)],
    size: ThreadSize,
) -> Option<(ThreadSide, ThreadPlacement)> {
    let roots: Vec<(FaceId, u64)> = faces
        .iter()
        .filter_map(|(face, part)| Some((*face, root_turn(*part)?)))
        .collect();
    let (first_root, _) = roots.first()?;
    let root = solid.face(*first_root)?;
    let Surface::Cylinder(cylinder) = root.surface() else {
        return None;
    };
    let side = if root.sense().is_same() {
        ThreadSide::External
    } else {
        ThreadSide::Internal
    };
    let frame = cylinder.frame();
    let origin = frame.origin();
    let axis = frame.normal().normalize_or_zero();
    let along = |point: Point3| (point - origin).dot(axis);
    let ids: Vec<FaceId> = faces.iter().map(|(face, _)| *face).collect();
    let extent: Vec<f64> = face_edges(solid, &ids)
        .into_iter()
        .flat_map(|(_, points)| points)
        .map(along)
        .collect();
    let low = extent.iter().copied().fold(f64::INFINITY, f64::min);
    let high = extent.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !low.is_finite() || !high.is_finite() || high - low <= END_TOLERANCE {
        return None;
    }
    let turns = roots.iter().map(|(_, turn)| *turn);
    let first_turn = turns.clone().min()?;
    let last_turn = turns.max()?;
    let at_turn = |wanted: u64| -> Vec<FaceId> {
        roots
            .iter()
            .filter(|(_, turn)| *turn == wanted)
            .map(|(face, _)| *face)
            .collect()
    };
    let starts_low = first_turn == last_turn
        || mean_along(solid, &at_turn(first_turn), &along)
            <= mean_along(solid, &at_turn(last_turn), &along);
    let (at, direction) = if starts_low {
        (low, axis)
    } else {
        (high, -axis)
    };
    Some((
        side,
        ThreadPlacement {
            start: origin + axis * at,
            direction,
            length: high - low,
            radius: cylinder.radius(),
            thread_radius: size.drawn_diameter(side) / 2.0,
        },
    ))
}

type ThreadFaces = BTreeMap<(Option<FaceCopy>, u64), Vec<(FaceId, u64)>>;

fn thread_faces(solid: &Solid, part: &dyn Fn(FaceOrigin) -> Option<(u64, u64)>) -> ThreadFaces {
    let mut grouped = ThreadFaces::new();
    for (id, face) in solid.faces() {
        let Some(origin) = face.origin() else {
            continue;
        };
        if let Some((group, part)) = part(origin.original()) {
            grouped
                .entry((origin.copy(), group))
                .or_default()
                .push((id, part));
        }
    }
    grouped
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Thread,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let body_name = inputs
        .document
        .feature(definition.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let context = Context {
        feature,
        definition,
        inputs,
        body_name,
    };
    let depth = match &definition.length {
        ThreadLength::Full => None,
        ThreadLength::Depth(depth) => Some(context.depth(depth)?),
    };
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let resolution = definition.resolution(solid);
    let faces = match &resolution {
        Resolution::One(face) => vec![*face],
        Resolution::Pieces(pieces) => pieces.clone(),
        Resolution::Tied(_) | Resolution::Missing => return Err(context.unresolved(&resolution)),
    };
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let bore = Bore::of(solid, &faces).map_err(|error| match error {
        BoreError::NotCylinder(face) => context.not_round(solid, face),
        BoreError::NoExtent => context.unresolved(&Resolution::Missing),
    })?;
    let side = bore.side;
    if !definition
        .size
        .family()
        .classes(side)
        .contains(&definition.class)
    {
        return Err(context.wrong_class(side));
    }
    if !definition.size.fits(bore.diameter, side) {
        return Err(context.misfit(&bore));
    }
    if definition.is_modelled() {
        let face = faces.first().map_or_else(
            || "the threaded face".to_owned(),
            |face| lowercase_first(&context.describe_face(solid, *face)),
        );
        let Some(spec) =
            bore.modelled_spec(definition.size, definition.hand, depth, definition.reversed)
        else {
            return Err(context.error(
                format!(
                    "The {} thread on {face} cannot be modelled.",
                    definition.size.label()
                ),
                "Make the thread cosmetic.".to_owned(),
            ));
        };
        let worded = ThreadWords {
            size: definition.size,
            side,
            face: &face,
        };
        let (threaded, tools) =
            cut_thread(solid, &spec, feature.id().raw(), None).map_err(|error| {
                match worded.words(&error) {
                    None => Failure::Cancelled,
                    Some(words) => context
                        .error(words.reason, words.remedy)
                        .placed(words.place),
                }
            })?;
        return Ok(FeatureResult::Solid(
            SolidResult::new(definition.body, threaded).cutting(tools),
        ));
    }
    Ok(FeatureResult::Thread(ThreadResult {
        side,
        designation: definition.designation().text(),
        depth,
        placement: bore.placement(
            depth,
            definition.reversed,
            definition.size.drawn_diameter(side),
        ),
    }))
}

fn placeable(state: &FeatureState) -> bool {
    matches!(state, FeatureState::UpToDate | FeatureState::Outdated)
}

pub fn placed_threads(document: &Document, evaluation: &Evaluation) -> Vec<PlacedThread> {
    let standing: Vec<(FeatureId, &Solid)> = evaluation
        .bodies()
        .filter_map(|(body, _)| Some((body, evaluation.body(body)?)))
        .collect();
    let mut placed = Vec::new();
    for feature in document.active_features() {
        let Some(status) = evaluation.feature(feature.id()) else {
            continue;
        };
        if !placeable(&status.state) {
            continue;
        }
        let result = status.result.as_deref();
        if let (Some(thread), Some(FeatureResult::Thread(evaluated))) =
            (feature.kind.thread(), result)
        {
            placed.extend(place_thread(feature.id(), thread, evaluated, &standing));
        } else if let (Some(thread), Some(FeatureResult::Solid(_))) =
            (feature.kind.thread(), result)
        {
            placed.extend(place_modelled(feature.id(), thread, &standing));
        } else if let Some(hole) = feature.kind.hole() {
            placed.extend(hole_threads(
                feature.id(),
                hole,
                &standing,
                &evaluation.parameters,
            ));
        }
    }
    placed
}

fn place_modelled(
    feature: FeatureId,
    thread: &Thread,
    standing: &[(FeatureId, &Solid)],
) -> Vec<PlacedThread> {
    let raw = feature.raw();
    let own = |origin: FaceOrigin| match origin {
        FaceOrigin::Side {
            feature: maker,
            entity,
        } if maker == raw => Some((0, entity)),
        _ => None,
    };
    let designation = thread.designation().text();
    let mut placed = Vec::new();
    for (body, solid) in standing {
        for faces in thread_faces(solid, &own).values() {
            if let Some((side, placement)) = modelled_placement(solid, faces, thread.size) {
                placed.push(PlacedThread {
                    feature,
                    body: *body,
                    designation: designation.clone(),
                    side,
                    pitch: thread.size.pitch(),
                    placement,
                    modelled: true,
                });
            }
        }
    }
    placed
}

fn place_thread(
    feature: FeatureId,
    thread: &Thread,
    evaluated: &ThreadResult,
    standing: &[(FeatureId, &Solid)],
) -> Option<PlacedThread> {
    let own = standing.iter().filter(|(body, _)| *body == thread.body);
    let others = standing.iter().filter(|(body, _)| *body != thread.body);
    own.chain(others).find_map(|(body, solid)| {
        let faces = match thread.resolution(solid) {
            Resolution::One(face) => vec![face],
            Resolution::Pieces(pieces) => pieces,
            Resolution::Tied(_) | Resolution::Missing => return None,
        };
        let bore = Bore::of(solid, &faces).ok()?;
        Some(PlacedThread {
            feature,
            body: *body,
            designation: evaluated.designation.clone(),
            side: bore.side,
            pitch: thread.size.pitch(),
            placement: bore.placement(
                evaluated.depth,
                thread.reversed,
                thread.size.drawn_diameter(bore.side),
            ),
            modelled: false,
        })
    })
}

pub fn hole_thread(hole: &Hole) -> Option<ThreadDesignation> {
    if !matches!(hole.shape, HoleShape::Round) {
        return None;
    }
    let size = ThreadSize::of_hole(hole.standard?)?;
    Some(ThreadDesignation {
        size,
        class: hole
            .thread
            .class
            .unwrap_or_else(|| size.family().default_class(ThreadSide::Internal)),
        hand: hole.thread.hand,
    })
}

fn hole_wall(feature: FeatureId, origin: FaceOrigin) -> Option<(Option<FaceCopy>, u64)> {
    let FaceOrigin::Side {
        feature: maker,
        entity,
    } = origin.original()
    else {
        return None;
    };
    (maker == feature.raw() && Hole::is_wall(entity)).then_some((origin.copy(), entity))
}

fn hole_threads(
    feature: FeatureId,
    hole: &Hole,
    standing: &[(FeatureId, &Solid)],
    parameters: &ParameterValues,
) -> Vec<PlacedThread> {
    let Some(designation) = hole_thread(hole) else {
        return Vec::new();
    };
    let depth = match &hole.thread.depth {
        None => None,
        Some(depth) => match depth.evaluate_as(Dimension::LENGTH, &|id| parameters.value(id)) {
            Ok(value) if value.is_finite() && value > 0.0 => Some(value),
            Ok(_) | Err(_) => return Vec::new(),
        },
    };
    let text = designation.text();
    let raw = feature.raw();
    let own = |origin: FaceOrigin| match origin {
        FaceOrigin::Side {
            feature: maker,
            entity,
        } if maker == raw => Hole::thread_part(entity),
        _ => None,
    };
    let mut placed = Vec::new();
    for (body, solid) in standing {
        let modelled = thread_faces(solid, &own);
        let mut walls: BTreeMap<(Option<FaceCopy>, u64), Vec<FaceId>> = BTreeMap::new();
        for (id, face) in solid.faces() {
            if let Some(wall) = face.origin().and_then(|origin| hole_wall(feature, origin)) {
                walls.entry(wall).or_default().push(id);
            }
        }
        for faces in modelled.values() {
            let Some((side, placement)) = modelled_placement(solid, faces, designation.size) else {
                continue;
            };
            placed.push(PlacedThread {
                feature,
                body: *body,
                designation: text.clone(),
                side,
                pitch: designation.size.pitch(),
                placement,
                modelled: true,
            });
        }
        for (wall, faces) in &walls {
            if modelled.contains_key(wall) {
                continue;
            }
            let Ok(bore) = Bore::of(solid, faces) else {
                continue;
            };
            placed.push(PlacedThread {
                feature,
                body: *body,
                designation: text.clone(),
                side: bore.side,
                pitch: designation.size.pitch(),
                placement: bore.placement(depth, false, designation.size.drawn_diameter(bore.side)),
                modelled: false,
            });
        }
    }
    placed
}
