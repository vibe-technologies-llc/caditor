use std::ops::Range;

use crate::{
    gpu::{self, Bytes, GrowableBuffer, Records},
    lines::{LINE_POINT_BYTES, LINE_POINT_SLOTS, LINE_POINT_STRIDE},
    mesh::StyleLayout,
    spans::{SlabSizes, SlabSpan, Slabs},
    styles::{StyleTable, StyleTexel, style_texel_bytes},
};

const LINE_SLAB_BYTES: u64 = 16 << 20;
const FIRST_LINE_SLAB_BYTES: u64 = 256 << 10;
const FIRST_LINE_SLAB_STYLES: u64 = 256;
const LINE_SLAB_STYLES: u64 = 1 << 16;
const PACKED_SHARE: u64 = 4;
const POINTS: usize = 0;
const STYLES: usize = 1;

pub(crate) struct LineSlab {
    points: wgpu::Buffer,
    styles: StyleTable,
}

impl LineSlab {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        [points, styles]: [u64; 2],
    ) -> Self {
        let largest_side = device.limits().max_texture_dimension_2d;
        Self {
            points: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("line points"),
                size: points * LINE_POINT_STRIDE,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            styles: StyleTable::new(
                device,
                layout,
                "line styles",
                StyleLayout::new(usize::try_from(styles).unwrap_or(usize::MAX), largest_side),
            ),
        }
    }
}

pub(crate) type LineSlabs = Slabs<LineSlab, 2>;

pub(crate) fn line_slabs(device: &wgpu::Device) -> LineSlabs {
    let side = u64::from(device.limits().max_texture_dimension_2d.max(1));
    let most = gpu::buffer_limit(device).min(LINE_SLAB_BYTES);
    let most_styles = LINE_SLAB_STYLES.min(side.saturating_mul(side));
    Slabs::new(SlabSizes {
        first: [
            most.min(FIRST_LINE_SLAB_BYTES) / LINE_POINT_STRIDE,
            FIRST_LINE_SLAB_STYLES.min(most_styles),
        ],
        most: [most / LINE_POINT_STRIDE, most_styles],
    })
}

pub(crate) enum LinePoints {
    Own {
        points: GrowableBuffer,
        styles: StyleTable,
    },
    Packed(SlabSpan<LineSlab, 2>),
}

impl LinePoints {
    fn buffer(&self) -> &wgpu::Buffer {
        match self {
            Self::Own { points, .. } => points.buffer(),
            Self::Packed(span) => &span.contents().points,
        }
    }

    fn styles(&self) -> &wgpu::BindGroup {
        match self {
            Self::Own { styles, .. } => &styles.bind_group,
            Self::Packed(span) => &span.contents().styles.bind_group,
        }
    }

    fn first(&self) -> u32 {
        match self {
            Self::Own { .. } => 0,
            Self::Packed(span) => u32::try_from(span.start(POINTS)).unwrap_or(u32::MAX),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_packed(&self) -> bool {
        matches!(self, Self::Packed(_))
    }
}

pub(crate) struct LineGpu<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub slabs: &'a mut LineSlabs,
    pub styles_layout: &'a wgpu::BindGroupLayout,
    pub staging: &'a mut Bytes,
}

pub(crate) fn upload_lines<I>(
    kept: Option<LinePoints>,
    gpu: LineGpu<'_>,
    (count, styles): (u64, &[StyleTexel]),
    records: impl FnOnce(u32) -> I,
) -> (Option<LinePoints>, u64)
where
    I: Iterator<Item = [u8; LINE_POINT_BYTES]>,
{
    let LineGpu {
        device,
        queue,
        slabs,
        styles_layout,
        staging,
    } = gpu;

    if count == 0 {
        return (None, 0);
    }

    let lengths = [count, styles.len() as u64];
    let kept_own = match kept {
        Some(LinePoints::Own { points, styles }) => Some((points, styles)),
        Some(LinePoints::Packed(_)) | None => None,
    };
    let packed = match slabs.sizes().packs(lengths, PACKED_SHARE) {
        true => slabs.place(lengths, |capacities| {
            LineSlab::new(device, styles_layout, capacities)
        }),
        false => None,
    };

    if let Some(span) = packed {
        let style_base = u32::try_from(span.start(STYLES)).unwrap_or(u32::MAX);
        let slab = span.contents();
        let written = usize::try_from(count).unwrap_or(usize::MAX);

        gpu::write_records(
            queue,
            &slab.points,
            span.start(POINTS) * LINE_POINT_STRIDE,
            written,
            records(style_base),
        );
        slab.styles
            .write_run(queue, staging, span.start(STYLES), styles);
        return (Some(LinePoints::Packed(span)), count);
    }

    let (kept_points, kept_styles) = kept_own.unzip();
    let mut points = kept_points
        .unwrap_or_else(|| GrowableBuffer::new(device, "line points", wgpu::BufferUsages::VERTEX));

    let uploaded = points.upload(
        device,
        queue,
        Records {
            count,
            per_primitive: 1,
            records: records(0),
        },
    );
    let layout = StyleLayout::new(styles.len(), device.limits().max_texture_dimension_2d);
    let styles = StyleTable::written(
        kept_styles,
        (device, queue),
        styles_layout,
        "line styles",
        layout,
        &style_texel_bytes(styles.iter().copied(), layout),
    );
    (Some(LinePoints::Own { points, styles }), uploaded)
}

pub(crate) struct LineBinding<'a> {
    pipeline: &'a wgpu::RenderPipeline,
    set: bool,
    bound: Option<&'a wgpu::Buffer>,
}

impl<'a> LineBinding<'a> {
    pub(crate) fn of(pipeline: &'a wgpu::RenderPipeline) -> Self {
        Self {
            pipeline,
            set: false,
            bound: None,
        }
    }

    pub(crate) fn draw(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        points: &'a LinePoints,
        instances: Range<u32>,
    ) {
        let buffer = points.buffer();

        if instances.is_empty() || buffer.size() <= u64::from(LINE_POINT_SLOTS) * LINE_POINT_STRIDE
        {
            return;
        }
        if !self.set {
            pass.set_pipeline(self.pipeline);
            self.set = true;
        }
        if self.bound != Some(buffer) {
            pass.set_bind_group(1, points.styles(), &[]);
            for slot in 0..LINE_POINT_SLOTS {
                pass.set_vertex_buffer(slot, buffer.slice(u64::from(slot) * LINE_POINT_STRIDE..));
            }
            self.bound = Some(buffer);
        }

        let first = points.first();

        pass.draw_indexed(
            0..gpu::QUAD_INDEX_COUNT,
            0,
            first.saturating_add(instances.start)..first.saturating_add(instances.end),
        );
    }
}
