use caditor_geometry::{Point2, Vector2};

use crate::{profile::ProfileCurve, tolerance::LINEAR_RESOLUTION};

pub(super) const FIRST_SIDE: u64 = 1;
pub(super) const SECOND_SIDE: u64 = 2;
pub(super) const BLEND_CURVE: u64 = 3;
const CLEARANCE_CURVES: [u64; 5] = [4, 5, 6, 7, 8];

const SMALLEST_PIECE: f64 = 10.0 * LINEAR_RESOLUTION;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum SectionCurve {
    Line,
    Circle { center: Point2, radius: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SectionSide {
    pub curve: SectionCurve,
    pub inward: Vector2,
    pub normal: Vector2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Section {
    pub corner: Point2,
    pub sides: [SectionSide; 2],
    pub convex: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Miss {
    Short,
    Angle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    First,
    Second,
}

impl Side {
    pub fn other(self) -> Self {
        match self {
            Self::First => Self::Second,
            Self::Second => Self::First,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Cut {
    Fillet { center: Point2, radius: f64 },
    Chamfer,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Blend {
    pub feet: [Point2; 2],
    pub cut: Cut,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Offset {
    Line { origin: Point2, direction: Vector2 },
    Circle { center: Point2, radius: f64 },
}

impl SectionSide {
    fn toward_ball(&self, convex: bool) -> Vector2 {
        if convex { -self.normal } else { self.normal }
    }

    fn offset(&self, corner: Point2, convex: bool, distance: f64) -> Option<Offset> {
        let side = self.toward_ball(convex);
        match self.curve {
            SectionCurve::Line => Some(Offset::Line {
                origin: corner + side * distance,
                direction: self.inward,
            }),
            SectionCurve::Circle { center, radius } => {
                let radius = if side.dot(center - corner) > 0.0 {
                    radius - distance
                } else {
                    radius + distance
                };
                (radius > SMALLEST_PIECE).then_some(Offset::Circle { center, radius })
            }
        }
    }

    fn foot(&self, corner: Point2, point: Point2) -> Option<Point2> {
        match self.curve {
            SectionCurve::Line => Some(corner + self.inward * (point - corner).dot(self.inward)),
            SectionCurve::Circle { center, radius } => {
                Some(center + (point - center).try_normalize()? * radius)
            }
        }
    }

    fn along(&self, corner: Point2, distance: f64) -> Option<Point2> {
        match self.curve {
            SectionCurve::Line => Some(corner + self.inward * distance),
            SectionCurve::Circle { center, radius } => {
                let half = distance / (2.0 * radius);
                if half >= 1.0 {
                    return None;
                }
                let turn = 2.0 * half.asin();
                let from = corner - center;
                let angle = if from.perp_dot(self.inward) >= 0.0 {
                    turn
                } else {
                    -turn
                };
                Some(center + Vector2::from_angle(angle).rotate(from))
            }
        }
    }

    fn toward_corner(&self, corner: Point2, foot: Point2) -> Option<Vector2> {
        match self.curve {
            SectionCurve::Line => Some(-self.inward),
            SectionCurve::Circle { center, .. } => {
                let tangent = (foot - center).perp().try_normalize()?;
                Some(if tangent.dot(corner - foot) >= 0.0 {
                    tangent
                } else {
                    -tangent
                })
            }
        }
    }

    fn whole(&self, corner: Point2) -> Offset {
        match self.curve {
            SectionCurve::Line => Offset::Line {
                origin: corner,
                direction: self.inward,
            },
            SectionCurve::Circle { center, radius } => Offset::Circle { center, radius },
        }
    }

    fn reaches(&self, corner: Point2, foot: Point2) -> bool {
        (foot - corner).dot(self.inward) > SMALLEST_PIECE
    }

    fn lifted(&self, corner: Point2, point: Point2, clearance: f64) -> Option<Point2> {
        match self.curve {
            SectionCurve::Line => Some(point + self.normal * clearance),
            SectionCurve::Circle { center, radius } => {
                let outward = self.normal.dot(corner - center) > 0.0;
                let lifted = if outward {
                    radius + clearance
                } else {
                    radius - clearance
                };
                (lifted > SMALLEST_PIECE).then(|| center + (point - center) * (lifted / radius))
            }
        }
    }

    fn piece(&self, entity: u64, corner: Point2, foot: Point2) -> ProfileCurve {
        match self.curve {
            SectionCurve::Line => ProfileCurve::line(entity, corner, foot),
            SectionCurve::Circle { center, .. } => short_arc(entity, center, corner, foot),
        }
    }
}

fn short_arc(entity: u64, center: Point2, from: Point2, to: Point2) -> ProfileCurve {
    if (from - center).perp_dot(to - center) >= 0.0 {
        ProfileCurve::arc(entity, center, from, to)
    } else {
        ProfileCurve::arc(entity, center, to, from)
    }
}

fn intersections(first: Offset, second: Offset) -> Vec<Point2> {
    match (first, second) {
        (
            Offset::Line {
                origin: a,
                direction: da,
            },
            Offset::Line {
                origin: b,
                direction: db,
            },
        ) => {
            let denominator = da.perp_dot(db);
            if denominator.abs() <= f64::EPSILON {
                return Vec::new();
            }
            vec![a + da * ((b - a).perp_dot(db) / denominator)]
        }
        (Offset::Line { origin, direction }, Offset::Circle { center, radius })
        | (Offset::Circle { center, radius }, Offset::Line { origin, direction }) => {
            let offset = origin - center;
            let half_b = direction.dot(offset);
            let c = offset.length_squared() - radius * radius;
            let discriminant = half_b * half_b - c;
            if discriminant < 0.0 {
                return Vec::new();
            }
            let root = discriminant.sqrt();
            vec![
                origin + direction * (-half_b - root),
                origin + direction * (-half_b + root),
            ]
        }
        (
            Offset::Circle {
                center: a,
                radius: ra,
            },
            Offset::Circle {
                center: b,
                radius: rb,
            },
        ) => {
            let between = b - a;
            let distance = between.length();
            if distance <= f64::EPSILON || distance > ra + rb || distance < (ra - rb).abs() {
                return Vec::new();
            }
            let along = (ra * ra - rb * rb + distance * distance) / (2.0 * distance);
            let across = (ra * ra - along * along).max(0.0).sqrt();
            let unit = between / distance;
            let middle = a + unit * along;
            vec![middle + unit.perp() * across, middle - unit.perp() * across]
        }
    }
}

impl Section {
    pub fn fillet(&self, radius: f64) -> Option<Blend> {
        let [first, second] = &self.sides;
        let offsets = (
            first.offset(self.corner, self.convex, radius)?,
            second.offset(self.corner, self.convex, radius)?,
        );
        intersections(offsets.0, offsets.1)
            .into_iter()
            .filter_map(|center| {
                let feet = [
                    first.foot(self.corner, center)?,
                    second.foot(self.corner, center)?,
                ];
                let reaches =
                    first.reaches(self.corner, feet[0]) && second.reaches(self.corner, feet[1]);
                reaches.then_some(Blend {
                    feet,
                    cut: Cut::Fillet { center, radius },
                })
            })
            .min_by(|a, b| {
                let distance = |blend: &Blend| match blend.cut {
                    Cut::Fillet { center, .. } => center.distance(self.corner),
                    Cut::Chamfer => 0.0,
                };
                distance(a).total_cmp(&distance(b))
            })
    }

    pub fn chamfer(&self, distances: [f64; 2]) -> Option<Blend> {
        let [first, second] = &self.sides;
        let feet = [
            first.along(self.corner, distances[0])?,
            second.along(self.corner, distances[1])?,
        ];
        let reaches = first.reaches(self.corner, feet[0]) && second.reaches(self.corner, feet[1]);
        reaches.then_some(Blend {
            feet,
            cut: Cut::Chamfer,
        })
    }

    pub fn angled_chamfer(
        &self,
        measured_on: Side,
        distance: f64,
        angle: f64,
    ) -> Result<Blend, Miss> {
        let [first, second] = &self.sides;
        let (near, far) = match measured_on {
            Side::First => (first, second),
            Side::Second => (second, first),
        };
        let foot = near
            .along(self.corner, distance)
            .filter(|foot| near.reaches(self.corner, *foot))
            .ok_or(Miss::Short)?;
        let back = near.toward_corner(self.corner, foot).ok_or(Miss::Short)?;
        let turn = if back.perp_dot(far.inward) >= 0.0 {
            angle
        } else {
            -angle
        };
        let direction = Vector2::from_angle(turn).rotate(back);
        let cut = Offset::Line {
            origin: foot,
            direction,
        };
        let other = intersections(cut, far.whole(self.corner))
            .into_iter()
            .filter(|point| {
                (*point - foot).dot(direction) > SMALLEST_PIECE && far.reaches(self.corner, *point)
            })
            .min_by(|a, b| a.distance(self.corner).total_cmp(&b.distance(self.corner)))
            .ok_or(Miss::Angle)?;
        let feet = match measured_on {
            Side::First => [foot, other],
            Side::Second => [other, foot],
        };
        Ok(Blend {
            feet,
            cut: Cut::Chamfer,
        })
    }

    pub fn profile(&self, blend: &Blend) -> Vec<ProfileCurve> {
        let [first, second] = &self.sides;
        let [first_foot, second_foot] = blend.feet;
        vec![
            first.piece(FIRST_SIDE, self.corner, first_foot),
            self.cut(blend),
            second.piece(SECOND_SIDE, self.corner, second_foot),
        ]
    }

    pub fn cleared_profile(&self, blend: &Blend, clearance: f64) -> Option<Vec<ProfileCurve>> {
        let [first, second] = &self.sides;
        let [first_foot, second_foot] = blend.feet;
        let [first_rise, first_run, bridge, second_run, second_rise] = CLEARANCE_CURVES;
        let first_corner = first.lifted(self.corner, self.corner, clearance)?;
        let first_lifted = first.lifted(self.corner, first_foot, clearance)?;
        let second_corner = second.lifted(self.corner, self.corner, clearance)?;
        let second_lifted = second.lifted(self.corner, second_foot, clearance)?;
        Some(vec![
            self.cut(blend),
            ProfileCurve::line(second_rise, second_foot, second_lifted),
            second.piece(second_run, second_corner, second_lifted),
            ProfileCurve::line(bridge, second_corner, first_corner),
            first.piece(first_run, first_corner, first_lifted),
            ProfileCurve::line(first_rise, first_lifted, first_foot),
        ])
    }

    fn cut(&self, blend: &Blend) -> ProfileCurve {
        let [first_foot, second_foot] = blend.feet;
        match blend.cut {
            Cut::Fillet { center, .. } => short_arc(BLEND_CURVE, center, first_foot, second_foot),
            Cut::Chamfer => ProfileCurve::line(BLEND_CURVE, first_foot, second_foot),
        }
    }

    pub fn reach(&self, blend: &Blend) -> f64 {
        let feet = blend
            .feet
            .iter()
            .map(|foot| foot.distance(self.corner))
            .fold(0.0, f64::max);
        let center = match blend.cut {
            Cut::Fillet { center, .. } => center.distance(self.corner),
            Cut::Chamfer => 0.0,
        };
        feet.max(center)
    }
}
