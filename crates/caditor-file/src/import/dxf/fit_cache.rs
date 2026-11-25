use std::collections::BTreeMap;

use caditor_geometry::{Point2, Vector2, larger};

use crate::import::dxf::geometry::Nurbs;

const SAME_POINT: f64 = 1e-9;
const NEGLIGIBLE_LEG: f64 = 1e-6;
const CELL: f64 = 1e-6;
const REACH_SLACK: f64 = 2.0;

#[derive(Default)]
pub(super) struct FitCache {
    entries: BTreeMap<Key, Grid>,
    stored: usize,
}

#[derive(Default)]
struct Grid {
    cells: BTreeMap<usize, BTreeMap<i64, BTreeMap<i64, Vec<Entry>>>>,
}

#[derive(Clone, Copy)]
struct Signature {
    across: f64,
    spread: f64,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    degree: usize,
    knots: Vec<u64>,
    weights: Option<Vec<u64>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    WithinTolerance,
    ClosestTheSamplesAllow,
}

struct Entry {
    order: usize,
    points: Vec<Point2>,
    fitted: Vec<Point2>,
    relative_deviation: f64,
    reach: Reach,
}

pub(super) struct Canonical {
    key: Key,
    origin: Point2,
    along: Vector2,
    leg: f64,
    points: Vec<Point2>,
}

impl Canonical {
    pub(super) fn of(nurbs: &Nurbs) -> Option<Self> {
        let points: Vec<Point2> = nurbs
            .points
            .iter()
            .map(|point| Point2::new(point.x, point.y))
            .collect();
        let origin = *points.first()?;

        let farthest = points
            .iter()
            .map(|point| point.distance(origin))
            .fold(0.0, larger);
        let far = points
            .iter()
            .find(|point| point.distance(origin) > farthest * NEGLIGIBLE_LEG)?;
        let leg = far.distance(origin);

        let along = (*far - origin) / leg;
        let usable = leg.is_finite() && leg > 0.0 && along.is_finite();
        let bits = |values: &[f64]| values.iter().map(|value| value.to_bits()).collect();

        let mut canonical = Self {
            key: Key {
                degree: nurbs.degree,
                knots: bits(&nurbs.knots),
                weights: nurbs.weights.as_deref().map(bits),
            },
            origin,
            along,
            leg,
            points: Vec::new(),
        };

        canonical.points = points.iter().map(|point| canonical.local(*point)).collect();
        usable.then_some(canonical)
    }

    fn local(&self, point: Point2) -> Point2 {
        let offset = point - self.origin;
        Point2::new(
            offset.dot(self.along) / self.leg,
            offset.dot(self.along.perp()) / self.leg,
        )
    }

    fn placed(&self, local: Point2, mirrored: bool) -> Point2 {
        let across = if mirrored { -local.y } else { local.y };
        self.origin + (self.along * local.x + self.along.perp() * across) * self.leg
    }
}

impl Signature {
    fn of(points: &[Point2]) -> Self {
        Self {
            across: points.iter().map(|point| point.x).sum(),
            spread: points.iter().map(|point| point.length()).sum(),
        }
    }

    fn reach(points: &[Point2]) -> f64 {
        REACH_SLACK
            * SAME_POINT
            * points
                .iter()
                .map(|point| point.length().max(1.0))
                .sum::<f64>()
    }
}

fn cell(value: f64) -> i64 {
    (value / CELL).floor() as i64
}

impl Grid {
    fn insert(&mut self, entry: Entry) {
        let signature = Signature::of(&entry.points);

        self.cells
            .entry(entry.points.len())
            .or_default()
            .entry(cell(signature.across))
            .or_default()
            .entry(cell(signature.spread))
            .or_default()
            .push(entry);
    }

    fn near(&self, points: &[Point2]) -> Vec<&Entry> {
        let Some(columns) = self.cells.get(&points.len()) else {
            return Vec::new();
        };
        let signature = Signature::of(points);
        let reach = Signature::reach(points);

        let across = cell(signature.across - reach)..=cell(signature.across + reach);
        let spread = cell(signature.spread - reach)..=cell(signature.spread + reach);

        let mut near: Vec<&Entry> = columns
            .range(across)
            .flat_map(|(_, rows)| rows.range(spread.clone()))
            .flat_map(|(_, entries)| entries)
            .collect();

        near.sort_unstable_by_key(|entry| entry.order);
        near
    }
}

