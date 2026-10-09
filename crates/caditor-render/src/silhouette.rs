use std::sync::Arc;

use caditor_geometry::{Point3, RigidTransform};
use glam::Vec3;

use crate::{
    culling::{ClipWindow, placed_corners},
    gpu::{self, Bytes},
    mesh::{Corner, Placed, PlacedAt, ShadedMesh, UploadBudget, take_of},
    scene::{Color, Layer, Primitive},
};

pub const SILHOUETTE_STRIDE: u64 = 60;
const SILHOUETTE_BINDING: u32 = 3;
const SILHOUETTE_UNIFORM_BYTES: u64 = 80;
const QUAD_VERTICES: u32 = 6;
const SNORM16_SCALE: f32 = i16::MAX as f32;
const POSITIONS_BYTES: usize = 36;
const NORMAL_BYTES: usize = 8;

#[derive(Debug, Clone)]
pub struct Silhouette {
    pub mesh: Arc<ShadedMesh>,
    pub color: Color,
    pub width: f32,
    pub dashed: bool,
    pub dashed_where_hidden: bool,
    pub placement: Option<RigidTransform>,
}

impl PartialEq for Silhouette {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mesh, &other.mesh)
            && self.color == other.color
            && self.width == other.width
            && self.dashed == other.dashed
            && self.dashed_where_hidden == other.dashed_where_hidden
            && self.placement == other.placement
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Written {
    placed: PlacedAt,
    color: Color,
    width: f32,
    dashed: bool,
    dashed_where_hidden: bool,
}

struct Chunk {
    buffer: wgpu::Buffer,
    count: u32,
}

struct SilhouetteUpload {
    mesh: Arc<ShadedMesh>,
    chunks: Vec<Chunk>,
    per_chunk: usize,
    total: usize,
    written: usize,
    next_triangle: usize,
}

impl SilhouetteUpload {
    fn start(device: &wgpu::Device, mesh: Arc<ShadedMesh>) -> Self {
        let total = mesh.curved_triangle_count();
        let per_chunk = usize::try_from(gpu::buffer_limit(device) / SILHOUETTE_STRIDE)
            .unwrap_or(usize::MAX)
            .max(1);
        let chunks = (0..total.div_ceil(per_chunk))
            .map(|chunk| {
                let count = per_chunk.min(total - chunk * per_chunk);
                Chunk {
                    buffer: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("silhouette triangles"),
                        size: count as u64 * SILHOUETTE_STRIDE,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    count: u32::try_from(count).unwrap_or(u32::MAX),
                }
            })
            .collect();
        Self {
            mesh,
            chunks,
            per_chunk,
            total,
            written: 0,
            next_triangle: 0,
        }
    }

    fn advance(
        &mut self,
        queue: &wgpu::Queue,
        bytes: &mut Bytes,
        budget: &mut UploadBudget,
    ) -> bool {
        while self.written < self.total {
            let (chunk, within) = (self.written / self.per_chunk, self.written % self.per_chunk);
            let room = (self.per_chunk - within).min(self.total - self.written);
            let granted = budget.grant(room, SILHOUETTE_STRIDE);
            if granted == 0 {
                return false;
            }
            bytes.clear();
            let mut staged = 0;
            for (index, corners) in self.mesh.curved_triangles(self.next_triangle).take(granted) {
                stage_triangle(bytes, &corners);
                self.next_triangle = index + 1;
                staged += 1;
            }
            if let Some(chunk) = self.chunks.get(chunk) {
                queue.write_buffer(
                    &chunk.buffer,
                    within as u64 * SILHOUETTE_STRIDE,
                    bytes.as_slice(),
                );
            }
            self.written += staged;
            if staged < granted {
                self.total = self.written;
            }
        }
        true
    }

    fn finish(self, device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> GpuSilhouette {
        let drawn = u32::try_from(self.total).unwrap_or(u32::MAX);
        let mut left = drawn;
        let chunks = self
            .chunks
            .into_iter()
            .map(|chunk| {
                let count = chunk.count.min(left);
                left -= count;
                Chunk { count, ..chunk }
            })
            .filter(|chunk| chunk.count > 0)
            .collect();
        GpuSilhouette::with_chunks(device, layout, self.mesh, chunks)
    }
}

fn stage_triangle(bytes: &mut Bytes, corners: &[Corner; 3]) {
    let mut packed = [0u8; SILHOUETTE_STRIDE as usize];
    let (positions, normals) = packed.split_at_mut(POSITIONS_BYTES);
    let position_floats = corners.iter().flat_map(|corner| corner.position.to_array());
    for (slot, value) in positions
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(position_floats)
    {
        *slot = value.to_le_bytes();
    }
    for (slot, corner) in normals
        .as_chunks_mut::<NORMAL_BYTES>()
        .0
        .iter_mut()
        .zip(corners)
    {
        *slot = snorm16(corner.normal);
    }
    bytes.extend(&packed);
}

fn snorm16(normal: Vec3) -> [u8; NORMAL_BYTES] {
    let snorm = |value: f32| (value.clamp(-1.0, 1.0) * SNORM16_SCALE) as i16;
    let [[x0, x1], [y0, y1], [z0, z1]] = normal.to_array().map(|value| snorm(value).to_le_bytes());
    [x0, x1, y0, y1, z0, z1, 0, 0]
}

struct GpuSilhouette {
    mesh: Arc<ShadedMesh>,
    chunks: Arc<[Chunk]>,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    written: Option<Written>,
    corners: Option<[Point3; 8]>,
}

impl GpuSilhouette {
    fn with_chunks(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        mesh: Arc<ShadedMesh>,
        chunks: Arc<[Chunk]>,
    ) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("silhouette placement"),
            size: SILHOUETTE_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("silhouette placement"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: SILHOUETTE_BINDING,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            mesh,
            chunks,
            uniform,
            bind_group,
            written: None,
            corners: None,
        }
    }

