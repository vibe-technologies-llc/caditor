use std::collections::BTreeSet;

use caditor_geometry::{Aabb2, Point2, Vector2};

use crate::{
    curve2::{Circle2, Curve2},
    interval::Interval,
    measure::Accuracy,
    numeric::GAUSS_LEGENDRE,
    parametric::Parametric,
    profile::{Piece, PieceId, Region, Side},
    tolerance::LINEAR_RESOLUTION,
};

const QUADRATURE_TOLERANCE: f64 = 1e-11;
const MAX_QUADRATURE_DEPTH: usize = 24;
const ISOTROPIC_SHARE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Integrals {
    perimeter: f64,
    area: f64,
    x: f64,
    y: f64,
    xx: f64,
    yy: f64,
    xy: f64,
}

impl Integrals {
    fn plus(self, other: Self) -> Self {
        Self {
            perimeter: self.perimeter + other.perimeter,
            area: self.area + other.area,
            x: self.x + other.x,
            y: self.y + other.y,
            xx: self.xx + other.xx,
            yy: self.yy + other.yy,
            xy: self.xy + other.xy,
        }
    }

    fn scaled(self, factor: f64) -> Self {
        Self {
            perimeter: self.perimeter * factor,
            area: self.area * factor,
            x: self.x * factor,
            y: self.y * factor,
            xx: self.xx * factor,
            yy: self.yy * factor,
            xy: self.xy * factor,
        }
    }

    fn enclosed(self) -> Self {
        Self {
            perimeter: 0.0,
            ..self
        }
    }

    fn reversed(self) -> Self {
        Self {
            perimeter: self.perimeter,
            ..self.scaled(-1.0)
        }
    }

    fn misfit(self, other: Self, size: f64) -> f64 {
        let difference = self.plus(other.scaled(-1.0));
        [
            difference.perimeter / size,
            difference.area / size.powi(2),
            difference.x / size.powi(3),
            difference.y / size.powi(3),
            difference.xx / size.powi(4),
            difference.yy / size.powi(4),
            difference.xy / size.powi(4),
        ]
        .into_iter()
        .fold(0.0, |most, value| most.max(value.abs()))
    }

    fn fan(point: Vector2, velocity: Vector2) -> Self {
        let swept = point.perp_dot(velocity);
        Self {
            perimeter: velocity.length(),
            area: swept / 2.0,
            x: point.x * swept / 3.0,
            y: point.y * swept / 3.0,
            xx: point.x * point.x * swept / 4.0,
            yy: point.y * point.y * swept / 4.0,
            xy: point.x * point.y * swept / 4.0,
        }
    }

    fn segment(from: Vector2, to: Vector2) -> Self {
        let swept = from.perp_dot(to);
        Self {
            perimeter: from.distance(to),
            area: swept / 2.0,
            x: swept * (from.x + to.x) / 6.0,
            y: swept * (from.y + to.y) / 6.0,
            xx: swept * (from.x * from.x + from.x * to.x + to.x * to.x) / 12.0,
            yy: swept * (from.y * from.y + from.y * to.y + to.y * to.y) / 12.0,
            xy: swept * (2.0 * from.x * from.y + from.x * to.y + to.x * from.y + 2.0 * to.x * to.y)
                / 24.0,
        }
    }

