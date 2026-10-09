use caditor_document::CancelToken;
use caditor_geometry::{Aabb2, Point2, Vector2};

use super::{
    Construction, ExportError,
    figure::{Anchor, Figure, Layer, Motion, Shape, Text},
    nest::{Item, nest},
};

const MIN_SIDE_BY_SIDE_GAP: f64 = 10.0;
const SIDE_BY_SIDE_GAP_FRACTION: f64 = 0.1;
const MIN_TEXT_HEIGHT: f64 = 2.5;
const TEXT_HEIGHT_FRACTION: f64 = 0.02;
const LABEL_BAND: f64 = 2.0;
const LABEL_LIFT: f64 = 0.5;

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
            let items: Vec<Item<'_>> = parts
                .iter()
                .map(|(part, _)| {
                    let label = if labelled && !part.label.is_empty() {
                        label_text(&part.label, Point2::ZERO, text_height).width()
                    } else {
                        0.0
                    };
                    Item {
                        figure: &part.figure,
                        label,
                        band,
                    }
                })
                .collect();
            let nested = nest(&items, &nesting, cancel)?;
            (nested.motions, nested.too_wide)
        }
    };
    let mut figure = Figure::default();
    for ((part, bounds), motion) in parts.into_iter().zip(motions) {
        let mut moved = Figure::default();
        moved.append_moved(part.figure, motion);
        let placed = moved
            .bounds()
            .unwrap_or_else(|| moved_bounds(bounds, motion));
        figure.shapes.append(&mut moved.shapes);
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