    fn sharing(&self, device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        Self::with_chunks(
            device,
            layout,
            Arc::clone(&self.mesh),
            Arc::clone(&self.chunks),
        )
    }

    fn write(
        &mut self,
        queue: &wgpu::Queue,
        bytes: &mut Bytes,
        silhouette: &Silhouette,
        eye: Point3,
    ) {
        let written = Written {
            placed: PlacedAt {
                placement: silhouette.placement,
                eye,
            },
            color: silhouette.color,
            width: silhouette.width,
            dashed: silhouette.dashed,
            dashed_where_hidden: silhouette.dashed_where_hidden,
        };
        if self.written == Some(written) {
            return;
        }
        self.written = Some(written);
        self.corners = self
            .mesh
            .bounds()
            .map(|bounds| placed_corners(bounds, silhouette.placement));
        let placed = Placed::of(self.mesh.origin(), silhouette.placement, eye);
        let [turn_x, turn_y, turn_z] = placed.turn;
        bytes.clear();
        bytes
            .vec4(placed.offset, silhouette.width)
            .floats(&silhouette.color.to_array())
            .vec4(turn_x, Layer::Model.depth_bias(Primitive::Line))
            .vec4(turn_y, if silhouette.dashed { 1.0 } else { 0.0 })
            .vec4(turn_z, 0.0);
        queue.write_buffer(&self.uniform, 0, bytes.as_slice());
    }
}

enum Prepared {
    Ready(Box<GpuSilhouette>),
    Uploading(SilhouetteUpload),
}

pub struct SilhouetteCache {
    layout: wgpu::BindGroupLayout,
    silhouettes: Vec<GpuSilhouette>,
    uploads: Vec<SilhouetteUpload>,
    rejected: Vec<Arc<ShadedMesh>>,
    staging: Bytes,
}

impl SilhouetteCache {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("silhouette placement"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: SILHOUETTE_BINDING,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        Self {
            layout,
            silhouettes: Vec::new(),
            uploads: Vec::new(),
            rejected: Vec::new(),
            staging: Bytes::default(),
        }
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn sibling(&self, device: &wgpu::Device) -> Self {
        Self {
            layout: self.layout.clone(),
            silhouettes: self
                .silhouettes
                .iter()
                .map(|silhouette| silhouette.sharing(device, &self.layout))
                .collect(),
            uploads: Vec::new(),
            rejected: self.rejected.clone(),
            staging: Bytes::default(),
        }
    }

    pub fn is_uploading(&self) -> bool {
        !self.uploads.is_empty()
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        silhouettes: &[Silhouette],
        eye: Point3,
        budget: &mut UploadBudget,
    ) -> u32 {
        let mut previous = std::mem::take(&mut self.silhouettes);
        let mut uploads = std::mem::take(&mut self.uploads);
        let mut rejected = std::mem::take(&mut self.rejected);
        let mut kept_rejected = Vec::new();
        let mut newly_rejected = 0;
        for silhouette in silhouettes
            .iter()
            .filter(|silhouette| !silhouette.mesh.is_empty())
        {
            if let Some(refused) = take_of(&mut rejected, &silhouette.mesh, |refused| refused) {
                kept_rejected.push(refused);
                continue;
            }
            let reused = take_of(&mut previous, &silhouette.mesh, |cached| &cached.mesh);
            let started = take_of(&mut uploads, &silhouette.mesh, |upload| &upload.mesh);
            let staging = &mut self.staging;
            let layout = &self.layout;
            let (prepared, error) = gpu::scoped(device, || {
                let mut ready = match reused {
                    Some(gpu) => gpu,
                    None => {
                        let mut upload = started.unwrap_or_else(|| {
                            SilhouetteUpload::start(device, Arc::clone(&silhouette.mesh))
                        });
                        if !upload.advance(queue, staging, budget) {
                            return Prepared::Uploading(upload);
                        }
                        upload.finish(device, layout)
                    }
                };
                ready.write(queue, staging, silhouette, eye);
                Prepared::Ready(Box::new(ready))
            });
            match (prepared, error) {
                (Prepared::Ready(gpu), None) => self.silhouettes.push(*gpu),
                (Prepared::Uploading(upload), None) => self.uploads.push(upload),
                (_, Some(error)) => {
                    log::warn!(
                        "the graphics device refused the silhouette of a mesh of {} triangles, so it is not drawn: {error}",
                        silhouette.mesh.triangle_count()
                    );
                    kept_rejected.push(Arc::clone(&silhouette.mesh));
                    newly_rejected += 1;
                }
            }
        }
        if self.is_uploading() {
            self.keep_previous(device, queue, previous, eye);
        }
        self.rejected = kept_rejected;
        newly_rejected
    }

    fn keep_previous(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut previous: Vec<GpuSilhouette>,
        eye: Point3,
    ) {
        let staging = &mut self.staging;
        let ((), error) = gpu::scoped(device, || {
            for silhouette in &mut previous {
                if let Some(written) = silhouette.written {
                    let kept = Silhouette {
                        mesh: Arc::clone(&silhouette.mesh),
                        color: written.color,
                        width: written.width,
                        dashed: written.dashed,
                        dashed_where_hidden: written.dashed_where_hidden,
                        placement: written.placed.placement,
                    };
                    silhouette.write(queue, staging, &kept, eye);
                }
            }
        });
        match error {
            None => self.silhouettes.append(&mut previous),
            Some(error) => log::warn!(
                "the graphics device refused to keep {} silhouettes shown while their replacements upload: {error}",
                previous.len()
            ),
        }
    }

    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        window: &ClipWindow,
    ) {
        self.draw_chosen(pass, pipeline, window, |_| true);
    }

