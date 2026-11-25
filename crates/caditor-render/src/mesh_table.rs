use std::{num::NonZeroU64, ops::Range, sync::Arc};

use parking_lot::Mutex;

use crate::{
    gpu::{self, Bytes, Pack},
    mesh::{FaceStyle, StyleLayout, SurfaceFinish, pack_color},
    scene::{Color, PickId},
    spans::{Held, SharedSpans, Spans},
    styles::texel_rows,
};

pub(crate) const PLACEMENT_BYTES: u64 = 80;
pub(crate) const WINDOW_SLOTS: u32 = 192;
pub(crate) const WINDOW_BYTES: u64 = PLACEMENT_BYTES * WINDOW_SLOTS as u64;
const WINDOW_ALIGNMENT: u64 = 256;
const SMALLEST_UNIFORM_BINDING: u64 = 16 << 10;
const _: () = assert!(
    WINDOW_BYTES.is_multiple_of(WINDOW_ALIGNMENT) && WINDOW_BYTES <= SMALLEST_UNIFORM_BINDING
);
pub(crate) const STYLE_BINDING: u32 = 1;
pub(crate) const PLACEMENT_BINDING: u32 = 2;
const STYLE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
const FIRST_STYLE_TEXELS: u64 = 1024;
const GROWTH: u64 = 2;
const MAX_STYLE_SPANS: usize = 16;

pub(crate) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("mesh placements and face styles"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: STYLE_BINDING,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            window_layout_entry(PLACEMENT_BINDING),
        ],
    })
}

pub(crate) fn window_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(WINDOW_BYTES),
        },
        count: None,
    }
}

pub(crate) fn window_buffer(device: &wgpu::Device, label: &str, windows: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: windows * WINDOW_BYTES,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(crate) fn window_binding(buffer: &wgpu::Buffer) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset: 0,
        size: NonZeroU64::new(WINDOW_BYTES),
    })
}

pub(crate) fn most_windows(device: &wgpu::Device) -> u64 {
    (gpu::buffer_limit(device) / WINDOW_BYTES).max(1)
}

pub(crate) fn windows_wanted(slots: &Spans) -> u64 {
    let wanted = match slots.longest_free() {
        0 => slots.capacity().saturating_add(1),
        _ => slots.capacity(),
    };
    wanted.div_ceil(u64::from(WINDOW_SLOTS))
}

