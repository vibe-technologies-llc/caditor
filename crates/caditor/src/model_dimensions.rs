use std::collections::BTreeMap;

use caditor_document::{FeatureId, ParameterOwner, Transaction};
use caditor_expression::{Dimension, Quantity};
use caditor_geometry::Point3;
use caditor_render::View;
use egui::{
    Align2, Color32, Id, Key, Order, Pos2, Rect, Sense, Stroke, Ui, WidgetInfo, WidgetType, vec2,
};

use crate::{
    annotation_layout,
    annotations::{self, Annotations, Surface},
    canvas,
    feature_values::{self, FeatureValue, ValueSlot},
    field::{self, Expected, NamedField},
    model::{Action, Model},
    scene,
    selection::{Pickable, Selection},
    units::Units,
    value_shapes::Drawn,
    viewport::SketchScreen,
};

pub const MAX_SUBJECTS: usize = 4;
const STACK_OFFSET: f32 = 14.0;
const STACK_GAP: f32 = 3.0;
const LINE_WIDTH: f32 = 1.2;
const TICK_HALF: f32 = 4.0;
const LEADER_REACH: f32 = 28.0;
const LEADER_DOT: f32 = 2.5;
const FIELD_WIDTH: f32 = 120.0;
const FIELD_LIFT: f32 = 14.0;
const FIELD_MARGIN: f32 = 4.0;
pub const VALUE_HINT: &str =
    "Double-click to change it, or highlight it from the keyboard and press Enter.";

