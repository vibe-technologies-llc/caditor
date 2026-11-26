use std::{collections::BTreeMap, f64::consts::PI};

use caditor_geometry::{Maths, Point2, Point3, Vector3};

use super::{
    HiddenLineError,
    bernstein::{Bernstein, Quadrants},
    contour::{facing, turning_points},
};
use crate::{
    bspline::BSpline,
    curve::Curve,
    interrupt,
    interval::Interval,
    surface::{BSplineSurface, Revolution, Surface},
    tally,
};

pub(super) const FLAT_FACING: f64 = 1e-10;
pub(super) const SAME_LINE: f64 = 1e-9;
const BOUND_SLACK: f64 = FLAT_FACING;
const MAX_SEARCH_DEPTH: usize = 6;
const MAX_SEARCH_POINTS: usize = 4096;
const MAX_BOUNDED_SPANS: usize = 4096;
const MAX_FOUND: usize = 32;
const SEEN_ALONG_AXIS: f64 = 1e-9;
const ON_AXIS: f64 = 1e-9;
const NET_WORK: usize = 2;
const SPAN_WORK: usize = 10;
const RESTRICTION_WORK: usize = 2;

pub(super) fn is_positive(surface: &Surface, uv: Point2, toward: Vector3) -> bool {
    facing(surface, uv, toward) >= -FLAT_FACING
}

pub(super) struct Lattice<'a> {
    pub surface: &'a Surface,
    pub toward: Vector3,
    pub us: &'a [f64],
    pub vs: &'a [f64],
    pub positive: &'a [bool],
    pub wraps_u: bool,
}

impl Lattice<'_> {
    fn sign(&self, column: usize, row: usize) -> Option<bool> {
        self.positive.get(row * self.us.len() + column).copied()
    }

    fn node(&self, column: usize, row: usize) -> Option<Point2> {
        Some(Point2::new(*self.us.get(column)?, *self.vs.get(row)?))
    }

    fn settled(&self, column: usize, row: usize) -> Option<bool> {
        let first = self.sign(column, row)?;
        let columns = column.saturating_sub(1)..=(column + 2).min(self.us.len().saturating_sub(1));
        let rows = row.saturating_sub(1)..=(row + 2).min(self.vs.len().saturating_sub(1));

        rows.flat_map(|row| columns.clone().map(move |column| (column, row)))
            .all(|(column, row)| self.sign(column, row) == Some(first))
            .then_some(first)
    }
}

struct Budget {
    points: usize,
}

impl Budget {
    fn spend(&mut self) -> bool {
        match self.points.checked_sub(1) {
            Some(left) => {
                self.points = left;
                tally::add_searched(1);
                true
            }
            None => false,
        }
    }
}

pub(super) fn missed(lattice: &Lattice<'_>) -> Result<Vec<Point2>, HiddenLineError> {
    let mut budget = Budget {
        points: MAX_SEARCH_POINTS,
    };
    match lattice.surface {
        Surface::BSpline(spline) => spline_missed(lattice, spline, &mut budget),
        Surface::Revolution(revolution) => revolution_missed(lattice, revolution, &mut budget),
        _ => Ok(Vec::new()),
    }
}

fn breaks_of(knots: &[f64], domain: Interval) -> Vec<f64> {
    let mut breaks: Vec<f64> = std::iter::once(domain.start())
        .chain(
            knots
                .iter()
                .copied()
                .filter(|knot| *knot > domain.start() && *knot < domain.end()),
        )
        .chain(std::iter::once(domain.end()))
        .collect();

    breaks.dedup();
    breaks
}

fn pieces(low: f64, high: f64, domain: Interval, period: Option<f64>) -> Vec<(f64, f64)> {
    match period {
        Some(period) if period > 0.0 => {
            let turns = ((low - domain.start()) / period).floor();
            let (low, high) = (low - turns * period, high - turns * period);

            if high <= domain.end() {
                vec![(low, high)]
            } else {
                vec![
                    (low, domain.end()),
                    (domain.start(), (high - period).min(domain.end())),
                ]
            }
        }
        _ => vec![(domain.clamp(low), domain.clamp(high))],
    }
}

