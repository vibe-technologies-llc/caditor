use caditor_document::{Document, FeatureId, FeatureKind, Transaction};
use caditor_expression::Expression;
use caditor_geometry::{Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, View};

use crate::{
    curve_track::CurveTrack,
    feature_fields::Rule,
    handle_snap::{self, Guide, Snap},
    manipulator::{self, Held},
    model::Model,
    move_manipulator::{
        ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HEAD_HALF_POINTS, HIT_POINTS, Handle,
        HandleSegment, segment_distance, segments_apart,
    },
    scene_palette::ScenePalette,
    turn_handles::Swing,
    units::Units,
    value_gauges::{Builder, Gauge, Mapping, Measured, Track},
};

const ARROW_SHARE: f64 = 0.7;
const MOST_GAUGES: usize = 24;
const CLEARANCE_POINTS: f64 = 2.0 * HEAD_HALF_POINTS;
#[cfg(test)]
const GRIP_FRACTION: f64 = 0.6;

#[derive(Debug, Clone, PartialEq)]
struct Shown {
    gauge: Gauge,
    per_point: f64,
}

impl Shown {
    fn foot(&self) -> Point3 {
        self.gauge.track.at(self.position())
    }

    fn position(&self) -> f64 {
        self.gauge.mapping.position(self.gauge.value)
    }

    fn direction(&self) -> Vector3 {
        self.gauge.track.direction(self.position())
    }

    fn segment(&self) -> (Point3, Point3) {
        let foot = self.foot();
        let direction = self.direction();
        (
            foot + direction * GAP_POINTS * self.per_point,
            foot + direction * ARROW_POINTS * ARROW_SHARE * self.per_point,
        )
    }
}

fn shown_kind(model: &Model, feature: FeatureId) -> Option<&FeatureKind> {
    model
        .draft_kind(feature)
        .or_else(|| Some(&model.document().feature(feature)?.kind))
}

fn committed_kind(document: &Document, feature: FeatureId) -> Option<&FeatureKind> {
    Some(&document.feature(feature)?.kind)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueHandles {
    pub feature: FeatureId,
    shown: Vec<Shown>,
    forward: Vector3,
}

impl ValueHandles {
    pub fn of(
        model: &Model,
        feature: FeatureId,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let kind = shown_kind(model, feature)?;
        let forward = view.forward();
        let per_point_at = |at: Point3| {
            let depth = view.view_depth(at);
            let per_point = pixels_per_point * view.units_per_pixel_at(depth);
            (view.is_drawn_at(depth) && per_point.is_finite() && per_point > 0.0)
                .then_some(per_point)
        };
        let builder = Builder {
            model,
            feature,
            per_point: &per_point_at,
        };
        let shown: Vec<Shown> = builder
            .gauges(kind)
            .into_iter()
            .filter_map(|gauge| {
                let probe = Shown {
                    gauge,
                    per_point: 1.0,
                };
                let per_point = per_point_at(probe.foot())?;
                let sight = view.sight_at(probe.foot());
                let seen = probe.direction().dot(sight).abs() < END_ON;
                seen.then_some(Shown { per_point, ..probe })
            })
            .take(MOST_GAUGES)
            .collect();
        (!shown.is_empty()).then_some(Self {
            feature,
            shown,
            forward,
        })
    }

    pub fn clear_of(
        self,
        view: &View,
        pixels_per_point: f64,
        taken: &[HandleSegment],
    ) -> Option<Self> {
        let clearance = CLEARANCE_POINTS * pixels_per_point;
        let on_screen = |(from, to): HandleSegment| Some((view.project(from)?, view.project(to)?));
        let mut drawn: Vec<(Vector2, Vector2)> =
            taken.iter().copied().filter_map(on_screen).collect();
        let mut shown = Vec::with_capacity(self.shown.len());
        for found in self.shown {
            let Some(seen) = on_screen(found.segment()) else {
                continue;
            };
            if drawn
                .iter()
                .all(|other| segments_apart(seen, *other) >= clearance)
            {
                drawn.push(seen);
                shown.push(found);
            }
        }
        (!shown.is_empty()).then_some(Self { shown, ..self })
    }

    fn all(&self) -> impl Iterator<Item = &Shown> {
        self.shown.iter()
    }

    fn find(&self, measured: Measured) -> Option<&Shown> {
        self.all().find(|shown| shown.gauge.measured == measured)
    }

    #[cfg(test)]
    pub fn step(&self) -> f64 {
        self.all()
            .next()
            .map_or(1.0, |shown| shown.gauge.measured.step(shown.per_point))
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        let Handle::Value(measured) = handle else {
            return None;
        };
        held(model.document(), self.feature, measured)?.driven()
    }

    pub fn words(&self, model: &Model, measured: Measured) -> String {
        shown_kind(model, self.feature).map_or_else(String::new, |kind| measured.words(kind))
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let reach = HIT_POINTS * pixels_per_point;
        self.all()
            .filter_map(|shown| {
                let (from, to) = shown.segment();
                let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
                Some((distance, Handle::Value(shown.gauge.measured)))
            })
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, handle)| handle)
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<Point3> {
        let Handle::Value(measured) = handle else {
            return None;
        };
        let shown = self.find(measured)?;
        let (from, tip) = shown.segment();
        let grip = from.lerp(tip, GRIP_FRACTION);
        match &shown.gauge.track {
            Track::Line { direction, .. } => Some(grip + *direction * along),
            Track::Turn(swing) => {
                let axis = swing.radial.cross(swing.normal);
                let rotation =
                    caditor_geometry::Rotation3::from_axis_angle(axis, along.to_radians());
                Some(swing.foot + rotation * (grip - swing.foot))
            }
            Track::Curve(track) => {
                let position = shown.position() + along;
                Some(track.at(position) + track.tangent(position) * grip.distance(shown.foot()))
            }
        }
    }

    #[cfg(test)]
    pub fn foot(&self, handle: Handle) -> Option<Point3> {
        let Handle::Value(measured) = handle else {
            return None;
        };
        Some(self.find(measured)?.foot())
    }

    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette, highlighted: Option<Handle>) {
        for shown in self.all() {
            let (from, tip) = shown.segment();
            let colour = if highlighted == Some(Handle::Value(shown.gauge.measured)) {
                palette.handle_highlighted
            } else {
                palette.handle
            };
            Arrow {
                from,
                tip,
                direction: shown.direction(),
                per_point: shown.per_point,
                forward: self.forward,
            }
            .add_to(batch, colour);
        }
    }

    pub fn typing(&self, model: &Model, measured: Measured) -> Option<Typing> {
        let kind = committed_kind(model.document(), self.feature)?;
        Some(Typing {
            feature: self.feature,
            measured,
            caption: measured.caption(kind)?,
            rule: measured.rule(kind),
        })
    }
}

