use caditor_geometry::{Aabb, Point2, Point3};

use crate::box_tree::BoxTree;

const RELATIVE_SLACK: f64 = 1e-9;
const SCANNED_SEGMENTS: usize = 32;

#[derive(Debug, Clone, Default)]
pub(crate) struct PolygonIndex {
    segments: Vec<[Point2; 2]>,
    tree: BoxTree,
    reach: f64,
    slack: f64,
}

fn flat_box(a: Point2, b: Point2) -> Aabb {
    Aabb::from_point(Point3::new(a.x, a.y, 0.0)).including(Point3::new(b.x, b.y, 0.0))
}

pub(crate) fn crosses(a: Point2, b: Point2, point: Point2) -> bool {
    (a.y > point.y) != (b.y > point.y)
        && point.x < a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
}

impl PolygonIndex {
    pub fn new<'p>(polygons: impl IntoIterator<Item = &'p [Point2]>) -> Self {
        let segments: Vec<[Point2; 2]> = polygons
            .into_iter()
            .flat_map(|polygon| {
                polygon
                    .iter()
                    .zip(polygon.iter().cycle().skip(1))
                    .map(|(a, b)| [*a, *b])
            })
            .collect();
        if segments.len() <= SCANNED_SEGMENTS {
            return Self {
                segments,
                ..Self::default()
            };
        }
        let boxes: Vec<Aabb> = segments.iter().map(|[a, b]| flat_box(*a, *b)).collect();
        let extent = boxes.iter().copied().reduce(Aabb::union);
        let reach = extent.map_or(0.0, |extent| extent.max().x);
        let slack = extent.map_or(0.0, |extent| {
            let size = (extent.max() - extent.min()).max_element();
            let magnitude = extent.max().abs().max(extent.min().abs()).max_element();
            RELATIVE_SLACK * (1.0 + size + magnitude)
        });
        Self {
            segments,
            tree: BoxTree::new(boxes),
            reach,
            slack,
        }
    }

    pub fn contains(&self, point: Point2) -> bool {
        if self.segments.len() <= SCANNED_SEGMENTS {
            return self
                .segments
                .iter()
                .filter(|[a, b]| crosses(*a, *b, point))
                .count()
                % 2
                == 1;
        }
        let toward = flat_box(point, Point2::new(self.reach.max(point.x), point.y));
        self.tree
            .overlapping(&toward, self.slack)
            .into_iter()
            .filter_map(|index| self.segments.get(index))
            .filter(|[a, b]| crosses(*a, *b, point))
            .count()
            % 2
            == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_support::Random, topology::validate::inside_polygon};

    #[test]
    fn agrees_with_a_scan_of_every_segment() {
        let mut random = Random::new(7);
        let star = |random: &mut Random, center: Point2, count: usize| -> Vec<Point2> {
            (0..count)
                .map(|index| {
                    let angle = std::f64::consts::TAU * index as f64 / count as f64;
                    let radius = random.between(1.0, 5.0);
                    center + radius * Point2::new(angle.cos(), angle.sin())
                })
                .collect()
        };
        let outer = star(&mut random, Point2::ZERO, 300);
        let small = star(&mut random, Point2::ZERO, 12);
        let hole = star(&mut random, Point2::new(0.5, -0.25), 40)
            .into_iter()
            .map(|point| Point2::new(0.5, -0.25) + 0.15 * (point - Point2::new(0.5, -0.25)))
            .collect::<Vec<_>>();
        let polygons = [outer.clone(), hole.clone()];
        let index = PolygonIndex::new(polygons.iter().map(Vec::as_slice));

        let mut probes: Vec<Point2> = (0..2000)
            .map(|_| Point2::new(random.between(-6.0, 6.0), random.between(-6.0, 6.0)))
            .collect();
        probes.extend(outer.iter().chain(&hole).copied());

        let scanned = PolygonIndex::new([small.as_slice()]);
        for probe in probes {
            let expected = inside_polygon(&outer, probe) != inside_polygon(&hole, probe);
            assert_eq!(index.contains(probe), expected, "{probe:?}");
            assert_eq!(
                scanned.contains(probe),
                inside_polygon(&small, probe),
                "{probe:?}"
            );
        }
    }

    #[test]
    fn nothing_is_inside_no_polygon() {
        let index = PolygonIndex::new(std::iter::empty());
        assert!(!index.contains(Point2::ZERO));
    }
}
