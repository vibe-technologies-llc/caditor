use std::f32::consts::FRAC_PI_8;

pub const FONT_NAME: &str = "caditor-icons";
pub const FILLET: &str = "\u{F8E0}";
pub const CHAMFER: &str = "\u{F8E1}";
pub const SHELL: &str = "\u{F8E2}";
pub const EXTRUDE: &str = "\u{F8E3}";
pub const REVOLVE: &str = "\u{F8E4}";
pub const LINEAR_PATTERN: &str = "\u{F8E5}";
pub const CIRCULAR_PATTERN: &str = "\u{F8E6}";

const UNITS_PER_EM: u16 = 1024;
const UNITS_PER_GRID_STEP: f32 = 4.0;
const GRID_BASELINE: f32 = 240.0;
const HALF_STROKE: f32 = 8.0;
const ASCENT: i16 = 960;
const DESCENT: i16 = -64;
const ARC_STEP_DEGREES: f32 = 5.0;
const ARROW_HEAD: f32 = 40.0;
const CIRCLE_SEGMENTS: usize = 8;

type Point = (f32, f32);

enum Mark {
    Path(&'static [Point]),
    Loop(&'static [Point]),
    Arc {
        centre: Point,
        radii: Point,
        start: f32,
        sweep: f32,
        arrow: bool,
    },
    Fill(&'static [Point]),
    Dot(Point, f32),
    Ring(Point, f32),
}

struct Icon {
    glyph: &'static str,
    marks: &'static [Mark],
}

const ICONS: [Icon; 7] = [
    Icon {
        glyph: FILLET,
        marks: &[
            Mark::Path(&[(40.0, 216.0), (40.0, 136.0)]),
            Mark::Arc {
                centre: (136.0, 136.0),
                radii: (96.0, 96.0),
                start: 180.0,
                sweep: 90.0,
                arrow: false,
            },
            Mark::Path(&[(136.0, 40.0), (216.0, 40.0), (216.0, 216.0), (40.0, 216.0)]),
            Mark::Dot((40.0, 40.0), 12.0),
        ],
    },
    Icon {
        glyph: CHAMFER,
        marks: &[
            Mark::Loop(&[
                (40.0, 216.0),
                (40.0, 112.0),
                (112.0, 40.0),
                (216.0, 40.0),
                (216.0, 216.0),
            ]),
            Mark::Dot((40.0, 40.0), 12.0),
        ],
    },
    Icon {
        glyph: SHELL,
        marks: &[
            Mark::Loop(&[
                (128.0, 32.0),
                (224.0, 80.0),
                (224.0, 176.0),
                (128.0, 224.0),
                (32.0, 176.0),
                (32.0, 80.0),
            ]),
            Mark::Path(&[(32.0, 80.0), (128.0, 128.0), (224.0, 80.0)]),
            Mark::Path(&[(128.0, 128.0), (128.0, 224.0)]),
            Mark::Loop(&[(128.0, 56.0), (176.0, 80.0), (128.0, 104.0), (80.0, 80.0)]),
        ],
    },
    Icon {
        glyph: EXTRUDE,
        marks: &[
            Mark::Loop(&[(24.0, 208.0), (80.0, 168.0), (232.0, 168.0), (176.0, 208.0)]),
            Mark::Path(&[(128.0, 184.0), (128.0, 40.0)]),
            Mark::Path(&[(88.0, 80.0), (128.0, 40.0), (168.0, 80.0)]),
        ],
    },
    Icon {
        glyph: REVOLVE,
        marks: &[
            Mark::Path(&[(128.0, 24.0), (128.0, 92.0)]),
            Mark::Path(&[(128.0, 124.0), (128.0, 232.0)]),
            Mark::Arc {
                centre: (128.0, 144.0),
                radii: (96.0, 36.0),
                start: 200.0,
                sweep: 290.0,
                arrow: true,
            },
        ],
    },
    Icon {
        glyph: LINEAR_PATTERN,
        marks: &[
            Mark::Fill(&[(32.0, 56.0), (88.0, 56.0), (88.0, 112.0), (32.0, 112.0)]),
            Mark::Loop(&[(104.0, 64.0), (152.0, 64.0), (152.0, 112.0), (104.0, 112.0)]),
            Mark::Loop(&[(176.0, 64.0), (224.0, 64.0), (224.0, 112.0), (176.0, 112.0)]),
            Mark::Path(&[(40.0, 176.0), (216.0, 176.0)]),
            Mark::Path(&[(184.0, 144.0), (216.0, 176.0), (184.0, 208.0)]),
        ],
    },
    Icon {
        glyph: CIRCULAR_PATTERN,
        marks: &[
            Mark::Dot((128.0, 128.0), 12.0),
            Mark::Dot((128.0, 44.0), 24.0),
            Mark::Ring((208.0, 102.0), 24.0),
            Mark::Ring((177.0, 196.0), 24.0),
            Mark::Ring((79.0, 196.0), 24.0),
            Mark::Ring((48.0, 102.0), 24.0),
        ],
    },
];

#[derive(Debug, Clone, Copy, PartialEq)]
struct OutlinePoint {
    x: i16,
    y: i16,
    on_curve: bool,
}

type Contour = Vec<OutlinePoint>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Winding {
    Filled,
    Hole,
}

fn to_font((x, y): Point) -> (i16, i16) {
    (
        (x * UNITS_PER_GRID_STEP).round() as i16,
        ((GRID_BASELINE - y) * UNITS_PER_GRID_STEP).round() as i16,
    )
}

fn on_curve(point: Point) -> OutlinePoint {
    let (x, y) = to_font(point);
    OutlinePoint {
        x,
        y,
        on_curve: true,
    }
}

fn polygon(points: &[Point]) -> Contour {
    points.iter().copied().map(on_curve).collect()
}

fn circle((cx, cy): Point, radius: f32) -> Contour {
    let control = radius / FRAC_PI_8.cos();
    (0..CIRCLE_SEGMENTS * 2)
        .map(|step| {
            let angle = step as f32 * FRAC_PI_8;
            let reach = if step % 2 == 0 { radius } else { control };
            let (x, y) = to_font((cx + reach * angle.cos(), cy + reach * angle.sin()));
            OutlinePoint {
                x,
                y,
                on_curve: step % 2 == 0,
            }
        })
        .collect()
}

fn signed_area(contour: &[OutlinePoint]) -> f32 {
    let next = contour.iter().cycle().skip(1);
    contour
        .iter()
        .zip(next)
        .map(|(a, b)| f32::from(a.x) * f32::from(b.y) - f32::from(b.x) * f32::from(a.y))
        .sum::<f32>()
        / 2.0
}

fn oriented(mut contour: Contour, winding: Winding) -> Contour {
    let clockwise = signed_area(&contour) < 0.0;
    if clockwise != (winding == Winding::Filled) {
        contour.reverse();
    }
    contour
}

fn normal((ax, ay): Point, (bx, by): Point) -> Option<Point> {
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx.hypot(dy);
    (length > f32::EPSILON).then(|| (-dy / length * HALF_STROKE, dx / length * HALF_STROKE))
}

fn stroke(points: &[Point], closed: bool) -> Vec<Contour> {
    let closing = closed
        .then(|| points.last().copied().zip(points.first().copied()))
        .flatten();
    let segments = points
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] => Some((*a, *b)),
            _ => None,
        })
        .chain(closing);
    let bands = segments.filter_map(|(a, b)| {
        let (nx, ny) = normal(a, b)?;
        Some(polygon(&[
            (a.0 + nx, a.1 + ny),
            (b.0 + nx, b.1 + ny),
            (b.0 - nx, b.1 - ny),
            (a.0 - nx, a.1 - ny),
        ]))
    });
    let joints = points.iter().map(|point| circle(*point, HALF_STROKE));
    bands.chain(joints).collect()
}