impl FitCache {
    pub(super) fn find(&self, canonical: &Canonical, tolerance: f64) -> Option<(Vec<Point2>, f64)> {
        self.entries
            .get(&canonical.key)?
            .near(&canonical.points)
            .into_iter()
            .find_map(|entry| {
                let deviation = entry.relative_deviation * canonical.leg;

                if deviation > tolerance && entry.reach == Reach::WithinTolerance {
                    return None;
                }
                [false, true]
                    .into_iter()
                    .find(|mirrored| entry.matches(&canonical.points, *mirrored))
                    .map(|mirrored| {
                        let control_points = entry
                            .fitted
                            .iter()
                            .map(|point| canonical.placed(*point, mirrored))
                            .collect();
                        (control_points, deviation)
                    })
            })
    }

    pub(super) fn store(
        &mut self,
        canonical: Canonical,
        fitted: &[Point2],
        deviation: f64,
        tolerance: f64,
    ) {
        let reach = if deviation <= tolerance {
            Reach::WithinTolerance
        } else {
            Reach::ClosestTheSamplesAllow
        };
        let entry = Entry {
            order: self.stored,
            points: canonical.points.clone(),
            fitted: fitted.iter().map(|point| canonical.local(*point)).collect(),
            relative_deviation: deviation / canonical.leg,
            reach,
        };

        self.stored += 1;
        self.entries.entry(canonical.key).or_default().insert(entry);
    }
}

impl Entry {
    fn matches(&self, points: &[Point2], mirrored: bool) -> bool {
        self.points.len() == points.len()
            && self.points.iter().zip(points).all(|(own, other)| {
                let other = if mirrored {
                    Point2::new(other.x, -other.y)
                } else {
                    *other
                };
                own.distance(other) <= SAME_POINT * own.length().max(1.0)
            })
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Point3;

    use super::*;

    fn nurbs(map: impl Fn(Point2) -> Point2) -> Nurbs {
        let points = [(0.0, 0.0), (1.0, 2.0), (3.0, 2.5), (4.0, 0.5), (6.0, 1.0)]
            .map(|(x, y)| map(Point2::new(x, y)))
            .map(|point| Point3::new(point.x, point.y, 0.0));
        let knots = vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0, 1.0];
        Nurbs::new(3, knots, points.to_vec(), None).unwrap()
    }

    fn cache_holding(original: &Nurbs) -> (FitCache, Vec<Point2>) {
        let canonical = Canonical::of(original).unwrap();
        let fitted: Vec<Point2> = (0..4)
            .map(|index| Point2::new(f64::from(index), f64::from(index * index)))
            .collect();
        let mut cache = FitCache::default();

        cache.store(canonical, &fitted, 0.01, 1.0);
        (cache, fitted)
    }

    #[test]
    fn a_moved_rotated_or_mirrored_copy_takes_the_stored_fit_along() {
        let original = nurbs(|point| point);
        let (cache, fitted) = cache_holding(&original);
        let turn = Vector2::from_angle(0.7);
        let moves: [Box<dyn Fn(Point2) -> Point2>; 3] = [
            Box::new(|point| point + Vector2::new(50.0, -20.0)),
            Box::new(move |point| turn.rotate(point) * 2.0),
            Box::new(|point| Point2::new(-point.x, point.y) + Vector2::new(7.0, 3.0)),
        ];
        for transform in moves {
            let copy = nurbs(&transform);
            let found = cache
                .find(&Canonical::of(&copy).unwrap(), 1.0)
                .map(|(points, _)| points)
                .unwrap();
            let expected_start = transform(fitted[0]);
            let expected_end = transform(fitted[3]);

            assert_eq!(found.len(), fitted.len());
            assert!(found[0].distance(expected_start) < 1e-9);
            assert!(found[3].distance(expected_end) < 1e-9);
        }
    }

    #[test]
    fn a_different_shape_or_a_copy_too_large_for_the_stored_deviation_is_not_reused() {
        let original = nurbs(|point| point);
        let (cache, _) = cache_holding(&original);

        let bent = nurbs(|point| Point2::new(point.x, point.y + 0.001 * point.x * point.x));
        let large = nurbs(|point| point * 100.0);

        assert!(cache.find(&Canonical::of(&bent).unwrap(), 1.0).is_none());
        assert!(cache.find(&Canonical::of(&large).unwrap(), 0.5).is_none());
        assert!(cache.find(&Canonical::of(&large).unwrap(), 5.0).is_some());
    }

