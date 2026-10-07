use caditor_document::{FeatureId, FeatureKind, Move, MoveAxis, Transaction};
use caditor_expression::Dimension;
use caditor_geometry::{Plane, Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, Color, Fill, Layer, Line, Stroke, View};

use crate::{
    canvas,
    model::Model,
    move_tools, scene,
    selection::Axis,
    units::{LengthUnit, Units},
};

const ARROW_POINTS: f64 = 96.0;
const GAP_POINTS: f64 = 14.0;
const HEAD_POINTS: f64 = 18.0;
const HEAD_HALF_POINTS: f64 = 7.0;
const SHAFT_WIDTH: f32 = 3.0;
const SQUARE_FROM: f64 = 0.3;
const SQUARE_TO: f64 = 0.5;
const SQUARE_ALPHA: f32 = 0.45;
const HIT_POINTS: f64 = 8.0;
const END_ON: f64 = 0.97;
const EDGE_ON: f64 = 0.2;
const STEP_POINTS: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Along(MoveAxis),
    Across(MoveAxis),
}

impl Handle {
    const ALL: [Self; 6] = [
        Self::Across(MoveAxis::X),
        Self::Across(MoveAxis::Y),
        Self::Across(MoveAxis::Z),
        Self::Along(MoveAxis::X),
        Self::Along(MoveAxis::Y),
        Self::Along(MoveAxis::Z),
    ];

    fn moves(self) -> Vec<MoveAxis> {
        match self {
            Self::Along(axis) => vec![axis],
            Self::Across(normal) => MoveAxis::ALL
                .into_iter()
                .filter(|axis| *axis != normal)
                .collect(),
        }
    }

    pub fn words(self) -> String {
        match self {
            Self::Along(axis) => format!("Drag to move the body along {}", axis.name()),
            Self::Across(_) => {
                let names: Vec<&str> = self.moves().iter().map(|axis| axis.name()).collect();
                format!("Drag to move the body in the {} plane", names.concat())
            }
        }
    }
}

fn axis_colour(axis: MoveAxis) -> Color {
    let shown = match axis {
        MoveAxis::X => Axis::X,
        MoveAxis::Y => Axis::Y,
        MoveAxis::Z => Axis::Z,
    };
    let [red, green, blue] = shown.rgb();
    Color::from_rgb8(red, green, blue)
}

const HIGHLIGHTED: Color = scene::opaque(canvas::HOVERED);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Manipulator {
    pub feature: FeatureId,
    origin: Point3,
    reach: f64,
    forward: Vector3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    manipulator: Manipulator,
    highlighted: Option<Handle>,
}

impl Manipulator {
    pub fn of(
        model: &Model,
        open: Option<FeatureId>,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let feature = open?;
        let FeatureKind::Move(movement) = &model.document().feature(feature)?.kind else {
            return None;
        };
        let body = if movement.copy {
            feature
        } else {
            movement.body
        };
        let centre = model.evaluation().body(body)?.bounding_box()?.center();
        let origin = match model.draft_placement() {
            Some((moved, placement)) if moved == body => placement.apply_point(centre),
            Some(_) | None => centre,
        };
        let depth = view.view_depth(origin);
        if !(depth.is_finite() && depth > 0.0) {
            return None;
        }
        let reach = ARROW_POINTS * pixels_per_point * view.units_per_pixel_at(depth);
        (reach.is_finite() && reach > 0.0).then_some(Self {
            feature,
            origin,
            reach,
            forward: view.forward(),
        })
    }

    fn per_point(&self) -> f64 {
        self.reach / ARROW_POINTS
    }

    pub fn step(&self) -> f64 {
        nice_step(self.per_point() * STEP_POINTS)
    }

    fn arrow(&self, axis: MoveAxis) -> Option<(Point3, Point3)> {
        let direction = axis.direction();
        (direction.dot(self.forward).abs() < END_ON).then(|| {
            (
                self.origin + direction * GAP_POINTS * self.per_point(),
                self.origin + direction * self.reach,
            )
        })
    }

