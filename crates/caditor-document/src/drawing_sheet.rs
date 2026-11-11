use std::collections::BTreeMap;

use caditor_geometry::{Aabb2, Plane, Point2, Vector2};
use caditor_lettering::{Alignment, Style, lettered};

use crate::{
    drawings::{
        DetailView, Drawing, DrawingScale, DrawnView, Projection, ScaleRatio, SectionView,
        SheetOrientation, SheetSize, StandardView,
    },
    properties::ModelProperties,
};

pub const VIEW_GAP: f64 = 15.0;
pub const TITLE_BLOCK_WIDTH: f64 = 180.0;
pub const TITLE_BLOCK_HEIGHT: f64 = 36.0;
const TITLE_ROW: f64 = 9.0;
const VIEW_AREA_PADDING: f64 = 5.0;
const FILING_MARGIN: f64 = 20.0;
const NARROW_MARGIN: f64 = 10.0;
const WIDE_MARGIN: f64 = 20.0;
const CENTRING_MARK: f64 = 5.0;
const CAPTION_HEIGHT: f64 = 1.8;
const VALUE_HEIGHT: f64 = 3.5;
const TITLE_HEIGHT: f64 = 5.0;
const SMALLEST_TEXT: f64 = 2.0;
const CELL_INSET: f64 = 1.5;
const CAPTION_INSET: f64 = 1.0;
const CAPTION_GAP: f64 = 1.0;
const ESTIMATED_WIDTH_PER_HEIGHT: f64 = 0.75;
const SYMBOL_DIAMETER: f64 = 5.0;
const SYMBOL_LENGTH: f64 = 5.0;
const SYMBOL_GAP: f64 = 2.5;
const ELLIPSIS: char = '…';
pub const SECTION_LETTER_HEIGHT: f64 = 5.0;
const SECTION_LABEL_GAP: f64 = 3.0;
const SECTION_LABEL_ROOM: f64 = SECTION_LETTER_HEIGHT + SECTION_LABEL_GAP;
const CUTTING_OVERSHOOT: f64 = 4.0;
const CUTTING_END: f64 = 6.0;
const ARROW_LENGTH: f64 = 8.0;
const ARROW_HEAD_LENGTH: f64 = 3.0;
const ARROW_HEAD_HALF_WIDTH: f64 = 1.0;
const LETTER_GAP: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAnchor {
    Start,
    Middle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SheetText {
    pub at: Point2,
    pub height: f64,
    pub content: String,
    pub anchor: TextAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SheetMark {
    Line(Point2, Point2),
    Circle { center: Point2, radius: f64 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleFields {
    pub title: String,
    pub part_number: String,
    pub revision: String,
    pub author: String,
    pub organisation: String,
    pub drawing: String,
}

impl TitleFields {
    pub fn of(properties: &ModelProperties, model_name: &str, drawing: &Drawing) -> Self {
        let title = if properties.title.is_empty() {
            model_name.to_owned()
        } else {
            properties.title.clone()
        };
        Self {
            title,
            part_number: properties.part_number.clone(),
            revision: properties.revision.clone(),
            author: properties.author.clone(),
            organisation: properties.organisation.clone(),
            drawing: drawing.name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CuttingLine {
    pub letter: char,
    pub chain: (Point2, Point2),
    pub ends: [(Point2, Point2); 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedView {
    pub view: DrawnView,
    pub origin: Point2,
    pub scale: f64,
}

impl PlacedView {
    pub fn on_sheet(&self, point: Point2) -> Point2 {
        self.origin + point * self.scale
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SheetPlan {
    pub width: f64,
    pub height: f64,
    pub frame: Aabb2,
    pub view_area: Aabb2,
    pub marks: Vec<SheetMark>,
    pub texts: Vec<SheetText>,
    pub views: Vec<PlacedView>,
    pub cutting: Vec<CuttingLine>,
    pub unmarked: Vec<char>,
    pub scale: ScaleRatio,
    pub fits: bool,
}

pub fn sheet_size(size: SheetSize, orientation: SheetOrientation) -> (f64, f64) {
    let (short, long) = size.short_and_long_sides();
    match orientation {
        SheetOrientation::Landscape => (long, short),
        SheetOrientation::Portrait => (short, long),
    }
}

fn frame_of(size: SheetSize, width: f64, height: f64) -> Aabb2 {
    let margin = match size {
        SheetSize::A0 | SheetSize::A1 => WIDE_MARGIN,
        SheetSize::A2 | SheetSize::A3 | SheetSize::A4 => NARROW_MARGIN,
    };
    Aabb2::from_point(Point2::new(FILING_MARGIN.max(margin), margin))
        .including(Point2::new(width - margin, height - margin))
}

fn rectangle(marks: &mut Vec<SheetMark>, bounds: Aabb2) {
    let (low, high) = (bounds.min(), bounds.max());
    let corners = [
        low,
        Point2::new(high.x, low.y),
        high,
        Point2::new(low.x, high.y),
    ];
    for (index, corner) in corners.iter().enumerate() {
        let next = corners
            .get((index + 1) % corners.len())
            .copied()
            .unwrap_or(low);
        marks.push(SheetMark::Line(*corner, next));
    }
}

fn centring_marks(marks: &mut Vec<SheetMark>, width: f64, height: f64, frame: Aabb2) {
    let (low, high) = (frame.min(), frame.max());
    let middle = Point2::new(width / 2.0, height / 2.0);
    marks.push(SheetMark::Line(
        Point2::new(middle.x, 0.0),
        Point2::new(middle.x, low.y + CENTRING_MARK),
    ));
    marks.push(SheetMark::Line(
        Point2::new(middle.x, height),
        Point2::new(middle.x, high.y - CENTRING_MARK),
    ));
    marks.push(SheetMark::Line(
        Point2::new(0.0, middle.y),
        Point2::new(low.x + CENTRING_MARK, middle.y),
    ));
    marks.push(SheetMark::Line(
        Point2::new(width, middle.y),
        Point2::new(high.x - CENTRING_MARK, middle.y),
    ));
}

pub fn text_width(content: &str, height: f64) -> f64 {
    let per_height = lettered(content, Style::default(), Alignment::Left).map_or_else(
        |_| ESTIMATED_WIDTH_PER_HEIGHT * content.chars().count() as f64,
        |laid| laid.width,
    );
    height * per_height
}

fn fitted_text(content: &str, height: f64, room: f64) -> (String, f64) {
    if text_width(content, height) <= room {
        return (content.to_owned(), height);
    }
    let shrunk = (room / text_width(content, 1.0)).max(SMALLEST_TEXT);
    if text_width(content, shrunk) <= room {
        return (content.to_owned(), shrunk);
    }
    let characters: Vec<char> = content.chars().collect();
    let cut = (0..characters.len())
        .rev()
        .map(|kept| {
            let kept: String = characters.iter().take(kept).collect();
            format!("{}{ELLIPSIS}", kept.trim_end())
        })
        .find(|cut| text_width(cut, shrunk) <= room)
        .unwrap_or_else(|| ELLIPSIS.to_string());
    (cut, shrunk)
}

struct Cell {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    caption: &'static str,
    value: String,
    value_height: f64,
}

fn cell_texts(texts: &mut Vec<SheetText>, origin: Point2, cell: &Cell, room_taken: f64) {
    let left = origin.x + cell.x + CELL_INSET;
    let top = origin.y + cell.y + cell.height;
    texts.push(SheetText {
        at: Point2::new(left, top - CAPTION_INSET - CAPTION_HEIGHT),
        height: CAPTION_HEIGHT,
        content: cell.caption.to_owned(),
        anchor: TextAnchor::Start,
    });
    if cell.value.is_empty() {
        return;
    }
    let room = cell.width - 2.0 * CELL_INSET - room_taken;
    let (content, height) = fitted_text(&cell.value, cell.value_height, room);
    let below_caption = cell.height - CELL_INSET - CAPTION_INSET - CAPTION_HEIGHT - CAPTION_GAP;
    let baseline = origin.y + cell.y + CELL_INSET + ((below_caption - height) / 2.0).max(0.0);
    texts.push(SheetText {
        at: Point2::new(left, baseline),
        height,
        content,
        anchor: TextAnchor::Start,
    });
}

fn projection_symbol(
    marks: &mut Vec<SheetMark>,
    projection: Projection,
    right: f64,
    middle_y: f64,
) {
    let big = SYMBOL_DIAMETER / 2.0;
    let small = big / 2.0;
    let width = SYMBOL_LENGTH + SYMBOL_GAP + SYMBOL_DIAMETER;
    let left = right - width;
    let (cone_left, circles_center_x) = match projection {
        Projection::FirstAngle => (left, right - big),
        Projection::ThirdAngle => (left + SYMBOL_DIAMETER + SYMBOL_GAP, left + big),
    };
    let cone_right = cone_left + SYMBOL_LENGTH;
    let corners = [
        Point2::new(cone_left, middle_y - small),
        Point2::new(cone_right, middle_y - big),
        Point2::new(cone_right, middle_y + big),
        Point2::new(cone_left, middle_y + small),
    ];
    for (index, corner) in corners.iter().enumerate() {
        let next = corners
            .get((index + 1) % corners.len())
            .copied()
            .unwrap_or(*corner);
        marks.push(SheetMark::Line(*corner, next));
    }
    let center = Point2::new(circles_center_x, middle_y);
    marks.push(SheetMark::Circle {
        center,
        radius: big,
    });
    marks.push(SheetMark::Circle {
        center,
        radius: small,
    });
}

fn title_block(
    marks: &mut Vec<SheetMark>,
    texts: &mut Vec<SheetText>,
    frame: Aabb2,
    fields: &TitleFields,
    drawing: &Drawing,
    scale: ScaleRatio,
) {
    let origin = Point2::new(frame.max().x - TITLE_BLOCK_WIDTH, frame.min().y);
    let block = Aabb2::from_point(origin)
        .including(origin + Vector2::new(TITLE_BLOCK_WIDTH, TITLE_BLOCK_HEIGHT));
    rectangle(marks, block);
    let at = |x: f64, y: f64| origin + Vector2::new(x, y);
    let rules = [
        (at(0.0, TITLE_ROW), at(TITLE_BLOCK_WIDTH, TITLE_ROW)),
        (
            at(0.0, 2.0 * TITLE_ROW),
            at(TITLE_BLOCK_WIDTH, 2.0 * TITLE_ROW),
        ),
        (
            at(110.0, 3.0 * TITLE_ROW),
            at(TITLE_BLOCK_WIDTH, 3.0 * TITLE_ROW),
        ),
        (at(30.0, 0.0), at(30.0, TITLE_ROW)),
        (at(55.0, 0.0), at(55.0, TITLE_ROW)),
        (at(110.0, 0.0), at(110.0, TITLE_ROW)),
        (at(140.0, 0.0), at(140.0, TITLE_ROW)),
        (at(70.0, TITLE_ROW), at(70.0, 2.0 * TITLE_ROW)),
        (at(130.0, TITLE_ROW), at(130.0, 2.0 * TITLE_ROW)),
        (at(110.0, 2.0 * TITLE_ROW), at(110.0, TITLE_BLOCK_HEIGHT)),
    ];
    marks.extend(rules.map(|(from, to)| SheetMark::Line(from, to)));
    let cell = |x, y, width, height, caption, value: &str, value_height| Cell {
        x,
        y,
        width,
        height,
        caption,
        value: value.to_owned(),
        value_height,
    };
    let symbol_room = SYMBOL_LENGTH + SYMBOL_GAP + SYMBOL_DIAMETER + CELL_INSET;
    let cells = [
        cell(0.0, 18.0, 110.0, 18.0, "Title", &fields.title, TITLE_HEIGHT),
        cell(
            110.0,
            27.0,
            70.0,
            9.0,
            "Part number",
            &fields.part_number,
            VALUE_HEIGHT,
        ),
        cell(
            110.0,
            18.0,
            70.0,
            9.0,
            "Revision",
            &fields.revision,
            VALUE_HEIGHT,
        ),
        cell(
            0.0,
            9.0,
            70.0,
            9.0,
            "Organisation",
            &fields.organisation,
            VALUE_HEIGHT,
        ),
        cell(
            70.0,
            9.0,
            60.0,
            9.0,
            "Drawn by",
            &fields.author,
            VALUE_HEIGHT,
        ),
        cell(
            130.0,
            9.0,
            50.0,
            9.0,
            "Drawing",
            &fields.drawing,
            VALUE_HEIGHT,
        ),
        cell(0.0, 0.0, 30.0, 9.0, "Scale", &scale.label(), VALUE_HEIGHT),
        cell(
            30.0,
            0.0,
            25.0,
            9.0,
            "Size",
            drawing.size.name(),
            VALUE_HEIGHT,
        ),
        cell(110.0, 0.0, 30.0, 9.0, "Units", "mm", VALUE_HEIGHT),
        cell(140.0, 0.0, 40.0, 9.0, "Sheet", "1 / 1", VALUE_HEIGHT),
    ];
    for cell in &cells {
        cell_texts(texts, origin, cell, 0.0);
    }
    let projection = cell(
        55.0,
        0.0,
        55.0,
        9.0,
        "Projection",
        drawing.projection.name(),
        VALUE_HEIGHT * 0.8,
    );
    cell_texts(texts, origin, &projection, symbol_room);
    projection_symbol(
        marks,
        drawing.projection,
        origin.x + 110.0 - CELL_INSET,
        origin.y + (TITLE_ROW - CAPTION_HEIGHT) / 2.0,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Column {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Row {
    Upper,
    Middle,
    Lower,
}

fn cell_of(view: StandardView, projection: Projection) -> (Column, Row) {
    match (view, projection) {
        (StandardView::Front, _) => (Column::Middle, Row::Middle),
        (StandardView::Top, Projection::FirstAngle) => (Column::Middle, Row::Lower),
        (StandardView::Top, Projection::ThirdAngle) => (Column::Middle, Row::Upper),
        (StandardView::Right, Projection::FirstAngle)
        | (StandardView::Left, Projection::ThirdAngle) => (Column::Left, Row::Middle),
        (StandardView::Right, Projection::ThirdAngle)
        | (StandardView::Left, Projection::FirstAngle) => (Column::Right, Row::Middle),
        (StandardView::Isometric, _) => (Column::Right, Row::Upper),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Span {
    aligned: Option<(f64, f64)>,
    free: f64,
}

impl Span {
    fn align(&mut self, low: f64, high: f64) {
        self.aligned = Some(match self.aligned {
            Some((from, to)) => (from.min(low), to.max(high)),
            None => (low, high),
        });
    }

    fn length(&self) -> f64 {
        let aligned = self.aligned.map_or(0.0, |(low, high)| high - low);
        aligned.max(self.free)
    }
}

#[derive(Debug, Clone, Copy)]
enum SideKind {
    Section(char),
    Detail(DetailView),
}

struct SideView {
    kind: SideKind,
    bounds: Aabb2,
}

impl SideView {
    fn view(&self) -> DrawnView {
        match self.kind {
            SideKind::Section(letter) => DrawnView::Section(letter),
            SideKind::Detail(detail) => DrawnView::Detail(detail.letter),
        }
    }

    fn scale(&self, sheet: f64) -> f64 {
        match self.kind {
            SideKind::Section(_) => sheet,
            SideKind::Detail(detail) => detail.scale.ratio_at(sheet).factor(),
        }
    }

    fn label(&self, sheet: f64) -> String {
        match self.kind {
            SideKind::Section(letter) => format!("{letter}\u{2013}{letter}"),
            SideKind::Detail(detail) => detail.label_at(sheet),
        }
    }
}

struct SideColumn {
    views: Vec<SideView>,
}

impl SideColumn {
    fn of(drawing: &Drawing, extents: &BTreeMap<DrawnView, Aabb2>) -> Self {
        let sections = drawing.sections.iter().filter_map(|section| {
            let bounds = *extents.get(&DrawnView::Section(section.letter))?;
            Some(SideView {
                kind: SideKind::Section(section.letter),
                bounds,
            })
        });
        let details = drawing.details.iter().filter_map(|detail| {
            let bounds = *extents.get(&DrawnView::Detail(detail.letter))?;
            Some(SideView {
                kind: SideKind::Detail(*detail),
                bounds,
            })
        });
        Self {
            views: sections.chain(details).collect(),
        }
    }

    fn width(&self, sheet: f64) -> f64 {
        self.views
            .iter()
            .map(|view| view.bounds.size().x * view.scale(sheet))
            .fold(0.0, f64::max)
    }

    fn height(&self, sheet: f64) -> f64 {
        self.views
            .iter()
            .map(|view| view.bounds.size().y * view.scale(sheet))
            .sum()
    }

    fn fixed_height(&self) -> f64 {
        let count = self.views.len() as f64;
        count * SECTION_LABEL_ROOM + VIEW_GAP * (count - 1.0).max(0.0)
    }

    fn total_height(&self, sheet: f64) -> f64 {
        self.height(sheet) + self.fixed_height()
    }
}

struct Grid {
    columns: BTreeMap<Column, Span>,
    rows: BTreeMap<Row, Span>,
    side: SideColumn,
}

impl Grid {
    fn of(drawing: &Drawing, extents: &BTreeMap<DrawnView, Aabb2>) -> Self {
        let mut grid = Self {
            columns: BTreeMap::new(),
            rows: BTreeMap::new(),
            side: SideColumn::of(drawing, extents),
        };
        for view in &drawing.views {
            let Some(bounds) = extents.get(&DrawnView::Standard(*view)) else {
                continue;
            };
            let (column, row) = cell_of(*view, drawing.projection);
            let (low, high) = (bounds.min(), bounds.max());
            let column = grid.columns.entry(column).or_default();
            let row = grid.rows.entry(row).or_default();
            if *view == StandardView::Isometric {
                column.free = column.free.max(high.x - low.x);
                row.free = row.free.max(high.y - low.y);
            } else {
                column.align(low.x, high.x);
                row.align(low.y, high.y);
            }
        }
        grid
    }

    fn lengths<Key>(spans: &BTreeMap<Key, Span>) -> (f64, f64) {
        let total = spans.values().map(Span::length).sum::<f64>();
        let gaps = VIEW_GAP * spans.len().saturating_sub(1) as f64;
        (total, gaps)
    }

    fn is_empty(&self) -> bool {
        self.columns.is_empty() && self.rows.is_empty() && self.side.views.is_empty()
    }

    fn joining_gap(&self) -> f64 {
        if self.columns.is_empty() || self.side.views.is_empty() {
            0.0
        } else {
            VIEW_GAP
        }
    }

    fn fits(&self, area: Aabb2, scale: f64) -> bool {
        let (across, across_gaps) = Self::lengths(&self.columns);
        let (down, down_gaps) = Self::lengths(&self.rows);
        let size = area.size();
        let slack = 1e-9 * (1.0 + size.x.max(size.y));
        let wide = across * scale + self.side.width(scale) + across_gaps + self.joining_gap();
        wide <= size.x + slack
            && down * scale + down_gaps <= size.y + slack
            && self.side.total_height(scale) <= size.y + slack
    }
}

fn chosen_scale(drawing: &Drawing, grid: &Grid, area: Aabb2) -> (ScaleRatio, bool) {
    match drawing.scale {
        DrawingScale::Ratio(ratio) => (ratio, grid.fits(area, ratio.factor())),
        DrawingScale::Fit => {
            if grid.is_empty() {
                return (ScaleRatio::new(1, 1), true);
            }
            let fitting = ScaleRatio::PREFERRED
                .into_iter()
                .find(|ratio| grid.fits(area, ratio.factor()));
            match fitting {
                Some(ratio) => (ratio, true),
                None => {
                    let smallest = ScaleRatio::PREFERRED
                        .last()
                        .copied()
                        .unwrap_or(ScaleRatio::new(1, 1));
                    (smallest, false)
                }
            }
        }
    }
}

fn starts<Key: Copy + Ord>(
    spans: &BTreeMap<Key, Span>,
    scale: f64,
    from: f64,
) -> BTreeMap<Key, f64> {
    let mut at = from;
    let mut placed = BTreeMap::new();
    for (key, span) in spans {
        placed.insert(*key, at);
        at += span.length() * scale + VIEW_GAP;
    }
    placed
}

fn placed_side(
    column: &SideColumn,
    left: f64,
    center_y: f64,
    sheet_scale: f64,
    marking: &mut Marking<'_>,
) -> Vec<PlacedView> {
    let width = column.width(sheet_scale);
    let mut top = center_y + column.total_height(sheet_scale) / 2.0;
    let mut views = Vec::new();
    for side in &column.views {
        let scale = side.scale(sheet_scale);
        let size = side.bounds.size() * scale;
        let middle = left + width / 2.0;
        marking.texts.push(SheetText {
            at: Point2::new(middle, top - SECTION_LETTER_HEIGHT / 2.0),
            height: SECTION_LETTER_HEIGHT,
            content: side.label(sheet_scale),
            anchor: TextAnchor::Middle,
        });
        let view_top = top - SECTION_LABEL_ROOM;
        let placed = PlacedView {
            view: side.view(),
            origin: Point2::new(
                middle - size.x / 2.0 - side.bounds.min().x * scale,
                view_top - size.y - side.bounds.min().y * scale,
            ),
            scale,
        };
        if let SideKind::Detail(detail) = side.kind {
            marking.marks.push(SheetMark::Circle {
                center: placed.on_sheet(detail.centre),
                radius: detail.radius * scale,
            });
        }
        views.push(placed);
        top = view_top - size.y - VIEW_GAP;
    }
    views
}

fn placed_views(
    drawing: &Drawing,
    extents: &BTreeMap<DrawnView, Aabb2>,
    grid: &Grid,
    area: Aabb2,
    scale: f64,
    marking: &mut Marking<'_>,
) -> Vec<PlacedView> {
    let (across, across_gaps) = Grid::lengths(&grid.columns);
    let (down, down_gaps) = Grid::lengths(&grid.rows);
    let width = across * scale + across_gaps;
    let height = down * scale + down_gaps;
    let total = width + grid.joining_gap() + grid.side.width(scale);
    let center = area.center();
    let grid_left = center.x - total / 2.0;
    let lefts = starts(&grid.columns, scale, grid_left);
    let tops = starts(&grid.rows, scale, 0.0);
    let top = center.y + height / 2.0;
    let mut views = Vec::new();
    for view in &drawing.views {
        let Some(bounds) = extents.get(&DrawnView::Standard(*view)) else {
            continue;
        };
        let (column, row) = cell_of(*view, drawing.projection);
        let (Some(column_span), Some(row_span), Some(left), Some(down_from_top)) = (
            grid.columns.get(&column),
            grid.rows.get(&row),
            lefts.get(&column),
            tops.get(&row),
        ) else {
            continue;
        };
        let cell_width = column_span.length() * scale;
        let cell_height = row_span.length() * scale;
        let bottom = top - down_from_top - cell_height;
        let origin = if *view == StandardView::Isometric {
            let cell_center = Point2::new(left + cell_width / 2.0, bottom + cell_height / 2.0);
            cell_center - bounds.center() * scale
        } else {
            let (low_x, high_x) = column_span.aligned.unwrap_or((0.0, 0.0));
            let (low_y, high_y) = row_span.aligned.unwrap_or((0.0, 0.0));
            Point2::new(
                left + (cell_width - (high_x - low_x) * scale) / 2.0 - low_x * scale,
                bottom + (cell_height - (high_y - low_y) * scale) / 2.0 - low_y * scale,
            )
        };
        views.push(PlacedView {
            view: DrawnView::Standard(*view),
            origin,
            scale,
        });
    }
    views.extend(placed_side(
        &grid.side,
        grid_left + width + grid.joining_gap(),
        center.y,
        scale,
        marking,
    ));
    views
}

struct Marking<'a> {
    marks: &'a mut Vec<SheetMark>,
    texts: &'a mut Vec<SheetText>,
}

impl Marking<'_> {
    fn arrow(&mut self, tip: Point2, pointing: Vector2, letter: char) {
        let tail = tip - pointing * ARROW_LENGTH;
        let head = tip - pointing * ARROW_HEAD_LENGTH;
        let side = pointing.perp() * ARROW_HEAD_HALF_WIDTH;
        self.marks.push(SheetMark::Line(tail, tip));
        self.marks.push(SheetMark::Line(head + side, tip));
        self.marks.push(SheetMark::Line(head - side, tip));
        self.texts.push(SheetText {
            at: tail - pointing * (LETTER_GAP + SECTION_LETTER_HEIGHT / 2.0),
            height: SECTION_LETTER_HEIGHT,
            content: letter.to_string(),
            anchor: TextAnchor::Middle,
        });
    }
}

fn cutting_line(
    section: &SectionView,
    cutting: &Plane,
    parent: &PlacedView,
    bounds: Aabb2,
    marking: &mut Marking<'_>,
) -> Option<CuttingLine> {
    let DrawnView::Standard(view) = parent.view else {
        return None;
    };
    let looking = view.hidden_line_view().ok()?;
    let frame = looking.frame();
    let normal = cutting.normal();
    let pointing =
        Vector2::new(frame.x_axis().dot(normal), frame.y_axis().dot(normal)).try_normalize()?;
    let along = pointing.perp();
    let through = frame.to_local(cutting.origin());
    let (least, most) = (bounds.min(), bounds.max());
    let (low, high) = [
        least,
        most,
        Point2::new(least.x, most.y),
        Point2::new(most.x, least.y),
    ]
    .iter()
    .map(|corner| (*corner - through).dot(along))
    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), at| {
        (low.min(at), high.max(at))
    });
    if !(low.is_finite() && high.is_finite()) {
        return None;
    }
    let start = parent.on_sheet(through + along * low) - along * CUTTING_OVERSHOOT;
    let end = parent.on_sheet(through + along * high) + along * CUTTING_OVERSHOOT;
    marking.arrow(start, pointing, section.letter);
    marking.arrow(end, pointing, section.letter);
    Some(CuttingLine {
        letter: section.letter,
        chain: (start, end),
        ends: [
            (start, start + along * CUTTING_END),
            (end, end - along * CUTTING_END),
        ],
    })
}

struct Cuts<'a> {
    planes: &'a BTreeMap<char, Plane>,
    extents: &'a BTreeMap<DrawnView, Aabb2>,
}

fn cutting_lines(
    drawing: &Drawing,
    cuts: &Cuts<'_>,
    views: &[PlacedView],
    marking: &mut Marking<'_>,
) -> (Vec<CuttingLine>, Vec<char>) {
    let mut lines = Vec::new();
    let mut unmarked = Vec::new();
    for section in &drawing.sections {
        let Some(cutting) = cuts.planes.get(&section.letter) else {
            continue;
        };
        let parent = DrawnView::Standard(section.parent);
        let line = views
            .iter()
            .find(|placed| placed.view == parent)
            .zip(cuts.extents.get(&parent))
            .and_then(|(placed, bounds)| cutting_line(section, cutting, placed, *bounds, marking));
        match line {
            Some(line) => lines.push(line),
            None => unmarked.push(section.letter),
        }
    }
    (lines, unmarked)
}

fn detail_circles(drawing: &Drawing, views: &[PlacedView], marking: &mut Marking<'_>) {
    let outward = Vector2::new(1.0, 1.0).normalize();
    for detail in &drawing.details {
        let drawn = DrawnView::Detail(detail.letter);
        let Some(parent) = views
            .iter()
            .find(|placed| placed.view == detail.parent.drawn())
        else {
            continue;
        };
        if !views.iter().any(|placed| placed.view == drawn) {
            continue;
        }
        let center = parent.on_sheet(detail.centre);
        let radius = detail.radius * parent.scale;
        marking.marks.push(SheetMark::Circle { center, radius });
        marking.texts.push(SheetText {
            at: center + outward * (radius + LETTER_GAP + SECTION_LETTER_HEIGHT / 2.0),
            height: SECTION_LETTER_HEIGHT,
            content: detail.letter.to_string(),
            anchor: TextAnchor::Middle,
        });
    }
}

pub fn lay_out_sheet(
    drawing: &Drawing,
    fields: &TitleFields,
    extents: &BTreeMap<DrawnView, Aabb2>,
    cuts: &BTreeMap<char, Plane>,
) -> SheetPlan {
    let (width, height) = sheet_size(drawing.size, drawing.orientation);
    let frame = frame_of(drawing.size, width, height);
    let view_area = Aabb2::from_point(
        frame.min() + Vector2::new(VIEW_AREA_PADDING, TITLE_BLOCK_HEIGHT + VIEW_AREA_PADDING),
    )
    .including(frame.max() - Vector2::splat(VIEW_AREA_PADDING));
    let grid = Grid::of(drawing, extents);
    let (scale, fits) = chosen_scale(drawing, &grid, view_area);
    let mut marks = Vec::new();
    let mut texts = Vec::new();
    rectangle(&mut marks, frame);
    centring_marks(&mut marks, width, height, frame);
    title_block(&mut marks, &mut texts, frame, fields, drawing, scale);
    let mut marking = Marking {
        marks: &mut marks,
        texts: &mut texts,
    };
    let views = placed_views(
        drawing,
        extents,
        &grid,
        view_area,
        scale.factor(),
        &mut marking,
    );
    let (cutting, unmarked) = cutting_lines(
        drawing,
        &Cuts {
            planes: cuts,
            extents,
        },
        &views,
        &mut marking,
    );
    detail_circles(drawing, &views, &mut marking);
    SheetPlan {
        width,
        height,
        frame,
        view_area,
        marks,
        texts,
        views,
        cutting,
        unmarked,
        scale,
        fits,
    }
}
