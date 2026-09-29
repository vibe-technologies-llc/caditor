use std::{collections::BTreeMap, sync::Arc};

use caditor_document::{FeatureId, FeatureState};
use caditor_expression::Expression;
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch, SketchSolution};
use egui::{
    Align2, Color32, FontId, Galley, Id, Key, Order, Pos2, Rect, Sense, Shape, Stroke, TextEdit,
    Ui,
    text::{CCursor, CCursorRange},
    vec2,
};

use crate::{
    annotation_layout::{self, DimensionLayout, GlyphKind},
    canvas,
    field::{self, DimensionTarget},
    model::{Action, Model},
    selection::{Pickable, Selection},
    sketch_status,
    snap::Screen,
};

const LABEL_FONT_SIZE: f32 = 13.0;
const GLYPH_FONT_SIZE: f32 = 11.0;
const LABEL_PADDING: egui::Vec2 = vec2(4.0, 2.0);
const LABEL_GAP: f32 = 3.0;
const CORNER_RADIUS: f32 = 3.0;
const STROKE_WIDTH: f32 = 1.2;
const ARROW_LENGTH: f32 = 9.0;
const ARROW_HALF_WIDTH: f32 = 3.2;
const GLYPH_SIZE: f32 = 15.0;
const GLYPH_HIT_SIZE: f32 = 16.0;
const DOT_RADIUS: f32 = 3.5;
const RING_WIDTH: f32 = 1.5;
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
const EDIT_HINT: &str = "Double-click to change it.";

pub fn field_id(feature: FeatureId, constraint: ConstraintId) -> Id {
    Id::new(("canvas-dimension", feature, constraint))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    Normal,
    Conflicting,
    Redundant,
}

struct DimensionMark {
    constraint: ConstraintId,
    layout: DimensionLayout,
    text: String,
    standing: Standing,
    description: String,
}

struct GlyphMark {
    constraint: ConstraintId,
    anchor: EntityId,
    kind: GlyphKind,
    center: Vector2,
    standing: Standing,
    description: String,
}

struct Marks {
    dimensions: Vec<DimensionMark>,
    glyphs: Vec<GlyphMark>,
}