    fn square(&self, normal: MoveAxis) -> Option<[Point3; 4]> {
        if normal.direction().dot(self.forward).abs() < EDGE_ON {
            return None;
        }
        let [first, second] = <[MoveAxis; 2]>::try_from(Handle::Across(normal).moves()).ok()?;
        let corner = |a: f64, b: f64| {
            self.origin + first.direction() * a * self.reach + second.direction() * b * self.reach
        };
        Some([
            corner(SQUARE_FROM, SQUARE_FROM),
            corner(SQUARE_TO, SQUARE_FROM),
            corner(SQUARE_TO, SQUARE_TO),
            corner(SQUARE_FROM, SQUARE_TO),
        ])
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let inside = Handle::ALL.into_iter().find(|handle| match handle {
            Handle::Across(normal) => self.square(*normal).is_some_and(|corners| {
                let projected: Option<Vec<Vector2>> =
                    corners.iter().map(|corner| view.project(*corner)).collect();
                projected.is_some_and(|projected| within(&projected, cursor))
            }),
            Handle::Along(_) => false,
        });
        let reach = HIT_POINTS * pixels_per_point;
        let nearest = MoveAxis::ALL
            .into_iter()
            .filter_map(|axis| {
                let (from, to) = self.arrow(axis)?;
                let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
                (distance <= reach).then_some((distance, Handle::Along(axis)))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, handle)| handle);
        nearest.or(inside)
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<Point3> {
        match handle {
            Handle::Along(axis) => {
                let (from, tip) = self.arrow(axis)?;
                Some(from.lerp(tip, 0.6) + axis.direction() * along)
            }
            Handle::Across(normal) => {
                let corners = self.square(normal)?;
                Some(corners[0].lerp(corners[2], 0.5))
            }
        }
    }

    pub fn drawn(self, highlighted: Option<Handle>) -> Drawn {
        Drawn {
            manipulator: self,
            highlighted,
        }
    }
}

impl Drawn {
    pub fn add_to(&self, batch: &mut Batch) {
        let manipulator = &self.manipulator;
        let colour = |handle: Handle, axis: MoveAxis| {
            if self.highlighted == Some(handle) {
                HIGHLIGHTED
            } else {
                axis_colour(axis)
            }
        };
        for normal in MoveAxis::ALL {
            if let Some(corners) = manipulator.square(normal) {
                let handle = Handle::Across(normal);
                batch.fills.push(Fill::convex(
                    &corners,
                    colour(handle, normal).with_alpha(SQUARE_ALPHA),
                    Layer::Front,
                    None,
                ));
            }
        }
        for axis in MoveAxis::ALL {
            let Some((from, tip)) = manipulator.arrow(axis) else {
                continue;
            };
            let handle = Handle::Along(axis);
            let colour = colour(handle, axis);
            let direction = axis.direction();
            let head = HEAD_POINTS * manipulator.per_point();
            let base = tip - direction * head;
            batch.lines.push(Line {
                start: from,
                end: base,
                color: colour,
                width: SHAFT_WIDTH,
                layer: Layer::Front,
                pick: None,
                stroke: Stroke::Solid,
            });
            let Some(side) = direction.cross(manipulator.forward).try_normalize() else {
                continue;
            };
            let half = side * HEAD_HALF_POINTS * manipulator.per_point();
            batch.fills.push(Fill::convex(
                &[tip, base + half, base - half],
                colour,
                Layer::Front,
                None,
            ));
        }
    }
}

fn within(polygon: &[Vector2], point: Vector2) -> bool {
    let turns: Vec<f64> = polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(a, b)| (*b - *a).perp_dot(point - *a))
        .collect();
    turns.iter().all(|turn| *turn >= 0.0) || turns.iter().all(|turn| *turn <= 0.0)
}

fn segment_distance(from: Vector2, to: Vector2, point: Vector2) -> f64 {
    let along = to - from;
    let length_squared = along.length_squared();
    if length_squared == 0.0 {
        return from.distance(point);
    }
    let fraction = ((point - from).dot(along) / length_squared).clamp(0.0, 1.0);
    (from + along * fraction).distance(point)
}

