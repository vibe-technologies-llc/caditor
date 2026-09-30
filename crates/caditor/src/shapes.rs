use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{ArcGeometry, MAX_LENGTH};

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

    pub fn outline(&self, max_segment_angle: f64) -> Vec<Point2> {
        let [first, second] = self.centers;
        let [a, b, c, d] = self.corners();
        let mut outline = ArcGeometry::from_points(second, a, b).polyline(max_segment_angle);
        outline.extend(ArcGeometry::from_points(first, c, d).polyline(max_segment_angle));
        outline.push(a);
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
        let outline = slot.outline(PI / 8.0);
        assert!(
            outline
                .iter()
                .any(|point| near(*point, Point2::new(13.0, 0.0)))
        );
        assert!(
            outline
                .iter()
                .any(|point| near(*point, Point2::new(-3.0, 0.0)))
        );

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
}
