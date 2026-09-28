use caditor_geometry::{Aabb, Point2, Point3, Vector3};

use crate::{
    coordinates::angle_between,
    curve::{Curve, IntersectionCurve},
    intersect::{
        IntersectionError, SurfacePatch, boxes_overlap,
        solve::{
            Constraint, Contact, Coordinate, closest_approach, contact_direction, refine_contact,
            refine_on_plane, uv_direction,
        },
        surface_surface::{Raw, RawCurve, RawPoint, recognize::recognize},
        wrap_into,
    },
    interval::Interval,
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;
const MAX_SEED_PAIRS: usize = 1 << 15;
const MAX_SEED_DEPTH: usize = 40;
const LEAF_CURVATURE: f64 = 0.5;
const MIN_LEAF_FRACTION: f64 = 1.0 / 512.0;
const MAX_LEAF_FRACTION: f64 = 0.25;
const MIN_LEAF: f64 = 1e3 * LINEAR_RESOLUTION;
const TOUCH_SINE: f64 = 1e-5;
const STOP_SINE: f64 = 1e-6;
const SEED_MERGE: f64 = 1e-7;
const COVERED_SINE: f64 = 0.2;
const ON_BRANCH: f64 = 10.0 * LINEAR_RESOLUTION;
const STEP_FRACTION: f64 = 0.125;
const TARGET_TURN: f64 = 0.1;
const MAX_TURN: f64 = 0.3;
const MAX_CORRECTION: f64 = 0.35;
const MIN_STEP: f64 = 1e-2 * LINEAR_RESOLUTION;
const MAX_STEPS: usize = 1 << 15;
const GRID: usize = 3;
const EXIT_BISECTIONS: usize = 48;
const PERIOD_SLACK: f64 = 1e-9;
const CURVATURE_SAMPLES: [f64; 3] = [0.1, 0.5, 0.9];

#[derive(Debug, Clone, Copy)]
struct Seed {
    contact: Contact,
    sine: f64,
}

fn surface_curvature(surface: &Surface, uv: Point2) -> f64 {
    let derivatives = surface.evaluate(uv.x, uv.y);
    let Some(normal) = surface.normal(uv.x, uv.y) else {
        return 0.0;
    };
    let (su, sv) = (derivatives.du.length(), derivatives.dv.length());
    let ratio = |value: f64, scale: f64| {
        if scale > f64::MIN_POSITIVE {
            value.abs() / scale
        } else {
            0.0
        }
    };
    ratio(derivatives.duu.dot(normal), su * su)
        .max(ratio(derivatives.dvv.dot(normal), sv * sv))
        .max(ratio(derivatives.duv.dot(normal), su * sv))
}

fn leaf_size(patch: &SurfacePatch) -> f64 {
    let diagonal = patch.bounding_box().diagonal();
    let (u, v) = (patch.u_range(), patch.v_range());
    let curvature = CURVATURE_SAMPLES
        .iter()
        .flat_map(|a| {
            CURVATURE_SAMPLES
                .iter()
                .map(move |b| Point2::new(u.at(*a), v.at(*b)))
        })
        .map(|uv| surface_curvature(patch.surface(), uv))
        .fold(0.0, f64::max);
    let by_curvature = if curvature > 0.0 {
        LEAF_CURVATURE / curvature
    } else {
        f64::INFINITY
    };
    by_curvature
        .min(diagonal * MAX_LEAF_FRACTION)
        .max(diagonal * MIN_LEAF_FRACTION)
        .max(MIN_LEAF)
}

fn signed_distance(surface: &Surface, point: Point3) -> f64 {
    let uv = surface.project(point, None);
    let offset = point - surface.point_at(uv);
    match surface.normal(uv.x, uv.y) {
        Some(normal) => offset.dot(normal),
        None => offset.length(),
    }
}

struct Node<'a> {
    patch: SurfacePatch<'a>,
    bounds: Aabb,
    apart: bool,
    children: Option<[usize; 2]>,
}

struct Arena<'a> {
    nodes: Vec<Node<'a>>,
    other: &'a Surface,
    leaf: f64,
}

