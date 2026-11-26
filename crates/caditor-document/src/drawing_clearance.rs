use std::f64::consts::{PI, TAU};

use ahash::AHashMap;
use caditor_geometry::{Point2, Vector2};
use caditor_kernel::{Curve2, SamplingTolerance, ViewPiece};
use caditor_sketch::annotation::{Footprint, Obstacles};

use crate::{
    drawing_dimensions::{
        DIMENSION_TEXT_HEIGHT, DrawnLines, SheetAnnotation, arc_point, text_corners,
    },
    drawing_sheet::{PlacedView, SheetMark, SheetPlan, SheetText},
    tally,
};

pub(crate) const LABEL_CLEARANCE: f64 = 0.5;
const LABEL_CELL: f64 = 4.0;
const LINE_CELL: f64 = 2.0;
const LINE_CHORD: f64 = 0.1;
const LINE_ANGLE: f64 = PI / 18.0;
const MARK_STEP: f64 = PI / 16.0;
const DEGENERATE: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Clash {
    Clear,
    OnLine,
    OnText,
}

type Segment = (Point2, Point2);
type CellKey = (i64, i64);

struct SheetLines {
    reach: (Point2, Point2),
    segments: Vec<Segment>,
    cells: AHashMap<CellKey, Vec<usize>>,
}

impl SheetLines {
    fn new(reach: (Point2, Point2)) -> Self {
        Self {
            reach,
            segments: Vec::new(),
            cells: AHashMap::new(),
        }
    }

    fn cell_size() -> f64 {
        LINE_CELL * DIMENSION_TEXT_HEIGHT
    }

    fn cell_of(at: f64) -> i64 {
        (at / Self::cell_size()).floor() as i64
    }

    fn add(&mut self, (from, to): Segment) {
        if !(from.is_finite() && to.is_finite()) {
            return;
        }

        let Some((from, to)) = clipped((from, to), self.reach) else {
            return;
        };
        let index = self.segments.len();

        self.segments.push((from, to));
        for cell in cells_along((from, to)) {
            self.cells.entry(cell).or_default().push(index);
        }
    }

    fn crossed_by(&self, around: &TextBox) -> bool {
        let (low, high) = around.reach();

        let (low, high) = (low.max(self.reach.0), high.min(self.reach.1));

        if low.x > high.x || low.y > high.y {
            return false;
        }

        let columns = Self::cell_of(low.x)..=Self::cell_of(high.x);
        let rows = Self::cell_of(low.y)..=Self::cell_of(high.y);

        columns
            .flat_map(|column| rows.clone().map(move |row| (column, row)))
            .filter_map(|cell| self.cells.get(&cell))
            .flatten()
            .filter_map(|index| self.segments.get(*index))
            .any(|segment| {
                tally::add(1);
                around.crossed(*segment)
            })
    }
}

fn cells_along((from, to): Segment) -> Vec<CellKey> {
    let size = SheetLines::cell_size();
    let (left, right) = if from.x <= to.x {
        (from, to)
    } else {
        (to, from)
    };

    let run = right.x - left.x;
    let height_at = |x: f64| {
        if run <= DEGENERATE {
            left.y
        } else {
            left.y + (right.y - left.y) * ((x - left.x) / run)
        }
    };

    let mut cells = Vec::new();

    for column in SheetLines::cell_of(left.x)..=SheetLines::cell_of(right.x) {
        let (enter, leave) = if run <= DEGENERATE {
            (left.y, right.y)
        } else {
            let start = (column as f64 * size).max(left.x);
            let end = ((column as f64 + 1.0) * size).min(right.x);
            (height_at(start), height_at(end))
        };

        let rows = SheetLines::cell_of(enter.min(leave))..=SheetLines::cell_of(enter.max(leave));

        cells.extend(rows.map(|row| (column, row)));
    }
    cells
}

fn clipped((from, to): Segment, (low, high): (Point2, Point2)) -> Option<Segment> {
    let along = to - from;
    let mut enter: f64 = 0.0;
    let mut leave: f64 = 1.0;

    for (start, step, (least, most)) in [
        (from.x, along.x, (low.x, high.x)),
        (from.y, along.y, (low.y, high.y)),
    ] {
        if step.abs() <= DEGENERATE {
            if start < least || start > most {
                return None;
            }
            continue;
        }

        let first = (least - start) / step;
        let second = (most - start) / step;

        enter = enter.max(first.min(second));
        leave = leave.min(first.max(second));
        if enter > leave {
            return None;
        }
    }
    Some((from + along * enter, from + along * leave))
}

struct TextBox {
    centre: Point2,
    along: Vector2,
    half: Vector2,
}

impl TextBox {
    fn of(text: &SheetText) -> Self {
        let [low, _, high, _] = text_corners(text);
        let along = Vector2::from_angle(text.angle);
        let up = along.perp();

        let span = high - low;
        Self {
            centre: (low + high) / 2.0,
            along,
            half: Vector2::new(span.dot(along).abs(), span.dot(up).abs()) / 2.0
                + Vector2::splat(label_margin()),
        }
    }

