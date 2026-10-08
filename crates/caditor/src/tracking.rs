use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};

use crate::{
    snap::{self, Accept, Pointer, Screen, Target},
    trimming,
};

pub const MAX_ACQUIRED_POINTS: usize = 6;
pub const MAX_ACQUIRED_LINES: usize = 4;
pub const TRACK_TOLERANCE: f64 = 6.0;
const MIN_TRACK_LENGTH: f64 = 12.0;
const PARALLEL_TOLERANCE: f64 = 1e-9;
pub const ALIGNED_CROSSING_TOLERANCE: f64 = 12.0;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Acquired {
    points: Vec<EntityId>,
    lines: Vec<EntityId>,
}

impl Acquired {
    pub fn note(&mut self, sketch: &Sketch, target: Target) {
        match target {
            Target::Pending(_)
            | Target::Intersection(..)
            | Target::Centre { .. }
            | Target::Centroid(_) => {}
            Target::Point(point) => {
                self.point(point);
                for line in lines_ending_at(sketch, point) {
                    self.line(line);
                }
            }
            Target::Quadrant { centre, .. } => self.point(centre),
            Target::Midpoint(curve)
            | Target::Curve(curve)
            | Target::Extension(curve)
            | Target::Tangent(curve) => {
                if let Some(Entity::Circle { center, .. } | Entity::Arc { center, .. }) =
                    sketch.entity(curve)
                {
                    self.point(*center);
                }
            }
        }
    }

    pub fn point(&mut self, point: EntityId) {
        remember(&mut self.points, point, MAX_ACQUIRED_POINTS);
    }

    fn line(&mut self, line: EntityId) {
        remember(&mut self.lines, line, MAX_ACQUIRED_LINES);
    }

    pub fn retain(&mut self, sketch: &Sketch) {
        self.points.retain(|point| sketch.point(*point).is_some());
        self.lines
            .retain(|line| sketch.line_endpoints(*line).is_some());
    }

    pub fn lines(&self) -> &[EntityId] {
        &self.lines
    }

    #[cfg(test)]
    pub fn points(&self) -> &[EntityId] {
        &self.points
    }
}

fn remember(list: &mut Vec<EntityId>, id: EntityId, most: usize) {
    list.retain(|kept| *kept != id);
    list.insert(0, id);
    list.truncate(most);
}

