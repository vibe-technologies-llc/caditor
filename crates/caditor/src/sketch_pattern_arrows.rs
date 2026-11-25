use caditor_document::FeatureId;
use caditor_expression::Quantity;
use caditor_geometry::{Plane, Point2, Ray, Vector2, Vector3};
use caditor_render::{Batch, View};
use caditor_sketch::{Distribution, EntityId, Sides, Sketch, Spread};
use egui::{Align, Align2, Grid, Id, Layout, Order, Rect, Ui, vec2};

use crate::{
    feature_tree,
    field::{self, LiveField},
    model::Model,
    move_manipulator::{
        ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HIT_POINTS, segment_distance, step_for,
    },
    patterning::{Direction, PatternField, PatternSettings, PatternTextError, Patterning, Spec},
    scene_palette::ScenePalette,
    sketch_drag::{circles_of, extent_centre, points_of},
    sketch_handles::{degrees_of, plane_hit, step_value, wrapped},
    turn_handles::STEP_DEGREES,
    units::Units,
    widgets,
};

const ARROW_SHARE: f64 = 0.7;
const SMALLEST_RADIUS: f64 = 1e-6;
const FULL_TURN_DEGREES: f64 = 360.0;
const HALF_TURN_DEGREES: f64 = 180.0;
const FIELD_WIDTH: f32 = 72.0;
const FIELD_GAP: f32 = 8.0;
const FIELD_MARGIN: f32 = 4.0;
const SIDEWAYS: f64 = 0.5;
#[cfg(test)]
const GRIP_FRACTION: f64 = 0.6;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum PatternGrip {
    Distance(Direction),
    Angle,
}

impl PatternGrip {
    pub fn field(self) -> PatternField {
        match self {
            Self::Distance(direction) => PatternField::Spacing(direction),
            Self::Angle => PatternField::TotalAngle,
        }
    }

    pub fn count(self) -> PatternField {
        match self {
            Self::Distance(direction) => PatternField::Count(direction),
            Self::Angle => PatternField::Copies,
        }
    }

    pub fn words(self, settings: &PatternSettings) -> String {
        format!(
            "Drag to change the {}, or type it beside the arrow",
            settings.wording().noun(self.field())
        )
    }
}

