use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};

use crate::{
    body_snap::BodySnaps,
    inference::Inferences,
    snap::{
        self, Accept, AxisEnd, Held, Hold, Lookup, NOT_KEPT_THERE, ON_THE_GRID, Pointer, Screen,
        Snapped, Target,
    },
    trimming,
};

pub const MAX_ACQUIRED_POINTS: usize = 6;
pub const MAX_ACQUIRED_LINES: usize = 4;
pub const TRACK_TOLERANCE: f64 = 6.0;
const MIN_TRACK_LENGTH: f64 = 12.0;
const PARALLEL_TOLERANCE: f64 = 1e-9;
pub const ALIGNED_CROSSING_TOLERANCE: f64 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Anchor {
    target: Target,
    near: Point2,
}

impl Anchor {
    fn is(self, other: Self) -> bool {
        let same_place = || {
            self.near.distance(other.near)
                <= SAME_PLACE_TOLERANCE * (1.0 + self.near.abs().max_element())
        };
        self.target == other.target && (!self.target.has_several_places() || same_place())
    }
}

const SAME_PLACE_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Acquired {
    anchors: Vec<Anchor>,
    lines: Vec<EntityId>,
}

impl Acquired {
    pub fn note(&mut self, sketch: &Sketch, target: Target, at: Point2) {
        match target {
            Target::Pending(_) | Target::Body(_) => {}
            Target::Point(point) => {
                self.anchor(target, at);
                for line in lines_ending_at(sketch, point) {
                    self.line(line);
                }
            }
            Target::Quadrant { centre, .. } | Target::AxisEnd { centre, .. } => {
                self.point(centre);
                self.anchor(target, at);
            }
            Target::Midpoint(curve) => {
                self.centre_of(sketch, curve);
                self.anchor(target, at);
            }
            Target::Intersection(..) | Target::Centre { .. } | Target::Centroid(_) => {
                self.anchor(target, at);
            }
            Target::Curve(curve) | Target::Extension(curve) | Target::Tangent(curve) => {
                self.centre_of(sketch, curve);
            }
        }
    }

    fn centre_of(&mut self, sketch: &Sketch, curve: EntityId) {
        if let Some(
            Entity::Circle { center, .. }
            | Entity::Arc { center, .. }
            | Entity::Ellipse { center, .. }
            | Entity::EllipticalArc { center, .. },
        ) = sketch.entity(curve)
        {
            self.point(*center);
        }
    }

    pub fn point(&mut self, point: EntityId) {
        self.anchor(Target::Point(point), Point2::ZERO);
    }

    fn anchor(&mut self, target: Target, near: Point2) {
        let anchor = Anchor { target, near };
        self.anchors.retain(|kept| !kept.is(anchor));
        self.anchors.insert(0, anchor);
        self.anchors.truncate(MAX_ACQUIRED_POINTS);
    }

    fn line(&mut self, line: EntityId) {
        self.lines.retain(|kept| *kept != line);
        self.lines.insert(0, line);
        self.lines.truncate(MAX_ACQUIRED_LINES);
    }

    pub fn retain(&mut self, sketch: &Sketch) {
        self.anchors.retain_mut(|anchor| {
            let defined = anchor
                .target
                .involved()
                .all(|entity| sketch.contains(entity));
            if defined
                && anchor.target.has_several_places()
                && let Some(at) = snap::position_of(sketch, anchor.target, anchor.near, &[])
            {
                anchor.near = at;
            }
            defined
        });
        self.lines
            .retain(|line| sketch.line_endpoints(*line).is_some());
    }

    pub fn lines(&self) -> &[EntityId] {
        &self.lines
    }

    #[cfg(test)]
    pub fn targets(&self) -> Vec<Target> {
        self.anchors.iter().map(|anchor| anchor.target).collect()
    }
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
    pub anchor: Target,
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

    fn constraint(self, sketch: &Sketch, point: EntityId) -> Option<Constraint> {
        let through = self.anchor.point_target(sketch, self.from)?;
        Some(match self.axis {
            Axis::Level => Constraint::HorizontalPoints(point, through),
            Axis::Upright => Constraint::VerticalPoints(point, through),
            Axis::Slanted { line, square, .. } => Constraint::OnDirection {
                point,
                through,
                line,
                square,
            },
        })
    }

    fn is_held(self, sketch: &Sketch) -> bool {
        self.anchor.point_target(sketch, self.from).is_some()
    }