fn spans_over(breaks: &[f64], low: f64, high: f64) -> Vec<(usize, f64, f64)> {
    let first = breaks
        .partition_point(|value| *value <= low)
        .saturating_sub(1);

    breaks
        .windows(2)
        .enumerate()
        .skip(first)
        .take_while(|(_, pair)| pair.first().is_some_and(|start| *start < high))
        .filter_map(|(index, pair)| match pair {
            [start, end] if end > start => {
                let (from, to) = (low.max(*start), high.min(*end));
                (to > from).then(|| {
                    (
                        index,
                        (from - start) / (end - start),
                        (to - start) / (end - start),
                    )
                })
            }
            _ => None,
        })
        .collect()
}

fn crossed(vector: &[Bernstein; 3], toward: Vector3) -> Option<[Bernstein; 3]> {
    let [x, y, z] = vector;
    Some([
        y.scaled(toward.z).plus(&z.scaled(-toward.y))?,
        z.scaled(toward.x).plus(&x.scaled(-toward.z))?,
        x.scaled(toward.y).plus(&y.scaled(-toward.x))?,
    ])
}

fn dot(first: &[Bernstein; 3], second: &[Bernstein; 3]) -> Option<Bernstein> {
    let [a, b, c] = first;
    let [d, e, f] = second;
    a.product(d).plus(&b.product(e))?.plus(&c.product(f))
}

fn spline_facing(
    net: &[[f64; 4]],
    width: usize,
    toward: Vector3,
    rational: bool,
) -> Option<Bernstein> {
    let height = net.len().checked_div(width)?;
    let [x, y, z, w] = net.first()?;

    let origin = (*w > 0.0).then(|| Point3::new(x / w, y / w, z / w))?;

    if !rational {
        let at = |column: usize, row: usize| {
            net.get(column * height + row)
                .map(|[x, y, z, _]| Vector3::new(*x, *y, *z))
        };
        let along_u = (0..height)
            .flat_map(|row| (0..width.saturating_sub(1)).map(move |column| (column, row)))
            .map(|(column, row)| Some(at(column + 1, row)? - at(column, row)?))
            .collect::<Option<Vec<Vector3>>>()?;
        let across_v = (0..height.saturating_sub(1))
            .flat_map(|row| (0..width).map(move |column| (column, row)))
            .map(|(column, row)| Some((at(column, row + 1)? - at(column, row)?).cross(toward)))
            .collect::<Option<Vec<Vector3>>>()?;

        return Bernstein::dot_of(&along_u, width.checked_sub(1)?, &across_v, width);
    }

    let component = |index: usize| {
        let coefficients = (0..height)
            .flat_map(|row| (0..width).map(move |column| (column, row)))
            .map(|(column, row)| {
                let [x, y, z, w] = net.get(column * height + row).copied()?;
                Some(match index {
                    0 => x - w * origin.x,
                    1 => y - w * origin.y,
                    2 => z - w * origin.z,
                    _ => w,
                })
            })
            .collect::<Option<Vec<f64>>>()?;
        Bernstein::new(width, coefficients)
    };
    let point = [component(0)?, component(1)?, component(2)?];
    let along_u = [
        point[0].along_u()?,
        point[1].along_u()?,
        point[2].along_u()?,
    ];
    let along_v = [
        point[0].along_v()?,
        point[1].along_v()?,
        point[2].along_v()?,
    ];

    let across_v = crossed(&along_v, toward)?;
    let turning = dot(&along_u, &across_v)?;
    let weight = component(3)?;
    let across_u = crossed(&along_u, toward)?;
    let leaning_v = dot(&point, &across_v)?;
    let leaning_u = dot(&point, &across_u)?;

    weight
        .product(&turning)
        .plus(&weight.along_u()?.product(&leaning_v).scaled(-1.0))?
        .plus(&weight.along_v()?.product(&leaning_u))
}

fn knot_index(knots: &[f64], degree: usize, count: usize, span: Interval) -> Option<usize> {
    let index = knots
        .partition_point(|knot| *knot <= span.middle())
        .checked_sub(1)?;
    Some(index.clamp(degree, count.checked_sub(1)?))
}

