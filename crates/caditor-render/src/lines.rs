use std::{collections::hash_map::Entry, iter, ops::Range};

use ahash::AHashMap;
use caditor_geometry::Point3;

use crate::{
    gpu::{self, Pack},
    mesh::{StyleLayout, pack_color},
    scene::{Line, PickId, Stroke},
    styles::{StyleTexel, style_texel_bytes},
    viewport::relative_to_eye,
};

pub(crate) const LINE_POINT_BYTES: usize = 24;
pub(crate) const LINE_POINT_STRIDE: u64 = LINE_POINT_BYTES as u64;
pub(crate) const LINE_POINT_SLOTS: u32 = 4;
const DRAWN: u32 = 1 << 31;
const LINKED: u32 = 1 << 30;
const STYLE_INDEX_LIMIT: usize = LINKED as usize;
const PADDING_POINTS: u64 = 2;
const CLOSED_STRIP_EXTRA_POINTS: u64 = 3;
const OPEN_STRIP_EXTRA_POINTS: u64 = 1;
const MIN_CLOSED_SEGMENTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum StrokeKind {
    Solid,
    Dashed,
    DashedWhereHidden { seen_dashed: bool },
}

impl StrokeKind {
    fn of(stroke: Stroke) -> Self {
        match stroke {
            Stroke::Solid => Self::Solid,
            Stroke::Dashed { .. } => Self::Dashed,
            Stroke::DashedWhereHidden { seen_dashed, .. } => {
                Self::DashedWhereHidden { seen_dashed }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LineStyle {
    color: u32,
    width: u32,
    flags: u32,
    kind: StrokeKind,
}

impl LineStyle {
    fn of(line: &Line) -> Self {
        Self {
            color: pack_color(line.color),
            width: line.width.to_bits(),
            flags: line.layer.flags() | line.stroke.flags(),
            kind: StrokeKind::of(line.stroke),
        }
    }

    fn texel(self) -> StyleTexel {
        [self.color, self.width, self.flags, 0]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Strip {
    lines: Range<usize>,
    closed: bool,
}

impl Strip {
    fn points(&self) -> u64 {
        let segments = self.lines.len() as u64;
        segments
            + if self.closed {
                CLOSED_STRIP_EXTRA_POINTS
            } else {
                OPEN_STRIP_EXTRA_POINTS
            }
    }

    fn drawn_from(&self) -> u64 {
        u64::from(self.closed)
    }
}

pub(crate) struct LineStrips<'a> {
    lines: Vec<&'a Line>,
    styles: Vec<LineStyle>,
    style_of: Vec<u32>,
    strips: Vec<Strip>,
    shown_strips: usize,
}

impl<'a> LineStrips<'a> {
    pub(crate) fn of(lines: &'a [Line], largest_side: u32) -> Self {
        let side = u64::from(largest_side.max(1));
        let capacity = usize::try_from(side.saturating_mul(side))
            .unwrap_or(usize::MAX)
            .min(STYLE_INDEX_LIMIT);
        let shown = |line: &&Line| line.color.alpha > 0.0;
        let ordered = lines
            .iter()
            .filter(shown)
            .chain(lines.iter().filter(|line| !shown(line)));

        let mut kept = Vec::with_capacity(lines.len());
        let mut styles = Vec::new();
        let mut style_of = Vec::with_capacity(lines.len());
        let mut indices: AHashMap<LineStyle, u32> = AHashMap::new();
        for line in ordered {
            let style = LineStyle::of(line);
            let next = styles.len();
            let index = match indices.entry(style) {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(_) if next >= capacity => break,
                Entry::Vacant(entry) => {
                    styles.push(style);
                    *entry.insert(u32::try_from(next).unwrap_or(u32::MAX))
                }
            };
            kept.push(line);
            style_of.push(index);
        }
        let shown_lines = kept.iter().filter(|line| shown(line)).count();
        let strips = strips(&kept, &style_of, shown_lines);
        let shown_strips = strips
            .iter()
            .take_while(|strip| strip.lines.start < shown_lines)
            .count();
        Self {
            lines: kept,
            styles,
            style_of,
            strips,
            shown_strips,
        }
    }

    pub(crate) fn point_count(&self) -> u64 {
        PADDING_POINTS + self.strips.iter().map(Strip::points).sum::<u64>()
    }

    pub(crate) fn shown_instances(&self) -> u64 {
        self.strips
            .iter()
            .take(self.shown_strips)
            .map(Strip::points)
            .sum()
    }

    #[cfg(test)]
    pub(crate) fn segments(&self) -> (u64, u64) {
        let shown = self.lines.iter().filter(|line| line.color.alpha > 0.0);
        (shown.count() as u64, self.lines.len() as u64)
    }

    pub(crate) fn hidden_runs(&self) -> Vec<Range<u32>> {
        let mut runs: Vec<Range<u32>> = Vec::new();
        let mut extendable = false;
        let mut first_point = 1_u64;
        for strip in self.strips.iter().take(self.shown_strips) {
            let first_instance = first_point - 1 + strip.drawn_from();
            let drawn = first_instance..first_instance + strip.lines.len() as u64;
            first_point += strip.points();
            let flagged = self
                .lines
                .get(strip.lines.start)
                .is_some_and(|line| line.stroke.dashes_where_hidden());
            if !flagged {
                extendable = false;
                continue;
            }
            let drawn = instance(drawn.start)..instance(drawn.end);
            match runs.last_mut() {
                Some(run) if extendable => run.end = drawn.end,
                _ => runs.push(drawn),
            }
            extendable = true;
        }
        runs
    }

    pub(crate) fn points(
        &self,
        anchor: Point3,
    ) -> impl Iterator<Item = [u8; LINE_POINT_BYTES]> + '_ {
        let padding = || [0; LINE_POINT_BYTES];
        iter::once_with(padding)
            .chain(
                self.strips
                    .iter()
                    .flat_map(move |strip| self.strip_points(strip, anchor)),
            )
            .chain(iter::once_with(padding))
    }

    fn strip_points(
        &self,
        strip: &Strip,
        anchor: Point3,
    ) -> impl Iterator<Item = [u8; LINE_POINT_BYTES]> + '_ {
        let at = |index: usize| {
            self.lines
                .get(index)
                .zip(self.style_of.get(index))
                .map(|(line, style)| (*line, *style))
        };
        let first = at(strip.lines.start);
        let last = strip.lines.end.checked_sub(1).and_then(at);
        let point = move |(line, style): (&Line, u32), position: Point3, bits: u32| {
            point_record(line, position, anchor, style | bits)
        };
        let leading = last
            .filter(|_| strip.closed)
            .map(|last| point(last, last.0.start, LINKED));
        let starts = strip
            .lines
            .clone()
            .filter_map(at)
            .map(move |line| point(line, line.0.start, DRAWN | LINKED));
        let closing = last
            .filter(|_| strip.closed)
            .map(|last| point(last, last.0.end, LINKED));
        let trailing = if strip.closed {
            first.map(|first| point(first, first.0.end, 0))
        } else {
            last.map(|last| point(last, last.0.end, 0))
        };
        leading
            .into_iter()
            .chain(starts)
            .chain(closing)
            .chain(trailing)
    }

    pub(crate) fn shown_segments(&self) -> usize {
        self.strips
            .iter()
            .take(self.shown_strips)
            .map(|strip| strip.lines.len())
            .sum()
    }

    pub(crate) fn simplified(&self, tolerance: f64) -> Vec<Line> {
        let mut simplified = Vec::new();
        for strip in self.strips.iter().take(self.shown_strips) {
            let Some(lines) = self.lines.get(strip.lines.clone()) else {
                continue;
            };
            if is_within(lines, tolerance) {
                continue;
            }
            let kept = kept_corners(lines, tolerance);
            for pair in kept.windows(2) {
                let &[from, to] = pair else {
                    continue;
                };
                let (Some(first), Some(end)) = (lines.get(from), corner(lines, to)) else {
                    continue;
                };
                simplified.push(Line {
                    end,
                    ..Line::clone(first)
                });
            }
        }
        simplified
    }

    pub(crate) fn style_layout(&self, largest_side: u32) -> StyleLayout {
        StyleLayout::new(self.styles.len(), largest_side)
    }

    pub(crate) fn style_texels(&self, layout: StyleLayout) -> Vec<u8> {
        style_texel_bytes(self.styles.iter().map(|style| style.texel()), layout)
    }
}

pub(crate) fn line_instances(uploaded_points: u64) -> u32 {
    instance(uploaded_points.saturating_sub(u64::from(LINE_POINT_SLOTS) - 1))
}

fn instance(index: u64) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

fn point_record(
    line: &Line,
    position: Point3,
    anchor: Point3,
    word: u32,
) -> [u8; LINE_POINT_BYTES] {
    gpu::record(|record| {
        record
            .vec3(relative_to_eye(position, anchor))
            .f32(line.stroke.along())
            .u32(PickId::raw(line.pick))
            .u32(word);
    })
}

fn corner(lines: &[&Line], index: usize) -> Option<Point3> {
    match lines.get(index) {
        Some(line) => Some(line.start),
        None if index == lines.len() => lines.last().map(|line| line.end),
        None => None,
    }
}

fn is_within(lines: &[&Line], tolerance: f64) -> bool {
    let Some(first) = lines.first() else {
        return true;
    };
    lines.iter().all(|line| {
        first.start.distance(line.start) <= tolerance && first.start.distance(line.end) <= tolerance
    })
}

fn kept_corners(lines: &[&Line], tolerance: f64) -> Vec<usize> {
    let count = lines.len();
    let mut kept = vec![false; count + 1];
    for (index, pair) in lines.windows(2).enumerate() {
        if let ([before, after], Some(keep)) = (pair, kept.get_mut(index + 1)) {
            *keep = before.pick != after.pick;
        }
    }
    for end in [0, count] {
        if let Some(keep) = kept.get_mut(end) {
            *keep = true;
        }
    }
    let anchors: Vec<usize> = (0..=count)
        .filter(|index| kept.get(*index).copied().unwrap_or(false))
        .collect();
    let mut pending: Vec<(usize, usize)> = anchors
        .windows(2)
        .filter_map(|pair| match *pair {
            [from, to] => Some((from, to)),
            _ => None,
        })
        .collect();
    while let Some((from, to)) = pending.pop() {
        let (Some(start), Some(end)) = (corner(lines, from), corner(lines, to)) else {
            continue;
        };
        let farthest = (from + 1..to)
            .filter_map(|index| {
                let point = corner(lines, index)?;
                Some((index, distance_to_segment(point, start, end)))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, distance)) = farthest
            && distance > tolerance
            && let Some(keep) = kept.get_mut(index)
        {
            *keep = true;
            pending.push((from, index));
            pending.push((index, to));
        }
    }
    (0..=count)
        .filter(|index| kept.get(*index).copied().unwrap_or(false))
        .collect()
}

fn distance_to_segment(point: Point3, start: Point3, end: Point3) -> f64 {
    let along = end - start;
    let length_squared = along.length_squared();
    let share = if length_squared > 0.0 {
        ((point - start).dot(along) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.distance(start + along * share)
}

fn joins(before: (&Line, u32), after: (&Line, u32)) -> bool {
    before.1 == after.1
        && before.0.end == after.0.start
        && before.0.start != before.0.end
        && after.0.start != after.0.end
}

fn strips(lines: &[&Line], style_of: &[u32], shown_lines: usize) -> Vec<Strip> {
    let at = |index: usize| lines.get(index).copied().zip(style_of.get(index).copied());
    let mut strips: Vec<Strip> = Vec::new();
    for index in 0..lines.len() {
        let continues = index != shown_lines
            && strips.last().is_some_and(|strip| strip.lines.end == index)
            && index
                .checked_sub(1)
                .and_then(at)
                .zip(at(index))
                .is_some_and(|(before, after)| joins(before, after));
        match strips.last_mut() {
            Some(strip) if continues => strip.lines.end = index + 1,
            _ => strips.push(Strip {
                lines: index..index + 1,
                closed: false,
            }),
        }
    }
    for strip in &mut strips {
        let ends = strip
            .lines
            .end
            .checked_sub(1)
            .and_then(at)
            .zip(at(strip.lines.start));
        strip.closed = strip.lines.len() >= MIN_CLOSED_SEGMENTS
            && ends.is_some_and(|(last, first)| joins(last, first));
    }
    strips
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        scene::{Color, Layer},
        styles::STYLE_TEXEL_BYTES,
    };

    fn line(start: [f64; 2], end: [f64; 2], alpha: u8, stroke: Stroke) -> Line {
        Line {
            start: Point3::new(start[0], start[1], 0.0),
            end: Point3::new(end[0], end[1], 0.0),
            color: Color::from_rgba8(200, 10, 10, alpha),
            width: 2.0,
            layer: Layer::Model,
            pick: None,
            stroke,
        }
    }

    fn solid(start: [f64; 2], end: [f64; 2]) -> Line {
        line(start, end, 255, Stroke::Solid)
    }

    fn words(strips: &LineStrips<'_>) -> Vec<(f32, f32, u32)> {
        strips
            .points(Point3::ZERO)
            .map(|record| {
                let float = |at: usize| f32::from_le_bytes(record[at..at + 4].try_into().unwrap());
                let word = u32::from_le_bytes(record[20..24].try_into().unwrap());
                (float(0), float(4), word & (DRAWN | LINKED))
            })
            .collect()
    }

    #[test]
    fn a_polyline_shares_its_inner_points_and_a_lone_segment_costs_what_it_did() {
        let lines = [
            solid([0.0, 0.0], [1.0, 0.0]),
            solid([1.0, 0.0], [1.0, 1.0]),
            solid([1.0, 1.0], [2.0, 1.0]),
            solid([5.0, 5.0], [6.0, 5.0]),
        ];

        let strips = LineStrips::of(&lines, 64);

        assert_eq!(strips.strips.len(), 2);
        assert_eq!(strips.point_count(), 2 + 4 + 2);
        assert_eq!(line_instances(strips.point_count()), 5);
        assert_eq!(strips.styles.len(), 1);
        assert_eq!(
            words(&strips),
            vec![
                (0.0, 0.0, 0),
                (0.0, 0.0, DRAWN | LINKED),
                (1.0, 0.0, DRAWN | LINKED),
                (1.0, 1.0, DRAWN | LINKED),
                (2.0, 1.0, 0),
                (5.0, 5.0, DRAWN | LINKED),
                (6.0, 5.0, 0),
                (0.0, 0.0, 0),
            ]
        );
    }

    #[test]
    fn a_closed_loop_carries_its_closing_neighbours_on_both_sides() {
        let lines = [
            solid([0.0, 0.0], [1.0, 0.0]),
            solid([1.0, 0.0], [1.0, 1.0]),
            solid([1.0, 1.0], [0.0, 0.0]),
        ];

        let strips = LineStrips::of(&lines, 64);

        assert_eq!(
            strips.strips,
            vec![Strip {
                lines: 0..3,
                closed: true
            }]
        );
        assert_eq!(
            words(&strips),
            vec![
                (0.0, 0.0, 0),
                (1.0, 1.0, LINKED),
                (0.0, 0.0, DRAWN | LINKED),
                (1.0, 0.0, DRAWN | LINKED),
                (1.0, 1.0, DRAWN | LINKED),
                (0.0, 0.0, LINKED),
                (1.0, 0.0, 0),
                (0.0, 0.0, 0),
            ]
        );
    }

    #[test]
    fn only_touching_segments_of_one_style_and_stroke_join() {
        let dashed = Stroke::Dashed { along: 0.0 };
        let wider = Line {
            width: 3.0,
            ..solid([2.0, 0.0], [3.0, 0.0])
        };
        let lines = [
            solid([0.0, 0.0], [1.0, 0.0]),
            solid([1.0, 0.0], [2.0, 0.0]),
            wider,
            line([3.0, 0.0], [4.0, 0.0], 255, dashed),
            solid([4.0, 0.0], [5.0, 0.0]),
            solid([5.5, 0.0], [6.0, 0.0]),
            solid([6.0, 0.0], [6.0, 0.0]),
            solid([6.0, 0.0], [7.0, 0.0]),
        ];

        let strips = LineStrips::of(&lines, 64);

        assert_eq!(
            strips
                .strips
                .iter()
                .map(|strip| strip.lines.clone())
                .collect::<Vec<_>>(),
            vec![0..2, 2..3, 3..4, 4..5, 5..6, 6..7, 7..8]
        );
    }

    #[test]
    fn invisible_segments_follow_the_shown_ones_and_never_join_them() {
        let lines = [
            line([0.0, 0.0], [1.0, 0.0], 0, Stroke::Solid),
            solid([1.0, 0.0], [2.0, 0.0]),
            solid([2.0, 0.0], [3.0, 0.0]),
        ];

        let strips = LineStrips::of(&lines, 64);

        assert_eq!(strips.segments(), (2, 3));
        assert_eq!(strips.shown_instances(), 3);
        assert_eq!(
            strips
                .strips
                .iter()
                .map(|strip| strip.lines.clone())
                .collect::<Vec<_>>(),
            vec![0..2, 2..3]
        );
    }

    #[test]
    fn hidden_runs_cover_the_shown_strips_dashed_where_hidden_across_their_undrawn_ends() {
        let hidden = Stroke::DashedWhereHidden {
            along: 0.0,
            seen_dashed: false,
        };
        let lines = [
            line([0.0, 0.0], [0.0, 1.0], 255, hidden),
            line([0.0, 1.0], [0.0, 2.0], 255, hidden),
            line([1.0, 0.0], [1.0, 1.0], 0, hidden),
            line([2.0, 0.0], [2.0, 1.0], 255, hidden),
            solid([3.0, 0.0], [3.0, 1.0]),
            line([4.0, 0.0], [4.0, 1.0], 255, hidden),
            line([5.0, 0.0], [5.0, 1.0], 255, Stroke::Dashed { along: 0.0 }),
        ];

        let strips = LineStrips::of(&lines, 64);

        assert_eq!(strips.hidden_runs(), vec![0..4, 7..8]);
        assert!(LineStrips::of(&lines[4..5], 64).hidden_runs().is_empty());
    }

    #[test]
    fn simplified_strips_stay_within_the_tolerance_keep_where_the_pick_changes_and_drop_specks() {
        let segments = 256;
        let corner = |index: usize| {
            let angle = std::f64::consts::TAU * (index % segments) as f64 / segments as f64;
            [10.0 * angle.cos(), 10.0 * angle.sin()]
        };
        let lines: Vec<Line> = (0..segments)
            .map(|index| Line {
                pick: PickId::from_index(index / 128),
                ..solid(corner(index), corner(index + 1))
            })
            .collect();
        let speck = [
            solid([20.0, 20.0], [20.03, 20.0]),
            solid([20.03, 20.0], [20.03, 20.03]),
        ];
        let with_speck: Vec<Line> = lines.iter().chain(&speck).cloned().collect();
        let strips = LineStrips::of(&with_speck, 64);
        let tolerance = 0.05;

        let simplified = strips.simplified(tolerance);
        let kept: Vec<Point3> = simplified.iter().map(|line| line.start).collect();
        let distance_to_kept = |point: Point3| {
            simplified
                .iter()
                .map(|line| distance_to_segment(point, line.start, line.end))
                .fold(f64::INFINITY, f64::min)
        };

        assert_eq!(strips.shown_segments(), segments + speck.len());
        assert!(simplified.iter().all(|line| line.start.x < 15.0));
        assert!(simplified.len() < segments / 4, "{} kept", simplified.len());
        assert!(simplified.len() > 8);
        assert!(
            lines
                .iter()
                .all(|line| distance_to_kept(line.start) <= tolerance)
        );
        assert!(kept.contains(&lines[128].start));
        assert!(simplified.iter().all(|line| {
            line.pick
                == lines
                    .iter()
                    .find(|original| original.start == line.start)
                    .unwrap()
                    .pick
        }));
        assert_eq!(simplified.last().unwrap().end, lines[0].start);
        assert_eq!(
            LineStrips::of(&simplified, 64).strips,
            vec![Strip {
                lines: 0..simplified.len(),
                closed: true
            }]
        );
    }

    #[test]
    fn styles_past_what_the_texture_holds_are_left_out_with_their_lines() {
        let lines: Vec<Line> = (0..6)
            .map(|index| Line {
                width: 1.0 + index as f32,
                ..solid([index as f64, 0.0], [index as f64, 1.0])
            })
            .collect();

        let strips = LineStrips::of(&lines, 2);

        assert_eq!(strips.styles.len(), 4);
        assert_eq!(strips.segments(), (4, 4));
        assert_eq!(strips.style_layout(2), StyleLayout::new(4, 2));
        assert_eq!(
            strips.style_texels(strips.style_layout(2)).len(),
            4 * STYLE_TEXEL_BYTES as usize
        );
    }
}
