use std::{iter, ops::Range};

use crate::{
    gpu::{Bytes, Pack},
    mesh::StyleLayout,
};

pub(crate) const STYLE_TABLE_BINDING: u32 = 4;
pub(crate) const STYLE_TEXEL_BYTES: u32 = 16;

pub(crate) type StyleTexel = [u32; 4];

pub(crate) struct StyleTable {
    layout: StyleLayout,
    texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
}

pub(crate) fn style_texel_bytes(
    texels: impl Iterator<Item = StyleTexel>,
    layout: StyleLayout,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(layout.texels() * STYLE_TEXEL_BYTES as usize);
    let padded = texels.chain(iter::repeat([0; 4])).take(layout.texels());

    for texel in padded.flatten() {
        bytes.extend_from_slice(&texel.to_le_bytes());
    }
    bytes
}

impl StyleTable {
    pub(crate) fn layout_entry() -> wgpu::BindGroupLayoutEntry {
        wgpu::BindGroupLayoutEntry {
            binding: STYLE_TABLE_BINDING,
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
        label: &str,
        layout: StyleLayout,
        bytes: &[u8],
    ) -> Self {
        let table = kept
            .filter(|kept| kept.layout == layout)
            .unwrap_or_else(|| Self::new(device, bind_group_layout, label, layout));

        queue.write_texture(
            table.texture.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(layout.columns.saturating_mul(STYLE_TEXEL_BYTES)),
                rows_per_image: Some(layout.rows),
            },
            layout.extent(),
        );
        table
    }

    pub(crate) fn new(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
        label: &str,
        layout: StyleLayout,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
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
            label: Some(label),
            layout: bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: STYLE_TABLE_BINDING,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Self {
            layout,
            texture,
            bind_group,
        }
    }

    pub(crate) fn write_run(
        &self,
        queue: &wgpu::Queue,
        bytes: &mut Bytes,
        start: u64,
        texels: &[StyleTexel],
    ) {
        for rect in texel_rows(start..start + texels.len() as u64, self.layout.columns) {
            bytes.clear();
            for word in texels
                .get(rect.texels.clone())
                .unwrap_or_default()
                .iter()
                .flatten()
            {
                bytes.u32(*word);
            }
            rect.write(queue, &self.texture, bytes);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TexelRect {
    pub texels: Range<usize>,
    column: u32,
    row: u32,
    width: u32,
    rows: u32,
}

impl TexelRect {
    pub(crate) fn write(&self, queue: &wgpu::Queue, texture: &wgpu::Texture, bytes: &Bytes) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: self.column,
                    y: self.row,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytes.as_slice(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width.saturating_mul(STYLE_TEXEL_BYTES)),
                rows_per_image: Some(self.rows),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.rows,
                depth_or_array_layers: 1,
            },
        );
    }
}

pub(crate) fn texel_rows(texels: Range<u64>, columns: u32) -> impl Iterator<Item = TexelRect> {
    let columns = u64::from(columns.max(1));
    let narrow = |value: u64| u32::try_from(value).unwrap_or(u32::MAX);
    let mut next = texels.start;

    iter::from_fn(move || {
        if next >= texels.end {
            return None;
        }

        let (row, column) = (next / columns, next % columns);
        let left = texels.end - next;

        let rows = match column {
            0 => (left / columns).max(1),
            _ => 1,
        };
        let width = match rows {
            1 => (columns - column).min(left),
            _ => columns,
        };
        let first = next - texels.start;

        let rect = TexelRect {
            texels: usize::try_from(first).unwrap_or(usize::MAX)
                ..usize::try_from(first + width * rows).unwrap_or(usize::MAX),
            column: narrow(column),
            row: narrow(row),
            width: narrow(width),
            rows: narrow(rows),
        };

        next += width * rows;
        Some(rect)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_texels_is_written_as_its_partial_first_row_whole_rows_and_partial_last_row() {
        let rects: Vec<(Range<usize>, [u32; 4])> = texel_rows(6..31, 8)
            .map(|rect| (rect.texels, [rect.column, rect.row, rect.width, rect.rows]))
            .collect();
        let aligned: Vec<Range<usize>> = texel_rows(8..24, 8).map(|rect| rect.texels).collect();
        let short: Vec<[u32; 4]> = texel_rows(3..5, 8)
            .map(|rect| [rect.column, rect.row, rect.width, rect.rows])
            .collect();

        assert_eq!(
            rects,
            vec![
                (0..2, [6, 0, 2, 1]),
                (2..18, [0, 1, 8, 2]),
                (18..25, [0, 3, 7, 1]),
            ]
        );
        assert_eq!(aligned, vec![0..16]);
        assert_eq!(short, vec![[3, 0, 2, 1]]);
        assert_eq!(texel_rows(4..4, 8).count(), 0);
    }
}