    fn sector(center: Vector2, radius: f64, from: f64, to: f64) -> Self {
        let sweep = to - from;
        let (sin_from, cos_from) = from.sin_cos();
        let (sin_to, cos_to) = to.sin_cos();
        let double_sines = (2.0 * to).sin() - (2.0 * from).sin();
        let area = radius.powi(2) / 2.0 * sweep;
        let u = radius.powi(3) / 3.0 * (sin_to - sin_from);
        let v = -radius.powi(3) / 3.0 * (cos_to - cos_from);
        let uu = radius.powi(4) / 8.0 * (sweep + double_sines / 2.0);
        let vv = radius.powi(4) / 8.0 * (sweep - double_sines / 2.0);
        let uv = radius.powi(4) / 8.0 * (sin_to * sin_to - sin_from * sin_from);
        Self {
            perimeter: radius * sweep.abs(),
            area,
            x: center.x * area + u,
            y: center.y * area + v,
            xx: center.x * center.x * area + 2.0 * center.x * u + uu,
            yy: center.y * center.y * area + 2.0 * center.y * v + vv,
            xy: center.x * center.y * area + center.x * v + center.y * u + uv,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaMoments {
    pub about_x: f64,
    pub about_y: f64,
    pub product: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrincipalMoments {
    pub major: f64,
    pub minor: f64,
    pub angle: Option<f64>,
}

impl AreaMoments {
    pub fn principal(&self) -> PrincipalMoments {
        let mean = (self.about_x + self.about_y) / 2.0;
        let half_difference = (self.about_x - self.about_y) / 2.0;
        let spread = half_difference.hypot(self.product);
        let isotropic = spread <= ISOTROPIC_SHARE * mean.abs();
        PrincipalMoments {
            major: mean + spread,
            minor: mean - spread,
            angle: (!isotropic).then(|| (-self.product).atan2(half_difference) / 2.0),
        }
    }

    pub fn polar(&self) -> f64 {
        self.about_x + self.about_y
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Section {
    pub area: f64,
    pub perimeter: f64,
    pub centroid: Point2,
    pub moments: AreaMoments,
    pub accuracy: Accuracy,
}

pub fn section_of<'a>(regions: impl IntoIterator<Item = &'a Region>) -> Option<Section> {
    let regions: Vec<&Region> = regions.into_iter().collect();
    let bounds = regions
        .iter()
        .filter_map(|region| region.bounds())
        .reduce(Aabb2::union)?;
    let reference = bounds.center();
    let size = bounds.size().length().max(LINEAR_RESOLUTION);
    let pieces: Vec<&Piece> = regions.iter().flat_map(|region| region.pieces()).collect();
    let sides: BTreeSet<(&PieceId, Side)> = pieces
        .iter()
        .map(|piece| (piece.id(), piece.side()))
        .collect();
    let mut total = Integrals::default();
    let mut accuracy = Accuracy::Exact;
    for piece in pieces {
        let (integrals, exact) = piece_integrals(piece, reference, size);
        let inside = sides.contains(&(piece.id(), opposite(piece.side())));
        total = total.plus(if inside {
            integrals.enclosed()
        } else {
            integrals
        });
        if !exact {
            accuracy = Accuracy::Approximate;
        }
    }
    let area = total.area;
    if !(area.is_finite() && area > 0.0) {
        return None;
    }
    let (x, y) = (total.x / area, total.y / area);
    Some(Section {
        area,
        perimeter: total.perimeter,
        centroid: reference + Vector2::new(x, y),
        moments: AreaMoments {
            about_x: total.yy - area * y * y,
            about_y: total.xx - area * x * x,
            product: total.xy - area * x * y,
        },
        accuracy,
    })
}

fn opposite(side: Side) -> Side {
    match side {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

fn piece_integrals(piece: &Piece, reference: Point2, size: f64) -> (Integrals, bool) {
    let relative = |point: Point2| point - reference;
    match piece.curve() {
        Curve2::Line(_) => (
            Integrals::segment(relative(piece.start()), relative(piece.end())),
            true,
        ),
        Curve2::Circle(circle) => (
            arc_integrals(
                circle,
                piece.start_parameter(),
                piece.end_parameter(),
                reference,
            ),
            true,
        ),
        curve @ Curve2::BSpline(_) => {
            let forward = adaptive(curve, piece.range(), reference, size);
            let integrals = if piece.is_reversed() {
                forward.reversed()
            } else {
                forward
            };
            (integrals, false)
        }
    }
}

fn arc_integrals(circle: &Circle2, start: f64, end: f64, reference: Point2) -> Integrals {
    let axis = circle.x_axis();
    let base = axis.y.atan2(axis.x);
    let sense = if circle.is_counter_clockwise() {
        1.0
    } else {
        -1.0
    };
    let angle = |parameter: f64| base + sense * parameter;
    let radius = circle.radius();
    let center = circle.center() - reference;
    let polar = |angle: f64| {
        let (sin, cos) = angle.sin_cos();
        center + Vector2::new(cos, sin) * radius
    };
    let (from, to) = (angle(start), angle(end));
    let (first, last) = (polar(from), polar(to));
    let sides = Integrals::segment(first, center)
        .plus(Integrals::segment(center, last))
        .enclosed();
    sides.plus(Integrals::sector(center, radius, from, to))
}

fn gauss(curve: &Curve2, low: f64, high: f64, reference: Point2) -> Integrals {
    let half = (high - low) / 2.0;
    let middle = (high + low) / 2.0;
    GAUSS_LEGENDRE
        .iter()
        .fold(Integrals::default(), |sum, (node, weight)| {
            let derivatives = curve.evaluate(middle + half * node);
            sum.plus(
                Integrals::fan(derivatives.point - reference, derivatives.first)
                    .scaled(weight * half),
            )
        })
}

fn adaptive(curve: &Curve2, range: Interval, reference: Point2, size: f64) -> Integrals {
    let length = range.length().max(f64::MIN_POSITIVE);
    let mut pending: Vec<(f64, f64, Integrals, usize)> = curve
        .seeds(range)
        .windows(2)
        .filter_map(|pair| match pair {
            [low, high] if high > low => {
                Some((*low, *high, gauss(curve, *low, *high, reference), 0))
            }
            _ => None,
        })
        .collect();
    let mut total = Integrals::default();
    while let Some((low, high, whole, depth)) = pending.pop() {
        let middle = (low + high) / 2.0;
        let left = gauss(curve, low, middle, reference);
        let right = gauss(curve, middle, high, reference);
        let halves = left.plus(right);
        let allowed = QUADRATURE_TOLERANCE * (high - low) / length;
        let settled = halves.misfit(whole, size) <= allowed;
        if settled || depth >= MAX_QUADRATURE_DEPTH || middle <= low || middle >= high {
            total = total.plus(halves);
        } else {
            pending.push((low, middle, left, depth + 1));
            pending.push((middle, high, right, depth + 1));
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::{
        profile::{Profile, ProfileCurve},
        test_support::{arc, circle, line, rectangle, spline},
    };

    fn regions(curves: &[ProfileCurve]) -> Vec<Region> {
        Profile::new(curves).unwrap().regions().to_vec()
    }

    fn only(curves: &[ProfileCurve]) -> Section {
        let regions = regions(curves);
        assert_eq!(regions.len(), 1);
        section_of(&regions).unwrap()
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1e-9 * (1.0 + expected.abs()),
            "{actual} vs {expected}"
        );
    }

    fn assert_point(actual: Point2, expected: (f64, f64)) {
        assert_close(actual.x, expected.0);
        assert_close(actual.y, expected.1);
    }

    #[test]
    fn a_rectangle_has_its_textbook_section() {
        let section = only(&rectangle(1, (10.0, 20.0), (14.0, 26.0)));

        assert_close(section.area, 24.0);
        assert_close(section.perimeter, 20.0);
        assert_point(section.centroid, (12.0, 23.0));
        assert_close(section.moments.about_x, 4.0 * 6.0_f64.powi(3) / 12.0);
        assert_close(section.moments.about_y, 6.0 * 4.0_f64.powi(3) / 12.0);
        assert_close(section.moments.product, 0.0);
        assert_close(section.moments.principal().angle.unwrap(), 0.0);
        assert_eq!(section.accuracy, Accuracy::Exact);
    }

    #[test]
    fn a_circle_is_exact_and_has_no_principal_direction() {
        let section = only(&[circle(1, (3.0, -4.0), 2.5)]);
        let principal = section.moments.principal();

        assert_close(section.area, PI * 2.5 * 2.5);
        assert_close(section.perimeter, 2.0 * PI * 2.5);
        assert_point(section.centroid, (3.0, -4.0));
        assert_close(section.moments.about_x, PI * 2.5_f64.powi(4) / 4.0);
        assert_close(section.moments.about_y, PI * 2.5_f64.powi(4) / 4.0);
        assert_close(section.moments.polar(), PI * 2.5_f64.powi(4) / 2.0);
        assert_close(principal.major, principal.minor);
        assert_eq!(principal.angle, None);
        assert_eq!(section.accuracy, Accuracy::Exact);
    }

    #[test]
    fn a_rectangle_with_a_hole_loses_the_hole() {
        let mut curves = rectangle(1, (0.0, 0.0), (20.0, 10.0));
        curves.push(circle(5, (5.0, 5.0), 2.0));
        let regions = regions(&curves);
        let plate = regions.iter().find(|region| region.depth() == 0).unwrap();

        let section = section_of([plate]).unwrap();

        let hole = PI * 4.0;
        let area = 200.0 - hole;
        let x = (200.0 * 10.0 - hole * 5.0) / area;
        assert_close(section.area, area);
        assert_close(section.perimeter, 60.0 + 4.0 * PI);
        assert_point(section.centroid, (x, 5.0));
        assert_close(
            section.moments.about_x,
            20.0 * 1000.0 / 12.0 - PI * 16.0 / 4.0,
        );
        assert_close(
            section.moments.about_y,
            10.0 * 8000.0 / 12.0 + 200.0 * (10.0 - x).powi(2)
                - (PI * 16.0 / 4.0 + hole * (5.0 - x).powi(2)),
        );
        assert_close(section.moments.product, 0.0);
    }

    #[test]
    fn an_l_section_finds_its_principal_axes() {
        let section = only(&[
            line(1, (0.0, 0.0), (4.0, 0.0)),
            line(2, (4.0, 0.0), (4.0, 1.0)),
            line(3, (4.0, 1.0), (1.0, 1.0)),
            line(4, (1.0, 1.0), (1.0, 3.0)),
            line(5, (1.0, 3.0), (0.0, 3.0)),
            line(6, (0.0, 3.0), (0.0, 0.0)),
        ]);
        let principal = section.moments.principal();

        assert_close(section.area, 6.0);
        assert_close(section.perimeter, 14.0);
        assert_point(section.centroid, (1.5, 1.0));
        assert_close(section.moments.about_x, 4.0);
        assert_close(section.moments.about_y, 8.5);
        assert_close(section.moments.product, -3.0);
        assert_close(principal.major, 10.0);
        assert_close(principal.minor, 2.5);
        assert_close(principal.angle.unwrap(), 2.0_f64.atan());
    }

    #[test]
    fn a_slot_of_two_arcs_is_exact() {
        let (half, radius) = (5.0, 2.0);
        let section = only(&[
            arc(1, (half, 0.0), (half, -radius), (half, radius)),
            line(2, (half, radius), (-half, radius)),
            arc(3, (-half, 0.0), (-half, radius), (-half, -radius)),
            line(4, (-half, -radius), (half, -radius)),
        ]);

        let length = 2.0 * half;
        let disc = PI * radius * radius;
        let offset = half + 4.0 * radius / (3.0 * PI);
        let half_disc_own = PI * radius.powi(4) / 8.0 - disc / 2.0 * (offset - half).powi(2);
        assert_close(section.area, 2.0 * radius * length + disc);
        assert_close(section.perimeter, 2.0 * length + 2.0 * PI * radius);
        assert_point(section.centroid, (0.0, 0.0));
        assert_close(
            section.moments.about_x,
            length * (2.0 * radius).powi(3) / 12.0 + PI * radius.powi(4) / 4.0,
        );
        assert_close(
            section.moments.about_y,
            2.0 * radius * length.powi(3) / 12.0
                + 2.0 * (half_disc_own + disc / 2.0 * offset * offset),
        );
        assert_close(section.moments.product, 0.0);
        assert_eq!(section.accuracy, Accuracy::Exact);
    }

    #[test]
    fn regions_side_by_side_add_up_and_their_shared_side_is_no_perimeter() {
        let mut curves = rectangle(1, (0.0, 0.0), (10.0, 4.0));
        curves.push(line(5, (6.0, -1.0), (6.0, 5.0)));
        let halves = regions(&curves);

        let both = section_of(&halves).unwrap();

        assert_eq!(halves.len(), 2);
        assert_close(both.area, 40.0);
        assert_close(both.perimeter, 28.0);
        assert_point(both.centroid, (5.0, 2.0));
        assert_close(both.moments.about_x, 10.0 * 64.0 / 12.0);
        assert_close(both.moments.about_y, 4.0 * 1000.0 / 12.0);
    }

    #[test]
    fn a_spline_side_is_integrated_within_tolerance_and_says_so() {
        let section = only(&[
            spline(1, &[(0.0, 0.0), (1.0, 2.0), (2.0, 0.0)]),
            line(2, (2.0, 0.0), (0.0, 0.0)),
        ]);
        let arch = 5.0_f64.sqrt() + (2.0 + 5.0_f64.sqrt()).ln() / 2.0;

        assert_close(section.area, 4.0 / 3.0);
        assert_close(section.perimeter, 2.0 + arch);
        assert_point(section.centroid, (1.0, 0.4));
        assert_eq!(section.accuracy, Accuracy::Approximate);
    }
}