impl Marks {
    fn collect(model: &Model, feature: FeatureId, screen: &impl Screen) -> Option<Self> {
        let owner = model.document().feature(feature)?;
        let definition = owner.kind.sketch()?;
        let shown = model.displayed_sketch(owner)?;
        let standings = Standings::load(model, feature);
        let centre = centre_of(&shown);
        let screen_centre = centre.and_then(|centre| screen.to_screen(centre));
        let mut dimensions = Vec::new();
        let mut groups: BTreeMap<EntityId, Vec<(ConstraintId, GlyphKind)>> = BTreeMap::new();
        for (id, constraint) in definition.constraints() {
            let Some(expression) = constraint.dimension() else {
                for (entity, kind) in annotation_layout::glyphs_of(&shown, constraint) {
                    groups.entry(entity).or_default().push((id, kind));
                }
                continue;
            };
            let layout = annotation_layout::measured(&shown, constraint)
                .and_then(|measured| annotation_layout::layout(&measured, screen, centre));
            if let Some(layout) = layout {
                dimensions.push(DimensionMark {
                    constraint: id,
                    layout,
                    text: label_text(model, constraint, expression),
                    standing: standings.of(id),
                    description: definition.describe(constraint),
                });
            }
        }
        let glyphs = groups
            .into_iter()
            .flat_map(|(anchor, items)| {
                let positions = annotation_layout::glyph_anchor(&shown, anchor, screen)
                    .map(|place| annotation_layout::stack_glyphs(place, items.len(), screen_centre))
                    .unwrap_or_default();
                items
                    .into_iter()
                    .zip(positions)
                    .map(|((constraint, kind), center)| GlyphMark {
                        constraint,
                        anchor,
                        kind,
                        center,
                        standing: standings.of(constraint),
                        description: definition.describe_constraint(constraint),
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        Some(Self { dimensions, glyphs })
    }

    fn dimension(&self, constraint: ConstraintId) -> Option<&DimensionMark> {
        self.dimensions
            .iter()
            .find(|mark| mark.constraint == constraint)
    }
}

struct Standings<'a> {
    conflicting: Vec<ConstraintId>,
    solution: Option<&'a SketchSolution>,
}

impl<'a> Standings<'a> {
    fn load(model: &'a Model, feature: FeatureId) -> Self {
        let conflicting = match model
            .evaluation()
            .feature(feature)
            .map(|status| &status.state)
        {
            Some(FeatureState::Failed(error)) => error.constraints.clone(),
            Some(FeatureState::UpToDate | FeatureState::Outdated) | None => Vec::new(),
        };
        Self {
            conflicting,
            solution: sketch_status::up_to_date_solution(model.evaluation(), feature),
        }
    }

    fn of(&self, constraint: ConstraintId) -> Standing {
        if self.conflicting.contains(&constraint) {
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

fn label_text(model: &Model, constraint: &Constraint, expression: &Expression) -> String {
    let text = model.document().expression_text(expression);
    let text = match field::value_preview(model.parameters(), expression, model.length_unit()) {
        Some(value) => format!("{text} {value}"),
        None => text,
    };
    match constraint {
        Constraint::Radius { .. } => format!("{RADIUS_PREFIX}{text}"),
        Constraint::Diameter { .. } => format!("{DIAMETER_PREFIX}{text}"),
        _ => text,
    }
}

pub struct Surface<'a, S> {
    pub rect: Rect,
    pub screen: &'a S,
    pub feature: FeatureId,
    pub interactive: bool,
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
}

struct Placed {
    pickable: Pickable,
    hit: Rect,
    key: (ConstraintId, Option<EntityId>),
    description: String,
    editable: bool,
}

impl Annotations {
    pub fn hovered(&self) -> Option<Pickable> {
        self.hovered
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
        self.field = self.field.filter(|open| open.feature == surface.feature);
        self.request = self
            .request
            .filter(|request| request.feature == surface.feature);
        let Some(marks) = Marks::collect(model, surface.feature, surface.screen) else {
            self.field = None;
            return;
        };
        self.open_requested(model, &marks);

        let painter = ui.painter_at(surface.rect);
        let editing = self.field.map(|open| open.constraint);
        let labels: Vec<Option<(Arc<Galley>, Rect)>> = marks
            .dimensions
            .iter()
            .map(|mark| {
                (editing != Some(mark.constraint)).then(|| {
                    let galley = painter.layout_no_wrap(
                        mark.text.clone(),
                        FontId::proportional(LABEL_FONT_SIZE),
                        Color32::PLACEHOLDER,
                    );
                    let rect = label_rect(surface.rect, &mark.layout, galley.size());
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
            .zip(&labels)
            .filter_map(|(mark, label)| {
                label.as_ref().map(|(_, rect)| Placed {
                    pickable: pickable(mark.constraint),
                    hit: *rect,
                    key: (mark.constraint, None),
                    description: format!("{}. {EDIT_HINT}", mark.description),
                    editable: true,
                })
            })
            .chain(marks.glyphs.iter().map(|mark| Placed {
                pickable: pickable(mark.constraint),
                hit: Rect::from_center_size(
                    to_pos(surface.rect, mark.center),
                    egui::Vec2::splat(GLYPH_HIT_SIZE),
                ),
                key: (mark.constraint, Some(mark.anchor)),
                description: mark.description.clone(),
                editable: false,
            }));
        if surface.interactive {
            for target in placed {
                self.interact(ui, surface, target, selection);
            }
        }

        let color = |constraint, standing| {
            let pickable = pickable(constraint);
            if self.hovered == Some(pickable) {
                canvas::HOVERED
            } else if selection.contains(pickable) {
                canvas::SELECTED
            } else {
                match standing {
                    Standing::Normal => canvas::DIMENSION,
                    Standing::Conflicting => canvas::ERROR,
                    Standing::Redundant => canvas::WARNING,
                }
            }
        };
        for (mark, label) in marks.dimensions.iter().zip(labels) {
            paint_dimension(
                &painter,
                surface.rect,
                &mark.layout,
                label,
                color(mark.constraint, mark.standing),
            );
        }
        for mark in &marks.glyphs {
            paint_glyph(
                &painter,
                to_pos(surface.rect, mark.center),
                mark.kind,
                color(mark.constraint, mark.standing),
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
    ) {
        let hit = target.hit.intersect(surface.rect);
        if !hit.is_positive() {
            return;
        }
        let response = ui.interact(
            hit,
            Id::new(("sketch-annotation", surface.feature, target.key)),
            Sense::CLICK,
        );
        if response.hovered() {
            self.hovered = Some(target.pickable);
        }
        if response.clicked() {
            let toggle = ui.input(|input| input.modifiers.shift || input.modifiers.command);
            if toggle {
                selection.toggle(target.pickable);
            } else {
                selection.replace_with(target.pickable);
            }
        }
        if target.editable
            && response.double_clicked()
            && let Pickable::SketchConstraint {
                feature,
                constraint,
            } = target.pickable
        {
            self.open(feature, constraint);
        }
        response.on_hover_text(target.description);
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
        let stored = document.expression_text(expression);
        let target = DimensionTarget {
            feature: open.feature,
            constraint: open.constraint,
        };
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
                                    model.length_unit(),
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

fn label_rect(rect: Rect, layout: &DimensionLayout, text: egui::Vec2) -> Rect {
    let size = text + LABEL_PADDING * 2.0;
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
        painter.rect_filled(label, CORNER_RADIUS, canvas::BACKDROP);
        painter.galley(label.min + LABEL_PADDING, galley, color);
    }
}

pub fn glyph_letter(kind: GlyphKind) -> Option<&'static str> {
    match kind {
        GlyphKind::Horizontal => Some("H"),
        GlyphKind::Vertical => Some("V"),
        GlyphKind::Equal => Some("="),
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
        | GlyphKind::Parallel
        | GlyphKind::Perpendicular
        | GlyphKind::Tangent
        | GlyphKind::Midpoint
        | GlyphKind::Concentric
        | GlyphKind::Collinear
        | GlyphKind::Symmetric
        | GlyphKind::Fix => {}
    }
    painter.rect_filled(
        Rect::from_center_size(center, egui::Vec2::splat(GLYPH_SIZE)),
        CORNER_RADIUS,
        canvas::BACKDROP,
    );
    if let Some(letter) = glyph_letter(kind) {
        let galley = painter.layout_no_wrap(
            letter.to_owned(),
            FontId::proportional(GLYPH_FONT_SIZE),
            Color32::PLACEHOLDER,
        );
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
        | GlyphKind::Coincident
        | GlyphKind::OnCurve => {}
    }
}

#[cfg(test)]
mod tests {
    use egui::{FontId, RawInput};

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
            let renders = context.fonts_mut(|fonts| {
                fonts.has_glyphs(&FontId::proportional(LABEL_FONT_SIZE), text.trim())
            });
            assert!(renders, "'{text}' cannot be drawn with the app fonts");
        }
    }
}