fn lines_ending_at(sketch: &Sketch, point: EntityId) -> Vec<EntityId> {
    sketch
        .entities()
        .filter_map(|(id, entity)| match entity {
            Entity::Line { start, end } if *start == point || *end == point => Some(id),
            _ => None,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Axis {
    Level,
    Upright,
    Slanted {
        line: EntityId,
        square: bool,
        along: Vector2,
    },
}

impl Axis {
    fn along(self) -> Vector2 {
        match self {
            Self::Level => Vector2::X,
            Self::Upright => Vector2::Y,
            Self::Slanted { along, .. } => along,
        }
    }

    fn words(self, sketch: &Sketch) -> String {
        match self {
            Self::Level => "horizontal from".to_owned(),
            Self::Upright => "vertical from".to_owned(),
            Self::Slanted {
                line, square: true, ..
            } => format!("perpendicular to {} from", sketch.entity_label(line)),
            Self::Slanted {
                line,
                square: false,
                ..
            } => format!("parallel to {} from", sketch.entity_label(line)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    pub point: EntityId,
    pub from: Point2,
    pub axis: Axis,
}

impl Track {
    fn project(self, at: Point2) -> Point2 {
        match self.axis {
            Axis::Level => Point2::new(at.x, self.from.y),
            Axis::Upright => Point2::new(self.from.x, at.y),
            Axis::Slanted { along, .. } => {
                let fraction = (at - self.from).dot(along) / along.length_squared();
                self.from + along * fraction
            }
        }
    }

    fn constraint(self, point: EntityId) -> Option<Constraint> {
        match self.axis {
            Axis::Level => Some(Constraint::HorizontalPoints(point, self.point)),
            Axis::Upright => Some(Constraint::VerticalPoints(point, self.point)),
            Axis::Slanted { .. } => None,
        }
    }

    fn crossing(self, through: Point2, along: Vector2) -> Option<Point2> {
        let direction = self.axis.along();
        let denominator = along.perp_dot(direction);
        if denominator.abs() <= PARALLEL_TOLERANCE * along.length() * direction.length() {
            return None;
        }
        let fraction = direction.perp_dot(through - self.from) / denominator;
        Some(through + along * fraction)
    }

    fn meeting(self, other: Self) -> Option<Point2> {
        other
            .crossing(self.from, self.axis.along())
            .map(|crossing| self.project(crossing))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tracks {
    pub level: Option<Track>,
    pub upright: Option<Track>,
    pub slanted: Option<Track>,
}

impl Tracks {
    fn one(track: Track) -> Self {
        match track.axis {
            Axis::Level => Self {
                level: Some(track),
                ..Self::default()
            },
            Axis::Upright => Self {
                upright: Some(track),
                ..Self::default()
            },
            Axis::Slanted { .. } => Self {
                slanted: Some(track),
                ..Self::default()
            },
        }
    }

    pub fn iter(self) -> impl Iterator<Item = Track> {
        self.level
            .into_iter()
            .chain(self.upright)
            .chain(self.slanted)
    }

    pub fn points(self) -> impl Iterator<Item = EntityId> {
        self.iter().map(|track| track.point)
    }

    pub fn constraints(self, point: EntityId) -> impl Iterator<Item = Constraint> {
        self.iter().filter_map(move |track| track.constraint(point))
    }

    pub fn guides(self, to: Point2) -> impl Iterator<Item = [Point2; 2]> {
        self.iter().map(move |track| [track.from, to])
    }

    pub fn label(self, sketch: &Sketch) -> Option<String> {
        let words: Vec<String> = self
            .iter()
            .map(|track| {
                format!(
                    "{} {}",
                    track.axis.words(sketch),
                    sketch.entity_label(track.point)
                )
            })
            .collect();
        (!words.is_empty()).then(|| words.join(", "))
    }

    fn position(self, at: Point2) -> Option<Point2> {
        match (self.level, self.upright, self.slanted) {
            (Some(level), Some(upright), _) => Some(Point2::new(upright.from.x, level.from.y)),
            (Some(track), None, Some(slanted)) | (None, Some(track), Some(slanted)) => {
                Some(track.meeting(slanted).unwrap_or_else(|| track.project(at)))
            }
            (Some(track), None, None) | (None, Some(track), None) | (None, None, Some(track)) => {
                Some(track.project(at))
            }
            (None, None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tracked {
    pub position: Point2,
    pub tracks: Tracks,
}

pub fn nearby(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
    acquired: &Acquired,
    excluded: &[EntityId],
) -> Tracks {
    let mut best: [Option<(f64, Track)>; 3] = [None, None, None];
    for &point in &acquired.points {
        if excluded.contains(&point) {
            continue;
        }
        let Some(from) = sketch.point(point) else {
            continue;
        };
        let Some(from_on_screen) = screen.to_screen(from) else {
            continue;
        };
        if from_on_screen.distance(pointer.screen) < MIN_TRACK_LENGTH {
            continue;
        }
        let axes = [(0, Axis::Level), (1, Axis::Upright)]
            .into_iter()
            .chain(slanted_axes(sketch, acquired, from).map(|axis| (2, axis)));
        for (slot, axis) in axes {
            let track = Track { point, from, axis };
            let Some(on_screen) = screen.to_screen(track.project(pointer.sketch)) else {
                continue;
            };
            let offset = on_screen.distance(pointer.screen);
            let Some(kept) = best.get_mut(slot) else {
                continue;
            };
            if offset <= TRACK_TOLERANCE && kept.is_none_or(|(kept, _)| offset < kept) {
                *kept = Some((offset, track));
            }
        }
    }
    let [level, upright, slanted] = best.map(|found| found.map(|(_, track)| track));
    let shares_a_point = |track: Track| {
        level
            .into_iter()
            .chain(upright)
            .any(|other| other.point == track.point)
    };
    match (level, upright) {
        (Some(level), Some(upright)) if level.point == upright.point => Tracks::default(),
        (level, upright) => Tracks {
            level,
            upright,
            slanted: slanted.filter(|track| !shares_a_point(*track)),
        },
    }
}

fn slanted_axes(sketch: &Sketch, acquired: &Acquired, from: Point2) -> impl Iterator<Item = Axis> {
    acquired.lines.iter().flat_map(move |&line| {
        let Some(along) = sketch.line_direction(line).and_then(Vector2::try_normalize) else {
            return Vec::new();
        };
        let through = sketch.line_endpoints(line).map_or(from, |(start, _)| start);
        let on_line = along.perp_dot(from - through).abs()
            <= ON_LINE_TOLERANCE * (1.0 + through.distance(from));
        [(false, along), (true, along.perp())]
            .into_iter()
            .filter(|(square, _)| *square || !on_line)
            .filter(|(_, along)| along.x.abs() > LEVEL_TOLERANCE && along.y.abs() > LEVEL_TOLERANCE)
            .map(|(square, along)| Axis::Slanted {
                line,
                square,
                along,
            })
            .collect()
    })
}

const ON_LINE_TOLERANCE: f64 = 1e-9;
const LEVEL_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Landing {
    pub position: Point2,
    pub target: Option<Target>,
    pub tracks: Tracks,
}

impl Landing {
    pub fn joins(self, point: EntityId) -> Vec<Constraint> {
        self.target
            .map(|target| target.joins(point))
            .unwrap_or_default()
            .into_iter()
            .chain(self.tracks.constraints(point))
            .collect()
    }

    pub fn label(self, sketch: &Sketch) -> Option<String> {
        let target = self.target.map(|target| target.label(sketch));
        match (target, self.tracks.label(sketch)) {
            (target, None) => target,
            (Some(target), Some(tracked)) => Some(format!("{target}, {tracked}")),
            (None, Some(tracked)) => Some(trimming::capitalized(&tracked)),
        }
    }

    pub fn guides(self, sketch: &Sketch) -> Vec<[Point2; 2]> {
        let extension = match self.target {
            Some(Target::Extension(line)) => snap::extension_guide(sketch, line, self.position),
            _ => None,
        };
        self.tracks.guides(self.position).chain(extension).collect()
    }
}

pub fn land(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
    acquired: &Acquired,
    ignored: &[EntityId],
) -> Option<Landing> {
    let snapped = snap::resolve(
        sketch,
        screen,
        pointer,
        &[],
        Accept::Anything,
        acquired.lines(),
        ignored,
    );
    let tracks = nearby(sketch, screen, pointer, acquired, ignored);
    match snapped {
        Some(snapped) => Some(
            on_curve(
                sketch,
                snapped.target,
                tracks,
                screen,
                pointer,
                ALIGNED_CROSSING_TOLERANCE,
            )
            .map_or(
                Landing {
                    position: snapped.position,
                    target: Some(snapped.target),
                    tracks: Tracks::default(),
                },
                |tracked| Landing {
                    position: tracked.position,
                    target: Some(snapped.target),
                    tracks: tracked.tracks,
                },
            ),
        ),
        None => alone(tracks, screen, pointer).map(|tracked| Landing {
            position: tracked.position,
            target: None,
            tracks: tracked.tracks,
        }),
    }
}

pub fn alone(tracks: Tracks, screen: &impl Screen, pointer: Pointer) -> Option<Tracked> {
    let position = tracks.position(pointer.sketch)?;
    let offset = screen.to_screen(position)?.distance(pointer.screen);
    (offset <= TRACK_TOLERANCE * std::f64::consts::SQRT_2).then_some(Tracked { position, tracks })
}

pub fn on_ray(
    tracks: Tracks,
    through: Point2,
    along: Vector2,
    screen: &impl Screen,
    pointer: Pointer,
    within: f64,
) -> Option<Tracked> {
    nearest(
        tracks.iter().filter_map(|track| {
            Some(Tracked {
                position: track.crossing(through, along)?,
                tracks: Tracks::one(track),
            })
        }),
        screen,
        pointer,
        within,
    )
}

pub fn on_curve(
    sketch: &Sketch,
    target: Target,
    tracks: Tracks,
    screen: &impl Screen,
    pointer: Pointer,
    within: f64,
) -> Option<Tracked> {
    nearest(
        tracks
            .iter()
            .filter(|track| {
                target
                    .entity()
                    .is_none_or(|curve| !runs_along(sketch, curve, *track))
            })
            .filter_map(|track| {
                let position = snap::crossing_along(
                    sketch,
                    target,
                    track.from,
                    track.axis.along(),
                    pointer.sketch,
                )?;
                Some(Tracked {
                    position,
                    tracks: Tracks::one(track),
                })
            }),
        screen,
        pointer,
        within,
    )
}

fn runs_along(sketch: &Sketch, curve: EntityId, track: Track) -> bool {
    let Some(direction) = sketch.line_direction(curve) else {
        return false;
    };
    let through = sketch
        .line_endpoints(curve)
        .map_or(Point2::ZERO, |ends| ends.0);
    let length = direction.length();
    let along = track.axis.along();
    let parallel = along.perp_dot(direction).abs() <= PARALLEL_TOLERANCE * length;
    let on = direction.perp_dot(track.from - through).abs()
        <= PARALLEL_TOLERANCE * length * (1.0 + through.distance(track.from));
    parallel && on
}

fn nearest(
    candidates: impl Iterator<Item = Tracked>,
    screen: &impl Screen,
    pointer: Pointer,
    within: f64,
) -> Option<Tracked> {
    candidates
        .filter_map(|tracked| {
            let offset = screen.to_screen(tracked.position)?.distance(pointer.screen);
            (offset <= within).then_some((offset, tracked))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, tracked)| tracked)
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;

    use super::*;
    use crate::snap::tests::Scaled;

    fn pointer(sketch: Point2) -> Pointer {
        Pointer {
            screen: Scaled(10.0).to_screen(sketch).unwrap(),
            sketch,
        }
    }

    #[test]
    fn points_are_acquired_newest_first_and_forgotten_past_the_limit() {
        let mut sketch = Sketch::new(Plane::XY);
        let points: Vec<EntityId> = (0..8)
            .map(|index| sketch.add_point(Point2::new(f64::from(index), 0.0)))
            .collect();
        let mut acquired = Acquired::default();

        for point in &points {
            acquired.note(&sketch, Target::Point(*point));
        }
        acquired.note(&sketch, Target::Point(points[3]));

        assert_eq!(acquired.points().len(), MAX_ACQUIRED_POINTS);
        assert_eq!(acquired.points().first(), Some(&points[3]));
        assert!(!acquired.points().contains(&points[0]));
    }

    #[test]
    fn hovering_a_circle_acquires_its_centre_and_a_line_end_its_line() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::new(5.0, 5.0), 2.0);
        let centre = sketch.entity(circle).unwrap().points()[0];
        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let end = sketch.entity(line).unwrap().points()[1];
        let mut acquired = Acquired::default();

        acquired.note(&sketch, Target::Curve(circle));
        acquired.note(&sketch, Target::Point(end));

        assert_eq!(acquired.points(), &[end, centre]);
        assert_eq!(acquired.lines(), &[line]);

        sketch.remove_unused_entity(circle).unwrap();
        sketch.remove_unused_entity(centre).unwrap();
        acquired.retain(&sketch);

        assert_eq!(acquired.points(), &[end]);
    }

    #[test]
    fn the_pointer_tracks_acquired_points_level_upright_or_both() {
        let mut sketch = Sketch::new(Plane::XY);
        let left = sketch.add_point(Point2::new(0.0, 20.0));
        let below = sketch.add_point(Point2::new(30.0, 0.0));
        let mut acquired = Acquired::default();
        acquired.point(left);
        acquired.point(below);
        let screen = Scaled(10.0);

        let level = nearby(
            &sketch,
            &screen,
            pointer(Point2::new(15.0, 20.3)),
            &acquired,
            &[],
        );
        assert_eq!(level.level.map(|track| track.point), Some(left));
        assert_eq!(level.upright, None);
        let placed = alone(level, &screen, pointer(Point2::new(15.0, 20.3))).unwrap();
        assert_eq!(placed.position, Point2::new(15.0, 20.0));

        let corner = pointer(Point2::new(30.2, 19.6));
        let both = nearby(&sketch, &screen, corner, &acquired, &[]);
        let placed = alone(both, &screen, corner).unwrap();
        assert_eq!(placed.position, Point2::new(30.0, 20.0));
        assert_eq!(
            both.label(&sketch),
            Some(format!(
                "horizontal from Point {left}, vertical from Point {below}"
            ))
        );
        assert_eq!(
            both.constraints(EntityId::from_raw(99)).collect::<Vec<_>>(),
            vec![
                Constraint::HorizontalPoints(EntityId::from_raw(99), left),
                Constraint::VerticalPoints(EntityId::from_raw(99), below),
            ]
        );

        let excluded = nearby(&sketch, &screen, corner, &acquired, &[left]);
        assert_eq!(excluded.level, None);

        let far = nearby(
            &sketch,
            &screen,
            pointer(Point2::new(15.0, 21.0)),
            &acquired,
            &[],
        );
        assert_eq!(far, Tracks::default());
    }

    #[test]
    fn a_track_meets_a_ray_or_a_curve_but_never_runs_along_a_line() {
        let mut sketch = Sketch::new(Plane::XY);
        let tracked = sketch.add_point(Point2::new(40.0, 0.0));
        let slanted = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(60.0, 30.0));
        let level = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(80.0, 0.0));
        let track = Tracks::one(Track {
            point: tracked,
            from: Point2::new(40.0, 0.0),
            axis: Axis::Upright,
        });
        let screen = Scaled(10.0);

        let on_ray = on_ray(
            track,
            Point2::new(0.0, 10.0),
            Vector2::X,
            &screen,
            pointer(Point2::new(40.4, 10.0)),
            12.0,
        )
        .unwrap();
        assert_eq!(on_ray.position, Point2::new(40.0, 10.0));

        let crossing = on_curve(
            &sketch,
            Target::Curve(slanted),
            track,
            &screen,
            pointer(Point2::new(40.3, 20.2)),
            12.0,
        )
        .unwrap();
        assert!(crossing.position.distance(Point2::new(40.0, 20.0)) < 1e-12);

        let along = Tracks::one(Track {
            point: tracked,
            from: Point2::new(40.0, 0.0),
            axis: Axis::Level,
        });
        assert_eq!(
            on_curve(
                &sketch,
                Target::Curve(level),
                along,
                &screen,
                pointer(Point2::new(20.0, 0.0)),
                12.0
            ),
            None
        );
    }

    #[test]
    fn an_acquired_line_guides_points_square_to_it_from_its_end_and_parallel_to_it_elsewhere() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 40.0));
        let end = sketch.entity(line).unwrap().points()[1];
        let other = sketch.add_point(Point2::new(50.0, 0.0));
        let mut acquired = Acquired::default();
        acquired.note(&sketch, Target::Point(end));
        acquired.point(other);
        let screen = Scaled(10.0);
        let square = pointer(Point2::new(10.2, 55.1));
        let parallel = pointer(Point2::new(74.1, 31.9));
        let crossing = pointer(Point2::new(83.4, 0.1));

        let squared = nearby(&sketch, &screen, square, &acquired, &[]);
        let placed = alone(squared, &screen, square).unwrap();
        let along = nearby(&sketch, &screen, parallel, &acquired, &[]);
        let on_extension = nearby(
            &sketch,
            &screen,
            pointer(Point2::new(45.0, 60.1)),
            &acquired,
            &[],
        );
        let both = nearby(&sketch, &screen, crossing, &acquired, &[]);
        let met = alone(both, &screen, crossing).unwrap();

        assert_eq!(
            squared.label(&sketch),
            Some(format!(
                "perpendicular to {} from {}",
                sketch.entity_label(line),
                sketch.entity_label(end)
            ))
        );
        assert!(
            (placed.position - Point2::new(30.0, 40.0))
                .dot(Vector2::new(3.0, 4.0))
                .abs()
                < 1e-9
        );
        assert_eq!(squared.constraints(EntityId::from_raw(99)).count(), 0);
        assert!(matches!(
            along.slanted,
            Some(Track {
                axis: Axis::Slanted { square: false, .. },
                point,
                ..
            }) if point == other
        ));
        assert_eq!(on_extension.slanted, None);
        assert_eq!(both.level.map(|track| track.point), Some(other));
        assert!(met.position.distance(Point2::new(250.0 / 3.0, 0.0)) < 1e-9);
        assert_eq!(
            both.constraints(EntityId::from_raw(99)).collect::<Vec<_>>(),
            vec![Constraint::HorizontalPoints(EntityId::from_raw(99), other)]
        );
    }
}
