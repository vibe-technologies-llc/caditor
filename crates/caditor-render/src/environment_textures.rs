use crate::environment::{Environment, IRRADIANCE_HEIGHT, IRRADIANCE_WIDTH};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const TEXEL_BYTES: u32 = 8;

pub(crate) struct EnvironmentViews {
    pub radiance: wgpu::TextureView,
    pub irradiance: wgpu::TextureView,
}

pub(crate) fn sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("environment"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

pub(crate) fn stand_in(device: &wgpu::Device) -> EnvironmentViews {
    EnvironmentViews {
        radiance: texture(device, "environment stand-in", (1, 1), 1)
            .create_view(&Default::default()),
        irradiance: texture(device, "environment stand-in", (1, 1), 1)
            .create_view(&Default::default()),
    }
}

pub(crate) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    environment: &Environment,
) -> Option<EnvironmentViews> {
    let largest = device.limits().max_texture_dimension_2d;
    if environment.width() > largest {
        return None;
    }
    let levels = u32::try_from(environment.level_count()).ok()?;
    let radiance = texture(
        device,
        "environment",
        (environment.width(), environment.height()),
        levels,
    );
    for level in 0..environment.level_count() {
        let (bytes, width, height) = environment.level(level)?;
        write(
            queue,
            &radiance,
            u32::try_from(level).ok()?,
            bytes,
            (width, height),
        );
    }
    let irradiance = texture(
        device,
        "environment irradiance",
        (IRRADIANCE_WIDTH, IRRADIANCE_HEIGHT),
        1,
    );
    write(
        queue,
        &irradiance,
        0,
        environment.irradiance(),
        (IRRADIANCE_WIDTH, IRRADIANCE_HEIGHT),
    );
    Some(EnvironmentViews {
        radiance: radiance.create_view(&Default::default()),
        irradiance: irradiance.create_view(&Default::default()),
    })
}

fn texture(
    device: &wgpu::Device,
    label: &'static str,
    (width, height): (u32, u32),
    levels: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
    bytes: &[u8],
    (width, height): (u32, u32),
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * TEXEL_BYTES),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}
