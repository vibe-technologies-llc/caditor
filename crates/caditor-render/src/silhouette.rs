use std::sync::Arc;

use caditor_geometry::{Point3, RigidTransform};
use parking_lot::Mutex;

use crate::{
    by_mesh::{ByMesh, DrawnOrder, OfMesh},
    culling::{ClipWindow, placed_corners},
    gpu::{self, Bytes, Pack},
    mesh::{Corner, Placed, PlacedAt, ShadedMesh, UploadBudget},
    mesh_table::{
        PLACEMENT_BYTES, WINDOW_SLOTS, grown_count, most_windows, window_binding, window_buffer,
        window_layout_entry, window_offset, window_slot, windows_wanted,
    },
    scene::Color,
    spans::{Held, SharedSpans, SlabSizes, SlabSpan, Slabs, Spans},
};

const SILHOUETTE_BYTES: usize = 48;
pub const SILHOUETTE_STRIDE: u64 = SILHOUETTE_BYTES as u64;
const SILHOUETTE_BINDING: u32 = 3;
const QUAD_TRIANGLE_VERTICES: u32 = 6;
const SILHOUETTE_SLAB_BYTES: u64 = 16 << 20;
const FIRST_SILHOUETTE_SLAB_BYTES: u64 = 256 << 10;
const PACKED_SHARE: u64 = 4;
const TRIANGLES: usize = 0;

#[derive(Clone, Debug)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
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

type SilhouetteSlabs = Slabs<wgpu::Buffer, 1>;

fn silhouette_slabs(device: &wgpu::Device) -> SilhouetteSlabs {
    let most = gpu::buffer_limit(device).min(SILHOUETTE_SLAB_BYTES);
    Slabs::new(SlabSizes {
        first: [most.min(FIRST_SILHOUETTE_SLAB_BYTES) / SILHOUETTE_STRIDE],
        most: [most / SILHOUETTE_STRIDE],
    })
}

fn triangle_buffer(device: &wgpu::Device, triangles: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("silhouette triangles"),
        size: triangles * SILHOUETTE_STRIDE,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

enum ChunkPlace {
    Own(wgpu::Buffer),
    Packed(SlabSpan<wgpu::Buffer, 1>),
}

struct Chunk {
    place: ChunkPlace,
    count: u32,
}

impl Chunk {
    fn buffer(&self) -> &wgpu::Buffer {
        match &self.place {
            ChunkPlace::Own(buffer) => buffer,
            ChunkPlace::Packed(span) => span.contents(),
        }
    }

    fn first(&self) -> u32 {
        match &self.place {
            ChunkPlace::Own(_) => 0,
            ChunkPlace::Packed(span) => u32::try_from(span.start(TRIANGLES)).unwrap_or(u32::MAX),
        }
    }
}

struct SilhouetteUpload {
    mesh: Arc<ShadedMesh>,
    slot: Held,
    chunks: Vec<Chunk>,
    per_chunk: usize,
    total: usize,
    written: usize,
    next_triangle: usize,
}

impl SilhouetteUpload {
    fn start(
        device: &wgpu::Device,
        mesh: Arc<ShadedMesh>,
        slabs: &mut SilhouetteSlabs,
        slot: Held,
    ) -> Self {
        let total = mesh.curved_triangle_count();
        let lengths = [total as u64];
        let packed = match slabs.sizes().packs(lengths, PACKED_SHARE) {
            true => slabs.place(lengths, |[triangles]| triangle_buffer(device, triangles)),
            false => None,
        };

        let (chunks, per_chunk) = match packed {
            Some(span) => {
                let chunk = Chunk {
                    place: ChunkPlace::Packed(span),
                    count: u32::try_from(total).unwrap_or(u32::MAX),
                };
                (vec![chunk], total.max(1))
            }
            None => {
                let per_chunk = usize::try_from(gpu::buffer_limit(device) / SILHOUETTE_STRIDE)
                    .unwrap_or(usize::MAX)
                    .max(1);
                let chunks = (0..total.div_ceil(per_chunk))
                    .map(|chunk| {
                        let count = per_chunk.min(total - chunk * per_chunk);
                        Chunk {
                            place: ChunkPlace::Own(triangle_buffer(device, count as u64)),
                            count: u32::try_from(count).unwrap_or(u32::MAX),
                        }
                    })
                    .collect();
                (chunks, per_chunk)
            }
        };
        Self {
            mesh,
            slot,
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
                let offset = (u64::from(chunk.first()) + within as u64) * SILHOUETTE_STRIDE;

                gpu::write_records(queue, chunk.buffer(), offset, granted, records);
            }
            self.next_triangle = next_triangle;
            self.written += staged;
            if staged < granted {
                self.total = self.written;
            }
        }
        true
    }

    fn finish(self) -> GpuSilhouette {
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
        GpuSilhouette::with_chunks(self.mesh, chunks, self.slot)
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

struct SilhouetteTable {
    styles: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    slots: SharedSpans,
}

impl SilhouetteTable {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, windows: u64) -> Self {
        let (styles, bind_group) = Self::resources(device, layout, windows);
        Self {
            styles,
            bind_group,
            slots: Arc::new(Mutex::new(Spans::new(windows * u64::from(WINDOW_SLOTS)))),
        }
    }

    fn resources(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        windows: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let styles = window_buffer(device, "silhouette styles", windows);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("silhouette styles"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: SILHOUETTE_BINDING,
                resource: window_binding(&styles),
            }],
        });
        (styles, bind_group)
    }

    fn hold(&self) -> Option<Held> {
        Held::take(&self.slots, 1)
    }

    fn windows(&self) -> u64 {
        self.slots.lock().capacity() / u64::from(WINDOW_SLOTS)
    }

    fn grown(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) -> Option<(u64, wgpu::Buffer, wgpu::BindGroup)> {
        let wanted = windows_wanted(&self.slots.lock());
        let windows = grown_count(self.windows(), wanted, most_windows(device))?;
        let (styles, bind_group) = Self::resources(device, layout, windows);
        Some((windows, styles, bind_group))
    }

    fn install(&mut self, (windows, styles, bind_group): (u64, wgpu::Buffer, wgpu::BindGroup)) {
        self.slots.lock().grow(windows * u64::from(WINDOW_SLOTS));
        self.styles = styles;
        self.bind_group = bind_group;
    }

    fn write(&self, queue: &wgpu::Queue, slot: &Held, bytes: &Bytes) {
        queue.write_buffer(
            &self.styles,
            slot.start() * PLACEMENT_BYTES,
            bytes.as_slice(),
        );
    }
}

