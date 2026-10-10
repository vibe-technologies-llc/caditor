use std::{collections::BTreeMap, rc::Rc};

use caditor_geometry::Point2;
use ttf_parser::{
    Face, GlyphId, OutlineBuilder, Tag,
    gpos::{PairAdjustment, PositioningSubtable},
};

use crate::import::svg::{
    overlap::{Segment, merged, traced},
    path::Outline,
};

const WEIGHT_AXIS: Tag = Tag::from_bytes(b"wght");
const KERNING: Tag = Tag::from_bytes(b"kern");
const LATIN: Tag = Tag::from_bytes(b"latn");
const DEFAULT_SCRIPT: Tag = Tag::from_bytes(b"DFLT");
const MISSING: GlyphId = GlyphId(0);

pub(super) struct Typeface<'f> {
    face: Face<'f>,
    units_per_em: f64,
    weights: Option<(f64, f64)>,
    kerning: Vec<u16>,
    weighted: BTreeMap<u16, Face<'f>>,
    outlines: BTreeMap<(u16, GlyphId), Rc<Outline>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Weight(u16);

impl<'f> Typeface<'f> {
    pub fn parse(data: &'f [u8]) -> Option<Self> {
        let face = Face::parse(data, 0).ok()?;
        let units_per_em = f64::from(face.units_per_em());
        let weights = face
            .variation_axes()
            .into_iter()
            .find(|axis| axis.tag == WEIGHT_AXIS)
            .map(|axis| (f64::from(axis.min_value), f64::from(axis.max_value)));
        let kerning = kerning_lookups(&face);
        Some(Self {
            face,
            units_per_em,
            weights,
            kerning,
            weighted: BTreeMap::new(),
            outlines: BTreeMap::new(),
        })
    }

    pub fn units_per_em(&self) -> f64 {
        self.units_per_em
    }

    pub fn weight(&self, wanted: f64) -> Weight {
        match self.weights {
            Some((lightest, heaviest)) => {
                Weight(wanted.clamp(lightest, heaviest).round().clamp(0.0, 1000.0) as u16)
            }
            None => Weight(0),
        }
    }

    pub fn glyph(&self, character: char) -> Option<GlyphId> {
        self.face
            .glyph_index(character)
            .filter(|glyph| *glyph != MISSING)
    }

    pub fn advance(&mut self, weight: Weight, glyph: Option<GlyphId>) -> f64 {
        let glyph = glyph.unwrap_or(MISSING);
        self.weighted(weight)
            .glyph_hor_advance(glyph)
            .map_or(0.0, f64::from)
    }

    pub fn kerning(&self, first: GlyphId, second: GlyphId) -> f64 {
        let Some(positioning) = self.face.tables().gpos else {
            return 0.0;
        };
        let mut total = 0.0;
        for index in &self.kerning {
            let Some(lookup) = positioning.lookups.get(*index) else {
                continue;
            };
            let adjusted = lookup
                .subtables
                .into_iter::<PositioningSubtable<'_>>()
                .find_map(|subtable| match subtable {
                    PositioningSubtable::Pair(pair) => pair_adjustment(pair, first, second),
                    _ => None,
                });
            total += adjusted.map_or(0.0, f64::from);
        }
        total
    }

    pub fn outline(&mut self, weight: Weight, glyph: GlyphId) -> Rc<Outline> {
        if let Some(outline) = self.outlines.get(&(weight.0, glyph)) {
            return Rc::clone(outline);
        }
        let segments = self.segments(weight, glyph);
        let outline = Rc::new(merged(&segments).unwrap_or_else(|| traced(&segments)));
        self.outlines.insert((weight.0, glyph), Rc::clone(&outline));
        outline
    }

    pub fn curve_count(&mut self, weight: Weight, glyph: GlyphId) -> usize {
        match self.outlines.get(&(weight.0, glyph)) {
            Some(outline) => outline.shapes.len(),
            None => self.segments(weight, glyph).len(),
        }
    }

    fn segments(&mut self, weight: Weight, glyph: GlyphId) -> Vec<Segment> {
        let mut pen = Pen::default();
        self.weighted(weight).outline_glyph(glyph, &mut pen);
        pen.segments
    }

    fn weighted(&mut self, weight: Weight) -> &Face<'f> {
        let base = &self.face;
        self.weighted.entry(weight.0).or_insert_with(|| {
            let mut face = base.clone();
            if weight.0 > 0 {
                face.set_variation(WEIGHT_AXIS, f32::from(weight.0));
            }
            face
        })
    }
}

fn kerning_lookups(face: &Face<'_>) -> Vec<u16> {
    let Some(positioning) = face.tables().gpos else {
        return Vec::new();
    };
    let language = positioning
        .scripts
        .find(LATIN)
        .or_else(|| positioning.scripts.find(DEFAULT_SCRIPT))
        .and_then(|script| script.default_language);
    let Some(language) = language else {
        return Vec::new();
    };
    let mut lookups: Vec<u16> = language
        .feature_indices
        .into_iter()
        .filter_map(|index| positioning.features.get(index))
        .filter(|feature| feature.tag == KERNING)
        .flat_map(|feature| feature.lookup_indices)
        .collect();
    lookups.sort_unstable();
    lookups.dedup();
    lookups
}

fn pair_adjustment(pair: PairAdjustment<'_>, first: GlyphId, second: GlyphId) -> Option<i16> {
    match pair {
        PairAdjustment::Format1 { coverage, sets } => {
            let index = coverage.get(first)?;
            let (record, _) = sets.get(index)?.get(second)?;
            Some(record.x_advance)
        }
        PairAdjustment::Format2 {
            coverage,
            classes,
            matrix,
        } => {
            if !coverage.contains(first) {
                return None;
            }
            let (record, _) = matrix.get((classes.0.get(first), classes.1.get(second)))?;
            Some(record.x_advance)
        }
    }
}

#[derive(Default)]
struct Pen {
    segments: Vec<Segment>,
    start: Point2,
    at: Point2,
}

impl Pen {
    fn reach(&mut self, segment: Segment, to: Point2) {
        let drawn = match &segment {
            Segment::Line(from, to) => from != to,
            Segment::Bezier(points) => points.iter().any(|point| *point != self.at),
        };
        if drawn {
            self.segments.push(segment);
        }
        self.at = to;
    }
}

fn point(x: f32, y: f32) -> Point2 {
    Point2::new(f64::from(x), f64::from(y))
}

impl OutlineBuilder for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = point(x, y);
        self.at = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = point(x, y);
        self.reach(Segment::Line(self.at, to), to);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let to = point(x, y);
        self.reach(Segment::Bezier(vec![self.at, point(x1, y1), to]), to);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let to = point(x, y);
        let points = vec![self.at, point(x1, y1), point(x2, y2), to];
        self.reach(Segment::Bezier(points), to);
    }

    fn close(&mut self) {
        let start = self.start;
        self.reach(Segment::Line(self.at, start), start);
    }
}
