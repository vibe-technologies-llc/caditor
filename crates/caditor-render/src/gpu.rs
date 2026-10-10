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

const QUAD_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];
pub const QUAD_INDEX_COUNT: u32 = QUAD_INDICES.len() as u32;

#[derive(Clone)]
pub struct QuadIndices(wgpu::Buffer);

impl QuadIndices {
    pub fn new(device: &wgpu::Device) -> Self {
        let bytes: Vec<u8> = QUAD_INDICES
            .iter()
            .flat_map(|index| index.to_le_bytes())
            .collect();
        Self(wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("quad corners"),
                contents: &bytes,
                usage: wgpu::BufferUsages::INDEX,
            },
        ))
    }

    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_index_buffer(self.0.slice(..), wgpu::IndexFormat::Uint16);
    }
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

pub trait Pack {
    fn put(&mut self, bytes: &[u8]) -> &mut Self;

    fn f32(&mut self, value: f32) -> &mut Self {
        self.put(&value.to_le_bytes())
    }

    fn u32(&mut self, value: u32) -> &mut Self {
        self.put(&value.to_le_bytes())
    }

    fn floats(&mut self, values: &[f32]) -> &mut Self {
        for value in values {
            self.f32(*value);
        }
        self
    }

    fn vec3(&mut self, value: Vec3) -> &mut Self {
        self.floats(&value.to_array())
    }

    fn vec4(&mut self, value: Vec3, w: f32) -> &mut Self {
        self.vec3(value).f32(w)
    }

    fn mat4(&mut self, value: Mat4) -> &mut Self {
        self.floats(&value.to_cols_array())
    }

    fn unorm8x4(&mut self, values: [f32; 4]) -> &mut Self {
        self.put(&values.map(unorm8))
    }

    fn octahedral(&mut self, normal: Vec3) -> &mut Self {
        self.put(&octahedral(normal))
    }
}

pub const OCTAHEDRAL_BYTES: usize = 4;
const SNORM16_SCALE: f32 = i16::MAX as f32;
const UNORM8_SCALE: f32 = u8::MAX as f32;
const ABSENT_NORMAL: [f32; 2] = [-1.0, -1.0];
const NEAR_ABSENT_SUM: f32 = -1.9996;
const FAR_CORNER: [f32; 2] = [1.0, 1.0];

fn unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * UNORM8_SCALE).round() as u8
}

fn snorm16(value: f32) -> [u8; 2] {
    ((value.clamp(-1.0, 1.0) * SNORM16_SCALE).round() as i16).to_le_bytes()
}

fn octahedral(normal: Vec3) -> [u8; OCTAHEDRAL_BYTES] {
    let [[x0, x1], [y0, y1]] = folded(normal).map(snorm16);
    [x0, x1, y0, y1]
}

fn folded(normal: Vec3) -> [f32; 2] {
    let length = normal.abs().element_sum();
    if !(length > 0.0 && length.is_finite()) {
        return ABSENT_NORMAL;
    }
    let on = normal / length;
    let side = |value: f32| if value >= 0.0 { 1.0 } else { -1.0 };
    let [x, y] = if on.z >= 0.0 {
        [on.x, on.y]
    } else {
        [
            (1.0 - on.y.abs()) * side(on.x),
            (1.0 - on.x.abs()) * side(on.y),
        ]
    };
    if x + y < NEAR_ABSENT_SUM {
        FAR_CORNER
    } else {
        [x, y]
    }
}

#[derive(Debug, Default)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    const KEPT_CAPACITY: usize = 64 << 10;

    pub fn clear(&mut self) {
        if self.0.capacity() > Self::KEPT_CAPACITY {
            self.0 = Vec::new();
        } else {
            self.0.clear();
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    #[cfg(test)]
    pub fn len(&self) -> u64 {
        self.0.len() as u64
    }

    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }
}

impl Pack for Bytes {
    fn put(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.extend_from_slice(bytes);
        self
    }
}

pub struct Record<const N: usize> {
    bytes: [u8; N],
    at: usize,
}

impl<const N: usize> Pack for Record<N> {
    fn put(&mut self, bytes: &[u8]) -> &mut Self {
        let end = self.at.saturating_add(bytes.len());
        if let Some(slot) = self.bytes.get_mut(self.at..end) {
            slot.copy_from_slice(bytes);
        }
        self.at = end;
        self
    }
}

