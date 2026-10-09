use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use glam::{Mat4, Vec3};

use crate::{
    RenderError,
    settings::{AdapterPreference, Msaa},
};

pub type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone, Default)]
pub struct DeviceLoss(Arc<AtomicBool>);

impl DeviceLoss {
    pub fn watch(device: &wgpu::Device, wake: Wake) -> Self {
        let loss = Self::default();
        let lost = Arc::clone(&loss.0);
        device.set_device_lost_callback(move |reason, message| {
            log::error!("the graphics device was lost ({reason:?}): {message}");
            lost.store(true, Ordering::Release);
            wake();
        });
        device.on_uncaptured_error(Arc::new(|error| {
            log::error!("the graphics device reported an error: {error}");
        }));
        loss
    }

    pub fn is_lost(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub fn scoped<T>(device: &wgpu::Device, work: impl FnOnce() -> T) -> (T, Option<wgpu::Error>) {
    let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let invalid = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = work();
    let refused = pollster::block_on(invalid.pop());
    let exhausted = pollster::block_on(out_of_memory.pop());
    (value, exhausted.or(refused))
}

pub struct OpenedDevice {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

pub async fn open_device(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
    preference: AdapterPreference,
) -> Result<OpenedDevice, RenderError> {
    let mut failure = RenderError::NoAdapter;
    let preferred = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: power_preference(preference),
            compatible_surface: surface,
            ..Default::default()
        })
        .await;
    let tried = match preferred {
        Ok(adapter) => {
            let info = adapter.get_info();
            match request_device(adapter).await {
                Ok(opened) => return Ok(opened),
                Err(error) => failure = error,
            }
            Some(info)
        }
        Err(error) => {
            log::warn!("no preferred graphics adapter: {error}");
            None
        }
    };
    let mut others: Vec<wgpu::Adapter> = instance
        .enumerate_adapters(wgpu::Backends::all())
        .await
        .into_iter()
        .filter(|adapter| surface.is_none_or(|surface| adapter.is_surface_supported(surface)))
        .filter(|adapter| tried.as_ref() != Some(&adapter.get_info()))
        .collect();
    others.sort_by_key(|adapter| adapter_rank(&adapter.get_info()));
    for adapter in others {
        match request_device(adapter).await {
            Ok(opened) => return Ok(opened),
            Err(error) => failure = error,
        }
    }
    Err(failure)
}

async fn request_device(adapter: wgpu::Adapter) -> Result<OpenedDevice, RenderError> {
    let info = adapter.get_info();
    let mut failure = RenderError::NoAdapter;
    for request in device_requests(&adapter) {
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("caditor"),
            required_features: request.features,
            required_limits: request.limits,
            ..Default::default()
        };
        match adapter.request_device(&descriptor).await {
            Ok((device, queue)) => {
                log::info!(
                    "using graphics adapter {} ({:?}, {:?}) with {} limits",
                    info.name,
                    info.backend,
                    info.device_type,
                    request.name
                );
                return Ok(OpenedDevice {
                    adapter,
                    device,
                    queue,
                });
            }
            Err(error) => {
                log::warn!(
                    "the graphics adapter {} refused a device with {} limits: {error}",
                    info.name,
                    request.name
                );
                failure = RenderError::RequestDevice(error);
            }
        }
    }
    Err(failure)
}

fn power_preference(preference: AdapterPreference) -> wgpu::PowerPreference {
    wgpu::PowerPreference::from_env().unwrap_or_else(|| preference.power())
}

#[cfg(windows)]
const BACKEND_ORDER: [wgpu::Backend; 3] = [
    wgpu::Backend::Dx12,
    wgpu::Backend::Vulkan,
    wgpu::Backend::Gl,
];
#[cfg(not(windows))]
const BACKEND_ORDER: [wgpu::Backend; 2] = [wgpu::Backend::Vulkan, wgpu::Backend::Gl];