struct GpuSilhouette {
    mesh: Arc<ShadedMesh>,
    chunks: Arc<[Chunk]>,
    slot: Held,
    written: Option<Written>,
    corners: Option<[Point3; 8]>,
}

impl GpuSilhouette {
    fn with_chunks(mesh: Arc<ShadedMesh>, chunks: Arc<[Chunk]>, slot: Held) -> Self {
        Self {
            mesh,
            chunks,
            slot,
            written: None,
            corners: None,
        }
    }

    fn sharing(&self, table: &SilhouetteTable) -> Option<Self> {
        Some(Self::with_chunks(
            Arc::clone(&self.mesh),
            Arc::clone(&self.chunks),
            table.hold()?,
        ))
    }

    fn needs_writing(&self, silhouette: &Silhouette, anchor: Point3) -> bool {
        self.written != Some(Written::of(silhouette, anchor))
    }

    fn write(
        &mut self,
        (queue, table): (&wgpu::Queue, &SilhouetteTable),
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
        self.put((queue, table), bytes);
    }

    fn put(&self, (queue, table): (&wgpu::Queue, &SilhouetteTable), bytes: &mut Bytes) {
        let Some(written) = self.written else {
            return;
        };
        let placed = Placed::of(
            self.mesh.origin(),
            written.placed.placement,
            written.placed.anchor,
        );
        let [turn_x, turn_y, turn_z] = placed.turn;

        bytes.clear();
        bytes
            .vec4(placed.offset, written.width)
            .floats(&written.color.to_array())
            .vec4(turn_x, 0.0)
            .vec4(turn_y, if written.dashed { 1.0 } else { 0.0 })
            .vec4(turn_z, 0.0);
        table.write(queue, &self.slot, bytes);
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

enum Source {
    Reused(Box<GpuSilhouette>),
    Started(SilhouetteUpload),
    New(Held),
}

pub struct SilhouetteCache {
    layout: wgpu::BindGroupLayout,
    table: SilhouetteTable,
    regrown: bool,
    slabs: SilhouetteSlabs,
    silhouettes: Vec<GpuSilhouette>,
    uploads: Vec<SilhouetteUpload>,
    rejected: Vec<Arc<ShadedMesh>>,
    previous: ByMesh<GpuSilhouette>,
    started: ByMesh<SilhouetteUpload>,
    refused: ByMesh<Arc<ShadedMesh>>,
    staging: Bytes,
    order: DrawnOrder,
    stopped: bool,
    left_out: usize,
}

impl SilhouetteCache {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("silhouette styles"),
            entries: &[window_layout_entry(SILHOUETTE_BINDING)],
        });
        let table = SilhouetteTable::new(device, &layout, 1);
        Self::with_table(device, layout, table)
    }

    fn with_table(
        device: &wgpu::Device,
        layout: wgpu::BindGroupLayout,
        table: SilhouetteTable,
    ) -> Self {
        Self {
            layout,
            table,
            regrown: false,
            slabs: silhouette_slabs(device),
            silhouettes: Vec::new(),
            uploads: Vec::new(),
            rejected: Vec::new(),
            previous: ByMesh::default(),
            started: ByMesh::default(),
            refused: ByMesh::default(),
            staging: Bytes::default(),
            order: DrawnOrder::default(),
            stopped: false,
            left_out: 0,
        }
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn sibling(&self, device: &wgpu::Device) -> Self {
        let windows = (self.silhouettes.len() as u64)
            .div_ceil(u64::from(WINDOW_SLOTS))
            .max(1);
        let table = SilhouetteTable::new(device, &self.layout, windows);
        let silhouettes = self
            .silhouettes
            .iter()
            .filter_map(|silhouette| silhouette.sharing(&table))
            .collect();
        Self {
            silhouettes,
            rejected: self.rejected.clone(),
            ..Self::with_table(device, self.layout.clone(), table)
        }
    }

    pub fn is_uploading(&self) -> bool {
        !self.stopped && !self.uploads.is_empty()
    }

    pub fn left_out(&self) -> usize {
        self.left_out
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
        self.stopped = budget.is_stopped();
        self.left_out = 0;

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

            if self.stopped && reused.is_none() {
                self.left_out += 1;
                self.uploads.extend(started);
                continue;
            }

            let source = match (reused, started) {
                (Some(ready), _) => Source::Reused(Box::new(ready)),
                (None, Some(upload)) => Source::Started(upload),
                (None, None) => match self.hold(device) {
                    Some(slot) => Source::New(slot),
                    None => {
                        self.rejected.push(Arc::clone(&silhouette.mesh));
                        newly_rejected += 1;
                        continue;
                    }
                },
            };
            let staging = &mut self.staging;
            let slabs = &mut self.slabs;
            let table = &self.table;

            let (prepared, error) = gpu::scoped(device, || {
                let mut upload = match source {
                    Source::Reused(mut ready) => {
                        ready.write((queue, table), staging, silhouette, anchor);
                        return Prepared::Ready(ready);
                    }
                    Source::Started(upload) => upload,
                    Source::New(slot) => {
                        SilhouetteUpload::start(device, Arc::clone(&silhouette.mesh), slabs, slot)
                    }
                };

                if !upload.advance(queue, budget) {
                    return Prepared::Uploading(upload);
                }

                let mut ready = upload.finish();

                ready.write((queue, table), staging, silhouette, anchor);
                Prepared::Ready(Box::new(ready))
            });
            match (prepared, error) {
                (Prepared::Ready(gpu), None) => {
                    rewritten = true;
                    self.silhouettes.push(*gpu);
                }
                (Prepared::Uploading(upload), None) => self.uploads.push(upload),
                (prepared, Some(error)) => {
                    drop(prepared);
                    log::warn!(
                        "the graphics device refused the silhouette of a mesh of {} triangles, so it is not drawn: {error}",
                        silhouette.mesh.triangle_count()
                    );
                    self.slabs.release_unused();
                    self.rejected.push(Arc::clone(&silhouette.mesh));
                    newly_rejected += 1;
                }
            }
        }
        self.started.clear();
        self.refused.clear();
        if !self.uploads.is_empty() || self.left_out > 0 {
            rewritten |= self.keep_previous(device, queue, anchor);
        }
        self.previous.clear();
        self.slabs.release_unused();
        if std::mem::take(&mut self.regrown) {
            rewritten |= self.rewrite_all(device, queue);
        }
        self.order.note(&self.silhouettes, rewritten);
        newly_rejected
    }

    fn hold(&mut self, device: &wgpu::Device) -> Option<Held> {
        if let Some(slot) = self.table.hold() {
            return Some(slot);
        }

        let (grown, error) = gpu::scoped(device, || self.table.grown(device, &self.layout));

        match (grown, error) {
            (Some(grown), None) => {
                self.table.install(grown);
                self.regrown = true;
                self.table.hold()
            }
            (_, Some(error)) => {
                log::warn!(
                    "the graphics device refused room for the style of a silhouette, so it is not drawn: {error}"
                );
                None
            }
            (None, None) => {
                log::warn!(
                    "the styles of more silhouettes do not fit what the graphics device holds, so one is not drawn"
                );
                None
            }
        }
    }

    fn rewrite_all(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let staging = &mut self.staging;
        let table = &self.table;

        let ((), error) = gpu::scoped(device, || {
            for silhouette in &self.silhouettes {
                silhouette.put((queue, table), staging);
            }
        });

        if let Some(error) = error {
            log::warn!(
                "the graphics device refused to rewrite the styles of {} silhouettes after making room for more: {error}",
                self.silhouettes.len()
            );
        }
        true
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
        let table = &self.table;

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

                    silhouette.write((queue, table), staging, &kept, anchor);
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

        let mut styles_window = None;
        let mut bound: Option<&wgpu::Buffer> = None;

        for silhouette in seen {
            let slot = silhouette.slot.start();
            let offset = window_offset(slot);
            let first_vertex = window_slot(slot) * QUAD_TRIANGLE_VERTICES;

            if styles_window != Some(offset) {
                pass.set_bind_group(1, &self.table.bind_group, &[offset]);
                styles_window = Some(offset);
            }
            for chunk in silhouette.chunks.iter() {
                let buffer = chunk.buffer();

                if bound != Some(buffer) {
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    bound = Some(buffer);
                }

                let first = chunk.first();

                pass.draw(
                    first_vertex..first_vertex + QUAD_TRIANGLE_VERTICES,
                    first..first.saturating_add(chunk.count),
                );
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