impl<'a> Arena<'a> {
    fn new(patch: SurfacePatch<'a>, other: &'a Surface) -> Self {
        let mut arena = Self {
            nodes: Vec::new(),
            other,
            leaf: leaf_size(&patch),
        };
        arena.push(patch);
        arena
    }

    fn push(&mut self, patch: SurfacePatch<'a>) -> usize {
        let bounds = patch.bounding_box();
        let apart =
            self.other.distance(bounds.center()) > bounds.half_extent().length() + TOLERANCE;
        self.nodes.push(Node {
            patch,
            bounds,
            apart,
            children: None,
        });
        self.nodes.len() - 1
    }

    fn children(&mut self, index: usize) -> [usize; 2] {
        if let Some(children) = self.nodes.get(index).and_then(|node| node.children) {
            return children;
        }
        let Some(patch) = self.nodes.get(index).map(|node| node.patch) else {
            return [index, index];
        };
        let [low, high] = patch.halves();
        let children = [self.push(low), self.push(high)];
        if let Some(node) = self.nodes.get_mut(index) {
            node.children = Some(children);
        }
        children
    }
}

struct Tracer<'a> {
    patches: [SurfacePatch<'a>; 2],
    max_step: f64,
    curves: Vec<(Curve, Aabb)>,
    points: Vec<RawPoint>,
}

struct Marched {
    contacts: Vec<Contact>,
    closed: bool,
    tangent_end: bool,
}

enum Stalled {
    Tangent,
    Collapsed,
}

struct Step {
    contact: Contact,
    tangent: Vector3,
    turn: f64,
}

impl<'a> Tracer<'a> {
    fn surfaces(&self) -> [&'a Surface; 2] {
        let [first, second] = self.patches;
        [first.surface(), second.surface()]
    }

    fn inside(&self, contact: &Contact) -> bool {
        let [first, second] = self.patches;
        let [a, b] = contact.uv;
        first.contains(a) && second.contains(b)
    }

    fn seed_at(&self, contact: Contact) -> Option<Seed> {
        if !self.inside(&contact) {
            return None;
        }
        let sine = contact_direction(self.surfaces(), &contact).map_or(0.0, |(_, sine)| sine);
        Some(Seed { contact, sine })
    }

    fn leaf_seeds(&self, a: &SurfacePatch, b: &SurfacePatch) -> Option<Seed> {
        let surfaces = self.surfaces();
        let start = [a.bounds().center(), b.bounds().center()];
        let (uv, gap) = closest_approach(surfaces, start);
        if gap <= TOLERANCE {
            let [first, second] = uv;
            let fallback = Contact {
                uv,
                point: a
                    .surface()
                    .point_at(first)
                    .lerp(b.surface().point_at(second), 0.5),
            };
            let contact = refine_contact(surfaces, uv, Constraint::Free).unwrap_or(fallback);
            if let Some(seed) = self.seed_at(contact) {
                return Some(seed);
            }
        }
        self.grid_seed(a, b)
    }

    fn grid_seed(&self, a: &SurfacePatch, b: &SurfacePatch) -> Option<Seed> {
        let (u, v) = (a.u_range(), a.v_range());
        let other = b.surface();
        let at = |row: usize, column: usize| {
            Point2::new(
                u.at(column as f64 / GRID as f64),
                v.at(row as f64 / GRID as f64),
            )
        };
        let value = |uv: Point2| signed_distance(other, a.surface().point_at(uv));
        let mut pairs = Vec::new();
        for row in 0..=GRID {
            for column in 0..=GRID {
                if column < GRID {
                    pairs.push((at(row, column), at(row, column + 1)));
                }
                if row < GRID {
                    pairs.push((at(row, column), at(row + 1, column)));
                }
            }
        }
        for (start, end) in pairs {
            let (low, high) = (value(start), value(end));
            if (low < 0.0) == (high < 0.0) {
                continue;
            }
            let along = |fraction: f64| value(start.lerp(end, fraction));
            let fraction =
                crate::intersect::solve::bracket_root(along, 0.0, 1.0, low, high).clamp(0.0, 1.0);
            let uv = start.lerp(end, fraction);
            let point = a.surface().point_at(uv);
            let guess = [uv, other.project(point, Some(b.bounds().center()))];
            if let Some(seed) = refine_contact(self.surfaces(), guess, Constraint::Free)
                .and_then(|contact| self.seed_at(contact))
            {
                return Some(seed);
            }
        }
        None
    }

