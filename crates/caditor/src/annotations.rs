use std::{collections::BTreeMap, sync::Arc};

use caditor_document::{Document, FeatureId, FeatureState, Transaction};
use caditor_expression::{Dimension, Expression, Quantity};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{
    Constraint, ConstraintId, Entity, EntityId, Sketch, SketchSolution,
    annotation::{self, Footprint, Measured, Obstacles},
};
use egui::{
    Align2, Color32, Galley, Id, Key, Order, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, vec2,
};

use crate::{
    annotation_layout::{self, DimensionLayout, GlyphKind, GlyphSite, LabelFrame, Reach, Thinning},
    appearance, canvas, completion,
    feature_tree::count,
    field::{self, DimensionTarget, Reference},
    model::{Action, Model},
    selection::{Pickable, Selection},
    sketch_status, sketch_tools,
    snap::Screen,
    units::Units,
    viewport::SketchScreen,
};

const LABEL_GAP: f32 = 3.0;
const GLYPH_CLEARANCE: f32 = 1.5;
const STROKE_WIDTH: f32 = 1.2;
const ARROW_LENGTH: f32 = 9.0;
const ARROW_HALF_WIDTH: f32 = 3.2;
const GLYPH_SIZE: f32 = 15.0;
const GLYPH_HIT_SIZE: f32 = 16.0;
const DOT_RADIUS: f32 = 3.5;
const RING_WIDTH: f32 = 1.5;
const OPEN_END_RADIUS: f32 = 6.0;
const RING_MERGE: f64 = OPEN_END_RADIUS as f64 / 2.0;
const RING_CLUSTER_CELL: f64 = OPEN_END_RADIUS as f64 * 4.0;
const MOST_RINGS_PER_CELL: usize = 4;
const CLUSTER_RADIUS: f32 = OPEN_END_RADIUS + 3.0;
const BEYOND_RADIUS: f32 = 7.0;
const BEYOND_DASH: f32 = 4.0;
const SYMBOL_HALF: f32 = 4.0;
const SYMBOL_WIDTH: f32 = 1.4;
const PARALLEL_GAP: f32 = 1.8;
const PARALLEL_LEAN: f32 = 2.2;
const TANGENT_RADIUS: f32 = 2.8;
const TANGENT_LIFT: f32 = 1.2;
const FIELD_WIDTH: f32 = 120.0;
const FIELD_LIFT: f32 = 14.0;
const FIELD_MARGIN: f32 = 4.0;
const REQUEST_FRAMES: u8 = 30;
const MAX_STACKED: usize = 4;
const LISTED_BEYOND: usize = 6;
const RADIUS_PREFIX: &str = "R ";
const RHO_PREFIX: &str = "rho ";
const DIAMETER_PREFIX: &str = "Ø ";
const MIDPOINT_DOT: f32 = 1.8;
const INNER_RING: f32 = 1.8;
const DASH_GAP: f32 = 1.2;
const LOCK_HALF_WIDTH: f32 = 3.4;
const LOCK_TOP: f32 = -0.4;
const LOCK_BOTTOM: f32 = 4.2;
const SHACKLE_RADIUS: f32 = 2.2;
const SHACKLE_STEPS: usize = 8;
const MIRROR_TIP: f32 = 1.2;
const FRAME_GAP: f32 = 1.5;
pub const FRAME_WIDTH: f32 = 2.0;
const FRAME_DASH: f32 = 4.0;
const FRAME_DASH_GAP: f32 = 3.0;
const POINTER_REACH: f32 = 64.0;
const COLLAPSED_RADIUS: f32 = 2.0;
const COLLAPSED_HIT_SIZE: f32 = 10.0;
const COLLAPSED_SPACING: f64 = 8.0;
const MAX_MEASURED_TEXTS: usize = 1 << 16;
const EDIT_HINT: &str = "Double-click to change it, or drag its label to move it.";
const COLLAPSED_HINT: &str = "Click to show its label, or double-click to change it.";
const OUTSIDE_HINT: &str = "Double-click to change it, or highlight it from the keyboard and \
                            press Enter.";
const CLUSTERED_ENDS_HELP: &str = "here, too close together to ring one by one at this zoom: \
                                   curve ends joined to nothing. Zoom in to tell them apart, and \
                                   join them to close the outline.";
pub const MOVE_LABEL_TRANSACTION: &str = "Move dimension label";
const NO_LABEL_TO_MOVE: &str = "Select one dimension alone to move its label";

pub fn field_id(feature: FeatureId, constraint: ConstraintId) -> Id {
    Id::new(("canvas-dimension", feature, constraint))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    Normal,
    Conflicting,
    Redundant,
    Inactive,
}

impl Standing {
    fn collapsible(self) -> bool {
        matches!(self, Self::Normal | Self::Inactive)
    }

