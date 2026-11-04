use caditor_geometry::{Point2, Vector2};
use caditor_lettering::{GlyphId, Instance, Segment, Typeface};

use crate::import::{
    ImportError, MAX_READ_CURVES,
    svg::{
        Context, MAX_NAMED_ELEMENTS, MAX_NESTING, Walker, XLINK_NAMESPACE,
        path::{path_outline, traced},
        shapes::{Axis, outline_of},
        style::Inherited,
        syntax::{Length, Matrix, numbers, transform_list},
        text_path::{Method, Side, Spacing, Track},
        text_style::{Anchor, FontShift, TextStyle},
    },
    xml::{Content, Node},
};

const SPACE: char = ' ';
const FALLBACK_SUBSCRIPT_EMS: f64 = 0.2;
const FALLBACK_SUPERSCRIPT_EMS: f64 = 0.4;
const MAX_CHORD_STEPS: usize = 4096;
const CHORD_STEPS_PER_ADVANCE: f64 = 32.0;
const CHORD_BISECTIONS: usize = 60;
const PIECES_PER_ADVANCE: f64 = 6.0;
const MOST_PIECES: usize = 32;
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
    warped_along: Option<usize>,
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
            let segments = typeface.outline(placed.instance, glyph);
            let warping = placed.warped_along.and_then(|index| tracks.get(index));
            let (outline, matrix) = match warping {
                Some(track) => (traced(&warped(&segments, placed, track)), context.matrix),
                None => (
                    traced(&segments),
                    Matrix::scale(placed.scale * placed.stretch, -placed.scale)
                        .then(&Matrix::rotation(placed.rotate))
                        .then(&Matrix::translation(placed.at.x, placed.at.y))
                        .then(&context.matrix),
                ),
            };
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
        let method = match text_path.attribute("method").map(str::trim) {
            Some("stretch") => Method::Stretch,
            _ => Method::Align,
        };
        let spacing = match text_path.attribute("spacing").map(str::trim) {
            Some("auto") => Spacing::Auto,
            _ => Spacing::Exact,
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
        track.method = method;
        track.spacing = spacing;
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
            rise: font_rise(typeface, instance, &letter.style.text, units_per_em),
            off_path: false,
            warped_along: None,
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
            Run::OnPath(index) => {
                path_end = tracks.get(index).and_then(|track| along(run, track, index));
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

fn font_rise(
    typeface: &mut Typeface<'_>,
    instance: Instance,
    style: &TextStyle<'_>,
    units_per_em: f64,
) -> f64 {
    if style.font_shift == FontShift::default() {
        return style.rise;
    }
    let FontShift {
        subscripted,
        superscripted,
        of_line_height,
    } = style.font_shift;
    let in_ems = |units: Option<f64>, fallback: f64| units.map_or(fallback, |u| u / units_per_em);
    let dropped = in_ems(typeface.subscript_drop(instance), FALLBACK_SUBSCRIPT_EMS);
    let raised = in_ems(
        typeface.superscript_rise(instance),
        FALLBACK_SUPERSCRIPT_EMS,
    );
    let line = in_ems(typeface.line_height(instance), 1.0);
    style.rise + raised * superscripted - dropped * subscripted + line * of_line_height
}

fn along(run: &mut [Placed], track: &Track, index: usize) -> Option<Point2> {
    match (track.method, track.spacing) {
        (Method::Stretch, _) => stretched_along(run, track, index),
        (Method::Align, Spacing::Exact) => aligned_along(run, track),
        (Method::Align, Spacing::Auto) => chorded_along(run, track),
    }
}

fn aligned_along(run: &mut [Placed], track: &Track) -> Option<Point2> {
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

fn chord_end(track: &Track, start: f64, advance: f64) -> f64 {
    let Some((from, _)) = track.extended(start) else {
        return start + advance;
    };
    let reach = |distance: f64| {
        track
            .extended(distance)
            .map_or(0.0, |(point, _)| (point - from).length())
    };
    let step = advance / CHORD_STEPS_PER_ADVANCE;
    let mut near = start;
    for count in 1..=MAX_CHORD_STEPS {
        let far = start + step * count as f64;
        if reach(far) >= advance {
            let (mut low, mut high) = (near, far);
            for _ in 0..CHORD_BISECTIONS {
                let middle = 0.5 * (low + high);
                if reach(middle) >= advance {
                    high = middle;
                } else {
                    low = middle;
                }
            }
            return high;
        }
        near = far;
    }
    start + advance
}

fn chorded_along(run: &mut [Placed], track: &Track) -> Option<Point2> {
    let mut end = None;
    let mut shift = 0.0;
    for letter in run {
        let start = track.start + letter.at.x + shift;
        let finish = chord_end(track, start, letter.advance);
        shift += finish - start - letter.advance;
        let middle = 0.5 * (start + finish);
        let ends = track.extended(start).zip(track.extended(finish));
        let (Some(((from, _), (to, _))), true) = (ends, (0.0..=track.length()).contains(&middle))
        else {
            letter.off_path = true;
            continue;
        };
        let Some(direction) = (to - from).try_normalize() else {
            letter.off_path = true;
            continue;
        };
        let normal = Vector2::new(-direction.y, direction.x);
        let offset = letter.at.y - letter.rise;
        letter.at = from + normal * offset;
        letter.rotate += direction.y.atan2(direction.x).to_degrees();
        letter.rise = 0.0;
        end = Some(to);
    }
    end
}

fn stretched_along(run: &mut [Placed], track: &Track, index: usize) -> Option<Point2> {
    let mut end = None;
    for letter in run {
        let middle = track.start + letter.at.x + letter.advance / 2.0;
        if track.at(middle).is_none() {
            letter.off_path = true;
            continue;
        }
        letter.at.y -= letter.rise;
        letter.rise = 0.0;
        letter.warped_along = Some(index);
        end = track
            .extended(track.start + letter.at.x + letter.advance)
            .map(|(point, _)| point);
    }
    end
}

fn warped(segments: &[Segment], placed: &Placed, track: &Track) -> Vec<Segment> {
    let across = placed.scale * placed.stretch;
    let place = |point: Point2| {
        let distance = track.start + placed.at.x + point.x * across;
        track
            .extended(distance)
            .map_or(point, |(on_path, tangent)| {
                let normal = Vector2::new(-tangent.y, tangent.x);
                on_path + normal * (placed.at.y - point.y * placed.scale)
            })
    };
    let piece_length = (placed.advance / PIECES_PER_ADVANCE).max(f64::EPSILON);
    let mut warped = Vec::with_capacity(segments.len());
    for segment in segments {
        let line;
        let points: &[Point2] = match segment {
            Segment::Line(from, to) => {
                line = [*from, *to];
                &line
            }
            Segment::Bezier(points) => points,
        };
        let (low, high) = points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                (low.min(point.x), high.max(point.x))
            });
        let from = track.start + placed.at.x + low * across;
        let to = track.start + placed.at.x + high * across;
        let pieces = if track.turns_between(from, to) {
            (((to - from) / piece_length).ceil() as usize).clamp(1, MOST_PIECES)
        } else {
            1
        };
        match segment {
            Segment::Line(start, end) => {
                for step in 0..pieces {
                    let (first, last) = (
                        start.lerp(*end, step as f64 / pieces as f64),
                        start.lerp(*end, (step + 1) as f64 / pieces as f64),
                    );
                    warped.push(Segment::Line(place(first), place(last)));
                }
            }
            Segment::Bezier(points) => {
                let mut rest = points.clone();
                for step in 0..pieces {
                    let share = 1.0 / (pieces - step) as f64;
                    let (head, tail) = split_bezier(&rest, share);
                    warped.push(Segment::Bezier(head.into_iter().map(place).collect()));
                    rest = tail;
                }
            }
        }
    }
    warped
}

fn split_bezier(points: &[Point2], at: f64) -> (Vec<Point2>, Vec<Point2>) {
    let mut head = Vec::with_capacity(points.len());
    let mut tail = Vec::with_capacity(points.len());
    let mut level = points.to_vec();
    while let Some(first) = level.first().copied() {
        head.push(first);
        tail.push(level.last().copied().unwrap_or(first));
        level = level
            .windows(2)
            .map(|pair| match pair {
                [from, to] => from.lerp(*to, at),
                _ => Point2::ZERO,
            })
            .collect();
    }
    tail.reverse();
    (head, tail)
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

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use caditor_lettering::INTER;

    use super::*;
    use crate::import::svg::path::path_outline;

    fn circle(radius: f64) -> Track {
        let data = format!(
            "M {radius} 0 A {radius} {radius} 0 1 1 {} 0 A {radius} {radius} 0 1 1 {radius} 0",
            -radius
        );
        Track::of(&path_outline(&data), &Matrix::IDENTITY, Side::Left)
    }

    fn run(count: usize, advance: f64) -> Vec<Placed> {
        let typeface = Typeface::parse(INTER, None).unwrap();
        let instance = typeface.instance(400.0, false);
        (0..count)
            .map(|index| Placed {
                at: Point2::new(advance * index as f64, 0.0),
                advance,
                glyph: None,
                instance,
                scale: 1.0,
                stretch: 1.0,
                rotate: 0.0,
                rise: 0.0,
                off_path: false,
                warped_along: None,
            })
            .collect()
    }

    #[test]
    fn auto_spacing_puts_each_letter_s_start_a_chord_of_its_advance_from_the_last() {
        let track = circle(40.0);
        let mut automatic = run(5, 10.0);
        let mut exact = run(5, 10.0);

        chorded_along(&mut automatic, &track);
        aligned_along(&mut exact, &track);

        for pair in automatic.windows(2) {
            assert!(
                ((pair[1].at - pair[0].at).length() - 10.0).abs() < 1e-9,
                "{:?}",
                pair
            );
        }
        let angle = |at: Point2| at.y.atan2(at.x).rem_euclid(TAU);
        let covered = |letters: &[Placed]| angle(letters[4].at) - angle(letters[0].at);
        let predicted = 2.0 * (10.0_f64 / 80.0).asin() / (10.0 / 40.0);
        assert!(
            (covered(&automatic) / covered(&exact) - predicted).abs() < 0.01,
            "{} against {}",
            covered(&automatic),
            covered(&exact)
        );
        assert!(automatic.iter().all(|letter| !letter.off_path));
    }

    #[test]
    fn auto_spacing_on_a_straight_track_keeps_the_exact_places() {
        let track = Track::of(
            &path_outline("M 0 0 L 100 0"),
            &Matrix::IDENTITY,
            Side::Left,
        );
        let mut automatic = run(4, 10.0);
        let mut exact = run(4, 10.0);

        chorded_along(&mut automatic, &track);
        aligned_along(&mut exact, &track);

        for (automatic, exact) in automatic.iter().zip(&exact) {
            assert!((automatic.at - exact.at).length() < 1e-9);
            assert!((automatic.rotate - exact.rotate).abs() < 1e-9);
        }
    }

    #[test]
    fn a_chord_that_the_track_cannot_reach_leaves_the_letter_off_it() {
        let track = Track::of(&path_outline("M 0 0 L 24 0"), &Matrix::IDENTITY, Side::Left);
        let mut letters = run(4, 10.0);

        chorded_along(&mut letters, &track);

        assert_eq!(
            letters
                .iter()
                .map(|letter| letter.off_path)
                .collect::<Vec<_>>(),
            [false, false, true, true]
        );
    }
}
