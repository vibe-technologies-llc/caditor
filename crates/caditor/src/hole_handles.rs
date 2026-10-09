use caditor_document::{Edit, FeatureId, Hole, Transaction};
use caditor_geometry::{Plane, Point2, Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, Fill, Layer, View};
use caditor_sketch::Entity;

use crate::{
    hole_panel::POSITION_CAPTIONS,
    hole_tools,
    model::Model,
    move_manipulator::{
        ABOUT_AXIS, ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HIGHLIGHTED, HIT_POINTS, Handle,
        HoleGrip, segment_distance, step_for, within,
    },
    scene,
    units::Units,
};

const HOLE_ARROW_SHARE: f64 = 0.7;
const SQUARE_HALF_POINTS: f64 = 9.0;
const SQUARE_ALPHA: f32 = 0.55;
const EDGE_ON: f64 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoleHandles {
    pub feature: FeatureId,
    plane: Plane,
    at: Point2,
    per_point: f64,
    forward: Vector3,
}

fn open_hole(model: &Model, feature: FeatureId) -> Option<&Hole> {
    model.document().feature(feature)?.kind.hole()
}

fn previewed_point(model: &Model, feature: FeatureId, hole: &Hole) -> Option<Point2> {
    model
        .draft_transaction(feature)?
        .edits()
        .iter()
        .find_map(|edit| match edit {
            Edit::SetSketchEntity {
                feature: sketch,
                entity: Entity::Point(at),
                ..
            } if *sketch == hole.sketch => Some(*at),
            _ => None,
        })
}

impl HoleGrip {
    fn direction(self, plane: &Plane) -> Option<Vector3> {
        match self {
            Self::AlongX => Some(plane.x_axis()),
            Self::AlongY => Some(plane.y_axis()),
            Self::OnFace => None,
        }
    }
}

impl HoleHandles {
    pub fn of(
        model: &Model,
        feature: FeatureId,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let document = model.document();
        let hole = open_hole(model, feature)?;
        let lone = hole_tools::lone_point(document, hole)?;
        let plane = scene::sketch_plane(document, model.evaluation(), hole.sketch)?;
        let at = previewed_point(model, feature, hole).unwrap_or(lone.at);
        let centre = plane.to_world(at);
        let depth = view.view_depth(centre);
        let per_point = pixels_per_point * view.units_per_pixel_at(depth);
        let forward = view.forward();
        let shown = depth.is_finite() && depth > 0.0 && per_point.is_finite() && per_point > 0.0;
        shown.then_some(Self {
            feature,
            plane,
            at,
            per_point,
            forward,
        })
    }

    fn centre(&self) -> Point3 {
        self.plane.to_world(self.at)
    }

    fn arrow(&self, grip: HoleGrip) -> Option<(Point3, Point3, Vector3)> {
        let direction = grip.direction(&self.plane)?;
        (direction.dot(self.forward).abs() < END_ON).then(|| {
            let centre = self.centre();
            (
                centre + direction * GAP_POINTS * self.per_point,
                centre + direction * ARROW_POINTS * HOLE_ARROW_SHARE * self.per_point,
                direction,
            )
        })
    }

    fn square(&self) -> Option<[Point3; 4]> {
        if self.plane.normal().dot(self.forward).abs() < EDGE_ON {
            return None;
        }
        let half = SQUARE_HALF_POINTS * self.per_point;
        let (x, y) = (self.plane.x_axis() * half, self.plane.y_axis() * half);
        let centre = self.centre();
        Some([
            centre - x - y,
            centre + x - y,
            centre + x + y,
            centre - x + y,
        ])
    }

    pub fn step(&self) -> f64 {
        step_for(self.per_point)
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let reach = HIT_POINTS * pixels_per_point;
        let arrow = [HoleGrip::AlongX, HoleGrip::AlongY]
            .into_iter()
            .filter_map(|grip| {
                let (from, to, _) = self.arrow(grip)?;
                let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
                Some((distance, Handle::Hole(grip)))
            })
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, handle)| handle);
        let inside = self.square().is_some_and(|corners| {
            let projected: Option<Vec<Vector2>> =
                corners.iter().map(|corner| view.project(*corner)).collect();
            projected.is_some_and(|projected| within(&projected, cursor))
        });
        arrow.or_else(|| inside.then_some(Handle::Hole(HoleGrip::OnFace)))
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<Point3> {
        let Handle::Hole(grip) = handle else {
            return None;
        };
        match self.arrow(grip) {
            Some((from, tip, direction)) => Some(from.lerp(tip, 0.6) + direction * along),
            None => Some(self.centre() + self.plane.x_axis() * along),
        }
    }

    pub fn add_to(&self, batch: &mut Batch, highlighted: Option<Handle>) {
        let colour = |grip: HoleGrip| {
            if highlighted == Some(Handle::Hole(grip)) {
                HIGHLIGHTED
            } else {
                ABOUT_AXIS
            }
        };
        if let Some(corners) = self.square() {
            batch.fills.push(Fill::convex(
                &corners,
                colour(HoleGrip::OnFace).with_alpha(SQUARE_ALPHA),
                Layer::Front,
                None,
            ));
        }
        for grip in [HoleGrip::AlongX, HoleGrip::AlongY] {
            let Some((from, tip, direction)) = self.arrow(grip) else {
                continue;
            };
            Arrow {
                from,
                tip,
                direction,
                per_point: self.per_point,
                forward: self.forward,
            }
            .add_to(batch, colour(grip));
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HoleDrag {
    hole: Hole,
    grip: HoleGrip,
    plane: Plane,
    grabbed: Point2,
    step: f64,
    from: Point2,
    at: Point2,
}

impl HoleDrag {
    pub fn begin(model: &Model, handles: &HoleHandles, grip: HoleGrip, ray: Ray) -> Option<Self> {
        let hole = open_hole(model, handles.feature)?;
        let from = hole_tools::lone_point(model.document(), hole)?.at;
        let plane = handles.plane;
        let grabbed = plane.to_local(ray.at(ray.intersect_plane(&plane)?));
        Some(Self {
            hole: hole.clone(),
            grip,
            plane,
            grabbed,
            step: handles.step(),
            from,
            at: from,
        })
    }

    pub fn follow(&mut self, ray: Ray, free: bool) -> bool {
        let Some(at) = ray
            .intersect_plane(&self.plane)
            .map(|distance| self.plane.to_local(ray.at(distance)))
        else {
            return false;
        };
        let pulled = at - self.grabbed;
        let pulled = match self.grip {
            HoleGrip::AlongX => Vector2::new(pulled.x, 0.0),
            HoleGrip::AlongY => Vector2::new(0.0, pulled.y),
            HoleGrip::OnFace => pulled,
        };
        let wanted = self.from + pulled;
        let placed = if free {
            wanted
        } else {
            let snapped = (wanted / self.step).round() * self.step;
            match self.grip {
                HoleGrip::AlongX => Point2::new(snapped.x, self.from.y),
                HoleGrip::AlongY => Point2::new(self.from.x, snapped.y),
                HoleGrip::OnFace => snapped,
            }
        };
        let changed = placed != self.at;
        self.at = placed;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.at != self.from
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        hole_tools::moved(model.document(), feature, &self.hole, self.at).ok()
    }

    pub fn readout(&self, units: Units) -> String {
        let [x, y] = POSITION_CAPTIONS;
        format!(
            "{x} {}, {y} {}",
            units.readout_text(self.at.x),
            units.readout_text(self.at.y)
        )
    }
}