fn nice_step(at_least: f64) -> f64 {
    if !(at_least.is_finite() && at_least > 0.0) {
        return 1.0;
    }
    let decade = 10.0_f64.powf(at_least.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|factor| factor * decade)
        .find(|step| *step >= at_least)
        .unwrap_or(10.0 * decade)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manipulating {
    pub feature: FeatureId,
    pub handle: Handle,
    start: Move,
    from: [f64; 3],
    origin: Point3,
    grabbed: Point3,
    step: f64,
    offset: [f64; 3],
}

impl Manipulating {
    pub fn begin(
        model: &Model,
        manipulator: &Manipulator,
        handle: Handle,
        ray: Ray,
    ) -> Option<Self> {
        let FeatureKind::Move(start) = &model.document().feature(manipulator.feature)?.kind else {
            return None;
        };
        let parameters = model.parameters();
        let from: Option<Vec<f64>> = MoveAxis::ALL
            .into_iter()
            .map(|axis| {
                axis.of(&start.offset)
                    .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
                    .ok()
            })
            .collect();
        let from = <[f64; 3]>::try_from(from?).ok()?;
        let grabbed = point_on(handle, manipulator.origin, ray)?;
        Some(Self {
            feature: manipulator.feature,
            handle,
            start: start.clone(),
            from,
            origin: manipulator.origin,
            grabbed,
            step: manipulator.step(),
            offset: from,
        })
    }

    pub fn follow(&mut self, ray: Ray, free: bool) -> bool {
        let Some(at) = point_on(self.handle, self.origin, ray) else {
            return false;
        };
        let moved = at - self.grabbed;
        let mut offset = self.from;
        for axis in self.handle.moves() {
            let along = moved.dot(axis.direction());
            let rounded = if free {
                along
            } else {
                (along / self.step).round() * self.step
            };
            if let Some(value) = offset.get_mut(axis.index()) {
                *value += rounded;
            }
        }
        let changed = offset != self.offset;
        self.offset = offset;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.offset != self.from
    }

    pub fn movement(&self, unit: LengthUnit) -> Move {
        let mut movement = self.start.clone();
        for axis in self.handle.moves() {
            if let Some(value) = self.offset.get(axis.index()) {
                *axis.of_mut(&mut movement.offset) = unit.measured(*value);
            }
        }
        movement
    }

    pub fn transaction(&self, model: &Model) -> Option<Transaction> {
        move_tools::edit(
            model.document(),
            self.feature,
            self.movement(model.length_unit()),
        )
    }

    pub fn readout(&self, units: Units) -> String {
        self.handle
            .moves()
            .into_iter()
            .filter_map(|axis| {
                let index = axis.index();
                let moved = self.offset.get(index)? - self.from.get(index)?;
                let sign = if moved >= 0.0 { "+" } else { "" };
                Some(format!(
                    "{} {sign}{}",
                    axis.name(),
                    units.readout_text(moved)
                ))
            })
            .collect::<Vec<_>>()
            .join("   ")
    }
}

fn point_on(handle: Handle, origin: Point3, ray: Ray) -> Option<Point3> {
    match handle {
        Handle::Along(axis) => {
            let along = ray.closest_along_line(origin, axis.direction())?;
            Some(origin + axis.direction() * along)
        }
        Handle::Across(normal) => {
            let plane = Plane::new(origin, normal.direction())?;
            Some(ray.at(ray.intersect_plane(&plane)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use caditor_render::Viewpoint;

    use super::*;

    fn looking_down_at_the_origin() -> View {
        let viewpoint =
            Viewpoint::looking_from(Vector3::new(1.0, -1.0, 1.0), Point3::ZERO, 200.0).unwrap();
        View::new(viewpoint, 800.0, 600.0)
    }

    fn manipulator(view: &View) -> Manipulator {
        let depth = view.view_depth(Point3::ZERO);
        Manipulator {
            feature: FeatureId::from_raw(3),
            origin: Point3::ZERO,
            reach: ARROW_POINTS * view.units_per_pixel_at(depth),
            forward: view.forward(),
        }
    }

    #[test]
    fn the_arrows_and_squares_are_hit_where_they_are_drawn() {
        let view = looking_down_at_the_origin();
        let manipulator = manipulator(&view);

        for axis in MoveAxis::ALL {
            let (from, tip) = manipulator.arrow(axis).unwrap();
            let middle = view.project(from.lerp(tip, 0.5)).unwrap();
            assert_eq!(
                manipulator.hit(&view, middle, 1.0),
                Some(Handle::Along(axis))
            );
        }
        let square = manipulator.square(MoveAxis::Z).unwrap();
        let centre = view.project(square[0].lerp(square[2], 0.5)).unwrap();
        assert_eq!(
            manipulator.hit(&view, centre, 1.0),
            Some(Handle::Across(MoveAxis::Z))
        );
        let away = view.project(Point3::new(-200.0, 200.0, 0.0)).unwrap();
        assert_eq!(manipulator.hit(&view, away, 1.0), None);
    }

    #[test]
    fn an_arrow_pointing_at_the_eye_is_left_out() {
        let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
        let view = View::new(viewpoint, 800.0, 600.0);
        let manipulator = manipulator(&view);

        assert_eq!(manipulator.arrow(MoveAxis::Z), None);
        assert!(manipulator.arrow(MoveAxis::X).is_some());
        assert!(manipulator.square(MoveAxis::Z).is_some());
        assert_eq!(manipulator.square(MoveAxis::X), None);
    }

    #[test]
    fn steps_are_one_two_or_five_of_a_power_of_ten() {
        assert_eq!(nice_step(0.3), 0.5);
        assert_eq!(nice_step(1.0), 1.0);
        assert_eq!(nice_step(1.2), 2.0);
        assert_eq!(nice_step(7.0), 10.0);
        assert!((nice_step(0.013) - 0.02).abs() < 1e-12);
    }

    #[test]
    fn a_point_on_a_handle_follows_the_ray_along_its_axis_or_plane() {
        let ray = Ray::new(Point3::new(7.0, 3.0, 50.0), Vector3::NEG_Z).unwrap();

        let along = point_on(Handle::Along(MoveAxis::X), Point3::ZERO, ray).unwrap();
        assert!(along.distance(Point3::new(7.0, 0.0, 0.0)) < 1e-9);
        let across = point_on(Handle::Across(MoveAxis::Z), Point3::ZERO, ray).unwrap();
        assert!(across.distance(Point3::new(7.0, 3.0, 0.0)) < 1e-9);
    }

    #[test]
    fn the_words_name_the_axis_or_plane() {
        assert_eq!(
            Handle::Along(MoveAxis::Y).words(),
            "Drag to move the body along Y"
        );
        assert_eq!(
            Handle::Across(MoveAxis::Y).words(),
            "Drag to move the body in the XZ plane"
        );
    }
}