fn hodograph(
    knots: &[f64],
    degree: usize,
    index: usize,
    from: Point3,
    to: Point3,
) -> Option<Vector3> {
    let gap = knots.get(index + degree + 1)? - knots.get(index + 1)?;
    (gap > 0.0).then(|| (to - from) / gap)
}

fn net_bound(spline: &BSplineSurface, u: Interval, v: Interval, toward: Vector3) -> Option<Reach> {
    if spline.weights().is_some() {
        return None;
    }

    let (u_degree, v_degree) = (spline.u_degree(), spline.v_degree());
    let (u_knots, v_knots) = (spline.u_knots(), spline.v_knots());

    let column = knot_index(u_knots, u_degree, spline.columns(), u)?;
    let row = knot_index(v_knots, v_degree, spline.rows(), v)?;

    let (first_column, first_row) = (column.checked_sub(u_degree)?, row.checked_sub(v_degree)?);
    let point = |column: usize, row: usize| spline.control_point(column, row);

    let mut along_u = Vec::with_capacity(u_degree * (v_degree + 1));

    for net_row in first_row..=row {
        for net_column in first_column..column {
            along_u.extend(hodograph(
                u_knots,
                u_degree,
                net_column,
                point(net_column, net_row)?,
                point(net_column + 1, net_row)?,
            ));
        }
    }

    let mut across_v = Vec::with_capacity((u_degree + 1) * v_degree);

    for net_row in first_row..row {
        for net_column in first_column..=column {
            across_v.extend(
                hodograph(
                    v_knots,
                    v_degree,
                    net_row,
                    point(net_column, net_row)?,
                    point(net_column, net_row + 1)?,
                )
                .map(|along| along.cross(toward)),
            );
        }
    }

    if along_u.is_empty() || across_v.is_empty() {
        return None;
    }

    let longest = |vectors: &[Vector3]| {
        vectors
            .iter()
            .fold(0.0_f64, |most, vector| most.max(vector.length()))
    };
    let floor = -BOUND_SLACK * longest(&along_u) * longest(&across_v);
    let mut reach = Reach {
        positive: false,
        negative: false,
    };

    for along in &along_u {
        for across in &across_v {
            let value = along.dot(*across);

            reach.positive |= value > floor;
            reach.negative |= value < floor;
        }
    }
    Some(reach)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reach {
    positive: bool,
    negative: bool,
}

impl Reach {
    fn holds(self, positive: bool) -> bool {
        if positive {
            self.positive
        } else {
            self.negative
        }
    }
}

struct SpanBound {
    reach: Reach,
    quadrants: Option<Option<Quadrants>>,
}

struct SplineFacing<'a> {
    spline: &'a BSplineSurface,
    toward: Vector3,
    u_breaks: Vec<f64>,
    v_breaks: Vec<f64>,
    spans: BTreeMap<(usize, usize), SpanBound>,
}

impl<'a> SplineFacing<'a> {
    fn new(spline: &'a BSplineSurface, toward: Vector3, low: Point2, high: Point2) -> Option<Self> {
        let u_breaks = breaks_of(spline.u_knots(), spline.u_domain());
        let v_breaks = breaks_of(spline.v_knots(), spline.v_domain());

        let count = |breaks: &[f64], low: f64, high: f64, domain: Interval, period| {
            pieces(low, high, domain, period)
                .iter()
                .map(|(low, high)| spans_over(breaks, *low, *high).len())
                .sum::<usize>()
        };
        let spans = count(
            &u_breaks,
            low.x,
            high.x,
            spline.u_domain(),
            spline.u_period(),
        ) * count(
            &v_breaks,
            low.y,
            high.y,
            spline.v_domain(),
            spline.v_period(),
        );

        (spans <= MAX_BOUNDED_SPANS).then_some(Self {
            spline,
            toward,
            u_breaks,
            v_breaks,
            spans: BTreeMap::new(),
        })
    }

    fn span(&self, column: usize, row: usize) -> Option<(Interval, Interval)> {
        let span = |breaks: &[f64], index: usize| {
            Interval::new(*breaks.get(index)?, *breaks.get(index + 1)?)
        };
        Some((span(&self.u_breaks, column)?, span(&self.v_breaks, row)?))
    }

