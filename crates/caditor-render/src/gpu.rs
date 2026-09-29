use glam::{Mat4, Vec3};

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
    pub const SHRINK_AFTER_UPLOADS: u32 = 300;

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
}
