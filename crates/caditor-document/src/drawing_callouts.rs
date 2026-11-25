use std::collections::BTreeMap;

use caditor_expression::{Dimension, Expression, format_decimals};
use caditor_geometry::{Point2, Vector2};
use caditor_kernel::{EdgeName, FaceId, FaceOrigin, Solid};
use caditor_sketch::{EntityId, annotation::arrowhead};
use thiserror::Error;

use crate::{
    describe::edge_faces,
    document::{Document, FeatureId},
    drawing_dimensions::{
        AnnotationProblem, DrawnLines, SheetAnnotation, lines_of, note_on_sheet, seen_edge,
    },
    drawing_sheet::{SheetMark, SheetPlan, SheetText, TextAnchor, text_width},
    drawing_views::{DrawnBodies, drawn_bodies},
    drawings::{Annotation, AnnotationId, Drawing, DrawingCallout},
    hole::{Hole, HoleDepth, HoleShape, HoleSizing, HoleStyle, circle_sizes},
    recompute::Evaluation,
    thread::{Thread, ThreadLength, hole_thread},
    values::ParameterValues,
};

pub const CALLOUT_TEXT_HEIGHT: f64 = 3.5;
const LINE_PITCH: f64 = 1.6;
const SYMBOL_GAP: f64 = 0.15;
const WORD_GAP: f64 = 0.35;
const CALLOUT_DECIMALS: usize = 2;
const ANGLE_DECIMALS: usize = 1;
const LEADER_SHOULDER: f64 = 1.0;
const SHOULDER_GAP: f64 = 0.5;
const UNKNOWN: &str = "?";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutSymbol {
    Diameter,
    Depth,
    Counterbore,
    Countersink,
}

impl CalloutSymbol {
    pub fn character(self) -> char {
        match self {
            Self::Diameter => '\u{2300}',
            Self::Depth => '\u{21a7}',
            Self::Counterbore => '\u{2334}',
            Self::Countersink => '\u{2335}',
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Self::Diameter => "diameter",
            Self::Depth => "depth",
            Self::Counterbore => "counterbore",
            Self::Countersink => "countersink",
        }
    }

    fn width(self) -> f64 {
        match self {
            Self::Diameter => 0.9,
            Self::Depth => 0.6,
            Self::Counterbore | Self::Countersink => 0.8,
        }
    }