pub struct Outside<'a> {
    pub rect: Rect,
    pub view: &'a View,
    pub pixels_per_point: f64,
    pub subjects: &'a [FeatureId],
    pub highlight: Option<Pickable>,
    pub interactive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SketchesKey {
    revision: u64,
    subjects: Vec<FeatureId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValuesKey {
    revision: u64,
    evaluation: u64,
    units: Units,
    subjects: Vec<FeatureId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenValue {
    feature: FeatureId,
    slot: ValueSlot,
    focus_pending: bool,
    select_all_pending: bool,
}

#[derive(Debug, Clone)]
struct ShownValue {
    value: FeatureValue,
    text: String,
    name: String,
}

struct PlacedValue {
    index: usize,
    rect: Rect,
    path: Vec<Pos2>,
    leader: Option<Pos2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stack {
    Centre(FeatureId),
    Leader(FeatureId),
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShownShape {
    Stacked,
    Path(usize),
    Leader,
}

fn screen_halfway(points: &[Pos2]) -> Option<Pos2> {
    let legs = || {
        points
            .windows(2)
            .filter_map(|pair| Some((*pair.first()?, *pair.get(1)?)))
    };
    let total: f32 = legs().map(|(from, to)| from.distance(to)).sum();
    let mut left = total / 2.0;
    for (from, to) in legs() {
        let length = from.distance(to);
        if length >= left && length > 0.0 {
            return Some(from.lerp(to, left / length));
        }
        left -= length;
    }
    points.first().copied()
}

fn end_ticks(points: &[Pos2]) -> Vec<(Pos2, egui::Vec2)> {
    let first = points
        .first()
        .zip(points.get(1))
        .map(|(end, next)| (*end, *next - *end));
    let last = points
        .len()
        .checked_sub(2)
        .and_then(|before| Some((*points.last()?, *points.last()? - *points.get(before)?)));
    first
        .into_iter()
        .chain(last)
        .map(|(end, along)| (end, along.normalized().rot90() * TICK_HALF))
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct ModelDimensions {
    sketches_key: Option<SketchesKey>,
    sketches: Vec<FeatureId>,
    annotations: BTreeMap<FeatureId, Annotations>,
    values_key: Option<ValuesKey>,
    values: Vec<ShownValue>,
    shown: Vec<Pickable>,
    hovered: Option<Pickable>,
    field: Option<OpenValue>,
}

pub fn value_field_id(feature: FeatureId, slot: ValueSlot) -> Id {
    Id::new(("model-value-field", feature, slot))
}

pub fn value_pickable(value: &FeatureValue) -> Pickable {
    Pickable::FeatureValue {
        feature: value.feature,
        value: value.slot,
    }
}

fn owner_of(value: &FeatureValue) -> ParameterOwner {
    ParameterOwner::Feature {
        feature: value.feature,
        value: value.caption.to_owned(),
    }
}

fn label_text(model: &Model, value: &FeatureValue) -> String {
    let document = model.document();
    let units = model.units();
    let parameters = model.parameters();
    let shown = value
        .expression
        .evaluate_as(value.dimension, &|id| parameters.value(id))
        .ok()
        .map(|amount| {
            units.show(match value.dimension {
                Dimension::LENGTH => Quantity::length(amount),
                Dimension::ANGLE => Quantity::angle(amount),
                _ => Quantity::plain(amount),
            })
        });
    let driving = field::driving_parameter(document, &owner_of(value), &value.expression);
    let caption = value.caption;
    match (driving, shown) {
        (Some(parameter), Some(shown)) => format!("{} = {shown}", parameter.name),
        (None, Some(shown)) if value.expression.is_literal() => format!("{caption} {shown}"),
        (None, Some(shown)) => format!(
            "{caption} {} = {shown}",
            document.expression_text(&value.expression)
        ),
        (_, None) => format!("{caption} {}", document.expression_text(&value.expression)),
    }
}

fn value_change(model: &Model, value: &FeatureValue, text: &str) -> Result<Transaction, String> {
    let document = model.document();
    let parameters = model.parameters();
    let units = model.units();
    let owner = owner_of(value);
    let parse = |text: &str| {
        let parsed = field::parse_expression(
            document,
            parameters,
            text,
            Expected {
                dimension: Some(value.dimension),
                non_negative: false,
            },
            units,
        )?;
        let evaluated = parameters
            .evaluate_expression(&parsed)
            .map_err(|error| field::sentence(&error.to_string()))?
            .value;
        value.rule.check(evaluated)?;
        Ok(parsed)
    };
    let named = NamedField {
        document,
        parameters,
        units,
        dimension: Some(value.dimension),
        owner,
        current: &value.expression,
    };
    named.through_reference(text, parse).unwrap_or_else(|| {
        named.transaction(text, parse, |expression| {
            feature_values::change(model, value.feature, value.slot, expression)
        })
    })
}

fn sketches_of(model: &Model, subjects: &[FeatureId]) -> Vec<FeatureId> {
    let document = model.document();
    let active: Vec<FeatureId> = document
        .active_features()
        .map(caditor_document::Feature::id)
        .collect();
    let subject_sketches = subjects.iter().filter_map(|subject| {
        let owner = document.feature(*subject)?;
        owner
            .kind
            .sketch()
            .map(|_| *subject)
            .or_else(|| feature_values::sketch_of(&owner.kind))
    });
    let mut sketches: Vec<FeatureId> = document
        .active_features()
        .filter(|feature| feature.kind.sketch().is_some() && !feature.hidden)
        .map(caditor_document::Feature::id)
        .chain(subject_sketches.filter(|sketch| active.contains(sketch)))
        .collect();
    sketches.sort_unstable();
    sketches.dedup();
    sketches
}

impl ModelDimensions {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn hovered(&self) -> Option<Pickable> {
        self.hovered
    }

    pub fn highlightable(&self) -> &[Pickable] {
        &self.shown
    }

    pub fn holds(&self, pickable: Pickable) -> bool {
        self.shown.contains(&pickable)
    }

    #[cfg(test)]
    pub fn shape_of(&self, pickable: Pickable) -> Option<ShownShape> {
        self.values
            .iter()
            .find(|shown| value_pickable(&shown.value) == pickable)
            .map(|shown| match &shown.value.drawn {
                Drawn::Stacked => ShownShape::Stacked,
                Drawn::Path(points) => ShownShape::Path(points.len()),
                Drawn::Leader(_) => ShownShape::Leader,
            })
    }

    #[cfg(test)]
    pub fn sketch_label_rects(&self) -> Vec<(FeatureId, Rect)> {
        self.annotations
            .iter()
            .flat_map(|(feature, annotations)| {
                annotations.label_rects().map(|rect| (*feature, rect))
            })
            .collect()
    }

    pub fn held(&self) -> Vec<FeatureId> {
        let value_feature = |pickable: Option<Pickable>| match pickable {
            Some(Pickable::FeatureValue { feature, .. }) => Some(feature),
            _ => None,
        };
        let open = self.field.map(|open| open.feature);
        open.into_iter()
            .chain(value_feature(self.hovered).filter(|hovered| Some(*hovered) != open))
            .collect()
    }

    pub fn open(&mut self, pickable: Pickable) -> bool {
        if !self.holds(pickable) {
            return false;
        }
        match pickable {
            Pickable::SketchConstraint {
                feature,
                constraint,
            } => match self.annotations.get_mut(&feature) {
                Some(annotations) => {
                    annotations.request_field(feature, constraint);
                    true
                }
                None => false,
            },
            Pickable::FeatureValue { feature, value } => {
                self.field = Some(OpenValue {
                    feature,
                    slot: value,
                    focus_pending: true,
                    select_all_pending: true,
                });
                true
            }
            _ => false,
        }
    }

    fn refresh(&mut self, model: &Model, subjects: &[FeatureId]) {
        let sketches_key = SketchesKey {
            revision: model.revision(),
            subjects: subjects.to_vec(),
        };
        if self.sketches_key.as_ref() != Some(&sketches_key) {
            self.sketches = sketches_of(model, subjects);
            self.sketches_key = Some(sketches_key);
            let kept = &self.sketches;
            self.annotations
                .retain(|feature, _| kept.binary_search(feature).is_ok());
        }
        let values_key = ValuesKey {
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            units: model.units(),
            subjects: subjects.to_vec(),
        };
        if self.values_key.as_ref() != Some(&values_key) {
            let document = model.document();
            self.values = subjects
                .iter()
                .flat_map(|subject| feature_values::of(model, *subject))
                .map(|value| {
                    let name = document
                        .feature(value.feature)
                        .map(|owner| owner.name.clone())
                        .unwrap_or_default();
                    ShownValue {
                        text: label_text(model, &value),
                        name,
                        value,
                    }
                })
                .collect();
            self.values_key = Some(values_key);
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        outside: &Outside<'_>,
        selection: &mut Selection,
        actions: &mut Vec<Action>,
    ) {
        let mut subjects = self.held();
        for subject in outside.subjects {
            if !subjects.contains(subject) {
                subjects.push(*subject);
            }
        }
        subjects.truncate(MAX_SUBJECTS);
        self.refresh(model, &subjects);
        self.hovered = None;
        self.field = self.field.filter(|open| {
            self.values
                .iter()
                .any(|shown| shown.value.feature == open.feature && shown.value.slot == open.slot)
        });
        let placed = self.place_values(ui, outside);
        let mut taken: Vec<Rect> = placed.iter().map(|target| target.rect).collect();
        let mut shown = Vec::new();
        for feature in self.sketches.clone() {
            let Some(plane) = scene::sketch_plane(model.document(), model.evaluation(), feature)
            else {
                continue;
            };
            let screen = SketchScreen::new(*outside.view, plane, outside.pixels_per_point);
            let surface = Surface {
                rect: outside.rect,
                screen: &screen,
                feature,
                interactive: outside.interactive,
                glyphs: false,
                highlight: outside.highlight,
                first_dimension_scales: false,
                outside: true,
                taken: &taken,
            };
            let annotations = self.annotations.entry(feature).or_default();
            annotations.show(ui, model, &surface, selection, actions);
            taken.extend(annotations.label_rects());
            self.hovered = self.hovered.or(annotations.hovered());
            shown.extend(
                annotations
                    .shown_dimensions()
                    .into_iter()
                    .map(|constraint| Pickable::SketchConstraint {
                        feature,
                        constraint,
                    }),
            );
        }
        shown.extend(
            placed
                .iter()
                .filter_map(|placed| self.values.get(placed.index))
                .map(|shown| value_pickable(&shown.value)),
        );
        self.shown = shown;
        self.paint_values(ui, outside, &placed);
        self.show_field(ui, model, outside, &placed, actions);
    }

    fn place_values(&self, ui: &Ui, outside: &Outside<'_>) -> Vec<PlacedValue> {
        let to_screen = |point: Point3| {
            let pixel = outside.view.project(point)? / outside.pixels_per_point;
            let at = outside.rect.min + vec2(pixel.x as f32, pixel.y as f32);
            at.is_finite().then_some(at)
        };
        let painter = ui.painter_at(outside.rect);
        let first = self.values.first().map(|shown| shown.value.feature);
        let mut labels = annotation_layout::obstacles();
        let mut stacks: BTreeMap<Stack, Pos2> = BTreeMap::new();
        let mut placed = Vec::new();
        for (index, shown) in self.values.iter().enumerate() {
            let size = canvas::chip_size(
                painter
                    .layout_no_wrap(shown.text.clone(), canvas::body(), Color32::PLACEHOLDER)
                    .size(),
            );
            let feature = shown.value.feature;
            let path: Vec<Pos2> = match &shown.value.drawn {
                Drawn::Path(points) => points
                    .iter()
                    .map(|point| to_screen(*point))
                    .collect::<Option<Vec<Pos2>>>()
                    .unwrap_or_default(),
                Drawn::Stacked | Drawn::Leader(_) => Vec::new(),
            };
            let leader = match shown.value.drawn {
                Drawn::Leader(at) => to_screen(at),
                Drawn::Stacked | Drawn::Path(_) => None,
            };
            let mut stacked = |stack: Stack, start: Pos2| {
                let next = stacks.entry(stack).or_insert(start);
                let rect = Rect::from_min_size(*next, size);
                *next = rect.left_bottom() + vec2(0.0, STACK_GAP);
                rect
            };
            let rect = if let Some(middle) = screen_halfway(&path) {
                Rect::from_center_size(middle, size)
            } else if let Some(at) = leader {
                stacked(
                    Stack::Leader(feature),
                    at + vec2(LEADER_REACH, -LEADER_REACH - size.y),
                )
            } else {
                let Some(anchor) = to_screen(shown.value.anchor) else {
                    continue;
                };
                stacked(
                    Stack::Centre(feature),
                    anchor + vec2(STACK_OFFSET, STACK_OFFSET),
                )
            };
            if !rect.intersects(outside.rect) {
                continue;
            }
            let taken = annotations::footprint(outside.rect, rect);
            if Some(feature) != first && annotation_layout::mostly_covered(&labels, &taken) {
                continue;
            }
            labels.add(taken);
            placed.push(PlacedValue {
                index,
                rect,
                path,
                leader,
            });
        }
        placed
    }

    fn paint_values(&mut self, ui: &Ui, outside: &Outside<'_>, placed: &[PlacedValue]) {
        let painter = ui.painter_at(outside.rect);
        let chrome = canvas::chrome(ui.ctx());
        for target in placed {
            let Some(shown) = self.values.get(target.index) else {
                continue;
            };
            let pickable = value_pickable(&shown.value);
            if outside.interactive {
                let id = Id::new(("model-value", shown.value.feature, shown.value.slot));
                let response = ui.interact(target.rect, id, Sense::click());
                let spoken = format!("{} {}: {}", shown.name, shown.value.caption, shown.text);
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &spoken));
                if response.hovered() {
                    self.hovered = Some(pickable);
                }
                if response.double_clicked() {
                    self.field = Some(OpenValue {
                        feature: shown.value.feature,
                        slot: shown.value.slot,
                        focus_pending: true,
                        select_all_pending: true,
                    });
                }
                response.on_hover_text(format!(
                    "{} of {}. {VALUE_HINT}",
                    shown.value.caption, shown.name
                ));
            }
            let open = self.field.is_some_and(|open| {
                open.feature == shown.value.feature && open.slot == shown.value.slot
            });
            let color = if self.hovered == Some(pickable) || outside.highlight == Some(pickable) {
                chrome.hovered
            } else {
                chrome.dimension
            };
            let stroke = Stroke::new(LINE_WIDTH, color);
            if target.path.len() >= 2 {
                painter.line(target.path.clone(), stroke);
                for (end, across) in end_ticks(&target.path) {
                    painter.line_segment([end - across, end + across], stroke);
                }
            }
            if let Some(at) = target.leader {
                painter.line_segment([at, target.rect.clamp(at)], stroke);
                painter.circle_filled(at, LEADER_DOT, color);
            }
            if !open {
                canvas::paint_backdrop(&painter, target.rect);
                painter.galley(
                    target.rect.min + canvas::PADDING,
                    painter.layout_no_wrap(shown.text.clone(), canvas::body(), color),
                    color,
                );
            }
        }
    }

    fn show_field(
        &mut self,
        ui: &Ui,
        model: &Model,
        outside: &Outside<'_>,
        placed: &[PlacedValue],
        actions: &mut Vec<Action>,
    ) {
        let Some(mut open) = self.field else {
            return;
        };
        let found = placed.iter().find_map(|target| {
            let shown = self.values.get(target.index)?;
            (shown.value.feature == open.feature && shown.value.slot == open.slot)
                .then_some((shown, target.rect))
        });
        let Some((shown, rect)) = found else {
            self.field = None;
            return;
        };
        let value = &shown.value;
        let id = value_field_id(value.feature, value.slot);
        let stored = field::driven_text(model.document(), &owner_of(value), &value.expression);
        let field = egui::Area::new(id.with("area"))
            .order(Order::Foreground)
            .fixed_pos(rect.center() - vec2(0.0, FIELD_LIFT))
            .pivot(Align2::CENTER_TOP)
            .constrain_to(outside.rect)
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
                            |text| value_change(model, value, text),
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
        if field.response.has_focus() {
            open.focus_pending = false;
            if open.select_all_pending {
                field::select_all(ui.ctx(), id, &stored);
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