    fn shares_anchor(self, other: Self) -> bool {
        self.anchor == other.anchor && self.from == other.from
    }

    fn words(self, sketch: &Sketch) -> String {
        match (self.anchor, self.axis) {
            (
                Target::Midpoint(middle),
                Axis::Slanted {
                    line, square: true, ..
                },
            ) if middle == line => {
                format!(
                    "on the perpendicular bisector of {}",
                    sketch.entity_label(line)
                )
            }
            (anchor, axis) => format!("{} {}", axis.words(sketch), anchor_words(sketch, anchor)),
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

    pub fn anchors(self) -> impl Iterator<Item = Target> {
        self.iter().map(|track| track.anchor)
    }

    pub fn constraints(self, sketch: &Sketch, point: EntityId) -> impl Iterator<Item = Constraint> {
        self.iter()
            .filter_map(move |track| track.constraint(sketch, point))
    }

    pub fn guides(self, to: Point2) -> impl Iterator<Item = [Point2; 2]> {
        self.iter().map(move |track| [track.from, to])
    }

    pub fn label(self, sketch: &Sketch) -> Option<String> {
        let (held, loose): (Vec<Track>, Vec<Track>) =
            self.iter().partition(|track| track.is_held(sketch));
        let words: Vec<String> = held
            .iter()
            .chain(&loose)
            .map(|track| track.words(sketch))
            .collect();
        let kept = if loose.is_empty() { "" } else { NOT_KEPT_THERE };
        (!words.is_empty()).then(|| format!("{}{kept}", words.join(", ")))
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
    ignored: &[EntityId],
    own: &[Target],
) -> Tracks {
    let mut best: [Option<(f64, Track)>; 3] = [None, None, None];
    for anchor in &acquired.anchors {
        if own.contains(&anchor.target) {
            continue;
        }
        let Some(from) = snap::position_of(sketch, anchor.target, anchor.near, ignored) else {
            continue;
        };
        let Some(from_on_screen) = screen.to_screen(from) else {
            continue;
        };
        if from_on_screen.distance(pointer.screen) < MIN_TRACK_LENGTH {
            continue;
        }
        let axes = [(0, Axis::Level), (1, Axis::Upright)].into_iter().chain(
            slanted_axes(sketch, acquired, from)
                .chain(bisector(sketch, anchor.target))
                .map(|axis| (2, axis)),
        );
        for (slot, axis) in axes {
            let track = Track {
                anchor: anchor.target,
                from,
                axis,
            };
            let Some(on_screen) = screen.to_screen(track.project(pointer.sketch)) else {
                continue;
            };
            let offset = on_screen.distance(pointer.screen);
            let Some(kept) = best.get_mut(slot) else {
                continue;
            };
            if offset <= TRACK_TOLERANCE
                && kept.is_none_or(|kept| prefers(sketch, (offset, track), kept))
            {
                *kept = Some((offset, track));
            }
        }
    }
    let [level, upright, slanted] = best.map(|found| found.map(|(_, track)| track));
    let shares_an_anchor = |track: Track| {
        level
            .into_iter()
            .chain(upright)
            .any(|other| other.shares_anchor(track))
    };
    match (level, upright) {
        (Some(level), Some(upright)) if level.shares_anchor(upright) => Tracks::default(),
        (level, upright) => Tracks {
            level,
            upright,
            slanted: slanted.filter(|track| !shares_an_anchor(*track)),
        },
    }
}

fn prefers(
    sketch: &Sketch,
    (offset, track): (f64, Track),
    (kept_offset, kept): (f64, Track),
) -> bool {
    if (offset - kept_offset).abs() <= TIED_OFFSET {
        let rank = |track: Track| {
            (
                track.is_held(sketch),
                matches!(track.anchor, Target::Point(_)),
            )
        };
        rank(track) > rank(kept)
    } else {
        offset < kept_offset
    }
}

const TIED_OFFSET: f64 = 1e-6;

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
            .filter(|(_, along)| is_slanted(*along))
            .map(|(square, along)| Axis::Slanted {
                line,
                square,
                along,
            })
            .collect()
    })
}

fn bisector(sketch: &Sketch, anchor: Target) -> Option<Axis> {
    let Target::Midpoint(line) = anchor else {
        return None;
    };
    let (start, end) = sketch.line_endpoints(line)?;
    let along = (end - start).try_normalize()?.perp();
    is_slanted(along).then_some(Axis::Slanted {
        line,
        square: true,
        along,
    })
}

