use std::{
    collections::BTreeMap,
    f64::consts::{FRAC_PI_4, TAU},
    fmt::Debug,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use caditor_geometry::{Maths, Point2, Point3};

use crate::{
    bspline::BSpline, curve::Curve, profile::ProfileCurve, surface::BSplineSurface,
    tessellation::Mesh,
};

pub(crate) fn cancelled_after<T>(allowed: usize, work: impl FnOnce() -> T) -> (T, usize) {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&polls);
    let stop = Arc::new(move || counter.fetch_add(1, Ordering::SeqCst) >= allowed);
    let result = crate::interruptible(stop, work);
    (result, polls.load(Ordering::SeqCst))
}

pub(crate) fn polls_during<T>(work: impl FnOnce() -> T) -> (T, usize) {
    cancelled_after(usize::MAX, work)
}

pub(crate) fn assert_cancelled_anywhere<T, E: Debug>(
    name: &str,
    work: impl Fn() -> Result<T, E>,
    cancelled: impl Fn(&E) -> bool,
) -> usize {
    let (finished, total) = cancelled_after(usize::MAX, &work);

    assert!(finished.is_ok(), "{name}: {:?}", finished.err());

    let mut allowed = 0;

    while allowed < total {
        let (result, _) = cancelled_after(allowed, &work);

        match result {
            Err(error) if cancelled(&error) => {}
            Err(error) => panic!("{name} stopped after {allowed} of {total} polls: {error:?}"),
            Ok(_) => panic!("{name} finished after {allowed} of {total} polls"),
        }
        allowed = allowed * 5 / 4 + 1;
    }
    total
}

pub(crate) struct Random(u64);

impl Random {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);

        let mut mixed = self.0;

        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^ (mixed >> 31)
    }

    pub(crate) fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(crate) fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    pub(crate) fn point(&mut self, reach: f64) -> Point3 {
        Point3::new(
            self.between(-reach, reach),
            self.between(-reach, reach),
            self.between(-reach, reach),
        )
    }

    pub(crate) fn point2(&mut self, reach: f64) -> Point2 {
        Point2::new(self.between(-reach, reach), self.between(-reach, reach))
    }
}

pub(crate) fn line(entity: u64, from: (f64, f64), to: (f64, f64)) -> ProfileCurve {
    ProfileCurve::line(entity, Point2::new(from.0, from.1), Point2::new(to.0, to.1))
}

pub(crate) fn circle(entity: u64, center: (f64, f64), radius: f64) -> ProfileCurve {
    ProfileCurve::circle(entity, Point2::new(center.0, center.1), radius)
}

pub(crate) fn arc(
    entity: u64,
    center: (f64, f64),
    from: (f64, f64),
    to: (f64, f64),
) -> ProfileCurve {
    ProfileCurve::arc(
        entity,
        Point2::new(center.0, center.1),
        Point2::new(from.0, from.1),
        Point2::new(to.0, to.1),
    )
}

pub(crate) fn spline(entity: u64, points: &[(f64, f64)]) -> ProfileCurve {
    let count = points.len();
    let degree = 3.min(count - 1);

    let spans = count - degree;
    let knots = std::iter::repeat_n(0.0, degree + 1)
        .chain((1..spans).map(|index| index as f64 / spans as f64))
        .chain(std::iter::repeat_n(1.0, degree + 1))
        .collect();
    ProfileCurve::spline(
        entity,
        degree,
        knots,
        points.iter().map(|(x, y)| Point2::new(*x, *y)).collect(),
    )
}

pub(crate) fn rectangle(first: u64, min: (f64, f64), max: (f64, f64)) -> Vec<ProfileCurve> {
    vec![
        line(first, min, (max.0, min.1)),
        line(first + 1, (max.0, min.1), max),
        line(first + 2, max, (min.0, max.1)),
        line(first + 3, (min.0, max.1), min),
    ]
}

