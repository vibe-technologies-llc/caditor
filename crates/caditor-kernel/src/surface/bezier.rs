use caditor_geometry::{Aabb, Point3, Vector3};

use crate::intersect::boxes_overlap;

const MAX_SPLITS: usize = 24;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BezierPatch {
    width: usize,
    net: Box<[[f64; 4]]>,
    hull: Hull,
}

#[derive(Clone, Debug, PartialEq)]
struct Hull {
    bounds: Aabb,
    slab: Option<Slab>,
    points: Box<[Point3]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Slab {
    normal: Vector3,
    near: f64,
    far: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum Direction {
    U,
    V,
}

impl Slab {
    fn misses(&self, bounds: &Aabb) -> bool {
        let middle = bounds.center().dot(self.normal);
        let reach = bounds.half_extent().dot(self.normal.abs());
        middle - reach > self.far || middle + reach < self.near
    }

    fn clears(&self, points: &[Point3], margin: f64) -> bool {
        let heights = || points.iter().map(|point| point.dot(self.normal));
        heights().all(|height| height > self.far + margin)
            || heights().all(|height| height < self.near - margin)
    }
}

impl Hull {
    fn of(width: usize, net: &[[f64; 4]]) -> Option<Self> {
        let points = net
            .iter()
            .map(|[x, y, z, w]| (*w > 0.0).then(|| Point3::new(x / w, y / w, z / w)))
            .collect::<Option<Box<[Point3]>>>()?;
        let bounds = Aabb::from_points(points.iter().copied())?;

        let height = points.len().checked_div(width)?;
        let corner = |column: usize, row: usize| points.get(row * width + column).copied();
        let (last_column, last_row) = (width.checked_sub(1)?, height.checked_sub(1)?);

        let slab = match (
            corner(0, 0),
            corner(last_column, 0),
            corner(0, last_row),
            corner(last_column, last_row),
        ) {
            (Some(start), Some(across), Some(along), Some(end)) => (end - start)
                .cross(across - along)
                .try_normalize()
                .map(|normal| {
                    let (near, far) = points
                        .iter()
                        .map(|point| point.dot(normal))
                        .fold((f64::INFINITY, f64::NEG_INFINITY), |(near, far), height| {
                            (near.min(height), far.max(height))
                        });
                    Slab { normal, near, far }
                }),
            _ => None,
        };
        Some(Self {
            bounds,
            slab,
            points,
        })
    }

    fn misses(&self, bounds: &Aabb) -> bool {
        !boxes_overlap(&self.bounds, bounds, 0.0)
            || self.slab.is_some_and(|slab| slab.misses(bounds))
    }

    fn apart(&self, other: &Hull, margin: f64) -> bool {
        !boxes_overlap(&self.bounds, &other.bounds, margin)
            || self
                .slab
                .is_some_and(|slab| slab.clears(&other.points, margin))
            || other
                .slab
                .is_some_and(|slab| slab.clears(&self.points, margin))
    }
}

impl BezierPatch {
    pub(crate) fn new(width: usize, net: Vec<[f64; 4]>) -> Option<Self> {
        if width == 0 || !net.len().is_multiple_of(width) {
            return None;
        }

        let hull = Hull::of(width, &net)?;
        Some(Self {
            width,
            net: net.into_boxed_slice(),
            hull,
        })
    }

    pub(crate) fn heap_size(&self) -> usize {
        size_of_val(&*self.net) + size_of_val(&*self.hull.points)
    }

    pub(crate) fn bounds(&self) -> Aabb {
        self.hull.bounds
    }

    pub(crate) fn misses(&self, bounds: &Aabb) -> bool {
        self.hull.misses(bounds)
    }

    pub(crate) fn apart(&self, other: &Self, margin: f64, tests: &mut usize) -> bool {
        self.apart_within(other, margin, tests, MAX_SPLITS)
    }

    fn apart_within(&self, other: &Self, margin: f64, tests: &mut usize, splits: usize) -> bool {
        let Some(left) = tests.checked_sub(1) else {
            return false;
        };

        *tests = left;
        if self.hull.apart(&other.hull, margin) {
            return true;
        }

        let Some(splits) = splits.checked_sub(1) else {
            return false;
        };

        let (larger, smaller) = if self.hull.bounds.diagonal() >= other.hull.bounds.diagonal() {
            (self, other)
        } else {
            (other, self)
        };
        larger.halves().is_some_and(|halves| {
            halves
                .iter()
                .all(|half| half.apart_within(smaller, margin, tests, splits))
        })
    }

    fn height(&self) -> usize {
        self.net.len() / self.width
    }

    fn point(&self, column: usize, row: usize) -> Option<Point3> {
        self.hull.points.get(row * self.width + column).copied()
    }

    fn extent(&self, direction: Direction) -> f64 {
        let (lines, length) = match direction {
            Direction::U => (self.height(), self.width),
            Direction::V => (self.width, self.height()),
        };
        let at = |line: usize, index: usize| match direction {
            Direction::U => self.point(index, line),
            Direction::V => self.point(line, index),
        };
        (0..lines)
            .filter_map(|line| Some(at(line, 0)?.distance(at(line, length.checked_sub(1)?)?)))
            .fold(0.0, f64::max)
    }

    fn halves(&self) -> Option<[Self; 2]> {
        let direction = if self.extent(Direction::U) >= self.extent(Direction::V) {
            Direction::U
        } else {
            Direction::V
        };
        let (width, height) = (self.width, self.height());

        let (lines, length) = match direction {
            Direction::U => (height, width),
            Direction::V => (width, height),
        };
        let index = |line: usize, along: usize| match direction {
            Direction::U => line * width + along,
            Direction::V => along * width + line,
        };
        let mut low = vec![[0.0; 4]; self.net.len()];
        let mut high = vec![[0.0; 4]; self.net.len()];

        for line in 0..lines {
            let mut points = (0..length)
                .map(|along| self.net.get(index(line, along)).copied())
                .collect::<Option<Vec<[f64; 4]>>>()?;

            for level in 0..length {
                *low.get_mut(index(line, level))? = *points.first()?;
                *high.get_mut(index(line, length - 1 - level))? = *points.last()?;
                points = points
                    .windows(2)
                    .map(|pair| match pair {
                        [[a, b, c, d], [e, f, g, h]] => {
                            [0.5 * (a + e), 0.5 * (b + f), 0.5 * (c + g), 0.5 * (d + h)]
                        }
                        _ => [0.0; 4],
                    })
                    .collect();
            }
        }
        Some([Self::new(width, low)?, Self::new(width, high)?])
    }
}