fn is_slanted(along: Vector2) -> bool {
    along.x.abs() > LEVEL_TOLERANCE && along.y.abs() > LEVEL_TOLERANCE
}

fn anchor_words(sketch: &Sketch, anchor: Target) -> String {
    match anchor {
        Target::Point(point) => sketch.entity_label(point),
        Target::Midpoint(curve) => format!("the midpoint of {}", sketch.entity_label(curve)),
        Target::Quadrant { curve, side, .. } => format!(
            "the {} of {}",
            side.name().to_lowercase(),
            sketch.entity_label(curve)
        ),
        Target::AxisEnd {
            curve,
            end: AxisEnd::FarMajor,
            ..
        } => format!(
            "the far end of the major axis of {}",
            sketch.entity_label(curve)
        ),
        Target::AxisEnd { curve, .. } => {
            format!("an end of the minor axis of {}", sketch.entity_label(curve))
        }
        Target::Intersection(first, second) => format!(
            "the crossing of {} and {}",
            sketch.entity_label(first),
            sketch.entity_label(second)
        ),
        Target::Centre { outline, .. } | Target::Centroid(outline) => format!(
            "the centre of the outline of {}",
            sketch.entity_label(outline)
        ),
        Target::Pending(_)
        | Target::Curve(_)
        | Target::Extension(_)
        | Target::Tangent(_)
        | Target::Body(_) => anchor.label(sketch),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShownOutlines(Vec<(EntityId, Vec<EntityId>)>);

impl ShownOutlines {
    pub fn keep(&mut self, sketch: &Sketch, targets: impl IntoIterator<Item = Target>) {
        let wanted: Vec<EntityId> = targets.into_iter().filter_map(outline_of).collect();
        self.0.retain(|(outline, _)| wanted.contains(outline));
        for outline in wanted {
            if !self.0.iter().any(|(kept, _)| *kept == outline) {
                let pieces =
                    snap::outline_through(sketch, outline).unwrap_or_else(|| vec![outline]);
                self.0.push((outline, pieces));
            }
        }
    }

    pub fn keep_none(&mut self) {
        self.0.clear();
    }

    fn add_pieces(&self, outline: EntityId, shown: &mut Vec<EntityId>) {
        match self.0.iter().find(|(kept, _)| *kept == outline) {
            Some((_, pieces)) => shown.extend_from_slice(pieces),
            None => shown.push(outline),
        }
    }

    pub fn show(&self, target: Target, shown: &mut Vec<EntityId>) {
        match target {
            Target::Centre {
                outline,
                corners: (first, second),
            } => {
                self.add_pieces(outline, shown);
                shown.extend([first, second]);
            }
            Target::Centroid(outline) => self.add_pieces(outline, shown),
            Target::Pending(_)
            | Target::Point(_)
            | Target::Curve(_)
            | Target::Extension(_)
            | Target::Midpoint(_)
            | Target::Quadrant { .. }
            | Target::AxisEnd { .. }
            | Target::Tangent(_)
            | Target::Intersection(..)
            | Target::Body(_) => {
                shown.extend(target.entity().into_iter().chain(target.second_entity()))
            }
        }
    }
}

fn outline_of(target: Target) -> Option<EntityId> {
    match target {
        Target::Centre { outline, .. } | Target::Centroid(outline) => Some(outline),
        Target::Pending(_)
        | Target::Point(_)
        | Target::Curve(_)
        | Target::Extension(_)
        | Target::Midpoint(_)
        | Target::Quadrant { .. }
        | Target::AxisEnd { .. }
        | Target::Tangent(_)
        | Target::Intersection(..)
        | Target::Body(_) => None,
    }
}

const ON_LINE_TOLERANCE: f64 = 1e-9;
const LEVEL_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Landing {
    pub position: Point2,
    pub target: Option<Target>,
    pub tracks: Tracks,
    pub on_grid: bool,
}

impl Landing {
    pub fn joins(self, sketch: &Sketch, point: EntityId) -> Vec<Constraint> {
        self.target
            .map(|target| target.joins(sketch, point))
            .unwrap_or_default()
            .into_iter()
            .chain(self.tracks.constraints(sketch, point))
            .collect()
    }

    pub fn label(self, sketch: &Sketch) -> Option<String> {
        let target = self.target.map(|target| target.label(sketch));
        match (target, self.tracks.label(sketch)) {
            (None, None) if self.on_grid => Some(ON_THE_GRID.to_owned()),
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
    (acquired, ignored): (&Acquired, &[EntityId]),
    hold: Option<Hold>,
    bodies: Option<&BodySnaps>,
) -> Option<Landing> {
    if let Some(hold) = hold {
        let lookup = Lookup {
            pending: &[],
            accept: Accept::Anything,
            extended: acquired.lines(),
            ignored,
            bodies,
            inferred: Inferences::default(),
        };
        return land_held(sketch, screen, pointer, lookup, hold);
    }
    let landing = land_freely(sketch, screen, pointer, acquired, ignored);
    match bodies {
        Some(bodies) => onto_body(bodies, landing, screen, pointer),
        None => landing,
    }
}

fn onto_body(
    bodies: &BodySnaps,
    landing: Option<Landing>,
    screen: &impl Screen,
    pointer: Pointer,
) -> Option<Landing> {
    let on_body = |snapped: Snapped| Landing {
        position: snapped.position,
        target: Some(snapped.target),
        tracks: Tracks::default(),
        on_grid: false,
    };
    match landing {
        Some(landing) if landing.target.is_some_and(Target::is_point_like) => Some(landing),
        Some(landing) => bodies
            .point(screen, pointer, Accept::Anything, Inferences::default())
            .map(on_body)
            .or(Some(landing)),
        None => bodies
            .point(screen, pointer, Accept::Anything, Inferences::default())
            .or_else(|| bodies.edge(screen, pointer, Accept::Anything, Inferences::default()))
            .map(on_body),
    }
}

fn land_freely(
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
    let tracks = nearby(sketch, screen, pointer, acquired, ignored, &[]);
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
                    on_grid: false,
                },
                |tracked| Landing {
                    position: tracked.position,
                    target: Some(snapped.target),
                    tracks: tracked.tracks,
                    on_grid: false,
                },
            ),
        ),
        None => alone(tracks, screen, pointer).map(|tracked| Landing {
            position: tracked.position,
            target: None,
            tracks: tracked.tracks,
            on_grid: false,
        }),
    }
}