pub(crate) fn grown_count(current: u64, wanted: u64, most: u64) -> Option<u64> {
    let grown = match wanted > current {
        true => current.saturating_mul(GROWTH).max(wanted),
        false => current,
    };
    (wanted <= most).then_some(grown.min(most))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Capacity {
    pub windows: u64,
    pub texels: u64,
}

impl Capacity {
    pub(crate) const FIRST: Self = Self {
        windows: 1,
        texels: FIRST_STYLE_TEXELS,
    };

    pub(crate) fn slots(self) -> u64 {
        self.windows * u64::from(WINDOW_SLOTS)
    }

    pub(crate) fn holding(slots: u64, texels: u64) -> Self {
        Self {
            windows: slots.div_ceil(u64::from(WINDOW_SLOTS)).max(1),
            texels: texels.max(1),
        }
    }

    fn at_least(self, other: Self) -> Self {
        Self {
            windows: self.windows.max(other.windows),
            texels: self.texels.max(other.texels),
        }
    }

    fn grown(self, wanted: Self, most: Self) -> Option<Self> {
        Some(Self {
            windows: grown_count(self.windows, wanted.windows, most.windows)?,
            texels: grown_count(self.texels, wanted.texels, most.texels)?,
        })
    }
}

pub(crate) struct MeshTable {
    placements: wgpu::Buffer,
    styles: wgpu::Texture,
    style_layout: StyleLayout,
    pub bind_group: wgpu::BindGroup,
    slots: SharedSpans,
    texels: SharedSpans,
}

pub(crate) struct Holding {
    pub slot: Held,
    pub texels: Held,
}

impl MeshTable {
    pub(crate) fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        capacity: Capacity,
    ) -> Self {
        let (placements, styles, style_layout, bind_group) = resources(device, layout, capacity);
        Self {
            slots: Arc::new(Mutex::new(Spans::new(capacity.slots()))),
            texels: Arc::new(Mutex::new(Spans::new(style_layout.texels() as u64))),
            placements,
            styles,
            style_layout,
            bind_group,
        }
    }

    pub(crate) fn hold(&self, faces: u64) -> Option<Holding> {
        Some(Holding {
            slot: Held::take(&self.slots, 1)?,
            texels: Held::take(&self.texels, faces.max(1))?,
        })
    }

    fn capacity(&self) -> Capacity {
        Capacity {
            windows: self.slots.lock().capacity() / u64::from(WINDOW_SLOTS),
            texels: self.texels.lock().capacity(),
        }
    }

    pub(crate) fn most(device: &wgpu::Device) -> Capacity {
        let limits = device.limits();
        let side = u64::from(limits.max_texture_dimension_2d.max(1));
        Capacity {
            windows: most_windows(device),
            texels: side.saturating_mul(side),
        }
    }

    pub(crate) fn wanting(&self, faces: u64) -> Capacity {
        let capacity = self.capacity();
        let slots = self.slots.lock();
        let texels = self.texels.lock();
        let needed = |spans: &Spans, wanted: u64| match spans.longest_free() >= wanted {
            true => spans.capacity(),
            false => spans.capacity().saturating_add(wanted),
        };
        Capacity {
            windows: windows_wanted(&slots),
            texels: needed(&texels, faces.max(1)),
        }
        .at_least(capacity)
    }

    pub(crate) fn grown_resources(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        wanted: Capacity,
    ) -> Option<GrownTable> {
        let capacity = self.capacity().grown(wanted, Self::most(device))?;
        let (placements, styles, style_layout, bind_group) = resources(device, layout, capacity);
        Some(GrownTable {
            capacity,
            placements,
            styles,
            style_layout,
            bind_group,
        })
    }

    pub(crate) fn install(&mut self, grown: GrownTable) {
        self.slots.lock().grow(grown.capacity.slots());
        self.texels.lock().grow(grown.style_layout.texels() as u64);
        self.placements = grown.placements;
        self.styles = grown.styles;
        self.style_layout = grown.style_layout;
        self.bind_group = grown.bind_group;
    }

    pub(crate) fn write_placement(&self, queue: &wgpu::Queue, slot: &Held, bytes: &Bytes) {
        queue.write_buffer(
            &self.placements,
            slot.start() * PLACEMENT_BYTES,
            bytes.as_slice(),
        );
    }

    pub(crate) fn write_styles(
        &self,
        queue: &wgpu::Queue,
        bytes: &mut Bytes,
        texels: &Held,
        faces: &[FaceStyle],
        written: Option<&[FaceStyle]>,
    ) {
        let target = StyleTarget {
            texture: &self.styles,
            columns: self.style_layout.columns.max(1),
            base: texels.start(),
            faces,
        };
        let count = usize::try_from(texels.len()).unwrap_or(usize::MAX);

        let spans = written
            .filter(|written| written.len() == faces.len())
            .and_then(|written| {
                changed_spans(
                    written,
                    faces,
                    (target.base, u64::from(target.columns), count),
                )
            });

        match spans {
            Some(spans) => {
                for span in spans {
                    target.write(queue, bytes, span);
                }
            }
            None => target.write(queue, bytes, 0..count),
        }
    }
}

pub(crate) struct GrownTable {
    capacity: Capacity,
    placements: wgpu::Buffer,
    styles: wgpu::Texture,
    style_layout: StyleLayout,
    bind_group: wgpu::BindGroup,
}

fn resources(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    capacity: Capacity,
) -> (wgpu::Buffer, wgpu::Texture, StyleLayout, wgpu::BindGroup) {
    let style_layout = StyleLayout::new(
        usize::try_from(capacity.texels).unwrap_or(usize::MAX),
        device.limits().max_texture_dimension_2d,
    );
    let placements = window_buffer(device, "mesh placements", capacity.windows);
    let styles = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mesh face styles"),
        size: style_layout.extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: STYLE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let styles_view = styles.create_view(&wgpu::TextureViewDescriptor::default());

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mesh placements and face styles"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: STYLE_BINDING,
                resource: wgpu::BindingResource::TextureView(&styles_view),
            },
            wgpu::BindGroupEntry {
                binding: PLACEMENT_BINDING,
                resource: window_binding(&placements),
            },
        ],
    });
    (placements, styles, style_layout, bind_group)
}

pub(crate) fn window_offset(slot: u64) -> u32 {
    u32::try_from(slot / u64::from(WINDOW_SLOTS) * WINDOW_BYTES).unwrap_or(u32::MAX)
}

pub(crate) fn window_slot(slot: u64) -> u32 {
    u32::try_from(slot % u64::from(WINDOW_SLOTS)).unwrap_or(0)
}