pub fn field_id(field: PatternField) -> Id {
    Id::unique((
        "sketch-pattern-canvas-field",
        field.key(),
        field.direction().map(Direction::index),
    ))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Track {
    Line {
        base: Point2,
        along: Vector2,
        scale: f64,
    },
    Turn {
        centre: Point2,
        reach: Vector2,
    },
}

impl Track {
    fn at(self, value: f64) -> Point2 {
        match self {
            Self::Line { base, along, scale } => base + along * value * scale,
            Self::Turn { centre, reach } => {
                centre + Vector2::from_angle(value.to_radians()).rotate(reach)
            }
        }
    }

    fn direction(self, value: f64) -> Vector2 {
        match self {
            Self::Line { along, .. } if value < 0.0 => -along,
            Self::Line { along, .. } => along,
            Self::Turn { reach, .. } => {
                let tangent = Vector2::from_angle(value.to_radians())
                    .rotate(reach)
                    .perp()
                    .normalize_or_zero();
                if value < 0.0 { -tangent } else { tangent }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Limit {
    Flips,
    Least,
    Total,
    EachWay,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Shown {
    grip: PatternGrip,
    track: Track,
    limit: Limit,
    value: f64,
    sign: f64,
    per_point: f64,
}

impl Shown {
    fn foot(&self) -> Point2 {
        self.track.at(self.value)
    }

    fn direction(&self) -> Vector2 {
        self.track.direction(self.value)
    }

    fn segment(&self) -> (Point2, Point2) {
        let foot = self.foot();
        let direction = self.direction();
        (
            foot + direction * GAP_POINTS * self.per_point,
            foot + direction * ARROW_POINTS * ARROW_SHARE * self.per_point,
        )
    }

    fn step(&self) -> f64 {
        match self.track {
            Track::Line { .. } => step_for(self.per_point),
            Track::Turn { .. } => STEP_DEGREES,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PatternArrows {
    pub feature: FeatureId,
    plane: Plane,
    shown: Vec<Shown>,
    sight: Vector3,
}

pub struct Context<'a> {
    pub feature: FeatureId,
    pub sketch: &'a Sketch,
    pub settings: &'a PatternSettings,
    pub patterning: &'a Patterning,
}

impl PatternArrows {
    pub fn of(context: &Context<'_>, view: &View, pixels_per_point: f64) -> Option<Self> {
        let Context {
            feature,
            sketch,
            settings,
            patterning,
        } = context;
        let kind = patterning.kind();

        if patterning.picking().is_some() || !patterning.previewed() {
            return None;
        }

        let spec = settings.spec(kind)?;
        let centre = patterning.chosen_centre();
        let repeated: Vec<EntityId> = patterning
            .selected()
            .iter()
            .copied()
            .filter(|item| Some(*item) != centre)
            .collect();
        let points = points_of(sketch, &repeated);

        if points.is_empty() {
            return None;
        }

        let anchor = extent_centre(sketch, &points, &circles_of(sketch, &repeated));
        let plane = sketch.plane();
        let found: Vec<Shown> = match spec {
            Spec::Rectangular(pattern) => pattern
                .directions
                .iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    let unit = sketch.heading_direction(&row.heading).ok()?;
                    let unit = if row.reversed { -unit } else { unit };
                    let sign = if row.distance.value < 0.0 { -1.0 } else { 1.0 };
                    let gaps = row.count.saturating_sub(1) as f64;
                    let scale = match pattern.distribution {
                        Distribution::Spacing => gaps,
                        Distribution::Extent => 1.0,
                    };
                    let limit = match row.sides {
                        Sides::One => Limit::Flips,
                        Sides::Both => Limit::Least,
                    };

                    (row.count >= 2 && row.distance.value != 0.0).then_some(Shown {
                        grip: PatternGrip::Distance(Direction(index)),
                        track: Track::Line {
                            base: anchor,
                            along: unit * sign,
                            scale,
                        },
                        limit,
                        value: row.distance.value.abs(),
                        sign,
                        per_point: 1.0,
                    })
                })
                .collect(),
            Spec::Circular(pattern) => {
                let centre_at = centre.and_then(|point| centre_of(sketch, point));
                let (angle, limit) = match &pattern.spread {
                    Spread::Total(angle) => (Some(angle.value), Limit::Total),
                    Spread::Symmetric(angle) => (Some(angle.value), Limit::EachWay),
                    Spread::FullTurn => (None, Limit::Total),
                };
                let reach = centre_at.map(|at| anchor - at);

                centre_at
                    .zip(reach)
                    .zip(angle)
                    .filter(|((_, reach), angle)| {
                        pattern.count >= 2 && reach.length() > SMALLEST_RADIUS && *angle != 0.0
                    })
                    .map(|((centre, reach), angle)| Shown {
                        grip: PatternGrip::Angle,
                        track: Track::Turn { centre, reach },
                        limit,
                        value: angle,
                        sign: 1.0,
                        per_point: 1.0,
                    })
                    .into_iter()
                    .collect()
            }
        };

        let shown: Vec<Shown> = found
            .into_iter()
            .filter_map(|probe: Shown| {
                let foot = plane.to_world(probe.foot());
                let depth = view.view_depth(foot);
                let per_point = pixels_per_point * view.units_per_pixel_at(depth);

                let usable = view.is_drawn_at(depth) && per_point.is_finite() && per_point > 0.0;
                let direction = in_world(&plane, probe.direction());
                let seen = direction.dot(view.sight_at(foot)).abs() < END_ON;

                (usable && seen).then_some(Shown { per_point, ..probe })
            })
            .collect();
        (!shown.is_empty()).then(|| Self {
            feature: *feature,
            plane,
            shown,
            sight: view.forward(),
        })
    }

    fn find(&self, grip: PatternGrip) -> Option<&Shown> {
        self.shown.iter().find(|shown| shown.grip == grip)
    }

    pub fn grips(&self) -> impl Iterator<Item = PatternGrip> + '_ {
        self.shown.iter().map(|shown| shown.grip)
    }

    fn screen_segment(&self, view: &View, shown: &Shown) -> Option<(Vector2, Vector2)> {
        let (from, to) = shown.segment();
        Some((
            view.project(self.plane.to_world(from))?,
            view.project(self.plane.to_world(to))?,
        ))
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<PatternGrip> {
        let reach = HIT_POINTS * pixels_per_point;
        self.shown
            .iter()
            .filter_map(|shown| {
                let (from, to) = self.screen_segment(view, shown)?;
                Some((segment_distance(from, to, cursor), shown.grip))
            })
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, grip)| grip)
    }

    #[cfg(test)]
    pub fn grip(&self, grip: PatternGrip, along: f64) -> Option<caditor_geometry::Point3> {
        let shown = self.find(grip)?;
        let (from, tip) = shown.segment();

        let held = from.lerp(tip, GRIP_FRACTION);

        let moved = match shown.track {
            Track::Line { .. } => held + shown.direction() * along,
            Track::Turn { centre, .. } => {
                let turned = if shown.value < 0.0 { -along } else { along };
                centre + Vector2::from_angle(turned.to_radians()).rotate(held - centre)
            }
        };
        Some(self.plane.to_world(moved))
    }

    #[cfg(test)]
    pub fn foot(&self, grip: PatternGrip) -> Option<Point2> {
        Some(self.find(grip)?.foot())
    }

    pub fn beside(&self, view: &View, grip: PatternGrip) -> Option<(Vector2, Vector2)> {
        let shown = self.find(grip)?;
        let (from, to) = self.screen_segment(view, shown)?;
        Some((to, (to - from).normalize_or_zero()))
    }

    pub fn drawn(&self, highlighted: Option<PatternGrip>) -> Drawn {
        Drawn {
            arrows: self.clone(),
            highlighted,
        }
    }

    fn add_to(&self, batch: &mut Batch, palette: &ScenePalette, highlighted: Option<PatternGrip>) {
        for shown in &self.shown {
            let (from, tip) = shown.segment();
            let colour = if highlighted == Some(shown.grip) {
                palette.handle_highlighted
            } else {
                palette.handle
            };

            Arrow {
                from: self.plane.to_world(from),
                tip: self.plane.to_world(tip),
                direction: in_world(&self.plane, shown.direction()),
                per_point: shown.per_point,
                forward: self.sight,
            }
            .add_to(batch, colour);
        }
    }
}

fn in_world(plane: &Plane, direction: Vector2) -> Vector3 {
    plane.x_axis() * direction.x + plane.y_axis() * direction.y
}

fn centre_of(sketch: &Sketch, point: EntityId) -> Option<Point2> {
    if point == EntityId::ORIGIN {
        return Some(Point2::ZERO);
    }
    sketch.point(point)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    arrows: PatternArrows,
    highlighted: Option<PatternGrip>,
}

impl Drawn {
    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette) {
        self.arrows.add_to(batch, palette, self.highlighted);
    }
}

pub fn driven(model: &Model, settings: &PatternSettings, grip: PatternGrip) -> Option<String> {
    let field = grip.field();
    let text = settings.text(field)?;
    let document = model.document();

    let expression = field::parse_expression(
        document,
        model.parameters(),
        text,
        field.expected(),
        model.units(),
    )
    .ok()?;
    let used = expression.parameters();

    if used.is_empty() {
        return None;
    }

    let names: Vec<String> = used
        .into_iter()
        .map(|id| {
            document.parameter(id).map_or_else(
                || "a deleted parameter".to_owned(),
                |parameter| parameter.name.clone(),
            )
        })
        .collect();
    let names = feature_tree::in_words(&names);
    let caption = settings.wording().caption(field);
    Some(format!(
        "{caption} follows {names}; change {names} in Parameters or type a value in the panel"
    ))
}

#[derive(Clone, Debug, PartialEq)]
struct Kept {
    text: String,
    reversed: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Follow {
    Line { grabbed: f64 },
    Turn { last: f64, swept: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PatternArrowDrag {
    pub feature: FeatureId,
    pub grip: PatternGrip,
    plane: Plane,
    track: Track,
    limit: Limit,
    follow: Follow,
    sign: f64,
    step: f64,
    from: f64,
    value: f64,
    caption: &'static str,
    kept: Kept,
}

impl PatternArrowDrag {
    pub fn begin(
        arrows: &PatternArrows,
        settings: &PatternSettings,
        grip: PatternGrip,
        ray: Ray,
    ) -> Option<Self> {
        let shown = arrows.find(grip)?;
        let pointed = plane_hit(&arrows.plane, ray)?;
        let direction = grip.field().direction();

        let follow = match shown.track {
            Track::Line { along, .. } => Follow::Line {
                grabbed: (pointed - Point2::ZERO).dot(along),
            },
            Track::Turn { centre, .. } => Follow::Turn {
                last: degrees_of(pointed - centre)?,
                swept: 0.0,
            },
        };
        let kept = Kept {
            text: settings.text(grip.field())?.to_owned(),
            reversed: direction.is_some_and(|direction| settings.reversed(direction)),
        };
        Some(Self {
            feature: arrows.feature,
            grip,
            plane: arrows.plane,
            track: shown.track,
            limit: shown.limit,
            follow,
            sign: shown.sign,
            step: shown.step(),
            from: shown.value,
            value: shown.value,
            caption: settings.wording().caption(grip.field()),
            kept,
        })
    }

    pub fn follow(&mut self, ray: Ray, free: bool) -> bool {
        let Some(pointed) = plane_hit(&self.plane, ray) else {
            return false;
        };
        let wanted = match (&mut self.follow, self.track) {
            (Follow::Line { grabbed }, Track::Line { along, scale, .. }) => {
                if scale <= 0.0 {
                    return false;
                }

                let moved = (pointed - Point2::ZERO).dot(along) - *grabbed;
                self.from + moved / scale
            }
            (Follow::Turn { last, swept }, Track::Turn { centre, .. }) => {
                let Some(now) = degrees_of(pointed - centre) else {
                    return false;
                };

                *swept += wrapped(now - *last);
                *last = now;
                self.from + *swept
            }
            (Follow::Line { .. }, Track::Turn { .. })
            | (Follow::Turn { .. }, Track::Line { .. }) => {
                return false;
            }
        };
        let value = self.limited(step_value(wanted, self.step, free));
        let changed = value != self.value;

        self.value = value;
        changed
    }

    fn limited(&self, value: f64) -> f64 {
        let least = self.step;
        match self.limit {
            Limit::Least => value.max(least),
            Limit::Flips | Limit::Total => {
                let most = match self.limit {
                    Limit::Total => FULL_TURN_DEGREES - least,
                    Limit::Flips | Limit::Least | Limit::EachWay => f64::INFINITY,
                };
                let size = value.abs().clamp(least, most);
                let leaning = if value == 0.0 { self.from } else { value };
                if leaning < 0.0 { -size } else { size }
            }
            Limit::EachWay => {
                let size = value.abs().clamp(least, HALF_TURN_DEGREES - least);
                if self.from < 0.0 { -size } else { size }
            }
        }
    }

    fn flipped(&self) -> bool {
        self.limit == Limit::Flips && self.value < 0.0
    }

    fn text(&self, units: Units) -> String {
        match self.track {
            Track::Line { .. } => units.show(Quantity::length(self.value.abs() * self.sign)),
            Track::Turn { .. } => units.show(Quantity::angle(self.value)),
        }
    }

    pub fn write(&self, settings: &mut PatternSettings, units: Units) {
        let text = self.text(units);

        if let Some(held) = settings.text_mut(self.grip.field()) {
            *held = text;
        }
        if let Some(direction) = self.grip.field().direction() {
            settings.set_reversed(direction, self.kept.reversed != self.flipped());
        }
    }

    pub fn cancel(&self, settings: &mut PatternSettings) {
        if let Some(held) = settings.text_mut(self.grip.field()) {
            held.clone_from(&self.kept.text);
        }
        if let Some(direction) = self.grip.field().direction() {
            settings.set_reversed(direction, self.kept.reversed);
        }
    }

    pub fn readout(&self, units: Units) -> String {
        let value = match self.track {
            Track::Line { .. } => units.readout_text(self.value.abs() * self.sign),
            Track::Turn { .. } => units.angle_readout(self.value),
        };
        let shown = format!("{} {value}", self.caption);

        if self.flipped() {
            format!("{shown}, flipped")
        } else {
            shown
        }
    }
}

pub fn show_fields(
    ui: &Ui,
    (rect, pixels_per_point): (Rect, f32),
    (view, arrows): (&View, &PatternArrows),
    (settings, problems): (&mut PatternSettings, &[(PatternField, PatternTextError)]),
) {
    let grips: Vec<PatternGrip> = arrows.grips().collect();

    for grip in grips {
        let Some((tip, heading)) = arrows.beside(view, grip) else {
            continue;
        };
        let at = rect.min + vec2(tip.x as f32, tip.y as f32) / pixels_per_point;
        let (pivot, gap) = placed(heading);

        egui::Area::new(field_id(grip.field()).with("area"))
            .order(Order::Foreground)
            .fixed_pos(at + gap)
            .pivot(pivot)
            .constrain_to(rect)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(FIELD_MARGIN)
                    .show(ui, |ui| {
                        Grid::new(field_id(grip.field()).with("grid"))
                            .num_columns(2)
                            .show(ui, |ui| {
                                for field in [grip.count(), grip.field()] {
                                    field_row(ui, settings, problems, field);
                                }
                            });
                    });
            });
    }
}

fn field_row(
    ui: &mut Ui,
    settings: &mut PatternSettings,
    problems: &[(PatternField, PatternTextError)],
    field: PatternField,
) {
    let wording = settings.wording();

    widgets::caption(ui, wording.caption(field));

    let Some(text) = settings.text_mut(field) else {
        ui.end_row();
        return;
    };
    let room = vec2(FIELD_WIDTH, ui.spacing().interact_size.y);

    let LiveField { response, .. } = ui
        .allocate_ui_with_layout(room, Layout::left_to_right(Align::Center), |ui| {
            field::live_value_field(ui, field_id(field), text, field.placeholder())
        })
        .inner;

    response.on_hover_text(wording.about(field));
    ui.end_row();
    if let Some((_, problem)) = problems.iter().find(|(wrong, _)| *wrong == field) {
        widgets::error_row(ui, &problem.to_string());
    }
}

fn placed(heading: Vector2) -> (Align2, egui::Vec2) {
    if heading.x.abs() >= heading.y.abs() * SIDEWAYS {
        if heading.x >= 0.0 {
            (Align2::LEFT_CENTER, vec2(FIELD_GAP, 0.0))
        } else {
            (Align2::RIGHT_CENTER, vec2(-FIELD_GAP, 0.0))
        }
    } else if heading.y >= 0.0 {
        (Align2::CENTER_TOP, vec2(0.0, FIELD_GAP))
    } else {
        (Align2::CENTER_BOTTOM, vec2(0.0, -FIELD_GAP))
    }
}

pub fn field_open(ctx: &egui::Context, settings: &PatternSettings) -> bool {
    let focused = ctx.memory(|memory| memory.focused());
    focused.is_some_and(|focused| {
        settings
            .directions()
            .flat_map(|direction| {
                [
                    PatternField::Count(direction),
                    PatternField::Spacing(direction),
                ]
            })
            .chain([PatternField::Copies, PatternField::TotalAngle])
            .any(|field| field_id(field) == focused)
    })
}