    fn quadrants(&mut self, column: usize, row: usize, positive: bool) -> Option<&mut Quadrants> {
        let (spline, toward) = (self.spline, self.toward);
        let span = self.span(column, row);

        let bound = self.spans.entry((column, row)).or_insert_with(|| {
            tally::add_searched(NET_WORK);
            SpanBound {
                reach: span
                    .and_then(|(u, v)| net_bound(spline, u, v, toward))
                    .unwrap_or(Reach {
                        positive: true,
                        negative: true,
                    }),
                quadrants: None,
            }
        });

        if !bound.reach.holds(positive) {
            return None;
        }
        bound
            .quadrants
            .get_or_insert_with(|| {
                tally::add_searched(SPAN_WORK);

                let (u, v) = span?;
                spline_facing(
                    &spline.bezier_net(u, v)?,
                    spline.u_degree() + 1,
                    toward,
                    spline.weights().is_some(),
                )
                .map(Quadrants::new)
            })
            .as_mut()
    }

    fn may_hold(&mut self, low: Point2, high: Point2, positive: bool) -> bool {
        let spline = self.spline;
        let u_pieces = pieces(low.x, high.x, spline.u_domain(), spline.u_period());
        let v_pieces = pieces(low.y, high.y, spline.v_domain(), spline.v_period());

        for (u_low, u_high) in u_pieces {
            for (v_low, v_high) in &v_pieces {
                let columns = spans_over(&self.u_breaks, u_low, u_high);
                let rows = spans_over(&self.v_breaks, *v_low, *v_high);
                for (row, row_low, row_high) in &rows {
                    for (column, column_low, column_high) in &columns {
                        let Some(quadrants) = self.quadrants(*column, *row, positive) else {
                            continue;
                        };
                        if quadrants.may_hold(
                            (*column_low, *column_high),
                            (*row_low, *row_high),
                            positive,
                            BOUND_SLACK,
                        ) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

fn spline_missed(
    lattice: &Lattice<'_>,
    spline: &BSplineSurface,
    budget: &mut Budget,
) -> Result<Vec<Point2>, HiddenLineError> {
    let (Some(low), Some(high)) = (
        lattice.node(0, 0),
        lattice.node(
            lattice.us.len().saturating_sub(1),
            lattice.vs.len().saturating_sub(1),
        ),
    ) else {
        return Ok(Vec::new());
    };

    let Some(mut bound) = SplineFacing::new(spline, lattice.toward, low, high) else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();

    for row in 0..lattice.vs.len().saturating_sub(1) {
        interrupt::check()?;
        for column in 0..lattice.us.len().saturating_sub(1) {
            if found.len() >= MAX_FOUND || budget.points == 0 {
                return Ok(found);
            }

            let Some(class) = lattice.settled(column, row) else {
                continue;
            };
            let (Some(low), Some(high)) =
                (lattice.node(column, row), lattice.node(column + 1, row + 1))
            else {
                continue;
            };

            found.extend(cell_search(lattice, &mut bound, low, high, !class, budget)?);
        }
    }
    Ok(found)
}

fn cell_search(
    lattice: &Lattice<'_>,
    bound: &mut SplineFacing<'_>,
    low: Point2,
    high: Point2,
    wanted: bool,
    budget: &mut Budget,
) -> Result<Option<Point2>, HiddenLineError> {
    let mut pending = vec![(low, high, 0_usize)];

    while let Some((low, high, depth)) = pending.pop() {
        if !bound.may_hold(low, high, wanted) {
            continue;
        }
        if !budget.spend() {
            return Ok(None);
        }
        interrupt::check()?;

        let middle = low.lerp(high, 0.5);

        if is_positive(lattice.surface, middle, lattice.toward) == wanted {
            return Ok(Some(middle));
        }
        if depth < MAX_SEARCH_DEPTH {
            for (from, to) in [
                (low, middle),
                (Point2::new(middle.x, low.y), Point2::new(high.x, middle.y)),
                (Point2::new(low.x, middle.y), Point2::new(middle.x, high.y)),
                (middle, high),
            ] {
                pending.push((from, to, depth + 1));
            }
        }
    }
    Ok(None)
}

struct Meridian {
    radial: Vector3,
    turned: Vector3,
}

fn meridian_of(revolution: &Revolution, v_low: f64, v_high: f64) -> Option<Meridian> {
    let axis = revolution.axis_direction();

    [0.5, 0.0, 1.0, 0.25, 0.75]
        .into_iter()
        .map(|share| v_low + (v_high - v_low) * share)
        .find_map(|v| {
            let point = revolution.evaluate(0.0, v).point;
            let offset = point - revolution.axis_point(point);

            let radial = (offset.length() > ON_AXIS * (1.0 + point.length()))
                .then(|| offset.try_normalize())
                .flatten()?;
            Some(Meridian {
                radial,
                turned: axis.cross(radial),
            })
        })
}

pub(super) fn meridians(
    surface: &Surface,
    toward: Vector3,
    (u_low, u_high): (f64, f64),
    (v_low, v_high): (f64, f64),
) -> Vec<f64> {
    let Surface::Revolution(revolution) = surface else {
        return Vec::new();
    };
    let axis = revolution.axis_direction();
    let across = toward - axis * toward.dot(axis);

    if across.length() <= SEEN_ALONG_AXIS {
        return Vec::new();
    }

    let Some(meridian) = meridian_of(revolution, v_low, v_high) else {
        return Vec::new();
    };
    let facing_most = toward
        .dot(meridian.turned)
        .atan2_libm(toward.dot(meridian.radial));
    [facing_most, facing_most + PI]
        .into_iter()
        .map(|angle| u_low + (angle - u_low).rem_euclid(2.0 * PI))
        .filter(|angle| *angle < u_high)
        .collect()
}

struct ProfileFacing<'a> {
    profile: &'a BSpline<Point3>,
    origin: Point3,
    meridian: Meridian,
    axis: Vector3,
    breaks: Vec<f64>,
    spans: BTreeMap<usize, Option<[Bernstein; 4]>>,
}

fn profile_terms(
    span: &BSpline<Point3>,
    origin: Point3,
    meridian: &Meridian,
    axis: Vector3,
) -> Option<[Bernstein; 4]> {
    let weights = span.weights();
    let weight = |index: usize| weights.and_then(|weights| weights.get(index)).copied();
    let line = |measure: &dyn Fn(Point3, f64) -> f64| {
        Bernstein::line(
            span.control_points()
                .iter()
                .enumerate()
                .map(|(index, point)| measure(*point, weight(index).unwrap_or(1.0)))
                .collect(),
        )
    };
    let radius = line(&|point, weight| weight * (point - origin).dot(meridian.radial))?;
    let height = line(&|point, weight| weight * (point - origin).dot(axis))?;

    let (rising, widening) = if weights.is_some() {
        let weight = line(&|_, weight| weight)?;
        let weight_slope = weight.along_u()?;
        (
            height
                .along_u()?
                .product(&weight)
                .plus(&height.product(&weight_slope).scaled(-1.0))?,
            radius
                .along_u()?
                .product(&weight)
                .plus(&radius.product(&weight_slope).scaled(-1.0))?,
        )
    } else {
        (height.along_u()?, radius.along_u()?)
    };
    Some([radius, rising, widening, height])
}

impl<'a> ProfileFacing<'a> {
    fn new(revolution: &'a Revolution, meridian: Meridian) -> Option<Self> {
        let Curve::BSpline(profile) = revolution.profile() else {
            return None;
        };
        Some(Self {
            profile,
            origin: revolution.axis_origin(),
            meridian,
            axis: revolution.axis_direction(),
            breaks: breaks_of(profile.knots(), profile.domain()),
            spans: BTreeMap::new(),
        })
    }

    fn may_hold(&mut self, low: f64, high: f64, along: f64, up: f64, positive: bool) -> bool {
        let (profile, origin, axis) = (self.profile, self.origin, self.axis);
        let low = profile.domain().clamp(low);
        let high = profile.domain().clamp(high);

        for (index, from, to) in spans_over(&self.breaks, low, high) {
            let breaks = &self.breaks;
            let meridian = &self.meridian;

            let terms = self.spans.entry(index).or_insert_with(|| {
                tally::add_searched(SPAN_WORK);

                let span = Interval::new(*breaks.get(index)?, *breaks.get(index + 1)?)?;
                profile_terms(&profile.restricted(span)?, origin, meridian, axis)
            });
            let Some([radius, rising, widening, _]) = terms.as_ref() else {
                continue;
            };

            tally::add_searched(RESTRICTION_WORK);

            let held = rising
                .scaled(along)
                .plus(&widening.scaled(-up))
                .map(|slope| radius.product(&slope).restricted((from, to), (0.0, 1.0)));
            if held.is_some_and(|polynomial| polynomial.may_hold(positive, BOUND_SLACK)) {
                return true;
            }
        }
        false
    }
}

fn revolution_missed(
    lattice: &Lattice<'_>,
    revolution: &Revolution,
    budget: &mut Budget,
) -> Result<Vec<Point2>, HiddenLineError> {
    let (Some(first), Some(last)) = (lattice.us.first(), lattice.us.last()) else {
        return Ok(Vec::new());
    };
    let (Some(v_low), Some(v_high)) = (lattice.vs.first(), lattice.vs.last()) else {
        return Ok(Vec::new());
    };
    let axis = revolution.axis_direction();
    let up = lattice.toward.dot(axis);

    let mut searched = meridians(
        lattice.surface,
        lattice.toward,
        (*first, *last),
        (*v_low, *v_high),
    );

    if !lattice.wraps_u || searched.is_empty() {
        searched.extend([*first, *last]);
    }
    searched.sort_by(f64::total_cmp);
    searched.dedup();

    let mut bound = meridian_of(revolution, *v_low, *v_high)
        .and_then(|meridian| ProfileFacing::new(revolution, meridian));
    let mut found = Vec::new();

    for u in searched {
        interrupt::check()?;

        let reach = SAME_LINE * (last - first).abs();
        let Some((column, u)) = lattice
            .us
            .iter()
            .copied()
            .enumerate()
            .min_by(|(_, a), (_, b)| (a - u).abs().total_cmp(&(b - u).abs()))
            .filter(|(_, nearest)| (nearest - u).abs() <= reach)
        else {
            continue;
        };
        let along = bound.as_ref().map_or(0.0, |bound| {
            revolution
                .rotate(bound.meridian.radial, u)
                .dot(lattice.toward)
        });

        let found_here = match bound.as_mut() {
            Some(bound) => meridian_search(lattice, column, budget, &mut |low, high, positive| {
                bound.may_hold(low, high, along, up, positive)
            })?,
            None => meridian_turns(lattice, column, budget),
        };

        found.extend(found_here);
        if found.len() >= MAX_FOUND {
            break;
        }
    }
    found.truncate(MAX_FOUND);
    Ok(found)
}

fn meridian_search(
    lattice: &Lattice<'_>,
    column: usize,
    budget: &mut Budget,
    may_hold: &mut dyn FnMut(f64, f64, bool) -> bool,
) -> Result<Vec<Point2>, HiddenLineError> {
    let Some(u) = lattice.us.get(column).copied() else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();

    for (row, pair) in lattice.vs.windows(2).enumerate() {
        let [low, high] = pair else {
            continue;
        };
        let (Some(below), Some(above)) = (lattice.sign(column, row), lattice.sign(column, row + 1))
        else {
            continue;
        };

        if below != above {
            continue;
        }

        let mut pending = vec![(*low, *high, 0_usize)];
        while let Some((low, high, depth)) = pending.pop() {
            if !may_hold(low, high, !below) {
                continue;
            }
            if !budget.spend() {
                return Ok(found);
            }
            interrupt::check()?;

            let middle = 0.5 * (low + high);
            let point = Point2::new(u, middle);

            if is_positive(lattice.surface, point, lattice.toward) != below {
                found.push(point);
                break;
            }
            if depth < 2 * MAX_SEARCH_DEPTH {
                pending.push((middle, high, depth + 1));
                pending.push((low, middle, depth + 1));
            }
        }
    }
    Ok(found)
}

fn meridian_turns(lattice: &Lattice<'_>, column: usize, budget: &mut Budget) -> Vec<Point2> {
    let Some(u) = lattice.us.get(column).copied() else {
        return Vec::new();
    };
    let mut value = |v: f64| {
        if budget.spend() {
            facing(lattice.surface, Point2::new(u, v), lattice.toward) + FLAT_FACING
        } else {
            f64::NAN
        }
    };

    let values: Vec<(f64, f64)> = lattice.vs.iter().map(|v| (*v, value(*v))).collect();
    turning_points(&values, &mut value)
        .into_iter()
        .filter(|turn| {
            values
                .get(turn.after)
                .is_some_and(|(_, at)| (turn.value >= 0.0) != (*at >= 0.0))
        })
        .map(|turn| Point2::new(u, turn.parameter))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fixtures::bumped_sheet, test_support::Random};

    const REACH: f64 = 1e-7;
    const CLEAR: f64 = 1e-3;

    fn weighted(spline: &BSplineSurface) -> BSplineSurface {
        let weights = (0..spline.control_points().len())
            .map(|index| 1.0 + 0.5 * ((index * 7) % 5) as f64 / 4.0)
            .collect();
        BSplineSurface::new(
            spline.u_degree(),
            spline.v_degree(),
            spline.u_knots().to_vec(),
            spline.v_knots().to_vec(),
            spline.columns(),
            spline.control_points().to_vec(),
            Some(weights),
        )
        .unwrap()
    }

    fn direction(random: &mut Random) -> Vector3 {
        random.point(1.0).try_normalize().unwrap_or(Vector3::Z)
    }

    #[test]
    fn span_polynomials_take_the_sign_of_the_facing() {
        let plain = bumped_sheet(10.0, 4.0, 7, 3.0);
        let mut random = Random::new(7);

        for spline in [weighted(&plain), plain] {
            let surface = Surface::from(spline.clone());
            for _ in 0..8 {
                let toward = direction(&mut random);
                let mut bound = SplineFacing::new(
                    &spline,
                    toward,
                    Point2::new(0.0, 0.0),
                    Point2::new(1.0, 1.0),
                )
                .unwrap();
                for _ in 0..200 {
                    let uv = Point2::new(random.between(0.01, 0.99), random.between(0.01, 0.99));
                    let seen = facing(&surface, uv, toward);

                    if seen.abs() < CLEAR {
                        continue;
                    }

                    let (low, high) = (uv - Point2::splat(REACH), uv + Point2::splat(REACH));

                    assert!(bound.may_hold(low, high, seen > 0.0), "{uv:?} {toward:?}");
                    assert!(!bound.may_hold(low, high, seen < 0.0), "{uv:?} {toward:?}");
                }
            }
        }
    }

    #[test]
    fn profile_polynomials_take_the_sign_of_the_facing_on_every_meridian() {
        let points = vec![
            Point3::new(2.0, 0.0, -1.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(3.0, 0.0, 2.0),
            Point3::new(2.5, 0.0, 3.0),
        ];
        let knots = vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0];

        let profiles = [
            BSpline::new(3, knots.clone(), points.clone()).unwrap(),
            BSpline::rational(3, knots, points, vec![1.0, 2.0, 0.5, 1.5, 1.0]).unwrap(),
        ];
        let mut random = Random::new(11);

        for profile in profiles {
            let revolution =
                Revolution::new(Curve::BSpline(profile), Point3::ZERO, Vector3::Z).unwrap();
            let surface = Surface::from(revolution.clone());
            let meridian = meridian_of(&revolution, 0.0, 1.0).unwrap();
            let radial = meridian.radial;

            let mut bound = ProfileFacing::new(&revolution, meridian).unwrap();

            for _ in 0..8 {
                let toward = direction(&mut random);
                for _ in 0..100 {
                    let (u, v) = (random.between(0.0, 6.0), random.between(0.01, 0.99));

                    let seen = facing(&surface, Point2::new(u, v), toward);

                    if seen.abs() < CLEAR {
                        continue;
                    }

                    let along = revolution.rotate(radial, u).dot(toward);
                    let up = toward.dot(Vector3::Z);
                    let (low, high) = (v - REACH, v + REACH);

                    assert!(bound.may_hold(low, high, along, up, seen > 0.0), "{u} {v}");
                    assert!(!bound.may_hold(low, high, along, up, seen < 0.0), "{u} {v}");
                }
            }
        }
    }
}