pub fn record<const N: usize>(fill: impl FnOnce(&mut Record<N>)) -> [u8; N] {
    let mut record = Record {
        bytes: [0; N],
        at: 0,
    };
    fill(&mut record);
    record.bytes
}

pub fn write_records<const N: usize>(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    offset: u64,
    count: usize,
    records: impl IntoIterator<Item = [u8; N]>,
) {
    let size = (count as u64)
        .checked_mul(N as u64)
        .and_then(wgpu::BufferSize::new);
    let Some(mut view) = size.and_then(|size| queue.write_buffer_with(buffer, offset, size)) else {
        return;
    };
    let mut records = records.into_iter();
    for slot in view.slice(..).into_chunks::<N>().0 {
        slot.write(records.next().unwrap_or([0; N]));
    }
}

pub fn resolves_linearly(adapter: &wgpu::Adapter) -> bool {
    adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::VIEW_FORMATS)
}

pub fn buffer_limit(device: &wgpu::Device) -> u64 {
    device.limits().max_buffer_size / wgpu::COPY_BUFFER_ALIGNMENT * wgpu::COPY_BUFFER_ALIGNMENT
}

pub struct GrowableBuffer {
    label: &'static str,
    usage: wgpu::BufferUsages,
    buffer: wgpu::Buffer,
    limit: u64,
    truncated: bool,
}

impl GrowableBuffer {
    pub const INITIAL_SIZE: u64 = 4096;
    const HEADROOM_SHARE: u64 = 4;
    const SHRINK_SHARE: u64 = 4;

    pub fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        let limit = buffer_limit(device);
        Self {
            label,
            usage,
            buffer: Self::allocate(device, label, usage, Self::INITIAL_SIZE.min(limit)),
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

    pub fn upload<const N: usize>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        Records {
            count,
            per_primitive,
            records,
        }: Records<impl Iterator<Item = [u8; N]>>,
    ) -> u64 {
        let primitive = (N as u64).saturating_mul(per_primitive.max(1));
        let fitting = self.limit / primitive.max(1) * per_primitive.max(1);
        let kept = count.min(fitting);
        self.note_truncation(kept < count);

        let length = kept * N as u64;
        if let Some(size) = resized(self.buffer.size(), length, self.limit) {
            self.buffer = Self::allocate(device, self.label, self.usage, size);
        }
        let kept_records = usize::try_from(kept).unwrap_or(usize::MAX);
        write_records(
            queue,
            &self.buffer,
            0,
            kept_records,
            records.take(kept_records),
        );
        kept
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

    pub fn slice_between(&self, start: u64, end: u64) -> wgpu::BufferSlice<'_> {
        let size = self.buffer.size();
        let end = end.min(size);
        self.buffer.slice(start.min(end)..end)
    }
}

pub struct Records<I> {
    pub count: u64,
    pub per_primitive: u64,
    pub records: I,
}

fn resized(current: u64, length: u64, limit: u64) -> Option<u64> {
    let fitting = fitting_size(length).min(limit);
    let outgrown = length > current;
    let mostly_empty = length < current / GrowableBuffer::SHRINK_SHARE && fitting < current;
    (outgrown || mostly_empty).then_some(fitting)
}

