use std::{collections::BTreeMap, sync::Arc};

use caditor_document::{FeatureId, FeatureState, Transaction};
use caditor_expression::{Dimension, Expression, Quantity};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch, SketchSolution};
use egui::{
    Align2, Color32, Galley, Id, Key, Order, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
    TextEdit, Ui,
    text::{CCursor, CCursorRange},
    vec2,
};

use crate::{
    annotation_layout::{
        self, DimensionLayout, Footprint, GlyphAnchor, GlyphKind, LabelFrame, Obstacles,
    },
    appearance, canvas,
    field::{self, DimensionTarget},
    model::{Action, Model},
    selection::{Pickable, Selection},
    sketch_status, sketch_tools,
    snap::Screen,
    units::Units,
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
const EDIT_HINT: &str = "Double-click to change it, or drag its label to move it.";
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

struct DimensionMark {
    constraint: ConstraintId,
    layout: DimensionLayout,
    frame: Option<LabelFrame>,
    text: String,
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

struct GlyphMark {
    constraint: ConstraintId,
    anchor: EntityId,
    kind: GlyphKind,
    center: Vector2,
    standing: Standing,
    hover: Hover,
}

struct GlyphItem {
    constraint: ConstraintId,
    kind: GlyphKind,
    standing: Standing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Hover {
    Dimension(ConstraintId),
    Glyph(ConstraintId),
    Beyond(Vec<ConstraintId>),
}

impl Hover {
    fn describe(&self, sketch: &Sketch) -> String {
        match self {
            Self::Dimension(constraint) => {
                let described = sketch
                    .constraint(*constraint)
                    .map(|constraint| sketch.describe(constraint))
                    .unwrap_or_default();
                format!("{described}. {EDIT_HINT}")
            }
            Self::Glyph(constraint) => sketch.describe_constraint(*constraint),
            Self::Beyond(hidden) => beyond_description(sketch, hidden),
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

#[derive(Debug, Clone, Default)]
struct LabelTexts {
    key: Option<TextsKey>,
    texts: BTreeMap<ConstraintId, String>,
}

impl LabelTexts {
    fn refresh(&mut self, model: &Model, feature: FeatureId) {
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
    }

    fn text(
        &mut self,
        model: &Model,
        shown: &Sketch,
        id: ConstraintId,
        constraint: &Constraint,
        expression: &Expression,
    ) -> String {
        self.texts
            .entry(id)
            .or_insert_with(|| label_text(model, shown, id, constraint, expression))
            .clone()
    }
}

struct GlyphGroup {
    anchor: EntityId,
    place: GlyphAnchor,
    items: Vec<GlyphItem>,
}

struct Marks {
    dimensions: Vec<DimensionMark>,
    groups: Vec<GlyphGroup>,
    screen_centre: Option<Vector2>,
    open_ends: Vec<Vector2>,
    beyond: Vec<[Vector2; 2]>,
}

impl Marks {
    fn collect(
        model: &Model,
        feature: FeatureId,
        screen: &impl Screen,
        view: Vector2,
        texts: &mut LabelTexts,
        dragged: Option<(ConstraintId, Vector2)>,
    ) -> Option<Self> {
        texts.refresh(model, feature);
        let owner = model.document().feature(feature)?;
        let definition = owner.kind.sketch()?;
        let shown = model.displayed_sketch(owner)?;
        let standings = Standings::load(model, feature, definition);
        let centre = centre_of(&shown);
        let screen_centre = centre.and_then(|centre| screen.to_screen(centre));
        let mut dimensions = Vec::new();
        let mut groups: BTreeMap<EntityId, Vec<(ConstraintId, GlyphKind)>> = BTreeMap::new();
        let mut measured_dimensions = Vec::new();
        for (id, constraint) in definition.constraints() {
            let Some(expression) = constraint.dimension() else {
                for (entity, kind) in annotation_layout::glyphs_of(&shown, constraint) {
                    groups.entry(entity).or_default().push((id, kind));
                }
                continue;
            };
            measured_dimensions.push((
                id,
                constraint,
                expression,
                annotation_layout::measured(&shown, constraint),
            ));
        }
        let offset_of = |id: ConstraintId| match dragged {
            Some((dragged, offset)) if dragged == id => Some(offset),
            _ => definition.label_offset(id),
        };
        let unplaced: Vec<_> = measured_dimensions
            .iter()
            .map(|(id, _, _, measured)| measured.filter(|_| offset_of(*id).is_none()))
            .collect();
        let lanes = annotation_layout::lanes(&unplaced, centre, extent_of(&shown));
        for ((id, constraint, expression, measured), lane) in
            measured_dimensions.into_iter().zip(lanes)
        {
            let frame = measured.as_ref().and_then(annotation_layout::label_frame);
            let placed = frame
                .zip(offset_of(id))
                .map(|(frame, offset)| frame.place(offset));
            let layout = measured.and_then(|measured| {
                annotation_layout::layout(&measured, screen, centre, lane, placed)
            });
            if let Some(layout) = layout {
                dimensions.push(DimensionMark {
                    constraint: id,
                    layout,
                    frame,
                    text: texts.text(model, &shown, id, constraint, expression),
                    standing: standings.of(id),
                });
            }
        }
        let groups = groups
            .into_iter()
            .filter_map(|(anchor, items)| {
                Some(GlyphGroup {
                    anchor,
                    place: annotation_layout::within_view(
                        annotation_layout::glyph_anchor(&shown, anchor, screen)?,
                        view,
                    )?,
                    items: items
                        .into_iter()
                        .map(|(constraint, kind)| GlyphItem {
                            constraint,
                            kind,
                            standing: standings.of(constraint),
                        })
                        .collect(),
                })
            })
            .collect();
        let result = sketch_status::up_to_date_result(model.evaluation(), feature);
        let in_view = |at: &Vector2| at.x >= 0.0 && at.y >= 0.0 && at.x <= view.x && at.y <= view.y;
        let open_ends = result
            .into_iter()
            .flat_map(|result| &result.open_ends)
            .filter_map(|end| screen.to_screen(shown.point(*end)?))
            .filter(in_view)
            .collect();
        let beyond = result
            .into_iter()
            .flat_map(|result| &result.beyond)
            .filter_map(|beyond| {
                let at = screen.to_screen(shown.point(beyond.point)?)?;
                let end = screen.to_screen(shown.nearest_end(*beyond)?)?;
                (in_view(&at) || in_view(&end)).then_some([end, at])
            })
            .collect();
        Some(Self {
            dimensions,
            groups,
            screen_centre,
            open_ends,
            beyond,
        })
    }

    fn glyphs(&self, mut blocked: Obstacles) -> Vec<GlyphMark> {
        let half = Vector2::splat(f64::from(GLYPH_SIZE / 2.0 + GLYPH_CLEARANCE));
        let mut glyphs = Vec::new();
        for group in &self.groups {
            let (shown, hidden) = if group.items.len() > MAX_STACKED {
                group.items.split_at(MAX_STACKED - 1)
            } else {
                (group.items.as_slice(), &[][..])
            };
            let slots = shown.len() + usize::from(!hidden.is_empty());
            let mut positions = annotation_layout::place_glyphs(
                group.place,
                slots,
                self.screen_centre,
                half,
                &blocked,
            );
            for center in &positions {
                blocked.add(Footprint {
                    center: *center,
                    half,
                });
            }
            let beyond_at = (!hidden.is_empty()).then(|| positions.pop()).flatten();
            glyphs.extend(shown.iter().zip(positions).map(|(item, center)| GlyphMark {
                constraint: item.constraint,
                anchor: group.anchor,
                kind: item.kind,
                center,
                standing: item.standing,
                hover: Hover::Glyph(item.constraint),
            }));
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

    fn dimension(&self, constraint: ConstraintId) -> Option<&DimensionMark> {
        self.dimensions
            .iter()
            .find(|mark| mark.constraint == constraint)
    }
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
            | Entity::Spline { .. } => None,
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
            | Entity::Spline { .. } => None,
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
        Constraint::Radius { .. } => format!("{RADIUS_PREFIX}{text}"),
        Constraint::Diameter { .. } | Constraint::AxisDiameter { .. } => {
            format!("{DIAMETER_PREFIX}{text}")
        }
        _ => text,
    }
}

pub struct Surface<'a, S> {
    pub rect: Rect,
    pub screen: &'a S,
    pub feature: FeatureId,
    pub interactive: bool,
    pub glyphs: bool,
    pub highlight: Option<Pickable>,
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
    places: BTreeMap<(FeatureId, ConstraintId), LabelPlace>,
}

struct Placed {
    pickable: Pickable,
    hit: Rect,
    key: (ConstraintId, Option<EntityId>),
    hover: Hover,
    label: Option<(Vector2, LabelFrame)>,
}

impl Annotations {
    pub fn hovered(&self) -> Option<Pickable> {
        self.hovered
    }

    pub fn label_place(&self, feature: FeatureId, constraint: ConstraintId) -> Option<LabelPlace> {
        self.places.get(&(feature, constraint)).copied()
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

    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        surface: &Surface<'_, impl Screen>,
        selection: &mut Selection,
        actions: &mut Vec<Action>,
    ) {
        self.hovered = None;
        self.places.clear();
        self.dragging = self
            .dragging
            .filter(|drag| drag.feature == surface.feature && surface.interactive);
        self.field = self.field.filter(|open| open.feature == surface.feature);
        self.request = self
            .request
            .filter(|request| request.feature == surface.feature);
        let view = Vector2::new(
            f64::from(surface.rect.width()),
            f64::from(surface.rect.height()),
        );
        let dragged = self
            .dragging
            .and_then(|drag| Some((drag.constraint, drag.offset?)));
        let Some(marks) = Marks::collect(
            model,
            surface.feature,
            surface.screen,
            view,
            &mut self.texts,
            dragged,
        ) else {
            self.field = None;
            return;
        };
        self.open_requested(model, &marks);
        for mark in &marks.dimensions {
            if let Some(frame) = mark.frame {
                self.places.insert(
                    (surface.feature, mark.constraint),
                    LabelPlace {
                        at: mark.layout.at,
                        frame,
                    },
                );
            }
        }

        let painter = ui.painter_at(surface.rect);
        let editing = self.field.map(|open| open.constraint);
        let labels: Vec<Option<(Arc<Galley>, Rect)>> = marks
            .dimensions
            .iter()
            .map(|mark| {
                (editing != Some(mark.constraint)).then(|| {
                    let galley = painter.layout_no_wrap(
                        mark.text.clone(),
                        canvas::body(),
                        Color32::PLACEHOLDER,
                    );
                    let rect = label_rect(surface.rect, &mark.layout, galley.size());
                    (galley, rect)
                })
            })
            .collect();
        let mut blocked = Obstacles::default();
        for (_, rect) in labels.iter().flatten() {
            blocked.add(footprint(surface.rect, rect.expand(GLYPH_CLEARANCE)));
        }
        let glyphs = if surface.glyphs {
            marks.glyphs(blocked)
        } else {
            Vec::new()
        };
        let pickable = |constraint| Pickable::SketchConstraint {
            feature: surface.feature,
            constraint,
        };
        let placed = marks
            .dimensions
            .iter()
            .zip(&labels)
            .filter_map(|(mark, label)| {
                label.as_ref().map(|(_, rect)| Placed {
                    pickable: pickable(mark.constraint),
                    hit: *rect,
                    key: (mark.constraint, None),
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
            .chain(glyphs.iter().map(|mark| Placed {
                pickable: pickable(mark.constraint),
                hit: Rect::from_center_size(
                    to_pos(surface.rect, mark.center),
                    egui::Vec2::splat(GLYPH_HIT_SIZE),
                ),
                key: (mark.constraint, Some(mark.anchor)),
                hover: mark.hover.clone(),
                label: None,
            }));
        if surface.interactive
            && let Some(definition) = model
                .document()
                .feature(surface.feature)
                .and_then(|owner| owner.kind.sketch())
        {
            for target in placed {
                self.interact(ui, surface, target, selection, definition);
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

        let color = |constraint, standing| {
            let pickable = pickable(constraint);
            if self.hovered == Some(pickable) || surface.highlight == Some(pickable) {
                canvas::HOVERED
            } else if selection.contains(pickable) {
                canvas::SELECTED
            } else {
                match standing {
                    Standing::Normal => canvas::DIMENSION,
                    Standing::Conflicting => canvas::ERROR,
                    Standing::Redundant => canvas::WARNING,
                    Standing::Inactive => canvas::MUTED,
                }
            }
        };
        let framed = |standing: Standing| {
            appearance::is_high_contrast(ui.visuals())
                .then(|| standing.frame())
                .flatten()
        };
        for (mark, label) in marks.dimensions.iter().zip(labels) {
            let tint = color(mark.constraint, mark.standing);
            let frame = framed(mark.standing).zip(label.as_ref().map(|(_, rect)| *rect));
            paint_dimension(&painter, surface.rect, &mark.layout, label, tint);
            if let Some((frame, rect)) = frame {
                paint_frame(&painter, rect, frame, tint);
            }
        }
        for mark in &glyphs {
            let center = to_pos(surface.rect, mark.center);
            let tint = color(mark.constraint, mark.standing);
            match &mark.hover {
                Hover::Beyond(hidden) => paint_beyond(&painter, center, hidden.len(), tint),
                Hover::Dimension(_) | Hover::Glyph(_) => {
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
                Stroke::new(STROKE_WIDTH, canvas::MUTED),
                BEYOND_DASH,
                BEYOND_DASH,
            ));
            painter.circle_stroke(at, BEYOND_RADIUS, Stroke::new(RING_WIDTH, canvas::MUTED));
        }
        for end in &marks.open_ends {
            painter.circle_stroke(
                to_pos(surface.rect, *end),
                OPEN_END_RADIUS,
                Stroke::new(RING_WIDTH, canvas::WARNING),
            );
        }
        self.show_field(ui, model, surface, &marks, actions);
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
        surface: &Surface<'_, impl Screen>,
        target: Placed,
        selection: &mut Selection,
        definition: &Sketch,
    ) {
        let hit = target.hit.intersect(surface.rect);
        if !hit.is_positive() {
            return;
        }
        let sense = if target.label.is_some() {
            Sense::click_and_drag()
        } else {
            Sense::CLICK
        };
        let response = ui.interact(
            hit,
            Id::new(("sketch-annotation", surface.feature, target.key)),
            sense,
        );
        if response.hovered() {
            self.hovered = Some(target.pickable);
        }
        if let (Some((label, frame)), Pickable::SketchConstraint { constraint, .. }) =
            (target.label, target.pickable)
        {
            self.drag_label(surface, &response, constraint, label, frame);
        }
        if response.clicked() {
            let toggle = ui.input(|input| input.modifiers.shift || input.modifiers.command);
            if toggle {
                selection.toggle(target.pickable);
            } else {
                selection.replace_with(target.pickable);
            }
        }
        if matches!(target.hover, Hover::Dimension(_))
            && response.double_clicked()
            && let Pickable::SketchConstraint {
                feature,
                constraint,
            } = target.pickable
        {
            self.open(feature, constraint);
        }
        response.on_hover_ui(|ui| {
            ui.label(target.hover.describe(definition));
        });
    }

    fn drag_label(
        &mut self,
        surface: &Surface<'_, impl Screen>,
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
        surface: &Surface<'_, impl Screen>,
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
        let stored = field::value_text(document, &target.owner(), expression);
        let field = egui::Area::new(id.with("area"))
            .order(Order::Foreground)
            .fixed_pos(to_pos(surface.rect, mark.layout.label) - vec2(0.0, FIELD_LIFT))
            .pivot(Align2::CENTER_TOP)
            .constrain_to(surface.rect)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(FIELD_MARGIN)
                    .show(ui, |ui| {
                        let field = field::commit_field(
                            ui,
                            id,
                            &stored,
                            FIELD_WIDTH,
                            open.focus_pending,
                            |text| {
                                field::dimension_transaction(
                                    document,
                                    model.parameters(),
                                    target,
                                    text,
                                    model.units(),
                                )
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
                select_all(ui.ctx(), id, &stored);
                open.select_all_pending = false;
            }
        }
        let lost_focus = field.response.lost_focus();
        let entered = ui.input(|input| input.key_pressed(Key::Enter));
        if lost_focus && field.error.is_some() && entered {
            open.focus_pending = true;
        }
        self.field = (!lost_focus || field.error.is_some()).then_some(open);
    }
}

fn select_all(context: &egui::Context, id: Id, text: &str) {
    let mut state = TextEdit::load_state(context, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(0),
        CCursor::new(text.chars().count()),
    )));
    TextEdit::store_state(context, id, state);
}

fn to_pos(rect: Rect, point: Vector2) -> Pos2 {
    rect.min + vec2(point.x as f32, point.y as f32)
}

fn to_vec(vector: Vector2) -> egui::Vec2 {
    vec2(vector.x as f32, vector.y as f32)
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
        context.set_fonts(crate::fonts::definitions());
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
}