    fn marks(self, origin: Point2, height: f64) -> Vec<SheetMark> {
        let at = |x: f64, y: f64| origin + Vector2::new(x, y) * height;
        let polyline = |points: &[(f64, f64)]| -> Vec<SheetMark> {
            points
                .windows(2)
                .filter_map(|pair| match pair {
                    [(x0, y0), (x1, y1)] => Some(SheetMark::Line(at(*x0, *y0), at(*x1, *y1))),
                    _ => None,
                })
                .collect()
        };

        match self {
            Self::Diameter => vec![
                SheetMark::Circle {
                    center: at(0.45, 0.5),
                    radius: 0.36 * height,
                },
                SheetMark::Line(at(0.1, 0.0), at(0.8, 1.0)),
            ],
            Self::Depth => {
                let mut marks = polyline(&[(0.3, 1.0), (0.3, 0.0)]);

                marks.extend(polyline(&[(0.08, 0.35), (0.3, 0.0), (0.52, 0.35)]));
                marks.extend(polyline(&[(0.0, 0.0), (0.6, 0.0)]));
                marks
            }
            Self::Counterbore => polyline(&[(0.0, 0.7), (0.0, 0.0), (0.8, 0.0), (0.8, 0.7)]),
            Self::Countersink => polyline(&[(0.0, 0.7), (0.4, 0.0), (0.8, 0.7)]),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CalloutPiece {
    Text(String),
    Symbol(CalloutSymbol),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CalloutLine(pub Vec<CalloutPiece>);

fn gap_before(previous: Option<&CalloutPiece>, piece: &CalloutPiece) -> f64 {
    match (previous, piece) {
        (None, _) => 0.0,
        (Some(CalloutPiece::Text(_)), CalloutPiece::Symbol(_)) => WORD_GAP,
        (Some(_), _) => SYMBOL_GAP,
    }
}

impl CalloutLine {
    fn written(&self, symbol: impl Fn(CalloutSymbol) -> String) -> String {
        let mut written = String::new();
        let mut previous: Option<&CalloutPiece> = None;

        for piece in &self.0 {
            if matches!(
                (previous, piece),
                (Some(CalloutPiece::Text(_)), CalloutPiece::Symbol(_))
            ) {
                written.push(' ');
            }
            match piece {
                CalloutPiece::Text(text) => written.push_str(text),
                CalloutPiece::Symbol(found) => written.push_str(&symbol(*found)),
            }
            previous = Some(piece);
        }
        written
    }

    pub fn text(&self) -> String {
        self.written(|symbol| symbol.character().to_string())
    }

    pub fn spoken(&self) -> String {
        self.written(|symbol| format!("{} ", symbol.words()))
    }

    fn width(&self, height: f64) -> f64 {
        let mut width = 0.0;
        let mut previous = None;

        for piece in &self.0 {
            width += gap_before(previous, piece) * height
                + match piece {
                    CalloutPiece::Text(text) => text_width(text, height),
                    CalloutPiece::Symbol(symbol) => symbol.width() * height,
                };
            previous = Some(piece);
        }
        width
    }

    fn laid(&self, left: Point2, height: f64, marks: &mut Vec<SheetMark>) -> Vec<SheetText> {
        let mut pen = left;
        let mut previous = None;
        let mut texts = Vec::new();

        for piece in &self.0 {
            pen += Vector2::new(gap_before(previous, piece) * height, 0.0);
            match piece {
                CalloutPiece::Text(text) => {
                    texts.push(SheetText {
                        at: pen,
                        height,
                        content: text.clone(),
                        anchor: TextAnchor::Start,
                        angle: 0.0,
                    });
                    pen += Vector2::new(text_width(text, height), 0.0);
                }
                CalloutPiece::Symbol(symbol) => {
                    marks.extend(symbol.marks(pen, height));
                    pen += Vector2::new(symbol.width() * height, 0.0);
                }
            }
            previous = Some(piece);
        }
        texts
    }
}

pub type CalloutTexts = BTreeMap<AnnotationId, Result<Vec<CalloutLine>, AnnotationProblem>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum CalloutRefusal {
    #[error("that edge belongs to no hole or threaded face; click an edge of a hole or a thread")]
    NotAHole,
}

fn measure(value: f64) -> String {
    format_decimals(value, CALLOUT_DECIMALS)
}

fn with_depth(mut pieces: Vec<CalloutPiece>, depth: Option<f64>) -> CalloutLine {
    if let Some(depth) = depth {
        pieces.extend([
            CalloutPiece::Symbol(CalloutSymbol::Depth),
            CalloutPiece::Text(measure(depth)),
        ]);
    }
    CalloutLine(pieces)
}

fn diameter_pieces(diameter: f64) -> Vec<CalloutPiece> {
    vec![
        CalloutPiece::Symbol(CalloutSymbol::Diameter),
        CalloutPiece::Text(measure(diameter)),
    ]
}

struct Reader<'a> {
    parameters: &'a ParameterValues,
}

impl Reader<'_> {
    fn value(
        &self,
        expression: &Expression,
        dimension: Dimension,
    ) -> Result<f64, AnnotationProblem> {
        expression
            .evaluate_as(dimension, &|id| self.parameters.value(id))
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or(AnnotationProblem::CalloutValue)
    }

    fn length(&self, expression: &Expression) -> Result<f64, AnnotationProblem> {
        self.value(expression, Dimension::LENGTH)
    }

    fn thread(&self, thread: &Thread) -> Result<Vec<CalloutLine>, AnnotationProblem> {
        let depth = match &thread.length {
            ThreadLength::Full => None,
            ThreadLength::Depth(depth) => Some(self.length(depth)?),
        };
        Ok(vec![with_depth(
            vec![CalloutPiece::Text(thread.designation().text())],
            depth,
        )])
    }

    fn hole(
        &self,
        hole: &Hole,
        circle: Option<f64>,
    ) -> Result<Vec<CalloutLine>, AnnotationProblem> {
        let typed = self.length(&hole.diameter)?;
        let diameter = circle.unwrap_or(typed);
        let head = match (hole.sizing, circle) {
            (HoleSizing::CirclesAndHeads, Some(circle)) => circle / typed,
            _ => 1.0,
        };
        let depth = match &hole.depth {
            HoleDepth::Blind(depth) => Some(self.length(depth)?),
            HoleDepth::ThroughAll | HoleDepth::UpToNext { .. } | HoleDepth::UpToFace { .. } => None,
        };

        let bore = match &hole.shape {
            HoleShape::Round => with_depth(diameter_pieces(diameter), depth),
            HoleShape::Slot { length, .. } => {
                let across = format!(
                    "{} \u{d7} {}",
                    measure(diameter),
                    measure(self.length(length)? + diameter)
                );
                with_depth(vec![CalloutPiece::Text(across)], depth)
            }
        };
        let mut lines = Vec::new();

        if let Some(designation) = hole_thread(hole) {
            let thread_depth = match &hole.thread.depth {
                Some(thread_depth) => Some(self.length(thread_depth)?),
                None => depth,
            };

            lines.push(with_depth(
                vec![CalloutPiece::Text(designation.text())],
                thread_depth,
            ));
        }
        lines.push(bore);

        let counterbore = |wide: f64, deep: f64| {
            let mut pieces = vec![CalloutPiece::Symbol(CalloutSymbol::Counterbore)];

            pieces.extend(diameter_pieces(wide));
            with_depth(pieces, Some(deep))
        };

        match &hole.style {
            HoleStyle::Plain => {}
            HoleStyle::Counterbore {
                diameter: wide,
                depth: deep,
            } => lines.push(counterbore(
                self.length(wide)? * head,
                self.length(deep)? * head,
            )),
            HoleStyle::Stepped(steps) => {
                for step in steps {
                    lines.push(counterbore(
                        self.length(&step.diameter)? * head,
                        self.length(&step.depth)? * head,
                    ));
                }
            }
            HoleStyle::Countersink {
                diameter: wide,
                angle,
            } => {
                let angle = self.value(angle, Dimension::ANGLE)?;
                let mut pieces = vec![
                    CalloutPiece::Symbol(CalloutSymbol::Countersink),
                    CalloutPiece::Symbol(CalloutSymbol::Diameter),
                ];

                pieces.push(CalloutPiece::Text(format!(
                    "{} \u{d7} {}\u{b0}",
                    measure(self.length(wide)? * head),
                    format_decimals(angle, ANGLE_DECIMALS)
                )));
                lines.push(CalloutLine(pieces));
            }
        }
        Ok(lines)
    }
}

fn circle_diameter(document: &Document, hole: &Hole, point: Option<EntityId>) -> Option<f64> {
    if !hole.sizing.by_circles() {
        return None;
    }

    let sketch = document.feature(hole.sketch)?.kind.sketch()?;
    let sizes = circle_sizes(sketch);

    if let Some(size) = point.and_then(|point| sizes.get(&point)) {
        return Some(size.diameter);
    }

    let mut diameters = sizes.values().map(|size| size.diameter);

    let first = diameters.next()?;
    diameters.all(|other| other == first).then_some(first)
}

fn circles_differ(document: &Document, hole: &Hole) -> bool {
    hole.sizing.by_circles() && circle_diameter(document, hole, None).is_none()
}

pub fn callout_lines(
    document: &Document,
    parameters: &ParameterValues,
    feature: FeatureId,
    point: Option<EntityId>,
) -> Result<Vec<CalloutLine>, AnnotationProblem> {
    let found = document
        .feature(feature)
        .ok_or(AnnotationProblem::FeatureGone)?;
    let reader = Reader { parameters };

    if let Some(hole) = found.kind.hole() {
        return reader.hole(hole, circle_diameter(document, hole, point));
    }
    match found.kind.thread() {
        Some(thread) => reader.thread(thread),
        None => Err(AnnotationProblem::FeatureGone),
    }
}

fn named_edges<'a>(
    solids: &'a [&'a Solid],
    edge: EdgeName,
) -> impl Iterator<Item = (&'a Solid, Vec<FaceId>)> + 'a {
    solids.iter().flat_map(move |solid| {
        solid
            .edges()
            .filter(move |(_, found)| found.name() == edge)
            .map(move |(id, _)| (*solid, edge_faces(solid, id)))
    })
}

fn made_by(solid: &Solid, face: FaceId) -> Option<FaceOrigin> {
    solid
        .face(face)
        .and_then(|found| found.origin())
        .map(FaceOrigin::original)
}

fn hole_point(solids: &[&Solid], feature: FeatureId, edge: EdgeName) -> Option<EntityId> {
    named_edges(solids, edge).find_map(|(solid, faces)| {
        faces.iter().find_map(|face| match made_by(solid, *face)? {
            FaceOrigin::Side {
                feature: maker,
                entity,
            } if maker == feature.raw() => Some(EntityId::from_raw(Hole::point_of(entity))),
            _ => None,
        })
    })
}

pub fn callout_feature(
    document: &Document,
    evaluation: &Evaluation,
    drawing: &Drawing,
    edge: EdgeName,
) -> Result<FeatureId, CalloutRefusal> {
    let bodies = drawn_bodies(document, evaluation, drawing);
    let solids = bodies.solids();

    for (solid, faces) in named_edges(&solids, edge) {
        let threaded = document.active_features().find(|feature| {
            feature.kind.thread().is_some_and(|thread| {
                thread
                    .resolution(solid)
                    .found()
                    .iter()
                    .any(|face| faces.contains(face))
            })
        });

        if let Some(thread) = threaded {
            return Ok(thread.id());
        }

        let drilled = faces.iter().find_map(|face| {
            let maker = FeatureId::from_raw(made_by(solid, *face)?.feature());
            document.feature(maker)?.kind.hole().map(|_| maker)
        });

        if let Some(hole) = drilled {
            return Ok(hole);
        }
    }
    Err(CalloutRefusal::NotAHole)
}

pub fn callout_texts(
    document: &Document,
    evaluation: &Evaluation,
    drawing: &Drawing,
) -> CalloutTexts {
    let mut bodies: Option<DrawnBodies> = None;
    let mut texts = CalloutTexts::new();

    for callout in &drawing.callouts {
        let needs_point = document
            .feature(callout.feature)
            .and_then(|feature| feature.kind.hole())
            .is_some_and(|hole| circles_differ(document, hole));
        let point = if needs_point {
            let solids = bodies
                .get_or_insert_with(|| drawn_bodies(document, evaluation, drawing))
                .solids();

            hole_point(&solids, callout.feature, callout.edge)
        } else {
            None
        };

        texts.insert(
            callout.id,
            callout_lines(document, &evaluation.parameters, callout.feature, point),
        );
    }
    texts
}

struct Block {
    at: Point2,
    half_width: f64,
    level: f64,
}

fn leader(
    callout: &DrawingCallout,
    plan: &SheetPlan,
    lines: &DrawnLines<'_>,
    block: &Block,
) -> Option<Result<Vec<SheetMark>, AnnotationProblem>> {
    let height = CALLOUT_TEXT_HEIGHT;
    let placed = plan.placed(callout.view)?;
    let view_lines = lines.get(&callout.view)?;

    let seen = match seen_edge(view_lines, callout.edge) {
        Ok(seen) => seen,
        Err(problem) => return Some(Err(problem)),
    };
    let round = seen.round();

    let anchor = placed.on_sheet(round.map_or_else(|| seen.middle(), |(centre, _)| centre));
    let side = if anchor.x >= block.at.x { 1.0 } else { -1.0 };

    let shoulder = Point2::new(
        block.at.x + side * (block.half_width + SHOULDER_GAP * height),
        block.level,
    );
    let elbow = shoulder + Vector2::new(side * LEADER_SHOULDER * height, 0.0);
    let from = (elbow - placed.origin) / placed.scale.max(f64::MIN_POSITIVE);
    let target = placed.on_sheet(match round {
        Some((centre, radius)) => {
            centre + (from - centre).try_normalize().unwrap_or(Vector2::X) * radius
        }
        None => seen
            .curve
            .point(seen.curve.closest_parameter(from, seen.range)),
    });

    let mut marks = vec![SheetMark::Line(shoulder, elbow)];

    if let Some(pointing) = (target - elbow).try_normalize() {
        marks.push(SheetMark::Line(elbow, target));
        marks.extend(lines_of(arrowhead(target, pointing, height)));
    }
    Some(Ok(marks))
}

fn extent(written: &[CalloutLine], at: Point2) -> (f64, f64) {
    let height = CALLOUT_TEXT_HEIGHT;
    let width = written
        .iter()
        .map(|line| line.width(height))
        .fold(0.0, f64::max);
    let count = written.len().max(1) as f64;

    let tall = height + (count - 1.0) * LINE_PITCH * height;
    (width, at.y + tall / 2.0)
}

fn unknown() -> Vec<CalloutLine> {
    vec![CalloutLine(vec![CalloutPiece::Text(UNKNOWN.to_owned())])]
}

pub(crate) fn sheet_callout(
    drawing: &Drawing,
    callout: &DrawingCallout,
    plan: &SheetPlan,
    lines: &DrawnLines<'_>,
    texts: &CalloutTexts,
) -> SheetAnnotation {
    let height = CALLOUT_TEXT_HEIGHT;
    let at = note_on_sheet(plan, callout.at);
    let read = texts
        .get(&callout.id)
        .cloned()
        .unwrap_or(Err(AnnotationProblem::FeatureGone));
    let (written, mut problem) = match read {
        Ok(written) => (written, None),
        Err(problem) => (unknown(), Some(problem)),
    };

    let (width, top) = extent(&written, at);

    let block = Block {
        at,
        half_width: width / 2.0,
        level: top - height / 2.0,
    };

    let pointing = if drawing.has_view(callout.view) {
        leader(callout, plan, lines, &block)
    } else {
        Some(Err(AnnotationProblem::ViewGone))
    };
    let mut marks = match pointing {
        Some(Ok(marks)) if problem.is_none() => marks,
        Some(Err(lost)) => {
            problem = problem.or(Some(lost));
            Vec::new()
        }
        Some(Ok(_)) | None => Vec::new(),
    };
    let written = if problem.is_some() {
        unknown()
    } else {
        written
    };
    let (width, top) = extent(&written, at);
    let left = at.x - width / 2.0;
    let mut laid_texts = Vec::new();

    for (index, line) in written.iter().enumerate() {
        let baseline = top - height - index as f64 * LINE_PITCH * height;

        laid_texts.extend(line.laid(Point2::new(left, baseline), height, &mut marks));
    }
    SheetAnnotation {
        annotation: Annotation::Callout(callout.id),
        marks,
        texts: laid_texts,
        measure: None,
        problem,
    }
}
