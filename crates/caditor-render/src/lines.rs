use std::{collections::hash_map::Entry, iter, ops::Range};

use ahash::AHashMap;
use caditor_geometry::Point3;

use crate::{
    gpu::{self, Pack},
    mesh::{StyleLayout, pack_color},
    scene::{Line, PickId, Stroke},
    viewport::relative_to_eye,
};

pub(crate) const LINE_POINT_BYTES: usize = 24;
pub(crate) const LINE_POINT_STRIDE: u64 = LINE_POINT_BYTES as u64;
pub(crate) const LINE_POINT_SLOTS: u32 = 4;
pub(crate) const LINE_STYLE_BINDING: u32 = 4;
const LINE_STYLE_TEXEL_BYTES: u32 = 16;
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

    fn texel(self) -> [u32; 4] {
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

    pub(crate) fn style_layout(&self, largest_side: u32) -> StyleLayout {
        StyleLayout::new(self.styles.len(), largest_side)
    }

    pub(crate) fn style_texels(&self, layout: StyleLayout) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(layout.texels() * LINE_STYLE_TEXEL_BYTES as usize);
        let texels = self
            .styles
            .iter()
            .map(|style| style.texel())
            .chain(iter::repeat([0; 4]))
            .take(layout.texels());
        for texel in texels.flatten() {
            bytes.extend_from_slice(&texel.to_le_bytes());
        }
        bytes
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

pub(crate) struct LineStyles {
    layout: StyleLayout,
    texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
}

impl LineStyles {
    pub(crate) fn layout_entry() -> wgpu::BindGroupLayoutEntry {
        wgpu::BindGroupLayoutEntry {
            binding: LINE_STYLE_BINDING,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }
    }

    pub(crate) fn written(
        kept: Option<Self>,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        bind_group_layout: &wgpu::BindGroupLayout,
        strips: &LineStrips<'_>,
    ) -> Self {
        let layout = strips.style_layout(device.limits().max_texture_dimension_2d);
        let styles = kept
            .filter(|kept| kept.layout == layout)
            .unwrap_or_else(|| Self::new(device, bind_group_layout, layout));
        queue.write_texture(
            styles.texture.as_image_copy(),
            &strips.style_texels(layout),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(layout.columns.saturating_mul(LINE_STYLE_TEXEL_BYTES)),
                rows_per_image: Some(layout.rows),
            },
            layout.extent(),
        );
        styles
    }

    fn new(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
        layout: StyleLayout,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("line styles"),
            size: layout.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("line styles"),
            layout: bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: LINE_STYLE_BINDING,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Self {
            layout,
            texture,
            bind_group,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Color, Layer};

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
            4 * LINE_STYLE_TEXEL_BYTES as usize
        );
    }
}
