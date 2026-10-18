use std::{collections::BTreeMap, sync::Arc};

use caditor_geometry::Point2;
use skrifa::{
    FontRef, GlyphId, MetadataProvider, OutlineGlyphCollection, Tag,
    instance::{Location, LocationRef, Size},
    metrics::{GlyphMetrics, Metrics},
    outline::{DrawSettings, OutlinePen},
    raw::{
        TableProvider,
        tables::gpos::{Gpos, PairPos, PositionSubtables},
    },
};

use crate::overlap::{Segment, merged};

const WEIGHT_AXIS: Tag = Tag::new(b"wght");
const KERNING: Tag = Tag::new(b"kern");
const LATIN: Tag = Tag::new(b"latn");
const DEFAULT_SCRIPT: Tag = Tag::new(b"DFLT");
const MISSING: GlyphId = GlyphId::NOTDEF;

pub struct Typeface<'f> {
    upright: Cut<'f>,
    italic: Option<Cut<'f>>,
}

struct Cut<'f> {
    font: FontRef<'f>,
    glyphs: OutlineGlyphCollection<'f>,
    positioning: Option<Gpos<'f>>,
    units_per_em: f64,
    weights: Option<(f64, f64)>,
    kerning: Vec<u16>,
    locations: BTreeMap<u16, Location>,
    outlines: BTreeMap<(u16, GlyphId), Arc<[Segment]>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instance {
    weight: u16,
    italic: bool,
}

impl<'f> Typeface<'f> {
    pub fn parse(upright: &'f [u8], italic: Option<&'f [u8]>) -> Option<Self> {
        Some(Self {
            upright: Cut::parse(upright)?,
            italic: italic.and_then(Cut::parse),
        })
    }

    pub fn has_italic(&self) -> bool {
        self.italic.is_some()
    }

    pub fn units_per_em(&self) -> f64 {
        self.upright.units_per_em
    }

    pub fn instance(&self, wanted_weight: f64, wanted_italic: bool) -> Instance {
        let italic = wanted_italic && self.italic.is_some();
        Instance {
            weight: self.cut(italic).weight(wanted_weight),
            italic,
        }
    }

    pub fn glyph(&self, character: char, instance: Instance) -> Option<GlyphId> {
        self.cut(instance.italic).glyph(character)
    }

    pub fn advance(&mut self, instance: Instance, glyph: Option<GlyphId>) -> f64 {
        let glyph = glyph.unwrap_or(MISSING);
        self.cut_mut(instance.italic)
            .advance(instance.weight, glyph)
    }

    pub fn kerning(&self, instance: Instance, first: GlyphId, second: GlyphId) -> f64 {
        self.cut(instance.italic).kerning(first, second)
    }

    pub fn outline(&mut self, instance: Instance, glyph: GlyphId) -> Arc<[Segment]> {
        self.cut_mut(instance.italic)
            .outline(instance.weight, glyph)
    }

    pub fn curve_count(&mut self, instance: Instance, glyph: GlyphId) -> usize {
        self.cut_mut(instance.italic)
            .curve_count(instance.weight, glyph)
    }

    pub fn cap_height(&mut self, instance: Instance) -> Option<f64> {
        self.cut_mut(instance.italic).cap_height(instance.weight)
    }

    pub fn line_height(&mut self, instance: Instance) -> Option<f64> {
        self.cut_mut(instance.italic).line_height(instance.weight)
    }

    fn cut(&self, italic: bool) -> &Cut<'f> {
        match (&self.italic, italic) {
            (Some(cut), true) => cut,
            _ => &self.upright,
        }
    }

    fn cut_mut(&mut self, italic: bool) -> &mut Cut<'f> {
        match (&mut self.italic, italic) {
            (Some(cut), true) => cut,
            _ => &mut self.upright,
        }
    }
}

impl<'f> Cut<'f> {
    fn parse(data: &'f [u8]) -> Option<Self> {
        let font = FontRef::from_index(data, 0).ok()?;
        let units_per_em = f64::from(font.head().ok()?.units_per_em());
        let weights = font
            .axes()
            .iter()
            .find(|axis| axis.tag() == WEIGHT_AXIS)
            .map(|axis| (f64::from(axis.min_value()), f64::from(axis.max_value())));
        let positioning = font.gpos().ok();
        let kerning = positioning
            .as_ref()
            .map(kerning_lookups)
            .unwrap_or_default();
        Some(Self {
            glyphs: font.outline_glyphs(),
            font,
            positioning,
            units_per_em,
            weights,
            kerning,
            locations: BTreeMap::new(),
            outlines: BTreeMap::new(),
        })
    }

    fn weight(&self, wanted: f64) -> u16 {
        match self.weights {
            Some((lightest, heaviest)) => {
                wanted.clamp(lightest, heaviest).round().clamp(0.0, 1000.0) as u16
            }
            None => 0,
        }
    }

    fn glyph(&self, character: char) -> Option<GlyphId> {
        self.font
            .charmap()
            .map(character)
            .filter(|glyph| *glyph != MISSING)
    }

