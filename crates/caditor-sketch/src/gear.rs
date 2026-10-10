use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::{Constraint, MAX_LENGTH},
    curve::{ArcGeometry, BSpline, Faceting},
    entity::{Entity, SplineKind},
    fit,
    id::EntityId,
    sketch::{Sketch, SketchError},
};

pub const MIN_TEETH: u32 = 5;
pub const MAX_TEETH: u32 = 400;
pub const MIN_PRESSURE_ANGLE: f64 = 10.0;
pub const MAX_PRESSURE_ANGLE: f64 = 35.0;
pub const ADDENDUM: f64 = 1.0;
pub const DEDENDUM: f64 = 1.25;
pub const UNDERCUT_SLACK: f64 = 0.01;
pub const FLANK_TOLERANCE: f64 = 1e-4;
pub const MIN_TIP_LAND: f64 = 0.01;
pub const MIN_RIM: f64 = 0.5;
const MIN_FLANK_POINTS: usize = 4;
const MAX_FLANK_POINTS: usize = 24;
const FLANK_SAMPLES: usize = 64;
const CHECKS_PER_SAMPLE: usize = 4;
const START_ROLL: f64 = 0.03;
const START_ROLL_SHARE: f64 = 0.25;
const FILLET_SCAN: usize = 64;
const BISECTIONS: usize = 60;
const LARGEST_FILLET_STEPS: usize = 40;
const SHIFT_STEP: f64 = 0.01;
const TOUCHING: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GearError {
    #[error("the module must be greater than zero")]
    ModuleNotPositive,
    #[error("a gear has {MIN_TEETH} to {MAX_TEETH} teeth, not {teeth}")]
    TeethOutOfRange { teeth: u32 },
    #[error(
        "the pressure angle must lie between {MIN_PRESSURE_ANGLE}° and {MAX_PRESSURE_ANGLE}°, \
         usually 20°"
    )]
    PressureAngleOutOfRange,
    #[error("the profile shift must be a plain number, such as 0 or 0.3")]
    ShiftNotFinite,
    #[error("the root fillet cannot be negative; use 0 for a sharp root")]
    RootFilletNegative,
    #[error("the bore cannot be negative; use 0 for none")]
    BoreNegative,
    #[error("{}", undercut_words(*.teeth, *.pressure_angle, *.shift, *.least_teeth, *.least_shift))]
    Undercut {
        teeth: u32,
        pressure_angle: f64,
        shift: f64,
        least_teeth: u32,
        least_shift: f64,
    },
    #[error(
        "the teeth come to a point before the tip circle; use a smaller profile shift or more \
         teeth"
    )]
    PointedTeeth,
    #[error("the root circle reaches the centre; use a larger profile shift or more teeth")]
    RootThroughCentre,
    #[error(
        "the teeth meet at the root circle with no gap between them; use a smaller profile shift"
    )]
    NoGap,
    #[error(
        "a root fillet of {} mm does not fit the gap between the teeth: use at most {} mm",
        millimetres(*.fillet),
        millimetres(*.largest)
    )]
    RootFilletTooLarge { fillet: f64, largest: f64 },
    #[error(
        "a bore of {} mm reaches the roots of the teeth: use at most {} mm across",
        millimetres(*.bore),
        millimetres(*.largest)
    )]
    BoreTooLarge { bore: f64, largest: f64 },
    #[error("the gear reaches further than {} m from the origin", MAX_LENGTH / 1_000.0)]
    OutOfReach,
    #[error("the tooth flanks could not be traced within {} mm", millimetres(*.tolerance))]
    FlankNotTraced { tolerance: f64 },
    #[error("{label} is not a point to centre the gear on")]
    NotAPoint { entity: EntityId, label: String },
    #[error(transparent)]
    Edit(SketchError),
}

fn undercut_words(
    teeth: u32,
    pressure_angle: f64,
    shift: f64,
    least_teeth: u32,
    least_shift: f64,
) -> String {
    let shifted = if shift == 0.0 {
        "with no profile shift".to_owned()
    } else {
        format!("with a profile shift of {}", plain(shift))
    };
    format!(
        "{teeth} teeth are undercut at a pressure angle of {}° {shifted}: use at least \
         {least_teeth} teeth, or a profile shift of at least {}",
        plain(pressure_angle),
        plain(least_shift)
    )
}

