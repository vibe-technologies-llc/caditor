use caditor_geometry::{Point2, Vector2};
use caditor_kernel::ProfileCurve;
use caditor_lettering::{Alignment, Segment, Style, lettered};
use caditor_sketch::{EntityId, Faceting, Sketch, Text, TextAlign, TextSize, letter_curve};

const MAX_BEZIER_STEPS: f64 = 64.0;

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedLetter {
    pub position: usize,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlacedText {
    pub letters: Vec<PlacedLetter>,
    pub missing: Vec<char>,
}

impl PlacedText {
    pub fn polylines(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        self.letters
            .iter()
            .flat_map(|letter| &letter.segments)
            .map(|segment| faceted(segment, faceting))
            .collect()
    }

    fn curves(&self, text: EntityId) -> impl Iterator<Item = ProfileCurve> + '_ {
        self.letters.iter().flat_map(move |letter| {
            letter
                .segments
                .iter()
                .enumerate()
                .filter_map(move |(index, segment)| {
                    Some(segment.profile_curve(letter_curve(text, letter.position, index)?))
                })
        })
    }
}

pub fn lettered_at(text: &Text, anchor: Point2, size: TextSize) -> Option<PlacedText> {
    let style = Style {
        bold: text.bold,
        italic: text.italic,
    };
    let alignment = match text.align {
        TextAlign::Left => Alignment::Left,
        TextAlign::Centre => Alignment::Centre,
        TextAlign::Right => Alignment::Right,
    };
    let lettered = lettered(&text.content, style, alignment).ok()?;
    let (sin, cos) = size.angle.to_radians().sin_cos();
    let place = |point: Point2| {
        anchor
            + Vector2::new(cos * point.x - sin * point.y, sin * point.x + cos * point.y)
                * size.height
    };
    Some(PlacedText {
        letters: lettered
            .letters
            .iter()
            .map(|letter| PlacedLetter {
                position: letter.position,
                segments: letter
                    .segments
                    .iter()
                    .map(|segment| segment.mapped(place))
                    .collect(),
            })
            .collect(),
        missing: lettered.missing,
    })
}

pub fn placed_text(sketch: &Sketch, id: EntityId) -> Option<PlacedText> {
    let text = sketch.text(id)?;
    lettered_at(text, sketch.point(text.anchor)?, sketch.text_size(id)?)
}

pub(crate) fn text_curves(sketch: &Sketch) -> impl Iterator<Item = ProfileCurve> + '_ {
    sketch.texts().flat_map(|(id, _)| {
        placed_text(sketch, id)
            .map(|placed| placed.curves(id).collect::<Vec<_>>())
            .unwrap_or_default()
    })
}

fn faceted(segment: &Segment, faceting: Faceting) -> Vec<Point2> {
    let steps = match segment {
        Segment::Line(..) => 1,
        Segment::Bezier(points) => {
            let degree = points.len().saturating_sub(1) as f64;
            let bend = points
                .windows(3)
                .map(|triple| match triple {
                    [first, middle, last] => (*first - *middle * 2.0 + *last).length(),
                    _ => 0.0,
                })
                .fold(0.0, f64::max);
            let deviation = bend * degree * (degree - 1.0).max(0.0) / 8.0;
            (deviation / faceting.chord())
                .sqrt()
                .ceil()
                .clamp(1.0, MAX_BEZIER_STEPS) as usize
        }
    };
    segment.sampled(steps)
}