    #[test]
    fn a_fit_that_missed_the_tolerance_with_every_attempt_is_reused_for_a_copy_of_any_size() {
        let original = nurbs(|point| point);
        let canonical = Canonical::of(&original).unwrap();
        let fitted: Vec<Point2> = (0..4)
            .map(|index| Point2::new(f64::from(index), 0.0))
            .collect();
        let mut cache = FitCache::default();

        cache.store(canonical, &fitted, 0.5, 0.1);

        let same = cache.find(&Canonical::of(&original).unwrap(), 0.1);
        let large = cache.find(&Canonical::of(&nurbs(|point| point * 100.0)).unwrap(), 0.1);
        let bent = nurbs(|point| Point2::new(point.x, point.y + 0.001 * point.x * point.x));

        assert!(same.is_some_and(|(_, deviation)| (deviation - 0.5).abs() < 1e-12));
        assert!(large.is_some_and(|(_, deviation)| (deviation - 50.0).abs() < 1e-9));
        assert!(cache.find(&Canonical::of(&bent).unwrap(), 0.1).is_none());
    }

    fn canonical_of(points: Vec<Point2>) -> Canonical {
        Canonical {
            key: Key {
                degree: 3,
                knots: Vec::new(),
                weights: None,
            },
            origin: Point2::ZERO,
            along: Vector2::X,
            leg: 1.0,
            points,
        }
    }

    fn shape(x: f64, y: f64) -> Vec<Point2> {
        vec![
            Point2::ZERO,
            Point2::new(1.0, 0.0),
            Point2::new(x, y),
            Point2::new(x + 1.0, y * 0.5),
        ]
    }

    fn fit_at(value: f64) -> Vec<Point2> {
        vec![Point2::new(value, value)]
    }

    #[test]
    fn a_lookup_among_thousands_of_same_knot_shapes_compares_only_its_neighbours() {
        let mut cache = FitCache::default();
        let shapes: Vec<Vec<Point2>> = (0..3000_u32)
            .map(|index| {
                shape(
                    0.5 + f64::from(index) * 1e-3,
                    0.25 + f64::from(index % 7) * 0.1,
                )
            })
            .collect();

        for (index, points) in shapes.iter().enumerate() {
            cache.store(
                canonical_of(points.clone()),
                &fit_at(index as f64),
                0.01,
                1.0,
            );
        }

        let key = canonical_of(Vec::new()).key;
        let grid = cache.entries.get(&key).unwrap();

        for (index, points) in shapes.iter().enumerate() {
            let near = grid.near(points);
            let found = cache.find(&canonical_of(points.clone()), 1.0).unwrap();

            assert!(near.len() <= 2, "{} candidates", near.len());
            assert_eq!(found.0, fit_at(index as f64));
        }
    }

    #[test]
    fn near_equal_shapes_on_either_side_of_a_cell_boundary_still_match() {
        let mut cache = FitCache::default();

        let boundary = ((1.5 / CELL).floor() + 1.0) * CELL;
        let below = shape(boundary - 1.0 - 3e-10, 0.4);
        let above = shape(boundary - 1.0 + 3e-10, 0.4);

        cache.store(canonical_of(below.clone()), &fit_at(1.0), 0.01, 1.0);

        let stored_cell = cell(Signature::of(&below).across);
        let query_cell = cell(Signature::of(&above).across);

        assert_ne!(stored_cell, query_cell);
        assert!(cache.find(&canonical_of(above), 1.0).is_some());
    }

    #[test]
    fn of_two_matching_entries_the_first_stored_is_reused() {
        let mut cache = FitCache::default();
        let first = shape(0.5, 0.4);
        let second = shape(0.5 + 4e-10, 0.4);

        cache.store(canonical_of(first.clone()), &fit_at(1.0), 0.01, 1.0);
        cache.store(canonical_of(second.clone()), &fit_at(2.0), 0.01, 1.0);

        assert_eq!(
            cache.find(&canonical_of(second), 1.0).unwrap().0,
            fit_at(1.0)
        );
        assert_eq!(
            cache.find(&canonical_of(first), 1.0).unwrap().0,
            fit_at(1.0)
        );
    }
}