fn adapter_rank(info: &wgpu::AdapterInfo) -> (u8, u8) {
    let kind = match info.device_type {
        wgpu::DeviceType::IntegratedGpu => 0,
        wgpu::DeviceType::DiscreteGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    };
    let backend = BACKEND_ORDER
        .iter()
        .position(|backend| *backend == info.backend)
        .map_or(u8::MAX, |place| u8::try_from(place).unwrap_or(u8::MAX));
    (kind, backend)
}

pub struct DeviceRequest {
    pub name: &'static str,
    pub features: wgpu::Features,
    pub limits: wgpu::Limits,
}

pub fn device_requests(adapter: &wgpu::Adapter) -> [DeviceRequest; 3] {
    let offered = adapter.limits();
    let format_features =
        adapter.features() & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    [
        DeviceRequest {
            name: "the adapter's",
            features: format_features,
            limits: offered.clone(),
        },
        DeviceRequest {
            name: "default",
            features: wgpu::Features::empty(),
            limits: within(wgpu::Limits::defaults(), &offered),
        },
        DeviceRequest {
            name: "downlevel",
            features: wgpu::Features::empty(),
            limits: within(wgpu::Limits::downlevel_webgl2_defaults(), &offered),
        },
    ]
}

pub fn within(base: wgpu::Limits, offered: &wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_buffer_size: offered.max_buffer_size,
        ..base.using_resolution(offered.clone())
    }
    .or_worse_values_from(offered)
}

pub fn offered_msaa(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    color: wgpu::TextureFormat,
    depth: wgpu::TextureFormat,
) -> Vec<Msaa> {
    let adapter_specific = device
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        || !adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::WEBGPU_TEXTURE_FORMAT_SUPPORT);
    let features = |format: wgpu::TextureFormat| {
        if adapter_specific {
            adapter.get_texture_format_features(format).flags
        } else {
            format.guaranteed_format_features(device.features()).flags
        }
    };
    let color = features(color);
    let depth = features(depth);
    let resolves = color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE);
    Msaa::ALL
        .into_iter()
        .filter(|level| {
            let count = level.samples();
            count == 1
                || (resolves
                    && color.sample_count_supported(count)
                    && depth.sample_count_supported(count))
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> u64 {
        self.0.len() as u64
    }

    pub fn f32(&mut self, value: f32) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn extend(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.extend_from_slice(bytes);
        self
    }

    pub fn floats(&mut self, values: &[f32]) -> &mut Self {
        for value in values {
            self.f32(*value);
        }
        self
    }

    pub fn vec3(&mut self, value: Vec3) -> &mut Self {
        self.floats(&value.to_array())
    }

    pub fn vec4(&mut self, value: Vec3, w: f32) -> &mut Self {
        self.vec3(value).f32(w)
    }

    pub fn mat4(&mut self, value: Mat4) -> &mut Self {
        self.floats(&value.to_cols_array())
    }
}

pub fn buffer_limit(device: &wgpu::Device) -> u64 {
    device.limits().max_buffer_size / wgpu::COPY_BUFFER_ALIGNMENT * wgpu::COPY_BUFFER_ALIGNMENT
}

pub struct GrowableBuffer {
    label: &'static str,
    usage: wgpu::BufferUsages,
    buffer: wgpu::Buffer,
    small_uploads: u32,
    limit: u64,
    truncated: bool,
}

impl GrowableBuffer {
    pub const INITIAL_SIZE: u64 = 4096;
    pub const SHRINK_AFTER_UPLOADS: u32 = 8;

    pub fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        let limit = buffer_limit(device);
        Self {
            label,
            usage,
            buffer: Self::allocate(device, label, usage, Self::INITIAL_SIZE.min(limit)),
            small_uploads: 0,
            limit,
            truncated: false,
        }
    }

    fn allocate(
        device: &wgpu::Device,
        label: &'static str,
        usage: wgpu::BufferUsages,
        size: u64,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &Bytes,
        unit: u64,
    ) -> u64 {
        let unit = unit.max(wgpu::COPY_BUFFER_ALIGNMENT);
        let length = bytes.len().min(self.limit / unit * unit);
        self.note_truncation(length < bytes.len());

        let fitting = fitting_size(length).min(self.limit);
        self.small_uploads = if fitting < self.buffer.size() / 4 {
            self.small_uploads.saturating_add(1)
        } else {
            0
        };
        if length > self.buffer.size() || self.small_uploads >= Self::SHRINK_AFTER_UPLOADS {
            self.buffer = Self::allocate(device, self.label, self.usage, fitting);
            self.small_uploads = 0;
        }
        let written = usize::try_from(length)
            .ok()
            .and_then(|length| bytes.as_slice().get(..length))
            .unwrap_or_default();
        if !written.is_empty() {
            queue.write_buffer(&self.buffer, 0, written);
        }
        written.len() as u64 / unit
    }

    fn note_truncation(&mut self, truncated: bool) {
        if truncated && !self.truncated {
            log::warn!(
                "the {} do not fit in a graphics buffer of at most {} bytes, so only the first are drawn",
                self.label,
                self.limit
            );
        }
        self.truncated = truncated;
    }

    #[cfg(test)]
    pub fn size(&self) -> u64 {
        self.buffer.size()
    }

    pub fn slice(&self, length: u64) -> wgpu::BufferSlice<'_> {
        self.buffer.slice(..length.min(self.buffer.size()))
    }
}

