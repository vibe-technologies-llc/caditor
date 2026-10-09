use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::{FRAC_PI_2, PI, TAU},
};

use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::ArcGeometry,
    entity::{Entity, Role},
    id::{ConstraintId, EntityId},
    open_ends::{Classes, curve_ends},
    relation::Relations,
    sketch::{Sketch, SketchError},
    solve::Solved,
};

pub const RELATIVE_DISTANCE: f64 = 1e-3;
pub const ANGLE_DEGREES: f64 = 1.0;
const SMALLEST_DISTANCE: f64 = 1e-6;
const MAX_ROUNDS: usize = 64;
const MOVE_LIMIT: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    pub distance: f64,
    pub angle: f64,
}

impl Tolerance {
    pub fn of(sketch: &Sketch) -> Self {
        Self::relative(sketch, RELATIVE_DISTANCE, ANGLE_DEGREES)
    }

    pub fn relative(sketch: &Sketch, share_of_extent: f64, angle_degrees: f64) -> Self {
        Self {
            distance: (sketch.extent() * share_of_extent).max(SMALLEST_DISTANCE),
            angle: angle_degrees.to_radians(),
        }
    }

    fn parallel(self, a: Vector2, b: Vector2) -> bool {
        let lengths = a.length() * b.length();
        lengths > 0.0 && a.perp_dot(b).abs() <= self.angle.sin() * lengths
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationKind {
    Coincident,
    Concentric,
    Symmetric,
    Horizontal,
    Vertical,
    Tangent,
    Perpendicular,
    Parallel,
    Equal,
}

impl RelationKind {
    pub const ALL: [Self; 9] = [
        Self::Coincident,
        Self::Concentric,
        Self::Symmetric,
        Self::Horizontal,
        Self::Vertical,
        Self::Tangent,
        Self::Perpendicular,
        Self::Parallel,
        Self::Equal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Coincident => "Coincident ends",
            Self::Concentric => "Concentric circles and arcs",
            Self::Symmetric => "Symmetric about an axis",
            Self::Horizontal => "Horizontal lines",
            Self::Vertical => "Vertical lines",
            Self::Tangent => "Tangent curves",
            Self::Perpendicular => "Perpendicular lines",
            Self::Parallel => "Parallel lines",
            Self::Equal => "Equal lengths and radii",
        }
    }

    pub fn of(constraint: &Constraint) -> Option<Self> {
        Some(match constraint {
            Constraint::Coincident(..) => Self::Coincident,
            Constraint::Concentric(..) => Self::Concentric,
            Constraint::Symmetric { .. } => Self::Symmetric,
            Constraint::Horizontal(_) | Constraint::HorizontalPoints(..) => Self::Horizontal,
            Constraint::Vertical(_) | Constraint::VerticalPoints(..) => Self::Vertical,
            Constraint::Tangent(..) => Self::Tangent,
            Constraint::Perpendicular(..) => Self::Perpendicular,
            Constraint::Parallel(..) => Self::Parallel,
            Constraint::Equal(..) => Self::Equal,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    pub constraints: Vec<Constraint>,
    pub degrees_of_freedom: usize,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InferenceError {
    #[error("the sketch has to solve as it stands first: {0}")]
    Unsolved(SketchError),
    #[error("it was cancelled")]
    Cancelled,
    #[error("{label} is not a point")]
    NotAPoint { entity: EntityId, label: String },
}

impl Sketch {
    pub fn extent(&self) -> f64 {
        let mut low = Point2::splat(f64::INFINITY);
        let mut high = Point2::splat(f64::NEG_INFINITY);
        for (_, entity) in self.entities() {
            match *entity {
                Entity::Point(at) => {
                    low = low.min(at);
                    high = high.max(at);
                }
                Entity::Circle { center, radius } => {
                    if let Some(at) = self.point(center) {
                        low = low.min(at - Vector2::splat(radius));
                        high = high.max(at + Vector2::splat(radius));
                    }
                }
                _ => {}
            }
        }
        let diagonal = high.distance(low);
        if diagonal.is_finite() { diagonal } else { 0.0 }
    }

    pub fn shown_relations(
        &self,
        tolerance: Tolerance,
        kinds: &BTreeSet<RelationKind>,
    ) -> Vec<Constraint> {
        let mut finder = Finder::new(self, tolerance);
        if kinds.contains(&RelationKind::Coincident) {
            finder.coincident_ends();
        }
        if kinds.contains(&RelationKind::Concentric) {
            finder.concentric();
        }
        if kinds.contains(&RelationKind::Symmetric) {
            finder.symmetric();
        }
        finder.level_and_upright(
            kinds.contains(&RelationKind::Horizontal),
            kinds.contains(&RelationKind::Vertical),
        );
        if kinds.contains(&RelationKind::Tangent) {
            finder.tangent();
        }
        finder.directions(
            kinds.contains(&RelationKind::Perpendicular),
            kinds.contains(&RelationKind::Parallel),
        );
        if kinds.contains(&RelationKind::Equal) {
            finder.equal();
        }
        finder.found
    }

    pub fn inferred_relations<F>(
        &self,
        tolerance: Tolerance,
        kinds: &BTreeSet<RelationKind>,
        value_of: &F,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Kept, InferenceError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let found = self.shown_relations(tolerance, kinds);
        let stages: Vec<Vec<Constraint>> = found
            .chunk_by(|a, b| RelationKind::of(a) == RelationKind::of(b))
            .map(<[Constraint]>::to_vec)
            .collect();
        self.keep_holding(stages, tolerance, value_of, cancelled)
    }

    pub(crate) fn keep_holding<F>(
        &self,
        stages: Vec<Vec<Constraint>>,
        tolerance: Tolerance,
        value_of: &F,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Kept, InferenceError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let mut holding = Holding {
            value_of,
            cancelled,
            limit: tolerance.distance * MOVE_LIMIT,
            state: self
                .solve(value_of, cancelled)
                .map_err(|error| match error {
                    SketchError::Cancelled => InferenceError::Cancelled,
                    error => InferenceError::Unsolved(error),
                })?,
            kept: Vec::new(),
        };
        for stage in stages {
            if !holding.all_at_once(&stage)? {
                for candidate in stage {
                    holding.one(candidate)?;
                }
            }
        }
        Ok(Kept {
            constraints: holding.kept,
            degrees_of_freedom: holding.state.solution.degrees_of_freedom(),
        })
    }
}

struct Holding<'a, F> {
    value_of: &'a F,
    cancelled: &'a dyn Fn() -> bool,
    limit: f64,
    state: Solved,
    kept: Vec<Constraint>,
}

impl<F> Holding<'_, F>
where
    F: Fn(ParameterId) -> Result<Quantity, EvalError>,
{
    fn solve(&self, sketch: &Sketch) -> Result<Solved, SketchError> {
        sketch.solve_from(self.value_of, self.cancelled, &[], Some(&self.state.memo))
    }

    fn all_at_once(&mut self, stage: &[Constraint]) -> Result<bool, InferenceError> {
        let mut working = self.state.geometry.clone();
        let mut added = BTreeMap::new();
        for candidate in stage {
            if let Ok(id) = working.add_constraint(candidate.clone()) {
                added.insert(id, candidate.clone());
            }
        }
        if added.is_empty() {
            return Ok(true);
        }
        for _ in 0..MAX_ROUNDS {
            match self.solve(&working) {
                Ok(mut solved) => {
                    if !moved_at_most(&self.state.geometry, &solved.geometry, self.limit) {
                        return Ok(false);
                    }
                    for redundancy in solved.solution.redundancies() {
                        if added.remove(&redundancy.constraint).is_some()
                            && solved
                                .geometry
                                .remove_constraint(redundancy.constraint)
                                .is_err()
                        {
                            return Ok(false);
                        }
                    }
                    self.kept.extend(added.into_values());
                    self.state = solved;
                    return Ok(true);
                }
                Err(SketchError::Cancelled) => return Err(InferenceError::Cancelled),
                Err(error) => {
                    let mut culprits = BTreeSet::new();
                    culprits_of(&error, &working, &added, &mut culprits);
                    if culprits.is_empty() {
                        return Ok(false);
                    }
                    for culprit in culprits {
                        added.remove(&culprit);
                        if working.remove_constraint(culprit).is_err() {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    fn one(&mut self, candidate: Constraint) -> Result<(), InferenceError> {
        let mut working = self.state.geometry.clone();
        let Ok(id) = working.add_constraint(candidate.clone()) else {
            return Ok(());
        };
        match self.solve(&working) {
            Ok(solved)
                if solved.solution.redundancy(id).is_none()
                    && moved_at_most(&self.state.geometry, &solved.geometry, self.limit) =>
            {
                self.kept.push(candidate);
                self.state = solved;
                Ok(())
            }
            Err(SketchError::Cancelled) => Err(InferenceError::Cancelled),
            Ok(_) | Err(_) => Ok(()),
        }
    }
}

fn moved_at_most(before: &Sketch, after: &Sketch, limit: f64) -> bool {
    before.entities().all(|(id, entity)| match *entity {
        Entity::Point(at) => after
            .point(id)
            .is_some_and(|moved| moved.distance(at) <= limit),
        Entity::Circle { radius, .. } => after
            .circle(id)
            .is_some_and(|(_, moved)| (moved - radius).abs() <= limit),
        _ => true,
    })
}

fn culprits_of(
    error: &SketchError,
    sketch: &Sketch,
    added: &BTreeMap<ConstraintId, Constraint>,
    culprits: &mut BTreeSet<ConstraintId>,
) {
    let newest_touching = |touched: &BTreeSet<EntityId>| {
        added
            .iter()
            .rev()
            .find(|(_, constraint)| {
                constraint
                    .entities()
                    .iter()
                    .any(|entity| touched.contains(entity))
            })
            .map(|(id, _)| *id)
    };
    let with_points = |entities: &[EntityId]| -> BTreeSet<EntityId> {
        entities
            .iter()
            .flat_map(|entity| {
                let mut touched = sketch
                    .entity(*entity)
                    .map(Entity::points)
                    .unwrap_or_default();
                touched.push(*entity);
                touched
            })
            .collect()
    };
    let found = match error {
        SketchError::Conflict { constraints } => constraints
            .iter()
            .rev()
            .find(|constraint| added.contains_key(constraint))
            .copied(),
        SketchError::Unsolvable { entities, newest } => newest
            .filter(|newest| added.contains_key(newest))
            .or_else(|| newest_touching(&with_points(entities))),
        SketchError::NoLength { entity, .. } => newest_touching(&with_points(&[*entity])),
        SketchError::Several(errors) => {
            for error in errors {
                culprits_of(error, sketch, added, culprits);
            }
            None
        }
        _ => None,
    };
    culprits.extend(found);
}

struct Finder<'a> {
    sketch: &'a Sketch,
    tolerance: Tolerance,
    classes: Classes,
    relations: Relations,
    straight: BTreeSet<EntityId>,
    found: Vec<Constraint>,
}

impl<'a> Finder<'a> {
    fn new(sketch: &'a Sketch, tolerance: Tolerance) -> Self {
        let mut classes = Classes::default();
        let mut straight = BTreeSet::new();
        for (_, constraint) in sketch.active_constraints() {
            match *constraint {
                Constraint::Coincident(a, b)
                    if sketch.point(a).is_some() && sketch.point(b).is_some() =>
                {
                    classes.join(a, b);
                }
                Constraint::Horizontal(line) | Constraint::Vertical(line) => {
                    straight.insert(line);
                }
                _ => {}
            }
        }
        Self {
            sketch,
            tolerance,
            classes,
            relations: sketch.relations(),
            straight,
            found: Vec::new(),
        }
    }

    fn offer(&mut self, constraint: Constraint) -> bool {
        let sketch = self.sketch;
        if sketch.check_constraint(&constraint).is_err()
            || self.relations.restating(sketch, &constraint).is_some()
            || self.relations.contradicting(&constraint).is_some()
        {
            return false;
        }
        let id = ConstraintId::from_raw(u64::MAX - self.found.len() as u64);
        self.relations.record(sketch, id, &constraint);
        self.found.push(constraint);
        true
    }

    fn joined(&self, a: EntityId, b: EntityId) -> bool {
        self.classes.root(a) == self.classes.root(b)
    }

    fn curves(&self, role: Role) -> Vec<EntityId> {
        self.sketch
            .entities()
            .filter(|(id, entity)| !id.is_reference() && entity.role() == role)
            .map(|(id, _)| id)
            .collect()
    }

    fn coincident_ends(&mut self) {
        let mut owners: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        for (curve, entity) in self.sketch.entities() {
            if !curve.is_reference() {
                for end in curve_ends(entity) {
                    owners.entry(end).or_default().push(curve);
                }
            }
        }
        let mut points: BTreeSet<EntityId> = owners.keys().copied().collect();
        points.extend(self.sketch.free_points());
        points.insert(EntityId::ORIGIN);
        let located = self.located(points);
        let grid = Grid::new(&located, self.tolerance.distance);
        for (index, &(point, at)) in located.iter().enumerate() {
            for other in grid.near(at) {
                let Some(&(neighbour, there)) = located.get(other) else {
                    continue;
                };
                let same_curve = owners.get(&point).is_some_and(|curves| {
                    owners
                        .get(&neighbour)
                        .is_some_and(|others| curves.iter().any(|curve| others.contains(curve)))
                });
                if other <= index
                    || same_curve
                    || self.joined(point, neighbour)
                    || at.distance(there) > self.tolerance.distance
                {
                    continue;
                }
                if self.offer(Constraint::Coincident(point, neighbour)) {
                    self.classes.join(point, neighbour);
                }
            }
        }
    }

    fn concentric(&mut self) {
        let curves = self.curves(Role::Circular);
        let centres: Vec<(EntityId, EntityId, Point2)> = curves
            .into_iter()
            .filter_map(|curve| {
                let centre = self.sketch.center_of(curve)?;
                Some((curve, centre, self.sketch.point(centre)?))
            })
            .collect();
        let located: Vec<(EntityId, Point2)> =
            centres.iter().map(|&(curve, _, at)| (curve, at)).collect();
        let grid = Grid::new(&located, self.tolerance.distance);
        for (index, &(curve, centre, at)) in centres.iter().enumerate() {
            for other in grid.near(at) {
                let Some(&(neighbour, other_centre, there)) = centres.get(other) else {
                    continue;
                };
                if other <= index
                    || self.joined(centre, other_centre)
                    || at.distance(there) > self.tolerance.distance
                {
                    continue;
                }
                if self.offer(Constraint::Concentric(curve, neighbour)) {
                    self.classes.join(centre, other_centre);
                }
            }
        }
    }

    fn symmetric(&mut self) {
        let mut axes = vec![
            (
                EntityId::HORIZONTAL_AXIS,
                Point2::ZERO,
                Vector2::X,
                Vec::new(),
            ),
            (
                EntityId::VERTICAL_AXIS,
                Point2::ZERO,
                Vector2::Y,
                Vec::new(),
            ),
        ];
        for line in self.curves(Role::Line) {
            if let Some((start, end)) = self.sketch.line_endpoints(line)
                && self.sketch.is_construction(line)
                && start.distance(end) > self.tolerance.distance
            {
                let own = self
                    .sketch
                    .entity(line)
                    .map(Entity::points)
                    .unwrap_or_default();
                axes.push((line, start, (end - start).normalize(), own));
            }
        }
        let arc_centres: BTreeSet<EntityId> = self
            .sketch
            .entities()
            .filter_map(|(_, entity)| match *entity {
                Entity::Arc { center, .. } => Some(center),
                _ => None,
            })
            .collect();
        let points: BTreeSet<EntityId> = self
            .sketch
            .entities()
            .filter(|(id, entity)| {
                !id.is_reference()
                    && !arc_centres.contains(id)
                    && matches!(entity, Entity::Point(_))
            })
            .map(|(id, _)| id)
            .collect();
        let located = self.located(points);
        let grid = Grid::new(&located, self.tolerance.distance);
        for (axis, through, along, own) in axes {
            let mut paired = BTreeSet::new();
            for &(point, at) in &located {
                let offset = at - through;
                let across = along.perp_dot(offset);
                let root = self.classes.root(point);
                if across.abs() <= self.tolerance.distance
                    || own.contains(&point)
                    || paired.contains(&root)
                {
                    continue;
                }
                let image = at - along.perp() * (2.0 * across);
                let partner = grid
                    .near(image)
                    .into_iter()
                    .filter_map(|other| located.get(other).copied())
                    .filter(|&(other, there)| {
                        let other_root = self.classes.root(other);
                        other_root != root
                            && !paired.contains(&other_root)
                            && !own.contains(&other)
                            && there.distance(image) <= self.tolerance.distance
                    })
                    .min_by(|a, b| a.1.distance(image).total_cmp(&b.1.distance(image)));
                let Some((other, _)) = partner else {
                    continue;
                };
                if self.offer(Constraint::Symmetric {
                    first: point,
                    second: other,
                    about: axis,
                }) {
                    paired.insert(root);
                    paired.insert(self.classes.root(other));
                }
            }
        }
    }

    fn level_and_upright(&mut self, horizontal: bool, vertical: bool) {
        for line in self.curves(Role::Line) {
            let Some((start, end)) = self.sketch.line_endpoints(line) else {
                continue;
            };
            let direction = end - start;
            if direction.length() <= self.tolerance.distance || self.straight.contains(&line) {
                continue;
            }
            let level = self.tolerance.parallel(direction, Vector2::X);
            let upright = self.tolerance.parallel(direction, Vector2::Y);
            let offered = (horizontal && level && self.offer(Constraint::Horizontal(line)))
                || (vertical && upright && self.offer(Constraint::Vertical(line)));
            if offered || level || upright {
                self.straight.insert(line);
            }
        }
    }

    fn tangent(&mut self) {
        let mut joints: BTreeMap<EntityId, Vec<(EntityId, EntityId)>> = BTreeMap::new();
        let lines = self.curves(Role::Line);
        let circular = self.curves(Role::Circular);
        for &curve in lines.iter().chain(&circular) {
            if let Some(entity) = self.sketch.entity(curve) {
                for end in curve_ends(entity) {
                    joints
                        .entry(self.classes.root(end))
                        .or_default()
                        .push((curve, end));
                }
            }
        }
        let mut touching = BTreeSet::new();
        for ends in joints.values() {
            for (index, &(first, first_end)) in ends.iter().enumerate() {
                for &(second, second_end) in ends.iter().skip(index + 1) {
                    if first == second || !touching.insert(ordered(first, second)) {
                        continue;
                    }
                    let directions = (
                        self.direction_at(first, first_end),
                        self.direction_at(second, second_end),
                    );
                    let same_centre = self.sketch.center_of(first).is_some()
                        && self.sketch.center_of(first) == self.sketch.center_of(second);
                    if let (Some(a), Some(b)) = directions
                        && !same_centre
                        && self.tolerance.parallel(a, b)
                    {
                        self.offer(Constraint::Tangent(first, second));
                    }
                }
            }
        }
        for &circle in &circular {
            for &line in &lines {
                if !touching.contains(&ordered(line, circle)) && self.line_touches(line, circle) {
                    self.offer(Constraint::Tangent(line, circle));
                }
            }
        }
        for (index, &first) in circular.iter().enumerate() {
            for &second in circular.iter().skip(index + 1) {
                if !touching.contains(&ordered(first, second)) && self.circles_touch(first, second)
                {
                    self.offer(Constraint::Tangent(first, second));
                }
            }
        }
    }

    fn direction_at(&self, curve: EntityId, end: EntityId) -> Option<Vector2> {
        match self.sketch.entity(curve)? {
            Entity::Line { .. } => self.sketch.line_direction(curve),
            Entity::Arc { center, .. } => {
                Some((self.sketch.point(end)? - self.sketch.point(*center)?).perp())
            }
            _ => None,
        }
    }

    fn on_drawn_circle(&self, curve: EntityId, at: Point2) -> bool {
        match self.sketch.entity(curve) {
            Some(Entity::Circle { .. }) => true,
            Some(Entity::Arc { .. }) => self
                .sketch
                .arc(curve)
                .is_some_and(|arc| within_sweep(&arc, at, self.tolerance.distance)),
            _ => false,
        }
    }

    fn line_touches(&self, line: EntityId, circle: EntityId) -> bool {
        let (Some((start, end)), Some((centre, radius))) =
            (self.sketch.line_endpoints(line), self.sketch.circle(circle))
        else {
            return false;
        };
        let span = end - start;
        let length = span.length();
        if length <= self.tolerance.distance || radius <= self.tolerance.distance {
            return false;
        }
        let along = span / length;
        let reach = along.dot(centre - start);
        let foot = start + along * reach;
        let slack = self.tolerance.distance;
        (centre.distance(foot) - radius).abs() <= slack
            && reach >= -slack
            && reach <= length + slack
            && self.on_drawn_circle(circle, foot)
    }

    fn circles_touch(&self, first: EntityId, second: EntityId) -> bool {
        let (Some((a, first_radius)), Some((b, second_radius))) =
            (self.sketch.circle(first), self.sketch.circle(second))
        else {
            return false;
        };
        let apart = a.distance(b);
        let slack = self.tolerance.distance;
        if apart <= slack {
            return false;
        }
        let towards = (b - a) / apart;
        let contact = if (apart - (first_radius + second_radius)).abs() <= slack {
            a + towards * first_radius
        } else if (apart - (first_radius - second_radius).abs()).abs() <= slack {
            if first_radius >= second_radius {
                a + towards * first_radius
            } else {
                b - towards * second_radius
            }
        } else {
            return false;
        };
        self.on_drawn_circle(first, contact) && self.on_drawn_circle(second, contact)
    }

    fn directions(&mut self, perpendicular: bool, parallel: bool) {
        let mut slanted: Vec<(f64, EntityId)> = self
            .curves(Role::Line)
            .into_iter()
            .filter(|line| !self.straight.contains(line))
            .filter_map(|line| {
                let direction = self.sketch.line_direction(line)?;
                (direction.length() > self.tolerance.distance)
                    .then(|| (direction.y.atan2(direction.x).rem_euclid(PI), line))
            })
            .collect();
        slanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let groups = anchored_groups(&slanted, self.tolerance.angle);
        if perpendicular {
            for (index, first) in groups.iter().enumerate() {
                for second in groups.iter().skip(index + 1) {
                    let (Some(&(first_angle, _)), Some(&(second_angle, _))) =
                        (first.first(), second.first())
                    else {
                        continue;
                    };
                    if ((second_angle - first_angle).abs() - FRAC_PI_2).abs() > self.tolerance.angle
                    {
                        continue;
                    }
                    if let Some((a, b)) = self.square_pair(first, second) {
                        self.offer(Constraint::Perpendicular(a, b));
                    }
                }
            }
        }
        if parallel {
            for group in &groups {
                if let Some((&(_, first), rest)) = group.split_first() {
                    for &(_, other) in rest {
                        self.offer(Constraint::Parallel(first, other));
                    }
                }
            }
        }
    }

    fn square_pair(
        &self,
        first: &[(f64, EntityId)],
        second: &[(f64, EntityId)],
    ) -> Option<(EntityId, EntityId)> {
        let ends = |line: EntityId| -> Vec<EntityId> {
            self.sketch
                .entity(line)
                .map(curve_ends)
                .unwrap_or_default()
                .into_iter()
                .map(|end| self.classes.root(end))
                .collect()
        };
        let joined = first.iter().find_map(|&(_, a)| {
            let a_ends = ends(a);
            second
                .iter()
                .find(|&&(_, b)| ends(b).iter().any(|end| a_ends.contains(end)))
                .map(|&(_, b)| (a, b))
        });
        joined.or_else(|| Some((first.first()?.1, second.first()?.1)))
    }

    fn equal(&mut self) {
        let mut lengths: Vec<(f64, EntityId)> = self
            .curves(Role::Line)
            .into_iter()
            .filter_map(|line| {
                let (start, end) = self.sketch.line_endpoints(line)?;
                Some((start.distance(end), line))
            })
            .filter(|(length, _)| *length > self.tolerance.distance)
            .collect();
        let mut radii: Vec<(f64, EntityId)> = self
            .curves(Role::Circular)
            .into_iter()
            .filter_map(|curve| Some((self.sketch.circle(curve)?.1, curve)))
            .filter(|(radius, _)| *radius > self.tolerance.distance)
            .collect();
        for sizes in [&mut lengths, &mut radii] {
            sizes.sort_by(|a, b| a.0.total_cmp(&b.0));
            for group in anchored_groups(sizes, self.tolerance.distance) {
                if let Some((&(_, first), rest)) = group.split_first() {
                    for &(_, other) in rest {
                        self.offer(Constraint::Equal(first, other));
                    }
                }
            }
        }
    }

    fn located(&self, points: BTreeSet<EntityId>) -> Vec<(EntityId, Point2)> {
        points
            .into_iter()
            .filter_map(|point| Some((point, self.sketch.point(point)?)))
            .collect()
    }
}

fn ordered(a: EntityId, b: EntityId) -> (EntityId, EntityId) {
    if a <= b { (a, b) } else { (b, a) }
}

pub(crate) fn within_sweep(arc: &ArcGeometry, at: Point2, slack: f64) -> bool {
    let turn = at - arc.center;
    let past_start = (turn.y.atan2(turn.x) - arc.start_angle).rem_euclid(TAU);
    let angular_slack = if arc.radius > 0.0 {
        slack / arc.radius
    } else {
        0.0
    };
    past_start <= arc.sweep + angular_slack || past_start >= TAU - angular_slack
}

fn anchored_groups(sorted: &[(f64, EntityId)], tolerance: f64) -> Vec<Vec<(f64, EntityId)>> {
    let mut groups: Vec<Vec<(f64, EntityId)>> = Vec::new();
    for &(value, id) in sorted {
        match groups.last_mut() {
            Some(group)
                if group
                    .first()
                    .is_some_and(|&(anchor, _)| value - anchor <= tolerance) =>
            {
                group.push((value, id));
            }
            _ => groups.push(vec![(value, id)]),
        }
    }
    groups
}

pub(crate) struct Grid {
    cell: f64,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
}

impl Grid {
    pub(crate) fn new(points: &[(EntityId, Point2)], cell: f64) -> Self {
        let mut grid = Self {
            cell: cell.max(SMALLEST_DISTANCE),
            cells: BTreeMap::new(),
        };
        for (index, &(_, at)) in points.iter().enumerate() {
            let key = grid.key(at);
            grid.cells.entry(key).or_default().push(index);
        }
        grid
    }

    fn key(&self, at: Point2) -> (i64, i64) {
        (
            (at.x / self.cell).floor() as i64,
            (at.y / self.cell).floor() as i64,
        )
    }

    pub(crate) fn near(&self, at: Point2) -> Vec<usize> {
        let (x, y) = self.key(at);
        let mut found = Vec::new();
        for column in x.saturating_sub(1)..=x.saturating_add(1) {
            for row in y.saturating_sub(1)..=y.saturating_add(1) {
                if let Some(indices) = self.cells.get(&(column, row)) {
                    found.extend(indices);
                }
            }
        }
        found.sort_unstable();
        found
    }
}

#[cfg(test)]
mod tests;
