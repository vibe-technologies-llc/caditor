use caditor_geometry::{Aabb, Point2, Point3, Vector2, Vector3, larger};

use crate::{
    intersect::{SurfacePatch, surface_surface::overlap::sampled_kind},
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;
const LEAF_CURVATURE: f64 = 0.5;
const MIN_LEAF_FRACTION: f64 = 1.0 / 512.0;
const MAX_LEAF_FRACTION: f64 = 0.25;
pub(super) const MIN_LEAF: f64 = 1e3 * LINEAR_RESOLUTION;
const CURVATURE_SAMPLES: [f64; 3] = [0.1, 0.5, 0.9];
const FIELD_CURVATURE_REACH: f64 = 0.5;
const FOOT_ALONG_NORMAL: f64 = 1e-6;
const MAX_KEPT_PIECES: usize = 1 << 15;

pub(crate) struct Subdivision<'a> {
    pieces: Vec<Piece<'a>>,
    leaf: Option<f64>,
}

struct Piece<'a> {
    patch: SurfacePatch<'a>,
    bounds: Aabb,
    centre: Option<Option<Centre>>,
    bend: Option<Option<Bend>>,
    children: Option<[usize; 2]>,
}

#[derive(Clone, Copy)]
struct Centre {
    point: Point3,
    normal: Vector3,
}

#[derive(Clone, Copy)]
struct Bend {
    reach: f64,
    curvature: f64,
}

impl<'a> Piece<'a> {
    fn new(patch: SurfacePatch<'a>) -> Self {
        Self {
            patch,
            bounds: patch.bounding_box(),
            centre: None,
            bend: None,
            children: None,
        }
    }

    fn centre(&mut self) -> Option<Centre> {
        *self.centre.get_or_insert_with(|| {
            let surface = self.patch.surface();
            let uv = self.patch.bounds().center();
            Some(Centre {
                point: surface.point_at(uv),
                normal: surface.normal(uv.x, uv.y)?,
            })
        })
    }

    fn bend(&mut self, point: Point3) -> Option<Bend> {
        *self.bend.get_or_insert_with(|| {
            let surface = self.patch.surface();
            let uv = self.patch.bounds().center();
            let (u, v) = (self.patch.u_range(), self.patch.v_range());

            let corners = [
                Point2::new(u.start(), v.start()),
                Point2::new(u.end(), v.start()),
                Point2::new(u.start(), v.end()),
                Point2::new(u.end(), v.end()),
            ];
            let reach = corners
                .iter()
                .map(|corner| surface.point_at(*corner).distance(point))
                .fold(0.0, larger);
            let [first, second, third, fourth] = corners;

            let curvature = largest_curvature(surface, &[uv, first, second, third, fourth])?;
            Some(Bend { reach, curvature })
        })
    }

    fn apart_from(&mut self, other: &Surface) -> bool {
        match other {
            Surface::BSpline(spline) => spline.hulls_miss(&self.bounds.expanded(TOLERANCE)),
            _ => {
                other.distance(self.bounds.center())
                    > self.bounds.half_extent().length() + TOLERANCE
                    || self.apart_to_second_order(other)
            }
        }
    }

    fn apart_to_second_order(&mut self, other: &Surface) -> bool {
        let Some(Centre { point, normal }) = self.centre() else {
            return false;
        };

        let foot = other.project(point, None);

        let offset = point - other.point_at(foot);
        let distance = offset.length();

        let Some(other_normal) = other.normal(foot.x, foot.y) else {
            return false;
        };

        if offset.cross(other_normal).length() > FOOT_ALONG_NORMAL * distance + TOLERANCE {
            return false;
        }

        let Some(Bend {
            reach,
            curvature: own,
        }) = self.bend(point)
        else {
            return false;
        };

        let derivatives = other.evaluate(foot.x, foot.y);
        let (su, sv) = (derivatives.du.length(), derivatives.dv.length());

        if su <= LINEAR_RESOLUTION || sv <= LINEAR_RESOLUTION {
            return false;
        }

        let (du, dv) = (reach / su, reach / sv);

        let Some(field) = largest_curvature(
            other,
            &[
                foot,
                foot + Vector2::new(du, 0.0),
                foot - Vector2::new(du, 0.0),
                foot + Vector2::new(0.0, dv),
                foot - Vector2::new(0.0, dv),
            ],
        ) else {
            return false;
        };

        if field * (distance + reach) >= FIELD_CURVATURE_REACH {
            return false;
        }

        let sine = normal.cross(other_normal).length();
        let bending = own + 2.0 * field;
        distance > sine * reach + bending * reach * reach + TOLERANCE
    }
}