    fn seeds(&self) -> Result<Vec<Seed>, IntersectionError> {
        let [first, second] = self.patches;
        let mut arenas = [
            Arena::new(first, second.surface()),
            Arena::new(second, first.surface()),
        ];
        let mut pending = vec![(0usize, 0usize, 0usize)];
        let mut seeds: Vec<Seed> = Vec::new();
        let mut visited = 0usize;
        while let Some((a, b, depth)) = pending.pop() {
            visited += 1;
            if visited > MAX_SEED_PAIRS {
                return Err(IntersectionError::TooComplex(MAX_SEED_PAIRS));
            }
            let [first_arena, second_arena] = &arenas;
            let (Some(a_node), Some(b_node)) =
                (first_arena.nodes.get(a), second_arena.nodes.get(b))
            else {
                continue;
            };
            if a_node.apart
                || b_node.apart
                || !boxes_overlap(&a_node.bounds, &b_node.bounds, TOLERANCE)
            {
                continue;
            }
            let a_ratio = a_node.bounds.diagonal() / first_arena.leaf;
            let b_ratio = b_node.bounds.diagonal() / second_arena.leaf;
            if (a_ratio <= 1.0 && b_ratio <= 1.0) || depth >= MAX_SEED_DEPTH {
                let (a_box, b_box) = (a_node.bounds, b_node.bounds);
                let covered = seeds.iter().any(|known| {
                    known.sine > COVERED_SINE
                        && boxes_overlap(&a_box, &Aabb::from_point(known.contact.point), TOLERANCE)
                        && boxes_overlap(&b_box, &Aabb::from_point(known.contact.point), TOLERANCE)
                });
                if covered {
                    continue;
                }
                if let Some(seed) = self.leaf_seeds(&a_node.patch, &b_node.patch)
                    && !seeds
                        .iter()
                        .any(|known| known.contact.point.distance(seed.contact.point) <= SEED_MERGE)
                {
                    seeds.push(seed);
                }
                continue;
            }
            let [first_arena, second_arena] = &mut arenas;
            if a_ratio >= b_ratio {
                let [low, high] = first_arena.children(a);
                pending.push((high, b, depth + 1));
                pending.push((low, b, depth + 1));
            } else {
                let [low, high] = second_arena.children(b);
                pending.push((a, high, depth + 1));
                pending.push((a, low, depth + 1));
            }
        }
        Ok(seeds)
    }

    fn distance_to_branches(&self, point: Point3) -> f64 {
        let mut best = f64::INFINITY;
        for (curve, bounds) in &self.curves {
            if !boxes_overlap(bounds, &Aabb::from_point(point), ON_BRANCH) {
                continue;
            }
            let Curve::Intersection(intersection) = curve else {
                continue;
            };
            let nodes = intersection.nodes();
            let mut order: [(usize, f64); 2] = [(0, f64::INFINITY); 2];
            for (index, node) in nodes.iter().enumerate() {
                let distance = node.point.distance_squared(point);
                let [best, second] = order;
                if distance < best.1 {
                    order = [(index, distance), best];
                } else if distance < second.1 {
                    order = [best, (index, distance)];
                }
            }
            for (nearest, _) in order {
                let low = nodes
                    .get(nearest.saturating_sub(2))
                    .map_or(0.0, |node| node.parameter);
                let high = nodes
                    .get((nearest + 2).min(nodes.len().saturating_sub(1)))
                    .map_or(low, |node| node.parameter);
                let Some(range) = Interval::new(low, high) else {
                    continue;
                };
                let parameter = curve.closest_parameter(point, range);
                best = best.min(curve.point(parameter).distance(point));
            }
        }
        best
    }

