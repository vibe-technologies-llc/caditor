use caditor_document::{Document, FeatureId, FeatureKind, Transaction};
use caditor_expression::Expression;
use caditor_geometry::{Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, View};

use crate::{
    handle_snap::Snap,
    manipulator::{self, Held},
    model::Model,
    move_manipulator::{
        ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HIT_POINTS, Handle, segment_distance,
    },
    scene_palette::ScenePalette,
    turn_handles::Swing,
    units::Units,
    value_gauges::{Builder, Gauge, Mapping, Measured, Track},
};

const ARROW_SHARE: f64 = 0.7;
const MOST_GAUGES: usize = 4;
#[cfg(test)]
const GRIP_FRACTION: f64 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Shown {
    gauge: Gauge,
    per_point: f64,
}

impl Shown {
    fn foot(&self) -> Point3 {
        let position = self.gauge.mapping.position(self.gauge.value);
        match self.gauge.track {
            Track::Line { base, direction } => base + direction * position,
            Track::Turn(swing) => swing.at(position),
        }
    }

    fn direction(&self) -> Vector3 {
        match self.gauge.track {
            Track::Line { direction, .. } => direction,
            Track::Turn(swing) => swing.tangent(self.gauge.mapping.position(self.gauge.value)),
        }
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueHandles {
    pub feature: FeatureId,
    shown: [Option<Shown>; MOST_GAUGES],
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
            (depth.is_finite() && depth > 0.0 && per_point.is_finite() && per_point > 0.0)
                .then_some(per_point)
        };
        let builder = Builder {
            model,
            feature,
            per_point: &per_point_at,
        };
        let mut shown = [None; MOST_GAUGES];
        let visible = builder.gauges(kind).into_iter().filter_map(|gauge| {
            let probe = Shown {
                gauge,
                per_point: 1.0,
            };
            let per_point = per_point_at(probe.foot())?;
            let seen = probe.direction().dot(forward).abs() < END_ON;
            seen.then_some(Shown { gauge, per_point })
        });
        for (slot, found) in shown.iter_mut().zip(visible) {
            *slot = Some(found);
        }
        shown.iter().any(Option::is_some).then_some(Self {
            feature,
            shown,
            forward,
        })
    }

    fn all(&self) -> impl Iterator<Item = &Shown> {
        self.shown.iter().flatten()
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
        match shown.gauge.track {
            Track::Line { direction, .. } => Some(grip + direction * along),
            Track::Turn(swing) => {
                let axis = swing.radial.cross(swing.normal);
                let rotation =
                    caditor_geometry::Rotation3::from_axis_angle(axis, along.to_radians());
                Some(swing.foot + rotation * (grip - swing.foot))
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
    let document = model.document();
    let held = held(document, feature, measured)?;
    let mut kind = committed_kind(document, feature)?.clone();
    let mut named = Vec::new();
    held.set(measured.slot(&mut kind)?, value, &mut named);
    measured.tidy(&mut kind);
    manipulator::keeping_names(document, feature, kind, named)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typing {
    pub feature: FeatureId,
    pub measured: Measured,
    pub caption: String,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueDrag {
    start: FeatureKind,
    measured: Measured,
    mapping: Mapping,
    follow: Follow,
    step: f64,
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
        let gauge = shown.gauge;
        let mut probe = start.clone();
        let parameters = model.parameters();
        let from = measured
            .slot(&mut probe)?
            .evaluate_as(measured.amount().dimension(), &|id| parameters.value(id))
            .ok()?;
        let follow = match gauge.track {
            Track::Line { base, direction } => Follow::Line {
                base,
                direction,
                grabbed: ray.closest_along_line(base, direction)?,
            },
            Track::Turn(swing) => Follow::Turn {
                swing,
                last: swing.degrees_of(ray)?,
                swept: 0.0,
            },
        };
        Some(Self {
            start,
            measured,
            mapping: gauge.mapping,
            follow,
            step: measured.step(shown.per_point),
            from,
            value: from,
            snapped: None,
        })
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        let from_position = self.mapping.position(self.from);
        let pulled = match &mut self.follow {
            Follow::Line {
                base,
                direction,
                grabbed,
            } => ray
                .closest_along_line(*base, *direction)
                .map(|at| from_position + at - *grabbed),
            Follow::Turn { swing, last, swept } => swing.degrees_of(ray).map(|now| {
                let turned = (now - *last + 180.0).rem_euclid(360.0) - 180.0;
                *last = now;
                *swept += turned;
                from_position + *swept
            }),
        };
        let pulled = pulled.and_then(|position| self.mapping.value(position));
        let snapped = match (&self.follow, snap.filter(|_| !free)) {
            (
                Follow::Line {
                    base, direction, ..
                },
                Some(snap),
            ) => snap
                .along(*base, *direction)
                .and_then(|position| self.mapping.value(position))
                .filter(|value| snap.holds(*value, pulled, self.step))
                .map(|value| (value, snap)),
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
        let value = self.measured.accepts(wanted, self.step, &self.start);
        let changed = value != self.value || snap != self.snapped;
        self.value = value;
        self.snapped = snap;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.value != self.from
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let value = self.measured.amount().expression(model.units(), self.value);
        written(model, feature, self.measured, value)
    }

    pub fn readout(&self, units: Units) -> String {
        let caption = self.measured.caption(&self.start).unwrap_or_default();
        let value = self.measured.amount().text(units, self.value);
        match self.snapped {
            Some(snap) => format!("{caption} {value} {}", snap.kind.words()),
            None => format!("{caption} {value}"),
        }
    }
}