    fn frame(self) -> Option<Frame> {
        match self {
            Self::Conflicting => Some(Frame::Solid),
            Self::Redundant => Some(Frame::Dashed),
            Self::Normal | Self::Inactive => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    Solid,
    Dashed,
}

#[derive(Debug, Clone)]
struct DimensionMark {
    constraint: ConstraintId,
    layout: DimensionLayout,
    frame: Option<LabelFrame>,
    text: String,
    standing: Standing,
    label: Option<Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RingCluster {
    center: Vector2,
    ends: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct OpenEndMarks {
    rings: Vec<Vector2>,
    clusters: Vec<RingCluster>,
}

#[derive(Debug, Clone, Copy)]
struct CollapsedMark {
    constraint: ConstraintId,
    center: Vector2,
    standing: Standing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelPlace {
    pub at: Point2,
    pub frame: LabelFrame,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelMoving {
    pub feature: FeatureId,
    pub constraint: ConstraintId,
    pub place: LabelPlace,
}

impl LabelMoving {
    pub fn transaction(&self, model: &Model, target: Point2) -> Transaction {
        let mut transaction = sketch_tools::settled_transaction(
            model,
            self.feature,
            MOVE_LABEL_TRANSACTION.to_owned(),
        );
        transaction.set_sketch_label(
            self.feature,
            self.constraint,
            Some(self.place.frame.offset_of(target)),
        );
        transaction.finish()
    }
}

pub fn label_to_move(
    sketch: &Sketch,
    entities: &[EntityId],
    constraints: &[ConstraintId],
    drawing: bool,
) -> Result<ConstraintId, String> {
    match (entities, constraints) {
        ([], [only])
            if !drawing
                && sketch
                    .constraint(*only)
                    .is_some_and(|constraint| constraint.dimension().is_some()) =>
        {
            Ok(*only)
        }
        _ => Err(NO_LABEL_TO_MOVE.to_owned()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LabelDrag {
    feature: FeatureId,
    constraint: ConstraintId,
    grab: Vector2,
    offset: Option<Vector2>,
}

#[derive(Debug, Clone)]
struct GlyphMark {
    constraint: ConstraintId,
    anchor: EntityId,
    kind: GlyphKind,
    center: Vector2,
    standing: Standing,
    hover: Hover,
}

#[derive(Debug, Clone, Copy)]
struct GlyphItem {
    constraint: ConstraintId,
    kind: GlyphKind,
    standing: Standing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Hover {
    Dimension(ConstraintId),
    Collapsed(ConstraintId),
    Glyph(ConstraintId),
    Beyond(Vec<ConstraintId>),
    OpenEnds(usize),
}

impl Hover {
    fn describe(&self, sketch: &Sketch, outside: bool) -> String {
        match self {
            Self::Dimension(constraint) | Self::Collapsed(constraint) if outside => {
                let described = sketch
                    .constraint(*constraint)
                    .map(|constraint| sketch.describe(constraint))
                    .unwrap_or_default();
                format!("{described}. {OUTSIDE_HINT}")
            }
            Self::Dimension(constraint) => {
                let described = sketch
                    .constraint(*constraint)
                    .map(|constraint| sketch.describe(constraint))
                    .unwrap_or_default();
                format!("{described}. {EDIT_HINT}")
            }
            Self::Collapsed(constraint) => {
                let described = sketch
                    .constraint(*constraint)
                    .map(|constraint| sketch.describe(constraint))
                    .unwrap_or_default();
                format!("{described}. {COLLAPSED_HINT}")
            }
            Self::Glyph(constraint) => sketch.describe_constraint(*constraint),
            Self::Beyond(hidden) => beyond_description(sketch, hidden),
            Self::OpenEnds(ends) => {
                format!(
                    "{} {CLUSTERED_ENDS_HELP}",
                    count(*ends, "open end", "open ends")
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextsKey {
    feature: FeatureId,
    revision: u64,
    evaluation: u64,
    sketches: u64,
    units: Units,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SizesKey {
    pixels_per_point: f32,
    fonts: usize,
}

impl SizesKey {
    fn of(painter: &egui::Painter) -> Self {
        Self {
            pixels_per_point: painter.pixels_per_point(),
            fonts: painter.fonts(|fonts| fonts.definitions().font_data.len()),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct LabelTexts {
    key: Option<TextsKey>,
    texts: BTreeMap<ConstraintId, String>,
    sizes_key: Option<SizesKey>,
    sizes: BTreeMap<String, egui::Vec2>,
}

impl LabelTexts {
    fn refresh(&mut self, painter: &egui::Painter, model: &Model, feature: FeatureId) {
        let key = TextsKey {
            feature,
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            sketches: model.display().sketches.generation(),
            units: model.units(),
        };
        if self.key != Some(key) {
            self.key = Some(key);
            self.texts.clear();
        }
        let sizes_key = SizesKey::of(painter);
        if self.sizes_key != Some(sizes_key) || self.sizes.len() >= MAX_MEASURED_TEXTS {
            self.sizes_key = Some(sizes_key);
            self.sizes.clear();
        }
    }

    fn label(
        &mut self,
        painter: &egui::Painter,
        model: &Model,
        shown: &Sketch,
        id: ConstraintId,
        constraint: &Constraint,
        expression: &Expression,
    ) -> (&str, egui::Vec2) {
        let text = self
            .texts
            .entry(id)
            .or_insert_with(|| label_text(model, shown, id, constraint, expression));
        let size = match self.sizes.get(text.as_str()) {
            Some(size) => *size,
            None => {
                let size = painter
                    .layout_no_wrap(text.clone(), canvas::body(), Color32::PLACEHOLDER)
                    .size();
                self.sizes.insert(text.clone(), size);
                size
            }
        };
        (text.as_str(), size)
    }
}

#[derive(Debug, Clone)]
struct GlyphGroup {
    anchor: EntityId,
    site: Option<GlyphSite>,
    items: Vec<GlyphItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SketchKey {
    feature: FeatureId,
    revision: u64,
    evaluation: u64,
    sketches: u64,
    dragged: Option<ConstraintId>,
    outside: bool,
}

impl SketchKey {
    fn of(model: &Model, feature: FeatureId, dragged: Option<ConstraintId>, outside: bool) -> Self {
        Self {
            feature,
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            sketches: model.display().sketches.generation(),
            dragged,
            outside,
        }
    }
}

#[derive(Debug, Clone)]
struct MeasuredDimension {
    constraint: ConstraintId,
    measured: Measured,
    lane: usize,
    frame: Option<LabelFrame>,
    reach: Option<Reach>,
    offset: Option<Vector2>,
    standing: Standing,
}

#[derive(Debug, Clone)]
struct Measures {
    key: SketchKey,
    dimensions: Vec<MeasuredDimension>,
    groups: Vec<GlyphGroup>,
    centre: Option<Point2>,
    open_ends: Vec<Point2>,
    beyond: Vec<[Point2; 2]>,
}

impl Measures {
    fn collect(model: &Model, key: SketchKey, definition: &Sketch, shown: &Sketch) -> Self {
        let standings = Standings::load(model, key.feature, definition);
        let centre = centre_of(shown);
        let mut groups: BTreeMap<EntityId, Vec<GlyphItem>> = BTreeMap::new();
        let mut measured = Vec::new();
        for (id, constraint) in definition.constraints() {
            if constraint.dimension().is_some() {
                measured.push((id, annotation::measured(shown, constraint)));
                continue;
            }
            if key.outside {
                continue;
            }
            for (entity, kind) in annotation_layout::glyphs_of(shown, constraint) {
                groups.entry(entity).or_default().push(GlyphItem {
                    constraint: id,
                    kind,
                    standing: standings.of(id),
                });
            }
        }
        let placed =
            |id: ConstraintId| key.dragged == Some(id) || definition.label_offset(id).is_some();
        let unplaced: Vec<_> = measured
            .iter()
            .map(|(id, measured)| measured.filter(|_| !placed(*id)))
            .collect();
        let lanes = annotation::lanes(&unplaced, centre, extent_of(shown));
        let dimensions = measured
            .into_iter()
            .zip(lanes)
            .filter_map(|((constraint, measured), lane)| {
                let measured = measured?;
                let frame = annotation_layout::label_frame(&measured);
                Some(MeasuredDimension {
                    constraint,
                    measured,
                    lane,
                    frame,
                    reach: Reach::of(&measured, frame),
                    offset: definition.label_offset(constraint),
                    standing: standings.of(constraint),
                })
            })
            .collect();
        let result = sketch_status::up_to_date_result(model.evaluation(), key.feature)
            .filter(|_| !key.outside);
        let open_ends = result
            .into_iter()
            .flat_map(|result| &result.open_ends)
            .filter_map(|end| shown.point(*end))
            .collect();
        let beyond = result
            .into_iter()
            .flat_map(|result| &result.beyond)
            .filter_map(|beyond| Some([shown.nearest_end(*beyond)?, shown.point(beyond.point)?]))
            .collect();
        Self {
            key,
            dimensions,
            groups: groups
                .into_iter()
                .map(|(anchor, items)| GlyphGroup {
                    anchor,
                    site: GlyphSite::of(shown, anchor),
                    items,
                })
                .collect(),
            centre,
            open_ends,
            beyond,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ViewKey {
    sketch: SketchKey,
    screen: SketchScreen,
    rect: Rect,
    units: Units,
    dragged: Option<Vector2>,
    editing: Option<ConstraintId>,
    forced: Vec<ConstraintId>,
    kept: Vec<EntityId>,
    glyphs: bool,
}

impl ViewKey {
    fn thinning(&self, constraint: ConstraintId) -> Thinning {
        if self.forced.binary_search(&constraint).is_ok() || self.sketch.dragged == Some(constraint)
        {
            Thinning::Never
        } else {
            Thinning::WhenCrowded
        }
    }

    fn group_thinning(&self, group: &GlyphGroup) -> Thinning {
        let kept = self.kept.binary_search(&group.anchor).is_ok()
            || group
                .items
                .iter()
                .any(|item| self.thinning(item.constraint) == Thinning::Never);
        if kept {
            Thinning::Never
        } else {
            Thinning::WhenCrowded
        }
    }
}

#[derive(Debug, Clone)]
struct Marks {
    key: ViewKey,
    dimensions: Vec<DimensionMark>,
    collapsed: Vec<CollapsedMark>,
    glyphs: Vec<GlyphMark>,
    open_ends: OpenEndMarks,
    beyond: Vec<[Vector2; 2]>,
}

impl Marks {
    fn lay_out(
        painter: &egui::Painter,
        model: &Model,
        measures: &Measures,
        sketches: [&Sketch; 2],
        key: ViewKey,
        texts: &mut LabelTexts,
    ) -> Self {
        let [definition, shown] = sketches;
        let screen = &key.screen.projector();
        let view = Vector2::new(f64::from(key.rect.width()), f64::from(key.rect.height()));
        let (kept, thinned): (Vec<_>, Vec<_>) = measures
            .dimensions
            .iter()
            .partition(|dimension| key.thinning(dimension.constraint) == Thinning::Never);
        let ordered = kept
            .into_iter()
            .map(|dimension| (dimension, Thinning::Never))
            .chain(
                thinned
                    .into_iter()
                    .map(|dimension| (dimension, Thinning::WhenCrowded)),
            );
        let mut labels = annotation_layout::obstacles();
        let mut dimensions = Vec::new();
        let mut collapsed = Vec::new();
        for (dimension, thinning) in ordered {
            let id = dimension.constraint;
            let offset = match key.dragged {
                Some(offset) if measures.key.dragged == Some(id) => Some(offset),
                _ => dimension.offset,
            };
            let placed = dimension
                .frame
                .zip(offset)
                .map(|(frame, offset)| (frame, frame.place(offset)));
            let crowding = thinning == Thinning::WhenCrowded;
            let on_screen = dimension
                .reach
                .filter(|_| crowding)
                .and_then(|reach| reach.on_screen(placed, screen));
            if let Some(on_screen) = on_screen {
                if !on_screen.near_view(dimension.lane, view) {
                    continue;
                }
                if on_screen.collapses() && dimension.standing.collapsible() {
                    collapsed.push(CollapsedMark {
                        constraint: id,
                        center: on_screen.centre(),
                        standing: dimension.standing,
                    });
                    continue;
                }
            }
            let constraint = definition.constraint(id);
            let expression = constraint.and_then(Constraint::dimension);
            let (Some(constraint), Some(expression)) = (constraint, expression) else {
                continue;
            };
            let (text, size) = texts.label(painter, model, shown, id, constraint, expression);
            let editing = key.editing == Some(id);
            if !editing
                && let Some(on_screen) = on_screen
                && annotation_layout::crowded(
                    &labels,
                    &on_screen
                        .label_neighbourhood(dimension.lane, to_vector(canvas::chip_size(size))),
                )
            {
                continue;
            }
            let Some(layout) = annotation_layout::layout(
                &dimension.measured,
                screen,
                measures.centre,
                dimension.lane,
                placed.map(|(_, at)| at),
            ) else {
                continue;
            };
            let label = (!editing).then(|| label_rect(key.rect, &layout, size));
            if let Some(rect) = label {
                let taken = footprint(key.rect, rect);
                if crowding && annotation_layout::mostly_covered(&labels, &taken) {
                    continue;
                }
                labels.add(taken);
            }
            dimensions.push(DimensionMark {
                constraint: id,
                layout,
                frame: dimension.frame,
                text: text.to_owned(),
                standing: dimension.standing,
                label,
            });
        }
        dimensions.sort_by_key(|mark| mark.constraint);
        let glyphs = if key.glyphs {
            let mut blocked = annotation_layout::obstacles();
            for rect in dimensions.iter().filter_map(|mark| mark.label) {
                blocked.add(footprint(key.rect, rect.expand(GLYPH_CLEARANCE)));
            }
            let screen_centre = measures.centre.and_then(|centre| screen.to_screen(centre));
            place_glyphs(&measures.groups, screen, &key, screen_centre, blocked)
        } else {
            Vec::new()
        };
        let in_view = |at: &Vector2| at.x >= 0.0 && at.y >= 0.0 && at.x <= view.x && at.y <= view.y;
        let mut spaced = ScreenCells::over(view, COLLAPSED_SPACING);
        collapsed.retain(|mark: &CollapsedMark| in_view(&mark.center) && spaced.take(mark.center));
        collapsed.sort_by_key(|mark| mark.constraint);
        let open_ends = open_end_marks(
            measures
                .open_ends
                .iter()
                .filter_map(|end| screen.to_screen(*end))
                .filter(in_view),
            view,
        );
        let beyond = measures
            .beyond
            .iter()
            .filter_map(|[end, at]| {
                let (end, at) = (screen.to_screen(*end)?, screen.to_screen(*at)?);
                (in_view(&at) || in_view(&end)).then_some([end, at])
            })
            .collect();
        Self {
            key,
            dimensions,
            collapsed,
            glyphs,
            open_ends,
            beyond,
        }
    }

    fn dimension(&self, constraint: ConstraintId) -> Option<&DimensionMark> {
        self.dimensions
            .iter()
            .find(|mark| mark.constraint == constraint)
    }
}

struct ScreenGrid {
    cells_per_point: f64,
    columns: usize,
    rows: usize,
}

impl ScreenGrid {
    fn over(view: Vector2, cell: f64) -> Self {
        let cells_per_point = cell.recip();
        let span = |length: f64| (length.max(0.0) * cells_per_point) as usize + 1;
        Self {
            cells_per_point,
            columns: span(view.x),
            rows: span(view.y),
        }
    }

    fn len(&self) -> usize {
        self.columns.saturating_mul(self.rows)
    }

    fn index(&self, at: Vector2) -> Option<usize> {
        let cell = at * self.cells_per_point;
        let inside = cell.x >= 0.0 && cell.y >= 0.0;
        let (column, row) = (cell.x as usize, cell.y as usize);
        (inside && column < self.columns && row < self.rows).then(|| row * self.columns + column)
    }
}

struct ScreenCells {
    grid: ScreenGrid,
    taken: Vec<u64>,
}

impl ScreenCells {
    fn over(view: Vector2, cell: f64) -> Self {
        let grid = ScreenGrid::over(view, cell);
        Self {
            taken: vec![0; grid.len().div_ceil(64)],
            grid,
        }
    }

    fn take(&mut self, at: Vector2) -> bool {
        let Some(index) = self.grid.index(at) else {
            return false;
        };
        let Some(word) = self.taken.get_mut(index / 64) else {
            return false;
        };
        let bit = 1_u64 << (index % 64);
        let free = *word & bit == 0;
        *word |= bit;
        free
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RingTally {
    rings: usize,
    ends: usize,
    sum: Vector2,
}

fn open_end_marks(ends: impl Iterator<Item = Vector2>, view: Vector2) -> OpenEndMarks {
    let mut fine = ScreenCells::over(view, RING_MERGE);
    let coarse = ScreenGrid::over(view, RING_CLUSTER_CELL);
    let mut tallies = vec![RingTally::default(); coarse.len()];
    let mut rings = Vec::new();
    for at in ends {
        let Some((cell, tally)) = coarse
            .index(at)
            .and_then(|cell| Some((cell, tallies.get_mut(cell)?)))
        else {
            continue;
        };
        tally.ends += 1;
        tally.sum += at;
        if fine.take(at) {
            tally.rings += 1;
            rings.push((at, cell));
        }
    }
    let dense = |tally: &RingTally| tally.rings > MOST_RINGS_PER_CELL;
    OpenEndMarks {
        rings: rings
            .into_iter()
            .filter(|(_, cell)| !tallies.get(*cell).is_some_and(dense))
            .map(|(at, _)| at)
            .collect(),
        clusters: tallies
            .iter()
            .filter(|tally| dense(tally))
            .map(|tally| RingCluster {
                center: tally.sum / tally.ends as f64,
                ends: tally.ends,
            })
            .collect(),
    }
}

fn place_glyphs(
    groups: &[GlyphGroup],
    screen: &impl Screen,
    key: &ViewKey,
    screen_centre: Option<Vector2>,
    mut blocked: Obstacles,
) -> Vec<GlyphMark> {
    let half = Vector2::splat(f64::from(GLYPH_SIZE / 2.0 + GLYPH_CLEARANCE));
    let view = Vector2::new(f64::from(key.rect.width()), f64::from(key.rect.height()));
    let (kept, thinned): (Vec<_>, Vec<_>) = groups
        .iter()
        .partition(|group| key.group_thinning(group) == Thinning::Never);
    let ordered = kept
        .into_iter()
        .map(|group| (group, Thinning::Never))
        .chain(
            thinned
                .into_iter()
                .map(|group| (group, Thinning::WhenCrowded)),
        );
    let mut glyphs = Vec::new();
    for (group, thinning) in ordered {
        let Some(anchor) = group.site.and_then(|site| site.on_screen(screen)) else {
            continue;
        };
        let collapsible = thinning == Thinning::WhenCrowded
            && group.items.iter().all(|item| item.standing.collapsible());
        if collapsible && annotation_layout::collapses(anchor) {
            continue;
        }
        let Some(place) = annotation_layout::within_view(anchor, view) else {
            continue;
        };
        let (stacked, hidden) = if group.items.len() > MAX_STACKED {
            group.items.split_at(MAX_STACKED - 1)
        } else {
            (group.items.as_slice(), &[][..])
        };
        let slots = stacked.len() + usize::from(!hidden.is_empty());
        let Some(mut positions) =
            annotation_layout::place_glyphs(place, slots, screen_centre, half, &blocked, thinning)
        else {
            continue;
        };
        for center in &positions {
            blocked.add(Footprint {
                center: *center,
                half,
            });
        }
        let beyond_at = (!hidden.is_empty()).then(|| positions.pop()).flatten();
        glyphs.extend(
            stacked
                .iter()
                .zip(positions)
                .map(|(item, center)| GlyphMark {
                    constraint: item.constraint,
                    anchor: group.anchor,
                    kind: item.kind,
                    center,
                    standing: item.standing,
                    hover: Hover::Glyph(item.constraint),
                }),
        );
        if let (Some(center), Some(first)) = (beyond_at, hidden.first()) {
            glyphs.push(GlyphMark {
                constraint: first.constraint,
                anchor: group.anchor,
                kind: first.kind,
                center,
                standing: first.standing,
                hover: Hover::Beyond(hidden.iter().map(|item| item.constraint).collect()),
            });
        }
    }
    glyphs
}

struct Standings<'a> {
    conflicting: Vec<ConstraintId>,
    inactive: Vec<ConstraintId>,
    solution: Option<&'a SketchSolution>,
}

impl<'a> Standings<'a> {
    fn load(model: &'a Model, feature: FeatureId, definition: &Sketch) -> Self {
        let conflicting = match model
            .evaluation()
            .feature(feature)
            .map(|status| &status.state)
        {
            Some(FeatureState::Failed(error)) => error.constraints.clone(),
            Some(
                FeatureState::UpToDate
                | FeatureState::Outdated
                | FeatureState::Suppressed
                | FeatureState::RolledBack,
            )
            | None => Vec::new(),
        };
        Self {
            conflicting,
            inactive: definition.inactive().collect(),
            solution: sketch_status::up_to_date_solution(model.evaluation(), feature),
        }
    }

    fn of(&self, constraint: ConstraintId) -> Standing {
        if self.inactive.contains(&constraint) {
            Standing::Inactive
        } else if self.conflicting.contains(&constraint) {
            Standing::Conflicting
        } else if self
            .solution
            .and_then(|solution| solution.redundancy(constraint))
            .is_some()
        {
            Standing::Redundant
        } else {
            Standing::Normal
        }
    }
}

fn extent_of(sketch: &Sketch) -> f64 {
    sketch
        .entities()
        .filter_map(|(_, entity)| match entity {
            Entity::Point(position) => Some(position.x.abs().max(position.y.abs())),
            Entity::Line { .. }
            | Entity::Circle { .. }
            | Entity::Arc { .. }
            | Entity::Spline { .. }
            | Entity::Ellipse { .. }
            | Entity::EllipticalArc { .. } => None,
        })
        .fold(0.0, f64::max)
}

fn centre_of(sketch: &Sketch) -> Option<Point2> {
    let points: Vec<Point2> = sketch
        .entities()
        .filter_map(|(_, entity)| match entity {
            Entity::Point(position) => Some(*position),
            Entity::Line { .. }
            | Entity::Circle { .. }
            | Entity::Arc { .. }
            | Entity::Spline { .. }
            | Entity::Ellipse { .. }
            | Entity::EllipticalArc { .. } => None,
        })
        .collect();
    let first = *points.first()?;
    let (low, high) = points.iter().fold((first, first), |(low, high), point| {
        (low.min(*point), high.max(*point))
    });
    Some((low + high) / 2.0)
}

fn label_text(
    model: &Model,
    shown: &Sketch,
    id: ConstraintId,
    constraint: &Constraint,
    expression: &Expression,
) -> String {
    let measured = shown
        .measured(constraint)
        .filter(|_| !shown.is_active(id))
        .and_then(|value| {
            let quantity = match constraint.dimension_kind()? {
                Dimension::ANGLE => Quantity::angle(value),
                Dimension::NONE => Quantity::plain(value),
                _ => Quantity::length(value),
            };
            Some(model.units().show(quantity))
        });
    let text = match measured {
        Some(value) => format!("({value})"),
        None => {
            let text = model.document().expression_text(expression);
            match field::value_preview(model.parameters(), expression, model.units()) {
                Some(value) => format!("{text} {value}"),
                None => text,
            }
        }
    };
    match constraint {
        Constraint::Radius { .. }
        | Constraint::MajorRadius { .. }
        | Constraint::MinorRadius { .. } => format!("{RADIUS_PREFIX}{text}"),
        Constraint::Rho { .. } => format!("{RHO_PREFIX}{text}"),
        Constraint::Diameter { .. } | Constraint::AxisDiameter { .. } => {
            format!("{DIAMETER_PREFIX}{text}")
        }
        _ => text,
    }
}

pub struct Surface<'a> {
    pub rect: Rect,
    pub screen: &'a SketchScreen,
    pub feature: FeatureId,
    pub interactive: bool,
    pub glyphs: bool,
    pub highlight: Option<Pickable>,
    pub first_dimension_scales: bool,
    pub outside: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenField {
    feature: FeatureId,
    constraint: ConstraintId,
    focus_pending: bool,
    select_all_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FieldRequest {
    feature: FeatureId,
    constraint: ConstraintId,
    frames_left: u8,
}

#[derive(Debug, Clone, Default)]
pub struct Annotations {
    hovered: Option<Pickable>,
    field: Option<OpenField>,
    request: Option<FieldRequest>,
    texts: LabelTexts,
    dragging: Option<LabelDrag>,
    dropped: Option<LabelDrag>,
    measures: Option<Measures>,
    marks: Option<Marks>,
    #[cfg(test)]
    layouts: usize,
}

struct Placed {
    pickable: Option<Pickable>,
    hit: Rect,
    key: MarkKey,
    hover: Hover,
    label: Option<(Vector2, LabelFrame)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MarkKey {
    Constraint(ConstraintId, Option<EntityId>),
    OpenEnds(usize),
}

impl MarkKey {
    fn constraint(self) -> Option<ConstraintId> {
        match self {
            Self::Constraint(constraint, _) => Some(constraint),
            Self::OpenEnds(_) => None,
        }
    }
}

fn annotation_id(feature: FeatureId, key: MarkKey) -> Id {
    Id::new(("sketch-annotation", feature, key))
}

struct PointerReach {
    pointers: Vec<Pos2>,
    dragged: Option<ConstraintId>,
    focused: Option<Id>,
}

impl PointerReach {
    fn of(ui: &Ui, dragging: Option<LabelDrag>) -> Self {
        let pointers = ui.input(|input| {
            [
                input.pointer.hover_pos(),
                input.pointer.interact_pos(),
                input.pointer.press_origin(),
            ]
            .into_iter()
            .flatten()
            .collect()
        });
        Self {
            pointers,
            dragged: dragging.map(|drag| drag.constraint),
            focused: ui.memory(|memory| memory.focused()),
        }
    }

    fn reaches(&self, feature: FeatureId, target: &Placed) -> bool {
        let near = target.hit.expand(POINTER_REACH);
        self.pointers.iter().any(|pointer| near.contains(*pointer))
            || (target.label.is_some()
                && self.dragged.is_some()
                && self.dragged == target.key.constraint())
            || self
                .focused
                .is_some_and(|focused| focused == annotation_id(feature, target.key))
    }
}

impl Annotations {
    pub fn hovered(&self) -> Option<Pickable> {
        self.hovered
    }

    pub fn label_place(&self, feature: FeatureId, constraint: ConstraintId) -> Option<LabelPlace> {
        let mark = self
            .marks
            .as_ref()
            .filter(|marks| marks.key.sketch.feature == feature)?
            .dimension(constraint)?;
        Some(LabelPlace {
            at: mark.layout.at,
            frame: mark.frame?,
        })
    }

    #[cfg(test)]
    pub fn layouts(&self) -> usize {
        self.layouts
    }

    #[cfg(test)]
    pub fn laid_out(&self) -> Vec<ConstraintId> {
        self.marks
            .iter()
            .flat_map(|marks| &marks.dimensions)
            .map(|mark| mark.constraint)
            .collect()
    }

    #[cfg(test)]
    pub fn collapsed(&self) -> Vec<(ConstraintId, Vector2)> {
        self.marks
            .iter()
            .flat_map(|marks| &marks.collapsed)
            .map(|mark| (mark.constraint, mark.center))
            .collect()
    }

    #[cfg(test)]
    pub fn open_end_rings(&self) -> (Vec<Vector2>, Vec<(Vector2, usize)>) {
        self.marks.as_ref().map_or_else(Default::default, |marks| {
            (
                marks.open_ends.rings.clone(),
                marks
                    .open_ends
                    .clusters
                    .iter()
                    .map(|cluster| (cluster.center, cluster.ends))
                    .collect(),
            )
        })
    }

    #[cfg(test)]
    pub fn glyph_centres(&self) -> Vec<(EntityId, Vector2)> {
        self.marks
            .iter()
            .flat_map(|marks| &marks.glyphs)
            .map(|mark| (mark.anchor, mark.center))
            .collect()
    }

    pub fn shown_dimensions(&self) -> Vec<ConstraintId> {
        let Some(marks) = &self.marks else {
            return Vec::new();
        };
        let mut shown: Vec<ConstraintId> = marks
            .dimensions
            .iter()
            .filter(|mark| mark.label.is_some())
            .map(|mark| mark.constraint)
            .chain(marks.collapsed.iter().map(|mark| mark.constraint))
            .collect();
        shown.sort_unstable();
        shown.dedup();
        shown
    }

    pub fn request_field(&mut self, feature: FeatureId, constraint: ConstraintId) {
        self.request = Some(FieldRequest {
            feature,
            constraint,
            frames_left: REQUEST_FRAMES,
        });
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn forget_marks(&mut self) {
        self.field = None;
        self.measures = None;
        self.marks = None;
    }

    fn forced(&self, selection: &Selection, surface: &Surface<'_>) -> Vec<ConstraintId> {
        let feature = surface.feature;
        let mut forced: Vec<ConstraintId> = selection
            .iter()
            .chain(surface.highlight)
            .filter_map(|pickable| match pickable {
                Pickable::SketchConstraint {
                    feature: owner,
                    constraint,
                } if owner == feature => Some(constraint),
                _ => None,
            })
            .chain(self.field.map(|open| open.constraint))
            .chain(self.request.map(|request| request.constraint))
            .collect();
        forced.sort_unstable();
        forced.dedup();
        forced
    }

    fn kept(selection: &Selection, feature: FeatureId) -> Vec<EntityId> {
        let mut kept: Vec<EntityId> = selection
            .iter()
            .filter_map(|pickable| match pickable {
                Pickable::SketchEntity {
                    feature: owner,
                    entity,
                } if owner == feature => Some(entity),
                _ => None,
            })
            .collect();
        kept.sort_unstable();
        kept.dedup();
        kept
    }

    fn refresh_marks(
        &mut self,
        painter: &egui::Painter,
        model: &Model,
        surface: &Surface<'_>,
        selection: &Selection,
    ) -> Option<Marks> {
        let dragged = self
            .dragging
            .and_then(|drag| Some((drag.constraint, drag.offset?)));
        let owner = model.document().feature(surface.feature)?;
        let definition = owner.kind.sketch()?;
        let shown = model.displayed_sketch(owner)?;
        self.texts.refresh(painter, model, surface.feature);
        let sketch = SketchKey::of(
            model,
            surface.feature,
            dragged.map(|(id, _)| id),
            surface.outside,
        );
        let measures = match self.measures.take() {
            Some(measures) if measures.key == sketch => measures,
            _ => Measures::collect(model, sketch, definition, &shown),
        };
        let key = ViewKey {
            sketch,
            screen: *surface.screen,
            rect: surface.rect,
            units: model.units(),
            dragged: dragged.map(|(_, offset)| offset),
            editing: self.field.map(|open| open.constraint),
            forced: self.forced(selection, surface),
            kept: Self::kept(selection, surface.feature),
            glyphs: surface.glyphs,
        };
        let marks = match self.marks.take() {
            Some(marks) if marks.key == key => marks,
            _ => {
                #[cfg(test)]
                {
                    self.layouts += 1;
                }
                Marks::lay_out(
                    painter,
                    model,
                    &measures,
                    [definition, &shown],
                    key,
                    &mut self.texts,
                )
            }
        };
        self.measures = Some(measures);
        Some(marks)
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        surface: &Surface<'_>,
        selection: &mut Selection,
        actions: &mut Vec<Action>,
    ) {
        self.hovered = None;
        self.dragging = self
            .dragging
            .filter(|drag| drag.feature == surface.feature && surface.interactive);
        self.field = self.field.filter(|open| open.feature == surface.feature);
        self.request = self
            .request
            .filter(|request| request.feature == surface.feature);
        let painter = ui.painter_at(surface.rect);
        let Some(marks) = self.refresh_marks(&painter, model, surface, selection) else {
            self.forget_marks();
            return;
        };
        self.open_requested(model, &marks);

        let labels: Vec<Option<(Arc<Galley>, Rect)>> = marks
            .dimensions
            .iter()
            .map(|mark| {
                mark.label.map(|rect| {
                    let galley = painter.layout_no_wrap(
                        mark.text.clone(),
                        canvas::body(),
                        Color32::PLACEHOLDER,
                    );
                    (galley, rect)
                })
            })
            .collect();
        let pickable = |constraint| Pickable::SketchConstraint {
            feature: surface.feature,
            constraint,
        };
        let placed = marks
            .dimensions
            .iter()
            .filter_map(|mark| {
                mark.label.map(|rect| Placed {
                    pickable: Some(pickable(mark.constraint)),
                    hit: rect,
                    key: MarkKey::Constraint(mark.constraint, None),
                    hover: Hover::Dimension(mark.constraint),
                    label: mark.frame.map(|frame| {
                        let centre = rect.center() - surface.rect.min;
                        (
                            Vector2::new(f64::from(centre.x), f64::from(centre.y)),
                            frame,
                        )
                    }),
                })
            })
            .chain(marks.collapsed.iter().map(|mark| Placed {
                pickable: Some(pickable(mark.constraint)),
                hit: Rect::from_center_size(
                    to_pos(surface.rect, mark.center),
                    egui::Vec2::splat(COLLAPSED_HIT_SIZE),
                ),
                key: MarkKey::Constraint(mark.constraint, None),
                hover: Hover::Collapsed(mark.constraint),
                label: None,
            }))
            .chain(marks.glyphs.iter().map(|mark| Placed {
                pickable: Some(pickable(mark.constraint)),
                hit: Rect::from_center_size(
                    to_pos(surface.rect, mark.center),
                    egui::Vec2::splat(GLYPH_HIT_SIZE),
                ),
                key: MarkKey::Constraint(mark.constraint, Some(mark.anchor)),
                hover: mark.hover.clone(),
                label: None,
            }))
            .chain(
                marks
                    .open_ends
                    .clusters
                    .iter()
                    .enumerate()
                    .map(|(index, cluster)| Placed {
                        pickable: None,
                        hit: Rect::from_center_size(
                            to_pos(surface.rect, cluster.center),
                            egui::Vec2::splat(CLUSTER_RADIUS * 2.0),
                        ),
                        key: MarkKey::OpenEnds(index),
                        hover: Hover::OpenEnds(cluster.ends),
                        label: None,
                    }),
            );
        if surface.interactive
            && let Some(definition) = model
                .document()
                .feature(surface.feature)
                .and_then(|owner| owner.kind.sketch())
        {
            let reach = PointerReach::of(ui, self.dragging);
            for target in placed.filter(|target| reach.reaches(surface.feature, target)) {
                self.interact(ui, surface, target, selection, model.document(), definition);
            }
        }
        if let Some(drop) = self.dropped.take()
            && let Some(offset) = drop.offset
        {
            let mut transaction = sketch_tools::settled_transaction(
                model,
                drop.feature,
                MOVE_LABEL_TRANSACTION.to_owned(),
            );
            transaction.set_sketch_label(drop.feature, drop.constraint, Some(offset));
            actions.push(Action::Apply(transaction.finish()));
        }

        let chrome = canvas::chrome(ui.ctx());
        let color = |constraint, standing| {
            let pickable = pickable(constraint);
            if self.hovered == Some(pickable) || surface.highlight == Some(pickable) {
                chrome.hovered
            } else if selection.contains(pickable) {
                chrome.selected
            } else {
                match standing {
                    Standing::Normal => chrome.dimension,
                    Standing::Conflicting => chrome.error,
                    Standing::Redundant => chrome.warning,
                    Standing::Inactive => chrome.muted,
                }
            }
        };
        let framed = |standing: Standing| {
            appearance::is_high_contrast(ui)
                .then(|| standing.frame())
                .flatten()
        };
        for mark in &marks.collapsed {
            painter.circle_filled(
                to_pos(surface.rect, mark.center),
                COLLAPSED_RADIUS,
                color(mark.constraint, mark.standing),
            );
        }
        for (mark, label) in marks.dimensions.iter().zip(labels) {
            let tint = color(mark.constraint, mark.standing);
            let frame = framed(mark.standing).zip(label.as_ref().map(|(_, rect)| *rect));
            paint_dimension(&painter, surface.rect, &mark.layout, label, tint);
            if let Some((frame, rect)) = frame {
                paint_frame(&painter, rect, frame, tint);
            }
        }
        for mark in &marks.glyphs {
            let center = to_pos(surface.rect, mark.center);
            let tint = color(mark.constraint, mark.standing);
            match &mark.hover {
                Hover::Beyond(hidden) => paint_beyond(&painter, center, hidden.len(), tint),
                Hover::Dimension(_)
                | Hover::Collapsed(_)
                | Hover::Glyph(_)
                | Hover::OpenEnds(_) => {
                    paint_glyph(&painter, center, mark.kind, tint);
                    if let Some(frame) = framed(mark.standing) {
                        let glyph = Rect::from_center_size(center, egui::Vec2::splat(GLYPH_SIZE));
                        paint_frame(&painter, glyph, frame, tint);
                    }
                }
            }
        }
        for [end, at] in &marks.beyond {
            let (end, at) = (to_pos(surface.rect, *end), to_pos(surface.rect, *at));
            painter.extend(Shape::dashed_line(
                &[end, at],
                Stroke::new(STROKE_WIDTH, chrome.muted),
                BEYOND_DASH,
                BEYOND_DASH,
            ));
            painter.circle_stroke(at, BEYOND_RADIUS, Stroke::new(RING_WIDTH, chrome.muted));
        }
        let ring = Stroke::new(RING_WIDTH, chrome.warning);
        for end in &marks.open_ends.rings {
            painter.circle_stroke(to_pos(surface.rect, *end), OPEN_END_RADIUS, ring);
        }
        for cluster in &marks.open_ends.clusters {
            let center = to_pos(surface.rect, cluster.center);
            painter.circle_stroke(center, OPEN_END_RADIUS, ring);
            painter.circle_stroke(center, CLUSTER_RADIUS, ring);
        }
        self.show_field(ui, model, surface, &marks, actions);
        self.marks = Some(marks);
    }

    fn open_requested(&mut self, model: &Model, marks: &Marks) {
        let Some(request) = self.request else {
            return;
        };
        if marks.dimension(request.constraint).is_some() {
            self.open(request.feature, request.constraint);
            self.request = None;
            return;
        }
        let exists = model
            .document()
            .feature(request.feature)
            .and_then(|owner| owner.kind.sketch())
            .is_some_and(|sketch| sketch.constraint(request.constraint).is_some());
        self.request = if exists {
            Some(request)
        } else {
            request
                .frames_left
                .checked_sub(1)
                .map(|frames_left| FieldRequest {
                    frames_left,
                    ..request
                })
        };
    }

    fn open(&mut self, feature: FeatureId, constraint: ConstraintId) {
        self.field = Some(OpenField {
            feature,
            constraint,
            focus_pending: true,
            select_all_pending: true,
        });
    }

    fn interact(
        &mut self,
        ui: &Ui,
        surface: &Surface<'_>,
        target: Placed,
        selection: &mut Selection,
        document: &Document,
        definition: &Sketch,
    ) {
        let hit = target.hit.intersect(surface.rect);
        if !hit.is_positive() {
            return;
        }
        let sense = match (target.pickable, target.label) {
            (Some(_), Some(_)) if !surface.outside => Sense::click_and_drag(),
            (Some(_), _) => Sense::CLICK,
            (None, _) => Sense::hover(),
        };
        let response = ui.interact(hit, annotation_id(surface.feature, target.key), sense);
        if let Some(pickable) = target.pickable {
            self.pick(ui, surface, &target, pickable, &response, selection);
            Self::offer(document, surface.feature, &target, &response);
        }
        response.on_hover_ui(|ui| {
            ui.label(target.hover.describe(definition, surface.outside));
        });
    }

    fn pick(
        &mut self,
        ui: &Ui,
        surface: &Surface<'_>,
        target: &Placed,
        pickable: Pickable,
        response: &egui::Response,
        selection: &mut Selection,
    ) {
        if response.hovered() {
            self.hovered = Some(pickable);
        }
        if let (Some((label, frame)), Pickable::SketchConstraint { constraint, .. }) =
            (target.label, pickable)
            && !surface.outside
        {
            self.drag_label(surface, response, constraint, label, frame);
        }
        if response.clicked() && !surface.outside && !completion::take_click(ui.ctx()) {
            let toggle = ui.input(|input| input.modifiers.shift || input.modifiers.command);
            if toggle {
                selection.toggle(pickable);
            } else {
                selection.replace_with(pickable);
            }
        }
        if matches!(target.hover, Hover::Dimension(_) | Hover::Collapsed(_))
            && response.double_clicked()
            && let Pickable::SketchConstraint {
                feature,
                constraint,
            } = pickable
        {
            self.open(feature, constraint);
        }
    }

    fn offer(document: &Document, feature: FeatureId, target: &Placed, response: &egui::Response) {
        let (Hover::Dimension(constraint) | Hover::Collapsed(constraint)) = target.hover else {
            return;
        };
        if !response.hovered() {
            return;
        }
        let expression = document
            .feature(feature)
            .and_then(|owner| owner.kind.sketch())
            .and_then(|sketch| sketch.constraint(constraint))
            .and_then(Constraint::dimension);
        if let Some(expression) = expression {
            completion::offer_insertion(
                &response.ctx,
                completion::dimension_text(document, expression),
            );
        }
    }

    fn drag_label(
        &mut self,
        surface: &Surface<'_>,
        response: &egui::Response,
        constraint: ConstraintId,
        label: Vector2,
        frame: LabelFrame,
    ) {
        let within = |position: Pos2| {
            let offset = position - surface.rect.min;
            Vector2::new(f64::from(offset.x), f64::from(offset.y))
        };
        let pointer = response.interact_pointer_pos().map(within);
        let pressed = response
            .ctx
            .input(|input| input.pointer.press_origin())
            .map(within);
        if response.drag_started()
            && let Some(pressed) = pressed
        {
            self.dragging = Some(LabelDrag {
                feature: surface.feature,
                constraint,
                grab: label - pressed,
                offset: None,
            });
        }
        let Some(mut drag) = self.dragging.filter(|drag| drag.constraint == constraint) else {
            return;
        };
        if response.dragged()
            && let Some(at) =
                pointer.and_then(|pointer| surface.screen.to_sketch(pointer + drag.grab))
        {
            drag.offset = Some(frame.offset_of(at));
            self.dragging = Some(drag);
        }
        if response.drag_stopped() {
            self.dragging = None;
            self.dropped = Some(drag);
        }
    }

    fn show_field(
        &mut self,
        ui: &Ui,
        model: &Model,
        surface: &Surface<'_>,
        marks: &Marks,
        actions: &mut Vec<Action>,
    ) {
        let Some(mut open) = self.field else {
            return;
        };
        let document = model.document();
        let expression = document
            .feature(open.feature)
            .and_then(|owner| owner.kind.sketch())
            .and_then(|sketch| sketch.constraint(open.constraint))
            .and_then(Constraint::dimension);
        let (Some(expression), Some(mark)) = (expression, marks.dimension(open.constraint)) else {
            self.field = None;
            return;
        };
        let id = field_id(open.feature, open.constraint);
        let target = DimensionTarget {
            feature: open.feature,
            constraint: open.constraint,
        };
        let stored = if surface.outside {
            field::driven_text(document, &target.owner(), expression)
        } else {
            field::value_text(document, &target.owner(), expression)
        };
        if let Some(held) = document.owned_parameter(&target.owner(), expression) {
            completion::editing_parameter(ui, id, held.id());
        }
        let field = egui::Area::new(id.with("area"))
            .order(Order::Foreground)
            .fixed_pos(to_pos(surface.rect, mark.layout.label) - vec2(0.0, FIELD_LIFT))
            .pivot(Align2::CENTER_TOP)
            .constrain_to(surface.rect)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(FIELD_MARGIN)
                    .show(ui, |ui| {
                        let field = field::value_field(
                            ui,
                            id,
                            &stored,
                            FIELD_WIDTH,
                            open.focus_pending,
                            |text| {
                                if surface.outside {
                                    field::dimension_edit(
                                        document,
                                        model.parameters(),
                                        (target, Reference::Followed),
                                        text,
                                        model.units(),
                                    )
                                } else {
                                    sketch_tools::dimension_change(
                                        model,
                                        target,
                                        text,
                                        surface.first_dimension_scales,
                                    )
                                }
                            },
                        );
                        if let Some(error) = &field.error {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                        field
                    })
                    .inner
            })
            .inner;
        if let Some(transaction) = field.committed {
            actions.push(Action::Apply(transaction));
        }
        let focused = field.response.has_focus();
        if focused {
            open.focus_pending = false;
            if open.select_all_pending {
                field::select_all(ui.ctx(), id, &stored);
                open.select_all_pending = false;
            }
        }
        let lost_focus = field.left;
        let entered = ui.input(|input| input.key_pressed(Key::Enter));
        if lost_focus && field.error.is_some() && entered {
            open.focus_pending = true;
        }
        self.field = (!lost_focus || field.error.is_some()).then_some(open);
    }
}

fn to_pos(rect: Rect, point: Vector2) -> Pos2 {
    rect.min + vec2(point.x as f32, point.y as f32)
}

fn to_vec(vector: Vector2) -> egui::Vec2 {
    vec2(vector.x as f32, vector.y as f32)
}

fn to_vector(size: egui::Vec2) -> Vector2 {
    Vector2::new(f64::from(size.x), f64::from(size.y))
}

fn footprint(surface: Rect, rect: Rect) -> Footprint {
    let center = rect.center() - surface.min;
    let half = rect.size() / 2.0;
    Footprint {
        center: Vector2::new(f64::from(center.x), f64::from(center.y)),
        half: Vector2::new(f64::from(half.x), f64::from(half.y)),
    }
}

fn label_rect(rect: Rect, layout: &DimensionLayout, text: egui::Vec2) -> Rect {
    let size = canvas::chip_size(text);
    let side = to_vec(layout.label_side);
    let reach = if side == egui::Vec2::ZERO {
        0.0
    } else {
        side.x.abs() * size.x / 2.0 + side.y.abs() * size.y / 2.0 + LABEL_GAP
    };
    Rect::from_center_size(to_pos(rect, layout.label) + side * reach, size)
}

fn paint_dimension(
    painter: &egui::Painter,
    rect: Rect,
    layout: &DimensionLayout,
    label: Option<(Arc<Galley>, Rect)>,
    color: Color32,
) {
    let stroke = Stroke::new(STROKE_WIDTH, color);
    for points in &layout.strokes {
        let points = points.iter().map(|point| to_pos(rect, *point)).collect();
        painter.add(Shape::line(points, stroke));
    }
    for arrow in &layout.arrows {
        let tip = to_pos(rect, arrow.tip);
        let direction = to_vec(arrow.direction);
        let back = tip - direction * ARROW_LENGTH;
        let side = direction.rot90() * ARROW_HALF_WIDTH;
        painter.add(Shape::convex_polygon(
            vec![tip, back + side, back - side],
            color,
            Stroke::NONE,
        ));
    }
    if let Some((galley, label)) = label {
        canvas::paint_backdrop(painter, label);
        painter.galley(label.min + canvas::PADDING, galley, color);
    }
}

fn paint_frame(painter: &egui::Painter, rect: Rect, frame: Frame, color: Color32) {
    let outline = rect.expand(FRAME_GAP);
    let stroke = Stroke::new(FRAME_WIDTH, color);
    match frame {
        Frame::Solid => {
            painter.rect_stroke(outline, canvas::RADIUS, stroke, StrokeKind::Outside);
        }
        Frame::Dashed => {
            let corners = [
                outline.left_top(),
                outline.right_top(),
                outline.right_bottom(),
                outline.left_bottom(),
                outline.left_top(),
            ];
            painter.extend(Shape::dashed_line(
                &corners,
                stroke,
                FRAME_DASH,
                FRAME_DASH_GAP,
            ));
        }
    }
}

pub fn glyph_letter(kind: GlyphKind) -> Option<&'static str> {
    match kind {
        GlyphKind::Horizontal => Some("H"),
        GlyphKind::Vertical => Some("V"),
        GlyphKind::Equal => Some("="),
        GlyphKind::Curvature => Some("κ"),
        GlyphKind::Parallel
        | GlyphKind::Perpendicular
        | GlyphKind::Tangent
        | GlyphKind::Coincident
        | GlyphKind::OnCurve
        | GlyphKind::Midpoint
        | GlyphKind::Concentric
        | GlyphKind::Collinear
        | GlyphKind::Symmetric
        | GlyphKind::Fix => None,
    }
}

fn beyond_description(sketch: &Sketch, hidden: &[ConstraintId]) -> String {
    let listed: Vec<String> = hidden
        .iter()
        .take(LISTED_BEYOND)
        .map(|constraint| sketch.describe_constraint(*constraint))
        .collect();
    let unlisted = hidden.len().saturating_sub(LISTED_BEYOND);
    let more = if unlisted > 0 {
        format!("\nand {unlisted} more")
    } else {
        String::new()
    };
    format!(
        "{} more constraints here:\n{}{more}",
        hidden.len(),
        listed.join("\n")
    )
}

fn paint_beyond(painter: &egui::Painter, center: Pos2, count: usize, color: Color32) {
    let galley = painter.layout_no_wrap(
        format!("+{count}"),
        canvas::emphasis(),
        Color32::PLACEHOLDER,
    );
    canvas::paint_backdrop(
        painter,
        Rect::from_center_size(center, galley.size().max(egui::Vec2::splat(GLYPH_SIZE))),
    );
    painter.galley(center - galley.size() / 2.0, galley, color);
}

fn paint_glyph(painter: &egui::Painter, center: Pos2, kind: GlyphKind, color: Color32) {
    let stroke = Stroke::new(SYMBOL_WIDTH, color);
    let at = |x: f32, y: f32| center + vec2(x, y);
    match kind {
        GlyphKind::Coincident => {
            painter.circle_filled(center, DOT_RADIUS, color);
            return;
        }
        GlyphKind::OnCurve => {
            painter.circle_stroke(center, DOT_RADIUS, Stroke::new(RING_WIDTH, color));
            return;
        }
        GlyphKind::Horizontal
        | GlyphKind::Vertical
        | GlyphKind::Equal
        | GlyphKind::Curvature
        | GlyphKind::Parallel
        | GlyphKind::Perpendicular
        | GlyphKind::Tangent
        | GlyphKind::Midpoint
        | GlyphKind::Concentric
        | GlyphKind::Collinear
        | GlyphKind::Symmetric
        | GlyphKind::Fix => {}
    }
    canvas::paint_backdrop(
        painter,
        Rect::from_center_size(center, egui::Vec2::splat(GLYPH_SIZE)),
    );
    if let Some(letter) = glyph_letter(kind) {
        let galley =
            painter.layout_no_wrap(letter.to_owned(), canvas::emphasis(), Color32::PLACEHOLDER);
        painter.galley(center - galley.size() / 2.0, galley, color);
        return;
    }
    match kind {
        GlyphKind::Parallel => {
            for gap in [-PARALLEL_GAP, PARALLEL_GAP] {
                painter.line_segment(
                    [
                        at(gap - PARALLEL_LEAN, SYMBOL_HALF),
                        at(gap + PARALLEL_LEAN, -SYMBOL_HALF),
                    ],
                    stroke,
                );
            }
        }
        GlyphKind::Perpendicular => {
            painter.line_segment([at(0.0, -SYMBOL_HALF), at(0.0, SYMBOL_HALF)], stroke);
            painter.line_segment(
                [at(-SYMBOL_HALF, SYMBOL_HALF), at(SYMBOL_HALF, SYMBOL_HALF)],
                stroke,
            );
        }
        GlyphKind::Tangent => {
            let touching = TANGENT_RADIUS - TANGENT_LIFT;
            painter.circle_stroke(at(0.0, -TANGENT_LIFT), TANGENT_RADIUS, stroke);
            painter.line_segment(
                [at(-SYMBOL_HALF, touching), at(SYMBOL_HALF, touching)],
                stroke,
            );
        }
        GlyphKind::Midpoint => {
            painter.line_segment([at(-SYMBOL_HALF, 0.0), at(SYMBOL_HALF, 0.0)], stroke);
            painter.circle_filled(center, MIDPOINT_DOT, color);
        }
        GlyphKind::Concentric => {
            painter.circle_stroke(center, SYMBOL_HALF, stroke);
            painter.circle_stroke(center, INNER_RING, stroke);
        }
        GlyphKind::Collinear => {
            for side in [-1.0, 1.0] {
                painter.line_segment(
                    [
                        at(side * DASH_GAP, -side * DASH_GAP),
                        at(side * SYMBOL_HALF, -side * SYMBOL_HALF),
                    ],
                    stroke,
                );
            }
        }
        GlyphKind::Symmetric => {
            painter.line_segment([at(0.0, -SYMBOL_HALF), at(0.0, SYMBOL_HALF)], stroke);
            for side in [-1.0, 1.0] {
                painter.add(Shape::convex_polygon(
                    vec![
                        at(side * SYMBOL_HALF, -SYMBOL_HALF + MIRROR_TIP),
                        at(side * SYMBOL_HALF, SYMBOL_HALF - MIRROR_TIP),
                        at(side * MIRROR_TIP, 0.0),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        GlyphKind::Fix => {
            painter.rect_filled(
                Rect::from_min_max(
                    at(-LOCK_HALF_WIDTH, LOCK_TOP),
                    at(LOCK_HALF_WIDTH, LOCK_BOTTOM),
                ),
                1.0,
                color,
            );
            let shackle = (0..=SHACKLE_STEPS)
                .map(|step| {
                    let angle = std::f32::consts::PI * step as f32 / SHACKLE_STEPS as f32;
                    at(
                        -SHACKLE_RADIUS * angle.cos(),
                        LOCK_TOP - SHACKLE_RADIUS * angle.sin(),
                    )
                })
                .collect();
            painter.add(Shape::line(shackle, stroke));
        }
        GlyphKind::Horizontal
        | GlyphKind::Vertical
        | GlyphKind::Equal
        | GlyphKind::Curvature
        | GlyphKind::Coincident
        | GlyphKind::OnCurve => {}
    }
}

#[cfg(test)]
mod tests {
    use egui::RawInput;

    use super::*;

    #[test]
    fn every_glyph_letter_and_label_prefix_renders_with_the_app_fonts() {
        let context = egui::Context::default();
        context.set_fonts(crate::fonts::definitions_with(&[]));
        let mut output = context.run_ui(RawInput::default(), |_| {});
        output.textures_delta.clear();
        let kinds = [
            GlyphKind::Horizontal,
            GlyphKind::Vertical,
            GlyphKind::Parallel,
            GlyphKind::Perpendicular,
            GlyphKind::Tangent,
            GlyphKind::Curvature,
            GlyphKind::Equal,
            GlyphKind::Coincident,
            GlyphKind::OnCurve,
            GlyphKind::Midpoint,
            GlyphKind::Concentric,
            GlyphKind::Collinear,
            GlyphKind::Symmetric,
            GlyphKind::Fix,
        ];
        let texts = kinds.into_iter().filter_map(glyph_letter).chain([
            RADIUS_PREFIX,
            DIAMETER_PREFIX,
            "0123456789.,+-*/()= mm deg °",
        ]);
        for text in texts {
            let renders = context.fonts_mut(|fonts| fonts.has_glyphs(&canvas::body(), text.trim()));
            assert!(renders, "'{text}' cannot be drawn with the app fonts");
        }
    }

    #[test]
    fn open_end_rings_closer_than_half_their_radius_merge_into_one() {
        let marks = open_end_marks(
            [
                Vector2::new(10.0, 10.0),
                Vector2::new(11.0, 10.5),
                Vector2::new(30.0, 10.0),
                Vector2::new(10.0, 30.0),
                Vector2::new(-1.0, 10.0),
                Vector2::new(10.0, 400.0),
            ]
            .into_iter(),
            Vector2::new(200.0, 100.0),
        );

        assert_eq!(
            marks,
            OpenEndMarks {
                rings: vec![
                    Vector2::new(10.0, 10.0),
                    Vector2::new(30.0, 10.0),
                    Vector2::new(10.0, 30.0),
                ],
                clusters: Vec::new(),
            }
        );
    }

    #[test]
    fn open_end_rings_crowding_a_cell_become_one_ring_counting_its_ends() {
        let crowded = (0..6).flat_map(|step| {
            let at = Vector2::new(100.0 + 3.5 * f64::from(step), 52.0);
            [at, at + Vector2::new(0.5, 0.5)]
        });
        let apart = [Vector2::new(10.0, 10.0), Vector2::new(30.0, 10.0)];

        let marks = open_end_marks(apart.into_iter().chain(crowded), Vector2::new(200.0, 100.0));

        assert_eq!(marks.rings, apart.to_vec());
        assert_eq!(marks.clusters.len(), 1);
        assert_eq!(marks.clusters[0].ends, 12);
        assert!(
            marks.clusters[0]
                .center
                .distance(Vector2::new(109.0, 52.25))
                < 1e-9
        );
        assert!(
            Hover::OpenEnds(12)
                .describe(&Sketch::new(caditor_geometry::Plane::XY), false)
                .starts_with("12 open ends here")
        );
    }
}