pub(crate) fn assert_watertight(name: &str, mesh: &Mesh) {
    let mut directed: BTreeMap<(u32, u32), usize> = BTreeMap::new();

    for [a, b, c] in mesh.position_triangles() {
        assert!(a != b && b != c && a != c, "{name}: degenerate triangle");
        for edge in [(a, b), (b, c), (c, a)] {
            *directed.entry(edge).or_default() += 1;
        }
    }

    assert!(!directed.is_empty(), "{name}: no triangles");
    for ((from, to), count) in &directed {
        assert_eq!(*count, 1, "{name}: edge {from}->{to} used {count} times");
        assert_eq!(
            directed.get(&(*to, *from)),
            Some(&1),
            "{name}: edge {from}->{to} has no opposite ({:?} -> {:?})",
            mesh.position(*from),
            mesh.position(*to)
        );
    }
}

pub(crate) struct Helix {
    pub(crate) turns: usize,
    pub(crate) pitch: f64,
}

impl Helix {
    const POINTS_PER_TURN: usize = 24;
    const DEGREE: usize = 3;

    fn count(&self) -> usize {
        Self::POINTS_PER_TURN * self.turns + 1
    }

    fn knots(&self) -> Vec<f64> {
        let spans = self.count() - Self::DEGREE;
        let mut knots = vec![0.0; Self::DEGREE];

        knots.extend((0..=spans).map(|span| span as f64));
        knots.extend([spans as f64; Self::DEGREE]);
        knots
    }

    fn at(&self, radius: f64, rise: f64, index: usize) -> Point3 {
        let turned = index as f64 / Self::POINTS_PER_TURN as f64;
        let angle = TAU * turned;
        Point3::new(
            radius * angle.cos_libm(),
            radius * angle.sin_libm(),
            self.pitch * turned + rise,
        )
    }

    pub(crate) fn flank(&self, sides: [(f64, f64); 2]) -> BSplineSurface {
        let [(inner, inner_rise), (outer, outer_rise)] = sides;

        let control_points: Vec<Point3> = (0..self.count())
            .flat_map(|index| {
                [
                    self.at(inner, inner_rise, index),
                    self.at(outer, outer_rise, index),
                ]
            })
            .collect();

        BSplineSurface::new(
            1,
            Self::DEGREE,
            vec![0.0, 0.0, 1.0, 1.0],
            self.knots(),
            2,
            control_points,
            None,
        )
        .unwrap()
    }

    pub(crate) fn exact_flank(&self, sides: [(f64, f64); 2]) -> BSplineSurface {
        let [(inner, inner_rise), (outer, outer_rise)] = sides;
        let quarters = 4 * self.turns;
        let corner = FRAC_PI_4.cos_libm();
        let weight = |index: usize| if index % 2 == 1 { corner } else { 1.0 };
        let at = |radius: f64, rise: f64, index: usize| {
            let angle = FRAC_PI_4 * index as f64;
            let reach = radius / weight(index);
            Point3::new(
                reach * angle.cos_libm(),
                reach * angle.sin_libm(),
                self.pitch * index as f64 / 8.0 + rise,
            )
        };
        let mut knots = vec![0.0; 3];

        for quarter in 1..quarters {
            knots.extend([quarter as f64; 2]);
        }
        knots.extend([quarters as f64; 3]);

        let indices = 0..2 * quarters + 1;
        let control_points: Vec<Point3> = indices
            .clone()
            .flat_map(|index| [at(inner, inner_rise, index), at(outer, outer_rise, index)])
            .collect();
        let weights: Vec<f64> = indices.flat_map(|index| [weight(index); 2]).collect();

        BSplineSurface::new(
            1,
            2,
            vec![0.0, 0.0, 1.0, 1.0],
            knots,
            2,
            control_points,
            Some(weights),
        )
        .unwrap()
    }

    pub(crate) fn curve(&self, radius: f64, rise: f64) -> Curve {
        let control_points: Vec<Point3> = (0..self.count())
            .map(|index| self.at(radius, rise, index))
            .collect();

        BSpline::new(Self::DEGREE, self.knots(), control_points)
            .unwrap()
            .into()
    }
}