fn arc_points(centre: Point, radii: Point, start: f32, sweep: f32) -> Vec<Point> {
    let steps = (sweep.abs() / ARC_STEP_DEGREES).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|step| {
            let angle = (start + sweep * step as f32 / steps as f32).to_radians();
            (
                centre.0 + radii.0 * angle.cos(),
                centre.1 + radii.1 * angle.sin(),
            )
        })
        .collect()
}

fn ribbon(points: &[Point]) -> Contour {
    let offsets: Vec<(Point, Point)> = points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            let before = index.checked_sub(1).and_then(|before| points.get(before));
            let after = points.get(index + 1);
            let (from, to) = match (before, after) {
                (Some(before), Some(after)) => (*before, *after),
                (None, Some(after)) => (*point, *after),
                (Some(before), None) => (*before, *point),
                (None, None) => return None,
            };
            let (nx, ny) = normal(from, to)?;
            Some(((point.0 + nx, point.1 + ny), (point.0 - nx, point.1 - ny)))
        })
        .collect();
    let left = offsets.iter().map(|(left, _)| *left);
    let right = offsets.iter().rev().map(|(_, right)| *right);
    polygon(&left.chain(right).collect::<Vec<_>>())
}

fn arrow_head(points: &[Point]) -> Vec<Contour> {
    let (Some(tip), Some(before)) = (points.last(), points.iter().rev().nth(1)) else {
        return Vec::new();
    };
    let heading = (tip.1 - before.1).atan2(tip.0 - before.0);
    let wing = |turn: f32| {
        let angle = heading + turn;
        (
            tip.0 - ARROW_HEAD * angle.cos(),
            tip.1 - ARROW_HEAD * angle.sin(),
        )
    };
    let quarter = std::f32::consts::FRAC_PI_4;
    stroke(&[wing(-quarter), *tip, wing(quarter)], false)
}

