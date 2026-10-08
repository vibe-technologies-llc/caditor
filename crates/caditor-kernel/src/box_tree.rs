use caditor_geometry::{Aabb, Point3, Vector3};

use crate::intersect::boxes_overlap;

const LEAF_SIZE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Node {
    bounds: Aabb,
    first: usize,
    count: usize,
    children: Option<[usize; 2]>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BoxTree {
    nodes: Vec<Node>,
    items: Vec<(usize, Aabb)>,
}

fn coordinate(point: Point3, axis: usize) -> f64 {
    match axis {
        0 => point.x,
        1 => point.y,
        _ => point.z,
    }
}

fn widest_axis(items: &[(usize, Aabb)]) -> usize {
    let Some(centers) = Aabb::from_points(items.iter().map(|(_, bounds)| bounds.center())) else {
        return 0;
    };
    let extent = centers.max() - centers.min();
    if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    }
}

fn build(nodes: &mut Vec<Node>, items: &mut [(usize, Aabb)], first: usize) -> Option<usize> {
    let bounds = items
        .iter()
        .map(|(_, bounds)| *bounds)
        .reduce(Aabb::union)?;
    let index = nodes.len();
    nodes.push(Node {
        bounds,
        first,
        count: items.len(),
        children: None,
    });
    if items.len() > LEAF_SIZE {
        let axis = widest_axis(items);
        let middle = items.len() / 2;
        items.select_nth_unstable_by(middle, |a, b| {
            coordinate(a.1.center(), axis).total_cmp(&coordinate(b.1.center(), axis))
        });
        let (low, high) = items.split_at_mut(middle);
        let children = build(nodes, low, first).zip(build(nodes, high, first + middle));
        if let Some(node) = nodes.get_mut(index) {
            node.children = children.map(|(left, right)| [left, right]);
        }
    }
    Some(index)
}

impl BoxTree {
    pub fn new(boxes: impl IntoIterator<Item = Aabb>) -> Self {
        let mut items: Vec<(usize, Aabb)> = boxes.into_iter().enumerate().collect();
        let mut nodes = Vec::with_capacity(2 * items.len() / LEAF_SIZE + 1);
        build(&mut nodes, &mut items, 0);
        Self { nodes, items }
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.nodes.as_slice()) + size_of_val(self.items.as_slice())
    }

    pub fn overlapping(&self, query: &Aabb, margin: f64) -> Vec<usize> {
        self.matching(|bounds| boxes_overlap(bounds, query, margin))
    }

    pub fn matching(&self, test: impl Fn(&Aabb) -> bool) -> Vec<usize> {
        self.matching_among(test, |_| true)
    }

    fn matching_among(
        &self,
        test: impl Fn(&Aabb) -> bool,
        accept: impl Fn(usize) -> bool,
    ) -> Vec<usize> {
        let mut found = Vec::new();
        let mut pending = Vec::new();
        if !self.nodes.is_empty() {
            pending.push(0);
        }
        while let Some(index) = pending.pop() {
            let Some(node) = self.nodes.get(index) else {
                continue;
            };
            if !test(&node.bounds) {
                continue;
            }
            if let Some(children) = node.children {
                pending.extend(children);
                continue;
            }
            let members = self
                .items
                .get(node.first..node.first + node.count)
                .unwrap_or_default();
            found.extend(
                members
                    .iter()
                    .filter(|(item, bounds)| accept(*item) && test(bounds))
                    .map(|(item, _)| *item),
            );
        }
        found.sort_unstable();
        found
    }

    pub fn possibly_nearest(&self, point: Point3, slack: f64) -> Vec<usize> {
        self.possibly_nearest_among(point, slack, |_| true)
    }

    pub fn possibly_nearest_among(
        &self,
        point: Point3,
        slack: f64,
        accept: impl Fn(usize) -> bool,
    ) -> Vec<usize> {
        let mut reach = f64::INFINITY;
        let mut pending = Vec::new();
        if !self.nodes.is_empty() {
            pending.push(0);
        }
        while let Some(index) = pending.pop() {
            let Some(node) = self.nodes.get(index) else {
                continue;
            };
            if nearest_squared(&node.bounds, point) > reach {
                continue;
            }
            if let Some([left, right]) = node.children {
                let distance = |child: usize| {
                    self.nodes
                        .get(child)
                        .map_or(f64::INFINITY, |node| nearest_squared(&node.bounds, point))
                };
                let (near, far) = if distance(left) <= distance(right) {
                    (left, right)
                } else {
                    (right, left)
                };
                pending.extend([far, near]);
                continue;
            }
            let members = self
                .items
                .get(node.first..node.first + node.count)
                .unwrap_or_default();
            for (_, bounds) in members.iter().filter(|(item, _)| accept(*item)) {
                reach = reach.min(farthest_squared(bounds, point));
            }
        }
        let reach = reach.sqrt() + slack;
        let reach = reach * reach;
        self.matching_among(|bounds| nearest_squared(bounds, point) <= reach, accept)
    }
}

