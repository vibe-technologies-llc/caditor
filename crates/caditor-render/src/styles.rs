use std::iter;

use crate::mesh::StyleLayout;

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

    fn new(
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
}
