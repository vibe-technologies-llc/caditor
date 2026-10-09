use caditor_document::CancelToken;
use caditor_geometry::{Aabb2, Point2, Vector2};

use super::{
    Construction, ExportError,
    figure::{Anchor, Figure, Layer, Motion, Shape, Text},
};

const MIN_SIDE_BY_SIDE_GAP: f64 = 10.0;
const SIDE_BY_SIDE_GAP_FRACTION: f64 = 0.1;
const MIN_TEXT_HEIGHT: f64 = 2.5;
const TEXT_HEIGHT_FRACTION: f64 = 0.02;
const LABEL_BAND: f64 = 2.0;
const LABEL_LIFT: f64 = 0.5;
const FIT: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DrawingSheet {
    pub layout: SheetLayout,
    pub annotations: Annotations,
    pub construction: Construction,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SheetLayout {
    #[default]
    SideBySide,
    Nested(Nesting),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Annotations {
    #[default]
    LeftOut,
    Included,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nesting {
    width: f64,
    spacing: f64,
    turns: bool,
}

impl Nesting {
    pub fn new(width: f64, spacing: f64, turns: bool) -> Option<Self> {
        (width.is_finite() && width > 0.0 && spacing.is_finite() && spacing >= 0.0).then_some(
            Self {
                width,
                spacing,
                turns,
            },
        )
    }

    pub fn width(&self) -> f64 {
        self.width
    }

    pub fn spacing(&self) -> f64 {
        self.spacing
    }

    pub fn turns(&self) -> bool {
        self.turns
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Part {
    pub(super) figure: Figure,
    pub(super) label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Arranged {
    pub(super) figure: Figure,
    pub(super) too_wide: usize,
}

pub(super) fn text_height<'a>(figures: impl IntoIterator<Item = &'a Figure>) -> f64 {
    let largest = figures
        .into_iter()
        .filter_map(Figure::bounds)
        .map(|bounds| bounds.size().max_element())
        .fold(0.0, f64::max);
    (largest * TEXT_HEIGHT_FRACTION).max(MIN_TEXT_HEIGHT)
}

pub(super) fn arranged(
    parts: Vec<Part>,
    sheet: &DrawingSheet,
    text_height: f64,
    cancel: &CancelToken,
) -> Result<Arranged, ExportError> {
    let labelled = sheet.annotations == Annotations::Included;
    let parts: Vec<(Part, Aabb2)> = parts
        .into_iter()
        .filter_map(|part| {
            let bounds = part.figure.bounds()?;
            Some((part, bounds))
        })
        .collect();
    if parts.is_empty() {
        return Err(ExportError::NoCurves);
    }
    let (motions, too_wide) = match sheet.layout {
        SheetLayout::SideBySide => (side_by_side(&parts), 0),
        SheetLayout::Nested(nesting) => {
            let band = if labelled {
                LABEL_BAND * text_height
            } else {
                0.0
            };
            let items: Vec<Item> = parts
                .iter()
                .map(|(part, bounds)| {
                    let label = if labelled {
                        label_text(&part.label, Point2::ZERO, text_height).width()
                    } else {
                        0.0
                    };
                    Item::of(bounds.size(), label, band)
                })
                .collect();
            let nested = nest(&items, &nesting, cancel)?;
            let motions = parts
                .iter()
                .zip(&nested.placements)
                .map(|((_, bounds), placement)| placement.motion(*bounds, band))
                .collect();
            (motions, nested.too_wide)
        }
    };
    let mut figure = Figure::default();
    for ((part, bounds), motion) in parts.into_iter().zip(motions) {
        let placed = moved_bounds(bounds, motion);
        figure.append_moved(part.figure, motion);
        if labelled && !part.label.is_empty() {
            let baseline = Point2::new(
                placed.min().x,
                placed.min().y - LABEL_BAND * text_height + LABEL_LIFT * text_height,
            );
            figure.push(
                Layer::Labels,
                Shape::Text(label_text(&part.label, baseline, text_height)),
            );
        }
    }
    Ok(Arranged { figure, too_wide })
}

fn label_text(label: &str, at: Point2, height: f64) -> Text {
    Text {
        at,
        height,
        angle: 0.0,
        content: label.to_owned(),
        anchor: Anchor::Start,
    }
}

fn moved_bounds(bounds: Aabb2, motion: Motion) -> Aabb2 {
    let (low, high) = (bounds.min(), bounds.max());
    Aabb2::from_points(
        [
            low,
            high,
            Point2::new(low.x, high.y),
            Point2::new(high.x, low.y),
        ]
        .map(|corner| motion.point(corner)),
    )
    .unwrap_or(bounds)
}

fn side_by_side(parts: &[(Part, Aabb2)]) -> Vec<Motion> {
    let largest = parts
        .iter()
        .map(|(_, bounds)| bounds.size().max_element())
        .fold(0.0, f64::max);
    let gap = (largest * SIDE_BY_SIDE_GAP_FRACTION).max(MIN_SIDE_BY_SIDE_GAP);
    let mut placed: Option<Aabb2> = None;
    parts
        .iter()
        .map(|(_, bounds)| {
            let offset = match placed {
                None => Vector2::ZERO,
                Some(placed) => Vector2::new(
                    placed.max().x + gap - bounds.min().x,
                    placed.min().y - bounds.min().y,
                ),
            };
            let motion = Motion::shift(offset);
            let moved = moved_bounds(*bounds, motion);
            placed = Some(placed.map_or(moved, |placed| placed.union(moved)));
            motion
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Item {
    pub(super) upright: Vector2,
    pub(super) turned: Vector2,
}

impl Item {
    pub(super) fn of(size: Vector2, label: f64, band: f64) -> Self {
        Self {
            upright: Vector2::new(size.x.max(label), size.y + band),
            turned: Vector2::new(size.y.max(label), size.x + band),
        }
    }

    fn size(&self, turned: bool) -> Vector2 {
        if turned { self.turned } else { self.upright }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Placement {
    pub(super) at: Point2,
    pub(super) turned: bool,
}

impl Placement {
    fn motion(&self, bounds: Aabb2, band: f64) -> Motion {
        let turn = Motion {
            turned: self.turned,
            offset: Vector2::ZERO,
        };
        let low = moved_bounds(bounds, turn).min();
        Motion {
            turned: self.turned,
            offset: self.at + Vector2::new(0.0, band) - low,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Nested {
    pub(super) placements: Vec<Placement>,
    pub(super) too_wide: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    low: Point2,
    high: Point2,
}

impl Rect {
    fn clear_of(&self, other: &Self, spacing: f64) -> bool {
        self.high.x <= other.low.x - spacing + FIT
            || other.high.x + spacing <= self.low.x + FIT
            || self.high.y <= other.low.y - spacing + FIT
            || other.high.y + spacing <= self.low.y + FIT
    }
}

pub(super) fn nest(
    items: &[Item],
    nesting: &Nesting,
    cancel: &CancelToken,
) -> Result<Nested, ExportError> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|first, second| {
        let area = |index: &usize| {
            items
                .get(*index)
                .map_or(0.0, |item| item.upright.x * item.upright.y)
        };
        area(second).total_cmp(&area(first))
    });
    let mut placed: Vec<Rect> = Vec::with_capacity(items.len());
    let mut placements = vec![
        Placement {
            at: Point2::ZERO,
            turned: false,
        };
        items.len()
    ];
    let mut too_wide = 0;
    for index in order {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let Some(item) = items.get(index) else {
            continue;
        };
        let orientations: &[bool] = if nesting.turns && item.turned != item.upright {
            &[false, true]
        } else {
            &[false]
        };
        let best = orientations
            .iter()
            .filter_map(|turned| {
                let size = item.size(*turned);
                let at = lowest_spot(&placed, size, nesting)?;
                Some((at, size, *turned))
            })
            .min_by(|(first, first_size, _), (second, second_size, _)| {
                first
                    .y
                    .total_cmp(&second.y)
                    .then(first.x.total_cmp(&second.x))
                    .then((first.y + first_size.y).total_cmp(&(second.y + second_size.y)))
            });
        let (at, size, turned) = best.unwrap_or_else(|| {
            too_wide += 1;
            let turned = nesting.turns && item.turned.x < item.upright.x;
            let top = placed
                .iter()
                .map(|rect| rect.high.y + nesting.spacing)
                .fold(0.0, f64::max);
            (Point2::new(0.0, top), item.size(turned), turned)
        });
        placed.push(Rect {
            low: at,
            high: at + size,
        });
        if let Some(placement) = placements.get_mut(index) {
            *placement = Placement { at, turned };
        }
    }
    Ok(Nested {
        placements,
        too_wide,
    })
}

fn lowest_spot(placed: &[Rect], size: Vector2, nesting: &Nesting) -> Option<Point2> {
    if size.x > nesting.width + FIT {
        return None;
    }
    let candidates = |side: fn(&Rect) -> f64| {
        let mut values: Vec<f64> = std::iter::once(0.0)
            .chain(placed.iter().map(|rect| side(rect) + nesting.spacing))
            .collect();
        values.sort_by(f64::total_cmp);
        values.dedup();
        values
    };
    let columns = candidates(|rect| rect.high.x);
    let rows = candidates(|rect| rect.high.y);
    rows.iter().find_map(|y| {
        columns.iter().find_map(|x| {
            let spot = Rect {
                low: Point2::new(*x, *y),
                high: Point2::new(x + size.x, y + size.y),
            };
            (spot.high.x <= nesting.width + FIT
                && placed
                    .iter()
                    .all(|rect| spot.clear_of(rect, nesting.spacing)))
            .then_some(spot.low)
        })
    })
}