fn land_held(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
    lookup: Lookup,
    hold: Hold,
) -> Option<Landing> {
    let landing = match snap::held(sketch, screen, pointer, lookup, hold.grid)? {
        Held::Target(snapped) => Landing {
            position: snapped.position,
            target: Some(snapped.target),
            tracks: Tracks::default(),
            on_grid: false,
        },
        Held::Grid(position) => Landing {
            position,
            target: None,
            tracks: Tracks::default(),
            on_grid: true,
        },
    };
    Some(landing)
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
    use caditor_sketch::{PointTarget, Quadrant};

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
            acquired.note(&sketch, Target::Point(*point), Point2::ZERO);
        }
        acquired.note(&sketch, Target::Point(points[3]), Point2::ZERO);

        assert_eq!(acquired.targets().len(), MAX_ACQUIRED_POINTS);
        assert_eq!(acquired.targets().first(), Some(&Target::Point(points[3])));
        assert!(!acquired.targets().contains(&Target::Point(points[0])));
    }

    #[test]
    fn hovering_a_circle_acquires_its_centre_and_a_line_end_its_line() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::new(5.0, 5.0), 2.0);
        let centre = sketch.entity(circle).unwrap().points()[0];
        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let end = sketch.entity(line).unwrap().points()[1];
        let mut acquired = Acquired::default();

        acquired.note(&sketch, Target::Curve(circle), Point2::new(7.0, 5.0));
        acquired.note(&sketch, Target::Point(end), Point2::new(10.0, 0.0));

        assert_eq!(
            acquired.targets(),
            vec![Target::Point(end), Target::Point(centre)]
        );
        assert_eq!(acquired.lines(), &[line]);

        sketch.remove_unused_entity(circle).unwrap();
        sketch.remove_unused_entity(centre).unwrap();
        acquired.retain(&sketch);

        assert_eq!(acquired.targets(), vec![Target::Point(end)]);
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
            &[],
        );
        assert_eq!(
            level.level.map(|track| track.anchor),
            Some(Target::Point(left))
        );
        assert_eq!(level.upright, None);
        let placed = alone(level, &screen, pointer(Point2::new(15.0, 20.3))).unwrap();
        assert_eq!(placed.position, Point2::new(15.0, 20.0));

        let corner = pointer(Point2::new(30.2, 19.6));
        let both = nearby(&sketch, &screen, corner, &acquired, &[], &[]);
        let placed = alone(both, &screen, corner).unwrap();
        assert_eq!(placed.position, Point2::new(30.0, 20.0));
        assert_eq!(
            both.label(&sketch),
            Some(format!(
                "horizontal from Point {left}, vertical from Point {below}"
            ))
        );
        assert_eq!(
            both.constraints(&sketch, EntityId::from_raw(99))
                .collect::<Vec<_>>(),
            vec![
                Constraint::HorizontalPoints(EntityId::from_raw(99), left.into()),
                Constraint::VerticalPoints(EntityId::from_raw(99), below.into()),
            ]
        );

        let excluded = nearby(&sketch, &screen, corner, &acquired, &[left], &[]);
        assert_eq!(excluded.level, None);

        let far = nearby(
            &sketch,
            &screen,
            pointer(Point2::new(15.0, 21.0)),
            &acquired,
            &[],
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
            anchor: Target::Point(tracked),
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
            anchor: Target::Point(tracked),
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
        acquired.note(&sketch, Target::Point(end), Point2::new(30.0, 40.0));
        acquired.point(other);
        let screen = Scaled(10.0);
        let square = pointer(Point2::new(10.2, 55.1));
        let parallel = pointer(Point2::new(74.1, 31.9));
        let crossing = pointer(Point2::new(83.4, 0.1));

        let squared = nearby(&sketch, &screen, square, &acquired, &[], &[]);
        let placed = alone(squared, &screen, square).unwrap();
        let along = nearby(&sketch, &screen, parallel, &acquired, &[], &[]);
        let on_extension = nearby(
            &sketch,
            &screen,
            pointer(Point2::new(45.0, 60.1)),
            &acquired,
            &[],
            &[],
        );
        let both = nearby(&sketch, &screen, crossing, &acquired, &[], &[]);
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
        assert_eq!(
            squared
                .constraints(&sketch, EntityId::from_raw(99))
                .collect::<Vec<_>>(),
            vec![Constraint::OnDirection {
                point: EntityId::from_raw(99),
                through: PointTarget::Point(end),
                line,
                square: true,
            }]
        );
        assert!(matches!(
            along.slanted,
            Some(Track {
                axis: Axis::Slanted { square: false, .. },
                anchor: Target::Point(point),
                ..
            }) if point == other
        ));
        assert_eq!(on_extension.slanted, None);
        assert_eq!(
            both.level.map(|track| track.anchor),
            Some(Target::Point(other))
        );
        assert!(met.position.distance(Point2::new(250.0 / 3.0, 0.0)) < 1e-9);
        assert_eq!(
            both.constraints(&sketch, EntityId::from_raw(99))
                .collect::<Vec<_>>(),
            vec![
                Constraint::HorizontalPoints(EntityId::from_raw(99), other.into()),
                Constraint::OnDirection {
                    point: EntityId::from_raw(99),
                    through: PointTarget::Point(end),
                    line,
                    square: true,
                },
            ]
        );
    }

    fn tracks_at(sketch: &Sketch, acquired: &Acquired, at: Point2) -> Tracks {
        nearby(sketch, &Scaled(10.0), pointer(at), acquired, &[], &[])
    }

    fn placed_at(sketch: &Sketch, acquired: &Acquired, at: Point2) -> Option<Tracked> {
        alone(tracks_at(sketch, acquired, at), &Scaled(10.0), pointer(at))
    }

    #[test]
    fn the_middle_of_a_line_tracks_level_upright_and_along_its_perpendicular_bisector() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 40.0));
        let middle = Point2::new(15.0, 20.0);
        let mut acquired = Acquired::default();
        let label = sketch.entity_label(line);

        acquired.note(&sketch, Target::Midpoint(line), middle);
        let level = placed_at(&sketch, &acquired, Point2::new(45.0, 20.3)).unwrap();
        let upright = placed_at(&sketch, &acquired, Point2::new(15.3, -10.0)).unwrap();
        let bisecting = placed_at(&sketch, &acquired, Point2::new(-5.2, 35.1)).unwrap();

        assert_eq!(acquired.targets(), vec![Target::Midpoint(line)]);
        assert_eq!(level.position, Point2::new(45.0, 20.0));
        assert_eq!(
            level.tracks.label(&sketch),
            Some(format!("horizontal from the midpoint of {label}"))
        );
        assert_eq!(
            level
                .tracks
                .constraints(&sketch, EntityId::from_raw(99))
                .collect::<Vec<_>>(),
            vec![Constraint::HorizontalPoints(
                EntityId::from_raw(99),
                PointTarget::Middle(line)
            )]
        );
        assert_eq!(upright.position, Point2::new(15.0, -10.0));
        assert!(
            (bisecting.position - middle)
                .dot(Vector2::new(3.0, 4.0))
                .abs()
                < 1e-9
        );
        assert_eq!(
            bisecting.tracks.label(&sketch),
            Some(format!("on the perpendicular bisector of {label}"))
        );
        assert_eq!(
            bisecting
                .tracks
                .constraints(&sketch, EntityId::from_raw(99))
                .collect::<Vec<_>>(),
            vec![Constraint::OnDirection {
                point: EntityId::from_raw(99),
                through: PointTarget::Middle(line),
                line,
                square: true,
            }]
        );
        assert_eq!(
            bisecting.tracks.anchors().collect::<Vec<_>>(),
            vec![Target::Midpoint(line)]
        );
    }

    #[test]
    fn a_level_line_has_its_bisector_as_the_upright_track_through_its_middle() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(50.0, 10.0));
        let mut acquired = Acquired::default();

        acquired.note(&sketch, Target::Midpoint(line), Point2::new(30.0, 10.0));
        let above = tracks_at(&sketch, &acquired, Point2::new(30.2, 40.0));

        assert_eq!(
            above.upright.map(|track| track.anchor),
            Some(Target::Midpoint(line))
        );
        assert_eq!(above.slanted, None);
    }

    #[test]
    fn crossings_outline_centres_and_axis_ends_are_acquired_and_tracked() {
        let mut sketch = Sketch::new(Plane::XY);
        let rising = sketch.add_line(Point2::ZERO, Point2::new(30.0, 30.0));
        let falling = sketch.add_line(Point2::new(0.0, 20.0), Point2::new(30.0, -10.0));
        let ellipse = sketch.add_ellipse(Point2::new(100.0, 0.0), Point2::new(110.0, 10.0), 4.0);
        let corners = [
            Point2::new(40.0, 40.0),
            Point2::new(60.0, 40.0),
            Point2::new(60.0, 50.0),
            Point2::new(40.0, 50.0),
        ];
        let sides: Vec<EntityId> = corners
            .iter()
            .zip(corners.iter().cycle().skip(1))
            .map(|(from, to)| sketch.add_line(*from, *to))
            .collect();
        let at = |position: Point2| {
            snap::resolve(
                &sketch,
                &Scaled(10.0),
                pointer(position),
                &[],
                Accept::Anything,
                &[],
                &[],
            )
            .unwrap()
        };
        let crossing = at(Point2::new(10.3, 10.4));
        let centre = at(Point2::new(50.2, 45.3));
        let far_end = at(Point2::new(90.2, -9.8));
        let mut acquired = Acquired::default();

        for snapped in [crossing, centre, far_end] {
            acquired.note(&sketch, snapped.target, snapped.position);
        }
        let level_with_crossing = placed_at(&sketch, &acquired, Point2::new(-30.0, 10.2)).unwrap();
        let below_centre = placed_at(&sketch, &acquired, Point2::new(50.3, -40.0)).unwrap();
        let level_with_far_end = placed_at(&sketch, &acquired, Point2::new(150.0, -10.4)).unwrap();
        let met = placed_at(&sketch, &acquired, Point2::new(10.2, 45.1)).unwrap();

        assert!(matches!(crossing.target, Target::Intersection(..)));
        assert!(
            matches!(centre.target, Target::Centre { outline, .. } if sides.contains(&outline))
        );
        assert!(matches!(
            far_end.target,
            Target::AxisEnd { curve, end: AxisEnd::FarMajor, .. } if curve == ellipse
        ));
        assert_eq!(level_with_crossing.position, Point2::new(-30.0, 10.0));
        assert_eq!(
            level_with_crossing.tracks.label(&sketch),
            Some(format!(
                "horizontal from the crossing of {} and {}",
                sketch.entity_label(rising),
                sketch.entity_label(falling)
            ))
        );
        assert_eq!(below_centre.position, Point2::new(50.0, -40.0));
        assert!(
            level_with_far_end
                .position
                .distance(Point2::new(150.0, -10.0))
                < 1e-9
        );
        assert!(met.position.distance(Point2::new(10.0, 45.0)) < 1e-9);
        let met_constraints: Vec<Constraint> = met
            .tracks
            .constraints(&sketch, EntityId::from_raw(99))
            .collect();
        assert_eq!(met_constraints.len(), 2);
        assert!(met_constraints.contains(&Constraint::VerticalPoints(
            EntityId::from_raw(99),
            PointTarget::Crossing(rising, falling)
        )));

        let ellipse_centre = sketch.entity(ellipse).unwrap().points()[0];
        sketch.remove_unused_entity(falling).unwrap();
        acquired.retain(&sketch);

        assert_eq!(
            acquired.targets(),
            vec![far_end.target, Target::Point(ellipse_centre), centre.target]
        );
    }

    #[test]
    fn both_crossings_of_a_line_and_a_circle_are_kept_apart_and_each_tracked() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(-20.0, 0.0), Point2::new(20.0, 0.0));
        let circle = sketch.add_circle(Point2::ZERO, 10.0);
        let crossing = Target::Intersection(line.min(circle), line.max(circle));
        let mut acquired = Acquired::default();

        acquired.note(&sketch, crossing, Point2::new(10.0, 0.0));
        acquired.note(&sketch, crossing, Point2::new(-10.0, 0.0));
        acquired.note(&sketch, crossing, Point2::new(10.0, 0.0));
        let right = placed_at(&sketch, &acquired, Point2::new(10.2, 30.0)).unwrap();
        let left = placed_at(&sketch, &acquired, Point2::new(-10.2, 30.0)).unwrap();

        assert_eq!(acquired.targets(), vec![crossing, crossing]);
        assert_eq!(right.position, Point2::new(10.0, 30.0));
        assert_eq!(left.position, Point2::new(-10.0, 30.0));
    }

    #[test]
    fn a_track_through_a_sketch_point_wins_over_another_lying_on_it() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::ZERO, 10.0);
        let centre = sketch.entity(circle).unwrap().points()[0];
        let right = Target::Quadrant {
            curve: circle,
            centre,
            side: snap::Side::Right,
        };
        let top = Target::Quadrant {
            curve: circle,
            centre,
            side: snap::Side::Top,
        };
        let mut acquired = Acquired::default();

        acquired.note(&sketch, right, Point2::new(10.0, 0.0));
        acquired.note(&sketch, top, Point2::new(0.0, 10.0));
        let level_with_centre = tracks_at(&sketch, &acquired, Point2::new(40.0, 0.2));
        let level_with_top = tracks_at(&sketch, &acquired, Point2::new(40.0, 10.2));

        assert_eq!(acquired.targets(), vec![top, Target::Point(centre), right]);
        assert_eq!(
            level_with_centre
                .constraints(&sketch, EntityId::from_raw(99))
                .collect::<Vec<_>>(),
            vec![Constraint::HorizontalPoints(
                EntityId::from_raw(99),
                centre.into()
            )]
        );
        assert_eq!(level_with_top.level.map(|track| track.anchor), Some(top));
        assert_eq!(
            level_with_top.label(&sketch),
            Some(format!(
                "horizontal from the top of {}",
                sketch.entity_label(circle)
            ))
        );
        assert_eq!(
            level_with_top
                .constraints(&sketch, EntityId::from_raw(99))
                .collect::<Vec<_>>(),
            vec![Constraint::HorizontalPoints(
                EntityId::from_raw(99),
                PointTarget::Quadrant {
                    curve: circle,
                    side: Quadrant::Top,
                }
            )]
        );
    }

    #[test]
    fn a_target_moving_in_a_drag_or_placed_by_the_shape_itself_draws_no_track() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(50.0, 10.0));
        let mut acquired = Acquired::default();
        acquired.note(&sketch, Target::Midpoint(line), Point2::new(30.0, 10.0));
        let above = pointer(Point2::new(30.2, 40.0));
        let screen = Scaled(10.0);

        let moving = nearby(&sketch, &screen, above, &acquired, &[line], &[]);
        let own = nearby(
            &sketch,
            &screen,
            above,
            &acquired,
            &[],
            &[Target::Midpoint(line)],
        );

        assert_eq!(moving, Tracks::default());
        assert_eq!(own, Tracks::default());
    }
}