fn held(document: &Document, feature: FeatureId, measured: Measured) -> Option<Held> {
    let mut probe = committed_kind(document, feature)?.clone();
    let caption = measured.caption(&probe)?;
    Some(Held::of(
        document,
        feature,
        &caption,
        measured.slot(&mut probe)?,
    ))
}

pub fn written(
    model: &Model,
    feature: FeatureId,
    measured: Measured,
    value: Expression,
) -> Option<Transaction> {
    let kind = committed_kind(model.document(), feature)?.clone();
    written_into(model, feature, measured, kind, value)
}

fn written_into(
    model: &Model,
    feature: FeatureId,
    measured: Measured,
    mut kind: FeatureKind,
    value: Expression,
) -> Option<Transaction> {
    let document = model.document();
    let held = held(document, feature, measured)?;
    let mut named = Vec::new();
    held.set(measured.slot(&mut kind)?, value, &mut named);
    measured.tidy(&mut kind);
    manipulator::keeping_names(document, feature, kind, named)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Typing {
    pub feature: FeatureId,
    pub measured: Measured,
    pub caption: String,
    pub rule: Rule,
}

#[derive(Debug, Clone, PartialEq)]
enum Follow {
    Line {
        base: Point3,
        direction: Vector3,
        grabbed: f64,
    },
    Turn {
        swing: Swing,
        last: f64,
        swept: f64,
    },
    Curve {
        track: CurveTrack,
        grabbed: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueDrag {
    start: FeatureKind,
    measured: Measured,
    mapping: Mapping,
    follow: Follow,
    step: f64,
    reverses: bool,
    from: f64,
    value: f64,
    snapped: Option<Snap>,
}

impl ValueDrag {
    pub fn begin(
        model: &Model,
        handles: &ValueHandles,
        measured: Measured,
        ray: Ray,
    ) -> Option<Self> {
        let start = committed_kind(model.document(), handles.feature)?.clone();
        let shown = handles.find(measured)?;
        let gauge = &shown.gauge;
        let mut probe = start.clone();
        let parameters = model.parameters();
        let from = measured
            .slot(&mut probe)?
            .evaluate_as(measured.amount().dimension(), &|id| parameters.value(id))
            .ok()?;
        let follow = match &gauge.track {
            Track::Line { base, direction } => Follow::Line {
                base: *base,
                direction: *direction,
                grabbed: ray.closest_along_line(*base, *direction)?,
            },
            Track::Turn(swing) => Follow::Turn {
                swing: *swing,
                last: swing.degrees_of(ray)?,
                swept: 0.0,
            },
            Track::Curve(track) => Follow::Curve {
                track: track.clone(),
                grabbed: track.nearest_to_ray(&ray)?,
            },
        };
        let reverses = measured.reversed(&start).is_some();
        Some(Self {
            start,
            measured,
            mapping: gauge.mapping,
            follow,
            step: measured.step(shown.per_point),
            reverses,
            from,
            value: from,
            snapped: None,
        })
    }

    pub fn guide(&self) -> Guide {
        match &self.follow {
            Follow::Line {
                base, direction, ..
            } => Guide::Line {
                base: *base,
                direction: *direction,
            },
            Follow::Turn { .. } | Follow::Curve { .. } => Guide::Points,
        }
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        let from_position = self.mapping.position(self.from);
        let pulled_position = match &mut self.follow {
            Follow::Line {
                base,
                direction,
                grabbed,
            } => ray
                .closest_along_line(*base, *direction)
                .map(|at| from_position + at - *grabbed),
            Follow::Turn { swing, last, swept } => {
                swing.degrees_near(ray, Some(*last)).map(|now| {
                    let turned = (now - *last + 180.0).rem_euclid(360.0) - 180.0;
                    *last = now;
                    *swept += turned;
                    from_position + *swept
                })
            }
            Follow::Curve { track, grabbed } => track
                .nearest_to_ray(&ray)
                .map(|at| from_position + at - *grabbed),
        };
        let pulled = pulled_position.and_then(|position| self.mapping.value(position));
        let snapped = match (&self.follow, snap.filter(|_| !free)) {
            (
                Follow::Line {
                    base, direction, ..
                },
                Some(snap),
            ) => snap
                .along(*base, *direction)
                .and_then(|position| self.mapping.value(position))
                .filter(|value| {
                    let off = handle_snap::off_line(&ray, *base, *direction);
                    snap.holds(*value, pulled, off, self.step)
                })
                .map(|value| (value, snap)),
            (Follow::Turn { swing, .. }, Some(snap)) => pulled_position
                .and_then(|near| swing.snapped_near(snap, near))
                .and_then(|position| self.mapping.value(position))
                .filter(|value| self.limited(*value) == *value)
                .map(|value| (value, snap)),
            (Follow::Curve { track, .. }, Some(snap)) => snap
                .pinned_point()
                .and_then(|point| {
                    let position = track.nearest_to_point(point)?;
                    let off = track.at(position).distance(point);
                    Some((self.mapping.value(position)?, off))
                })
                .filter(|(value, off)| snap.holds_on_track(*off, *value, pulled, self.step))
                .map(|(value, _)| (value, snap)),
            _ => None,
        };
        let (wanted, snap) = match snapped {
            Some((value, snap)) => (value, Some(snap)),
            None => {
                let Some(wanted) = pulled else {
                    return false;
                };
                let rounded = if free {
                    wanted
                } else {
                    (wanted / self.step).round() * self.step
                };
                (rounded, None)
            }
        };
        let value = self.limited(wanted);
        let changed = value != self.value || snap != self.snapped;
        self.value = value;
        self.snapped = snap;
        changed
    }

    fn limited(&self, value: f64) -> f64 {
        if !self.reverses {
            return self.measured.accepts(value, self.step, &self.start);
        }
        let size = self.measured.accepts(value.abs(), self.step, &self.start);
        if value < 0.0 { -size } else { size }
    }

    fn flips(&self) -> bool {
        self.reverses && self.value < 0.0
    }

    fn size(&self) -> f64 {
        if self.flips() {
            -self.value
        } else {
            self.value
        }
    }

    pub fn has_moved(&self) -> bool {
        self.value != self.from
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let value = self
            .measured
            .amount()
            .expression(model.units(), self.size());
        let committed = committed_kind(model.document(), feature)?;
        let kind = if self.flips() {
            self.measured.reversed(committed)?
        } else {
            committed.clone()
        };
        written_into(model, feature, self.measured, kind, value)
    }

    pub fn readout(&self, units: Units) -> String {
        let caption = self.measured.caption(&self.start).unwrap_or_default();
        let value = self.measured.amount().text(units, self.size());
        let shown = match self.snapped {
            Some(snap) => format!("{caption} {value} {}", snap.kind.words()),
            None => format!("{caption} {value}"),
        };
        if self.flips() {
            format!("{shown}, {}", self.measured.reversal_words())
        } else {
            shown
        }
    }
}