fn plain(value: f64) -> String {
    let text = format!("{value:.3}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn millimetres(value: f64) -> String {
    plain(value)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpurGear {
    pub module: f64,
    pub teeth: u32,
    pub pressure_angle: f64,
    pub profile_shift: f64,
    pub root_fillet: f64,
    pub bore: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GearCircles {
    pub pitch: f64,
    pub base: f64,
    pub root: f64,
    pub tip: f64,
}

impl GearCircles {
    pub fn radii(&self) -> [f64; 4] {
        [self.pitch, self.base, self.root, self.tip]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GearPiece {
    Arc {
        center: Point2,
        about_gear: bool,
        from: Point2,
        to: Point2,
        counter_clockwise: bool,
    },
    Line {
        from: Point2,
        to: Point2,
    },
    Flank {
        points: Vec<Point2>,
    },
}

impl GearPiece {
    pub fn from(&self) -> Point2 {
        match self {
            Self::Arc { from, .. } | Self::Line { from, .. } => *from,
            Self::Flank { points } => points.first().copied().unwrap_or(Point2::ZERO),
        }
    }

    pub fn to(&self) -> Point2 {
        match self {
            Self::Arc { to, .. } | Self::Line { to, .. } => *to,
            Self::Flank { points } => points.last().copied().unwrap_or(Point2::ZERO),
        }
    }

    pub fn spline(&self) -> Option<BSpline> {
        match self {
            Self::Flank { points } => BSpline::clamped(points.clone()),
            Self::Arc { .. } | Self::Line { .. } => None,
        }
    }

    pub fn faceted(&self, faceting: Faceting) -> Vec<Point2> {
        match self {
            Self::Arc {
                center,
                from,
                to,
                counter_clockwise,
                ..
            } => {
                if *counter_clockwise {
                    ArcGeometry::from_points(*center, *from, *to).faceted(faceting)
                } else {
                    let mut points =
                        ArcGeometry::from_points(*center, *to, *from).faceted(faceting);
                    points.reverse();
                    points
                }
            }
            Self::Line { from, to } => vec![*from, *to],
            Self::Flank { points } => BSpline::clamped(points.clone())
                .map(|spline| spline.faceted(faceting))
                .unwrap_or_default(),
        }
    }

    fn placed(&self, place: &impl Fn(Point2) -> Point2) -> Self {
        match self {
            Self::Arc {
                center,
                about_gear,
                from,
                to,
                counter_clockwise,
            } => Self::Arc {
                center: place(*center),
                about_gear: *about_gear,
                from: place(*from),
                to: place(*to),
                counter_clockwise: *counter_clockwise,
            },
            Self::Line { from, to } => Self::Line {
                from: place(*from),
                to: place(*to),
            },
            Self::Flank { points } => Self::Flank {
                points: points.iter().map(|point| place(*point)).collect(),
            },
        }
    }

    fn mirrored_backwards(&self) -> Self {
        let mirror = |point: Point2| Point2::new(point.x, -point.y);
        match self {
            Self::Arc {
                center,
                about_gear,
                from,
                to,
                counter_clockwise,
            } => Self::Arc {
                center: mirror(*center),
                about_gear: *about_gear,
                from: mirror(*to),
                to: mirror(*from),
                counter_clockwise: *counter_clockwise,
            },
            Self::Line { from, to } => Self::Line {
                from: mirror(*to),
                to: mirror(*from),
            },
            Self::Flank { points } => Self::Flank {
                points: points.iter().rev().map(|point| mirror(*point)).collect(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GearOutline {
    pub centre: Point2,
    pub circles: GearCircles,
    pub pieces: Vec<GearPiece>,
    pub bore: Option<f64>,
    pub deviation: f64,
}

impl GearOutline {
    pub fn faceted(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        let bore = self
            .bore
            .map(|radius| ArcGeometry::full_circle(self.centre, radius).faceted(faceting));
        self.pieces
            .iter()
            .map(|piece| piece.faceted(faceting))
            .chain(bore)
            .collect()
    }

    pub fn construction(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        self.circles
            .radii()
            .into_iter()
            .map(|radius| ArcGeometry::full_circle(self.centre, radius).faceted(faceting))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GearCentre {
    Free(Point2),
    Point(EntityId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawnGear {
    pub centre: EntityId,
    pub outline: Vec<EntityId>,
    pub circles: [EntityId; 4],
    pub bore: Option<EntityId>,
}

#[derive(Debug, Clone, Copy)]
struct Involute {
    base: f64,
    half: f64,
    root: f64,
    tip_roll: f64,
    start_roll: f64,
    gap: f64,
}

impl Involute {
    fn radius(&self, roll: f64) -> f64 {
        self.base * (1.0 + roll * roll).sqrt()
    }

    fn angle(&self, roll: f64) -> f64 {
        self.half - (roll - roll.atan())
    }

    fn point(&self, roll: f64) -> Point2 {
        polar(self.radius(roll), self.angle(roll))
    }

    fn normal(&self, roll: f64) -> Vector2 {
        let angle = self.angle(roll);
        let outward = Vector2::from_angle(angle);
        let around = Vector2::new(-angle.sin(), angle.cos());
        (outward * roll + around) / (1.0 + roll * roll).sqrt()
    }

    fn roll_at(&self, radius: f64) -> f64 {
        ((radius / self.base).powi(2) - 1.0).max(0.0).sqrt()
    }

    fn has_line(&self) -> bool {
        self.root < self.radius(self.start_roll)
    }

    fn foot_angle(&self) -> f64 {
        self.angle(self.start_roll)
    }
}

#[derive(Debug, Clone, Copy)]
struct Fillet {
    centre: Point2,
    touch: Point2,
    root: Point2,
    on_line: bool,
    roll: f64,
}

fn polar(radius: f64, angle: f64) -> Point2 {
    Point2::new(radius * angle.cos(), radius * angle.sin())
}

fn angle_of(point: Point2) -> f64 {
    point.y.atan2(point.x)
}

fn involute_function(angle: f64) -> f64 {
    angle.tan() - angle
}

impl SpurGear {
    pub fn circles(&self) -> GearCircles {
        let pitch = self.module * f64::from(self.teeth) / 2.0;
        GearCircles {
            pitch,
            base: pitch * self.pressure_angle.to_radians().cos(),
            root: pitch - self.module * (DEDENDUM - self.profile_shift),
            tip: pitch + self.module * (ADDENDUM + self.profile_shift),
        }
    }

    fn reach(&self) -> f64 {
        f64::from(self.teeth) * self.pressure_angle.to_radians().sin().powi(2) / 2.0
    }

    pub fn undercut(&self) -> f64 {
        ADDENDUM - self.profile_shift - self.reach()
    }

    pub fn least_teeth(&self) -> u32 {
        let squared = self.pressure_angle.to_radians().sin().powi(2);
        let least = 2.0 * (ADDENDUM - UNDERCUT_SLACK - self.profile_shift) / squared;
        if least.is_finite() && least > 0.0 {
            (least.ceil() as u32).max(MIN_TEETH)
        } else {
            MIN_TEETH
        }
    }

    pub fn least_shift(&self) -> f64 {
        let least = ADDENDUM - UNDERCUT_SLACK - self.reach();
        (least / SHIFT_STEP).ceil() * SHIFT_STEP
    }

    fn checked(&self) -> Result<Involute, GearError> {
        if !(self.module.is_finite() && self.module > 0.0) {
            return Err(GearError::ModuleNotPositive);
        }
        if !(MIN_TEETH..=MAX_TEETH).contains(&self.teeth) {
            return Err(GearError::TeethOutOfRange { teeth: self.teeth });
        }
        if !(self.pressure_angle.is_finite()
            && (MIN_PRESSURE_ANGLE..=MAX_PRESSURE_ANGLE).contains(&self.pressure_angle))
        {
            return Err(GearError::PressureAngleOutOfRange);
        }
        if !self.profile_shift.is_finite() {
            return Err(GearError::ShiftNotFinite);
        }
        if self.root_fillet.is_nan() || self.root_fillet < 0.0 {
            return Err(GearError::RootFilletNegative);
        }
        if self.bore.is_nan() || self.bore < 0.0 {
            return Err(GearError::BoreNegative);
        }
        let circles = self.circles();
        if circles.root <= 0.0 {
            return Err(GearError::RootThroughCentre);
        }
        if self.undercut() > UNDERCUT_SLACK {
            return Err(GearError::Undercut {
                teeth: self.teeth,
                pressure_angle: self.pressure_angle,
                shift: self.profile_shift,
                least_teeth: self.least_teeth(),
                least_shift: self.least_shift(),
            });
        }
        let teeth = f64::from(self.teeth);
        let pressure = self.pressure_angle.to_radians();
        let half_at_pitch = PI / (2.0 * teeth) + 2.0 * self.profile_shift * pressure.tan() / teeth;
        let mut involute = Involute {
            base: circles.base,
            half: half_at_pitch + involute_function(pressure),
            root: circles.root,
            tip_roll: 0.0,
            start_roll: 0.0,
            gap: PI / teeth,
        };
        involute.tip_roll = involute.roll_at(circles.tip);
        let lowest = START_ROLL.min(involute.tip_roll * START_ROLL_SHARE);
        involute.start_roll = if circles.root >= involute.radius(lowest) {
            involute.roll_at(circles.root)
        } else {
            lowest
        };
        let land = 2.0 * circles.tip * involute.angle(involute.tip_roll);
        if land.is_nan()
            || land < MIN_TIP_LAND * self.module
            || involute.start_roll >= involute.tip_roll
        {
            return Err(GearError::PointedTeeth);
        }
        if involute.foot_angle() >= involute.gap {
            return Err(GearError::NoGap);
        }
        Ok(involute)
    }

    fn fillet(involute: &Involute, radius: f64) -> Option<Fillet> {
        let reach = involute.root + radius;
        if involute.has_line() {
            let along = (reach * reach - radius * radius).sqrt();
            if along <= involute.radius(involute.start_roll) {
                let line = involute.foot_angle();
                let centre = polar(reach, line + (radius / reach).asin());
                return Some(Fillet {
                    centre,
                    touch: polar(along, line),
                    root: polar(involute.root, angle_of(centre)),
                    on_line: true,
                    roll: involute.start_roll,
                })
                .filter(|fillet| angle_of(fillet.root) < involute.gap);
            }
        }
        let gap_at =
            |roll: f64| (involute.point(roll) + involute.normal(roll) * radius).length() - reach;
        let mut low = involute.start_roll;
        let roll = if gap_at(low) >= 0.0 {
            low
        } else {
            let step = (involute.tip_roll - low) / FILLET_SCAN as f64;
            let high = (1..=FILLET_SCAN)
                .map(|index| involute.start_roll + step * index as f64)
                .find(|roll| {
                    let reached = gap_at(*roll) >= 0.0;
                    if !reached {
                        low = *roll;
                    }
                    reached
                })?;
            let mut high = high;
            for _ in 0..BISECTIONS {
                let middle = 0.5 * (low + high);
                if gap_at(middle) >= 0.0 {
                    high = middle;
                } else {
                    low = middle;
                }
            }
            high
        };
        if roll >= involute.tip_roll {
            return None;
        }
        let touch = involute.point(roll);
        let centre = touch + involute.normal(roll) * radius;
        Some(Fillet {
            centre,
            touch,
            root: polar(involute.root, angle_of(centre)),
            on_line: false,
            roll,
        })
        .filter(|fillet| angle_of(fillet.root) < involute.gap)
    }

    pub fn largest_root_fillet(&self) -> Option<f64> {
        let involute = self.checked().ok()?;
        let mut low = 0.0;
        let mut high = involute.root;
        for _ in 0..LARGEST_FILLET_STEPS {
            let middle = 0.5 * (low + high);
            if Self::fillet(&involute, middle).is_some() {
                low = middle;
            } else {
                high = middle;
            }
        }
        Some(low)
    }

    pub fn largest_bore(&self) -> f64 {
        2.0 * (self.circles().root - MIN_RIM * self.module).max(0.0)
    }

    fn half_tooth(&self, involute: &Involute) -> Result<(Vec<GearPiece>, f64), GearError> {
        let fillet = if self.root_fillet > 0.0 {
            let found = Self::fillet(involute, self.root_fillet).ok_or_else(|| {
                GearError::RootFilletTooLarge {
                    fillet: self.root_fillet,
                    largest: self.largest_root_fillet().unwrap_or(0.0),
                }
            })?;
            Some(found)
        } else {
            None
        };
        let low_roll = fillet
            .filter(|fillet| !fillet.on_line)
            .map_or(involute.start_roll, |fillet| fillet.roll);
        let tolerance = FLANK_TOLERANCE * self.module;
        let (points, deviation) = traced_flank(involute, low_roll, tolerance)
            .ok_or(GearError::FlankNotTraced { tolerance })?;
        let mut pieces = vec![GearPiece::Flank { points }];
        let mut last = involute.point(low_roll);
        let line_end = match fillet {
            Some(fillet) if fillet.on_line => Some(fillet.touch),
            Some(_) => None,
            None => involute
                .has_line()
                .then(|| polar(involute.root, involute.foot_angle())),
        };
        if let Some(end) = line_end
            && (end - last).length() > TOUCHING * self.module
        {
            pieces.push(GearPiece::Line {
                from: last,
                to: end,
            });
            last = end;
        }
        if let Some(fillet) = fillet {
            let turn = (last - fillet.centre).perp_dot(fillet.root - fillet.centre);
            pieces.push(GearPiece::Arc {
                center: fillet.centre,
                about_gear: false,
                from: last,
                to: fillet.root,
                counter_clockwise: turn > 0.0,
            });
        }
        Ok((pieces, deviation))
    }

    pub fn outline(&self, centre: Point2) -> Result<GearOutline, GearError> {
        let involute = self.checked()?;
        let circles = self.circles();
        if self.bore > 0.0 && self.bore > self.largest_bore() {
            return Err(GearError::BoreTooLarge {
                bore: self.bore,
                largest: self.largest_bore(),
            });
        }
        if !(centre.is_finite() && centre.length() + circles.tip <= MAX_LENGTH) {
            return Err(GearError::OutOfReach);
        }
        let (upper, deviation) = self.half_tooth(&involute)?;
        let lower: Vec<GearPiece> = upper
            .iter()
            .rev()
            .map(GearPiece::mirrored_backwards)
            .collect();
        let tip = involute.point(involute.tip_roll);
        let tip_arc = GearPiece::Arc {
            center: Point2::ZERO,
            about_gear: true,
            from: Point2::new(tip.x, -tip.y),
            to: tip,
            counter_clockwise: true,
        };
        let upper_root = upper.last().map_or(tip, GearPiece::to);
        let lower_root = lower.first().map_or(tip, GearPiece::from);
        let step = TAU / f64::from(self.teeth);
        let place = |turn: f64| {
            let (sine, cosine) = turn.sin_cos();
            move |point: Point2| {
                Point2::new(
                    centre.x + point.x * cosine - point.y * sine,
                    centre.y + point.x * sine + point.y * cosine,
                )
            }
        };
        let mut pieces = Vec::new();
        for tooth in 0..self.teeth {
            let here = place(step * f64::from(tooth));
            let next = place(step * f64::from((tooth + 1) % self.teeth));
            pieces.push(tip_arc.placed(&here));
            pieces.extend(upper.iter().map(|piece| piece.placed(&here)));
            pieces.push(GearPiece::Arc {
                center: centre,
                about_gear: true,
                from: here(upper_root),
                to: next(lower_root),
                counter_clockwise: true,
            });
            pieces.extend(lower.iter().map(|piece| piece.placed(&next)));
        }
        Ok(GearOutline {
            centre,
            circles,
            pieces,
            bore: (self.bore > 0.0).then_some(self.bore / 2.0),
            deviation,
        })
    }
}

fn traced_flank(involute: &Involute, low_roll: f64, tolerance: f64) -> Option<(Vec<Point2>, f64)> {
    let roll_at = |share: f64| involute.tip_roll + (low_roll - involute.tip_roll) * share;
    let parameters: Vec<f64> = (0..=FLANK_SAMPLES)
        .map(|index| index as f64 / FLANK_SAMPLES as f64)
        .collect();
    let samples: Vec<Point2> = parameters
        .iter()
        .map(|share| involute.point(roll_at(*share)))
        .collect();
    let (first, last) = (*samples.first()?, *samples.last()?);
    let checks = FLANK_SAMPLES * CHECKS_PER_SAMPLE;
    let mut best: Option<(Vec<Point2>, f64)> = None;
    for count in MIN_FLANK_POINTS..=MAX_FLANK_POINTS {
        let template = BSpline::clamped(vec![Point2::ZERO; count])?;
        let Some(spline) = fit::least_squares(&template, &samples, &parameters, first, last) else {
            continue;
        };
        let deviation = (0..=checks)
            .map(|index| {
                let share = index as f64 / checks as f64;
                spline
                    .point_at(share)
                    .distance(involute.point(roll_at(share)))
            })
            .fold(0.0, f64::max);
        if best.as_ref().is_none_or(|best| deviation < best.1) {
            best = Some((spline.control_points().to_vec(), deviation));
        }
        if deviation <= tolerance {
            break;
        }
    }
    best.filter(|best| best.1 <= tolerance)
}

impl Sketch {
    pub fn add_gear(
        &mut self,
        gear: &SpurGear,
        centre: GearCentre,
    ) -> Result<DrawnGear, GearError> {
        let at = match centre {
            GearCentre::Free(at) => at,
            GearCentre::Point(point) => self.point(point).ok_or_else(|| GearError::NotAPoint {
                entity: point,
                label: self.entity_label(point),
            })?,
        };
        let outline = gear.outline(at)?;
        let centre = match centre {
            GearCentre::Free(at) => self.add_point(at),
            GearCentre::Point(point) if point.is_reference() => {
                let held = self.add_point(at);
                self.add_constraint(Constraint::Coincident(held, point))
                    .map_err(GearError::Edit)?;
                held
            }
            GearCentre::Point(point) => point,
        };
        let circles = outline.circles.radii().map(|radius| {
            self.insert(Entity::Circle {
                center: centre,
                radius,
            })
        });
        for circle in circles {
            self.set_construction(circle, true)
                .map_err(GearError::Edit)?;
        }
        let first = self.add_point(outline.pieces.first().map_or(at, GearPiece::from));
        let mut last = first;
        let mut drawn = Vec::with_capacity(outline.pieces.len());
        let count = outline.pieces.len();
        for (index, piece) in outline.pieces.iter().enumerate() {
            let end = if index + 1 == count {
                first
            } else {
                self.add_point(piece.to())
            };
            let entity = match piece {
                GearPiece::Arc {
                    center,
                    about_gear,
                    counter_clockwise,
                    ..
                } => {
                    let center = if *about_gear {
                        centre
                    } else {
                        self.add_point(*center)
                    };
                    let (start, end) = if *counter_clockwise {
                        (last, end)
                    } else {
                        (end, last)
                    };
                    Entity::Arc { center, start, end }
                }
                GearPiece::Line { .. } => Entity::Line { start: last, end },
                GearPiece::Flank { points } => {
                    let inner = points
                        .get(1..points.len().saturating_sub(1))
                        .unwrap_or_default();
                    let mut ids = Vec::with_capacity(points.len());
                    ids.push(last);
                    for point in inner {
                        ids.push(self.add_point(*point));
                    }
                    ids.push(end);
                    Entity::Spline {
                        points: ids,
                        kind: SplineKind::OPEN,
                    }
                }
            };
            drawn.push(self.insert(entity));
            last = end;
        }
        let bore = outline.bore.map(|radius| {
            self.insert(Entity::Circle {
                center: centre,
                radius,
            })
        });
        Ok(DrawnGear {
            centre,
            outline: drawn,
            circles,
            bore,
        })
    }
}

#[cfg(test)]
mod tests;
