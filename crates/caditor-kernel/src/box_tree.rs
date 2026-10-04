use caditor_geometry::{Aabb, Point3};

use crate::intersect::boxes_overlap;

const LEAF_SIZE: usize = 4;

#[derive(Debug, Clone, Copy)]
struct Node {
    bounds: Aabb,
    first: usize,
    count: usize,
    children: Option<[usize; 2]>,
}

#[derive(Debug, Clone, Default)]
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

    pub fn overlapping(&self, query: &Aabb, margin: f64) -> Vec<usize> {
        self.matching(|bounds| boxes_overlap(bounds, query, margin))
    }

    pub fn matching(&self, test: impl Fn(&Aabb) -> bool) -> Vec<usize> {
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
                    .filter(|(_, bounds)| test(bounds))
                    .map(|(item, _)| *item),
            );
        }
        found.sort_unstable();
        found
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Aabb, Point3, Vector3};

    use super::BoxTree;
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
    fn an_empty_tree_finds_nothing() {
        let tree = BoxTree::new(std::iter::empty());
        assert!(tree.overlapping(&cube(0.0, 0.0, 0.0, 1.0), 0.0).is_empty());
    }
}
