use std::f64::consts::TAU;

use caditor_geometry::{Point2, Point3, Vector2};

use crate::import::{
    dxf::geometry::{Nurbs, Shape},
    svg::syntax::Scanner,
};

const STRAIGHTNESS: f64 = 1e-9;

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Outline {
    pub shapes: Vec<Shape>,
    pub damaged: bool,
}

impl Outline {
    pub fn line(&mut self, from: Point2, to: Point2) {
        if from != to {
            self.shapes.push(Shape::Line(flat(from), flat(to)));
        }
    }

    pub fn bezier(&mut self, points: &[Point2]) {
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            return;
        };
        if points.iter().all(|point| point == first) {
            return;
        }
        if is_straight(points, *first, *last) {
            self.line(*first, *last);
            return;
        }
        let count = points.len();
        let knots = [vec![0.0; count], vec![1.0; count]].concat();
        let control_points = points.iter().copied().map(flat).collect();
        if let Some(nurbs) = Nurbs::new(count - 1, knots, control_points, None) {
            self.shapes.push(Shape::Spline(nurbs));
        } else {
            self.damaged = true;
        }
    }

    pub fn ellipse(
        &mut self,
        center: Point2,
        major: Vector2,
        minor: Vector2,
        start: f64,
        sweep: f64,
    ) {
        if sweep != 0.0 && major != Vector2::ZERO && minor != Vector2::ZERO {
            self.shapes.push(Shape::Conic {
                center: flat(center),
                major: major.extend(0.0),
                minor: minor.extend(0.0),
                start,
                sweep,
            });
        }
    }

    fn arc(&mut self, from: Point2, to: Point2, arc: Arc) {
        if from == to {
            return;
        }
        let (rx, ry) = (arc.radii.x.abs(), arc.radii.y.abs());
        if rx == 0.0 || ry == 0.0 {
            self.line(from, to);
            return;
        }
        let (sin, cos) = arc.rotation.to_radians().sin_cos();
        let half = (from - to) / 2.0;
        let local = Vector2::new(cos * half.x + sin * half.y, -sin * half.x + cos * half.y);
        let reach = (local.x / rx).powi(2) + (local.y / ry).powi(2);
        let (rx, ry) = if reach > 1.0 {
            (rx * reach.sqrt(), ry * reach.sqrt())
        } else {
            (rx, ry)
        };
        let numerator = (rx * ry).powi(2) - (rx * local.y).powi(2) - (ry * local.x).powi(2);
        let denominator = (rx * local.y).powi(2) + (ry * local.x).powi(2);
        let root = if denominator > 0.0 {
            (numerator / denominator).max(0.0).sqrt()
        } else {
            0.0
        };
        let sign = if arc.large == arc.sweep { -1.0 } else { 1.0 };
        let centre_local = Vector2::new(rx * local.y / ry, -ry * local.x / rx) * (sign * root);
        let middle = (from + to) / 2.0;
        let center = middle
            + Vector2::new(
                cos * centre_local.x - sin * centre_local.y,
                sin * centre_local.x + cos * centre_local.y,
            );
        let angle = |vector: Vector2| (vector.y / ry).atan2(vector.x / rx);
        let start = angle(local - centre_local);
        let end = angle(-local - centre_local);
        let mut turn = (end - start).rem_euclid(TAU);
        if !arc.sweep && turn > 0.0 {
            turn -= TAU;
        }
        let major = Vector2::new(cos, sin) * rx;
        let minor = Vector2::new(-sin, cos) * ry;
        if turn < 0.0 {
            self.ellipse(center, major, -minor, -start, -turn);
        } else {
            self.ellipse(center, major, minor, start, turn);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Arc {
    radii: Vector2,
    rotation: f64,
    large: bool,
    sweep: bool,
}

fn flat(point: Point2) -> Point3 {
    point.extend(0.0)
}

fn is_straight(points: &[Point2], first: Point2, last: Point2) -> bool {
    let chord = last - first;
    let length_squared = chord.length_squared();
    if length_squared == 0.0 || !length_squared.is_finite() {
        return false;
    }
    let length = length_squared.sqrt();
    points.iter().all(|point| {
        let offset = *point - first;
        chord.perp_dot(offset).abs() <= STRAIGHTNESS * length_squared
            && (-STRAIGHTNESS * length_squared..=(1.0 + STRAIGHTNESS) * length_squared)
                .contains(&chord.dot(offset))
            && offset.length() <= length * (1.0 + STRAIGHTNESS)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Previous {
    Cubic,
    Quadratic,
    Other,
}

struct Pen {
    current: Point2,
    start: Point2,
    control: Point2,
    previous: Previous,
}

pub(super) fn path_outline(data: &str) -> Outline {
    let mut outline = Outline::default();
    let mut scanner = Scanner::new(data);
    let mut pen = Pen {
        current: Point2::ZERO,
        start: Point2::ZERO,
        control: Point2::ZERO,
        previous: Previous::Other,
    };
    let mut command: Option<u8> = None;
    let mut started = false;
    loop {
        if scanner.at_end() {
            return outline;
        }
        let given = scanner
            .peek()
            .filter(|byte| b"MmZzLlHhVvCcSsQqTtAa".contains(byte));
        if let Some(letter) = given {
            scanner.advance();
            command = Some(letter);
        } else if command.is_some_and(|letter| letter.eq_ignore_ascii_case(&b'z')) {
            outline.damaged = true;
            return outline;
        }
        let Some(letter) = command else {
            outline.damaged = true;
            return outline;
        };
        if !started && !letter.eq_ignore_ascii_case(&b'm') {
            outline.damaged = true;
            return outline;
        }
        started = true;
        if !segment(&mut scanner, letter, &mut pen, &mut outline) {
            outline.damaged = true;
            return outline;
        }
        command = match letter {
            b'M' => Some(b'L'),
            b'm' => Some(b'l'),
            other => Some(other),
        };
        scanner.skip_separator();
    }
}

fn segment(scanner: &mut Scanner<'_>, letter: u8, pen: &mut Pen, outline: &mut Outline) -> bool {
    let relative = letter.is_ascii_lowercase();
    let origin = if relative { pen.current } else { Point2::ZERO };
    let read = |scanner: &mut Scanner<'_>| -> Option<f64> {
        scanner.skip_space();
        let value = scanner.number();
        scanner.skip_separator();
        value
    };
    let point = |scanner: &mut Scanner<'_>| -> Option<Point2> {
        let x = read(scanner)?;
        let y = read(scanner)?;
        Some(origin + Vector2::new(x, y))
    };
    let from = pen.current;
    let (next, control, kind) = match letter.to_ascii_uppercase() {
        b'Z' => {
            outline.line(from, pen.start);
            pen.current = pen.start;
            pen.previous = Previous::Other;
            return true;
        }
        b'M' => {
            let Some(to) = point(scanner) else {
                return false;
            };
            pen.start = to;
            (to, to, Previous::Other)
        }
        b'L' => {
            let Some(to) = point(scanner) else {
                return false;
            };
            outline.line(from, to);
            (to, to, Previous::Other)
        }
        b'H' | b'V' => {
            let Some(value) = read(scanner) else {
                return false;
            };
            let to = match (letter.eq_ignore_ascii_case(&b'h'), relative) {
                (true, true) => Point2::new(from.x + value, from.y),
                (true, false) => Point2::new(value, from.y),
                (false, true) => Point2::new(from.x, from.y + value),
                (false, false) => Point2::new(from.x, value),
            };
            outline.line(from, to);
            (to, to, Previous::Other)
        }
        b'C' | b'S' => {
            let first = if letter.eq_ignore_ascii_case(&b'c') {
                point(scanner)
            } else {
                Some(reflected(pen, Previous::Cubic))
            };
            let (Some(first), Some(second), Some(to)) = (first, point(scanner), point(scanner))
            else {
                return false;
            };
            outline.bezier(&[from, first, second, to]);
            (to, second, Previous::Cubic)
        }
        b'Q' | b'T' => {
            let control = if letter.eq_ignore_ascii_case(&b'q') {
                point(scanner)
            } else {
                Some(reflected(pen, Previous::Quadratic))
            };
            let (Some(control), Some(to)) = (control, point(scanner)) else {
                return false;
            };
            outline.bezier(&[from, control, to]);
            (to, control, Previous::Quadratic)
        }
        _ => {
            let Some(arc) = arc_parameters(scanner) else {
                return false;
            };
            let Some(to) = point(scanner) else {
                return false;
            };
            outline.arc(from, to, arc);
            (to, to, Previous::Other)
        }
    };
    pen.current = next;
    pen.control = control;
    pen.previous = kind;
    true
}

fn reflected(pen: &Pen, kind: Previous) -> Point2 {
    if pen.previous == kind {
        pen.current * 2.0 - pen.control
    } else {
        pen.current
    }
}

fn arc_parameters(scanner: &mut Scanner<'_>) -> Option<Arc> {
    let mut number = || {
        scanner.skip_space();
        let value = scanner.number();
        scanner.skip_separator();
        value
    };
    let (rx, ry, rotation) = (number()?, number()?, number()?);
    scanner.skip_space();
    let large = scanner.flag()?;
    scanner.skip_separator();
    let sweep = scanner.flag()?;
    scanner.skip_separator();
    Some(Arc {
        radii: Vector2::new(rx, ry),
        rotation,
        large,
        sweep,
    })
}
