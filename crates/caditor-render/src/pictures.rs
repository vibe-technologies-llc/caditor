use std::sync::Arc;

use caditor_geometry::Point3;

use crate::{gpu, scene::PickId, viewport::relative_to_eye};

pub const MAX_PICTURE_TEXTURE_SIDE: u32 = 4096;
const RGBA: usize = 4;
const UNIFORM_SIZE: u64 = 96;
const CORNERS: usize = 4;
const PICTURE_VERTICES: u32 = 6;

#[derive(PartialEq, Eq)]
pub struct PictureTexture {
    width: u32,
    height: u32,
    levels: Vec<Vec<u8>>,
}

impl std::fmt::Debug for PictureTexture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PictureTexture")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("levels", &self.levels.len())
            .finish()
    }
}

impl PictureTexture {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let expected = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(RGBA)?;
        if width == 0 || height == 0 || rgba.len() != expected {
            return None;
        }
        let mut level = Level {
            width,
            height,
            rgba,
        };
        while level.width > MAX_PICTURE_TEXTURE_SIDE || level.height > MAX_PICTURE_TEXTURE_SIDE {
            level = level.halved();
        }
        let (width, height) = (level.width, level.height);
        let mut levels = Vec::new();
        loop {
            let next = (level.width > 1 || level.height > 1).then(|| level.halved());
            levels.push(level.rgba);
            match next {
                Some(halved) => level = halved,
                None => break,
            }
        }
        Some(Self {
            width,
            height,
            levels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn held_bytes(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    fn level_size(&self, level: usize) -> (u32, u32) {
        let shift = u32::try_from(level).unwrap_or(u32::MAX).min(31);
        ((self.width >> shift).max(1), (self.height >> shift).max(1))
    }

    pub fn texel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let first = self.levels.first()?;
        let at = (usize::try_from(y).ok()? * usize::try_from(self.width).ok()?
            + usize::try_from(x).ok()?)
            * RGBA;
        let texel = first.get(at..at + RGBA)?;
        texel.try_into().ok()
    }
}

struct Level {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Level {
    fn halved(&self) -> Self {
        let width = (self.width / 2).max(1);
        let height = (self.height / 2).max(1);
        let source_width = self.width as usize;
        let texel = |x: u32, y: u32, channel: usize| -> u32 {
            let x = x.min(self.width - 1) as usize;
            let y = y.min(self.height - 1) as usize;
            u32::from(
                self.rgba
                    .get((y * source_width + x) * RGBA + channel)
                    .copied()
                    .unwrap_or_default(),
            )
        };
        let mut rgba = Vec::with_capacity(width as usize * height as usize * RGBA);
        for y in 0..height {
            for x in 0..width {
                let (left, top) = (x * self.width / width, y * self.height / height);
                let right = ((x + 1) * self.width / width).max(left + 1);
                let bottom = ((y + 1) * self.height / height).max(top + 1);
                let count = (right - left) * (bottom - top);
                for channel in 0..RGBA {
                    let mut sum = 0;
                    for sample_y in top..bottom {
                        for sample_x in left..right {
                            sum += texel(sample_x, sample_y, channel);
                        }
                    }
                    rgba.push(u8::try_from((sum + count / 2) / count).unwrap_or(u8::MAX));
                }
            }
        }
        Self {
            width,
            height,
            rgba,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenePicture {
    pub texture: Arc<PictureTexture>,
    pub corners: [Point3; 4],
    pub alpha: f32,
    pub in_front: bool,
    pub pick: Option<PickId>,
}

struct GpuTexture {
    source: Arc<PictureTexture>,
    view: Option<wgpu::TextureView>,
}

struct GpuPicture {
    source: Arc<PictureTexture>,
    uniform: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    written: Vec<u8>,
    in_front: bool,
    picked: bool,
    bound: bool,
}

pub(crate) struct PictureCache {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: Vec<GpuTexture>,
    pictures: Vec<GpuPicture>,
}

impl PictureCache {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        Self {
            sampler: sampler(device),
            layout,
            textures: Vec::new(),
            pictures: Vec::new(),
        }
    }

    pub fn sibling(&self, device: &wgpu::Device) -> Self {
        Self {
            layout: self.layout.clone(),
            sampler: sampler(device),
            textures: Vec::new(),
            pictures: Vec::new(),
        }
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pictures: &[ScenePicture],
        anchor: Point3,
    ) -> PicturesPrepared {
        let mut prepared = PicturesPrepared::default();
        let kept_before = self.textures.len();
        self.textures.retain(|texture| {
            pictures
                .iter()
                .any(|picture| Arc::ptr_eq(&picture.texture, &texture.source))
        });
        prepared.changed |= self.textures.len() != kept_before;
        let largest_side = device.limits().max_texture_dimension_2d;
        for picture in pictures {
            if !self
                .textures
                .iter()
                .any(|texture| Arc::ptr_eq(&texture.source, &picture.texture))
            {
                let (view, error) = gpu::scoped(device, || {
                    upload(device, queue, &picture.texture, largest_side)
                });
                let view = view.filter(|_| error.is_none());
                if let Some(error) = error {
                    log::warn!("the graphics device refused a reference image: {error}");
                }
                prepared.refused += u32::from(view.is_none());
                self.textures.push(GpuTexture {
                    source: Arc::clone(&picture.texture),
                    view,
                });
                prepared.changed = true;
            }
        }
        if self.pictures.len() > pictures.len() {
            self.pictures.truncate(pictures.len());
            prepared.changed = true;
        }
        let held_before = self.pictures.len();
        self.pictures.retain(|kept| {
            pictures
                .iter()
                .any(|picture| Arc::ptr_eq(&picture.texture, &kept.source))
        });
        prepared.changed |= self.pictures.len() != held_before;
        for (index, picture) in pictures.iter().enumerate() {
            let bytes = uniform_bytes(picture, anchor);
            let rebind = self
                .pictures
                .get(index)
                .is_none_or(|kept| !Arc::ptr_eq(&kept.source, &picture.texture));
            if rebind {
                let view = self
                    .textures
                    .iter()
                    .find(|texture| Arc::ptr_eq(&texture.source, &picture.texture))
                    .and_then(|texture| texture.view.as_ref());
                let made = self.make(device, view, picture);
                if index < self.pictures.len() {
                    if let Some(slot) = self.pictures.get_mut(index) {
                        *slot = made;
                    }
                } else {
                    self.pictures.push(made);
                }
                prepared.changed = true;
            }
            if let Some(kept) = self.pictures.get_mut(index) {
                kept.in_front = picture.in_front;
                kept.picked = picture.pick.is_some();
                if kept.written != bytes {
                    queue.write_buffer(&kept.uniform, 0, &bytes);
                    kept.written = bytes;
                    prepared.changed = true;
                }
            }
        }
        prepared
    }

    fn make(
        &self,
        device: &wgpu::Device,
        view: Option<&wgpu::TextureView>,
        picture: &ScenePicture,
    ) -> GpuPicture {
        let source = Arc::clone(&picture.texture);
        let in_front = picture.in_front;
        let picked = picture.pick.is_some();
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("picture"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let Some(view) = view else {
            return GpuPicture {
                source,
                bind_group: None,
                uniform,
                written: Vec::new(),
                in_front,
                picked,
                bound: false,
            };
        };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("picture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        GpuPicture {
            source,
            uniform,
            bind_group: Some(bind_group),
            written: Vec::new(),
            in_front,
            picked,
            bound: true,
        }
    }

    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        in_front: bool,
    ) {
        let mut set = false;
        for picture in self.drawn().filter(|picture| picture.in_front == in_front) {
            if !set {
                pass.set_pipeline(pipeline);
                set = true;
            }
            pass.set_bind_group(1, picture.bind_group.as_ref(), &[]);
            pass.draw(0..PICTURE_VERTICES, 0..1);
        }
    }

    pub fn draw_pick(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        let mut set = false;
        for picture in self.drawn().filter(|picture| picture.picked) {
            if !set {
                pass.set_pipeline(pipeline);
                set = true;
            }
            pass.set_bind_group(1, picture.bind_group.as_ref(), &[]);
            pass.draw(0..PICTURE_VERTICES, 0..1);
        }
    }

    fn drawn(&self) -> impl Iterator<Item = &GpuPicture> {
        self.pictures
            .iter()
            .filter(|picture| picture.bound && !picture.written.is_empty())
    }
}

#[derive(Debug, Default)]
pub(crate) struct PicturesPrepared {
    pub changed: bool,
    pub refused: u32,
}

fn sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("picture"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &PictureTexture,
    largest_side: u32,
) -> Option<wgpu::TextureView> {
    let first = (0..texture.levels.len()).find(|level| {
        let (width, height) = texture.level_size(*level);
        width <= largest_side && height <= largest_side
    })?;
    let (width, height) = texture.level_size(first);
    let levels = texture.levels.get(first..)?;
    let made = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("picture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: u32::try_from(levels.len()).ok()?,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (offset, rgba) in levels.iter().enumerate() {
        let (level_width, level_height) = texture.level_size(first + offset);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &made,
                mip_level: u32::try_from(offset).ok()?,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(level_width * RGBA as u32),
                rows_per_image: Some(level_height),
            },
            wgpu::Extent3d {
                width: level_width,
                height: level_height,
                depth_or_array_layers: 1,
            },
        );
    }
    Some(made.create_view(&wgpu::TextureViewDescriptor::default()))
}

fn uniform_bytes(picture: &ScenePicture, anchor: Point3) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(UNIFORM_SIZE as usize);
    for corner in picture.corners.iter().take(CORNERS) {
        let relative = relative_to_eye(*corner, anchor);
        for value in [relative.x, relative.y, relative.z, 1.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    let alpha = if picture.alpha.is_finite() {
        picture.alpha.clamp(0.0, 1.0)
    } else {
        1.0
    };
    for value in [alpha, f32::from(u8::from(picture.in_front)), 0.0, 0.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [PickId::raw(picture.pick), 0, 0, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::PictureTexture;

    #[test]
    fn a_texture_keeps_halving_down_to_one_texel_averaging_each_block() {
        let rgba = [
            [0, 0, 0, 255],
            [255, 255, 255, 255],
            [255, 255, 255, 255],
            [0, 0, 0, 255],
        ]
        .concat();
        let texture = PictureTexture::new(2, 2, rgba).unwrap();

        assert_eq!(texture.levels.len(), 2);
        assert_eq!(texture.levels[1], vec![128, 128, 128, 255]);
        assert_eq!(texture.texel(1, 0), Some([255, 255, 255, 255]));
    }

    #[test]
    fn a_picture_wider_than_a_texture_may_be_is_shrunk_keeping_its_shape() {
        let width = super::MAX_PICTURE_TEXTURE_SIDE * 2;
        let texture = PictureTexture::new(width, 2, vec![200; width as usize * 2 * 4]).unwrap();

        assert_eq!(texture.width(), super::MAX_PICTURE_TEXTURE_SIDE);
        assert_eq!(texture.height(), 1);
        assert_eq!(texture.texel(0, 0), Some([200; 4]));
    }

    #[test]
    fn pixels_not_matching_the_size_are_refused() {
        assert_eq!(PictureTexture::new(2, 2, vec![0; 3]), None);
        assert_eq!(PictureTexture::new(0, 2, Vec::new()), None);
    }
}