impl<'a> Subdivision<'a> {
    pub(crate) fn new(patch: SurfacePatch<'a>) -> Self {
        Self {
            pieces: vec![Piece::new(patch)],
            leaf: None,
        }
    }

    pub(crate) fn worth_keeping(surface: &Surface) -> bool {
        sampled_kind(surface)
    }

    pub(super) fn of(&mut self, patch: &SurfacePatch<'a>) -> &mut Self {
        let same = self.pieces.first().is_some_and(|root| {
            std::ptr::eq(root.patch.surface(), patch.surface())
                && root.patch.bounds() == patch.bounds()
        });

        if !same || self.pieces.len() > MAX_KEPT_PIECES {
            *self = Self::new(*patch);
        }
        self
    }

    #[cfg(test)]
    pub(crate) fn pieces(&self) -> usize {
        self.pieces.len()
    }

    pub(super) fn root_bounds(&self) -> Aabb {
        self.pieces
            .first()
            .map_or_else(|| Aabb::from_point(Point3::ZERO), |root| root.bounds)
    }

    pub(super) fn leaf(&mut self) -> f64 {
        if let Some(leaf) = self.leaf {
            return leaf;
        }

        let leaf = self
            .pieces
            .first()
            .map_or(MIN_LEAF, |root| leaf_size(&root.patch, root.bounds));

        self.leaf = Some(leaf);
        leaf
    }

    pub(super) fn patch(&self, index: usize) -> Option<SurfacePatch<'a>> {
        self.pieces.get(index).map(|piece| piece.patch)
    }

    pub(super) fn bounds(&self, index: usize) -> Option<Aabb> {
        self.pieces.get(index).map(|piece| piece.bounds)
    }

    pub(super) fn apart_from(&mut self, index: usize, other: &Surface) -> bool {
        self.pieces
            .get_mut(index)
            .is_some_and(|piece| piece.apart_from(other))
    }

    pub(super) fn children(&mut self, index: usize) -> [usize; 2] {
        if let Some(children) = self.pieces.get(index).and_then(|piece| piece.children) {
            return children;
        }

        let Some(patch) = self.patch(index) else {
            return [index, index];
        };
        let [low, high] = patch.halves();

        self.pieces.push(Piece::new(low));
        self.pieces.push(Piece::new(high));

        let children = [self.pieces.len() - 2, self.pieces.len() - 1];

        if let Some(piece) = self.pieces.get_mut(index) {
            piece.children = Some(children);
        }
        children
    }
}

fn surface_curvature(surface: &Surface, uv: Point2) -> f64 {
    let derivatives = surface.evaluate(uv.x, uv.y);
    let Some(normal) = surface.normal_evaluated(uv.x, uv.y, &derivatives) else {
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

fn leaf_size(patch: &SurfacePatch, bounds: Aabb) -> f64 {
    let diagonal = bounds.diagonal();
    let (u, v) = (patch.u_range(), patch.v_range());

    let curvature = CURVATURE_SAMPLES
        .iter()
        .flat_map(|a| {
            CURVATURE_SAMPLES
                .iter()
                .map(move |b| Point2::new(u.at(*a), v.at(*b)))
        })
        .map(|uv| surface_curvature(patch.surface(), uv))
        .fold(0.0, larger);
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

pub(super) fn curvature_bound(surface: &Surface, uv: Point2) -> Option<f64> {
    let derivatives = surface.evaluate(uv.x, uv.y);
    let normal = surface.normal_evaluated(uv.x, uv.y, &derivatives)?;
    let (su, sv) = (derivatives.du.length(), derivatives.dv.length());

    if su <= LINEAR_RESOLUTION || sv <= LINEAR_RESOLUTION {
        return None;
    }

    let curvature = (derivatives.duu.dot(normal).abs() / (su * su))
        .max(derivatives.dvv.dot(normal).abs() / (sv * sv))
        .max(derivatives.duv.dot(normal).abs() / (su * sv));
    curvature.is_finite().then_some(curvature)
}

fn largest_curvature(surface: &Surface, uvs: &[Point2]) -> Option<f64> {
    uvs.iter()
        .map(|uv| curvature_bound(surface, *uv))
        .try_fold(0.0, |largest: f64, curvature| {
            Some(larger(largest, curvature?))
        })
}
