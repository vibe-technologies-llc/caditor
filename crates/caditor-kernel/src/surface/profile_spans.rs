use caditor_geometry::{Aabb, Point3, Vector3};

use crate::{
    box_tree::BoxTree, bspline::BSpline, curve::Curve, interval::Interval,
    tolerance::LINEAR_RESOLUTION,
};

const MIN_INDEXED_SPANS: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProfileSpans {
    flattened: Option<Vector3>,
    tree: BoxTree,
    spans: Vec<Interval>,
}

impl ProfileSpans {
    pub(crate) fn of(profile: &Curve, flattened: Option<Vector3>) -> Option<Self> {
        let Curve::BSpline(spline) = profile else {
            return None;
        };
        let spans: Vec<Interval> = spline
            .breakpoints()
            .windows(2)
            .filter_map(|pair| match pair {
                [start, end] => Interval::new(*start, *end),
                _ => None,
            })
            .collect();
        if spans.len() < MIN_INDEXED_SPANS {
            return None;
        }
        let hulls: Vec<Aabb> = spans
            .iter()
            .map(|span| hull(spline, *span, flattened))
            .collect::<Option<_>>()?;
        Some(Self {
            flattened,
            tree: BoxTree::new(hulls),
            spans,
        })
    }

    pub(crate) fn heap_size(&self) -> usize {
        size_of::<Self>() + size_of_val(self.spans.as_slice()) + self.tree.heap_size()
    }

    pub(crate) fn runs(&self, point: Point3) -> Vec<Interval> {
        let point = flatten(point, self.flattened);
        let mut runs: Vec<Interval> = Vec::new();
        for span in self
            .tree
            .possibly_nearest(point, LINEAR_RESOLUTION)
            .into_iter()
            .filter_map(|index| self.spans.get(index))
        {
            match runs.last_mut() {
                Some(last) if last.end() == span.start() => {
                    *last = Interval::new(last.start(), span.end()).unwrap_or(*last);
                }
                _ => runs.push(*span),
            }
        }
        runs
    }
}

fn flatten(point: Point3, direction: Option<Vector3>) -> Point3 {
    match direction {
        Some(direction) => point - direction * point.dot(direction),
        None => point,
    }
}

fn hull(spline: &BSpline<Point3>, span: Interval, flattened: Option<Vector3>) -> Option<Aabb> {
    Aabb::from_points(
        spline
            .control_points_over(span)
            .iter()
            .map(|point| flatten(*point, flattened)),
    )
}