fn fitting_size(length: u64) -> u64 {
    length
        .saturating_add(length / GrowableBuffer::HEADROOM_SHARE)
        .div_ceil(wgpu::COPY_BUFFER_ALIGNMENT)
        .saturating_mul(wgpu::COPY_BUFFER_ALIGNMENT)
        .max(GrowableBuffer::INITIAL_SIZE)
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
    fn a_record_packs_values_in_order_and_drops_what_does_not_fit() {
        let packed = record::<8>(|record| {
            record.u32(7).f32(2.0).u32(9);
        });

        assert_eq!(packed[..4], 7u32.to_le_bytes());
        assert_eq!(packed[4..], 2.0f32.to_le_bytes());
    }

    fn unfolded(packed: [u8; OCTAHEDRAL_BYTES]) -> Vec3 {
        let [x0, x1, y0, y1] = packed;
        let normal = [[x0, x1], [y0, y1]]
            .map(|bytes| (f32::from(i16::from_le_bytes(bytes)) / SNORM16_SCALE).max(-1.0));
        let [x, y] = normal;
        if x + y < -1.9998 {
            return Vec3::ZERO;
        }
        let z = 1.0 - x.abs() - y.abs();
        let side = |value: f32| if value >= 0.0 { 1.0 } else { -1.0 };
        let (x, y) = if z < 0.0 {
            ((1.0 - y.abs()) * side(x), (1.0 - x.abs()) * side(y))
        } else {
            (x, y)
        };
        Vec3::new(x, y, z).normalize()
    }

    fn error(normal: Vec3) -> f64 {
        unfolded(octahedral(normal))
            .as_dvec3()
            .angle_between(normal.as_dvec3().normalize())
    }

    #[test]
    fn octahedral_normals_come_back_within_a_hundredth_of_a_degree() {
        let steps = 64;
        let mut worst = 0.0f64;
        for latitude in 0..=steps {
            for longitude in 0..steps * 2 {
                let polar = std::f32::consts::PI * latitude as f32 / steps as f32;
                let azimuth = std::f32::consts::PI * longitude as f32 / steps as f32;
                let normal = Vec3::new(
                    polar.sin() * azimuth.cos(),
                    polar.sin() * azimuth.sin(),
                    polar.cos(),
                );
                worst = worst.max(error(normal));
            }
        }

        assert!(worst.to_degrees() < 0.01, "{}", worst.to_degrees());
    }

    #[test]
    fn a_missing_normal_stays_missing_and_nothing_else_reads_as_missing() {
        let near_down = Vec3::new(-1e-4, -1e-4, -1.0).normalize();
        let barely_down = Vec3::new(-3e-4, -1e-4, -1.0).normalize();

        assert_eq!(unfolded(octahedral(Vec3::ZERO)), Vec3::ZERO);
        assert_eq!(unfolded(octahedral(Vec3::NAN)), Vec3::ZERO);
        assert!(error(near_down) < 4e-4);
        assert!(error(barely_down) < 4e-4);
        assert_eq!(unfolded(octahedral(-Vec3::Z)), -Vec3::Z);
        assert_eq!(unfolded(octahedral(Vec3::new(-0.0, -0.0, -1.0))), -Vec3::Z);
    }

    #[test]
    fn eight_bit_colours_pack_exactly_and_others_round() {
        let packed = record::<4>(|record| {
            record.unorm8x4([0.0, 128.0 / 255.0, 1.0, 0.35]);
        });
        let clamped = record::<4>(|record| {
            record.unorm8x4([-1.0, 2.0, f32::NAN, 0.5]);
        });

        assert_eq!(packed, [0, 128, 255, 89]);
        assert_eq!(clamped, [0, 255, 0, 128]);
    }

    #[test]
    fn staging_keeps_only_a_small_capacity_once_cleared() {
        let mut small = Bytes::default();
        small.floats(&[1.0; 100]);
        let mut large = Bytes::default();
        large.floats(&[1.0; 100_000]);

        small.clear();
        large.clear();

        assert!(small.capacity() >= 400);
        assert_eq!(large.capacity(), 0);
    }

    #[test]
    fn a_buffer_grows_to_a_quarter_more_than_it_needs_and_shrinks_once_under_a_quarter_full() {
        let limit = 1 << 30;
        let grown = resized(4096, 100_001, limit);
        let kept_full = resized(125_004, 100_000, limit);
        let kept_quarter = resized(125_004, 125_004 / 4, limit);
        let shrunk = resized(125_004, 30_000, limit);
        let emptied = resized(125_004, 0, limit);
        let small_stays = resized(4096, 0, limit);
        let capped = resized(4096, limit - 4, limit);

        assert_eq!(grown, Some(125_004));
        assert_eq!(kept_full, None);
        assert_eq!(kept_quarter, None);
        assert_eq!(shrunk, Some(37_500));
        assert_eq!(emptied, Some(GrowableBuffer::INITIAL_SIZE));
        assert_eq!(small_stays, None);
        assert_eq!(capped, Some(limit));
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