    pub fn draw_hidden(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        window: &ClipWindow,
    ) {
        self.draw_chosen(pass, pipeline, window, |written| {
            written.dashed_where_hidden
        });
    }

    fn draw_chosen(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        window: &ClipWindow,
        chosen: impl Fn(&Written) -> bool,
    ) {
        let mut seen = self
            .silhouettes
            .iter()
            .filter(|silhouette| !silhouette.chunks.is_empty())
            .filter(|silhouette| silhouette.written.as_ref().is_some_and(&chosen))
            .filter(|silhouette| {
                silhouette
                    .corners
                    .is_none_or(|corners| window.sees(&corners))
            })
            .peekable();
        if seen.peek().is_none() {
            return;
        }
        pass.set_pipeline(pipeline);
        for silhouette in seen {
            pass.set_bind_group(1, &silhouette.bind_group, &[]);
            for chunk in silhouette.chunks.iter() {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                pass.draw(0..QUAD_VERTICES, 0..chunk.count);
            }
        }
    }

    #[cfg(test)]
    pub fn triangles(&self) -> usize {
        self.silhouettes
            .iter()
            .flat_map(|silhouette| silhouette.chunks.iter())
            .map(|chunk| chunk.count as usize)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Vector3;

    use super::*;
    use crate::mesh::{MeshFace, MeshPoint};

    fn bent(lean: f64) -> ShadedMesh {
        let at = |x: f64, y: f64, normal: Vector3| MeshPoint {
            position: Point3::new(x, y, 0.0),
            normal,
        };
        ShadedMesh::new([MeshFace {
            points: vec![
                at(0.0, 0.0, Vector3::Z),
                at(1.0, 0.0, Vector3::Z),
                at(0.0, 1.0, Vector3::Z),
                at(1.0, 1.0, Vector3::new(lean, 0.0, 1.0)),
            ],
            triangles: vec![[0, 1, 2], [1, 3, 2]],
        }])
    }

    #[test]
    fn only_triangles_whose_normals_differ_can_hold_a_silhouette() {
        let curved: Vec<usize> = bent(0.5)
            .curved_triangles(0)
            .map(|(index, _)| index)
            .collect();

        assert_eq!(curved, vec![1]);
        assert_eq!(bent(0.5).curved_triangle_count(), 1);
        assert_eq!(bent(0.0).curved_triangles(0).count(), 0);
        assert_eq!(bent(0.5).curved_triangles(2).count(), 0);
    }

    #[test]
    fn triangles_pack_three_positions_then_three_signed_sixteen_bit_normals() {
        let mut bytes = Bytes::default();
        let corner = |x: f32, normal: Vec3| Corner {
            position: Vec3::new(x, 0.0, 0.0),
            normal,
        };

        stage_triangle(
            &mut bytes,
            &[
                corner(1.0, Vec3::new(1.0, -1.0, 0.5)),
                corner(2.0, Vec3::Z),
                corner(3.0, Vec3::X),
            ],
        );

        assert_eq!(bytes.len(), SILHOUETTE_STRIDE);
        assert_eq!(bytes.as_slice()[..4], 1.0f32.to_le_bytes());
        assert_eq!(bytes.as_slice()[24..28], 3.0f32.to_le_bytes());
        assert_eq!(
            bytes.as_slice()[36..44],
            [0xff, 0x7f, 0x01, 0x80, 0xff, 0x3f, 0, 0]
        );
        assert_eq!(bytes.as_slice()[44..52], [0, 0, 0, 0, 0xff, 0x7f, 0, 0]);
    }
}