const UNSTYLED_FACE: FaceStyle = FaceStyle {
    color: Color::from_rgb8(160, 164, 172),
    pick: None,
    finish: None,
};

fn changed_spans(
    written: &[FaceStyle],
    faces: &[FaceStyle],
    (base, columns, count): (u64, u64, usize),
) -> Option<Vec<Range<usize>>> {
    let columns = columns.max(1);
    let mut spans: Vec<(u64, Range<usize>)> = Vec::new();

    for (index, (old, new)) in written.iter().zip(faces).enumerate().take(count) {
        if old == new {
            continue;
        }

        let row = (base + index as u64) / columns;
        match spans.last_mut() {
            Some((last, span)) if *last == row => span.end = index + 1,
            _ => spans.push((row, index..index + 1)),
        }
    }
    (spans.len() <= MAX_STYLE_SPANS).then(|| spans.into_iter().map(|(_, span)| span).collect())
}

struct StyleTarget<'a> {
    texture: &'a wgpu::Texture,
    columns: u32,
    base: u64,
    faces: &'a [FaceStyle],
}

impl StyleTarget<'_> {
    fn write(&self, queue: &wgpu::Queue, bytes: &mut Bytes, faces: Range<usize>) {
        let start = self.base + faces.start as u64;

        for rect in texel_rows(start..start + faces.len() as u64, self.columns) {
            self.pack(
                bytes,
                faces.start + rect.texels.start..faces.start + rect.texels.end,
            );
            rect.write(queue, self.texture, bytes);
        }
    }

    fn pack(&self, bytes: &mut Bytes, faces: Range<usize>) {
        bytes.clear();
        for face in faces {
            let style = self.faces.get(face).copied().unwrap_or(UNSTYLED_FACE);

            bytes
                .u32(pack_color(style.color))
                .u32(PickId::raw(style.pick))
                .u32(SurfaceFinish::packed(style.finish))
                .u32(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_rows_whose_styles_changed_are_written_and_many_scattered_changes_write_everything() {
        let style = |red: u8| FaceStyle {
            color: Color::from_rgb8(red, 0, 0),
            pick: None,
            finish: None,
        };
        let before: Vec<FaceStyle> = (0..40).map(|_| style(0)).collect();
        let mut after = before.clone();

        for face in [3, 5, 6, 25] {
            after[face] = style(9);
        }

        let scattered: Vec<FaceStyle> = (0..40).map(|face| style(face as u8)).collect();

        let spans = changed_spans(&before, &after, (0, 10, 40));
        let shifted = changed_spans(&before, &after, (4, 10, 40));
        let unchanged = changed_spans(&after, &after, (0, 10, 40));
        let everywhere = changed_spans(&before, &scattered, (0, 1, 40));
        let cut_short = changed_spans(&before, &after, (0, 10, 4));

        assert_eq!(spans, Some(vec![3..7, 25..26]));
        assert_eq!(shifted, Some(vec![3..6, 6..7, 25..26]));
        assert_eq!(unchanged, Some(Vec::new()));
        assert_eq!(everywhere, None);
        assert_eq!(
            cut_short.map(|spans| spans.into_iter().flatten().collect::<Vec<_>>()),
            Some(vec![3])
        );
    }

    #[test]
    fn a_table_grows_at_least_twofold_to_what_is_wanted_and_never_past_the_device() {
        let most = Capacity {
            windows: 8,
            texels: 4096,
        };

        let doubled = Capacity::FIRST.grown(
            Capacity {
                windows: 2,
                texels: 1100,
            },
            most,
        );
        let wanted = Capacity::FIRST.grown(
            Capacity {
                windows: 5,
                texels: 1024,
            },
            most,
        );
        let refused = Capacity::FIRST.grown(
            Capacity {
                windows: 9,
                texels: 1,
            },
            most,
        );

        assert_eq!(
            doubled,
            Some(Capacity {
                windows: 2,
                texels: 2048,
            })
        );
        assert_eq!(
            wanted,
            Some(Capacity {
                windows: 5,
                texels: 1024,
            })
        );
        assert_eq!(refused, None);
        assert_eq!(window_offset(191), 0);
        assert_eq!(window_offset(192), 15360);
        assert_eq!(window_slot(193), 1);
        assert_eq!(Capacity::holding(193, 0).windows, 2);
    }

    #[test]
    fn the_shader_reads_as_many_placements_in_a_window_as_a_window_holds() {
        let declared = format!("const WINDOW_SLOTS: u32 = {WINDOW_SLOTS}u;");

        assert!(include_str!("viewport.wgsl").contains(&declared));
    }
}
