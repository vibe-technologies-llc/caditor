use std::{cmp::Ordering, collections::BTreeSet};

use caditor_geometry::Point2;

use crate::{
    constraint::Constraint,
    curve::ArcGeometry,
    entity::Entity,
    id::EntityId,
    inference::{Grid, Tolerance, within_sweep},
    open_ends::{Classes, curve_ends},
    sketch::Sketch,
};

const OVERLAP_SAMPLES: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub enum Flaw {
    NearlyJoined {
        first: EntityId,
        second: EntityId,
        gap: f64,
    },
    LiesOn {
        curve: EntityId,
        on: EntityId,
    },
    Overlaps {
        curve: EntityId,
        other: EntityId,
    },
    NoLength {
        curve: EntityId,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fix {
    pub remove: Vec<EntityId>,
    pub add: Vec<Constraint>,
}

impl Fix {
    pub fn is_empty(&self) -> bool {
        self.remove.is_empty() && self.add.is_empty()
    }
}

impl Sketch {
    pub fn flaws(&self, tolerance: Tolerance) -> Vec<Flaw> {
        let mut classes = self.coincident_classes();
        let mut flaws: Vec<Flaw> = self
            .curve_ids()
            .into_iter()
            .filter(|curve| self.has_no_length(*curve, tolerance))
            .map(|curve| Flaw::NoLength { curve })
            .collect();
        let short: BTreeSet<EntityId> = flaws
            .iter()
            .filter_map(|flaw| match flaw {
                Flaw::NoLength { curve } => Some(*curve),
                _ => None,
            })
            .collect();
        for curve in &short {
            if let [start, end] = self.entity(*curve).map(curve_ends).unwrap_or_default()[..] {
                classes.join(start, end);
            }
        }
        flaws.extend(self.nearly_joined(&classes, &short, tolerance));
        flaws.extend(self.overlaps(&short, tolerance));
        flaws
    }

    pub fn fix(&self, flaw: &Flaw) -> Fix {
        let classes = self.coincident_classes();
        match *flaw {
            Flaw::NearlyJoined { first, second, .. } => Fix {
                remove: Vec::new(),
                add: vec![Constraint::Coincident(first, second)],
            },
            Flaw::NoLength { curve } => {
                let mut fix = self.removal(curve);
                let ends: Vec<EntityId> = self
                    .entity(curve)
                    .map(curve_ends)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|end| !fix.remove.contains(end))
                    .collect();
                if let [start, end] = ends.as_slice()
                    && classes.root(*start) != classes.root(*end)
                {
                    fix.add.push(Constraint::Coincident(*start, *end));
                }
                fix
            }
            Flaw::LiesOn { curve, on } => {
                let mut fix = self.removal(curve);
                let on_ends: Vec<EntityId> = self
                    .entity(on)
                    .map(curve_ends)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|end| classes.root(end))
                    .collect();
                let kept_ends: Vec<EntityId> = self
                    .entity(curve)
                    .map(curve_ends)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|end| !fix.remove.contains(end))
                    .filter(|end| !on_ends.contains(&classes.root(*end)))
                    .collect();
                for end in kept_ends {
                    let held = Constraint::Coincident(end, on);
                    if self.check_constraint(&held).is_ok() {
                        fix.add.push(held);
                    }
                }
                fix
            }
            Flaw::Overlaps { .. } => Fix::default(),
        }
    }

    fn coincident_classes(&self) -> Classes {
        let mut classes = Classes::default();
        for (_, constraint) in self.active_constraints() {
            if let Constraint::Coincident(a, b) = *constraint
                && self.point(a).is_some()
                && self.point(b).is_some()
            {
                classes.join(a, b);
            }
        }
        classes
    }

    fn curve_ids(&self) -> Vec<EntityId> {
        self.entities()
            .filter(|(id, entity)| {
                !id.is_reference()
                    && !self.is_projected(*id)
                    && matches!(
                        entity,
                        Entity::Line { .. }
                            | Entity::Circle { .. }
                            | Entity::Arc { .. }
                            | Entity::Spline { .. }
                    )
            })
            .map(|(id, _)| id)
            .collect()
    }

    fn has_no_length(&self, curve: EntityId, tolerance: Tolerance) -> bool {
        let tiny = tolerance.distance;
        match self.entity(curve) {
            Some(Entity::Line { .. }) => self
                .line_endpoints(curve)
                .is_some_and(|(start, end)| start.distance(end) <= tiny),
            Some(Entity::Circle { radius, .. }) => *radius <= tiny,
            Some(Entity::Arc { .. }) => self
                .arc(curve)
                .is_some_and(|arc| arc.radius <= tiny || arc.radius * arc.sweep <= tiny),
            Some(Entity::Spline { points, .. }) => {
                let points: Vec<Point2> = points
                    .iter()
                    .filter_map(|point| self.point(*point))
                    .collect();
                points
                    .first()
                    .is_some_and(|first| points.iter().all(|point| point.distance(*first) <= tiny))
            }
            _ => false,
        }
    }

    fn removal(&self, curve: EntityId) -> Fix {
        let mut remove = vec![curve];
        for point in self.entity(curve).map(Entity::points).unwrap_or_default() {
            let used_elsewhere = self.entities_using(point).iter().any(|user| *user != curve)
                || !self.constraints_using(point).is_empty();
            if !used_elsewhere && !remove.contains(&point) {
                remove.push(point);
            }
        }
        Fix {
            remove,
            add: Vec::new(),
        }
    }

    fn nearly_joined(
        &self,
        classes: &Classes,
        short: &BTreeSet<EntityId>,
        tolerance: Tolerance,
    ) -> Vec<Flaw> {
        let mut ends: Vec<(EntityId, EntityId)> = Vec::new();
        for curve in self.curve_ids() {
            if short.contains(&curve) {
                continue;
            }
            for end in self.entity(curve).map(curve_ends).unwrap_or_default() {
                ends.push((end, curve));
            }
        }
        let located: Vec<(EntityId, Point2)> = ends
            .iter()
            .filter_map(|&(end, _)| Some((end, self.point(end)?)))
            .collect();
        let grid = Grid::new(&located, tolerance.distance);
        let mut flagged = BTreeSet::new();
        let mut flaws = Vec::new();
        for (index, &(end, at)) in located.iter().enumerate() {
            for other in grid.near(at) {
                let (Some(&(neighbour, there)), Some(&(_, curve)), Some(&(_, other_curve))) =
                    (located.get(other), ends.get(index), ends.get(other))
                else {
                    continue;
                };
                let pair = (classes.root(end), classes.root(neighbour));
                let gap = at.distance(there);
                if other <= index
                    || curve == other_curve
                    || pair.0 == pair.1
                    || gap > tolerance.distance
                    || !flagged.insert((pair.0.min(pair.1), pair.0.max(pair.1)))
                {
                    continue;
                }
                flaws.push(Flaw::NearlyJoined {
                    first: end,
                    second: neighbour,
                    gap,
                });
            }
        }
        flaws
    }

    fn overlaps(&self, short: &BTreeSet<EntityId>, tolerance: Tolerance) -> Vec<Flaw> {
        let mut boxes: Vec<(EntityId, Point2, Point2)> = self
            .curve_ids()
            .into_iter()
            .filter(|curve| !short.contains(curve))
            .filter_map(|curve| {
                let reach = self.reach(curve)?;
                Some((curve, reach.0, reach.1))
            })
            .collect();
        boxes.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        let mut flaws = Vec::new();
        for (index, &(first, low, high)) in boxes.iter().enumerate() {
            for &(second, other_low, other_high) in boxes.iter().skip(index + 1) {
                if other_low.x > high.x + tolerance.distance {
                    break;
                }
                if other_low.y > high.y + tolerance.distance
                    || other_high.y < low.y - tolerance.distance
                {
                    continue;
                }
                let (newer, older) = if first < second {
                    (second, first)
                } else {
                    (first, second)
                };
                let newer_on_older = self.share_on(newer, older, tolerance);
                let older_on_newer = self.share_on(older, newer, tolerance);
                let flaw = match (newer_on_older, older_on_newer) {
                    (Some(Share::Whole), _) => Flaw::LiesOn {
                        curve: newer,
                        on: older,
                    },
                    (_, Some(Share::Whole)) => Flaw::LiesOn {
                        curve: older,
                        on: newer,
                    },
                    (Some(Share::Part), _) | (_, Some(Share::Part)) => Flaw::Overlaps {
                        curve: newer,
                        other: older,
                    },
                    (None, None) => continue,
                };
                flaws.push(flaw);
            }
        }
        flaws
    }

    fn reach(&self, curve: EntityId) -> Option<(Point2, Point2)> {
        let (low, high) = match self.entity(curve)? {
            Entity::Line { .. } => {
                let (start, end) = self.line_endpoints(curve)?;
                (start.min(end), start.max(end))
            }
            Entity::Circle { .. } | Entity::Arc { .. } => {
                let (centre, radius) = self.circle(curve)?;
                (centre - radius, centre + radius)
            }
            _ => return None,
        };
        Some((low, high))
    }

    fn share_on(&self, curve: EntityId, on: EntityId, tolerance: Tolerance) -> Option<Share> {
        let samples = self.samples(curve)?;
        let on_count = samples
            .iter()
            .filter(|point| self.lies_on(on, **point, tolerance))
            .count();
        match on_count.cmp(&samples.len()) {
            Ordering::Equal => Some(Share::Whole),
            _ if on_count >= 2 => Some(Share::Part),
            _ => None,
        }
    }

    fn samples(&self, curve: EntityId) -> Option<Vec<Point2>> {
        let fractions =
            (0..OVERLAP_SAMPLES).map(|step| (step as f64 + 0.5) / OVERLAP_SAMPLES as f64);
        match self.entity(curve)? {
            Entity::Line { .. } => {
                let (start, end) = self.line_endpoints(curve)?;
                Some(fractions.map(|share| start.lerp(end, share)).collect())
            }
            Entity::Circle { .. } => {
                let (centre, radius) = self.circle(curve)?;
                let circle = ArcGeometry::full_circle(centre, radius);
                Some(
                    fractions
                        .map(|share| circle.point_at(share * circle.sweep))
                        .collect(),
                )
            }
            Entity::Arc { .. } => {
                let arc = self.arc(curve)?;
                Some(
                    fractions
                        .map(|share| arc.point_at(arc.start_angle + share * arc.sweep))
                        .collect(),
                )
            }
            _ => None,
        }
    }

    fn lies_on(&self, curve: EntityId, point: Point2, tolerance: Tolerance) -> bool {
        let slack = tolerance.distance;
        match self.entity(curve) {
            Some(Entity::Line { .. }) => self.line_endpoints(curve).is_some_and(|(start, end)| {
                let span = end - start;
                let length = span.length();
                if length <= slack {
                    return false;
                }
                let along = span / length;
                let reach = along.dot(point - start);
                along.perp_dot(point - start).abs() <= slack
                    && reach >= -slack
                    && reach <= length + slack
            }),
            Some(Entity::Circle { .. }) => self
                .circle(curve)
                .is_some_and(|(centre, radius)| (point.distance(centre) - radius).abs() <= slack),
            Some(Entity::Arc { .. }) => self.arc(curve).is_some_and(|arc| {
                (point.distance(arc.center) - arc.radius).abs() <= slack
                    && within_sweep(&arc, point, slack)
            }),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Share {
    Whole,
    Part,
}

#[cfg(test)]
mod tests;