fn nearest_squared(bounds: &Aabb, point: Point3) -> f64 {
    let outside = (bounds.min() - point)
        .max(point - bounds.max())
        .max(Vector3::ZERO);
    outside.length_squared()
}

fn farthest_squared(bounds: &Aabb, point: Point3) -> f64 {
    let reach = (point - bounds.min())
        .abs()
        .max((bounds.max() - point).abs());
    reach.length_squared()
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Aabb, Point3, Vector3};

    use super::{BoxTree, farthest_squared, nearest_squared};
    use crate::intersect::{boxes_overlap, line_window};

    fn cube(x: f64, y: f64, z: f64, size: f64) -> Aabb {
        Aabb::from_point(Point3::new(x, y, z)).including(Point3::new(x + size, y + size, z + size))
    }

    #[test]
    fn finds_exactly_the_overlapping_boxes() {
        let boxes: Vec<Aabb> = (0..400)
            .map(|index| {
                let i = f64::from(index);
                cube(
                    (i * 7.3) % 50.0,
                    (i * 3.1) % 40.0,
                    (i * 1.7) % 30.0,
                    1.0 + (i % 5.0),
                )
            })
            .collect();
        let tree = BoxTree::new(boxes.iter().copied());
        for query in [
            cube(0.0, 0.0, 0.0, 3.0),
            cube(20.0, 10.0, 5.0, 8.0),
            cube(49.0, 39.0, 29.0, 0.5),
            cube(-10.0, -10.0, -10.0, 1.0),
            cube(-100.0, -100.0, -100.0, 300.0),
        ] {
            let expected: Vec<usize> = boxes
                .iter()
                .enumerate()
                .filter(|(_, bounds)| boxes_overlap(bounds, &query, 1e-6))
                .map(|(index, _)| index)
                .collect();
            assert_eq!(tree.overlapping(&query, 1e-6), expected);
        }
    }

    #[test]
    fn finds_exactly_the_boxes_a_ray_passes() {
        let boxes: Vec<Aabb> = (0..400)
            .map(|index| {
                let i = f64::from(index);
                cube((i * 7.3) % 50.0, (i * 3.1) % 40.0, (i * 1.7) % 30.0, 0.5)
            })
            .collect();
        let tree = BoxTree::new(boxes.iter().copied());
        for (origin, direction) in [
            (Point3::new(-5.0, 20.0, 15.0), Vector3::X),
            (Point3::ZERO, Vector3::new(5.0, 4.0, 3.0).normalize()),
            (
                Point3::new(25.0, 20.0, 15.0),
                Vector3::new(-1.0, 0.3, 0.2).normalize(),
            ),
            (Point3::new(-5.0, -5.0, -5.0), Vector3::NEG_Z),
        ] {
            let passes = |bounds: &Aabb| line_window(origin, direction, bounds).is_some();
            let expected: Vec<usize> = boxes
                .iter()
                .enumerate()
                .filter(|(_, bounds)| passes(bounds))
                .map(|(index, _)| index)
                .collect();
            assert_eq!(tree.matching(passes), expected);
        }
    }

    #[test]
    fn the_boxes_possibly_nearest_a_point_are_those_no_farther_than_some_box_reaches() {
        let boxes: Vec<Aabb> = (0..400)
            .map(|index| {
                let i = f64::from(index);
                cube(
                    (i * 7.3) % 50.0,
                    (i * 3.1) % 40.0,
                    (i * 1.7) % 30.0,
                    0.5 + (i % 3.0),
                )
            })
            .collect();
        let tree = BoxTree::new(boxes.iter().copied());
        for point in [
            Point3::new(10.0, 10.0, 10.0),
            Point3::new(-20.0, 5.0, 3.0),
            Point3::new(49.0, 39.0, 29.0),
            Point3::new(25.0, 100.0, -40.0),
        ] {
            let reach = boxes
                .iter()
                .map(|bounds| farthest_squared(bounds, point))
                .fold(f64::INFINITY, f64::min)
                .sqrt()
                + 1e-6;
            let expected: Vec<usize> = boxes
                .iter()
                .enumerate()
                .filter(|(_, bounds)| nearest_squared(bounds, point) <= reach * reach)
                .map(|(index, _)| index)
                .collect();

            let found = tree.possibly_nearest(point, 1e-6);

            assert!(!found.is_empty());
            assert_eq!(found, expected, "{point}");
        }
    }

    #[test]
    fn an_empty_tree_finds_nothing() {
        let tree = BoxTree::new(std::iter::empty());
        assert!(tree.overlapping(&cube(0.0, 0.0, 0.0, 1.0), 0.0).is_empty());
    }
}