    fn kerning(&self, first: GlyphId, second: GlyphId) -> f64 {
        let Some(lookups) = self
            .positioning
            .as_ref()
            .and_then(|positioning| positioning.lookup_list().ok())
        else {
            return 0.0;
        };
        let mut total = 0.0;
        for index in &self.kerning {
            let Ok(lookup) = lookups.lookups().get(usize::from(*index)) else {
                continue;
            };
            let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else {
                continue;
            };
            let adjusted = subtables
                .iter()
                .flatten()
                .find_map(|pair| pair_adjustment(&pair, first, second));
            total += adjusted.map_or(0.0, f64::from);
        }
        total
    }

    fn advance(&mut self, weight: u16, glyph: GlyphId) -> f64 {
        let font = self.font.clone();
        let location = self.location(weight);
        GlyphMetrics::new(&font, Size::unscaled(), location)
            .advance_width(glyph)
            .map_or(0.0, f64::from)
    }

    fn outline(&mut self, weight: u16, glyph: GlyphId) -> Arc<[Segment]> {
        if let Some(outline) = self.outlines.get(&(weight, glyph)) {
            return Arc::clone(outline);
        }
        let segments = self.segments(weight, glyph);
        let outline: Arc<[Segment]> = merged(&segments).unwrap_or(segments).into();
        self.outlines.insert((weight, glyph), Arc::clone(&outline));
        outline
    }

    fn curve_count(&mut self, weight: u16, glyph: GlyphId) -> usize {
        match self.outlines.get(&(weight, glyph)) {
            Some(outline) => outline.len(),
            None => self.segments(weight, glyph).len(),
        }
    }

    fn cap_height(&mut self, weight: u16) -> Option<f64> {
        let font = self.font.clone();
        let location = self.location(weight);
        Metrics::new(&font, Size::unscaled(), location)
            .cap_height
            .map(f64::from)
            .filter(|height| height.is_finite() && *height > 0.0)
    }

    fn line_height(&mut self, weight: u16) -> Option<f64> {
        let font = self.font.clone();
        let location = self.location(weight);
        let metrics = Metrics::new(&font, Size::unscaled(), location);
        let height =
            f64::from(metrics.ascent) - f64::from(metrics.descent) + f64::from(metrics.leading);
        (height.is_finite() && height > 0.0).then_some(height)
    }

    fn segments(&mut self, weight: u16, glyph: GlyphId) -> Vec<Segment> {
        let mut pen = Pen::default();
        let glyphs = self.glyphs.clone();
        let location = self.location(weight);
        if let Some(outline) = glyphs.get(glyph) {
            let settings = DrawSettings::unhinted(Size::unscaled(), location);
            let _ = outline.draw(settings, &mut pen);
        }
        pen.segments
    }

    fn location(&mut self, weight: u16) -> LocationRef<'_> {
        let font = &self.font;
        self.locations
            .entry(weight)
            .or_insert_with(|| {
                if weight > 0 {
                    font.axes().location([("wght", f32::from(weight))])
                } else {
                    Location::default()
                }
            })
            .coords()
            .into()
    }
}

fn kerning_lookups(positioning: &Gpos<'_>) -> Vec<u16> {
    let (Ok(scripts), Ok(features)) = (positioning.script_list(), positioning.feature_list())
    else {
        return Vec::new();
    };
    let script = scripts
        .index_for_tag(LATIN)
        .or_else(|| scripts.index_for_tag(DEFAULT_SCRIPT))
        .and_then(|index| scripts.get(index).ok());
    let language = script
        .as_ref()
        .and_then(|script| script.default_lang_sys())
        .and_then(Result::ok);
    let Some(language) = language else {
        return Vec::new();
    };
    let mut lookups: Vec<u16> = language
        .feature_indices()
        .iter()
        .filter_map(|index| features.get(index.get()).ok())
        .filter(|feature| feature.tag == KERNING)
        .flat_map(|feature| {
            feature
                .lookup_list_indices()
                .iter()
                .map(|index| index.get())
                .collect::<Vec<_>>()
        })
        .collect();
    lookups.sort_unstable();
    lookups.dedup();
    lookups
}

fn pair_adjustment(pair: &PairPos<'_>, first: GlyphId, second: GlyphId) -> Option<i16> {
    match pair {
        PairPos::Format1(table) => {
            let index = table.coverage().ok()?.get(first)?;
            let set = table.pair_sets().get(usize::from(index)).ok()?;
            let second = second.to_u32();
            set.pair_value_records()
                .iter()
                .flatten()
                .find(|record| record.second_glyph().to_u32() == second)
                .and_then(|record| record.value_record1().x_advance())
        }
        PairPos::Format2(table) => {
            table.coverage().ok()?.get(first)?;
            let first_class = table.class_def1().ok()?.get(first);
            let second_class = table.class_def2().ok()?.get(second);
            let class1 = table.class1_records().get(usize::from(first_class)).ok()?;
            let class2 = class1
                .class2_records()
                .get(usize::from(second_class))
                .ok()?;
            class2.value_record1().x_advance()
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

impl OutlinePen for Pen {
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