fn filled(contours: Vec<Contour>) -> Vec<(Contour, Winding)> {
    contours
        .into_iter()
        .map(|contour| (contour, Winding::Filled))
        .collect()
}

fn contours(mark: &Mark) -> Vec<(Contour, Winding)> {
    match mark {
        Mark::Path(points) => filled(stroke(points, false)),
        Mark::Loop(points) => filled(stroke(points, true)),
        Mark::Fill(points) => filled(vec![polygon(points)]),
        Mark::Dot(centre, radius) => filled(vec![circle(*centre, *radius)]),
        Mark::Ring(centre, radius) => vec![
            (circle(*centre, radius + HALF_STROKE), Winding::Filled),
            (circle(*centre, radius - HALF_STROKE), Winding::Hole),
        ],
        Mark::Arc {
            centre,
            radii,
            start,
            sweep,
            arrow,
        } => {
            let points = arc_points(*centre, *radii, *start, *sweep);
            let caps = [points.first(), points.last()]
                .into_iter()
                .flatten()
                .map(|end| circle(*end, HALF_STROKE));
            let head = if *arrow {
                arrow_head(&points)
            } else {
                Vec::new()
            };
            filled(
                std::iter::once(ribbon(&points))
                    .chain(caps)
                    .chain(head)
                    .collect(),
            )
        }
    }
}

fn outline(icon: &Icon) -> Vec<(Contour, Winding)> {
    icon.marks
        .iter()
        .flat_map(contours)
        .map(|(contour, winding)| (oriented(contour, winding), winding))
        .collect()
}

fn contours_of(icon: &Icon) -> Vec<Contour> {
    outline(icon)
        .into_iter()
        .map(|(contour, _)| contour)
        .collect()
}

struct Bounds {
    x_min: i16,
    y_min: i16,
    x_max: i16,
    y_max: i16,
}

fn bounds(contours: &[Contour]) -> Bounds {
    let points = contours.iter().flatten();
    Bounds {
        x_min: points
            .clone()
            .map(|point| point.x)
            .min()
            .unwrap_or_default(),
        y_min: points
            .clone()
            .map(|point| point.y)
            .min()
            .unwrap_or_default(),
        x_max: points
            .clone()
            .map(|point| point.x)
            .max()
            .unwrap_or_default(),
        y_max: points.map(|point| point.y).max().unwrap_or_default(),
    }
}

trait Write {
    fn u16(&mut self, value: u16);
    fn i16(&mut self, value: i16);
    fn u32(&mut self, value: u32);
    fn pad(&mut self);
}

impl Write for Vec<u8> {
    fn u16(&mut self, value: u16) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn i16(&mut self, value: i16) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn pad(&mut self) {
        while !self.len().is_multiple_of(4) {
            self.push(0);
        }
    }
}

