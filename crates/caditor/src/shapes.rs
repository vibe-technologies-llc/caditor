use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{ArcGeometry, Faceting, MAX_LENGTH};

pub const MIN_SIDES: usize = 3;
pub const MAX_SIDES: usize = 64;
pub const DEFAULT_SIDES: usize = 6;
pub const DEGENERATE_LENGTH: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Circular {
    pub center: Point2,
    pub counter_clockwise: bool,
}

impl Circular {
    pub fn arc(self, start: Point2, end: Point2) -> ArcGeometry {
        let (from, to) = if self.counter_clockwise {
            (start, end)
        } else {
            (end, start)
        };
        ArcGeometry::from_points(self.center, from, to)
    }
}

fn within_reach(center: Point2, radius: f64) -> bool {
    radius.is_finite() && radius <= MAX_LENGTH && center.abs().max_element() <= MAX_LENGTH
}

pub fn through_three(start: Point2, end: Point2, through: Point2) -> Option<Circular> {
    let to_end = end - start;
    let to_through = through - start;
    let twice_area = 2.0 * to_end.perp_dot(to_through);
    if twice_area == 0.0 {
        return None;
    }
    let offset = Vector2::new(
        to_through.y * to_end.length_squared() - to_end.y * to_through.length_squared(),
        to_end.x * to_through.length_squared() - to_through.x * to_end.length_squared(),
    ) / twice_area;
    let center = start + offset;
    within_reach(center, offset.length()).then_some(Circular {
        center,
        counter_clockwise: twice_area < 0.0,
    })
}

pub fn tangent_from(start: Point2, direction: Vector2, end: Point2) -> Option<Circular> {
    let direction = direction.try_normalize()?;
    let normal = direction.perp();
    let chord = end - start;
    let across = normal.dot(chord);
    if across == 0.0 {
        return None;
    }
    let signed_radius = chord.length_squared() / (2.0 * across);
    let center = start + normal * signed_radius;
    within_reach(center, signed_radius.abs()).then_some(Circular {
        center,
        counter_clockwise: signed_radius > 0.0,
    })
}

