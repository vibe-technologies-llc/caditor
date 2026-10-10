use std::sync::Arc;

use caditor_geometry::{Point3, RigidTransform};

use crate::{
    by_mesh::{ByMesh, DrawnOrder, OfMesh},
    culling::{ClipWindow, placed_corners},
    gpu::{self, Bytes, Pack},
    mesh::{Corner, Placed, PlacedAt, ShadedMesh, UploadBudget},
    scene::Color,
};

const SILHOUETTE_BYTES: usize = 48;
pub const SILHOUETTE_STRIDE: u64 = SILHOUETTE_BYTES as u64;
const SILHOUETTE_BINDING: u32 = 3;
const SILHOUETTE_UNIFORM_BYTES: u64 = 80;

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

impl Written {
    fn of(silhouette: &Silhouette, anchor: Point3) -> Self {
        Self {
            placed: PlacedAt {
                placement: silhouette.placement,
                anchor,
            },
            color: silhouette.color,
            width: silhouette.width,
            dashed: silhouette.dashed,
            dashed_where_hidden: silhouette.dashed_where_hidden,
        }
    }
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

    fn advance(&mut self, queue: &wgpu::Queue, budget: &mut UploadBudget) -> bool {
        while self.written < self.total {
            let (chunk, within) = (self.written / self.per_chunk, self.written % self.per_chunk);
            let room = (self.per_chunk - within).min(self.total - self.written);
            let granted = budget.grant(room, SILHOUETTE_STRIDE);
            if granted == 0 {
                return false;
            }
            let mut staged = 0;
            let mut next_triangle = self.next_triangle;
            let records = self
                .mesh
                .curved_triangles(self.next_triangle)
                .take(granted)
                .map(|(index, corners)| {
                    next_triangle = index + 1;
                    staged += 1;
                    packed_triangle(&corners)
                });
            if let Some(chunk) = self.chunks.get(chunk) {
                let offset = within as u64 * SILHOUETTE_STRIDE;
                gpu::write_records(queue, &chunk.buffer, offset, granted, records);
            }
            self.next_triangle = next_triangle;
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

fn packed_triangle(corners: &[Corner; 3]) -> [u8; SILHOUETTE_BYTES] {
    gpu::record(|record| {
        for corner in corners {
            record.vec3(corner.position);
        }
        for corner in corners {
            record.octahedral(corner.normal);
        }
    })
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

    fn needs_writing(&self, silhouette: &Silhouette, anchor: Point3) -> bool {
        self.written != Some(Written::of(silhouette, anchor))
    }

    fn write(
        &mut self,
        queue: &wgpu::Queue,
        bytes: &mut Bytes,
        silhouette: &Silhouette,
        anchor: Point3,
    ) {
        let written = Written::of(silhouette, anchor);
        if self.written == Some(written) {
            return;
        }
        self.written = Some(written);
        self.corners = self
            .mesh
            .bounds()
            .map(|bounds| placed_corners(bounds, silhouette.placement));
        let placed = Placed::of(self.mesh.origin(), silhouette.placement, anchor);
        let [turn_x, turn_y, turn_z] = placed.turn;
        bytes.clear();
        bytes
            .vec4(placed.offset, silhouette.width)
            .floats(&silhouette.color.to_array())
            .vec4(turn_x, 0.0)
            .vec4(turn_y, if silhouette.dashed { 1.0 } else { 0.0 })
            .vec4(turn_z, 0.0);
        queue.write_buffer(&self.uniform, 0, bytes.as_slice());
    }
}

impl OfMesh for GpuSilhouette {
    fn mesh(&self) -> &Arc<ShadedMesh> {
        &self.mesh
    }
}

impl OfMesh for SilhouetteUpload {
    fn mesh(&self) -> &Arc<ShadedMesh> {
        &self.mesh
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
    previous: ByMesh<GpuSilhouette>,
    started: ByMesh<SilhouetteUpload>,
    refused: ByMesh<Arc<ShadedMesh>>,
    staging: Bytes,
    order: DrawnOrder,
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
            previous: ByMesh::default(),
            started: ByMesh::default(),
            refused: ByMesh::default(),
            staging: Bytes::default(),
            order: DrawnOrder::default(),
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
            previous: ByMesh::default(),
            started: ByMesh::default(),
            refused: ByMesh::default(),
            staging: Bytes::default(),
            order: DrawnOrder::default(),
        }
    }

    pub fn is_uploading(&self) -> bool {
        !self.uploads.is_empty()
    }

    pub fn changed(&self) -> bool {
        self.order.changed()
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        silhouettes: &[Silhouette],
        anchor: Point3,
        budget: &mut UploadBudget,
    ) -> u32 {
        self.previous.refill(&mut self.silhouettes);
        self.started.refill(&mut self.uploads);
        self.refused.refill(&mut self.rejected);
        let mut newly_rejected = 0;
        let mut rewritten = false;
        for silhouette in silhouettes
            .iter()
            .filter(|silhouette| !silhouette.mesh.is_empty())
        {
            if let Some(refused) = self.refused.take(&silhouette.mesh) {
                self.rejected.push(refused);
                continue;
            }
            let reused = self.previous.take(&silhouette.mesh);
            if let Some(ready) = &reused
                && !ready.needs_writing(silhouette, anchor)
            {
                self.silhouettes.extend(reused);
                continue;
            }
            let started = self.started.take(&silhouette.mesh);
            let staging = &mut self.staging;
            let layout = &self.layout;
            let (prepared, error) = gpu::scoped(device, || {
                let mut ready = match reused {
                    Some(gpu) => gpu,
                    None => {
                        let mut upload = started.unwrap_or_else(|| {
                            SilhouetteUpload::start(device, Arc::clone(&silhouette.mesh))
                        });
                        if !upload.advance(queue, budget) {
                            return Prepared::Uploading(upload);
                        }
                        upload.finish(device, layout)
                    }
                };
                ready.write(queue, staging, silhouette, anchor);
                Prepared::Ready(Box::new(ready))
            });
            match (prepared, error) {
                (Prepared::Ready(gpu), None) => {
                    rewritten = true;
                    self.silhouettes.push(*gpu);
                }
                (Prepared::Uploading(upload), None) => self.uploads.push(upload),
                (_, Some(error)) => {
                    log::warn!(
                        "the graphics device refused the silhouette of a mesh of {} triangles, so it is not drawn: {error}",
                        silhouette.mesh.triangle_count()
                    );
                    self.rejected.push(Arc::clone(&silhouette.mesh));
                    newly_rejected += 1;
                }
            }
        }
        self.started.clear();
        self.refused.clear();
        if self.is_uploading() {
            rewritten |= self.keep_previous(device, queue, anchor);
        }
        self.previous.clear();
        self.order.note(&self.silhouettes, rewritten);
        newly_rejected
    }

    fn keep_previous(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        anchor: Point3,
    ) -> bool {
        let mut previous: Vec<GpuSilhouette> = self.previous.rest().collect();
        if previous.iter().all(|silhouette| {
            silhouette
                .written
                .is_none_or(|written| written.placed.anchor == anchor)
        }) {
            self.silhouettes.append(&mut previous);
            return false;
        }
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
                    silhouette.write(queue, staging, &kept, anchor);
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
        true
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
                pass.draw_indexed(0..gpu::QUAD_INDEX_COUNT, 0, 0..chunk.count);
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
    use glam::Vec3;

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
    fn triangles_pack_three_positions_then_three_octahedral_normals() {
        let corner = |x: f32, normal: Vec3| Corner {
            position: Vec3::new(x, 0.0, 0.0),
            normal,
        };

        let bytes = packed_triangle(&[
            corner(1.0, Vec3::new(1.0, -1.0, 0.5)),
            corner(2.0, Vec3::Z),
            corner(3.0, Vec3::X),
        ]);

        assert_eq!(bytes[..4], 1.0f32.to_le_bytes());
        assert_eq!(bytes[24..28], 3.0f32.to_le_bytes());
        assert_eq!(bytes[36..40], [0x33, 0x33, 0xcd, 0xcc]);
        assert_eq!(bytes[40..44], [0, 0, 0, 0]);
        assert_eq!(bytes[44..48], [0xff, 0x7f, 0, 0]);
    }
}