    fn step(&self, current: &Contact, tangent: Vector3, step: &mut f64) -> Result<Step, Stalled> {
        let surfaces = self.surfaces();
        let (direction, sine) = contact_direction(surfaces, current).ok_or(Stalled::Tangent)?;
        if sine < STOP_SINE {
            return Err(Stalled::Tangent);
        }
        let direction = if direction.dot(tangent) < 0.0 {
            -direction
        } else {
            direction
        };
        let [first, second] = surfaces;
        let [uv_first, uv_second] = current.uv;
        let (first_derivatives, second_derivatives) = (
            first.evaluate(uv_first.x, uv_first.y),
            second.evaluate(uv_second.x, uv_second.y),
        );
        while *step >= MIN_STEP {
            let h = *step;
            let predicted = current.point + direction * h;
            let guess = [
                uv_first + uv_direction(&first_derivatives, direction * h),
                uv_second + uv_direction(&second_derivatives, direction * h),
            ];
            let check = |contact: Contact| {
                let (next, _) = contact_direction(surfaces, &contact)?;
                let next = if next.dot(direction) < 0.0 {
                    -next
                } else {
                    next
                };
                let turn = angle_between(direction, next);
                let correction = contact.point.distance(predicted);
                let moved = contact.point.distance(current.point);
                (correction <= MAX_CORRECTION * h && moved >= 0.5 * h && turn <= MAX_TURN)
                    .then_some(Step {
                        contact,
                        tangent: next,
                        turn,
                    })
            };
            let accepted = refine_contact(
                surfaces,
                guess,
                Constraint::Plane {
                    point: predicted,
                    normal: direction,
                },
            )
            .and_then(check)
            .or_else(|| refine_on_plane(surfaces, guess, predicted, direction).and_then(check));
            if let Some(found) = accepted {
                let growth = (TARGET_TURN / found.turn.max(1e-6)).clamp(0.5, 2.0);
                *step = (h * growth).min(self.max_step);
                return Ok(found);
            }
            *step *= 0.5;
        }
        Err(Stalled::Collapsed)
    }