fn count(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

fn simple_glyph(contours: &[Contour]) -> Vec<u8> {
    let mut data = Vec::new();
    if contours.is_empty() {
        return data;
    }
    let bounds = bounds(contours);
    data.i16(i16::try_from(contours.len()).unwrap_or(i16::MAX));
    data.i16(bounds.x_min);
    data.i16(bounds.y_min);
    data.i16(bounds.x_max);
    data.i16(bounds.y_max);
    let mut end = 0;
    for contour in contours {
        end += contour.len();
        data.u16(count(end.saturating_sub(1)));
    }
    data.u16(0);
    let points: Vec<OutlinePoint> = contours.iter().flatten().copied().collect();
    data.extend(points.iter().map(|point| u8::from(point.on_curve)));
    let mut previous = OutlinePoint {
        x: 0,
        y: 0,
        on_curve: true,
    };
    let mut ys = Vec::new();
    for point in &points {
        data.i16(point.x.wrapping_sub(previous.x));
        ys.i16(point.y.wrapping_sub(previous.y));
        previous = *point;
    }
    data.extend(ys);
    data.pad();
    data
}

struct Glyphs {
    data: Vec<Vec<u8>>,
    points: u16,
    contours: u16,
    bounds: Bounds,
}

fn glyphs() -> Glyphs {
    let outlines: Vec<Vec<Contour>> = ICONS.iter().map(contours_of).collect();
    let all: Vec<Contour> = outlines.iter().flatten().cloned().collect();
    let data = std::iter::once(Vec::new())
        .chain(outlines.iter().map(|contours| simple_glyph(contours)))
        .collect();
    Glyphs {
        data,
        points: count(
            outlines
                .iter()
                .map(|contours| contours.iter().map(Vec::len).sum::<usize>())
                .max()
                .unwrap_or_default(),
        ),
        contours: count(outlines.iter().map(Vec::len).max().unwrap_or_default()),
        bounds: bounds(&all),
    }
}

fn code_points() -> Vec<u16> {
    ICONS
        .iter()
        .filter_map(|icon| icon.glyph.chars().next())
        .map(|glyph| u16::try_from(u32::from(glyph)).unwrap_or_default())
        .collect()
}

fn cmap() -> Vec<u8> {
    let mut segments: Vec<(u16, u16)> = code_points()
        .into_iter()
        .zip(1u16..)
        .map(|(code, glyph)| (code, glyph.wrapping_sub(code)))
        .collect();
    segments.push((u16::MAX, 1));
    let segment_count = count(segments.len());
    let entry_selector = segment_count.ilog2() as u16;
    let search_range = 2 * (1u16 << entry_selector);
    let mut table = Vec::new();
    table.u16(0);
    table.u16(1);
    table.u16(3);
    table.u16(1);
    table.u32(12);
    table.u16(4);
    table.u16(16 + 8 * segment_count);
    table.u16(0);
    table.u16(segment_count * 2);
    table.u16(search_range);
    table.u16(entry_selector);
    table.u16(segment_count * 2 - search_range);
    for (code, _) in &segments {
        table.u16(*code);
    }
    table.u16(0);
    for (code, _) in &segments {
        table.u16(*code);
    }
    for (_, delta) in &segments {
        table.u16(*delta);
    }
    for _ in &segments {
        table.u16(0);
    }
    table
}

fn head(bounds: &Bounds) -> Vec<u8> {
    let mut table = Vec::new();
    table.u32(0x0001_0000);
    table.u32(0x0001_0000);
    table.u32(0);
    table.u32(0x5F0F_3CF5);
    table.u16(0x000B);
    table.u16(UNITS_PER_EM);
    table.extend([0; 16]);
    table.i16(bounds.x_min);
    table.i16(bounds.y_min);
    table.i16(bounds.x_max);
    table.i16(bounds.y_max);
    table.u16(0);
    table.u16(8);
    table.i16(2);
    table.i16(1);
    table.i16(0);
    table
}

fn hhea(glyph_count: u16, bounds: &Bounds) -> Vec<u8> {
    let mut table = Vec::new();
    table.u32(0x0001_0000);
    table.i16(ASCENT);
    table.i16(DESCENT);
    table.i16(0);
    table.u16(UNITS_PER_EM);
    table.i16(bounds.x_min);
    table.i16(i16::try_from(UNITS_PER_EM).unwrap_or(i16::MAX) - bounds.x_max);
    table.i16(bounds.x_max);
    table.i16(1);
    table.i16(0);
    table.i16(0);
    table.extend([0; 8]);
    table.i16(0);
    table.u16(glyph_count);
    table
}

fn maxp(glyphs: &Glyphs) -> Vec<u8> {
    let mut table = Vec::new();
    table.u32(0x0001_0000);
    table.u16(count(glyphs.data.len()));
    table.u16(glyphs.points);
    table.u16(glyphs.contours);
    table.u16(0);
    table.u16(0);
    table.u16(2);
    table.extend([0; 16]);
    table
}

fn hmtx(glyphs: &Glyphs) -> Vec<u8> {
    let mut table = Vec::new();
    for _ in &glyphs.data {
        table.u16(UNITS_PER_EM);
        table.i16(0);
    }
    table
}

fn post() -> Vec<u8> {
    let mut table = Vec::new();
    table.u32(0x0003_0000);
    table.u32(0);
    table.i16(-100);
    table.i16(50);
    table.extend([0; 20]);
    table
}

fn checksum(table: &[u8]) -> u32 {
    table.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0; 4];
        for (slot, byte) in word.iter_mut().zip(chunk) {
            *slot = *byte;
        }
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

pub fn font() -> Vec<u8> {
    let glyphs = glyphs();
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for data in &glyphs.data {
        loca.u32(u32::try_from(glyf.len()).unwrap_or(u32::MAX));
        glyf.extend(data);
    }
    loca.u32(u32::try_from(glyf.len()).unwrap_or(u32::MAX));
    let glyph_count = count(glyphs.data.len());
    let tables: [(&[u8; 4], Vec<u8>); 8] = [
        (b"cmap", cmap()),
        (b"glyf", glyf),
        (b"head", head(&glyphs.bounds)),
        (b"hhea", hhea(glyph_count, &glyphs.bounds)),
        (b"hmtx", hmtx(&glyphs)),
        (b"loca", loca),
        (b"maxp", maxp(&glyphs)),
        (b"post", post()),
    ];
    let table_count = count(tables.len());
    let entry_selector = table_count.ilog2() as u16;
    let search_range = 16 * (1u16 << entry_selector);
    let mut font = Vec::new();
    font.u32(0x0001_0000);
    font.u16(table_count);
    font.u16(search_range);
    font.u16(entry_selector);
    font.u16(table_count * 16 - search_range);
    let mut offset = 12 + 16 * tables.len();
    let mut bodies = Vec::new();
    for (tag, table) in &tables {
        font.extend_from_slice(*tag);
        font.u32(checksum(table));
        font.u32(u32::try_from(offset).unwrap_or(u32::MAX));
        font.u32(u32::try_from(table.len()).unwrap_or(u32::MAX));
        let mut padded = table.clone();
        padded.pad();
        offset += padded.len();
        bodies.extend(padded);
    }
    font.extend(bodies);
    font
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_private_use_character_of_its_own() {
        let codes = code_points();

        assert_eq!(codes.len(), ICONS.len());
        assert!(codes.iter().all(|code| (0xE000..=0xF8FF).contains(code)));
        assert!(codes.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn outlines_stay_inside_the_phosphor_square() {
        for icon in &ICONS {
            let contours = contours_of(icon);
            let bounds = bounds(&contours);

            assert!(!contours.is_empty());
            assert!(bounds.x_min >= 0 && bounds.x_max <= 1024);
            assert!(bounds.y_min >= DESCENT && bounds.y_max <= ASCENT);
        }
    }

    #[test]
    fn filled_contours_turn_clockwise_and_holes_the_other_way() {
        for icon in &ICONS {
            for (contour, winding) in outline(icon) {
                let area = signed_area(&contour);

                assert!(area != 0.0);
                assert_eq!(area < 0.0, winding == Winding::Filled);
            }
        }
    }
}