    fn local(&self, point: Point2) -> Vector2 {
        let offset = point - self.centre;
        Vector2::new(offset.dot(self.along), offset.dot(self.along.perp()))
    }

    fn reach(&self) -> (Point2, Point2) {
        let up = self.along.perp();
        let extent = Vector2::new(
            (self.along.x * self.half.x).abs() + (up.x * self.half.y).abs(),
            (self.along.y * self.half.x).abs() + (up.y * self.half.y).abs(),
        );
        (self.centre - extent, self.centre + extent)
    }

    fn crossed(&self, (from, to): Segment) -> bool {
        clipped((self.local(from), self.local(to)), (-self.half, self.half)).is_some()
    }
}

fn label_margin() -> f64 {
    LABEL_CLEARANCE * DIMENSION_TEXT_HEIGHT / 2.0
}

fn label_footprint(text: &SheetText) -> Footprint {
    let [first, others @ ..] = text_corners(text);

    let (low, high) = others
        .into_iter()
        .fold((first, first), |(low, high), corner| {
            (low.min(corner), high.max(corner))
        });
    Footprint {
        center: (low + high) / 2.0,
        half: (high - low) / 2.0 + Vector2::splat(label_margin()),
    }
}

fn polyline(points: Vec<Point2>) -> Vec<Segment> {
    points
        .windows(2)
        .filter_map(|pair| match pair {
            [from, to] => Some((*from, *to)),
            _ => None,
        })
        .collect()
}

fn arc_segments(center: Point2, radius: f64, (start, end): (f64, f64)) -> Vec<Segment> {
    let sweep = end - start;
    let steps = (sweep.abs() / MARK_STEP).ceil().clamp(1.0, TAU / MARK_STEP) as usize;

    polyline(
        (0..=steps)
            .map(|step| arc_point(center, radius, start + sweep * step as f64 / steps as f64))
            .collect(),
    )
}

fn mark_segments(mark: &SheetMark) -> Vec<Segment> {
    match *mark {
        SheetMark::Line(from, to) => vec![(from, to)],
        SheetMark::Circle { center, radius } => arc_segments(center, radius, (0.0, TAU)),
        SheetMark::Arc {
            center,
            radius,
            start,
            end,
        } => arc_segments(center, radius, (start, end)),
    }
}

fn piece_segments(piece: &ViewPiece, placed: &PlacedView) -> Vec<Segment> {
    match piece.curve {
        Curve2::Line(_) => vec![(
            placed.on_sheet(piece.curve.point(piece.range.start())),
            placed.on_sheet(piece.curve.point(piece.range.end())),
        )],
        _ => {
            let chord = LINE_CHORD / placed.scale.max(f64::MIN_POSITIVE);
            let tolerance = SamplingTolerance::new(chord, LINE_ANGLE)
                .unwrap_or_else(|| SamplingTolerance::for_extent(1.0));

            polyline(
                piece
                    .curve
                    .sample(piece.range, &tolerance)
                    .into_iter()
                    .map(|sample| placed.on_sheet(sample.point))
                    .collect(),
            )
        }
    }
}

pub(crate) struct Clearance {
    labels: Obstacles,
    lines: SheetLines,
}

impl Clearance {
    pub(crate) fn of_sheet(plan: &SheetPlan, lines: &DrawnLines<'_>, hidden: bool) -> Self {
        let margin = Vector2::splat(SheetLines::cell_size());
        let mut clearance = Self {
            labels: Obstacles::new(LABEL_CELL * DIMENSION_TEXT_HEIGHT),
            lines: SheetLines::new((
                Point2::ZERO - margin,
                Point2::new(plan.width, plan.height) + margin,
            )),
        };

        for placed in &plan.views {
            let Some(view_lines) = lines.get(&placed.view) else {
                continue;
            };
            for piece in view_lines.drawn_pieces(hidden) {
                for segment in piece_segments(piece, placed) {
                    clearance.lines.add(segment);
                }
            }
        }
        for cutting in &plan.cutting {
            clearance.lines.add(cutting.chain);
            for end in cutting.ends {
                clearance.lines.add(end);
            }
        }
        clearance.add_drawn(&plan.marks, &plan.texts);
        clearance
    }

    pub(crate) fn add(&mut self, laid: &SheetAnnotation) {
        self.add_drawn(&laid.marks, &laid.texts);
    }

    fn add_drawn(&mut self, marks: &[SheetMark], texts: &[SheetText]) {
        for text in texts {
            self.labels.add(label_footprint(text));
        }
        for segment in marks.iter().flat_map(mark_segments) {
            self.lines.add(segment);
        }
    }

    pub(crate) fn clash(&self, texts: &[SheetText]) -> Clash {
        let on_text = texts
            .iter()
            .any(|text| self.labels.overlap(&label_footprint(text)) > 0.0);

        if on_text {
            Clash::OnText
        } else if texts
            .iter()
            .any(|text| self.lines.crossed_by(&TextBox::of(text)))
        {
            Clash::OnLine
        } else {
            Clash::Clear
        }
    }
}