pub fn leaving(circular: Circular, point: Point2) -> Option<Vector2> {
    let radial = (point - circular.center).try_normalize()?;
    Some(if circular.counter_clockwise {
        radial.perp()
    } else {
        -radial.perp()
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub centers: [Point2; 2],
    pub radius: f64,
    across: Vector2,
}

impl Slot {
    pub fn new(first: Point2, second: Point2, width_point: Point2) -> Option<Self> {
        let along = (second - first).try_normalize()?;
        let across = along.perp();
        let radius = across.dot(width_point - first).abs();
        (radius >= DEGENERATE_LENGTH && within_reach(first, radius)).then_some(Self {
            centers: [first, second],
            radius,
            across,
        })
    }

    pub fn corners(&self) -> [Point2; 4] {
        let [first, second] = self.centers;
        let offset = self.across * self.radius;
        [
            second - offset,
            second + offset,
            first + offset,
            first - offset,
        ]
    }

    pub fn outline(&self, faceting: Faceting) -> Vec<Point2> {
        let [first, second] = self.centers;
        let [a, b, c, d] = self.corners();
        let mut outline = ArcGeometry::from_points(second, a, b).faceted(faceting);
        outline.extend(ArcGeometry::from_points(first, c, d).faceted(faceting));
        outline.push(a);
        outline
    }
}

pub fn mirrored(point: Point2, about: Point2) -> Point2 {
    about * 2.0 - point
}

pub fn rectangle_on_side(
    first: Point2,
    second: Point2,
    width_point: Point2,
) -> Option<[Point2; 4]> {
    let across = (second - first).try_normalize()?.perp();
    let width = across.dot(width_point - second);
    let corners = [
        first,
        second,
        second + across * width,
        first + across * width,
    ];
    let reached = corners.iter().all(|corner| within_reach(*corner, 0.0));
    (width.abs() >= DEGENERATE_LENGTH && reached).then_some(corners)
}

pub fn circle_on_diameter(first: Point2, second: Point2) -> Option<ArcGeometry> {
    let center = first.midpoint(second);
    let radius = first.distance(second) / 2.0;
    (radius >= DEGENERATE_LENGTH && within_reach(center, radius))
        .then(|| ArcGeometry::full_circle(center, radius))
}

pub fn circle_through_three(first: Point2, second: Point2, third: Point2) -> Option<ArcGeometry> {
    let circular = through_three(first, second, third)?;
    Some(ArcGeometry::full_circle(
        circular.center,
        circular.center.distance(first),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcSlot {
    pub center: Point2,
    pub ends: [Point2; 2],
    pub half_width: f64,
}

impl ArcSlot {
    pub fn new(center: Point2, ends: [Point2; 2], width_point: Point2) -> Option<Self> {
        let [first, last] = ends;
        let radius = center.distance(first);
        let half_width = (center.distance(width_point) - radius).abs();
        let slot = Self {
            center,
            ends: [first, center + (last - center).try_normalize()? * radius],
            half_width,
        };
        let inside = radius - half_width >= DEGENERATE_LENGTH;
        let ends_apart = inside && slot.sweep() + 2.0 * (half_width / radius).asin() < TAU;
        (half_width >= DEGENERATE_LENGTH && ends_apart && within_reach(center, radius + half_width))
            .then_some(slot)
    }

    fn centerline(&self) -> ArcGeometry {
        let [first, last] = self.ends;
        ArcGeometry::from_points(self.center, first, last)
    }

    fn sweep(&self) -> f64 {
        self.centerline().sweep
    }

    fn at(&self, angle: f64, radius: f64) -> Point2 {
        self.center + Vector2::from_angle(angle) * radius
    }

    pub fn corners(&self) -> [Point2; 4] {
        let arc = self.centerline();
        let (outer, inner) = (arc.radius + self.half_width, arc.radius - self.half_width);
        let (first, last) = (arc.start_angle, arc.end_angle());
        [
            self.at(first, outer),
            self.at(last, outer),
            self.at(first, inner),
            self.at(last, inner),
        ]
    }

    pub fn outline(&self, faceting: Faceting) -> Vec<Point2> {
        let [first, last] = self.ends;
        let [outer_first, outer_last, inner_first, inner_last] = self.corners();
        let mut inner =
            ArcGeometry::from_points(self.center, inner_first, inner_last).faceted(faceting);
        inner.reverse();
        let mut outline =
            ArcGeometry::from_points(self.center, outer_first, outer_last).faceted(faceting);
        outline.extend(ArcGeometry::from_points(last, outer_last, inner_last).faceted(faceting));
        outline.extend(inner);
        outline.extend(ArcGeometry::from_points(first, inner_first, outer_first).faceted(faceting));
        outline
    }
}

pub fn polygon_corners(center: Point2, corner: Point2, sides: usize) -> Vec<Point2> {
    let offset = corner - center;
    let radius = offset.length();
    let first_angle = offset.y.atan2(offset.x);
    std::iter::once(corner)
        .chain((1..sides).map(|index| {
            let angle = first_angle + TAU * index as f64 / sides as f64;
            center + Vector2::from_angle(angle) * radius
        }))
        .collect()
}

fn half_side_angle(sides: usize) -> f64 {
    PI / sides as f64
}

pub fn polygon_around_side_middle(center: Point2, middle: Point2, sides: usize) -> Vec<Point2> {
    let offset = middle - center;
    let half_side = half_side_angle(sides);
    let first = center
        + Vector2::from_angle(offset.y.atan2(offset.x) - half_side)
            * (offset.length() / half_side.cos());
    polygon_corners(center, first, sides)
}

pub fn polygon_on_side(
    first: Point2,
    second: Point2,
    sides: usize,
) -> Option<(Point2, Vec<Point2>)> {
    let side = second - first;
    let inward = side.try_normalize()?.perp();
    let inscribed = side.length() / (2.0 * half_side_angle(sides).tan());
    let center = first.midpoint(second) + inward * inscribed;
    let mut corners = polygon_corners(center, first, sides);
    if let Some(corner) = corners.get_mut(1) {
        *corner = second;
    }
    within_reach(center, center.distance(first)).then_some((center, corners))
}

pub fn polygon_name(sides: usize) -> String {
    match sides {
        3 => "triangle".to_owned(),
        4 => "square".to_owned(),
        5 => "pentagon".to_owned(),
        6 => "hexagon".to_owned(),
        7 => "heptagon".to_owned(),
        8 => "octagon".to_owned(),
        9 => "nonagon".to_owned(),
        10 => "decagon".to_owned(),
        12 => "dodecagon".to_owned(),
        sides => format!("{sides}-sided polygon"),
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use super::*;

    const TOLERANCE: f64 = 1e-9;

    fn near(a: Point2, b: Point2) -> bool {
        a.distance(b) < TOLERANCE
    }

    #[test]
    fn an_arc_through_three_points_runs_from_start_through_the_third_to_end() {
        let over = through_three(Point2::X, -Point2::X, Point2::Y).unwrap();
        assert!(near(over.center, Point2::ZERO));
        assert!(over.counter_clockwise);
        let arc = over.arc(Point2::X, -Point2::X);
        assert!((arc.sweep - PI).abs() < TOLERANCE);
        assert!(arc.start_angle.abs() < TOLERANCE);

        let under = through_three(Point2::X, -Point2::X, Point2::new(0.6, -0.8)).unwrap();
        assert!(near(under.center, Point2::ZERO));
        assert!(!under.counter_clockwise);
        let arc = under.arc(Point2::X, -Point2::X);
        assert!((arc.start_angle.abs() - PI).abs() < TOLERANCE);
        assert!((arc.sweep - PI).abs() < TOLERANCE);

        let long_way = through_three(Point2::X, Point2::Y, Point2::new(-0.6, -0.8)).unwrap();
        assert!((long_way.arc(Point2::X, Point2::Y).sweep - 3.0 * FRAC_PI_2).abs() < TOLERANCE);
    }

    #[test]
    fn three_points_in_line_or_nearly_so_make_no_arc() {
        assert_eq!(
            through_three(Point2::ZERO, Point2::X, Point2::new(0.5, 0.0)),
            None
        );
        assert_eq!(
            through_three(Point2::ZERO, Point2::X, Point2::new(3.0, 0.0)),
            None
        );
        assert_eq!(
            through_three(Point2::ZERO, Point2::X, Point2::new(0.5, 1e-12)),
            None
        );
    }

    #[test]
    fn a_tangent_arc_turns_towards_its_end_and_leaves_along_the_circle() {
        let left = tangent_from(Point2::ZERO, Point2::X, Point2::new(1.0, 1.0)).unwrap();
        assert!(near(left.center, Point2::Y));
        assert!(left.counter_clockwise);
        let direction = leaving(left, Point2::new(1.0, 1.0)).unwrap();
        assert!(near(direction, Point2::Y));

        let right = tangent_from(Point2::ZERO, Point2::X, Point2::new(1.0, -1.0)).unwrap();
        assert!(near(right.center, -Point2::Y));
        assert!(!right.counter_clockwise);
        assert!(near(
            leaving(right, Point2::new(1.0, -1.0)).unwrap(),
            -Point2::Y
        ));

        let behind = tangent_from(Point2::ZERO, Point2::X, Point2::new(-1.0, 1.0)).unwrap();
        let arc = behind.arc(Point2::ZERO, Point2::new(-1.0, 1.0));
        assert!(arc.sweep > PI);

        assert_eq!(
            tangent_from(Point2::ZERO, Point2::X, Point2::new(5.0, 0.0)),
            None
        );
        assert_eq!(tangent_from(Point2::ZERO, Point2::ZERO, Point2::Y), None);
    }

    #[test]
    fn a_slot_is_as_wide_as_twice_the_distance_of_its_third_point_from_its_axis() {
        let slot = Slot::new(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(4.0, -3.0)).unwrap();
        assert!((slot.radius - 3.0).abs() < TOLERANCE);
        let [a, b, c, d] = slot.corners();
        assert!(near(a, Point2::new(10.0, -3.0)));
        assert!(near(b, Point2::new(10.0, 3.0)));
        assert!(near(c, Point2::new(0.0, 3.0)));
        assert!(near(d, Point2::new(0.0, -3.0)));
        let outline = slot.outline(Faceting::within(1e-4));
        for bulge in [Point2::new(13.0, 0.0), Point2::new(-3.0, 0.0)] {
            assert!(outline.iter().any(|point| point.distance(bulge) < 0.05));
        }

        assert_eq!(Slot::new(Point2::ZERO, Point2::ZERO, Point2::Y), None);
        assert_eq!(
            Slot::new(Point2::ZERO, Point2::X, Point2::new(7.0, 0.0)),
            None
        );
    }

    #[test]
    fn a_polygon_starts_at_its_placed_corner_and_spaces_the_rest_evenly() {
        let corners = polygon_corners(Point2::new(1.0, 1.0), Point2::new(3.0, 1.0), 4);
        let expected = [
            Point2::new(3.0, 1.0),
            Point2::new(1.0, 3.0),
            Point2::new(-1.0, 1.0),
            Point2::new(1.0, -1.0),
        ];
        assert_eq!(corners.len(), 4);
        for (corner, expected) in corners.iter().zip(expected) {
            assert!(near(*corner, expected), "{corner}");
        }
        assert_eq!(polygon_name(6), "hexagon");
        assert_eq!(polygon_name(11), "11-sided polygon");
    }

    fn all_near(found: &[Point2], expected: &[Point2]) -> bool {
        found.len() == expected.len()
            && found
                .iter()
                .zip(expected)
                .all(|(found, expected)| near(*found, *expected))
    }

    #[test]
    fn a_rectangle_on_a_side_turns_with_it_and_takes_its_width_across_it() {
        let first = Point2::ZERO;
        let second = Point2::new(3.0, 4.0);
        let across = Vector2::new(-0.8, 0.6);

        let left = rectangle_on_side(
            first,
            second,
            second + across * 2.0 + Vector2::new(6.0, 8.0),
        );
        let right = rectangle_on_side(first, second, first - across * 3.0).unwrap();

        assert!(all_near(
            &left.unwrap(),
            &[first, second, second + across * 2.0, first + across * 2.0]
        ));
        assert!(all_near(
            &right,
            &[first, second, second - across * 3.0, first - across * 3.0]
        ));
        assert_eq!(
            rectangle_on_side(first, second, Point2::new(6.0, 8.0)),
            None
        );
        assert_eq!(rectangle_on_side(first, first, Point2::Y), None);
    }

    #[test]
    fn a_circle_on_a_diameter_is_centred_between_its_ends() {
        let circle = circle_on_diameter(Point2::new(2.0, 1.0), Point2::new(12.0, 1.0)).unwrap();

        assert!(near(circle.center, Point2::new(7.0, 1.0)));
        assert!((circle.radius - 5.0).abs() < TOLERANCE);
        assert!((circle.sweep - TAU).abs() < TOLERANCE);
        assert_eq!(circle_on_diameter(Point2::X, Point2::X), None);
    }

    #[test]
    fn a_circle_through_three_points_passes_through_each_unless_they_are_in_line() {
        let points = [
            Point2::new(10.0, 0.0),
            Point2::new(0.0, 10.0),
            Point2::new(-6.0, -8.0),
        ];

        let circle = circle_through_three(points[0], points[1], points[2]).unwrap();

        assert!(near(circle.center, Point2::ZERO), "{}", circle.center);
        for point in points {
            assert!((circle.center.distance(point) - circle.radius).abs() < TOLERANCE);
        }
        assert_eq!(
            circle_through_three(Point2::ZERO, Point2::X, Point2::new(2.0, 0.0)),
            None
        );
        assert_eq!(circle_through_three(Point2::X, Point2::X, Point2::Y), None);
    }

    #[test]
    fn a_polygon_around_a_side_middle_puts_the_middle_of_its_first_side_there() {
        let square = polygon_around_side_middle(Point2::ZERO, Point2::new(0.0, -5.0), 4);
        let hexagon = polygon_around_side_middle(Point2::new(1.0, 2.0), Point2::new(4.0, 6.0), 6);

        assert!(all_near(
            &square,
            &[
                Point2::new(-5.0, -5.0),
                Point2::new(5.0, -5.0),
                Point2::new(5.0, 5.0),
                Point2::new(-5.0, 5.0),
            ]
        ));
        assert!(near(hexagon[0].midpoint(hexagon[1]), Point2::new(4.0, 6.0)));
        for (index, corner) in hexagon.iter().enumerate() {
            let next = hexagon[(index + 1) % hexagon.len()];
            let middle = corner.midpoint(next);
            assert!((middle.distance(Point2::new(1.0, 2.0)) - 5.0).abs() < TOLERANCE);
        }
    }

    #[test]
    fn a_polygon_on_a_side_lies_to_its_left_starting_with_that_side() {
        let (center, square) = polygon_on_side(Point2::ZERO, Point2::new(10.0, 0.0), 4).unwrap();
        let (hexagon_center, hexagon) =
            polygon_on_side(Point2::new(10.0, 0.0), Point2::ZERO, 6).unwrap();

        assert!(near(center, Point2::new(5.0, 5.0)), "{center}");
        assert!(all_near(
            &square,
            &[
                Point2::ZERO,
                Point2::new(10.0, 0.0),
                Point2::new(10.0, 10.0),
                Point2::new(0.0, 10.0),
            ]
        ));
        assert!(hexagon_center.y < 0.0);
        assert_eq!(hexagon[1], Point2::ZERO);
        for (index, corner) in hexagon.iter().enumerate() {
            let next = hexagon[(index + 1) % hexagon.len()];
            assert!((corner.distance(next) - 10.0).abs() < TOLERANCE);
            assert!((corner.distance(hexagon_center) - 10.0).abs() < TOLERANCE);
        }
        assert_eq!(polygon_on_side(Point2::X, Point2::X, 5), None);
    }

    #[test]
    fn an_arc_slot_is_two_concentric_arcs_joined_by_ends_that_bulge_outwards() {
        let slot = ArcSlot::new(
            Point2::ZERO,
            [Point2::new(10.0, 0.0), Point2::new(0.0, 20.0)],
            Point2::new(12.0, 0.0),
        )
        .unwrap();

        let outline = slot.outline(Faceting::within(1e-4));
        let reaches = |bulge: Point2| outline.iter().any(|point| point.distance(bulge) < 0.06);

        assert!(near(slot.ends[1], Point2::new(0.0, 10.0)));
        assert!((slot.half_width - 2.0).abs() < TOLERANCE);
        assert!(all_near(
            &slot.corners(),
            &[
                Point2::new(12.0, 0.0),
                Point2::new(0.0, 12.0),
                Point2::new(8.0, 0.0),
                Point2::new(0.0, 8.0),
            ]
        ));
        for bulge in [Point2::new(10.0, -2.0), Point2::new(-2.0, 10.0)] {
            assert!(reaches(bulge), "{bulge}");
        }
        assert!(near(outline[0], *outline.last().unwrap()));
    }

    #[test]
    fn an_arc_slot_needs_a_width_narrower_than_its_arc_and_ends_that_do_not_meet() {
        let ends = [Point2::new(10.0, 0.0), Point2::new(0.0, 10.0)];
        let nearly_round = [Point2::new(10.0, 0.0), Point2::from_angle(-0.1) * 10.0];

        assert_eq!(
            ArcSlot::new(Point2::ZERO, ends, Point2::new(0.0, -10.0)),
            None
        );
        assert_eq!(
            ArcSlot::new(Point2::ZERO, ends, Point2::new(21.0, 0.0)),
            None
        );
        assert_eq!(ArcSlot::new(Point2::ZERO, ends, Point2::ZERO), None);
        assert_eq!(
            ArcSlot::new(Point2::ZERO, nearly_round, Point2::new(12.0, 0.0)),
            None
        );
        assert!(ArcSlot::new(Point2::ZERO, nearly_round, Point2::new(10.4, 0.0)).is_some());
        assert_eq!(
            ArcSlot::new(Point2::ZERO, [Point2::X, Point2::ZERO], Point2::ZERO),
            None
        );
    }
}