fn fitting_size(length: u64) -> u64 {
    length.next_power_of_two().max(GrowableBuffer::INITIAL_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_little_endian_values_in_order() {
        let mut bytes = Bytes::default();
        bytes.u32(7).vec4(Vec3::new(1.0, 2.0, 3.0), 4.0);

        assert_eq!(bytes.len(), 20);
        assert_eq!(bytes.as_slice().get(..4), Some(&7u32.to_le_bytes()[..]));
        assert_eq!(bytes.as_slice().get(16..), Some(&4.0f32.to_le_bytes()[..]));
    }

    #[test]
    fn conservative_limits_keep_the_adapters_resolution_and_buffers_but_never_pass_it() {
        let offered = wgpu::Limits {
            max_texture_dimension_2d: 16_384,
            max_buffer_size: 1 << 34,
            max_vertex_attributes: 12,
            ..wgpu::Limits::downlevel_defaults()
        };

        let limits = within(wgpu::Limits::downlevel_webgl2_defaults(), &offered);

        assert_eq!(limits.max_texture_dimension_2d, 16_384);
        assert_eq!(limits.max_buffer_size, 1 << 34);
        assert_eq!(limits.max_vertex_attributes, 12);
        assert_eq!(limits.max_storage_buffers_per_shader_stage, 0);
        assert!(limits.check_limits(&offered));
    }

    #[test]
    fn integrated_vulkan_adapters_come_first_and_software_ones_last() {
        let info = wgpu::AdapterInfo::new;
        let mut adapters = [
            info(wgpu::DeviceType::Cpu, wgpu::Backend::Vulkan),
            info(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan),
            info(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Gl),
            info(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Vulkan),
        ];

        adapters.sort_by_key(adapter_rank);

        assert_eq!(
            adapters.map(|adapter| (adapter.device_type, adapter.backend)),
            [
                (wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Vulkan),
                (wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Gl),
                (wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan),
                (wgpu::DeviceType::Cpu, wgpu::Backend::Vulkan),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn direct3d_comes_before_vulkan_and_opengl_on_windows() {
        let info = wgpu::AdapterInfo::new;
        let mut adapters = [
            info(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Gl),
            info(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Vulkan),
            info(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Dx12),
        ];

        adapters.sort_by_key(adapter_rank);

        assert_eq!(
            adapters.map(|adapter| adapter.backend),
            [
                wgpu::Backend::Dx12,
                wgpu::Backend::Vulkan,
                wgpu::Backend::Gl
            ]
        );
    }
}