    fn exit(&self, inside: &Contact, outside: &Contact) -> Option<Contact> {
        let mut best: Option<(f64, Coordinate, f64)> = None;
        let [inside_first, inside_second] = inside.uv;
        let [outside_first, outside_second] = outside.uv;
        let [first, second] = self.patches;
        let checks = [
            (
                first,
                true,
                inside_first.x,
                outside_first.x,
                Coordinate::FirstU,
            ),
            (
                first,
                false,
                inside_first.y,
                outside_first.y,
                Coordinate::FirstV,
            ),
            (
                second,
                true,
                inside_second.x,
                outside_second.x,
                Coordinate::SecondU,
            ),
            (
                second,
                false,
                inside_second.y,
                outside_second.y,
                Coordinate::SecondV,
            ),
        ];
        for (patch, along_u, from, to, coordinate) in checks {
            let (range, period) = if along_u {
                (patch.u_range(), patch.surface().u_period())
            } else {
                (patch.v_range(), patch.surface().v_period())
            };
            if period.is_some_and(|period| range.length() >= period * (1.0 - PERIOD_SLACK)) {
                continue;
            }
            let shift = from - wrap_into(from, range, period);
            let (low, high) = (range.start() + shift, range.end() + shift);
            let bound = if to > high {
                high
            } else if to < low {
                low
            } else {
                continue;
            };
            let fraction = if to != from {
                ((bound - from) / (to - from)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if best.is_none_or(|(known, _, _)| fraction < known) {
                best = Some((fraction, coordinate, bound));
            }
        }
        let (fraction, coordinate, bound) = best?;
        let guess = [
            inside_first.lerp(outside_first, fraction),
            inside_second.lerp(outside_second, fraction),
        ];
        let reach = 2.0 * inside.point.distance(outside.point) + TOLERANCE;
        let solved = refine_contact(
            self.surfaces(),
            guess,
            Constraint::Parameter {
                coordinate,
                value: bound,
            },
        )
        .filter(|contact| contact.point.distance(inside.point) <= reach);
        solved.or_else(|| self.bisect_exit(inside, outside))
    }

    fn boundary_ahead(&self, current: &Contact, tangent: Vector3) -> Option<Contact> {
        let surfaces = self.surfaces();
        let [first, second] = self.patches;
        let [uv_first, uv_second] = current.uv;
        let [surface_first, surface_second] = surfaces;
        let velocity_first = uv_direction(&surface_first.evaluate(uv_first.x, uv_first.y), tangent);
        let velocity_second =
            uv_direction(&surface_second.evaluate(uv_second.x, uv_second.y), tangent);
        let checks = [
            (
                first,
                true,
                uv_first.x,
                velocity_first.x,
                Coordinate::FirstU,
            ),
            (
                first,
                false,
                uv_first.y,
                velocity_first.y,
                Coordinate::FirstV,
            ),
            (
                second,
                true,
                uv_second.x,
                velocity_second.x,
                Coordinate::SecondU,
            ),
            (
                second,
                false,
                uv_second.y,
                velocity_second.y,
                Coordinate::SecondV,
            ),
        ];
        let mut nearest: Option<(f64, Coordinate, f64)> = None;
        for (patch, along_u, value, velocity, coordinate) in checks {
            let (range, period) = if along_u {
                (patch.u_range(), patch.surface().u_period())
            } else {
                (patch.v_range(), patch.surface().v_period())
            };
            if period.is_some_and(|period| range.length() >= period * (1.0 - PERIOD_SLACK))
                || velocity == 0.0
                || !velocity.is_finite()
            {
                continue;
            }
            let shift = value - wrap_into(value, range, period);
            let bound = if velocity > 0.0 {
                range.end() + shift
            } else {
                range.start() + shift
            };
            let reach = (bound - value) / velocity;
            if reach >= 0.0 && nearest.is_none_or(|(known, _, _)| reach < known) {
                nearest = Some((reach, coordinate, bound));
            }
        }
        let (reach, coordinate, bound) = nearest?;
        if reach > self.max_step {
            return None;
        }
        refine_contact(
            surfaces,
            current.uv,
            Constraint::Parameter {
                coordinate,
                value: bound,
            },
        )
        .filter(|contact| contact.point.distance(current.point) <= 2.0 * self.max_step)
    }

    fn bisect_exit(&self, inside: &Contact, outside: &Contact) -> Option<Contact> {
        let normal = (outside.point - inside.point).try_normalize()?;
        let (mut low, mut high) = (*inside, *outside);
        for _ in 0..EXIT_BISECTIONS {
            let [a0, a1] = low.uv;
            let [b0, b1] = high.uv;
            let guess = [a0.lerp(b0, 0.5), a1.lerp(b1, 0.5)];
            let middle = refine_on_plane(
                self.surfaces(),
                guess,
                low.point.lerp(high.point, 0.5),
                normal,
            )?;
            if self.inside(&middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
        Some(low)
    }

    fn march(&self, seed: &Contact, forward: bool) -> Result<Marched, IntersectionError> {
        let mut contacts = vec![*seed];
        let Some((initial, _)) = contact_direction(self.surfaces(), seed) else {
            return Ok(Marched {
                contacts,
                closed: false,
                tangent_end: true,
            });
        };
        let seed_tangent = if forward { initial } else { -initial };
        let mut tangent = seed_tangent;
        let mut step = self.max_step * 0.25;
        let mut steps = 0usize;
        while steps < MAX_STEPS {
            steps += 1;
            let Some(current) = contacts.last().copied() else {
                break;
            };
            let next = match self.step(&current, tangent, &mut step) {
                Ok(next) => next,
                Err(Stalled::Tangent) => {
                    return Ok(Marched {
                        contacts,
                        closed: false,
                        tangent_end: true,
                    });
                }
                Err(Stalled::Collapsed) => {
                    let boundary = self
                        .boundary_ahead(&current, tangent)
                        .ok_or(IntersectionError::Unfollowable)?;
                    if boundary.point.distance(current.point) > TOLERANCE {
                        contacts.push(boundary);
                    }
                    return Ok(Marched {
                        contacts,
                        closed: false,
                        tangent_end: false,
                    });
                }
            };
            if !self.inside(&next.contact) {
                if let Some(boundary) = self.exit(&current, &next.contact)
                    && boundary.point.distance(current.point) > TOLERANCE
                {
                    contacts.push(boundary);
                }
                return Ok(Marched {
                    contacts,
                    closed: false,
                    tangent_end: false,
                });
            }
            if steps >= 3 && closes(seed, seed_tangent, &current, &next) {
                contacts.push(*seed);
                return Ok(Marched {
                    contacts,
                    closed: true,
                    tangent_end: false,
                });
            }
            let merged = self.distance_to_branches(next.contact.point) <= ON_BRANCH;
            contacts.push(next.contact);
            tangent = next.tangent;
            if merged {
                return Ok(Marched {
                    contacts,
                    closed: false,
                    tangent_end: false,
                });
            }
        }
        Err(IntersectionError::Unfollowable)
    }

    fn trace(&mut self, seed: &Seed) -> Result<(), IntersectionError> {
        let forward = self.march(&seed.contact, true)?;
        let (contacts, closed, ends) = if forward.closed {
            (forward.contacts, true, Vec::new())
        } else {
            let backward = self.march(&seed.contact, false)?;
            let mut ends = Vec::new();
            if backward.tangent_end
                && let Some(first) = backward.contacts.last()
            {
                ends.push(first.point);
            }
            if forward.tangent_end
                && let Some(last) = forward.contacts.last()
            {
                ends.push(last.point);
            }
            let mut contacts: Vec<Contact> = backward.contacts.into_iter().rev().collect();
            contacts.extend(forward.contacts.into_iter().skip(1));
            (contacts, false, ends)
        };
        self.points.extend(ends.into_iter().map(|point| RawPoint {
            point,
            tangent: true,
        }));
        if contacts.len() < 2 {
            return Ok(());
        }
        let [first, second] = self.surfaces();
        let Some(curve) =
            IntersectionCurve::from_contacts([first.clone(), second.clone()], &contacts, closed)
        else {
            return Ok(());
        };
        let curve = Curve::Intersection(curve);
        let bounds = curve.bounding_box(curve.domain().clipped(1.0));
        self.curves.push((curve, bounds));
        Ok(())
    }
}

fn closes(seed: &Contact, seed_tangent: Vector3, current: &Contact, next: &Step) -> bool {
    let (from, to) = (current.point, next.contact.point);
    let (before, after) = (
        (from - seed.point).dot(seed_tangent),
        (to - seed.point).dot(seed_tangent),
    );
    if before >= 0.0 || after < 0.0 {
        return false;
    }
    let fraction = -before / (after - before);
    let crossing = from.lerp(to, fraction);
    let chord = from.distance(to);
    crossing.distance(seed.point) <= chord * (0.25 * next.turn + 0.05) + TOLERANCE
}

pub(crate) fn intersect(
    first: &SurfacePatch,
    second: &SurfacePatch,
) -> Result<Raw, IntersectionError> {
    let smallest = first
        .bounding_box()
        .diagonal()
        .min(second.bounding_box().diagonal());
    let mut tracer = Tracer {
        patches: [*first, *second],
        max_step: (STEP_FRACTION * smallest).max(MIN_LEAF),
        curves: Vec::new(),
        points: Vec::new(),
    };
    let seeds = tracer.seeds()?;
    let touch_merge = 0.5 * leaf_size(first).min(leaf_size(second));
    let mut touches: Vec<Seed> = Vec::new();
    for seed in &seeds {
        if seed.sine <= TOUCH_SINE {
            touches.push(*seed);
            continue;
        }
        if tracer.distance_to_branches(seed.contact.point) <= ON_BRANCH {
            continue;
        }
        tracer.trace(seed)?;
    }
    let mut raw = Raw::default();
    for touch in touches {
        let point = touch.contact.point;
        let known = tracer
            .points
            .iter()
            .chain(&raw.points)
            .any(|existing| existing.point.distance(point) <= touch_merge);
        if known || tracer.distance_to_branches(point) <= ON_BRANCH {
            continue;
        }
        raw.points.push(RawPoint {
            point,
            tangent: true,
        });
    }
    raw.points.extend(tracer.points.iter().copied());
    for (curve, _) in tracer.curves {
        let Curve::Intersection(intersection) = &curve else {
            continue;
        };
        let (curve, range) = recognize(intersection).unwrap_or_else(|| {
            let range = intersection.domain();
            (curve.clone(), range)
        });
        raw.curves.push(RawCurve {
            curve,
            range,
            tangent: false,
        });
    }
    Ok(raw)
}
