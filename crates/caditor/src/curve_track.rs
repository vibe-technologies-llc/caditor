use std::sync::Arc;

use caditor_geometry::{Point3, Ray, Vector3};

const SAME_POSITION: f64 = 1e-12;
const SAME_POINT: f64 = 1e-12;
const PARALLEL: f64 = 1e-18;
const ON_THE_CURVE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackPoint {
    pub position: f64,
    pub point: Point3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Reach {
    Within,
    Before,
    After,
    Both,
}

impl Reach {
    fn clamp(self, share: f64) -> f64 {
        match self {
            Self::Within => share.clamp(0.0, 1.0),
            Self::Before => share.min(1.0),
            Self::After => share.max(0.0),
            Self::Both => share,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Piece {
    from: TrackPoint,
    to: TrackPoint,
    reach: Reach,
}

impl Piece {
    fn run(&self) -> Vector3 {
        self.to.point - self.from.point
    }

    fn at_share(&self, share: f64) -> (f64, Point3) {
        (
            self.from.position + (self.to.position - self.from.position) * share,
            self.from.point + self.run() * share,
        )
    }

    fn share_of(&self, position: f64) -> f64 {
        (position - self.from.position) / (self.to.position - self.from.position)
    }

    fn nearest_to_point(&self, point: Point3) -> Option<(f64, f64)> {
        let run = self.run();
        let length = run.length_squared();
        (length > PARALLEL).then_some(())?;
        let share = self
            .reach
            .clamp((point - self.from.point).dot(run) / length);
        let (position, on) = self.at_share(share);
        Some((on.distance(point), position))
    }

    fn nearest_to_ray(&self, ray: &Ray) -> Option<(f64, f64)> {
        let run = self.run();
        let toward = ray.direction();
        let offset = self.from.point - ray.origin();
        let (a, b, d, e) = (
            run.length_squared(),
            run.dot(toward),
            run.dot(offset),
            toward.dot(offset),
        );
        (a > PARALLEL).then_some(())?;
        let apart = a - b * b;
        let free = if apart > PARALLEL * a {
            (b * e - d) / apart
        } else {
            0.0
        };
        let (position, on) = self.at_share(self.reach.clamp(free));
        let along = (on - ray.origin()).dot(toward);
        Some((on.distance(ray.at(along)), position))
    }

    fn crossing(&self, from: Point3, direction: Vector3) -> Option<f64> {
        let run = self.run();
        let normal = direction.cross(run);
        let across = normal.length_squared();
        (across > PARALLEL).then_some(())?;
        let offset = self.from.point - from;
        let along = offset.cross(run).dot(normal) / across;
        let share = offset.cross(direction).dot(normal) / across;
        let gap = (from + direction * along).distance(self.from.point + run * share);
        let inside = self.reach.clamp(share) == share;
        (inside && gap <= ON_THE_CURVE * run.length().max(1.0)).then_some(along)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CurveTrack {
    points: Arc<[TrackPoint]>,
    first: Piece,
}

impl CurveTrack {
    pub fn new(points: impl IntoIterator<Item = TrackPoint>) -> Option<Self> {
        let mut kept: Vec<TrackPoint> = points
            .into_iter()
            .filter(|point| point.position.is_finite() && point.point.is_finite())
            .collect();
        kept.sort_by(|a, b| a.position.total_cmp(&b.position));
        kept.dedup_by(|later, earlier| {
            later.position - earlier.position <= SAME_POSITION
                || later.point.distance(earlier.point) <= SAME_POINT
        });
        let (from, to) = match kept.as_slice() {
            [from, to, ..] => (*from, *to),
            _ => return None,
        };
        let reach = if kept.len() == 2 {
            Reach::Both
        } else {
            Reach::Before
        };
        Some(Self {
            points: kept.into(),
            first: Piece { from, to, reach },
        })
    }

    fn pieces(&self) -> impl Iterator<Item = Piece> + '_ {
        let last = self.points.len().saturating_sub(2);
        self.points
            .windows(2)
            .enumerate()
            .filter_map(move |(index, pair)| {
                let [from, to] = pair else {
                    return None;
                };
                let reach = match (index == 0, index == last) {
                    (true, true) => Reach::Both,
                    (true, false) => Reach::Before,
                    (false, true) => Reach::After,
                    (false, false) => Reach::Within,
                };
                Some(Piece {
                    from: *from,
                    to: *to,
                    reach,
                })
            })
    }

    fn piece_at(&self, position: f64) -> Piece {
        self.pieces()
            .find(|piece| match piece.reach {
                Reach::After | Reach::Both => true,
                Reach::Within | Reach::Before => position <= piece.to.position,
            })
            .unwrap_or(self.first)
    }

    pub fn at(&self, position: f64) -> Point3 {
        let piece = self.piece_at(position);
        piece.at_share(piece.share_of(position)).1
    }

    pub fn tangent(&self, position: f64) -> Vector3 {
        self.piece_at(position)
            .run()
            .try_normalize()
            .unwrap_or(Vector3::X)
    }

    pub fn nearest_to_ray(&self, ray: &Ray) -> Option<f64> {
        nearest(self.pieces().filter_map(|piece| piece.nearest_to_ray(ray)))
    }

    pub fn nearest_to_point(&self, point: Point3) -> Option<f64> {
        nearest(
            self.pieces()
                .filter_map(|piece| piece.nearest_to_point(point)),
        )
    }

    pub fn first_crossing(&self, from: Point3, direction: Vector3) -> Option<f64> {
        let direction = direction.try_normalize()?;
        self.pieces()
            .map(|piece| Piece {
                reach: Reach::Within,
                ..piece
            })
            .filter_map(|piece| piece.crossing(from, direction))
            .filter(|along| *along > ON_THE_CURVE)
            .min_by(f64::total_cmp)
    }
}

fn nearest(found: impl Iterator<Item = (f64, f64)>) -> Option<f64> {
    found
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, position)| position)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_2;

    use super::*;

    fn quarter_circle() -> CurveTrack {
        CurveTrack::new((0..=64).map(|index| {
            let angle = FRAC_PI_2 * f64::from(index) / 64.0;
            TrackPoint {
                position: 10.0 * angle,
                point: Point3::new(10.0 * angle.cos(), 10.0 * angle.sin(), 0.0),
            }
        }))
        .unwrap()
    }

    #[test]
    fn a_track_interpolates_between_its_points_and_runs_on_past_its_ends() {
        let track = quarter_circle();
        let end = 10.0 * FRAC_PI_2;

        assert!(track.at(0.0).distance(Point3::new(10.0, 0.0, 0.0)) < 1e-12);
        assert!(track.at(end).distance(Point3::new(0.0, 10.0, 0.0)) < 1e-12);
        assert!((track.at(end / 2.0).distance(Point3::ZERO) - 10.0).abs() < 1e-2);
        assert!(track.at(end + 5.0).distance(Point3::new(-5.0, 10.0, 0.0)) < 0.1);
        assert!(track.tangent(0.0).distance(Vector3::Y) < 0.02);
    }

    #[test]
    fn the_position_nearest_a_ray_or_a_point_is_found_along_the_whole_track() {
        let track = quarter_circle();
        let middle = Point3::new(10.0 * 0.5_f64.sqrt(), 10.0 * 0.5_f64.sqrt(), 0.0);
        let ray = Ray::new(middle + Vector3::Z * 50.0, -Vector3::Z).unwrap();

        let from_ray = track.nearest_to_ray(&ray).unwrap();
        let from_point = track.nearest_to_point(middle * 1.2).unwrap();

        assert!(
            (from_ray - 2.5 * FRAC_PI_2 * 2.0).abs() < 1e-2,
            "{from_ray}"
        );
        assert!((from_point - from_ray).abs() < 1e-2, "{from_point}");
    }

    #[test]
    fn a_line_from_inside_crosses_the_track_where_it_meets_it() {
        let track = quarter_circle();

        let along = track
            .first_crossing(Point3::ZERO, Vector3::new(1.0, 1.0, 0.0))
            .unwrap();

        assert!((along - 10.0).abs() < 1e-2, "{along}");
        assert_eq!(track.first_crossing(Point3::ZERO, -Vector3::X), None);
    }
}
