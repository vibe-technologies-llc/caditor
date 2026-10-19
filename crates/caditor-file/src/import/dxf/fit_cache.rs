use std::collections::BTreeMap;

use caditor_geometry::{Point2, Vector2};

use crate::import::dxf::geometry::Nurbs;

const SAME_POINT: f64 = 1e-9;
const NEGLIGIBLE_LEG: f64 = 1e-6;

#[derive(Default)]
pub(super) struct FitCache {
    entries: BTreeMap<Key, Vec<Entry>>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    degree: usize,
    knots: Vec<u64>,
    weights: Option<Vec<u64>>,
}

struct Entry {
    points: Vec<Point2>,
    fitted: Vec<Point2>,
    relative_deviation: f64,
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
            .fold(0.0, f64::max);
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

impl FitCache {
    pub(super) fn find(&self, canonical: &Canonical, tolerance: f64) -> Option<(Vec<Point2>, f64)> {
        let entries = self.entries.get(&canonical.key)?;
        entries.iter().find_map(|entry| {
            let deviation = entry.relative_deviation * canonical.leg;
            if deviation > tolerance {
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

    pub(super) fn store(&mut self, canonical: Canonical, fitted: &[Point2], deviation: f64) {
        let entry = Entry {
            points: canonical.points.clone(),
            fitted: fitted.iter().map(|point| canonical.local(*point)).collect(),
            relative_deviation: deviation / canonical.leg,
        };
        self.entries.entry(canonical.key).or_default().push(entry);
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
        cache.store(canonical, &fitted, 0.01);
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
}
