use caditor_geometry::{Point2, Vector2};
use ttf_parser::GlyphId;

use crate::import::{
    ImportError, MAX_READ_CURVES,
    svg::{
        Context, MAX_NAMED_ELEMENTS, MAX_NESTING, Walker, XLINK_NAMESPACE,
        font::{Instance, Typeface},
        path::path_outline,
        shapes::{Axis, outline_of},
        style::Inherited,
        syntax::{Length, Matrix, numbers, transform_list},
        text_path::{Side, Track},
        text_style::Anchor,
        xml::{Content, Node},
    },
};

const SPACE: char = ' ';
const MAX_FAMILY_CHARS: usize = 40;
const INTER_NAMES: [&str; 4] = ["inter", "intervariable", "intervar", "interdisplay"];
const SANS_SERIF_NAMES: [&str; 3] = ["sans-serif", "system-ui", "ui-sans-serif"];
const TRACK_SHAPES: [&str; 7] = [
    "path", "rect", "circle", "ellipse", "line", "polyline", "polygon",
];

#[derive(Debug, Clone, Copy)]
struct Letter<'a> {
    character: char,
    style: Inherited<'a>,
    track: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct Span<'a, 't> {
    node: Node<'a, 't>,
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct Gathered<'a, 't> {
    letters: Vec<Letter<'a>>,
    spans: Vec<Span<'a, 't>>,
    tracks: Vec<Track>,
    track: Option<usize>,
    after_space: bool,
    viewport: Vector2,
}

impl<'a> Gathered<'a, '_> {
    fn push_text(&mut self, text: &str, style: Inherited<'a>) {
        for character in text.chars() {
            let character = match character {
                '\n' | '\r' | '\t' => SPACE,
                character if character.is_control() => continue,
                character => character,
            };
            let is_space = character == SPACE;
            if is_space && self.after_space && !style.text.preserve_space {
                continue;
            }
            self.after_space = is_space;
            self.letters.push(Letter {
                character,
                style,
                track: self.track,
            });
        }
    }

    fn trim_end(&mut self) {
        if let Some(last) = self.letters.last()
            && last.character == SPACE
            && !last.style.text.preserve_space
        {
            self.letters.pop();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Fit {
    start: usize,
    end: usize,
    length: f64,
    glyphs: bool,
}

#[derive(Debug, Clone)]
struct Positions {
    x: Vec<Option<f64>>,
    y: Vec<Option<f64>>,
    dx: Vec<Option<f64>>,
    dy: Vec<Option<f64>>,
    rotate: Vec<Option<f64>>,
    fits: Vec<Fit>,
}

impl Positions {
    fn of(spans: &[Span<'_, '_>], count: usize, viewport: Vector2) -> Self {
        let mut positions = Self {
            x: vec![None; count],
            y: vec![None; count],
            dx: vec![None; count],
            dy: vec![None; count],
            rotate: vec![None; count],
            fits: Vec::new(),
        };
        for span in spans {
            let start = span.start.min(count);
            let end = span.end.min(count);
            let node = span.node;
            let lists = [
                (&mut positions.x, "x", Axis::Horizontal),
                (&mut positions.y, "y", Axis::Vertical),
                (&mut positions.dx, "dx", Axis::Horizontal),
                (&mut positions.dy, "dy", Axis::Vertical),
            ];
            for (resolved, name, axis) in lists {
                let values = lengths(node.attribute(name), axis.reference(viewport));
                let targets = resolved.get_mut(start..end).unwrap_or_default();
                for (target, value) in targets.iter_mut().zip(values) {
                    *target = Some(value);
                }
            }
            let (angles, _) = numbers(node.attribute("rotate").unwrap_or_default());
            if let Some(last) = angles.last().copied() {
                let targets = positions.rotate.get_mut(start..end).unwrap_or_default();
                for (index, target) in targets.iter_mut().enumerate() {
                    *target = Some(angles.get(index).copied().unwrap_or(last));
                }
            }
            let fitted = lengths(node.attribute("textLength"), viewport.x)
                .first()
                .copied()
                .filter(|length| *length >= 0.0);
            if let Some(length) = fitted
                && start < end
            {
                positions.fits.push(Fit {
                    start,
                    end,
                    length,
                    glyphs: node.attribute("lengthAdjust").map(str::trim)
                        == Some("spacingAndGlyphs"),
                });
            }
        }
        positions
    }
}

fn lengths(text: Option<&str>, reference: f64) -> Vec<f64> {
    text.unwrap_or_default()
        .split(|character: char| character == ',' || character.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map_while(|part| {
            Length::parse(part)
                .map(|length| length.pixels(reference))
                .filter(|value| value.is_finite())
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct Placed {
    at: Point2,
    advance: f64,
    glyph: Option<GlyphId>,
    instance: Instance,
    scale: f64,
    stretch: f64,
    rotate: f64,
    rise: f64,
    off_path: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    Straight,
    OnPath(usize),
    AfterPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Chunk {
    start: usize,
    end: usize,
    run: Run,
}

impl<'a, 't> Walker<'a, 't> {
    pub(super) fn text(
        &mut self,
        node: Node<'a, 't>,
        context: &Context<'a>,
    ) -> Result<(), ImportError> {
        let mut gathered = Gathered {
            letters: Vec::new(),
            spans: Vec::new(),
            tracks: Vec::new(),
            track: None,
            after_space: true,
            viewport: context.viewport,
        };
        self.gather(node, context.style, context.depth, &mut gathered);
        gathered.trim_end();
        let Gathered {
            letters,
            spans,
            tracks,
            ..
        } = gathered;
        if letters.is_empty() {
            return Ok(());
        }
        let positions = Positions::of(&spans, letters.len(), context.viewport);
        let Some(typeface) = self.typeface.as_mut() else {
            return Ok(());
        };
        let italic_carried = typeface.has_italic();
        let placed = lay_out(typeface, &letters, &positions, &tracks);
        self.note_style(&letters, italic_carried);
        let mut unpainted = false;
        let mut hidden = false;
        for (letter, placed) in letters.iter().zip(&placed) {
            let Some(glyph) = placed.glyph else {
                if !letter.character.is_whitespace() {
                    self.tally.missing_letters = self.tally.missing_letters.saturating_add(1);
                }
                continue;
            };
            if placed.off_path {
                if !letter.character.is_whitespace() {
                    self.tally.off_path = self.tally.off_path.saturating_add(1);
                }
                continue;
            }
            if !letter.style.visible {
                hidden = true;
                continue;
            }
            if letter.style.unpainted() {
                unpainted = true;
                continue;
            }
            if placed.scale <= 0.0 || placed.stretch <= 0.0 {
                continue;
            }
            let Some(typeface) = self.typeface.as_mut() else {
                return Ok(());
            };
            if self.shapes.len() >= MAX_READ_CURVES {
                let count = typeface.curve_count(placed.instance, glyph);
                self.charge(count)?;
                self.beyond_the_limit = self.beyond_the_limit.saturating_add(count);
                continue;
            }
            let outline = typeface.outline(placed.instance, glyph);
            let matrix = Matrix::scale(placed.scale * placed.stretch, -placed.scale)
                .then(&Matrix::rotation(placed.rotate))
                .then(&Matrix::translation(placed.at.x, placed.at.y))
                .then(&context.matrix);
            let lettered = Context {
                style: letter.style,
                ..context.clone()
            };
            self.place(&outline.shapes, &matrix, &lettered)?;
        }
        if hidden {
            self.tally.hidden += 1;
        }
        if unpainted {
            self.tally.unpainted += 1;
        }
        Ok(())
    }

    fn gather(
        &mut self,
        node: Node<'a, 't>,
        style: Inherited<'a>,
        depth: usize,
        gathered: &mut Gathered<'a, 't>,
    ) {
        let slot = gathered.spans.len();
        let start = gathered.letters.len();
        gathered.spans.push(Span {
            node,
            start,
            end: start,
        });
        for content in node.content() {
            match content {
                Content::Text(text) => gathered.push_text(text, style),
                Content::Element(child) if self.is_svg(child) => {
                    self.gather_child(child, style, depth, gathered);
                }
                Content::Element(_) => {}
            }
        }
        let end = gathered.letters.len();
        if let Some(span) = gathered.spans.get_mut(slot) {
            span.end = end;
        }
    }

    fn gather_child(
        &mut self,
        child: Node<'a, 't>,
        style: Inherited<'a>,
        depth: usize,
        gathered: &mut Gathered<'a, 't>,
    ) {
        let on_path = child.name() == "textPath";
        if !on_path && !matches!(child.name(), "tspan" | "a") {
            return;
        }
        if depth >= MAX_NESTING {
            self.tally.too_deep += 1;
            return;
        }
        let properties = self.properties(child);
        if properties.hidden {
            self.tally.hidden += 1;
            return;
        }
        let style = style.under(&properties);
        if !on_path || gathered.track.is_some() {
            self.gather(child, style, depth + 1, gathered);
            return;
        }
        let Some(track) = self.track(child, gathered.viewport) else {
            self.tally.missing += 1;
            return;
        };
        gathered.track = Some(gathered.tracks.len());
        gathered.tracks.push(track);
        self.gather(child, style, depth + 1, gathered);
        gathered.track = None;
    }

    fn track(&self, text_path: Node<'a, 't>, viewport: Vector2) -> Option<Track> {
        let side = match text_path.attribute("side").map(str::trim) {
            Some("right") => Side::Right,
            _ => Side::Left,
        };
        let mut track = match text_path.attribute("path") {
            Some(data) => Track::of(&path_outline(data), &Matrix::IDENTITY, side),
            None => {
                let target = text_path
                    .attribute("href")
                    .or_else(|| text_path.attribute_in(XLINK_NAMESPACE, "href"))
                    .and_then(|reference| reference.trim().strip_prefix('#'))
                    .and_then(|id| self.ids.get(id).copied())
                    .filter(|target| {
                        self.is_svg(*target) && TRACK_SHAPES.contains(&target.name())
                    })?;
                let matrix = target
                    .attribute("transform")
                    .and_then(transform_list)
                    .unwrap_or(Matrix::IDENTITY);
                Track::of(&outline_of(target, viewport), &matrix, side)
            }
        };
        let length = track.length();
        track.start = text_path
            .attribute("startOffset")
            .and_then(|offset| Length::parse(offset.trim()))
            .map(|offset| offset.pixels(length))
            .filter(|offset| offset.is_finite())
            .unwrap_or(0.0);
        Some(track)
    }

    fn note_style(&mut self, letters: &[Letter<'a>], italic_carried: bool) {
        let mut italic = false;
        for letter in letters {
            italic = italic || letter.style.text.italic;
            let Some(family) = letter.style.text.family.and_then(substituted) else {
                continue;
            };
            let known = self
                .tally
                .substituted_families
                .iter()
                .any(|known| known.eq_ignore_ascii_case(&family));
            if !known && self.tally.substituted_families.len() < MAX_NAMED_ELEMENTS {
                self.tally.substituted_families.insert(family);
            }
        }
        if italic && !italic_carried {
            self.tally.italic += 1;
        }
    }
}

fn lay_out(
    typeface: &mut Typeface<'_>,
    letters: &[Letter<'_>],
    positions: &Positions,
    tracks: &[Track],
) -> Vec<Placed> {
    let units_per_em = typeface.units_per_em();
    let mut placed: Vec<Placed> = Vec::with_capacity(letters.len());
    let mut starts: Vec<(usize, Run)> = Vec::new();
    let mut pen = Point2::ZERO;
    let mut previous: Option<(GlyphId, Instance, f64)> = None;
    for (index, letter) in letters.iter().enumerate() {
        let at = |list: &[Option<f64>]| list.get(index).copied().flatten();
        let before = index
            .checked_sub(1)
            .and_then(|before| letters.get(before))
            .map(|before| before.track);
        let entering = letter.track.is_some() && before != Some(letter.track);
        let leaving = letter.track.is_none() && before.flatten().is_some();
        let (x, y) = match letter.track {
            Some(_) => (None, None),
            None => (at(&positions.x), at(&positions.y)),
        };
        let absolute = x.is_some() || y.is_some();
        if entering || leaving {
            pen = Point2::ZERO;
            previous = None;
        }
        if index == 0 || entering || leaving || absolute {
            let run = match letter.track {
                Some(track) => Run::OnPath(track),
                None if leaving && !absolute => Run::AfterPath,
                None => Run::Straight,
            };
            starts.push((index, run));
        }
        let instance = typeface.instance(letter.style.text.weight, letter.style.text.italic);
        let glyph = typeface.glyph(letter.character, instance);
        let scale = letter.style.text.size / units_per_em;
        if let (Some((before, before_instance, before_scale)), Some(glyph), None) =
            (previous, glyph, x)
            && before_instance == instance
        {
            pen.x += typeface.kerning(instance, before, glyph) * before_scale;
        }
        if let Some(x) = x {
            pen.x = x;
        }
        if let Some(y) = y {
            pen.y = y;
        }
        pen += Vector2::new(
            at(&positions.dx).unwrap_or(0.0),
            at(&positions.dy).unwrap_or(0.0),
        );
        let advance = typeface.advance(instance, glyph) * scale;
        placed.push(Placed {
            at: pen,
            advance,
            glyph,
            instance,
            scale,
            stretch: 1.0,
            rotate: at(&positions.rotate).unwrap_or(0.0),
            rise: letter.style.text.rise,
            off_path: false,
        });
        let word_spacing = if letter.character == SPACE {
            letter.style.text.word_spacing
        } else {
            0.0
        };
        pen.x += advance + letter.style.text.letter_spacing + word_spacing;
        previous = glyph.map(|glyph| (glyph, instance, scale));
    }
    let ends = starts
        .iter()
        .skip(1)
        .map(|(start, _)| *start)
        .chain([letters.len()]);
    let chunks: Vec<Chunk> = starts
        .iter()
        .zip(ends)
        .map(|((start, run), end)| Chunk {
            start: *start,
            end,
            run: *run,
        })
        .collect();
    for fit in positions.fits.iter().rev() {
        let end_of_chunk = chunks
            .iter()
            .find(|chunk| (chunk.start..chunk.end).contains(&fit.start))
            .map_or(fit.end, |chunk| chunk.end.max(fit.end));
        fitted(&mut placed, fit, end_of_chunk);
    }
    let mut path_end: Option<Point2> = None;
    for chunk in &chunks {
        let anchor = letters
            .get(chunk.start)
            .map_or(Anchor::Start, |letter| letter.style.text.anchor);
        let Some(run) = placed.get_mut(chunk.start..chunk.end) else {
            continue;
        };
        anchored(run, anchor);
        match chunk.run {
            Run::OnPath(track) => {
                path_end = tracks.get(track).and_then(|track| along(run, track));
            }
            Run::AfterPath => {
                let shift = path_end.map_or(Vector2::ZERO, |end| end - Point2::ZERO);
                for letter in run.iter_mut() {
                    letter.at += shift;
                    letter.at.y -= letter.rise;
                }
            }
            Run::Straight => {
                for letter in run.iter_mut() {
                    letter.at.y -= letter.rise;
                }
            }
        }
    }
    placed
}

fn fitted(placed: &mut [Placed], fit: &Fit, end_of_chunk: usize) {
    let Some(run) = placed.get_mut(fit.start..fit.end) else {
        return;
    };
    let (low, high) = extent(run);
    let natural = high - low;
    if !natural.is_finite() {
        return;
    }
    let delta = fit.length - natural;
    if fit.glyphs {
        if natural <= 0.0 {
            return;
        }
        let factor = fit.length / natural;
        for letter in run.iter_mut() {
            letter.at.x = low + (letter.at.x - low) * factor;
            letter.advance *= factor;
            letter.stretch *= factor;
        }
    } else {
        let gaps = run.len().saturating_sub(1);
        if gaps == 0 {
            return;
        }
        let step = delta / gaps as f64;
        for (index, letter) in run.iter_mut().enumerate() {
            letter.at.x += step * index as f64;
        }
    }
    for letter in placed.get_mut(fit.end..end_of_chunk).unwrap_or_default() {
        letter.at.x += delta;
    }
}

fn along(run: &mut [Placed], track: &Track) -> Option<Point2> {
    let mut end = None;
    for letter in run {
        let middle = track.start + letter.at.x + letter.advance / 2.0;
        let Some((point, tangent)) = track.at(middle) else {
            letter.off_path = true;
            continue;
        };
        let normal = Vector2::new(-tangent.y, tangent.x);
        let offset = letter.at.y - letter.rise;
        letter.at = point - tangent * (letter.advance / 2.0) + normal * offset;
        letter.rotate += tangent.y.atan2(tangent.x).to_degrees();
        letter.rise = 0.0;
        end = Some(point + tangent * (letter.advance / 2.0));
    }
    end
}

fn extent(chunk: &[Placed]) -> (f64, f64) {
    chunk
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), placed| {
            let end = placed.at.x + placed.advance;
            (
                low.min(placed.at.x).min(end),
                high.max(placed.at.x).max(end),
            )
        })
}

fn anchored(chunk: &mut [Placed], anchor: Anchor) {
    let Some(first) = chunk.first().map(|placed| placed.at.x) else {
        return;
    };
    let (low, high) = extent(chunk);
    let shift = first
        - match anchor {
            Anchor::Start => low,
            Anchor::Middle => (low + high) / 2.0,
            Anchor::End => high,
        };
    for placed in chunk {
        placed.at.x += shift;
    }
}

fn substituted(families: &str) -> Option<String> {
    let first = families.split(',').next()?.trim();
    let unquoted = ['"', '\'']
        .into_iter()
        .find_map(|quote| first.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(first)
        .trim();
    let squashed: String = unquoted
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-')
        .flat_map(char::to_lowercase)
        .collect();
    let lowered = unquoted.to_lowercase();
    let is_inter = INTER_NAMES.contains(&squashed.as_str());
    let is_sans_serif = SANS_SERIF_NAMES.contains(&lowered.as_str());
    if unquoted.is_empty() || is_inter || is_sans_serif {
        return None;
    }
    Some(
        unquoted
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .take(MAX_FAMILY_CHARS)
            .collect(),
    )
}
